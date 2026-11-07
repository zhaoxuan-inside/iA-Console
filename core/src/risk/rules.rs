//! I-001：风险规则匹配（algorithm，纯函数）。同一输入恒同一输出（闸门终检复验依赖确定性）。

use super::{RiskAssessment, RiskRulesInput};
use crate::framework::{Evolution, Metrics};
use crate::types::RiskLevel;

/// I-001：按规则集匹配命令，取最高风险等级；未命中任何规则 → safe（业务"否"仍属成功）。
pub fn match_risk_rules(
    input: &RiskRulesInput,
    output: &mut RiskAssessment,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    let mut level = RiskLevel::Safe;
    let mut hits = Vec::new();
    let mut notes = Vec::new();
    for rule in &input.rules {
        if input.command.contains(&rule.pattern) {
            if rule.level > level {
                level = rule.level;
            }
            hits.push(rule.id.clone());
            notes.push(format!("[{}] {}", rule.id, rule.note));
        }
    }
    output.level = level;
    output.hits = hits;
    output.impact = notes.join("\n");
    output.requires_confirm = level >= RiskLevel::Dangerous;
    metrics.counter("risk.matched", output.hits.len() as u64);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RiskLevel;

    fn rule(id: &str, pattern: &str, level: RiskLevel) -> crate::risk::RiskRule {
        crate::risk::RiskRule {
            id: id.to_string(),
            pattern: pattern.to_string(),
            level,
            note: format!("note-{id}"),
        }
    }

    fn match_cmd(command: &str, rules: Vec<crate::risk::RiskRule>) -> RiskAssessment {
        let input = RiskRulesInput {
            command: command.into(),
            rules,
        };
        let mut out = RiskAssessment::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(match_risk_rules(&input, &mut out, &mut m, &mut e));
        out
    }

    #[test]
    fn no_match_is_safe() {
        let out = match_cmd("ls -la", vec![rule("r1", "rm -rf", RiskLevel::Destructive)]);
        assert_eq!(out.level, RiskLevel::Safe);
        assert!(out.hits.is_empty());
        assert!(!out.requires_confirm);
    }

    #[test]
    fn highest_level_wins_and_notes_aggregate() {
        let rules = vec![
            rule("r-caution", "sudo", RiskLevel::Caution),
            rule("r-danger", "rm -rf", RiskLevel::Dangerous),
            rule("r-destr", "mkfs", RiskLevel::Destructive),
        ];
        let out = match_cmd("sudo mkfs.ext4 /dev/sda1", rules);
        assert_eq!(out.level, RiskLevel::Destructive);
        assert_eq!(out.hits, vec!["r-caution", "r-destr"]);
        assert_eq!(out.hits.len(), 2);
        assert!(out.impact.contains("note-r-caution"));
        assert!(out.impact.contains("note-r-destr"));
        assert!(out.requires_confirm);
    }

    #[test]
    fn deterministic_for_same_input() {
        let rules = vec![rule("r", "rm -rf", RiskLevel::Dangerous)];
        let a = match_cmd("rm -rf /tmp/x", rules.clone());
        let b = match_cmd("rm -rf /tmp/x", rules);
        assert_eq!(a, b);
    }
}
