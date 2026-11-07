//! 方案库编排（域 5）：save/search/delete/export（I-012~I-015 + Q47 导出语义）。
//! name 唯一约束冲突转译 IAC-4101；保存/删除/导出记审计（Q45 非执行类操作）。

use super::{Recipe, RecipeDeleteInput, RecipeExportInput, RecipeFilter, RecipeInput};
use crate::framework::{Context, Evolution, Metrics};
use crate::history::store::record_audit;
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use crate::risk::assess_static;
use crate::session::now_ms;

const COLS: &str = "id, title, description, kind, name, command_template, tags, platform, risk_level, usage_count, source_execution_id, created_at, updated_at";

const INSERT_RECIPE: &str = "INSERT INTO recipes (id, title, description, kind, name, command_template, tags, platform, risk_level, usage_count, source_execution_id, created_at, updated_at) VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?10) RETURNING id";

/// 保存方案（AC-10）。风险等级由静态规则评估（域 8 一致口径）。
pub fn save_recipe(
    input: &RecipeInput,
    output: &mut Recipe,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.title.trim().is_empty() || input.command_template.trim().is_empty() {
        evolution.fail(
            crate::error::codes::RECIPE_NAME_CONFLICT,
            "方案标题与命令模板不能为空",
        );
        return false;
    }
    let risk = assess_static(&input.command_template);
    let tags = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".into());
    let q = SqlQuery {
        sql: INSERT_RECIPE.into(),
        params: vec![
            input.title.as_str().into(),
            opt(input.description.as_deref()),
            input.kind.as_str().into(),
            opt(input.name.as_deref()),
            input.command_template.as_str().into(),
            tags.as_str().into(),
            input.platform.as_str().into(),
            risk_text(risk).into(),
            opt(input.source_execution_id.as_deref()),
            now_ms().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        // UNIQUE 冲突（name 重复，rusqlite "UNIQUE constraint failed: recipes.name"）→ IAC-4101；
        // 其余存储故障保持 IAC-3102 原语义（锁冲突/缺表等不得误报为重名）
        let reason = evolution.reason.as_deref().unwrap_or_default();
        if reason.contains("UNIQUE constraint") {
            evolution.fail(
                crate::error::codes::RECIPE_NAME_CONFLICT,
                format!(
                    "方案保存失败：name「{}」已存在",
                    input.name.as_deref().unwrap_or_default()
                ),
            );
        } else {
            evolution.propagate("save_recipe", "方案存储失败");
        }
        return false;
    }
    let id = rows.text(0, "id").unwrap_or_default().to_string();
    let mut got = None;
    if !read_recipe(&id, &mut got, context, metrics, evolution) || got.is_none() {
        evolution.propagate("save_recipe", "回读方案失败");
        return false;
    }
    *output = got.unwrap();
    record_audit(
        &context.session.actor,
        "recipe_save",
        output.name.as_deref().unwrap_or(&output.title),
        serde_json::json!({ "id": output.id, "kind": output.kind }),
        context,
        metrics,
        evolution,
    );
    metrics.counter("recipe.saved", 1);
    true
}

/// 检索方案（关键词/标签/平台/kind，updated_at 倒序）。
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
        params.push(format!("%{qk}%").as_str().into());
        let n = params.len();
        clauses.push(format!(
            "(title LIKE ?{n} OR description LIKE ?{n} OR command_template LIKE ?{n})"
        ));
    }
    if let Some(tag) = &input.tag {
        params.push(format!("%\"{tag}\"%").as_str().into());
        clauses.push(format!("tags LIKE ?{}", params.len()));
    }
    if let Some(p) = &input.platform {
        params.push(p.as_str().into());
        clauses.push(format!("platform IN (?{}, 'all')", params.len()));
    }
    if let Some(k) = &input.kind {
        params.push(k.as_str().into());
        clauses.push(format!("kind = ?{}", params.len()));
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    params.push((input.limit.max(1) as i64).into());
    let sql = format!(
        "SELECT {COLS} FROM recipes{where_sql} ORDER BY updated_at DESC LIMIT ?{}",
        params.len()
    );
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(
        &SqlQuery { sql, params },
        &mut rows,
        context,
        metrics,
        evolution,
    ) {
        evolution.propagate("search_recipes", "检索方案失败");
        return false;
    }
    for i in 0..rows.len() {
        output.push(recipe_from_row(&rows, i));
    }
    metrics.counter("recipe.searched", output.len() as u64);
    true
}

/// 删除方案（ia-recipe rm；按 id 或 name）。
pub fn delete_recipe(
    input: &RecipeDeleteInput,
    output: &mut bool,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let (clause, param) = if let Some(id) = &input.id {
        ("id = ?1", id.clone())
    } else if let Some(name) = &input.name {
        ("name = ?1", name.clone())
    } else {
        evolution.fail(
            crate::error::codes::RECIPE_NAME_CONFLICT,
            "删除需要 id 或 name",
        );
        return false;
    };
    let q = SqlQuery {
        sql: format!("DELETE FROM recipes WHERE {clause}"),
        params: vec![param.as_str().into()],
    };
    let mut ack = Default::default();
    if !context
        .deps
        .store
        .storage_execute(&q, &mut ack, context, metrics, evolution)
    {
        evolution.propagate("delete_recipe", "删除方案失败");
        return false;
    }
    *output = ack.rows_affected > 0;
    record_audit(
        &context.session.actor,
        "recipe_delete",
        &param,
        serde_json::json!({ "deleted": *output }),
        context,
        metrics,
        evolution,
    );
    true
}

/// 导出方案库（ia-recipe export：JSONL 归档）。
pub fn export_recipes(
    input: &RecipeExportInput,
    output: &mut super::RecipeExportReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: format!("SELECT {COLS} FROM recipes ORDER BY created_at ASC"),
        params: vec![],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("export_recipes", "查询方案失败");
        return false;
    }
    let mut body = String::new();
    for i in 0..rows.len() {
        body.push_str(&serde_json::to_string(&recipe_from_row(&rows, i)).unwrap_or_default());
        body.push('\n');
    }
    if let Some(dir) = std::path::Path::new(&input.path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = format!("{}.tmp", input.path);
    if let Err(e) = std::fs::write(&tmp, &body).and_then(|_| std::fs::rename(&tmp, &input.path)) {
        evolution.fail(crate::error::codes::STORAGE, format!("方案导出失败: {e}"));
        return false;
    }
    output.exported = rows.len() as u64;
    output.path = input.path.clone();
    record_audit(
        &context.session.actor,
        "recipe_export",
        &input.path,
        serde_json::json!({ "exported": output.exported }),
        context,
        metrics,
        evolution,
    );
    true
}

/// 按 ID 取方案（script 工坊 load 用）。
pub fn get_recipe_by_id(
    id: &str,
    output: &mut Option<Recipe>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    read_recipe(id, output, context, metrics, evolution)
}

/// 使用计数自增（script run / 方案命中时调用）。
pub fn bump_usage(
    recipe_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "UPDATE recipes SET usage_count = usage_count + 1 WHERE id = ?1".into(),
        params: vec![recipe_id.into()],
    };
    context
        .deps
        .store
        .storage_execute(&q, &mut Default::default(), context, metrics, evolution)
}

// ---------- 内部辅助 ----------

/// 无副作用读取（save_recipe 回读共用）。
fn read_recipe(
    id: &str,
    output: &mut Option<Recipe>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: format!("SELECT {COLS} FROM recipes WHERE id = ?1"),
        params: vec![id.into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("read_recipe", "查询方案失败");
        return false;
    }
    *output = if rows.is_empty() {
        None
    } else {
        Some(recipe_from_row(&rows, 0))
    };
    true
}

/// 行 → Recipe。
fn recipe_from_row(rows: &QueryRows, i: usize) -> Recipe {
    Recipe {
        id: rows.text(i, "id").unwrap_or_default().into(),
        title: rows.text(i, "title").unwrap_or_default().into(),
        description: rows
            .text(i, "description")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        kind: rows.text(i, "kind").unwrap_or("command").into(),
        name: rows
            .text(i, "name")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        command_template: rows.text(i, "command_template").unwrap_or_default().into(),
        tags: serde_json::from_str(rows.text(i, "tags").unwrap_or("[]")).unwrap_or_default(),
        platform: rows.text(i, "platform").unwrap_or("all").into(),
        risk_level: risk_from_text(rows.text(i, "risk_level").unwrap_or("safe")),
        usage_count: rows.int(i, "usage_count").unwrap_or(0) as u64,
        source_execution_id: rows
            .text(i, "source_execution_id")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        created_at: rows.int(i, "created_at").unwrap_or(0),
        updated_at: rows.int(i, "updated_at").unwrap_or(0),
    }
}

/// 可空参数包装。
fn opt(v: Option<&str>) -> SqlValue {
    match v {
        Some(s) => s.into(),
        None => SqlValue::Null,
    }
}

/// RiskLevel → 存储文本。
fn risk_text(level: crate::types::RiskLevel) -> &'static str {
    match level {
        crate::types::RiskLevel::Safe => "safe",
        crate::types::RiskLevel::Caution => "caution",
        crate::types::RiskLevel::Dangerous => "dangerous",
        crate::types::RiskLevel::Destructive => "destructive",
    }
}

/// 存储文本 → RiskLevel。
fn risk_from_text(s: &str) -> crate::types::RiskLevel {
    match s {
        "caution" => crate::types::RiskLevel::Caution,
        "dangerous" => crate::types::RiskLevel::Dangerous,
        "destructive" => crate::types::RiskLevel::Destructive,
        _ => crate::types::RiskLevel::Safe,
    }
}
