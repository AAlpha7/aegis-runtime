use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::embedding::EmbeddingClient;
use crate::identity::AgentIdentity;
use crate::memory::MemoryHandle;
use crate::models::{AlertSeverity, ProbeSignal};
use crate::sandbox::SandboxEngine;
use crate::settings;

// ── Alert cooldown ───────────────────────────────────────────────────────────

const DEFAULT_ALERT_COOLDOWN: Duration = Duration::from_secs(4 * 3600); // 4 hours

/// Maximum wall-clock time for a single heartbeat Wasm probe, including any
/// HTTP fetches made by the skill. Each fetch has a 10 s socket timeout; this
/// cap covers the entire probe so that a slow/hung skill cannot block the
/// Tokio async worker forever.
///
/// 30 s = 3 × the per-request HTTP timeout, giving skills that make a few
/// sequential fetches enough headroom while still bounding the blocking thread.
pub(crate) const HEARTBEAT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Interval for checking if heartbeat is re-enabled when disabled (seconds).
pub(crate) const DISABLED_CHECK_INTERVAL_SECS: u64 = 60;

/// Hour (UTC) at which the daily summary is compiled.
pub(crate) const DAILY_SUMMARY_HOUR: u32 = 9;

/// A user-defined monitoring intent that filters heartbeat alerts by semantic relevance.
///
/// When registered for a skill, only alerts whose payload is semantically similar
/// to the intent description (above the threshold) will be escalated to the LLM.
/// Alerts below the threshold are silently dropped, saving LLM tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringIntent {
    pub skill_name: String,
    pub description: String,
    /// Cosine similarity threshold. Alerts below this are filtered.
    pub threshold: f32,
}

/// Cached monitoring intent data keyed by `(user_did, skill_name)`.
///
/// Monitoring intents are user-scoped. Caching by skill alone lets one user's
/// filter override another user's intent for the same heartbeat skill.
type IntentCache = RwLock<HashMap<(String, String), (Vec<f32>, f32)>>;

/// Per-skill alert state for cooldown tracking.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlertState {
    pub(crate) last_alert_sent: Instant,
    pub(crate) was_alerting: bool,
    /// A recovery arrived inside the resolve debounce and is owed once the
    /// condition has stayed clear long enough.
    pub(crate) resolve_pending: bool,
}

/// What action to take for the current alert cycle.
#[derive(Debug)]
pub(crate) enum CooldownAction {
    /// First alert (no previous state, or was previously resolved).
    Send,
    /// Alert still ongoing, cooldown expired — send a reminder.
    Reminder,
    /// Was alerting, now resolved — send a "[RESOLVED]" notification.
    Resolved,
    /// Alert still ongoing, within cooldown — suppress (no push).
    Suppress,
    /// Not alerting, not previously alerting — normal silent.
    Silent,
}

// ── Alert policy ────────────────────────────────────────────────────────────

/// How alerts are delivered for a specific skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DeliveryMode {
    /// All alerts (Info, Warning, Critical) are pushed immediately.
    All,
    /// Only Warning and above are pushed.
    #[default]
    WarningAndAbove,
    /// Only Critical alerts are pushed.
    CriticalOnly,
    /// All alerts are silently logged, nothing pushed to Telegram.
    Silent,
}

/// Per-skill alert delivery policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertPolicy {
    /// How to filter alerts by severity before sending.
    #[serde(default)]
    pub delivery: DeliveryMode,
    /// Whether to send a "✅ [RESOLVED]" notification when an alert clears.
    /// Default: true (send resolved notifications).
    #[serde(default = "default_true")]
    pub send_resolved: bool,
    /// Per-skill cooldown in seconds. 0 = use the global default (4 hours).
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Optional quiet hours (UTC). During these hours, only Critical alerts are sent.
    /// Format: (start_hour, end_hour) where start_hour > end_hour means overnight.
    /// Default: None (quiet hours disabled — alerts sent at all times).
    #[serde(default)]
    pub quiet_hours: Option<(u8, u8)>,
    /// Whether to pin alert messages in Telegram. Default: true for real alerts.
    #[serde(default = "default_true")]
    pub pin_messages: bool,
    /// Temporary mute: suppress ALL alerts until this Unix timestamp.
    /// After this time, alerts resume automatically under the normal policy.
    /// Set via `mute_hours` parameter. None = not muted.
    #[serde(default)]
    pub muted_until: Option<i64>,
}

fn default_true() -> bool {
    true
}

impl Default for AlertPolicy {
    fn default() -> Self {
        Self {
            delivery: DeliveryMode::WarningAndAbove,
            send_resolved: true,
            cooldown_secs: 0,
            quiet_hours: None,
            pin_messages: true,
            muted_until: None,
        }
    }
}

impl AlertPolicy {
    /// Check if the current UTC time falls within quiet hours.
    /// During quiet hours, only Critical alerts should be sent.
    pub fn in_quiet_hours(&self, now_utc_hour: u8) -> bool {
        match self.quiet_hours {
            None => false,
            Some((start, end)) if start <= end => {
                // Same-day range: e.g., (9, 17) = 9am-5pm
                now_utc_hour >= start && now_utc_hour < end
            }
            Some((start, end)) => {
                // Overnight range: e.g., (22, 6) = 10pm-6am
                now_utc_hour >= start || now_utc_hour < end
            }
        }
    }

    /// Check if the skill is currently muted (all alerts suppressed until `muted_until`).
    pub fn is_muted(&self) -> bool {
        if let Some(until) = self.muted_until {
            let now = chrono::Utc::now().timestamp();
            now < until
        } else {
            false
        }
    }

    /// Check if an alert with the given severity should be delivered under this policy.
    pub fn should_deliver(&self, severity: AlertSeverity, now_utc_hour: u8) -> bool {
        use AlertSeverity::*;
        use DeliveryMode::*;

        // Temporary mute: suppress ALL alerts until muted_until expires
        if self.is_muted() {
            return false;
        }

        // Silent mode = never deliver
        if self.delivery == Silent {
            return false;
        }

        // During quiet hours, only Critical alerts pass
        if self.in_quiet_hours(now_utc_hour) && severity < Critical {
            return false;
        }

        // Check delivery mode threshold
        match self.delivery {
            All => true,
            WarningAndAbove => severity >= Warning,
            CriticalOnly => severity >= Critical,
            Silent => false, // handled above but for completeness
        }
    }

    /// Get the effective cooldown duration for this policy.
    pub fn effective_cooldown(&self, global_default: Duration) -> Duration {
        if self.cooldown_secs > 0 {
            Duration::from_secs(self.cooldown_secs)
        } else {
            global_default
        }
    }
}

// ── Autonomous heartbeat config ─────────────────────────────────────────────

/// Configuration for autonomous actions during heartbeat cycles.
///
/// When enabled for a skill, the heartbeat engine will invoke `on_autonomous()`
/// on the handler before falling through to the standard `on_alert()` path.
/// This allows the LLM to take action (tool calls, notifications) without
/// human intervention, within configurable safety bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousConfig {
    /// Whether autonomous mode is enabled. Default: false (standard alert path).
    #[serde(default)]
    pub enabled: bool,
    /// Maximum LLM reasoning rounds per heartbeat cycle.
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    /// Maximum tool calls the LLM may make per heartbeat cycle.
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    /// Allowlist of tools the LLM may call. Empty = use `default_autonomous_allowed_tools()`.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Denylist of tools the LLM must never call, even if in the allowlist.
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// Maximum tokens the autonomous system may spend per calendar day (UTC).
    #[serde(default = "default_daily_token_budget")]
    pub daily_token_budget: u32,
    /// When to notify the user about autonomous actions.
    #[serde(default)]
    pub notification_mode: NotificationMode,
}

/// When the user should be notified about autonomous heartbeat actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum NotificationMode {
    /// Always notify, even if no action was taken.
    Always,
    /// Notify only when an action was actually taken (default).
    #[default]
    OnAction,
    /// Never notify (silent autonomous operation).
    Never,
}

fn default_max_rounds() -> usize {
    5
}
fn default_max_tool_calls() -> usize {
    8
}
fn default_daily_token_budget() -> u32 {
    500_000
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_rounds: default_max_rounds(),
            max_tool_calls: default_max_tool_calls(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            daily_token_budget: default_daily_token_budget(),
            notification_mode: NotificationMode::default(),
        }
    }
}

// ── True Heartbeat Configuration ──────────────────────────────────────────

fn default_interval_minutes() -> u32 {
    30
}

/// Configuration for the True Heartbeat: a fixed-interval timer that
/// unconditionally invokes the LLM with a full system-state snapshot.
///
/// Unlike probe-based heartbeats, the True Heartbeat does NOT depend on
/// any skill's Wasm probe returning `require_llm_attention: true`.
/// The LLM receives a [`SituationSnapshot`] and decides what to do.
///
/// Reuses [`AutonomousConfig`] for tool gating, budgets, and notification
/// settings to avoid fragmentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrueHeartbeatConfig {
    /// Whether the true heartbeat is enabled. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Interval in minutes between true heartbeat cycles. Default: 30.
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u32,
    /// Legacy field — quiet hours are now controlled by `HeartbeatConfig.quiet_hours`
    /// (user-local timezone, `HH:MM` strings). Kept for serde backward compat
    /// with old DB rows; ignored at runtime.
    #[serde(default, skip_serializing)]
    pub quiet_hours: Option<(u8, u8)>,
    /// Autonomous config overrides for true heartbeat cycles.
    /// If None, uses the global AutonomousConfig.
    #[serde(default)]
    pub autonomous_overrides: Option<AutonomousConfig>,
}

impl Default for TrueHeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: default_interval_minutes(),
            quiet_hours: None,
            autonomous_overrides: None,
        }
    }
}

impl TrueHeartbeatConfig {
    /// Resolve the effective AutonomousConfig for this true heartbeat.
    /// Uses overrides if set, otherwise falls back to `fallback`.
    pub fn effective_autonomous_config(&self, fallback: &AutonomousConfig) -> AutonomousConfig {
        self.autonomous_overrides
            .clone()
            .unwrap_or_else(|| fallback.clone())
    }
}

// ── Situation Snapshot ───────────────────────────────────────────────────

/// Full system-state snapshot compiled for the True Heartbeat.
///
/// Assembled from multiple memory queries and passed to the LLM
/// as a structured context block via [`to_prompt_block()`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SituationSnapshot {
    /// Timestamp when this snapshot was compiled.
    pub compiled_at: String,
    /// User's configured timezone name.
    pub timezone: String,
    /// Recent heartbeat audit entries (all skills, last N cycles).
    pub recent_probe_results: Vec<crate::memory::HeartbeatAuditEntry>,
    /// Pending/active tasks and reminders.
    pub pending_tasks: Vec<crate::memory::Task>,
    /// Upcoming deferred intents (time-based triggers due within 1 hour).
    pub upcoming_intents: Vec<crate::memory::DeferredIntent>,
    /// System health: per-skill metrics.
    pub skill_metrics: Vec<crate::memory::SkillMetrics>,
    /// Circuit breaker states for skills currently tripped: (skill, consecutive_failures, last_error).
    pub tripped_circuit_breakers: Vec<(String, i64, Option<String>)>,
    /// Pending HITL approval requests: (key, value).
    pub hitl_pending: Vec<(String, String)>,
    /// Daily autonomous token budget: (used, total).
    pub budget_status: (u32, u32),
    /// Unix timestamp of user's last message (from L1). None if unknown.
    pub user_last_active_unix: Option<i64>,
}

impl SituationSnapshot {
    /// Render the snapshot as a structured XML block for the LLM system prompt.
    pub fn to_prompt_block(&self) -> String {
        let mut out = String::with_capacity(2048);
        out.push_str(&format!(
            "<situation_snapshot compiled=\"{}\" tz=\"{}\">\n",
            self.compiled_at, self.timezone
        ));

        // Budget
        out.push_str(&format!(
            "TOKEN BUDGET: {}/{} used today\n\n",
            self.budget_status.0, self.budget_status.1
        ));

        // User activity
        if let Some(last) = self.user_last_active_unix {
            let ago_secs = (chrono::Utc::now().timestamp() - last).max(0);
            let ago_mins = ago_secs / 60;
            out.push_str(&format!("USER LAST ACTIVE: {} minutes ago\n\n", ago_mins));
        } else {
            out.push_str("USER LAST ACTIVE: unknown\n\n");
        }

        // Pending tasks
        if !self.pending_tasks.is_empty() {
            out.push_str(&format!("PENDING TASKS ({}):\n", self.pending_tasks.len()));
            for t in self.pending_tasks.iter().take(10) {
                let due = t.due_date.as_deref().unwrap_or("no due date");
                out.push_str(&format!(
                    "  - [{}] {} (due: {}, status: {})\n",
                    t.priority, t.title, due, t.status
                ));
            }
            out.push('\n');
        }

        // Upcoming intents (times in user timezone)
        if !self.upcoming_intents.is_empty() {
            let tz = crate::settings::parse_tz_str(&self.timezone);
            out.push_str(&format!(
                "UPCOMING TRIGGERS ({}):\n",
                self.upcoming_intents.len()
            ));
            for i in self.upcoming_intents.iter().take(5) {
                let trigger = i
                    .trigger_at_unix
                    .map(|ts| {
                        chrono::DateTime::from_timestamp(ts, 0)
                            .map(|dt| dt.with_timezone(&tz).format("%H:%M").to_string())
                            .unwrap_or_else(|| ts.to_string())
                    })
                    .unwrap_or_else(|| "event-based".to_string());
                out.push_str(&format!("  - {} (fires at: {})\n", i.context_text, trigger));
            }
            out.push('\n');
        }

        // System health — only show concerns
        let unhealthy: Vec<_> = self
            .skill_metrics
            .iter()
            .filter(|m| m.circuit_open || m.consecutive_failures > 0)
            .collect();
        if !unhealthy.is_empty() {
            out.push_str("SYSTEM HEALTH CONCERNS:\n");
            for m in &unhealthy {
                out.push_str(&format!(
                    "  - {}: {} consecutive failures, circuit {}{}\n",
                    m.skill_name,
                    m.consecutive_failures,
                    if m.circuit_open { "OPEN" } else { "closed" },
                    m.last_error
                        .as_ref()
                        .map(|e| format!(", last error: {}", e))
                        .unwrap_or_default(),
                ));
            }
            out.push('\n');
        } else if !self.skill_metrics.is_empty() {
            out.push_str(&format!(
                "SYSTEM HEALTH: All {} skills healthy\n\n",
                self.skill_metrics.len()
            ));
        }

        // Tripped circuit breakers (supplement to metrics)
        if !self.tripped_circuit_breakers.is_empty() {
            out.push_str("TRIPPED CIRCUIT BREAKERS:\n");
            for (skill, failures, reason) in &self.tripped_circuit_breakers {
                out.push_str(&format!(
                    "  - {} ({} failures{})\n",
                    skill,
                    failures,
                    reason
                        .as_ref()
                        .map(|r| format!(": {}", r))
                        .unwrap_or_default(),
                ));
            }
            out.push('\n');
        }

        // HITL pending
        if !self.hitl_pending.is_empty() {
            out.push_str(&format!(
                "PENDING APPROVALS ({}):\n",
                self.hitl_pending.len()
            ));
            for (key, _value) in self.hitl_pending.iter().take(5) {
                out.push_str(&format!("  - {}\n", key));
            }
            out.push('\n');
        }

        // Recent probe results (timestamps converted to user timezone)
        if !self.recent_probe_results.is_empty() {
            let tz = crate::settings::parse_tz_str(&self.timezone);
            out.push_str(&format!(
                "RECENT PROBE RESULTS ({}):\n",
                self.recent_probe_results.len()
            ));
            for entry in self.recent_probe_results.iter().take(10) {
                let tools_str = if entry.tools_called.is_empty() {
                    "no tools".to_string()
                } else {
                    entry.tools_called.join(", ")
                };
                // Convert DB UTC timestamp to user's local timezone
                let local_ts =
                    chrono::NaiveDateTime::parse_from_str(&entry.timestamp, "%Y-%m-%d %H:%M:%S")
                        .map(|naive| {
                            naive
                                .and_utc()
                                .with_timezone(&tz)
                                .format("%H:%M %Z")
                                .to_string()
                        })
                        .unwrap_or_else(|_| entry.timestamp.clone());
                out.push_str(&format!(
                    "  - [{}] {} (severity: {}, tools: {})\n",
                    local_ts, entry.skill_name, entry.severity, tools_str
                ));
            }
            out.push('\n');
        }

        out.push_str("</situation_snapshot>");
        out
    }
}

/// Default set of tools considered safe for autonomous heartbeat actions.
/// These are read-only or notification-only tools that cannot cause harm.
pub fn default_autonomous_allowed_tools() -> Vec<String> {
    vec![
        "web_search",
        "x_search",
        "web_reader",
        "server_monitor",
        "us_stock_quote",
        "recall_memory",
        // FU#2 Phase 6: also allowed in autonomous heartbeat cycles. Registry
        // schema + validator gate at write time; cannot corrupt profile.
        "update_profile",
        "get_current_time",
        "knowledge_base",
        "notify_user",
        "news_digest",
        "calendar_reader",
        "update_heartbeat_memo",
        // Step 8a heartbeat-scoped tools. The ToolGate `HeartbeatAllowed`
        // policy is the authoritative scope enforcer; listed here so
        // `self_audit` and the pre-filter layer recognize them as known
        // heartbeat-mode names.
        "noop",
        "write_memory",
        "write_next_tick_note",
        // Step 9b approval-gated proposal tools.
        "propose_config_change",
        // Step 10 — rate/source/dedup-gated proactive message.
        "send_message",
        // Subcall primitive (design §6.3) — lets the heartbeat LLM
        // self-decide to delegate long explorations to isolated subcalls
        // instead of burning its own tick context. `final_answer` is
        // deliberately NOT added here — it's `SubcallOnly` policy and
        // only the inner subcall LLM should call it.
        "spawn_subcall",
        "recall_subcall",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
mod allowed_tools_tests {
    use super::*;

    #[test]
    fn heartbeat_allowed_tools_include_subcall_primitives() {
        let tools = default_autonomous_allowed_tools();
        assert!(
            tools.iter().any(|t| t == "spawn_subcall"),
            "heartbeat allowed-tools must include spawn_subcall (design §6.3)"
        );
        assert!(
            tools.iter().any(|t| t == "recall_subcall"),
            "heartbeat allowed-tools must include recall_subcall (design §6.3)"
        );
        assert!(
            !tools.iter().any(|t| t == "final_answer"),
            "final_answer is SubcallOnly policy — only the inner subcall LLM \
             should call it; excluding it from heartbeat scope prevents the \
             parent heartbeat tick from accidentally exiting as a subcall"
        );
    }
}

/// Outcome of an autonomous heartbeat action cycle.
///
/// Unified with the chat-turn return type per design §2.1 / §15.2 Step 3:
/// heartbeat ticks and user turns now share the same
/// [`crate::orchestrator::OrchestratorResult`] shape so every caller /
/// audit / test sees one model. The alias is retained to keep existing
/// call sites readable — new code should prefer `OrchestratorResult`.
pub type AutonomousOutcome = crate::orchestrator::OrchestratorResult;

/// Per-tool-call timing and metadata, captured during the ReAct loop.
/// Serialized to JSON and stored in heartbeat_audit_log.tool_call_details.
/// Used by credit assignment (P1) to analyze tool effectiveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDetail {
    /// Name of the tool that was called.
    pub tool_name: String,
    /// Which ReAct round this call occurred in (1-indexed).
    pub round: usize,
    /// Wall-clock latency of the tool execution in milliseconds.
    pub latency_ms: u64,
    /// Size of the tool result in bytes.
    pub result_size: usize,
    /// Whether the tool was blocked by the gate policy.
    pub was_blocked: bool,
    /// First 200 characters of the tool result, for embedding-based citation detection.
    pub result_preview: String,
}

/// Outcome classification for credit assignment.
/// Tags each autonomous cycle as success, fallthrough, or no-action.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum OutcomeTag {
    /// Completed with meaningful actions taken.
    Success,
    /// LLM could not handle the situation, fell through to alert path.
    Fallthrough,
    /// Completed but took no meaningful action.
    #[default]
    NoAction,
}

/// Aggregated credit score for a single tool over a time period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCreditScore {
    pub tool_name: String,
    pub times_called: u32,
    pub times_cited: u32,
    /// Effectiveness = (times_cited / times_called) * success_rate
    pub effectiveness_score: f64,
    pub wasted_calls: u32,
    pub avg_latency_ms: f64,
}

/// Complete credit analysis report for a time period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditAnalysisReport {
    pub analysis_date: String,
    pub tool_scores: Vec<ToolCreditScore>,
    /// Tools ranked by effectiveness score (highest first).
    pub top_effective: Vec<String>,
    /// Tools with highest wasted call count.
    pub most_wasteful: Vec<String>,
}

/// A proposed change to a prompt module from the auto-optimization engine.
/// Generated by analyzing failure patterns and tool effectiveness data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptChangeProposal {
    pub module_name: String,
    pub original_text: String,
    pub modified_text: String,
    pub rationale: String,
}

/// Built-in default alert policies for known skills.
/// Skills not listed here get AlertPolicy::default() (WarningAndAbove, send_resolved=true, global cooldown).
pub fn default_alert_policies() -> HashMap<String, AlertPolicy> {
    let mut m = HashMap::new();

    m.insert(
        "server_monitor".to_string(),
        AlertPolicy {
            delivery: DeliveryMode::WarningAndAbove,
            send_resolved: true,
            cooldown_secs: 7200, // 2 hours
            quiet_hours: None,
            pin_messages: true,
            muted_until: None,
        },
    );

    m.insert(
        "commute_traffic_monitor".to_string(),
        AlertPolicy {
            delivery: DeliveryMode::All,
            send_resolved: false,
            cooldown_secs: 1800, // 30 minutes
            quiet_hours: None,
            pin_messages: false,
            muted_until: None,
        },
    );

    m
}

/// Result of a heartbeat probe execution.
#[derive(Debug)]
pub struct HeartbeatResult {
    pub skill_name: String,
    pub probe: ProbeSignal,
    pub elapsed_ms: u128,
}

/// The Chronos heartbeat engine.
///
/// Three-tier rocket model for token-bankruptcy prevention:
/// 1. `tokio-cron-scheduler` triggers on schedule
/// 2. System DID forges IntentEnvelope (bypasses LLM — 0 tokens)
/// 3. Wasm engine silently executes probe (~50ms, 0 tokens)
/// 4. Read ProbeSignal:
///    - `require_llm_attention == false` → silent drop, instance destroyed (0 tokens, 0 push)
///    - `require_llm_attention == true`  → escalate: LLM polishes alert → Telegram push
pub struct ChronosEngine {
    sandbox: Arc<SandboxEngine>,
    /// System identity used to sign heartbeat IntentEnvelopes.
    /// Goes through the same Ed25519 verification path as user envelopes.
    system_identity: Arc<AgentIdentity>,
    /// Shared runtime settings — used to gate heartbeats for disabled features.
    settings: Arc<settings::RuntimeSettings>,
    /// Per-skill alert state for cooldown suppression.
    /// See `with_resolve_debounce`: how long a probe must stay clear before ✅.
    pub(crate) resolve_debounce: Duration,
    /// Hold non-critical probe alerts during the owner's quiet hours
    /// (`heartbeat_config`, owner timezone). Off under `cfg(test)` so the
    /// cooldown state-machine tests do not depend on the wall clock.
    pub(crate) honor_owner_quiet_hours: bool,
    pub(crate) alert_state: Mutex<HashMap<String, AlertState>>,
    /// How long to suppress repeated alerts for the same skill.
    pub(crate) alert_cooldown: Duration,
    /// Per-skill execution lock to prevent overlapping heartbeats.
    /// If a heartbeat is still running when the next cron tick fires, the new one is skipped.
    running_skills: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Embedding client for semantic triage of heartbeat alerts.
    /// None when constructed without triage support (backward compatible).
    embedder: Option<EmbeddingClient>,
    /// Memory handle for loading monitoring intents from L2.
    /// None when constructed without triage support (backward compatible).
    memory: Option<MemoryHandle>,
    /// Cached monitoring intents: `(user_did, skill_name)` → `(embedding, threshold)`.
    /// Refreshed on set/remove operations; avoids L2 queries on every heartbeat.
    intent_cache: IntentCache,
    /// Per-skill alert policies. Skills not in the map use AlertPolicy::default().
    alert_policies: RwLock<HashMap<String, AlertPolicy>>,
    /// Per-skill autonomous action configs. Skills not in the map use the global config.
    autonomous_configs: RwLock<HashMap<String, AutonomousConfig>>,
    /// Global autonomous config applied when no per-skill config exists.
    global_autonomous_config: RwLock<AutonomousConfig>,
    /// Running total of tokens consumed by autonomous actions today (UTC).
    daily_autonomous_tokens: std::sync::atomic::AtomicU32,
    /// The UTC date (YYYY-MM-DD) when `daily_autonomous_tokens` was last reset.
    daily_token_reset_date: Mutex<String>,
    /// Configuration for the True Heartbeat (fixed-interval, unconditional LLM wake).
    true_heartbeat_config: RwLock<TrueHeartbeatConfig>,
    /// Set once `start_scheduler_with_accounts` has successfully started the
    /// cron scheduler in this process. Runtime account-loop spawning
    /// (`register_account_cron_jobs`) consults this to know whether a live
    /// scheduler exists to add jobs to.
    scheduler_started: std::sync::atomic::AtomicBool,
    /// Runtime cron-registration context, captured by
    /// `start_scheduler_with_accounts` after a successful start so accounts
    /// added at RUNTIME get their per-account cron jobs immediately instead
    /// of waiting for the next restart (multi-mailbox Slice 1 W1). `None`
    /// until the scheduler starts (or in deployments without cron skills).
    pub(crate) runtime_cron: Mutex<Option<RuntimeCronContext>>,
}

/// Everything needed to add/remove per-account cron jobs AFTER startup:
/// the live scheduler handle, the same heartbeat handler the startup jobs
/// captured, and the per-account job slots (for removal when an account is
/// deleted). See `ChronosEngine::register_account_cron_jobs`.
pub(crate) struct RuntimeCronContext {
    pub(crate) sched: tokio_cron_scheduler::JobScheduler,
    pub(crate) handler: Arc<dyn HeartbeatHandler>,
    /// account_id → this account's registered (or in-flight) cron-job slot.
    /// Presence of a key (even with empty guids, used as an in-flight
    /// reservation) means the account's jobs are registered or being
    /// registered — the dup-guard for concurrent adds.
    pub(crate) account_jobs: HashMap<String, AccountJobsSlot>,
    /// Monotonic generation source for reservations. A remove-and-readd of
    /// the SAME account id mid-registration creates a NEW slot with a NEW
    /// generation, so the stale in-flight registration can detect that its
    /// reservation was consumed and roll its jobs back instead of recording
    /// them into the new generation's slot (Codex adversarial round 2).
    pub(crate) next_generation: u64,
    /// Guids of the NO-account fallback cron jobs that a zero-account startup
    /// registers for multi-account skills. Once the FIRST runtime account is
    /// registered these become duplicates (the fallback resolves to the new
    /// default account, double-probing it every tick — parent Codex P2), so
    /// the first successful `register_account_cron_jobs` drains this list and
    /// removes the jobs from the scheduler. Empty when startup had accounts.
    /// Asymmetry: removing the LAST account does NOT restore the fallback
    /// (it returns on the next restart); a fallback probe without any account
    /// has nothing useful to probe, so nothing is lost in the interim.
    pub(crate) multi_account_fallback_guids: Vec<uuid::Uuid>,
}

/// One account's cron-job slot: the reservation generation plus the guids of
/// the jobs registered under it (empty while the registration is in flight).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountJobsSlot {
    pub(crate) generation: u64,
    pub(crate) guids: Vec<uuid::Uuid>,
}

/// Callback trait for handling heartbeat results.
/// Allows decoupling from concrete LLM/Telegram implementations for testability.
pub trait HeartbeatHandler: Send + Sync {
    /// Called when a probe requires LLM attention (escalation).
    /// Implementation should call LLM to polish the alert, then push via Telegram.
    fn on_alert(
        &self,
        skill_name: &str,
        probe: &ProbeSignal,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>>;

    /// Called when a probe is silent (no attention needed).
    /// Default: do nothing (0 tokens, 0 push).
    fn on_silent(&self, _skill_name: &str, _probe: &ProbeSignal) {
        // Silent drop — the whole point of the three-tier rocket
    }

    /// Called after every heartbeat dispatch (alert or silent) with the probe result.
    /// Default: no-op. Override to record observations for proactive learning.
    fn on_observation(&self, _skill_name: &str, _probe: &ProbeSignal, _was_alert: bool) {
        // Default no-op — backwards compatible
    }

    /// Called by the True Heartbeat timer with a pre-compiled [`SituationSnapshot`].
    ///
    /// Unlike `on_autonomous` (which reacts to a specific skill's probe), this receives
    /// a full system-state snapshot and should invoke the LLM unconditionally.
    ///
    /// Default implementation: falls through (no autonomous action taken).
    fn on_true_heartbeat(
        &self,
        _snapshot: &SituationSnapshot,
        _config: &AutonomousConfig,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AutonomousOutcome>> + Send + '_>>
    {
        Box::pin(async {
            Ok(AutonomousOutcome {
                fell_through_to_alert: true,
                outcome_tag: OutcomeTag::Fallthrough,
                ..Default::default()
            })
        })
    }

    /// G7: Called by the True Heartbeat loop to run upstream health probes.
    /// Default: no-op. Override to check OpenRouter/Google/Calendar availability.
    fn on_health_probe(
        &self,
        _chat_id: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

impl ChronosEngine {
    pub fn new(
        sandbox: Arc<SandboxEngine>,
        system_identity: Arc<AgentIdentity>,
        settings: Arc<settings::RuntimeSettings>,
    ) -> Self {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Self {
            sandbox,
            system_identity,
            settings,
            resolve_debounce: Duration::from_secs(15 * 60),
            honor_owner_quiet_hours: !cfg!(test),
            alert_state: Mutex::new(HashMap::new()),
            alert_cooldown: DEFAULT_ALERT_COOLDOWN,
            running_skills: Mutex::new(HashMap::new()),
            embedder: None,
            memory: None,
            intent_cache: RwLock::new(HashMap::new()),
            alert_policies: RwLock::new(default_alert_policies()),
            autonomous_configs: RwLock::new(HashMap::new()),
            global_autonomous_config: RwLock::new(AutonomousConfig::default()),
            daily_autonomous_tokens: std::sync::atomic::AtomicU32::new(0),
            daily_token_reset_date: Mutex::new(today),
            true_heartbeat_config: RwLock::new(TrueHeartbeatConfig::default()),
            scheduler_started: std::sync::atomic::AtomicBool::new(false),
            runtime_cron: Mutex::new(None),
        }
    }

    /// True once the cron scheduler has been started in this process.
    pub fn scheduler_running(&self) -> bool {
        self.scheduler_started
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record that the cron scheduler started (called by
    /// `start_scheduler_with_accounts` after a successful `sched.start()`).
    pub(crate) fn mark_scheduler_started(&self) {
        self.scheduler_started
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // ── Accessor methods for extracted impl blocks ───────────────────────

    /// Reference to the sandbox engine.
    pub(crate) fn sandbox_ref(&self) -> &SandboxEngine {
        &self.sandbox
    }

    /// Arc clone of the sandbox engine (for spawn_blocking).
    pub(crate) fn sandbox_arc(&self) -> Arc<SandboxEngine> {
        Arc::clone(&self.sandbox)
    }

    /// Reference to the system identity.
    pub(crate) fn system_identity_ref(&self) -> &AgentIdentity {
        &self.system_identity
    }

    /// Arc clone of the system identity (for spawn_blocking).
    pub(crate) fn system_identity_arc(&self) -> Arc<AgentIdentity> {
        Arc::clone(&self.system_identity)
    }

    /// Reference to the runtime settings.
    pub(crate) fn settings_ref(&self) -> &settings::RuntimeSettings {
        &self.settings
    }

    /// Optional reference to the embedding client.
    pub(crate) fn embedder(&self) -> Option<&EmbeddingClient> {
        self.embedder.as_ref()
    }

    /// Optional reference to the memory handle.
    pub(crate) fn memory_ref(&self) -> Option<&MemoryHandle> {
        self.memory.as_ref()
    }

    /// Lock the alert state mutex (for cooldown logic in chronos_probes).
    pub(crate) fn alert_state_lock(
        &self,
    ) -> parking_lot::MutexGuard<'_, HashMap<String, AlertState>> {
        self.alert_state.lock()
    }

    /// Get the alert cooldown duration.
    pub(crate) fn alert_cooldown_duration(&self) -> Duration {
        self.alert_cooldown
    }

    /// Read reference to the global autonomous config.
    pub(crate) fn global_autonomous_config_ref(&self) -> &RwLock<AutonomousConfig> {
        &self.global_autonomous_config
    }

    /// Read reference to the true heartbeat config.
    pub(crate) fn true_heartbeat_config_ref(&self) -> &RwLock<TrueHeartbeatConfig> {
        &self.true_heartbeat_config
    }

    /// Raw daily token counter (no reset check).
    pub(crate) fn daily_tokens_used_raw(&self) -> u32 {
        self.daily_autonomous_tokens.load(Ordering::Relaxed)
    }

    // ── Triage / intent cache ────────────────────────────────────────────

    /// Set the embedding client and memory handle to enable semantic triage.
    ///
    /// When both are set, heartbeat alerts are filtered by monitoring intents:
    /// only alerts whose payload is semantically similar to a registered intent
    /// (above the threshold) are escalated.
    pub fn set_triage(&mut self, embedder: EmbeddingClient, memory: MemoryHandle) {
        self.embedder = Some(embedder);
        self.memory = Some(memory);
    }

    /// Test-only: attach a memory handle without requiring an embedder.
    /// Production paths must use [`set_triage`] so the monitoring-intent
    /// similarity filter has an embedder. Archive paths tolerate
    /// `embedder = None` (embedding is best-effort).
    #[cfg(test)]
    pub(crate) fn set_memory_for_tests(&mut self, memory: MemoryHandle) {
        self.memory = Some(memory);
    }

    /// Update the local intent cache for a user-scoped skill intent.
    pub fn cache_intent(
        &self,
        user_did: &str,
        skill_name: &str,
        embedding: Vec<f32>,
        threshold: f32,
    ) {
        self.intent_cache.write().insert(
            (user_did.to_string(), skill_name.to_string()),
            (embedding, threshold),
        );
    }

    /// Remove a user-scoped skill intent from the local cache.
    pub fn uncache_intent(&self, user_did: &str, skill_name: &str) {
        self.intent_cache
            .write()
            .remove(&(user_did.to_string(), skill_name.to_string()));
    }

    /// Look up a cached monitoring intent for one user + skill pair.
    pub(crate) fn cached_intent(
        &self,
        user_did: &str,
        skill_name: &str,
    ) -> Option<(Vec<f32>, f32)> {
        self.intent_cache
            .read()
            .get(&(user_did.to_string(), skill_name.to_string()))
            .cloned()
    }
    /// Extract the monitoring-intent owner DID from a heartbeat payload.
    ///
    /// `user_did` is preferred. `chat_id` falls back to the canonical Telegram DID
    /// so older/default payloads that only carry chat identity still resolve.
    pub(crate) fn monitoring_intent_owner_did(payload: &serde_json::Value) -> Option<String> {
        if let Some(user_did) = payload.get("user_did").and_then(|v| v.as_str())
            && !user_did.is_empty()
        {
            return Some(user_did.to_string());
        }

        payload
            .get("chat_id")
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
            })
            .map(|chat_id| format!("did:aegis:telegram:{chat_id}"))
    }

    /// Load all monitoring intents from the unified tasks table into the local cache.
    /// Called at startup and after set/remove operations.
    /// The `_did` parameter is kept for backward compatibility but is no longer used
    /// since `get_all_monitoring_intents` returns intents across all users.
    pub async fn refresh_intent_cache(&self, _did: &str) {
        let Some(memory) = &self.memory else { return };
        // Query from the unified tasks table (trigger_type = 'semantic_alert')
        let records = match memory.get_all_monitoring_intents().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load monitoring intents from tasks table");
                return;
            }
        };

        let mut cache = self.intent_cache.write();
        cache.clear();
        for record in records {
            cache.insert(
                (record.user_did.clone(), record.monitoring_skill.clone()),
                (record.embedding, record.monitoring_threshold),
            );
        }
        tracing::info!(
            count = cache.len(),
            "monitoring intent cache refreshed from tasks table"
        );
    }

    /// Get or create the per-skill AtomicBool execution lock.
    pub(crate) fn running_flag(&self, skill_name: &str) -> Arc<AtomicBool> {
        let mut map = self.running_skills.lock();
        map.entry(skill_name.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    // ── Alert policy management ──────────────────────────────────────────

    /// Get the alert policy for a skill (falls back to default if not configured).
    pub fn get_policy(&self, skill_name: &str) -> AlertPolicy {
        self.alert_policies
            .read()
            .get(skill_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Set a custom alert policy for a skill.
    pub fn set_policy(&self, skill_name: &str, policy: AlertPolicy) {
        self.alert_policies
            .write()
            .insert(skill_name.to_string(), policy);
    }

    /// Remove custom policy for a skill (reverts to default).
    pub fn remove_policy(&self, skill_name: &str) {
        self.alert_policies.write().remove(skill_name);
    }

    /// List all configured policies (both default and custom).
    pub fn list_policies(&self) -> HashMap<String, AlertPolicy> {
        self.alert_policies.read().clone()
    }

    // ── Autonomous config management ──────────────────────────────────────

    /// Get the effective autonomous config for a skill.
    ///
    /// Resolution order: per-skill config → global config → AutonomousConfig::default().
    pub fn get_autonomous_config(&self, skill_name: &str) -> AutonomousConfig {
        if let Some(cfg) = self.autonomous_configs.read().get(skill_name) {
            return cfg.clone();
        }
        self.global_autonomous_config.read().clone()
    }

    /// Set a per-skill autonomous config.
    pub fn set_autonomous_config(&self, skill_name: &str, config: AutonomousConfig) {
        self.autonomous_configs
            .write()
            .insert(skill_name.to_string(), config);
    }

    /// Set the global autonomous config (used for skills without per-skill config).
    pub fn set_global_autonomous_config(&self, config: AutonomousConfig) {
        *self.global_autonomous_config.write() = config;
    }

    /// Check if the daily autonomous token budget has capacity for at least one
    /// more cycle under the given config.
    pub fn within_daily_budget(&self, config: &AutonomousConfig) -> bool {
        self.maybe_reset_daily_counter();
        let used = self.daily_autonomous_tokens.load(Ordering::Relaxed);
        used < config.daily_token_budget
    }

    /// Reset the daily token counter if the UTC date has rolled over.
    pub fn maybe_reset_daily_counter(&self) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut reset_date = self.daily_token_reset_date.lock();
        if *reset_date != today {
            self.daily_autonomous_tokens.store(0, Ordering::Relaxed);
            *reset_date = today;
        }
    }

    /// Record token usage from an autonomous cycle.
    pub fn track_autonomous_tokens(&self, tokens: u32) {
        self.daily_autonomous_tokens
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// Get the number of tokens used today by autonomous actions.
    pub fn daily_tokens_used(&self) -> u32 {
        self.maybe_reset_daily_counter();
        self.daily_autonomous_tokens.load(Ordering::Relaxed)
    }

    /// Restore the daily token counter from the heartbeat audit log in the database.
    /// This should be called once at startup to recover the counter after process restart.
    pub async fn restore_daily_tokens_from_db(&self, memory: &crate::memory::MemoryHandle) {
        match memory.sum_todays_heartbeat_tokens().await {
            Ok(tokens) => {
                self.daily_autonomous_tokens
                    .store(tokens, Ordering::Relaxed);
                tracing::info!(
                    restored_tokens = tokens,
                    "[Chronos] Restored daily token counter from audit log"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[Chronos] Failed to restore daily token counter from DB (starting at 0)"
                );
            }
        }
    }
}

/// Outcome of a heartbeat cycle.
#[derive(Debug)]
pub enum HeartbeatOutcome {
    /// Probe returned require_llm_attention=false. 0 tokens consumed, 0 pushes.
    Silent {
        skill_name: String,
        elapsed_ms: u128,
    },
    /// Probe returned require_llm_attention=true. LLM was called, alert was pushed.
    Escalated {
        skill_name: String,
        alert_message: String,
        elapsed_ms: u128,
    },
    /// Alert was suppressed due to cooldown. 0 tokens consumed, 0 pushes.
    Suppressed {
        skill_name: String,
        elapsed_ms: u128,
    },
}

#[cfg(test)]
#[path = "chronos_tests.rs"]
mod tests;
