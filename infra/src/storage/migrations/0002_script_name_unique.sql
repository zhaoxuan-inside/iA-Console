-- Schema v2（M8 脚本工坊）：脚本名全局唯一（kind=script 且 name 非空）。
CREATE UNIQUE INDEX IF NOT EXISTS idx_recipes_script_name
  ON recipes(name) WHERE kind = 'script' AND name IS NOT NULL;
