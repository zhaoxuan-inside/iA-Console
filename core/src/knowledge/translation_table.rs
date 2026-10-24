//! 内置静态翻译映射表（T-025）：linux↔windows 高频命令 + 参数级四类映射。
//! macos 与 linux 同属 POSIX，差异条目单独列出。数据为 MVP 核心集（AC-19/23）。

use crate::knowledge::ParamMapKind;

/// 参数映射元组：(源参数, 目标参数, 类别, 说明)。运行时转换为 ParamMapping。
pub type ParamTuple = (
    &'static str,
    Option<&'static str>,
    ParamMapKind,
    &'static str,
);

/// 一条内置映射：方向（源方言→目标平台）、源命令（首词）、目标命令、参数映射、差异说明。
pub struct BuiltinEntry {
    pub from: &'static str,
    pub to: &'static str,
    pub command_from: &'static str,
    pub command_to: &'static str,
    pub params: &'static [ParamTuple],
    pub notes: &'static str,
}

/// 内置条目（linux/macos → windows 为主，windows → linux 反向一组）。
pub const BUILTIN: &[BuiltinEntry] = &[
    // ---- linux → macos（POSIX 亲缘，补齐旧版承诺的 macos 条目） ----
    BuiltinEntry {
        from: "linux",
        to: "macos",
        command_from: "apt-get",
        command_to: "brew",
        params: &[
            (
                "install",
                Some("install"),
                ParamMapKind::Direct,
                "安装语义一致",
            ),
            (
                "update",
                None,
                ParamMapKind::Renamed,
                "brew 对应 update/upgrade 两步",
            ),
        ],
        notes: "apt-get → Homebrew；apt-get install → brew install",
    },
    BuiltinEntry {
        from: "linux",
        to: "macos",
        command_from: "systemctl",
        command_to: "launchctl",
        params: &[
            (
                "start",
                Some("bootstrap"),
                ParamMapKind::Renamed,
                "launchctl bootstrap gui/$(id -u) <plist>",
            ),
            (
                "stop",
                Some("bootout"),
                ParamMapKind::Renamed,
                "launchctl bootout gui/$(id -u) <plist>",
            ),
        ],
        notes: "systemd → launchd；服务单元为 plist 文件",
    },
    BuiltinEntry {
        from: "linux",
        to: "macos",
        command_from: "ip",
        command_to: "ifconfig",
        params: &[("addr", None, ParamMapKind::None, "ifconfig 直接列接口")],
        notes: "ip(8) 为 Linux 专有，macOS 用 ifconfig",
    },
    BuiltinEntry {
        from: "linux",
        to: "macos",
        command_from: "ss",
        command_to: "lsof",
        params: &[(
            "-tlnp",
            Some("-i -P -n"),
            ParamMapKind::Renamed,
            "lsof -i -P -n 列监听端口",
        )],
        notes: "ss → lsof/netstat；macOS 无 ss",
    },
    BuiltinEntry {
        from: "macos",
        to: "linux",
        command_from: "brew",
        command_to: "apt-get",
        params: &[(
            "install",
            Some("install"),
            ParamMapKind::Direct,
            "安装语义一致",
        )],
        notes: "Homebrew → apt-get（Debian 系）",
    },
    BuiltinEntry {
        from: "macos",
        to: "linux",
        command_from: "open",
        command_to: "xdg-open",
        params: &[],
        notes: "macOS open → xdg-open",
    },
    BuiltinEntry {
        from: "macos",
        to: "linux",
        command_from: "pbcopy",
        command_to: "xclip",
        params: &[(
            "-selection clipboard",
            None,
            ParamMapKind::ValueTransform,
            "xclip 需显式 selection",
        )],
        notes: "pbcopy → xclip -selection clipboard",
    },
    // ---- windows → macos ----
    BuiltinEntry {
        from: "windows",
        to: "macos",
        command_from: "ipconfig",
        command_to: "ifconfig",
        params: &[(
            "/all",
            None,
            ParamMapKind::ValueTransform,
            "ifconfig -a 展示全部",
        )],
        notes: "Windows ipconfig → ifconfig",
    },
    BuiltinEntry {
        from: "windows",
        to: "macos",
        command_from: "tasklist",
        command_to: "ps",
        params: &[(
            "/v",
            Some("aux"),
            ParamMapKind::Renamed,
            "ps aux 展示全部进程详情",
        )],
        notes: "tasklist → ps",
    },
    // ---- linux → windows ----
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "ls",
        command_to: "Get-ChildItem",
        params: &[
            (
                "-l",
                None,
                ParamMapKind::ValueTransform,
                "PowerShell 默认即详细列表，-l 无需转换",
            ),
            (
                "-a",
                Some("-Force"),
                ParamMapKind::Renamed,
                "含隐藏项改用 -Force",
            ),
            ("-h", None, ParamMapKind::None, "人类可读尺寸需手动格式化"),
        ],
        notes: "Get-ChildItem 列出目录内容；隐藏项需 -Force",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "cat",
        command_to: "Get-Content",
        params: &[(
            "-n",
            None,
            ParamMapKind::ValueTransform,
            "行号需配合 Select-Object 实现了",
        )],
        notes: "读文件内容",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "rm",
        command_to: "Remove-Item",
        params: &[
            ("-r", Some("-Recurse"), ParamMapKind::Renamed, "递归删除"),
            ("-f", Some("-Force"), ParamMapKind::Renamed, "强制删除"),
        ],
        notes: "删除文件/目录；递归+强制需 -Recurse -Force",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "cp",
        command_to: "Copy-Item",
        params: &[("-r", Some("-Recurse"), ParamMapKind::Renamed, "递归复制")],
        notes: "复制文件/目录",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "mv",
        command_to: "Move-Item",
        params: &[],
        notes: "移动/重命名",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "pwd",
        command_to: "Get-Location",
        params: &[],
        notes: "显示当前目录",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "ps",
        command_to: "Get-Process",
        params: &[(
            "aux",
            None,
            ParamMapKind::None,
            "PowerShell 无 aux 语义，进程属性经 Format-List",
        )],
        notes: "列出进程",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "grep",
        command_to: "Select-String",
        params: &[
            (
                "-i",
                Some("-CaseInsensitive"),
                ParamMapKind::Renamed,
                "忽略大小写",
            ),
            (
                "-v",
                None,
                ParamMapKind::None,
                "反向匹配需 Where-Object 组合",
            ),
        ],
        notes: "文本搜索",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "which",
        command_to: "Get-Command",
        params: &[],
        notes: "定位命令",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "man",
        command_to: "Get-Help",
        params: &[],
        notes: "查看帮助",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "kill",
        command_to: "Stop-Process",
        params: &[("-9", Some("-Force"), ParamMapKind::Renamed, "强制终止")],
        notes: "终止进程",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "touch",
        command_to: "New-Item -ItemType File",
        params: &[],
        notes: "创建空文件（已存在仅更新时间的语义不同）",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "clear",
        command_to: "Clear-Host",
        params: &[],
        notes: "清屏",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "curl",
        command_to: "Invoke-WebRequest",
        params: &[(
            "-O",
            None,
            ParamMapKind::ValueTransform,
            "下载用 -OutFile <文件名>",
        )],
        notes: "HTTP 请求",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "head",
        command_to: "Get-Content -TotalCount",
        params: &[(
            "-n",
            None,
            ParamMapKind::ValueTransform,
            "-n N 的 N 直接给 -TotalCount",
        )],
        notes: "取文件头部",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "tail",
        command_to: "Get-Content -Tail",
        params: &[(
            "-f",
            Some("-Wait"),
            ParamMapKind::Renamed,
            "跟随文件用 -Wait",
        )],
        notes: "取文件尾部",
    },
    BuiltinEntry {
        from: "linux",
        to: "windows",
        command_from: "mkdir",
        command_to: "New-Item -ItemType Directory",
        params: &[(
            "-p",
            None,
            ParamMapKind::ValueTransform,
            "PowerShell 自动创建父目录",
        )],
        notes: "创建目录",
    },
    // ---- windows → linux ----
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Get-ChildItem",
        command_to: "ls",
        params: &[("-Force", Some("-a"), ParamMapKind::Renamed, "含隐藏项")],
        notes: "列出目录内容",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Get-Content",
        command_to: "cat",
        params: &[],
        notes: "读文件内容",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Remove-Item",
        command_to: "rm",
        params: &[
            ("-Recurse", Some("-r"), ParamMapKind::Renamed, "递归删除"),
            ("-Force", Some("-f"), ParamMapKind::Renamed, "强制删除"),
        ],
        notes: "删除文件/目录",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Copy-Item",
        command_to: "cp",
        params: &[],
        notes: "复制",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Move-Item",
        command_to: "mv",
        params: &[],
        notes: "移动/重命名",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Get-Process",
        command_to: "ps",
        params: &[],
        notes: "列出进程",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Select-String",
        command_to: "grep",
        params: &[(
            "-CaseInsensitive",
            Some("-i"),
            ParamMapKind::Renamed,
            "忽略大小写",
        )],
        notes: "文本搜索",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Stop-Process",
        command_to: "kill",
        params: &[],
        notes: "终止进程",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Get-Help",
        command_to: "man",
        params: &[],
        notes: "查看帮助",
    },
    BuiltinEntry {
        from: "windows",
        to: "linux",
        command_from: "Clear-Host",
        command_to: "clear",
        params: &[],
        notes: "清屏",
    },
];

/// 按方向与源命令首词查找内置条目。
pub fn lookup(from: &str, to: &str, command_from: &str) -> Option<&'static BuiltinEntry> {
    let first = command_from.split_whitespace().next()?;
    BUILTIN
        .iter()
        .find(|e| e.from == from && e.to == to && e.command_from == first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_entries_both_directions() {
        assert!(lookup("linux", "windows", "ls").is_some());
        assert!(lookup("windows", "linux", "Get-ChildItem").is_some());
        assert!(lookup("linux", "windows", "不存在的命令").is_none());
        assert!(BUILTIN.len() >= 20, "内置核心集 ≥20 条");
    }

    #[test]
    fn param_mappings_use_four_kinds() {
        let ls = lookup("linux", "windows", "ls").unwrap();
        assert!(ls.params.iter().any(|p| p.2 == ParamMapKind::Renamed));
        assert!(ls.params.iter().any(|p| p.2 == ParamMapKind::None));
    }
}
