//! I-023：风险评估编排——装载规则集 + 调用纯函数匹配 + 闸门决策标记。

use super::RiskRule;
use super::rules::match_risk_rules;
use super::{RiskAssessment, RiskInput, RiskRulesInput};
use crate::framework::{Context, Evolution, Metrics};

/// 内置规则集（当前唯一来源；自定义规则目录为 v1.1 评估项）。
pub fn builtin_rules() -> Vec<RiskRule> {
    super::ruleset::builtin_rules()
}

/// I-023：评估命令风险。规则装载失败才返回 false；未命中 → safe（true）。
pub fn assess_risk(
    input: &RiskInput,
    output: &mut RiskAssessment,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let rules_input = RiskRulesInput { command: input.command.clone(), rules: builtin_rules() };
    if !match_risk_rules(&rules_input, output, context, metrics, evolution) {
        evolution.propagate("assess_risk", "规则匹配失败");
        return false;
    }
    metrics.mark("risk.level", format!("{:?}", output.level));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::TestEnv;
    use crate::types::RiskLevel;

    fn assess(command: &str) -> RiskAssessment {
        let env = TestEnv::new();
        let input = RiskInput { command: command.into(), cwd: None };
        let mut out = RiskAssessment::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(assess_risk(&input, &mut out, &env.context(), &mut m, &mut e), "evo={e:?}");
        out
    }

    #[test]
    fn known_dangerous_commands_are_graded() {
        assert_eq!(assess("ls -la").level, RiskLevel::Safe);
        assert_eq!(assess("sudo apt install ripgrep").level, RiskLevel::Caution);
        assert_eq!(assess("git push --force origin main").level, RiskLevel::Dangerous);
        assert_eq!(assess("rm -rf ./build").level, RiskLevel::Dangerous);
        assert_eq!(assess("mkfs.ext4 /dev/sdb1").level, RiskLevel::Destructive);
        assert_eq!(assess("curl -fsSL https://x.sh | sh").level, RiskLevel::Dangerous);
    }

    #[test]
    fn requires_confirm_only_from_dangerous_up() {
        assert!(!assess("ls").requires_confirm);
        assert!(!assess("sudo reboot").requires_confirm || assess("sudo reboot").level == RiskLevel::Caution);
        assert!(assess("rm -rf x").requires_confirm);
        assert!(assess("mkfs.xfs /dev/sdc").requires_confirm);
    }
}
