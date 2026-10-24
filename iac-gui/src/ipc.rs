//! GUI 的 IPC 客户端（与 iac/src/ipc.rs 同协议：UDS JSONL 信封）。
//! 重建变化：daemon 自动拉起（Q40）、请求级 session_id（域 18 多标签）、取消登记（Q22）。
//! spawn_request 后台线程执行，UI 线程零阻塞。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 读超时：覆盖 LLM 往返与长命令（超过视为 daemon 死锁，报错解阻塞）。
const READ_TIMEOUT: Duration = Duration::from_secs(300);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// 缺省 socket 路径（infra::config 统一定义）。
pub fn default_socket_path() -> std::path::PathBuf {
    infra::config::socket_path()
}

/// daemon 自动拉起（Q40）：连接失败 → 启动 iacd（exe 同目录优先）→ 短退避重试。
/// 返回 Err 时携带最终错误（GUI 以 IAC-6101 条目呈现，不静默）。
pub fn ensure_daemon_and_request(
    method: &str,
    params: serde_json::Value,
    session_id: Option<String>,
) -> Result<IpcResponse, String> {
    let mut last_err = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500));
        }
        match request_inner(method, params.clone(), session_id.clone()) {
            Ok(resp) => return Ok(resp),
            Err(e) if e.starts_with("IAC-6101") && attempt == 0 => {
                last_err = e;
                spawn_daemon();
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err)
}

/// 拉起 iacd（脱离进程组；exe 同目录优先，其次 PATH）。
fn spawn_daemon() {
    let exe = std::env::current_exe().ok();
    let iacd = exe
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join("iacd"))
        .unwrap_or_else(|| std::path::PathBuf::from("iacd"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = std::process::Command::new(&iacd)
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(unix))]
    {
        let _ = iacd;
    }
}

/// 发送请求并取回响应；daemon 未运行/超时 → Err（IAC-6101 语义）。
pub fn request(
    method: &str,
    params: serde_json::Value,
    session_id: Option<String>,
) -> Result<IpcResponse, String> {
    request_inner(method, params, session_id)
}

fn request_inner(
    method: &str,
    params: serde_json::Value,
    session_id: Option<String>,
) -> Result<IpcResponse, String> {
    let stream = UnixStream::connect(default_socket_path())
        .map_err(|_| "IAC-6101: daemon 未运行，正在自动拉起…".to_string())?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(WRITE_TIMEOUT)))
        .map_err(|e| format!("IAC-6101: 设置超时失败 {e}"))?;
    let mut writer = stream.try_clone().expect("clone stream");
    static N: AtomicU64 = AtomicU64::new(0);
    let id = format!(
        "gui-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    );
    let envelope = serde_json::json!({
        "v": 1, "id": id, "method": method, "params": params, "session_id": session_id,
    });
    writer
        .write_all(format!("{envelope}\n").as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|e| format!("IAC-6101: 写入失败 {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| match e.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
            "IAC-6101: daemon 响应超时（300s），请检查 iacd 状态".to_string()
        }
        _ => format!("IAC-6101: 读取失败 {e}"),
    })?;
    if line.is_empty() {
        return Err("IAC-6101: daemon 提前断开连接".to_string());
    }
    serde_json::from_str(line.trim()).map_err(|e| format!("IAC-6101: 响应解析失败 {e}"))
}

/// 取消请求（Q22）：向 daemon 登记 request id，迟到结果将被改写 IAC-6107 丢弃。
pub fn cancel(request_id: &str) {
    let _ = request(
        "iac_cancel",
        serde_json::json!({ "request_id": request_id }),
        None,
    );
}

/// 后台线程发请求，响应经 reply_tx 送回 UI 线程（线程创建失败也要回传，防止永久 busy）。
pub fn spawn_request(
    method: &'static str,
    params: serde_json::Value,
    session_id: Option<String>,
    reply_tx: Sender<(&'static str, Result<IpcResponse, String>)>,
) {
    let tx = reply_tx.clone();
    let spawned = std::thread::Builder::new()
        .name("iac-ipc-client".into())
        .spawn(move || {
            let result = ensure_daemon_and_request(method, params, session_id);
            let _ = tx.send((method, result));
        });
    if spawned.is_err() {
        let _ = reply_tx.send((method, Err("IAC-6101: IPC 线程创建失败".to_string())));
    }
}

/// 提取提案 id（iac_ask 的 JSON result）。
pub fn extract_proposal_id(resp: &IpcResponse) -> Option<String> {
    resp.result
        .as_ref()?
        .get("id")?
        .as_str()
        .map(str::to_string)
}
