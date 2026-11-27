use std::future::Future;
use std::pin::Pin;

use crate::llm::types::{AegisChatRequest, AegisMessage, AegisMessageRole, AegisRequestMetadata};
use crate::llm::{LlmProviderErrorKind, LlmRouter, LlmTaskClass};
use crate::prompt_opt::OptPhase;

use super::model::SkillOptError;

pub type SkillOptFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct SkillOptRequest {
    pub phase: OptPhase,
    pub system: String,
    pub user: String,
}

pub trait SkillOptChat: Send + Sync {
    fn complete(
        &self,
        request: SkillOptRequest,
    ) -> SkillOptFuture<'_, Result<String, SkillOptError>>;
}

/// Every completion calls [`LlmRouter::route_chat`] with
/// [`LlmTaskClass::SkillOptimization`].
pub struct RoutedSkillLlm {
    router: LlmRouter,
}

impl RoutedSkillLlm {
    pub fn new(router: LlmRouter) -> Self {
        Self { router }
    }

    pub fn router(&self) -> &LlmRouter {
        &self.router
    }
}

impl SkillOptChat for RoutedSkillLlm {
    fn complete(
        &self,
        request: SkillOptRequest,
    ) -> SkillOptFuture<'_, Result<String, SkillOptError>> {
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
                    task_class: LlmTaskClass::SkillOptimization,
                    request_id: Some(format!("skill-opt-{}", request.phase.as_str())),
                    chat_id: None,
                    user_visible: false,
                    reasoning_effort: None,
                },
            };
            let decision = self.router.route_chat(&chat_request)?;
            if decision.provider.is_paid_api()
                && !self.router.config().skillfile_optimization_allow_paid_api
            {
                return Err(SkillOptError::fail_closed(
                    LlmProviderErrorKind::PaidApiFallbackDisabled,
                    "SkillOptimization refuses paid API spend unless skillfile_optimization_allow_paid_api=true; failing closed",
                ));
            }
            let response = decision.backend.chat(chat_request).await?;
            Ok(response.message.content)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::llm::backend::{LlmBackend, LlmBackendFuture};
    use crate::llm::types::{AegisChatResponse, AegisMessage, AegisMessageRole};
    use crate::llm::{LlmProviderKind, LlmRouter, LlmRouterConfig};
    use crate::prompt_opt::{Budget, CountingPaidBackend, PromptRegistry, RunPolicy};
    use crate::skill_opt::optimize::run_skill_optimization;
    use crate::skill_opt::pilot::{BASELINE_SKILL_MARKDOWN, PILOT_SKILL_NAME, pilot_task};

    struct CountingSubscription {
        hits: Arc<AtomicU32>,
        model: String,
    }

    impl LlmBackend for CountingSubscription {
        fn chat(&self, _req: AegisChatRequest) -> LlmBackendFuture<'_, AegisChatResponse> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
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
    async fn blocked_skill_optimization_fails_closed_without_paid_calls() {
        let paid_hits = Arc::new(AtomicU32::new(0));
        let subscription_hits = Arc::new(AtomicU32::new(0));
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::XaiSubscription,
            xai_subscription_enabled: true,
            paid_api_fallback_enabled: true,
            prompt_optimization_allow_paid_api: true,
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
        let task = pilot_task(BASELINE_SKILL_MARKDOWN);
        let registry = PromptRegistry::new(task.baseline_instruction.clone());
        let err = run_skill_optimization(
            &task,
            registry,
            &RoutedSkillLlm::new(router),
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
            PILOT_SKILL_NAME,
        )
        .await
        .expect_err("ungranted skill optimization must fail closed");
        assert_eq!(
            err.provider_kind(),
            Some(LlmProviderErrorKind::BackgroundTaskBlocked)
        );
        assert_eq!(paid_hits.load(Ordering::SeqCst), 0);
        assert_eq!(subscription_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn paid_primary_with_fallback_on_still_fail_closes() {
        let paid_hits = Arc::new(AtomicU32::new(0));
        let config = LlmRouterConfig {
            llm_provider: LlmProviderKind::OpenAiCompat,
            paid_api_fallback_enabled: true,
            skillfile_optimization_allow_paid_api: false,
            ..LlmRouterConfig::default()
        };
        let router = LlmRouter::new(
            Arc::new(CountingPaidBackend::new(Arc::clone(&paid_hits))),
            config,
        );
        let err = RoutedSkillLlm::new(router)
            .complete(SkillOptRequest {
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
        assert!(
            err.to_string()
                .contains("skillfile_optimization_allow_paid_api")
        );
        assert_eq!(paid_hits.load(Ordering::SeqCst), 0);
    }
}
