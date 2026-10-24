//! 会话持久化与恢复（域 6）：事件流落盘（G6）、全形态恢复（Q46）、对话轨（Q31~Q33）。

pub mod store;

use serde::{Deserialize, Serialize};

/// 会话事件类型。Q46 持久化面 = 命令/提案/AI 回答/备注/诊断/环境声明/任务段分隔。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    #[default]
    Command,
    Proposal,
    AiMessage,
    /// `#` 会话备注（域 13：并入对话轨，可钉住跨 `---`，Q29/Q33）。
    Note,
    Diagnosis,
    /// `---` 任务段分隔（域 13：AI 上下文窗口不跨越最近分界，Q26）。
    Separator,
    /// 声明的环境变量（`export K=V` / `set K=V`；Q46 持久化面）。
    EnvVar,
}

/// 单条会话事件（session_events 行领域形态；payload 为 JSON）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: i64,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub at: i64,
    /// 钉住：跨 `---` 段仍注入 AI 上下文（Q33）。
    #[serde(default)]
    pub pinned: bool,
}

/// 会话信息（iac_sessions_current 返回；恢复面含 env 与事件流）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub host: String,
    pub status: String,
    /// 声明的环境变量（Q46）。
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub events: Vec<SessionEvent>,
}

/// 当前会话获取（按 cwd+host 复用 active 会话；GUI/CLI 首连即调）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CurrentSessionInput {
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// 会话摘要（iac_session_list 行）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub status: String,
    pub event_count: u64,
    pub updated_at: i64,
}

/// 会话列表输入。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionListInput {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

/// 恢复输入（Q46：CLI/TUI 经 `ia-sessions resume` 恢复指定会话）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionResumeInput {
    pub session_id: String,
}
