//! iac CLI 入口（框架签名豁免）：clap 子命令 → IPC → 渲染。
//! 标记语言（?? / --- / #）在 TUI 输入行解析；CLI 为等价子命令出口。

use clap::{Parser, Subcommand};

/// iA-console 前端：让天下没有难用的 Console。
#[derive(Parser)]
#[command(name = "iac", version, about)]
struct Cli {
    /// 结构化 JSON 输出
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 意图 → 提案（说人话拿命令）
    Ask {
        intent: String,
        #[arg(long)]
        cwd: Option<String>,
        /// 展开逐参数详解（默认摘要态）
        #[arg(long)]
        explain: bool,
    },
    /// 执行命令（过风险闸门；人类直输弱提醒不拦截）
    Run {
        #[command(flatten)]
        target: RunTarget,
        /// 翻译高危/Agent 路径的确认标志
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// 参数白话解释
    Explain { command: Vec<String> },
    /// 诊断最近/指定失败执行
    Doctor {
        #[arg(long)]
        execution: Option<String>,
    },
    /// 历史检索
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },
    /// 方案库
    Recipe {
        #[command(subcommand)]
        action: RecipeAction,
    },
    /// 会话管理
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// 配置
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// daemon 生命周期
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// MCP stdio 服务（供外部 Agent 接入）
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// TUI 交互界面（M6 落地）
    Chat,
    /// 习惯方言翻译（M5 落地）
    Translate {
        #[arg(long, value_enum)]
        from: Option<Dialect>,
        command: Vec<String>,
    },
}

#[derive(clap::ValueEnum, Clone)]
enum Dialect {
    Linux,
    Macos,
    Windows,
}

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
struct RunTarget {
    /// 提案 ID（确认后执行）
    #[arg(long)]
    proposal: Option<String>,
    /// 直接执行的命令
    command: Vec<String>,
}

#[derive(Subcommand)]
enum HistoryAction {
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Search { q: String },
    Show { id: String },
}

#[derive(Subcommand)]
enum RecipeAction {
    Save {
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "with")]
        command: Vec<String>,
        #[arg(long)]
        tags: Vec<String>,
    },
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Search { q: String },
    Show { id: String },
    Rm { id: String },
}

#[derive(Subcommand)]
enum SessionAction {
    New,
    List,
    Show { id: String },
    Resume { id: String },
}

#[derive(Subcommand)]
enum ConfigAction {
    Get { key: String },
    Path,
}

#[derive(Subcommand)]
enum DaemonAction {
    Start,
    Status,
    Stop,
}

#[derive(Subcommand)]
enum McpAction {
    Serve,
}

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    let code = run(cli, json_mode);
    std::process::exit(code);
}

fn run(cli: Cli, json_mode: bool) -> i32 {
    match cli.command {
        Commands::Ask { intent, cwd, explain } => {
            let mut params = serde_json::json!({ "intent": intent, "cwd": cwd });
            if explain {
                params["explain"] = serde_json::json!(true);
            }
            emit(ipc::request("iac_ask", params, None), json_mode)
        }
        Commands::Run { target, yes, cwd } => {
            let (method, params) = match (&target.proposal, target.command.is_empty()) {
                (Some(pid), _) => (
                    "iac_execute",
                    serde_json::json!({"proposal_id": pid, "confirm_dangerous": yes, "cwd": cwd, "origin": "proposal"}),
                ),
                (None, false) => (
                    "iac_execute",
                    serde_json::json!({"command": target.command.join(" "), "confirm_dangerous": yes, "cwd": cwd, "origin": "direct"}),
                ),
                _ => {
                    eprintln!("用法：iac run <命令> 或 iac run --proposal <id>");
                    return 2;
                }
            };
            emit(ipc::request(method, params, None), json_mode)
        }
        Commands::Explain { command } => {
            if command.is_empty() {
                eprintln!("用法：iac explain <命令...>");
                return 2;
            }
            emit(ipc::request("iac_explain", serde_json::json!({"command": command.join(" ")}), None), json_mode)
        }
        Commands::Doctor { execution } => {
            emit(ipc::request("iac_diagnose", serde_json::json!({"execution_id": execution}), None), json_mode)
        }
        Commands::History { action } => match action {
            HistoryAction::List { limit } => emit(ipc::request("iac_history_query", serde_json::json!({"limit": limit}), None), json_mode),
            HistoryAction::Search { q } => emit(ipc::request("iac_history_query", serde_json::json!({"q": q, "limit": 50}), None), json_mode),
            HistoryAction::Show { id: _ } => {
                eprintln!("iac history show 随 T-009 详情路由开放（当前用 search）");
                2
            }
        },
        Commands::Recipe { action } => match action {
            RecipeAction::Save { title, description, name, command, tags } => {
                if command.is_empty() {
                    eprintln!("用法：iac recipe save <标题> --with <命令...>");
                    return 2;
                }
                emit(
                    ipc::request("iac_recipe_save", serde_json::json!({"title": title, "description": description, "name": name, "command_template": command.join(" "), "tags": tags, "platform": "all"}), None),
                    json_mode,
                )
            }
            RecipeAction::List { limit } => emit(ipc::request("iac_recipes_search", serde_json::json!({"limit": limit}), None), json_mode),
            RecipeAction::Search { q } => emit(ipc::request("iac_recipes_search", serde_json::json!({"q": q, "limit": 50}), None), json_mode),
            RecipeAction::Show { id: _ } => {
                eprintln!("iac recipe show 随详情路由开放（当前用 search）");
                2
            }
            RecipeAction::Rm { id: _ } => {
                eprintln!("iac recipe rm 随删除路由开放");
                2
            }
        },
        Commands::Session { action } => match action {
            SessionAction::New => emit(ipc::request("iac_sessions_current", serde_json::json!({"cwd": ".", "shell": "sh", "host": "local"}), None), json_mode),
            SessionAction::List => {
                eprintln!("iac session list 随列表路由开放");
                2
            }
            SessionAction::Show { id: _ } | SessionAction::Resume { id: _ } => {
                eprintln!("iac session show/resume 随路由开放");
                2
            }
        },
        Commands::Config { action } => match action {
            ConfigAction::Get { key } => {
                let resp = ipc::request("iac_config_get", serde_json::json!({}), None);
                match resp {
                    Ok(r) if r.ok => {
                        let v = r.result.unwrap_or_default();
                        let picked = key.split('.').fold(&v, |acc, k| &acc[k]);
                        println!("{}", serde_json::to_string_pretty(picked).unwrap_or_default());
                        0
                    }
                    _ => render_err(&resp, json_mode),
                }
            }
            ConfigAction::Path => {
                println!("{}", infra::config::default_config_path().map(|p| p.display().to_string()).unwrap_or_else(|| "<不可定位>".into()));
                0
            }
        },
        Commands::Daemon { action } => match action {
            DaemonAction::Start => daemon_start(),
            DaemonAction::Status => {
                let resp = ipc::request("iac_ping", serde_json::json!({}), None);
                match resp {
                    Ok(r) if r.ok => {
                        println!("运行中");
                        0
                    }
                    _ => {
                        println!("未运行（iac daemon start 启动）");
                        1
                    }
                }
            }
            DaemonAction::Stop => {
                let resp = ipc::request("iac_daemon_stop", serde_json::json!({}), None);
                if matches!(&resp, Ok(r) if r.ok) {
                    println!("停机指令已发送");
                    0
                } else {
                    println!("stop 指令已送达（daemon 将自行退出）");
                    0
                }
            }
        },
        Commands::Mcp { action } => match action {
            McpAction::Serve => mcp::serve(),
        },
        Commands::Chat => tui::run_chat(json_mode),
        Commands::Translate { from, command } => {
            if command.is_empty() {
                eprintln!("用法：iac translate <命令...> [--from linux|macos|windows]");
                return 2;
            }
            let from = from.map(|d| match d {
                Dialect::Linux => "linux",
                Dialect::Macos => "macos",
                Dialect::Windows => "windows",
            });
            let mut params = serde_json::json!({"command": command.join(" ")});
            if let Some(f) = from {
                params["from_dialect"] = serde_json::json!(f);
            }
            emit(ipc::request("iac_translate", params, None), json_mode)
        }
    }
}

fn emit(resp: Result<iac_core::server::IpcResponse, String>, json_mode: bool) -> i32 {
    match resp {
        Ok(r) => {
            println!("{}", ipc::render(&r, json_mode));
            if r.ok { 0 } else { 1 }
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

fn render_err(resp: &Result<iac_core::server::IpcResponse, String>, _json_mode: bool) -> i32 {
    match resp {
        Ok(r) => println!("{}", ipc::render(r, false)),
        Err(msg) => eprintln!("{msg}"),
    }
    0
}

/// 后台启动 iacd：脱离终端、标准流落盘。
fn daemon_start() -> i32 {
    let socket = ipc::default_socket_path();
    if UnixStreamProbe::alive(&socket) {
        println!("已在运行");
        return 0;
    }
    let exe = std::env::current_exe().expect("当前可执行");
    let iacd_exe = exe.parent().map(|d| d.join("iacd")).unwrap_or_else(|| "iacd".into());
    let log = infra::config::default_config_path().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(&iacd_exe);
        cmd.process_group(0).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if let Some(d) = log {
            cmd.env("XDG_RUNTIME_DIR", d);
        }
        match cmd.spawn() {
            Ok(_) => {
                println!("daemon 已启动");
                0
            }
            Err(e) => {
                eprintln!("启动失败（请确认 iacd 在 PATH）：{e}");
                1
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("Windows 后台启动随 v1.1 落地");
        1
    }
}

struct UnixStreamProbe;

impl UnixStreamProbe {
    #[cfg(unix)]
    fn alive(socket: &std::path::Path) -> bool {
        std::os::unix::net::UnixStream::connect(socket).is_ok()
    }
    #[cfg(not(unix))]
    fn alive(_socket: &std::path::Path) -> bool {
        false
    }
}

pub mod ipc;
pub mod mcp;
pub mod theme;
pub mod tui;

