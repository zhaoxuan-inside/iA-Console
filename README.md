# iA-console

> 天下没有难用的 Console。

iA-console 是一个**伴生智能内核**（daemon + CLI/TUI），不替换你现有的终端，而是在旁边补齐命令行缺失的能力：

- **意图输入**：说人话拿命令——自然语言 → 结构化提案（命令 + 逐参数白话解释 + 风险等级）
- **习惯方言翻译**：习惯 Linux 就一直用 `ls`——在 Windows 上自动翻译成等价命令并解释差异（含参数级对应）；翻译不了的组合命令由 AI 规划成脚本并沉淀进映射，下次零成本
- **输入标记**：`??ls` 让 AI 解释、`/config` 进配置、`---` 开新任务段、`#` 写备注——手不离键盘
- **错误医生**：命令失败自动诊断，给出根因与修复提案
- **风险闸门**：危险命令先展示影响面、二次确认
- **执行回滚（v2 规划）**：效果日志记录每次执行的文件级变化——回滚到任意执行点、单文件版本恢复；删除先进回收站、自动清理，不靠 git 不用判断命令类型
- **全量记录**：每次执行（含 Agent 发起）留痕命令、输出、上下文、审计
- **方案库**：好命令存下来、打标签、可检索
- **会话持久化**：关掉终端，回来接着干
- **内置本地模型**：模型插件（引擎+权重）从 HuggingFace/魔搭下载安装，也可直配本地模型路径或 OpenAI/Anthropic 远端
- **iA-Writer 式界面**：留白、弱框线、焦点模式、白天/夜晚主题
- **Agent 原生**：内置 MCP Server，外部 Agent 可查历史、执行命令、存方案（同一套闸门与审计）

跨 Linux / macOS / Windows。AI 后端可插拔（OpenAI 兼容协议：GLM / OpenAI / DeepSeek / Ollama 等）。

## 快速开始（v0.1.0，Linux/macOS）

```bash
cargo build --release
./target/release/iacd --foreground        # 终端 1：守护进程
./target/release/iac --json run "echo hello"   # 终端 2：闸门执行
./target/release/iac history search hello --json
./target/release/iac translate "Get-ChildItem" --from windows  # 方言翻译（零 AI）
./target/release/iac mcp serve            # Agent 接入（MCP stdio）

# 需要 AI 能力：配置 Provider 后
iac config path                           # 找到 config.toml
iac ask "找出最近7天修改的10个最大文件"
```

## 状态

**v0.1.0 已实现**（P7 编码完成）：M1 基础设施 / M2 记录域 / M3 智能域 / M4 服务接入 / M5 翻译+modelhub / M6 体验件。v2 预排：执行回滚（M7 效果日志）与脚本工坊（M8）。

架构与流程文档从 [`_00_project_idx.json`](_00_project_idx.json) 读起；120 项自动化测试（`cargo test --workspace`）。

## 文档入口

阅读从 [`_00_project_idx.json`](_00_project_idx.json) 开始。

## License

MIT
