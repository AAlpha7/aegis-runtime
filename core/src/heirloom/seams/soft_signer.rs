//! W3 dev impl — `SoftThresholdSigner` (Ed25519 1-of-1).
//!
//! Holds a single Ed25519 SigningKey. For tests, use `new_deterministic(seed)`.
//! Production flows load the key via `from_signing_key(...)` after decrypting
//! a passphrase-protected key file — that decryption lives in `SoftEnclave`.
//!
//! **rand_core version conflict workaround**: `ed25519-dalek 2.x` depends on
//! `rand_core 0.6` (via `curve25519-dalek`), while `rand 0.9` uses `rand_core 0.9`.
//! `SigningKey::generate(&mut rng)` will NOT compile when the rng is from
//! `rand 0.9`. We follow the repo's canonical pattern at
//! `core/src/identity.rs:132-142`: fill 32 bytes via `getrandom::fill` (or from
//! a test-seeded `StdRng`) and call `SigningKey::from_bytes(&raw_32)`.

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use rand::RngCore;
use rand::SeedableRng;
use rand::rngs::StdRng;

use super::threshold_signer::{
    AggregateSig, GroupPubKey, ShareId, SignerError, ThresholdSigner, ThresholdSpec,
};

/// Dev impl — 1-of-1 Ed25519. `scheme_id = "frost-ed25519-1of1-dev"`.
///
/// **Never use in production.** The secret key lives in process memory.
/// Pairs with `SoftEnclave` in W5 for soft-key-on-disk dev flows.
pub struct SoftThresholdSigner {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl SoftThresholdSigner {
    /// Deterministic constructor for tests. Same seed -> same key -> same sigs.
    /// Uses `StdRng::seed_from_u64(seed)` to fill 32 bytes then
    /// `SigningKey::from_bytes` — NOT `SigningKey::generate(&mut rng)`, which
    /// would require rand_core 0.6 compat (see module-level doc).
    pub fn new_deterministic(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut key_bytes = [0u8; 32];
        rng.fill_bytes(&mut key_bytes);
        let signing_key = SigningKey::from_bytes(&key_bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Production-style constructor using OS CSPRNG. Mirrors the repo's
    /// canonical pattern at `core/src/identity.rs:132-142`.
    pub fn new_from_os_rng() -> Result<Self, SignerError> {
        let mut key_bytes = [0u8; 32];
        getrandom::fill(&mut key_bytes)
            .map_err(|e| SignerError::KeyGenFailed(format!("getrandom: {e}")))?;
        let signing_key = SigningKey::from_bytes(&key_bytes);
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            signing_key,
            verifying_key,
        })
    }

    /// Wrap an externally-loaded key. Used by `SoftEnclave` after decrypting
    /// a passphrase-protected key file.
    pub fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Raw access to the underlying verifying key for tests / external use.
    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Build the `schema:signature_scheme_registered` payload for this
    /// signer. W10 (Phase C) calls this at bootstrap and appends the
    /// resulting payload as an event on the log. See spec §5 Q5.2
    /// Round-8 P1-2 for the "log is self-describing" rationale.
    pub fn build_registration_payload(
        &self,
        valid_from_hlc: crate::heirloom::HlcTimestamp,
    ) -> super::threshold_signer::SchemeRegisteredPayload {
        super::threshold_signer::SchemeRegisteredPayload {
            scheme_id: self.scheme_id().to_string(),
            algorithm: "Ed25519".to_string(),
            threshold: self.quorum(),
            group_pubkey: self.group_pubkey(),
            verifier_spec_ref: "RFC 8032".to_string(),
            valid_from_hlc,
            valid_until_hlc: None,
        }
    }
}

impl std::fmt::Debug for SoftThresholdSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never leak secret key bytes.
        f.debug_struct("SoftThresholdSigner")
            .field("scheme_id", &self.scheme_id())
            .field("verifying_key", &hex::encode(self.verifying_key_bytes()))
            .finish()
    }
}

impl ThresholdSigner for SoftThresholdSigner {
    fn sign(&self, message: &[u8]) -> Result<AggregateSig, SignerError> {
        let sig = self.signing_key.sign(message);
        Ok(AggregateSig {
            scheme_id: self.scheme_id().to_string(),
            bytes: sig.to_bytes().to_vec(),
            participants: vec![ShareId::new("dev")],
        })
    }

    fn scheme_id(&self) -> &'static str {
        "frost-ed25519-1of1-dev"
    }

    fn quorum(&self) -> ThresholdSpec {
        ThresholdSpec::one_of_one()
    }

    fn group_pubkey(&self) -> GroupPubKey {
        GroupPubKey::from_bytes(self.verifying_key_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::super::threshold_signer::verify_aggregate;
    use super::*;

    #[test]
    fn t3_soft_signer_scheme_id_is_dev_1of1() {
        let s = SoftThresholdSigner::new_deterministic(1);
        assert_eq!(s.scheme_id(), "frost-ed25519-1of1-dev");
    }

    #[test]
    fn t3_soft_signer_quorum_is_1_of_1() {
        let s = SoftThresholdSigner::new_deterministic(1);
        assert_eq!(s.quorum(), ThresholdSpec::one_of_one());
    }

    #[test]
    fn t3_soft_signer_sign_then_verify_succeeds() {
        let signer = SoftThresholdSigner::new_deterministic(42);
        let msg = b"aegis:heirloom:event:abc";
        let sig = signer.sign(msg).unwrap();
        verify_aggregate(&signer.group_pubkey(), msg, &sig).expect("verify must succeed");
    }

    #[test]
    fn t3_soft_signer_verify_fails_on_tampered_message() {
        let signer = SoftThresholdSigner::new_deterministic(42);
        let sig = signer.sign(b"original").unwrap();
        let err = verify_aggregate(&signer.group_pubkey(), b"tampered", &sig);
        assert!(err.is_err(), "verify must fail on different message");
    }

    #[test]
    fn t3_soft_signer_verify_fails_on_tampered_signature_bytes() {
        let signer = SoftThresholdSigner::new_deterministic(42);
        let msg = b"original";
        let mut sig = signer.sign(msg).unwrap();
        // Flip one byte of the signature.
        sig.bytes[0] ^= 0xFF;
        let err = verify_aggregate(&signer.group_pubkey(), msg, &sig);
        assert!(err.is_err(), "verify must fail on bit-flipped signature");
    }

    #[test]
    fn t3_soft_signer_same_seed_produces_same_key() {
        let a = SoftThresholdSigner::new_deterministic(99);
        let b = SoftThresholdSigner::new_deterministic(99);
        assert_eq!(a.verifying_key_bytes(), b.verifying_key_bytes());
    }

    #[test]
    fn t3_soft_signer_different_seeds_produce_different_keys() {
        let a = SoftThresholdSigner::new_deterministic(1);
        let b = SoftThresholdSigner::new_deterministic(2);
        assert_ne!(a.verifying_key_bytes(), b.verifying_key_bytes());
    }

    #[test]
    fn t3_soft_signer_sig_is_64_bytes_participants_include_dev() {
        let s = SoftThresholdSigner::new_deterministic(7);
        let sig = s.sign(b"x").unwrap();
        assert_eq!(sig.bytes.len(), 64);
        assert_eq!(sig.participants, vec![ShareId::new("dev")]);
    }

    #[test]
    fn t3_soft_signer_signs_empty_message_successfully() {
        let s = SoftThresholdSigner::new_deterministic(0);
        let sig = s.sign(b"").unwrap();
        assert_eq!(sig.bytes.len(), 64);
        verify_aggregate(&s.group_pubkey(), b"", &sig).expect("empty msg sig must verify");
    }

    #[test]
    fn t3_soft_signer_build_registration_payload_matches_signer_state() {
        let signer = SoftThresholdSigner::new_deterministic(42);
        let valid_from = crate::heirloom::HlcTimestamp {
            phys: 1_729_000_000_000,
            log: 0,
        };
        let p = signer.build_registration_payload(valid_from);
        assert_eq!(p.scheme_id, "frost-ed25519-1of1-dev");
        assert_eq!(p.algorithm, "Ed25519");
        assert_eq!(p.threshold, ThresholdSpec::one_of_one());
        assert_eq!(p.group_pubkey, signer.group_pubkey());
        assert_eq!(p.verifier_spec_ref, "RFC 8032");
        assert_eq!(p.valid_from_hlc, valid_from);
        assert!(p.valid_until_hlc.is_none());
    }

    #[test]
    fn t3_soft_signer_registration_payload_pubkey_matches_verify_key() {
        let signer = SoftThresholdSigner::new_deterministic(7);
        let p =
            signer.build_registration_payload(crate::heirloom::HlcTimestamp { phys: 1, log: 0 });
        // The pubkey in the registration payload MUST be the same bytes a
        // future reader would feed to verify_aggregate() — this is the spec's
        // "log is self-describing" invariant.
        assert_eq!(p.group_pubkey.as_bytes(), &signer.verifying_key_bytes());
    }
}
