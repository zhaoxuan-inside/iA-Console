//! 本地引擎 supervisor（IF-010）：插件引擎子进程拉起 + 健康检查 + 端口管理（ADR-007）。
//! 注册表按模型路径复用：同模型已健康 → 直接复用；健康检查失败/子进程已死 → 清理后重拉。
//! 任何失败路径（未就绪、spawn 失败）都会终止本次拉起的进程，不留泄漏。

use iac_core::framework::{Context, Evolution, Metrics};
use serde_json::json;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// 默认引擎启动预算（模型加载可达数十秒，3s 必然误杀）。
const DEFAULT_STARTUP_SECS: u32 = 30;

/// 引擎注册表条目：key=模型规范路径（同一模型全局唯一实例）。
struct EngineEntry {
    key: String,
    base_url: String,
    child: Child,
}

/// 引擎子进程注册表：持有 Child 使其随 daemon 存活；Poison 时取内层值恢复。
static ENGINES: Mutex<Vec<EngineEntry>> = Mutex::new(Vec::new());

fn registry() -> MutexGuard<'static, Vec<EngineEntry>> {
    ENGINES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 注册表快照（daemon 状态查询/测试用）：(pid, base_url, 模型路径)。
pub fn registered_engines() -> Vec<(u32, String, String)> {
    let mut reg = registry();
    reap_dead(&mut reg);
    reg.iter()
        .map(|e| (e.child.id(), e.base_url.clone(), e.key.clone()))
        .collect()
}

/// ensure 输入（_06 EngineSpec 字段只增演进：补齐目录/二进制/模型定位/启动预算）。
#[derive(Debug, Clone, Default)]
pub struct EngineSpec {
    pub plugin_id: String,
    pub plugin_dir: Option<PathBuf>,
    pub engine_binary: Option<String>,
    pub model_file: Option<String>,
    pub context_len: u32,
    pub preferred_port: Option<u16>,
    /// 引擎启动健康预算（秒）；0 = 用默认 30s。
    pub startup_timeout_secs: u32,
}

/// ensure 输出（_06 EngineHandle）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineHandle {
    pub base_url: String,
    pub pid: u32,
    pub model: String,
}

/// IF-010：确保本地引擎运行（同模型已健康→复用；未运行→拉起）。
/// 失败语义：未安装 IAC-1401、启动失败 IAC-1403；失败路径无进程泄漏。
pub fn local_engine_ensure(
    input: &EngineSpec,
    output: &mut EngineHandle,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(dir) = &input.plugin_dir else {
        evolution.fail(
            "IAC-1401",
            "插件未安装：请先 iac model install（可 iac model list 查看）",
        );
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
    let key = engine_key(&model);
    let budget = Duration::from_secs(if input.startup_timeout_secs == 0 {
        DEFAULT_STARTUP_SECS
    } else {
        input.startup_timeout_secs
    } as u64);

    // 短锁：收割已死实例；同模型健康实例直接复用；不健康实例终止后重拉
    {
        let mut reg = registry();
        reap_dead(&mut reg);
        if let Some(pos) = reg.iter().position(|e| e.key == key) {
            if engine_healthy(&reg[pos].base_url) {
                let e = &reg[pos];
                output.base_url = e.base_url.clone();
                output.pid = e.child.id();
                output.model = model.display().to_string();
                metrics.counter("local_engine.reused", 1);
                return true;
            }
            let mut stale = reg.remove(pos);
            let _ = stale.child.kill();
            let _ = stale.child.wait();
        }
    } // 锁在此释放：拉起+健康等待最长可达启动预算，不能占锁
    let start_port = input.preferred_port.unwrap_or(18080);
    if let Some((base_url, mut child)) =
        try_ports(&engine_path, &model, input.context_len, start_port, budget)
    {
        let pid = child.id();
        // 短锁登记；并发竞争（他人已拉起同模型）则复用其并终止本实例
        let mut reg = registry();
        reap_dead(&mut reg);
        if let Some(pos) = reg.iter().position(|e| e.key == key) {
            if engine_healthy(&reg[pos].base_url) {
                let _ = child.kill();
                let _ = child.wait();
                let e = &reg[pos];
                output.base_url = e.base_url.clone();
                output.pid = e.child.id();
                output.model = model.display().to_string();
                metrics.counter("local_engine.reused", 1);
                return true;
            }
        }
        reg.push(EngineEntry {
            key,
            base_url: base_url.clone(),
            child,
        });
        output.base_url = base_url;
        output.pid = pid;
        output.model = model.display().to_string();
        metrics.counter("local_engine.started", 1);
        return true;
    }
    evolution.fail("IAC-1403", "本地引擎启动失败（端口尝试 10 次均未就绪）");
    false
}

/// 引擎复用键：模型文件规范路径。
fn engine_key(model: &Path) -> String {
    model
        .canonicalize()
        .unwrap_or_else(|_| model.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// 停止指定模型的引擎实例（会话关闭/模型卸载用）。返回是否停止了实例。
pub fn local_engine_stop(model_path: &Path) -> bool {
    let key = engine_key(model_path);
    let mut reg = registry();
    if let Some(pos) = reg.iter().position(|e| e.key == key) {
        let mut e = reg.remove(pos);
        let _ = e.child.kill();
        let _ = e.child.wait();
        true
    } else {
        false
    }
}

/// 停止全部引擎（daemon 退出收尾）。返回停止数量。
pub fn local_engine_stop_all() -> usize {
    let mut reg = registry();
    let n = reg.len();
    for mut e in reg.drain(..) {
        let _ = e.child.kill();
        let _ = e.child.wait();
    }
    n
}

/// 清理注册表中已退出的子进程（健康检查失败/崩溃后的尸体）。
fn reap_dead(reg: &mut Vec<EngineEntry>) {
    reg.retain_mut(|e| match e.child.try_wait() {
        Ok(Some(_)) => false, // 已退出：移除
        Ok(None) => true,
        Err(_) => true, // try_wait 罕见错误：保留条目，下次健康检查会淘汰
    });
}

/// 端口扫描启动（local_engine_ensure 子步骤）：成功返回 (端口, base_url, 子进程)。
/// 每次尝试失败即终止该子进程；进程早退时健康检查快速失败，不空等预算。
fn try_ports(
    engine_path: &Path,
    model: &Path,
    context_len: u32,
    start_port: u16,
    budget: Duration,
) -> Option<(String, Child)> {
    for port in start_port..start_port + 10 {
        if !port_free(port) {
            continue;
        }
        let Some(mut child) = spawn_engine(engine_path, model, context_len, port) else {
            continue;
        };
        if wait_healthy(port, budget, &mut child) {
            return Some((format!("http://127.0.0.1:{port}"), child));
        }
        // 未就绪/已退出：终止本次拉起的进程，防泄漏
        let _ = child.kill();
        let _ = child.wait();
    }
    None
}

/// 端口空闲检测。
fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 拉起引擎子进程（stdout/stderr 追加落日志文件；调用方负责入注册表或终止）。
fn spawn_engine(engine_path: &Path, model: &Path, context_len: u32, port: u16) -> Option<Child> {
    let mut cmd = Command::new(engine_path);
    cmd.arg("--model")
        .arg(model)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg(context_len.to_string())
        .stdin(Stdio::null());
    if let Some(dir) = engine_path.parent() {
        if let Ok(log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("engine.log"))
        {
            match log.try_clone() {
                Ok(stderr) => {
                    cmd.stdout(log).stderr(stderr);
                }
                Err(_) => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
            }
        }
    }
    cmd.spawn().ok()
}

/// 健康检查：GET /health 200 视为就绪；子进程退出立即失败（不空等预算）。
fn wait_healthy(port: u16, budget: Duration, child: &mut Child) -> bool {
    let deadline = std::time::Instant::now() + budget;
    let client = match crate::httpx::shared_client() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("http://127.0.0.1:{port}/health");
    loop {
        // 子进程已退出：端口永远不会就绪
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) => {}
            Err(_) => return false,
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        if let Ok(resp) = client.get(&url).timeout(Duration::from_millis(400)).send() {
            if resp.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// 健康检查复用导出（daemon 降级判定用）。
pub fn engine_healthy(base_url: &str) -> bool {
    let Ok(client) = crate::httpx::shared_client() else {
        return false;
    };
    client
        .get(format!("{base_url}/health"))
        .timeout(Duration::from_millis(400))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// JSON 视图（daemon 降级日志用）。
pub fn engine_status_json(healthy: bool, base_url: &str) -> serde_json::Value {
    let engines: Vec<serde_json::Value> = registered_engines()
        .into_iter()
        .map(|(pid, url, model)| json!({"pid": pid, "base_url": url, "model": model}))
        .collect();
    json!({"local_engine": {"healthy": healthy, "base_url": base_url, "engines": engines}})
}
