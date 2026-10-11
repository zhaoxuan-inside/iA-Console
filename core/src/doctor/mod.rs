//! 错误医生域（词典：Diagnosis/FixProposal）。静态快速通道 + LLM 证据链诊断。

pub mod handler;

pub use handler::{build_failure_context, diagnose_failure, parse_diagnosis_response, DiagnosePromptBundle};

use serde::{Deserialize, Serialize};

/// 候选修复命令（采纳后走同一闸门）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixProposal {
    pub command: String,
    pub explanation: String,
}

/// 结构化诊断（AC-03：根因须引用 stderr 证据；AC-15：明示无自动回滚）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub root_cause: String,
    /// stderr 原文引用（证据链）。
    pub evidence: Vec<String>,
    /// 0.0~1.0。
    pub confidence: f32,
    pub fix_proposals: Vec<FixProposal>,
    /// v1 恒为无自动回滚提示（AC-15）。
    pub rollback_note: String,
    /// 无 stderr 证据引用的假设标记（幻觉约束）。
    pub speculation: bool,
}

/// 诊断输入：指定执行记录，缺省=最近一次失败。
#[derive(Debug, Clone, Default)]
pub struct DiagnoseInput {
    pub execution_id: Option<String>,
}
