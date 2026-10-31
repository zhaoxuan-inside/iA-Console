# iA-console 缺陷与设计债总登记（v0.2.6 逆向）

- **来源**：四路只读提取（2026-10-31，commit f79e7e7），每条有代码证据。
- **分级**：S0=假象功能（承诺存在但不可达/断裂，最伤产品信用）；S1=数据正确性；S2=行为缺陷；S3=架构债（重建根因）；S4=卫生。
- **用途**：重建验收的对账清单——重建版必须逐条关闭或显式裁定不继承。

## S0 假象功能（11 项）

| # | 缺陷 | 证据 | 影响面 |
|---|---|---|---|
| S0-1 | GUI 确认状态机 Armed/AwaitWord 零迁移入口，Q35~Q37 确认升级 UX 整体是假象 | app.rs:22-30、154、879、1221 | GUI 全部危险/破坏性命令确认 |
| S0-2 | CLI yes/no 确认链断裂：匹配的渲染文本无渲染器产出 | console.rs:396-406（全仓无该格式产出） | CLI 提案确认恒失败 |
| S0-3 | CLI 确认参数键错：发 `confirm_dangerous`，契约字段 `confirmed` | console.rs:378 ↔ executor/mod.rs:37 | CLI 确认位恒 false |
| S0-4 | CLI 弱提醒（红字+⚠）在不可达代码里 | tui.rs:55-78 仅被无调用者的 paragraph 调用；console.rs:440 自注占位 | AC-04 CLI 侧 |
| S0-5 | GUI 多候选 ↑↓ 切换 / `e` 展开 / context_json 展示未实现 | entries 只读渲染、expanded 恒 false、Tab.expanded 零使用 | Q17 GUI 交互 |
| S0-6 | GUI Esc 生成期取消死代码（busy_request 恒 None） | app.rs:757、1180、149 | Q22 GUI 侧 |
| S0-7 | GUI AiError 重试发错方法+空参（params 恒 {}，重试硬编码 iac_ask） | app.rs:680、1307、1318 | 失败恢复 |
| S0-8 | `iac_sessions_current` 响应分支无请求方，会话回放空转 | app.rs:1261-1286 | Q46 GUI 回放 |
| S0-9 | 弱提醒描边仅 GUI 提案卡有；终端面无任何风险呈现 | app.rs:1093-1107；PTY 路径无闸门 | 直输安全语义 |
| S0-10 | `iac daemon mcp` 纯指引死入口 | main.rs:509-512 | 命令面卫生 |
| S0-11 | GUI `iac_execute` 后 proposal 标记链：OfferHelp→诊断可用，但 Armed 提示文案渲染在不可达分支 | app.rs:1129-1133 | 提示信用 |

## S1 数据正确性（6 项）

| # | 缺陷 | 证据 |
|---|---|---|
| S1-1 | 脚本 collect 查最近 separator **无 session_id 过滤**，跨会话取段 | script/collect.rs:69-74 |
| S1-2 | truncate 语义不一致：executor 按字节、script 按 char | handler.rs:381-390 ↔ script/mod.rs:530-537 |
| S1-3 | 数据目录双源：data_dir() 不读 IAC_DATA_DIR，snapstore 读且回退 temp_dir | infra/config.rs:11-15 ↔ snapstore/mod.rs:15-23 |
| S1-4 | cwd 双源漂移：终端面 cd 不回写 tab.cwd（无 OSC7 跟踪），布局/标题/块 cwd 失真 | app.rs:928-941；layout.rs:34 |
| S1-5 | 回滚 list 的 q 过滤在 SQL LIMIT 之后内存执行 | rollback/list.rs:24-46 |
| S1-6 | 错误码语义漂移（INTENT_INVALID vs UNKNOWN_METHOD 混用；RECIPE_NAME_CONFLICT 兼收空标题；IAC-1401~1403 裸字符串未入表） | dispatch.rs:120/147；recipe/handler.rs:24-30；ai/local.rs:44-72 |

## S2 行为缺陷（10 项）

| # | 缺陷 | 证据 |
|---|---|---|
| S2-1 | PtyEvent::Exit 未处理，shell 退出后标签僵尸 | app.rs:189 |
| S2-2 | apply_ipc_result 无条件解锁 busy，迟到回复清掉在途请求 | app.rs:1179 |
| S2-3 | config_set 落盘失败被 `let _=` 吞 | iacd/src/lib.rs:210-213 |
| S2-4 | IPC 解析失败静默 stub id，不回 PROTOCOL_MISMATCH | iacd/src/lib.rs:230-238 |
| S2-5 | daemon stop 请求失败也打印成功 | main.rs:499-508 |
| S2-6 | config get 点路径 fold 遇标量中间节点 panic | main.rs:456-460 |
| S2-7 | Anthropic 头构造 `expect` 可 panic；OpenAI 不发 max_tokens；每调用新建 Client 无复用 | ai/mod.rs:43-45、121-126、154-157 |
| S2-8 | 本地引擎 ENGINE_CHILDREN 只增不清、pid 恒 0，daemon 退出不回收 | ai/local.rs:12、68、119-125 |
| S2-9 | kill_tree 仅 child.kill()，process_group 已设但无 killpg，管道子进程残留 | process/mod.rs:83、118-123 |
| S2-10 | 视口 id 复用冲突（Id::new(windows.len())），窗口 0 bounds 持久化但从不应用 | app.rs:366-370、407 |

## S3 架构债（重建根因，9 项）

| # | 债 | 证据 |
|---|---|---|
| S3-1 | **双执行路径分裂**：终端面（PTY 直达，无闸门/无记账）vs 输入行（daemon 全功能）；同名输入语义不一致（GUI 直写 PTY vs chat 走 daemon） | app.rs:923-951 ↔ tui.rs:145-148 |
| S3-2 | 内置命令映射**三份**：CLI 本地展开 / core route_builtin / dispatch 测试内嵌副本，策略还相反 | console.rs:238-365 ↔ dispatch.rs:735+ |
| S3-3 | IPC 客户端两份（CLI/GUI 各一，能力不对齐：GUI 有拉起/超时/取消，CLI 有 actor） | iac/src/ipc.rs ↔ iac-gui/src/ipc.rs |
| S3-4 | 主题表两份手写，Day 模式 12 槽全部漂移 | iac/src/theme.rs ↔ iac-gui/src/theme.rs |
| S3-5 | 双状态体（TuiState/ConsoleState）+ GUI lib.rs 复刻 app.rs 响应处理少字段 | tui.rs:13-31 ↔ console.rs:30-39；lib.rs:64-132 |
| S3-6 | daemon 拉起逻辑近逐行重复两份 | main.rs:634-670 ↔ iac-gui/ipc.rs:45-67 |
| S3-7 | app.rs 1443 行单文件杂糅渲染/状态机/IPC 分流/shell/主题解析；每帧全量克隆 entries | app.rs 全文、579 |
| S3-8 | Evolution fail/ok 一致性靠调用方自觉，多处 fail 后手工置 ok | framework.rs:151-156；handler.rs:122-124 等 |
| S3-9 | MCP 工具表与 dispatch 方法表双手工维护；run_mcp_mode 与 serve 两份装配 | mcp.rs:11-43；lib.rs:39-77 vs 114-147 |

## S4 卫生（12 项，抽样）

死代码：`risk::assess_level`、`RiskInput.cwd`、replay.rs 空 if、sweep_expired 常量排序、`engine_healthy` 无使用者、model_plugins 表 core 无引用、renamed op 永不产生但回放必失败、提案重复 load_proposal、PATH OnceLock 不失效、recipes 冗余索引、OSC52 回显剥壳再包壳、20 个源文件零内联测试（集成测试仅 3 文件 16 例）。

## 重建验收口径

1. S0 全部关闭：每项 UI 承诺必须有可达代码路径 + 测试。
2. S1/S2 逐条关闭或在 PRD 显式裁定不继承。
3. S3-1（单执行模型）与 S3-2~S3-6（单一事实源）为架构验收硬门。
