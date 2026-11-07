//! iac IPC 客户端：阻塞 UDS JSONL（入口豁免层的唯一 daemon 触达点）。
//! 重建变化：请求级 session_id 绑定（域 18 多标签独立会话，恢复后可重绑）、
//! 连接/读/写三段超时（防 daemon 挂起永久阻塞）、多候选提案渲染（Q17）、
//! 失败兜底提示（Q18）、ia-help 就地渲染、stderr 与闸门升级提示。

use iac_core::server::IpcResponse;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 缺省 socket 路径（infra::config 统一定义）。
pub fn default_socket_path() -> PathBuf {
    infra::config::socket_path()
}

/// 连接超时：socket 存在但 daemon 死锁时快速失败。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 写超时：单请求信封很小，10s 足够。
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// 读超时：须容纳长命令执行（exec.timeout_ms 级）与 AI 调用（分钟级），
/// 取宽上限防 daemon 挂起时永久阻塞。
const READ_TIMEOUT: Duration = Duration::from_secs(600);

/// 客户端真实 cwd（Q45 字段真实性：cwd/捕获根随客户端而非 daemon 进程）。
pub fn client_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".into())
}

/// 客户端登录 shell 名（会话登记用）。
pub fn client_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sh".into())
}

/// 进程级会话绑定：首个请求前取/建当前会话，后续请求回显 session_id；
/// `iac session resume` 后经 rebind_session 重绑，恢复对同进程后续请求生效（Q45/Q46）。
static SESSION: Mutex<Option<String>> = Mutex::new(None);

fn session_id() -> Option<String> {
    let mut guard = SESSION.lock().ok()?;
    if guard.is_none() {
        let resp = request_inner(
            "iac_sessions_current",
            serde_json::json!({"cwd": client_cwd(), "shell": client_shell(), "host": "local"}),
            None,
            None,
        )
        .ok()
        .filter(|r| r.ok)?;
        let id = resp
            .result?
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        *guard = id;
    }
    guard.clone()
}

/// 会话恢复后重绑进程级 session（Q46：恢复对同进程后续请求生效）。
pub fn rebind_session(id: &str) {
    if let Ok(mut guard) = SESSION.lock() {
        *guard = Some(id.to_string());
    }
}

/// 发送一次请求并取回响应。daemon 未运行 → IAC-6101 错误信封。
pub fn request(
    method: &str,
    params: serde_json::Value,
    actor: Option<&str>,
) -> Result<IpcResponse, String> {
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
    let stream = connect_bounded(&socket, CONNECT_TIMEOUT)?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(|e| format!("IAC-6101: 写超时设置失败 {e}"))?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| format!("IAC-6101: 读超时设置失败 {e}"))?;
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
    reader.read_line(&mut line).map_err(|e| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            format!(
                "IAC-6101: daemon {}s 无响应（读取超时）",
                READ_TIMEOUT.as_secs()
            )
        }
        _ => format!("IAC-6101: 读取失败 {e}"),
    })?;
    serde_json::from_str(line.trim()).map_err(|e| format!("IAC-6101: 响应解析失败 {e}"))
}

/// 有界连接：connect 在工作线程执行，超时即报错（后端 UDS connect 对
/// backlog 满的 socket 可能阻塞；本工具链 std 无 UnixStream::connect_timeout）。
/// 超时后残留线程随进程退出消亡（CLI 短生命周期，无泄漏面）。
fn connect_bounded(socket: &std::path::Path, timeout: Duration) -> Result<UnixStream, String> {
    let path = socket.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&path));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(_)) => Err("IAC-6101: daemon 未运行，请先执行 iac daemon start".into()),
        Err(_) => Err(format!(
            "IAC-6101: 连接超时（{}s）：daemon 无响应",
            timeout.as_secs()
        )),
    }
}

/// 请求 ID（进程号 + 递增计数，前端本地生成即可；进程内唯一）。
fn uuid_like_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: OnceLock<AtomicU64> = OnceLock::new();
    let n = N
        .get_or_init(AtomicU64::default)
        .fetch_add(1, Ordering::SeqCst);
    format!("cli-{}-{n}", std::process::id())
}

/// 渲染响应：ok → result JSON 或人类摘要；error → stderr 风格字符串。
/// 人类模式下信封附错误参考（ok=true 但带 error，如闸门拒绝 IAC-2105 的原因）时追加附注。
pub fn render(resp: &IpcResponse, json_mode: bool) -> String {
    if resp.ok {
        let body = if json_mode {
            serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&serde_json::Value::Null))
                .unwrap_or_default()
        } else {
            humanize(resp.result.as_ref().unwrap_or(&serde_json::Value::Null))
        };
        if !json_mode {
            if let Some(err) = &resp.error {
                return format!("{body}\n\n[{}] {}", err.code, err.message);
            }
        }
        return body;
    }
    // 契约 ok=false 必有 error；违约时兜底不 panic（CLI 不因信封异常而崩溃）
    let err = resp
        .error
        .as_ref()
        .map(|e| format!("[{}] {}", e.code, e.message))
        .unwrap_or_else(|| "[IAC-6100] 未知错误（信封缺 error 详情）".to_string());
    err
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
        let mut lines =
            vec!["  提案候选（yes 执行活动候选；[/] 切换，CLI 用 --candidate-index）:".to_string()];
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
            lines.push(format!("\n    [执行 iac run --proposal {id} --yes] [取消]"));
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
        // stderr 摘要：失败场景往往是关键信息，不再丢弃
        if let Some(err_out) = obj.get("stderr_summary").and_then(|v| v.as_str()) {
            if !err_out.trim().is_empty() {
                lines.push("\n[stderr]".to_string());
                lines.push(err_out.to_string());
            }
        }
        // 闸门升级提示（Q35/Q37：被拒时给出客户端下一步动作）
        if status == "Rejected" {
            match obj
                .get("gate")
                .and_then(|g| g.get("action"))
                .and_then(|v| v.as_str())
            {
                Some("require_double_enter") => {
                    lines.push("\n  dangerous：需二次确认（再次输入 yes / 加 --yes）".into());
                }
                Some("require_confirm_word") => {
                    lines.push("\n  destructive：需输入「确认 <命令尾词>」".into());
                }
                _ => {}
            }
        }
        // 失败兜底（Q18）：就地「要不要 AI 帮忙？」
        if obj
            .get("offer_help")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            lines.push("\n  命令失败了——要不要 AI 帮忙？运行 `iac doctor` 一键诊断".into());
        }
        return lines.join("\n");
    }
    serde_json::to_string_pretty(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iac_core::error::IacError;

    fn resp_ok(result: serde_json::Value) -> IpcResponse {
        IpcResponse {
            v: 1,
            id: "t".into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    #[test]
    fn humanize_shows_stderr_when_present() {
        let r = resp_ok(serde_json::json!({
            "status": "Failed", "exit_code": 1,
            "stdout_summary": "部分输出",
            "stderr_summary": "No such file",
        }));
        let text = render(&r, false);
        assert!(text.contains("状态：Failed"));
        assert!(text.contains("[stderr]"));
        assert!(text.contains("No such file"));
    }

    #[test]
    fn humanize_gate_hints_on_rejected() {
        let r = resp_ok(serde_json::json!({
            "status": "Rejected",
            "gate": {"level": "destructive", "action": "require_confirm_word"},
        }));
        let text = render(&r, false);
        assert!(text.contains("确认 <命令尾词>"));
    }

    #[test]
    fn render_appends_envelope_error_hint_in_human_mode() {
        let mut r = resp_ok(serde_json::json!({"status": "Rejected"}));
        r.error = Some(IacError::new(
            "IAC-2105",
            "destructive 命令需要输入确认词「确认 sdb1」",
        ));
        let text = render(&r, false);
        assert!(text.contains("[IAC-2105]"));
        assert!(text.contains("确认 sdb1"));
        // JSON 模式保持纯 JSON，不附注
        assert!(!render(&r, true).contains("[IAC-2105]"));
    }

    #[test]
    fn render_survives_missing_error_on_failure_envelope() {
        // 契约违约兜底：ok=false 且 error 缺失 → 不得 panic，给可读提示
        let mut r = resp_ok(serde_json::json!({}));
        r.ok = false;
        r.error = None;
        let text = render(&r, false);
        assert!(text.contains("IAC-6100"), "兜底错误码可见：{text}");
    }
}
