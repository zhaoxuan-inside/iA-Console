//! ConsoleApp：主窗口状态与绘制（入口豁免）。
//! 中央 = egui_term 真终端（vendor 修复版，真实 shell）；右侧 = AI 侧栏（异步 IPC → daemon）。
//! chg-021 焦点模型：点击谁谁聚焦——终端与侧栏输入互不抢占，键盘跟随焦点而非指针。

use crate::sidebar::{self, SidebarState};
use crate::theme::{self, Mode, Token};
use egui_term::{
    AnsiColor, AnsiNamedColor, AnsiRgb, BackendCommand, BackendSettings, PtyEvent, TerminalBackend,
    TerminalTheme, TerminalView,
};
use iac_core::server::IpcResponse;
use std::sync::mpsc::{Receiver, Sender};

/// IPC 响应回传信封：(方法名, 结果/错误)。
type IpcReply = (&'static str, Result<IpcResponse, String>);

/// 终端标签。
struct TermTab {
    id: u64,
    title: String,
    backend: TerminalBackend,
}

/// 主应用。
pub struct ConsoleApp {
    mode: Mode,
    tabs: Vec<TermTab>,
    active: usize,
    next_tab_id: u64,
    pty_tx: Sender<(u64, PtyEvent)>,
    pty_rx: Receiver<(u64, PtyEvent)>,
    ipc_tx: Sender<IpcReply>,
    ipc_rx: Receiver<IpcReply>,
    sidebar: SidebarState,
    /// 焦点归属：true=终端 / false=侧栏输入。只随点击变化，不做每帧抢占。
    terminal_focus: bool,
    term_theme: TerminalTheme,
}

impl ConsoleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let (pty_tx, pty_rx) = std::sync::mpsc::channel();
        let (ipc_tx, ipc_rx) = std::sync::mpsc::channel();
        let backend = TerminalBackend::new(
            0,
            cc.egui_ctx.clone(),
            pty_tx.clone(),
            BackendSettings { shell: system_shell(), ..Default::default() },
        )
        .expect("终端后端初始化");
        let mode = Mode::Night;
        let app = Self {
            mode,
            tabs: vec![TermTab { id: 0, title: default_title(0), backend }],
            active: 0,
            next_tab_id: 1,
            pty_tx,
            pty_rx,
            ipc_tx,
            ipc_rx,
            sidebar: SidebarState::default(),
            terminal_focus: true,
            term_theme: TerminalTheme::new(Box::new(theme::terminal_palette(mode))),
        };
        theme::apply_style(&cc.egui_ctx, mode);
        app
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut TermTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn close_tab(&mut self, id: u64) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            self.tabs.remove(pos);
            self.active = self
                .active
                .saturating_sub(1)
                .min(self.tabs.len().saturating_sub(1));
        }
    }

    /// PTY 事件流：Exit 关标签；PtyWrite/颜色查询须应答，否则 vim/nvim 等交互程序卡查询。
    fn drain_pty_events(&mut self, ctx: &egui::Context) {
        while let Ok((id, ev)) = self.pty_rx.try_recv() {
            match ev {
                PtyEvent::Exit => self.close_tab(id),
                PtyEvent::PtyWrite(text) => {
                    if let Some(tab) = self.tab_mut(id) {
                        tab.backend.process_command(BackendCommand::Write(text.into_bytes()));
                    }
                }
                PtyEvent::Title(title) => {
                    if let Some(tab) = self.tab_mut(id) {
                        tab.title = if title.is_empty() { default_title(id) } else { title };
                    }
                }
                PtyEvent::ResetTitle => {
                    if let Some(tab) = self.tab_mut(id) {
                        tab.title = default_title(id);
                    }
                }
                PtyEvent::ColorRequest(index, formatter) => {
                    let reply = formatter(self.query_color(index));
                    if let Some(tab) = self.tab_mut(id) {
                        tab.backend.process_command(BackendCommand::Write(reply.into_bytes()));
                    }
                }
                PtyEvent::ClipboardStore(_, text) => ctx.copy_text(text),
                _ => {}
            }
        }
    }

    /// 颜色查询应答：OSC 10/11（256=前景 257=背景）与 0..256 索引色，全部取自当前令牌调色板。
    fn query_color(&self, index: usize) -> AnsiRgb {
        let color = if index < 256 {
            self.term_theme.get_color(AnsiColor::Indexed(index as u8))
        } else {
            let named = match index {
                i if i == AnsiNamedColor::Foreground as usize => AnsiNamedColor::Foreground,
                i if i == AnsiNamedColor::Background as usize => AnsiNamedColor::Background,
                _ => AnsiNamedColor::Foreground,
            };
            self.term_theme.get_color(AnsiColor::Named(named))
        };
        AnsiRgb { r: color.r(), g: color.g(), b: color.b() }
    }

    /// 异步 IPC 响应落侧栏（每帧 drain，UI 永不等待）。
    fn drain_ipc_results(&mut self) {
        while let Ok((method, resp)) = self.ipc_rx.try_recv() {
            sidebar::apply_ipc_result(&mut self.sidebar, method, &resp);
        }
    }
}

impl eframe::App for ConsoleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_pty_events(ctx);
        self.drain_ipc_results();
        self.draw_tab_bar(ctx);
        self.draw_sidebar(ctx);
        self.draw_terminal(ctx);
    }
}

impl ConsoleApp {
    /// 顶部标签栏：标签切换、新建、主题切换。
    fn draw_tab_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, tab) in self.tabs.iter().enumerate() {
                    let label = if i == self.active {
                        egui::RichText::new(format!("● {}", tab.title))
                            .color(theme::color(self.mode, Token::Accent))
                    } else {
                        egui::RichText::new(format!("○ {}", tab.title))
                            .color(theme::color(self.mode, Token::Fg60))
                    };
                    if ui.selectable_label(i == self.active, label).clicked() {
                        self.active = i;
                        self.terminal_focus = true;
                    }
                }
                if ui.button("+").clicked() {
                    let id = self.next_tab_id;
                    self.next_tab_id += 1;
                    if let Ok(backend) = TerminalBackend::new(
                        id,
                        ctx.clone(),
                        self.pty_tx.clone(),
                        BackendSettings { shell: system_shell(), ..Default::default() },
                    ) {
                        self.tabs.push(TermTab { id, title: default_title(id), backend });
                        self.active = self.tabs.len() - 1;
                        self.terminal_focus = true;
                    }
                }
                let theme_label = match self.mode {
                    Mode::Night => "☾",
                    Mode::Day => "☀",
                };
                if ui.button(theme_label).clicked() {
                    self.mode = match self.mode {
                        Mode::Day => Mode::Night,
                        Mode::Night => Mode::Day,
                    };
                    self.term_theme = TerminalTheme::new(Box::new(theme::terminal_palette(self.mode)));
                    theme::apply_style(ctx, self.mode);
                }
            });
        });
    }

    /// 右侧 AI 侧栏：条目流 + 输入行（Enter 提交，标记分发；IPC 全异步）。
    fn draw_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("ai_sidebar")
            .default_width(380.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("AI").color(theme::color(self.mode, Token::Fg)));
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &self.sidebar.entries {
                            self.show_entry(ui, entry);
                            ui.add_space(4.0);
                        }
                        // 有在途请求时给可见反馈（busy 状态现在跨帧存在）
                        if self.sidebar.busy {
                            ui.add(egui::Label::new(
                                egui::RichText::new("处理中…")
                                    .color(theme::color(self.mode, Token::Fg40))
                                    .size(12.0),
                            ));
                        }
                    });
                ui.separator();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.sidebar.input)
                        .hint_text("直接命令 / ??问 AI / yes 确认提案 / /help")
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                if response.clicked() {
                    self.terminal_focus = false;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let line = self.sidebar.input.trim().to_string();
                    self.sidebar.input.clear();
                    if !line.is_empty() {
                        let action =
                            sidebar::plan_action(&line, self.sidebar.last_proposal.is_some());
                        sidebar::perform(&mut self.sidebar, action, &self.ipc_tx);
                    }
                }
            });
    }

    /// 单条侧栏条目渲染。
    fn show_entry(&self, ui: &mut egui::Ui, entry: &sidebar::Entry) {
        match entry {
            sidebar::Entry::Output(text) => ui.add(egui::Label::new(
                egui::RichText::new(text)
                    .color(theme::color(self.mode, Token::Fg))
                    .monospace()
                    .size(12.0),
            )),
            sidebar::Entry::Notice(text) => ui.add(egui::Label::new(
                egui::RichText::new(text)
                    .color(theme::color(self.mode, Token::Caution))
                    .size(12.0),
            )),
            sidebar::Entry::Note(text) => ui.add(egui::Label::new(
                egui::RichText::new(format!("# {text}"))
                    .color(theme::color(self.mode, Token::Fg40))
                    .size(12.0),
            )),
            sidebar::Entry::Separator => ui.add(egui::Label::new(
                egui::RichText::new("· · ·").color(theme::color(self.mode, Token::Fg25)),
            )),
            sidebar::Entry::Proposal { command, risk, .. } => {
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("✚ {command}"))
                        .color(theme::color(self.mode, Token::Accent))
                        .monospace()
                        .size(13.0),
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("   风险 {risk} · yes 执行 / no 拒绝"))
                        .color(theme::color(self.mode, Token::Caution))
                        .size(12.0),
                ))
            }
        };
    }

    /// 中央真终端：焦点只随点击变化。
    fn draw_terminal(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let term = TerminalView::new(ui, &mut tab.backend)
                    .set_focus(self.terminal_focus)
                    .set_theme(self.term_theme.clone())
                    .set_size(ui.available_size());
                let response = ui.add(term);
                if response.clicked() {
                    self.terminal_focus = true;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("所有终端已关闭，点 + 新建")
                            .color(theme::color(self.mode, Token::Fg60)),
                    );
                });
            }
        });
    }
}

/// 系统 shell（unix=$SHELL 回退 sh；windows=cmd）。
fn system_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
    #[cfg(windows)]
    {
        "cmd.exe".into()
    }
}

/// 标签缺省标题。
fn default_title(id: u64) -> String {
    if id == 0 { "sh".into() } else { format!("sh-{id}") }
}

/// 供 AI 提案确认后向终端写入命令（BackendCommand::Write + 换行）。
/// v1：确认执行走 IPC 闸门（留痕），终端写入仅作为"把命令带进 shell"的快捷方式，
/// 默认不启用以避免绕过闸门（保留接口，v1.1 决策）。
#[allow(dead_code)]
fn write_to_terminal(backend: &mut TerminalBackend, command: &str) {
    backend.process_command(BackendCommand::Write(format!("{command}\n").into_bytes()));
}
