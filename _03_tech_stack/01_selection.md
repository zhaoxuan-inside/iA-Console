# 01 选型清单与依赖约束（_03_tech_stack）

> 状态：draft　更新：2026-10-11

（每个关键选型另行撰写 ADR；本文件是选型总览与依赖账本。版本统一策略：以初始化 `Cargo.toml` 时 crates.io 最新稳定版为准，`Cargo.lock` 入库锁定；下表版本号为写入时的目标基线。）

## 1. 选型清单

| # | 领域 | 候选 | 结论 | 关键理由 | ADR |
|---|---|---|---|---|---|
| 1 | 开发语言 | Rust / Go / TypeScript | Rust | 单二进制跨平台分发、daemon 长驻资源占用低、终端生态成熟（ratatui/atuin 先例）、内存安全 | ADR-001 |
| 2 | 异步运行时 | tokio / async-std / smol | tokio | 事实标准、生态最全（net/process/signal/sync）、维护活跃 | ADR-002 |
| 3 | TUI 框架 | ratatui+crossterm / egui / Bubbletea(需 Go) | ratatui+crossterm | 即时模式 TUI 事实标准、跨平台终端事件、纯 Rust | ADR-003 |
| 4 | 本地存储 | SQLite(rusqlite) / redb / sled / JSON 文件 | rusqlite(bundled) | WAL 读写不互锁、事务+索引满足检索、单文件零部署 | ADR-004 |
| 5 | Agent 接入 | MCP(rmcp) / 自研 HTTP / 仅 CLI | rmcp（stdio MCP Server） | Agent 生态原生发现与调用、工具自描述、复用同一 dispatch | ADR-005 |
| 6 | daemon IPC | UDS+NamedPipe(JSONL) / gRPC / 本地 HTTP | UDS + Windows NamedPipe，JSONL 信封 | 本机零配置、无端口攻击面、协议轻 | ADR-005 |
| 7 | LLM 接入 | 自写 OpenAI 兼容客户端(reqwest) / async-openai / 各家官方 SDK | 自写 OpenAI 兼容客户端 | 一套协议覆盖 GLM/OpenAI/DeepSeek/Ollama/vLLM/本地引擎，Provider 档案化可插拔 | ADR-006 |
| 8 | 内置本地模型 | 插件化引擎子进程(llama-server) / 进程内 FFI(llama-cpp-2) / 依赖 Ollama | 插件化引擎 + GGUF 模型（modelhub 从 HF/魔搭下载安装） | 构建链零 C++ 依赖、引擎与模型独立升级、进程隔离；本地端点同为 OpenAI 兼容协议复用 AiAdapter | ADR-007 |
| 9 | 下载校验 | sha2 / 手写 | sha2 crate | 插件完整性校验（sha256）为安全底线 | —（常规选型） |
| 10 | CLI 解析 | clap / argh / 手写 | clap 4 | derive API 成熟、子命令+补全生成开箱即用 | —（常规选型） |
| 11 | 序列化 | serde / 手写 | serde + serde_json | Rust 序列化事实标准 | —（常规选型） |
| 12 | 日志/观测后端 | tracing / log | tracing | 结构化、span 天然承载会话/请求追踪；作为 metrics 门面的后端实现 | —（常规选型） |
| 13 | 错误处理 | thiserror / anyhow / 手写 | thiserror(库内定义)+anyhow(入口呈现) | 错误码可定义、边界转换轻 | —（常规选型） |

## 2. 依赖与版本约束

| 依赖 | 版本基线 | 用途 | 许可证 | 约束 / 升级策略 |
|---|---|---|---|---|
| tokio | 1.x（full features） | 异步运行时 | MIT | 锁 major；minor 随 Cargo.lock |
| ratatui | 0.29+ | TUI 渲染 | MIT | 0.x 阶段锁 minor，升级走专项 |
| crossterm | 0.28+ | 终端事件/跨平台 | MIT | 同上 |
| rusqlite | 0.3x（bundled feature） | SQLite 驱动 | MIT | bundled 锁定自带 SQLite 版本，避免系统库差异 |
| rmcp | 0.x 最新稳定 | MCP SDK | MIT/Apache-2.0 | 0.x 阶段锁 minor，协议升级专项跟进 |
| clap | 4.x（derive） | CLI | MIT/Apache-2.0 | 锁 major |
| serde / serde_json | 1.x | 序列化 | MIT/Apache-2.0 | 锁 major |
| reqwest | 0.12.x | LLM HTTP 客户端（仅 infra::ai 内） | MIT/Apache-2.0 | 锁 major.minor |
| tracing / tracing-subscriber | 0.1x | 观测后端 | MIT | 锁 major |
| thiserror / anyhow | 2.x / 1.x | 错误处理 | MIT/Apache-2.0 | 锁 major |
| uuid | 1.x | 记录 ID 生成（经 storage Adapter 使用） | MIT/Apache-2.0 | 锁 major |
| sha2 | 0.10.x | 模型插件 sha256 完整性校验（经 infra::modelhub 使用） | MIT/Apache-2.0 | 锁 major |
| llama-server（外部资产，非 crate） | 与模型插件同版本配套 | 本地推理引擎子进程（OpenAI 兼容端点） | MIT | 经 modelhub 下载安装，不入主构建链；版本随插件清单 |

（锁文件入库；引入新依赖前先在此登记并确认许可证。业务代码禁止直接 import 以上依赖，一律经 infra Adapter——serde derive 视为类型系统扩展除外，见 _04 依赖规则。）

## 3. ADR 索引

| ADR | 决策 | 状态 |
|---|---|---|
| ADR-001 | 开发语言选 Rust | accepted |
| ADR-002 | 异步运行时选 tokio | accepted |
| ADR-003 | TUI 选 ratatui + crossterm | accepted |
| ADR-004 | 本地存储选 SQLite（rusqlite bundled） | accepted |
| ADR-005 | Agent 接入选 MCP（rmcp），daemon IPC 选 UDS/NamedPipe JSONL | accepted |
| ADR-006 | LLM 接入选 OpenAI 兼容协议自写客户端 | accepted |
| ADR-007 | 内置模型选插件化引擎+GGUF 模型（HF/魔搭下载安装） | accepted |
