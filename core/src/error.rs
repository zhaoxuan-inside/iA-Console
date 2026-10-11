//! IAC 错误码体系（见 _06 external 第 4 节）。错误码只增不改义。

/// 机器可读错误码常量。命名 = 表中错误码的语义化名字。
pub mod codes {
    pub const CONFIG_INVALID: &str = "IAC-5101";
    pub const CONFIG_KEY_MISSING: &str = "IAC-5102";
    pub const CONFIG_IO: &str = "IAC-5103";
    pub const CONFIG_WRITE: &str = "IAC-5104";
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
