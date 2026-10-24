//! 通用纯算法：FNV-1a 64 稳定散列（source_hash / 缓存键生成，零依赖）。

/// FNV-1a 64 位散列，输出 16 位十六进制小写。
pub fn fnv1a64_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_distinct() {
        let a = fnv1a64_hex("ls|linux");
        assert_eq!(a, fnv1a64_hex("ls|linux"), "同输入恒同输出");
        assert_ne!(a, fnv1a64_hex("ls|macos"));
        assert_eq!(a.len(), 16);
    }
}
