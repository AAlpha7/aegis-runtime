//! OpenRouter API client functions for credits, activity, and usage data.

use std::time::Duration;

use serde_json::Value;

/// Fetch OpenRouter account credits and/or activity data.
///
/// Actions:
/// - `balance`: GET /api/v1/credits → total_credits, total_usage
/// - `activity`: GET /api/v1/activity → usage breakdown by model/day
/// - `summary`: both balance + today's activity in one response
///
/// `mgmt_key`: optional management/provisioning key for activity endpoint
/// (regular API keys get 403 on /activity). Falls back to `api_key` if None.
pub async fn fetch_openrouter_usage(
    http: &reqwest::Client,
    api_key: &str,
    api_base: &str,
    action: &str,
    date_filter: Option<&str>,
    mgmt_key: Option<&str>,
) -> String {
    // Derive base URL (strip /v1 suffix if present — credits/activity are under /api/v1/)
    let base = if api_base.contains("openrouter.ai") {
        "https://openrouter.ai/api/v1"
    } else {
        api_base.trim_end_matches('/')
    };

    // Activity endpoint needs management key; credits works with regular key
    let activity_key = mgmt_key.unwrap_or(api_key);

    match action {
        "balance" => fetch_openrouter_credits(http, api_key, base).await,
        "activity" => fetch_openrouter_activity(http, activity_key, base, date_filter).await,
        "summary" => {
            // Fetch both in parallel — credits with regular key, activity with mgmt key
            let (credits, activity) = tokio::join!(
                fetch_openrouter_credits_raw(http, api_key, base),
                fetch_openrouter_activity_raw(http, activity_key, base, None),
            );

            let mut result = serde_json::json!({"status": "ok"});

            match credits {
                Ok(c) => {
                    result["balance"] = serde_json::json!({
                        "total_credits": c.0,
                        "total_usage": c.1,
                        "remaining": c.0 - c.1,
                    });
                }
                Err(e) => {
                    result["balance_error"] = Value::String(e);
                }
            }

            // Summarize today's activity
            match activity {
                Ok(entries) => {
                    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                    let today_entries: Vec<&Value> = entries
                        .iter()
                        .filter(|e| e.get("date").and_then(|d| d.as_str()) == Some(&today))
                        .collect();

                    let total_cost: f64 = today_entries
                        .iter()
                        .filter_map(|e| e.get("usage").and_then(|v| v.as_f64()))
                        .sum();
                    let total_requests: i64 = today_entries
                        .iter()
                        .filter_map(|e| e.get("requests").and_then(|v| v.as_i64()))
                        .sum();
                    let total_prompt: i64 = today_entries
                        .iter()
                        .filter_map(|e| e.get("prompt_tokens").and_then(|v| v.as_i64()))
                        .sum();
                    let total_completion: i64 = today_entries
                        .iter()
                        .filter_map(|e| e.get("completion_tokens").and_then(|v| v.as_i64()))
                        .sum();
                    let total_reasoning: i64 = today_entries
                        .iter()
                        .filter_map(|e| e.get("reasoning_tokens").and_then(|v| v.as_i64()))
                        .sum();

                    result["today"] = serde_json::json!({
                        "date": today,
                        "total_cost_usd": total_cost,
                        "total_requests": total_requests,
                        "total_prompt_tokens": total_prompt,
                        "total_completion_tokens": total_completion,
                        "total_reasoning_tokens": total_reasoning,
                        "models_used": today_entries.len(),
                        "interpretation_warning": "This is UTC-date provider activity. A zero provider day does not prove local Aegis Runtime made no calls; compare the local summary too.",
                    });

                    // Top 5 models by cost today
                    let mut model_costs: Vec<(&str, f64)> = today_entries
                        .iter()
                        .filter_map(|e| {
                            let model = e.get("model").and_then(|m| m.as_str())?;
                            let cost = e.get("usage").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            Some((model, cost))
                        })
                        .collect();
                    model_costs
                        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    model_costs.truncate(5);

                    if !model_costs.is_empty() {
                        result["today"]["top_models"] = model_costs
                            .iter()
                            .map(|(m, c)| serde_json::json!({"model": m, "cost_usd": c}))
                            .collect();
                    }
                }
                Err(e) => {
                    result["activity_error"] = Value::String(e);
                }
            }

            result.to_string()
        }
        _ => serde_json::json!({
            "status": "error",
            "error": format!("Unknown action '{}'. Use: balance, activity, summary", action),
        })
        .to_string(),
    }
}

/// Fetch credits and return formatted JSON string.
async fn fetch_openrouter_credits(http: &reqwest::Client, api_key: &str, base: &str) -> String {
    match fetch_openrouter_credits_raw(http, api_key, base).await {
        Ok((total, used)) => serde_json::json!({
            "status": "ok",
            "total_credits": total,
            "total_usage": used,
            "remaining": total - used,
        })
        .to_string(),
        Err(e) => serde_json::json!({"status": "error", "error": e}).to_string(),
    }
}

/// Fetch credits raw values: (total_credits, total_usage).
async fn fetch_openrouter_credits_raw(
    http: &reqwest::Client,
    api_key: &str,
    base: &str,
) -> std::result::Result<(f64, f64), String> {
    let resp = http
        .get(format!("{base}/credits"))
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("credits request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("credits API returned {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("credits JSON parse failed: {e}"))?;

    let total = body["data"]["total_credits"].as_f64().unwrap_or(0.0);
    let used = body["data"]["total_usage"].as_f64().unwrap_or(0.0);
    Ok((total, used))
}

/// Fetch activity and return formatted JSON string.
async fn fetch_openrouter_activity(
    http: &reqwest::Client,
    api_key: &str,
    base: &str,
    date_filter: Option<&str>,
) -> String {
    match fetch_openrouter_activity_raw(http, api_key, base, date_filter).await {
        Ok(entries) => summarize_openrouter_activity(&entries, date_filter).to_string(),
        Err(e) => serde_json::json!({"status": "error", "error": e}).to_string(),
    }
}

fn summarize_openrouter_activity(entries: &[Value], date_filter: Option<&str>) -> Value {
    #[derive(Default)]
    struct Aggregate {
        model_display: String,
        provider_name: Option<String>,
        cost: f64,
        requests: i64,
        prompt: i64,
        completion: i64,
        reasoning: i64,
        byok_usage: f64,
    }

    let mut by_model: std::collections::HashMap<String, Aggregate> =
        std::collections::HashMap::new();
    let mut by_day: std::collections::HashMap<String, Aggregate> = std::collections::HashMap::new();

    for entry in entries {
        let model_key = entry
            .get("model_permaslug")
            .or_else(|| entry.get("model"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();
        let model_display = entry
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&model_key)
            .to_string();
        let provider_name = entry
            .get("provider_name")
            .and_then(|p| p.as_str())
            .map(str::to_string);
        let date = entry
            .get("date")
            .and_then(|d| d.as_str())
            .unwrap_or("unknown")
            .to_string();
        let cost = entry.get("usage").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let byok_usage = entry
            .get("byok_usage_inference")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let requests = entry.get("requests").and_then(|v| v.as_i64()).unwrap_or(0);
        let prompt = entry
            .get("prompt_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let completion = entry
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let reasoning = entry
            .get("reasoning_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let model = by_model.entry(model_key).or_default();
        if model.model_display.is_empty() {
            model.model_display = model_display;
            model.provider_name = provider_name.clone();
        }
        model.cost += cost;
        model.byok_usage += byok_usage;
        model.requests += requests;
        model.prompt += prompt;
        model.completion += completion;
        model.reasoning += reasoning;

        let day = by_day.entry(date).or_default();
        day.cost += cost;
        day.byok_usage += byok_usage;
        day.requests += requests;
        day.prompt += prompt;
        day.completion += completion;
        day.reasoning += reasoning;
    }

    let mut models: Vec<Value> = by_model
        .iter()
        .map(|(model, agg)| {
            let estimates = crate::llm_pricing::estimate_visible_provider_cost(
                model,
                agg.prompt,
                agg.completion,
                agg.reasoning,
            );
            let (prompt_completion_cost, with_reasoning_cost, unexplained_delta) =
                if let Some((prompt_completion, with_reasoning)) = estimates {
                    (
                        Some(prompt_completion),
                        Some(with_reasoning),
                        Some(agg.cost - with_reasoning),
                    )
                } else {
                    (None, None, None)
                };
            serde_json::json!({
                "model": model,
                "model_display": agg.model_display,
                "provider_name": agg.provider_name,
                "total_cost_usd": agg.cost,
                "provider_usage_usd": agg.cost,
                "byok_usage_usd": agg.byok_usage,
                "requests": agg.requests,
                "prompt_tokens": agg.prompt,
                "completion_tokens": agg.completion,
                "reasoning_tokens": agg.reasoning,
                "visible_prompt_completion_cost_usd": prompt_completion_cost,
                "visible_with_reasoning_cost_usd": with_reasoning_cost,
                "unexplained_provider_cost_delta_usd": unexplained_delta,
                "cost_note": "provider_usage_usd is authoritative. visible_* fields are estimates from public token pricing; unexplained delta can include cache/write/search/provider adjustments or pricing fields not exposed in activity.",
            })
        })
        .collect();
    models.sort_by(|a, b| {
        b["total_cost_usd"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["total_cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut days: Vec<Value> = by_day
        .iter()
        .map(|(date, agg)| {
            serde_json::json!({
                "date": date,
                "total_cost_usd": agg.cost,
                "provider_usage_usd": agg.cost,
                "byok_usage_usd": agg.byok_usage,
                "requests": agg.requests,
                "prompt_tokens": agg.prompt,
                "completion_tokens": agg.completion,
                "reasoning_tokens": agg.reasoning,
            })
        })
        .collect();
    days.sort_by(|a, b| {
        a["date"]
            .as_str()
            .unwrap_or("")
            .cmp(b["date"].as_str().unwrap_or(""))
    });

    let total_cost: f64 = by_model.values().map(|v| v.cost).sum();
    let total_reqs: i64 = by_model.values().map(|v| v.requests).sum();
    let total_prompt: i64 = by_model.values().map(|v| v.prompt).sum();
    let total_completion: i64 = by_model.values().map(|v| v.completion).sum();
    let total_reasoning: i64 = by_model.values().map(|v| v.reasoning).sum();

    serde_json::json!({
        "status": "ok",
        "period": date_filter.unwrap_or("last 30 days"),
        "total_cost_usd": total_cost,
        "total_requests": total_reqs,
        "total_prompt_tokens": total_prompt,
        "total_completion_tokens": total_completion,
        "total_reasoning_tokens": total_reasoning,
        "models": models,
        "by_day": days,
        "interpretation_warning": "Do not recompute provider total only from prompt/completion tokens. Use provider_usage_usd as the billed amount and show visible_with_reasoning_cost_usd/unexplained_provider_cost_delta_usd when explaining differences.",
    })
}

/// Fetch activity raw entries.
async fn fetch_openrouter_activity_raw(
    http: &reqwest::Client,
    api_key: &str,
    base: &str,
    date_filter: Option<&str>,
) -> std::result::Result<Vec<Value>, String> {
    let mut url = format!("{base}/activity");
    if let Some(date) = date_filter {
        url.push_str(&format!("?date={date}"));
    }

    let resp = http
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("activity request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Read body for detailed error message from OpenRouter
        let body_text = resp.text().await.unwrap_or_default();
        let detail = if let Ok(v) = serde_json::from_str::<Value>(&body_text) {
            v.get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| body_text.chars().take(200).collect())
        } else {
            body_text.chars().take(200).collect()
        };
        return Err(format!(
            "activity API returned {} — {} (提示：activity 端点可能需要 Provisioning Key，\
             普通 API Key 只能查询 balance。可在 openrouter.ai/settings/keys 查看)",
            status, detail
        ));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("activity JSON parse failed: {e}"))?;

    match body.get("data").and_then(|d| d.as_array()) {
        Some(arr) => Ok(arr.clone()),
        None => Ok(vec![]),
    }
}

/// RFC 822 header-value CRLF guard.
///
/// Returns `true` when `s` contains a carriage-return (`\r`) or line-feed
/// (`\n`) byte. RFC 822 / RFC 5322 headers are CRLF-terminated, so any raw
/// CR or LF inside a header value lets an attacker forge additional headers
/// (hidden `Bcc`, forged `From`/`Return-Path`, `X-*` markers that downstream
/// filters key on, etc.).
///
/// Canonical CRLF guard for header-like values (today: calendar attendee
/// addresses in `orchestrator_tools_calendar`).
///
/// Bodies (the content after the header/body separator) are NOT checked —
/// newlines there are legitimate message text.
pub(crate) fn rfc822_header_has_crlf(s: &str) -> bool {
    s.bytes().any(|b| b == b'\r' || b == b'\n')
}

/// Base64url encode (RFC 4648 Section 5) without padding.
/// Kept as a small dependency-free utility.
pub fn base64url_encode(input: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 2 < input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        result.push(CHARS[((n >> 6) & 63) as usize] as char);
        result.push(CHARS[(n & 63) as usize] as char);
        i += 3;
    }
    let remaining = input.len() - i;
    if remaining == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        result.push(CHARS[((n >> 6) & 63) as usize] as char);
    } else if remaining == 1 {
        let n = (input[i] as u32) << 16;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activity_summary_preserves_reasoning_and_cost_delta() {
        let entries = vec![
            json!({
                "date": "2026-04-27 00:00:00",
                "model_permaslug": "x-ai/grok-4.1-fast",
                "model": "x-ai/grok-4.1-fast",
                "provider_name": "xai",
                "usage": 48.680871,
                "requests": 10714,
                "prompt_tokens": 121447935,
                "completion_tokens": 13427077,
                "reasoning_tokens": 10461260,
                "byok_usage_inference": 0.0
            }),
            json!({
                "date": "2026-04-27 00:00:00",
                "model_permaslug": "google/gemini-embedding-001",
                "model": "google/gemini-embedding-001",
                "usage": 0.545739,
                "requests": 78885,
                "prompt_tokens": 3866838,
                "completion_tokens": 0,
                "reasoning_tokens": 0
            }),
        ];

        let summary = summarize_openrouter_activity(&entries, None);
        assert_eq!(summary["total_reasoning_tokens"], 10_461_260);
        assert_eq!(summary["by_day"][0]["reasoning_tokens"], 10_461_260);

        let grok = summary["models"]
            .as_array()
            .expect("models array")
            .iter()
            .find(|m| m["model"] == "x-ai/grok-4.1-fast")
            .expect("grok row");

        assert_eq!(grok["reasoning_tokens"], 10_461_260);
        assert!((grok["provider_usage_usd"].as_f64().unwrap() - 48.680871).abs() < 0.000001);
        assert!(
            (grok["visible_prompt_completion_cost_usd"].as_f64().unwrap() - 31.0031255).abs()
                < 0.000001
        );
        assert!(
            grok["unexplained_provider_cost_delta_usd"]
                .as_f64()
                .unwrap()
                > 12.0,
            "provider usage must retain the unexplained delta instead of hiding it"
        );
    }

    // ── base64url_encode: empty input ───────────────────────────────────

    #[test]
    fn base64url_encode_empty_input() {
        assert_eq!(base64url_encode(b""), "");
    }

    // ── base64url_encode: single byte (remainder == 1) ──────────────────

    #[test]
    fn base64url_encode_single_byte() {
        // 'f' = 0x66 → base64url "Zg" (no padding)
        assert_eq!(base64url_encode(b"f"), "Zg");
    }

    // ── base64url_encode: two bytes (remainder == 2) ────────────────────

    #[test]
    fn base64url_encode_two_bytes() {
        // 'fo' → base64url "Zm8" (no padding)
        assert_eq!(base64url_encode(b"fo"), "Zm8");
    }

    // ── base64url_encode: three bytes (exact multiple of 3) ─────────────

    #[test]
    fn base64url_encode_three_bytes() {
        // 'foo' → base64url "Zm9v"
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
    }

    // ── base64url_encode: RFC 4648 test vectors ─────────────────────────

    #[test]
    fn base64url_encode_rfc4648_vectors() {
        // Standard test vectors from RFC 4648 (without padding)
        assert_eq!(base64url_encode(b""), "");
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
        assert_eq!(base64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(base64url_encode(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_encode(b"foobar"), "Zm9vYmFy");
    }

    // ── base64url_encode: uses URL-safe chars (- and _ instead of + /) ──

    #[test]
    fn base64url_encode_uses_url_safe_alphabet() {
        // Bytes that produce + and / in standard base64 should use - and _
        // 0xFB, 0xFF, 0xFE → standard base64 "u//+" → base64url "u__-"
        let input: &[u8] = &[0xFB, 0xFF, 0xFE];
        let result = base64url_encode(input);
        assert!(
            !result.contains('+'),
            "base64url must not contain '+', got: {result}"
        );
        assert!(
            !result.contains('/'),
            "base64url must not contain '/', got: {result}"
        );
    }

    #[test]
    fn base64url_encode_no_padding() {
        // base64url should never have '=' padding chars
        for input in &[b"a".as_slice(), b"ab", b"abc", b"abcd", b"abcde"] {
            let result = base64url_encode(input);
            assert!(
                !result.contains('='),
                "base64url must not contain padding '=', input len={}, got: {result}",
                input.len()
            );
        }
    }

    // ── base64url_encode: binary data ───────────────────────────────────

    #[test]
    fn base64url_encode_binary_data() {
        let input: Vec<u8> = (0u8..=255).collect();
        let result = base64url_encode(&input);
        // 256 bytes → ceil(256/3)*4 = 344 chars (minus padding adjustment)
        // remainder 256 % 3 = 1 → 2 chars instead of 4 for last group
        // (85 * 4) + 2 = 342
        assert_eq!(result.len(), 342);
        // Must only contain base64url-safe characters
        assert!(
            result
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    // ── base64url_encode: realistic Google-like MIME message ─────────────

    #[test]
    fn base64url_encode_realistic_email_content() {
        let email = b"From: user@example.com\r\nTo: recipient@example.com\r\n\
                       Subject: Test\r\n\r\nHello, world!";
        let encoded = base64url_encode(email);
        assert!(!encoded.is_empty());
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    // ── base64url_encode: UTF-8 bytes ───────────────────────────────────

    #[test]
    fn base64url_encode_utf8_bytes() {
        // Chinese characters as UTF-8 bytes
        let input = "\u{4f60}\u{597d}\u{4e16}\u{754c}".as_bytes(); // "hello world" in Chinese
        let result = base64url_encode(input);
        assert!(!result.is_empty());
        assert!(
            result
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    // ── base64url_encode: deterministic ─────────────────────────────────

    #[test]
    fn base64url_encode_is_deterministic() {
        let input = b"deterministic test data 12345";
        let a = base64url_encode(input);
        let b = base64url_encode(input);
        assert_eq!(a, b, "Same input must produce same output");
    }

    // ── rfc822_header_has_crlf: canonical helper ────────────────────────

    #[test]
    fn rfc822_header_has_crlf_clean_string_is_false() {
        assert!(!rfc822_header_has_crlf(""));
        assert!(!rfc822_header_has_crlf("user@example.com"));
        assert!(!rfc822_header_has_crlf("Subject line with spaces"));
        assert!(!rfc822_header_has_crlf("\u{4f60}\u{597d}"));
    }

    #[test]
    fn rfc822_header_has_crlf_rejects_bare_cr() {
        assert!(rfc822_header_has_crlf("victim@x.com\rBcc: attacker@y.com"));
    }

    #[test]
    fn rfc822_header_has_crlf_rejects_bare_lf() {
        assert!(rfc822_header_has_crlf("victim@x.com\nBcc: attacker@y.com"));
    }

    #[test]
    fn rfc822_header_has_crlf_rejects_crlf_pair() {
        assert!(rfc822_header_has_crlf(
            "victim@x.com\r\nBcc: attacker@y.com"
        ));
    }

    #[test]
    fn rfc822_header_has_crlf_rejects_crlf_anywhere() {
        assert!(rfc822_header_has_crlf("leading\rtext"));
        assert!(rfc822_header_has_crlf("trailing text\n"));
        assert!(rfc822_header_has_crlf("\nleading"));
    }
}
