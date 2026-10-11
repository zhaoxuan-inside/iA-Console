# ADR-009-真终端内核选 egui_term + portable-pty（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

独立 GUI 的终端标签必须"和 Konsole 一样"：真实 shell 会话（cd/环境变量持久）、vim/htop 等全屏交互程序可用（AC-45）。这需要完整终端仿真（VT100/xterm 转义序列解析 + 网格状态机）+ 伪终端（PTY）。

## 决策

- **PTY 层**：portable-pty（WezTerm 项目出品，MIT/Apache-2.0，跨平台 winpty/conpty/openpty 封装）
- **仿真内核**：alacritty_terminal（alacritty 的核心 crate，MIT/Apache-2.0）
- **egui 组件**：egui_term（封装上述两者；若 API 不稳则回退：portable-pty + vte + 自绘网格组件，内核不变）

## 理由

- 复用 alacritty（Rust 终端事实标杆）数万行验证过的仿真内核，不自研
- portable-pty 是 Rust 生态 PTY 事实标准（wezterm/多个项目在用），三平台覆盖
- 全部 MIT/Apache 许可，与依赖账本一致

## 备选方案

| 方案 | 未选原因 |
|---|---|
| 自写仿真器 | alacritty 级别工作量，数万行，非本项目核心竞争力 |
| 不做真终端（仅智能块） | 交互程序不可用，不满足"像 Konsole"的产品要求（可作 v1.1 前置过渡，不满足本期目标） |
| webview + xterm.js | 与 ADR-008 绑定 Tauri，已否决 |

## 后果

- 正面：Konsole 同级终端能力；仿真与渲染解耦（内核可换）。
- 负面/代价：多两个 0.x 依赖（锁 minor）；PTY 读取需独立线程 + 通道桥接 egui 帧。
- 对 _04 影响：gui 前端内部新增 terminal 子模块；PTY 生命周期归 GUI 前端，不入 daemon。
