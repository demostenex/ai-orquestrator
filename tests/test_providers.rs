mod common;

use std::env;

use ai_orchestrator::providers::base::{ChatMessage, Provider};
use ai_orchestrator::providers::{
    anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAiProvider,
};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── OpenAI ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn openai_complete_returns_parsed_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "gpt-4o",
            "choices": [{
                "message": { "content": "{\"approved\": true}" },
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    env::set_var("OPENAI_API_KEY", "test-key");
    env::set_var(
        "OPENAI_BASE_URL",
        format!("{}/v1/chat/completions", server.uri()),
    );

    let provider = OpenAiProvider::new("gpt-4o".to_string()).unwrap();
    let result = provider
        .complete(
            "system",
            vec![ChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
        )
        .await
        .unwrap();

    assert_eq!(result.text, "{\"approved\": true}");
    assert_eq!(result.finish_reason, "stop");
    assert_eq!(result.provider, "openai");
}

#[tokio::test]
#[serial]
async fn openai_complete_propagates_api_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "Invalid API key" }
        })))
        .mount(&server)
        .await;

    env::set_var("OPENAI_API_KEY", "bad-key");
    env::set_var(
        "OPENAI_BASE_URL",
        format!("{}/v1/chat/completions", server.uri()),
    );

    let provider = OpenAiProvider::new("gpt-4o".to_string()).unwrap();
    let result = provider
        .complete(
            "system",
            vec![ChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
        )
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("401"),
        "error should mention status 401, got: {err}"
    );
}

// ── Gemini ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn gemini_complete_returns_parsed_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{ "text": "{\"step_id\": \"001\"}" }]
                },
                "finishReason": "STOP"
            }]
        })))
        .mount(&server)
        .await;

    env::set_var("GEMINI_API_KEY", "test-key");
    env::set_var("GEMINI_BASE_URL", server.uri());

    let provider = GeminiProvider::new("gemini-2.0-flash".to_string()).unwrap();
    let result = provider
        .complete(
            "system",
            vec![ChatMessage {
                role: "user".to_string(),
                content: "prompt".to_string(),
            }],
        )
        .await
        .unwrap();

    assert_eq!(result.text, "{\"step_id\": \"001\"}");
    assert_eq!(result.provider, "gemini");
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn anthropic_complete_returns_parsed_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "{\"approved\": false}" }]
        })))
        .mount(&server)
        .await;

    env::set_var("ANTHROPIC_API_KEY", "test-key");
    env::set_var("ANTHROPIC_BASE_URL", server.uri());

    let provider = AnthropicProvider::new("claude-3-5-sonnet-20241022".to_string()).unwrap();
    let result = provider
        .complete(
            "system",
            vec![ChatMessage {
                role: "user".to_string(),
                content: "prompt".to_string(),
            }],
        )
        .await
        .unwrap();

    assert_eq!(result.text, "{\"approved\": false}");
    assert_eq!(result.provider, "anthropic");
}

// ── API key ausente ───────────────────────────────────────────────────────────

#[test]
#[serial]
fn openai_missing_api_key_returns_error() {
    env::remove_var("OPENAI_API_KEY");
    let result = OpenAiProvider::new("gpt-4o".to_string());
    assert!(result.is_err());
    let err = result.err().unwrap().to_string();
    assert!(err.contains("OPENAI_API_KEY"), "got: {err}");
}

#[test]
#[serial]
fn gemini_missing_api_key_returns_error() {
    env::remove_var("GEMINI_API_KEY");
    let result = GeminiProvider::new("gemini-2.0-flash".to_string());
    assert!(result.is_err());
    let err = result.err().unwrap().to_string();
    assert!(err.contains("GEMINI_API_KEY"), "got: {err}");
}

#[test]
#[serial]
fn anthropic_missing_api_key_returns_error() {
    env::remove_var("ANTHROPIC_API_KEY");
    let result = AnthropicProvider::new("claude-3-5-sonnet-20241022".to_string());
    assert!(result.is_err());
    let err = result.err().unwrap().to_string();
    assert!(err.contains("ANTHROPIC_API_KEY"), "got: {err}");
}
