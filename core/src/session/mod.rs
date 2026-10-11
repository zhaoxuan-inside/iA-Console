//! 会话域（词典：Session/SessionEvent）。store.rs 为数据方法，resume.rs 为恢复编排。

pub mod resume;
pub mod store;

pub use resume::resume_session;
pub use store::{
    append_session_event, create_session, get_or_create_current, list_sessions, load_session,
    now_ms,
};

use serde::{Deserialize, Serialize};

/// 会话完整信息（含事件流；list 场景 events 为空）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub host: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SessionEvent>,
}

/// 列表项摘要（不含事件流）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub host: String,
    pub status: String,
    pub updated_at: i64,
}

/// 会话事件（kind 见 DDL：command|proposal|ai_message|note|diagnosis|separator）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    pub seq: i64,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

/// 事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {

    Command,
    Proposal,
    AiMessage,
    Note,
    Diagnosis,
    /// `---` 任务分隔（AC-27：其后 AI 调用不携带之前上下文）。
    Separator,
}

impl EventKind {
    /// 存储文本形式（DDL kind 列，纯文本不带引号）。
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Command => "command",
            EventKind::Proposal => "proposal",
            EventKind::AiMessage => "ai_message",
            EventKind::Note => "note",
            EventKind::Diagnosis => "diagnosis",
            EventKind::Separator => "separator",
        }
    }

    /// 从存储文本解析；未知值降级为 note（corrupted 判定交给 payload 校验）。
    pub fn parse(s: &str) -> Self {
        match s {
            "proposal" => EventKind::Proposal,
            "ai_message" => EventKind::AiMessage,
            "diagnosis" => EventKind::Diagnosis,
            "separator" => EventKind::Separator,
            "command" => EventKind::Command,
            _ => EventKind::Note,
        }
    }
}

/// create_session 输入（title 缺省自动生成）。
#[derive(Debug, Clone, Default)]
pub struct NewSessionInput {
    pub title: Option<String>,
    pub cwd: String,
    pub shell: String,
    pub host: String,
}

/// append_session_event 输入（seq 由方法内分配，保证连续）。
#[derive(Debug, Clone)]
pub struct SessionEventInput {
    pub session_id: String,
    pub kind: EventKind,
    pub payload: serde_json::Value,
}

/// 写入确认（事件落库后的 ID）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordAck {
    pub id: String,
}

/// 会话 ID 引用。
#[derive(Debug, Clone, Default)]
pub struct SessionId {
    pub id: String,
}

/// list_sessions 过滤（v1：条数限制，按 updated_at 倒序）。
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub limit: u32,
}

/// get_or_create_current 输入：按 cwd+host 复用活动会话，否则新建。
#[derive(Debug, Clone, Default)]
pub struct CurrentSessionInput {
    pub cwd: String,
    pub shell: String,
    pub host: String,
}
