//! iacd 库主体：daemon 运行时（tokio UDS 监听 + spawn_blocking dispatch）。
//! 重建变化：iac.v2.db、daemon 内置 MCP 模式（Q43）、config_set 应用与落盘、
//! 请求级 session_id（域 18 多标签独立会话）、取消登记（Q22 迟到结果丢弃）。

use iac_core::framework::{Actor, Context, Deps, Locale, SessionCtx};
use iac_core::ports::{StorePort, SweepReport};
use iac_core::server::{dispatch_request, IpcRequest, IpcResponse};
use infra::ai::RealAi;
use infra::process::RealProcess;
use infra::storage::Storage;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub mod mcp;

/// daemon 启动时刻（ia-daemon status 的 uptime 数据源）。
static STARTED: OnceLock<Instant> = OnceLock::new();

/// 运行参数。
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    pub foreground: bool,
}

/// 缺省 socket 路径（infra::config 统一定义）。
pub fn default_socket_path() -> PathBuf {
    infra::config::socket_path()
}

/// 缺省数据目录：~/.local/share/iac。
pub fn default_data_dir() -> PathBuf {
    infra::config::data_dir()
}

/// MCP 模式（Q43）：进程内 dispatch 的 stdio JSON-RPC 服务；不走 socket。
pub fn run_mcp_mode(data_dir: &std::path::Path) -> i32 {
    let _ = std::fs::create_dir_all(data_dir);
    let storage = match Storage::open(&data_dir.join("iac.v2.db")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("存储打开失败: {e}");
            return 1;
        }
    };
    let mut ack = iac_core::ports::MigrationAck::default();
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    {
        let ctx = Context {
            session: &SessionCtx::default(),
            config: &iac_core::config::Config::default(),
            deps: &Deps::default(),
            locale: &Locale::default(),
        };
        if !storage.storage_migrate(
            &iac_core::ports::MigrationDir {},
            &mut ack,
            &ctx,
            &mut m,
            &mut evo,
        ) {
            eprintln!("迁移失败（IAC-3103）: {}", evo.reason.unwrap_or_default());
            return 1;
        }
    }
    let config = Arc::new(load_config());
    let deps = Arc::new(Deps {
        store: Box::new(storage),
        ai: Box::new(RealAi),
        process: Box::new(RealProcess),
        snap: Box::new(infra::snapstore::RealSnap::new()),
    });
    mcp::stdio_loop(deps, config)
}

/// 已取消请求登记（Q22：取消后到达的结果改写 IAC-6107，迟到不落客户端）。
pub type CancelledSet = Arc<Mutex<HashMap<String, ()>>>;

/// 运行 daemon（阻塞直至 stop 指令）。返回已服务的连接数。
pub fn run(options: DaemonOptions) -> anyhow::Result<u64> {
    let _ = STARTED.set(Instant::now());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (_tx, rx) = tokio::sync::mpsc::channel::<()>(1);
    runtime.block_on(serve(options, rx))
}

async fn serve(
    options: DaemonOptions,
    _shutdown: tokio::sync::mpsc::Receiver<()>,
) -> anyhow::Result<u64> {
    // 单实例：socket 可连接 → 已有 daemon（IAC-6103）
    if UnixStream::connect(&options.socket_path).await.is_ok() {
        anyhow::bail!(
            "daemon 已在运行（IAC-6103）：{}",
            options.socket_path.display()
        );
    }
    if let Some(dir) = options.socket_path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let _ = std::fs::remove_file(&options.socket_path);
    let listener = UnixListener::bind(&options.socket_path)?;
    tokio::fs::write(
        options.socket_path.with_extension("pid"),
        std::process::id().to_string(),
    )
    .await?;

    tokio::fs::create_dir_all(&options.data_dir).await?;
    // 重建基线数据文件：iac.v2.db（与旧 iac.db schema 物理隔离）
    let storage = Storage::open(&options.data_dir.join("iac.v2.db"))
        .map_err(|e| anyhow::anyhow!("存储打开失败: {e}"))?;
    {
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
            .storage_migrate(
                &iac_core::ports::MigrationDir {},
                &mut ack,
                &ctx,
                &mut m,
                &mut evo,
            )
            .then_some(())
            .ok_or_else(|| {
                anyhow::anyhow!("迁移失败（IAC-3103）: {}", evo.reason.unwrap_or_default())
            })?;
    }
    let config = Arc::new(std::sync::RwLock::new(load_config()));
    let cancelled: CancelledSet = Arc::new(Mutex::new(HashMap::new()));
    let deps = Arc::new(Deps {
        store: Box::new(storage),
        ai: Box::new(RealAi),
        process: Box::new(RealProcess),
        snap: Box::new(infra::snapstore::RealSnap::new()),
    });
    // 快照/回收站 retention（AC-31/34）：此前 sweep_expired 仅测试可达，生产休眠导致只增不减。
    // 启动即清扫一次 + 每小时周期清扫（策略取 rollback 配置）
    {
        let cfg = config.read().expect("config lock poisoned");
        spawn_retention_sweep(
            Arc::clone(&deps),
            cfg.rollback.retention_days,
            cfg.rollback.retention_max_bytes,
        );
    }

    let mut served: u64 = 0;
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
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
                let cancelled = Arc::clone(&cancelled);
                let stop_tx = stop_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, deps, config, cancelled, stop_tx).await {
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

/// 快照/回收站 retention 周期清扫（AC-31/34）：启动即清一次，此后每小时一次。
fn spawn_retention_sweep(deps: Arc<Deps>, days: u32, max_bytes: u64) {
    tokio::spawn(async move {
        let policy = iac_core::ports::RetentionPolicy { days, max_bytes };
        loop {
            let deps = Arc::clone(&deps);
            let report = tokio::task::spawn_blocking(move || {
                let session = SessionCtx::default();
                let config = iac_core::config::Config::default();
                let locale = Locale::default();
                let ctx = Context {
                    session: &session,
                    config: &config,
                    deps: &deps,
                    locale: &locale,
                };
                let mut out = SweepReport::default();
                let mut m = iac_core::framework::Metrics::default();
                let mut evo = iac_core::framework::Evolution::default();
                if deps
                    .snap
                    .sweep_expired(&policy, &mut out, &ctx, &mut m, &mut evo)
                {
                    out
                } else {
                    eprintln!(
                        "[iacd] retention 清扫失败: {}",
                        evo.reason.unwrap_or_default()
                    );
                    SweepReport::default()
                }
            })
            .await
            .unwrap_or_default();
            if report.removed > 0 || report.freed_bytes > 0 {
                eprintln!(
                    "[iacd] retention 清扫：删除 {} 项，释放 {} 字节",
                    report.removed, report.freed_bytes
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

/// 单连接：建立会话 → JSONL 逐行 dispatch → JSONL 响应。
async fn handle_connection(
    stream: UnixStream,
    deps: Arc<Deps>,
    config: Arc<std::sync::RwLock<iac_core::config::Config>>,
    cancelled: CancelledSet,
    stop_tx: tokio::sync::watch::Sender<bool>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader);
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
        let config_task = Arc::clone(&config);
        let cancelled = Arc::clone(&cancelled);
        let req_text = trimmed.to_string();
        let (mut resp, is_stop, apply_cfg) = tokio::task::spawn_blocking(move || {
            handle_request(&req_text, &deps, &config_task, &cancelled)
        })
        .await?;
        if let Some(new_cfg) = apply_cfg {
            // config_set 应用 + 落盘（组合根职责）
            if let Ok(mut w) = config.write() {
                *w = new_cfg.clone();
            }
            let mut ack = infra::config::ConfigWriteAck::default();
            let mut evo = iac_core::framework::Evolution::default();
            if infra::config::save_config(&new_cfg, &mut ack, &mut evo) {
                // core 层回包 applied 恒 false（组合根落盘结果 core 无从知晓）；落盘成功回填真值
                resp = mark_config_applied(&resp);
            } else {
                eprintln!("[iacd] config_set 落盘失败: {}", evo.reason.unwrap_or_default());
            }
        }
        if is_stop {
            let _ = stop_tx.send(true);
        }
        writer.write_all(format!("{resp}\n").as_bytes()).await?;
        writer.flush().await?;
    }
}

/// 单请求处理：解析 → Context 组装 → dispatch → 信封。
/// 返回 (响应文本, 是否停机, 待应用配置)。
fn handle_request(
    req_text: &str,
    deps: &Deps,
    config: &std::sync::RwLock<iac_core::config::Config>,
    cancelled: &CancelledSet,
) -> (String, bool, Option<iac_core::config::Config>) {
    let parse_stub = IpcRequest {
        v: 1,
        id: "unknown".into(),
        method: String::new(),
        params: Value::Null,
        actor: None,
        session_id: None,
    };
    let req: IpcRequest = serde_json::from_str(req_text).unwrap_or(parse_stub);
    // 取消登记受理（Q22）：iac_cancel 由接入层在路由前处理——登记目标 request_id，
    // 使 in-flight 请求在完成后被改写 IAC-6107 丢弃（此前只登记不受理，链路整体断链）。
    if req.method == "iac_cancel" {
        let rid = req
            .params
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let out = if rid.is_empty() {
            IpcResponse::failure(&req, "IAC-6106", "iac_cancel 缺 request_id")
        } else {
            let _ = cancelled.lock().map(|mut m| {
                if m.len() > 4096 {
                    m.clear(); // 孤儿登记（目标已完成的取消）兜底，防泄漏
                }
                m.insert(rid.clone(), ());
            });
            IpcResponse::success(&req, json!({ "cancelled": rid }))
        };
        return (serde_json::to_string(&out).unwrap_or_default(), false, None);
    }
    // ia-daemon status：组合根持有进程真数据（pid/uptime/version），接入层直答。
    // MCP 路径不经此处，由 core route 的 iac_ping 兜底（同为真可达性证据）
    if req.method == "iac_builtin" && is_daemon_status(&req.params) {
        let uptime_ms = STARTED
            .get()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let out = IpcResponse::success(
            &req,
            json!({
                "builtin": "ia-daemon",
                "result": {
                    "status": "running",
                    "pid": std::process::id(),
                    "uptime_ms": uptime_ms,
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        );
        return (serde_json::to_string(&out).unwrap_or_default(), false, None);
    }
    // 取消登记（Q22）：请求前已被登记 → 迟到结果改写 IAC-6107
    let was_cancelled = req.id != "unknown"
        && cancelled
            .lock()
            .map(|mut m| m.remove(&req.id).is_some())
            .unwrap_or(false);
    let actor = match req.actor.as_deref() {
        Some(a) if a.starts_with("agent:") => {
            Actor::Agent(a.trim_start_matches("agent:").to_string())
        }
        _ => Actor::Human,
    };
    let session = SessionCtx {
        session_id: req.session_id.clone().unwrap_or_default(),
        request_id: req.id.clone(),
        actor,
    };
    let locale = Locale {
        lang: "zh-CN".into(),
        timezone: "Asia/Shanghai".into(),
    };
    let cfg_guard = config.read().expect("config lock poisoned");
    let ctx = Context {
        session: &session,
        config: &cfg_guard,
        deps,
        locale: &locale,
    };
    let mut out = IpcResponse {
        v: 1,
        id: req.id.clone(),
        ok: false,
        result: None,
        error: None,
    };
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let _ = dispatch_request(&req, &mut out, &ctx, &mut m, &mut evo);
    drop(cfg_guard);
    log_metrics(&m);
    // dispatch 期间被登记的取消 → 本次结果改写丢弃（Q22：取消=丢弃迟到结果）
    let cancelled_during = req.id != "unknown"
        && cancelled
            .lock()
            .map(|mut m| m.remove(&req.id).is_some())
            .unwrap_or(false);
    if (was_cancelled || cancelled_during) && out.ok {
        out.ok = false;
        out.result = None;
        out.error = Some(iac_core::error::IacError::new(
            iac_core::error::codes::CANCELLED,
            "请求已被客户端取消（结果已丢弃；执行类副作用以留痕为准）",
        ));
    }
    // config_set：解析计算后的新配置交给连接层应用
    let apply_cfg = if req.method == "iac_config_set" && out.ok {
        out.result
            .as_ref()
            .and_then(|r| r.get("config"))
            .cloned()
            .and_then(|c| serde_json::from_value::<iac_core::config::Config>(c).ok())
    } else {
        None
    };
    // 停机判定：iac_daemon_stop 直发，或经 ia-daemon stop 映射后产生 DAEMON_STOP 信封
    // （修复前仅匹配直发方法名，GUI 文本命令 ia-daemon stop 永不真正停机）
    let is_stop = req.method == "iac_daemon_stop"
        || out
            .error
            .as_ref()
            .map(|e| e.code == iac_core::error::codes::DAEMON_STOP)
            .unwrap_or(false);
    (
        serde_json::to_string(&out).unwrap_or_default(),
        is_stop,
        apply_cfg,
    )
}

/// config_set 回包 applied 回填：落盘成功 → true（core 层无从知晓组合根落盘结果）。
fn mark_config_applied(resp: &str) -> String {
    match serde_json::from_str::<Value>(resp) {
        Ok(mut v) => {
            if let Some(r) = v.get_mut("result").and_then(Value::as_object_mut) {
                r.insert("applied".into(), Value::Bool(true));
            }
            serde_json::to_string(&v).unwrap_or_else(|_| resp.to_string())
        }
        Err(_) => resp.to_string(),
    }
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

/// ia-daemon status 请求判定（name=ia-daemon 且未带 stop 子命令）。
fn is_daemon_status(params: &Value) -> bool {
    if params.get("name").and_then(Value::as_str) != Some("ia-daemon") {
        return false;
    }
    !params
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().any(|v| v.as_str() == Some("stop")))
        .unwrap_or(false)
}

/// 观测落地（framework.rs 约定的「运行时后端」）：计数聚合一行、失败标记与
/// Warn/Error 日志逐条。此前 Metrics 事件只进内存 Vec 随请求丢弃，生产零观测。
fn log_metrics(m: &iac_core::framework::Metrics) {
    use iac_core::framework::{Level, MetricEvent};
    let mut counters: HashMap<String, u64> = HashMap::new();
    for ev in m.events() {
        match ev {
            MetricEvent::Counter { name, n } => {
                *counters.entry(name.clone()).or_default() += n;
            }
            MetricEvent::Mark { key, value } if key.ends_with("_failed") => {
                eprintln!("[iacd] mark {key}={value}");
            }
            MetricEvent::Log { level, msg } => match level {
                Level::Warn | Level::Error => eprintln!("[iacd] {level:?}: {msg}"),
                _ => {}
            },
            MetricEvent::Mark { .. } => {}
        }
    }
    if !counters.is_empty() {
        let summary = counters
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("[iacd] metrics {summary}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps() -> Deps {
        Deps::default()
    }

    fn req_text(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn cancel_registration_discards_late_result() {
        let deps = deps();
        let config = std::sync::RwLock::new(iac_core::config::Config::defaults());
        let cancelled: CancelledSet = Arc::new(Mutex::new(HashMap::new()));
        // 1) iac_cancel 受理登记（修复前：路由表无此方法，返回未知方法错误）
        let (resp, _, _) = handle_request(
            &req_text(r#"{"v":1,"id":"c1","method":"iac_cancel","params":{"request_id":"req-9"}}"#),
            &deps,
            &config,
            &cancelled,
        );
        assert!(resp.contains("\"cancelled\":\"req-9\""), "resp={resp}");
        // 2) 已登记请求的结果在完成后被改写 IAC-6107 丢弃（Q22 语义端到端成立）
        let (resp2, _, _) = handle_request(
            &req_text(r#"{"v":1,"id":"req-9","method":"iac_ping"}"#),
            &deps,
            &config,
            &cancelled,
        );
        assert!(resp2.contains("IAC-6107"), "resp={resp2}");
        // 3) 未登记请求不受影响
        let (resp3, _, _) = handle_request(
            &req_text(r#"{"v":1,"id":"req-8","method":"iac_ping"}"#),
            &deps,
            &config,
            &cancelled,
        );
        assert!(resp3.contains("\"pong\":true"), "resp={resp3}");
    }

    #[test]
    fn daemon_status_reports_real_process_data() {
        let deps = deps();
        let config = std::sync::RwLock::new(iac_core::config::Config::defaults());
        let cancelled: CancelledSet = Arc::new(Mutex::new(HashMap::new()));
        let _ = STARTED.set(Instant::now());
        let (resp, _, _) = handle_request(
            &req_text(
                r#"{"v":1,"id":"c2","method":"iac_builtin","params":{"name":"ia-daemon","args":[]}}"#,
            ),
            &deps,
            &config,
            &cancelled,
        );
        assert!(
            resp.contains("\"pid\""),
            "status 必须携带真实 pid（修复前为罐头 ping-able）：{resp}"
        );
        assert!(resp.contains("\"uptime_ms\""), "resp={resp}");
        // stop 子命令不在此拦截（照常走路由）
        let (resp2, is_stop, _) = handle_request(
            &req_text(
                r#"{"v":1,"id":"c3","method":"iac_builtin","params":{"name":"ia-daemon","args":["stop"]}}"#,
            ),
            &deps,
            &config,
            &cancelled,
        );
        assert!(is_stop && !resp2.contains("\"pid\""));
    }

    #[test]
    fn config_applied_flips_after_persist() {
        let resp = r#"{"v":1,"id":"x","ok":true,"result":{"config":{},"applied":false}}"#;
        let marked = mark_config_applied(resp);
        assert!(marked.contains("\"applied\":true"), "marked={marked}");
        // 非 JSON 输入原样返回（不 panic）
        assert_eq!(mark_config_applied("not-json"), "not-json");
    }
}
