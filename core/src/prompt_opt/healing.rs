use std::path::PathBuf;

use crate::llm::{LlmProviderKind, LlmRouter, LlmRouterConfig, LlmTaskClass};
use crate::settings::RuntimeSettings;

use super::llm::{CountingPaidBackend, RoutedPromptLlm};
use super::model::{
    Budget, Example, OptimizationTask, PromptOptError, Rubric, RubricDimension, RunPolicy,
};
use super::optimize::{OptimizationReport, run_optimization};
use super::registry::PromptRegistry;

/// Saved-settings view of one healing subscription round.
///
/// The helper copies the allowlist and model from `settings`. It does not
/// grant `prompt_optimization` by itself, and it forces paid fallback off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealingSubscriptionPlan {
    pub model: String,
    pub allowlist: Vec<LlmTaskClass>,
    pub allow_all_background: bool,
    pub config: LlmRouterConfig,
}

pub const HEALING_TASK_NAME: &str = "healing_image_prompt_expansion";

/// First validation task: expand a short idea into a healing-style image prompt.
/// The rubric is a matter of taste (「有生气但不乱」), not a ban on richness.
pub fn healing_image_task() -> OptimizationTask {
    OptimizationTask {
        name: HEALING_TASK_NAME.to_string(),
        io_description: "Expand a short healing-style image idea into one image-generation prompt."
            .to_string(),
        baseline_instruction: "Expand the short idea into one healing-style image prompt. Describe the subject, the light, and the place in plain words.".to_string(),
        rubric: Rubric {
            taste: "有生气但不乱".to_string(),
            dimensions: vec![
                RubricDimension {
                    id: "vitality".to_string(),
                    label: "有生气".to_string(),
                    weight: 0.5,
                    guidance: "画面要有生命力：光、动静、温度，或一个活着的主体。丰富可以，只要还是同一口气。有生气但不乱。".to_string(),
                },
                RubricDimension {
                    id: "not_chaotic".to_string(),
                    label: "不乱".to_string(),
                    weight: 0.5,
                    guidance: "画面读得成一个场景。层次和丰富都可以，只要还是同一个场景，不要无关物体、互打架的风格或视觉噪音。".to_string(),
                },
            ],
        },
        examples: vec![
            Example {
                id: "cat-window".to_string(),
                input: "清晨窗边的猫".to_string(),
            },
            Example {
                id: "rain-path".to_string(),
                input: "雨后的小路".to_string(),
            },
            Example {
                id: "warm-kitchen".to_string(),
                input: "暖色厨房".to_string(),
            },
            Example {
                id: "afternoon-desk".to_string(),
                input: "午后书桌".to_string(),
            },
        ],
        validation_ratio: 0.5,
    }
}

pub struct LiveCredentialPaths {
    pub identity_key: PathBuf,
    pub credential_db: PathBuf,
}

impl LiveCredentialPaths {
    pub fn from_env() -> Self {
        let identity_key = std::env::var("IDENTITY_KEY_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./aegis_runtime_identity.key"));
        let credential_db = std::env::var("CREDENTIAL_DB_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./aegis_runtime_credentials.db"));
        Self {
            identity_key,
            credential_db,
        }
    }
}

/// Why a subscription-quota round cannot start. `None` means both files exist
/// and the caller may try to load the Grok credential.
pub fn live_credential_blocker(paths: &LiveCredentialPaths) -> Option<String> {
    let mut missing = Vec::new();
    if !paths.identity_key.is_file() {
        missing.push(format!("identity key {}", paths.identity_key.display()));
    }
    if !paths.credential_db.is_file() {
        missing.push(format!("credential db {}", paths.credential_db.display()));
    }
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "Subscription credential is unavailable ({}). Prompt optimization did not call a model and did not use a paid API key. Grant prompt_optimization on xai_subscription_background_task_allowlist or codex_background_task_allowlist only after Grok or Codex subscription auth is stored, and keep paid_api_fallback_enabled=false.",
        missing.join(", ")
    ))
}

/// Build the router config for a healing round from the same settings the
/// owner saved. An empty allowlist fails closed. The model is
/// `xai_subscription_model` from those settings. The settings registry owns
/// the default model id; this file does not hardcode one.
pub fn healing_subscription_plan(
    settings: &RuntimeSettings,
) -> Result<HealingSubscriptionPlan, PromptOptError> {
    let mut config = LlmRouterConfig::from_runtime_settings(settings).map_err(|err| {
        PromptOptError::Routing {
            kind: err.kind,
            message: err.message,
        }
    })?;
    config.paid_api_fallback_enabled = false;
    config.prompt_optimization_allow_paid_api = false;
    if !config.allows_subscription_background(
        LlmProviderKind::XaiSubscription,
        LlmTaskClass::PromptOptimization,
    ) {
        return Err(PromptOptError::FailClosed {
            kind: crate::llm::LlmProviderErrorKind::BackgroundTaskBlocked,
            message: "prompt_optimization is not granted on xai_subscription_background_task_allowlist; failing closed without self-grant or paid API fallback".to_string(),
        });
    }
    let model = settings
        .get("xai_subscription_model")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| PromptOptError::invalid("xai_subscription_model is empty"))?;
    Ok(HealingSubscriptionPlan {
        allowlist: config.xai_subscription_background_task_allowlist.clone(),
        allow_all_background: config.xai_subscription_allow_background_tasks,
        model,
        config,
    })
}

/// One subscription-quota round for the healing-image task.
///
/// `paid_api_fallback_enabled` is forced false. The paid backend counts calls
/// and returns an error if selected, so this path cannot spend API credits.
/// The Grok backend allowlist is the saved allowlist, not a hardcoded grant.
pub async fn run_healing_subscription_round_from_files(
    paths: &LiveCredentialPaths,
    settings: &RuntimeSettings,
) -> Result<OptimizationReport, PromptOptError> {
    let plan = healing_subscription_plan(settings)?;
    if let Some(blocker) = live_credential_blocker(paths) {
        return Err(PromptOptError::FailClosed {
            kind: crate::llm::LlmProviderErrorKind::AuthRequired,
            message: blocker,
        });
    }
    let aes_key =
        crate::credential_store::derive_key_from_ed25519(&read_identity_key(&paths.identity_key)?);
    let connection = crate::encrypted_sqlite::open(&paths.credential_db)
        .map_err(|err| PromptOptError::invalid(format!("could not open credential db: {err}")))?;
    let store = crate::credential_store::CredentialStore::open(connection, aes_key)
        .map_err(|err| PromptOptError::invalid(format!("could not unlock credential db: {err}")))?;
    let credential = crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(|err| PromptOptError::Routing {
            kind: err.kind,
            message: err.message,
        })?
        .ok_or_else(|| PromptOptError::FailClosed {
            kind: crate::llm::LlmProviderErrorKind::AuthRequired,
            message:
                "credential db has no Grok subscription OAuth credential; refusing paid API spend"
                    .to_string(),
        })?;

    let paid_hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut config = plan.config;
    config.llm_provider = LlmProviderKind::XaiSubscription;
    config.xai_subscription_enabled = true;
    let subscription = crate::llm::XaiSubscriptionBackend::with_credential(plan.model, credential)
        .with_background_task_allowlist(plan.allowlist)
        .with_allow_all_background(plan.allow_all_background);
    let router = LlmRouter::new(
        std::sync::Arc::new(CountingPaidBackend::new(paid_hits)),
        config,
    )
    .with_xai_subscription(std::sync::Arc::new(subscription));
    let task = healing_image_task();
    let registry = PromptRegistry::new(task.baseline_instruction.clone());
    run_optimization(
        &task,
        registry,
        &RoutedPromptLlm::new(router),
        Budget {
            max_calls: 8,
            max_cost_micros: 3_000_000,
            cost_per_call_micros: 50_000,
            max_tokens: 20_000,
            tokens_per_call: 1_200,
            calls_used: 0,
            cost_used_micros: 0,
            tokens_used: 0,
        },
        1,
        RunPolicy::explicit(),
    )
    .await
}

fn read_identity_key(path: &std::path::Path) -> Result<[u8; 32], PromptOptError> {
    let hex_text = std::fs::read_to_string(path).map_err(|err| {
        PromptOptError::invalid(format!(
            "could not read identity key {}: {err}",
            path.display()
        ))
    })?;
    let bytes = hex::decode(hex_text.trim())
        .map_err(|err| PromptOptError::invalid(format!("identity key is not hex: {err}")))?;
    bytes
        .try_into()
        .map_err(|_| PromptOptError::invalid("identity key file must be exactly 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healing_rubric_is_alive_but_not_chaotic() {
        let task = healing_image_task();
        assert_eq!(task.rubric.taste, "有生气但不乱");
        let guidance = task
            .rubric
            .dimensions
            .iter()
            .map(|dimension| dimension.guidance.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(guidance.contains("有生气但不乱"));
        assert!(!guidance.to_ascii_lowercase().contains("no stacking"));
        assert!(!guidance.to_ascii_lowercase().contains("restrained"));
        assert!(!guidance.contains("禁止堆叠"));
    }

    #[test]
    fn missing_subscription_files_are_a_blocker() {
        let paths = LiveCredentialPaths {
            identity_key: PathBuf::from("/tmp/aegis-runtime-prompt-opt-missing.key"),
            credential_db: PathBuf::from("/tmp/aegis-runtime-prompt-opt-missing.db"),
        };
        let blocker = live_credential_blocker(&paths).expect("missing files block the live round");
        assert!(blocker.contains("paid_api_fallback_enabled=false"));
        assert!(blocker.contains("prompt_optimization"));
    }

    #[test]
    fn helper_fails_closed_when_saved_allowlist_omits_prompt_optimization() {
        let settings = RuntimeSettings::new();
        let err =
            healing_subscription_plan(&settings).expect_err("empty allowlist must fail closed");
        assert!(err.to_string().contains("failing closed"));
        assert!(err.to_string().contains("allowlist"));
        assert!(err.to_string().contains("self-grant"));
    }

    #[test]
    fn helper_uses_saved_allowlist_and_model() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "xai_subscription_background_task_allowlist",
                serde_json::json!(["prompt_optimization"]),
            )
            .unwrap();
        settings
            .set(
                "xai_subscription_model",
                serde_json::json!("subscription-model-from-settings"),
            )
            .unwrap();
        let plan = healing_subscription_plan(&settings).unwrap();
        assert_eq!(plan.model, "subscription-model-from-settings");
        assert_eq!(plan.allowlist, vec![LlmTaskClass::PromptOptimization]);
        assert!(!plan.allow_all_background);
        assert!(!plan.config.paid_api_fallback_enabled);
        assert!(!plan.config.prompt_optimization_allow_paid_api);
        assert_eq!(
            plan.model,
            settings
                .get("xai_subscription_model")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap()
        );
    }
}
