# ADR-007-内置模型选插件化引擎+GGUF 模型（HF/魔搭下载安装）（_03_tech_stack）

> 状态：accepted　日期：2026-10-11

## 背景

G12/AC-21/22 要求"内置模型"：不配置任何 API Key 也能开箱使用 AI 能力，且模型以插件形式从 HuggingFace 或魔搭 ModelScope 下载安装。_02 Q7 对比了进程内 FFI、插件化引擎子进程、依赖 Ollama 三条路径。

## 决策

- **推理引擎**：llama.cpp 的 `llama-server`（OpenAI 兼容端点）作为外部资产，**不入主构建链**，由 modelhub 随插件包下载安装，daemon 以子进程拉起并做 supervisor 管理。
- **模型**：GGUF 权重文件，默认插件推荐小参数量 instruct 模型（选型 open，候选 Qwen2.5-Coder-1.5B / Qwen3-1.7B 级别，GGUF Q4 档）。
- **下载源**：HuggingFace / 魔搭 ModelScope / HF 镜像（hf-mirror），源可配置；清单含 sha256、大小、上下文长度，安装时强校验。
- **接入方式**：本地引擎注册为 Provider 档案 `local-builtin`（base_url=127.0.0.1:<port>），对 core 而言与云后端无差别——全部经 `AiAdapter`（IF-005）的 OpenAI 兼容协议调用；支持云不可用时自动降级本地（AC-22）。

## 理由

- 主构建链保持纯 Rust：不引入 cmake/C++ 工具链，三平台发布矩阵不翻倍（_02 Q7-S1 的主要风险）。
- 引擎与模型独立升级，互不牵连；引擎崩溃被进程隔离，supervisor 重启即可恢复。
- 本地端点与云后端同协议，AiAdapter 零改动复用——"内置模型"对架构只是多一个 Provider。
- HF + ModelScope 双源覆盖国内外网络环境，镜像可配。

## 备选方案

| 方案 | 未选原因 |
|---|---|
| 进程内 FFI（llama-cpp-2） | 构建链引入 C++/cmake，跨平台构建矩阵复杂；引擎崩溃拖垮 daemon；二进制体积大。v2 若需降低调用延迟再评估 |
| 依赖 Ollama | "开箱可用"依赖用户自装，违背 G12；保留为兼容 Provider 档案（用户已装 Ollama 时直接填 base_url 即用） |

## 后果

- 正面：零 API Key 开箱可用（一次引导式下载）；发布矩阵简单；引擎/模型/源三者皆可替换。
- 负面/代价：首次使用需下载数百 MB 级插件包（断点续传+多源缓解）；本地小模型质量低于云后端（定位为离线兜底与隐私路径，云路径仍为质量首选）；子进程端口与生命周期管理需 supervisor 逻辑。
- 对 _04 设计的影响：新增 infra 模块 modelhub（IF-008 modelhub_install / IF-009 modelhub_list）与 infra::ai 内 local_engine_ensure（IF-010）；新增 model_plugins 表与 PluginInfo 模型；引擎子进程经 process Adapter 规约管理。
