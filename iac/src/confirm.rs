//! 提案确认流（AC-09 + Q35/Q37）：console 与 chat 共用的确认状态机。
//! 结构化驱动：只读 daemon 回包（result.status / result.gate.action），
//! 不从渲染文本反解析（旧 extract_field 因渲染串无生产者而整体失效的教训）。
//!
//! 语义（与 core gate_decision/check_escalation 矩阵对齐）：
//! - yes 首次发送 confirmed=false → safe 直接执行；
//! - dangerous 被闸门打回（require_double_enter）→ 本地武装，二次 yes 发 confirmed=true；
//! - destructive 被打回（require_confirm_word）→ 保留 pending 等待确认词输入，
//!   词以 confirmed=false + confirm_word 透传，由服务端 check_escalation 校验
//!   （「确认 <尾词>」不匹配即拒绝——客户端不代放行）。

use serde_json::{json, Value};

/// 待确认提案（id + 候选命令表 + 活动位 + 客户端武装态）。
pub struct PendingProposal {
    pub id: String,
    pub candidates: Vec<String>,
    pub active: usize,
    pub armed: bool,
    /// destructive 打回后的等待确认词态（pending 保持至词校验终态）。
    pub awaiting_word: bool,
}

impl PendingProposal {
    /// 活动候选命令原文。
    pub fn command(&self) -> &str {
        self.candidates
            .get(self.active)
            .map(String::as_str)
            .unwrap_or_default()
    }
}

/// 确认流状态（console/chat 各持一份）。
#[derive(Default)]
pub struct ProposalFlow {
    pub pending: Option<PendingProposal>,
}

/// 处理产出（Out=输出块，Tip=系统提示行）。
pub enum FlowLine {
    Out(String),
    Tip(String),
}

/// 回包里的闸门动作（snake_case serde，core types::GateAction）。
fn gate_action(result: &Value) -> Option<&str> {
    result.get("gate")?.get("action")?.as_str()
}

fn status_of(result: &Value) -> &str {
    result.get("status").and_then(Value::as_str).unwrap_or("")
}

impl ProposalFlow {
    /// 是否有待确认提案（调用方据此拦截 yes/no/确认词输入）。
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 从 iac_ask 结构化回包登记待确认提案；返回界面提示（无 id 时 None）。
    pub fn register(&mut self, result: &Value) -> Option<String> {
        let id = result.get("id")?.as_str()?.to_string();
        let active = result.get("active").and_then(Value::as_u64).unwrap_or(0) as usize;
        let candidates = result
            .get("candidates")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|c| {
                        c.get("command")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.pending = Some(PendingProposal {
            id: id.clone(),
            candidates,
            active,
            armed: false,
            awaiting_word: false,
        });
        Some(format!("提案 {id} → 输入 yes 执行，no 拒绝"))
    }

    /// 切换活动候选（delta=±1）；返回切换后的命令原文。
    pub fn cycle_candidate(&mut self, delta: i32) -> Option<String> {
        let p = self.pending.as_mut()?;
        let n = p.candidates.len();
        if n == 0 {
            return None;
        }
        let cur = p.active as i32;
        p.active = (cur + delta).rem_euclid(n as i32) as usize;
        Some(p.command().to_string())
    }

    /// yes/no/确认词 处理；None=非确认输入（调用方继续常规分发）。
    pub fn handle(&mut self, line: &str) -> Option<Vec<FlowLine>> {
        let line = line.trim();
        if !self.has_pending() {
            return None;
        }
        if line == "no" {
            let p = self.pending.take();
            return Some(vec![FlowLine::Tip(format!("提案 {} 已拒绝", p?.id))]);
        }
        let is_yes = line == "yes";
        let is_word = line.starts_with("确认");
        if !is_yes && !is_word {
            return None;
        }
        let (pid, armed) = {
            let p = self.pending.as_ref()?;
            (p.id.clone(), p.armed)
        };
        let mut params = json!({
            "proposal_id": pid,
            "origin": "proposal",
            "cwd": crate::ipc::client_cwd(),
            // 武装态二次 yes 置位（Q35）；确认词路径恒 false——词由服务端
            // check_escalation 校验（RequireConfirmWord 分支），客户端不代放行
            "confirmed": armed && !is_word,
            "candidate_index": self.pending.as_ref().map(|p| p.active),
        });
        if is_word {
            params["confirm_word"] = json!(line);
        }
        let resp = match crate::ipc::request("iac_execute", params, None) {
            Ok(r) => r,
            Err(e) => return Some(vec![FlowLine::Tip(e)]),
        };
        let result = resp.result.clone().unwrap_or_default();
        if status_of(&result) == "Rejected" {
            match gate_action(&result) {
                Some("require_double_enter") if is_yes => {
                    if let Some(p) = self.pending.as_mut() {
                        p.armed = true;
                    }
                    return Some(vec![
                        FlowLine::Out(crate::ipc::render(&resp, false)),
                        FlowLine::Tip("dangerous 提案：再次输入 yes 执行（二次确认）".into()),
                    ]);
                }
                Some("require_confirm_word") => {
                    let tail = self
                        .pending
                        .as_ref()
                        .map(|p| {
                            p.command()
                                .split_whitespace()
                                .last()
                                .unwrap_or_default()
                                .to_string()
                        })
                        .unwrap_or_default();
                    // pending 保留并置等待词态：下一行「确认 xx」才有送达对象
                    // （此前此处清空 pending，确认词落入常规分发——确认流断链）
                    if let Some(p) = self.pending.as_mut() {
                        p.awaiting_word = true;
                    }
                    return Some(vec![
                        FlowLine::Out(crate::ipc::render(&resp, false)),
                        FlowLine::Tip(format!(
                            "destructive 提案：输入「确认 {tail}」执行（尾词=命令最后一个词）"
                        )),
                    ]);
                }
                _ => {}
            }
        }
        // 执行完成（或终态拒绝）：清待确认
        self.pending = None;
        Some(vec![FlowLine::Out(crate::ipc::render(&resp, false))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_result() -> Value {
        json!({
            "id": "p-1",
            "active": 1,
            "candidates": [
                {"command": "ls -a", "risk_level": "safe"},
                {"command": "rm -rf build", "risk_level": "dangerous"},
            ],
            "status": "proposed",
        })
    }

    #[test]
    fn register_reads_structured_proposal() {
        let mut flow = ProposalFlow::default();
        let notice = flow.register(&ask_result()).expect("有 id");
        assert!(notice.contains("p-1"));
        let p = flow.pending.as_ref().expect("已登记");
        assert_eq!(p.command(), "rm -rf build", "活动位=1 的候选");
        assert!(!p.armed);
        assert!(!p.awaiting_word, "登记初值不在等待词态");
        assert!(flow.has_pending());
    }

    #[test]
    fn register_without_id_is_noop() {
        let mut flow = ProposalFlow::default();
        assert!(flow.register(&json!({"candidates": []})).is_none());
        assert!(!flow.has_pending());
    }

    #[test]
    fn no_cancels_pending() {
        let mut flow = ProposalFlow::default();
        flow.register(&ask_result());
        let lines = flow.handle("no").expect("no 被确认流拦截");
        assert!(matches!(lines[0], FlowLine::Tip(_)));
        assert!(!flow.has_pending());
    }

    #[test]
    fn non_confirm_lines_pass_through_when_pending() {
        let mut flow = ProposalFlow::default();
        flow.register(&ask_result());
        assert!(flow.handle("ls").is_none(), "普通输入不拦截");
        assert!(flow.handle("").is_none(), "空输入不拦截");
        // 「确认」前缀/yes 触发 IPC 路径，由端到端覆盖；此处只验证普通行穿透
    }

    #[test]
    fn cycle_candidate_wraps_around() {
        let mut flow = ProposalFlow::default();
        flow.register(&ask_result());
        assert_eq!(flow.cycle_candidate(1).as_deref(), Some("ls -a")); // 1→2 wrap→0
        assert_eq!(flow.cycle_candidate(-1).as_deref(), Some("rm -rf build"));
    }

    #[test]
    fn gate_action_reads_structured_path() {
        let v = json!({"gate": {"action": "require_confirm_word"}});
        assert_eq!(gate_action(&v), Some("require_confirm_word"));
        assert_eq!(gate_action(&json!({"status": "Ok"})), None);
        assert_eq!(status_of(&json!({"status": "Rejected"})), "Rejected");
    }
}
