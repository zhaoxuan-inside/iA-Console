# iA-console

> 天下没有难用的 Console。

iA-console 是一个**伴生智能内核**（daemon + CLI/TUI），不替换你现有的终端，而是在旁边补齐命令行缺失的能力：

- **意图输入**：说人话拿命令——自然语言 → 结构化提案（命令 + 逐参数白话解释 + 风险等级）
- **错误医生**：命令失败自动诊断，给出根因与修复提案
- **风险闸门**：危险命令先展示影响面、二次确认
- **全量记录**：每次执行（含 Agent 发起）留痕命令、输出、上下文、审计
- **方案库**：好命令存下来、打标签、可检索
- **会话持久化**：关掉终端，回来接着干
- **Agent 原生**：内置 MCP Server，外部 Agent 可查历史、执行命令、存方案（同一套闸门与审计）

跨 Linux / macOS / Windows。AI 后端可插拔（OpenAI 兼容协议：GLM / OpenAI / DeepSeek / Ollama 等）。

## 状态

设计阶段（P1~P6 已落盘，见 `_00_project_idx.json`），编码未开始。

## 文档入口

阅读从 [`_00_project_idx.json`](_00_project_idx.json) 开始。

## License

MIT
