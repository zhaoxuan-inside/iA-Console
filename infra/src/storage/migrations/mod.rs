//! 嵌入迁移资产。重建基线 schema 与旧 iac.db 不兼容（新库文件名 iac.v2.db 隔离），
//! 0001 即全量 schema：会话含 env（Q46）、执行记录含全量输出（Q45）、事件含 pinned 对话轨（Q33）。
