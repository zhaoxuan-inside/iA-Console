//! 服务接入域（词典：dispatch）。IPC 与 MCP 共享同一路由（人机同构，AC-08）。

pub mod dispatch;
pub mod envelope;

pub use dispatch::dispatch_request;
pub use envelope::{IpcRequest, IpcResponse};

/// 缺省 socket 路径：$XDG_RUNTIME_DIR/iac/daemon.sock，回退 /tmp（ADR-005；iac/iacd 共用）。
pub fn default_socket_path() -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| std::path::PathBuf::from(d).join("iac").join("daemon.sock"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/iac-daemon.sock"))
}
