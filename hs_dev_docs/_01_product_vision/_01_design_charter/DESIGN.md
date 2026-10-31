# 品味宪章（DESIGN.md）——iA-console 重建基线 v0.2.1

- **状态**：`CONFIRMED`（2026-10-24 UI 重设计落地；令牌与实现同源，`iac-gui/src/theme.rs` 为唯一事实源）
- **设计系统**：**Linear 式表面层次**为主系统（surface 四级阶梯承载层级、不用阴影、点缀色克制使用），
  终端调色板参照 **Warp** 的现代克制 ANSI。规范快照：`design/refs/linear.app.design.md`、`design/refs/warp.design.md`
  （来源 [awesome-design-md](https://github.com/VoltAgent/awesome-design-md)，2026 精选）
- **设计语言**：内容即界面、留白、发丝线分隔、8px 圆角、紧凑间距（xs~sm 节奏）

## 1. 原则

1. **表面阶梯承载层级**：canvas → surface-1（条区/终端容器）→ surface-2（卡片/输入框）→ surface-3（悬停），
   层级靠明度阶梯而非阴影（Linear 核心手法）。
2. **点缀色克制**：薰衣草蓝仅用于焦点/活动位/主操作（活动候选左竖条、选区、accent 条），大面积界面保持中性。
3. **终端优先**：管理功能全部在终端面内以等宽文本渲染（域 18，Q38）；GUI 只做窗口外壳、卡片容器与鼠标支持。
4. **风险即色相**：caution=琥珀、dangerous=橙红、destructive=红（等级差用色相而非仅明度）；弱提醒不拦截（Q34）。
5. **圆角与发丝线**：控件 6px、卡片 8px、窗口 10px；1px 发丝线描边，无重边框。

## 2. 令牌表（与 `theme.rs` 同源；色值出处见 rgb() 注释）

| 令牌 | 夜 | 昼 | 语义 |
|---|---|---|---|
| Bg（canvas） | #010102 | #fbfbfc | 最深底 |
| Surface | #0f1011 | #f2f3f5 | 条区容器/终端底 |
| SurfaceRaised | #141516 | #ffffff | 卡片/输入框 |
| SurfaceHover | #18191a | #e9eaee | 悬停面 |
| Hairline | #23252a | #e3e4e8 | 发丝线描边 |
| Fg（ink） | #f7f8f8 | #1c1c1f | 正文 |
| Fg60（subtle） | #8a8f98 | #6c707a | 次级文本 |
| Fg40（tertiary） | #62666d | #9b9ea6 | 三级文本 |
| Fg25 | #4b4f56 | #b7bac1 | 弱化文本 |
| Accent | #828fff（暗底亮调） | #5e6ad2（品牌） | 点缀：焦点/活动位 |
| Safe / Caution | #3dd68c / #ffb224 | #217a53 / #ad5f00 | 风险语义 |
| Dangerous / Destructive | #f76b15 / #ff6369 | #bd4b00 / #d13438 | 橙红→红等级差 |
| SynKeyword / SynString | #b18aff / #45d4b8 | #8250df / #0a7c6e | 语法令牌 |

## 3. 终端 ANSI（夜）

surface-1 暖炭底 + 现代克制 ANSI：blue 槽=品牌 #5e6ad2（bright #828fff）、red #e5484d、green #30a46c、
yellow #ffb224、magenta #bf7af0、cyan #4cc2d1；昼为白底深色变体。完整表见 `terminal_palette()`。

## 4. 布局（域 18 重建落定）

- **每窗口**：标签栏（顶部，圆角选中态）→ 任务条区（surface-1 容器 + 发丝线描边，可折叠）→ 真终端（主体）→ 主输入行（钉底，surface 输入框）。
- **提案卡**：surface-2 卡片 + 发丝线描边 + 8px 圆角；头部「提案 + id + 风险 chip（等级色 18% 底 pill）」；
  活动候选 accent 左竖条 + 风险着色；`e` 展开逐参数解释与注入上下文（Q16/Q17/Q20）。
- **每标签**：独立 PTY 会话 + 独立任务流 + 独立会话绑定（Q39/Q46）。
- **侧栏（辅助）**：对话轨视图（Ctrl+B），不作为 AI 唯一入口（Q7）。

## 5. 字体与字号

- 系统 sans + CJK 运行时注入（Noto/WQY）；终端与任务条等宽。
- 字号阶梯（Linear body-sm/caption 节奏）：Body 13.5 / Button 13 / Small 11.5 / Monospace 13。
