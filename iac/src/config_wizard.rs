//! 交互式配置向导（`iac config` 裸命令，入口豁免；参照 opencode CLI 的交互式配置体验）。
//! 设计语言同 console.rs（iA 式）：内容即界面、无框线、留白、焦点行 ▍ accent。
//! 写入统一走 `iac_config_set`：daemon 热应用 + 落盘 + 审计；界面就地反馈保存结果与错误码。
//! 本地快照以 daemon 回包 config 为准（保存成功回填、失败回滚），不自行造第二事实。

use crate::theme::{Mode, Token};
use iac_core::config::ProviderProfile;
use iac_core::types::ProviderKind;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde_json::{json, Value};

/// 可编辑键类别。
pub enum Kind {
    /// 枚举：合法 serde 值（core 词典同名）。
    Enum(&'static [&'static str]),
    Bool,
    U32,
    U64,
    Text,
    /// provider 档案列表（子界面编辑，整组 JSON 写回）。
    Providers,
}

/// 一条可配置项（静态目录驱动整个向导：键序=展示序）。
pub struct Item {
    pub key: &'static str,
    pub section: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub hint: &'static str,
}

/// 配置目录（Config schema 全量标量键 + providers）。
pub static CATALOG: &[Item] = &[
    Item {
        key: "ui.theme",
        section: "界面",
        label: "主题",
        kind: Kind::Enum(&["auto", "day", "night"]),
        hint: "day 日间 / night 夜间 / auto 跟随环境",
    },
    Item {
        key: "ui.habit_dialect",
        section: "界面",
        label: "习惯方言",
        kind: Kind::Enum(&["linux", "macos", "windows"]),
        hint: "translate 的目标平台方言",
    },
    Item {
        key: "ui.clipboard",
        section: "界面",
        label: "剪贴板",
        kind: Kind::Enum(&["osc52", "off"]),
        hint: "复制通道：osc52 终端回写 / off 关闭",
    },
    Item {
        key: "ai.default_provider",
        section: "模型",
        label: "缺省 provider",
        kind: Kind::Text,
        hint: "取下方 provider 档案名",
    },
    Item {
        key: "ai.fallback_local",
        section: "模型",
        label: "本地回退",
        kind: Kind::Bool,
        hint: "远端不可用时回退本地模型",
    },
    Item {
        key: "providers",
        section: "模型",
        label: "provider 档案",
        kind: Kind::Providers,
        hint: "远端/本地模型后端档案（Enter 进入管理）",
    },
    Item {
        key: "modelhub.source",
        section: "模型插件",
        label: "下载源",
        kind: Kind::Enum(&["Hf", "Modelscope", "hf-mirror"]),
        hint: "模型插件拉取源",
    },
    Item {
        key: "rollback.enabled",
        section: "回滚",
        label: "总开关",
        kind: Kind::Bool,
        hint: "效果日志回滚机制开关",
    },
    Item {
        key: "rollback.capture_scope",
        section: "回滚",
        label: "捕获范围",
        kind: Kind::Enum(&["cwdtree", "explicit", "allhome"]),
        hint: "快照捕获：cwdtree 工作树 / explicit 显式 / allhome 全主目录",
    },
    Item {
        key: "rollback.max_entries",
        section: "回滚",
        label: "条目上限",
        kind: Kind::U32,
        hint: "回滚点硬上限（≤65536）",
    },
    Item {
        key: "rollback.on_conflict",
        section: "回滚",
        label: "冲突策略",
        kind: Kind::Enum(&["ask", "skip", "abort"]),
        hint: "回放遇冲突：ask 逐项问 / skip 跳过 / abort 中止",
    },
    Item {
        key: "rollback.retention_days",
        section: "回滚",
        label: "保留天数",
        kind: Kind::U32,
        hint: "回滚快照保留天数",
    },
    Item {
        key: "rollback.retention_max_bytes",
        section: "回滚",
        label: "保留容量",
        kind: Kind::U64,
        hint: "回滚快照保留容量（字节）",
    },
    Item {
        key: "history.retention_days",
        section: "历史",
        label: "保留天数",
        kind: Kind::U32,
        hint: "执行记录保留天数（双限 LRU）",
    },
    Item {
        key: "history.retention_max_bytes",
        section: "历史",
        label: "保留容量",
        kind: Kind::U64,
        hint: "执行记录保留容量（字节）",
    },
    Item {
        key: "render.output_detection",
        section: "渲染",
        label: "语言检测",
        kind: Kind::Bool,
        hint: "输出语言自动检测（高亮接管）",
    },
    Item {
        key: "exec.timeout_ms",
        section: "执行",
        label: "超时",
        kind: Kind::U64,
        hint: "命令执行超时（毫秒）",
    },
    Item {
        key: "exec.stdout_summary_max",
        section: "执行",
        label: "stdout 上限",
        kind: Kind::U64,
        hint: "stdout 摘要上限（字节；全量输出另行入库）",
    },
    Item {
        key: "exec.stderr_summary_max",
        section: "执行",
        label: "stderr 上限",
        kind: Kind::U64,
        hint: "stderr 摘要上限（字节）",
    },
];

/// provider 表单字段（序号=展示序；kind 走 Enter 轮换不进文本编辑）。
static FORM_FIELDS: &[(&str, &str)] = &[
    ("name", "名称（档案唯一名）"),
    ("kind", "类型（Enter 轮换）"),
    ("base_url", "接口地址（远端必填）"),
    ("api_key_env", "密钥环境变量名（远端必填；不存密钥本体）"),
    ("model", "模型名（远端必填）"),
    ("path", "本地模型路径（local-path 必填）"),
];

const KIND_CYCLE: &[&str] = &["Openai", "Anthropic", "local-plugin", "local-path"];

/// 点路径取值（快照 JSON）。
pub fn value_of(cfg: &Value, key: &str) -> Value {
    key.split('.').fold(cfg.clone(), |acc, k| {
        acc.get(k).cloned().unwrap_or(Value::Null)
    })
}

/// 值的行内展示（字符串裸显、Null 破折号）。
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

/// 文本输入合法性（数值键按 parse 校验；文本非空）。
pub fn input_ok(kind: &Kind, s: &str) -> bool {
    match kind {
        Kind::U32 => s.parse::<u32>().is_ok(),
        Kind::U64 => s.parse::<u64>().is_ok(),
        Kind::Text => !s.trim().is_empty(),
        _ => true,
    }
}

/// provider 字段读取（表单展示/预填）。
pub fn field_str(p: &ProviderProfile, field: &str) -> String {
    match field {
        "name" => p.name.clone(),
        "kind" => kind_name(&p.kind).into(),
        "base_url" => p.base_url.clone().unwrap_or_default(),
        "api_key_env" => p.api_key_env.clone().unwrap_or_default(),
        "model" => p.model.clone().unwrap_or_default(),
        "path" => p.path.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// provider 字段写入（空串→None，可选字段留空即清除）。
pub fn set_field(p: &mut ProviderProfile, field: &str, v: &str) {
    let trimmed = v.trim().to_string();
    let opt = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.clone())
    };
    match field {
        "name" => p.name = trimmed,
        "base_url" => p.base_url = opt,
        "api_key_env" => p.api_key_env = opt,
        "model" => p.model = opt,
        "path" => p.path = opt,
        _ => {}
    }
}

/// kind 的 serde 名（展示与轮换的统一出处）。
fn kind_name(k: &ProviderKind) -> &'static str {
    match k {
        ProviderKind::Openai => "Openai",
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::LocalPlugin => "local-plugin",
        ProviderKind::LocalPath => "local-path",
    }
}

/// kind 轮换（Enter 切换四类）。
fn kind_next(k: &ProviderKind) -> ProviderKind {
    let cur = KIND_CYCLE
        .iter()
        .position(|c| *c == kind_name(k))
        .unwrap_or(0);
    let next = KIND_CYCLE[(cur + 1) % KIND_CYCLE.len()];
    serde_json::from_value(json!(next)).unwrap_or(ProviderKind::LocalPlugin)
}

/// 中日韩宽度感知的左对齐补空（列表值列对齐）。
fn pad(s: &str, width: usize) -> String {
    let w: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

/// 保存反馈（底部状态行）。
enum Status {
    None,
    Saved(String),
    Failed(String),
}

enum Screen {
    List,
    Edit,
    Providers,
    Form,
}

/// 标量键编辑态（枚举用 picker，文本/数值用 input）。
struct EditState {
    item: usize,
    picker: usize,
    input: String,
}

/// 向导状态机：screen × 光标 × 编辑缓冲。
pub struct Wizard {
    mode: Mode,
    cfg: Value,
    selected: usize,
    screen: Screen,
    edit: Option<EditState>,
    providers: Vec<ProviderProfile>,
    prov_selected: usize,
    form_idx: usize,
    form_field: usize,
    /// 表单字段文本编辑缓冲。
    form_input: Option<(String, String)>,
    status: Status,
}

impl Wizard {
    fn new(cfg: Value) -> Self {
        let providers = serde_json::from_value::<Vec<ProviderProfile>>(value_of(&cfg, "providers"))
            .unwrap_or_default();
        let mode = match value_of(&cfg, "ui.theme").as_str() {
            Some("day") => Mode::Day,
            _ => Mode::Night,
        };
        Self {
            mode,
            cfg,
            selected: 0,
            screen: Screen::List,
            edit: None,
            providers,
            prov_selected: 0,
            form_idx: 0,
            form_field: 0,
            form_input: None,
            status: Status::None,
        }
    }

    /// 写一个键：daemon 回包即新事实（成功回填快照、失败回滚本地副本）。
    fn save(&mut self, key: &str, value: &str) {
        let resp = crate::ipc::request("iac_config_set", json!({"key": key, "value": value}), None);
        match resp {
            Ok(r) if r.ok => {
                if let Some(cfg) = r.result.as_ref().and_then(|x| x.get("config")) {
                    self.cfg = cfg.clone();
                }
                self.status = Status::Saved(format!("{key} = {value}"));
            }
            Ok(r) => {
                self.status = Status::Failed(crate::ipc::render(&r, false));
            }
            Err(e) => self.status = Status::Failed(e),
        }
        self.sync_providers();
    }

    /// 以 daemon 快照为准回同步 provider 本地副本。
    fn sync_providers(&mut self) {
        if let Ok(p) =
            serde_json::from_value::<Vec<ProviderProfile>>(value_of(&self.cfg, "providers"))
        {
            self.providers = p;
            self.prov_selected = self
                .prov_selected
                .min(self.providers.len().saturating_sub(1));
        }
    }

    /// providers 整组写回。
    fn save_providers(&mut self) {
        let body = serde_json::to_string(&self.providers).unwrap_or_else(|_| "[]".into());
        self.save("providers", &body);
    }

    /// 新增档案（缺省 local-plugin，名字去重）。
    fn provider_add(&mut self) {
        let mut n = self.providers.len() + 1;
        let mut name = format!("provider-{n}");
        while self.providers.iter().any(|p| p.name == name) {
            n += 1;
            name = format!("provider-{n}");
        }
        self.providers.push(ProviderProfile {
            name,
            kind: ProviderKind::LocalPlugin,
            base_url: None,
            api_key_env: None,
            model: None,
            path: None,
        });
        self.prov_selected = self.providers.len() - 1;
        self.save_providers();
    }

    fn provider_remove(&mut self) {
        if self.prov_selected < self.providers.len() {
            self.providers.remove(self.prov_selected);
            self.prov_selected = self.prov_selected.saturating_sub(1);
            self.save_providers();
        }
    }

    /// List 行 Enter：按类别分派（bool 即切即存；enum/文本进编辑；档案进子界面）。
    fn open_item(&mut self) {
        let idx = self.selected.min(CATALOG.len() - 1);
        let item = &CATALOG[idx];
        match &item.kind {
            Kind::Bool => {
                let cur = value_of(&self.cfg, item.key).as_bool().unwrap_or(false);
                let next = if cur { "false" } else { "true" };
                self.save(item.key, next);
            }
            Kind::Providers => self.screen = Screen::Providers,
            Kind::Enum(opts) => {
                let cur = display(&value_of(&self.cfg, item.key));
                self.edit = Some(EditState {
                    item: idx,
                    picker: opts.iter().position(|o| *o == cur).unwrap_or(0),
                    input: String::new(),
                });
                self.screen = Screen::Edit;
            }
            _ => {
                self.edit = Some(EditState {
                    item: idx,
                    picker: 0,
                    input: display(&value_of(&self.cfg, item.key)),
                });
                self.screen = Screen::Edit;
            }
        }
    }

    /// Edit 行确认：枚举取 picker、文本取 input（校验后保存）。
    fn confirm_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        let item = &CATALOG[e.item];
        let value = match &item.kind {
            Kind::Enum(opts) => opts.get(e.picker).copied().unwrap_or_default().to_string(),
            _ => e.input.trim().to_string(),
        };
        if input_ok(&item.kind, &value) {
            self.save(item.key, &value);
        } else {
            self.status = Status::Failed(format!("输入非法：{value}"));
        }
        self.screen = Screen::List;
    }

    /// Form 字段确认：写入本地档案并整组保存。
    fn confirm_form_input(&mut self) {
        let Some((field, text)) = self.form_input.take() else {
            return;
        };
        if let Some(p) = self.providers.get_mut(self.form_idx) {
            set_field(p, &field, &text);
            self.save_providers();
        }
    }

    /// 单键事件 → 状态迁移；返回 false 退出向导。
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return false;
        }
        if key.kind != crossterm::event::KeyEventKind::Press {
            return true;
        }
        match (&self.screen, key.code) {
            (Screen::List, KeyCode::Esc) => return false,
            (Screen::List, KeyCode::Up) => self.selected = self.selected.saturating_sub(1),
            (Screen::List, KeyCode::Down) => {
                self.selected = (self.selected + 1).min(CATALOG.len() - 1)
            }
            (Screen::List, KeyCode::Enter) => self.open_item(),
            (Screen::Edit, KeyCode::Esc) => {
                self.edit = None;
                self.screen = Screen::List;
            }
            (Screen::Edit, KeyCode::Up) => {
                if let Some(e) = self.edit.as_mut() {
                    if let Kind::Enum(opts) = &CATALOG[e.item].kind {
                        e.picker = e.picker.saturating_sub(1).min(opts.len() - 1);
                    }
                }
            }
            (Screen::Edit, KeyCode::Down) => {
                if let Some(e) = self.edit.as_mut() {
                    if let Kind::Enum(opts) = &CATALOG[e.item].kind {
                        e.picker = (e.picker + 1).min(opts.len() - 1);
                    }
                }
            }
            (Screen::Edit, KeyCode::Enter) => self.confirm_edit(),
            (Screen::Edit, KeyCode::Backspace) => {
                if let Some(e) = self.edit.as_mut() {
                    e.input.pop();
                }
            }
            (Screen::Edit, KeyCode::Char(c)) => {
                if let Some(e) = self.edit.as_mut() {
                    if !matches!(CATALOG[e.item].kind, Kind::Enum(_)) {
                        e.input.push(c);
                    }
                }
            }
            (Screen::Providers, KeyCode::Esc) => self.screen = Screen::List,
            (Screen::Providers, KeyCode::Up) => {
                self.prov_selected = self.prov_selected.saturating_sub(1)
            }
            (Screen::Providers, KeyCode::Down) => {
                if !self.providers.is_empty() {
                    self.prov_selected = (self.prov_selected + 1).min(self.providers.len() - 1);
                }
            }
            (Screen::Providers, KeyCode::Enter) => {
                if self.prov_selected < self.providers.len() {
                    self.form_idx = self.prov_selected;
                    self.form_field = 0;
                    self.form_input = None;
                    self.screen = Screen::Form;
                }
            }
            (Screen::Providers, KeyCode::Char('n')) => self.provider_add(),
            (Screen::Providers, KeyCode::Char('d')) => self.provider_remove(),
            (Screen::Form, KeyCode::Esc) => {
                if self.form_input.is_some() {
                    self.form_input = None;
                } else {
                    self.screen = Screen::Providers;
                }
            }
            (Screen::Form, KeyCode::Up) => {
                if self.form_input.is_none() {
                    self.form_field = self.form_field.saturating_sub(1);
                }
            }
            (Screen::Form, KeyCode::Down) => {
                if self.form_input.is_none() {
                    self.form_field = (self.form_field + 1).min(FORM_FIELDS.len() - 1);
                }
            }
            (Screen::Form, KeyCode::Enter) => {
                if self.form_input.is_some() {
                    self.confirm_form_input();
                } else {
                    let field = FORM_FIELDS[self.form_field].0;
                    if field == "kind" {
                        if let Some(p) = self.providers.get_mut(self.form_idx) {
                            p.kind = kind_next(&p.kind);
                            self.save_providers();
                        }
                    } else if let Some(p) = self.providers.get(self.form_idx) {
                        self.form_input = Some((field.into(), field_str(p, field)));
                    }
                }
            }
            (Screen::Form, KeyCode::Backspace) => {
                if let Some((_, buf)) = self.form_input.as_mut() {
                    buf.pop();
                }
            }
            (Screen::Form, KeyCode::Char(c)) => {
                if let Some((_, buf)) = self.form_input.as_mut() {
                    buf.push(c);
                }
            }
            _ => {}
        }
        true
    }
}

/// 入口（iac config 裸命令）：TTY 门卫 → 拉配置 → 全屏向导。
pub fn run() -> i32 {
    use crossterm::tty::IsTty;
    if !std::io::stdout().is_tty() {
        eprintln!("iac config 需要交互终端；非交互场景用 iac config set <key> <value>");
        return 2;
    }
    let cfg = match crate::ipc::request("iac_config_get", json!({}), None) {
        Ok(r) if r.ok => r.result.unwrap_or(Value::Null),
        Ok(r) => {
            eprintln!("{}", crate::ipc::render(&r, false));
            return 1;
        }
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut wiz = Wizard::new(cfg);
    let mut terminal = match crate::console::setup() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("终端初始化失败: {e}");
            return 1;
        }
    };
    loop {
        let _ = terminal.draw(|f| draw(f, &wiz));
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(key)) => {
                if !wiz.handle_key(key) {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    crate::console::teardown(terminal);
    0
}

/// 主列表行模型（section 头 + 条目行）。
enum Row {
    Header(&'static str),
    Entry(usize),
}

fn draw(f: &mut Frame, wiz: &Wizard) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let (lines, scroll) = match wiz.screen {
        Screen::List => {
            let rows = list_rows();
            let lines: Vec<Line> = rows.iter().map(|r| row_line(wiz, r)).collect();
            let pos = rows
                .iter()
                .position(|r| matches!(r, Row::Entry(i) if *i == wiz.selected))
                .unwrap_or(0);
            let visible = chunks[0].height as usize;
            (lines, (pos + 1).saturating_sub(visible) as u16)
        }
        Screen::Edit => (edit_lines(wiz), 0),
        Screen::Providers => (providers_lines(wiz), 0),
        Screen::Form => (form_lines(wiz), 0),
    };
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), chunks[0]);
    f.render_widget(Paragraph::new(bottom_line(wiz)), chunks[1]);
}

/// 列表行序（section 变更处插头）。
fn list_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut last = "";
    for (i, item) in CATALOG.iter().enumerate() {
        if item.section != last {
            rows.push(Row::Header(item.section));
            last = item.section;
        }
        rows.push(Row::Entry(i));
    }
    rows
}

fn row_line(wiz: &Wizard, row: &Row) -> Line<'static> {
    match row {
        Row::Header(s) => Line::from(Span::styled(
            format!("  {s}"),
            Style::default().fg(hex(wiz.mode, Token::Fg60)),
        )),
        Row::Entry(i) => {
            let item = &CATALOG[*i];
            let selected = *i == wiz.selected;
            let marker = if selected { " ▍ " } else { "   " };
            let value = if matches!(item.kind, Kind::Providers) {
                let n = wiz.providers.len();
                format!("{n} 个档案 →")
            } else {
                display(&value_of(&wiz.cfg, item.key))
            };
            let mut spans = vec![Span::styled(
                marker.to_string(),
                Style::default().fg(hex(wiz.mode, Token::Accent)),
            )];
            spans.push(Span::styled(
                pad(item.label, 14),
                Style::default()
                    .fg(hex(wiz.mode, Token::Fg))
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
            spans.push(Span::styled(
                value,
                Style::default().fg(if selected {
                    hex(wiz.mode, Token::Accent)
                } else {
                    hex(wiz.mode, Token::Fg60)
                }),
            ));
            Line::from(spans)
        }
    }
}

/// Edit 屏：标题 + 提示 + 枚举选项 / 输入行。
fn edit_lines(wiz: &Wizard) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(e) = wiz.edit.as_ref() else {
        return lines;
    };
    let item = &CATALOG[e.item];
    lines.push(Line::from(Span::styled(
        format!("  {} · {}", item.section, item.label),
        Style::default().fg(hex(wiz.mode, Token::Fg60)),
    )));
    lines.push(Line::from(Span::styled(
        format!("  {}", item.hint),
        Style::default().fg(hex(wiz.mode, Token::Fg40)),
    )));
    lines.push(Line::from(""));
    match &item.kind {
        Kind::Enum(opts) => {
            let cur = display(&value_of(&wiz.cfg, item.key));
            for (i, opt) in opts.iter().enumerate() {
                let cursor = i == e.picker;
                let marker = if cursor { " ▍ " } else { "   " };
                let mut spans = vec![Span::styled(
                    marker.to_string(),
                    Style::default().fg(hex(wiz.mode, Token::Accent)),
                )];
                spans.push(Span::styled(
                    *opt,
                    Style::default()
                        .fg(hex(wiz.mode, Token::Fg))
                        .add_modifier(if cursor {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ));
                if *opt == cur {
                    spans.push(Span::styled(
                        "  · 当前".to_string(),
                        Style::default().fg(hex(wiz.mode, Token::Fg40)),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
        _ => {
            lines.push(Line::from(vec![
                Span::styled(
                    " ▍ ".to_string(),
                    Style::default().fg(hex(wiz.mode, Token::Accent)),
                ),
                Span::styled(
                    e.input.clone(),
                    Style::default()
                        .fg(hex(wiz.mode, Token::Fg))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }
    lines
}

/// Providers 屏：档案列表 + 空态。
fn providers_lines(wiz: &Wizard) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        format!("  provider 档案（{}）", wiz.providers.len()),
        Style::default().fg(hex(wiz.mode, Token::Fg60)),
    ))];
    if wiz.providers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  暂无档案；按 n 新增".to_string(),
            Style::default().fg(hex(wiz.mode, Token::Fg40)),
        )));
        return lines;
    }
    for (i, p) in wiz.providers.iter().enumerate() {
        let cursor = i == wiz.prov_selected;
        let marker = if cursor { " ▍ " } else { "   " };
        let tail = p
            .model
            .clone()
            .or_else(|| p.path.clone())
            .unwrap_or_else(|| p.base_url.clone().unwrap_or_default());
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                Style::default().fg(hex(wiz.mode, Token::Accent)),
            ),
            Span::styled(
                pad(&p.name, 14),
                Style::default()
                    .fg(hex(wiz.mode, Token::Fg))
                    .add_modifier(if cursor {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!("{}  {}", kind_name(&p.kind), tail),
                Style::default().fg(hex(wiz.mode, Token::Fg60)),
            ),
        ]));
    }
    lines
}

/// Form 屏：字段表 + 编辑中输入行。
fn form_lines(wiz: &Wizard) -> Vec<Line<'static>> {
    let Some(p) = wiz.providers.get(wiz.form_idx) else {
        return providers_lines(wiz);
    };
    let mut lines = vec![Line::from(Span::styled(
        format!("  provider · {}", p.name),
        Style::default().fg(hex(wiz.mode, Token::Fg60)),
    ))];
    for (i, (field, label)) in FORM_FIELDS.iter().enumerate() {
        let cursor = i == wiz.form_field;
        let marker = if cursor { " ▍ " } else { "   " };
        let value = field_str(p, field);
        let shown = if value.is_empty() {
            "—".to_string()
        } else {
            value
        };
        lines.push(Line::from(vec![
            Span::styled(
                marker.to_string(),
                Style::default().fg(hex(wiz.mode, Token::Accent)),
            ),
            Span::styled(
                pad(label.split('（').next().unwrap_or(label), 10),
                Style::default().fg(hex(wiz.mode, Token::Fg60)),
            ),
            Span::styled(
                shown,
                Style::default()
                    .fg(hex(wiz.mode, Token::Fg))
                    .add_modifier(if cursor {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
    }
    lines.push(Line::from(""));
    if let Some((field, buf)) = wiz.form_input.as_ref() {
        let label = FORM_FIELDS
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, l)| *l)
            .unwrap_or(field);
        lines.push(Line::from(Span::styled(
            format!("  {label}"),
            Style::default().fg(hex(wiz.mode, Token::Fg40)),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                " ▍ ".to_string(),
                Style::default().fg(hex(wiz.mode, Token::Accent)),
            ),
            Span::styled(
                buf.clone(),
                Style::default()
                    .fg(hex(wiz.mode, Token::Fg))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {}", FORM_FIELDS[wiz.form_field].1),
            Style::default().fg(hex(wiz.mode, Token::Fg40)),
        )));
    }
    lines
}

/// 底部行：屏内键位提示 + 最近保存反馈。
fn bottom_line(wiz: &Wizard) -> Line<'static> {
    let help = match wiz.screen {
        Screen::List => "↑↓ 选择 · Enter 编辑 · Esc 退出",
        Screen::Edit => "↑↓ 选项 · Enter 保存 · Esc 取消",
        Screen::Providers => "↑↓ 选择 · Enter 编辑 · n 新增 · d 删除 · Esc 返回",
        Screen::Form => "↑↓ 选择 · Enter 编辑 · Esc 返回",
    };
    let mut spans = vec![Span::styled(
        format!(" {help}"),
        Style::default().fg(hex(wiz.mode, Token::Fg40)),
    )];
    let status = match &wiz.status {
        Status::None => None,
        Status::Saved(s) => Some((format!("已保存 {s}"), Token::Accent)),
        Status::Failed(s) => Some((s.clone(), Token::Dangerous)),
    };
    if let Some((text, token)) = status {
        spans.push(Span::styled("   ".to_string(), Style::default()));
        spans.push(Span::styled(
            text,
            Style::default().fg(hex(wiz.mode, token)),
        ));
    }
    Line::from(spans)
}

fn hex(mode: Mode, token: Token) -> Color {
    let (r, g, b) = crate::theme::rgb(mode, token);
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keys_unique_and_complete() {
        let keys: Vec<&str> = CATALOG.iter().map(|i| i.key).collect();
        let uniq: std::collections::HashSet<&str> = keys.iter().copied().collect();
        assert_eq!(keys.len(), uniq.len(), "配置键不重复");
        for must in [
            "ui.theme",
            "ui.habit_dialect",
            "ui.clipboard",
            "ai.default_provider",
            "ai.fallback_local",
            "providers",
            "modelhub.source",
            "rollback.enabled",
            "rollback.capture_scope",
            "rollback.max_entries",
            "rollback.on_conflict",
            "rollback.retention_days",
            "rollback.retention_max_bytes",
            "history.retention_days",
            "history.retention_max_bytes",
            "render.output_detection",
            "exec.timeout_ms",
            "exec.stdout_summary_max",
            "exec.stderr_summary_max",
        ] {
            assert!(keys.contains(&must), "目录缺 {must}");
        }
        // 分组连续（列表头按 section 变更插入；新 section 不得回头出现）
        let mut seen = std::collections::HashSet::from([CATALOG[0].section]);
        for w in CATALOG.windows(2) {
            if w[0].section != w[1].section {
                assert!(
                    seen.insert(w[1].section),
                    "section {} 重复出现（须连续）",
                    w[1].section
                );
            }
        }
    }

    #[test]
    fn enum_options_are_serde_legal() {
        use iac_core::types::*;
        for item in CATALOG {
            if let Kind::Enum(opts) = &item.kind {
                for o in *opts {
                    let ok = match item.key {
                        "ui.theme" => serde_json::from_value::<ThemeMode>(json!(o)).is_ok(),
                        "ui.habit_dialect" => serde_json::from_value::<Platform>(json!(o)).is_ok(),
                        "ui.clipboard" => serde_json::from_value::<ClipboardMode>(json!(o)).is_ok(),
                        "modelhub.source" => {
                            serde_json::from_value::<ModelhubSource>(json!(o)).is_ok()
                        }
                        "rollback.capture_scope" => {
                            serde_json::from_value::<CaptureScope>(json!(o)).is_ok()
                        }
                        "rollback.on_conflict" => {
                            serde_json::from_value::<ConflictPolicy>(json!(o)).is_ok()
                        }
                        _ => false,
                    };
                    assert!(ok, "{} 选项 {o} 不是合法 serde 值", item.key);
                }
            }
        }
    }

    #[test]
    fn value_of_walks_dotted_path_and_display() {
        let cfg = json!({"ui": {"theme": "night"}, "exec": {"timeout_ms": 120000}});
        assert_eq!(display(&value_of(&cfg, "ui.theme")), "night");
        assert_eq!(display(&value_of(&cfg, "exec.timeout_ms")), "120000");
        assert_eq!(display(&value_of(&cfg, "no.such")), "—");
    }

    #[test]
    fn input_ok_validates_by_kind() {
        assert!(input_ok(&Kind::U32, "90"));
        assert!(!input_ok(&Kind::U32, "-1"));
        assert!(!input_ok(&Kind::U32, "abc"));
        assert!(input_ok(&Kind::U64, "1073741824"));
        assert!(!input_ok(&Kind::U64, ""));
        assert!(input_ok(&Kind::Text, "remote"));
        assert!(!input_ok(&Kind::Text, "  "));
    }

    #[test]
    fn provider_fields_round_trip() {
        let mut p = ProviderProfile {
            name: "remote".into(),
            kind: ProviderKind::Openai,
            base_url: None,
            api_key_env: None,
            model: None,
            path: None,
        };
        set_field(&mut p, "base_url", "https://api.example.com");
        set_field(&mut p, "api_key_env", "OPENAI_KEY");
        set_field(&mut p, "model", "gpt-x");
        assert_eq!(field_str(&p, "base_url"), "https://api.example.com");
        assert_eq!(field_str(&p, "api_key_env"), "OPENAI_KEY");
        // 空串清除可选字段；名称直写
        set_field(&mut p, "model", " ");
        assert_eq!(field_str(&p, "model"), "");
        set_field(&mut p, "name", "renamed");
        assert_eq!(p.name, "renamed");
    }

    #[test]
    fn providers_json_round_trip_matches_daemon_contract() {
        let list = vec![ProviderProfile {
            name: "remote".into(),
            kind: ProviderKind::Openai,
            base_url: Some("https://api.example.com".into()),
            api_key_env: Some("OPENAI_KEY".into()),
            model: Some("gpt-x".into()),
            path: None,
        }];
        let body = serde_json::to_string(&list).expect("序列化");
        let back: Vec<ProviderProfile> = serde_json::from_str(&body).expect("反序列化");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "remote");
        assert_eq!(kind_name(&back[0].kind), "Openai");
    }

    #[test]
    fn kind_cycle_visits_all_four() {
        let mut k = ProviderKind::LocalPlugin;
        let mut seen = vec![kind_name(&k)];
        for _ in 0..4 {
            k = kind_next(&k);
            seen.push(kind_name(&k));
        }
        for expect in ["Openai", "Anthropic", "local-plugin", "local-path"] {
            assert!(seen.contains(&expect), "轮换覆盖 {expect}：{seen:?}");
        }
    }

    #[test]
    fn pad_aligns_cjk_labels() {
        assert_eq!(pad("主题", 6), "主题  ");
        assert_eq!(pad("abc", 6), "abc   ");
        assert_eq!(pad("超长中文名", 4), "超长中文名");
    }
}
