-- iA-console 重建基线 schema（0001 全量；数据库文件 iac.v2.db，不与旧 iac.db 共存）
-- 重建口径：PRD v1.4 已确认项（Q0~Q49）。

-- 会话（域 6）：env 为声明的环境变量 JSON 数组 [["K","V"],...]（Q46）
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL DEFAULT '',
  cwd TEXT NOT NULL DEFAULT '',
  shell TEXT NOT NULL DEFAULT '',
  host TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active',
  env_json TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- 会话事件流（G6 落盘；pinned=钉住跨 `---` 段，Q33；kind 全量可写，旧版仅 command/proposal 是缺陷）
CREATE TABLE IF NOT EXISTS session_events (
  session_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  pinned INTEGER NOT NULL DEFAULT 0,
  at INTEGER NOT NULL,
  PRIMARY KEY (session_id, seq)
);

-- 提案（域 1）：candidates 为候选数组 JSON（多候选 Q17），context_json 为注入上下文快照（Q20 透明可查）
CREATE TABLE IF NOT EXISTS proposals (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  intent_text TEXT NOT NULL,
  candidates TEXT NOT NULL DEFAULT '[]',
  active INTEGER NOT NULL DEFAULT 0,
  platform TEXT NOT NULL DEFAULT 'linux',
  verified INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'proposed',
  created_at INTEGER NOT NULL,
  context_json TEXT
);

-- 执行记录（域 7）：stdout_full/stderr_full 全量输出入库（Q45）
CREATE TABLE IF NOT EXISTS executions (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  proposal_id TEXT,
  command TEXT NOT NULL,
  argv TEXT NOT NULL DEFAULT '[]',
  cwd TEXT NOT NULL DEFAULT '.',
  shell TEXT NOT NULL DEFAULT 'sh',
  host TEXT NOT NULL DEFAULT 'local',
  actor TEXT NOT NULL DEFAULT 'human',
  exit_code INTEGER,
  status TEXT NOT NULL DEFAULT 'ok',
  risk_level TEXT NOT NULL DEFAULT 'safe',
  stdout_summary TEXT,
  stderr_summary TEXT,
  stdout_full TEXT NOT NULL DEFAULT '',
  stderr_full TEXT NOT NULL DEFAULT '',
  started_at INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL DEFAULT 0,
  journal_ref TEXT
);
CREATE INDEX IF NOT EXISTS idx_executions_started ON executions(started_at);
CREATE INDEX IF NOT EXISTS idx_executions_session ON executions(session_id);

-- 方案库（域 5）：kind=command|script；name 脚本工坊全局唯一（AC-38）
CREATE TABLE IF NOT EXISTS recipes (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT,
  kind TEXT NOT NULL DEFAULT 'command',
  name TEXT,
  command_template TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '[]',
  platform TEXT NOT NULL DEFAULT 'all',
  risk_level TEXT NOT NULL DEFAULT 'safe',
  usage_count INTEGER NOT NULL DEFAULT 0,
  source_execution_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_recipes_name ON recipes(name) WHERE name IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_recipes_script_name ON recipes(name) WHERE kind = 'script' AND name IS NOT NULL;

-- 命令知识缓存（域 1 grounding）
CREATE TABLE IF NOT EXISTS knowledge_cache (
  id TEXT PRIMARY KEY,                       -- hash(command+platform)
  command TEXT NOT NULL,
  platform TEXT NOT NULL,
  schema_json TEXT NOT NULL,                 -- CommandKnowledge
  source_hash TEXT NOT NULL,                 -- help 文本指纹
  verified INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  UNIQUE(command, platform)
);

-- 审计（域 7）：非执行类操作 + 确认/拒绝（Q45）
CREATE TABLE IF NOT EXISTS audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at INTEGER NOT NULL,
  actor TEXT NOT NULL DEFAULT 'human',
  action TEXT NOT NULL,
  target TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_audit_at ON audit_log(at);

-- 方言翻译缓存（域 11）：AI 条目 verified=0（AC-23）
CREATE TABLE IF NOT EXISTS translations (
  id TEXT PRIMARY KEY,
  original TEXT NOT NULL,
  translated TEXT NOT NULL,
  from_platform TEXT NOT NULL,
  to_platform TEXT NOT NULL,
  param_maps TEXT NOT NULL DEFAULT '[]',
  source TEXT NOT NULL DEFAULT 'ai',
  verified INTEGER NOT NULL DEFAULT 0,
  recipe_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(original, from_platform, to_platform)
);

-- 模型插件（域 12）
CREATE TABLE IF NOT EXISTS model_plugins (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  engine TEXT NOT NULL,
  model TEXT NOT NULL,
  files TEXT NOT NULL,
  total_size INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'available',
  source TEXT NOT NULL,
  installed_at INTEGER NOT NULL
);

-- 回滚效果日志（域 15，AC-29~34）
CREATE TABLE IF NOT EXISTS rollback_journal (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  execution_id TEXT NOT NULL,
  path TEXT NOT NULL,
  op TEXT NOT NULL,
  before_ref TEXT,
  after_ref TEXT,
  extra_path TEXT,
  scope TEXT NOT NULL DEFAULT 'in',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_journal_exec ON rollback_journal(execution_id);
