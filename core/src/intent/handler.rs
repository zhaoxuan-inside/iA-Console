//! 意图编排（I-025 handle_ask / I-026 handle_explain / I-007 build_explain_doc）。
//! 主流程：会话 → 知识 grounding → AI 生成（解析失败重试 1 次）→ 未注册参数拒绝 → 风险评估 → 落库 + 事件。

use super::{AskInput, ExplainInput, ParamSpecDoc, Proposal, ProposalDraft, ProposalStatus};
use crate::config::ProviderProfile;
use crate::framework::{Context, Evolution, Metrics};
use crate::intent::parse::{parse_explain_response, parse_proposal_response};
use crate::intent::prompt::{build_intent_prompt, IntentPromptInput, PromptBundle};
use crate::intent::store::{get_proposal, save_proposal, set_proposal_status};
use crate::knowledge::{cache_knowledge, lookup_knowledge, CommandRef, KnowledgeAck};
use crate::ports::{AiRequest, LlmResponse, ProcessSpec};
use crate::risk::{assess_risk, RiskAssessment, RiskInput};
use crate::session::{
    append_session_event, get_or_create_current, CurrentSessionInput, SessionInfo,
};
use crate::types::Platform;

/// MVP 主机标识（跨平台 hostname 无 std 解；v1.1 引 hostname 能力）。
const LOCAL_HOST: &str = "local";

/// I-025：意图 → 提案（AC-01/11/14/20）。
pub fn handle_ask(
    input: &AskInput,
    output: &mut Proposal,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if !validate_intent(input, evolution) {
        return false;
    }
    let platform = input.platform.unwrap_or_else(Platform::detect);
    let mut session = empty_session();
    if !ensure_ask_session(input, &mut session, context, metrics, evolution) {
        return false; // evolution 已写明
    }
    let knowledge = lookup_for_ask(input, platform, context, metrics, evolution);
    let Some(draft) = generate_checked_draft(input, platform, knowledge.as_ref(), context, metrics, evolution) else {
        evolution.propagate("handle_ask", "生成提案失败");
        return false;
    };
    let risk_level = match assess_and_level(&draft, input, context, metrics, evolution) {
        Some(l) => l,
        None => {
            evolution.propagate("handle_ask", "风险评估失败");
            return false;
        }
    };
    let mut proposal = assemble_proposal(input, &session.id, &draft, risk_level, knowledge.is_some(), platform);
    persist_proposal(&mut proposal, &session.id, context, metrics, evolution);
    *output = proposal;
    true
}

/// 提案落库+会话事件（handle_ask 子步骤；事件失败不阻断）。
fn persist_proposal(proposal: &mut Proposal, session_id: &str, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) {
    if !save_proposal(proposal, context, metrics, evolution) {
        evolution.propagate("handle_ask", "保存提案失败");
        return;
    }
    append_proposal_event(session_id, proposal, context, metrics, evolution);
    metrics.counter("ask.completed", 1);
}

/// 风险评估到等级（handle_ask 子步骤）。
fn assess_and_level(
    draft: &ProposalDraft,
    input: &AskInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<crate::types::RiskLevel> {
    let mut risk = RiskAssessment::default();
    let risk_in = RiskInput { command: draft.command.clone(), cwd: input.cwd.clone() };
    assess_risk(&risk_in, &mut risk, context, metrics, evolution).then_some(risk.level)
}

/// 意图校验（IAC-1103）。
fn validate_intent(input: &AskInput, evolution: &mut Evolution) -> bool {
    let ok = !input.intent.trim().is_empty() && input.intent.chars().count() <= 2000;
    if !ok {
        evolution.fail("IAC-1103", "意图为空或超过 2000 字符");
    }
    ok
}

/// 会话就绪（handle_ask 子步骤）：取/建当前会话。
fn ensure_ask_session(
    input: &AskInput,
    session: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let sess_in = CurrentSessionInput {
        cwd: input.cwd.clone().unwrap_or_else(|| ".".into()),
        shell: "sh".into(),
        host: LOCAL_HOST.into(),
    };
    if !get_or_create_current(&sess_in, session, context, metrics, evolution) {
        evolution.propagate("handle_ask", "取当前会话失败");
        return false;
    }
    true
}

/// 生成并校验草稿（handle_ask 子步骤）：AI 生成 + grounding 参数拒绝。
fn generate_checked_draft(
    input: &AskInput,
    platform: Platform,
    knowledge: Option<&crate::knowledge::CommandKnowledge>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ProposalDraft> {
    let provider = default_provider(context.config);
    let draft = generate_draft(input, platform, knowledge, &provider, context, metrics, evolution)?;
    if let Some(k) = knowledge {
        if violates_known_params(&draft, k) {
            evolution.fail("IAC-1102", "提案包含知识库未注册的参数（grounding 拒绝）");
            return None;
        }
    }
    Some(draft)
}


/// 提案组装（handle_ask 子步骤）。
fn assemble_proposal(
    input: &AskInput,
    session_id: &str,
    draft: &ProposalDraft,
    risk_level: crate::types::RiskLevel,
    verified: bool,
    platform: Platform,
) -> Proposal {
    Proposal {
        id: String::new(),
        session_id: session_id.to_string(),
        intent_text: input.intent.clone(),
        command: draft.command.clone(),
        argv: draft.argv.clone(),
        explanation: draft.explanation.clone(),
        risk_level,
        platform,
        verified,
        status: ProposalStatus::Proposed,
        created_at: 0,
    }
}

/// I-026：参数解释（AC-02）：知识命中 → 文档；未命中 → `--help` 解析 → AI 补全，均回写缓存。
pub fn handle_explain(
    input: &ExplainInput,
    output: &mut ParamSpecDoc,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let platform = input.platform.unwrap_or_else(Platform::detect);
    let cref = CommandRef { command: input.command.clone(), platform, source_hash: None };
    let mut cached = None;
    if !lookup_knowledge(&cref, &mut cached, context, metrics, evolution) {
        evolution.propagate("handle_explain", "查询知识失败");
        return false;
    }
    if let Some(k) = cached {
        return build_explain_doc(&k, output, context, metrics, evolution);
    }
    let Some(knowledge) = knowledge_from_help(input, platform, context, metrics, evolution) else {
        return ai_explain_fallback(input, platform, context, metrics, evolution, output);
    };
    let mut ack = crate::knowledge::KnowledgeAck::default();
    if !cache_knowledge(&knowledge, &mut ack, context, metrics, evolution) {
        evolution.propagate("handle_explain", "写缓存失败");
        return false;
    }
    build_explain_doc(&knowledge, output, context, metrics, evolution)
}

/// `--help` 抓取与解析（handle_explain 子步骤）：解析失败返回 None（转 AI 兜底）。
fn knowledge_from_help(
    input: &ExplainInput,
    platform: Platform,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<crate::knowledge::CommandKnowledge> {
    let spec = ProcessSpec {
        command: format!("{} --help", input.command),
        cwd: None,
        env: vec![],
        timeout_ms: 5_000,
    };
    let mut result = crate::ports::ProcessResult::default();
    if !context.deps.process.spawn_process(&spec, &mut result, context, metrics, evolution)
        || result.exit_code != Some(0)
    {
        evolution.fail("IAC-1104", format!("命令 {} 无法解释（--help 不可得）", input.command));
        return None;
    }
    let help = crate::common::help_parser::HelpText {
        command: input.command.clone(),
        platform,
        text: result.stdout.clone(),
    };
    let mut knowledge = crate::knowledge::CommandKnowledge {
        command: String::new(),
        platform,
        subcommands: vec![],
        params: vec![],
        source_hash: String::new(),
        verified: false,
    };
    crate::common::help_parser::parse_help_text(&help, &mut knowledge, context, metrics, evolution).then_some(knowledge)
}

/// I-007：命令知识 → 参数文档（data）。
pub fn build_explain_doc(
    input: &crate::knowledge::CommandKnowledge,
    output: &mut ParamSpecDoc,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    *output = ParamSpecDoc {
        params: input.params.clone(),
        impact: String::new(),
        summary: format!("{}：{} 个参数，{} 个子命令", input.command, input.params.len(), input.subcommands.len()),
    };
    metrics.counter("explain.built", 1);
    true
}

/// 生成提案草稿：prompt → AI → 解析（失败自动重试 1 次，AC-01/IAC-1102 约定）。
fn generate_draft(
    input: &AskInput,
    platform: Platform,
    knowledge: Option<&crate::knowledge::CommandKnowledge>,
    provider: &ProviderProfile,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ProposalDraft> {
    for attempt in 0..2 {
        let mut bundle = PromptBundle::default();
        let pin = IntentPromptInput {
            intent: input.intent.clone(),
            cwd: input.cwd.clone().unwrap_or_else(|| ".".into()),
            platform,
            knowledge: knowledge.cloned(),
            recent_commands: vec![],
        };
        if !build_intent_prompt(&pin, &mut bundle, context, metrics, evolution) {
            evolution.propagate("generate_draft", "组装 prompt 失败");
            return None;
        }
        let ai_in = AiRequest {
            system: bundle.system.clone(),
            messages: vec![("user".into(), bundle.user.clone())],
            temperature: 0.2,
            timeout_ms: 30_000,
        };
        let mut resp = LlmResponse::default();
        if !context.deps.ai.ai_complete(provider, &ai_in, &mut resp, context, metrics, evolution) {
            evolution.propagate("generate_draft", "AI 调用失败");
            return None;
        }
        let mut draft = ProposalDraft::default();
        if parse_proposal_response(&resp, &mut draft, context, metrics, evolution) {
            return Some(draft);
        }
        if attempt == 1 {
            evolution.propagate("generate_draft", "提案解析失败（已重试 1 次）");
            metrics.counter("intent.parse_giveup", 1);
            return None;
        }
        metrics.counter("intent.parse_retry", 1);
    }
    None
}

/// 解析失败的 AI 补全兜底：产出未验证知识并回写缓存（AC-02）。
fn ai_explain_fallback(
    input: &ExplainInput,
    platform: Platform,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    output: &mut ParamSpecDoc,
) -> bool {
    let provider = default_provider(context.config);
    let ai_in = AiRequest {
        system: "你是命令行专家。解释命令参数，只输出 JSON：{params:[{flag,meaning,required,default}],summary}".into(),
        messages: vec![("user".into(), format!("解释命令：{}", input.command))],
        temperature: 0.2,
        timeout_ms: 30_000,
    };
    let mut resp = LlmResponse::default();
    if !context.deps.ai.ai_complete(&provider, &ai_in, &mut resp, context, metrics, evolution) {
        evolution.propagate("ai_explain_fallback", "AI 补全失败");
        return false;
    }
    let Some((params, summary)) = parse_explain_response(&resp) else {
        evolution.fail("IAC-1104", "AI 参数解释解析失败");
        return false;
    };
    let knowledge = crate::knowledge::CommandKnowledge {
        command: input.command.clone(),
        platform,
        subcommands: vec![],
        params: params.clone(),
        source_hash: crate::common::hash::fnv1a64_hex(&format!("ai:{},{}", input.command, resp.text.len())),
        verified: false,
    };
    let mut ack = KnowledgeAck::default();
    if !cache_knowledge(&knowledge, &mut ack, context, metrics, evolution) {
        evolution.propagate("ai_explain_fallback", "写缓存失败");
        return false;
    }
    *output = ParamSpecDoc { params, impact: String::new(), summary };
    true
}

/// 意图首词作为命令候选查知识（查不到不影响主流程）。
fn lookup_for_ask(
    input: &AskInput,
    platform: Platform,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<crate::knowledge::CommandKnowledge> {
    let first = input.intent.split_whitespace().next().unwrap_or_default();
    if first.is_empty() {
        return None;
    }
    let cref = CommandRef { command: first.to_string(), platform, source_hash: None };
    let mut found = None;
    let _ = lookup_knowledge(&cref, &mut found, context, metrics, evolution);
    found
}

/// grounding 校验：知识命中时，提案参数必须为已注册参数子集（AC-01 可信来源）。
fn violates_known_params(draft: &ProposalDraft, knowledge: &crate::knowledge::CommandKnowledge) -> bool {
    if knowledge.params.is_empty() {
        return false;
    }
    draft
        .explanation
        .params
        .iter()
        .any(|p| !knowledge.params.iter().any(|k| k.flag == p.flag))
}

/// 默认 Provider 档案（local-builtin 为隐式档案，引擎端点随 T-027 接线）。
pub fn default_provider(config: &crate::config::Config) -> ProviderProfile {
    let name = &config.ai.default_provider;
    config
        .providers
        .iter()
        .find(|p| &p.name == name)
        .cloned()
        .unwrap_or(ProviderProfile {
            name: "local-builtin".into(),
            kind: crate::types::ProviderKind::LocalPlugin,
            base_url: Some("http://127.0.0.1:18080".into()),
            api_key_env: None,
            model: Some("local".into()),
            path: None,
        })
}

/// 提案落会话事件（失败不阻断主流程，审计兜底）。
fn append_proposal_event(session_id: &str, proposal: &Proposal, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) {
    let event = crate::session::SessionEventInput {
        session_id: session_id.into(),
        kind: crate::session::EventKind::Proposal,
        payload: serde_json::json!({ "proposal_id": proposal.id, "command": proposal.command, "risk": proposal.risk_level }),
    };
    let mut ack = crate::session::RecordAck::default();
    let _ = append_session_event(&event, &mut ack, context, metrics, evolution);
}

fn empty_session() -> SessionInfo {
    SessionInfo {
        id: String::new(),
        title: String::new(),
        cwd: String::new(),
        shell: String::new(),
        host: String::new(),
        status: String::new(),
        created_at: 0,
        updated_at: 0,
        events: vec![],
    }
}

/// 提案状态流转（executor 确认后调用；暴露给 executor 模块复用）。
pub fn mark_proposal_status(
    proposal_id: &str,
    status: ProposalStatus,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    set_proposal_status(&crate::intent::ProposalId { id: proposal_id.into() }, status, context, metrics, evolution)
}

/// 按 ID 取提案（executor 使用）。
pub fn load_proposal(
    proposal_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<Proposal> {
    let mut out = None;
    get_proposal(&crate::intent::ProposalId { id: proposal_id.into() }, &mut out, context, metrics, evolution);
    out
}
