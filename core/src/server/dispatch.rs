//! I-030：唯一路由入口——方法名 → 编排方法，信封错误码统一转译。
//! 人类（TUI/CLI）与 Agent（MCP）经同一 dispatch，闸门与审计行为一致（AC-08）。

use crate::error::codes;
use crate::executor::handle_execute;
use crate::framework::{Context, Evolution, Metrics};
use crate::history::query_history;

use crate::recipe::{search_recipes, RecipeFilter};
use crate::server::envelope::{IpcRequest, IpcResponse};
use crate::session::get_or_create_current;
use crate::{doctor, executor, history, intent, recipe, session};
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
        *output = IpcResponse::failure(input, codes::PROTOCOL_MISMATCH, format!("协议版本不支持: v{}", input.v));
        return true;
    }
    let params = input.params.clone();
    let result = route(input.method.as_str(), &params, context, metrics, evolution);
    *output = match result {
        Ok(v) => IpcResponse::success(input, v),
        Err((code, msg)) if code.as_str() == codes::DAEMON_STOP => IpcResponse::failure(input, &code, msg),
        Err((code, msg)) => {
            if evolution.reason_code.is_none() {
                evolution.fail(&code, msg.clone());
            }
            IpcResponse::failure(input, evolution.reason_code.as_deref().unwrap_or(&code), msg)
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
        "iac_ping" => Ok(json!({"pong": true})),
        "iac_ask" => route_ask(params, context, metrics, evolution),
        "iac_execute" => route_execute(params, context, metrics, evolution),
        "iac_explain" => route_explain(params, context, metrics, evolution),
        "iac_diagnose" => route_diagnose(params, context, metrics, evolution),
        "iac_history_query" => route_history(params, context, metrics, evolution),
        "iac_recipes_search" => route_recipes_search(params, context, metrics, evolution),
        "iac_recipe_save" => route_recipe_save(params, context, metrics, evolution),
        "iac_translate" => route_translate(params, context, metrics, evolution),
        "iac_sessions_current" => route_sessions_current(params, context, metrics, evolution),
        "iac_rollback" => route_rollback(params, context, metrics, evolution),
        "iac_script" => route_script(params, context, metrics, evolution),
        "iac_config_get" => Ok(serde_json::to_value(context.config).unwrap_or_default()),
        "iac_daemon_stop" => Err((codes::DAEMON_STOP.to_string(), "停机指令已受理（由 daemon 入口处理）".to_string())),
        other => Err((codes::UNKNOWN_METHOD.to_string(), format!("未知方法 {other}，可用方法见 _06 external 第 2.2 节"))),
    }
}


/// iac_ask：意图 → 提案。
fn route_ask(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: intent::AskInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = intent::Proposal {
        id: String::new(),
        session_id: String::new(),
        intent_text: String::new(),
        command: String::new(),
        argv: vec![],
        explanation: Default::default(),
        risk_level: crate::types::RiskLevel::Safe,
        platform: crate::types::Platform::Linux,
        verified: false,
        status: crate::intent::ProposalStatus::Proposed,
        created_at: 0,
    };
    if !intent::handle_ask(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-1102".to_string(), format!("提案序列化失败: {e}")))
}

/// iac_execute：闸门执行（Agent 路径经同一闸门，AC-08）。
fn route_execute(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let mut input: executor::ExecuteInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    if matches!(context.session.actor, crate::framework::Actor::Agent(_)) && input.origin.is_none() {
        input.origin = Some(executor::CommandOrigin::Translated); // Agent 发起视同翻译路径（恒确认）
    }
    let mut out = executor::ExecutionOutcome {
        execution_id: String::new(),
        exit_code: None,
        status: crate::history::ExecStatus::Ok,
        stdout_summary: None,
        stderr_summary: None,
        duration_ms: None,
        diagnosis: None,
        gate: Default::default(),
    };
    if !handle_execute(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-1102".to_string(), format!("结果序列化失败: {e}")))
}

/// iac_explain：参数解释。
fn route_explain(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: intent::ExplainInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = intent::ParamSpecDoc::default();
    if !intent::handle_explain(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-1102".to_string(), format!("序列化失败: {e}")))
}

/// iac_diagnose：错误诊断。
fn route_diagnose(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: doctor::DiagnoseInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = doctor::Diagnosis {
        root_cause: String::new(),
        evidence: vec![],
        confidence: 0.0,
        fix_proposals: vec![],
        rollback_note: String::new(),
        speculation: false,
    };
    if !doctor::diagnose_failure(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-1201".into(), format!("序列化失败: {e}")))
}

/// iac_history_query：历史检索。
fn route_history(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: history::HistoryFilter = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = history::HistoryPage::default();
    if !query_history(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
}

/// iac_recipes_search：方案检索。
fn route_recipes_search(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: RecipeFilter = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = Vec::new();
    if !search_recipes(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
}

/// iac_recipe_save：保存方案。
fn route_recipe_save(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: recipe::RecipeInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = recipe::Recipe {
        id: String::new(),
        title: String::new(),
        description: None,
        kind: "command".into(),
        name: None,
        command_template: String::new(),
        tags: vec![],
        platform: "all".into(),
        risk_level: crate::types::RiskLevel::Safe,
        usage_count: 0,
        source_execution_id: None,
        created_at: 0,
        updated_at: 0,
    };
    if !recipe::save_recipe(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
}

/// iac_translate：跨方言翻译（AC-19/20）。
fn route_translate(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: crate::knowledge::TranslationInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.to_string(), format!("参数解析失败: {e}")))?;
    let mut out = crate::knowledge::TranslationResult::native_fallback(&input);
    if !intent::translate::translate_command(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-1301".to_string(), format!("序列化失败: {e}")))
}

/// iac_sessions_current：当前会话。
fn route_sessions_current(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    let input: session::CurrentSessionInput = serde_json::from_value(params.clone())
        .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
    let mut out = session::SessionInfo {
        id: String::new(),
        title: String::new(),
        cwd: String::new(),
        shell: String::new(),
        host: String::new(),
        status: String::new(),
        created_at: 0,
        updated_at: 0,
        events: vec![],
    };
    if !get_or_create_current(&input, &mut out, context, metrics, evolution) {
        return Err((evolution.reason_code.clone().unwrap_or_default(), evolution.reason.clone().unwrap_or_default()));
    }
    serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
}

/// iac_rollback（API-012，M7）：list / replay / file_history / restore 四动作。
fn route_rollback(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    use crate::rollback;
    use crate::types::{FileVersionRef, RollbackListFilter, RollbackRequest};
    let action = params.get("action").and_then(Value::as_str).unwrap_or("list");
    let fail = |e: &Evolution| Err((e.reason_code.clone().unwrap_or_default(), e.reason.clone().unwrap_or_default()));
    match action {
        "list" => {
            let filter = params.get("filter").unwrap_or(params);
            let input: RollbackListFilter = serde_json::from_value(filter.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = Vec::new();
            if !rollback::list_rollback_points(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        "replay" => {
            let input: RollbackRequest = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = crate::types::RollbackReport::default();
            if !rollback::rollback_to(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        "file_history" => {
            let input: rollback::FilePath = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = Vec::new();
            if !rollback::file_history(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        "restore" => {
            let input: FileVersionRef = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = crate::types::RestoreReport { path: String::new(), restored_from: 0, gate: Default::default() };
            if !rollback::restore_file(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        other => Err((codes::UNKNOWN_METHOD.into(), format!("iac_rollback 未知 action: {other}"))),
    }
}

/// iac_script（API-013/014，M8）：make / refine / run 三动作。
fn route_script(params: &Value, context: &Context, metrics: &mut Metrics, evolution: &mut Evolution) -> Result<Value, (String, String)> {
    use crate::script;
    let action = params.get("action").and_then(Value::as_str).unwrap_or("make");
    let fail = |e: &Evolution| Err((e.reason_code.clone().unwrap_or_default(), e.reason.clone().unwrap_or_default()));
    match action {
        "make" => {
            let input: script::ScriptMakeInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = script::ScriptMakeOutput::default();
            if !script::make_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        "refine" => {
            let input: script::ScriptRefineInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = script::ScriptRefineOutput::default();
            if !script::refine_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        "run" => {
            let input: script::ScriptRunInput = serde_json::from_value(params.clone())
                .map_err(|e| (codes::UNKNOWN_METHOD.into(), format!("参数解析失败: {e}")))?;
            let mut out = crate::executor::ExecutionOutcome {
                execution_id: String::new(), exit_code: None, status: crate::history::ExecStatus::Ok,
                stdout_summary: None, stderr_summary: None, duration_ms: None, diagnosis: None,
                gate: Default::default(),
            };
            if !script::run_script(&input, &mut out, context, metrics, evolution) {
                return fail(evolution);
            }
            serde_json::to_value(&out).map_err(|e| ("IAC-3102".into(), format!("序列化失败: {e}")))
        }
        other => Err((codes::UNKNOWN_METHOD.into(), format!("iac_script 未知 action: {other}"))),
    }
}
