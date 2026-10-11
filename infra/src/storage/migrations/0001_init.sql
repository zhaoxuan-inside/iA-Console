-- Schema v1（_06 internal 第 3 节 DDL）。rollback_journal v1 建表不写入（v2/M7 启用）。

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  cwd TEXT NOT NULL,
  shell TEXT NOT NULL,
  host TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',     -- active|closed|corrupted
  created_at INTEGER NOT NULL,               -- unix ms
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);

CREATE TABLE IF NOT EXISTS session_events (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id),
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,                        -- command|proposal|ai_message|note|diagnosis
  payload TEXT NOT NULL,                     -- JSON
  created_at INTEGER NOT NULL,
  UNIQUE(session_id, seq)
);

CREATE TABLE IF NOT EXISTS proposals (
  id TEXT PRIMARY KEY,
  session_id TEXT REFERENCES sessions(id),
  intent_text TEXT NOT NULL,
  command TEXT NOT NULL,
  argv TEXT NOT NULL,                        -- JSON array
  explanation TEXT NOT NULL,                 -- JSON: ParamSpecDoc + impact
  risk_level TEXT NOT NULL,                  -- safe|caution|dangerous|destructive
  platform TEXT NOT NULL,                    -- linux|macos|windows
  verified INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'proposed',   -- proposed|confirmed|executed|rejected
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_proposals_session ON proposals(session_id, created_at);

CREATE TABLE IF NOT EXISTS executions (
  id TEXT PRIMARY KEY,
  session_id TEXT REFERENCES sessions(id),
  proposal_id TEXT REFERENCES proposals(id),
  command TEXT NOT NULL,
  argv TEXT NOT NULL,                        -- JSON array
  cwd TEXT NOT NULL,
  shell TEXT NOT NULL,
  host TEXT NOT NULL,
  actor TEXT NOT NULL,                       -- human|agent:<client>
  exit_code INTEGER,                         -- NULL=未退出（超时/取消）
  status TEXT NOT NULL,                      -- ok|failed|timeout|cancelled|rejected
  risk_level TEXT NOT NULL,
  stdout_summary TEXT,                       -- ≤64KB
  stderr_summary TEXT,
  stdout_ref TEXT, stderr_ref TEXT,          -- 超限原文落盘引用
  started_at INTEGER NOT NULL,
  duration_ms INTEGER,
  journal_ref TEXT                           -- v2 回滚：执行前像 manifest 在快照仓库的引用
);
CREATE INDEX IF NOT EXISTS idx_exec_time ON executions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_exec_cwd ON executions(cwd);
CREATE INDEX IF NOT EXISTS idx_exec_cmd ON executions(command);

CREATE TABLE IF NOT EXISTS recipes (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT,
  kind TEXT NOT NULL DEFAULT 'command',      -- command|script（组合翻译/脚本工坊生成的脚本）
  name TEXT,                                 -- 脚本工坊命名（kind=script 时全局唯一）
  command_template TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '[]',           -- JSON array
  platform TEXT NOT NULL,                    -- linux|macos|windows|all
  risk_level TEXT NOT NULL,
  usage_count INTEGER NOT NULL DEFAULT 0,
  source_execution_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recipes_title ON recipes(title);
CREATE UNIQUE INDEX IF NOT EXISTS idx_recipes_name ON recipes(name) WHERE name IS NOT NULL;

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

CREATE TABLE IF NOT EXISTS audit_log (
  id TEXT PRIMARY KEY,
  ts INTEGER NOT NULL,
  session_id TEXT,
  actor TEXT NOT NULL,                       -- human|agent:<client>|system
  action TEXT NOT NULL,                      -- execute|confirm|reject|save_recipe|session_resume|translate|...
  target TEXT,
  detail TEXT                                -- JSON
);
CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts DESC);

CREATE TABLE IF NOT EXISTS translations (
  id TEXT PRIMARY KEY,                       -- hash(from_dialect+to_platform+command_from)
  from_dialect TEXT NOT NULL,                -- linux|macos|windows（习惯方言）
  to_platform TEXT NOT NULL,                 -- linux|macos|windows（主机平台）
  command_from TEXT NOT NULL,
  command_to TEXT NOT NULL,
  params_json TEXT,                          -- 参数级映射：[{from,to,kind,note}]
  notes TEXT,                                -- 差异说明
  kind TEXT NOT NULL DEFAULT 'direct',       -- direct|composition
  recipe_id TEXT REFERENCES recipes(id),     -- composition 时指向生成的脚本方案
  verified INTEGER NOT NULL DEFAULT 0,       -- 用户确认后转正
  source TEXT NOT NULL,                      -- builtin|ai|user
  updated_at INTEGER NOT NULL,
  UNIQUE(from_dialect, to_platform, command_from)
);

CREATE TABLE IF NOT EXISTS model_plugins (
  id TEXT PRIMARY KEY,                       -- plugin_id
  name TEXT NOT NULL,
  engine TEXT NOT NULL,                      -- llama-server 及版本
  model TEXT NOT NULL,                       -- GGUF 文件标识
  files TEXT NOT NULL,                       -- JSON: [{path,sha256,size}]
  total_size INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'available',  -- available|active|broken
  source TEXT NOT NULL,                      -- hf|modelscope|hf-mirror
  installed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rollback_journal (  -- v2（M7）启用，v1 建表不写入
  seq INTEGER PRIMARY KEY,                   -- 全局单调，回滚点定位
  execution_id TEXT NOT NULL REFERENCES executions(id),
  path TEXT NOT NULL,
  op TEXT NOT NULL,                          -- created|modified|deleted|renamed
  before_ref TEXT,                           -- 快照仓库内容引用（前像）
  after_ref TEXT,                            -- 快照仓库内容引用（后像）
  extra_path TEXT,                           -- renamed 的原路径
  scope TEXT NOT NULL DEFAULT 'in',          -- in|out_of_scope
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rj_exec ON rollback_journal(execution_id, seq);
CREATE INDEX IF NOT EXISTS idx_rj_path ON rollback_journal(path, seq);
