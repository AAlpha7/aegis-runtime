use std::collections::{BTreeSet, HashMap};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::ImageUrl;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestToolMessageContent, ChatCompletionRequestToolMessageContentPart,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, ChatCompletionTool, ChatCompletionTools,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, CreateChatCompletionResponse,
    FunctionCall, FunctionObjectArgs,
};
use chrono::Timelike;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::agent_skills::{self, AgentSkill};
use crate::chronos::ChronosEngine;
use crate::embedding::EmbeddingClient;
use crate::identity::AgentIdentity;
use crate::intent;
use crate::llm::{
    AegisChatRequest, AegisRequestMetadata, CodexOAuthBackend, LlmProviderError, LlmProviderKind,
    LlmRouter, LlmRouterConfig, LlmTaskClass, RouteDecision, XaiSubscriptionAuthManager,
    XaiSubscriptionBackend,
};
use crate::llm_client;
use crate::memory::{self, MemoryHandle, Message};
use crate::models::SkillCapability;
use crate::sandbox::SandboxEngine;
use crate::settings::RuntimeSettings;

// Re-export menu types for backward compatibility with other modules
pub use crate::menu::{CommandSource, CustomCommand};

/// The orchestrator: multi-round permission-inversion ReAct loop.
///
/// Runs up to MAX_ROUNDS rounds of LLM→tool→LLM, with a global budget
/// of MAX_TOTAL_TOOL_CALLS. The LLM may request multiple tool calls per round;
/// Wasm skills execute in parallel while pseudo-tools run serially.
///
/// Security pipeline (per round):
///
/// 1. Telegram message → build context (L1 + L2 + L3 memory)
/// 2. Convert loaded Wasm skills → ChatCompletionTool format
/// 3. Call LLM (async-openai via OpenRouter)
/// 4. LLM returns tool\_calls JSON (or text reply → loop ends)
/// 5. Per tool\_call: intercept pseudo-tools → ToolGate permission check
///    (Once/Intent/Domain/Always policies) → dispatch-table resolution
///    → exfiltration detection (cross-round) → Ed25519 sign → Wasm sandbox
///    → post-hooks → tool result isolation markers
/// 6. Loop back to step 3 with accumulated messages
/// 7. When LLM stops calling tools → extract text → return final reply
///
/// Progress event sender. Historically `Sender<(i64, String)>`; now a typed
/// fan-out wrapper (`ProgressRouterSender`) that decodes each typed
/// [`crate::progress_router::ProgressEvent`] back to the exact legacy Telegram
/// string (byte-identical) and optionally forwards the typed event to a
/// per-turn webapp sink. See `core/src/progress_router.rs`.
pub type ProgressSender = crate::progress_router::ProgressRouterSender;

use crate::progress_router::{OwnerNoticeKind, ProgressEvent, ProgressPayload};

fn single_owner_telegram_user_did(chat_id: i64) -> String {
    single_owner_telegram_user_did_from_env(
        chat_id,
        std::env::var("OWNER_DID").ok().as_deref(),
        std::env::var("ADMIN_CHAT_ID").ok().as_deref(),
    )
}

fn single_owner_telegram_user_did_from_env(
    chat_id: i64,
    owner_did: Option<&str>,
    admin_chat_id: Option<&str>,
) -> String {
    owner_did
        .map(str::trim)
        .filter(|did| !did.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            admin_chat_id
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|admin_chat_id| *admin_chat_id != 0)
                .map(|admin_chat_id| format!("did:aegis:telegram:{admin_chat_id}"))
        })
        .unwrap_or_else(|| format!("did:aegis:telegram:{chat_id}"))
}

fn quoted_reply_parts(message: &str) -> Option<(&str, &str)> {
    const QUOTE_PREFIXES: [&str; 2] = ["[引用消息]", "[Quoted message]"];
    const REPLY_PREFIXES: [&str; 2] = ["[用户回复]", "[User reply]"];

    for reply_prefix in REPLY_PREFIXES {
        let Some((quoted_part, reply_part)) = message.split_once(reply_prefix) else {
            continue;
        };
        let quoted = QUOTE_PREFIXES
            .iter()
            .find_map(|prefix| quoted_part.trim().strip_prefix(prefix))
            .map(str::trim)?;
        return Some((quoted, reply_part.trim()));
    }
    None
}

fn is_task_completion_confirmation(reply: &str) -> bool {
    let normalized = reply
        .trim()
        .trim_matches(|c: char| {
            c.is_ascii_punctuation() || matches!(c, '。' | '，' | '！' | '？' | '；' | '、')
        })
        .trim()
        .to_lowercase();
    matches!(
        normalized.as_str(),
        "done"
            | "finished"
            | "complete"
            | "completed"
            | "完成"
            | "完成了"
            | "已完成"
            | "搞定"
            | "搞定了"
            | "已搞定"
    )
}

fn single_task_id_from_text(text: &str) -> Option<i64> {
    let mut ids = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '#' {
            continue;
        }
        let mut digits = String::new();
        while let Some((_, next)) = chars.peek().copied() {
            if next.is_ascii_digit() {
                digits.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            continue;
        }
        if let Ok(id) = digits.parse::<i64>()
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    (ids.len() == 1).then_some(ids[0])
}

/// Injected failure for [`Orchestrator::run_llm_fallback_drill`].
///
/// The drill walks the same authorized-fallback chain as a foreground turn,
/// but replaces the primary attempt with this fault so we can prove a backup
/// still answers when the primary is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrillFault {
    /// Do not call the primary at all; treat it as timed out.
    PrimaryTimeout,
}

impl DrillFault {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryTimeout => "primary_timeout",
        }
    }
}

/// Owner-facing diagnostic for a backup-model drill.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FallbackDrillReport {
    pub fault: &'static str,
    pub primary_provider: String,
    pub primary_model: String,
    pub fallback_authorized: bool,
    pub paid_api_fallback_enabled: bool,
    pub answered_by: Option<FallbackDrillAnswer>,
    pub elapsed_ms: u64,
    pub ok: bool,
    pub error: Option<String>,
}

/// Provider/model that actually produced the drill completion.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FallbackDrillAnswer {
    pub provider: String,
    pub model: String,
    pub reply_chars: usize,
}

/// Ephemeral runtime signal that ALL configured LLM providers/fallbacks are
/// currently exhausted for foreground (user-visible) requests. Set by the
/// terminal "all models failed" path in [`Orchestrator::chat_create_with_fallback`]
/// and cleared on the next successful foreground completion. Lives only in
/// memory (an `Arc<RwLock<Option<LlmDegraded>>>` on the orchestrator) — it is
/// NOT persisted, so a service restart clears it. Surfaced to the owner via the
/// webapp status projection (`llm_degraded`) and a one-shot Telegram alert on
/// the healthy→degraded transition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LlmDegraded {
    /// Short human-readable cause (e.g. "Codex usage limit" or the upstream
    /// error head, truncated). Owner-facing.
    pub reason: String,
    /// RFC3339 timestamp of when the current outage episode began.
    pub since: String,
    /// The active `llm_provider` setting value at the time of the outage
    /// (e.g. "codex_oauth", "xai_subscription", "openrouter").
    pub provider: String,
}

/// Pure decision for the foreground LLM degraded edge-trigger.
///
/// Given the CURRENT degraded state and a new exhaustion event, returns the
/// next state and whether this is a healthy→degraded TRANSITION (the only case
/// that fires the owner Telegram alert).
///
/// - `current == None`  → transition (warn). `since` = `now`.
/// - `current == Some`  → NOT a transition (no warn). `since` is PRESERVED from
///   the existing state so the outage episode keeps its original start time even
///   though `reason`/`provider` are refreshed to the latest observation.
fn compute_llm_degraded_transition(
    current: Option<&LlmDegraded>,
    reason: &str,
    provider: &str,
    now: &str,
) -> (LlmDegraded, bool) {
    match current {
        None => (
            LlmDegraded {
                reason: reason.to_string(),
                since: now.to_string(),
                provider: provider.to_string(),
            },
            true,
        ),
        Some(existing) => (
            LlmDegraded {
                reason: reason.to_string(),
                since: existing.since.clone(),
                provider: provider.to_string(),
            },
            false,
        ),
    }
}

/// Derive a short, owner-facing reason string from a terminal LLM error,
/// localized to `locale` (the owner's `display_language`). The error is
/// categorized into a small set of BUCKETS, each backed by an i18n key:
///
/// - `usage_limit`     — recognizable provider credit/usage exhaustion
/// - `all_unavailable` — empty error head (all-providers-down)
/// - `generic`         — anything else
///
/// The raw English error head is NEVER leaked into the user-facing reason
/// (it would mix an English fragment into the localized banner / Telegram
/// alert); it is recorded only in a tracing log for diagnostics. Never empty.
fn degraded_reason_from_error(err: &anyhow::Error, locale: &str) -> String {
    let full = format!("{err:#}");
    let bucket = if crate::llm_client::is_provider_credit_exhaustion_error(&full) {
        "provider.degraded.reason.usage_limit"
    } else if full.lines().next().unwrap_or(&full).trim().is_empty() {
        "provider.degraded.reason.all_unavailable"
    } else {
        // Keep the raw English head in logs only — never in the user reason.
        let head = full.lines().next().unwrap_or(&full).trim();
        tracing::warn!(
            error_head = %truncate_str(head, TRUNCATE_PREVIEW_MEDIUM),
            "[LLM Degraded] generic provider failure (raw head logged, not surfaced to owner)"
        );
        "provider.degraded.reason.generic"
    };
    crate::i18n::t(bucket, locale).to_string()
}

/// In-memory snapshot that the primary embedder is degraded (serving embed
/// calls from the backup model, and — critically — that semantic dedup is being
/// SKIPPED in that state, so duplicate memory writes can leak through the
/// weaker string-cooldown fallback). `None` = healthy.
///
/// Mirrors [`LlmDegraded`]: NOT persisted (a restart clears it), surfaced to the
/// owner via the webapp status projection (`embedder_degraded`) plus the
/// edge-triggered health-probe Telegram alert on the transition. Set on the
/// healthy→degraded edge and cleared on recovery by the runtime health probe.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EmbedderDegraded {
    /// RFC3339 timestamp of when the current degraded episode began.
    pub since: String,
    /// The primary (preferred) embedding model that is currently down.
    pub primary_model: String,
    /// The backup model now serving embed calls (`None` if no backup exists).
    pub backup_model: Option<String>,
}

/// Edge classification for a degraded-state boolean sampled over time. Used to
/// alert/flip the snapshot ONCE per transition instead of every probe tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedEdge {
    /// healthy → degraded: set the snapshot + fire the one-shot owner alert.
    Entered,
    /// degraded → healthy: clear the snapshot + fire the one-shot recovery notice.
    Recovered,
    /// No change since the previous sample: do nothing (no spam).
    Unchanged,
}

/// Pure edge-detector for a degraded boolean. `last` is the previously observed
/// value (`None` on the first sample, treated as "healthy/false" to mirror the
/// health probe's `last_was_degraded.unwrap_or(false)`). Returns the transition
/// edge so the caller acts exactly ONCE on each flip.
pub fn compute_degraded_edge(last: Option<bool>, current: bool) -> DegradedEdge {
    match (last.unwrap_or(false), current) {
        (false, true) => DegradedEdge::Entered,
        (true, false) => DegradedEdge::Recovered,
        _ => DegradedEdge::Unchanged,
    }
}

/// Owner-visible embedding-service health, derived from the ACTUAL probe
/// outcome (the ground truth of "is embedding serving right now") — NOT the
/// failover `is_degraded()` flag alone. The flag is insufficient for the
/// owner signal (Codex): with no backup configured it may never flip on a
/// primary failure, and once set it stays set even if the backup later fails,
/// so a boolean edge misses both the no-backup total outage AND the
/// backup→unreachable escalation. Keying the signal on the probe result +
/// this 3-state classifier fixes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderHealth {
    /// The probe embed succeeded and the client is NOT failed over — primary
    /// is serving normally.
    Healthy,
    /// The probe embed succeeded but the client is failed over — the BACKUP is
    /// serving (semantic dedup paused, but embeddings still produced).
    BackupServing,
    /// The probe embed FAILED — nothing is serving (primary AND any backup are
    /// down, or there is no backup). Ground truth from the probe, independent
    /// of the failover flag.
    Unreachable,
}

/// Classify embedding health from a probe tick. `probe_ok` = this tick's embed
/// call succeeded; `is_degraded` = the client is failed over off its primary.
pub fn classify_embedder_health(probe_ok: bool, is_degraded: bool) -> EmbedderHealth {
    match (probe_ok, is_degraded) {
        (false, _) => EmbedderHealth::Unreachable,
        (true, true) => EmbedderHealth::BackupServing,
        (true, false) => EmbedderHealth::Healthy,
    }
}

pub struct Orchestrator {
    // ── LLM / API core ──────────────────────────────────────────────────
    llm_client: Client<OpenAIConfig>,
    model: String,
    pub(crate) api_base: String,
    pub(crate) api_key: String,

    // ── Core infrastructure ─────────────────────────────────────────────
    /// Shared reqwest client for raw HTTP calls (web search, Google API, executor).
    pub(crate) http_client: reqwest::Client,
    pub(crate) sandbox: Arc<SandboxEngine>,
    pub(crate) identity: Arc<AgentIdentity>,
    pub(crate) memory: MemoryHandle,
    pub(crate) embedder: EmbeddingClient,
    system_prompt: String,
    pub(crate) settings: Arc<RuntimeSettings>,

    // ── Grouped sub-structs ─────────────────────────────────────────────
    /// Optional service engines (chronos, MCP, etc.).
    pub(crate) services: ServiceEngines,
    /// Credential and OAuth state.
    pub(crate) credentials: CredentialState,
    /// Caching and optimization state.
    pub(crate) caches: CacheState,

    // ── Skill execution ─────────────────────────────────────────────────
    pub(crate) progress_tx: Option<ProgressSender>,
    pub(crate) agent_skills: Vec<AgentSkill>,
    pub(crate) activated_skills: std::sync::Mutex<std::collections::HashSet<String>>,

    // ── Runtime state ───────────────────────────────────────────────────
    /// Unified tool permission gate.
    tool_gate: crate::tool_gate::ToolGate,
    /// Tool execution context (blocked URLs, callback group key, task info).
    pub(crate) tool_exec: ToolExecutionState,
    /// Concurrency flag: true when LLM is processing a user message or heartbeat.
    pub(crate) llm_busy: Arc<std::sync::atomic::AtomicBool>,
    /// Single-flight guard for `trigger_corpus_reembed`. Set when a reembed
    /// is in progress; a concurrent tool call returns `already_in_progress`
    /// immediately rather than starting a duplicate long-running pass.
    pub(crate) reembed_in_progress: Arc<std::sync::atomic::AtomicBool>,
    /// Notification deduplication state.
    pub(crate) notification_state: Arc<NotificationState>,
    /// Serializes menu load→modify→save→apply to prevent lost-update races.
    pub(crate) menu_lock: Arc<tokio::sync::Mutex<()>>,
    /// Cached Google account email (Calendar primary id) for attendee policy.
    google_self_addr: tokio::sync::OnceCell<String>,
    /// G7: Last time upstream health probes ran.
    last_health_probe_at: parking_lot::Mutex<Option<std::time::Instant>>,
    /// G7: Previous health probe results (service_name → was_healthy).
    health_probe_state: parking_lot::Mutex<std::collections::HashMap<String, bool>>,
    /// Subcall primitive registry — in-memory trace store for nested
    /// bounded LLM runs (spec docs/subcall-primitive-design.md §7.3). Held
    /// as `Arc` so a `SessionGcGuard` can own a clone without borrowing
    /// `&self` for the rest of the turn, and so the hourly safety-net
    /// sweep task in `main.rs` can reference it from its own tokio task.
    pub(crate) subcall_registry: Arc<crate::subcall::SubcallRegistry>,
    /// Ephemeral "all LLM providers exhausted" signal for foreground requests.
    /// `None` = healthy. Set on the terminal all-models-failed path, cleared on
    /// the next successful foreground completion. Not persisted (restart clears).
    /// Read by the webapp status projection and gated by an edge-triggered
    /// Telegram owner alert on the healthy→degraded transition.
    pub(crate) llm_degraded: Arc<parking_lot::RwLock<Option<LlmDegraded>>>,
    /// Ephemeral "primary embedder down — semantic dedup paused" signal.
    /// `None` = healthy. Set/cleared by the runtime health probe on the
    /// degraded transition (mirrors `llm_degraded`). Read by the webapp status
    /// projection so the owner sees when dedup quality is degraded. Not
    /// persisted (restart clears).
    pub(crate) embedder_degraded: Arc<parking_lot::RwLock<Option<EmbedderDegraded>>>,
}

/// Optional service engines that the Orchestrator may or may not have configured.
/// Each is set post-construction via `set_*()` methods.
pub(crate) struct ServiceEngines {
    /// Sender for the deferred-intent event bus. Wired by `main.rs` so
    /// approval flows that start long-lived runtimes (e.g. the OAuth proxy
    /// bootstrap) can publish events into the same channel the intent engine
    /// consumes.
    pub intent_event_tx: Option<tokio::sync::mpsc::Sender<crate::intent::IntentEvent>>,
    pub chronos: parking_lot::Mutex<Option<Arc<ChronosEngine>>>,
    pub mcp_client: Option<Arc<crate::mcp_client::McpClientManager>>,
    pub tts: Option<Arc<crate::multimodal::TtsSynthesizer>>,
    pub telegram_bot: Option<teloxide::Bot>,
    /// Hercules delegation client (`delegate_task`). `None` = not configured
    /// (no `HERCULES_API_BASE`/`HERCULES_TOKEN`); the tool then reports
    /// `not_configured` instead of pretending to delegate.
    pub hercules: parking_lot::Mutex<Option<Arc<crate::hercules_client::HerculesClient>>>,
    /// External coding agents for `run_external` (grok build primary, codex
    /// fallback). `None` = not wired on this instance.
    pub external_executors:
        parking_lot::Mutex<Option<Arc<crate::orchestrator_tools_external::ExternalExecutors>>>,
    /// Secure Computer Use pipeline (lazy; only when `AEGIS_SECURE_CU_ENABLED=1`).
    pub secure_cu: parking_lot::Mutex<Option<Arc<crate::computer_use::SecureCuPipeline>>>,
}

/// Credential and OAuth state.
pub(crate) struct CredentialState {
    /// SQLite path for credential store (Google auto-setup).
    pub credential_db_path: Option<String>,
    /// AES key for credential store encryption.
    pub credential_aes_key: Option<[u8; 32]>,
    /// Public URL for OAuth callback (e.g. "https://aegis.example.com").
    pub public_url: Option<String>,
}

/// Caching and optimization state.
pub(crate) struct CacheState {
    /// Lazily-initialized embedding caches (skill filter, tool review, factual grounding).
    pub embedding_caches: EmbeddingCaches,
    /// Cached tool effectiveness insights from P1 credit analysis.
    pub tool_effectiveness_cache: Arc<parking_lot::Mutex<String>>,
    /// Full schemas of deferred tools, keyed by tool name.
    /// Populated during build_tool_definitions_filtered, queried by search_tools handler.
    pub deferred_tool_schemas: parking_lot::Mutex<HashMap<String, ChatCompletionTools>>,
    /// P2: Prompt module overrides from auto prompt optimization.
    pub prompt_overrides: Arc<parking_lot::Mutex<HashMap<String, String>>>,
    /// Heartbeat agent-loop configuration (hot-reloaded on `heartbeat_config`
    /// setting writes; see `load_heartbeat_config_from_db` /
    /// `apply_heartbeat_config`). Defaults to the spec values at startup.
    pub heartbeat_config: Arc<parking_lot::RwLock<crate::heartbeat_config::HeartbeatConfig>>,
    /// Whether persisted alert policies have been loaded into ChronosEngine.
    pub alert_policies_loaded: std::sync::atomic::AtomicBool,
}

/// Cohesive sub-struct for notification deduplication state.
/// Extracted from Orchestrator to reduce field count and group related concerns.
pub(crate) struct NotificationState {
    /// Maps (chat_id, content_hash) → (sent_at, priority).
    /// Used to prevent duplicate `notify_user` messages within the dedup window.
    dedup: parking_lot::Mutex<
        std::collections::HashMap<(i64, u64), (chrono::DateTime<chrono::Utc>, NotifyPriority)>,
    >,
    /// Timestamp of the last notification sent to each chat_id.
    /// Used for decay window: if no notification for 48h, lower dedup threshold.
    last_at: parking_lot::Mutex<std::collections::HashMap<i64, chrono::DateTime<chrono::Utc>>>,
    /// Maps (chat_id, label) → last_sent_at.
    /// Prevents the same autonomous label from sending duplicate notifications
    /// when the LLM generates different wording across runs, defeating content-hash dedup.
    /// Cron probes use skill_name; true heartbeat uses "__heartbeat__".
    label_dedup:
        parking_lot::Mutex<std::collections::HashMap<(i64, String), chrono::DateTime<chrono::Utc>>>,
    /// G6: Daily notification counter per (chat_id, YYYY-MM-DD) → count.
    /// Resets automatically when the date changes.
    daily_count: parking_lot::Mutex<std::collections::HashMap<(i64, String), i32>>,
    /// G6: Pending notification buffer for temporal aggregation.
    /// Non-Urgent notifications are buffered here and flushed when the
    /// aggregation window expires. Tuple: (chat_id, message, buffered_at).
    pending_buffer: parking_lot::Mutex<Vec<(i64, String, chrono::DateTime<chrono::Utc>)>>,
    /// Step 10: heartbeat `send_message` history (per-chat) for semantic
    /// dedup and "last proactive message ago" observability. Each entry
    /// records `(chat_id, body, optional embedding, sent_at)`. Pre-Step-10
    /// entries have `None` embeddings and are skipped by cosine matching.
    /// Entries older than the dedup window (24h) are lazily pruned on write.
    sent_messages:
        parking_lot::Mutex<Vec<(i64, String, Option<Vec<f32>>, chrono::DateTime<chrono::Utc>)>>,
}

impl NotificationState {
    fn new() -> Self {
        Self {
            dedup: parking_lot::Mutex::new(std::collections::HashMap::new()),
            last_at: parking_lot::Mutex::new(std::collections::HashMap::new()),
            label_dedup: parking_lot::Mutex::new(std::collections::HashMap::new()),
            daily_count: parking_lot::Mutex::new(std::collections::HashMap::new()),
            pending_buffer: parking_lot::Mutex::new(Vec::new()),
            sent_messages: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Step 10 (design §5 gate 3): return true if any previously-sent message
    /// to `chat_id` within `window` has embedding cosine ≥ `threshold`
    /// vs the provided `embedding`. Pre-Step-10 entries with `None` embedding
    /// are skipped (back-compat) so legacy sends cannot spuriously block a
    /// new heartbeat message.
    pub(crate) fn content_cosine_matches(
        &self,
        chat_id: i64,
        embedding: &[f32],
        threshold: f32,
        window: chrono::Duration,
    ) -> bool {
        if embedding.is_empty() {
            return false;
        }
        let cutoff = chrono::Utc::now() - window;
        let log = self.sent_messages.lock();
        log.iter().any(|(cid, _, emb_opt, at)| {
            *cid == chat_id
                && *at >= cutoff
                && emb_opt
                    .as_ref()
                    .map(|e| {
                        crate::embedding::EmbeddingClient::cosine_similarity(embedding, e)
                            >= threshold
                    })
                    .unwrap_or(false)
        })
    }

    /// Step 10: record a successful heartbeat send. Prunes entries older
    /// than 24h on every write so the vec stays bounded (1 msg/tick × 48
    /// ticks/day ≤ tens of entries).
    pub(crate) fn record_sent_message(
        &self,
        chat_id: i64,
        body: String,
        embedding: Option<Vec<f32>>,
    ) {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::hours(24);
        let mut log = self.sent_messages.lock();
        log.retain(|(_, _, _, at)| *at >= cutoff);
        log.push((chat_id, body, embedding, now));
    }

    /// Step 10 §G refinement: minutes since the last recorded `send_message`
    /// to this chat (any chat if `None`). Returns `None` when nothing in
    /// state — caller falls back to the audit-row derivation.
    pub(crate) fn last_send_minutes_ago(&self, chat_id: Option<i64>) -> Option<u64> {
        let log = self.sent_messages.lock();
        let latest = log
            .iter()
            .filter(|(cid, _, _, _)| chat_id.map(|want| *cid == want).unwrap_or(true))
            .map(|(_, _, _, at)| *at)
            .max()?;
        let delta = chrono::Utc::now().signed_duration_since(latest);
        Some(delta.num_minutes().max(0) as u64)
    }

    /// G6: Check if the current hour (in user's timezone) falls within quiet hours.
    /// During quiet hours, only Urgent notifications are delivered.
    fn in_quiet_hours(&self, settings: &crate::settings::RuntimeSettings) -> bool {
        let start = settings
            .get("notification_quiet_start")
            .and_then(|v| v.as_u64())
            .unwrap_or(255) as u8;
        let end = settings
            .get("notification_quiet_end")
            .and_then(|v| v.as_u64())
            .unwrap_or(255) as u8;
        if start > 23 || end > 23 {
            return false; // Disabled (default 255)
        }
        let tz = crate::settings::parse_user_tz(settings);
        let now_hour = chrono::Utc::now().with_timezone(&tz).hour() as u8;
        if start <= end {
            now_hour >= start && now_hour < end
        } else {
            // Overnight: e.g. (22, 6) = 10pm-6am
            now_hour >= start || now_hour < end
        }
    }

    /// G6: Check if the daily notification cap has been reached for this chat.
    /// Returns true if over the limit.
    fn over_daily_cap(&self, chat_id: i64, settings: &crate::settings::RuntimeSettings) -> bool {
        let max = settings
            .get("notification_daily_max")
            .and_then(|v| v.as_i64())
            .unwrap_or(20) as i32;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let count = self.daily_count.lock();
        count.get(&(chat_id, today)).copied().unwrap_or(0) >= max
    }

    /// G6: Increment the daily notification counter for a chat.
    fn increment_daily_count(&self, chat_id: i64) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut count = self.daily_count.lock();
        *count.entry((chat_id, today)).or_insert(0) += 1;
    }

    /// G6: Purge stale daily counters (entries from days before today).
    fn cleanup_daily_counts(&self) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut count = self.daily_count.lock();
        count.retain(|(_, date), _| date == &today);
    }

    /// G6: Add a non-Urgent notification to the aggregation buffer.
    fn buffer_notification(&self, chat_id: i64, message: String) {
        let mut buf = self.pending_buffer.lock();
        buf.push((chat_id, message, chrono::Utc::now()));
    }

    /// G6: Drain buffered notifications whose oldest entry exceeds the aggregation window.
    /// Returns messages grouped by chat_id, only for chats where the window has expired.
    fn drain_expired_buffer(
        &self,
        window_secs: i64,
    ) -> std::collections::HashMap<i64, Vec<String>> {
        let now = chrono::Utc::now();
        let mut buf = self.pending_buffer.lock();
        if buf.is_empty() {
            return std::collections::HashMap::new();
        }

        // Find chats that have at least one message older than the window
        let threshold = now - chrono::Duration::seconds(window_secs);
        let expired_chats: std::collections::HashSet<i64> = buf
            .iter()
            .filter(|(_, _, t)| *t <= threshold)
            .map(|(cid, _, _)| *cid)
            .collect();

        if expired_chats.is_empty() {
            return std::collections::HashMap::new();
        }

        // Drain all messages for expired chats
        let mut result: std::collections::HashMap<i64, Vec<String>> =
            std::collections::HashMap::new();
        buf.retain(|(cid, msg, _)| {
            if expired_chats.contains(cid) {
                result.entry(*cid).or_default().push(msg.clone());
                false // remove from buffer
            } else {
                true // keep in buffer
            }
        });
        result
    }
}

/// Cohesive sub-struct for lazily-initialized embedding caches.
/// Groups skill filter, tool review, and factual grounding embeddings.
pub(crate) struct EmbeddingCaches {
    /// Skill description embeddings for semantic filtering.
    pub(crate) skills: parking_lot::Mutex<std::collections::HashMap<String, Vec<f32>>>,
    /// Tool description embeddings for the 3-layer review guard pipeline.
    pub(crate) tool_review: Arc<tokio::sync::OnceCell<std::collections::HashMap<String, Vec<f32>>>>,
    /// Factual grounding intent embeddings for correction/verification detection.
    pub(crate) factual_grounding: Arc<tokio::sync::OnceCell<Vec<Vec<f32>>>>,
}

impl EmbeddingCaches {
    fn new() -> Self {
        Self {
            skills: parking_lot::Mutex::new(std::collections::HashMap::new()),
            tool_review: Arc::new(tokio::sync::OnceCell::new()),
            factual_grounding: Arc::new(tokio::sync::OnceCell::new()),
        }
    }
}

/// Cohesive sub-struct for tool execution context state.
/// Groups the callback group key and task info — both used exclusively within
/// `execute_single_tool()` and intent dispatch.
pub(crate) struct ToolExecutionState {
    /// Current intent group_key for callback approval scoping.
    pub(crate) callback_group_key: Arc<tokio::sync::Mutex<Option<String>>>,
    /// Originating task info for auto-suspend on NeedApproval.
    pub(crate) callback_task_info: Arc<tokio::sync::Mutex<Option<(i64, i64)>>>,
}

impl ToolExecutionState {
    fn new() -> Self {
        Self {
            callback_group_key: Arc::new(tokio::sync::Mutex::new(None)),
            callback_task_info: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

/// Drop guard that resets an `AtomicBool` to `false` when dropped.
/// Used to ensure `llm_busy` is always released, even on panic/error.
struct BusyGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Normalize a notification message for dedup hashing.
///
/// Strips numbers, timestamps, and punctuation variations so that
/// semantically similar messages (e.g., "BTC $65,000" vs "BTC $65,001")
/// produce the same hash. This makes dedup effective for LLM-generated text.
pub(crate) fn normalize_for_dedup(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let mut chars = msg.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            // Skip all consecutive digits and numeric separators (,.:-)
            while chars.peek().is_some_and(|c| {
                c.is_ascii_digit() || *c == ',' || *c == '.' || *c == ':' || *c == '-'
            }) {
                chars.next();
            }
            // Replace ALL numeric sequences with placeholder.
            // Even single-digit numbers change between heartbeat runs
            // (e.g. "过去5h" → "过去6h") causing dedup misses.
            result.push('#');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Durable exact-content dedup for ASSISTANT-INITIATED proactive sends:
/// returns true when an identical (normalize_for_dedup-equal) assistant
/// message already exists in the owner's L1 timeline within the last
/// `window_hours`. L1 is restored from DB across restarts, so this backstops
/// the two known holes in the existing gates that let "嗨！有事吗?" deliver
/// twice at 23:05/01:00 (IMG_3939): the in-memory NotificationState dedup is
/// lost on process restart, and the semantic cosine gate is skipped entirely
/// while the embedder is degraded. Callers must be assistant-initiated paths
/// only (heartbeat send_message, autonomous notify) — owner-configured
/// recurring reminders repeat identical text BY DESIGN and must not route
/// through this check. Fail-open: any storage error reads as "not a
/// duplicate" so a glitch can never silently suppress a legitimate send.
///
/// STRICT comparison — whitespace-collapse only, digits preserved. Deliberately
/// NOT `normalize_for_dedup` (which strips every numeric sequence): a durable
/// 12h suppression that treats "BTC $65,001" as a repeat of "BTC $65,000"
/// would silently hide legitimate numeric updates (Codex round-1 HIGH).
/// Number-fuzzy matching stays exclusive to the short-lived in-memory gates.
pub(crate) async fn assistant_l1_exact_duplicate_within(
    memory: &crate::memory::MemoryHandle,
    user_did: &str,
    text: &str,
    window_hours: i64,
) -> bool {
    assistant_l1_exact_duplicate_within_impl(memory, user_did, text, window_hours, None).await
}

/// Anchor-scoped variant for SCHEDULED work-item deliveries: the prior L1 row
/// must carry the SAME FULL anchor identity (anchor_kind AND anchor_ref) in
/// addition to identical content. Two DISTINCT one-shot reminders that happen
/// to render the same text (e.g. two medication reminders scheduled
/// separately within 12h) have distinct anchor identities and must BOTH
/// deliver — only a replay/duplicate fire of the same obligation is
/// suppressed (Codex round-4 HIGH). anchor_ref alone is NOT globally
/// namespaced (deferred-intent group_keys are LLM-chosen and can collide with
/// e.g. an email_thread ref), so kind must match too (Codex round-5 HIGH).
pub(crate) async fn assistant_l1_exact_duplicate_with_anchor_within(
    memory: &crate::memory::MemoryHandle,
    user_did: &str,
    text: &str,
    window_hours: i64,
    anchor_kind: &str,
    anchor_ref: &str,
) -> bool {
    assistant_l1_exact_duplicate_within_impl(
        memory,
        user_did,
        text,
        window_hours,
        Some((anchor_kind, anchor_ref)),
    )
    .await
}

async fn assistant_l1_exact_duplicate_within_impl(
    memory: &crate::memory::MemoryHandle,
    user_did: &str,
    text: &str,
    window_hours: i64,
    required_anchor: Option<(&str, &str)>,
) -> bool {
    fn strict_normalize(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    let normalized = strict_normalize(text);
    if normalized.is_empty() {
        return false;
    }
    let cutoff = chrono::Utc::now().timestamp() - window_hours * 3600;
    let anchor_matches = |kind: Option<&str>, anchor: Option<&str>| {
        required_anchor.is_none_or(|(rk, ra)| kind == Some(rk) && anchor == Some(ra))
    };

    // Fast path: the in-memory L1 window (restored from its periodic snapshot
    // across restarts).
    match memory.get_l1(user_did).await {
        Ok(messages) => {
            if messages.iter().rev().any(|msg| {
                msg.role == "assistant"
                    && msg.timestamp >= cutoff
                    && anchor_matches(msg.anchor_kind.as_deref(), msg.anchor_ref.as_deref())
                    && strict_normalize(&msg.content) == normalized
            }) {
                return true;
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "L1 dedup check failed; falling through to L3");
        }
    }

    // Durable path: L1 is a bounded FIFO window (Codex round-6 MEDIUM — under
    // high chat volume, or after a crash that predates the last L1 snapshot,
    // a <12h-old message can already be evicted/absent). Every push_l1 also
    // archives synchronously to l3_archive, so scan the recent archive too.
    // Caveats, both fail-open toward delivering: L3 skips trivial (<5 byte)
    // messages, and the scan is capped at the 500 most recent rows.
    match memory.list_recent_l3(user_did, 500).await {
        Ok(entries) => entries.iter().rev().any(|entry| {
            entry.role == "assistant"
                && chrono::DateTime::parse_from_rfc3339(&entry.created_at)
                    .map(|dt| dt.timestamp() >= cutoff)
                    .unwrap_or(false)
                && anchor_matches(entry.anchor_kind.as_deref(), entry.anchor_ref.as_deref())
                && strict_normalize(&entry.content) == normalized
        }),
        Err(e) => {
            tracing::warn!(error = %e, "L3 dedup check failed; treating as not-duplicate");
            false
        }
    }
}

/// Notification priority tier for dedup and delivery control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyPriority {
    /// Critical alerts — always delivered immediately, no dedup.
    Urgent,
    /// Actionable updates — delivered with 24h dedup window (default).
    Important,
    /// Informational — included in daily summary only, not sent immediately.
    Routine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProactiveNotificationOutcome {
    Delivered,
    AlreadySatisfied,
    Deferred,
}

impl NotifyPriority {
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "urgent" => Self::Urgent,
            "routine" => Self::Routine,
            _ => Self::Important,
        }
    }
}

/// A notification queued by the `notify_user` pseudo-tool during a callback turn.
#[derive(Debug, Clone)]
pub struct NotifyUserRequest {
    pub message: String,
    pub pin: bool,
    pub priority: NotifyPriority,
    /// Tool names whose results this notification is based on.
    /// Used by grounding check to verify claims against specific tool outputs.
    /// Empty = unsourced (legacy behavior, penalized in grounding score).
    pub source_tools: Vec<String>,
}

/// Mutable context passed to tool handler methods, replacing 16 separate parameters.
///
/// Groups all per-turn mutable state that handlers need to read or modify.
pub(crate) struct ToolContext<'a> {
    pub user_did: &'a str,
    pub chat_id: Option<i64>,
    pub loaded_skills: &'a [SkillCapability],
    pub previous_tool_results: &'a mut Vec<String>,
    pub tools_called: &'a mut Vec<String>,
    pub tools_blocked: &'a mut Vec<String>,
    pub messages: &'a mut Vec<ChatCompletionRequestMessage>,
    pub total_tool_call_count: &'a mut usize,
    pub max_tool_calls: usize,
    pub is_callback: bool,
    pub callback_depth: i32,
    pub notifications: &'a mut Vec<NotifyUserRequest>,
    /// Deferred (stub) tools whose full schema was looked up this turn via
    /// `search_tools` or an empty-args auto-resolve. Later rounds of the same
    /// turn offer the full schema instead of the stub, so the model can pass
    /// real parameters (grok kept sending `{}` against the stub, 2026-09-08).
    pub promoted_tools: &'a mut Vec<String>,
    /// If set, tool calls are gated via `evaluate_autonomous` instead of the
    /// normal gate logic.  Used during autonomous heartbeat execution.
    pub autonomous_config: Option<&'a crate::chronos::AutonomousConfig>,
    pub data_access_context: Option<&'a RuntimeDataAccessContext>,
    /// Mid-tick mutable state shared with heartbeat-scoped tool handlers
    /// (Step 8a). `None` on every non-heartbeat path — handlers must treat
    /// a `None` here as "not heartbeat, skip the state update".
    pub heartbeat_tick_state:
        Option<std::sync::Arc<tokio::sync::Mutex<crate::heartbeat_config::HeartbeatTickState>>>,
    /// Caller context propagated from the react loop; drives tool-gate
    /// evaluation and subcall nesting checks. Replaces implicit reads of
    /// `is_callback` / `autonomous_config.is_some()` for gate purposes.
    pub caller_context: crate::tool_gate::CallerContext,
    /// `(remaining_tokens, cap)` for the parent session — used by
    /// `spawn_subcall` to clamp subcall budget
    /// (`ClampReason::ParentBudgetRemaining`). Threaded through
    /// `run_autonomous_react_loop` as of Task 2.6. The heartbeat path
    /// still passes the spec-authorized `(128_000, 128_000)` placeholder
    /// until a real per-session budget tracker lands; `run_subcall`
    /// forwards its parent's true `(remaining, cap)`.
    pub parent_token_budget: (usize, usize),
    /// `true` when the current tool invocation is replayed by
    /// `execute_approved_tool` after a Gate-layer HITL approval landed.
    /// Drives Task #12's single-approval UX: for the explicit
    /// user-via-Gate path, any downstream Config proposal (e.g.
    /// `update_setting` → `create_config_proposal`) is pre-approved and
    /// applied synchronously — the user already said "yes" at the Gate,
    /// forcing a second approval for the same action is friction.
    ///
    /// MUST be `false` for every non-Gate path (LLM-initiated tool calls,
    /// heartbeat ticks, admin HTTP, MCP server, panel submissions) so
    /// those flows keep their full two-layer defence-in-depth.
    pub gate_pre_approved: bool,
    /// Public-search budget for this turn. `None` on paths that never search.
    pub search_turn: Option<&'a mut crate::search_turn::SearchTurnLedger>,
}

/// Owner authentication evidence attached to a runtime turn.
///
/// Web-originated turns must carry the signed-in Web Console principal so
/// sensitive tools do not fall back to the Telegram chat identity and lose
/// already-established owner confirmation.
#[derive(Debug, Clone)]
pub struct RuntimeDataAccessContext {
    pub principal: crate::data_access_gate::AuthPrincipal,
    pub mfa_freshness_secs: Option<u64>,
}

impl ToolContext<'_> {
    /// Session key for `SubcallRegistry` — derived from `user_did`/`chat_id`.
    ///
    /// Matches the `user:{did}:chat:{cid_or_0}` namespace used by the
    /// user-turn `SessionGcGuard` in `process_message_multimodal`. It does
    /// NOT match the `heartbeat:{did}:{cid}:{ts}` namespace used by the
    /// heartbeat-tick guard — subcalls spawned from a heartbeat tick would
    /// land under the user-turn namespace, where the hourly safety-net
    /// sweep in `main.rs` still collects them but the tick's per-invocation
    /// `SessionGcGuard::drop` does not. Dormant today because heartbeat
    /// does not expose `spawn_subcall` (Phase 5). Phase 5 will need to
    /// thread an explicit session id through `run_autonomous_react_loop`
    /// to unify the two namespaces; Task 2.6 scope covered caller +
    /// parent_token_budget only.
    pub fn session_id(&self) -> String {
        match self.chat_id {
            Some(cid) => format!("user:{}:chat:{}", self.user_did, cid),
            None => format!("user:{}:chat:0", self.user_did),
        }
    }

    /// True when this ctx was constructed from inside a running subcall.
    /// `spawn_subcall` uses this to reject nesting early.
    pub fn caller_context_is_subcall(&self) -> bool {
        matches!(
            self.caller_context,
            crate::tool_gate::CallerContext::Subcall
        )
    }

    /// `(remaining, cap)` for the parent session's token budget. Consumed
    /// by `run_subcall` for `ParentBudgetRemaining` clamping.
    pub fn parent_token_budget(&self) -> (usize, usize) {
        self.parent_token_budget
    }
}

/// Reason a heartbeat tick fired. Design §15.2 Step 3.
///
/// Today only `Scheduled` exists (cron metronome). The probe path records
/// observations into `observed_events` for the next scheduled tick; the
/// early-wake variant (`ProbeBurst`) is intentionally deferred — a
/// latency-vs-cost tradeoff that will be layered in when/if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickTrigger {
    /// Fired by the cron metronome at the configured interval.
    Scheduled,
}

/// Result of a single orchestration turn.
///
/// Unified per design §2.1 / §15.2 Step 3: the heartbeat path no longer
/// returns a separate `AutonomousOutcome`. Heartbeat-specific fields
/// (`actions_taken`, `fell_through_to_alert`, `outcome_tag`) live here so
/// both user-initiated turns and scheduled ticks produce the same shape.
#[derive(Debug, Default)]
pub struct OrchestratorResult {
    pub reply: String,
    pub tools_called: Vec<String>,
    pub tools_blocked: Vec<String>,
    /// Number of LLM→tool→LLM rounds used in this turn.
    pub rounds_used: usize,
    /// Optional inline keyboard buttons to attach to the reply message.
    /// Each tuple is (label, callback_data).
    pub inline_buttons: Vec<(String, String)>,
    /// Notifications queued by the `notify_user` tool during callback execution.
    /// In callback mode, text replies are NOT sent to the user — only these
    /// explicit notifications are delivered.
    pub notifications: Vec<NotifyUserRequest>,
    /// Human-readable descriptions of actions taken during this turn.
    /// Populated primarily by the autonomous ReAct loop; empty for user turns.
    pub actions_taken: Vec<String>,
    /// If true, the autonomous handler decided it could not handle the situation
    /// and the engine should fall through to the standard alert path. Always
    /// false for user-initiated turns.
    pub fell_through_to_alert: bool,
    /// Outcome classification for credit assignment analysis (autonomous path).
    pub outcome_tag: crate::chronos::OutcomeTag,
    /// Parsed JSON arguments of the `final_answer` tool call, if the LLM
    /// invoked it. Populated by `run_autonomous_react_loop` whenever a
    /// tool call with `function.name == "final_answer"` is seen in any
    /// assistant message. Consumed by the subcall runner (see
    /// `core/src/subcall.rs::run_subcall`) to extract structured output.
    /// `None` for every non-subcall caller — they can ignore this field.
    pub final_answer_args: Option<serde_json::Value>,
    /// Full message history at end-of-loop (subcall-only consumer;
    /// heartbeat callers ignore via `..Default::default()`).
    /// Populated by `run_autonomous_react_loop` so `run_subcall` can
    /// persist the raw transcript into `subcall_trace` for later recall
    /// (spec §7.5). Empty `Vec` for every non-subcall caller.
    pub messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
    /// Per-tool `(tool_name, raw_output_json)` entries captured during the
    /// loop. Empty when no non-`final_answer` tool call was made. Subcall-
    /// only consumer; other callers already read `tools_called` for the
    /// name list and don't need raw bodies.
    pub tool_outputs: Vec<(String, String)>,
    /// `true` when the ReAct loop terminated because the per-tick tool-call
    /// budget was exhausted (mirrors the local `budget_exhausted` flag).
    /// `run_subcall` reads this to distinguish budget exhaustion from
    /// genuine schema-violation `InvalidOutput` (spec §7.2).
    pub budget_exhausted: bool,
    /// Owner turns: ms from the message reaching the orchestrator to the stored reply.
    pub latency_ms: Option<u64>,
    /// Summed `response.usage.total_tokens` across all LLM calls in this
    /// loop. Populated by `run_autonomous_react_loop`; non-subcall callers
    /// may ignore. `run_subcall` uses this to persist a real
    /// `subcall_log.tokens_used` and to credit engine-level daily budgets
    /// with the actual consumption (Phase 1.6b follow-up).
    pub total_tokens_used: u32,
}

impl OrchestratorResult {
    /// Adapter returning the reply as `Option<&str>` — `None` if empty.
    /// Heartbeat code historically worked with `Option<String>`; this keeps
    /// the read sites ergonomic without doubling the storage.
    pub fn reply_text(&self) -> Option<&str> {
        if self.reply.is_empty() {
            None
        } else {
            Some(self.reply.as_str())
        }
    }
}

impl OrchestratorResult {
    /// Convenience constructor for simple text replies (no tools, no buttons).
    /// Most HITL handlers just need to return a message; this eliminates
    /// 7 lines of Default field boilerplate per call site.
    pub(crate) fn reply(msg: impl Into<String>) -> Self {
        Self {
            reply: msg.into(),
            ..Default::default()
        }
    }
}

/// Maximum LLM→tool→LLM rounds per user message (prevents infinite loops).
const MAX_ROUNDS: usize = 15;
/// Maximum total tool calls across all rounds (token budget enforcement).
const MAX_TOTAL_TOOL_CALLS: usize = 35;
/// Per-call LLM timeout. 180s accommodates large tool schemas and slow providers.
const LLM_CALL_TIMEOUT: Duration = Duration::from_secs(180);

/// Default wall-clock time for a single Wasm skill execution (including I/O).
/// Fuel limits protect against CPU-bound loops, but host_fetch I/O can block
/// indefinitely. This timeout caps the total execution time.
/// Skills can override this via `timeout_secs` in manifest.json (max 300s).
const DEFAULT_WASM_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
/// Hard ceiling for per-skill timeout override.
const MAX_WASM_EXEC_TIMEOUT: Duration = Duration::from_secs(300);

/// Minimum cosine similarity for a skill to be considered relevant to a user query.
const SEMANTIC_SKILL_THRESHOLD: f32 = 0.30;
/// Maximum number of skills to include via semantic filtering.
/// If total loaded skills <= this, no filtering is applied.
const SEMANTIC_SKILL_TOP_K: usize = 10;
/// Minimum loaded skill count before semantic filtering kicks in.
/// Below this threshold, all skills are always included.
const SEMANTIC_FILTER_MIN_SKILLS: usize = 15;
/// Wasm skills that are always included regardless of semantic similarity score.
const CORE_WASM_SKILLS: &[&str] = &["calendar_reader", "web_reader", "sysadmin_ops"];

/// Tool pairs that must not execute in parallel (Wasm parallel dispatch).
/// Pseudo-tools are already serial. This catches Wasm skill conflicts.
/// Format: (tool_a, tool_b) — bidirectional.
const TOOL_CONFLICTS: &[(&str, &str)] = &[];

// ── Prompt cache break detection (CC/Alpha pattern) ───────────────────────

/// Global hash of the most recently seen stable prompt. 0 = not yet initialized.
static STABLE_PROMPT_HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// FNV-1a hash of a byte slice — deterministic, fast, no external deps.
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Detect unexpected changes to the stable prompt that would break LLM prompt caching.
/// Returns `Some((prev_hash, new_hash))` when a change is detected (skipping the first call
/// which just seeds the hash). Returns `None` on first call or when the prompt is unchanged.
fn detect_cache_break(stable_prompt: &str) -> Option<(u64, u64)> {
    let new_hash = fnv1a_hash(stable_prompt.as_bytes());
    let prev = STABLE_PROMPT_HASH.swap(new_hash, std::sync::atomic::Ordering::Relaxed);
    if prev != 0 && prev != new_hash {
        Some((prev, new_hash))
    } else {
        None
    }
}

/// Check if any pair of tool names in the batch has a conflict.
fn has_tool_conflict(tools: &[&str]) -> bool {
    for i in 0..tools.len() {
        for j in (i + 1)..tools.len() {
            for &(a, b) in TOOL_CONFLICTS {
                if (tools[i] == a && tools[j] == b) || (tools[i] == b && tools[j] == a) {
                    return true;
                }
            }
        }
    }
    false
}

/// Seconds in one hour (for time arithmetic).
pub(crate) const SECS_PER_HOUR: i64 = 3_600;
// SECS_PER_DAY (86_400) and WEBAPP_TOKEN_EXPIRY_SECS (14_400)
// removed — unused after prior refactor. Reintroduce as named constants
// if a future change re-adds the time arithmetic they supported.

/// Preview truncation for short contexts (action descriptions, etc.).
const TRUNCATE_PREVIEW_SHORT: usize = 80;
/// Preview truncation for medium contexts (replies, errors, etc.).
const TRUNCATE_PREVIEW_MEDIUM: usize = 200;
/// Preview truncation for long contexts (tool arguments, etc.).
const TRUNCATE_PREVIEW_LONG: usize = 500;

/// Outcome of executing a single wasm tool, collected for parallel dispatch.
#[allow(dead_code)]
struct WasmToolOutcome {
    tool_call_id: String,
    tool_name: String,
    result_str: String,
    /// Whether the tool was blocked (exfiltration, budget, etc.)
    was_blocked: bool,
    /// If true, caller should stop processing further tools (budget exhaustion / HITL).
    force_break: bool,
    /// Tool messages to append to the conversation.
    tool_messages: Vec<ChatCompletionRequestMessage>,
}

/// Build a `WasmToolOutcome` for early-return "blocked" cases (unregistered tool,
/// invalid JSON, missing required fields, exfiltration detection).
fn make_blocked_wasm_outcome(
    tool_call_id: &str,
    tool_name: &str,
    msg: &str,
) -> Result<WasmToolOutcome> {
    Ok(WasmToolOutcome {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        result_str: String::new(),
        was_blocked: true,
        force_break: false,
        tool_messages: vec![
            ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id)
                .content(msg)
                .build()?
                .into(),
        ],
    })
}
/// Scrub a Wasm tool result before it enters a MODEL-FACING sink. The
/// `previous_tool_results` vector is the one sink that is BOTH model-facing and
/// fanned out across multiple ReAct execution paths (sequential, parallel,
/// autonomous/heartbeat single-tool) — it is later formatted into LLM prompts by
/// the notification evaluator and subcall recall. Historical mail-client
/// HTML fields were scrubbed here before 2026-09-07; remaining tools pass
/// through unchanged. ALL `previous_tool_results.push` of a `WasmToolOutcome`
/// MUST go through this helper so a new push site cannot silently reintroduce the
/// HTML leak (Codex 2026-06-21, found three separate push sites).
fn model_facing_tool_result(_tool_name: &str, result_str: String) -> String {
    result_str
}
/// Intermediate context built during the context-building phase of
/// `process_message_multimodal`.  Carries data from `build_react_context()`
/// through `assemble_llm_messages()` and into `run_react_loop()`.
struct ReactContext {
    query_embedding: Option<Vec<f32>>,
    history: Vec<Message>,
    /// Stable portion of system prompt — changes only on skill load/unload.
    /// Sent as system[0] for prompt cache stability.
    stable_prompt: String,
    /// Volatile portion — changes per call (time, settings, facts, etc.).
    /// Sent as system[1].
    volatile_prompt: String,
}

fn render_active_work_items_block(
    items: &[crate::assistant_work_items::StoredAssistantWorkItem],
    preferred_anchor: Option<(&str, &str)>,
    limit: usize,
) -> String {
    if items.is_empty() {
        return String::new();
    }

    let mut matching = Vec::new();
    let mut other = Vec::new();
    for item in items {
        let is_match = preferred_anchor.is_some_and(|(kind, reference)| {
            item.item.anchor_kind.map(|v| v.as_str()) == Some(kind)
                && item.item.anchor_ref.as_deref() == Some(reference)
        });
        if is_match {
            matching.push(item);
        } else {
            other.push(item);
        }
    }

    let mut ordered = Vec::with_capacity(items.len());
    ordered.extend(matching);
    ordered.extend(other);
    ordered.truncate(limit);

    let mut block = String::from(
        "\n\n<active_assistant_work>\n\
         The assistant has ongoing background work and unresolved follow-ups. \
         Treat these as the same continuous secretary relationship, not separate bots.\n",
    );
    let mut last_anchor: Option<&str> = None;
    let mut grouped_count = 0usize;
    for item in ordered {
        let anchor_ref = item.item.anchor_ref.as_deref().unwrap_or("(none)");
        let anchor_changed = last_anchor != Some(anchor_ref);
        last_anchor = Some(anchor_ref);

        let mut related_count = 0usize;
        if anchor_ref != "(none)" {
            related_count = items
                .iter()
                .filter(|candidate| candidate.item.anchor_ref.as_deref() == Some(anchor_ref))
                .count();
        }

        if !anchor_changed && related_count > 1 {
            continue;
        }
        grouped_count += 1;
        if grouped_count > limit {
            break;
        }

        let grouped_suffix = if related_count > 1 {
            format!(" | grouped_related_work={related_count}")
        } else {
            String::new()
        };
        block.push_str(&format!(
            "- [{}] {} | anchor={} | delivery={} | source={}{}\n",
            item.item.status.as_str(),
            item.item.goal,
            anchor_ref,
            item.item.delivery_policy.as_str(),
            item.item.source_kind.as_str(),
            grouped_suffix,
        ));
    }
    block.push_str("</active_assistant_work>");
    block
}

fn render_preferred_anchor_block(preferred_anchor: Option<(&str, &str)>) -> String {
    let Some((kind, reference)) = preferred_anchor else {
        return String::new();
    };
    format!(
        "\n\n<conversation_anchor>\n\
         Current conversation anchor preference: {} = {}\n\
         Keep continuity with this thread/task unless the user clearly pivots away.\n\
         </conversation_anchor>",
        kind, reference
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UserMessagePersistence {
    Persist,
    Ephemeral,
}

impl UserMessagePersistence {
    fn should_persist_l1(&self) -> bool {
        matches!(self, Self::Persist)
    }

    fn should_append_user_to_runtime_history(&self) -> bool {
        !self.should_persist_l1()
    }
}

/// Result returned by `run_react_loop()`.  Contains all mutable state that
/// the coordinator (`process_message_multimodal`) needs for post-processing.
struct ReactLoopResult {
    /// Runtime disclosures must survive model review unchanged.
    behavior_notices: Vec<String>,
    reply: String,
    tools_called: Vec<String>,
    tools_blocked: Vec<String>,
    rounds_used: usize,
    notifications: Vec<NotifyUserRequest>,
    /// Tool result strings accumulated across all rounds — used by
    /// `evaluate_notifications` and `post_process_reply`.
    previous_tool_results: Vec<String>,
    /// Tool definitions used in this turn (needed for review pipeline).
    tools: Vec<ChatCompletionTools>,
    /// Exact `(tool, result)` pairs from the transcript (see
    /// `tool_outputs_from_transcript`); the evidence layer prefers these.
    tool_outputs: Vec<(String, String)>,
}

/// Mutable state accumulated across ReAct rounds.
///
/// Groups all per-turn tracking variables so they can be threaded through
/// `run_single_react_round()` without 15+ individual parameters.
struct ReactLoopState {
    messages: Vec<ChatCompletionRequestMessage>,
    tools_called: Vec<String>,
    tools_blocked: Vec<String>,
    previous_tool_results: Vec<String>,
    total_tool_call_count: usize,
    max_rounds: usize,
    max_tool_calls: usize,
    loaded_skills: Vec<SkillCapability>,
    notifications: Vec<NotifyUserRequest>,
    /// See `ToolContext::promoted_tools`.
    promoted_tools: Vec<String>,
    round: usize,
    /// Message index where each round started (for microcompaction age tracking).
    round_start_indices: Vec<usize>,
    xml_retried: bool,
    empty_retried: bool,
    live_data_no_tool_retried: bool,
    /// One correction round after a reply that invents a tool restriction.
    refusal_retried: bool,
    search: crate::search_turn::SearchTurnLedger,
}

/// Outcome of a single ReAct round, directing the coordinator loop.
enum RoundOutcome {
    /// Tool calls processed or retry injected — continue to next round.
    Continue,
    /// LLM produced a final text reply (no more tool calls).
    FinalReply(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentDataKind {
    Calendar,
    Tasks,
    ApiUsage,
    /// "How is the server" — totals from `server_monitor`.
    ServerStatus,
    /// Headlines — `news_digest` top stories.
    News,
    /// A request that names a URL to open/read: fetched with `browser_fetch`,
    /// never answered from memory (deploy 30 smoke: the model recited
    /// "Example Domain" without fetching, 2026-09-12).
    WebPage,
    /// Questions about the machine Aegis Runtime runs on (what takes disk space,
    /// what is running, what the logs say): answered by `sysadmin_ops`
    /// diagnose, never from memory or server_monitor totals (owner 2026-09-12).
    HostMachine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentDataWindow {
    Default,
    Today,
    Tomorrow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentDataRequirement {
    kind: CurrentDataKind,
    window: CurrentDataWindow,
    unread_only: bool,
    /// `HostMachine` only: the bounded read-only command to run.
    host_command: Option<&'static str>,
    /// `WebPage` only: the URL to fetch.
    target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeterministicToolPlan {
    tool_name: &'static str,
    arguments: serde_json::Value,
}

impl CurrentDataRequirement {
    fn deterministic_read_plan(
        self,
        tools: &[ChatCompletionTools],
    ) -> Option<DeterministicToolPlan> {
        match self.kind {
            CurrentDataKind::Calendar if has_declared_tool(tools, "calendar_reader") => {
                Some(DeterministicToolPlan {
                    tool_name: "calendar_reader",
                    arguments: serde_json::json!({
                        "action": if self.window == CurrentDataWindow::Tomorrow {
                            "tomorrow"
                        } else {
                            "today"
                        },
                    }),
                })
            }
            CurrentDataKind::WebPage if has_declared_tool(tools, "browser_fetch") => {
                Some(DeterministicToolPlan {
                    tool_name: "browser_fetch",
                    arguments: serde_json::json!({
                        "url": self.target.clone()?,
                        "mode": "text",
                        "max_chars": 4000,
                    }),
                })
            }
            CurrentDataKind::HostMachine if has_declared_tool(tools, "sysadmin_ops") => {
                Some(DeterministicToolPlan {
                    tool_name: "sysadmin_ops",
                    arguments: serde_json::json!({
                        "action_type": "diagnose",
                        "diagnostic_cmd": self.host_command?,
                    }),
                })
            }
            CurrentDataKind::ServerStatus if has_declared_tool(tools, "server_monitor") => {
                Some(DeterministicToolPlan {
                    tool_name: "server_monitor",
                    arguments: serde_json::json!({
                        "load_threshold_multiplier": 3.0,
                        "memory_threshold_percent": 80.0,
                        "disk_threshold_percent": 70.0,
                    }),
                })
            }
            CurrentDataKind::News if has_declared_tool(tools, "news_digest") => {
                Some(DeterministicToolPlan {
                    tool_name: "news_digest",
                    arguments: serde_json::json!({ "action": "top_stories", "count": 10 }),
                })
            }
            CurrentDataKind::Tasks if has_declared_tool(tools, "manage_tasks") => {
                Some(DeterministicToolPlan {
                    tool_name: "manage_tasks",
                    arguments: serde_json::json!({
                        "action": "list",
                    }),
                })
            }
            _ => None,
        }
    }
}

fn owner_request_requires_live_tool(user_message: &str, tools: &[ChatCompletionTools]) -> bool {
    infer_current_data_requirement(user_message, tools).is_some()
}

fn infer_current_data_requirement(
    user_message: &str,
    tools: &[ChatCompletionTools],
) -> Option<CurrentDataRequirement> {
    let normalized = user_message.to_ascii_lowercase();
    let has_available_tool = |names: &[&str]| {
        tools.iter().any(|tool| {
            if let ChatCompletionTools::Function(function_tool) = tool {
                names.contains(&function_tool.function.name.as_str())
            } else {
                false
            }
        })
    };
    // A URL in the request is a fetch, whatever the wording around it.
    if let Some(url) = first_url_in_text(user_message)
        && has_available_tool(&["browser_fetch"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::WebPage,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: None,
            target: Some(url),
        });
    }
    if !owner_text_has_lookup_intent(&normalized) {
        return None;
    }

    if contains_any(
        &normalized,
        &[
            "calendar", "schedule", "event", "meeting", "日程", "会议", "安排",
        ],
    ) && has_available_tool(&["calendar_reader"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::Calendar,
            window: if contains_any(&normalized, &["tomorrow", "明天"]) {
                CurrentDataWindow::Tomorrow
            } else {
                CurrentDataWindow::Today
            },
            unread_only: false,
            host_command: None,
            target: None,
        });
    }

    if let Some(command) = host_machine_command(&normalized)
        && has_available_tool(&["sysadmin_ops"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::HostMachine,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: Some(command),
            target: None,
        });
    }

    if contains_any(&normalized, &["服务器", "server", "主机", "机器"])
        && contains_any(
            &normalized,
            &[
                "状态",
                "怎么样",
                "怎样",
                "健康",
                "情况",
                "负载",
                "还好",
                "正常",
                "status",
                "health",
                "load",
                "doing",
                "ok",
            ],
        )
        && has_available_tool(&["server_monitor"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::ServerStatus,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: None,
            target: None,
        });
    }

    if contains_any(&normalized, &["新闻", "头条", "news", "headline"])
        && has_available_tool(&["news_digest"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::News,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: None,
            target: None,
        });
    }

    if contains_any(
        &normalized,
        &[
            "task",
            "tasks",
            "reminder",
            "reminders",
            "todo",
            "待办",
            "任务",
            "提醒",
        ],
    ) && has_available_tool(&["manage_tasks", "list_intents"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::Tasks,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: None,
            target: None,
        });
    }

    if contains_any(
        &normalized,
        &[
            "cost", "usage", "api", "token", "credit", "成本", "用量", "余额",
        ],
    ) && has_available_tool(&["check_api_usage"])
    {
        return Some(CurrentDataRequirement {
            kind: CurrentDataKind::ApiUsage,
            window: CurrentDataWindow::Default,
            unread_only: false,
            host_command: None,
            target: None,
        });
    }

    None
}

/// The bounded read-only command that answers a host-machine question, or
/// `None` when the message is not about the machine. Disk/space questions
/// get a one-level `du` over the data trees (`-m -d 1`, never `-s` with `-d`,
/// never the whole root; the executor allows up to 180 s and the skill
/// asks for all of it); process questions the top CPU consumers; log
/// questions the syslog tail. Every command is on the executor's read-only
/// whitelist and uses at most one pipe between whitelisted commands.
fn host_machine_command(normalized: &str) -> Option<&'static str> {
    // The message must be about THIS machine. Bare "log"/"space"/"running"
    // matched changelogs, disk space of a laptop, and "running late"
    // (review 2026-09-13), and every Chinese message counts as ≤3 "words",
    // so the lookup-intent pre-filter never protected these branches.
    let machine = contains_any(
        normalized,
        &[
            "服务器",
            "主机",
            "本机",
            "机器",
            "这台",
            "系统",
            "vps",
            "server",
            "host machine",
            "the machine",
            "the box",
            "this machine",
            "磁盘",
            "硬盘",
            "disk",
            "cpu",
            "内存",
            "memory",
            "进程",
            "process",
            "syslog",
            "日志",
            "server log",
            // "what is taking up (a lot of) space" only makes sense about this host.
            "占了大量空间",
            "占用空间",
            "占了空间",
            "空间占用",
            "占用了空间",
            "大文件",
            "disk space",
            "taking up space",
        ],
    );
    if !machine {
        return None;
    }
    if contains_any(
        normalized,
        &[
            "磁盘",
            "硬盘",
            "空间",
            "占了",
            "占用",
            "目录",
            "大文件",
            "存储",
            "disk",
            "storage",
            "director",
            "large file",
            "du ",
        ],
    ) && !contains_any(normalized, &["cpu", "内存", "memory", "进程", "process"])
    {
        return Some("du -xm -d 1 /home /opt /var /tmp");
    }
    if contains_any(
        normalized,
        &[
            "进程",
            "在跑",
            "跑着",
            "占cpu",
            "吃cpu",
            "占内存",
            "吃内存",
            "process",
            "hogging",
            "cpu",
            "内存",
            "memory",
        ],
    ) {
        return Some("ps aux --sort=-%cpu | head -n 15");
    }
    if contains_any(normalized, &["日志", "syslog", "报错日志", " log", "logs"]) {
        return Some("tail -n 100 /var/log/syslog");
    }
    None
}

/// True when a reply asserts a rule that forbids or restricts tool use in
/// this turn. No such rule exists anywhere in the runtime; the model invents
/// it (three times on 2026-09-12: "本轮禁止运行诊断命令"). The ReAct loop
/// answers such a reply with one correction and one more round.
pub(crate) fn reply_claims_tool_restriction(reply: &str) -> bool {
    let lower = reply.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "本轮禁止",
            "本轮限制",
            "本轮不允许",
            "禁止运行",
            "禁止执行",
            "禁止调用",
            "不允许运行",
            "不允许执行",
            "不允许调用",
            "被禁止",
            "无权运行",
            "无权执行",
            "not allowed to run",
            "not permitted to run",
            "forbidden to run",
            "forbidden from running",
            "restricted from running",
            "diagnostics are forbidden",
            "diagnostics are not allowed",
            "cannot run diagnostics",
            "tools are disabled",
            "tool use is disabled",
            "this turn forbids",
            "restricted this turn",
        ],
    )
}

/// Owner text that asks to look something up (read/list/check/what/看看/查…)
/// or is a terse current-data lookup. Shared by the live-data guard and the
/// reasoning-effort picker: such a request is never "casual".
pub(crate) fn owner_text_has_lookup_intent(normalized: &str) -> bool {
    let asks_to_read_or_summarize = contains_any(
        normalized,
        &[
            "summarize",
            "summary",
            "list",
            "show",
            "check",
            "fetch",
            "get",
            "new",
            "any",
            "read",
            "review",
            "what",
            "which",
            "today",
            "latest",
            "recent",
            "unread",
            "总结",
            "汇总",
            "概览",
            "列出",
            "看看",
            "查看",
            "检查",
            "查询",
            "查",
            "看一下",
            "看下",
            "帮我看",
            "拉取",
            "有没有",
            "有无",
            "读",
            "未读",
            "今天",
            "最新",
            "最近",
        ],
    );
    asks_to_read_or_summarize || looks_like_terse_current_data_lookup(normalized)
}

/// A reply that says the assistant cannot do / has no access to / does not
/// support what was asked — as opposed to an answer. Broader than
/// `reply_claims_tool_restriction`; on its own it only triggers the refusal
/// audit (a cheap model call that checks the tool catalog), never a retry.
pub(crate) fn reply_claims_capability_gap(reply: &str) -> bool {
    let lower = reply.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "无法",
            "不能",
            "暂不支持",
            "不支持",
            "没有权限",
            "无权",
            "做不到",
            "没法",
            "cannot",
            "can't",
            "unable to",
            "not able to",
            "no access",
            "not supported",
            "don't have access",
            "do not have access",
        ],
    )
}

/// One line per declared tool ("name: first 160 chars of description") for
/// the refusal audit prompt.
pub(crate) fn tool_catalog_for_audit(tools: &[ChatCompletionTools]) -> String {
    let mut lines = Vec::new();
    for tool in tools {
        if let ChatCompletionTools::Function(function_tool) = tool {
            let description = function_tool
                .function
                .description
                .as_deref()
                .unwrap_or("")
                .replace('\n', " ");
            let description: String = description.chars().take(160).collect();
            lines.push(format!(
                "- {}: {}",
                function_tool.function.name,
                description.trim()
            ));
        }
    }
    lines.join("\n")
}

/// First `http(s)://…` token in the text, with trailing punctuation removed.
pub(crate) fn first_url_in_text(text: &str) -> Option<String> {
    text.split_whitespace()
        .flat_map(|token| token.split(['，', '。', '（', '）', '「', '」', '"', '\'', '<', '>']))
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(|token| {
            token
                .trim_end_matches(['.', ',', ';', ':', ')', ']'])
                .to_string()
        })
        .filter(|url| url.len() > 8)
}

fn has_declared_tool(tools: &[ChatCompletionTools], name: &str) -> bool {
    tools.iter().any(|tool| {
        matches!(
            tool,
            ChatCompletionTools::Function(function_tool)
                if function_tool.function.name == name
        )
    })
}

fn looks_like_terse_current_data_lookup(normalized: &str) -> bool {
    // "怎么样 / 怎样 / how is" ask how something IS (a status lookup);
    // "怎么 / 如何 / how to" ask how to DO something (conceptual). Only the
    // latter is filtered out.
    let asks_how_to = normalized.replace("怎么样", "").contains("怎么")
        || (normalized.contains("how")
            && !contains_any(normalized, &["how is", "how's", "how are", "how was"]));
    if asks_how_to
        || contains_any(
            normalized,
            &[
                "系统",
                "设计",
                "安全",
                "原理",
                "架构",
                "方案",
                "规划",
                "分解",
                "为什么",
                "为何",
                "如何",
                "design",
                "architecture",
                "secure",
                "security",
                "why",
            ],
        )
    {
        return false;
    }

    let compact_char_count = normalized.chars().filter(|c| !c.is_whitespace()).count();
    let word_count = normalized.split_whitespace().count();
    let question_like = contains_any(normalized, &["?", "？"])
        || normalized.ends_with('吗')
        || normalized.ends_with('么');

    question_like || compact_char_count <= 8 || word_count <= 3
}

fn synthetic_live_data_tool_call(
    user_message: &str,
    tools: &[ChatCompletionTools],
) -> Option<Vec<ChatCompletionMessageToolCalls>> {
    let plan = infer_current_data_requirement(user_message, tools)
        .and_then(|requirement| requirement.deterministic_read_plan(tools))?;

    let arguments = serde_json::to_string(&plan.arguments).unwrap_or_else(|_| "{}".to_string());
    Some(vec![ChatCompletionMessageToolCalls::Function(
        ChatCompletionMessageToolCall {
            id: format!("aegis_live_data_{}", chrono::Utc::now().timestamp_millis()),
            function: FunctionCall {
                name: plan.tool_name.to_string(),
                arguments,
            },
        },
    )])
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[derive(Debug, Clone, Copy)]
struct ReactLoopBudgetOverride {
    max_rounds: usize,
    max_tool_calls: usize,
}

/// Push a tool result message into the messages vector.
///
/// Results are inserted at full size — each tool is responsible for limiting
/// its own output. Old large results are compacted by `microcompact_tool_results`
/// after 3 rounds (with a summary prefix, not blank).
pub(crate) fn push_tool_result(
    messages: &mut Vec<ChatCompletionRequestMessage>,
    tool_call_id: &str,
    content: &str,
) -> Result<()> {
    messages.push(
        ChatCompletionRequestToolMessageArgs::default()
            .tool_call_id(tool_call_id)
            .content(content)
            .build()?
            .into(),
    );
    Ok(())
}

/// Replace old large tool results with lightweight placeholders.
///
/// Scans messages older than `age_rounds` rounds for Tool messages whose
/// content exceeds `min_bytes`. Replaces them with a short placeholder,
/// preserving `tool_call_id` for conversation coherence.
///
/// Returns the number of messages compacted.
fn microcompact_tool_results(
    messages: &mut [ChatCompletionRequestMessage],
    round_start_indices: &[usize],
    current_round: usize,
    age_rounds: usize,
    min_bytes: usize,
) -> Result<usize> {
    if current_round <= age_rounds {
        return Ok(0);
    }
    let cutoff_round = current_round - age_rounds;
    // round_start_indices is 0-based: [0]=round1, [1]=round2, etc.
    let cutoff_idx = if cutoff_round >= 1 {
        round_start_indices
            .get(cutoff_round - 1)
            .copied()
            .unwrap_or(0)
    } else {
        0
    };

    let mut compacted = 0usize;
    for i in 0..cutoff_idx.min(messages.len()) {
        if let ChatCompletionRequestMessage::Tool(tool_msg) = &messages[i] {
            let content_len = match &tool_msg.content {
                ChatCompletionRequestToolMessageContent::Text(s) => s.len(),
                ChatCompletionRequestToolMessageContent::Array(parts) => parts
                    .iter()
                    .map(|p| match p {
                        ChatCompletionRequestToolMessageContentPart::Text(t) => t.text.len(),
                        #[allow(unreachable_patterns)]
                        _ => 0,
                    })
                    .sum(),
            };
            if content_len > min_bytes {
                let tool_call_id = tool_msg.tool_call_id.clone();
                // Keep a 200-char summary prefix so the LLM can judge whether
                // to re-call the tool. Better than a blank placeholder.
                let summary = match &tool_msg.content {
                    ChatCompletionRequestToolMessageContent::Text(s) => {
                        let boundary = s.floor_char_boundary(200);
                        format!(
                            "[Compacted — {} bytes, {} rounds ago] {}...\nRe-call the tool if you need full data.",
                            content_len,
                            current_round.saturating_sub(cutoff_round),
                            &s[..boundary],
                        )
                    }
                    _ => "[Tool result compacted — old round]".to_string(),
                };
                messages[i] = ChatCompletionRequestToolMessageArgs::default()
                    .tool_call_id(&tool_call_id)
                    .content(summary.as_str())
                    .build()?
                    .into();
                compacted += 1;
            }
        }
    }
    Ok(compacted)
}

/// Check whether a tool name is handled by the pseudo-tool intercept chain
/// (i.e., NOT dispatched to the Wasm sandbox).
fn is_pseudo_tool(name: &str, loaded_skills: &[SkillCapability]) -> bool {
    // MCP tools are dispatched as pseudo-tools (serial, async I/O to external servers).
    if crate::mcp_client::is_mcp_tool(name) {
        return true;
    }
    // If the name is in crate::tool_schemas::PSEUDO_SHADOW_NAMES, it's a pseudo-tool
    // (shadows a Wasm skill of the same name).
    if crate::tool_schemas::PSEUDO_SHADOW_NAMES.contains(&name) {
        return true;
    }
    // Also check stripped version for hallucinated "skill_" prefix
    if let Some(stripped) = name.strip_prefix("skill_")
        && crate::tool_schemas::PSEUDO_SHADOW_NAMES.contains(&stripped)
    {
        return true;
    }
    // If the name is NOT a loaded Wasm skill, it's a pseudo-tool.
    // Also check with "skill_" prefix stripped — LLMs sometimes hallucinate the prefix.
    let is_wasm = loaded_skills.iter().any(|s| s.skill_name == name)
        || name
            .strip_prefix("skill_")
            .is_some_and(|stripped| loaded_skills.iter().any(|s| s.skill_name == stripped));
    !is_wasm
}

#[cfg(test)]
fn tool_args_equivalent(a: &str, b: &str) -> bool {
    a == b
        || serde_json::from_str::<serde_json::Value>(a).ok()
            == serde_json::from_str::<serde_json::Value>(b).ok()
}

// Security constants and functions (shannon_entropy, detect_exfiltration,
// check_tool_permission, ToolPermissionResult) have been extracted to `crate::security`.
use crate::security::detect_exfiltration;
// Re-export for tests and external callers
pub use crate::security::{ToolPermissionResult, check_tool_permission, shannon_entropy};

// Utility functions (truncate_str, extract_text_content, sanitize_model_xml,
// classify_skill_error, sanitized_tool_error, json_parse_tool_error, kb_entry_to_json)
// have been extracted to `crate::utils`.
use crate::utils::{
    SANITIZE_FALLBACK_MSG, classify_skill_error, extract_text_content, sanitize_model_xml,
    sanitize_tool_result_markers, sanitized_tool_error, truncate_str,
};

// Prompt modules have been extracted to `crate::prompts`.
use crate::prompts::{
    core_system_prompt, default_system_prompt, prompt_module_alert_policies,
    prompt_module_api_usage, prompt_module_calendar, prompt_module_delegation,
    prompt_module_external_executor, prompt_module_goals, prompt_module_host_machine,
    prompt_module_intents, prompt_module_knowledge_base, prompt_module_mcp,
    prompt_module_monitoring, prompt_module_multimodal, prompt_module_proactive,
    prompt_module_settings, prompt_module_subcall, prompt_module_time,
    prompt_module_tool_effectiveness, prompt_module_web,
};

/// Minimum cosine similarity for L2 lessons to be considered relevant.
/// Calibrated for nomic-embed-text v1.5 embeddings; values below 0.4 tend
/// to match on shared vocabulary rather than semantic relevance.
const LESSON_RELEVANCE_THRESHOLD: f32 = 0.40;

/// Maximum number of relevant lessons to inject into the system prompt.
const MAX_INJECTED_LESSONS: usize = 15;

/// Maximum L2/L3 facts to inject into the system prompt.
const MAX_INJECTED_FACTS: usize = 30;

/// Minimum cosine similarity between user query and pre-computed intent
/// embeddings for factual grounding to trigger (forces web_search).
const FACTUAL_GROUNDING_THRESHOLD: f32 = 0.61;

/// Maximum total token count of all messages before LLM call.
/// Proactive compaction triggers at 100K (from CC/Alpha pattern) to keep
/// context lean and reduce per-call costs. 500K was the original conservative limit.
const MAX_MESSAGE_TOKENS: usize = crate::compaction::COMPACT_TRIGGER_TOKENS;

// EXFIL_PREV_SCAN_LEN, EXFIL_STEP, shannon_entropy, TOKEN_PATTERNS,
// contains_known_token_pattern, contains_hex_blob, extract_all_strings,
// detect_exfiltration → moved to crate::security

// FAST_TOKEN_MODE, set_fast_token_mode, estimate_messages_tokens → moved to crate::utils
// parse_timezone_offset → moved to crate::utils
use crate::utils::{estimate_messages_tokens, parse_timezone_offset};
// Re-export for backward compatibility with external test files
pub use crate::utils::set_fast_token_mode;

/// Explicitly configured summarization model (`summarization_model` setting),
/// `None` when the setting is empty/whitespace.
fn explicit_summarization_model(settings: &RuntimeSettings) -> Option<String> {
    settings
        .get("summarization_model")
        .and_then(|v| v.as_str().map(str::trim).map(str::to_owned))
        .filter(|s| !s.is_empty())
}

/// Decide provider + model for a rolling summary.
///
/// With `summarization_model` empty the request goes through the normal
/// router with task class `Summarization`, so the owner's provider policy
/// (Codex background gate, paid-API fallback) applies unchanged.
///
/// When the owner has named a `summarization_model`, that is an explicit
/// opt-in to spend paid OpenAI-compatible API credits on this background
/// task, so the request is routed straight to the OpenAI-compatible backend
/// for that model. Without this, a `codex_oauth` foreground provider with
/// `codex_allow_background_tasks=false` and `paid_api_fallback_enabled=false`
/// rejected every summary (`BackgroundTaskBlocked`) and long conversations
/// lost their memory.
fn route_summarization_decision(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
) -> Result<RouteDecision, LlmProviderError> {
    let config = LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(settings)?;
    let backend = crate::llm::OpenAiCompatBackend::new(client.clone(), model);
    let router = LlmRouter::new(Arc::new(backend), config);
    if explicit_summarization_model(settings).is_some() {
        return router.route_provider(LlmProviderKind::OpenAiCompat, LlmTaskClass::Summarization);
    }
    let route_request = AegisChatRequest {
        model: model.to_string(),
        messages: Vec::new(),
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        tool_choice: None,
        metadata: AegisRequestMetadata {
            task_class: LlmTaskClass::Summarization,
            request_id: None,
            chat_id: None,
            user_visible: false,
            reasoning_effort: None,
        },
    };
    router.route_chat(&route_request)
}

/// Resolve the model name a rolling summary is sent to, logging the route
/// (provider, model, paid-API accounting) at INFO. See
/// [`route_summarization_decision`] for the policy.
fn route_summarization_model(
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
) -> Result<String> {
    let explicit_model = explicit_summarization_model(settings).is_some();
    let decision =
        route_summarization_decision(client, settings, model).map_err(anyhow::Error::new)?;
    tracing::info!(
        label = "summarize",
        provider = %decision.provider,
        model = %decision.model,
        task_class = ?LlmTaskClass::Summarization,
        is_fallback = decision.is_fallback,
        explicit_summarization_model = explicit_model,
        paid_api_provider = decision.provider.is_paid_api(),
        paid_api_fallback_allowed = settings
            .get("paid_api_fallback_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "[LLM Route]"
    );
    Ok(decision.model)
}

fn llm_runtime_prompt_block(settings: &RuntimeSettings, startup_model: &str) -> String {
    let base_model = settings
        .get("llm_model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if startup_model.trim().is_empty() {
                "(provider default)".to_string()
            } else {
                startup_model.to_string()
            }
        });
    let provider_raw = settings
        .get("llm_provider")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "openai_compat".to_string());
    let provider = provider_raw
        .parse::<LlmProviderKind>()
        .unwrap_or(LlmProviderKind::OpenAiCompat);
    let xai_enabled = settings
        .get("xai_subscription_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let codex_enabled = settings
        .get("codex_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (effective_provider, effective_model, note) = match provider {
        LlmProviderKind::XaiSubscription if xai_enabled => {
            let model = settings
                .get("xai_subscription_model")
                .and_then(|v| v.as_str().map(str::to_owned))
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| base_model.clone());
            (
                "xai_subscription".to_string(),
                model,
                "Foreground owner chat is routed through the Grok subscription provider.",
            )
        }
        LlmProviderKind::CodexOAuth if codex_enabled => {
            let model = codex_runtime_model_name_from_settings(settings)
                .unwrap_or_else(|| base_model.clone());
            (
                "codex_oauth".to_string(),
                model,
                "Foreground owner chat is routed through the Codex OAuth provider.",
            )
        }
        _ => (
            provider.to_string(),
            base_model.clone(),
            "Foreground owner chat uses the base llm_model.",
        ),
    };

    format!(
        "\n\n<llm_runtime>\n\
         Foreground chat provider: {effective_provider}\n\
         Effective foreground chat model: {effective_model}\n\
         Base llm_model setting: {base_model}\n\
         Note: {note}\n\
         If the user asks what model or provider you are using, answer from \
         Effective foreground chat model/provider. Do not answer from \
         Base llm_model when a subscription provider is active.\n\
         Background, embedding, summarization, and specialized tasks may use \
         separate models or may be disabled by safety settings.\n\
         </llm_runtime>"
    )
}

fn codex_runtime_model_name_from_settings(settings: &RuntimeSettings) -> Option<String> {
    settings
        .get("codex_model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .map(|value| normalize_codex_cli_model(&value))
        .filter(|value| !value.trim().is_empty())
}

fn normalize_codex_cli_model(model: &str) -> String {
    let trimmed = model.trim();
    if let Some(rest) = trimmed.strip_prefix("openai-codex/") {
        rest.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
fn codex_cli_model_arg(model: &str) -> Option<String> {
    let normalized = normalize_codex_cli_model(model);
    if normalized.is_empty()
        || normalized.eq_ignore_ascii_case("codex-cli-default")
        || normalized.eq_ignore_ascii_case("default")
    {
        None
    } else {
        Some(normalized)
    }
}

fn codex_prompt_from_chat_request(request: &CreateChatCompletionRequest) -> Result<String> {
    let messages = serde_json::to_string_pretty(&request.messages)
        .context("failed to serialize Codex app-server prompt messages")?;
    let tool_note = if request
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
    {
        "Aegis dynamic tools are available in this turn. For current, private, external, or system data, call an appropriate dynamic tool when needed and answer from the tool result. Do not claim to have searched, read private data, or executed actions unless the current transcript or this turn's tool results contain evidence or a receipt."
    } else {
        "No Aegis tools were attached to this request."
    };
    Ok(format!(
        "You are the official Codex app-server acting as Aegis Runtime's foreground reasoning backend.\n\
         Safety contract:\n\
         - Answer the owner directly using the Aegis runtime context below.\n\
         - The transcript is conversation history and safety context, not a boundary that prevents tool use.\n\
         - Do not modify files, run shell commands, or perform external actions.\n\
         - Do not claim that an action completed unless the transcript or this turn's tool results contain a receipt.\n\
         - {tool_note}\n\
         - Preserve the user's requested language and tone.\n\n\
         <aegis_chat_transcript_json>\n{messages}\n</aegis_chat_transcript_json>"
    ))
}

fn codex_chat_tool_config() -> serde_json::Value {
    // Aegis dynamic tools own authorization, search and evidence receipts.
    // Native tools must not silently provide a second, unrecorded route.
    serde_json::json!({
        "web_search": "disabled",
        "features.shell_tool": false,
        "features.unified_exec": false,
        "apps._default.enabled": false,
    })
}

fn codex_app_server_base_instructions() -> &'static str {
    "You are Aegis Runtime's foreground reasoning backend. Answer the owner directly. \
     The transcript is context, not a boundary that prevents dynamic tool use. \
     When current/private/external data is needed, call available dynamic tools. \
     Do not claim actions completed unless the transcript or this turn's tool results contain receipts."
}

async fn write_codex_app_server_message(
    stdin: &mut tokio::process::ChildStdin,
    message: serde_json::Value,
) -> Result<()> {
    let line = serde_json::to_string(&message).context("serialize Codex app-server message")?;
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write Codex app-server message")?;
    stdin
        .write_all(b"\n")
        .await
        .context("write Codex app-server newline")?;
    stdin
        .flush()
        .await
        .context("flush Codex app-server stdin")?;
    Ok(())
}

fn codex_app_server_response_id(message: &serde_json::Value) -> Option<u64> {
    message.get("id").and_then(serde_json::Value::as_u64)
}

fn codex_app_server_method(message: &serde_json::Value) -> Option<&str> {
    message.get("method").and_then(serde_json::Value::as_str)
}

fn codex_app_server_error_from_response(message: &serde_json::Value) -> Option<String> {
    message.get("error").map(serde_json::Value::to_string)
}

fn codex_app_server_final_agent_text(message: &serde_json::Value) -> Option<String> {
    let item = message.pointer("/params/item")?;
    codex_app_server_agent_text_from_item(item)
}

fn codex_app_server_agent_text_from_item(item: &serde_json::Value) -> Option<String> {
    if item.get("type").and_then(serde_json::Value::as_str) != Some("agentMessage") {
        return None;
    }
    if item.get("phase").and_then(serde_json::Value::as_str) == Some("commentary") {
        return None;
    }
    item.get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
}

fn codex_app_server_turn_completed_text(message: &serde_json::Value) -> Option<String> {
    if codex_app_server_method(message) != Some("turn/completed") {
        return None;
    }
    let items = message
        .pointer("/params/turn/items")
        .and_then(serde_json::Value::as_array)?;
    items
        .iter()
        .rev()
        .find_map(codex_app_server_agent_text_from_item)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn codex_app_server_turn_completed_error(message: &serde_json::Value) -> Option<String> {
    if codex_app_server_method(message) != Some("turn/completed") {
        return None;
    }
    let turn = message.pointer("/params/turn")?;
    let status = turn.get("status").and_then(serde_json::Value::as_str);
    if status != Some("failed") {
        return None;
    }
    turn.pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            turn.get("error")
                .map(serde_json::Value::to_string)
                .filter(|text| !text.trim().is_empty())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAppServerToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

/// Token usage for one Codex app-server turn, extracted from the
/// `thread/tokenUsage/updated` notification's `params.tokenUsage.last`
/// breakdown (the Codex app-server protocol does NOT attach usage to the
/// `turn/completed` event). Carried back so the synthesized OpenAI-shaped
/// response can populate its `usage` field — without it, `log_llm_usage_timed`
/// sees `usage == None` and persists nothing, which is exactly the regression
/// that silently disabled `llm_call_log` once prod switched to the
/// `codex_oauth` provider (2026-05-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CodexAppServerUsage {
    /// Non-cached input tokens (maps to OpenAI `prompt_tokens`).
    prompt_tokens: u32,
    /// Output tokens incl. reasoning (maps to OpenAI `completion_tokens`).
    completion_tokens: u32,
    /// Cached input tokens (maps to OpenAI `prompt_tokens_details.cached_tokens`).
    cached_tokens: u32,
    /// Total tokens for the turn (maps to OpenAI `total_tokens`).
    total_tokens: u32,
}

/// Extract the per-turn usage breakdown from a `thread/tokenUsage/updated`
/// notification. Returns `None` for any other message. Field names follow the
/// Codex app-server `TokenUsageBreakdown` type (`totalTokens`, `inputTokens`,
/// `cachedInputTokens`, `outputTokens`, `reasoningOutputTokens`).
///
/// Repo convention (matches OpenAI): `prompt_tokens` is cache-INCLUSIVE and
/// `cached_tokens` is a subset of it (uncached = prompt - cached). Codex
/// `inputTokens` already includes the cached portion, so it maps directly to
/// `prompt_tokens` (we do NOT subtract cached — that would inflate the cache-hit
/// rate computed downstream). `total_tokens` = reported, else `input + output`.
fn codex_app_server_turn_usage(message: &serde_json::Value) -> Option<CodexAppServerUsage> {
    if codex_app_server_method(message) != Some("thread/tokenUsage/updated") {
        return None;
    }
    let last = message.pointer("/params/tokenUsage/last")?;
    let get = |key: &str| -> u32 {
        last.get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32
    };
    let input = get("inputTokens");
    let cached = get("cachedInputTokens").min(input);
    let output = get("outputTokens").saturating_add(get("reasoningOutputTokens"));
    let total = {
        let reported = get("totalTokens");
        if reported > 0 {
            reported
        } else {
            input.saturating_add(output)
        }
    };
    Some(CodexAppServerUsage {
        // prompt_tokens is cache-INCLUSIVE (repo convention: cached_tokens is a SUBSET
        // of prompt_tokens — see llm_pricing.rs `uncached = prompt - cached` and the
        // AuditDailyCacheRates query `cached*100/prompt`). Codex `inputTokens` already
        // includes cached input, so pass it through as-is — subtracting cached here
        // would inflate the computed cache-hit rate. This also matches OpenAI's real
        // `usage.prompt_tokens`, which is cache-inclusive.
        prompt_tokens: input,
        completion_tokens: output,
        cached_tokens: cached,
        total_tokens: total,
    })
}

/// Render a captured Codex usage breakdown into an OpenAI-shaped `usage` JSON
/// value (the shape `parse_chat_completion_response` deserializes and
/// `log_llm_usage_timed` reads). Returns `serde_json::Value::Null` when no
/// usage was observed so the response keeps the pre-fix `usage: null` and
/// logging is skipped rather than persisting a fabricated zero-token row.
fn codex_usage_to_openai_json(usage: Option<CodexAppServerUsage>) -> serde_json::Value {
    match usage {
        Some(u) => serde_json::json!({
            "prompt_tokens": u.prompt_tokens,
            "completion_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens,
            "prompt_tokens_details": { "cached_tokens": u.cached_tokens },
        }),
        None => serde_json::Value::Null,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexAppServerTurnOutput {
    FinalText(String),
    ToolCalls(Vec<CodexAppServerToolCall>),
}

/// A completed Codex app-server turn plus any usage telemetry observed during
/// the turn stream. `usage` is `None` when the server emitted no
/// `thread/tokenUsage/updated` notification (older servers / edge cases); in
/// that case the synthesized response keeps `usage: null` and logging skips —
/// matching the pre-fix behaviour rather than fabricating zeros.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAppServerTurn {
    output: CodexAppServerTurnOutput,
    usage: Option<CodexAppServerUsage>,
}

fn codex_app_server_model_attempts(primary: &str, retry_models: &[String]) -> Vec<String> {
    let mut attempts = Vec::new();
    for model in std::iter::once(primary.trim()).chain(retry_models.iter().map(|m| m.trim())) {
        if model.is_empty() || attempts.iter().any(|existing| existing == model) {
            continue;
        }
        attempts.push(model.to_string());
    }
    attempts
}

/// `(tool name, result)` pairs in call order, rebuilt from the transcript:
/// every executed tool call is an assistant message with `tool_calls` and a
/// tool message carrying its `tool_call_id`. `tools_called` zipped with
/// `previous_tool_results` was misaligned whenever a pseudo-tool (search_tools,
/// update_setting, …) recorded a name without a result — the server_monitor
/// output then landed under "search_tools" and the reply showed no evidence
/// (owner walk 2026-09-12).
pub(crate) fn tool_outputs_from_transcript(
    messages: &[ChatCompletionRequestMessage],
) -> Vec<(String, String)> {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut outputs = Vec::new();
    for message in messages {
        match message {
            ChatCompletionRequestMessage::Assistant(assistant) => {
                for call in assistant.tool_calls.iter().flatten() {
                    if let ChatCompletionMessageToolCalls::Function(call) = call {
                        names.insert(call.id.clone(), call.function.name.clone());
                    }
                }
            }
            ChatCompletionRequestMessage::Tool(tool) => {
                let Some(name) = names.get(&tool.tool_call_id) else {
                    continue;
                };
                let content = match &tool.content {
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                        text,
                    ) => text.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                outputs.push((name.clone(), content));
            }
            _ => {}
        }
    }
    outputs
}

fn codex_app_server_dynamic_tool_call(
    message: &serde_json::Value,
) -> Option<CodexAppServerToolCall> {
    let item = message.pointer("/params/item")?;
    if item.get("type").and_then(serde_json::Value::as_str) != Some("dynamicToolCall") {
        return None;
    }
    let id = item
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())?;
    let name = item
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())?;
    let arguments = item
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let arguments_json = serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
    Some(CodexAppServerToolCall {
        id,
        name,
        arguments_json,
    })
}

fn codex_app_server_tool_call_request(
    message: &serde_json::Value,
) -> Option<CodexAppServerToolCall> {
    if codex_app_server_method(message) != Some("item/tool/call") {
        return None;
    }
    let params = message.get("params")?;
    let id = params
        .get("callId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())?;
    let name = params
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let arguments_json = serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
    Some(CodexAppServerToolCall {
        id,
        name,
        arguments_json,
    })
}

fn codex_dynamic_tools_from_chat_tools(
    tools: Option<&[ChatCompletionTools]>,
) -> Vec<serde_json::Value> {
    let Some(tools) = tools else {
        return Vec::new();
    };
    tools
        .iter()
        .filter_map(|tool| {
            let ChatCompletionTools::Function(function_tool) = tool else {
                return None;
            };
            let function = &function_tool.function;
            let name = function.name.trim();
            if name.is_empty() {
                return None;
            }
            let input_schema = function
                .parameters
                .clone()
                .unwrap_or_else(|| serde_json::json!({"type": "object"}));
            Some(serde_json::json!({
                "name": name,
                "description": function.description.clone().unwrap_or_default(),
                "inputSchema": input_schema,
            }))
        })
        .collect()
}

/// Installer probe: same read-only app-server protocol as configured chat,
/// without dynamic tools, paid fallback, or persistence of call settings.
pub async fn verify_codex_connection(
    binary: &str,
    codex_home: &str,
    workdir: &str,
    model: &str,
    effort: Option<&str>,
) -> Result<()> {
    let turn = run_codex_app_server_chat(
        binary,
        codex_home,
        workdir,
        model,
        effort,
        &[],
        Duration::from_secs(55),
        "Reply with exactly AEGIS_OK. This is a connection test; do not use tools or read files.",
        &|_| {},
    )
    .await?;
    anyhow::ensure!(
        matches!(turn.output, CodexAppServerTurnOutput::FinalText(ref text) if !text.trim().is_empty()),
        "invalid probe response"
    );
    Ok(())
}

async fn run_codex_app_server_chat(
    binary: &str,
    codex_home: &str,
    workdir: &str,
    model: &str,
    reasoning_effort: Option<&str>,
    dynamic_tools: &[serde_json::Value],
    timeout: Duration,
    prompt: &str,
    on_agent_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<CodexAppServerTurn> {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        // Subscription chat and installer probes must not inherit a paid-key
        // override or the database key from their parent process.
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("XAI_API_KEY")
        .env_remove("AEGIS_RUNTIME_DB_KEY_HEX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !codex_home.trim().is_empty() {
        command.env("CODEX_HOME", codex_home);
        if let Some(parent) = std::path::Path::new(codex_home).parent() {
            command.env("HOME", parent);
        }
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn Codex app-server binary '{binary}'"))?;
    let mut stdin = child
        .stdin
        .take()
        .context("Codex app-server stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("Codex app-server stdout unavailable")?;
    let mut lines = BufReader::new(stdout).lines();

    let run = async {
        write_codex_app_server_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "aegis-runtime",
                        "title": "Aegis Runtime",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "requestAttestation": false,
                        "optOutNotificationMethods": [],
                    }
                }
            }),
        )
        .await?;

        loop {
            let Some(line) = lines
                .next_line()
                .await
                .context("read Codex app-server initialize response")?
            else {
                anyhow::bail!("Codex app-server exited before initialize completed");
            };
            let message: serde_json::Value =
                serde_json::from_str(&line).context("parse Codex app-server initialize line")?;
            if codex_app_server_response_id(&message) == Some(1) {
                if let Some(error) = codex_app_server_error_from_response(&message) {
                    anyhow::bail!("Codex app-server initialize failed: {error}");
                }
                break;
            }
        }

        write_codex_app_server_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialized",
            }),
        )
        .await?;

        write_codex_app_server_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "thread/start",
                "params": {
                    "model": model,
                    "cwd": workdir,
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                    "config": codex_chat_tool_config(),
                    "ephemeral": true,
                    "baseInstructions": codex_app_server_base_instructions(),
                    "developerInstructions": null,
                    "dynamicTools": dynamic_tools,
                    "environments": [],
                }
            }),
        )
        .await?;

        let thread_id = loop {
            let Some(line) = lines
                .next_line()
                .await
                .context("read Codex app-server thread/start response")?
            else {
                anyhow::bail!("Codex app-server exited before thread/start completed");
            };
            let message: serde_json::Value =
                serde_json::from_str(&line).context("parse Codex app-server thread/start line")?;
            if codex_app_server_response_id(&message) == Some(2) {
                if let Some(error) = codex_app_server_error_from_response(&message) {
                    anyhow::bail!("Codex app-server thread/start failed: {error}");
                }
                break message
                    .pointer("/result/thread/id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .context("Codex app-server thread/start response missing thread id")?;
            }
        };

        write_codex_app_server_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "turn/start",
                "params": {
                    "threadId": thread_id,
                    "input": [{
                        "type": "text",
                        "text": prompt,
                        "text_elements": [],
                    }],
                    "model": model,
                    "effort": reasoning_effort,
                    "cwd": workdir,
                    "environments": [],
                }
            }),
        )
        .await?;

        let mut final_text = String::new();
        let mut commentary_items = std::collections::HashSet::new();
        // Latest per-turn usage observed on the stream. The Codex app-server
        // emits `thread/tokenUsage/updated` (carrying the last turn's token
        // breakdown) shortly before it finishes a turn; capture it so the
        // synthesized response can populate `usage` and restore `llm_call_log`
        // persistence. None if the server never reports usage.
        let mut last_usage: Option<CodexAppServerUsage> = None;
        while let Some(line) = lines
            .next_line()
            .await
            .context("read Codex app-server turn line")?
        {
            let message: serde_json::Value =
                serde_json::from_str(&line).context("parse Codex app-server turn line")?;
            if codex_app_server_response_id(&message) == Some(3)
                && let Some(error) = codex_app_server_error_from_response(&message)
            {
                anyhow::bail!("Codex app-server turn/start failed: {error}");
            }
            if let Some(usage) = codex_app_server_turn_usage(&message) {
                last_usage = Some(usage);
            }
            if let Some(tool_call) = codex_app_server_tool_call_request(&message) {
                return Ok(CodexAppServerTurn {
                    output: CodexAppServerTurnOutput::ToolCalls(vec![tool_call]),
                    usage: last_usage,
                });
            }
            if codex_app_server_method(&message) == Some("item/started")
                && message
                    .pointer("/params/item/phase")
                    .and_then(serde_json::Value::as_str)
                    == Some("commentary")
                && let Some(id) = message
                    .pointer("/params/item/id")
                    .and_then(serde_json::Value::as_str)
            {
                commentary_items.insert(id.to_string());
            }
            if codex_app_server_method(&message) == Some("item/agentMessage/delta")
                && !message
                    .pointer("/params/itemId")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| commentary_items.contains(id))
                && let Some(delta) = message
                    .pointer("/params/delta")
                    .and_then(serde_json::Value::as_str)
            {
                final_text.push_str(delta);
                on_agent_delta(delta);
            }
            if codex_app_server_method(&message) == Some("item/completed")
                && let Some(text) = codex_app_server_final_agent_text(&message)
            {
                final_text = text.trim().to_string();
            }
            if codex_app_server_method(&message) == Some("turn/completed") {
                if let Some(error) = codex_app_server_turn_completed_error(&message) {
                    anyhow::bail!("Codex app-server turn failed: {error}");
                }
                if let Some(text) = codex_app_server_turn_completed_text(&message) {
                    return Ok(CodexAppServerTurn {
                        output: CodexAppServerTurnOutput::FinalText(text),
                        usage: last_usage,
                    });
                }
                let trimmed = final_text.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("Codex app-server turn completed without final assistant text");
                }
                return Ok(CodexAppServerTurn {
                    output: CodexAppServerTurnOutput::FinalText(trimmed.to_string()),
                    usage: last_usage,
                });
            }
            if codex_app_server_method(&message) == Some("item/started")
                && let Some(tool_call) = codex_app_server_dynamic_tool_call(&message)
            {
                return Ok(CodexAppServerTurn {
                    output: CodexAppServerTurnOutput::ToolCalls(vec![tool_call]),
                    usage: last_usage,
                });
            }
        }
        anyhow::bail!("Codex app-server exited before turn completed")
    };

    let result = tokio::time::timeout(timeout, run)
        .await
        .context("Codex app-server chat request timed out");
    let _ = stdin.shutdown().await;
    let _ = child.start_kill();
    result?
}

async fn summarize_messages(
    api_base: &str,
    api_key: &str,
    settings: &RuntimeSettings,
    model: &str,
    messages_json: &[String],
    did: &str,
    memory: &MemoryHandle,
) -> Result<()> {
    let combined = messages_json.join("\n---\n");

    // Truncate to avoid sending too much to the summarizer
    let input = if combined.len() > 8000 {
        let boundary = combined.floor_char_boundary(8000);
        format!("{}...(truncated)", &combined[..boundary])
    } else {
        combined
    };

    let system_prompt = "You are a context summarizer. Given a sequence of conversation messages, produce:\n\
        1. A brief CONTEXT SUMMARY (2-4 sentences) capturing the overall topic and key decisions.\n\
        2. A list of STRUCTURED FACTS as JSON array: [{\"topic\": \"...\", \"fact\": \"...\"}]\n\n\
        Respond in this exact format:\n\
        SUMMARY: <your summary>\n\
        FACTS: <JSON array>\n\n\
        Extract only genuinely useful facts (user preferences, decisions, technical details). \
        Skip greetings, acknowledgments, and trivial exchanges.";

    let client = llm_client::create_llm_client(api_key, api_base);
    let routed_model = route_summarization_model(&client, settings, model)?;

    let request = CreateChatCompletionRequestArgs::default()
        .model(&routed_model)
        .messages(vec![
            ChatCompletionRequestSystemMessageArgs::default()
                .content(system_prompt)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(&*input)
                .build()?
                .into(),
        ])
        .temperature(0.3_f32)
        .max_tokens(1024_u32)
        .build()?;

    let response = tokio::time::timeout(Duration::from_secs(30), client.chat().create(request))
        .await
        .context("summarization LLM call timed out")?
        .context("summarization LLM call failed")?;
    crate::llm_client::log_llm_usage(&response, "summarize");

    let reply = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("");

    // Parse response
    let summary = reply
        .lines()
        .find(|l| l.starts_with("SUMMARY:"))
        .map(|l| l.trim_start_matches("SUMMARY:").trim().to_string())
        .unwrap_or_else(|| reply.chars().take(TRUNCATE_PREVIEW_LONG).collect());

    let facts: Vec<(String, String)> = reply
        .lines()
        .find(|l| l.starts_with("FACTS:"))
        .and_then(|l| {
            let json_str = l.trim_start_matches("FACTS:").trim();
            serde_json::from_str::<Vec<serde_json::Value>>(json_str).ok()
        })
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| {
                    let topic = v.get("topic")?.as_str()?.to_string();
                    let fact = v.get("fact")?.as_str()?.to_string();
                    Some((topic, fact))
                })
                .collect()
        })
        .unwrap_or_default();

    let fact_count = facts.len();
    memory.store_summary(did, summary, facts).await?;

    tracing::info!(fact_count, "Background summarization completed");

    Ok(())
}

// sanitized_tool_error + json_parse_tool_error → moved to crate::utils

/// Test-only legacy predicate retained to pin the current-turn media scanning
/// contract while production routing uses capability-aware media planning.
#[cfg(test)]
fn messages_contain_routed_media(messages: &[ChatCompletionRequestMessage]) -> bool {
    let last_user = messages
        .iter()
        .rev()
        .find(|m| matches!(m, ChatCompletionRequestMessage::User(_)));
    match last_user {
        Some(ChatCompletionRequestMessage::User(u)) => match &u.content {
            ChatCompletionRequestUserMessageContent::Array(parts) => parts.iter().any(|p| {
                matches!(
                    p,
                    ChatCompletionRequestUserMessageContentPart::ImageUrl(_)
                        | ChatCompletionRequestUserMessageContentPart::File(_)
                )
            }),
            _ => false,
        },
        _ => false,
    }
}

/// Pure gate for vision routing. Returns `Some(vision_model)` iff ALL of:
/// (1) `vision_enabled` is true, (2) `vision_model` is non-empty,
/// (3) `vision_model != active_model`, (4) the latest user message in
/// `messages` carries media that should be routed from the active model to
/// the configured multimodal model.
///
/// Kept as a free fn so the full decision can be unit-tested without
/// a live `Orchestrator` instance.
fn decide_vision_routing(
    registry: &crate::model_capabilities::ModelCapabilityRegistry,
    vision_enabled: bool,
    vision_model: &str,
    active_model: &str,
    messages: &[ChatCompletionRequestMessage],
) -> Option<String> {
    if !vision_enabled {
        return None;
    }
    if vision_model.is_empty() {
        return None;
    }
    if vision_model == active_model {
        return None;
    }
    if !latest_user_message_requires_routed_model(registry, active_model, vision_model, messages) {
        return None;
    }
    Some(vision_model.to_string())
}

/// Decide whether a raw Telegram attachment should use the configured
/// multimodal routing target. Images intentionally route whenever the target
/// supports image input: `vision_model` is a quality/provider override, not
/// only a capability fallback. Documents and videos route only when the active
/// model lacks the needed modality to avoid unnecessary model switching.
fn attachment_has_routable_media(
    registry: &crate::model_capabilities::ModelCapabilityRegistry,
    attachment: &crate::multimodal::MediaAttachment,
    active_model: &str,
    target_model: &str,
) -> bool {
    use crate::model_capabilities::MediaPlan;

    let active_plan = MediaPlan::for_model(registry, active_model);
    let target_plan = MediaPlan::for_model(registry, target_model);

    match attachment {
        crate::multimodal::MediaAttachment::Image { .. } => target_plan.supports_image(),
        crate::multimodal::MediaAttachment::Document { .. } => {
            target_plan.supports_file() && !active_plan.supports_file()
        }
        crate::multimodal::MediaAttachment::Video { .. } => {
            target_plan.supports_video() && !active_plan.supports_video()
        }
        _ => false,
    }
}

fn latest_user_message_requires_routed_model(
    registry: &crate::model_capabilities::ModelCapabilityRegistry,
    active_model: &str,
    target_model: &str,
    messages: &[ChatCompletionRequestMessage],
) -> bool {
    use crate::model_capabilities::{MediaPlan, Modality};

    let active_plan = MediaPlan::for_model(registry, active_model);
    let target_plan = MediaPlan::for_model(registry, target_model);
    let last_user = messages
        .iter()
        .rev()
        .find(|m| matches!(m, ChatCompletionRequestMessage::User(_)));
    let Some(ChatCompletionRequestMessage::User(user)) = last_user else {
        return false;
    };
    let ChatCompletionRequestUserMessageContent::Array(parts) = &user.content else {
        return false;
    };

    parts.iter().any(|part| match part {
        ChatCompletionRequestUserMessageContentPart::ImageUrl(_) => target_plan.supports_image(),
        ChatCompletionRequestUserMessageContentPart::File(_) => {
            let Some(data_url) = file_part_data_url(part) else {
                return false;
            };
            if data_url.starts_with("data:video/") {
                target_plan.supports_video() && !active_plan.supports_video()
            } else {
                registry.supports_input(target_model, Modality::File)
                    && !active_plan.supports_file()
            }
        }
        _ => false,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RawMultimodalRequestPlan {
    has_video_part: bool,
    has_pdf_part: bool,
}

impl RawMultimodalRequestPlan {
    fn needs_raw_http(self) -> bool {
        self.has_video_part || self.has_pdf_part
    }
}

fn raw_multimodal_request_plan(request: &CreateChatCompletionRequest) -> RawMultimodalRequestPlan {
    let mut plan = RawMultimodalRequestPlan::default();
    for message in &request.messages {
        let ChatCompletionRequestMessage::User(user) = message else {
            continue;
        };
        let ChatCompletionRequestUserMessageContent::Array(parts) = &user.content else {
            continue;
        };
        for part in parts {
            let Some(data_url) = file_part_data_url(part) else {
                continue;
            };
            if data_url.starts_with("data:video/") {
                plan.has_video_part = true;
            } else if data_url.starts_with("data:application/pdf;") {
                plan.has_pdf_part = true;
            }
        }
    }
    plan
}

fn file_part_data_url(part: &ChatCompletionRequestUserMessageContentPart) -> Option<String> {
    if !matches!(part, ChatCompletionRequestUserMessageContentPart::File(_)) {
        return None;
    }
    // async-openai does not expose file_data on the enum wrapper. Keep this
    // pointer local and covered by tests so upstream serde shape changes fail
    // close to the OpenRouter PDF/video request-shaping path.
    match serde_json::to_value(part) {
        Ok(value) => value
            .pointer("/file/file_data")
            .and_then(Value::as_str)
            .map(str::to_string),
        Err(e) => {
            tracing::warn!(error = %e, "[multimodal] failed to inspect file content part");
            None
        }
    }
}

fn normalize_openrouter_video_parts(body: &mut Value) -> bool {
    let mut changed = false;
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return false;
    };

    for message in messages {
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            if part.get("type").and_then(Value::as_str) != Some("file") {
                continue;
            }
            let Some(data_url) = part
                .get("file")
                .and_then(|f| f.get("file_data"))
                .and_then(Value::as_str)
                .filter(|s| s.starts_with("data:video/"))
                .map(str::to_string)
            else {
                continue;
            };
            *part = serde_json::json!({
                "type": "video_url",
                "video_url": { "url": data_url }
            });
            changed = true;
        }
    }
    changed
}

fn ensure_openrouter_pdf_parser_plugin(body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    let plugins = obj
        .entry("plugins")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(arr) = plugins.as_array_mut() else {
        tracing::warn!(
            "[multimodal] request already contains non-array plugins; preserving user value"
        );
        return false;
    };

    for plugin in arr.iter_mut() {
        if plugin.get("id").and_then(Value::as_str) != Some("file-parser") {
            continue;
        }
        if plugin.pointer("/pdf/engine").is_some() {
            return false;
        }
        if let Some(plugin_obj) = plugin.as_object_mut() {
            plugin_obj.insert(
                "pdf".to_string(),
                serde_json::json!({ "engine": "cloudflare-ai" }),
            );
            return true;
        }
    }

    arr.push(serde_json::json!({
        "id": "file-parser",
        "pdf": { "engine": "cloudflare-ai" }
    }));
    true
}

fn parse_chat_completion_response(text: &str) -> Result<CreateChatCompletionResponse> {
    let mut value: Value =
        serde_json::from_str(text).context("chat completion response was not valid JSON")?;
    sanitize_openrouter_chat_completion_response(&mut value);
    serde_json::from_value(value).context("chat completion response was not OpenAI-compatible")
}

pub(super) use crate::llm::xai_subscription::prepare_xai_subscription_chat_body;

fn sanitize_openrouter_chat_completion_response(value: &mut Value) {
    if let Some(obj) = value.as_object_mut()
        && obj.get("service_tier").and_then(Value::as_str) == Some("standard")
    {
        obj.remove("service_tier");
    }

    let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        if let Some(choice_obj) = choice.as_object_mut() {
            choice_obj.remove("native_finish_reason");
            if let Some(message_obj) = choice_obj.get_mut("message").and_then(Value::as_object_mut)
            {
                message_obj.remove("reasoning_details");
                message_obj.remove("reasoning_content");
                message_obj.remove("reasoning");
            }
        }
    }
}

fn file_content_part(
    data_url: &str,
    filename: &str,
) -> Option<ChatCompletionRequestUserMessageContentPart> {
    use async_openai::types::chat::{ChatCompletionRequestMessageContentPartFile, FileObject};

    let file_obj: FileObject = match serde_json::from_value(serde_json::json!({
        "file_data": data_url,
        "filename": filename,
    })) {
        Ok(obj) => obj,
        Err(e) => {
            tracing::warn!(
                filename,
                error = %e,
                "[multimodal] failed to build file content part"
            );
            return None;
        }
    };

    Some(ChatCompletionRequestUserMessageContentPart::File(
        ChatCompletionRequestMessageContentPartFile { file: file_obj },
    ))
}

mod heartbeat;
mod lifecycle;
mod message_flow;
mod tools_dispatch;
#[cfg(test)]
pub(crate) use message_flow::record_unanswered_owner_turn;
pub(crate) use message_flow::{OwnerFailureDetail, classify_owner_failure};
pub(crate) use tools_dispatch::skill_needs_google_account;

#[cfg(test)]
mod tests;

// kb_entry_to_json → moved to crate::utils

// fetch_openrouter_usage, fetch_openrouter_credits, fetch_openrouter_credits_raw,
// fetch_openrouter_activity, fetch_openrouter_activity_raw → moved to crate::openrouter

// ---------------------------------------------------------------------------
// Multimodal helpers
// ---------------------------------------------------------------------------

/// Decode base64 image data and send as photo to a Telegram chat.
pub(crate) async fn send_image_b64_to_telegram(b64: &str, bot: &teloxide::Bot, cid: i64) -> String {
    use teloxide::prelude::Requester;
    let raw_b64 = b64.find(',').map(|i| &b64[i + 1..]).unwrap_or(b64);
    match crate::multimodal::decode_base64_standard(raw_b64) {
        Ok(img_bytes) => {
            let input = teloxide::types::InputFile::memory(img_bytes).file_name("generated.png");
            use teloxide::payloads::SendPhotoSetters;
            let thread = crate::telegram::wrap_thread_id(
                crate::topic_router::resolve_thread_id(cid, crate::topic_router::TopicKind::Main)
                    .await,
            );
            let mut req = bot.send_photo(crate::telegram::admin_dm_chat(cid), input);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            match req.await {
                Ok(_) => {
                    "{\"status\":\"success\",\"message\":\"Image generated and sent to user.\"}"
                        .to_string()
                }
                Err(e) => sanitized_tool_error(
                    "generate_image",
                    "Failed to send image / 发送图片失败",
                    &e,
                    "telegram_error",
                    true,
                    None,
                ),
            }
        }
        Err(e) => sanitized_tool_error(
            "generate_image",
            "Base64 decode failed / Base64解码失败",
            &e,
            "invalid_input",
            false,
            None,
        ),
    }
}

// parse_timezone_offset → moved to crate::utils

/// Extract URL host from tool arguments for domain-scoped approvals.
/// Only meaningful for web-related tools.
fn extract_url_host(tool_name: &str, tool_args: &str) -> Option<String> {
    if tool_name != "web_search" && tool_name != "web_reader" {
        return None;
    }
    if tool_args.len() > 100_000 {
        return None; // reject oversized tool args
    }
    let parsed: serde_json::Value = serde_json::from_str(tool_args).ok()?;
    let url_str = parsed.get("url").and_then(|v| v.as_str())?;
    url::Url::parse(url_str)
        .ok()
        .map(|u| u.host_str().unwrap_or("").to_string())
        .filter(|h| !h.is_empty())
}

// base64url_encode → moved to crate::openrouter

fn gate_pending_tool_name_for_owner(tool_name: &str) -> &'static str {
    match tool_name {
        "setup_google" => "Google connection change",
        "update_setting" => "Settings change",
        "sysadmin_ops" => "System operation",
        "manage_skill" => "Skill change",
        "approve_skill" => "Skill approval",
        _ => "Tool action",
    }
}

fn gate_pending_tool_list_for_owner(tool_names: &[String]) -> String {
    let mut labels = tool_names
        .iter()
        .map(|tool| gate_pending_tool_name_for_owner(tool).to_string())
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels.join(", ")
}

fn deterministic_reply_from_recent_tool_result(
    tools_called: &[String],
    previous_tool_results: &[String],
) -> Option<String> {
    let tool = tools_called.last()?.as_str();
    let raw = previous_tool_results.last()?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(raw).ok();
    if let Some(value) = parsed.as_ref() {
        let status = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let message = value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("summary").and_then(serde_json::Value::as_str))
            .filter(|text| !text.trim().is_empty());
        let error = value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.trim().is_empty());
        if status == "success" {
            let message = message.unwrap_or("The tool returned success.");
            let image_url = value
                .get("image_url")
                .and_then(serde_json::Value::as_str)
                .filter(|text| text.starts_with("/webapp/vnext/generated-image/"));
            return Some(match tool {
                "generate_image" if image_url.is_some() => {
                    let image_url = image_url.unwrap();
                    format!("{message}\n\n![Generated image]({image_url})")
                }
                "generate_image" => message.to_string(),
                _ => message.to_string(),
            });
        }
        if let Some(error) = error {
            return Some(format!("I could not finish that: {error}"));
        }
    }
    Some(format!(
        "I ran {} and received: {}",
        gate_pending_tool_name_for_owner(tool),
        truncate_str(raw, TRUNCATE_PREVIEW_MEDIUM)
    ))
}

/// Execute the `view_logs` pseudo-tool: read journalctl logs for the aegis-runtime service.
///
/// Runs `journalctl -u aegis-runtime` with optional `--lines`, `--since`, and `-p` flags.
/// Applies an optional case-insensitive text filter in-memory.
/// Output is capped at 32 KB to avoid overwhelming the LLM context.
pub(crate) async fn execute_view_logs(args: &Value) -> String {
    const MAX_LINES: u64 = 200;
    const DEFAULT_LINES: u64 = 50;
    const MAX_OUTPUT_BYTES: usize = 32 * 1024;
    const TIMEOUT_SECS: u64 = 5;

    let lines = args
        .get("lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_LINES)
        .min(MAX_LINES);

    let since = args.get("since").and_then(|v| v.as_str());
    let filter = args.get("filter").and_then(|v| v.as_str());
    let priority = args
        .get("priority")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    let mut cmd = tokio::process::Command::new("journalctl");
    cmd.arg("-u")
        .arg("aegis-runtime")
        .arg("--no-pager")
        .arg("-o")
        .arg("cat")
        .arg("--lines")
        .arg(lines.to_string());

    if let Some(s) = since {
        cmd.arg("--since").arg(s);
    }

    match priority {
        "error" => {
            cmd.arg("-p").arg("err");
        }
        "warning" => {
            cmd.arg("-p").arg("warning");
        }
        "info" => {
            cmd.arg("-p").arg("info");
        }
        _ => {} // "all" — no filter
    }

    let output = match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return serde_json::json!({
                "status": "error",
                "error": format!("failed to run journalctl: {e}"),
            })
            .to_string();
        }
        Err(_) => {
            return serde_json::json!({
                "status": "error",
                "error": "journalctl timed out after 5 seconds",
            })
            .to_string();
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return serde_json::json!({
            "status": "error",
            "error": format!("journalctl failed: {}", stderr.trim()),
        })
        .to_string();
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let all_lines: Vec<&str> = raw.lines().collect();
    let total_lines = all_lines.len();

    // Apply case-insensitive filter if provided
    let filtered: Vec<&str> = if let Some(f) = filter {
        let f_lower = f.to_lowercase();
        all_lines
            .into_iter()
            .filter(|line| line.to_lowercase().contains(&f_lower))
            .collect()
    } else {
        all_lines
    };

    let returned_lines = filtered.len();

    // Cap output at MAX_OUTPUT_BYTES
    let mut result_lines: Vec<&str> = Vec::new();
    let mut byte_count = 0usize;
    let mut truncated = false;
    for line in &filtered {
        let line_bytes = line.len() + 1; // +1 for newline
        if byte_count + line_bytes > MAX_OUTPUT_BYTES {
            truncated = true;
            break;
        }
        byte_count += line_bytes;
        result_lines.push(line);
    }

    serde_json::json!({
        "status": "ok",
        "total_lines": total_lines,
        "returned_lines": returned_lines,
        "displayed_lines": result_lines.len(),
        "truncated": truncated,
        "logs": result_lines,
    })
    .to_string()
}

/// Execute the `set_log_level` pseudo-tool: get or change the runtime log filter.
pub(crate) fn execute_set_log_level(args: &Value) -> String {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");

    match action {
        "get" => {
            let current = crate::logging::current_level();
            serde_json::json!({
                "status": "ok",
                "current_level": current,
            })
            .to_string()
        }
        "set" => {
            let level = match args.get("level").and_then(|v| v.as_str()) {
                Some(l) if !l.is_empty() => l,
                _ => {
                    return serde_json::json!({
                        "status": "error",
                        "error": "'level' parameter is required for 'set' action",
                    })
                    .to_string();
                }
            };

            match crate::logging::set_level(level) {
                Ok(()) => serde_json::json!({
                    "status": "ok",
                    "new_level": level,
                    "message": "Log level updated. Use view_logs to verify.",
                })
                .to_string(),
                Err(e) => serde_json::json!({
                    "status": "error",
                    "error": e,
                })
                .to_string(),
            }
        }
        _ => serde_json::json!({
            "status": "error",
            "error": format!("unknown action: {action}. Use 'get' or 'set'."),
        })
        .to_string(),
    }
}

// truncate_str, extract_text_content, sanitize_model_xml, classify_skill_error
// → moved to crate::utils

// Agent review and notification evaluator prompts, types, and parsing
// functions have been extracted to `crate::evaluators`.

// All prompt_module_* functions and core_system_prompt / default_system_prompt
// have been extracted to `crate::prompts`.

// (prompt_module_proactive through default_system_prompt removed — see crate::prompts)
