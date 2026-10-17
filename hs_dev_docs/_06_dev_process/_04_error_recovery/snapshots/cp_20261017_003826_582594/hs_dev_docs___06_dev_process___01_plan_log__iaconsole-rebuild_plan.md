# 任务计划账本：iaconsole-rebuild

- **task_id**：`iaconsole-rebuild`
- **标题**：iA-console 从零重建（hs-dev-skill 全流程）
- **change_size**：`L`
- **work_mode**：`DESIGN`
- **当前阶段**：`0`（需求发现与产品对齐）
- **断点**：`S1` — 第二轮审讯（AI 驱动核心思想定型 / UI 交互问题清单取证 / 开箱体验 / 验证环境）
- **更新时间**：`2026-10-16T01:15:00Z`

---

## 目标

以 hs-dev-skill 全流程从零重建 iA-console（伴生智能内核）。核心纪律（来自开发者确认的重建动机）：**需求必须沟通到「可验收的细节」并由开发者逐项签认**，杜绝再次出现「需求沟通不细 → 实现完全不符预期」。

## 范围 / 非目标

- **范围**：BUILD 阶段 0→6 全链，**全量一步到位**（旧产品全部能力 + v2 回滚/脚本工坊）；**双形态同步推进**（GUI 与 CLI/TUI 均为一等交付、共享内核与契约）；**三平台全支持**（Linux/macOS/Windows）；**质量优先、不设硬时间箱**。每条需求钉到 Gherkin/契约级细节。
- **头号验收主题**（来自 Q2 取证）：① 「AI 驱动命令」核心思想必须在主交互中真实兑现；② UI/交互质量（抖动等问题）须有可验收的标准清单。
- **非目标（本阶段）**：不触碰旧代码与未提交改动；旧文档（`_00~_10`）不作为新基线，仅作参考事实。
- **待裁决（实施前）**：旧代码与未提交 v2 改动的物理处置（归档 / 分支 / 移除）。

## 基线版本

- 产品基线：无（从零重建）。
- 参考输入：`_01_business/01_requirements.md`（十痛点/23 US/46 AC，**仅供参考事实**）、`README.md`、`_10_change_log.json`（chg-001~022）、git `f308793`（旧工程末次提交，未提交 v2 改动搁置）。
- 本次过程记录：`hs_dev_docs/`（2026-10-16 初始化）。

## 步骤

| step_id | 描述 | 状态 | 备注 |
|---------|------|------|------|
| S1 | 阶段 0：需求深挖审讯（逐能力场景走查 + 原型）→ DISC/PROD 冻结签收 | doing | 第 1 轮审讯已发（范围/形态/平台/节奏/取证） |
| S2 | 阶段 1：技术选型与 ADR（TECH 验收） | pending | |
| S3 | 阶段 2：系统架构设计（ARCH 验收） | pending | |
| S4 | 阶段 3：详细契约设计 DB/API/UI（DETAIL 验收） | pending | |
| S5 | 阶段 4：任务拆解与上下文装配（TASK 验收） | pending | |
| S6 | 阶段 5：垂直切片实施（IMPL 验收，TDD） | pending | |
| S7 | 阶段 6：交付验收与签收（DELIV 验收） | pending | |

## 对应指导文档

- 开场与审讯：`references/opening_alignment.md`、`references/grill_orchestration.md`、`references/confidence_decision.md`
- 阶段 0 验收：`references/product_acceptance.md`（DISC/PROD 清单）
- 各阶段：`references/stage_acceptance.md`、`references/engineering_acceptance.md`

## 验收项与证据位置

- 阶段 0：DISC/PROD 证据 → `hs_dev_docs/_01_product_vision/`、`_02_feature_spec/`、Q&A 日志 `_06_dev_process/_03_qa_log/qa_log.md`
- 阶段 1~3：ADR → `_00_project_foundation/_03_decision_record/`；架构/契约 → `_03_architecture_design/`、`_04_engineering_contract/`
- 阶段 4~6：任务/上下文包/变更/验证 → `_05_task_orchestration/`、`_06_dev_process/`
- 开发者签收：冻结确认单 → `hs_dev_docs/_06_dev_process/`（按模板 `freeze_signoff.md` / `stage_acceptance.md`）

## 阻断项 / 待决

| # | 内容 | 状态 |
|---|------|------|
| ~~B1~~ | ~~第一轮审讯答复（目标范围 / 首做形态 / 平台范围 / 节奏 / 不符预期取证）~~ | 已裁决（2026-10-16，见 QA 日志 Q1~Q6） |
| B2 | 旧代码与未提交改动的物理处置方式 | 实施阶段前裁决 |
| B3 | 「AI 驱动命令核心思想」的具体形态定义 | 第二轮审讯进行中 |
| B4 | UI/交互质量问题全清单与验收标准 | 第二轮审讯进行中 |
| B5 | MVP 裁定 Q1/Q2/Q3（在范围/核心思想定型后） | 第三轮审讯 |
| B6 | 三平台验证资源（可验证设备清单） | 第二轮审讯 Q4 |

## 下一步

收到第一轮审讯答复 → 落 QA 日志与 PRD 骨架 → 展开第二/三轮审讯（按返工代价排序：核心闭环场景 → 业务规则 → 视觉偏好）。
