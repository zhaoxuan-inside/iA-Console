//! T-010：会话恢复（I-024，AC-06）——载入会话、置 active 并刷新 updated_at，
//! 使同 cwd+host 的 get_or_create_current 复用它，后续执行与 AI 对话延续该会话。

use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{ExecuteAck, SqlQuery};
use crate::session::{now_ms, load_session, SessionId, SessionInfo};

/// I-024：恢复会话。不存在报 IAC-3101（经 evolution），成功返回完整会话（含事件流）。
pub fn resume_session(
    input: &SessionId,
    output: &mut SessionInfo,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut loaded = None;
    if !load_session(input, &mut loaded, context, metrics, evolution) {
        evolution.propagate("resume_session", "载入会话失败");
        return false;
    }
    let Some(info) = loaded else {
        evolution.fail("IAC-3101", format!("会话不存在: {}", input.id));
        return false;
    };
    let q = SqlQuery {
        sql: "UPDATE sessions SET status = 'active', updated_at = ?1 WHERE id = ?2".into(),
        params: vec![now_ms().into(), info.id.as_str().into()],
    };
    let mut ack = ExecuteAck::default();
    if !context.deps.store.storage_execute(&q, &mut ack, context, metrics, evolution) {
        evolution.propagate("resume_session", "激活会话失败");
        return false;
    }
    let mut resumed = info;
    resumed.status = "active".into();
    *output = resumed;
    metrics.counter("session.resumed", 1);
    true
}
