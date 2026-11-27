//! Filter stage: 6 hard gates run in fixed order. Spec §4 Q4.2.
//!
//! Order is load-bearing: cheapest + strictest first. Regression test
//! `t11_filter_gate_order_is_fixed` pins the order.

use crate::heirloom::enums::SensitivityTier;
use crate::heirloom::query::{CapabilitySet, OwnerState, QueryContext, Scene};
use crate::heirloom::rank::StoredEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterVerdict {
    Accept,
    Reject(RejectReason),
    /// Accepted but with a soft penalty that the rank formula should
    /// consume via `ctx_match.scene_score = -0.2`. Spec §4 Q4.2 Gate 6
    /// Round-9 polish P2.1.
    SoftPenalty(SoftPenaltyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Sacred,
    SensitivityExceedsCapability,
    BundleNotInCapability,
    OwnerStateScope,
    AsOfCursor,
    EventStatusRetracted,
    EventStatusSuperseded,
    SceneIncompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftPenaltyReason {
    SceneMismatch,
}

/// Gate 1 · Sacred gate. Sacred-tier events NEVER surface through
/// generic `query()`. Dedicated APIs (`read_constitution`,
/// `read_autonomy_grants`) have their own elevated access checks.
/// Spec §4 Q4.2 Gate 1 + 宪法 9.
pub fn gate1_sacred(event: &StoredEvent) -> FilterVerdict {
    if event.sensitivity_tier == SensitivityTier::Sacred {
        return FilterVerdict::Reject(RejectReason::Sacred);
    }
    FilterVerdict::Accept
}

/// Gate 4 · As-of temporal cursor. `event.hlc_ts <= cursor` required;
/// future events filtered out for time-travel queries. If `as_of == None`,
/// the gate accepts everything (current-time query).
pub fn gate4_as_of(event: &StoredEvent, ctx: &QueryContext) -> FilterVerdict {
    if let Some(cursor) = ctx.as_of
        && event.hlc_ts > cursor
    {
        return FilterVerdict::Reject(RejectReason::AsOfCursor);
    }
    FilterVerdict::Accept
}

/// Event status extracted from payload (Phase C3 — payload is opaque so
/// the shim reads a `status` key from the top-level JSON object if
/// parseable; default is `Active`). Retracted + Superseded fail Gate 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    #[default]
    Active,
    Retracted,
    Superseded,
}

/// Tag used by PlaintextCodec to frame payloads: `aegis:plaintext-v1:<header>\n<body>`.
/// extract_status strips this framing before attempting the JSON parse so
/// Gate 5 works against the Phase C2 writer flow (which always frames
/// payloads via the codec). Under future AEAD codec, payload is ciphertext
/// and neither raw-parse nor frame-strip yields JSON — status defaults to
/// Active. See Gotcha #11 for the P1.5 upgrade path.
const PLAINTEXT_FRAME_PREFIX: &[u8] = b"aegis:plaintext-v1:";

/// Extract `status` from a top-level JSON payload. Returns `Active` if the
/// payload is not valid JSON, doesn't have a `status` key, or the key is
/// not a recognized value. Phase D adds real payload schema.
///
/// Gotcha #11 from plan: this function is PlaintextCodec-coupled. Once
/// AEAD replaces plaintext (P1.5), payload is ciphertext and every event
/// would silently default to `Active` — effectively disabling Gate 5. The
/// P1.5 reader upgrade must either decrypt before calling this, OR
/// promote `status` to a typed column on `heirloom_event_log`. See the
/// plan's Non-Goals table ("Gate 5 AEAD-aware status decode → P1.5") for
/// the canonical deferral.
pub fn extract_status(payload_bytes: &[u8]) -> EventStatus {
    // Phase C2 writer always frames payload via PlaintextCodec:
    //   aegis:plaintext-v1:<constitution-header-json>\n<body-bytes>
    // Strip the framing prefix + header line if present before JSON parsing.
    let body = if payload_bytes.starts_with(PLAINTEXT_FRAME_PREFIX) {
        let after_tag = &payload_bytes[PLAINTEXT_FRAME_PREFIX.len()..];
        match after_tag.iter().position(|b| *b == b'\n') {
            Some(idx) => &after_tag[idx + 1..],
            None => return EventStatus::Active,
        }
    } else {
        payload_bytes
    };

    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return EventStatus::Active;
    };
    let Some(s) = v.get("status").and_then(|s| s.as_str()) else {
        return EventStatus::Active;
    };
    match s {
        "retracted" => EventStatus::Retracted,
        "superseded" => EventStatus::Superseded,
        _ => EventStatus::Active,
    }
}

/// Gate 5 · Event status. Rejects `retracted` and `superseded` events.
/// Caller passes the already-extracted status (reader does this once per
/// event in the hydration pass).
pub fn gate5_status(status: EventStatus) -> FilterVerdict {
    match status {
        EventStatus::Active => FilterVerdict::Accept,
        EventStatus::Retracted => FilterVerdict::Reject(RejectReason::EventStatusRetracted),
        EventStatus::Superseded => FilterVerdict::Reject(RejectReason::EventStatusSuperseded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::query::{
        CapabilitySet, CoreOnlyGrantProvider, OwnerState, QueryContext, Scene,
    };
    use crate::heirloom::rank::StoredEvent;
    use crate::heirloom::types::{BundleId, Did, LineageId, WitnessChannel};
    use chrono::{DateTime, Utc};
    use std::sync::Arc;

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    pub(super) fn sample_stored_event() -> StoredEvent {
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
            temporal_class: crate::heirloom::query::TemporalClass::Slow,
            scene_tag: Scene::Unknown,
            access_count: 0,
            embedding: None,
            token_cost: 1,
        }
    }

    pub(super) fn sample_ctx() -> QueryContext {
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

    // ── Gate 1 · Sacred ──────────────────────────────────────────────────

    #[test]
    fn t11_gate1_rejects_sacred_event() {
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Sacred;
        assert_eq!(
            gate1_sacred(&e),
            FilterVerdict::Reject(RejectReason::Sacred)
        );
    }

    #[test]
    fn t11_gate1_accepts_non_sacred_events() {
        let e = sample_stored_event();
        assert_eq!(gate1_sacred(&e), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate1_sacred_never_surfaces_through_query() {
        // SEMANTIC-CRITICAL: Sacred tier events (Core Soul / L2 corpus /
        // AutonomyGrant) must NEVER pass gate1 regardless of other fields.
        // 宪法 9 + spec §4 Q4.2 Gate 1.
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
        ] {
            let mut e = sample_stored_event();
            e.sensitivity_tier = tier;
            assert_eq!(gate1_sacred(&e), FilterVerdict::Accept, "tier {tier:?}");
        }
        let mut sacred = sample_stored_event();
        sacred.sensitivity_tier = SensitivityTier::Sacred;
        assert_eq!(
            gate1_sacred(&sacred),
            FilterVerdict::Reject(RejectReason::Sacred),
            "Sacred MUST be rejected"
        );
    }

    // ── Gate 4 · As-of ───────────────────────────────────────────────────

    #[test]
    fn t11_gate4_accepts_when_as_of_is_none() {
        let e = sample_stored_event();
        let ctx = sample_ctx();
        assert!(ctx.as_of.is_none());
        assert_eq!(gate4_as_of(&e, &ctx), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate4_accepts_event_at_or_before_cursor() {
        let mut e = sample_stored_event();
        e.hlc_ts = HlcTimestamp { phys: 50, log: 0 };
        let mut ctx = sample_ctx();
        ctx.as_of = Some(HlcTimestamp { phys: 100, log: 0 });
        assert_eq!(gate4_as_of(&e, &ctx), FilterVerdict::Accept);

        e.hlc_ts = HlcTimestamp { phys: 100, log: 0 };
        assert_eq!(gate4_as_of(&e, &ctx), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate4_rejects_event_after_cursor() {
        let mut e = sample_stored_event();
        e.hlc_ts = HlcTimestamp { phys: 200, log: 0 };
        let mut ctx = sample_ctx();
        ctx.as_of = Some(HlcTimestamp { phys: 100, log: 0 });
        assert_eq!(
            gate4_as_of(&e, &ctx),
            FilterVerdict::Reject(RejectReason::AsOfCursor)
        );
    }

    // ── Gate 5 · Status ──────────────────────────────────────────────────

    #[test]
    fn t11_gate5_accepts_active() {
        assert_eq!(gate5_status(EventStatus::Active), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate5_rejects_retracted() {
        assert_eq!(
            gate5_status(EventStatus::Retracted),
            FilterVerdict::Reject(RejectReason::EventStatusRetracted)
        );
    }

    #[test]
    fn t11_gate5_rejects_superseded() {
        assert_eq!(
            gate5_status(EventStatus::Superseded),
            FilterVerdict::Reject(RejectReason::EventStatusSuperseded)
        );
    }

    #[test]
    fn t11_extract_status_defaults_active_for_invalid_json() {
        assert_eq!(extract_status(b"not valid json"), EventStatus::Active);
        assert_eq!(extract_status(b"{}"), EventStatus::Active);
        assert_eq!(
            extract_status(br#"{"other":"retracted"}"#),
            EventStatus::Active
        );
        assert_eq!(
            extract_status(br#"{"status":"retracted"}"#),
            EventStatus::Retracted
        );
        assert_eq!(
            extract_status(br#"{"status":"superseded"}"#),
            EventStatus::Superseded
        );
    }
}

/// Gate 2 · Sensitivity ⊆ Capabilities. BOTH conditions must hold:
///   (a) event.sensitivity_tier <= capabilities.max_tier
///   (b) event.bundle_ids ∩ capabilities.accessible_bundles is non-empty
///
/// Spec §4 Q4.2 Gate 2: bundle intersection is NOT redundant with tier —
/// an Ordinary-tier event in a bundle the current identity cannot read
/// is still filtered out.
pub fn gate2_capability(event: &StoredEvent, caps: &CapabilitySet) -> FilterVerdict {
    if event.sensitivity_tier > caps.max_tier {
        return FilterVerdict::Reject(RejectReason::SensitivityExceedsCapability);
    }
    let intersects = event
        .bundle_ids
        .iter()
        .any(|b| caps.accessible_bundles.contains(b));
    if !intersects {
        return FilterVerdict::Reject(RejectReason::BundleNotInCapability);
    }
    FilterVerdict::Accept
}

/// Gate 3 · Owner State scope. Spec §4 Q4.2 Gate 3 table.
/// AbsentSoft read-only (current impl accepts all — orchestrator handles
/// "don't suggest proactive action events" at a higher layer).
pub fn gate3_owner_state(event: &StoredEvent, ctx: &QueryContext) -> FilterVerdict {
    match ctx.owner_state {
        OwnerState::Alive => FilterVerdict::Accept,
        OwnerState::AbsentSoft => FilterVerdict::Accept,
        OwnerState::AbsentHard => {
            // Only bundle:core + "continuation_mission:*" bundles (Phase D
            // Loyalty layer will resolve the real mission bundle list).
            let allowed = event.bundle_ids.iter().any(|b| {
                let s = b.as_str();
                s == "bundle:core" || s.starts_with("continuation_mission:")
            });
            if allowed {
                FilterVerdict::Accept
            } else {
                FilterVerdict::Reject(RejectReason::OwnerStateScope)
            }
        }
        OwnerState::Continuation => {
            let allowed = ctx.autonomy_grants.allowed_bundles();
            let intersects = event.bundle_ids.iter().any(|b| allowed.contains(b));
            if intersects {
                FilterVerdict::Accept
            } else {
                FilterVerdict::Reject(RejectReason::OwnerStateScope)
            }
        }
        OwnerState::SoulArchive => {
            // Phase C3: read-only + requires archive_reader capability.
            // Enforcement detail: this gate rejects everything; the
            // dedicated `read_constitution` / `read_autonomy_grants` APIs
            // serve archive queries. Test covers this.
            FilterVerdict::Reject(RejectReason::OwnerStateScope)
        }
    }
}

#[cfg(test)]
mod gate2_gate3_tests {
    use super::tests::{sample_ctx, sample_stored_event};
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::query::{AutonomyGrantProvider, CoreOnlyGrantProvider};
    use crate::heirloom::types::BundleId;
    use std::sync::Arc;

    // ── Gate 2 · Capability ──────────────────────────────────────────────

    #[test]
    fn t11_gate2_accepts_tier_and_bundle_both_ok() {
        let e = sample_stored_event();
        let caps = CapabilitySet::dev_default();
        assert_eq!(gate2_capability(&e, &caps), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate2_rejects_tier_exceeds_max() {
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Secret;
        let caps = CapabilitySet {
            max_tier: SensitivityTier::Ordinary,
            accessible_bundles: vec![BundleId::new("bundle:general")],
        };
        assert_eq!(
            gate2_capability(&e, &caps),
            FilterVerdict::Reject(RejectReason::SensitivityExceedsCapability)
        );
    }

    #[test]
    fn t11_gate2_rejects_when_bundle_intersection_empty_even_if_tier_allows() {
        // SEMANTIC-CRITICAL: spec §4 Q4.2 Gate 2 is AND, not OR.
        // Tier allowing is NOT sufficient — the bundle intersection must
        // also be non-empty. Otherwise an Ordinary-tier event in a bundle
        // the identity cannot decrypt would leak through.
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Ordinary;
        e.bundle_ids = vec![BundleId::new("bundle:health")];
        let caps = CapabilitySet {
            max_tier: SensitivityTier::Secret, // generous tier
            accessible_bundles: vec![BundleId::new("bundle:general")], // but NO health
        };
        assert_eq!(
            gate2_capability(&e, &caps),
            FilterVerdict::Reject(RejectReason::BundleNotInCapability),
            "Tier allowed but bundle mismatch must reject"
        );
    }

    #[test]
    fn t11_gate2_accepts_when_one_of_many_bundles_matches() {
        let mut e = sample_stored_event();
        e.bundle_ids = vec![
            BundleId::new("bundle:health"),
            BundleId::new("bundle:general"),
        ];
        let caps = CapabilitySet {
            max_tier: SensitivityTier::Secret,
            accessible_bundles: vec![BundleId::new("bundle:general")],
        };
        assert_eq!(gate2_capability(&e, &caps), FilterVerdict::Accept);
    }

    // ── Gate 3 · Owner State ─────────────────────────────────────────────

    #[test]
    fn t11_gate3_alive_accepts_anything() {
        let mut e = sample_stored_event();
        e.bundle_ids = vec![BundleId::new("bundle:anything")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::Alive;
        assert_eq!(gate3_owner_state(&e, &ctx), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate3_absent_hard_accepts_bundle_core() {
        let mut e = sample_stored_event();
        e.bundle_ids = vec![BundleId::new("bundle:core")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::AbsentHard;
        assert_eq!(gate3_owner_state(&e, &ctx), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate3_absent_hard_accepts_continuation_mission_prefix() {
        let mut e = sample_stored_event();
        e.bundle_ids = vec![BundleId::new("continuation_mission:finish_book")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::AbsentHard;
        assert_eq!(gate3_owner_state(&e, &ctx), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate3_absent_hard_rejects_other_bundles() {
        let mut e = sample_stored_event();
        e.bundle_ids = vec![BundleId::new("bundle:general")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::AbsentHard;
        assert_eq!(
            gate3_owner_state(&e, &ctx),
            FilterVerdict::Reject(RejectReason::OwnerStateScope)
        );
    }

    #[derive(Debug, Clone)]
    struct TwoBundleGrants;
    impl AutonomyGrantProvider for TwoBundleGrants {
        fn allowed_bundles(&self) -> Vec<BundleId> {
            vec![
                BundleId::new("bundle:core"),
                BundleId::new("continuation_mission:finish_book"),
            ]
        }
    }

    #[test]
    fn t11_gate3_continuation_consults_grant_provider() {
        // With CoreOnlyGrantProvider, continuation state accepts only
        // bundle:core. With a more permissive provider, it accepts that
        // provider's bundles too.
        let mut e_core = sample_stored_event();
        e_core.bundle_ids = vec![BundleId::new("bundle:core")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::Continuation;
        ctx.autonomy_grants = Arc::new(CoreOnlyGrantProvider);
        assert_eq!(gate3_owner_state(&e_core, &ctx), FilterVerdict::Accept);

        let mut e_general = sample_stored_event();
        e_general.bundle_ids = vec![BundleId::new("bundle:general")];
        assert_eq!(
            gate3_owner_state(&e_general, &ctx),
            FilterVerdict::Reject(RejectReason::OwnerStateScope),
            "CoreOnly provider rejects bundle:general"
        );

        // With a more permissive provider, bundle beyond core is allowed.
        let mut ctx2 = sample_ctx();
        ctx2.owner_state = OwnerState::Continuation;
        ctx2.autonomy_grants = Arc::new(TwoBundleGrants);
        let mut e_mission = sample_stored_event();
        e_mission.bundle_ids = vec![BundleId::new("continuation_mission:finish_book")];
        assert_eq!(gate3_owner_state(&e_mission, &ctx2), FilterVerdict::Accept);
    }

    #[test]
    fn t11_gate3_soul_archive_rejects_all_through_query() {
        let e = sample_stored_event();
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::SoulArchive;
        assert_eq!(
            gate3_owner_state(&e, &ctx),
            FilterVerdict::Reject(RejectReason::OwnerStateScope)
        );
    }
}

/// Gate 6 · Scene compatibility. Returns FilterVerdict with three-valued
/// outcome: Accept / SoftPenalty / Reject. SoftPenalty events proceed to
/// rank where `context_match()` applies a -0.2 scene_score.
///
/// Spec §4 Q4.2 Gate 6 + Round-9 P2.1.
pub fn gate6_scene(ctx_scene: Scene, event_scene: Scene) -> FilterVerdict {
    match (ctx_scene, event_scene) {
        (Scene::Work, Scene::Social) | (Scene::Work, Scene::Creative) => {
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        }
        (Scene::Debug, Scene::Social) | (Scene::Debug, Scene::Personal) => {
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        }
        (Scene::Crisis, _) => FilterVerdict::Accept,
        (_, Scene::Unknown) => FilterVerdict::Accept,
        (a, b) if a == b => FilterVerdict::Accept,
        _ => FilterVerdict::SoftPenalty(SoftPenaltyReason::SceneMismatch),
    }
}

#[cfg(test)]
mod gate6_tests {
    use super::*;

    #[test]
    fn t11_gate6_work_rejects_social_and_creative() {
        assert_eq!(
            gate6_scene(Scene::Work, Scene::Social),
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        );
        assert_eq!(
            gate6_scene(Scene::Work, Scene::Creative),
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        );
    }

    #[test]
    fn t11_gate6_debug_rejects_social_and_personal() {
        assert_eq!(
            gate6_scene(Scene::Debug, Scene::Social),
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        );
        assert_eq!(
            gate6_scene(Scene::Debug, Scene::Personal),
            FilterVerdict::Reject(RejectReason::SceneIncompatible)
        );
    }

    #[test]
    fn t11_gate6_crisis_accepts_anything() {
        for s in [
            Scene::Work,
            Scene::Personal,
            Scene::Debug,
            Scene::Social,
            Scene::Health,
            Scene::Financial,
            Scene::Learning,
            Scene::Creative,
            Scene::Crisis,
            Scene::Unknown,
        ] {
            assert_eq!(gate6_scene(Scene::Crisis, s), FilterVerdict::Accept);
        }
    }

    #[test]
    fn t11_gate6_unknown_event_scene_always_accepts() {
        // event_scene = Unknown always accepts regardless of ctx_scene.
        for s in [
            Scene::Work,
            Scene::Personal,
            Scene::Debug,
            Scene::Social,
            Scene::Health,
            Scene::Financial,
            Scene::Learning,
            Scene::Creative,
        ] {
            assert_eq!(gate6_scene(s, Scene::Unknown), FilterVerdict::Accept);
        }
    }

    #[test]
    fn t11_gate6_same_scene_accepts() {
        for s in [
            Scene::Work,
            Scene::Personal,
            Scene::Health,
            Scene::Financial,
            Scene::Learning,
        ] {
            assert_eq!(gate6_scene(s, s), FilterVerdict::Accept);
        }
    }

    #[test]
    fn t11_gate6_other_mismatches_soft_penalty() {
        // Personal vs Financial → neither the rejected Work/Debug combos
        // nor a matching pair nor Unknown — expected SoftPenalty.
        assert_eq!(
            gate6_scene(Scene::Personal, Scene::Financial),
            FilterVerdict::SoftPenalty(SoftPenaltyReason::SceneMismatch)
        );
        assert_eq!(
            gate6_scene(Scene::Learning, Scene::Health),
            FilterVerdict::SoftPenalty(SoftPenaltyReason::SceneMismatch)
        );
    }

    #[test]
    fn t11_gate6_soft_penalty_flows_into_ctx_match_not_filter() {
        // SEMANTIC-CRITICAL (Round-9 P2.1 guard — behavioral):
        // Gate 6 mismatches that are not in the Reject pair-list MUST
        // produce SoftPenalty (a NON-filter verdict). The rank-stage
        // context_match() then consumes the soft flag to apply -0.2.
        //
        // This test asserts *here* that the Verdict variant is SoftPenalty
        // and NOT Reject — if a future refactor makes this branch Reject,
        // rank loses its scene signal and the plan's
        // t11_context_match_scene_soft_penalty_adds_negative_0_2 (Task 8)
        // will start failing as well. Two gates at different layers.
        let v = gate6_scene(Scene::Personal, Scene::Financial);
        match v {
            FilterVerdict::SoftPenalty(SoftPenaltyReason::SceneMismatch) => {}
            other => panic!(
                "expected SoftPenalty(SceneMismatch), got {other:?} — Gate 6 must \
                 never Reject for non-listed mismatch pairs; that would break \
                 Round-9 P2.1 rank-stage scene_score flow"
            ),
        }
    }
}

/// Outcome of running all 6 gates in order. If any gate Rejects, returns
/// the first rejection. If any gate returns SoftPenalty, the outcome is
/// `Accept { soft_penalty: true }`; callers use this to adjust rank scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    Accept { soft_penalty: bool },
    Reject(RejectReason),
}

/// Run all 6 gates in their fixed order:
///   Gate 1 Sacred → Gate 2 Capability → Gate 3 OwnerState →
///   Gate 4 As-of → Gate 5 Status → Gate 6 Scene
///
/// Evaluates gates one at a time with short-circuit on first Reject. This
/// matters when gate3_owner_state becomes a real DB-backed
/// AutonomyGrantProvider call in Phase D (D7) — eagerly evaluating all 6
/// verdicts would waste a DB round-trip per rejected event.
/// SoftPenalty (Gate 6 only in Phase C3) does not terminate; it sets
/// `soft = true` and the pipeline returns Accept { soft_penalty: true }.
pub fn apply_all_gates(
    event: &StoredEvent,
    ctx: &QueryContext,
    status: EventStatus,
) -> GateOutcome {
    let mut soft = false;

    // Macro-style short-circuit: evaluate, dispatch, continue.
    macro_rules! check {
        ($verdict:expr) => {
            match $verdict {
                FilterVerdict::Accept => {}
                FilterVerdict::SoftPenalty(_) => soft = true,
                FilterVerdict::Reject(r) => return GateOutcome::Reject(r),
            }
        };
    }

    check!(gate1_sacred(event));
    check!(gate2_capability(event, &ctx.capabilities));
    check!(gate3_owner_state(event, ctx));
    check!(gate4_as_of(event, ctx));
    check!(gate5_status(status));
    check!(gate6_scene(ctx.scene, event.scene_tag));

    GateOutcome::Accept { soft_penalty: soft }
}

#[cfg(test)]
mod pipeline_tests {
    use super::tests::{sample_ctx, sample_stored_event};
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::types::BundleId;

    #[test]
    fn t11_pipeline_accepts_when_all_gates_pass() {
        let e = sample_stored_event();
        let ctx = sample_ctx();
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Active),
            GateOutcome::Accept {
                soft_penalty: false
            }
        );
    }

    #[test]
    fn t11_pipeline_short_circuits_on_first_reject() {
        // Sacred event with ANY other problem — Sacred wins.
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Sacred;
        let ctx = sample_ctx();
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Retracted),
            GateOutcome::Reject(RejectReason::Sacred)
        );
    }

    #[test]
    fn t11_pipeline_preserves_soft_penalty_across_accepts() {
        // Non-matching scene (Personal vs Financial) → SoftPenalty at
        // Gate 6; all other gates accept → Accept { soft_penalty: true }.
        let mut e = sample_stored_event();
        e.scene_tag = Scene::Financial;
        let mut ctx = sample_ctx();
        ctx.scene = Scene::Personal;
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Active),
            GateOutcome::Accept { soft_penalty: true }
        );
    }

    #[test]
    fn t11_pipeline_order_sacred_before_capability() {
        // Sacred + mismatched capability — Sacred must be reported
        // because Gate 1 runs first.
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Sacred;
        e.bundle_ids = vec![BundleId::new("bundle:nowhere")];
        let ctx = sample_ctx();
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Active),
            GateOutcome::Reject(RejectReason::Sacred)
        );
    }

    #[test]
    fn t11_pipeline_order_capability_before_owner_state() {
        // Capability mismatch + SoulArchive owner state — Capability
        // must be reported (Gate 2 before Gate 3).
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Secret;
        e.bundle_ids = vec![BundleId::new("bundle:nowhere")];
        let mut ctx = sample_ctx();
        ctx.owner_state = OwnerState::SoulArchive;
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Active),
            GateOutcome::Reject(RejectReason::BundleNotInCapability)
        );
    }

    #[test]
    fn t11_filter_gate_order_is_fixed() {
        // SEMANTIC-CRITICAL: gate order is load-bearing per spec §4 Q4.2.
        // Cheapest + strictest first. Regression guard: the array literal in
        // apply_all_gates must stay in this exact order. Any reordering
        // changes observable behavior for SoftPenalty events (later-gate
        // rejections hide earlier-gate rejections).
        //
        // This test doesn't (and can't) introspect Rust code order — it
        // asserts behavior: a single event that fails Sacred AND Capability
        // reports Sacred. A single event that fails Capability AND
        // OwnerState reports Capability. Etc.

        // Event: Sacred tier + mismatched capability → Sacred wins.
        let mut e = sample_stored_event();
        e.sensitivity_tier = SensitivityTier::Sacred;
        e.bundle_ids = vec![BundleId::new("bundle:no-match")];
        let ctx = sample_ctx();
        assert_eq!(
            apply_all_gates(&e, &ctx, EventStatus::Active),
            GateOutcome::Reject(RejectReason::Sacred)
        );

        // Event: mismatched capability + mismatched owner-state → Capability.
        let mut e2 = sample_stored_event();
        e2.sensitivity_tier = SensitivityTier::Secret;
        e2.bundle_ids = vec![BundleId::new("bundle:no-match")];
        let mut ctx2 = sample_ctx();
        ctx2.owner_state = OwnerState::SoulArchive;
        assert_eq!(
            apply_all_gates(&e2, &ctx2, EventStatus::Active),
            GateOutcome::Reject(RejectReason::BundleNotInCapability),
            "Capability must be checked before OwnerState"
        );

        // Event: SoulArchive (Gate 3) + future HLC (Gate 4) → Gate 3 wins.
        let mut e3 = sample_stored_event();
        e3.hlc_ts = HlcTimestamp {
            phys: 1_000_000,
            log: 0,
        };
        let mut ctx3 = sample_ctx();
        ctx3.owner_state = OwnerState::SoulArchive;
        ctx3.as_of = Some(HlcTimestamp { phys: 50, log: 0 });
        assert_eq!(
            apply_all_gates(&e3, &ctx3, EventStatus::Active),
            GateOutcome::Reject(RejectReason::OwnerStateScope),
            "OwnerState (Gate 3) must be checked before As-of (Gate 4)"
        );
    }
}
