//! W3 — ThresholdSigner trait + value types.
//!
//! Spec §5 Q5.2. Dev impl: `SoftThresholdSigner` (Ed25519 1-of-1).
//! Prod impl: `FrostThresholdSigner` (FROST-Ed25519 M-of-N) — P4.
//!
//! `AggregateSig::scheme_id` is stored inline with the signature so future
//! readers can pick the correct verifier after scheme upgrades. Historical
//! signatures stay verifiable with the original scheme_id's verifier.

use serde::{Deserialize, Serialize};

use crate::heirloom::types::hex_32;

// ── Value types ───────────────────────────────────────────────────────────

/// Threshold parameters: `m` signers required out of `n` total.
/// Dev: `{m:1, n:1}`. Prod FROST: `{m:3, n:5}` typical.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ThresholdSpec {
    pub m: usize,
    pub n: usize,
}

impl ThresholdSpec {
    pub const fn one_of_one() -> Self {
        Self { m: 1, n: 1 }
    }
}

/// Identifier of a single share participant. Opaque string in Phase B.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ShareId(String);

impl ShareId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Aggregate public key of the signing group. For 1-of-1 dev impl this is
/// simply the underlying Ed25519 verifying key bytes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct GroupPubKey(#[serde(with = "hex_32")] pub [u8; 32]);

impl GroupPubKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// An aggregate signature produced by a `ThresholdSigner`. Carries the
/// scheme_id inline so a future reader can pick the matching verifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateSig {
    /// Scheme tag — `"frost-ed25519-1of1-dev"` for dev; `"frost-ed25519-3of5-prod"`
    /// for a typical P4 prod threshold.
    pub scheme_id: String,
    /// Signature bytes. Ed25519 is 64 bytes; FROST-Ed25519 aggregate is also 64.
    #[serde(with = "hex_vec")]
    pub bytes: Vec<u8>,
    /// Which share IDs participated. `[ShareId("dev")]` for 1-of-1.
    pub participants: Vec<ShareId>,
}

mod hex_vec {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Threshold signer seam. Call-sites hold `Arc<dyn ThresholdSigner>` and
/// never change between dev and prod impls. Spec §5 Q5.2.
pub trait ThresholdSigner: Send + Sync {
    /// Produce an aggregate signature over `message`.
    fn sign(&self, message: &[u8]) -> Result<AggregateSig, SignerError>;

    /// Scheme tag. Matches the scheme_id written into produced `AggregateSig`s.
    fn scheme_id(&self) -> &'static str;

    /// Threshold parameters this signer is operating under.
    fn quorum(&self) -> ThresholdSpec;

    /// Aggregate public key for this group.
    fn group_pubkey(&self) -> GroupPubKey;

    /// Associated-function verify (spec §5 Q5.2). Delegates to the
    /// scheme-keyed free function so future readers without a signer
    /// instance can still verify — callable as
    /// `<SoftThresholdSigner as ThresholdSigner>::verify(...)`.
    ///
    /// `where Self: Sized` keeps the trait object-safe (the method is
    /// inaccessible through `dyn ThresholdSigner`, which is correct —
    /// verification MUST NOT depend on the signer instance state).
    fn verify(pubkey: &GroupPubKey, message: &[u8], sig: &AggregateSig) -> Result<(), SignerError>
    where
        Self: Sized,
    {
        verify_aggregate(pubkey, message, sig)
    }
}

/// Static verification free-function. Mirror of `ThresholdSigner::verify`
/// for call-sites that don't have an impl type in scope (e.g. historical-log
/// re-verification after all signers have been dropped). Both entry points
/// use the same scheme_id dispatch table.
///
/// For Phase B we only handle `"frost-ed25519-1of1-dev"` (plain Ed25519).
/// Future scheme registrations extend this match. Do NOT add a catch-all
/// arm — unknown schemes MUST fail loudly so a future reader sees a clear
/// "add your scheme here" signal.
pub fn verify_aggregate(
    pubkey: &GroupPubKey,
    message: &[u8],
    sig: &AggregateSig,
) -> Result<(), SignerError> {
    match sig.scheme_id.as_str() {
        "frost-ed25519-1of1-dev" => verify_ed25519(pubkey, message, &sig.bytes),
        other => Err(SignerError::UnknownScheme(other.to_string())),
    }
}

fn verify_ed25519(
    pubkey: &GroupPubKey,
    message: &[u8],
    sig_bytes: &[u8],
) -> Result<(), SignerError> {
    use ed25519_dalek::{Signature as EdSig, Verifier, VerifyingKey};
    let vk = VerifyingKey::from_bytes(pubkey.as_bytes())
        .map_err(|e| SignerError::InvalidKey(e.to_string()))?;
    if sig_bytes.len() != 64 {
        return Err(SignerError::InvalidSignature(format!(
            "expected 64 bytes, got {}",
            sig_bytes.len()
        )));
    }
    let mut fixed = [0u8; 64];
    fixed.copy_from_slice(sig_bytes);
    let s = EdSig::from_bytes(&fixed);
    vk.verify(message, &s)
        .map_err(|e| SignerError::VerificationFailed(e.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("unknown signature scheme: {0}")]
    UnknownScheme(String),
    #[error("signing failed: {0}")]
    SigningFailed(String),
    #[error("key generation failed: {0}")]
    KeyGenFailed(String),
}

// ── Scheme registration payload ───────────────────────────────────────────

/// Event-type tag for `schema:signature_scheme_registered` events.
///
/// Written by W10 (Phase C) at bootstrap to announce a signer scheme to the
/// event log. Future readers consult these events to pick the verifier for
/// historical signatures.
pub const SCHEME_REGISTERED_EVENT_TYPE: &str = "schema:signature_scheme_registered";

/// Event-type tag for the paired deprecation event. Emitted when a scheme is
/// retired (e.g. dev -> prod migration). Historical signatures under the
/// deprecated scheme remain verifiable; new signatures MUST NOT use it.
pub const SCHEME_DEPRECATED_EVENT_TYPE: &str = "schema:signature_scheme_deprecated";

/// Payload for a `schema:signature_scheme_registered` event. Spec §5 Q5.2
/// Round-8 P1-2 (lines 1087-1111).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemeRegisteredPayload {
    /// The scheme_id that subsequent `AggregateSig`s will carry inline.
    pub scheme_id: String,
    /// Underlying algorithm family — `"Ed25519"`, `"ECDSA-secp256k1"`, etc.
    pub algorithm: String,
    /// Threshold parameters (m, n).
    pub threshold: ThresholdSpec,
    /// Aggregate group public key for this scheme.
    pub group_pubkey: GroupPubKey,
    /// Pointer to the verifier specification — RFC number, IETF draft,
    /// or a citation that lets a future reader reconstruct the verifier.
    pub verifier_spec_ref: String,
    /// HLC timestamp from which this scheme is valid.
    pub valid_from_hlc: crate::heirloom::HlcTimestamp,
    /// HLC timestamp at which the scheme was deprecated, if ever.
    pub valid_until_hlc: Option<crate::heirloom::HlcTimestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t3_threshold_spec_one_of_one_is_1_1() {
        let s = ThresholdSpec::one_of_one();
        assert_eq!(s.m, 1);
        assert_eq!(s.n, 1);
    }

    #[test]
    fn t3_share_id_roundtrip() {
        let id = ShareId::new("dev-share");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""dev-share""#);
        let back: ShareId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn t3_group_pubkey_roundtrip_is_hex() {
        let pk = GroupPubKey::from_bytes([0xAA; 32]);
        let json = serde_json::to_string(&pk).unwrap();
        assert_eq!(json, format!(r#""{}""#, "aa".repeat(32)));
        let back: GroupPubKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pk);
    }

    #[test]
    fn t3_aggregate_sig_roundtrips_with_scheme_id_inline() {
        let sig = AggregateSig {
            scheme_id: "frost-ed25519-1of1-dev".to_string(),
            bytes: vec![0xDE; 64],
            participants: vec![ShareId::new("dev")],
        };
        let json = serde_json::to_string(&sig).unwrap();
        let back: AggregateSig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sig);
    }

    #[test]
    fn t3_verify_aggregate_unknown_scheme_errors() {
        let pk = GroupPubKey::from_bytes([0u8; 32]);
        let sig = AggregateSig {
            scheme_id: "pgp-v2".to_string(),
            bytes: vec![0u8; 64],
            participants: vec![],
        };
        let err = verify_aggregate(&pk, b"msg", &sig).unwrap_err();
        match err {
            SignerError::UnknownScheme(s) => assert_eq!(s, "pgp-v2"),
            other => panic!("expected UnknownScheme, got {other:?}"),
        }
    }

    #[test]
    fn t3_scheme_registered_payload_roundtrips_with_all_fields() {
        let p = SchemeRegisteredPayload {
            scheme_id: "frost-ed25519-1of1-dev".to_string(),
            algorithm: "Ed25519".to_string(),
            threshold: ThresholdSpec::one_of_one(),
            group_pubkey: GroupPubKey::from_bytes([0x11; 32]),
            verifier_spec_ref: "RFC 8032".to_string(),
            valid_from_hlc: crate::heirloom::HlcTimestamp {
                phys: 1_729_000_000_000,
                log: 0,
            },
            valid_until_hlc: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: SchemeRegisteredPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn t3_scheme_registered_event_type_constants() {
        assert_eq!(
            SCHEME_REGISTERED_EVENT_TYPE,
            "schema:signature_scheme_registered"
        );
        assert_eq!(
            SCHEME_DEPRECATED_EVENT_TYPE,
            "schema:signature_scheme_deprecated"
        );
    }
}
