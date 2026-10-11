//! 翻译映射存取（I-031/I-032）：translations 表按 (from,to,command_from) 唯一。

use super::translation::{TranslationAck, TranslationResult};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use crate::session::now_ms;

/// I-031：翻译缓存查找（AI 译文与用户确认条目；内置表在 translate_command 内查）。
pub fn lookup_translation(
    input: &super::translation::TranslationInput,
    output: &mut Option<TranslationResult>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "SELECT id, command_to, params_json, notes, kind, recipe_id, verified \
              FROM translations WHERE from_dialect = ?1 AND to_platform = ?2 AND command_from = ?3"
            .into(),
        params: vec![
            format!("{:?}", input.from_dialect).to_lowercase().as_str().into(),
            format!("{:?}", input.to_platform).to_lowercase().as_str().into(),
            input.command.as_str().into(),
        ],
    };
    // 缓存键 = 源命令首词（与登记一致；整条命令的参数差异由 params 映射消化）
    let Some(first_word) = input.command.split_whitespace().next() else {
        metrics.counter("translate.miss", 1);
        *output = None;
        return true;
    };
    let mut rows = QueryRows::default();
    let mut q = q;
    q.params[2] = first_word.into();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("lookup_translation", "查询翻译缓存失败");
        return false;
    }
    if rows.is_empty() {
        metrics.counter("translate.miss", 1);
        *output = None;
        return true;
    }
    let params: Vec<super::translation::ParamMapping> = rows
        .text(0, "params_json")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let kind_raw = rows.text(0, "kind").unwrap_or("direct");
    *output = Some(TranslationResult {
        command_from: input.command.clone(),
        command_to: rows.text(0, "command_to").unwrap_or_default().into(),
        params,
        notes: rows.text(0, "notes").unwrap_or_default().into(),
        source: super::translation::TranslationSource::Static,
        kind: if kind_raw == "composition" { super::translation::TranslationKind::Composition } else { super::translation::TranslationKind::Direct },
        recipe_id: rows.text(0, "recipe_id").map(str::to_string),
        verified: rows.int(0, "verified").unwrap_or(0) == 1,
        risk_level: crate::types::RiskLevel::Caution,
    });
    metrics.counter("translate.cache_hit", 1);
    true
}

/// 缓存写入输入：结果 + 显式方言上下文（I-032 契约细化，_06 已同步）。
#[derive(Debug, Clone)]
pub struct CacheTranslationInput {
    pub result: TranslationResult,
    pub from_dialect: crate::types::Platform,
    pub to_platform: crate::types::Platform,
}

/// I-032：翻译结果回写缓存（AI 译文 verified=false；用户确认转正）。
pub fn cache_translation(
    input: &CacheTranslationInput,
    output: &mut TranslationAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let result = &input.result;
    let Some(from) = result.command_from.split_whitespace().next() else {
        evolution.fail("IAC-1103", "原命令为空，无法登记翻译");
        return false;
    };
    let params_json = serde_json::to_string(&result.params).unwrap_or_else(|_| "[]".into());
    let kind = match result.kind {
        super::translation::TranslationKind::Composition => "composition",
        super::translation::TranslationKind::Direct => "direct",
    };
    let source = match result.source {
        super::translation::TranslationSource::Native => return true,
        super::translation::TranslationSource::Static => "user",
        super::translation::TranslationSource::Ai => "ai",
    };
    let sql = "INSERT INTO translations (id, from_dialect, to_platform, command_from, command_to, params_json, notes, kind, recipe_id, verified, source, updated_at) \
               VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
               ON CONFLICT(from_dialect, to_platform, command_from) DO UPDATE SET \
               command_to=?4, params_json=?5, notes=?6, kind=?7, recipe_id=?8, verified=?9, source=?10, updated_at=?11 \
               RETURNING id";
    let q = SqlQuery {
        sql: sql.into(),
        params: vec![
            format!("{:?}", input.from_dialect).to_lowercase().as_str().into(),
            format!("{:?}", input.to_platform).to_lowercase().as_str().into(),
            from.into(),
            result.command_to.as_str().into(),
            params_json.as_str().into(),
            result.notes.as_str().into(),
            kind.into(),
            opt(&result.recipe_id),
            i64::from(result.verified).into(),
            source.into(),
            now_ms().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("cache_translation", "写入翻译缓存失败");
        return false;
    }
    output.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("translate.cached", 1);
    true
}

fn opt(v: &Option<String>) -> SqlValue {
    v.as_deref().map(SqlValue::from).unwrap_or(SqlValue::Null)
}
