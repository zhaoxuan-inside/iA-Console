//! I-006：LLM 响应 → 提案草稿（data）。剥代码围栏 → JSON 解析 → 字段校验，失败 IAC-1102。

use super::ProposalDraft;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::LlmResponse;
use crate::knowledge::ParamSpec;
use serde_json::Value;

/// I-006：解析提案响应。输出草稿（命令/argv/解释）；risk 由编排层以 assess_risk 覆盖。
pub fn parse_proposal_response(
    input: &LlmResponse,
    output: &mut ProposalDraft,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let json_text = extract_json(&input.text);
    let v: Value = match serde_json::from_str(&json_text) {
        Ok(v) => v,
        Err(e) => {
            evolution.fail("IAC-1102", format!("提案 JSON 解析失败: {e}"));
            return false;
        }
    };
    let command = v["command"].as_str().unwrap_or_default().trim().to_string();
    if command.is_empty() {
        evolution.fail("IAC-1102", "提案缺少 command 字段");
        return false;
    }
    output.command = command;
    output.argv = shell_split(&output.command);
    output.explanation = super::ParamSpecDoc {
        params: parse_params(&v),
        impact: v["impact"].as_str().unwrap_or_default().to_string(),
        summary: v["summary"].as_str().unwrap_or_default().to_string(),
    };
    metrics.counter("intent.proposal_parsed", 1);
    true
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

/// 轻量分词（提案 argv 视图；引号段保持完整，执行仍走 shell 解释）。
fn shell_split(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    for ch in command.chars() {
        match in_quote {
            Some(q) if ch == q => in_quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => in_quote = Some(ch),
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

/// explain AI 兜底响应解析：params + summary。
pub fn parse_explain_response(resp: &LlmResponse) -> Option<(Vec<ParamSpec>, String)> {
    let v: Value = serde_json::from_str(&extract_json(&resp.text)).ok()?;
    let params = parse_params(&v);
    Some((params, v["summary"].as_str().unwrap_or_default().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ProposalDraft, Evolution> {
        let env = crate::framework::TestEnv::new();
        let resp = LlmResponse { text: text.into(), model: "test".into() };
        let mut out = ProposalDraft::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        parse_proposal_response(&resp, &mut out, &env.context(), &mut m, &mut e).then_some(out).ok_or(e)
    }

    const GOOD: &str = r#"{"command":"find /var/log -type f -mtime -7 | sort -rn | head -10","summary":"找出最近7天的10个最大文件","params":[{"flag":"-mtime -7","meaning":"最近7天修改"}],"impact":"只读操作","risk":"safe"}"#;

    #[test]
    fn parses_good_proposal() {
        let d = parse(GOOD).unwrap();
        assert_eq!(d.command, "find /var/log -type f -mtime -7 | sort -rn | head -10");
        assert_eq!(d.argv.first().unwrap(), "find");
        assert_eq!(d.explanation.params.len(), 1);
        assert_eq!(d.explanation.impact, "只读操作");
    }

    #[test]
    fn parses_fenced_markdown() {
        let fenced = format!("```json\n{GOOD}\n```");
        assert!(parse(&fenced).is_ok());
    }

    #[test]
    fn empty_command_fails_1102() {
        let e = parse(r#"{"command":"  ","summary":"x"}"#).unwrap_err();
        assert_eq!(e.reason_code.as_deref(), Some("IAC-1102"));
    }

    #[test]
    fn broken_json_fails_1102() {
        let e = parse("这不是 JSON").unwrap_err();
        assert_eq!(e.reason_code.as_deref(), Some("IAC-1102"));
    }
}
