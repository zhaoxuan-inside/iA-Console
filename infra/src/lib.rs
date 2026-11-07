//! infra：外部适配层（存储/AI/进程/快照仓库/模型源），实现 iac-core ports。
//! 依赖方向：infra → iac-core（单侧）；core 不感知 infra。

pub mod ai;
pub mod config;
pub mod fsutil;
pub mod httpx;
pub mod modelhub;
pub mod process;
pub mod snapstore;
pub mod storage;
