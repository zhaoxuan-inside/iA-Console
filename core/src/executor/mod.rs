//! 执行编排（域 1/8）：闸门矩阵（Q34~Q37）→ 执行 → 三账留痕（Q45）→ 失败兜底（Q18）。

pub mod handler;

use crate::doctor::Diagnosis;
use crate::history::ExecStatus;
use crate::types::{EffectEntry, GateAction, RiskLevel};
use serde::{Deserialize, Serialize};

/// 命令来源（闸门矩阵的列维度）：Direct=人类直输；Translated=翻译/Agent 视同路径；Proposal=AI 提案。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandOrigin {
    #[default]
    Direct,
    Translated,
    Proposal,
}

/// 执行请求（iac_execute 参数）。
/// - `command`：直输命令（Direct/Translated 路径）；
/// - `proposal_id` + `candidate_index`：提案候选确认执行（多候选 Q17，缺省 active 位）；
/// - `confirmed`：dangerous 及以上的武装态确认（Q21：二次 Enter 后置位；Agent 确认标志同字段）；
/// - `confirm_word`：destructive 确认词（Q37）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExecuteInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CommandOrigin>,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_word: Option<String>,
}

/// 闸门判定信息（执行记录与客户端呈现共用）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GateInfo {
    pub level: RiskLevel,
    pub action: GateAction,
    pub origin: CommandOrigin,
    pub actor_tag: String,
}

/// 执行结果（iac_execute 返回）。
/// 全量输出（Q45）入库在 handler 内完成，此处仅携带摘要；
/// `offer_help`：执行失败时就地「要不要 AI 帮忙？」一键转诊断/提案（Q18）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub execution_id: String,
    pub exit_code: Option<i32>,
    pub status: ExecStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Diagnosis>,
    #[serde(default)]
    pub gate: GateInfo,
    #[serde(default)]
    pub offer_help: bool,
    /// 本次执行产生的效果日志条目（域 15，回滚点呈现用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectEntry>,
}

/// 闸门矩阵（Q34~Q37 纯函数）：风险等级 × 来源 × 发起方 → 闸门动作。
/// - 人类直输：dangerous 起 WeakWarning（弱提醒不拦截）；
/// - 翻译/提案路径：dangerous → RequireDoubleEnter，destructive → RequireConfirmWord；
/// - Agent 且未带确认标志：dangerous 起 RejectAgentUnconfirmed（闸门与人一致）。
pub fn gate_decision(
    level: RiskLevel,
    origin: CommandOrigin,
    is_agent: bool,
    confirmed: bool,
) -> GateAction {
    if origin == CommandOrigin::Direct && !is_agent {
        if level >= RiskLevel::Caution {
            GateAction::WeakWarning
        } else {
            GateAction::Pass
        }
    } else if is_agent {
        if level >= RiskLevel::Dangerous && !confirmed {
            GateAction::RejectAgentUnconfirmed
        } else {
            GateAction::Pass
        }
    } else if !confirmed {
        if level >= RiskLevel::Destructive {
            GateAction::RequireConfirmWord
        } else if level >= RiskLevel::Dangerous {
            GateAction::RequireDoubleEnter
        } else {
            GateAction::Pass
        }
    } else {
        GateAction::Pass
    }
}

/// destructive 确认词：固定语义词「确认」+ 命令尾段（客户端呈现提示，服务端只校验语义词）。
pub fn confirm_word_for(command: &str) -> String {
    let tail = command.split_whitespace().last().unwrap_or(command);
    format!("确认 {tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_direct_is_never_blocked() {
        assert_eq!(
            gate_decision(RiskLevel::Destructive, CommandOrigin::Direct, false, false),
            GateAction::WeakWarning
        );
        assert_eq!(
            gate_decision(RiskLevel::Safe, CommandOrigin::Direct, false, false),
            GateAction::Pass
        );
    }

    #[test]
    fn proposal_path_escalates_by_level() {
        assert_eq!(
            gate_decision(RiskLevel::Dangerous, CommandOrigin::Proposal, false, false),
            GateAction::RequireDoubleEnter
        );
        assert_eq!(
            gate_decision(RiskLevel::Destructive, CommandOrigin::Translated, false, false),
            GateAction::RequireConfirmWord
        );
        assert_eq!(
            gate_decision(RiskLevel::Dangerous, CommandOrigin::Proposal, false, true),
            GateAction::Pass
        );
    }

    #[test]
    fn agent_without_confirm_is_rejected_at_dangerous() {
        assert_eq!(
            gate_decision(RiskLevel::Dangerous, CommandOrigin::Translated, true, false),
            GateAction::RejectAgentUnconfirmed
        );
        assert_eq!(
            gate_decision(RiskLevel::Caution, CommandOrigin::Translated, true, false),
            GateAction::Pass
        );
        // Agent 带确认标志即视为确认完成（Q35/Q37：Agent 无法做二次 Enter，标志即确认）
        assert_eq!(
            gate_decision(RiskLevel::Destructive, CommandOrigin::Translated, true, true),
            GateAction::Pass
        );
    }
}
