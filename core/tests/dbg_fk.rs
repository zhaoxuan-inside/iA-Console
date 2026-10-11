use iac_core::history::{record_execution, ExecStatus, ExecutionRecord};
use iac_core::types::RiskLevel;

#[test]
fn probe_fk() {
    let store = infra::storage::Storage::open_in_memory().unwrap();
    let env = iac_core::framework::TestEnv::new().with_store(Box::new(store));
    let mut evo = iac_core::framework::Evolution::default();
    let mut m = iac_core::framework::Metrics::default();
    let mut mig = iac_core::ports::MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&iac_core::ports::MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    let input = iac_core::session::NewSessionInput {
        title: None,
        cwd: ".".into(),
        shell: "sh".into(),
        host: "local".into(),
    };
    let mut info = iac_core::session::SessionInfo::default();
    let ok1 = iac_core::session::create_session(&input, &mut info, &env.context(), &mut m, &mut evo);
    println!("create ok={ok1} id={} evo={evo:?}", info.id);
    let rec = ExecutionRecord {
        id: String::new(),
        session_id: Some(info.id.clone()),
        proposal_id: None,
        command: "x".into(),
        argv: vec!["x".into()],
        cwd: ".".into(),
        shell: "sh".into(),
        host: "local".into(),
        actor: "human".into(),
        exit_code: Some(0),
        status: ExecStatus::Ok,
        risk_level: RiskLevel::Safe,
        stdout_summary: None,
        stderr_summary: None,
        stdout_ref: None,
        stderr_ref: None,
        started_at: 1,
        duration_ms: None,
        journal_ref: None,
    };
    let mut ack = iac_core::session::RecordAck::default();
    let ok2 = record_execution(&rec, &mut ack, &env.context(), &mut m, &mut evo);
    println!("record ok={ok2} evo={evo:?}");
    assert!(ok1 && ok2);
}
