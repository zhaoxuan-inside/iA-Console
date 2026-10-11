//! iac IPC 客户端：阻塞 UDS JSONL（入口豁免层的唯一 daemon 触达点）。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// 缺省 socket 路径（iac_core::server 统一定义）。
pub fn default_socket_path() -> PathBuf {
    iac_core::server::default_socket_path()
}

/// 发送一次请求并取回响应。daemon 未运行 → IAC-6101 错误信封。
pub fn request(method: &str, params: serde_json::Value, actor: Option<&str>) -> Result<IpcResponse, String> {
    let socket = default_socket_path();
    let Ok(stream) = UnixStream::connect(&socket) else {
        return Err("IAC-6101: daemon 未运行，请先执行 iac daemon start".into());
    };
    let mut writer = stream.try_clone().expect("clone stream");
    let envelope = serde_json::json!({
        "v": 1, "id": uuid_like_id(), "method": method, "params": params,
        "actor": actor,
    });
    writer
        .write_all(format!("{envelope}\n").as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|e| format!("IAC-6101: 写入失败 {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("IAC-6101: 读取失败 {e}"))?;
    serde_json::from_str(line.trim()).map_err(|e| format!("IAC-6101: 响应解析失败 {e}"))
}

/// 请求 ID（毫秒时间戳 + 计数，前端本地生成即可）。
fn uuid_like_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    format!("cli-{}-{n}", std::process::id())
}

/// 渲染响应：ok → result JSON 或人类摘要；error → stderr 风格字符串。
pub fn render(resp: &IpcResponse, json_mode: bool) -> String {
    if resp.ok {
        if json_mode {
            return serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&serde_json::Value::Null)).unwrap_or_default();
        }
        return humanize(resp.result.as_ref().unwrap_or(&serde_json::Value::Null));
    }
    let err = resp.error.as_ref().expect("ok=false 必有 error");
    format!("[{}] {}", err.code, err.message)
}

/// 人类可读摘要（iA Writer 式：留白、无框、语义信息）。
fn humanize(result: &serde_json::Value) -> String {
    let obj = match result.as_object() {
        Some(o) => o,
        None => return result.to_string(),
    };
    // 提案卡（摘要态）
    if let (Some(cmd), Some(risk)) = (obj.get("command").and_then(|v| v.as_str()), obj.get("risk_level").and_then(|v| v.as_str())) {
        let mut lines = vec![format!("  ▍ {cmd}")];
        if let Some(summary) = obj["explanation"]["summary"].as_str() {
            lines.push(format!("\n    {summary}"));
        }
        lines.push(format!("\n    风险：{risk}"));
        lines.push("    [执行 iac run --proposal <id>] [取消]".to_string());
        if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
            lines.push(format!("    提案 ID：{id}"));
        }
        return lines.join("\n");
    }
    // 执行结果
    if let Some(status) = obj.get("status").and_then(|v| v.as_str()) {
        let mut lines = vec![format!("  状态：{status}")];
        if let Some(code) = obj.get("exit_code") {
            lines.push(format!("  退出码：{code}"));
        }
        if let Some(out) = obj.get("stdout_summary").and_then(|v| v.as_str()) {
            lines.push(format!("\n{out}"));
        }
        if let Some(diag) = obj.get("diagnosis") {
            if !diag.is_null() {
                lines.push(format!("\n  诊断：{}", diag["root_cause"].as_str().unwrap_or("")));
                if let Some(note) = diag["rollback_note"].as_str() {
                    lines.push(format!("  {}", note));
                }
            }
        }
        return lines.join("\n");
    }
    serde_json::to_string_pretty(result).unwrap_or_default()
}
