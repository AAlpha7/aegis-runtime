use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::llm::backend::LlmBackend;
use crate::llm::types::{
    AegisChatRequest, AegisChatResponse, AegisMessage, AegisMessageRole, AegisRequestMetadata,
};
use crate::llm::{LlmProviderError, LlmProviderErrorKind, LlmRouter, LlmTaskClass};

use super::model::{OptPhase, PromptOptError};

pub type PromptOptFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct PromptOptRequest {
    pub phase: OptPhase,
    pub system: String,
    pub user: String,
}

pub trait PromptOptChat: Send + Sync {
    fn complete(
        &self,
        request: PromptOptRequest,
    ) -> PromptOptFuture<'_, Result<String, PromptOptError>>;
}

/// Subscription-first chat adapter. Every completion calls
/// [`LlmRouter::route_chat`] with [`LlmTaskClass::PromptOptimization`].
pub struct RoutedPromptLlm {
    router: LlmRouter,
}

impl RoutedPromptLlm {
    pub fn new(router: LlmRouter) -> Self {
        Self { router }
    }

    pub fn router(&self) -> &LlmRouter {
        &self.router
    }
}

impl PromptOptChat for RoutedPromptLlm {
    fn complete(
        &self,
        request: PromptOptRequest,
    ) -> PromptOptFuture<'_, Result<String, PromptOptError>> {
        Box::pin(async move {
            let chat_request = AegisChatRequest {
                model: String::new(),
                messages: vec![
                    AegisMessage {
                        role: AegisMessageRole::System,
                        content: request.system,
                    },
                    AegisMessage {
                        role: AegisMessageRole::User,
                        content: request.user,
                    },
                ],
                temperature: Some(0.2),
                max_tokens: Some(1200),
                tools: Vec::new(),
                tool_choice: None,
                metadata: AegisRequestMetadata {
                    task_class: LlmTaskClass::PromptOptimization,
                    request_id: Some(format!("prompt-opt-{}", request.phase.as_str())),
                    chat_id: None,
                    user_visible: false,
                    reasoning_effort: None,
                },
            };
            let decision = self.router.route_chat(&chat_request)?;
            if decision.provider.is_paid_api()
                && !self.router.config().prompt_optimization_allow_paid_api
            {
                return Err(PromptOptError::FailClosed {
                    kind: LlmProviderErrorKind::PaidApiFallbackDisabled,
                    message: "PromptOptimization refuses paid API spend unless prompt_optimization_allow_paid_api=true; failing closed"
                        .to_string(),
                });
            }
            let response = decision.backend.chat(chat_request).await?;
            Ok(response.message.content)
        })
    }
}

/// Paid backend that records a call. Live runs install this so a routing bug
/// cannot spend API credits silently: the router must not select it while
/// `paid_api_fallback_enabled` is false.
pub struct CountingPaidBackend {
    hits: Arc<std::sync::atomic::AtomicU32>,
    model: String,
}

impl CountingPaidBackend {
    pub fn new(hits: Arc<std::sync::atomic::AtomicU32>) -> Self {
        Self {
            hits,
            model: "paid-model".to_string(),
        }
    }
}

impl LlmBackend for CountingPaidBackend {
    fn chat(
        &self,
        _req: AegisChatRequest,
    ) -> crate::llm::backend::LlmBackendFuture<'_, AegisChatResponse> {
        self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async {
            Err(LlmProviderError::new(
                LlmProviderErrorKind::PaidApiFallbackDisabled,
                "paid API backend was invoked for prompt optimization",
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::llm::backend::LlmBackendFuture;
    use crate::llm::{LlmProviderKind, LlmRouter, LlmRouterConfig};
    use crate::prompt_opt::healing::healing_image_task;
    use crate::prompt_opt::model::{Budget, RunPolicy};
    use crate::prompt_opt::optimize::run_optimization;
    use crate::prompt_opt::registry::PromptRegistry;

    struct CountingSubscription {
        hits: Arc<AtomicU32>,
        model: String,
    }

    impl LlmBackend for CountingSubscription {
        fn chat(&self, req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let task_class = req.metadata.task_class;
            Box::pin(async move {
                let _ = task_class;
                Ok(AegisChatResponse {
                    message: AegisMessage {
                        role: AegisMessageRole::Assistant,
                        content: "INSTRUCTION:\nunused\n".to_string(),
                    },
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: None,
                    provider_request_id: None,
                })
            })
        }

        fn provider_name(&self) -> &'static str {
            "xai_subscription"
        }

        fn model_name(&self) -> &str {
            &self.model
        }
    }

    #[tokio::test]
    async fn blocked_prompt_optimization_fails_closed_without_paid_calls() {
        let paid_hits = Arc::new(AtomicU32::new(0));
        let subscription_hits = Arc::new(AtomicU32::new(0));
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: true,
            ..LlmRouterConfig::default()
        };
        let router = LlmRouter::new(
            Arc::new(CountingPaidBackend::new(Arc::clone(&paid_hits))),
            config,
        )
        .with_xai_subscription(Arc::new(CountingSubscription {
            hits: Arc::clone(&subscription_hits),
            model: "grok-subscription-model".to_string(),
        }));
        let task = healing_image_task();
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let err = run_optimization(
            &task,
            registry,
            &RoutedPromptLlm::new(router),
            Budget {
                max_calls: 3,
                max_cost_micros: 1_000_000,
                cost_per_call_micros: 1,
                max_tokens: 20_000,
                tokens_per_call: 1,
                calls_used: 0,
                cost_used_micros: 0,
                tokens_used: 0,
            },
            1,
            RunPolicy::explicit(),
        )
        .await
        .expect_err("subscription background denial must fail the optimization");
        assert_eq!(
            err.provider_kind(),
            Some(LlmProviderErrorKind::BackgroundTaskBlocked)
        );
        assert_eq!(paid_hits.load(Ordering::SeqCst), 0);
        assert_eq!(subscription_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn selected_paid_provider_is_refused_when_fallback_is_off() {
        let paid_hits = Arc::new(AtomicU32::new(0));
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: false,
            ..LlmRouterConfig::default()
        };
        let router = LlmRouter::new(
            Arc::new(CountingPaidBackend::new(Arc::clone(&paid_hits))),
            config,
        );
        let err = RoutedPromptLlm::new(router)
            .complete(PromptOptRequest {
                phase: OptPhase::Propose,
                system: "system".to_string(),
                user: "user".to_string(),
            })
            .await
            .expect_err("paid provider must fail closed");
        assert_eq!(
            err.provider_kind(),
            Some(LlmProviderErrorKind::PaidApiFallbackDisabled)
        );
        assert_eq!(paid_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn paid_primary_with_fallback_on_still_fail_closes() {
        let paid_hits = Arc::new(AtomicU32::new(0));
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            prompt_optimization_allow_paid_api: false,
            ..LlmRouterConfig::default()
        };
        let router = LlmRouter::new(
            Arc::new(CountingPaidBackend::new(Arc::clone(&paid_hits))),
            config,
        );
        let err = RoutedPromptLlm::new(router)
            .complete(PromptOptRequest {
                phase: OptPhase::Propose,
                system: "system".to_string(),
                user: "user".to_string(),
            })
            .await
            .expect_err("paid primary must fail closed even when fallback is on");
        assert_eq!(
            err.provider_kind(),
            Some(LlmProviderErrorKind::PaidApiFallbackDisabled)
        );
        assert!(
            err.to_string()
                .contains("prompt_optimization_allow_paid_api")
        );
        assert_eq!(paid_hits.load(Ordering::SeqCst), 0);
    }
}
