//! 执行记录与审计（域 7）：全量输出入库（Q45）+ 天数/容量双限 LRU 与手动清理导出（Q47）。

pub mod store;

use serde::{Deserialize, Serialize};

/// 执行状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecStatus {
    #[default]
    Ok,
    Failed,
    Timeout,
    Rejected,
}

/// 执行记录全行（executions 表领域形态；stdout_full/stderr_full=全量输出，Q45）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub argv: Vec<String>,
    pub cwd: String,
    pub shell: String,
    pub host: String,
    pub actor: String,
    pub exit_code: Option<i32>,
    pub status: ExecStatus,
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
    #[serde(default)]
    pub stdout_full: String,
    #[serde(default)]
    pub stderr_full: String,
    pub started_at: i64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_ref: Option<String>,
}

/// 历史检索过滤（iac_history_query）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ExecStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
    /// 检索结果是否携带全量输出（缺省仅摘要；Q45：存储全量≠注入全量）。
    #[serde(default)]
    pub with_full_output: bool,
}

fn default_limit() -> u32 {
    50
}

/// 历史分页结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryPage {
    pub total: u64,
    pub items: Vec<ExecutionRecord>,
}

/// 保留清理输入（Q47 双限 LRU；`dry_run` 只报告将清退多少）。
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct HistoryCleanInput {
    #[serde(default)]
    pub dry_run: bool,
}

/// 清理报告。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct HistoryCleanReport {
    pub removed: u64,
    pub remaining: u64,
    pub bytes_freed: u64,
}

/// 导出归档输入（Q47 `ia-history export`；path 为目标 JSONL 文件）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryExportInput {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

/// 导出报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryExportReport {
    pub exported: u64,
    pub path: String,
}
