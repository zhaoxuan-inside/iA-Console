//! 内置命令（域 13，Q24~Q30）：系统管理功能统一 `ia-` 命名空间，废弃旧 `/` 前缀。
//! 与外部命令同一分流（Q15：`ia-` 零撞名，「内置优先于 PATH」仅兜底保险）。
//!
//! 路由映射（dispatch 层）：各 builtin 按子命令映射到 `iac_*` 方法——
//! - ia-help → 就地列出全部内置命令（无需 daemon）；
//! - ia-config [get|set ...] → iac_config_get / iac_config_set；
//! - ia-model [list|use <名>] → iac_config_get / iac_config_set（ai.default_provider）；
//! - ia-daemon [status|stop] → iac_ping / iac_daemon_stop（start 由前端拉起，Q40）；
//! - ia-history [query|clean|export] → iac_history_query / iac_history_clean / iac_history_export；
//! - ia-recipe [save|search|show|rm|export] → iac_recipe_save / iac_recipes_search / …；
//! - ia-sessions [list|resume <id>] → iac_session_list / iac_session_resume（Q46）；
//! - ia-rollback [list|replay ...] → iac_rollback（Q49 双触达之命令侧）；
//! - ia-file-history <路径> → iac_rollback file_history（Q49）；
//! - ia-script [make|refine|run] → iac_script；
//! - ia-theme [day|night|auto] → iac_config_set（ui.theme）；
//! - ia-exit → 前端本地处理（结束当前会话/窗口）。

use serde::{Deserialize, Serialize};

/// 内置命令定义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinDef {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
}

/// 内置命令总表（`ia-help` 列全部；名单随能力域走查登记扩充）。
pub const BUILTINS: &[BuiltinDef] = &[
    BuiltinDef {
        name: "ia-help",
        summary: "列出全部内置命令",
        usage: "ia-help",
    },
    BuiltinDef {
        name: "ia-config",
        summary: "查看/修改配置",
        usage: "ia-config [get|set <键> <值>]",
    },
    BuiltinDef {
        name: "ia-model",
        summary: "模型后端列表与切换",
        usage: "ia-model [list|use <名>]",
    },
    BuiltinDef {
        name: "ia-daemon",
        summary: "daemon 生命周期管理",
        usage: "ia-daemon [status|stop]",
    },
    BuiltinDef {
        name: "ia-history",
        summary: "执行记录检索/清理/导出",
        usage: "ia-history [query <词>|clean|export <路径>]",
    },
    BuiltinDef {
        name: "ia-recipe",
        summary: "方案库保存/检索/删除/导出",
        usage: "ia-recipe [save|search <词>|show <名>|rm <名>|export <路径>]",
    },
    BuiltinDef {
        name: "ia-sessions",
        summary: "会话列表与恢复（Q46）",
        usage: "ia-sessions [list|resume <id>]",
    },
    BuiltinDef {
        name: "ia-rollback",
        summary: "回滚点列表与回放（Q49）",
        usage: "ia-rollback [list|replay [seq]]",
    },
    BuiltinDef {
        name: "ia-file-history",
        summary: "单文件版本历史与恢复（Q49）",
        usage: "ia-file-history <路径> [restore <seq>]",
    },
    BuiltinDef {
        name: "ia-script",
        summary: "脚本工坊制作/精修/运行",
        usage: "ia-script [make|refine <名>|run <名>]",
    },
    BuiltinDef {
        name: "ia-theme",
        summary: "主题切换",
        usage: "ia-theme [day|night|auto]",
    },
    BuiltinDef {
        name: "ia-exit",
        summary: "结束当前会话/窗口",
        usage: "ia-exit",
    },
];

/// 解析后的内置命令调用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinCall {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// 行首 `ia-<name>` 解析（Q30：内置命令与外部命令同一 Q15 分流，行首优先）。
/// 仅当首 token 以 `ia-` 开头且在总表内才算内置命令；不在表内交由 dispatch 报 IAC-1501。
pub fn parse(line: &str) -> Option<BuiltinCall> {
    let line = line.trim();
    let first = line.split_whitespace().next()?;
    if !first.starts_with("ia-") {
        return None;
    }
    let name = first.to_string();
    let args: Vec<String> = line
        .strip_prefix(first)
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    Some(BuiltinCall { name, args })
}

/// 查总表。
pub fn lookup(name: &str) -> Option<&'static BuiltinDef> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// `ia-help` 就地渲染（无需 daemon，Q30）。
pub fn help_text() -> String {
    let mut out = String::from("内置命令（ia- 命名空间）：\n");
    for b in BUILTINS {
        out.push_str(&format!(
            "  {:<18} {}\n    用法: {}\n",
            b.name, b.summary, b.usage
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_registry_names() {
        let call = parse("ia-rollback list --limit 5").unwrap();
        assert_eq!(call.name, "ia-rollback");
        assert_eq!(call.args, vec!["list", "--limit", "5"]);
        assert!(lookup("ia-rollback").is_some());
    }

    #[test]
    fn parse_rejects_non_ia_and_unknown_prefix() {
        assert!(parse("ls -la").is_none());
        assert!(parse("iam-not-a-builtin").is_none());
        // 不在总表的 ia-xxx 也返回调用体，由 dispatch 报 IAC-1501 并列全表
        assert!(parse("ia-future").is_some());
        assert!(lookup("ia-future").is_none());
    }

    #[test]
    fn help_lists_every_builtin() {
        let text = help_text();
        for b in BUILTINS {
            assert!(text.contains(b.name), "ia-help 缺少 {}", b.name);
        }
    }
}
