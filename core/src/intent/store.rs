//! 提案存取（proposals 表）：保存（多候选 JSON）、回读、状态推进。

use super::{Proposal, ProposalStatus};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};

const COLS: &str = "id, session_id, intent_text, candidates, active, platform, verified, status, created_at, context_json";

/// 保存提案（INSERT；id 由 caller 生成或为空时入库生成）。
pub fn save_proposal(
    proposal: &mut Proposal,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let candidates = serde_json::to_string(&proposal.candidates).unwrap_or_else(|_| "[]".into());
    let q = SqlQuery {
        sql: "INSERT INTO proposals (id, session_id, intent_text, candidates, active, platform, verified, status, created_at, context_json) \
              VALUES (COALESCE(NULLIF(?1,''), lower(hex(randomblob(16)))), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
              RETURNING id"
            .into(),
        params: vec![
            proposal.id.as_str().into(),
            proposal.session_id.as_str().into(),
            proposal.intent_text.as_str().into(),
            candidates.as_str().into(),
            SqlValue::Int(proposal.active as i64),
            format!("{:?}", proposal.platform).to_lowercase().as_str().into(),
            i64::from(proposal.verified).into(),
            status_text(proposal.status).into(),
            proposal.created_at.into(),
            proposal.context_json.clone().unwrap_or_default().as_str().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("save_proposal", "写入提案失败");
        return false;
    }
    proposal.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("proposal.saved", 1);
    true
}

/// 回读提案（iac_execute 按 proposal_id 取候选）。
pub fn get_proposal(
    proposal_id: &str,
    output: &mut Option<Proposal>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: format!("SELECT {COLS} FROM proposals WHERE id = ?1"),
        params: vec![proposal_id.into()],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("get_proposal", "查询提案失败");
        return false;
    }
    *output = if rows.is_empty() {
        None
    } else {
        Some(proposal_from_row(&rows, 0))
    };
    true
}

/// 状态推进（confirmed/executed/rejected/cancelled）。
pub fn set_proposal_status(
    proposal_id: &str,
    status: ProposalStatus,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql: "UPDATE proposals SET status = ?1 WHERE id = ?2".into(),
        params: vec![status_text(status).into(), proposal_id.into()],
    };
    context
        .deps
        .store
        .storage_execute(&q, &mut Default::default(), context, metrics, evolution)
}

/// 行 → Proposal（损坏 candidates 按空处理）。
fn proposal_from_row(rows: &QueryRows, i: usize) -> Proposal {
    Proposal {
        id: rows.text(i, "id").unwrap_or_default().into(),
        session_id: rows.text(i, "session_id").unwrap_or_default().into(),
        intent_text: rows.text(i, "intent_text").unwrap_or_default().into(),
        candidates: serde_json::from_str(rows.text(i, "candidates").unwrap_or("[]"))
            .unwrap_or_default(),
        active: rows.int(i, "active").unwrap_or(0) as usize,
        platform: match rows.text(i, "platform").unwrap_or("linux") {
            "macos" => crate::types::Platform::Macos,
            "windows" => crate::types::Platform::Windows,
            _ => crate::types::Platform::Linux,
        },
        verified: rows.int(i, "verified").unwrap_or(0) == 1,
        status: match rows.text(i, "status").unwrap_or("proposed") {
            "confirmed" => ProposalStatus::Confirmed,
            "executed" => ProposalStatus::Executed,
            "rejected" => ProposalStatus::Rejected,
            "cancelled" => ProposalStatus::Cancelled,
            _ => ProposalStatus::Proposed,
        },
        created_at: rows.int(i, "created_at").unwrap_or(0),
        context_json: rows
            .text(i, "context_json")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

/// 状态 → 存储文本。
fn status_text(s: ProposalStatus) -> &'static str {
    match s {
        ProposalStatus::Proposed => "proposed",
        ProposalStatus::Confirmed => "confirmed",
        ProposalStatus::Executed => "executed",
        ProposalStatus::Rejected => "rejected",
        ProposalStatus::Cancelled => "cancelled",
    }
}
