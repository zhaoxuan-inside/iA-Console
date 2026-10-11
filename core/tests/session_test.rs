//! T-007 会话数据方法测试：真实 SQLite（内存库）上的建/追加/载入/列表/取或建。
//! core 集成测试经 dev-dependency 引 infra（Cargo 允许 dev 环依赖环）。

use core::framework::TestEnv;
use core::ports::{MigrationAck, MigrationDir};
use core::session::{
    append_session_event, create_session, get_or_create_current, list_sessions, load_session,
    CurrentSessionInput, EventKind, NewSessionInput, SessionEventInput, SessionFilter, SessionId,
};
use infra::storage::Storage;

fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("in-memory db");
    let env = TestEnv::new().with_store(Box::new(store));
    // 迁移到 v1
    let mut ack = MigrationAck::default();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut ack, &env.context(), &mut m, &mut evo));
    env
}

fn new_input() -> NewSessionInput {
    NewSessionInput { title: None, cwd: "/var/log".into(), shell: "zsh".into(), host: "host-a".into() }
}

#[test]
fn create_and_load_roundtrip() {
    let env = env_with_db();
    let mut info = core::session::SessionInfo {
        id: String::new(),
        title: String::new(),
        cwd: String::new(),
        shell: String::new(),
        host: String::new(),
        status: String::new(),
        created_at: 0,
        updated_at: 0,
        events: vec![],
    };
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    assert!(create_session(&new_input(), &mut info, &env.context(), &mut m, &mut evo));
    assert_eq!(info.id.len(), 32);
    assert_eq!(info.status, "active");
    assert_eq!(info.cwd, "/var/log");
    assert!(info.title.starts_with("会话"));

    let mut loaded = None;
    let sid = SessionId { id: info.id.clone() };
    assert!(load_session(&sid, &mut loaded, &env.context(), &mut m, &mut evo));
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, info.id);
    assert!(loaded.events.is_empty());

    // 不存在 → None（业务"否"，仍 true）
    let mut none = None;
    assert!(load_session(&SessionId { id: "missing".into() }, &mut none, &env.context(), &mut m, &mut evo));
    assert!(none.is_none());
}

#[test]
fn append_events_assigns_monotonic_seq() {
    let env = env_with_db();
    let mut info = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    create_session(&new_input(), &mut info, &env.context(), &mut m, &mut evo);

    for i in 0..3 {
        let input = SessionEventInput {
            session_id: info.id.clone(),
            kind: EventKind::Command,
            payload: serde_json::json!({"cmd": format!("cmd-{i}")}),
        };
        let mut ack = core::session::RecordAck::default();
        assert!(append_session_event(&input, &mut ack, &env.context(), &mut m, &mut evo));
        assert_eq!(ack.id.len(), 32);
    }

    let mut loaded = None;
    load_session(&SessionId { id: info.id.clone() }, &mut loaded, &env.context(), &mut m, &mut evo);
    let events = loaded.unwrap().events;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[2].seq, 3);
    assert_eq!(events[1].kind, EventKind::Command);
}

#[test]
fn corrupted_payload_marks_session() {
    let env = env_with_db();
    let mut info = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    create_session(&new_input(), &mut info, &env.context(), &mut m, &mut evo);

    // 直接写入坏 payload 模拟损坏
    let bad = core::ports::SqlQuery {
        sql: "INSERT INTO session_events (id, session_id, seq, kind, payload, created_at) VALUES ('bad', ?1, 1, 'note', '{broken', 1)".into(),
        params: vec![info.id.as_str().into()],
    };
    let mut exec = core::ports::ExecuteAck::default();
    assert!(env.deps.store.storage_execute(&bad, &mut exec, &env.context(), &mut m, &mut evo));

    let mut loaded = None;
    load_session(&SessionId { id: info.id.clone() }, &mut loaded, &env.context(), &mut m, &mut evo);
    let s = loaded.unwrap();
    assert_eq!(s.status, "corrupted");
    assert!(s.events.is_empty(), "损坏事件被跳过");

    // 列表不受影响
    let mut list = vec![];
    assert!(list_sessions(&SessionFilter { limit: 10 }, &mut list, &env.context(), &mut m, &mut evo));
    assert_eq!(list.len(), 1);
}

#[test]
fn get_or_create_current_reuses_active_session() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let input = CurrentSessionInput { cwd: "/var/log".into(), shell: "zsh".into(), host: "host-a".into() };
    let mut first = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(get_or_create_current(&input, &mut first, &env.context(), &mut m, &mut evo));

    let mut second = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(get_or_create_current(&input, &mut second, &env.context(), &mut m, &mut evo));
    assert_eq!(first.id, second.id, "同 cwd+host 复用活动会话");

    // 不同 cwd → 新会话
    let other = CurrentSessionInput { cwd: "/tmp".into(), shell: "zsh".into(), host: "host-a".into() };
    let mut third = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(get_or_create_current(&other, &mut third, &env.context(), &mut m, &mut evo));
    assert_ne!(first.id, third.id);
}
