//! I-041 collect_commands（data）：任务段（`---` 分隔的最近段）或行号区间 (to, from]
//! 统一收集执行命令序列（seq=executions.rowid，与回滚同一套"行"语义）。

use crate::common::script_assembler::ScriptCommand;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use serde::{Deserialize, Serialize};

/// 收集范围（_06 I-041 契约类型 CollectRange）：segment_id 优先，否则行号区间。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_seq: Option<i64>,
}

/// 收集命令序列：segment_id="last" → 最近一个 `---` 分隔段的时间窗；否则 (to, from] 行号区间。
pub fn collect_commands(
    input: &CollectRange,
    output: &mut Vec<ScriptCommand>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.segment_id.as_deref() == Some("last") {
        return collect_last_segment(output, context, metrics, evolution);
    }
    let from = input.from_seq.unwrap_or(i64::MAX);
    let to = input.to_seq.unwrap_or(0);
    if from <= to {
        evolution.fail("IAC-1604", format!("行号区间无效（from={from} ≤ to={to}）"));
        return false;
    }
    let mut rows = QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT rowid AS seq, command FROM executions \
              WHERE command NOT LIKE '@%' AND rowid > ?1 AND rowid <= ?2 ORDER BY rowid ASC"
            .into(),
        params: vec![SqlValue::Int(to), SqlValue::Int(from)],
    };
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("collect_commands", "执行记录读取失败");
        return false;
    }
    *output = rows_to_commands(&rows);
    metrics.counter("script.collected", output.len() as u64);
    true
}

/// 最近一个任务段：找最近 separator 事件的时间点，收集其后（含分隔后首条）至今的执行。
fn collect_last_segment(
    output: &mut Vec<ScriptCommand>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let mut rows = QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT payload, created_at FROM session_events WHERE kind='separator' \
              ORDER BY seq DESC LIMIT 1"
            .into(),
        params: vec![],
    };
    // 无分隔事件 → 收集全部
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("collect_last_segment", "会话事件读取失败");
        return false;
    }
    let since = if rows.is_empty() {
        0
    } else {
        rows.int(0, "created_at").unwrap_or(0)
    };
    let mut execs = QueryRows::default();
    let q2 = SqlQuery {
        sql: "SELECT rowid AS seq, command, started_at FROM executions \
              WHERE command NOT LIKE '@%' AND started_at >= ?1 ORDER BY rowid ASC"
            .into(),
        params: vec![SqlValue::Int(since)],
    };
    if !context.deps.store.storage_query(&q2, &mut execs, context, metrics, evolution) {
        evolution.propagate("collect_last_segment", "执行记录读取失败");
        return false;
    }
    *output = rows_to_commands(&execs);
    metrics.counter("script.collected_segment", output.len() as u64);
    true
}

/// 行 → 命令条目（空命令跳过）。
fn rows_to_commands(rows: &QueryRows) -> Vec<ScriptCommand> {
    let mut out = Vec::new();
    for i in 0..rows.len() {
        let seq = rows.int(i, "seq").unwrap_or(0);
        let command = rows.text(i, "command").unwrap_or("").trim().to_string();
        if !command.is_empty() {
            out.push(ScriptCommand { seq, command });
        }
    }
    out
}
