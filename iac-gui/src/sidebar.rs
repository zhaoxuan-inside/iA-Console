//! AI 侧栏状态与提交逻辑（纯逻辑部分可单测；渲染在 app.rs）。
//! chg-021：IPC 全异步——perform 只投递，响应经通道回 UI 线程再落条目，窗口不再冻结。

use super::ipc;
use iac_core::common::marker_parser::{parse_input_marker, InputDecision, RawInput};
use iac_core::framework::Evolution;
use iac_core::server::IpcResponse;
use std::sync::mpsc::Sender;

/// 侧栏条目。
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Output(String),
    Notice(String),
    Separator,
    Note(String),
    /// 提案卡：命令 + 提案 id + 风险级（yes/no 确认流）。
    Proposal { id: String, command: String, risk: String },
}

/// 侧栏状态。
#[derive(Debug, Default)]
pub struct SidebarState {
    pub input: String,
    pub entries: Vec<Entry>,
    pub last_proposal: Option<String>,
    pub last_command: Option<String>,
    pub busy: bool,
}

/// 提交结果：需要发起的 IPC 调用（method, params）或本地提示。
pub enum Action {
    Ipc(&'static str, serde_json::Value),
    Notice(String),
    Separator,
    Note(String),
    /// 确认/拒绝最近提案。
    Confirm(bool),
}

/// 输入 → Action（标记语义；yes/no 走提案确认，AC-46）。
pub fn plan_action(line: &str, has_proposal: bool) -> Action {
    let env = iac_core::framework::TestEnv::new();
    let mut decision = InputDecision::Command { command: String::new() };
    let mut m = iac_core::framework::Metrics::default();
    let mut evo = Evolution::default();
    let _ = parse_input_marker(&RawInput { line: line.to_string() }, &mut decision, &env.context(), &mut m, &mut evo);
    match decision {
        InputDecision::Command { command } if command == "yes" || command == "no" => {
            if has_proposal {
                Action::Confirm(command == "yes")
            } else {
                Action::Notice("当前没有待确认提案".into())
            }
        }
        InputDecision::Command { command } => {
            Action::Ipc("iac_execute", serde_json::json!({"command": command, "origin": "direct"}))
        }
        InputDecision::AiAsk { intent } => Action::Ipc("iac_ask", serde_json::json!({"intent": intent, "cwd": "."})),
        InputDecision::AiExplain { command } => Action::Ipc("iac_explain", serde_json::json!({"command": command})),
        InputDecision::TaskSeparator => Action::Separator,
        InputDecision::Note { text } => Action::Note(text),
        InputDecision::UnknownSlash { name } => Action::Notice(format!("[IAC-1501] 未知管理命令 /{name}")),
        InputDecision::Literal { command } => {
            Action::Ipc("iac_execute", serde_json::json!({"command": command, "origin": "direct"}))
        }
        InputDecision::Slash { name, args: _ } => match name.as_str() {
            "help" => Action::Notice(HELP.into()),
            "history" => Action::Ipc("iac_history_query", serde_json::json!({"limit": 20})),
            "recipe" => Action::Ipc("iac_recipes_search", serde_json::json!({"limit": 20})),
            "sessions" => Action::Ipc("iac_sessions_current", serde_json::json!({"cwd": "."})),
            other => Action::Notice(format!("/{other} 在侧栏不可用（/help 查看）")),
        },
    }
}

/// 执行 Action：IPC 类后台投递（响应经 reply_tx 回 UI 线程），本地类直接落条目。
pub fn perform(
    state: &mut SidebarState,
    action: Action,
    reply_tx: &Sender<(&'static str, Result<IpcResponse, String>)>,
) {
    if state.busy {
        state.entries.push(Entry::Notice("上一次请求处理中…".into()));
        return;
    }
    match action {
        Action::Notice(text) => state.entries.push(Entry::Notice(text)),
        Action::Separator => {
            state.entries.push(Entry::Separator);
            state.entries.push(Entry::Notice("—— 新任务段 ——".into()));
        }
        Action::Note(text) => state.entries.push(Entry::Note(text)),
        Action::Ipc(method, params) => {
            state.busy = true;
            ipc::spawn_request(method, params, reply_tx.clone());
        }
        Action::Confirm(yes) => {
            let Some(pid) = state.last_proposal.clone() else {
                state.entries.push(Entry::Notice("当前没有待确认提案".into()));
                return;
            };
            state.last_proposal = None;
            if yes {
                state.busy = true;
                ipc::spawn_request(
                    "iac_execute",
                    serde_json::json!({"proposal_id": pid, "confirm_dangerous": true, "origin": "proposal"}),
                    reply_tx.clone(),
                );
            } else {
                state.entries.push(Entry::Notice(format!("提案 {pid} 已拒绝")));
            }
        }
    }
}

/// IPC 响应落条目并复位 busy（UI 线程每帧调用；纯状态变换）。
pub fn apply_ipc_result(
    state: &mut SidebarState,
    method: &str,
    resp: &Result<IpcResponse, String>,
) {
    state.busy = false;
    match resp {
        Err(e) => state.entries.push(Entry::Notice(e.clone())),
        Ok(r) => {
            for entry in render_result(method, r, state.last_command.as_deref()) {
                state.entries.push(entry);
            }
            if method == "iac_ask" {
                if let Some(id) = ipc::extract_proposal_id(r) {
                    state.last_proposal = Some(id);
                }
                if let Some(cmd) = ipc::extract_command(r) {
                    state.last_command = Some(cmd);
                }
            }
        }
    }
}

/// 响应 → 人类可读条目（核心字段优先，缺失回退紧凑 JSON）。
pub fn render_result(
    method: &str,
    resp: &IpcResponse,
    last_command: Option<&str>,
) -> Vec<Entry> {
    if !resp.ok {
        let err = resp
            .error
            .as_ref()
            .map(|e| format!("[{}] {}", e.code, e.message))
            .unwrap_or_else(|| "[IAC-1102] 未知错误".into());
        return vec![Entry::Notice(err)];
    }
    let result = resp.result.as_ref().unwrap_or(&serde_json::Value::Null);
    match method {
        "iac_ask" => vec![render_proposal(result)],
        "iac_execute" => vec![render_execution(result, last_command)],
        "iac_explain" => vec![render_explain(result)],
        _ => vec![Entry::Output(compact_json(result))],
    }
}

/// 提案 → 提案卡条目（命令缺失说明是纯回答，回退紧凑 JSON）。
fn render_proposal(v: &serde_json::Value) -> Entry {
    let command = str_field(v, "command");
    if command.is_empty() {
        return Entry::Output(compact_json(v));
    }
    Entry::Proposal {
        id: str_field(v, "id"),
        command,
        risk: str_field(v, "risk_level").to_lowercase(),
    }
}

/// 执行结果 → 命令 + 退出码 + 摘要。
fn render_execution(v: &serde_json::Value, last_command: Option<&str>) -> Entry {
    let command = last_command.unwrap_or("?");
    let exit = v
        .get("exit_code")
        .and_then(|x| x.as_i64())
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".into());
    let status = str_field(v, "status");
    let mut lines = vec![format!("▸ {command}"), format!("  退出码 {exit} · {status}")];
    for (tag, key) in [("", "stdout_summary"), ("[stderr] ", "stderr_summary")] {
        if let Some(text) = v.get(key).and_then(|x| x.as_str()) {
            if !text.is_empty() {
                lines.push(format!("  {tag}{}", text.trim_end()));
            }
        }
    }
    Entry::Output(lines.join("\n"))
}

/// 参数解释 → 摘要 + 影响 + 参数表。
fn render_explain(v: &serde_json::Value) -> Entry {
    let mut lines = Vec::new();
    let summary = str_field(v, "summary");
    if !summary.is_empty() {
        lines.push(summary);
    }
    let impact = str_field(v, "impact");
    if !impact.is_empty() {
        lines.push(format!("影响：{impact}"));
    }
    if let Some(params) = v.get("params").and_then(|x| x.as_array()) {
        for p in params {
            lines.push(format!("  {} {}", str_field(p, "flag"), str_field(p, "meaning")));
        }
    }
    if lines.is_empty() {
        Entry::Output(compact_json(v))
    } else {
        Entry::Output(lines.join("\n"))
    }
}

/// 紧凑 JSON 文本（超长截断）。
fn compact_json(v: &serde_json::Value) -> String {
    let text = serde_json::to_string(v).unwrap_or_default();
    if text.chars().count() > 400 {
        let cut: String = text.chars().take(400).collect();
        format!("{cut}…")
    } else {
        text
    }
}

/// 取字符串字段（缺失回空串）。
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

const HELP: &str = "AI 侧栏标记：\n  直接输入      经闸门执行\n  ??ls / ??问句  AI 解释/提案\n  yes / no      确认最近提案\n  /history /recipe /sessions\n  --- 分段 / # 备注\n  \\命令 按字面执行";

#[cfg(test)]
mod tests {
    use super::*;
    use iac_core::server::envelope::IpcRequest;

    fn test_request() -> IpcRequest {
        IpcRequest {
            v: 1,
            id: "t".into(),
            method: "iac_ask".into(),
            params: serde_json::Value::Null,
            actor: None,
        }
    }

    fn ok_resp(result: serde_json::Value) -> IpcResponse {
        IpcResponse::success(&test_request(), result)
    }

    fn err_resp(code: &str, message: &str) -> IpcResponse {
        IpcResponse::failure(&test_request(), code, message)
    }

    #[test]
    fn plan_maps_markers() {
        assert!(matches!(plan_action("??ls", false), Action::Ipc("iac_explain", _)));
        assert!(matches!(plan_action("怎么找大文件", false), Action::Ipc("iac_execute", _)));
        assert!(matches!(plan_action("---", false), Action::Separator));
        assert!(matches!(plan_action("# 备注", false), Action::Note(_)));
    }

    #[test]
    fn yes_no_requires_pending_proposal() {
        assert!(matches!(plan_action("yes", false), Action::Notice(_)));
        assert!(matches!(plan_action("yes", true), Action::Confirm(true)));
        assert!(matches!(plan_action("no", true), Action::Confirm(false)));
    }

    #[test]
    fn perform_ipc_goes_async_and_result_applies() {
        // 有 daemon 则成功、无 daemon 则错误——均经通道回传，UI 线程不阻塞
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = SidebarState::default();
        perform(&mut state, plan_action("??如何列出文件", false), &tx);
        assert!(state.busy, "提交后应置 busy");
        let (method, resp) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("后台线程必须回传响应");
        assert_eq!(method, "iac_ask");
        apply_ipc_result(&mut state, method, &resp);
        assert!(!state.busy, "响应落地后 busy 复位");
        assert!(!state.entries.is_empty());
    }

    #[test]
    fn confirm_yes_dispatches_execute_and_clears_proposal() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = SidebarState { last_proposal: Some("p-1".into()), ..Default::default() };
        perform(&mut state, plan_action("yes", true), &tx);
        assert!(state.last_proposal.is_none(), "确认后清提案指针");
        let (method, _) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("确认必须发起 IPC");
        assert_eq!(method, "iac_execute");
    }

    #[test]
    fn render_ask_makes_proposal_card() {
        let resp = ok_resp(serde_json::json!({
            "id": "p-9", "command": "du -h --max-depth=1", "risk_level": "Caution"
        }));
        assert_eq!(
            render_result("iac_ask", &resp, None),
            vec![Entry::Proposal {
                id: "p-9".into(),
                command: "du -h --max-depth=1".into(),
                risk: "caution".into(),
            }]
        );
    }

    #[test]
    fn render_execute_summarizes_exit() {
        let resp = ok_resp(serde_json::json!({
            "execution_id": "e-1", "exit_code": 0, "status": "ok",
            "stdout_summary": "total 0", "stderr_summary": null
        }));
        let entries = render_result("iac_execute", &resp, Some("ls -la"));
        match &entries[0] {
            Entry::Output(text) => {
                assert!(text.contains("▸ ls -la"), "缺命令行: {text}");
                assert!(text.contains("退出码 0"), "缺退出码: {text}");
                assert!(text.contains("total 0"), "缺输出摘要: {text}");
            }
            other => panic!("应为 Output 条目，实际 {other:?}"),
        }
    }

    #[test]
    fn render_error_makes_notice() {
        let resp = err_resp("IAC-3101", "风险闸门拒绝");
        assert_eq!(
            render_result("iac_execute", &resp, None),
            vec![Entry::Notice("[IAC-3101] 风险闸门拒绝".into())]
        );
    }

    #[test]
    fn apply_ipc_error_unblocks_and_records() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut state = SidebarState { busy: true, ..Default::default() };
        perform(&mut state, Action::Ipc("iac_ping", serde_json::json!({})), &tx);
        let resp = Err("IAC-6101: daemon 未运行".to_string());
        apply_ipc_result(&mut state, "iac_ping", &resp);
        assert!(!state.busy);
        assert!(matches!(&state.entries[1], Entry::Notice(t) if t.contains("IAC-6101")));
    }
}
