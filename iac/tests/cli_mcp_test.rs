//! T-022/T-023 CLI 与 MCP 集成测试：真实 daemon + 真实二进制（CARGO_BIN_EXE）。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("iac-cli-{tag}-{}", std::process::id()));
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

fn spawn_daemon(dir: &std::path::Path) -> std::thread::JoinHandle<()> {
    let options = iacd::DaemonOptions { socket_path: dir.join("iac").join("daemon.sock"), data_dir: dir.to_path_buf(), foreground: true };
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    std::thread::spawn(move || {
        let _ = iacd::run(options, rx);
    })
}

#[test]
fn cli_ask_and_history_via_real_daemon() {
    let dir = tmp_dir("ask");
    let daemon = spawn_daemon(&dir);
    wait_socket(&dir.join("iac").join("daemon.sock"));

    // iac ask --json（无 AI 后端 → IAC-1101 错误信封；CLI 退出码 1 且输出错误码）
    let out = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["--json", "ask", "列出文件", "--cwd", "."])
        .env("XDG_RUNTIME_DIR", &dir)
        .output()
        .expect("run iac");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("IAC-1101"), "无 AI 后端应返回 1101：{stdout}");
    assert_eq!(out.status.code(), Some(1));

    // iac run --json（真实进程执行）
    let out = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["--json", "run", "echo cli-e2e"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output()
        .expect("run iac");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"status\": \"ok\""), "stdout={stdout}");
    assert!(stdout.contains("cli-e2e"));

    // iac history search --json
    let out = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["--json", "history", "search", "cli-e2e"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output()
        .expect("run iac");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"total\": 1"), "stdout={stdout}");

    // 收尾
    let _ = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["daemon", "stop"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output();
    daemon.join().expect("daemon join");
}

#[test]
fn cli_daemon_status_without_daemon() {
    let dir = tmp_dir("status");
    let out = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["daemon", "status"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output()
        .expect("run iac");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("未运行"));
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn mcp_serve_initialize_and_tools_list() {
    let dir = tmp_dir("mcp");
    let daemon = spawn_daemon(&dir);
    wait_socket(&dir.join("iac").join("daemon.sock"));

    let mut child = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["mcp", "serve"])
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn iac mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    // initialize
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("ia-console"), "line={line}");

    // tools/list
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#).unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("iac_execute"), "line={line}");
    assert!(line.contains("iac_ask"));

    // tools/call：iac_execute（agent 路径）
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"iac_execute","arguments":{{"command":"echo mcp-e2e"}}}}}}"#).unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("mcp-e2e"), "line={line}");

    // 未知方法 → -32601
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":4,"method":"bogus","params":{{}}}}"#).unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("-32601"));

    drop(stdin);
    child.wait().expect("mcp exit");
    let _ = Command::new(env!("CARGO_BIN_EXE_iac"))
        .args(["daemon", "stop"])
        .env("XDG_RUNTIME_DIR", &dir)
        .output();
    daemon.join().expect("daemon join");
}
