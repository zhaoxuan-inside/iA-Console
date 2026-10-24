//! I-036 diff_effects（data）：manifest vs 现况 → 效果日志入库（rollback_journal）。

use super::{journal_insert, next_seq, EffectEntryRef, ManifestDoc};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{ContentRef, SnapshotData, SqlQuery};
use crate::types::{EffectEntry, ManifestRef};

/// manifest 与现况对比生成效果日志：created/modified/deleted 三类；
/// 内容超限未入库的变更标 out_of_scope（AC-29）。
pub fn diff_effects(
    input: &ManifestRef,
    output: &mut Vec<EffectEntry>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.manifest_ref.is_empty() || input.execution_id.is_empty() {
        return true;
    }
    let mut doc_bytes = SnapshotData::default();
    let mref = ContentRef { reference: input.manifest_ref.clone() };
    if !context.deps.snap.snapstore_fetch(&mref, &mut doc_bytes, context, metrics, evolution) {
        evolution.propagate("diff_effects", "manifest 取回失败");
        return false;
    }
    let doc: ManifestDoc = match serde_json::from_slice(&doc_bytes.bytes) {
        Ok(d) => d,
        Err(e) => {
            evolution.fail("IAC-1601", format!("manifest 解析失败: {e}"));
            return false;
        }
    };
    let mut current = Vec::new();
    let tree_in = crate::ports::CaptureTreeInput {
        roots: doc.roots.iter().map(std::path::PathBuf::from).collect(),
        excludes: super::CAPTURE_EXCLUDES.iter().map(|s| s.to_string()).collect(),
        max_file_bytes: super::CAPTURE_MAX_FILE_BYTES,
        max_total_bytes: super::CAPTURE_MAX_TOTAL_BYTES,
    };
    if !context.deps.snap.capture_tree(&tree_in, &mut current, context, metrics, evolution) {
        evolution.propagate("diff_effects", "现况捕获失败");
        return false;
    }
    let entries = build_entries(&doc, &current, input, context, metrics, evolution);
    let mut out = Vec::new();
    for e in &entries {
        if !journal_insert(e, context, metrics, evolution) {
            evolution.propagate("diff_effects", "效果日志入库失败");
            return false;
        }
        if let Some(domain) = e.to_entry() {
            out.push(domain);
        }
    }
    enforce_max_entries(context, metrics, evolution);
    let _ = context.deps.store.storage_execute(
        &crate::ports::SqlQuery {
            sql: "UPDATE executions SET journal_ref=? WHERE id=?".into(),
            params: vec![
                crate::ports::SqlValue::Text(input.manifest_ref.clone()),
                crate::ports::SqlValue::Text(input.execution_id.clone()),
            ],
        },
        &mut crate::ports::ExecuteAck::default(),
        context,
        metrics,
        evolution,
    );
    metrics.counter("rollback.effects", out.len() as u64);
    *output = out;
    true
}

/// 生成效果中间条目（in/单文件内容引用缺失 → out_of_scope）。
fn build_entries(
    doc: &ManifestDoc,
    current: &[crate::ports::FileCaptured],
    input: &ManifestRef,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Vec<EffectEntryRef> {
    let now = crate::session::now_ms();
    let mut seq = next_seq(context, metrics, evolution).unwrap_or(1);
    let before: std::collections::HashMap<&str, &crate::ports::FileCaptured> =
        doc.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let after: std::collections::HashMap<&str, &crate::ports::FileCaptured> =
        current.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut entries = Vec::new();
    let mut push = |path: &str, op: &str, before: Option<&str>, after: Option<&str>, entries: &mut Vec<EffectEntryRef>| {
        let scope = if before.or(after).is_some() { "in" } else { "out_of_scope" };
        entries.push(EffectEntryRef {
            seq,
            execution_id: input.execution_id.clone(),
            path: path.to_string(),
            op: op.to_string(),
            before_ref: before.map(str::to_string),
            after_ref: after.map(str::to_string),
            extra_path: None,
            scope: scope.to_string(),
            created_at: now,
        });
        seq += 1;
    };
    for (path, old) in &before {
        match after.get(*path) {
            None => push(path, "deleted", old.content_ref.as_deref(), None, &mut entries),
            Some(new) if new.sha256 != old.sha256 => {
                push(path, "modified", old.content_ref.as_deref(), new.content_ref.as_deref(), &mut entries)
            }
            Some(_) => {}
        }
    }
    for (path, new) in &after {
        if !before.contains_key(*path) {
            push(path, "created", None, new.content_ref.as_deref(), &mut entries);
        }
    }
    entries
}

/// AC-32：回滚执行点数超 rollback.max_entries 时清退最旧执行点的日志行。
fn enforce_max_entries(
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<()> {
    let max = context.config.rollback.max_entries as i64;
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT COUNT(DISTINCT execution_id) AS n FROM rollback_journal".into(),
        params: vec![],
    };
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        return None;
    }
    let points = rows.int(0, "n")?;
    if points <= max {
        return Some(());
    }
    let excess = points - max;
    let dq = SqlQuery {
        sql: "DELETE FROM rollback_journal WHERE execution_id IN (\
              SELECT execution_id FROM rollback_journal GROUP BY execution_id \
              ORDER BY MIN(seq) LIMIT ?)"
            .into(),
        params: vec![crate::ports::SqlValue::Int(excess)],
    };
    let mut ack = crate::ports::ExecuteAck::default();
    if context.deps.store.storage_execute(&dq, &mut ack, context, metrics, evolution) {
        metrics.counter("rollback.retired_points", excess as u64);
        Some(())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::TestEnv;

    #[test]
    fn empty_manifest_is_noop() {
        let env = TestEnv::new();
        let mut out = Vec::new();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(diff_effects(&ManifestRef::default(), &mut out, &env.context(), &mut m, &mut evo));
        assert!(out.is_empty());
    }

}
