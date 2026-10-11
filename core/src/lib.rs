//! iA-console 领域与应用逻辑库。
//!
//! 分层（见 _04_framework_design）：app 编排 / domain 数据+算法 / common 纯算法。
//! 本 crate 不依赖任何第三方行为组件，外部触达一律经 `infra` 实现的 ports trait。

pub mod config;
pub mod error;
pub mod framework;
pub mod ports;
pub mod session;
pub mod types;
