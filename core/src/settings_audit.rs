//! Runtime settings consistency audit.
//!
//! Catches combinations that look configured but cannot actually take over
//! (authorized backup with no model, provider with no model, paid fallback
//! with no credentials). Findings are owner-language; codes stay stable for
//! logs and the post-deploy check.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::google_auth::{CRED_SERVICE, REQUIRED_CRED_KEYS};
use crate::i18n::t;
use crate::model_catalog::{ModelCatalog, catalog_provider_id};
use crate::settings::RuntimeSettings;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SettingsFinding {
    pub code: &'static str,
    /// `"attention"` or `"notice"`.
    pub severity: &'static str,
    /// Owner-facing message in the requested locale.
    pub message: String,
}

const SEVERITY_ATTENTION: &str = "attention";

/// Env keys the OpenAI-compatible / paid-fallback path is built from
/// (`main.rs` + `main_skill_loader.rs`). Placeholder values count as missing.
const PAID_PROVIDER_ENV_KEYS: &[&str] = &["OPENAI_API_KEY", "OPENROUTER_API_KEY"];

const DEFAULT_CREDENTIAL_DB_PATH: &str = "./aegis_runtime_credentials.db";

pub fn audit_runtime_settings(
    settings: &RuntimeSettings,
    locale: &str,
    catalog: Option<&ModelCatalog>,
) -> Vec<SettingsFinding> {
    audit_runtime_settings_with(settings, locale, catalog, google_routing_configured())
}

/// Pure core of [`audit_runtime_settings`] with the "Google is configured"
/// predicate injected, so the executor-off rule is testable without a
/// credential store or process env.
pub(crate) fn audit_runtime_settings_with(
    settings: &RuntimeSettings,
    locale: &str,
    catalog: Option<&ModelCatalog>,
    google_configured: bool,
) -> Vec<SettingsFinding> {
    let mut findings = Vec::new();

    let fallback_authorized = settings.is_enabled("fallback_model_authorized");
    let fallback_model = setting_str(settings, "fallback_model");
    let llm_model = setting_str(settings, "llm_model");
    let llm_provider = setting_str(settings, "llm_provider");

    if fallback_authorized && fallback_model.is_empty() {
        findings.push(finding(
            "fallback_authorized_without_model",
            SEVERITY_ATTENTION,
            locale,
        ));
    }

    if !fallback_model.is_empty() && fallback_model == llm_model {
        findings.push(finding(
            "fallback_model_equals_primary",
            SEVERITY_ATTENTION,
            locale,
        ));
    }

    if llm_provider.eq_ignore_ascii_case("openrouter") && llm_model.is_empty() {
        findings.push(finding(
            "provider_without_model",
            SEVERITY_ATTENTION,
            locale,
        ));
    }

    if settings.is_enabled("paid_api_fallback_enabled") && !paid_provider_credentials_configured() {
        findings.push(finding(
            "paid_fallback_without_provider",
            SEVERITY_ATTENTION,
            locale,
        ));
    }

    // Google data is routed through the external executor, so an executor
    // that is off only matters once Google is actually connected. A fresh
    // install with no Google at all must not start with this warning.
    if google_configured && !settings.is_enabled("external_executor_enabled") {
        findings.push(finding(
            "external_executor_disabled_but_google_routing",
            SEVERITY_ATTENTION,
            locale,
        ));
    }

    if let Some(catalog) = catalog {
        if let Some(finding) =
            llm_model_outside_provider_catalog(catalog, &llm_provider, &llm_model, locale)
        {
            findings.push(finding);
        }
        if let Some(finding) =
            fallback_model_outside_provider_catalog(catalog, &fallback_model, locale)
        {
            findings.push(finding);
        }
    }

    findings
}

/// Log each finding at WARN with its stable code. No-op when the map is clean.
pub fn log_runtime_settings_audit(settings: &RuntimeSettings) {
    let locale = settings.display_language();
    let catalog = crate::model_catalog::cached_model_catalog();
    for finding in audit_runtime_settings(settings, &locale, catalog.as_ref()) {
        tracing::warn!(
            code = finding.code,
            severity = finding.severity,
            "{}",
            finding.message
        );
    }
}

pub(crate) fn paid_provider_credentials_configured() -> bool {
    PAID_PROVIDER_ENV_KEYS.iter().any(|key| env_key_usable(key))
}

/// Whether Google data access is configured on this install, mirroring the
/// two ways the runtime learns about it:
/// - the encrypted credential store (`CREDENTIAL_DB_PATH`, the same file
///   `main.rs` opens) holds a complete `google_oauth` account — the rows
///   `GoogleAuthManager::from_credential_store` requires — checked by row
///   presence without decrypting anything;
/// - `GOOGLE_CLIENT_ID` + `GOOGLE_CLIENT_SECRET` are set (`main.rs` pre-stores
///   them into that account), or `GOOGLE_CALENDAR_TOKEN` is set (the sandbox
///   turns it into the `www.googleapis.com` auth header).
///
/// Console-login credentials (`GOOGLE_LOGIN_*`) are not data routing and do
/// not count.
pub(crate) fn google_routing_configured() -> bool {
    google_env_configured(|key| std::env::var(key).ok())
        || credential_db_has_google_oauth(&credential_db_path())
}

fn credential_db_path() -> PathBuf {
    std::env::var("CREDENTIAL_DB_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CREDENTIAL_DB_PATH))
}

/// Env half of [`google_routing_configured`] with the lookup injected.
pub(crate) fn google_env_configured(lookup: impl Fn(&str) -> Option<String>) -> bool {
    let usable = |key: &str| lookup(key).is_some_and(|value| !value.trim().is_empty());
    (usable("GOOGLE_CLIENT_ID") && usable("GOOGLE_CLIENT_SECRET"))
        || usable("GOOGLE_CALENDAR_TOKEN")
}

/// Credential-store half of [`google_routing_configured`]: true when the
/// store at `path` holds every row `GoogleAuthManager::from_credential_store`
/// needs for the primary account or any `google_oauth:<id>` account. Opened
/// read-only; a missing or unreadable store fails open to "unconfigured".
pub(crate) fn credential_db_has_google_oauth(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match crate::encrypted_sqlite::open_with_flags(path, flags) {
        Ok(conn) => conn,
        Err(error) => {
            tracing::debug!(
                path = %path.display(),
                %error,
                "[SettingsAudit] credential store not readable; treating Google as unconfigured"
            );
            return false;
        }
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT service, key FROM credentials WHERE service = ?1 OR service LIKE ?2")
    else {
        return false;
    };
    let pattern = format!("{CRED_SERVICE}:%");
    let Ok(rows) = stmt.query_map(rusqlite::params![CRED_SERVICE, pattern], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return false;
    };
    let mut keys_by_service: HashMap<String, HashSet<String>> = HashMap::new();
    for (service, key) in rows.flatten() {
        keys_by_service.entry(service).or_default().insert(key);
    }
    keys_by_service
        .values()
        .any(|keys| REQUIRED_CRED_KEYS.iter().all(|key| keys.contains(*key)))
}

fn env_key_usable(key: &str) -> bool {
    match std::env::var(key) {
        Ok(value) => {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed != "sk-placeholder"
        }
        Err(_) => false,
    }
}

fn setting_str(settings: &RuntimeSettings, key: &str) -> String {
    settings
        .get(key)
        .and_then(|value| value.as_str().map(str::trim).map(str::to_owned))
        .unwrap_or_default()
}

fn finding(code: &'static str, severity: &'static str, locale: &str) -> SettingsFinding {
    SettingsFinding {
        code,
        severity,
        message: t(&format!("settings.audit.{code}"), locale).to_string(),
    }
}

fn catalog_finding(
    code: &'static str,
    locale: &str,
    model: &str,
    provider: &str,
) -> SettingsFinding {
    let message = t(&format!("settings.audit.{code}"), locale)
        .replace("{model}", model)
        .replace("{provider}", provider);
    SettingsFinding {
        code,
        severity: SEVERITY_ATTENTION,
        message,
    }
}

/// Fail-open: silent when the selected provider's catalog is absent or
/// unavailable. Attention only when a real list is cached and lacks the id.
fn llm_model_outside_provider_catalog(
    catalog: &ModelCatalog,
    llm_provider: &str,
    llm_model: &str,
    locale: &str,
) -> Option<SettingsFinding> {
    if llm_model.is_empty() {
        return None;
    }
    let provider = catalog_provider_id(llm_provider);
    let entry = catalog.providers().iter().find(|p| p.id == provider)?;
    if entry.is_unavailable() {
        return None;
    }
    if entry.models.iter().any(|m| m.id == llm_model) {
        return None;
    }
    Some(catalog_finding(
        "llm_model_not_in_provider_catalog",
        locale,
        llm_model,
        provider,
    ))
}

/// Fail-open like `validate_fallback_model_setting`: silent when no catalog
/// can confidently reject the id (missing/unavailable connected providers).
fn fallback_model_outside_provider_catalog(
    catalog: &ModelCatalog,
    fallback_model: &str,
    locale: &str,
) -> Option<SettingsFinding> {
    if fallback_model.is_empty() {
        return None;
    }
    if catalog.provider_for_model(fallback_model).is_some() {
        return None;
    }
    let connected_unavailable = catalog
        .providers()
        .iter()
        .any(|p| p.connected && p.is_unavailable());
    if connected_unavailable {
        return None;
    }
    // Need at least one usable (non-unavailable) provider list to conclude
    // the id is outside the catalog; otherwise stay silent.
    let usable: Vec<&str> = catalog
        .providers()
        .iter()
        .filter(|p| !p.is_unavailable())
        .map(|p| p.id.as_str())
        .collect();
    if usable.is_empty() {
        return None;
    }
    Some(catalog_finding(
        "fallback_model_not_in_provider_catalog",
        locale,
        fallback_model,
        &usable.join("/"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings_with(pairs: &[(&str, serde_json::Value)]) -> RuntimeSettings {
        let settings = RuntimeSettings::new();
        for (key, value) in pairs {
            settings.set(key, value.clone()).unwrap();
        }
        settings
    }

    fn codes(findings: &[SettingsFinding]) -> Vec<&str> {
        findings.iter().map(|finding| finding.code).collect()
    }

    #[test]
    fn fallback_authorized_without_model_is_attention_in_owner_language() {
        let settings = settings_with(&[
            ("fallback_model_authorized", json!(true)),
            ("fallback_model", json!("")),
            ("external_executor_enabled", json!(true)),
        ]);
        let zh = audit_runtime_settings(&settings, "zh", None);
        assert!(
            codes(&zh).contains(&"fallback_authorized_without_model"),
            "codes={:?}",
            codes(&zh)
        );
        let finding = zh
            .iter()
            .find(|item| item.code == "fallback_authorized_without_model")
            .unwrap();
        assert_eq!(finding.severity, "attention");
        assert_eq!(
            finding.message,
            "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。"
        );

        let en = audit_runtime_settings(&settings, "en", None);
        let en_finding = en
            .iter()
            .find(|item| item.code == "fallback_authorized_without_model")
            .unwrap();
        assert_eq!(
            en_finding.message,
            "Backup model is authorized, but no backup model is selected: chat will not switch automatically when the primary fails."
        );
    }

    #[test]
    fn fallback_model_equals_primary() {
        let settings = settings_with(&[
            ("fallback_model_authorized", json!(true)),
            ("fallback_model", json!("same-model")),
            ("llm_model", json!("same-model")),
            ("external_executor_enabled", json!(true)),
        ]);
        let findings = audit_runtime_settings(&settings, "zh", None);
        assert!(codes(&findings).contains(&"fallback_model_equals_primary"));
        assert!(!codes(&findings).contains(&"fallback_authorized_without_model"));
    }

    #[test]
    fn provider_without_model_only_for_openrouter() {
        let settings = settings_with(&[
            ("llm_provider", json!("openrouter")),
            ("llm_model", json!("")),
            ("external_executor_enabled", json!(true)),
        ]);
        let findings = audit_runtime_settings(&settings, "en", None);
        assert!(codes(&findings).contains(&"provider_without_model"));

        let other = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("")),
            ("external_executor_enabled", json!(true)),
        ]);
        let other_findings = audit_runtime_settings(&other, "en", None);
        assert!(!codes(&other_findings).contains(&"provider_without_model"));
    }

    #[test]
    fn paid_fallback_without_provider_tracks_env_credentials() {
        let settings = settings_with(&[
            ("paid_api_fallback_enabled", json!(true)),
            ("external_executor_enabled", json!(true)),
        ]);
        let findings = audit_runtime_settings(&settings, "zh", None);
        let has = codes(&findings).contains(&"paid_fallback_without_provider");
        assert_eq!(has, !paid_provider_credentials_configured());
    }

    #[test]
    fn executor_off_is_silent_on_a_fresh_install_without_google() {
        let settings = RuntimeSettings::new();
        assert!(!settings.is_enabled("external_executor_enabled"));
        let fresh = audit_runtime_settings_with(&settings, "zh", None, false);
        assert!(
            !codes(&fresh).contains(&"external_executor_disabled_but_google_routing"),
            "fresh install without Google must not warn about Google routing: {:?}",
            codes(&fresh)
        );
        // Negative control: the same map with Google configured raises it,
        // so the assertion above fails if the predicate is ignored.
        let configured = audit_runtime_settings_with(&settings, "zh", None, true);
        let finding = configured
            .iter()
            .find(|item| item.code == "external_executor_disabled_but_google_routing")
            .expect("executor-off finding when Google is configured");
        assert_eq!(finding.severity, "attention");
        assert!(finding.message.contains("Google"));
        assert_eq!(
            finding.message,
            "外部执行层已关闭，Google 的数据现在读不到。"
        );
    }

    #[test]
    fn executor_on_is_silent_even_with_google_configured() {
        let settings = settings_with(&[("external_executor_enabled", json!(true))]);
        let findings = audit_runtime_settings_with(&settings, "en", None, true);
        assert!(!codes(&findings).contains(&"external_executor_disabled_but_google_routing"));
    }

    #[test]
    fn google_env_predicate_needs_the_client_pair_or_a_calendar_token() {
        fn lookup(
            vars: &'static [(&'static str, &'static str)],
        ) -> impl Fn(&str) -> Option<String> {
            move |key| {
                vars.iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.to_string())
            }
        }
        assert!(!google_env_configured(lookup(&[])));
        assert!(!google_env_configured(lookup(&[(
            "GOOGLE_CLIENT_ID",
            "id.apps.googleusercontent.com"
        )])));
        assert!(!google_env_configured(lookup(&[
            ("GOOGLE_CLIENT_ID", "   "),
            ("GOOGLE_CLIENT_SECRET", "s"),
        ])));
        assert!(google_env_configured(lookup(&[
            ("GOOGLE_CLIENT_ID", "id.apps.googleusercontent.com"),
            ("GOOGLE_CLIENT_SECRET", "s"),
        ])));
        assert!(google_env_configured(lookup(&[(
            "GOOGLE_CALENDAR_TOKEN",
            "ya29.token"
        )])));
        // Console login is not data routing.
        assert!(!google_env_configured(lookup(&[
            ("GOOGLE_LOGIN_CLIENT_ID", "id"),
            ("GOOGLE_LOGIN_CLIENT_SECRET", "s"),
        ])));
    }

    #[test]
    fn credential_db_predicate_matches_the_google_auth_loader() {
        use crate::credential_store::{CredentialStore, derive_key_from_ed25519};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aegis_runtime_credentials.db");
        assert!(
            !credential_db_has_google_oauth(&path),
            "missing store = unconfigured"
        );

        let key = derive_key_from_ed25519(&[7u8; 32]);
        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&path).unwrap(), key).unwrap();
        assert!(
            !credential_db_has_google_oauth(&path),
            "empty store = unconfigured"
        );

        store.store("google_oauth", "client_id", "id").unwrap();
        store.store("google_oauth", "client_secret", "s").unwrap();
        assert!(
            !credential_db_has_google_oauth(&path),
            "pending (unauthorized) client credentials are not a working Google setup"
        );

        store
            .store("google_oauth", "refresh_token", "1//r")
            .unwrap();
        assert!(
            credential_db_has_google_oauth(&path),
            "complete primary account = configured"
        );

        // A complete secondary account counts on its own.
        let second = dir.path().join("second.db");
        let store2 =
            CredentialStore::open(crate::encrypted_sqlite::open(&second).unwrap(), key).unwrap();
        store2
            .store("google_oauth:work", "client_id", "id")
            .unwrap();
        store2
            .store("google_oauth:work", "client_secret", "s")
            .unwrap();
        assert!(!credential_db_has_google_oauth(&second));
        store2
            .store("google_oauth:work", "refresh_token", "1//r")
            .unwrap();
        assert!(credential_db_has_google_oauth(&second));

        // Unrelated services never count.
        let other = dir.path().join("other.db");
        let store3 =
            CredentialStore::open(crate::encrypted_sqlite::open(&other).unwrap(), key).unwrap();
        store3.store("github", "client_id", "id").unwrap();
        store3.store("github", "client_secret", "s").unwrap();
        store3.store("github", "refresh_token", "r").unwrap();
        assert!(!credential_db_has_google_oauth(&other));
    }

    #[test]
    fn clean_map_has_no_attention_findings() {
        let settings = settings_with(&[
            ("fallback_model_authorized", json!(true)),
            ("fallback_model", json!("backup-model")),
            ("llm_model", json!("primary-model")),
            ("llm_provider", json!("codex_oauth")),
            ("paid_api_fallback_enabled", json!(false)),
            ("external_executor_enabled", json!(true)),
        ]);
        let findings = audit_runtime_settings(&settings, "zh", None);
        let attention: Vec<_> = findings
            .iter()
            .filter(|item| item.severity == "attention")
            .map(|item| item.code)
            .collect();
        let unexpected: Vec<_> = attention
            .into_iter()
            .filter(|code| *code != "paid_fallback_without_provider")
            .collect();
        assert!(
            unexpected.is_empty(),
            "clean map should not emit attention findings {unexpected:?} (paid_fallback depends on env)"
        );
    }

    #[test]
    fn healthy_production_shaped_config_has_zero_attention_when_paid_credentials_present() {
        // Mirrors the owner production shape from the task brief. The paid
        // credential check follows ambient env (same as the router); when a
        // real paid key is present this config must be silent.
        let settings = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("codex-primary")),
            ("fallback_model_authorized", json!(true)),
            ("fallback_model", json!("grok-4.6")),
            ("paid_api_fallback_enabled", json!(true)),
            ("external_executor_enabled", json!(true)),
        ]);
        let findings = audit_runtime_settings(&settings, "zh", None);
        let attention: Vec<_> = findings
            .iter()
            .filter(|item| item.severity == "attention")
            .map(|item| item.code)
            .collect();
        if paid_provider_credentials_configured() {
            assert!(
                attention.is_empty(),
                "healthy production shape must be silent when paid credentials exist; got {attention:?}"
            );
        } else {
            assert_eq!(
                attention,
                vec!["paid_fallback_without_provider"],
                "without paid credentials only paid_fallback_without_provider should fire"
            );
        }

        let en = audit_runtime_settings(&settings, "en", None);
        for finding in en.iter().chain(findings.iter()) {
            assert!(
                !finding.message.contains("run_external")
                    && !finding.message.contains("browser_fetch")
                    && !finding.message.contains("回执")
                    && !finding.message.contains("事项")
                    && !finding.message.contains("主人")
                    && !finding.message.contains("幕僚长"),
                "owner message must stay plain: {}",
                finding.message
            );
        }
    }

    #[test]
    fn llm_model_not_in_provider_catalog_when_cached_list_lacks_id() {
        let settings = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("stale-model")),
            ("external_executor_enabled", json!(true)),
        ]);
        let catalog = crate::model_catalog::ModelCatalog::from_providers(vec![
            crate::model_catalog::ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![crate::model_catalog::CatalogModel {
                    id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                }],
                source: "codex_cli_bundled_catalog".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
        ]);
        let zh = audit_runtime_settings(&settings, "zh", Some(&catalog));
        let finding = zh
            .iter()
            .find(|item| item.code == "llm_model_not_in_provider_catalog")
            .expect("stale llm_model must raise attention");
        assert_eq!(finding.severity, "attention");
        assert_eq!(
            finding.message,
            "模型设置里的 stale-model 不在 codex_oauth 的模型列表里，调用可能失败；建议在设置页重选。"
        );

        let en = audit_runtime_settings(&settings, "en", Some(&catalog));
        let en_finding = en
            .iter()
            .find(|item| item.code == "llm_model_not_in_provider_catalog")
            .unwrap();
        assert!(en_finding.message.contains("stale-model"));
        assert!(en_finding.message.contains("codex_oauth"));
    }

    #[test]
    fn fallback_model_not_in_provider_catalog_when_cached_lists_lack_id() {
        let settings = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("gpt-5.5")),
            ("fallback_model", json!("missing-backup")),
            ("external_executor_enabled", json!(true)),
        ]);
        let catalog = crate::model_catalog::ModelCatalog::from_providers(vec![
            crate::model_catalog::ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![crate::model_catalog::CatalogModel {
                    id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                }],
                source: "codex_cli_bundled_catalog".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            crate::model_catalog::ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![crate::model_catalog::CatalogModel {
                    id: "grok-4.6".into(),
                    display_name: "Grok 4.6".into(),
                }],
                source: "xai_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
        ]);
        let zh = audit_runtime_settings(&settings, "zh", Some(&catalog));
        let finding = zh
            .iter()
            .find(|item| item.code == "fallback_model_not_in_provider_catalog")
            .expect("unknown fallback must raise attention");
        assert_eq!(finding.severity, "attention");
        assert!(finding.message.contains("missing-backup"));
        assert!(finding.message.contains("codex_oauth/xai_subscription"));
        assert!(finding.message.contains("建议在设置页重选"));
    }

    #[test]
    fn catalog_model_rules_are_silent_when_uncached_or_unavailable() {
        let settings = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("stale-model")),
            ("fallback_model", json!("missing-backup")),
            ("external_executor_enabled", json!(true)),
        ]);
        let none = audit_runtime_settings(&settings, "zh", None);
        assert!(!codes(&none).contains(&"llm_model_not_in_provider_catalog"));
        assert!(!codes(&none).contains(&"fallback_model_not_in_provider_catalog"));

        let unavailable = crate::model_catalog::ModelCatalog::from_providers(vec![
            crate::model_catalog::ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![],
                source: "unavailable".into(),
                fetched_at_unix: None,
                default_model: String::new(),
            },
            crate::model_catalog::ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![],
                source: "unavailable".into(),
                fetched_at_unix: None,
                default_model: String::new(),
            },
        ]);
        let soft = audit_runtime_settings(&settings, "zh", Some(&unavailable));
        assert!(!codes(&soft).contains(&"llm_model_not_in_provider_catalog"));
        assert!(!codes(&soft).contains(&"fallback_model_not_in_provider_catalog"));
    }

    #[test]
    fn known_catalog_ids_do_not_raise_model_outside_list() {
        let settings = settings_with(&[
            ("llm_provider", json!("codex_oauth")),
            ("llm_model", json!("gpt-5.5")),
            ("fallback_model", json!("grok-4.6")),
            ("external_executor_enabled", json!(true)),
        ]);
        let catalog = crate::model_catalog::ModelCatalog::from_providers(vec![
            crate::model_catalog::ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![crate::model_catalog::CatalogModel {
                    id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                }],
                source: "codex_cli_bundled_catalog".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            crate::model_catalog::ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![crate::model_catalog::CatalogModel {
                    id: "grok-4.6".into(),
                    display_name: "Grok 4.6".into(),
                }],
                source: "xai_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
        ]);
        let findings = audit_runtime_settings(&settings, "zh", Some(&catalog));
        assert!(!codes(&findings).contains(&"llm_model_not_in_provider_catalog"));
        assert!(!codes(&findings).contains(&"fallback_model_not_in_provider_catalog"));
    }
}
