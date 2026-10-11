//! 存储 Adapter（IF-001/002/004）：rusqlite 唯一封装入口。
//! WAL 模式 + 写串行化（Mutex）；第三方异常一律转译 IAC-3102/3103 进 evolution。

use core::error::codes;
use core::framework::{Context, Evolution, Metrics};
use core::ports::{ExecuteAck, MigrationAck, MigrationDir, QueryRows, SqlQuery, SqlValue, StorePort};
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
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 内存库（测试与临时场景）。
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn: Mutex::new(conn) })
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
        let mut out = Vec::new();
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let mut vals = Vec::with_capacity(names.len());
                    for i in 0..names.len() {
                        vals.push(from_sqlite(row.get_ref_unwrap(i)));
                    }
                    out.push(vals);
                }
                Ok(None) => break,
                Err(e) => return fail_storage(evolution, e),
            }
        }
        output.columns = names;
        output.rows = out;
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

    /// IF-004：版本化迁移（幂等）。
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
        if let Err(e) = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
        ) {
            return fail_migrate(evolution, e);
        }
        for (name, sql) in MIGRATIONS {
            let done: i64 = match conn
                .query_row("SELECT COUNT(*) FROM schema_migrations WHERE version=?1", [name], |r| r.get(0))
            {
                Ok(n) => n,
                Err(e) => return fail_migrate(evolution, e),
            };
            if done > 0 {
                continue;
            }
            let batch = format!(
                "BEGIN; {sql}\nINSERT INTO schema_migrations(version, applied_at) VALUES('{name}', strftime('%s','now'));\nCOMMIT;"
            );
            if let Err(e) = conn.execute_batch(&batch) {
                return fail_migrate(evolution, e);
            }
            output.applied.push(name.to_string());
        }
        metrics.counter("storage.migrated", output.applied.len() as u64);
        true
    }
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
