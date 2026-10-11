# 02 数据流与调用流（_04_framework_design）

> 状态：draft　更新：2026-10-11

## 1. 核心数据流

### 流 1：意图 → 提案 → 执行 → 记录（主闭环，AC-01/04/05）

```mermaid
sequenceDiagram
    participant U as 用户(TUI/CLI)
    participant S as server(dispatch)
    participant I as intent
    participant K as knowledge
    participant AI as infra::ai
    participant R as risk
    participant E as executor
    participant P as infra::process
    participant H as history
    participant SE as session

    U->>S: ask(意图文本)
    S->>I: handle_ask
    I->>SE: ensure_session（取/建当前会话）
    I->>K: lookup_knowledge(命令族)
    K-->>I: ParamSpec 集合（可空→降级纯 LLM）
    I->>AI: ai_complete(意图+知识+平台)
    AI-->>I: LlmResponse
    I->>I: parse_proposal_response（JSON 结构校验）
    I->>R: assess_risk(命令)
    R-->>I: RiskAssessment
    I->>SE: append_session_event(proposal)
    I-->>U: Proposal（命令+解释+风险+影响面）
    U->>S: confirm(proposal_id)
    S->>E: confirm_and_execute
    E->>R: assess_risk（终检）
    Note over E: dangerous/destructive → 要求确认标志
    E->>P: spawn_process(命令+cwd+env)
    P-->>E: ProcessResult(exit_code/stdout/stderr)
    E->>H: record_execution + append_audit_entry
    E->>SE: append_session_event(command)
    alt 退出码 = 0
        E-->>U: ExecutionOutcome(ok)
    else 退出码 ≠ 0
        E->>S: 转诊断（见流 2）
    end
```

### 流 2：失败 → 诊断（AC-03）

```mermaid
sequenceDiagram
    participant E as executor
    participant D as doctor
    participant H as history
    participant AI as infra::ai
    participant U as 用户/Agent

    E->>H: get_execution_detail(失败记录)
    E->>D: diagnose_failure(记录)
    D->>D: build_failure_context（stderr+退出码+cwd+近 N 条历史；先过静态快速通道）
    D->>AI: ai_complete(诊断 prompt)
    D->>D: parse_diagnosis_response（要求证据引用）
    D-->>U: Diagnosis（根因+证据+置信度+修复提案）
    U->>E: 采纳修复提案 → 走流 1 确认执行
```

### 流 3：外部 Agent 经 MCP（AC-08）

```mermaid
sequenceDiagram
    participant A as 外部 Agent
    participant M as mcp handler(豁免)
    participant S as server(dispatch)
    participant CORE as core 编排方法

    A->>M: tools/call iac_execute(command, confirm_dangerous?)
    M->>S: 同一 IPC 信封（method=iac_execute）
    S->>CORE: handle_execute（同闸门同审计）
    CORE-->>A: 结构化结果（exit_code/输出摘要/记录 ID）
```

### 流 4：跨方言直接输入翻译（AC-19/20/23/24，零 AI 成本优先）

```mermaid
sequenceDiagram
    participant U as 用户(TUI/CLI)
    participant S as server(dispatch)
    participant T as intent(translate_command)
    participant K as knowledge
    participant AI as infra::ai
    participant RC as recipe

    U->>S: 直接输入命令（如 ls -la；习惯方言=linux，主机=windows）
    Note over S,T: 习惯方言=主机方言 → 不翻译，直入风险闸门
    S->>T: translate_command(command, from=habit, to=host)
    T->>K: lookup_translation(静态映射表：命令级+参数级)
    alt 静态命中（含参数四类映射说明）
        K-->>T: 等价命令+参数映射表+差异说明（零 AI 调用，AC-23）
    else 一对一可译但未收录
        T->>AI: ai_complete(双方言翻译)
        T->>K: cache_translation(verified=false 回写)
    else 需命令组合（不可一对一）
        T->>AI: ai_complete(组合规划)
        T->>RC: save_recipe(平台化脚本, tags=translation/auto)
        T->>K: cache_translation(kind=composition, recipe_id)
    end
    T-->>U: 译文行（等价命令/脚本 + 风险；差异说明与参数映射收起，e/--explain 按需展开）→ 确认后走流 1 执行（verified 转正）
    Note over T: AI 不可用时返回原命令+说明，不阻塞直接执行
```

### 流 5：执行回滚——效果日志（v2，AC-29/30/31，无逐命令判断）

```mermaid
sequenceDiagram
    participant E as executor
    participant RB as rollback
    participant SS as infra::snapstore
    participant U as 用户

    Note over E,SS: 执行前：统一策略圈定捕获范围（cwd 子树+显式路径+影响面）
    E->>SS: snapstore_put(捕获范围前像 manifest，内容寻址去重)
    E->>E: 正常执行（不转译、不判断可逆性）
    E->>RB: diff_effects(前像 manifest vs 现况)
    RB->>RB: 效果日志入库（每文件 op/before/after，全局 seq）
    Note over U,RB: 事后（history 式列表：行号 seq/时间/当时命令/可回滚标识 ✔◐✘）
    U->>RB: iac rollback --list / --to <行> / --from <行> [--to <行>] [--on-conflict <ask|skip|abort>]
    RB->>SS: snapstore_fetch(各条目前像)
    RB->>RB: 按指定范围逐行倒序回放：modified→前像替换 / deleted→恢复 / created→移入回收站 / renamed→反向改名
    Note over RB: 冲突条目按 on-conflict 策略处理：ask 逐条询问 / skip 默认跳过 / abort 中止；决策计入报告
    RB-->>U: 回滚报告（applied/skipped+原因/回收站位置）
    U->>RB: iac file history <路径>
    RB-->>U: 该文件版本列表（每次执行一条：时间+命令+变化）
    U->>RB: iac file restore <路径> --to <版本>
    RB->>SS: 取该版本内容直接替换
```

### 流 6：脚本工坊——范围变一个命令（v2，AC-38/39/40）

```mermaid
sequenceDiagram
    participant U as 用户
    participant S as server(dispatch)
    participant SC as script
    participant RC as recipe
    participant AI as infra::ai
    participant E as executor(链)

    U->>S: iac script make --segment <任务段> \| --from <行> [--to <行>] --name <名> [--refine]
    S->>SC: collect_commands(范围)
    SC-->>SC: assemble_script（确定性拼接 + 平台头部 set -e / ErrorActionPreference）
    alt --refine 显式触发
        SC->>AI: ai_complete(参数化/预检/跨方言精修)
        SC-->>U: 修订 diff → 确认后覆盖（原稿保留）
    end
    SC->>RC: save_recipe(kind=script, name 唯一, platform=all)
    Note over U,E: 此后"变成一个命令"
    U->>S: iac run @<名>（或 script run）
    S->>SC: run_script → 风险评估（内部最高）→ 闸门 → 执行 → 记录 + 审计
    Note over SC,E: 文件效果纳入回滚日志（流 5 覆盖）
```

## 2. 主调用链

以"用户意图到执行落库"为例（方法名以 _06 编号为准）：

```text
入口(iac cli/tui · 豁免) → dispatch_request(I-030, orchestration/app/server)
  → handle_ask(I-025, orchestration/app/intent)
      → ensure session: create_session / load_session(I-016/I-018, data/domain/session)
      → lookup_knowledge(I-003, data/domain/knowledge) → parse_help_text(I-002, algorithm/common)（未命中时）
      → ai_complete(IF-005, data/infra/ai)
      → parse_proposal_response(I-006, data/domain/intent)
      → assess_risk(I-023, orchestration/domain/risk) → match_risk_rules(I-001, algorithm/common)
      → append_session_event(I-017, data/domain/session)
  → confirm_and_execute(I-028, orchestration/app/executor)
      → spawn_process(IF-003, data/infra/process)
      → record_execution(I-008) + append_audit_entry(I-011)(data/domain/history)
      → [失败] diagnose_failure(I-029, orchestration/app/doctor)
            → build_failure_context(I-021, data/domain/doctor) → ai_complete(IF-005)
```

每个节点标注：方法名（_06 接口 ID）、category、层、模块；登记 _07 时映射为 m-xxx。

## 3. 数据模型概览

| 实体 | 关键字段 | 存储 | 归属模块 |
|---|---|---|---|
| Session | id/title/cwd/shell/host/status/时间 | SQLite sessions | session |
| SessionEvent | session_id/seq/kind/payload(JSON) | SQLite session_events | session |
| Proposal | id/intent/command/explanation/risk/platform/status | SQLite proposals | intent |
| ExecutionRecord | id/session_id/command/argv/cwd/actor/exit_code/输出摘要+落盘引用/risk/耗时/status/journal_ref | SQLite executions | history |
| RollbackJournal | seq/execution_id/path/op(created\|modified\|deleted\|renamed)/before/after（内容寻址引用） | SQLite rollback_journal + 快照仓库 | rollback |
| Snapshot（快照仓库） | 内容寻址对象（去重+压缩）+ manifest | `~/.local/share/iac/snapshots/` | rollback/snapstore |
| AuditEntry | ts/actor/action/target/detail(JSON) | SQLite audit_log | history |
| Recipe | id/title/description/command_template/tags/platform/risk/usage_count | SQLite recipes | recipe |
| CommandKnowledge | command/platform/schema(JSON)/source_hash/verified | SQLite knowledge_cache | knowledge |
| CommandTranslation | from_dialect/to_platform/command_from/command_to/notes/verified | SQLite translations | knowledge |
| ModelPlugin | id/name/engine/files(sha256)/size/status/installed_at | SQLite model_plugins | modelhub |

完整 DDL 见 _06 internal 第 3 节。关系：Session 1—N SessionEvent/ExecutionRecord/Proposal；ExecutionRecord 0—1 Proposal；Recipe 0—1 源 ExecutionRecord。

## 4. 容量与扩展假设

- 个人使用强度：执行记录 ~200 条/天 → 10 年约 73 万行；SQLite 单机轻松承载，索引检索毫秒级，满足 AC-13。
- 输出存储：单条 stdout/stderr 摘要上限 64KB，原文超 1MB 落盘文件（`~/.local/share/iac/outputs/`）记录引用，防库膨胀。
- 知识缓存：按 command+platform 唯一，量级千行内，无增长压力。
- 演进边界：单用户单机为设计边界；若未来多设备同步，引入行级 updated_at + 附加同步 journal 即可，不影响现 schema 主体；多主机管理属新需求走迭代流程。
