//! ConsoleApp：主窗口状态与绘制（入口豁免）。
//! 中央 = egui_term 真终端（真实 shell，cd/交互程序持久）；右侧 = AI 侧栏（IPC → daemon）。

use crate::sidebar::{self, SidebarState};
use crate::theme::{self, Mode, Token};
use egui_term::{BackendCommand, BackendSettings, PtyEvent, TerminalBackend, TerminalView};
use std::sync::mpsc::{Receiver, Sender};

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
    sidebar: SidebarState,
}

impl ConsoleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let (tx, rx) = std::sync::mpsc::channel();
        let backend = TerminalBackend::new(
            0,
            cc.egui_ctx.clone(),
            tx.clone(),
            BackendSettings { shell: system_shell(), ..Default::default() },
        )
        .expect("终端后端初始化");
        let app = Self {
            mode: Mode::Night,
            tabs: vec![TermTab { id: 0, title: "sh".into(), backend }],
            active: 0,
            next_tab_id: 1,
            pty_tx: tx,
            pty_rx: rx,
            sidebar: SidebarState::default(),
        };
        theme::apply_style(&cc.egui_ctx, app.mode);
        app
    }
}

impl eframe::App for ConsoleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // PTY 退出事件 → 关闭对应标签
        while let Ok((id, ev)) = self.pty_rx.try_recv() {
            if matches!(ev, PtyEvent::Exit) {
                if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
                    self.tabs.remove(pos);
                    self.active = self
                        .active
                        .saturating_sub(1)
                        .min(self.tabs.len().saturating_sub(1));
                }
            }
        }

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
                        self.tabs.push(TermTab { id, title: format!("sh-{id}"), backend });
                        self.active = self.tabs.len() - 1;
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
                    theme::apply_style(ctx, self.mode);
                }
            });
        });
    }

    /// 右侧 AI 侧栏：条目流 + 输入行（Enter 提交，标记分发）。
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
                            };
                            ui.add_space(4.0);
                        }
                    });
                ui.separator();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.sidebar.input)
                        .hint_text("直接命令 / ??问 AI / yes 确认提案 / /help")
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let line = self.sidebar.input.trim().to_string();
                    self.sidebar.input.clear();
                    if !line.is_empty() {
                        let action =
                            sidebar::plan_action(&line, self.sidebar.last_proposal.is_some());
                        sidebar::perform(&mut self.sidebar, action);
                    }
                }
                response.request_focus();
            });
    }

    /// 中央真终端。
    fn draw_terminal(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                let term = TerminalView::new(ui, &mut tab.backend)
                    .set_focus(true)
                    .set_size(ui.available_size());
                ui.add(term);
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

/// 供 AI 提案确认后向终端写入命令（BackendCommand::Write + 换行）。
/// v1：确认执行走 IPC 闸门（留痕），终端写入仅作为"把命令带进 shell"的快捷方式，
/// 默认不启用以避免绕过闸门（保留接口，v1.1 决策）。
#[allow(dead_code)]
fn write_to_terminal(backend: &mut TerminalBackend, command: &str) {
    backend.process_command(BackendCommand::Write(format!("{command}\n").into_bytes()));
}
