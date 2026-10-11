//! 智能域·命令知识（词典：CommandKnowledge/ParamSpec）。parser 位于 common（纯算法）。

pub mod store;
pub mod translation;
pub use translation::{
    TranslationAck, TranslationInput, TranslationKind, TranslationResult, TranslationSource,
};
pub mod translation_store;

pub use store::{cache_knowledge, lookup_knowledge};
pub use translation_store::{cache_translation, lookup_translation, CacheTranslationInput};

use crate::types::Platform;
use serde::{Deserialize, Serialize};

/// 单参数结构描述（词典：ParamSpec）。
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
    pub platform: Platform,
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
    pub platform: Platform,
    /// 调用方持有的指纹；不一致视为过期未命中。
    pub source_hash: Option<String>,
}

/// 写入确认。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KnowledgeAck {
    pub id: String,
}
