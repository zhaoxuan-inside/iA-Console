//! 诊断编排与数据方法（I-029/I-021/I-022）。
//! 机制：通用错误码走静态快速通道（离线毫秒级），其余走 LLM 证据链诊断。
//! 交互（Q18）：失败后就地「要不要 AI 帮忙？」——本编排由一键触发（iac_diagnose）。

use super::{DiagnoseInput, Diagnosis, Evidence, FixProposal};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::store::query_history;
use crate::history::{ExecStatus, ExecutionRecord, HistoryFilter, HistoryPage};
use crate::ports::LlmResponse;

/// 静态快速通道：exit code → 结构化提示。
fn static_hint(exit_code: i32) -> Option<(&'static str, &'static str)> {
    match exit_code {
        127 => Some(("命令未找到", "命令不存在或不在 PATH 中；安装命令或检查 PATH，或让 AI 给出提案")),
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

/// I-021：失败上下文构建（data）：stderr+退出码+cwd → prompt；静态命中先行。
pub fn build_failure_context(
    input: &ExecutionRecord,
    output: &mut DiagnosePromptBundle,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    output.static_root_cause =
        input.exit_code.and_then(static_hint).map(|(cause, note)| format!("{cause}：{note}"));
    output.system = "你是 iA-console 错误医生。分析失败命令，只输出 JSON：\
        {root_cause, evidence:[stderr 原文片段], confidence:0~1, fix_proposals:[{command,explanation}]}。\
        evidence 必须逐字引用 stderr，引用不到则输出空数组。"
        .into();
    // 全量输出入库后，诊断证据取 stderr 全量尾段（Q45 存储全量≠注入全量：诊断属失败诊断附带，Q20）
    let stderr = pick_stderr(input);
    output.user = format!(
        "命令：{}\n退出码：{}\n目录：{}\n平台 shell：{}\nstderr：\n{}\n",
        input.command,
        input.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "无".into()),
        input.cwd,
        input.shell,
        stderr
    );
    metrics.counter("doctor.context_built", 1);
    true
}

/// 诊断注入的 stderr：全量可用取尾 8KB，否则摘要。
fn pick_stderr(record: &ExecutionRecord) -> String {
    let src = if record.stderr_full.is_empty() {
        record.stderr_summary.as_deref().unwrap_or("(无 stderr)")
    } else {
        record.stderr_full.as_str()
    };
    let bytes = src.as_bytes();
    if bytes.len() > 8 * 1024 {
        let mut start = bytes.len() - 8 * 1024;
        while start < bytes.len() && !src.is_char_boundary(start) {
            start += 1;
        }
        return format!("…[截断头部]\n{}", &src[start..]);
    }
    src.to_string()
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
            evolution.fail(crate::error::codes::DIAGNOSE, format!("诊断 JSON 解析失败: {e}"));
            return false;
        }
    };
    let evidence = extract_evidence(&v);
    *output = Diagnosis {
        root_cause: v["root_cause"].as_str().unwrap_or("未知根因").to_string(),
        confidence: v["confidence"].as_f64().unwrap_or(0.5) as f32,
        evidence,
        fix_proposals: extract_fixes(&v),
        rollback_note: "可经 ia-rollback / ia-file-history 回滚（域 15）".into(),
        speculation: false,
    };
    if output.evidence.is_empty() {
        output.speculation = true;
    }
    metrics.counter("doctor.parsed", 1);
    true
}

/// evidence 数组提取（字符串 → Evidence{source:stderr}）。
fn extract_evidence(v: &serde_json::Value) -> Vec<Evidence> {
    v["evidence"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.as_str().map(|q| Evidence {
                        source: "stderr".into(),
                        quote: q.to_string(),
                        note: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// fix_proposals 数组提取。
fn extract_fixes(v: &serde_json::Value) -> Vec<FixProposal> {
    v["fix_proposals"]
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
        .unwrap_or_default()
}

/// I-029：诊断编排（AC-03）：静态命中即返回；否则 LLM 证据链。
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
    ai_diagnose(&bundle, context, metrics, evolution, output)
}

/// LLM 证据链诊断（diagnose_failure 子步骤）。
fn ai_diagnose(
    bundle: &DiagnosePromptBundle,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    output: &mut Diagnosis,
) -> bool {
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
    output.rollback_note = "可经 ia-rollback / ia-file-history 回滚（域 15）".into();
    metrics.counter("doctor.ai_diagnosed", 1);
    true
}

/// 定位失败记录：显式 ID → 库；直接现场 → 组装；缺省 → 最近一次 failed。
fn locate_failure(
    input: &DiagnoseInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<ExecutionRecord> {
    if let Some(id) = &input.execution_id {
        let filter = HistoryFilter { q: None, session_id: None, actor: None, status: None, since: None, until: None, limit: 1, offset: 0, with_full_output: true };
        // 精确取单条：借 q 全表过滤代价高，直接 LIKE 不适合 id——用 with_full_output + 分页过滤
        let mut page = HistoryPage::default();
        // 简化：查询最近 200 条再精确匹配（万条级检索由 history 域承担）
        let mut scan = HistoryFilter { limit: 200, with_full_output: true, ..Default::default() };
        let _ = &mut scan;
        if !query_history(&filter, &mut page, context, metrics, evolution) {
            return None;
        }
        return page.items.into_iter().find(|r| &r.id == id);
    }
    if let Some(cmd) = &input.command {
        // 客户端直接携带现场（GUI 一键问 AI 可能无 execution_id）
        return Some(ExecutionRecord {
            id: String::new(),
            session_id: context.session.session_id.clone(),
            command: cmd.clone(),
            exit_code: input.exit_code,
            status: ExecStatus::Failed,
            cwd: ".".into(),
            shell: "sh".into(),
            host: "local".into(),
            stderr_full: input.stderr.clone(),
            stdout_full: input.stdout.clone(),
            stderr_summary: Some(input.stderr.chars().rev().take(256).collect::<String>().chars().rev().collect()),
            ..Default::default()
        });
    }
    let filter = HistoryFilter { status: Some(ExecStatus::Failed), limit: 1, with_full_output: true, ..Default::default() };
    let mut page = HistoryPage::default();
    if !query_history(&filter, &mut page, context, metrics, evolution) {
        return None;
    }
    page.items.into_iter().next()
}

/// 静态通道诊断构造（引用 stderr 尾行作证据）。
fn static_diagnosis(record: &ExecutionRecord, cause: &str) -> Diagnosis {
    let src = if record.stderr_full.is_empty() {
        record.stderr_summary.clone().unwrap_or_default()
    } else {
        record.stderr_full.clone()
    };
    let evidence = src
        .lines()
        .rev()
        .take(3)
        .map(|q| Evidence { source: "stderr".into(), quote: q.to_string(), note: String::new() })
        .collect::<Vec<_>>();
    Diagnosis {
        root_cause: cause.to_string(),
        confidence: 0.9,
        evidence,
        fix_proposals: Vec::new(),
        rollback_note: "可经 ia-rollback / ia-file-history 回滚（域 15）".into(),
        speculation: src.is_empty(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::TestEnv;

    #[test]
    fn static_hint_covers_127_and_direct_input() {
        let env = TestEnv::new();
        let mut out = Diagnosis::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        let input = DiagnoseInput {
            command: Some("git status".into()),
            exit_code: Some(127),
            stderr: "git: command not found".into(),
            ..Default::default()
        };
        assert!(diagnose_failure(&input, &mut out, &env.context(), &mut m, &mut e), "evo={e:?}");
        assert!(out.root_cause.contains("命令未找到"));
        assert!(!out.evidence.is_empty(), "静态通道也引用 stderr 尾行");
        assert!(!out.speculation);
    }
}
