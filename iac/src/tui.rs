//! TUI 骨架（T-028/T-031/T-033/T-035，入口豁免）：单列主轴 + 标记分发 + 主题/复制。
//! 设计语言见 _04/03：内容即界面、无框线、留白、焦点行 accent。

use crate::confirm::FlowLine;
use crate::theme::{rgb, CopyScope, Mode, Token};
use iac_core::common::marker_parser::{classify, looks_like_command, InputDecision};
use iac_core::common::render::{detect_format, highlight, HighlightInput, TextFormat};
use iac_core::ports::RawText;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 会话状态（前端持有，业务在 daemon）。
pub struct TuiState {
    pub mode: Mode,
    pub history: Vec<String>,
    pub input: String,
    pub last_command: String,
    pub last_translated: Option<String>,
    /// 剪贴板通道开关（启动时读 ui.clipboard；off=不写 OSC52）。
    pub clipboard_on: bool,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            mode: Mode::Night,
            history: Vec::new(),
            input: String::new(),
            last_command: String::new(),
            last_translated: None,
            clipboard_on: true,
        }
    }
}

/// token → ratatui Color（查表取色，无逐帧 hex 解析）。
pub fn styled(mode: Mode, token: Token, bold: bool) -> Style {
    let (r, g, b) = rgb(mode, token);
    Style::default()
        .fg(Color::Rgb(r, g, b))
        .add_modifier(if bold {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

/// 输入行渲染：普通 fg；dangerous/destructive 命中红字+⚠
/// （弱提醒三件套之文字件，AC-04；console 输入行实时预警）。
pub fn render_input(mode: Mode, input: &str, risk_level: Option<&str>) -> Line<'static> {
    let dangerous = matches!(risk_level, Some("dangerous") | Some("destructive"));
    let mut spans = vec![Span::styled(
        "▍ ".to_string(),
        styled(mode, Token::Accent, false),
    )];
    if dangerous {
        spans.push(Span::styled(
            input.to_string(),
            styled(mode, Token::Destructive, true),
        ));
        spans.push(Span::styled(
            " ⚠".to_string(),
            styled(mode, Token::Destructive, false),
        ));
    } else {
        spans.push(Span::styled(
            input.to_string(),
            styled(mode, Token::Fg, true),
        ));
    }
    Line::from(spans)
}

/// 结果渲染：格式检测 + 高亮 + 语法令牌上色（AC-41/42；不改变内容）。
pub fn render_output(state: &TuiState, text: &str) -> Vec<Line<'static>> {
    let env = iac_core::framework::TestEnv::new();
    let mut format = TextFormat::Plain;
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let input = RawText {
        content: text.to_string(),
        hint: None,
    };
    let _ = detect_format(&input, &mut format, &env.context(), &mut m, &mut evo);
    let spans_in = {
        let hi = HighlightInput {
            text: text.to_string(),
            format: format.clone(),
        };
        let mut out = iac_core::common::render::StyledSpans::default();
        let _ = highlight(&hi, &mut out, &env.context(), &mut m, &mut evo);
        out
    };
    // span → 行：按真实行起点偏移归属；span 先裁剪到行内再按起点排序，
    // span 之间的空隙补 Fg 段——core 高亮只产出 token 级稀疏 span，不补洞会丢字
    // （JSON 丢空格、YAML 丢值、代码丢非关键字词）。
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line_text in body.split('\n') {
        let line_start = offset;
        let line_end = line_start + line_text.len();
        offset = line_end + 1; // 越过 '\n'
                               // 裁剪：跨行 span（如含换行的注释段）截到本行，而不是整体丢弃
        let mut clipped: Vec<(usize, usize, &iac_core::common::render::TokenKind)> = spans_in
            .spans
            .iter()
            .filter_map(|(s, e, kind)| {
                let s = (*s).max(line_start);
                let e = (*e).min(line_end);
                (e > s).then_some((s, e, kind))
            })
            .collect();
        clipped.sort_by_key(|(s, _, _)| *s);
        let mut spans = Vec::new();
        let mut pos = line_start;
        for (s, e, kind) in clipped {
            if s > pos {
                spans.push(Span::styled(
                    line_text[pos - line_start..s - line_start].to_string(),
                    styled(state.mode, Token::Fg, false),
                ));
            }
            if e > pos {
                let token = match kind {
                    iac_core::common::render::TokenKind::Keyword => Token::SynKeyword,
                    iac_core::common::render::TokenKind::Str => Token::SynString,
                    iac_core::common::render::TokenKind::Comment
                    | iac_core::common::render::TokenKind::Punct => Token::Fg25,
                    iac_core::common::render::TokenKind::Number => Token::Fg60,
                };
                let from = s.max(pos);
                spans.push(Span::styled(
                    line_text[from - line_start..e - line_start].to_string(),
                    styled(state.mode, token, false),
                ));
                pos = e;
            }
        }
        if pos < line_end {
            spans.push(Span::styled(
                line_text[pos - line_start..].to_string(),
                styled(state.mode, Token::Fg, false),
            ));
        }
        if spans.is_empty() {
            spans.push(Span::styled(
                line_text.to_string(),
                styled(state.mode, Token::Fg, false),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// 输入决策 → IPC 方法与参数（标记语义落地；cwd 随客户端透传，Q45）。
pub fn decision_to_request(
    decision: &InputDecision,
    cwd: &str,
) -> Option<(&'static str, serde_json::Value)> {
    Some(match decision {
        InputDecision::AiExplain { command } => {
            ("iac_explain", serde_json::json!({"command": command}))
        }
        InputDecision::AiAsk { intent } => {
            ("iac_ask", serde_json::json!({"intent": intent, "cwd": cwd}))
        }
        InputDecision::LiteralCommand(command) => (
            "iac_execute",
            serde_json::json!({"command": command, "origin": "direct", "cwd": cwd}),
        ),
        _ => return None,
    })
}

/// OSC52 输出（真实写入终端由入口层执行；此处返回序列）。
pub fn copy_to_clipboard(scope: CopyScope, content: &str) -> String {
    let _ = scope;
    crate::theme::osc52(content)
}

/// 精简 TUI 主循环（行模式 chat）：Enter 执行（标记分发）；q 退出；提案确认流；
/// ia-theme/ia-exit 本地自理；yc/yt/ya 复制；cwd 随客户端透传（Q45）。
pub fn run_chat(json_mode: bool) -> i32 {
    use std::io::Write;
    let (_, clipboard_on) = crate::console::config_ui();
    let mut state = TuiState {
        clipboard_on,
        ..TuiState::default()
    };
    let mut proposal = crate::confirm::ProposalFlow::default();
    let cwd = crate::ipc::client_cwd();
    println!(
        "iA-console chat（Enter 执行 / \\转义 / ??问 AI / ia-help 管理 / --- 分段 / # 备注 / yes·no·确认词 确认提案 / yc·yt·ya 复制 / q 退出）"
    );
    loop {
        print!("▍ ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "q" || line == "quit" {
            break;
        }
        // 复制三档（AC-37）：精确 yc/yt/ya；无译文时 yt 回退命令原文并提示
        let scope = match line.as_str() {
            "yc" => Some(CopyScope::Command),
            "yt" => Some(CopyScope::Translated),
            "ya" => Some(CopyScope::All),
            _ => None,
        };
        if let Some(scope) = scope {
            if !state.clipboard_on {
                println!("剪贴板通道已关闭（ui.clipboard=off；iac config → 界面 → 剪贴板 可开启）");
                continue;
            }
            if scope == CopyScope::Translated && state.last_translated.is_none() {
                println!("尚无译文，yt 回退为命令原文");
            }
            let content = crate::theme::build_copy_content(
                scope,
                &state.last_command,
                state.last_translated.as_deref(),
                &state.history.join("\n"),
            );
            // 序列直写（无换行；OSC52 不移动光标，行模式终端安全）
            print!("{}", copy_to_clipboard(scope, &content));
            std::io::stdout().flush().ok();
            println!("\n已复制（{} 字节）", content.len());
            continue;
        }
        // 提案确认流（yes/no/确认词；仅存在待确认提案时拦截）
        if let Some(lines) = proposal.handle(&line) {
            for l in lines {
                match l {
                    FlowLine::Out(t) => {
                        state.history.push(t.clone());
                        println!("{t}");
                    }
                    FlowLine::Tip(t) => println!("{t}"),
                }
            }
            continue;
        }
        let decision = classify(&line, looks_like_command);
        if let InputDecision::LiteralCommand(cmd) = &decision {
            state.last_command = cmd.clone();
            // 弱提醒三件套之文字件（AC-04，行模式形态）：提示不拦截
            match iac_core::risk::assess_static(cmd) {
                iac_core::types::RiskLevel::Dangerous => {
                    println!("⚠ dangerous：该命令影响面较大，请留意")
                }
                iac_core::types::RiskLevel::Destructive => {
                    println!("⚠ destructive：该命令不可逆，请留意")
                }
                _ => {}
            }
        }
        match decision_to_request(&decision, &cwd) {
            None => match &decision {
                InputDecision::TaskSeparator => {
                    state.history.push("---".into());
                    let _ = crate::ipc::request(
                        "iac_session_event",
                        serde_json::json!({"kind": "separator"}),
                        None,
                    );
                    println!("—— 新任务段 ——");
                }
                InputDecision::Note(text) => {
                    state.history.push(format!("# {text}"));
                    let _ = crate::ipc::request(
                        "iac_session_event",
                        serde_json::json!({"kind": "note", "payload": {"text": text}}),
                        None,
                    );
                }
                InputDecision::Builtin(call) => match call.name.as_str() {
                    // 本地自理（Q28）：ia-exit 退出、ia-theme 行模式无渲染主题
                    "ia-exit" => break,
                    "ia-theme" => {
                        println!("行模式无主题渲染（全屏界面用 iac console / iac config）")
                    }
                    _ => {
                        if let Some(out) = crate::console::builtin_output(call) {
                            state.history.push(out.clone());
                            println!("{out}");
                        }
                    }
                },
                _ => {}
            },
            Some((method, params)) => match crate::ipc::request(method, params, None) {
                Ok(resp) => {
                    // AiAsk：结构化登记提案（不从渲染文本反解析）
                    if method == "iac_ask" {
                        if let Some(r) = resp.result.as_ref() {
                            if proposal.register(r).is_some() {
                                if let Some(p) = &proposal.pending {
                                    state.last_command = p.command().to_string();
                                }
                            }
                        }
                    }
                    let rendered = crate::ipc::render(&resp, json_mode);
                    state.history.push(rendered.clone());
                    println!("{rendered}");
                }
                Err(msg) => println!("{msg}"),
            },
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_input_renders_warning_marker() {
        let line = render_input(Mode::Night, "rm -rf ./build", Some("dangerous"));
        let joined: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(joined.contains("⚠"));
        assert_eq!(joined, "▍ rm -rf ./build ⚠");
    }

    #[test]
    fn decision_maps_to_ipc_methods() {
        let d = classify("??ls", looks_like_command);
        let (method, _) = decision_to_request(&d, ".").unwrap();
        assert_eq!(method, "iac_explain");
    }

    #[test]
    fn literal_command_carries_client_cwd() {
        let d = classify("ls -la", looks_like_command);
        let (method, params) = decision_to_request(&d, "/tmp/work").unwrap();
        assert_eq!(method, "iac_execute");
        assert_eq!(params["cwd"], "/tmp/work");
        assert_eq!(params["origin"], "direct");
    }

    #[test]
    fn copy_scopes_follow_ac37() {
        // yc：命令原文剥提示符
        let content = crate::theme::build_copy_content(CopyScope::Command, "$ ls", None, "");
        assert_eq!(content, "ls", "纯净规则剥提示符");
        // yt：有译文取译文，无译文回退命令原文
        assert_eq!(
            crate::theme::build_copy_content(
                CopyScope::Translated,
                "ls",
                Some("Get-ChildItem"),
                ""
            ),
            "Get-ChildItem"
        );
        assert_eq!(
            crate::theme::build_copy_content(CopyScope::Translated, "ls", None, ""),
            "ls"
        );
        // ya：历史快照行拼接
        assert_eq!(
            crate::theme::build_copy_content(CopyScope::All, "", None, "a\nb"),
            "a\nb"
        );
    }

    #[test]
    fn render_output_keeps_json_spacing() {
        let state = TuiState::default();
        let text = r#"{"model": "gpt-4", "n": 0.7}"#;
        let joined: String = render_output(&state, text)[0]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        assert_eq!(joined, text, "JSON token 间空格不得丢失");
    }

    #[test]
    fn render_output_keeps_yaml_value() {
        let state = TuiState::default();
        let lines = render_output(&state, "name: value\nother: 42\n");
        let joined: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        assert_eq!(
            joined,
            vec!["name: value".to_string(), "other: 42".to_string()],
            "YAML 键后内容不得丢失"
        );
    }

    #[test]
    fn render_output_keeps_code_text() {
        let state = TuiState::default();
        let text = "```shell\ngit commit -m \"msg\"\n```";
        let joined: Vec<String> = render_output(&state, text)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        assert!(
            joined
                .iter()
                .any(|l| l.contains("git commit") && l.contains("-m")),
            "代码行非关键字词不得丢失：{joined:?}"
        );
    }

    #[test]
    fn render_output_attributes_spans_by_real_line_offsets() {
        // 第二行文本是第一行的子串扩展：旧实现 match_indices 定位到 0 会错位
        let state = TuiState::default();
        let lines = render_output(&state, "abc\nxabc");
        assert_eq!(lines.len(), 2);
        let second: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert_eq!(second, "xabc", "第二行 span 拼接须覆盖整行原文");
        let first: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert_eq!(first, "abc");
    }

    #[test]
    fn render_output_handles_trailing_newline_and_empty_lines() {
        let state = TuiState::default();
        let lines = render_output(&state, "a\n\nb\n");
        assert_eq!(lines.len(), 3, "尾随换行不产生空行");
        let joined: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        assert_eq!(
            joined,
            vec!["a".to_string(), String::new(), "b".to_string()]
        );
    }
}
