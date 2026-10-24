//! I-040 list_rollback_points（data）：history 式回滚点聚合视图（seq/时间/命令/可回滚标识，AC-33）。

use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{SqlQuery, SqlValue};
use crate::types::{RollbackListFilter, RollbackPointView};

/// 回滚执行点列表：每执行点一行，seq=该执行最末日志行（回滚到该行=撤销其后全部）。
/// 可回滚标识：全 in 且引用完整=✔；部分 out=◐；全 out 或引用缺失=✘+原因。
pub fn list_rollback_points(
    input: &RollbackListFilter,
    output: &mut Vec<RollbackPointView>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT e.id AS exec_id, MAX(j.seq) AS seq, MAX(j.created_at) AS at, e.command AS command, \
              COUNT(*) AS total, SUM(CASE WHEN j.scope='out_of_scope' THEN 1 ELSE 0 END) AS out_n, \
              SUM(CASE WHEN (j.op IN ('modified','deleted') AND j.before_ref IS NULL) \
                   OR (j.op IN ('created','modified') AND j.after_ref IS NULL) THEN 1 ELSE 0 END) AS missing_n \
              FROM rollback_journal j JOIN executions e ON e.id = j.execution_id \
              GROUP BY e.id ORDER BY seq DESC LIMIT ?"
            .into(),
        params: vec![SqlValue::Int(i64::from(input.limit.max(1)))],
    };
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("list_rollback_points", "回滚点读取失败");
        return false;
    }
    let mut views = Vec::new();
    for i in 0..rows.len() {
        let total = rows.int(i, "total").unwrap_or(0) as u32;
        let out_n = rows.int(i, "out_n").unwrap_or(0) as u32;
        let missing = rows.int(i, "missing_n").unwrap_or(0) > 0;
        // q 过滤：命令 LIKE 子串（大小写不敏感，SQLite LIKE 默认 ASCII）
        if let Some(qs) = &input.q {
            let cmd = rows.text(i, "command").unwrap_or("");
            if !cmd.to_lowercase().contains(&qs.to_lowercase()) {
                continue;
            }
        }
        let rollbackable = crate::rollback::replay::point_rollbackable(out_n, total, missing);
        views.push(RollbackPointView {
            seq: rows.int(i, "seq").unwrap_or(0),
            at: rows.int(i, "at").unwrap_or(0),
            command: rows.text(i, "command").unwrap_or("").to_string(),
            effect_count: total,
            rollbackable,
        });
    }
    metrics.counter("rollback.list", views.len() as u64);
    *output = views;
    true
}
