//! 快照仓库 Adapter（IF-011/012/013 + 捕获/回收站/恢复写入，M7/T-037）。
//! 内容寻址（sha256）+ gzip 压缩 + 去重；retention 天数/容量双轨 LRU（AC-31/32）。
//! 第三方组件：sha2 / flate2 / std::fs——业务层禁直接触达，一律经本模块。

use crate::fsutil;
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{
    CaptureTreeInput, ContentRef, FileCaptured, RetentionPolicy, SnapPort, SnapshotData,
    SweepReport, TrashAck, TrashRequest, WriteFileInput,
};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// 数据根：`IAC_DATA_DIR` 覆盖（测试隔离），缺省 `$XDG_DATA_HOME/iac`。
pub fn data_root() -> PathBuf {
    std::env::var("IAC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("iac")
        })
}

fn snapshots_dir() -> PathBuf {
    data_root().join("snapshots")
}

fn trash_dir() -> PathBuf {
    data_root().join("trash")
}

/// 跨文件系统判定（rename EXDEV）：unix 18 / windows ERROR_NOT_SAME_DEVICE 17。
fn is_cross_device(e: &std::io::Error) -> bool {
    if cfg!(windows) {
        e.raw_os_error() == Some(17)
    } else {
        e.raw_os_error() == Some(18)
    }
}

/// 真实实现：本地文件系统内容寻址仓库。
#[derive(Debug, Default)]
pub struct RealSnap;

impl RealSnap {
    pub fn new() -> Self {
        Self
    }

    /// 内容入库（snapstore_put 与 capture_tree 共用；调用方已算好 hex，免二次哈希）。
    /// 去重命中仅计数；未命中 gzip + 原子写。
    fn store_content(
        &self,
        bytes: &[u8],
        hex: &str,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        let dir = snapshots_dir().join(&hex[..2]);
        let target = dir.join(format!("{hex}.gz"));
        if !target.exists() {
            let compressed = gzip(bytes);
            if let Err(e) = fsutil::write_atomic(&target, &compressed) {
                evolution.fail("IAC-1601", format!("快照写入失败: {e}"));
                return false;
            }
        } else {
            metrics.counter("snapstore.dedup", 1);
        }
        true
    }
}

impl SnapPort for RealSnap {
    fn snapstore_put(
        &self,
        input: &SnapshotData,
        output: &mut ContentRef,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let hex = fsutil::sha256_hex(&input.bytes);
        if !self.store_content(&input.bytes, &hex, metrics, evolution) {
            return false;
        }
        output.reference = format!("sha256:{hex}");
        true
    }

    fn snapstore_fetch(
        &self,
        input: &ContentRef,
        output: &mut SnapshotData,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let Some(hex) = input.reference.strip_prefix("sha256:") else {
            evolution.fail("IAC-1602", format!("非法内容引用: {}", input.reference));
            return false;
        };
        let hex = hex.to_ascii_lowercase();
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            evolution.fail("IAC-1602", format!("非法内容引用: {}", input.reference));
            return false;
        }
        let path = snapshots_dir().join(&hex[..2]).join(format!("{hex}.gz"));
        let raw = match std::fs::read(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                evolution.fail(
                    "IAC-1602",
                    format!("快照不存在（可能已被回收）: {}", input.reference),
                );
                return false;
            }
            Err(e) => {
                evolution.fail("IAC-1602", format!("快照读取失败: {e}"));
                return false;
            }
        };
        let bytes = match gunzip(&raw) {
            Ok(b) => b,
            Err(e) => {
                evolution.fail("IAC-1602", format!("快照解压失败: {e}"));
                return false;
            }
        };
        if fsutil::sha256_hex(&bytes) != hex {
            evolution.fail("IAC-1602", "快照完整性校验失败（sha256 不符）");
            return false;
        }
        output.bytes = bytes;
        true
    }

    fn sweep_expired(
        &self,
        input: &RetentionPolicy,
        output: &mut SweepReport,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let mut report = SweepReport::default();
        let now = now_secs();
        let day_secs = u64::from(input.days) * 86_400;
        // 快照：先按天数清退，再按容量 LRU（mtime 升序）清退
        let mut snaps = list_files(&snapshots_dir());
        snaps.sort_by_key(|(_, meta)| meta.1);
        for (path, (size, mtime)) in &snaps {
            if now.saturating_sub(*mtime) > day_secs && std::fs::remove_file(path).is_ok() {
                report.removed += 1;
                report.freed_bytes += *size;
            }
        }
        // 天数清退后重新列举，仍按 mtime 升序清退到容量线内
        let mut rest = list_files(&snapshots_dir());
        rest.sort_by_key(|(_, meta)| meta.1);
        let mut total: u64 = rest.iter().map(|(_, (size, _))| size).sum();
        for (path, (size, _)) in &rest {
            if total <= input.max_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                report.removed += 1;
                report.freed_bytes += *size;
                total = total.saturating_sub(*size);
                metrics.counter("snapstore.swept", 1);
            }
        }
        // 回收站：仅按天数清退
        for (path, (size, mtime)) in list_files(&trash_dir()) {
            if now.saturating_sub(mtime) > day_secs && std::fs::remove_file(path).is_ok() {
                report.removed += 1;
                report.freed_bytes += size;
            }
        }
        // 终检：total 只在删除成功时扣减，无需再扫目录
        if total > input.max_bytes {
            evolution.fail("IAC-1601", "快照仓库超限且无法回收，请调大 retention 上限");
            return false;
        }
        *output = report;
        true
    }

    fn capture_tree(
        &self,
        input: &CaptureTreeInput,
        output: &mut Vec<FileCaptured>,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let mut files = Vec::new();
        let mut stored_total: u64 = 0;
        for root in &input.roots {
            walk(root, &input.excludes, &mut |abs| {
                let Ok(meta) = std::fs::metadata(abs) else {
                    return;
                };
                if !meta.is_file() || meta.is_symlink() {
                    return;
                }
                if abs.strip_prefix(root).is_err() {
                    return;
                };
                // 先 stat 判尺寸再决定读取方式：超限文件只流式取指纹，不整载入内存
                let size = meta.len();
                let storable =
                    size <= input.max_file_bytes && stored_total + size <= input.max_total_bytes;
                let payload = if storable {
                    std::fs::read(abs).ok()
                } else {
                    None
                };
                let hex = match &payload {
                    Some(b) => fsutil::sha256_hex(b),
                    None => match fsutil::sha256_file(abs) {
                        Ok(h) => h,
                        Err(_) => return,
                    },
                };
                let mut captured = FileCaptured {
                    path: abs.to_string_lossy().to_string(),
                    sha256: hex,
                    size,
                    stored: false,
                    content_ref: None,
                };
                if let Some(bytes) = payload {
                    if self.store_content(&bytes, &captured.sha256, metrics, evolution) {
                        captured.stored = true;
                        captured.content_ref = Some(format!("sha256:{}", captured.sha256));
                        stored_total += captured.size;
                    }
                }
                files.push(captured);
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        *output = files;
        true
    }

    fn trash_file(
        &self,
        input: &TrashRequest,
        output: &mut TrashAck,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        if !input.abs_path.exists() {
            output.moved = false;
            return true;
        }
        let dir = trash_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            evolution.fail("IAC-1601", "回收站目录创建失败");
            return false;
        }
        // tag 一并扁平化（杜绝分隔符/穿越）；冲突自增后缀，绝不覆盖已有回收条目
        let flat: String = format!("{}_{}", input.tag, input.abs_path.to_string_lossy())
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let mut target = dir.join(&flat);
        let mut n = 1u32;
        while target.exists() {
            if n > 1000 {
                evolution.fail("IAC-1601", format!("回收站条目冲突过多: {flat}"));
                return false;
            }
            target = dir.join(format!("{flat}__{n}"));
            n += 1;
        }
        match std::fs::rename(&input.abs_path, &target) {
            Ok(()) => {
                output.trash_path = target.to_string_lossy().to_string();
                output.moved = true;
                true
            }
            Err(e) if is_cross_device(&e) => {
                // 跨文件系统 rename 必败（EXDEV，如 /tmp tmpfs → home 盘）：复制+删除兜底。
                // 回滚效果均为常规文件（diff 依内容指纹），目录不支持跨设备回退。
                if !input.abs_path.is_file() {
                    evolution.fail("IAC-1601", format!("跨文件系统移动仅支持常规文件: {e}"));
                    return false;
                }
                match std::fs::copy(&input.abs_path, &target)
                    .and_then(|_| std::fs::remove_file(&input.abs_path))
                {
                    Ok(_) => {
                        output.trash_path = target.to_string_lossy().to_string();
                        output.moved = true;
                        true
                    }
                    Err(e2) => {
                        let _ = std::fs::remove_file(&target); // 半成品清理
                        evolution.fail("IAC-1601", format!("移入回收站失败: {e2}"));
                        false
                    }
                }
            }
            Err(e) => {
                evolution.fail("IAC-1601", format!("移入回收站失败: {e}"));
                false
            }
        }
    }

    fn write_file(
        &self,
        input: &WriteFileInput,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        // 原子写：恢复过程崩溃不留半写文件
        match fsutil::write_atomic(&input.path, &input.bytes) {
            Ok(_) => true,
            Err(e) => {
                evolution.fail("IAC-1601", format!("恢复写入失败: {e}"));
                false
            }
        }
    }

    fn data_root(&self) -> PathBuf {
        data_root()
    }
}

/// 当前 Unix 秒。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// gzip 压缩。
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = enc.write_all(bytes);
    enc.finish().unwrap_or_default()
}

/// gzip 解压。
fn gunzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut dec = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

/// 列出目录下全部文件（递归：快照按 {hex前2}/{hex}.gz 分片，非递归会全部漏扫）。
fn list_files(dir: &Path) -> Vec<(PathBuf, (u64, u64))> {
    let mut out = Vec::new();
    list_files_into(dir, &mut out);
    out
}

fn list_files_into(dir: &Path, out: &mut Vec<(PathBuf, (u64, u64))>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            list_files_into(&entry.path(), out);
        } else if meta.is_file() {
            out.push((
                entry.path(),
                (
                    meta.len(),
                    meta.modified()
                        .map(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        })
                        .unwrap_or(0),
                ),
            ));
        }
    }
}

/// 递归遍历（跳过排除名、符号链接），叶子回调绝对路径。
fn walk(cur: &Path, excludes: &[String], cb: &mut impl FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(cur) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if excludes.iter().any(|x| x == &name) {
            continue;
        }
        if meta.is_symlink() {
            continue;
        }
        let p = entry.path();
        if meta.is_dir() {
            walk(&p, excludes, cb);
        } else if meta.is_file() {
            cb(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iac_core::framework::TestEnv;
    use std::sync::Mutex;

    /// IAC_DATA_DIR 是进程级环境变量：FS 断言类测试串行化避免竞态。
    static FS_LOCK: Mutex<()> = Mutex::new(());

    fn env_with_root(tag: &str) -> (TestEnv, PathBuf) {
        let root = std::env::temp_dir().join(format!("iac-snap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("snapshots")).unwrap();
        std::env::set_var("IAC_DATA_DIR", &root);
        (TestEnv::new(), root)
    }

    #[test]
    fn put_fetch_roundtrip_and_dedup() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("rt");
        let snap = RealSnap::new();
        let mut cref = ContentRef::default();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(snap.snapstore_put(
            &SnapshotData {
                bytes: b"hello rollback".to_vec()
            },
            &mut cref,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(cref.reference.starts_with("sha256:"));
        // 去重：同内容再写仍成功
        let mut cref2 = ContentRef::default();
        assert!(snap.snapstore_put(
            &SnapshotData {
                bytes: b"hello rollback".to_vec()
            },
            &mut cref2,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert_eq!(cref, cref2);
        // 取回内容一致
        let mut back = SnapshotData::default();
        assert!(snap.snapstore_fetch(&cref, &mut back, &env.context(), &mut m, &mut evo));
        assert_eq!(back.bytes, b"hello rollback".to_vec());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fetch_missing_is_1602() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("miss");
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        let mut out = SnapshotData::default();
        assert!(!snap.snapstore_fetch(
            &ContentRef {
                reference: "sha256:deadbeef".into()
            },
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert_eq!(evo.reason_code.as_deref(), Some("IAC-1602"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn capture_tree_excludes_and_caps() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("cap");
        let tree = root.join("tree");
        std::fs::create_dir_all(tree.join("sub").join(".git")).unwrap();
        std::fs::write(tree.join("a.txt"), b"A").unwrap();
        std::fs::write(tree.join("sub").join("b.txt"), b"BB").unwrap();
        std::fs::write(tree.join("sub").join(".git").join("c"), b"skip").unwrap();
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        let mut out = Vec::new();
        assert!(snap.capture_tree(
            &CaptureTreeInput {
                roots: vec![tree.clone()],
                excludes: vec![".git".into()],
                max_file_bytes: 1024,
                max_total_bytes: 4096,
            },
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert_eq!(out.len(), 2, "排除 .git 后只捕获两个文件");
        assert!(out.iter().all(|f| f.stored && f.content_ref.is_some()));
        // 单文件超限：max_file=0 时全部只记指纹不存内容
        let mut out2 = Vec::new();
        assert!(snap.capture_tree(
            &CaptureTreeInput {
                roots: vec![tree.clone()],
                excludes: vec![".git".into()],
                max_file_bytes: 0,
                max_total_bytes: 1,
            },
            &mut out2,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(out2.iter().all(|f| !f.stored));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn short_reference_fails_1602_without_panic() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("shortref");
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        let mut out = SnapshotData::default();
        // 此前 &hex[..2] 对短引用直接 panic
        for bad in ["sha256:a", "sha256:", "sha256:zzzz", "not-a-ref"] {
            assert!(!snap.snapstore_fetch(
                &ContentRef {
                    reference: bad.into()
                },
                &mut out,
                &env.context(),
                &mut m,
                &mut evo
            ));
            assert_eq!(evo.reason_code.as_deref(), Some("IAC-1602"), "ref={bad}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn trash_collision_never_overwrites() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("trashcol");
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        // 两个不同文件扁平化后同名（a/b.txt 与 a_b.txt → *_a_b_txt）：都须保留
        let sub = root.join("a");
        std::fs::create_dir_all(&sub).unwrap();
        let f1 = sub.join("b.txt");
        let f2 = root.join("a_b.txt");
        std::fs::write(&f1, b"one").unwrap();
        std::fs::write(&f2, b"two-two").unwrap();
        let mut ack1 = TrashAck::default();
        let mut ack2 = TrashAck::default();
        assert!(snap.trash_file(
            &TrashRequest {
                abs_path: f1.clone(),
                tag: "t1".into()
            },
            &mut ack1,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(snap.trash_file(
            &TrashRequest {
                abs_path: f2.clone(),
                tag: "t1".into()
            },
            &mut ack2,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(ack1.moved && ack2.moved);
        assert_ne!(ack1.trash_path, ack2.trash_path, "冲突条目应各自落位");
        let kept: Vec<u64> = std::fs::read_dir(trash_dir())
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .collect();
        assert_eq!(kept.len(), 2, "两条回收条目都存在，无覆盖");
        assert!(
            kept.contains(&3) && kept.contains(&7),
            "两个文件内容都完整保留（3B + 7B）"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn trash_and_sweep() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("sweep");
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        // 放入 3 份不同且不可压缩的内容（gzip 后仍大于单文件上限，确保容量清退可触发）
        for i in 0..3u8 {
            let bytes: Vec<u8> = (0..512u32)
                .map(|n| {
                    let h = n
                        .wrapping_mul(2654435761)
                        .wrapping_add(u32::from(i).wrapping_mul(0x9E37_79B9));
                    (h >> 24) as u8 ^ n as u8
                })
                .collect();
            let mut cref = ContentRef::default();
            assert!(snap.snapstore_put(
                &SnapshotData { bytes },
                &mut cref,
                &env.context(),
                &mut m,
                &mut evo
            ));
        }
        // 容量压到单文件以下 → 至少清退 2 份
        let mut report = SweepReport::default();
        assert!(snap.sweep_expired(
            &RetentionPolicy {
                days: 7,
                max_bytes: 256
            },
            &mut report,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(
            report.removed >= 2,
            "removed={} freed={} removed_bytes_detail={:?}",
            report.removed,
            report.freed_bytes,
            list_files(&snapshots_dir())
        );
        // 回收站：移动并清退
        let f = root.join("victim.txt");
        std::fs::write(&f, b"bye").unwrap();
        let mut ack = TrashAck::default();
        assert!(snap.trash_file(
            &TrashRequest {
                abs_path: f.clone(),
                tag: "e1_1".into()
            },
            &mut ack,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(ack.moved && !f.exists() && ack.trash_path.contains("trash"));
        let mut report2 = SweepReport::default();
        assert!(snap.sweep_expired(
            &RetentionPolicy {
                days: 0,
                max_bytes: u64::MAX
            },
            &mut report2,
            &env.context(),
            &mut m,
            &mut evo
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
