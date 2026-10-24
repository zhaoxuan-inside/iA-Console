//! I-006：LLM 响应 → 候选列表（多候选 Q17）。剥代码围栏 → JSON 解析 → 字段校验，失败 IAC-1102。
//! risk 由编排层以 combine_risk（静态为主、AI 为辅冲突取更高，Q34）合成。

use super::{Candidate, ParamSpecDoc};
use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::ParamSpec;
use crate::ports::LlmResponse;
use crate::types::RiskLevel;
use serde_json::Value;

/// 解析产物：候选草稿（风险已按 Q34 合成）。
#[derive(Debug, Clone)]
pub struct ParsedCandidates {
    pub candidates: Vec<Candidate>,
    /// 原始响应（解析失败时供反馈留底）。
    pub raw: String,
}

/// I-006：解析提案响应（多候选；至少 1 个有效候选即成功）。
pub fn parse_proposal_response(
    input: &LlmResponse,
    output: &mut ParsedCandidates,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let json_text = extract_json(&input.text);
    let v: Value = match serde_json::from_str(&json_text) {
        Ok(v) => v,
        Err(e) => {
            evolution.fail(
                crate::error::codes::AI_PARSE,
                format!("提案 JSON 解析失败: {e}"),
            );
            return false;
        }
    };
    let empty = Vec::new();
    let arr = v["candidates"].as_array().unwrap_or(&empty);
    for c in arr {
        let command = c["command"].as_str().unwrap_or_default().trim().to_string();
        if command.is_empty() {
            continue;
        }
        output.candidates.push(Candidate {
            command: command.clone(),
            argv: shell_split(&command),
            summary: c["summary"].as_str().unwrap_or_default().to_string(),
            risk_level: combine_risk(
                crate::risk::assess_static(&command),
                parse_risk(c["risk"].as_str()),
            ),
            conventional: c["conventional"].as_bool().unwrap_or(false),
            explanation: ParamSpecDoc {
                params: parse_params(c),
                impact: c["impact"].as_str().unwrap_or_default().to_string(),
                summary: c["summary"].as_str().unwrap_or_default().to_string(),
            },
        });
    }
    if output.candidates.is_empty() {
        evolution.fail(crate::error::codes::AI_PARSE, "提案缺少有效 command 字段");
        return false;
    }
    metrics.counter("intent.candidates_parsed", output.candidates.len() as u64);
    true
}

/// AI 参数解释解析（handle_explain 兜底用）：(params, summary)。
pub fn parse_explain_response(resp: &LlmResponse) -> Option<(Vec<ParamSpec>, String)> {
    let v: Value = serde_json::from_str(&extract_json(&resp.text)).ok()?;
    let params = parse_params(&v);
    Some((
        params,
        v["summary"].as_str().unwrap_or_default().to_string(),
    ))
}

/// 风险合成（Q34：静态为主、AI 自评为辅、冲突取更高）。
fn combine_risk(static_level: RiskLevel, ai_level: Option<RiskLevel>) -> RiskLevel {
    crate::risk::combine_risk(static_level, ai_level)
}

/// AI 风险字段 → RiskLevel（非法值按 None 处理，静态判定兜底）。
fn parse_risk(s: Option<&str>) -> Option<RiskLevel> {
    match s? {
        "safe" => Some(RiskLevel::Safe),
        "caution" => Some(RiskLevel::Caution),
        "dangerous" => Some(RiskLevel::Dangerous),
        "destructive" => Some(RiskLevel::Destructive),
        _ => None,
    }
}

/// 剥离 markdown 代码围栏，截取首尾大括号之间的 JSON。
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();
    let fenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim());
    let body = fenced.unwrap_or(trimmed);
    match (body.find('{'), body.rfind('}')) {
        (Some(a), Some(b)) if b > a => body[a..=b].to_string(),
        _ => body.to_string(),
    }
}

/// params 数组 → ParamSpec（缺字段宽容处理）。
fn parse_params(v: &Value) -> Vec<ParamSpec> {
    v["params"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let flag = p["flag"].as_str()?.trim().to_string();
                    if flag.is_empty() {
                        return None;
                    }
                    Some(ParamSpec {
                        flag,
                        meaning: p["meaning"].as_str().unwrap_or_default().to_string(),
                        required: p["required"].as_bool().unwrap_or(false),
                        default: p["default"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 极简 shell 分词（引号感知；参数级语义由解释文档承载）。
pub fn shell_split(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(text: &str) -> LlmResponse {
        LlmResponse {
            text: text.into(),
            model: "test".into(),
        }
    }

    #[test]
    fn parses_multi_candidates_with_risk_conflict_higher() {
        let env = crate::framework::TestEnv::new();
        let mut out = ParsedCandidates {
            candidates: vec![],
            raw: String::new(),
        };
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        let text = r#"```json
        {"candidates":[
          {"command":"du -ah . | sort -rh | head -20","summary":"列最大的 20 个","conventional":true,"risk":"safe","params":[],"impact":"只读"},
          {"command":"rm -rf ./build","summary":"删构建目录","conventional":false,"risk":"safe","params":[],"impact":"删除"}
        ]}
        "#;
        assert!(
            parse_proposal_response(&resp(text), &mut out, &env.context(), &mut m, &mut e),
            "evo={e:?}"
        );
        assert_eq!(out.candidates.len(), 2);
        // 静态规则 rm -rf → dangerous；AI 报 safe → 冲突取更高（Q34）
        assert_eq!(out.candidates[1].risk_level, RiskLevel::Dangerous);
        assert!(out.candidates[0].conventional);
    }

    #[test]
    fn rejects_response_without_command() {
        let env = crate::framework::TestEnv::new();
        let mut out = ParsedCandidates {
            candidates: vec![],
            raw: String::new(),
        };
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(!parse_proposal_response(
            &resp("not json"),
            &mut out,
            &env.context(),
            &mut m,
            &mut e
        ));
        assert_eq!(e.reason_code.as_deref(), Some("IAC-1102"));
    }

    #[test]
    fn shell_split_respects_quotes() {
        assert_eq!(shell_split("echo \"a b\" c"), vec!["echo", "a b", "c"]);
    }
}
