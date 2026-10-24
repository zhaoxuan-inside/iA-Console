//! 命令知识与方言翻译（域 11）：静态映射优先零 AI（G11/AC-19/20），组合规划自增长（AC-24）。

pub mod store;
pub mod translation_store;
pub mod translation_table;

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

// ---------- 命令知识缓存（域 1 grounding；I-003/I-004） ----------

/// 单参数结构描述（词典：ParamSpec；intent 提案解释共用此类型）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub flag: String,
    pub meaning: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// 命令在某平台的结构化元数据（词典：CommandKnowledge）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandKnowledge {
    pub command: String,
    pub platform: crate::types::Platform,
    #[serde(default)]
    pub subcommands: Vec<String>,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    /// help 文本指纹（变更即失效）。
    pub source_hash: String,
    /// help 解析=可信；AI 补全=false（用户确认后转正）。
    pub verified: bool,
}

/// knowledge 查询引用。
#[derive(Debug, Clone, Default)]
pub struct CommandRef {
    pub command: String,
    pub platform: crate::types::Platform,
    /// 调用方持有的指纹；不一致视为过期未命中。
    pub source_hash: Option<String>,
}

/// 写入确认。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KnowledgeAck {
    pub id: String,
}

// ---------- 方言翻译（域 11） ----------

/// 参数级映射四类（AC-23）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamMapKind {
    Direct,
    Renamed,
    ValueTransform,
    None,
}

/// 单个参数映射。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamMap {
    pub kind: ParamMapKind,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default)]
    pub note: String,
}

/// 翻译输入（iac_translate）。
#[derive(Debug, Clone, Deserialize)]
pub struct TranslationInput {
    pub command: String,
    #[serde(default = "crate::types::Platform::detect")]
    pub from: crate::types::Platform,
    #[serde(default = "crate::types::Platform::detect")]
    pub to: crate::types::Platform,
}

/// 翻译结果。source 标注来源（native/static/cache/ai；AC-22 来源标注同语义）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    #[serde(default)]
    pub param_maps: Vec<ParamMap>,
    pub source: String,
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<RiskLevel>,
    /// 组合规划自增长：AI 组合入方案库后回填（AC-24）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
}

impl TranslationResult {
    /// 未命中/AI 不可用时的降级：返回原命令与说明，不阻塞直接执行（AC-19 边界）。
    pub fn native_fallback(input: &TranslationInput) -> Self {
        Self {
            original: input.command.clone(),
            translated: input.command.clone(),
            param_maps: vec![],
            source: "native".into(),
            note: "静态映射未命中且 AI 不可用，保留原命令".into(),
            risk_level: None,
            recipe_id: None,
        }
    }
}
