//! HTTP 共享 Client（infra 内部复用）：进程级连接池，避免每次调用重建 TLS/池。
//! 超时策略：共享 Client 不设总超时；调用方按请求用 `RequestBuilder::timeout`（AI），
//! 或经 `download_client` 拿「连接/读分时超时、无总超时」的下载专用档（大文件）。

use std::time::Duration;

static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();

/// 共享 Client（连接池复用；构建失败极罕见，缓存重试语义由调用方决定）。
pub fn shared_client() -> Result<&'static reqwest::blocking::Client, String> {
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    let _ = CLIENT.set(c);
    Ok(CLIENT.get().expect("client just set"))
}

/// 下载专用 Client：无总超时（大文件不因体积超限），连接/传输分时超时防假死。
/// blocking builder 无 read_timeout，用 socket 层 TCP_USER_TIMEOUT（60s 无确认传输即断）
/// 达成等价效果；服务端慢速但持续发送属正常进度，不拦截。
pub fn download_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .tcp_user_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_client_is_same_instance() {
        let a = shared_client().unwrap();
        let b = shared_client().unwrap();
        assert!(std::ptr::eq(a, b));
    }
}
