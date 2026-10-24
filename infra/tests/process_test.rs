//! T-005 进程 Adapter 测试：输出捕获、非零退出、超时终止、cwd/env、平台 shell。

use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{ProcessResult, ProcessSpec, StorePort};
use infra::process::spawn_process;
use std::path::PathBuf;

fn run(command: &str, timeout_ms: u64, cwd: Option<PathBuf>, env: Vec<(String, String)>) -> (bool, ProcessResult, Evolution) {
    let envf = TestEnv::new();
    let spec = ProcessSpec { command: command.into(), cwd, env, timeout_ms };
    let mut out = ProcessResult::default();
    let mut evo = Evolution::default();
    let ok = spawn_process(&spec, &mut out, &envf.context(), &mut Metrics::default(), &mut evo);
    (ok, out, evo)
}

#[test]
fn captures_stdout_and_zero_exit() {
    let (ok, out, evo) = run("printf 'hello iac'", 10_000, None, vec![]);
    assert!(ok);
    assert!(evo.ok);
    assert_eq!(out.exit_code, Some(0));
    assert_eq!(out.stdout, "hello iac");
    assert_eq!(out.shell_used, "sh");
    assert!(out.duration_ms < 10_000);
}

#[test]
fn nonzero_exit_is_normal_output() {
    let (ok, out, evo) = run("echo oops >&2; exit 3", 10_000, None, vec![]);
    assert!(ok, "业务上的非零退出属于成功执行");
    assert!(evo.ok);
    assert_eq!(out.exit_code, Some(3));
    assert!(out.stderr.contains("oops"));
}

#[test]
fn timeout_kills_process_group_and_fails() {
    let (ok, out, evo) = run("sleep 5", 200, None, vec![]);
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-2103"));
    assert_eq!(out.exit_code, None);
    // 进程组已终止：总耗时应远小于 5s（杀掉后快速返回）
    assert!(out.duration_ms < 3_000, "实际 {}ms", out.duration_ms);
}

#[test]
fn cwd_and_env_are_applied() {
    let dir = std::env::temp_dir();
    let (ok, out, _) = run("pwd; printf \"$IAC_TEST_VAR\"", 10_000, Some(dir.clone()), vec![("IAC_TEST_VAR".into(), "v1".into())]);
    assert!(ok);
    assert_eq!(out.stdout.trim_end(), format!("{}\nv1", dir.display()));
}

#[test]
fn store_trait_object_still_compiles_for_dox() {
    // 守卫：StorePort 作为 trait 对象可用（Deps 持有 Box<dyn StorePort>）
    fn assert_store(_: &dyn StorePort) {}
    assert_store(&iac_core::ports::NullStore);
}
