use super::backend::{LlmBackend, LlmBackendFuture};
use super::types::{
    AegisChatRequest, AegisChatResponse, AegisEmbedRequest, AegisEmbedResponse, LlmProviderError,
    LlmProviderErrorKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexOAuthBackendState {
    AuthRequired,
    Unavailable,
}

pub struct CodexOAuthBackend {
    model: String,
    state: CodexOAuthBackendState,
}

impl CodexOAuthBackend {
    pub fn auth_required(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            state: CodexOAuthBackendState::AuthRequired,
        }
    }

    pub fn unavailable(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            state: CodexOAuthBackendState::Unavailable,
        }
    }

    pub fn state(&self) -> CodexOAuthBackendState {
        self.state
    }
}

impl LlmBackend for CodexOAuthBackend {
    fn chat(&self, _req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
        Box::pin(async move {
            match self.state {
                CodexOAuthBackendState::AuthRequired => Err(LlmProviderError::new(
                    LlmProviderErrorKind::AuthRequired,
                    "Codex OAuth backend requires explicit owner authorization; no paid API fallback was attempted",
                )),
                CodexOAuthBackendState::Unavailable => Err(LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    "Codex OAuth backend skeleton is unavailable until a real native or sidecar integration is configured",
                )),
            }
        })
    }

    fn embed(&self, _req: AegisEmbedRequest) -> LlmBackendFuture<'_, AegisEmbedResponse> {
        Box::pin(async {
            Err(LlmProviderError::new(
                LlmProviderErrorKind::EmbeddingBlocked,
                "embeddings must never route through Codex OAuth",
            ))
        })
    }

    fn provider_name(&self) -> &'static str {
        "codex_oauth"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::{
        AegisChatRequest, AegisEmbedRequest, AegisRequestMetadata, LlmTaskClass,
    };

    fn chat_req() -> AegisChatRequest {
        AegisChatRequest {
            model: "requested".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: vec![],
            tool_choice: None,
            metadata: AegisRequestMetadata {
                task_class: LlmTaskClass::ForegroundUserRequest,
                request_id: Some("req-1".to_string()),
                chat_id: Some(42),
                user_visible: true,
                reasoning_effort: None,
            },
        }
    }

    fn embed_req() -> AegisEmbedRequest {
        AegisEmbedRequest {
            model: "embed".to_string(),
            input: vec!["hello".to_string()],
            metadata: AegisRequestMetadata {
                task_class: LlmTaskClass::Embedding,
                request_id: Some("embed-1".to_string()),
                chat_id: Some(42),
                user_visible: false,
                reasoning_effort: None,
            },
        }
    }

    #[tokio::test]
    async fn codex_oauth_skeleton_requires_auth_without_fallback() {
        let backend = CodexOAuthBackend::auth_required("codex-model");

        assert_eq!(backend.provider_name(), "codex_oauth");
        assert_eq!(backend.model_name(), "codex-model");
        assert_eq!(backend.state(), CodexOAuthBackendState::AuthRequired);

        let err = backend
            .chat(chat_req())
            .await
            .expect_err("skeleton must not make a real request");
        assert_eq!(err.kind, LlmProviderErrorKind::AuthRequired);
        assert!(err.message.contains("no paid API fallback"));
    }

    #[tokio::test]
    async fn codex_oauth_skeleton_reports_unavailable() {
        let backend = CodexOAuthBackend::unavailable("codex-model");
        let err = backend
            .chat(chat_req())
            .await
            .expect_err("unavailable skeleton must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::ProviderUnavailable);
    }

    #[tokio::test]
    async fn codex_oauth_skeleton_blocks_embeddings() {
        let backend = CodexOAuthBackend::auth_required("codex-model");
        let err = backend
            .embed(embed_req())
            .await
            .expect_err("Codex OAuth must not accept embedding requests");
        assert_eq!(err.kind, LlmProviderErrorKind::EmbeddingBlocked);
    }
}
