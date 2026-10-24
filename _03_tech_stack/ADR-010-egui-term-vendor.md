# ADR-010-egui_term vendor fork（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

用户实测 ia-console 0.1.0 终端"基本输入不可用"：鼠标不在终端区域时按键全部静默丢失（上游 `view.rs` 输入门槛含 `contains_pointer`）；且上游 0.1.0 仅一个版本、无维护活动，缺陷不会由上游修复。另有渲染度量与退出噪音缺陷需一并处置。

## 决策

- egui_term 0.1.0 以 vendor 形式入库 `vendor/egui-term/`，根 `Cargo.toml` 用 `[patch.crates-io]` 指向本地路径（iac-gui 依赖声明不变）。
- 在 vendor 上修四处：①输入门槛只看焦点（去 `contains_pointer`）；②单元尺寸亚像素化——cell 宽高保持 f32，只对行列数取整（原实现对 cell 取 floor 致字形逐列重叠错位）；③渲染逐 cell 字体集克隆外提为每帧一次；④PTY 订阅线程发送失败改优雅收线（原 panic）。
- 重导出 alacritty `Event/Color/NamedColor/Rgb`，应用层可应答 PTY 查询（`PtyWrite`/OSC 10-11 颜色查询）。

## 理由

- 焦点/输入语义在 widget 内部，应用层无法绕过；crate 约 2000 行，vendor 成本低。
- alacritty_terminal 0.25 内核与按键转义表完整正确，全部保留。
- `[patch.crates-io]` 对业务代码零侵入，Cargo.lock 锁定可复现。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| 应用层自写键盘转发绕过 | 需复制整张按键表，长期双份维护 |
| 换终端组件/框架 | 推翻 ADR-008/009，过度工程 |
| 等上游修复 | 上游无维护活动，不可等 |

## 后果

- 正面：输入/渲染正确性可控；终端查询在应用层闭环（vim/nvim 查询不再悬挂）。
- 负面/代价：升级上游需手工 rebase（当前无上游活动，风险低）；vendor 目录入库。
- 对 _00：tech_stack 的 egui_term 条目标注 vendored。

## 补充补丁（0.2.1，2026-10-24）

- `BackendSettings` 增加 `env: HashMap<String,String>` 并透传 `tty::Options.env`（上游缺口）。
- 应用侧注入 `TERM=xterm-256color` / `COLORTERM=truecolor`：修复 PTY 默认 TERM=dumb 破坏现代提示符（starship 等首启报错）。
