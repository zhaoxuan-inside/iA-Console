//! T-002 配置 Adapter 集成测试（临时路径读写、首跑默认、坏文件拒绝）。

use iac_core::framework::{Evolution, Metrics, TestEnv};
use infra::config::{load_config, save_config_to, ConfigPath};
use infra::storage::Storage;
use std::path::PathBuf;

fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("db");
    TestEnv::new().with_store(Box::new(store))
}

#[test]
fn missing_file_means_first_run_defaults() {
    let env = env_with_db();
    let mut out = iac_core::config::Config::defaults();
    let mut evo = Evolution::default();
    let input = ConfigPath { path: Some(PathBuf::from("/nonexistent/iac/config.toml")) };
    assert!(load_config(&input, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(out, iac_core::config::Config::defaults());
}

#[test]
fn save_then_load_roundtrip() {
    let env = env_with_db();
    let tmp = std::env::temp_dir().join(format!("iac-cfg-{}.toml", std::process::id()));
    let mut cfg = iac_core::config::Config::defaults();
    cfg.ui.theme = iac_core::types::ThemeMode::Day;
    let mut ack = infra::config::ConfigWriteAck { path: PathBuf::new(), bytes: 0 };
    let mut evo = Evolution::default();
    assert!(save_config_to(&tmp, &cfg, &mut ack, &mut evo));
    assert!(ack.bytes > 0);
    let mut out = iac_core::config::Config::defaults();
    assert!(load_config(&ConfigPath { path: Some(tmp.clone()) }, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(out.ui.theme, iac_core::types::ThemeMode::Day);
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn invalid_toml_fails_with_5101() {
    let env = env_with_db();
    let tmp = std::env::temp_dir().join(format!("iac-cfg-bad-{}.toml", std::process::id()));
    std::fs::write(&tmp, "ui = broken [").expect("write");
    let mut out = iac_core::config::Config::defaults();
    let mut evo = Evolution::default();
    assert!(!load_config(&ConfigPath { path: Some(tmp.clone()) }, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-5101"));
    std::fs::remove_file(&tmp).ok();
}
