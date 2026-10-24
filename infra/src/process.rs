//! 进程执行适配（IF-003）：按平台 shell 解释执行。
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{ProcessPort, ProcessResult, ProcessSpec};

/// 进程适配器（实现由重建阶段填充）。
pub struct ProcessAdapter;

impl ProcessPort for ProcessAdapter {
    fn spawn_process(
        &self,
        _input: &ProcessSpec,
        _output: &mut ProcessResult,
        _context: &Context,
        _metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        evolution.fail("IAC-2106", "进程适配重建中（未接线）");
        false
    }
}
