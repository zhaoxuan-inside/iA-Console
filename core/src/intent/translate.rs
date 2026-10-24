//! I-033：跨方言翻译编排（AC-19/20/23/24）。链路：内置表（零 AI）→ 缓存 → AI 翻译/组合 → 回写。
//! 组合规划自增长：AI 组合脚本入方案库（kind=script, tags=translation/auto）并回填 recipe_id（AC-24）。

use crate::knowledge::translation_table;
use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::translation_store::{cache_translation, lookup_translation, CacheTranslationInput};
use crate::knowledge::{ParamMap, ParamMapKind, TranslationInput, TranslationResult};
use crate::intent::handler::default_provider;
use crate::ports::{AiRequest, LlmResponse};
use crate::recipe::handler::save_recipe;
use crate::recipe::RecipeInput;
use crate::risk::assess_static;
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
    if input.from == input.to {
        *output = same_dialect(input);
        return true;
    }
    // 1) 内置静态表（零 AI，AC-20）
    let from = dialect_name(input.from);
    let to = dialect_name(input.to);
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
        hit.risk_level = Some(assess_static(&hit.translated));
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
        original: input.command.clone(),
        translated: input.command.clone(),
        param_maps: vec![],
        source: "native".into(),
        note: "习惯方言与主机平台一致，无需翻译".into(),
        risk_level: Some(crate::types::RiskLevel::Safe),
        recipe_id: None,
    }
}

/// 内置条目 → 结果：首词替换 + 参数级映射 + 未匹配参数透传警示。
fn from_builtin(input: &TranslationInput, entry: &translation_table::BuiltinEntry) -> TranslationResult {
    let mut params_out: Vec<ParamMap> = Vec::new();
    let mut mapped_out: Vec<String> = Vec::new();
    let mut remaining: Vec<String> =
        input.command.split_whitespace().skip(1).map(str::to_string).collect();
    let mut unmapped = Vec::new();
    // 组合短旗标拆分：-la → -l、-a 逐项查映射
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
    for &(from_flag, to_flag, kind, note) in entry.params {
        if let Some(pos) = remaining.iter().position(|a| a == from_flag) {
            let arg = remaining.remove(pos);
            match kind {
                ParamMapKind::None => {
                    unmapped.push(format!("{from_flag}（{note}）"));
                    params_out.push(ParamMap {
                        kind: ParamMapKind::None,
                        from: arg,
                        to: None,
                        note: note.to_string(),
                    });
                }
                ParamMapKind::ValueTransform => {
                    params_out.push(ParamMap {
                        kind: ParamMapKind::ValueTransform,
                        from: arg,
                        to: None,
                        note: note.to_string(),
                    });
                }
                _ => {
                    let to_flag_str = to_flag.map(str::to_string).unwrap_or_else(|| arg.clone());
                    params_out.push(ParamMap {
                        kind,
                        from: arg,
                        to: Some(to_flag_str.clone()),
                        note: note.to_string(),
                    });
                    mapped_out.push(to_flag_str);
                }
            }
        }
    }
    let mut translated = entry.command_to.to_string();
    for a in mapped_out.iter().chain(remaining.iter()) {
        translated.push(' ');
        translated.push_str(a);
    }
    let mut note = entry.notes.to_string();
    if !unmapped.is_empty() {
        note.push_str(&format!("；以下参数无直接等价：{}", unmapped.join("、")));
    }
    TranslationResult {
        original: input.command.clone(),
        translated,
        param_maps: params_out,
        source: "static".into(),
        note,
        risk_level: Some(assess_static(entry.command_to)),
        recipe_id: None,
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
                 {translated: 等价命令或脚本, composition: true|false, note: 差异说明(中文), \
                 param_maps: [{kind: direct|renamed|value_transform|none, from, to, note}], \
                 risk: safe|caution|dangerous|destructive}。一对一可译 composition=false；需要命令组合/管道/脚本 composition=true。"
            .into(),
        messages: vec![(
            "user".into(),
            format!("源方言：{:?}；目标平台：{:?}；原命令：{}", input.from, input.to, input.command),
        )],
        temperature: 0.2,
        timeout_ms: 30_000,
    };
    let mut resp = LlmResponse::default();
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
        original: input.command.clone(),
        translated: parsed.translated.clone(),
        param_maps: parsed.param_maps,
        source: "ai".into(),
        note: parsed.note,
        risk_level: Some(assess_static(&parsed.translated)),
        recipe_id: None,
    };
    // 组合：脚本入方案库（kind=script，tags=translation/auto）并回填 recipe_id（AC-24）
    if parsed.composition {
        let recipe_in = RecipeInput {
            title: format!("翻译脚本：{}", truncate(&input.command, 40)),
            description: Some(result.note.clone()),
            kind: "script".into(),
            name: None,
            command_template: result.translated.clone(),
            tags: vec!["translation".into(), "auto".into()],
            platform: dialect_name(input.to).into(),
            source_execution_id: None,
        };
        let mut recipe = crate::recipe::Recipe::default();
        if save_recipe(&recipe_in, &mut recipe, context, metrics, evolution) {
            result.recipe_id = Some(recipe.id.clone());
        }
    }
    let cache_in = CacheTranslationInput { input: input.clone(), result: result.clone(), verified: false };
    let _ = cache_translation(&cache_in, context, metrics, evolution);
    metrics.counter("translate.ai_done", 1);
    *output = result;
    true
}

struct ParsedTranslation {
    translated: String,
    param_maps: Vec<ParamMap>,
    note: String,
    composition: bool,
}

/// 解析 AI 翻译响应（剥围栏）。
fn parse_translation(text: &str) -> Option<ParsedTranslation> {
    let t = text.trim();
    let body = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).map(str::trim).unwrap_or(t);
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(&body[start..=end]).ok()?;
    let param_maps = v["param_maps"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(ParamMap {
                        kind: match p["kind"].as_str() {
                            Some("renamed") => ParamMapKind::Renamed,
                            Some("value_transform") => ParamMapKind::ValueTransform,
                            Some("none") => ParamMapKind::None,
                            _ => ParamMapKind::Direct,
                        },
                        from: p["from"].as_str()?.to_string(),
                        to: p["to"].as_str().map(str::to_string),
                        note: p["note"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ParsedTranslation {
        translated: v["translated"].as_str()?.to_string(),
        param_maps,
        note: v["note"].as_str().unwrap_or_default().to_string(),
        composition: v["composition"].as_bool().unwrap_or(false),
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
