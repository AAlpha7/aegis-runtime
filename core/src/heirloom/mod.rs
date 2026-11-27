//! Heirloom event log substrate (P1 Phase A).
//!
//! This module is the append-only, hash-chained, canonically-serialized
//! event log that backs the Heirloom architecture. It is the single source
//! of truth for every durable write in the system.
//!
//! Spec: `docs/heirloom/specs/2026-04-16-p1-substrate-design.md` §2-§3.
//! Constitutions: `docs/heirloom/constitutions.md` (1 append-only, 3 self-
//! describing, 9 ethics primacy).
//!
//! Phase A scope (W1+W2): Envelope + JCS canonicalization + BLAKE3 hash
//! chain + HLC timestamps + JSONL persistence. No signing, no encryption,
//! no projectors — those arrive in later workstreams.

pub mod canonical;
pub mod enums;
pub mod envelope;
pub mod filter;
pub mod genesis;
pub mod hebbian;
pub mod hlc;
pub mod log;
pub mod owner_state_machine;
pub mod projector;
pub mod projectors;
pub mod query;
pub mod rank;
pub mod reader;
pub mod schema;
pub mod schema_bootstrap;
pub mod seams;
pub mod sensitivity;
pub mod truncate;
pub mod types;
pub mod writer;

#[cfg(test)]
mod tests;

// Public API re-exports
pub use canonical::{canonical_bytes, event_hash};
pub use enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
pub use envelope::EventEnvelope;
pub use genesis::GENESIS_PREV_HASH;
pub use hlc::{Hlc, HlcTimestamp};
pub use log::{HeirloomEventLog, Tip};
pub use owner_state_machine::{
    OwnerStateError, OwnerStateMachine, OwnerStateTransitionPayload, allowed_transitions,
    is_transition_allowed,
};
pub use schema_bootstrap::{
    SCHEMA_TYPE_DEFS, SchemaTypeDefPayload, SchemaTypeDefinition, SchemaTypeDeprecatedPayload,
};
pub use sensitivity::{sensitivity_tier_from_wire, sensitivity_tier_wire_str};
pub use types::{BundleId, Did, LineageId, Signature};
pub use writer::{DbBackedSchemaRegistry, NoopSchemaRegistry, SchemaRegistry};

pub use query::{
    AutonomyGrantProvider, CapabilitySet, CoreOnlyGrantProvider, EntityRef, IntentCategory,
    IntentClass, QueryContext, Scene, TemporalClass,
};

pub use filter::{EventStatus, GateOutcome, RejectReason};
pub use rank::{RankWeights, StoredEvent};
pub use reader::{HeirloomReader, ReaderError};
pub use truncate::Scored;
