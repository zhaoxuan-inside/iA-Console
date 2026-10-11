# ADR-002-异步运行时选 tokio（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

daemon 需同时承载：UDS/NamedPipe 多客户端连接、MCP stdio、子进程执行（长输出流式捕获）、定时清理。均为 IO 密集，需要异步运行时。ADR-001 已定 Rust。

## 决策

选定 **tokio**（1.x，features = full），全项目唯一异步运行时。

## 理由

- Rust 异步事实标准：net/process/signal/sync/io-util 子能力齐全，UDS、NamedPipe、子进程流式输出均有现成 API。
- 生态配套最全（tracing、hyper/reqwest 等均原生 tokio）。
- 维护活跃度与文档覆盖远超备选。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| async-std | 维护趋停，生态萎缩 |
| smol | 轻量但子能力需拼装，NamedPipe 等平台细节覆盖不全 |

## 后果

- 正面：平台细节（Windows NamedPipe、子进程 kill on drop）有官方级实现。
- 负面/代价：二进制增大、async 传染至 core 层（在 _04 约束：core 方法以同步五段为主，仅 server 接入层与 process Adapter 为 async，边界处 block_on/桥接）。
- 对 _04 设计的影响：接入层（server/tui 事件循环）async，领域层保持同步可测；process Adapter 内部 async、对外同步语义封装。
