//! 配置 Adapter（IF-006/IF-007）：TOML 文件 IO + 环境变量覆盖 + 校验。
//! 第三方组件：toml / dirs / std::fs——业务层禁直接触达，一律经本模块。

use core::config::Config;
use core::error::codes;
use core::framework::{Context, Evolution, Metrics};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// load_config 输入：显式路径为空时用平台缺省路径。
#[derive(Debug, Clone, Default)]
pub struct ConfigPath {
    pub path: Option<PathBuf>,
}

/// save_config 输出确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWriteAck {
    pub path: PathBuf,
    pub bytes: usize,
}

/// 平台缺省配置路径：`$XDG_CONFIG_HOME|~/.config /iac/config.toml`（Windows: %APPDATA%）。
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("iac").join("config.toml"))
}

/// IF-006：加载配置。文件缺失=首次运行给默认值；解析/校验失败经 evolution 返回 IAC-5101。
pub fn load_config(
    input: &ConfigPath,
    output: &mut Config,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(path) = input.path.clone().or_else(default_config_path) else {
        *output = Config::defaults();
        return true;
    };
    match read_file(&path) {
        Ok(None) => {
            *output = Config::defaults();
            metrics.counter("config.first_run", 1);
            true
        }
        Ok(Some(raw)) => parse_and_fill(&raw, output, metrics, evolution),
        Err(e) => {
            evolution.fail(codes::CONFIG_IO, format!("读取配置失败: {e}"));
            false
        }
    }
}

/// IF-007：保存配置到平台缺省路径（密钥永不入盘，仅存 api_key_env 变量名）。
pub fn save_config(
    input: &Config,
    output: &mut ConfigWriteAck,
    _context: &Context,
    _metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(path) = default_config_path() else {
        evolution.fail(codes::CONFIG_IO, "无法定位配置目录");
        return false;
    };
    save_config_to(&path, input, output, evolution)
}

/// 实际写盘实现（save_config 的路径解析包装；测试可用临时路径直调）。
pub fn save_config_to(
    path: &Path,
    input: &Config,
    output: &mut ConfigWriteAck,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if let Err(msg) = input.validate() {
        evolution.fail(codes::CONFIG_INVALID, msg);
        return false;
    }
    let body = match toml::to_string_pretty(input) {
        Ok(s) => s,
        Err(e) => {
            evolution.fail(codes::CONFIG_INVALID, format!("配置序列化失败: {e}"));
            return false;
        }
    };
    match write_atomic(path, &body) {
        Ok(bytes) => {
            output.path = path.to_path_buf();
            output.bytes = bytes;
            true
        }
        Err(e) => {
            evolution.fail(codes::CONFIG_WRITE, format!("配置写入失败: {e}"));
            false
        }
    }
}

/// 读文件：不存在视为首次运行（Ok(None)），其余 IO 错误上抛转译。
fn read_file(path: &Path) -> Result<Option<String>, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// 解析 + env 覆盖 + 校验 + 填充 output。
fn parse_and_fill(
    raw: &str,
    output: &mut Config,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    match toml::from_str::<Config>(raw) {
        Ok(mut c) => {
            c.apply_env();
            if let Err(msg) = c.validate() {
                evolution.fail(codes::CONFIG_INVALID, msg);
                return false;
            }
            *output = c;
            metrics.counter("config.loaded", 1);
            true
        }
        Err(e) => {
            evolution.fail(codes::CONFIG_INVALID, format!("配置解析失败: {e}"));
            false
        }
    }
}

/// 原子写：先写临时文件再改名，避免半写文件。
fn write_atomic(path: &Path, body: &str) -> std::io::Result<usize> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(body.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::framework::TestEnv;

    fn env_fixture() -> TestEnv {
        TestEnv::new()
    }

    fn ack() -> ConfigWriteAck {
        ConfigWriteAck { path: PathBuf::new(), bytes: 0 }
    }

    #[test]
    fn missing_file_means_first_run_defaults() {
        let env = env_fixture();
        let mut out = Config::defaults();
        let mut evo = Evolution::default();
        let input = ConfigPath { path: Some(PathBuf::from("/nonexistent/iac/config.toml")) };
        assert!(load_config(&input, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
        assert_eq!(out, Config::defaults());
        assert!(evo.ok);
    }

    #[test]
    fn save_then_load_roundtrip_via_explicit_paths() {
        let tmp = std::env::temp_dir().join(format!("iac-test-{}.toml", std::process::id()));
        let env = env_fixture();
        let mut cfg = Config::defaults();
        cfg.providers.push(core::config::ProviderProfile {
            name: "glm".into(),
            kind: core::types::ProviderKind::LocalPlugin,
            base_url: None,
            api_key_env: None,
            model: None,
            path: None,
        });
        cfg.ai.default_provider = "glm".into();
        let mut ack = ack();
        let mut evo = Evolution::default();
        assert!(save_config_to(&tmp, &cfg, &mut ack, &mut evo));
        assert_eq!(ack.path, tmp);
        assert!(ack.bytes > 0);
        // 临时目录里的文件不含密钥字段：内容可读且无 key 字样
        let on_disk = std::fs::read_to_string(&tmp).unwrap();
        assert!(!on_disk.contains("api_key ="));
        let mut out = Config::defaults();
        let input = ConfigPath { path: Some(tmp.clone()) };
        assert!(load_config(&input, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
        assert_eq!(out.ai.default_provider, "glm");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn invalid_toml_fails_with_5101() {
        let tmp = std::env::temp_dir().join(format!("iac-bad-{}.toml", std::process::id()));
        std::fs::write(&tmp, "ui = not valid [").unwrap();
        let env = env_fixture();
        let mut out = Config::defaults();
        let mut evo = Evolution::default();
        let input = ConfigPath { path: Some(tmp.clone()) };
        assert!(!load_config(&input, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
        assert_eq!(evo.reason_code.as_deref(), Some(codes::CONFIG_INVALID));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn invalid_values_fail_validation() {
        let tmp = std::env::temp_dir().join(format!("iac-over-{}.toml", std::process::id()));
        std::fs::write(&tmp, "[rollback]\nmax_entries = 70000").unwrap();
        let env = env_fixture();
        let mut out = Config::defaults();
        let mut evo = Evolution::default();
        let input = ConfigPath { path: Some(tmp.clone()) };
        assert!(!load_config(&input, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
        assert_eq!(evo.reason_code.as_deref(), Some(codes::CONFIG_INVALID));
        std::fs::remove_file(&tmp).ok();
    }
}
