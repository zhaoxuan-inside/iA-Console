# 01 对外 API 规范（_06_interface_design）

> 状态：draft　更新：2026-10-11

## 1. 协议与风格

三类接入面，共享同一 dispatch（I-030）与同一闸门/审计语义：

| 接入面 | 协议 | 使用者 | 认证 |
|---|---|---|---|
| CLI（`iac <subcommand>`） | 进程参数 + stdout（人读文本 / `--json` 结构化） | 人类、脚本 | 本机用户 |
| MCP（`iac mcp serve`，stdio JSON-RPC） | MCP tools，`iac_` 前缀 | 外部 Agent | 派生进程的本机用户 |
| IPC（UDS / NamedPipe，JSONL 信封） | `{v,id,method,params}` → `{v,id,ok,result\|error}` | TUI、本地集成 | socket 文件权限 / 管道 ACL（本机用户） |

- 内容类型 JSON（UTF-8）；时间统一 unix 毫秒；ID 统一 UUIDv4（storage Adapter 生成）。
- IPC 与 MCP 工具一一对应：`method` 名即 MCP tool 名（`iac_*`），payload 同构。

## 2. API 清单

### 2.1 CLI 子命令

| 编号 | 命令 | 用途 | 对应 _05 任务 | 模块 |
|---|---|---|---|---|
| CMD-001 | `iac ask <意图> [--json] [--cwd <dir>]` | 意图→提案 | T-016 | intent |
| CMD-002 | `iac explain <命令...>` | 参数白话解释 | T-016 | intent |
| CMD-003 | `iac run <命令...> [--yes] [--cwd <dir>]` | 执行（过闸门） | T-017 | executor |
| CMD-004 | `iac run --proposal <id> [--yes]` | 执行已确认提案 | T-017 | executor |
| CMD-005 | `iac doctor [--last\|--execution <id>]` | 诊断失败 | T-018 | doctor |
| CMD-006 | `iac history list\|show <id>\|search <q>` | 历史检索 | T-008/009 | history |
| CMD-007 | `iac recipe save\|list\|search\|show\|rm` | 方案库 | T-019 | recipe |
| CMD-008 | `iac session new\|list\|show <id>\|resume <id>` | 会话管理 | T-007/010 | session |
| CMD-009 | `iac daemon start\|stop\|status [--foreground]` | daemon 生命周期 | T-021 | server |
| CMD-010 | `iac mcp serve` | MCP stdio 服务 | T-023 | server |
| CMD-011 | `iac chat` | TUI 交互界面 | T-024 | tui |
| CMD-012 | `iac config get <key>\|set <key> <val>\|path` | 配置管理 | T-002 | config |
| CMD-013 | `iac translate <命令...> [--from <linux\|macos\|windows>]` | 习惯方言→主机方言翻译 | T-025/032 | intent |
| CMD-014 | `iac model install <plugin_id>\|list\|remove <plugin_id>` | 模型插件管理 | T-026 | modelhub |

### 2.2 MCP 工具 / IPC methods（同名同构）

| 编号 | 工具名 | 用途 | 对应 _05 任务 | 模块 |
|---|---|---|---|---|
| API-001 | `iac_ask` | 意图→提案 | T-016 | intent |
| API-002 | `iac_execute` | 执行命令（闸门+审计） | T-017 | executor |
| API-003 | `iac_history_query` | 检索执行记录 | T-009 | history |
| API-004 | `iac_explain` | 参数解释 | T-016 | intent |
| API-005 | `iac_diagnose` | 诊断指定/最近失败 | T-018 | doctor |
| API-006 | `iac_recipes_search` | 检索方案库 | T-019 | recipe |
| API-007 | `iac_recipe_save` | 保存方案 | T-019 | recipe |
| API-008 | `iac_sessions_current` | 当前会话信息 | T-007 | session |
| API-009 | `iac_config_get` | 配置只读（不含密钥） | T-002 | config |
| API-010 | `iac_translate` | 跨方言命令翻译 | T-025 | intent |
| API-011 | `iac_model_list` | 已装模型插件列表 | T-026 | modelhub |

## 3. 接口定义（关键 API）

### API-001 `iac_ask`

- **请求**：

| 字段 | 类型 | 必填 | 校验 |
|---|---|---|---|
| intent | string | 是 | 1~2000 字符，非全空白 |
| cwd | string | 否 | 默认当前目录 |
| platform | string | 否 | 默认运行时检测（linux/macos/windows） |

- **响应**（Proposal）：

| 字段 | 类型 | 说明 |
|---|---|---|
| proposal_id | string(uuid) | 提案 ID |
| command | string | 完整命令串 |
| argv | string[] | 参数数组形态 |
| explanation | ParamSpecDoc | 逐参数白话解释 + 影响面说明 |
| risk_level | string | safe/caution/dangerous/destructive |
| platform | string | 目标平台 |
| verified | bool | 命令知识是否命中（false=纯 LLM 未验证） |

- **错误**：IAC-1101 / IAC-1102 / IAC-1103 / IAC-3102 / IAC-5102
- **示例**：

```json
{"intent": "找出最近7天修改过的10个最大文件", "cwd": "/var/log"}
→ {"proposal_id":"018f...","command":"find /var/log -type f -mtime -7 -printf '%s %p\\n' | sort -rn | head -10",
   "argv":["find","/var/log","-type","-mtime","-7",...],
   "explanation":{"params":[{"flag":"-mtime -7","meaning":"最近 7 天内修改"},...],"impact":"只读操作，无文件修改"},
   "risk_level":"safe","platform":"linux","verified":true}
```

### API-002 `iac_execute`

- **请求**：

| 字段 | 类型 | 必填 | 校验 |
|---|---|---|---|
| command | string | 是 | 非空（与 proposal_id 二选一） |
| proposal_id | string | 否 | 执行既有提案 |
| session_id | string | 否 | 默认当前会话 |
| confirm_dangerous | bool | 否 | risk≥dangerous 时必须为 true |
| timeout_ms | int | 否 | 默认 120000，上限 3600000 |

- **响应**（ExecutionOutcome）：

| 字段 | 类型 | 说明 |
|---|---|---|
| execution_id | string | 执行记录 ID |
| exit_code | int/null | null=超时或被取消 |
| status | string | ok/failed/timeout/cancelled/rejected |
| stdout_summary / stderr_summary | string | 截断摘要（64KB） |
| duration_ms | int | 耗时 |
| diagnosis | Diagnosis/null | 失败时附带 |
| gate | object | 风险判定与是否要求确认 |

- **错误**：IAC-2101~2106 / IAC-3101 / IAC-3102
- **Agent 行为约定**：destructive 必须携带 `confirm_dangerous: true`，否则返回 gate 影响面说明并拒绝（与人同闸门，AC-08）。

### API-003 `iac_history_query`

- **请求**：`q`（命令/输出关键词）、`cwd`、`since`/`until`（unix ms）、`status`、`actor`、`limit`（默认 50，上限 500）、`offset`
- **响应**：`{total, items: ExecutionRecord[]}`（字段见 internal 数据模型）
- **错误**：IAC-3102

### API-005 `iac_diagnose`

- **请求**：`execution_id`（可空=最近一次失败执行）
- **响应**：Diagnosis：`{root_cause, evidence[](stderr 原文引用), confidence(0~1), fix_proposals: FixProposal[], rollback_note}`（v1 恒提示无自动回滚）
- **错误**：IAC-1201 / IAC-2101(记录不存在→3101 语义归 3101)

### API-010 `iac_translate`

- **请求**：`command`（必填，非空）、`from`（习惯方言，缺省取配置 `ui.habit_dialect`）、`to`（目标平台，缺省运行时检测）
- **响应**（TranslationResult）：`{translated_command, notes(差异说明), source(static|ai), verified, risk_level}`
- **行为**：习惯方言=目标平台 → 原样返回（source=native）；静态映射命中零 AI 调用（AC-20）；未命中走 AI 翻译并回写缓存；AI 不可用返回原命令+说明（IAC-1301）。翻译结果一律为提案，须经确认执行。
- **错误**：IAC-1301 / IAC-1101 / IAC-5102

### API-011 `iac_model_list`

- **请求**：无参
- **响应**：`{items: [{plugin_id, name, engine, model, size_bytes, status(available|active|broken), installed_at}]}`
- **错误**：IAC-3102；安装/删除经 CLI（CMD-014）触发，Agent 侧 v1 不开放写操作。

### 其余 API（API-004/006/007/008/009）payload 按 internal 数据模型同构映射，实现前如需调整在 _05 任务卡内确认并回填本文件。

## 4. 错误码表

| 错误码 | HTTP 语义* | 含义 | 调用方应对 |
|---|---|---|---|
| IAC-1101 | 503 | AI 后端不可达或超时 | 检查网络与 Provider 配置；直接执行功能不受影响 |
| IAC-1102 | 502 | 提案解析失败（重试 1 次后） | 更换模型或重试；原始响应已留存 |
| IAC-1103 | 400 | 意图为空/超长 | 修正输入（≤2000 字符） |
| IAC-1104 | 404 | 参数解释知识不可得 | 回退查看 man/--help |
| IAC-1201 | 502 | 诊断生成失败 | 重试或人工分析 |
| IAC-2101 | 404 | 命令不存在 | 安装命令或检查 PATH |
| IAC-2102 | 500 | 命令非零退出（附 exit_code） | 查看 diagnosis |
| IAC-2103 | 504 | 执行超时被终止 | 调整 timeout_ms |
| IAC-2104 | 499 | 用户取消/确认超时 | 重新发起 |
| IAC-2105 | 403 | 高危命令未确认 | 携带 confirm_dangerous 重试（先阅读影响面） |
| IAC-2106 | 501 | 平台不支持该命令形态 | 使用平台等价命令 |
| IAC-3101 | 404 | 会话/记录不存在 | 核对 ID |
| IAC-3102 | 500 | 存储不可用 | 检查磁盘与文件权限 |
| IAC-3103 | 500 | Schema 迁移失败 | 查看日志；恢复备份后再试 |
| IAC-4101 | 404 | 方案不存在 | 核对 recipe ID |
| IAC-5101 | 400 | 配置无效 | 修正 config.toml（iac config path 定位） |
| IAC-5102 | 401 | 缺少 API Key | 按文档设置 Provider 环境变量 |
| IAC-6101 | 503 | daemon 未运行 | `iac daemon start` |
| IAC-6102 | 400 | 协议版本不匹配 | 升级 iac / iacd 至一致版本 |
| IAC-6103 | 409 | daemon 已在运行 | 直接使用现有实例 |
| IAC-1301 | 404 | 翻译未命中且 AI 不可用 | 使用原命令或配置后端后重试 |
| IAC-1401 | 404 | 模型插件不存在 | 用 iac model list 查看可用插件 |
| IAC-1402 | 422 | 插件下载校验失败（sha256/大小不符） | 换源重试；勿绕过校验 |
| IAC-1403 | 503 | 本地引擎启动失败 | 查日志；确认插件完整或回退云后端 |
| IAC-1501 | 400 | 未知斜杠标记命令 | 用 `/help` 查看可用标记 |

## 6. 输入标记规范（G13，仅在 iA 输入框/TUI 输入行解析，不改变 shell 语义）

| 标记 | 含义 | 示例 | 行为 |
|---|---|---|---|
| `??<内容>` | AI 唤起：`??ls` 解释命令（=iac explain）；`??<自然语言>` 生成提案（=iac ask） | `??docker run 的 -v 是什么意思` | 不经 shell；结果同流 1/流 2 |
| `/<名称>` | iA-console 管理命令：`/config`（配置：输入终端类型=习惯方言、本地模型路径、远端模型地址、主题）、`/help`、`/history`、`/recipe`、`/session`、`/model`、`/theme`、`/exit` | `/config` | TUI 打开对应面板；CLI 进入交互向导；未知命令报 IAC-1501 |
| `---`（独行） | 任务分隔：归档当前任务段，其后 AI 调用不携带该段之前上下文（AC-27），历史记录仍完整 | 连续多任务时防上下文串味、控 token | 落一条 separator 会话事件 |
| `#<文字>` | 会话备注（不执行，入会话事件流 kind=note） | `# 这里是在排查磁盘` | 与 shell 注释语义一致，零学习成本 |
| `\`（转义） | 取消标记语义，按字面执行 | `\??foo`、`\---` | 后续字符原样进入命令路径 |

**冲突说明**：`??`（shell 通配）、`/config`（按路径执行）、`---`、`#`（注释）在 shell 中各有语义，但标记只在 iA 输入框解析、从不注入 shell；用户经现有终端执行同样字符串行为不变。CLI 子命令（iac ask/config/...）始终是标记的等价出口。

**配置项**（`iac config` / `/config` 面板可改）：

| 键 | 取值 | 说明 |
|---|---|---|
| ui.habit_dialect | linux/macos/windows | 习惯方言（翻译源） |
| ui.theme | day/night/auto | 主题，auto 跟随终端背景 |
| providers.<name>.type | openai/anthropic/local-plugin/local-path | Provider 四类（ADR-006 修订） |
| providers.<name>.base_url / api_key_env / model | string | 远端两类的地址/密钥环境变量/模型名 |
| providers.<name>.path | string | local-path 型的模型文件路径（GGUF） |
| ai.default_provider / ai.fallback_local | string/bool | 默认后端与云→本地降级开关 |
| modelhub.source | hf/modelscope/hf-mirror | 插件下载源 |

*HTTP 语义列仅作状态码语义参照（CLI `--json` 与 IPC error 复用该语义），实际无 HTTP 服务。

## 5. 版本与兼容策略

- CLI：语义化版本；子命令与 flag 只增不删，废弃先标记 deprecated 警告再跨 2 个 minor 移除。
- IPC/MCP：信封与工具 payload 携带 `v` 字段（当前 1）；不兼容变更升 major 并要求前后端版本一致（IAC-6102 拦截）。
- MCP 工具名冻结：`iac_*` 前缀不重命名，废弃工具在 tools 列表标注 deprecated 保留 2 个 minor。
- 兼容承诺：错误码只增不改义；执行记录/会话 schema 只加列不删列（迁移单向）。
