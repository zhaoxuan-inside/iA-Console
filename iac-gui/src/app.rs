//! ConsoleApp：iA-Writer 式任务流主界面（_04/03 设计语言的 GUI 实现）。
//! 主轴 = 输入行 + 任务流，无常驻侧栏/按钮（概念翻译表）；管理面板按需浮出用毕即收；
//! 真终端 = 全屏覆盖层（Ctrl+T / /term，AC-45），焦点互不抢占。
//! chg-023：丢弃 Konsole 式布局，按产品设计语言重做 UI。

use crate::sidebar::{self, SidebarState};
use crate::theme::{self, Mode, Token};
use egui_term::{
    AnsiColor, AnsiNamedColor, AnsiRgb, BackendCommand, BackendSettings, PtyEvent, TerminalBackend,
    TerminalTheme, TerminalView,
};
use iac_core::server::IpcResponse;
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};

/// IPC 响应回传信封：(方法名, 结果/错误)。
type IpcReply = (&'static str, Result<IpcResponse, String>);

/// 主应用。
pub struct ConsoleApp {
    /// 主题配置（day/night/auto，AC-28）；运行时解析为 Mode。
    theme_pref: iac_core::config::ThemeMode,
    mode: Mode,
    /// 真终端后端（覆盖层宿主；单会话，cd/环境变量持久）。
    terminal: Option<TerminalBackend>,
    terminal_open: bool,
    pty_tx: Sender<(u64, PtyEvent)>,
    pty_rx: Receiver<(u64, PtyEvent)>,
    ipc_tx: Sender<IpcReply>,
    ipc_rx: Receiver<IpcReply>,
    flow: SidebarState,
    /// 活动提案 id（accent 色条唯一来源）。
    active_proposal: Option<String>,
    /// 展开态提案（e 键双向）。
    expanded: HashSet<String>,
    /// /config 浮出面板开关。
    config_open: bool,
    /// 输入历史（↑↓ 翻阅）。
    input_history: Vec<String>,
    history_cursor: Option<usize>,
    /// 输入框焦点（键盘跟随焦点而非指针）。
    input_focus: bool,
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
        let theme_pref = iac_core::config::ThemeMode::Auto;
        let mode = resolve_mode(theme_pref, cc.egui_ctx.clone());
        let mut app = Self {
            theme_pref,
            mode,
            terminal: Some(backend),
            terminal_open: false,
            pty_tx,
            pty_rx,
            ipc_tx,
            ipc_rx,
            flow: SidebarState::default(),
            active_proposal: None,
            expanded: HashSet::new(),
            config_open: false,
            input_history: Vec::new(),
            history_cursor: None,
            input_focus: true,
            term_theme: TerminalTheme::new(Box::new(theme::terminal_palette(mode))),
        };
        app.flow.entries.push(sidebar::Entry::Output(WELCOME.into()));
        theme::apply_style(&cc.egui_ctx, mode);
        app
    }

    fn tab_mut(&mut self) -> Option<&mut TerminalBackend> {
        self.terminal.as_mut()
    }

    /// PTY 事件流：PtyWrite/颜色查询须应答，否则交互程序卡查询；Title 联动（预留）。
    fn drain_pty_events(&mut self, ctx: &egui::Context) {
        while let Ok((id, ev)) = self.pty_rx.try_recv() {
            match ev {
                PtyEvent::PtyWrite(text) => {
                    if let Some(tab) = self.tab_mut() {
                        tab.process_command(BackendCommand::Write(text.into_bytes()));
                    }
                }
                PtyEvent::ColorRequest(index, formatter) => {
                    let reply = formatter(self.query_color(index));
                    if let Some(tab) = self.tab_mut() {
                        tab.process_command(BackendCommand::Write(reply.into_bytes()));
                    }
                }
                PtyEvent::ClipboardStore(_, text) => ctx.copy_text(text),
                _ => {}
            }
        }
    }

    /// 颜色查询应答：OSC 10/11 与索引色全部取自当前令牌调色板。
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

    /// 异步 IPC 响应落任务流（每帧 drain，UI 永不等待）。
    fn drain_ipc_results(&mut self) {
        while let Ok((method, resp)) = self.ipc_rx.try_recv() {
            sidebar::apply_ipc_result(&mut self.flow, method, &resp);
            if method == "iac_ask" {
                self.active_proposal = self.flow.last_proposal.clone();
            }
        }
    }

    /// 输入提交：回显 → 分发 → 面板/主题/终端动作就地生效。
    fn submit_line(&mut self, line: &str) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        self.input_history.push(line.clone());
        self.history_cursor = None;
        self.flow.entries.push(sidebar::Entry::InputEcho(line.clone()));
        let action = sidebar::plan_action(&line, self.flow.last_proposal.is_some());
        match action {
            sidebar::Action::OpenConfig => self.config_open = true,
            sidebar::Action::CycleTheme => self.cycle_theme(),
            sidebar::Action::ToggleTerminal => self.terminal_open = !self.terminal_open,
            other => sidebar::perform(&mut self.flow, other, &self.ipc_tx),
        }
    }

    /// 主题循环 day→night→auto（AC-28）。
    fn cycle_theme(&mut self) {
        self.theme_pref = match self.theme_pref {
            iac_core::config::ThemeMode::Day => iac_core::config::ThemeMode::Night,
            iac_core::config::ThemeMode::Night => iac_core::config::ThemeMode::Auto,
            iac_core::config::ThemeMode::Auto => iac_core::config::ThemeMode::Day,
        };
        self.flow.entries.push(sidebar::Entry::Notice(format!(
            "主题 → {}（auto 跟随系统明暗）",
            match self.theme_pref {
                iac_core::config::ThemeMode::Day => "day",
                iac_core::config::ThemeMode::Night => "night",
                iac_core::config::ThemeMode::Auto => "auto",
            }
        )));
    }
}

impl eframe::App for ConsoleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_pty_events(ctx);
        self.drain_ipc_results();
        // AC-28：auto 主题跟随系统明暗
        let resolved = resolve_mode(self.theme_pref, ctx.clone());
        if resolved != self.mode {
            self.mode = resolved;
            self.term_theme = TerminalTheme::new(Box::new(theme::terminal_palette(self.mode)));
            theme::apply_style(ctx, self.mode);
        }
        self.handle_global_keys(ctx);

        if self.terminal_open {
            self.draw_terminal_overlay(ctx);
            return;
        }
        if self.config_open {
            self.draw_config_panel(ctx);
        }
        self.draw_task_flow(ctx);
    }
}

impl ConsoleApp {
    /// 全局键位：Ctrl+T 终端覆盖层；输入框失焦时 e/y 复制/提案键生效。
    fn handle_global_keys(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::T) && i.modifiers.ctrl) {
            self.terminal_open = !self.terminal_open;
        }
        if self.terminal_open || self.input_focus {
            return;
        }
        // 卡片操作（输入框未持键盘时）
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            if let Some(pid) = self.active_proposal.clone() {
                if !self.expanded.insert(pid.clone()) {
                    self.expanded.remove(&pid);
                }
                // 展开即按需解释（AC-01/35：解释是按需服务）
                self.flow.busy = false;
                sidebar::perform(
                    &mut self.flow,
                    sidebar::Action::Ipc("iac_explain", serde_json::json!({"command": pid})),
                    &self.ipc_tx,
                );
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::C) && i.modifiers.ctrl) {
            // Ctrl+C 保留给终端语义；卡片复制走 y 前缀
        }
    }

    /// 主轴：任务流 + 底部输入行（内容即界面，无框）。
    fn draw_task_flow(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // 输入行钉底（子面板先占位，滚动区填剩余）
            egui::TopBottomPanel::bottom("flow_input")
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)))
                .show_inside(ui, |ui| {
                    ui.separator();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.flow.input)
                            .hint_text("说人话 / 直接命令 / ??问 AI / /help")
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace),
                    );
                    self.input_focus = response.has_focus();
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let line = self.flow.input.trim().to_string();
                        self.flow.input.clear();
                        self.submit_line(&line);
                    }
                    // ↑↓ 翻阅输入历史
                    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) && !self.input_history.is_empty() {
                        let idx = match self.history_cursor {
                            Some(i) => i.saturating_sub(1),
                            None => self.input_history.len() - 1,
                        };
                        self.history_cursor = Some(idx);
                        self.flow.input = self.input_history[idx].clone();
                    }
                    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        if let Some(i) = self.history_cursor {
                            self.flow.input = self.input_history.get(i + 1).cloned().unwrap_or_default();
                            self.history_cursor = self.input_history.get(i + 1).is_some().then_some(i + 1);
                        }
                    }
                });
            ui.heading(egui::RichText::new("iA Console").color(theme::color(self.mode, Token::Fg)));
            ui.add_space(2.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    let entries = self.flow.entries.clone();
                    let n = entries.len();
                    for (i, entry) in entries.iter().enumerate() {
                        self.show_entry(ui, entry, i + 1 == n);
                        ui.add_space(4.0);
                    }
                    if self.flow.busy {
                        let dots = if ctx.input(|i| i.time) as u64 % 2 == 0 { "· ·" } else { "· · ·" };
                        ui.add(egui::Label::new(
                            egui::RichText::new(dots).color(theme::color(self.mode, Token::Fg25)).size(13.0),
                        ));
                    }
                });
        });
    }

    /// 单条条目渲染（无框化组件规范，_04/03 §4）。
    fn show_entry(&mut self, ui: &mut egui::Ui, entry: &sidebar::Entry, is_last: bool) {
        let mode = self.mode;
        let c = |t: Token| theme::color(mode, t);
        let rich = |text: &str, color: Color32, bold: bool| {
            let rt = egui::RichText::new(text).color(color).size(13.0);
            if bold { rt.strong() } else { rt }
        };
        use Color32;
        match entry {
            sidebar::Entry::InputEcho(line) => {
                // 标记字符弱化（同 iA Writer 弱化语法字符）
                let (marker, rest) = split_marker(line);
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(rich("▍", c(Token::Accent), true)));
                    if !marker.is_empty() {
                        ui.add(egui::Label::new(rich(marker, c(Token::Fg25), false)));
                    }
                    ui.add(egui::Label::new(rich(rest, c(Token::Fg), true)));
                });
            }
            sidebar::Entry::Output(text) => {
                ui.add(egui::Label::new(rich(text, c(Token::Fg), false)).wrap());
            }
            sidebar::Entry::Notice(text) => {
                // 错误 = 行内红字 + 错误码弱化，不弹窗
                ui.add(egui::Label::new(rich(text, c(Token::Destructive), false)).wrap());
            }
            sidebar::Entry::Separator => {
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::top_center(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(rich("· · ·", c(Token::Fg25), false)));
                });
                ui.add_space(4.0);
            }
            sidebar::Entry::Note(text) => {
                ui.add(egui::Label::new(rich(&format!("# {text}"), c(Token::Fg40), false)).wrap());
            }
            sidebar::Entry::Proposal { id, command, risk } => {
                let active = self.active_proposal.as_deref() == Some(id.as_str());
                let expanded = self.expanded.contains(id);
                let bar_color = if active { c(Token::Accent) } else { c(Token::Fg25) };
                let risk_color = risk_color(mode, risk);
                // 左侧 accent 色条 + 命令（bold），无外框
                ui.horizontal(|ui| {
                    bar(ui, bar_color);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(rich(command, c(Token::Fg), true)).wrap());
                        ui.add_space(1.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(rich(
                                &format!("提案 {id} · 风险：{risk}"),
                                c(Token::Fg60),
                                false,
                            )));
                            ui.add(egui::Label::new(rich(risk, risk_color, true)));
                        });
                        if expanded {
                            ui.add(egui::Label::new(rich(
                                "逐参数解释见下方说明条目（e 收起）",
                                c(Token::Fg40),
                                false,
                            )));
                        }
                        ui.horizontal(|ui| {
                            let actions = if active { "[执行 Enter/y] [解释 e] [取消 n]" } else { "[激活：点击] [解释 e]" };
                            ui.add(egui::Label::new(rich(actions, c(Token::Fg40), false)));
                        });
                    });
                });
            }
            sidebar::Entry::Translation { from, to, risk } => {
                // 原命令灰阶弱化 + → + 译文正常着色（AC-36）
                let warn = matches!(risk.as_str(), "dangerous" | "destructive");
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(rich(from, c(Token::Fg25), false)));
                    ui.add(egui::Label::new(rich("→", c(Token::Fg25), false)));
                    ui.add(egui::Label::new(rich(to, c(Token::Fg), true)));
                    if warn {
                        ui.add(egui::Label::new(rich("⚠", risk_color(mode, risk), true)));
                    }
                });
            }
            sidebar::Entry::Execution { command, exit, status, stdout, stderr, risk } => {
                let warn = matches!(risk.as_str(), "dangerous" | "destructive");
                let cmd_color = if warn { c(Token::Destructive) } else { c(Token::Fg) };
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(rich("$ ", c(Token::Fg25), false)));
                    ui.add(egui::Label::new(rich(command, cmd_color, true)).wrap());
                    if warn {
                        ui.add(egui::Label::new(rich("⚠", c(Token::Caution), true)));
                    }
                });
                ui.add(egui::Label::new(rich(&format!("  退出码 {exit} · {status}"), c(Token::Fg40), false)));
                if !stdout.is_empty() {
                    ui.add(egui::Label::new(rich(&indent(stdout), c(Token::Fg60), false)).wrap());
                }
                if !stderr.is_empty() {
                    ui.add(egui::Label::new(rich(&indent(stderr), c(Token::Fg60), false)).wrap());
                }
            }
            sidebar::Entry::List { title, rows } => {
                // 无斑马纹无表头：seq fg-25 / 正文 fg（G18 面板 = 任务流内列表）
                ui.add(egui::Label::new(rich(title, c(Token::Fg40), false)));
                for r in rows {
                    ui.add(egui::Label::new(rich(r, c(Token::Fg), false)));
                }
                if is_last && !rows.is_empty() {
                    ui.add(egui::Label::new(rich(
                        "回放：输入 rollback --to <行>（或 CLI iac rollback --to <行>）",
                        c(Token::Fg40),
                        false,
                    )));
                }
            }
        }
    }

    /// 真终端覆盖层（AC-45：全屏交互程序可用；Esc/Ctrl+T 返回任务流）。
    fn draw_terminal_overlay(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(2.0);
            ui.add(egui::Label::new(
                egui::RichText::new("真终端（Ctrl+T / Esc 返回任务流）")
                    .color(theme::color(self.mode, Token::Fg25))
                    .size(11.0),
            ));
            if let Some(tab) = self.terminal.as_mut() {
                let term = TerminalView::new(ui, tab)
                    .set_focus(true)
                    .set_theme(self.term_theme.clone())
                    .set_size(ui.available_size());
                let response = ui.add(term);
                // Esc 返回任务流（终端内 Esc 由应用消费优先级低：Ctrl+T 为主通道）
                if response.clicked() {
                    self.input_focus = false;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape) && i.modifiers.ctrl) {
                self.terminal_open = false;
                self.input_focus = true;
            }
        });
    }

    /// /config 浮出面板（用毕即收）：主题 / 习惯方言 / 后端说明。
    fn draw_config_panel(&mut self, ctx: &egui::Context) {
        let mode = self.mode;
        egui::Window::new("配置")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .resizable(false)
            .collapsible(false)
            .frame(egui::Frame::window(&ctx.style()).fill(theme::color(mode, Token::Bg)).stroke(egui::Stroke::new(1.0, theme::color(mode, Token::Fg25))))
            .show(ctx, |ui| {
                let c = |t: Token| theme::color(mode, t);
                ui.add(egui::Label::new(
                    egui::RichText::new("主题").color(c(Token::Fg60)).size(12.0),
                ));
                ui.horizontal(|ui| {
                    for (label, pref) in [
                        ("day", iac_core::config::ThemeMode::Day),
                        ("night", iac_core::config::ThemeMode::Night),
                        ("auto", iac_core::config::ThemeMode::Auto),
                    ] {
                        let selected = self.theme_pref == pref;
                        if ui.selectable_label(selected, label).clicked() {
                            self.theme_pref = pref;
                            self.mode = resolve_mode(pref, ctx.clone());
                            self.term_theme = TerminalTheme::new(Box::new(theme::terminal_palette(self.mode)));
                            theme::apply_style(ctx, self.mode);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.add(egui::Label::new(
                    egui::RichText::new("习惯方言 / 模型后端：iac config 向导（CLI，写 config.toml）")
                        .color(c(Token::Fg40))
                        .size(11.0),
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new("Esc 或关闭本窗口返回任务流").color(c(Token::Fg25)).size(11.0),
                ));
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.config_open = false;
                }
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.config_open = false;
        }
    }
}

use egui::Color32;

/// 输入行标记拆分：`??` `/` `---` `#` `\` 前缀弱化。
fn split_marker(line: &str) -> (&str, &str) {
    for p in ["??", "---", "/config", "/help", "/history", "/recipe", "/sessions", "/rollback", "/theme", "/term", "/script", "/", "#", "\\"] {
        if let Some(rest) = line.strip_prefix(p) {
            return (p, rest);
        }
    }
    ("", line)
}

/// 摘要行缩进（两空格，列表规范）。
fn indent(text: &str) -> String {
    text.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

/// 风险语义色。
fn risk_color(mode: Mode, risk: &str) -> Color32 {
    let token = match risk {
        "safe" => Token::Safe,
        "caution" => Token::Caution,
        "dangerous" => Token::Dangerous,
        _ => Token::Destructive,
    };
    theme::color(mode, token)
}

/// 左侧竖色条（2 cell 宽视觉近似：小矩形）。
fn bar(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 1.0, color);
}

/// 主题偏好 → 运行时模式（auto 跟随系统明暗，AC-28）。
fn resolve_mode(pref: iac_core::config::ThemeMode, ctx: egui::Context) -> Mode {
    match pref {
        iac_core::config::ThemeMode::Day => Mode::Day,
        iac_core::config::ThemeMode::Night => Mode::Night,
        iac_core::config::ThemeMode::Auto => {
            let dark = ctx.input(|i| i.viewport().dark_mode).unwrap_or(true);
            if dark { Mode::Night } else { Mode::Day }
        }
    }
}

/// 系统 shell（unix=$SHELL 回退 sh）。
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

const WELCOME: &str = "天下没有难用的 Console。直接输命令、说人话要命令（ask）、??问 AI；/help 看全部标记。Ctrl+T 开真终端。";
