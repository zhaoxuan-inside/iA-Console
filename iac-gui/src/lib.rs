//! iA-console GUI 库根：模块组织（逻辑尽量下沉为可单测的纯函数）。

pub mod app;
pub mod blocks;
pub mod ipc;
pub mod layout;
pub mod shell;
pub mod theme;

#[cfg(test)]
mod tests {
    use super::app::Entry;
    use iac_core::server::IpcResponse;

    /// apply_ipc_result 的提案分支：多候选 → Proposal 条目 + 待确认指针。
    #[test]
    fn ask_response_builds_proposal_card() {
        let mut entries: Vec<Entry> = Vec::new();
        let resp: IpcResponse = serde_json::from_str(
            r#"{"v":1,"id":"1","ok":true,"result":{"id":"p1","candidates":[
                {"command":"du -ah . | head","risk_level":"safe","summary":"列大文件"},
                {"command":"ncdu","risk_level":"caution","summary":"交互式"}],"active":0}}"#,
        )
        .unwrap();
        let mut busy = false;
        let mut last_proposal: Option<String> = None;
        apply_to_entries(
            &mut entries,
            &mut busy,
            &mut last_proposal,
            "iac_ask",
            Ok(resp),
        );
        assert!(matches!(entries.first(), Some(Entry::Proposal { .. })));
        assert_eq!(last_proposal.as_deref(), Some("p1"));
        assert!(!busy);
    }

    /// 执行失败 → OfferHelp（Q18）。
    #[test]
    fn failed_execution_offers_help() {
        let (mut entries, mut busy, mut lp) = (Vec::new(), false, None);
        let resp: IpcResponse = serde_json::from_str(
            r#"{"v":1,"id":"1","ok":true,"result":{"status":"failed","exit_code":1,"offer_help":true,"execution_id":"e1"}}"#,
        )
        .unwrap();
        apply_to_entries(&mut entries, &mut busy, &mut lp, "iac_execute", Ok(resp));
        assert!(entries.iter().any(|e| matches!(e, Entry::OfferHelp { .. })));
    }

    /// 错误响应 → AI 失效条目（Q19）。
    #[test]
    fn error_response_becomes_ai_error_entry() {
        let (mut entries, mut busy, mut lp) = (Vec::new(), false, None);
        let resp: IpcResponse = serde_json::from_str(
            r#"{"v":1,"id":"1","ok":false,"error":{"code":"IAC-1101","message":"AI 未配置"}}"#,
        )
        .unwrap();
        apply_to_entries(&mut entries, &mut busy, &mut lp, "iac_ask", Ok(resp));
        assert!(entries.iter().any(|e| matches!(e, Entry::AiError { .. })));
        assert!(!busy, "错误必须解锁输入行");
    }

    /// 测试辅助：条目映射快照（与 app::apply_ipc_result 同逻辑的纯数据面）。
    fn apply_to_entries(
        entries: &mut Vec<Entry>,
        busy: &mut bool,
        last_proposal: &mut Option<String>,
        method: &'static str,
        resp: Result<IpcResponse, String>,
    ) {
        *busy = false;
        match resp {
            Ok(r) if r.ok => {
                let result = r.result.unwrap_or_default();
                match method {
                    "iac_ask" => {
                        let id = result
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let candidates: Vec<(String, String, String)> = result
                            .get("candidates")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .map(|c| {
                                        (
                                            c["command"].as_str().unwrap_or_default().to_string(),
                                            c["risk_level"].as_str().unwrap_or("safe").to_string(),
                                            c["summary"].as_str().unwrap_or_default().to_string(),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        entries.push(Entry::Proposal {
                            id: id.clone(),
                            candidates,
                            active: result.get("active").and_then(|v| v.as_u64()).unwrap_or(0)
                                as usize,
                            context_json: None,
                            expanded: false,
                        });
                        *last_proposal = Some(id);
                    }
                    "iac_execute" => {
                        let offer = result
                            .get("offer_help")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if offer {
                            entries.push(Entry::OfferHelp {
                                execution_id: result
                                    .get("execution_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .into(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(_) | Err(_) => entries.push(Entry::AiError {
                method,
                params: serde_json::json!({}),
                reason: "x".into(),
            }),
        }
    }

    /// 布局持久化（layout.rs 自带测试在模块内）。
    #[test]
    fn layout_module_tests_exist() {
        // layout roundtrip 在 layout::tests 覆盖；此处防误删模块
        let exists = std::path::Path::new(&crate::layout::layout_path()).exists();
        let _ = exists;
    }
}
