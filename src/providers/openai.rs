use std::env;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::base::{ChatMessage, Provider, ProviderError, ProviderResponse};

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
    endpoint: String,
}

impl OpenAiProvider {
    pub fn new(model: String) -> Result<Self> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|_| ProviderError::MissingApiKey("OPENAI_API_KEY"))?;
        let endpoint = env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

        Ok(Self {
            client: Client::new(),
            api_key,
            model,
            endpoint,
        })
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn complete(&self, system: &str, messages: Vec<ChatMessage>) -> Result<ProviderResponse> {
        let mut payload_messages = vec![json!({ "role": "system", "content": system })];
        payload_messages.extend(messages.into_iter().map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
            })
        }));

        let body = json!({
            "model": self.model,
            "messages": payload_messages,
            "response_format": { "type": "json_object" },
            "temperature": 0.1
        });

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("failed to call OpenAI endpoint {}", self.endpoint))?;

        let status = response.status();
        let value: Value = response.json().await.context("failed to decode OpenAI response body")?;

        if !status.is_success() {
            return Err(anyhow!("OpenAI request failed with status {}: {}", status, value));
        }

        let text = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse(value.to_string()))?
            .to_string();
        let finish_reason = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&self.model)
            .to_string();

        Ok(ProviderResponse {
            text,
            model,
            finish_reason,
            provider: self.name().to_string(),
        })
    }

    fn name(&self) -> &str {
        "openai"
    }
}
