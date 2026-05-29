pub mod auditor;
pub mod dev;

use crate::providers::base::ProviderResponse;

#[derive(Debug, Clone)]
pub struct AgentCall<T> {
    pub parsed: T,
    pub prompt: String,
    pub provider_response: ProviderResponse,
}
