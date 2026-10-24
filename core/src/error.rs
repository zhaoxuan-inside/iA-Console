//! IAC 错误码体系（见 _06 external 第 4 节）。错误码只增不改义。

/// 机器可读错误码常量。命名 = 表中错误码的语义化名字。
pub mod codes {
    pub const AI_UNAVAILABLE: &str = "IAC-1101";
    pub const AI_PARSE: &str = "IAC-1102";
    pub const INTENT_INVALID: &str = "IAC-1103";
    pub const DIAGNOSE: &str = "IAC-1201";
    pub const TRANSLATE_MISS: &str = "IAC-1301";
    pub const BUILTIN_UNKNOWN: &str = "IAC-1501";
    pub const CONFIRM_TIMEOUT: &str = "IAC-2104";
    pub const EXEC_REJECTED: &str = "IAC-2105";
    pub const EXEC_NOT_FOUND: &str = "IAC-2101";
    pub const EXEC_TIMEOUT: &str = "IAC-2103";
    pub const EXEC_PLATFORM: &str = "IAC-2106";
    pub const ROLLBACK_LIMIT: &str = "IAC-1601";
    pub const ROLLBACK_MISSING: &str = "IAC-1602";
    pub const REPLAY_CONFLICT: &str = "IAC-1603";
    pub const RECIPE_NAME_CONFLICT: &str = "IAC-4101";
    pub const STORAGE: &str = "IAC-3102";
    pub const STORAGE_MIGRATE: &str = "IAC-3103";
    pub const CONFIG_INVALID: &str = "IAC-5101";
    pub const CONFIG_KEY_MISSING: &str = "IAC-5102";
    pub const CONFIG_IO: &str = "IAC-5103";
    pub const CONFIG_WRITE: &str = "IAC-5104";
    pub const PROTOCOL_MISMATCH: &str = "IAC-6102";
    pub const UNKNOWN_METHOD: &str = "IAC-6105";
    pub const DAEMON_STOP: &str = "IAC-6106";
    pub const CANCELLED: &str = "IAC-6107";
}

/// 结构化错误（信封 error 与 Evolution.reason_code 共用）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IacError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl IacError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into(), detail: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iac_error_carries_code_and_message() {
        let e = IacError::new(codes::CONFIG_INVALID, "bad key");
        assert_eq!(e.code, "IAC-5101");
        assert_eq!(e.message, "bad key");
        assert!(e.detail.is_none());
    }
}
