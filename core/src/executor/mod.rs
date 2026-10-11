//! 执行域（词典：ExecutionOutcome/GateInfo）。闸门语义见 AC-04/AC-35：人类直输弱提醒不拦截。

pub mod handler;

pub use handler::{confirm_and_execute, handle_execute};

use crate::doctor::Diagnosis;
use serde::{Deserialize, Serialize};

/// 闸门信息（弱提醒标记 + 确认要求 + 转换视图，_06 internal 契约）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateInfo {
    pub level: crate::types::RiskLevel,
    pub requires_confirm: bool,
    pub weak_warning: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion: Option<ConversionView>,
}

impl Default for GateInfo {
    fn default() -> Self {
        Self { level: crate::types::RiskLevel::Safe, requires_confirm: false, weak_warning: false, conversion: None }
    }
}

/// 转换视图（翻译行展示：灰阶原命令 + 译文）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionView {
    pub from_command: String,
    pub to_command: String,
}

/// 执行结果（词典：ExecutionOutcome）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub execution_id: String,
    pub exit_code: Option<i32>,
    pub status: crate::history::ExecStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Diagnosis>,
    pub gate: GateInfo,
}

/// 命令来源：决定确认路径（AC-04：人类直输弱提醒；翻译高危/Agent 恒确认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOrigin {
    Direct,
    Translated,
    Proposal,
}

/// 执行输入（_06 ExecuteInput；origin 为字段只增演进）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExecuteInput {
    pub command: Option<String>,
    pub proposal_id: Option<String>,
    pub session_id: Option<String>,
    pub confirm_dangerous: bool,
    pub timeout_ms: Option<u64>,
    pub origin: Option<CommandOrigin>,
}
