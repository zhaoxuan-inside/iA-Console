# ADR-001-开发语言选 Rust（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

iA-console 需要交付三样东西：常驻后台 daemon（长驻、低资源、高可靠）、交互式 TUI 前端、跨 Linux/macOS/Windows 分发（痛点用户多为非专家，安装必须零依赖一步到位）。_02 的结论要求本地 SQLite、MCP Server、OpenAI 兼容 HTTP 客户端与终端控制能力。

## 决策

选定 **Rust（stable 通道，Cargo.lock 锁定）** 作为唯一开发语言。

## 理由

- 单静态二进制跨平台分发，用户无需装运行时——"天下没有难用的 Console"的第一步是安装不难。
- daemon 长驻资源占用低（无 GC 抖动、内存占用 MB 级），适合伴生形态常开。
- 终端工具生态成熟且先例充分：atuin、zoxide、bat 均为 Rust；ratatui/crossterm 覆盖 TUI 需求。
- 官方 MCP Rust SDK（rmcp）可用，满足痛点 9 的 Agent 接入。
- 内存安全保证 daemon 长期运行不因野指针/越界崩溃。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| Go | 开发速度快、并发简单，但二进制体积更大、TUI 生态（Bubbletea）需绑定 Go 工具链；与 Rust 差距主要在开发速度而非能力，综合分发与资源占用后仍选 Rust |
| TypeScript/Node | MCP 生态第一顺位，但分发需要运行时（或打包出巨大二进制），daemon 内存占用高，与"零依赖伴生"定位冲突 |

## 后果

- 正面：分发、资源、可靠性、终端生态四项全占。
- 负面/代价：开发速度慢于 Go，编译期长，async Rust 学习曲线陡（需在 _04 公共机制里固化使用模式）。
- 对 _04 设计的影响：workspace 拆分为 iac/iacd/core/infra 四 crate 以控编译范围；trait（ports）先行使 core 可脱离 infra 单测。
