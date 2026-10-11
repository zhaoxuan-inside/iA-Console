//! 本地引擎 supervisor（IF-010）：插件引擎子进程拉起 + 健康检查 + 端口管理（ADR-007）。

use iac_core::framework::{Context, Evolution, Metrics};
use serde_json::json;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 引擎子进程注册表：持有 Child 使其随 daemon 存活。
static ENGINE_CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());

/// ensure 输入（_06 EngineSpec 字段只增演进：补齐目录/二进制/模型定位）。
#[derive(Debug, Clone, Default)]
pub struct EngineSpec {
    pub plugin_id: String,
    pub plugin_dir: Option<PathBuf>,
    pub engine_binary: Option<String>,
    pub model_file: Option<String>,
    pub context_len: u32,
    pub preferred_port: Option<u16>,
}

/// ensure 输出（_06 EngineHandle）。
#[derive(Debug, Clone, PartialEq)]
pub struct EngineHandle {
    pub base_url: String,
    pub pid: u32,
    pub model: String,
}

/// IF-010：确保本地引擎运行（已健康→复用；未运行→拉起）。失败语义：未安装 IAC-1401、启动失败 IAC-1403。
pub fn local_engine_ensure(
    input: &EngineSpec,
    output: &mut EngineHandle,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(dir) = &input.plugin_dir else {
        evolution.fail("IAC-1401", "插件未安装：请先 iac model install（可 iac model list 查看）");
        return false;
    };
    let Some(engine) = &input.engine_binary else {
        evolution.fail("IAC-1401", "引擎二进制缺失：重新安装插件（IAC-1401）");
        return false;
    };
    let engine_path = dir.join(engine);
    if !engine_path.exists() {
        evolution.fail("IAC-1401", format!("引擎不存在：{}", engine_path.display()));
        return false;
    }
    let model = dir.join(input.model_file.as_deref().unwrap_or("model.gguf"));
    if !model.exists() {
        evolution.fail("IAC-1401", format!("模型文件不存在：{}", model.display()));
        return false;
    }
    let start_port = input.preferred_port.unwrap_or(18080);
    if let Some((_port, url)) = try_ports(&engine_path, &model, input.context_len, start_port, metrics) {
        output.base_url = url;
        output.pid = 0; // 引擎由注册表持有；pid 随 T-027 收尾补齐
        output.model = model.display().to_string();
        return true;
    }
    evolution.fail("IAC-1403", "本地引擎启动失败（端口尝试 10 次均未就绪）");
    false
}

/// 端口扫描启动（local_engine_ensure 子步骤）：返回 (端口, base_url)。
fn try_ports(
    engine_path: &PathBuf,
    model: &PathBuf,
    context_len: u32,
    start_port: u16,
    metrics: &mut Metrics,
) -> Option<(u16, String)> {
    for port in start_port..start_port + 10 {
        if !port_free(port) {
            continue;
        }
        let started = spawn_engine(engine_path, model, context_len, port)
            && wait_healthy(port, Duration::from_secs(3));
        if started {
            metrics.counter("local_engine.started", 1);
            return Some((port, format!("http://127.0.0.1:{port}")));
        }
    }
    None
}

/// 端口空闲检测。
fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 拉起引擎子进程（stdout/stderr 落日志文件；进程入注册表）。
fn spawn_engine(engine_path: &PathBuf, model: &PathBuf, context_len: u32, port: u16) -> bool {
    let log = engine_path.parent().map(|d| d.join("engine.log"));
    let mut cmd = Command::new(engine_path);
    cmd.arg("--model").arg(model)
        .arg("--port").arg(port.to_string())
        .arg("--ctx-size").arg(context_len.to_string())
        .stdin(std::process::Stdio::null());
    if let Some(log) = log {
        if let Ok(f) = std::fs::File::create(&log) {
            cmd.stdout(f.try_clone().expect("log clone")).stderr(f);
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            ENGINE_CHILDREN.lock().expect("engine registry").push(child);
            true
        }
        Err(_) => false,
    }
}

/// 健康检查：GET /health 200 视为就绪。
fn wait_healthy(port: u16, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .expect("http client");
    while Instant::now() < deadline {
        if let Ok(resp) = client.get(format!("http://127.0.0.1:{port}/health")).send() {
            if resp.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

/// 健康检查复用导出（daemon 降级判定用）。
pub fn engine_healthy(base_url: &str) -> bool {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_millis(400)).build().expect("http client");
    client.get(format!("{base_url}/health")).send().map(|r| r.status().is_success()).unwrap_or(false)
}

/// JSON 视图（daemon 降级日志用）。
pub fn engine_status_json(healthy: bool, base_url: &str) -> serde_json::Value {
    json!({"local_engine": {"healthy": healthy, "base_url": base_url}})
}
