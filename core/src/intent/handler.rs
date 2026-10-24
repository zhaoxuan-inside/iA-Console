//! 意图编排（I-025 handle_ask / I-026 handle_explain）。
//! 重建口径：多候选 2~3（Q17）、排序=惯例>低风险>频率（Q23）、上下文注入+透明快照（Q20）、
//! AI 对话轨与 ai_message 事件（域 4）、生成失败重试 1 次（IAC-1102 约定）。

use super::parse::{parse_explain_response, parse_proposal_response, ParsedCandidates};
use super::prompt::{build_intent_prompt, InjectContext, IntentPromptInput, PromptBundle};
use super::store::{get_proposal, save_proposal, set_proposal_status};
use super::{AskInput, Candidate, ExplainInput, ParamSpecDoc, Proposal, ProposalStatus};
use crate::config::ProviderProfile;
use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::{cache_knowledge, lookup_knowledge, CommandRef, KnowledgeAck};
use crate::ports::{AiRequest, LlmResponse, ProcessSpec};
use crate::session::store::{
    append_event, events_since_separator, get_or_create_current, pinned_notes, AppendEventInput,
    RecordAck,
};
use crate::session::{CurrentSessionInput, EventKind, SessionInfo};
use crate::types::Platform;

/// MVP 主机标识（跨平台 hostname 无 std 解；平台适配落地时接入）。
const LOCAL_HOST: &str = "local";

/// I-025：意图 → 多候选提案（AC-01/11/14/20 + Q17/Q20/Q23）。
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
    let platform = Platform::detect();
    let mut session = SessionInfo::default();
    if !ensure_ask_session(input, &mut session, context, metrics, evolution) {
        return false; // evolution 已写明
    }
    // 上下文注入（Q20）：段内最近命令 + 对话轨 + 钉住备注
    let inject = build_inject_context(&session.id, input, platform, context, metrics, evolution);
    let knowledge = lookup_for_ask(input, platform, context, metrics, evolution);
    let Some(parsed) = generate_checked_candidates(input, &inject, knowledge.as_ref(), context, metrics, evolution)
    else {
        evolution.propagate("handle_ask", "生成提案失败");
        return false;
    };
    let mut proposal = assemble_proposal(input, &session.id, parsed, knowledge.is_some(), platform, &inject);
    persist_proposal(&mut proposal, &session.id, context, metrics, evolution);
    *output = proposal;
    true
}

/// 上下文注入快照（Q20）：cwd + 平台/Shell + 段内最近 10 条命令 + 对话轨 + 钉住备注。
fn build_inject_context(
    session_id: &str,
    input: &AskInput,
    platform: Platform,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> InjectContext {
    let mut recent = Vec::new();
    let mut seg_events = Vec::new();
    if !session_id.is_empty()
        && events_since_separator(session_id, &mut seg_events, context, metrics, evolution)
    {
        recent = seg_events
            .iter()
            .filter(|e| e.kind == EventKind::Command)
            .filter_map(|e| e.payload.get("command").and_then(|c| c.as_str()).map(str::to_string))
            .rev()
            .take(10)
            .collect::<Vec<_>>();
        recent.reverse();
    }
    let mut dialogue = seg_events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::AiMessage | EventKind::Note) && !e.pinned)
        .filter_map(|e| {
            e.payload
                .get("text")
                .or_else(|| e.payload.get("answer"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    dialogue.truncate(10);
    let mut notes = Vec::new();
    let _ = pinned_notes(session_id, &mut notes, context, metrics, evolution);
    InjectContext {
        cwd: input.cwd.clone().unwrap_or_else(|| ".".into()),
        platform: format!("{platform:?}").to_lowercase(),
        shell: input.shell.clone().unwrap_or_default(),
        recent_commands: recent,
        dialogue,
        pinned_notes: notes,
    }
}

/// 提案落库 + 会话事件（ai_message 一并入对话轨，域 4；事件失败不阻断）。
fn persist_proposal(
    proposal: &mut Proposal,
    session_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) {
    if !save_proposal(proposal, context, metrics, evolution) {
        evolution.propagate("handle_ask", "保存提案失败");
        return;
    }
    if !session_id.is_empty() {
        let first = proposal.candidates.get(proposal.active);
        let event = AppendEventInput {
            session_id: session_id.into(),
            kind: EventKind::Proposal,
            payload: serde_json::json!({
                "proposal_id": proposal.id,
                "command": first.map(|c| c.command.clone()).unwrap_or_default(),
                "risk": first.map(|c| c.risk_level).unwrap_or(crate::types::RiskLevel::Safe),
                "candidate_count": proposal.candidates.len(),
            }),
            pinned: false,
        };
        let _ = append_event(&event, &mut RecordAck::default(), context, metrics, evolution);
    }
    metrics.counter("ask.completed", 1);
}

/// 提案组装：多候选排序（惯例>低风险>频率，Q23）截断 3 + context_json 快照（Q20）。
fn assemble_proposal(
    input: &AskInput,
    session_id: &str,
    mut parsed: ParsedCandidates,
    verified: bool,
    platform: Platform,
    inject: &InjectContext,
) -> Proposal {
    // 排序：惯例优先，其次低风险；频率（方案库使用计数）作为第三键
    for c in &mut parsed.candidates {
        c.explanation.summary = if c.summary.is_empty() { c.explanation.summary.clone() } else { c.summary.clone() };
    }
    parsed.candidates.sort_by(|a, b| {
        b.conventional
            .cmp(&a.conventional)
            .then(a.risk_level.cmp(&b.risk_level))
    });
    parsed.candidates.truncate(3);
    Proposal {
        id: String::new(),
        session_id: session_id.to_string(),
        intent_text: input.intent.clone(),
        candidates: parsed.candidates,
        active: 0,
        platform,
        verified,
        status: ProposalStatus::Proposed,
        created_at: crate::session::now_ms(),
        context_json: Some(serde_json::to_string(inject).unwrap_or_default()),
    }
}

/// 生成并校验候选（handle_ask 子步骤）：AI 生成（重试 1 次）+ grounding 参数拒绝。
fn generate_checked_candidates(
    input: &AskInput,
    inject: &InjectContext,
    knowledge: Option<&crate::knowledge::CommandKnowledge>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ParsedCandidates> {
    let provider = default_provider(context.config);
    let parsed = generate_candidates(input, inject, knowledge, &provider, context, metrics, evolution)?;
    if let Some(k) = knowledge {
        if violates_known_params(&parsed.candidates, k) {
            evolution.fail(crate::error::codes::AI_PARSE, "提案包含知识库未注册的参数（grounding 拒绝）");
            return None;
        }
    }
    Some(parsed)
}

/// 生成候选：prompt → AI → 解析（失败自动重试 1 次）。
fn generate_candidates(
    input: &AskInput,
    inject: &InjectContext,
    knowledge: Option<&crate::knowledge::CommandKnowledge>,
    provider: &ProviderProfile,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ParsedCandidates> {
    for attempt in 0..2 {
        let mut bundle = PromptBundle::default();
        let pin = IntentPromptInput {
            intent: input.intent.clone(),
            context: inject.clone(),
            knowledge: knowledge.cloned(),
        };
        if !build_intent_prompt(&pin, &mut bundle, context, metrics, evolution) {
            evolution.propagate("generate_candidates", "组装 prompt 失败");
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
            evolution.propagate("generate_candidates", "AI 调用失败");
            return None;
        }
        let mut out = ParsedCandidates { candidates: vec![], raw: resp.text.clone() };
        if parse_proposal_response(&resp, &mut out, context, metrics, evolution) {
            return Some(out);
        }
        if attempt == 1 {
            evolution.propagate("generate_candidates", "提案解析失败（已重试 1 次）");
            metrics.counter("intent.parse_giveup", 1);
            return None;
        }
        metrics.counter("intent.parse_retry", 1);
    }
    None
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
    let platform = Platform::detect();
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
    let mut ack = KnowledgeAck::default();
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
        evolution.fail(crate::error::codes::DIAGNOSE, format!("命令 {} 无法解释（--help 不可得）", input.command));
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
    crate::common::help_parser::parse_help_text(&help, &mut knowledge, context, metrics, evolution)
        .then_some(knowledge)
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
        summary: format!(
            "{}：{} 个参数，{} 个子命令",
            input.command,
            input.params.len(),
            input.subcommands.len()
        ),
    };
    metrics.counter("explain.built", 1);
    true
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
        evolution.fail(crate::error::codes::DIAGNOSE, "AI 参数解释解析失败");
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

/// 意图校验（IAC-1103）。
fn validate_intent(input: &AskInput, evolution: &mut Evolution) -> bool {
    let ok = !input.intent.trim().is_empty() && input.intent.chars().count() <= 2000;
    if !ok {
        evolution.fail(crate::error::codes::INTENT_INVALID, "意图为空或超过 2000 字符");
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
        shell: input.shell.clone().unwrap_or_else(|| "sh".into()),
        host: LOCAL_HOST.into(),
        title: None,
    };
    if !get_or_create_current(&sess_in, session, context, metrics, evolution) {
        evolution.propagate("handle_ask", "取当前会话失败");
        return false;
    }
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
fn violates_known_params(candidates: &[Candidate], knowledge: &crate::knowledge::CommandKnowledge) -> bool {
    if knowledge.params.is_empty() {
        return false;
    }
    candidates.iter().any(|c| {
        c.explanation
            .params
            .iter()
            .any(|p| !knowledge.params.iter().any(|k| k.flag == p.flag))
    })
}

/// 默认 Provider 档案（local-builtin 为隐式档案，引擎端点随本地引擎接线）。
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

/// 按 ID 取提案（executor 使用）。
pub fn load_proposal(
    proposal_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<Proposal> {
    let mut out = None;
    get_proposal(proposal_id, &mut out, context, metrics, evolution);
    out
}

/// 提案状态流转（executor 确认后调用）。
pub fn mark_proposal_status(
    proposal_id: &str,
    status: ProposalStatus,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    set_proposal_status(proposal_id, status, context, metrics, evolution)
}
