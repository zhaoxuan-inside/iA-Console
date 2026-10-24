//! I-005：意图 prompt 组装（纯函数，无 IO）。
//! 重建变化：多候选输出（Q17）、上下文注入扩展（Q20：cwd+平台/Shell+最近 10 条命令+段内对话轨+钉住备注）、
//! 透明可查（context_json 快照由 handler 保存）。

use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::CommandKnowledge;

/// 组装产物：system + user 两条消息。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptBundle {
    pub system: String,
    pub user: String,
    pub knowledge_hit: bool,
}

/// 上下文注入快照（Q20 透明面：与 prompt 同源，保存进 proposal.context_json）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InjectContext {
    pub cwd: String,
    pub platform: String,
    pub shell: String,
    /// 最近 10 条命令（当前任务段内）。
    pub recent_commands: Vec<String>,
    /// 段内对话轨（用户问题/AI 回答摘要）。
    pub dialogue: Vec<String>,
    /// 钉住备注（跨 `---` 段，Q33）。
    pub pinned_notes: Vec<String>,
}

/// 组装输入。
#[derive(Debug, Clone, Default)]
pub struct IntentPromptInput {
    pub intent: String,
    pub context: InjectContext,
    pub knowledge: Option<CommandKnowledge>,
}

/// I-005：组装提案生成 prompt（多候选 2~3 个，Q17）。
pub fn build_intent_prompt(
    input: &IntentPromptInput,
    output: &mut PromptBundle,
    _context: &Context,
    metrics: &mut Metrics,
    _evolution: &mut Evolution,
) -> bool {
    output.knowledge_hit = input.knowledge.is_some();
    output.system = system_prompt(&input.context.platform);
    output.user = user_prompt(input);
    metrics.counter("intent.prompt_built", 1);
    true
}

/// 系统提示：角色 + 多候选 JSON schema + 平台约束 + 风险自评（Q34：AI 为辅）。
fn system_prompt(platform: &str) -> String {
    format!(
        "你是 iA-console 命令行专家。把用户的意图转换为可在当前平台执行的 shell 命令。\n\
         目标平台：{platform}（命令必须符合该平台 shell 语法）。\n\
         给出 2~3 个候选方案，按平台惯例程度排序（最惯用的在前）。\n\
         只输出 JSON，格式：{{\"candidates\":[{{\"command\":\"完整命令\",\"summary\":\"一句话中文说明\",\
         \"conventional\":true,\"risk\":\"safe|caution|dangerous|destructive\",\
         \"params\":[{{\"flag\":\"-x\",\"meaning\":\"白话解释\",\"required\":false,\"default\":null}}],\
         \"impact\":\"影响面中文说明\"}}]}}。不要输出 JSON 以外的任何文字。"
    )
}

/// 用户提示：意图 + 上下文注入（Q20）+ 知识 grounding。
fn user_prompt(input: &IntentPromptInput) -> String {
    let ctx = &input.context;
    let mut parts = vec![format!("意图：{}", input.intent), format!("工作目录：{}", ctx.cwd)];
    if !ctx.shell.is_empty() {
        parts.push(format!("Shell：{}", ctx.shell));
    }
    if let Some(k) = &input.knowledge {
        let params: Vec<String> = k.params.iter().map(|p| p.flag.clone()).collect();
        parts.push(format!(
            "已知命令知识（{}，参数仅限：{}，子命令：{}）",
            k.command,
            params.join(", "),
            k.subcommands.join(", ")
        ));
    }
    if !ctx.recent_commands.is_empty() {
        parts.push(format!("近期执行：{}", ctx.recent_commands.join(" ; ")));
    }
    if !ctx.dialogue.is_empty() {
        parts.push(format!("本段对话：{}", ctx.dialogue.join(" ; ")));
    }
    if !ctx.pinned_notes.is_empty() {
        parts.push(format!("钉住备注：{}", ctx.pinned_notes.join(" ; ")));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_multi_candidate_schema_and_context() {
        let env = crate::framework::TestEnv::new();
        let mut out = PromptBundle::default();
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        let input = IntentPromptInput {
            intent: "找大文件".into(),
            context: InjectContext {
                cwd: "/data".into(),
                platform: "linux".into(),
                shell: "bash".into(),
                recent_commands: vec!["cd /data".into()],
                dialogue: vec!["用户：磁盘满了".into()],
                pinned_notes: vec!["在调 nginx".into()],
            },
            knowledge: None,
        };
        assert!(build_intent_prompt(&input, &mut out, &env.context(), &mut m, &mut e));
        assert!(out.system.contains("candidates"), "system 必须要求多候选（Q17）");
        assert!(out.system.contains("conventional"), "排序标记（Q23）");
        assert!(out.user.contains("钉住备注"), "注入透明面（Q20/Q33）");
        assert!(out.user.contains("近期执行"));
    }
}
