//! GUI 的极简 IPC 客户端（与 iac/src/ipc.rs 同协议：UDS JSONL 信封）。
//! 前端豁免层；与 bin 间不复用以避免 crate 互依赖。
//! chg-021：读写加超时；spawn_request 后台线程执行，UI 线程零阻塞。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 读超时：覆盖 LLM 往返与长命令（超过视为 daemon 死锁，报错解阻塞）。
const READ_TIMEOUT: Duration = Duration::from_secs(300);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// 缺省 socket 路径（core::server 统一定义）。
pub fn default_socket_path() -> std::path::PathBuf {
    iac_core::server::default_socket_path()
}

/// 发送请求并取回响应；daemon 未运行/超时 → Err（IAC-6101 语义）。
pub fn request(method: &str, params: serde_json::Value) -> Result<IpcResponse, String> {
    let stream =
        UnixStream::connect(default_socket_path()).map_err(|_| "IAC-6101: daemon 未运行，请先启动 iacd".to_string())?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(WRITE_TIMEOUT)))
        .map_err(|e| format!("IAC-6101: 设置超时失败 {e}"))?;
    let mut writer = stream.try_clone().expect("clone stream");
    static N: AtomicU64 = AtomicU64::new(0);
    let id = format!("gui-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst));
    let envelope = serde_json::json!({"v": 1, "id": id, "method": method, "params": params});
    writer
        .write_all(format!("{envelope}\n").as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|e| format!("IAC-6101: 写入失败 {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| match e.kind() {
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

/// 后台线程发请求，响应经 reply_tx 送回 UI 线程（线程创建失败也要回传，防止侧栏永久 busy）。
pub fn spawn_request(
    method: &'static str,
    params: serde_json::Value,
    reply_tx: Sender<(&'static str, Result<IpcResponse, String>)>,
) {
    let tx = reply_tx.clone();
    let spawned = std::thread::Builder::new()
        .name("iac-ipc-client".into())
        .spawn(move || {
            let result = request(method, params);
            let _ = tx.send((method, result));
        });
    if spawned.is_err() {
        let _ = reply_tx.send((method, Err("IAC-6101: IPC 线程创建失败".to_string())));
    }
}

/// 提取提案 id（iac_ask 的 JSON result）。
pub fn extract_proposal_id(resp: &IpcResponse) -> Option<String> {
    resp.result.as_ref()?.get("id")?.as_str().map(str::to_string)
}

/// 提取命令文本（iac_ask 的 JSON result）。
pub fn extract_command(resp: &IpcResponse) -> Option<String> {
    resp.result.as_ref()?.get("command")?.as_str().map(str::to_string)
}
