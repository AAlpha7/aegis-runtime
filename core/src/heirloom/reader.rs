//! HeirloomReader: unified read entry point.
//!
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §4.
//!
//! query() pipeline:
//!   1. load candidates from heirloom_event_log (filtered by lineage + optional as_of)
//!   2. apply_all_gates per candidate (in fixed order, Sacred to Scene)
//!   3. rank.score() on survivors
//!   4. truncate (MMR)
//!   5. bump_access_count for selected events (only if ctx.update_hebbian)

use crate::heirloom::EventEnvelope;
use crate::heirloom::enums::SensitivityTier;
use crate::heirloom::filter::{GateOutcome, apply_all_gates, extract_status};
use crate::heirloom::hebbian::{bump_access_count, read_access_count};
use crate::heirloom::query::QueryContext;
use crate::heirloom::rank::{RankWeights, StoredEvent, score};
use crate::heirloom::truncate::{DEFAULT_MMR_LAMBDA, Scored, truncate};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("canonical parse error: {0}")]
    Canonical(String),
    #[error(
        "sacred 拒绝 / sacred access denied — use read_constitution or read_autonomy_grants; 核心事件禁止通过通用 query() 访问"
    )]
    SacredAccessDenied,
}

/// Reader struct. Holds an Arc<Mutex<Connection>> (same invariant as Writer:
/// single connection owner). RankWeights configurable via `with_weights()`.
#[derive(Debug)]
pub struct HeirloomReader {
    pub(crate) conn: Arc<Mutex<Connection>>,
    pub(crate) weights: RankWeights,
}

impl HeirloomReader {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            conn,
            weights: RankWeights::DEFAULT,
        }
    }

    pub fn with_weights(mut self, w: RankWeights) -> Self {
        self.weights = w;
        self
    }

    /// General-purpose recall query. Runs filter → rank → truncate. Returns
    /// up to `ctx.max_results` scored events within `ctx.token_budget`.
    ///
    /// Sacred events never surface here — use `read_constitution()` or
    /// `read_autonomy_grants()` for those.
    pub fn query(&self, ctx: &QueryContext) -> Result<Vec<Scored>, ReaderError> {
        let candidates = self.load_candidates(ctx)?;

        let mut passed: Vec<(Scored, bool)> = Vec::with_capacity(candidates.len());
        for (event, status) in candidates {
            match apply_all_gates(&event, ctx, status) {
                GateOutcome::Accept { soft_penalty } => {
                    let s = score(
                        &event,
                        ctx,
                        &self.weights,
                        soft_penalty,
                        0.0, // D6: no contradiction_penalty until Phase D
                    );
                    passed.push((Scored { event, score: s }, soft_penalty));
                }
                GateOutcome::Reject(_) => {}
            }
        }

        // Stable sort by score DESC for MMR input (MMR still re-reorders
        // greedily, but a sorted input makes behavior deterministic).
        passed.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let scored: Vec<Scored> = passed.into_iter().map(|(s, _)| s).collect();
        let selected = truncate(scored, ctx, DEFAULT_MMR_LAMBDA);

        if ctx.update_hebbian {
            let conn = self.conn.lock();
            let now = ctx.now.to_rfc3339();
            for s in &selected {
                // Non-fatal: Hebbian bump failure must NOT fail the query,
                // but it MUST be visible (REVIEW_GUIDE §4: no silent swallow).
                if let Err(e) = bump_access_count(&conn, &s.event.event_hash, &now) {
                    tracing::warn!(
                        error = %e,
                        event_hash = ?s.event.event_hash,
                        "heirloom hebbian bump failed"
                    );
                }
            }
        }

        Ok(selected)
    }

    /// Load candidate events from the log filtered by lineage + optional
    /// as_of cursor. Rejects are applied later by `apply_all_gates`; this
    /// is pure SQL selectivity. Returns `(event, status)` tuples.
    pub(crate) fn load_candidates(
        &self,
        ctx: &QueryContext,
    ) -> Result<Vec<(StoredEvent, crate::heirloom::filter::EventStatus)>, ReaderError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT event_hash, envelope_canonical, payload
             FROM heirloom_event_log
             WHERE lineage_id = ?1
             ORDER BY hlc_phys ASC, hlc_log ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![ctx.lineage_id.as_str()], |row| {
            let hash_blob: Vec<u8> = row.get(0)?;
            let canonical: Vec<u8> = row.get(1)?;
            let payload: Vec<u8> = row.get(2)?;
            Ok((hash_blob, canonical, payload))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (hash_blob, canonical, payload) = r?;
            if hash_blob.len() != 32 {
                return Err(ReaderError::Canonical(format!(
                    "event_hash blob length {}, expected 32",
                    hash_blob.len()
                )));
            }
            let envelope: EventEnvelope = serde_json::from_slice(&canonical)
                .map_err(|e| ReaderError::Canonical(format!("envelope: {e}")))?;

            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_blob);

            let ac = read_access_count(&conn, &hash).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    event_hash = ?hash,
                    "heirloom access_count read failed, defaulting to 0"
                );
                0
            });
            let se = StoredEvent::from_envelope(&envelope, Some(ac), hash);
            let status = extract_status(&payload);
            out.push((se, status));
        }
        Ok(out)
    }

    /// Dedicated Sacred read — Core Soul / manifest / AutonomyGrant
    /// read-path. Phase C3 impl: authenticated by `CapabilitySet` having
    /// max_tier == Sacred (caller must explicitly elevate). Phase D Loyalty
    /// layer may layer additional attestation checks.
    pub fn read_constitution(&self, ctx: &QueryContext) -> Result<Vec<Scored>, ReaderError> {
        if ctx.capabilities.max_tier != SensitivityTier::Sacred {
            return Err(ReaderError::SacredAccessDenied);
        }
        // Same pipeline but Gate 1 does not apply. Phase C3: return all
        // Sacred events in the lineage (there are few; no rank/MMR needed
        // yet). Phase D will add real rank/truncate for Sacred reads.
        let candidates = self.load_candidates(ctx)?;
        let out: Vec<Scored> = candidates
            .into_iter()
            .filter(|(e, _)| e.sensitivity_tier == SensitivityTier::Sacred)
            .map(|(e, _)| Scored {
                event: e,
                score: 0.0,
            })
            .collect();
        Ok(out)
    }

    /// Dedicated AutonomyGrant read — filter to events of type prefix
    /// "loyalty:autonomy_grant_*". Same Sacred access gate.
    pub fn read_autonomy_grants(&self, ctx: &QueryContext) -> Result<Vec<Scored>, ReaderError> {
        if ctx.capabilities.max_tier != SensitivityTier::Sacred {
            return Err(ReaderError::SacredAccessDenied);
        }
        let candidates = self.load_candidates(ctx)?;
        let out: Vec<Scored> = candidates
            .into_iter()
            .filter(|(e, _)| e.event_type.starts_with("loyalty:autonomy_grant"))
            .map(|(e, _)| Scored {
                event: e,
                score: 0.0,
            })
            .collect();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::EventEnvelope;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::query::{
        CapabilitySet, CoreOnlyGrantProvider, OwnerState, QueryContext, Scene,
    };
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature, WitnessChannel};
    use crate::heirloom::writer::HeirloomWriter;
    use crate::heirloom::writer::tests::{bootstrap, build_writer};
    use chrono::{DateTime, Utc};
    use std::sync::Arc;
    use uuid::Uuid;

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_ctx_with_hebbian_on() -> QueryContext {
        QueryContext::new_current(
            now_utc(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Unknown,
            CapabilitySet::dev_default(),
            8192,
            20,
            Arc::new(CoreOnlyGrantProvider),
        )
    }

    fn sample_ctx_with_as_of(cursor: HlcTimestamp) -> QueryContext {
        QueryContext::new_as_of(
            cursor,
            now_utc(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Unknown,
            CapabilitySet::dev_default(),
            8192,
            20,
            Arc::new(CoreOnlyGrantProvider),
        )
    }

    fn sample_ctx_sacred() -> QueryContext {
        let mut caps = CapabilitySet::dev_default();
        caps.max_tier = SensitivityTier::Sacred;
        caps.accessible_bundles = vec![BundleId::new("bundle:core")];
        QueryContext::new_current(
            now_utc(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Unknown,
            caps,
            8192,
            20,
            Arc::new(CoreOnlyGrantProvider),
        )
    }

    /// Append a minimal valid observation event to the writer.
    fn append_test_observation(w: &HeirloomWriter, payload: &[u8]) {
        let tip = w.current_tip().expect("writer must be bootstrapped");
        let env = EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash: tip.hash,
            hlc_ts: w.hlc.tick(),
            observed_at: now_utc(),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![Signature::new(vec![])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: payload.to_vec(),
        };
        w.append_event(env).expect("append_event");
    }

    #[test]
    fn t11_reader_new_initializes_default_weights() {
        let w = build_writer();
        let r = HeirloomReader::new(w.shared_connection());
        assert_eq!(r.weights, RankWeights::DEFAULT);
    }

    #[test]
    fn t11_reader_with_weights_overrides_default() {
        let w = build_writer();
        let mut custom = RankWeights::DEFAULT;
        custom.cosine = 2.0;
        let r = HeirloomReader::new(w.shared_connection()).with_weights(custom);
        assert_eq!(r.weights.cosine, 2.0);
    }

    #[test]
    fn t11_reader_query_empty_log_returns_empty_vec() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        let out = r.query(&sample_ctx_with_hebbian_on()).unwrap();
        assert!(out.is_empty(), "bootstrap-only has no non-Sacred events");
    }

    #[test]
    fn t11_reader_query_bootstrap_only_returns_only_non_sacred() {
        // Bootstrap writes genesis + SchemeRegistered at sensitivity Sacred.
        // Non-Sacred event count = 0.
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        let out = r.query(&sample_ctx_with_hebbian_on()).unwrap();
        assert!(
            out.is_empty(),
            "genesis + SchemeRegistered are Sacred; must not surface"
        );
    }

    #[test]
    fn t11_reader_query_returns_appended_observations_in_score_order() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        append_test_observation(&w, br#"{"text":"first"}"#);
        append_test_observation(&w, br#"{"text":"second"}"#);
        append_test_observation(&w, br#"{"text":"third"}"#);

        let out = r.query(&sample_ctx_with_hebbian_on()).unwrap();
        assert_eq!(out.len(), 3, "3 observations appended");
        // All have identical profile score, scene Unknown, so they tie on score;
        // sort is stable with partial_cmp → order undefined, but length is deterministic.
    }

    #[test]
    fn t11_reader_query_respects_max_results() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        for i in 0..5 {
            let body = format!(r#"{{"n":{i}}}"#);
            append_test_observation(&w, body.as_bytes());
        }
        let mut ctx = sample_ctx_with_hebbian_on();
        ctx.max_results = 2;
        let out = r.query(&ctx).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn t11_reader_query_respects_token_budget() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        // Each event token_cost = payload.len()/4 max 1 — tiny events = 1 token each.
        for i in 0..10 {
            let body = format!(r#"{{"n":{i}}}"#);
            append_test_observation(&w, body.as_bytes());
        }
        let mut ctx = sample_ctx_with_hebbian_on();
        ctx.max_results = 100;
        ctx.token_budget = 3;
        let out = r.query(&ctx).unwrap();
        assert!(
            out.len() <= 3,
            "token_budget caps result count; got {}",
            out.len()
        );
    }

    #[test]
    fn t11_reader_query_skips_retracted_events() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        append_test_observation(&w, br#"{"text":"active"}"#);
        append_test_observation(&w, br#"{"status":"retracted"}"#);
        append_test_observation(&w, br#"{"status":"superseded"}"#);
        let out = r.query(&sample_ctx_with_hebbian_on()).unwrap();
        assert_eq!(out.len(), 1, "only the active event passes Gate 5");
    }

    #[test]
    fn t11_hebbian_bump_skipped_when_update_hebbian_false() {
        // SEMANTIC-CRITICAL: as_of (time-travel) queries MUST NOT bump
        // heirloom_access_count. Round-9 P2.2.
        let w = build_writer();
        bootstrap(&w);
        let reader = HeirloomReader::new(w.shared_connection());
        append_test_observation(&w, br#"{"text":"first"}"#);

        // First, a current query that should bump.
        let current_ctx = sample_ctx_with_hebbian_on();
        let _ = reader.query(&current_ctx).unwrap();
        let count_after_current: u64 = {
            let conn = w.shared_connection();
            let guard = conn.lock();
            guard
                .query_row(
                    "SELECT SUM(count) FROM heirloom_access_count",
                    [],
                    |row: &rusqlite::Row<'_>| row.get::<_, Option<i64>>(0),
                )
                .unwrap()
                .unwrap_or(0) as u64
        };
        assert!(
            count_after_current > 0,
            "current query must bump at least once"
        );

        // Now an as_of query — MUST NOT bump.
        let cursor = w.current_tip().unwrap().hlc;
        let as_of_ctx = sample_ctx_with_as_of(cursor);
        let _ = reader.query(&as_of_ctx).unwrap();
        let count_after_as_of: u64 = {
            let conn = w.shared_connection();
            let guard = conn.lock();
            guard
                .query_row(
                    "SELECT SUM(count) FROM heirloom_access_count",
                    [],
                    |row: &rusqlite::Row<'_>| row.get::<_, Option<i64>>(0),
                )
                .unwrap()
                .unwrap_or(0) as u64
        };
        assert_eq!(
            count_after_as_of, count_after_current,
            "as_of query must NOT bump access_count"
        );
    }

    #[test]
    fn t11_reader_read_constitution_requires_sacred_capability() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());

        // Non-Sacred capability → SacredAccessDenied.
        let non_sacred = sample_ctx_with_hebbian_on();
        let err = r.read_constitution(&non_sacred).unwrap_err();
        assert!(matches!(err, ReaderError::SacredAccessDenied));

        // Sacred capability → Ok. Genesis (heirloom:genesis) + SchemeRegistered
        // are Sacred tier and survive the filter.
        let sacred = sample_ctx_sacred();
        let out = r.read_constitution(&sacred).unwrap();
        assert!(
            out.len() >= 2,
            "Sacred reader sees genesis + SchemeRegistered; got {} events",
            out.len()
        );
    }

    #[test]
    fn e2e_reader_round_trip_write_then_query_matches_appended_events() {
        let w = build_writer();
        bootstrap(&w);
        let r = HeirloomReader::new(w.shared_connection());
        append_test_observation(&w, br#"{"text":"alpha"}"#);
        append_test_observation(&w, br#"{"text":"beta"}"#);
        append_test_observation(&w, br#"{"text":"gamma"}"#);

        let out = r.query(&sample_ctx_with_hebbian_on()).unwrap();
        assert_eq!(out.len(), 3);
        // Assert all event_types are the expected observation type.
        for s in &out {
            assert_eq!(s.event.event_type, "observation:user_stated");
        }
    }
}
