//! 输入分流（域 1/13，Q15/Q24~Q30）：混合识别——自然语言自动走 AI、命令形态直接执行、
//! `??` 强制 AI（仅行首）、`\` 强制字面、`ia-` 内置命令、`---` 任务段、`#` 会话备注。
//! 纯函数：命令可解析性由调用方注入（PATH/内建检查），本模块不做 IO。

use crate::builtin::BuiltinCall;

/// 单行输入的分流决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputDecision {
    /// 直接执行：`\` 剥前缀字面，或首词可解析为命令（Q15 命令形态）。
    LiteralCommand(String),
    /// 自然语言 → AI 生成提案（Q15；`??<自然语言>` 同此）。
    AiAsk { intent: String },
    /// `??<命令形态>` → AI 解释命令（AC-25 语义，行首限定 Q24）。
    AiExplain { command: String },
    /// `ia-xxx` 内置命令（Q28；未知名单交 dispatch 报 IAC-1501）。
    Builtin(BuiltinCall),
    /// 整行 `---`：任务段分隔（AI 上下文窗口不跨越最近分界，Q26）。
    TaskSeparator,
    /// 行首 `#`：会话备注（并入对话轨，可钉住，Q29）。
    Note(String),
}

/// 混合识别分流（Q15 主成功流第一步）。
///
/// `is_command`：首词是否可解析为可执行命令（PATH 扫描 + shell 内建名单，由前端注入；
/// 保持纯函数可测）。规则按优先级：
/// 1. 行首 `\` → 剥前缀字面（转义本义，Q30）；
/// 2. 整行 `---` → TaskSeparator（Q26）；
/// 3. 行首 `??` → 首词像命令 → AiExplain，否则 AiAsk（Q24 行首特权）；
/// 4. 行首 `#` → Note（shell 注释本义保留，Q29）；
/// 5. 行首 `ia-` → Builtin（Q28）；
/// 6. 其余：is_command(首词) → LiteralCommand，否则 AiAsk（自然语言自动走 AI）。
pub fn classify(line: &str, is_command: impl Fn(&str) -> bool) -> InputDecision {
    let trimmed = line.trim();
    // 1) `\` 强制字面
    if let Some(rest) = trimmed.strip_prefix('\\') {
        let rest = rest.trim();
        if rest.is_empty() {
            return InputDecision::LiteralCommand(line.to_string());
        }
        return InputDecision::LiteralCommand(rest.to_string());
    }
    // 2) `---` 任务分隔
    if trimmed == "---" {
        return InputDecision::TaskSeparator;
    }
    // 3) `??` 仅行首有特权（Q24）
    if let Some(rest) = trimmed.strip_prefix("??") {
        let rest = rest.trim();
        if let Some(first) = rest.split_whitespace().next() {
            if is_command(first) {
                return InputDecision::AiExplain {
                    command: rest.to_string(),
                };
            }
        }
        return InputDecision::AiAsk {
            intent: rest.to_string(),
        };
    }
    // 4) `#` 备注（行首）
    if let Some(rest) = trimmed.strip_prefix('#') {
        return InputDecision::Note(rest.trim().to_string());
    }
    // 5) `ia-` 内置命令（行首；不在总表的也上交 dispatch 报 IAC-1501）
    if trimmed
        .split_whitespace()
        .next()
        .is_some_and(|w| w.starts_with("ia-"))
    {
        if let Some(call) = crate::builtin::parse(trimmed) {
            return InputDecision::Builtin(call);
        }
    }
    // 6) 混合识别：命令形态直执，自然语言走 AI（Q15）
    match trimmed.split_whitespace().next() {
        Some(first) if is_command(first) => InputDecision::LiteralCommand(trimmed.to_string()),
        Some(_) => InputDecision::AiAsk {
            intent: trimmed.to_string(),
        },
        None => InputDecision::AiAsk {
            intent: String::new(),
        },
    }
}

/// 命令形态判定（PATH 扫描 + shell 内建名单；GUI/CLI 共用口径，混合识别 Q15 的 is_command 注入实现）。
pub fn looks_like_command(word: &str) -> bool {
    use std::sync::OnceLock;
    static PATH_DIRS: OnceLock<Vec<std::path::PathBuf>> = OnceLock::new();
    const BUILTINS: &[&str] = &[
        "cd", "pushd", "popd", "export", "unset", "alias", "source", "exit", "history", "set",
        "bg", "fg", "jobs", "wait", "read", "ulimit", "umask", "type", "echo", "true", "false",
    ];
    if BUILTINS.contains(&word) {
        return true;
    }
    if word.contains('/') {
        return std::path::Path::new(word).exists();
    }
    let dirs = PATH_DIRS.get_or_init(|| {
        std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    });
    dirs.iter().any(|d| d.join(word).exists())
}

/// 环境变量声明解析（Q46：`export K=V` / `set K=V`；仅行首声明生效）。
pub fn parse_env_declaration(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("export ")
        .or_else(|| trimmed.strip_prefix("set "))?;
    let (key, value) = rest.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    Some((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用命令判定：白名单 + 路径形。
    fn known(cmd: &str) -> bool {
        matches!(
            cmd,
            "ls" | "cd" | "git" | "rm" | "echo" | "docker" | "export" | "set"
        ) || cmd.starts_with("./")
            || cmd.starts_with('/')
    }

    #[test]
    fn natural_language_goes_to_ai() {
        assert_eq!(
            classify("怎么找大文件", known),
            InputDecision::AiAsk {
                intent: "怎么找大文件".into()
            }
        );
        assert_eq!(
            classify("帮我压缩这个目录", known),
            InputDecision::AiAsk {
                intent: "帮我压缩这个目录".into()
            }
        );
    }

    #[test]
    fn command_form_executes_directly() {
        assert_eq!(
            classify("ls -la", known),
            InputDecision::LiteralCommand("ls -la".into())
        );
        assert_eq!(
            classify("./deploy.sh prod", known),
            InputDecision::LiteralCommand("./deploy.sh prod".into())
        );
    }

    #[test]
    fn double_question_forces_ai_line_start_only() {
        assert_eq!(
            classify("??怎么查端口占用", known),
            InputDecision::AiAsk {
                intent: "怎么查端口占用".into()
            }
        );
        assert_eq!(
            classify("??ls", known),
            InputDecision::AiExplain {
                command: "ls".into()
            }
        );
        // 行中 `??` 保持通配本义（Q24）
        assert_eq!(
            classify("find . -name '??x'", known),
            InputDecision::AiAsk {
                intent: "find . -name '??x'".into()
            }
        );
    }

    #[test]
    fn backslash_forces_literal() {
        assert_eq!(
            classify("\\rm -rf x", known),
            InputDecision::LiteralCommand("rm -rf x".into())
        );
        assert_eq!(
            classify("\\??foo", known),
            InputDecision::LiteralCommand("??foo".into())
        );
        assert_eq!(
            classify("\\ia-help", known),
            InputDecision::LiteralCommand("ia-help".into())
        );
    }

    #[test]
    fn separator_and_note_and_builtin() {
        assert_eq!(classify("---", known), InputDecision::TaskSeparator);
        assert_eq!(
            classify("# 这段在调 nginx", known),
            InputDecision::Note("这段在调 nginx".into())
        );
        assert_eq!(
            classify("ia-help", known),
            InputDecision::Builtin(BuiltinCall {
                name: "ia-help".into(),
                args: vec![]
            })
        );
        assert_eq!(
            classify("ia-rollback list", known),
            InputDecision::Builtin(BuiltinCall {
                name: "ia-rollback".into(),
                args: vec!["list".into()]
            })
        );
    }

    #[test]
    fn env_declarations_parse() {
        assert_eq!(
            parse_env_declaration("export FOO=bar"),
            Some(("FOO".into(), "bar".into()))
        );
        assert_eq!(
            parse_env_declaration("set PATH=/usr/bin"),
            Some(("PATH".into(), "/usr/bin".into()))
        );
        assert_eq!(
            parse_env_declaration("export FOO=\"quoted value\""),
            Some(("FOO".into(), "quoted value".into()))
        );
        assert_eq!(parse_env_declaration("export BAD-KEY=1"), None);
        assert_eq!(parse_env_declaration("echo export FOO=1"), None);
    }
}
