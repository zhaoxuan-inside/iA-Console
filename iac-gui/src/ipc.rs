//! GUI 的极简 IPC 客户端（与 iac/src/ipc.rs 同协议：UDS JSONL 信封）。
//! 前端豁免层；与 bin 间不复用以避免 crate 互依赖。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};

/// 缺省 socket 路径（core::server 统一定义）。
pub fn default_socket_path() -> std::path::PathBuf {
    iac_core::server::default_socket_path()
}

/// 发送请求并取回响应；daemon 未运行 → Err（含 IAC-6101 语义）。
pub fn request(method: &str, params: serde_json::Value) -> Result<IpcResponse, String> {
    let stream = UnixStream::connect(default_socket_path())
        .map_err(|_| "IAC-6101: daemon 未运行，请先启动 iacd".to_string())?;
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
    reader.read_line(&mut line).map_err(|e| format!("IAC-6101: 读取失败 {e}"))?;
    serde_json::from_str(line.trim()).map_err(|e| format!("IAC-6101: 响应解析失败 {e}"))
}

/// 渲染为侧栏文本（ok → result JSON 紧凑串；error → [code] msg）。
pub fn render(resp: &IpcResponse) -> String {
    if resp.ok {
        serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&serde_json::Value::Null))
            .unwrap_or_default()
    } else {
        let err = resp.error.as_ref().expect("ok=false 必有 error");
        format!("[{}] {}", err.code, err.message)
    }
}

/// 提取提案 id（iac_ask 的 JSON result）。
pub fn extract_proposal_id(resp: &IpcResponse) -> Option<String> {
    resp.result.as_ref()?.get("id")?.as_str().map(str::to_string)
}

/// 提取命令文本。
pub fn extract_command(resp: &IpcResponse) -> Option<String> {
    resp.result.as_ref()?.get("command")?.as_str().map(str::to_string)
}
