//! 服务接入域（词典：dispatch）。IPC 与 MCP 共享同一路由（人机同构，AC-08）。

pub mod dispatch;
pub mod envelope;

pub use dispatch::dispatch_request;
pub use envelope::{IpcRequest, IpcResponse};
