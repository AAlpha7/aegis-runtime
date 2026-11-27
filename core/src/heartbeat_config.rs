//! Heartbeat agent-loop configuration (design: `docs/heartbeat-autonomy-design.md`).
//!
//! Backed by a single `user_settings` row keyed `heartbeat_config` (JSON), loaded
//! once at startup into an in-memory cache and hot-reloaded whenever the LLM or
//! user writes that key. Every write is mirrored into `config_audit_log` so we
//! can attribute drift.
//!
//! Step 1 scope: data layer + migration only — nothing in the codebase reads
//! this cache yet. Later steps render it into the heartbeat wake-up prompt,
//! enforce per-tick budgets, and gate proposal dedup.

use chrono::Timelike;
use serde::{Deserialize, Serialize};

/// Maximum bytes allowed for `next_tick_note` (design §3, §15.1).
pub const DEFAULT_NEXT_TICK_NOTE_MAX_BYTES: u32 = 10_240;

/// Root heartbeat configuration object.
///
/// Stored as JSON under the `heartbeat_config` settings key. All fields have
/// serde defaults so old DB rows (missing new fields) deserialize cleanly —
/// this mirrors the ALTER TABLE + COALESCE read-default discipline called out
/// in the plan's Wiring Requirements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat ticks, in minutes.
    #[serde(default = "default_frequency_minutes")]
    pub frequency_minutes: u32,

    /// Quiet hours — no heartbeat LLM invocation inside this window (user local tz).
    #[serde(default)]
    pub quiet_hours: QuietHours,

    /// Max bytes the LLM may write into `next_tick_note` (single self-TODO slot).
    #[serde(default = "default_next_tick_note_max_bytes")]
    pub next_tick_note_max_bytes: u32,

    /// Per-tick budget sub-limits (tool calls, proactive messages).
    #[serde(default)]
    pub per_tick: PerTickBudget,

    /// Duplicate-suppression config (embedding cosine thresholds).
    #[serde(default)]
    pub dedup: DedupConfig,

    /// Controlled autonomy expansion, disabled by default.
    ///
    /// This is intentionally a separate nested object so expanding heartbeat
    /// behavior requires opting into explicit budgets and approval semantics
    /// instead of silently broadening the existing cleanup config.
    #[serde(default)]
    pub controlled_thaw: ControlledThawConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuietHours {
    /// Local-time start, `HH:MM` (24h).
    pub start: String,
    /// Local-time end, `HH:MM` (24h).
    pub end: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerTickBudget {
    /// Max tool calls per tick before forced `noop`.
    #[serde(default = "default_per_tick_tool_calls")]
    pub tool_calls: u32,
    /// Max `send_message` invocations per tick.
    #[serde(default = "default_per_tick_send_message")]
    pub send_message: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DedupConfig {
    /// Cosine similarity above which two items are considered duplicates.
    #[serde(default = "default_dedup_cosine_threshold")]
    pub cosine_threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlledThawConfig {
    /// Master switch for post-P0 controlled heartbeat autonomy.
    #[serde(default)]
    pub enabled: bool,
    /// Cap LLM-dispatched heartbeat wakeups to avoid returning to noisy polling.
    #[serde(default = "default_controlled_thaw_llm_wakeups_per_hour")]
    pub max_llm_wakeups_per_hour: u32,
    /// Cap background proposal attempts before daily reset.
    #[serde(default = "default_controlled_thaw_background_proposals_per_day")]
    pub max_background_proposals_per_day: u32,
    /// External side effects from heartbeat must remain HITL-gated.
    #[serde(default = "default_controlled_thaw_require_approval")]
    pub require_approval_for_external_actions: bool,
}

// ── defaults (design §15.1 locked parameters) ───────────────────────────────

fn default_frequency_minutes() -> u32 {
    30
}
fn default_next_tick_note_max_bytes() -> u32 {
    DEFAULT_NEXT_TICK_NOTE_MAX_BYTES
}
fn default_per_tick_tool_calls() -> u32 {
    8
}
fn default_per_tick_send_message() -> u32 {
    1
}
fn default_dedup_cosine_threshold() -> f32 {
    0.85
}
fn default_controlled_thaw_llm_wakeups_per_hour() -> u32 {
    2
}
fn default_controlled_thaw_background_proposals_per_day() -> u32 {
    3
}
fn default_controlled_thaw_require_approval() -> bool {
    true
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            frequency_minutes: default_frequency_minutes(),
            quiet_hours: QuietHours::default(),
            next_tick_note_max_bytes: default_next_tick_note_max_bytes(),
            per_tick: PerTickBudget::default(),
            dedup: DedupConfig::default(),
            controlled_thaw: ControlledThawConfig::default(),
        }
    }
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            start: "02:00".to_string(),
            end: "07:00".to_string(),
        }
    }
}

impl Default for PerTickBudget {
    fn default() -> Self {
        Self {
            tool_calls: default_per_tick_tool_calls(),
            send_message: default_per_tick_send_message(),
        }
    }
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            cosine_threshold: default_dedup_cosine_threshold(),
        }
    }
}

impl Default for ControlledThawConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_llm_wakeups_per_hour: default_controlled_thaw_llm_wakeups_per_hour(),
            max_background_proposals_per_day: default_controlled_thaw_background_proposals_per_day(
            ),
            require_approval_for_external_actions: default_controlled_thaw_require_approval(),
        }
    }
}

/// Apply controlled-thaw tool exposure limits to a heartbeat tool allowlist.
///
/// Default-off config returns the original list unchanged. When enabled, this
/// keeps the first runtime slice conservative:
/// - direct `send_message` is hidden while external actions require approval.
pub fn filter_controlled_thaw_tools(
    thaw: &ControlledThawConfig,
    allowed_tools: &[String],
) -> Vec<String> {
    if !thaw.enabled {
        return allowed_tools.to_vec();
    }

    allowed_tools
        .iter()
        .filter(|tool| {
            let name = tool.as_str();
            !(thaw.require_approval_for_external_actions && name == "send_message")
        })
        .cloned()
        .collect()
}

impl QuietHours {
    /// Check whether the current wall-clock time (in the user's local timezone)
    /// falls within the configured quiet window.
    ///
    /// `start` and `end` are `HH:MM` 24-hour strings. Overnight wrapping is
    /// supported: `start="22:00", end="06:00"` covers 22:00-23:59 and 00:00-05:59.
    ///
    /// # Timezone
    ///
    /// The `tz` parameter should be the user's IANA timezone (e.g. `Asia/Shanghai`).
    /// The heartbeat loop resolves it from the `user_timezone` setting.
    // TODO(future): make timezone configurable via HeartbeatConfig
    pub fn in_quiet_hours(&self, tz: chrono_tz::Tz) -> bool {
        let now_local = chrono::Utc::now().with_timezone(&tz);
        self.contains_local_time(now_local.hour(), now_local.minute())
    }

    /// Pure logic: does the `(HH, MM)` pair fall inside `[start, end)`?
    ///
    /// Extracted so unit tests can inject arbitrary times without mocking the
    /// system clock.
    fn contains_local_time(&self, hour: u32, minute: u32) -> bool {
        let (sh, sm) = match Self::parse_hhmm(&self.start) {
            Some(v) => v,
            None => return false, // unparseable → not in quiet hours
        };
        let (eh, em) = match Self::parse_hhmm(&self.end) {
            Some(v) => v,
            None => return false,
        };

        let start_mins = sh * 60 + sm;
        let end_mins = eh * 60 + em;
        let now_mins = hour * 60 + minute;

        if start_mins <= end_mins {
            // Same-day range, e.g. 02:00-07:00
            now_mins >= start_mins && now_mins < end_mins
        } else {
            // Overnight wrap, e.g. 22:00-06:00
            now_mins >= start_mins || now_mins < end_mins
        }
    }

    /// Parse `"HH:MM"` into `(hour, minute)`. Returns `None` on bad format.
    fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return None;
        }
        let h: u32 = parts[0].parse().ok()?;
        let m: u32 = parts[1].parse().ok()?;
        if h >= 24 || m >= 60 {
            return None;
        }
        Some((h, m))
    }
}

impl HeartbeatConfig {
    /// Parse from a JSON value; unknown/missing fields fall back to defaults.
    /// Returns `None` if the value is not a JSON object we can decode.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    /// Serialize to a JSON object suitable for `store_setting`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Audit-log actor — who wrote a config change.
///
/// Kept as an explicit enum so call sites are forced to pick one of the three
/// legitimate sources; a random string would invite typos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAuditActor {
    Llm,
    User,
    Migration,
}

impl ConfigAuditActor {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigAuditActor::Llm => "llm",
            ConfigAuditActor::User => "user",
            ConfigAuditActor::Migration => "migration",
        }
    }
}

/// Outcome of a single heartbeat tick — stored as TEXT in
/// `heartbeat_audit_log.outcome` (design §13).
///
/// Every metronome tick writes one row; `outcome` tells you which path ran.
/// Old rows (pre-Step-2) have NULL here — readers must treat that as "unknown".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeartbeatTickOutcome {
    /// Zero-cost precheck short-circuited the tick before any LLM call.
    SkippedPrecheck,
    /// LLM ran but produced no tool calls and no user-facing message.
    Noop,
    /// LLM ran but only performed heartbeat-internal bookkeeping.
    MaintenanceOnly,
    /// LLM ran and produced tool calls / messages / memo writes.
    Acted,
    /// Daily or per-tick budget was exhausted — no LLM dispatched.
    BudgetExhausted,
    /// LLM call / tool loop failed; breadcrumb is in `llm_reply`.
    Error,
}

impl HeartbeatTickOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            HeartbeatTickOutcome::SkippedPrecheck => "skipped_precheck",
            HeartbeatTickOutcome::Noop => "noop",
            HeartbeatTickOutcome::MaintenanceOnly => "maintenance_only",
            HeartbeatTickOutcome::Acted => "acted",
            HeartbeatTickOutcome::BudgetExhausted => "budget_exhausted",
            HeartbeatTickOutcome::Error => "error",
        }
    }
}

impl std::str::FromStr for HeartbeatTickOutcome {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skipped_precheck" => Ok(HeartbeatTickOutcome::SkippedPrecheck),
            "noop" => Ok(HeartbeatTickOutcome::Noop),
            "maintenance_only" => Ok(HeartbeatTickOutcome::MaintenanceOnly),
            "acted" => Ok(HeartbeatTickOutcome::Acted),
            "budget_exhausted" => Ok(HeartbeatTickOutcome::BudgetExhausted),
            "error" => Ok(HeartbeatTickOutcome::Error),
            other => Err(format!("unknown HeartbeatTickOutcome: {other}")),
        }
    }
}

/// Classification for a tool/action observed during a heartbeat tick.
///
/// This is intentionally separate from [`HeartbeatTickOutcome`]. Phase 1 only
/// gives later runtime and summary code a tested vocabulary; it does not change
/// how ticks are stored or counted today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatActionClass {
    UserVisible,
    ExternalSideEffect,
    ProposalCreated,
    ApprovalRequested,
    MemoryWriteMeaningful,
    MaintenanceOnly,
    Noop,
}

impl HeartbeatActionClass {
    pub fn is_meaningful(self) -> bool {
        !matches!(
            self,
            HeartbeatActionClass::MaintenanceOnly | HeartbeatActionClass::Noop
        )
    }
}

/// Helper-level outcome used by heartbeat cleanup code before it writes or
/// rewrites any audit-log behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatClassifiedOutcome {
    Noop,
    MaintenanceOnly,
    Acted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTickClassification {
    pub outcome: HeartbeatClassifiedOutcome,
    pub user_visible_action_count: usize,
    pub external_side_effect_count: usize,
    pub maintenance_action_count: usize,
    pub noop_count: usize,
    pub bookkeeping_tool_names: Vec<String>,
    pub meaningful_tool_names: Vec<String>,
}

/// Heartbeat tools that only maintain internal state or inspect local context.
///
/// Unknown tools are treated as meaningful by default so new side-effecting
/// tools are not hidden from user-facing accounting until explicitly classified.
pub fn is_heartbeat_bookkeeping_tool(name: &str) -> bool {
    matches!(
        name,
        "noop"
            | "update_heartbeat_memo"
            | "write_next_tick_note"
            | "system_evaluate_memo_update"
            | "get_current_time"
            | "view_heartbeat_audit"
    )
}

pub fn classify_heartbeat_tool(name: &str) -> HeartbeatActionClass {
    match name {
        "noop" => HeartbeatActionClass::Noop,
        "update_heartbeat_memo"
        | "write_next_tick_note"
        | "system_evaluate_memo_update"
        | "get_current_time"
        | "view_heartbeat_audit" => HeartbeatActionClass::MaintenanceOnly,
        "send_message" | "notify_user" => HeartbeatActionClass::UserVisible,
        "propose_config_change"
        | "propose_task"
        | "create_task"
        | "create_reminder"
        | "propose_reminder" => HeartbeatActionClass::ProposalCreated,
        "request_approval" | "approval_request" => HeartbeatActionClass::ApprovalRequested,
        "write_memory" => HeartbeatActionClass::MemoryWriteMeaningful,
        _ => HeartbeatActionClass::ExternalSideEffect,
    }
}

pub fn is_meaningful_heartbeat_tool(name: &str) -> bool {
    classify_heartbeat_tool(name).is_meaningful()
}

pub fn classify_heartbeat_tick_tools<'a, I>(tool_names: I) -> HeartbeatTickClassification
where
    I: IntoIterator<Item = &'a str>,
{
    let mut user_visible_action_count = 0;
    let mut external_side_effect_count = 0;
    let mut maintenance_action_count = 0;
    let mut noop_count = 0;
    let mut bookkeeping_tool_names = Vec::new();
    let mut meaningful_tool_names = Vec::new();

    for name in tool_names {
        match classify_heartbeat_tool(name) {
            HeartbeatActionClass::UserVisible => {
                user_visible_action_count += 1;
                meaningful_tool_names.push(name.to_string());
            }
            HeartbeatActionClass::ExternalSideEffect => {
                external_side_effect_count += 1;
                meaningful_tool_names.push(name.to_string());
            }
            HeartbeatActionClass::ProposalCreated
            | HeartbeatActionClass::ApprovalRequested
            | HeartbeatActionClass::MemoryWriteMeaningful => {
                meaningful_tool_names.push(name.to_string());
            }
            HeartbeatActionClass::MaintenanceOnly => {
                maintenance_action_count += 1;
                bookkeeping_tool_names.push(name.to_string());
            }
            HeartbeatActionClass::Noop => {
                noop_count += 1;
                bookkeeping_tool_names.push(name.to_string());
            }
        }
    }

    let outcome = if !meaningful_tool_names.is_empty() {
        HeartbeatClassifiedOutcome::Acted
    } else if maintenance_action_count > 0 {
        HeartbeatClassifiedOutcome::MaintenanceOnly
    } else {
        HeartbeatClassifiedOutcome::Noop
    };

    HeartbeatTickClassification {
        outcome,
        user_visible_action_count,
        external_side_effect_count,
        maintenance_action_count,
        noop_count,
        bookkeeping_tool_names,
        meaningful_tool_names,
    }
}

pub fn classify_heartbeat_tick_outcome<'a, I>(
    tool_names: I,
    has_reply_text: bool,
    budget_exhausted: bool,
) -> HeartbeatTickOutcome
where
    I: IntoIterator<Item = &'a str>,
{
    if budget_exhausted {
        return HeartbeatTickOutcome::BudgetExhausted;
    }
    if has_reply_text {
        return HeartbeatTickOutcome::Acted;
    }

    match classify_heartbeat_tick_tools(tool_names).outcome {
        HeartbeatClassifiedOutcome::Acted => HeartbeatTickOutcome::Acted,
        HeartbeatClassifiedOutcome::MaintenanceOnly => HeartbeatTickOutcome::MaintenanceOnly,
        HeartbeatClassifiedOutcome::Noop => HeartbeatTickOutcome::Noop,
    }
}

/// Reason a precheck-skip tick was not dispatched to the LLM (design §9, §13).
///
/// Stored as TEXT in `heartbeat_audit_log.skip_reason`; NULL for non-skip rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeartbeatSkipReason {
    /// No due intents, HITL, tripped breakers, or imminent deadlines.
    NoNewContext,
    /// An internal queue (e.g. approval queue) is at capacity.
    QueueFull,
    /// Inside configured quiet-hours window.
    QuietHours,
    /// Upstream backpressure (e.g. rate limiter tripped, degraded deps).
    Backpressure,
}

impl HeartbeatSkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            HeartbeatSkipReason::NoNewContext => "no_new_context",
            HeartbeatSkipReason::QueueFull => "queue_full",
            HeartbeatSkipReason::QuietHours => "quiet_hours",
            HeartbeatSkipReason::Backpressure => "backpressure",
        }
    }
}

impl std::str::FromStr for HeartbeatSkipReason {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "no_new_context" => Ok(HeartbeatSkipReason::NoNewContext),
            "queue_full" => Ok(HeartbeatSkipReason::QueueFull),
            "quiet_hours" => Ok(HeartbeatSkipReason::QuietHours),
            "backpressure" => Ok(HeartbeatSkipReason::Backpressure),
            other => Err(format!("unknown HeartbeatSkipReason: {other}")),
        }
    }
}

/// Snapshot of live counters surfaced to the LLM in the heartbeat wake-up
/// prompt alongside [`HeartbeatConfig`] (design §7).
///
/// Captured once per tick, at tick start — subsequent config writes during the
/// tick must NOT re-render this snapshot. Idempotent: two renders of the same
/// snapshot produce byte-identical output.
///
/// Fields that depend on infrastructure not yet shipped (Step 9 approval queue)
/// are left as `None` and rendered as `—` in the prompt block. This is
/// deliberate: we never fabricate counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetSnapshot {
    /// Count of `heartbeat_audit_log` rows for today (local date).
    pub heartbeat_today_count: u32,
    /// Tool calls used so far this tick (always `0` at block-render time since
    /// the block is rendered before the first LLM call).
    pub tool_calls_this_tick: u32,
    /// `send_message` invocations used so far this tick.
    pub send_message_this_tick: u32,
    /// Minutes since the last proactive (`Acted` outcome) heartbeat row. `None`
    /// if no such row exists yet.
    pub last_proactive_message_minutes_ago: Option<u64>,
    /// Config-change proposals pending user approval. `None` until Step 9.
    pub config_pending_approvals: Option<u32>,
}

impl BudgetSnapshot {
    /// Build an "empty" snapshot with unknowns left as `None` and counters at
    /// zero. Handy for unit tests and for ticks where we can't query the DB.
    pub fn empty_stub() -> Self {
        Self {
            heartbeat_today_count: 0,
            tool_calls_this_tick: 0,
            send_message_this_tick: 0,
            last_proactive_message_minutes_ago: None,
            config_pending_approvals: None,
        }
    }
}

/// Outcome of a [`Orchestrator::write_heartbeat_memo`] call (design §5, Step 6).
///
/// `Inline`: content fit under the configured `next_tick_note_max_bytes`
/// ceiling and is stored verbatim in the `heartbeat_memo` setting.
///
/// `Spilled`: content exceeded the cap and was split atomically — the full
/// content was written to L2 under topic `heartbeat:plan:<slug>`, while the
/// `heartbeat_memo` setting holds only a pointer JSON
/// (`{overflow: true, slug, synopsis, bytes, at}`). Overflow is a normal
/// operating mode, never a failure — the LLM sees success and uses the slug
/// on next tick to retrieve the full plan on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoWriteOutcome {
    /// Stored inline in settings. `bytes` = length of the full content written.
    Inline { bytes: usize },
    /// Stored as pointer + L2 spill. `slug` identifies the L2 row under
    /// `heartbeat:plan:<slug>`. `synopsis` is the 1-line preview (≤160 chars,
    /// never empty — placeholder `(empty)` if the content had no text).
    /// `bytes` = length of the full content (not the pointer).
    Spilled {
        slug: String,
        synopsis: String,
        bytes: usize,
    },
}

/// Build a deterministic, URL-safe slug for an overflow-spill memo.
///
/// Format: `YYYYMMDD-<8 hex chars of sha256(content)>`. Deterministic so
/// the same content written twice produces the same slug (natural L2-topic
/// dedup) but date-prefixed so slugs sort chronologically and are
/// human-scannable.
///
/// `now_utc` is injected for testability; production callers pass
/// `chrono::Utc::now()`.
pub fn build_memo_spill_slug(content: &str, now_utc: chrono::DateTime<chrono::Utc>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}-{}", now_utc.format("%Y%m%d"), hex)
}

/// Extract a 1-line synopsis from memo content.
///
/// Takes the first non-empty line (trimmed). If a sentence-ending
/// punctuation mark appears before char 160, truncate at it (inclusive).
/// Otherwise truncate at char 160 and append `…`. Returns `"(empty)"` when
/// the content has no non-empty line — placeholder so prompts stay
/// readable (never a bare empty string).
pub fn build_memo_synopsis(content: &str) -> String {
    const MAX_CHARS: usize = 160;
    let first_line = content
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first_line.is_empty() {
        return "(empty)".to_string();
    }
    let chars: Vec<char> = first_line.chars().collect();
    if let Some(period_idx) = chars
        .iter()
        .take(MAX_CHARS)
        .position(|&c| c == '.' || c == '。' || c == '!' || c == '?' || c == '！' || c == '？')
    {
        return chars[..=period_idx].iter().collect::<String>();
    }
    if chars.len() <= MAX_CHARS {
        return first_line.to_string();
    }
    let mut truncated: String = chars[..MAX_CHARS].iter().collect();
    truncated.push('…');
    truncated
}

/// Mid-tick mutable state observed/updated by heartbeat-scoped tools (design
/// §4.3, Step 8a). Threaded through [`crate::orchestrator::ToolContext`] as
/// `Option<Arc<Mutex<HeartbeatTickState>>>` — `None` on non-heartbeat paths.
///
/// After the ReAct loop finalizes, `run_heartbeat_tick` locks the mutex and
/// folds the relevant fields into the `HeartbeatAuditEntry` row being written.
///
/// Fields populated this round (8a):
/// - `next_tick_note_bytes` — set by the `write_next_tick_note` handler.
/// - `memo_write_count` — incremented by `update_heartbeat_memo` so the legacy
///   scratchpad tool cannot churn multiple write attempts in one heartbeat tick.
///
/// Fields reserved for Step 8b (populated but not yet produced by handlers):
/// - `proposals_created` — will be filled by `propose_config_change`.
/// - `send_message_count` — incremented on each successful `send_message` fire.
/// - `send_message_retries` — incremented on each rejection so the retry cap
///   (design §5: 2 retries per tick) can be enforced at handler entry.
///
/// `tools_called` is deliberately NOT duplicated here: `ToolContext.tools_called`
/// already tracks it for the whole tick.
#[derive(Debug, Default, Clone)]
pub struct HeartbeatTickState {
    /// Bytes of the latest `write_next_tick_note` call this tick (None until a
    /// successful write). Reflects the full content size regardless of inline
    /// vs spill storage.
    pub next_tick_note_bytes: Option<i64>,
    /// Count of `update_heartbeat_memo` write attempts this tick. The
    /// legacy memo tool is retained for compatibility, but heartbeat may write
    /// it at most once per tick; use `write_next_tick_note` for the newer
    /// explicit next-tick note path.
    pub memo_write_count: u32,
    /// Approval-queue proposal keys created this tick (Step 9b). Each entry is
    /// the `state_key` of the inserted HITL row (e.g. `config_<cycle>_<n>`),
    /// matching the `proposals_created` column on `heartbeat_audit_log`.
    pub proposals_created: Vec<String>,
    /// Count of successful `send_message` invocations this tick (Step 10). A
    /// `u32` (not a `bool`) so future budgets can raise the 1/tick cap without
    /// a data-model change; today the cap is `heartbeat_config.per_tick.send_message`.
    pub send_message_count: u32,
    /// Count of `send_message` rejections this tick (Step 10, design §5 retry
    /// cap). Incremented on any rejection verdict; at cap the handler short-
    /// circuits with `retry_cap_exceeded` non-retryable.
    pub send_message_retries: u32,
    /// ULID-like identifier for this tick. Set at the top of
    /// `run_heartbeat_tick` so tool handlers can stamp proposals with the
    /// originating cycle id. `None` on non-heartbeat paths. The same value is
    /// written to `heartbeat_audit_log.cycle_id` so proposals and their
    /// audit row are linked.
    pub cycle_id: Option<String>,
}

/// Generate a unique-enough cycle id without a new dependency.
///
/// Format: `YYYYMMDDHHMMSS<ms>-<4 hex random>`. Collisions are extremely
/// unlikely within a single process (hundreds of ticks/day) — this is a
/// deliberate ulid stand-in per Step 9b briefing (section A).
pub fn new_heartbeat_cycle_id() -> String {
    use rand::Rng;
    let now = chrono::Utc::now();
    let mut rng = rand::rng();
    let suffix: u16 = rng.random();
    format!("{}-{:04x}", now.format("%Y%m%d%H%M%S%3f"), suffix)
}

/// A single row in `config_audit_log` (insert + list APIs on `MemoryHandle`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigAuditEntry {
    pub id: i64,
    /// RFC3339 UTC timestamp.
    pub at: String,
    /// One of `llm | user | migration`.
    pub actor: String,
    /// Previous JSON (may be NULL/empty for first-write and for migrations).
    pub old_json: Option<String>,
    /// New JSON.
    pub new_json: String,
    /// Free-form human explanation.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_spec_15_1() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(cfg.frequency_minutes, 30);
        assert_eq!(cfg.quiet_hours.start, "02:00");
        assert_eq!(cfg.quiet_hours.end, "07:00");
        assert_eq!(cfg.next_tick_note_max_bytes, 10_240);
        assert_eq!(cfg.per_tick.tool_calls, 8);
        assert_eq!(cfg.per_tick.send_message, 1);
        assert!((cfg.dedup.cosine_threshold - 0.85).abs() < 1e-6);
        assert!(!cfg.controlled_thaw.enabled);
        assert_eq!(cfg.controlled_thaw.max_llm_wakeups_per_hour, 2);
        assert_eq!(cfg.controlled_thaw.max_background_proposals_per_day, 3);
        assert!(cfg.controlled_thaw.require_approval_for_external_actions);
    }

    #[test]
    fn roundtrip_default() {
        let cfg = HeartbeatConfig::default();
        let json = cfg.to_json();
        let back = HeartbeatConfig::from_json(&json).expect("decode");
        assert_eq!(cfg, back);
    }

    #[test]
    fn roundtrip_custom() {
        let cfg = HeartbeatConfig {
            frequency_minutes: 15,
            quiet_hours: QuietHours {
                start: "23:00".into(),
                end: "06:30".into(),
            },
            next_tick_note_max_bytes: 4096,
            per_tick: PerTickBudget {
                tool_calls: 4,
                send_message: 2,
            },
            dedup: DedupConfig {
                cosine_threshold: 0.92,
            },
            controlled_thaw: ControlledThawConfig {
                enabled: true,
                max_llm_wakeups_per_hour: 1,
                max_background_proposals_per_day: 2,
                require_approval_for_external_actions: true,
            },
        };
        let json = cfg.to_json();
        let back = HeartbeatConfig::from_json(&json).expect("decode");
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_json_fills_defaults() {
        // Old row missing new fields should deserialize to defaults.
        let partial = serde_json::json!({
            "frequency_minutes": 45,
            "quiet_hours": { "start": "01:00", "end": "08:00" }
        });
        let cfg = HeartbeatConfig::from_json(&partial).expect("decode");
        assert_eq!(cfg.frequency_minutes, 45);
        assert_eq!(cfg.quiet_hours.start, "01:00");
        // Defaults survive.
        assert_eq!(cfg.next_tick_note_max_bytes, 10_240);
        assert_eq!(cfg.per_tick.tool_calls, 8);
        assert!((cfg.dedup.cosine_threshold - 0.85).abs() < 1e-6);
        assert_eq!(cfg.controlled_thaw, ControlledThawConfig::default());
    }

    #[test]
    fn controlled_thaw_partial_json_keeps_approval_and_budget_defaults() {
        let partial = serde_json::json!({
            "controlled_thaw": {
                "enabled": true
            }
        });

        let cfg = HeartbeatConfig::from_json(&partial).expect("decode");

        assert!(cfg.controlled_thaw.enabled);
        assert_eq!(cfg.controlled_thaw.max_llm_wakeups_per_hour, 2);
        assert_eq!(cfg.controlled_thaw.max_background_proposals_per_day, 3);
        assert!(cfg.controlled_thaw.require_approval_for_external_actions);
    }

    #[test]
    fn controlled_thaw_filter_is_noop_when_disabled() {
        let thaw = ControlledThawConfig::default();
        let tools = vec![
            "noop".to_string(),
            "send_message".to_string(),
            "propose_config_change".to_string(),
        ];

        assert_eq!(filter_controlled_thaw_tools(&thaw, &tools), tools);
    }

    #[test]
    fn controlled_thaw_filter_hides_direct_send_by_default() {
        let thaw = ControlledThawConfig {
            enabled: true,
            ..ControlledThawConfig::default()
        };
        let tools = vec![
            "noop".to_string(),
            "send_message".to_string(),
            "propose_config_change".to_string(),
        ];

        assert_eq!(
            filter_controlled_thaw_tools(&thaw, &tools),
            vec!["noop".to_string(), "propose_config_change".to_string()]
        );
    }

    #[test]
    fn actor_string_roundtrip() {
        assert_eq!(ConfigAuditActor::Llm.as_str(), "llm");
        assert_eq!(ConfigAuditActor::User.as_str(), "user");
        assert_eq!(ConfigAuditActor::Migration.as_str(), "migration");
    }

    #[test]
    fn tick_outcome_roundtrip_all_variants() {
        use std::str::FromStr;
        for v in [
            HeartbeatTickOutcome::SkippedPrecheck,
            HeartbeatTickOutcome::Noop,
            HeartbeatTickOutcome::MaintenanceOnly,
            HeartbeatTickOutcome::Acted,
            HeartbeatTickOutcome::BudgetExhausted,
            HeartbeatTickOutcome::Error,
        ] {
            let s = v.as_str();
            let back = HeartbeatTickOutcome::from_str(s).expect("parse");
            assert_eq!(v, back, "roundtrip failed for {s}");
        }
        assert!(HeartbeatTickOutcome::from_str("garbage").is_err());
    }

    #[test]
    fn heartbeat_classification_marks_bookkeeping_tools_explicitly() {
        for tool in [
            "noop",
            "update_heartbeat_memo",
            "write_next_tick_note",
            "system_evaluate_memo_update",
            "get_current_time",
            "view_heartbeat_audit",
        ] {
            assert!(
                is_heartbeat_bookkeeping_tool(tool),
                "{tool} should be bookkeeping-only"
            );
            assert!(
                !is_meaningful_heartbeat_tool(tool),
                "{tool} should not be meaningful work"
            );
        }
    }

    #[test]
    fn heartbeat_classification_treats_unknown_tools_as_meaningful() {
        assert_eq!(
            classify_heartbeat_tool("new_external_tool"),
            HeartbeatActionClass::ExternalSideEffect
        );
        assert!(is_meaningful_heartbeat_tool("new_external_tool"));
        assert!(!is_heartbeat_bookkeeping_tool("new_external_tool"));
    }

    #[test]
    fn heartbeat_classification_treats_notifications_as_user_visible() {
        for tool in ["send_message", "notify_user"] {
            assert_eq!(
                classify_heartbeat_tool(tool),
                HeartbeatActionClass::UserVisible
            );
            assert!(is_meaningful_heartbeat_tool(tool));
            assert!(!is_heartbeat_bookkeeping_tool(tool));
        }
    }

    #[test]
    fn heartbeat_classification_empty_and_noop_ticks_are_noop() {
        let empty = classify_heartbeat_tick_tools(std::iter::empty());
        assert_eq!(empty.outcome, HeartbeatClassifiedOutcome::Noop);
        assert_eq!(empty.noop_count, 0);
        assert!(empty.meaningful_tool_names.is_empty());

        let noop = classify_heartbeat_tick_tools(["noop"]);
        assert_eq!(noop.outcome, HeartbeatClassifiedOutcome::Noop);
        assert_eq!(noop.noop_count, 1);
        assert_eq!(noop.bookkeeping_tool_names, vec!["noop"]);
        assert!(noop.meaningful_tool_names.is_empty());
    }

    #[test]
    fn heartbeat_classification_memo_only_tick_is_maintenance_not_acted() {
        let classification =
            classify_heartbeat_tick_tools(["update_heartbeat_memo", "write_next_tick_note"]);

        assert_eq!(
            classification.outcome,
            HeartbeatClassifiedOutcome::MaintenanceOnly
        );
        assert_eq!(classification.maintenance_action_count, 2);
        assert_eq!(classification.noop_count, 0);
        assert_eq!(
            classification.bookkeeping_tool_names,
            vec!["update_heartbeat_memo", "write_next_tick_note"]
        );
        assert!(classification.meaningful_tool_names.is_empty());
    }

    #[test]
    fn heartbeat_classification_mixed_tick_keeps_meaningful_tools_separate() {
        let classification = classify_heartbeat_tick_tools([
            "get_current_time",
            "send_message",
            "propose_config_change",
            "write_memory",
            "new_external_tool",
        ]);

        assert_eq!(classification.outcome, HeartbeatClassifiedOutcome::Acted);
        assert_eq!(classification.user_visible_action_count, 1);
        assert_eq!(classification.external_side_effect_count, 1);
        assert_eq!(classification.maintenance_action_count, 1);
        assert_eq!(
            classification.bookkeeping_tool_names,
            vec!["get_current_time"]
        );
        assert_eq!(
            classification.meaningful_tool_names,
            vec![
                "send_message",
                "propose_config_change",
                "write_memory",
                "new_external_tool"
            ]
        );
    }

    #[test]
    fn heartbeat_runtime_outcome_reclassifies_bookkeeping_only_ticks() {
        let memo_only = classify_heartbeat_tick_outcome(
            ["update_heartbeat_memo", "write_next_tick_note"],
            false,
            false,
        );
        assert_eq!(memo_only, HeartbeatTickOutcome::MaintenanceOnly);

        let noop = classify_heartbeat_tick_outcome(["noop"], false, false);
        assert_eq!(noop, HeartbeatTickOutcome::Noop);

        let meaningful =
            classify_heartbeat_tick_outcome(["get_current_time", "send_message"], false, false);
        assert_eq!(meaningful, HeartbeatTickOutcome::Acted);

        let text_reply = classify_heartbeat_tick_outcome(["update_heartbeat_memo"], true, false);
        assert_eq!(text_reply, HeartbeatTickOutcome::Acted);

        let budget = classify_heartbeat_tick_outcome(["send_message"], true, true);
        assert_eq!(budget, HeartbeatTickOutcome::BudgetExhausted);
    }

    #[test]
    fn skip_reason_roundtrip_all_variants() {
        use std::str::FromStr;
        for v in [
            HeartbeatSkipReason::NoNewContext,
            HeartbeatSkipReason::QueueFull,
            HeartbeatSkipReason::QuietHours,
            HeartbeatSkipReason::Backpressure,
        ] {
            let s = v.as_str();
            let back = HeartbeatSkipReason::from_str(s).expect("parse");
            assert_eq!(v, back, "roundtrip failed for {s}");
        }
        assert!(HeartbeatSkipReason::from_str("").is_err());
    }
    #[test]
    fn slug_is_deterministic_for_same_content() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-15T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let s1 = build_memo_spill_slug("hello world", now);
        let s2 = build_memo_spill_slug("hello world", now);
        assert_eq!(s1, s2, "same content → same slug");
        assert!(s1.starts_with("20260415-"), "date prefix present: {s1}");
        assert_eq!(s1.len(), 17, "YYYYMMDD-8hex = 17 chars");
        let s3 = build_memo_spill_slug("different", now);
        assert_ne!(s1, s3, "different content → different slug");
    }

    #[test]
    fn synopsis_short_line_kept_whole() {
        assert_eq!(build_memo_synopsis("short line"), "short line");
    }

    #[test]
    fn synopsis_truncated_at_period() {
        let s = build_memo_synopsis("First sentence. More after.");
        assert_eq!(s, "First sentence.");
    }

    #[test]
    fn synopsis_truncated_at_cap_with_ellipsis() {
        let long: String = "x".repeat(200);
        let s = build_memo_synopsis(&long);
        // 160 'x' + '…'
        assert_eq!(s.chars().count(), 161);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn synopsis_empty_content_returns_placeholder() {
        assert_eq!(build_memo_synopsis(""), "(empty)");
        assert_eq!(build_memo_synopsis("   \n\t   "), "(empty)");
    }

    #[test]
    fn heartbeat_tick_state_default_is_empty() {
        let st = HeartbeatTickState::default();
        assert!(st.next_tick_note_bytes.is_none());
        assert!(st.proposals_created.is_empty());
        assert_eq!(st.send_message_count, 0);
        assert_eq!(st.send_message_retries, 0);
    }

    #[test]
    fn synopsis_skips_blank_first_line() {
        let s = build_memo_synopsis("\n\n  actual content here\n");
        assert_eq!(s, "actual content here");
    }

    // ── QuietHours::in_quiet_hours / contains_local_time tests ──────────

    #[test]
    fn quiet_hours_normal_range() {
        // 02:00-07:00 — default
        let qh = QuietHours::default();
        assert!(qh.contains_local_time(2, 0), "exactly at start");
        assert!(qh.contains_local_time(3, 30), "mid-range");
        assert!(qh.contains_local_time(6, 59), "just before end");
        assert!(!qh.contains_local_time(7, 0), "exactly at end (exclusive)");
        assert!(!qh.contains_local_time(1, 59), "before start");
        assert!(!qh.contains_local_time(12, 0), "well outside");
    }

    #[test]
    fn quiet_hours_overnight_wrap() {
        // 22:00-06:00 — crosses midnight
        let qh = QuietHours {
            start: "22:00".into(),
            end: "06:00".into(),
        };
        assert!(qh.contains_local_time(22, 0), "start");
        assert!(qh.contains_local_time(23, 30), "before midnight");
        assert!(qh.contains_local_time(0, 0), "midnight");
        assert!(qh.contains_local_time(3, 0), "early morning");
        assert!(qh.contains_local_time(5, 59), "just before end");
        assert!(!qh.contains_local_time(6, 0), "exactly at end (exclusive)");
        assert!(!qh.contains_local_time(12, 0), "midday");
        assert!(!qh.contains_local_time(21, 59), "just before start");
    }

    #[test]
    fn quiet_hours_with_minutes() {
        // 23:30-06:15 — overnight with minute precision
        let qh = QuietHours {
            start: "23:30".into(),
            end: "06:15".into(),
        };
        assert!(!qh.contains_local_time(23, 29), "1 min before start");
        assert!(qh.contains_local_time(23, 30), "exactly at start");
        assert!(qh.contains_local_time(0, 0), "midnight");
        assert!(qh.contains_local_time(6, 14), "just before end");
        assert!(!qh.contains_local_time(6, 15), "exactly at end");
    }

    #[test]
    fn quiet_hours_full_day_same_start_end() {
        // start == end → empty range (never quiet)
        let qh = QuietHours {
            start: "08:00".into(),
            end: "08:00".into(),
        };
        for h in 0..24 {
            assert!(
                !qh.contains_local_time(h, 0),
                "hour {h} should not be quiet when start==end"
            );
        }
    }

    #[test]
    fn quiet_hours_invalid_format_returns_false() {
        let qh = QuietHours {
            start: "bad".into(),
            end: "07:00".into(),
        };
        assert!(!qh.contains_local_time(3, 0), "bad start format → false");

        let qh2 = QuietHours {
            start: "02:00".into(),
            end: "".into(),
        };
        assert!(!qh2.contains_local_time(3, 0), "empty end → false");
    }

    #[test]
    fn quiet_hours_parse_hhmm_edge_cases() {
        assert_eq!(QuietHours::parse_hhmm("00:00"), Some((0, 0)));
        assert_eq!(QuietHours::parse_hhmm("23:59"), Some((23, 59)));
        assert_eq!(QuietHours::parse_hhmm("24:00"), None); // hour out of range
        assert_eq!(QuietHours::parse_hhmm("12:60"), None); // minute out of range
        assert_eq!(QuietHours::parse_hhmm("12"), None); // no colon
        assert_eq!(QuietHours::parse_hhmm("ab:cd"), None); // non-numeric
    }
}
