//! T-006 AI Adapter 测试：本地 mock HTTP server 覆盖 OpenAI/Anthropic 形态与错误路径。

use core::config::ProviderProfile;
use core::types::ProviderKind;
use core::framework::{Evolution, Metrics, TestEnv};
use core::ports::{AiRequest, LlmResponse};
use infra::ai::ai_complete;
use std::io::{Read, Write};
use std::net::TcpListener;

fn provider(kind: ProviderKind, base: String, key_env: &str) -> ProviderProfile {
    ProviderProfile {
        name: "test".into(),
        kind,
        base_url: Some(base),
        api_key_env: Some(key_env.into()),
        model: Some("test-model".into()),
        path: None,
    }
}

fn req() -> AiRequest {
    AiRequest {
        system: "你是 iA-console".into(),
        messages: vec![("user".into(), "ls 是什么".into())],
        temperature: 0.2,
        timeout_ms: 5_000,
    }
}

/// 单请求 mock server：读一个 HTTP 请求，回给定状态与 body（参数形如 "200 OK\r\n\r\n{json}"）。
fn serve_once(response: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let (status, body) = response.split_once("\r\n\r\n").unwrap_or((response, ""));
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn run(p: &ProviderProfile) -> (bool, LlmResponse, Evolution) {
    let env = TestEnv::new();
    let mut out = LlmResponse::default();
    let mut evo = Evolution::default();
    let ok = ai_complete(p, &req(), &mut out, &env.context(), &mut Metrics::default(), &mut evo);
    (ok, out, evo)
}

#[test]
fn openai_shape_success() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let base = serve_once("200 OK\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"列出文件\"}}],\"model\":\"test-model\"}");
    let (ok, out, evo) = run(&provider(ProviderKind::Openai, base, "IAC_TEST_KEY"));
    assert!(ok);
    assert!(evo.ok);
    assert_eq!(out.text, "列出文件");
    assert_eq!(out.model, "test-model");
}

#[test]
fn anthropic_shape_success() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let base = serve_once("200 OK\r\n\r\n{\"content\":[{\"text\":\"你好\"}],\"model\":\"claude-x\"}");
    let (ok, out, _) = run(&provider(ProviderKind::Anthropic, base, "IAC_TEST_KEY"));
    assert!(ok);
    assert_eq!(out.text, "你好");
    assert_eq!(out.model, "claude-x");
}

#[test]
fn server_error_maps_to_1101() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let base = serve_once("500 Internal Server Error\r\n\r\n{\"error\":\"boom\"}");
    let (ok, _, evo) = run(&provider(ProviderKind::Openai, base, "IAC_TEST_KEY"));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
}

#[test]
fn malformed_json_maps_to_1101() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let base = serve_once("200 OK\r\n\r\nnot-json");
    let (ok, _, evo) = run(&provider(ProviderKind::Openai, base, "IAC_TEST_KEY"));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
    assert!(evo.reason.as_deref().unwrap().contains("响应解析失败"));
}

#[test]
fn missing_key_env_maps_to_5102() {
    // IAC_TEST_MISSING_KEY 故意不设置
    let (ok, _, evo) = run(&provider(ProviderKind::Openai, "http://127.0.0.1:1".into(), "IAC_TEST_MISSING_KEY"));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-5102"));
}

#[test]
fn unreachable_backend_maps_to_1101() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let (ok, _, evo) = run(&provider(ProviderKind::Openai, "http://127.0.0.1:9".into(), "IAC_TEST_KEY"));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
}
