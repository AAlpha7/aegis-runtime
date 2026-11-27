//! Canonical serialization per RFC 8785 JCS (JSON Canonicalization Scheme).
//!
//! The envelope's `event_hash` must be deterministic — two independent
//! implementations (now or 300 years from now) must produce identical
//! bytes from structurally-identical envelopes. RFC 8785 specifies:
//!   - object keys sorted lexicographically
//!   - no whitespace
//!   - shortest number representation per ECMA-262
//!   - minimal string escaping
//!
//! We delegate to the `serde_jcs` crate (MIT/Apache).

use crate::heirloom::envelope::EventEnvelope;

/// RFC 8785 canonical bytes for an envelope. Used as BLAKE3 input.
///
/// Returns `serde_json::Error` because `serde_jcs` 0.2.0 delegates its error
/// type to serde_json — it does not expose its own public Error type.
pub fn canonical_bytes(envelope: &EventEnvelope) -> Result<Vec<u8>, serde_json::Error> {
    let s = serde_jcs::to_string(envelope)?;
    Ok(s.into_bytes())
}

/// Compute BLAKE3(canonical_bytes(envelope)). This is the envelope's
/// `event_hash`, used as `prev_hash` on the next event.
pub fn event_hash(envelope: &EventEnvelope) -> Result<[u8; 32], serde_json::Error> {
    let bytes = canonical_bytes(envelope)?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

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
    fn t6_canonical_bytes_keys_sorted_alphabetically() {
        let env = sample_envelope();
        let bytes = canonical_bytes(&env).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // author_did comes before bundle_ids alphabetically; both before event_id.
        let author_pos = s.find(r#""author_did""#).unwrap();
        let bundle_pos = s.find(r#""bundle_ids""#).unwrap();
        let event_pos = s.find(r#""event_id""#).unwrap();
        assert!(author_pos < bundle_pos, "author_did < bundle_ids");
        assert!(bundle_pos < event_pos, "bundle_ids < event_id");
    }

    #[test]
    fn t6_canonical_bytes_no_whitespace() {
        let env = sample_envelope();
        let bytes = canonical_bytes(&env).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // RFC 8785 forbids any whitespace between tokens.
        // Verify no space follows any comma or colon.
        assert!(
            !s.contains(", "),
            "canonical JSON must not have space after comma"
        );
        assert!(
            !s.contains(": "),
            "canonical JSON must not have space after colon"
        );
        assert!(!s.contains('\n'), "canonical JSON must be single-line");
    }

    #[test]
    fn t6_canonical_bytes_deterministic_across_calls() {
        let env = sample_envelope();
        let b1 = canonical_bytes(&env).unwrap();
        let b2 = canonical_bytes(&env).unwrap();
        let b3 = canonical_bytes(&env).unwrap();
        assert_eq!(b1, b2);
        assert_eq!(b2, b3);
    }

    #[test]
    fn t6_canonical_bytes_changes_when_any_field_changes() {
        let env = sample_envelope();
        let b0 = canonical_bytes(&env).unwrap();

        let mut env2 = env.clone();
        env2.schema_version = 2;
        assert_ne!(canonical_bytes(&env2).unwrap(), b0);

        let mut env3 = env.clone();
        env3.prev_hash = [0xBB; 32];
        assert_ne!(canonical_bytes(&env3).unwrap(), b0);

        let mut env4 = env.clone();
        env4.hlc_ts.log = 1;
        assert_ne!(canonical_bytes(&env4).unwrap(), b0);

        let mut env5 = env.clone();
        env5.payload = b"different".to_vec();
        assert_ne!(canonical_bytes(&env5).unwrap(), b0);
    }

    #[test]
    fn t6_event_hash_produces_32_bytes_and_matches_blake3() {
        let env = sample_envelope();
        let bytes = canonical_bytes(&env).unwrap();
        let expected = *blake3::hash(&bytes).as_bytes();
        let actual = event_hash(&env).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 32);
    }
}
