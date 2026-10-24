//! ia-console 主应用（域 18 重建）：多窗口 × 窗口内标签 × 每标签独立终端会话（Q39），
//! 启动恢复全部布局与状态（Q41），daemon 自动拉起（Q40），主输入行直驱（Q7/Q15），
//! 提案卡多候选 + 确认升级（Q17/Q35~Q37），管理功能终端面内 TUI 风格渲染（Q38）。

use crate::ipc;
use crate::layout::{self, Layout, TabLayout, WindowLayout};
use crate::theme::{self, Mode, Token};
use egui_term::{
    AnsiColor, AnsiNamedColor, AnsiRgb, BackendCommand, BackendSettings, PtyEvent, TerminalBackend,
    TerminalTheme, TerminalView,
};
use iac_core::common::marker_parser::{classify, looks_like_command, InputDecision};
use iac_core::server::IpcResponse;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// IPC 响应回传信封：(目标标签 id, 方法名, 结果/错误)。
type IpcReply = (u64, &'static str, Result<IpcResponse, String>);

/// 提案卡确认升级状态机（Q35~Q37）。
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmState {
    /// 普通态：Enter 直接确认。
    Idle,
    /// 武装态（dangerous）：展示警告摘要，再次 Enter 确认（Q21）。
    Armed,
    /// 确认词态（destructive）：输入行变为确认词捕获（Q37）。
    AwaitWord,
}

/// 任务流条目。
#[derive(Debug, Clone)]
pub enum Entry {
    InputEcho(String),
    /// TUI 风格输出（管理面板/历史/方案列表的等宽文本）。
    Output(String),
    Notice(String),
    Note {
        text: String,
        pinned: bool,
    },
    Separator,
    /// 提案卡（多候选，Q17）。
    Proposal {
        id: String,
        candidates: Vec<(String, String, String)>,
        active: usize,
        context_json: Option<String>,
        expanded: bool,
    },
    /// 失败兜底（Q18）：执行失败 → 就地「要不要 AI 帮忙？」。
    OfferHelp {
        execution_id: String,
    },
    /// AI 失效（Q19）：具体原因 + 可重试。
    AiError {
        method: &'static str,
        params: serde_json::Value,
        reason: String,
    },
}

/// 单标签状态：独立终端会话 + 任务流 + 提案确认态。
pub struct Tab {
    pub id: u64,
    pub title: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub backend: TerminalBackend,
    pub pty_rx: Receiver<(u64, PtyEvent)>,
    pub entries: Vec<Entry>,
    pub input: String,
    pub busy: bool,
    pub busy_request: Option<String>,
    pub input_history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub last_proposal: Option<String>,
    pub active_proposal: Option<String>,
    pub confirm_state: ConfirmState,
    pub expanded: HashSet<String>,
    pub sidebar_open: bool,
    pub dialogue: Vec<(String, String)>,
}

impl Tab {
    fn new(id: u64, ctx: egui::Context, cwd: &str, session_id: Option<String>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = TerminalBackend::new(
            id,
            ctx,
            tx,
            // TERM/COLORTERM：默认 dumb 会破坏 starship 等现代提示符（首启体验）
            BackendSettings {
                shell: system_shell(),
                env: [
                    ("TERM".to_string(), "xterm-256color".to_string()),
                    ("COLORTERM".to_string(), "truecolor".to_string()),
                ]
                .into(),
                ..Default::default()
            },
        )
        .expect("终端后端初始化");
        // 恢复 cwd（Q41/Q46）：PTY 以 shell 默认目录启动后注入 cd
        if !cwd.is_empty() && cwd != "." {
            backend.process_command(BackendCommand::Write(
                format!("cd '{cwd}' && clear\n").into_bytes(),
            ));
        }
        Self {
            id,
            title: if cwd.is_empty() {
                "sh".into()
            } else {
                format!("sh · {}", short_cwd(cwd))
            },
            cwd: cwd.into(),
            session_id,
            backend,
            pty_rx: rx,
            entries: vec![],
            input: String::new(),
            busy: false,
            busy_request: None,
            input_history: vec![],
            history_cursor: None,
            last_proposal: None,
            active_proposal: None,
            confirm_state: ConfirmState::Idle,
            expanded: HashSet::new(),
            sidebar_open: false,
            dialogue: vec![],
        }
    }

    /// PTY 事件流：PtyWrite/颜色查询须应答，否则交互程序卡查询。
    fn drain_pty(&mut self, ctx: &egui::Context, theme: &TerminalTheme) {
        while let Ok((_, ev)) = self.pty_rx.try_recv() {
            match ev {
                PtyEvent::PtyWrite(text) => {
                    self.backend
                        .process_command(BackendCommand::Write(text.into_bytes()));
                }
                PtyEvent::ColorRequest(index, formatter) => {
                    let reply = formatter(query_color(theme, index));
                    self.backend
                        .process_command(BackendCommand::Write(reply.into_bytes()));
                }
                PtyEvent::ClipboardStore(_, text) => ctx.copy_text(text),
                _ => {}
            }
        }
    }
}

/// 主应用：多窗口（viewport immediate）+ 主题 + 布局持久化。
pub struct ConsoleApp {
    pub theme_pref: iac_core::types::ThemeMode,
    pub mode: Mode,
    pub windows: Vec<WindowState>,
    pub next_tab_id: u64,
    pub ipc_tx: Sender<IpcReply>,
    pub ipc_rx: Receiver<IpcReply>,
    pub term_theme: TerminalTheme,
}

/// 单窗口状态（域 18：每窗口独立 OS 窗口，含多标签）。
pub struct WindowState {
    pub viewport: egui::ViewportId,
    pub bounds: [f32; 4],
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl ConsoleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let (ipc_tx, ipc_rx) = std::sync::mpsc::channel();
        let saved = layout::load();
        let theme_pref = saved
            .as_ref()
            .map(|l| match l.theme.as_str() {
                "day" => iac_core::types::ThemeMode::Day,
                "night" => iac_core::types::ThemeMode::Night,
                _ => iac_core::types::ThemeMode::Auto,
            })
            .unwrap_or_default();
        let mode = resolve_mode(theme_pref, cc.egui_ctx.clone());
        let term_theme = TerminalTheme::new(Box::new(theme::terminal_palette(mode)));
        let mut app = Self {
            theme_pref,
            mode,
            windows: vec![],
            next_tab_id: 0,
            ipc_tx,
            ipc_rx,
            term_theme,
        };
        // 启动恢复（Q41）：全部窗口与标签；无布局 → 单窗口单标签
        match saved {
            Some(l) if !l.windows.is_empty() => {
                for (i, w) in l.windows.into_iter().enumerate() {
                    let mut ws = WindowState {
                        viewport: egui::ViewportId(egui::Id::new(i)),
                        bounds: w.bounds,
                        tabs: vec![],
                        active: 0,
                    };
                    for t in w.tabs {
                        ws.tabs.push(app.make_tab(&cc.egui_ctx, &t));
                    }
                    if ws.tabs.is_empty() {
                        ws.tabs
                            .push(app.make_tab(&cc.egui_ctx, &TabLayout::default()));
                    }
                    app.windows.push(ws);
                }
            }
            _ => {
                let mut ws = WindowState {
                    viewport: egui::ViewportId(egui::Id::new(0)),
                    bounds: [0.0, 0.0, 1280.0, 820.0],
                    tabs: vec![],
                    active: 0,
                };
                ws.tabs
                    .push(app.make_tab(&cc.egui_ctx, &TabLayout::default()));
                app.windows.push(ws);
            }
        }
        let welcome = windows0_mut(&mut app);
        if let Some(w) = welcome {
            if let Some(tab) = w.tabs.get_mut(w.active) {
                tab.entries.push(Entry::Notice(WELCOME.into()));
            }
        }
        theme::apply_style(&cc.egui_ctx, mode);
        app
    }

    fn make_tab(&mut self, ctx: &egui::Context, t: &TabLayout) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        Tab::new(id, ctx.clone(), &t.cwd, t.session_id.clone())
    }

    /// IPC 响应落对应标签（每帧 drain，UI 永不等待）。
    fn drain_ipc(&mut self) {
        while let Ok((tab_id, method, resp)) = self.ipc_rx.try_recv() {
            let Some(tab) = self.tab_by_id_mut(tab_id) else {
                continue;
            };
            apply_ipc_result(tab, method, resp);
        }
    }

    fn tab_by_id_mut(&mut self, tab_id: u64) -> Option<&mut Tab> {
        self.windows
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .find(|t| t.id == tab_id)
    }

    /// 布局持久化（Q41：退出与变更时）。
    pub fn persist_layout(&self) {
        let l = Layout {
            theme: match self.theme_pref {
                iac_core::types::ThemeMode::Day => "day".into(),
                iac_core::types::ThemeMode::Night => "night".into(),
                iac_core::types::ThemeMode::Auto => "auto".into(),
            },
            windows: self
                .windows
                .iter()
                .map(|w| WindowLayout {
                    bounds: w.bounds,
                    tabs: w
                        .tabs
                        .iter()
                        .map(|t| TabLayout {
                            title: t.title.clone(),
                            cwd: t.cwd.clone(),
                            session_id: t.session_id.clone(),
                        })
                        .collect(),
                })
                .collect(),
        };
        let _ = layout::save(&l);
    }
}

fn windows0_mut(app: &mut ConsoleApp) -> Option<&mut WindowState> {
    app.windows.get_mut(0)
}

pub const WELCOME: &str = concat!(
    "iA Console v",
    env!("CARGO_PKG_VERSION"),
    " — 天下没有难用的 Console\n\n",
    "说人话或直接输入命令；??问 AI；ia-help 管理命令；--- 新任务段；# 备注（可钉住）；\n",
    "Ctrl+Shift+N 新窗口 / Ctrl+Shift+T 新标签 / Ctrl+B 对话轨 / Ctrl+Shift+W 关标签。"
);

impl eframe::App for ConsoleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 主题解析（auto 跟随系统，AC-28）
        let resolved = resolve_mode(self.theme_pref, ctx.clone());
        if resolved != self.mode {
            self.mode = resolved;
            self.term_theme = TerminalTheme::new(Box::new(theme::terminal_palette(self.mode)));
            theme::apply_style(ctx, self.mode);
        }
        for w in &mut self.windows {
            for t in &mut w.tabs {
                t.drain_pty(ctx, &self.term_theme);
            }
        }
        self.drain_ipc();
        // 全局键位（新窗口/标签/侧栏）
        self.handle_global_keys(ctx);

        // 主窗口（viewport 0）作为本帧 primary
        if let Some(primary) = self.windows.first() {
            let primary_id = primary.viewport;
            let bounds = primary.bounds;
            let idx = 0;
            let _ = primary_id;
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = bounds;
                self.render_window(ui, idx);
            });
        }
        // 其余窗口：immediate viewport（每帧同步渲染，可直接借用 &mut self）
        for i in 1..self.windows.len() {
            let bounds = self.windows[i].bounds;
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("iA Console — 窗口 {}", i + 1))
                .with_inner_size([bounds[2], bounds[3]]);
            ctx.show_viewport_immediate(
                egui::ViewportId(egui::Id::new(i)),
                builder,
                |_ctx, _class| {
                    // immediate viewport 的回调上下文为 &Context：渲染经 CentralPanel 挂到该 viewport
                    egui::CentralPanel::default().show(_ctx, |ui| {
                        self.render_window(ui, i);
                    });
                },
            );
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_layout();
    }
}

impl ConsoleApp {
    /// 全局键位：新窗口/新标签/关标签/侧栏/主题。
    fn handle_global_keys(&mut self, ctx: &egui::Context) {
        let (shift, ctrl) = ctx.input(|i| (i.modifiers.shift, i.modifiers.ctrl));
        if ctx.input(|i| i.key_pressed(egui::Key::N)) && ctrl && shift {
            let ctx2 = ctx.clone();
            let tab = self.make_tab_viewport(&ctx2);
            self.windows.push(WindowState {
                viewport: egui::ViewportId(egui::Id::new(self.windows.len())),
                bounds: [0.0, 0.0, 1280.0, 820.0],
                tabs: vec![tab],
                active: 0,
            });
        }
        if ctx.input(|i| i.key_pressed(egui::Key::T)) && ctrl && shift {
            let ctx2 = ctx.clone();
            let tab = self.make_tab_viewport(&ctx2);
            let Some(active_window) = active_window_index(ctx) else {
                return;
            };
            if let Some(w) = self.windows.get_mut(active_window) {
                w.tabs.push(tab);
                w.active = w.tabs.len() - 1;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::W)) && ctrl && shift {
            let Some(active_window) = active_window_index(ctx) else {
                return;
            };
            if let Some(w) = self.windows.get_mut(active_window) {
                if w.tabs.len() > 1 {
                    w.tabs.remove(w.active);
                    w.active = w.active.saturating_sub(1);
                } else if self.windows.len() > 1 {
                    self.windows.remove(active_window);
                }
                self.persist_layout();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::B)) && ctrl {
            let Some(active_window) = active_window_index(ctx) else {
                return;
            };
            if let Some(w) = self.windows.get_mut(active_window) {
                if let Some(tab) = w.tabs.get_mut(w.active) {
                    tab.sidebar_open = !tab.sidebar_open;
                }
            }
        }
    }

    /// viewport 内新建标签（自包含构造，避免借用冲突）。
    fn make_tab_viewport(&mut self, ctx: &egui::Context) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        Tab::new(id, ctx.clone(), "", None)
    }

    /// 单窗口渲染：标签栏 + 任务条区（TUI 风格）+ 真终端 + 主输入行（+可选对话轨侧栏）。
    fn render_window(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let term_theme = self.term_theme.clone();
        // 激活窗口跟踪（键盘动作落在焦点所在窗口）
        let window_active = ui.input(|i| i.viewport().focused.unwrap_or(false));
        if window_active {
            LAST_FOCUSED_WINDOW.with(|c| c.set(window_idx));
        }
        // 标签栏
        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let titles: Vec<String> = match self.windows.get(window_idx) {
                        Some(w) => w.tabs.iter().map(|t| t.title.clone()).collect(),
                        None => vec![],
                    };
                    let active = self.windows.get(window_idx).map(|w| w.active).unwrap_or(0);
                    for (i, title) in titles.iter().enumerate() {
                        let selected = i == active;
                        let text = if selected {
                            egui::RichText::new(title).strong()
                        } else {
                            egui::RichText::new(title)
                        };
                        if ui.selectable_label(selected, text.monospace()).clicked() {
                            if let Some(w) = self.windows.get_mut(window_idx) {
                                w.active = i;
                            }
                        }
                    }
                    if ui
                        .selectable_label(false, egui::RichText::new("＋ 新标签").monospace())
                        .clicked()
                    {
                        let ctx2 = ui.ctx().clone();
                        let tab = self.make_tab_viewport(&ctx2);
                        if let Some(w) = self.windows.get_mut(window_idx) {
                            w.tabs.push(tab);
                            w.active = w.tabs.len() - 1;
                        }
                    }
                });
            });
        // 底部输入行（先占位）
        egui::TopBottomPanel::bottom("input")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)))
            .show_inside(ui, |ui| {
                ui.separator();
                self.draw_input_line(ui, window_idx);
            });
        // 可选对话轨侧栏（Q7/Q31 辅助视图）
        if let Some(w) = self.windows.get(window_idx) {
            if w.tabs.get(w.active).is_some_and(|t| t.sidebar_open) {
                egui::SidePanel::right("dialogue")
                    .resizable(true)
                    .default_width(280.0)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 8)))
                    .show_inside(ui, |ui| {
                        ui.label(egui::RichText::new("对话轨（辅助视图）").monospace().weak());
                        ui.separator();
                        self.draw_sidebar(ui, window_idx);
                    });
            }
        }
        // 主区：任务条区（可折叠）+ 真终端
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 0)))
            .show_inside(ui, |ui| {
                self.draw_task_strip(ui, window_idx);
                self.draw_terminal(ui, window_idx, &term_theme, mode);
            });
    }

    /// 任务条区（域 18：管理功能全部在终端面内渲染，TUI 风格条目）。
    fn draw_task_strip(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        let Some(w) = self.windows.get_mut(window_idx) else {
            return;
        };
        let active = w.active;
        let Some(tab) = w.tabs.get_mut(active) else {
            return;
        };
        // iA 调性：条区不用容器框——留白与字重分层（发丝线已是最重装饰）
        egui::CollapsingHeader::new(
            egui::RichText::new("任务条（提案 / AI / 管理）")
                .monospace()
                .size(11.0)
                .color(c(Token::Fg40)),
        )
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let entries = tab.entries.clone();
                    let confirm = tab.confirm_state.clone();
                    let busy = tab.busy;
                    for entry in &entries {
                        match entry {
                            Entry::InputEcho(line) => {
                                ui.label(
                                    egui::RichText::new(format!("▍ {line}"))
                                        .monospace()
                                        .color(c(Token::Fg25)),
                                );
                            }
                            Entry::Output(text) => {
                                // TUI 风格：等宽、无框线
                                ui.label(egui::RichText::new(text).monospace().color(c(Token::Fg)));
                            }
                            Entry::Notice(text) => {
                                ui.label(
                                    egui::RichText::new(text).monospace().color(c(Token::Fg25)),
                                );
                            }
                            Entry::Note { text, pinned } => {
                                let tag = if *pinned { "📌" } else { "" };
                                ui.label(
                                    egui::RichText::new(format!("# {text} {tag}"))
                                        .monospace()
                                        .color(c(Token::Fg60)),
                                );
                            }
                            Entry::Separator => {
                                ui.separator();
                            }
                            Entry::Proposal {
                                id,
                                candidates,
                                active: act,
                                context_json,
                                expanded,
                            } => {
                                draw_proposal_card(
                                    ui,
                                    &c,
                                    id,
                                    candidates,
                                    *act,
                                    context_json.as_deref(),
                                    *expanded,
                                    &confirm,
                                );
                            }
                            Entry::OfferHelp { execution_id } => {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("命令失败了——要不要 AI 帮忙？")
                                            .monospace()
                                            .color(c(Token::Destructive)),
                                    );
                                    if ui
                                        .selectable_label(
                                            false,
                                            egui::RichText::new("[问 AI]").monospace(),
                                        )
                                        .clicked()
                                    {
                                        let (tid, sid) = (tab.id, tab.session_id.clone());
                                        let exec = execution_id.clone();
                                        let tx = self.ipc_tx.clone();
                                        let _ = std::thread::Builder::new().spawn(move || {
                                            let resp = ipc::ensure_daemon_and_request(
                                                "iac_diagnose",
                                                serde_json::json!({"execution_id": exec}),
                                                sid,
                                            );
                                            let _ = tx.send((tid, "iac_diagnose", resp));
                                        });
                                    }
                                });
                            }
                            Entry::AiError {
                                method: _,
                                params,
                                reason,
                            } => {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("AI 不可用：{reason}"))
                                            .monospace()
                                            .color(c(Token::Destructive)),
                                    );
                                    let retry = params.clone();
                                    if ui
                                        .selectable_label(
                                            false,
                                            egui::RichText::new("[重试]").monospace(),
                                        )
                                        .clicked()
                                    {
                                        let (tid, sid) = (tab.id, tab.session_id.clone());
                                        let tx = self.ipc_tx.clone();
                                        let method: &'static str = "iac_ask";
                                        let _ = std::thread::Builder::new().spawn(move || {
                                            let resp =
                                                ipc::ensure_daemon_and_request(method, retry, sid);
                                            let _ = tx.send((tid, method, resp));
                                        });
                                    }
                                });
                            }
                        }
                        ui.add_space(3.0);
                    }
                    if busy {
                        let dots = if ui.input(|i| (i.time as u64).is_multiple_of(2)) {
                            "· ·"
                        } else {
                            "· · ·"
                        };
                        ui.label(egui::RichText::new(dots).monospace().color(c(Token::Fg25)));
                    }
                });
        });
    }

    /// 真终端（域 18 主体）。
    fn draw_terminal(
        &mut self,
        ui: &mut egui::Ui,
        window_idx: usize,
        term_theme: &TerminalTheme,
        _mode: Mode,
    ) {
        let Some(w) = self.windows.get_mut(window_idx) else {
            return;
        };
        let active = w.active;
        let Some(tab) = w.tabs.get_mut(active) else {
            return;
        };
        let term = TerminalView::new(ui, &mut tab.backend)
            .set_focus(true)
            .set_theme(term_theme.clone())
            .set_size(ui.available_size());
        ui.add(term);
    }

    /// 主输入行（Q7 直驱；生成期锁定 + Esc 取消 Q22；↑↓ 历史）。
    fn draw_input_line(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        let pending_line = {
            let Some(w) = self.windows.get_mut(window_idx) else {
                return;
            };
            let active = w.active;
            let Some(tab) = w.tabs.get_mut(active) else {
                return;
            };
            let hint = match tab.confirm_state {
                ConfirmState::AwaitWord => "输入确认词（见任务条提示）…",
                _ if tab.busy => "AI 处理中（Esc 取消）…",
                _ => "说人话 / 直接命令 / ??问 AI / ia-help",
            };
            let response = ui.add(
                egui::TextEdit::singleline(&mut tab.input)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
            let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
            if esc && tab.busy {
                // 生成期取消（Q22）：登记取消 + 立即解锁
                if let Some(rid) = tab.busy_request.take() {
                    let rid2 = rid.clone();
                    std::thread::Builder::new()
                        .spawn(move || ipc::cancel(&rid2))
                        .ok();
                }
                tab.busy = false;
                tab.entries
                    .push(Entry::Notice("已取消（迟到结果将丢弃）".into()));
            }
            if up && !tab.input_history.is_empty() {
                let idx = match tab.history_cursor {
                    Some(i) => i.saturating_sub(1),
                    None => tab.input_history.len() - 1,
                };
                tab.history_cursor = Some(idx);
                tab.input = tab.input_history[idx].clone();
            }
            if down {
                if let Some(i) = tab.history_cursor {
                    tab.input = tab.input_history.get(i + 1).cloned().unwrap_or_default();
                    tab.history_cursor = tab.input_history.get(i + 1).is_some().then_some(i + 1);
                }
            }
            enter
                .then(|| tab.input.trim().to_string())
                .filter(|l| !l.is_empty())
                .inspect(|_| tab.input.clear())
        };
        let _ = c(Token::Fg25);
        if let Some(line) = pending_line {
            self.submit_line(window_idx, &line);
        }
    }

    /// 对话轨侧栏（Q31：续聊与历史场所）。
    fn draw_sidebar(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let Some(w) = self.windows.get_mut(window_idx) else {
            return;
        };
        let active = w.active;
        let Some(tab) = w.tabs.get_mut(active) else {
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (who, text) in &tab.dialogue {
                    ui.label(
                        egui::RichText::new(format!("{who}: {text}"))
                            .monospace()
                            .small(),
                    );
                }
            });
        ui.separator();
        let response = ui.add(
            egui::TextEdit::singleline(&mut tab.input)
                .hint_text("侧栏续聊（iac_ask）")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let text = tab.input.trim().to_string();
            tab.input.clear();
            if !text.is_empty() {
                tab.dialogue.push(("你".into(), text.clone()));
                let (tid, sid) = (tab.id, tab.session_id.clone());
                let tx = self.ipc_tx.clone();
                let _ = std::thread::Builder::new().spawn(move || {
                    let resp = ipc::ensure_daemon_and_request(
                        "iac_ask",
                        serde_json::json!({"intent": text}),
                        sid,
                    );
                    let _ = tx.send((tid, "iac_ask", resp));
                });
            }
        }
    }

    /// 输入提交：混合识别分流（Q15）→ PTY/AI/内置/分段/备注/env。
    fn submit_line(&mut self, window_idx: usize, line: &str) {
        if line.is_empty() {
            return;
        }
        let Some(w) = self.windows.get_mut(window_idx) else {
            return;
        };
        let active = w.active;
        let Some(tab) = w.tabs.get_mut(active) else {
            return;
        };
        let sid = tab.session_id.clone();
        let tab_id = tab.id;
        // 确认词态（Q37）：输入行专用于确认词
        if tab.confirm_state == ConfirmState::AwaitWord {
            let word = line.to_string();
            tab.confirm_state = ConfirmState::Idle;
            if let Some(pid) = tab.active_proposal.clone() {
                tab.entries
                    .push(Entry::Notice("确认词已提交，执行中…".into()));
                spawn_ipc(
                    tab_id,
                    sid,
                    "iac_execute",
                    &self.ipc_tx,
                    serde_json::json!({"proposal_id": pid, "confirmed": true, "confirm_word": word, "origin": "proposal"}),
                );
                tab.busy = true;
            }
            return;
        }
        // yes/no 提案确认（保留文本习惯）
        if line == "yes" || line == "no" {
            if let Some(pid) = tab.last_proposal.clone() {
                if line == "yes" {
                    tab.entries.push(Entry::Notice("提案确认，执行中…".into()));
                    spawn_ipc(
                        tab_id,
                        sid,
                        "iac_execute",
                        &self.ipc_tx,
                        serde_json::json!({"proposal_id": pid, "confirmed": true, "origin": "proposal"}),
                    );
                    tab.busy = true;
                } else {
                    tab.entries.push(Entry::Notice("提案已拒绝".into()));
                }
                tab.last_proposal = None;
            } else {
                tab.entries.push(Entry::Notice("当前没有待确认提案".into()));
            }
            return;
        }
        tab.input_history.push(line.to_string());
        tab.history_cursor = None;
        tab.entries.push(Entry::InputEcho(line.to_string()));
        let decision = classify(line, looks_like_command);
        let mut new_theme: Option<iac_core::types::ThemeMode> = None;
        let mut needs_persist = false;
        match decision {
            InputDecision::LiteralCommand(command) => {
                // 真终端执行（域 18 主体）：写入 PTY
                tab.backend
                    .process_command(BackendCommand::Write(format!("{command}\n").into_bytes()));
                // 输入行 cd 更新已知 cwd（恢复面，Q41 注释所述边界）
                if let Some(rest) = command.strip_prefix("cd ") {
                    let target = rest.trim().trim_matches('"').trim_matches('\'');
                    if !target.is_empty() {
                        tab.cwd = if target.starts_with('/') {
                            target.to_string()
                        } else {
                            PathBuf::from(&tab.cwd)
                                .join(target)
                                .to_string_lossy()
                                .to_string()
                        };
                        tab.title = format!("sh · {}", short_cwd(&tab.cwd));
                        needs_persist = true;
                    }
                }
                // 会话事件留痕（对话轨完整可检索）
                spawn_ipc(
                    tab_id,
                    sid,
                    "iac_session_event",
                    &self.ipc_tx,
                    serde_json::json!({"kind": "command", "payload": {"command": command}}),
                );
            }
            InputDecision::AiAsk { intent } => {
                let cwd = tab.cwd.clone();
                let params = serde_json::json!({"intent": intent, "cwd": cwd, "shell": "sh"});
                spawn_ipc(tab_id, sid, "iac_ask", &self.ipc_tx, params);
                tab.busy = true;
            }
            InputDecision::AiExplain { command } => {
                spawn_ipc(
                    tab_id,
                    sid,
                    "iac_explain",
                    &self.ipc_tx,
                    serde_json::json!({"command": command}),
                );
                tab.busy = true;
            }
            InputDecision::Builtin(call) => match call.name.as_str() {
                "ia-help" => tab
                    .entries
                    .push(Entry::Output(iac_core::builtin::help_text())),
                "ia-theme" => {
                    let next = match line.split_whitespace().nth(1) {
                        Some("day") => iac_core::types::ThemeMode::Day,
                        Some("night") => iac_core::types::ThemeMode::Night,
                        Some("auto") => iac_core::types::ThemeMode::Auto,
                        _ => match self.theme_pref {
                            iac_core::types::ThemeMode::Day => iac_core::types::ThemeMode::Night,
                            iac_core::types::ThemeMode::Night => iac_core::types::ThemeMode::Auto,
                            iac_core::types::ThemeMode::Auto => iac_core::types::ThemeMode::Day,
                        },
                    };
                    new_theme = Some(next);
                    let name = match next {
                        iac_core::types::ThemeMode::Day => "day",
                        iac_core::types::ThemeMode::Night => "night",
                        iac_core::types::ThemeMode::Auto => "auto",
                    };
                    tab.entries.push(Entry::Notice(format!("主题 → {name}")));
                    self.persist_layout();
                }
                "ia-exit" => {
                    // 关当前标签（若单标签则关窗口）
                    if w.tabs.len() > 1 {
                        w.tabs.remove(active);
                        w.active = active.saturating_sub(1);
                    }
                    self.persist_layout();
                }
                _ => {
                    spawn_ipc(
                        tab_id,
                        sid,
                        "iac_builtin",
                        &self.ipc_tx,
                        serde_json::json!({"name": call.name, "args": call.args}),
                    );
                    tab.busy = true;
                }
            },
            InputDecision::TaskSeparator => {
                tab.entries.push(Entry::Separator);
                spawn_ipc(
                    tab_id,
                    sid,
                    "iac_session_event",
                    &self.ipc_tx,
                    serde_json::json!({"kind": "separator"}),
                );
            }
            InputDecision::Note(text) => {
                tab.entries.push(Entry::Note {
                    text: text.clone(),
                    pinned: false,
                });
                spawn_ipc(
                    tab_id,
                    sid,
                    "iac_session_event",
                    &self.ipc_tx,
                    serde_json::json!({"kind": "note", "payload": {"text": text}, "pinned": false}),
                );
            }
        }
        if let Some(t) = new_theme {
            self.theme_pref = t;
        }
        if needs_persist {
            self.persist_layout();
        }
    }
}

/// IPC 后台投递（统一线程封装）。
fn spawn_ipc(
    tab_id: u64,
    sid: Option<String>,
    method: &'static str,
    tx: &Sender<IpcReply>,
    params: serde_json::Value,
) {
    let tx = tx.clone();
    let _ = std::thread::Builder::new().spawn(move || {
        let resp = ipc::ensure_daemon_and_request(method, params, sid);
        let _ = tx.send((tab_id, method, resp));
    });
}

/// 提案卡渲染（多候选 ↑↓ 切换 / e 展开 / Enter 确认 / Esc 取消，Q16/Q17/Q20）。
#[allow(clippy::too_many_arguments)]
fn draw_proposal_card(
    ui: &mut egui::Ui,
    c: &impl Fn(Token) -> egui::Color32,
    id: &str,
    candidates: &[(String, String, String)],
    active: usize,
    context_json: Option<&str>,
    expanded: bool,
    confirm: &ConfirmState,
) {
    // iA 调性：无卡片框——accent 刻度 + 排版层级（字重/灰阶）承载全部结构
    ui.horizontal(|ui| {
        // 标题行：accent 刻度 + 「提案 · id」+ 风险文本（非 pill）
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 12.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 1, c(Token::Accent));
        ui.label(
            egui::RichText::new("提案")
                .monospace()
                .color(c(Token::Fg60)),
        );
        ui.label(egui::RichText::new(id).monospace().color(c(Token::Fg25)));
        if let Some((_, risk, _)) = candidates.get(active) {
            ui.label(
                egui::RichText::new(format!("· {risk}"))
                    .monospace()
                    .color(risk_text_color(c, risk)),
            );
        }
    });
    ui.add_space(2.0);
    for (i, (cmd, risk, summary)) in candidates.iter().enumerate() {
        let is_active = i == active;
        ui.horizontal(|ui| {
            // 活动候选：accent 刻度标记（与标题行同语法）
            let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 15.0), egui::Sense::hover());
            if is_active {
                ui.painter().rect_filled(rect, 1, c(Token::Accent));
            }
            // 活动候选按风险着色（等级语义可见），非活动保持次级灰
            let cmd_color = if is_active {
                risk_text_color(c, risk)
            } else {
                c(Token::Fg60)
            };
            ui.label(egui::RichText::new(cmd).monospace().color(cmd_color));
        });
        if expanded && !summary.is_empty() {
            ui.label(
                egui::RichText::new(format!("   {summary}"))
                    .monospace()
                    .small()
                    .color(c(Token::Fg60)),
            );
        }
    }
    if expanded {
        if let Some(ctx_json) = context_json {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("注入上下文（透明可查）：{ctx_json}"))
                    .monospace()
                    .small()
                    .color(c(Token::Fg25)),
            );
        }
    }
    let hint = match confirm {
        ConfirmState::Armed => "⚠ dangerous：再次 Enter 确认（武装态）",
        ConfirmState::AwaitWord => "⚠ destructive：在输入行输入确认词",
        ConfirmState::Idle => "Enter 执行 · ↑↓ 切换候选 · e 展开 · Esc 取消",
    };
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(hint)
            .monospace()
            .small()
            .color(c(Token::Fg40)),
    );
}

/// 风险文本色：dangerous 走橙红（与 destructive 红拉开等级差）。
fn risk_text_color(c: &impl Fn(Token) -> egui::Color32, risk: &str) -> egui::Color32 {
    match risk {
        "destructive" => c(Token::Destructive),
        "dangerous" => c(Token::Dangerous),
        "caution" => c(Token::Caution),
        _ => c(Token::Fg),
    }
}

/// IPC 响应 → 标签条目（多候选提案卡/失败兜底/AI 失效）。
fn apply_ipc_result(tab: &mut Tab, method: &'static str, resp: Result<IpcResponse, String>) {
    tab.busy = false;
    tab.busy_request = None;
    match resp {
        Ok(r) if r.ok => {
            let result = r.result.unwrap_or_default();
            match method {
                "iac_ask" => {
                    let id = result
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let candidates = result
                        .get("candidates")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .map(|c| {
                                    (
                                        c["command"].as_str().unwrap_or_default().to_string(),
                                        c["risk_level"].as_str().unwrap_or("safe").to_string(),
                                        c["summary"].as_str().unwrap_or_default().to_string(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let active =
                        result.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let context_json = result
                        .get("context_json")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    tab.entries.push(Entry::Proposal {
                        id: id.clone(),
                        candidates,
                        active,
                        context_json,
                        expanded: false,
                    });
                    tab.last_proposal = Some(id.clone());
                    tab.active_proposal = Some(id);
                    tab.confirm_state = ConfirmState::Idle;
                    // 对话轨（Q31）：AI 回答入侧栏
                    if let Some(first) = result
                        .pointer("/candidates/0/summary")
                        .and_then(|v| v.as_str())
                    {
                        tab.dialogue.push(("AI".into(), first.to_string()));
                    }
                }
                "iac_execute" => {
                    let status = result
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let exit = result
                        .get("exit_code")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1);
                    let out = result
                        .get("stdout_summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    tab.entries.push(Entry::Output(format!(
                        "状态：{status}（退出码 {exit}）\n{out}"
                    )));
                    if result
                        .get("offer_help")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        let exec = result
                            .get("execution_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        tab.entries.push(Entry::OfferHelp { execution_id: exec });
                    }
                }
                "iac_sessions_current" => {
                    // 首连会话绑定（域 18）
                    if let Some(id) = result.get("id").and_then(|v| v.as_str()) {
                        if tab.session_id.is_none() {
                            tab.session_id = Some(id.to_string());
                        }
                        if let Some(events) = result.get("events").and_then(|v| v.as_array()) {
                            for e in events {
                                let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                                if matches!(kind, "ai_message" | "note") {
                                    let text = e
                                        .pointer("/payload/text")
                                        .or_else(|| e.pointer("/payload/answer"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if !text.is_empty() {
                                        tab.dialogue.push((
                                            if kind == "note" { "备注" } else { "AI" }.into(),
                                            text.to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                "iac_builtin" => {
                    if let Some(inner) = result.get("result") {
                        let rendered = serde_json::to_string_pretty(inner).unwrap_or_default();
                        tab.entries.push(Entry::Output(rendered));
                    }
                }
                _ => {
                    let rendered = serde_json::to_string_pretty(&result).unwrap_or_default();
                    tab.entries.push(Entry::Output(rendered));
                }
            }
        }
        Ok(r) => {
            let err = r
                .error
                .map(|e| format!("[{}] {}", e.code, e.message))
                .unwrap_or_default();
            if method == "iac_ask" || method == "iac_explain" {
                tab.entries.push(Entry::AiError {
                    method,
                    params: serde_json::json!({}),
                    reason: err,
                });
            } else {
                tab.entries.push(Entry::Notice(err));
            }
        }
        Err(msg) => {
            if method == "iac_ask" || method == "iac_explain" {
                tab.entries.push(Entry::AiError {
                    method,
                    params: serde_json::json!({}),
                    reason: msg,
                });
            } else {
                tab.entries.push(Entry::Notice(msg));
            }
        }
    }
}

/// 系统缺省 shell。
fn system_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// cwd 显示缩略（保留尾部两段）。
fn short_cwd(cwd: &str) -> String {
    let parts: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        cwd.to_string()
    } else {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    }
}

/// 最近聚焦窗口（键位动作归属）。
fn active_window_index(_ctx: &egui::Context) -> Option<usize> {
    LAST_FOCUSED_WINDOW.with(|c| Some(c.get()))
}

thread_local! {
    static LAST_FOCUSED_WINDOW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 颜色查询应答：OSC 10/11 与索引色全部取自当前令牌调色板。
fn query_color(theme: &TerminalTheme, index: usize) -> AnsiRgb {
    let color = if index < 256 {
        theme.get_color(AnsiColor::Indexed(index as u8))
    } else {
        let named = match index {
            i if i == AnsiNamedColor::Foreground as usize => AnsiNamedColor::Foreground,
            i if i == AnsiNamedColor::Background as usize => AnsiNamedColor::Background,
            _ => AnsiNamedColor::Foreground,
        };
        theme.get_color(AnsiColor::Named(named))
    };
    AnsiRgb {
        r: color.r(),
        g: color.g(),
        b: color.b(),
    }
}

/// auto 主题跟随系统明暗（AC-28）。
pub fn resolve_mode(pref: ThemeModeAlias, ctx: egui::Context) -> Mode {
    match pref {
        ThemeModeAlias::Day => Mode::Day,
        ThemeModeAlias::Night => Mode::Night,
        ThemeModeAlias::Auto => {
            let dark = ctx.input(|i| {
                i.raw
                    .system_theme
                    .map(|t| matches!(t, egui::Theme::Dark))
                    .unwrap_or(true)
            });
            if dark {
                Mode::Night
            } else {
                Mode::Day
            }
        }
    }
}

use iac_core::types::ThemeMode as ThemeModeAlias;
