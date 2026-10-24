//! 布局持久化（域 18 启动恢复，Q41）：windows×tabs×cwd×session_id，退出与变更时原子写。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 顶层布局。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// day | night | auto
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub windows: Vec<WindowLayout>,
}

fn default_theme() -> String {
    "auto".into()
}

/// 单窗口布局。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowLayout {
    #[serde(default)]
    pub bounds: [f32; 4],
    #[serde(default)]
    pub tabs: Vec<TabLayout>,
}

/// 单标签布局（真终端会话现场）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TabLayout {
    #[serde(default)]
    pub title: String,
    /// 已知 cwd（输入行 cd 会更新；PTY 内部 cd 依赖 OSC7，未跟踪项保持初值）。
    #[serde(default)]
    pub cwd: String,
    /// daemon 会话 id（恢复绑定，Q46）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// 布局文件路径（infra::config::layout_path）。
pub fn layout_path() -> PathBuf {
    infra::config::layout_path()
}

/// 加载布局；缺失/损坏 → None（首次运行）。
pub fn load() -> Option<Layout> {
    let raw = std::fs::read_to_string(layout_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 原子保存布局（临时文件 + 改名）。
pub fn save(layout: &Layout) -> Result<(), String> {
    let path = layout_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let body = serde_json::to_string_pretty(layout).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("写入失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("改名失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_roundtrip() {
        let layout = Layout {
            theme: "night".into(),
            windows: vec![WindowLayout {
                bounds: [10.0, 20.0, 1280.0, 820.0],
                tabs: vec![
                    TabLayout {
                        title: "sh".into(),
                        cwd: "/tmp".into(),
                        session_id: Some("s1".into()),
                    },
                    TabLayout {
                        title: "build".into(),
                        cwd: "/opt".into(),
                        session_id: None,
                    },
                ],
            }],
        };
        let json = serde_json::to_string(&layout).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(back, layout);
        assert_eq!(back.windows[0].tabs[0].session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn empty_layout_defaults() {
        let back: Layout = serde_json::from_str("{}").unwrap();
        assert_eq!(back.theme, "auto");
        assert!(back.windows.is_empty());
    }
}
