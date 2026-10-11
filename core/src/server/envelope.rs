//! IPC/MCP 共享信封（_06 external 第 1 节）：JSONL `{v,id,method,params}` → `{v,id,ok,result|error}`。

use crate::error::IacError;
use serde::{Deserialize, Serialize};

/// 请求信封。
#[derive(Debug, Clone, Deserialize)]
pub struct IpcRequest {
    pub v: u32,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// 发起方标识（agent:<client>）；缺省 human。字段只增演进（_06 同步随 P8）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

/// 响应信封。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcResponse {
    pub v: u32,
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IacError>,
}

impl IpcResponse {
    pub fn success(req: &IpcRequest, result: serde_json::Value) -> Self {
        Self { v: 1, id: req.id.clone(), ok: true, result: Some(result), error: None }
    }

    pub fn failure(req: &IpcRequest, code: &str, message: impl Into<String>) -> Self {
        Self {
            v: 1,
            id: req.id.clone(),
            ok: false,
            result: None,
            error: Some(IacError::new(code, message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let req_json = r#"{"v":1,"id":"r1","method":"iac_ask","params":{"intent":"ls"}}"#;
        let req: IpcRequest = serde_json::from_str(req_json).unwrap();
        assert_eq!(req.method, "iac_ask");
        let resp = IpcResponse::success(&req, serde_json::json!({"ok": true}));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(!s.contains("\"error\""), "None 字段应省略");
    }
}
