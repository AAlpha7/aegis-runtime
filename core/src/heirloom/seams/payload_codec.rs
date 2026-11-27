//! W4 — PayloadCodec trait.
//!
//! Spec §5 Q5.4. Dev: `PlaintextCodec`. Prod P1.5+: `AeadCodec` (XChaCha20Poly1305).
//!
//! **Round-8 P0-1 lock:** `constitution` is a mandatory positional parameter
//! on `encrypt()`. It is NOT optional, NOT in a settings struct, NOT defaulted.
//! Any impl that wants to silently skip the seal-chain link cannot do so
//! because the parameter is on the trait signature itself.

use rand::RngCore;

use super::super::enums::PayloadEncoding;
use super::secure_enclave::SecureEnclave;
use super::types::{BundlePolicy, ConstitutionTuple};

pub trait PayloadCodec: Send + Sync {
    /// Encrypt a plaintext payload for a bundle under a constitution. Returns
    /// the encoding tag to write into the envelope + the ciphertext bytes.
    ///
    /// `rng` is passed in as `&mut dyn RngCore` so the call-site owns the
    /// RNG's seeding (a testable point). Prod AEAD codecs consume randomness
    /// for nonces; dev PlaintextCodec ignores it.
    fn encrypt(
        &self,
        plaintext: &[u8],
        policy: &BundlePolicy,
        constitution: &ConstitutionTuple,
        enclave: &dyn SecureEnclave,
        rng: &mut dyn RngCore,
    ) -> Result<(PayloadEncoding, Vec<u8>), CodecError>;

    /// Decrypt an envelope's payload bytes. Spec §5 Q5.4: the `envelope`
    /// parameter is passed by reference so the codec can derive the expected
    /// `ConstitutionTuple` (`envelope.l0/l1/l2_manifest_hash`) directly — the
    /// caller cannot supply a forged constitution that would bypass the seal
    /// chain. MUST fail (CodecError::ConstitutionDrift) if the sealed
    /// constitution does not equal the envelope's constitution triple.
    fn decrypt(
        &self,
        encoding: &PayloadEncoding,
        ciphertext: &[u8],
        envelope: &super::super::EventEnvelope,
        enclave: &dyn SecureEnclave,
    ) -> Result<Vec<u8>, CodecError>;

    /// The encoding tag this codec emits when `encrypt()` is called.
    /// Call-sites use this to pre-route envelopes at routing time rather
    /// than after encrypt returns.
    fn encoding_tag(&self) -> PayloadEncoding;
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("unsupported payload encoding: {0}")]
    UnsupportedEncoding(String),
    #[error("invalid framing: {0}")]
    InvalidFraming(String),
    #[error("constitution tuple drift — decrypt refused")]
    ConstitutionDrift,
    #[error("enclave error: {0}")]
    Enclave(String),
}
