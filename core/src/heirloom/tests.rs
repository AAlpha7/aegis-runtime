//! Module-level integration + property tests.
//!
//! These tests cross file boundaries and exercise the full Phase A
//! substrate: HLC → envelope construction → append → canonicalize →
//! write → read → verify.

#[cfg(test)]
mod integration {
    use crate::heirloom::canonical::event_hash;
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::envelope::EventEnvelope;
    use crate::heirloom::genesis::GENESIS_PREV_HASH;
    use crate::heirloom::hlc::{Hlc, HlcTimestamp};
    use crate::heirloom::log::HeirloomEventLog;
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
    use chrono::Utc;
    use uuid::Uuid;

    fn build(prev_hash: [u8; 32], hlc: HlcTimestamp, payload: &[u8]) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash,
            hlc_ts: hlc,
            observed_at: Utc::now(),
            l0_hash: [0x10; 32],
            l1_hash: [0x20; 32],
            l2_manifest_hash: [0x30; 32],
            lineage_id: LineageId::new("lineage:e2e"),
            author_did: Did::new("did:aegis:e2e"),
            signatures: vec![Signature::new(vec![0x00])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn t12_e2e_hlc_driven_append_chain_of_10() {
        let hlc = Hlc::new();
        let mut log = HeirloomEventLog::new();
        let mut prev = GENESIS_PREV_HASH;
        for i in 0..10 {
            let ts = hlc.tick();
            let ev = build(prev, ts, format!("evt-{i}").as_bytes());
            prev = log.append(ev).unwrap();
        }
        assert_eq!(log.len(), 10);
        assert!(HeirloomEventLog::verify_chain(log.events()).is_ok());
    }

    #[test]
    fn t12_e2e_jsonl_roundtrip_preserves_exact_hashes() {
        let hlc = Hlc::new();
        let mut orig = HeirloomEventLog::new();
        let mut prev = GENESIS_PREV_HASH;
        let mut expected_hashes = Vec::new();
        for i in 0..5 {
            let ts = hlc.tick();
            let ev = build(prev, ts, format!("evt-{i}").as_bytes());
            prev = orig.append(ev).unwrap();
            expected_hashes.push(prev);
        }

        let file = tempfile::NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();
        let replay = HeirloomEventLog::read_jsonl(file.path()).unwrap();

        // Every event's re-computed hash must match the original.
        for (i, ev) in replay.events().iter().enumerate() {
            let h = event_hash(ev).unwrap();
            assert_eq!(h, expected_hashes[i], "hash mismatch at event {i}");
        }
        assert_eq!(replay.tip().unwrap().hash, *expected_hashes.last().unwrap());
    }

    #[test]
    fn t12_e2e_tampered_jsonl_detected_on_read() {
        let hlc = Hlc::new();
        let mut orig = HeirloomEventLog::new();
        let mut prev = GENESIS_PREV_HASH;
        for i in 0..3 {
            let ts = hlc.tick();
            let ev = build(prev, ts, format!("evt-{i}").as_bytes());
            prev = orig.append(ev).unwrap();
        }
        let file = tempfile::NamedTempFile::new().unwrap();
        orig.write_jsonl(file.path()).unwrap();

        // Flip the payload hex of line 2 from "evt-1" = 6576742d31 to "evt-X" = 6576742d58.
        let content = std::fs::read_to_string(file.path()).unwrap();
        let tampered = content.replacen("6576742d31", "6576742d58", 1);
        std::fs::write(file.path(), tampered).unwrap();

        let err = HeirloomEventLog::read_jsonl(file.path()).unwrap_err();
        // Either ChainBreak (mid-chain) or Canonical (if hex encoding breaks):
        assert!(
            matches!(
                err,
                crate::heirloom::log::LogError::ChainBreak { .. }
                    | crate::heirloom::log::LogError::Canonical(_)
                    | crate::heirloom::log::LogError::Json(_)
            ),
            "expected ChainBreak/Canonical/Json, got {err:?}"
        );
    }
}

#[cfg(test)]
mod properties {
    use crate::heirloom::canonical::{canonical_bytes, event_hash};
    use crate::heirloom::enums::{EventTypeTag, PayloadEncoding, SensitivityTier};
    use crate::heirloom::envelope::EventEnvelope;
    use crate::heirloom::genesis::GENESIS_PREV_HASH;
    use crate::heirloom::hlc::{Hlc, HlcTimestamp};
    use crate::heirloom::log::HeirloomEventLog;
    use crate::heirloom::types::{BundleId, Did, LineageId, Signature};
    use chrono::Utc;
    use proptest::prelude::*;
    use uuid::Uuid;

    fn build(prev_hash: [u8; 32], hlc: HlcTimestamp, payload: Vec<u8>) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            schema_version: 1,
            event_type: EventTypeTag::new("observation:user_stated"),
            prev_hash,
            hlc_ts: hlc,
            observed_at: Utc::now(),
            l0_hash: [0x10; 32],
            l1_hash: [0x20; 32],
            l2_manifest_hash: [0x30; 32],
            lineage_id: LineageId::new("lineage:prop"),
            author_did: Did::new("did:aegis:prop"),
            signatures: vec![Signature::new(vec![0x00])],
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            payload_encoding: PayloadEncoding::Plaintext,
            payload,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn p1_any_append_sequence_verifies_as_chain(
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..32), 1..16)
        ) {
            let hlc = Hlc::new();
            let mut log = HeirloomEventLog::new();
            let mut prev = GENESIS_PREV_HASH;
            for p in payloads {
                let ts = hlc.tick();
                let ev = build(prev, ts, p);
                prev = log.append(ev).unwrap();
            }
            prop_assert!(HeirloomEventLog::verify_chain(log.events()).is_ok());
        }

        #[test]
        fn p2_canonical_bytes_deterministic(
            payload in prop::collection::vec(any::<u8>(), 0..128),
            hlc_phys in any::<u64>(),
            hlc_log in any::<u32>(),
        ) {
            let ev = build(
                GENESIS_PREV_HASH,
                HlcTimestamp { phys: hlc_phys, log: hlc_log },
                payload,
            );
            let b1 = canonical_bytes(&ev).unwrap();
            let b2 = canonical_bytes(&ev).unwrap();
            prop_assert_eq!(b1, b2);
        }

        #[test]
        fn p3_event_hash_changes_with_payload_change(
            p1 in prop::collection::vec(any::<u8>(), 1..32),
            p2 in prop::collection::vec(any::<u8>(), 1..32),
        ) {
            prop_assume!(p1 != p2);
            let hlc = HlcTimestamp { phys: 1000, log: 0 };
            let e1 = build(GENESIS_PREV_HASH, hlc, p1);
            let e2_payload = p2;
            let mut e2 = e1.clone();
            e2.payload = e2_payload;
            let h1 = event_hash(&e1).unwrap();
            let h2 = event_hash(&e2).unwrap();
            prop_assert_ne!(h1, h2);
        }

        #[test]
        fn p4_hlc_ticks_strictly_monotonic_under_arbitrary_clock_regressions(
            regressions in prop::collection::vec(any::<u64>(), 1..32)
        ) {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicU64, Ordering};
            let clk = Arc::new(AtomicU64::new(regressions[0].max(1)));
            let clk_clone = clk.clone();
            let hlc = Hlc::with_clock(Box::new(move || clk_clone.load(Ordering::SeqCst)));
            let mut last = HlcTimestamp::zero();
            for t in &regressions {
                clk.store(*t, Ordering::SeqCst);
                let now = hlc.tick();
                prop_assert!(now > last, "HLC regressed: {:?} -> {:?}", last, now);
                last = now;
            }
        }

        #[test]
        fn p5_any_non_tail_tamper_invalidates_chain(
            length in 2usize..8,
            tamper_index in 0usize..7,
        ) {
            // Exclude tail and out-of-range — verify_chain only checks LINKS
            // (prev_hash → event_hash), so tail tampers don't break it
            // (tail has no successor). Full tip-integrity is a signed-tip
            // concern that arrives in W3. prop_assume rejects sub-samples
            // cleanly so every sample that runs actually asserts.
            prop_assume!(tamper_index < length - 1);
            let hlc = Hlc::new();
            let mut log = HeirloomEventLog::new();
            let mut prev = GENESIS_PREV_HASH;
            for i in 0..length {
                let ts = hlc.tick();
                let ev = build(prev, ts, format!("e{i}").into_bytes());
                prev = log.append(ev).unwrap();
            }
            let mut events: Vec<EventEnvelope> = log.events().to_vec();
            // Tamper: change payload of events[tamper_index] AFTER write.
            events[tamper_index].payload = vec![0xFFu8; 4];
            let r = HeirloomEventLog::verify_chain(&events);
            prop_assert!(r.is_err(), "non-tail tamper at {tamper_index}/{length} must fail verify");
        }

        #[test]
        fn p6_envelope_serde_roundtrip_preserves_every_field(
            payload in prop::collection::vec(any::<u8>(), 0..64),
            hlc_phys in any::<u64>(),
            hlc_log in any::<u32>(),
        ) {
            let ev = build(
                GENESIS_PREV_HASH,
                HlcTimestamp { phys: hlc_phys, log: hlc_log },
                payload,
            );
            let json = serde_json::to_string(&ev).unwrap();
            let back: EventEnvelope = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back, ev);
        }

        #[test]
        fn p7_hlc_phys_distinct_u64_produce_distinct_event_hash(
            a in any::<u64>(),
            b in any::<u64>(),
        ) {
            // Regression gate: serde_jcs 0.2.0 f64-coerces u64 > 2^53 without
            // stringification. If `phys` ever stops being serialized as a
            // string, this test will fail for high-value a/b pairs that
            // differ only at bits below 2^53 precision.
            prop_assume!(a != b);
            let hlc_a = HlcTimestamp { phys: a, log: 0 };
            let hlc_b = HlcTimestamp { phys: b, log: 0 };
            let ev_a = build(GENESIS_PREV_HASH, hlc_a, vec![0u8]);
            let ev_b = build(GENESIS_PREV_HASH, hlc_b, vec![0u8]);
            let h_a = event_hash(&ev_a).unwrap();
            let h_b = event_hash(&ev_b).unwrap();
            prop_assert_ne!(h_a, h_b, "u64 phys coerced to f64 — JCS precision loss");
        }
    }
}

// ── Phase D Task 10: end-to-end integration smoke test ───────────────────

#[cfg(test)]
mod phase_d_e2e {
    use crate::heirloom::owner_state_machine::OwnerStateMachine;
    use crate::heirloom::query::{IntentCategory, IntentClass, Scene};
    use crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS;
    use crate::heirloom::seams::ConstitutionTuple;
    use crate::heirloom::types::{Did, LineageId, OwnerState, WitnessChannel};
    use crate::heirloom::writer::WriterError;
    use crate::heirloom::writer::tests::{build_test_envelope, build_writer_with_registry};
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    #[test]
    fn t_d10_phase_d_e2e_bootstrap_reject_transition_scene_query() {
        // 1. Bootstrap
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        // 2. Unknown event rejected with audit
        let bogus = build_test_envelope("bogus:unregistered");
        let err = w.append_event(bogus);
        assert!(matches!(err, Err(WriterError::SchemaUnknown { .. })));
        let audit_count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'ai:schema_rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit_count, 1);

        // 3. Owner state transition
        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        assert_eq!(sm.current(), OwnerState::Alive);
        sm.transition(&w, OwnerState::AbsentSoft, "e2e test")
            .unwrap();

        // 4. Scene judge — sample a financial intent at weekday noon
        let noon = Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap();
        let s = Scene::judge_deterministic(
            noon,
            WitnessChannel::Mcp,
            &[IntentClass::Category(IntentCategory::Financial)],
            false,
        );
        assert_eq!(s, Scene::Financial);

        // 5. Cold start OwnerStateMachine — reads AbsentSoft from log
        let sm2 = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        assert_eq!(sm2.current(), OwnerState::AbsentSoft);
    }

    #[test]
    fn t_d13_phase_d_e2e_deprecation_and_ethics_roundtrip() {
        // Extends Task 10 E2E with the Round-2 items:
        // - schema:type_deprecated round-trip (Task 12)
        // - ethics gate fires on AI-initiated autonomy state transition (Task 13)
        let w = build_writer_with_registry();
        w.register_projector(Arc::new(
            crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
        ));
        w.bootstrap_fresh_log_with_types(
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
            SCHEMA_TYPE_DEFS,
        )
        .unwrap();

        // 1. Happy-path: append a learned:interest event
        let mut good = build_test_envelope("learned:interest");
        good.payload = br#"{"topic":"xiangqi"}"#.to_vec();
        w.append_event(good).unwrap();

        // 2. Deprecate the type
        w.deprecate_type("learned:interest", "replaced by profile:slot_upserted")
            .unwrap();

        // 3. New append of the deprecated type fails
        let mut blocked = build_test_envelope("learned:interest");
        blocked.payload = br#"{"topic":"go"}"#.to_vec();
        let r = w.append_event(blocked);
        assert!(matches!(r, Err(WriterError::SchemaUnknown { .. })));

        // 4. Historical event STILL queryable
        let count: i64 = w
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:interest'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "historical event preserved across deprecation");

        // 5. OwnerStateMachine transition — PermissiveEthics allows it;
        // autonomy: event lands, is_ai_action matches the autonomy: prefix,
        // ethics gate runs and returns Allow.
        let sm = OwnerStateMachine::new_from_log(
            w.shared_connection(),
            LineageId::new("lineage:test"),
            Did::new("did:aegis:test"),
            ConstitutionTuple::zeros(),
        )
        .unwrap();
        sm.transition(&w, OwnerState::AbsentSoft, "e2e").unwrap();
        assert_eq!(sm.current(), OwnerState::AbsentSoft);

        let transition_count: i64 = w.conn.lock().query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'autonomy:state_transitioned'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(transition_count, 1);

        // 6. Scene sanity — Financial priority untouched by Round-2 changes
        let noon = Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap();
        let s = Scene::judge_deterministic(
            noon,
            WitnessChannel::Mcp,
            &[IntentClass::Category(IntentCategory::Financial)],
            false,
        );
        assert_eq!(s, Scene::Financial);
    }
}
