//! 受控干净 shell（Q52）：GUI 终端标签与宿主 dotfiles 完全解耦。
//! PTY 仍拉起系统 shell（真终端内核，域 18 不变），但禁读用户 rc——
//! zsh 走 `ZDOTDIR`、bash 走 `--rcfile`、fish 走 `--no-config`；
//! 环境变量照常继承（PATH 等不受影响），提示符 = iA 自有极简两行。

use std::path::{Path, PathBuf};

/// PTY 拉起参数（BackendSettings 直填）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlledShell {
    pub shell: String,
    pub args: Vec<String>,
    /// 追加环境变量（叠加在 TERM/COLORTERM 之上）。
    pub env: Vec<(String, String)>,
}

/// 依据 shell 路径给出受控启动参数；未知 shell 原样透传（旧行为兜底）。
pub fn controlled(shell: &str) -> ControlledShell {
    let dir = rc_dir();
    let name = Path::new(shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match name {
        "zsh" => ControlledShell {
            shell: shell.to_owned(),
            args: vec![],
            env: vec![("ZDOTDIR".to_owned(), dir.to_string_lossy().into_owned())],
        },
        "bash" => ControlledShell {
            shell: shell.to_owned(),
            args: vec![
                "--rcfile".to_owned(),
                dir.join("bashrc").to_string_lossy().into_owned(),
            ],
            env: vec![],
        },
        "fish" => ControlledShell {
            shell: shell.to_owned(),
            args: vec!["--no-config".to_owned()],
            env: vec![],
        },
        _ => ControlledShell {
            shell: shell.to_owned(),
            args: vec![],
            env: vec![],
        },
    }
}

/// 受控 rc 目录：`<data_dir>/shell`（XDG 数据目录下，与布局/快照同源）。
pub fn rc_dir() -> PathBuf {
    infra::config::data_dir().join("shell")
}

/// 幂等落盘受控 rc（内嵌资源随包升级覆盖写入）。
pub fn ensure_rc_files(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(".zshrc"), include_str!("shell_assets/zshrc"))?;
    std::fs::write(dir.join("bashrc"), include_str!("shell_assets/bashrc"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_routes_via_zdotdir() {
        let c = controlled("/bin/zsh");
        assert_eq!(c.args, Vec::<String>::new());
        assert_eq!(
            c.env,
            vec![(
                "ZDOTDIR".to_owned(),
                rc_dir().to_string_lossy().into_owned()
            )]
        );
    }

    #[test]
    fn bash_routes_via_rcfile() {
        let c = controlled("/usr/bin/bash");
        assert!(c.env.is_empty());
        assert_eq!(c.args[0], "--rcfile");
        assert!(c.args[1].ends_with("bashrc"));
    }

    #[test]
    fn fish_gets_no_config() {
        assert_eq!(
            controlled("/usr/bin/fish").args,
            vec!["--no-config".to_owned()]
        );
    }

    #[test]
    fn unknown_shell_passes_through() {
        let c = controlled("/bin/sh");
        assert!(c.args.is_empty() && c.env.is_empty());
    }

    #[test]
    fn rc_files_are_written_idempotently() {
        let dir = std::env::temp_dir().join(format!("iac-shell-test-{}", std::process::id()));
        ensure_rc_files(&dir).expect("首次写入");
        ensure_rc_files(&dir).expect("重复写入幂等");
        assert!(dir.join(".zshrc").is_file());
        assert!(dir.join("bashrc").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
