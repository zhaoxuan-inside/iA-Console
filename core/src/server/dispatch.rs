//! I-030：唯一路由入口——方法名 → 编排方法，信封错误码统一转译。
//! 人类（GUI/TUI/CLI）与 Agent（MCP）经同一 dispatch，闸门与审计行为一致（AC-08）。
//! 注：`iac_cancel` 由 daemon 接入层在路由前处理（需访问请求注册表），不进本表。

use crate::error::codes;
use crate::executor::handle_execute;
use crate::framework::{Context, Evolution, Metrics};
use crate::history::store::{clean_history, export_history, query_history, record_audit};
use crate::history::{
    HistoryCleanInput, HistoryCleanReport, HistoryExportInput, HistoryExportReport, HistoryFilter,
    HistoryPage,
};
use crate::intent::translate::translate_command;
use crate::intent::{handle_ask, handle_explain};
use crate::knowledge::{TranslationInput, TranslationResult};
use crate::recipe::handler::{delete_recipe, export_recipes, save_recipe, search_recipes};
use crate::recipe::{
    Recipe, RecipeDeleteInput, RecipeExportInput, RecipeExportReport, RecipeFilter, RecipeInput,
};
use crate::server::envelope::{IpcRequest, IpcResponse};
use crate::session::store::{
    append_event, get_or_create_current, list_sessions, resume_session, AppendEventInput, RecordAck,
};
use crate::session::{
    CurrentSessionInput, EventKind, SessionInfo, SessionListInput, SessionResumeInput,
    SessionSummary,
};
use crate::{doctor, executor, intent, rollback, script};
use serde_json::{json, Value};

/// I-030：路由分发。编排返回 false → 信封 error（evolution.reason_code）。
pub fn dispatch_request(
    input: &IpcRequest,
    output: &mut IpcResponse,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    if input.v != 1 {
        *output = IpcResponse::failure(
            input,
            codes::PROTOCOL_MISMATCH,
            format!("协议版本不支持: v{}", input.v),
        );
        return true;
    }
    let params = input.params.clone();
    let result = route(input.method.as_str(), &params, context, metrics, evolution);
    *output = match result {
        Ok(v) => IpcResponse::success(input, v),
        Err((code, msg)) if code == codes::DAEMON_STOP => IpcResponse::failure(input, &code, msg),
        Err((code, msg)) => {
            if evolution.reason_code.is_none() {
                evolution.fail(&code, msg.clone());
            }
            IpcResponse::failure(
                input,
                evolution.reason_code.as_deref().unwrap_or(&code),
                msg,
            )
        }
    };
    metrics.counter("dispatch.routed", 1);
    true
}

/// 方法路由表（dispatch_request 子步骤）。
fn route(
    method: &str,
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    match method {
        "iac_ping" => Ok(json!({"pong": true, "version": env!("CARGO_PKG_VERSION")})),
        "iac_builtin" => route_builtin(params, context, metrics, evolution),
        "iac_ask" => route_ask(params, context, metrics, evolution),
        "iac_execute" => route_execute(params, context, metrics, evolution),
        "iac_explain" => route_explain(params, context, metrics, evolution),
        "iac_diagnose" => route_diagnose(params, context, metrics, evolution),
        "iac_history_query" => route_history(params, context, metrics, evolution),
        "iac_history_clean" => route_history_clean(params, context, metrics, evolution),
        "iac_history_export" => route_history_export(params, context, metrics, evolution),
        "iac_recipes_search" => route_recipes_search(params, context, metrics, evolution),
        "iac_recipe_save" => route_recipe_save(params, context, metrics, evolution),
        "iac_recipe_delete" => route_recipe_delete(params, context, metrics, evolution),
        "iac_recipe_export" => route_recipe_export(params, context, metrics, evolution),
        "iac_translate" => route_translate(params, context, metrics, evolution),
        "iac_sessions_current" => route_sessions_current(params, context, metrics, evolution),
        "iac_session_list" => route_session_list(params, context, metrics, evolution),
        "iac_session_resume" => route_session_resume(params, context, metrics, evolution),
        "iac_session_event" => route_session_event(params, context, metrics, evolution),
        "iac_rollback" => route_rollback(params, context, metrics, evolution),
        "iac_script" => route_script(params, context, metrics, evolution),
        "iac_config_get" => Ok(serde_json::to_value(context.config).unwrap_or_default()),
        "iac_config_set" => route_config_set(params, context, metrics, evolution),
        "iac_daemon_stop" => Err((
            codes::DAEMON_STOP.to_string(),
            "停机指令已受理（由 daemon 入口处理）".to_string(),
        )),
        other => Err((
            codes::UNKNOWN_METHOD.to_string(),
            format!("未知方法 {other}（ia-help 查看内置命令；可用方法见重建契约）"),
        )),
    }
}

// ---------- 域 1/4：意图与解释 ----------

/// iac_ask：意图 → 多候选提案（Q17）。
fn route_ask(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: intent::AskInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::INTENT_INVALID.into(), format!("参数解析失败: {e}")))?;
    let mut out = intent::Proposal {
        id: String::new(),
        session_id: String::new(),
        intent_text: String::new(),
        candidates: vec![],
        active: 0,
        platform: crate::types::Platform::Linux,
        verified: false,
        status: intent::ProposalStatus::Proposed,
        created_at: 0,
        context_json: None,
    };
    if !handle_ask(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::AI_PARSE.into(), format!("提案序列化失败: {e}")))
}

/// iac_execute：闸门执行（Agent 路径经同一闸门，AC-08；Agent 无 origin 视同翻译路径）。
fn route_execute(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let mut input: executor::ExecuteInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    if matches!(context.session.actor, crate::framework::Actor::Agent(_)) && input.origin.is_none()
    {
        input.origin = Some(executor::CommandOrigin::Translated); // Agent 发起视同翻译路径（恒确认）
    }
    let mut out = executor::ExecutionOutcome::default();
    if !handle_execute(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::AI_PARSE.into(), format!("结果序列化失败: {e}")))
}

/// iac_explain：参数解释。
fn route_explain(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: intent::ExplainInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = intent::ParamSpecDoc::default();
    if !handle_explain(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::AI_PARSE.into(), format!("序列化失败: {e}")))
}

/// iac_diagnose：错误诊断（Q18 一键「AI 帮忙」触发）。
fn route_diagnose(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: doctor::DiagnoseInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = doctor::Diagnosis::default();
    if !doctor::handler::diagnose_failure(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::DIAGNOSE.into(), format!("序列化失败: {e}")))
}

// ---------- 域 7：历史与审计 ----------

/// iac_history_query：历史检索。
fn route_history(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: HistoryFilter = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = HistoryPage::default();
    if !query_history(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_history_clean：保留清理（Q47 双限 LRU；记审计）。
fn route_history_clean(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: HistoryCleanInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = HistoryCleanReport::default();
    if !clean_history(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    if !input.dry_run && out.removed > 0 {
        let _ = record_audit(
            &context.session.actor,
            "history_clean",
            "executions",
            serde_json::json!({ "removed": out.removed, "bytes_freed": out.bytes_freed }),
            context,
            metrics,
            evolution,
        );
    }
    serde_json::to_value(out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_history_export：导出归档（Q47；记审计）。
fn route_history_export(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: HistoryExportInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = HistoryExportReport::default();
    if !export_history(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    let _ = record_audit(
        &context.session.actor,
        "history_export",
        &input.path,
        serde_json::json!({ "exported": out.exported }),
        context,
        metrics,
        evolution,
    );
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

// ---------- 域 5：方案库 ----------

/// iac_recipes_search：方案检索。
fn route_recipes_search(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: RecipeFilter = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out: Vec<Recipe> = Vec::new();
    if !search_recipes(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_recipe_save：保存方案（审计由 handler 记）。
fn route_recipe_save(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: RecipeInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = Recipe::default();
    if !save_recipe(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_recipe_delete：删除方案。
fn route_recipe_delete(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: RecipeDeleteInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = false;
    if !delete_recipe(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    Ok(json!({ "deleted": out }))
}

/// iac_recipe_export：方案导出。
fn route_recipe_export(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: RecipeExportInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = RecipeExportReport::default();
    if !export_recipes(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

// ---------- 域 11：翻译 ----------

/// iac_translate：跨方言翻译（AC-19/20/23/24）。
fn route_translate(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: TranslationInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = TranslationResult::native_fallback(&input);
    if !translate_command(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out)
        .map_err(|e| (codes::TRANSLATE_MISS.into(), format!("序列化失败: {e}")))
}

// ---------- 域 6：会话 ----------

/// iac_sessions_current：当前会话（含 env 与事件流）。
fn route_sessions_current(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: CurrentSessionInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = SessionInfo::default();
    if !get_or_create_current(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_session_list：会话列表。
fn route_session_list(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: SessionListInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out: Vec<SessionSummary> = Vec::new();
    if !list_sessions(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_session_resume：恢复指定会话（Q46）。
fn route_session_resume(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let input: SessionResumeInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = SessionInfo::default();
    if !resume_session(&input, &mut out, context, metrics, evolution) {
        return fail(evolution);
    }
    serde_json::to_value(&out).map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
}

/// iac_session_event：客户端事件持久化（separator/note/env/ai_message；对话轨与分段 Q26/Q29/Q33）。
fn route_session_event(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let sid = params
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or(&context.session.session_id)
        .to_string();
    let kind = params.get("kind").and_then(Value::as_str).unwrap_or("note");
    let pinned = params
        .get("pinned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut seq = None;
    let ev = match kind {
        "separator" => {
            let _ = pinned;
            crate::session::store::record_separator(&sid, context, metrics, evolution)
        }
        "note" => {
            let text = params
                .pointer("/payload/text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            crate::session::store::record_note(&sid, text, pinned, context, metrics, evolution)
        }
        "env" => {
            let k = params
                .pointer("/payload/key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let v = params
                .pointer("/payload/value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            crate::session::store::record_env(&sid, k, v, context, metrics, evolution)
        }
        "ai_message" => {
            let mut ack = RecordAck::default();
            let ok = append_event(
                &AppendEventInput {
                    session_id: sid,
                    kind: EventKind::AiMessage,
                    payload: params.get("payload").cloned().unwrap_or(json!({})),
                    pinned,
                },
                &mut ack,
                context,
                metrics,
                evolution,
            );
            seq = Some(ack.seq);
            ok
        }
        other => {
            return Err((
                codes::UNKNOWN_METHOD.into(),
                format!("iac_session_event 未知 kind: {other}"),
            ))
        }
    };
    if !ev {
        return fail(evolution);
    }
    match seq {
        Some(s) if s > 0 => Ok(json!({ "recorded": true, "seq": s })),
        _ => Ok(json!({ "recorded": true })),
    }
}

// ---------- 域 15/16：回滚与脚本 ----------

/// iac_rollback：list / replay / file_history / restore 四动作（Q49 触达）。
fn route_rollback(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let action = params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list");
    match action {
        "list" => {
            let filter = params.get("filter").unwrap_or(params);
            let input: crate::types::RollbackListFilter = serde_json::from_value(filter.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = Vec::new();
            if !rollback::list_rollback_points(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        "replay" => {
            let input: crate::types::RollbackRequest = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = crate::types::RollbackReport::default();
            if !rollback::rollback_to(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            let _ = record_audit(
                &context.session.actor,
                "rollback_replay",
                "rollback_journal",
                serde_json::json!({ "applied": out.applied.len(), "skipped": out.skipped.len() }),
                context,
                metrics,
                evolution,
            );
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        "file_history" => {
            let input: rollback::FilePath = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = Vec::new();
            if !rollback::file_history(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        "restore" => {
            let input: crate::types::FileVersionRef = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = crate::types::RestoreReport {
                path: String::new(),
                restored_from: 0,
                gate: Default::default(),
            };
            if !rollback::restore_file(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            let _ = record_audit(
                &context.session.actor,
                "rollback_restore",
                &out.path,
                serde_json::json!({ "restored_from": out.restored_from }),
                context,
                metrics,
                evolution,
            );
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        other => Err((
            codes::UNKNOWN_METHOD.into(),
            format!("iac_rollback 未知 action: {other}"),
        )),
    }
}

/// iac_script：make / refine / run 三动作（AC-38~40）。
fn route_script(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let action = params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("make");
    match action {
        "make" => {
            let input: script::ScriptMakeInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = script::ScriptMakeOutput::default();
            if !script::make_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        "refine" => {
            let input: script::ScriptRefineInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = script::ScriptRefineOutput::default();
            if !script::refine_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        "run" => {
            let input: script::ScriptRunInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = executor::ExecutionOutcome::default();
            if !script::run_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out)
                .map_err(|e| (codes::STORAGE.into(), format!("序列化失败: {e}")))
        }
        other => Err((
            codes::UNKNOWN_METHOD.into(),
            format!("iac_script 未知 action: {other}"),
        )),
    }
}

// ---------- 配置 ----------

/// iac_config_set：计算新配置返回（daemon 侧负责应用与落盘）；记审计。
/// 支持键：配置 schema 全量标量键 + providers（JSON 数组，交互向导与直改同通道）。
fn route_config_set(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    let key = params
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let value = params
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut cfg = context.config.clone();
    if !apply_config_set(&mut cfg, &key, &value)? {
        return Err((
            codes::CONFIG_INVALID.into(),
            format!("配置值非法: {key}={value}"),
        ));
    }
    if let Err(msg) = cfg.validate() {
        return Err((codes::CONFIG_INVALID.into(), msg));
    }
    let _ = record_audit(
        &context.session.actor,
        "config_change",
        &key,
        json!({ "to": value }),
        context,
        metrics,
        evolution,
    );
    Ok(json!({ "config": serde_json::to_value(&cfg).unwrap_or_default(), "applied": false }))
}

/// 键值应用（route_config_set 子步骤）：不支持键 → Err；值解析失败 → Ok(false)。
fn apply_config_set(
    cfg: &mut crate::config::Config,
    key: &str,
    value: &str,
) -> Result<bool, (String, String)> {
    use crate::config::ProviderProfile;
    use crate::types::{
        CaptureScope, ClipboardMode, ConflictPolicy, ModelhubSource, Platform, ThemeMode,
    };
    fn parse_enum<T: serde::de::DeserializeOwned>(v: &str) -> Result<T, serde_json::Error> {
        serde_json::from_value(json!(v))
    }
    Ok(match key {
        "ui.theme" => parse_enum(value).map(|t: ThemeMode| cfg.ui.theme = t).is_ok(),
        "ui.habit_dialect" => {
            parse_enum(value).map(|p: Platform| cfg.ui.habit_dialect = p).is_ok()
        }
        "ui.clipboard" => {
            parse_enum(value).map(|c: ClipboardMode| cfg.ui.clipboard = c).is_ok()
        }
        "ai.default_provider" if !value.is_empty() => {
            cfg.ai.default_provider = value.to_string();
            true
        }
        "ai.fallback_local" => value
            .parse::<bool>()
            .map(|b| cfg.ai.fallback_local = b)
            .is_ok(),
        "providers" => {
            // 数组整体替换（交互向导的 provider 表单与 CLI 直改共用）
            match serde_json::from_str::<Vec<ProviderProfile>>(value) {
                Ok(list) => {
                    cfg.providers = list;
                    true
                }
                Err(_) => false,
            }
        }
        "modelhub.source" => {
            parse_enum(value).map(|s: ModelhubSource| cfg.modelhub.source = s).is_ok()
        }
        "rollback.enabled" => value.parse::<bool>().map(|b| cfg.rollback.enabled = b).is_ok(),
        "rollback.capture_scope" => {
            parse_enum(value).map(|s: CaptureScope| cfg.rollback.capture_scope = s).is_ok()
        }
        "rollback.max_entries" => value
            .parse::<u32>()
            .map(|n| cfg.rollback.max_entries = n)
            .is_ok(),
        "rollback.on_conflict" => {
            parse_enum(value).map(|p: ConflictPolicy| cfg.rollback.on_conflict = p).is_ok()
        }
        "rollback.retention_days" => value
            .parse::<u32>()
            .map(|n| cfg.rollback.retention_days = n)
            .is_ok(),
        "rollback.retention_max_bytes" => value
            .parse::<u64>()
            .map(|n| cfg.rollback.retention_max_bytes = n)
            .is_ok(),
        "history.retention_days" => value
            .parse::<u32>()
            .map(|n| cfg.history.retention_days = n)
            .is_ok(),
        "history.retention_max_bytes" => value
            .parse::<u64>()
            .map(|n| cfg.history.retention_max_bytes = n)
            .is_ok(),
        "render.output_detection" => value
            .parse::<bool>()
            .map(|b| cfg.render.output_detection = b)
            .is_ok(),
        "exec.timeout_ms" => value.parse::<u64>().map(|n| cfg.exec.timeout_ms = n).is_ok(),
        "exec.stdout_summary_max" => value
            .parse::<usize>()
            .map(|n| cfg.exec.stdout_summary_max = n)
            .is_ok(),
        "exec.stderr_summary_max" => value
            .parse::<usize>()
            .map(|n| cfg.exec.stderr_summary_max = n)
            .is_ok(),
        "" => false,
        other => {
            return Err((
                codes::CONFIG_INVALID.into(),
                format!(
                    "不支持的配置键 {other}（支持 ui.theme/habit_dialect/clipboard、ai.default_provider/fallback_local、providers、modelhub.source、rollback.*、history.*、render.output_detection、exec.*；或运行 iac config 进入交互向导）"
                ),
            ))
        }
    })
}

/// iac_builtin：`ia-` 内置命令统一路由（Q28/Q30；GUI 与 CLI 共用映射，单一来源）。
/// ia-help/ia-theme/ia-exit 的本地交互语义由前端自理，此处处理 IPC 型名单。
fn route_builtin(
    params: &Value,
    context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> Result<Value, (String, String)> {
    use crate::builtin;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let args: Vec<String> = params
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if builtin::lookup(&name).is_none() {
        return Err((
            codes::BUILTIN_UNKNOWN.into(),
            format!("未知内置命令 {name}。可用名单：\n{}", builtin::help_text()),
        ));
    }
    let mut call = |method: &str, p: Value| route(method, &p, context, metrics, evolution);
    let text_arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let out = match name.as_str() {
        "ia-help" => Ok(json!({"help_text": builtin::help_text()})),
        "ia-config" => match args.first().map(String::as_str) {
            Some("set") if args.len() >= 3 => {
                call("iac_config_set", json!({"key": args[1], "value": args[2]}))
            }
            _ => call("iac_config_get", json!({})),
        },
        "ia-model" => match args.first().map(String::as_str) {
            Some("use") if args.len() >= 2 => call(
                "iac_config_set",
                json!({"key": "ai.default_provider", "value": args[1]}),
            ),
            _ => call("iac_config_get", json!({})),
        },
        "ia-daemon" => match args.first().map(String::as_str) {
            Some("stop") => call("iac_daemon_stop", json!({})),
            // 状态：经同链路 iac_ping 实测可达性（daemon 组合根会在信封层叠加 pid/uptime）
            _ => call("iac_ping", json!({})),
        },
        "ia-history" => match args.first().map(String::as_str) {
            Some("clean") => call(
                "iac_history_clean",
                json!({"dry_run": args.iter().any(|a| a == "--dry-run" || a == "-n")}),
            ),
            Some("export") if args.len() >= 2 => {
                call("iac_history_export", json!({"path": text_arg(1)}))
            }
            _ => call("iac_history_query", json!({"limit": 20})),
        },
        "ia-recipe" => match args.first().map(String::as_str) {
            Some("search") if args.len() >= 2 => {
                call("iac_recipes_search", json!({"q": text_arg(1), "limit": 20}))
            }
            Some("show") if args.len() >= 2 => {
                call("iac_recipes_search", json!({"q": text_arg(1), "limit": 5}))
            }
            Some("rm") if args.len() >= 2 => {
                call("iac_recipe_delete", json!({"name": text_arg(1)}))
            }
            Some("export") if args.len() >= 2 => {
                call("iac_recipe_export", json!({"path": text_arg(1)}))
            }
            Some("save") => Err((
                codes::UNKNOWN_METHOD.into(),
                "ia-recipe save 需要多字段参数，请用 CLI：iac recipe save 或 GUI 表单".into(),
            )),
            _ => call("iac_recipes_search", json!({"limit": 20})),
        },
        "ia-sessions" => match args.first().map(String::as_str) {
            Some("resume") if args.len() >= 2 => {
                call("iac_session_resume", json!({"session_id": text_arg(1)}))
            }
            _ => call("iac_session_list", json!({"limit": 20})),
        },
        "ia-rollback" => match args.first().map(String::as_str) {
            Some("replay") => call(
                "iac_rollback",
                json!({"action": "replay", "range": {"to": args.get(1).and_then(|s| s.parse::<i64>().ok())}, "on_conflict": "ask"}),
            ),
            _ => call(
                "iac_rollback",
                json!({"action": "list", "filter": {"limit": 50}}),
            ),
        },
        "ia-file-history" => match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some(path), Some("restore")) => call(
                "iac_rollback",
                json!({"action": "restore", "path": path, "seq": args.get(2).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)}),
            ),
            (Some(path), _) => call(
                "iac_rollback",
                json!({"action": "file_history", "path": path}),
            ),
            _ => Ok(
                json!({"usage": builtin::lookup("ia-file-history").map(|b| b.usage).unwrap_or_default()}),
            ),
        },
        "ia-script" => {
            // 武装态与确认词随行内旗标透传（-y/--yes/--confirm-word <词>），
            // 不预置 confirm_dangerous——闸门修复后此处自动武装会绕过 dangerous 确认
            let confirm_dangerous = args.iter().any(|a| a == "-y" || a == "--yes");
            let confirm_word = args
                .iter()
                .position(|a| a == "--confirm-word")
                .and_then(|i| args.get(i + 1))
                .cloned();
            // 其余词元 = 脚本位置参数（ScriptRunInput.args；控制旗标及其值不入参）。
            // run 时跳过前两个词元（子命令、脚本名）从第 3 个起收集
            let mut script_args: Vec<String> = Vec::new();
            let mut i = if args.first().map(String::as_str) == Some("run") {
                2
            } else {
                0
            };
            while i < args.len() {
                let a = &args[i];
                if a == "-y" || a == "--yes" {
                    i += 1;
                } else if a == "--confirm-word" {
                    i += 2; // 旗标 + 值
                } else {
                    script_args.push(a.clone());
                    i += 1;
                }
            }
            match args.first().map(String::as_str) {
                Some("run") if args.len() >= 2 => call(
                    "iac_script",
                    json!({
                        "action": "run",
                        "name": text_arg(1).trim_start_matches('@'),
                        "args": script_args,
                        "confirm_dangerous": confirm_dangerous,
                        "confirm_word": confirm_word,
                    }),
                ),
                Some("refine") if args.len() >= 2 => call(
                    "iac_script",
                    json!({"action": "refine", "name": text_arg(1)}),
                ),
                _ => Err((
                    codes::UNKNOWN_METHOD.into(),
                    "ia-script make 需要行号/任务段语义，经 CLI：iac script make <名> --segment last"
                        .into(),
                )),
            }
        }
        other => Err((
            codes::BUILTIN_UNKNOWN.into(),
            format!("内置命令 {other} 无 IPC 形态（前端本地处理）"),
        )),
    }?;
    Ok(json!({ "builtin": name, "result": out }))
}

/// 编排失败 → (code, message)。
fn fail(evolution: &Evolution) -> Result<Value, (String, String)> {
    Err((
        evolution
            .reason_code
            .clone()
            .unwrap_or_else(|| codes::STORAGE.into()),
        evolution.reason.clone().unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_config_set(key: &str, value: &str) -> Result<Value, (String, String)> {
        let session = crate::framework::SessionCtx {
            session_id: "s-test".into(),
            request_id: "r-test".into(),
            actor: crate::framework::Actor::Human,
        };
        let deps = crate::framework::Deps::default();
        let locale = crate::framework::Locale::default();
        let cfg = crate::config::Config::defaults();
        let context = crate::framework::Context {
            session: &session,
            config: &cfg,
            deps: &deps,
            locale: &locale,
        };
        let (mut metrics, mut evolution) = (Metrics::default(), Evolution::default());
        route_config_set(
            &json!({"key": key, "value": value}),
            &context,
            &mut metrics,
            &mut evolution,
        )
    }

    fn applied_config(resp: &Result<Value, (String, String)>) -> crate::config::Config {
        let v = resp.as_ref().expect("ok");
        serde_json::from_value(v.get("config").cloned().unwrap_or(Value::Null))
            .expect("config 反序列化")
    }

    #[test]
    fn config_set_covers_all_scalar_keys() {
        // 枚举 / 布尔 / 数值 / 文本各抽一键走全链路（计算+校验+审计不炸）
        let cfg = applied_config(&call_config_set("ui.theme", "night"));
        assert_eq!(cfg.ui.theme, crate::types::ThemeMode::Night);
        let cfg = applied_config(&call_config_set("ui.clipboard", "off"));
        assert_eq!(cfg.ui.clipboard, crate::types::ClipboardMode::Off);
        let cfg = applied_config(&call_config_set("ai.fallback_local", "true"));
        assert!(cfg.ai.fallback_local);
        let cfg = applied_config(&call_config_set("modelhub.source", "hf-mirror"));
        assert_eq!(cfg.modelhub.source, crate::types::ModelhubSource::HfMirror);
        let cfg = applied_config(&call_config_set("rollback.on_conflict", "skip"));
        assert_eq!(cfg.rollback.on_conflict, crate::types::ConflictPolicy::Skip);
        let cfg = applied_config(&call_config_set("rollback.max_entries", "2048"));
        assert_eq!(cfg.rollback.max_entries, 2048);
        let cfg = applied_config(&call_config_set("history.retention_days", "30"));
        assert_eq!(cfg.history.retention_days, 30);
        let cfg = applied_config(&call_config_set("exec.timeout_ms", "30000"));
        assert_eq!(cfg.exec.timeout_ms, 30_000);
        let cfg = applied_config(&call_config_set("exec.stdout_summary_max", "1024"));
        assert_eq!(cfg.exec.stdout_summary_max, 1024);
    }

    #[test]
    fn config_set_rejects_bad_values_and_unknown_keys() {
        let err = call_config_set("ui.theme", "紫色").expect_err("非法枚举须失败");
        assert_eq!(err.0, codes::CONFIG_INVALID);
        let err = call_config_set("rollback.max_entries", "很大").expect_err("非法数值须失败");
        assert_eq!(err.0, codes::CONFIG_INVALID);
        let err = call_config_set("ai.default_provider", "").expect_err("空 provider 须失败");
        assert_eq!(err.0, codes::CONFIG_INVALID);
        let err = call_config_set("no.such_key", "1").expect_err("未知键须失败");
        assert!(
            err.1.contains("不支持的配置键"),
            "错误信息指明键名：{}",
            err.1
        );
    }

    #[test]
    fn config_set_providers_array_replaces_whole_list() {
        let body = r#"[{"name":"remote","kind":"Openai","base_url":"https://api.example.com","api_key_env":"OPENAI_KEY","model":"gpt-x"},{"name":"local","kind":"local-plugin"}]"#;
        let cfg = applied_config(&call_config_set("providers", body));
        assert_eq!(cfg.providers.len(), 2);
        assert_eq!(cfg.providers[0].name, "remote");
        assert_eq!(
            cfg.providers[1].kind,
            crate::types::ProviderKind::LocalPlugin
        );
        // 结构校验兜底：远端缺 api_key_env 被拒（validate 报错）
        let bad = r#"[{"name":"remote","kind":"Openai","base_url":"https://api.example.com","model":"gpt-x"}]"#;
        let err = call_config_set("providers", bad).expect_err("远端档案缺字段须失败");
        assert!(err.1.contains("api_key_env"), "指明缺失字段：{}", err.1);
        // 非法 JSON
        let err = call_config_set("providers", "[{").expect_err("坏 JSON 须失败");
        assert_eq!(err.0, codes::CONFIG_INVALID);
    }
}
