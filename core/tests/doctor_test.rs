//! T-018 医生测试：静态通道命中、LLM 证据链、speculation 标记、AI 失败降级。

use core::doctor::{build_failure_context, diagnose_failure, parse_diagnosis_response, DiagnoseInput, DiagnosePromptBundle, Diagnosis};
use core::framework::{AiPort, Context, Evolution, Metrics, TestEnv};
use core::history::{record_execution, ExecStatus, ExecutionRecord};
use core::ports::{AiRequest, LlmResponse, MigrationAck, MigrationDir};
use infra::storage::Storage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use core::types::RiskLevel;

struct MockAi {
    responses: Vec<&'static str>,
    calls: Arc<AtomicUsize>,
}

impl AiPort for MockAi {
    #[allow(clippy::too_many_arguments)]
    fn ai_complete(
        &self,
        _p: &core::config::ProviderProfile,
        _i: &AiRequest,
        output: &mut LlmResponse,
        _c: &Context,
        m: &mut Metrics,
        e: &mut Evolution,
    ) -> bool {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.responses.get(n) {
            Some(t) => {
                output.text = (*t).to_string();
                m.counter("ai.mock", 1);
                true
            }
            None => {
                e.fail("IAC-1101", "mock 耗尽");
                false
            }
        }
    }
}

fn env_with(responses: Vec<&'static str>) -> (TestEnv, Arc<AtomicUsize>, String) {
    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    let mut info = core::session::SessionInfo { id: String::new(), title: String::new(), cwd: String::new(), shell: String::new(), host: String::new(), status: String::new(), created_at: 0, updated_at: 0, events: vec![] };
    assert!(core::session::create_session(&core::session::NewSessionInput { title: None, cwd: ".".into(), shell: "sh".into(), host: "local".into() }, &mut info, &env.context(), &mut m, &mut evo));
    let calls = Arc::new(AtomicUsize::new(0));
    (env.with_ai(Box::new(MockAi { responses, calls: Arc::clone(&calls) })), calls, info.id)
}

fn failed_record(sid: &str, exit: i32, stderr: Option<&str>) -> ExecutionRecord {
    ExecutionRecord {
        id: String::new(),
        session_id: Some(sid.into()),
        proposal_id: None,
        command: "failing-cmd".into(),
        argv: vec!["failing-cmd".into()],
        cwd: ".".into(),
        shell: "sh".into(),
        host: "local".into(),
        actor: "human".into(),
        exit_code: Some(exit),
        status: ExecStatus::Failed,
        risk_level: RiskLevel::Safe,
        stdout_summary: None,
        stderr_summary: stderr.map(str::to_string),
        stdout_ref: None,
        stderr_ref: None,
        started_at: 1,
        duration_ms: Some(10),
        journal_ref: None,
    }
}

#[test]
fn build_failure_context_flags_static_hits() {
    let env = TestEnv::new();
    let rec = failed_record("s", 127, None);
    let mut bundle = DiagnosePromptBundle::default();
    let mut m = Metrics::default();
    assert!(build_failure_context(&rec, &mut bundle, &env.context(), &mut m, &mut Evolution::default()));
    assert!(bundle.static_root_cause.as_deref().unwrap().contains("命令未找到"));
    assert!(bundle.user.contains("127"));
}

#[test]
fn static_channel_answers_without_ai() {
    let (env, calls, sid) = env_with(vec![]);
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut ack = core::session::RecordAck::default();
    assert!(record_execution(&failed_record(&sid, 127, Some("sh: definitely_missing_cmd_xyz: command not found")), &mut ack, &env.context(), &mut m, &mut evo));
    let mut d = Diagnosis { root_cause: String::new(), evidence: vec![], confidence: 0.0, fix_proposals: vec![], rollback_note: String::new(), speculation: false };
    assert!(diagnose_failure(&DiagnoseInput { execution_id: Some(ack.id.clone()) }, &mut d, &env.context(), &mut m, &mut evo));
    assert!(d.root_cause.contains("命令未找到"));
    assert!(!d.speculation, "静态通道引用了 stderr 尾行作证据");
    assert!(d.evidence.iter().any(|e| e.contains("command not found")));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "静态命中不调 AI");
    assert!(d.rollback_note.contains("无自动回滚"));
}

#[test]
fn llm_diagnosis_with_evidence() {
    let (env, calls, sid) = env_with(vec![r#"{"root_cause":"配置文件语法错误","evidence":["unexpected token at line 3"],"confidence":0.8,"fix_proposals":[{"command":"vim config.toml","explanation":"修正第 3 行"}]}"#]);
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut ack = core::session::RecordAck::default();
    assert!(record_execution(&failed_record(&sid, 1, Some("unexpected token at line 3")), &mut ack, &env.context(), &mut m, &mut evo));
    let mut d = Diagnosis { root_cause: String::new(), evidence: vec![], confidence: 0.0, fix_proposals: vec![], rollback_note: String::new(), speculation: false };
    assert!(diagnose_failure(&DiagnoseInput { execution_id: Some(ack.id.clone()) }, &mut d, &env.context(), &mut m, &mut evo));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(d.root_cause.contains("语法错误"));
    assert_eq!(d.evidence, vec!["unexpected token at line 3"]);
    assert!(!d.speculation);
    assert_eq!(d.fix_proposals.len(), 1);
}

#[test]
fn no_evidence_marks_speculation() {
    let (env, _, sid) = env_with(vec![r#"{"root_cause":"猜测的原因","evidence":[],"confidence":0.3}"#]);
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut ack = core::session::RecordAck::default();
    assert!(record_execution(&failed_record(&sid, 1, Some("某错误")), &mut ack, &env.context(), &mut m, &mut evo));
    let mut d = Diagnosis { root_cause: String::new(), evidence: vec![], confidence: 0.0, fix_proposals: vec![], rollback_note: String::new(), speculation: false };
    assert!(diagnose_failure(&DiagnoseInput { execution_id: Some(ack.id) }, &mut d, &env.context(), &mut m, &mut evo));
    assert!(d.speculation, "无 stderr 证据引用必须标 speculation");
}

#[test]
fn ai_failure_propagates_when_static_misses() {
    let (env, calls, sid) = env_with(vec![]);
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut ack = core::session::RecordAck::default();
    assert!(record_execution(&failed_record(&sid, 1, Some("复杂错误")), &mut ack, &env.context(), &mut m, &mut evo));
    let mut d = Diagnosis { root_cause: String::new(), evidence: vec![], confidence: 0.0, fix_proposals: vec![], rollback_note: String::new(), speculation: false };
    assert!(!diagnose_failure(&DiagnoseInput { execution_id: Some(ack.id) }, &mut d, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn parse_diagnosis_requires_valid_json() {
    let env = TestEnv::new();
    let resp = LlmResponse { text: "不是 JSON".into(), model: "m".into() };
    let mut d = Diagnosis { root_cause: String::new(), evidence: vec![], confidence: 0.0, fix_proposals: vec![], rollback_note: String::new(), speculation: false };
    let mut evo = Evolution::default();
    assert!(!parse_diagnosis_response(&resp, &mut d, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1201"));
}
