//! EntityIndexProjector — indexes every event by its `author_did`.
//!
//! Phase C1 only emits `role = "author"` rows. Phase C2+ projectors for
//! specific event types (ai:tool_invoked, learned:*, etc.) will emit
//! additional `role = "subject"` / `role = "reference"` rows from payload
//! fields.

use crate::heirloom::EventEnvelope;
use crate::heirloom::projector::{EventTypePattern, Projector, ProjectorError};

pub const ROLE_AUTHOR: &str = "author";
pub const ROLE_SUBJECT: &str = "subject";
pub const ROLE_REFERENCE: &str = "reference";

#[derive(Debug, Default, Clone, Copy)]
pub struct EntityIndexProjector;

impl EntityIndexProjector {
    pub const fn new() -> Self {
        Self
    }
}

impl Projector for EntityIndexProjector {
    fn for_event_types(&self) -> Vec<EventTypePattern> {
        vec![EventTypePattern::Any]
    }

    fn apply(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: &EventEnvelope,
    ) -> Result<(), ProjectorError> {
        let hash = crate::heirloom::canonical::event_hash(envelope)
            .expect("EventEnvelope must be JCS-serializable");

        tx.prepare_cached(
            "INSERT INTO heirloom_entity_index (entity_did, event_hash, role, hlc_phys)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?
        .execute(rusqlite::params![
            envelope.author_did.as_str(),
            hash.as_slice(),
            ROLE_AUTHOR,
            envelope.hlc_ts.phys as i64,
        ])
        .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;

        Ok(())
    }

    fn projector_name(&self) -> &'static str {
        "EntityIndexProjector"
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

    fn envelope_with_author(author: &str, hlc_phys: u64) -> EventEnvelope {
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
            author_did: Did::new(author),
            signatures: vec![],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: b"{}".to_vec(),
        }
    }

    fn fresh_db_with_event(envelope: &EventEnvelope) -> (Connection, [u8; 32]) {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        let hash = crate::heirloom::canonical::event_hash(envelope).unwrap();
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
        (conn, hash)
    }

    #[test]
    fn t9_entity_index_emits_author_row() {
        let envelope = envelope_with_author("did:aegis:alice", 100);
        let (conn, hash) = fresh_db_with_event(&envelope);
        let tx = conn.unchecked_transaction().unwrap();
        EntityIndexProjector::new().apply(&tx, &envelope).unwrap();
        tx.commit().unwrap();

        let rows: Vec<(String, String)> = conn
            .prepare("SELECT entity_did, role FROM heirloom_entity_index WHERE event_hash = ?1")
            .unwrap()
            .query_map(rusqlite::params![hash.as_slice()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![("did:aegis:alice".to_string(), "author".to_string())]
        );
    }

    #[test]
    fn t9_entity_index_records_hlc() {
        let envelope = envelope_with_author("did:aegis:alice", 777);
        let (conn, hash) = fresh_db_with_event(&envelope);
        let tx = conn.unchecked_transaction().unwrap();
        EntityIndexProjector::new().apply(&tx, &envelope).unwrap();
        tx.commit().unwrap();
        let phys: i64 = conn
            .query_row(
                "SELECT hlc_phys FROM heirloom_entity_index WHERE event_hash = ?1",
                rusqlite::params![hash.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(phys, 777);
    }

    #[test]
    fn t9_entity_index_role_constants_match_db_strings() {
        assert_eq!(ROLE_AUTHOR, "author");
        assert_eq!(ROLE_SUBJECT, "subject");
        assert_eq!(ROLE_REFERENCE, "reference");
    }

    #[test]
    fn t9_entity_index_two_events_same_author_produce_two_rows() {
        let e1 = envelope_with_author("did:aegis:alice", 1);
        let (conn, h1) = fresh_db_with_event(&e1);
        let mut e2 = envelope_with_author("did:aegis:alice", 2);
        e2.event_id = Uuid::parse_str("01925b3e-0000-7000-8000-000000000002").unwrap();

        // Insert event_log stub for e2 too.
        let h2 = crate::heirloom::canonical::event_hash(&e2).unwrap();
        conn.execute(
            "INSERT INTO heirloom_event_log (
                event_hash, prev_hash, event_id_uuid, schema_version, event_type,
                hlc_phys, hlc_log, observed_at_rfc3339, l0_hash, l1_hash,
                l2_manifest_hash, lineage_id, author_did, sensitivity_tier,
                bundle_ids_json, payload_encoding_json, payload, signatures_json,
                envelope_canonical, inserted_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                h2.as_slice(),
                e2.prev_hash.as_slice(),
                e2.event_id.to_string(),
                e2.schema_version,
                e2.event_type.as_str(),
                e2.hlc_ts.phys as i64,
                e2.hlc_ts.log as i64,
                e2.observed_at.to_rfc3339(),
                e2.l0_hash.as_slice(),
                e2.l1_hash.as_slice(),
                e2.l2_manifest_hash.as_slice(),
                e2.lineage_id.as_str(),
                e2.author_did.as_str(),
                crate::heirloom::sensitivity::sensitivity_tier_wire_str(&e2.sensitivity_tier)
                    .to_string(),
                serde_json::to_string(&e2.bundle_ids).unwrap(),
                serde_json::to_string(&e2.payload_encoding).unwrap(),
                e2.payload.as_slice(),
                serde_json::to_string(&e2.signatures).unwrap(),
                crate::heirloom::canonical::canonical_bytes(&e2)
                    .unwrap()
                    .as_slice(),
                0_i64,
            ],
        )
        .unwrap();

        let tx = conn.unchecked_transaction().unwrap();
        EntityIndexProjector::new().apply(&tx, &e1).unwrap();
        EntityIndexProjector::new().apply(&tx, &e2).unwrap();
        tx.commit().unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM heirloom_entity_index WHERE entity_did = 'did:aegis:alice' AND role = 'author'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2);
        // h1 and h2 must be distinct.
        assert_ne!(h1, h2);
    }

    #[test]
    fn t9_entity_index_duplicate_apply_violates_pk() {
        let envelope = envelope_with_author("did:aegis:alice", 1);
        let (conn, _hash) = fresh_db_with_event(&envelope);
        let tx = conn.unchecked_transaction().unwrap();
        EntityIndexProjector::new().apply(&tx, &envelope).unwrap();
        let err = EntityIndexProjector::new().apply(&tx, &envelope);
        assert!(
            err.is_err(),
            "PK (entity_did, event_hash, role) must reject"
        );
    }

    #[test]
    fn t9_entity_index_projector_name_is_stable() {
        assert_eq!(
            EntityIndexProjector::new().projector_name(),
            "EntityIndexProjector"
        );
    }
}
