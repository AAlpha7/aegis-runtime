//! Centralised LLM client factory, retry configuration, and usage logging.
//!
//! Every module that needs an `async_openai::Client<OpenAIConfig>` should call
//! [`create_llm_client`] instead of constructing the config+client inline.
//! This keeps the configuration in one place and makes future changes
//! (custom headers, retry policies, etc.) a single-point edit.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::CreateChatCompletionResponse;

// ── Global DB persistence channel ──────────────────────────────────────────

struct LlmLogEntry {
    label: String,
    model: String,
    provider: Option<String>,
    provider_request_id: Option<String>,
    prompt_tokens: u32,
    completion_tokens: u32,
    cached_tokens: u32,
    total_tokens: u32,
    latency_ms: Option<u64>,
    is_fallback: bool,
}

static LLM_LOG_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<LlmLogEntry>> = OnceLock::new();

/// Set once on the first send failure; used to suppress a log storm when the
/// receiver task is dead and every subsequent call would fail.
static LLM_LOG_CHANNEL_BROKEN: AtomicBool = AtomicBool::new(false);

/// Monotonic counter of dropped `LlmLogEntry` sends.  Operator-visible via
/// [`llm_log_send_failure_count`]; [`crate::self_audit`]'s
/// `llm_log_coverage` check already surfaces the symptom via DB coverage,
/// but this lets ops correlate by exact count if needed.
static LLM_LOG_SEND_FAILURES: AtomicU64 = AtomicU64::new(0);

const PROVIDER_CREDIT_EXHAUSTION_BACKGROUND_COOLDOWN: Duration = Duration::from_secs(15 * 60);

static PROVIDER_CREDIT_EXHAUSTION_BLOCK_UNTIL_SECS: AtomicU64 = AtomicU64::new(0);

/// How many LLM-log channel sends have been dropped since boot.  Useful for
/// tests and future telemetry endpoints.
pub fn llm_log_send_failure_count() -> u64 {
    LLM_LOG_SEND_FAILURES.load(Ordering::Relaxed)
}

pub fn is_provider_credit_exhaustion_error(message: &str) -> bool {
    message.contains("Insufficient credits")
        || message.contains("\"code\":402")
        || message.contains("status 402")
        || message.contains("returned 402")
}

pub fn is_provider_credit_exhaustion_cooldown(message: &str) -> bool {
    message.contains("model provider credits are exhausted")
        && message.contains("background LLM calls are cooling down")
}

pub fn record_provider_credit_exhaustion(label: &str) {
    let until = current_epoch_secs()
        .saturating_add(PROVIDER_CREDIT_EXHAUSTION_BACKGROUND_COOLDOWN.as_secs());
    let previous = PROVIDER_CREDIT_EXHAUSTION_BLOCK_UNTIL_SECS.swap(until, Ordering::SeqCst);
    if previous < current_epoch_secs() {
        tracing::warn!(
            label,
            cooldown_secs = PROVIDER_CREDIT_EXHAUSTION_BACKGROUND_COOLDOWN.as_secs(),
            "[LLM Route] provider credits exhausted; suppressing background LLM calls during cooldown"
        );
    }
}

pub fn provider_credit_exhaustion_suppresses_background() -> bool {
    provider_credit_exhaustion_suppresses_background_at(current_epoch_secs())
}

fn provider_credit_exhaustion_suppresses_background_at(now_secs: u64) -> bool {
    PROVIDER_CREDIT_EXHAUSTION_BLOCK_UNTIL_SECS.load(Ordering::Relaxed) > now_secs
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn reset_provider_credit_exhaustion_for_test() {
    PROVIDER_CREDIT_EXHAUSTION_BLOCK_UNTIL_SECS.store(0, Ordering::SeqCst);
}

/// Attempt to queue an entry for DB persistence.  On failure, increments
/// [`LLM_LOG_SEND_FAILURES`] and logs an ERROR exactly once per process
/// lifetime (subsequent failures only bump the counter to avoid log storm).
fn try_send_log_entry(entry: LlmLogEntry) {
    let Some(tx) = LLM_LOG_TX.get() else {
        // Channel not initialised (unit tests, pre-startup).  Silent skip
        // matches the prior behaviour — not an error.
        return;
    };
    send_or_record_failure(tx, entry, &LLM_LOG_SEND_FAILURES, &LLM_LOG_CHANNEL_BROKEN);
}

/// Testable core of [`try_send_log_entry`].  Takes the channel, counter, and
/// log-once flag as parameters so unit tests can drive them without touching
/// process-wide statics.
fn send_or_record_failure(
    tx: &tokio::sync::mpsc::UnboundedSender<LlmLogEntry>,
    entry: LlmLogEntry,
    failure_counter: &AtomicU64,
    broken_flag: &AtomicBool,
) {
    if let Err(send_err) = tx.send(entry) {
        failure_counter.fetch_add(1, Ordering::Relaxed);
        // Log loudly the first time so operators see the channel is dead
        // and can restart; stay quiet thereafter so the log isn't flooded.
        if !broken_flag.swap(true, Ordering::SeqCst) {
            let dropped = &send_err.0;
            tracing::error!(
                label = %dropped.label,
                model = %dropped.model,
                "[llm_log] send failed — receiver task appears dead; LLM-call \
                 log entries will be dropped until service restart. \
                 (self_audit.llm_log_coverage also surfaces this; operators \
                 should investigate the receiver task and restart.)"
            );
        }
    }
}

/// Register a global channel that persists every [`log_llm_usage`] call to
/// the `llm_call_log` table.  Call once during startup after the memory
/// actor is initialised.
pub fn init_llm_log_db(memory: crate::memory::MemoryHandle) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    if LLM_LOG_TX.set(tx).is_err() {
        return; // already initialised
    }
    tokio::spawn(run_llm_log_receiver(rx, memory));
}

/// The DB-persistence receiver loop, extracted so it can be unit-tested
/// against a real `MemoryHandle` without touching the process-wide
/// `LLM_LOG_TX` `OnceLock`.  Drains `LlmLogEntry`s and INSERTs each into
/// `llm_call_log` via [`crate::memory::MemoryHandle::store_llm_call_log`].
///
/// The loop must NEVER return on a handler error, or the channel appears
/// alive (sender unaware) while entries silently vanish.
/// `store_llm_call_log` logs its own warnings and never panics; if a future
/// handler addition can panic, wrap it in `tokio::spawn` or `catch_unwind`
/// to protect the loop.
async fn run_llm_log_receiver(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<LlmLogEntry>,
    memory: crate::memory::MemoryHandle,
) {
    while let Some(e) = rx.recv().await {
        let cost = crate::llm_pricing::calculate_cost_for_provider(
            e.provider.as_deref(),
            &e.model,
            e.prompt_tokens,
            e.completion_tokens,
            e.cached_tokens,
        );
        memory
            .store_llm_call_log(
                &e.label,
                &e.model,
                e.provider.as_deref(),
                e.provider_request_id.as_deref(),
                e.prompt_tokens,
                e.completion_tokens,
                e.cached_tokens,
                e.total_tokens,
                cost,
                e.latency_ms,
                e.is_fallback,
            )
            .await;
    }
    // If we reach here, every sender was dropped — which in production
    // only happens at process shutdown because the sender lives in a
    // static OnceLock.  Still worth logging so we can detect the
    // unexpected case in dev/test.
    tracing::error!(
        "[llm_log] receiver task exited — all senders dropped. \
         In production this means process shutdown; if seen at runtime, \
         investigate."
    );
}

// ── Shared constants ────────────────────────────────────────────────────────

/// Bare Grok id for subscription web search. Not an OpenRouter slug.
pub const WEB_SEARCH_SUBSCRIPTION_MODEL: &str = "grok-4.7";

/// OpenRouter native-search slug (`:online`). Used only when the owner sets
/// `web_search_provider=openrouter`. The default search route is the Grok
/// subscription (`web_search` module), which does not send this slug.
pub const WEB_SEARCH_MODEL: &str = "x-ai/grok-4.7:online";

/// Models the settings selector offers for one search provider.
pub fn web_search_model_options(provider: &str) -> &'static [&'static str] {
    if matches!(provider, "auto" | "codex_oauth") {
        return &[];
    }
    if provider == "openrouter" {
        &[
            "x-ai/grok-4.7:online",
            "x-ai/grok-4.6:online",
            "x-ai/grok-4.5:online",
        ]
    } else {
        &["grok-4.7", "grok-4.6", "grok-4.5"]
    }
}

/// OpenRouter slug with the native `:online` suffix. Bare ids gain the `x-ai/`
/// vendor prefix. Empty input is the owner's problem; callers substitute the
/// default before calling this.
pub fn openrouter_vendor_online_slug(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.ends_with(":online") {
        return trimmed.to_string();
    }
    if trimmed.contains('/') {
        return format!("{trimmed}:online");
    }
    format!("x-ai/{trimmed}:online")
}

/// Maximum items per buffer before oldest are dropped.
/// Used by bounded in-memory ring buffers.
pub const MAX_BUFFER_SIZE: usize = 200;

/// Cosine similarity threshold above which a new item is considered a duplicate.
/// Used for embedding-based dedup.
pub const DEDUP_SIMILARITY_THRESHOLD: f32 = 0.85;

// ── Factory ─────────────────────────────────────────────────────────────────

/// Create an `async_openai` client configured for the given API endpoint.
///
/// This is the **only** place in the codebase where `OpenAIConfig` is assembled;
/// all callers should use this function instead of constructing their own.
///
/// Retry policy: 2s → 4s → 8s → 16s backoff on 429/5xx, capped at 30s total.
/// Non-retryable errors (413, 400, network failures) propagate immediately.
pub fn create_llm_client(api_key: &str, api_base: &str) -> Client<OpenAIConfig> {
    let config = OpenAIConfig::new()
        .with_api_key(api_key)
        .with_api_base(api_base);

    let backoff = backoff::ExponentialBackoffBuilder::new()
        .with_initial_interval(std::time::Duration::from_secs(2))
        .with_multiplier(2.0)
        .with_max_interval(std::time::Duration::from_secs(16))
        .with_max_elapsed_time(Some(std::time::Duration::from_secs(30)))
        .build();

    Client::with_config(config).with_backoff(backoff)
}

// ── Usage logging ─────────────────────────────────────────────────────────

/// Log token usage from an LLM response, including cache hit statistics.
///
/// Call this after every successful `chat().create()` response to track
/// prompt/completion tokens and cache effectiveness. The `label` parameter
/// identifies the callsite (e.g. "react_round", "autonomous").
/// Hidden reasoning tokens: xAI reports them in `total` but not in
/// `completion`; fall back to the difference when the detail block is absent.
/// Latency tracks this number (~50 tok/s on grok), not prompt size.
pub fn reasoning_tokens(usage: &async_openai::types::chat::CompletionUsage) -> u32 {
    usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            usage
                .total_tokens
                .saturating_sub(usage.prompt_tokens)
                .saturating_sub(usage.completion_tokens)
        })
}

pub fn log_llm_usage(response: &CreateChatCompletionResponse, label: &str) {
    if let Some(usage) = &response.usage {
        let cached = usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0);
        let cache_pct = if usage.prompt_tokens > 0 {
            (cached as f64 / usage.prompt_tokens as f64) * 100.0
        } else {
            0.0
        };
        let reasoning = reasoning_tokens(usage);
        tracing::info!(
            label,
            prompt = usage.prompt_tokens,
            completion = usage.completion_tokens,
            reasoning,
            total = usage.total_tokens,
            cached,
            cache_pct = format!("{:.1}%", cache_pct),
            "[LLM Usage]"
        );
        // Persist to DB via global channel (if registered).
        try_send_log_entry(LlmLogEntry {
            label: label.to_string(),
            model: response.model.clone(),
            provider: provider_from_response(&response.id, &response.model).map(str::to_string),
            provider_request_id: Some(response.id.clone()),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: cached,
            total_tokens: usage.total_tokens,
            latency_ms: None,
            is_fallback: false,
        });
    }
}

/// Log token usage with latency and fallback info (for primary LLM call path).
pub fn log_llm_usage_timed(
    response: &CreateChatCompletionResponse,
    label: &str,
    elapsed: std::time::Duration,
    is_fallback: bool,
) {
    if let Some(usage) = &response.usage {
        let cached = usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0);
        let cache_pct = if usage.prompt_tokens > 0 {
            (cached as f64 / usage.prompt_tokens as f64) * 100.0
        } else {
            0.0
        };
        let latency_ms = elapsed.as_millis() as u64;
        let reasoning = reasoning_tokens(usage);
        tracing::info!(
            label,
            prompt = usage.prompt_tokens,
            completion = usage.completion_tokens,
            reasoning,
            total = usage.total_tokens,
            cached,
            cache_pct = format!("{:.1}%", cache_pct),
            latency_ms,
            is_fallback,
            "[LLM Usage]"
        );
        try_send_log_entry(LlmLogEntry {
            label: label.to_string(),
            model: response.model.clone(),
            provider: provider_from_response(&response.id, &response.model).map(str::to_string),
            provider_request_id: Some(response.id.clone()),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: cached,
            total_tokens: usage.total_tokens,
            latency_ms: Some(latency_ms),
            is_fallback,
        });
    }
}

fn provider_from_response(response_id: &str, model: &str) -> Option<&'static str> {
    if response_id.starts_with("gen-") || response_id.starts_with("gen_") {
        Some("openrouter")
    } else if response_id.starts_with("codex-app-server-") || model.starts_with("gpt-5.5") {
        Some("codex_oauth")
    } else if model.starts_with("grok-") {
        Some("xai_subscription")
    } else {
        None
    }
}

/// Log token usage AND persist to database for observability dashboards.
///
/// Delegates to [`log_llm_usage`] which now handles DB persistence
/// automatically via the global channel registered by [`init_llm_log_db`].
/// The `_model` and `_memory` parameters are retained for call-site
/// compatibility but are no longer used directly.
pub async fn log_llm_usage_with_db(
    response: &CreateChatCompletionResponse,
    label: &str,
    _model: &str,
    _memory: &crate::memory::MemoryHandle,
) {
    log_llm_usage(response, label);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_returns_usable_client() {
        // Smoke test: factory should not panic with arbitrary credentials.
        let _client = create_llm_client("test-key", "http://localhost:1234/v1");
    }

    #[test]
    fn constants_are_consistent() {
        assert!(
            WEB_SEARCH_MODEL.ends_with(":online"),
            "WEB_SEARCH_MODEL must use :online suffix for OpenRouter native search"
        );
        assert!(MAX_BUFFER_SIZE > 0);
        assert!(
            (0.0..=1.0).contains(&DEDUP_SIMILARITY_THRESHOLD),
            "DEDUP_SIMILARITY_THRESHOLD must be a valid cosine similarity"
        );
    }

    #[test]
    fn provider_credit_exhaustion_detection_matches_openrouter_402_shapes() {
        assert!(is_provider_credit_exhaustion_error(
            r#"failed deserialization of: {"error":{"message":"Insufficient credits","code":402}}"#
        ));
        assert!(is_provider_credit_exhaustion_error(
            r#"embedding API returned 402: {"error":{"message":"Insufficient credits"}}"#
        ));
        assert!(is_provider_credit_exhaustion_error(
            r#"status 402: {"error":{"message":"Insufficient credits"}}"#
        ));
        assert!(!is_provider_credit_exhaustion_error(
            "status 429: rate limited"
        ));
        assert!(is_provider_credit_exhaustion_cooldown(
            "learning skipped: model provider credits are exhausted; background LLM calls are cooling down"
        ));
        assert!(!is_provider_credit_exhaustion_cooldown(
            "model provider credits are exhausted"
        ));
    }

    #[test]
    fn provider_credit_exhaustion_cooldown_suppresses_background_temporarily() {
        reset_provider_credit_exhaustion_for_test();
        assert!(!provider_credit_exhaustion_suppresses_background());

        record_provider_credit_exhaustion("test");

        let now = current_epoch_secs();
        assert!(provider_credit_exhaustion_suppresses_background_at(now));
        assert!(provider_credit_exhaustion_suppresses_background_at(
            now + PROVIDER_CREDIT_EXHAUSTION_BACKGROUND_COOLDOWN.as_secs() - 1
        ));
        assert!(!provider_credit_exhaustion_suppresses_background_at(
            now + PROVIDER_CREDIT_EXHAUSTION_BACKGROUND_COOLDOWN.as_secs() + 1
        ));

        reset_provider_credit_exhaustion_for_test();
    }

    #[test]
    fn provider_inference_preserves_subscription_and_openrouter_cost_classes() {
        assert_eq!(
            provider_from_response("gen-abc", "google/gemini-3.1-flash-lite-20260507"),
            Some("openrouter")
        );
        assert_eq!(
            provider_from_response("chatcmpl-opaque", "grok-4.3"),
            Some("xai_subscription")
        );
        assert_eq!(
            provider_from_response("codex-app-server-123", "gpt-5.5"),
            Some("codex_oauth")
        );
        assert_eq!(provider_from_response("chatcmpl-opaque", "unknown"), None);
    }

    #[test]
    fn log_llm_usage_with_cache_data() {
        let json = r#"{
            "id": "test", "object": "chat.completion", "created": 0,
            "model": "test-model",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 1000, "completion_tokens": 50, "total_tokens": 1050,
                "prompt_tokens_details": {"cached_tokens": 800}
            }
        }"#;
        let response: CreateChatCompletionResponse = serde_json::from_str(json).unwrap();
        // Should not panic; logging only
        log_llm_usage(&response, "test_cached");
    }

    #[test]
    fn log_llm_usage_with_none_usage() {
        let json = r#"{
            "id": "test", "object": "chat.completion", "created": 0,
            "model": "test-model", "choices": []
        }"#;
        let response: CreateChatCompletionResponse = serde_json::from_str(json).unwrap();
        // Should silently skip when usage is None
        log_llm_usage(&response, "test_none");
    }

    // Regression for 2026-04-17 silent outage: `let _ = tx.send(entry)`
    // swallowed a 4h channel failure.  Now the first send failure flips the
    // `broken_flag` (which gates the one-shot ERROR log) and every failure
    // bumps the counter.  Test uses test-scoped atomics + its own channel
    // to avoid touching the process-wide statics.
    #[test]
    fn log_channel_send_failure_logs_error_once() {
        use tokio::sync::mpsc;

        let (tx, rx) = mpsc::unbounded_channel::<LlmLogEntry>();
        // Drop the receiver to simulate a dead receiver task.
        drop(rx);

        let counter = AtomicU64::new(0);
        let broken = AtomicBool::new(false);

        let make_entry = |label: &str| LlmLogEntry {
            label: label.to_string(),
            model: "test-model".to_string(),
            provider: None,
            provider_request_id: Some("test".to_string()),
            prompt_tokens: 10,
            completion_tokens: 5,
            cached_tokens: 0,
            total_tokens: 15,
            latency_ms: None,
            is_fallback: false,
        };

        // First send: counter goes 0→1, broken flag flips false→true
        // (this is the transition that gates the one-shot ERROR log).
        send_or_record_failure(&tx, make_entry("first"), &counter, &broken);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert!(broken.load(Ordering::SeqCst));

        // Second send: counter bumps to 2, broken flag stays true so the
        // ERROR log path is NOT taken again (log-storm suppression).
        send_or_record_failure(&tx, make_entry("second"), &counter, &broken);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        assert!(broken.load(Ordering::SeqCst));

        // Third send: confirm further bumps still work and the flag stays
        // latched (the condition for skipping the ERROR emission).
        send_or_record_failure(&tx, make_entry("third"), &counter, &broken);
        assert_eq!(counter.load(Ordering::Relaxed), 3);
        assert!(broken.load(Ordering::SeqCst));
    }

    // Happy path: healthy receiver should NOT touch the failure counter
    // or flip the broken flag.  Protects against a future refactor that
    // accidentally marks every send as a failure.
    #[test]
    fn log_channel_send_success_leaves_state_clean() {
        use tokio::sync::mpsc;

        let (tx, mut rx) = mpsc::unbounded_channel::<LlmLogEntry>();
        let counter = AtomicU64::new(0);
        let broken = AtomicBool::new(false);

        send_or_record_failure(
            &tx,
            LlmLogEntry {
                label: "ok".to_string(),
                model: "m".to_string(),
                provider: None,
                provider_request_id: Some("test".to_string()),
                prompt_tokens: 1,
                completion_tokens: 1,
                cached_tokens: 0,
                total_tokens: 2,
                latency_ms: None,
                is_fallback: false,
            },
            &counter,
            &broken,
        );

        assert_eq!(counter.load(Ordering::Relaxed), 0);
        assert!(!broken.load(Ordering::SeqCst));
        // Receiver saw the entry.
        let received = rx.try_recv().expect("entry should have been queued");
        assert_eq!(received.label, "ok");
    }

    // Publicly exposed accessor should reflect the global counter monotonically.
    #[test]
    fn llm_log_send_failure_count_is_monotonic_accessor() {
        // We don't assert an exact value (other tests may have incremented
        // the global counter); just that it reads the current value without
        // panicking and is non-decreasing across two reads.
        let a = llm_log_send_failure_count();
        let b = llm_log_send_failure_count();
        assert!(b >= a);
    }

    // ── Regression for the 2026-05-21 silent llm_call_log stoppage ──────────
    //
    // Per-call LLM usage logging stopped persisting to `llm_call_log` for ~18
    // days after prod switched to the `codex_oauth` provider, whose synthesized
    // responses carried `usage: null` (so `log_llm_usage_timed` early-returned
    // and never queued an entry). The DB writer itself was healthy.
    //
    // This test exercises the REAL channel → receiver → `store_llm_call_log`
    // path against an in-memory DB and asserts a row lands. It fails if the
    // receiver loop is ever unwired from `store_llm_call_log` (the writer-side
    // failure mode), complementing the orchestrator-side test that proves the
    // codex_oauth response now carries usage (the upstream failure mode).
    #[tokio::test]
    async fn llm_log_entry_persists_a_row_through_real_receiver() {
        use tokio::sync::mpsc;

        let mem =
            crate::memory::MemoryHandle::spawn_in_memory().expect("spawn in-memory memory actor");

        let (tx, rx) = mpsc::unbounded_channel::<LlmLogEntry>();
        // Spawn the SAME receiver loop init_llm_log_db spawns in production.
        let receiver = tokio::spawn(run_llm_log_receiver(rx, mem.clone()));

        tx.send(LlmLogEntry {
            label: "react_round".to_string(),
            model: "gpt-5.5".to_string(),
            provider: Some("codex_oauth".to_string()),
            provider_request_id: Some("codex-app-server-abc".to_string()),
            prompt_tokens: 800,
            completion_tokens: 100,
            cached_tokens: 200,
            total_tokens: 1_300,
            latency_ms: Some(1_500),
            is_fallback: false,
        })
        .expect("send entry to live receiver");

        // Drop the sender so the receiver loop drains the queue and exits;
        // awaiting it guarantees the INSERT completed before we query.
        drop(tx);
        receiver.await.expect("receiver task joined");

        let coverage = mem
            .query_cost_coverage(1)
            .await
            .expect("query cost coverage");
        assert_eq!(
            coverage.row_count, 1,
            "the real receiver must INSERT one llm_call_log row per entry"
        );
        assert_eq!(coverage.total_prompt_tokens, 800);
        assert_eq!(coverage.total_completion_tokens, 100);
        assert_eq!(coverage.total_cached_tokens, 200);
        assert_eq!(coverage.total_tokens, 1_300);
    }
}
