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

impl From<u64> for SqlValue {
    fn from(n: u64) -> Self {
        SqlValue::Int(n as i64)
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

/// 进程执行输入（IF-003）：命令为 shell 语法串，按平台经 shell 解释。
#[derive(Debug, Clone, Default)]
pub struct ProcessSpec {
    pub command: String,
    pub cwd: Option<std::path::PathBuf>,
    pub env: Vec<(String, String)>,
    pub timeout_ms: u64,
}

/// 进程执行结果。exit_code=None 表示超时被终止。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcessResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub shell_used: String,
}

/// 结构化渲染检测输入（I-045；hint=代码块语言标记/shebang）。
#[derive(Debug, Clone, Default)]
pub struct RawText {
    pub content: String,
    pub hint: Option<String>,
}

/// 模型调用输入（IF-005）：system+messages、温度、超时。
#[derive(Debug, Clone, Default)]
pub struct AiRequest {
    pub system: String,
    pub messages: Vec<(String, String)>,
    pub temperature: f32,
    pub timeout_ms: u64,
}

/// 模型调用统一输出（OpenAI/Anthropic/本地引擎均在 infra::ai 内转译为本结构）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmResponse {
    pub text: String,
    pub model: String,
}

/// AI Port（IF-005 的 trait 形态；infra::ai 实现）。
pub trait AiPort {
    #[allow(clippy::too_many_arguments)]
    fn ai_complete(
        &self,
        provider: &crate::config::ProviderProfile,
        input: &AiRequest,
        output: &mut LlmResponse,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;
}

/// 测试桩：AI 未接线。
#[derive(Debug, Default)]
pub struct NullAi;

impl AiPort for NullAi {
    fn ai_complete(
        &self,
        _provider: &crate::config::ProviderProfile,
        _input: &AiRequest,
        _output: &mut LlmResponse,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1101", "AI 未接线（NullAi 测试桩）");
        false
    }
}

/// 进程 Port（IF-003 的 trait 形态；infra::process 实现）。
pub trait ProcessPort {
    fn spawn_process(
        &self,
        input: &ProcessSpec,
        output: &mut ProcessResult,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;
}

/// 测试桩：进程未接线。
#[derive(Debug, Default)]
pub struct NullProcess;

impl ProcessPort for NullProcess {
    fn spawn_process(
        &self,
        _input: &ProcessSpec,
        _output: &mut ProcessResult,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-2106", "进程未接线（NullProcess 测试桩）");
        false
    }
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

/// 快照数据（IF-011 输入：原始内容，压缩由 Adapter 决定）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotData {
    pub bytes: Vec<u8>,
}

/// 内容引用（IF-011 输出）：`sha256:<hex>`，内容寻址天然去重。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRef {
    pub reference: String,
}

/// retention 策略（IF-013 输入）：天数 + 容量双轨。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub days: u32,
    pub max_bytes: u64,
}

/// 清理报告（IF-013 输出）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub removed: u32,
    pub freed_bytes: u64,
}

/// 目录捕获输入：范围根 + 排除名 + 单文件/总量字节上限。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTreeInput {
    pub roots: Vec<std::path::PathBuf>,
    pub excludes: Vec<String>,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

/// 一次捕获中的一个文件：内容寻址元数据（stored=false 表示超限未存内容，仅指纹）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCaptured {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<String>,
}

/// 回收站请求：把现文件移入回收站（而非抹除，AC-31）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashRequest {
    pub abs_path: std::path::PathBuf,
    pub tag: String,
}

/// 回收站确认。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashAck {
    pub trash_path: String,
    pub moved: bool,
}

/// 恢复写入（回放/单文件恢复共用）：内容直接替换目标文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFileInput {
    pub path: std::path::PathBuf,
    pub bytes: Vec<u8>,
}

/// 快照 Port（IF-011/012/013 + 捕获/回收站/恢复写入；infra::snapstore 实现）。
pub trait SnapPort {
    /// IF-011：内容寻址写入（gzip 压缩 + 去重）。
    fn snapstore_put(
        &self,
        input: &SnapshotData,
        output: &mut ContentRef,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// IF-012：按引用取回并校验完整性；已被回收报 IAC-1602。
    fn snapstore_fetch(
        &self,
        input: &ContentRef,
        output: &mut SnapshotData,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// IF-013：retention（天数+容量 LRU）清理快照与回收站；无法回收到位报 IAC-1601。
    fn sweep_expired(
        &self,
        input: &RetentionPolicy,
        output: &mut SweepReport,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 捕获目录树指纹：遍历 roots（排除 excludes），小文件入库取内容引用，大文件仅记指纹。
    fn capture_tree(
        &self,
        input: &CaptureTreeInput,
        output: &mut Vec<FileCaptured>,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 文件移入回收站（保留相对结构）；目标已不存在返回 moved=false。
    fn trash_file(
        &self,
        input: &TrashRequest,
        output: &mut TrashAck,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 内容直接替换写入（父目录自动创建）。
    fn write_file(
        &self,
        input: &WriteFileInput,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool;

    /// 数据根目录（快照/回收站的宿主目录）。
    fn data_root(&self) -> std::path::PathBuf;
}

/// 测试桩：快照未接线。
#[derive(Debug, Default)]
pub struct NullSnap;

impl SnapPort for NullSnap {
    fn snapstore_put(
        &self,
        _input: &SnapshotData,
        _output: &mut ContentRef,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn snapstore_fetch(
        &self,
        _input: &ContentRef,
        _output: &mut SnapshotData,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn sweep_expired(
        &self,
        _input: &RetentionPolicy,
        _output: &mut SweepReport,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn capture_tree(
        &self,
        _input: &CaptureTreeInput,
        _output: &mut Vec<FileCaptured>,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn trash_file(
        &self,
        _input: &TrashRequest,
        _output: &mut TrashAck,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn write_file(
        &self,
        _input: &WriteFileInput,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库未接线（NullSnap 测试桩）");
        false
    }

    fn data_root(&self) -> std::path::PathBuf {
        std::env::temp_dir()
    }
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
