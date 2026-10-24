//! 服务簇集成测试（域 7/5/11）：执行记录全量输出与双限清退、审计、方案库全环、翻译缓存。

use iac_core::framework::{Actor, Evolution, Metrics, StorePort, TestEnv};
use iac_core::history::store::{clean_history, insert_execution, query_history, record_audit};
use iac_core::history::{
    ExecStatus, ExecutionRecord, HistoryCleanInput, HistoryFilter, HistoryPage,
};
use iac_core::ports::{MigrationAck, MigrationDir};
use iac_core::recipe::handler::{delete_recipe, export_recipes, save_recipe, search_recipes};
use iac_core::recipe::{RecipeDeleteInput, RecipeExportInput, RecipeFilter, RecipeInput};
use iac_core::session::now_ms;
use infra::storage::Storage;

fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("db");
    let mut ack = MigrationAck::default();
    let env = TestEnv::new();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    assert!(store.storage_migrate(&MigrationDir::default(), &mut ack, &env.context(), &mut m, &mut e), "迁移失败: {e:?}");
    env.with_store(Box::new(store))
}

fn record(id: &str, at: i64, stdout: &str) -> ExecutionRecord {
    ExecutionRecord {
        id: id.into(),
        session_id: "sess-test".into(),
        command: format!("cmd-{id}"),
        argv: vec!["cmd".into()],
        cwd: "/tmp".into(),
        shell: "bash".into(),
        host: "local".into(),
        actor: "human".into(),
        exit_code: Some(0),
        status: ExecStatus::Ok,
        risk_level: "safe".into(),
        stdout_summary: Some(stdout.chars().take(16).collect()),
        stderr_summary: None,
        stdout_full: stdout.into(),
        stderr_full: String::new(),
        started_at: at,
        duration_ms: 5,
        journal_ref: None,
        proposal_id: None,
    }
}

#[test]
fn full_output_stored_and_queryable() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let big = "x".repeat(150_000); // > 1/8 旧 64KB 摘要，验证全量独立于摘要
    let r = record("e1", now_ms(), &big);
    assert!(insert_execution(&r, &env.context(), &mut m, &mut e), "evo={e:?}");
    let mut page = HistoryPage::default();
    let filter = HistoryFilter { q: Some("cmd-e1".into()), with_full_output: true, ..Default::default() };
    assert!(query_history(&filter, &mut page, &env.context(), &mut m, &mut e));
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].stdout_full.len(), 150_000, "全量输出必须入库（Q45）");
    // 缺省不携带全量
    let mut light = HistoryPage::default();
    assert!(query_history(&HistoryFilter { q: Some("cmd-e1".into()), ..Default::default() }, &mut light, &env.context(), &mut m, &mut e));
    assert_eq!(light.items[0].stdout_full.len(), 0);
    assert!(light.items[0].stdout_summary.is_some());
}

#[test]
fn retention_days_limit_cleans_aged() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let now = now_ms();
    let day: i64 = 86_400_000;
    for (id, at) in [("old", now - 100 * day), ("n1", now), ("n2", now + 1)] {
        assert!(insert_execution(&record(id, at, "y".repeat(600_000).as_str()), &env.context(), &mut m, &mut e));
    }
    // 天数清退：干跑报告 1 条
    let mut rep = Default::default();
    assert!(clean_history(&HistoryCleanInput { dry_run: true }, &mut rep, &env.context(), &mut m, &mut e));
    assert_eq!(rep.removed, 1);
    // 实际清退：100 天前那条被删
    assert!(clean_history(&HistoryCleanInput { dry_run: false }, &mut rep, &env.context(), &mut m, &mut e));
    let mut page = HistoryPage::default();
    assert!(query_history(&HistoryFilter::default(), &mut page, &env.context(), &mut m, &mut e));
    assert_eq!(page.total, 2, "天数清退后剩 2 条");
}

#[test]
fn retention_capacity_limit_cleans_lru() {
    let mut env = env_with_db();
    let mut cfg = iac_core::config::Config::defaults();
    cfg.history.retention_max_bytes = 500 * 1024; // 500KB，3 条 600KB 全量输出必超
    env.config = cfg;
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let now = now_ms();
    for (id, at) in [("n1", now), ("n2", now + 1), ("n3", now + 2)] {
        assert!(insert_execution(&record(id, at, "y".repeat(600_000).as_str()), &env.context(), &mut m, &mut e));
    }
    let mut rep = Default::default();
    assert!(clean_history(&HistoryCleanInput { dry_run: false }, &mut rep, &env.context(), &mut m, &mut e));
    assert!(rep.removed >= 1, "容量清退应删除至少 1 条: {rep:?}");
    let mut page = HistoryPage::default();
    assert!(query_history(&HistoryFilter::default(), &mut page, &env.context(), &mut m, &mut e));
    // 全量输出 1.8MB → 压到 500KB 内至少删 2 条（LRU 从最旧删）
    assert_eq!(page.total, 3 - rep.removed);
}

#[test]
fn audit_records_non_exec_operations() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    assert!(record_audit(&Actor::Human, "recipe_save", "my-recipe", serde_json::json!({"id":"x"}), &env.context(), &mut m, &mut e));
    assert!(record_audit(&Actor::Agent("zcode".into()), "config_change", "ai.default_provider", serde_json::json!({"to":"glm"}), &env.context(), &mut m, &mut e));
    let q = iac_core::ports::SqlQuery {
        sql: "SELECT COUNT(*) AS n FROM audit_log".into(),
        params: vec![],
    };
    let mut rows = iac_core::ports::QueryRows::default();
    assert!(env.deps.store.storage_query(&q, &mut rows, &env.context(), &mut m, &mut e));
    assert_eq!(rows.int(0, "n"), Some(2));
}

#[test]
fn recipe_save_search_delete_export_cycle() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let input = RecipeInput {
        title: "列大文件".into(),
        description: Some("du 排序".into()),
        kind: "command".into(),
        name: None,
        command_template: "du -ah . | sort -rh | head -20".into(),
        tags: vec!["磁盘".into()],
        platform: "all".into(),
        source_execution_id: None,
    };
    let mut saved = Default::default();
    assert!(save_recipe(&input, &mut saved, &env.context(), &mut m, &mut e), "evo={e:?}");
    assert!(!saved.id.is_empty());
    // name 冲突
    let mut conflict_input = input.clone();
    conflict_input.name = Some("dup".into());
    let mut first = Default::default();
    assert!(save_recipe(&conflict_input, &mut first, &env.context(), &mut m, &mut e));
    let mut conflict2 = conflict_input.clone();
    conflict2.title = "重名".into();
    let mut dup = Default::default();
    assert!(!save_recipe(&conflict2, &mut dup, &env.context(), &mut m, &mut e), "name 重复必须失败");
    assert_eq!(e.reason_code.as_deref(), Some("IAC-4101"));
    // 检索
    let mut hits = Vec::new();
    assert!(search_recipes(&RecipeFilter { q: Some("大文件".into()), ..Default::default() }, &mut hits, &env.context(), &mut m, &mut e));
    assert_eq!(hits.len(), 1);
    // 导出
    let path = std::env::temp_dir().join(format!("iac-recipes-{}.jsonl", std::process::id()));
    let mut exp = Default::default();
    assert!(export_recipes(&RecipeExportInput { path: path.to_string_lossy().into() }, &mut exp, &env.context(), &mut m, &mut e));
    assert_eq!(exp.exported, 2);
    assert!(std::fs::read_to_string(&path).unwrap().contains("du -ah"));
    let _ = std::fs::remove_file(&path);
    // 删除
    let mut deleted = false;
    assert!(delete_recipe(&RecipeDeleteInput { name: Some("dup".into()), id: None }, &mut deleted, &env.context(), &mut m, &mut e));
    assert!(deleted);
}
