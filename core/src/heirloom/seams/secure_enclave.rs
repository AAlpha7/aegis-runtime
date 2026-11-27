//! W5 — SecureEnclave + KeyEncryptionKey trait surfaces.
//!
//! Spec §5 Q5.3. Dev impl (`SoftEnclave`, Task 9) stores keys in process
//! memory and emits a loud `tracing::warn!` on construction. Prod impls
//! (P4) front a hardware secure element (Apple SE / Titan / TPM 2.0).
//!
//! **Round-8 P0-1 lock:** `WrapContext.constitution` is a mandatory param
//! — unwrap fails if the constitution hash does not match the wrap-time
//! hash. This is the third link (seal chain) of 宪法 9.a. No KEK impl can
//! silently skip it because the mandatory param is on the trait signature.

use std::sync::Arc;

use super::threshold_signer::ThresholdSigner;
use super::types::{ConstitutionTuple, KekDomain, WrapContext, WrappedKey, ZeroizeReason};

// ── Value types ───────────────────────────────────────────────────────────

/// Remote-attestation proof that the enclave is running expected code.
/// Stub in Phase B (dev impl returns a hardcoded "soft-enclave-dev" tag).
/// P4 prod impls return signed platform evidence (e.g. Apple DeviceCheck,
/// TPM quote).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attestation {
    pub platform: String,
    pub evidence: Vec<u8>,
}

/// Liveness proof — owner has recently interacted and is awake.
/// Stub in Phase B. P4 impls use biometric / device-local signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessProof {
    pub timestamp_ms: u64,
    pub signal: String,
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Wraps/unwraps Bundle DEKs under a domain-specific KEK. Spec §5 Q5.3.
pub trait KeyEncryptionKey: Send + Sync {
    /// Wrap a 32-byte DEK under this KEK. Bound to `context.constitution`.
    fn wrap(&self, dek: &[u8; 32], context: &WrapContext) -> Result<WrappedKey, EnclaveError>;

    /// Unwrap a previously-wrapped DEK. MUST fail if `context.constitution`
    /// does not match the constitution tuple the wrap was performed under.
    fn unwrap(&self, wrapped: &WrappedKey, context: &WrapContext)
    -> Result<[u8; 32], EnclaveError>;

    fn domain(&self) -> KekDomain;
}

/// Secure enclave seam. Dev: `SoftEnclave`. Prod: `AppleSE` / `TitanSE` / `TpmSE`.
pub trait SecureEnclave: Send + Sync {
    fn attestation(&self) -> Result<Attestation, EnclaveError>;

    fn threshold_signer(&self) -> Arc<dyn ThresholdSigner>;

    fn kek(&self, domain: KekDomain) -> Result<Arc<dyn KeyEncryptionKey>, EnclaveError>;

    fn request_zeroize(&self, reason: ZeroizeReason) -> Result<(), EnclaveError>;

    fn verify_liveness(&self) -> Result<LivenessProof, EnclaveError>;

    /// The constitution tuple this enclave is currently sealed under. Returned
    /// so call-sites can pass it into `PayloadCodec::encrypt` without having
    /// to thread it from somewhere else.
    fn current_constitution(&self) -> ConstitutionTuple;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnclaveError {
    #[error("unknown KEK domain requested")]
    UnknownDomain,
    #[error("wrap algorithm {0} not supported by this KEK")]
    UnsupportedAlgorithm(String),
    #[error("constitution tuple drift — unwrap refused")]
    ConstitutionDrift,
    #[error("wrapped-key context mismatch (bundle or generation differs)")]
    ContextMismatch,
    #[error("invalid wrapped-key framing: {0}")]
    InvalidFraming(String),
    #[error("liveness unavailable in this enclave")]
    LivenessUnavailable,
    #[error("attestation unavailable: {0}")]
    AttestationFailed(String),
}

#[cfg(test)]
mod tests {
    // SoftEnclave impl + tests land in Task 9.
}
