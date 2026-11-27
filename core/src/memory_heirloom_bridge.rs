//! HeirloomBridge: shim layer between `MemoryHandle` and `HeirloomWriter`.
//!
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §3 Q3.9 + §6 Q6.4.
//! Phase E Dual-Write. Legacy SQLite writes fire FIRST; the bridge then
//! tries to append a corresponding heirloom event. Errors are soft-failed
//! via `tracing::warn!` (rate-limited via `DivergenceCounter`).

use crate::heirloom::writer::HeirloomWriter;
use crate::heirloom::{
    EventEnvelope, HlcTimestamp, LineageId, Signature,
    enums::{EventTypeTag, PayloadEncoding, SensitivityTier},
    types::Did,
};
use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

pub(crate) const SHIM_LINEAGE: &str = "lineage:aegis:memory-shim";

/// Experimental runtime opt-in. Unset, `off`, and unrecognized values stay off.
pub fn experimental_enabled() -> bool {
    explicit_opt_in(
        std::env::var("AEGIS_RUNTIME_HEIRLOOM_BRIDGE")
            .ok()
            .as_deref(),
    )
}

fn explicit_opt_in(value: Option<&str>) -> bool {
    value == Some("on")
}

/// Rate-limited divergence counter. First 10 calls return `true`; subsequent
/// calls sample 1-in-100. Process-scoped in-memory AtomicU64 + persistent
/// mirror row in `heirloom_divergence_counter(key='bridge_soft_fail_total')`
/// so cumulative counts survive process restart (FIX 4 — feedback_transparent_limits).
#[derive(Debug)]
pub struct DivergenceCounter {
    total: AtomicU64,
    /// Optional persistent sink. `None` under `#[cfg(test)]` helpers that
    /// don't need persistence; `Some` when constructed via `new_persistent`.
    persist_conn: Option<Arc<parking_lot::Mutex<rusqlite::Connection>>>,
}

impl Default for DivergenceCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl DivergenceCounter {
    /// In-memory-only counter (tests).
    pub fn new() -> Self {
        Self {
            total: AtomicU64::new(0),
            persist_conn: None,
        }
    }

    /// Persistent counter that mirrors every tick into
    /// `heirloom_divergence_counter`. `conn` must already have the table
    /// (created by `heirloom::schema::ensure_tables` — see Task 1.5).
    ///
    /// Loads the existing row on construction so process restart continues
    /// the count rather than resetting to 0.
    pub fn new_persistent(conn: Arc<parking_lot::Mutex<rusqlite::Connection>>) -> Self {
        // Explicit match (not `.unwrap_or(0)`) — distinguish "no row yet"
        // (first-ever run, start from 0) from "SQL error" (logged as warn,
        // then also start from 0 so the counter still functions in degraded
        // mode). This preserves the feedback_transparent_limits principle
        // while not blocking startup on a counter-table read hiccup.
        let initial: i64 = match conn.lock().query_row(
            "SELECT value FROM heirloom_divergence_counter WHERE key = 'bridge_soft_fail_total'",
            [],
            |r| r.get(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => 0,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "DivergenceCounter persistent load 失败，从 0 起算 / \
                     DivergenceCounter persistent load failed, restarting from 0",
                );
                0
            }
        };
        Self {
            total: AtomicU64::new(initial as u64),
            persist_conn: Some(conn),
        }
    }

    pub fn should_emit(&self) -> bool {
        let n = self.total.fetch_add(1, Ordering::Relaxed);
        // Best-effort persist — swallow errors (soft-fail consistency).
        if let Some(conn) = &self.persist_conn {
            let new_total = (n as i64) + 1;
            let _ = conn.lock().execute(
                "INSERT INTO heirloom_divergence_counter(key, value) VALUES ('bridge_soft_fail_total', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = ?1",
                [new_total],
            );
        }
        n < 10 || n.is_multiple_of(100)
    }

    pub fn count(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
}

/// Shim bridge. `MemoryHandle` holds `Option<Arc<HeirloomBridge>>`.
pub struct HeirloomBridge {
    writer: Arc<HeirloomWriter>,
    did: Did,
    enabled: bool,
    counter: DivergenceCounter,
}

impl HeirloomBridge {
    /// Test / in-memory constructor. Uses `DivergenceCounter::new()` (no persistence).
    pub fn new(writer: Arc<HeirloomWriter>, did: Did) -> Self {
        Self {
            writer,
            did,
            enabled: true,
            counter: DivergenceCounter::new(),
        }
    }

    /// Experimental constructor requiring `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on`
    /// and persisting divergence counts into
    /// `heirloom_divergence_counter` (FIX 4 — feedback_transparent_limits).
    ///
    /// The `writer.shared_connection()` is re-used for the counter persistence
    /// sink so a single connection handles both event-log INSERTs and
    /// counter-row UPSERTs (SQLite WAL permits the sequential access pattern).
    pub fn new_with_env(writer: Arc<HeirloomWriter>, did: Did) -> Self {
        Self::new_persistent(writer, did, experimental_enabled())
    }

    fn new_persistent(writer: Arc<HeirloomWriter>, did: Did, enabled: bool) -> Self {
        let counter = if enabled {
            DivergenceCounter::new_persistent(writer.shared_connection())
        } else {
            DivergenceCounter::new()
        };
        Self {
            writer,
            did,
            enabled,
            counter,
        }
    }

    pub fn did(&self) -> &Did {
        &self.did
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn writer(&self) -> &Arc<HeirloomWriter> {
        &self.writer
    }
    pub fn counter(&self) -> &DivergenceCounter {
        &self.counter
    }

    /// Emit an envelope. **Soft-fail**: any error is rate-limit-warned and
    /// swallowed. Returns `()` to prevent callers from `?`-propagating (D2).
    ///
    /// Caller builds with sentinel `hlc_ts: HlcTimestamp{0,0}`;
    /// `HeirloomWriter::append_event` overwrites with `self.hlc.tick()` (G5).
    pub fn try_emit(&self, envelope: EventEnvelope) {
        if !self.enabled {
            return;
        }
        match self.writer.append_event(envelope) {
            Ok(_hash) => {}
            Err(e) => {
                crate::behavior_notice::record(
                    "heirloom_write_unavailable",
                    "普通记忆写入已继续，但本次 Heirloom 附加记录未写入；两份记录可能不一致。",
                    "Standard memory writing continued, but this Heirloom record was not written; the records may differ.",
                );
                // should_emit() already ticks the counter unconditionally;
                // the return value only gates the tracing::warn! to rate-limit
                // log spam. Every failure counts toward the persisted total.
                if self.counter.should_emit() {
                    tracing::warn!(
                        error = %e,
                        count = self.counter.count(),
                        "HeirloomBridge: soft-fail divergence / heirloom 写入失败，legacy 写入继续",
                    );
                }
            }
        }
    }
}

// ── Envelope builders (one per shimmed event type) ────────────────────

/// File-local: build any event envelope with sentinel HLC.
pub(crate) fn build_envelope(
    did: &Did,
    event_type: &str,
    payload: serde_json::Value,
    tier: SensitivityTier,
) -> EventEnvelope {
    EventEnvelope {
        event_id: Uuid::now_v7(),
        schema_version: 1,
        event_type: EventTypeTag::new(event_type),
        prev_hash: [0; 32],
        hlc_ts: HlcTimestamp { phys: 0, log: 0 },
        observed_at: Utc::now(),
        l0_hash: [0; 32],
        l1_hash: [0; 32],
        l2_manifest_hash: [0; 32],
        lineage_id: LineageId::new(SHIM_LINEAGE),
        author_did: did.clone(),
        signatures: vec![Signature::new(vec![])],
        sensitivity_tier: tier,
        bundle_ids: vec![],
        payload_encoding: PayloadEncoding::Plaintext,
        payload: serde_json::to_vec(&payload).expect("static JSON"),
    }
}

// Envelope-builder impl block. These helpers are consumed by memory.rs +
// orchestrator.rs shim tails in Tasks 3-8; flagged `#[allow(dead_code)]` until
// all callers are wired in so the crate still compiles between tasks.
#[allow(dead_code)]
impl HeirloomBridge {
    /// Topic → `learned:<subspace>` mapping. Task 1 D5 defaults to
    /// `learned:interest`; Task 8 adds the `learned:lesson_crystallized`
    /// branch for `topic.starts_with("learned:lesson")`.
    pub(crate) fn topic_to_event_type(topic: &str) -> &'static str {
        if topic.starts_with("learned:lesson") || topic == "lesson" {
            "learned:lesson_crystallized"
        } else if topic.starts_with("habit") {
            "learned:habit"
        } else {
            "learned:interest"
        }
    }

    pub(crate) fn build_learned_envelope(did: &Did, topic: &str, fact: &str) -> EventEnvelope {
        let event_type = Self::topic_to_event_type(topic);
        let payload = serde_json::json!({
            "topic": topic, "fact": fact,
            "subspace": event_type.trim_start_matches("learned:"),
        });
        build_envelope(did, event_type, payload, SensitivityTier::Sensitive)
    }

    pub(crate) fn build_observation_envelope(
        did: &Did,
        topic: &str,
        fact: &str,
        origin_is_user: bool,
    ) -> EventEnvelope {
        let event_type = if origin_is_user {
            "observation:user_stated"
        } else {
            "ai:observation"
        };
        let payload = serde_json::json!({
            "topic": topic, "fact": fact,
            "origin": if origin_is_user { "user" } else { "ai" },
        });
        build_envelope(did, event_type, payload, SensitivityTier::Sensitive)
    }

    /// L1 envelope with role-aware branching + `reply_to_hash` (G7).
    pub(crate) fn build_l1_envelope(&self, role: &str, text: &str) -> EventEnvelope {
        let is_assistant = role == "assistant";
        let event_type = if is_assistant {
            "ai:said"
        } else {
            "observation:user_stated"
        };
        let mut payload = serde_json::json!({ "role": role, "text": text });
        if is_assistant && let Some(tip) = self.writer.current_tip() {
            payload["reply_to_hash"] = serde_json::Value::String(hex::encode(tip.hash));
        }
        build_envelope(&self.did, event_type, payload, SensitivityTier::Sensitive)
    }

    pub(crate) fn build_tool_invoked_envelope(
        did: &Did,
        tool_name: &str,
        args_serialized: &[u8],
    ) -> EventEnvelope {
        let args_hash = blake3::hash(args_serialized);
        let payload = serde_json::json!({
            "tool_name": tool_name,
            "args_hash": args_hash.to_hex().to_string(),
        });
        build_envelope(did, "ai:tool_invoked", payload, SensitivityTier::Sensitive)
    }

    pub(crate) fn build_tool_rejected_envelope(
        did: &Did,
        tool_name: &str,
        reason: &str,
        policy_ref: &str,
    ) -> EventEnvelope {
        let payload = serde_json::json!({
            "tool_name": tool_name, "reason": reason, "policy_ref": policy_ref,
        });
        build_envelope(
            did,
            "ai:tool_rejected_by_gate",
            payload,
            SensitivityTier::Sensitive,
        )
    }

    /// Build the `profile:slot_upserted` envelope using REAL `ProfileEntry`
    /// fields — `category` (not "slot"), `key` (not "sub_slot"), `value`
    /// (String, not value_json). Grep-confirmed field names in memory.rs:218:
    ///   `pub struct ProfileEntry { id, agent_did, category, key, value,
    ///    confidence, source, created_at, updated_at }`
    /// (FIX 3 CTO Round 1.)
    pub(crate) fn build_profile_upsert_envelope(
        did: &Did,
        category: &str,
        key: &str,
        value: &str,
        confidence: f64,
        source: &str,
    ) -> EventEnvelope {
        let payload = serde_json::json!({
            "category": category,
            "key": key,
            "value": value,
            "confidence": confidence,
            "source": source,
        });
        build_envelope(
            did,
            "profile:slot_upserted",
            payload,
            SensitivityTier::Sensitive,
        )
    }

    pub(crate) fn build_lesson_crystallized_envelope(
        did: &Did,
        title: &str,
        body: &str,
    ) -> EventEnvelope {
        let payload = serde_json::json!({ "title": title, "body": body });
        build_envelope(
            did,
            "learned:lesson_crystallized",
            payload,
            SensitivityTier::Sensitive,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn build_proactive_suggestion_envelope(
        did: &Did,
        suggestion: &str,
        reasoning_summary: &str,
    ) -> EventEnvelope {
        let payload = serde_json::json!({
            "suggestion": suggestion, "reasoning_summary": reasoning_summary,
        });
        build_envelope(
            did,
            "ai:proactive_suggestion",
            payload,
            SensitivityTier::Sensitive,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::{seams::ConstitutionTuple, writer::tests::build_writer_with_registry};

    fn bootstrapped_writer() -> Arc<HeirloomWriter> {
        let w = Arc::new(build_writer_with_registry());
        // Register the SchemaRegistryProjector BEFORE bootstrap so the 46
        // schema:type_defined events populate the registry DB rows; otherwise
        // the schema gate rejects every non-hardcoded event type.
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple {
                l0_hash: [0; 32],
                l1_hash: [0; 32],
                l2_manifest_hash: [0; 32],
            },
            crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
        )
        .unwrap();
        w
    }

    fn sample_envelope(et: &str) -> EventEnvelope {
        build_envelope(
            &Did::new("did:aegis:test"),
            et,
            serde_json::json!({}),
            SensitivityTier::Ordinary,
        )
    }

    #[test]
    fn t_e1_divergence_counter_first_10_emit() {
        let c = DivergenceCounter::new();
        let emits: u64 = (0..10).map(|_| c.should_emit() as u64).sum();
        assert_eq!(emits, 10, "first 10 calls must emit");
    }

    #[test]
    fn t_e1_divergence_counter_samples_after_10() {
        let c = DivergenceCounter::new();
        for _ in 0..10 {
            c.should_emit();
        }
        let mut emits_after = 0;
        for _ in 10..200 {
            if c.should_emit() {
                emits_after += 1;
            }
        }
        assert!(
            (1..=5).contains(&emits_after),
            "expected 1-5 samples, got {emits_after}"
        );
    }

    #[test]
    fn t_e1_try_emit_bridge_disabled_noops() {
        let w = bootstrapped_writer();
        let mut bridge = HeirloomBridge::new(w.clone(), Did::new("did:aegis:test"));
        bridge.enabled = false;
        bridge.try_emit(sample_envelope("observation:user_stated"));
        let conn = w.shared_connection();
        let n: i64 = conn
            .lock()
            .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
            .unwrap();
        // Bootstrap = genesis + SchemeRegistered + 46 type_defined = 48.
        // Task 1.5 bumps to 50; this test uses `>= 48` floor.
        assert!(
            n >= 48,
            "disabled bridge must not append beyond bootstrap; got {n}"
        );
    }

    #[tokio::test]
    async fn t_e1_try_emit_schema_unknown_warns_but_no_panic() {
        let w = bootstrapped_writer();
        let bridge = HeirloomBridge::new(w.clone(), Did::new("did:aegis:test"));
        let (_, notices) = crate::behavior_notice::collect("en", None, None, async {
            bridge.try_emit(sample_envelope("invented:nonsense"));
        })
        .await;
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("Heirloom record was not written"));
        assert_eq!(
            bridge.counter.count(),
            1,
            "counter must tick on schema reject"
        );
        bridge.try_emit(sample_envelope("observation:user_stated"));
        let conn = w.shared_connection();
        let n: i64 = conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'observation:user_stated'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "registered event_type must append even after a rejected one"
        );
    }

    #[test]
    fn t_e9_only_explicit_on_enables_bridge() {
        let w = bootstrapped_writer();
        for value in [
            None,
            Some("off"),
            Some(""),
            Some("true"),
            Some("ON"),
            Some("typo"),
        ] {
            let bridge = HeirloomBridge::new_persistent(
                w.clone(),
                Did::new("did:aegis:test"),
                explicit_opt_in(value),
            );
            assert!(!bridge.enabled());
            bridge.try_emit(sample_envelope("observation:user_stated"));
        }
        let count = || {
            w.shared_connection().lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'observation:user_stated'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap()
        };
        assert_eq!(count(), 0);
        let bridge = HeirloomBridge::new_persistent(
            w.clone(),
            Did::new("did:aegis:test"),
            explicit_opt_in(Some("on")),
        );
        assert!(bridge.enabled());
        bridge.try_emit(sample_envelope("observation:user_stated"));
        assert_eq!(count(), 1);
    }

    // FIX 4 — persistent counter survives synthetic restart.
    #[test]
    fn t_e_div_count_persists_between_restarts() {
        let w = bootstrapped_writer();
        // First "process":
        {
            let bridge =
                HeirloomBridge::new_persistent(w.clone(), Did::new("did:aegis:test"), true);
            for _ in 0..3 {
                bridge.try_emit(sample_envelope("invented:nonsense"));
            }
            assert_eq!(bridge.counter.count(), 3);
        }
        // Second "process" reuses the same writer connection (simulating
        // `main.rs` restart). Counter should load `3` from the persisted row.
        let bridge2 = HeirloomBridge::new_persistent(w.clone(), Did::new("did:aegis:test"), true);
        assert_eq!(
            bridge2.counter.count(),
            3,
            "persistent counter must continue across restarts"
        );
        // Verify the row itself exists with value 3.
        let conn = w.shared_connection();
        let v: i64 = conn
            .lock()
            .query_row(
                "SELECT value FROM heirloom_divergence_counter WHERE key = 'bridge_soft_fail_total'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, 3, "persisted row must be 3");
    }
}
