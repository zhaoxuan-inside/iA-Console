//! 风险域（词典：RiskAssessment / risk_gate）。

pub mod assess;
pub mod rules;
pub mod ruleset;

pub use assess::{assess_risk, builtin_rules};
pub use rules::match_risk_rules;

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

/// 一条风险规则：pattern 为命令串子串匹配（确定性、零依赖；误报由 note 解释，MVP 边界）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskRule {
    pub id: String,
    pub pattern: String,
    pub level: RiskLevel,
    /// 命中时展示的影响面/差异说明。
    pub note: String,
}

/// 风险匹配输入：命令串 + 规则集（规则集由 assess_risk 装载注入，本函数保持纯函数）。
#[derive(Debug, Clone, Default)]
pub struct RiskRulesInput {
    pub command: String,
    pub rules: Vec<RiskRule>,
}

/// 风险评估输出（词典：RiskAssessment）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    /// 命中的规则 id 列表。
    pub hits: Vec<String>,
    /// 影响面说明（命中规则 note 汇总，弱提醒展示用）。
    pub impact: String,
    /// 翻译/Agent 路径的确认要求（人类直输恒走弱提醒，见 AC-04）。
    pub requires_confirm: bool,
}

impl Default for RiskAssessment {
    fn default() -> Self {
        Self {
            level: RiskLevel::Safe,
            hits: Vec::new(),
            impact: String::new(),
            requires_confirm: false,
        }
    }
}

/// 闸门执行输入（词典：RiskInput 归并进此处）。
#[derive(Debug, Clone, Default)]
pub struct RiskInput {
    pub command: String,
    pub cwd: Option<String>,
}
