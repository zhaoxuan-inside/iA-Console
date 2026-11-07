//! 执行回滚（M7，v0.2.0）：效果日志方案（_02 Q6-S4）——不逐命令判断可逆性，
//! 执行前统一捕获前像入快照仓库，执行后 diff 生成效果日志（AC-29），
//! 逆序回放/单文件恢复/回收站/retention（AC-30~34）。
//!
//! 编排（capture/replay/restore）与数据（diff/file_history/list）经 SnapPort
//! 与 StorePort 触达外部组件；本模块不含任何直接 IO。

pub mod capture;
pub mod diff;
pub mod file_history;
pub mod list;
pub mod replay;

pub use capture::capture_pre_image;
pub use diff::diff_effects;
pub use file_history::{file_history, restore_file, FilePath};
pub use list::list_rollback_points;
pub use replay::rollback_to;

use crate::ports::{FileCaptured, SqlQuery, SqlValue};
use serde::{Deserialize, Serialize};

/// 捕获排除名：永不入前像（构建产物/VCS 内部/依赖目录）。
pub const CAPTURE_EXCLUDES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".cache",
    "__pycache__",
    ".venv",
];

/// 单文件入库上限（字节）：超过仅记指纹（AC-29 out_of_scope 语义来源之一）。
pub const CAPTURE_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// 一次捕获的入库总量上限（字节）。
pub const CAPTURE_MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// 前像清单文档（manifest 落快照仓库的 JSON 形态）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDoc {
    pub cwd: String,
    pub roots: Vec<String>,
    pub files: Vec<FileCaptured>,
}

/// 捕获范围请求（_06 I-035 契约类型 CaptureScope）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureScope {
    pub execution_hint: String,
    pub cwd: String,
    pub explicit_paths: Vec<String>,
    pub impact_paths: Vec<String>,
}

/// 按配置解析捕获根目录（CwdTree/Explicit/AllHome 三模式，词典：CaptureScope）。
pub fn resolve_roots(input: &CaptureScope, context: &crate::framework::Context) -> Vec<String> {
    use crate::types::CaptureScope as Mode;
    let cwd = input.cwd.clone();
    let mut roots = match context.config.rollback.capture_scope {
        Mode::CwdTree => vec![cwd.clone()],
        Mode::Explicit => Vec::new(),
        Mode::AllHome => vec![std::env::var("HOME").unwrap_or_else(|_| cwd.clone())],
    };
    for p in input.explicit_paths.iter().chain(input.impact_paths.iter()) {
        if !p.is_empty() && !roots.contains(p) {
            roots.push(p.clone());
        }
    }
    roots.retain(|r| !r.is_empty());
    roots
}

/// 下一个 journal seq（daemon 单写者串行化保证并发安全，AC-16）。
pub fn next_seq(
    context: &crate::framework::Context,
    metrics: &mut crate::framework::Metrics,
    evolution: &mut crate::framework::Evolution,
) -> Option<i64> {
    let mut rows = crate::ports::QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT COALESCE(MAX(seq),0) AS m FROM rollback_journal".into(),
        params: vec![],
    };
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        return None;
    }
    Some(rows.int(0, "m").unwrap_or(0) + 1)
}

/// journal 插入一条效果（op/scope 以小写串落库）。
pub fn journal_insert(
    entry: &EffectEntryRef,
    context: &crate::framework::Context,
    metrics: &mut crate::framework::Metrics,
    evolution: &mut crate::framework::Evolution,
) -> bool {
    let q = SqlQuery {
        sql: "INSERT INTO rollback_journal (seq, execution_id, path, op, before_ref, after_ref, extra_path, scope, created_at) VALUES (?,?,?,?,?,?,?,?,?)".into(),
        params: vec![
            SqlValue::Int(entry.seq),
            SqlValue::Text(entry.execution_id.clone()),
            SqlValue::Text(entry.path.clone()),
            SqlValue::Text(entry.op.clone()),
            entry.before_ref.clone().map(SqlValue::Text).unwrap_or(SqlValue::Null),
            entry.after_ref.clone().map(SqlValue::Text).unwrap_or(SqlValue::Null),
            entry.extra_path.clone().map(SqlValue::Text).unwrap_or(SqlValue::Null),
            SqlValue::Text(entry.scope.clone()),
            SqlValue::Int(entry.created_at),
        ],
    };
    let mut ack = crate::ports::ExecuteAck::default();
    context
        .deps
        .store
        .storage_execute(&q, &mut ack, context, metrics, evolution)
}

/// journal 行中间形态（字符串 op/scope，落库与领域类型互转用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntryRef {
    pub seq: i64,
    pub execution_id: String,
    pub path: String,
    pub op: String,
    pub before_ref: Option<String>,
    pub after_ref: Option<String>,
    pub extra_path: Option<String>,
    pub scope: String,
    pub created_at: i64,
}

use crate::types::{CaptureScopeFlag, EffectEntry, EffectOp, ManifestRef};

impl EffectEntryRef {
    /// 转领域类型（未知 op/scope 返回 None，防御性）。
    pub fn to_entry(&self) -> Option<EffectEntry> {
        let op = match self.op.as_str() {
            "created" => EffectOp::Created,
            "modified" => EffectOp::Modified,
            "deleted" => EffectOp::Deleted,
            "renamed" => EffectOp::Renamed,
            _ => return None,
        };
        let scope = match self.scope.as_str() {
            "in" => CaptureScopeFlag::In,
            "out_of_scope" => CaptureScopeFlag::OutOfScope,
            _ => return None,
        };
        Some(EffectEntry {
            seq: self.seq,
            execution_id: self.execution_id.clone(),
            path: self.path.clone(),
            op,
            before_ref: self.before_ref.clone(),
            after_ref: self.after_ref.clone(),
            extra_path: self.extra_path.clone(),
            scope,
            created_at: self.created_at,
        })
    }
}

/// 执行编排共用（executor/脚本工坊）：执行前捕获前像。回滚关闭或范围为空 → 返回 None。
pub fn pre_capture(
    cwd: &str,
    context: &crate::framework::Context,
    metrics: &mut crate::framework::Metrics,
    evolution: &mut crate::framework::Evolution,
) -> Option<ManifestRef> {
    let mut manifest = ManifestRef::default();
    let input = CaptureScope {
        execution_hint: String::new(),
        cwd: cwd.to_string(),
        explicit_paths: vec![],
        impact_paths: vec![],
    };
    if !capture_pre_image(&input, &mut manifest, context, metrics, evolution) {
        evolution.ok = true;
        metrics.counter("rollback.capture_failed", 1);
        return None;
    }
    if manifest.manifest_ref.is_empty() {
        return None;
    }
    Some(manifest)
}

/// 执行编排共用：执行后 diff 入 journal 并回填 executions.journal_ref（尽力而为不阻断）。
/// 返回本次落账的效果条目（ExecutionOutcome.effects 呈现用；diff 失败/无前像返回空）。
pub fn post_diff(
    manifest: &mut ManifestRef,
    execution_id: &str,
    context: &crate::framework::Context,
    metrics: &mut crate::framework::Metrics,
    evolution: &mut crate::framework::Evolution,
) -> Vec<EffectEntry> {
    if manifest.manifest_ref.is_empty() {
        return Vec::new();
    }
    manifest.execution_id = execution_id.to_string();
    let mut entries = Vec::new();
    if !diff_effects(manifest, &mut entries, context, metrics, evolution) {
        evolution.ok = true;
        metrics.counter("rollback.diff_failed", 1);
        return Vec::new();
    }
    entries
}
