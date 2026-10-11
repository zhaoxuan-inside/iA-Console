//! 方案库（词典：Recipe）：save/search/get/delete（I-012~I-015）。

use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{ExecuteAck, QueryRows, SqlQuery, SqlValue};
use crate::session::now_ms;
use crate::types::RiskLevel;
use serde::{Deserialize, Serialize};

/// 方案资产（recipes 表同构；kind=script 时 name 全局唯一）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
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
    pub platform: String,
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub usage_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_execution_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_kind() -> String {
    "command".into()
}

/// 保存输入（id 空即新建；kind=script 时 name 必填且唯一）。
impl Default for RecipeInput {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: None,
            kind: None,
            name: None,
            command_template: String::new(),
            tags: Vec::new(),
            platform: "all".into(),
            risk_level: RiskLevel::Safe,
            source_execution_id: None,
        }
    }
}

pub struct RecipeInput {
    pub title: String,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub command_template: String,
    pub tags: Vec<String>,
    pub platform: String,
    pub risk_level: RiskLevel,
    pub source_execution_id: Option<String>,
}

/// 方案 ID 引用。
#[derive(Debug, Clone, Default)]
pub struct RecipeId {
    pub id: String,
}

/// 检索过滤：关键词（标题/描述/命令模板）+ 标签 + 平台。
#[derive(Debug, Clone, Default)]
pub struct RecipeFilter {
    pub q: Option<String>,
    pub tag: Option<String>,
    pub platform: Option<String>,
    pub limit: u32,
}

/// 删除确认。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeleteAck {
    pub deleted: bool,
}

const COLS: &str = "id, title, description, kind, name, command_template, tags, platform, risk_level, usage_count, source_execution_id, created_at, updated_at";

/// I-012：保存方案（AC-10；name 唯一约束冲突转译 IAC-4101 语义）。
pub fn save_recipe(
    input: &RecipeInput,
    output: &mut Recipe,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.title.trim().is_empty() || input.command_template.trim().is_empty() {
        evolution.fail("IAC-4101", "方案标题与命令模板不能为空");
        return false;
    }
    let now = now_ms();
    let kind = input.kind.clone().unwrap_or_else(|| "command".into());
    let tags = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".into());
    let sql = "INSERT INTO recipes (id, title, description, kind, name, command_template, tags, platform, risk_level, usage_count, source_execution_id, created_at, updated_at) \
               VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?10) \
               RETURNING id";
    let q = SqlQuery {
        sql: sql.into(),
        params: vec![
            input.title.as_str().into(),
            opt(input.description.as_deref()),
            kind.as_str().into(),
            opt(input.name.as_deref()),
            input.command_template.as_str().into(),
            tags.as_str().into(),
            input.platform.as_str().into(),
            risk_text(&input.risk_level).into(),
            opt(input.source_execution_id.as_deref()),
            now.into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("save_recipe", "保存方案失败（name 重复?）");
        return false;
    }
    let id = rows.text(0, "id").unwrap_or_default().to_string();
    let mut got = None;
    if !get_recipe(&RecipeId { id: id.clone() }, &mut got, context, metrics, evolution) || got.is_none() {
        evolution.propagate("save_recipe", "回读方案失败");
        return false;
    }
    *output = got.unwrap();
    metrics.counter("recipe.saved", 1);
    true
}

/// I-013：检索方案（关键词/标签/平台，updated_at 倒序）。
pub fn search_recipes(
    input: &RecipeFilter,
    output: &mut Vec<Recipe>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    if let Some(qk) = &input.q {
        params.push(format!("%{qk}%").into());
        let n = params.len();
        clauses.push(format!("(title LIKE ?{n} OR description LIKE ?{n} OR command_template LIKE ?{n})"));
    }
    if let Some(tag) = &input.tag {
        params.push(format!("%\"{tag}\"%").into());
        clauses.push(format!("tags LIKE ?{}", params.len()));
    }
    if let Some(p) = &input.platform {
        params.push(p.as_str().into());
        clauses.push(format!("platform IN (?{}, 'all')", params.len()));
    }
    let where_sql = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
    params.push((input.limit.max(1) as i64).into());
    let sql = format!("SELECT {COLS} FROM recipes{where_sql} ORDER BY updated_at DESC LIMIT ?{}", params.len());
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&SqlQuery { sql, params }, &mut rows, context, metrics, evolution) {
        evolution.propagate("search_recipes", "检索方案失败");
        return false;
    }
    *output = (0..rows.len()).map(|i| recipe_from_row(&rows, i)).collect();
    metrics.counter("recipe.searched", output.len() as u64);
    true
}

/// I-014：取方案详情（并累加使用计数）。
pub fn get_recipe(
    input: &RecipeId,
    output: &mut Option<Recipe>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery { sql: format!("SELECT {COLS} FROM recipes WHERE id = ?1"), params: vec![input.id.as_str().into()] };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("get_recipe", "查询方案失败");
        return false;
    }
    if rows.is_empty() {
        *output = None;
        return true;
    }
    let mut recipe = recipe_from_row(&rows, 0);
    let up = SqlQuery {
        sql: "UPDATE recipes SET usage_count = usage_count + 1 WHERE id = ?1".into(),
        params: vec![input.id.as_str().into()],
    };
    let mut ack = ExecuteAck::default();
    let _ = context.deps.store.storage_execute(&up, &mut ack, context, metrics, evolution);
    recipe.usage_count += 1;
    *output = Some(recipe);
    true
}

/// I-015：删除方案。
pub fn delete_recipe(
    input: &RecipeId,
    output: &mut DeleteAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery { sql: "DELETE FROM recipes WHERE id = ?1".into(), params: vec![input.id.as_str().into()] };
    let mut ack = ExecuteAck::default();
    if !context.deps.store.storage_execute(&q, &mut ack, context, metrics, evolution) {
        evolution.propagate("delete_recipe", "删除方案失败");
        return false;
    }
    output.deleted = ack.rows_affected > 0;
    metrics.counter("recipe.deleted", u64::from(output.deleted));
    true
}

fn opt(v: Option<&str>) -> SqlValue {
    v.map(SqlValue::from).unwrap_or(SqlValue::Null)
}

fn risk_text(r: &RiskLevel) -> &'static str {
    match r {
        RiskLevel::Safe => "safe",
        RiskLevel::Caution => "caution",
        RiskLevel::Dangerous => "dangerous",
        RiskLevel::Destructive => "destructive",
    }
}

fn risk_parse(s: &str) -> RiskLevel {
    match s {
        "caution" => RiskLevel::Caution,
        "dangerous" => RiskLevel::Dangerous,
        "destructive" => RiskLevel::Destructive,
        _ => RiskLevel::Safe,
    }
}

fn recipe_from_row(rows: &QueryRows, i: usize) -> Recipe {
    Recipe {
        id: rows.text(i, "id").unwrap_or_default().into(),
        title: rows.text(i, "title").unwrap_or_default().into(),
        description: rows.text(i, "description").map(str::to_string),
        kind: rows.text(i, "kind").unwrap_or("command").into(),
        name: rows.text(i, "name").map(str::to_string),
        command_template: rows.text(i, "command_template").unwrap_or_default().into(),
        tags: rows
            .text(i, "tags")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default(),
        platform: rows.text(i, "platform").unwrap_or("all").into(),
        risk_level: risk_parse(rows.text(i, "risk_level").unwrap_or("safe")),
        usage_count: rows.int(i, "usage_count").unwrap_or(0),
        source_execution_id: rows.text(i, "source_execution_id").map(str::to_string),
        created_at: rows.int(i, "created_at").unwrap_or(0),
        updated_at: rows.int(i, "updated_at").unwrap_or(0),
    }
}
