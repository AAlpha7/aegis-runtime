//! Web Console Google Login primitives.
//!
//! This module owns the standalone Web Console Google Login flow. It is kept
//! separate from the Workspace/Google OAuth flow so login cannot silently gain
//! mail scopes, refresh tokens, or background API access.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

const DEFAULT_REDIRECT_PATH: &str = "/auth/callback/google";
const DEFAULT_SESSION_COOKIE_NAME: &str = "__Host-aegis_session";
const DEFAULT_SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const DEFAULT_SESSION_IDLE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const LOGIN_STATE_TTL_SECS: u64 = 600;
const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_JWKS_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v3/certs";
const GOOGLE_OIDC_ISSUERS: &[&str] = &["accounts.google.com", "https://accounts.google.com"];
const LOGIN_SCOPES: [&str; 3] = ["openid", "email", "profile"];
const TOTP_SECRET_BYTES: usize = 20;
const TOTP_STEP_SECS: u64 = 30;
const TOTP_DIGITS: u32 = 6;
pub const CSRF_HEADER_NAME: &str = "x-aegis-csrf";
/// Subject stored for a session minted through the local login link.
pub const LOCAL_LOGIN_SUB: &str = "local:owner";
/// E-mail stored for a session minted through the local login link. The
/// `.invalid` TLD (RFC 2606) guarantees it can never collide with a real
/// Google identity.
pub const LOCAL_LOGIN_EMAIL: &str = "owner@local.invalid";
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebConsoleLoginConfig {
    pub enabled: bool,
    pub client_id: String,
    pub client_secret_present: bool,
    pub allowed_emails: HashSet<String>,
    pub allowed_domain: Option<String>,
    pub redirect_path: String,
    pub session_cookie_secret_present: bool,
    pub session_cookie_name: String,
    pub session_ttl_secs: u64,
    pub session_idle_ttl_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLoginState {
    pub state_hash: String,
    pub nonce: String,
    pub code_verifier: String,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub return_to: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginUrl {
    pub url: String,
    pub state: PendingLoginState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleLoginTokenResponse {
    pub id_token: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleTokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoogleJwks {
    pub keys: Vec<GoogleJwk>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoogleJwk {
    pub kid: String,
    pub n: String,
    pub e: String,
    #[serde(default)]
    pub alg: Option<String>,
    #[serde(default)]
    pub kty: Option<String>,
    #[serde(default)]
    pub r#use: Option<String>,
}

impl GoogleJwks {
    fn find(&self, kid: &str) -> Option<&GoogleJwk> {
        self.keys.iter().find(|key| key.kid == kid)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GoogleIdClaims {
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default)]
    pub auth_time: Option<i64>,
    #[serde(default)]
    pub amr: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub hd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MfaLevel {
    None,
    GoogleOnly,
    GoogleFresh,
    Totp,
    WebAuthn,
    RecoveryCode,
    /// Control of this host, proven by the local login token. Opens S3
    /// detail reads. Does not satisfy approval step-up.
    Host,
    /// Explicit second confirmation from the local host session. Satisfies
    /// approval step-up for the freshness window. Does not open S3 detail reads.
    HostConfirmed,
}

impl MfaLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GoogleOnly => "google_only",
            Self::GoogleFresh => "google_fresh",
            Self::Totp => "totp",
            Self::WebAuthn => "webauthn",
            Self::RecoveryCode => "recovery_code",
            Self::Host => "host",
            Self::HostConfirmed => "host_confirmed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "google_only" => Self::GoogleOnly,
            "google_fresh" => Self::GoogleFresh,
            "totp" => Self::Totp,
            "webauthn" => Self::WebAuthn,
            "recovery_code" => Self::RecoveryCode,
            "host" => Self::Host,
            "host_confirmed" => Self::HostConfirmed,
            _ => Self::None,
        }
    }
}

impl MfaStepUpPolicy {
    pub fn recent_mfa(reason: impl Into<String>) -> Self {
        Self {
            required_level: RequiredMfaLevel::Any,
            freshness_secs: crate::data_access_gate::DEFAULT_MFA_FRESHNESS_SECS,
            reason: reason.into(),
        }
    }

    pub fn recent_webauthn(reason: impl Into<String>) -> Self {
        Self {
            required_level: RequiredMfaLevel::WebAuthn,
            freshness_secs: crate::data_access_gate::DEFAULT_MFA_FRESHNESS_SECS,
            reason: reason.into(),
        }
    }
}

pub fn require_recent_mfa(
    policy: &MfaStepUpPolicy,
    current_level: MfaLevel,
    mfa_verified_at: Option<u64>,
    now: u64,
) -> MfaStepUpDecision {
    if recent_mfa_satisfies(
        policy.required_level,
        current_level,
        mfa_verified_at,
        now,
        policy.freshness_secs,
    ) {
        MfaStepUpDecision::Satisfied
    } else {
        MfaStepUpDecision::Required(MfaRequiredResponse {
            error: "mfa_required",
            required_level: policy.required_level.as_str().to_string(),
            reason: policy.reason.clone(),
            freshness_secs: policy.freshness_secs,
            challenge_hint: mfa_challenge_hint(policy.required_level),
        })
    }
}

fn mfa_challenge_hint(required_level: RequiredMfaLevel) -> MfaChallengeHint {
    let start_url = match required_level {
        RequiredMfaLevel::Any => "/ui/?tab=system",
        RequiredMfaLevel::WebAuthn => "/mfa/webauthn/assert/start",
    };
    MfaChallengeHint {
        start_url: start_url.into(),
    }
}

pub fn require_recent_session_mfa(
    policy: &MfaStepUpPolicy,
    session: &WebConsoleSession,
    now: u64,
) -> MfaStepUpDecision {
    require_recent_mfa(policy, session.mfa_level, session.mfa_verified_at, now)
}

fn recent_mfa_satisfies(
    required_level: RequiredMfaLevel,
    current_level: MfaLevel,
    mfa_verified_at: Option<u64>,
    now: u64,
    freshness_secs: u64,
) -> bool {
    let Some(verified_at) = mfa_verified_at else {
        return false;
    };
    if verified_at > now || now.saturating_sub(verified_at) > freshness_secs {
        return false;
    }
    match required_level {
        RequiredMfaLevel::Any => matches!(
            current_level,
            MfaLevel::GoogleFresh | MfaLevel::Totp | MfaLevel::WebAuthn | MfaLevel::HostConfirmed
        ),
        RequiredMfaLevel::WebAuthn => current_level == MfaLevel::WebAuthn,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredMfaLevel {
    Any,
    WebAuthn,
}

impl RequiredMfaLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::WebAuthn => "webauthn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MfaStepUpPolicy {
    pub required_level: RequiredMfaLevel,
    pub freshness_secs: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MfaChallengeHint {
    pub start_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MfaRequiredResponse {
    pub error: &'static str,
    pub required_level: String,
    pub reason: String,
    pub freshness_secs: u64,
    pub challenge_hint: MfaChallengeHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MfaStepUpDecision {
    Satisfied,
    Required(MfaRequiredResponse),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebConsoleSession {
    pub session_id_hash: String,
    pub google_sub: String,
    pub email: String,
    pub email_verified: bool,
    pub created_at: u64,
    pub expires_at: u64,
    pub idle_expires_at: u64,
    pub last_seen_at: Option<u64>,
    pub revoked_at: Option<u64>,
    pub mfa_level: MfaLevel,
    pub mfa_verified_at: Option<u64>,
    pub mfa_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedSession {
    pub cookie_token: String,
    pub session: WebConsoleSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMfaEvidence {
    pub level: MfaLevel,
    pub verified_at: u64,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MfaStatus {
    pub mfa_level: String,
    pub mfa_verified_at: Option<u64>,
    pub requires_mfa_setup: bool,
    pub webauthn_registered_count: u64,
    pub webauthn_available: bool,
    pub webauthn_status: String,
    pub totp_configured: bool,
    pub preferred_step_up_method: String,
    pub recovery_codes_remaining: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TotpSetup {
    pub secret_base32: String,
    pub otpauth_url: String,
    pub issuer: String,
    pub account_name: String,
    pub digits: u32,
    pub period_secs: u64,
    pub algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebAuthnCredentialSummary {
    pub id: String,
    pub nickname: Option<String>,
    pub transports: Vec<String>,
    pub backup_eligible: Option<bool>,
    pub backup_state: Option<bool>,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

impl WebConsoleLoginConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        let enabled = parse_bool(&lookup("GOOGLE_LOGIN_ENABLED")).unwrap_or(false);
        let allowed_emails = lookup("GOOGLE_LOGIN_ALLOWED_EMAILS")
            .map(|value| parse_allowed_emails(&value))
            .unwrap_or_default();
        let allowed_domain = lookup("GOOGLE_LOGIN_ALLOWED_DOMAIN")
            .map(|value| value.trim().trim_start_matches('@').to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        let redirect_path = lookup("GOOGLE_LOGIN_REDIRECT_PATH")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_REDIRECT_PATH.to_string());
        let session_cookie_name = lookup("SESSION_COOKIE_NAME")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION_COOKIE_NAME.to_string());

        Self {
            enabled,
            client_id: lookup("GOOGLE_LOGIN_CLIENT_ID")
                .or_else(|| lookup("GOOGLE_CLIENT_ID"))
                .unwrap_or_default()
                .trim()
                .to_string(),
            client_secret_present: lookup("GOOGLE_LOGIN_CLIENT_SECRET")
                .or_else(|| lookup("GOOGLE_CLIENT_SECRET"))
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            allowed_emails,
            allowed_domain,
            redirect_path,
            session_cookie_secret_present: lookup("SESSION_COOKIE_SECRET")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            session_cookie_name,
            session_ttl_secs: parse_u64(&lookup("SESSION_TTL_SECS"))
                .unwrap_or(DEFAULT_SESSION_TTL_SECS),
            session_idle_ttl_secs: parse_u64(&lookup("SESSION_IDLE_TTL_SECS"))
                .unwrap_or(DEFAULT_SESSION_IDLE_TTL_SECS),
        }
    }

    pub fn validate_for_login(&self) -> Result<()> {
        if !self.enabled {
            bail!("Google Login is disabled");
        }
        if self.client_id.is_empty() {
            bail!("GOOGLE_LOGIN_CLIENT_ID is required when Google Login is enabled");
        }
        if !self.client_secret_present {
            bail!("GOOGLE_LOGIN_CLIENT_SECRET is required when Google Login is enabled");
        }
        if self.allowed_emails.is_empty() && self.allowed_domain.is_none() {
            bail!(
                "GOOGLE_LOGIN_ALLOWED_EMAILS or GOOGLE_LOGIN_ALLOWED_DOMAIN is required when Google Login is enabled"
            );
        }
        if !self.allowed_emails.is_empty() && self.allowed_domain.is_some() {
            bail!(
                "GOOGLE_LOGIN_ALLOWED_EMAILS and GOOGLE_LOGIN_ALLOWED_DOMAIN are mutually exclusive"
            );
        }
        if !self.session_cookie_secret_present {
            bail!("SESSION_COOKIE_SECRET is required when Google Login is enabled");
        }
        if !self.redirect_path.starts_with('/') {
            bail!("GOOGLE_LOGIN_REDIRECT_PATH must be an absolute path");
        }
        if !self.session_cookie_name.starts_with("__Host-") {
            bail!("SESSION_COOKIE_NAME must use a __Host- prefix");
        }
        if self.session_ttl_secs == 0 || self.session_idle_ttl_secs == 0 {
            bail!("session TTLs must be positive");
        }
        if self.session_idle_ttl_secs > self.session_ttl_secs {
            bail!("SESSION_IDLE_TTL_SECS must not exceed SESSION_TTL_SECS");
        }
        Ok(())
    }

    pub fn is_email_allowed(&self, email: &str) -> bool {
        if !self.allowed_emails.is_empty() && self.allowed_domain.is_some() {
            return false;
        }
        let email = email.trim().to_ascii_lowercase();
        if self.allowed_emails.contains(&email) {
            return true;
        }
        let Some(domain) = &self.allowed_domain else {
            return false;
        };
        email
            .rsplit_once('@')
            .map(|(_, actual_domain)| actual_domain.eq_ignore_ascii_case(domain))
            .unwrap_or(false)
    }
}

pub fn google_login_client_secret_from_env() -> Option<String> {
    std::env::var("GOOGLE_LOGIN_CLIENT_SECRET")
        .ok()
        .or_else(|| std::env::var("GOOGLE_CLIENT_SECRET").ok())
        .filter(|value| !value.trim().is_empty())
}

pub fn build_google_login_url(
    config: &WebConsoleLoginConfig,
    public_url: &str,
    return_to: Option<String>,
) -> Result<LoginUrl> {
    config.validate_for_login()?;
    let public_url = public_url.trim().trim_end_matches('/');
    if public_url.is_empty() {
        bail!("AEGIS_PUBLIC_URL is required for Google Login");
    }

    let redirect_uri = format!("{public_url}{}", config.redirect_path);
    let state_raw = random_urlsafe(32)?;
    let nonce = random_urlsafe(32)?;
    let code_verifier = random_urlsafe(64)?;
    let code_challenge = pkce_s256_challenge(&code_verifier);
    let now = now_secs();
    let state = PendingLoginState {
        state_hash: sha256_hex(state_raw.as_bytes()),
        nonce,
        code_verifier,
        code_challenge,
        redirect_uri,
        return_to,
        created_at: now,
        expires_at: now + LOGIN_STATE_TTL_SECS,
    };
    pending_login_store()
        .lock()
        .map_err(|_| anyhow!("pending login store poisoned"))?
        .insert(state.state_hash.clone(), state.clone());

    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &state.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &LOGIN_SCOPES.join(" "))
        .append_pair("state", &state_raw)
        .append_pair("nonce", &state.nonce)
        .append_pair("code_challenge", &state.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("max_age", "0")
        .append_pair(
            "claims",
            &json!({
                "id_token": {
                    "auth_time": { "essential": true }
                }
            })
            .to_string(),
        )
        .append_pair("prompt", "select_account");
    let query = serializer.finish();

    Ok(LoginUrl {
        url: format!("{GOOGLE_AUTH_ENDPOINT}?{query}"),
        state,
    })
}

pub fn sanitize_login_return_to(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.len() > 512 {
        return None;
    }
    if value.contains('\\') || value.chars().any(char::is_control) {
        return None;
    }
    if !value.starts_with("/ui") || value.starts_with("//") {
        return None;
    }
    if value == "/ui" || value.starts_with("/ui/") || value.starts_with("/ui?") {
        Some(value.to_string())
    } else {
        None
    }
}

pub fn consume_pending_login_state(state_raw: &str) -> Result<PendingLoginState> {
    if state_raw.trim().is_empty() {
        bail!("missing state");
    }
    let state_hash = sha256_hex(state_raw.as_bytes());
    let mut store = pending_login_store()
        .lock()
        .map_err(|_| anyhow!("pending login store poisoned"))?;
    let state = store
        .remove(&state_hash)
        .ok_or_else(|| anyhow!("invalid or expired state"))?;
    if state.expires_at < now_secs() {
        bail!("expired state");
    }
    Ok(state)
}

pub fn has_pending_login_state(state_raw: &str) -> bool {
    if state_raw.trim().is_empty() {
        return false;
    }
    let state_hash = sha256_hex(state_raw.as_bytes());
    pending_login_store()
        .lock()
        .map(|store| store.contains_key(&state_hash))
        .unwrap_or(false)
}

pub fn build_google_login_token_exchange_form(
    config: &WebConsoleLoginConfig,
    client_secret: &str,
    pending: &PendingLoginState,
    code: &str,
) -> Result<Vec<(&'static str, String)>> {
    config.validate_for_login()?;
    if client_secret.trim().is_empty() {
        bail!("GOOGLE_LOGIN_CLIENT_SECRET is required");
    }
    let code = code.trim();
    if code.is_empty() {
        bail!("authorization code is required");
    }
    Ok(vec![
        ("client_id", config.client_id.clone()),
        ("client_secret", client_secret.to_string()),
        ("code", code.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("code_verifier", pending.code_verifier.clone()),
    ])
}

pub async fn exchange_google_login_code(
    http: &reqwest::Client,
    config: &WebConsoleLoginConfig,
    client_secret: &str,
    pending: &PendingLoginState,
    code: &str,
) -> Result<GoogleLoginTokenResponse> {
    let form = build_google_login_token_exchange_form(config, client_secret, pending, code)?;
    let resp = http
        .post(GOOGLE_TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context("exchanging Google Login authorization code")?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .context("reading Google Login token response")?;
    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<GoogleTokenErrorResponse>(&body) {
            bail!(
                "Google Login token exchange failed: {} - {}",
                err.error,
                err.error_description.unwrap_or_default()
            );
        }
        bail!("Google Login token exchange failed with HTTP {status}");
    }

    let token_response: GoogleLoginTokenResponse =
        serde_json::from_str(&body).context("parsing Google Login token response")?;
    if token_response.id_token.trim().is_empty() {
        bail!("Google Login token response did not include an id_token");
    }
    if token_response.refresh_token.is_some() {
        bail!("Google Login token response unexpectedly included a refresh_token");
    }
    if let Some(scope) = &token_response.scope {
        reject_non_login_scopes(scope)?;
    }
    Ok(token_response)
}

pub async fn fetch_google_oidc_jwks(http: &reqwest::Client) -> Result<GoogleJwks> {
    let resp = http
        .get(GOOGLE_JWKS_ENDPOINT)
        .send()
        .await
        .context("fetching Google OIDC JWKS")?;
    let status = resp.status();
    if !status.is_success() {
        bail!("Google OIDC JWKS endpoint returned HTTP {status}");
    }
    let jwks: GoogleJwks = resp.json().await.context("parsing Google OIDC JWKS")?;
    if jwks.keys.is_empty() {
        bail!("Google OIDC JWKS returned zero keys");
    }
    Ok(jwks)
}

pub fn verify_google_id_token(
    id_token: &str,
    jwks: &GoogleJwks,
    config: &WebConsoleLoginConfig,
    expected_nonce: &str,
) -> Result<GoogleIdClaims> {
    let header = decode_header(id_token).context("decoding Google ID token header")?;
    if !matches!(header.alg, Algorithm::RS256) {
        bail!(
            "Google ID token uses unsupported algorithm {:?}",
            header.alg
        );
    }
    let kid = header
        .kid
        .as_deref()
        .ok_or_else(|| anyhow!("Google ID token header missing kid"))?;
    let jwk = jwks
        .find(kid)
        .ok_or_else(|| anyhow!("Google ID token kid not found in JWKS"))?;
    let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .context("building Google ID token decoding key")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(GOOGLE_OIDC_ISSUERS);
    validation.set_audience(&[config.client_id.as_str()]);
    validation.validate_exp = true;

    let data = decode::<GoogleIdClaims>(id_token, &key, &validation)
        .context("verifying Google ID token signature and standard claims")?;
    validate_google_id_claims(&data.claims, config, expected_nonce)?;
    Ok(data.claims)
}

pub fn validate_google_id_claims(
    claims: &GoogleIdClaims,
    config: &WebConsoleLoginConfig,
    expected_nonce: &str,
) -> Result<()> {
    if !GOOGLE_OIDC_ISSUERS.contains(&claims.iss.as_str()) {
        bail!("Google ID token issuer is invalid");
    }
    if claims.aud != config.client_id {
        bail!("Google ID token audience is invalid");
    }
    if claims.sub.trim().is_empty() {
        bail!("Google ID token subject is missing");
    }
    if claims.nonce.as_deref() != Some(expected_nonce) {
        bail!("Google ID token nonce is invalid");
    }
    if claims.email.trim().is_empty() {
        bail!("Google ID token email is missing");
    }
    if !claims.email_verified {
        bail!("Google ID token email is not verified");
    }
    if !config.is_email_allowed(&claims.email) {
        bail!("Google ID token email is not allowed");
    }
    Ok(())
}

pub fn init_session_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS web_console_sessions (
          session_id_hash TEXT PRIMARY KEY,
          google_sub TEXT NOT NULL,
          email TEXT NOT NULL,
          email_verified INTEGER NOT NULL,
          created_at INTEGER NOT NULL,
          expires_at INTEGER NOT NULL,
          idle_expires_at INTEGER NOT NULL,
          last_seen_at INTEGER,
          revoked_at INTEGER,
          user_agent_hash TEXT,
          ip_prefix_hash TEXT,
          mfa_level TEXT NOT NULL DEFAULT 'none',
          mfa_verified_at INTEGER,
          mfa_method TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_web_console_sessions_google_sub
          ON web_console_sessions(google_sub);

        CREATE INDEX IF NOT EXISTS idx_web_console_sessions_expires
          ON web_console_sessions(expires_at, idle_expires_at, revoked_at);
        "#,
    )
}

pub fn init_mfa_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS webauthn_credentials (
          id TEXT PRIMARY KEY,
          google_sub TEXT NOT NULL,
          credential_id BLOB NOT NULL UNIQUE,
          public_key_cose BLOB NOT NULL,
          sign_count INTEGER,
          transports_json TEXT,
          backup_eligible INTEGER,
          backup_state INTEGER,
          created_at INTEGER NOT NULL,
          last_used_at INTEGER,
          nickname TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_google_sub
          ON webauthn_credentials(google_sub);

        CREATE TABLE IF NOT EXISTS webauthn_challenges (
          challenge_id TEXT PRIMARY KEY,
          google_sub TEXT NOT NULL,
          purpose TEXT NOT NULL,
          challenge_state_json TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          expires_at INTEGER NOT NULL,
          consumed_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS totp_secrets (
          google_sub TEXT PRIMARY KEY,
          secret_encrypted BLOB NOT NULL,
          created_at INTEGER NOT NULL,
          verified_at INTEGER,
          last_used_counter INTEGER
        );

        CREATE TABLE IF NOT EXISTS recovery_codes (
          google_sub TEXT NOT NULL,
          code_hash TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          used_at INTEGER,
          PRIMARY KEY (google_sub, code_hash)
        );
        "#,
    )
}

pub fn create_session(
    conn: &Connection,
    config: &WebConsoleLoginConfig,
    google_sub: &str,
    email: &str,
    email_verified: bool,
    now: u64,
) -> Result<CreatedSession> {
    create_session_with_mfa_evidence(conn, config, google_sub, email, email_verified, now, None)
}

pub fn create_session_with_mfa_evidence(
    conn: &Connection,
    config: &WebConsoleLoginConfig,
    google_sub: &str,
    email: &str,
    email_verified: bool,
    now: u64,
    mfa_evidence: Option<SessionMfaEvidence>,
) -> Result<CreatedSession> {
    if google_sub.trim().is_empty() {
        bail!("google_sub is required");
    }
    if !email_verified {
        bail!("email_verified must be true to create a Web Console session");
    }
    if !config.is_email_allowed(email) {
        bail!("email is not allowed for Web Console login");
    }
    insert_session(
        conn,
        config,
        google_sub,
        email,
        email_verified,
        now,
        mfa_evidence,
    )
}

/// Session for the owner who followed the startup-printed local login link
/// (`GET /auth/login/local?token=…`).
///
/// This is the ONLY path that skips the `GOOGLE_LOGIN_ALLOWED_*` allowlist,
/// and it does so on purpose: the allowlist exists to decide which Google
/// identity may own the console, but a local session has no Google identity
/// to check. Its authorization step is the token gate in front of the route —
/// the token is either the operator's own `AEGIS_LOCAL_LOGIN_TOKEN` or a
/// random value printed once to the operator's stderr — so a caller that
/// reaches this function has already proven control of the host. Keep it
/// reachable from that handler alone; do not route other logins through it.
pub fn create_local_session(
    conn: &Connection,
    config: &WebConsoleLoginConfig,
    now: u64,
) -> Result<CreatedSession> {
    insert_session(
        conn,
        config,
        LOCAL_LOGIN_SUB,
        LOCAL_LOGIN_EMAIL,
        true,
        now,
        None,
    )
}

/// Called only after a verified Passkey assertion for the host-paired owner.
pub(crate) fn create_passkey_session(conn: &Connection, now: u64) -> Result<CreatedSession> {
    insert_session(
        conn,
        &WebConsoleLoginConfig::from_env(),
        LOCAL_LOGIN_SUB,
        LOCAL_LOGIN_EMAIL,
        true,
        now,
        Some(SessionMfaEvidence {
            level: MfaLevel::WebAuthn,
            verified_at: now,
            method: "passkey".into(),
        }),
    )
}

/// Effective local login token for this process, derived from the
/// environment (see [`local_login_token_from_lookup`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalLoginToken {
    pub token: String,
    /// `true` when nothing configured the token and startup minted a random
    /// one; `false` for an explicit `AEGIS_LOCAL_LOGIN_TOKEN`.
    pub generated: bool,
}

/// Decide whether the local login link is enabled and with which token.
///
/// * `AEGIS_LOCAL_LOGIN_TOKEN=off` → disabled.
/// * `AEGIS_LOCAL_LOGIN_TOKEN=<non-empty>` → that fixed token.
/// * unset/empty → a random token from `generate` **only** when no other
///   console principal is configured (Google Login not enabled and no
///   Telegram bot token); otherwise disabled, so an installed deployment does
///   not silently grow a second login path.
///
/// `generate` is injected so the policy can be unit-tested without an RNG
/// and so the caller can decide how to react to RNG failure.
pub fn local_login_token_from_lookup(
    mut lookup: impl FnMut(&str) -> Option<String>,
    generate: impl FnOnce() -> Result<String>,
) -> Result<Option<LocalLoginToken>> {
    match lookup("AEGIS_LOCAL_LOGIN_TOKEN").map(|value| value.trim().to_string()) {
        Some(value) if value.eq_ignore_ascii_case("off") => return Ok(None),
        Some(value) if !value.is_empty() => {
            return Ok(Some(LocalLoginToken {
                token: value,
                generated: false,
            }));
        }
        _ => {}
    }
    let google_login_enabled = parse_bool(&lookup("GOOGLE_LOGIN_ENABLED")).unwrap_or(false);
    let telegram_configured =
        lookup("TELOXIDE_TOKEN").is_some_and(|value| !value.trim().is_empty());
    if google_login_enabled || telegram_configured {
        return Ok(None);
    }
    Ok(Some(LocalLoginToken {
        token: generate().context("generating local login token")?,
        generated: true,
    }))
}

/// 32 random bytes, hex-encoded (64 chars), for the generated local login
/// token.
pub fn generate_local_login_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).context("OS RNG failed")?;
    Ok(hex::encode(bytes))
}

/// Persist a session row. Callers own the identity checks: the Google path
/// verifies the e-mail allowlist first, the local path is gated by the local
/// login token (see [`create_local_session`]).
fn insert_session(
    conn: &Connection,
    config: &WebConsoleLoginConfig,
    google_sub: &str,
    email: &str,
    email_verified: bool,
    now: u64,
    mfa_evidence: Option<SessionMfaEvidence>,
) -> Result<CreatedSession> {
    init_session_schema(conn).context("initializing web_console_sessions schema")?;
    crate::data_access_gate::init_sensitive_read_schema(conn)
        .context("initializing sensitive read access schema")?;
    let cookie_token = random_urlsafe(32)?;
    let session_id_hash = sha256_hex(cookie_token.as_bytes());
    let email = email.trim().to_ascii_lowercase();
    let mfa_level = mfa_evidence
        .as_ref()
        .map(|evidence| evidence.level)
        .unwrap_or(MfaLevel::GoogleOnly);
    let mfa_verified_at = mfa_evidence.as_ref().map(|evidence| evidence.verified_at);
    let mfa_method = mfa_evidence.map(|evidence| evidence.method);
    let session = WebConsoleSession {
        session_id_hash,
        google_sub: google_sub.to_string(),
        email,
        email_verified,
        created_at: now,
        expires_at: now + config.session_ttl_secs,
        idle_expires_at: now + config.session_idle_ttl_secs,
        last_seen_at: None,
        revoked_at: None,
        mfa_level,
        mfa_verified_at,
        mfa_method,
    };

    conn.execute(
        r#"
        INSERT INTO web_console_sessions (
          session_id_hash, google_sub, email, email_verified, created_at,
          expires_at, idle_expires_at, last_seen_at, revoked_at,
          mfa_level, mfa_verified_at, mfa_method
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?10)
        "#,
        params![
            &session.session_id_hash,
            &session.google_sub,
            &session.email,
            i64::from(session.email_verified),
            session.created_at as i64,
            session.expires_at as i64,
            session.idle_expires_at as i64,
            session.mfa_level.as_str(),
            session.mfa_verified_at.map(|value| value as i64),
            session.mfa_method.as_deref(),
        ],
    )
    .context("inserting web_console_session")?;

    Ok(CreatedSession {
        cookie_token,
        session,
    })
}

pub fn google_fresh_auth_mfa_evidence(
    claims: &GoogleIdClaims,
    now: u64,
    freshness_secs: u64,
) -> Option<SessionMfaEvidence> {
    let auth_time = fresh_claim_timestamp(claims.auth_time, now, freshness_secs)?;
    Some(SessionMfaEvidence {
        level: MfaLevel::GoogleFresh,
        verified_at: auth_time,
        method: "google_auth_time".to_string(),
    })
}

pub fn google_login_mfa_evidence(
    claims: &GoogleIdClaims,
    now: u64,
    freshness_secs: u64,
) -> Option<SessionMfaEvidence> {
    if let Some(evidence) = google_fresh_auth_mfa_evidence(claims, now, freshness_secs) {
        return Some(evidence);
    }

    // Google only returns auth_time when the OAuth client has that optional
    // claim enabled. A freshly issued ID token is still tied to this exact
    // callback by state, nonce, PKCE, and one-time code exchange, so it is a
    // reasonable S3 short-window signal. S4 remains WebAuthn-only elsewhere.
    let issued_at = fresh_claim_timestamp(claims.iat, now, freshness_secs)?;
    Some(SessionMfaEvidence {
        level: MfaLevel::GoogleFresh,
        verified_at: issued_at,
        method: "google_login_callback".to_string(),
    })
}

fn fresh_claim_timestamp(value: Option<i64>, now: u64, freshness_secs: u64) -> Option<u64> {
    let value = value?;
    if value < 0 {
        return None;
    }
    let value = value as u64;
    if value > now || now.saturating_sub(value) > freshness_secs {
        return None;
    }
    Some(value)
}

pub fn validate_session(
    conn: &Connection,
    cookie_token: &str,
    now: u64,
    session_idle_ttl_secs: u64,
) -> Result<Option<WebConsoleSession>> {
    if cookie_token.trim().is_empty() {
        return Ok(None);
    }
    let session_id_hash = sha256_hex(cookie_token.as_bytes());
    let session = conn
        .query_row(
            r#"
            SELECT session_id_hash, google_sub, email, email_verified, created_at,
                   expires_at, idle_expires_at, last_seen_at, revoked_at,
                   mfa_level, mfa_verified_at, mfa_method
            FROM web_console_sessions
            WHERE session_id_hash = ?1
            "#,
            params![session_id_hash],
            row_to_session,
        )
        .optional()
        .context("querying web_console_session")?;

    let Some(session) = session else {
        return Ok(None);
    };
    if session.revoked_at.is_some() || session.expires_at <= now || session.idle_expires_at <= now {
        return Ok(None);
    }
    let next_idle_expires_at = session
        .expires_at
        .min(now.saturating_add(session_idle_ttl_secs));
    conn.execute(
        "UPDATE web_console_sessions SET last_seen_at = ?1, idle_expires_at = ?2 WHERE session_id_hash = ?3",
        params![
            now as i64,
            next_idle_expires_at as i64,
            session.session_id_hash
        ],
    )
    .context("updating web_console_session last_seen_at and idle expiry")?;
    Ok(Some(WebConsoleSession {
        last_seen_at: Some(now),
        idle_expires_at: next_idle_expires_at,
        ..session
    }))
}

pub fn revoke_session(conn: &Connection, cookie_token: &str, now: u64) -> Result<bool> {
    if cookie_token.trim().is_empty() {
        return Ok(false);
    }
    let session_id_hash = sha256_hex(cookie_token.as_bytes());
    let changed = conn
        .execute(
            r#"
            UPDATE web_console_sessions
            SET revoked_at = ?1
            WHERE session_id_hash = ?2 AND revoked_at IS NULL
            "#,
            params![now as i64, session_id_hash],
        )
        .context("revoking web_console_session")?;
    Ok(changed > 0)
}

pub fn get_mfa_status(conn: &Connection, session: &WebConsoleSession) -> Result<MfaStatus> {
    init_mfa_schema(conn).context("initializing Web Console MFA schema")?;
    let webauthn_registered_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM webauthn_credentials WHERE google_sub = ?1",
            params![&session.google_sub],
            |row| row.get::<_, i64>(0),
        )
        .context("counting WebAuthn credentials")? as u64;
    let totp_configured = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM totp_secrets WHERE google_sub = ?1 AND verified_at IS NOT NULL)",
            params![&session.google_sub],
            |row| row.get::<_, i64>(0),
        )
        .context("checking TOTP status")?
        != 0;
    let recovery_codes_remaining: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recovery_codes WHERE google_sub = ?1 AND used_at IS NULL",
            params![&session.google_sub],
            |row| row.get::<_, i64>(0),
        )
        .context("counting recovery codes")? as u64;

    let webauthn_available = false;
    let webauthn_status = "deferred".to_string();
    let usable_webauthn_configured = webauthn_available && webauthn_registered_count > 0;
    let current_session_has_step_up = matches!(
        session.mfa_level,
        MfaLevel::GoogleFresh | MfaLevel::Totp | MfaLevel::WebAuthn
    ) && session.mfa_verified_at.is_some();

    Ok(MfaStatus {
        mfa_level: session.mfa_level.as_str().to_string(),
        mfa_verified_at: session.mfa_verified_at,
        requires_mfa_setup: !current_session_has_step_up
            && !totp_configured
            && !usable_webauthn_configured,
        webauthn_registered_count,
        webauthn_available,
        webauthn_status,
        totp_configured,
        preferred_step_up_method: "totp".to_string(),
        recovery_codes_remaining,
    })
}

pub fn list_webauthn_credentials(
    conn: &Connection,
    google_sub: &str,
) -> Result<Vec<WebAuthnCredentialSummary>> {
    init_mfa_schema(conn).context("initializing Web Console MFA schema")?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, transports_json, backup_eligible, backup_state, created_at,
                   last_used_at, nickname
            FROM webauthn_credentials
            WHERE google_sub = ?1
            ORDER BY created_at DESC, id ASC
            "#,
        )
        .context("preparing WebAuthn credential listing")?;
    let rows = stmt
        .query_map(params![google_sub], |row| {
            let transports_json: Option<String> = row.get(1)?;
            let backup_eligible: Option<i64> = row.get(2)?;
            let backup_state: Option<i64> = row.get(3)?;
            Ok(WebAuthnCredentialSummary {
                id: row.get(0)?,
                nickname: row.get(6)?,
                transports: parse_transport_list(transports_json.as_deref()),
                backup_eligible: backup_eligible.map(|value| value != 0),
                backup_state: backup_state.map(|value| value != 0),
                created_at: row.get::<_, i64>(4)? as u64,
                last_used_at: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
            })
        })
        .context("querying WebAuthn credentials")?;

    let mut credentials = Vec::new();
    for row in rows {
        credentials.push(row.context("reading WebAuthn credential row")?);
    }
    Ok(credentials)
}

pub fn start_totp_setup(
    conn: &Connection,
    aes_key: &[u8; 32],
    session: &WebConsoleSession,
    issuer: &str,
    now: u64,
) -> Result<TotpSetup> {
    init_mfa_schema(conn).context("initializing Web Console MFA schema")?;
    let mut secret = [0u8; TOTP_SECRET_BYTES];
    getrandom::fill(&mut secret).context("OS RNG failed for TOTP secret")?;
    let encrypted =
        crate::credential_store::encrypt(aes_key, &secret).context("encrypting TOTP secret")?;
    conn.execute(
        r#"
        INSERT INTO totp_secrets (
          google_sub, secret_encrypted, created_at, verified_at, last_used_counter
        )
        VALUES (?1, ?2, ?3, NULL, NULL)
        ON CONFLICT(google_sub) DO UPDATE SET
          secret_encrypted = excluded.secret_encrypted,
          created_at = excluded.created_at,
          verified_at = NULL,
          last_used_counter = NULL
        "#,
        params![session.google_sub, encrypted, now as i64],
    )
    .context("storing pending TOTP secret")?;

    let secret_base32 = base32_encode_no_padding(&secret);
    let issuer = non_empty_or(issuer.trim(), "Aegis Runtime");
    let account_name = session.email.clone();
    let otpauth_url = format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA256&digits={}&period={}",
        urlencoding::encode(issuer),
        urlencoding::encode(&account_name),
        secret_base32,
        urlencoding::encode(issuer),
        TOTP_DIGITS,
        TOTP_STEP_SECS
    );

    Ok(TotpSetup {
        secret_base32,
        otpauth_url,
        issuer: issuer.to_string(),
        account_name,
        digits: TOTP_DIGITS,
        period_secs: TOTP_STEP_SECS,
        algorithm: "SHA256".into(),
    })
}

pub fn finish_totp_setup(
    conn: &Connection,
    aes_key: &[u8; 32],
    session: &WebConsoleSession,
    code: &str,
    now: u64,
) -> Result<bool> {
    init_mfa_schema(conn).context("initializing Web Console MFA schema")?;
    let Some((secret, last_used_counter)) = load_totp_secret(conn, aes_key, &session.google_sub)?
    else {
        return Ok(false);
    };
    let Some(counter) = verify_totp_code_for_secret(&secret, code, now, last_used_counter) else {
        return Ok(false);
    };
    conn.execute(
        r#"
        UPDATE totp_secrets
        SET verified_at = ?2,
            last_used_counter = ?3
        WHERE google_sub = ?1
        "#,
        params![session.google_sub, now as i64, counter as i64],
    )
    .context("marking TOTP secret verified")?;
    update_session_mfa(conn, session, MfaLevel::Totp, now, "totp")?;
    Ok(true)
}

/// Failed step-up attempts per account: after `TOTP_MAX_FAILURES` inside the
/// window the account is locked for `TOTP_LOCKOUT_SECS` (a six-digit code
/// must not be guessable at request rate from a stolen session cookie).
static TOTP_FAILURES: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, (u32, u64)>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));
const TOTP_MAX_FAILURES: u32 = 5;
const TOTP_LOCKOUT_SECS: u64 = 15 * 60;

/// `true` when the account is currently locked out.
pub fn totp_step_up_locked(google_sub: &str, now: u64) -> bool {
    let map = TOTP_FAILURES.lock();
    map.get(google_sub)
        .is_some_and(|(fails, locked_until)| *fails >= TOTP_MAX_FAILURES && *locked_until > now)
}

fn totp_step_up_record(google_sub: &str, ok: bool, now: u64) {
    let mut map = TOTP_FAILURES.lock();
    if ok {
        map.remove(google_sub);
        return;
    }
    let entry = map.entry(google_sub.to_string()).or_insert((0, 0));
    if entry.0 >= TOTP_MAX_FAILURES && entry.1 <= now {
        // a previous lockout expired: start a fresh count
        entry.0 = 0;
    }
    entry.0 += 1;
    if entry.0 >= TOTP_MAX_FAILURES {
        entry.1 = now + TOTP_LOCKOUT_SECS;
    }
}

pub fn verify_totp_step_up(
    conn: &Connection,
    aes_key: &[u8; 32],
    session: &WebConsoleSession,
    code: &str,
    now: u64,
) -> Result<bool> {
    if totp_step_up_locked(&session.google_sub, now) {
        tracing::warn!("[WebConsole] TOTP step-up refused: account locked after repeated failures");
        return Ok(false);
    }
    let ok = verify_totp_step_up_unthrottled(conn, aes_key, session, code, now)?;
    totp_step_up_record(&session.google_sub, ok, now);
    Ok(ok)
}

fn verify_totp_step_up_unthrottled(
    conn: &Connection,
    aes_key: &[u8; 32],
    session: &WebConsoleSession,
    code: &str,
    now: u64,
) -> Result<bool> {
    init_mfa_schema(conn).context("initializing Web Console MFA schema")?;
    let verified_at: Option<i64> = conn
        .query_row(
            "SELECT verified_at FROM totp_secrets WHERE google_sub = ?1",
            params![session.google_sub],
            |row| row.get(0),
        )
        .optional()
        .context("querying TOTP verification state")?
        .flatten();
    if verified_at.is_none() {
        return Ok(false);
    }
    let Some((secret, last_used_counter)) = load_totp_secret(conn, aes_key, &session.google_sub)?
    else {
        return Ok(false);
    };
    let Some(counter) = verify_totp_code_for_secret(&secret, code, now, last_used_counter) else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE totp_secrets SET last_used_counter = ?2 WHERE google_sub = ?1",
        params![session.google_sub, counter as i64],
    )
    .context("updating TOTP last used counter")?;
    update_session_mfa(conn, session, MfaLevel::Totp, now, "totp")?;
    Ok(true)
}

pub fn confirm_local_host_step_up(
    conn: &Connection,
    session: &WebConsoleSession,
    now: u64,
) -> Result<()> {
    if session.google_sub != LOCAL_LOGIN_SUB {
        bail!("host confirmation is only available for the local host session");
    }
    update_session_mfa(
        conn,
        session,
        MfaLevel::HostConfirmed,
        now,
        "local_host_confirm",
    )
}

fn update_session_mfa(
    conn: &Connection,
    session: &WebConsoleSession,
    level: MfaLevel,
    now: u64,
    method: &str,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE web_console_sessions
        SET mfa_level = ?2,
            mfa_verified_at = ?3,
            mfa_method = ?4
        WHERE session_id_hash = ?1
        "#,
        params![session.session_id_hash, level.as_str(), now as i64, method],
    )
    .context("updating Web Console session MFA state")?;
    Ok(())
}

fn load_totp_secret(
    conn: &Connection,
    aes_key: &[u8; 32],
    google_sub: &str,
) -> Result<Option<(Vec<u8>, Option<u64>)>> {
    let row: Option<(Vec<u8>, Option<i64>)> = conn
        .query_row(
            "SELECT secret_encrypted, last_used_counter FROM totp_secrets WHERE google_sub = ?1",
            params![google_sub],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("querying TOTP secret")?;
    let Some((encrypted, last_used_counter)) = row else {
        return Ok(None);
    };
    let secret =
        crate::credential_store::decrypt(aes_key, &encrypted).context("decrypting TOTP secret")?;
    Ok(Some((
        secret,
        last_used_counter.and_then(|value| u64::try_from(value).ok()),
    )))
}

fn verify_totp_code_for_secret(
    secret: &[u8],
    code: &str,
    now: u64,
    last_used_counter: Option<u64>,
) -> Option<u64> {
    let normalized = code.trim();
    if normalized.len() != TOTP_DIGITS as usize
        || !normalized.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let current_counter = now / TOTP_STEP_SECS;
    for offset in [-1_i64, 0, 1] {
        let counter = if offset < 0 {
            current_counter.checked_sub(offset.unsigned_abs())?
        } else {
            current_counter.saturating_add(offset as u64)
        };
        if last_used_counter.is_some_and(|last| counter <= last) {
            continue;
        }
        if totp_code(secret, counter) == normalized {
            return Some(counter);
        }
    }
    None
}

pub fn totp_code(secret: &[u8], counter: u64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let modulo = 10_u32.pow(TOTP_DIGITS);
    format!("{:0width$}", binary % modulo, width = TOTP_DIGITS as usize)
}

pub fn totp_code_from_base32(secret_base32: &str, counter: u64) -> Result<String> {
    let secret = base32_decode_no_padding(secret_base32)?;
    Ok(totp_code(&secret, counter))
}

fn base32_encode_no_padding(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer: u16 = 0;
    let mut bits_left = 0_u8;
    for &byte in bytes {
        buffer = (buffer << 8) | u16::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            output.push(char::from(ALPHABET[index]));
            bits_left -= 5;
        }
    }
    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        output.push(char::from(ALPHABET[index]));
    }
    output
}

fn base32_decode_no_padding(value: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits_left = 0_u8;
    for byte in value
        .bytes()
        .filter(|byte| *byte != b'=' && !byte.is_ascii_whitespace())
    {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => bail!("invalid base32 character"),
        };
        buffer = (buffer << 5) | u32::from(val);
        bits_left += 5;
        if bits_left >= 8 {
            output.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }
    Ok(output)
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

pub fn build_session_cookie_header(
    config: &WebConsoleLoginConfig,
    cookie_token: &str,
) -> Result<String> {
    validate_cookie_name(&config.session_cookie_name)?;
    if !is_cookie_token_safe(cookie_token) {
        bail!("cookie token contains invalid characters");
    }
    Ok(format!(
        "{}={}; Path=/; Max-Age={}; Secure; HttpOnly; SameSite=Lax",
        config.session_cookie_name, cookie_token, config.session_ttl_secs
    ))
}

pub fn build_clear_session_cookie_header(config: &WebConsoleLoginConfig) -> Result<String> {
    validate_cookie_name(&config.session_cookie_name)?;
    Ok(format!(
        "{}=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Lax",
        config.session_cookie_name
    ))
}

pub fn csrf_token_for_session(session: &WebConsoleSession) -> String {
    let material = format!(
        "web-console-csrf:{}:{}:{}",
        session.session_id_hash, session.google_sub, session.created_at
    );
    sha256_hex(material.as_bytes())
}

pub fn extract_cookie_value(cookie_header: &str, cookie_name: &str) -> Option<String> {
    if validate_cookie_name(cookie_name).is_err() {
        return None;
    }
    cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if name == cookie_name && is_cookie_token_safe(value) {
            Some(value.to_string())
        } else {
            None
        }
    })
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<WebConsoleSession> {
    let email_verified: i64 = row.get(3)?;
    let mfa_level: String = row.get(9)?;
    Ok(WebConsoleSession {
        session_id_hash: row.get(0)?,
        google_sub: row.get(1)?,
        email: row.get(2)?,
        email_verified: email_verified != 0,
        created_at: row.get::<_, i64>(4)? as u64,
        expires_at: row.get::<_, i64>(5)? as u64,
        idle_expires_at: row.get::<_, i64>(6)? as u64,
        last_seen_at: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        revoked_at: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
        mfa_level: MfaLevel::parse(&mfa_level),
        mfa_verified_at: row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
        mfa_method: row.get(11)?,
    })
}

fn validate_cookie_name(name: &str) -> Result<()> {
    if !name.starts_with("__Host-") {
        bail!("SESSION_COOKIE_NAME must use a __Host- prefix");
    }
    if name
        .bytes()
        .any(|byte| !matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        bail!("SESSION_COOKIE_NAME contains invalid characters");
    }
    Ok(())
}

fn is_cookie_token_safe(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
}

fn pending_login_store() -> &'static Mutex<HashMap<String, PendingLoginState>> {
    static STORE: OnceLock<Mutex<HashMap<String, PendingLoginState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_bool(value: &Option<String>) -> Option<bool> {
    value.as_deref().and_then(|value| {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

fn parse_u64(value: &Option<String>) -> Option<u64> {
    value.as_deref()?.trim().parse().ok()
}

fn parse_allowed_emails(value: &str) -> HashSet<String> {
    value
        .split([',', ';', '\n'])
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect()
}

fn reject_non_login_scopes(scope: &str) -> Result<()> {
    let allowed: HashSet<&str> = LOGIN_SCOPES
        .into_iter()
        .chain([
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
        ])
        .collect();
    for item in scope.split_whitespace() {
        if !allowed.contains(item) {
            bail!("Google Login token response included non-login scope `{item}`");
        }
    }
    Ok(())
}

fn parse_transport_list(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(value).unwrap_or_default()
}

fn random_urlsafe(byte_len: usize) -> Result<String> {
    let mut bytes = vec![0u8; byte_len];
    getrandom::fill(&mut bytes).context("OS RNG failed")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn pkce_s256_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_step_up_locks_after_repeated_failures() {
        let sub = format!("test-sub-{}", std::process::id());
        let now = 1_800_000_000u64;
        for _ in 0..4 {
            totp_step_up_record(&sub, false, now);
            assert!(!totp_step_up_locked(&sub, now));
        }
        totp_step_up_record(&sub, false, now);
        assert!(totp_step_up_locked(&sub, now), "fifth failure locks");
        assert!(totp_step_up_locked(&sub, now + TOTP_LOCKOUT_SECS - 1));
        assert!(
            !totp_step_up_locked(&sub, now + TOTP_LOCKOUT_SECS + 1),
            "lockout expires"
        );
        // After expiry the counter restarts; a success clears everything.
        totp_step_up_record(&sub, false, now + TOTP_LOCKOUT_SECS + 1);
        assert!(!totp_step_up_locked(&sub, now + TOTP_LOCKOUT_SECS + 1));
        totp_step_up_record(&sub, true, now + TOTP_LOCKOUT_SECS + 2);
        assert!(!totp_step_up_locked(&sub, now + TOTP_LOCKOUT_SECS + 2));
    }

    fn test_config() -> WebConsoleLoginConfig {
        WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_LOGIN_CLIENT_ID" => Some("client-123.apps.googleusercontent.com".into()),
            "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret-present".into()),
            "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com, owner@example.com".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret-present".into()),
            _ => None,
        })
    }

    #[test]
    fn google_login_disabled_by_default() {
        let config = WebConsoleLoginConfig::from_lookup(|_| None);
        assert!(!config.enabled);
        assert!(config.validate_for_login().is_err());
        let credentials_only = WebConsoleLoginConfig::from_lookup(|name| match name {
            "GOOGLE_CLIENT_ID" => Some("existing-workspace-client".into()),
            "GOOGLE_CLIENT_SECRET" => Some("existing-workspace-secret".into()),
            _ => None,
        });
        assert!(!credentials_only.enabled);
        assert!(credentials_only.validate_for_login().is_err());
    }

    #[test]
    fn google_login_defaults_to_trusted_browser_session_window() {
        let config = test_config();
        assert_eq!(config.session_ttl_secs, 30 * 24 * 60 * 60);
        assert_eq!(config.session_idle_ttl_secs, 30 * 24 * 60 * 60);
    }

    #[test]
    fn google_login_config_normalizes_allowed_emails() {
        let config = test_config();
        assert!(config.is_email_allowed("l@example.com"));
        assert!(config.is_email_allowed("OWNER@example.com"));
        assert!(!config.is_email_allowed("other@example.com"));
    }

    #[test]
    fn google_login_config_can_allow_domain() {
        let config = WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_LOGIN_CLIENT_ID" => Some("client".into()),
            "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret".into()),
            "GOOGLE_LOGIN_ALLOWED_DOMAIN" => Some("@example.org".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
            _ => None,
        });
        assert!(config.is_email_allowed("owner@example.org"));
        assert!(!config.is_email_allowed("owner@example.com"));
    }

    #[test]
    fn google_login_config_can_reuse_existing_google_client() {
        let config = WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_CLIENT_ID" => Some("shared-client.apps.googleusercontent.com".into()),
            "GOOGLE_CLIENT_SECRET" => Some("shared-secret".into()),
            "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("owner@example.org".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
            _ => None,
        });

        assert_eq!(config.client_id, "shared-client.apps.googleusercontent.com");
        assert!(config.client_secret_present);
        assert!(config.validate_for_login().is_ok());
    }

    #[test]
    fn google_login_config_rejects_mixed_email_and_domain_allowlists() {
        let config = WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_LOGIN_CLIENT_ID" => Some("client".into()),
            "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret".into()),
            "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("owner@example.org".into()),
            "GOOGLE_LOGIN_ALLOWED_DOMAIN" => Some("example.org".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
            _ => None,
        });
        let err = config.validate_for_login().unwrap_err().to_string();
        assert!(err.contains("mutually exclusive"));
        assert!(!config.is_email_allowed("owner@example.org"));
    }

    #[test]
    fn google_login_config_requires_allowlist_when_enabled() {
        let config = WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_LOGIN_CLIENT_ID" => Some("client".into()),
            "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
            _ => None,
        });
        let err = config.validate_for_login().unwrap_err().to_string();
        assert!(err.contains("GOOGLE_LOGIN_ALLOWED_EMAILS"));
    }

    #[test]
    fn google_login_url_uses_login_only_scopes_and_pkce() {
        let login = build_google_login_url(&test_config(), "https://aegis.example", None)
            .expect("login url");
        let parsed = url::Url::parse(&login.url).unwrap();
        assert_eq!(
            parsed.as_str().split('?').next().unwrap(),
            GOOGLE_AUTH_ENDPOINT
        );
        let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("openid email profile")
        );
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(params.get("max_age").map(String::as_str), Some("0"));
        let claims: serde_json::Value =
            serde_json::from_str(params.get("claims").expect("claims param")).unwrap();
        assert_eq!(
            claims["id_token"]["auth_time"]["essential"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("https://aegis.example/auth/callback/google")
        );
        assert_eq!(login.state.code_challenge, params["code_challenge"]);
        assert_eq!(login.state.redirect_uri, params["redirect_uri"]);
    }

    #[test]
    fn google_login_state_preserves_only_safe_ui_return_paths() {
        let login = build_google_login_url(
            &test_config(),
            "https://aegis.example",
            sanitize_login_return_to(Some("/ui/?tab=mail&item=email%3A42")),
        )
        .expect("login url");
        assert_eq!(
            login.state.return_to.as_deref(),
            Some("/ui/?tab=mail&item=email%3A42")
        );

        for unsafe_value in [
            "https://evil.example/ui/",
            "//evil.example/ui/",
            "/admin",
            "/ui-admin",
            "/ui/with\\backslash",
            "/ui/\nheader",
        ] {
            assert_eq!(sanitize_login_return_to(Some(unsafe_value)), None);
        }
    }

    #[test]
    fn google_login_token_exchange_form_uses_only_login_fields() {
        let login = build_google_login_url(&test_config(), "https://aegis.example", None)
            .expect("login url");
        let form = build_google_login_token_exchange_form(
            &test_config(),
            "client-secret",
            &login.state,
            " auth-code ",
        )
        .expect("exchange form");
        let fields: HashMap<_, _> = form.into_iter().collect();

        assert_eq!(
            fields.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(fields.get("code").map(String::as_str), Some("auth-code"));
        assert_eq!(
            fields.get("redirect_uri").map(String::as_str),
            Some("https://aegis.example/auth/callback/google")
        );
        assert_eq!(
            fields.get("code_verifier").map(String::as_str),
            Some(login.state.code_verifier.as_str())
        );
        assert!(!fields.contains_key("scope"));
        assert!(!fields.contains_key("access_type"));
        assert!(!fields.contains_key("refresh_token"));
    }

    #[test]
    fn google_login_claim_validation_accepts_allowed_verified_identity() {
        let config = test_config();
        let claims = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: None,
            auth_time: None,
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };

        validate_google_id_claims(&claims, &config, "nonce-123").expect("claims valid");
    }

    #[test]
    fn google_login_claim_validation_rejects_confused_or_unallowed_identity() {
        let config = test_config();
        let base = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: None,
            auth_time: None,
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };

        let mut wrong_aud = base.clone();
        wrong_aud.aud = "other-client".into();
        assert!(validate_google_id_claims(&wrong_aud, &config, "nonce-123").is_err());

        let mut wrong_nonce = base.clone();
        wrong_nonce.nonce = Some("other-nonce".into());
        assert!(validate_google_id_claims(&wrong_nonce, &config, "nonce-123").is_err());

        let mut unverified = base.clone();
        unverified.email_verified = false;
        assert!(validate_google_id_claims(&unverified, &config, "nonce-123").is_err());

        let mut unallowed = base;
        unallowed.email = "other@example.net".into();
        assert!(validate_google_id_claims(&unallowed, &config, "nonce-123").is_err());
    }

    #[test]
    fn google_login_rejects_non_login_scopes() {
        assert!(reject_non_login_scopes("openid email profile").is_ok());
        assert!(reject_non_login_scopes(
            "email profile https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile openid"
        )
        .is_ok());
        assert!(
            reject_non_login_scopes("openid email https://www.googleapis.com/auth/drive").is_err()
        );
        assert!(
            reject_non_login_scopes("openid https://www.googleapis.com/auth/calendar.readonly")
                .is_err()
        );
    }

    #[test]
    fn google_login_state_is_one_time() {
        let login = build_google_login_url(&test_config(), "https://aegis.example", None)
            .expect("login url");
        let parsed = url::Url::parse(&login.url).unwrap();
        let params: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let state = params.get("state").unwrap();

        let consumed = consume_pending_login_state(state).expect("state consumed");
        assert_eq!(consumed.state_hash, login.state.state_hash);
        assert!(consume_pending_login_state(state).is_err());
    }

    #[test]
    fn web_console_session_create_validate_and_revoke() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();

        let created = create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100)
            .expect("created session");
        assert_ne!(created.cookie_token, created.session.session_id_hash);
        assert_eq!(created.session.email, "l@example.com");
        assert_eq!(created.session.mfa_level, MfaLevel::GoogleOnly);
        let data_access_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('sensitive_read_grants', 'sensitive_read_audit')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(data_access_tables, 2);

        let validated = validate_session(
            &conn,
            &created.cookie_token,
            101,
            config.session_idle_ttl_secs,
        )
        .expect("validated lookup")
        .expect("valid session");
        assert_eq!(validated.session_id_hash, created.session.session_id_hash);
        assert!(
            validate_session(&conn, "wrong-token", 101, config.session_idle_ttl_secs)
                .unwrap()
                .is_none()
        );

        assert!(revoke_session(&conn, &created.cookie_token, 102).unwrap());
        assert!(
            validate_session(
                &conn,
                &created.cookie_token,
                103,
                config.session_idle_ttl_secs,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn web_console_session_validation_slides_idle_expiry() {
        let conn = Connection::open_in_memory().unwrap();
        let config = WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_LOGIN_CLIENT_ID" => Some("client-123.apps.googleusercontent.com".into()),
            "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret-present".into()),
            "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret-present".into()),
            "SESSION_TTL_SECS" => Some("1000".into()),
            "SESSION_IDLE_TTL_SECS" => Some("100".into()),
            _ => None,
        });
        let created = create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100)
            .expect("created session");
        assert_eq!(created.session.expires_at, 1100);
        assert_eq!(created.session.idle_expires_at, 200);

        let validated = validate_session(
            &conn,
            &created.cookie_token,
            150,
            config.session_idle_ttl_secs,
        )
        .expect("validated lookup")
        .expect("valid session");
        assert_eq!(validated.last_seen_at, Some(150));
        assert_eq!(validated.idle_expires_at, 250);

        let stored_idle_expires_at: i64 = conn
            .query_row(
                "SELECT idle_expires_at FROM web_console_sessions WHERE session_id_hash = ?1",
                params![validated.session_id_hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_idle_expires_at, 250);
    }

    #[test]
    fn google_auth_time_can_create_short_lived_private_access_session() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let claims = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: Some(1_000),
            auth_time: Some(990),
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };
        let evidence = google_fresh_auth_mfa_evidence(&claims, 1_000, 600)
            .expect("fresh Google auth evidence");
        assert_eq!(evidence.level, MfaLevel::GoogleFresh);
        assert_eq!(evidence.verified_at, 990);
        assert_eq!(evidence.method, "google_auth_time");

        let created = create_session_with_mfa_evidence(
            &conn,
            &config,
            &claims.sub,
            &claims.email,
            claims.email_verified,
            1_000,
            Some(evidence),
        )
        .expect("created session");
        assert_eq!(created.session.mfa_level, MfaLevel::GoogleFresh);
        assert_eq!(created.session.mfa_verified_at, Some(990));
        assert_eq!(
            created.session.mfa_method.as_deref(),
            Some("google_auth_time")
        );
    }

    #[test]
    fn google_login_evidence_falls_back_to_fresh_id_token_iat() {
        let config = test_config();
        let claims = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: Some(995),
            auth_time: None,
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };

        let evidence =
            google_login_mfa_evidence(&claims, 1_000, 600).expect("fresh Google login evidence");
        assert_eq!(evidence.level, MfaLevel::GoogleFresh);
        assert_eq!(evidence.verified_at, 995);
        assert_eq!(evidence.method, "google_login_callback");
    }

    #[test]
    fn google_login_iat_fallback_fails_closed_when_missing_stale_or_future() {
        let config = test_config();
        let mut claims = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: None,
            auth_time: None,
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };
        assert!(google_login_mfa_evidence(&claims, 1_000, 600).is_none());

        claims.iat = Some(399);
        assert!(google_login_mfa_evidence(&claims, 1_000, 600).is_none());

        claims.iat = Some(1_001);
        assert!(google_login_mfa_evidence(&claims, 1_000, 600).is_none());
    }

    #[test]
    fn google_auth_time_evidence_fails_closed_when_missing_stale_or_future() {
        let config = test_config();
        let mut claims = GoogleIdClaims {
            iss: "https://accounts.google.com".into(),
            aud: config.client_id.clone(),
            exp: 4_102_444_800,
            sub: "google-sub".into(),
            email: "L@example.com".into(),
            email_verified: true,
            nonce: Some("nonce-123".into()),
            iat: Some(1_000),
            auth_time: None,
            amr: vec![],
            name: None,
            picture: None,
            hd: None,
        };
        assert!(google_fresh_auth_mfa_evidence(&claims, 1_000, 600).is_none());

        claims.auth_time = Some(399);
        assert!(google_fresh_auth_mfa_evidence(&claims, 1_000, 600).is_none());

        claims.auth_time = Some(1_001);
        assert!(google_fresh_auth_mfa_evidence(&claims, 1_000, 600).is_none());
    }

    #[test]
    fn web_console_session_rejects_unverified_or_unallowed_email() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();

        assert!(create_session(&conn, &config, "sub", "l@example.com", false, 100).is_err());
        assert!(create_session(&conn, &config, "sub", "other@example.net", true, 100).is_err());
    }

    #[test]
    fn web_console_mfa_status_defaults_to_setup_required() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let created = create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100)
            .expect("created session");

        let status = get_mfa_status(&conn, &created.session).expect("mfa status");
        assert_eq!(status.mfa_level, "google_only");
        assert!(status.requires_mfa_setup);
        assert_eq!(status.webauthn_registered_count, 0);
        assert!(!status.webauthn_available);
        assert_eq!(status.webauthn_status, "deferred");
        assert!(!status.totp_configured);
        assert_eq!(status.preferred_step_up_method, "totp");
        assert_eq!(status.recovery_codes_remaining, 0);
    }

    #[test]
    fn web_console_mfa_status_treats_google_fresh_session_as_ready() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let created = create_session_with_mfa_evidence(
            &conn,
            &config,
            "google-sub-1",
            "L@example.com",
            true,
            100,
            Some(SessionMfaEvidence {
                level: MfaLevel::GoogleFresh,
                verified_at: 100,
                method: "google_login_callback".to_string(),
            }),
        )
        .expect("created session");

        let status = get_mfa_status(&conn, &created.session).expect("mfa status");
        assert_eq!(status.mfa_level, "google_fresh");
        assert!(!status.requires_mfa_setup);
        assert!(!status.totp_configured);
    }

    #[test]
    fn web_console_webauthn_listing_redacts_secret_material() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let created = create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100)
            .expect("created session");
        init_mfa_schema(&conn).unwrap();
        conn.execute(
            r#"
            INSERT INTO webauthn_credentials (
              id, google_sub, credential_id, public_key_cose, sign_count,
              transports_json, backup_eligible, backup_state, created_at,
              last_used_at, nickname
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                "cred-public-id",
                "google-sub-1",
                vec![1_u8, 2, 3],
                vec![4_u8, 5, 6],
                7_i64,
                r#"["internal","usb"]"#,
                1_i64,
                0_i64,
                123_i64,
                456_i64,
                "Laptop passkey",
            ],
        )
        .unwrap();

        let status = get_mfa_status(&conn, &created.session).expect("mfa status");
        assert!(status.requires_mfa_setup);
        assert_eq!(status.webauthn_registered_count, 1);
        assert!(!status.webauthn_available);
        assert_eq!(status.webauthn_status, "deferred");

        let credentials =
            list_webauthn_credentials(&conn, &created.session.google_sub).expect("credentials");
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].id, "cred-public-id");
        assert_eq!(credentials[0].nickname.as_deref(), Some("Laptop passkey"));
        assert_eq!(credentials[0].transports, vec!["internal", "usb"]);
        assert_eq!(credentials[0].backup_eligible, Some(true));
        assert_eq!(credentials[0].backup_state, Some(false));

        let serialized = serde_json::to_value(&credentials[0]).unwrap();
        assert!(serialized.get("credential_id").is_none());
        assert!(serialized.get("public_key_cose").is_none());
        assert!(serialized.get("sign_count").is_none());
    }

    #[test]
    fn recent_mfa_helper_accepts_totp_or_webauthn_for_any_policy() {
        let policy = MfaStepUpPolicy::recent_mfa("S3 sensitive read");
        assert_eq!(
            require_recent_mfa(&policy, MfaLevel::GoogleFresh, Some(950), 1_000),
            MfaStepUpDecision::Satisfied
        );
        assert_eq!(
            require_recent_mfa(&policy, MfaLevel::Totp, Some(950), 1_000),
            MfaStepUpDecision::Satisfied
        );
        assert_eq!(
            require_recent_mfa(&policy, MfaLevel::WebAuthn, Some(950), 1_000),
            MfaStepUpDecision::Satisfied
        );
    }

    #[test]
    fn recent_mfa_helper_rejects_stale_future_or_recovery_code() {
        let policy = MfaStepUpPolicy::recent_mfa("delete memory entry");
        assert!(matches!(
            require_recent_mfa(&policy, MfaLevel::Totp, Some(300), 1_000),
            MfaStepUpDecision::Required(_)
        ));
        assert!(matches!(
            require_recent_mfa(&policy, MfaLevel::Totp, Some(1_001), 1_000),
            MfaStepUpDecision::Required(_)
        ));
        assert!(matches!(
            require_recent_mfa(&policy, MfaLevel::RecoveryCode, Some(950), 1_000),
            MfaStepUpDecision::Required(_)
        ));
    }

    #[test]
    fn host_control_does_not_satisfy_approval_step_up() {
        let policy = MfaStepUpPolicy::recent_mfa("high-risk approval");
        assert_eq!(MfaLevel::parse(MfaLevel::Host.as_str()), MfaLevel::Host);
        assert!(matches!(
            require_recent_mfa(&policy, MfaLevel::Host, Some(950), 1_000),
            MfaStepUpDecision::Required(_)
        ));
    }

    #[test]
    fn host_confirmed_satisfies_approval_step_up_only_while_fresh() {
        let policy = MfaStepUpPolicy::recent_mfa("high-risk approval");
        assert_eq!(MfaLevel::parse("host_confirmed"), MfaLevel::HostConfirmed);
        assert_eq!(
            require_recent_mfa(&policy, MfaLevel::HostConfirmed, Some(950), 1_000),
            MfaStepUpDecision::Satisfied
        );
        assert!(matches!(
            require_recent_mfa(&policy, MfaLevel::HostConfirmed, Some(300), 1_000),
            MfaStepUpDecision::Required(_)
        ));
        let read_policy = crate::data_access_gate::policy_for_request(
            crate::data_access_gate::DataSensitivity::S3Sensitive,
            &[crate::data_access_gate::DataAction::ReadBody],
        );
        assert!(matches!(
            crate::data_access_gate::evaluate_mfa(
                &read_policy,
                MfaLevel::HostConfirmed,
                Some(950),
                1_000,
            ),
            crate::data_access_gate::DataAccessPolicyDecision::RequireMfa { .. }
        ));
    }

    #[test]
    fn webauthn_policy_rejects_totp_with_structured_response() {
        let policy = MfaStepUpPolicy::recent_webauthn("S4 sensitive read");
        let decision = require_recent_mfa(&policy, MfaLevel::Totp, Some(950), 1_000);
        let MfaStepUpDecision::Required(response) = decision else {
            panic!("expected mfa_required response");
        };
        assert_eq!(response.error, "mfa_required");
        assert_eq!(response.required_level, "webauthn");
        assert_eq!(response.reason, "S4 sensitive read");
        assert_eq!(
            response.freshness_secs,
            crate::data_access_gate::DEFAULT_MFA_FRESHNESS_SECS
        );
        assert_eq!(
            response.challenge_hint.start_url,
            "/mfa/webauthn/assert/start"
        );
    }

    #[test]
    fn any_mfa_policy_points_to_totp_settings() {
        let policy = MfaStepUpPolicy::recent_mfa("S3 sensitive read");
        let decision = require_recent_mfa(&policy, MfaLevel::GoogleOnly, None, 1_000);
        let MfaStepUpDecision::Required(response) = decision else {
            panic!("expected mfa_required response");
        };
        assert_eq!(response.error, "mfa_required");
        assert_eq!(response.required_level, "any");
        assert_eq!(response.challenge_hint.start_url, "/ui/?tab=system");
    }

    #[test]
    fn session_mfa_helper_uses_session_mfa_state() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let mut session =
            create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100)
                .unwrap()
                .session;
        session.mfa_level = MfaLevel::WebAuthn;
        session.mfa_verified_at = Some(950);

        let policy = MfaStepUpPolicy::recent_webauthn("change allowed domains");
        assert_eq!(
            require_recent_session_mfa(&policy, &session, 1_000),
            MfaStepUpDecision::Satisfied
        );
    }

    #[test]
    fn web_console_session_cookie_header_has_secure_host_attributes() {
        let config = test_config();
        let header = build_session_cookie_header(&config, "opaque-token").unwrap();

        assert!(header.starts_with("__Host-aegis_session=opaque-token; Path=/;"));
        assert!(header.contains("Secure"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
        assert!(header.contains("Max-Age=2592000"));

        let clear = build_clear_session_cookie_header(&config).unwrap();
        assert!(clear.starts_with("__Host-aegis_session=; Path=/; Max-Age=0;"));
        assert!(clear.contains("Secure"));
        assert!(clear.contains("HttpOnly"));
    }

    #[test]
    fn web_console_session_cookie_header_rejects_unsafe_values() {
        let config = test_config();
        assert!(build_session_cookie_header(&config, "bad;token").is_err());
        assert!(build_session_cookie_header(&config, "bad token").is_err());

        let mut bad_name = config;
        bad_name.session_cookie_name = "__Host-bad;name".into();
        assert!(build_session_cookie_header(&bad_name, "opaque-token").is_err());
        assert!(build_clear_session_cookie_header(&bad_name).is_err());
    }

    #[test]
    fn web_console_csrf_token_is_stable_and_session_bound() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let first =
            create_session(&conn, &config, "google-sub-1", "L@example.com", true, 100).unwrap();
        let second =
            create_session(&conn, &config, "google-sub-2", "L@example.com", true, 100).unwrap();

        assert_eq!(
            csrf_token_for_session(&first.session),
            csrf_token_for_session(&first.session)
        );
        assert_ne!(
            csrf_token_for_session(&first.session),
            csrf_token_for_session(&second.session)
        );
        assert_eq!(csrf_token_for_session(&first.session).len(), 64);
    }

    #[test]
    fn web_console_cookie_extracts_only_safe_matching_value() {
        let header = "other=value; __Host-aegis_session=abc-DEF_123; theme=dark";
        assert_eq!(
            extract_cookie_value(header, "__Host-aegis_session").as_deref(),
            Some("abc-DEF_123")
        );
        assert!(
            extract_cookie_value("__Host-aegis_session=bad token", "__Host-aegis_session")
                .is_none()
        );
        assert!(
            extract_cookie_value("__Host-aegis_session=bad,token", "__Host-aegis_session")
                .is_none()
        );
        assert!(extract_cookie_value("__Host-bad;name=value", "__Host-bad;name").is_none());
    }

    #[test]
    fn create_local_session_bypasses_allowlist_only_through_local_path() {
        let conn = Connection::open_in_memory().unwrap();
        let config = test_config();
        let now = 1_800_000_000u64;

        // Negative control: the regular Google path still refuses the local
        // e-mail, so the allowlist bypass is not a general regression.
        let refused = create_session(
            &conn,
            &config,
            LOCAL_LOGIN_SUB,
            LOCAL_LOGIN_EMAIL,
            true,
            now,
        );
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.to_string().contains("not allowed")),
            "create_session must keep enforcing the allowlist: {refused:?}"
        );

        let created = create_local_session(&conn, &config, now).unwrap();
        assert_eq!(created.session.google_sub, LOCAL_LOGIN_SUB);
        assert_eq!(created.session.email, LOCAL_LOGIN_EMAIL);
        assert!(created.session.email_verified);
        assert_eq!(created.session.mfa_level, MfaLevel::GoogleOnly);

        // The row is a real session: the cookie token validates and is
        // rejected once the parent TTL is over.
        let found = validate_session(
            &conn,
            &created.cookie_token,
            now + 1,
            config.session_idle_ttl_secs,
        )
        .unwrap()
        .expect("local session validates");
        assert_eq!(found.email, LOCAL_LOGIN_EMAIL);
        assert!(
            validate_session(
                &conn,
                &created.cookie_token,
                now + config.session_ttl_secs,
                config.session_idle_ttl_secs
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn generated_local_login_token_is_32_bytes_hex() {
        let a = generate_local_login_token().unwrap();
        let b = generate_local_login_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, b, "two draws must differ");
    }
}
