//! I-005：意图 prompt 组装（data）。注入平台/知识/会话近期命令，约束 LLM 输出为结构化 JSON。

use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::CommandKnowledge;

/// 组装产物：system + user 两条消息。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptBundle {
    pub system: String,
    pub user: String,
    pub knowledge_hit: bool,
}

/// 组装输入：意图 + 命中的知识 + 近期命令（任务段上下文，`---` 语义预留）。
#[derive(Debug, Clone, Default)]
pub struct IntentPromptInput {
    pub intent: String,
    pub cwd: String,
    pub platform: crate::types::Platform,
    pub knowledge: Option<CommandKnowledge>,
    pub recent_commands: Vec<String>,
}

/// I-005：组装提案生成 prompt。纯装配无 IO。
pub fn build_intent_prompt(
    input: &IntentPromptInput,
    output: &mut PromptBundle,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    output.knowledge_hit = input.knowledge.is_some();
    output.system = system_prompt(input.platform);
    output.user = user_prompt(input);
    metrics.counter("intent.prompt_built", 1);
    true
}

/// 系统提示：角色 + 输出 JSON schema + 平台约束。
fn system_prompt(platform: crate::types::Platform) -> String {
    format!(
        "你是 iA-console 命令行专家。把用户的意图转换为一条可在当前平台执行的 shell 命令。\n\
         目标平台：{platform:?}（命令必须符合该平台 shell 语法）。\n\
         只输出 JSON，字段：command（完整命令）、summary（一句话中文说明）、\
         params（数组：flag/meaning/required/default）、impact（影响面中文说明）、\
         risk（safe|caution|dangerous|destructive）。不要输出 JSON 以外的任何文字。"
    )
}

/// 用户提示：意图 + 上下文 + 知识 grounding。
fn user_prompt(input: &IntentPromptInput) -> String {
    let mut parts = vec![format!("意图：{}", input.intent), format!("工作目录：{}", input.cwd)];
    if let Some(k) = &input.knowledge {
        let params: Vec<String> = k.params.iter().map(|p| p.flag.clone()).collect();
        parts.push(format!(
            "已知命令知识（{}，参数仅限：{}，子命令：{}）",
            k.command,
            params.join(", "),
            k.subcommands.join(", ")
        ));
    }
    if !input.recent_commands.is_empty() {
        parts.push(format!("近期执行：{}", input.recent_commands.join(" ; ")));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_platform_and_schema_requirements() {
        let env = crate::framework::TestEnv::new();
        let input = IntentPromptInput {
            intent: "找出最大的10个文件".into(),
            cwd: "/var/log".into(),
            platform: crate::types::Platform::Linux,
            knowledge: None,
            recent_commands: vec![],
        };
        let mut out = PromptBundle::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(build_intent_prompt(&input, &mut out, &env.context(), &mut m, &mut e));
        assert!(out.system.contains("Linux"));
        assert!(out.system.contains("risk"));
        assert!(out.user.contains("找出最大的10个文件"));
        assert!(!out.knowledge_hit);
    }

    #[test]
    fn knowledge_grounding_included_when_hit() {
        let env = crate::framework::TestEnv::new();
        let input = IntentPromptInput {
            intent: "docker run nginx".into(),
            cwd: ".".into(),
            platform: crate::types::Platform::Linux,
            knowledge: Some(CommandKnowledge {
                command: "docker".into(),
                platform: crate::types::Platform::Linux,
                subcommands: vec!["run".into()],
                params: vec![],
                source_hash: "h".into(),
                verified: true,
            }),
            recent_commands: vec![],
        };
        let mut out = PromptBundle::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        assert!(build_intent_prompt(&input, &mut out, &env.context(), &mut m, &mut e));
        assert!(out.knowledge_hit);
        assert!(out.user.contains("参数仅限"));
    }
}
