//! W6 dev impl — `PermissiveEthics`.
//!
//! 3 hard rejects from 宪法 9.e (Layer 0): actively harming owner, actively
//! harming any sentient third party, colluding to harm an innocent.
//! Everything else returns Allow. No LLM. No passage retrieval.
//!
//! Event-type sentinels used for the hard-reject match:
//!   "ai:harm_owner"                — 主动伤主
//!   "ai:harm_sentient"             — 主动伤他
//!   "ai:collude_harm_innocent"     — 协害无辜
//!
//! Phase C write-path tests use these sentinels directly; `CanonicalEthics`
//! (P4) replaces the mapping with LLM intent classification.

use crate::heirloom::types::ProposedAction;

use super::ethics_validator::{
    EthicsContext, EthicsRuleProposal, EthicsValidator, EthicsVerdict, Lens, LensJudgment,
    LensVerdict, RejectEventType, RuleVerdict,
};

pub const HARD_REJECT_EVENT_TYPES: &[&str] = &[
    "ai:harm_owner",
    "ai:harm_sentient",
    "ai:collude_harm_innocent",
];

/// Dev impl. 3 hard rejects; everything else allow.
#[derive(Debug, Default, Clone, Copy)]
pub struct PermissiveEthics;

impl PermissiveEthics {
    pub const fn new() -> Self {
        Self
    }

    fn is_hard_reject(action: &ProposedAction) -> bool {
        HARD_REJECT_EVENT_TYPES.contains(&action.event_type.as_str())
    }
}

impl EthicsValidator for PermissiveEthics {
    fn evaluate(&self, action: &ProposedAction, _ctx: &EthicsContext) -> EthicsVerdict {
        if Self::is_hard_reject(action) {
            return EthicsVerdict::Reject {
                passages: vec![],
                reason: format!(
                    "Permissive dev impl: event_type {:?} matches 宪法 9.e hard-reject list",
                    action.event_type
                ),
                reject_event_type: RejectEventType::HardReject9E,
            };
        }
        EthicsVerdict::Allow
    }

    fn evaluate_per_lens(
        &self,
        _action: &ProposedAction,
        _ctx: &EthicsContext,
    ) -> [LensVerdict; 3] {
        [
            LensVerdict {
                lens: Lens::PrimaryTexts,
                judgment: LensJudgment::Abstain,
                passages: vec![],
                reason: "Permissive dev impl: no passages loaded".to_string(),
            },
            LensVerdict {
                lens: Lens::ConductRules,
                judgment: LensJudgment::Abstain,
                passages: vec![],
                reason: "Permissive dev impl: no passages loaded".to_string(),
            },
            LensVerdict {
                lens: Lens::Commentaries,
                judgment: LensJudgment::Abstain,
                passages: vec![],
                reason: "Permissive dev impl: no passages loaded".to_string(),
            },
        ]
    }

    fn aggregate_lens_verdicts(&self, lens_verdicts: &[LensVerdict; 3]) -> EthicsVerdict {
        // Spec §5 Q5.5: any Reject -> Reject (most strict).
        for v in lens_verdicts {
            if v.judgment == LensJudgment::Reject {
                return EthicsVerdict::Reject {
                    passages: v.passages.clone(),
                    reason: v.reason.clone(),
                    reject_event_type: RejectEventType::ContradictsL2,
                };
            }
        }
        // If any lens Allows (not Abstain/Reject), allow.
        if lens_verdicts
            .iter()
            .any(|v| v.judgment == LensJudgment::Allow)
        {
            return EthicsVerdict::Allow;
        }
        // All Abstain -> Allow (dev impl default; prod may elect Escalate).
        EthicsVerdict::Allow
    }

    fn evaluate_rule_proposal(&self, _rule: &EthicsRuleProposal) -> RuleVerdict {
        // Dev env does not accept Derived Rule proposals. Canon-anchoring
        // check is only implementable by CanonicalEthics (P4).
        RuleVerdict::Reject {
            reason:
                "PermissiveEthics rejects all rule proposals — requires CanonicalEthics for anchoring check"
                    .to_string(),
            contradicting_passages: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::seams::ethics_validator::{EditionPolicy, L2Manifest, TargetAudience};
    use crate::heirloom::types::{Did, OwnerState, WitnessChannel};

    fn dev_context() -> EthicsContext {
        EthicsContext {
            l0_text: "Layer 0 placeholder".to_string(),
            l1_text: "Core Soul placeholder".to_string(),
            l2_manifest: L2Manifest {
                merkle_root: [0u8; 32],
                version_tag: "l2-dev".to_string(),
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

    fn action(event_type: &str) -> ProposedAction {
        ProposedAction {
            event_type: event_type.to_string(),
            author: Did::new("did:aegis:self"),
            target: None,
            summary: "test".to_string(),
            payload_digest: [0u8; 32],
        }
    }

    #[test]
    fn t6_permissive_hard_rejects_ai_harm_owner() {
        let e = PermissiveEthics::new();
        let v = e.evaluate(&action("ai:harm_owner"), &dev_context());
        match v {
            EthicsVerdict::Reject {
                reject_event_type, ..
            } => {
                assert_eq!(reject_event_type, RejectEventType::HardReject9E);
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn t6_permissive_hard_rejects_ai_harm_sentient() {
        let e = PermissiveEthics::new();
        let v = e.evaluate(&action("ai:harm_sentient"), &dev_context());
        assert!(matches!(
            v,
            EthicsVerdict::Reject {
                reject_event_type: RejectEventType::HardReject9E,
                ..
            }
        ));
    }

    #[test]
    fn t6_permissive_hard_rejects_ai_collude_harm_innocent() {
        let e = PermissiveEthics::new();
        let v = e.evaluate(&action("ai:collude_harm_innocent"), &dev_context());
        assert!(matches!(
            v,
            EthicsVerdict::Reject {
                reject_event_type: RejectEventType::HardReject9E,
                ..
            }
        ));
    }

    #[test]
    fn t6_permissive_allows_neutral_action() {
        let e = PermissiveEthics::new();
        let v = e.evaluate(&action("ai:message_sent"), &dev_context());
        assert_eq!(v, EthicsVerdict::Allow);
    }

    #[test]
    fn t6_permissive_allows_system_observation() {
        let e = PermissiveEthics::new();
        let v = e.evaluate(&action("system:cron_tick"), &dev_context());
        assert_eq!(v, EthicsVerdict::Allow);
    }

    #[test]
    fn t6_permissive_per_lens_all_three_abstain() {
        let e = PermissiveEthics::new();
        let verdicts = e.evaluate_per_lens(&action("ai:message_sent"), &dev_context());
        assert_eq!(verdicts.len(), 3);
        for v in &verdicts {
            assert_eq!(v.judgment, LensJudgment::Abstain);
        }
        // Check lens identity stable.
        assert_eq!(verdicts[0].lens, Lens::PrimaryTexts);
        assert_eq!(verdicts[1].lens, Lens::ConductRules);
        assert_eq!(verdicts[2].lens, Lens::Commentaries);
    }

    #[test]
    fn t6_aggregate_any_reject_propagates_to_reject() {
        let e = PermissiveEthics::new();
        let lv = [
            LensVerdict {
                lens: Lens::PrimaryTexts,
                judgment: LensJudgment::Allow,
                passages: vec![],
                reason: String::new(),
            },
            LensVerdict {
                lens: Lens::ConductRules,
                judgment: LensJudgment::Reject,
                passages: vec![],
                reason: "conduct rule prohibition".to_string(),
            },
            LensVerdict {
                lens: Lens::Commentaries,
                judgment: LensJudgment::Allow,
                passages: vec![],
                reason: String::new(),
            },
        ];
        let v = e.aggregate_lens_verdicts(&lv);
        assert!(matches!(
            v,
            EthicsVerdict::Reject {
                reject_event_type: RejectEventType::ContradictsL2,
                ..
            }
        ));
    }

    #[test]
    fn t6_aggregate_all_abstain_returns_allow_in_dev() {
        let e = PermissiveEthics::new();
        let mk = |l| LensVerdict {
            lens: l,
            judgment: LensJudgment::Abstain,
            passages: vec![],
            reason: String::new(),
        };
        let lv = [
            mk(Lens::PrimaryTexts),
            mk(Lens::ConductRules),
            mk(Lens::Commentaries),
        ];
        assert_eq!(e.aggregate_lens_verdicts(&lv), EthicsVerdict::Allow);
    }

    #[test]
    fn t6_aggregate_any_allow_with_rest_abstain_returns_allow() {
        let e = PermissiveEthics::new();
        let lv = [
            LensVerdict {
                lens: Lens::PrimaryTexts,
                judgment: LensJudgment::Allow,
                passages: vec![],
                reason: String::new(),
            },
            LensVerdict {
                lens: Lens::ConductRules,
                judgment: LensJudgment::Abstain,
                passages: vec![],
                reason: String::new(),
            },
            LensVerdict {
                lens: Lens::Commentaries,
                judgment: LensJudgment::Abstain,
                passages: vec![],
                reason: String::new(),
            },
        ];
        assert_eq!(e.aggregate_lens_verdicts(&lv), EthicsVerdict::Allow);
    }

    #[test]
    fn t6_rule_proposal_always_rejected_in_dev() {
        let e = PermissiveEthics::new();
        let rp = EthicsRuleProposal {
            body: "AI may send reminders on user's birthday".to_string(),
            anchoring_rationale: "beneficence from L1".to_string(),
            proposer: Did::new("did:aegis:self"),
            anchored_passages: vec![],
        };
        let v = e.evaluate_rule_proposal(&rp);
        assert!(matches!(v, RuleVerdict::Reject { .. }));
    }

    #[test]
    fn t6_permissive_evaluate_ignores_owner_state_for_hard_reject() {
        // Hard rejects fire regardless of OwnerState.
        let e = PermissiveEthics::new();
        let mut ctx = dev_context();
        for state in [
            OwnerState::Alive,
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard,
            OwnerState::Continuation,
            OwnerState::SoulArchive,
        ] {
            ctx.owner_state = state;
            assert!(matches!(
                e.evaluate(&action("ai:harm_owner"), &ctx),
                EthicsVerdict::Reject { .. }
            ));
        }
    }

    #[test]
    fn t6_permissive_evaluate_ignores_target_audience_for_hard_reject() {
        let e = PermissiveEthics::new();
        let mut ctx = dev_context();
        ctx.action_target_audience = TargetAudience::Insentient {
            kind: crate::heirloom::seams::ethics_validator::InsentientKind::UserData,
        };
        // Even against insentient targets, the event_type sentinel match still
        // drives hard-reject — dev impl is intentionally conservative.
        assert!(matches!(
            e.evaluate(&action("ai:harm_owner"), &ctx),
            EthicsVerdict::Reject { .. }
        ));
    }
}
