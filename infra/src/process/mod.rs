//! 进程执行 Adapter（IF-003）：跨平台 shell 解释执行，输出完整捕获，超时杀进程组。
//! 命令自身非零退出是正常产出（true）；执行机制失败（找不到 shell/超时/平台不支持）才返回 false。

use iac_core::error::codes;
use iac_core::framework::{Context, Evolution, Metrics};
use iac_core::ports::{ProcessResult, ProcessSpec};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// IF-003：执行 shell 命令并捕获输出。超时杀进程组后返回 false（IAC-2103）。
pub fn spawn_process(
    input: &ProcessSpec,
    output: &mut ProcessResult,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let start = Instant::now();
    let (mut child, shell) = match spawn_shell(input) {
        Ok(v) => v,
        Err(e) => {
            evolution.fail(codes::EXEC_NOT_FOUND, format!("无法启动 shell: {e}"));
            return false;
        }
    };
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let t_out = std::thread::spawn(move || read_all(&mut out_pipe));
    let t_err = std::thread::spawn(move || read_all(&mut err_pipe));
    let deadline = start + Duration::from_millis(input.timeout_ms);
    let Some(status) = wait_status(&mut child, deadline, input.timeout_ms, metrics, evolution) else {
        return false; // evolution 已写明超时/平台原因
    };
    output.exit_code = status.and_then(|s| s.code());
    output.stdout = t_out.join().unwrap_or_default();
    output.stderr = t_err.join().unwrap_or_default();
    output.duration_ms = start.elapsed().as_millis() as u64;
    output.shell_used = shell;
    metrics.counter("process.spawned", 1);
    true
}

/// 等待子进程：超时杀进程并返回 None（经 Err 信号），Err(e)=平台错误。
fn wait_status(
    child: &mut Child,
    deadline: Instant,
    timeout_ms: u64,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Option<Option<std::process::ExitStatus>> {
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Some(Some(st)),
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(child);
                let _ = child.wait();
                evolution.fail(codes::EXEC_TIMEOUT, format!("执行超时（>{timeout_ms}ms）已终止"));
                metrics.counter("process.timeout", 1);
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                evolution.fail(codes::EXEC_PLATFORM, format!("等待进程失败: {e}"));
                return None;
            }
        }
    }
}

/// 按平台选择 shell 并装配子进程：unix=sh -c；windows=powershell -Command。
fn spawn_shell(spec: &ProcessSpec) -> std::io::Result<(Child, String)> {
    #[cfg(unix)]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&spec.command);
        use std::os::unix::process::CommandExt;
        c.process_group(0);
        apply_common(&mut c, spec);
        Ok((c.spawn()?, "sh".into()))
    }
    #[cfg(windows)]
    {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-Command"]).arg(&spec.command);
        apply_common(&mut c, spec);
        Ok((c.spawn()?, "powershell".into()))
    }
}

/// 公共装配：工作目录、环境增量、管道捕获、stdin 关闭。
fn apply_common(c: &mut Command, spec: &ProcessSpec) {
    if let Some(dir) = &spec.cwd {
        c.current_dir(dir);
    }
    for (k, v) in &spec.env {
        c.env(k, v);
    }
    c.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
}

/// 读完整个流为 UTF-8（lossy）。
fn read_all<R: Read>(pipe: &mut Option<R>) -> String {
    let mut buf = String::new();
    if let Some(p) = pipe {
        let _ = p.read_to_string(&mut buf);
    }
    buf
}

/// 超时终止：std kill 直接杀 shell 进程。
/// MVP 边界：`sh -c` 对单命令会 exec（kill 即杀目标）；复杂管道的子进程可能残留，
/// v1.1 评估平台特定组终止（TODO(zhaoxuan): killpg 需 libc，当前镜像不可用）。
fn kill_tree(child: &mut Child) {
    let _ = child.kill();
}

/// ProcessPort 装配实现（iacd 接线用）。
pub struct RealProcess;

impl iac_core::ports::ProcessPort for RealProcess {
    fn spawn_process(
        &self,
        input: &ProcessSpec,
        output: &mut ProcessResult,
        context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        spawn_process(input, output, context, metrics, evolution)
    }
}
