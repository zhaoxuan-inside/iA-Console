//! 存储 Adapter（IF-001/002/004）：rusqlite 唯一封装入口。
//! WAL 模式 + 写串行化（Mutex）；第三方异常一律转译 IAC-3102/3103 进 evolution。

use iac_core::error::codes;
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{
    ExecuteAck, MigrationAck, MigrationDir, QueryRows, SqlQuery, SqlValue, StorePort,
};
use rusqlite::types::Value as SqliteValue;
use rusqlite::Connection;
use std::sync::Mutex;

/// 嵌入迁移（T-004：Schema v1）。新增版本按序追加。
pub const MIGRATIONS: &[(&str, &str)] = &[("0001_init", include_str!("migrations/0001_init.sql"))];

/// SQLite 连接持有者。Connection 非 Sync，经 Mutex 串行化写（ADR-004 单写者）。
pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    /// 打开（或创建）数据库文件并启用 WAL。
    pub fn open(path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 内存库（测试与临时场景）。
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 公共 pragma：WAL、外键、busy_timeout（跨进程短暂争锁时等待而非立即报错）。
    fn configure(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("storage mutex poisoned")
    }
}

/// SqlValue → rusqlite Value。
fn to_sqlite(v: &SqlValue) -> SqliteValue {
    match v {
        SqlValue::Null => SqliteValue::Null,
        SqlValue::Int(n) => SqliteValue::Integer(*n),
        SqlValue::Real(f) => SqliteValue::Real(*f),
        SqlValue::Text(s) => SqliteValue::Text(s.clone()),
        SqlValue::Blob(b) => SqliteValue::Blob(b.clone()),
    }
}

/// rusqlite ValueRef → SqlValue。
fn from_sqlite(v: rusqlite::types::ValueRef<'_>) -> SqlValue {
    use rusqlite::types::ValueRef as V;
    match v {
        V::Null => SqlValue::Null,
        V::Integer(n) => SqlValue::Int(n),
        V::Real(f) => SqlValue::Real(f),
        V::Text(t) => SqlValue::Text(String::from_utf8_lossy(t).into_owned()),
        V::Blob(b) => SqlValue::Blob(b.to_vec()),
    }
}

impl StorePort for Storage {
    /// IF-001：只读查询。
    fn storage_query(
        &self,
        input: &SqlQuery,
        output: &mut QueryRows,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let conn = self.lock();
        let mut stmt = match conn.prepare(&input.sql) {
            Ok(s) => s,
            Err(e) => return fail_storage(evolution, e),
        };
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let params: Vec<SqliteValue> = input.params.iter().map(to_sqlite).collect();
        let mut rows = match stmt.query(rusqlite::params_from_iter(params.iter())) {
            Ok(r) => r,
            Err(e) => return fail_storage(evolution, e),
        };
        if !collect_rows(&mut rows, &names, output, evolution) {
            return false;
        }
        output.columns = names;
        metrics.counter("storage.query_rows", output.rows.len() as u64);
        true
    }

    /// IF-002：写执行（INSERT/UPDATE/DELETE/DDL）。
    fn storage_execute(
        &self,
        input: &SqlQuery,
        output: &mut ExecuteAck,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let conn = self.lock();
        let params: Vec<SqliteValue> = input.params.iter().map(to_sqlite).collect();
        match conn.execute(&input.sql, rusqlite::params_from_iter(params.iter())) {
            Ok(n) => {
                output.rows_affected = n as u64;
                output.last_rowid = conn.last_insert_rowid();
                metrics.counter("storage.executed", 1);
                true
            }
            Err(e) => fail_storage(evolution, e),
        }
    }

    /// IF-004：版本化迁移（幂等；单个迁移失败自动回滚，不留悬挂事务）。
    fn storage_migrate(
        &self,
        _input: &MigrationDir,
        output: &mut MigrationAck,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.succeed();
        let conn = self.lock();
        if let Err(e) = apply_migrations(&conn, MIGRATIONS, &mut output.applied) {
            return fail_migrate(evolution, e);
        }
        metrics.counter("storage.migrated", output.applied.len() as u64);
        true
    }
}

/// 逐版本应用迁移（幂等）。每个迁移在独立事务中执行：任一语句失败即整体回滚，
/// 连接上不残留打开的事务（此前 BEGIN/COMMIT 拼接 batch 失败会悬挂事务）。
fn apply_migrations(
    conn: &Connection,
    migrations: &[(&str, &str)],
    applied: &mut Vec<String>,
) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
    )?;
    for (name, sql) in migrations {
        let done: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version=?1",
            [name],
            |r| r.get(0),
        )?;
        if done > 0 {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        let result = tx.execute_batch(sql).and_then(|_| {
            tx.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES(?1, strftime('%s','now'))",
                [name],
            )
            .map(|_| ())
        });
        if let Err(e) = result {
            drop(tx); // 析构即回滚
            return Err(e);
        }
        tx.commit()?;
        applied.push(name.to_string());
    }
    Ok(())
}

/// 行集收集（storage_query 子步骤）。
fn collect_rows(
    rows: &mut rusqlite::Rows<'_>,
    names: &[String],
    output: &mut QueryRows,
    evolution: &mut Evolution,
) -> bool {
    let mut out = Vec::new();
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut vals = Vec::with_capacity(names.len());
                for i in 0..names.len() {
                    match row.get_ref(i) {
                        Ok(v) => vals.push(from_sqlite(v)),
                        Err(e) => return fail_storage(evolution, e),
                    }
                }
                out.push(vals);
            }
            Ok(None) => break,
            Err(e) => return fail_storage(evolution, e),
        }
    }
    output.rows = out;
    true
}

/// 查询/执行失败转译（IAC-3102）。
fn fail_storage(evolution: &mut Evolution, e: rusqlite::Error) -> bool {
    evolution.fail(codes::STORAGE, format!("存储操作失败: {e}"));
    false
}

/// 迁移失败转译（IAC-3103）。
fn fail_migrate(evolution: &mut Evolution, e: rusqlite::Error) -> bool {
    evolution.fail(codes::STORAGE_MIGRATE, format!("迁移失败: {e}"));
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 迁移中途失败：整体回滚、不留悬挂事务、连接仍可用（此前 BEGIN/COMMIT batch 会悬挂）。
    #[test]
    fn failed_migration_rolls_back_and_connection_stays_usable() {
        let conn = Connection::open_in_memory().unwrap();
        let bad = [
            ("0001_good", "CREATE TABLE t1(a INTEGER);"),
            (
                "0002_bad",
                "CREATE TABLE t2(a INTEGER); INSERT INTO 不存在的表 VALUES(1);",
            ),
        ];
        let mut applied = Vec::new();
        assert!(apply_migrations(&conn, &bad, &mut applied).is_err());
        assert_eq!(applied, vec!["0001_good"], "失败的 0002 不记账");
        // 0002 整体回滚：t2 不存在
        let t2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='t2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t2, 0);
        // 0001 已提交且记账；重跑幂等
        let mut applied2 = Vec::new();
        assert!(apply_migrations(&conn, &bad[..1], &mut applied2).is_ok());
        assert!(applied2.is_empty(), "0001 已记账不重复应用");
        // 连接无残留事务：常规读写正常
        conn.execute_batch("CREATE TABLE t3(a); INSERT INTO t3 VALUES(7);")
            .unwrap();
        let v: i64 = conn
            .query_row("SELECT a FROM t3", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 7);
    }
}
