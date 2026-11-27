//! P1 Phase B — crypto + loyalty seams.
//!
//! Six trait surfaces defined by spec §5 (2026-04-16 round 6):
//!
//! - [`ThresholdSigner`] — Ed25519 signing (dev: 1-of-1; prod: FROST M-of-N).
//! - [`PayloadCodec`] — encrypt/decrypt envelope payloads (dev: plaintext).
//! - [`SecureEnclave`] — key custody (dev: soft-key-on-disk; prod: hardware SE).
//! - [`EthicsValidator`] — 宪法 9 gate (dev: 3 hard rejects; prod: canonical).
//! - [`WelfareOracle`] — 宪法 8 gate, Alive only (dev: noop; prod: time series).
//! - [`ZeroizeHook`] — tamper/duress memory wipe (dev: in-memory; prod: SE).
//!
//! Call-sites (W10 HeirloomWriter) hold `Arc<dyn Seam>` and never change when
//! the impl is swapped dev↔prod. See spec §5 opening diagram.
//!
//! **Non-goal:** none of these seams writes to the event log. The controller
//! (W10) is the sole writer; seams only return verdicts / bytes.

pub mod types;

pub mod soft_signer;
pub mod threshold_signer;

pub mod secure_enclave;
pub mod soft_enclave;

pub mod payload_codec;
pub mod plaintext_codec;

pub mod ethics_validator;
pub mod permissive_ethics;

pub mod welfare_oracle;

pub mod zeroize_hook;

#[cfg(test)]
mod tests;

// Public re-exports. Kept flat so call-sites `use heirloom::seams::{...}`.
pub use types::{
    BundlePolicy, ConstitutionTuple, EventHash, KekDomain, WrapContext, WrappedKey, ZeroizeReason,
};

pub use soft_signer::SoftThresholdSigner;
pub use threshold_signer::{
    AggregateSig, GroupPubKey, ShareId, SignerError, ThresholdSigner, ThresholdSpec,
    verify_aggregate,
};

pub use secure_enclave::{
    Attestation, EnclaveError, KeyEncryptionKey, LivenessProof, SecureEnclave,
};
pub use soft_enclave::{SoftEnclave, SoftKek};

pub use payload_codec::{CodecError, PayloadCodec};
pub use plaintext_codec::PlaintextCodec;

pub use ethics_validator::{
    EditionPolicy, EscalationFallback, EthicsContext, EthicsRuleProposal, EthicsValidator,
    EthicsVerdict, InsentientKind, L2Manifest, Lens, LensJudgment, LensVerdict, PassageSource,
    PenanceWeight, ReferencePassage, RejectEventType, RuleVerdict, TargetAudience,
};
pub use permissive_ethics::{HARD_REJECT_EVENT_TYPES, PermissiveEthics};

pub use welfare_oracle::{
    NoopWelfareOracle, WelfareError, WelfareIndicator, WelfareObservation, WelfareOracle,
    WelfareScore,
};

pub use zeroize_hook::{
    InMemoryZeroizer, MaterialHandle, TamperTrigger, ZeroizableMaterial, ZeroizeError, ZeroizeHook,
};
