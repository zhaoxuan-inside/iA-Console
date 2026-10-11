# ADR-006-LLM 接入选 OpenAI 兼容协议自写客户端（_03_tech_stack）

> 状态：accepted（修订 2026-10-11：补 Anthropic 规范适配器与本地路径 Provider，见文末修订记录）　日期：2026-10-11

## 背景

_02 Q1/Q2 的结论依赖 LLM（提案生成、诊断分析）。G10/AC-18 要求多后端可插拔：GLM、OpenAI、DeepSeek、Ollama、vLLM 等，可按 Provider 档案切换且不重启 daemon。

## 决策

以 **reqwest** 自写轻量客户端，统一走 **OpenAI Chat Completions 兼容协议**；Provider 档案（base_url、api_key_env、model、参数）配置化，运行时可切换。位于 `infra::ai`，对 core 仅暴露 `AiAdapter` trait。

## 理由

- OpenAI Chat Completions 是行业事实兼容标准：GLM、DeepSeek、Moonshot、Ollama、vLLM 均提供兼容端点，一套客户端覆盖绝大多数后端。
- 自写客户端仅覆盖 chat completions 一条路径（非流式优先，流式 v1.1），代码量小、可控性强，避免引入全家桶抽象。
- Provider 档案化满足 AC-18（切换不重启）。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| async-openai | 绑定 OpenAI 形态与版本节奏，非 OpenAI 后端的兼容差异处理受制于人 |
| 各家官方 SDK | 逐家接入成本高，多数语言 SDK 无 Rust 一等支持 |
| langchain-rust | 抽象过重，本场景只需一条补全路径 |

## 后果

- 正面：一套协议多后端；依赖面最小；Provider 配置即能力。
- 负面/代价：Anthropic 原生协议（非兼容端点）v1 不支持（经兼容端点可用），响应格式异常需自担容错（parse 失败重试一次已在 _01 边界场景约定）。
- 对 _04 设计的影响：`infra::ai` 为唯一 HTTP 出口；core 经 `AiAdapter` trait 依赖，单测以 mock 实现注入。

## 修订记录（2026-10-11，评审补充）

Provider 档案扩展为四类，`infra::ai` 对内实现两套协议适配器、对外仍是 IF-005 单一入口：

| type | 说明 | 协议 |
|---|---|---|
| openai | GLM/OpenAI/DeepSeek/Ollama/vLLM 等兼容端点 | OpenAI Chat Completions |
| anthropic | Claude 系列原生远端，无需兼容网关 | Anthropic Messages API（x-api-key） |
| local-plugin | modelhub 插件安装的本地引擎（ADR-007） | OpenAI 兼容（127.0.0.1） |
| local-path | 用户直接配置的本地模型文件路径（GGUF），经插件引擎加载 | OpenAI 兼容（127.0.0.1） |

原"负面/代价"中 Anthropic 原生协议不支持的条目作废：anthropic 型 Provider 由独立适配器直连，请求/响应在 `infra::ai` 内转换为统一 LlmResponse，core 无感知。
