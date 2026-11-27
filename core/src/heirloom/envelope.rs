//! EventEnvelope: 16-field canonical header for every heirloom event.
//!
//! Spec: `docs/heirloom/specs/2026-04-16-p1-substrate-design.md` §2 Q2.2.
//!
//! JSON field ordering in serde output is NOT canonical — use
//! `crate::heirloom::canonical::canonical_bytes()` for hashing input.

use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
use crate::heirloom::hlc::HlcTimestamp;
use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    // ─── Identity ───
    /// UUIDv7 time-ordered event identifier.
    pub event_id: Uuid,

    /// **Envelope** schema version — describes THIS struct's 16-field
    /// shape only. Bumped when an envelope field is added / removed /
    /// renamed. **Distinct from payload versioning** (Round-9 polish P2.4):
    /// payload shapes are defined per `event_type` by `schema:type_defined`
    /// events in the schema ledger (spec §2 Q2.8), each carrying its own
    /// `payload_schema_version`. The two version numbers evolve
    /// independently: envelope v1 logs may contain both payload v1 and v2
    /// variants of the same event_type; envelope v2 upgrades do not
    /// invalidate older payload versions. See spec §2 Q2.2 (lines 86-92,
    /// 144-151) for the full rationale.
    pub schema_version: u32,

    /// Event type tag (`"domain:action"`). Validated against the schema
    /// ledger (W18) at write time; Phase A treats it as opaque.
    pub event_type: EventTypeTag,

    // ─── Chain ───
    #[serde(with = "hex_array_32")]
    pub prev_hash: [u8; 32],
    pub hlc_ts: HlcTimestamp,
    pub observed_at: DateTime<Utc>,

    // ─── Constitution ───
    #[serde(with = "hex_array_32")]
    pub l0_hash: [u8; 32],
    #[serde(with = "hex_array_32")]
    pub l1_hash: [u8; 32],
    #[serde(with = "hex_array_32")]
    pub l2_manifest_hash: [u8; 32],

    // ─── Provenance ───
    pub lineage_id: LineageId,
    pub author_did: Did,
    pub signatures: Vec<Signature>,

    // ─── Security ───
    pub sensitivity_tier: SensitivityTier,
    pub bundle_ids: Vec<BundleId>,

    // ─── Payload ───
    pub payload_encoding: PayloadEncoding,
    #[serde(with = "hex_bytes")]
    pub payload: Vec<u8>,
}

// Hex-string serde helpers for [u8; 32] fields — JSON doesn't have raw bytes.
mod hex_array_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if v.len() != 32 {
            return Err(serde::de::Error::custom(format!(
                "expected 32-byte hex, got {}",
                v.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        Ok(out)
    }
}

// Same-style helper for variable-length Vec<u8> (payload).
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

impl EventEnvelope {
    /// True if this event represents an AI-originated action.
    ///
    /// Round 2 (Task 13): matches both `"ai:"` and `"autonomy:"` prefixes
    /// so that AI-initiated state transitions (event_type
    /// `autonomy:state_transitioned`, spec Q2.3:172) traverse the
    /// ethics/welfare gate chain. Without this, `HeirloomWriter::run_ethics_gate`
    /// short-circuits on `!is_ai_action()` and the gate never sees state
    /// transitions — a hole that Task 13 (user directive "请按最长远最优方案做")
    /// closes.
    pub fn is_ai_action(&self) -> bool {
        let et = self.event_type.as_str();
        et.starts_with("ai:") || et.starts_with("autonomy:")
    }

    /// True if this is a proactive AI action — the subset of `is_ai_action`
    /// that does NOT include reactive replies (`ai:said`), reject events
    /// (`ai:action_rejected_by_validator`, `ai:welfare_rejected`), or audit
    /// markers (`ai:reflection`). Welfare gate (spec §5 Q5.9 step 3) fires
    /// only on proactive AI actions.
    pub fn is_proactive_ai(&self) -> bool {
        if !self.is_ai_action() {
            return false;
        }
        let action = self.event_type.as_str();
        !matches!(
            action,
            "ai:said"
                | "ai:action_rejected_by_validator"
                | "ai:welfare_rejected"
                | "ai:schema_rejected"
                | "ai:reflection"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::parse_str("01925b3e-0000-7000-8000-000000000001").unwrap(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash: [0xAA; 32],
            hlc_ts: HlcTimestamp {
                phys: 1729123456789,
                log: 0,
            },
            observed_at: DateTime::parse_from_rfc3339("2026-04-17T09:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0x10; 32],
            l1_hash: [0x20; 32],
            l2_manifest_hash: [0x30; 32],
            lineage_id: LineageId::new("lineage:test:001"),
            author_did: Did::new("did:aegis:test:alice"),
            signatures: vec![Signature::new(vec![0xDE, 0xAD])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{\"text\":\"hello\"}".to_vec(),
        }
    }

    #[test]
    fn t5_envelope_serde_roundtrip_preserves_all_fields() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn t5_envelope_hash_fields_hex_encoded() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        // Three 32-byte arrays serialize as 64-char hex strings.
        assert!(json.contains(
            r#""prev_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa""#
        ));
        assert!(json.contains(
            r#""l0_hash":"1010101010101010101010101010101010101010101010101010101010101010""#
        ));
    }

    #[test]
    fn t5_envelope_payload_hex_encoded() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        // b"{\"text\":\"hello\"}" = 7b 22 74 65 78 74 22 3a 22 68 65 6c 6c 6f 22 7d
        assert!(json.contains(r#""payload":"7b2274657874223a2268656c6c6f227d""#));
    }

    #[test]
    fn t5_envelope_event_type_serializes_as_string_not_object() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains(r#""event_type":"observation:user_stated""#));
    }

    #[test]
    fn t5_envelope_hlc_ts_nested_object_with_stringified_phys() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        // phys stringified to avoid JCS u64→f64 coercion; see hlc.rs docstring.
        assert!(json.contains(r#""hlc_ts":{"phys":"1729123456789","log":0}"#));
    }

    #[test]
    fn t5_envelope_rejects_hex_with_wrong_length_for_prev_hash() {
        // prev_hash must be exactly 32 bytes; shorter input errors.
        let mut json = serde_json::to_string(&sample_envelope()).unwrap();
        json = json.replace(
            r#""prev_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa""#,
            r#""prev_hash":"ffff""#,
        );
        let result: Result<EventEnvelope, _> = serde_json::from_str(&json);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected 32-byte hex")
        );
    }

    // ── Task 5: is_ai_action / is_proactive_ai helpers ────────────────────

    fn envelope_with_type(tag: &str) -> EventEnvelope {
        let mut e = sample_envelope();
        e.event_type = crate::heirloom::enums::EventTypeTag::new(tag);
        e
    }

    #[test]
    fn t10_is_ai_action_true_for_ai_prefix() {
        assert!(envelope_with_type("ai:sent_email").is_ai_action());
    }

    #[test]
    fn t10_is_ai_action_false_for_observation_prefix() {
        assert!(!envelope_with_type("observation:user_stated").is_ai_action());
    }

    #[test]
    fn t10_is_ai_action_false_for_schema_prefix() {
        assert!(!envelope_with_type("schema:type_defined").is_ai_action());
    }

    #[test]
    fn t10_is_proactive_ai_true_for_tool_invoked() {
        assert!(envelope_with_type("ai:sent_email").is_proactive_ai());
    }

    #[test]
    fn t10_is_proactive_ai_false_for_said() {
        assert!(!envelope_with_type("ai:said").is_proactive_ai());
    }

    #[test]
    fn t10_is_proactive_ai_false_for_reject_events() {
        assert!(!envelope_with_type("ai:action_rejected_by_validator").is_proactive_ai());
        assert!(!envelope_with_type("ai:welfare_rejected").is_proactive_ai());
        assert!(!envelope_with_type("ai:schema_rejected").is_proactive_ai());
        assert!(!envelope_with_type("ai:reflection").is_proactive_ai());
    }

    #[test]
    fn t10_is_proactive_ai_false_for_non_ai() {
        assert!(!envelope_with_type("observation:user_stated").is_proactive_ai());
    }

    // ── Phase D Task 13: is_ai_action autonomy: prefix ────────────────────

    #[test]
    fn t_d13_is_ai_action_true_for_autonomy_prefix() {
        // Round 2: autonomy:state_transitioned + any other autonomy:* action
        // must flow through is_ai_action so the ethics gate fires.
        assert!(envelope_with_type("autonomy:state_transitioned").is_ai_action());
        assert!(envelope_with_type("autonomy:grant_signed").is_ai_action());
        assert!(envelope_with_type("autonomy:death_trigger_armed").is_ai_action());
    }

    #[test]
    fn t_d13_is_ai_action_false_for_other_prefixes_unchanged() {
        // Regression pin: the existing Phase C2 behavior for non-ai
        // non-autonomy events must be preserved.
        assert!(!envelope_with_type("observation:user_stated").is_ai_action());
        assert!(!envelope_with_type("schema:type_defined").is_ai_action());
        assert!(!envelope_with_type("owner:consent_granted").is_ai_action());
        assert!(!envelope_with_type("heirloom:genesis").is_ai_action());
    }

    #[test]
    fn t_d13_is_proactive_ai_true_for_autonomy_state_transitioned() {
        // Composition pin: is_proactive_ai = is_ai_action && !blacklist.
        // autonomy:state_transitioned is not in the blacklist, so after the
        // Task 13 is_ai_action extension it automatically becomes proactive.
        assert!(envelope_with_type("autonomy:state_transitioned").is_proactive_ai());
    }
}
