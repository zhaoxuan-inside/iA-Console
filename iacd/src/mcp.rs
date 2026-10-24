//! daemon 内置 MCP 服务（Q43）：stdio JSON-RPC MCP，进程内直调 dispatch（actor=agent:mcp，
//! 同闸门同审计 AC-08）。旧版 MCP 挂在 iac CLI 侧经 IPC 透传，重建按 Q43 收归 daemon。

use iac_core::framework::{Actor, Context, Deps, Locale, SessionCtx};
use iac_core::server::{dispatch_request, IpcRequest, IpcResponse};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 工具定义（域 9 四能力 + 诊断/翻译/会话辅助；与 IPC 方法同表）。
const TOOLS: &[(&str, &str)] = &[
    (
        "iac_ask",
        "意图→多候选提案（闸门在执行端）：输入 {intent, cwd?, shell?}",
    ),
    (
        "iac_execute",
        "执行命令（闸门+审计一致）：输入 {command, confirmed?, confirm_word?, origin?}",
    ),
    ("iac_explain", "参数解释：输入 {command}"),
    (
        "iac_diagnose",
        "诊断失败执行：输入 {execution_id? 或 command/exit_code/stderr}",
    ),
    (
        "iac_history_query",
        "历史检索（含 Agent 发起留痕）：输入 {q?, actor?, status?, limit?}",
    ),
    (
        "iac_recipes_search",
        "方案检索：输入 {q?, tag?, platform?, kind?, limit?}",
    ),
    (
        "iac_recipe_save",
        "保存方案：输入 {title, command_template, tags?, platform?, name?}",
    ),
    ("iac_translate", "习惯方言翻译：输入 {command, from?, to?}"),
    (
        "iac_sessions_current",
        "当前会话信息（事件流）：输入 {cwd?, shell?, host?}",
    ),
];

/// MCP stdio 主循环（阻塞至 EOF）。session 固定派生自 agent:mcp 连接。
pub fn stdio_loop(deps: Arc<Deps>, config: Arc<iac_core::config::Config>) -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let n = AtomicU64::new(0);
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // 通知类消息静默
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let resp = match method.as_str() {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "ia-console", "version": env!("CARGO_PKG_VERSION")}
                }
            }),
            "ping" => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => {
                let tools: Vec<serde_json::Value> = TOOLS
                    .iter()
                    .map(|(name, desc)| {
                        serde_json::json!({"name": name, "description": desc, "inputSchema": {"type": "object"}})
                    })
                    .collect();
                serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
            }
            "tools/call" => tools_call(&req, &deps, &config, &n),
            _ => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "method not found"}
            }),
        };
        if writeln!(stdout, "{resp}").is_err() {
            break; // 客户端关闭
        }
        let _ = stdout.flush();
    }
    0
}

/// tools/call：进程内直调 dispatch（同一入口，AC-08）。
fn tools_call(
    req: &serde_json::Value,
    deps: &Arc<Deps>,
    config: &Arc<iac_core::config::Config>,
    n: &AtomicU64,
) -> serde_json::Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let name = req["params"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let arguments = match req["params"]["arguments"].clone() {
        Value::Null => serde_json::json!({}),
        v => v,
    };
    if !name.starts_with("iac_") {
        return jsonrpc_error(&id, -32602, "未知工具");
    }
    let _ = n.fetch_add(1, Ordering::SeqCst);
    let ipc_req = IpcRequest {
        v: 1,
        id: format!("mcp-{}", n.load(Ordering::SeqCst)),
        method: name.clone(),
        params: arguments,
        actor: Some("agent:mcp".into()),
        session_id: None,
    };
    let session = SessionCtx {
        session_id: String::new(),
        request_id: ipc_req.id.clone(),
        actor: Actor::Agent("mcp".into()),
    };
    let locale = Locale {
        lang: "zh-CN".into(),
        timezone: "Asia/Shanghai".into(),
    };
    let ctx = Context {
        session: &session,
        config,
        deps,
        locale: &locale,
    };
    let mut out = IpcResponse {
        v: 1,
        id: ipc_req.id.clone(),
        ok: false,
        result: None,
        error: None,
    };
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let _ = dispatch_request(&ipc_req, &mut out, &ctx, &mut m, &mut evo);
    let text = if out.ok {
        serde_json::to_string_pretty(out.result.as_ref().unwrap_or(&Value::Null))
            .unwrap_or_default()
    } else {
        format!(
            "[{}] {}",
            out.error
                .as_ref()
                .map(|e| e.code.clone())
                .unwrap_or_default(),
            out.error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_default()
        )
    };
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {"content": [{"type": "text", "text": text}], "isError": !out.ok}
    })
}

fn jsonrpc_error(id: &Value, code: i32, msg: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}
