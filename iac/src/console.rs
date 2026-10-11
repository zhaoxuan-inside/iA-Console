//! 全屏 Console UI 客户端（iac console，入口豁免）：iA-console 的主界面。
//! 设计语言见 _04/03（iA Writer 式）：内容即界面、无框线、留白、焦点行 accent、日夜主题。

use crate::theme::{CopyScope, Mode, Token};
use crate::tui::{copy_to_clipboard, render_output};
use iac_core::common::marker_parser::{parse_input_marker, InputDecision, RawInput};
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
                "iA Console v0.1.0 — 天下没有难用的 Console\n\n\
                 直接输入命令；??问 AI；/管理；--- 新任务段；# 备注；y 前缀复制；Ctrl+C 退出。"
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
                        Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => break,
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
        state.entries.push(Entry::Notice(format!("已复制（{} 字节）", content.len())));
        return;
    }
    let env = iac_core::framework::TestEnv::new();
    let mut decision = InputDecision::Command { command: String::new() };
    let mut m = iac_core::framework::Metrics::default();
    let mut evo = iac_core::framework::Evolution::default();
    let _ = parse_input_marker(&RawInput { line: line.clone() }, &mut decision, &env.context(), &mut m, &mut evo);
    match decision {
        InputDecision::Command { command } => {
            state.last_command = command.clone();
            let rendered = ipc_call("iac_execute", serde_json::json!({"command": command, "origin": "direct"}));
            state.entries.push(Entry::Output(rendered));
        }
        InputDecision::AiAsk { intent } => {
            let rendered = ipc_call("iac_ask", serde_json::json!({"intent": intent, "cwd": "."}));
            if let Some(id) = extract_field(&rendered, "id") {
                state.last_proposal = Some(id.clone());
                state.entries.push(Entry::Notice(format!("提案 {} → 输入 yes 执行，no 拒绝", id)));
            }
            state.entries.push(Entry::Output(rendered));
        }
        InputDecision::AiExplain { command } => {
            state.entries.push(Entry::Output(ipc_call("iac_explain", serde_json::json!({"command": command}))));
        }
        InputDecision::Slash { name, args } => handle_slash(state, &name, &args),
        InputDecision::TaskSeparator => {
            state.entries.push(Entry::Separator);
            state.entries.push(Entry::Notice("—— 新任务段 ——".into()));
        }
        InputDecision::Note { text } => state.entries.push(Entry::Note(text)),
        InputDecision::Literal { command } => {
            state.last_command = command.clone();
            state.entries.push(Entry::Output(ipc_call("iac_execute", serde_json::json!({"command": command, "origin": "direct"}))));
        }
        InputDecision::UnknownSlash { name } => {
            state.entries.push(Entry::Notice(format!("[IAC-1501] 未知管理命令 /{name}（/help 查看）")));
        }
    }
    state.scroll = 0;
}

/// yes/no：对最近提案的确认执行/拒绝（提案卡确认流，AC-09）。
fn confirm_last(state: &mut ConsoleState, yes: bool) {
    let Some(pid) = state.last_proposal.clone() else {
        state.entries.push(Entry::Notice("当前没有待确认提案".into()));
        return;
    };
    if yes {
        state.entries.push(Entry::Output(ipc_call(
            "iac_execute",
            serde_json::json!({"proposal_id": pid, "confirm_dangerous": true, "origin": "proposal"}),
        )));
    } else {
        state.entries.push(Entry::Notice(format!("提案 {pid} 已拒绝")));
    }
}

/// 管理命令（本地处理的子集；其余转 IPC/提示）。
fn handle_slash(state: &mut ConsoleState, name: &str, _args: &str) {
    match name {
        "help" => state.entries.push(Entry::Output(HELP_TEXT.into())),
        "theme" => {
            state.mode = match state.mode {
                Mode::Day => Mode::Night,
                Mode::Night => Mode::Day,
            };
            state.entries.push(Entry::Notice(format!("主题已切换：{:?}", state.mode)));
        }
        "exit" => state.entries.push(Entry::Notice("用 Ctrl+C 或 Esc 退出".into())),
        "history" => state.entries.push(Entry::Output(ipc_call("iac_history_query", serde_json::json!({"limit": 20})))),
        "recipe" => state.entries.push(Entry::Output(ipc_call("iac_recipes_search", serde_json::json!({"limit": 20})))),
        "sessions" => state.entries.push(Entry::Output(ipc_call("iac_sessions_current", serde_json::json!({"cwd": "."})))),
        _ => state.entries.push(Entry::Notice(format!("/{name} 随 v1.1 开放（当前支持 /help /theme /history /recipe /sessions /exit）"))),
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
    let input_span = Span::styled(state.input.clone(), Style::default().fg(hex(state.mode, Token::Fg)).add_modifier(Modifier::BOLD));
    f.render_widget(Paragraph::new(Line::from(vec![prompt, input_span])), chunks[1]);
    let cursor_x = 2 + state.input.chars().count() as u16;
    f.set_cursor_position((cursor_x.min(area.width.saturating_sub(1)), chunks[1].y));
}

/// 条目 → 渲染行（含结构化高亮接管，AC-41/42）。
fn render_entries(state: &ConsoleState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for entry in &state.entries {
        match entry {
            Entry::Separator => lines.push(Line::from(Span::styled("· · ·".to_string(), style_of(state, Token::Fg25)))),
            Entry::Note(text) => lines.push(Line::from(Span::styled(format!("# {text}"), style_of(state, Token::Fg40)))),
            Entry::Notice(text) => lines.push(Line::from(Span::styled(text.clone(), style_of(state, Token::Caution)))),
            Entry::Output(text) => lines.extend(render_output(&tui_view(state), text)),
        }
        lines.push(Line::from(Span::styled(String::new(), Style::default())));
    }
    lines
}

/// 复用 TuiState 的主题视图（渲染引擎按 mode 着色）。
fn tui_view(state: &ConsoleState) -> crate::tui::TuiState {
    crate::tui::TuiState { mode: state.mode, ..crate::tui::TuiState::default() }
}

fn style_of(state: &ConsoleState, token: Token) -> Style {
    Style::default().fg(hex(state.mode, token))
}

fn hex(mode: Mode, token: Token) -> Color {
    let (r, g, b) = crate::theme::rgb(mode, token);
    Color::Rgb(r, g, b)
}

const HELP_TEXT: &str = "iA Console — 标记与命令\n\n  直接输入      执行命令（闸门保护）\n  ??ls          AI 解释 ls\n  ??自然语言    AI 生成提案\n  yes / no      确认或拒绝最近提案\n  /help         本帮助\n  /theme        切换日夜主题\n  /history      最近执行\n  /recipe       方案库\n  /sessions     当前会话\n  ---           新任务段\n  # 文字        会话备注\n  yc / yt / ya  复制命令/译文/全部\n  \\命令         按字面执行\n  Ctrl+C / Esc  退出";

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
    fn theme_toggle_via_slash() {
        let mut state = ConsoleState::default();
        handle_slash(&mut state, "theme", "");
        assert_eq!(state.mode, Mode::Day);
    }
}
