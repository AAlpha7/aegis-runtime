//! Heirloom event log: append-only, hash-chained, tip-tracked.
//!
//! Phase A provides the in-memory and JSONL-file shapes. W10 (HeirloomWriter)
//! will port this behind a transactional SQLite-backed writer; the trait
//! here is what recalls will use once that lands.

use crate::heirloom::canonical::event_hash;
use crate::heirloom::envelope::EventEnvelope;
use crate::heirloom::genesis::GENESIS_PREV_HASH;
use crate::heirloom::hlc::HlcTimestamp;
use serde::de::Error as _;

/// Head of the chain: latest event's hash + HLC. Genesis-unconfigured log
/// returns `None` for tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tip {
    pub hash: [u8; 32],
    pub hlc: HlcTimestamp,
}

/// In-memory append-only event log. JSONL persistence is a separate
/// concern (Task 11).
#[derive(Debug, Default)]
pub struct HeirloomEventLog {
    events: Vec<EventEnvelope>,
    tip: Option<Tip>,
}

/// Errors from log operations.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("event prev_hash {actual:02x?} does not match tip hash {expected:02x?}")]
    ChainBreak {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    #[error("event hlc {actual:?} does not exceed tip hlc {expected:?}")]
    HlcRegression {
        expected: HlcTimestamp,
        actual: HlcTimestamp,
    },
    #[error("genesis event must have prev_hash == GENESIS_PREV_HASH, got {actual:02x?}")]
    InvalidGenesis { actual: [u8; 32] },
    #[error("canonical serialization failed: {0}")]
    Canonical(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
}

impl HeirloomEventLog {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            tip: None,
        }
    }

    /// Current tip (latest appended event's hash + HLC). `None` if empty.
    pub fn tip(&self) -> Option<Tip> {
        self.tip
    }

    /// Number of appended events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Borrow the full event vector (tests + replay).
    pub fn events(&self) -> &[EventEnvelope] {
        &self.events
    }

    /// Append an event. Validates:
    ///   - First event's prev_hash == GENESIS_PREV_HASH
    ///   - Subsequent events' prev_hash == previous event's computed hash
    ///   - HLC strictly increases across the chain
    ///
    /// On success, returns the appended event's hash.
    pub fn append(&mut self, envelope: EventEnvelope) -> Result<[u8; 32], LogError> {
        // Validate chain link
        let expected_prev = match self.tip {
            None => GENESIS_PREV_HASH,
            Some(t) => t.hash,
        };
        if envelope.prev_hash != expected_prev {
            if self.tip.is_none() {
                return Err(LogError::InvalidGenesis {
                    actual: envelope.prev_hash,
                });
            }
            return Err(LogError::ChainBreak {
                expected: expected_prev,
                actual: envelope.prev_hash,
            });
        }

        // Validate HLC monotonicity
        if let Some(t) = self.tip
            && envelope.hlc_ts <= t.hlc
        {
            return Err(LogError::HlcRegression {
                expected: t.hlc,
                actual: envelope.hlc_ts,
            });
        }

        // Compute this event's hash (seals the envelope)
        let new_hash = event_hash(&envelope).map_err(|e| LogError::Canonical(e.to_string()))?;
        let new_hlc = envelope.hlc_ts;
        self.events.push(envelope);
        self.tip = Some(Tip {
            hash: new_hash,
            hlc: new_hlc,
        });
        Ok(new_hash)
    }

    /// Validate the chain integrity of an externally-supplied event vector.
    /// Used by replay/audit paths where events are parsed from storage and
    /// must be re-verified before trust.
    pub fn verify_chain(events: &[EventEnvelope]) -> Result<(), LogError> {
        let mut expected_prev = GENESIS_PREV_HASH;
        let mut prev_hlc: Option<HlcTimestamp> = None;
        for (i, ev) in events.iter().enumerate() {
            if ev.prev_hash != expected_prev {
                return Err(if i == 0 {
                    LogError::InvalidGenesis {
                        actual: ev.prev_hash,
                    }
                } else {
                    LogError::ChainBreak {
                        expected: expected_prev,
                        actual: ev.prev_hash,
                    }
                });
            }
            if let Some(p) = prev_hlc
                && ev.hlc_ts <= p
            {
                return Err(LogError::HlcRegression {
                    expected: p,
                    actual: ev.hlc_ts,
                });
            }
            expected_prev = event_hash(ev).map_err(|e| LogError::Canonical(e.to_string()))?;
            prev_hlc = Some(ev.hlc_ts);
        }
        Ok(())
    }

    /// Persist events to a JSONL file (one envelope per line). Overwrites
    /// any existing content. Atomicity is NOT guaranteed at this layer —
    /// callers that need crash-safe writes should write to a temp file and
    /// rename (W10 HeirloomWriter will do that).
    pub fn write_jsonl(&self, path: &std::path::Path) -> Result<(), LogError> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        for ev in &self.events {
            let line = serde_json::to_string(ev)?;
            writeln!(f, "{line}")?;
        }
        Ok(())
    }

    /// Read events from a JSONL file and rebuild the log. Every event is
    /// re-validated via verify_chain before the log is returned — a tampered
    /// file fails loudly.
    pub fn read_jsonl(path: &std::path::Path) -> Result<Self, LogError> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        for (line_no, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let ev: EventEnvelope = serde_json::from_str(line)
                .map_err(|e| serde_json::Error::custom(format!("line {}: {}", line_no + 1, e)))?;
            events.push(ev);
        }
        Self::verify_chain(&events)?;
        let tip = match events.last() {
            None => None,
            Some(last) => Some(Tip {
                hash: event_hash(last).map_err(|e| LogError::Canonical(e.to_string()))?,
                hlc: last.hlc_ts,
            }),
        };
        Ok(Self { events, tip })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    fn make_event(prev_hash: [u8; 32], hlc: HlcTimestamp, payload: &[u8]) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash,
            hlc_ts: hlc,
            observed_at: DateTime::parse_from_rfc3339("2026-04-17T09:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0x10; 32],
            l1_hash: [0x20; 32],
            l2_manifest_hash: [0x30; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![Signature::new(vec![0x00])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn t9_empty_log_has_no_tip_and_len_zero() {
        let log = HeirloomEventLog::new();
        assert!(log.tip().is_none());
        assert_eq!(log.len(), 0);
        assert!(log.is_empty());
    }

    #[test]
    fn t9_genesis_append_sets_tip() {
        let mut log = HeirloomEventLog::new();
        let ev = make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 1000, log: 0 },
            b"first",
        );
        let hash = log.append(ev).unwrap();
        let tip = log.tip().unwrap();
        assert_eq!(tip.hash, hash);
        assert_eq!(tip.hlc, HlcTimestamp { phys: 1000, log: 0 });
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn t9_second_event_must_reference_first_hash() {
        let mut log = HeirloomEventLog::new();
        let ev1 = make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 1000, log: 0 },
            b"first",
        );
        let h1 = log.append(ev1).unwrap();
        let ev2 = make_event(h1, HlcTimestamp { phys: 1001, log: 0 }, b"second");
        let h2 = log.append(ev2).unwrap();
        assert_ne!(h1, h2);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn t9_first_event_with_wrong_prev_hash_rejected_as_invalid_genesis() {
        let mut log = HeirloomEventLog::new();
        let ev = make_event([0xFF; 32], HlcTimestamp { phys: 1, log: 0 }, b"bogus");
        let err = log.append(ev).unwrap_err();
        assert!(matches!(err, LogError::InvalidGenesis { .. }));
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn t9_second_event_with_wrong_prev_hash_rejected_as_chain_break() {
        let mut log = HeirloomEventLog::new();
        let ev1 = make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 1, log: 0 },
            b"first",
        );
        let _ = log.append(ev1).unwrap();
        let ev2 = make_event([0xBB; 32], HlcTimestamp { phys: 2, log: 0 }, b"bad");
        let err = log.append(ev2).unwrap_err();
        assert!(matches!(err, LogError::ChainBreak { .. }));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn t9_hlc_regression_rejected() {
        let mut log = HeirloomEventLog::new();
        let ev1 = make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 1000, log: 0 },
            b"first",
        );
        let h1 = log.append(ev1).unwrap();
        let ev2 = make_event(h1, HlcTimestamp { phys: 500, log: 0 }, b"second");
        let err = log.append(ev2).unwrap_err();
        assert!(matches!(err, LogError::HlcRegression { .. }));
    }

    #[test]
    fn t9_hlc_tie_rejected() {
        let mut log = HeirloomEventLog::new();
        let hlc = HlcTimestamp { phys: 1000, log: 0 };
        let ev1 = make_event(GENESIS_PREV_HASH, hlc, b"first");
        let h1 = log.append(ev1).unwrap();
        // Same hlc — must be strictly greater.
        let ev2 = make_event(h1, hlc, b"second");
        let err = log.append(ev2).unwrap_err();
        assert!(matches!(err, LogError::HlcRegression { .. }));
    }

    #[test]
    fn t9_events_accessor_preserves_order() {
        let mut log = HeirloomEventLog::new();
        let ev1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = log.append(ev1).unwrap();
        let ev2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        let h2 = log.append(ev2).unwrap();
        let ev3 = make_event(h2, HlcTimestamp { phys: 3, log: 0 }, b"three");
        let _ = log.append(ev3).unwrap();

        let events = log.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].payload, b"one");
        assert_eq!(events[1].payload, b"two");
        assert_eq!(events[2].payload, b"three");
    }

    #[test]
    fn t10_verify_chain_empty_is_ok() {
        let result = HeirloomEventLog::verify_chain(&[]);
        assert!(result.is_ok());
    }

    #[test]
    fn t10_verify_chain_single_valid_genesis() {
        let events = vec![make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 1, log: 0 },
            b"g",
        )];
        assert!(HeirloomEventLog::verify_chain(&events).is_ok());
    }

    #[test]
    fn t10_verify_chain_single_invalid_genesis_fails() {
        let events = vec![make_event(
            [0xFF; 32],
            HlcTimestamp { phys: 1, log: 0 },
            b"bad",
        )];
        let err = HeirloomEventLog::verify_chain(&events).unwrap_err();
        assert!(matches!(err, LogError::InvalidGenesis { .. }));
    }

    #[test]
    fn t10_verify_chain_three_valid_events() {
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = event_hash(&e1).unwrap();
        let e2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        let h2 = event_hash(&e2).unwrap();
        let e3 = make_event(h2, HlcTimestamp { phys: 3, log: 0 }, b"three");
        let events = vec![e1, e2, e3];
        assert!(HeirloomEventLog::verify_chain(&events).is_ok());
    }

    #[test]
    fn t10_verify_chain_detects_middle_tamper() {
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = event_hash(&e1).unwrap();
        let mut e2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        // Compute honest h2 first…
        let h2_honest = event_hash(&e2).unwrap();
        // …then tamper e2's payload AFTER its hash was derived.
        e2.payload = b"tampered".to_vec();
        let e3 = make_event(h2_honest, HlcTimestamp { phys: 3, log: 0 }, b"three");
        let events = vec![e1, e2, e3];
        let err = HeirloomEventLog::verify_chain(&events).unwrap_err();
        assert!(matches!(err, LogError::ChainBreak { .. }));
    }

    #[test]
    fn t10_verify_chain_detects_broken_link() {
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let e2 = make_event(
            [0xDE; 32], // wrong prev_hash
            HlcTimestamp { phys: 2, log: 0 },
            b"two",
        );
        let events = vec![e1, e2];
        let err = HeirloomEventLog::verify_chain(&events).unwrap_err();
        assert!(matches!(err, LogError::ChainBreak { .. }));
    }

    #[test]
    fn t10_verify_chain_detects_hlc_regression() {
        let e1 = make_event(
            GENESIS_PREV_HASH,
            HlcTimestamp { phys: 100, log: 0 },
            b"one",
        );
        let h1 = event_hash(&e1).unwrap();
        let e2 = make_event(h1, HlcTimestamp { phys: 50, log: 0 }, b"two");
        let events = vec![e1, e2];
        let err = HeirloomEventLog::verify_chain(&events).unwrap_err();
        assert!(matches!(err, LogError::HlcRegression { .. }));
    }

    use tempfile::NamedTempFile;

    #[test]
    fn t11_write_jsonl_empty_log_produces_empty_file() {
        let log = HeirloomEventLog::new();
        let file = NamedTempFile::new().unwrap();
        log.write_jsonl(file.path()).unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn t11_write_jsonl_one_event_per_line() {
        let mut log = HeirloomEventLog::new();
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = log.append(e1).unwrap();
        let e2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        let _ = log.append(e2).unwrap();
        let file = NamedTempFile::new().unwrap();
        log.write_jsonl(file.path()).unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line must parse as a full envelope.
        for line in lines {
            assert!(serde_json::from_str::<EventEnvelope>(line).is_ok());
        }
    }

    #[test]
    fn t11_read_jsonl_roundtrips_empty_file() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "").unwrap();
        let log = HeirloomEventLog::read_jsonl(file.path()).unwrap();
        assert_eq!(log.len(), 0);
        assert!(log.tip().is_none());
    }

    #[test]
    fn t11_read_jsonl_reconstructs_chain_and_tip() {
        let mut orig = HeirloomEventLog::new();
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = orig.append(e1).unwrap();
        let e2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        let h2 = orig.append(e2).unwrap();

        let file = NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();

        let replay = HeirloomEventLog::read_jsonl(file.path()).unwrap();
        assert_eq!(replay.len(), 2);
        let tip = replay.tip().unwrap();
        assert_eq!(tip.hash, h2);
        assert_eq!(tip.hlc, HlcTimestamp { phys: 2, log: 0 });
    }

    #[test]
    fn t11_read_jsonl_rejects_tampered_event() {
        // NOTE: verify_chain only checks LINKS (prev_hash → event_hash), so we
        // need at least 3 events and tamper a non-tail one to trigger
        // ChainBreak. (See p5 property test for the same reasoning.)
        let mut orig = HeirloomEventLog::new();
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let h1 = orig.append(e1).unwrap();
        let e2 = make_event(h1, HlcTimestamp { phys: 2, log: 0 }, b"two");
        let h2 = orig.append(e2).unwrap();
        let e3 = make_event(h2, HlcTimestamp { phys: 3, log: 0 }, b"three");
        let _ = orig.append(e3).unwrap();

        let file = NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();

        // Corrupt: flip the payload bytes on line 2 (the middle event).
        let content = std::fs::read_to_string(file.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // Replace the payload hex from "two" (74776f) with "tax" (746178).
        let line2_tampered = lines[1].replacen("74776f", "746178", 1);
        let tampered = format!("{}\n{}\n{}\n", lines[0], line2_tampered, lines[2]);
        std::fs::write(file.path(), tampered).unwrap();

        let err = HeirloomEventLog::read_jsonl(file.path()).unwrap_err();
        assert!(matches!(err, LogError::ChainBreak { .. }));
    }

    #[test]
    fn t11_read_jsonl_rejects_invalid_json_line() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "not json\n").unwrap();
        let err = HeirloomEventLog::read_jsonl(file.path()).unwrap_err();
        assert!(matches!(err, LogError::Json(_)));
    }

    #[test]
    fn t11_write_then_read_100_events_preserves_all() {
        let mut orig = HeirloomEventLog::new();
        let mut prev = GENESIS_PREV_HASH;
        for i in 0..100 {
            let ev = make_event(
                prev,
                HlcTimestamp {
                    phys: 1 + i as u64,
                    log: 0,
                },
                format!("event-{i}").as_bytes(),
            );
            prev = orig.append(ev).unwrap();
        }

        let file = NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();
        let replay = HeirloomEventLog::read_jsonl(file.path()).unwrap();
        assert_eq!(replay.len(), 100);
        for (i, ev) in replay.events().iter().enumerate() {
            assert_eq!(ev.payload, format!("event-{i}").as_bytes());
        }
    }

    #[test]
    fn t11_read_jsonl_tolerates_trailing_newline() {
        let mut orig = HeirloomEventLog::new();
        let e1 = make_event(GENESIS_PREV_HASH, HlcTimestamp { phys: 1, log: 0 }, b"one");
        let _ = orig.append(e1).unwrap();
        let file = NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();

        // Add an extra blank line at the end.
        let mut content = std::fs::read_to_string(file.path()).unwrap();
        content.push('\n');
        content.push('\n');
        std::fs::write(file.path(), content).unwrap();

        let replay = HeirloomEventLog::read_jsonl(file.path()).unwrap();
        assert_eq!(replay.len(), 1);
    }
}
