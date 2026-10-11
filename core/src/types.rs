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
