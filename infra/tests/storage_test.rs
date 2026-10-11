//! T-003/T-004 存储测试：迁移幂等、写查往返、参数绑定、错误转译。

use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{ExecuteAck, MigrationAck, MigrationDir, QueryRows, SqlQuery, SqlValue, StorePort};
use infra::storage::Storage;

fn store() -> Storage {
    Storage::open_in_memory().expect("in-memory db")
}

fn q(sql: &str, params: Vec<SqlValue>) -> SqlQuery {
    SqlQuery { sql: sql.into(), params }
}

#[test]
fn migrate_applies_v1_and_is_idempotent() {
    let s = store();
    let env = TestEnv::new();
    let mut ack = MigrationAck::default();
    let mut evo = Evolution::default();
    assert!(s.storage_migrate(&MigrationDir {}, &mut ack, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(ack.applied, vec!["0001_init".to_string()]);
    // 幂等：二次运行为空
    let mut ack2 = MigrationAck::default();
    assert!(s.storage_migrate(&MigrationDir {}, &mut ack2, &env.context(), &mut Metrics::default(), &mut evo));
    assert!(ack2.applied.is_empty());
    // v1 关键表存在
    let mut rows = QueryRows::default();
    let tables = "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name";
    assert!(s.storage_query(&q(tables, vec![]), &mut rows, &env.context(), &mut Metrics::default(), &mut evo));
    for t in ["sessions", "session_events", "proposals", "executions", "recipes", "knowledge_cache", "audit_log", "translations", "model_plugins", "rollback_journal"] {
        assert!(rows.rows.iter().any(|r| r.iter().any(|v| matches!(v, SqlValue::Text(n) if n == t))), "缺表 {t}");
    }
}

#[test]
fn execute_and_query_roundtrip_with_params() {
    let s = store();
    let env = TestEnv::new();
    let mut evo = Evolution::default();
    let mut mig = MigrationAck::default();
    s.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut Metrics::default(), &mut evo);

    let mut exec = ExecuteAck::default();
    let insert = "INSERT INTO sessions (id, title, cwd, shell, host, status, created_at, updated_at) \
                  VALUES (lower(hex(randomblob(16))), ?1, ?2, 'zsh', 'host-a', 'active', 1000, 1000)";
    assert!(s.storage_execute(&q(insert, vec!["会话一".into(), "/var/log".into()]), &mut exec, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(exec.rows_affected, 1);

    let mut rows = QueryRows::default();
    assert!(s.storage_query(&q("SELECT title, cwd FROM sessions WHERE cwd = ?1", vec!["/var/log".into()]), &mut rows, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.text(0, "title"), Some("会话一"));
    assert_eq!(rows.text(0, "cwd"), Some("/var/log"));
}

#[test]
fn bad_sql_translates_to_iac_3102() {
    let s = store();
    let env = TestEnv::new();
    let mut evo = Evolution::default();
    let mut rows = QueryRows::default();
    assert!(!s.storage_query(&q("SELECT * FROM 不存在的表", vec![]), &mut rows, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-3102"));
    assert!(evo.reason.as_deref().unwrap().contains("存储操作失败"));
}

#[test]
fn unique_violation_surfaces_as_storage_error() {
    let s = store();
    let env = TestEnv::new();
    let mut evo = Evolution::default();
    let mut mig = MigrationAck::default();
    s.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut Metrics::default(), &mut evo);
    let insert = "INSERT INTO sessions (id, title, cwd, shell, host, created_at, updated_at) VALUES ('dup', 'a', '/', 'sh', 'h', 1, 1)";
    let mut exec = ExecuteAck::default();
    assert!(s.storage_execute(&q(insert, vec![]), &mut exec, &env.context(), &mut Metrics::default(), &mut evo));
    assert!(!s.storage_execute(&q(insert, vec![]), &mut exec, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-3102"));
}
