//! I-035 capture_pre_image（orchestration）：执行前统一捕获前像 → manifest 入快照仓库。

use super::{resolve_roots, CaptureScope, ManifestDoc};
use crate::types::ManifestRef;
use crate::framework::{Context, Evolution, Metrics};
use crate::ports::{CaptureTreeInput, SnapshotData};

/// 执行前捕获前像：按配置圈定范围 → 指纹+内容入快照仓库 → manifest 落库取引用。
/// 回滚关闭/范围为空时输出空 manifest_ref（调用方据此跳过 diff）。
pub fn capture_pre_image(
    input: &CaptureScope,
    output: &mut ManifestRef,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if !context.config.rollback.enabled {
        return true;
    }
    let mut roots = resolve_roots(input, context);
    roots = roots
        .into_iter()
        .map(|r| std::path::PathBuf::from(shellexpand(&r)))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    if roots.is_empty() {
        return true;
    }
    let mut files = Vec::new();
    let tree_in = CaptureTreeInput {
        roots: roots.iter().map(std::path::PathBuf::from).collect(),
        excludes: super::CAPTURE_EXCLUDES.iter().map(|s| s.to_string()).collect(),
        max_file_bytes: super::CAPTURE_MAX_FILE_BYTES,
        max_total_bytes: super::CAPTURE_MAX_TOTAL_BYTES,
    };
    if !context.deps.snap.capture_tree(&tree_in, &mut files, context, metrics, evolution) {
        evolution.propagate("capture_pre_image", "目录捕获失败");
        return false;
    }
    let doc = ManifestDoc {
        cwd: input.cwd.clone(),
        roots,
        files,
    };
    let body = match serde_json::to_vec(&doc) {
        Ok(b) => b,
        Err(e) => {
            evolution.fail("IAC-1601", format!("manifest 序列化失败: {e}"));
            return false;
        }
    };
    let mut cref = crate::ports::ContentRef::default();
    if !context
        .deps
        .snap
        .snapstore_put(&SnapshotData { bytes: body }, &mut cref, context, metrics, evolution)
    {
        evolution.propagate("capture_pre_image", "manifest 入库失败");
        return false;
    }
    metrics.counter("rollback.capture", 1);
    *output = ManifestRef {
        execution_id: input.execution_hint.clone(),
        manifest_ref: cref.reference,
        journal_ref: String::new(),
    };
    true
}

/// 展开 `~` 前缀（捕获根允许 ~ 写法）。
fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::TestEnv;

    #[test]
    fn disabled_means_empty_manifest() {
        let mut env = TestEnv::new();
        env.config.rollback.enabled = false;
        let mut out = ManifestRef::default();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(capture_pre_image(
            &CaptureScope { execution_hint: "e1".into(), cwd: "/tmp".into(), ..Default::default() },
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert_eq!(out.manifest_ref, "");
    }

    #[test]
    fn missing_roots_mean_empty_manifest() {
        let env = TestEnv::new();
        let mut out = ManifestRef::default();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(capture_pre_image(
            &CaptureScope { execution_hint: "e2".into(), cwd: "/nonexistent-iac-dir".into(), ..Default::default() },
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ));
        assert_eq!(out.manifest_ref, "");
    }
}
