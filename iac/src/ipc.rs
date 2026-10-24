//! iac IPC 客户端：阻塞 UDS JSONL（入口豁免层的唯一 daemon 触达点）。
//! 重建变化：请求级 session_id 绑定（域 18 多标签独立会话）、多候选提案渲染（Q17）、
//! 失败兜底提示（Q18）、ia-help 就地渲染。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 缺省 socket 路径（infra::config 统一定义）。
pub fn default_socket_path() -> PathBuf {
    infra::config::socket_path()
}

/// 进程级会话绑定：首个请求前取/建当前会话，后续请求回显 session_id（Q45 会话归属真实化）。
fn session_id() -> Option<String> {
    static SID: OnceLock<Option<String>> = OnceLock::new();
    SID.get_or_init(|| {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".into());
        let resp = request_inner(
            "iac_sessions_current",
            serde_json::json!({"cwd": cwd, "shell": "sh", "host": "local"}),
            None,
            None,
        )
        .ok()
        .filter(|r| r.ok)?;
        resp.result?
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
    .clone()
}

/// 发送一次请求并取回响应。daemon 未运行 → IAC-6101 错误信封。
pub fn request(method: &str, params: serde_json::Value, actor: Option<&str>) -> Result<IpcResponse, String> {
    request_inner(method, params, actor, session_id())
}

/// 内部发送（session 绑定前也会用到）。
fn request_inner(
    method: &str,
    params: serde_json::Value,
    actor: Option<&str>,
    sid: Option<String>,
) -> Result<IpcResponse, String> {
    let socket = default_socket_path();
    let Ok(stream) = UnixStream::connect(&socket) else {
        return Err("IAC-6101: daemon 未运行，请先执行 iac daemon start".into());
    };
    let mut writer = stream.try_clone().expect("clone stream");
    let envelope = serde_json::json!({
        "v": 1, "id": uuid_like_id(), "method": method, "params": params,
        "actor": actor, "session_id": sid,
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
            return serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&serde_json::Value::Null))
                .unwrap_or_default();
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
    // 内置命令 ia-help 就地列表
    if let Some(text) = obj.get("help_text").and_then(|v| v.as_str()) {
        return text.to_string();
    }
    // 提案卡（多候选 Q17：列全部候选 + 活动位标记）
    if let Some(candidates) = obj.get("candidates").and_then(|v| v.as_array()) {
        let active = obj.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let mut lines = vec!["  提案候选（↑↓切换候选，GUI；CLI 用 --candidate-index）:".to_string()];
        for (i, c) in candidates.iter().enumerate() {
            let mark = if i == active { "▍" } else { " " };
            let cmd = c["command"].as_str().unwrap_or_default();
            let risk = c["risk_level"].as_str().unwrap_or("safe");
            let summary = c["summary"].as_str().unwrap_or_default();
            lines.push(format!("  {mark} [{risk}] {cmd}"));
            if !summary.is_empty() {
                lines.push(format!("      {summary}"));
            }
        }
        if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
            lines.push(format!(
                "\n    [执行 iac run --proposal {id} --yes] [取消]",
            ));
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
        // 失败兜底（Q18）：就地「要不要 AI 帮忙？」
        if obj.get("offer_help").and_then(|v| v.as_bool()).unwrap_or(false) {
            lines.push("\n  命令失败了——要不要 AI 帮忙？运行 `iac doctor` 一键诊断".into());
        }
        return lines.join("\n");
    }
    serde_json::to_string_pretty(result).unwrap_or_default()
}
