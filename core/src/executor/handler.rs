//! 执行编排（I-027/I-028）：闸门 → 执行 → 记录+审计+事件 → 失败转诊断（AC-03/04/05）。

use super::{CommandOrigin, ExecuteInput, ExecutionOutcome, GateInfo};
use crate::doctor::{diagnose_failure, DiagnoseInput};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::{
    append_audit_entry, get_execution_detail, record_execution, ExecStatus, ExecutionId,
    ExecutionRecord,
};
use crate::intent::{load_proposal, mark_proposal_status, ProposalStatus};
use crate::risk::{assess_risk, RiskAssessment, RiskInput};
use crate::ports::{ExecuteAck, ProcessSpec, ProcessResult, SqlQuery};
use crate::session::{append_session_event, now_ms, EventKind, SessionEventInput};

/// I-027：执行主流程。业务结果（含非零退出/超时/拒绝）均为 true；仅编排内部故障返回 false。
pub fn handle_execute(
    input: &ExecuteInput,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(command) = resolve_command(input, context, metrics, evolution) else {
        evolution.propagate("handle_execute", "解析待执行命令失败");
        return false;
    };
    let mut risk = RiskAssessment::default();
    let risk_in = RiskInput { command: command.clone(), cwd: None };
    if !assess_risk(&risk_in, &mut risk, context, metrics, evolution) {
        evolution.propagate("handle_execute", "风险评估失败");
        return false;
    }
    let gate = gate_decision(&risk, input, context);
    let mut record = build_record(input, &command, &risk, context);
    if gate.requires_confirm && !input.confirm_dangerous {
        record.status = ExecStatus::Rejected;
        record.id = persist(&record, context, metrics, evolution).unwrap_or_default();
        *output = outcome(&record, &gate, None);
        evolution.fail("IAC-2105", format!("高危命令需确认：{}", risk.impact));
        evolution.ok = true; // 业务结果：rejected 属正常分支，错误码供信封层参考
        return true;
    }
    let timeout = input.timeout_ms.unwrap_or(context.config.exec.timeout_ms);
    let spec = ProcessSpec { command, cwd: None, env: vec![], timeout_ms: timeout };
    let mut result = ProcessResult::default();
    if !context.deps.process.spawn_process(&spec, &mut result, context, metrics, evolution) {
        record.status = if evolution.reason_code.as_deref() == Some("IAC-2103") { ExecStatus::Timeout } else { ExecStatus::Failed };
        record.id = persist(&record, context, metrics, evolution).unwrap_or_default();
        *output = outcome(&record, &gate, None);
        evolution.ok = true;
        return true;
    }
    record.exit_code = result.exit_code;
    record.duration_ms = Some(result.duration_ms);
    record.stdout_summary = Some(truncate(&result.stdout, context.config.exec.stdout_summary_max));
    record.stderr_summary = Some(truncate(&result.stderr, context.config.exec.stderr_summary_max.max(context.config.exec.stdout_summary_max)));
    record.status = if result.exit_code == Some(0) { ExecStatus::Ok } else { ExecStatus::Failed };
    record.id = persist(&record, context, metrics, evolution).unwrap_or_default();
    metrics.counter("executed", 1);
    let diagnosis = if record.status == ExecStatus::Failed {
        run_diagnosis(&record, context, metrics, evolution)
    } else {
        None
    };
    *output = outcome(&record, &gate, diagnosis);
    true
}

/// I-028：确认后执行——置提案 confirmed 并按 Proposal 来源执行。
pub fn confirm_and_execute(
    input: &ExecuteInput,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if let Some(pid) = &input.proposal_id {
        if !mark_proposal_status(pid, ProposalStatus::Confirmed, context, metrics, evolution) {
            evolution.propagate("confirm_and_execute", "更新提案状态失败");
            return false;
        }
    }
    let mut confirmed = ExecuteInput { confirm_dangerous: true, origin: Some(CommandOrigin::Proposal), ..input.clone() };
    confirmed.confirm_dangerous = true;
    handle_execute(&confirmed, output, context, metrics, evolution)
}

/// 解析待执行命令：直输命令或提案命令。
fn resolve_command(
    input: &ExecuteInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<String> {
    if let Some(cmd) = &input.command {
        return Some(cmd.clone());
    }
    let pid = input.proposal_id.as_ref()?;
    let proposal = load_proposal(pid, context, metrics, evolution)?;
    Some(proposal.command)
}

/// 闸门决策（AC-04/35）：人类直输弱提醒不拦截；翻译高危与 Agent 恒确认。
fn gate_decision(risk: &RiskAssessment, input: &ExecuteInput, context: &Context) -> GateInfo {
    let is_agent = matches!(context.session.actor, crate::framework::Actor::Agent(_));
    let origin = input.origin.unwrap_or(CommandOrigin::Direct);
    let needs = risk.level >= crate::types::RiskLevel::Dangerous
        && (origin == CommandOrigin::Translated || (is_agent && !input.confirm_dangerous));
    GateInfo {
        level: risk.level,
        requires_confirm: needs,
        weak_warning: risk.level >= crate::types::RiskLevel::Caution && !is_agent,
        conversion: None,
    }
}

/// 组装执行记录（含会话归属与 actor）。
fn build_record(input: &ExecuteInput, command: &str, risk: &RiskAssessment, context: &Context) -> ExecutionRecord {
    ExecutionRecord {
        id: String::new(),
        session_id: Some(input.session_id.clone().unwrap_or_else(|| context.session.session_id.clone())),
        proposal_id: input.proposal_id.clone(),
        command: command.to_string(),
        argv: command.split_whitespace().map(str::to_string).collect(),
        cwd: ".".into(),
        shell: "sh".into(),
        host: "local".into(),
        actor: context.session.actor.as_tag(),
        exit_code: None,
        status: ExecStatus::Ok,
        risk_level: risk.level,
        stdout_summary: None,
        stderr_summary: None,
        stdout_ref: None,
        stderr_ref: None,
        started_at: now_ms(),
        duration_ms: None,
        journal_ref: None,
    }
}

/// 落三账：执行记录 + 审计 + 会话事件（AC-05）；返回执行 ID。审计/事件失败不阻断。
fn persist(record: &ExecutionRecord, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Option<String> {
    let mut ack = crate::session::RecordAck::default();
    if record_execution(record, &mut ack, context, metrics, evolution) {
        let execution_id = ack.id.clone();
        let entry = crate::history::AuditEntry {
            session_id: record.session_id.clone(),
            actor: record.actor.clone(),
            action: "execute".into(),
            target: Some(record.command.clone()),
            detail: Some(serde_json::json!({"status": record.status, "exit_code": record.exit_code})),
        };
        let mut a_ack = crate::session::RecordAck::default();
        let _ = append_audit_entry(&entry, &mut a_ack, context, metrics, evolution);
        let event = SessionEventInput {
            session_id: record.session_id.clone().unwrap_or_default(),
            kind: EventKind::Command,
            payload: serde_json::json!({"execution_id": ack.id, "command": record.command, "status": record.status}),
        };
        let mut e_ack = crate::session::RecordAck::default();
        let _ = append_session_event(&event, &mut e_ack, context, metrics, evolution);
        return Some(execution_id);
    }
    None
}

/// 失败诊断（AC-03；诊断失败不阻断执行结果返回）。
fn run_diagnosis(record: &ExecutionRecord, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Option<crate::doctor::Diagnosis> {
    let input = DiagnoseInput { execution_id: Some(record.id.clone()) };
    let mut d = crate::doctor::Diagnosis {
        root_cause: String::new(),
        evidence: vec![],
        confidence: 0.0,
        fix_proposals: vec![],
        rollback_note: String::new(),
        speculation: false,
    };
    diagnose_failure(&input, &mut d, context, metrics, evolution).then_some(d)
}

/// 组装输出。执行记录 ID 回填。
fn outcome(record: &ExecutionRecord, gate: &GateInfo, diagnosis: Option<crate::doctor::Diagnosis>) -> ExecutionOutcome {
    ExecutionOutcome {
        execution_id: record.id.clone(),
        exit_code: record.exit_code,
        status: record.status,
        stdout_summary: record.stdout_summary.clone(),
        stderr_summary: record.stderr_summary.clone(),
        duration_ms: record.duration_ms,
        diagnosis,
        gate: gate.clone(),
    }
}

/// 字节上限截断（char 边界安全）。
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…[截断]", &s[..end])
}

/// 提案执行记录引用辅助（record_execution 内使用；保留以扩展 proposal_id 关联）。
#[allow(dead_code)]
fn touch_proposal_status(proposal_id: &str, status: ProposalStatus, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool {
    mark_proposal_status(proposal_id, status, context, metrics, evolution)
}

/// 执行记录查询辅助（供 outcome 引用；T-023 CLI 直接走 dispatch）。
#[allow(dead_code)]
fn load_execution(id: &str, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Option<ExecutionRecord> {
    let mut out = None;
    let _ = get_execution_detail(&ExecutionId { id: id.into() }, &mut out, context, metrics, evolution);
    out
}

/// SQL 执行辅助（预留给提案状态落库复用）。
#[allow(dead_code)]
fn exec_sql(sql: &str, params: Vec<crate::ports::SqlValue>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool {
    let mut ack = ExecuteAck::default();
    context.deps.store.storage_execute(&SqlQuery { sql: sql.into(), params }, &mut ack, context, metrics, evolution)
}
