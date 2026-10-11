# ADR-004-本地存储选 SQLite（rusqlite bundled）（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

_02 Q3 结论：全量执行记录、会话事件流、方案库、审计日志、知识缓存需要"多条件检索 + 事务追加 + 多客户端并发 + 单文件零部署"。数据规模个人级（十万行内），AC-13 要求万条级检索秒回，AC-16 要求并发不互锁。

## 决策

选定 **SQLite**，驱动 **rusqlite**（bundled feature，编译期打包 SQLite，避免系统库版本差异），运行时启用 **WAL 模式**；数据库单文件 `~/.local/share/iac/iac.db`（Windows 为 `%LOCALAPPDATA%\iac\iac.db`，经 dirs crate 定位）。

## 理由

- WAL：读不阻塞写，满足 TUI/CLI/MCP 多客户端并发（AC-16）。
- 索引 + SQL：时间/目录/命令/状态多条件组合检索开箱即用（AC-13），后续可平滑启用 FTS5 全文检索。
- 事务：会话事件流、审计的追加式写入天然原子（AC-07）。
- bundled 单文件零部署，随二进制分发无系统依赖。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| redb / sled | KV 无二级索引，多条件检索需自建倒排，等于重写半个数据库 |
| JSON 文件集 | 万条级扫描慢、无原子批量写、并发锁脆弱 |

## 后果

- 正面：检索、事务、并发、演进（FTS5）四项全满足。
- 负面/代价：SQL 语句散布于 data 方法（以 storage Adapter 薄原语约束书写边界）；需要版本化迁移机制（v1 内建 schema_migrations 表）。
- 对 _04 设计的影响：storage Adapter 只暴露 query/execute/migrate 三个原语；SQL 归属各模块 data 方法；写操作经 daemon 单线程化串行执行，规避多写者竞争。
