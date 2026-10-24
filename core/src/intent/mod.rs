//! 意图域（词典：Intent/Proposal）。prompt/parse 为数据方法，handler 为编排。

pub mod handler;
pub mod parse;
pub mod prompt;
pub mod store;
pub mod translate;
pub mod translation_table;

pub use handler::{build_explain_doc, default_provider, handle_ask, handle_explain, load_proposal, mark_proposal_status};
pub use translate::translate_command;

use crate::knowledge::ParamSpec;
use crate::types::{Platform, RiskLevel};
use serde::{Deserialize, Serialize};

/// 用户意图输入（AC-01：1~2000 字符校验在编排内）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AskInput {
    pub intent: String,
    pub cwd: Option<String>,
    pub platform: Option<Platform>,
}

/// 参数文档（词典：ParamSpecDoc，提案 explanation 的主体）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParamSpecDoc {
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub impact: String,
    #[serde(default)]
    pub summary: String,
}

/// 提案状态机（词典：Proposal）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Proposed,
    Confirmed,
    Executed,
    Rejected,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalStatus::Proposed => "proposed",
            ProposalStatus::Confirmed => "confirmed",
            ProposalStatus::Executed => "executed",
            ProposalStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "confirmed" => ProposalStatus::Confirmed,
            "executed" => ProposalStatus::Executed,
            "rejected" => ProposalStatus::Rejected,
            _ => ProposalStatus::Proposed,
        }
    }
}

/// 提案（词典：Proposal，proposals 表同构）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    pub intent_text: String,
    pub command: String,
    pub argv: Vec<String>,
    pub explanation: ParamSpecDoc,
    pub risk_level: RiskLevel,
    pub platform: Platform,
    pub verified: bool,
    pub status: ProposalStatus,
    pub created_at: i64,
}

/// LLM 解析后的提案草稿（parse_proposal_response 产出；落库前由编排补齐会话/风险/状态）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProposalDraft {
    pub command: String,
    pub argv: Vec<String>,
    pub explanation: ParamSpecDoc,
}

/// 提案 ID 引用。
#[derive(Debug, Clone, Default)]
pub struct ProposalId {
    pub id: String,
}

/// 参数解释输入（AC-02）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExplainInput {
    pub command: String,
    pub platform: Option<Platform>,
}
