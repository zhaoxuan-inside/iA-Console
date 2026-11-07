//! 执行编排（I-027）：闸门矩阵（Q34~Q37）→ 确认升级校验 → 执行 → 三账留痕（Q45 全量输出）
//! → 失败兜底 offer_help（Q18）→ 回滚前像/效果日志（域 15）。
//! 业务结果（含非零退出/超时/拒绝）均为 true；仅编排内部故障返回 false。

use super::{
    confirm_word_for, gate_decision, CommandOrigin, ExecuteInput, ExecutionOutcome, GateInfo,
};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::store::{insert_execution, record_audit};
use crate::history::{ExecStatus, ExecutionRecord};
use crate::intent::{load_proposal, mark_proposal_status, ProposalStatus};
use crate::ports::{ProcessResult, ProcessSpec};
use crate::session::now_ms;
use crate::session::store::{append_event, record_env, AppendEventInput, RecordAck};
use crate::session::EventKind;
use crate::types::{GateAction, RiskLevel};

/// I-027：执行主流程。
pub fn handle_execute(
    input: &ExecuteInput,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let resolved = resolve_command(input, context, metrics, evolution);
    let Some((command, origin)) = resolved else {
        evolution.propagate("handle_execute", "解析待执行命令失败");
        return false;
    };
    // 环境变量声明（Q46）：记录入会话 env 与事件流，不作为进程执行；
    // 声明的变量经 session_env 注入后续 iac_execute/脚本执行的子进程环境
    if origin == CommandOrigin::Direct {
        if let Some((k, v)) = crate::common::marker_parser::parse_env_declaration(&command) {
            return record_env_only(input, &k, &v, output, context, metrics, evolution);
        }
    }
    // 风险：提案候选已含 AI 合成风险（Q34 冲突取更高），此处只对直输/翻译重评
    let (level, hits) = risk_level_for(input, &command, origin, context, metrics, evolution);
    let is_agent = matches!(context.session.actor, crate::framework::Actor::Agent(_));
    let action = gate_decision(level, origin, is_agent, input.confirmed);
    let gate = GateInfo {
        level,
        action,
        origin,
        actor_tag: context.session.actor.as_tag(),
        hits,
    };
    let mut record = build_record(input, &command, level, context);
    // 确认升级校验（Q21/Q37）
    if let Err(msg) = check_escalation(&gate, input, &command) {
        return reject_and_record(
            &mut record,
            &gate,
            output,
            &msg,
            context,
            metrics,
            evolution,
        );
    }
    run_and_record(
        command,
        input,
        &mut record,
        &gate,
        output,
        context,
        metrics,
        evolution,
    )
}

/// 确认升级校验（Q34~Q37 矩阵的执行侧）：
/// - RequireDoubleEnter：必须 confirmed=true（客户端武装态二次 Enter 后置位）；
/// - RequireConfirmWord：confirm_word 必须与 confirm_word_for(command) 语义匹配；
/// - RejectAgentUnconfirmed：拒绝并返回影响面说明（IAC-2105）。
fn check_escalation(gate: &GateInfo, input: &ExecuteInput, command: &str) -> Result<(), String> {
    match gate.action {
        GateAction::Pass | GateAction::WeakWarning => Ok(()),
        GateAction::RequireDoubleEnter if input.confirmed => Ok(()),
        GateAction::RequireDoubleEnter => Err("dangerous 提案需要武装态确认（二次 Enter）".into()),
        GateAction::RequireConfirmWord => {
            let expect = confirm_word_for(command);
            let got = input.confirm_word.as_deref().unwrap_or_default().trim();
            let ok = got == expect || got == expect.split_whitespace().next().unwrap_or_default();
            if ok {
                Ok(())
            } else {
                Err(format!("destructive 命令需要输入确认词「{expect}」"))
            }
        }
        GateAction::RejectAgentUnconfirmed => Err(format!(
            "Agent 未携带确认标志，已拒绝执行（影响面：{} 级命令）",
            risk_text(gate.level)
        )),
    }
}

/// 拒绝分支：rejected 留痕（业务结果 true，IAC-2105 供信封参考）。
fn reject_and_record(
    record: &mut ExecutionRecord,
    gate: &GateInfo,
    output: &mut ExecutionOutcome,
    msg: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    record.status = ExecStatus::Rejected;
    persist(record, context, metrics, evolution);
    *output = outcome(record, gate, Vec::new());
    record_audit(
        &context.session.actor,
        "reject",
        &record.command,
        serde_json::json!({ "level": risk_text(gate.level), "reason": msg }),
        context,
        metrics,
        evolution,
    );
    evolution.fail(crate::error::codes::EXEC_REJECTED, msg);
    evolution.ok = true; // 业务结果：rejected 属正常分支
    true
}

/// 执行分支：回滚前像 → 进程 → 三账 → 效果日志 → 失败 offer_help（Q18）。
#[allow(clippy::too_many_arguments)]
fn run_and_record(
    command: String,
    input: &ExecuteInput,
    record: &mut ExecutionRecord,
    gate: &GateInfo,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    // 回滚前像捕获（域 15，尽力而为）：闸门通过后、执行前
    let cwd = record.cwd.clone();
    let mut manifest = crate::rollback::pre_capture(&cwd, context, metrics, evolution);
    record.journal_ref = manifest.as_ref().map(|m| m.manifest_ref.clone());
    // 会话已声明环境变量注入执行（Q46）：export 累积对后续 iac_execute/脚本可见
    let env = crate::session::store::session_env(
        &context.session.session_id,
        context,
        metrics,
        evolution,
    );
    let spec = ProcessSpec {
        command,
        cwd: input.cwd.clone().map(std::path::PathBuf::from),
        env,
        timeout_ms: context.config.exec.timeout_ms,
    };
    let mut result = ProcessResult::default();
    if !context
        .deps
        .process
        .spawn_process(&spec, &mut result, context, metrics, evolution)
    {
        record.status =
            if evolution.reason_code.as_deref() == Some(crate::error::codes::EXEC_TIMEOUT) {
                ExecStatus::Timeout
            } else {
                ExecStatus::Failed
            };
        persist(record, context, metrics, evolution);
        *output = outcome(record, gate, Vec::new());
        evolution.ok = true;
        return true;
    }
    record.exit_code = result.exit_code;
    record.duration_ms = result.duration_ms;
    record.stdout_summary = Some(truncate(
        &result.stdout,
        context.config.exec.stdout_summary_max,
    ));
    record.stderr_summary = Some(truncate(
        &result.stderr,
        context.config.exec.stderr_summary_max,
    ));
    // 全量输出入库（Q45）
    record.stdout_full = result.stdout.clone();
    record.stderr_full = result.stderr.clone();
    record.status = if result.exit_code == Some(0) {
        ExecStatus::Ok
    } else {
        ExecStatus::Failed
    };
    persist(record, context, metrics, evolution);
    let effects = match manifest.as_mut() {
        Some(m) => crate::rollback::post_diff(m, &record.id, context, metrics, evolution),
        None => Vec::new(),
    };
    metrics.counter("executed", 1);
    let mut out = outcome(record, gate, effects);
    // 失败兜底（Q18）：就地「要不要 AI 帮忙？」——诊断由用户一键触发（iac_diagnose）
    out.offer_help = record.status == ExecStatus::Failed;
    *output = out;
    true
}

/// 环境变量声明分支：写会话 env 与事件流（Q46），返回成功 outcome。
fn record_env_only(
    input: &ExecuteInput,
    key: &str,
    value: &str,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let sid = context.session.session_id.clone();
    let ok = !sid.is_empty() && record_env(&sid, key, value, context, metrics, evolution);
    let mut out = ExecutionOutcome {
        execution_id: String::new(),
        exit_code: Some(0),
        status: ExecStatus::Ok,
        gate: GateInfo {
            level: RiskLevel::Safe,
            action: GateAction::Pass,
            origin: CommandOrigin::Direct,
            actor_tag: context.session.actor.as_tag(),
            hits: Vec::new(),
        },
        ..Default::default()
    };
    if !ok {
        // 会话不可用时仍返回成功（env 记录尽力而为），副作用由客户端 PTY 承担
        out.exit_code = Some(0);
    }
    let _ = input;
    *output = out;
    true
}

/// 解析待执行命令：直输命令或提案候选（多候选按 candidate_index，Q17）。
/// 返回 (命令, 来源)；提案路径同时把 candidate 的合成风险带入。
fn resolve_command(
    input: &ExecuteInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<(String, CommandOrigin)> {
    if let Some(cmd) = &input.command {
        return Some((cmd.clone(), input.origin.unwrap_or(CommandOrigin::Direct)));
    }
    let pid = input.proposal_id.as_ref()?;
    let proposal = load_proposal(pid, context, metrics, evolution)?;
    let idx = input
        .candidate_index
        .unwrap_or(proposal.active)
        .min(proposal.candidates.len().saturating_sub(1));
    let candidate = proposal.candidates.get(idx)?;
    Some((candidate.command.clone(), CommandOrigin::Proposal))
}

/// 风险等级：提案候选用生成期合成风险（Q34），直输/翻译用静态规则现评。
/// 返回 (等级, 命中规则 id)；提案合成风险无静态命中明细。
fn risk_level_for(
    input: &ExecuteInput,
    command: &str,
    origin: CommandOrigin,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> (RiskLevel, Vec<String>) {
    let _ = (metrics, evolution);
    if origin == CommandOrigin::Proposal {
        if let Some(pid) = &input.proposal_id {
            let mut m2 = Metrics::default();
            let mut e2 = Evolution::default();
            if let Some(p) = crate::intent::load_proposal(pid, context, &mut m2, &mut e2) {
                let idx = input
                    .candidate_index
                    .unwrap_or(p.active)
                    .min(p.candidates.len().saturating_sub(1));
                if let Some(c) = p.candidates.get(idx) {
                    return (c.risk_level, Vec::new());
                }
            }
        }
    }
    let detail = crate::risk::assess_detail(command);
    (detail.level, detail.hits)
}

/// 组装执行记录（cwd 优先取客户端声明，Q45 字段真实性）。
fn build_record(
    input: &ExecuteInput,
    command: &str,
    level: RiskLevel,
    context: &Context,
) -> ExecutionRecord {
    ExecutionRecord {
        id: String::new(),
        session_id: context.session.session_id.clone(),
        proposal_id: input.proposal_id.clone(),
        command: command.to_string(),
        // 账本 argv 对齐 sh -c 真实参数边界（引号感知；不闭合退空白切分）
        argv: crate::common::shell_split::argv_of(command),
        cwd: input.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into())
        }),
        shell: "sh".into(),
        host: "local".into(),
        actor: context.session.actor.as_tag(),
        exit_code: None,
        status: ExecStatus::Ok,
        risk_level: risk_text(level).into(),
        stdout_summary: None,
        stderr_summary: None,
        stdout_full: String::new(),
        stderr_full: String::new(),
        started_at: now_ms(),
        duration_ms: 0,
        journal_ref: None,
    }
}

/// 落三账（AC-05）：执行记录（全量输出）+ 审计 + 会话事件；审计/事件失败不阻断。
fn persist(
    record: &mut ExecutionRecord,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) {
    if !insert_execution(record, context, metrics, evolution) {
        evolution.propagate("persist", "执行记录入库失败");
        return;
    }
    let _ = record_audit(
        &context.session.actor,
        "execute",
        &record.command,
        serde_json::json!({ "status": record.status.as_str(), "exit_code": record.exit_code, "risk": record.risk_level }),
        context,
        metrics,
        evolution,
    );
    if !record.session_id.is_empty() {
        let event = AppendEventInput {
            session_id: record.session_id.clone(),
            kind: EventKind::Command,
            payload: serde_json::json!({
                "execution_id": record.id,
                "command": record.command,
                "status": record.status.as_str(),
                "cwd": record.cwd,
            }),
            pinned: false,
        };
        let _ = append_event(
            &event,
            &mut RecordAck::default(),
            context,
            metrics,
            evolution,
        );
    }
    if record.proposal_id.is_some() && record.status == ExecStatus::Ok {
        // 提案确认执行成功 → executed（尽力而为）
        let mut m2 = Metrics::default();
        let mut e2 = Evolution::default();
        let _ = mark_proposal_status(
            record.proposal_id.as_deref().unwrap_or_default(),
            ProposalStatus::Executed,
            context,
            &mut m2,
            &mut e2,
        );
    }
}

/// 组装输出（执行记录 ID 回填；effects=本次执行落账的效果条目）。
fn outcome(
    record: &ExecutionRecord,
    gate: &GateInfo,
    effects: Vec<crate::types::EffectEntry>,
) -> ExecutionOutcome {
    ExecutionOutcome {
        execution_id: record.id.clone(),
        exit_code: record.exit_code,
        status: record.status,
        stdout_summary: record.stdout_summary.clone(),
        stderr_summary: record.stderr_summary.clone(),
        duration_ms: Some(record.duration_ms).filter(|d| *d > 0),
        diagnosis: None,
        gate: gate.clone(),
        offer_help: record.status == ExecStatus::Failed,
        effects,
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
    format!("{}…[截断，全量已入库]", &s[..end])
}

fn risk_text(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::Safe => "safe",
        RiskLevel::Caution => "caution",
        RiskLevel::Dangerous => "dangerous",
        RiskLevel::Destructive => "destructive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalation_requires_confirmation_artifacts() {
        let mut input = ExecuteInput {
            confirmed: false,
            ..Default::default()
        };
        let gate = GateInfo {
            action: GateAction::RequireDoubleEnter,
            ..Default::default()
        };
        assert!(check_escalation(&gate, &input, "git push --force").is_err());
        input.confirmed = true;
        assert!(check_escalation(&gate, &input, "git push --force").is_ok());
        // destructive 确认词
        let gate_d = GateInfo {
            action: GateAction::RequireConfirmWord,
            ..Default::default()
        };
        assert!(check_escalation(&gate_d, &input, "mkfs.ext4 /dev/sdb1").is_err());
        input.confirm_word = Some(confirm_word_for("mkfs.ext4 /dev/sdb1"));
        assert!(check_escalation(&gate_d, &input, "mkfs.ext4 /dev/sdb1").is_ok());
        // agent 未确认拒绝
        let gate_a = GateInfo {
            action: GateAction::RejectAgentUnconfirmed,
            ..Default::default()
        };
        assert!(check_escalation(&gate_a, &input, "rm -rf /").is_err());
    }
}
