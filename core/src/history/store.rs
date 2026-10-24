//! 执行记录存取（域 7）：全量输出入库（Q45）、双限 LRU 清退与导出归档（Q47）、审计条目（Q45）。
//! SQL 全参数化；失败转译 IAC-3102。

use super::{
    ExecStatus, ExecutionRecord, HistoryCleanInput, HistoryCleanReport, HistoryExportInput,
    HistoryExportReport, HistoryFilter, HistoryPage,
};
use crate::framework::{Actor, Context, Evolution, Metrics};
use crate::ports::{QueryRows, SqlQuery};
use crate::session::now_ms;

impl ExecStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecStatus::Ok => "ok",
            ExecStatus::Failed => "failed",
            ExecStatus::Timeout => "timeout",
            ExecStatus::Rejected => "rejected",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "failed" => ExecStatus::Failed,
            "timeout" => ExecStatus::Timeout,
            "rejected" => ExecStatus::Rejected,
            _ => ExecStatus::Ok,
        }
    }
}

/// 执行记录入库（三账之一；stdout_full/stderr_full 全量写入，Q45）。
/// id 空串时由 SQL 生成并回填（修复重建期发现的主键碰撞静默丢账缺陷）。
pub fn insert_execution(
    record: &mut ExecutionRecord,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let argv = serde_json::to_string(&record.argv).unwrap_or_else(|_| "[]".into());
    let q = SqlQuery {
        sql: "INSERT INTO executions (id, session_id, proposal_id, command, argv, cwd, shell, host, actor, \
              exit_code, status, risk_level, stdout_summary, stderr_summary, stdout_full, stderr_full, \
              started_at, duration_ms, journal_ref) \
              VALUES (COALESCE(NULLIF(?1,''), lower(hex(randomblob(16)))),?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19) \
              RETURNING id"
            .into(),
        params: vec![
            record.id.as_str().into(),
            record.session_id.as_str().into(),
            record.proposal_id.clone().unwrap_or_default().as_str().into(),
            record.command.as_str().into(),
            argv.as_str().into(),
            record.cwd.as_str().into(),
            record.shell.as_str().into(),
            record.host.as_str().into(),
            record.actor.as_str().into(),
            record.exit_code.map(i64::from).unwrap_or(0).into(),
            record.status.as_str().into(),
            record.risk_level.as_str().into(),
            record.stdout_summary.clone().unwrap_or_default().as_str().into(),
            record.stderr_summary.clone().unwrap_or_default().as_str().into(),
            record.stdout_full.as_str().into(),
            record.stderr_full.as_str().into(),
            record.started_at.into(),
            record.duration_ms.into(),
            record.journal_ref.clone().unwrap_or_default().as_str().into(),
        ],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("insert_execution", "写入执行记录失败");
        return false;
    }
    record.id = rows.text(0, "id").unwrap_or_default().to_string();
    metrics.counter("history.recorded", 1);
    true
}

/// 历史检索（iac_history_query）：q/会话/发起方/状态/时间窗 + 分页；全量输出按需携带。
pub fn query_history(
    input: &HistoryFilter,
    output: &mut HistoryPage,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut where_clause = String::from(" WHERE 1=1");
    let mut params: Vec<crate::ports::SqlValue> = Vec::new();
    if let Some(q) = &input.q {
        where_clause.push_str(&format!(" AND command LIKE ?{}", params.len() + 1));
        params.push(format!("%{q}%").as_str().into());
    }
    if let Some(s) = &input.session_id {
        where_clause.push_str(&format!(" AND session_id = ?{}", params.len() + 1));
        params.push(s.as_str().into());
    }
    if let Some(a) = &input.actor {
        where_clause.push_str(&format!(" AND actor = ?{}", params.len() + 1));
        params.push(a.as_str().into());
    }
    if let Some(st) = &input.status {
        where_clause.push_str(&format!(" AND status = ?{}", params.len() + 1));
        params.push(st.as_str().into());
    }
    if let Some(since) = input.since {
        where_clause.push_str(&format!(" AND started_at >= ?{}", params.len() + 1));
        params.push(since.into());
    }
    if let Some(until) = input.until {
        where_clause.push_str(&format!(" AND started_at <= ?{}", params.len() + 1));
        params.push(until.into());
    }
    // 总数
    let count_q = SqlQuery {
        sql: format!("SELECT COUNT(*) AS total FROM executions{where_clause}"),
        params: params.clone(),
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&count_q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("query_history", "统计历史总数失败");
        return false;
    }
    output.total = rows.int(0, "total").unwrap_or(0) as u64;
    // 分页
    let cols = if input.with_full_output {
        "id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, status, \
         risk_level, stdout_summary, stderr_summary, stdout_full, stderr_full, started_at, duration_ms, journal_ref"
    } else {
        "id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, status, \
         risk_level, stdout_summary, stderr_summary, '' AS stdout_full, '' AS stderr_full, started_at, duration_ms, journal_ref"
    };
    let mut page_params = params.clone();
    page_params.push(i64::from(input.limit.max(1)).into());
    page_params.push(i64::from(input.offset).into());
    let page_q = SqlQuery {
        sql: format!(
            "SELECT {cols} FROM executions{where_clause} ORDER BY started_at DESC, id DESC \
             LIMIT ?{} OFFSET ?{}",
            page_params.len() - 1,
            page_params.len()
        ),
        params: page_params,
    };
    let mut page_rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&page_q, &mut page_rows, context, metrics, evolution)
    {
        evolution.propagate("query_history", "查询历史失败");
        return false;
    }
    for i in 0..page_rows.len() {
        output.items.push(record_from_row(&page_rows, i));
    }
    metrics.counter("history.queried", output.items.len() as u64);
    true
}

/// 保留清理（Q47 双限 LRU）：先按天数，再按容量；dry_run 只报告。
pub fn clean_history(
    input: &HistoryCleanInput,
    output: &mut HistoryCleanReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let cfg = &context.config.history;
    let now = now_ms();
    let cutoff = now - i64::from(cfg.retention_days) * 86_400_000;
    let mut report = HistoryCleanReport::default();
    // 1) 天数清退
    if input.dry_run {
        let mut rows = QueryRows::default();
        let q = SqlQuery {
            sql: "SELECT COUNT(*) AS n, COALESCE(SUM(LENGTH(stdout_full)+LENGTH(stderr_full)),0) AS bytes FROM executions WHERE started_at < ?1".into(),
            params: vec![cutoff.into()],
        };
        if !context
            .deps
            .store
            .storage_query(&q, &mut rows, context, metrics, evolution)
        {
            evolution.propagate("clean_history", "统计超期记录失败");
            return false;
        }
        report.removed = rows.int(0, "n").unwrap_or(0) as u64;
        report.bytes_freed = rows.int(0, "bytes").unwrap_or(0) as u64;
        *output = report;
        return true;
    }
    let del_aged = SqlQuery {
        sql: "DELETE FROM executions WHERE started_at < ?1".into(),
        params: vec![cutoff.into()],
    };
    if !delete_and_count(&del_aged, &mut report, context, metrics, evolution) {
        return false;
    }
    // 2) 容量清退（LRU：从最旧开始删至容量内）
    let total_q = SqlQuery {
        sql: "SELECT COALESCE(SUM(LENGTH(stdout_full)+LENGTH(stderr_full)),0) AS bytes, COUNT(*) AS n FROM executions".into(),
        params: vec![],
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&total_q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("clean_history", "统计占用失败");
        return false;
    }
    let mut total = rows.int(0, "bytes").unwrap_or(0);
    report.remaining = rows.int(0, "n").unwrap_or(0) as u64;
    while total > i64::try_from(cfg.retention_max_bytes).unwrap_or(i64::MAX) {
        // 单批删除最旧 100 条直至容量内
        let ids_q = SqlQuery {
            sql: "SELECT id, LENGTH(stdout_full)+LENGTH(stderr_full) AS bytes FROM executions \
                  ORDER BY started_at ASC, id ASC LIMIT 100"
                .into(),
            params: vec![],
        };
        let mut id_rows = QueryRows::default();
        if !context
            .deps
            .store
            .storage_query(&ids_q, &mut id_rows, context, metrics, evolution)
        {
            evolution.propagate("clean_history", "选取最旧记录失败");
            return false;
        }
        if id_rows.is_empty() {
            break;
        }
        let mut removed_n = 0u64;
        for i in 0..id_rows.len() {
            let id = id_rows.text(i, "id").unwrap_or_default().to_string();
            let del = SqlQuery {
                sql: "DELETE FROM executions WHERE id = ?1".into(),
                params: vec![id.as_str().into()],
            };
            if !context.deps.store.storage_execute(
                &del,
                &mut Default::default(),
                context,
                metrics,
                evolution,
            ) {
                evolution.propagate("clean_history", "容量清退删除失败");
                return false;
            }
            total -= id_rows.int(i, "bytes").unwrap_or(0);
            report.removed += 1;
            removed_n += 1;
        }
        report.remaining = report.remaining.saturating_sub(removed_n);
        if removed_n == 0 {
            break;
        }
    }
    metrics.counter("history.cleaned", report.removed);
    *output = report;
    true
}

/// 导出归档（Q47：JSONL 落盘；时间窗可选）。
pub fn export_history(
    input: &HistoryExportInput,
    output: &mut HistoryExportReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let mut where_clause = String::from(" WHERE 1=1");
    let mut params: Vec<crate::ports::SqlValue> = Vec::new();
    if let Some(since) = input.since {
        where_clause.push_str(&format!(" AND started_at >= ?{}", params.len() + 1));
        params.push(since.into());
    }
    if let Some(until) = input.until {
        where_clause.push_str(&format!(" AND started_at <= ?{}", params.len() + 1));
        params.push(until.into());
    }
    let q = SqlQuery {
        sql: format!("SELECT id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, \
                      status, risk_level, stdout_summary, stderr_summary, stdout_full, stderr_full, \
                      started_at, duration_ms, journal_ref FROM executions{where_clause} ORDER BY started_at ASC"),
        params,
    };
    let mut rows = QueryRows::default();
    if !context
        .deps
        .store
        .storage_query(&q, &mut rows, context, metrics, evolution)
    {
        evolution.propagate("export_history", "查询导出数据失败");
        return false;
    }
    let mut body = String::new();
    for i in 0..rows.len() {
        let record = record_from_row(&rows, i);
        body.push_str(&serde_json::to_string(&record).unwrap_or_default());
        body.push('\n');
    }
    if let Err(e) = write_atomic(std::path::Path::new(&input.path), &body) {
        evolution.fail(crate::error::codes::STORAGE, format!("归档写入失败: {e}"));
        return false;
    }
    output.exported = rows.len() as u64;
    output.path = input.path.clone();
    metrics.counter("history.exported", output.exported);
    true
}

/// 审计条目追加（Q45：非执行类操作入审计；失败不阻断主流程）。
pub fn record_audit(
    actor: &Actor,
    action: &str,
    target: &str,
    detail: serde_json::Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let q = SqlQuery {
        sql:
            "INSERT INTO audit_log (at, actor, action, target, detail) VALUES (?1, ?2, ?3, ?4, ?5)"
                .into(),
        params: vec![
            now_ms().into(),
            actor.as_tag().as_str().into(),
            action.into(),
            target.into(),
            serde_json::to_string(&detail)
                .unwrap_or_default()
                .as_str()
                .into(),
        ],
    };
    let ok = context.deps.store.storage_execute(
        &q,
        &mut Default::default(),
        context,
        metrics,
        evolution,
    );
    if !ok {
        evolution.propagate("record_audit", "审计写入失败");
    }
    ok
}

// ---------- 内部辅助 ----------

/// 删除并计数（clean_history 子步骤）。
fn delete_and_count(
    q: &SqlQuery,
    report: &mut HistoryCleanReport,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let mut ack = Default::default();
    if !context
        .deps
        .store
        .storage_execute(q, &mut ack, context, metrics, evolution)
    {
        evolution.propagate("clean_history", "删除失败");
        return false;
    }
    report.removed += ack.rows_affected;
    true
}

/// 行 → ExecutionRecord。
fn record_from_row(rows: &QueryRows, i: usize) -> ExecutionRecord {
    let argv: Vec<String> =
        serde_json::from_str(rows.text(i, "argv").unwrap_or("[]")).unwrap_or_default();
    ExecutionRecord {
        id: rows.text(i, "id").unwrap_or_default().into(),
        session_id: rows.text(i, "session_id").unwrap_or_default().into(),
        proposal_id: non_empty(rows.text(i, "proposal_id")),
        command: rows.text(i, "command").unwrap_or_default().into(),
        argv,
        cwd: rows.text(i, "cwd").unwrap_or_default().into(),
        shell: rows.text(i, "shell").unwrap_or_default().into(),
        host: rows.text(i, "host").unwrap_or_default().into(),
        actor: rows.text(i, "actor").unwrap_or_default().into(),
        exit_code: rows
            .int(i, "exit_code")
            .map(i32::try_from)
            .transpose()
            .unwrap_or(None),
        status: ExecStatus::parse_str(rows.text(i, "status").unwrap_or("ok")),
        risk_level: rows.text(i, "risk_level").unwrap_or("safe").into(),
        stdout_summary: non_empty(rows.text(i, "stdout_summary")),
        stderr_summary: non_empty(rows.text(i, "stderr_summary")),
        stdout_full: rows.text(i, "stdout_full").unwrap_or_default().into(),
        stderr_full: rows.text(i, "stderr_full").unwrap_or_default().into(),
        started_at: rows.int(i, "started_at").unwrap_or(0),
        duration_ms: rows.int(i, "duration_ms").unwrap_or(0) as u64,
        journal_ref: non_empty(rows.text(i, "journal_ref")),
    }
}

/// 空串转 None（可空列读取约定）。
fn non_empty(v: Option<&str>) -> Option<String> {
    v.filter(|s| !s.is_empty()).map(str::to_string)
}

/// 原子写（临时文件 + 改名）。
fn write_atomic(path: &std::path::Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}
