//! I-042 assemble_script（algorithm，纯函数）：命令序列 → sh / PowerShell 双版本脚本。
//! 确定性拼接零 AI（AC-38）；自动注入 `set -euo pipefail` / `$ErrorActionPreference='Stop'` 头。

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// 目标脚本形态（词典：ScriptTarget）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptTarget {
    Sh,
    Bat,
    #[default]
    Both,
}

/// 待装配的命令条目（collect_commands 输出）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptCommand {
    pub seq: i64,
    pub command: String,
}

/// 装配输入：命令序列 + 目标形态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptAssemblyInput {
    pub entries: Vec<ScriptCommand>,
    pub target: ScriptTarget,
    pub max_risk: RiskLevel,
}

/// 装配输出：name_hint + 各平台正文（多行字符串，含平台头部）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssembledScript {
    pub name_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sh_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ps_body: Option<String>,
    pub max_risk: RiskLevel,
}

impl Default for AssembledScript {
    fn default() -> Self {
        Self {
            name_hint: String::new(),
            sh_body: None,
            ps_body: None,
            max_risk: RiskLevel::Safe,
        }
    }
}

/// 确定性拼接：条目原样保留（方言混合不改写），头部按平台注入，条目带 `# [seq]` 注释行。
pub fn assemble_script(
    input: &ScriptAssemblyInput,
    output: &mut AssembledScript,
    _metrics: &mut crate::framework::Metrics,
) -> bool {
    let mut sh = String::from("#!/bin/sh\nset -euo pipefail\n");
    let mut ps = String::from("$ErrorActionPreference = 'Stop'\n");
    for e in &input.entries {
        let _ = writeln!(sh, "# [{}] {}", e.seq, e.command);
        let _ = writeln!(sh, "{}", e.command);
        let _ = writeln!(ps, "# [{}] {}", e.seq, e.command);
        let _ = writeln!(ps, "{}", e.command);
    }
    let hint = input
        .entries
        .first()
        .map(|e| {
            e.command
                .split_whitespace()
                .next()
                .unwrap_or("iac")
                .to_string()
        })
        .unwrap_or_else(|| "iac".to_string());
    let (sh_body, ps_body) = match input.target {
        ScriptTarget::Sh => (Some(sh), None),
        ScriptTarget::Bat => (None, Some(ps)),
        ScriptTarget::Both => (Some(sh), Some(ps)),
    };
    *output = AssembledScript {
        name_hint: hint,
        sh_body,
        ps_body,
        max_risk: input.max_risk,
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Metrics;

    fn sample() -> ScriptAssemblyInput {
        ScriptAssemblyInput {
            entries: vec![
                ScriptCommand {
                    seq: 1,
                    command: "mkdir -p build".into(),
                },
                ScriptCommand {
                    seq: 2,
                    command: "cp a.txt build/".into(),
                },
            ],
            target: ScriptTarget::Both,
            max_risk: RiskLevel::Safe,
        }
    }

    #[test]
    fn both_versions_with_headers() {
        let mut out = AssembledScript::default();
        assert!(assemble_script(
            &sample(),
            &mut out,
            &mut Metrics::default()
        ));
        let sh = out.sh_body.unwrap();
        let ps = out.ps_body.unwrap();
        assert!(sh.starts_with("#!/bin/sh\nset -euo pipefail\n"));
        assert!(ps.starts_with("$ErrorActionPreference = 'Stop'\n"));
        assert!(sh.contains("# [1] mkdir -p build"));
        assert!(sh.contains("cp a.txt build/"));
        assert_eq!(out.name_hint, "mkdir");
    }

    #[test]
    fn single_target_omits_other() {
        let mut input = sample();
        input.target = ScriptTarget::Sh;
        let mut out = AssembledScript::default();
        assert!(assemble_script(&input, &mut out, &mut Metrics::default()));
        assert!(out.sh_body.is_some() && out.ps_body.is_none());
    }

    #[test]
    fn dialect_mix_kept_verbatim() {
        let mut input = sample();
        input.entries.push(ScriptCommand {
            seq: 3,
            command: "Get-ChildItem build".into(),
        });
        let mut out = AssembledScript::default();
        assert!(assemble_script(&input, &mut out, &mut Metrics::default()));
        assert!(
            out.sh_body.unwrap().contains("Get-ChildItem build"),
            "方言混合原样保留"
        );
    }
}
