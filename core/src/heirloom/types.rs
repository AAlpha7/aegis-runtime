//! Opaque newtype wrappers for DID / lineage / bundle / signature.
//!
//! These are stubs in Phase A — full impls arrive in W3 (threshold signer),
//! W5 (key management), and W9 (bundle projector). Here they carry bytes
//! through envelope serde and that is all.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Decentralized identifier. Stringly-typed for now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Did(String);

impl Did {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lineage identifier. Each Heirloom instance has exactly one at genesis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct LineageId(String);

impl LineageId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bundle identifier. Used in envelope `bundle_ids` for scoping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct BundleId(String);

impl BundleId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Aggregate signature bytes. Phase A stores bytes only; W3 parses by scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(Vec<u8>);

impl Signature {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

// Serialize as lowercase hex string (compact, canonical, JCS-friendly).
impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        Ok(Self(bytes))
    }
}

/// Lifecycle state of the owner with respect to the Heirloom.
///
/// **Spec literal** — five variants exactly per spec §4 (lines 786-792) + §5 Q5.5
/// (`EthicsContext.owner_state`). Phase C §4 recall gates use these specific
/// names; any rename is a breaking change across the substrate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OwnerState {
    /// Owner alive and reachable. No gate restrictions.
    Alive,
    /// Owner temporarily unreachable (brief absence). §4: Gate 3 filters out
    /// "主动发起" event types from result suggestions; read-only OK.
    AbsentSoft,
    /// Owner long-unreachable (prolonged absence). §4: Gate 3 restricts to
    /// `bundle:core` + bundles relevant to standing Continuation missions.
    AbsentHard,
    /// Owner no longer reachable; Heirloom honors standing instructions.
    /// §4: Gate 3 filters event `bundle_ids` against `AutonomyGrant.allowed_bundles`.
    /// Welfare gate disabled; ethics Escalate uses `EscalationFallback`.
    Continuation,
    /// Heirloom is archived / read-only; only legitimate successors may access
    /// bundles matching their own grant scope. §4: Gate 3 strictest.
    SoulArchive,
}

/// Where an observation or action was witnessed.
///
/// **Spec literal** — closed 7-variant enum per spec §3 Q3.4 line 534. Do not
/// add variants in Phase B. Phase C adds new channels via schema:type_defined
/// events if needed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WitnessChannel {
    Telegram,
    Voice,
    Email,
    Mcp,
    Browser,
    Cron,
    Migration,
}

/// The minimal description of a proposed AI action that any seam may receive
/// for evaluation. Full envelope is not passed — seams should never need
/// access to the encrypted payload, only the verdict-relevant fields.
///
/// `payload_digest` is `blake3(canonical_payload_bytes)` so ethics/welfare
/// can reason about identity-of-payload (deduplication, retry detection)
/// without ever seeing plaintext.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedAction {
    /// Fully-qualified event type — `"ai:message_sent"`, `"ai:tool_call"`, etc.
    pub event_type: String,
    /// The DID that will be author of the resulting event.
    pub author: Did,
    /// Optional DID of the recipient / subject. `None` for self-directed or
    /// ambient-observation actions.
    pub target: Option<Did>,
    /// Short natural-language summary of what is being proposed. Used by
    /// `CanonicalEthics` in P4; ignored by `PermissiveEthics`.
    pub summary: String,
    /// `blake3(canonical_payload_bytes)` — seams can reason about payload
    /// identity without access to plaintext.
    #[serde(with = "hex_32")]
    pub payload_digest: [u8; 32],
}

pub(crate) mod hex_32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub(crate) fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let raw = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if raw.len() != 32 {
            return Err(serde::de::Error::custom(format!(
                "expected 32 bytes of hex, got {}",
                raw.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&raw);
        Ok(out)
    }
}

/// Standing autonomy grant from the owner. Phase B only carries it as a
/// value type; W10 HeirloomWriter will enforce it. Phase C will add
/// revocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutonomyGrant {
    /// Stable identifier — `blake3(canonical_bytes(grant))` for audit.
    pub id: String,
    /// What the AI is authorized to do, free-form scope tag.
    pub scope: String,
    /// When the grant was issued (authoring envelope's HLC).
    pub granted_at_hlc: crate::heirloom::HlcTimestamp,
    /// Optional expiry — `None` means indefinite-until-revoked.
    pub expires_at_hlc: Option<crate::heirloom::HlcTimestamp>,
    /// DIDs the grant applies to (empty = unrestricted target).
    pub targets: Vec<Did>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t2_did_new_preserves_string() {
        let did = Did::new("did:aegis:telegram:4242");
        assert_eq!(did.as_str(), "did:aegis:telegram:4242");
    }

    #[test]
    fn t2_did_serde_roundtrip_is_plain_string() {
        let did = Did::new("did:aegis:local:alice");
        let json = serde_json::to_string(&did).unwrap();
        assert_eq!(json, r#""did:aegis:local:alice""#);
        let back: Did = serde_json::from_str(&json).unwrap();
        assert_eq!(back, did);
    }

    #[test]
    fn t2_lineage_id_new_and_display() {
        let lid = LineageId::new("lineage:daisy:001");
        assert_eq!(lid.as_str(), "lineage:daisy:001");
    }

    #[test]
    fn t2_bundle_id_equality_is_structural() {
        let a = BundleId::new("bundle:general");
        let b = BundleId::new("bundle:general");
        let c = BundleId::new("bundle:other");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn t2_signature_holds_opaque_bytes_and_roundtrips_hex() {
        let sig = Signature::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let json = serde_json::to_string(&sig).unwrap();
        assert_eq!(json, r#""deadbeef""#);
        let back: Signature = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_bytes(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn t2_owner_state_serde_roundtrips_as_snake_case() {
        let s = OwnerState::Alive;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#""alive""#);
        let back: OwnerState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);

        let j2 = serde_json::to_string(&OwnerState::Continuation).unwrap();
        assert_eq!(j2, r#""continuation""#);
    }

    #[test]
    fn t2_witness_channel_serializes_snake_case_per_spec() {
        assert_eq!(
            serde_json::to_string(&WitnessChannel::Telegram).unwrap(),
            r#""telegram""#,
        );
        assert_eq!(
            serde_json::to_string(&WitnessChannel::Migration).unwrap(),
            r#""migration""#,
        );
        let back: WitnessChannel = serde_json::from_str(r#""voice""#).unwrap();
        assert_eq!(back, WitnessChannel::Voice);
    }

    #[test]
    fn t2_proposed_action_roundtrip_preserves_event_type_and_target() {
        let a = ProposedAction {
            event_type: "ai:message_sent".to_string(),
            author: Did::new("did:aegis:self"),
            target: Some(Did::new("did:aegis:telegram:4242")),
            summary: "reply with weather forecast".to_string(),
            payload_digest: [0u8; 32],
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: ProposedAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }
}
