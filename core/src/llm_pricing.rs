//! LLM pricing engine: per-call cost calculation with 3-tier lookup.
//!
//! Tier 1: DB table `llm_pricing` (populated by API fetch)
//! Tier 2: Builtin hardcoded map (known models)
//! Tier 3: None -> cost_usd = NULL (honest "unknown")

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde_json::Value;

use crate::memory::{BackfillResult, CostCoverage};

/// Per-token pricing for a model (USD per token).
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub prompt_per_token: f64,
    pub completion_per_token: f64,
    pub cache_read_per_token: f64,
    pub cache_write_per_token: f64,
    pub image_per_token: f64,
    pub web_search_cost: f64,
    pub request_cost: f64,
}

impl Default for ModelPricing {
    fn default() -> Self {
        Self {
            prompt_per_token: 0.0,
            completion_per_token: 0.0,
            cache_read_per_token: 0.0,
            cache_write_per_token: 0.0,
            image_per_token: 0.0,
            web_search_cost: 0.0,
            request_cost: 0.0,
        }
    }
}

static PRICING_CACHE: OnceLock<Arc<RwLock<HashMap<String, ModelPricing>>>> = OnceLock::new();

const PRICING_STATE_SKILL: &str = "_llm_pricing";
const LAST_REFRESH_SUCCESS_KEY: &str = "last_refresh_success";
const PRICING_REFRESH_CHECK_SECS: u64 = 3_600;
const PRICING_REFRESH_INITIAL_DELAY_SECS: u64 = 30;
const PRICING_REFRESH_INTERVAL_SECS: i64 = 24 * 3_600;
const PRICING_UNPRICED_RETRY_SECS: i64 = 3_600;

fn cache() -> &'static Arc<RwLock<HashMap<String, ModelPricing>>> {
    PRICING_CACHE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

fn routed_budget_alert_destination(
    admin_chat_id: i64,
) -> (teloxide::types::ChatId, crate::topic_router::TopicKind) {
    let notification_class =
        crate::telegram_notification_router::NotificationClass::RoutineNotification;
    let route = crate::telegram_notification_router::route_telegram_notification(
        admin_chat_id,
        crate::telegram_notification_router::proactive_route_request(
            notification_class,
            crate::telegram_notification_router::PrivacyLevel::SystemOnly,
            None,
        ),
    );
    (
        teloxide::types::ChatId(route.chat_id),
        route.topic_kind.unwrap_or_else(|| {
            crate::telegram_notification_router::topic_for_class(notification_class)
        }),
    )
}

/// Normalize model name: strip OpenRouter routing suffixes.
fn normalize_model(model: &str) -> &str {
    model
        .strip_suffix(":online")
        .or_else(|| model.strip_suffix(":free"))
        .or_else(|| model.strip_suffix(":extended"))
        .unwrap_or(model)
}

/// Some providers append dated snapshot suffixes to a stable billing model
/// identifier, for example `google/gemini-3.1-flash-lite-20260507` while the
/// pricing table exposes `google/gemini-3.1-flash-lite`.
fn strip_dated_snapshot_suffix(model: &str) -> Option<&str> {
    let (base, suffix) = model.rsplit_once('-')?;
    if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) && base.contains('/') {
        Some(base)
    } else {
        None
    }
}

fn pricing_lookup_candidates(model: &str) -> Vec<&str> {
    let normalized = normalize_model(model);
    let mut candidates = vec![normalized];
    if let Some(base) = strip_dated_snapshot_suffix(normalized)
        && base != normalized
    {
        candidates.push(base);
    }
    candidates
}

/// Calculate cost for a single LLM call.
/// Returns None if model pricing is unknown (cost_usd will be NULL in DB).
///
/// Includes: prompt tokens, cached input tokens, completion tokens, flat request cost.
/// Excludes: web_search_cost (no web_searches_count available from API response).
/// Web search costs will be added in Phase 3 when the API response parser is extended.
pub fn calculate_cost(
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
    cached_tokens: u32,
) -> Option<f64> {
    let p = lookup_pricing(model)?;
    let uncached_input = prompt_tokens.saturating_sub(cached_tokens) as f64;
    let cached_input = cached_tokens as f64;
    let output = completion_tokens as f64;
    Some(
        uncached_input * p.prompt_per_token
            + cached_input * p.cache_read_per_token
            + output * p.completion_per_token
            + p.request_cost,
    )
}

/// Return true for owner-authorized subscription providers that have no
/// marginal API-credit cost inside Aegis Runtime. They still consume provider
/// quota, but should not be reported as missing OpenRouter/API pricing.
pub fn is_zero_marginal_subscription_usage(provider: Option<&str>, model: &str) -> bool {
    let provider = provider.unwrap_or_default().trim().to_ascii_lowercase();
    matches!(
        provider.as_str(),
        "codex_oauth" | "xai_subscription" | "grok_subscription"
    ) || (provider.is_empty() && (model.starts_with("grok-") || model.starts_with("gpt-5.5")))
}

/// Calculate cost while preserving the distinction between unknown pricing and
/// known zero-marginal subscription usage.
pub fn calculate_cost_for_provider(
    provider: Option<&str>,
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
    cached_tokens: u32,
) -> Option<f64> {
    if is_zero_marginal_subscription_usage(provider, model) {
        return Some(0.0);
    }
    calculate_cost(model, prompt_tokens, completion_tokens, cached_tokens)
}

/// Estimate provider-side visible token cost from public prompt/completion
/// pricing. Reasoning tokens are commonly charged like output tokens by
/// providers that expose them separately, but this is still an estimate; the
/// provider `usage` field remains the source of truth.
pub(crate) fn estimate_visible_provider_cost(
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
    reasoning_tokens: i64,
) -> Option<(f64, f64)> {
    let p = lookup_pricing(model)?;
    let prompt = prompt_tokens.max(0) as f64;
    let completion = completion_tokens.max(0) as f64;
    let reasoning = reasoning_tokens.max(0) as f64;
    let prompt_completion =
        prompt * p.prompt_per_token + completion * p.completion_per_token + p.request_cost;
    let with_reasoning = prompt_completion + reasoning * p.completion_per_token;
    Some((prompt_completion, with_reasoning))
}

/// Lookup pricing: in-memory cache -> builtin fallback -> None.
fn lookup_pricing(model: &str) -> Option<ModelPricing> {
    let candidates = pricing_lookup_candidates(model);

    // Tier 1: in-memory cache (populated from DB)
    {
        let cache = cache().read();
        for candidate in &candidates {
            if let Some(p) = cache.get(*candidate) {
                return Some(p.clone());
            }
        }
    }

    // Tier 2: builtin hardcoded fallback
    for candidate in &candidates {
        if let Some(p) = builtin_pricing(candidate) {
            return Some(p);
        }
    }
    None
}

/// Hardcoded pricing for known models (cold-start fallback).
/// Prices sourced from OpenRouter API on 2026-04-11.
fn builtin_pricing(model: &str) -> Option<ModelPricing> {
    match model {
        "x-ai/grok-4.1-fast" => Some(ModelPricing {
            prompt_per_token: 0.0000002,
            completion_per_token: 0.0000005,
            cache_read_per_token: 0.00000005,
            web_search_cost: 0.005,
            ..Default::default()
        }),
        "google/gemini-3.1-flash-lite-preview" => Some(ModelPricing {
            prompt_per_token: 0.00000025,
            completion_per_token: 0.0000015,
            cache_read_per_token: 0.000000025,
            ..Default::default()
        }),
        "google/gemini-2.5-flash-preview" => Some(ModelPricing {
            prompt_per_token: 0.00000015,
            completion_per_token: 0.0000006,
            cache_read_per_token: 0.0000000375,
            ..Default::default()
        }),
        "openai/gpt-4.1-mini" => Some(ModelPricing {
            prompt_per_token: 0.0000004,
            completion_per_token: 0.0000016,
            cache_read_per_token: 0.0000001,
            ..Default::default()
        }),
        "openai/gpt-4.1-nano" => Some(ModelPricing {
            prompt_per_token: 0.0000001,
            completion_per_token: 0.0000004,
            cache_read_per_token: 0.000000025,
            ..Default::default()
        }),
        _ => None,
    }
}

/// Load pricing from DB into in-memory cache.
pub fn load_pricing_from_db(conn: &rusqlite::Connection) -> usize {
    let mut stmt = match conn.prepare(
        "SELECT model, prompt_per_token, completion_per_token, cache_read_per_token,
                cache_write_per_token, image_per_token, web_search_cost, request_cost
         FROM llm_pricing",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "[Pricing] Failed to query llm_pricing table");
            return 0;
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ModelPricing {
                prompt_per_token: row.get(1)?,
                completion_per_token: row.get(2)?,
                cache_read_per_token: row.get(3)?,
                cache_write_per_token: row.get(4)?,
                image_per_token: row.get(5)?,
                web_search_cost: row.get(6)?,
                request_cost: row.get(7)?,
            },
        ))
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "[Pricing] Failed to query llm_pricing rows");
            return 0;
        }
    };

    let mut cache = cache().write();
    let mut count = 0;
    for row in rows.flatten() {
        cache.insert(row.0, row.1);
        count += 1;
    }
    if count > 0 {
        tracing::info!(models = count, "[Pricing] Loaded pricing from DB");
    }
    count
}

/// Upsert a model's pricing into the DB and refresh the in-memory cache.
pub fn upsert_pricing(
    conn: &rusqlite::Connection,
    model: &str,
    pricing: &ModelPricing,
    source: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO llm_pricing (model, prompt_per_token, completion_per_token,
         cache_read_per_token, cache_write_per_token, image_per_token,
         web_search_cost, request_cost, source, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
         ON CONFLICT(model) DO UPDATE SET
           prompt_per_token = excluded.prompt_per_token,
           completion_per_token = excluded.completion_per_token,
           cache_read_per_token = excluded.cache_read_per_token,
           cache_write_per_token = excluded.cache_write_per_token,
           image_per_token = excluded.image_per_token,
           web_search_cost = excluded.web_search_cost,
           request_cost = excluded.request_cost,
           source = excluded.source,
           updated_at = datetime('now')",
        rusqlite::params![
            model,
            pricing.prompt_per_token,
            pricing.completion_per_token,
            pricing.cache_read_per_token,
            pricing.cache_write_per_token,
            pricing.image_per_token,
            pricing.web_search_cost,
            pricing.request_cost,
            source,
        ],
    )?;

    // Update in-memory cache
    cache().write().insert(model.to_string(), pricing.clone());
    Ok(())
}

/// Fetch pricing from OpenRouter's live `/models` endpoint.
///
/// The endpoint returns dollar-per-token strings under `pricing`. Unknown
/// fields are ignored so this remains compatible with provider-side additions.
pub async fn fetch_pricing_from_openrouter(
    http: &reqwest::Client,
    api_base: &str,
) -> Result<Vec<(String, ModelPricing)>> {
    let base = if api_base.contains("openrouter.ai") {
        "https://openrouter.ai/api/v1".to_string()
    } else {
        api_base.trim_end_matches('/').to_string()
    };
    let resp = http
        .get(format!("{base}/models"))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("OpenRouter models request failed")?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .context("OpenRouter models response read failed")?;
    if !status.is_success() {
        anyhow::bail!(
            "OpenRouter models API returned {status}: {}",
            text.chars().take(300).collect::<String>()
        );
    }

    let body: Value =
        serde_json::from_str(&text).context("OpenRouter models response JSON parse failed")?;
    let Some(models) = body.get("data").and_then(Value::as_array) else {
        anyhow::bail!("OpenRouter models response missing data array");
    };

    let mut rows = Vec::new();
    for model in models {
        let Some(id) = model
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(pricing) = model.get("pricing").and_then(Value::as_object) else {
            continue;
        };
        let p = ModelPricing {
            prompt_per_token: parse_price_field(id, pricing.get("prompt"), "prompt"),
            completion_per_token: parse_price_field(id, pricing.get("completion"), "completion"),
            cache_read_per_token: parse_price_field(
                id,
                pricing.get("input_cache_read"),
                "input_cache_read",
            ),
            cache_write_per_token: parse_price_field(
                id,
                pricing.get("input_cache_write"),
                "input_cache_write",
            ),
            image_per_token: parse_price_field(id, pricing.get("image"), "image"),
            web_search_cost: parse_price_field(id, pricing.get("web_search"), "web_search"),
            request_cost: parse_price_field(id, pricing.get("request"), "request"),
        };
        rows.push((id.to_string(), p));
    }

    Ok(rows)
}

/// Upsert fetched OpenRouter pricing rows. Kept synchronous so SQLite handles
/// never cross async await points and do not poison spawned futures' `Send`.
pub fn upsert_pricing_rows(
    conn: &rusqlite::Connection,
    rows: &[(String, ModelPricing)],
    source: &str,
) -> Result<usize> {
    for (id, pricing) in rows {
        upsert_pricing(conn, id, pricing, source)
            .with_context(|| format!("upserting OpenRouter pricing for {id}"))?;
    }
    tracing::info!(
        models = rows.len(),
        source,
        "[Pricing] Upserted pricing rows"
    );
    Ok(rows.len())
}

#[derive(Debug, Clone)]
pub struct PricingRefreshResult {
    pub models_updated: usize,
    pub rows_backfilled: usize,
    pub rows_still_null: usize,
    pub models_missing: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PricingRefreshDecision {
    Refresh(&'static str),
    Skip(&'static str),
}

fn pricing_refresh_decision(
    coverage: &CostCoverage,
    last_success: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> PricingRefreshDecision {
    if coverage.pricing_rows == 0 && coverage.api_row_count > 0 {
        return PricingRefreshDecision::Refresh("pricing_table_empty");
    }

    let seconds_since_success = last_success
        .map(|ts| now.signed_duration_since(ts).num_seconds())
        .unwrap_or(i64::MAX);

    if coverage.api_null_cost_rows > 0 && seconds_since_success >= PRICING_UNPRICED_RETRY_SECS {
        return PricingRefreshDecision::Refresh("api_unpriced_rows_present");
    }

    if seconds_since_success >= PRICING_REFRESH_INTERVAL_SECS {
        return PricingRefreshDecision::Refresh("scheduled_daily_refresh");
    }

    PricingRefreshDecision::Skip("pricing_recent_enough")
}

fn parse_refresh_success_ts(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| {
        DateTime::parse_from_rfc3339(&value)
            .ok()
            .map(|ts| ts.with_timezone(&Utc))
    })
}

pub async fn refresh_openrouter_pricing_and_backfill(
    memory: &crate::memory::MemoryHandle,
    http: &reqwest::Client,
    api_base: &str,
    db_path: &str,
) -> Result<PricingRefreshResult> {
    let rows = fetch_pricing_from_openrouter(http, api_base).await?;
    let models_updated = {
        let conn = crate::encrypted_sqlite::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening pricing DB at {db_path}"))?;
        upsert_pricing_rows(&conn, &rows, "openrouter_models_api")?
    };
    let BackfillResult {
        rows_updated,
        rows_still_null,
        models_missing,
    } = memory.backfill_cost_usd().await?;
    memory
        .store_skill_state(
            PRICING_STATE_SKILL,
            LAST_REFRESH_SUCCESS_KEY,
            &Utc::now().to_rfc3339(),
        )
        .await
        .ok();

    Ok(PricingRefreshResult {
        models_updated,
        rows_backfilled: rows_updated,
        rows_still_null,
        models_missing,
    })
}

/// Keep OpenRouter/API pricing self-maintaining. This task is deterministic
/// and low-cost: it performs an hourly DB coverage check and only calls the
/// OpenRouter models API when the local cache is empty, unpriced API rows are
/// present after the retry window, or the daily refresh interval elapsed.
pub fn spawn_pricing_refresh_task(
    memory: crate::memory::MemoryHandle,
    http: reqwest::Client,
    api_base: String,
    db_path: String,
) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(
            PRICING_REFRESH_INITIAL_DELAY_SECS,
        ))
        .await;

        loop {
            let coverage = match memory.query_cost_coverage(2).await {
                Ok(coverage) => coverage,
                Err(e) => {
                    tracing::debug!(error = %e, "[Pricing] Coverage query failed");
                    tokio::time::sleep(std::time::Duration::from_secs(PRICING_REFRESH_CHECK_SECS))
                        .await;
                    continue;
                }
            };
            let last_success = parse_refresh_success_ts(
                memory
                    .load_skill_state(PRICING_STATE_SKILL, LAST_REFRESH_SUCCESS_KEY)
                    .await
                    .ok()
                    .flatten(),
            );
            let decision = pricing_refresh_decision(&coverage, last_success, Utc::now());

            match decision {
                PricingRefreshDecision::Refresh(reason) => {
                    match refresh_openrouter_pricing_and_backfill(
                        &memory, &http, &api_base, &db_path,
                    )
                    .await
                    {
                        Ok(result) => tracing::info!(
                            reason,
                            models_updated = result.models_updated,
                            rows_backfilled = result.rows_backfilled,
                            rows_still_null = result.rows_still_null,
                            missing_models = result.models_missing.len(),
                            "[Pricing] Refreshed OpenRouter pricing and backfilled costs"
                        ),
                        Err(e) => tracing::warn!(
                            reason,
                            error = %e,
                            "[Pricing] OpenRouter pricing refresh failed"
                        ),
                    }
                }
                PricingRefreshDecision::Skip(reason) => {
                    tracing::debug!(
                        reason,
                        pricing_rows = coverage.pricing_rows,
                        api_null_cost_rows = coverage.api_null_cost_rows,
                        "[Pricing] Refresh skipped"
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(PRICING_REFRESH_CHECK_SECS)).await;
        }
    });
}

fn parse_price_field(model_id: &str, value: Option<&Value>, field: &str) -> f64 {
    match value {
        Some(Value::String(s)) => match s.parse::<f64>() {
            Ok(price) => price,
            Err(e) => {
                tracing::warn!(
                    model = %model_id,
                    field,
                    value = %s,
                    error = %e,
                    "[Pricing] Invalid OpenRouter pricing value"
                );
                0.0
            }
        },
        Some(Value::Number(n)) => n.as_f64().unwrap_or_else(|| {
            tracing::warn!(
                model = %model_id,
                field,
                value = %n,
                "[Pricing] Non-f64 OpenRouter pricing number"
            );
            0.0
        }),
        _ => 0.0,
    }
}

/// Spawn a background task that checks LLM spending against budget thresholds
/// every 15 minutes and sends Telegram alerts at 80% and 100% budget usage.
pub fn spawn_budget_monitor(
    memory: crate::memory::MemoryHandle,
    settings: std::sync::Arc<crate::settings::RuntimeSettings>,
    bot: teloxide::Bot,
    admin_chat_id: i64,
) {
    tokio::spawn(async move {
        use std::time::Duration;
        // Track which alerts were already sent today to avoid spam
        let mut last_alert_date = String::new();
        let mut alerted_periods: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        loop {
            tokio::time::sleep(Duration::from_secs(900)).await;

            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            if today != last_alert_date {
                last_alert_date = today;
                alerted_periods.clear();
            }

            let daily = match memory.query_daily_cost_totals(30).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(error = %e, "[BudgetMonitor] Failed to query cost totals");
                    continue;
                }
            };

            // Calculate period totals
            let today_cost: f64 = daily.last().map(|d| d.cost).unwrap_or(0.0);
            let week_cost: f64 = daily.iter().rev().take(7).map(|d| d.cost).sum();
            let month_cost: f64 = daily.iter().map(|d| d.cost).sum();

            let budget_daily = settings
                .get("budget_daily_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(5.0);
            let budget_weekly = settings
                .get("budget_weekly_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(25.0);
            let budget_monthly = settings
                .get("budget_monthly_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(80.0);

            // Check each budget threshold (100% critical, 80% warning)
            let checks: &[(&str, f64, f64)] = &[
                ("Daily", today_cost, budget_daily),
                ("Weekly", week_cost, budget_weekly),
                ("Monthly", month_cost, budget_monthly),
            ];

            for (period, spent, budget) in checks {
                if *budget <= 0.0 {
                    continue;
                }
                let pct = spent / budget * 100.0;
                let alert_key = format!(
                    "{period}_{}",
                    if pct >= 100.0 { "critical" } else { "warning" }
                );

                if pct >= 100.0 && !alerted_periods.contains(&alert_key) {
                    let msg = format!(
                        "[预算超限 Budget CRITICAL] {} API 花费 ${:.4} 已超过预算 ${:.2} ({:.0}%)",
                        period, spent, budget, pct
                    );
                    tracing::warn!("{msg}");
                    if crate::telegram::record_owner_web_notice_if_push_delivered(
                        &memory,
                        admin_chat_id,
                        &msg,
                    )
                    .await
                    {
                        alerted_periods.insert(alert_key);
                        continue;
                    }
                    let html = crate::telegram::markdown_to_telegram_html(&msg);
                    let (chat_id, topic) = routed_budget_alert_destination(admin_chat_id);
                    if let Err(e) =
                        crate::telegram::send_html_to_topic(&bot, chat_id, topic, &html).await
                    {
                        tracing::warn!(error = %e, "[BudgetMonitor] Failed to send critical alert");
                        continue; // Don't mark as sent so it retries next cycle
                    }
                    alerted_periods.insert(alert_key);
                } else if pct >= 80.0 && !alerted_periods.contains(&alert_key) {
                    let msg = format!(
                        "[预算警告 Budget Warning] {} API 花费 ${:.4} 已达预算 ${:.2} 的 {:.0}%",
                        period, spent, budget, pct
                    );
                    tracing::info!("{msg}");
                    if crate::telegram::record_owner_web_notice_if_push_delivered(
                        &memory,
                        admin_chat_id,
                        &msg,
                    )
                    .await
                    {
                        alerted_periods.insert(alert_key);
                        continue;
                    }
                    let html = crate::telegram::markdown_to_telegram_html(&msg);
                    let (chat_id, topic) = routed_budget_alert_destination(admin_chat_id);
                    if let Err(e) =
                        crate::telegram::send_html_to_topic(&bot, chat_id, topic, &html).await
                    {
                        tracing::warn!(error = %e, "[BudgetMonitor] Failed to send warning alert");
                        continue;
                    }
                    alerted_periods.insert(alert_key);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_cost_known_model() {
        let cost = calculate_cost("x-ai/grok-4.1-fast", 1000, 200, 500);
        assert!(cost.is_some());
        let c = cost.unwrap();
        // uncached: 500 * 0.0000002 = 0.0001
        // cached: 500 * 0.00000005 = 0.000025
        // output: 200 * 0.0000005 = 0.0001
        let expected = 500.0 * 0.0000002 + 500.0 * 0.00000005 + 200.0 * 0.0000005;
        assert!((c - expected).abs() < 1e-12);
    }

    #[test]
    fn calculate_cost_unknown_model_returns_none() {
        assert!(calculate_cost("unknown/model-xyz", 1000, 200, 0).is_none());
    }

    #[test]
    fn normalize_strips_suffixes() {
        assert_eq!(
            normalize_model("x-ai/grok-4.1-fast:online"),
            "x-ai/grok-4.1-fast"
        );
        assert_eq!(normalize_model("model:free"), "model");
        assert_eq!(normalize_model("model:extended"), "model");
        assert_eq!(normalize_model("model"), "model");
    }

    #[test]
    fn pricing_candidates_include_dated_snapshot_base() {
        assert_eq!(
            pricing_lookup_candidates("google/gemini-3.1-flash-lite-20260507"),
            vec![
                "google/gemini-3.1-flash-lite-20260507",
                "google/gemini-3.1-flash-lite"
            ]
        );
        assert_eq!(
            pricing_lookup_candidates("google/gemini-3.1-flash-lite-20260507:online"),
            vec![
                "google/gemini-3.1-flash-lite-20260507",
                "google/gemini-3.1-flash-lite"
            ]
        );
    }

    #[test]
    fn calculate_cost_with_suffix() {
        // :online suffix should be stripped for pricing lookup
        let cost = calculate_cost("x-ai/grok-4.1-fast:online", 1000, 100, 0);
        assert!(cost.is_some());
    }

    #[test]
    fn calculate_cost_uses_base_pricing_for_dated_snapshot_alias() {
        let conn = rusqlite::Connection::open_in_memory().expect("sqlite");
        conn.execute_batch(
            "CREATE TABLE llm_pricing (
                model TEXT PRIMARY KEY,
                prompt_per_token REAL NOT NULL,
                completion_per_token REAL NOT NULL,
                cache_read_per_token REAL NOT NULL DEFAULT 0,
                cache_write_per_token REAL NOT NULL DEFAULT 0,
                image_per_token REAL NOT NULL DEFAULT 0,
                web_search_cost REAL NOT NULL DEFAULT 0,
                request_cost REAL NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT 'test',
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .expect("schema");
        upsert_pricing(
            &conn,
            "test/provider-model",
            &ModelPricing {
                prompt_per_token: 0.000001,
                completion_per_token: 0.000002,
                ..Default::default()
            },
            "test",
        )
        .expect("upsert pricing");

        let cost = calculate_cost("test/provider-model-20260507", 100, 50, 0);
        assert!((cost.expect("cost") - 0.0002).abs() < 1e-12);
    }

    #[test]
    fn subscription_usage_is_zero_marginal_not_unknown() {
        assert_eq!(
            calculate_cost_for_provider(Some("xai_subscription"), "grok-4.3", 10_000, 500, 0),
            Some(0.0)
        );
        assert_eq!(
            calculate_cost_for_provider(Some("codex_oauth"), "gpt-5.5", 10_000, 500, 0),
            Some(0.0)
        );
        assert_eq!(
            calculate_cost_for_provider(None, "provider/unknown-model", 10_000, 500, 0),
            None
        );
    }

    fn coverage_for_refresh_test(
        pricing_rows: i64,
        api_row_count: i64,
        api_null_cost_rows: i64,
    ) -> CostCoverage {
        CostCoverage {
            row_count: api_row_count,
            priced_row_count: api_row_count - api_null_cost_rows,
            null_cost_rows: api_null_cost_rows,
            api_row_count,
            api_priced_row_count: api_row_count - api_null_cost_rows,
            api_null_cost_rows,
            zero_marginal_subscription_rows: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_cached_tokens: 0,
            total_tokens: 0,
            min_timestamp: None,
            max_timestamp: None,
            pricing_rows,
            pricing_sources: 0,
            pricing_min_updated_at: None,
            pricing_max_updated_at: None,
        }
    }

    #[test]
    fn pricing_refresh_decision_repairs_empty_cache_and_throttles_retries() {
        let now = Utc::now();
        assert_eq!(
            pricing_refresh_decision(&coverage_for_refresh_test(0, 3, 3), Some(now), now),
            PricingRefreshDecision::Refresh("pricing_table_empty")
        );
        assert_eq!(
            pricing_refresh_decision(
                &coverage_for_refresh_test(10, 3, 3),
                Some(now - chrono::Duration::minutes(30)),
                now,
            ),
            PricingRefreshDecision::Skip("pricing_recent_enough")
        );
        assert_eq!(
            pricing_refresh_decision(
                &coverage_for_refresh_test(10, 3, 3),
                Some(now - chrono::Duration::hours(2)),
                now,
            ),
            PricingRefreshDecision::Refresh("api_unpriced_rows_present")
        );
        assert_eq!(
            pricing_refresh_decision(
                &coverage_for_refresh_test(10, 0, 0),
                Some(now - chrono::Duration::hours(25)),
                now,
            ),
            PricingRefreshDecision::Refresh("scheduled_daily_refresh")
        );
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let cost = calculate_cost("x-ai/grok-4.1-fast", 0, 0, 0);
        assert_eq!(cost, Some(0.0));
    }

    #[test]
    fn parse_price_field_accepts_openrouter_strings_and_numbers() {
        assert_eq!(
            parse_price_field("m", Some(&serde_json::json!("0.0000015")), "prompt"),
            0.0000015
        );
        assert_eq!(
            parse_price_field("m", Some(&serde_json::json!(0.25)), "request"),
            0.25
        );
        assert_eq!(
            parse_price_field("m", Some(&serde_json::json!("not-a-price")), "prompt"),
            0.0
        );
    }
}
