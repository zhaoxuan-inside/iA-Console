//! LLM Adapter（IF-005）：OpenAI 兼容 + Anthropic 原生双协议，Provider 档案化（ADR-006 修订）。
//! 本模块是项目唯一 HTTP 出口；第三方异常转译 IAC-1101/5102 进 evolution。

use core::config::ProviderProfile;
use core::types::ProviderKind;
use core::error::codes;
use core::framework::{Context, Evolution, Metrics};
use core::ports::LlmResponse;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// IF-005：统一补全调用。按 Provider 档案选协议，密钥经环境变量（IAC-5102）。
pub fn ai_complete(
    provider: &ProviderProfile,
    input: &core::ports::AiRequest,
    output: &mut LlmResponse,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    evolution.succeed();
    let Some(key) = api_key(provider) else {
        evolution.fail(codes::CONFIG_KEY_MISSING, format!("环境变量 {} 未设置", provider.api_key_env.as_deref().unwrap_or("<未配置 api_key_env>")));
        return false;
    };
    let client = Client::builder().timeout(Duration::from_millis(input.timeout_ms)).build();
    let Ok(client) = client else {
        evolution.fail(codes::AI_UNAVAILABLE, "HTTP 客户端构建失败");
        return false;
    };
    let (url, body, headers) = match provider.kind {
        ProviderKind::Anthropic => anthropic_payload(provider, input, &key),
        _ => openai_payload(provider, input, &key),
    };
    metrics.counter("ai.calls", 1);
    let resp = client.post(&url).json(&body).headers(headers).send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            evolution.fail(codes::AI_UNAVAILABLE, format!("AI 后端不可达: {e}"));
            return false;
        }
    };
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        evolution.fail(codes::AI_UNAVAILABLE, format!("AI 后端返回 {status}: {}", truncate(&text, 300)));
        return false;
    }
    match parse_text(&text, provider.kind) {
        Some((content, model)) => {
            output.text = content;
            output.model = model;
            true
        }
        None => {
            evolution.fail(codes::AI_UNAVAILABLE, format!("响应解析失败: {}", truncate(&text, 300)));
            false
        }
    }
}

/// 密钥只从环境变量读取，永不落盘/入日志。
fn api_key(p: &ProviderProfile) -> Option<String> {
    let name = p.api_key_env.as_deref()?;
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// OpenAI 兼容：POST {base}/chat/completions，Bearer 鉴权。
fn openai_payload(p: &ProviderProfile, input: &core::ports::AiRequest, key: &str) -> (String, Value, reqwest::header::HeaderMap) {
    let url = format!("{}/chat/completions", p.base_url.clone().unwrap_or_default());
    let mut msgs = vec![json!({"role": "system", "content": input.system})];
    for (role, content) in &input.messages {
        msgs.push(json!({"role": role, "content": content}));
    }
    let body = json!({
        "model": p.model.clone().unwrap_or_default(),
        "messages": msgs,
        "temperature": input.temperature,
    });
    (url, body, bearer(key))
}

/// Anthropic 原生：POST {base}/v1/messages，x-api-key + anthropic-version。
fn anthropic_payload(p: &ProviderProfile, input: &core::ports::AiRequest, key: &str) -> (String, Value, reqwest::header::HeaderMap) {
    let base = p.base_url.clone().unwrap_or_default();
    let url = if base.ends_with("/v1") { format!("{base}/messages") } else { format!("{base}/v1/messages") };
    let msgs: Vec<Value> = input.messages.iter().map(|(r, c)| json!({"role": r, "content": c})).collect();
    let body = json!({
        "model": p.model.clone().unwrap_or_default(),
        "system": input.system,
        "messages": msgs,
        "max_tokens": 2048,
        "temperature": input.temperature,
    });
    let mut h = reqwest::header::HeaderMap::new();
    h.insert("x-api-key", reqwest::header::HeaderValue::from_str(key).expect("invalid header key"));
    h.insert("anthropic-version", reqwest::header::HeaderValue::from_static("2023-06-01"));
    (url, body, h)
}

/// Bearer 头。
fn bearer(key: &str) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
        h.insert(reqwest::header::AUTHORIZATION, v);
    }
    h
}

/// 从两种协议响应中取文本（OpenAI choices[0].message.content / Anthropic content[0].text）。
fn parse_text(body: &str, kind: ProviderKind) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(body).ok()?;
    match kind {
        ProviderKind::Anthropic => {
            let text = v["content"][0]["text"].as_str()?.to_string();
            Some((text, v["model"].as_str().unwrap_or_default().to_string()))
        }
        _ => {
            let text = v["choices"][0]["message"]["content"].as_str()?.to_string();
            Some((text, v["model"].as_str().unwrap_or_default().to_string()))
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
