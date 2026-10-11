//! 会话数据方法（I-016~I-020）：经 StorePort 落库与检索，本模块无直接 IO。
//! ID 由 SQL `lower(hex(randomblob(16)))` 生成（存储域职责，ADR-004）。

use super::{
    CurrentSessionInput, EventKind, NewSessionInput, RecordAck, SessionEvent, SessionEventInput,
    SessionFilter, SessionId, SessionInfo, SessionSummary,
};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery};

/// 当前 unix 毫秒（时间统一 unix ms 存储，_04 公共机制）。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// I-016：创建会话。
pub fn create_session(
    input: &NewSessionInput,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let now = now_ms();
    let title = input.title.clone().unwrap_or_else(|| format!("会话 {now}"));
    let sql = "INSERT INTO sessions (id, title, cwd, shell, host, status, created_at, updated_at) \
               VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, 'active', ?5, ?5) \
               RETURNING id, title, cwd, shell, host, status, created_at, updated_at";
    let q = SqlQuery {
        sql: sql.into(),
        params: vec![
            title.as_str().into(),
            input.cwd.as_str().into(),
            input.shell.as_str().into(),
            input.host.as_str().into(),
            now.into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("create_session", "写入会话失败");
        return false;
    }
    *output = session_from_row(&rows, 0);
    metrics.counter("session.created", 1);
    true
}

/// I-017：追加会话事件（seq = 当前最大 + 1，UNIQUE(session_id, seq) 保证连续）。
pub fn append_session_event(
    input: &SessionEventInput,
    output: &mut RecordAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let now = now_ms();
    let mut rows = QueryRows::default();
    let seq_q = SqlQuery {
        sql: "SELECT COALESCE(MAX(seq), 0) + 1 AS next_seq FROM session_events WHERE session_id = ?1".into(),
        params: vec![input.session_id.as_str().into()],
    };
    if !context.deps.store.storage_query(&seq_q, &mut rows, context, metrics, evolution) {
        evolution.propagate("append_session_event", "查询事件序号失败");
        return false;
    }
    let seq = rows.int(0, "next_seq").unwrap_or(1);
    let insert = "INSERT INTO session_events (id, session_id, seq, kind, payload, created_at) \
                  VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5) RETURNING id";
    let q = SqlQuery {
        sql: insert.into(),
        params: vec![
            input.session_id.as_str().into(),
            seq.into(),
            input.kind.as_str().into(),
            serde_json::to_string(&input.payload).unwrap_or_default().as_str().into(),
            now.into(),
        ],
    };
    let mut ack_rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut ack_rows, context, metrics, evolution) {
        evolution.propagate("append_session_event", "写入事件失败");
        return false;
    }
    touch_session(&input.session_id, context, metrics, evolution);
    output.id = ack_rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("session.event_appended", 1);
    true
}

/// I-018：载入会话（含事件流）。payload 损坏 → 会话标 corrupted，可用事件照常返回。
pub fn load_session(
    input: &SessionId,
    output: &mut Option<SessionInfo>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut rows = QueryRows::default();
    let q = SqlQuery {
        sql: "SELECT id, title, cwd, shell, host, status, created_at, updated_at FROM sessions WHERE id = ?1".into(),
        params: vec![input.id.as_str().into()],
    };
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("load_session", "查询会话失败");
        return false;
    }
    if rows.is_empty() {
        *output = None;
        return true;
    }
    let mut info = session_from_row(&rows, 0);
    let mut ev_rows = QueryRows::default();
    let eq = SqlQuery {
        sql: "SELECT id, seq, kind, payload, created_at FROM session_events WHERE session_id = ?1 ORDER BY seq".into(),
        params: vec![input.id.as_str().into()],
    };
    if !context.deps.store.storage_query(&eq, &mut ev_rows, context, metrics, evolution) {
        evolution.propagate("load_session", "查询事件流失败");
        return false;
    }
    let mut corrupted = false;
    info.events = (0..ev_rows.len())
        .filter_map(|i| event_from_row(&ev_rows, i, &input.id, &mut corrupted))
        .collect();
    if corrupted {
        info.status = "corrupted".into();
        metrics.counter("session.corrupted", 1);
    }
    *output = Some(info);
    true
}

/// I-019：会话列表（updated_at 倒序，limit 缺省 20）。
pub fn list_sessions(
    input: &SessionFilter,
    output: &mut Vec<SessionSummary>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let limit = input.limit.max(1) as i64;
    let q = SqlQuery {
        sql: "SELECT id, title, cwd, host, status, updated_at FROM sessions ORDER BY updated_at DESC LIMIT ?1".into(),
        params: vec![limit.into()],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("list_sessions", "查询会话列表失败");
        return false;
    }
    *output = (0..rows.len())
        .map(|i| SessionSummary {
            id: rows.text(i, "id").unwrap_or_default().into(),
            title: rows.text(i, "title").unwrap_or_default().into(),
            cwd: rows.text(i, "cwd").unwrap_or_default().into(),
            host: rows.text(i, "host").unwrap_or_default().into(),
            status: rows.text(i, "status").unwrap_or_default().into(),
            updated_at: rows.int(i, "updated_at").unwrap_or(0),
        })
        .collect();
    metrics.counter("session.listed", output.len() as u64);
    true
}

/// I-020：取或建当前会话（cwd+host 活动会话复用，否则新建）。
pub fn get_or_create_current(
    input: &CurrentSessionInput,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "SELECT id FROM sessions WHERE cwd = ?1 AND host = ?2 AND status = 'active' ORDER BY updated_at DESC LIMIT 1".into(),
        params: vec![input.cwd.as_str().into(), input.host.as_str().into()],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("get_or_create_current", "查询当前会话失败");
        return false;
    }
    if let Some(id) = rows.text(0, "id") {
        let sid = SessionId { id: id.to_string() };
        let mut found = None;
        if load_session(&sid, &mut found, context, metrics, evolution) && found.is_some() {
            *output = found.unwrap();
            return true;
        }
    }
    let new_in = NewSessionInput {
        title: None,
        cwd: input.cwd.clone(),
        shell: input.shell.clone(),
        host: input.host.clone(),
    };
    if !create_session(&new_in, output, context, metrics, evolution) {
        evolution.propagate("get_or_create_current", "新建会话失败");
        return false;
    }
    true
}

/// 更新会话 updated_at（事件追加后调用；失败不阻断主流程）。
fn touch_session(session_id: &str, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool {
    let q = SqlQuery {
        sql: "UPDATE sessions SET updated_at = ?1 WHERE id = ?2".into(),
        params: vec![now_ms().into(), session_id.into()],
    };
    let mut ack = crate::ports::ExecuteAck::default();
    context.deps.store.storage_execute(&q, &mut ack, context, metrics, evolution)
}

/// 行 → SessionInfo。
fn session_from_row(rows: &QueryRows, i: usize) -> SessionInfo {
    SessionInfo {
        id: rows.text(i, "id").unwrap_or_default().into(),
        title: rows.text(i, "title").unwrap_or_default().into(),
        cwd: rows.text(i, "cwd").unwrap_or_default().into(),
        shell: rows.text(i, "shell").unwrap_or_default().into(),
        host: rows.text(i, "host").unwrap_or_default().into(),
        status: rows.text(i, "status").unwrap_or_default().into(),
        created_at: rows.int(i, "created_at").unwrap_or(0),
        updated_at: rows.int(i, "updated_at").unwrap_or(0),
        events: Vec::new(),
    }
}

/// 行 → SessionEvent；payload 损坏时置 corrupted 标记并跳过该条。
fn event_from_row(rows: &QueryRows, i: usize, session_id: &str, corrupted: &mut bool) -> Option<SessionEvent> {
    let raw = rows.text(i, "payload").unwrap_or_default();
    let payload: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            *corrupted = true;
            return None;
        }
    };
    let kind_raw = rows.text(i, "kind").unwrap_or_default();
    let kind = EventKind::parse(kind_raw);
    Some(SessionEvent {
        id: rows.text(i, "id").unwrap_or_default().into(),
        session_id: session_id.into(),
        seq: rows.int(i, "seq").unwrap_or(0),
        kind,
        payload,
        created_at: rows.int(i, "created_at").unwrap_or(0),
    })
}
