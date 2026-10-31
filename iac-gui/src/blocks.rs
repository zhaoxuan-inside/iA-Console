//! 块式输出（Q54）：受控 shell 事件侧信道 → 命令块。
//! rc 钩子（shell_assets）向 `IAC_EVENT_FILE` 追加 US(\x1f) 分隔行：
//!   `S<US><epoch 秒.微><US><命令（换行转义 \\n）>` —— preexec
//!   `E<US><epoch><US><退出码><US><cwd>`           —— precmd
//! GUI 每帧增量轮询（seek+offset），解析为命令块（退出码/耗时/cwd），
//! 呈现于终端下方块条（Warp 式块历史的克制版，Q53 调性）。

/// 一条命令块；`exit=None` 表示仍在运行。
#[derive(Debug, Clone, PartialEq)]
pub struct ShellBlock {
    pub command: String,
    pub started: f64,
    pub exit: Option<i32>,
    pub duration: f64,
    pub cwd: String,
}

/// 块数上限（防长会话无界增长，LRU 截断旧块）。
const BLOCK_CAP: usize = 500;

/// 增量解析事件行；`pending` 保存未闭合的运行中块。
pub fn ingest(pending: &mut Vec<ShellBlock>, blocks: &mut Vec<ShellBlock>, raw: &str) {
    for line in raw.lines() {
        let mut parts = line.split('\x1f');
        match parts.next() {
            Some("S") => {
                let started = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let command = parts.next().unwrap_or_default().replace("\\n", "\n");
                // GUI 自身注入的 cwd 恢复（Q41/Q46）不算用户命令
                if command.starts_with("cd '") && command.ends_with("&& clear") {
                    continue;
                }
                pending.push(ShellBlock {
                    command,
                    started,
                    exit: None,
                    duration: 0.0,
                    cwd: String::new(),
                });
            }
            Some("E") => {
                let ended = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let exit = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let cwd = parts.next().unwrap_or_default().to_owned();
                if let Some(mut block) = pending.pop() {
                    block.exit = Some(exit);
                    block.duration = (ended - block.started).max(0.0);
                    block.cwd = cwd;
                    blocks.push(block);
                    if blocks.len() > BLOCK_CAP {
                        let drop = blocks.len() - BLOCK_CAP;
                        blocks.drain(0..drop);
                    }
                }
            }
            _ => {}
        }
    }
}

/// 轮询事件文件自 `offset` 起的新内容；返回新 offset（文件消失/重置则回退到 0 重读）。
pub fn poll(
    path: &std::path::Path,
    offset: u64,
    pending: &mut Vec<ShellBlock>,
    blocks: &mut Vec<ShellBlock>,
) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return offset;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < offset { 0 } else { offset };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return offset;
    }
    let mut raw = String::new();
    let n = file.read_to_string(&mut raw).unwrap_or(0);
    if n > 0 {
        ingest(pending, blocks, &raw);
    }
    start + n as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<ShellBlock>, Vec<ShellBlock>) {
        (Vec::new(), Vec::new())
    }

    #[test]
    fn s_then_e_builds_closed_block() {
        let (mut pending, mut blocks) = sample();
        ingest(&mut pending, &mut blocks, "S\x1f1000.5\x1fls -la\n");
        assert!(pending.len() == 1 && blocks.is_empty());
        ingest(&mut pending, &mut blocks, "E\x1f1001.25\x1f0\x1f/home/u\n");
        assert!(pending.is_empty());
        let b = &blocks[0];
        assert_eq!(b.command, "ls -la");
        assert_eq!(b.exit, Some(0));
        assert!((b.duration - 0.75).abs() < 1e-9);
        assert_eq!(b.cwd, "/home/u");
    }

    #[test]
    fn cwd_restore_injection_is_skipped() {
        let (mut pending, mut blocks) = sample();
        ingest(
            &mut pending,
            &mut blocks,
            "S\x1f10.0\x1fcd '/home/u/proj' && clear\nE\x1f10.1\x1f0\x1f/home/u/proj\n",
        );
        assert!(pending.is_empty() && blocks.is_empty());
    }

    #[test]
    fn e_without_pending_is_ignored() {
        let (mut pending, mut blocks) = sample();
        ingest(&mut pending, &mut blocks, "E\x1f1.0\x1f0\x1f/\n");
        assert!(pending.is_empty() && blocks.is_empty());
    }

    #[test]
    fn multiline_command_is_restored_and_running_block_stays_pending() {
        let (mut pending, mut blocks) = sample();
        ingest(&mut pending, &mut blocks, "S\x1f5.0\x1fecho a\\necho b\n");
        assert_eq!(pending[0].command, "echo a\necho b");
        assert_eq!(pending[0].exit, None);
        assert!(blocks.is_empty());
    }

    #[test]
    fn poll_resumes_from_offset_and_handles_truncation() {
        let dir = std::env::temp_dir().join(format!("iac-blocks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.log");
        std::fs::write(&path, "S\x1f1.0\x1fls\nE\x1f1.1\x1f0\x1f/\n").unwrap();
        let (mut pending, mut blocks) = sample();
        let off = poll(&path, 0, &mut pending, &mut blocks);
        assert_eq!(blocks.len(), 1);
        // 无新增 → 原样返回
        assert_eq!(poll(&path, off, &mut pending, &mut blocks), off);
        // 文件被截断重写 → 回退 0 重读
        std::fs::write(&path, "S\x1f2.0\x1fpwd\n").unwrap();
        poll(&path, off, &mut pending, &mut blocks);
        assert!(!pending.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
