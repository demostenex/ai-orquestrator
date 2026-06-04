use std::env;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::base::{ChatMessage, Provider, ProviderError, ProviderResponse};

pub struct GeminiProvider {
    client: Client,
    api_key: String,
    model: String,
    endpoint: String,
}

impl GeminiProvider {
    pub fn new(model: String) -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey("GEMINI_API_KEY"))?;
        let endpoint = env::var("GEMINI_BASE_URL").unwrap_or_else(|_| {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
            )
        });

        Ok(Self {
            client: Client::new(),
            api_key,
            model,
            endpoint,
        })
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn complete(&self, system: &str, messages: Vec<ChatMessage>) -> Result<ProviderResponse> {
        let combined_prompt = messages
            .into_iter()
            .map(|message| format!("[{}]\n{}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n\n");

        let body = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": combined_prompt }]
            }],
            "systemInstruction": {
                "parts": [{ "text": system }]
            },
            "generationConfig": {
                "responseMimeType": "application/json",
                "temperature": 0.1
            }
        });

        let response = self
            .client
            .post(&self.endpoint)
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .with_context(|| format!("failed to call Gemini endpoint {}", self.endpoint))?;

        let status = response.status();
        let value: Value = response
            .json()
            .await
            .context("failed to decode Gemini response body")?;

        if !status.is_success() {
            return Err(anyhow!(
                "Gemini request failed with status {}: {}",
                status,
                value
            ));
        }

        let text = value
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse(value.to_string()))?
            .to_string();
        let finish_reason = value
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        Ok(ProviderResponse {
            text,
            model: self.model.clone(),
            finish_reason,
            provider: self.name().to_string(),
        })
    }

    fn name(&self) -> &str {
        "gemini"
    }
}
