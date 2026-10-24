//! 快照仓库 Adapter（IF-011/012/013 + 捕获/回收站/恢复写入，M7/T-037）。
//! 内容寻址（sha256）+ gzip 压缩 + 去重；retention 天数/容量双轨 LRU（AC-31/32）。
//! 第三方组件：sha2 / flate2 / std::fs——业务层禁直接触达，一律经本模块。

use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{
    CaptureTreeInput, ContentRef, FileCaptured, RetentionPolicy, SnapPort, SnapshotData,
    SweepReport, TrashAck, TrashRequest, WriteFileInput,
};
use sha2::{Digest, Sha256};
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

/// 真实实现：本地文件系统内容寻址仓库。
#[derive(Debug, Default)]
pub struct RealSnap;

impl RealSnap {
    pub fn new() -> Self {
        Self
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
        let hex = sha256_hex(&input.bytes);
        let dir = snapshots_dir().join(&hex[..2]);
        let target = dir.join(format!("{hex}.gz"));
        if !target.exists() {
            let compressed = gzip(&input.bytes);
            if let Err(e) = write_atomic(&target, &compressed) {
                evolution.fail("IAC-1601", format!("快照写入失败: {e}"));
                return false;
            }
        } else {
            metrics.counter("snapstore.dedup", 1);
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
        let path = snapshots_dir().join(&hex[..2]).join(format!("{hex}.gz"));
        let raw = match std::fs::read(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                evolution.fail("IAC-1602", format!("快照已被回收: {}", input.reference));
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
        if sha256_hex(&bytes) != hex {
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
        // 快照：先按天数清退，再按容量 LRU 清退
        let mut snaps = list_files(&snapshots_dir());
        snaps.sort_by_key(|(_, meta)| meta.1);
        let now = now_secs();
        for (path, (size, mtime)) in &snaps {
            let aged = now.saturating_sub(*mtime) > u64::from(input.days) * 86_400;
            if aged {
                if std::fs::remove_file(path).is_ok() {
                    report.removed += 1;
                    report.freed_bytes += *size;
                }
            }
        }
        let mut rest: Vec<u64> = list_files(&snapshots_dir())
            .into_iter()
            .map(|(_, (size, _))| size)
            .collect();
        let mut total: u64 = rest.iter().sum();
        rest.sort_unstable_by_key(|_| 0u8); // 顺序已被删除破坏，重新按 mtime 清退
        let mut ordered = list_files(&snapshots_dir());
        ordered.sort_by_key(|(_, meta)| meta.1);
        for (path, (size, _)) in &ordered {
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
        for (path, (_, mtime)) in list_files(&trash_dir()) {
            if now.saturating_sub(mtime) > u64::from(input.days) * 86_400 {
                if std::fs::remove_file(path).is_ok() {
                    report.removed += 1;
                    report.freed_bytes += 0;
                }
            }
        }
        if list_files(&snapshots_dir())
            .into_iter()
            .map(|(_, (s, _))| s)
            .sum::<u64>()
            > input.max_bytes
        {
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
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let mut files = Vec::new();
        let mut stored_total: u64 = 0;
        for root in &input.roots {
            walk(root, root, &input.excludes, &mut |abs| {
                let Ok(meta) = std::fs::metadata(abs) else { return };
                if !meta.is_file() || meta.is_symlink() {
                    return;
                }
                let Ok(_rel) = abs.strip_prefix(root) else { return };
                let Ok(bytes) = std::fs::read(abs) else { return };
                let hex = sha256_hex(&bytes);
                let storable = bytes.len() as u64 <= input.max_file_bytes
                    && stored_total + bytes.len() as u64 <= input.max_total_bytes;
                let mut captured = FileCaptured {
                    path: abs.to_string_lossy().to_string(),
                    sha256: hex,
                    size: bytes.len() as u64,
                    stored: false,
                    content_ref: None,
                };
                if storable {
                    let mut cref = ContentRef::default();
                    if self.snapstore_put(
                        &SnapshotData { bytes },
                        &mut cref,
                        context,
                        metrics,
                        evolution,
                    ) {
                        captured.stored = true;
                        captured.content_ref = Some(cref.reference);
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
        let flat: String = input
            .abs_path
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let target = dir.join(format!("{}_{}", input.tag, flat));
        match std::fs::rename(&input.abs_path, &target) {
            Ok(()) => {
                output.trash_path = target.to_string_lossy().to_string();
                output.moved = true;
                true
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
        if let Some(parent) = input.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                evolution.fail("IAC-1601", format!("父目录创建失败: {e}"));
                return false;
            }
        }
        match std::fs::write(&input.path, &input.bytes) {
            Ok(()) => true,
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

/// sha256 十六进制小写。
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
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

/// 原子写（临时文件 + 改名）。
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// 列出目录下全部文件：(路径, (大小字节, mtime 秒))。
fn list_files(dir: &Path) -> Vec<(PathBuf, (u64, u64))> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            out.push((
                entry.path(),
                (meta.len(), meta.modified().map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
                }).unwrap_or(0)),
            ));
        }
    }
    out
}

/// 递归遍历（跳过排除名、符号链接），叶子回调绝对路径。
fn walk(root: &Path, cur: &Path, excludes: &[String], cb: &mut impl FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(cur) else { return };
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
            walk(root, &p, excludes, cb);
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
            &SnapshotData { bytes: b"hello rollback".to_vec() },
            &mut cref,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(cref.reference.starts_with("sha256:"));
        // 去重：同内容再写仍成功
        let mut cref2 = ContentRef::default();
        assert!(snap.snapstore_put(
            &SnapshotData { bytes: b"hello rollback".to_vec() },
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
            &ContentRef { reference: "sha256:deadbeef".into() },
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
    fn trash_and_sweep() {
        let _guard = FS_LOCK.lock().unwrap();
        let (env, root) = env_with_root("sweep");
        let snap = RealSnap::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        // 放入 3 份不同内容
        for i in 0..3 {
            let mut cref = ContentRef::default();
            assert!(snap.snapstore_put(
                &SnapshotData { bytes: vec![b'x' + i; 100] },
                &mut cref,
                &env.context(),
                &mut m,
                &mut evo
            ));
        }
        // 容量压到 100 → 只留最旧之外的部分被清退
        let mut report = SweepReport::default();
        assert!(snap.sweep_expired(
            &RetentionPolicy { days: 7, max_bytes: 100 },
            &mut report,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(report.removed >= 2);
        // 回收站：移动并清退
        let f = root.join("victim.txt");
        std::fs::write(&f, b"bye").unwrap();
        let mut ack = TrashAck::default();
        assert!(snap.trash_file(
            &TrashRequest { abs_path: f.clone(), tag: "e1_1".into() },
            &mut ack,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert!(ack.moved && !f.exists() && ack.trash_path.contains("trash"));
        let mut report2 = SweepReport::default();
        assert!(snap.sweep_expired(
            &RetentionPolicy { days: 0, max_bytes: u64::MAX },
            &mut report2,
            &env.context(),
            &mut m,
            &mut evo
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
