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
use iac_core::common::render::{detect_format, highlight, HighlightInput, StyledSpans, TextFormat, TokenKind};
use iac_core::ports::RawText;
use iac_core::server::IpcResponse;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// IPC 响应回传信封：(目标标签 id, 方法名, 结果/错误)。
type IpcReply = (u64, &'static str, Result<IpcResponse, String>);

/// daemon 连接状态（iac_ping 真实探测，不做假指示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DaemonStatus {
    #[default]
    Unknown,
    Ok,
    Err,
}

/// daemon 探测状态 + 最近探测时间 + 版本（回包取回）。
#[derive(Default)]
struct DaemonState {
    status: DaemonStatus,
    checked_at: Option<Instant>,
    version: String,
}

/// 块条动作（Warp 式块动作克制版）。
#[derive(Debug, Clone, PartialEq)]
enum BlockAction {
    /// 复制命令原文。
    CopyCommand(String),
    /// 复制块 cwd。
    CopyCwd(String),
    /// 插入输入行（不执行）。
    InsertInput(String),
    /// 写回 PTY 重跑（与直输同语义：iac_session_event 留痕）。
    Rerun(String),
}

/// 提案卡鼠标动作（↑↓/Enter/Esc 走键盘路径，展开/选中走点击路径）。
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProposalUiAction {
    Select(usize),
    ToggleExpand,
}

/// 高亮段：(文本, token 类别)；None = Fg 补洞段。
type HlSeg = (String, Option<TokenKind>);

/// 命令面板动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteAction {
    NewTab,
    CloseTab,
    NewWindow,
    CycleTheme,
    ToggleSidebar,
    ClearBlocks,
    ClearTasks,
    FocusInput,
    CopyCwd,
}

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
    pub sidebar_open: bool,
    pub dialogue: Vec<(String, String)>,
    /// 块式输出（Q54）：已闭合命令块 / 运行中块 / 事件文件与游标。
    pub blocks: Vec<crate::blocks::ShellBlock>,
    pub pending_blocks: Vec<crate::blocks::ShellBlock>,
    pub event_file: PathBuf,
    pub event_offset: u64,
    /// Output 条目高亮缓存（条目只追加，下标即键；None=纯文本）。
    pub hl_cache: HashMap<usize, Option<Vec<Vec<HlSeg>>>>,
}

impl Tab {
    fn new(id: u64, ctx: egui::Context, cwd: &str, session_id: Option<String>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        // 受控干净 shell（Q52）：禁读宿主 rc（含 starship 等），提示符 = iA 自有极简两行；
        // 受控 rc 落盘失败时退回直启（可用性优先，仅外观降级）。
        let spawn = match crate::shell::ensure_rc_files(&crate::shell::rc_dir()) {
            Ok(()) => crate::shell::controlled(&system_shell()),
            Err(_) => crate::shell::ControlledShell {
                shell: system_shell(),
                args: vec![],
                env: vec![],
            },
        };
        let mut env = spawn.env;
        // TERM/COLORTERM：默认 dumb 会破坏现代 TUI 程序的色彩与按键（首启体验）
        env.push(("TERM".to_owned(), "xterm-256color".to_owned()));
        env.push(("COLORTERM".to_owned(), "truecolor".to_owned()));
        // 块式输出（Q54）：per-tab 事件文件（创建即截断旧会话残留），rc 钩子经 IAC_EVENT_FILE 追加
        let event_dir = infra::config::data_dir().join("shell-events");
        let _ = std::fs::create_dir_all(&event_dir);
        let event_file = event_dir.join(format!("{id}.log"));
        let _ = std::fs::File::create(&event_file);
        env.push((
            "IAC_EVENT_FILE".to_owned(),
            event_file.to_string_lossy().into_owned(),
        ));
        let mut backend = TerminalBackend::new(
            id,
            ctx,
            tx,
            BackendSettings {
                shell: spawn.shell,
                args: spawn.args,
                env: env.into_iter().collect(),
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
            sidebar_open: false,
            dialogue: vec![],
            blocks: vec![],
            pending_blocks: vec![],
            event_file,
            event_offset: 0,
            hl_cache: HashMap::new(),
        }
    }

    /// 块式输出（Q54）：增量轮询受控 shell 事件文件。
    fn poll_shell_events(&mut self) {
        self.event_offset = crate::blocks::poll(
            &self.event_file,
            self.event_offset,
            &mut self.pending_blocks,
            &mut self.blocks,
        );
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
    /// 命令面板（Ctrl+K）：开合 / 过滤词 / 选中行。
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_sel: usize,
    /// 「聚焦输入行」面板动作置位，输入行渲染时消费。
    pub focus_input: bool,
    /// daemon 探测（iac_ping）。
    probe_tx: Sender<Result<IpcResponse, String>>,
    probe_rx: Receiver<Result<IpcResponse, String>>,
    daemon: DaemonState,
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
        let (probe_tx, probe_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            theme_pref,
            mode,
            windows: vec![],
            next_tab_id: 0,
            ipc_tx,
            ipc_rx,
            term_theme,
            palette_open: false,
            palette_query: String::new(),
            palette_sel: 0,
            focus_input: false,
            probe_tx,
            probe_rx,
            daemon: DaemonState::default(),
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
        app.probe_daemon();
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

    /// daemon 探测回包：更新真实连接状态（iac_ping）。
    fn drain_probe(&mut self) {
        while let Ok(resp) = self.probe_rx.try_recv() {
            self.daemon.checked_at = Some(Instant::now());
            match resp {
                Ok(r) if r.ok => {
                    self.daemon.status = DaemonStatus::Ok;
                    self.daemon.version = r
                        .result
                        .as_ref()
                        .and_then(|v| v.get("version"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                _ => self.daemon.status = DaemonStatus::Err,
            }
        }
        // 周期复测（30s）：状态栏 pill 永远反映真实探测结果
        if self
            .daemon
            .checked_at
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
        {
            self.probe_daemon();
        }
    }

    /// 发起一次 daemon 探测（iac_ping）。
    fn probe_daemon(&mut self) {
        self.daemon.checked_at = Some(Instant::now());
        let tx = self.probe_tx.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            let resp = ipc::ensure_daemon_and_request("iac_ping", serde_json::json!({}), None);
            let _ = tx.send(resp);
        });
    }

    /// 关闭指定窗口的指定标签（单标签则关窗口；PTY 生命周期随 backend drop 终止，
    /// 与既有 Ctrl+Shift+W 关标签同语义）。
    fn close_tab_at(&mut self, window_idx: usize, tab_idx: usize) {
        let Some(w) = self.windows.get_mut(window_idx) else {
            return;
        };
        if w.tabs.len() > 1 {
            w.tabs.remove(tab_idx);
            w.active = if tab_idx <= w.active {
                w.active.saturating_sub(1).min(w.tabs.len() - 1)
            } else {
                w.active.min(w.tabs.len() - 1)
            };
        } else if self.windows.len() > 1 {
            self.windows.remove(window_idx);
        }
        self.persist_layout();
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
    "Ctrl+K 命令面板 / Ctrl+Shift+N 新窗口 / Ctrl+Shift+T 新标签 / Ctrl+B 对话轨 / Ctrl+Shift+W 关标签。"
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
                t.poll_shell_events();
            }
        }
        self.drain_ipc();
        self.drain_probe();
        // 全局键位（新窗口/标签/侧栏/命令面板）
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
            let idx = self
                .windows
                .get(active_window)
                .map(|w| w.active)
                .unwrap_or(0);
            self.close_tab_at(active_window, idx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::K)) && ctrl && !shift {
            // 命令面板（Ctrl+K）：开合即重置过滤与选中
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
            self.palette_sel = 0;
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
        // 标签栏（tab chip：选中=SurfaceRaised+发丝线，busy accent 圆点，hover 关闭）
        let c = |t: Token| theme::color(mode, t);
        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let titles: Vec<String> = match self.windows.get(window_idx) {
                        Some(w) => w.tabs.iter().map(|t| t.title.clone()).collect(),
                        None => vec![],
                    };
                    let busy_flags: Vec<bool> = match self.windows.get(window_idx) {
                        Some(w) => w
                            .tabs
                            .iter()
                            .map(|t| t.busy || !t.pending_blocks.is_empty())
                            .collect(),
                        None => vec![],
                    };
                    let active = self.windows.get(window_idx).map(|w| w.active).unwrap_or(0);
                    for (i, title) in titles.iter().enumerate() {
                        let selected = i == active;
                        let chip = egui::Frame::NONE
                            .fill(if selected {
                                c(Token::SurfaceRaised)
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .stroke(if selected {
                                egui::Stroke::new(1.0_f32, c(Token::Hairline))
                            } else {
                                egui::Stroke::NONE
                            })
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(8, 3));
                        chip.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if busy_flags[i] {
                                    theme::status_dot(ui, c(Token::Accent), 3.0);
                                }
                                if ui
                                    .selectable_label(
                                        selected,
                                        egui::RichText::new(title.clone()),
                                    )
                                    .clicked()
                                {
                                    if let Some(w) = self.windows.get_mut(window_idx) {
                                        w.active = i;
                                    }
                                }
                                if ui
                                    .selectable_label(
                                        false,
                                        egui::RichText::new("×")
                                            .small()
                                            .color(c(Token::Fg40)),
                                    )
                                    .clicked()
                                {
                                    self.close_tab_at(window_idx, i);
                                }
                            });
                        });
                    }
                    if ui.selectable_label(false, egui::RichText::new("＋")).clicked() {
                        let ctx2 = ui.ctx().clone();
                        let tab = self.make_tab_viewport(&ctx2);
                        if let Some(w) = self.windows.get_mut(window_idx) {
                            w.tabs.push(tab);
                            w.active = w.tabs.len() - 1;
                        }
                    }
                });
            });
        // 状态栏（opencode 式：daemon 真实状态 / cwd / shell / 版本，最底一行）
        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(24.0)
            .frame(
                egui::Frame::NONE
                    .fill(c(Token::Surface))
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show_inside(ui, |ui| {
                self.draw_statusbar(ui, window_idx);
            });
        // 底部输入行（先占位）
        egui::TopBottomPanel::bottom("input")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)))
            .show_inside(ui, |ui| {
                ui.separator();
                self.draw_input_line(ui, window_idx);
            });
        // 块条（Q54）：受控 shell 命令块，叠在输入行上方（空则不占位）
        let has_blocks = self
            .windows
            .get(window_idx)
            .and_then(|w| w.tabs.get(w.active))
            .is_some_and(|t| !t.blocks.is_empty() || !t.pending_blocks.is_empty());
        if has_blocks {
            egui::TopBottomPanel::bottom("blocks")
                .resizable(true)
                .default_height(140.0)
                .min_height(64.0)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 4)))
                .show_inside(ui, |ui| {
                    self.draw_blocks(ui, window_idx);
                });
        }
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
        // 命令面板（Ctrl+K；仅聚焦窗口渲染，浮层置顶）
        if self.palette_open && window_active {
            self.draw_palette(ui, window_idx);
        }
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
        let strip = egui::Frame::NONE
            .fill(c(Token::Surface))
            .stroke(egui::Stroke::new(1.0_f32, c(Token::Hairline)))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(10, 8));
        strip.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(1.0);
            egui::CollapsingHeader::new(
                egui::RichText::new("任务条（提案 / AI / 管理）")
                    .small()
                    .color(c(Token::Fg60)),
            )
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let confirm = tab.confirm_state.clone();
                        let busy = tab.busy;
                        let count = tab.entries.len();
                        let mut proposal_action: Option<(String, ProposalUiAction)> = None;
                        for idx in 0..count {
                            let entry = tab.entries[idx].clone();
                            match &entry {
                                Entry::InputEcho(line) => {
                                    ui.label(
                                        egui::RichText::new(format!("▍ {line}"))
                                            .monospace()
                                            .color(c(Token::Fg25)),
                                    );
                                }
                                Entry::Output(text) => {
                                    // AC-41/42：core 高亮（hl_cache 缓存，文本恒不改）；
                                    // None=Plain/Markdown 纯文本
                                    let segs = tab
                                        .hl_cache
                                        .entry(idx)
                                        .or_insert_with(|| hl_spans(text));
                                    match segs {
                                        Some(lines) => {
                                            for line in lines.iter() {
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 0.0;
                                                    for (seg, kind) in line {
                                                        let token = (*kind)
                                                            .map(syntax_token)
                                                            .unwrap_or(Token::Fg);
                                                        ui.label(
                                                            egui::RichText::new(seg.as_str())
                                                                .monospace()
                                                                .color(c(token)),
                                                        );
                                                    }
                                                });
                                            }
                                        }
                                        None => {
                                            ui.label(
                                                egui::RichText::new(text.as_str())
                                                    .monospace()
                                                    .color(c(Token::Fg)),
                                            );
                                        }
                                    }
                                }
                                Entry::Notice(text) => {
                                    ui.label(
                                        egui::RichText::new(text.as_str()).color(c(Token::Fg40)),
                                    );
                                }
                                Entry::Note { text, pinned } => {
                                    let tag = if *pinned { "📌" } else { "" };
                                    ui.label(
                                        egui::RichText::new(format!("# {text} {tag}"))
                                            .small()
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
                                    if let Some(action) = draw_proposal_card(
                                        ui,
                                        &c,
                                        id,
                                        candidates,
                                        *act,
                                        context_json.as_deref(),
                                        *expanded,
                                        &confirm,
                                    ) {
                                        proposal_action = Some((id.clone(), action));
                                    }
                                }
                                Entry::OfferHelp { execution_id } => {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("命令失败了——要不要 AI 帮忙？")
                                                .color(c(Token::Destructive)),
                                        );
                                        if ui
                                            .add(egui::Button::new(
                                                egui::RichText::new("问 AI").small(),
                                            ))
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
                                                .color(c(Token::Destructive)),
                                        );
                                        let retry = params.clone();
                                        if ui
                                            .add(egui::Button::new(
                                                egui::RichText::new("重试").small(),
                                            ))
                                            .clicked()
                                        {
                                            let (tid, sid) = (tab.id, tab.session_id.clone());
                                            let tx = self.ipc_tx.clone();
                                            let method: &'static str = "iac_ask";
                                            let _ = std::thread::Builder::new().spawn(move || {
                                                let resp = ipc::ensure_daemon_and_request(
                                                    method, retry, sid,
                                                );
                                                let _ = tx.send((tid, method, resp));
                                            });
                                        }
                                    });
                                }
                            }
                            ui.add_space(3.0);
                        }
                        // 提案卡鼠标动作落账（↑↓/Enter 走输入行键位路径）
                        if let Some((pid, action)) = proposal_action {
                            for e in tab.entries.iter_mut() {
                                if let Entry::Proposal {
                                    id,
                                    active,
                                    expanded,
                                    ..
                                } = e
                                {
                                    if *id == pid {
                                        match action {
                                            ProposalUiAction::Select(i) => *active = i,
                                            ProposalUiAction::ToggleExpand => {
                                                *expanded = !*expanded
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        if busy {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(12.0).color(c(Token::Accent)));
                                ui.label(
                                    egui::RichText::new("处理中…")
                                        .small()
                                        .color(c(Token::Fg40)),
                                );
                            });
                        }
                    });
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

    /// 主输入行（Q7 直驱；生成期锁定 + Esc 取消 Q22；↑↓ 历史；❯ 前缀 + 直输弱提醒
    /// AC-04 移植；提案卡键位 Q16/Q17 接线；命令面板打开时键位让位）。
    fn draw_input_line(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        let palette_open = self.palette_open;
        let pending_line = {
            let Some(w) = self.windows.get_mut(window_idx) else {
                return;
            };
            let active = w.active;
            let Some(tab) = w.tabs.get_mut(active) else {
                return;
            };
            let tab_id = tab.id;
            let sid = tab.session_id.clone();
            let mut request_focus = false;
            if self.focus_input {
                self.focus_input = false;
                request_focus = true;
            }
            // 直输弱提醒（AC-04 移植）：dangerous/destructive 命中 → ❯/⚠ 红显（仅视觉，不拦截）
            let risk = input_risk(&tab.input);
            let hint = match tab.confirm_state {
                ConfirmState::AwaitWord => "输入确认词（见任务条提示）…",
                _ if tab.busy => "AI 处理中（Esc 取消）…",
                _ => "说人话 / 直接命令 / ??问 AI / ia-help",
            };
            let (enter, esc, up, down) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                )
            });
            let mut lost_focus_enter = false;
            let mut input_focused = false;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("❯")
                        .monospace()
                        .strong()
                        .color(if risk.is_some() {
                            c(Token::Destructive)
                        } else {
                            c(Token::Accent)
                        }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(level) = risk {
                        ui.label(
                            egui::RichText::new(format!("⚠ {level}"))
                                .small()
                                .color(c(Token::Destructive)),
                        );
                    }
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut tab.input)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace),
                    );
                    lost_focus_enter = response.lost_focus() && enter;
                    input_focused = response.has_focus();
                    if request_focus {
                        response.request_focus();
                    }
                });
            });
            if !palette_open {
                // 提案卡键位（Q16/Q17 接线，去假象化）：输入行聚焦且为空时，↑↓ 切候选 /
                // Esc 取消 / Enter 执行活动候选（走闸门确认路径，与文本 yes 同径：
                // 首次 confirmed=false 交服务端闸门判定，Armed 态二次 Enter 才置 true——
                // 一次直发 true 会绕过 Q35 双确认与 Q37 确认词）。
                // 展开走卡片上的「展开/收起」点击（键盘 e 与文本输入冲突，不画假提示）。
                let mut nav_consumed = false;
                if input_focused
                    && tab.input.is_empty()
                    && !tab.busy
                    && matches!(
                        tab.confirm_state,
                        ConfirmState::Idle | ConfirmState::Armed
                    )
                    && tab.active_proposal.is_some()
                {
                    let pid = tab.active_proposal.clone().unwrap_or_default();
                    let pidx = tab
                        .entries
                        .iter()
                        .rposition(|e| matches!(e, Entry::Proposal { id, .. } if *id == pid));
                    match pidx {
                        Some(i) => {
                            if let Entry::Proposal {
                                candidates,
                                active,
                                ..
                            } = &mut tab.entries[i]
                            {
                                if !candidates.is_empty() {
                                    if up {
                                        *active = if *active == 0 {
                                            candidates.len() - 1
                                        } else {
                                            *active - 1
                                        };
                                        nav_consumed = true;
                                    }
                                    if down {
                                        *active = (*active + 1) % candidates.len();
                                        nav_consumed = true;
                                    }
                                }
                            }
                            if esc {
                                tab.active_proposal = None;
                                tab.last_proposal = None;
                                tab.confirm_state = ConfirmState::Idle;
                                tab.entries.push(Entry::Notice("提案已取消".into()));
                            } else if enter {
                                let active_idx = match tab.entries.get(i) {
                                    Some(Entry::Proposal { active, .. }) => *active,
                                    _ => 0,
                                };
                                let confirmed = tab.confirm_state == ConfirmState::Armed;
                                tab.entries
                                    .push(Entry::Notice("提案执行中…".into()));
                                spawn_ipc(
                                    tab_id,
                                    sid,
                                    "iac_execute",
                                    &self.ipc_tx,
                                    serde_json::json!({"proposal_id": pid, "candidate_index": active_idx, "confirmed": confirmed, "origin": "proposal"}),
                                );
                                tab.busy = true;
                            }
                        }
                        None => {
                            tab.active_proposal = None;
                        }
                    }
                }
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
                // ↑↓ 历史：输入行聚焦且未被提案占用时生效（聚焦门修复：终端聚焦时
                // 方向键不再悄悄改写输入框）
                if input_focused && up && !nav_consumed && !tab.input_history.is_empty() {
                    let idx = match tab.history_cursor {
                        Some(i) => i.saturating_sub(1),
                        None => tab.input_history.len() - 1,
                    };
                    tab.history_cursor = Some(idx);
                    tab.input = tab.input_history[idx].clone();
                }
                if input_focused && down && !nav_consumed {
                    if let Some(i) = tab.history_cursor {
                        tab.input = tab.input_history.get(i + 1).cloned().unwrap_or_default();
                        tab.history_cursor =
                            tab.input_history.get(i + 1).is_some().then_some(i + 1);
                    }
                }
            }
            lost_focus_enter
                .then(|| tab.input.trim().to_string())
                .filter(|l| !l.is_empty())
                .inspect(|_| tab.input.clear())
        };
        if let Some(line) = pending_line {
            self.submit_line(window_idx, &line);
        }
    }

    /// 状态栏（opencode 式）：daemon 真实状态 pill（点击复测）/ cwd pill（点击复制）/
    /// shell 名；右端 版本 / 主题 / Ctrl+K 键帽。
    fn draw_statusbar(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        let daemon_status = self.daemon.status;
        let daemon_version = self.daemon.version.clone();
        let (cwd, shell_name) = {
            let Some(w) = self.windows.get(window_idx) else {
                return;
            };
            let Some(tab) = w.tabs.get(w.active) else {
                return;
            };
            let shell = system_shell();
            let name = shell.rsplit('/').next().unwrap_or("sh").to_owned();
            (tab.cwd.clone(), name)
        };
        let theme_name = match self.theme_pref {
            iac_core::types::ThemeMode::Day => "主题 day",
            iac_core::types::ThemeMode::Night => "主题 night",
            iac_core::types::ThemeMode::Auto => "主题 auto",
        };
        let mut reprobe = false;
        let mut open_palette = false;
        ui.horizontal(|ui| {
            // daemon pill：状态永远来自真实 iac_ping（不做假指示）
            let (dot, text) = match daemon_status {
                DaemonStatus::Ok => (
                    c(Token::Safe),
                    if daemon_version.is_empty() {
                        "daemon".to_owned()
                    } else {
                        format!("daemon · {daemon_version}")
                    },
                ),
                DaemonStatus::Err => (c(Token::Destructive), "daemon · 未连接".to_owned()),
                DaemonStatus::Unknown => (c(Token::Fg25), "daemon · 探测中".to_owned()),
            };
            let pill = egui::Frame::NONE
                .fill(c(Token::SurfaceRaised))
                .corner_radius(egui::CornerRadius::same(9))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::status_dot(ui, dot, 3.0);
                        ui.label(egui::RichText::new(text).small().color(c(Token::Fg60)));
                    });
                });
            let pill_resp = ui
                .interact(
                    pill.response.rect,
                    egui::Id::new(("daemon-pill", window_idx)),
                    egui::Sense::click(),
                )
                .on_hover_text("点击重新探测 daemon");
            if pill_resp.clicked() {
                reprobe = true;
            }
            // cwd pill：点击复制路径
            let cwd_r = ui.selectable_label(
                false,
                egui::RichText::new(short_cwd(&cwd))
                    .small()
                    .color(c(Token::Fg60)),
            );
            let cwd_full = cwd.clone();
            if cwd_r.on_hover_text(cwd_full).clicked() {
                ui.ctx().copy_text(cwd.clone());
            }
            ui.label(
                egui::RichText::new(shell_name.as_str())
                    .small()
                    .color(c(Token::Fg25)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .small()
                        .color(c(Token::Fg25)),
                );
                ui.label(egui::RichText::new(theme_name).small().color(c(Token::Fg40)));
                if theme::kbd_chip(ui, mode, "Ctrl+K", "sb-kbd").clicked() {
                    open_palette = true;
                }
            });
        });
        if reprobe {
            self.probe_daemon();
        }
        if open_palette {
            self.palette_open = true;
        }
    }

    /// 命令面板（Raycast 式）：Ctrl+K 开合；过滤动作、↑↓ 选择、Enter 执行、Esc 关闭。
    /// SurfaceRaised + 发丝线 + 10px 圆角浮层（维持无阴影立场）。
    fn draw_palette(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        egui::Area::new(egui::Id::new(("cmd-palette", window_idx)))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(c(Token::SurfaceRaised))
                    .stroke(egui::Stroke::new(1.0_f32, c(Token::Hairline)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_min_width(560.0);
                        ui.set_width(ui.available_width());
                        let query_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .hint_text("搜索命令…")
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Body),
                        );
                        query_resp.request_focus();
                        let (enter, esc, up, down) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::Enter),
                                i.key_pressed(egui::Key::Escape),
                                i.key_pressed(egui::Key::ArrowUp),
                                i.key_pressed(egui::Key::ArrowDown),
                            )
                        });
                        let hits = palette_hits(&self.palette_query);
                        if up {
                            self.palette_sel = self.palette_sel.saturating_sub(1);
                        }
                        if down {
                            self.palette_sel = (self.palette_sel + 1).min(hits.len().saturating_sub(1));
                        }
                        if esc {
                            self.palette_open = false;
                        }
                        if self.palette_sel >= hits.len() {
                            self.palette_sel = hits.len().saturating_sub(1);
                        }
                        ui.add_space(4.0);
                        let mut executed = false;
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for (row, &hit) in hits.iter().enumerate() {
                                    let (label, kbd, action) = PALETTE_ACTIONS[hit];
                                    let selected = row == self.palette_sel;
                                    let row_frame = egui::Frame::NONE
                                        .fill(if selected {
                                            c(Token::SurfaceHover)
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .corner_radius(egui::CornerRadius::same(6))
                                        .inner_margin(egui::Margin::symmetric(6, 4));
                                    let inner = row_frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(label).color(if selected {
                                                    c(Token::Fg)
                                                } else {
                                                    c(Token::Fg60)
                                                }),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    theme::kbd_chip(
                                                        ui,
                                                        mode,
                                                        kbd,
                                                        &format!("pal-kbd-{hit}"),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                    if (inner.response.clicked() || (enter && selected))
                                        && !executed
                                    {
                                        executed = true;
                                        self.palette_open = false;
                                        self.run_palette_action(action, ui.ctx().clone());
                                    }
                                }
                            });
                    });
            });
    }

    /// 命令面板动作执行（与既有热键/内置命令同语义）。
    fn run_palette_action(&mut self, action: PaletteAction, ctx: egui::Context) {
        let active_window = active_window_index(&ctx);
        match action {
            PaletteAction::NewTab => {
                if let Some(wi) = active_window {
                    let tab = self.make_tab_viewport(&ctx);
                    if let Some(w) = self.windows.get_mut(wi) {
                        w.tabs.push(tab);
                        w.active = w.tabs.len() - 1;
                    }
                }
            }
            PaletteAction::CloseTab => {
                if let Some(wi) = active_window {
                    let idx = self.windows.get(wi).map(|w| w.active).unwrap_or(0);
                    self.close_tab_at(wi, idx);
                }
            }
            PaletteAction::NewWindow => {
                let tab = self.make_tab_viewport(&ctx);
                self.windows.push(WindowState {
                    viewport: egui::ViewportId(egui::Id::new(self.windows.len())),
                    bounds: [0.0, 0.0, 1280.0, 820.0],
                    tabs: vec![tab],
                    active: 0,
                });
            }
            PaletteAction::CycleTheme => {
                let next = self.cycle_theme_pref();
                let name = match next {
                    iac_core::types::ThemeMode::Day => "day",
                    iac_core::types::ThemeMode::Night => "night",
                    iac_core::types::ThemeMode::Auto => "auto",
                };
                if let Some(wi) = active_window {
                    if let Some(w) = self.windows.get_mut(wi) {
                        if let Some(t) = w.tabs.get_mut(w.active) {
                            t.entries.push(Entry::Notice(format!("主题 → {name}")));
                        }
                    }
                }
                self.persist_layout();
            }
            PaletteAction::ToggleSidebar => {
                if let Some(wi) = active_window {
                    if let Some(w) = self.windows.get_mut(wi) {
                        if let Some(t) = w.tabs.get_mut(w.active) {
                            t.sidebar_open = !t.sidebar_open;
                        }
                    }
                }
            }
            PaletteAction::ClearBlocks => {
                if let Some(wi) = active_window {
                    if let Some(w) = self.windows.get_mut(wi) {
                        if let Some(t) = w.tabs.get_mut(w.active) {
                            t.blocks.clear();
                            t.pending_blocks.clear();
                        }
                    }
                }
            }
            PaletteAction::ClearTasks => {
                if let Some(wi) = active_window {
                    if let Some(w) = self.windows.get_mut(wi) {
                        if let Some(t) = w.tabs.get_mut(w.active) {
                            t.entries.clear();
                        }
                    }
                }
            }
            PaletteAction::FocusInput => self.focus_input = true,
            PaletteAction::CopyCwd => {
                if let Some(wi) = active_window {
                    if let Some(cwd) = self
                        .windows
                        .get(wi)
                        .and_then(|w| w.tabs.get(w.active))
                        .map(|t| t.cwd.clone())
                    {
                        ctx.copy_text(cwd);
                    }
                }
            }
        }
    }

    /// 主题循环切换（day→night→auto→day）。
    fn cycle_theme_pref(&mut self) -> iac_core::types::ThemeMode {
        let next = match self.theme_pref {
            iac_core::types::ThemeMode::Day => iac_core::types::ThemeMode::Night,
            iac_core::types::ThemeMode::Night => iac_core::types::ThemeMode::Auto,
            iac_core::types::ThemeMode::Auto => iac_core::types::ThemeMode::Day,
        };
        self.theme_pref = next;
        next
    }

    /// 对话轨侧栏（Q31：续聊与历史场所）。
    /// 块条（Q54）：命令块行（SurfaceRaised 卡 + 退出码/耗时着色，Warp 式块历史的克制版）。
    /// 块条（Q54 现代化）：hover 动作条（复制/插入/重跑）+ cwd + 运行中 Spinner；
    /// 点击整行 = 插入输入行（不执行）。
    fn draw_blocks(&mut self, ui: &mut egui::Ui, window_idx: usize) {
        let mode = self.mode;
        let Some(tab) = self
            .windows
            .get_mut(window_idx)
            .and_then(|w| w.tabs.get_mut(w.active))
        else {
            return;
        };
        let (blocks, pending) = (tab.blocks.clone(), tab.pending_blocks.clone());
        let tab_id = tab.id;
        let mut actions: Vec<BlockAction> = vec![];
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (i, b) in blocks.iter().enumerate() {
                    if let Some(a) = shell_block_row(
                        ui,
                        mode,
                        b,
                        false,
                        tab_id ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9),
                    ) {
                        actions.push(a);
                    }
                }
                for (i, b) in pending.iter().enumerate() {
                    if let Some(a) = shell_block_row(
                        ui,
                        mode,
                        b,
                        true,
                        tab_id ^ (i as u64 + 100_001).wrapping_mul(0x9E37_79B9),
                    ) {
                        actions.push(a);
                    }
                }
            });
        if actions.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        for action in actions {
            match action {
                BlockAction::CopyCommand(cmd) | BlockAction::CopyCwd(cmd) => ctx.copy_text(cmd),
                BlockAction::InsertInput(cmd) => tab.input = cmd,
                BlockAction::Rerun(cmd) => {
                    // 与直输同语义：写 PTY + 会话留痕（Q55 闸门悬置，本轮不新增执行门槛）
                    tab.backend
                        .process_command(BackendCommand::Write(format!("{cmd}\n").into_bytes()));
                    let sid = tab.session_id.clone();
                    spawn_ipc(
                        tab_id,
                        sid,
                        "iac_session_event",
                        &self.ipc_tx,
                        serde_json::json!({"kind": "command", "payload": {"command": cmd}}),
                    );
                }
            }
        }
    }

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
        // 确认词态（Q37）：输入行专用于确认词；confirmed=false 交服务端
        // check_escalation 校验（「确认 <尾词>」不匹配即拒绝，客户端不代放行）
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
                    serde_json::json!({"proposal_id": pid, "confirmed": false, "confirm_word": word, "origin": "proposal"}),
                );
                tab.busy = true;
            }
            return;
        }
        // yes/no 提案确认（保留文本习惯）：yes 首次 confirmed=false（safe 直接执行，
        // dangerous 打回后置 Armed 再 yes 才置 true）；提案引用保留至终态
        // （apply_ipc_result 按 gate.action 推进/清理），打回后仍有送达对象
        if line == "yes" || line == "no" {
            if let Some(pid) = tab.last_proposal.clone() {
                if line == "yes" {
                    let confirmed = tab.confirm_state == ConfirmState::Armed;
                    tab.entries.push(Entry::Notice("提案确认，执行中…".into()));
                    spawn_ipc(
                        tab_id,
                        sid,
                        "iac_execute",
                        &self.ipc_tx,
                        serde_json::json!({"proposal_id": pid, "confirmed": confirmed, "origin": "proposal"}),
                    );
                    tab.busy = true;
                } else {
                    tab.entries.push(Entry::Notice("提案已拒绝".into()));
                    tab.last_proposal = None;
                    tab.active_proposal = None;
                    tab.confirm_state = ConfirmState::Idle;
                }
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
                    self.close_tab_at(window_idx, active);
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

/// 提案卡渲染（多候选：↑↓/Enter/Esc 键位在输入行接线，点击候选选中、
/// 点击「展开/收起」切换详情；提示文案与真实可触发键位一一对应）。
/// 返回鼠标动作（无则 None）。
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
) -> Option<ProposalUiAction> {
    let mut action: Option<ProposalUiAction> = None;
    // Linear 式卡片：surface-2 底 + 发丝线描边 + 8px 圆角；活动候选以 accent 左竖条标记
    let card = egui::Frame::NONE
        .fill(c(Token::SurfaceRaised))
        .stroke(egui::Stroke::new(1.0_f32, c(Token::Hairline)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10));
    card.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // 头部：标题 + 活动候选风险 chip + 展开/收起（真实可点的切换）
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("提案 ")
                    .strong()
                    .color(c(Token::Fg)),
            );
            ui.label(egui::RichText::new(id).monospace().small().color(c(Token::Fg60)));
            if let Some((_, risk, _)) = candidates.get(active) {
                risk_chip(ui, c, risk);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggle = if expanded { "收起" } else { "展开" };
                if ui
                    .selectable_label(false, egui::RichText::new(toggle).small().color(c(Token::Fg40)))
                    .clicked()
                {
                    action = Some(ProposalUiAction::ToggleExpand);
                }
            });
        });
        ui.add_space(2.0);
        for (i, (cmd, risk, summary)) in candidates.iter().enumerate() {
            let is_active = i == active;
            ui.horizontal(|ui| {
                // accent 左竖条标记活动候选
                let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
                if is_active {
                    ui.painter().rect_filled(rect, 1.5, c(Token::Accent));
                }
                // 活动候选按风险着色（等级语义可见），非活动保持次级灰；点击行=选中
                let cmd_color = if is_active {
                    risk_text_color(c, risk)
                } else {
                    c(Token::Fg60)
                };
                if ui
                    .selectable_label(is_active, egui::RichText::new(cmd).monospace().color(cmd_color))
                    .clicked()
                {
                    action = Some(ProposalUiAction::Select(i));
                }
            });
            if expanded && !summary.is_empty() {
                ui.label(
                    egui::RichText::new(format!("   {summary}"))
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
                        .small()
                        .color(c(Token::Fg25)),
                );
            }
        }
        let hint = match confirm {
            ConfirmState::Armed => "⚠ dangerous：再次 Enter 确认（武装态）",
            ConfirmState::AwaitWord => "⚠ destructive：在输入行输入确认词",
            ConfirmState::Idle => "Enter 执行 · ↑↓ 切候选 · 点击候选选中 · Esc 取消",
        };
        ui.add_space(3.0);
        ui.label(
            egui::RichText::new(hint)
                .small()
                .color(c(Token::Fg40)),
        );
    });
    action
}

/// 风险 chip：等级色 18% 底 + 等级色文本 + 4px 圆角（现代 pill 标签）。
fn risk_chip(ui: &mut egui::Ui, c: &impl Fn(Token) -> egui::Color32, risk: &str) {
    let (color_token, label) = match risk {
        "destructive" => (Token::Destructive, "destructive"),
        "dangerous" => (Token::Dangerous, "dangerous"),
        "caution" => (Token::Caution, "caution"),
        _ => (Token::Safe, "safe"),
    };
    let fill = c(color_token).gamma_multiply(0.18);
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .monospace()
                    .small()
                    .color(c(color_token)),
            );
        });
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
                    let gate_action = result
                        .get("gate")
                        .and_then(|g| g.get("action"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    // 确认升级（Q35/Q37）：Rejected + 打回动作 → 置位确认态机并保留提案；
                    // 其余（执行完成/终态拒绝）→ 清确认态与提案引用
                    if status == "Rejected" {
                        match gate_action.as_str() {
                            "require_double_enter" => {
                                tab.confirm_state = ConfirmState::Armed;
                                tab.entries.push(Entry::Notice(
                                    "⚠ dangerous：再次 Enter / yes 执行（二次确认）".into(),
                                ));
                                return;
                            }
                            "require_confirm_word" => {
                                let tail = tab
                                    .active_proposal
                                    .as_ref()
                                    .and_then(|pid| {
                                        tab.entries.iter().rev().find_map(|e| {
                                            match e {
                                                Entry::Proposal { id, candidates, active, .. }
                                                    if id == pid =>
                                                {
                                                    candidates.get(*active).map(|(c, _, _)| {
                                                        c.split_whitespace()
                                                            .last()
                                                            .unwrap_or_default()
                                                            .to_string()
                                                    })
                                                }
                                                _ => None,
                                            }
                                        })
                                    })
                                    .unwrap_or_default();
                                tab.confirm_state = ConfirmState::AwaitWord;
                                tab.entries.push(Entry::Notice(format!(
                                    "⚠ destructive：在输入行输入「确认 {tail}」执行"
                                )));
                                return;
                            }
                            _ => {}
                        }
                    }
                    tab.confirm_state = ConfirmState::Idle;
                    tab.last_proposal = None;
                    tab.active_proposal = None;
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

/// 命令块行（Q54 现代化）：hover 显动作条（复制/插入/重跑）+ cwd chip；
/// 运行中 Spinner；点击整行 = 插入输入行（不执行）。
fn shell_block_row(
    ui: &mut egui::Ui,
    mode: Mode,
    block: &crate::blocks::ShellBlock,
    running: bool,
    id_salt: u64,
) -> Option<BlockAction> {
    let c = |t: Token| theme::color(mode, t);
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 30.0),
        egui::Sense::click(),
    );
    // 底框：hover 抬升（surface-3），常态 surface-2 + 发丝线（无阴影立场）
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        6.0,
        c(if resp.hovered() {
            Token::SurfaceHover
        } else {
            Token::SurfaceRaised
        }),
    );
    painter.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0_f32, c(Token::Hairline)),
        egui::StrokeKind::Inside,
    );
    let command = block.command.lines().next().unwrap_or_default().to_string();
    let hovered = resp.hovered();
    let mut action: Option<BlockAction> = None;
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(4.0))
            .id_salt(id_salt),
        |ui| {
        ui.set_width(rect.width() - 8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("❯").monospace().color(c(Token::Accent)));
            ui.label(
                egui::RichText::new(command.as_str())
                    .monospace()
                    .color(c(Token::Fg)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if hovered {
                    // Warp 式动作条（克制版）：重跑最右、复制最左
                    if ui
                        .selectable_label(false, egui::RichText::new("重跑").small())
                        .clicked()
                    {
                        action = Some(BlockAction::Rerun(command.clone()));
                    }
                    if ui
                        .selectable_label(false, egui::RichText::new("插入").small())
                        .clicked()
                    {
                        action = Some(BlockAction::InsertInput(command.clone()));
                    }
                    if ui
                        .selectable_label(false, egui::RichText::new("复制").small())
                        .clicked()
                    {
                        action = Some(BlockAction::CopyCommand(command.clone()));
                    }
                } else if running {
                    ui.add(egui::Spinner::new().size(10.0).color(c(Token::Accent)));
                    ui.label(
                        egui::RichText::new("运行中")
                            .small()
                            .color(c(Token::Fg40)),
                    );
                } else if block.exit == Some(0) {
                    ui.label(
                        egui::RichText::new(format!("✓ {:.2}s", block.duration))
                            .small()
                            .color(c(Token::Safe)),
                    );
                    if !block.cwd.is_empty() {
                        let cwd_r = ui.selectable_label(
                            false,
                            egui::RichText::new(short_cwd(&block.cwd))
                                .small()
                                .color(c(Token::Fg40)),
                        );
                        let cwd_full = block.cwd.clone();
                        if cwd_r.on_hover_text(cwd_full).clicked() {
                            action = Some(BlockAction::CopyCwd(block.cwd.clone()));
                        }
                    }
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "✗ {} · {:.2}s",
                            block.exit.unwrap_or(-1),
                            block.duration
                        ))
                        .small()
                        .color(c(Token::Dangerous)),
                    );
                }
            });
        });
        },
    );
    // 整行点击 = 插入输入行（不执行；动作按钮已注册在其上，点击不会穿透）
    if action.is_none() && resp.clicked() {
        action = Some(BlockAction::InsertInput(block.command.clone()));
    }
    action
}

// ---------- 命令面板动作表（Raycast 式；纯数据便于测试） ----------

/// 面板动作表：(显示名, 键位提示, 动作)。kbd 提示为空串者仅面板可达。
const PALETTE_ACTIONS: [(&str, &str, PaletteAction); 9] = [
    ("聚焦输入行", "", PaletteAction::FocusInput),
    ("新建标签", "Ctrl+⇧T", PaletteAction::NewTab),
    ("关闭当前标签", "Ctrl+⇧W", PaletteAction::CloseTab),
    ("新建窗口", "Ctrl+⇧N", PaletteAction::NewWindow),
    ("切换主题（day→night→auto）", "", PaletteAction::CycleTheme),
    ("开/关对话轨侧栏", "Ctrl+B", PaletteAction::ToggleSidebar),
    ("清空块条", "", PaletteAction::ClearBlocks),
    ("清空任务条", "", PaletteAction::ClearTasks),
    ("复制当前目录路径", "", PaletteAction::CopyCwd),
];

/// 面板过滤：query 对动作名做大小写不敏感包含匹配（空 query = 全量）。
fn palette_hits(query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    PALETTE_ACTIONS
        .iter()
        .enumerate()
        .filter(|(_, (label, _, _))| q.is_empty() || label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

// ---------- 输入弱提醒 / 语法高亮（与 TUI 同语义的移植） ----------

/// 输入行直输弱提醒（AC-04 移植，与 iac/src/console.rs `input_risk` 同语义）：
/// dangerous/destructive → 视觉预警；?? 问 AI 等非直输路径不预警；不拦截。
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

/// core TokenKind → 主题令牌（与 iac/src/tui.rs 映射一致）。
fn syntax_token(kind: TokenKind) -> Token {
    match kind {
        TokenKind::Keyword => Token::SynKeyword,
        TokenKind::Str => Token::SynString,
        TokenKind::Comment | TokenKind::Punct => Token::Fg25,
        TokenKind::Number => Token::Fg60,
    }
}

/// core 高亮 → 每行分段（None = Plain/Markdown 不做词法）。
/// 与 iac/src/tui.rs `render_output` 同款管线（TestEnv + detect + highlight）。
fn hl_spans(text: &str) -> Option<Vec<Vec<HlSeg>>> {
    let env = iac_core::framework::TestEnv::new();
    let mut format = TextFormat::Plain;
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let input = RawText {
        content: text.to_string(),
        hint: None,
    };
    let _ = detect_format(&input, &mut format, &env.context(), &mut m, &mut evo);
    if matches!(format, TextFormat::Markdown | TextFormat::Plain) {
        return None;
    }
    let hi = HighlightInput {
        text: text.to_string(),
        format,
    };
    let mut out = StyledSpans::default();
    let _ = highlight(&hi, &mut out, &env.context(), &mut m, &mut evo);
    Some(span_lines(text, &out.spans))
}

/// span → 行：按真实行起点偏移归属；跨行 span 截到本行，span 间隙补 Fg 段
/// （core 只产出稀疏 token span，不补洞会丢字——iac/src/tui.rs 同结论）。
fn span_lines(text: &str, spans: &[(usize, usize, TokenKind)]) -> Vec<Vec<HlSeg>> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line_text in body.split('\n') {
        let line_start = offset;
        let line_end = line_start + line_text.len();
        offset = line_end + 1; // 越过 '\n'
        let mut clipped: Vec<(usize, usize, &TokenKind)> = spans
            .iter()
            .filter_map(|(s, e, kind)| {
                let s = (*s).max(line_start);
                let e = (*e).min(line_end);
                (e > s).then_some((s, e, kind))
            })
            .collect();
        clipped.sort_by_key(|(s, _, _)| *s);
        let mut segs: Vec<HlSeg> = Vec::new();
        let mut pos = line_start;
        for (s, e, kind) in clipped {
            if s > pos {
                segs.push((line_text[pos - line_start..s - line_start].to_string(), None));
            }
            if e > pos {
                let from = s.max(pos);
                segs.push((
                    line_text[from - line_start..e - line_start].to_string(),
                    Some(*kind),
                ));
                pos = e;
            }
        }
        if pos < line_end {
            segs.push((line_text[pos - line_start..].to_string(), None));
        }
        if segs.is_empty() {
            segs.push((line_text.to_string(), None));
        }
        lines.push(segs);
    }
    lines
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

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-04 移植：直输弱提醒与 TUI console 同语义。
    #[test]
    fn input_risk_matches_tui_semantics() {
        assert_eq!(input_risk("rm -rf ./build"), Some("dangerous"));
        assert_eq!(input_risk("ls -la"), None);
        assert_eq!(input_risk(""), None);
        assert_eq!(input_risk("   "), None);
        assert_eq!(input_risk("??rm -rf"), None, "问 AI 路径不做直输预警");
    }

    /// 面板动作表可发现性：动作名不得重复（面板是键位的可发现入口）。
    #[test]
    fn palette_table_unique_labels() {
        assert!(PALETTE_ACTIONS.len() >= 8);
        let mut labels: Vec<_> = PALETTE_ACTIONS.iter().map(|(l, _, _)| *l).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), PALETTE_ACTIONS.len());
    }

    /// 面板过滤：空 query 全量且保持表序；不命中为空。
    #[test]
    fn palette_filter_matches_and_preserves_order() {
        assert_eq!(palette_hits("").len(), PALETTE_ACTIONS.len());
        assert!(palette_hits("zzz-不存在").is_empty());
        let hits = palette_hits("标签");
        assert!(hits.iter().all(|&i| PALETTE_ACTIONS[i].0.contains("标签")));
        let all = palette_hits("");
        assert_eq!(
            all,
            (0..PALETTE_ACTIONS.len()).collect::<Vec<_>>(),
            "空过滤保持表序"
        );
    }

    /// 高亮分段必须无损还原原文（补洞不丢字），且数字 token 命中。
    #[test]
    fn hl_spans_reconstructs_text_losslessly() {
        let lines = hl_spans("{\"a\": 1}").expect("JSON 应走高亮");
        let flat: String = lines.iter().flatten().map(|(s, _)| s.as_str()).collect();
        assert_eq!(flat, "{\"a\": 1}");
        assert!(
            lines
                .iter()
                .flatten()
                .any(|(_, k)| *k == Some(TokenKind::Number))
        );
    }

    /// Plain 文本不做词法（与 TUI 语义一致）。
    #[test]
    fn plain_text_has_no_lexical_highlight() {
        assert!(hl_spans("普通一句话，不加修饰").is_none());
    }
}
