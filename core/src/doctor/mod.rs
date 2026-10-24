//! 错误医生（域 3）：失败自动诊断（G3/AC-03）——静态快速通道 + LLM 证据链；
//! 交互形态按 Q18：失败后就地「要不要 AI 帮忙？」，一键转诊断与修复提案。

pub mod handler;

use serde::{Deserialize, Serialize};

/// 诊断输入（iac_diagnose；可由 execution_id 关联，也可直接携带现场）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiagnoseInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

/// 诊断证据：逐字引用 stderr/stdout 原文（AC-03）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: String,
    pub quote: String,
    pub note: String,
}

/// 修复提案候选：采纳后转提案走同一闸门（Q18 一键转提案）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FixProposal {
    pub command: String,
    #[serde(default)]
    pub explanation: String,
}

/// 诊断报告。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub root_cause: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub confidence: f32,
    #[serde(default)]
    pub fix_proposals: Vec<FixProposal>,
    #[serde(default)]
    pub rollback_note: String,
    /// 证据为空时的推测标记（LLM 证据链纪律）。
    #[serde(default)]
    pub speculation: bool,
}
