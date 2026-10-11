# 02 模块间接口与数据模型（_06_interface_design）

> 状态：draft　更新：2026-10-11

## 1. 内部接口清单

所有方法签名符合五段规范：`boolean method(input, output, context, metrics, evolution)`。Rust 映射：input 传不可变引用，output/metrics/evolution 传可变引用，context 传不可变引用（见 method-spec 第四节）。层取值：app / domain / common / infra。

**infra Adapter 方法的 IO 说明**：`data 禁止 IO` 规则适用于 domain 层业务方法；infra Adapter 方法（IF-xxx）是项目唯一的第三方 IO 边界，其内部允许 IO，职责为"调用外部组件 + 结果装配 + 异常转译"。

| 接口ID | 签名 | category | 层 | 模块 | 对应任务 | 备注 |
|---|---|---|---|---|---|---|
| I-001 | `fn match_risk_rules(input: &RiskRulesInput, output: &mut RiskAssessment, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | algorithm | common | risk | T-011 | 纯函数，跨项目可复用 |
| I-002 | `fn parse_help_text(input: &HelpText, output: &mut CommandKnowledge, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | algorithm | common | knowledge | T-013 | 纯函数；false=格式不识别 |
| I-003 | `fn lookup_knowledge(input: &CommandRef, output: &mut Option<CommandKnowledge>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | knowledge | T-014 | 查无结果仍为 true（业务"否"） |
| I-004 | `fn cache_knowledge(input: &CommandKnowledge, output: &mut CacheAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | knowledge | T-014 | source_hash 变更覆盖 |
| I-005 | `fn build_intent_prompt(input: &AskInput, output: &mut PromptBundle, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | intent | T-015 | 组装意图+知识+平台+会话摘要 |
| I-006 | `fn parse_proposal_response(input: &LlmResponse, output: &mut Proposal, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | intent | T-015 | JSON 校验；未注册参数拒绝 |
| I-007 | `fn build_explain_doc(input: &CommandKnowledge, output: &mut ParamSpecDoc, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | intent | T-016 | 知识→参数文档装配 |
| I-008 | `fn record_execution(input: &ExecutionRecord, output: &mut RecordAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | history | T-008 | 含超大输出截断+落盘引用 |
| I-009 | `fn query_history(input: &HistoryFilter, output: &mut HistoryPage, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | history | T-009 | 组合过滤 |
| I-010 | `fn get_execution_detail(input: &ExecutionId, output: &mut Option<ExecutionRecord>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | history | T-008 | |
| I-011 | `fn append_audit_entry(input: &AuditEntry, output: &mut RecordAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | history | T-008 | 追加式，禁改写 |
| I-012 | `fn save_recipe(input: &RecipeInput, output: &mut Recipe, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | recipe | T-019 | |
| I-013 | `fn search_recipes(input: &RecipeFilter, output: &mut Vec<Recipe>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | recipe | T-019 | 关键词+标签+平台 |
| I-014 | `fn get_recipe(input: &RecipeId, output: &mut Option<Recipe>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | recipe | T-019 | |
| I-015 | `fn delete_recipe(input: &RecipeId, output: &mut DeleteAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | recipe | T-019 | |
| I-016 | `fn create_session(input: &NewSessionInput, output: &mut SessionInfo, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | session | T-007 | |
| I-017 | `fn append_session_event(input: &SessionEvent, output: &mut RecordAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | session | T-007 | seq 连续性约束 |
| I-018 | `fn load_session(input: &SessionId, output: &mut Option<SessionInfo>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | session | T-007 | 损坏→corrupted 标记 |
| I-019 | `fn list_sessions(input: &SessionFilter, output: &mut Vec<SessionSummary>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | session | T-007 | |
| I-020 | `fn get_or_create_current(input: &CurrentSessionInput, output: &mut SessionInfo, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | session | T-007 | 按 cwd+host 复用或新建 |
| I-021 | `fn build_failure_context(input: &ExecutionRecord, output: &mut DiagnosePromptBundle, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | doctor | T-018 | 先过静态快速通道 |
| I-022 | `fn parse_diagnosis_response(input: &LlmResponse, output: &mut Diagnosis, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | doctor | T-018 | 无证据引用→speculation 标记 |
| I-023 | `fn assess_risk(input: &RiskInput, output: &mut RiskAssessment, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | domain | risk | T-012 | 装载规则→I-001→闸门决策 |
| I-024 | `fn resume_session(input: &SessionId, output: &mut SessionInfo, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | domain | session | T-010 | 载入+置当前会话 |
| I-025 | `fn handle_ask(input: &AskInput, output: &mut Proposal, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | intent | T-016 | 主流程：I-020→I-003→(I-002→I-004)→I-005→IF-005→I-006→I-023→I-017 |
| I-026 | `fn handle_explain(input: &ExplainInput, output: &mut ParamSpecDoc, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | intent | T-016 | I-003→未命中→取 help→I-002→I-004→I-007 |
| I-027 | `fn handle_execute(input: &ExecuteInput, output: &mut ExecutionOutcome, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | executor | T-017 | I-023→IF-003→I-008/I-011/I-017→失败转 I-029 |
| I-028 | `fn confirm_and_execute(input: &ExecuteInput, output: &mut ExecutionOutcome, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | executor | T-017 | 确认分支：置 proposal 状态→I-027 语义 |
| I-029 | `fn diagnose_failure(input: &DiagnoseInput, output: &mut Diagnosis, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | doctor | T-018 | I-010→I-021→IF-005→I-022 |
| I-030 | `fn dispatch_request(input: &IpcRequest, output: &mut IpcResponse, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | server | T-020 | 唯一路由入口，注入 metrics/evolution |
| I-031 | `fn lookup_translation(input: &TranslationInput, output: &mut Option<TranslationResult>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | knowledge | T-024 | 静态表+缓存双层查找 |
| I-032 | `fn cache_translation(input: &TranslationResult, output: &mut RecordAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | knowledge | T-024 | AI 译文回写，verified=false |
| I-033 | `fn translate_command(input: &TranslationInput, output: &mut TranslationResult, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | intent | T-025 | 静态命中零 AI；一对一未收录→AI；需组合→AI 规划脚本入库+登记映射；降级返回原文 |
| I-034 | `fn parse_input_marker(input: &RawInput, output: &mut InputDecision, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | algorithm | common | server | T-029 | 纯函数：标记分发（??//---/#/\ 转义），普通命令原样通过 |
| I-035 | `fn capture_pre_image(input: &CaptureScope, output: &mut ManifestRef, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | domain | rollback | T-036 | 执行前统一捕获前像（v2，M7） |
| I-036 | `fn diff_effects(input: &ManifestRef, output: &mut Vec<EffectEntry>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | rollback | T-036 | manifest vs 现况 → 效果日志入库（v2） |
| I-037 | `fn rollback_to(input: &RollbackRequest, output: &mut RollbackReport, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | domain | rollback | T-036 | 范围回放（--to/--from）逐行倒序+冲突决策（v2） |
| I-038 | `fn file_history(input: &FilePath, output: &mut Vec<FileVersion>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | rollback | T-038 | 路径版本列表=各次执行效果条目（v2） |
| I-039 | `fn restore_file(input: &FileVersionRef, output: &mut RestoreReport, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | domain | rollback | T-038 | 取版本内容直接替换，过风险闸门（v2） |
| I-040 | `fn list_rollback_points(input: &RollbackListFilter, output: &mut Vec<RollbackPointView>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | rollback | T-039 | history 式回滚点聚合视图：seq/时间/命令/可回滚标识（v2） |
| I-041 | `fn collect_commands(input: &CollectRange, output: &mut Vec<CommandEntry>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | domain | script | T-040 | 任务段/行号区间统一取命令序列（v2） |
| I-042 | `fn assemble_script(input: &ScriptAssemblyInput, output: &mut AssembledScript, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | algorithm | common | script | T-040 | 确定性拼接 + 平台头部（set -e / ErrorActionPreference）（v2） |
| I-043 | `fn refine_script(input: &AssembledScript, output: &mut ScriptRefineDiff, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | script | T-041 | AI 精修：参数化/预检/跨方言适配，diff 确认（v2） |
| I-044 | `fn run_script(input: &ScriptRunInput, output: &mut ExecutionOutcome, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | app | script | T-042 | 命名脚本执行：platform 选版本、闸门（内部最高风险）、记录（v2） |
| IF-001 | `fn storage_query(input: &SqlQuery, output: &mut QueryRows, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | storage | T-003 | SQL 原语，rusqlite 封装 |
| IF-002 | `fn storage_execute(input: &SqlStatement, output: &mut ExecuteAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | storage | T-003 | 写原语，含 ID 生成 |
| IF-003 | `fn spawn_process(input: &ProcessSpec, output: &mut ProcessResult, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | process | T-005 | 平台 shell 选择+超时+捕获 |
| IF-004 | `fn storage_migrate(input: &MigrationDir, output: &mut MigrationAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | infra | storage | T-003/004 | 版本化迁移，幂等 |
| IF-005 | `fn ai_complete(input: &AiRequest, output: &mut LlmResponse, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | ai | T-006 | OpenAI 兼容，Provider 档案 |
| IF-006 | `fn load_config(input: &ConfigPath, output: &mut Config, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | config | T-002 | TOML+env 覆盖 |
| IF-007 | `fn save_config(input: &Config, output: &mut ConfigWriteAck, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | config | T-002 | 密钥不落盘 |
| IF-008 | `fn modelhub_install(input: &PluginRef, output: &mut InstallResult, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | modelhub | T-026 | 多源下载+sha256+断点续传；IO 边界 |
| IF-009 | `fn modelhub_list(input: &PluginFilter, output: &mut Vec<PluginInfo>, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | modelhub | T-026 | 已装插件清单 |
| IF-010 | `fn local_engine_ensure(input: &EngineSpec, output: &mut EngineHandle, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | infra | ai | T-027 | 引擎子进程 supervisor：拉起/健康检查/端口；未装→引导 |
| IF-011 | `fn snapstore_put(input: &SnapshotData, output: &mut ContentRef, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | snapstore | T-035 | 内容寻址写入，去重+压缩（v2） |
| IF-012 | `fn snapstore_fetch(input: &ContentRef, output: &mut SnapshotData, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | data | infra | snapstore | T-035 | 按引用取回（v2） |
| IF-013 | `fn sweep_expired(input: &RetentionPolicy, output: &mut SweepReport, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> bool` | orchestration | infra | snapstore | T-035 | retention LRU 清理快照与回收站（v2） |

## 2. 关键接口契约

### I-025 `handle_ask`（意图主流程）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&AskInput` | intent: 1~2000 字符；cwd/platform 可空默认 |
| output | `&mut Proposal` | 填充 command/argv/explanation/risk_level/platform/verified |
| context | `&Context` | 需 session(当前会话)、config(provider/闸门策略)、deps(StorePort/AiAdapter) |
| metrics | `&mut Metrics` | 记录：knowledge_hit、prompt_tokens、降级发生 |
| evolution | `&mut Evolution` | false 时 reasonCode∈{IAC-1101,IAC-1102,IAC-1103}；降级路径写 suggestions |

- **前置条件**：存储可用（迁移已完成）；intent 非空。
- **后置条件**：成功时 Proposal 已落库（status=proposed）且会话追加 proposal 事件。
- **失败语义**（false 分支）：空/超长意图→IAC-1103（不调 LLM）；AI 不可达→IAC-1101（一次重试后）；解析失败→重试 1 次→IAC-1102；存储写失败→IAC-3102。所有分支写 evolution.cause。

### I-027 `handle_execute`（执行主流程）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&ExecuteInput` | command 或 proposal_id 二选一；confirm_dangerous 默认 false |
| output | `&mut ExecutionOutcome` | 填充 execution_id/exit_code/status/摘要/diagnosis/gate |
| context | `&Context` | 需 session、config(闸门策略/timeout 上限)、deps(StorePort/ProcessAdapter) |
| metrics | `&mut Metrics` | 记录：exit_code、duration_ms、gate_decision |
| evolution | `&mut Evolution` | false 时 reasonCode∈{IAC-2101..2106,IAC-3102}；rejected 写原因 |

- **前置条件**：命令已过 I-023 评估；destructive 时 input.confirm_dangerous=true。
- **后置条件**：无论成败，执行记录+审计+会话事件均已落库（AC-05）；失败时已触发诊断（AC-03）。
- **失败语义**：闸门拒绝→IAC-2105（status=rejected，影响面在 output.gate）；ENOENT→IAC-2101；超时→IAC-2103（kill 进程组）；注意**业务上的"命令非零退出"属于成功执行**（true），status=failed 并附 diagnosis，仅基础设施失败返回 false。

### I-023 `assess_risk`（风险评估）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&RiskInput` | command+argv；cwd |
| output | `&mut RiskAssessment` | level/hits(命中规则)/impact(影响面文案)/requires_confirm |
| context | `&Context` | config(自定义规则目录) |
| metrics | `&mut Metrics` | 记录命中规则数 |
| evolution | `&mut Evolution` | 规则集加载失败→IAC-3102 |

- **前置/后置**：无 IO 副作用（读规则除外）；同一输入恒同一输出（确定性，供闸门终检复验）。
- **失败语义**：仅规则装载失败返回 false；未命中任何规则→safe（true）。

### I-029 `diagnose_failure`

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&DiagnoseInput` | execution_id 可空=最近失败 |
| output | `&mut Diagnosis` | root_cause/evidence/confidence/fix_proposals/rollback_note |
| metrics | `&mut Metrics` | 记录静态通道命中、AI 调用 |
| evolution | `&mut Evolution` | AI 失败→降级静态通道结果或 IAC-1201 |

- **失败语义**：静态快速通道命中即返回（true，不调 AI）；AI 不可达且静态未命中→IAC-1201；证据引用为空的假设标 speculation=true。

### I-033 `translate_command`（跨方言翻译）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&TranslationInput` | command 非空；from/to 均为 Platform 枚举 |
| output | `&mut TranslationResult` | command_to/params(参数级映射)/notes/source/kind/verified/risk_level/recipe_id |
| context | `&Context` | config(habit_dialect)、deps(StorePort/AiAdapter) |
| metrics | `&mut Metrics` | 记录 source(static\|ai\|native)、kind(direct\|composition)、AI 调用次数 |
| evolution | `&mut Evolution` | AI 不可用且静态未命中→降级返回原文（true，source=native 并附说明） |

- **前置条件**：无（幂等、只读翻译，不执行命令）。
- **后置条件**：AI 一对一翻译成功→回写缓存（verified=false）；AI 组合规划成功→脚本存方案库（kind=script，tags=translation/auto）+ 登记映射（kind=composition，recipe_id 回填）；不产生执行与审计之外的副作用。
- **失败语义**：仅存储不可用（IAC-3102）返回 false；翻译"无等价命令"属于成功（true）并以 notes 说明。
- **参数级映射四类**（AC-23）：direct（直映）/ renamed（改名）/ value_transform（值变换）/ none（无等价，附替代建议）。

### I-034 `parse_input_marker`（输入标记分发）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&RawInput` | 用户输入行原文 |
| output | `&mut InputDecision` | 枚举：Ai{explain\|ask, content} / Slash{name, args} / TaskSeparator / Note{text} / Literal(escape 后命令) / Command(原样) |
| metrics | `&mut Metrics` | 记录标记类型计数 |
| evolution | `&mut Evolution` | 未知斜杠命令→true 但 decision=UnknownSlash（由调用方转 IAC-1501） |

- **失败语义**：纯字符串分发，恒为 true；解析歧义（如 `---` 非独行）按普通命令处理。

### IF-003 `spawn_process`（infra Adapter）

| 段 | 类型 | 约束 |
|---|---|---|
| input | `&ProcessSpec` | command/argv、cwd、env 增量、timeout_ms、平台 shell 选择 |
| output | `&mut ProcessResult` | exit_code/stdout/stderr（截断前原始字节上限 1MB）、duration_ms、shell_used |
| metrics | `&mut Metrics` | 记录耗时、输出字节数 |
| evolution | `&mut Evolution` | spawn 系统错误→IAC-2101/2106；超时→IAC-2103 |

- **后置条件**：进程已终止（kill 进程组），无僵尸残留；输出已完整捕获。
- **失败语义**：本方法仅代表"执行机制失败"（找不到命令/超时/平台不支持）；命令自身非零退出是正常产出（true）。

## 3. 数据模型 Schema（SQLite DDL v1）

```sql
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  cwd TEXT NOT NULL,
  shell TEXT NOT NULL,
  host TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',     -- active|closed|corrupted
  created_at INTEGER NOT NULL,               -- unix ms
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC);

CREATE TABLE session_events (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id),
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,                        -- command|proposal|ai_message|note|diagnosis
  payload TEXT NOT NULL,                     -- JSON
  created_at INTEGER NOT NULL,
  UNIQUE(session_id, seq)
);

CREATE TABLE proposals (
  id TEXT PRIMARY KEY,
  session_id TEXT REFERENCES sessions(id),
  intent_text TEXT NOT NULL,
  command TEXT NOT NULL,
  argv TEXT NOT NULL,                        -- JSON array
  explanation TEXT NOT NULL,                 -- JSON: ParamSpecDoc + impact
  risk_level TEXT NOT NULL,                  -- safe|caution|dangerous|destructive
  platform TEXT NOT NULL,                    -- linux|macos|windows
  verified INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'proposed',   -- proposed|confirmed|executed|rejected
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_proposals_session ON proposals(session_id, created_at);

CREATE TABLE executions (
  id TEXT PRIMARY KEY,
  session_id TEXT REFERENCES sessions(id),
  proposal_id TEXT REFERENCES proposals(id),
  command TEXT NOT NULL,
  argv TEXT NOT NULL,                        -- JSON array
  cwd TEXT NOT NULL,
  shell TEXT NOT NULL,
  host TEXT NOT NULL,
  actor TEXT NOT NULL,                       -- human|agent:<client>
  exit_code INTEGER,                         -- NULL=未退出（超时/取消）
  status TEXT NOT NULL,                      -- ok|failed|timeout|cancelled|rejected
  risk_level TEXT NOT NULL,
  stdout_summary TEXT,                       -- ≤64KB
  stderr_summary TEXT,
  stdout_ref TEXT, stderr_ref TEXT,          -- 超限原文落盘引用
  started_at INTEGER NOT NULL,
  duration_ms INTEGER,
  journal_ref TEXT                          -- v2 回滚：执行前像 manifest 在快照仓库的引用（效果日志入口）
);
CREATE INDEX idx_exec_time ON executions(started_at DESC);
CREATE INDEX idx_exec_cwd ON executions(cwd);
CREATE INDEX idx_exec_cmd ON executions(command);

CREATE TABLE recipes (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT,
  kind TEXT NOT NULL DEFAULT 'command',      -- command|script（组合翻译/脚本工坊生成的脚本）
  name TEXT,                                 -- 脚本工坊命名（kind=script 时全局唯一，iac run @name 的名字）
  command_template TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '[]',           -- JSON array
  platform TEXT NOT NULL,                    -- linux|macos|windows|all
  risk_level TEXT NOT NULL,
  usage_count INTEGER NOT NULL DEFAULT 0,
  source_execution_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_recipes_title ON recipes(title);
CREATE UNIQUE INDEX idx_recipes_name ON recipes(name) WHERE name IS NOT NULL;

CREATE TABLE knowledge_cache (
  id TEXT PRIMARY KEY,                       -- hash(command+platform)
  command TEXT NOT NULL,
  platform TEXT NOT NULL,
  schema_json TEXT NOT NULL,                 -- CommandKnowledge
  source_hash TEXT NOT NULL,                 -- help 文本指纹
  verified INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  UNIQUE(command, platform)
);

CREATE TABLE audit_log (
  id TEXT PRIMARY KEY,
  ts INTEGER NOT NULL,
  session_id TEXT,
  actor TEXT NOT NULL,                       -- human|agent:<client>|system
  action TEXT NOT NULL,                      -- execute|confirm|reject|save_recipe|session_resume|translate|...
  target TEXT,
  detail TEXT                                -- JSON
);
CREATE INDEX idx_audit_ts ON audit_log(ts DESC);

CREATE TABLE translations (
  id TEXT PRIMARY KEY,                       -- hash(from_dialect+to_platform+command_from)
  from_dialect TEXT NOT NULL,                -- linux|macos|windows（习惯方言）
  to_platform TEXT NOT NULL,                 -- linux|macos|windows（主机平台）
  command_from TEXT NOT NULL,
  command_to TEXT NOT NULL,
  params_json TEXT,                          -- 参数级映射：[{from,to,kind(direct|renamed|value_transform|none),note}]
  notes TEXT,                                -- 差异说明
  kind TEXT NOT NULL DEFAULT 'direct',       -- direct（一对一）|composition（组合脚本）
  recipe_id TEXT REFERENCES recipes(id),     -- composition 时指向生成的脚本方案
  verified INTEGER NOT NULL DEFAULT 0,       -- 用户确认后转正
  source TEXT NOT NULL,                      -- builtin|ai|user
  updated_at INTEGER NOT NULL,
  UNIQUE(from_dialect, to_platform, command_from)
);

CREATE TABLE model_plugins (
  id TEXT PRIMARY KEY,                       -- plugin_id
  name TEXT NOT NULL,
  engine TEXT NOT NULL,                      -- llama-server 及版本
  model TEXT NOT NULL,                       -- GGUF 文件标识
  files TEXT NOT NULL,                       -- JSON: [{path,sha256,size}]
  total_size INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'available',  -- available|active|broken
  source TEXT NOT NULL,                      -- hf|modelscope|hf-mirror
  installed_at INTEGER NOT NULL
);

CREATE TABLE rollback_journal (               -- v2（M7）启用，v1 建表不写入
  seq INTEGER PRIMARY KEY,                   -- 全局单调，回滚点定位
  execution_id TEXT NOT NULL REFERENCES executions(id),
  path TEXT NOT NULL,
  op TEXT NOT NULL,                          -- created|modified|deleted|renamed
  before_ref TEXT,                           -- 快照仓库内容引用（前像）
  after_ref TEXT,                            -- 快照仓库内容引用（后像）
  extra_path TEXT,                           -- renamed 的原路径
  scope TEXT NOT NULL DEFAULT 'in',          -- in|out_of_scope
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_rj_exec ON rollback_journal(execution_id, seq);
CREATE INDEX idx_rj_path ON rollback_journal(path, seq);
```

### 核心 payload 类型（Rust）

```rust
pub struct AskInput { pub intent: String, pub cwd: Option<String>, pub platform: Option<Platform> }
pub struct Proposal { pub id: String, pub session_id: String, pub intent_text: String, pub command: String,
    pub argv: Vec<String>, pub explanation: ParamSpecDoc, pub risk_level: RiskLevel, pub platform: Platform,
    pub verified: bool, pub status: ProposalStatus, pub created_at: i64 }
pub struct ParamSpecDoc { pub params: Vec<ParamSpec>, pub impact: String, pub summary: String }
pub struct ParamSpec { pub flag: String, pub meaning: String, pub required: bool, pub default: Option<String> }
pub enum RiskLevel { Safe, Caution, Dangerous, Destructive }
pub struct RiskAssessment { pub level: RiskLevel, pub hits: Vec<String>, pub impact: String, pub requires_confirm: bool }
pub struct ExecuteInput { pub command: Option<String>, pub proposal_id: Option<String>, pub session_id: Option<String>,
    pub confirm_dangerous: bool, pub timeout_ms: Option<u64> }
pub struct GateInfo { pub level: RiskLevel, pub requires_confirm: bool, pub weak_warning: bool,
    pub conversion: Option<ConversionView> }   // 人类直输恒 weak_warning=true、requires_confirm=false；翻译高危/Agent 反之
pub struct ConversionView { pub from_command: String, pub to_command: String }   // 前端统一以灰阶弱化呈现原命令（AC-36）
pub struct ExecutionOutcome { pub execution_id: String, pub exit_code: Option<i32>, pub status: ExecStatus,
    pub stdout_summary: Option<String>, pub stderr_summary: Option<String>, pub duration_ms: Option<u64>,
    pub diagnosis: Option<Diagnosis>, pub gate: GateInfo }
pub struct Diagnosis { pub root_cause: String, pub evidence: Vec<String>, pub confidence: f32,
    pub fix_proposals: Vec<FixProposal>, pub rollback_note: String, pub speculation: bool }
pub struct FixProposal { pub command: String, pub explanation: String }
pub struct ExecutionRecord { /* 与 executions 表字段一一对应，见 DDL */ }
pub struct AuditEntry { pub ts: i64, pub session_id: Option<String>, pub actor: String,
    pub action: String, pub target: Option<String>, pub detail: Option<serde_json::Value> }
pub struct SessionInfo { pub id: String, pub title: String, pub cwd: String, pub shell: String,
    pub host: String, pub status: String, pub created_at: i64, pub updated_at: i64,
    pub events: Vec<SessionEvent> }
pub struct SessionEvent { pub id: String, pub session_id: String, pub seq: i64, pub kind: EventKind,
    pub payload: serde_json::Value, pub created_at: i64 }
pub struct Recipe { pub id: String, pub title: String, pub description: Option<String>,
    pub command_template: String, pub tags: Vec<String>, pub platform: Platform, pub risk_level: RiskLevel,
    pub usage_count: i64, pub source_execution_id: Option<String>, pub created_at: i64, pub updated_at: i64 }
pub struct CommandKnowledge { pub command: String, pub platform: Platform, pub subcommands: Vec<String>,
    pub params: Vec<ParamSpec>, pub source_hash: String, pub verified: bool }
pub struct HistoryFilter { pub q: Option<String>, pub cwd: Option<String>, pub since: Option<i64>,
    pub until: Option<i64>, pub status: Option<ExecStatus>, pub actor: Option<String>,
    pub limit: u32, pub offset: u32 }
pub struct IpcRequest { pub v: u32, pub id: String, pub method: String, pub params: serde_json::Value }
pub struct IpcResponse { pub v: u32, pub id: String, pub ok: bool,
    pub result: Option<serde_json::Value>, pub error: Option<IacError> }
pub struct IacError { pub code: String, pub message: String, pub detail: Option<serde_json::Value> }
pub struct TranslationInput { pub command: String, pub from_dialect: Platform, pub to_platform: Platform }
pub struct TranslationResult { pub command_from: String, pub command_to: String,
    pub params: Vec<ParamMapping>, pub notes: String, pub source: TranslationSource,
    pub kind: TranslationKind, pub recipe_id: Option<String>, pub verified: bool, pub risk_level: RiskLevel }
pub struct ParamMapping { pub from_param: String, pub to_param: Option<String>,
    pub kind: ParamMapKind, pub note: Option<String> }
pub enum ParamMapKind { Direct, Renamed, ValueTransform, None }
pub enum TranslationSource { Native, Static, Ai }
pub enum TranslationKind { Direct, Composition }
pub enum RawInput { Line(String) }
pub enum InputDecision { AiExplain { command: String }, AiAsk { intent: String },
    Slash { name: String, args: String }, TaskSeparator, Note { text: String },
    Literal { command: String }, Command { command: String }, UnknownSlash { name: String } }
pub struct PluginRef { pub plugin_id: String, pub source: DownloadSource }
pub struct InstallResult { pub plugin_id: String, pub files: Vec<InstalledFile>, pub total_size: u64, pub duration_ms: u64 }
pub struct PluginInfo { pub plugin_id: String, pub name: String, pub engine: String, pub model: String,
    pub total_size: u64, pub status: PluginStatus, pub installed_at: i64 }
pub struct EngineSpec { pub plugin_id: String, pub context_len: u32, pub preferred_port: Option<u16> }
pub struct EngineHandle { pub base_url: String, pub pid: u32, pub model: String }
pub struct CaptureScope { pub execution_hint: String, pub cwd: String, pub explicit_paths: Vec<String>, pub impact_paths: Vec<String> }
pub struct ManifestRef { pub execution_id: String, pub manifest_ref: String, pub journal_ref: String }
pub enum EffectOp { Created, Modified, Deleted, Renamed }
pub struct EffectEntry { pub seq: i64, pub execution_id: String, pub path: String, pub op: EffectOp,
    pub before_ref: Option<String>, pub after_ref: Option<String>, pub extra_path: Option<String>,
    pub scope: CaptureScopeFlag, pub created_at: i64 }
pub enum CaptureScopeFlag { In, OutOfScope }
pub enum RollbackPoint { ExecutionId(String), Seq(i64), Timestamp(i64), Last }
pub struct RollbackRange { pub from: Option<i64>, pub to: Option<i64> }   // 回放 (to, from] 区间；from 缺省=最新，to 缺省=最远
pub enum ConflictPolicy { Ask, Skip, Abort }                              // --on-conflict；非交互缺省 Abort
pub struct RollbackRequest { pub range: RollbackRange, pub on_conflict: ConflictPolicy }
pub struct RollbackListFilter { pub q: Option<String>, pub since: Option<i64>, pub limit: u32 }
pub enum Rollbackable { Full, Partial { reason: String }, None { reason: String } }
pub struct RollbackPointView { pub seq: i64, pub at: i64, pub command: String,
    pub effect_count: u32, pub rollbackable: Rollbackable }
pub struct RollbackReport { pub applied: Vec<EffectEntry>, pub skipped: Vec<SkippedEntry>, pub trash_dir: String }
pub struct SkippedEntry { pub entry: EffectEntry, pub reason: String }    // ask 拒绝/skip 策略/引用缺失/out_of_scope
pub struct FileVersion { pub seq: i64, pub execution_id: String, pub op: EffectOp,
    pub content_ref: String, pub at: i64, pub command: String }
pub struct FileVersionRef { pub path: String, pub seq: i64 }
pub struct RestoreReport { pub path: String, pub restored_from: i64, pub gate: GateInfo }
pub struct CollectRange { pub segment_id: Option<String>, pub from_seq: Option<i64>, pub to_seq: Option<i64> }  // 任务段或行号区间（与回滚同一套"行"语义）
pub struct CommandEntry { pub seq: i64, pub command: String, pub cwd: String }
pub enum ScriptTarget { Sh, Bat, Both }
pub struct ScriptAssemblyInput { pub entries: Vec<CommandEntry>, pub target: ScriptTarget }
pub struct AssembledScript { pub name_hint: String, pub sh_body: Option<String>, pub ps_body: Option<String>,
    pub target: ScriptTarget, pub max_risk: RiskLevel }
pub struct ScriptRefineDiff { pub refined: AssembledScript, pub notes: Vec<String> }
pub struct ScriptRunInput { pub name: String, pub args: Vec<String>, pub confirm_dangerous: bool }
```

### 支撑类型（五段签名公共段，P7 随 T-002 定稿实现）

```rust
pub struct Context<'a> {
    pub session: &'a SessionCtx,        // session_id、request_id、actor(human|agent:<client>)
    pub config: &'a Config,             // 已解析运行配置
    pub deps: &'a Deps,                 // StorePort / AiAdapter / ProcessAdapter / ConfigPort 句柄
    pub locale: &'a Locale,             // 语言、时区
}
pub struct Metrics { /* 框架注入：enter/exit 自动计时；log(level,msg,fields)；counter/gauge/histogram；mark */ }
pub struct Evolution {
    pub ok: bool,
    pub reason_code: Option<String>,    // IAC-xxxx
    pub reason: Option<String>,
    pub retryable: bool,
    pub suggestions: Vec<String>,
    pub cause: Vec<CauseLink>,          // 子方法失败链，逐层合并
}
pub struct CauseLink { pub step: String, pub reason_code: Option<String>, pub reason: String }
```

## 4. 接口演进规则

- 接口变更必须同步更新本文件与 `_07_method_idx.json`（用 `scripts/sop/add-method.py`）。
- 不兼容变更必须新增接口并废弃旧接口（_07 标记 deprecated + replaced_by），错误码只增不改义。
- payload 结构（Propasal/Diagnosis 等）字段只增不删；删除字段需升协议 major。
