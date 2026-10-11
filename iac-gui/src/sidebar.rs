//! AI 侧栏状态与提交逻辑（纯逻辑部分可单测；渲染在 app.rs）。

use super::ipc;
use iac_core::common::marker_parser::{parse_input_marker, InputDecision, RawInput};
use iac_core::framework::Evolution;

/// 侧栏条目。
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Output(String),
    Notice(String),
    Separator,
    Note(String),
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

/// 执行 Action：发 IPC 并落条目（busy 期间忽略新提交）。
pub fn perform(state: &mut SidebarState, action: Action) {
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
        Action::Confirm(yes) => {
            let Some(pid) = state.last_proposal.clone() else {
                state.entries.push(Entry::Notice("当前没有待确认提案".into()));
                return;
            };
            if yes {
                state.busy = true;
                let resp = ipc::request(
                    "iac_execute",
                    serde_json::json!({"proposal_id": pid, "confirm_dangerous": true, "origin": "proposal"}),
                );
                state.busy = false;
                match resp {
                    Ok(r) => {
                        state.last_command = ipc::extract_command(&r);
                        state.entries.push(Entry::Output(ipc::render(&r)));
                    }
                    Err(e) => state.entries.push(Entry::Notice(e)),
                }
            } else {
                state.entries.push(Entry::Notice(format!("提案 {pid} 已拒绝")));
            }
            state.last_proposal = None;
        }
        Action::Ipc(method, params) => {
            state.busy = true;
            let resp = ipc::request(method, params);
            state.busy = false;
            match resp {
                Ok(r) => {
                    if method == "iac_ask" {
                        if let Some(id) = ipc::extract_proposal_id(&r) {
                            state.last_proposal = Some(id.clone());
                            state.entries.push(Entry::Notice(format!("提案 {id} → yes 执行 / no 拒绝")));
                        }
                        if let Some(cmd) = ipc::extract_command(&r) {
                            state.last_command = Some(cmd);
                        }
                    }
                    state.entries.push(Entry::Output(ipc::render(&r)));
                }
                Err(e) => state.entries.push(Entry::Notice(e)),
            }
        }
    }
}

const HELP: &str = "AI 侧栏标记：\n  直接输入      经闸门执行\n  ??ls / ??问句  AI 解释/提案\n  yes / no      确认最近提案\n  /history /recipe /sessions\n  --- 分段 / # 备注\n  \\命令 按字面执行";

#[cfg(test)]
mod tests {
    use super::*;

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
    fn perform_ai_ask_records_proposal() {
        // 无 daemon：Notice 报 IAC-6101，不 panic
        let mut state = SidebarState::default();
        perform(&mut state, plan_action("??如何列出文件", false));
        assert!(!state.entries.is_empty());
    }
}
