//! 全屏 Console UI 客户端（iac console，入口豁免）：iA-console 的主界面。
//! 设计语言见 _04/03（iA Writer 式）：内容即界面、无框线、留白、焦点行 accent、日夜主题。

use crate::theme::{CopyScope, Mode, Token};
use crate::tui::{copy_to_clipboard, render_output};
use iac_core::common::marker_parser::{classify, looks_like_command, InputDecision};

#[allow(unused_imports)]
use looks_like_command as _llc;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 一条界面记录：会话流里的每个可见块。
#[derive(Debug, Clone)]
pub enum Entry {
    /// 普通输出（已渲染文本，含提案/结果/诊断）。
    Output(String),
    /// 任务分隔（---）。
    Separator,
    /// 备注。
    Note(String),
    /// 系统提示（弱提醒/复制反馈等）。
    Notice(String),
}

/// Console 状态。
pub struct ConsoleState {
    pub mode: Mode,
    pub entries: Vec<Entry>,
    pub input: String,
    pub scroll: u16,
    pub last_command: String,
    pub last_translated: Option<String>,
    /// 上一条提案 id（供 iac run --proposal）。
    pub last_proposal: Option<String>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            mode: Mode::Night,
            entries: vec![Entry::Output(
                "iA Console v0.2.0 — 天下没有难用的 Console\n\n\
                 说人话或直接输入命令；??问 AI；ia-help 管理；--- 新任务段；# 备注；yc/yt/ya 复制；Ctrl+C 退出。"
                    .into(),
            )],
            input: String::new(),
            scroll: 0,
            last_command: String::new(),
            last_translated: None,
            last_proposal: None,
        }
    }
}

/// 主循环（入口豁免）：crossterm 事件 → 状态迁移 → ratatui 渲染。
pub fn run(mode_override: Option<Mode>) -> i32 {
    let mut state = ConsoleState::default();
    if let Some(m) = mode_override {
        state.mode = m;
    }
    let mut terminal = match setup() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("终端初始化失败: {e}");
            return 1;
        }
    };
    loop {
        let _ = terminal.draw(|f| draw(f, &state));
        match crossterm::event::read() {
            Ok(ev) => match ev {
                crossterm::event::Event::Key(key) => {
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    use crossterm::event::KeyCode::*;
                    match key.code {
                        Char('c')
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            break
                        }
                        Esc => break,
                        Enter => submit(&mut state),
                        Backspace => {
                            state.input.pop();
                        }
                        Up => state.scroll = state.scroll.saturating_add(1),
                        Down => state.scroll = state.scroll.saturating_sub(1),
                        PageUp => state.scroll = state.scroll.saturating_add(10),
                        PageDown => state.scroll = state.scroll.saturating_sub(10),
                        Char(c) => state.input.push(c),
                        _ => {}
                    }
                }
                crossterm::event::Event::Mouse(ev) => {
                    if let crossterm::event::MouseEventKind::ScrollUp = ev.kind {
                        state.scroll = state.scroll.saturating_add(3);
                    }
                    if let crossterm::event::MouseEventKind::ScrollDown = ev.kind {
                        state.scroll = state.scroll.saturating_sub(3);
                    }
                }
                _ => {}
            },
            Err(_) => break,
        }
    }
    teardown(terminal);
    0
}

/// 终端初始化：raw mode + 备用屏 + 鼠标滚轮。
fn setup() -> Result<ratatui::DefaultTerminal, Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    Ok(ratatui::Terminal::new(backend)?)
}

/// 终端还原。
fn teardown(terminal: ratatui::DefaultTerminal) {
    ratatui::restore();
    drop(terminal);
}

/// 提交输入：标记分发 → 本地处理或 IPC。
fn submit(state: &mut ConsoleState) {
    let line = state.input.trim().to_string();
    state.input.clear();
    if line.is_empty() {
        return;
    }
    if line == "yes" || line == "no" {
        confirm_last(state, line == "yes");
        return;
    }
    // 复制三档（AC-37）：yc/yt/ya
    if let Some(scope) = copy_scope_of(&line) {
        let content = crate::theme::build_copy_content(
            scope,
            &state.last_command,
            state.last_translated.as_deref(),
            &snapshot(state),
        );
        println!("{}", copy_to_clipboard(scope, &content));
        state
            .entries
            .push(Entry::Notice(format!("已复制（{} 字节）", content.len())));
        return;
    }
    let decision = classify(&line, looks_like_command);
    match decision {
        InputDecision::LiteralCommand(command) => {
            state.last_command = command.clone();
            let rendered = ipc_call(
                "iac_execute",
                serde_json::json!({"command": command, "origin": "direct"}),
            );
            state.entries.push(Entry::Output(rendered));
        }
        InputDecision::AiAsk { intent } => {
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into());
            let rendered = ipc_call("iac_ask", serde_json::json!({"intent": intent, "cwd": cwd}));
            if let Some(id) = extract_field(&rendered, "id") {
                state.last_proposal = Some(id.clone());
                state.entries.push(Entry::Notice(format!(
                    "提案 {} → 输入 yes 执行，no 拒绝",
                    id
                )));
            }
            state.entries.push(Entry::Output(rendered));
        }
        InputDecision::AiExplain { command } => {
            state.entries.push(Entry::Output(ipc_call(
                "iac_explain",
                serde_json::json!({"command": command}),
            )));
        }
        InputDecision::Builtin(call) => {
            // 本地处理子集；其余走共享 IPC 映射（Q28/Q30）
            match call.name.as_str() {
                "ia-theme" => {
                    state.mode = match state.mode {
                        Mode::Day => Mode::Night,
                        Mode::Night => Mode::Day,
                    };
                    state
                        .entries
                        .push(Entry::Notice(format!("主题已切换：{:?}", state.mode)));
                }
                "ia-exit" => state
                    .entries
                    .push(Entry::Notice("用 Ctrl+C 或 Esc 退出".into())),
                _ => {
                    if let Some(out) = builtin_output(&call) {
                        state.entries.push(Entry::Output(out));
                    }
                }
            }
        }
        InputDecision::TaskSeparator => {
            state.entries.push(Entry::Separator);
            let _ = crate::ipc::request(
                "iac_session_event",
                serde_json::json!({"kind": "separator"}),
                None,
            );
            state.entries.push(Entry::Notice("—— 新任务段 ——".into()));
        }
        InputDecision::Note(text) => {
            let _ = crate::ipc::request(
                "iac_session_event",
                serde_json::json!({"kind": "note", "payload": {"text": text}}),
                None,
            );
            state.entries.push(Entry::Note(text));
        }
    }
    state.scroll = 0;
}

/// 内置命令共享处理（GUI/CLI 共用；Q28/Q30 名单与 IPC 映射）。
/// 返回要展示的文本；本地处理的（ia-theme/ia-exit）由前端自理。
pub fn builtin_output(call: &iac_core::builtin::BuiltinCall) -> Option<String> {
    let args = &call.args;
    match call.name.as_str() {
        "ia-help" => Some(iac_core::builtin::help_text()),
        "ia-config" => match args.first().map(String::as_str) {
            Some("set") if args.len() >= 3 => Some(ipc_call(
                "iac_config_set",
                serde_json::json!({"key": args[1], "value": args[2]}),
            )),
            _ => Some(ipc_call("iac_config_get", serde_json::json!({}))),
        },
        "ia-model" => match args.first().map(String::as_str) {
            Some("use") if args.len() >= 2 => Some(ipc_call(
                "iac_config_set",
                serde_json::json!({"key": "ai.default_provider", "value": args[1]}),
            )),
            _ => Some(ipc_call("iac_config_get", serde_json::json!({"key": "ai"}))),
        },
        "ia-daemon" => match args.first().map(String::as_str) {
            Some("stop") => Some(ipc_call("iac_daemon_stop", serde_json::json!({}))),
            _ => {
                let alive = crate::ipc::request("iac_ping", serde_json::json!({}), None)
                    .map(|r| r.ok)
                    .unwrap_or(false);
                Some(if alive {
                    "daemon 运行中".into()
                } else {
                    "daemon 未运行（iac daemon start）".into()
                })
            }
        },
        "ia-history" => match args.first().map(String::as_str) {
            Some("clean") => Some(ipc_call(
                "iac_history_clean",
                serde_json::json!({"dry_run": args.iter().any(|a| a == "--dry-run" || a == "-n")}),
            )),
            Some("export") if args.len() >= 2 => Some(ipc_call(
                "iac_history_export",
                serde_json::json!({"path": args[1]}),
            )),
            _ => Some(ipc_call(
                "iac_history_query",
                serde_json::json!({"limit": 20}),
            )),
        },
        "ia-recipe" => match args.first().map(String::as_str) {
            Some("search") if args.len() >= 2 => Some(ipc_call(
                "iac_recipes_search",
                serde_json::json!({"q": args[1], "limit": 20}),
            )),
            Some("rm") if args.len() >= 2 => Some(ipc_call(
                "iac_recipe_delete",
                serde_json::json!({"name": args[1]}),
            )),
            Some("export") if args.len() >= 2 => Some(ipc_call(
                "iac_recipe_export",
                serde_json::json!({"path": args[1]}),
            )),
            _ => Some(ipc_call(
                "iac_recipes_search",
                serde_json::json!({"limit": 20}),
            )),
        },
        "ia-sessions" => match args.first().map(String::as_str) {
            Some("resume") if args.len() >= 2 => Some(ipc_call(
                "iac_session_resume",
                serde_json::json!({"session_id": args[1]}),
            )),
            _ => Some(ipc_call(
                "iac_session_list",
                serde_json::json!({"limit": 20}),
            )),
        },
        "ia-rollback" => match args.first().map(String::as_str) {
            Some("replay") => Some(ipc_call(
                "iac_rollback",
                serde_json::json!({
                    "action": "replay",
                    "range": {"to": args.get(1).and_then(|s| s.parse::<i64>().ok())},
                    "on_conflict": "ask"
                }),
            )),
            _ => Some(ipc_call(
                "iac_rollback",
                serde_json::json!({"action": "list", "filter": {"limit": 50}}),
            )),
        },
        "ia-file-history" => match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some(path), Some("restore")) => Some(ipc_call(
                "iac_rollback",
                serde_json::json!({"action": "restore", "path": path, "seq": args.get(2).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)}),
            )),
            (Some(path), _) => Some(ipc_call(
                "iac_rollback",
                serde_json::json!({"action": "file_history", "path": path}),
            )),
            _ => Some(
                iac_core::builtin::lookup("ia-file-history")
                    .map(|b| b.usage.to_string())
                    .unwrap_or_default(),
            ),
        },
        "ia-script" => match args.first().map(String::as_str) {
            Some("run") if args.len() >= 2 => Some(ipc_call(
                "iac_script",
                serde_json::json!({"action": "run", "name": args[1].trim_start_matches('@'), "confirm_dangerous": true}),
            )),
            Some("refine") if args.len() >= 2 => Some(ipc_call(
                "iac_script",
                serde_json::json!({"action": "refine", "name": args[1]}),
            )),
            _ => Some(ipc_call(
                "iac_script",
                serde_json::json!({"action": "make", "name": format!("script-{}", iac_core::session::now_ms()), "confirm": false}),
            )),
        },
        unknown => {
            // 未登记名单 → IAC-1501 列全部（Q30）
            Some(format!(
                "[IAC-1501] 未知内置命令 {unknown}。可用名单：\n{}",
                iac_core::builtin::help_text()
            ))
        }
    }
}

/// yes/no：对最近提案的确认执行/拒绝（提案卡确认流，AC-09）。
fn confirm_last(state: &mut ConsoleState, yes: bool) {
    let Some(pid) = state.last_proposal.clone() else {
        state
            .entries
            .push(Entry::Notice("当前没有待确认提案".into()));
        return;
    };
    if yes {
        state.entries.push(Entry::Output(ipc_call(
            "iac_execute",
            serde_json::json!({"proposal_id": pid, "confirm_dangerous": true, "origin": "proposal"}),
        )));
    } else {
        state
            .entries
            .push(Entry::Notice(format!("提案 {pid} 已拒绝")));
    }
}

/// IPC 调用并渲染为界面文本（人类摘要；错误原样带错误码）。
fn ipc_call(method: &str, params: serde_json::Value) -> String {
    match crate::ipc::request(method, params, None) {
        Ok(resp) => crate::ipc::render(&resp, false),
        Err(e) => e,
    }
}

/// 从渲染文本提取字段（提案 id 等；摘要渲染保持纯文本）。
fn extract_field(rendered: &str, field: &str) -> Option<String> {
    // 摘要渲染中 id 行形如 "提案 ID：<id>"
    if field == "id" {
        for line in rendered.lines() {
            if let Some(rest) = line.strip_prefix("    提案 ID：") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

/// 复制三档识别：输入 yc/yt/ya（AC-37 键位的输入行形态）。
fn copy_scope_of(line: &str) -> Option<CopyScope> {
    match line {
        "yc" => Some(CopyScope::Command),
        "yt" => Some(CopyScope::Translated),
        "ya" => Some(CopyScope::All),
        _ => None,
    }
}

/// 全量快照（复制 all 档）。
fn snapshot(state: &ConsoleState) -> String {
    state
        .entries
        .iter()
        .map(|e| match e {
            Entry::Output(t) | Entry::Note(t) | Entry::Notice(t) => t.clone(),
            Entry::Separator => "---".into(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 界面绘制：输出区（滚动）+ 输入行（焦点 accent）。
fn draw(f: &mut Frame, state: &ConsoleState) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let lines = render_entries(state);
    let total = lines.len() as u16;
    let visible = chunks[0].height;
    let scroll = state.scroll.min(total.saturating_sub(visible));
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), chunks[0]);
    // 输入行（焦点 accent + ⚠ 弱提醒占位由 submit 前的 gate 呈现）
    let prompt = Span::styled("▍ ", Style::default().fg(hex(state.mode, Token::Accent)));
    let input_span = Span::styled(
        state.input.clone(),
        Style::default()
            .fg(hex(state.mode, Token::Fg))
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![prompt, input_span])),
        chunks[1],
    );
    let cursor_x = 2 + state.input.chars().count() as u16;
    f.set_cursor_position((cursor_x.min(area.width.saturating_sub(1)), chunks[1].y));
}

/// 条目 → 渲染行（含结构化高亮接管，AC-41/42）。
fn render_entries(state: &ConsoleState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for entry in &state.entries {
        match entry {
            Entry::Separator => lines.push(Line::from(Span::styled(
                "· · ·".to_string(),
                style_of(state, Token::Fg25),
            ))),
            Entry::Note(text) => lines.push(Line::from(Span::styled(
                format!("# {text}"),
                style_of(state, Token::Fg40),
            ))),
            Entry::Notice(text) => lines.push(Line::from(Span::styled(
                text.clone(),
                style_of(state, Token::Caution),
            ))),
            Entry::Output(text) => lines.extend(render_output(&tui_view(state), text)),
        }
        lines.push(Line::from(Span::styled(String::new(), Style::default())));
    }
    lines
}

/// 复用 TuiState 的主题视图（渲染引擎按 mode 着色）。
fn tui_view(state: &ConsoleState) -> crate::tui::TuiState {
    crate::tui::TuiState {
        mode: state.mode,
        ..crate::tui::TuiState::default()
    }
}

fn style_of(state: &ConsoleState, token: Token) -> Style {
    Style::default().fg(hex(state.mode, token))
}

fn hex(mode: Mode, token: Token) -> Color {
    let (r, g, b) = crate::theme::rgb(mode, token);
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_scope_recognizes_three_keys() {
        assert_eq!(copy_scope_of("yc"), Some(CopyScope::Command));
        assert_eq!(copy_scope_of("yt"), Some(CopyScope::Translated));
        assert_eq!(copy_scope_of("ya"), Some(CopyScope::All));
        assert_eq!(copy_scope_of("ls"), None);
    }

    #[test]
    fn snapshot_collects_all_entries() {
        let state = ConsoleState {
            entries: vec![
                Entry::Output("第一段".into()),
                Entry::Separator,
                Entry::Note("备注".into()),
            ],
            ..ConsoleState::default()
        };
        let snap = snapshot(&state);
        assert_eq!(snap.lines().count(), 3, "三段 join 为 3 行");
        assert!(snap.contains("第一段") && snap.contains("---") && snap.contains("备注"));
    }

    #[test]
    fn extract_field_reads_proposal_id_line() {
        let rendered = "  ▍ ls\n\n    风险：safe\n    提案 ID：abc-123";
        assert_eq!(extract_field(rendered, "id").as_deref(), Some("abc-123"));
    }

    #[test]
    fn theme_toggle_via_classify() {
        // ia-theme 经混合识别进入本地处理（Q28 内置命名空间）
        let decision = classify("ia-theme", looks_like_command);
        assert!(matches!(decision, InputDecision::Builtin(_)));
    }
}
