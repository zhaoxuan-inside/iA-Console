//! iac CLI 入口（框架签名豁免）：clap 子命令 → IPC → 渲染。
//! 重建变化：MCP 收归 daemon（Q43，`iacd --mcp`）、历史清理/导出（Q47）、
//! 会话列表/恢复（Q46）、配置 set（审计走 daemon）、翻译 from/to 契约、确认词（Q37）。

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
    /// 意图 → 多候选提案（说人话拿命令；也可在 console 主输入行直输）
    Ask {
        intent: String,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// 执行命令（过风险闸门；人类直输弱提醒不拦截）
    Run {
        #[command(flatten)]
        target: RunTarget,
        /// dangerous 武装态确认标志（二次确认语义，Q35）
        #[arg(long)]
        yes: bool,
        /// destructive 确认词（Q37）
        #[arg(long)]
        confirm_word: Option<String>,
        /// 多候选下标（缺省活动位，Q17）
        #[arg(long)]
        candidate_index: Option<usize>,
    },
    /// 参数白话解释
    Explain { command: Vec<String> },
    /// 诊断最近/指定失败执行（Q18 一键「AI 帮忙」）
    Doctor {
        #[arg(long)]
        execution: Option<String>,
    },
    /// 历史检索 / 保留清理 / 导出（Q47）
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },
    /// 方案库
    Recipe {
        #[command(subcommand)]
        action: RecipeAction,
    },
    /// 会话列表与恢复（Q46）
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// 配置（裸命令进入交互式向导，参照 opencode CLI）
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// daemon 生命周期
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// 全屏 Console 客户端（主界面）
    Console {
        #[arg(long, value_enum)]
        theme: Option<crate::theme::ModeArg>,
    },
    /// 行模式对话（无备用屏）
    Chat,
    /// 执行回滚（域 15：效果日志；Q49 双触达之 CLI 侧）
    Rollback {
        /// 列出回滚点（history 式）
        #[arg(long)]
        list: bool,
        /// 回滚到指定行（其后全部逆序回放）
        #[arg(long)]
        to: Option<i64>,
        /// 从指定行回放（缺省=最新）
        #[arg(long)]
        from: Option<i64>,
        /// 冲突策略（ask/skip/abort；非交互缺省 abort）
        #[arg(long)]
        on_conflict: Option<String>,
    },
    /// 单文件版本与恢复（域 15）
    File {
        #[command(subcommand)]
        action: FileAction,
    },
    /// 脚本工坊（域 16）
    Script {
        #[command(subcommand)]
        action: ScriptAction,
    },
    /// 习惯方言翻译（域 11）
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

#[derive(clap::Subcommand)]
enum FileAction {
    /// 查看文件全部版本（对应各次执行）
    History { path: String },
    /// 恢复到指定版本（直接替换；现文件入回收站）
    Restore {
        path: String,
        /// 版本行号（iac file history 查看）
        #[arg(long)]
        seq: i64,
    },
}

#[derive(clap::Subcommand)]
enum ScriptAction {
    /// 从行号区间/任务段制作脚本并命名注册
    Make {
        name: String,
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
        /// 任务段收集（"last"=最近一个 --- 段）
        #[arg(long)]
        segment: Option<String>,
        /// 目标形态：sh/bat/both（缺省 both）
        #[arg(long, default_value = "both")]
        target: String,
        /// 显式触发 AI 精修
        #[arg(long)]
        refine: bool,
    },
    /// AI 精修：先出修订稿，确认后写回
    Refine {
        name: String,
        #[arg(long)]
        confirm: bool,
        #[arg(long)]
        refined_body: Option<String>,
    },
}

#[derive(Subcommand)]
enum HistoryAction {
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Search {
        q: String,
    },
    /// 双限清退预览/执行（Q47）
    Clean {
        #[arg(long)]
        dry_run: bool,
    },
    /// 导出归档 JSONL（Q47）
    Export {
        path: String,
    },
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
    Search {
        q: String,
    },
    Rm {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// 导出归档 JSONL
    Export {
        path: String,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// 取/建当前会话
    New,
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// 恢复指定会话（Q46）
    Resume { id: String },
}

#[derive(Subcommand)]
enum ConfigAction {
    Get {
        key: String,
    },
    /// 修改配置（daemon 应用并落盘，记审计）
    Set {
        key: String,
        value: String,
    },
    Path,
}

#[derive(Subcommand)]
enum DaemonAction {
    Start,
    Status,
    Stop,
    /// daemon 内置 MCP 服务（Q43；stdio JSON-RPC，供外部 Agent 接入）
    Mcp,
}

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    let code = run(cli, json_mode);
    std::process::exit(code);
}

fn run(cli: Cli, json_mode: bool) -> i32 {
    match cli.command {
        Commands::Ask { intent, cwd } => emit(
            crate::ipc::request(
                "iac_ask",
                serde_json::json!({ "intent": intent, "cwd": cwd }),
                None,
            ),
            json_mode,
        ),
        Commands::Run {
            target,
            yes,
            confirm_word,
            candidate_index,
        } => {
            // @脚本名：脚本工坊命名脚本（AC-39）
            if target.proposal.is_none()
                && target.command.len() == 1
                && target.command[0].starts_with('@')
            {
                let name = target.command[0][1..].to_string();
                return emit(
                    crate::ipc::request(
                        "iac_script",
                        serde_json::json!({"action": "run", "name": name, "args": [], "confirm_dangerous": yes, "confirm_word": confirm_word}),
                        None,
                    ),
                    json_mode,
                );
            }
            let (method, params) = match (&target.proposal, target.command.is_empty()) {
                (Some(pid), _) => (
                    "iac_execute",
                    serde_json::json!({
                        "proposal_id": pid, "confirmed": yes, "confirm_word": confirm_word,
                        "candidate_index": candidate_index, "origin": "proposal"
                    }),
                ),
                (None, false) => (
                    "iac_execute",
                    serde_json::json!({
                        "command": target.command.join(" "), "confirmed": yes,
                        "confirm_word": confirm_word, "origin": "direct"
                    }),
                ),
                _ => {
                    eprintln!("用法：iac run <命令> 或 iac run --proposal <id>");
                    return 2;
                }
            };
            emit(crate::ipc::request(method, params, None), json_mode)
        }
        Commands::Explain { command } => {
            if command.is_empty() {
                eprintln!("用法：iac explain <命令...>");
                return 2;
            }
            emit(
                crate::ipc::request(
                    "iac_explain",
                    serde_json::json!({"command": command.join(" ")}),
                    None,
                ),
                json_mode,
            )
        }
        Commands::Doctor { execution } => emit(
            crate::ipc::request(
                "iac_diagnose",
                serde_json::json!({"execution_id": execution}),
                None,
            ),
            json_mode,
        ),
        Commands::History { action } => match action {
            HistoryAction::List { limit } => emit(
                crate::ipc::request(
                    "iac_history_query",
                    serde_json::json!({"limit": limit}),
                    None,
                ),
                json_mode,
            ),
            HistoryAction::Search { q } => emit(
                crate::ipc::request(
                    "iac_history_query",
                    serde_json::json!({"q": q, "limit": 50}),
                    None,
                ),
                json_mode,
            ),
            HistoryAction::Clean { dry_run } => emit(
                crate::ipc::request(
                    "iac_history_clean",
                    serde_json::json!({"dry_run": dry_run}),
                    None,
                ),
                json_mode,
            ),
            HistoryAction::Export { path } => emit(
                crate::ipc::request(
                    "iac_history_export",
                    serde_json::json!({"path": path}),
                    None,
                ),
                json_mode,
            ),
        },
        Commands::Recipe { action } => match action {
            RecipeAction::Save {
                title,
                description,
                name,
                command,
                tags,
            } => {
                if command.is_empty() {
                    eprintln!("用法：iac recipe save <标题> --with <命令...>");
                    return 2;
                }
                emit(
                    crate::ipc::request(
                        "iac_recipe_save",
                        serde_json::json!({"title": title, "description": description, "name": name, "command_template": command.join(" "), "tags": tags, "platform": "all"}),
                        None,
                    ),
                    json_mode,
                )
            }
            RecipeAction::List { limit } => emit(
                crate::ipc::request(
                    "iac_recipes_search",
                    serde_json::json!({"limit": limit}),
                    None,
                ),
                json_mode,
            ),
            RecipeAction::Search { q } => emit(
                crate::ipc::request(
                    "iac_recipes_search",
                    serde_json::json!({"q": q, "limit": 50}),
                    None,
                ),
                json_mode,
            ),
            RecipeAction::Rm { id, name } => emit(
                crate::ipc::request(
                    "iac_recipe_delete",
                    serde_json::json!({"id": id, "name": name}),
                    None,
                ),
                json_mode,
            ),
            RecipeAction::Export { path } => emit(
                crate::ipc::request("iac_recipe_export", serde_json::json!({"path": path}), None),
                json_mode,
            ),
        },
        Commands::Session { action } => match action {
            SessionAction::New => emit(
                crate::ipc::request(
                    "iac_sessions_current",
                    serde_json::json!({"cwd": ".", "shell": "sh", "host": "local"}),
                    None,
                ),
                json_mode,
            ),
            SessionAction::List { limit } => emit(
                crate::ipc::request(
                    "iac_session_list",
                    serde_json::json!({"limit": limit}),
                    None,
                ),
                json_mode,
            ),
            SessionAction::Resume { id } => emit(
                crate::ipc::request(
                    "iac_session_resume",
                    serde_json::json!({"session_id": id}),
                    None,
                ),
                json_mode,
            ),
        },
        Commands::Config { action } => match action {
            None => crate::config_wizard::run(),
            Some(ConfigAction::Get { key }) => {
                let resp = crate::ipc::request("iac_config_get", serde_json::json!({}), None);
                match resp {
                    Ok(r) if r.ok => {
                        let v = r.result.unwrap_or_default();
                        let picked = key.split('.').fold(&v, |acc, k| &acc[k]);
                        println!(
                            "{}",
                            serde_json::to_string_pretty(picked).unwrap_or_default()
                        );
                        0
                    }
                    _ => render_err(&resp, json_mode),
                }
            }
            Some(ConfigAction::Set { key, value }) => emit(
                crate::ipc::request(
                    "iac_config_set",
                    serde_json::json!({"key": key, "value": value}),
                    None,
                ),
                json_mode,
            ),
            Some(ConfigAction::Path) => {
                println!(
                    "{}",
                    infra::config::default_config_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<不可定位>".into())
                );
                0
            }
        },
        Commands::Daemon { action } => match action {
            DaemonAction::Start => daemon_start(),
            DaemonAction::Status => {
                let resp = crate::ipc::request("iac_ping", serde_json::json!({}), None);
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
                let resp = crate::ipc::request("iac_daemon_stop", serde_json::json!({}), None);
                if matches!(&resp, Ok(r) if r.ok) {
                    println!("停机指令已发送");
                    0
                } else {
                    println!("stop 指令已送达（daemon 将自行退出）");
                    0
                }
            }
            DaemonAction::Mcp => {
                eprintln!("MCP 服务由 daemon 内置（Q43）：运行 `iacd --mcp`，在 Agent 配置中将该命令注册为 stdio MCP server");
                2
            }
        },
        Commands::Console { theme } => {
            let mode = theme.map(|t| match t {
                crate::theme::ModeArg::Day => crate::theme::Mode::Day,
                crate::theme::ModeArg::Night => crate::theme::Mode::Night,
            });
            crate::console::run(mode)
        }
        Commands::Chat => crate::tui::run_chat(json_mode),
        Commands::Rollback {
            list,
            to,
            from,
            on_conflict,
        } => {
            let params = if list || (to.is_none() && from.is_none()) {
                serde_json::json!({"action": "list", "filter": {"limit": 50}})
            } else {
                serde_json::json!({
                    "action": "replay",
                    "range": {"from": from, "to": to},
                    "on_conflict": on_conflict.unwrap_or_else(|| "abort".into())
                })
            };
            emit(crate::ipc::request("iac_rollback", params, None), json_mode)
        }
        Commands::File { action } => match action {
            FileAction::History { path } => emit(
                crate::ipc::request(
                    "iac_rollback",
                    serde_json::json!({"action": "file_history", "path": path}),
                    None,
                ),
                json_mode,
            ),
            FileAction::Restore { path, seq } => emit(
                crate::ipc::request(
                    "iac_rollback",
                    serde_json::json!({"action": "restore", "path": path, "seq": seq}),
                    None,
                ),
                json_mode,
            ),
        },
        Commands::Script { action } => match action {
            ScriptAction::Make {
                name,
                from,
                to,
                segment,
                target,
                refine,
            } => emit(
                crate::ipc::request(
                    "iac_script",
                    serde_json::json!({"action": "make", "name": name, "from_seq": from, "to_seq": to, "segment": segment, "target": target, "refine": refine}),
                    None,
                ),
                json_mode,
            ),
            ScriptAction::Refine {
                name,
                confirm,
                refined_body,
            } => emit(
                crate::ipc::request(
                    "iac_script",
                    serde_json::json!({"action": "refine", "name": name, "confirm": confirm, "refined_body": refined_body}),
                    None,
                ),
                json_mode,
            ),
        },
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
                params["from"] = serde_json::json!(f);
            }
            emit(
                crate::ipc::request("iac_translate", params, None),
                json_mode,
            )
        }
    }
}

fn emit(resp: Result<iac_core::server::IpcResponse, String>, json_mode: bool) -> i32 {
    match resp {
        Ok(r) => {
            println!("{}", crate::ipc::render(&r, json_mode));
            if r.ok {
                0
            } else {
                1
            }
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

fn render_err(resp: &Result<iac_core::server::IpcResponse, String>, _json_mode: bool) -> i32 {
    match resp {
        Ok(r) => println!("{}", crate::ipc::render(r, false)),
        Err(msg) => eprintln!("{msg}"),
    }
    0
}

/// 后台启动 iacd：脱离终端、标准流落盘。
fn daemon_start() -> i32 {
    let socket = crate::ipc::default_socket_path();
    if UnixStreamProbe::alive(&socket) {
        println!("已在运行");
        return 0;
    }
    let exe = std::env::current_exe().expect("当前可执行");
    let iacd_exe = exe
        .parent()
        .map(|d| d.join("iacd"))
        .unwrap_or_else(|| "iacd".into());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(&iacd_exe);
        cmd.process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
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
        let _ = (iacd_exe, socket);
        eprintln!("Windows 后台启动随平台适配落地");
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

pub mod config_wizard;
pub mod console;
pub mod ipc;
pub mod theme;
pub mod tui;
