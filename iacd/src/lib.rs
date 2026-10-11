//! iacd 库主体：daemon 运行时（tokio UDS 监听 + spawn_blocking dispatch）。
//! 入口签名豁免（_00.conventions）；业务全部下沉 core 五段方法。

use iac_core::framework::{Actor, Context, Deps, Locale, SessionCtx};
use iac_core::server::{dispatch_request, IpcRequest, IpcResponse};
use infra::ai::RealAi;
use infra::process::RealProcess;
use iac_core::ports::StorePort;
use infra::storage::Storage;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// 运行参数。
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    #[allow(dead_code)]
    pub foreground: bool,
}

/// 缺省 socket 路径（转发 iac_core::server 统一定义）。
pub fn default_socket_path() -> PathBuf {
    iac_core::server::default_socket_path()
}

/// 缺省数据目录：~/.local/share/iac。
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("iac")
}

/// 运行 daemon（阻塞直至 stop 指令或 shutdown 信号）。返回已服务的连接数。
pub fn run(options: DaemonOptions, shutdown: tokio::sync::mpsc::Receiver<()>) -> anyhow::Result<u64> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(serve(options, shutdown))
}

async fn serve(options: DaemonOptions, _shutdown: tokio::sync::mpsc::Receiver<()>) -> anyhow::Result<u64> {
    // 单实例：socket 可连接 → 已有 daemon（IAC-6103）
    if UnixStream::connect(&options.socket_path).await.is_ok() {
        anyhow::bail!("daemon 已在运行（IAC-6103）：{}", options.socket_path.display());
    }
    if let Some(dir) = options.socket_path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let _ = std::fs::remove_file(&options.socket_path);
    let listener = UnixListener::bind(&options.socket_path)?;
    tokio::fs::write(options.socket_path.with_extension("pid"), std::process::id().to_string()).await?;

    tokio::fs::create_dir_all(&options.data_dir).await?;
    let storage = Storage::open(&options.data_dir.join("iac.db")).map_err(|e| anyhow::anyhow!("存储打开失败: {e}"))?;
    {
        // 启动即迁移（幂等，IAC-3103 失败则拒绝服务）
        let mut ack = iac_core::ports::MigrationAck::default();
        let mut evo = iac_core::framework::Evolution::default();
        let mut m = iac_core::framework::Metrics::default();
        let ctx = Context {
            session: &SessionCtx::default(),
            config: &iac_core::config::Config::default(),
            deps: &Deps::default(),
            locale: &Locale::default(),
        };
        storage
            .storage_migrate(&iac_core::ports::MigrationDir {}, &mut ack, &ctx, &mut m, &mut evo)
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("迁移失败（IAC-3103）: {}", evo.reason.unwrap_or_default()))?;
    }
    let config = Arc::new(load_config());
    let deps = Arc::new(Deps {
        store: Box::new(storage),
        ai: Box::new(RealAi),
        process: Box::new(RealProcess),
    });

    let mut served: u64 = 0;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut stop_rx = stop_rx;
    loop {
        tokio::select! {
            _ = stop_rx.changed() => break,
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(v) => v,
                    Err(e) => { eprintln!("accept 错误: {e}"); continue; }
                };
                served += 1;
                let deps = Arc::clone(&deps);
                let config = Arc::clone(&config);
                let stop_tx = stop_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, deps, config, stop_tx).await {
                        eprintln!("连接处理错误: {e}");
                    }
                });
            }
        }
    }
    let _ = tokio::fs::remove_file(&options.socket_path).await;
    let _ = tokio::fs::remove_file(options.socket_path.with_extension("pid")).await;
    Ok(served)
}

/// 单连接：建立会话 → JSONL 逐行 dispatch → JSONL 响应。
async fn handle_connection(
    stream: UnixStream,
    deps: Arc<Deps>,
    config: Arc<iac_core::config::Config>,
    stop_tx: tokio::sync::watch::Sender<bool>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader);
    let session_id = {
        let deps = Arc::clone(&deps);
        tokio::task::spawn_blocking(move || {
            let mut info = iac_core::session::SessionInfo::default();
            let mut evo = iac_core::framework::Evolution::default();
            let mut m = iac_core::framework::Metrics::default();
            let input = iac_core::session::CurrentSessionInput { cwd: ".".into(), shell: "sh".into(), host: "local".into() };
            let ctx = Context {
                session: &SessionCtx::default(),
                config: &iac_core::config::Config::default(),
                deps: &deps,
                locale: &Locale::default(),
            };
            if iac_core::session::get_or_create_current(&input, &mut info, &ctx, &mut m, &mut evo) {
                info.id
            } else {
                String::new()
            }
        })
        .await?
    };
    let mut line = String::new();
    loop {
        line.clear();
        if lines.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let deps = Arc::clone(&deps);
        let config = Arc::clone(&config);
        let session_id = session_id.clone();
        let req_text = trimmed.to_string();
        let (resp, is_stop) = tokio::task::spawn_blocking(move || handle_request(&req_text, &deps, &config, session_id)).await?;
        if is_stop {
            let _ = stop_tx.send(true);
        }
        writer.write_all(format!("{resp}\n").as_bytes()).await?;
        writer.flush().await?;
    }
}

/// 单请求处理：解析 → Context 组装 → dispatch → 信封。
fn handle_request(req_text: &str, deps: &Deps, config: &iac_core::config::Config, session_id: String) -> (String, bool) {
    let parse_stub = IpcRequest { v: 1, id: "unknown".into(), method: String::new(), params: Value::Null, actor: None };
    let req: IpcRequest = serde_json::from_str(req_text).unwrap_or(parse_stub);
    let actor = match req.actor.as_deref() {
        Some(a) if a.starts_with("agent:") => Actor::Agent(a.trim_start_matches("agent:").to_string()),
        _ => Actor::Human,
    };
    let session = SessionCtx { session_id, request_id: req.id.clone(), actor };
    let locale = Locale { lang: "zh-CN".into(), timezone: "Asia/Shanghai".into() };
    let ctx = Context { session: &session, config, deps, locale: &locale };
    let mut out = IpcResponse { v: 1, id: req.id.clone(), ok: false, result: None, error: None };
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let _ = dispatch_request(&req, &mut out, &ctx, &mut m, &mut evo);
    let is_stop = req.method == "iac_daemon_stop";
    (serde_json::to_string(&out).unwrap_or_default(), is_stop)
}

/// 启动时加载配置（文件缺失/无效回退默认值并继续）。
fn load_config() -> iac_core::config::Config {
    let mut cfg = iac_core::config::Config::defaults();
    if let Some(p) = infra::config::default_config_path() {
        if let Ok(raw) = std::fs::read_to_string(&p) {
            if let Ok(mut parsed) = toml::from_str::<iac_core::config::Config>(&raw) {
                parsed.apply_env();
                if parsed.validate().is_ok() {
                    cfg = parsed;
                }
            }
        }
    }
    cfg
}
