//! 意图 → 提案（域 1）：AI 生成多候选（Q17）、逐参数解释（AC-01）、上下文注入（Q20）。

pub mod handler;
pub mod parse;
pub mod prompt;
pub mod store;
pub mod translate;

pub use handler::{default_provider, handle_ask, handle_explain, load_proposal, mark_proposal_status};
pub use parse::parse_explain_response;

use crate::types::{Platform, RiskLevel};
use serde::{Deserialize, Serialize};

/// 逐参数白话解释（提案卡 `e` 展开区内容，AC-01/AC-35）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParamSpecDoc {
    #[serde(default)]
    pub params: Vec<crate::knowledge::ParamSpec>,
    #[serde(default)]
    pub impact: String,
    #[serde(default)]
    pub summary: String,
}

/// 提案候选（Q17 多候选 2~3 个；Q23 排序=平台惯例>低风险>历史使用频率）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub command: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub summary: String,
    pub risk_level: RiskLevel,
    /// 平台惯例标记（排序第一优先键，Q23）。
    #[serde(default)]
    pub conventional: bool,
    #[serde(default)]
    pub explanation: ParamSpecDoc,
}

/// 提案状态机：proposed → confirmed → executed / rejected / cancelled。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    #[default]
    Proposed,
    Confirmed,
    Executed,
    Rejected,
    Cancelled,
}

/// 提案（iac_ask 返回）。多候选按 Q23 排序截断至 3；
/// `context_json` 记录本次注入的上下文快照（Q20 透明可查，客户端 `e` 展开呈现）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub session_id: String,
    pub intent_text: String,
    pub candidates: Vec<Candidate>,
    /// 活动候选下标（客户端上下键切换的持久位）。
    #[serde(default)]
    pub active: usize,
    pub platform: Platform,
    pub verified: bool,
    pub status: ProposalStatus,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_json: Option<String>,
}

/// 意图输入（iac_ask 参数）。cwd/shell 由客户端会话携带（Q20 注入源）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AskInput {
    pub intent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

/// 解释输入（iac_explain 参数）：`??ls` 或提案 `e` 展开触发。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExplainInput {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}
