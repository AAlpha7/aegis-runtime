//! Shared value types used by multiple seams.
//!
//! Types used by a single seam live in that seam's file. Types here are
//! referenced by at least two seams to avoid cyclic imports.

use crate::heirloom::types::hex_32;
use serde::{Deserialize, Serialize};

use super::super::BundleId;

/// `blake3(canonical_bytes(envelope))` — stable identifier of a past event.
///
/// Separate from the envelope's own `event_id` UUID; `EventHash` is the
/// content-address used by hash chains, projectors, and signatures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct EventHash(#[serde(with = "hex_32")] pub [u8; 32]);

impl EventHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Triple of constitution layer hashes bound to every encrypt/decrypt call.
///
/// Spec §5 Q5.3 + Q5.4 Round-8 P0-1: pass explicitly to `PayloadCodec::encrypt`
/// so no impl can silently skip the seal-chain link. All three are `blake3`
/// digests; `l2_manifest_hash` digests the merkle root of the sanzang manifest.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ConstitutionTuple {
    #[serde(with = "hex_32")]
    pub l0_hash: [u8; 32],
    #[serde(with = "hex_32")]
    pub l1_hash: [u8; 32],
    #[serde(with = "hex_32")]
    pub l2_manifest_hash: [u8; 32],
}

impl ConstitutionTuple {
    /// Convenience for tests — all-zero triple.
    pub const fn zeros() -> Self {
        Self {
            l0_hash: [0u8; 32],
            l1_hash: [0u8; 32],
            l2_manifest_hash: [0u8; 32],
        }
    }
}

/// Policy bound to a bundle that governs how its payloads are encrypted.
///
/// P1 stub — only carries the ID and encoding selector. P4 will add
/// rotation schedule, allowed recipients, etc. Spec §5 Q5.4.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BundlePolicy {
    pub bundle_id: BundleId,
    /// Which encoding a new event in this bundle should use. Dev: `"plaintext"`.
    /// Prod AeadCodec: `"aead-v1"`. Codecs self-select based on this tag.
    pub preferred_encoding: String,
}

/// Which KEK should wrap a given DEK. Spec §5 Q5.1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KekDomain {
    /// Daily conversation, observations, learned facts.
    UserData,
    /// Sacred-tier constitution-level material. Never leaves SE in prod.
    Constitution,
    /// Autonomy grants, death triggers, succession events.
    Loyalty,
    /// Short-TTL decrypt cache (default 4h). Only layer allowed to persist KEK.
    Session,
}

/// Context for a single `wrap` / `unwrap` call. Bound to `ConstitutionTuple`
/// via spec §5 Q5.3 line 1142 — if the constitution hash drifts, unwrap MUST
/// fail. Field named `constitution_hash` exactly per spec Q5.3; the encrypt
/// signature parameter at Q5.4 uses short form `constitution:` for brevity,
/// but on persisted/reified data structures we keep the spec-literal name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WrapContext {
    pub bundle_id: BundleId,
    /// Monotonic per-bundle DEK generation. Rotation increments this.
    pub generation: u32,
    pub constitution_hash: ConstitutionTuple,
}

/// Output of `KeyEncryptionKey::wrap`. Opaque to callers; only the owning KEK
/// knows the wrap algorithm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedKey {
    /// Tag identifying the wrap algorithm (`"soft-kek-v1"`, `"aes-kw-256"`, etc.).
    pub algorithm: String,
    /// Ciphertext of the wrapped DEK including any algorithm-specific framing.
    #[serde(with = "hex_vec")]
    pub ciphertext: Vec<u8>,
    /// The domain the wrapping KEK belongs to. Redundant with `WrapContext`
    /// but carried on the wire for audit / cross-check.
    pub domain: KekDomain,
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

/// Why a zeroize is being requested. Spec §5 Q5.7.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ZeroizeReason {
    OwnerCommand,
    TamperDetected,
    DuressTriggered,
    CriticalBreach,
    LivenessLostCritical,
}
