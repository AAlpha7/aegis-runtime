//! Integration tests for Phase C1 projector foundation.
//!
//! Exercises `ProjectorDispatcher` + all 3 projectors against in-memory
//! SQLite. Every test builds its own fresh DB via `ensure_tables()`.

use std::sync::Arc;

use crate::heirloom::EventEnvelope;
use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
use crate::heirloom::hlc::HlcTimestamp;
use crate::heirloom::projector::{Projector, ProjectorDispatcher, ProjectorError};
use crate::heirloom::projectors::{
    BundleIndexProjector, EntityIndexProjector, SchemaRegistryProjector,
};
use crate::heirloom::schema::ensure_tables;
use crate::heirloom::seams::threshold_signer::{
    GroupPubKey, SchemeRegisteredPayload, ThresholdSpec,
};
use crate::heirloom::types::{BundleId, Did, LineageId};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use uuid::Uuid;

fn envelope(
    event_type: &str,
    author: &str,
    bundles: Vec<&str>,
    payload: Vec<u8>,
    hlc_phys: u64,
    id_suffix: u8,
) -> EventEnvelope {
    let mut id_bytes = [0u8; 16];
    id_bytes[15] = id_suffix;
    let uuid = uuid::Uuid::from_bytes(id_bytes);
    EventEnvelope {
        event_id: uuid,
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
        author_did: Did::new(author),
        signatures: vec![],
        sensitivity_tier: SensitivityTier::Ordinary,
        bundle_ids: bundles.into_iter().map(BundleId::new).collect(),
        payload_encoding: PayloadEncoding::Plaintext,
        payload,
    }
}

fn insert_event_log(conn: &Connection, env: &EventEnvelope) -> [u8; 32] {
    let hash = crate::heirloom::canonical::event_hash(env).unwrap();
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
            env.prev_hash.as_slice(),
            env.event_id.to_string(),
            env.schema_version,
            env.event_type.as_str(),
            env.hlc_ts.phys as i64,
            env.hlc_ts.log as i64,
            env.observed_at.to_rfc3339(),
            env.l0_hash.as_slice(),
            env.l1_hash.as_slice(),
            env.l2_manifest_hash.as_slice(),
            env.lineage_id.as_str(),
            env.author_did.as_str(),
            crate::heirloom::sensitivity::sensitivity_tier_wire_str(&env.sensitivity_tier)
                .to_string(),
            serde_json::to_string(&env.bundle_ids).unwrap(),
            serde_json::to_string(&env.payload_encoding).unwrap(),
            env.payload.as_slice(),
            serde_json::to_string(&env.signatures).unwrap(),
            crate::heirloom::canonical::canonical_bytes(env)
                .unwrap()
                .as_slice(),
            0_i64,
        ],
    )
    .unwrap();
    hash
}

fn build_dispatcher_with_all_three() -> ProjectorDispatcher {
    let mut d = ProjectorDispatcher::new();
    d.register(Arc::new(BundleIndexProjector::new()));
    d.register(Arc::new(EntityIndexProjector::new()));
    d.register(Arc::new(SchemaRegistryProjector::new()));
    d
}

#[test]
fn i9_dispatcher_routes_schema_event_to_three_projectors() {
    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();

    let payload = SchemeRegisteredPayload {
        scheme_id: "test-scheme".to_string(),
        algorithm: "Ed25519".to_string(),
        threshold: ThresholdSpec::one_of_one(),
        group_pubkey: GroupPubKey::from_bytes([0x11; 32]),
        verifier_spec_ref: "RFC 8032".to_string(),
        valid_from_hlc: HlcTimestamp { phys: 1, log: 0 },
        valid_until_hlc: None,
    };
    let env = envelope(
        "schema:signature_scheme_registered",
        "did:aegis:bootstrap",
        vec!["bundle:core"],
        serde_json::to_vec(&payload).unwrap(),
        1,
        1,
    );
    let _hash = insert_event_log(&conn, &env);

    let dispatcher = build_dispatcher_with_all_three();
    let tx = conn.unchecked_transaction().unwrap();
    let hit = dispatcher.dispatch(&tx, &env).unwrap();
    tx.commit().unwrap();
    assert_eq!(hit, 3, "schema event fires all three projectors");

    let bundle_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    let entity_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_entity_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    let schema_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_schema_registry", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(bundle_rows, 1);
    assert_eq!(entity_rows, 1);
    assert_eq!(schema_rows, 1);
}

#[test]
fn i9_dispatcher_routes_non_schema_event_to_two_projectors_only() {
    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();

    let env = envelope(
        "observation:user_stated",
        "did:aegis:alice",
        vec!["bundle:general", "bundle:personal"],
        b"{}".to_vec(),
        5,
        2,
    );
    insert_event_log(&conn, &env);

    let dispatcher = build_dispatcher_with_all_three();
    let tx = conn.unchecked_transaction().unwrap();
    let hit = dispatcher.dispatch(&tx, &env).unwrap();
    tx.commit().unwrap();
    assert_eq!(hit, 2, "non-schema fires bundle + entity, not schema");

    let bundle_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    let schema_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_schema_registry", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(bundle_rows, 2, "one row per bundle");
    assert_eq!(schema_rows, 0, "schema projector untouched");
}

#[test]
fn i9_projector_error_rolls_back_all_projections_within_transaction() {
    // Custom "always fails" projector appended after the real three.
    struct FailingProjector;
    impl Projector for FailingProjector {
        fn for_event_types(&self) -> Vec<crate::heirloom::projector::EventTypePattern> {
            vec![crate::heirloom::projector::EventTypePattern::Any]
        }
        fn apply(
            &self,
            _tx: &rusqlite::Transaction<'_>,
            _envelope: &EventEnvelope,
        ) -> Result<(), ProjectorError> {
            Err(ProjectorError::Invariant {
                projector: "FailingProjector".to_string(),
                reason: "test abort".to_string(),
            })
        }
        fn projector_name(&self) -> &'static str {
            "FailingProjector"
        }
    }

    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    let env = envelope(
        "observation:user_stated",
        "did:aegis:alice",
        vec!["bundle:general"],
        b"{}".to_vec(),
        1,
        3,
    );
    insert_event_log(&conn, &env);

    let mut dispatcher = build_dispatcher_with_all_three();
    dispatcher.register(Arc::new(FailingProjector));

    let tx = conn.unchecked_transaction().unwrap();
    let err = dispatcher.dispatch(&tx, &env);
    assert!(err.is_err());
    // Simulate what the W10 HeirloomWriter will do: rollback on any err.
    drop(tx); // unchecked_transaction's Drop rolls back if not committed.

    // Because the transaction rolled back, neither bundle_index nor entity_index
    // rows survive.
    let bundle_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    let entity_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_entity_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(bundle_rows, 0, "rollback erases bundle rows");
    assert_eq!(entity_rows, 0, "rollback erases entity rows");
}

#[test]
fn i9_register_then_deprecate_produces_two_registry_rows_and_flips_valid_until() {
    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();

    let payload = SchemeRegisteredPayload {
        scheme_id: "scheme-x".to_string(),
        algorithm: "Ed25519".to_string(),
        threshold: ThresholdSpec::one_of_one(),
        group_pubkey: GroupPubKey::from_bytes([0x22; 32]),
        verifier_spec_ref: "RFC 8032".to_string(),
        valid_from_hlc: HlcTimestamp { phys: 100, log: 0 },
        valid_until_hlc: None,
    };
    let register_env = envelope(
        "schema:signature_scheme_registered",
        "did:aegis:bootstrap",
        vec!["bundle:core"],
        serde_json::to_vec(&payload).unwrap(),
        100,
        10,
    );
    insert_event_log(&conn, &register_env);

    let dispatcher = build_dispatcher_with_all_three();
    let tx = conn.unchecked_transaction().unwrap();
    dispatcher.dispatch(&tx, &register_env).unwrap();
    tx.commit().unwrap();

    // Now deprecate.
    let mut deprecate_env = envelope(
        "schema:signature_scheme_deprecated",
        "did:aegis:bootstrap",
        vec!["bundle:core"],
        serde_json::to_vec(&payload).unwrap(),
        500,
        11,
    );
    deprecate_env.event_id = Uuid::parse_str("01925b3e-0000-7000-8000-0000000000bb").unwrap();
    insert_event_log(&conn, &deprecate_env);

    let tx = conn.unchecked_transaction().unwrap();
    dispatcher.dispatch(&tx, &deprecate_env).unwrap();
    tx.commit().unwrap();

    // Two registry rows total (registered row + deprecated row).
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM heirloom_schema_registry WHERE id = 'scheme-x'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total, 2);

    // The 'registered' row now has valid_until = 500.
    let valid_until: i64 = conn
        .query_row(
            "SELECT valid_until_phys FROM heirloom_schema_registry
             WHERE kind = 'signature_scheme_registered' AND id = 'scheme-x'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(valid_until, 500);
}

#[test]
fn i9_full_append_flow_across_many_events_is_consistent() {
    let conn = Connection::open_in_memory().unwrap();
    ensure_tables(&conn).unwrap();
    let dispatcher = build_dispatcher_with_all_three();

    // Append 10 varied events.
    for i in 0..10u8 {
        let event_type = if i % 3 == 0 {
            "observation:user_stated"
        } else if i % 3 == 1 {
            "ai:tool_invoked"
        } else {
            "schema:type_defined"
        };
        let env = envelope(
            event_type,
            if i % 2 == 0 {
                "did:aegis:alice"
            } else {
                "did:aegis:bob"
            },
            if i % 2 == 0 {
                vec!["bundle:general"]
            } else {
                vec!["bundle:work", "bundle:personal"]
            },
            br#"{"payload":"seed"}"#.to_vec(),
            100 + i as u64,
            100 + i,
        );
        insert_event_log(&conn, &env);
        let tx = conn.unchecked_transaction().unwrap();
        dispatcher.dispatch(&tx, &env).unwrap();
        tx.commit().unwrap();
    }

    // Entity index has one row per event (all role=author).
    let entity_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_entity_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(entity_rows, 10);

    // Schema registry has ceil(10/3) = 4 rows (i=0,3,6,9 → schema:type_defined).
    // Wait — i%3 == 2 is schema. That's i=2,5,8 → 3 schema events.
    // Actually: i%3 == 0 → observation (i=0,3,6,9: 4 events)
    //          i%3 == 1 → ai:tool_invoked (i=1,4,7: 3 events)
    //          i%3 == 2 → schema:type_defined (i=2,5,8: 3 events)
    let schema_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_schema_registry", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(schema_rows, 3);

    // Bundle index: even i (5 events) get "bundle:general" (1 row each),
    // odd i (5 events) get 2 bundles each = 10 rows. Total: 5 + 10 = 15 rows.
    let bundle_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM heirloom_bundle_index", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(bundle_rows, 15);
}
