//! 主题令牌与剪贴板（T-030/T-033，前端纯数据与构建器）。
//! 色值来自 _04/03 设计语言（14 令牌双主题）；OSC52 为 TUI 剪贴板唯一通道。

use serde::{Deserialize, Serialize};

/// 主题令牌（14 个语义槽位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Bg,
    Fg,
    Fg60,
    Fg40,
    Fg25,
    Accent,
    Safe,
    Caution,
    Dangerous,
    Destructive,
    DiffAdd,
    DiffDel,
    SynKeyword,
    SynString,
}

/// 主题模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Day,
    Night,
}

/// 主题表：token → "#RRGGBB"。
pub fn palette(mode: Mode) -> [(&'static str, &'static str); 14] {
    match mode {
        Mode::Day => [
            ("bg", "#FAFAF7"),
            ("fg", "#262A33"),
            ("fg-60", "#6B7080"),
            ("fg-40", "#9CA1AC"),
            ("fg-25", "#C3C7CE"),
            ("accent", "#316BFF"),
            ("safe", "#2E7D32"),
            ("caution", "#B26A00"),
            ("dangerous", "#C7500A"),
            ("destructive", "#B3261E"),
            ("diff-add", "#2E7D32"),
            ("diff-del", "#B3261E"),
            ("syn-keyword", "#8250DF"),
            ("syn-string", "#0A6E6E"),
        ],
        Mode::Night => [
            ("bg", "#1B1D22"),
            ("fg", "#D6D8DD"),
            ("fg-60", "#8B8F98"),
            ("fg-40", "#5C6068"),
            ("fg-25", "#3A3E46"),
            ("accent", "#6E9BFF"),
            ("safe", "#66BB6A"),
            ("caution", "#FFB74D"),
            ("dangerous", "#FF8A50"),
            ("destructive", "#FF5C52"),
            ("diff-add", "#66BB6A"),
            ("diff-del", "#FF5C52"),
            ("syn-keyword", "#C297FF"),
            ("syn-string", "#6FC7C7"),
        ],
    }
}

/// token → (R, G, B)（console 渲染用）。
pub fn rgb(mode: Mode, token: Token) -> (u8, u8, u8) {
    let hex = color(mode, token);
    let h = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
    )
}

/// token → 颜色值。
pub fn color(mode: Mode, token: Token) -> &'static str {
    palette(mode)
        .iter()
        .find(|(name, _)| token_name(token) == *name)
        .map(|(_, hex)| *hex)
        .unwrap_or("#000000")
}

fn token_name(t: Token) -> &'static str {
    match t {
        Token::Bg => "bg",
        Token::Fg => "fg",
        Token::Fg60 => "fg-60",
        Token::Fg40 => "fg-40",
        Token::Fg25 => "fg-25",
        Token::Accent => "accent",
        Token::Safe => "safe",
        Token::Caution => "caution",
        Token::Dangerous => "dangerous",
        Token::Destructive => "destructive",
        Token::DiffAdd => "diff-add",
        Token::DiffDel => "diff-del",
        Token::SynKeyword => "syn-keyword",
        Token::SynString => "syn-string",
    }
}

/// 复制三档（词典：CopyScope）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyScope {
    /// 仅当前命令原文。
    Command,
    /// 仅译文原文。
    Translated,
    /// 全量可见快照（去色码）。
    All,
}

/// 三档内容组装（AC-37 纯净规则：命令/译文档零装饰，全部档去色码快照）。
pub fn build_copy_content(scope: CopyScope, command: &str, translated: Option<&str>, full_snapshot: &str) -> String {
    match scope {
        CopyScope::Command => clean_command(command),
        CopyScope::Translated => translated.map(clean_command).unwrap_or_else(|| clean_command(command)),
        CopyScope::All => full_snapshot.lines().map(str::trim_end).collect::<Vec<_>>().join("\n"),
    }
}

/// 纯净命令：剥提示符/行号/⚠/色码/尾随换行，粘贴即执行。
fn clean_command(cmd: &str) -> String {
    cmd.trim()
        .trim_start_matches("$ ")
        .trim_start_matches('▍')
        .trim()
        .to_string()
}

/// OSC 52 序列：`ESC]52;c;<base64>(BEL|ST)`。SSH 远程同样生效（ADR 前端通道约定）。
pub fn osc52(content: &str) -> String {
    use base64_like::*;
    format!("\x1b]52;c;{};\x07", b64_encode(content.as_bytes()))
}

/// 依赖无关的 base64 标准字母表实现（零第三方，仅 OSC52 用途）。
pub mod base64_like {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn b64_encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(TABLE[(n >> 18) as usize & 63] as char);
            out.push(TABLE[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_cover_all_tokens_both_modes() {
        for mode in [Mode::Day, Mode::Night] {
            let p = palette(mode);
            assert_eq!(p.len(), 14);
            assert!(p.iter().all(|(_, hex)| hex.starts_with('#') && hex.len() == 7));
        }
    }

    #[test]
    fn day_night_differ_on_fg() {
        assert_ne!(color(Mode::Day, Token::Fg), color(Mode::Night, Token::Fg));
        assert_eq!(color(Mode::Day, Token::Destructive), "#B3261E");
    }

    #[test]
    fn copy_command_scope_is_pure() {
        let c = build_copy_content(CopyScope::Command, "$ rm -rf ./build\n", None, "全文");
        assert_eq!(c, "rm -rf ./build");
    }

    #[test]
    fn copy_translated_falls_back_to_command() {
        assert_eq!(build_copy_content(CopyScope::Translated, "ls", Some("Get-ChildItem"), ""), "Get-ChildItem");
        assert_eq!(build_copy_content(CopyScope::Translated, "ls", None, ""), "ls");
    }

    #[test]
    fn osc52_wraps_base64() {
        let seq = osc52("ls");
        assert!(seq.starts_with("\x1b]52;c;"));
        assert!(seq.ends_with("\x07"));
        assert_eq!(seq, "\x1b]52;c;bHM=;\x07");
    }
}

/// clap 值枚举（console --theme）。
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ModeArg {
    Day,
    Night,
}
