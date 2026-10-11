//! ports：core 依赖的 infra Adapter trait（依赖倒置边界，见 _04 依赖规则）。
//! infra 侧实现本 trait；core 单测以 NullStore/mock 注入，不触达真实外部组件。

use crate::framework::{Context, Evolution, Metrics};
use serde::{Deserialize, Serialize};

/// SQL 参数值（跨 SQLite 边界的最小类型集）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SqlValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<&str> for SqlValue {
    fn from(s: &str) -> Self {
        SqlValue::Text(s.to_string())
    }
}

impl From<String> for SqlValue {
    fn from(s: String) -> Self {
        SqlValue::Text(s)
    }
}

impl From<i64> for SqlValue {
    fn from(n: i64) -> Self {
        SqlValue::Int(n)
    }
}

/// SQL 输入（查询与写共用形状；SQL 一律参数化，禁止拼接）。
#[derive(Debug, Clone, Default)]
pub struct SqlQuery {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

/// 查询结果：列名 + 行。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
}

impl QueryRows {
    /// 取第 i 行第 col 列的文本值（data 方法取值辅助）。
    pub fn text(&self, row: usize, col: &str) -> Option<&str> {
        let ci = self.columns.iter().position(|c| c == col)?;
        match self.rows.get(row)?.get(ci)? {
            SqlValue::Text(s) => Some(s),
            _ => None,
        }
    }

    /// 取第 i 行第 col 列的整数值。
    pub fn int(&self, row: usize, col: &str) -> Option<i64> {
        let ci = self.columns.iter().position(|c| c == col)?;
        match self.rows.get(row)?.get(ci)? {
            SqlValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// 写执行确认。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecuteAck {
    pub rows_affected: u64,
    pub last_rowid: i64,
}

/// 迁移输入：v1 迁移以嵌入资产随二进制分发（ADR-004）。
#[derive(Debug, Clone, Default)]
pub struct MigrationDir {}

/// 迁移确认：本轮实际应用的版本。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MigrationAck {
    pub applied: Vec<String>,
}

/// 存储 Port（IF-001/002/004 的 trait 形态；infra::storage 实现）。
pub trait StorePort {
    /// 只读查询（SELECT）。失败转译 IAC-3102。
    fn storage_query(
        &self,
        input: &SqlQuery,
        output: &mut QueryRows,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 写执行（INSERT/UPDATE/DELETE/DDL），返回影响行数与 last_rowid。
    fn storage_execute(
        &self,
        input: &SqlQuery,
        output: &mut ExecuteAck,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 版本化迁移（幂等，按 schema_migrations 记账）。
    fn storage_migrate(
        &self,
        input: &MigrationDir,
        output: &mut MigrationAck,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;
}

/// 测试桩：未接线时的空实现（fail-fast，暴露未注入依赖的用例）。
#[derive(Debug, Default)]
pub struct NullStore;

impl StorePort for NullStore {
    fn storage_query(
        &self,
        _input: &SqlQuery,
        _output: &mut QueryRows,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-3102", "存储未接线（NullStore 测试桩）");
        false
    }

    fn storage_execute(
        &self,
        _input: &SqlQuery,
        _output: &mut ExecuteAck,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-3102", "存储未接线（NullStore 测试桩）");
        false
    }

    fn storage_migrate(
        &self,
        _input: &MigrationDir,
        _output: &mut MigrationAck,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-3102", "存储未接线（NullStore 测试桩）");
        false
    }
}
