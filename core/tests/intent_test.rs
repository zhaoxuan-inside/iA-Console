//! T-016 意图编排测试：MockAi 覆盖正常提案、解析重试、AI 不可用降级、空意图、explain 缓存回写。

use core::config::ProviderProfile;
use core::framework::{AiPort, Context, Evolution, Metrics};
use core::intent::{
    handle_ask, handle_explain, AskInput, ExplainInput, Proposal, ProposalStatus,
};
use core::ports::{AiRequest, LlmResponse, MigrationAck, MigrationDir};
use infra::storage::Storage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const GOOD_JSON: &str = r#"{"command":"ls -la /var/log","summary":"列出日志目录","params":[{"flag":"-la","meaning":"长格式含隐藏"}],"impact":"只读","risk":"safe"}"#;
const BAD_JSON: &str = "完全不是 JSON 的回答";

/// 可编程 Mock：按调用序返回预设响应（记录调用次数供断言）。
struct MockAi {
    responses: Vec<&'static str>,
    calls: Arc<AtomicUsize>,
}

impl AiPort for MockAi {
    #[allow(clippy::too_many_arguments)]
    fn ai_complete(
        &self,
        _provider: &ProviderProfile,
        _input: &AiRequest,
        output: &mut LlmResponse,
        _context: &Context,
        metrics: &mut Metrics,
        evolution: &mut Evolution,
    ) -> bool {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let Some(text) = self.responses.get(n).or(self.responses.last()) else {
            evolution.fail("IAC-1101", "mock 响应耗尽");
            return false;
        };
        output.text = (*text).to_string();
        output.model = "mock".into();
        metrics.counter("ai.mock", 1);
        true
    }
}

fn env_with_ai(responses: Vec<&'static str>) -> (TestEnv, Arc<AtomicUsize>) {
    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    // 迁移
    let mut ack = MigrationAck::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut ack, &env.context(), &mut m, &mut evo));
    let calls = Arc::new(AtomicUsize::new(0));
    let ai = MockAi { responses, calls: Arc::clone(&calls) };
    (env.with_ai(Box::new(ai)), calls)
}

fn empty_proposal() -> Proposal {
    Proposal {
        id: String::new(),
        session_id: String::new(),
        intent_text: String::new(),
        command: String::new(),
        argv: vec![],
        explanation: Default::default(),
        risk_level: core::types::RiskLevel::Safe,
        platform: core::types::Platform::Linux,
        verified: false,
        status: ProposalStatus::Proposed,
        created_at: 0,
    }
}

use core::framework::TestEnv;

#[test]
fn ask_returns_persisted_proposal() {
    let (env, calls) = env_with_ai(vec![GOOD_JSON]);
    let mut out = empty_proposal();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = AskInput { intent: "列出日志目录".into(), cwd: Some("/var/log".into()), platform: None };
    assert!(handle_ask(&input, &mut out, &env.context(), &mut m, &mut evo), "evo={evo:?}");
    assert_eq!(out.id.len(), 32, "提案已落库");
    assert_eq!(out.command, "ls -la /var/log");
    assert_eq!(out.status, ProposalStatus::Proposed);
    assert_eq!(out.risk_level, core::types::RiskLevel::Safe);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // 会话事件已记录
    let mut session = None;
    assert!(core::session::load_session(&core::session::SessionId { id: out.session_id.clone() }, &mut session, &env.context(), &mut m, &mut evo));
    assert_eq!(session.unwrap().events.len(), 1);
}

#[test]
fn parse_failure_retries_once_then_fails() {
    let (env, calls) = env_with_ai(vec![BAD_JSON, BAD_JSON]);
    let mut out = empty_proposal();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = AskInput { intent: "列出文件".into(), cwd: None, platform: None };
    assert!(!handle_ask(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1102"));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "解析失败恰好重试 1 次");
}

#[test]
fn parse_retry_succeeds_on_second_response() {
    let (env, calls) = env_with_ai(vec![BAD_JSON, GOOD_JSON]);
    let mut out = empty_proposal();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = AskInput { intent: "列出文件".into(), cwd: None, platform: None };
    assert!(handle_ask(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn ai_unavailable_maps_to_1101() {
    let (env, calls) = env_with_ai(vec![]);
    let mut out = empty_proposal();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = AskInput { intent: "列出文件".into(), cwd: None, platform: None };
    assert!(!handle_ask(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "AI 不可达不做解析重试");
}

#[test]
fn empty_intent_rejected_without_ai_call() {
    let (env, calls) = env_with_ai(vec![GOOD_JSON]);
    let mut out = empty_proposal();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = AskInput { intent: "   ".into(), cwd: None, platform: None };
    assert!(!handle_ask(&input, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1103"));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "空意图不调用模型（AC-20 精神）");
}

#[test]
fn explain_uses_help_parser_and_caches() {
    // mock 进程：真实 sh 执行 `printf` 输出 help 文本
    struct MockProc;
    impl core::ports::ProcessPort for MockProc {
        fn spawn_process(
            &self,
            input: &core::ports::ProcessSpec,
            output: &mut core::ports::ProcessResult,
            _ctx: &Context,
            m: &mut Metrics,
            e: &mut Evolution,
        ) -> bool {
            assert!(input.command.ends_with("--help"));
            output.exit_code = Some(0);
            output.stdout = "Usage: ls\n\n  -a, --all     do not ignore entries\n  -l            long format\n".into();
            output.shell_used = "mock".into();
            m.counter("proc.mock", 1);
            e.succeed();
            true
        }
    }
    let store = Storage::open_in_memory().expect("db");
    let mut evo0 = Evolution::default();
    let mut m0 = Metrics::default();
    let mut mig = MigrationAck::default();
    let base = TestEnv::new().with_store(Box::new(store));
    assert!(base.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &base.context(), &mut m0, &mut evo0));
    let env = base
        .with_ai(Box::new(MockAi { responses: vec![], calls: Arc::new(AtomicUsize::new(0)) }))
        .with_process(Box::new(MockProc));
    let mut out = core::intent::ParamSpecDoc::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let input = ExplainInput { command: "ls".into(), platform: None };
    assert!(handle_explain(&input, &mut out, &env.context(), &mut m, &mut evo), "evo={evo:?}");
    assert!(out.summary.contains("3 个参数"));

    // 第二次调用命中缓存（不再触发进程）
    let mut out2 = core::intent::ParamSpecDoc::default();
    assert!(handle_explain(&input, &mut out2, &env.context(), &mut m, &mut evo));
    assert_eq!(out2.params.len(), 3);
}
