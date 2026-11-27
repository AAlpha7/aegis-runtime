//! Telegram WebApp initData HMAC-SHA256 verification.
//!
//! When a Telegram Mini App opens, it receives `initData` — a URL-encoded string
//! containing user identity and a hash. The backend MUST verify this hash using
//! the bot token to authenticate requests (zero-trust, no passwords needed).
//!
//! Verification steps (per Telegram docs):
//! 1. Parse initData into key-value pairs
//! 2. Remove the `hash` field
//! 3. Sort remaining fields alphabetically
//! 4. Join as `key=value\n` lines
//! 5. HMAC-SHA256(key=HMAC-SHA256("WebAppData", bot_token), data=joined_string)
//! 6. Compare with the provided hash
//!
//! This ensures requests genuinely originate from Telegram.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Maximum age of initData before it's considered stale.
/// 1800 seconds (30 min) accommodates panel-style UIs where the user
/// browses and clicks buttons over several minutes. Telegram generates
/// initData once on WebApp open — shorter windows cause silent 401 on
/// button clicks after timeout. Security is maintained by HMAC signature
/// verification + admin_id access control — the time window only limits
/// replay attacks, which are low-risk for read-heavy panels.
const MAX_AUTH_AGE_SECS: u64 = 1800;

/// Forward clock-skew tolerance. If the server clock trails Telegram's
/// clock (e.g. NTP not yet synced after boot), `auth_date` can look
/// "in the future". Accept up to 5 minutes of future skew before rejecting,
/// matching common SSO/OIDC leeway values.
const FUTURE_SKEW_TOLERANCE_SECS: u64 = 300;

/// Verified Telegram WebApp user identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebAppUser {
    pub id: i64,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub language_code: String,
    /// Whether the user is a Telegram Premium subscriber.
    #[serde(default)]
    pub is_premium: bool,
}

/// Result of verifying Telegram WebApp initData.
#[derive(Debug, Clone)]
pub struct VerifiedInitData {
    /// The authenticated Telegram user.
    pub user: WebAppUser,
    /// Chat ID where the WebApp was opened (if available).
    pub chat_id: Option<i64>,
    /// Unix timestamp from the initData.
    pub auth_date: u64,
    /// Raw query_id for answering WebApp queries.
    pub query_id: Option<String>,
}

/// Verify Telegram WebApp initData using HMAC-SHA256.
///
/// `init_data`: The raw URL-encoded initData string from `Telegram.WebApp.initData`.
/// `bot_token`: The Telegram Bot API token (e.g., "123456:ABC-DEF").
///
/// Returns the verified user identity, or an error if verification fails.
pub fn verify_init_data(init_data: &str, bot_token: &str) -> Result<VerifiedInitData> {
    if init_data.is_empty() {
        bail!("initData is empty");
    }

    // Parse URL-encoded key=value pairs
    let params: BTreeMap<String, String> = form_urlencoded::parse(init_data.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Extract and remove hash
    let received_hash = params
        .get("hash")
        .ok_or_else(|| anyhow::anyhow!("initData missing 'hash' field"))?
        .clone();

    // Build data-check-string: sorted key=value pairs (excluding hash), joined by \n
    let data_check_string: String = params
        .iter()
        .filter(|(k, _)| k.as_str() != "hash")
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    // Step 1: secret_key = HMAC-SHA256("WebAppData", bot_token)
    let mut secret_hmac =
        HmacSha256::new_from_slice(b"WebAppData").expect("HMAC accepts any key size");
    secret_hmac.update(bot_token.as_bytes());
    let secret_key = secret_hmac.finalize().into_bytes();

    // Step 2: hash = HMAC-SHA256(secret_key, data_check_string)
    let mut data_hmac = HmacSha256::new_from_slice(&secret_key).expect("HMAC accepts any key size");
    data_hmac.update(data_check_string.as_bytes());
    let computed_hash = hex::encode(data_hmac.finalize().into_bytes());

    // Step 3: Constant-time comparison
    if !constant_time_eq(&computed_hash, &received_hash) {
        bail!("initData hash verification failed — possible tampering");
    }

    // Validate auth_date freshness. We use signed arithmetic so that a
    // *backward-drifting* server clock (auth_date > now) is caught as
    // "future skew" rather than silently passing via saturating_sub(0).
    let auth_date: i64 = params
        .get("auth_date")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let delta = now - auth_date;
    if delta > MAX_AUTH_AGE_SECS as i64 {
        bail!("initData too old (auth_date={auth_date}, now={now}, max_age={MAX_AUTH_AGE_SECS}s)");
    }
    if delta < -(FUTURE_SKEW_TOLERANCE_SECS as i64) {
        bail!(
            "initData auth_date is too far in the future (auth_date={auth_date}, now={now}, skew={}s, tolerance={FUTURE_SKEW_TOLERANCE_SECS}s) — check server clock",
            -delta
        );
    }
    if delta < 0 {
        tracing::debug!(
            auth_date,
            now,
            skew_secs = -delta,
            "initData auth_date slightly ahead of server clock — within tolerance"
        );
    }

    // Parse user JSON
    let user_json = params
        .get("user")
        .ok_or_else(|| anyhow::anyhow!("initData missing 'user' field"))?;
    let user: WebAppUser = serde_json::from_str(user_json)
        .map_err(|e| anyhow::anyhow!("Failed to parse user JSON: {e}"))?;

    // Extract optional fields
    let chat_id = params
        .get("chat_instance")
        .and_then(|s| s.parse::<i64>().ok());
    let query_id = params.get("query_id").cloned();

    // auth_date is non-negative in practice (rejected earlier if too far in
    // the future); clamp to u64 for the public struct.
    let auth_date_u64 = auth_date.max(0) as u64;

    Ok(VerifiedInitData {
        user,
        chat_id,
        auth_date: auth_date_u64,
        query_id,
    })
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Verify that a Telegram user ID is the admin for this bot instance.
/// Used to restrict WebApp access to the authorized admin only.
pub fn verify_admin_access(user_id: i64, admin_chat_id: i64) -> Result<()> {
    if user_id != admin_chat_id {
        bail!("Access denied: user {user_id} is not the admin ({admin_chat_id})");
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector from Telegram's official documentation.
    fn make_test_init_data(bot_token: &str) -> String {
        let auth_date = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let user_json = serde_json::json!({
            "id": 12345678,
            "first_name": "Test",
            "last_name": "User",
            "username": "testuser",
            "language_code": "en"
        })
        .to_string();

        // Build data-check-string (sorted)
        let mut params = BTreeMap::new();
        params.insert("auth_date", auth_date.to_string());
        params.insert("user", user_json.clone());
        params.insert("query_id", "test_query".into());

        let data_check_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        // Compute HMAC
        let mut secret_hmac = HmacSha256::new_from_slice(b"WebAppData").unwrap();
        secret_hmac.update(bot_token.as_bytes());
        let secret_key = secret_hmac.finalize().into_bytes();

        let mut data_hmac = HmacSha256::new_from_slice(&secret_key).unwrap();
        data_hmac.update(data_check_string.as_bytes());
        let hash = hex::encode(data_hmac.finalize().into_bytes());

        // URL-encode
        format!(
            "auth_date={}&user={}&query_id=test_query&hash={}",
            auth_date,
            urlencoding::encode(&user_json),
            hash
        )
    }

    #[test]
    fn test_verify_valid_init_data() {
        let bot_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";
        let init_data = make_test_init_data(bot_token);

        let result = verify_init_data(&init_data, bot_token);
        assert!(result.is_ok(), "Verification failed: {:?}", result.err());

        let verified = result.unwrap();
        assert_eq!(verified.user.id, 12345678);
        assert_eq!(verified.user.username, "testuser");
        assert_eq!(verified.query_id.as_deref(), Some("test_query"));
    }

    #[test]
    fn test_verify_tampered_data() {
        let bot_token = "123456:ABC-DEF";
        let init_data = make_test_init_data(bot_token);

        // Tamper with the data
        let tampered = init_data.replace("testuser", "hacker");
        let result = verify_init_data(&tampered, bot_token);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("hash verification"),
            "Should fail hash check"
        );
    }

    #[test]
    fn test_verify_wrong_token() {
        let bot_token = "123456:ABC-DEF";
        let init_data = make_test_init_data(bot_token);

        let result = verify_init_data(&init_data, "wrong_token");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_empty_init_data() {
        let result = verify_init_data("", "token");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_verify_missing_hash() {
        let result = verify_init_data("auth_date=123&user={}", "token");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash"));
    }

    #[test]
    fn test_verify_expired_data() {
        let bot_token = "123456:ABC-DEF";
        let old_auth_date = 1000000; // Very old timestamp

        let user_json = serde_json::json!({"id": 1, "first_name": "X"}).to_string();

        let mut params = BTreeMap::new();
        params.insert("auth_date", old_auth_date.to_string());
        params.insert("user", user_json.clone());

        let data_check_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut secret_hmac = HmacSha256::new_from_slice(b"WebAppData").unwrap();
        secret_hmac.update(bot_token.as_bytes());
        let secret_key = secret_hmac.finalize().into_bytes();

        let mut data_hmac = HmacSha256::new_from_slice(&secret_key).unwrap();
        data_hmac.update(data_check_string.as_bytes());
        let hash = hex::encode(data_hmac.finalize().into_bytes());

        let init_data = format!(
            "auth_date={}&user={}&hash={}",
            old_auth_date,
            urlencoding::encode(&user_json),
            hash
        );

        let result = verify_init_data(&init_data, bot_token);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too old"));
    }

    /// Helper: build initData with a specified auth_date (for skew tests).
    fn make_init_data_with_auth_date(bot_token: &str, auth_date: i64) -> String {
        let user_json = serde_json::json!({"id": 1, "first_name": "X"}).to_string();
        let mut params = BTreeMap::new();
        params.insert("auth_date", auth_date.to_string());
        params.insert("user", user_json.clone());

        let data_check_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut secret_hmac = HmacSha256::new_from_slice(b"WebAppData").unwrap();
        secret_hmac.update(bot_token.as_bytes());
        let secret_key = secret_hmac.finalize().into_bytes();

        let mut data_hmac = HmacSha256::new_from_slice(&secret_key).unwrap();
        data_hmac.update(data_check_string.as_bytes());
        let hash = hex::encode(data_hmac.finalize().into_bytes());

        format!(
            "auth_date={}&user={}&hash={}",
            auth_date,
            urlencoding::encode(&user_json),
            hash
        )
    }

    #[test]
    fn test_verify_future_skew_within_tolerance() {
        // Server clock trails Telegram by 60s — should still accept.
        let bot_token = "123456:ABC-DEF";
        let now_plus_60 = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            + 60;
        let init_data = make_init_data_with_auth_date(bot_token, now_plus_60);
        let result = verify_init_data(&init_data, bot_token);
        assert!(
            result.is_ok(),
            "Should accept future skew within tolerance, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_verify_future_skew_beyond_tolerance() {
        // Server clock trails Telegram by 10 min — must reject (possible
        // misconfiguration or spoofed future timestamp).
        let bot_token = "123456:ABC-DEF";
        let now_plus_600 = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64)
            + 600;
        let init_data = make_init_data_with_auth_date(bot_token, now_plus_600);
        let result = verify_init_data(&init_data, bot_token);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("future"),
            "Error should mention 'future' skew"
        );
    }

    #[test]
    fn test_admin_access_check() {
        assert!(verify_admin_access(12345, 12345).is_ok());
        assert!(verify_admin_access(12345, 99999).is_err());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("hello", "hello"));
        assert!(!constant_time_eq("hello", "world"));
        assert!(!constant_time_eq("hello", "hell"));
    }

    #[test]
    fn test_webapp_user_deserialization() {
        let json = r#"{"id":123,"first_name":"张","username":"zhangsan","is_premium":true}"#;
        let user: WebAppUser = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, 123);
        assert_eq!(user.first_name, "张");
        assert!(user.is_premium);
        assert!(user.last_name.is_empty()); // default
    }
}
