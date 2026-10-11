//! T-008/T-009 历史测试：全字段往返、过滤检索、审计追加。

use core::framework::TestEnv;
use core::history::{
    append_audit_entry, get_execution_detail, query_history, record_execution, AuditEntry,
    ExecStatus, ExecutionId, ExecutionRecord, HistoryFilter,
};
use core::ports::{MigrationAck, MigrationDir};
use core::session::{create_session, NewSessionInput};
use core::types::RiskLevel;
use infra::storage::Storage;

fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("in-memory db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut ack = MigrationAck::default();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut ack, &env.context(), &mut m, &mut evo));
    env
}

/// 建一个真实会话供外键引用，返回 session_id。
fn seed_session(env: &TestEnv, m: &mut core::framework::Metrics, evo: &mut core::framework::Evolution) -> String {
    let mut info = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(create_session(&NewSessionInput { title: None, cwd: "/var/log".into(), shell: "zsh".into(), host: "host-a".into() }, &mut info, &env.context(), m, evo));
    info.id
}

fn sample(command: &str, cwd: &str, status: ExecStatus, started_at: i64, session_id: &str) -> ExecutionRecord {
    ExecutionRecord {
        id: String::new(),
        session_id: Some(session_id.into()),
        proposal_id: None,
        command: command.into(),
        argv: command.split_whitespace().map(str::to_string).collect(),
        cwd: cwd.into(),
        shell: "zsh".into(),
        host: "host-a".into(),
        actor: "human".into(),
        exit_code: Some(0),
        status,
        risk_level: RiskLevel::Safe,
        stdout_summary: Some(format!("out of {command}")),
        stderr_summary: None,
        stdout_ref: None,
        stderr_ref: None,
        started_at,
        duration_ms: Some(120),
        journal_ref: None,
    }
}

#[test]
fn record_and_detail_roundtrip_full_fields() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let sid = seed_session(&env, &mut m, &mut evo);
    let mut ack = core::session::RecordAck::default();
    let rec = sample("find /var/log -type f", "/var/log", ExecStatus::Ok, 1_000, &sid);
    assert!(record_execution(&rec, &mut ack, &env.context(), &mut m, &mut evo), "evo={evo:?}");
    assert_eq!(ack.id.len(), 32);

    let mut detail = None;
    assert!(get_execution_detail(&ExecutionId { id: ack.id.clone() }, &mut detail, &env.context(), &mut m, &mut evo));
    let d = detail.unwrap();
    assert_eq!(d.command, "find /var/log -type f");
    assert_eq!(d.argv, vec!["find", "/var/log", "-type", "f"]);
    assert_eq!(d.exit_code, Some(0));
    assert_eq!(d.status, ExecStatus::Ok);
    assert_eq!(d.risk_level, RiskLevel::Safe);
    assert_eq!(d.session_id.as_deref(), Some(sid.as_str()));
    assert_eq!(d.duration_ms, Some(120));

    // 不存在 → None
    let mut none = None;
    assert!(get_execution_detail(&ExecutionId { id: "missing".into() }, &mut none, &env.context(), &mut m, &mut evo));
    assert!(none.is_none());
}

#[test]
fn query_filters_combine() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let sid = seed_session(&env, &mut m, &mut evo);
    let mut ack = core::session::RecordAck::default();
    let mut total = 0;
    for i in 0..5 {
        let status = if i % 2 == 0 { ExecStatus::Ok } else { ExecStatus::Failed };
        assert!(record_execution(&sample(&format!("cmd{i}"), "/var/log", status, 1_000 + i, &sid), &mut ack, &env.context(), &mut m, &mut evo));
        total += 1;
    }
    assert!(record_execution(&sample("other", "/tmp", ExecStatus::Ok, 2_000, &sid), &mut ack, &env.context(), &mut m, &mut evo));

    // 关键词
    let mut page = core::history::HistoryPage::default();
    let f = HistoryFilter { q: Some("cmd3".into()), limit: 50, ..Default::default() };
    assert!(query_history(&f, &mut page, &env.context(), &mut m, &mut evo));
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].command, "cmd3");

    // 状态 + 目录
    let f = HistoryFilter { cwd: Some("/var/log".into()), status: Some(ExecStatus::Failed), limit: 50, ..Default::default() };
    assert!(query_history(&f, &mut page, &env.context(), &mut m, &mut evo));
    assert_eq!(page.total, 2);

    // 时间窗
    let f = HistoryFilter { since: Some(2_000), limit: 50, ..Default::default() };
    assert!(query_history(&f, &mut page, &env.context(), &mut m, &mut evo));
    assert_eq!(page.total, 1);

    // 无过滤 → 全量 6
    let f = HistoryFilter { limit: 50, ..Default::default() };
    assert!(query_history(&f, &mut page, &env.context(), &mut m, &mut evo));
    assert_eq!(page.total, total + 1);
    assert_eq!(page.items[0].command, "other", "started_at 倒序");

    // 分页
    let f = HistoryFilter { limit: 2, offset: 1, ..Default::default() };
    assert!(query_history(&f, &mut page, &env.context(), &mut m, &mut evo));
    assert_eq!(page.total, 6);
    assert_eq!(page.items.len(), 2);
}

#[test]
fn audit_entries_append_only() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let sid = seed_session(&env, &mut m, &mut evo);
    for action in ["execute", "confirm", "save_recipe"] {
        let entry = AuditEntry {
            session_id: Some(sid.clone()),
            actor: "agent:zcode".into(),
            action: action.into(),
            target: Some("target-x".into()),
            detail: Some(serde_json::json!({"k": "v"})),
        };
        let mut ack = core::session::RecordAck::default();
        assert!(append_audit_entry(&entry, &mut ack, &env.context(), &mut m, &mut evo));
        assert_eq!(ack.id.len(), 32);
    }
    let mut rows = core::ports::QueryRows::default();
    let q = core::ports::SqlQuery { sql: "SELECT actor, action FROM audit_log ORDER BY ts, id".into(), params: vec![] };
    assert!(env.deps.store.storage_query(&q, &mut rows, &env.context(), &mut m, &mut evo));
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.text(0, "actor"), Some("agent:zcode"));
}
