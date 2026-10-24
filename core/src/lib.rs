//! iac-core：伴生智能内核（纯域层，依赖倒置见 ports.rs）。
//! 重建基线：PRD v1.4（Q0~Q49 已确认项）+ 现有功能语义平价（2026-10-24 开发者重建启动令）。

pub mod builtin;
pub mod common;
pub mod config;
pub mod doctor;
pub mod error;
pub mod executor;
pub mod framework;
pub mod history;
pub mod intent;
pub mod knowledge;
pub mod ports;
pub mod recipe;
pub mod risk;
pub mod rollback;
pub mod script;
pub mod server;
pub mod session;
pub mod types;
