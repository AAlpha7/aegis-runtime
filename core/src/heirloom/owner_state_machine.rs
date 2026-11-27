//! Owner State Machine — reconstructs current owner state from the event
//! log and validates transitions.
//!
//! Spec: docs/heirloom/specs/2026-04-16-p1-substrate-design.md §5 Loyalty.
//! OwnerState literal variants: types.rs (5 variants, spec-locked).
//!
//! Phase D Task 6-7. Wire event_type is `autonomy:state_transitioned`
//! (spec §2 Q2.3 line 172 / D7); Rust type stays `OwnerStateTransitionPayload`
//! for readability.

use crate::heirloom::types::OwnerState;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Wire-shape of an `autonomy:state_transitioned` payload.
///
/// Per spec §2 Q2.3 line 172, the state-transition event lives under the
/// `autonomy:` domain (Continuation / 继任). The Rust type is still named
/// `OwnerStateTransitionPayload` for readability — it carries the
/// `OwnerState` enum — but the on-wire event_type string is
/// `autonomy:state_transitioned`. See D7.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerStateTransitionPayload {
    pub from: OwnerState,
    pub to: OwnerState,
    /// Free-form audit string (D4). Welfare oracle may structure it in P2+.
    pub reason: String,
    /// Wall-clock at transition authorship. For audit display only; the
    /// authoritative timestamp is the envelope's `hlc_ts`.
    pub timestamp_rfc3339: String,
}

/// All transitions permitted by P1 (D5). Key = from, value = vec of allowed to.
/// SoulArchive is terminal (empty vec).
pub fn allowed_transitions(from: OwnerState) -> &'static [OwnerState] {
    match from {
        OwnerState::Alive => &[
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard,
            OwnerState::Continuation,
            OwnerState::SoulArchive,
        ],
        OwnerState::AbsentSoft => &[OwnerState::Alive, OwnerState::AbsentHard],
        OwnerState::AbsentHard => &[OwnerState::Alive, OwnerState::Continuation],
        OwnerState::Continuation => &[OwnerState::Alive, OwnerState::SoulArchive],
        OwnerState::SoulArchive => &[], // terminal
    }
}

pub fn is_transition_allowed(from: OwnerState, to: OwnerState) -> bool {
    if from == to {
        return false; // no-op transitions rejected; force explicit idempotency at call site
    }
    allowed_transitions(from).contains(&to)
}

#[derive(Debug, Error)]
pub enum OwnerStateError {
    #[error("illegal state transition: {from:?} → {to:?}")]
    IllegalTransition { from: OwnerState, to: OwnerState },
    #[error("writer error: {0}")]
    Writer(#[from] crate::heirloom::writer::WriterError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("canonical parse error: {0}")]
    Canonical(String),
}

// ── OwnerStateMachine (Task 7) ────────────────────────────────────────────

use crate::heirloom::seams::ConstitutionTuple;
use crate::heirloom::types::{Did, LineageId};
use crate::heirloom::writer::{HeirloomWriter, OwnerStateProvider};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

/// Live owner state machine. Reads initial state from the event log on
/// construction (cold-start recovery) and records transitions via the
/// Writer. Thread-safe via an inner Mutex.
#[derive(Debug)]
pub struct OwnerStateMachine {
    current: Mutex<OwnerState>,
    #[allow(dead_code)] // retained for diagnostic / future restart without writer
    conn: Arc<Mutex<Connection>>,
    lineage: LineageId,
    owner_did: Did,
    constitution: ConstitutionTuple,
}

impl OwnerStateMachine {
    /// Read the most recent `autonomy:state_transitioned` event in the log
    /// (for the given lineage) and set the current state to its `to` field.
    /// If no such event exists, default to `Alive`.
    ///
    /// Per spec §2 Q2.3 line 172 the event_type is `autonomy:state_transitioned`
    /// (D7). Do not change the SELECT without also updating Task 1
    /// `SCHEMA_TYPE_DEFS`, Task 6 envelope builder, and the semantic-critical
    /// test `t_d3_owner_state_machine_reads_latest_transition_event_on_startup`.
    pub fn new_from_log(
        conn: Arc<Mutex<Connection>>,
        lineage: LineageId,
        owner_did: Did,
        constitution: ConstitutionTuple,
    ) -> Result<Self, OwnerStateError> {
        let initial = {
            let c = conn.lock();
            Self::read_latest_state(&c, lineage.as_str())?
        };
        Ok(Self {
            current: Mutex::new(initial),
            conn,
            lineage,
            owner_did,
            constitution,
        })
    }

    fn read_latest_state(
        conn: &Connection,
        lineage_id: &str,
    ) -> Result<OwnerState, OwnerStateError> {
        let row: rusqlite::Result<Vec<u8>> = conn.query_row(
            "SELECT payload FROM heirloom_event_log
             WHERE event_type = 'autonomy:state_transitioned' AND lineage_id = ?1
             ORDER BY hlc_phys DESC, hlc_log DESC, event_hash DESC
             LIMIT 1",
            rusqlite::params![lineage_id],
            |r| r.get(0),
        );
        match row {
            Ok(payload) => {
                // The stored payload column goes through the Writer's payload
                // codec (PlaintextCodec frames with `aegis:plaintext-v1:{JCS
                // constitution header}\n{raw payload}`; AeadCodec encrypts).
                // For P1 dev with PlaintextCodec we strip the framing by
                // looking for the first `\n` and decoding the remainder as
                // the raw payload JSON. If any non-plaintext framing leaks
                // into P1 the decode falls back to raw-JSON parse.
                let raw_json = decode_plaintext_framed_or_raw(&payload);
                let parsed: OwnerStateTransitionPayload = serde_json::from_slice(raw_json)
                    .map_err(|e| OwnerStateError::Canonical(format!("payload: {e}")))?;
                Ok(parsed.to)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(OwnerState::Alive),
            Err(e) => Err(OwnerStateError::Sqlite(e)),
        }
    }

    pub fn current(&self) -> OwnerState {
        *self.current.lock()
    }

    /// Validate + append a transition event. On success, mutate the
    /// in-memory state; on failure (illegal transition or writer error),
    /// leave state unchanged.
    pub fn transition(
        &self,
        writer: &HeirloomWriter,
        to: OwnerState,
        reason: &str,
    ) -> Result<(), OwnerStateError> {
        let from = self.current();
        if !is_transition_allowed(from, to) {
            return Err(OwnerStateError::IllegalTransition { from, to });
        }
        let payload = OwnerStateTransitionPayload {
            from,
            to,
            reason: reason.to_string(),
            timestamp_rfc3339: chrono::Utc::now().to_rfc3339(),
        };
        let env = writer.build_owner_state_transitioned_envelope(
            &self.lineage,
            &self.owner_did,
            &self.constitution,
            &payload,
        );
        writer.append_event(env)?;
        *self.current.lock() = to;
        Ok(())
    }
}

impl OwnerStateProvider for OwnerStateMachine {
    fn current(&self) -> OwnerState {
        self.current()
    }
}

/// Strip PlaintextCodec framing (`aegis:plaintext-v1:{header}\n{raw}`) from
/// a stored `payload` blob, returning the raw payload bytes. If the blob
/// does NOT start with the frame tag (e.g. the writer was configured with
/// a different codec, or the payload was inserted directly via a raw SQL
/// test fixture), the blob is returned unchanged — callers can still
/// attempt a raw JSON decode.
fn decode_plaintext_framed_or_raw(blob: &[u8]) -> &[u8] {
    const PLAINTEXT_FRAME_TAG: &[u8] = b"aegis:plaintext-v1:";
    if !blob.starts_with(PLAINTEXT_FRAME_TAG) {
        return blob;
    }
    let rest = &blob[PLAINTEXT_FRAME_TAG.len()..];
    match rest.iter().position(|&b| b == b'\n') {
        Some(nl) => &rest[nl + 1..],
        None => blob, // malformed framing — let caller attempt raw decode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_d6_alive_can_go_anywhere() {
        for to in [
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard,
            OwnerState::Continuation,
            OwnerState::SoulArchive,
        ] {
            assert!(
                is_transition_allowed(OwnerState::Alive, to),
                "Alive → {to:?} must be allowed"
            );
        }
    }

    #[test]
    fn t_d6_soul_archive_is_terminal() {
        for to in [
            OwnerState::Alive,
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard,
            OwnerState::Continuation,
        ] {
            assert!(
                !is_transition_allowed(OwnerState::SoulArchive, to),
                "SoulArchive → {to:?} must be rejected"
            );
        }
    }

    #[test]
    fn t_d6_absent_soft_paths() {
        assert!(is_transition_allowed(
            OwnerState::AbsentSoft,
            OwnerState::Alive
        ));
        assert!(is_transition_allowed(
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard
        ));
        assert!(!is_transition_allowed(
            OwnerState::AbsentSoft,
            OwnerState::Continuation
        ));
        assert!(!is_transition_allowed(
            OwnerState::AbsentSoft,
            OwnerState::SoulArchive
        ));
    }

    #[test]
    fn t_d6_absent_hard_paths() {
        assert!(is_transition_allowed(
            OwnerState::AbsentHard,
            OwnerState::Alive
        ));
        assert!(is_transition_allowed(
            OwnerState::AbsentHard,
            OwnerState::Continuation
        ));
        assert!(!is_transition_allowed(
            OwnerState::AbsentHard,
            OwnerState::AbsentSoft
        ));
        assert!(!is_transition_allowed(
            OwnerState::AbsentHard,
            OwnerState::SoulArchive
        ));
    }

    #[test]
    fn t_d6_continuation_paths() {
        assert!(is_transition_allowed(
            OwnerState::Continuation,
            OwnerState::Alive
        ));
        assert!(is_transition_allowed(
            OwnerState::Continuation,
            OwnerState::SoulArchive
        ));
        assert!(!is_transition_allowed(
            OwnerState::Continuation,
            OwnerState::AbsentSoft
        ));
        assert!(!is_transition_allowed(
            OwnerState::Continuation,
            OwnerState::AbsentHard
        ));
    }

    #[test]
    fn t_d6_no_self_transition() {
        for state in [
            OwnerState::Alive,
            OwnerState::AbsentSoft,
            OwnerState::AbsentHard,
            OwnerState::Continuation,
            OwnerState::SoulArchive,
        ] {
            assert!(
                !is_transition_allowed(state, state),
                "self-transition {state:?} → {state:?} must be rejected"
            );
        }
    }

    #[test]
    fn t_d6_payload_roundtrip() {
        let p = OwnerStateTransitionPayload {
            from: OwnerState::Alive,
            to: OwnerState::AbsentSoft,
            reason: "unreachable 24h".to_string(),
            timestamp_rfc3339: "2026-04-18T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: OwnerStateTransitionPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
