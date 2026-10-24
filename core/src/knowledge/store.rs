//! 知识库存取（I-003/I-004）：knowledge_cache 表按 (command, platform) 唯一，source_hash 失效。

use super::{CommandKnowledge, CommandRef, KnowledgeAck};
use crate::common::hash::fnv1a64_hex;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery};
use crate::session::now_ms;

/// 缓存键：hash(command + platform)。
fn cache_key(command: &str, platform: &crate::types::Platform) -> String {
    fnv1a64_hex(&format!("{command}|{platform:?}"))
}

/// I-003：查询命令知识。未命中（或 source_hash 过期）→ None（业务"否"，仍 true）。
pub fn lookup_knowledge(
    input: &CommandRef,
    output: &mut Option<CommandKnowledge>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let id = cache_key(&input.command, &input.platform);
    let q = SqlQuery {
        sql: "SELECT schema_json, source_hash, verified FROM knowledge_cache WHERE id = ?1".into(),
        params: vec![id.as_str().into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("lookup_knowledge", "查询知识缓存失败");
        return false;
    }
    if rows.is_empty() {
        metrics.counter("knowledge.miss", 1);
        *output = None;
        return true;
    }
    let stored_hash = rows.text(0, "source_hash").unwrap_or_default().to_string();
    if let Some(expect) = &input.source_hash {
        if expect != &stored_hash {
            metrics.counter("knowledge.stale", 1);
            *output = None;
            return true;
        }
    }
    *output = parse_cached(&rows);
    true
}

/// 行 → CommandKnowledge（lookup_knowledge 子步骤；损坏视为未命中）。
fn parse_cached(rows: &QueryRows) -> Option<CommandKnowledge> {
    let schema = rows.text(0, "schema_json")?;
    let mut k: CommandKnowledge = serde_json::from_str(schema).ok()?;
    k.verified = rows.int(0, "verified").unwrap_or(0) == 1;
    k.source_hash = rows.text(0, "source_hash").unwrap_or_default().to_string();
    Some(k)
}

/// I-004：写入/更新缓存（upsert；source_hash 变更覆盖）。
pub fn cache_knowledge(
    input: &CommandKnowledge,
    output: &mut KnowledgeAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let id = cache_key(&input.command, &input.platform);
    let schema = serde_json::to_string(input).unwrap_or_default();
    let q = SqlQuery {
        sql: "INSERT INTO knowledge_cache (id, command, platform, schema_json, source_hash, verified, updated_at) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
              ON CONFLICT(command, platform) DO UPDATE SET schema_json=?4, source_hash=?5, verified=?6, updated_at=?7 \
              RETURNING id"
            .into(),
        params: vec![
            id.as_str().into(),
            input.command.as_str().into(),
            format!("{:?}", input.platform).to_lowercase().as_str().into(),
            schema.as_str().into(),
            input.source_hash.as_str().into(),
            i64::from(input.verified).into(),
            now_ms().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("cache_knowledge", "写入知识缓存失败");
        return false;
    }
    output.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("knowledge.cached", 1);
    true
}
