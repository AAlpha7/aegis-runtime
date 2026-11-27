//! CEO Voice Agent — mint ephemeral xAI Realtime client credentials.
//! Spec: docs/design/2026-07-25-ceo-voice-assistant.md
//!
//! Credential order (transcription + Hercules STT/voice):
//!   1. Owner Grok **subscription** OAuth (Aegis encrypted credential store)
//!   2. Grok CLI / gtok-style token from `~/.grok/auth.json` (same login as agents)
//!   3. Optional paid API keys only as last resort (XAI_API_KEY / …)
//!
//! Browser never sees the bearer. Server mints a short-lived client token for
//! WebSocket subprotocol `xai-client-secret.{token}`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const XAI_REALTIME_CLIENT_SECRETS_URL: &str = "https://api.x.ai/v1/realtime/client_secrets";
pub const DEFAULT_VOICE_MODEL: &str = "grok-voice-think-fast-2.0";
pub const DEFAULT_WS_URL: &str = "wss://api.x.ai/v1/realtime";
pub const CLIENT_SECRET_TTL_SECONDS: u64 = 300;
pub const DEFAULT_MAX_SESSION_SECONDS: u64 = 600;

/// Owner-facing refusal when chat is bound to the Codex subscription.
/// Realtime voice stays on the xAI channel; this is not a Codex session and
/// not an OpenRouter voice fallback.
pub const CODEX_REALTIME_VOICE_UNSUPPORTED: &str = "Realtime voice is not supported on the Codex subscription; switch to Grok for voice, or use chat.";

/// `true` when the owner explicitly stored Codex as `llm_provider`.
///
/// `None` is the registry seed (`get` reports `codex_oauth` before any store).
/// That seed is not a chat-provider selection, so the Grok mint path stays.
pub fn llm_provider_blocks_realtime_voice(stored_llm_provider: Option<&str>) -> bool {
    match stored_llm_provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => matches!(
            raw.parse::<crate::llm::LlmProviderKind>(),
            Ok(crate::llm::LlmProviderKind::CodexOAuth)
        ),
        None => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceKeySource {
    /// Aegis Settings → Connect Grok (device-code OAuth, encrypted store).
    XaiSubscription,
    /// `~/.grok/auth.json` machine login (Grok CLI / gtok-style).
    GrokCli,
    /// Paid console key (last resort).
    ApiKey,
    /// Test stub.
    Stub,
}

impl VoiceKeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XaiSubscription => "xai-subscription",
            Self::GrokCli => "grok-cli",
            Self::ApiKey => "api-key",
            Self::Stub => "stub",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MintedVoiceSession {
    pub client_secret: String,
    pub expires_in_seconds: u64,
    pub model: String,
    pub ws_url: String,
    pub max_session_seconds: u64,
    /// Where the mint bearer came from (never the token itself).
    #[serde(default)]
    pub key_source: String,
}

/// Grok CLI / gtok machine token from `~/.grok/auth.json` (Hercules shape).
///
/// File shape: `{ "https://auth.x.ai::<id>": { "key": "...", "expires_at": "..." } }`
pub fn read_grok_cli_access_token(home: &Path) -> Option<String> {
    read_grok_cli_access_token_from_auth_file(&home.join(".grok").join("auth.json"))
}

/// Read token from an explicit auth.json path (same shape as Grok CLI).
fn read_grok_cli_access_token_from_auth_file(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let obj = parsed.as_object()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    for value in obj.values() {
        let entry = value.as_object()?;
        let key = entry.get("key").and_then(|v| v.as_str())?;
        if key.trim().len() < 20 {
            continue;
        }
        if let Some(exp) = entry.get("expires_at").and_then(|v| v.as_str())
            && let Ok(t) = chrono::DateTime::parse_from_rfc3339(exp)
            // Skip if expired more than 60s ago (Hercules tolerance).
            && t.timestamp_millis() < now - 60_000
        {
            continue;
        }
        return Some(key.trim().to_string());
    }
    None
}

pub fn read_grok_cli_access_token_from_env_home() -> Option<String> {
    // Explicit file path (systemd / deploy can point at a machine login).
    if let Ok(path) = std::env::var("AEGIS_GROK_AUTH_JSON") {
        let trimmed = path.trim();
        if !trimmed.is_empty()
            && let Some(tok) = read_grok_cli_access_token_from_auth_file(Path::new(trimmed))
        {
            return Some(tok);
        }
    }
    // Optional home override (Hercules / multi-user hosts).
    if let Ok(home) = std::env::var("AEGIS_GROK_HOME") {
        let home = home.trim();
        if !home.is_empty()
            && let Some(tok) = read_grok_cli_access_token(Path::new(home))
        {
            return Some(tok);
        }
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs_next_home());
    home.as_ref().and_then(|h| read_grok_cli_access_token(h))
}

fn dirs_next_home() -> Option<PathBuf> {
    // Avoid extra crate: expand via std only.
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Paid console keys only — last resort, not the primary path.
pub fn resolve_paid_api_key_from_env() -> Option<String> {
    for key in ["XAI_API_KEY", "GROK_API_KEY", "AEGIS_STT_API_KEY"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() && !trimmed.contains("YOUR_") && !trimmed.contains("changeme") {
                return Some(trimmed.to_string());
            }
        }
    }
    let base = std::env::var("OPENAI_API_BASE").unwrap_or_default();
    if (base.contains("x.ai") || base.contains("api.x.ai"))
        && let Ok(value) = std::env::var("OPENAI_API_KEY")
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !trimmed.contains("YOUR_") {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Whether a stored Grok subscription access token is still usable for mint.
///
/// Matches `XaiSubscriptionCredential::needs_refresh` inverse: token must be
/// non-empty and either have no expiry, or expire more than 60s from `now_ms`.
/// Status used to treat any non-empty access_token as `realtime_available`,
/// which lied after revoke/expiry and made the orb attempt Realtime (no
/// dictation fallback) until mint failed.
pub fn subscription_access_token_usable(
    access_token: &str,
    expires_at_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    if access_token.trim().is_empty() {
        return false;
    }
    match expires_at_ms {
        Some(expires_at_ms) => expires_at_ms > now_ms.saturating_add(60_000),
        None => true,
    }
}

/// Env-only cascade used when no AdminState (tests / status probe without store).
/// Prefers Grok CLI subscription token over paid API keys.
pub fn resolve_voice_bearer_from_env() -> Option<(String, VoiceKeySource)> {
    if std::env::var("AEGIS_VOICE_TEST_STUB").ok().as_deref() == Some("1") {
        return Some(("stub".into(), VoiceKeySource::Stub));
    }
    if let Some(token) = read_grok_cli_access_token_from_env_home() {
        return Some((token, VoiceKeySource::GrokCli));
    }
    resolve_paid_api_key_from_env().map(|k| (k, VoiceKeySource::ApiKey))
}

/// Backward-compatible name: token string only (env / CLI).
pub fn resolve_voice_api_key_from_env() -> Option<String> {
    resolve_voice_bearer_from_env().map(|(t, _)| t)
}

pub fn voice_agent_model_from_env() -> String {
    std::env::var("AEGIS_VOICE_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_VOICE_MODEL.to_string())
}

pub fn voice_ws_url_from_env() -> String {
    std::env::var("AEGIS_VOICE_WS_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_WS_URL.to_string())
}

pub fn max_session_seconds_from_env() -> u64 {
    std::env::var("AEGIS_VOICE_MAX_SESSION_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n >= 60)
        .map(|n: u64| n.min(1800))
        .unwrap_or(DEFAULT_MAX_SESSION_SECONDS)
}

/// Extract client secret from documented / OpenAI-compatible mint shapes.
pub fn extract_client_secret(data: &serde_json::Value) -> Option<String> {
    if let Some(value) = data.get("value").and_then(|v| v.as_str()) {
        let t = value.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    match data.get("client_secret") {
        Some(serde_json::Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(obj) => obj
            .get("value")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        None => None,
    }
}

/// Mint via xAI (or return stub when AEGIS_VOICE_TEST_STUB=1).
pub async fn mint_realtime_client_secret(
    bearer: &str,
    expires_seconds: u64,
    key_source: VoiceKeySource,
) -> Result<MintedVoiceSession, String> {
    if key_source == VoiceKeySource::Stub
        || std::env::var("AEGIS_VOICE_TEST_STUB").ok().as_deref() == Some("1")
    {
        return Ok(MintedVoiceSession {
            client_secret: format!("stub-voice-{}", expires_seconds),
            expires_in_seconds: expires_seconds,
            model: voice_agent_model_from_env(),
            ws_url: voice_ws_url_from_env(),
            max_session_seconds: max_session_seconds_from_env(),
            key_source: VoiceKeySource::Stub.as_str().to_string(),
        });
    }
    let client = reqwest::Client::new();
    let res = client
        .post(XAI_REALTIME_CLIENT_SECRETS_URL)
        .bearer_auth(bearer)
        .header("User-Agent", "aegis-runtime/ceo-voice")
        .json(&serde_json::json!({ "expires_after": { "seconds": expires_seconds } }))
        .send()
        .await
        .map_err(|e| format!("Voice mint network error: {e}"))?;
    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| format!("Voice mint body error: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Voice mint failed ({}): {} [source={}]",
            status.as_u16(),
            body.chars().take(300).collect::<String>(),
            key_source.as_str()
        ));
    }
    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Voice mint JSON error: {e}"))?;
    let secret = extract_client_secret(&data)
        .ok_or_else(|| "Voice mint returned an unrecognized response shape".to_string())?;
    Ok(MintedVoiceSession {
        client_secret: secret,
        expires_in_seconds: expires_seconds,
        model: voice_agent_model_from_env(),
        ws_url: voice_ws_url_from_env(),
        max_session_seconds: max_session_seconds_from_env(),
        key_source: key_source.as_str().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn codex_llm_provider_blocks_realtime_and_grok_does_not() {
        assert!(llm_provider_blocks_realtime_voice(Some("codex_oauth")));
        assert!(llm_provider_blocks_realtime_voice(Some("codex")));
        assert!(llm_provider_blocks_realtime_voice(Some("  CODEX_OAUTH  ")));
        assert!(!llm_provider_blocks_realtime_voice(Some(
            "xai_subscription"
        )));
        assert!(!llm_provider_blocks_realtime_voice(Some("grok")));
        assert!(!llm_provider_blocks_realtime_voice(Some("openrouter")));
        assert!(!llm_provider_blocks_realtime_voice(Some("openai_compat")));
        assert!(!llm_provider_blocks_realtime_voice(None));
        assert!(!llm_provider_blocks_realtime_voice(Some("")));
        assert!(!llm_provider_blocks_realtime_voice(Some("   ")));
    }

    #[test]
    fn extracts_flat_and_nested_secrets() {
        let flat = serde_json::json!({ "value": "abc" });
        assert_eq!(extract_client_secret(&flat).as_deref(), Some("abc"));
        let nested = serde_json::json!({ "client_secret": { "value": "xyz" } });
        assert_eq!(extract_client_secret(&nested).as_deref(), Some("xyz"));
        let str_nested = serde_json::json!({ "client_secret": "tok" });
        assert_eq!(extract_client_secret(&str_nested).as_deref(), Some("tok"));
    }

    #[test]
    fn reads_grok_cli_auth_json_shape() {
        let dir = std::env::temp_dir().join(format!("aegis-voice-cli-{}", std::process::id()));
        let grok = dir.join(".grok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&grok).unwrap();
        let path = grok.join("auth.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // Far-future expiry.
        // Fixture only — not a real credential. // gitleaks:allow
        let fixture_key = format!("{}{}", "test-fixture-token-", "not-a-real-credential-xx");
        write!(
            f,
            r#"{{"https://auth.x.ai::test":{{"key":"{fixture_key}","expires_at":"2099-01-01T00:00:00Z"}}}}"#
        )
        .unwrap();
        let token = read_grok_cli_access_token(&dir).expect("token");
        assert_eq!(token, fixture_key);
        let from_file = read_grok_cli_access_token_from_auth_file(&path).expect("file token");
        assert_eq!(from_file, fixture_key);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscription_access_token_usable_respects_expiry_skew() {
        let now = 1_000_000_u64;
        assert!(!subscription_access_token_usable(
            "",
            Some(now + 120_000),
            now
        ));
        assert!(!subscription_access_token_usable(
            "   ",
            Some(now + 120_000),
            now
        ));
        // Inside 60s skew → not usable (same as needs_refresh).
        assert!(!subscription_access_token_usable(
            "tok",
            Some(now + 30_000),
            now
        ));
        assert!(!subscription_access_token_usable("tok", Some(now), now));
        assert!(!subscription_access_token_usable("tok", Some(now - 1), now));
        // Beyond 60s → usable.
        assert!(subscription_access_token_usable(
            "tok",
            Some(now + 60_001),
            now
        ));
        // Unknown expiry → optimistic usable when token present.
        assert!(subscription_access_token_usable("tok", None, now));
    }
}
