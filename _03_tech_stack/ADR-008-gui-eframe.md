# ADR-008-独立 GUI 客户端选 egui/eframe（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

产品形态演进（G18）：独立 GUI 升为主形态，要求"像 Konsole 一样"——点击图标直接打开自己的窗口、内置真终端、AI 侧栏。_02 Q11 对比 egui/Tauri/Flutter/自绘四路径。

## 决策

GUI 框架选定 **egui/eframe**（纯 Rust 即时模式）；终端内核组件选定 **egui_term**（alacritty_terminal 后端）+ **portable-pty**（ADR-009）。新增前端 crate `iac-gui`（二进制 `ia-console`）。

## 理由

- 纯 Rust 单栈：与内核同语言（延续 ADR-001 代码统一性初衷；Tauri=双栈、Flutter=双语言均否决）
- 单二进制分发不变；egui_term 直接提供 Konsole 级终端组件（alacritty 内核），避免重写终端仿真
- 即时模式适合"命令块流 + 侧栏面板"的流式界面；14 令牌主题直映 egui Style

## 备选方案

| 方案 | 未选原因 |
|---|---|
| Tauri | Web 前端双技术栈、打包体积大；终端仿真需 xterm.js 桥接 |
| Flutter | 双语言 + Linux 桌面弱 + SSH 失效（ADR-003 已论证） |
| 自写 vte 网格 | alacritty 数万行核心重写，工程黑洞（仅作回退） |

## 后果

- 正面：产品形态一步到位（独立窗口 + 真终端 + AI 侧栏）；内核与数据零改动。
- 负面/代价：egui_term 0.x API 变动风险（锁 minor + 回退路径）；GUI 依赖 GPU 上下文（glow 回退）；二进制增大。
- 对 _04 影响：新增 gui 前端（第四前端），架构分层不变。
