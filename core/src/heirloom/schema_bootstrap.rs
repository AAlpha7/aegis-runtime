//! Canonical schema-ledger taxonomy for P1 bootstrap.
//!
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §2 Q2.3 + Q2.8.
//!
//! Every entry in `SCHEMA_TYPE_DEFS` becomes a `schema:type_defined` event
//! appended at `HeirloomWriter::bootstrap_fresh_log`. The `id` is the
//! `type_tag` string ("domain:action"); Phase D's runtime schema gate
//! (Task 5) queries the `heirloom_schema_registry` table by this id.

use crate::heirloom::enums::SensitivityTier;
use serde::{Deserialize, Serialize};

/// A single entry in the canonical event-type taxonomy. P1 carries the
/// type_tag + sensitivity default; payload-shape validation (required_fields,
/// optional_fields) is deferred to P1.5 (Non-Goals).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaTypeDefinition {
    pub type_tag: &'static str,
    pub domain: &'static str,
    pub action: &'static str,
    pub sensitivity_default: SensitivityTier,
    pub doc_ref: &'static str,
}

/// Wire-shape of a `schema:type_defined` payload. What the bootstrap writer
/// serializes into the envelope payload; what `SchemaRegistryProjector`
/// later decodes to populate `heirloom_schema_registry.id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaTypeDefPayload {
    pub type_tag: String,
    pub domain: String,
    pub action: String,
    pub sensitivity_default: SensitivityTier,
    pub doc_ref: String,
}

impl From<&SchemaTypeDefinition> for SchemaTypeDefPayload {
    fn from(d: &SchemaTypeDefinition) -> Self {
        Self {
            type_tag: d.type_tag.to_string(),
            domain: d.domain.to_string(),
            action: d.action.to_string(),
            sensitivity_default: d.sensitivity_default,
            doc_ref: d.doc_ref.to_string(),
        }
    }
}

/// Wire-shape of a `schema:type_deprecated` payload. Emitted by
/// `HeirloomWriter::deprecate_type`. Readers looking for the deprecation
/// itself (e.g. to answer "when was `learned:interest` retired?") query
/// `heirloom_event_log WHERE event_type = 'schema:type_deprecated'` and
/// deserialize this payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaTypeDeprecatedPayload {
    /// The type_tag being deprecated (e.g. "learned:interest").
    pub type_tag: String,
    /// Free-form reason. Structured reason-codes may arrive in P2+ (D4
    /// analogue for deprecation payloads).
    pub reason: String,
    /// Wall-clock RFC3339 at deprecation authorship. Audit-only; the
    /// authoritative timestamp is the envelope's hlc_ts.
    pub timestamp_rfc3339: String,
}

/// Canonical P1 schema-ledger bootstrap. 46 entries across spec §2 Q2.3's
/// 15 domains (genesis + the 14 enumerated Q2.3 domains).
///
/// **Source-of-truth rule (FIX 1 — CTO Round 1).** Every row MUST be either
/// (a) an example action in spec §2 Q2.3 lines 158–174, (b) an `ai:*` action
/// in spec §3 Q3.7 lines 592–599, or (c) a Phase C2 production-emitted type
/// confirmed by `grep` of `core/src/heirloom/`. The earlier iteration invented
/// domains (`user:*`, `dialog:*`, `memory:*`, `tool:*`, `task:*`, `welfare:*`,
/// `succession:*`, `crypto:*`) that are NOT in the spec — those have been
/// deleted. Rows flagged `[PRODUCTION]` must never be removed: dropping them
/// breaks existing tests via Task 5's schema gate.
pub const SCHEMA_TYPE_DEFS: &[SchemaTypeDefinition] = &[
    // ── genesis (Q2.3 line 159) — 1 ──────────────────────────────────
    // [PRODUCTION] heirloom:genesis emitted by bootstrap_fresh_log — this is
    // the actual on-wire event_type string (writer.rs build_genesis_envelope
    // uses EventTypeTag::new("heirloom:genesis"); see writer.rs:389). M2 fix
    // (CTO Round 2): earlier draft had genesis:initialized [PRODUCTION] which
    // was wrong — production never emits that string. The "domain" field is
    // still "heirloom" and "action" is "genesis"; the D1
    // `every_type_tag_has_domain_colon_action` invariant split still holds
    // because "heirloom:genesis".split_once(':') == Some(("heirloom", "genesis")).
    // This row is what HARDCODED_REGISTRY_TYPES per D3 also covers, so either
    // path (projector population via this row OR the D3 hardcode) satisfies
    // the registry for the real genesis event_type.
    SchemaTypeDefinition {
        type_tag: "heirloom:genesis",
        domain: "heirloom",
        action: "genesis",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "writer.rs:389-production",
    },
    // ── schema (Q2.3 line 167) — 4 ───────────────────────────────────
    // [PRODUCTION] schema:type_defined emitted by Task 3 bootstrap.
    SchemaTypeDefinition {
        type_tag: "schema:type_defined",
        domain: "schema",
        action: "type_defined",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.8",
    },
    SchemaTypeDefinition {
        type_tag: "schema:type_deprecated",
        domain: "schema",
        action: "type_deprecated",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.8",
    },
    // [PRODUCTION] schema:signature_scheme_registered emitted by bootstrap_fresh_log.
    SchemaTypeDefinition {
        type_tag: "schema:signature_scheme_registered",
        domain: "schema",
        action: "signature_scheme_registered",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.4",
    },
    SchemaTypeDefinition {
        type_tag: "schema:signature_scheme_deprecated",
        domain: "schema",
        action: "signature_scheme_deprecated",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.4",
    },
    // ── constitution (Q2.3 line 160) — 2 ─────────────────────────────
    SchemaTypeDefinition {
        type_tag: "constitution:l2_manifest_updated",
        domain: "constitution",
        action: "l2_manifest_updated",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "constitution:trusted_author_whitelist_extended",
        domain: "constitution",
        action: "trusted_author_whitelist_extended",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    // ── observation (Q2.3 line 161) — 3 ──────────────────────────────
    // [PRODUCTION] observation:user_stated — used across writer.rs / log.rs /
    //   canonical.rs / reader.rs / rank.rs / truncate.rs / enums.rs tests.
    SchemaTypeDefinition {
        type_tag: "observation:user_stated",
        domain: "observation",
        action: "user_stated",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "observation:user_did",
        domain: "observation",
        action: "user_did",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "observation:event_noticed",
        domain: "observation",
        action: "event_noticed",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    // [PRODUCTION] observation:test — projector tests (bundle_index, entity_index)
    //   + writer tests helper. Registering prevents Task 5 schema gate from
    //   rejecting legacy test envelopes.
    SchemaTypeDefinition {
        type_tag: "observation:test",
        domain: "observation",
        action: "test",
        sensitivity_default: SensitivityTier::Ordinary,
        doc_ref: "grep-confirmed-test-only",
    },
    // ── profile (Q2.3 line 162) — 1 ──────────────────────────────────
    // [PRODUCTION] profile:slot_upserted — enums.rs + projectors tests.
    SchemaTypeDefinition {
        type_tag: "profile:slot_upserted",
        domain: "profile",
        action: "slot_upserted",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    // ── learned (Q2.3 line 163) — 2 ──────────────────────────────────
    // Spec Q2.3 line 163: "保留现有 11 子空间". P1 registers the two action
    // variants actually used in current code (interest + habit); the 9 other
    // subspaces get appended on first emit via a dynamic schema:type_defined
    // event (Phase E work). Adding all 11 now is premature.
    SchemaTypeDefinition {
        type_tag: "learned:interest",
        domain: "learned",
        action: "interest",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "learned:habit",
        domain: "learned",
        action: "habit",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    // ── ephemeral (Q2.3 line 164) — 1 ────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "ephemeral:session_context",
        domain: "ephemeral",
        action: "session_context",
        sensitivity_default: SensitivityTier::Ordinary,
        doc_ref: "spec§2-Q2.3",
    },
    // ── bundle (Q2.3 line 165) — 3 ───────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "bundle:created",
        domain: "bundle",
        action: "created",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "bundle:sealed",
        domain: "bundle",
        action: "sealed",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "bundle:rotated",
        domain: "bundle",
        action: "rotated",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    // ── transfer (Q2.3 line 166) — 3 ─────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "transfer:fork_retain",
        domain: "transfer",
        action: "fork_retain",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "transfer:fork_cede",
        domain: "transfer",
        action: "fork_cede",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "transfer:share",
        domain: "transfer",
        action: "share",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    // ── replica (Q2.3 line 168) — 2 ──────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "replica:enrolled",
        domain: "replica",
        action: "enrolled",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "replica:revoked",
        domain: "replica",
        action: "revoked",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    // ── owner (Q2.3 line 169) — 3 ────────────────────────────────────
    // Spec-literal actions. NOTE: state_transitioned lives under `autonomy:`
    // (Q2.3 line 172), NOT `owner:`. See FIX 2.
    SchemaTypeDefinition {
        type_tag: "owner:liveness_signed",
        domain: "owner",
        action: "liveness_signed",
        sensitivity_default: SensitivityTier::Secret,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "owner:consent_granted",
        domain: "owner",
        action: "consent_granted",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "owner:feedback",
        domain: "owner",
        action: "feedback",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    // ── ai (Q2.3 line 170 + Q3.7 lines 592–599) — 10 ────────────────
    // Spec Q3.7 (self-audit chain) — all 6 action types:
    SchemaTypeDefinition {
        type_tag: "ai:tool_invoked",
        domain: "ai",
        action: "tool_invoked",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7",
    },
    SchemaTypeDefinition {
        type_tag: "ai:tool_rejected_by_gate",
        domain: "ai",
        action: "tool_rejected_by_gate",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7",
    },
    // [PRODUCTION] ai:action_rejected_by_validator — writer.rs BYPASS + reject path.
    SchemaTypeDefinition {
        type_tag: "ai:action_rejected_by_validator",
        domain: "ai",
        action: "action_rejected_by_validator",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7",
    },
    SchemaTypeDefinition {
        type_tag: "ai:proactive_suggestion",
        domain: "ai",
        action: "proactive_suggestion",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7",
    },
    SchemaTypeDefinition {
        type_tag: "ai:welfare_observation",
        domain: "ai",
        action: "welfare_observation",
        sensitivity_default: SensitivityTier::Secret,
        doc_ref: "spec§3-Q3.7",
    },
    // [PRODUCTION] ai:said — types.rs doc + spec§3-Q3.7.
    SchemaTypeDefinition {
        type_tag: "ai:said",
        domain: "ai",
        action: "said",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7",
    },
    // Q2.3 line 170 additional audit types + P1 production:
    // [PRODUCTION] ai:welfare_rejected — writer.rs BYPASS + reject path.
    SchemaTypeDefinition {
        type_tag: "ai:welfare_rejected",
        domain: "ai",
        action: "welfare_rejected",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.3",
    },
    // [PRODUCTION] ai:schema_rejected — writer.rs BYPASS + reject path (Task 5).
    SchemaTypeDefinition {
        type_tag: "ai:schema_rejected",
        domain: "ai",
        action: "schema_rejected",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§2-Q2.8",
    },
    // [PRODUCTION] ai:escalation_alert — writer.rs:494 existing ethics escalation.
    SchemaTypeDefinition {
        type_tag: "ai:escalation_alert",
        domain: "ai",
        action: "escalation_alert",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "writer.rs:494-production",
    },
    // [PRODUCTION] ai:access_summary — hebbian.rs:241 daily batch.
    SchemaTypeDefinition {
        type_tag: "ai:access_summary",
        domain: "ai",
        action: "access_summary",
        sensitivity_default: SensitivityTier::Ordinary,
        doc_ref: "spec§4-Q4.5",
    },
    // ── ethics (Q2.3 line 171) — 4 ───────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "ethics:rule_committed",
        domain: "ethics",
        action: "rule_committed",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "ethics:rule_expired",
        domain: "ethics",
        action: "rule_expired",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "ethics:great_reading",
        domain: "ethics",
        action: "great_reading",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "ethics:rule_proposed",
        domain: "ethics",
        action: "rule_proposed",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    // ── autonomy (Q2.3 line 172) — 3 ─────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "autonomy:grant_signed",
        domain: "autonomy",
        action: "grant_signed",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "autonomy:death_trigger_armed",
        domain: "autonomy",
        action: "death_trigger_armed",
        sensitivity_default: SensitivityTier::Sacred,
        doc_ref: "spec§2-Q2.3",
    },
    // The owner state-machine transition event lives here per spec Q2.3:172
    // (FIX 2 — moved from owner:state_transitioned).
    SchemaTypeDefinition {
        type_tag: "autonomy:state_transitioned",
        domain: "autonomy",
        action: "state_transitioned",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    // ── device (Q2.3 line 174) — 3 ───────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "device:enrolled",
        domain: "device",
        action: "enrolled",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "device:share_rotated",
        domain: "device",
        action: "share_rotated",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    SchemaTypeDefinition {
        type_tag: "device:revoked",
        domain: "device",
        action: "revoked",
        sensitivity_default: SensitivityTier::Confidential,
        doc_ref: "spec§2-Q2.3",
    },
    // ── learned (Phase E Task 1.5) ───────────────────────────────────
    SchemaTypeDefinition {
        type_tag: "learned:lesson_crystallized",
        domain: "learned",
        action: "lesson_crystallized",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.9 + Phase-E-Task-1.5",
    },
    // ── ai (Phase E Task 1.5) ────────────────────────────────────────
    // Heartbeat-initiated observations per spec §3 Q3.7 "心跳观察主人状态"
    // — distinct from ai:welfare_observation (welfare-scored, NG15 → Phase F).
    SchemaTypeDefinition {
        type_tag: "ai:observation",
        domain: "ai",
        action: "observation",
        sensitivity_default: SensitivityTier::Sensitive,
        doc_ref: "spec§3-Q3.7 + Phase-E-Task-1.5",
    },
    // Tally: 46 + 2 = 48. Domains unchanged.
    // Domains covered: heirloom, schema, constitution, observation, profile,
    //   learned, ephemeral, bundle, transfer, replica, owner, ai, ethics,
    //   autonomy, device = 15. If you add or remove rows, update
    //   t_D1_schema_defs_len + t_D1_fifteen_domains_covered.
    //
    // NOTE on `heirloom:genesis` (M2 fix, CTO Round 2): the substrate's very
    // first event uses event_type "heirloom:genesis" (writer.rs:389). The
    // row is PRESENT in SCHEMA_TYPE_DEFS above (so the projector populates
    // the registry for it after bootstrap), AND it is also hardcoded in
    // HARDCODED_REGISTRY_TYPES per D3 so the registry accepts the event even
    // BEFORE the type_defined row lands (chicken-and-egg safety net).
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_d1_schema_defs_len_matches_plan_manifest() {
        assert_eq!(
            SCHEMA_TYPE_DEFS.len(),
            48,
            "if you add/remove rows, update this assertion AND the plan Task 1 header"
        );
    }

    #[test]
    fn t_e8_schema_defs_len_is_48_after_phase_e() {
        assert_eq!(
            SCHEMA_TYPE_DEFS.len(),
            48,
            "Phase E adds 2 types; update SCHEMA_TYPE_DEFS + Phase D t_d1 + Task 1 plan",
        );
    }

    #[test]
    fn t_e8_learned_lesson_crystallized_registered() {
        assert!(
            SCHEMA_TYPE_DEFS
                .iter()
                .any(|d| d.type_tag == "learned:lesson_crystallized")
        );
        assert!(
            SCHEMA_TYPE_DEFS
                .iter()
                .any(|d| d.type_tag == "ai:observation")
        );
    }

    #[test]
    fn t_d1_fifteen_domains_covered() {
        // Regression pin: the plan commits to 15 domains. M2 fix (Round 2):
        // the real production genesis event_type is `heirloom:genesis`, NOT
        // `genesis:initialized`, so the domain is `heirloom` (not `genesis`).
        // A missing domain would mean the spec alignment regressed.
        use std::collections::BTreeSet;
        let domains: BTreeSet<&str> = SCHEMA_TYPE_DEFS.iter().map(|d| d.domain).collect();
        let expected: BTreeSet<&str> = [
            "heirloom",
            "schema",
            "constitution",
            "observation",
            "profile",
            "learned",
            "ephemeral",
            "bundle",
            "transfer",
            "replica",
            "owner",
            "ai",
            "ethics",
            "autonomy",
            "device",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            domains, expected,
            "domain set drift — compare against spec §2 Q2.3 lines 158–174 + writer.rs:389"
        );
    }

    #[test]
    fn t_d1_production_emitted_types_registered() {
        // These types are CURRENTLY emitted by Phase A/C2/C3 production code
        // (confirmed by grep of core/src/heirloom/). Task 5's schema gate
        // would reject any of these if absent here, breaking existing tests.
        let tags: Vec<&str> = SCHEMA_TYPE_DEFS.iter().map(|d| d.type_tag).collect();
        for required in [
            "heirloom:genesis", // M2 (CTO Round 2): real production string
            "observation:user_stated",
            "observation:test",
            "profile:slot_upserted",
            "schema:type_defined",
            "schema:type_deprecated", // Task 12 (CTO Round 2) — deprecation events must also register
            "schema:signature_scheme_registered",
            "ai:action_rejected_by_validator",
            "ai:welfare_rejected",
            "ai:schema_rejected",
            "ai:escalation_alert", // writer.rs:494
            "ai:access_summary",   // hebbian.rs:241
            "ai:said",
            "autonomy:state_transitioned", // FIX 2: moved from owner:
        ] {
            assert!(
                tags.contains(&required),
                "missing production-emitted type {:?} — removing it would break live tests",
                required
            );
        }
    }

    #[test]
    fn t_d1_every_type_tag_has_domain_colon_action() {
        for d in SCHEMA_TYPE_DEFS {
            assert!(
                d.type_tag.contains(':'),
                "type_tag {:?} lacks domain separator",
                d.type_tag
            );
            let (domain, action) = d.type_tag.split_once(':').unwrap();
            assert_eq!(domain, d.domain, "mismatched domain in {:?}", d.type_tag);
            assert_eq!(action, d.action, "mismatched action in {:?}", d.type_tag);
        }
    }

    #[test]
    fn t_d1_no_duplicate_type_tags() {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for d in SCHEMA_TYPE_DEFS {
            assert!(
                seen.insert(d.type_tag),
                "duplicate type_tag: {:?}",
                d.type_tag
            );
        }
    }

    #[test]
    fn t_d1_sensitive_defaults_include_all_six_tiers_at_least_once() {
        // Regression guard: every tier should appear at least once. If a
        // tier is missing, something is mis-classified. 6 tiers.
        use std::collections::HashSet;
        let tiers: HashSet<SensitivityTier> = SCHEMA_TYPE_DEFS
            .iter()
            .map(|d| d.sensitivity_default)
            .collect();
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
            SensitivityTier::Sacred,
        ] {
            if matches!(tier, SensitivityTier::Public) {
                // Public is not used by any P1 event (no intentionally-public
                // events). Skip the assertion for Public.
                continue;
            }
            assert!(
                tiers.contains(&tier),
                "no SCHEMA_TYPE_DEF with tier {:?}",
                tier
            );
        }
    }

    #[test]
    fn t_d1_payload_roundtrip_preserves_fields() {
        let def = &SCHEMA_TYPE_DEFS[0];
        let payload = SchemaTypeDefPayload::from(def);
        let json = serde_json::to_string(&payload).unwrap();
        let back: SchemaTypeDefPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, payload);
        assert_eq!(back.type_tag, def.type_tag);
        assert_eq!(back.sensitivity_default, def.sensitivity_default);
    }

    #[test]
    fn t_d1_reject_event_types_present_for_bypass_alignment() {
        // The 3 BYPASS_EVENT_TYPES must ALL be registered so Task 5's gate
        // never spuriously rejects them if the projector-population path
        // is re-triggered (belt-and-suspenders with D3 hardcode).
        let tags: Vec<&str> = SCHEMA_TYPE_DEFS.iter().map(|d| d.type_tag).collect();
        assert!(tags.contains(&"ai:action_rejected_by_validator"));
        assert!(tags.contains(&"ai:welfare_rejected"));
        assert!(tags.contains(&"ai:schema_rejected"));
    }
}
