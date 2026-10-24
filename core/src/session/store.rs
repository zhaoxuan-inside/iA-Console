//! 会话存取（域 6）：事件流全量落盘（G6）、按 cwd+host 复用当前会话、全形态恢复（Q46）、
//! 对话轨与钉住备注查询（Q33）。SQL 全参数化；失败转译 IAC-3102。

use super::{
    now_ms, CurrentSessionInput, EventKind, SessionEvent, SessionInfo, SessionListInput,
    SessionResumeInput, SessionSummary,
};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery};

/// 事件追加输入。
#[derive(Debug, Clone)]
pub struct AppendEventInput {
    pub session_id: String,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub pinned: bool,
}

/// 事件写入确认。
#[derive(Debug, Clone, Default)]
pub struct RecordAck {
    pub seq: i64,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Command => "command",
            EventKind::Proposal => "proposal",
            EventKind::AiMessage => "ai_message",
            EventKind::Note => "note",
            EventKind::Diagnosis => "diagnosis",
            EventKind::Separator => "separator",
            EventKind::EnvVar => "env_var",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "proposal" => EventKind::Proposal,
            "ai_message" => EventKind::AiMessage,
            "note" => EventKind::Note,
            "diagnosis" => EventKind::Diagnosis,
            "separator" => EventKind::Separator,
            "env_var" => EventKind::EnvVar,
            _ => EventKind::Command,
        }
    }
}

/// 当前会话获取（GUI/CLI/MCP 首连即调）：cwd+host 复用 active 会话，否则新建。
pub fn get_or_create_current(
    input: &CurrentSessionInput,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.cwd.is_empty() && input.host.is_empty() {
        // 无定位信息：直接新建（测试与 MCP 场景）
        return create_session(input, output, context, metrics, evolution);
    }
    let q = SqlQuery {
        sql: "SELECT id FROM sessions WHERE cwd = ?1 AND host = ?2 AND status = 'active' \
              ORDER BY updated_at DESC LIMIT 1"
            .into(),
        params: vec![input.cwd.as_str().into(), input.host.as_str().into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("get_or_create_current", "查询当前会话失败");
        return false;
    }
    if let Some(id) = rows.text(0, "id") {
        let mut loaded = SessionInfo::default();
        if load_session(id, &mut loaded, context, metrics, evolution) {
            *output = loaded;
            metrics.counter("session.reused", 1);
            return true;
        }
        // 复用失败降级新建
    }
    create_session(input, output, context, metrics, evolution)
}

/// 创建会话（get_or_create_current 子步骤）。
fn create_session(
    input: &CurrentSessionInput,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let now = now_ms();
    let title = input.title.clone().unwrap_or_else(|| format!("会话 {now}"));
    let q = SqlQuery {
        sql: "INSERT INTO sessions (id, title, cwd, shell, host, status, env_json, created_at, updated_at) \
              VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, 'active', '[]', ?5, ?5) \
              RETURNING id, title, cwd, shell, host, status, env_json, created_at, updated_at"
            .into(),
        params: vec![
            title.as_str().into(),
            input.cwd.as_str().into(),
            input.shell.as_str().into(),
            input.host.as_str().into(),
            now.into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("create_session", "写入会话失败");
        return false;
    }
    *output = session_from_row(&rows, 0, &[]);
    metrics.counter("session.created", 1);
    true
}

/// 载入会话（含事件流）。payload 损坏的事件跳过，不影响其余恢复（继承 corrupted 语义）。
pub fn load_session(
    session_id: &str,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "SELECT id, title, cwd, shell, host, status, env_json, created_at, updated_at \
              FROM sessions WHERE id = ?1"
            .into(),
        params: vec![session_id.into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("load_session", "查询会话失败");
        return false;
    }
    if rows.is_empty() {
        evolution.fail(
            crate::error::codes::UNKNOWN_METHOD,
            format!("会话不存在: {session_id}"),
        );
        return false;
    }
    let env = parse_env(rows.text(0, "env_json").unwrap_or("[]"));
    let mut events = Vec::new();
    if !load_events(session_id, &mut events, context, metrics, evolution) {
        evolution.propagate("load_session", "加载事件流失败");
        return false;
    }
    *output = session_from_row(&rows, 0, &env);
    output.events = events;
    true
}

/// 会话列表（ia-sessions list）。
pub fn list_sessions(
    input: &SessionListInput,
    output: &mut Vec<SessionSummary>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "SELECT s.id, s.title, s.cwd, s.status, s.updated_at, \
              (SELECT COUNT(*) FROM session_events e WHERE e.session_id = s.id) AS event_count \
              FROM sessions s ORDER BY s.updated_at DESC LIMIT ?1"
            .into(),
        params: vec![i64::from(input.limit).into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("list_sessions", "查询会话列表失败");
        return false;
    }
    for i in 0..rows.len() {
        output.push(SessionSummary {
            id: rows.text(i, "id").unwrap_or_default().into(),
            title: rows.text(i, "title").unwrap_or_default().into(),
            cwd: rows.text(i, "cwd").unwrap_or_default().into(),
            status: rows.text(i, "status").unwrap_or_default().into(),
            event_count: rows.int(i, "event_count").unwrap_or(0) as u64,
            updated_at: rows.int(i, "updated_at").unwrap_or(0),
        });
    }
    true
}

/// 恢复指定会话（Q46：ia-sessions resume）。
pub fn resume_session(
    input: &SessionResumeInput,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    if !load_session(&input.session_id, output, context, metrics, evolution) {
        return false;
    }
    metrics.counter("session.resumed", 1);
    true
}

/// 追加事件（seq 自增；全部 EventKind 可写——旧版只写 command/proposal 是缺陷）。
pub fn append_event(
    input: &AppendEventInput,
    output: &mut RecordAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let seq_q = SqlQuery {
        sql:
            "SELECT COALESCE(MAX(seq), 0) + 1 AS next_seq FROM session_events WHERE session_id = ?1"
                .into(),
        params: vec![input.session_id.as_str().into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&seq_q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("append_event", "查询事件序号失败");
        return false;
    }
    let seq = rows.int(0, "next_seq").unwrap_or(1);
    let q = SqlQuery {
        sql: "INSERT INTO session_events (session_id, seq, kind, payload, pinned, at) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            .into(),
        params: vec![
            input.session_id.as_str().into(),
            seq.into(),
            input.kind.as_str().into(),
            serde_json::to_string(&input.payload)
                .unwrap_or_default()
                .as_str()
                .into(),
            i64::from(input.pinned).into(),
            now_ms().into(),
        ],
    };
    if !context
        .deps
        .store
        .storage_execute(&q, &mut Default::default(), context, metrics, evolution)
    {
        evolution.propagate("append_event", "写入事件失败");
        return false;
    }
    touch_session(&input.session_id, context, metrics, evolution);
    output.seq = seq;
    metrics.counter("session.event_appended", 1);
    true
}

/// `---` 任务段分隔（Q26）。
pub fn record_separator(
    session_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    append_event(
        &AppendEventInput {
            session_id: session_id.into(),
            kind: EventKind::Separator,
            payload: serde_json::json!({}),
            pinned: false,
        },
        &mut RecordAck::default(),
        context,
        metrics,
        evolution,
    )
}

/// `#` 备注（并入对话轨，可钉住跨段，Q29/Q33）。
pub fn record_note(
    session_id: &str,
    text: &str,
    pinned: bool,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    append_event(
        &AppendEventInput {
            session_id: session_id.into(),
            kind: EventKind::Note,
            payload: serde_json::json!({ "text": text }),
            pinned,
        },
        &mut RecordAck::default(),
        context,
        metrics,
        evolution,
    )
}

/// 声明环境变量（写事件流并更新 sessions.env_json，Q46）。
pub fn record_env(
    session_id: &str,
    key: &str,
    value: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    // 1) 事件流
    if !append_event(
        &AppendEventInput {
            session_id: session_id.into(),
            kind: EventKind::EnvVar,
            payload: serde_json::json!({ "key": key, "value": value }),
            pinned: false,
        },
        &mut RecordAck::default(),
        context,
        metrics,
        evolution,
    ) {
        return false;
    }
    // 2) sessions.env_json upsert
    let mut info = SessionInfo::default();
    if !load_session(session_id, &mut info, context, metrics, evolution) {
        return false;
    }
    let mut env: Vec<(String, String)> = info.env.clone();
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value.into();
    } else {
        env.push((key.into(), value.into()));
    }
    let json = serde_json::to_string(&env).unwrap_or_else(|_| "[]".into());
    let q = SqlQuery {
        sql: "UPDATE sessions SET env_json = ?1, updated_at = ?2 WHERE id = ?3".into(),
        params: vec![json.as_str().into(), now_ms().into(), session_id.into()],
    };
    if !context
        .deps
        .store
        .storage_execute(&q, &mut Default::default(), context, metrics, evolution)
    {
        evolution.propagate("record_env", "更新会话环境失败");
        return false;
    }
    true
}

/// 最近一个 `---` 分隔之后的事件（AI 上下文窗口按段截断，Q26）。
pub fn events_since_separator(
    session_id: &str,
    output: &mut Vec<SessionEvent>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut all = Vec::new();
    if !load_events(session_id, &mut all, context, metrics, evolution) {
        return false;
    }
    match all.iter().rposition(|e| e.kind == EventKind::Separator) {
        Some(pos) => output.extend(all.into_iter().skip(pos + 1)),
        None => output.extend(all),
    }
    true
}

/// 钉住备注（跨 `---` 段注入，Q33）。
pub fn pinned_notes(
    session_id: &str,
    output: &mut Vec<String>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut all = Vec::new();
    if !load_events(session_id, &mut all, context, metrics, evolution) {
        return false;
    }
    output.extend(
        all.iter()
            .filter(|e| e.pinned && e.kind == EventKind::Note)
            .filter_map(|e| {
                e.payload
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(str::to_string)
            }),
    );
    true
}

// ---------- 内部辅助 ----------

/// 事件流加载（load_session 子步骤；损坏 payload 跳过）。
fn load_events(
    session_id: &str,
    output: &mut Vec<SessionEvent>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let q = SqlQuery {
        sql: "SELECT seq, kind, payload, pinned, at FROM session_events WHERE session_id = ?1 ORDER BY seq ASC"
            .into(),
        params: vec![session_id.into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("load_events", "查询事件流失败");
        return false;
    }
    for i in 0..rows.len() {
        let payload_raw = rows.text(i, "payload").unwrap_or("{}");
        let payload: serde_json::Value =
            serde_json::from_str(payload_raw).unwrap_or(serde_json::json!({}));
        output.push(SessionEvent {
            seq: rows.int(i, "seq").unwrap_or(0),
            kind: EventKind::parse_str(rows.text(i, "kind").unwrap_or("command")),
            payload,
            at: rows.int(i, "at").unwrap_or(0),
            pinned: rows.int(i, "pinned").unwrap_or(0) == 1,
        });
    }
    true
}

/// 行 → SessionInfo。
fn session_from_row(rows: &QueryRows, row: usize, env: &[(String, String)]) -> SessionInfo {
    SessionInfo {
        id: rows.text(row, "id").unwrap_or_default().into(),
        title: rows.text(row, "title").unwrap_or_default().into(),
        cwd: rows.text(row, "cwd").unwrap_or_default().into(),
        shell: rows.text(row, "shell").unwrap_or_default().into(),
        host: rows.text(row, "host").unwrap_or_default().into(),
        status: rows.text(row, "status").unwrap_or_default().into(),
        env: env.to_vec(),
        created_at: rows.int(row, "created_at").unwrap_or(0),
        updated_at: rows.int(row, "updated_at").unwrap_or(0),
        events: vec![],
    }
}

/// env_json → 键值对（损坏按空处理）。
fn parse_env(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<Vec<(String, String)>>(raw).unwrap_or_default()
}

/// 触碰会话更新时间。
fn touch_session(
    session_id: &str,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) {
    let q = SqlQuery {
        sql: "UPDATE sessions SET updated_at = ?1 WHERE id = ?2".into(),
        params: vec![now_ms().into(), session_id.into()],
    };
    let _ = context.deps.store.storage_execute(
        &q,
        &mut Default::default(),
        context,
        metrics,
        evolution,
    );
}
