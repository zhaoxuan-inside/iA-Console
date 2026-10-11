//! T-036 端到端：v0.1.0 主链路（daemon → CLI ask/run/history → MCP → 翻译）。
//! 更全链路（翻译组合/标记/复制）已分别在 translate_test 与 cli_mcp_test 覆盖。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::JoinHandle;

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("iac-e2e-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn wait_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("socket 未就绪");
}

fn spawn_daemon(dir: &std::path::Path) -> JoinHandle<()> {
    let options = iacd::DaemonOptions {
        socket_path: dir.join("iac").join("daemon.sock"),
        data_dir: dir.to_path_buf(),
        foreground: true,
    };
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    std::thread::spawn(move || {
        let _ = iacd::run(options, rx);
    })
}

fn iac(args: &[&str], dir: &PathBuf) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["--json"])
        .args(args)
        .env("XDG_RUNTIME_DIR", dir)
        .output()
        .expect("run iac");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn e2e_v010_main_loop() {
    let dir = tmp_dir("main");
    let daemon = spawn_daemon(&dir);
    wait_socket(&dir.join("iac").join("daemon.sock"));

    // ① 执行（闸门+三账）
    let out = iac(&["run", "echo e2e-step-1"], &dir);
    assert!(out.contains("\"status\": \"ok\""), "out={out}");

    // ② 历史
    let out = iac(&["history", "search", "e2e-step-1"], &dir);
    assert!(out.contains("\"total\": 1"), "out={out}");

    // ③ 方案库保存与检索
    let out = iac(&["recipe", "save", "清理演示", "--with", "echo clean", "--tags", "演示"], &dir);
    assert!(out.contains("清理演示"), "out={out}");
    let out = iac(&["recipe", "search", "清理演示"], &dir);
    assert!(out.contains("清理演示"));

    // ④ 翻译（静态表零 AI）
    let out = iac(&["translate", "Get-ChildItem", "--from", "windows"], &dir);
    assert!(out.contains("ls"), "out={out}");

    // ⑤ MCP：Agent 列工具并执行（同闸门）
    let mut child = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["mcp", "serve"])
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"iac_execute","arguments":{{"command":"echo mcp-e2e"}}}}}}"#).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("mcp-e2e"), "line={line}");
    drop(stdin);
    let _ = child.wait();

    // ⑥ Agent 高危未确认 → rejected（同闸门，AC-08）
    let mut child = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["mcp", "serve"])
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"iac_execute","arguments":{{"command":"echo 'rm -rf x'"}}}}}}"#).unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("rejected"), "line={line}");
    drop(stdin);
    let _ = child.wait();

    // 收尾：stop
    let _ = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["daemon", "stop"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output();
    daemon.join().expect("daemon join");
}
