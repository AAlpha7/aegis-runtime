//! Rank stage: 7-term weighted scoring formula + staleness curve +
//! source_trust table + StoredEvent DTO. Spec §4 Q4.3.
//!
//! Phase C3 hardcodes the default RankWeights; Phase D will add a
//! config layer. Deviations D1-D6 documented at plan top.

use crate::heirloom::EventEnvelope;
use crate::heirloom::enums::SensitivityTier;
use crate::heirloom::hlc::HlcTimestamp;
use crate::heirloom::query::{EntityRef, Scene, TemporalClass};
use crate::heirloom::types::{BundleId, Did, LineageId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Row-shape hydrated from `heirloom_event_log`. Carries envelope fields
/// plus optional payload-extracted extensions (salience, entity_refs,
/// temporal_class, scene_tag). All extensions default per D2-D4.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    // Identity
    pub event_hash: [u8; 32],
    pub event_type: String,
    pub author_did: Did,
    pub lineage_id: LineageId,

    // Chain
    pub hlc_ts: HlcTimestamp,
    pub observed_at: DateTime<Utc>,

    // Security
    pub sensitivity_tier: SensitivityTier,
    pub bundle_ids: Vec<BundleId>,

    // Payload-extracted extensions (defaults per D2-D4)
    pub salience: f32,
    pub entity_refs: Vec<EntityRef>,
    pub temporal_class: TemporalClass,
    pub scene_tag: Scene,

    // Projection-backed
    pub access_count: u64,

    // Embedding (populated by recall.rs; None in Phase C3 unit tests)
    pub embedding: Option<Vec<f32>>,

    // Approximate token cost for truncate (default: byte count / 4)
    pub token_cost: usize,
}

impl StoredEvent {
    /// Build from an envelope with explicit access_count. Payload-extracted
    /// fields default per D2-D4.
    pub fn from_envelope(
        env: &EventEnvelope,
        access_count: Option<u64>,
        event_hash: [u8; 32],
    ) -> Self {
        Self {
            event_hash,
            event_type: env.event_type.as_str().to_string(),
            author_did: env.author_did.clone(),
            lineage_id: env.lineage_id.clone(),
            hlc_ts: env.hlc_ts,
            observed_at: env.observed_at,
            sensitivity_tier: env.sensitivity_tier,
            bundle_ids: env.bundle_ids.clone(),
            salience: 0.5,
            entity_refs: vec![],
            temporal_class: TemporalClass::default(),
            scene_tag: Scene::Unknown,
            access_count: access_count.unwrap_or(0),
            embedding: None,
            token_cost: (env.payload.len() / 4).max(1),
        }
    }
}

/// 7-term rank weight struct (spec §4 Q4.3 default table).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RankWeights {
    pub cosine: f32,
    pub source_trust: f32,
    pub salience: f32,
    pub ctx_match: f32,
    pub hebbian: f32,
    pub contradiction_penalty: f32,
    pub staleness_penalty: f32,
}

impl RankWeights {
    pub const DEFAULT: Self = Self {
        cosine: 1.0,
        source_trust: 0.3,
        salience: 0.2,
        ctx_match: 0.4,
        hebbian: 0.15,
        contradiction_penalty: 0.5,
        staleness_penalty: 0.25,
    };
}

impl Default for RankWeights {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Source trust prior by event_type prefix. Spec §4 Q4.3 default table.
/// Unknown prefixes fall back to 0.5 (neutral) rather than 0.0 to avoid
/// zeroing the rank for Phase D-added event types we haven't tabulated.
pub fn source_trust(event_type: &str) -> f32 {
    if event_type.starts_with("profile:") {
        0.9
    } else if event_type.starts_with("observation:") {
        0.7
    } else if event_type.starts_with("learned:") {
        0.6
    } else if event_type.starts_with("ephemeral:") {
        0.2
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
    use crate::heirloom::{EventEnvelope, HlcTimestamp};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    fn sample_envelope() -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp { phys: 100, log: 0 },
            observed_at: DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![Signature::new(vec![])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{}".to_vec(),
        }
    }

    #[test]
    fn t11_stored_event_from_envelope_defaults_extensions() {
        let e = sample_envelope();
        let se = StoredEvent::from_envelope(&e, None, [0x11; 32]);
        assert_eq!(se.event_hash, [0x11; 32]);
        assert_eq!(se.salience, 0.5, "D2: default salience = 0.5");
        assert_eq!(se.temporal_class, TemporalClass::Slow, "D4: default = Slow");
        assert!(se.entity_refs.is_empty(), "D3: default = empty");
        assert_eq!(se.scene_tag, Scene::Unknown);
        assert_eq!(se.access_count, 0);
    }

    #[test]
    fn t11_stored_event_from_envelope_uses_explicit_access_count() {
        let e = sample_envelope();
        let se = StoredEvent::from_envelope(&e, Some(42), [0x22; 32]);
        assert_eq!(se.access_count, 42);
    }

    #[test]
    fn t11_rank_weights_default_matches_spec_table() {
        let w = RankWeights::DEFAULT;
        assert_eq!(w.cosine, 1.0);
        assert_eq!(w.source_trust, 0.3);
        assert_eq!(w.salience, 0.2);
        assert_eq!(w.ctx_match, 0.4);
        assert_eq!(w.hebbian, 0.15);
        assert_eq!(w.contradiction_penalty, 0.5);
        assert_eq!(w.staleness_penalty, 0.25);
    }

    #[test]
    fn t11_source_trust_profile_highest() {
        assert_eq!(source_trust("profile:preference_set"), 0.9);
    }

    #[test]
    fn t11_source_trust_ephemeral_lowest() {
        assert_eq!(source_trust("ephemeral:mood_flash"), 0.2);
    }

    #[test]
    fn t11_source_trust_learned_medium() {
        assert_eq!(source_trust("learned:interest"), 0.6);
    }

    #[test]
    fn t11_source_trust_observation_medium_high() {
        assert_eq!(source_trust("observation:user_stated"), 0.7);
    }

    #[test]
    fn t11_source_trust_unknown_fallback_is_0_5() {
        assert_eq!(source_trust("unrecognized:whatever"), 0.5);
    }
}

/// Cosine similarity between two embeddings. Returns 0.0 if either is None
/// or lengths mismatch. Phase C3 uses the same length-guard as recall.rs.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Context match score — spec §4 Q4.2 Round-9 P2.1. Absorbs Gate 6
/// SoftPenalty as `scene_score = -0.2`. `soft_penalty_scene` input lets
/// the caller skip re-running gate6 (it already did during filter).
pub fn context_match(
    event: &StoredEvent,
    ctx: &crate::heirloom::query::QueryContext,
    soft_penalty_scene: bool,
) -> f32 {
    let entity_score = if entities_fully_match(&event.entity_refs, &ctx.active_entities) {
        0.5
    } else {
        0.0
    };
    let scene_score = if soft_penalty_scene {
        -0.2
    } else if event.scene_tag == Scene::Unknown || event.scene_tag == ctx.scene {
        0.3
    } else {
        // Gate 6 said Accept without SoftPenalty — Scene::Crisis case.
        // Score the same as matching scene.
        0.3
    };
    let temporal_score = if temporal_class_compatible(event.temporal_class, ctx) {
        0.2
    } else {
        0.0
    };
    entity_score + scene_score + temporal_score
}

/// Entity match: true iff every entity in ctx.active_entities also appears
/// in event.entity_refs. Phase C3 is strict AND (per spec: "entity 全匹配").
/// With D3 (entity_refs default empty), this returns false for all events
/// that don't yet have Consolidate-extracted entities — which is fine.
pub fn entities_fully_match(event_ents: &[EntityRef], ctx_ents: &[EntityRef]) -> bool {
    if ctx_ents.is_empty() {
        return false;
    }
    ctx_ents
        .iter()
        .all(|ce| event_ents.iter().any(|ee| ee.id == ce.id))
}

/// Temporal compatibility: Timeless and Slow are always compatible;
/// Ephemeral compatible only for queries within 30 days; Critical always
/// compatible (critical events want to surface wherever).
pub fn temporal_class_compatible(
    cls: TemporalClass,
    _ctx: &crate::heirloom::query::QueryContext,
) -> bool {
    matches!(
        cls,
        TemporalClass::Timeless | TemporalClass::Slow | TemporalClass::Critical
    )
}

/// The 7-term rank score. Positive = more relevant. Typical range
/// [-1.0, 3.0] with default weights.
pub fn score(
    event: &StoredEvent,
    ctx: &crate::heirloom::query::QueryContext,
    weights: &RankWeights,
    soft_penalty_scene: bool,
    contradiction_penalty: f32, // D6: 0.0 in P1
) -> f32 {
    let cos = match (&ctx.query_embedding, &event.embedding) {
        (Some(q), Some(e)) => cosine_similarity(q, e),
        _ => 0.0,
    };
    let st = source_trust(&event.event_type);
    let sal = event.salience;
    let ctxm = context_match(event, ctx, soft_penalty_scene);
    let hebb = ((event.access_count as f32) + 1.0).ln();
    let stale = staleness(event, ctx.now);

    weights.cosine * cos
        + weights.source_trust * st
        + weights.salience * sal
        + weights.ctx_match * ctxm
        + weights.hebbian * hebb
        - weights.contradiction_penalty * contradiction_penalty
        - weights.staleness_penalty * stale
}

/// Forward declaration; staleness() body lives at Task 9.
/// Stub here so score() compiles against the final 7-term signature.
pub fn staleness(event: &StoredEvent, now: DateTime<Utc>) -> f32 {
    let age_days = (now - event.observed_at).num_days() as f32;
    let raw = match event.temporal_class {
        TemporalClass::Timeless => 0.0,
        TemporalClass::Slow => age_days / 3650.0,
        TemporalClass::Ephemeral => age_days / 30.0,
        TemporalClass::Critical => {
            let over = age_days - 7.0;
            if over <= 0.0 { 0.0 } else { over / 30.0 }
        }
    };
    raw.clamp(0.0, 1.0)
}

#[cfg(test)]
mod score_tests {
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::query::{
        CapabilitySet, CoreOnlyGrantProvider, EntityRef as QEntityRef, OwnerState, QueryContext,
        Scene,
    };
    use crate::heirloom::types::{BundleId, Did, LineageId, WitnessChannel};
    use chrono::{DateTime, Utc};
    use std::sync::Arc;

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_stored() -> StoredEvent {
        StoredEvent {
            event_hash: [0; 32],
            event_type: "observation:user_stated".to_string(),
            author_did: Did::new("did:aegis:test"),
            lineage_id: LineageId::new("lineage:test"),
            hlc_ts: HlcTimestamp { phys: 100, log: 0 },
            observed_at: now_utc(),
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            salience: 0.5,
            entity_refs: vec![],
            temporal_class: TemporalClass::Slow,
            scene_tag: Scene::Work,
            access_count: 0,
            embedding: None,
            token_cost: 1,
        }
    }

    fn sample_ctx() -> QueryContext {
        QueryContext::new_current(
            now_utc(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Work,
            CapabilitySet::dev_default(),
            2048,
            20,
            Arc::new(CoreOnlyGrantProvider),
        )
    }

    // ── cosine_similarity ────────────────────────────────────────────────

    #[test]
    fn t11_cosine_similarity_identical_is_1() {
        let a = vec![0.6, 0.8, 0.0];
        let b = a.clone();
        let c = cosine_similarity(&a, &b);
        assert!((c - 1.0).abs() < 1e-5, "cos = {c}");
    }

    #[test]
    fn t11_cosine_similarity_orthogonal_is_0() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-5);
    }

    #[test]
    fn t11_cosine_similarity_handles_length_mismatch() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    // ── context_match ────────────────────────────────────────────────────

    #[test]
    fn t11_context_match_entities_fully_match_adds_0_5() {
        let mut e = sample_stored();
        e.entity_refs = vec![QEntityRef::new("did:alice"), QEntityRef::new("did:bob")];
        let mut ctx = sample_ctx();
        ctx.active_entities = vec![QEntityRef::new("did:alice")];
        let cm_match = context_match(&e, &ctx, false);

        // With entities_fully_match = true, ctx_scene == event_scene → 0.5 entity + 0.3 scene + 0.2 temporal = 1.0
        assert!((cm_match - 1.0).abs() < 1e-5, "cm = {cm_match}");

        // Without active entities, entity_score is 0 → 0.0 + 0.3 + 0.2 = 0.5
        let mut ctx2 = sample_ctx();
        ctx2.active_entities = vec![];
        let cm_no_entity = context_match(&e, &ctx2, false);
        assert!((cm_no_entity - 0.5).abs() < 1e-5, "cm = {cm_no_entity}");
    }

    #[test]
    fn t11_context_match_scene_soft_penalty_adds_negative_0_2() {
        // With soft_penalty_scene = true, scene_score is -0.2.
        // Temporal score 0.2, entity score 0 → total 0.0.
        let e = sample_stored();
        let ctx = sample_ctx();
        let cm = context_match(&e, &ctx, true);
        assert!((cm - 0.0).abs() < 1e-5, "cm = {cm}");
    }

    // ── score() ──────────────────────────────────────────────────────────

    #[test]
    fn t11_score_uses_weighted_sum() {
        // Craft an event with predictable inputs.
        // source_trust("observation:user_stated") = 0.7
        // salience 0.5, context_match 0.5 (no entity; same scene; temporal slow),
        // hebbian ln(1) = 0, staleness(fresh Slow) = 0, contradiction_penalty = 0.
        // Expected score = 0.3 * 0.7 + 0.2 * 0.5 + 0.4 * 0.5 = 0.21 + 0.1 + 0.2 = 0.51
        let e = sample_stored();
        let ctx = sample_ctx();
        let w = RankWeights::DEFAULT;
        let s = score(&e, &ctx, &w, false, 0.0);
        assert!((s - 0.51).abs() < 1e-5, "score = {s}");
    }

    #[test]
    fn t11_score_hebbian_uses_log_not_linear() {
        // access_count 9 → ln(10) ≈ 2.3026.
        // Linear-weighted (wrong) would be 0.15 * 9 = 1.35.
        // Log-weighted (correct) is 0.15 * 2.3026 ≈ 0.3454.
        //
        // Baseline (access_count=0) = 0.51. With count=9 log-weighted gives
        // 0.51 + 0.3454 = 0.8554. Linear would give 0.51 + 1.35 = 1.86.
        let mut e = sample_stored();
        e.access_count = 9;
        let ctx = sample_ctx();
        let w = RankWeights::DEFAULT;
        let s = score(&e, &ctx, &w, false, 0.0);
        let expected_log = 0.51 + 0.15 * (10f32).ln();
        assert!(
            (s - expected_log).abs() < 1e-3,
            "score = {s}, expected ≈ {expected_log}"
        );
        assert!(
            s < 1.0,
            "log-scaled hebbian should keep score below 1.0 at count=9; got {s}"
        );
    }

    #[test]
    fn t11_entities_fully_match_requires_all_ctx_entities_present() {
        let ev = vec![QEntityRef::new("did:alice"), QEntityRef::new("did:bob")];
        let ctx_all = vec![QEntityRef::new("did:alice"), QEntityRef::new("did:bob")];
        assert!(entities_fully_match(&ev, &ctx_all));

        // Subset of ev in ctx — AND semantics says true (all ctx are in ev).
        let ctx_some = vec![QEntityRef::new("did:alice")];
        assert!(entities_fully_match(&ev, &ctx_some));

        // A ctx entity NOT in ev — false.
        let ctx_missing = vec![QEntityRef::new("did:charlie")];
        assert!(!entities_fully_match(&ev, &ctx_missing));

        // Empty ctx — false per Gotcha #6 (no vacuous truth).
        assert!(!entities_fully_match(&ev, &[]));
    }
}

#[cfg(test)]
mod staleness_tests {
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::types::{BundleId, Did, LineageId};
    use chrono::{DateTime, Duration, Utc};

    fn evt_observed_at(observed_at: DateTime<Utc>, cls: TemporalClass) -> StoredEvent {
        StoredEvent {
            event_hash: [0; 32],
            event_type: "observation:user_stated".to_string(),
            author_did: Did::new("did:aegis:test"),
            lineage_id: LineageId::new("lineage:test"),
            hlc_ts: HlcTimestamp { phys: 100, log: 0 },
            observed_at,
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            salience: 0.5,
            entity_refs: vec![],
            temporal_class: cls,
            scene_tag: Scene::Unknown,
            access_count: 0,
            embedding: None,
            token_cost: 1,
        }
    }

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn t11_staleness_timeless_always_0() {
        let e_now = evt_observed_at(now_utc(), TemporalClass::Timeless);
        assert_eq!(staleness(&e_now, now_utc()), 0.0);
        // Even centuries old.
        let e_old = evt_observed_at(
            now_utc() - Duration::days(365 * 100),
            TemporalClass::Timeless,
        );
        assert_eq!(staleness(&e_old, now_utc()), 0.0);
    }

    #[test]
    fn t11_staleness_slow_half_life_10_years() {
        // Slow: age_days / 3650 — at 3650 days, raw = 1.0 (full stale);
        // at 1825 days, raw ≈ 0.5.
        let e_mid = evt_observed_at(now_utc() - Duration::days(1825), TemporalClass::Slow);
        let s_mid = staleness(&e_mid, now_utc());
        assert!((s_mid - 0.5).abs() < 1e-3, "mid staleness = {s_mid}");

        let e_full = evt_observed_at(now_utc() - Duration::days(3650), TemporalClass::Slow);
        let s_full = staleness(&e_full, now_utc());
        assert!((s_full - 1.0).abs() < 1e-3, "full staleness = {s_full}");
    }

    #[test]
    fn t11_staleness_ephemeral_half_life_30_days() {
        // Ephemeral: age_days / 30 — at 30 days, raw = 1.0; at 15 days, raw ≈ 0.5.
        let e15 = evt_observed_at(now_utc() - Duration::days(15), TemporalClass::Ephemeral);
        let s15 = staleness(&e15, now_utc());
        assert!((s15 - 0.5).abs() < 1e-3);

        let e30 = evt_observed_at(now_utc() - Duration::days(30), TemporalClass::Ephemeral);
        let s30 = staleness(&e30, now_utc());
        assert!((s30 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn t11_staleness_critical_grace_period_7_days() {
        // Critical: 0 up to age 7 days; then (age - 7) / 30.
        let e5 = evt_observed_at(now_utc() - Duration::days(5), TemporalClass::Critical);
        assert_eq!(staleness(&e5, now_utc()), 0.0);

        let e7 = evt_observed_at(now_utc() - Duration::days(7), TemporalClass::Critical);
        assert_eq!(staleness(&e7, now_utc()), 0.0);

        let e_22 = evt_observed_at(now_utc() - Duration::days(22), TemporalClass::Critical);
        // 22-7=15, 15/30 = 0.5
        let s = staleness(&e_22, now_utc());
        assert!((s - 0.5).abs() < 1e-3);
    }

    #[test]
    fn t11_staleness_clamps_at_1() {
        // Even a 100-year-old Slow event clamps at 1.0.
        let e = evt_observed_at(now_utc() - Duration::days(365 * 100), TemporalClass::Slow);
        assert_eq!(staleness(&e, now_utc()), 1.0);

        // Same for Ephemeral well past 30 days.
        let e2 = evt_observed_at(now_utc() - Duration::days(365), TemporalClass::Ephemeral);
        assert_eq!(staleness(&e2, now_utc()), 1.0);
    }

    #[test]
    fn t11_staleness_negative_age_clamps_to_0() {
        // Future-dated event (from clock skew) → age_days < 0 → clamp to 0.
        let e = evt_observed_at(now_utc() + Duration::days(5), TemporalClass::Slow);
        assert_eq!(staleness(&e, now_utc()), 0.0);
    }
}
