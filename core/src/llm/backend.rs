use std::future::Future;
use std::pin::Pin;

use super::types::{
    AegisChatRequest, AegisChatResponse, AegisEmbedRequest, AegisEmbedResponse, LlmProviderError,
};

pub type LlmBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, LlmProviderError>> + Send + 'a>>;

pub trait LlmBackend: Send + Sync {
    fn chat(&self, req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse>;

    fn embed(&self, _req: AegisEmbedRequest) -> LlmBackendFuture<'_, AegisEmbedResponse> {
        Box::pin(async {
            Err(LlmProviderError::unsupported_feature(
                "embedding is not supported by this LLM backend",
            ))
        })
    }

    fn provider_name(&self) -> &'static str;

    fn model_name(&self) -> &str;
}
