//! T-021 daemon 集成测试：启动 → ping → execute → 单实例 → stop 退出。
//! daemon 在独立线程运行（run() 阻塞），测试用阻塞 UnixStream 客户端驱动。

use iacd::DaemonOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("iac-daemon-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn request(stream: &mut UnixStream, id: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let line = serde_json::json!({"v":1, "id": id, "method": method, "params": params}).to_string();
    stream.write_all(format!("{line}\n").as_bytes()).expect("write");
    stream.flush().expect("flush");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read");
    serde_json::from_str(line.trim()).expect("响应 JSON")
}

fn wait_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("socket 未就绪: {}", path.display());
}

#[test]
fn daemon_serves_ping_execute_and_stops() {
    let dir = tmp_dir("main");
    let socket = Arc::new(dir.join("daemon.sock"));
    let options = DaemonOptions { socket_path: (*socket).clone(), data_dir: dir.clone(), foreground: true };
    let (_keep_tx, rx) = tokio::sync::mpsc::channel(1);
    let handle = std::thread::spawn(move || iacd::run(options, rx));

    wait_socket(&socket);
    let mut stream = UnixStream::connect(socket.as_path()).expect("connect");

    // ping
    let resp = request(&mut stream, "t1", "iac_ping", serde_json::json!({}));
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["result"]["pong"], true);

    // execute（真实进程）
    let resp = request(&mut stream, "t2", "iac_execute", serde_json::json!({"command": "echo daemon-ok"}));
    assert_eq!(resp["ok"], true, "resp={resp}");
    assert_eq!(resp["result"]["status"], "ok");
    assert!(resp["result"]["stdout_summary"].as_str().unwrap().contains("daemon-ok"));

    // 单实例：同 socket 二次启动应失败（IAC-6103）
    let (_tx2, rx2) = tokio::sync::mpsc::channel(1);
    let second_socket = Arc::clone(&socket);
    let second = std::thread::spawn(move || {
        iacd::run(DaemonOptions { socket_path: (*second_socket).clone(), data_dir: tmp_dir("second"), foreground: true }, rx2)
    });
    let second_err = second.join().expect("join");
    assert!(second_err.is_err(), "第二个 daemon 应启动失败");
    assert!(second_err.unwrap_err().to_string().contains("IAC-6103"));

    // stop → daemon 自行退出
    request(&mut stream, "t3", "iac_daemon_stop", serde_json::json!({}));
    let _ = handle.join().expect("daemon 退出");
    assert!(!socket.exists(), "退出后 socket 清理");
}
