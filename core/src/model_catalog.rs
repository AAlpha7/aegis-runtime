//! Provider-sourced model catalogs for console settings validation.
//!
//! Model ids come from each provider's live list (Codex app-server / CLI,
//! xAI `GET /models`, OpenRouter `GET /api/v1/models`). Disconnected or failed
//! providers yield an empty list with `source = "unavailable"` — never an
//! error. Fetches are bounded by a 10 s timeout; the in-memory cache is read
//! without holding the mutex across the network call.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::admin::AdminState;
use crate::i18n::{self, LOCALE_EN, LOCALE_ZH};
use crate::llm::xai_subscription::{self, XaiSubscriptionAuthConfig, XaiSubscriptionAuthManager};
use crate::settings::RuntimeSettings;

const CACHE_TTL: Duration = Duration::from_secs(3600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const SKILL_STATE_SKILL: &str = "model_catalog";
const SKILL_STATE_OPENROUTER_KEY: &str = "openrouter";
const RECOMMENDED_API_MODEL: &str = "deepseek/deepseek-v4.1-flash";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalog {
    pub id: String,
    pub connected: bool,
    pub models: Vec<CatalogModel>,
    pub source: String,
    pub fetched_at_unix: Option<i64>,
    /// Provider default model id from runtime settings (may be empty).
    /// Filled when assembling a catalog response; not meaningful inside the
    /// raw per-provider fetch caches.
    #[serde(default)]
    pub default_model: String,
}

impl ProviderCatalog {
    fn unavailable(id: &str, connected: bool) -> Self {
        Self {
            id: id.to_string(),
            connected,
            models: Vec::new(),
            source: "unavailable".to_string(),
            fetched_at_unix: None,
            default_model: String::new(),
        }
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        self.source == "unavailable"
    }
}

#[derive(Debug, Clone)]
pub struct ModelCatalog {
    providers: Vec<ProviderCatalog>,
}

impl ModelCatalog {
    /// Assemble a catalog from already-fetched provider entries (tests + audit).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_providers(providers: Vec<ProviderCatalog>) -> Self {
        Self { providers }
    }

    pub fn providers(&self) -> &[ProviderCatalog] {
        &self.providers
    }

    /// First provider whose list contains `model` (order: codex → xAI → OpenRouter).
    pub fn provider_for_model(&self, model: &str) -> Option<&str> {
        let needle = model.trim();
        if needle.is_empty() {
            return None;
        }
        for provider in &self.providers {
            if provider.models.iter().any(|m| m.id == needle) {
                return Some(provider.id.as_str());
            }
        }
        None
    }

    pub fn contains(&self, provider: &str, model: &str) -> bool {
        let needle = model.trim();
        self.providers
            .iter()
            .find(|p| p.id == provider)
            .is_some_and(|p| p.models.iter().any(|m| m.id == needle))
    }

    /// A recommendation, never a saved selection or fallback authorization.
    /// Do not advertise a model absent from the connected provider's catalog.
    pub fn recommended_api_model(&self) -> Option<&str> {
        self.provider("openrouter")
            .filter(|provider| provider.connected && !provider.is_unavailable())
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.id == RECOMMENDED_API_MODEL)
            })
            .map(|model| model.id.as_str())
    }

    fn provider(&self, id: &str) -> Option<&ProviderCatalog> {
        self.providers.iter().find(|p| p.id == id)
    }
}

struct ProviderCache {
    slot: Mutex<Option<(Instant, ProviderCatalog)>>,
}

impl ProviderCache {
    const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    fn get(&self) -> Option<ProviderCatalog> {
        let guard = self.slot.lock();
        let (stored_at, catalog) = guard.as_ref()?;
        if stored_at.elapsed() < CACHE_TTL {
            let mut cached = catalog.clone();
            // Keep "unavailable" so validation can still soften; mark live
            // successes as served from cache.
            if cached.source != "unavailable" {
                cached.source = "cache".to_string();
            }
            Some(cached)
        } else {
            None
        }
    }

    fn put(&self, catalog: ProviderCatalog) {
        *self.slot.lock() = Some((Instant::now(), catalog));
    }

    #[cfg(test)]
    fn clear(&self) {
        *self.slot.lock() = None;
    }

    #[cfg(test)]
    fn put_with_age(&self, catalog: ProviderCatalog, age: Duration) {
        let stored_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
        *self.slot.lock() = Some((stored_at, catalog));
    }
}

static CODEX_CACHE: ProviderCache = ProviderCache::new();
static XAI_CACHE: ProviderCache = ProviderCache::new();
static OPENROUTER_CACHE: ProviderCache = ProviderCache::new();

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn open_credential_store(state: &AdminState) -> Option<crate::credential_store::CredentialStore> {
    let db_path = state.credential_db_path.as_deref()?;
    let aes_key = state.credential_aes_key?;
    let conn = crate::encrypted_sqlite::open(db_path).ok()?;
    crate::credential_store::CredentialStore::open(conn, aes_key).ok()
}

fn bilingual_unknown_model() -> String {
    let zh = i18n::t("console.model.unknown", LOCALE_ZH);
    let en = i18n::t("console.model.unknown", LOCALE_EN);
    format!("{zh} / {en}")
}

fn bilingual_choose_model() -> String {
    let zh = i18n::t("console.model.choose", LOCALE_ZH);
    let en = i18n::t("console.model.choose", LOCALE_EN);
    format!("{zh} / {en}")
}

/// Owner-safe error when OpenRouter is selected with an empty model id.
pub fn openrouter_empty_model_error() -> String {
    bilingual_choose_model()
}

/// Strip the `openai-codex/` prefix the way `Orchestrator::codex_model` does.
fn normalize_codex_model_id(model: &str) -> String {
    let trimmed = model.trim();
    if let Some(rest) = trimmed.strip_prefix("openai-codex/") {
        rest.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Provider default model for an empty `llm_model` (matches `active_model`).
pub fn provider_default_model(settings: &RuntimeSettings, provider: &str) -> String {
    match provider {
        "codex_oauth" => settings
            .get("codex_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .map(|value| normalize_codex_model_id(&value))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default(),
        "xai_subscription" | "grok_subscription" => settings
            .get("xai_subscription_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Resolve the effective LLM model id: raw `llm_model` when non-empty,
/// otherwise the selected provider's default. Shared by
/// `Orchestrator::active_model` and the model-catalog endpoint.
pub fn resolve_effective_llm_model(
    settings: &RuntimeSettings,
    llm_model: &str,
    llm_provider: &str,
) -> String {
    let trimmed = llm_model.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    provider_default_model(settings, llm_provider)
}

fn apply_provider_default_models(catalog: &mut ModelCatalog, settings: &RuntimeSettings) {
    for provider in &mut catalog.providers {
        provider.default_model = provider_default_model(settings, &provider.id);
    }
}

/// Assemble whatever provider catalogs are currently in the process-wide
/// caches, without triggering a network fetch. Returns `None` when every
/// cache slot is empty/expired.
pub fn cached_model_catalog() -> Option<ModelCatalog> {
    let providers: Vec<ProviderCatalog> =
        [CODEX_CACHE.get(), XAI_CACHE.get(), OPENROUTER_CACHE.get()]
            .into_iter()
            .flatten()
            .collect();
    if providers.is_empty() {
        None
    } else {
        Some(ModelCatalog { providers })
    }
}

/// Canonical catalog provider id for a runtime `llm_provider` setting.
pub fn catalog_provider_id(llm_provider: &str) -> &str {
    match llm_provider {
        "grok_subscription" => "xai_subscription",
        other => other,
    }
}

/// Validate `llm_model` against the serving provider's catalog.
///
/// Empty model is allowed (provider default). When the serving provider's
/// catalog is `unavailable`, accept with a warning so a fetch failure cannot
/// lock the owner out.
pub fn validate_llm_model_setting(
    catalog: &ModelCatalog,
    llm_provider: &str,
    model: &str,
) -> Result<(), String> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let provider = match llm_provider {
        "codex_oauth" | "xai_subscription" | "grok_subscription" | "openrouter" => {
            if llm_provider == "grok_subscription" {
                "xai_subscription"
            } else {
                llm_provider
            }
        }
        _ => llm_provider,
    };
    match catalog.provider(provider) {
        Some(entry) if entry.is_unavailable() => {
            tracing::warn!(
                provider,
                model = %trimmed,
                "accepting llm_model while provider catalog is unavailable"
            );
            Ok(())
        }
        Some(entry) if entry.models.iter().any(|m| m.id == trimmed) => Ok(()),
        Some(_) => Err(bilingual_unknown_model()),
        None => {
            // Provider not in catalog response — treat as unavailable.
            tracing::warn!(
                provider,
                model = %trimmed,
                "accepting llm_model; serving provider missing from catalog"
            );
            Ok(())
        }
    }
}

/// Validate `fallback_model`. Empty is allowed (no backup). When every
/// *connected* provider that could own the id is unavailable, accept with a
/// warning. Otherwise the id must appear in some loaded catalog.
pub fn validate_fallback_model_setting(catalog: &ModelCatalog, model: &str) -> Result<(), String> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if catalog.provider_for_model(trimmed).is_some() {
        return Ok(());
    }
    let connected_unavailable = catalog
        .providers
        .iter()
        .any(|p| p.connected && p.is_unavailable());
    if connected_unavailable {
        tracing::warn!(
            model = %trimmed,
            "accepting fallback_model while a connected provider catalog is unavailable"
        );
        return Ok(());
    }
    Err(bilingual_unknown_model())
}

async fn load_codex_catalog(state: &AdminState) -> ProviderCatalog {
    if let Some(cached) = CODEX_CACHE.get() {
        return cached;
    }

    let login = crate::admin::codex_cli_login_status_for_catalog(state).await;
    let connected = login.logged_in;
    if !connected {
        let unavailable = ProviderCatalog::unavailable("codex_oauth", false);
        CODEX_CACHE.put(unavailable.clone());
        return unavailable;
    }

    let (options, source) = crate::admin::codex_cli_model_options_for_catalog(state).await;
    let models = options
        .into_iter()
        .filter_map(|opt| {
            let id = opt
                .get("slug")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            let display_name = opt
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id.as_str())
                .to_string();
            Some(CatalogModel { id, display_name })
        })
        .collect::<Vec<_>>();

    let catalog = ProviderCatalog {
        id: "codex_oauth".to_string(),
        connected: true,
        models,
        source: source.to_string(),
        fetched_at_unix: Some(now_unix()),
        default_model: String::new(),
    };
    CODEX_CACHE.put(catalog.clone());
    catalog
}

async fn fetch_xai_models(api_base: &str, access_token: &str) -> Result<Vec<CatalogModel>, String> {
    let url = format!("{}/models", api_base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("xAI models HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut models = Vec::new();
    if let Some(data) = body.get("data").and_then(|v| v.as_array()) {
        for entry in data {
            let id = match entry.get("id").and_then(|v| v.as_str()).map(str::trim) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => continue,
            };
            let display_name = entry
                .get("name")
                .or_else(|| entry.get("display_name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id.as_str())
                .to_string();
            models.push(CatalogModel { id, display_name });
        }
    }
    models.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

async fn load_xai_catalog(state: &AdminState) -> ProviderCatalog {
    if let Some(cached) = XAI_CACHE.get() {
        return cached;
    }

    // Load credential then drop the SQLite store before any .await so this
    // future stays Send for axum handlers.
    let credential = {
        let Some(store) = open_credential_store(state) else {
            let unavailable = ProviderCatalog::unavailable("xai_subscription", false);
            XAI_CACHE.put(unavailable.clone());
            return unavailable;
        };
        match XaiSubscriptionAuthManager::load_credential(&store) {
            Ok(Some(cred)) => cred,
            Ok(None) => {
                let unavailable = ProviderCatalog::unavailable("xai_subscription", false);
                XAI_CACHE.put(unavailable.clone());
                return unavailable;
            }
            Err(err) => {
                tracing::warn!(error = %err, "xAI credential load failed for model catalog");
                let unavailable = ProviderCatalog::unavailable("xai_subscription", true);
                XAI_CACHE.put(unavailable.clone());
                return unavailable;
            }
        }
    };

    let api_base = state
        .settings
        .get("xai_subscription_api_base")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://api.x.ai/v1".to_string());

    // Never log the token. Bound refresh+fetch at 10s.
    let manager = XaiSubscriptionAuthManager::new(XaiSubscriptionAuthConfig::default());
    let needs_refresh = credential.needs_refresh(xai_subscription::now_ms());
    let timed = async {
        let (access_token, refreshed) = if needs_refresh {
            match manager.refresh_credential(&credential).await {
                Ok(refreshed) => (refreshed.access_token.clone(), Some(refreshed)),
                Err(err) => {
                    tracing::warn!(error = %err, "xAI credential refresh failed for model catalog");
                    (credential.access_token.clone(), None)
                }
            }
        } else {
            (credential.access_token.clone(), None)
        };
        let models = fetch_xai_models(&api_base, &access_token).await?;
        Ok::<_, String>((models, refreshed))
    };

    match tokio::time::timeout(FETCH_TIMEOUT, timed).await {
        Ok(Ok((models, refreshed))) => {
            if let Some(refreshed) = refreshed
                && let Some(store) = open_credential_store(state)
            {
                let _ = XaiSubscriptionAuthManager::store_credential(&store, &refreshed);
            }
            let catalog = ProviderCatalog {
                id: "xai_subscription".to_string(),
                connected: true,
                models,
                source: "xai_models_api".to_string(),
                fetched_at_unix: Some(now_unix()),
                default_model: String::new(),
            };
            XAI_CACHE.put(catalog.clone());
            catalog
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "xAI model list fetch failed");
            let unavailable = ProviderCatalog::unavailable("xai_subscription", true);
            XAI_CACHE.put(unavailable.clone());
            unavailable
        }
        Err(_) => {
            tracing::warn!("xAI model catalog refresh/fetch timed out");
            let unavailable = ProviderCatalog::unavailable("xai_subscription", true);
            XAI_CACHE.put(unavailable.clone());
            unavailable
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Option<Vec<OpenRouterModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelEntry {
    id: Option<String>,
    name: Option<String>,
}

async fn fetch_openrouter_models() -> Result<Vec<CatalogModel>, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("OpenRouter models HTTP {}", resp.status()));
    }
    let body: OpenRouterModelsResponse = resp.json().await.map_err(|e| e.to_string())?;
    let mut models = body
        .data
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let id = entry.id?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            let display_name = entry
                .name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            Some(CatalogModel { id, display_name })
        })
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

async fn load_persisted_openrouter(state: &AdminState) -> Option<ProviderCatalog> {
    let raw = state
        .memory
        .load_skill_state(SKILL_STATE_SKILL, SKILL_STATE_OPENROUTER_KEY)
        .await
        .ok()
        .flatten()?;
    let mut catalog: ProviderCatalog = serde_json::from_str(&raw).ok()?;
    catalog.source = "cache".to_string();
    Some(catalog)
}

async fn persist_openrouter(state: &AdminState, catalog: &ProviderCatalog) {
    if let Ok(raw) = serde_json::to_string(catalog)
        && let Err(err) = state
            .memory
            .store_skill_state(SKILL_STATE_SKILL, SKILL_STATE_OPENROUTER_KEY, &raw)
            .await
    {
        tracing::warn!(error = %err, "failed to persist OpenRouter model catalog");
    }
}

async fn load_openrouter_catalog(state: &AdminState) -> ProviderCatalog {
    if let Some(cached) = OPENROUTER_CACHE.get() {
        return cached;
    }

    // OpenRouter models API is public; treat as connected when we can serve a list.
    match fetch_openrouter_models().await {
        Ok(models) => {
            let catalog = ProviderCatalog {
                id: "openrouter".to_string(),
                connected: true,
                models,
                source: "openrouter_models_api".to_string(),
                fetched_at_unix: Some(now_unix()),
                default_model: String::new(),
            };
            OPENROUTER_CACHE.put(catalog.clone());
            persist_openrouter(state, &catalog).await;
            catalog
        }
        Err(err) => {
            tracing::warn!(error = %err, "OpenRouter model list fetch failed");
            if let Some(persisted) = load_persisted_openrouter(state).await {
                OPENROUTER_CACHE.put(persisted.clone());
                return persisted;
            }
            let unavailable = ProviderCatalog::unavailable("openrouter", true);
            OPENROUTER_CACHE.put(unavailable.clone());
            unavailable
        }
    }
}

/// Load the three provider catalogs (Codex, xAI, OpenRouter) with a 1 h
/// in-memory cache. Never returns an error for a single provider failure.
pub async fn load_model_catalog(state: &AdminState) -> ModelCatalog {
    let (codex, xai, openrouter) = tokio::join!(
        load_codex_catalog(state),
        load_xai_catalog(state),
        load_openrouter_catalog(state),
    );
    let mut catalog = ModelCatalog {
        providers: vec![codex, xai, openrouter],
    };
    apply_provider_default_models(&mut catalog, &state.settings);
    catalog
}

#[cfg(test)]
static TEST_CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Serialize tests that mutate the process-wide catalog caches.
#[cfg(test)]
/// Async-aware so a test may hold it across `.await` points.
pub(crate) async fn lock_model_catalog_caches_for_tests() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_CACHE_LOCK.lock().await
}

#[cfg(test)]
pub(crate) fn clear_model_catalog_caches_for_tests() {
    CODEX_CACHE.clear();
    XAI_CACHE.clear();
    OPENROUTER_CACHE.clear();
}

#[cfg(test)]
pub(crate) fn seed_provider_cache_for_tests(catalog: ProviderCatalog) {
    match catalog.id.as_str() {
        "codex_oauth" => CODEX_CACHE.put(catalog),
        "xai_subscription" => XAI_CACHE.put(catalog),
        "openrouter" => OPENROUTER_CACHE.put(catalog),
        _ => {}
    }
}

#[cfg(test)]
pub(crate) fn seed_provider_cache_aged_for_tests(catalog: ProviderCatalog, age: Duration) {
    match catalog.id.as_str() {
        "codex_oauth" => CODEX_CACHE.put_with_age(catalog, age),
        "xai_subscription" => XAI_CACHE.put_with_age(catalog, age),
        "openrouter" => OPENROUTER_CACHE.put_with_age(catalog, age),
        _ => {}
    }
}

#[cfg(test)]
fn test_catalog(providers: Vec<ProviderCatalog>) -> ModelCatalog {
    ModelCatalog::from_providers(providers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_requires_connected_available_catalog_and_exact_model() {
        let mut provider = ProviderCatalog::unavailable("openrouter", true);
        provider.models.push(CatalogModel {
            id: RECOMMENDED_API_MODEL.into(),
            display_name: "DeepSeek V4.1 Flash".into(),
        });
        assert_eq!(
            test_catalog(vec![provider.clone()]).recommended_api_model(),
            None
        );
        provider.source = "openrouter_models_api".into();
        assert_eq!(
            test_catalog(vec![provider.clone()]).recommended_api_model(),
            Some(RECOMMENDED_API_MODEL)
        );
        provider.connected = false;
        assert_eq!(
            test_catalog(vec![provider.clone()]).recommended_api_model(),
            None
        );
        provider.connected = true;
        provider.models[0].id = "deepseek/deepseek-v4-pro".into();
        assert_eq!(test_catalog(vec![provider]).recommended_api_model(), None);
        assert_eq!(test_catalog(vec![]).recommended_api_model(), None);
    }

    #[test]
    fn provider_for_model_returns_first_matching_provider() {
        let catalog = test_catalog(vec![
            ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                }],
                source: "codex_cli_bundled_catalog".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![
                    CatalogModel {
                        id: "grok-4.6".into(),
                        display_name: "Grok 4.6".into(),
                    },
                    CatalogModel {
                        id: "gpt-5.5".into(),
                        display_name: "GPT-5.5 (xAI)".into(),
                    },
                ],
                source: "xai_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            ProviderCatalog {
                id: "openrouter".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "deepseek/deepseek-v4-pro".into(),
                    display_name: "DeepSeek V4 Pro".into(),
                }],
                source: "openrouter_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
        ]);

        assert_eq!(catalog.provider_for_model("gpt-5.5"), Some("codex_oauth"));
        assert_eq!(
            catalog.provider_for_model("grok-4.6"),
            Some("xai_subscription")
        );
        assert_eq!(
            catalog.provider_for_model("deepseek/deepseek-v4-pro"),
            Some("openrouter")
        );
        assert_eq!(catalog.provider_for_model("missing-model"), None);
        assert!(catalog.contains("codex_oauth", "gpt-5.5"));
        assert!(!catalog.contains("openrouter", "gpt-5.5"));
    }

    #[test]
    fn validation_accepts_known_ids_and_rejects_unknown() {
        let catalog = test_catalog(vec![
            ProviderCatalog {
                id: "codex_oauth".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                }],
                source: "codex_cli_bundled_catalog".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "grok-4.6".into(),
                    display_name: "Grok 4.6".into(),
                }],
                source: "xai_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            ProviderCatalog {
                id: "openrouter".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "deepseek/deepseek-v4-pro".into(),
                    display_name: "DeepSeek V4 Pro".into(),
                }],
                source: "openrouter_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
        ]);

        assert!(validate_llm_model_setting(&catalog, "codex_oauth", "gpt-5.5").is_ok());
        assert!(validate_llm_model_setting(&catalog, "codex_oauth", "").is_ok());
        assert!(validate_llm_model_setting(&catalog, "codex_oauth", "nope").is_err());
        assert!(validate_fallback_model_setting(&catalog, "grok-4.6").is_ok());
        assert!(validate_fallback_model_setting(&catalog, "").is_ok());
        assert!(validate_fallback_model_setting(&catalog, "nope").is_err());
    }

    #[test]
    fn validation_accepts_when_serving_provider_catalog_is_unavailable() {
        let catalog = test_catalog(vec![
            ProviderCatalog::unavailable("codex_oauth", true),
            ProviderCatalog {
                id: "xai_subscription".into(),
                connected: true,
                models: vec![CatalogModel {
                    id: "grok-4.6".into(),
                    display_name: "Grok 4.6".into(),
                }],
                source: "xai_models_api".into(),
                fetched_at_unix: Some(1),
                default_model: String::new(),
            },
            ProviderCatalog::unavailable("openrouter", false),
        ]);

        assert!(
            validate_llm_model_setting(&catalog, "codex_oauth", "legacy-gemini-id").is_ok(),
            "unavailable serving catalog must accept to avoid locking the owner out"
        );
        assert!(
            validate_fallback_model_setting(&catalog, "unknown-fallback").is_ok(),
            "connected unavailable catalog must soften fallback rejection"
        );
    }

    #[tokio::test]
    async fn in_memory_cache_is_used_until_ttl_expires() {
        let _guard = lock_model_catalog_caches_for_tests().await;
        clear_model_catalog_caches_for_tests();
        let seeded = ProviderCatalog {
            id: "openrouter".into(),
            connected: true,
            models: vec![CatalogModel {
                id: "deepseek/deepseek-v4-pro".into(),
                display_name: "DeepSeek V4 Pro".into(),
            }],
            source: "openrouter_models_api".into(),
            fetched_at_unix: Some(1),
            default_model: String::new(),
        };
        seed_provider_cache_for_tests(seeded);

        let hit = OPENROUTER_CACHE
            .get()
            .expect("warm cache should return a hit");
        assert_eq!(hit.source, "cache");
        assert_eq!(hit.models.len(), 1);
        assert_eq!(hit.models[0].id, "deepseek/deepseek-v4-pro");

        // Age past TTL — cache must miss.
        clear_model_catalog_caches_for_tests();
        let aged = ProviderCatalog {
            id: "openrouter".into(),
            connected: true,
            models: vec![CatalogModel {
                id: "deepseek/deepseek-v4-pro".into(),
                display_name: "DeepSeek V4 Pro".into(),
            }],
            source: "openrouter_models_api".into(),
            fetched_at_unix: Some(1),
            default_model: String::new(),
        };
        seed_provider_cache_aged_for_tests(aged, CACHE_TTL + Duration::from_secs(1));
        assert!(
            OPENROUTER_CACHE.get().is_none(),
            "expired cache entry must not be reused"
        );
        clear_model_catalog_caches_for_tests();
    }

    #[test]
    fn resolve_effective_llm_model_matches_provider_defaults() {
        let settings = RuntimeSettings::new();
        settings
            .set("codex_model", serde_json::json!("gpt-5.5"))
            .unwrap();
        settings
            .set("xai_subscription_model", serde_json::json!("grok-4.3"))
            .unwrap();

        assert_eq!(
            resolve_effective_llm_model(&settings, "custom/slug", "codex_oauth"),
            "custom/slug"
        );
        assert_eq!(
            resolve_effective_llm_model(&settings, "", "codex_oauth"),
            "gpt-5.5"
        );
        assert_eq!(
            resolve_effective_llm_model(&settings, "  ", "xai_subscription"),
            "grok-4.3"
        );
        assert_eq!(resolve_effective_llm_model(&settings, "", "openrouter"), "");
        assert_eq!(provider_default_model(&settings, "openrouter"), "");
    }

    #[tokio::test]
    async fn cached_model_catalog_assembles_warm_caches_without_fetch() {
        let _guard = lock_model_catalog_caches_for_tests().await;
        clear_model_catalog_caches_for_tests();
        assert!(cached_model_catalog().is_none());

        seed_provider_cache_for_tests(ProviderCatalog {
            id: "xai_subscription".into(),
            connected: true,
            models: vec![CatalogModel {
                id: "grok-4.6".into(),
                display_name: "Grok 4.6".into(),
            }],
            source: "xai_models_api".into(),
            fetched_at_unix: Some(1),
            default_model: String::new(),
        });
        let catalog = cached_model_catalog().expect("warm cache should assemble");
        assert_eq!(
            catalog.provider_for_model("grok-4.6"),
            Some("xai_subscription")
        );
        clear_model_catalog_caches_for_tests();
    }
}
