//! 全屏 Console UI 客户端（iac console，入口豁免）：iA-console 的主界面。
//! 设计语言见 _04/03（iA Writer 式）：内容即界面、无框线、留白、焦点行 accent、日夜主题。
//!
//! 审计修复要点：滚动=距底部模型（0=跟随最新输出，上滚回看、方向与惯例一致）；
//! 提案确认走 confirm::ProposalFlow 结构化状态机（yes/no/确认词/[/] 切候选）；
//! Output 条目渲染缓存（主题变更后失效重算，非逐帧全量重高亮）；
//! ia-script 旗标透传不预置武装（与 core route_builtin 同语义）；会话恢复重绑+事件回灌。

use crate::confirm::{FlowLine, ProposalFlow};
use crate::theme::{rgb, str_width, CopyScope, Mode, Token};
use crate::tui::copy_to_clipboard;
use iac_core::common::marker_parser::{classify, looks_like_command, InputDecision};

#[allow(unused_imports)]
use looks_like_command as _llc;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::io::Write;

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
    /// 距底部行数（0=跟随最新输出；上滚增大、下滚回落，方向与惯例一致）。
    pub scroll_from_bottom: usize,
    pub last_command: String,
    pub last_translated: Option<String>,
    /// 剪贴板通道开关（启动时读 ui.clipboard；off=不写 OSC52）。
    pub clipboard_on: bool,
    /// 提案确认状态机（yes/no/确认词/候选切换）。
    pub proposal: ProposalFlow,
    /// Output 条目的渲染缓存（index 对齐 entries；主题不符时失效重算）。
    line_cache: Vec<Option<(Mode, Vec<Line<'static>>)>>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            mode: Mode::Night,
            entries: vec![Entry::Output(
                "iA Console v0.2.0 — 天下没有难用的 Console\n\n\
                 说人话或直接输入命令；??问 AI；ia-help 管理；--- 新任务段；# 备注；\n\
                 提案：yes 执行 / no 拒绝 / 「确认 <尾词>」/ [ ] 切候选 / Esc 取消；\n\
                 yc/yt/ya 复制；Ctrl+C 退出。"
                    .into(),
            )],
            input: String::new(),
            scroll_from_bottom: 0,
            last_command: String::new(),
            last_translated: None,
            clipboard_on: true,
            proposal: ProposalFlow::default(),
            line_cache: vec![None],
        }
    }
}

/// 条目上限：长会话的内存/渲染界（超限从最旧淘汰；界内窗口化渲染每帧只克隆可视行）。
const MAX_ENTRIES: usize = 4000;

/// 追加条目并保持渲染缓存对齐。
fn add_entry(state: &mut ConsoleState, entry: Entry) {
    state.entries.push(entry);
    state.line_cache.push(None);
    while state.entries.len() > MAX_ENTRIES {
        state.entries.remove(0);
        state.line_cache.remove(0);
    }
}

/// 追加输出块：跟随最新（距底=0）。
fn push_output(state: &mut ConsoleState, text: String) {
    add_entry(state, Entry::Output(text));
    state.scroll_from_bottom = 0;
}

/// 确认流产出 → 界面条目。
fn push_flow(state: &mut ConsoleState, lines: Vec<FlowLine>) {
    for line in lines {
        match line {
            FlowLine::Out(t) => push_output(state, t),
            FlowLine::Tip(t) => add_entry(state, Entry::Notice(t)),
        }
    }
}

/// 从 config 回包解析 ui 偏好（theme/clipboard；纯函数可测）。
/// day→Day；night/auto/缺省→Night（auto 跟随环境随重建落地）。
fn parse_ui_prefs(result: &serde_json::Value) -> (Mode, bool) {
    let ui = result.get("ui").cloned().unwrap_or_default();
    let mode = match ui.get("theme").and_then(|v| v.as_str()) {
        Some("day") => Mode::Day,
        _ => Mode::Night,
    };
    let clipboard_on = ui.get("clipboard").and_then(|v| v.as_str()) != Some("off");
    (mode, clipboard_on)
}

/// 启动时读 ui 段：主题 + 剪贴板通道开关（daemon 不可达时双缺省）。
pub(crate) fn config_ui() -> (Mode, bool) {
    match crate::ipc::request("iac_config_get", serde_json::json!({}), None) {
        Ok(r) if r.ok => parse_ui_prefs(r.result.as_ref().unwrap_or(&serde_json::Value::Null)),
        _ => (Mode::Night, true),
    }
}

/// 主循环（入口豁免）：crossterm 事件 → 状态迁移 → ratatui 渲染。
pub fn run(mode_override: Option<Mode>) -> i32 {
    let (cfg_mode, clipboard_on) = config_ui();
    let mut state = ConsoleState {
        mode: mode_override.unwrap_or(cfg_mode),
        clipboard_on,
        ..ConsoleState::default()
    };
    let mut terminal = match setup() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("终端初始化失败: {e}");
            return 1;
        }
    };
    loop {
        let _ = terminal.draw(|f| draw(f, &mut state));
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
                        Esc => {
                            // 有待确认提案先取消提案，否则退出
                            if state.proposal.has_pending() {
                                if let Some(lines) = state.proposal.handle("no") {
                                    push_flow(&mut state, lines);
                                }
                            } else {
                                break;
                            }
                        }
                        Enter => submit(&mut state),
                        Backspace => {
                            state.input.pop();
                        }
                        Up => state.scroll_from_bottom += 1,
                        Down => {
                            state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(1)
                        }
                        PageUp => state.scroll_from_bottom += 10,
                        PageDown => {
                            state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(10)
                        }
                        Char('[') if state.proposal.has_pending() => {
                            if let Some(cmd) = state.proposal.cycle_candidate(-1) {
                                add_entry(&mut state, Entry::Notice(format!("候选 → {cmd}")));
                            }
                        }
                        Char(']') if state.proposal.has_pending() => {
                            if let Some(cmd) = state.proposal.cycle_candidate(1) {
                                add_entry(&mut state, Entry::Notice(format!("候选 → {cmd}")));
                            }
                        }
                        Char(c) => state.input.push(c),
                        _ => {}
                    }
                }
                crossterm::event::Event::Mouse(ev) => match ev.kind {
                    crossterm::event::MouseEventKind::ScrollUp => state.scroll_from_bottom += 3,
                    crossterm::event::MouseEventKind::ScrollDown => {
                        state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(3)
                    }
                    _ => {}
                },
                _ => {}
            },
            Err(_) => break,
        }
    }
    teardown(terminal);
    0
}

/// 终端初始化：raw mode + 备用屏 + 鼠标滚轮（config_wizard 复用）。
pub(crate) fn setup() -> Result<ratatui::DefaultTerminal, Box<dyn std::error::Error>> {
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
pub(crate) fn teardown(terminal: ratatui::DefaultTerminal) {
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
    // 提案确认流优先：仅存在待确认提案时拦截 yes/no/确认词（否则 yes 落入常规执行）
    if let Some(lines) = state.proposal.handle(&line) {
        push_flow(state, lines);
        state.scroll_from_bottom = 0;
        return;
    }
    // 复制三档（AC-37）：yc/yt/ya
    if let Some(scope) = copy_scope_of(&line) {
        if !state.clipboard_on {
            add_entry(
                state,
                Entry::Notice(
                    "剪贴板通道已关闭（ui.clipboard=off；iac config → 界面 → 剪贴板 可开启）"
                        .into(),
                ),
            );
            state.scroll_from_bottom = 0;
            return;
        }
        if scope == CopyScope::Translated && state.last_translated.is_none() {
            add_entry(state, Entry::Notice("尚无译文，yt 回退为命令原文".into()));
        }
        let content = crate::theme::build_copy_content(
            scope,
            &state.last_command,
            state.last_translated.as_deref(),
            &snapshot(state),
        );
        // OSC52 序列直写（无换行）：序列不移动光标，不打乱 ratatui 的绘制状态
        let _ = std::io::stdout().write_all(copy_to_clipboard(scope, &content).as_bytes());
        let _ = std::io::stdout().flush();
        add_entry(
            state,
            Entry::Notice(format!("已复制（{} 字节）", content.len())),
        );
        state.scroll_from_bottom = 0;
        return;
    }
    let decision = classify(&line, looks_like_command);
    match decision {
        InputDecision::LiteralCommand(command) => {
            state.last_command = command.clone();
            push_output(
                state,
                ipc_call(
                    "iac_execute",
                    serde_json::json!({
                        "command": command, "origin": "direct",
                        "cwd": crate::ipc::client_cwd()
                    }),
                ),
            );
        }
        InputDecision::AiAsk { intent } => {
            // 结构化登记：提案 id / 活动候选命令取自回包，不从渲染文本反解析
            match crate::ipc::request(
                "iac_ask",
                serde_json::json!({"intent": intent, "cwd": crate::ipc::client_cwd()}),
                None,
            ) {
                Ok(resp) => {
                    let notice = resp
                        .result
                        .as_ref()
                        .and_then(|r| state.proposal.register(r));
                    if let Some(cmd) = state
                        .proposal
                        .pending
                        .as_ref()
                        .filter(|_| notice.is_some())
                        .map(|p| p.command().to_string())
                    {
                        state.last_command = cmd;
                    }
                    if let Some(n) = notice {
                        add_entry(state, Entry::Notice(n));
                    }
                    push_output(state, crate::ipc::render(&resp, false));
                }
                Err(e) => push_output(state, e),
            }
        }
        InputDecision::AiExplain { command } => {
            push_output(
                state,
                ipc_call("iac_explain", serde_json::json!({"command": command})),
            );
        }
        InputDecision::Builtin(call) => {
            // 本地处理子集；其余走共享 IPC 映射（Q28/Q30）
            match call.name.as_str() {
                "ia-theme" => {
                    state.mode = match state.mode {
                        Mode::Day => Mode::Night,
                        Mode::Night => Mode::Day,
                    };
                    add_entry(
                        state,
                        Entry::Notice(format!("主题已切换：{:?}", state.mode)),
                    );
                }
                "ia-exit" => add_entry(state, Entry::Notice("用 Ctrl+C 或 Esc 退出".into())),
                "ia-sessions" => ia_sessions(state, &call.args),
                _ => {
                    if let Some(out) = builtin_output(&call) {
                        push_output(state, out);
                    }
                }
            }
        }
        InputDecision::TaskSeparator => {
            add_entry(state, Entry::Separator);
            let _ = crate::ipc::request(
                "iac_session_event",
                serde_json::json!({"kind": "separator"}),
                None,
            );
            add_entry(state, Entry::Notice("—— 新任务段 ——".into()));
        }
        InputDecision::Note(text) => {
            let _ = crate::ipc::request(
                "iac_session_event",
                serde_json::json!({"kind": "note", "payload": {"text": text}}),
                None,
            );
            add_entry(state, Entry::Note(text));
        }
    }
    state.scroll_from_bottom = 0;
}

/// 会话恢复（Q46）：重绑进程级 session + 事件流回灌界面。
fn ia_sessions(state: &mut ConsoleState, args: &[String]) {
    match args.first().map(String::as_str) {
        Some("resume") if args.len() >= 2 => {
            let id = args[1].clone();
            match crate::ipc::request(
                "iac_session_resume",
                serde_json::json!({"session_id": id}),
                None,
            ) {
                Ok(r) if r.ok => {
                    crate::ipc::rebind_session(&id);
                    let events = r
                        .result
                        .as_ref()
                        .and_then(|v| v.get("events"))
                        .and_then(|e| e.as_array())
                        .cloned()
                        .unwrap_or_default();
                    add_entry(
                        state,
                        Entry::Notice(format!(
                            "已恢复会话 {}（{} 条事件，后续请求已重绑）",
                            id,
                            events.len()
                        )),
                    );
                    for ev in events {
                        let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        let payload = ev.get("payload").cloned().unwrap_or_default();
                        match kind {
                            "separator" => add_entry(state, Entry::Separator),
                            "note" => {
                                if let Some(t) = payload.get("text").and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        add_entry(state, Entry::Note(t.to_string()));
                                    }
                                }
                            }
                            "command" => {
                                if let Some(c) = payload.get("command").and_then(|v| v.as_str()) {
                                    push_output(state, c.to_string());
                                }
                            }
                            _ => {
                                let s = serde_json::to_string_pretty(&payload).unwrap_or_default();
                                if s != "null" {
                                    push_output(state, s);
                                }
                            }
                        }
                    }
                }
                Ok(r) => push_output(state, crate::ipc::render(&r, false)),
                Err(e) => push_output(state, e),
            }
        }
        _ => push_output(
            state,
            ipc_call("iac_session_list", serde_json::json!({"limit": 20})),
        ),
    }
}

/// 内置命令共享处理（GUI/CLI 共用；Q28/Q30 名单与 IPC 映射）。
/// 返回要展示的文本；本地处理的（ia-theme/ia-exit/ia-sessions）由前端自理。
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
            // chat 等无界面调用方：恢复即重绑（Q46 对同进程后续请求生效）
            Some("resume") if args.len() >= 2 => {
                let id = &args[1];
                match crate::ipc::request(
                    "iac_session_resume",
                    serde_json::json!({"session_id": id}),
                    None,
                ) {
                    Ok(r) if r.ok => {
                        crate::ipc::rebind_session(id);
                        Some(crate::ipc::render(&r, false))
                    }
                    Ok(r) => Some(crate::ipc::render(&r, false)),
                    Err(e) => Some(e),
                }
            }
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
            (Some(path), Some("restore")) => {
                match args.get(2).and_then(|s| s.parse::<i64>().ok()) {
                    Some(seq) => Some(ipc_call(
                        "iac_rollback",
                        serde_json::json!({"action": "restore", "path": path, "seq": seq}),
                    )),
                    None => Some("用法：ia-file-history <路径> restore <版本行号>".into()),
                }
            }
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
        "ia-script" => {
            // 与 core route_builtin 同语义：武装态/确认词随旗标透传（-y/--yes/--confirm-word），
            // 不预置 confirm_dangerous——自动武装会绕过 dangerous 确认
            let confirm_dangerous = args.iter().any(|a| a == "-y" || a == "--yes");
            let confirm_word = args
                .iter()
                .position(|a| a == "--confirm-word")
                .and_then(|i| args.get(i + 1))
                .cloned();
            match args.first().map(String::as_str) {
                Some("run") if args.len() >= 2 => Some(ipc_call(
                    "iac_script",
                    serde_json::json!({
                        "action": "run", "name": args[1].trim_start_matches('@'),
                        "confirm_dangerous": confirm_dangerous, "confirm_word": confirm_word,
                        "cwd": crate::ipc::client_cwd()
                    }),
                )),
                Some("refine") if args.len() >= 2 => Some(ipc_call(
                    "iac_script",
                    serde_json::json!({"action": "refine", "name": args[1]}),
                )),
                _ => Some(
                    "ia-script make 需要行号/任务段语义，经 CLI：iac script make <名> --segment last"
                        .into(),
                ),
            }
        }
        unknown => {
            // 未登记名单 → IAC-1501 列全部（Q30）
            Some(format!(
                "[IAC-1501] 未知内置命令 {unknown}。可用名单：\n{}",
                iac_core::builtin::help_text()
            ))
        }
    }
}

/// IPC 调用并渲染为界面文本（人类摘要；错误原样带错误码）。
fn ipc_call(method: &str, params: serde_json::Value) -> String {
    match crate::ipc::request(method, params, None) {
        Ok(resp) => crate::ipc::render(&resp, false),
        Err(e) => e,
    }
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

/// 顶部偏移换算（纯函数可测）：total 行、visible 高、距底 from_bottom；超 u16 截断。
fn top_offset(total: usize, visible: usize, from_bottom: usize) -> u16 {
    let max = total.saturating_sub(visible);
    let off = max.saturating_sub(from_bottom);
    u16::try_from(off).unwrap_or(u16::MAX)
}

/// 界面绘制：输出区（窗口化滚动）+ 输入行（焦点 accent；危险输入弱提醒）。
fn draw(f: &mut Frame, state: &mut ConsoleState) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let visible = chunks[0].height as usize;
    // 总行数（惰性物化全部 Output 缓存；此后帧内条目只读长度，不克隆行）
    let total: usize = (0..state.entries.len()).map(|i| entry_len(state, i)).sum();
    let scroll = top_offset(total, visible, state.scroll_from_bottom) as usize;
    let lines = collect_window(state, scroll, visible);
    f.render_widget(Paragraph::new(lines), chunks[0]);
    // 输入行（焦点 accent；超宽输入保持光标可见的横向窗口；
    // ▍ 为窄块元素：光标横坐标按「块 1 列 + 空格 1 列」= 2 计）
    let max_input = area.width.saturating_sub(3) as usize;
    let (shown, _cut) = window_tail(&state.input, max_input);
    let risk = input_risk(&state.input);
    f.render_widget(
        Paragraph::new(crate::tui::render_input(state.mode, shown, risk)),
        chunks[1],
    );
    let cursor_x = 2 + str_width(shown) as u16;
    f.set_cursor_position((cursor_x.min(area.width.saturating_sub(1)), chunks[1].y));
}

/// 物化 Output 条目的渲染缓存（主题不符即重算回填；其余类型即时构造不入缓存）。
fn ensure_output_cached(state: &mut ConsoleState, idx: usize) {
    let mode = state.mode;
    let fresh = state
        .line_cache
        .get(idx)
        .and_then(|c| c.as_ref())
        .map(|(m, _)| *m == mode)
        .unwrap_or(false);
    if fresh {
        return;
    }
    let Entry::Output(text) = &state.entries[idx] else {
        return;
    };
    let text = text.clone();
    let view = crate::tui::TuiState {
        mode,
        ..crate::tui::TuiState::default()
    };
    let ls = crate::tui::render_output(&view, &text);
    if idx < state.line_cache.len() {
        state.line_cache[idx] = Some((mode, ls));
    }
}

/// 条目 → 渲染行（Output 走缓存；AC-41/42 结构化高亮）。
fn entry_lines(state: &mut ConsoleState, idx: usize) -> Vec<Line<'static>> {
    match &state.entries[idx] {
        Entry::Separator => {
            return vec![Line::from(Span::styled(
                "· · ·".to_string(),
                style_of(state, Token::Fg25),
            ))]
        }
        Entry::Note(text) => {
            return vec![Line::from(Span::styled(
                format!("# {text}"),
                style_of(state, Token::Fg40),
            ))]
        }
        Entry::Notice(text) => {
            return vec![Line::from(Span::styled(
                text.clone(),
                style_of(state, Token::Caution),
            ))]
        }
        Entry::Output(_) => {}
    }
    ensure_output_cached(state, idx);
    state
        .line_cache
        .get(idx)
        .and_then(|c| c.as_ref())
        .map(|(_, ls)| ls.clone())
        .unwrap_or_default()
}

/// 条目总行数（含条目后的空行；Output 行数取自缓存，缺失先物化）。
fn entry_len(state: &mut ConsoleState, idx: usize) -> usize {
    let n = match &state.entries[idx] {
        Entry::Output(_) => {
            ensure_output_cached(state, idx);
            state
                .line_cache
                .get(idx)
                .and_then(|c| c.as_ref())
                .map(|(_, ls)| ls.len())
                .unwrap_or(1)
        }
        _ => 1,
    };
    n + 1
}

/// 收集可视窗口内的行（只物化/克隆命中条目；每条目行后接一条空行）。
/// skip 语义：仅窗口起点所在的条目需要跳过前缀，其后条目从头取（saturating 归零）。
fn collect_window(state: &mut ConsoleState, start: usize, len: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut pos = 0usize;
    for i in 0..state.entries.len() {
        if lines.len() >= len {
            break;
        }
        let n = entry_len(state, i);
        let entry_end = pos + n;
        if entry_end > start {
            let want = len - lines.len();
            let mut full = entry_lines(state, i);
            full.push(Line::from(Span::styled(String::new(), Style::default())));
            lines.extend(full.into_iter().skip(start.saturating_sub(pos)).take(want));
            if lines.len() >= len {
                break;
            }
        }
        pos = entry_end;
    }
    lines
}

/// 输入行风险预警（AC-04 弱提醒文字件）：直输命令命中 dangerous/destructive → 红字+⚠，不拦截。
fn input_risk(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    match classify(trimmed, looks_like_command) {
        InputDecision::LiteralCommand(cmd) => match iac_core::risk::assess_static(&cmd) {
            iac_core::types::RiskLevel::Dangerous => Some("dangerous"),
            iac_core::types::RiskLevel::Destructive => Some("destructive"),
            _ => None,
        },
        _ => None,
    }
}

/// 超宽输入的尾部窗口（保持光标可见）；返回 (可见串, 被裁列数)。
fn window_tail(s: &str, max_cols: usize) -> (&str, usize) {
    if str_width(s) <= max_cols {
        return (s, 0);
    }
    let mut cols = 0usize;
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        let cw = if c.is_ascii() { 1 } else { 2 };
        if cols + cw > max_cols {
            break;
        }
        cols += cw;
        start = i;
    }
    (&s[start..], str_width(&s[..start]))
}

fn style_of(state: &ConsoleState, token: Token) -> Style {
    Style::default().fg(hex(state.mode, token))
}

fn hex(mode: Mode, token: Token) -> Color {
    let (r, g, b) = rgb(mode, token);
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
    fn theme_toggle_via_classify() {
        // ia-theme 经混合识别进入本地处理（Q28 内置命名空间）
        let decision = classify("ia-theme", looks_like_command);
        assert!(matches!(decision, InputDecision::Builtin(_)));
    }

    #[test]
    fn top_offset_follows_bottom_and_respects_history() {
        // 0 = 跟随最新（贴底）
        assert_eq!(top_offset(100, 10, 0), 90);
        // 上滚 5 行：顶部偏移回退
        assert_eq!(top_offset(100, 10, 5), 85);
        // 滚过头贴顶
        assert_eq!(top_offset(100, 10, 500), 0);
        // 内容不足一屏
        assert_eq!(top_offset(5, 10, 0), 0);
        // 超 u16 行数不回绕
        assert_eq!(top_offset(100_000, 10, 0), u16::MAX);
    }

    #[test]
    fn window_tail_keeps_cursor_visible() {
        assert_eq!(window_tail("short", 10), ("short", 0));
        let (shown, cut) = window_tail("abcdefgh", 4);
        assert_eq!(shown, "efgh");
        assert_eq!(cut, 4);
        // 宽字符不劈半
        let (shown, _) = window_tail("主题测试内容", 5);
        assert!(str_width(shown) <= 5);
        assert!(shown.ends_with("容"));
    }

    #[test]
    fn window_collect_matches_full_and_slices() {
        let mut state = ConsoleState::default();
        add_entry(&mut state, Entry::Separator);
        add_entry(&mut state, Entry::Note("备注".into()));
        let total: usize = (0..state.entries.len())
            .map(|i| entry_len(&mut state, i))
            .sum();
        let joined = |ls: &[Line<'static>]| -> Vec<String> {
            ls.iter()
                .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
                .collect()
        };
        let full = joined(&collect_window(&mut state, 0, total));
        assert_eq!(full.len(), total, "全窗=总行数（含每条目后空行）");
        // 中部窗口切片与全窗逐行对齐
        let mid = joined(&collect_window(&mut state, 2, 3));
        assert_eq!(mid, &full[2..5], "窗口切片与全窗一致");
        // 零高窗口=空
        assert!(collect_window(&mut state, 0, 0).is_empty());
    }

    #[test]
    fn entries_capped_at_limit() {
        let mut state = ConsoleState::default();
        for i in 0..(MAX_ENTRIES + 10) {
            add_entry(&mut state, Entry::Notice(format!("n{i}")));
        }
        assert_eq!(state.entries.len(), MAX_ENTRIES);
        assert_eq!(state.line_cache.len(), MAX_ENTRIES, "缓存与条目保持对齐");
    }

    #[test]
    fn parse_ui_prefs_reads_theme_and_clipboard() {
        assert_eq!(
            parse_ui_prefs(&serde_json::json!({"ui": {"theme": "day", "clipboard": "osc52"}})),
            (Mode::Day, true)
        );
        assert_eq!(
            parse_ui_prefs(&serde_json::json!({"ui": {"theme": "auto", "clipboard": "off"}})),
            (Mode::Night, false),
            "auto 暂映射 Night；off=关闭剪贴板通道"
        );
        assert_eq!(
            parse_ui_prefs(&serde_json::Value::Null),
            (Mode::Night, true)
        );
    }

    #[test]
    fn input_risk_warns_on_dangerous_direct_command() {
        assert_eq!(input_risk("rm -rf ./build"), Some("dangerous"));
        assert_eq!(input_risk("ls -la"), None);
        assert_eq!(input_risk(""), None);
        assert_eq!(input_risk("??rm -rf"), None, "问 AI 路径不做直输预警");
    }
}
