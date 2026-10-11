//! 诊断编排与数据方法（I-029/I-021/I-022）。
//! 机制（_02 Q2）：通用错误码走静态快速通道（离线毫秒级），其余走 LLM 证据链诊断。

use super::{DiagnoseInput, Diagnosis, FixProposal};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::{get_execution_detail, query_history, ExecutionId, ExecutionRecord, HistoryFilter};
use crate::ports::LlmResponse;

/// 静态快速通道：exit code → 结构化提示（_02 Q2-S1）。
fn static_hint(exit_code: i32) -> Option<(&'static str, &'static str)> {
    match exit_code {
        127 => Some(("命令未找到", "命令不存在或不在 PATH 中；安装命令或检查 PATH（回滚日志见 v2）")),
        126 => Some(("命令不可执行", "找到命令但无执行权限；检查 chmod 与文件类型")),
        13 => Some(("权限被拒绝", "当前用户无权访问目标文件/目录")),
        _ => None,
    }
}

/// 诊断 prompt 组装产物。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagnosePromptBundle {
    pub system: String,
    pub user: String,
    /// 静态通道命中时直接给出根因，跳过 LLM。
    pub static_root_cause: Option<String>,
}

/// I-021：失败上下文构建（data）：stderr+退出码+cwd+近期历史 → prompt；静态命中先行。
pub fn build_failure_context(
    input: &ExecutionRecord,
    output: &mut DiagnosePromptBundle,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    output.static_root_cause = input.exit_code.and_then(static_hint).map(|(cause, note)| format!("{cause}：{note}"));
    output.system = "你是 iA-console 错误医生。分析失败命令，只输出 JSON：{root_cause, evidence:[stderr 原文片段], confidence:0~1, fix_proposals:[{command,explanation}]}。evidence 必须逐字引用 stderr，引用不到则输出空数组。".into();
    let stderr = input.stderr_summary.as_deref().unwrap_or("(无 stderr)");
    output.user = format!(
        "命令：{}\n退出码：{}\n目录：{}\n平台 shell：{}\nstderr：\n{}\n",
        input.command, input.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "无".into()), input.cwd, input.shell, stderr
    );
    metrics.counter("doctor.context_built", 1);
    true
}

/// I-022：LLM 响应 → Diagnosis（data）。证据为空 → speculation=true（幻觉约束）。
pub fn parse_diagnosis_response(
    input: &LlmResponse,
    output: &mut Diagnosis,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let text = strip_fences(&input.text);
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            evolution.fail("IAC-1201", format!("诊断 JSON 解析失败: {e}"));
            return false;
        }
    };
    let evidence: Vec<String> = v["evidence"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let fixes = v["fix_proposals"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(FixProposal {
                        command: f["command"].as_str()?.to_string(),
                        explanation: f["explanation"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    *output = Diagnosis {
        root_cause: v["root_cause"].as_str().unwrap_or("未知根因").to_string(),
        confidence: v["confidence"].as_f64().unwrap_or(0.5) as f32,
        evidence,
        fix_proposals: fixes,
        rollback_note: "当前版本无自动回滚能力；高危操作请依赖回滚日志（v2）".into(),
        speculation: false,
    };
    if output.evidence.is_empty() {
        output.speculation = true;
    }
    metrics.counter("doctor.parsed", 1);
    true
}

/// I-029：诊断编排（AC-03）：静态命中即返回；否则 LLM 证据链；AI 不可用降级静态并标注。
pub fn diagnose_failure(
    input: &DiagnoseInput,
    output: &mut Diagnosis,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let record = match locate_failure(input, context, metrics, evolution) {
        Some(r) => r,
        None => {
            evolution.propagate("diagnose_failure", "未找到失败执行记录");
            return false;
        }
    };
    let mut bundle = DiagnosePromptBundle::default();
    if !build_failure_context(&record, &mut bundle, context, metrics, evolution) {
        evolution.propagate("diagnose_failure", "构建诊断上下文失败");
        return false;
    }
    if let Some(cause) = bundle.static_root_cause.clone() {
        *output = static_diagnosis(&record, &cause);
        metrics.counter("doctor.static_hit", 1);
        return true;
    }
    let provider = crate::intent::handler::default_provider(context.config);
    let ai_in = crate::ports::AiRequest {
        system: bundle.system.clone(),
        messages: vec![("user".into(), bundle.user.clone())],
        temperature: 0.2,
        timeout_ms: 30_000,
    };
    let mut resp = LlmResponse::default();
    if !context.deps.ai.ai_complete(&provider, &ai_in, &mut resp, context, metrics, evolution) {
        evolution.propagate("diagnose_failure", "AI 诊断失败（静态通道未命中）");
        return false;
    }
    if !parse_diagnosis_response(&resp, output, context, metrics, evolution) {
        evolution.propagate("diagnose_failure", "解析诊断失败");
        return false;
    }
    output.rollback_note = "当前版本无自动回滚能力；高危操作请依赖回滚日志（v2）".into();
    metrics.counter("doctor.ai_diagnosed", 1);
    true
}

/// 定位失败记录：显式 ID 或最近一次 failed。
fn locate_failure(
    input: &DiagnoseInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ExecutionRecord> {
    match &input.execution_id {
        Some(id) => {
            let mut out = None;
            if !get_execution_detail(&ExecutionId { id: id.clone() }, &mut out, context, metrics, evolution) {
                return None;
            }
            out
        }
        None => {
            let filter = HistoryFilter { status: Some(crate::history::ExecStatus::Failed), limit: 1, ..Default::default() };
            let mut page = crate::history::HistoryPage::default();
            if !query_history(&filter, &mut page, context, metrics, evolution) {
                return None;
            }
            page.items.into_iter().next()
        }
    }
}

/// 静态通道诊断构造（引用 stderr 尾行作证据）。
fn static_diagnosis(record: &ExecutionRecord, cause: &str) -> Diagnosis {
    let evidence = record
        .stderr_summary
        .as_deref()
        .map(|s| s.lines().rev().take(3).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    Diagnosis {
        root_cause: cause.to_string(),
        confidence: 0.9,
        evidence,
        fix_proposals: Vec::new(),
        rollback_note: "当前版本无自动回滚能力；高危操作请依赖回滚日志（v2）".into(),
        speculation: record.stderr_summary.is_none(),
    }
}

fn strip_fences(text: &str) -> &str {
    let t = text.trim();
    let body = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t).trim();
    match (body.find('{'), body.rfind('}')) {
        (Some(a), Some(b)) if b > a => &body[a..=b],
        _ => body,
    }
}
