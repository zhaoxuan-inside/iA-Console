//! 输入标记解析（I-034，algorithm）：?? / 命令 / --- / # / \ 转义（_06 external 第 6 节）。
//! 仅在 iA 输入框解析，不改变 shell 语义。

use crate::framework::{Context, Evolution, Metrics};

/// 用户输入行原文。
#[derive(Debug, Clone, Default)]
pub struct RawInput {
    pub line: String,
}

/// 解析决策（词典：InputMarker）。
#[derive(Debug, Clone, PartialEq)]
pub enum InputDecision {
    /// `??命令`：AI 解释命令。
    AiExplain { command: String },
    /// `??自然语言`：AI 提问生成提案。
    AiAsk { intent: String },
    /// `/名称 参数`：管理命令（/config 等）。
    Slash { name: String, args: String },
    /// `---`（独行）：任务分隔。
    TaskSeparator,
    /// `#文字`：会话备注。
    Note { text: String },
    /// `\` 转义后按字面执行。
    Literal { command: String },
    /// 普通命令原样通过。
    Command { command: String },
    /// 未知斜杠命令（调用方转 IAC-1501）。
    UnknownSlash { name: String },
}

/// I-034：标记分发（纯函数，恒 true；未知斜杠经 decision 表达）。
pub fn parse_input_marker(
    input: &RawInput,
    output: &mut InputDecision,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    let line = input.line.trim();
    *output = classify(line);
    metrics.mark("marker.kind", decision_tag(output));
    true
}

fn classify(line: &str) -> InputDecision {
    // 转义优先
    if let Some(rest) = line.strip_prefix('\\') {
        return InputDecision::Literal { command: rest.trim().to_string() };
    }
    // 任务分隔：整行仅 ---
    if line == "---" {
        return InputDecision::TaskSeparator;
    }
    // AI 唤起：??<内容>
    if let Some(rest) = line.strip_prefix("??") {
        let content = rest.trim();
        if content.is_empty() {
            return InputDecision::AiAsk { intent: String::new() };
        }
        // 首词像命令（无空格且非中文）→ 解释；否则提问
        let first = content.split_whitespace().next().unwrap_or_default();
        let looks_like_command = !content.contains(' ') && first.chars().all(|c| c.is_ascii_graphic());
        return if looks_like_command {
            InputDecision::AiExplain { command: content.to_string() }
        } else {
            InputDecision::AiAsk { intent: content.to_string() }
        };
    }
    // 备注：# 开头
    if let Some(rest) = line.strip_prefix('#') {
        return InputDecision::Note { text: rest.trim().to_string() };
    }
    // 管理命令：/ 开头
    if let Some(rest) = line.strip_prefix('/') {
        let mut parts = rest.splitn(2, ' ');
        let name = parts.next().unwrap_or_default().trim().to_string();
        let args = parts.next().unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return InputDecision::Command { command: line.to_string() };
        }
        return if KNOWN_SLASH.contains(&name.as_str()) {
            InputDecision::Slash { name, args }
        } else {
            InputDecision::UnknownSlash { name }
        };
    }
    InputDecision::Command { command: line.to_string() }
}

/// 已知管理命令（_06 external 标记表）。
pub const KNOWN_SLASH: &[&str] = &[
    "config", "help", "history", "recipe", "session", "model", "theme", "exit",
];

fn decision_tag(d: &InputDecision) -> &'static str {
    match d {
        InputDecision::AiExplain { .. } => "ai_explain",
        InputDecision::AiAsk { .. } => "ai_ask",
        InputDecision::Slash { .. } => "slash",
        InputDecision::TaskSeparator => "separator",
        InputDecision::Note { .. } => "note",
        InputDecision::Literal { .. } => "literal",
        InputDecision::Command { .. } => "command",
        InputDecision::UnknownSlash { .. } => "unknown_slash",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> InputDecision {
        let env = crate::framework::TestEnv::new();
        let input = RawInput { line: line.into() };
        let mut out = InputDecision::Command { command: String::new() };
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(parse_input_marker(&input, &mut out, &env.context(), &mut m, &mut e));
        out
    }

    #[test]
    fn ai_markers_split_explain_vs_ask() {
        assert_eq!(parse("??ls"), InputDecision::AiExplain { command: "ls".into() });
        assert_eq!(
            parse("??如何找出大文件"),
            InputDecision::AiAsk { intent: "如何找出大文件".into() }
        );
    }

    #[test]
    fn slash_note_separator_and_escape() {
        assert_eq!(parse("/config"), InputDecision::Slash { name: "config".into(), args: String::new() });
        assert_eq!(parse("/model set x"), InputDecision::Slash { name: "model".into(), args: "set x".into() });
        assert!(matches!(parse("/foo"), InputDecision::UnknownSlash { name } if name == "foo"));
        assert_eq!(parse("---"), InputDecision::TaskSeparator);
        assert_eq!(parse("# 排查磁盘"), InputDecision::Note { text: "排查磁盘".into() });
        assert_eq!(parse("\\??foo"), InputDecision::Literal { command: "??foo".into() });
    }

    #[test]
    fn plain_command_passthrough() {
        assert_eq!(
            parse("ls -la /var/log"),
            InputDecision::Command { command: "ls -la /var/log".into() }
        );
        // /config 不是独行 --- 或空斜杠时的普通路径不误判
        assert!(matches!(parse("./config --x"), InputDecision::Command { .. }));
    }
}
