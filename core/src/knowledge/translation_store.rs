//! 翻译缓存存取（I-031/I-032）：translations 表按 (original, from, to) 唯一。
//! 缓存键 = 完整源命令（参数不同即未命中，杜绝首词键误取旧参数译文）。

use super::{ParamMap, TranslationInput, TranslationResult};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use crate::session::now_ms;

/// 翻译缓存查找（AI 译文与用户确认条目；内置表在 translate_command 内查）。
pub fn lookup_translation(
    input: &TranslationInput,
    output: &mut Option<TranslationResult>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.command.trim().is_empty() {
        metrics.counter("translate.miss", 1);
        *output = None;
        return true;
    }
    let q = SqlQuery {
        sql: "SELECT original, translated, param_maps, source, recipe_id, verified \
              FROM translations WHERE from_platform = ?1 AND to_platform = ?2 AND original = ?3 \
              ORDER BY created_at DESC LIMIT 1"
            .into(),
        params: vec![
            platform_text(input.from).into(),
            platform_text(input.to).into(),
            input.command.trim().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("lookup_translation", "查询翻译缓存失败");
        return false;
    }
    if rows.is_empty() {
        metrics.counter("translate.miss", 1);
        *output = None;
        return true;
    }
    let param_maps: Vec<ParamMap> =
        serde_json::from_str(rows.text(0, "param_maps").unwrap_or("[]")).unwrap_or_default();
    let source = rows.text(0, "source").unwrap_or("cache").to_string();
    *output = Some(TranslationResult {
        // 存储的原文回填（校验方据此确认命中与当前命令一致）
        original: rows.text(0, "original").unwrap_or_default().into(),
        translated: rows.text(0, "translated").unwrap_or_default().into(),
        param_maps,
        // schema 无 note 列（历史 SELECT 含 note 恒报 IAC-3102，缓存读取自 v0.2.0 起从未生效）
        note: format!("命中翻译缓存（{source}）"),
        source,
        risk_level: None,
        recipe_id: rows
            .text(0, "recipe_id")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    });
    metrics.counter("translate.cache_hit", 1);
    true
}

/// 翻译缓存写入输入。
#[derive(Debug, Clone)]
pub struct CacheTranslationInput {
    pub input: TranslationInput,
    pub result: TranslationResult,
    pub verified: bool,
}

/// 翻译缓存写入（upsert；AI 条目 verified=false，AC-23 用户确认后转正）。
pub fn cache_translation(
    input: &CacheTranslationInput,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    // 缓存键 = 完整源命令（与 lookup 同键；参数不同即独立条目，防误取旧参数译文）
    let command = input.input.command.trim().to_string();
    if command.is_empty() {
        return true;
    }
    let param_maps =
        serde_json::to_string(&input.result.param_maps).unwrap_or_else(|_| "[]".into());
    let q = SqlQuery {
        sql: "INSERT INTO translations (id, original, translated, from_platform, to_platform, \
              param_maps, source, verified, recipe_id, created_at) \
              VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
              ON CONFLICT(original, from_platform, to_platform) \
              DO UPDATE SET translated = ?2, param_maps = ?5, source = ?6, verified = ?7, \
              recipe_id = ?8, created_at = ?9"
            .into(),
        params: vec![
            command.as_str().into(),
            input.result.translated.as_str().into(),
            platform_text(input.input.from).into(),
            platform_text(input.input.to).into(),
            param_maps.as_str().into(),
            input.result.source.as_str().into(),
            i64::from(input.verified).into(),
            input
                .result
                .recipe_id
                .clone()
                .unwrap_or_default()
                .as_str()
                .into(),
            now_ms().into(),
        ],
    };
    if !context
        .deps
        .store
        .storage_execute(&q, &mut Default::default(), context, metrics, evolution)
    {
        evolution.propagate("cache_translation", "写翻译缓存失败");
        return false;
    }
    metrics.counter("translate.cached", 1);
    true
}

/// Platform → 存储文本。
fn platform_text(p: crate::types::Platform) -> &'static str {
    match p {
        crate::types::Platform::Linux => "linux",
        crate::types::Platform::Macos => "macos",
        crate::types::Platform::Windows => "windows",
    }
}

/// 未使用的 SqlValue 导入告警抑制说明：查询参数构造保留显式类型。
#[allow(unused_imports)]
use SqlValue as _SqlValue;
