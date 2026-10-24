use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_SHELL: &str = "/bin/bash";

#[derive(Debug, Clone)]
pub struct BackendSettings {
    pub shell: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// 额外环境变量（透传 alacritty_terminal tty::Options.env；TERM/COLORTERM 等）。
    pub env: HashMap<String, String>,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            shell: DEFAULT_SHELL.to_string(),
            args: vec![],
            working_directory: None,
            env: HashMap::new(),
        }
    }
}
