//! BundleIndexProjector — indexes every event by its `bundle_ids[]` entries.

use crate::heirloom::EventEnvelope;
use crate::heirloom::projector::{EventTypePattern, Projector, ProjectorError};

/// Compute `blake3(canonical_bytes(envelope))` once so we don't pay it in
/// every projector. Phase C2 W10 will compute this once per append and pass
/// it via envelope metadata; for Phase C1 we recompute per-projector.
fn event_hash_bytes(envelope: &EventEnvelope) -> [u8; 32] {
    crate::heirloom::canonical::event_hash(envelope)
        .expect("EventEnvelope must be JCS-serializable")
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BundleIndexProjector;

impl BundleIndexProjector {
    pub const fn new() -> Self {
        Self
    }
}

impl Projector for BundleIndexProjector {
    fn for_event_types(&self) -> Vec<EventTypePattern> {
        vec![EventTypePattern::Any]
    }

    fn apply(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: &EventEnvelope,
    ) -> Result<(), ProjectorError> {
        let hash = event_hash_bytes(envelope);
        let mut stmt = tx
            .prepare_cached(
                "INSERT INTO heirloom_bundle_index (bundle_id, event_hash, hlc_phys)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;

        for bundle in &envelope.bundle_ids {
            stmt.execute(rusqlite::params![
                bundle.as_str(),
                hash.as_slice(),
                envelope.hlc_ts.phys as i64
            ])
            .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;
        }
        Ok(())
    }

    fn projector_name(&self) -> &'static str {
        "BundleIndexProjector"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::schema::ensure_tables;
    use crate::heirloom::types::{BundleId, Did, LineageId};
    use chrono::{DateTime, Utc};
    use rusqlite::Connection;
    use uuid::Uuid;

    fn envelope_with_bundles(bundles: Vec<&str>, hlc_phys: u64) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::parse_str("01925b3e-0000-7000-8000-000000000001").unwrap(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:test"),
            prev_hash: [0; 32],
            hlc_ts: HlcTimestamp {
                phys: hlc_phys,
                log: 0,
            },
            observed_at: DateTime::parse_from_rfc3339("2026-04-17T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
            lineage_id: LineageId::new("lineage:test"),
            author_did: Did::new("did:aegis:test"),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: bundles.into_iter().map(BundleId::new).collect(),
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{}".to_vec(),
        }
    }

    fn fresh_db() -> Connection {
        // Each test owns a fresh in-memory DB. Phase C1 does NOT enable
        // `PRAGMA foreign_keys = ON` here because the BundleIndex tests
        // insert a matching event_log stub via `insert_event_log_stub` —
        // that satisfies the FK even when it fires. Task 1's FK-activation
        // test is the separate proof that FKs work when enabled.
        let c = Connection::open_in_memory().unwrap();
        ensure_tables(&c).unwrap();
        c
    }

    fn insert_event_log_stub(conn: &Connection, envelope: &EventEnvelope, hash: [u8; 32]) {
        conn.execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version, event_type,
                hlc_phys, hlc_log, observed_at_rfc3339, l0_hash, l1_hash,
                l2_manifest_hash, lineage_id, author_did, sensitivity_tier,
                bundle_ids_json, payload_encoding_json, payload, signatures_json,
                envelope_canonical, inserted_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                hash.as_slice(),
                envelope.prev_hash.as_slice(),
                envelope.event_id.to_string(),
                envelope.schema_version,
                envelope.event_type.as_str(),
                envelope.hlc_ts.phys as i64,
                envelope.hlc_ts.log as i64,
                envelope.observed_at.to_rfc3339(),
                envelope.l0_hash.as_slice(),
                envelope.l1_hash.as_slice(),
                envelope.l2_manifest_hash.as_slice(),
                envelope.lineage_id.as_str(),
                envelope.author_did.as_str(),
                crate::heirloom::sensitivity::sensitivity_tier_wire_str(&envelope.sensitivity_tier)
                    .to_string(),
                serde_json::to_string(&envelope.bundle_ids).unwrap(),
                serde_json::to_string(&envelope.payload_encoding).unwrap(),
                envelope.payload.as_slice(),
                serde_json::to_string(&envelope.signatures).unwrap(),
                crate::heirloom::canonical::canonical_bytes(envelope)
                    .unwrap()
                    .as_slice(),
                0_i64,
            ],
        )
        .unwrap();
    }

    #[test]
    fn t9_bundle_index_emits_one_row_per_bundle() {
        let conn = fresh_db();
        let envelope = envelope_with_bundles(vec!["bundle:a", "bundle:b"], 100);
        let hash = event_hash_bytes(&envelope);
        insert_event_log_stub(&conn, &envelope, hash);

        let tx = conn.unchecked_transaction().unwrap();
        BundleIndexProjector::new().apply(&tx, &envelope).unwrap();
        tx.commit().unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM heirloom_bundle_index WHERE event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn t9_bundle_index_records_hlc_for_later_scan() {
        let conn = fresh_db();
        let envelope = envelope_with_bundles(vec!["bundle:general"], 42_000);
        let hash = event_hash_bytes(&envelope);
        insert_event_log_stub(&conn, &envelope, hash);

        let tx = conn.unchecked_transaction().unwrap();
        BundleIndexProjector::new().apply(&tx, &envelope).unwrap();
        tx.commit().unwrap();

        let hlc_phys: i64 = conn
            .query_row(
                "SELECT hlc_phys FROM heirloom_bundle_index
                 WHERE bundle_id = 'bundle:general' AND event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hlc_phys, 42_000);
    }

    #[test]
    fn t9_bundle_index_empty_bundle_list_is_no_op() {
        let conn = fresh_db();
        let envelope = envelope_with_bundles(vec![], 7);
        let hash = event_hash_bytes(&envelope);
        insert_event_log_stub(&conn, &envelope, hash);

        let tx = conn.unchecked_transaction().unwrap();
        BundleIndexProjector::new().apply(&tx, &envelope).unwrap();
        tx.commit().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "no bundles → no rows");
    }

    #[test]
    fn t9_bundle_index_rejects_duplicate_insert_via_pk() {
        let conn = fresh_db();
        let envelope = envelope_with_bundles(vec!["bundle:a"], 1);
        let hash = event_hash_bytes(&envelope);
        insert_event_log_stub(&conn, &envelope, hash);

        let tx = conn.unchecked_transaction().unwrap();
        BundleIndexProjector::new().apply(&tx, &envelope).unwrap();
        // Second apply on same envelope → PK violation because
        // (bundle_id, event_hash) is UNIQUE.
        let err = BundleIndexProjector::new().apply(&tx, &envelope);
        assert!(err.is_err());
    }

    #[test]
    fn t9_bundle_index_projector_name_is_stable() {
        assert_eq!(
            BundleIndexProjector::new().projector_name(),
            "BundleIndexProjector"
        );
    }
}
