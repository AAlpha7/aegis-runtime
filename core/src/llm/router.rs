use std::{fmt, sync::Arc};

use serde_json::Value;

use super::backend::LlmBackend;
use super::types::{
    AegisChatRequest, LlmProviderError, LlmProviderErrorKind, LlmProviderKind, LlmTaskClass,
};
use crate::settings::RuntimeSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRouterConfig {
    pub llm_provider: LlmProviderKind,
    pub paid_api_fallback_enabled: bool,
    pub codex_enabled: bool,
    pub codex_foreground_only: bool,
    pub codex_allow_background_tasks: bool,
    /// Background task classes allowed to use Codex when
    /// `codex_allow_background_tasks` is false. Empty by default.
    pub codex_background_task_allowlist: Vec<LlmTaskClass>,
    pub xai_subscription_enabled: bool,
    pub xai_subscription_foreground_only: bool,
    pub xai_subscription_allow_background_tasks: bool,
    /// Background task classes allowed to use the Grok subscription when
    /// `xai_subscription_allow_background_tasks` is false. Empty by default.
    /// Granting `PromptOptimization` does not grant any other background class.
    pub xai_subscription_background_task_allowlist: Vec<LlmTaskClass>,
    /// Paid API spend for `PromptOptimization` only. `paid_api_fallback_enabled`
    /// does not grant this. Default false.
    pub prompt_optimization_allow_paid_api: bool,
    /// Paid API spend for `SkillOptimization` only. `paid_api_fallback_enabled`
    /// does not grant this. Default false. Granting `PromptOptimization` does
    /// not grant this class.
    pub skillfile_optimization_allow_paid_api: bool,
}

impl LlmRouterConfig {
    pub fn from_runtime_settings(settings: &RuntimeSettings) -> Result<Self, LlmProviderError> {
        let provider = settings
            .get("llm_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "codex_oauth".to_string())
            .parse()?;

        Ok(Self {
            llm_provider: provider,
            paid_api_fallback_enabled: setting_bool(
                settings.get("paid_api_fallback_enabled"),
                false,
            ),
            codex_enabled: setting_bool(settings.get("codex_enabled"), false),
            codex_foreground_only: setting_bool(settings.get("codex_foreground_only"), true),
            codex_allow_background_tasks: setting_bool(
                settings.get("codex_allow_background_tasks"),
                false,
            ),
            codex_background_task_allowlist: setting_task_allowlist(
                settings.get("codex_background_task_allowlist"),
            ),
            xai_subscription_enabled: setting_bool(settings.get("xai_subscription_enabled"), false),
            xai_subscription_foreground_only: setting_bool(
                settings.get("xai_subscription_foreground_only"),
                true,
            ),
            xai_subscription_allow_background_tasks: setting_bool(
                settings.get("xai_subscription_allow_background_tasks"),
                false,
            ),
            xai_subscription_background_task_allowlist: setting_task_allowlist(
                settings.get("xai_subscription_background_task_allowlist"),
            ),
            prompt_optimization_allow_paid_api: setting_bool(
                settings.get("prompt_optimization_allow_paid_api"),
                false,
            ),
            skillfile_optimization_allow_paid_api: setting_bool(
                settings.get("skillfile_optimization_allow_paid_api"),
                false,
            ),
        })
    }

    /// Build config for migrated legacy OpenAI-compatible chat call paths.
    ///
    /// `RuntimeSettings::new()` seeds `llm_provider=codex_oauth`, but Phase 1
    /// intentionally left existing direct OpenAI-compatible call sites
    /// unchanged. During Phase 2 migration, treat the seeded default as
    /// "not explicitly selected" so production keeps its current behavior.
    /// Once the owner explicitly stores `llm_provider`, the router enforces
    /// the selected provider and all cost-safety gates.
    pub fn from_runtime_settings_for_legacy_chat_path(
        settings: &RuntimeSettings,
    ) -> Result<Self, LlmProviderError> {
        let mut config = Self::from_runtime_settings(settings)?;
        if settings.get_if_stored("llm_provider").is_none() {
            config.llm_provider = LlmProviderKind::OpenAiCompat;
        }
        Ok(config)
    }

    /// Whether this subscription provider may serve `task_class`.
    ///
    /// Foreground classes are allowed. Other classes need the global background
    /// flag or an explicit per-class grant. A denied class must fail closed
    /// before any paid-API fallback is considered.
    pub fn allows_subscription_background(
        &self,
        provider: LlmProviderKind,
        task_class: LlmTaskClass,
    ) -> bool {
        if task_class.is_foreground() {
            return true;
        }
        match provider {
            LlmProviderKind::CodexOAuth => {
                self.codex_allow_background_tasks
                    || self.codex_background_task_allowlist.contains(&task_class)
            }
            LlmProviderKind::XaiSubscription => {
                self.xai_subscription_allow_background_tasks
                    || self
                        .xai_subscription_background_task_allowlist
                        .contains(&task_class)
            }
            LlmProviderKind::OpenAiCompat
            | LlmProviderKind::OpenRouter
            | LlmProviderKind::OpenAiApi
            | LlmProviderKind::Local => false,
        }
    }
}

impl Default for LlmRouterConfig {
    fn default() -> Self {
        Self {
            llm_provider: LlmProviderKind::CodexOAuth,
            paid_api_fallback_enabled: false,
            codex_enabled: false,
            codex_foreground_only: true,
            codex_allow_background_tasks: false,
            codex_background_task_allowlist: Vec::new(),
            xai_subscription_enabled: false,
            xai_subscription_foreground_only: true,
            xai_subscription_allow_background_tasks: false,
            xai_subscription_background_task_allowlist: Vec::new(),
            prompt_optimization_allow_paid_api: false,
            skillfile_optimization_allow_paid_api: false,
        }
    }
}

#[derive(Clone)]
pub struct RouteDecision {
    pub backend: Arc<dyn LlmBackend>,
    pub provider: LlmProviderKind,
    pub model: String,
    pub is_fallback: bool,
}

impl fmt::Debug for RouteDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteDecision")
            .field("backend_provider", &self.backend.provider_name())
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("is_fallback", &self.is_fallback)
            .finish()
    }
}

pub struct LlmRouter {
    openai_compat: Arc<dyn LlmBackend>,
    codex: Option<Arc<dyn LlmBackend>>,
    xai_subscription: Option<Arc<dyn LlmBackend>>,
    local: Option<Arc<dyn LlmBackend>>,
    config: LlmRouterConfig,
}

impl LlmRouter {
    pub fn new(openai_compat: Arc<dyn LlmBackend>, config: LlmRouterConfig) -> Self {
        Self {
            openai_compat,
            codex: None,
            xai_subscription: None,
            local: None,
            config,
        }
    }

    pub fn with_codex(mut self, codex: Arc<dyn LlmBackend>) -> Self {
        self.codex = Some(codex);
        self
    }

    pub fn with_xai_subscription(mut self, xai_subscription: Arc<dyn LlmBackend>) -> Self {
        self.xai_subscription = Some(xai_subscription);
        self
    }

    pub fn with_local(mut self, local: Arc<dyn LlmBackend>) -> Self {
        self.local = Some(local);
        self
    }

    pub fn config(&self) -> &LlmRouterConfig {
        &self.config
    }

    pub fn route_chat(&self, req: &AegisChatRequest) -> Result<RouteDecision, LlmProviderError> {
        self.route_provider(self.config.llm_provider, req.metadata.task_class)
    }

    pub fn route_provider(
        &self,
        provider: LlmProviderKind,
        task_class: LlmTaskClass,
    ) -> Result<RouteDecision, LlmProviderError> {
        if task_class.is_embedding() {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::EmbeddingBlocked,
                "embeddings must not route through the chat LLM router or Codex OAuth",
            ));
        }

        match provider {
            LlmProviderKind::CodexOAuth => self.route_codex(task_class),
            LlmProviderKind::XaiSubscription => self.route_xai_subscription(task_class),
            LlmProviderKind::OpenAiCompat
            | LlmProviderKind::OpenRouter
            | LlmProviderKind::OpenAiApi => {
                self.refuse_paid_prompt_optimization(provider, task_class)?;
                Ok(self.openai_decision(provider, false))
            }
            LlmProviderKind::Local => self.local_decision(false),
        }
    }

    fn refuse_paid_prompt_optimization(
        &self,
        provider: LlmProviderKind,
        task_class: LlmTaskClass,
    ) -> Result<(), LlmProviderError> {
        if !provider.is_paid_api() {
            return Ok(());
        }
        let message = match task_class {
            LlmTaskClass::PromptOptimization if !self.config.prompt_optimization_allow_paid_api => {
                "PromptOptimization refuses paid API spend unless prompt_optimization_allow_paid_api=true; failing closed"
            }
            LlmTaskClass::SkillOptimization
                if !self.config.skillfile_optimization_allow_paid_api =>
            {
                "SkillOptimization refuses paid API spend unless skillfile_optimization_allow_paid_api=true; failing closed"
            }
            _ => return Ok(()),
        };
        Err(LlmProviderError::new(
            LlmProviderErrorKind::PaidApiFallbackDisabled,
            message,
        ))
    }

    fn route_codex(&self, task_class: LlmTaskClass) -> Result<RouteDecision, LlmProviderError> {
        if !self.config.codex_enabled {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::ProviderDisabled,
                "Codex OAuth provider is selected but codex_enabled=false",
            ));
        }

        if !self
            .config
            .allows_subscription_background(LlmProviderKind::CodexOAuth, task_class)
        {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::BackgroundTaskBlocked,
                format!(
                    "Codex OAuth is not allowed for task class {task_class:?} by default; failing closed without paid API fallback"
                ),
            ));
        }

        if let Some(codex) = &self.codex {
            return Ok(RouteDecision {
                backend: Arc::clone(codex),
                provider: LlmProviderKind::CodexOAuth,
                model: codex.model_name().to_string(),
                is_fallback: false,
            });
        }

        if self.local.is_some() {
            return self.local_decision(true);
        }

        if self.config.paid_api_fallback_enabled {
            self.refuse_paid_prompt_optimization(LlmProviderKind::OpenAiCompat, task_class)?;
            return Ok(self.openai_decision(LlmProviderKind::OpenAiCompat, true));
        }

        Err(LlmProviderError::new(
            LlmProviderErrorKind::PaidApiFallbackDisabled,
            "Codex OAuth backend is unavailable and paid_api_fallback_enabled=false; refusing to use OpenAI-compatible API credits",
        ))
    }

    fn route_xai_subscription(
        &self,
        task_class: LlmTaskClass,
    ) -> Result<RouteDecision, LlmProviderError> {
        if !self.config.xai_subscription_enabled {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::ProviderDisabled,
                "Grok subscription provider is selected but xai_subscription_enabled=false",
            ));
        }

        if !self
            .config
            .allows_subscription_background(LlmProviderKind::XaiSubscription, task_class)
        {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::BackgroundTaskBlocked,
                format!(
                    "Grok subscription is not allowed for task class {task_class:?} by default; failing closed without paid API fallback"
                ),
            ));
        }

        if let Some(xai_subscription) = &self.xai_subscription {
            return Ok(RouteDecision {
                backend: Arc::clone(xai_subscription),
                provider: LlmProviderKind::XaiSubscription,
                model: xai_subscription.model_name().to_string(),
                is_fallback: false,
            });
        }

        if self.local.is_some() {
            return self.local_decision(true);
        }

        if self.config.paid_api_fallback_enabled {
            self.refuse_paid_prompt_optimization(LlmProviderKind::OpenAiCompat, task_class)?;
            return Ok(self.openai_decision(LlmProviderKind::OpenAiCompat, true));
        }

        Err(LlmProviderError::new(
            LlmProviderErrorKind::PaidApiFallbackDisabled,
            "Grok subscription backend is unavailable and paid_api_fallback_enabled=false; refusing to use OpenAI-compatible API credits",
        ))
    }

    fn openai_decision(&self, provider: LlmProviderKind, is_fallback: bool) -> RouteDecision {
        RouteDecision {
            backend: Arc::clone(&self.openai_compat),
            provider,
            model: self.openai_compat.model_name().to_string(),
            is_fallback,
        }
    }

    fn local_decision(&self, is_fallback: bool) -> Result<RouteDecision, LlmProviderError> {
        let Some(local) = &self.local else {
            return Err(LlmProviderError::new(
                LlmProviderErrorKind::ProviderUnavailable,
                "local LLM provider is selected but no local backend is configured",
            ));
        };

        Ok(RouteDecision {
            backend: Arc::clone(local),
            provider: LlmProviderKind::Local,
            model: local.model_name().to_string(),
            is_fallback,
        })
    }
}

fn setting_task_allowlist(value: Option<Value>) -> Vec<LlmTaskClass> {
    let raw = match value {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        Some(Value::String(text)) => text
            .split([',', '\n'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut allowlist = Vec::new();
    for name in raw {
        if let Some(task_class) = LlmTaskClass::from_setting_name(&name)
            && !allowlist.contains(&task_class)
        {
            allowlist.push(task_class);
        }
    }
    allowlist
}

fn setting_bool(value: Option<Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(v)) => v,
        Some(Value::String(v)) => v.eq_ignore_ascii_case("true"),
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::llm::backend::LlmBackendFuture;
    use crate::llm::types::{
        AegisChatResponse, AegisMessage, AegisMessageRole, AegisRequestMetadata,
    };

    use super::*;

    struct MockBackend {
        provider: &'static str,
        model: String,
    }

    impl MockBackend {
        fn new(provider: &'static str, model: &str) -> Self {
            Self {
                provider,
                model: model.to_string(),
            }
        }
    }

    impl LlmBackend for MockBackend {
        fn chat(&self, _req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
            Box::pin(async {
                Ok(AegisChatResponse {
                    message: AegisMessage {
                        role: AegisMessageRole::Assistant,
                        content: String::new(),
                    },
                    tool_calls: vec![],
                    usage: None,
                    finish_reason: None,
                    provider_request_id: None,
                })
            })
        }

        fn provider_name(&self) -> &'static str {
            self.provider
        }

        fn model_name(&self) -> &str {
            &self.model
        }
    }

    fn chat_req(task_class: LlmTaskClass) -> AegisChatRequest {
        AegisChatRequest {
            model: "requested-model".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            tools: vec![],
            tool_choice: None,
            metadata: AegisRequestMetadata {
                task_class,
                request_id: None,
                chat_id: None,
                user_visible: task_class.is_foreground(),
                reasoning_effort: None,
            },
        }
    }

    fn router(config: LlmRouterConfig) -> LlmRouter {
        LlmRouter::new(
            Arc::new(MockBackend::new("openai_compat", "paid-model")),
            config,
        )
    }

    #[test]
    fn unknown_provider_does_not_use_paid_api() {
        let err = "surprise-provider"
            .parse::<LlmProviderKind>()
            .expect_err("unknown provider must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::UnknownProvider);
    }

    #[test]
    fn provider_json_strings_match_display_and_parse_contract() {
        let cases = [
            (LlmProviderKind::CodexOAuth, "codex_oauth"),
            (LlmProviderKind::XaiSubscription, "xai_subscription"),
            (LlmProviderKind::OpenAiCompat, "openai_compat"),
            (LlmProviderKind::OpenRouter, "openrouter"),
            (LlmProviderKind::OpenAiApi, "openai_api"),
            (LlmProviderKind::Local, "local"),
        ];

        for (provider, expected) in cases {
            assert_eq!(provider.to_string(), expected);
            assert_eq!(expected.parse::<LlmProviderKind>().unwrap(), provider);
            assert_eq!(
                serde_json::to_value(provider).unwrap(),
                serde_json::Value::String(expected.to_string())
            );
            assert_eq!(
                serde_json::from_value::<LlmProviderKind>(serde_json::Value::String(
                    expected.to_string()
                ))
                .unwrap(),
                provider
            );
        }

        assert_eq!(
            "grok_subscription".parse::<LlmProviderKind>().unwrap(),
            LlmProviderKind::XaiSubscription
        );
    }

    #[test]
    fn codex_unavailable_does_not_use_openai_key_when_paid_fallback_disabled() {
        let config = LlmRouterConfig {
            codex_enabled: true,
            paid_api_fallback_enabled: false,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect_err("paid API fallback must be blocked by default");
        assert_eq!(err.kind, LlmProviderErrorKind::PaidApiFallbackDisabled);
    }

    #[test]
    fn background_heartbeat_does_not_route_to_codex_by_default() {
        let config = LlmRouterConfig {
            codex_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::BackgroundHeartbeat))
            .expect_err("background heartbeat must not route to Codex by default");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);
    }

    #[test]
    fn background_codex_requires_allow_background_even_when_foreground_only_false() {
        let config = LlmRouterConfig {
            codex_enabled: true,
            codex_foreground_only: false,
            codex_allow_background_tasks: false,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::BackgroundHeartbeat))
            .expect_err("background Codex requires explicit codex_allow_background_tasks=true");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);
    }

    #[test]
    fn embedding_never_routes_to_codex() {
        let config = LlmRouterConfig {
            codex_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::Embedding))
            .expect_err("embedding task class must be blocked");
        assert_eq!(err.kind, LlmProviderErrorKind::EmbeddingBlocked);
    }

    #[test]
    fn foreground_request_routes_to_codex_only_when_codex_enabled() {
        let err = router(LlmRouterConfig::default())
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect_err("codex_enabled=false must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::ProviderDisabled);

        let config = LlmRouterConfig {
            codex_enabled: true,
            ..LlmRouterConfig::default()
        };
        let decision = router(config)
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect("foreground request can use enabled Codex backend");
        assert_eq!(decision.provider, LlmProviderKind::CodexOAuth);
        assert_eq!(decision.model, "codex-model");
        assert!(!decision.is_fallback);
    }

    #[test]
    fn paid_api_fallback_requires_explicit_setting() {
        let config = LlmRouterConfig {
            codex_enabled: true,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        };
        let decision = router(config)
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect("explicit paid fallback may use OpenAI-compatible backend");
        assert_eq!(decision.provider, LlmProviderKind::OpenAiCompat);
        assert_eq!(decision.model, "paid-model");
        assert!(decision.is_fallback);
    }

    #[test]
    fn xai_subscription_unavailable_does_not_use_openai_key_when_paid_fallback_disabled() {
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: false,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect_err("paid API fallback must be blocked by default");
        assert_eq!(err.kind, LlmProviderErrorKind::PaidApiFallbackDisabled);
    }

    #[test]
    fn xai_subscription_background_is_blocked_by_default() {
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::BackgroundHeartbeat))
            .expect_err("background heartbeat must not route to Grok subscription by default");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);
    }

    #[test]
    fn foreground_request_routes_to_xai_subscription_only_when_enabled() {
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect_err("xai_subscription_enabled=false must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::ProviderDisabled);

        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            ..LlmRouterConfig::default()
        };
        let decision = router(config)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect("foreground request can use enabled Grok subscription backend");
        assert_eq!(decision.provider, LlmProviderKind::XaiSubscription);
        assert_eq!(decision.model, "grok-subscription-model");
        assert!(!decision.is_fallback);
    }

    #[test]
    fn legacy_chat_path_uses_openai_compat_when_provider_is_only_seeded_default() {
        let settings = RuntimeSettings::new();
        let config =
            LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(&settings).unwrap();

        assert_eq!(config.llm_provider, LlmProviderKind::OpenAiCompat);
        let decision = router(config)
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect("unstored default must preserve existing direct OpenAI-compatible behavior");
        assert_eq!(decision.provider, LlmProviderKind::OpenAiCompat);
        assert!(!decision.is_fallback);
    }

    #[test]
    fn legacy_chat_path_enforces_explicit_codex_selection() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "llm_provider",
                serde_json::Value::String("codex_oauth".to_string()),
            )
            .unwrap();
        let config =
            LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(&settings).unwrap();

        assert_eq!(config.llm_provider, LlmProviderKind::CodexOAuth);
        let err = router(config)
            .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
            .expect_err("explicit Codex selection must enforce codex_enabled");
        assert_eq!(err.kind, LlmProviderErrorKind::ProviderDisabled);
    }

    #[test]
    fn prompt_optimization_is_blocked_unless_granted_alone() {
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect_err("ungranted prompt optimization must fail closed");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);
        assert!(err.message.contains("failing closed"));

        let mut granted = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: false,
            xai_subscription_background_task_allowlist: vec![LlmTaskClass::PromptOptimization],
            ..LlmRouterConfig::default()
        };
        let decision = router(granted.clone())
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect("an explicit per-class grant routes to the subscription");
        assert_eq!(decision.provider, LlmProviderKind::XaiSubscription);
        assert!(!decision.is_fallback);
        assert!(!decision.provider.is_paid_api());

        let heartbeat = router(granted.clone())
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::BackgroundHeartbeat))
            .expect_err("granting prompt optimization must not open other background classes");
        assert_eq!(heartbeat.kind, LlmProviderErrorKind::BackgroundTaskBlocked);

        granted.llm_provider = LlmProviderKind::CodexOAuth;
        granted.codex_enabled = true;
        granted.codex_background_task_allowlist = vec![LlmTaskClass::PromptOptimization];
        granted.xai_subscription_background_task_allowlist.clear();
        let codex = router(granted)
            .with_codex(Arc::new(MockBackend::new("codex_oauth", "codex-model")))
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect("codex per-class grant routes prompt optimization");
        assert_eq!(codex.provider, LlmProviderKind::CodexOAuth);
    }

    #[test]
    fn prompt_optimization_allowlist_roundtrips_from_settings() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "xai_subscription_background_task_allowlist",
                serde_json::json!(["prompt_optimization", "not_a_class", "prompt_optimization"]),
            )
            .unwrap();
        settings
            .set(
                "codex_background_task_allowlist",
                serde_json::json!("prompt_optimization,review_guard"),
            )
            .unwrap();
        let config = LlmRouterConfig::from_runtime_settings(&settings).unwrap();
        assert_eq!(
            config.xai_subscription_background_task_allowlist,
            vec![LlmTaskClass::PromptOptimization]
        );
        assert_eq!(
            config.codex_background_task_allowlist,
            vec![LlmTaskClass::PromptOptimization, LlmTaskClass::ReviewGuard]
        );
        assert!(!config.paid_api_fallback_enabled);
        assert!(config.allows_subscription_background(
            LlmProviderKind::XaiSubscription,
            LlmTaskClass::PromptOptimization
        ));
        assert!(!config.allows_subscription_background(
            LlmProviderKind::XaiSubscription,
            LlmTaskClass::Summarization
        ));
    }

    #[test]
    fn denied_prompt_optimization_does_not_select_paid_fallback() {
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(config)
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect_err("denied background class must not fall through to paid API");
        assert_eq!(err.kind, LlmProviderErrorKind::BackgroundTaskBlocked);
    }

    #[test]
    fn paid_primary_with_fallback_on_still_fail_closes_prompt_optimization() {
        let blocked = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        };
        let err = router(blocked)
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect_err("paid primary must not skip the class gate");
        assert_eq!(err.kind, LlmProviderErrorKind::PaidApiFallbackDisabled);
        assert!(err.message.contains("prompt_optimization_allow_paid_api"));
        assert!(err.message.contains("failing closed"));

        let foreground = router(LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        })
        .route_chat(&chat_req(LlmTaskClass::ForegroundUserRequest))
        .expect("foreground chat on a paid primary is unchanged");
        assert_eq!(foreground.provider, LlmProviderKind::OpenAiCompat);

        let mut opted_in = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            prompt_optimization_allow_paid_api: true,
            ..LlmRouterConfig::default()
        };
        let allowed = router(opted_in.clone())
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect("explicit paid opt-in is the only paid path");
        assert!(allowed.provider.is_paid_api());

        opted_in.llm_provider = LlmProviderKind::XaiSubscription;
        opted_in.xai_subscription_enabled = true;
        opted_in.xai_subscription_background_task_allowlist =
            vec![LlmTaskClass::PromptOptimization];
        opted_in.prompt_optimization_allow_paid_api = false;
        let missing_backend = router(opted_in)
            .route_chat(&chat_req(LlmTaskClass::PromptOptimization))
            .expect_err("granted subscription with no backend must not bill");
        assert_eq!(
            missing_backend.kind,
            LlmProviderErrorKind::PaidApiFallbackDisabled
        );
        assert!(
            missing_backend
                .message
                .contains("prompt_optimization_allow_paid_api")
        );
    }

    #[test]
    fn skill_optimization_paid_gate_mirrors_prompt_optimization() {
        let blocked = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            prompt_optimization_allow_paid_api: true,
            ..LlmRouterConfig::default()
        };
        let err = router(blocked)
            .route_chat(&chat_req(LlmTaskClass::SkillOptimization))
            .expect_err("opting in prompt optimization must not spend for skill optimization");
        assert_eq!(err.kind, LlmProviderErrorKind::PaidApiFallbackDisabled);
        assert!(
            err.message
                .contains("skillfile_optimization_allow_paid_api")
        );
        assert!(err.message.contains("failing closed"));

        let granted_prompt_only = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: true,
            xai_subscription_background_task_allowlist: vec![LlmTaskClass::PromptOptimization],
            ..LlmRouterConfig::default()
        };
        let denied = router(granted_prompt_only)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::SkillOptimization))
            .expect_err("a prompt_optimization grant must not open skill optimization");
        assert_eq!(denied.kind, LlmProviderErrorKind::BackgroundTaskBlocked);

        let granted_skill = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: false,
            skillfile_optimization_allow_paid_api: false,
            xai_subscription_background_task_allowlist: vec![LlmTaskClass::SkillOptimization],
            ..LlmRouterConfig::default()
        };
        let decision = router(granted_skill)
            .with_xai_subscription(Arc::new(MockBackend::new(
                "xai_subscription",
                "grok-subscription-model",
            )))
            .route_chat(&chat_req(LlmTaskClass::SkillOptimization))
            .expect("an explicit skill_optimization grant routes to the subscription");
        assert_eq!(decision.provider, LlmProviderKind::XaiSubscription);
        assert!(!decision.provider.is_paid_api());
    }
}
