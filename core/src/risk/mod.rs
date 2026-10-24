//! 风险评估（域 8）：静态规则库为主判定（离线可用，Q34）。

pub mod rules;
pub mod ruleset;

use crate::types::RiskLevel;

/// 静态规则评估：取全部命中规则的最高等级（纯函数）。
pub fn assess_risk(command: &str) -> RiskLevel {
    ruleset::assess_static(command)
}

/// 合成风险（Q34：静态为主、AI 自评为辅、冲突取更高）。
pub fn combine_risk(static_level: RiskLevel, ai_level: Option<RiskLevel>) -> RiskLevel {
    match ai_level {
        Some(ai) if ai > static_level => ai,
        _ => static_level,
    }
}
