//! iacd：iA-console 守护进程。UDS IPC 服务 + daemon 内置 MCP（Q43）。
//! 入口签名豁免（_00.conventions）。

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("iacd 0.2.0 — iA-console 守护进程（重建基线）");
        println!("用法：iacd [--foreground] [--socket <路径>] [--data <目录>] [--mcp]");
        println!("  --foreground  前台运行（默认亦为前台；后台化由 iac daemon start 负责）");
        println!("  --mcp         daemon 内置 MCP 服务（stdio JSON-RPC，Q43；不监听 socket）");
        return;
    }
    let mcp_mode = args.iter().any(|a| a == "--mcp");
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

    if mcp_mode {
        std::process::exit(iacd::run_mcp_mode(&data));
    }
    println!(
        "iacd 监听 {}: 数据目录 {}",
        socket.display(),
        data.display()
    );
    match iacd::run(iacd::DaemonOptions {
        socket_path: socket,
        data_dir: data,
        foreground: true,
    }) {
        Ok(n) => println!("iacd 已退出（服务连接数 {n}）"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
