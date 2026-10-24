//! SQLite 存储（ADR-004）：WAL、单连接 Mutex 串行、嵌入迁移。
pub mod migrations;

use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{
    ExecuteAck, MigrationAck, MigrationDir, QueryRows, SnapPort, SqlQuery, StorePort,
};

/// 存储句柄（StorePort + SnapPort 实现由重建阶段填充）。
pub struct Storage {
    _path: std::path::PathBuf,
}

impl Storage {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        Ok(Self { _path: path.to_path_buf() })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Ok(Self { _path: ":memory:".into() })
    }
}

impl StorePort for Storage {
    fn storage_query(
        &self,
        _input: &SqlQuery,
        _output: &mut QueryRows,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-3102", "存储重建中（storage_query 未接线）");
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
        evolution.fail("IAC-3102", "存储重建中（storage_execute 未接线）");
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
        evolution.fail("IAC-3103", "存储重建中（storage_migrate 未接线）");
        false
    }
}
