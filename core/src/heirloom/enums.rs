//! Envelope enum fields: event type tag, sensitivity tier, payload encoding.
//!
//! All enums use `snake_case` JSON representation for stability. `EventTypeTag`
//! is an open string-backed type (not a closed enum) because P1 introduces
//! ~14 domains × N actions via the `schema:type_defined` ledger — new types
//! are registered at runtime, not at compile time.

use serde::{Deserialize, Serialize};

/// Event type tag: `"domain:action"`. Opaque string; validation happens at
/// the schema-ledger layer (W18), not here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct EventTypeTag(String);

impl EventTypeTag {
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn domain(&self) -> Option<&str> {
        self.0.split_once(':').map(|(d, _)| d)
    }
    pub fn action(&self) -> Option<&str> {
        self.0.split_once(':').map(|(_, a)| a)
    }
}

/// Sensitivity tier (spec §2 Q2.6). Six levels, only ever ascends at write
/// time (fail-safe default = `Sensitive` for unknown categories).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityTier {
    Public,
    Ordinary,
    Sensitive,
    Confidential,
    Secret,
    Sacred,
}

/// Payload encoding (spec §2 Q2.5). Phase A uses only `Plaintext`; AEAD
/// variants arrive in W4 (PayloadCodec).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PayloadEncoding {
    Plaintext,
    #[serde(rename = "aead_v1")]
    AeadV1 {
        algo: String,
        nonce: String,
        key_ref: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t3_event_type_tag_domain_and_action_split() {
        let tag = EventTypeTag::new("observation:user_stated");
        assert_eq!(tag.domain(), Some("observation"));
        assert_eq!(tag.action(), Some("user_stated"));
    }

    #[test]
    fn t3_event_type_tag_no_colon_returns_none() {
        let tag = EventTypeTag::new("malformed");
        assert_eq!(tag.domain(), None);
        assert_eq!(tag.action(), None);
    }

    #[test]
    fn t3_event_type_tag_serde_transparent_string() {
        let tag = EventTypeTag::new("profile:slot_upserted");
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, r#""profile:slot_upserted""#);
    }

    #[test]
    fn t3_sensitivity_tier_snake_case_serde() {
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Sacred).unwrap(),
            r#""sacred""#
        );
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Confidential).unwrap(),
            r#""confidential""#
        );
    }

    #[test]
    fn t3_sensitivity_tier_ordering_matches_severity() {
        assert!(SensitivityTier::Public < SensitivityTier::Ordinary);
        assert!(SensitivityTier::Ordinary < SensitivityTier::Sensitive);
        assert!(SensitivityTier::Sensitive < SensitivityTier::Confidential);
        assert!(SensitivityTier::Confidential < SensitivityTier::Secret);
        assert!(SensitivityTier::Secret < SensitivityTier::Sacred);
    }

    #[test]
    fn t3_sensitivity_tier_roundtrip_all_variants() {
        for tier in [
            SensitivityTier::Public,
            SensitivityTier::Ordinary,
            SensitivityTier::Sensitive,
            SensitivityTier::Confidential,
            SensitivityTier::Secret,
            SensitivityTier::Sacred,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: SensitivityTier = serde_json::from_str(&json).unwrap();
            assert_eq!(back, tier);
        }
    }

    #[test]
    fn t3_payload_encoding_plaintext_tagged() {
        let p = PayloadEncoding::Plaintext;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"{"kind":"plaintext"}"#);
        let back: PayloadEncoding = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn t3_payload_encoding_aead_v1_carries_nonce_and_key_ref() {
        let p = PayloadEncoding::AeadV1 {
            algo: "xchacha20poly1305".to_string(),
            nonce: "010203".to_string(),
            key_ref: "bundle:general:gen1".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""kind":"aead_v1""#));
        assert!(json.contains(r#""algo":"xchacha20poly1305""#));
        let back: PayloadEncoding = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
