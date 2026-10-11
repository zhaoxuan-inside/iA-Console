# ADR-005-Agent 接入选 MCP（rmcp）、daemon IPC 选 UDS/NamedPipe JSONL（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

_02 Q4/Q5 结论：外部 Agent 需要结构化、可发现、与人类同构的接入方式（痛点 9）；前端 TUI 需要与 daemon 常驻通信。接入面不能引入端口攻击面，且必须与人类执行同一闸门与审计。

## 决策

- **Agent 接入**：以 **rmcp**（官方 MCP Rust SDK）实现 stdio MCP Server（`iac mcp serve`），暴露 `iac_` 前缀工具集。
- **daemon IPC**：Unix domain socket（`$XDG_RUNTIME_DIR/iac/daemon.sock`）/ Windows named pipe（`\\.\pipe\iac-daemon`），**JSONL 信封协议**（`{v,id,method,params}` → `{v,id,ok,result|error}`）。
- MCP 工具与 IPC method 共享同一 dispatch 与同一方法集，MCP handler 与 TUI 均为 daemon 服务的薄适配。
- 本机安全：socket 文件权限/管道 ACL 限定本机用户；v1 不做远程网络监听（HTTP API 为 v1.1 评估项）。

## 理由

- MCP 是 Agent 生态原生协议：工具自描述（schema 即文档），ZCode/Claude Code 等零适配接入，直接命中痛点 9。
- UDS/NamedPipe：本机零配置、内核级通道、无端口暴露，比本地 HTTP 少一层鉴权与攻击面。
- 单一 dispatch 承载两种入口，保证"人机同构"（AC-08）实现上无分叉。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| 仅 CLI 文本协议 | 输出面向人，Agent 解析脆弱、无能力发现性；保留 `--json` 作为辅助出口 |
| 本地 HTTP/REST | 需端口与鉴权设计，Agent 侧无自动发现；v1 不做 |
| gRPC | 强类型但 Agent 生态不原生，.proto 分发负担 |

## 后果

- 正面：Agent 接入零边际适配；入口统一；本机安全模型简单。
- 负面/代价：需同时维护 MCP schema 与 IPC envelope 两层薄适配（共享 payload 模型，成本可控）；rmcp 处于 0.x，协议升级需跟进。
- 对 _04 设计的影响：server 模块承担 dispatch + 两个接入适配；IPC 协议版本字段 v1 起步，不兼容变更升 major。
