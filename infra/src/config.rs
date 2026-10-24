//! 路径与配置文件 IO（组合根用）：平台目录、socket、数据文件定位。

use std::path::PathBuf;

/// 平台数据目录（XDG；Windows 用 AppData）。
pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("iac")
}

/// 配置文件路径（config.toml）。
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("iac")
        .join("config.toml")
}

/// IPC socket 路径（UDS；Windows NamedPipe 随平台适配落地）。
pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join("ia-console.sock")
}

/// GUI 布局持久化文件（域 18 启动恢复）。
pub fn layout_path() -> PathBuf {
    data_dir().join("layout.json")
}
