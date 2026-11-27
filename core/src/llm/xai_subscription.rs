use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::backend::{LlmBackend, LlmBackendFuture};
use super::types::{
    AegisChatRequest, AegisChatResponse, AegisEmbedRequest, AegisEmbedResponse, AegisMessage,
    AegisMessageRole, AegisToolCall, AegisUsage, LlmProviderError, LlmProviderErrorKind,
    LlmTaskClass,
};
use crate::credential_store::CredentialStore;

pub const XAI_SUBSCRIPTION_CREDENTIAL_SERVICE: &str = "xai_subscription";
pub const XAI_SUBSCRIPTION_CREDENTIAL_KEY: &str = "oauth_credential";
pub const XAI_SUBSCRIPTION_PENDING_DEVICE_KEY: &str = "pending_device_authorization";
pub const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const XAI_OAUTH_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const XAI_OAUTH_ISSUER: &str = "https://auth.x.ai";
pub const XAI_OAUTH_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const XAI_DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const XAI_DEFAULT_API_BASE: &str = "https://api.x.ai/v1";
const XAI_DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const XAI_DEVICE_CODE_DEFAULT_INTERVAL_SECS: u64 = 5;
const XAI_DEVICE_CODE_SLOW_DOWN_INCREMENT_SECS: u64 = 5;
/// xAI answers `slow_down` when a device-code poller is faster than it
/// tolerates; after that every subsequent poll must wait at least this long.
pub const XAI_DEVICE_CODE_SLOW_DOWN_MIN_INTERVAL_SECS: u64 = 10;

/// Interval a device-code poller must use after xAI returned `slow_down`
/// (RFC 8628 §3.5: add 5s to the current interval), floored at
/// [`XAI_DEVICE_CODE_SLOW_DOWN_MIN_INTERVAL_SECS`].
pub fn slow_down_interval_secs(current_interval_secs: u64) -> u64 {
    current_interval_secs
        .saturating_add(XAI_DEVICE_CODE_SLOW_DOWN_INCREMENT_SECS)
        .max(XAI_DEVICE_CODE_SLOW_DOWN_MIN_INTERVAL_SECS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XaiSubscriptionBackendState {
    AuthRequired,
    Unavailable,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XaiSubscriptionCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: Option<u64>,
    pub token_endpoint: String,
    pub device_authorization_endpoint: Option<String>,
    pub issuer: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub account_id: Option<String>,
    pub auth_flow: Option<String>,
}

impl XaiSubscriptionCredential {
    pub fn needs_refresh(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms + 60_000)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XaiDeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_at_ms: u64,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XaiDeviceCodePoll {
    Pending,
    SlowDown { interval_secs: u64 },
    Authorized(XaiSubscriptionCredential),
}

#[derive(Debug, Clone)]
pub struct XaiSubscriptionAuthConfig {
    pub discovery_url: String,
    pub request_timeout: Duration,
    pub enforce_trusted_endpoints: bool,
}

impl Default for XaiSubscriptionAuthConfig {
    fn default() -> Self {
        Self {
            discovery_url: XAI_OAUTH_DISCOVERY_URL.to_string(),
            request_timeout: Duration::from_secs(XAI_DEFAULT_REQUEST_TIMEOUT_SECS),
            enforce_trusted_endpoints: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct XaiSubscriptionAuthManager {
    http: reqwest::Client,
    config: XaiSubscriptionAuthConfig,
}

#[derive(Debug, Clone)]
struct XaiOAuthDiscovery {
    token_endpoint: String,
    device_authorization_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct DiscoveryResponse {
    token_endpoint: Option<String>,
    device_authorization_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: Option<String>,
    user_code: Option<String>,
    verification_uri: Option<String>,
    verification_uri_complete: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiChatCompletionResponse {
    id: Option<String>,
    choices: Vec<XaiChatChoice>,
    usage: Option<XaiChatUsage>,
}

#[derive(Debug, Deserialize)]
struct XaiChatChoice {
    message: XaiChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiChatMessage {
    content: Option<String>,
    tool_calls: Option<Vec<XaiChatToolCall>>,
}

#[derive(Debug, Deserialize)]
struct XaiChatToolCall {
    id: Option<String>,
    function: Option<XaiChatToolFunction>,
}

#[derive(Debug, Deserialize)]
struct XaiChatToolFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XaiGeneratedImage {
    pub data_url: String,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiImageGenerationResponse {
    id: Option<String>,
    data: Vec<XaiImageGenerationData>,
}

#[derive(Debug, Deserialize)]
struct XaiImageGenerationData {
    b64_json: Option<String>,
}

pub struct XaiSubscriptionBackend {
    model: String,
    api_base: String,
    http: reqwest::Client,
    state: XaiSubscriptionBackendState,
    credential: Option<XaiSubscriptionCredential>,
    /// Setting-level fallback used when `AegisRequestMetadata.reasoning_effort`
    /// is unset. Empty/`None` omits the field, matching today's behaviour.
    default_reasoning_effort: Option<String>,
    /// Background classes this backend may serve after the router granted them.
    /// Empty preserves the foreground-only gate.
    background_task_allowlist: Vec<LlmTaskClass>,
    /// Mirrors `xai_subscription_allow_background_tasks`. When true, every
    /// background class is accepted, matching the router.
    allow_all_background: bool,
}

impl XaiSubscriptionBackend {
    pub fn auth_required(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_base: XAI_DEFAULT_API_BASE.to_string(),
            http: reqwest::Client::new(),
            state: XaiSubscriptionBackendState::AuthRequired,
            credential: None,
            default_reasoning_effort: None,
            background_task_allowlist: Vec::new(),
            allow_all_background: false,
        }
    }

    pub fn unavailable(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_base: XAI_DEFAULT_API_BASE.to_string(),
            http: reqwest::Client::new(),
            state: XaiSubscriptionBackendState::Unavailable,
            credential: None,
            default_reasoning_effort: None,
            background_task_allowlist: Vec::new(),
            allow_all_background: false,
        }
    }

    pub fn with_credential(
        model: impl Into<String>,
        credential: XaiSubscriptionCredential,
    ) -> Self {
        Self::with_credential_and_api_base(model, credential, XAI_DEFAULT_API_BASE)
    }

    pub fn with_credential_and_api_base(
        model: impl Into<String>,
        credential: XaiSubscriptionCredential,
        api_base: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            api_base: api_base.into(),
            http: reqwest::Client::new(),
            state: XaiSubscriptionBackendState::Ready,
            credential: Some(credential),
            default_reasoning_effort: None,
            background_task_allowlist: Vec::new(),
            allow_all_background: false,
        }
    }

    /// Allow specific background classes through this backend.
    ///
    /// The router remains the policy gate. This list is the matching backend
    /// check so a granted class such as `PromptOptimization` can complete a
    /// subscription call. Other background classes stay blocked.
    #[must_use]
    pub fn with_background_task_allowlist(mut self, allowlist: Vec<LlmTaskClass>) -> Self {
        self.background_task_allowlist = allowlist;
        self
    }

    /// Match the router global background flag. Off by default.
    #[must_use]
    pub fn with_allow_all_background(mut self, allow: bool) -> Self {
        self.allow_all_background = allow;
        self
    }

    fn accepts_task_class(&self, task_class: LlmTaskClass) -> bool {
        task_class.is_foreground()
            || self.allow_all_background
            || self.background_task_allowlist.contains(&task_class)
    }

    /// Setting-level fallback for `reasoning_effort` when the request metadata
    /// does not set one. Empty values omit the field.
    #[must_use]
    pub fn with_default_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.default_reasoning_effort = effort
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
    }

    pub fn state(&self) -> XaiSubscriptionBackendState {
        self.state
    }

    pub async fn generate_image_data_url(
        &self,
        prompt: &str,
        aspect_ratio: &str,
    ) -> Result<XaiGeneratedImage, LlmProviderError> {
        let credential = self.ready_credential_for_multimodal("image generation")?;
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::UnsupportedFeature,
                "image generation prompt must not be empty",
            ));
        }

        let url = format!("{}/images/generations", self.api_base.trim_end_matches('/'));
        let body = xai_subscription_image_body(prompt, aspect_ratio);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&credential.access_token)
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("Grok subscription image request failed: {err}"),
                )
            })?;
        let parsed: XaiImageGenerationResponse =
            read_json_response(response, "Grok subscription image generation").await?;
        parse_xai_image_generation_data_url(parsed)
    }

    fn ready_credential_for_multimodal(
        &self,
        feature: &str,
    ) -> Result<&XaiSubscriptionCredential, LlmProviderError> {
        match self.state {
            XaiSubscriptionBackendState::AuthRequired => Err(LlmProviderError::new(
                LlmProviderErrorKind::AuthRequired,
                format!("Grok subscription {feature} requires owner authorization"),
            )),
            XaiSubscriptionBackendState::Unavailable => Err(LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("Grok subscription {feature} is unavailable"),
            )),
            XaiSubscriptionBackendState::Ready => {
                let Some(credential) = &self.credential else {
                    return Err(LlmProviderError::new(
                        LlmProviderErrorKind::AuthRequired,
                        format!("Grok subscription {feature} has no loaded credential"),
                    ));
                };
                if credential.needs_refresh(now_ms()) {
                    return Err(LlmProviderError::new(
                        LlmProviderErrorKind::AuthRequired,
                        format!(
                            "Grok subscription access token is expired or near expiry; refresh owner authorization before {feature}"
                        ),
                    ));
                }
                Ok(credential)
            }
        }
    }

    async fn chat_ready(
        &self,
        req: AegisChatRequest,
        credential: &XaiSubscriptionCredential,
    ) -> Result<AegisChatResponse, LlmProviderError> {
        if !self.accepts_task_class(req.metadata.task_class) {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::BackgroundTaskBlocked,
                "Grok subscription backend only accepts foreground requests",
            ));
        }
        if credential.needs_refresh(now_ms()) {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::AuthRequired,
                "Grok subscription access token is expired or near expiry; refresh owner authorization before retrying",
            ));
        }

        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));
        let effort = resolve_xai_subscription_reasoning_effort(
            req.metadata.reasoning_effort.as_deref(),
            self.default_reasoning_effort.as_deref(),
        );
        let body = xai_subscription_chat_body(&req, &self.model, effort.as_deref());

        let response = self
            .http
            .post(&url)
            .bearer_auth(&credential.access_token)
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("Grok subscription chat request failed: {err}"),
                )
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("Grok subscription chat response could not be read: {err}"),
            )
        })?;
        if !status.is_success() {
            return Err(LlmProviderError::new(
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    LlmProviderErrorKind::AuthRequired
                } else {
                    LlmProviderErrorKind::ProviderUnavailable
                },
                format!(
                    "Grok subscription chat failed with HTTP {}: {}",
                    status.as_u16(),
                    redact_token_like_text(&text)
                ),
            ));
        }
        let parsed: XaiChatCompletionResponse = serde_json::from_str(&text).map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("Grok subscription chat response was not valid JSON: {err}"),
            )
        })?;
        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                "Grok subscription chat response did not contain any choices",
            )
        })?;
        let content = choice.message.content.unwrap_or_default();
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|call| {
                let function = call.function?;
                let name = function.name?;
                let arguments = function
                    .arguments
                    .and_then(|args| serde_json::from_str(&args).ok())
                    .unwrap_or_else(|| serde_json::json!({}));
                Some(AegisToolCall {
                    id: call.id.unwrap_or_else(|| "xai-tool-call".to_string()),
                    name,
                    arguments,
                })
            })
            .collect();
        Ok(AegisChatResponse {
            message: AegisMessage {
                role: AegisMessageRole::Assistant,
                content,
            },
            tool_calls,
            usage: parsed.usage.map(|usage| AegisUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cached_input_tokens: None,
                total_tokens: usage.total_tokens,
            }),
            finish_reason: choice.finish_reason,
            provider_request_id: parsed.id,
        })
    }
}

/// Prefer a per-request effort when set; otherwise the configured setting.
/// Blank strings are treated as unset so the field is omitted.
pub(crate) fn resolve_xai_subscription_reasoning_effort(
    per_request: Option<&str>,
    default: Option<&str>,
) -> Option<String> {
    per_request
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            default
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn xai_subscription_chat_body(
    req: &AegisChatRequest,
    model: &str,
    reasoning_effort: Option<&str>,
) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": match message.role {
                    AegisMessageRole::System => "system",
                    AegisMessageRole::User => "user",
                    AegisMessageRole::Assistant => "assistant",
                    AegisMessageRole::Tool => "tool",
                },
                "content": message.content,
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
    });
    if let Some(temperature) = req.temperature {
        body["temperature"] = serde_json::json!(temperature);
    }
    if let Some(max_tokens) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(max_tokens);
    }
    if !req.tools.is_empty() {
        body["tools"] = serde_json::Value::Array(
            req.tools
                .iter()
                .filter(|tool| !tool.name.trim().is_empty())
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.schema,
                        }
                    })
                })
                .collect(),
        );
    }
    if let Some(choice) = &req.tool_choice {
        body["tool_choice"] = serde_json::json!({
            "type": "function",
            "function": { "name": choice.name },
        });
    }
    prepare_xai_subscription_chat_body(&mut body, model.to_string(), reasoning_effort);
    body
}

/// Shared xAI chat-body finalizer used by both the LLM-router backend and the
/// orchestrator's subscription HTTP path. Sets `model`, applies
/// `reasoning_effort` when non-empty, normalizes tool schemas, and drops
/// legacy pre-tools fields.
pub(crate) fn prepare_xai_subscription_chat_body(
    body: &mut serde_json::Value,
    model: String,
    reasoning_effort: Option<&str>,
) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    obj.insert("model".to_string(), serde_json::Value::String(model));
    match reasoning_effort.map(str::trim).filter(|v| !v.is_empty()) {
        Some(effort) => {
            obj.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(effort.to_string()),
            );
        }
        None => {
            obj.remove("reasoning_effort");
        }
    }

    if let Some(tools) = obj.get_mut("tools") {
        normalize_xai_subscription_tools(tools);
    }

    // xAI supports the modern OpenAI-style `tools[].function` request
    // shape. Drop only the legacy pre-tools fields.
    obj.remove("function_call");
    obj.remove("functions");
}

fn normalize_xai_subscription_tools(tools: &mut serde_json::Value) {
    let Some(tool_arr) = tools.as_array_mut() else {
        return;
    };
    for tool in tool_arr {
        let Some(tool_obj) = tool.as_object_mut() else {
            continue;
        };
        if tool_obj.get("type").and_then(serde_json::Value::as_str) != Some("function") {
            continue;
        }
        let Some(function_obj) = tool_obj
            .get_mut("function")
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        function_obj.remove("strict");
        if let Some(parameters) = function_obj.get_mut("parameters") {
            normalize_xai_subscription_json_schema(parameters);
        }
    }
}

fn normalize_xai_subscription_json_schema(schema: &mut serde_json::Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    if let Some(const_value) = obj.remove("const") {
        obj.insert(
            "enum".to_string(),
            serde_json::Value::Array(vec![const_value.clone()]),
        );
        obj.entry("type".to_string())
            .or_insert_with(|| json_schema_type_for_const(&const_value));
    }

    for key in [
        "$schema",
        "$defs",
        "definitions",
        "oneOf",
        "anyOf",
        "allOf",
        "additionalProperties",
        "maxLength",
        "minLength",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "pattern",
        "format",
        "examples",
    ] {
        obj.remove(key);
    }

    if let Some(properties) = obj
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        for property_schema in properties.values_mut() {
            normalize_xai_subscription_json_schema(property_schema);
        }
    }
    if let Some(items) = obj.get_mut("items") {
        normalize_xai_subscription_json_schema(items);
    }

    if !obj.contains_key("type") && !obj.contains_key("properties") && !obj.contains_key("enum") {
        obj.insert(
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        );
    }
    if obj.get("type").and_then(serde_json::Value::as_str) == Some("object") {
        obj.entry("properties".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }
}

fn json_schema_type_for_const(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::String(
        match value {
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
            _ => "string",
        }
        .to_string(),
    )
}

pub(crate) fn xai_subscription_image_body(prompt: &str, aspect_ratio: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "grok-imagine-image-2.0",
        "prompt": prompt,
        "n": 1,
        "response_format": "b64_json",
        "aspect_ratio": normalize_xai_image_aspect_ratio(aspect_ratio),
        "resolution": "1k"
    })
}

fn normalize_xai_image_aspect_ratio(value: &str) -> &'static str {
    match value.trim() {
        "16:9" => "16:9",
        "9:16" => "9:16",
        "4:3" => "4:3",
        "3:4" => "3:4",
        "3:2" => "3:2",
        "2:3" => "2:3",
        _ => "1:1",
    }
}

fn parse_xai_image_generation_data_url(
    response: XaiImageGenerationResponse,
) -> Result<XaiGeneratedImage, LlmProviderError> {
    let b64 = response
        .data
        .into_iter()
        .find_map(|item| item.b64_json)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                "Grok subscription image generation returned no base64 image data",
            )
        })?;
    if !b64
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            "Grok subscription image generation returned invalid base64 image data",
        ));
    }
    Ok(XaiGeneratedImage {
        data_url: format!("data:image/png;base64,{b64}"),
        provider_request_id: response.id,
    })
}

impl LlmBackend for XaiSubscriptionBackend {
    fn chat(&self, req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
        Box::pin(async move {
            match self.state {
                XaiSubscriptionBackendState::AuthRequired => Err(LlmProviderError::new(
                    LlmProviderErrorKind::AuthRequired,
                    "Grok subscription provider requires explicit owner authorization; no paid API fallback was attempted",
                )),
                XaiSubscriptionBackendState::Unavailable => Err(LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    "Grok subscription provider is unavailable until an official xAI subscription auth flow is configured",
                )),
                XaiSubscriptionBackendState::Ready => {
                    let Some(credential) = &self.credential else {
                        return Err(LlmProviderError::new(
                            LlmProviderErrorKind::AuthRequired,
                            "Grok subscription provider is enabled but no credential is loaded",
                        ));
                    };
                    self.chat_ready(req, credential).await
                }
            }
        })
    }

    fn embed(&self, _req: AegisEmbedRequest) -> LlmBackendFuture<'_, AegisEmbedResponse> {
        Box::pin(async {
            Err(LlmProviderError::new(
                LlmProviderErrorKind::EmbeddingBlocked,
                "embeddings must never route through Grok subscription auth",
            ))
        })
    }

    fn provider_name(&self) -> &'static str {
        "xai_subscription"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

impl XaiSubscriptionAuthManager {
    pub fn new(config: XaiSubscriptionAuthConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    pub async fn start_device_code(&self) -> Result<XaiDeviceAuthorization, LlmProviderError> {
        let discovery = self.discovery().await?;
        let response = self
            .http
            .post(&discovery.device_authorization_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .body(form_encode(&[
                ("client_id", XAI_OAUTH_CLIENT_ID),
                ("scope", XAI_OAUTH_SCOPE),
            ]))
            .timeout(self.config.request_timeout)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("xAI device code request failed: {err}"),
                )
            })?;
        let body: DeviceCodeResponse =
            read_json_response(response, "xAI device code request").await?;
        let device_code = required_field(body.device_code, "device_code")?;
        let user_code = required_field(body.user_code, "user_code")?;
        let verification_uri = self.trusted_endpoint(
            &required_field(body.verification_uri, "verification_uri")?,
            "device verification URI",
        )?;
        let verification_uri_complete = body
            .verification_uri_complete
            .filter(|value| !value.trim().is_empty())
            .map(|value| self.trusted_endpoint(&value, "complete device verification URI"))
            .transpose()?;
        let expires_in = body.expires_in.unwrap_or(900);
        Ok(XaiDeviceAuthorization {
            device_code,
            user_code,
            verification_uri,
            verification_uri_complete,
            expires_at_ms: now_ms() + expires_in.saturating_mul(1000),
            interval_secs: body
                .interval
                .unwrap_or(XAI_DEVICE_CODE_DEFAULT_INTERVAL_SECS),
        })
    }

    pub async fn poll_device_code_once(
        &self,
        authorization: &XaiDeviceAuthorization,
    ) -> Result<XaiDeviceCodePoll, LlmProviderError> {
        if now_ms() >= authorization.expires_at_ms {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::AuthRequired,
                "xAI device code expired. Start a new sign-in.",
            ));
        }
        let discovery = self.discovery().await?;
        let response = self
            .http
            .post(&discovery.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .body(form_encode(&[
                ("grant_type", XAI_DEVICE_CODE_GRANT_TYPE),
                ("client_id", XAI_OAUTH_CLIENT_ID),
                ("device_code", &authorization.device_code),
            ]))
            .timeout(self.config.request_timeout)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("xAI device token poll failed: {err}"),
                )
            })?;
        let status = response.status();
        let body: TokenResponse = response.json().await.map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("xAI device token response was not valid JSON: {err}"),
            )
        })?;
        if status.is_success() {
            return Ok(XaiDeviceCodePoll::Authorized(
                self.token_response_to_credential(body, &discovery, Some("device-code"))?,
            ));
        }
        match body.error.as_deref() {
            Some("authorization_pending") => Ok(XaiDeviceCodePoll::Pending),
            Some("slow_down") => Ok(XaiDeviceCodePoll::SlowDown {
                interval_secs: slow_down_interval_secs(authorization.interval_secs),
            }),
            Some("access_denied") | Some("authorization_denied") => Err(LlmProviderError::new(
                LlmProviderErrorKind::AuthRequired,
                "xAI device authorization was denied",
            )),
            Some("expired_token") => Err(LlmProviderError::new(
                LlmProviderErrorKind::AuthRequired,
                "xAI device code expired. Start a new sign-in.",
            )),
            _ => Err(LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!(
                    "xAI device token exchange failed with HTTP {}{}",
                    status.as_u16(),
                    body.error_description
                        .map(|desc| format!(": {}", redact_token_like_text(&desc)))
                        .unwrap_or_default()
                ),
            )),
        }
    }

    pub async fn refresh_credential(
        &self,
        credential: &XaiSubscriptionCredential,
    ) -> Result<XaiSubscriptionCredential, LlmProviderError> {
        let token_endpoint = self.trusted_endpoint(&credential.token_endpoint, "token endpoint")?;
        let response = self
            .http
            .post(&token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .body(form_encode(&[
                ("grant_type", "refresh_token"),
                ("client_id", XAI_OAUTH_CLIENT_ID),
                ("refresh_token", &credential.refresh_token),
            ]))
            .timeout(self.config.request_timeout)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("xAI OAuth refresh failed: {err}"),
                )
            })?;
        let discovery = XaiOAuthDiscovery {
            token_endpoint,
            device_authorization_endpoint: credential
                .device_authorization_endpoint
                .clone()
                .unwrap_or_default(),
        };
        let body: TokenResponse = read_json_response(response, "xAI OAuth refresh").await?;
        let mut refreshed =
            self.token_response_to_credential(body, &discovery, credential.auth_flow.as_deref())?;
        if refreshed.refresh_token.is_empty() {
            refreshed
                .refresh_token
                .clone_from(&credential.refresh_token);
        }
        Ok(refreshed)
    }

    pub fn store_credential(
        store: &CredentialStore,
        credential: &XaiSubscriptionCredential,
    ) -> Result<(), LlmProviderError> {
        let serialized = serde_json::to_string(credential).map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("could not serialize xAI subscription credential: {err}"),
            )
        })?;
        store
            .store(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_CREDENTIAL_KEY,
                &serialized,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not store xAI subscription credential: {err}"),
                )
            })
    }

    pub fn load_credential(
        store: &CredentialStore,
    ) -> Result<Option<XaiSubscriptionCredential>, LlmProviderError> {
        let Some(serialized) = store
            .get(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_CREDENTIAL_KEY,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not load xAI subscription credential: {err}"),
                )
            })?
        else {
            return Ok(None);
        };
        serde_json::from_str(&serialized).map(Some).map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("stored xAI subscription credential is invalid: {err}"),
            )
        })
    }

    pub fn delete_credential(store: &CredentialStore) -> Result<bool, LlmProviderError> {
        store
            .delete(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_CREDENTIAL_KEY,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not delete xAI subscription credential: {err}"),
                )
            })
    }

    pub fn store_pending_device_authorization(
        store: &CredentialStore,
        authorization: &XaiDeviceAuthorization,
    ) -> Result<(), LlmProviderError> {
        let serialized = serde_json::to_string(authorization).map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("could not serialize xAI pending device authorization: {err}"),
            )
        })?;
        store
            .store(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_PENDING_DEVICE_KEY,
                &serialized,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not store xAI pending device authorization: {err}"),
                )
            })
    }

    pub fn load_pending_device_authorization(
        store: &CredentialStore,
    ) -> Result<Option<XaiDeviceAuthorization>, LlmProviderError> {
        let Some(serialized) = store
            .get(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_PENDING_DEVICE_KEY,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not load xAI pending device authorization: {err}"),
                )
            })?
        else {
            return Ok(None);
        };
        serde_json::from_str(&serialized).map(Some).map_err(|err| {
            LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                format!("stored xAI pending device authorization is invalid: {err}"),
            )
        })
    }

    pub fn delete_pending_device_authorization(
        store: &CredentialStore,
    ) -> Result<bool, LlmProviderError> {
        store
            .delete(
                XAI_SUBSCRIPTION_CREDENTIAL_SERVICE,
                XAI_SUBSCRIPTION_PENDING_DEVICE_KEY,
            )
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("could not delete xAI pending device authorization: {err}"),
                )
            })
    }

    async fn discovery(&self) -> Result<XaiOAuthDiscovery, LlmProviderError> {
        let discovery_url = self.trusted_endpoint(&self.config.discovery_url, "discovery URL")?;
        let response = self
            .http
            .get(discovery_url)
            .header("Accept", "application/json")
            .header("User-Agent", "aegis-runtime/xai-subscription")
            .timeout(self.config.request_timeout)
            .send()
            .await
            .map_err(|err| {
                LlmProviderError::new(
                    LlmProviderErrorKind::ProviderUnavailable,
                    format!("xAI OAuth discovery failed: {err}"),
                )
            })?;
        let body: DiscoveryResponse = read_json_response(response, "xAI OAuth discovery").await?;
        Ok(XaiOAuthDiscovery {
            token_endpoint: self.trusted_endpoint(
                &required_field(body.token_endpoint, "token_endpoint")?,
                "token endpoint",
            )?,
            device_authorization_endpoint: self.trusted_endpoint(
                &required_field(
                    body.device_authorization_endpoint,
                    "device_authorization_endpoint",
                )?,
                "device authorization endpoint",
            )?,
        })
    }

    fn trusted_endpoint(&self, endpoint: &str, label: &str) -> Result<String, LlmProviderError> {
        if !self.config.enforce_trusted_endpoints || is_trusted_xai_oauth_endpoint(endpoint) {
            return Ok(endpoint.to_string());
        }
        Err(LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!("xAI OAuth discovery returned untrusted {label}"),
        ))
    }

    fn token_response_to_credential(
        &self,
        body: TokenResponse,
        discovery: &XaiOAuthDiscovery,
        auth_flow: Option<&str>,
    ) -> Result<XaiSubscriptionCredential, LlmProviderError> {
        let access_token = required_field(body.access_token, "access_token")?;
        let refresh_token = required_field(body.refresh_token, "refresh_token")?;
        let identity = resolve_identity(body.id_token.as_deref().unwrap_or(&access_token));
        Ok(XaiSubscriptionCredential {
            access_token,
            refresh_token,
            expires_at_ms: body
                .expires_in
                .map(|secs| now_ms() + secs.saturating_mul(1000)),
            token_endpoint: discovery.token_endpoint.clone(),
            device_authorization_endpoint: if discovery.device_authorization_endpoint.is_empty() {
                None
            } else {
                Some(discovery.device_authorization_endpoint.clone())
            },
            issuer: XAI_OAUTH_ISSUER.to_string(),
            email: identity.email,
            display_name: identity.display_name,
            account_id: identity.account_id,
            auth_flow: auth_flow.map(str::to_string),
        })
    }
}

#[derive(Debug, Default)]
struct XaiIdentity {
    email: Option<String>,
    display_name: Option<String>,
    account_id: Option<String>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub fn is_trusted_xai_oauth_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host == "x.ai" || host.ends_with(".x.ai")
}

async fn read_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, LlmProviderError> {
    let status = response.status();
    let text = response.text().await.map_err(|err| {
        LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!("{context} response could not be read: {err}"),
        )
    })?;
    if !status.is_success() {
        return Err(LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!(
                "{context} failed with HTTP {}: {}",
                status.as_u16(),
                redact_token_like_text(&text)
            ),
        ));
    }
    serde_json::from_str(&text).map_err(|err| {
        LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!("{context} response was not valid JSON: {err}"),
        )
    })
}

fn required_field(value: Option<String>, field: &str) -> Result<String, LlmProviderError> {
    let Some(value) = value else {
        return Err(LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!("xAI OAuth response is missing {field}"),
        ));
    };
    if value.trim().is_empty() {
        return Err(LlmProviderError::new(
            LlmProviderErrorKind::ProviderUnavailable,
            format!("xAI OAuth response has empty {field}"),
        ));
    }
    Ok(value)
}

fn form_encode(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn resolve_identity(jwt: &str) -> XaiIdentity {
    let Some(payload) = jwt.split('.').nth(1) else {
        return XaiIdentity::default();
    };
    let Ok(decoded) =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
    else {
        return XaiIdentity::default();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&decoded) else {
        return XaiIdentity::default();
    };
    XaiIdentity {
        email: value
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        display_name: value
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        account_id: value
            .get("sub")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

fn redact_token_like_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for marker in ["access_token", "refresh_token", "id_token", "device_code"] {
        if redacted.contains(marker) {
            redacted = format!("{marker}=<redacted>");
        }
    }
    redacted.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use crate::credential_store::{CredentialStore, derive_key_from_ed25519};
    use crate::identity::AgentIdentity;
    use crate::llm::types::{
        AegisChatRequest, AegisEmbedRequest, AegisMessage, AegisMessageRole, AegisRequestMetadata,
        AegisToolChoice, AegisToolSpec, LlmTaskClass,
    };
    use axum::Router;
    use axum::extract::State;
    use axum::routing::{get, post};
    use base64::Engine;
    use rusqlite::Connection;
    use serde_json::json;

    #[derive(Default)]
    struct MockXaiState {
        requests: Mutex<Vec<String>>,
    }

    fn chat_req() -> AegisChatRequest {
        AegisChatRequest {
            model: "requested".to_string(),
            messages: vec![AegisMessage {
                role: AegisMessageRole::User,
                content: "hello".to_string(),
            }],
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

    fn background_chat_req() -> AegisChatRequest {
        AegisChatRequest {
            metadata: AegisRequestMetadata {
                task_class: LlmTaskClass::BackgroundHeartbeat,
                request_id: Some("req-bg".to_string()),
                chat_id: Some(42),
                user_visible: false,
                reasoning_effort: None,
            },
            ..chat_req()
        }
    }

    fn tool_chat_req() -> AegisChatRequest {
        AegisChatRequest {
            tools: vec![AegisToolSpec {
                name: "calendar_reader".to_string(),
                description: "Read Calendar safely".to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string" },
                        "query": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            }],
            tool_choice: Some(AegisToolChoice {
                name: "calendar_reader".to_string(),
            }),
            ..chat_req()
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
    async fn xai_subscription_skeleton_requires_auth_without_fallback() {
        let backend = XaiSubscriptionBackend::auth_required("grok-4.1-fast");

        assert_eq!(backend.provider_name(), "xai_subscription");
        assert_eq!(backend.model_name(), "grok-4.1-fast");
        assert_eq!(backend.state(), XaiSubscriptionBackendState::AuthRequired);

        let err = backend
            .chat(chat_req())
            .await
            .expect_err("skeleton must not make a real request");
        assert_eq!(err.kind, LlmProviderErrorKind::AuthRequired);
        assert!(err.message.contains("no paid API fallback"));
    }

    #[tokio::test]
    async fn xai_subscription_skeleton_reports_unavailable() {
        let backend = XaiSubscriptionBackend::unavailable("grok-4.1-fast");
        let err = backend
            .chat(chat_req())
            .await
            .expect_err("unavailable skeleton must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::ProviderUnavailable);
    }

    #[tokio::test]
    async fn xai_subscription_skeleton_blocks_embeddings() {
        let backend = XaiSubscriptionBackend::auth_required("grok-4.1-fast");
        let err = backend
            .embed(embed_req())
            .await
            .expect_err("Grok subscription must not accept embedding requests");
        assert_eq!(err.kind, LlmProviderErrorKind::EmbeddingBlocked);
    }

    #[test]
    fn slow_down_interval_never_drops_below_ten_seconds() {
        assert_eq!(slow_down_interval_secs(5), 10);
        assert_eq!(slow_down_interval_secs(1), 10);
        assert_eq!(slow_down_interval_secs(10), 15);
        assert_eq!(slow_down_interval_secs(u64::MAX), u64::MAX);
    }

    #[test]
    fn xai_subscription_chat_body_preserves_tool_runtime_contract() {
        let body = xai_subscription_chat_body(&tool_chat_req(), "grok-4.3", None);

        assert_eq!(body["model"], "grok-4.3");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "calendar_reader");
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["required"][0],
            "action"
        );
        assert_eq!(body["tool_choice"]["function"]["name"], "calendar_reader");
    }

    #[test]
    fn xai_subscription_chat_body_carries_per_request_reasoning_effort() {
        let mut req = chat_req();
        req.metadata.reasoning_effort = Some("low".to_string());
        let effort = resolve_xai_subscription_reasoning_effort(
            req.metadata.reasoning_effort.as_deref(),
            Some("medium"),
        );
        let body = xai_subscription_chat_body(&req, "grok-4.6", effort.as_deref());
        assert_eq!(body["reasoning_effort"], "low");
    }

    #[test]
    fn xai_subscription_chat_body_falls_back_to_setting_when_metadata_unset() {
        let req = chat_req();
        let effort = resolve_xai_subscription_reasoning_effort(
            req.metadata.reasoning_effort.as_deref(),
            Some("medium"),
        );
        let body = xai_subscription_chat_body(&req, "grok-4.6", effort.as_deref());
        assert_eq!(body["reasoning_effort"], "medium");

        let omitted = resolve_xai_subscription_reasoning_effort(None, None);
        let body = xai_subscription_chat_body(&req, "grok-4.6", omitted.as_deref());
        assert!(
            body.get("reasoning_effort").is_none(),
            "unset effort omits the field"
        );

        let blank = resolve_xai_subscription_reasoning_effort(Some("  "), Some("high"));
        assert_eq!(blank.as_deref(), Some("high"));
    }

    #[test]
    fn xai_subscription_image_body_uses_subscription_image_endpoint_shape() {
        let body = xai_subscription_image_body("Aegis avatar", "2:1");

        assert_eq!(body["model"], "grok-imagine-image-2.0");
        assert_eq!(body["prompt"], "Aegis avatar");
        assert_eq!(body["response_format"], "b64_json");
        assert_eq!(body["aspect_ratio"], "1:1");
        assert_eq!(body["resolution"], "1k");
    }

    #[test]
    fn xai_image_parser_returns_storable_data_url() {
        let parsed = parse_xai_image_generation_data_url(XaiImageGenerationResponse {
            id: Some("img-1".into()),
            data: vec![XaiImageGenerationData {
                b64_json: Some("QUJDRA==".into()),
            }],
        })
        .expect("image data should parse");

        assert_eq!(parsed.provider_request_id.as_deref(), Some("img-1"));
        assert_eq!(parsed.data_url, "data:image/png;base64,QUJDRA==");
    }

    #[tokio::test]
    #[ignore = "live xAI image generation smoke; set AEGIS_LIVE_XAI_IMAGE_SMOKE=1"]
    async fn live_xai_subscription_image_generation_smoke() {
        assert_eq!(
            std::env::var("AEGIS_LIVE_XAI_IMAGE_SMOKE").ok().as_deref(),
            Some("1"),
            "live smoke is opt-in because it may consume one provider image request"
        );
        if let Ok(env_file) = std::env::var("AEGIS_ENV_FILE") {
            let _ = dotenvy::from_path_override(env_file);
        }

        let identity_key_path =
            std::env::var("IDENTITY_KEY_PATH").expect("IDENTITY_KEY_PATH is required");
        assert!(
            Path::new(&identity_key_path).exists(),
            "live smoke must not create a new identity key"
        );
        let credential_db_path =
            std::env::var("CREDENTIAL_DB_PATH").expect("CREDENTIAL_DB_PATH is required");
        let identity = AgentIdentity::load_or_generate(Path::new(&identity_key_path))
            .expect("load production identity");
        let store = CredentialStore::open(
            Connection::open(&credential_db_path).expect("open credential DB"),
            derive_key_from_ed25519(&identity.signing_key_bytes()),
        )
        .expect("open credential store");
        let credential = XaiSubscriptionAuthManager::load_credential(&store)
            .expect("load xAI subscription credential")
            .expect("xAI subscription credential is missing");
        assert!(
            !credential.needs_refresh(now_ms()),
            "xAI subscription credential needs refresh before live image smoke"
        );
        let api_base = std::env::var("XAI_SUBSCRIPTION_API_BASE")
            .unwrap_or_else(|_| XAI_DEFAULT_API_BASE.to_string());
        let backend =
            XaiSubscriptionBackend::with_credential_and_api_base("grok-4.3", credential, api_base);

        let generated = backend
            .generate_image_data_url(
                "A compact elegant Aegis Runtime avatar smoke test, adult female AI assistant, clean Apple-like studio lighting, no text, no logo",
                "1:1",
            )
            .await
            .expect("live xAI image generation should return an image");

        assert!(generated.data_url.starts_with("data:image/png;base64,"));
        assert!(generated.data_url.len() > 1_000);
        eprintln!(
            "live_xai_image_smoke_ok provider_request_id={} data_url_bytes={}",
            generated.provider_request_id.as_deref().unwrap_or("<none>"),
            generated.data_url.len()
        );
    }

    #[test]
    fn xai_oauth_endpoint_trust_is_strict() {
        assert!(is_trusted_xai_oauth_endpoint(
            "https://auth.x.ai/oauth2/token"
        ));
        assert!(is_trusted_xai_oauth_endpoint(
            "https://accounts.x.ai/oauth2/device"
        ));
        assert!(!is_trusted_xai_oauth_endpoint(
            "http://auth.x.ai/oauth2/token"
        ));
        assert!(!is_trusted_xai_oauth_endpoint(
            "https://x.ai.evil.test/oauth2/token"
        ));
        assert!(!is_trusted_xai_oauth_endpoint("not a url"));
    }

    #[tokio::test]
    async fn xai_device_code_flow_uses_official_shape_and_parses_identity() {
        let state = Arc::new(MockXaiState::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let discovery_base = base_url.clone();
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let base_url = discovery_base.clone();
                    async move {
                        axum::Json(json!({
                            "device_authorization_endpoint": format!("{base_url}/oauth2/device/code"),
                            "token_endpoint": format!("{base_url}/oauth2/token")
                        }))
                    }
                }),
            )
            .route(
                "/oauth2/device/code",
                post(
                    |State(state): State<Arc<MockXaiState>>, body: String| async move {
                        state.requests.lock().unwrap().push(body);
                        axum::Json(json!({
                            "device_code": "device-code-1",
                            "user_code": "ABCD-1234",
                            "verification_uri": "http://127.0.0.1/oauth2/device",
                            "verification_uri_complete": "http://127.0.0.1/oauth2/device?user_code=ABCD-1234",
                            "expires_in": 900,
                            "interval": 5
                        }))
                    },
                ),
            )
            .route(
                "/oauth2/token",
                post(
                    |State(state): State<Arc<MockXaiState>>, body: String| async move {
                        state.requests.lock().unwrap().push(body);
                        axum::Json(json!({
                            "access_token": jwt(json!({"exp": 4_102_444_800u64, "sub": "acct-1"})),
                            "refresh_token": "refresh-1",
                            "id_token": jwt(json!({
                                "sub": "acct-1",
                                "email": "owner@example.com",
                                "name": "Owner"
                            })),
                            "expires_in": 120
                        }))
                    },
                ),
            )
            .with_state(Arc::clone(&state));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let manager = XaiSubscriptionAuthManager::new(XaiSubscriptionAuthConfig {
            discovery_url: format!("{base_url}/.well-known/openid-configuration"),
            enforce_trusted_endpoints: false,
            ..XaiSubscriptionAuthConfig::default()
        });
        let authorization = manager.start_device_code().await.unwrap();
        assert_eq!(authorization.user_code, "ABCD-1234");

        let credential = match manager.poll_device_code_once(&authorization).await.unwrap() {
            XaiDeviceCodePoll::Authorized(credential) => credential,
            other => panic!("expected authorized credential, got {other:?}"),
        };
        assert_eq!(credential.email.as_deref(), Some("owner@example.com"));
        assert_eq!(credential.display_name.as_deref(), Some("Owner"));
        assert_eq!(credential.account_id.as_deref(), Some("acct-1"));
        assert_eq!(credential.auth_flow.as_deref(), Some("device-code"));

        let requests = state.requests.lock().unwrap();
        assert!(requests[0].contains(&format!(
            "client_id={}",
            urlencoding::encode(XAI_OAUTH_CLIENT_ID)
        )));
        assert!(requests[0].contains(&format!("scope={}", urlencoding::encode(XAI_OAUTH_SCOPE))));
        assert!(
            requests[1]
                .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code")
        );
        assert!(requests[1].contains("device_code=device-code-1"));
        handle.abort();
    }

    #[test]
    fn xai_subscription_credential_store_roundtrips_encrypted_json() {
        let store = CredentialStore::open(Connection::open_in_memory().unwrap(), [7; 32]).unwrap();
        let credential = sample_credential();

        XaiSubscriptionAuthManager::store_credential(&store, &credential).unwrap();
        let pending = XaiDeviceAuthorization {
            device_code: "device-code-1".to_string(),
            user_code: "ABCD-1234".to_string(),
            verification_uri: "https://accounts.x.ai/oauth2/device".to_string(),
            verification_uri_complete: Some(
                "https://accounts.x.ai/oauth2/device?user_code=ABCD-1234".to_string(),
            ),
            expires_at_ms: now_ms() + 600_000,
            interval_secs: 5,
        };
        XaiSubscriptionAuthManager::store_pending_device_authorization(&store, &pending).unwrap();

        let loaded = XaiSubscriptionAuthManager::load_credential(&store)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, credential);
        let loaded_pending = XaiSubscriptionAuthManager::load_pending_device_authorization(&store)
            .unwrap()
            .unwrap();
        assert_eq!(loaded_pending, pending);
        assert!(XaiSubscriptionAuthManager::delete_pending_device_authorization(&store).unwrap());
        assert!(
            XaiSubscriptionAuthManager::load_pending_device_authorization(&store)
                .unwrap()
                .is_none()
        );
        assert!(XaiSubscriptionAuthManager::delete_credential(&store).unwrap());
        assert!(
            XaiSubscriptionAuthManager::load_credential(&store)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn ready_backend_posts_foreground_chat_to_xai_endpoint() {
        let state = Arc::new(MockXaiState::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/chat/completions",
                post(
                    |State(state): State<Arc<MockXaiState>>, body: String| async move {
                        state.requests.lock().unwrap().push(body);
                        axum::Json(json!({
                            "id": "chatcmpl-1",
                            "choices": [{
                                "message": {"role": "assistant", "content": "hello from grok"},
                                "finish_reason": "stop"
                            }],
                            "usage": {
                                "prompt_tokens": 3,
                                "completion_tokens": 4,
                                "total_tokens": 7
                            }
                        }))
                    },
                ),
            )
            .with_state(Arc::clone(&state));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let backend = XaiSubscriptionBackend::with_credential_and_api_base(
            "grok-4.3",
            sample_credential(),
            base_url.clone(),
        );
        let response = backend.chat(tool_chat_req()).await.unwrap();
        assert_eq!(response.message.content, "hello from grok");
        assert_eq!(response.provider_request_id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(response.usage.unwrap().total_tokens, Some(7));
        let request = state.requests.lock().unwrap().join("\n");
        assert!(request.contains("\"model\":\"grok-4.3\""));
        assert!(request.contains("\"role\":\"user\""));
        assert!(request.contains("\"tools\""));
        assert!(request.contains("\"calendar_reader\""));

        let err = backend
            .chat(background_chat_req())
            .await
            .expect_err("backend must block background calls even if router was bypassed");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);

        let prompt_backend = XaiSubscriptionBackend::with_credential_and_api_base(
            "grok-4.3",
            sample_credential(),
            base_url,
        )
        .with_background_task_allowlist(vec![LlmTaskClass::PromptOptimization]);
        let prompt_response = prompt_backend
            .chat(prompt_optimization_chat_req())
            .await
            .expect("allowlisted prompt optimization may use the subscription backend");
        assert_eq!(prompt_response.message.content, "hello from grok");
        let still_blocked = prompt_backend
            .chat(background_chat_req())
            .await
            .expect_err("allowlisting prompt optimization must not open heartbeat");
        assert_eq!(
            still_blocked.kind,
            LlmProviderErrorKind::BackgroundTaskBlocked
        );
        handle.abort();
    }

    fn prompt_optimization_chat_req() -> AegisChatRequest {
        AegisChatRequest {
            metadata: AegisRequestMetadata {
                task_class: LlmTaskClass::PromptOptimization,
                request_id: Some("req-prompt-opt".to_string()),
                chat_id: Some(42),
                user_visible: false,
                reasoning_effort: None,
            },
            ..chat_req()
        }
    }

    fn sample_credential() -> XaiSubscriptionCredential {
        XaiSubscriptionCredential {
            access_token: "access-1".to_string(),
            refresh_token: "refresh-1".to_string(),
            expires_at_ms: Some(now_ms() + 600_000),
            token_endpoint: "https://auth.x.ai/oauth2/token".to_string(),
            device_authorization_endpoint: Some("https://auth.x.ai/oauth2/device/code".to_string()),
            issuer: XAI_OAUTH_ISSUER.to_string(),
            email: Some("owner@example.com".to_string()),
            display_name: Some("Owner".to_string()),
            account_id: Some("acct-1".to_string()),
            auth_flow: Some("device-code".to_string()),
        }
    }

    fn jwt(payload: serde_json::Value) -> String {
        format!(
            "{}.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_string(&json!({"alg":"none","typ":"JWT"})).unwrap()),
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_string(&payload).unwrap())
        )
    }
}
