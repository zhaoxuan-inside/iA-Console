//! 运行配置（词典：Config）。结构即配置文件 schema（TOML）。
//! 读取/写入的 IO 在 iacd 组合根；本模块只承载结构与纯逻辑（默认值、env 覆盖、校验）。
//!
//! 重建对照 PRD v1.4：
//! - 闸门矩阵固定（Q34~Q37），无 confirm_mode 开关（旧 ConfirmMode 死配置已删除）；
//! - 执行记录全量输出入库（Q45），无 1MB 落盘引用分流（旧 output_raw_max 已删除）；
//! - 新增 history 保留策略：天数+容量双限 LRU（Q47）。

use crate::types::{
    CaptureScope, ClipboardMode, ConflictPolicy, ModelhubSource, Platform, ProviderKind, ThemeMode,
};
use serde::{Deserialize, Serialize};

/// 界面配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub habit_dialect: Platform,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default)]
    pub clipboard: ClipboardMode,
}

/// 模型调度配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_provider_name")]
    pub default_provider: String,
    #[serde(default)]
    pub fallback_local: bool,
}

fn default_provider_name() -> String {
    "local-builtin".into()
}

/// 一个可调用的模型后端档案（ADR-006 修订四类）。密钥一律经环境变量，不落盘。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub name: String,
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Anthropic 必填 max_tokens；缺省 2048。OpenAI 侧仅显式配置时下发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// 模型插件下载源。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelhubConfig {
    #[serde(default)]
    pub source: ModelhubSource,
}

/// 回滚配置（效果日志机制；max_entries 硬上限 65536）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub capture_scope: CaptureScope,
    #[serde(default = "default_max_entries")]
    pub max_entries: u32,
    #[serde(default)]
    pub on_conflict: ConflictPolicy,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_retention_bytes")]
    pub retention_max_bytes: u64,
}

/// 执行记录保留策略（Q47：天数+容量双限 LRU 清退，可配置）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryConfig {
    #[serde(default = "default_history_days")]
    pub retention_days: u32,
    #[serde(default = "default_history_bytes")]
    pub retention_max_bytes: u64,
}

fn default_true() -> bool {
    true
}
fn default_max_entries() -> u32 {
    1024
}
fn default_retention_days() -> u32 {
    7
}
fn default_retention_bytes() -> u64 {
    1024 * 1024 * 1024
}
fn default_history_days() -> u32 {
    90
}
fn default_history_bytes() -> u64 {
    1024 * 1024 * 1024
}

/// 渲染配置（结构化渲染 G17 参考事实；高亮引擎随域 17 走查后定）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderConfig {
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default = "default_true")]
    pub output_detection: bool,
}

fn default_languages() -> Vec<String> {
    vec!["python".into(), "shell".into(), "bat".into()]
}

/// 执行边界（超时；摘要列仍截断存储供列表快速呈现，全量输出另行入库）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecConfig {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_summary_max")]
    pub stdout_summary_max: usize,
    #[serde(default = "default_summary_max")]
    pub stderr_summary_max: usize,
}

fn default_timeout_ms() -> u64 {
    120_000
}
fn default_summary_max() -> usize {
    64 * 1024
}

// 子结构 Default 与各 serde default 函数同源（缺省值唯一出处）。
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider_name(),
            fallback_local: false,
        }
    }
}
impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_scope: CaptureScope::default(),
            max_entries: default_max_entries(),
            on_conflict: ConflictPolicy::default(),
            retention_days: default_retention_days(),
            retention_max_bytes: default_retention_bytes(),
        }
    }
}
impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            retention_days: default_history_days(),
            retention_max_bytes: default_history_bytes(),
        }
    }
}
impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            languages: default_languages(),
            output_detection: true,
        }
    }
}
impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            stdout_summary_max: default_summary_max(),
            stderr_summary_max: default_summary_max(),
        }
    }
}

/// 运行配置根结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub providers: Vec<ProviderProfile>,
    #[serde(default)]
    pub modelhub: ModelhubConfig,
    #[serde(default)]
    pub rollback: RollbackConfig,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub render: RenderConfig,
    #[serde(default)]
    pub exec: ExecConfig,
}

impl Config {
    /// 首次运行/文件缺失时的缺省配置（AC 全部默认值与配置表一致）。
    pub fn defaults() -> Self {
        let mut c = Config::default();
        c.ui.habit_dialect = Platform::detect();
        c
    }

    /// `IAC_*` 环境变量覆盖（缺省实现读进程环境；测试用 `apply_env_with` 注入）。
    pub fn apply_env(&mut self) {
        self.apply_env_with(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    }

    /// 覆盖优先级：环境变量 > 配置文件 > 默认值。
    pub fn apply_env_with(&mut self, lookup: impl Fn(&str) -> Option<String>) {
        if let Some(v) = lookup("IAC_HABIT_DIALECT") {
            if let Ok(p) = serde_json::from_value(serde_json::json!(v)) {
                self.ui.habit_dialect = p;
            }
        }
        if let Some(v) = lookup("IAC_THEME") {
            if let Ok(t) = serde_json::from_value(serde_json::json!(v)) {
                self.ui.theme = t;
            }
        }
        if let Some(v) = lookup("IAC_DEFAULT_PROVIDER") {
            self.ai.default_provider = v;
        }
    }

    /// 结构校验（IAC-5101 的判定来源）。
    pub fn validate(&self) -> Result<(), String> {
        let mut names: Vec<&str> = Vec::new();
        for p in &self.providers {
            if p.name.is_empty() {
                return Err("provider.name 不能为空".into());
            }
            if names.contains(&p.name.as_str()) {
                return Err(format!("provider.name 重复: {}", p.name));
            }
            names.push(&p.name);
            match p.kind {
                ProviderKind::Openai | ProviderKind::Anthropic => {
                    if p.base_url.is_none() || p.api_key_env.is_none() || p.model.is_none() {
                        return Err(format!(
                            "provider[{}] 远端类型必须配置 base_url/api_key_env/model",
                            p.name
                        ));
                    }
                }
                ProviderKind::LocalPath if p.path.is_none() => {
                    return Err(format!(
                        "provider[{}] local-path 必须配置模型文件 path",
                        p.name
                    ));
                }
                _ => {}
            }
        }
        if self.rollback.max_entries > 65536 {
            return Err("rollback.max_entries 上限 65536".into());
        }
        if !self.ai.providers_exist(&self.providers) {
            return Err(format!(
                "ai.default_provider 指向未定义的 provider: {}",
                self.ai.default_provider
            ));
        }
        Ok(())
    }
}

impl AiConfig {
    /// 本地内置档案 local-builtin 由 modelhub 接线提供，视作隐式存在。
    fn providers_exist(&self, providers: &[ProviderProfile]) -> bool {
        self.default_provider == "local-builtin"
            || providers.iter().any(|p| p.name == self.default_provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(Config::defaults().validate().is_ok());
    }

    #[test]
    fn defaults_carry_product_decisions() {
        let c = Config::defaults();
        assert_eq!(c.rollback.max_entries, 1024);
        assert_eq!(c.history.retention_days, 90);
        assert_eq!(c.render.languages, vec!["python", "shell", "bat"]);
        assert_eq!(c.ai.default_provider, "local-builtin");
        assert_eq!(c.exec.timeout_ms, 120_000);
    }

    #[test]
    fn env_overrides_win_over_file_values() {
        let mut c = Config::defaults();
        c.apply_env_with(|k| {
            (k == "IAC_HABIT_DIALECT")
                .then(|| "windows".into())
                .or((k == "IAC_DEFAULT_PROVIDER").then(|| "glm".into()))
        });
        assert_eq!(c.ui.habit_dialect, Platform::Windows);
        assert_eq!(c.ai.default_provider, "glm");
    }

    #[test]
    fn env_ignores_bad_values_and_keeps_defaults() {
        let mut c = Config::defaults();
        c.apply_env_with(|k| (k == "IAC_HABIT_DIALECT").then(|| "dos".into()));
        assert_eq!(c.ui.habit_dialect, Platform::detect());
    }

    #[test]
    fn validate_rejects_duplicate_and_incomplete_providers() {
        let mut c = Config::defaults();
        c.providers.push(ProviderProfile {
            name: "glm".into(),
            kind: ProviderKind::Openai,
            base_url: None,
            api_key_env: None,
            model: None,
            path: None,
            max_tokens: None,
        });
        assert!(c.validate().is_err());
        let p2 = ProviderProfile {
            name: "glm".into(),
            kind: ProviderKind::LocalPlugin,
            base_url: None,
            api_key_env: None,
            model: None,
            path: None,
            max_tokens: None,
        };
        c.providers.push(p2);
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_oversize_journal() {
        let mut c = Config::defaults();
        c.rollback.max_entries = 65_537;
        assert!(c.validate().is_err());
    }

    #[test]
    fn toml_roundtrip_preserves_decisions() {
        let c = Config::defaults();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, c);
    }
}
