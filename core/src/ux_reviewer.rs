//! UX Reviewer — checks UI schemas from a non-technical user perspective.
//! Called after panel rendering changes to catch confusing labels, unclear
//! terminology, and poor information hierarchy.
//!
//! # Integration status: NOT YET WIRED
//!
//! Fully implemented with fail-open semantics. Intended integration point:
//! call `review_ui_schema()` in the SDUI panel rendering pipeline (sdui.rs)
//! for owner-facing copy review. The LLM client and model name are already
//! available there.

use std::sync::Arc;

use anyhow::Context;
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, CreateChatCompletionResponse,
    ResponseFormat,
};
use serde::{Deserialize, Serialize};

use crate::llm::{
    AegisChatRequest, AegisRequestMetadata, LlmRouter, LlmRouterConfig, LlmTaskClass,
    OpenAiCompatBackend,
};
use crate::settings::RuntimeSettings;

const UX_REVIEW_PROMPT: &str = "\
You are a non-technical user looking at a dashboard interface. \
You will receive a JSON UI schema that describes what the user sees.\n\n\
Evaluate from a REGULAR USER perspective (not a developer):\n\
1. Can you understand what each section/tab does just from its title?\n\
2. Are there any technical terms, abbreviations, or jargon that a regular person wouldn't understand?\n\
3. Is the information hierarchy clear — can you tell what's important at a glance?\n\
4. If you wanted to take an action (approve, cancel, etc.), would you know where to click?\n\
5. Are labels in a consistent language (not mixing languages)?\n\n\
Reply with JSON:\n\
{\"score\": 1-10, \"issues\": [{\"element\": \"...\", \"problem\": \"...\", \"suggestion\": \"...\"}], \"overall\": \"brief summary\"}\n\
Score 8+ = good UX. Score below 6 = needs work. Be specific about confusing elements.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxReviewResult {
    pub score: u8,
    pub issues: Vec<UxIssue>,
    pub overall: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxIssue {
    pub element: String,
    pub problem: String,
    pub suggestion: String,
}

impl Default for UxReviewResult {
    fn default() -> Self {
        Self {
            score: 10,
            issues: vec![],
            overall: "No review performed".into(),
        }
    }
}

/// Run a UX review on a rendered UI schema.
/// Returns issues found from a non-technical user perspective.
/// Fail-open: any error returns a passing result.
pub async fn review_ui_schema(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
    schema_json: &str,
    context: &str, // e.g. "admin panel" or "monitor dashboard"
) -> UxReviewResult {
    let system = match ChatCompletionRequestSystemMessageArgs::default()
        .content(UX_REVIEW_PROMPT)
        .build()
    {
        Ok(s) => s.into(),
        Err(_) => return UxReviewResult::default(),
    };

    // Truncate schema to avoid token limits
    let schema_preview: String = schema_json.chars().take(4000).collect();
    let input = format!("Context: {}\n\nUI Schema:\n{}", context, schema_preview);

    let user = match ChatCompletionRequestUserMessageArgs::default()
        .content(&*input)
        .build()
    {
        Ok(u) => u.into(),
        Err(_) => return UxReviewResult::default(),
    };

    let request = match CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![system, user])
        .response_format(ResponseFormat::JsonObject)
        .build()
    {
        Ok(r) => r,
        Err(_) => return UxReviewResult::default(),
    };

    let response =
        match create_ux_review_chat_completion(llm_client, settings, request, model).await {
            Ok(r) => {
                crate::llm_client::log_llm_usage(&r, "ux_review");
                r
            }
            Err(e) => {
                tracing::debug!(error = %e, "UX reviewer LLM call failed (fail-open)");
                return UxReviewResult::default();
            }
        };

    let text = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("{}");

    serde_json::from_str(text).unwrap_or_default()
}

fn route_ux_review_model(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
) -> anyhow::Result<String> {
    let config = LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(settings)
        .map_err(anyhow::Error::new)?;
    let backend = OpenAiCompatBackend::new(llm_client.clone(), model);
    let router = LlmRouter::new(Arc::new(backend), config);
    let request = AegisChatRequest {
        model: model.to_string(),
        messages: Vec::new(),
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        tool_choice: None,
        metadata: AegisRequestMetadata {
            task_class: LlmTaskClass::ReviewGuard,
            request_id: None,
            chat_id: None,
            user_visible: false,
            reasoning_effort: None,
        },
    };
    let decision = router.route_chat(&request).map_err(anyhow::Error::new)?;
    tracing::info!(
        label = "ux_review",
        provider = %decision.provider,
        model = %decision.model,
        task_class = ?LlmTaskClass::ReviewGuard,
        is_fallback = decision.is_fallback,
        paid_api_provider = decision.provider.is_paid_api(),
        paid_api_fallback_allowed = settings
            .get("paid_api_fallback_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "[LLM Route]"
    );
    Ok(decision.model)
}

async fn create_ux_review_chat_completion(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    mut request: CreateChatCompletionRequest,
    model: &str,
) -> anyhow::Result<CreateChatCompletionResponse> {
    request.model = route_ux_review_model(llm_client, settings, model)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        llm_client.chat().create(request),
    )
    .await
    .context("UX reviewer LLM call timed out")?
    .context("UX reviewer LLM call failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ux_review_prompt_content() {
        assert!(UX_REVIEW_PROMPT.contains("non-technical"));
        assert!(UX_REVIEW_PROMPT.contains("jargon"));
        assert!(UX_REVIEW_PROMPT.contains("score"));
    }

    #[test]
    fn test_default_result_is_passing() {
        let r = UxReviewResult::default();
        assert_eq!(r.score, 10);
        assert!(r.issues.is_empty());
    }

    #[test]
    fn test_parse_review_result() {
        let json = r#"{"score": 7, "issues": [{"element": "tab label", "problem": "uses jargon", "suggestion": "rename"}], "overall": "mostly clear"}"#;
        let r: UxReviewResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.score, 7);
        assert_eq!(r.issues.len(), 1);
    }

    #[test]
    fn ux_review_routing_blocks_explicit_codex_background_by_default() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "llm_provider",
                serde_json::Value::String("codex_oauth".to_string()),
            )
            .unwrap();
        let llm_client = crate::llm_client::create_llm_client("fake", "http://localhost");

        let err = route_ux_review_model(&llm_client, &settings, "test-model").unwrap_err();
        assert!(
            format!("{err:#}").contains("codex_enabled=false")
                || format!("{err:#}").contains("not allowed for task class ReviewGuard"),
            "unexpected routing error: {err:#}"
        );
    }
}
