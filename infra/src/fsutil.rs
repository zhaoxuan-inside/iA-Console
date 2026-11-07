//! 文件与摘要公用件（infra 内部复用）：原子写（fsync + 唯一临时名）、sha256（内存/流式）。
//! 业务层禁直接触达 std::fs 关键路径，统一经此处。

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内唯一临时文件序号（避免并发写同一目标时临时名互踩）。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 原子写：唯一临时文件 + fsync + 改名 + 父目录 fsync。
/// 返回写入字节数。中途崩溃只会留下孤儿临时文件，目标文件要么旧要么新。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<usize> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_extension(format!("tmp.{seq}.{file_name}"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all(); // 尽力而为：目录项持久化
        }
    }
    Ok(bytes.len())
}

/// sha256 十六进制小写（内存数据）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// 流式 sha256（文件逐块读，内存占用恒定；大文件指纹用）。
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_roundtrip_and_sha() {
        let dir = std::env::temp_dir().join(format!("iac-fsutil-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a/b/c.txt");
        assert_eq!(write_atomic(&p, b"hello").unwrap(), 5);
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        assert_eq!(sha256_hex(b"hello"), sha256_file(&p).unwrap());
        // 重复写同一路径（同进程内唯一临时名，不互踩）
        write_atomic(&p, b"world!").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"world!");
        // 目录无孤儿临时文件
        assert!(std::fs::read_dir(dir.join("a/b")).unwrap().count() == 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
