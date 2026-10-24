//! M7 回滚 / M8 脚本工坊集成测试（T-038~T-041、T-042/T-044 验收）。
//! 真实 Storage（内存库）+ 真实 SnapPort（临时目录，IAC_DATA_DIR 隔离）。

use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{ExecuteAck, MigrationAck, MigrationDir, QueryRows, SqlQuery, SqlValue};
use infra::snapstore::RealSnap;
use infra::storage::Storage;
use std::path::PathBuf;
use std::sync::Mutex;

/// IAC_DATA_DIR 为进程级环境变量：全部 FS 测试串行执行。
static FS_LOCK: Mutex<()> = Mutex::new(());

struct Fixture {
    env: TestEnv,
    work: PathBuf,
    sess_id: String,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let guard = FS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let data = std::env::temp_dir().join(format!("iac-m7-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("IAC_DATA_DIR", &data);
        let store = Storage::open_in_memory().unwrap();
        let mut env = TestEnv::new()
            .with_store(Box::new(store))
            .with_snap(Box::new(RealSnap::new()))
            .with_process(Box::new(infra::process::RealProcess));
        let mut ack = MigrationAck::default();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut ack, &env.context(), &mut m, &mut evo));
        // 供引用的会话
        let mut info = iac_core::session::SessionInfo::default();
        assert!(iac_core::session::store::get_or_create_current(
            &iac_core::session::CurrentSessionInput { cwd: "/tmp".into(), shell: "sh".into(), host: "t".into(), title: None },
            &mut info, &env.context(), &mut m, &mut evo,
        ));
        env.session.session_id = info.id.clone();
        let work = data.join("work");
        std::fs::create_dir_all(&work).unwrap();
        Self { env, work, sess_id: info.id, _guard: guard }
    }

    fn cwd(&self) -> String {
        self.work.to_string_lossy().to_string()
    }

    fn seed_exec(&self, command: &str) -> String {
        let id = uuid_hex();
        let q = SqlQuery {
            sql: "INSERT INTO executions (id, session_id, proposal_id, command, argv, cwd, shell, host, actor, exit_code, status, risk_level, started_at, journal_ref) \
                  VALUES (?, ?, NULL, ?, '[]', '.', 'sh', 'local', 'human', 0, 'ok', 'safe', strftime('%s','now'), NULL)"
                .into(),
            params: vec![SqlValue::Text(id), SqlValue::Text(self.sess_id.clone()), SqlValue::Text(command.into())],
        };
        let mut ack = ExecuteAck::default();
        let mut evo = Evolution::default();
        let mut m = Metrics::default();
        assert!(self.env.deps.store.storage_execute(&q, &mut ack, &self.env.context(), &mut m, &mut evo));
        let mut rows = QueryRows::default();
        let sel = SqlQuery { sql: "SELECT id FROM executions WHERE rowid=?".into(), params: vec![SqlValue::Int(ack.last_rowid)] };
        assert!(self.env.deps.store.storage_query(&sel, &mut rows, &self.env.context(), &mut m, &mut evo));
        rows.text(0, "id").unwrap().to_string()
    }

    /// 模拟"命令已执行"：捕获前像 → 改文件 → diff 入账（等价 executor 的 post_diff 流程）。
    fn run_tracked(&self, exec_id: &str, mutate: impl FnOnce()) -> Vec<iac_core::types::EffectEntry> {
        let manifest = iac_core::rollback::pre_capture(&self.cwd(), &self.env.context(), &mut Metrics::default(), &mut Evolution::default()).expect("捕获");
        mutate();
        if manifest.manifest_ref.is_empty() {
            return Vec::new();
        }
        let mut m = manifest;
        m.execution_id = exec_id.to_string();
        let mut effects = Vec::new();
        assert!(iac_core::rollback::diff_effects(&m, &mut effects, &self.env.context(), &mut Metrics::default(), &mut Evolution::default()));
        effects
    }
}

fn uuid_hex() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("e-{n:032x}")
}

#[test]
fn capture_diff_journal_full_cycle() {
    let fx = Fixture::new("cycle");
    let exec = fx.seed_exec("echo data > a.txt");
    std::fs::write(fx.work.join("a.txt"), b"v1").unwrap();
    std::fs::write(fx.work.join("keep.txt"), b"same").unwrap();
    // 第一次：捕获基线（无变化无条目）
    let mut m0 = iac_core::rollback::pre_capture(&fx.cwd(), &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()).unwrap();
    m0.execution_id = "e-base".into();
    let mut none = Vec::new();
    assert!(iac_core::rollback::diff_effects(&m0, &mut none, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    // 第二次：created + modified
    let effects = fx.run_tracked(&exec, || {
        std::fs::write(fx.work.join("a.txt"), b"v2").unwrap();
        std::fs::write(fx.work.join("b.txt"), b"new").unwrap();
    });
    assert_eq!(effects.len(), 2, "created b.txt + modified a.txt");
    assert!(effects.iter().all(|e| e.scope == iac_core::types::CaptureScopeFlag::In));
    let exec2 = fx.seed_exec("touch keep.txt");
    let effects2 = fx.run_tracked(&exec2, || {});
    assert!(effects2.is_empty(), "无变化无条目");
}

#[test]
fn replay_rollback_created_moves_to_trash() {
    let fx = Fixture::new("replay-created");
    let exec = fx.seed_exec("create b.txt");
    let effects = fx.run_tracked(&exec, || {
        std::fs::write(fx.work.join("b.txt"), b"data").unwrap();
    });
    assert_eq!(effects.len(), 1);
    let seq = effects[0].seq;
    let mut report = iac_core::types::RollbackReport::default();
    let req = iac_core::types::RollbackRequest {
        range: iac_core::types::RollbackRange { from: None, to: Some(seq - 1) },
        on_conflict: iac_core::types::ConflictPolicy::Skip,
    };
    assert!(iac_core::rollback::rollback_to(&req, &mut report, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert_eq!(report.applied.len(), 1);
    assert!(!fx.work.join("b.txt").exists(), "created 撤销后文件入回收站");
    assert!(report.trash_dir.contains("trash"));
}

#[test]
fn replay_modified_restores_previous_content() {
    let fx = Fixture::new("replay-mod");
    let exec = fx.seed_exec("edit a.txt");
    std::fs::write(fx.work.join("a.txt"), b"v1").unwrap();
    fx.run_tracked(&exec, || {
        std::fs::write(fx.work.join("a.txt"), b"v2").unwrap();
    });
    let mut report = iac_core::types::RollbackReport::default();
    let req = iac_core::types::RollbackRequest {
        range: iac_core::types::RollbackRange { from: None, to: None },
        on_conflict: iac_core::types::ConflictPolicy::Skip,
    };
    assert!(iac_core::rollback::rollback_to(&req, &mut report, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert_eq!(std::fs::read(fx.work.join("a.txt")).unwrap(), b"v1", "modified 撤销恢复前像");
}

#[test]
fn replay_conflict_policy() {
    let fx = Fixture::new("conflict");
    let exec = fx.seed_exec("edit c.txt");
    std::fs::write(fx.work.join("c.txt"), b"v1").unwrap();
    fx.run_tracked(&exec, || {
        std::fs::write(fx.work.join("c.txt"), b"v2").unwrap();
    });
    // 用户手动又改了 → 回放冲突
    std::fs::write(fx.work.join("c.txt"), b"manual").unwrap();
    let mut report = iac_core::types::RollbackReport::default();
    let req = iac_core::types::RollbackRequest {
        range: iac_core::types::RollbackRange { from: None, to: None },
        on_conflict: iac_core::types::ConflictPolicy::Abort,
    };
    assert!(iac_core::rollback::rollback_to(&req, &mut report, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert!(report.applied.is_empty());
    let mut report2 = iac_core::types::RollbackReport::default();
    let req2 = iac_core::types::RollbackRequest {
        range: iac_core::types::RollbackRange { from: None, to: None },
        on_conflict: iac_core::types::ConflictPolicy::Skip,
    };
    assert!(iac_core::rollback::rollback_to(&req2, &mut report2, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert!(report2.applied.is_empty() && report2.skipped.len() == 1);
    assert!(report2.skipped[0].reason.contains("IAC-1603"));
}

#[test]
fn replay_deleted_recreates_file() {
    let fx = Fixture::new("deleted");
    std::fs::write(fx.work.join("d.txt"), b"before-delete").unwrap();
    let exec = fx.seed_exec("rm d.txt");
    let effects = fx.run_tracked(&exec, || {
        std::fs::remove_file(fx.work.join("d.txt")).unwrap();
    });
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0].op, iac_core::types::EffectOp::Deleted));
    let mut report = iac_core::types::RollbackReport::default();
    let req = iac_core::types::RollbackRequest {
        range: iac_core::types::RollbackRange { from: None, to: None },
        on_conflict: iac_core::types::ConflictPolicy::Skip,
    };
    assert!(iac_core::rollback::rollback_to(&req, &mut report, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert_eq!(std::fs::read(fx.work.join("d.txt")).unwrap(), b"before-delete", "deleted 撤销重建前像");
}

#[test]
fn list_and_file_history_and_restore() {
    let fx = Fixture::new("list");
    std::fs::write(fx.work.join("a.txt"), b"v0").unwrap();
    // 执行点 1：a.txt v0→v1
    let exec1 = fx.seed_exec("edit a.txt step1");
    fx.run_tracked(&exec1, || {
        std::fs::write(fx.work.join("a.txt"), b"v1").unwrap();
    });
    // 执行点 2：a.txt v1→v2
    let exec2 = fx.seed_exec("edit a.txt step2");
    fx.run_tracked(&exec2, || {
        std::fs::write(fx.work.join("a.txt"), b"v2").unwrap();
    });
    let mut points = Vec::new();
    assert!(iac_core::rollback::list_rollback_points(
        &iac_core::types::RollbackListFilter { q: None, since: None, limit: 50 },
        &mut points, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(points.len(), 2, "两个执行点");
    assert!(points.iter().all(|p| matches!(p.rollbackable, iac_core::types::Rollbackable::Full)));
    let mut filtered = Vec::new();
    assert!(iac_core::rollback::list_rollback_points(
        &iac_core::types::RollbackListFilter { q: Some("step1".into()), since: None, limit: 50 },
        &mut filtered, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(filtered.len(), 1, "q 过滤命中 step1");
    // 文件版本链：v1（step1 后）+ v2（step2 后）
    let mut versions = Vec::new();
    assert!(iac_core::rollback::file_history(
        &iac_core::rollback::FilePath { path: fx.work.join("a.txt").to_string_lossy().to_string() },
        &mut versions, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(versions.len(), 2);
    // 恢复到 step1 后的版本（v1）
    let target = versions[0].seq;
    let mut report = iac_core::types::RestoreReport { path: String::new(), restored_from: 0, gate: Default::default() };
    assert!(iac_core::rollback::restore_file(
        &iac_core::types::FileVersionRef { path: fx.work.join("a.txt").to_string_lossy().to_string(), seq: target },
        &mut report, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(report.restored_from, target);
    assert_eq!(std::fs::read(fx.work.join("a.txt")).unwrap(), b"v1", "恢复到 v1 版本内容");
}

#[test]
fn max_entries_retires_oldest_points() {
    let mut fx = Fixture::new("retire");
    fx.env.config.rollback.max_entries = 2;
    for i in 0..4 {
        let exec = fx.seed_exec(&format!("gen {i}"));
        let work = fx.work.clone();
        let _ = fx.run_tracked(&exec, move || {
            std::fs::write(work.join(format!("g{i}.txt")), format!("v{i}")).unwrap();
        });
    }
    let mut points = Vec::new();
    assert!(iac_core::rollback::list_rollback_points(
        &iac_core::types::RollbackListFilter { q: None, since: None, limit: 100 },
        &mut points, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(points.len(), 2, "超限清退最旧执行点（保留最新 2 个）");
}

#[test]
fn collect_range_and_make_and_run_script() {
    let fx = Fixture::new("script");
    fx.seed_exec("echo one");
    fx.seed_exec("echo two");
    fx.seed_exec("pwd");
    let mut cmds = Vec::new();
    assert!(iac_core::script::collect_commands(
        &iac_core::script::CollectRange { segment_id: None, from_seq: None, to_seq: None },
        &mut cmds, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(cmds.len(), 3);
    let mut out = iac_core::script::ScriptMakeOutput::default();
    assert!(iac_core::script::make_script(
        &iac_core::script::ScriptMakeInput {
            name: "build-all".into(),
            from_seq: None, to_seq: None,
            segment: None,
            target: iac_core::common::script_assembler::ScriptTarget::Sh,
            refine: false,
            description: None,
        },
        &mut out, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    assert_eq!(out.command_count, 3);
    assert_eq!(out.max_risk, iac_core::types::RiskLevel::Safe);
    let body = out.sh_body.expect("sh 正文");
    assert!(body.contains("set -euo pipefail") && body.contains("echo one") && body.contains("pwd"));
    // 重复注册同名 → 失败（name 全局唯一）
    let mut dup = iac_core::script::ScriptMakeOutput::default();
    assert!(!iac_core::script::make_script(
        &iac_core::script::ScriptMakeInput {
            name: "build-all".into(),
            from_seq: None, to_seq: None, segment: None,
            target: iac_core::common::script_assembler::ScriptTarget::Sh,
            refine: false, description: None,
        },
        &mut dup, &fx.env.context(), &mut Metrics::default(), &mut Evolution::default(),
    ));
    // run @build-all：走闸门与记录（真实进程）
    let mut outcome = iac_core::executor::ExecutionOutcome::default();
    let mut run_evo = Evolution::default();
    assert!(iac_core::script::run_script(
        &iac_core::script::ScriptRunInput { name: "build-all".into(), args: vec![], confirm_dangerous: false, confirm_word: None },
        &mut outcome, &fx.env.context(), &mut Metrics::default(), &mut run_evo,
    ));
    eprintln!("DEBUG run: exit={:?} status={:?} reason={:?} {}", outcome.exit_code, outcome.status, run_evo.reason_code, run_evo.reason.as_deref().unwrap_or(""));
    assert_eq!(outcome.exit_code, Some(0));
}
