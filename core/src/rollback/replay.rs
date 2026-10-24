//! I-037 rollback_to（orchestration）：范围回放（to/from）逐行倒序 + 冲突决策 + 回收站（AC-30/31/33/34）。

use super::EffectEntryRef;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{ContentRef, SnapshotData, SqlQuery, SqlValue, TrashRequest};
use crate::types::{ConflictPolicy, EffectOp, RollbackReport, RollbackRequest, Rollbackable};

/// 范围回放：对 (to, from] 区间逐行倒序逆操作；created→回收站、modified→回收站现值+写回前像、
/// deleted→重建前像。冲突（现内容≠预期 after）按 --on-conflict 决策；引用全部缺失报 IAC-1602。
pub fn rollback_to(
    input: &RollbackRequest,
    output: &mut RollbackReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let to = input.range.to.unwrap_or(0);
    let from = input.range.from.unwrap_or(i64::MAX);
    if from <= to {
        evolution.fail("IAC-1604", format!("回滚范围无效（from={from} ≤ to={to}）"));
        return false;
    }
    let entries = load_range(to, from, context, metrics, evolution);
    if entries.is_none() {
        evolution.propagate("rollback_to", "效果日志读取失败");
        return false;
    }
    let entries = entries.unwrap();
    if entries.is_empty() {
        evolution.fail(
            "IAC-1604",
            "回滚范围内没有任何效果条目（用 iac rollback --list 查看有效行号）",
        );
        return false;
    }
    let mut report = RollbackReport {
        applied: Vec::new(),
        skipped: Vec::new(),
        trash_dir: context
            .deps
            .snap
            .data_root()
            .join("trash")
            .to_string_lossy()
            .to_string(),
    };
    let mut swept = 0u32;
    for row in entries.iter().rev() {
        let Some(entry) = row.to_entry() else {
            continue;
        };
        let needed = needed_ref(&entry);
        let mut content = SnapshotData::default();
        let fetched = match &needed {
            Some(r) => context.deps.snap.snapstore_fetch(
                &ContentRef {
                    reference: r.clone(),
                },
                &mut content,
                context,
                metrics,
                evolution,
            ),
            None => true,
        };
        if entry.scope == crate::types::CaptureScopeFlag::OutOfScope {
            report.skipped.push(crate::types::SkippedEntry {
                entry: entry.clone(),
                reason: "超出捕获范围，不可自动回滚（AC-29）".into(),
            });
            continue;
        }
        if !fetched {
            swept += 1;
            report.skipped.push(crate::types::SkippedEntry {
                entry: entry.clone(),
                reason: "前像快照已被 retention 回收（IAC-1602）".into(),
            });
            continue;
        }
        match apply_entry(
            &entry,
            &content.bytes,
            input.on_conflict,
            &report.trash_dir,
            context,
            metrics,
            evolution,
        ) {
            Ok(()) => report.applied.push(entry),
            Err(reason) => {
                let conflict = reason.contains("IAC-1603");
                if conflict && input.on_conflict == ConflictPolicy::Abort {
                    mark_abort_remainder(&entries, &entry, &mut report);
                    evolution.fail("IAC-1603", reason);
                    evolution.ok = true;
                    *output = report;
                    return true;
                }
                report.skipped.push(crate::types::SkippedEntry {
                    entry: entry.clone(),
                    reason,
                });
                if conflict {
                    metrics.counter("rollback.conflict_skipped", 1);
                }
            }
        }
    }
    if !report.applied.is_empty() || swept == 0 {
        // 撤销过的内容统一入回收站；引用缺失时给最近可用提示由前端呈现
    }
    metrics.counter("rollback.applied", report.applied.len() as u64);
    *output = report;
    true
}

/// 该条目逆操作所需的前像引用。
fn needed_ref(entry: &crate::types::EffectEntry) -> Option<String> {
    match entry.op {
        EffectOp::Created => None, // 撤销创建=移入回收站，无需前像
        EffectOp::Modified | EffectOp::Deleted | EffectOp::Renamed => entry.before_ref.clone(),
    }
}

/// 应用单条逆操作；Err(原因) = 冲突（含 IAC-1603 字样）或失败。
fn apply_entry(
    entry: &crate::types::EffectEntry,
    before_bytes: &[u8],
    policy: ConflictPolicy,
    _trash_dir: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&entry.path);
    let current = std::fs::read(&path).ok();
    match entry.op {
        EffectOp::Created => {
            if current.is_none() {
                return Err("目标文件已不存在，视为已满足".into());
            }
            if !content_matches(
                context,
                metrics,
                evolution,
                current.as_deref(),
                &entry.after_ref,
            )? {
                return Err("IAC-1603: 目标路径已被后续操作改变".into());
            }
            let mut ack = crate::ports::TrashAck::default();
            if !context.deps.snap.trash_file(
                &TrashRequest {
                    abs_path: path,
                    tag: format!("{}_{}", entry.execution_id, entry.seq),
                },
                &mut ack,
                context,
                metrics,
                evolution,
            ) {
                return Err("移入回收站失败（IAC-1601）".into());
            }
            Ok(())
        }
        EffectOp::Modified => {
            if current.is_none() {
                return Err("IAC-1603: 目标文件已被删除".into());
            }
            if !content_matches(
                context,
                metrics,
                evolution,
                current.as_deref(),
                &entry.after_ref,
            )? {
                if policy == ConflictPolicy::Abort {
                    return Err("IAC-1603: 目标路径已被后续操作改变".into());
                }
                return Err("IAC-1603: 冲突→skip".into());
            }
            trash_and_write(context, metrics, evolution, &path, before_bytes)
        }
        EffectOp::Deleted => {
            if current.is_some() {
                return Err("IAC-1603: 目标路径已存在新内容".into());
            }
            write_through(context, metrics, evolution, &path, before_bytes)
        }
        EffectOp::Renamed => {
            // v0.2.0 不产生 renamed 效果（created+deleted 组合表达）；防御性处理
            Err("renamed 条目请用单文件恢复处理".into())
        }
    }
}

/// 现内容与效果后像一致（内容寻址等价比较：现内容入库引用 == after_ref）。
fn content_matches(
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    current: Option<&[u8]>,
    after_ref: &Option<String>,
) -> Result<bool, String> {
    let Some(bytes) = current else {
        return Ok(false);
    };
    let Some(expected) = after_ref else {
        return Ok(false);
    };
    let mut cref = ContentRef::default();
    if !context.deps.snap.snapstore_put(
        &SnapshotData {
            bytes: bytes.to_vec(),
        },
        &mut cref,
        context,
        metrics,
        evolution,
    ) {
        return Err("现内容指纹计算失败（IAC-1601）".into());
    }
    Ok(cref.reference == *expected)
}

/// 现文件入回收站后写回前像内容。
fn trash_and_write(
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), String> {
    let mut ack = crate::ports::TrashAck::default();
    if !context.deps.snap.trash_file(
        &TrashRequest {
            abs_path: path.to_path_buf(),
            tag: format!("revert-{}", now_tag()),
        },
        &mut ack,
        context,
        metrics,
        evolution,
    ) {
        return Err("移入回收站失败（IAC-1601）".into());
    }
    write_through(context, metrics, evolution, path, bytes)
}

/// 直接写入。
fn write_through(
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), String> {
    if !context.deps.snap.write_file(
        &crate::ports::WriteFileInput {
            path: path.to_path_buf(),
            bytes: bytes.to_vec(),
        },
        context,
        metrics,
        evolution,
    ) {
        return Err("恢复写入失败（IAC-1601）".into());
    }
    Ok(())
}

fn now_tag() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

/// Abort 策略：剩余未处理条目全部记为 skipped。
fn mark_abort_remainder(
    entries: &[EffectEntryRef],
    abort_at: &crate::types::EffectEntry,
    report: &mut RollbackReport,
) {
    let mut rest = false;
    for row in entries.iter().rev() {
        if row.seq == abort_at.seq {
            rest = true;
            continue;
        }
        if rest {
            if let Some(e) = row.to_entry() {
                report.skipped.push(crate::types::SkippedEntry {
                    entry: e,
                    reason: "abort 策略中止回放（IAC-1603）".into(),
                });
            }
        }
    }
}

/// 读取 (to, from] 区间 journal 行（seq 降序由调用方 rev 处理，此处升序）。
fn load_range(
    to: i64,
    from: i64,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<Vec<EffectEntryRef>> {
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT seq, execution_id, path, op, before_ref, after_ref, extra_path, scope, created_at \
              FROM rollback_journal WHERE seq > ? AND seq <= ? ORDER BY seq ASC"
            .into(),
        params: vec![SqlValue::Int(to), SqlValue::Int(from)],
    };
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..rows.len() {
        let strv = |col: &str| rows.text(i, col).map(str::to_string);
        out.push(EffectEntryRef {
            seq: rows.int(i, "seq")?,
            execution_id: strv("execution_id")?,
            path: strv("path")?,
            op: strv("op")?,
            before_ref: strv("before_ref"),
            after_ref: strv("after_ref"),
            extra_path: strv("extra_path"),
            scope: strv("scope")?,
            created_at: rows.int(i, "created_at")?,
        });
    }
    Some(out)
}

/// 供 list 判定用（公共：引用完整性与范围标记聚合）。
pub fn point_rollbackable(out_count: u32, total: u32, refs_missing: bool) -> Rollbackable {
    if total == 0 || (refs_missing && out_count == total) {
        Rollbackable::None {
            reason: "效果条目缺失或引用不可用".into(),
        }
    } else if out_count == 0 && !refs_missing {
        Rollbackable::Full
    } else {
        Rollbackable::Partial {
            reason: format!("{out_count}/{total} 项超出捕获范围或引用不可用"),
        }
    }
}
