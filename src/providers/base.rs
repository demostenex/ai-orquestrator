use anyhow::Result;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub text: String,
    pub model: String,
    pub finish_reason: String,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("missing environment variable {0}")]
    MissingApiKey(&'static str),
    #[error("provider returned an invalid response: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, system: &str, messages: Vec<ChatMessage>) -> Result<ProviderResponse>;
    fn name(&self) -> &str;
}
