//! 路径与配置文件 IO（组合根用）：平台目录、socket、数据文件定位、TOML 读写。
//! 数据文件名 iac.v2.db：重建 schema 与旧 iac.db 不兼容，物理隔离避免迁移冲突。

use std::path::{Path, PathBuf};

use iac_core::config::Config;
use iac_core::error::codes;
use iac_core::framework::{Context, Evolution, Metrics};

/// 平台数据目录（XDG；Windows 用 AppData）。
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("iac")
}

/// 平台缺省配置路径：`$XDG_CONFIG_HOME|~/.config /iac/config.toml`（Windows: %APPDATA%）。
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("iac").join("config.toml"))
}

/// 配置文件路径（config.toml）。
pub fn config_path() -> PathBuf {
    default_config_path().unwrap_or_else(|| PathBuf::from("iac/config.toml"))
}

/// IPC socket 路径（UDS；Windows NamedPipe 随平台适配落地）。
pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join("ia-console.sock")
}

/// 数据库文件（重建基线：iac.v2.db）。
pub fn db_path() -> PathBuf {
    data_dir().join("iac.v2.db")
}

/// 快照仓库根（域 15 底座）。
pub fn snapshots_dir() -> PathBuf {
    data_dir().join("snapshots")
}

/// 回收站根。
pub fn trash_dir() -> PathBuf {
    data_dir().join("trash")
}

/// GUI 布局持久化文件（域 18 启动恢复）。
pub fn layout_path() -> PathBuf {
    data_dir().join("layout.json")
}

/// 配置定位输入（path 缺省用平台路径）。
#[derive(Debug, Clone, Default)]
pub struct ConfigPath {
    pub path: Option<PathBuf>,
}

/// 写入确认。
#[derive(Debug, Clone, Default)]
pub struct ConfigWriteAck {
    pub path: PathBuf,
    pub bytes: usize,
}

/// IF-006：加载配置。文件缺失=首次运行给默认值；解析/校验失败经 evolution 返回 IAC-5101。
pub fn load_config(
    input: &ConfigPath,
    output: &mut Config,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(path) = input.path.clone().or_else(default_config_path) else {
        *output = Config::defaults();
        return true;
    };
    match read_file(&path) {
        Ok(None) => {
            *output = Config::defaults();
            metrics.counter("config.first_run", 1);
            true
        }
        Ok(Some(raw)) => parse_and_fill(&raw, output, metrics, evolution),
        Err(e) => {
            evolution.fail(codes::CONFIG_IO, format!("读取配置失败: {e}"));
            false
        }
    }
}

/// IF-007：保存配置到平台缺省路径（密钥永不入盘，仅存 api_key_env 变量名）。
pub fn save_config(input: &Config, output: &mut ConfigWriteAck, evolution: &mut Evolution) -> bool {
    let Some(path) = default_config_path() else {
        evolution.fail(codes::CONFIG_IO, "无法定位配置目录");
        return false;
    };
    save_config_to(&path, input, output, evolution)
}

/// 实际写盘实现（save_config 的路径解析包装；测试可用临时路径直调）。
pub fn save_config_to(
    path: &Path,
    input: &Config,
    output: &mut ConfigWriteAck,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if let Err(msg) = input.validate() {
        evolution.fail(codes::CONFIG_INVALID, msg);
        return false;
    }
    let body = match toml::to_string_pretty(input) {
        Ok(s) => s,
        Err(e) => {
            evolution.fail(codes::CONFIG_INVALID, format!("配置序列化失败: {e}"));
            return false;
        }
    };
    match write_atomic(path, &body) {
        Ok(bytes) => {
            output.path = path.to_path_buf();
            output.bytes = bytes;
            true
        }
        Err(e) => {
            evolution.fail(codes::CONFIG_WRITE, format!("配置写入失败: {e}"));
            false
        }
    }
}

/// 读文件：不存在视为首次运行（Ok(None)），其余 IO 错误上抛转译。
fn read_file(path: &Path) -> Result<Option<String>, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// 解析 + env 覆盖 + 校验 + 填充 output。
fn parse_and_fill(
    raw: &str,
    output: &mut Config,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    match toml::from_str::<Config>(raw) {
        Ok(mut c) => {
            c.apply_env();
            if let Err(msg) = c.validate() {
                evolution.fail(codes::CONFIG_INVALID, msg);
                return false;
            }
            *output = c;
            metrics.counter("config.loaded", 1);
            true
        }
        Err(e) => {
            evolution.fail(codes::CONFIG_INVALID, format!("配置解析失败: {e}"));
            false
        }
    }
}

/// 原子写：唯一临时文件 + fsync + 改名（fsutil::write_atomic）。
fn write_atomic(path: &Path, body: &str) -> std::io::Result<usize> {
    crate::fsutil::write_atomic(path, body.as_bytes())
}
