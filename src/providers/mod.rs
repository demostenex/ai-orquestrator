pub mod anthropic;
pub mod base;
pub mod gemini;
pub mod openai;

use anyhow::{anyhow, Result};

use crate::core::config::Config;

use self::anthropic::AnthropicProvider;
use self::base::Provider;
use self::gemini::GeminiProvider;
use self::openai::OpenAiProvider;

pub fn create_provider(config: &Config) -> Result<Box<dyn Provider>> {
    match config.provider.as_str() {
        "openai" => Ok(Box::new(OpenAiProvider::new(config.model.clone())?)),
        "anthropic" => Ok(Box::new(AnthropicProvider::new(config.model.clone())?)),
        "gemini" => Ok(Box::new(GeminiProvider::new(config.model.clone())?)),
        other => Err(anyhow!("unsupported provider: {other}")),
    }
}
