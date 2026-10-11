//! 提案持久化内部方法（proposals 表；不入 _07——经编排方法间接触达）。

use super::{Proposal, ProposalId, ProposalStatus};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use crate::session::now_ms;

/// 保存提案（INSERT，ID 由 SQL 生成）。
pub fn save_proposal(
    input: &mut Proposal,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    input.created_at = now_ms();
    let sql = "INSERT INTO proposals (id, session_id, intent_text, command, argv, explanation, risk_level, platform, verified, status, created_at) \
               VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id";
    let q = SqlQuery {
        sql: sql.into(),
        params: proposal_params(input),
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("save_proposal", "写入提案失败");
        return false;
    }
    input.id = rows.text(0, "id").unwrap_or_default().to_string();
    true
}

/// 按 ID 载入提案。
pub fn get_proposal(
    input: &ProposalId,
    output: &mut Option<Proposal>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let q = SqlQuery {
        sql: "SELECT id, session_id, intent_text, command, argv, explanation, risk_level, platform, verified, status, created_at \
              FROM proposals WHERE id = ?1"
            .into(),
        params: vec![input.id.as_str().into()],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("get_proposal", "查询提案失败");
        return false;
    }
    *output = if rows.is_empty() { None } else { Some(proposal_from_row(&rows, 0)) };
    true
}

/// 更新提案状态（proposed→confirmed→executed / rejected）。
pub fn set_proposal_status(
    input: &ProposalId,
    status: ProposalStatus,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let q = SqlQuery {
        sql: "UPDATE proposals SET status = ?1 WHERE id = ?2".into(),
        params: vec![status.as_str().into(), input.id.as_str().into()],
    };
    let mut ack = crate::ports::ExecuteAck::default();
    context.deps.store.storage_execute(&q, &mut ack, context, metrics, evolution)
}

fn proposal_params(p: &Proposal) -> Vec<SqlValue> {
    vec![
        p.session_id.as_str().into(),
        p.intent_text.as_str().into(),
        p.command.as_str().into(),
        serde_json::to_string(&p.argv).unwrap_or_default().as_str().into(),
        serde_json::to_string(&p.explanation).unwrap_or_default().as_str().into(),
        risk_text(&p.risk_level).into(),
        format!("{:?}", p.platform).to_lowercase().as_str().into(),
        i64::from(p.verified).into(),
        p.status.as_str().into(),
        p.created_at.into(),
    ]
}

fn proposal_from_row(rows: &QueryRows, i: usize) -> Proposal {
    Proposal {
        id: rows.text(i, "id").unwrap_or_default().into(),
        session_id: rows.text(i, "session_id").unwrap_or_default().into(),
        intent_text: rows.text(i, "intent_text").unwrap_or_default().into(),
        command: rows.text(i, "command").unwrap_or_default().into(),
        argv: rows
            .text(i, "argv")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default(),
        explanation: rows
            .text(i, "explanation")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default(),
        risk_level: risk_parse(rows.text(i, "risk_level").unwrap_or("caution")),
        platform: crate::types::Platform::Linux,
        verified: rows.int(i, "verified").unwrap_or(0) == 1,
        status: ProposalStatus::parse(rows.text(i, "status").unwrap_or("proposed")),
        created_at: rows.int(i, "created_at").unwrap_or(0),
    }
}

fn risk_text(r: &crate::types::RiskLevel) -> &'static str {
    match r {
        crate::types::RiskLevel::Safe => "safe",
        crate::types::RiskLevel::Caution => "caution",
        crate::types::RiskLevel::Dangerous => "dangerous",
        crate::types::RiskLevel::Destructive => "destructive",
    }
}

fn risk_parse(s: &str) -> crate::types::RiskLevel {
    match s {
        "safe" => crate::types::RiskLevel::Safe,
        "dangerous" => crate::types::RiskLevel::Dangerous,
        "destructive" => crate::types::RiskLevel::Destructive,
        _ => crate::types::RiskLevel::Caution,
    }
}
