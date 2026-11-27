use std::path::PathBuf;

use crate::llm::{LlmProviderErrorKind, LlmProviderKind, LlmRouter, LlmTaskClass};
use crate::prompt_opt::CountingPaidBackend;
use crate::settings::RuntimeSettings;

use super::llm::RoutedSkillLlm;
use super::model::SkillOptError;

/// Saved-settings view of one skill-file subscription round.
///
/// The helper copies the allowlist and model from `settings`. It does not
/// grant `skill_optimization` by itself, and it forces paid fallback off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSubscriptionPlan {
    pub model: String,
    pub allowlist: Vec<LlmTaskClass>,
    pub allow_all_background: bool,
    pub config: crate::llm::LlmRouterConfig,
}

pub struct SkillCredentialPaths {
    pub identity_key: PathBuf,
    pub credential_db: PathBuf,
}

impl SkillCredentialPaths {
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

pub fn skill_credential_blocker(paths: &SkillCredentialPaths) -> Option<String> {
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
        "Subscription credential is unavailable ({}). Skill optimization did not call a model and did not use a paid API key. Grant skill_optimization on xai_subscription_background_task_allowlist only after Grok subscription auth is stored, and keep paid_api_fallback_enabled=false.",
        missing.join(", ")
    ))
}

pub fn skill_subscription_plan(
    settings: &RuntimeSettings,
) -> Result<SkillSubscriptionPlan, SkillOptError> {
    let mut config = crate::llm::LlmRouterConfig::from_runtime_settings(settings)
        .map_err(SkillOptError::routing)?;
    config.paid_api_fallback_enabled = false;
    config.skillfile_optimization_allow_paid_api = false;
    if !config.allows_subscription_background(
        LlmProviderKind::XaiSubscription,
        LlmTaskClass::SkillOptimization,
    ) {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::BackgroundTaskBlocked,
            "skill_optimization is not granted on xai_subscription_background_task_allowlist; failing closed without self-grant or paid API fallback",
        ));
    }
    let model = settings
        .get("xai_subscription_model")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| SkillOptError::invalid("xai_subscription_model is empty"))?;
    Ok(SkillSubscriptionPlan {
        allowlist: config.xai_subscription_background_task_allowlist.clone(),
        allow_all_background: config.xai_subscription_allow_background_tasks,
        model,
        config,
    })
}

pub fn settings_from_db(path: &std::path::Path) -> Result<RuntimeSettings, SkillOptError> {
    let settings = RuntimeSettings::new();
    if !path.exists() {
        return Ok(settings);
    }
    let connection = crate::encrypted_sqlite::open(path).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::UnsupportedFeature,
            format!("could not open settings db {}: {err}", path.display()),
        )
    })?;
    let mut statement = connection
        .prepare("SELECT key, value FROM user_settings")
        .map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not read user_settings: {err}"),
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok((key, value))
        })
        .map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not query user_settings: {err}"),
            )
        })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::UnsupportedFeature,
                format!("could not read a user_settings row: {err}"),
            )
        })?;
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&value) {
            map.insert(key, json);
        }
    }
    settings.load_from_map(map);
    Ok(settings)
}

/// Build a Grok subscription chat for one explicit round.
///
/// `paid_api_fallback_enabled` and `skillfile_optimization_allow_paid_api` are
/// forced false. The paid backend returns an error if selected.
pub async fn open_subscription_chat(
    paths: &SkillCredentialPaths,
    plan: SkillSubscriptionPlan,
) -> Result<RoutedSkillLlm, SkillOptError> {
    if let Some(blocker) = skill_credential_blocker(paths) {
        return Err(SkillOptError::fail_closed(
            LlmProviderErrorKind::AuthRequired,
            blocker,
        ));
    }
    let aes_key =
        crate::credential_store::derive_key_from_ed25519(&read_identity_key(&paths.identity_key)?);
    let connection = crate::encrypted_sqlite::open(&paths.credential_db).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::AuthRequired,
            format!("could not open credential db: {err}"),
        )
    })?;
    let store =
        crate::credential_store::CredentialStore::open(connection, aes_key).map_err(|err| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::AuthRequired,
                format!("could not unlock credential db: {err}"),
            )
        })?;
    let credential = crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(SkillOptError::routing)?
        .ok_or_else(|| {
            SkillOptError::fail_closed(
                LlmProviderErrorKind::AuthRequired,
                "credential db has no Grok subscription OAuth credential; refusing paid API spend",
            )
        })?;
    let paid_hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut config = plan.config;
    config.llm_provider = LlmProviderKind::XaiSubscription;
    config.xai_subscription_enabled = true;
    config.paid_api_fallback_enabled = false;
    config.skillfile_optimization_allow_paid_api = false;
    let subscription = crate::llm::XaiSubscriptionBackend::with_credential(plan.model, credential)
        .with_background_task_allowlist(plan.allowlist)
        .with_allow_all_background(plan.allow_all_background);
    let router = LlmRouter::new(
        std::sync::Arc::new(CountingPaidBackend::new(paid_hits)),
        config,
    )
    .with_xai_subscription(std::sync::Arc::new(subscription));
    Ok(RoutedSkillLlm::new(router))
}

fn read_identity_key(path: &std::path::Path) -> Result<[u8; 32], SkillOptError> {
    let hex_text = std::fs::read_to_string(path).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::AuthRequired,
            format!("could not read identity key {}: {err}", path.display()),
        )
    })?;
    let bytes = hex::decode(hex_text.trim()).map_err(|err| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::AuthRequired,
            format!("identity key is not hex: {err}"),
        )
    })?;
    bytes.try_into().map_err(|_| {
        SkillOptError::fail_closed(
            LlmProviderErrorKind::AuthRequired,
            "identity key file must be exactly 32 bytes",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_fails_closed_when_saved_allowlist_omits_skill_optimization() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "xai_subscription_background_task_allowlist",
                serde_json::json!(["prompt_optimization"]),
            )
            .unwrap();
        let err = skill_subscription_plan(&settings)
            .expect_err("a prompt_optimization grant must not start skill evolution");
        assert!(err.to_string().contains("failing closed"));
        assert!(err.to_string().contains("skill_optimization"));
        assert!(err.to_string().contains("self-grant"));
    }

    #[test]
    fn helper_uses_saved_allowlist_and_forces_paid_off() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "xai_subscription_background_task_allowlist",
                serde_json::json!(["skill_optimization"]),
            )
            .unwrap();
        settings
            .set(
                "xai_subscription_model",
                serde_json::json!("subscription-model-from-settings"),
            )
            .unwrap();
        settings
            .set("paid_api_fallback_enabled", serde_json::json!(true))
            .unwrap();
        settings
            .set(
                "skillfile_optimization_allow_paid_api",
                serde_json::json!(true),
            )
            .unwrap();
        let plan = skill_subscription_plan(&settings).unwrap();
        assert_eq!(plan.model, "subscription-model-from-settings");
        assert_eq!(plan.allowlist, vec![LlmTaskClass::SkillOptimization]);
        assert!(!plan.config.paid_api_fallback_enabled);
        assert!(!plan.config.skillfile_optimization_allow_paid_api);
    }

    #[test]
    fn missing_subscription_files_are_a_blocker() {
        let paths = SkillCredentialPaths {
            identity_key: PathBuf::from("/tmp/aegis-runtime-skill-opt-missing.key"),
            credential_db: PathBuf::from("/tmp/aegis-runtime-skill-opt-missing.db"),
        };
        let blocker = skill_credential_blocker(&paths).expect("missing files block the round");
        assert!(blocker.contains("paid_api_fallback_enabled=false"));
        assert!(blocker.contains("skill_optimization"));
    }
}
