# 01 功能矩阵与任务卡（_05_function_design）

> 状态：draft　更新：2026-10-11

## 1. 功能矩阵（模块 × 层）

接口编号对应 _06 internal 接口清单（I-xxx=五段方法 / IF-xxx=infra Adapter 方法 / 豁免=框架签名入口，不入 _07）。

| 模块 \ 层 | app（编排） | domain（数据+域内编排） | common（算法） | infra（Adapter） |
|---|---|---|---|---|
| server | dispatch_request(I-030)；daemon 监听循环/MCP handler（豁免） | — | — | — |
| intent | handle_ask(I-025)、handle_explain(I-026)、translate_command(I-033) | build_intent_prompt(I-005)、parse_proposal_response(I-006)、build_explain_doc(I-007) | — | — |
| executor | handle_execute(I-027)、confirm_and_execute(I-028) | — | — | spawn_process(IF-003) |
| doctor | diagnose_failure(I-029) | build_failure_context(I-021)、parse_diagnosis_response(I-022) | — | — |
| risk | — | assess_risk(I-023，编排) | match_risk_rules(I-001) | — |
| session | — | create_session(I-016)、append_session_event(I-017)、load_session(I-018)、list_sessions(I-019)、get_or_create_current(I-020)、resume_session(I-024，编排) | — | — |
| history | — | record_execution(I-008)、query_history(I-009)、get_execution_detail(I-010)、append_audit_entry(I-011) | — | — |
| recipe | — | save_recipe(I-012)、search_recipes(I-013)、get_recipe(I-014)、delete_recipe(I-015) | — | — |
| knowledge | — | lookup_knowledge(I-003)、cache_knowledge(I-004)、lookup_translation(I-031)、cache_translation(I-032) | parse_help_text(I-002) | — |
| ai | — | — | — | ai_complete(IF-005)、local_engine_ensure(IF-010) |
| modelhub | — | — | — | modelhub_install(IF-008)、modelhub_list(IF-009) |
| 输入解析 | — | — | parse_input_marker(I-034，归属 server) | — |
| rollback | — | diff_effects(I-036)、file_history(I-038)、list_rollback_points(I-040) | — | — |
| rollback（编排） | — | capture_pre_image(I-035)、rollback_to(I-037)、restore_file(I-039)（编排） | — | — |
| snapstore | — | — | — | snapstore_put(IF-011)、snapstore_fetch(IF-012)、sweep_expired(IF-013) |
| storage | — | — | — | storage_query(IF-001)、storage_execute(IF-002)、storage_migrate(IF-004) |
| config | — | — | — | load_config(IF-006)、save_config(IF-007) |
| tui / cli | 事件循环、子命令入口（豁免） | — | — | — |

（编号与 _06 internal 接口清单一一对应；本矩阵展示归属与覆盖，防漏防重。）

## 2. 任务卡

粒度约定：每卡 = 一组紧密相关方法 + 测试，验收方式全部为可执行测试或命令；卡内编码时按 TDD 自由拆小步（RED→GREEN→REFACTOR）。编号即 P7 执行顺序，Git 提交 scope 引用卡号。

### M1 内核与基础设施

### T-001 Cargo workspace 脚手架

| 字段 | 内容 |
|---|---|
| 涉及文件 | `Cargo.toml`(workspace)、`iac/`、`iacd/`、`core/`、`infra/` 各 `Cargo.toml` 与占位 `src/`、`.gitignore` 校验 |
| 产出方法 | 无（结构搭建） |
| 验收方式 | `cargo check --workspace` 通过；`iac --help`、`iacd --help` 可打印占位 |
| 依赖任务 | 无 |
| 预估 | 10 分钟 |

### T-002 配置 Adapter（US-10/11）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/config/mod.rs`、`infra/tests/config_test.rs` |
| 产出方法 | `load_config`、`save_config`（data，layer=infra） |
| 验收方式 | 单测：TOML 读写往返、默认值填充、`IAC_*` 环境变量覆盖优先级 |
| 依赖任务 | T-001 |
| 预估 | 25 分钟 |

### T-003 存储 Adapter（US-9 支撑）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/storage/mod.rs`、`infra/tests/storage_test.rs` |
| 产出方法 | `storage_query`、`storage_execute`、`storage_migrate`（data/migrate=orchestration，layer=infra） |
| 验收方式 | 内存库单测：建表→写→查往返、参数绑定防注入、迁移版本幂等 |
| 依赖任务 | T-001 |
| 预估 | 25 分钟 |

### T-004 数据库 Schema v1（AC-05/07）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/storage/migrations/v1.sql`（7 张表 DDL，见 _06 internal 3 节） |
| 产出方法 | 无（SQL 资产） |
| 验收方式 | 迁移测试：空库迁移成功、重复迁移幂等、表结构断言 |
| 依赖任务 | T-003 |
| 预估 | 15 分钟 |

### T-005 进程 Adapter（US-12 支撑）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/process/mod.rs`、`infra/tests/process_test.rs` |
| 产出方法 | `spawn_process`（data，layer=infra） |
| 验收方式 | 单测：echo 输出捕获、非零退出码、超时 kill、平台 shell 选择（unix: sh；windows: pwsh→powershell→cmd 回退） |
| 依赖任务 | T-001 |
| 预估 | 25 分钟 |

### T-006 AI Adapter（US-11）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/ai/mod.rs`（Provider 档案→OpenAI 兼容请求）、`infra/tests/ai_test.rs` |
| 产出方法 | `ai_complete`（data，layer=infra） |
| 验收方式 | 本地 mock HTTP server 单测：正常响应、超时、非 200、JSON 非法；真连测试标记 `#[ignore]` 由环境变量开启 |
| 依赖任务 | T-001 |
| 预估 | 25 分钟 |

### M2 记录域

### T-007 会话数据方法（US-6）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/session/store.rs`、`core/tests/session_test.rs` |
| 产出方法 | `create_session`、`append_session_event`、`load_session`、`list_sessions`（data） |
| 验收方式 | 单测（内存库）：建立→追加事件→载入一致性；seq 连续性；损坏 payload 标记 corrupted 不影响列表 |
| 依赖任务 | T-004 |
| 预估 | 25 分钟 |

### T-008 执行记录与审计（US-9，AC-05）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/history/store.rs`、`core/tests/history_test.rs` |
| 产出方法 | `record_execution`、`get_execution_detail`、`append_audit_entry`（data） |
| 验收方式 | 单测：全字段往返、超大输出截断摘要+落盘引用、密钥脱敏断言 |
| 依赖任务 | T-004 |
| 预估 | 25 分钟 |

### T-009 历史检索（AC-13）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/history/query.rs`、`core/tests/history_query_test.rs` |
| 产出方法 | `query_history`（data） |
| 验收方式 | 单测：万条种子数据按 关键词/目录/时间窗/状态 组合过滤，断言 <1s（debug 构建放宽为正确性优先） |
| 依赖任务 | T-008 |
| 预估 | 15 分钟 |

### T-010 会话恢复（US-6，AC-06）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/session/resume.rs` |
| 产出方法 | `resume_session`（orchestration） |
| 验收方式 | 单测：恢复后事件流完整、当前会话指针切换、不存在会话报 IAC-3101 |
| 依赖任务 | T-007 |
| 预估 | 10 分钟 |

### M3 智能域

### T-011 风险规则匹配算法（US-8，AC-04）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/common/risk_matcher.rs`、`core/tests/risk_matcher_test.rs` |
| 产出方法 | `match_risk_rules`（algorithm，纯函数） |
| 验收方式 | 单测样本表：`rm -rf /`、`mkfs`、`dd of=/dev/sd*`、`git push --force`（dangerous）、`ls`（safe）等 ≥20 例分级断言 |
| 依赖任务 | T-001 |
| 预估 | 20 分钟 |

### T-012 风险评估编排（US-8）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/risk/assess.rs`、`core/src/risk/rules.rs`（内置规则集） |
| 产出方法 | `assess_risk`（orchestration：装载规则→调用匹配→闸门决策） |
| 验收方式 | 单测：内置规则集加载、destructive 必须要求确认、自定义规则目录覆盖 |
| 依赖任务 | T-011 |
| 预估 | 15 分钟 |

### T-013 help 文本解析算法（US-2，AC-02）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/common/help_parser.rs`、`core/tests/help_parser_test.rs` |
| 产出方法 | `parse_help_text`（algorithm，纯函数） |
| 验收方式 | 单测样本：GNU 风格（ls --help）、Cobra 风格、usage 行解析；解析失败返回 false + evolution 原因 |
| 依赖任务 | T-001 |
| 预估 | 30 分钟 |

### T-014 知识库存取（US-1/2 支撑）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/knowledge/store.rs` |
| 产出方法 | `lookup_knowledge`、`cache_knowledge`（data） |
| 验收方式 | 单测：命中/未命中、source_hash 变更失效、unverified 标记 |
| 依赖任务 | T-004、T-013 |
| 预估 | 15 分钟 |

### T-015 提案生成数据方法（US-1/12，AC-01/14）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/intent/prompt.rs`、`core/src/intent/parse.rs` |
| 产出方法 | `build_intent_prompt`、`parse_proposal_response`（data） |
| 验收方式 | 单测：prompt 含平台/知识/会话上下文；合法/非法 JSON、未注册参数拒绝、重试信号 |
| 依赖任务 | T-014 |
| 预估 | 30 分钟 |

### T-016 意图编排（US-1/2，AC-01/02/11）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/intent/handler.rs`、`core/tests/intent_test.rs` |
| 产出方法 | `handle_ask`、`handle_explain`、`build_explain_doc`（orchestration/data） |
| 验收方式 | mock AiAdapter 单测：正常提案、AI 不可用降级路径（IAC-1101）、空意图（IAC-1103）、解析失败重试一次（IAC-1102）、explain 缓存回写 |
| 依赖任务 | T-012、T-015 |
| 预估 | 30 分钟 |

### T-017 执行编排（US-3/8，AC-04/05/15）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/executor/handler.rs`、`core/tests/executor_test.rs` |
| 产出方法 | `handle_execute`、`confirm_and_execute`（orchestration） |
| 验收方式 | 单测（mock process/ai）：safe 直执行、destructive 无确认拒绝（IAC-2105）、执行落库+审计+会话事件、失败触发诊断、密钥脱敏 |
| 依赖任务 | T-012、T-008、T-005 |
| 预估 | 30 分钟 |

### T-018 错误诊断（US-3，AC-03/15）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/doctor/context.rs`、`core/src/doctor/handler.rs`、`core/tests/doctor_test.rs` |
| 产出方法 | `build_failure_context`、`parse_diagnosis_response`、`diagnose_failure`（data/orchestration） |
| 验收方式 | 单测（mock ai）：静态快速通道命中（127 码）、证据引用缺失标记 speculation、AI 不可用降级、修复提案走闸门 |
| 依赖任务 | T-016 |
| 预估 | 30 分钟 |

### T-019 方案库（US-5，AC-10）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/recipe/store.rs`、`core/tests/recipe_test.rs` |
| 产出方法 | `save_recipe`、`search_recipes`、`get_recipe`、`delete_recipe`（data） |
| 验收方式 | 单测：保存→标签/关键词检索、平台过滤、删除后检索消失、从执行记录转存 |
| 依赖任务 | T-004 |
| 预估 | 20 分钟 |

### M4 服务与接入

### T-020 请求路由 dispatch（US-7/10 支撑）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/server/dispatch.rs`、`core/src/server/envelope.rs`、`core/tests/dispatch_test.rs` |
| 产出方法 | `dispatch_request`（orchestration）+ 信封类型定义 |
| 验收方式 | 单测：各 method 路由正确、未知 method 报错、协议版本不匹配（IAC-6102）、错误码→信封 error 映射 |
| 依赖任务 | T-010、T-016、T-017、T-018、T-019 |
| 预估 | 25 分钟 |

### T-021 daemon 二进制（US-10，AC-17）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iacd/src/main.rs`、`iacd/src/ipc.rs`（入口豁免） |
| 产出方法 | 豁免入口：UDS/NamedPipe 监听、pid 文件、优雅退出 |
| 验收方式 | 集成测试：daemon start→IPC ping→stop；二次启动报 IAC-6103 |
| 依赖任务 | T-020 |
| 预估 | 25 分钟 |

### T-022 CLI 二进制（US-1~10）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iac/src/main.rs`、`iac/src/cli/`（入口豁免） |
| 产出方法 | 豁免入口：clap 子命令（ask/run/doctor/history/recipe/session/daemon/config/chat）→ IPC 适配；`--json` 输出 |
| 验收方式 | assert_cmd 集成测试：history/recipe/session 子命令对 mock daemon 的请求/响应、daemon 未启动报 IAC-6101 |
| 依赖任务 | T-021 |
| 预估 | 25 分钟 |

### T-023 MCP Server（US-7，AC-08）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iacd/src/mcp.rs`（handler 豁免） |
| 产出方法 | 豁免入口：rmcp tools（iac_ask/iac_execute/iac_history_query/iac_explain/iac_diagnose/iac_recipes_search/iac_recipe_save/iac_sessions_current）适配 dispatch |
| 验收方式 | 集成测试：MCP 客户端列出工具、调用 iac_execute 走闸门与审计断言 |
| 依赖任务 | T-021 |
| 预估 | 25 分钟 |

### M5 方言翻译与内置模型

### T-024 翻译映射存取（US-13，AC-19）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/knowledge/translation.rs` |
| 产出方法 | `lookup_translation`（I-031）、`cache_translation`（I-032）（data） |
| 验收方式 | 单测：命中/未命中、verified=false→true 转正、方言+平台组合键 |
| 依赖任务 | T-004 |
| 预估 | 15 分钟 |

### T-025 翻译编排与静态映射数据集（US-13/15，AC-19/20/23/24）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/intent/translate.rs`、`core/src/intent/translation_table/`（内置数据集：官方常用命令全集核心条目，含参数级四类映射，目标 200~400 条随版本维护） |
| 产出方法 | `translate_command`（I-033，orchestration） |
| 验收方式 | 单测（mock ai）：习惯=主机→不翻译；`ls`/`grep`/`rm -r` 等静态命中零 AI 且返回参数级映射表（直映/改名/值变换/无等价四类）；一对一可译未收录→AI 翻译回写；需组合（如管道类）→AI 规划生成脚本→存方案库(tags=translation/auto)→登记映射 kind=composition→再次输入静态命中；AI 不可用→返回原文+说明不阻塞 |
| 依赖任务 | T-024、T-016、T-019 |
| 预估 | 40 分钟 |

### T-026 modelhub 插件下载安装（US-14，AC-21）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/modelhub/mod.rs`、`infra/tests/modelhub_test.rs` |
| 产出方法 | `modelhub_install`（IF-008）、`modelhub_list`（IF-009）（data，layer=infra） |
| 验收方式 | 单测（mock HTTP）：HF/魔搭源下载、sha256 校验失败拒绝、断点续传、安装注册 model_plugins 表 |
| 依赖任务 | T-002 |
| 预估 | 30 分钟 |

### T-027 本地引擎 Provider 接线（US-14，AC-22）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/ai/local.rs` |
| 产出方法 | `local_engine_ensure`（IF-010，orchestration，layer=infra）：拉起插件引擎子进程+健康检查+端口管理；Provider 档案 local-builtin；云→本地降级开关 |
| 验收方式 | 单测：引擎未安装→引导提示；健康检查通过；降级路径断言 |
| 依赖任务 | T-026、T-005 |
| 预估 | 25 分钟 |

### M6 体验与收敛

### T-028 TUI 骨架与提案流（US-4，AC-09）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iac/src/tui/app.rs`、`iac/src/tui/proposal_card.rs`（豁免组件） |
| 产出方法 | 豁免组件：事件循环、意图输入、提案卡渲染、确认/拒绝键位；数据组装下沉 core |
| 验收方式 | 手动冒烟脚本 + 组件单测（数据组装部分）：提案卡含命令/解释/风险/影响面四要素 |
| 依赖任务 | T-022 |
| 预估 | 30 分钟 |

### T-029 输入标记解析器（US-16，AC-25/26/27）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/common/marker_parser.rs`、`core/tests/marker_parser_test.rs` |
| 产出方法 | `parse_input_marker`（I-034，algorithm）：`??`→AI 解释/提问、`/`→管理命令、`---`→任务分隔、`#`→备注、`\`→转义字面 |
| 验收方式 | 单测样本表：各标记正例、转义反例、未知斜杠命令（IAC-1501）、普通命令不受影响 |
| 依赖任务 | T-001 |
| 预估 | 20 分钟 |

### T-030 主题系统与设计令牌（US-17，AC-28）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iac/src/tui/theme.rs`、`themes/day.toml`、`themes/night.toml`（豁免组件+数据资产） |
| 产出方法 | 豁免组件：主题令牌加载、day/night/auto（OSC 11 探测终端背景）、256/16 色降级、Nerd Font 渐进增强 |
| 验收方式 | 组件单测：令牌解析、auto 探测 mock、降级映射表；iA-Writer 式排版令牌（间距/灰阶）断言 |
| 依赖任务 | T-028 |
| 预估 | 25 分钟 |

### T-031 TUI 面板与方言设置（US-4/6/13/16）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iac/src/tui/panels.rs`（豁免组件） |
| 产出方法 | 豁免组件：AI 对话面板（`??` 显式唤起）、历史浏览、习惯方言设置、`/config` 配置面板（终端类型/本地与远端模型地址/主题）、`---` 任务分隔展示 |
| 验收方式 | 组件数据单测 + 手动冒烟脚本 |
| 依赖任务 | T-028、T-025、T-029、T-030 |
| 预估 | 30 分钟 |

### T-032 CLI translate/config 适配（US-13/16）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `iac/src/cli/translate.rs`、`iac/src/cli/config_wizard.rs`（入口豁免） |
| 产出方法 | 豁免入口：`iac translate`（CMD-013）适配 dispatch；`iac config` 交互式向导（对应 TUI /config） |
| 验收方式 | assert_cmd：翻译请求/响应、--json 输出、向导写入配置往返 |
| 依赖任务 | T-025、T-029 |
| 预估 | 15 分钟 |

### T-033 端到端验证与文档（P8 输入）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `tests/e2e/main.rs`（或集成测试目录）、`README.md` quickstart 更新 |
| 产出方法 | 无 |
| 验收方式 | E2E：daemon 启动→ask(mock AI)→confirm→execute→history→session resume→方言翻译（含组合闭环）→标记解析→MCP 工具调用全链路；输出验收清单 |
| 依赖任务 | T-023、T-027、T-031 |
| 预估 | 30 分钟 |

## 3. 里程碑

| 里程碑 | 包含任务 | 完成标志 |
|---|---|---|
| M1 内核与基础设施 | T-001~T-006 | 四个 Adapter 可用，`cargo test --workspace` 全绿 |
| M2 记录域 | T-007~T-010 | 执行/会话/审计/检索落库可查 |
| M3 智能域 | T-011~T-019 | ask→gate→execute→doctor 库级闭环（mock AI） |
| M4 服务与接入 | T-020~T-023 | daemon + CLI + MCP 三入口可用 |
| M5 方言翻译与内置模型 | T-024~T-027 | 翻译零 AI 成本路径可用（含参数映射与组合闭环）；本地模型插件可装可切 |
| M6 体验与收敛 | T-028~T-033 | 标记语言、日夜主题可用，E2E 全链路通过 → v0.1.0 |
| M7 执行回滚（v2 → v0.2.0） | T-034~T-038 | 效果日志回滚全链路可用（范围回放 + 单文件版本 + 回收站 + history 式列表） |

## 4. v2 预排任务卡（M7 执行回滚，机制见 _02 Q6-S4 效果日志方案）

### T-034 快照仓库 Adapter（US-18，AC-29）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `infra/src/snapstore/mod.rs`、`infra/tests/snapstore_test.rs` |
| 产出方法 | `snapstore_put`（IF-011）、`snapstore_fetch`（IF-012）、`sweep_expired`（IF-013，retention 清理） |
| 验收方式 | 单测：内容寻址去重（同内容单存储）、压缩往返、reflink/硬链接/复制降级链、retention LRU 清理边界 |
| 依赖任务 | T-003 |
| 预估 | 30 分钟 |

### T-035 前像捕获与效果 diff（US-18，AC-29）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/rollback/capture.rs`、`core/src/rollback/diff.rs` |
| 产出方法 | `capture_pre_image`（I-035，orchestration：圈定范围→manifest→入库）、`diff_effects`（I-036，data：manifest vs 现况→EffectEntry 入库） |
| 验收方式 | 单测：范围圈定（cwd 子树/显式路径/影响面并集）、created/modified/deleted/renamed 四类 diff、范围外写入标 out_of_scope |
| 依赖任务 | T-034、T-008 |
| 预估 | 30 分钟 |

### T-036 回滚回放（US-18，AC-30/33/34）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/rollback/replay.rs`、`core/src/rollback/trash.rs` |
| 产出方法 | `rollback_to`（I-037，orchestration：范围回放 `--to <行>` / `--from <行> [--to <行>]`；逐行倒序；created→trash 而非抹除；`--on-conflict <ask|skip|abort>` 冲突决策；IAC-1602/1603/1604 语义） |
| 验收方式 | 单测：三种范围语义正确性（到指定行/区间/到最远）、逐行倒序顺序断言、冲突 ask/skip/abort 三策略、trash 落位、目标已回收报错、范围参数无效报 IAC-1604 |
| 依赖任务 | T-035 |
| 预估 | 35 分钟 |

### T-037 文件版本历史与恢复（US-18，AC-30）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/rollback/file_history.rs` |
| 产出方法 | `file_history`（I-038，data：路径版本列表=各次执行的效果条目）、`restore_file`（I-039，orchestration：取版本内容直接替换，过风险闸门） |
| 验收方式 | 单测：同一文件多次修改的版本链、恢复到任一版本内容断言、恢复动作过闸门 |
| 依赖任务 | T-036 |
| 预估 | 25 分钟 |

### T-038 回滚点列表与日志长度控制（US-18，AC-32/33）

| 字段 | 内容 |
|---|---|
| 涉及文件 | `core/src/rollback/list.rs` |
| 产出方法 | `list_rollback_points`（I-040，data：history 式聚合视图=seq/时间/当时命令/可回滚标识 ✔可回滚/◐部分/✘不可+原因；rollback.max_entries 超限清退联动） |
| 验收方式 | 单测：三态判定（引用存活/out_of_scope/引用已回收）、默认 1024 与上限 65536 配置边界、最旧执行点清退后列表与快照一致 |
| 依赖任务 | T-035 |
| 预估 | 20 分钟 |
