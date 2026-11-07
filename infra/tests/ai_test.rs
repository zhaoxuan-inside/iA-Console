//! T-006 AI Adapter 测试：本地 mock HTTP server 覆盖 OpenAI/Anthropic 形态与错误路径。

use iac_core::config::ProviderProfile;
use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{AiRequest, LlmResponse};
use iac_core::types::ProviderKind;
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
        max_tokens: None,
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
    let ok = ai_complete(
        p,
        &req(),
        &mut out,
        &env.context(),
        &mut Metrics::default(),
        &mut evo,
    );
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
    let base =
        serve_once("200 OK\r\n\r\n{\"content\":[{\"text\":\"你好\"}],\"model\":\"claude-x\"}");
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
    let (ok, _, evo) = run(&provider(
        ProviderKind::Openai,
        "http://127.0.0.1:1".into(),
        "IAC_TEST_MISSING_KEY",
    ));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-5102"));
}

#[test]
fn unreachable_backend_maps_to_1101() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let (ok, _, evo) = run(&provider(
        ProviderKind::Openai,
        "http://127.0.0.1:9".into(),
        "IAC_TEST_KEY",
    ));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1101"));
}

/// 捕获请求体的 mock server（断言请求形状用）。
fn serve_capture(response: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let cap = captured.clone();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = vec![0u8; 65536];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let body = req
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            *cap.lock().unwrap() = body;
            let (status, resp_body) = response.split_once("\r\n\r\n").unwrap_or((response, ""));
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                resp_body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (format!("http://{addr}"), captured)
}

#[test]
fn malformed_key_fails_gracefully_not_panic() {
    // 密钥含换行：此前 HeaderValue::from_str().expect() 会 panic，现应转译 IAC-5102
    std::env::set_var("IAC_TEST_BAD_KEY", "key\nwith-newline");
    let (ok, _, evo) = run(&provider(
        ProviderKind::Anthropic,
        "http://127.0.0.1:1".into(),
        "IAC_TEST_BAD_KEY",
    ));
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-5102"));
    assert!(evo.reason.as_deref().unwrap().contains("非法字符"));
}

#[test]
fn empty_system_is_not_sent() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let base = serve_once(
        "200 OK\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"ok\"}}],\"model\":\"m\"}",
    );
    let p = provider(ProviderKind::Openai, base, "IAC_TEST_KEY");
    let env = TestEnv::new();
    let input = AiRequest {
        system: String::new(),
        messages: vec![("user".into(), "hi".into())],
        temperature: 0.2,
        timeout_ms: 5_000,
    };
    let mut out = LlmResponse::default();
    let mut evo = Evolution::default();
    assert!(ai_complete(
        &p,
        &input,
        &mut out,
        &env.context(),
        &mut Metrics::default(),
        &mut evo
    ));
    // 请求体不含 system 消息（serve_once 不回显请求，这里用捕获版再验一次）
    let (base2, captured) = serve_capture(
        "200 OK\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"ok\"}}],\"model\":\"m\"}",
    );
    let p2 = provider(ProviderKind::Openai, base2, "IAC_TEST_KEY");
    let mut out2 = LlmResponse::default();
    let mut evo2 = Evolution::default();
    assert!(ai_complete(
        &p2,
        &input,
        &mut out2,
        &env.context(),
        &mut Metrics::default(),
        &mut evo2
    ));
    let body = captured.lock().unwrap().clone();
    assert!(
        !body.contains("\"role\":\"system\""),
        "空 system 不应下发: {body}"
    );
}

#[test]
fn anthropic_max_tokens_from_profile() {
    std::env::set_var("IAC_TEST_KEY", "test-key-123");
    let (base, captured) =
        serve_capture("200 OK\r\n\r\n{\"content\":[{\"text\":\"你好\"}],\"model\":\"claude-x\"}");
    let mut p = provider(ProviderKind::Anthropic, base, "IAC_TEST_KEY");
    p.max_tokens = Some(777);
    let env = TestEnv::new();
    let mut out = LlmResponse::default();
    let mut evo = Evolution::default();
    assert!(ai_complete(
        &p,
        &req(),
        &mut out,
        &env.context(),
        &mut Metrics::default(),
        &mut evo
    ));
    let body = captured.lock().unwrap().clone();
    assert!(
        body.contains("\"max_tokens\":777"),
        "档案 max_tokens 应透传: {body}"
    );
}
