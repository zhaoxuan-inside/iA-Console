//! T-020 dispatch 测试：路由正确性、未知方法、版本拦截、错误码进信封、人机同构。

use iac_core::framework::{Actor, Context, Evolution, Metrics, TestEnv};
use iac_core::ports::{MigrationAck, MigrationDir};
use iac_core::server::{dispatch_request, IpcRequest, IpcResponse};
use infra::storage::Storage;

/// 真实进程 Adapter（执行回显用）。
struct RealProc;
impl iac_core::ports::ProcessPort for RealProc {
    fn spawn_process(
        &self,
        input: &iac_core::ports::ProcessSpec,
        output: &mut iac_core::ports::ProcessResult,
        ctx: &Context,
        m: &mut Metrics,
        e: &mut Evolution,
    ) -> bool {
        infra::process::spawn_process(input, output, ctx, m, e)
    }
}

fn env_with(actor: Actor) -> TestEnv {
    // (函数体内 env 声明加 mut)
    let store = Storage::open_in_memory().expect("db");
    let mut env = TestEnv::new().with_store(Box::new(store)).with_process(Box::new(RealProc));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    // 模拟 daemon 行为：连接建立时先确保当前会话，并把真实 session_id 写回 SessionCtx
    let mut session = iac_core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(iac_core::session::get_or_create_current(&iac_core::session::CurrentSessionInput { cwd: ".".into(), shell: "sh".into(), host: "local".into() }, &mut session, &env.context(), &mut m, &mut evo));
    env.session.session_id = session.id;
    actor_session(env, actor)
}

fn actor_session(mut env: TestEnv, actor: Actor) -> TestEnv {
    env.session.actor = actor;
    env
}

fn call(env: &TestEnv, method: &str, params: serde_json::Value) -> IpcResponse {
    let req = IpcRequest { v: 1, id: "t1".into(), method: method.into(), params, actor: None };
    let mut out = IpcResponse { v: 1, id: String::new(), ok: false, result: None, error: None };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    assert!(dispatch_request(&req, &mut out, &env.context(), &mut m, &mut evo), "dispatch 本身不失败");
    out
}

#[test]
fn ping_and_config_route() {
    let env = env_with(Actor::Human);
    let pong = call(&env, "iac_ping", serde_json::json!({}));
    assert!(pong.ok);
    assert_eq!(pong.result.unwrap()["pong"], true);
    let cfg = call(&env, "iac_config_get", serde_json::json!({}));
    assert!(cfg.ok);
    assert_eq!(cfg.result.unwrap()["ai"]["default_provider"], "local-builtin");
}

#[test]
fn unknown_method_and_version_mismatch() {
    let env = env_with(Actor::Human);
    let bad = call(&env, "iac_nonsense", serde_json::json!({}));
    assert!(!bad.ok);
    assert_eq!(bad.error.unwrap().code, "IAC-6105");
    // 版本不匹配
    let req = IpcRequest { v: 2, id: "v2".into(), method: "iac_ping".into(), params: serde_json::json!({}), actor: None };
    let mut out = IpcResponse { v: 1, id: String::new(), ok: false, result: None, error: None };
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    assert!(dispatch_request(&req, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.error.unwrap().code, "IAC-6102");
}

#[test]
fn history_route_after_execute_record() {
    // 人机同构：同一 dispatch，Agent 发起执行落同一套记录
    let env = env_with(Actor::Human);
    let exec = call(&env, "iac_execute", serde_json::json!({"command": "echo via-dispatch"}));
    assert!(exec.ok, "err={:?}", exec.error);
    let status = exec.result.as_ref().unwrap()["status"].as_str().unwrap().to_string();
    assert_eq!(status, "ok");
    let hist = call(&env, "iac_history_query", serde_json::json!({"q": "via-dispatch", "limit": 10}));
    assert!(hist.ok);
    assert_eq!(hist.result.unwrap()["total"], 1);
}

#[test]
fn agent_execute_dangerous_rejected_via_same_gate() {
    let env = env_with(Actor::Agent("zcode".into()));
    let exec = call(&env, "iac_execute", serde_json::json!({"command": "echo 'rm -rf x'"}));
    assert!(exec.ok, "拒绝是业务结果");
    assert_eq!(exec.result.unwrap()["status"], "rejected");
}

#[test]
fn recipe_save_and_search_roundtrip() {
    let env = env_with(Actor::Human);
    let save = call(&env, "iac_recipe_save", serde_json::json!({
        "title": "清理日志", "command_template": "rm /tmp/*.log", "tags": ["清理"], "platform": "linux"
    }));
    assert!(save.ok, "err={:?}", save.error);
    let search = call(&env, "iac_recipes_search", serde_json::json!({"q": "清理", "limit": 10}));
    assert!(search.ok);
    assert_eq!(search.result.unwrap().as_array().unwrap().len(), 1);
}
