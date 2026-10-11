//! iacd：iA-console 守护进程。承载 IPC/MCP 服务与全部业务编排。
//! 入口签名豁免（_00.conventions）。

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("iacd 0.1.0 — iA-console 守护进程");
        println!("用法：iacd [--foreground] [--socket <路径>] [--data <目录>]");
        println!("  --foreground  前台运行（默认亦为前台；后台化由 iac daemon start 负责）");
        return;
    }
    let socket = args
        .iter()
        .position(|a| a == "--socket")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(iacd::default_socket_path);
    let data = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(iacd::default_data_dir);
    let (_, rx) = tokio::sync::mpsc::channel(1);
    println!("iacd 监听 {}: 数据目录 {}", socket.display(), data.display());
    match iacd::run(iacd::DaemonOptions { socket_path: socket, data_dir: data, foreground: true }, rx) {
        Ok(n) => println!("iacd 已退出（服务连接数 {n}）"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

use std::path::PathBuf;
