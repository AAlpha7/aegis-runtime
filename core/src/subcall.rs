//! Subcall primitive — delegate bounded subtasks to a fresh LLM context.
//!
//! See `docs/subcall-primitive-design.md` for the full design. High-level
//! contract: a caller (either the parent LLM via `spawn_subcall` tool, or
//! Rust code) submits a `SubcallSpec`;
//! we run a bounded ReAct loop in a fresh context; we return a
//! `SubcallResult` with structured output and an explicit `SubcallStatus`
//! for every way the run can end (success, budget exceeded, timeout,
//! tool blocked, schema violation, etc.).
//!
//! Transparency invariant: every system-imposed constraint is visible
//! in the returned `SubcallResult` (`status`, `clamped_budgets`, or
//! `last_assistant_text`) — never silent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_openai::types::chat::ChatCompletionRequestMessage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Identifier ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubcallId(String);

impl SubcallId {
    pub fn new() -> Self {
        Self(format!("sc_{}", Uuid::new_v4().simple()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct a `SubcallId` from an externally-supplied string. Used by
    /// tool handlers (e.g. `recall_subcall`) to look up IDs surfaced to the
    /// LLM in prior subcall results. Caller is responsible for validation —
    /// `SubcallRegistry::recall` surfaces a `not_found` error for unknown ids.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

impl Default for SubcallId {
    fn default() -> Self {
        Self::new()
    }
}

// ── Status ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubcallStatus {
    Ok,
    BudgetExceeded {
        limit: String,
        used: usize,
        cap: usize,
    },
    Timeout {
        elapsed_ms: u64,
    },
    ToolBlocked {
        tool: String,
        reason: String,
    },
    InvalidOutput {
        schema_error: String,
    },
    NestedSubcallAttempted,
    SessionCapReached {
        used: usize,
        cap: usize,
    },
    Failed {
        error: String,
    },
}

impl SubcallStatus {
    /// Return the serde `kind` tag as a plain string (e.g. `"ok"`,
    /// `"budget_exceeded"`). Used by callers that want to log status
    /// without serializing the entire enum — avoids `.unwrap()` on the
    /// serde_json::to_value path and keeps the kind-tag vocabulary in
    /// one place.
    pub fn serde_kind_tag(&self) -> Option<String> {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
    }
}

// ── Clamp record ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClampRecord {
    pub field: String,
    pub requested: usize,
    pub applied: usize,
    pub reason: ClampReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClampReason {
    HardCap,
    ParentBudgetRemaining,
}

// ── Origin ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubcallOrigin {
    LlmDirected,
    CodeDirected { site: String },
}

// ── Hard caps (design §4.4, §7.3) ──────────────────────────────────

pub const MAX_ROUNDS_CAP: usize = 10;
pub const MAX_TOOL_CALLS_CAP: usize = 20;
pub const MAX_TOKENS_CAP: usize = 30_000;
// Bumped 5 → 8 on 2026-04-17 so a 7-way deep fanout
// doesn't silently hit the cap and return `SessionCapReached` for its
// 6th/7th fanout subcall. Spec §4.4 hard-cap table lists this as the
// parent-session budget; choose a value strictly greater than any
// documented fanout count.
pub const MAX_SUBCALLS_PER_SESSION: usize = 8;
pub const MAX_CONCURRENT_SUBCALLS: usize = 3;
pub const SUBCALL_TIMEOUT_SECS: u64 = 60;
pub const MIN_USEFUL_TOKENS: usize = 2_000;
pub const DEFAULT_MAX_ROUNDS: usize = 5;
pub const DEFAULT_MAX_TOOL_CALLS: usize = 10;
pub const DEFAULT_MAX_TOKENS: usize = 30_000;

// ── Storage governance (design §7.5) ──────────────────────────────────
//
// Soft cap on the total byte footprint of `subcall_trace`. Enforced by
// `MemoryHandle::subcall_enforce_lru_cap`, invoked daily from the
// governance task spawned in `main.rs`. When the sum of `size_bytes`
// across all trace rows exceeds this cap, the oldest-by-`accessed_at`
// rows are deleted until the total is back under the cap, and the
// corresponding `subcall_log.trace_available` flag is flipped to 0 so
// subsequent recall returns `trace_evicted` instead of a stale read.
pub const LRU_TOTAL_BYTES_CAP: i64 = 500 * 1024 * 1024; // 500 MB

// ── Spec ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SubcallSpec {
    pub task: String,
    pub context_brief: Option<String>,
    pub allowed_tools: Vec<String>,
    pub output_schema: serde_json::Value,
    pub max_rounds: usize,
    pub max_tool_calls: usize,
    pub max_tokens: usize,
    pub parent_session_id: String,
    pub origin: SubcallOrigin,
}

impl SubcallSpec {
    /// Clamp caller-supplied budgets to system hard caps.
    ///
    /// Returns the list of clamps actually applied (empty when everything
    /// was within limits). Mutates `self` in place so callers see the
    /// final effective values.
    pub fn clamp_hard_caps(&mut self) -> Vec<ClampRecord> {
        let mut clamps = Vec::new();

        if self.max_rounds > MAX_ROUNDS_CAP {
            clamps.push(ClampRecord {
                field: "max_rounds".to_string(),
                requested: self.max_rounds,
                applied: MAX_ROUNDS_CAP,
                reason: ClampReason::HardCap,
            });
            self.max_rounds = MAX_ROUNDS_CAP;
        }

        if self.max_tool_calls > MAX_TOOL_CALLS_CAP {
            clamps.push(ClampRecord {
                field: "max_tool_calls".to_string(),
                requested: self.max_tool_calls,
                applied: MAX_TOOL_CALLS_CAP,
                reason: ClampReason::HardCap,
            });
            self.max_tool_calls = MAX_TOOL_CALLS_CAP;
        }

        if self.max_tokens > MAX_TOKENS_CAP {
            clamps.push(ClampRecord {
                field: "max_tokens".to_string(),
                requested: self.max_tokens,
                applied: MAX_TOKENS_CAP,
                reason: ClampReason::HardCap,
            });
            self.max_tokens = MAX_TOKENS_CAP;
        }

        clamps
    }
}

// ── Result ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubcallResult {
    pub id: SubcallId,
    pub structured_output: serde_json::Value,
    pub summary: String,
    pub tool_calls_made: Vec<String>,
    pub tokens_used: usize,
    pub rounds_used: usize,
    pub status: SubcallStatus,
    pub clamped_budgets: Vec<ClampRecord>,
    pub last_assistant_text: Option<String>,
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn base_spec() -> SubcallSpec {
        SubcallSpec {
            task: "test task".to_string(),
            context_brief: None,
            allowed_tools: vec![],
            output_schema: serde_json::json!({}),
            max_rounds: DEFAULT_MAX_ROUNDS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_tokens: DEFAULT_MAX_TOKENS,
            parent_session_id: "sess_test".to_string(),
            origin: SubcallOrigin::LlmDirected,
        }
    }

    #[test]
    fn subcall_spec_clamps_max_rounds_to_hard_cap() {
        let mut spec = base_spec();
        spec.max_rounds = 100;
        let clamps = spec.clamp_hard_caps();
        assert_eq!(spec.max_rounds, MAX_ROUNDS_CAP);
        assert_eq!(clamps.len(), 1);
        assert_eq!(clamps[0].field, "max_rounds");
        assert_eq!(clamps[0].requested, 100);
        assert_eq!(clamps[0].applied, MAX_ROUNDS_CAP);
        assert_eq!(clamps[0].reason, ClampReason::HardCap);
    }

    #[test]
    fn subcall_spec_clamps_max_tokens_to_hard_cap() {
        let mut spec = base_spec();
        spec.max_tokens = 1_000_000;
        let clamps = spec.clamp_hard_caps();
        assert_eq!(spec.max_tokens, MAX_TOKENS_CAP);
        assert!(clamps.iter().any(|c| c.field == "max_tokens"));
    }

    #[test]
    fn subcall_spec_clamps_empty_when_within_caps() {
        let mut spec = base_spec();
        let clamps = spec.clamp_hard_caps();
        assert!(clamps.is_empty(), "No clamps expected when within caps");
    }

    #[test]
    fn subcall_id_format_starts_with_sc_underscore() {
        let id = SubcallId::new();
        assert!(id.as_str().starts_with("sc_"));
    }

    #[test]
    fn subcall_status_serializes_budget_exceeded_with_all_fields() {
        let status = SubcallStatus::BudgetExceeded {
            limit: "max_rounds".to_string(),
            used: 10,
            cap: 10,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["kind"], "budget_exceeded");
        assert_eq!(json["limit"], "max_rounds");
        assert_eq!(json["used"], 10);
        assert_eq!(json["cap"], 10);
    }

    #[test]
    fn subcall_status_serializes_session_cap_reached() {
        let status = SubcallStatus::SessionCapReached { used: 5, cap: 5 };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["kind"], "session_cap_reached");
    }

    #[test]
    fn clamp_reason_serializes_snake_case() {
        let reason = ClampReason::ParentBudgetRemaining;
        let json = serde_json::to_value(reason).unwrap();
        assert_eq!(json, serde_json::json!("parent_budget_remaining"));
    }

    #[test]
    fn serde_kind_tag_returns_variant_name_snake_case() {
        assert_eq!(SubcallStatus::Ok.serde_kind_tag().as_deref(), Some("ok"));
        assert_eq!(
            SubcallStatus::SessionCapReached { used: 5, cap: 5 }
                .serde_kind_tag()
                .as_deref(),
            Some("session_cap_reached")
        );
        assert_eq!(
            SubcallStatus::BudgetExceeded {
                limit: "max_rounds".to_string(),
                used: 10,
                cap: 10
            }
            .serde_kind_tag()
            .as_deref(),
            Some("budget_exceeded")
        );
        assert_eq!(
            SubcallStatus::Failed {
                error: "boom".to_string()
            }
            .serde_kind_tag()
            .as_deref(),
            Some("failed")
        );
    }
}

// ── Trace (in-memory payload tied to a SubcallId) ───────────────────

// Fields `result`, `messages`, `tool_outputs`, `created_at` are populated
// by the runner in Task 1.6 and read by `SubcallRegistry::recall` in
// Task 1.7. Tests exercise them via `make_trace`. Until the readers land,
// the compiler sees them as dead — expected, surfaced via this allow.
#[allow(dead_code)]
pub(crate) struct SubcallTrace {
    pub id: SubcallId,
    pub spec: SubcallSpec,
    pub result: SubcallResult,
    pub messages: Vec<ChatCompletionRequestMessage>,
    pub tool_outputs: Vec<(String, String)>,
    pub created_at: DateTime<Utc>,
}

// ── Recall result shapes (fully defined here; populated by Task 1.7) ──

#[derive(Debug, Clone, Serialize)]
pub struct RecallResult {
    pub subcall_id: SubcallId,
    pub matches: Vec<RecallFragment>,
    pub used_fallback_summary: bool,
    pub trace_evicted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallFragment {
    pub tool_name: String,
    pub text: String,
    pub score: i64,
}

// ── Registry (in-memory; DB fallback in Task 1.7) ───────────────────

pub struct SubcallRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

struct RegistryInner {
    traces: HashMap<SubcallId, SubcallTrace>,
    session_index: HashMap<String, Vec<SubcallId>>,
    // Spec §10 Risk #4 — last activity per session feeds the hourly
    // safety-net sweep. Updated on every `insert()` and on explicit
    // `touch_session()` calls, cleared by `gc_session` / `sweep_stale_sessions`.
    last_seen: HashMap<String, DateTime<Utc>>,
}

impl SubcallRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryInner {
                traces: HashMap::new(),
                session_index: HashMap::new(),
                last_seen: HashMap::new(),
            })),
        }
    }

    // Called by the runner in Task 1.6 after a subcall completes; tests
    // already exercise it. Allow until the runner lands.
    #[allow(dead_code)]
    pub(crate) fn insert(&self, trace: SubcallTrace) {
        let mut g = self.inner.lock().expect("SubcallRegistry lock poisoned");
        let sid = trace.spec.parent_session_id.clone();
        let id = trace.id.clone();
        g.session_index
            .entry(sid.clone())
            .or_default()
            .push(id.clone());
        g.traces.insert(id, trace);
        g.last_seen.insert(sid, Utc::now());
    }

    pub fn session_count(&self, session_id: &str) -> usize {
        let g = self.inner.lock().expect("SubcallRegistry lock poisoned");
        g.session_index
            .get(session_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    pub fn session_ids(&self, session_id: &str) -> Vec<SubcallId> {
        let g = self.inner.lock().expect("SubcallRegistry lock poisoned");
        g.session_index.get(session_id).cloned().unwrap_or_default()
    }

    pub fn gc_session(&self, session_id: &str) {
        let mut g = self.inner.lock().expect("SubcallRegistry lock poisoned");
        if let Some(ids) = g.session_index.remove(session_id) {
            for id in ids {
                g.traces.remove(&id);
            }
        }
        g.last_seen.remove(session_id);
    }

    /// Refresh `last_seen` for a session without inserting a trace. Called at
    /// turn boundaries so the hourly safety-net sweep can distinguish active
    /// sessions from abandoned ones.
    pub fn touch_session(&self, session_id: &str) {
        let mut g = self.inner.lock().expect("SubcallRegistry lock poisoned");
        g.last_seen.insert(session_id.to_string(), Utc::now());
    }

    /// Remove every session whose `last_seen` is older than `threshold` ago.
    /// Returns the number of sessions swept. Wired to the hourly cron in
    /// `main.rs` (spec §10 Risk #4 safety net).
    pub fn sweep_stale_sessions(&self, threshold: chrono::Duration) -> usize {
        let cutoff = Utc::now() - threshold;
        let stale: Vec<String> = {
            let g = self.inner.lock().expect("SubcallRegistry lock poisoned");
            g.last_seen
                .iter()
                .filter(|(_, ts)| **ts < cutoff)
                .map(|(sid, _)| sid.clone())
                .collect()
        };
        for sid in &stale {
            self.gc_session(sid);
        }
        stale.len()
    }
}

/// RAII guard that calls `gc_session` when dropped. Used at the top of
/// `process_message_multimodal` and `run_heartbeat_tick` so every return
/// path (including `?`-propagated errors) releases the session's in-memory
/// traces. The hourly `sweep_stale_sessions` cron is a safety net for the
/// rare path where the guard itself is skipped (panic without unwind, etc).
///
/// Concurrency note: the user-turn `session_id` format is `user:{did}:chat:{id}` —
/// intentionally stable across concurrent turns so the per-session subcall
/// cap (`MAX_SUBCALLS_PER_SESSION`) stays meaningful. Side-effect: two
/// concurrent turns for the same chat share a key, so whichever drops
/// first wipes the other's in-flight traces. The recall() DB fallback
/// mitigates this (evicted-like behavior) rather than breaking.
#[must_use = "SessionGcGuard must be bound to a local to run on drop"]
pub struct SessionGcGuard {
    registry: Arc<SubcallRegistry>,
    session_id: String,
}

impl SessionGcGuard {
    pub fn new(registry: Arc<SubcallRegistry>, session_id: String) -> Self {
        registry.touch_session(&session_id);
        Self {
            registry,
            session_id,
        }
    }
}

impl Drop for SessionGcGuard {
    fn drop(&mut self) {
        self.registry.gc_session(&self.session_id);
    }
}

impl Default for SubcallRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Recall (Task 1.7) ────────────────────────────────────────────────
//
// `recall` lets callers (the parent LLM, future diagnostics) retrieve the
// salient fragments of a completed subcall's tool outputs. Fast path is
// the in-memory registry; on miss we fall back to the `subcall_trace` DB
// row, touch `accessed_at` for LRU ordering, and rank fragments via
// `subcall_recall::rank_fragments`. If the log row exists but the trace
// has been LRU-evicted, we signal `trace_evicted=true` so the caller can
// inform the LLM that no rich fragments are available.

impl SubcallRegistry {
    /// Recall a prior subcall's tool outputs, ranked against `query`.
    ///
    /// * In-memory fast path: look up the trace in the registry. Hit →
    ///   rank + return.
    /// * DB fallback: `memory.subcall_fetch_trace` joins `subcall_log`
    ///   (for `trace_available` + `summary`) with `subcall_trace` (for
    ///   `tool_outputs_json`). If `trace_available=false` → return
    ///   `trace_evicted=true` with empty matches.
    /// * On a DB hit we `subcall_touch_accessed` so the row is counted
    ///   as hot for the next LRU sweep.
    /// * Empty query (after trim) returns the summary as a single
    ///   fallback fragment with `used_fallback_summary=true`.
    pub async fn recall(
        &self,
        subcall_id: &SubcallId,
        query: &str,
        memory: &crate::memory::MemoryHandle,
    ) -> Result<RecallResult> {
        // ── In-memory fast path ──
        let in_memory_trace = {
            let g = self.inner.lock().expect("SubcallRegistry lock poisoned");
            g.traces
                .get(subcall_id)
                .map(|t| (t.result.summary.clone(), t.tool_outputs.clone()))
        };

        if let Some((summary, tool_outputs)) = in_memory_trace {
            return Ok(build_recall_result(
                subcall_id.clone(),
                summary,
                &tool_outputs,
                query,
                /* trace_evicted */ false,
            ));
        }

        // ── DB fallback ──
        let subcall_id_str = subcall_id.as_str().to_string();
        let (exists, trace_available, summary_opt, tool_outputs_opt) =
            memory.subcall_fetch_trace(&subcall_id_str).await?;

        // NotFound: caller passed an id we have no record of (likely LLM
        // hallucination). Surface as an explicit error so the tool-facing
        // handler in Phase 2 can report status="not_found" distinctly
        // from status="trace_evicted".
        if !exists {
            return Err(anyhow::anyhow!(
                "subcall_id not found: {}",
                subcall_id.as_str()
            ));
        }

        if !trace_available {
            // Evicted: the log row survives but raw outputs are gone.
            // Surface the stored summary (per spec §7.2) so the LLM
            // still sees what the subcall concluded.
            let matches = match &summary_opt {
                Some(s) if !s.is_empty() => vec![RecallFragment {
                    tool_name: "summary".to_string(),
                    text: s.clone(),
                    score: 0,
                }],
                _ => {
                    tracing::warn!(
                        subcall_id = subcall_id.as_str(),
                        "subcall_log row predates summary column; evicted recall returns empty matches",
                    );
                    vec![]
                }
            };
            return Ok(RecallResult {
                subcall_id: subcall_id.clone(),
                matches,
                used_fallback_summary: true,
                trace_evicted: true,
            });
        }

        // Update accessed_at for LRU. Log + proceed on failure — a touch
        // miss must not fail the recall itself (we already have the data).
        if let Err(e) = memory.subcall_touch_accessed(&subcall_id_str).await {
            tracing::warn!(
                error = %e,
                subcall_id = subcall_id.as_str(),
                "subcall_touch_accessed failed",
            );
        }

        let summary = summary_opt.unwrap_or_default();
        let tool_outputs = tool_outputs_opt.unwrap_or_default();
        Ok(build_recall_result(
            subcall_id.clone(),
            summary,
            &tool_outputs,
            query,
            /* trace_evicted */ false,
        ))
    }
}

/// Convert a (summary, tool_outputs, query) triple into a `RecallResult`.
/// Factored out so both the in-memory and DB paths share one ranking
/// + fallback policy.
fn build_recall_result(
    subcall_id: SubcallId,
    summary: String,
    tool_outputs: &[(String, String)],
    query: &str,
    trace_evicted: bool,
) -> RecallResult {
    let query_trimmed = query.trim();
    if query_trimmed.is_empty() {
        return RecallResult {
            subcall_id,
            matches: vec![RecallFragment {
                tool_name: "summary".to_string(),
                text: summary,
                score: 0,
            }],
            used_fallback_summary: true,
            trace_evicted,
        };
    }

    let ranked = crate::subcall_recall::rank_fragments(query_trimmed, tool_outputs);
    if ranked.is_empty() {
        // No scored fragment — fall back to the summary so the LLM sees
        // something meaningful rather than an empty recall.
        return RecallResult {
            subcall_id,
            matches: vec![RecallFragment {
                tool_name: "summary".to_string(),
                text: summary,
                score: 0,
            }],
            used_fallback_summary: true,
            trace_evicted,
        };
    }

    RecallResult {
        subcall_id,
        matches: ranked
            .into_iter()
            .map(|f| RecallFragment {
                tool_name: f.tool_name,
                text: f.text,
                score: f.score,
            })
            .collect(),
        used_fallback_summary: false,
        trace_evicted,
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    fn make_trace(session: &str) -> SubcallTrace {
        let spec = SubcallSpec {
            task: "t".to_string(),
            context_brief: None,
            allowed_tools: vec![],
            output_schema: serde_json::json!({}),
            max_rounds: 5,
            max_tool_calls: 10,
            max_tokens: 30_000,
            parent_session_id: session.to_string(),
            origin: SubcallOrigin::LlmDirected,
        };
        let id = SubcallId::new();
        let result = SubcallResult {
            id: id.clone(),
            structured_output: serde_json::json!({}),
            summary: String::new(),
            tool_calls_made: vec![],
            tokens_used: 0,
            rounds_used: 0,
            status: SubcallStatus::Ok,
            clamped_budgets: vec![],
            last_assistant_text: None,
        };
        SubcallTrace {
            id,
            spec,
            result,
            messages: vec![],
            tool_outputs: vec![],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn registry_insert_then_count() {
        let reg = SubcallRegistry::new();
        reg.insert(make_trace("sess_a"));
        reg.insert(make_trace("sess_a"));
        reg.insert(make_trace("sess_b"));
        assert_eq!(reg.session_count("sess_a"), 2);
        assert_eq!(reg.session_count("sess_b"), 1);
        assert_eq!(reg.session_count("sess_missing"), 0);
    }

    #[test]
    fn registry_gc_session_removes_only_that_session() {
        let reg = SubcallRegistry::new();
        reg.insert(make_trace("sess_a"));
        reg.insert(make_trace("sess_b"));
        reg.gc_session("sess_a");
        assert_eq!(reg.session_count("sess_a"), 0);
        assert_eq!(reg.session_count("sess_b"), 1);
    }

    #[test]
    fn registry_session_ids_returns_all_ids() {
        let reg = SubcallRegistry::new();
        reg.insert(make_trace("sess_a"));
        reg.insert(make_trace("sess_a"));
        let ids = reg.session_ids("sess_a");
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn sweep_stale_sessions_removes_only_old_ones() {
        let reg = SubcallRegistry::new();
        reg.insert(make_trace("sess_fresh"));
        reg.insert(make_trace("sess_old"));

        // Manually backdate sess_old's last_seen past the threshold. The
        // public API never sets `last_seen` to the past; this direct mutation
        // simulates a long-idle session for the test.
        {
            let mut g = reg.inner.lock().unwrap();
            g.last_seen.insert(
                "sess_old".to_string(),
                Utc::now() - chrono::Duration::hours(2),
            );
        }

        let swept = reg.sweep_stale_sessions(chrono::Duration::hours(1));
        assert_eq!(swept, 1, "only sess_old should be swept");
        assert_eq!(reg.session_count("sess_fresh"), 1);
        assert_eq!(reg.session_count("sess_old"), 0);
    }

    #[test]
    fn session_gc_guard_drops_cleanup() {
        let reg = Arc::new(SubcallRegistry::new());
        reg.insert(make_trace("sess_guarded"));
        assert_eq!(reg.session_count("sess_guarded"), 1);
        {
            let _guard = SessionGcGuard::new(reg.clone(), "sess_guarded".to_string());
            // Guard touches last_seen on construction; session still present.
            assert_eq!(reg.session_count("sess_guarded"), 1);
        }
        // On drop, guard calls gc_session — session cleared.
        assert_eq!(reg.session_count("sess_guarded"), 0);
    }

    // ── Recall tests (Task 1.7) ─────────────────────────────────────────
    //
    // The three cases per the design:
    //   1. in-memory hit with a non-empty query → rank_fragments runs and
    //      returns at least one scored fragment.
    //   2. empty query → fallback summary path fires (`used_fallback_summary`).
    //   3. log row with `trace_available=0` (LRU-evicted) → recall reports
    //      `trace_evicted=true` with empty matches.

    #[tokio::test]
    async fn recall_in_memory_hit_returns_matches() {
        let reg = SubcallRegistry::new();
        let mut t = make_trace("sess_x");
        t.tool_outputs = vec![(
            "web_search".to_string(),
            "the auth middleware lives at /etc/auth/middleware.conf".to_string(),
        )];
        let id = t.id.clone();
        reg.insert(t);

        let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let result = reg.recall(&id, "auth middleware", &memory).await.unwrap();
        assert!(!result.matches.is_empty(), "expected ranked matches");
        assert!(!result.trace_evicted);
        assert!(!result.used_fallback_summary);
        // Ranking should have surfaced the tool_name we inserted.
        assert_eq!(result.matches[0].tool_name, "web_search");
    }

    #[tokio::test]
    async fn recall_empty_query_returns_fallback_summary() {
        let reg = SubcallRegistry::new();
        let mut t = make_trace("sess_x");
        t.result.summary = "found three candidate approaches".to_string();
        let id = t.id.clone();
        reg.insert(t);

        let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let result = reg.recall(&id, "", &memory).await.unwrap();
        assert!(result.used_fallback_summary);
        assert!(!result.trace_evicted);
        // Empty-query path returns exactly one summary fragment with score 0.
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].tool_name, "summary");
        assert_eq!(result.matches[0].score, 0);
        assert_eq!(result.matches[0].text, "found three candidate approaches");
    }

    #[tokio::test]
    async fn recall_db_fallback_when_in_memory_miss() {
        // Register NOTHING in-memory — the recall must traverse the DB.
        let reg = SubcallRegistry::new();
        let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();

        // Insert a subcall_log row with trace_available=0 to simulate
        // an LRU-evicted trace. Re-use the public helper so the column
        // list stays in sync.
        let id = SubcallId::new();
        let now_iso = Utc::now().to_rfc3339();
        memory
            .subcall_log_insert(
                id.as_str(),
                "sess_evicted",
                "llm_directed",
                Some("task preview"),
                &[],
                "ok",
                Some(1),
                Some(0),
                Some(0),
                Some(0),
                &now_iso,
                None,
                /* trace_available */ false,
                Some("evicted summary"),
            )
            .await
            .unwrap();

        let result = reg.recall(&id, "anything", &memory).await.unwrap();
        assert!(result.trace_evicted, "expected trace_evicted=true");
        assert!(result.used_fallback_summary);
        // Spec §7.2: evicted recall must surface the stored summary so the
        // LLM still sees what the subcall concluded. `matches[0]` is the
        // summary fragment (tool_name="summary", score=0).
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].tool_name, "summary");
        assert_eq!(result.matches[0].text, "evicted summary");
        assert_eq!(result.matches[0].score, 0);
    }

    #[tokio::test]
    async fn recall_unknown_id_returns_not_found_error() {
        // Recall on an id that has NO subcall_log row (LLM hallucinated it).
        // Must produce a distinct error — not a silent trace_evicted=true,
        // which would conflate "id invalid" with "trace LRU-evicted".
        let reg = SubcallRegistry::new();
        let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let bogus = SubcallId::new();

        let result = reg.recall(&bogus, "anything", &memory).await;
        let err = result.expect_err("unknown id must surface as Err, not trace_evicted");
        let msg = err.to_string();
        assert!(
            msg.contains("not found"),
            "error must mention 'not found', got: {}",
            msg
        );
        assert!(
            msg.contains(bogus.as_str()),
            "error must include the bogus id for traceability, got: {}",
            msg
        );
    }
}

// ── Runner (Task 1.6 — full loop wiring) ──
//
// `run_subcall` is the public entry point. Task 1.5 shipped the pre-check
// paths (parent budget, session cap); Task 1.6 (this module) completes the
// path: spec → system prompt → bounded ReAct loop → `final_answer`
// extraction → schema validation → DB persist → registry insert.
//
// Transparency invariant (design §3, §4.3): every failure mode returns a
// typed `SubcallStatus` — never a silent fallthrough.

use std::time::Duration;

use anyhow::Result;
use async_openai::types::chat::ChatCompletionTools;

use crate::memory::MemoryHandle;
use crate::orchestrator::Orchestrator;

/// Result of a parent-budget check before spawning a subcall.
#[allow(dead_code)] // consumed by `run_subcall`; fully live once Task 1.6 wires callers.
enum ParentBudgetCheck {
    Ok {
        effective_max_tokens: usize,
        clamps: Vec<ClampRecord>,
    },
    InsufficientBudget {
        used: usize,
        cap: usize,
    },
}

#[allow(dead_code)] // exercised by `budget_tests` + `run_subcall`; callers wired in Task 1.6.
fn check_parent_budget(
    requested_max_tokens: usize,
    parent_remaining: usize,
    parent_cap: usize,
) -> ParentBudgetCheck {
    if parent_remaining < MIN_USEFUL_TOKENS {
        return ParentBudgetCheck::InsufficientBudget {
            used: parent_cap.saturating_sub(parent_remaining),
            cap: parent_cap,
        };
    }
    if parent_remaining < requested_max_tokens {
        return ParentBudgetCheck::Ok {
            effective_max_tokens: parent_remaining,
            clamps: vec![ClampRecord {
                field: "max_tokens".to_string(),
                requested: requested_max_tokens,
                applied: parent_remaining,
                reason: ClampReason::ParentBudgetRemaining,
            }],
        };
    }
    ParentBudgetCheck::Ok {
        effective_max_tokens: requested_max_tokens,
        clamps: vec![],
    }
}

/// Public API: run a subcall with a parent session context.
///
/// `parent_remaining_tokens` / `parent_token_cap` let the runner clamp
/// subcall token budget to fit parent budget (surfaces via
/// `ClampReason::ParentBudgetRemaining`) or reject early when parent is
/// near exhaustion (`SubcallStatus::BudgetExceeded{limit:"parent_remaining_tokens"}`).
///
/// Flow (Task 1.6):
///   1. Clamp spec to hard caps.
///   2. Pre-check parent budget → possibly reject or clamp.
///   3. Pre-check session cap → possibly reject with existing ids.
///   4. Build system prompt + filtered tool list + `final_answer` sentinel.
///   5. Run `Orchestrator::run_autonomous_react_loop` with a 60s timeout.
///   6. Extract `final_answer_args`; fall back to `last_assistant_text`.
///   7. Validate top-level required keys in `spec.output_schema`.
///   8. Persist one row to `subcall_log` + one to `subcall_trace`.
///   9. Insert the in-memory trace into `registry`.
///
/// Every failure mode — schema violation, timeout, loop error — returns
/// a typed `SubcallStatus`. No silent fallthroughs.
#[allow(dead_code)] // wired to the spawn_subcall tool in a later phase.
pub async fn run_subcall(
    mut spec: SubcallSpec,
    orchestrator: &Orchestrator,
    memory: &MemoryHandle,
    registry: &SubcallRegistry,
    parent_remaining_tokens: usize,
    parent_token_cap: usize,
) -> Result<SubcallResult> {
    let subcall_id = SubcallId::new();
    let mut clamps = spec.clamp_hard_caps();

    // ── Pre-check 1: parent budget ──────────────────────────────────
    match check_parent_budget(spec.max_tokens, parent_remaining_tokens, parent_token_cap) {
        ParentBudgetCheck::Ok {
            effective_max_tokens,
            clamps: pc,
        } => {
            spec.max_tokens = effective_max_tokens;
            clamps.extend(pc);
        }
        ParentBudgetCheck::InsufficientBudget { used, cap } => {
            return Ok(SubcallResult {
                id: subcall_id,
                structured_output: serde_json::json!({}),
                summary: format!(
                    "Subcall rejected — parent session has {} tokens remaining, below MIN_USEFUL_TOKENS ({}).",
                    parent_token_cap.saturating_sub(used),
                    MIN_USEFUL_TOKENS,
                ),
                tool_calls_made: vec![],
                tokens_used: 0,
                rounds_used: 0,
                status: SubcallStatus::BudgetExceeded {
                    limit: "parent_remaining_tokens".to_string(),
                    used,
                    cap,
                },
                clamped_budgets: clamps,
                last_assistant_text: None,
            });
        }
    }

    // ── Pre-check 2: session cap ────────────────────────────────────
    let session_count = registry.session_count(&spec.parent_session_id);
    if session_count >= MAX_SUBCALLS_PER_SESSION {
        let existing_ids = registry.session_ids(&spec.parent_session_id);
        return Ok(SubcallResult {
            id: subcall_id,
            structured_output: serde_json::json!({
                "existing_subcall_ids": existing_ids.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
            }),
            summary: format!(
                "Session cap reached: {} of {} subcalls used.",
                session_count, MAX_SUBCALLS_PER_SESSION,
            ),
            tool_calls_made: vec![],
            tokens_used: 0,
            rounds_used: 0,
            status: SubcallStatus::SessionCapReached {
                used: session_count,
                cap: MAX_SUBCALLS_PER_SESSION,
            },
            clamped_budgets: clamps,
            last_assistant_text: None,
        });
    }

    // ── Build prompts + tool list ───────────────────────────────────
    let system_prompt = build_subcall_system_prompt(&spec);
    let user_msg = "Begin executing the task.".to_string();

    // Filter orchestrator tools to the spec's allowlist and append the
    // `final_answer` sentinel. If a requested tool isn't registered in
    // the orchestrator's top-level schema list, it's silently dropped
    // here. At run time a call to any tool outside `allowed_tools` is
    // blocked per call by `ToolGate::evaluate_autonomous` (Layer 2
    // allowlist): the block reason is fed back to the model as the tool
    // result and the loop continues, so the runner never produces
    // `SubcallStatus::ToolBlocked` for it.
    let tools = build_subcall_tool_list(orchestrator, &spec.allowed_tools);

    // Cost attribution label (design §7.4): flows through into every
    // `log_llm_usage(..., &label, ...)` inside the loop.
    let origin_tag = match &spec.origin {
        SubcallOrigin::LlmDirected => "llm_directed".to_string(),
        SubcallOrigin::CodeDirected { site } => format!("code:{site}"),
    };
    let label = format!("subcall:{}:{}", origin_tag, subcall_id.as_str());

    let config = crate::chronos::AutonomousConfig {
        enabled: true,
        max_rounds: spec.max_rounds,
        max_tool_calls: spec.max_tool_calls,
        allowed_tools: spec.allowed_tools.clone(),
        denied_tools: Vec::new(),
        daily_token_budget: 0, // unused on the subcall code path
        notification_mode: crate::chronos::NotificationMode::Never,
    };

    // ── Run bounded ReAct loop with timeout ─────────────────────────
    let started = std::time::Instant::now();
    let deadline = Duration::from_secs(SUBCALL_TIMEOUT_SECS);

    let user_did = "subcall"; // no per-user attribution at this layer
    let loop_future = orchestrator.run_autonomous_react_loop(
        system_prompt.clone(),
        user_msg.clone(),
        &config,
        tools,
        &label,
        "Info",
        None,
        user_did,
        None,
        None,
        // Task 2.6: subcalls must surface as `CallerContext::Subcall` so
        // InteractiveOnly / HeartbeatAllowed / ApprovalInCallback /
        // RequireApproval tools are Blocked by the gate (Task 2.1).
        // Forward the parent's `(remaining, cap)` so any nested
        // spawn_subcall inside this loop can clamp against the same
        // budget envelope the parent is operating under. Phase 1.6b:
        // the legacy `suppress_heartbeat_audit: bool` param was folded
        // into this enum — `Subcall` now implies audit-skip + transcript
        // capture inside the loop (spec §3 Risk #3 + §7.5).
        crate::tool_gate::CallerContext::Subcall,
        (parent_remaining_tokens, parent_token_cap),
    );

    let outcome = match tokio::time::timeout(deadline, loop_future).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let result = SubcallResult {
                id: subcall_id.clone(),
                structured_output: serde_json::json!({}),
                summary: format!("Subcall loop errored: {e}"),
                tool_calls_made: vec![],
                tokens_used: 0,
                rounds_used: 0,
                status: SubcallStatus::Failed {
                    error: e.to_string(),
                },
                clamped_budgets: clamps,
                last_assistant_text: None,
            };
            persist_subcall(
                memory,
                registry,
                spec,
                &result,
                &system_prompt,
                Vec::new(),
                Vec::new(),
                elapsed_ms,
            )
            .await;
            return Ok(result);
        }
        Err(_elapsed) => {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let result = SubcallResult {
                id: subcall_id.clone(),
                structured_output: serde_json::json!({}),
                summary: format!("Subcall timed out after {elapsed_ms}ms."),
                tool_calls_made: vec![],
                tokens_used: 0,
                rounds_used: 0,
                status: SubcallStatus::Timeout { elapsed_ms },
                clamped_budgets: clamps,
                last_assistant_text: None,
            };
            persist_subcall(
                memory,
                registry,
                spec,
                &result,
                &system_prompt,
                Vec::new(),
                Vec::new(),
                elapsed_ms,
            )
            .await;
            return Ok(result);
        }
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;

    // ── Extract structured output ───────────────────────────────────
    let last_assistant_text = if outcome.reply.is_empty() {
        None
    } else {
        Some(outcome.reply.clone())
    };

    let (structured_output, summary_from_final, status) = match &outcome.final_answer_args {
        Some(args) => match validate_output_schema(&spec.output_schema, args) {
            Ok(()) => {
                let summary = args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let structured = args
                    .get("structured_output")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                (structured, summary, SubcallStatus::Ok)
            }
            Err(schema_error) => {
                let structured = args
                    .get("structured_output")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let summary = args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (
                    structured,
                    summary,
                    SubcallStatus::InvalidOutput { schema_error },
                )
            }
        },
        // Spec §7.2: when `final_answer` is missing AND the loop ended
        // because a budget cap was hit, attribute the failure to
        // `BudgetExceeded{limit}` rather than collapsing into
        // InvalidOutput. The loop surfaces `budget_exhausted=true` only
        // on tool-call cap; the round cap is detectable by comparing
        // `rounds_used` to `spec.max_rounds`. Both are checked here so
        // whichever cap fired wins.
        None => {
            let round_cap_hit = outcome.rounds_used >= spec.max_rounds && spec.max_rounds > 0;
            if outcome.budget_exhausted || round_cap_hit {
                let (limit, used, cap) = if round_cap_hit {
                    (
                        "max_rounds".to_string(),
                        outcome.rounds_used,
                        spec.max_rounds,
                    )
                } else {
                    (
                        "max_tool_calls".to_string(),
                        outcome.tools_called.len(),
                        spec.max_tool_calls,
                    )
                };
                tracing::info!(
                    subcall_id = %subcall_id.as_str(),
                    %limit,
                    used,
                    cap,
                    "subcall hit budget cap"
                );
                // NOTE: spec §7.2 example shows `partial_output` and `hint`
                // fields on the LLM-facing wire format. We return empty
                // `structured_output: {}` here — the Phase 2 tool-facing
                // renderer (`orchestrator_tools_subcall.rs`, Task 2.3) is
                // responsible for enriching the JSON sent to the parent LLM
                // with a hint like "retry with smaller scope" and a
                // best-effort partial_output from `last_assistant_text`.
                (
                    serde_json::json!({}),
                    String::new(),
                    SubcallStatus::BudgetExceeded { limit, used, cap },
                )
            } else {
                (
                    serde_json::json!({}),
                    String::new(),
                    SubcallStatus::InvalidOutput {
                        schema_error: "final_answer was never called".to_string(),
                    },
                )
            }
        }
    };

    let summary_effective = if summary_from_final.is_empty() {
        last_assistant_text
            .clone()
            .unwrap_or_else(|| "Subcall completed without summary.".to_string())
    } else {
        summary_from_final
    };

    let result = SubcallResult {
        id: subcall_id.clone(),
        structured_output,
        summary: summary_effective,
        tool_calls_made: outcome.tools_called.clone(),
        // Phase 1.6b: real token accounting — `run_autonomous_react_loop`
        // now accumulates `response.usage.total_tokens` across every LLM
        // call in the loop and surfaces the sum on `OrchestratorResult`.
        // Persisted to `subcall_log.tokens_used` via `persist_subcall` and
        // used by callers to credit an engine-level daily budget
        // with real consumption (spec §7.4 label threading closed-loop).
        tokens_used: outcome.total_tokens_used as usize,
        rounds_used: outcome.rounds_used,
        status,
        clamped_budgets: clamps,
        last_assistant_text,
    };

    // Spec §7.5: persist the full transcript + per-tool raw bodies so
    // the Task 1.7 recall path can score fragments without re-running
    // the LLM. `outcome.messages` / `outcome.tool_outputs` are populated
    // by `run_autonomous_react_loop` (Task 1.6 follow-up).
    persist_subcall(
        memory,
        registry,
        spec,
        &result,
        &system_prompt,
        outcome.messages,
        outcome.tool_outputs,
        elapsed_ms,
    )
    .await;

    Ok(result)
}

/// Construct the subcall's system prompt (design §4.3). Succinct:
///   1. Task statement.
///   2. Optional context brief from the caller.
///   3. Explicit list of permitted tools (plus the `final_answer` sentinel).
///   4. Required output shape (top-level keys).
///   5. Reminder to call `final_answer` exactly once when done.
///
/// The LLM runs in a *fresh* context — no parent history, no drift from
/// global system instructions. The prompt is the whole contract.
#[allow(dead_code)] // exercised by tests + `run_subcall` (fully live once callers wired).
pub(crate) fn build_subcall_system_prompt(spec: &SubcallSpec) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are a focused sub-agent. Complete ONE task and return ");
    prompt.push_str("structured output via the `final_answer` tool.\n\n");

    prompt.push_str("## Task\n");
    prompt.push_str(spec.task.trim());
    prompt.push_str("\n\n");

    if let Some(brief) = spec.context_brief.as_ref().filter(|s| !s.trim().is_empty()) {
        prompt.push_str("## Context\n");
        prompt.push_str(brief.trim());
        prompt.push_str("\n\n");
    }

    prompt.push_str("## Allowed tools\n");
    if spec.allowed_tools.is_empty() {
        prompt.push_str("- final_answer (no other tools available)\n");
    } else {
        for tool in &spec.allowed_tools {
            prompt.push_str("- ");
            prompt.push_str(tool);
            prompt.push('\n');
        }
        prompt.push_str("- final_answer\n");
    }
    prompt.push('\n');

    prompt.push_str("## Required output shape\n");
    if let Some(obj) = spec.output_schema.as_object() {
        if obj.is_empty() {
            prompt.push_str("Any JSON object is acceptable in `structured_output`.\n");
        } else {
            prompt
                .push_str("`structured_output` must be a JSON object with these top-level keys:\n");
            for key in obj.keys() {
                prompt.push_str("- ");
                prompt.push_str(key);
                prompt.push('\n');
            }
        }
    } else {
        prompt.push_str("Any JSON value is acceptable in `structured_output`.\n");
    }
    prompt.push('\n');

    prompt.push_str(
        "When you have a complete answer, call `final_answer(structured_output=..., \
         summary=\"...\")` exactly once. Do not call it before you are done; do not \
         call it more than once.",
    );

    prompt
}

/// Build the tool schema list the subcall's LLM sees.
///
/// Filters the orchestrator's full tool-definition set to the
/// `allowed_tools` allowlist, then always appends `final_answer`. The
/// ToolGate (Phase 2) is the authoritative runtime enforcer — this is a
/// schema-visibility filter only.
fn build_subcall_tool_list(
    orchestrator: &Orchestrator,
    allowed_tools: &[String],
) -> Vec<ChatCompletionTools> {
    let all = orchestrator.build_tool_definitions_with_full_schemas();
    let mut filtered: Vec<ChatCompletionTools> = all
        .into_iter()
        .filter(|t| match t {
            async_openai::types::chat::ChatCompletionTools::Function(f) => {
                allowed_tools.iter().any(|a| a == &f.function.name)
            }
            _ => false,
        })
        .collect();
    if let Ok(fa) = crate::tool_schemas::build_final_answer_tool() {
        filtered.push(fa);
    }
    filtered
}

/// Permissive schema validation (design §4.3): check that every top-level
/// key in the declared `output_schema` is present in the returned
/// `structured_output` sub-object. Types and nested shapes are NOT
/// enforced — that is the LLM's responsibility; stricter validation is a
/// later concern.
///
/// Returns `Ok(())` on success, `Err(error_message)` on the first missing
/// key.
pub(crate) fn validate_output_schema(
    declared_schema: &serde_json::Value,
    final_answer_args: &serde_json::Value,
) -> std::result::Result<(), String> {
    let Some(obj) = declared_schema.as_object() else {
        // Non-object schema: nothing to enforce.
        return Ok(());
    };
    if obj.is_empty() {
        return Ok(());
    }
    let Some(structured) = final_answer_args.get("structured_output") else {
        return Err("missing `structured_output` in final_answer".to_string());
    };
    let Some(structured_obj) = structured.as_object() else {
        return Err("`structured_output` must be a JSON object".to_string());
    };
    for key in obj.keys() {
        if !structured_obj.contains_key(key) {
            return Err(format!("missing required field: {key}"));
        }
    }
    Ok(())
}

/// Best-effort persistence of a completed subcall: writes a `subcall_log`
/// row, writes a `subcall_trace` row (with 500 KB cap per design §7.5),
/// and inserts the in-memory trace into the registry. Any DB error is
/// logged but does NOT propagate — the result is already computed and
/// returning `Err` here would lose the result.
#[allow(clippy::too_many_arguments)]
async fn persist_subcall(
    memory: &MemoryHandle,
    registry: &SubcallRegistry,
    spec: SubcallSpec,
    result: &SubcallResult,
    system_prompt: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    tool_outputs: Vec<(String, String)>,
    duration_ms: u64,
) {
    let now_iso = Utc::now().to_rfc3339();
    let status_str = serde_json::to_value(&result.status)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".to_string());
    let origin_str = match &spec.origin {
        SubcallOrigin::LlmDirected => "llm_directed".to_string(),
        SubcallOrigin::CodeDirected { site } => format!("code:{site}"),
    };
    let task_preview: String = spec.task.chars().take(200).collect();

    let summary_for_db = if result.summary.is_empty() {
        None
    } else {
        Some(result.summary.as_str())
    };
    if let Err(e) = memory
        .subcall_log_insert(
            result.id.as_str(),
            &spec.parent_session_id,
            &origin_str,
            Some(&task_preview),
            &spec.allowed_tools,
            &status_str,
            Some(result.rounds_used as i32),
            Some(result.tool_calls_made.len() as i32),
            Some(result.tokens_used as i32),
            Some(duration_ms as i64),
            &now_iso,
            None,
            true,
            summary_for_db,
        )
        .await
    {
        tracing::warn!(error = %e, subcall_id = result.id.as_str(), "subcall_log_insert failed");
    }

    let messages_json = serde_json::to_string(&messages).unwrap_or_else(|_| "[]".into());
    let tool_outputs_json = truncate_tool_outputs_json(&tool_outputs, 500 * 1024);
    let size_bytes = (messages_json.len() + tool_outputs_json.len()) as i64;

    if let Err(e) = memory
        .subcall_trace_insert(
            result.id.as_str(),
            Some(system_prompt),
            &messages_json,
            &tool_outputs_json,
            &now_iso,
            &now_iso,
            size_bytes,
        )
        .await
    {
        tracing::warn!(error = %e, subcall_id = result.id.as_str(), "subcall_trace_insert failed");
    }

    let trace = SubcallTrace {
        id: result.id.clone(),
        spec,
        result: result.clone(),
        messages,
        tool_outputs,
        created_at: Utc::now(),
    };
    registry.insert(trace);
}

/// Serialize tool outputs as JSON, applying the 500 KB cap from design §7.5.
/// If the serialized payload exceeds `cap_bytes`, each individual output is
/// truncated to 50 KB with a sentinel object recording original size. This
/// is a coarse first pass — fine-grained per-tool budgeting lands with
/// Task 1.7's recall path.
fn truncate_tool_outputs_json(tool_outputs: &[(String, String)], cap_bytes: usize) -> String {
    let full = serde_json::to_string(tool_outputs).unwrap_or_else(|_| "[]".into());
    if full.len() <= cap_bytes {
        return full;
    }
    let truncated: Vec<serde_json::Value> = tool_outputs
        .iter()
        .map(|(name, raw)| {
            let kept: String = raw.chars().take(50_000).collect();
            serde_json::json!({
                "tool": name,
                "raw_output": kept,
                "truncated": true,
                "original_bytes": raw.len(),
            })
        })
        .collect();
    serde_json::to_string(&truncated).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn parent_budget_below_min_useful_rejects() {
        match check_parent_budget(30_000, 1_500, 100_000) {
            ParentBudgetCheck::InsufficientBudget { used, cap } => {
                assert_eq!(used, 98_500);
                assert_eq!(cap, 100_000);
            }
            _ => panic!("expected InsufficientBudget"),
        }
    }

    #[test]
    fn parent_budget_clamps_when_remaining_below_requested() {
        match check_parent_budget(30_000, 10_000, 100_000) {
            ParentBudgetCheck::Ok {
                effective_max_tokens,
                clamps,
            } => {
                assert_eq!(effective_max_tokens, 10_000);
                assert_eq!(clamps.len(), 1);
                assert_eq!(clamps[0].reason, ClampReason::ParentBudgetRemaining);
                assert_eq!(clamps[0].requested, 30_000);
                assert_eq!(clamps[0].applied, 10_000);
            }
            _ => panic!("expected Ok with clamp"),
        }
    }

    #[test]
    fn parent_budget_no_clamp_when_ample() {
        match check_parent_budget(30_000, 80_000, 100_000) {
            ParentBudgetCheck::Ok {
                effective_max_tokens,
                clamps,
            } => {
                assert_eq!(effective_max_tokens, 30_000);
                assert!(clamps.is_empty());
            }
            _ => panic!("expected Ok with no clamp"),
        }
    }
}

// ── Runner-adjacent unit tests (Task 1.6) ───────────────────────────
//
// These exercise the pure helpers (`build_subcall_system_prompt`,
// `validate_output_schema`, `truncate_tool_outputs_json`). End-to-end
// coverage of `run_subcall` (real `Orchestrator` against the shared mock
// OpenAI-compatible server) lives in `core/tests/subcall_integration.rs`.

#[cfg(test)]
mod runner_tests {
    use super::*;

    fn sample_spec() -> SubcallSpec {
        SubcallSpec {
            task: "Research the capital of France.".to_string(),
            context_brief: Some("Parent is planning a Paris trip.".to_string()),
            allowed_tools: vec!["web_search".to_string()],
            output_schema: serde_json::json!({"city": {}, "confidence": {}}),
            max_rounds: DEFAULT_MAX_ROUNDS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_tokens: DEFAULT_MAX_TOKENS,
            parent_session_id: "sess_rt".to_string(),
            origin: SubcallOrigin::LlmDirected,
        }
    }

    #[test]
    fn system_prompt_includes_task_tools_and_schema_keys() {
        let spec = sample_spec();
        let prompt = build_subcall_system_prompt(&spec);
        assert!(prompt.contains("Research the capital of France"));
        assert!(prompt.contains("Parent is planning a Paris trip"));
        assert!(prompt.contains("web_search"));
        assert!(prompt.contains("final_answer"));
        // Top-level schema keys are surfaced.
        assert!(prompt.contains("- city"));
        assert!(prompt.contains("- confidence"));
    }

    #[test]
    fn system_prompt_handles_empty_allowed_tools() {
        let mut spec = sample_spec();
        spec.allowed_tools.clear();
        let prompt = build_subcall_system_prompt(&spec);
        assert!(prompt.contains("no other tools available"));
        assert!(prompt.contains("final_answer"));
    }

    #[test]
    fn validate_output_schema_passes_when_all_keys_present() {
        let schema = serde_json::json!({"a": {}, "b": {}});
        let args = serde_json::json!({
            "structured_output": {"a": 1, "b": "two", "extra": true},
            "summary": "done",
        });
        assert!(validate_output_schema(&schema, &args).is_ok());
    }

    #[test]
    fn validate_output_schema_reports_missing_field() {
        let schema = serde_json::json!({"a": {}, "b": {}});
        let args = serde_json::json!({
            "structured_output": {"a": 1},
            "summary": "partial",
        });
        let err = validate_output_schema(&schema, &args).unwrap_err();
        assert!(
            err.contains("missing required field: b"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_output_schema_reports_missing_structured_output() {
        let schema = serde_json::json!({"a": {}});
        let args = serde_json::json!({"summary": "oops"});
        let err = validate_output_schema(&schema, &args).unwrap_err();
        assert!(
            err.contains("missing `structured_output`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_output_schema_permissive_on_empty_or_non_object_schema() {
        let empty = serde_json::json!({});
        let non_obj = serde_json::json!("just a string");
        let args = serde_json::json!({"structured_output": {}, "summary": ""});
        assert!(validate_output_schema(&empty, &args).is_ok());
        assert!(validate_output_schema(&non_obj, &args).is_ok());
    }

    #[test]
    fn budget_exceeded_status_wire_shape_for_max_rounds() {
        // Mirror what `run_subcall` produces when the loop hit the round
        // cap without calling `final_answer` (spec §7.2). If this shape
        // changes, callers parsing the status JSON will break.
        let status = SubcallStatus::BudgetExceeded {
            limit: "max_rounds".to_string(),
            used: 5,
            cap: 5,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["kind"], "budget_exceeded");
        assert_eq!(v["limit"], "max_rounds");
        assert_eq!(v["used"], 5);
        assert_eq!(v["cap"], 5);
    }

    #[test]
    fn budget_exceeded_status_wire_shape_for_max_tool_calls() {
        let status = SubcallStatus::BudgetExceeded {
            limit: "max_tool_calls".to_string(),
            used: 10,
            cap: 10,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["kind"], "budget_exceeded");
        assert_eq!(v["limit"], "max_tool_calls");
    }

    #[test]
    fn truncate_tool_outputs_json_noop_when_under_cap() {
        let outputs = vec![("t".to_string(), "short".to_string())];
        let s = truncate_tool_outputs_json(&outputs, 500 * 1024);
        // Standard array-of-tuples serialization.
        assert!(s.contains("short"));
        assert!(s.starts_with('['));
    }

    #[test]
    fn truncate_tool_outputs_json_shrinks_when_over_cap() {
        // 60 KB payload forces truncation when cap is 10 KB.
        let big = "x".repeat(60_000);
        let outputs = vec![("tool_a".to_string(), big)];
        let s = truncate_tool_outputs_json(&outputs, 10_000);
        assert!(s.contains("truncated"));
        assert!(s.contains("original_bytes"));
        // Truncated variant keeps at most 50 K chars of raw.
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let raw_len = parsed
            .as_array()
            .and_then(|a| a.first())
            .and_then(|o| o.get("raw_output"))
            .and_then(|v| v.as_str())
            .map(|s| s.len())
            .unwrap();
        assert!(raw_len <= 50_000, "raw_output not truncated: {raw_len}");
    }
}
