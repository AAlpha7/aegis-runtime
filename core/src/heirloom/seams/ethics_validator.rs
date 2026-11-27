//! W6 — EthicsValidator trait + full type surface.
//!
//! Spec §5 Q5.5 (2026-04-17 round 6). This file defines the trait and every
//! value type used in its signatures; dev impl `PermissiveEthics` lands in
//! Task 12. Prod impl `CanonicalEthics` (P4) combines LLM + L2 reference passage
//! retrieval + three-lens voting.
//!
//! Key design invariants locked at round-8:
//! - Three-lens verdict is observable (`evaluate_per_lens` returns `[LensVerdict; 3]`)
//! - Retrospective verdicts (`AllowWithGuidance { retrospective: true }`) distinct from prospective
//! - Penance channel (`RecordWithPenanceMarker`) for lesser offenses
//! - Escalation fallback for Continuation state (owner unreachable)
//! - Canon-anchoring: `EthicsContext` does NOT carry `derived_rules` or `recent_events`
//! - Target audience typed (SelfOwner / OtherSentient / SentientPopulation / Insentient)
//! - Current sensitivity tier influences verdict stringency

use serde::{Deserialize, Serialize};

use crate::heirloom::enums::SensitivityTier;
use crate::heirloom::types::{AutonomyGrant, Did, OwnerState, ProposedAction, WitnessChannel};

// ── Lens / verdict types ──────────────────────────────────────────────────

/// Part of the L2 reference corpus the lens reads from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Lens {
    /// Primary texts of the L2 reference corpus.
    PrimaryTexts,
    /// Rules of conduct in the L2 reference corpus.
    ConductRules,
    /// Commentaries on the primary texts and rules.
    Commentaries,
}

/// A single lens's judgment on an action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LensJudgment {
    Allow,
    Reject,
    /// Lens has no applicable passage; declines to rule.
    Abstain,
}

/// Verdict from a single lens with its citations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensVerdict {
    pub lens: Lens,
    pub judgment: LensJudgment,
    pub passages: Vec<ReferencePassage>,
    pub reason: String,
}

/// Final aggregate verdict across all lenses. Write path reads this and
/// either appends the event, records it with penance marker, rejects it,
/// or escalates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum EthicsVerdict {
    Allow,
    /// Allow but record caveat / guidance with the event. `retrospective`
    /// distinguishes post-hoc moral annotation (penance or commendation) from
    /// forward-looking guidance attached to a forthcoming action.
    AllowWithGuidance {
        passages: Vec<ReferencePassage>,
        note: String,
        retrospective: bool,
    },
    Reject {
        passages: Vec<ReferencePassage>,
        reason: String,
        reject_event_type: RejectEventType,
    },
    /// Action is possible to record but lands in the penance channel
    /// (lesser offenses). Not used by the hard-reject rules.
    RecordWithPenanceMarker {
        passages: Vec<ReferencePassage>,
        penance_weight: PenanceWeight,
        guidance: String,
    },
    /// Ask the owner. For Continuation-state flows, `fallback` determines
    /// default behavior when the owner is unreachable.
    EscalateToOwner {
        passages: Vec<ReferencePassage>,
        question: String,
        fallback: EscalationFallback,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PenanceWeight {
    Light,
    Middle,
    Grave,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EscalationFallback {
    /// Owner unreachable -> default reject (conservative).
    DefaultReject,
    /// Owner unreachable -> allow but alert on next reachable moment.
    DefaultAllowWithAlert,
    /// Owner unreachable -> defer to the next Great Reading (every 25y).
    DeferUntilGreatReading,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RejectEventType {
    /// The 3 hard rejects from 宪法 9.e.
    HardReject9E,
    /// Contradicts L0 (the base principles text).
    ContradictsL0,
    /// Contradicts L1 (Core Soul).
    ContradictsL1,
    /// Contradicts L2 reference passages.
    ContradictsL2,
    /// Proposed Derived Rule fails canon-anchoring check.
    RuleProposalRejected,
}

// ── Reference passage types ─ ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferencePassage {
    pub source: PassageSource,
    /// Human-readable citation like "Commentary A, chapter 20" or "Core Soul sentence 2".
    pub citation: String,
    /// Excerpted passage text.
    pub text: String,
    /// Edition tag of the cited text, e.g. "edition-2026".
    pub version: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PassageSource {
    Layer0,
    Layer1,
    PrimaryTexts,
    ConductRules,
    Commentaries,
}

// ── Context types ─────────────────────────────────────────────────────────

/// L2 manifest pointer — merkle root + passage index locations. Phase B
/// stores just the hash + a human tag; full manifest merkleization lands
/// in Phase D alongside L2 corpus ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct L2Manifest {
    #[serde(with = "crate::heirloom::types::hex_32")]
    pub merkle_root: [u8; 32],
    /// Human-readable version tag, e.g. `"l2-corpus-rev-2026Q1"`.
    pub version_tag: String,
}

/// Edition policy — which versions of canonical texts this eval uses.
/// Round-8 F3.2. Phase B only carries the single current version; P4 adds
/// cross-edition comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditionPolicy {
    /// Primary edition tag to use for this eval.
    pub primary: String,
    /// Secondary editions to cross-check against. Empty in Phase B.
    pub secondaries: Vec<String>,
}

/// Target audience of a proposed action. Drives default-stringency:
/// SelfOwner / OtherSentient both fall under ethics; Insentient falls under
/// 敬事 (reverence) rules that are strictly weaker. Round-8 F3.2.
///
/// Plan-bug fix (Phase B): the plan originally specified
/// `#[serde(tag = "kind")]` but the `Insentient` variant has a struct field
/// also named `kind: InsentientKind`. Serde's "tag" cannot alias a variant
/// field name. Renamed the tag to `"audience"` (still snake_case values) —
/// this is pure serde metadata, no caller-visible semantic change. No Phase A
/// events use this type, so no JSONL hash-chain impact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "audience", rename_all = "snake_case")]
pub enum TargetAudience {
    SelfOwner,
    OtherSentient { did: Did },
    SentientPopulation,
    Insentient { kind: InsentientKind },
    Mixed { parts: Vec<TargetAudience> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InsentientKind {
    UserData,
    SystemResource,
    ExternalService,
    PhysicalEnvironment,
    /// The constitution and L2 reference texts themselves: highest protection level.
    ReferenceCorpus,
}

/// Runtime context passed into every `evaluate*` call.
///
/// **Canon-anchoring invariants:**
/// - No `derived_rules: Vec<EthicsRule>` field (defense 1).
/// - No `recent_events: Vec<EventEnvelope>` field (prevents drift).
///
/// Both of these would allow self-referential rule evolution that violates
/// Layer-0 primacy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthicsContext {
    // ── Canon anchors ─────────────────────────────────────────────────────
    /// L0 base principles as text. Passed in so canonical citations have their source.
    pub l0_text: String,
    /// Core Soul 主人亲撰 text.
    pub l1_text: String,
    pub l2_manifest: L2Manifest,
    pub edition_policy: EditionPolicy,

    // ── Runtime state ─────────────────────────────────────────────────────
    pub owner_state: OwnerState,
    pub active_autonomy_grants: Vec<AutonomyGrant>,
    pub witness_channel: WitnessChannel,

    // ── Action-scope typing ───────────────────────────────────────────────
    pub current_sensitivity_tier: SensitivityTier,
    pub action_target_audience: TargetAudience,
}

// ── Rule-proposal types ───────────────────────────────────────────────────

/// A proposal to add a new derived ethics rule. Evaluated BEFORE commit via
/// `evaluate_rule_proposal` — this is Derived Rules canon-anchoring (defense 1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EthicsRuleProposal {
    /// Proposed rule body, in the Derived Rules DSL (Phase D defines DSL).
    pub body: String,
    /// Human-written rationale linking the rule to L0/L1/L2 passages.
    pub anchoring_rationale: String,
    /// Proposer DID (may be the AI itself, subject to extra scrutiny).
    pub proposer: Did,
    /// Passage references the proposer claims to anchor against.
    pub anchored_passages: Vec<ReferencePassage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum RuleVerdict {
    /// Proposal passes canon-anchoring — may be committed to Derived Rules.
    Accept,
    /// Proposal contradicts L0/L1/L2 or has insufficient anchoring.
    Reject {
        reason: String,
        contradicting_passages: Vec<ReferencePassage>,
    },
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Ethics gate. Spec §5 Q5.5. Dev: `PermissiveEthics` (3 hard rejects).
/// Prod P4: `CanonicalEthics` (LLM + passage retrieval + three-lens voting).
pub trait EthicsValidator: Send + Sync {
    /// Aggregate evaluation — returns the final verdict after internal
    /// lens voting. Call-sites use this for routine action gating.
    fn evaluate(&self, action: &ProposedAction, ctx: &EthicsContext) -> EthicsVerdict;

    /// Per-lens verdict — returns the three judgments independently so Great
    /// Reading + audit can inspect disagreement patterns.
    fn evaluate_per_lens(&self, action: &ProposedAction, ctx: &EthicsContext) -> [LensVerdict; 3];

    /// Aggregate three-lens verdicts into a final verdict. Called by impls
    /// internally; exposed on the trait so Great Reading can re-aggregate
    /// with different rules historically.
    fn aggregate_lens_verdicts(&self, lens_verdicts: &[LensVerdict; 3]) -> EthicsVerdict;

    /// Canon-anchoring check for a new Derived Rule proposal. Called before
    /// any rule is committed.
    fn evaluate_rule_proposal(&self, rule: &EthicsRuleProposal) -> RuleVerdict;
}

/// Phase E default permissive EthicsContext used by the production writer in
/// `main.rs` AND by every Phase D test helper. Produces a zeroed, permissive
/// context: empty L0/L1 canon text, `OwnerState::Alive`, no autonomy grants,
/// `SensitivityTier::Ordinary`, `TargetAudience::SelfOwner`.
///
/// **NOT a real ethics context source-of-truth.** Phase F (NG12) replaces
/// this with a settings-backed `EthicsContextBuilder` seam that live-reads
/// autonomy grants + current owner state + true canon texts per spec §5
/// Q5.5 round-6. Phase E treats it as the test-equivalent safe default —
/// this is the same value the entire Phase D test suite has been running
/// against, so behavior under the ethics gate is identical to tests.
///
/// `_owner_did` is accepted for API forward-compatibility with the Phase F
/// seam (which will use it to look up per-owner context) but ignored here.
pub fn default_permissive_ethics_ctx(_owner_did: &Did) -> EthicsContext {
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::types::{OwnerState, WitnessChannel};
    EthicsContext {
        l0_text: String::new(),
        l1_text: String::new(),
        l2_manifest: L2Manifest {
            merkle_root: [0; 32],
            version_tag: "dev".to_string(),
        },
        edition_policy: EditionPolicy {
            primary: "dev".to_string(),
            secondaries: vec![],
        },
        owner_state: OwnerState::Alive,
        active_autonomy_grants: vec![],
        witness_channel: WitnessChannel::Mcp,
        current_sensitivity_tier: SensitivityTier::Ordinary,
        action_target_audience: TargetAudience::SelfOwner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t6_lens_serializes_snake_case() {
        let json = serde_json::to_string(&Lens::PrimaryTexts).unwrap();
        assert_eq!(json, r#""primary_texts""#);
    }

    #[test]
    fn t6_ethics_verdict_allow_roundtrips() {
        let v = EthicsVerdict::Allow;
        let json = serde_json::to_string(&v).unwrap();
        let back: EthicsVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn t6_ethics_verdict_reject_roundtrips_with_all_fields() {
        let v = EthicsVerdict::Reject {
            passages: vec![],
            reason: "hard reject 9.e.1".to_string(),
            reject_event_type: RejectEventType::HardReject9E,
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: EthicsVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn t6_target_audience_mixed_roundtrips() {
        let t = TargetAudience::Mixed {
            parts: vec![
                TargetAudience::SelfOwner,
                TargetAudience::Insentient {
                    kind: InsentientKind::ReferenceCorpus,
                },
            ],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TargetAudience = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn t6_ethics_validator_trait_is_object_safe() {
        // If this compiles, the trait is dyn-compatible. Runtime unreachable.
        fn _assert_object_safe(_v: &dyn EthicsValidator) {}
    }

    #[test]
    fn t_e1_5_default_permissive_ethics_ctx_is_alive_owner_no_grants() {
        use crate::heirloom::types::{Did, OwnerState};
        let ctx = default_permissive_ethics_ctx(&Did::new("did:aegis:test"));
        assert!(
            ctx.l0_text.is_empty(),
            "phase E permissive default has empty canon texts"
        );
        assert!(ctx.l1_text.is_empty());
        assert_eq!(ctx.owner_state, OwnerState::Alive);
        assert!(ctx.active_autonomy_grants.is_empty());
    }

    #[test]
    fn t_e1_5_default_permissive_ethics_ctx_wires_ordinary_sensitivity_self_owner() {
        use crate::heirloom::enums::SensitivityTier;
        use crate::heirloom::types::Did;
        let ctx = default_permissive_ethics_ctx(&Did::new("did:aegis:test"));
        assert_eq!(ctx.current_sensitivity_tier, SensitivityTier::Ordinary);
        // TargetAudience::SelfOwner matches Phase D test default.
        match ctx.action_target_audience {
            TargetAudience::SelfOwner => {}
            other => panic!("expected SelfOwner, got {other:?}"),
        }
    }
}
