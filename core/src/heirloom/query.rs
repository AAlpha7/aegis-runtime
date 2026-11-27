//! QueryContext + supporting enums/structs for the heirloom read path.
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §4 Q4.1.
//!
//! Two construction helpers enforce the Round-9 P2.2 invariant: `as_of`
//! queries MUST NOT bump Hebbian counts. `new_current()` sets
//! `update_hebbian = true` + `as_of = None`; `new_as_of()` forces
//! `update_hebbian = false` regardless of what the caller asks for.

use crate::heirloom::enums::SensitivityTier;
use crate::heirloom::hlc::HlcTimestamp;
use crate::heirloom::types::{BundleId, Did, LineageId, WitnessChannel};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Structural intent category enum. String→enum mapping happens ONCE in
/// `parse()` — per feedback_no_static_matching, downstream code must
/// match on the enum variant, NOT substring-match the wrapped string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentCategory {
    Social,
    Health,
    Financial,
    Learning,
    Creative,
    TechnicalError,
    Debug,
    Other,
}

impl IntentCategory {
    /// The ONE place where we string-match intent names. All external
    /// producers (orchestrator intent.rs, tests) hit this function to
    /// project into the enum. Unknown inputs map to `Other`.
    ///
    /// # Precedence — DO NOT reorder without a regression test
    ///
    /// Order matters: `ends_with(':debug')` fires BEFORE `starts_with('social_')`
    /// (etc.) because `social:debug` legitimately classifies as `Debug` — a
    /// developer suppressing scene inference for a social-channel message must
    /// resolve to Debug, not Social. The current arm order is:
    ///
    /// 1. `technical_error` exact match → `TechnicalError`
    /// 2. `ends_with(':debug')` OR `"debug"` → `Debug`
    /// 3. `social_*` / `"social"` → `Social`
    /// 4. `health_*` / `"health"` → `Health`
    /// 5. `finance_*` / `financial_*` / `"financial"` → `Financial`
    /// 6. `learning_*` / `"learning"` → `Learning`
    /// 7. `creative_*` / `"creative"` → `Creative`
    /// 8. fall-through → `Other`
    pub fn parse(s: &str) -> IntentCategory {
        // Safety arms first — Debug / TechnicalError suppress productive scenes.
        if s == "technical_error" {
            return IntentCategory::TechnicalError;
        }
        if s.ends_with(":debug") || s == "debug" {
            return IntentCategory::Debug;
        }
        // Productive categories — prefix-based structural map.
        if s.starts_with("social_") || s == "social" {
            return IntentCategory::Social;
        }
        if s.starts_with("health_") || s == "health" {
            return IntentCategory::Health;
        }
        if s.starts_with("finance_") || s.starts_with("financial_") || s == "financial" {
            return IntentCategory::Financial;
        }
        if s.starts_with("learning_") || s == "learning" {
            return IntentCategory::Learning;
        }
        if s.starts_with("creative_") || s == "creative" {
            return IntentCategory::Creative;
        }
        IntentCategory::Other
    }
}

/// Intent wrapper — enum with an `Other(String)` arm to keep the legacy
/// free-form string producers working. Scene judgement matches on the
/// `Category` variant only; `Other` is opaque to rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IntentClass {
    Category(IntentCategory),
    Other(String),
}

impl IntentClass {
    /// Backward-compat constructor: string → IntentClass. Delegates to
    /// IntentCategory::parse; if parsing yields `Other`, the original
    /// string is captured in the Other arm for audit/debug.
    pub fn new(s: impl Into<String>) -> Self {
        let raw = s.into();
        match IntentCategory::parse(&raw) {
            IntentCategory::Other => IntentClass::Other(raw),
            known => IntentClass::Category(known),
        }
    }

    /// For legacy call-sites that expected a string back. Tests / tracing.
    pub fn as_str(&self) -> &str {
        match self {
            IntentClass::Category(c) => match c {
                IntentCategory::Social => "social",
                IntentCategory::Health => "health",
                IntentCategory::Financial => "financial",
                IntentCategory::Learning => "learning",
                IntentCategory::Creative => "creative",
                IntentCategory::TechnicalError => "technical_error",
                IntentCategory::Debug => "debug",
                IntentCategory::Other => "other",
            },
            IntentClass::Other(s) => s.as_str(),
        }
    }
}

/// A reference to an entity mentioned in event content. Phase C3 accepts
/// the Consolidate-LLM persisted shape; Phase D wires the producer.
/// Until then, all events have `entity_refs = vec![]` (D3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    /// Either a DID (for known entities) or a raw name string.
    pub id: String,
    /// Optional kind tag — "person", "org", "place", "project", etc.
    pub kind: Option<String>,
}

impl EntityRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: None,
        }
    }
}

/// Scene tag (spec §3 Q3.2). Deterministic rules produce this in the
/// orchestrator at query time (Task 2). Event-side scene is persisted
/// per event (Phase D Consolidate); Phase C3 reads it from payload if
/// present, else `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    Work,
    Personal,
    Debug,
    Social,
    Health,
    Financial,
    Learning,
    Creative,
    Crisis,
    Unknown,
}

/// Temporal class (spec §3 Q3.2). Determines staleness half-life in the
/// rank formula. Default = `Slow` when payload doesn't carry the field (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemporalClass {
    Timeless,
    #[default]
    Slow,
    Ephemeral,
    Critical,
}

/// Owner state (imported from heirloom::types, re-exported here for
/// convenience at query call-sites).
pub use crate::heirloom::types::OwnerState;

/// Capability set: what sensitivity tier the current identity can decrypt
/// AND which bundles they can access. Both conditions MUST hold for Gate 2
/// (spec §4 Q4.2 Gate 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySet {
    /// Maximum sensitivity tier this identity can read. Inclusive.
    pub max_tier: SensitivityTier,
    /// Bundles this identity is authorized to access. Empty vec means
    /// "no bundle access" — which will reject everything in Gate 2.
    pub accessible_bundles: Vec<BundleId>,
}

impl CapabilitySet {
    /// Permissive dev default: tier up to `Secret` (not Sacred), bundle
    /// access for `bundle:core` + `bundle:general`. Never use in prod.
    pub fn dev_default() -> Self {
        Self {
            max_tier: SensitivityTier::Secret,
            accessible_bundles: vec![
                BundleId::new("bundle:core"),
                BundleId::new("bundle:general"),
            ],
        }
    }

    /// Restrictive default for Continuation / archive states.
    pub fn archive_reader() -> Self {
        Self {
            max_tier: SensitivityTier::Confidential,
            accessible_bundles: vec![BundleId::new("bundle:core")],
        }
    }
}

/// Stub trait for Phase D Loyalty layer integration (D7). Gate 3 calls
/// `allowed_bundles()` only when `owner_state == Continuation`.
pub trait AutonomyGrantProvider: Send + Sync + std::fmt::Debug {
    fn allowed_bundles(&self) -> Vec<BundleId>;
}

/// Phase C3 default impl: allows only `bundle:core`. Phase D replaces
/// with a real store-backed impl reading live grants.
#[derive(Debug, Clone, Default)]
pub struct CoreOnlyGrantProvider;

impl AutonomyGrantProvider for CoreOnlyGrantProvider {
    fn allowed_bundles(&self) -> Vec<BundleId> {
        vec![BundleId::new("bundle:core")]
    }
}

/// Query context. Owned by the caller; passed by ref to `HeirloomReader::query()`.
#[derive(Debug, Clone)]
pub struct QueryContext {
    // When
    pub now: DateTime<Utc>,
    pub as_of: Option<HlcTimestamp>,

    // Who
    pub querying_did: Did,
    pub owner_state: OwnerState,
    pub lineage_id: LineageId,

    // Where
    pub channel: WitnessChannel,
    pub scene: Scene,

    // What
    pub recent_intents: Vec<IntentClass>,
    pub active_entities: Vec<EntityRef>,
    pub query_text: Option<String>,
    pub query_embedding: Option<Vec<f32>>,

    // Budget
    pub capabilities: CapabilitySet,
    pub token_budget: usize,
    pub max_results: usize,

    // Side effects
    pub update_hebbian: bool,

    // Grant provider (D7)
    pub autonomy_grants: Arc<dyn AutonomyGrantProvider>,
}

impl QueryContext {
    /// Standard current-time query. `update_hebbian` defaults to true.
    #[allow(clippy::too_many_arguments)]
    pub fn new_current(
        now: DateTime<Utc>,
        querying_did: Did,
        owner_state: OwnerState,
        lineage_id: LineageId,
        channel: WitnessChannel,
        scene: Scene,
        capabilities: CapabilitySet,
        token_budget: usize,
        max_results: usize,
        autonomy_grants: Arc<dyn AutonomyGrantProvider>,
    ) -> Self {
        Self {
            now,
            as_of: None,
            querying_did,
            owner_state,
            lineage_id,
            channel,
            scene,
            recent_intents: vec![],
            active_entities: vec![],
            query_text: None,
            query_embedding: None,
            capabilities,
            token_budget,
            max_results,
            update_hebbian: true,
            autonomy_grants,
        }
    }

    /// Time-travel query. ALWAYS forces `update_hebbian = false` — the
    /// Round-9 P2.2 invariant. The caller cannot override this.
    #[allow(clippy::too_many_arguments)]
    pub fn new_as_of(
        cursor: HlcTimestamp,
        now: DateTime<Utc>,
        querying_did: Did,
        owner_state: OwnerState,
        lineage_id: LineageId,
        channel: WitnessChannel,
        scene: Scene,
        capabilities: CapabilitySet,
        token_budget: usize,
        max_results: usize,
        autonomy_grants: Arc<dyn AutonomyGrantProvider>,
    ) -> Self {
        Self {
            now,
            as_of: Some(cursor),
            querying_did,
            owner_state,
            lineage_id,
            channel,
            scene,
            recent_intents: vec![],
            active_entities: vec![],
            query_text: None,
            query_embedding: None,
            capabilities,
            token_budget,
            max_results,
            update_hebbian: false,
            autonomy_grants,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn basic_ctx_current() -> QueryContext {
        QueryContext::new_current(
            fixed_now(),
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

    #[test]
    fn t11_query_new_current_defaults_hebbian_on_and_as_of_none() {
        let ctx = basic_ctx_current();
        assert!(ctx.update_hebbian);
        assert!(ctx.as_of.is_none());
    }

    #[test]
    fn t11_query_new_as_of_forces_update_hebbian_false() {
        // SEMANTIC-CRITICAL: time-travel queries must NEVER bump Hebbian
        // counts — Round-9 P2.2 invariant. Even a later field mutation
        // shouldn't flip it, but new_as_of() is the only correct constructor.
        let cursor = HlcTimestamp { phys: 1, log: 0 };
        let ctx = QueryContext::new_as_of(
            cursor,
            fixed_now(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Work,
            CapabilitySet::dev_default(),
            2048,
            20,
            Arc::new(CoreOnlyGrantProvider),
        );
        assert!(
            !ctx.update_hebbian,
            "as_of MUST force update_hebbian = false"
        );
        assert_eq!(ctx.as_of, Some(cursor));
    }

    #[test]
    fn t11_capability_set_dev_default_tier_is_secret_not_sacred() {
        let cs = CapabilitySet::dev_default();
        assert_eq!(cs.max_tier, SensitivityTier::Secret);
        // Sacred must NEVER be in a CapabilitySet's max_tier — spec §4 Gate 1.
        assert!(cs.max_tier < SensitivityTier::Sacred);
    }

    #[test]
    fn t11_capability_set_archive_reader_has_only_bundle_core() {
        let cs = CapabilitySet::archive_reader();
        assert_eq!(cs.accessible_bundles.len(), 1);
        assert_eq!(cs.accessible_bundles[0].as_str(), "bundle:core");
    }

    #[test]
    fn t11_temporal_class_default_is_slow() {
        assert_eq!(TemporalClass::default(), TemporalClass::Slow);
    }

    #[test]
    fn t11_core_only_grant_provider_returns_bundle_core() {
        let g = CoreOnlyGrantProvider;
        let allowed = g.allowed_bundles();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].as_str(), "bundle:core");
    }

    #[test]
    fn t11_scene_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Scene::Work).unwrap(), r#""work""#);
        assert_eq!(
            serde_json::to_string(&Scene::Crisis).unwrap(),
            r#""crisis""#
        );
    }
}

impl Scene {
    /// Phase C3 deterministic scene judgement per spec §3 Q3.2. Orchestrator
    /// calls this to populate `QueryContext.scene` at query time. Zero LLM.
    ///
    /// Rule priority (first match wins):
    ///   1. is_crisis flag                     → Crisis
    ///   2. channel == Voice && 22:00 <= hr || hr < 06:00 → Personal
    ///   3. recent_intents contains "technical_error" (or "debug" suffix) → Debug
    ///   4. 09:00 <= hr <= 18:00 && weekday    → Work
    ///   5. else                               → Unknown
    ///
    /// Spec rule about health/finance keywords uses the payload path; that
    /// lives on the event side (Phase D Consolidate), not at query time.
    ///
    /// Note (deviation from plan Task 2): the plan calls for the 4th arg to
    /// be `owner_state: OwnerState` and tests `OwnerState::Crisis`, but
    /// Phase A's `OwnerState` is spec-locked to exactly 5 variants (Alive /
    /// AbsentSoft / AbsentHard / Continuation / SoulArchive) with no
    /// `Crisis`. Rather than break the Phase A spec-literal type, Phase C3
    /// lifts the crisis signal to a `is_crisis: bool` parameter. The
    /// orchestrator computes it from whatever primary signal (e.g. an
    /// explicit crisis flag in the session, AbsentHard transition, ethics
    /// escalation). This is a pure API-shape change; the semantic rule
    /// ("Crisis trumps all other rules") is preserved verbatim.
    pub fn judge_deterministic(
        now: DateTime<Utc>,
        channel: WitnessChannel,
        recent_intents: &[IntentClass],
        is_crisis: bool,
    ) -> Scene {
        use chrono::{Datelike, Timelike, Weekday};

        if is_crisis {
            return Scene::Crisis;
        }

        let hour = now.hour();
        if channel == WitnessChannel::Voice && !(6..22).contains(&hour) {
            return Scene::Personal;
        }

        // Structural intent-based scene detection. Priority per Gotcha #9:
        // TechnicalError/Debug → Debug (safety first, debug signals override
        //   all productive categories)
        // Financial → Financial (most audit-sensitive)
        // Health → Health
        // Learning → Learning
        // Creative → Creative
        // Social → Social
        // (fall-through to time-based rules)
        let cat = recent_intents.iter().find_map(|i| match i {
            IntentClass::Category(c) => Some(*c),
            IntentClass::Other(_) => None,
        });
        match cat {
            Some(IntentCategory::TechnicalError) | Some(IntentCategory::Debug) => {
                return Scene::Debug;
            }
            Some(IntentCategory::Financial) => return Scene::Financial,
            Some(IntentCategory::Health) => return Scene::Health,
            Some(IntentCategory::Learning) => return Scene::Learning,
            Some(IntentCategory::Creative) => return Scene::Creative,
            Some(IntentCategory::Social) => return Scene::Social,
            Some(IntentCategory::Other) | None => {}
        }

        let is_weekday = !matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
        if is_weekday && (9..=18).contains(&hour) {
            return Scene::Work;
        }

        Scene::Unknown
    }
}

#[cfg(test)]
mod scene_judgement_tests {
    use super::*;
    use chrono::TimeZone;

    fn noon_weekday() -> DateTime<Utc> {
        // 2026-04-20 is a Monday (ISO weekday = 1)
        Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap()
    }
    fn three_am() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 20, 3, 0, 0).unwrap()
    }

    #[test]
    fn t11_scene_judge_crisis_owner_state_wins() {
        let s = Scene::judge_deterministic(
            noon_weekday(),
            WitnessChannel::Telegram,
            &[],
            true, // is_crisis
        );
        assert_eq!(s, Scene::Crisis);
    }

    #[test]
    fn t11_scene_judge_voice_at_night_is_personal() {
        let s = Scene::judge_deterministic(
            three_am(),
            WitnessChannel::Voice,
            &[],
            false, // is_crisis
        );
        assert_eq!(s, Scene::Personal);
    }

    #[test]
    fn t11_scene_judge_weekday_noon_defaults_to_work() {
        let s = Scene::judge_deterministic(
            noon_weekday(),
            WitnessChannel::Mcp,
            &[],
            false, // is_crisis
        );
        assert_eq!(s, Scene::Work);
    }

    #[test]
    fn t11_scene_judge_recent_technical_intent_is_debug() {
        let intents = vec![IntentClass::new("technical_error")];
        let s = Scene::judge_deterministic(
            noon_weekday(),
            WitnessChannel::Mcp,
            &intents,
            false, // is_crisis
        );
        assert_eq!(s, Scene::Debug);
    }

    #[test]
    fn t11_scene_judge_unknown_fallback() {
        // Saturday 3am, non-voice channel, no useful intent → Unknown.
        let sat_3am = Utc.with_ymd_and_hms(2026, 4, 18, 3, 0, 0).unwrap();
        let s = Scene::judge_deterministic(
            sat_3am,
            WitnessChannel::Mcp,
            &[],
            false, // is_crisis
        );
        assert_eq!(s, Scene::Unknown);
    }

    #[test]
    fn t11_scene_judge_owner_state_wins_over_weekday_rule() {
        // Even noon weekday, Crisis trumps Work.
        let s = Scene::judge_deterministic(
            noon_weekday(),
            WitnessChannel::Mcp,
            &[IntentClass::new("technical_error")],
            true, // is_crisis
        );
        assert_eq!(s, Scene::Crisis);
    }

    // ── Phase D Task 8/9: scene completion + IntentCategory ───────────────

    #[test]
    fn t_d5_scene_deterministic_all_10_scenes_have_positive_case() {
        // SEMANTIC-CRITICAL (#5): every Scene variant reachable via some
        // combination of inputs. Regression pin.
        use IntentCategory::*;
        let nw = noon_weekday();
        let three = three_am();
        let sat_3am = Utc.with_ymd_and_hms(2026, 4, 18, 3, 0, 0).unwrap();

        // 1. Crisis
        assert_eq!(
            Scene::judge_deterministic(nw, WitnessChannel::Telegram, &[], true),
            Scene::Crisis
        );
        // 2. Personal (voice+night)
        assert_eq!(
            Scene::judge_deterministic(three, WitnessChannel::Voice, &[], false),
            Scene::Personal
        );
        // 3. Debug
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(TechnicalError)],
                false
            ),
            Scene::Debug
        );
        // 4. Financial
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(Financial)],
                false
            ),
            Scene::Financial
        );
        // 5. Health
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(Health)],
                false
            ),
            Scene::Health
        );
        // 6. Learning
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(Learning)],
                false
            ),
            Scene::Learning
        );
        // 7. Creative
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(Creative)],
                false
            ),
            Scene::Creative
        );
        // 8. Social
        assert_eq!(
            Scene::judge_deterministic(
                nw,
                WitnessChannel::Mcp,
                &[IntentClass::Category(Social)],
                false
            ),
            Scene::Social
        );
        // 9. Work (weekday noon, no specific intent)
        assert_eq!(
            Scene::judge_deterministic(nw, WitnessChannel::Mcp, &[], false),
            Scene::Work
        );
        // 10. Unknown (weekend 3am, no intent)
        assert_eq!(
            Scene::judge_deterministic(sat_3am, WitnessChannel::Mcp, &[], false),
            Scene::Unknown
        );
    }

    #[test]
    fn t_d5_financial_beats_work_priority() {
        // A financial question at weekday noon is Financial, not Work —
        // intent-derived scenes take priority over time-derived (Gotcha #9).
        let t = Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap();
        assert_eq!(
            Scene::judge_deterministic(
                t,
                WitnessChannel::Mcp,
                &[IntentClass::Category(IntentCategory::Financial)],
                false
            ),
            Scene::Financial
        );
    }

    #[test]
    fn t_d5_crisis_beats_financial_priority() {
        let t = Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap();
        assert_eq!(
            Scene::judge_deterministic(
                t,
                WitnessChannel::Mcp,
                &[IntentClass::Category(IntentCategory::Financial)],
                true
            ),
            Scene::Crisis
        );
    }

    #[test]
    fn t_d5_intent_category_parse_covers_known_strings() {
        assert_eq!(IntentCategory::parse("social_chat"), IntentCategory::Social);
        assert_eq!(
            IntentCategory::parse("health_question"),
            IntentCategory::Health
        );
        assert_eq!(
            IntentCategory::parse("finance_advice"),
            IntentCategory::Financial
        );
        assert_eq!(
            IntentCategory::parse("learning_topic"),
            IntentCategory::Learning
        );
        assert_eq!(
            IntentCategory::parse("creative_brainstorm"),
            IntentCategory::Creative
        );
        assert_eq!(
            IntentCategory::parse("technical_error"),
            IntentCategory::TechnicalError
        );
        assert_eq!(IntentCategory::parse("foo:debug"), IntentCategory::Debug);
        assert_eq!(
            IntentCategory::parse("completely_unrecognized"),
            IntentCategory::Other
        );
    }

    #[test]
    fn t_d5_intent_class_backward_compat_new() {
        // IntentClass::new("technical_error") still works and yields Category(TechnicalError).
        let i = IntentClass::new("technical_error");
        assert!(matches!(
            i,
            IntentClass::Category(IntentCategory::TechnicalError)
        ));
    }

    #[test]
    fn t_d5_intent_category_parse_precedence_debug_beats_social() {
        // Precedence pin (FIX 8): `social:debug` must classify as Debug, NOT Social.
        // If someone reorders parse() arms, this catches it immediately.
        assert_eq!(IntentCategory::parse("social:debug"), IntentCategory::Debug);
        assert_eq!(
            IntentCategory::parse("finance:debug"),
            IntentCategory::Debug
        );
        assert_eq!(IntentCategory::parse("health:debug"), IntentCategory::Debug);
        // Plain social_* (no :debug suffix) still classifies as Social.
        assert_eq!(IntentCategory::parse("social_chat"), IntentCategory::Social);
    }
}
