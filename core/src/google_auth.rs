//! Google OAuth2 Token Manager for Aegis Runtime.
//!
//! Provides a unified OAuth2 authentication layer for Google Workspace APIs
//! (Calendar, Drive, Docs, Sheets). Supports two authorization flows:
//!
//! 1. **Telegram-driven (Web application)**: Public callback endpoint receives
//!    the authorization code automatically. Requires `AEGIS_PUBLIC_URL`.
//! 2. **Script/manual (Desktop application)**: User copies the authorization
//!    code and pastes it back (via `scripts/google-auth.sh` or Telegram).
//!
//! Both flows use `access_type=offline` to obtain a long-lived refresh token,
//! then transparently manage access token lifecycle (auto-refresh, caching,
//! thread-safe access).
//!
//! Tokens are stored encrypted in the credential store (AES-256-GCM, bound to
//! the agent's Ed25519 identity).
//!
//! # Credential Store Keys
//!
//! All stored under service = `"google_oauth"`:
//! - `client_id` — OAuth2 Client ID
//! - `client_secret` — OAuth2 Client Secret
//! - `refresh_token` — Long-lived refresh token
//! - `scopes` — Comma-separated granted scopes

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
#[cfg(not(test))]
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::credential_store::CredentialStore;

type HmacSha256 = Hmac<Sha256>;

const SCOPE_UPGRADE_CONTEXT_KEY: &str = "scope_upgrade_context_json";
#[cfg(not(test))]
const GOOGLE_OAUTH_SETUP_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(3_600);
#[cfg(not(test))]
const GOOGLE_OAUTH_CALLBACK_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(600);

#[cfg(not(test))]
pub(crate) const GOOGLE_OAUTH_SETUP_RATE_LIMIT_PER_HOUR: usize = 6;
#[cfg(not(test))]
pub(crate) const GOOGLE_OAUTH_CALLBACK_RATE_LIMIT_PER_10_MIN: usize = 12;

#[cfg(not(test))]
static GOOGLE_OAUTH_RATE_LIMITER: LazyLock<OAuthRateLimiter> =
    LazyLock::new(OAuthRateLimiter::default);

// ── Constants ────────────────────────────────────────────────────────────────

/// Google OAuth2 authorization endpoint.
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google OAuth2 token endpoint.
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google OAuth2 token revocation endpoint.
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Credential store service name for all Google OAuth2 credentials.
/// Secondary accounts use `google_oauth:<account_id>`.
pub(crate) const CRED_SERVICE: &str = "google_oauth";

/// Rows that must all be present (and non-empty) under [`CRED_SERVICE`] (or
/// a per-account service) before [`GoogleAuthManager::from_credential_store`]
/// reports Google as configured. `settings_audit` checks the same list.
pub(crate) const REQUIRED_CRED_KEYS: &[&str] = &["client_id", "client_secret", "refresh_token"];

/// Refresh the access token this many seconds before it actually expires.
/// Provides a safety margin to avoid race conditions with in-flight requests.
const REFRESH_MARGIN_SECS: u64 = 300; // 5 minutes

/// How long to cache a failed refresh before retrying (prevents log spam when
/// the refresh token is revoked or expired).
const FAILURE_COOLDOWN_SECS: u64 = 300; // 5 minutes

/// Default port for the local OAuth2 callback server.
const DEFAULT_CALLBACK_PORT: u16 = 8765;

/// Maximum age of an OAuth2 state parameter before it's considered expired.
/// 30 minutes: user may need to configure redirect URI in Google Cloud Console first.
const STATE_MAX_AGE_SECS: u64 = 1800; // 30 minutes

/// Default scopes for initial authorization (Calendar read/write +
/// Drive read-only + Drive app-scoped file create/update).
/// calendar.events grants read+write on the authenticated user's own calendar events.
/// It does NOT grant write on other people's calendars — so the "self-only" guarantee
/// holds at the Google scope layer (plus the app-layer attendee-policy check in
/// `orchestrator_tools_calendar::check_attendee_policy`).
/// drive.readonly preserves broad read access to existing Drive content.
/// drive.file permits the app to create/upload files (and modify files it
/// created/opened) without granting full-drive delete powers.
///
/// Migration note: Google prompts re-consent on next `setup_google(action=setup)` because
/// the requested scope set changed (`.readonly` → full `.events`). Existing read-only
/// refresh tokens keep working for reads, but the user-facing consent string on
/// re-setup changes to "see, edit, share, and permanently delete events on all your
/// calendars". See docs/calendar-write-capability-plan.md §2.1.
pub(crate) const DEFAULT_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/drive.readonly",
    "https://www.googleapis.com/auth/drive.file",
];

/// Read-only scopes for secondary account linking (Calendar + Drive).
pub(crate) const READONLY_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar.events.readonly",
    "https://www.googleapis.com/auth/drive.readonly",
];

/// Credential store service key for a Google OAuth account.
pub(crate) fn google_oauth_cred_service(account_id: &str) -> String {
    if account_id == "primary" || account_id.is_empty() {
        CRED_SERVICE.to_string()
    } else {
        format!("{CRED_SERVICE}:{account_id}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OAuthRateLimitExceeded {
    pub retry_after_secs: u64,
}

#[derive(Debug, Default)]
pub(crate) struct OAuthRateLimiter {
    hits: parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl OAuthRateLimiter {
    fn check_at(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
        now: Instant,
    ) -> Option<OAuthRateLimitExceeded> {
        if limit == 0 {
            return Some(OAuthRateLimitExceeded {
                retry_after_secs: window.as_secs().max(1),
            });
        }

        let mut hits_by_key = self.hits.lock();
        let hits = hits_by_key.entry(key.to_string()).or_default();
        while hits
            .front()
            .is_some_and(|oldest| now.saturating_duration_since(*oldest) >= window)
        {
            hits.pop_front();
        }

        if hits.len() >= limit {
            let oldest = hits.front().copied().unwrap_or(now);
            let elapsed = now.saturating_duration_since(oldest);
            let retry_after = window
                .checked_sub(elapsed)
                .unwrap_or_default()
                .as_secs()
                .max(1);
            return Some(OAuthRateLimitExceeded {
                retry_after_secs: retry_after,
            });
        }

        hits.push_back(now);
        None
    }
}

#[cfg(not(test))]
pub(crate) fn check_google_oauth_setup_rate_limit(key: &str) -> Option<OAuthRateLimitExceeded> {
    GOOGLE_OAUTH_RATE_LIMITER.check_at(
        &format!("setup:{key}"),
        GOOGLE_OAUTH_SETUP_RATE_LIMIT_PER_HOUR,
        GOOGLE_OAUTH_SETUP_RATE_LIMIT_WINDOW,
        Instant::now(),
    )
}

#[cfg(test)]
pub(crate) fn check_google_oauth_setup_rate_limit(_key: &str) -> Option<OAuthRateLimitExceeded> {
    None
}

#[cfg(not(test))]
pub(crate) fn check_google_oauth_callback_rate_limit(key: &str) -> Option<OAuthRateLimitExceeded> {
    GOOGLE_OAUTH_RATE_LIMITER.check_at(
        &format!("callback:{key}"),
        GOOGLE_OAUTH_CALLBACK_RATE_LIMIT_PER_10_MIN,
        GOOGLE_OAUTH_CALLBACK_RATE_LIMIT_WINDOW,
        Instant::now(),
    )
}

#[cfg(test)]
pub(crate) fn check_google_oauth_callback_rate_limit(_key: &str) -> Option<OAuthRateLimitExceeded> {
    None
}

// ── Token Provider Trait ─────────────────────────────────────────────────────

/// Trait for providing authorization tokens to the sandbox's host_fetch.
///
/// Implementations must be Send + Sync for use behind Arc in the sandbox.
/// The method is synchronous because host_fetch runs in rayon's blocking pool.
pub trait TokenProvider: Send + Sync {
    /// Get a valid access token. Implementations should auto-refresh if needed.
    /// Returns the token string (without "Bearer " prefix).
    fn get_token(&self) -> Result<String>;
}

// ── Google Token Response ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

// ── Cached Token ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

impl CachedToken {
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at - Duration::from_secs(REFRESH_MARGIN_SECS)
    }
}

// ── Setup Result ─────────────────────────────────────────────────────────────

/// Result of the OAuth2 setup flow.
#[derive(Debug, Serialize)]
pub struct SetupResult {
    pub scopes: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GoogleScopeUpgradeAuditContext {
    pub account_name: String,
    pub current_scopes: Vec<String>,
    pub requested_scopes: Vec<String>,
    pub new_scopes: Vec<String>,
    pub scopes_origin: String,
}

pub(crate) fn store_scope_upgrade_audit_context(
    store: &CredentialStore,
    pending_service: &str,
    context: &GoogleScopeUpgradeAuditContext,
) -> Result<()> {
    let raw = serde_json::to_string(context).context("serializing Google scope audit context")?;
    store.store(pending_service, SCOPE_UPGRADE_CONTEXT_KEY, &raw)?;
    Ok(())
}

pub(crate) fn load_scope_upgrade_audit_context(
    store: &CredentialStore,
    pending_service: &str,
) -> Result<Option<GoogleScopeUpgradeAuditContext>> {
    let Some(raw) = store.get(pending_service, SCOPE_UPGRADE_CONTEXT_KEY)? else {
        return Ok(None);
    };
    let context = serde_json::from_str(&raw).context("parsing Google scope audit context")?;
    Ok(Some(context))
}

pub(crate) fn delete_scope_upgrade_audit_context(
    store: &CredentialStore,
    pending_service: &str,
) -> Result<()> {
    store.delete(pending_service, SCOPE_UPGRADE_CONTEXT_KEY)?;
    Ok(())
}

pub(crate) fn build_scope_upgrade_audit_record(
    context: &GoogleScopeUpgradeAuditContext,
    granted_scopes: &[String],
    flow_type: &str,
) -> serde_json::Value {
    let added_granted_scopes: Vec<String> = granted_scopes
        .iter()
        .filter(|scope| !context.current_scopes.contains(scope))
        .cloned()
        .collect();

    serde_json::json!({
        "schema_version": 1,
        "event_kind": "scope_upgrade_completed",
        "result": "success",
        "account_name": context.account_name,
        "flow_type": flow_type,
        "scopes_origin": context.scopes_origin,
        "old_scopes": context.current_scopes,
        "requested_scopes": context.requested_scopes,
        "new_scopes": context.new_scopes,
        "granted_scopes": granted_scopes,
        "added_granted_scopes": added_granted_scopes,
        "include_granted_scopes": true,
        "created_at": chrono::Utc::now().to_rfc3339(),
    })
}

// ── Google Auth Manager ──────────────────────────────────────────────────────

/// Manages OAuth2 tokens for Google Workspace APIs.
///
/// Thread-safe: the cached token is protected by a `parking_lot::RwLock`.
/// Token refresh is automatic and transparent to callers.
pub struct GoogleAuthManager {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    cached: RwLock<Option<CachedToken>>,
    http_client: reqwest::blocking::Client,
    scopes: Vec<String>,
    /// Tracks the last refresh failure to avoid hammering Google every 10s
    /// when the token is revoked. Cleared on successful refresh.
    last_failure: RwLock<Option<(Instant, String)>>,
    /// Prevents concurrent token refreshes from racing against Google's endpoint.
    refresh_lock: parking_lot::Mutex<()>,
}

// Prevent tokens from appearing in debug output
impl std::fmt::Debug for GoogleAuthManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAuthManager")
            .field("client_id", &"[redacted]")
            .field("scopes", &self.scopes)
            .field("has_cached_token", &self.cached.read().is_some())
            .field("in_failure_cooldown", &self.last_failure.read().is_some())
            .finish()
    }
}

impl GoogleAuthManager {
    /// Create a new manager with explicit credentials.
    pub fn new(
        client_id: String,
        client_secret: String,
        refresh_token: String,
        scopes: Vec<String>,
    ) -> Self {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client_id,
            client_secret,
            refresh_token,
            cached: RwLock::new(None),
            http_client,
            scopes,
            last_failure: RwLock::new(None),
            refresh_lock: parking_lot::Mutex::new(()),
        }
    }

    /// Load credentials from the encrypted credential store.
    ///
    /// Returns `Ok(Some(manager))` if all required credentials exist,
    /// `Ok(None)` if Google OAuth is not configured, or `Err` on decryption failure.
    pub fn from_credential_store(store: &CredentialStore) -> Result<Option<Self>> {
        let client_id = match store.get(CRED_SERVICE, "client_id")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let client_secret = match store.get(CRED_SERVICE, "client_secret")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let refresh_token = match store.get(CRED_SERVICE, "refresh_token")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let scopes = store
            .get(CRED_SERVICE, "scopes")?
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|| DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect());

        Ok(Some(Self::new(
            client_id,
            client_secret,
            refresh_token,
            scopes,
        )))
    }

    /// Load credentials from the encrypted credential store using a custom service key.
    ///
    /// This allows loading multiple Google accounts stored under different service
    /// keys (e.g. "google_oauth:work", "google_oauth:personal").
    pub fn from_credential_store_named(
        store: &CredentialStore,
        cred_service: &str,
    ) -> Result<Option<Self>> {
        let client_id = match store.get(cred_service, "client_id")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let client_secret = match store.get(cred_service, "client_secret")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let refresh_token = match store.get(cred_service, "refresh_token")? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let scopes = store
            .get(cred_service, "scopes")?
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|| DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect());

        Ok(Some(Self::new(
            client_id,
            client_secret,
            refresh_token,
            scopes,
        )))
    }

    /// Save credentials to the encrypted credential store.
    pub fn save_to_credential_store(&self, store: &CredentialStore) -> Result<()> {
        store.store(CRED_SERVICE, "client_id", &self.client_id)?;
        store.store(CRED_SERVICE, "client_secret", &self.client_secret)?;
        store.store(CRED_SERVICE, "refresh_token", &self.refresh_token)?;
        store.store(CRED_SERVICE, "scopes", &self.scopes.join(","))?;
        Ok(())
    }

    /// Save credentials under a custom service key (for multi-account support).
    pub fn save_to_credential_store_named(
        &self,
        store: &CredentialStore,
        cred_service: &str,
    ) -> Result<()> {
        store.store(cred_service, "client_id", &self.client_id)?;
        store.store(cred_service, "client_secret", &self.client_secret)?;
        store.store(cred_service, "refresh_token", &self.refresh_token)?;
        store.store(cred_service, "scopes", &self.scopes.join(","))?;
        Ok(())
    }

    /// Get a valid access token, refreshing if necessary.
    ///
    /// This is the primary method called by the sandbox's host_fetch.
    /// Thread-safe: uses read lock for fast path, upgrades to write lock only on refresh.
    pub fn access_token(&self) -> Result<String> {
        // Fast path: check if cached token is still valid
        {
            let guard = self.cached.read();
            if let Some(ref cached) = *guard
                && !cached.is_expired()
            {
                return Ok(cached.access_token.clone());
            }
        }

        // Check failure cooldown — avoid hammering Google when token is revoked
        {
            let guard = self.last_failure.read();
            if let Some((failed_at, ref msg)) = *guard
                && failed_at.elapsed() < Duration::from_secs(FAILURE_COOLDOWN_SECS)
            {
                anyhow::bail!(
                    "Google OAuth2 token refresh in cooldown ({}s remaining): {}",
                    FAILURE_COOLDOWN_SECS.saturating_sub(failed_at.elapsed().as_secs()),
                    msg
                );
            }
        }

        // Slow path: acquire refresh lock to prevent concurrent refreshes
        let _guard = self.refresh_lock.lock();
        // Double-check: another thread may have refreshed while we waited
        {
            let guard = self.cached.read();
            if let Some(ref cached) = *guard
                && !cached.is_expired()
            {
                return Ok(cached.access_token.clone());
            }
        }
        match self.refresh_access_token() {
            Ok(token) => {
                // Clear failure state on success
                *self.last_failure.write() = None;
                Ok(token)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                *self.last_failure.write() = Some((Instant::now(), msg));
                Err(e)
            }
        }
    }

    /// Force a token refresh, bypassing the cache.
    fn refresh_access_token(&self) -> Result<String> {
        let resp = self
            .http_client
            .post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("refresh_token", self.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .context("failed to send token refresh request")?;

        let status = resp.status();
        let body = resp.text().context("failed to read token response body")?;

        if !status.is_success() {
            // Try to parse error details
            if let Ok(err) = serde_json::from_str::<TokenErrorResponse>(&body) {
                anyhow::bail!(
                    "Google OAuth2 token refresh failed: {} — {}",
                    err.error,
                    err.error_description.unwrap_or_default()
                );
            }
            anyhow::bail!(
                "Google OAuth2 token refresh failed with status {}: {}",
                status,
                &body[..body.len().min(200)]
            );
        }

        let token_resp: TokenResponse =
            serde_json::from_str(&body).context("failed to parse token response JSON")?;

        let cached = CachedToken {
            access_token: token_resp.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(token_resp.expires_in),
        };

        *self.cached.write() = Some(cached);

        Ok(token_resp.access_token)
    }

    /// Validate that the current refresh token is still usable.
    /// Returns Ok(()) if a fresh access token can be obtained.
    pub fn validate(&self) -> Result<()> {
        self.refresh_access_token().map(|_| ())
    }

    /// Revoke the Google OAuth2 tokens.
    pub fn revoke(&self) -> Result<()> {
        let resp = self
            .http_client
            .post(GOOGLE_REVOKE_URL)
            .form(&[("token", self.refresh_token.as_str())])
            .send()
            .context("failed to send revocation request")?;

        if !resp.status().is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("Token revocation failed: {}", &body[..body.len().min(200)]);
        }

        // Clear cached token
        *self.cached.write() = None;

        Ok(())
    }

    /// Delete all Google OAuth credentials from the credential store.
    pub fn delete_from_credential_store(store: &CredentialStore) -> Result<()> {
        store.delete(CRED_SERVICE, "client_id")?;
        store.delete(CRED_SERVICE, "client_secret")?;
        store.delete(CRED_SERVICE, "refresh_token")?;
        store.delete(CRED_SERVICE, "scopes")?;
        Ok(())
    }

    /// Delete one finalized Google OAuth account by account id.
    ///
    /// Unlike the legacy single-account helper, this is fail-closed: attempting
    /// to revoke an account that has no finalized OAuth credentials returns an
    /// error instead of reporting success.
    pub fn delete_from_credential_store_account(
        store: &CredentialStore,
        account_id: &str,
    ) -> Result<()> {
        let cred_service = google_oauth_cred_service(account_id.trim());
        Self::delete_from_credential_store_named(store, &cred_service)
    }

    /// Delete one finalized Google OAuth credential service.
    pub fn delete_from_credential_store_named(
        store: &CredentialStore,
        cred_service: &str,
    ) -> Result<()> {
        if !is_finalized_google_oauth_service(cred_service) {
            anyhow::bail!("refusing to delete non-finalized Google OAuth service `{cred_service}`");
        }

        let keys = ["client_id", "client_secret", "refresh_token", "scopes"];
        let mut had_any = false;
        for key in keys {
            if store.get(cred_service, key)?.is_some() {
                had_any = true;
                break;
            }
        }
        if !had_any {
            anyhow::bail!("Google OAuth account `{cred_service}` is not configured");
        }

        for key in keys {
            store.delete(cred_service, key)?;
        }
        Ok(())
    }

    /// Get the currently configured scopes.
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    /// Build the authorization URL with a custom redirect URI and CSRF state parameter.
    ///
    /// Used for Telegram-driven setup where the callback goes to the server's
    /// public URL instead of localhost.
    pub fn build_auth_url_with_redirect(
        client_id: &str,
        scopes: &[String],
        redirect_uri: &str,
        state: &str,
    ) -> String {
        let scope_str = scopes.join(" ");
        format!(
            "{}?client_id={}&redirect_uri={}\
             &response_type=code&scope={}&access_type=offline&prompt=consent\
             &include_granted_scopes=true&state={}",
            GOOGLE_AUTH_URL,
            urlencoding(client_id),
            urlencoding(redirect_uri),
            urlencoding(&scope_str),
            urlencoding(state),
        )
    }

    /// Exchange an authorization code for tokens using a custom redirect URI.
    ///
    /// Like `exchange_code` but accepts an explicit redirect_uri string
    /// (for server-side callback flows).
    pub fn exchange_code_with_redirect(
        client_id: &str,
        client_secret: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<(Self, Vec<String>)> {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;

        let resp = http_client
            .post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .context("failed to exchange authorization code")?;

        let status = resp.status();
        let body = resp.text().context("failed to read exchange response")?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<TokenErrorResponse>(&body) {
                anyhow::bail!(
                    "Code exchange failed: {} — {}",
                    err.error,
                    err.error_description.unwrap_or_default()
                );
            }
            anyhow::bail!(
                "Code exchange failed with status {}: {}",
                status,
                &body[..body.len().min(200)]
            );
        }

        let token_resp: TokenResponse =
            serde_json::from_str(&body).context("failed to parse exchange response")?;

        let refresh_token = token_resp
            .refresh_token
            .context("no refresh_token in response (was access_type=offline used?)")?;

        let scopes: Vec<String> = token_resp
            .scope
            .unwrap_or_default()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let cached = CachedToken {
            access_token: token_resp.access_token,
            expires_at: Instant::now() + Duration::from_secs(token_resp.expires_in),
        };

        let manager = Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            refresh_token,
            cached: RwLock::new(Some(cached)),
            http_client,
            scopes: scopes.clone(),
            last_failure: RwLock::new(None),
            refresh_lock: parking_lot::Mutex::new(()),
        };

        Ok((manager, scopes))
    }

    /// Build the authorization URL for the initial OAuth2 consent flow.
    pub fn build_auth_url(client_id: &str, scopes: &[String], port: u16) -> String {
        let scope_str = scopes.join(" ");
        let redirect_uri = format!("http://localhost:{}/callback", port);
        format!(
            "{}?client_id={}&redirect_uri={}\
             &response_type=code&scope={}&access_type=offline&prompt=consent\
             &include_granted_scopes=true",
            GOOGLE_AUTH_URL,
            urlencoding(client_id),
            urlencoding(&redirect_uri),
            urlencoding(&scope_str),
        )
    }

    /// Exchange an authorization code for tokens.
    ///
    /// Called after the user completes the browser consent flow.
    /// Returns a new `GoogleAuthManager` with the obtained tokens.
    pub fn exchange_code(
        client_id: &str,
        client_secret: &str,
        code: &str,
        port: u16,
    ) -> Result<(Self, Vec<String>)> {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;

        let redirect_uri = format!("http://localhost:{}/callback", port);

        let resp = http_client
            .post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect_uri.as_str()),
            ])
            .send()
            .context("failed to exchange authorization code")?;

        let status = resp.status();
        let body = resp.text().context("failed to read exchange response")?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<TokenErrorResponse>(&body) {
                anyhow::bail!(
                    "Code exchange failed: {} — {}",
                    err.error,
                    err.error_description.unwrap_or_default()
                );
            }
            anyhow::bail!(
                "Code exchange failed with status {}: {}",
                status,
                &body[..body.len().min(200)]
            );
        }

        let token_resp: TokenResponse =
            serde_json::from_str(&body).context("failed to parse exchange response")?;

        let refresh_token = token_resp
            .refresh_token
            .context("no refresh_token in response (was access_type=offline used?)")?;

        let scopes: Vec<String> = token_resp
            .scope
            .unwrap_or_default()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let cached = CachedToken {
            access_token: token_resp.access_token,
            expires_at: Instant::now() + Duration::from_secs(token_resp.expires_in),
        };

        let manager = Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            refresh_token,
            cached: RwLock::new(Some(cached)),
            http_client,
            scopes: scopes.clone(),
            last_failure: RwLock::new(None),
            refresh_lock: parking_lot::Mutex::new(()),
        };

        Ok((manager, scopes))
    }

    /// Run the one-time OAuth2 setup flow.
    ///
    /// 1. Starts a temporary HTTP server on localhost to receive the callback.
    /// 2. Prints the authorization URL for the user to open in a browser.
    /// 3. Waits for the callback with the authorization code (5-minute timeout).
    /// 4. Exchanges the code for tokens.
    ///
    /// The caller is responsible for persisting tokens via `save_to_credential_store()`.
    pub async fn setup_flow(
        client_id: &str,
        client_secret: &str,
        scopes: &[String],
    ) -> Result<(Arc<Self>, SetupResult)> {
        let port = DEFAULT_CALLBACK_PORT;
        let auth_url = Self::build_auth_url(client_id, scopes, port);

        // Channel to receive the authorization code from the callback
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

        // Start temporary callback server
        let callback_app = axum::Router::new().route(
            "/callback",
            axum::routing::get(move |query: axum::extract::Query<CallbackQuery>| {
                let tx = tx.clone();
                async move {
                    if let Some(ref code) = query.code {
                        if let Err(e) = tx.send(code.clone()).await {
                            tracing::warn!(error = %e, "OAuth callback channel closed, code delivery failed");
                        }
                        (
                            axum::http::StatusCode::OK,
                            "Authorization successful! You can close this tab.",
                        )
                    } else {
                        let _error = query
                            .error
                            .as_deref()
                            .unwrap_or("unknown error");
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            "Authorization failed. Check the Aegis Runtime logs.",
                        )
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .with_context(|| format!("binding callback server to port {port}"))?;

        let server_handle = tokio::spawn(async move {
            axum::serve(listener, callback_app).await.ok();
        });

        eprintln!("[GoogleAuth] Waiting for authorization...");
        eprintln!("[GoogleAuth] Open this URL in a browser:");
        eprintln!("  {auth_url}");

        // Wait for the callback (with timeout)
        let code = tokio::time::timeout(Duration::from_secs(300), rx.recv())
            .await
            .context("authorization timed out (5 minutes)")?
            .context("callback channel closed unexpectedly")?;

        // Stop the callback server
        server_handle.abort();

        // Exchange code for tokens (blocking)
        let (manager, granted_scopes) = tokio::task::spawn_blocking({
            let client_id = client_id.to_string();
            let client_secret = client_secret.to_string();
            move || Self::exchange_code(&client_id, &client_secret, &code, port)
        })
        .await
        .context("exchange task panicked")??;

        let result = SetupResult {
            scopes: granted_scopes.clone(),
            message: format!(
                "Google OAuth2 setup complete. Granted scopes: {}",
                granted_scopes.join(", ")
            ),
        };

        Ok((Arc::new(manager), result))
    }
}

// Implement TokenProvider for seamless sandbox integration
impl TokenProvider for GoogleAuthManager {
    fn get_token(&self) -> Result<String> {
        self.access_token()
    }
}

fn is_finalized_google_oauth_service(service: &str) -> bool {
    service == CRED_SERVICE
        || service
            .strip_prefix("google_oauth:")
            .is_some_and(|suffix| !suffix.trim().is_empty())
}

// ── Callback Query ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    error: Option<String>,
}

// ── URL Encoding Helper ──────────────────────────────────────────────────────

/// Minimal percent-encoding for URL query parameters.
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push(char::from(HEX_CHARS[(byte >> 4) as usize]));
                result.push(char::from(HEX_CHARS[(byte & 0x0F) as usize]));
            }
        }
    }
    result
}

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

// ── OAuth2 CSRF State Parameter ─────────────────────────────────────────

/// Generate an HMAC-signed OAuth2 state parameter encoding `chat_id`.
///
/// Legacy format: `{chat_id}:{unix_timestamp}:{hex_hmac}`
/// Flow-aware format: `{chat_id}:{flow_id}:{unix_timestamp}:{hex_hmac}`
///
/// The HMAC is computed over `{chat_id}:{timestamp}` using the provided
/// signing key (typically derived from the agent's Ed25519 identity).
pub fn generate_oauth_state(chat_id: i64, signing_key: &[u8; 32]) -> String {
    generate_oauth_state_for_flow(chat_id, "manual", signing_key)
}

pub fn generate_oauth_state_for_flow(
    chat_id: i64,
    flow_id: &str,
    signing_key: &[u8; 32],
) -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    generate_oauth_state_with_time(chat_id, flow_id, signing_key, timestamp)
}

/// Generate state with an explicit timestamp (for testing).
fn generate_oauth_state_with_time(
    chat_id: i64,
    flow_id: &str,
    signing_key: &[u8; 32],
    timestamp: u64,
) -> String {
    let payload = format!("{chat_id}:{flow_id}:{timestamp}");
    let mut mac = HmacSha256::new_from_slice(signing_key).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());
    format!("{payload}:{hmac_hex}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthState {
    pub chat_id: i64,
    pub flow_id: String,
}

/// Verify an HMAC-signed OAuth2 state parameter and extract the `chat_id`.
///
/// Returns `Err` if the signature is invalid, the state is malformed,
/// or the timestamp has expired (older than `STATE_MAX_AGE_SECS`).
pub fn verify_oauth_state(state: &str, signing_key: &[u8; 32]) -> Result<i64> {
    Ok(verify_oauth_state_details(state, signing_key)?.chat_id)
}

pub fn verify_oauth_state_details(state: &str, signing_key: &[u8; 32]) -> Result<OAuthState> {
    verify_oauth_state_with_time(state, signing_key, None)
}

/// Verify state with an optional override for "now" (for testing expiry).
fn verify_oauth_state_with_time(
    state: &str,
    signing_key: &[u8; 32],
    now_override: Option<u64>,
) -> Result<OAuthState> {
    let (chat_id, flow_id, timestamp, hmac_hex, payload) = if state.matches(':').count() >= 3 {
        let parts: Vec<&str> = state.splitn(4, ':').collect();
        if parts.len() != 4 {
            anyhow::bail!("malformed OAuth state: expected 4 colon-separated parts");
        }
        let chat_id: i64 = parts[0].parse().context("invalid chat_id in OAuth state")?;
        let flow_id = parts[1].to_string();
        let timestamp: u64 = parts[2]
            .parse()
            .context("invalid timestamp in OAuth state")?;
        let payload = format!("{chat_id}:{flow_id}:{timestamp}");
        (chat_id, flow_id, timestamp, parts[3], payload)
    } else {
        let parts: Vec<&str> = state.splitn(3, ':').collect();
        if parts.len() != 3 {
            anyhow::bail!("malformed OAuth state: expected 3 colon-separated parts");
        }
        let chat_id: i64 = parts[0].parse().context("invalid chat_id in OAuth state")?;
        let timestamp: u64 = parts[1]
            .parse()
            .context("invalid timestamp in OAuth state")?;
        let payload = format!("{chat_id}:manual:{timestamp}");
        (chat_id, "manual".to_string(), timestamp, parts[2], payload)
    };

    // Verify HMAC
    let mut mac = HmacSha256::new_from_slice(signing_key).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());

    let expected_bytes = hex::decode(hmac_hex).context("invalid hex in OAuth state HMAC")?;
    mac.verify_slice(&expected_bytes)
        .map_err(|_| anyhow::anyhow!("OAuth state HMAC verification failed"))?;

    // Check expiry
    let now = now_override.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });
    if now.saturating_sub(timestamp) > STATE_MAX_AGE_SECS {
        anyhow::bail!(
            "OAuth state expired (age {}s > max {}s)",
            now.saturating_sub(timestamp),
            STATE_MAX_AGE_SECS
        );
    }

    Ok(OAuthState { chat_id, flow_id })
}

/// Credential-store service key for pending OAuth setup (not yet authorized).
pub const CRED_SERVICE_PENDING: &str = "google_oauth_pending";

/// Credential-store service key for a chat-scoped pending OAuth setup.
///
/// Using the verified Telegram `chat_id` isolates concurrent setup attempts
/// across different user chats without changing the public callback state
/// protocol. `None` preserves the legacy single-slot key.
pub fn pending_cred_service(chat_id: Option<i64>) -> String {
    match chat_id {
        Some(chat_id) => format!("{CRED_SERVICE_PENDING}:{chat_id}"),
        None => CRED_SERVICE_PENDING.to_string(),
    }
}

/// Flow-specific pending OAuth service key.
pub fn pending_cred_service_for_flow(chat_id: i64, flow_id: &str) -> String {
    format!("{CRED_SERVICE_PENDING}:{chat_id}:{flow_id}")
}

pub fn generate_setup_flow_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── URL encoding ──

    #[test]
    fn urlencoding_basic() {
        assert_eq!(urlencoding("hello"), "hello");
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a+b=c"), "a%2Bb%3Dc");
    }

    #[test]
    fn urlencoding_scope_string() {
        let scopes = "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/drive.readonly";
        let encoded = urlencoding(scopes);
        assert!(encoded.contains("https%3A%2F%2F"));
        assert!(encoded.contains("%20")); // space between scopes
        assert!(!encoded.contains(' ')); // no literal spaces
    }

    #[test]
    fn urlencoding_preserves_safe_chars() {
        assert_eq!(urlencoding("abc-123_XYZ.~"), "abc-123_XYZ.~");
    }

    // ── Auth URL building ──

    #[test]
    fn build_auth_url_contains_required_params() {
        let scopes = vec!["https://www.googleapis.com/auth/calendar.events".to_string()];
        let url = GoogleAuthManager::build_auth_url("test-client-id", &scopes, 8765);

        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765%2Fcallback"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("include_granted_scopes=true"));
        assert!(url.contains("calendar.events"));
    }

    #[test]
    fn build_auth_url_multiple_scopes() {
        let scopes = vec![
            "https://www.googleapis.com/auth/calendar.events".to_string(),
            "https://www.googleapis.com/auth/drive.readonly".to_string(),
        ];
        let url = GoogleAuthManager::build_auth_url("cid", &scopes, 9999);
        // Scopes are space-separated (encoded as %20)
        assert!(url.contains("calendar.events%20"));
    }

    // ── CachedToken expiry ──

    #[test]
    fn cached_token_not_expired_when_fresh() {
        let token = CachedToken {
            access_token: "test".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn cached_token_expired_within_margin() {
        let token = CachedToken {
            access_token: "test".to_string(),
            // Expires in 4 minutes — within the 5-minute refresh margin
            expires_at: Instant::now() + Duration::from_secs(240),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn cached_token_expired_in_past() {
        let token = CachedToken {
            access_token: "test".to_string(),
            expires_at: Instant::now() - Duration::from_secs(10),
        };
        assert!(token.is_expired());
    }

    // ── GoogleAuthManager construction ──

    #[test]
    fn new_manager_has_no_cached_token() {
        let mgr = GoogleAuthManager::new(
            "cid".into(),
            "csecret".into(),
            "refresh".into(),
            vec!["scope1".into()],
        );
        assert!(mgr.cached.read().is_none());
    }

    #[test]
    fn debug_impl_redacts_secrets() {
        let mgr = GoogleAuthManager::new(
            "my-secret-client-id".into(),
            "my-secret-client-secret".into(),
            "my-secret-refresh-token".into(),
            vec!["scope".into()],
        );
        let debug = format!("{:?}", mgr);
        assert!(!debug.contains("my-secret-client-id"));
        assert!(!debug.contains("my-secret-client-secret"));
        assert!(!debug.contains("my-secret-refresh-token"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn scopes_accessor() {
        let mgr = GoogleAuthManager::new(
            "cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["a".into(), "b".into()],
        );
        assert_eq!(mgr.scopes(), &["a", "b"]);
    }

    // ── Credential store roundtrip ──

    #[test]
    fn credential_store_roundtrip() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let original = GoogleAuthManager::new(
            "test-client-id".into(),
            "test-secret".into(),
            "test-refresh-token".into(),
            vec!["scope1".into(), "scope2".into()],
        );

        original.save_to_credential_store(&store).unwrap();

        let loaded = GoogleAuthManager::from_credential_store(&store)
            .unwrap()
            .expect("should find stored credentials");

        assert_eq!(loaded.client_id, "test-client-id");
        assert_eq!(loaded.client_secret, "test-secret");
        assert_eq!(loaded.refresh_token, "test-refresh-token");
        assert_eq!(loaded.scopes, vec!["scope1", "scope2"]);
    }

    #[test]
    fn from_credential_store_returns_none_when_empty() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let result = GoogleAuthManager::from_credential_store(&store).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_credential_store_returns_none_when_partial() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        // Only store client_id, missing client_secret and refresh_token
        store.store(CRED_SERVICE, "client_id", "cid").unwrap();

        let result = GoogleAuthManager::from_credential_store(&store).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_from_credential_store_removes_all() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let mgr = GoogleAuthManager::new("cid".into(), "cs".into(), "rt".into(), vec!["s".into()]);
        mgr.save_to_credential_store(&store).unwrap();

        // Verify stored
        assert!(store.get(CRED_SERVICE, "client_id").unwrap().is_some());

        GoogleAuthManager::delete_from_credential_store(&store).unwrap();

        // Verify deleted
        assert!(store.get(CRED_SERVICE, "client_id").unwrap().is_none());
        assert!(store.get(CRED_SERVICE, "client_secret").unwrap().is_none());
        assert!(store.get(CRED_SERVICE, "refresh_token").unwrap().is_none());
        assert!(store.get(CRED_SERVICE, "scopes").unwrap().is_none());
    }

    #[test]
    fn delete_named_account_does_not_remove_default_account() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let primary = GoogleAuthManager::new(
            "primary-cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["s".into()],
        );
        primary.save_to_credential_store(&store).unwrap();
        let work = GoogleAuthManager::new(
            "work-cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["s".into()],
        );
        work.save_to_credential_store_named(&store, "google_oauth:work")
            .unwrap();

        GoogleAuthManager::delete_from_credential_store_account(&store, "work").unwrap();

        assert_eq!(
            store.get(CRED_SERVICE, "client_id").unwrap().as_deref(),
            Some("primary-cid")
        );
        assert!(
            store
                .get("google_oauth:work", "client_id")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get("google_oauth:work", "refresh_token")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn delete_default_account_does_not_remove_named_account() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let primary = GoogleAuthManager::new(
            "primary-cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["s".into()],
        );
        primary.save_to_credential_store(&store).unwrap();
        let work = GoogleAuthManager::new(
            "work-cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["s".into()],
        );
        work.save_to_credential_store_named(&store, "google_oauth:work")
            .unwrap();

        GoogleAuthManager::delete_from_credential_store_account(&store, "primary").unwrap();

        assert!(store.get(CRED_SERVICE, "client_id").unwrap().is_none());
        assert_eq!(
            store
                .get("google_oauth:work", "client_id")
                .unwrap()
                .as_deref(),
            Some("work-cid")
        );
    }

    #[test]
    fn delete_unknown_account_fails_closed() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        let primary = GoogleAuthManager::new(
            "primary-cid".into(),
            "cs".into(),
            "rt".into(),
            vec!["s".into()],
        );
        primary.save_to_credential_store(&store).unwrap();

        let err = GoogleAuthManager::delete_from_credential_store_account(&store, "missing")
            .expect_err("missing account should fail closed");

        assert!(err.to_string().contains("not configured"));
        assert_eq!(
            store.get(CRED_SERVICE, "client_id").unwrap().as_deref(),
            Some("primary-cid")
        );
    }

    #[test]
    fn default_scopes_contain_calendar_and_drive() {
        // Calendar must be full-write (events, not events.readonly) so
        // calendar_create/update/delete_event can succeed on the primary
        // account. See docs/calendar-write-capability-plan.md §2.1.
        assert!(
            DEFAULT_SCOPES.contains(&"https://www.googleapis.com/auth/calendar.events"),
            "DEFAULT_SCOPES must include calendar.events (read+write) — \
             readonly alone blocks the calendar write tools"
        );
        assert!(
            !DEFAULT_SCOPES
                .iter()
                .any(|s| s.contains("calendar.events.readonly")),
            "DEFAULT_SCOPES must not keep calendar.events.readonly once \
             the widened scope is granted; otherwise Google may surface \
             duplicate consent lines"
        );
        assert!(
            DEFAULT_SCOPES.contains(&"https://www.googleapis.com/auth/drive.readonly"),
            "DEFAULT_SCOPES must keep drive.readonly for broad Drive read access"
        );
        assert!(
            DEFAULT_SCOPES.contains(&"https://www.googleapis.com/auth/drive.file"),
            "DEFAULT_SCOPES must include drive.file so the system can upload \
             files without requesting full-drive delete powers"
        );
        assert_eq!(
            DEFAULT_SCOPES.len(),
            3,
            "DEFAULT_SCOPES is calendar.events + drive.readonly + drive.file"
        );
    }

    // ── Token provider trait ──

    #[test]
    fn token_provider_returns_cached_when_valid() {
        let mgr = GoogleAuthManager::new("cid".into(), "cs".into(), "rt".into(), vec![]);
        // Manually inject a cached token
        *mgr.cached.write() = Some(CachedToken {
            access_token: "cached-access-token".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });

        let token = mgr.get_token().unwrap();
        assert_eq!(token, "cached-access-token");
    }

    #[test]
    fn access_token_returns_cached_on_second_call() {
        let mgr = GoogleAuthManager::new("cid".into(), "cs".into(), "rt".into(), vec![]);
        // Inject cached token
        *mgr.cached.write() = Some(CachedToken {
            access_token: "token-123".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });

        let t1 = mgr.access_token().unwrap();
        let t2 = mgr.access_token().unwrap();
        assert_eq!(t1, t2);
        assert_eq!(t1, "token-123");
    }

    // ── Scope helpers ──

    #[test]
    fn scopes_parsed_from_comma_separated() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        store.store(CRED_SERVICE, "client_id", "cid").unwrap();
        store.store(CRED_SERVICE, "client_secret", "cs").unwrap();
        store.store(CRED_SERVICE, "refresh_token", "rt").unwrap();
        store
            .store(CRED_SERVICE, "scopes", "scope_a, scope_b, scope_c")
            .unwrap();

        let mgr = GoogleAuthManager::from_credential_store(&store)
            .unwrap()
            .unwrap();
        assert_eq!(mgr.scopes, vec!["scope_a", "scope_b", "scope_c"]);
    }

    #[test]
    fn default_scopes_used_when_not_stored() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();

        store.store(CRED_SERVICE, "client_id", "cid").unwrap();
        store.store(CRED_SERVICE, "client_secret", "cs").unwrap();
        store.store(CRED_SERVICE, "refresh_token", "rt").unwrap();
        // No scopes stored

        let mgr = GoogleAuthManager::from_credential_store(&store)
            .unwrap()
            .unwrap();
        assert_eq!(mgr.scopes.len(), DEFAULT_SCOPES.len());
    }

    // ── OAuth state parameter tests ──

    #[test]
    fn generate_verify_state_roundtrip() {
        let key = [42u8; 32];
        let chat_id = 123456789_i64;
        let state = generate_oauth_state(chat_id, &key);
        let result = verify_oauth_state(&state, &key).unwrap();
        assert_eq!(result, chat_id);
    }

    #[test]
    fn verify_state_rejects_tampered_signature() {
        let key = [42u8; 32];
        let state = generate_oauth_state(100, &key);
        // Tamper: flip a char in the HMAC
        let mut tampered = state.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'a' { 'b' } else { 'a' });
        assert!(verify_oauth_state(&tampered, &key).is_err());
    }

    #[test]
    fn verify_state_rejects_tampered_chat_id() {
        let key = [42u8; 32];
        let state = generate_oauth_state(100, &key);
        // Replace chat_id 100 with 999
        let tampered = state.replacen("100:", "999:", 1);
        assert!(verify_oauth_state(&tampered, &key).is_err());
    }

    #[test]
    fn verify_state_rejects_wrong_key() {
        let key1 = [42u8; 32];
        let key2 = [99u8; 32];
        let state = generate_oauth_state(100, &key1);
        assert!(verify_oauth_state(&state, &key2).is_err());
    }

    #[test]
    fn verify_state_rejects_expired() {
        let key = [42u8; 32];
        let old_timestamp = 1000u64; // way in the past
        let state = generate_oauth_state_with_time(100, "manual", &key, old_timestamp);
        // Verify with "now" = current time (will be >> 10 min past old_timestamp)
        let result = verify_oauth_state(&state, &key);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("expired"),
            "error should mention expiry"
        );
    }

    #[test]
    fn verify_state_accepts_within_window() {
        let key = [42u8; 32];
        let now = 1_700_000_000u64;
        let state = generate_oauth_state_with_time(100, "manual", &key, now);
        // Verify with "now" = 5 minutes later (within 10-minute window)
        let result = verify_oauth_state_with_time(&state, &key, Some(now + 300));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().chat_id, 100);
    }

    #[test]
    fn verify_state_rejects_malformed() {
        let key = [42u8; 32];
        assert!(verify_oauth_state("", &key).is_err());
        assert!(verify_oauth_state("just_one_part", &key).is_err());
        assert!(verify_oauth_state("two:parts", &key).is_err());
        assert!(verify_oauth_state("not_a_number:123:abcdef", &key).is_err());
    }

    #[test]
    fn verify_state_negative_chat_id() {
        let key = [42u8; 32];
        let chat_id = -1001234567890_i64; // Telegram group chat IDs are negative
        let state = generate_oauth_state(chat_id, &key);
        assert_eq!(verify_oauth_state(&state, &key).unwrap(), chat_id);
    }

    #[test]
    fn flow_aware_state_roundtrip_preserves_flow_id() {
        let key = [42u8; 32];
        let state = generate_oauth_state_for_flow(123, "flow-abc", &key);
        let parsed = verify_oauth_state_details(&state, &key).unwrap();
        assert_eq!(parsed.chat_id, 123);
        assert_eq!(parsed.flow_id, "flow-abc");
    }

    #[test]
    fn legacy_state_roundtrip_defaults_flow_id_to_manual() {
        let key = [42u8; 32];
        let state = generate_oauth_state(321, &key);
        let parsed = verify_oauth_state_details(&state, &key).unwrap();
        assert_eq!(parsed.chat_id, 321);
        assert_eq!(parsed.flow_id, "manual");
    }

    #[test]
    fn pending_cred_service_for_flow_namespaces_by_chat_and_flow() {
        assert_eq!(
            pending_cred_service_for_flow(123, "flow-abc"),
            "google_oauth_pending:123:flow-abc"
        );
        assert_eq!(
            pending_cred_service_for_flow(-100, "manual"),
            "google_oauth_pending:-100:manual"
        );
    }

    #[test]
    fn scope_upgrade_audit_context_roundtrips_and_record_shows_delta() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let key = crate::credential_store::derive_key_from_ed25519(&[42u8; 32]);
        let store = CredentialStore::open(conn, key).unwrap();
        let context = GoogleScopeUpgradeAuditContext {
            account_name: "work".to_string(),
            current_scopes: vec!["https://www.googleapis.com/auth/drive.readonly".to_string()],
            requested_scopes: vec![
                "https://www.googleapis.com/auth/drive.readonly".to_string(),
                "https://www.googleapis.com/auth/calendar.events".to_string(),
            ],
            new_scopes: vec!["https://www.googleapis.com/auth/calendar.events".to_string()],
            scopes_origin: "default".to_string(),
        };

        store_scope_upgrade_audit_context(&store, "google_oauth_pending:test", &context).unwrap();
        let loaded = load_scope_upgrade_audit_context(&store, "google_oauth_pending:test").unwrap();
        assert_eq!(loaded.as_ref(), Some(&context));

        let record = build_scope_upgrade_audit_record(
            &context,
            &[
                "https://www.googleapis.com/auth/drive.readonly".to_string(),
                "https://www.googleapis.com/auth/calendar.events".to_string(),
            ],
            "manual",
        );
        assert_eq!(record["event_kind"], "scope_upgrade_completed");
        assert_eq!(record["account_name"], "work");
        assert_eq!(record["schema_version"], 1);
        assert_eq!(
            record["added_granted_scopes"][0],
            "https://www.googleapis.com/auth/calendar.events"
        );

        delete_scope_upgrade_audit_context(&store, "google_oauth_pending:test").unwrap();
        assert!(
            load_scope_upgrade_audit_context(&store, "google_oauth_pending:test")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn oauth_rate_limiter_blocks_until_window_expires() {
        let limiter = OAuthRateLimiter::default();
        let key = "setup:chat-1:primary";
        let start = Instant::now();

        assert_eq!(
            limiter.check_at(key, 2, Duration::from_secs(60), start),
            None
        );
        assert_eq!(
            limiter.check_at(
                key,
                2,
                Duration::from_secs(60),
                start + Duration::from_secs(10)
            ),
            None
        );
        assert_eq!(
            limiter.check_at(
                key,
                2,
                Duration::from_secs(60),
                start + Duration::from_secs(20)
            ),
            Some(OAuthRateLimitExceeded {
                retry_after_secs: 40,
            })
        );

        assert_eq!(
            limiter.check_at(
                key,
                2,
                Duration::from_secs(60),
                start + Duration::from_secs(61)
            ),
            None
        );
    }

    // ── Custom redirect URI tests ──

    #[test]
    fn build_auth_url_with_redirect_contains_state() {
        let scopes = vec!["https://www.googleapis.com/auth/calendar.events".to_string()];
        let url = GoogleAuthManager::build_auth_url_with_redirect(
            "test-cid",
            &scopes,
            "https://aegis.example.com/auth/callback/google",
            "100:12345:abcdef",
        );
        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("client_id=test-cid"));
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Faegis.example.com%2Fauth%2Fcallback%2Fgoogle")
        );
        assert!(url.contains("state=100%3A12345%3Aabcdef"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
    }

    #[test]
    fn build_auth_url_with_redirect_multiple_scopes() {
        let scopes = vec![
            "https://www.googleapis.com/auth/calendar.events".to_string(),
            "https://www.googleapis.com/auth/drive.readonly".to_string(),
        ];
        let url = GoogleAuthManager::build_auth_url_with_redirect(
            "cid",
            &scopes,
            "http://localhost:8765/callback",
            "state123",
        );
        assert!(url.contains("calendar.events%20"));
        assert!(url.contains("state=state123"));
    }
}
