//! 脚本工坊（域 16）：范围收集 → 确定性拼接（零 AI 默认）→ AI 精修（显式触发，AC-40）
//! → 命名注册（kind=script，name 全局唯一，AC-38）→ `ia-script run`/`iac run @名` 过同一闸门（AC-39）。

pub mod collect;

use crate::executor::ExecutionOutcome;
use serde::{Deserialize, Serialize};

/// 脚本来源范围（域 16：`---` 任务段或行号区间，US-21）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptSource {
    /// 最近一个 `---` 分隔符之后的任务段。
    LastSegment,
    /// 事件流行号区间（含端点）。
    LineRange { from: i64, to: i64 },
}

/// 制作输入（iac_script make）。`ai_refine` 显式触发 AI 精修（默认 false 零 AI，AC-38）；
/// `confirm=true` 时才注册入库，否则仅返回预览稿。
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptMakeInput {
    pub source: ScriptSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub ai_refine: bool,
    #[serde(default)]
    pub confirm: bool,
}

/// 制作输出：sh 与 PowerShell 双版本预览（AC-38 双版本打包）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptMakeOutput {
    pub script_sh: String,
    pub script_ps: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    /// AI 精修是否成功（失败降级确定性拼接稿）。
    pub ai_refined: bool,
    pub registered: bool,
    /// 脚本整体风险（=脚本内部最高风险命令，AC-39）。
    pub risk_level: crate::types::RiskLevel,
}

/// 精修输入（iac_script refine）：修订稿先出，`confirm=true` 才覆盖写回（AC-40）。
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptRefineInput {
    pub name: String,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub confirm: bool,
}

/// 精修输出。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptRefineOutput {
    pub diff: String,
    pub applied: bool,
}

/// 运行输入（iac_script run / `iac run @名`）：作为一次执行过同一闸门与回滚（AC-39）。
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptRunInput {
    pub name: String,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_word: Option<String>,
}

/// 运行输出复用执行结果契约。
pub type ScriptRunOutput = ExecutionOutcome;
