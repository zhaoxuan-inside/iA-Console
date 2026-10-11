//! 记录域（词典：ExecutionRecord/AuditEntry）。store.rs 数据方法，query.rs 检索。

pub mod store;

pub use store::{append_audit_entry, get_execution_detail, query_history, record_execution};

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

/// 执行状态（词典：ExecutionRecord.status）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStatus {
    Ok,
    Failed,
    Timeout,
    Cancelled,
    Rejected,
}

/// 一次命令执行的全量留痕（词典：ExecutionRecord）。
/// 截断与落盘引用由调用方（executor）完成：stdout_summary ≤ exec.stdout_summary_max，
/// 原文超 exec.output_raw_max 时写 outputs/ 并填 *_ref。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    pub command: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub shell: String,
    pub host: String,
    /// human|agent:<client>
    pub actor: String,
    pub exit_code: Option<i32>,
    pub status: ExecStatus,
    pub risk_level: RiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_ref: Option<String>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// v2 回滚：执行前像 manifest 在快照仓库的引用
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_ref: Option<String>,
}

/// 审计条目（词典：AuditEntry）：与执行记录互补，记录非执行类操作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// human|agent:<client>|system
    pub actor: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

/// 执行 ID 引用。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExecutionId {
    pub id: String,
}

/// 历史检索过滤（AC-13：万条级秒回）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HistoryFilter {
    /// 命令/输出摘要关键词（LIKE）
    pub q: Option<String>,
    pub cwd: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub status: Option<ExecStatus>,
    pub actor: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

/// 检索结果页。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistoryPage {
    pub total: i64,
    pub items: Vec<ExecutionRecord>,
}
