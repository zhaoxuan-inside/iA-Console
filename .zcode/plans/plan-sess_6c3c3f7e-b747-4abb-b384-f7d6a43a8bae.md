# iA Console 独立 GUI 客户端（Konsole 级形态）实施计划

## 背景与产品决策

v0.1.0 的 `iac console` 是 TUI，寄宿在 konsole 里（ADR-003 当初的理由：SSH 远程可用、零外部依赖）。你的要求是**像 Konsole 一样的独立应用**：点图标直接开自己的窗口，不需要任何别的终端。本计划把**独立 GUI 升为主产品形态**，TUI/CLI 降为辅助通道（SSH 远程场景仍用 TUI）。

架构红利兑现：daemon/core/infra **零改动**，GUI 只是第四个前端（与 CLI/TUI/MCP 并列，同走 IPC）。

## 技术方案

- **GUI 框架**：eframe/egui（纯 Rust、即时模式、单二进制；ADR-008）
- **终端内核**：egui_term（egui 终端组件，alacritty_terminal 后端）+ portable-pty（WezTerm 的跨平台 PTY 层）（ADR-009）——真实 shell：`cd`/环境变量持久，vim/htop 等交互程序可用，与 Konsole 同级
- **新增 crate**：`iac-gui`（二进制名 `ia-console`），与 iac/iacd 并列；依赖 iac-core + ipc 客户端
- **界面**（iA-Writer 设计语言延续，14 令牌日夜主题 + 内嵌 JetBrains Mono 字体保证任何系统下渲染一致）：
  - 主区：真终端（多标签）
  - AI 侧栏：ask/解释、提案卡（yes/no 确认后结果以块状插入侧栏）、风险弱提醒
  - 底部/侧栏面板：历史、方案库、会话（走现有 IPC）
  - 输入标记语言在侧栏沿用（??、/、---、#、yc/yt/ya 复制）

## 文档演进（SOP 需求迭代，先行）

- `_01`：G18 独立 GUI 目标、US-23、AC-44~46（窗口直启/真终端/AI 侧栏）
- `_02`：Q11 GUI 框架与终端内核原理对比（egui vs Tauri vs Flutter；egui_term vs 自写 vte）
- `_03`：ADR-008（egui/eframe）、ADR-009（egui_term+portable-pty）；ADR-003 加"形态演进"注记
- `_04`：架构图加 gui 前端；`_05`：新里程碑 M9（GUI 基座）/M10（智能接线+打包）任务卡；`_09` 词条（独立客户端/真终端内核）；`_00` 同步

## 实施步骤（任务卡）

1. **T-G01 文档演进**（上述各层，逐文档提交）
2. **T-G02 GUI 基座**：iac-gui crate、eframe 窗口、主题令牌与内嵌字体、日/夜切换（验收：窗口可开、主题切换、无 konsole 依赖）
3. **T-G03 真终端内核**：egui_term + portable-pty、真实 shell、多标签、滚轮回滚（验收：cd 持久、htop 可跑）
4. **T-G04 AI 侧栏**：ask/解释/提案卡 yes/no、弱提醒、标记语言子集（验收：?? 出提案→yes 执行→结果入块）
5. **T-G05 面板**：历史/方案库/会话面板 + 复制三档（验收：面板数据与 daemon 一致）
6. **T-G06 打包与桌面集成**：release 构建、应用图标、`.desktop` Exec=`ia-console`（**无 konsole 包装**）、安装脚本（cargo install + desktop + icon + 软链）（验收：应用菜单点击直接开窗）
7. **T-G07 收敛验收**：GUI 冒烟 E2E（窗口/主题/终端/AI 全链）、全仓测试、SOP 校验、README 更新

## 风险与对策

- egui_term 处于活跃开发期（0.x）→ 锁 minor；若 API 不稳，回退自接 portable-pty + vte（组件层自写，内核 alacritty_terminal 不变）
- Wayland 下 GPU 上下文 → eframe glow 后端回退
- 打包体积增大（egui + winit）→ 可接受（仍远小于 Electron/Tauri 方案）

## 验收标准

- 应用菜单点击 "iA Console" → 独立窗口直接打开（无终端包装）
- 终端标签内 `cd /tmp` 后新命令 cwd 保持；`htop` 可交互运行
- AI 侧栏 `??` 出提案卡 → yes → 命令经闸门执行并留痕
- 日/夜主题切换全局生效；全部既有测试保持绿 + 新增 GUI 单测
