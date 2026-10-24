//! 方案库（域 5）：收藏/标签/检索/导出（G5）；kind=script 为脚本工坊资产（AC-38/39）。

pub mod handler;

use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

/// 方案条目（recipes 行领域形态）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// command | script
    pub kind: String,
    /// 脚本工坊命名（kind=script 全局唯一，AC-38）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub command_template: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_platform")]
    pub platform: String,
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_execution_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_platform() -> String {
    "all".into()
}

/// 保存输入（iac_recipe_save）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecipeInput {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub command_template: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default)]
    pub source_execution_id: Option<String>,
}

fn default_kind() -> String {
    "command".into()
}

/// 检索过滤（iac_recipes_search）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecipeFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

/// 删除输入（`ia-recipe rm`；按 name 或 id）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecipeDeleteInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// 导出输入（`ia-recipe export`：JSONL 归档）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecipeExportInput {
    pub path: String,
}

/// 导出报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecipeExportReport {
    pub exported: u64,
    pub path: String,
}
