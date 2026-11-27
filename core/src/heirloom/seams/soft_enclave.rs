//! W5 dev impl — `SoftEnclave` + `SoftKek`.
//!
//! **Never use in production.** Keys live in process memory.
//!
//! Implements the seal-chain check (Round-8 P0-1) with an explicit framed
//! layout that stores the wrap-time `ConstitutionTuple` alongside the DEK:
//!
//! ```text
//! wrapped.ciphertext =
//!     bundle_tag[..16]      // blake3(bundle_id)[..16]
//!  || generation_be[..4]     // u32 big-endian
//!  || l0_hash[..32]
//!  || l1_hash[..32]
//!  || l2_manifest_hash[..32]
//!  || dek[..32]              // 32 bytes of the wrapped DEK
//! ```
//!
//! Unwrap reads the stored tuple and compares it to `context.constitution`
//! under `subtle::ConstantTimeEq`, returning `ConstitutionDrift` when those
//! 96 bytes differ and `ContextMismatch` when bundle/generation differ.

use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;

use super::super::BundleId;
use super::secure_enclave::{
    Attestation, EnclaveError, KeyEncryptionKey, LivenessProof, SecureEnclave,
};
use super::soft_signer::SoftThresholdSigner;
use super::threshold_signer::ThresholdSigner;
use super::types::{ConstitutionTuple, KekDomain, WrapContext, WrappedKey, ZeroizeReason};
use super::zeroize_hook::{InMemoryZeroizer, ZeroizeHook};

const SOFT_WRAP_ALGORITHM: &str = "soft-kek-v1";
const BUNDLE_TAG_LEN: usize = 16;
const CONSTITUTION_HASH_LEN: usize = 32;
const WRAP_BODY_LEN: usize = BUNDLE_TAG_LEN + 4 + 3 * CONSTITUTION_HASH_LEN + 32;

/// Dev impl of `KeyEncryptionKey`. Stores the wrap-time ConstitutionTuple
/// alongside the DEK so unwrap can distinguish `ConstitutionDrift` from
/// `ContextMismatch` for any non-zero constitution triple.
pub struct SoftKek {
    domain: KekDomain,
}

impl SoftKek {
    pub fn new(domain: KekDomain) -> Self {
        Self { domain }
    }

    fn bundle_tag(bundle: &BundleId) -> [u8; BUNDLE_TAG_LEN] {
        let digest = *blake3::hash(bundle.as_str().as_bytes()).as_bytes();
        let mut out = [0u8; BUNDLE_TAG_LEN];
        out.copy_from_slice(&digest[..BUNDLE_TAG_LEN]);
        out
    }
}

impl KeyEncryptionKey for SoftKek {
    fn wrap(&self, dek: &[u8; 32], context: &WrapContext) -> Result<WrappedKey, EnclaveError> {
        let mut framed = Vec::with_capacity(WRAP_BODY_LEN);
        framed.extend_from_slice(&Self::bundle_tag(&context.bundle_id));
        framed.extend_from_slice(&context.generation.to_be_bytes());
        framed.extend_from_slice(&context.constitution_hash.l0_hash);
        framed.extend_from_slice(&context.constitution_hash.l1_hash);
        framed.extend_from_slice(&context.constitution_hash.l2_manifest_hash);
        framed.extend_from_slice(dek);
        Ok(WrappedKey {
            algorithm: SOFT_WRAP_ALGORITHM.to_string(),
            ciphertext: framed,
            domain: self.domain,
        })
    }

    fn unwrap(
        &self,
        wrapped: &WrappedKey,
        context: &WrapContext,
    ) -> Result<[u8; 32], EnclaveError> {
        if wrapped.algorithm != SOFT_WRAP_ALGORITHM {
            return Err(EnclaveError::UnsupportedAlgorithm(
                wrapped.algorithm.clone(),
            ));
        }
        if wrapped.domain != self.domain {
            return Err(EnclaveError::ContextMismatch);
        }
        if wrapped.ciphertext.len() != WRAP_BODY_LEN {
            return Err(EnclaveError::InvalidFraming(format!(
                "expected {} bytes, got {}",
                WRAP_BODY_LEN,
                wrapped.ciphertext.len()
            )));
        }

        // Slice layout (offsets):
        //   0..16       bundle_tag
        //   16..20      generation (big-endian u32)
        //   20..52      l0_hash
        //   52..84      l1_hash
        //   84..116     l2_manifest_hash
        //   116..148    dek
        let bytes = &wrapped.ciphertext;
        let stored_bundle = &bytes[0..BUNDLE_TAG_LEN];
        let stored_gen = u32::from_be_bytes(
            bytes[BUNDLE_TAG_LEN..BUNDLE_TAG_LEN + 4]
                .try_into()
                .expect("slice len checked above"),
        );
        let stored_l0 = &bytes[20..52];
        let stored_l1 = &bytes[52..84];
        let stored_l2 = &bytes[84..116];
        let dek_bytes = &bytes[116..148];

        // Constitution-drift check is BEFORE bundle/generation so the error
        // variant maps cleanly: if the caller updated their constitution but
        // kept bundle/gen, they get ConstitutionDrift, which is the most
        // diagnostic variant for the seal-chain scenario.
        let got_ctuple = [stored_l0, stored_l1, stored_l2].concat();
        let want_ctuple = [
            &context.constitution_hash.l0_hash[..],
            &context.constitution_hash.l1_hash[..],
            &context.constitution_hash.l2_manifest_hash[..],
        ]
        .concat();
        if got_ctuple.ct_eq(&want_ctuple).unwrap_u8() == 0 {
            return Err(EnclaveError::ConstitutionDrift);
        }

        let expected_bundle = Self::bundle_tag(&context.bundle_id);
        if stored_bundle.ct_eq(&expected_bundle[..]).unwrap_u8() == 0
            || stored_gen != context.generation
        {
            return Err(EnclaveError::ContextMismatch);
        }

        let mut out = [0u8; 32];
        out.copy_from_slice(dek_bytes);
        Ok(out)
    }

    fn domain(&self) -> KekDomain {
        self.domain
    }
}

/// Dev impl of `SecureEnclave`. Holds a single `SoftThresholdSigner` plus one
/// `SoftKek` per enumerated `KekDomain`. On construction emits a loud
/// `tracing::warn!` so operators can never miss they're in dev mode.
pub struct SoftEnclave {
    signer: Arc<dyn ThresholdSigner>,
    keks: HashMap<KekDomain, Arc<dyn KeyEncryptionKey>>,
    zeroizer: Arc<dyn ZeroizeHook>,
    constitution: RwLock<ConstitutionTuple>,
    liveness_clock_ms: Mutex<u64>,
}

impl SoftEnclave {
    pub fn new(
        signer: Arc<dyn ThresholdSigner>,
        zeroizer: Arc<dyn ZeroizeHook>,
        constitution: ConstitutionTuple,
    ) -> Self {
        tracing::warn!(
            "SoftEnclave instantiated — DEV ONLY, keys live in process memory, do not use in production"
        );
        let mut keks: HashMap<KekDomain, Arc<dyn KeyEncryptionKey>> = HashMap::new();
        for d in [
            KekDomain::UserData,
            KekDomain::Constitution,
            KekDomain::Loyalty,
            KekDomain::Session,
        ] {
            keks.insert(d, Arc::new(SoftKek::new(d)));
        }
        Self {
            signer,
            keks,
            zeroizer,
            constitution: RwLock::new(constitution),
            liveness_clock_ms: Mutex::new(0),
        }
    }

    /// Convenience for tests: everything wired up with a single seed.
    pub fn new_dev(seed: u64) -> Self {
        let signer = Arc::new(SoftThresholdSigner::new_deterministic(seed));
        let zeroizer = Arc::new(InMemoryZeroizer::new());
        Self::new(signer, zeroizer, ConstitutionTuple::zeros())
    }

    /// Swap the constitution tuple. Owner-authorized operation only — Phase C
    /// will gate this via owner command event; here it's unconditional for tests.
    pub fn rotate_constitution(&self, new_tuple: ConstitutionTuple) {
        *self.constitution.write() = new_tuple;
    }

    /// Advance the internal liveness clock (tests only).
    #[cfg(test)]
    pub fn set_liveness_clock_ms(&self, ms: u64) {
        *self.liveness_clock_ms.lock() = ms;
    }
}

impl SecureEnclave for SoftEnclave {
    fn attestation(&self) -> Result<Attestation, EnclaveError> {
        Ok(Attestation {
            platform: "soft-enclave-dev".to_string(),
            evidence: b"unattested-dev-build".to_vec(),
        })
    }

    fn threshold_signer(&self) -> Arc<dyn ThresholdSigner> {
        Arc::clone(&self.signer)
    }

    fn kek(&self, domain: KekDomain) -> Result<Arc<dyn KeyEncryptionKey>, EnclaveError> {
        self.keks
            .get(&domain)
            .cloned()
            .ok_or(EnclaveError::UnknownDomain)
    }

    fn request_zeroize(&self, reason: ZeroizeReason) -> Result<(), EnclaveError> {
        self.zeroizer
            .zeroize_all(reason)
            .map_err(|e| EnclaveError::AttestationFailed(format!("zeroize: {e}")))
    }

    fn verify_liveness(&self) -> Result<LivenessProof, EnclaveError> {
        let ts = *self.liveness_clock_ms.lock();
        Ok(LivenessProof {
            timestamp_ms: ts,
            signal: "soft-enclave-always-live".to_string(),
        })
    }

    fn current_constitution(&self) -> ConstitutionTuple {
        *self.constitution.read()
    }
}

impl std::fmt::Debug for SoftEnclave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftEnclave")
            .field("signer_scheme", &self.signer.scheme_id())
            .field("kek_domains", &self.keks.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_context(bundle: &str) -> WrapContext {
        WrapContext {
            bundle_id: BundleId::new(bundle),
            generation: 1,
            constitution_hash: ConstitutionTuple::zeros(),
        }
    }

    #[test]
    fn t5_soft_enclave_new_dev_exposes_all_four_domains() {
        let e = SoftEnclave::new_dev(1);
        for d in [
            KekDomain::UserData,
            KekDomain::Constitution,
            KekDomain::Loyalty,
            KekDomain::Session,
        ] {
            let kek = e.kek(d).expect("domain KEK must exist");
            assert_eq!(kek.domain(), d);
        }
    }

    #[test]
    fn t5_soft_enclave_attestation_is_soft_dev_stub() {
        let e = SoftEnclave::new_dev(1);
        let a = e.attestation().unwrap();
        assert_eq!(a.platform, "soft-enclave-dev");
    }

    #[test]
    fn t5_soft_enclave_threshold_signer_scheme_is_dev() {
        let e = SoftEnclave::new_dev(42);
        let s = e.threshold_signer();
        assert_eq!(s.scheme_id(), "frost-ed25519-1of1-dev");
    }

    #[test]
    fn t5_soft_kek_wrap_then_unwrap_same_context_returns_original_dek() {
        let e = SoftEnclave::new_dev(1);
        let kek = e.kek(KekDomain::UserData).unwrap();
        let dek = [0x7Fu8; 32];
        let ctx = dev_context("bundle:general");
        let wrapped = kek.wrap(&dek, &ctx).unwrap();
        let unwrapped = kek.unwrap(&wrapped, &ctx).unwrap();
        assert_eq!(unwrapped, dek);
    }

    #[test]
    fn t5_soft_kek_unwrap_rejects_constitution_drift_from_zero() {
        let e = SoftEnclave::new_dev(1);
        let kek = e.kek(KekDomain::UserData).unwrap();
        let dek = [0x33u8; 32];
        let wrap_ctx = WrapContext {
            bundle_id: BundleId::new("bundle:general"),
            generation: 1,
            constitution_hash: ConstitutionTuple::zeros(),
        };
        let wrapped = kek.wrap(&dek, &wrap_ctx).unwrap();

        let drifted_ctx = WrapContext {
            constitution_hash: ConstitutionTuple {
                l0_hash: [0xFF; 32],
                l1_hash: [0xFF; 32],
                l2_manifest_hash: [0xFF; 32],
            },
            ..wrap_ctx.clone()
        };
        let err = kek.unwrap(&wrapped, &drifted_ctx).unwrap_err();
        assert_eq!(err, EnclaveError::ConstitutionDrift);
    }

    #[test]
    fn t5_soft_kek_unwrap_rejects_constitution_drift_between_nonzero_tuples() {
        // Regression guard for the CTO-round-1 probe-narrowing bug: wrapping
        // under a non-zero constitution and then unwrapping under a different
        // non-zero constitution must still produce ConstitutionDrift, not
        // ContextMismatch.
        let e = SoftEnclave::new_dev(1);
        let kek = e.kek(KekDomain::UserData).unwrap();
        let dek = [0x11u8; 32];
        let wrap_ctuple = ConstitutionTuple {
            l0_hash: [0xAA; 32],
            l1_hash: [0xBB; 32],
            l2_manifest_hash: [0xCC; 32],
        };
        let wrap_ctx = WrapContext {
            bundle_id: BundleId::new("bundle:general"),
            generation: 1,
            constitution_hash: wrap_ctuple,
        };
        let wrapped = kek.wrap(&dek, &wrap_ctx).unwrap();

        // Flip only the constitution (not bundle, not generation).
        let drifted_ctuple = ConstitutionTuple {
            l0_hash: [0xAA; 32],          // same
            l1_hash: [0xBB; 32],          // same
            l2_manifest_hash: [0xDE; 32], // FLIPPED
        };
        let drifted_ctx = WrapContext {
            constitution_hash: drifted_ctuple,
            ..wrap_ctx.clone()
        };
        let err = kek.unwrap(&wrapped, &drifted_ctx).unwrap_err();
        assert_eq!(
            err,
            EnclaveError::ConstitutionDrift,
            "drift between two non-zero tuples must still be detected"
        );

        // Cross-check: identical non-zero tuples roundtrip successfully.
        let back = kek.unwrap(&wrapped, &wrap_ctx).unwrap();
        assert_eq!(back, dek);
    }

    #[test]
    fn t5_soft_kek_unwrap_rejects_bundle_drift() {
        let e = SoftEnclave::new_dev(1);
        let kek = e.kek(KekDomain::UserData).unwrap();
        let dek = [0x77u8; 32];
        let wrap_ctx = dev_context("bundle:alpha");
        let wrapped = kek.wrap(&dek, &wrap_ctx).unwrap();

        let mut drifted = wrap_ctx.clone();
        drifted.bundle_id = BundleId::new("bundle:beta");
        let err = kek.unwrap(&wrapped, &drifted).unwrap_err();
        assert_eq!(err, EnclaveError::ContextMismatch);
    }

    #[test]
    fn t5_soft_kek_unwrap_rejects_generation_drift() {
        let e = SoftEnclave::new_dev(1);
        let kek = e.kek(KekDomain::UserData).unwrap();
        let dek = [0x44u8; 32];
        let wrap_ctx = dev_context("bundle:general");
        let wrapped = kek.wrap(&dek, &wrap_ctx).unwrap();

        let mut drifted = wrap_ctx.clone();
        drifted.generation = 99;
        let err = kek.unwrap(&wrapped, &drifted).unwrap_err();
        assert_eq!(err, EnclaveError::ContextMismatch);
    }

    #[test]
    fn t5_soft_kek_unwrap_rejects_wrong_domain_wrapped_key() {
        let e = SoftEnclave::new_dev(1);
        let kek_ud = e.kek(KekDomain::UserData).unwrap();
        let kek_ct = e.kek(KekDomain::Constitution).unwrap();
        let dek = [0x11u8; 32];
        let ctx = dev_context("bundle:general");
        let wrapped_by_ud = kek_ud.wrap(&dek, &ctx).unwrap();
        // Try to unwrap with a different-domain KEK.
        let err = kek_ct.unwrap(&wrapped_by_ud, &ctx).unwrap_err();
        assert_eq!(err, EnclaveError::ContextMismatch);
    }

    #[test]
    fn t5_soft_enclave_rotate_constitution_changes_current() {
        let e = SoftEnclave::new_dev(1);
        assert_eq!(e.current_constitution(), ConstitutionTuple::zeros());
        let new_tuple = ConstitutionTuple {
            l0_hash: [0x1; 32],
            l1_hash: [0x2; 32],
            l2_manifest_hash: [0x3; 32],
        };
        e.rotate_constitution(new_tuple);
        assert_eq!(e.current_constitution(), new_tuple);
    }

    #[test]
    fn t5_soft_enclave_verify_liveness_returns_clock_timestamp() {
        let e = SoftEnclave::new_dev(1);
        e.set_liveness_clock_ms(1_729_123_456_789);
        let proof = e.verify_liveness().unwrap();
        assert_eq!(proof.timestamp_ms, 1_729_123_456_789);
    }
}
