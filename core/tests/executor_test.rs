//! T-017/T-018 执行与诊断测试：弱提醒/确认闸门/记录三账/失败诊断/静态通道。

use core::executor::{handle_execute, CommandOrigin, ExecuteInput};
use core::framework::{Actor, Context, Evolution, Metrics, TestEnv};
use core::ports::{MigrationAck, MigrationDir, ProcessResult, ProcessSpec};
use infra::storage::Storage;

/// 真实进程 Adapter（跑 echo/exit 等，M1 已验证）。
struct RealProc;
impl core::ports::ProcessPort for RealProc {
    fn spawn_process(
        &self,
        input: &ProcessSpec,
        output: &mut ProcessResult,
        ctx: &Context,
        m: &mut Metrics,
        e: &mut Evolution,
    ) -> bool {
        infra::process::spawn_process(input, output, ctx, m, e)
    }
}

fn env_with(actor: Actor) -> (TestEnv, String) {
    let store = Storage::open_in_memory().expect("db");
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    let env = TestEnv::new().with_store(Box::new(store)).with_process(Box::new(RealProc));
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    // 建真实会话供 executions.session_id 外键引用
    let mut session = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(core::session::create_session(&core::session::NewSessionInput { title: None, cwd: ".".into(), shell: "sh".into(), host: "local".into() }, &mut session, &env.context(), &mut m, &mut evo));
    (actor_env(env, actor), session.id)
}

fn actor_env(mut env: TestEnv, actor: Actor) -> TestEnv {
    env.session.actor = actor;
    env
}

fn direct(command: &str) -> ExecuteInput {
    ExecuteInput { command: Some(command.into()), origin: Some(CommandOrigin::Direct), ..Default::default() }
}

#[test]
fn human_direct_dangerous_is_weak_warning_not_blocked() {
    let (env, sid) = env_with(Actor::Human);
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None,
        gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = ExecuteInput { command: Some("echo test".into()), session_id: Some(sid.clone()), origin: Some(CommandOrigin::Direct), ..Default::default() };
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo), "evo={evo:?}");
    assert_eq!(out.status, core::history::ExecStatus::Ok);
    // 弱提醒标记位：safe 无警告
    assert!(!out.gate.weak_warning);
    assert!(!out.gate.requires_confirm);
    // 三账已落
    let mut rows = core::ports::QueryRows::default();
    let q = core::ports::SqlQuery { sql: "SELECT COUNT(*) AS n FROM executions".into(), params: vec![] };
    assert!(env.deps.store.storage_query(&q, &mut rows, &env.context(), &mut m, &mut evo));
    assert_eq!(rows.int(0, "n"), Some(1));
}

#[test]
fn human_direct_dangerous_executes_with_weak_warning() {
    // 直接输入高危：不拦截（AC-04），gate.weak_warning=true，正常执行
    let (env, sid) = env_with(Actor::Human);
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = ExecuteInput { command: Some("echo 'git push --force'".into()), session_id: Some(sid.clone()), origin: Some(CommandOrigin::Direct), ..Default::default() };
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert!(out.gate.weak_warning, "dangerous 命中应有弱提醒标记");
    assert!(!out.gate.requires_confirm, "人类直输不拦截");
    assert_eq!(out.status, core::history::ExecStatus::Ok);
}

#[test]
fn agent_dangerous_without_confirm_is_rejected() {
    let (env, sid) = env_with(Actor::Agent("zcode".into()));
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    // Agent 发起 dangerous：命令为 echo（闸门按内容判定 dangerous）
    let input = ExecuteInput {
        command: Some("echo 'rm -rf x'".into()),
        session_id: Some(sid.clone()),
        origin: Some(CommandOrigin::Direct),
        confirm_dangerous: false,
        ..Default::default()
    };
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.status, core::history::ExecStatus::Rejected);
    assert!(out.gate.requires_confirm);
    // 记录落账（rejected 也是一次留痕，AC-05）
    let mut rows = core::ports::QueryRows::default();
    let q = core::ports::SqlQuery { sql: "SELECT status FROM executions".into(), params: vec![] };
    assert!(env.deps.store.storage_query(&q, &mut rows, &env.context(), &mut m, &mut evo));
    assert_eq!(rows.text(0, "status"), Some("rejected"));
}

#[test]
fn failed_command_triggers_diagnosis_with_static_channel() {
    let (env, sid) = env_with(Actor::Human);
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    // 127：命令不存在 → 静态通道命中
    let mut input = direct("definitely_missing_cmd_xyz");
    input.session_id = Some(sid.clone());
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.status, core::history::ExecStatus::Failed);
    let d = out.diagnosis.expect("失败应附诊断");
    assert!(d.root_cause.contains("命令未找到"));
    assert!(!d.speculation);
    assert!(d.rollback_note.contains("无自动回滚"));
}

#[test]
fn translated_dangerous_requires_confirmation() {
    let (env, sid) = env_with(Actor::Human);
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = ExecuteInput {
        command: Some("echo 'rm -rf x'".into()),
        session_id: Some(sid.clone()),
        origin: Some(CommandOrigin::Translated),
        confirm_dangerous: false,
        ..Default::default()
    };
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.status, core::history::ExecStatus::Rejected, "翻译高危未确认应拒绝");
    // 携带确认重试 → 执行
    let mut out2 = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let input2 = ExecuteInput { command: Some("echo ok".into()), session_id: Some(sid.clone()), origin: Some(CommandOrigin::Translated), confirm_dangerous: true, ..Default::default() };
    assert!(handle_execute(&input2, &mut out2, &env.context(), &mut m, &mut evo));
    assert_eq!(out2.status, core::history::ExecStatus::Ok);
}

#[test]
fn timeout_records_timeout_status() {
    let (env, sid) = env_with(Actor::Human);
    let mut out = core::executor::ExecutionOutcome {
        execution_id: String::new(), exit_code: None, status: core::history::ExecStatus::Ok,
        stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None, gate: Default::default(),
    };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = ExecuteInput {
        command: Some("sleep 3".into()),
        session_id: Some(sid.clone()),
        timeout_ms: Some(200),
        origin: Some(CommandOrigin::Direct),
        ..Default::default()
    };
    assert!(handle_execute(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.status, core::history::ExecStatus::Timeout);
    assert_eq!(out.exit_code, None);
}
