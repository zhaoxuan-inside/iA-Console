//! 快照仓库（域 15 底座）：内容寻址 + gzip + 去重 + 回收站 + retention。
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{
    CaptureTreeInput, ContentRef, FileCaptured, RetentionPolicy, SnapshotData, SnapPort,
    SweepReport, TrashAck, TrashRequest, WriteFileInput,
};

/// 快照仓库句柄（SnapPort 实现由重建阶段填充）。
pub struct SnapStore {
    _root: std::path::PathBuf,
}

impl SnapStore {
    pub fn open(root: &std::path::Path) -> Self {
        Self { _root: root.to_path_buf() }
    }
}

impl SnapPort for SnapStore {
    fn snapstore_put(
        &self,
        _input: &SnapshotData,
        _output: &mut ContentRef,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库重建中（snapstore_put 未接线）");
        false
    }

    fn snapstore_fetch(
        &self,
        _input: &ContentRef,
        _output: &mut SnapshotData,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库重建中（snapstore_fetch 未接线）");
        false
    }

    fn sweep_expired(
        &self,
        _input: &RetentionPolicy,
        _output: &mut SweepReport,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1601", "快照仓库重建中（sweep_expired 未接线）");
        false
    }

    fn capture_tree(
        &self,
        _input: &CaptureTreeInput,
        _output: &mut Vec<FileCaptured>,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库重建中（capture_tree 未接线）");
        false
    }

    fn trash_file(
        &self,
        _input: &TrashRequest,
        _output: &mut TrashAck,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库重建中（trash_file 未接线）");
        false
    }

    fn write_file(
        &self,
        _input: &WriteFileInput,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-1602", "快照仓库重建中（write_file 未接线）");
        false
    }

    fn data_root(&self) -> std::path::PathBuf {
        self._root.clone()
    }
}
