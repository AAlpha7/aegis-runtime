use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmTaskClass {
    ForegroundUserRequest,
    CodingAgentForeground,
    ReviewGuard,
    Summarization,
    BackgroundHeartbeat,
    MemoryConsolidation,
    LearningExtraction,
    ScheduledResearch,
    Embedding,
    /// Background prompt-optimization loop. Subscription use is denied until
    /// the owner grants this class alone (or all background tasks).
    PromptOptimization,
    /// Background skill-file evolution. Subscription use is denied until the
    /// owner grants this class alone. Paid API spend also requires
    /// `skillfile_optimization_allow_paid_api` (default false).
    SkillOptimization,
    #[default]
    OtherBackground,
}

impl LlmTaskClass {
    pub fn is_foreground(self) -> bool {
        matches!(
            self,
            Self::ForegroundUserRequest | Self::CodingAgentForeground
        )
    }

    pub fn is_embedding(self) -> bool {
        matches!(self, Self::Embedding)
    }

    /// Snake-case name stored in subscription background allowlists.
    pub fn setting_name(self) -> &'static str {
        match self {
            Self::ForegroundUserRequest => "foreground_user_request",
            Self::CodingAgentForeground => "coding_agent_foreground",
            Self::ReviewGuard => "review_guard",
            Self::Summarization => "summarization",
            Self::BackgroundHeartbeat => "background_heartbeat",
            Self::MemoryConsolidation => "memory_consolidation",
            Self::LearningExtraction => "learning_extraction",
            Self::ScheduledResearch => "scheduled_research",
            Self::Embedding => "embedding",
            Self::PromptOptimization => "prompt_optimization",
            Self::SkillOptimization => "skill_optimization",
            Self::OtherBackground => "other_background",
        }
    }

    /// Parse an allowlist entry. Unknown names return `None` so a typo grants nothing.
    pub fn from_setting_name(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(parsed) =
            serde_json::from_value::<Self>(serde_json::Value::String(trimmed.to_ascii_lowercase()))
        {
            return Some(parsed);
        }
        match trimmed {
            "ForegroundUserRequest" => Some(Self::ForegroundUserRequest),
            "CodingAgentForeground" => Some(Self::CodingAgentForeground),
            "ReviewGuard" => Some(Self::ReviewGuard),
            "Summarization" => Some(Self::Summarization),
            "BackgroundHeartbeat" => Some(Self::BackgroundHeartbeat),
            "MemoryConsolidation" => Some(Self::MemoryConsolidation),
            "LearningExtraction" => Some(Self::LearningExtraction),
            "ScheduledResearch" => Some(Self::ScheduledResearch),
            "Embedding" => Some(Self::Embedding),
            "PromptOptimization" => Some(Self::PromptOptimization),
            "SkillOptimization" => Some(Self::SkillOptimization),
            "OtherBackground" => Some(Self::OtherBackground),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProviderKind {
    #[serde(rename = "codex_oauth")]
    CodexOAuth,
    #[serde(rename = "xai_subscription")]
    XaiSubscription,
    #[serde(rename = "openai_compat")]
    OpenAiCompat,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "openai_api")]
    OpenAiApi,
    #[serde(rename = "local")]
    Local,
}

impl LlmProviderKind {
    pub fn is_paid_api(self) -> bool {
        matches!(
            self,
            Self::OpenAiCompat | Self::OpenRouter | Self::OpenAiApi
        )
    }
}

impl fmt::Display for LlmProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::CodexOAuth => "codex_oauth",
            Self::XaiSubscription => "xai_subscription",
            Self::OpenAiCompat => "openai_compat",
            Self::OpenRouter => "openrouter",
            Self::OpenAiApi => "openai_api",
            Self::Local => "local",
        };
        f.write_str(value)
    }
}

impl FromStr for LlmProviderKind {
    type Err = LlmProviderError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex_oauth" | "codex" => Ok(Self::CodexOAuth),
            "xai_subscription" | "grok_subscription" | "grok" | "xai" => Ok(Self::XaiSubscription),
            "openai_compat" | "openai-compatible" | "openai_compatible" => Ok(Self::OpenAiCompat),
            "openrouter" => Ok(Self::OpenRouter),
            "openai_api" | "openai" => Ok(Self::OpenAiApi),
            "local" => Ok(Self::Local),
            other => Err(LlmProviderError::unknown_provider(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AegisMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisMessage {
    pub role: AegisMessageRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisToolSpec {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisToolChoice {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegisRequestMetadata {
    pub task_class: LlmTaskClass,
    pub request_id: Option<String>,
    pub chat_id: Option<i64>,
    pub user_visible: bool,
    /// Per-request xAI reasoning effort (`low`/`medium`/`high`). `None` falls
    /// back to the `xai_subscription_reasoning_effort` setting.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisChatRequest {
    pub model: String,
    pub messages: Vec<AegisMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<AegisToolSpec>,
    pub tool_choice: Option<AegisToolChoice>,
    pub metadata: AegisRequestMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisChatResponse {
    pub message: AegisMessage,
    pub tool_calls: Vec<AegisToolCall>,
    pub usage: Option<AegisUsage>,
    pub finish_reason: Option<String>,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisEmbedRequest {
    pub model: String,
    pub input: Vec<String>,
    pub metadata: AegisRequestMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegisEmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub usage: Option<AegisUsage>,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegisUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProviderErrorKind {
    UnknownProvider,
    AuthRequired,
    ProviderDisabled,
    ProviderUnavailable,
    PaidApiFallbackDisabled,
    BackgroundTaskBlocked,
    EmbeddingBlocked,
    UnsupportedFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct LlmProviderError {
    pub kind: LlmProviderErrorKind,
    pub message: String,
}

impl LlmProviderError {
    pub fn new(kind: LlmProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn unknown_provider(provider: impl Into<String>) -> Self {
        Self::new(
            LlmProviderErrorKind::UnknownProvider,
            format!(
                "unknown LLM provider `{}`; refusing to treat it as a paid OpenAI-compatible API",
                provider.into()
            ),
        )
    }

    pub fn unsupported_feature(message: impl Into<String>) -> Self {
        Self::new(LlmProviderErrorKind::UnsupportedFeature, message)
    }
}
