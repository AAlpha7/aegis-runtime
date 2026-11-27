//! Provider-neutral LLM routing primitives.
//!
//! This module is intentionally a skeleton in P0: existing call sites may keep
//! using `llm_client` directly while routing policy and safe defaults are
//! tested behind a small provider-neutral interface.

pub mod backend;
pub mod codex_oauth;
pub mod openai_compat;
pub mod router;
pub mod types;
pub mod xai_subscription;

pub use backend::LlmBackend;
pub use codex_oauth::{CodexOAuthBackend, CodexOAuthBackendState};
pub use openai_compat::OpenAiCompatBackend;
pub use router::{LlmRouter, LlmRouterConfig, RouteDecision};
pub use types::{
    AegisChatRequest, AegisChatResponse, AegisEmbedRequest, AegisEmbedResponse, AegisMessage,
    AegisMessageRole, AegisRequestMetadata, AegisToolCall, AegisToolChoice, AegisToolSpec,
    AegisUsage, LlmProviderError, LlmProviderErrorKind, LlmProviderKind, LlmTaskClass,
};
pub use xai_subscription::{
    XaiDeviceAuthorization, XaiDeviceCodePoll, XaiSubscriptionAuthConfig,
    XaiSubscriptionAuthManager, XaiSubscriptionBackend, XaiSubscriptionBackendState,
    XaiSubscriptionCredential,
};
