//! MCP stdio Server（T-023，豁免入口）：手写最小 JSON-RPC MCP（ADR-005 修订）。
//! Agent 经 stdio 调用 → IPC 透传 daemon（actor=agent:mcp，同闸门同审计 AC-08）。

use crate::ipc;
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// 工具定义（_06 external 第 2.2 节映射）。
const TOOLS: &[(&str, &str)] = &[
    ("iac_ask", "意图→提案：输入 {intent, cwd?}"),
    ("iac_execute", "执行命令（闸门+审计）：输入 {command, confirm_dangerous?, timeout_ms?}"),
    ("iac_explain", "参数解释：输入 {command}"),
    ("iac_diagnose", "诊断失败执行：输入 {execution_id?}"),
    ("iac_history_query", "历史检索：输入 {q?, cwd?, since?, limit?}"),
    ("iac_recipes_search", "方案检索：输入 {q?, tag?, platform?, limit?}"),
    ("iac_recipe_save", "保存方案：输入 {title, command_template, tags?, platform?}"),
    ("iac_sessions_current", "当前会话信息：输入 {}"),
];

/// MCP stdio 主循环（阻塞至 EOF）。
pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut n = AtomicU64::new(0);
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // 通知类消息静默
        };
        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or_default().to_string();
        let resp = match method.as_str() {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "ia-console", "version": "0.1.0"}
                }
            }),
            "ping" => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => {
                let tools: Vec<serde_json::Value> = TOOLS
                    .iter()
                    .map(|(name, desc)| serde_json::json!({"name": name, "description": desc, "inputSchema": {"type": "object"}}))
                    .collect();
                serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
            }
            "tools/call" => tools_call(&req, &mut n),
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

/// tools/call：名称映射到 IPC 方法，actor=agent:mcp（同闸门同审计）。
fn tools_call(req: &serde_json::Value, n: &mut AtomicU64) -> serde_json::Value {
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let name = req["params"]["name"].as_str().unwrap_or_default().to_string();
    let arguments = match req["params"]["arguments"].clone() {
        Value::Null => serde_json::json!({}),
        v => v,
    };
    if !name.starts_with("iac_") {
        return jsonrpc_error(&id, -32602, "未知工具");
    }
    let _ = n.fetch_add(1, Ordering::SeqCst);
    match ipc::request(&name, arguments, Some("agent:mcp")) {
        Ok(resp) => {
            let text = if resp.ok {
                serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&serde_json::Value::Null)).unwrap_or_default()
            } else {
                format!("[{}] {}", resp.error.as_ref().map(|e| e.code.clone()).unwrap_or_default(), resp.error.map(|e| e.message).unwrap_or_default())
            };
            let is_error = !resp.ok;
            serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"content": [{"type": "text", "text": text}], "isError": is_error}
            })
        }
        Err(e) => jsonrpc_error(&id, -32603, &e),
    }
}

fn jsonrpc_error(id: &serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
