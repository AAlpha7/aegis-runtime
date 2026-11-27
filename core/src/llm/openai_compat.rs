use async_openai::Client;
use async_openai::config::OpenAIConfig;

use super::backend::{LlmBackend, LlmBackendFuture};
use super::types::{AegisChatRequest, AegisChatResponse, LlmProviderError, LlmProviderErrorKind};

pub struct OpenAiCompatBackend {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAiCompatBackend {
    pub fn new(client: Client<OpenAIConfig>, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }

    pub fn from_api_config(api_key: &str, api_base: &str, model: impl Into<String>) -> Self {
        Self::new(
            crate::llm_client::create_llm_client(api_key, api_base),
            model,
        )
    }

    pub fn client(&self) -> &Client<OpenAIConfig> {
        &self.client
    }
}

impl LlmBackend for OpenAiCompatBackend {
    fn chat(&self, _req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
        Box::pin(async {
            Err(LlmProviderError::new(
                LlmProviderErrorKind::UnsupportedFeature,
                "provider-neutral OpenAI-compatible chat conversion is not wired in Phase 1; existing call sites still use llm_client directly",
            ))
        })
    }

    fn provider_name(&self) -> &'static str {
        "openai_compat"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
