# 上下文装载包 (Context Pack)：iaconsole-rebuild

- **change_size**：`L`
- **任务 ID**：`iaconsole-rebuild`
- **生成时间**：`2026-10-17T00:37:34Z`
- **状态**：`READY`
- **装载纪律**：必须物化切片正文；禁止只写路径。
- **查询**：`doc_index_db.py --root . search|slice`；Skill 详规用 `skill_ref_index.py search`（勿通读 references/）

---

## 1. 需求与目标（物化）
以 hs-dev-skill 全流程从零重建 iA-console（伴生智能内核）：需求钉到可验收细节并逐项签认；全量一步到位（旧全部能力+v2 回滚/脚本工坊）；双形态（GUI 主形态 + CLI/TUI）共享内核；三平台（Linux/macOS/Windows）全支持。

<!-- Agent：用 doc_index_db search/slice 粘贴 PRD/DoD 关键段落，勿只留链接；S 可仅 3～8 行 DoD -->

---

## 2. 数据模型切片（物化）
<!-- 粘贴相关表字段/约束原文片段；并注明 path + line range；未触达写 N/A -->

---

## 3. 接口与六元组签名（物化）
<!-- 粘贴方法签名、入参/出参、compensate/fallback；改 API 时读 api_engineering.md -->

---

## 4. 调用链与 Mock 契约（物化）
<!-- 粘贴依赖方法清单与契约关键字段 -->

---

## 5. 查询轨迹（降 Token）
- `doc_index_db` 命中 path 列表：
- `skill_ref_index` 命中 path 列表：
- 本会话已读 Skill references（禁止重复整读）：
