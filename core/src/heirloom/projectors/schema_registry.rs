//! SchemaRegistryProjector — projects `schema:*` events into the runtime
//! schema registry table.
//!
//! Three event types handled:
//! - `schema:type_defined`                 (kind="type_defined")
//! - `schema:signature_scheme_registered`  (kind="signature_scheme_registered")
//! - `schema:signature_scheme_deprecated`  (kind="signature_scheme_deprecated")
//!
//! Phase B Task 7.5 provides `SchemeRegisteredPayload`; this projector
//! decodes those bytes from `envelope.payload` and upserts a row.

use crate::heirloom::EventEnvelope;
use crate::heirloom::projector::{EventTypePattern, Projector, ProjectorError};
use crate::heirloom::seams::threshold_signer::SchemeRegisteredPayload;
use serde::{Deserialize, Serialize};

pub const KIND_TYPE_DEFINED: &str = "type_defined";
pub const KIND_SCHEME_REGISTERED: &str = "signature_scheme_registered";
pub const KIND_SCHEME_DEPRECATED: &str = "signature_scheme_deprecated";

/// Public view of a schema registry row. Read-only helper for Phase C2+
/// readers; does not roundtrip DB-exactly (we store the json blob verbatim).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchemaRegistryEntry {
    pub kind: String,
    pub id: String,
    pub payload_json: String,
    pub valid_from_phys: u64,
    pub valid_from_log: u32,
    pub valid_until_phys: Option<u64>,
    pub valid_until_log: Option<u32>,
    pub registered_event_hash: [u8; 32],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SchemaRegistryProjector;

impl SchemaRegistryProjector {
    pub const fn new() -> Self {
        Self
    }

    fn decode_scheme_registered(payload: &[u8]) -> Result<SchemeRegisteredPayload, ProjectorError> {
        serde_json::from_slice(payload).map_err(|e| ProjectorError::PayloadDecode {
            projector: "SchemaRegistryProjector".to_string(),
            reason: format!("SchemeRegisteredPayload: {e}"),
        })
    }
}

impl Projector for SchemaRegistryProjector {
    fn for_event_types(&self) -> Vec<EventTypePattern> {
        vec![EventTypePattern::domain("schema")]
    }

    fn apply(
        &self,
        tx: &rusqlite::Transaction<'_>,
        envelope: &EventEnvelope,
    ) -> Result<(), ProjectorError> {
        let event_hash = crate::heirloom::canonical::event_hash(envelope)
            .expect("EventEnvelope must be JCS-serializable");

        let (_domain, action) = envelope
            .event_type
            .as_str()
            .split_once(':')
            .ok_or_else(|| ProjectorError::Invariant {
                projector: self.projector_name().to_string(),
                reason: format!(
                    "event_type lacks domain separator: {}",
                    envelope.event_type.as_str()
                ),
            })?;

        match action {
            "type_defined" => {
                // Phase D: prefer payload.type_tag as id; fall back to event_id UUID
                // if payload is not the Phase-D shape (forward compat).
                let id = match serde_json::from_slice::<
                    crate::heirloom::schema_bootstrap::SchemaTypeDefPayload,
                >(&envelope.payload)
                {
                    Ok(p) => p.type_tag,
                    Err(_) => envelope.event_id.to_string(),
                };
                let payload_json = String::from_utf8_lossy(&envelope.payload).to_string();
                self.upsert(
                    tx,
                    KIND_TYPE_DEFINED,
                    &id,
                    &payload_json,
                    envelope,
                    event_hash,
                )?;
            }
            "signature_scheme_registered" => {
                let payload = Self::decode_scheme_registered(&envelope.payload)?;
                let payload_json =
                    serde_json::to_string(&payload).map_err(|e| ProjectorError::PayloadDecode {
                        projector: self.projector_name().to_string(),
                        reason: format!("re-encode: {e}"),
                    })?;
                self.upsert(
                    tx,
                    KIND_SCHEME_REGISTERED,
                    &payload.scheme_id,
                    &payload_json,
                    envelope,
                    event_hash,
                )?;
            }
            "type_deprecated" => {
                // Task 12 (Round 2): flip valid_until_phys on the matching
                // type_defined row. Reuses the SAME column semantics used by
                // signature_scheme_deprecated (no ALTER TABLE needed). If the
                // type_tag doesn't match any row, behave as a silent no-op
                // (the Writer's deprecate_type pre-check already rejects
                // unknown types; this branch is defensive).
                let payload: crate::heirloom::schema_bootstrap::SchemaTypeDeprecatedPayload =
                    serde_json::from_slice(&envelope.payload).map_err(|e| {
                        ProjectorError::PayloadDecode {
                            projector: self.projector_name().to_string(),
                            reason: format!("schema:type_deprecated payload: {e}"),
                        }
                    })?;
                tx.execute(
                    "UPDATE heirloom_schema_registry
                     SET valid_until_phys = ?1, valid_until_log = ?2
                     WHERE kind = 'type_defined' AND id = ?3 AND valid_until_phys IS NULL",
                    rusqlite::params![
                        envelope.hlc_ts.phys as i64,
                        envelope.hlc_ts.log as i64,
                        payload.type_tag,
                    ],
                )
                .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;
            }
            "signature_scheme_deprecated" => {
                // Payload shape matches SchemeRegisteredPayload (deprecation
                // event carries the deprecated scheme's metadata + a
                // valid_until_hlc that we capture).
                let payload = Self::decode_scheme_registered(&envelope.payload)?;
                let payload_json =
                    serde_json::to_string(&payload).map_err(|e| ProjectorError::PayloadDecode {
                        projector: self.projector_name().to_string(),
                        reason: format!("re-encode: {e}"),
                    })?;

                // Flip the valid_until on the existing registered row (if any),
                // then insert the deprecation row keyed under KIND_SCHEME_DEPRECATED.
                tx.execute(
                    "UPDATE heirloom_schema_registry
                     SET valid_until_phys = ?1, valid_until_log = ?2
                     WHERE kind = ?3 AND id = ?4 AND valid_until_phys IS NULL",
                    rusqlite::params![
                        envelope.hlc_ts.phys as i64,
                        envelope.hlc_ts.log as i64,
                        KIND_SCHEME_REGISTERED,
                        &payload.scheme_id,
                    ],
                )
                .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;

                self.upsert(
                    tx,
                    KIND_SCHEME_DEPRECATED,
                    &payload.scheme_id,
                    &payload_json,
                    envelope,
                    event_hash,
                )?;
            }
            other => {
                // Unknown schema sub-action — store as type_defined for forward
                // compatibility rather than erroring (schema ledger spec grows).
                let id = format!("{}-{}", other, envelope.event_id);
                let payload_json = String::from_utf8_lossy(&envelope.payload).to_string();
                self.upsert(
                    tx,
                    KIND_TYPE_DEFINED,
                    &id,
                    &payload_json,
                    envelope,
                    event_hash,
                )?;
            }
        }
        Ok(())
    }

    fn projector_name(&self) -> &'static str {
        "SchemaRegistryProjector"
    }
}

impl SchemaRegistryProjector {
    fn upsert(
        &self,
        tx: &rusqlite::Transaction<'_>,
        kind: &str,
        id: &str,
        payload_json: &str,
        envelope: &EventEnvelope,
        event_hash: [u8; 32],
    ) -> Result<(), ProjectorError> {
        tx.execute(
            "INSERT OR REPLACE INTO heirloom_schema_registry (
                kind, id, payload_json, valid_from_phys, valid_from_log,
                valid_until_phys, valid_until_log, registered_event_hash
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6)",
            rusqlite::params![
                kind,
                id,
                payload_json,
                envelope.hlc_ts.phys as i64,
                envelope.hlc_ts.log as i64,
                event_hash.as_slice(),
            ],
        )
        .map_err(|e| ProjectorError::Sqlite(self.projector_name().to_string(), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::schema::ensure_tables;
    use crate::heirloom::seams::threshold_signer::{GroupPubKey, ShareId, ThresholdSpec};
    use crate::heirloom::types::{BundleId, Did, LineageId};
    use chrono::{DateTime, Utc};
    use rusqlite::Connection;
    use uuid::Uuid;

    fn envelope(event_type: &str, payload: Vec<u8>, hlc_phys: u64) -> EventEnvelope {
        let _ = ShareId::new("ignored"); // ensure import stays live
        EventEnvelope {
            event_id: Uuid::parse_str("01925b3e-0000-7000-8000-000000000001").unwrap(),
            schema_version: 1,
            event_type: EventTypeTag::new(event_type),
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
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload,
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

    fn dev_scheme_payload() -> SchemeRegisteredPayload {
        SchemeRegisteredPayload {
            scheme_id: "frost-ed25519-1of1-dev".to_string(),
            algorithm: "Ed25519".to_string(),
            threshold: ThresholdSpec::one_of_one(),
            group_pubkey: GroupPubKey::from_bytes([0xAA; 32]),
            verifier_spec_ref: "RFC 8032".to_string(),
            valid_from_hlc: HlcTimestamp { phys: 1, log: 0 },
            valid_until_hlc: None,
        }
    }

    #[test]
    fn t9_schema_projector_pattern_is_schema_wildcard() {
        let p = SchemaRegistryProjector::new();
        let patterns = p.for_event_types();
        assert_eq!(patterns.len(), 1);
        assert!(patterns[0].matches("schema:type_defined"));
        assert!(patterns[0].matches("schema:signature_scheme_registered"));
        assert!(!patterns[0].matches("ai:tool_invoked"));
    }

    #[test]
    fn t9_schema_projector_type_defined_stores_payload_verbatim() {
        let env = envelope("schema:type_defined", br#"{"my":"shape"}"#.to_vec(), 1);
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &env).unwrap();
        tx.commit().unwrap();

        let kind: String = conn
            .query_row(
                "SELECT kind FROM heirloom_schema_registry LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "type_defined");
    }

    #[test]
    fn t9_schema_projector_scheme_registered_keyed_by_scheme_id() {
        let payload = dev_scheme_payload();
        let bytes = serde_json::to_vec(&payload).unwrap();
        let env = envelope("schema:signature_scheme_registered", bytes, 100);
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &env).unwrap();
        tx.commit().unwrap();

        let (kind, id): (String, String) = conn
            .query_row(
                "SELECT kind, id FROM heirloom_schema_registry WHERE kind = ?1",
                rusqlite::params![KIND_SCHEME_REGISTERED],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "signature_scheme_registered");
        assert_eq!(id, "frost-ed25519-1of1-dev");
    }

    #[test]
    fn t9_schema_projector_scheme_registered_payload_decode_fail_returns_error() {
        let env = envelope(
            "schema:signature_scheme_registered",
            b"not json at all".to_vec(),
            1,
        );
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        let err = SchemaRegistryProjector::new().apply(&tx, &env);
        assert!(matches!(err, Err(ProjectorError::PayloadDecode { .. })));
    }

    #[test]
    fn t9_schema_projector_deprecation_flips_valid_until_on_register_row() {
        let register_payload = dev_scheme_payload();
        let register_env = envelope(
            "schema:signature_scheme_registered",
            serde_json::to_vec(&register_payload).unwrap(),
            100,
        );
        let (conn, _h1) = fresh_db_with_event(&register_env);

        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new()
            .apply(&tx, &register_env)
            .unwrap();
        tx.commit().unwrap();

        // Now deprecate it at hlc_phys=200.
        let mut dep_env = envelope(
            "schema:signature_scheme_deprecated",
            serde_json::to_vec(&register_payload).unwrap(),
            200,
        );
        dep_env.event_id = Uuid::parse_str("01925b3e-0000-7000-8000-000000000002").unwrap();
        // Insert its event_log stub (different hash).
        let h2 = crate::heirloom::canonical::event_hash(&dep_env).unwrap();
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
                dep_env.prev_hash.as_slice(),
                dep_env.event_id.to_string(),
                dep_env.schema_version,
                dep_env.event_type.as_str(),
                dep_env.hlc_ts.phys as i64,
                dep_env.hlc_ts.log as i64,
                dep_env.observed_at.to_rfc3339(),
                dep_env.l0_hash.as_slice(),
                dep_env.l1_hash.as_slice(),
                dep_env.l2_manifest_hash.as_slice(),
                dep_env.lineage_id.as_str(),
                dep_env.author_did.as_str(),
                crate::heirloom::sensitivity::sensitivity_tier_wire_str(&dep_env.sensitivity_tier)
                    .to_string(),
                serde_json::to_string(&dep_env.bundle_ids).unwrap(),
                serde_json::to_string(&dep_env.payload_encoding).unwrap(),
                dep_env.payload.as_slice(),
                serde_json::to_string(&dep_env.signatures).unwrap(),
                crate::heirloom::canonical::canonical_bytes(&dep_env)
                    .unwrap()
                    .as_slice(),
                0_i64,
            ],
        )
        .unwrap();

        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &dep_env).unwrap();
        tx.commit().unwrap();

        // The REGISTER row's valid_until should now be set to 200.
        let valid_until: Option<i64> = conn
            .query_row(
                "SELECT valid_until_phys FROM heirloom_schema_registry
                 WHERE kind = 'signature_scheme_registered' AND id = 'frost-ed25519-1of1-dev'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(valid_until, Some(200));

        // The DEPRECATE row must also exist.
        let dep_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM heirloom_schema_registry
                 WHERE kind = 'signature_scheme_deprecated' AND id = 'frost-ed25519-1of1-dev'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dep_count, 1);
    }

    #[test]
    fn t9_schema_projector_unknown_subaction_stored_as_type_defined() {
        // Forward-compat: schema:some_future_action lands in type_defined bucket.
        let env = envelope("schema:some_future_action", b"payload-bytes".to_vec(), 1);
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &env).unwrap();
        tx.commit().unwrap();

        let kind: String = conn
            .query_row(
                "SELECT kind FROM heirloom_schema_registry LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "type_defined");
    }

    #[test]
    fn t9_schema_projector_name_is_stable() {
        assert_eq!(
            SchemaRegistryProjector::new().projector_name(),
            "SchemaRegistryProjector"
        );
    }

    #[test]
    fn t_d2_type_defined_keyed_by_payload_type_tag() {
        // This test drives the projector directly; it does NOT depend on the
        // type_tag being in SCHEMA_TYPE_DEFS. `learned:interest` is a real
        // spec-canonical action (Q2.3 line 163) used here as a representative.
        use crate::heirloom::enums::SensitivityTier;
        use crate::heirloom::schema_bootstrap::SchemaTypeDefPayload;
        let payload = SchemaTypeDefPayload {
            type_tag: "learned:interest".to_string(),
            domain: "learned".to_string(),
            action: "interest".to_string(),
            sensitivity_default: SensitivityTier::Sensitive,
            doc_ref: "spec§2-Q2.3".to_string(),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let env = envelope("schema:type_defined", bytes, 50);
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &env).unwrap();
        tx.commit().unwrap();

        let id: String = conn
            .query_row(
                "SELECT id FROM heirloom_schema_registry
                 WHERE kind = 'type_defined' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            id, "learned:interest",
            "type_defined row must be keyed by payload.type_tag, not event_id"
        );
    }

    #[test]
    fn t_d2_type_defined_malformed_payload_falls_back_to_event_id() {
        // Forward-compat: a non-Phase-D producer might emit type_defined with
        // an older shape. Don't hard-fail; fall back to event_id like before.
        let env = envelope("schema:type_defined", b"not-json".to_vec(), 99);
        let (conn, _hash) = fresh_db_with_event(&env);
        let tx = conn.unchecked_transaction().unwrap();
        SchemaRegistryProjector::new().apply(&tx, &env).unwrap();
        tx.commit().unwrap();
        // We expect one row, id == event_id uuid string.
        let id: String = conn
            .query_row(
                "SELECT id FROM heirloom_schema_registry WHERE kind = 'type_defined'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id, env.event_id.to_string());
    }
}
