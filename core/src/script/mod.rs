//! 脚本工坊（M8，v0.2.0）：任务段/行号区间 → 确定性拼接 sh/PS 脚本（AC-38）→
//! 命名注册（name 全局唯一）→ `iac run @名` 过同一闸门与审计（AC-39）→
//! AI 精修显式触发、diff 确认后覆盖（AC-40）。
//!
//! 编排（make/refine/run）+ 数据（collect）；拼接算法在 common::script_assembler。

pub mod collect;

pub use collect::{collect_commands, CollectRange};

use crate::common::script_assembler::{assemble_script, AssembledScript, ScriptAssemblyInput, ScriptTarget};
use crate::executor::{gate_decision, CommandOrigin, ExecutionOutcome, GateInfo};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::store::{insert_execution, record_audit};
use crate::history::{ExecStatus, ExecutionRecord};
use crate::intent::default_provider;
use crate::ports::{AiRequest, LlmResponse, ProcessResult, ProcessSpec, SqlQuery, SqlValue};
use crate::recipe::handler::{get_recipe_by_id, save_recipe};
use crate::recipe::RecipeInput;
use crate::risk::assess_static;
use crate::rollback::{post_diff, pre_capture};
use crate::session::store::{append_event, AppendEventInput, RecordAck};
use crate::session::EventKind;
use crate::types::{GateAction, RiskLevel};
use serde::{Deserialize, Serialize};

/// 脚本制作输入（API-013 make 动作）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptMakeInput {
    pub name: String,
    #[serde(default)]
    pub from_seq: Option<i64>,
    #[serde(default)]
    pub to_seq: Option<i64>,
    #[serde(default)]
    pub segment: Option<String>,
    #[serde(default)]
    pub target: ScriptTarget,
    #[serde(default)]
    pub refine: bool,
    #[serde(default)]
    pub description: Option<String>,
}

/// 脚本制作输出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptMakeOutput {
    pub recipe_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sh_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ps_body: Option<String>,
    pub max_risk: RiskLevel,
    pub refined: bool,
    pub command_count: u32,
}

impl Default for ScriptMakeOutput {
    fn default() -> Self {
        Self {
            recipe_id: String::new(),
            name: String::new(),
            sh_body: None,
            ps_body: None,
            max_risk: RiskLevel::Safe,
            refined: false,
            command_count: 0,
        }
    }
}

/// AI 精修输入（API-013 refine 动作）：confirm=false 产出修订稿（原稿不动）；confirm=true 写回。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptRefineInput {
    pub name: String,
    #[serde(default)]
    pub refined_body: Option<String>,
    #[serde(default)]
    pub confirm: bool,
}

/// AI 精修输出：refined_body + notes（参数化/预检/跨方言适配说明）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptRefineOutput {
    pub name: String,
    pub refined_body: String,
    pub notes: Vec<String>,
    pub applied: bool,
}

/// 命名脚本运行输入（API-014 / `iac run @名`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptRunInput {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub confirm_dangerous: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_word: Option<String>,
}

/// API-013 make：collect → assemble →（可选 AI 精修）→ 命名注册（kind=script，name 唯一）。
pub fn make_script(
    input: &ScriptMakeInput,
    output: &mut ScriptMakeOutput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.name.is_empty() {
        evolution.fail("IAC-1604", "脚本名不能为空");
        return false;
    }
    let mut entries = Vec::new();
    let range = CollectRange {
        segment_id: input.segment.clone(),
        from_seq: input.from_seq,
        to_seq: input.to_seq,
    };
    if !collect_commands(&range, &mut entries, context, metrics, evolution) {
        evolution.propagate("make_script", "命令收集失败");
        return false;
    }
    if entries.is_empty() {
        evolution.fail("IAC-1604", "指定范围内没有任何命令（用 iac history 查看行号）");
        return false;
    }
    let mut max_risk = RiskLevel::Safe;
    for e in &entries {
        max_risk = max_risk.max(assess_static(&e.command));
    }
    let mut assembled = AssembledScript::default();
    let asm_in = ScriptAssemblyInput { entries: entries.clone(), target: input.target, max_risk };
    if !assemble_script(&asm_in, &mut assembled, metrics) {
        evolution.propagate("make_script", "脚本装配失败");
        return false;
    }
    let mut refined = false;
    if input.refine {
        if let Some(body) = assembled.sh_body.clone() {
            if let Some(better) = ai_refine_body(&body, context, metrics, evolution) {
                assembled.sh_body = Some(better);
                refined = true;
            } else {
                evolution.ok = true; // AI 不可用：拼接稿仍可用（AC-40 降级）
            }
        }
    }
    let template = pack_template(&assembled);
    let mut recipe = crate::recipe::Recipe::default();
    let rin = RecipeInput {
        title: input.name.clone(),
        description: input.description.clone(),
        kind: "script".into(),
        name: Some(input.name.clone()),
        command_template: template,
        tags: vec!["script".into()],
        platform: "all".into(),
        source_execution_id: None,
    };
    if !save_recipe(&rin, &mut recipe, context, metrics, evolution) {
        evolution.propagate("make_script", "脚本注册失败（name 须全局唯一）");
        return false;
    }
    metrics.counter("script.made", 1);
    *output = ScriptMakeOutput {
        recipe_id: recipe.id,
        name: input.name.clone(),
        sh_body: assembled.sh_body,
        ps_body: assembled.ps_body,
        max_risk,
        refined,
        command_count: entries.len() as u32,
    };
    true
}

/// API-013 refine：confirm=false 产修订稿（原稿不动，AC-40）；confirm=true 写回。
pub fn refine_script(
    input: &ScriptRefineInput,
    output: &mut ScriptRefineOutput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut recipe = crate::recipe::Recipe::default();
    if !load_script_recipe(&input.name, &mut recipe, context, metrics, evolution) {
        return false;
    }
    if input.confirm {
        let Some(refined) = &input.refined_body else {
            evolution.fail("IAC-1604", "confirm=true 需要 refined_body");
            return false;
        };
        let rin = RecipeInput {
            title: recipe.title.clone(),
            description: recipe.description.clone(),
            kind: "script".into(),
            name: Some(input.name.clone()),
            command_template: refined.clone(),
            tags: recipe.tags.clone(),
            platform: recipe.platform.clone(),
            source_execution_id: recipe.source_execution_id.clone(),
        };
        let mut updated = crate::recipe::Recipe::default();
        if !save_recipe(&rin, &mut updated, context, metrics, evolution) {
            evolution.propagate("refine_script", "精修写回失败");
            return false;
        }
        *output = ScriptRefineOutput {
            name: input.name.clone(),
            refined_body: refined.clone(),
            notes: vec!["已确认覆盖原脚本".into()],
            applied: true,
        };
        return true;
    }
    let body = unpack_sh(&recipe.command_template);
    let notes = vec![
        "精修维度：参数化 / 依赖预检 / 跨方言适配".to_string(),
        "确认后经 action=refine + confirm=true + refined_body 写回；未确认前原脚本不变".to_string(),
    ];
    let Some(refined) = ai_refine_body(&body, context, metrics, evolution) else {
        evolution.fail("IAC-1101", "AI 后端不可达，无法精修；拼接稿仍可用");
        return false;
    };
    *output = ScriptRefineOutput { name: input.name.clone(), refined_body: refined, notes, applied: false };
    true
}

/// API-014 run（AC-39）：按名取脚本 → 平台选版本 → 闸门（内部最高风险）→ 执行 → 记录+审计。
pub fn run_script(
    input: &ScriptRunInput,
    output: &mut ExecutionOutcome,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut recipe = crate::recipe::Recipe::default();
    if !load_script_recipe(&input.name, &mut recipe, context, metrics, evolution) {
        return false;
    }
    let body = pick_platform_body(&recipe.command_template);
    let risk = assess_static(&body);
    let is_agent = matches!(context.session.actor, crate::framework::Actor::Agent(_));
    let action = gate_decision(risk, CommandOrigin::Direct, is_agent, input.confirm_dangerous);
    // 脚本闸门（AC-39 同一闸门）：destructive 仍要求确认词，dangerous 要求武装态标志
    let word_ok = input
        .confirm_word
        .as_deref()
        .map(|w| w.trim() == crate::executor::confirm_word_for(&body) || w.trim() == "确认")
        .unwrap_or(false);
    let blocked = matches!(action, GateAction::RejectAgentUnconfirmed)
        || (matches!(action, GateAction::RequireDoubleEnter) && !input.confirm_dangerous)
        || (matches!(action, GateAction::RequireConfirmWord) && !word_ok);
    if blocked {
        let expect = crate::executor::confirm_word_for(&body);
        evolution.fail(
            crate::error::codes::EXEC_REJECTED,
            format!("脚本内部含高危命令（{} 级），需要确认升级{}", risk_text(risk),
                if matches!(action, GateAction::RequireConfirmWord) { format!("：确认词「{expect}」") } else { String::new() }),
        );
        return false;
    }
    let gate = GateInfo {
        level: risk,
        action,
        origin: CommandOrigin::Direct,
        actor_tag: context.session.actor.as_tag(),
    };
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| ".".into());
    let mut manifest = pre_capture(&cwd, context, metrics, evolution);
    let timeout = context.config.exec.timeout_ms;
    let spec = ProcessSpec { command: body.clone(), cwd: None, env: vec![], timeout_ms: timeout };
    let mut result = ProcessResult::default();
    let spawn_ok = context.deps.process.spawn_process(&spec, &mut result, context, metrics, evolution);
    let mut record = build_script_record(input, risk, context);
    record.exit_code = result.exit_code;
    record.duration_ms = result.duration_ms;
    record.stdout_summary = Some(truncate(&result.stdout, context.config.exec.stdout_summary_max));
    record.stderr_summary = Some(truncate(&result.stderr, context.config.exec.stderr_summary_max));
    record.stdout_full = result.stdout.clone();
    record.stderr_full = result.stderr.clone();
    record.status = if spawn_ok && result.exit_code == Some(0) { ExecStatus::Ok } else { ExecStatus::Failed };
    record.journal_ref = manifest.as_ref().map(|m| m.manifest_ref.clone());
    persist_record(&record, context, metrics, evolution);
    if spawn_ok {
        if let Some(m) = manifest.as_mut() {
            post_diff(m, &record.id, context, metrics, evolution);
        }
    }
    if !context.session.session_id.is_empty() {
        let ev = AppendEventInput {
            session_id: context.session.session_id.clone(),
            kind: EventKind::Command,
            payload: serde_json::json!({"command": format!("@{}", input.name), "execution_id": record.id}),
            pinned: false,
        };
        let _ = append_event(&ev, &mut RecordAck::default(), context, metrics, evolution);
    }
    metrics.counter("script.ran", 1);
    let offer_help = record.status == ExecStatus::Failed;
    *output = ExecutionOutcome {
        execution_id: record.id,
        exit_code: record.exit_code,
        status: record.status,
        stdout_summary: record.stdout_summary,
        stderr_summary: record.stderr_summary,
        duration_ms: Some(record.duration_ms).filter(|d| *d > 0),
        diagnosis: None,
        gate,
        offer_help,
        effects: vec![],
    };
    true
}

/// 按名取 kind=script 的方案（IAC-1604=不存在）。
fn load_script_recipe(
    name: &str,
    output: &mut crate::recipe::Recipe,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT id FROM recipes WHERE kind='script' AND name=?".into(),
        params: vec![SqlValue::Text(name.to_string())],
    };
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("load_script_recipe", "方案查询失败");
        return false;
    }
    if rows.is_empty() {
        evolution.fail(crate::error::codes::RECIPE_NAME_CONFLICT, format!("脚本 @{name} 不存在（先 ia-script make 制作）"));
        return false;
    }
    let id = rows.text(0, "id").unwrap_or("").to_string();
    let mut opt = None;
    if !get_recipe_by_id(&id, &mut opt, context, metrics, evolution) {
        return false;
    }
    match opt {
        Some(r) => {
            *output = r;
            true
        }
        None => {
            evolution.fail("IAC-1604", "脚本方案不存在");
            false
        }
    }
}

/// 脚本模板双版本打包：sh 正文 + `# ---iac:ps---` 分隔 + PS 正文。
fn pack_template(assembled: &AssembledScript) -> String {
    let mut out = assembled.sh_body.clone().unwrap_or_default();
    if let Some(ps) = &assembled.ps_body {
        out.push_str("\n# ---iac:ps---\n");
        out.push_str(ps);
    }
    out
}

/// 解包 sh 正文。
fn unpack_sh(template: &str) -> String {
    template
        .split("\n# ---iac:ps---\n")
        .next()
        .unwrap_or(template)
        .to_string()
}

/// 按当前平台选正文（windows 取 PS 段，其余取 sh 段）。
fn pick_platform_body(template: &str) -> String {
    let mut parts = template.splitn(2, "\n# ---iac:ps---\n");
    let sh = parts.next().unwrap_or("").to_string();
    let ps = parts.next().map(str::to_string);
    if crate::types::Platform::detect() == crate::types::Platform::Windows {
        ps.unwrap_or(sh)
    } else {
        sh
    }
}

/// 脚本执行记录（@名 留痕，正文入 argv 首位供审计）。
fn build_script_record(input: &ScriptRunInput, risk: RiskLevel, context: &Context) -> ExecutionRecord {
    ExecutionRecord {
        id: String::new(),
        session_id: context.session.session_id.clone(),
        proposal_id: None,
        command: format!("@{}", input.name),
        argv: vec![format!("@{}", input.name)],
        cwd: std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| ".".into()),
        shell: "sh".into(),
        host: "local".into(),
        actor: context.session.actor.as_tag(),
        exit_code: None,
        status: ExecStatus::Ok,
        risk_level: risk_text(risk).into(),
        stdout_summary: None,
        stderr_summary: None,
        stdout_full: String::new(),
        stderr_full: String::new(),
        started_at: crate::session::now_ms(),
        duration_ms: 0,
        journal_ref: None,
    }
}

/// 持久化执行记录（审计由 record_execution 内部落账）。
fn persist_record(record: &ExecutionRecord, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) {
    if !insert_execution(record, context, metrics, evolution) {
        evolution.ok = true; // 记录失败不阻断执行结果返回
        return;
    }
    let _ = record_audit(
        &context.session.actor,
        "execute",
        &record.command,
        serde_json::json!({ "kind": "script", "status": record.status.as_str(), "exit_code": record.exit_code }),
        context,
        metrics,
        evolution,
    );
}

/// AI 精修订稿（显式触发才调用；失败返回 None 由调用方降级）。
fn ai_refine_body(
    body: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<String> {
    let provider = default_provider(context.config);
    let ai_in = AiRequest {
        system: "你是脚本工坊精修器。对给定 shell 脚本做：参数化（重复值提为变量）、依赖预检（command -v 检查）、跨方言适配提示。只输出修订后的完整脚本正文，不要解释文字。".into(),
        messages: vec![("user".into(), body.to_string())],
        temperature: 0.2,
        timeout_ms: 30_000,
    };
    let mut resp = LlmResponse::default();
    if !context.deps.ai.ai_complete(&provider, &ai_in, &mut resp, context, metrics, evolution) {
        return None;
    }
    let text = resp.text.trim().trim_matches('`').to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// 摘要截断（与 executor 同规则）。
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

/// 风险等级 → 存储文本。
fn risk_text(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::Safe => "safe",
        RiskLevel::Caution => "caution",
        RiskLevel::Dangerous => "dangerous",
        RiskLevel::Destructive => "destructive",
    }
}
