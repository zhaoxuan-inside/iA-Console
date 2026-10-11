//! 内置风险规则集（T-012）：官方常见危险命令的确定性匹配。
//! 覆盖为子串匹配，note 面向弱提醒展示；自定义规则目录为 v1.1 评估项（_01 未决）。

use super::RiskRule;
use crate::types::RiskLevel::{self as Lv, Caution, Dangerous, Destructive};

/// 内置规则表（元组：id/pattern/level/note）。
const BUILTIN: &[(&str, &str, Lv, &str)] = &[
    // ---- destructive：不可逆破坏 ----
    ("rm-rf-root", "rm -rf /", Destructive, "递归强删根路径，可致系统不可用"),
    ("rm-rf-home", "rm -rf ~", Destructive, "递归强删用户主目录"),
    ("mkfs", "mkfs", Destructive, "格式化文件系统，盘上数据将清空"),
    ("dd-device", "dd of=/dev/", Destructive, "直接写入块设备，覆盖原始数据"),
    ("wipefs", "wipefs", Destructive, "擦除文件系统签名"),
    ("fdisk", "fdisk", Destructive, "磁盘分区操作，可能破坏分区表"),
    ("shred-device", "shred /dev/", Destructive, "反复覆写块设备"),
    // ---- dangerous：可致损失，需确认（翻译/Agent 路径） ----
    ("rm-rf", "rm -rf", Dangerous, "递归强制删除，误删难恢复（v2 起有回滚日志）"),
    ("rm-fr", "rm -fr", Dangerous, "递归强制删除（参数序变体）"),
    ("git-force-push", "git push --force", Dangerous, "强推远端，覆盖他人提交"),
    ("git-reset-hard", "git reset --hard", Dangerous, "丢弃工作区与暂存区改动"),
    ("pipe-shell", "| sh", Dangerous, "网络脚本直接进 shell，来源不可信"),
    ("pipe-bash", "| bash", Dangerous, "网络脚本直接进 shell，来源不可信"),
    ("chmod-777", "chmod 777", Dangerous, "完全开放权限，安全风险"),
    ("docker-prune", "docker system prune", Dangerous, "批量清理容器/镜像/卷"),
    ("npm-publish", "npm publish", Dangerous, "对外发布包，公开不可撤回"),
    ("cargo-publish", "cargo publish", Dangerous, "对外发布 crate，公开不可撤回"),
    ("drop-database", "DROP DATABASE", Dangerous, "删除整个数据库（SQL）"),
    ("truncate-table", "TRUNCATE TABLE", Dangerous, "清空表数据（SQL）"),
    // ---- caution：需留意 ----
    ("sudo", "sudo", Caution, "提权执行，影响系统范围"),
    ("systemctl", "systemctl", Caution, "系统服务管理"),
    ("kill", "kill ", Caution, "终止进程"),
    ("pkill", "pkill", Caution, "按名终止进程"),
    ("git-push", "git push", Caution, "推送远端仓库"),
    ("docker-rm", "docker rm", Caution, "删除容器"),
    ("pkg-remove", " uninstall", Caution, "卸载软件包"),
    ("pkg-install", " install ", Caution, "安装软件包，变更系统状态"),
    ("chmod", "chmod", Caution, "变更文件权限"),
    ("chown", "chown", Caution, "变更文件属主"),
    ("crontab-r", "crontab -r", Caution, "删除全部定时任务"),
    ("write-etc", "> /etc/", Caution, "覆写系统配置目录文件"),
];

/// 构建内置规则集（每次调用克隆；规则为静态资产，运行时装配一次即可）。
pub fn builtin_rules() -> Vec<RiskRule> {
    BUILTIN
        .iter()
        .map(|(id, pattern, level, note)| RiskRule {
            id: (*id).into(),
            pattern: (*pattern).into(),
            level: *level,
            note: (*note).into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_cover_three_levels() {
        let rules = builtin_rules();
        let destructive = rules.iter().filter(|r| r.level == Destructive).count();
        let dangerous = rules.iter().filter(|r| r.level == Dangerous).count();
        let caution = rules.iter().filter(|r| r.level == Caution).count();
        assert!(destructive >= 5 && dangerous >= 8 && caution >= 8);
        assert!(rules.len() >= 20);
    }

    #[test]
    fn rule_ids_are_unique() {
        let mut ids: Vec<String> = builtin_rules().into_iter().map(|r| r.id).collect();
        ids.sort();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "规则 id 必须唯一");
    }
}
