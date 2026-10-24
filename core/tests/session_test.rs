//! 会话域集成测试（域 6）：真实 in-memory SQLite 上的复用/恢复/分段/钉住/env。

use iac_core::framework::StorePort;
use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{MigrationAck, MigrationDir};
use iac_core::session::store::{
    append_event, events_since_separator, get_or_create_current, list_sessions, pinned_notes,
    record_env, record_note, record_separator, resume_session, AppendEventInput, RecordAck,
};
use iac_core::session::{
    CurrentSessionInput, EventKind, SessionInfo, SessionListInput, SessionResumeInput,
};
use infra::storage::Storage;

/// in-memory 库 + 迁移到最新 schema。
fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("db");
    let mut ack = MigrationAck::default();
    let env = TestEnv::new();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    assert!(
        store.storage_migrate(
            &MigrationDir::default(),
            &mut ack,
            &env.context(),
            &mut m,
            &mut e
        ),
        "迁移失败: {e:?}"
    );
    env.with_store(Box::new(store))
}

fn new_session(env: &TestEnv, cwd: &str, m: &mut Metrics, e: &mut Evolution) -> SessionInfo {
    let mut out = SessionInfo::default();
    let input = CurrentSessionInput {
        cwd: cwd.into(),
        shell: "bash".into(),
        host: "local".into(),
        title: None,
    };
    assert!(
        get_or_create_current(&input, &mut out, &env.context(), m, e),
        "evo={e:?}"
    );
    out
}

#[test]
fn create_reuse_and_resume_session() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let s1 = new_session(&env, "/tmp", &mut m, &mut e);
    let s2 = new_session(&env, "/tmp", &mut m, &mut e);
    assert_eq!(s1.id, s2.id, "cwd+host 应复用会话");
    let s3 = new_session(&env, "/opt", &mut m, &mut e);
    assert_ne!(s1.id, s3.id);
    let mut back = SessionInfo::default();
    assert!(resume_session(
        &SessionResumeInput {
            session_id: s1.id.clone()
        },
        &mut back,
        &env.context(),
        &mut m,
        &mut e
    ));
    assert_eq!(back.id, s1.id);
    assert_eq!(back.cwd, "/tmp");
    let mut list = Vec::new();
    assert!(list_sessions(
        &SessionListInput { limit: 10 },
        &mut list,
        &env.context(),
        &mut m,
        &mut e
    ));
    assert_eq!(list.len(), 2);
}

#[test]
fn all_event_kinds_append_and_separator_truncates() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let sid = new_session(&env, "/w", &mut m, &mut e).id;
    let ctx = env.context();
    for text in ["a", "b"] {
        assert!(record_note(&sid, text, false, &ctx, &mut m, &mut e));
    }
    let mut ack = RecordAck::default();
    assert!(append_event(
        &AppendEventInput {
            session_id: sid.clone(),
            kind: EventKind::Command,
            payload: serde_json::json!({"command":"ls"}),
            pinned: false
        },
        &mut ack,
        &ctx,
        &mut m,
        &mut e
    ));
    assert!(record_separator(&sid, &ctx, &mut m, &mut e));
    assert!(record_note(&sid, "after", false, &ctx, &mut m, &mut e));
    let mut seg = Vec::new();
    assert!(events_since_separator(&sid, &mut seg, &ctx, &mut m, &mut e));
    assert_eq!(seg.len(), 1, "separator 后只见段内事件");
    assert_eq!(seg[0].kind, EventKind::Note);
    // 钉住跨段（Q33）
    assert!(record_note(&sid, "pinned-note", true, &ctx, &mut m, &mut e));
    let mut notes = Vec::new();
    assert!(pinned_notes(&sid, &mut notes, &ctx, &mut m, &mut e));
    assert_eq!(notes, vec!["pinned-note".to_string()]);
}

#[test]
fn env_declaration_updates_session() {
    let env = env_with_db();
    let mut m = Metrics::default();
    let mut e = Evolution::default();
    let sid = new_session(&env, "/e", &mut m, &mut e).id;
    let ctx = env.context();
    assert!(record_env(&sid, "FOO", "bar", &ctx, &mut m, &mut e));
    assert!(record_env(&sid, "FOO", "baz", &ctx, &mut m, &mut e));
    let mut back = SessionInfo::default();
    assert!(resume_session(
        &SessionResumeInput { session_id: sid },
        &mut back,
        &ctx,
        &mut m,
        &mut e
    ));
    assert_eq!(back.env, vec![("FOO".into(), "baz".into())]);
}
