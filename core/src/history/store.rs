//! 历史数据方法（I-008/010/011）与检索（I-009）：经 StorePort，无直接 IO。
//! ID 由 SQL randomblob 生成；argv/JSON 经 serde 序列化入库。

use super::{AuditEntry, ExecStatus, ExecutionId, ExecutionRecord, HistoryFilter, HistoryPage};
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery, SqlValue};
use crate::session::now_ms;
use crate::session::RecordAck;

const INSERT_EXEC: &str = "INSERT INTO executions (id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, status, risk_level, stdout_summary, stderr_summary, stdout_ref, stderr_ref, started_at, duration_ms, journal_ref) VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18) RETURNING id";

const COLS: &str = "id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, \
                    status, risk_level, stdout_summary, stderr_summary, stdout_ref, stderr_ref, \
                    started_at, duration_ms, journal_ref";

/// I-008：写入执行记录（AC-05）。id 由 SQL 生成后回填。
pub fn record_execution(
    input: &ExecutionRecord,
    output: &mut RecordAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery { sql: INSERT_EXEC.into(), params: record_params(input) };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("record_execution", "写入执行记录失败");
        return false;
    }
    output.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("history.recorded", 1);
    true
}

/// 执行记录 → INSERT 参数（record_execution 子步骤）。
fn record_params(input: &ExecutionRecord) -> Vec<SqlValue> {
    let argv = serde_json::to_string(&input.argv).unwrap_or_else(|_| "[]".into());
    vec![
        opt_text(input.session_id.as_deref()),
        opt_text(input.proposal_id.as_deref()),
        input.command.as_str().into(),
        argv.as_str().into(),
        input.cwd.as_str().into(),
        input.shell.as_str().into(),
        input.host.as_str().into(),
        input.actor.as_str().into(),
        opt_int(input.exit_code.map(|v| v as i64)),
        status_text(&input.status).into(),
        risk_text(&input.risk_level).into(),
        opt_text(input.stdout_summary.as_deref()),
        opt_text(input.stderr_summary.as_deref()),
        opt_text(input.stdout_ref.as_deref()),
        opt_text(input.stderr_ref.as_deref()),
        input.started_at.into(),
        opt_int(input.duration_ms.map(|v| v as i64)),
        opt_text(input.journal_ref.as_deref()),
    ]
}

/// I-010：取执行记录详情；不存在返回 None（业务"否"）。
pub fn get_execution_detail(
    input: &ExecutionId,
    output: &mut Option<ExecutionRecord>,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery { sql: format!("SELECT {COLS} FROM executions WHERE id = ?1"), params: vec![input.id.as_str().into()] };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("get_execution_detail", "查询执行记录失败");
        return false;
    }
    *output = if rows.is_empty() { None } else { Some(record_from_row(&rows, 0)) };
    true
}

/// I-011：追加审计条目（追加式，禁改写）。
pub fn append_audit_entry(
    input: &AuditEntry,
    output: &mut RecordAck,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let detail = input.detail.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default());
    let sql = "INSERT INTO audit_log (id, ts, session_id, actor, action, target, detail) \
               VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6) RETURNING id";
    let q = SqlQuery {
        sql: sql.into(),
        params: vec![
            now_ms().into(),
            opt_text(input.session_id.as_deref()),
            input.actor.as_str().into(),
            input.action.as_str().into(),
            opt_text(input.target.as_deref()),
            opt_text(detail.as_deref()),
        ],
    };
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&q, &mut rows, context, metrics, evolution) {
        evolution.propagate("append_audit_entry", "写入审计失败");
        return false;
    }
    output.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("audit.appended", 1);
    true
}

/// I-009：组合过滤检索（关键词/目录/时间窗/状态/发起方），返回总数与页。
pub fn query_history(
    input: &HistoryFilter,
    output: &mut HistoryPage,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let (where_sql, mut params) = build_where(input);
    let count_sql = format!("SELECT COUNT(*) AS total FROM executions{where_sql}");
    let mut count_rows = QueryRows::default();
    if !context.deps.store.storage_query(&count_sql.into_query(params.clone()), &mut count_rows, context, metrics, evolution) {
        evolution.propagate("query_history", "统计总数失败");
        return false;
    }
    output.total = count_rows.int(0, "total").unwrap_or(0);
    let limit = (input.limit.max(1) as i64).min(500);
    let offset = input.offset as i64;
    params.push(limit.into());
    params.push(offset.into());
    let page_sql = format!(
        "SELECT {COLS} FROM executions{where_sql} ORDER BY started_at DESC LIMIT ?{} OFFSET ?{}",
        params.len() - 1,
        params.len()
    );
    let mut rows = QueryRows::default();
    if !context.deps.store.storage_query(&page_sql.into_query(params), &mut rows, context, metrics, evolution) {
        evolution.propagate("query_history", "查询历史失败");
        return false;
    }
    output.items = (0..rows.len()).map(|i| record_from_row(&rows, i)).collect();
    metrics.counter("history.queried", output.items.len() as u64);
    true
}

/// 动态 WHERE 构建：只追加固定片段，值全部参数化。
fn build_where(f: &HistoryFilter) -> (String, Vec<SqlValue>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    if let Some(q) = &f.q {
        clauses.push(format!("(command LIKE ?{} OR stdout_summary LIKE ?{} OR stderr_summary LIKE ?{})", params.len() + 1, params.len() + 1, params.len() + 1));
        params.push(format!("%{q}%").into());
    }
    if let Some(v) = &f.cwd {
        params.push(v.as_str().into());
        clauses.push(format!("cwd = ?{}", params.len()));
    }
    if let Some(v) = f.since {
        params.push(v.into());
        clauses.push(format!("started_at >= ?{}", params.len()));
    }
    if let Some(v) = f.until {
        params.push(v.into());
        clauses.push(format!("started_at <= ?{}", params.len()));
    }
    if let Some(s) = &f.status {
        params.push(status_text(s).into());
        clauses.push(format!("status = ?{}", params.len()));
    }
    if let Some(a) = &f.actor {
        params.push(a.as_str().into());
        clauses.push(format!("actor = ?{}", params.len()));
    }
    if clauses.is_empty() {
        (String::new(), params)
    } else {
        (format!(" WHERE {}", clauses.join(" AND ")), params)
    }
}

/// (WHERE, params) → SqlQuery。
trait IntoQuery {
    fn into_query(self, params: Vec<SqlValue>) -> SqlQuery;
}
impl IntoQuery for String {
    fn into_query(self, params: Vec<SqlValue>) -> SqlQuery {
        SqlQuery { sql: self, params }
    }
}

fn status_text(s: &ExecStatus) -> &'static str {
    match s {
        ExecStatus::Ok => "ok",
        ExecStatus::Failed => "failed",
        ExecStatus::Timeout => "timeout",
        ExecStatus::Cancelled => "cancelled",
        ExecStatus::Rejected => "rejected",
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

fn status_parse(s: &str) -> ExecStatus {
    match s {
        "failed" => ExecStatus::Failed,
        "timeout" => ExecStatus::Timeout,
        "cancelled" => ExecStatus::Cancelled,
        "rejected" => ExecStatus::Rejected,
        _ => ExecStatus::Ok,
    }
}

fn risk_parse(s: &str) -> crate::types::RiskLevel {
    match s {
        "caution" => crate::types::RiskLevel::Caution,
        "dangerous" => crate::types::RiskLevel::Dangerous,
        "destructive" => crate::types::RiskLevel::Destructive,
        _ => crate::types::RiskLevel::Safe,
    }
}

fn opt_text(v: Option<&str>) -> SqlValue {
    v.map(SqlValue::from).unwrap_or(SqlValue::Null)
}

fn opt_int(v: Option<i64>) -> SqlValue {
    v.map(SqlValue::Int).unwrap_or(SqlValue::Null)
}

/// 行 → ExecutionRecord。
fn record_from_row(rows: &QueryRows, i: usize) -> ExecutionRecord {
    let argv: Vec<String> = rows
        .text(i, "argv")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    ExecutionRecord {
        id: rows.text(i, "id").unwrap_or_default().into(),
        session_id: rows.text(i, "session_id").map(str::to_string),
        proposal_id: rows.text(i, "proposal_id").map(str::to_string),
        command: rows.text(i, "command").unwrap_or_default().into(),
        argv,
        cwd: rows.text(i, "cwd").unwrap_or_default().into(),
        shell: rows.text(i, "shell").unwrap_or_default().into(),
        host: rows.text(i, "host").unwrap_or_default().into(),
        actor: rows.text(i, "actor").unwrap_or_default().into(),
        exit_code: rows.int(i, "exit_code").map(|v| v as i32),
        status: status_parse(rows.text(i, "status").unwrap_or("ok")),
        risk_level: risk_parse(rows.text(i, "risk_level").unwrap_or("safe")),
        stdout_summary: rows.text(i, "stdout_summary").map(str::to_string),
        stderr_summary: rows.text(i, "stderr_summary").map(str::to_string),
        stdout_ref: rows.text(i, "stdout_ref").map(str::to_string),
        stderr_ref: rows.text(i, "stderr_ref").map(str::to_string),
        started_at: rows.int(i, "started_at").unwrap_or(0),
        duration_ms: rows.int(i, "duration_ms").map(|v| v as u64),
        journal_ref: rows.text(i, "journal_ref").map(str::to_string),
    }
}
