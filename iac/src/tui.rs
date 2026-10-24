//! TUI 骨架（T-028/T-031/T-033/T-035，入口豁免）：单列主轴 + 标记分发 + 主题/复制。
//! 设计语言见 _04/03：内容即界面、无框线、留白、焦点行 accent。

use crate::theme::{color, CopyScope, Mode, Token};
use iac_core::common::marker_parser::{classify, looks_like_command, InputDecision};
use iac_core::common::render::{detect_format, highlight, HighlightInput, TextFormat};
use iac_core::ports::RawText;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// 会话状态（前端持有，业务在 daemon）。
pub struct TuiState {
    pub mode: Mode,
    pub history: Vec<String>,
    pub input: String,
    pub last_command: String,
    pub last_translated: Option<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            mode: Mode::Night,
            history: Vec::new(),
            input: String::new(),
            last_command: String::new(),
            last_translated: None,
        }
    }
}

/// token → ratatui Color。
pub fn styled(mode: Mode, token: Token, bold: bool) -> Style {
    let hex = color(mode, token);
    let (r, g, b) = parse_hex(hex);
    Style::default()
        .fg(Color::Rgb(r, g, b))
        .add_modifier(if bold {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

fn parse_hex(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
    )
}

/// 输入行渲染：普通 fg；dangerous 命中红字+⚠（弱提醒三件套之文字件，AC-04）。
pub fn render_input(state: &TuiState, risk_level: Option<&str>) -> Line<'static> {
    let dangerous = matches!(risk_level, Some("dangerous") | Some("destructive"));
    let mut spans = vec![Span::styled(
        "▍ ".to_string(),
        styled(state.mode, Token::Accent, false),
    )];
    if dangerous {
        spans.push(Span::styled(
            state.input.clone(),
            styled(state.mode, Token::Destructive, true),
        ));
        spans.push(Span::styled(
            " ⚠".to_string(),
            styled(state.mode, Token::Destructive, false),
        ));
    } else {
        spans.push(Span::styled(
            state.input.clone(),
            styled(state.mode, Token::Fg, true),
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
    // span → 行（简单按 \n 切分着色）
    let mut lines = Vec::new();
    for (i, line_text) in text.lines().enumerate() {
        let line_start = text
            .match_indices(line_text)
            .nth(i)
            .map(|(p, _)| p)
            .unwrap_or(0);
        let line_end = line_start + line_text.len();
        let mut spans = Vec::new();
        for (s, e, kind) in &spans_in.spans {
            if *s >= line_start && *e <= line_end && e > s {
                let token = match kind {
                    iac_core::common::render::TokenKind::Keyword => Token::SynKeyword,
                    iac_core::common::render::TokenKind::Str => Token::SynString,
                    iac_core::common::render::TokenKind::Comment
                    | iac_core::common::render::TokenKind::Punct => Token::Fg25,
                    iac_core::common::render::TokenKind::Number => Token::Fg60,
                };
                spans.push(Span::styled(
                    line_text[*s - line_start..*e - line_start].to_string(),
                    styled(state.mode, token, false),
                ));
            }
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

/// 输入决策 → IPC 方法与参数（标记语义落地）。
pub fn decision_to_request(decision: &InputDecision) -> Option<(&'static str, serde_json::Value)> {
    Some(match decision {
        InputDecision::AiExplain { command } => {
            ("iac_explain", serde_json::json!({"command": command}))
        }
        InputDecision::AiAsk { intent } => {
            ("iac_ask", serde_json::json!({"intent": intent, "cwd": "."}))
        }
        InputDecision::LiteralCommand(command) => (
            "iac_execute",
            serde_json::json!({"command": command, "origin": "direct"}),
        ),
        _ => return None,
    })
}

/// 复制键位组装（y 前缀：c/t/a，AC-37）。
pub fn copy_on_key(state: &TuiState, key: char) -> Option<(CopyScope, String)> {
    match key {
        'c' => Some((CopyScope::Command, state.last_command.clone())),
        't' => state
            .last_translated
            .as_ref()
            .map(|t| (CopyScope::Translated, t.clone())),
        'a' => Some((CopyScope::All, state.history.join("\n"))),
        _ => None,
    }
}

/// OSC52 输出（真实写入终端由入口层执行；此处返回序列）。
pub fn copy_to_clipboard(scope: CopyScope, content: &str) -> String {
    let _ = scope;
    crate::theme::osc52(content)
}

/// 历史行渲染（T-031 面板）：普通段落 + 结构化高亮（T-035 接管）。
pub fn render_history(state: &TuiState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for entry in &state.history {
        lines.extend(render_output(state, entry));
        lines.push(Line::from(Span::styled(String::new(), Style::default())));
    }
    lines
}

/// 段落小部件便捷构造（T-028 骨架用）。
pub fn paragraph(state: &TuiState) -> Paragraph<'static> {
    let mut lines = render_history(state);
    lines.push(render_input(state, None));
    Paragraph::new(lines)
}

/// 精简 TUI 主循环（T-028/T-031 骨架；完整多面板随 P8 后迭代）。
/// 键位：Enter 执行（标记分发）；Esc/q 退出；y 前缀复制（AC-37）。
pub fn run_chat(json_mode: bool) -> i32 {
    use std::io::Write;
    let mut state = TuiState::default();
    println!("iA-console chat（Enter 执行 / \\转义 / ??问 AI / ia-help 管理 / --- 分段 / # 备注 / q 退出）");
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
        // 复制键位（y + c/t/a）
        if let Some(rest) = line.strip_prefix('y') {
            if let Some((scope, content)) = copy_on_key(&state, rest.chars().next().unwrap_or(' '))
            {
                let seq = copy_to_clipboard(scope, &content);
                print!(
                    "\x1b]52;c;{};\x07",
                    seq.trim_start_matches("\x1b]52;c;")
                        .trim_end_matches('\x07')
                );
                std::io::stdout().flush().ok();
                println!("已复制（{} 字节）", content.len());
                continue;
            }
        }
        let decision = classify(&line, looks_like_command);
        match decision_to_request(&decision) {
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
                InputDecision::Builtin(call) => {
                    if let Some(out) = crate::console::builtin_output(call) {
                        state.history.push(out.clone());
                        println!("{out}");
                    }
                }
                _ => {}
            },
            Some((method, params)) => match crate::ipc::request(method, params, None) {
                Ok(resp) => {
                    let rendered = crate::ipc::render(&resp, json_mode);
                    state.history.push(rendered.clone());
                    if let Some(cmd) = resp
                        .result
                        .as_ref()
                        .and_then(|r| r.get("command"))
                        .and_then(|c| c.as_str())
                    {
                        state.last_command = cmd.to_string();
                    }
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
        let state = TuiState {
            input: "rm -rf ./build".into(),
            ..TuiState::default()
        };
        let line = render_input(&state, Some("dangerous"));
        let joined: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(joined.contains("⚠"));
        assert_eq!(joined, "▍ rm -rf ./build ⚠");
    }

    #[test]
    fn decision_maps_to_ipc_methods() {
        let d = classify("??ls", looks_like_command);
        let (method, _) = decision_to_request(&d).unwrap();
        assert_eq!(method, "iac_explain");
    }

    #[test]
    fn copy_scopes_follow_ac37() {
        let state = TuiState {
            last_command: "$ ls".into(),
            last_translated: Some("Get-ChildItem".into()),
            history: vec!["a".into(), "b".into()],
            ..TuiState::default()
        };
        let (scope, raw) = copy_on_key(&state, 'c').unwrap();
        let content = crate::theme::build_copy_content(scope, &raw, None, "");
        assert_eq!(content, "ls", "纯净规则剥提示符");
        let (scope2, content2) = copy_on_key(&state, 't').unwrap();
        assert_eq!(content2, "Get-ChildItem");
        let _ = scope2;
        let (_, all) = copy_on_key(&state, 'a').unwrap();
        assert_eq!(all, "a\nb");
        assert!(copy_on_key(&state, 'x').is_none());
    }
}
