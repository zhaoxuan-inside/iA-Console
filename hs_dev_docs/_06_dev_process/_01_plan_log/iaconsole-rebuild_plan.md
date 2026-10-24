# 任务计划账本：iaconsole-rebuild

- **task_id**：`iaconsole-rebuild`
- **标题**：iA-console 从零重建（hs-dev-skill 全流程）
- **change_size**：`L`
- **work_mode**：`DESIGN`
- **当前阶段**：`0`（需求发现与产品对齐）
- **断点**：`S6` 实现主体完成 — 重建五 crate 全部落成（2026-10-24）：检查点封存 `legacy/pre-rebuild`（d3d2bb1）→ 删旧 → 骨架契约（a671244）→ infra（b7581c2）→ core 服务簇（da529ca）→ core 执行簇（8802210）→ iacd+iac（c400a4d）→ iac-gui+联调修复（1c246f5）；全工作区 119 测试绿、clippy/fmt 零告警、端到端冒烟全链路通过（会话绑定/三账/agent 留痕/脚本工坊/双限清理/内置路由/MCP）。待办：DISC/PROD 冻结签收前——原型走查（呈现细则）、剩余 OPEN 域 PRD 回填（按实现平价）、打包（PKGBUILD 适配 0.2.0）
- **更新时间**：`2026-10-24T15:00:00Z`

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
| S1 | 阶段 0：需求深挖审讯（逐能力场景走查 + 原型）→ DISC/PROD 冻结签收 | doing | 第 1~14 轮闭环（Q0~Q49）；PRD 骨架 v1.4；域 1/4/6/7/8/9/13/18 八域需求面关闭；**开发者 2026-10-24 裁定：剩余 OPEN 域（2/3/5/10/11/12/14/16/17）细化走查后置，按现有功能语义平价重建后回填 PRD** |
| S2 | 阶段 1：技术选型与 ADR（TECH 验收） | merged→S6 | 开发者 2026-10-24 裁定并入实现：按既有 ADR（_02_principle，参考事实）直接落地，不另行走查 |
| S3 | 阶段 2：系统架构设计（ARCH 验收） | merged→S6 | 同上：沿用已验证架构（core 纯域+ports、infra 适配、iacd 组合根），重建中落地 |
| S4 | 阶段 3：详细契约设计 DB/API/UI（DETAIL 验收） | merged→S6 | 同上：DB 迁移/信封/内置命令名单随实现定稿，实现后回填文档 |
| S5 | 阶段 4：任务拆解与上下文装配（TASK 验收） | merged→S6 | 拆解记录见重建执行序列（infra→core→iacd/iac→iac-gui） |
| S6 | 阶段 5：垂直切片实施（IMPL 验收，TDD） | doing | **实现主体完成（2026-10-24）**：五 crate 重建落成，119 测试绿/端到端冒烟通过；证据=git 提交链 d3d2bb1→1c246f5 + 冒烟记录（会话绑定/三账/agent 留痕/脚本工坊/内置路由/MCP）。待 IMPL 验收提交与开发者走查 |
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
| B2 | 旧代码与未提交改动的物理处置方式 | **已终裁（2026-10-24，开发者重建启动令）：移除重建**；移除前检查点封存（本地 commit + tag `legacy/pre-rebuild`，不推送）；旧文档仅删代码不删 |
| B3 | 「AI 驱动命令核心思想」的具体形态定义 | 已裁决（Q7：主输入行直驱/方案 A；交互细则待域走查） |
| B4 | UI/交互质量问题全清单与验收标准 | 已收口（Q9 清单式验收；Q12 起步清单确认） |
| B5 | MVP 裁定 Q1/Q2/Q3（在范围/核心思想定型后） | 已关闭（Q13 核心假设=主输入行直驱闭环；Q14 全量无裁剪） |
| B6 | 三平台验证资源（可验证设备清单） | 已关闭（Q10/Q11：Linux/Windows/macOS 实机 + 云端 CI） |

## 下一步

重建执行序列（开发者 2026-10-24 启动令）：检查点封存（tag `legacy/pre-rebuild`）→ 删除 core/iac/iacd/iac-gui/infra 源码（保留 vendor/egui-term 与全部文档）→ 契约层重建（types/error/ports/envelope/config）→ infra（storage 含全量输出与双限 LRU 接线 / ai / modelhub / snapstore）→ core（混合识别分流 + `ia-` 内置命令命名空间 / 四级风险+确认升级 / 会话含 env 与对话轨 / 执行三账+全量输出 / intent 多候选+上下文注入 / doctor / translate / rollback 效果日志 / script 工坊补测试 / dispatch）→ iacd（UDS serve + daemon 内置 MCP 模式）→ iac（CLI + `--json` + console TUI）→ iac-gui（多窗口×标签×独立会话 / 启动恢复全部布局 / 自动拉起 daemon / 管理功能终端内渲染 / 主题持久化）→ 测试全绿 + clippy/fmt 零告警 → 本地 commit。域 2/3/5/10/11/12/14/16/17 的 PRD 细化走查在重建后按实现回填。
