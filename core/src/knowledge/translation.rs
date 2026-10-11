//! 跨方言翻译数据（词典：CommandTranslation/TranslationInput/TranslationResult）。

use crate::types::{Platform, RiskLevel};
use serde::{Deserialize, Serialize};

/// 翻译输入（_06 TranslationInput）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TranslationInput {
    pub command: String,
    pub from_dialect: Platform,
    pub to_platform: Platform,
}

/// 参数级映射（词典：ParamMapping，四类）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamMapping {
    pub from_param: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_param: Option<String>,
    /// direct 直映 | renamed 改名 | value_transform 值变换 | none 无等价
    pub kind: ParamMapKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 参数映射四类（AC-23）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamMapKind {
    Direct,
    Renamed,
    ValueTransform,
    None,
}

/// 译文来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationSource {
    /// 同方言无需翻译。
    Native,
    /// 静态映射（内置表或缓存）。
    Static,
    /// AI 翻译。
    Ai,
}

/// 翻译类别：一对一 / 组合脚本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationKind {
    Direct,
    Composition,
}

/// 翻译结果（词典：TranslationResult）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationResult {
    pub command_from: String,
    pub command_to: String,
    #[serde(default)]
    pub params: Vec<ParamMapping>,
    #[serde(default)]
    pub notes: String,
    pub source: TranslationSource,
    pub kind: TranslationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    pub verified: bool,
    pub risk_level: RiskLevel,
}

impl TranslationResult {
    /// 降级构造：返回原命令（AI 不可用且未命中，AC-19）。
    pub fn native_fallback(input: &TranslationInput) -> Self {
        Self {
            command_from: input.command.clone(),
            command_to: input.command.clone(),
            params: vec![],
            notes: "暂无该命令的等价翻译（AI 不可用）；已按原命令返回，可直接执行或稍后重试".into(),
            source: TranslationSource::Native,
            kind: TranslationKind::Direct,
            recipe_id: None,
            verified: false,
            risk_level: RiskLevel::Caution,
        }
    }
}

/// 写入确认（词典：RecordAck 复用形状）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TranslationAck {
    pub id: String,
}
