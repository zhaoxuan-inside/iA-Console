//! iA-console 基础设施层：外部组件唯一封装入口（Adapter）。
//!
//! 模块随任务卡逐个落地：storage / process / ai / config / modelhub / snapstore。
//! 每个外部依赖有且只有一个封装入口；第三方异常在此转译为 IAC 错误码 + evolution。

pub mod config;
