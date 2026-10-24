//! 执行回滚 v2（域 15）：效果日志机制（Q50 未决期间按现有效果日志重建，开发者保留重选权利）。
//! 不逐命令判断可逆性：执行前统一捕获前像 → 执行后 diff 生成效果日志 → 逆序回放；
//! 触达（Q49）：`ia-rollback`/`ia-file-history` 内置命令 + 自然语言 AI 提案双触达。

pub mod capture;
pub mod diff;
pub mod file_history;
pub mod list;
pub mod replay;

pub use crate::types::{
    EffectEntry, EffectOp, FileVersion, FileVersionRef, ManifestRef, RestoreReport,
    RollbackListFilter, RollbackPoint, RollbackPointView, RollbackRange, RollbackReport,
    RollbackRequest, Rollbackable,
};

/// 单文件版本查询输入（iac_rollback file_history / `ia-file-history <路径>`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilePath {
    pub path: String,
}
