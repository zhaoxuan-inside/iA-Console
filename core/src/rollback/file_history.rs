//! I-038 file_history / I-039 restore_file：路径版本链与单文件恢复（AC-30）。

use crate::executor::GateInfo;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{ContentRef, SnapshotData, SqlQuery, SqlValue, TrashRequest, WriteFileInput};
use crate::risk::{assess_risk, RiskAssessment, RiskInput};
use crate::types::{EffectOp, FileVersion};

/// 文件路径输入（_06 I-038 契约类型 FilePath）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePath {
    pub path: String,
}

use serde::{Deserialize, Serialize};

/// 路径全部版本：每次执行一条（该次执行后的文件状态；删除条目取其前像=最后存在版本）。
pub fn file_history(
    input: &FilePath,
    output: &mut Vec<FileVersion>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT j.seq, j.execution_id, j.op, j.before_ref, j.after_ref, j.created_at, e.command \
              FROM rollback_journal j JOIN executions e ON e.id = j.execution_id \
              WHERE j.path = ?1 OR j.extra_path = ?1 ORDER BY j.seq ASC"
            .into(),
        params: vec![SqlValue::Text(input.path.clone())],
    };
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("file_history", "版本链读取失败");
        return false;
    }
    let mut versions = Vec::new();
    for i in 0..rows.len() {
        let seq = rows.int(i, "seq").unwrap_or(0);
        let exec = rows.text(i, "execution_id").unwrap_or("").to_string();
        let op = match rows.text(i, "op") {
            Some("created") => EffectOp::Created,
            Some("modified") => EffectOp::Modified,
            Some("deleted") => EffectOp::Deleted,
            _ => EffectOp::Renamed,
        };
        let at = rows.int(i, "created_at").unwrap_or(0);
        let command = rows.text(i, "command").unwrap_or("").to_string();
        let after = rows.text(i, "after_ref").map(str::to_string);
        let before = rows.text(i, "before_ref").map(str::to_string);
        // 版本 = 该次执行后的状态；删除条目无后像 → 取前像（删除前的最后版本）
        let cref = after.or(before);
        if let Some(content_ref) = cref {
            versions.push(FileVersion {
                seq,
                execution_id: exec,
                op,
                content_ref,
                at,
                command,
            });
        }
    }
    // 连续相同内容去重（多次执行内容未变时只留最早版本）
    versions.dedup_by(|a, b| a.content_ref == b.content_ref);
    *output = versions;
    true
}

/// 恢复报告（_06 I-039 契约类型 FileVersionRef → RestoreReport）。
/// 取版本内容直接替换现文件（现文件先入回收站），恢复动作过风险闸门留痕。
pub fn restore_file(
    input: &crate::types::FileVersionRef,
    output: &mut crate::types::RestoreReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut versions = Vec::new();
    let fp = FilePath {
        path: input.path.clone(),
    };
    if !file_history(&fp, &mut versions, context, metrics, evolution) {
        evolution.propagate("restore_file", "版本链读取失败");
        return false;
    }
    let Some(version) = versions.iter().find(|v| v.seq == input.seq) else {
        evolution.fail(
            "IAC-1604",
            format!("行号 {} 不是该文件的可用版本", input.seq),
        );
        return false;
    };
    let mut content = SnapshotData::default();
    if !context.deps.snap.snapstore_fetch(
        &ContentRef {
            reference: version.content_ref.clone(),
        },
        &mut content,
        context,
        metrics,
        evolution,
    ) {
        evolution.propagate("restore_file", "版本内容已被回收（IAC-1602）");
        return false;
    }
    // 闸门：恢复=显式覆盖写，风险评估留痕；用户显式发起视同直输（弱提醒不拦截）
    let mut risk = RiskAssessment::default();
    let rin = RiskInput {
        command: format!("iac file restore {}", input.path),
        cwd: None,
    };
    if !assess_risk(&rin, &mut risk, context, metrics, evolution) {
        evolution.propagate("restore_file", "恢复闸门评估失败");
        return false;
    }
    let gate = GateInfo {
        level: risk.level,
        action: crate::types::GateAction::WeakWarning,
        origin: crate::executor::CommandOrigin::Direct,
        actor_tag: context.session.actor.as_tag(),
    };
    let path = std::path::PathBuf::from(&input.path);
    if path.exists() {
        let mut ack = crate::ports::TrashAck::default();
        if !context.deps.snap.trash_file(
            &TrashRequest {
                abs_path: path.clone(),
                tag: format!("restore-{}", input.seq),
            },
            &mut ack,
            context,
            metrics,
            evolution,
        ) {
            evolution.propagate("restore_file", "现文件入回收站失败");
            return false;
        }
    }
    if !context.deps.snap.write_file(
        &WriteFileInput {
            path,
            bytes: content.bytes,
        },
        context,
        metrics,
        evolution,
    ) {
        evolution.propagate("restore_file", "恢复写入失败");
        return false;
    }
    metrics.counter("rollback.restored", 1);
    *output = crate::types::RestoreReport {
        path: input.path.clone(),
        restored_from: input.seq,
        gate,
    };
    true
}
