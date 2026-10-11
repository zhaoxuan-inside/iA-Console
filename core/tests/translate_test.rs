//! T-025 翻译编排测试：内置命中零 AI、缓存回写、AI 翻译/组合、AI 不可用降级、同方言。

use iac_core::framework::{AiPort, Context, Evolution, Metrics, TestEnv};
use iac_core::intent::translate_command;
use iac_core::knowledge::translation::TranslationSource;
use iac_core::ports::{AiRequest, LlmResponse, MigrationAck, MigrationDir};
use iac_core::types::Platform;
use infra::storage::Storage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const COMPOSITION_JSON: &str = r#"{"command_to":"Get-ChildItem -Recurse | Sort-Object Length -Descending | Select-Object -First 10","kind":"composition","notes":"sed 无直接等价，用管道组合实现","params":[],"risk":"caution"}"#;

struct MockAi {
    responses: Vec<&'static str>,
    calls: Arc<AtomicUsize>,
}

impl AiPort for MockAi {
    #[allow(clippy::too_many_arguments)]
    fn ai_complete(
        &self,
        _p: &iac_core::config::ProviderProfile,
        _i: &AiRequest,
        output: &mut LlmResponse,
        _c: &Context,
        _m: &mut Metrics,
        e: &mut Evolution,
    ) -> bool {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.responses.get(n) {
            Some(t) => {
                output.text = (*t).to_string();
                true
            }
            None => {
                e.fail("IAC-1101", "mock 耗尽");
                false
            }
        }
    }
}

fn env_with(responses: Vec<&'static str>) -> (TestEnv, Arc<AtomicUsize>) {
    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    let calls = Arc::new(AtomicUsize::new(0));
    (env.with_ai(Box::new(MockAi { responses, calls: Arc::clone(&calls) })), calls)
}

fn translate(env: &TestEnv, command: &str, from: Platform, to: Platform) -> (bool, iac_core::knowledge::translation::TranslationResult, Evolution) {
    let input = iac_core::knowledge::translation::TranslationInput { command: command.into(), from_dialect: from, to_platform: to };
    let mut out = iac_core::knowledge::translation::TranslationResult::native_fallback(&input);
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let ok = translate_command(&input, &mut out, &env.context(), &mut m, &mut evo);
    (ok, out, evo)
}

#[test]
fn builtin_hit_costs_zero_ai() {
    let (env, calls) = env_with(vec![]);
    let (ok, out, evo) = translate(&env, "ls -la /var/log", Platform::Linux, Platform::Windows);
    assert!(ok);
    assert!(evo.ok);
    assert_eq!(out.source, TranslationSource::Static);
    assert_eq!(out.command_to, "Get-ChildItem -Force /var/log");
    // 参数级映射：-a→-Force（renamed），-l 值变换说明
    assert!(out.params.iter().any(|p| p.kind == iac_core::knowledge::translation::ParamMapKind::Renamed && p.to_param.as_deref() == Some("-Force")));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "静态命中零 AI 调用（AC-20）");
}

#[test]
fn same_dialect_is_native_passthrough() {
    let (env, calls) = env_with(vec![]);
    let (ok, out, _) = translate(&env, "ls", Platform::Linux, Platform::Linux);
    assert!(ok);
    assert_eq!(out.source, TranslationSource::Native);
    assert_eq!(out.command_to, "ls");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ai_translation_cached_for_reuse() {
    let (env, calls) = env_with(vec![
        r#"{"command_to":"Get-Content -TotalCount 5 file.log","kind":"direct","notes":"head -n 5 等价","params":[{"from_param":"-n","to_param":"-TotalCount","kind":"renamed","note":"行数直映"}],"risk":"safe"}"#,
    ]);
    // 首次：AI 翻译 + 回写缓存
    let (ok, out, evo) = translate(&env, "journalctl -u nginx --since today", Platform::Linux, Platform::Windows);
    assert!(ok, "evo={evo:?}");
    assert_eq!(out.source, TranslationSource::Ai);
    assert!(out.notes.contains("等价"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // 二次：缓存命中零 AI
    let (_, out2, _) = translate(&env, "journalctl -u nginx --since today", Platform::Linux, Platform::Windows);
    assert_eq!(out2.source, TranslationSource::Static);
    assert_eq!(out2.command_to, "Get-Content -TotalCount 5 file.log");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "缓存命中不重复调用");
}

#[test]
fn ai_composition_saves_script_recipe() {
    let (env, _) = env_with(vec![COMPOSITION_JSON]);
    let (ok, out, evo) = translate(&env, "du -a /var | sort -nr | head -10", Platform::Linux, Platform::Windows);
    assert!(ok, "evo={evo:?}");
    assert_eq!(out.kind, iac_core::knowledge::translation::TranslationKind::Composition);
    assert!(out.recipe_id.is_some(), "组合脚本应入方案库并回填 recipe_id");
    // 脚本资产可检索
    let mut recipes = vec![];
    assert!(iac_core::recipe::search_recipes(&iac_core::recipe::RecipeFilter { tag: Some("translation".into()), limit: 10, ..Default::default() }, &mut recipes, &env.context(), &mut Metrics::default(), &mut Evolution::default()));
    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].kind, "script");
}

#[test]
fn ai_unavailable_degrades_to_native() {
    let (env, calls) = env_with(vec![]);
    let (ok, out, evo) = translate(&env, "chattr +i /etc/fstab", Platform::Linux, Platform::Windows);
    assert!(ok, "降级为业务结果而非错误");
    assert!(evo.ok);
    assert_eq!(out.source, TranslationSource::Native);
    assert_eq!(out.command_to, out.command_from);
    assert!(out.notes.contains("AI 不可用"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
