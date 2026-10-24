//! 命令知识与方言翻译（域 11）：静态映射优先零 AI（G11/AC-19/20），组合规划自增长（AC-24）。

pub mod translation_store;
pub mod translation_table;

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

/// 参数级映射四类（AC-23）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamMapKind {
    Renamed,
    ValueTransform,
    NoEquivalent,
    Same,
}

/// 单个参数映射。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamMap {
    pub kind: ParamMapKind,
    pub from: String,
    pub to: String,
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

/// 翻译结果。source 标注来源（AC-22 来源标注同语义）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    #[serde(default)]
    pub param_maps: Vec<ParamMap>,
    /// static | cache | ai | native（未命中降级原文）。
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
