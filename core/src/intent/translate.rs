//! I-033：跨方言翻译编排（AC-19/20/23/24）。链路：内置表（零 AI）→ 缓存 → AI 翻译/组合 → 回写。

use super::translation_table;
use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::translation::{
    ParamMapKind, ParamMapping, TranslationInput, TranslationKind, TranslationResult, TranslationSource,
};
use crate::knowledge::{cache_translation, lookup_translation, CacheTranslationInput};
use crate::intent::handler::default_provider;
use crate::ports::AiRequest;
use crate::recipe::{save_recipe, RecipeInput};
use crate::risk::{assess_risk, RiskInput};
use crate::types::Platform;

/// I-033：翻译编排。幂等只读（不执行）；仅存储故障返回 false。
pub fn translate_command(
    input: &TranslationInput,
    output: &mut TranslationResult,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.from_dialect == input.to_platform {
        *output = same_dialect(input);
        return true;
    }
    // 1) 内置静态表（零 AI，AC-20）
    let from = dialect_name(input.from_dialect);
    let to = dialect_name(input.to_platform);
    if let Some(entry) = translation_table::lookup(from, to, &input.command) {
        *output = from_builtin(input, entry);
        metrics.counter("translate.static_hit", 1);
        return true;
    }
    // 2) 缓存（AI 历史译文/用户确认条目）
    let mut cached = None;
    if !lookup_translation(input, &mut cached, context, metrics, evolution) {
        evolution.propagate("translate_command", "查翻译缓存失败");
        return false;
    }
    if let Some(mut hit) = cached {
        let mut risk = crate::risk::RiskAssessment::default();
        if assess_risk(&RiskInput { command: hit.command_to.clone(), cwd: None }, &mut risk, context, metrics, evolution) {
            hit.risk_level = risk.level;
        }
        metrics.counter("translate.cache_hit", 1);
        *output = hit;
        return true;
    }
    // 3) AI 翻译（未命中且后端可用）
    translate_via_ai(input, context, metrics, evolution, output)
}

/// 同方言：原样返回（AC-19 前置分支）。
fn same_dialect(input: &TranslationInput) -> TranslationResult {
    TranslationResult {
        command_from: input.command.clone(),
        command_to: input.command.clone(),
        params: vec![],
        notes: "习惯方言与主机平台一致，无需翻译".into(),
        source: TranslationSource::Native,
        kind: TranslationKind::Direct,
        recipe_id: None,
        verified: true,
        risk_level: crate::types::RiskLevel::Safe,
    }
}

/// 内置条目 → 结果：首词替换 + 参数级映射 + 未匹配参数透传警示。
fn from_builtin(input: &TranslationInput, entry: &translation_table::BuiltinEntry) -> TranslationResult {
    let mut params_out: Vec<ParamMapping> = Vec::new();
    let mut mapped_out: Vec<String> = Vec::new();
    let mut remaining: Vec<String> = input
        .command
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect();
    let mut unmapped = Vec::new();
    // 组合短旗标拆分：-la → -l、-a 逐项查映射（ Renamed 输出目标旗标）
    let mut expanded: Vec<String> = Vec::new();
    for arg in remaining.iter() {
        if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
            for ch in arg[1..].chars() {
                expanded.push(format!("-{ch}"));
            }
        } else {
            expanded.push(arg.clone());
        }
    }
    remaining = expanded;
    for (from_flag, to_flag, kind, note) in entry.params {
        if let Some(pos) = remaining.iter().position(|a| a == from_flag) {
            let arg = remaining.remove(pos);
            match kind {
                ParamMapKind::None => {
                    unmapped.push(format!("{from_flag}（{note}）"));
                    params_out.push(ParamMapping { from_param: arg, to_param: None, kind: ParamMapKind::None, note: Some(note.to_string()) });
                }
                ParamMapKind::ValueTransform => {
                    params_out.push(ParamMapping { from_param: arg.clone(), to_param: None, kind: ParamMapKind::ValueTransform, note: Some(note.to_string()) });
                }
                _ => {
                    let to = to_flag.map(str::to_string).unwrap_or_else(|| arg.clone());
                    params_out.push(ParamMapping { from_param: arg, to_param: Some(to.clone()), kind: *kind, note: Some(note.to_string()) });
                    mapped_out.push(to);
                }
            }
        }
    }
    let mut command_to = entry.command_to.to_string();
    for a in mapped_out.iter().chain(remaining.iter()) {
        command_to.push(' ');
        command_to.push_str(a);
    }
    let mut notes = entry.notes.to_string();
    if !unmapped.is_empty() {
        notes.push_str(&format!("；以下参数无直接等价：{}", unmapped.join("、")));
    }
    TranslationResult {
        command_from: input.command.clone(),
        command_to,
        params: params_out,
        notes,
        source: TranslationSource::Static,
        kind: TranslationKind::Direct,
        recipe_id: None,
        verified: true,
        risk_level: crate::types::RiskLevel::Caution,
    }
}

/// AI 翻译兜底：一对一或组合脚本（AC-24），失败降级原文（AC-19）。
fn translate_via_ai(
    input: &TranslationInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    output: &mut TranslationResult,
) -> bool {
    let provider = default_provider(context.config);
    let ai_in = AiRequest {
        system: "你是跨平台命令翻译专家。把源方言命令翻译为目标平台等价命令。只输出 JSON：\
                 {command_to: 等价命令或脚本, kind: direct|composition, notes: 差异说明(中文), \
                 params: [{from_param, to_param, kind: direct|renamed|value_transform|none, note}], \
                 risk: safe|caution|dangerous|destructive}。一对一可译用 direct；需要命令组合/管道/脚本用 composition。"
            .into(),
        messages: vec![(
            "user".into(),
            format!("源方言：{:?}；目标平台：{:?}；原命令：{}", input.from_dialect, input.to_platform, input.command),
        )],
        temperature: 0.2,
        timeout_ms: 30_000,
    };
    let mut resp = crate::ports::LlmResponse::default();
    if !context.deps.ai.ai_complete(&provider, &ai_in, &mut resp, context, metrics, evolution) {
        // AI 不可用：降级返回原文，不阻塞（AC-19/IAC-1301 语义）
        metrics.counter("translate.degraded", 1);
        *output = TranslationResult::native_fallback(input);
        evolution.succeed();
        return true;
    }
    let Some(parsed) = parse_translation(&resp.text) else {
        metrics.counter("translate.parse_fail", 1);
        *output = TranslationResult::native_fallback(input);
        evolution.succeed();
        return true;
    };
    let mut result = TranslationResult {
        command_from: input.command.clone(),
        command_to: parsed.command_to,
        params: parsed.params,
        notes: parsed.notes,
        source: TranslationSource::Ai,
        kind: if parsed.composition { TranslationKind::Composition } else { TranslationKind::Direct },
        recipe_id: None,
        verified: false,
        risk_level: risk_of(&parsed.risk, input),
    };
    // 组合：脚本入方案库（kind=script，tags=translation/auto）并回填 recipe_id（AC-24）
    if result.kind == TranslationKind::Composition {
        let recipe_in = RecipeInput {
            title: format!("翻译脚本：{}", truncate(&input.command, 40)),
            description: Some(result.notes.clone()),
            kind: Some("script".into()),
            name: None,
            command_template: result.command_to.clone(),
            tags: vec!["translation".into(), "auto".into()],
            platform: dialect_name(input.to_platform).into(),
            risk_level: result.risk_level,
            source_execution_id: None,
        };
        let mut recipe = crate::recipe::Recipe::default();
        if save_recipe(&recipe_in, &mut recipe, context, metrics, evolution) {
            result.recipe_id = Some(recipe.id.clone());
        }
    }
    let cache_in = CacheTranslationInput {
        result: result.clone(),
        from_dialect: input.from_dialect,
        to_platform: input.to_platform,
    };
    let mut ack = crate::knowledge::TranslationAck::default();
    let _ = cache_translation(&cache_in, &mut ack, context, metrics, evolution);
    // 译文风险以评估器为准（闸门唯一依据）
    let mut risk = crate::risk::RiskAssessment::default();
    let _ = assess_risk(&RiskInput { command: result.command_to.clone(), cwd: None }, &mut risk, context, metrics, evolution);
    result.risk_level = risk.level;
    metrics.counter("translate.ai_done", 1);
    *output = result;
    true
}

fn risk_of(name: &str, input: &TranslationInput) -> crate::types::RiskLevel {
    let _ = input;
    match name {
        "safe" => crate::types::RiskLevel::Safe,
        "dangerous" => crate::types::RiskLevel::Dangerous,
        "destructive" => crate::types::RiskLevel::Destructive,
        _ => crate::types::RiskLevel::Caution,
    }
}

struct ParsedTranslation {
    command_to: String,
    params: Vec<ParamMapping>,
    notes: String,
    composition: bool,
    risk: String,
}

/// 解析 AI 翻译响应（剥围栏）。
fn parse_translation(text: &str) -> Option<ParsedTranslation> {
    let t = text.trim();
    let body = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).map(str::trim).unwrap_or(t);
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&body[start..=end]).ok()?;
    let kind = v["kind"].as_str().unwrap_or("direct");
    let params = v["params"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(ParamMapping {
                        from_param: p["from_param"].as_str()?.to_string(),
                        to_param: p["to_param"].as_str().map(str::to_string),
                        kind: match p["kind"].as_str() {
                            Some("renamed") => ParamMapKind::Renamed,
                            Some("value_transform") => ParamMapKind::ValueTransform,
                            Some("none") => ParamMapKind::None,
                            _ => ParamMapKind::Direct,
                        },
                        note: p["note"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ParsedTranslation {
        command_to: v["command_to"].as_str()?.to_string(),
        params,
        notes: v["notes"].as_str().unwrap_or_default().to_string(),
        composition: kind == "composition",
        risk: v["risk"].as_str().unwrap_or("caution").to_string(),
    })
}

fn dialect_name(p: Platform) -> &'static str {
    match p {
        Platform::Linux => "linux",
        Platform::Macos => "macos",
        Platform::Windows => "windows",
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
