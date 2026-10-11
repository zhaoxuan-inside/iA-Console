//! T-019 方案库测试：保存/检索（关键词+标签+平台）/详情/删除。

use core::framework::TestEnv;
use core::ports::{MigrationAck, MigrationDir};
use core::recipe::{delete_recipe, get_recipe, save_recipe, search_recipes, RecipeFilter, RecipeId, RecipeInput};
use core::types::RiskLevel;
use infra::storage::Storage;

fn env_with_db() -> TestEnv {
    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));
    env
}

fn input(title: &str, cmd: &str, tags: Vec<&str>, platform: &str) -> RecipeInput {
    RecipeInput {
        title: title.into(),
        description: Some(format!("{title} 的说明")),
        kind: None,
        name: None,
        command_template: cmd.into(),
        tags: tags.into_iter().map(str::to_string).collect(),
        platform: platform.into(),
        risk_level: RiskLevel::Safe,
        source_execution_id: None,
    }
}

#[test]
fn save_get_delete_lifecycle() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let mut recipe = core::recipe::Recipe {
        id: String::new(), title: String::new(), description: None, kind: String::new(), name: None,
        command_template: String::new(), tags: vec![], platform: String::new(), risk_level: RiskLevel::Safe,
        usage_count: 0, source_execution_id: None, created_at: 0, updated_at: 0,
    };
    assert!(save_recipe(&input("备份日志", "tar -czf /tmp/log.tgz /var/log", vec!["备份", "日志"], "linux"), &mut recipe, &env.context(), &mut m, &mut evo));
    assert_eq!(recipe.id.len(), 32);
    assert_eq!(recipe.tags, vec!["备份", "日志"]);

    // get：详情 + 使用计数 +1
    let mut got = None;
    assert!(get_recipe(&RecipeId { id: recipe.id.clone() }, &mut got, &env.context(), &mut m, &mut evo));
    let got = got.unwrap();
    assert_eq!(got.usage_count, 1);

    // delete：删除后 get 为 None
    let mut del = core::recipe::DeleteAck::default();
    assert!(delete_recipe(&RecipeId { id: recipe.id.clone() }, &mut del, &env.context(), &mut m, &mut evo));
    assert!(del.deleted);
    let mut gone = None;
    assert!(get_recipe(&RecipeId { id: recipe.id }, &mut gone, &env.context(), &mut m, &mut evo));
    assert!(gone.is_none());
}

#[test]
fn search_by_keyword_tag_and_platform() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let mut r = core::recipe::Recipe { id: String::new(), title: String::new(), description: None, kind: String::new(), name: None, command_template: String::new(), tags: vec![], platform: String::new(), risk_level: RiskLevel::Safe, usage_count: 0, source_execution_id: None, created_at: 0, updated_at: 0 };
    save_recipe(&input("备份日志", "tar -czf /tmp/log.tgz /var/log", vec!["备份"], "linux"), &mut r, &env.context(), &mut m, &mut evo);
    save_recipe(&input("清理容器", "docker system prune -f", vec!["清理", "docker"], "linux"), &mut r, &env.context(), &mut m, &mut evo);
    save_recipe(&input("Windows 清理", "Get-ChildItem | Remove-Item", vec!["清理"], "windows"), &mut r, &env.context(), &mut m, &mut evo);

    // 关键词
    let mut out = vec![];
    assert!(search_recipes(&RecipeFilter { q: Some("tar".into()), limit: 50, ..Default::default() }, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.len(), 1);
    // 标签
    out.clear();
    assert!(search_recipes(&RecipeFilter { tag: Some("清理".into()), limit: 50, ..Default::default() }, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.len(), 2);
    // 平台（含 all 兼容）：linux 命中 2 条（linux + 无 windows）
    out.clear();
    assert!(search_recipes(&RecipeFilter { platform: Some("linux".into()), limit: 50, ..Default::default() }, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.len(), 2);
    // 空 → 全量
    out.clear();
    assert!(search_recipes(&RecipeFilter { limit: 50, ..Default::default() }, &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(out.len(), 3);
}

#[test]
fn script_name_unique_conflict_fails() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let mut i1 = input("脚本 A", "echo a", vec![], "all");
    i1.kind = Some("script".into());
    i1.name = Some("backup".into());
    let mut r = core::recipe::Recipe { id: String::new(), title: String::new(), description: None, kind: String::new(), name: None, command_template: String::new(), tags: vec![], platform: String::new(), risk_level: RiskLevel::Safe, usage_count: 0, source_execution_id: None, created_at: 0, updated_at: 0 };
    assert!(save_recipe(&i1, &mut r, &env.context(), &mut m, &mut evo));
    let mut i2 = input("脚本 B", "echo b", vec![], "all");
    i2.kind = Some("script".into());
    i2.name = Some("backup".into());
    let mut r2 = core::recipe::Recipe { id: String::new(), title: String::new(), description: None, kind: String::new(), name: None, command_template: String::new(), tags: vec![], platform: String::new(), risk_level: RiskLevel::Safe, usage_count: 0, source_execution_id: None, created_at: 0, updated_at: 0 };
    assert!(!save_recipe(&i2, &mut r2, &env.context(), &mut m, &mut evo), "name 唯一约束应拒绝重复");
    assert!(evo.reason_code.is_some() || !evo.cause.is_empty());
}

#[test]
fn empty_title_rejected() {
    let env = env_with_db();
    let mut evo = core::framework::Evolution::default();
    let mut m = core::framework::Metrics::default();
    let mut r = core::recipe::Recipe { id: String::new(), title: String::new(), description: None, kind: String::new(), name: None, command_template: String::new(), tags: vec![], platform: String::new(), risk_level: RiskLevel::Safe, usage_count: 0, source_execution_id: None, created_at: 0, updated_at: 0 };
    assert!(!save_recipe(&input("", "echo", vec![], "all"), &mut r, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-4101"));
}
