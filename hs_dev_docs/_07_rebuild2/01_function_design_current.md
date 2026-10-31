# iA-console 功能设计·现状提取（第二次重建基线）

- **文档性质**：REVERSE 产物——从 v0.2.6 现存代码提取的**实际功能语义**（非理想设计）。事实均有 `文件:行号` 证据；标注约定：✅=真实可用、⚠=半成品/仅部分路径、❌=假象（代码存在但不可达/断裂）、⊘=声明未实现。
- **提取基准**：commit `f79e7e7`（v0.2.6-2），全仓 ~20.8k LOC（core 10.8k / iac 3.0k / iac-gui 2.5k / vendor 2.1k / infra 1.8k / iacd 0.55k）。提取方式：四路只读代理（core、infra+iacd、iac、iac-gui+vendor），2026-10-31。
- **用途**：第二次重建（Q55）的功能设计输入。旧需求语义以 PRD v2.3（Q0~Q54）为准，本文只描述**代码现状**。

---

## 1. 架构总貌（现状）

```
iac(CLI/TUI)  iac-gui(GUI)  iacd --mcp        ← 三个前端，各自实现
     └────────────┼──────────────┘
          UDS JSONL（iac.v1 信封，{v,id,method,params,actor?,session_id?}）
                  ↓
        iacd（组合根，每连接一 tokio task，spawn_blocking 同步 dispatch）
                  ↓
        core::server::dispatch（23 方法，IPC 与 MCP 同表）
                  ↓
     core（域：识别/闸门/执行/会话/方案库/脚本/回滚/翻译/诊断）
                  ↓
     infra（唯一 HTTP 出口 ai/、rusqlite storage、快照仓、进程执行、config IO）
```

**核心事实**：core/infra 层的域语义基本完整且经测试（141 测试）；**坏的集中在「最后一公里」——前端接线与双路径一致性**（见 02 缺陷登记 S0）。

## 2. 按功能域的现状

### D1 输入与混合识别（Q15/Q13/Q28/Q30）✅（core 侧）/ ⚠（前端侧）
- `classify(line, is_command)` 八级优先：`\` 字面 → `---` 分段 → `??`（首词 is_command→AiExplain 否则 AiAsk）→ `#` 备注 → `ia-` 内置（未知名也返回 Builtin，由 dispatch 报 IAC-1501）→ 首词 is_command→LiteralCommand → AiAsk（core/src/common/marker_parser.rs:34-86）。
- `looks_like_command`：20 个 shell 内建名表 → 含 `/` 查 Path::exists → PATH 目录扫描（OnceLock 进程内缓存，不刷新，marker_parser.rs:89-111）。
- ⚠ 前端分流后各写一遍：CLI console.rs:139-234 与 GUI app.rs:863-1033 是两套提交状态机；**同名输入语义不一致**（chat 的 LiteralCommand 走 daemon 记账，GUI 的 LiteralCommand 直写 PTY 不记账）。

### D2 风险闸门与确认升级（Q34~Q37）✅（core 判定）/ ❌（前端确认 UX）
- 规则库：31 条静态规则（destructive 7 / dangerous 12 / caution 12），**命令串子串匹配**（`command.contains(pattern)`，rules.rs:19），未命中=Safe；多命中取最高（rules.rs:20-22）。
- AI 自评合成：AI 等级高于静态则取 AI（risk/mod.rs:28-36）。
- 闸门矩阵（executor/mod.rs:81-110）：**直输（人类）≥Caution → WeakWarning 永不拦截**；Agent ≥Dangerous 未确认 → Reject；Translated/Proposal：≥Destructive→RequireConfirmWord、≥Dangerous→RequireDoubleEnter。
- 确认词：`确认 <命令尾词>` 或裸 `确认`（handler.rs:86）。
- ❌ **GUI 确认状态机整体不可达**：`ConfirmState{Idle,Armed,AwaitWord}` 只有 Idle 赋值点（app.rs:22-30、154、879、1221）——二次 Enter 武装态、确认词输入、Esc 取消全是 UI 假象；实际只认文本 `yes`/`no`。
- ❌ **CLI 确认链断裂**：confirm_last 匹配渲染文本 `    提案 ID：`（console.rs:396-406），全仓无渲染器产出该行 → last_proposal 恒空；且发送参数键 `confirm_dangerous`（console.rs:378），契约字段是 `confirmed`（executor/mod.rs:37）→ 确认位恒 false。
- ⚠ 弱提醒三件套：仅 GUI 提案卡有描边着色（app.rs:1093-1107）；CLI 输入行红字+⚠ 在不可达代码里（tui.rs:55-78、console.rs:440 自注占位）。
- **结构性缺口（Q55 触发点）**：闸门只覆盖 daemon 路径；终端面（PTY 直敲）零闸门、零记账——`rm -rf` 类命令直敲即达 shell。

### D3 执行与账本（Q45/Q47/Q21）✅
- 生命周期：resolve→（Direct+env 声明→只记账不执行）→风险→闸门→升级校验→pre_capture→spawn（timeout=config.exec.timeout_ms）→全量输出入库→insert_execution+audit+session_event（executor/handler.rs:20-72、129-194、311-362）。
- executions 表 19 列（含 stdout_full/stderr_full/journal_ref），id 由 SQL randomblob 生成回填（history/store.rs:43-47）；天数+容量双限清理 + JSONL 导出（history/store.rs:172/279）。
- ⚠ hardcode：执行记录 `shell:"sh"`、`host:"local"`（handler.rs:294-295）。

### D4 会话与恢复（Q46/Q41/Q33）✅（core）/ ⚠（GUI 接线）
- sessions 按 cwd+host 复用（session/store.rs:53-90）；session_events 7 种 kind，PK(session_id,seq)；持久化=命令历史/cwd/声明 env/对话轨/布局。
- ❌ GUI 侧 `iac_sessions_current` 响应分支存在但**全代码无请求方**（app.rs:1261-1286）——会话回放特性空转。
- ⚠ GUI cwd 双源：只有输入行 cd 更新 tab.cwd；终端面 cd 不回写（无 OSC7 跟踪），标题/布局漂移。

### D5 方案库（recipe）✅
- save（name 全局唯一→IAC-4101）/search/delete/export/usage（recipe/handler.rs:16-249）；recipes 表 kind/name/command_template/risk_level/usage_count + 双唯一索引（0001_init.sql:68-84）。

### D6 脚本工坊（make/refine/run，Q51 继承）✅（core）/ ⚠（GUI）
- make：collect 最近 separator 段或 (to,from] 区间 → 确定性零 AI 拼接（sh: `set -euo pipefail`；PS: `$ErrorActionPreference='Stop'`，双版本 `# ---iac:ps---` 分段）→ max_risk 逐条取最大 → 存 kind=script（script/mod.rs:107-188、script_assembler.rs:55-68）。
- refine：confirm=false AI 产稿（temp 0.2，失败降级拼接稿）；confirm=true 必须带 refined_body 覆盖写回。
- run：整体正文 assess_static → gate（Direct）→ 执行记录 `@名`。
- ❌ 数据正确性：collect_last_segment 的 SQL **无 session_id 过滤**（script/collect.rs:69-74）——跨会话取分隔点。

### D7 回滚与文件历史（效果日志方案，Q48/Q49）✅
- pre_capture 按 capture_scope（CwdTree/Explicit/AllHome）捕获，排除 .git/target/node_modules 等，单文件 16MB/总量 128MB 上限，manifest 内容寻址 `sha256:<hex>`（rollback/mod.rs:59-74、capture.rs:49-78）。
- 执行后 diff 产 created/modified/deleted 入 rollback_journal 并回填 journal_ref；超 1024 清退（diff.rs:10-86、161-202）。
- replay：倒序回放，modified 冲突（指纹≠after_ref）→IAC-1603 按 on_conflict；file_history 版本链 + restore 过闸门留痕（replay.rs:10-117、file_history.rs:77-165）。

### D8 配置体系 ✅
- 8 组全键 schema（ui/ai/providers/modelhub/rollback/history/render/exec，config.rs:184-202）；env 覆盖 3 键；校验 IAC-5101；原子写、密钥只经 api_key_env 读环境变量。
- ⚠ CLI `config get` 点路径 fold 遇标量中间节点会 panic（main.rs:456-460）；ia-model 传 `{"key":"ai"}` 但 daemon 忽略参数返回全量（dispatch.rs:97）。

### D9 AI Provider 体系 ✅ / ⚠
- 四 kind：openai/anthropic/local-plugin/local-path；reqwest::blocking 每次新建 Client；Anthropic max_tokens 硬编码 2048；密钥永不落盘（ai/mod.rs:26-187）。
- 上下文注入：cwd/platform/shell/最近 10 命令/对话轨 10 条/钉住备注，同快照入 proposal.context_json（Q20 透明，intent/handler.rs:67-117、155-162）。
- ⚠ local 引擎：句柄 pid 恒 0、ENGINE_CHILDREN 不回收、fallback_local 键无消费方；`ai.fallback_local`/`model_plugins` 表等存在但空转。
- ⊘ 非 Anthropic kind 一律走 openai payload（local-* 靠 OpenAI 兼容端点，ai/mod.rs:51-54）。

### D10 daemon 与 IPC/MCP ✅
- 单实例探测→bind→pid 文件→建库→迁移→select 主循环，每连接一 task、每请求 spawn_blocking；取消=同连接改写 IAC-6107（iacd/src/lib.rs:84-297）。
- MCP：`iacd --mcp` stdio JSON-RPC，9 工具与 IPC 同表，actor=agent:mcp（mcp.rs:12-150）。
- 信封 v1；错误码 24 个集中定义（error.rs:4-30）+ 3 个裸字符串码散落 infra（IAC-1401/1402/1403）。
- GUI 自动拉起：4 次×500ms 退避，exe 同目录 iacd 优先（iac-gui/src/ipc.rs:29-67）。

### D11 GUI 前端 ✅（终端面）/ ❌（提案交互）
- 多窗口 viewport immediate ×窗口内标签×每标签独立 PTY；启动恢复 layout.json（windows[].bounds+tabs[].{title,cwd,session_id}）；cwd 恢复=注入 `cd '{cwd}' && clear`（app.rs:239-269、129-134）。
- ❌ 多候选 ↑↓ 切换、`e` 展开、context_json 展示 UI 全部未实现（entries 每帧克隆只读渲染，expanded 恒 false，Tab.expanded 零使用）。
- ❌ Esc 生成期取消死代码（busy_request 恒 None）；AiError 重试发错方法+空参（app.rs:680、1307）；apply_ipc_result 无条件解锁 busy（迟到回复清掉在途 busy）。
- ❌ PtyEvent::Exit 未处理——shell 退出后标签僵尸。
- ⚠ lib.rs 测试里的 apply_to_entries 是 app.rs 真逻辑的复刻且少字段（双实现漂移）。

### D12 CLI 前端 ✅（基础流）/ ⚠
- 15 子命令 + 全局 --json；console 全屏 / chat 行模式两入口 + 共享 helper；config 向导 19 键 7 组、写回走 iac_config_set 以回包为单一事实。
- ❌ yes/no 确认链断裂（见 D2）；daemon stop 永远打印成功；`iac daemon mcp` 死入口。

### D13 终端集成（受控 shell Q52 + 块式输出 Q54）✅
- 受控启动矩阵：zsh→ZDOTDIR、bash→--rcfile、fish→--no-config（无钩子→无块）、未知→透传；rc 内嵌资产幂等落盘（shell.rs:18-62）。
- 事件侧信道：preexec `S<US>epoch<US>命令`、precmd `E<US>epoch<US>退出码<US>PWD`，GUI 增量 poll（seek+offset），500 块 LRU，cd 注入过滤（blocks.rs）。
- ⚠ fish/未知 shell 静默无块，UI 无提示。

### D14 主题与呈现 ✅ / ⚠
- GUI 16 token / CLI 14 token，Night 12 槽一致、**Day 12 槽全部漂移**（两份手写表，iac/src/theme.rs:52-66 ↔ iac-gui/src/theme.rs:52-135）；终端 ANSI 28 槽夜底 #0f1011。
- vendor/egui-term fork 六处实质变更（输入门槛只看焦点/亚像素 cell 度量/字体外提/订阅线程静默收线/Event 重导出/BackendSettings.env），alacritty_terminal 0.25.0（ADR-010）。

## 3. 核心契约摘录（重建必须保真的接口语义）

1. **dispatch 方法表**：23 方法（iac_ping/builtin/ask/execute/explain/diagnose/history_query/history_clean/history_export/recipes_search/recipe_save/recipe_delete/recipe_export/translate/sessions_current/session_list/session_resume/session_event/rollback/script/config_get/config_set/daemon_stop）——完整入参/出参见 core/src/server/dispatch.rs:76-592。
2. **DB 10 表**：sessions/session_events/proposals/executions(19 列)/recipes/knowledge_cache/audit_log/translations/model_plugins/rollback_journal（0001_init.sql）。
3. **错误码 24 个**（IAC-1101~6107，error.rs:4-30）。
4. **闸门矩阵语义**（executor/mod.rs:81-110）与四级风险语义（Q34）。
5. **脚本双版本格式**（sh/PS 分段标记）、回滚 manifest/journal 语义、翻译/知识缓存 schema。
6. **受控 shell 事件行格式**（S/E US 分隔）、ZDOTDIR/--rcfile 启动矩阵、layout.json schema。
7. **MCP 9 工具清单**与 actor 标识约定（`agent:<id>`）。

## 4. 功能域判定汇总（重建按此收口）

| 域 | 判定 | 重建含义 |
|---|---|---|
| D1 识别 / D3 执行账本 / D5 方案库 / D6 脚本 / D7 回滚 / D8 配置 / D9 AI / D10 daemon | ✅ 语义成立 | 语义与契约保真迁移 |
| D2 闸门 | 判定✅ / 前端❌ / 终端面⊘ | **重建主攻**：单一执行模型，闸门覆盖全路径 |
| D4 会话 | core✅ / GUI 接线❌ | 重建时一并接真 |
| D11/D12 前端 | 各自为政，三份内置映射/两份 IPC/两份主题/双状态体 | 重建主攻：核心运行时 + 薄前端 |
| D13 终端集成 | ✅ 新地基 | 保留并升级为闸门覆盖的挂点 |
