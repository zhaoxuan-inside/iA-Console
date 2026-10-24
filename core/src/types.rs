//! 领域枚举（词典 `_09`：Platform / RiskLevel / Theme / ConflictPolicy 等）。
//! 命名以词典 code_name 为准；serde 名与配置文件/信封格式一致。

use serde::{Deserialize, Serialize};

/// 目标操作系统（词典：Platform）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// 运行时平台检测（提案生成与翻译目标方的缺省值）。
    pub fn detect() -> Self {
        match std::env::consts::OS {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            _ => Platform::Linux,
        }
    }
}

/// 命令危险度四级（词典：RiskLevel）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Safe,
    Caution,
    Dangerous,
    Destructive,
}

/// 主题模式（词典：Theme）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Day,
    Night,
}

/// 剪贴板通道（词典：CopyScope 相关配置）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMode {
    #[default]
    Osc52,
    Off,
}

/// 回放冲突策略（词典：回滚 on_conflict）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    #[default]
    Ask,
    Skip,
    Abort,
}

/// 模型插件下载源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelhubSource {
    #[default]
    Hf,
    Modelscope,
    #[serde(rename = "hf-mirror")]
    HfMirror,
}

/// Provider 档案四类（ADR-006 修订）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    #[default]
    Openai,
    Anthropic,
    #[serde(rename = "local-plugin")]
    LocalPlugin,
    #[serde(rename = "local-path")]
    LocalPath,
}

/// 高危提示策略：weak=弱提醒不拦截；ask=强确认。翻译/Agent 路径恒确认，不受此开关影响。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmMode {
    #[default]
    Weak,
    Ask,
}

/// 回滚捕获范围策略（统一策略，无逐命令判断）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureScope {
    #[default]
    CwdTree,
    Explicit,
    AllHome,
}

/// 效果操作四类（词典：EffectOp，AC-29）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectOp {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// 捕获范围标记：in=范围内可回滚；out_of_scope=明示不可自动回滚（AC-29）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScopeFlag {
    In,
    OutOfScope,
}

/// 单条效果日志（rollback_journal 行的领域形态，_06 internal 3 节）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectEntry {
    pub seq: i64,
    pub execution_id: String,
    pub path: String,
    pub op: EffectOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_path: Option<String>,
    pub scope: CaptureScopeFlag,
    pub created_at: i64,
}

/// 前像清单引用：manifest 在快照仓库的引用（executions.journal_ref）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRef {
    pub execution_id: String,
    pub manifest_ref: String,
    pub journal_ref: String,
}

/// 回滚定位：执行点 / 日志行 / 时间戳 / 最新。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RollbackPoint {
    ExecutionId(String),
    Seq(i64),
    Timestamp(i64),
    Last,
}

/// 回放范围 (to, from]：from 缺省=最新，to 缺省=最远（AC-33 三种范围语义）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// 回滚请求：范围 + 冲突策略（非交互缺省 Abort）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackRequest {
    pub range: RollbackRange,
    pub on_conflict: ConflictPolicy,
}

/// 回滚点列表过滤。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackListFilter {
    pub q: Option<String>,
    pub since: Option<i64>,
    pub limit: u32,
}

/// 可回滚三态（AC-33：✔可回滚 / ◐部分 / ✘不可+原因）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rollbackable {
    Full,
    Partial { reason: String },
    None { reason: String },
}

/// 回滚点聚合视图（history 式列表行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackPointView {
    pub seq: i64,
    pub at: i64,
    pub command: String,
    pub effect_count: u32,
    pub rollbackable: Rollbackable,
}

/// 被跳过条目（skip 策略 / 冲突 / 引用缺失 / out_of_scope）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub entry: EffectEntry,
    pub reason: String,
}

/// 回滚报告：已应用 + 跳过 + 回收站位置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackReport {
    pub applied: Vec<EffectEntry>,
    pub skipped: Vec<SkippedEntry>,
    pub trash_dir: String,
}

/// 文件版本（对应各次执行的效果条目，AC-30）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersion {
    pub seq: i64,
    pub execution_id: String,
    pub op: EffectOp,
    pub content_ref: String,
    pub at: i64,
    pub command: String,
}

/// 单文件版本定位（恢复输入）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersionRef {
    pub path: String,
    pub seq: i64,
}

/// 恢复报告（含闸门留痕）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreReport {
    pub path: String,
    pub restored_from: i64,
    pub gate: crate::executor::GateInfo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_detect_follows_target_os() {
        let p = Platform::detect();
        assert!(matches!(p, Platform::Linux | Platform::Macos | Platform::Windows));
    }

    #[test]
    fn risk_level_orders_by_severity() {
        assert!(RiskLevel::Destructive > RiskLevel::Dangerous);
        assert!(RiskLevel::Dangerous > RiskLevel::Caution);
        assert!(RiskLevel::Caution > RiskLevel::Safe);
    }

    #[test]
    fn serde_names_match_config_format() {
        assert_eq!(serde_json::to_string(&Platform::Macos).unwrap(), "\"macos\"");
        assert_eq!(serde_json::to_string(&RiskLevel::Safe).unwrap(), "\"safe\"");
        assert_eq!(serde_json::to_string(&ThemeMode::Auto).unwrap(), "\"auto\"");
        assert_eq!(
            serde_json::to_string(&ModelhubSource::HfMirror).unwrap(),
            "\"hf-mirror\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::LocalPath).unwrap(),
            "\"local-path\""
        );
    }
}
