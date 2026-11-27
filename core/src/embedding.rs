use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Client for OpenAI-compatible embedding APIs (OpenRouter, OpenAI, etc).
///
/// Converts text into dense vector representations for semantic similarity search.
/// Uses Matryoshka dimension truncation for storage efficiency.
///
/// # Primary / backup auto-failover
///
/// The client can hold an optional `backup` model configuration (same `api_base`,
/// `api_key`, and `dimensions` as the primary, different `model`). When a primary
/// `embed()` call fails with an HTTP transport error, a non-2xx status, or a JSON
/// parse error, the client transparently retries with the backup model and enters
/// **degraded mode** (tracked by a shared `FailoverState`). Inside `embed()`, after
/// a cooldown (default 60s), an automatic recovery probe is issued against the
/// primary ("ping"). On success the state flips back to primary and degraded mode
/// is cleared.
///
/// ## Cross-model compatibility policy (approach b)
///
/// Different embedding models produce non-comparable vectors even at matching
/// dimensions: an HNSW index built on Gemini embeddings cannot be queried with
/// OpenAI text-embedding-3 vectors and yield meaningful cosine similarity.
///
/// The client handles this by exposing [`EmbeddingClient::is_degraded`] so that
/// **write-path callers** (those persisting vectors into durable, indexed storage
/// such as `l2_experience.embedding` or the HNSW graph) can check the mode and
/// skip persistence when degraded, falling back to the existing "store fact
/// without embedding" graceful path. The background `reembed_stale_facts`
/// migration will pick up the NULL-embedding rows on next startup once the
/// primary has recovered.
///
/// **Transient callers** (within-call similarity comparisons — dedup
/// dedup, pre-filter clustering, health probe) can ignore the degraded flag:
/// every vector compared in the same call comes from the same backend, so the
/// cosine math is internally consistent.
///
/// # 24h permanent-backup-promotion
///
/// When the primary has been down continuously for ≥ `promote_after_secs`
/// (default 86_400 = 24 h), the client **permanently** swaps the backup into
/// the primary slot and clears the backup slot (`backup = None`). A one-shot
/// alert payload is queued and consumed via the peek/clear pattern
/// ([`EmbeddingClient::peek_pending_promotion_alert`] +
/// [`EmbeddingClient::clear_pending_promotion_alert`]) so the health-probe
/// task only clears the queue after DB persist succeeds. Telegram send is
/// attempted once; failure is non-critical (DB is source of truth for
/// restart-time recovery).
///
/// Rationale: once the primary has been unavailable for a full day, treat the
/// old primary as a dead letter rather than demoting it to backup (which would
/// risk the same outage repeating).
///
/// ## Hot-reload revert
///
/// The client captures the original env-var-configured primary and backup
/// names at construction (`env_primary_model` / `env_backup_model`, immutable
/// after construction). The [`EmbeddingClient::revert_promotion`] method
/// restores those originals into the live state atomically — no process
/// restart needed. Invoked by the `unset_embedding_primary_promotion` tool
/// after clearing the DB settings.
///
/// **Surprising case**: revert restores the values captured AT STARTUP, not
/// whatever is currently in `/opt/aegis-runtime/shared/.env`. If the operator
/// edits the env file between startup and revert, those edits require a
/// process restart to take effect (revert will not pick them up).
#[derive(Clone)]
pub struct EmbeddingClient {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    /// Vector dimensions. Stored atomically so a settings-driven hot-reload
    /// via [`Self::apply_settings_update`] takes effect across every clone.
    /// A dim change is surfaced as [`UpdateOutcome::RequiresCorpusReembed`]
    /// — any in-place persisted vector of a different dim is incomparable.
    dimensions: Arc<AtomicU32>,
    /// Shared mutable state. Cloning the client shares the state so all clones
    /// see the same degraded/promotion flags, the same recovery probe cooldown,
    /// AND the same live `primary_model` / `backup_model` (so post-promotion
    /// every clone observes the swap).
    state: Arc<FailoverStateInner>,
    /// How long to wait between recovery probes to the primary, in milliseconds.
    /// Stored atomically so a settings hot-reload is observed across clones
    /// without re-threading a mutex guard through the hot `embed()` path.
    /// Default 60_000 ms (60 s).
    probe_cooldown_ms: Arc<AtomicU64>,
    /// How long the primary must be continuously down before the backup is
    /// permanently promoted to primary, in seconds. Atomic for hot-reload.
    /// Default 86_400 s (24 h).
    promote_after_secs: Arc<AtomicU64>,
    /// Original env-var-configured primary model name, captured at construction.
    /// Immutable after construction (hence outside the mutex — zero lock cost).
    /// Used by [`EmbeddingClient::revert_promotion`] to restore the env config
    /// after a permanent promotion has been reverted via the
    /// `unset_embedding_primary_promotion` tool.
    env_primary_model: String,
    /// Original env-var-configured backup model, captured at construction.
    /// Immutable. See [`env_primary_model`](Self::env_primary_model).
    env_backup_model: Option<String>,
}

/// Internal failover state protected by a Mutex.
///
/// Uses `Instant` for monotonic fields (`degraded_since_instant`, `last_probe_at`)
/// so durations cannot go "backwards" due to NTP adjustments, and `SystemTime`
/// for human-readable fields (`degraded_since_wall`) that must survive into
/// log/alert messages.
#[derive(Debug)]
struct FailoverStateInner {
    mu: std::sync::Mutex<FailoverState>,
}

#[derive(Debug, Clone)]
struct FailoverState {
    /// Live primary model name. Swapped with `backup_model` on permanent
    /// promotion; persisted in memory only (DB persistence is handled by
    /// main.rs after draining [`FailoverStateInner::take_pending_promotion`]).
    primary_model: String,
    /// Live backup model name. `None` after a permanent promotion has fired
    /// (per design: old primary is abandoned, not demoted) or if the client
    /// was constructed without a backup.
    backup_model: Option<String>,
    /// If `Some`, the primary has been failing and we are using backup.
    /// Monotonic value for elapsed-duration comparison (NTP-safe).
    degraded_since_instant: Option<Instant>,
    /// Wall-clock sibling of `degraded_since_instant`, captured at the same
    /// transition. Used for human-readable alert messages and future
    /// serialization (spec §1).
    degraded_since_wall: Option<SystemTime>,
    /// Last time we probed the primary for recovery. Used to rate-limit probes.
    last_probe_at: Option<Instant>,
    /// True once the 24h permanent-promotion path has fired in the CURRENT
    /// degraded window. Prevents re-firing on every subsequent `embed()` call.
    /// Cleared by `mark_recovered` and by the promotion path itself (so a
    /// future degradation on the NEWLY-promoted primary starts a fresh clock).
    promotion_fired: bool,
    /// One-shot alert payload queued by the promotion path. Drained by
    /// main.rs's health-probe tick: it persists `(from, to, at)` to the
    /// user_settings table and emits a distinct Telegram alert, then this
    /// field goes back to `None`.
    pending_promotion_alert: Option<PendingPromotion>,
}

/// Payload for a pending promotion notification. See `FailoverState::pending_promotion_alert`.
#[derive(Debug, Clone)]
pub struct PendingPromotion {
    /// Old primary that timed out after 24h of continuous failure.
    pub from: String,
    /// Backup that has been promoted to be the new primary.
    pub to: String,
    /// UTC wall-clock timestamp when the promotion fired (ISO-8601).
    pub at_iso: String,
}

/// Outcome of a [`EmbeddingClient::revert_promotion`] call.
///
/// Produced by the hot-reload revert path of the
/// `unset_embedding_primary_promotion` tool. Consumed by the handler to build
/// the JSON tool result returned to the LLM.
#[derive(Debug, Clone)]
pub struct RevertOutcome {
    /// `true` if the live primary differed from the env-original primary
    /// (i.e. a permanent promotion was active and has now been reverted).
    /// `false` if the revert was a no-op (live primary already equals env).
    pub was_promoted: bool,
    /// The live primary model name AS IT WAS immediately before the revert.
    /// When `was_promoted = true`, this is the promoted (DB-override) model.
    pub reverted_from: String,
    /// The env-original primary model name now in effect.
    pub reverted_to: String,
}

/// Outcome of a [`EmbeddingClient::apply_settings_update`] call.
///
/// Produced when a runtime setting for an embedding-related key is
/// approved via the HITL-gated `update_setting` flow. Used by the
/// approval handler to shape the LLM callback message (in particular
/// so the LLM knows when to surface a corpus-reembed consent alert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The setting change was accepted but the embedder state was already
    /// matching the new value (idempotent re-apply, e.g. value unchanged).
    SilentOk,
    /// The setting was applied atomically to the live embedder state —
    /// includes the previous and new values for observability.
    HotReloaded { from: String, to: String },
    /// The setting was applied, AND the stored corpus is now produced by a
    /// different model / dimension than the new live config. The caller
    /// should prompt the user to run `trigger_corpus_reembed`.
    RequiresCorpusReembed { old: String, new: String },
    /// The setting key is not one this client knows how to hot-reload
    /// (caller may have mis-dispatched) or the value failed validation.
    Rejected { reason: String },
}

/// A validated local update. Preparation performs no I/O or live mutation;
/// applying it after a durable commit cannot reject a candidate.
pub(crate) enum PreparedEmbeddingUpdate {
    Primary(String),
    Backup(Option<String>),
    Dimensions(u32),
    Cooldown(u64),
    PromoteAfter(u64),
}

impl PreparedEmbeddingUpdate {
    pub(crate) fn prepare(key: &str, value: &serde_json::Value) -> Result<Self, String> {
        match key {
            "embedding_primary_model" => {
                let new_primary = match value.as_str() {
                    Some(s) if !s.trim().is_empty() && s.len() <= 128 => s.trim().to_string(),
                    Some(_) => {
                        return Err(
                            "embedding_primary_model must be a non-empty string ≤ 128 chars".into(),
                        );
                    }
                    None => {
                        return Err("embedding_primary_model must be a string".into());
                    }
                };
                Ok(Self::Primary(new_primary))
            }
            "embedding_backup_model" => {
                let new_backup = match value.as_str() {
                    Some(s) if s.trim().is_empty() => None,
                    Some(s) if s.len() <= 128 => Some(s.trim().to_string()),
                    Some(_) => {
                        return Err("embedding_backup_model must be ≤ 128 chars".into());
                    }
                    None => {
                        return Err(
                            "embedding_backup_model must be a string (empty = disable)".into()
                        );
                    }
                };
                Ok(Self::Backup(new_backup))
            }
            "embedding_dimensions" => {
                let n = match value.as_u64() {
                    Some(n) if (1..=16_384).contains(&n) => n as u32,
                    _ => {
                        return Err("embedding_dimensions must be an integer in 1..=16384".into());
                    }
                };
                Ok(Self::Dimensions(n))
            }
            "embedding_probe_cooldown_secs" => {
                let secs = match value.as_u64() {
                    Some(n) if (1..=86_400).contains(&n) => n,
                    _ => {
                        return Err(
                            "embedding_probe_cooldown_secs must be an integer in 1..=86400".into(),
                        );
                    }
                };
                Ok(Self::Cooldown(secs))
            }
            "embedding_promote_after_secs" => {
                let secs = match value.as_u64() {
                    Some(n) if (3_600..=604_800).contains(&n) => n,
                    _ => {
                        return Err(
                            "embedding_promote_after_secs must be an integer in 3600..=604800"
                                .into(),
                        );
                    }
                };
                Ok(Self::PromoteAfter(secs))
            }
            _ => Err(format!("apply_settings_update: unknown key `{key}`")),
        }
    }

    pub(crate) fn value(&self) -> serde_json::Value {
        match self {
            Self::Primary(value) => serde_json::json!(value),
            Self::Backup(value) => serde_json::json!(value.clone().unwrap_or_default()),
            Self::Dimensions(value) => serde_json::json!(value),
            Self::Cooldown(value) | Self::PromoteAfter(value) => serde_json::json!(value),
        }
    }
}

impl FailoverStateInner {
    fn new(primary_model: String, backup_model: Option<String>) -> Self {
        Self {
            mu: std::sync::Mutex::new(FailoverState {
                primary_model,
                backup_model,
                degraded_since_instant: None,
                degraded_since_wall: None,
                last_probe_at: None,
                promotion_fired: false,
                pending_promotion_alert: None,
            }),
        }
    }

    fn mark_degraded(&self) {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        if g.degraded_since_instant.is_none() {
            g.degraded_since_instant = Some(Instant::now());
            g.degraded_since_wall = Some(SystemTime::now());
        }
    }

    fn mark_recovered(&self) {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        g.degraded_since_instant = None;
        g.degraded_since_wall = None;
        g.last_probe_at = None;
        g.promotion_fired = false;
    }

    fn record_probe(&self) {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        g.last_probe_at = Some(Instant::now());
    }

    fn is_degraded(&self) -> bool {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .degraded_since_instant
            .is_some()
    }

    fn last_probe_at(&self) -> Option<Instant> {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_probe_at
    }

    fn primary_model(&self) -> String {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .primary_model
            .clone()
    }

    fn backup_model(&self) -> Option<String> {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .backup_model
            .clone()
    }

    /// Check whether the degraded window has exceeded `promote_after_secs` and
    /// the promotion has not yet fired in this window. If so, atomically:
    /// - swap `primary_model` ↔ `backup_model` (old primary abandoned; new
    ///   client has NO backup, see module doc),
    /// - clear degraded flags (`degraded_since_*`, `last_probe_at`) so the
    ///   NEW primary starts a fresh clock on its next failure,
    /// - set `promotion_fired = false` (fresh window on the new primary),
    /// - queue a `PendingPromotion` payload for main.rs to persist + alert.
    ///
    /// No-ops if `backup_model` is `None` or `degraded_since_*` is `None`
    /// — both are guarded because a race between `mark_recovered` and this
    /// method must be safe.
    fn check_and_fire_promotion(&self, promote_after_secs: u64) {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());

        // Already fired in this window, or not degraded, or no backup: bail.
        if g.promotion_fired {
            return;
        }
        let Some(since_inst) = g.degraded_since_instant else {
            return;
        };
        if g.backup_model.is_none() {
            return;
        }
        if since_inst.elapsed() < Duration::from_secs(promote_after_secs) {
            return;
        }

        // All gates passed — perform the swap.
        let old_primary = std::mem::take(&mut g.primary_model);
        let new_primary = g.backup_model.take().expect("backup_model checked above");
        g.primary_model = new_primary.clone();
        // g.backup_model is already None (took above) — explicit for clarity.
        g.backup_model = None;

        // Fresh state on the newly-promoted primary.
        g.degraded_since_instant = None;
        g.degraded_since_wall = None;
        g.last_probe_at = None;
        g.promotion_fired = false;

        // Queue the one-shot alert for main.rs to drain.
        let at_iso = chrono::Utc::now().to_rfc3339();
        g.pending_promotion_alert = Some(PendingPromotion {
            from: old_primary.clone(),
            to: new_primary.clone(),
            at_iso: at_iso.clone(),
        });

        tracing::warn!(
            from = %old_primary,
            to = %new_primary,
            at = %at_iso,
            threshold_secs = promote_after_secs,
            "[Embedding] Primary DOWN for \u{2265} threshold \u{2014} promoting backup to permanent primary"
        );
    }

    fn take_pending_promotion(&self) -> Option<PendingPromotion> {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_promotion_alert
            .take()
    }

    /// Atomically restore the given env-original primary/backup names into the
    /// live state and clear all promotion/degradation flags. Returns the
    /// outcome struct describing the swap.
    ///
    /// Called by [`EmbeddingClient::revert_promotion`]. Kept as a method on
    /// the inner state so the restore runs under a single mutex acquisition.
    fn revert_to_env(&self, env_primary: &str, env_backup: Option<String>) -> RevertOutcome {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let live_primary = g.primary_model.clone();
        let was_promoted = live_primary != env_primary;

        // Restore live state to env-original config.
        g.primary_model = env_primary.to_string();
        g.backup_model = env_backup;

        // Wipe all degradation/promotion bookkeeping so the next embed()
        // starts a fresh cycle. The revert does NOT issue a probe itself —
        // if the restored primary is still dead, embed() will discover that
        // on the next call and re-enter the normal failover path.
        g.degraded_since_instant = None;
        g.degraded_since_wall = None;
        g.last_probe_at = None;
        g.promotion_fired = false;
        g.pending_promotion_alert = None;

        RevertOutcome {
            was_promoted,
            reverted_from: live_primary,
            reverted_to: env_primary.to_string(),
        }
    }

    fn peek_pending_promotion(&self) -> Option<PendingPromotion> {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_promotion_alert
            .clone()
    }

    fn clear_pending_promotion(&self) {
        self.mu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_promotion_alert = None;
    }

    /// Atomically swap the live primary model to `new_primary`. Returns the
    /// old primary name. Does NOT touch the backup slot. Clears
    /// degradation/promotion bookkeeping so the next `embed()` starts a
    /// fresh cycle against the new primary — any in-flight probe state from
    /// the previous primary is irrelevant.
    ///
    /// Used by [`EmbeddingClient::apply_settings_update`] for the
    /// `embedding_primary_model` key. The env-original primary is NOT
    /// updated — revert still restores the boot-time config by design.
    fn set_live_primary(&self, new_primary: &str) -> String {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::mem::replace(&mut g.primary_model, new_primary.to_string());
        g.degraded_since_instant = None;
        g.degraded_since_wall = None;
        g.last_probe_at = None;
        g.promotion_fired = false;
        g.pending_promotion_alert = None;
        old
    }

    /// Atomically swap the live backup model. `None` disables failover.
    /// Returns the old backup name (if any). Does not touch the primary
    /// slot. Clears degradation state for the same reason as
    /// [`Self::set_live_primary`].
    fn set_live_backup(&self, new_backup: Option<String>) -> Option<String> {
        let mut g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::mem::replace(&mut g.backup_model, new_backup);
        g.degraded_since_instant = None;
        g.degraded_since_wall = None;
        g.last_probe_at = None;
        g.promotion_fired = false;
        g.pending_promotion_alert = None;
        old
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
    dimensions: u32,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Classifies the outcome of an embed HTTP call for failover routing.
#[derive(Debug)]
enum EmbedError {
    /// Transport-layer error (network, TLS, timeout) — failover-eligible.
    Transport(anyhow::Error),
    /// API returned a non-2xx status — failover-eligible.
    ApiStatus { status: u16, body: String },
    /// Response body failed to parse as JSON — failover-eligible.
    ParseError(anyhow::Error),
    /// API returned an empty `data` array — NOT failover-eligible (legitimate
    /// data issue, same failure would happen against backup).
    EmptyData,
    /// Embedding endpoint / key is not configured (empty base, empty key, or
    /// `.env.example` placeholder). Callers MUST surface install steps — do
    /// not treat this as a transient 401 and silently drop L2.
    NotConfigured { reason: String },
}

impl From<EmbedError> for anyhow::Error {
    fn from(e: EmbedError) -> Self {
        match e {
            EmbedError::Transport(err) => err.context("embedding API request failed"),
            EmbedError::ApiStatus { status, body } => {
                anyhow::anyhow!("embedding API returned {status}: {body}")
            }
            EmbedError::ParseError(err) => err,
            EmbedError::EmptyData => anyhow::anyhow!("embedding API returned empty data array"),
            EmbedError::NotConfigured { reason } => anyhow::anyhow!(
                "embedding not configured: {reason}. Install the local dependency: \
                 scripts/ensure-local-embedding.sh  (Ollama + nomic-embed-text), or set \
                 EMBEDDING_API_BASE + EMBEDDING_API_KEY / a usable OPENAI_API_KEY"
            ),
        }
    }
}

impl EmbedError {
    /// Whether this error class should trigger a backup failover attempt.
    fn is_failover_eligible(&self) -> bool {
        matches!(
            self,
            EmbedError::Transport(_) | EmbedError::ApiStatus { .. } | EmbedError::ParseError(_)
        )
    }
}

impl EmbeddingClient {
    /// Create a new embedding client without a backup model.
    ///
    /// Reuses the same API base and key as the LLM client (OpenRouter).
    pub fn new(api_base: &str, api_key: &str, model: &str, dimensions: u32) -> Self {
        Self::new_with_backup(api_base, api_key, model, None, dimensions)
    }

    /// Create a new embedding client with an optional backup model.
    ///
    /// If `backup_model` is `Some`, `embed()` will fail over to it on transport,
    /// non-2xx, or parse errors from the primary, and enter degraded mode until
    /// the primary recovers (probed automatically after cooldown — default 60s;
    /// use [`new_with_backup_and_cooldown`] or [`set_probe_cooldown`] to override).
    ///
    /// Primary and backup share `api_base`, `api_key`, and `dimensions` — this
    /// is a hard requirement because HNSW / l2_experience storage assumes a
    /// single vector dimension.
    pub fn new_with_backup(
        api_base: &str,
        api_key: &str,
        model: &str,
        backup_model: Option<String>,
        dimensions: u32,
    ) -> Self {
        Self::new_with_backup_and_cooldown(
            api_base,
            api_key,
            model,
            backup_model,
            dimensions,
            Duration::from_secs(60),
        )
    }

    /// Like [`new_with_backup`] but with a configurable recovery-probe cooldown.
    ///
    /// Exposed so callers can read `EMBEDDING_PROBE_COOLDOWN_SECS` (or similar)
    /// and plumb it through without reaching into internal fields. The default
    /// 60s is a reasonable balance between fast recovery and not hammering a
    /// known-bad primary; larger values make sense for embeddings APIs with
    /// longer outage windows (e.g. provider incidents lasting hours).
    pub fn new_with_backup_and_cooldown(
        api_base: &str,
        api_key: &str,
        model: &str,
        backup_model: Option<String>,
        dimensions: u32,
        probe_cooldown: Duration,
    ) -> Self {
        Self::new_with_backup_cooldown_and_promote_threshold(
            api_base,
            api_key,
            model,
            backup_model,
            dimensions,
            probe_cooldown,
            86_400,
        )
    }

    /// Full constructor: like [`new_with_backup_and_cooldown`] but also
    /// configures the 24h permanent-promotion threshold.
    ///
    /// `promote_after_secs` is the continuous-downtime threshold at which the
    /// backup is permanently promoted and the old primary abandoned. Default
    /// 86_400 (24 h). The caller is expected to clamp the input — this
    /// constructor does not enforce bounds beyond `> 0` via `embed()` checks.
    ///
    /// The live state is seeded with `(model, backup_model)` — callers that
    /// want to distinguish the live effective model from the env-configured
    /// original (e.g. startup honoring a persisted DB promotion) should use
    /// [`Self::new_with_env_and_promote_threshold`] instead.
    pub fn new_with_backup_cooldown_and_promote_threshold(
        api_base: &str,
        api_key: &str,
        model: &str,
        backup_model: Option<String>,
        dimensions: u32,
        probe_cooldown: Duration,
        promote_after_secs: u64,
    ) -> Self {
        // Env originals default to the same values — legacy callers have no
        // separate startup-vs-live distinction, so revert is a no-op for them.
        Self::new_with_env_and_promote_threshold(
            api_base,
            api_key,
            model,
            backup_model.clone(),
            model,
            backup_model,
            dimensions,
            probe_cooldown,
            promote_after_secs,
        )
    }

    /// Full constructor with an explicit split between the **live effective**
    /// model names (what the client should use right now, possibly a DB
    /// promotion override) and the **env-original** model names (what the
    /// `EMBEDDING_MODEL` / `EMBEDDING_BACKUP_MODEL` env vars read at startup).
    ///
    /// The env originals are stored immutably on the client and used by
    /// [`Self::revert_promotion`] to roll back a permanent promotion
    /// in-process, without a service restart.
    ///
    /// # Arguments
    ///
    /// - `effective_primary` / `effective_backup` — what `embed()` will send
    ///   to the API on the next call. If a DB-persisted promotion is active,
    ///   these differ from the env originals.
    /// - `env_primary` / `env_backup` — captured from `EMBEDDING_MODEL` /
    ///   `EMBEDDING_BACKUP_MODEL` at process startup. Restored by `revert`.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_env_and_promote_threshold(
        api_base: &str,
        api_key: &str,
        effective_primary: &str,
        effective_backup: Option<String>,
        env_primary: &str,
        env_backup: Option<String>,
        dimensions: u32,
        probe_cooldown: Duration,
        promote_after_secs: u64,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client builder failed (TLS backend unavailable?)"),
            api_base: api_base.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            dimensions: Arc::new(AtomicU32::new(dimensions)),
            state: Arc::new(FailoverStateInner::new(
                effective_primary.to_string(),
                effective_backup,
            )),
            probe_cooldown_ms: Arc::new(AtomicU64::new(probe_cooldown.as_millis() as u64)),
            promote_after_secs: Arc::new(AtomicU64::new(promote_after_secs)),
            env_primary_model: env_primary.to_string(),
            env_backup_model: env_backup,
        }
    }

    /// True when the client has a non-empty api_base and a usable api_key
    /// (not empty / not an `.env.example` placeholder). Local Ollama uses
    /// `api_key=ollama`, which counts as usable.
    pub fn is_configured(&self) -> bool {
        !self.api_base.trim().is_empty()
            && crate::startup_reasoner::llm_api_key_usable(Some(self.api_key.as_str()))
            && !crate::startup_reasoner::EMBEDDING_API_KEY_PLACEHOLDERS
                .contains(&self.api_key.trim())
    }

    /// Human-readable reason when [`Self::is_configured`] is false.
    pub fn not_configured_reason(&self) -> Option<String> {
        if self.is_configured() {
            return None;
        }
        if self.api_base.trim().is_empty() {
            return Some("EMBEDDING_API_BASE / OPENAI_API_BASE is empty".into());
        }
        if self.api_key.trim().is_empty() {
            return Some("EMBEDDING_API_KEY / OPENAI_API_KEY is empty".into());
        }
        if crate::startup_reasoner::EMBEDDING_API_KEY_PLACEHOLDERS.contains(&self.api_key.trim())
            || crate::startup_reasoner::OPENAI_API_KEY_PLACEHOLDERS.contains(&self.api_key.trim())
        {
            return Some("embedding API key is still the .env.example placeholder".into());
        }
        Some("embedding API key is not usable".into())
    }

    /// Return the live primary model name (what the client is currently
    /// configured to prefer). Returns an owned `String` because the
    /// underlying value is interior-mutable (a permanent promotion can swap
    /// it at runtime). Cheap: short strings.
    pub fn primary_model(&self) -> String {
        self.state.primary_model()
    }

    /// Return the live backup model name if configured. `None` after a
    /// permanent promotion has fired.
    pub fn backup_model(&self) -> Option<String> {
        self.state.backup_model()
    }

    /// Return the model that would be used for the next embed call, assuming
    /// the current degraded state does not change.
    pub fn active_model(&self) -> String {
        let primary = self.state.primary_model();
        if self.state.is_degraded() {
            self.state.backup_model().unwrap_or(primary)
        } else {
            primary
        }
    }

    /// True if the primary has been marked degraded and we are currently
    /// serving traffic from the backup model.
    ///
    /// **Write-path callers** that persist vectors to durable, HNSW-indexed
    /// storage SHOULD check this flag and skip the embedding on true — store
    /// the fact with `embedding = None` instead. See the module doc-comment.
    pub fn is_degraded(&self) -> bool {
        self.state.is_degraded()
    }

    /// The configured 24h permanent-promotion threshold (seconds).
    pub fn promote_after_secs(&self) -> u64 {
        self.promote_after_secs.load(Ordering::Relaxed)
    }

    /// The configured recovery-probe cooldown.
    pub fn probe_cooldown(&self) -> Duration {
        Duration::from_millis(self.probe_cooldown_ms.load(Ordering::Relaxed))
    }

    /// The active vector dimensions for this client.
    pub fn dimensions(&self) -> u32 {
        self.dimensions.load(Ordering::Relaxed)
    }

    /// Peek at a queued promotion alert without consuming it. Paired with
    /// [`Self::clear_pending_promotion_alert`] for a safe commit-or-retry
    /// pattern: the caller should peek, perform side effects (DB persist +
    /// Telegram send), and only clear on success. If any side effect fails,
    /// the alert stays queued and the next health-probe tick will re-attempt.
    pub fn peek_pending_promotion_alert(&self) -> Option<PendingPromotion> {
        self.state.peek_pending_promotion()
    }

    /// Clear the queued promotion alert. Call this ONLY after all side effects
    /// triggered by the alert have succeeded. If called prematurely (before
    /// the DB persist succeeds), the permanent promotion silently reverts on
    /// the next process restart — a silent-failure class bug flagged by CTO
    /// round-1 review of 018fa2e.
    pub fn clear_pending_promotion_alert(&self) {
        self.state.clear_pending_promotion();
    }

    /// DEPRECATED: drain-unconditionally variant retained for backward compat
    /// with call sites that have no side effects to commit. Prefer the
    /// peek-then-clear pattern — see [`Self::peek_pending_promotion_alert`].
    pub fn take_pending_promotion_alert(&self) -> Option<PendingPromotion> {
        self.state.take_pending_promotion()
    }

    /// The env-original primary model captured at startup. Read-only snapshot
    /// of `EMBEDDING_MODEL`. Exposed for tests and observability (e.g. log
    /// the "config vs live" delta).
    pub fn env_primary_model(&self) -> &str {
        &self.env_primary_model
    }

    /// The env-original backup model captured at startup. Read-only snapshot
    /// of `EMBEDDING_BACKUP_MODEL`.
    pub fn env_backup_model(&self) -> Option<&str> {
        self.env_backup_model.as_deref()
    }

    /// **Hot-reload revert** for a 24h permanent promotion.
    ///
    /// Restores the live primary/backup models to the env-original values
    /// captured at startup and clears all degradation/promotion state. Takes
    /// effect immediately in-process — **no service restart required**.
    ///
    /// Called by the `unset_embedding_primary_promotion` tool handler after
    /// it clears the three DB-persisted promotion settings. The ordering
    /// (DB-first, then in-memory) is intentional so a crash between the two
    /// steps leaves a safe fallback: on restart, the startup path reads the
    /// (now-empty) DB settings and rebuilds the client from the env.
    ///
    /// Does NOT issue a recovery probe against the restored primary — if the
    /// primary is still down, the next `embed()` call will naturally re-enter
    /// the failover state machine. Leaving this to lazy discovery keeps the
    /// revert path free of network I/O and error paths.
    ///
    /// **Surprise case documented in `docs/DEPLOYMENT.md §3.3.2`**: the
    /// restored values are whatever `EMBEDDING_MODEL` / `EMBEDDING_BACKUP_MODEL`
    /// were at process start. If the operator has since edited `.env`, those
    /// edits require a full process restart — revert will NOT pick them up.
    ///
    /// **⚠️ Revert target is the BOOT-TIME env snapshot — NOT the currently-
    /// configured settings primary.** If the user has approved a settings-based
    /// primary change via `update_setting(embedding_primary_model=...)` or the
    /// `switch_model`-equivalent HITL path AFTER boot, calling this method
    /// will UNDO that change too: the live primary reverts all the way back
    /// to whatever `EMBEDDING_MODEL` was at process start, even if the current
    /// setting row says something else. To set the live primary to a specific
    /// model without blowing away settings-based changes, use
    /// `update_setting` / `switch_model` — not this method.
    pub fn revert_promotion(&self) -> RevertOutcome {
        let outcome = self
            .state
            .revert_to_env(&self.env_primary_model, self.env_backup_model.clone());
        if outcome.was_promoted {
            tracing::warn!(
                reverted_from = %outcome.reverted_from,
                reverted_to = %outcome.reverted_to,
                backup = ?self.env_backup_model,
                "[Embedding] Hot-reload revert applied — live primary restored to env-original config"
            );
        } else {
            tracing::info!(
                primary = %outcome.reverted_to,
                "[Embedding] revert_promotion: no active promotion, state flags cleared"
            );
        }
        outcome
    }

    /// Hot-reload a user-approved settings change for one of the six
    /// embedding-related keys. Called by the HITL approval handler
    /// (`Orchestrator::handle_config_approval`) AFTER the setting row has
    /// been persisted via [`RuntimeSettings::set`] + `memory.store_setting`.
    ///
    /// Known keys (key prefix `embedding_`):
    /// - `embedding_primary_model` → mutex swap; advise corpus reembed
    ///   (stored vectors are cross-model against the new primary).
    /// - `embedding_backup_model` → mutex swap of the backup slot only;
    ///   empty string clears the slot (disables failover).
    /// - `embedding_dimensions` → `AtomicU32` store + require corpus
    ///   reembed (stored vectors of a different dim are incomparable).
    /// - `embedding_probe_cooldown_secs` → `AtomicU64` store (ms).
    /// - `embedding_promote_after_secs` → `AtomicU64` store.
    ///
    /// Unknown keys return [`UpdateOutcome::Rejected`] — no state change.
    ///
    /// **Validation**: numeric keys are range-clamped (same bounds as the
    /// env-startup path) before being stored. Out-of-range values produce
    /// [`UpdateOutcome::Rejected`]. Model-name keys must be non-empty
    /// (except `embedding_backup_model`, where empty = disable).
    pub fn apply_settings_update(&self, key: &str, value: &serde_json::Value) -> UpdateOutcome {
        match PreparedEmbeddingUpdate::prepare(key, value) {
            Ok(update) => self.apply_prepared_settings_update(update),
            Err(reason) => UpdateOutcome::Rejected { reason },
        }
    }

    /// Short in-memory operations only: never hold a client lock over DB/network I/O.
    pub(crate) fn apply_prepared_settings_update(
        &self,
        update: PreparedEmbeddingUpdate,
    ) -> UpdateOutcome {
        match update {
            PreparedEmbeddingUpdate::Primary(new_primary) => {
                let old = self.state.set_live_primary(&new_primary);
                if old == new_primary {
                    return UpdateOutcome::SilentOk;
                }
                tracing::warn!(
                    from = %old,
                    to = %new_primary,
                    "[Embedding] settings hot-reload: primary model swapped"
                );
                // Primary change → existing corpus is cross-model vs new
                // primary. The consent alert is surfaced via the handler.
                UpdateOutcome::RequiresCorpusReembed {
                    old,
                    new: new_primary,
                }
            }
            PreparedEmbeddingUpdate::Backup(new_backup) => {
                let old = self.state.set_live_backup(new_backup.clone());
                if old == new_backup {
                    return UpdateOutcome::SilentOk;
                }
                tracing::info!(
                    from = ?old,
                    to = ?new_backup,
                    "[Embedding] settings hot-reload: backup model swapped"
                );
                UpdateOutcome::HotReloaded {
                    from: old.unwrap_or_default(),
                    to: new_backup.unwrap_or_default(),
                }
            }
            PreparedEmbeddingUpdate::Dimensions(n) => {
                let old = self.dimensions.swap(n, Ordering::SeqCst);
                if old == n {
                    return UpdateOutcome::SilentOk;
                }
                tracing::warn!(
                    from = old,
                    to = n,
                    "[Embedding] settings hot-reload: dimensions changed — stored vectors are incomparable"
                );
                // Any existing vector has the old dim — cannot cosine-compare
                // against new-dim queries. Full reembed is mandatory.
                UpdateOutcome::RequiresCorpusReembed {
                    old: old.to_string(),
                    new: n.to_string(),
                }
            }
            PreparedEmbeddingUpdate::Cooldown(secs) => {
                let new_ms = secs.saturating_mul(1_000);
                let old_ms = self.probe_cooldown_ms.swap(new_ms, Ordering::Relaxed);
                if old_ms == new_ms {
                    return UpdateOutcome::SilentOk;
                }
                tracing::info!(
                    from_secs = old_ms / 1_000,
                    to_secs = secs,
                    "[Embedding] settings hot-reload: probe cooldown updated"
                );
                UpdateOutcome::HotReloaded {
                    from: format!("{}s", old_ms / 1_000),
                    to: format!("{}s", secs),
                }
            }
            PreparedEmbeddingUpdate::PromoteAfter(secs) => {
                let old = self.promote_after_secs.swap(secs, Ordering::Relaxed);
                if old == secs {
                    return UpdateOutcome::SilentOk;
                }
                tracing::info!(
                    from = old,
                    to = secs,
                    "[Embedding] settings hot-reload: promote_after threshold updated"
                );
                UpdateOutcome::HotReloaded {
                    from: format!("{}s", old),
                    to: format!("{}s", secs),
                }
            }
        }
    }

    /// Test-only: force the degraded flag to true. Used by regression tests
    /// (e.g. learning-level `started_degraded` snapshot invariance) to
    /// simulate an outage without issuing a real failing HTTP call.
    #[cfg(test)]
    pub(crate) fn test_mark_degraded(&self) {
        self.state.mark_degraded();
    }

    /// Test-only: clear the degraded flag. Used alongside `test_mark_degraded`
    /// to simulate primary recovery at a deterministic point in the test.
    #[cfg(test)]
    pub(crate) fn test_mark_recovered(&self) {
        self.state.mark_recovered();
    }

    /// Test-only: backdate the `degraded_since` Instant AND wall-clock by
    /// `older` so the promotion-threshold check observes enough elapsed time
    /// without real sleep. Only fires if we're already degraded (caller
    /// should `test_mark_degraded` first).
    #[cfg(test)]
    pub(crate) fn test_backdate_degraded_since(&self, older: Duration) {
        let mut g = self.state.mu.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(inst) = g.degraded_since_instant {
            g.degraded_since_instant = Some(inst.checked_sub(older).unwrap_or(inst));
        }
        if let Some(wall) = g.degraded_since_wall {
            g.degraded_since_wall = Some(wall.checked_sub(older).unwrap_or(wall));
        }
    }

    /// Embed a text string into a dense vector.
    ///
    /// Tries the primary model first. If the primary is in degraded mode and
    /// the probe cooldown has elapsed, the primary is tested first as a
    /// recovery probe; on success the state flips back to primary.
    ///
    /// On failover-eligible primary failure (transport / non-2xx / parse
    /// error), and if a backup model is configured, the call is retried
    /// against the backup. Empty-result responses are NOT retried — they are
    /// treated as legitimate data issues.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // 24h permanent-promotion check. Runs BEFORE any HTTP work so an
        // already-expired degraded window flips before we waste a primary
        // round-trip. Promotion clears `degraded_since` and nulls the backup
        // slot, so the rest of this call will take the healthy path against
        // the newly-promoted primary with no backup configured.
        self.state
            .check_and_fire_promotion(self.promote_after_secs.load(Ordering::Relaxed));

        // Snapshot the live model names AFTER the possible promotion. Once
        // we've sampled here, a concurrent promotion from another clone will
        // not retarget this in-flight request — it will see the swap on the
        // next call.
        let primary = self.state.primary_model();
        let backup = self.state.backup_model();
        let was_degraded = self.state.is_degraded();

        // ── Recovery probe ────────────────────────────────────────────────
        // When we've been in degraded mode long enough, use this call as an
        // opportunity to probe the primary. If it works, flip back. If it
        // fails, skip straight to the backup path (do NOT retry primary —
        // that would double the cost of every call while degraded).
        let probe_cooldown = self.probe_cooldown();
        let should_probe = was_degraded
            && self
                .state
                .last_probe_at()
                .is_none_or(|t| Instant::now().saturating_duration_since(t) >= probe_cooldown);

        if should_probe {
            self.state.record_probe();
            match self.embed_with_model(text, &primary).await {
                Ok(v) => {
                    self.state.mark_recovered();
                    tracing::info!(
                        model = %primary,
                        "[Embedding] backup → primary (recovered)"
                    );
                    return Ok(v);
                }
                Err(e) if e.is_failover_eligible() => {
                    tracing::debug!(
                        model = %primary,
                        reason = %match &e {
                            EmbedError::Transport(err) => format!("transport: {err:#}"),
                            EmbedError::ApiStatus { status, body } => {
                                let trunc = &body[..body.len().min(200)];
                                format!("status {status}: {trunc}")
                            }
                            EmbedError::ParseError(err) => format!("parse: {err:#}"),
                            EmbedError::EmptyData => "empty data".into(),
                            EmbedError::NotConfigured { reason } => {
                                format!("not configured: {reason}")
                            }
                        },
                        "[Embedding] primary recovery probe failed; staying on backup"
                    );
                    // Fall through to the backup path below.
                }
                Err(e) => return Err(e.into()),
            }
        }

        // ── Normal path: try primary ──────────────────────────────────────
        // Skip this path if we are degraded AND did not just probe (we know
        // primary is failing, no point re-trying; go straight to backup),
        // or if the probe we just did already failed (no point trying again).
        if !was_degraded {
            match self.embed_with_model(text, &primary).await {
                Ok(v) => return Ok(v),
                Err(e) if e.is_failover_eligible() && backup.is_some() => {
                    let reason = match &e {
                        EmbedError::Transport(err) => format!("transport: {err:#}"),
                        EmbedError::ApiStatus { status, body } => {
                            let trunc = &body[..body.len().min(200)];
                            format!("status {status}: {trunc}")
                        }
                        EmbedError::ParseError(err) => format!("parse: {err:#}"),
                        EmbedError::EmptyData | EmbedError::NotConfigured { .. } => unreachable!(),
                    };
                    tracing::warn!(
                        primary = %primary,
                        backup = %backup.as_deref().unwrap_or("?"),
                        reason = %reason,
                        "[Embedding] primary → backup (reason: {reason})"
                    );
                    self.state.mark_degraded();
                    // fall through to backup
                }
                Err(e) => return Err(e.into()),
            }
        }

        // ── Backup path ───────────────────────────────────────────────────
        let backup = backup.context("primary embedding failed and no backup configured")?;
        self.embed_with_model(text, &backup)
            .await
            .map_err(Into::into)
    }

    /// Low-level: embed `text` against a specific `model` name, sharing the
    /// client's api_base / api_key / dimensions. Returns a classified error
    /// so callers can route failover decisions.
    async fn embed_with_model(&self, text: &str, model: &str) -> Result<Vec<f32>, EmbedError> {
        if let Some(reason) = self.not_configured_reason() {
            return Err(EmbedError::NotConfigured { reason });
        }

        let url = format!("{}/embeddings", self.api_base);
        let body = EmbeddingRequest {
            model,
            input: text,
            dimensions: self.dimensions.load(Ordering::Relaxed),
        };

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbedError::Transport(anyhow::Error::new(e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EmbedError::ApiStatus {
                status: status.as_u16(),
                body,
            });
        }

        let body_text = resp.text().await.map_err(|e| {
            EmbedError::Transport(
                anyhow::Error::new(e).context("failed to read embedding API response body"),
            )
        })?;

        let parsed: EmbeddingResponse = serde_json::from_str(&body_text).map_err(|e| {
            EmbedError::ParseError(anyhow::Error::new(e).context(format!(
                "failed to parse embedding API response: {}",
                &body_text[..body_text.len().min(500)]
            )))
        })?;

        parsed
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or(EmbedError::EmptyData)
    }

    /// Detect whether text is predominantly non-English (CJK, Cyrillic, Arabic, etc.).
    pub fn is_non_english(text: &str) -> bool {
        let total = text
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
            .count();
        if total == 0 {
            return false;
        }
        let non_latin = text
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
            .filter(|c| !c.is_ascii_alphanumeric())
            .count();
        non_latin as f64 / total as f64 > 0.3
    }

    /// Translate a short query to English using a cheap chat model via the same API.
    ///
    /// Returns `None` if translation fails (caller should proceed with original only).
    pub async fn translate_to_english(&self, text: &str) -> Option<String> {
        let url = format!("{}/chat/completions", self.api_base);

        let body = serde_json::json!({
            "model": "google/gemini-2.0-flash-lite-001",
            "max_tokens": 100,
            "temperature": 0,
            "messages": [{
                "role": "user",
                "content": format!(
                    "Translate the following text to English. Output ONLY the translation, nothing else.\n\n{text}"
                )
            }]
        });

        let resp = match self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "[EmbeddingClient] Translation request failed");
                return None;
            }
        };

        if !resp.status().is_success() {
            tracing::debug!(status = %resp.status(), "[EmbeddingClient] Translation API error");
            return None;
        }

        let json: serde_json::Value = resp.json().await.ok()?;
        let translated = json["choices"][0]["message"]["content"]
            .as_str()?
            .trim()
            .to_string();

        if translated.is_empty() || translated == text {
            return None;
        }

        tracing::info!("[EmbeddingClient] Translated query: {text:?} → {translated:?}");
        Some(translated)
    }

    /// Cosine similarity between two vectors.
    ///
    /// Returns a value in [-1, 1]. Higher = more similar.
    /// Returns 0.0 if either vector has zero norm or dimension mismatch.
    /// Dimension mismatch can happen during embedding model migration
    /// (e.g. old 512d facts vs new 768d queries) — returns 0.0 to exclude
    /// stale embeddings from results rather than panicking.
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }

        let mut dot: f32 = 0.0;
        let mut norm_a: f32 = 0.0;
        let mut norm_b: f32 = 0.0;

        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }

        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom == 0.0 {
            return 0.0;
        }
        dot / denom
    }

    /// Serialize an embedding vector to a SQLite-storable BLOB.
    pub fn to_blob(emb: &[f32]) -> Vec<u8> {
        emb.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Deserialize a BLOB back to an embedding vector.
    pub fn from_blob(blob: &[u8]) -> Vec<f32> {
        blob.as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = EmbeddingClient::cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = EmbeddingClient::cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = EmbeddingClient::cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = EmbeddingClient::cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn blob_roundtrip() {
        let original = vec![1.0_f32, -0.5, 7.77777, 0.0, -999.999];
        let blob = EmbeddingClient::to_blob(&original);
        let recovered = EmbeddingClient::from_blob(&blob);
        assert_eq!(original, recovered);
    }

    #[test]
    fn is_non_english_chinese() {
        assert!(EmbeddingClient::is_non_english("公司地址"));
        assert!(EmbeddingClient::is_non_english("我的公司在哪里"));
    }

    #[test]
    fn is_non_english_english() {
        assert!(!EmbeddingClient::is_non_english("company address"));
        assert!(!EmbeddingClient::is_non_english("where is my office"));
    }

    #[test]
    fn is_non_english_mixed() {
        // Mostly Chinese with some English
        assert!(EmbeddingClient::is_non_english("我在Microsoft工作"));
        // Mostly English with some Chinese
        assert!(!EmbeddingClient::is_non_english("Microsoft office in 北京"));
    }

    #[test]
    fn is_non_english_empty() {
        assert!(!EmbeddingClient::is_non_english(""));
        assert!(!EmbeddingClient::is_non_english("   "));
    }

    #[test]
    fn blob_roundtrip_512d() {
        let original: Vec<f32> = (0..512).map(|i| i as f32 * 0.001 - 0.256).collect();
        let blob = EmbeddingClient::to_blob(&original);
        assert_eq!(blob.len(), 512 * 4);
        let recovered = EmbeddingClient::from_blob(&blob);
        assert_eq!(original, recovered);
    }

    // ── Failover state machine tests ─────────────────────────────────────
    //
    // The tests below exercise the state machine directly via a small harness
    // that emulates the HTTP-dispatch layer of `embed()`. We don't hit the
    // network — we plug in a per-model response queue and drive
    // [`FailoverStateInner`] + [`EmbeddingClient`] through the exact same
    // transitions `embed()` triggers. This is enough to verify:
    //   - ok primary: no degradation
    //   - primary failure, backup ok: degraded mode entered, returns backup
    //     vector, WARN-log transition observed
    //   - primary failure, no backup: error bubbles up
    //   - recovery probe after cooldown: degraded → primary restored
    //   - EmptyData is NOT failover-eligible
    //
    // The harness intentionally mirrors the control flow in `embed()` — keep
    // them in sync when editing either.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Mock response for one embed_with_model call.
    #[derive(Debug, Clone)]
    enum MockResp {
        Ok(Vec<f32>),
        Transport,
        ApiStatus(u16),
        Parse,
        Empty,
    }

    impl MockResp {
        fn into_result(self) -> Result<Vec<f32>, EmbedError> {
            match self {
                MockResp::Ok(v) => Ok(v),
                MockResp::Transport => {
                    Err(EmbedError::Transport(anyhow::anyhow!("mock transport")))
                }
                MockResp::ApiStatus(s) => Err(EmbedError::ApiStatus {
                    status: s,
                    body: "mock body".into(),
                }),
                MockResp::Parse => Err(EmbedError::ParseError(anyhow::anyhow!("mock parse"))),
                MockResp::Empty => Err(EmbedError::EmptyData),
            }
        }
    }

    struct Harness {
        client: EmbeddingClient,
        primary_queue: Arc<Mutex<VecDeque<MockResp>>>,
        backup_queue: Arc<Mutex<VecDeque<MockResp>>>,
    }

    impl Harness {
        fn new(with_backup: bool) -> Self {
            Self::new_with_promote(with_backup, 86_400)
        }

        fn new_with_promote(with_backup: bool, promote_after_secs: u64) -> Self {
            let primary_queue = Arc::new(Mutex::new(VecDeque::new()));
            let backup_queue = Arc::new(Mutex::new(VecDeque::new()));
            let backup_model = if with_backup {
                Some("mock-backup".to_string())
            } else {
                None
            };
            let client = EmbeddingClient::new_with_backup_cooldown_and_promote_threshold(
                "https://mock.invalid",
                "mock-key",
                "mock-primary",
                backup_model,
                8,
                Duration::from_millis(1),
                promote_after_secs,
            );
            // Short cooldown to keep tests fast.
            client
                .probe_cooldown_ms
                .store(1, std::sync::atomic::Ordering::Relaxed);
            Self {
                client,
                primary_queue,
                backup_queue,
            }
        }

        /// Queue primary responses in order.
        fn push_primary(&self, r: MockResp) {
            self.primary_queue.lock().unwrap().push_back(r);
        }

        /// Queue backup responses in order.
        fn push_backup(&self, r: MockResp) {
            self.backup_queue.lock().unwrap().push_back(r);
        }

        /// Emulate `EmbeddingClient::embed` without making HTTP calls. The
        /// control flow here mirrors `embed()` 1:1 — see the doc-comment
        /// above this module. Includes the 24h permanent-promotion check at
        /// the start (same ordering as the real `embed()`).
        fn embed_mock(&self, _text: &str) -> Result<Vec<f32>> {
            let client = &self.client;

            // Mirror real embed(): promotion check runs FIRST.
            client
                .state
                .check_and_fire_promotion(client.promote_after_secs.load(Ordering::Relaxed));

            let was_degraded = client.state.is_degraded();

            let should_probe = was_degraded
                && client.state.last_probe_at().is_none_or(|t| {
                    Instant::now().saturating_duration_since(t) >= client.probe_cooldown()
                });

            if should_probe {
                client.state.record_probe();
                let resp = self
                    .primary_queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("primary queue empty");
                match resp.into_result() {
                    Ok(v) => {
                        client.state.mark_recovered();
                        return Ok(v);
                    }
                    Err(e) if e.is_failover_eligible() => {
                        // fall through to backup below
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            if !was_degraded {
                let resp = self
                    .primary_queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("primary queue empty");
                match resp.into_result() {
                    Ok(v) => return Ok(v),
                    Err(e) if e.is_failover_eligible() && client.backup_model().is_some() => {
                        client.state.mark_degraded();
                        // fall through to backup
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            // Backup path
            let _ = client
                .backup_model()
                .context("primary embedding failed and no backup configured")?;
            let resp = self
                .backup_queue
                .lock()
                .unwrap()
                .pop_front()
                .expect("backup queue empty");
            resp.into_result().map_err(Into::into)
        }
    }

    fn vec8(n: u8) -> Vec<f32> {
        (0..8).map(|i| (i + n) as f32).collect()
    }

    #[test]
    fn not_configured_skips_http_and_names_install_steps() {
        let empty = EmbeddingClient::new("", "", "nomic-embed-text", 768);
        assert!(!empty.is_configured());
        assert!(
            empty
                .not_configured_reason()
                .unwrap()
                .contains("EMBEDDING_API_BASE")
        );

        let placeholder = EmbeddingClient::new(
            "https://openrouter.ai/api/v1",
            "sk-or-v1-xxxxxxxx",
            "openai/text-embedding-3-small",
            512,
        );
        assert!(!placeholder.is_configured());

        let local = EmbeddingClient::new(
            "http://127.0.0.1:11434/v1",
            "ollama",
            "nomic-embed-text",
            768,
        );
        assert!(local.is_configured());

        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { empty.embed("ping").await })
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("embedding not configured") && msg.contains("ensure-local-embedding"),
            "expected loud install steps, got: {msg}"
        );
    }

    #[test]
    fn failover_primary_healthy_never_touches_backup() {
        let h = Harness::new(true);
        h.push_primary(MockResp::Ok(vec8(1)));
        let v = h.embed_mock("hello").unwrap();
        assert_eq!(v, vec8(1));
        assert!(!h.client.is_degraded());
        assert_eq!(h.client.active_model(), "mock-primary");
    }

    #[test]
    fn failover_primary_fails_backup_takes_over() {
        let h = Harness::new(true);
        h.push_primary(MockResp::ApiStatus(404));
        h.push_backup(MockResp::Ok(vec8(9)));
        let v = h.embed_mock("hello").unwrap();
        assert_eq!(v, vec8(9));
        assert!(h.client.is_degraded(), "should be in degraded mode");
        assert_eq!(h.client.active_model(), "mock-backup");
    }

    #[test]
    fn failover_transport_error_routes_to_backup() {
        let h = Harness::new(true);
        h.push_primary(MockResp::Transport);
        h.push_backup(MockResp::Ok(vec8(2)));
        let v = h.embed_mock("hello").unwrap();
        assert_eq!(v, vec8(2));
        assert!(h.client.is_degraded());
    }

    #[test]
    fn failover_parse_error_routes_to_backup() {
        let h = Harness::new(true);
        h.push_primary(MockResp::Parse);
        h.push_backup(MockResp::Ok(vec8(3)));
        let v = h.embed_mock("hello").unwrap();
        assert_eq!(v, vec8(3));
        assert!(h.client.is_degraded());
    }

    #[test]
    fn empty_data_is_not_failover_eligible() {
        // Per design: empty results are legitimate data issues — don't retry.
        let h = Harness::new(true);
        h.push_primary(MockResp::Empty);
        let err = h.embed_mock("hello").unwrap_err();
        assert!(
            err.to_string().contains("empty data array"),
            "unexpected error: {err}"
        );
        assert!(
            !h.client.is_degraded(),
            "empty data must NOT trigger degraded mode"
        );
    }

    #[test]
    fn no_backup_configured_primary_error_bubbles_up() {
        let h = Harness::new(false);
        h.push_primary(MockResp::ApiStatus(503));
        let err = h.embed_mock("hello").unwrap_err();
        assert!(err.to_string().contains("503"), "unexpected error: {err}");
        assert!(
            !h.client.is_degraded(),
            "no backup = no degraded state transition"
        );
    }

    #[test]
    fn second_call_while_degraded_goes_straight_to_backup() {
        let h = Harness::new(true);
        // First call: primary fails, backup succeeds → degraded.
        h.push_primary(MockResp::ApiStatus(404));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert!(h.client.is_degraded());

        // Second call, BEFORE cooldown elapses: should skip primary entirely,
        // go straight to backup. Prove that by queueing only backup.
        // (If we queued primary, the test harness would panic on empty queue
        // if probe fires.)
        //
        // Ensure the last_probe_at is set to "now" so cooldown (1ms) has NOT
        // elapsed yet. mark_degraded does not set last_probe_at, so we need
        // to set it explicitly to prevent an immediate probe.
        h.client.state.record_probe();
        h.push_backup(MockResp::Ok(vec8(2)));
        let v = h.embed_mock("b").unwrap();
        assert_eq!(v, vec8(2));
        assert!(h.client.is_degraded(), "still degraded after second call");
    }

    #[test]
    fn recovery_probe_after_cooldown_restores_primary() {
        let h = Harness::new(true);
        // Trip into degraded.
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert!(h.client.is_degraded());

        // Sleep a tad longer than probe_cooldown (1ms) so the next call
        // triggers a recovery probe against primary.
        std::thread::sleep(Duration::from_millis(5));

        // Probe succeeds → state flips back to primary.
        h.push_primary(MockResp::Ok(vec8(7)));
        let v = h.embed_mock("b").unwrap();
        assert_eq!(v, vec8(7));
        assert!(
            !h.client.is_degraded(),
            "successful probe should clear degraded flag"
        );
        assert_eq!(h.client.active_model(), "mock-primary");
    }

    #[test]
    fn failed_recovery_probe_keeps_backup_serving() {
        let h = Harness::new(true);
        // Trip into degraded.
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert!(h.client.is_degraded());

        std::thread::sleep(Duration::from_millis(5));

        // Probe fails (primary still down) AND backup is used for the retry.
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(2)));
        let v = h.embed_mock("b").unwrap();
        assert_eq!(v, vec8(2));
        assert!(
            h.client.is_degraded(),
            "failed probe must keep degraded flag"
        );
    }

    #[test]
    fn active_model_reflects_state() {
        let c = EmbeddingClient::new_with_backup(
            "https://mock.invalid",
            "k",
            "primary-x",
            Some("backup-x".to_string()),
            8,
        );
        assert_eq!(c.active_model(), "primary-x");
        c.state.mark_degraded();
        assert_eq!(c.active_model(), "backup-x");
        c.state.mark_recovered();
        assert_eq!(c.active_model(), "primary-x");
    }

    #[test]
    fn clone_shares_failover_state() {
        let c1 = EmbeddingClient::new_with_backup(
            "https://mock.invalid",
            "k",
            "primary-x",
            Some("backup-x".to_string()),
            8,
        );
        let c2 = c1.clone();
        assert!(!c1.is_degraded() && !c2.is_degraded());
        c1.state.mark_degraded();
        assert!(c2.is_degraded(), "clone must observe shared state");
        c2.state.mark_recovered();
        assert!(!c1.is_degraded());
    }

    // ── 24h permanent-promotion tests ────────────────────────────────────
    //
    // Uses `test_backdate_degraded_since` to fake time progression. The
    // check runs at the top of every `embed()` (and the harness' embed_mock),
    // so we can backdate then issue another call to trigger.

    #[test]
    fn promotion_fires_after_threshold_once() {
        // promote_after_secs = 100 s. Trip degraded, backdate past threshold,
        // verify the next call fires the promotion and the SAME-window
        // following call does NOT re-fire (promotion_fired flag).
        let h = Harness::new_with_promote(true, 100);

        // Step 1: trip into degraded state.
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert!(h.client.is_degraded());
        assert!(h.client.take_pending_promotion_alert().is_none());

        // Step 2: backdate degraded_since by > 100s so threshold is exceeded.
        h.client
            .test_backdate_degraded_since(Duration::from_secs(200));

        // Step 3: next embed triggers promotion BEFORE the HTTP path. After
        // promotion: degraded is cleared, backup is None, primary == old backup.
        // The call then takes the healthy path against the new primary.
        h.push_primary(MockResp::Ok(vec8(42)));
        let v = h.embed_mock("b").unwrap();
        assert_eq!(v, vec8(42));

        let alert = h
            .client
            .take_pending_promotion_alert()
            .expect("promotion should have queued an alert");
        assert_eq!(alert.from, "mock-primary");
        assert_eq!(alert.to, "mock-backup");
        assert!(!alert.at_iso.is_empty());

        // Post-promotion: primary name is the old backup; backup slot null.
        assert_eq!(h.client.primary_model(), "mock-backup");
        assert!(h.client.backup_model().is_none());
        assert!(!h.client.is_degraded());

        // Step 4: issue another embed while NOT degraded — should NOT
        // re-fire. Alert queue stays empty.
        h.push_primary(MockResp::Ok(vec8(7)));
        h.embed_mock("c").unwrap();
        assert!(
            h.client.take_pending_promotion_alert().is_none(),
            "promotion must not re-fire without a new degraded window"
        );
    }

    #[test]
    fn promotion_swaps_primary_and_clears_backup() {
        // Same scenario, but focus on the post-promotion accessor outputs.
        let h = Harness::new_with_promote(true, 10);
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert_eq!(h.client.primary_model(), "mock-primary");
        assert_eq!(h.client.backup_model().as_deref(), Some("mock-backup"));

        h.client
            .test_backdate_degraded_since(Duration::from_secs(30));

        h.push_primary(MockResp::Ok(vec8(2)));
        h.embed_mock("b").unwrap();

        assert_eq!(h.client.primary_model(), "mock-backup");
        assert_eq!(h.client.backup_model(), None);
        assert_eq!(h.client.active_model(), "mock-backup");
    }

    #[test]
    fn promotion_resets_state_for_new_degradation() {
        // After promotion, a fresh embed failure on the new primary must
        // start a NEW degraded window. Since backup is now None,
        // is_failover_eligible + backup.is_some() gate is false, so the
        // error bubbles up — but the degraded flag should still be FALSE
        // (degraded only sets when we fail over to a backup).
        let h = Harness::new_with_promote(true, 10);
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        h.client
            .test_backdate_degraded_since(Duration::from_secs(30));
        h.push_primary(MockResp::Ok(vec8(2)));
        h.embed_mock("b").unwrap();

        // Sanity: promotion happened, window cleared, alert drained.
        assert!(h.client.take_pending_promotion_alert().is_some());
        assert!(!h.client.is_degraded());

        // Post-promotion: primary (= old backup) fails. With no backup the
        // error bubbles up and degraded stays false (no failover to enter).
        h.push_primary(MockResp::ApiStatus(500));
        let err = h.embed_mock("c").unwrap_err();
        assert!(err.to_string().contains("500"));
        assert!(
            !h.client.is_degraded(),
            "no backup = no degraded state transition (same as initial no-backup case)"
        );
        // And critically: the promotion pending-alert queue is clean; no
        // double-fire.
        assert!(h.client.take_pending_promotion_alert().is_none());
    }

    #[test]
    fn threshold_not_triggered_before_duration() {
        // Degrade, backdate by (threshold - 10s). Promotion should NOT fire.
        let h = Harness::new_with_promote(true, 100);
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        assert!(h.client.is_degraded());

        h.client
            .test_backdate_degraded_since(Duration::from_secs(90));

        // Still-in-window: no probe yet (cooldown hasn't elapsed either,
        // because we just marked degraded), so straight-to-backup.
        h.client.state.record_probe(); // suppress probe to keep queue small
        h.push_backup(MockResp::Ok(vec8(2)));
        let v = h.embed_mock("b").unwrap();
        assert_eq!(v, vec8(2));

        // Critically: still degraded, alert queue empty, primary unchanged.
        assert!(h.client.is_degraded());
        assert!(h.client.take_pending_promotion_alert().is_none());
        assert_eq!(h.client.primary_model(), "mock-primary");
        assert_eq!(h.client.backup_model().as_deref(), Some("mock-backup"));
    }

    // ── Hot-reload revert tests ──────────────────────────────────────────
    //
    // Covers the `revert_promotion()` path invoked by the
    // `unset_embedding_primary_promotion` tool after it clears the three DB
    // settings. The Harness is constructed with
    // `new_with_backup_cooldown_and_promote_threshold`, which aliases
    // env_primary = effective_primary. To simulate "startup saw a DB
    // override", we bypass the Harness constructor and build a client
    // directly via `new_with_env_and_promote_threshold` with distinct
    // env vs effective values.

    #[test]
    fn revert_restores_env_config_after_promotion() {
        // Step 1: fire a promotion via backdate, same as the existing
        // promotion_fires_after_threshold_once test.
        let h = Harness::new_with_promote(true, 100);
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        h.client
            .test_backdate_degraded_since(Duration::from_secs(200));
        h.push_primary(MockResp::Ok(vec8(42)));
        h.embed_mock("b").unwrap();

        // Sanity: promoted — live primary is now the old backup.
        assert_eq!(h.client.primary_model(), "mock-backup");
        assert_eq!(h.client.backup_model(), None);
        let _ = h.client.take_pending_promotion_alert();

        // Step 2: revert. For the Harness client, env_primary = effective
        // at construction time = "mock-primary", env_backup = "mock-backup".
        let outcome = h.client.revert_promotion();
        assert!(outcome.was_promoted, "should detect the promotion");
        assert_eq!(outcome.reverted_from, "mock-backup");
        assert_eq!(outcome.reverted_to, "mock-primary");

        // Live state: restored.
        assert_eq!(h.client.primary_model(), "mock-primary");
        assert_eq!(h.client.backup_model().as_deref(), Some("mock-backup"));
        assert!(!h.client.is_degraded());
        assert!(h.client.take_pending_promotion_alert().is_none());
    }

    #[test]
    fn revert_is_no_op_when_not_promoted() {
        let h = Harness::new(true);
        // Fresh client — no promotion, no degradation.
        let outcome = h.client.revert_promotion();
        assert!(!outcome.was_promoted);
        assert_eq!(outcome.reverted_from, "mock-primary");
        assert_eq!(outcome.reverted_to, "mock-primary");

        // State unchanged.
        assert_eq!(h.client.primary_model(), "mock-primary");
        assert_eq!(h.client.backup_model().as_deref(), Some("mock-backup"));
        assert!(!h.client.is_degraded());
    }

    #[test]
    fn revert_does_not_break_failover() {
        // After revert restores the backup, the failover path must still
        // work: induce primary failure and verify traffic flips to backup.
        let h = Harness::new_with_promote(true, 100);
        // Fire + revert
        h.push_primary(MockResp::ApiStatus(503));
        h.push_backup(MockResp::Ok(vec8(1)));
        h.embed_mock("a").unwrap();
        h.client
            .test_backdate_degraded_since(Duration::from_secs(200));
        h.push_primary(MockResp::Ok(vec8(42)));
        h.embed_mock("b").unwrap();
        let _ = h.client.take_pending_promotion_alert();
        h.client.revert_promotion();
        assert_eq!(h.client.primary_model(), "mock-primary");
        assert_eq!(h.client.backup_model().as_deref(), Some("mock-backup"));

        // Now induce a fresh primary failure — backup must catch it.
        h.push_primary(MockResp::Transport);
        h.push_backup(MockResp::Ok(vec8(77)));
        let v = h.embed_mock("c").unwrap();
        assert_eq!(v, vec8(77), "backup should serve after revert");
        assert!(
            h.client.is_degraded(),
            "failover must enter degraded state post-revert"
        );
        assert_eq!(h.client.active_model(), "mock-backup");
    }

    #[test]
    fn revert_uses_env_not_effective_when_they_differ() {
        // Build a client directly with distinct env vs effective values
        // (simulates main.rs startup honoring a persisted DB promotion).
        let client = EmbeddingClient::new_with_env_and_promote_threshold(
            "https://mock.invalid",
            "k",
            // Effective = promoted-from-DB model (no backup per design §4).
            "promoted-live",
            None,
            // Env original = what EMBEDDING_MODEL was at startup + backup.
            "env-primary",
            Some("env-backup".to_string()),
            8,
            Duration::from_millis(1),
            100,
        );

        // Pre-revert: live primary is the DB override.
        assert_eq!(client.primary_model(), "promoted-live");
        assert_eq!(client.backup_model(), None);
        assert_eq!(client.env_primary_model(), "env-primary");
        assert_eq!(client.env_backup_model(), Some("env-backup"));

        // Revert: live state flips to env originals.
        let outcome = client.revert_promotion();
        assert!(outcome.was_promoted);
        assert_eq!(outcome.reverted_from, "promoted-live");
        assert_eq!(outcome.reverted_to, "env-primary");
        assert_eq!(client.primary_model(), "env-primary");
        assert_eq!(client.backup_model().as_deref(), Some("env-backup"));
    }

    // ── apply_settings_update hot-reload tests (2026-04-18 refactor) ─────

    /// Build a baseline client for settings-update tests.
    fn mk_settings_client() -> EmbeddingClient {
        EmbeddingClient::new_with_backup_cooldown_and_promote_threshold(
            "https://mock.invalid",
            "mock-key",
            "original-primary",
            Some("original-backup".to_string()),
            512,
            Duration::from_secs(60),
            86_400,
        )
    }

    #[test]
    fn apply_settings_update_primary_model_hot_swap() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_primary_model",
            &serde_json::Value::String("new-primary".to_string()),
        );
        // Primary change → corpus reembed required (stored vectors were
        // produced by the old primary; cosine is now meaningless).
        assert!(matches!(
            outcome,
            UpdateOutcome::RequiresCorpusReembed { .. }
        ));
        assert_eq!(c.primary_model(), "new-primary");
        // Backup slot untouched.
        assert_eq!(c.backup_model().as_deref(), Some("original-backup"));
    }

    #[test]
    fn apply_settings_update_primary_unchanged_is_silent_ok() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_primary_model",
            &serde_json::Value::String("original-primary".to_string()),
        );
        assert_eq!(outcome, UpdateOutcome::SilentOk);
        assert_eq!(c.primary_model(), "original-primary");
    }

    #[test]
    fn apply_settings_update_backup_swap() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_backup_model",
            &serde_json::Value::String("new-backup".to_string()),
        );
        assert!(matches!(outcome, UpdateOutcome::HotReloaded { .. }));
        assert_eq!(c.backup_model().as_deref(), Some("new-backup"));
        // Primary untouched.
        assert_eq!(c.primary_model(), "original-primary");
    }

    #[test]
    fn apply_settings_update_backup_empty_disables() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_backup_model",
            &serde_json::Value::String(String::new()),
        );
        assert!(matches!(outcome, UpdateOutcome::HotReloaded { .. }));
        assert_eq!(c.backup_model(), None, "empty string must disable backup");
    }

    #[test]
    fn apply_settings_update_dimensions_requires_reembed() {
        let c = mk_settings_client();
        assert_eq!(c.dimensions(), 512);
        let outcome = c.apply_settings_update(
            "embedding_dimensions",
            &serde_json::Value::Number(serde_json::Number::from(1024)),
        );
        // Any dim change is destructive to the stored corpus.
        assert!(matches!(
            outcome,
            UpdateOutcome::RequiresCorpusReembed { .. }
        ));
        assert_eq!(c.dimensions(), 1024);
    }

    #[test]
    fn apply_settings_update_dimensions_out_of_range_rejected() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_dimensions",
            &serde_json::Value::Number(serde_json::Number::from(0)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        // Live dim unchanged.
        assert_eq!(c.dimensions(), 512);

        let outcome = c.apply_settings_update(
            "embedding_dimensions",
            &serde_json::Value::Number(serde_json::Number::from(20_000)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        assert_eq!(c.dimensions(), 512);
    }

    #[test]
    fn apply_settings_update_probe_cooldown_hot_reload() {
        let c = mk_settings_client();
        assert_eq!(c.probe_cooldown(), Duration::from_secs(60));
        let outcome = c.apply_settings_update(
            "embedding_probe_cooldown_secs",
            &serde_json::Value::Number(serde_json::Number::from(120u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::HotReloaded { .. }));
        assert_eq!(c.probe_cooldown(), Duration::from_secs(120));
    }

    #[test]
    fn apply_settings_update_probe_cooldown_range_check() {
        let c = mk_settings_client();
        // Zero rejected.
        let outcome = c.apply_settings_update(
            "embedding_probe_cooldown_secs",
            &serde_json::Value::Number(serde_json::Number::from(0u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        // Too-large rejected.
        let outcome = c.apply_settings_update(
            "embedding_probe_cooldown_secs",
            &serde_json::Value::Number(serde_json::Number::from(100_000u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        assert_eq!(c.probe_cooldown(), Duration::from_secs(60));
    }

    #[test]
    fn apply_settings_update_promote_after_hot_reload() {
        let c = mk_settings_client();
        assert_eq!(c.promote_after_secs(), 86_400);
        let outcome = c.apply_settings_update(
            "embedding_promote_after_secs",
            &serde_json::Value::Number(serde_json::Number::from(172_800u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::HotReloaded { .. }));
        assert_eq!(c.promote_after_secs(), 172_800);
    }

    #[test]
    fn apply_settings_update_promote_after_range_check() {
        let c = mk_settings_client();
        // Below 3600 rejected.
        let outcome = c.apply_settings_update(
            "embedding_promote_after_secs",
            &serde_json::Value::Number(serde_json::Number::from(60u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        // Above 604800 rejected.
        let outcome = c.apply_settings_update(
            "embedding_promote_after_secs",
            &serde_json::Value::Number(serde_json::Number::from(10_000_000u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        assert_eq!(c.promote_after_secs(), 86_400);
    }

    #[test]
    fn apply_settings_update_unknown_key_rejected() {
        let c = mk_settings_client();
        let outcome = c.apply_settings_update(
            "embedding_unknown_key",
            &serde_json::Value::String("whatever".into()),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
    }

    #[test]
    fn apply_settings_update_clones_share_atomic_state() {
        // Regression: the hot-reload machinery relies on Arc<Atomic*> so
        // every clone observes the swap. If anyone demotes these fields
        // back to plain u64/u32 this test breaks loudly.
        let c1 = mk_settings_client();
        let c2 = c1.clone();

        let outcome = c1.apply_settings_update(
            "embedding_probe_cooldown_secs",
            &serde_json::Value::Number(serde_json::Number::from(600u64)),
        );
        assert!(matches!(outcome, UpdateOutcome::HotReloaded { .. }));
        // Second clone sees the update through the shared Arc<AtomicU64>.
        assert_eq!(c2.probe_cooldown(), Duration::from_secs(600));

        let outcome = c2.apply_settings_update(
            "embedding_dimensions",
            &serde_json::Value::Number(serde_json::Number::from(768u64)),
        );
        assert!(matches!(
            outcome,
            UpdateOutcome::RequiresCorpusReembed { .. }
        ));
        assert_eq!(c1.dimensions(), 768);
    }

    #[test]
    fn apply_settings_update_primary_model_string_validation() {
        let c = mk_settings_client();
        // Empty string rejected.
        let outcome = c.apply_settings_update(
            "embedding_primary_model",
            &serde_json::Value::String("".into()),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        // Non-string rejected.
        let outcome = c.apply_settings_update(
            "embedding_primary_model",
            &serde_json::Value::Number(serde_json::Number::from(42)),
        );
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        // Too long rejected.
        let long = "x".repeat(200);
        let outcome =
            c.apply_settings_update("embedding_primary_model", &serde_json::Value::String(long));
        assert!(matches!(outcome, UpdateOutcome::Rejected { .. }));
        assert_eq!(c.primary_model(), "original-primary");
    }
}
