use std::env;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::base::{ChatMessage, Provider, ProviderError, ProviderResponse};

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    endpoint: String,
}

impl AnthropicProvider {
    pub fn new(model: String) -> Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey("ANTHROPIC_API_KEY"))?;
        let endpoint = env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com/v1/messages".to_string());

        Ok(Self {
            client: Client::new(),
            api_key,
            model,
            endpoint,
        })
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete(&self, system: &str, messages: Vec<ChatMessage>) -> Result<ProviderResponse> {
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": messages
                .into_iter()
                .map(|message| json!({ "role": message.role, "content": message.content }))
                .collect::<Vec<_>>()
        });

        let response = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("failed to call Anthropic endpoint {}", self.endpoint))?;

        let status = response.status();
        let value: Value = response
            .json()
            .await
            .context("failed to decode Anthropic response body")?;

        if !status.is_success() {
            return Err(anyhow!("Anthropic request failed with status {}: {}", status, value));
        }

        let text = value
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse(value.to_string()))?
            .to_string();
        let finish_reason = value
            .get("stop_reason")
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
        "anthropic"
    }
}
