//! Hebbian projection + real-time access_count bump. Spec §4 Q4.5.
//!
//! access_count updates are authoritative via the projection table.
//! Daily batches emit `ai:access_summary` events for 50-year audit.
//! Phase C3 defines the batch event type + builder; the scheduler that
//! fires it is Phase E (Non-Goals).

use crate::heirloom::hlc::HlcTimestamp;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bump access_count for a single event_hash. Called ONLY when
/// `ctx.update_hebbian == true`. Upserts the row; idempotent-safe for
/// repeated calls.
pub fn bump_access_count(
    conn: &Connection,
    event_hash: &[u8; 32],
    now_rfc3339: &str,
) -> rusqlite::Result<u64> {
    conn.execute(
        "INSERT INTO heirloom_access_count (event_hash, count, last_hit_rfc3339)
         VALUES (?1, 1, ?2)
         ON CONFLICT(event_hash) DO UPDATE SET
           count = count + 1,
           last_hit_rfc3339 = excluded.last_hit_rfc3339",
        rusqlite::params![event_hash.as_slice(), now_rfc3339],
    )?;
    let new_count: i64 = conn.query_row(
        "SELECT count FROM heirloom_access_count WHERE event_hash = ?1",
        rusqlite::params![event_hash.as_slice()],
        |r| r.get(0),
    )?;
    Ok(new_count as u64)
}

/// Read the current access_count (for rank's Hebbian term).
pub fn read_access_count(conn: &Connection, event_hash: &[u8; 32]) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT count FROM heirloom_access_count WHERE event_hash = ?1",
        rusqlite::params![event_hash.as_slice()],
        |r| r.get::<_, i64>(0),
    )
    .map(|c| c as u64)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(other),
    })
}

/// Daily batch summary event payload. Spec §4 Q4.5: one event per day,
/// lists per-event access deltas. Phase C3 defines the shape; Phase E
/// wires the scheduler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessSummaryPayload {
    /// ISO-8601 date (YYYY-MM-DD) the summary covers.
    pub date: String,
    /// Map from hex-encoded event_hash to access delta for that day.
    pub access_deltas: HashMap<String, u64>,
    /// HLC at which the summary is emitted.
    pub emitted_at_hlc: HlcTimestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::schema::ensure_tables;
    use rusqlite::Connection;

    fn fresh_conn_with_event(hash: &[u8; 32]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        // Insert a minimal event_log row so FK to heirloom_access_count holds
        // when ensure_tables is upgraded to enable foreign keys in Phase D.
        conn.execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version,
                event_type, hlc_phys, hlc_log, observed_at_rfc3339,
                l0_hash, l1_hash, l2_manifest_hash, lineage_id, author_did,
                sensitivity_tier, bundle_ids_json, payload_encoding_json,
                payload, signatures_json, envelope_canonical, inserted_at_unix_ms
             ) VALUES (
                ?1, X'00', '00000000-0000-0000-0000-000000000001', 1,
                'observation:test', 1, 0, '2026-04-18T00:00:00Z',
                X'00', X'00', X'00', 'lineage:test', 'did:aegis:test',
                'ordinary', '[]', '{}', X'00', '[]', X'00', 0
             )",
            rusqlite::params![hash.as_slice()],
        )
        .unwrap();
        conn
    }

    #[test]
    fn t11_bump_access_count_creates_row_on_first_hit() {
        let hash = [0xAB; 32];
        let conn = fresh_conn_with_event(&hash);
        let c = bump_access_count(&conn, &hash, "2026-04-18T00:00:00Z").unwrap();
        assert_eq!(c, 1);
    }

    #[test]
    fn t11_bump_access_count_increments_existing() {
        let hash = [0xCD; 32];
        let conn = fresh_conn_with_event(&hash);
        assert_eq!(
            bump_access_count(&conn, &hash, "2026-04-18T00:00:00Z").unwrap(),
            1
        );
        assert_eq!(
            bump_access_count(&conn, &hash, "2026-04-18T00:00:01Z").unwrap(),
            2
        );
        assert_eq!(
            bump_access_count(&conn, &hash, "2026-04-18T00:00:02Z").unwrap(),
            3
        );
    }

    #[test]
    fn t11_bump_access_count_updates_last_hit_rfc3339() {
        let hash = [0xEF; 32];
        let conn = fresh_conn_with_event(&hash);
        bump_access_count(&conn, &hash, "2026-04-18T00:00:00Z").unwrap();
        bump_access_count(&conn, &hash, "2026-05-01T12:34:56Z").unwrap();
        let last: String = conn
            .query_row(
                "SELECT last_hit_rfc3339 FROM heirloom_access_count WHERE event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, "2026-05-01T12:34:56Z");
    }

    #[test]
    fn t11_read_access_count_missing_returns_zero() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let hash = [0x99; 32];
        assert_eq!(read_access_count(&conn, &hash).unwrap(), 0);
    }

    #[test]
    fn t11_read_access_count_after_bumps_returns_correct_value() {
        let hash = [0x42; 32];
        let conn = fresh_conn_with_event(&hash);
        bump_access_count(&conn, &hash, "2026-04-18T00:00:00Z").unwrap();
        bump_access_count(&conn, &hash, "2026-04-18T00:00:01Z").unwrap();
        bump_access_count(&conn, &hash, "2026-04-18T00:00:02Z").unwrap();
        assert_eq!(read_access_count(&conn, &hash).unwrap(), 3);
    }

    #[test]
    fn t11_access_summary_payload_serde_round_trip() {
        let mut deltas = HashMap::new();
        deltas.insert("aabb".to_string(), 3);
        deltas.insert("ccdd".to_string(), 7);
        let p = AccessSummaryPayload {
            date: "2026-04-17".to_string(),
            access_deltas: deltas,
            emitted_at_hlc: HlcTimestamp { phys: 42, log: 1 },
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: AccessSummaryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn t11_heirloom_access_count_table_created_by_ensure_tables() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='heirloom_access_count'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn t11_heirloom_access_count_index_descending_by_count() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_heirloom_access_count_count'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Also assert the lineage index on event_log was added.
        let lineage: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_heirloom_event_log_lineage'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lineage, 1);
    }
}

use crate::heirloom::EventEnvelope;
use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Build the daily access_summary envelope. Caller feeds this to
/// `HeirloomWriter::append_event`. Phase C3 provides the builder; scheduler
/// wiring is Phase E. Sensitivity = Ordinary (access patterns are not
/// confidential on their own).
#[allow(clippy::too_many_arguments)]
pub fn build_access_summary_envelope(
    date: &str,
    access_deltas: HashMap<String, u64>,
    emitted_at_hlc: HlcTimestamp,
    observed_at: DateTime<Utc>,
    author_did: Did,
    lineage_id: LineageId,
    constitution_l0: [u8; 32],
    constitution_l1: [u8; 32],
    constitution_l2_manifest: [u8; 32],
) -> EventEnvelope {
    let payload = AccessSummaryPayload {
        date: date.to_string(),
        access_deltas,
        emitted_at_hlc,
    };
    EventEnvelope {
        event_id: Uuid::now_v7(),
        schema_version: 1,
        event_type: EventTypeTag::new("ai:access_summary"),
        prev_hash: [0; 32],     // Writer fills via tip
        hlc_ts: emitted_at_hlc, // Writer overwrites via self.hlc.tick()
        observed_at,
        l0_hash: constitution_l0,
        l1_hash: constitution_l1,
        l2_manifest_hash: constitution_l2_manifest,
        lineage_id,
        author_did,
        signatures: vec![Signature::new(vec![])],
        sensitivity_tier: SensitivityTier::Ordinary,
        bundle_ids: vec![BundleId::new("bundle:core")],
        payload_encoding: PayloadEncoding::Plaintext,
        payload: serde_json::to_vec(&payload).expect("static shape"),
    }
}

#[cfg(test)]
mod build_envelope_tests {
    use super::*;
    use chrono::TimeZone;

    fn dummy_date() -> String {
        "2026-04-17".to_string()
    }

    fn dummy_observed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 18, 0, 0, 0).unwrap()
    }

    fn sample_deltas() -> HashMap<String, u64> {
        let mut d = HashMap::new();
        d.insert("aaaa".to_string(), 3);
        d.insert("bbbb".to_string(), 7);
        d
    }

    fn build_sample() -> EventEnvelope {
        build_access_summary_envelope(
            &dummy_date(),
            sample_deltas(),
            HlcTimestamp { phys: 42, log: 0 },
            dummy_observed(),
            Did::new("did:aegis:self"),
            LineageId::new("lineage:test"),
            [0x01; 32],
            [0x02; 32],
            [0x03; 32],
        )
    }

    #[test]
    fn t11_build_access_summary_envelope_event_type_is_ai_access_summary() {
        let e = build_sample();
        assert_eq!(e.event_type.as_str(), "ai:access_summary");
    }

    #[test]
    fn t11_build_access_summary_payload_decodes_from_envelope() {
        let e = build_sample();
        let back: AccessSummaryPayload = serde_json::from_slice(&e.payload).unwrap();
        assert_eq!(back.date, "2026-04-17");
        assert_eq!(back.access_deltas.get("aaaa"), Some(&3));
        assert_eq!(back.access_deltas.get("bbbb"), Some(&7));
        assert_eq!(back.emitted_at_hlc, HlcTimestamp { phys: 42, log: 0 });
    }

    #[test]
    fn t11_build_access_summary_envelope_sensitivity_is_ordinary() {
        let e = build_sample();
        assert_eq!(e.sensitivity_tier, SensitivityTier::Ordinary);
    }

    #[test]
    fn t11_build_access_summary_envelope_bundle_is_core() {
        let e = build_sample();
        assert_eq!(e.bundle_ids.len(), 1);
        assert_eq!(e.bundle_ids[0].as_str(), "bundle:core");
    }
}
