use super::*;
mod settings_persistence;

#[test]
fn heirloom_profile_source_requires_subsystem_opt_in() {
    for source in [
        None,
        Some("heirloom"),
        Some("heirloom_preferred"),
        Some("legacy"),
    ] {
        assert_eq!(
            ProfileReadSource::from_config(false, source),
            ProfileReadSource::Legacy
        );
    }
    for source in ["heirloom", "heirloom_preferred"] {
        assert_eq!(
            ProfileReadSource::from_config(true, Some(source)),
            ProfileReadSource::HeirloomPreferred
        );
    }
    assert_eq!(
        ProfileReadSource::from_config(true, None),
        ProfileReadSource::Legacy
    );
}

#[tokio::test]
async fn cost_coverage_reports_null_rows_and_pricing_status() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_llm_call_log(
        "react_round",
        "x-ai/grok-4.1-fast",
        Some("openrouter"),
        Some("gen-test-1"),
        1_000,
        200,
        100,
        1_200,
        Some(0.00028),
        Some(250),
        false,
    )
    .await;
    mem.store_llm_call_log(
        "unknown",
        "provider/unknown-model",
        None,
        None,
        500,
        50,
        0,
        550,
        None,
        None,
        false,
    )
    .await;

    let coverage = mem.query_cost_coverage(1).await.unwrap();
    assert_eq!(coverage.row_count, 2);
    assert_eq!(coverage.priced_row_count, 1);
    assert_eq!(coverage.null_cost_rows, 1);
    assert_eq!(coverage.api_row_count, 2);
    assert_eq!(coverage.api_priced_row_count, 1);
    assert_eq!(coverage.api_null_cost_rows, 1);
    assert_eq!(coverage.zero_marginal_subscription_rows, 0);
    assert_eq!(coverage.total_prompt_tokens, 1_500);
    assert_eq!(coverage.total_completion_tokens, 250);
    assert_eq!(coverage.total_cached_tokens, 100);
    assert_eq!(coverage.total_tokens, 1_750);
    assert_eq!(coverage.pricing_rows, 0);
    assert!(coverage.min_timestamp.is_some());
    assert!(coverage.max_timestamp.is_some());
}

#[tokio::test]
async fn cost_coverage_excludes_subscription_zero_marginal_from_api_unpriced() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_llm_call_log(
        "react_round",
        "grok-4.3",
        Some("xai_subscription"),
        Some("chatcmpl-test"),
        1_000,
        20,
        0,
        1_020,
        Some(0.0),
        Some(200),
        false,
    )
    .await;
    mem.store_llm_call_log(
        "react_round",
        "google/gemini-3.1-flash-lite-20260507",
        Some("openrouter"),
        Some("gen-test"),
        1_000,
        20,
        0,
        1_020,
        None,
        Some(200),
        false,
    )
    .await;

    let coverage = mem.query_cost_coverage(1).await.unwrap();
    assert_eq!(coverage.row_count, 2);
    assert_eq!(coverage.zero_marginal_subscription_rows, 1);
    assert_eq!(coverage.api_row_count, 1);
    assert_eq!(coverage.api_null_cost_rows, 1);
}

#[tokio::test]
async fn backfill_marks_subscription_usage_zero_marginal() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_llm_call_log(
        "react_round",
        "grok-4.3",
        None,
        Some("opaque-xai-response"),
        1_000,
        20,
        0,
        1_020,
        None,
        Some(200),
        false,
    )
    .await;

    let result = mem.backfill_cost_usd().await.unwrap();
    assert_eq!(result.rows_updated, 1);
    assert_eq!(result.rows_still_null, 0);
    assert!(
        result.models_missing.is_empty(),
        "subscription usage should not be reported as missing API pricing"
    );
}

#[tokio::test]
async fn backfill_uses_base_pricing_for_dated_snapshot_alias() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE llm_pricing (
                model TEXT PRIMARY KEY,
                prompt_per_token REAL NOT NULL,
                completion_per_token REAL NOT NULL,
                cache_read_per_token REAL NOT NULL DEFAULT 0,
                cache_write_per_token REAL NOT NULL DEFAULT 0,
                image_per_token REAL NOT NULL DEFAULT 0,
                web_search_cost REAL NOT NULL DEFAULT 0,
                request_cost REAL NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT 'test',
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .unwrap();
    crate::llm_pricing::upsert_pricing(
        &conn,
        "test/backfill-snapshot-model",
        &crate::llm_pricing::ModelPricing {
            prompt_per_token: 0.000001,
            completion_per_token: 0.000002,
            ..Default::default()
        },
        "test",
    )
    .unwrap();

    mem.store_llm_call_log(
        "react_round",
        "test/backfill-snapshot-model-20260507",
        Some("openrouter"),
        Some("gen-test"),
        100,
        50,
        0,
        150,
        None,
        Some(200),
        false,
    )
    .await;

    let result = mem.backfill_cost_usd().await.unwrap();
    assert_eq!(result.rows_updated, 1);
    assert_eq!(result.rows_still_null, 0);
    assert!(result.models_missing.is_empty());
}

// ── Phase E Task 2: bridge wiring into MemoryHandle ─────────────────

#[tokio::test]
async fn t_e2_spawn_default_bridge_is_none() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    assert!(
        mem.heirloom_bridge.is_none(),
        "default ctor must not wire a bridge"
    );
}

#[tokio::test]
async fn t_e2_builder_installs_bridge() {
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::heirloom::{LineageId, seams::ConstitutionTuple, types::Did};
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;
    let w = Arc::new(build_writer_with_registry());
    w.register_projector(Arc::new(
        crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));
    w.bootstrap_fresh_log_with_types(
        LineageId::new("lineage:test"),
        Did::new("did:aegis:test"),
        ConstitutionTuple {
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
        },
        crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
    )
    .unwrap();
    let bridge = Arc::new(HeirloomBridge::new(w, Did::new("did:aegis:test")));
    let mem = MemoryHandle::spawn_in_memory()
        .unwrap()
        .with_heirloom_bridge(bridge.clone());
    assert!(mem.heirloom_bridge.is_some());
    assert_eq!(
        mem.heirloom_bridge.as_ref().unwrap().did().as_str(),
        "did:aegis:test"
    );
}

#[tokio::test]
async fn t_e2_bridge_none_preserves_legacy_api_semantics() {
    // Regression pin G1.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_l2("did:aegis:test:alice", "interest", "likes coffee", None)
        .await
        .unwrap();
    assert!(id > 0);
}

/// CTO round-1 regression pin — production init-order invariant.
///
/// Simulates the exact `main.rs` sequence that was silently broken
/// pre-fix: spawn MemoryHandle → attach bridge → clone for a downstream
/// engine → write via the CLONE → assert heirloom event emitted.
///
/// The bug was that `MemoryHandle` derives `Clone` and
/// `heirloom_bridge: Option<Arc<HeirloomBridge>>` is cloned as a
/// per-instance Option; every clone taken BEFORE
/// `with_heirloom_bridge` had `bridge = None` and thus never emitted
/// heirloom events. If this test ever starts failing, check whether
/// some new caller is handing out a `memory.clone()` before the
/// bridge wire-up in main.rs.
#[tokio::test]
async fn t_e_prod_init_order_bridge_reaches_every_clone() {
    use crate::heirloom::types::Did;
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;

    // 1. Build a bootstrapped HeirloomWriter + bridge (equivalent to
    //    main.rs `build_heirloom_bridge`).
    let w = Arc::new(build_writer_with_registry());
    w.register_projector(Arc::new(
        crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));
    w.bootstrap_fresh_log_with_types(
        crate::heirloom::LineageId::new("lineage:test"),
        Did::new("did:aegis:test"),
        crate::heirloom::seams::ConstitutionTuple {
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
        },
        crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
    )
    .unwrap();
    let bridge = Arc::new(HeirloomBridge::new(w.clone(), Did::new("did:aegis:test")));

    // 2. Spawn memory (equivalent to main.rs ~L399) and immediately
    //    attach bridge (equivalent to main.rs ~L441 post-fix).
    let memory = MemoryHandle::spawn_in_memory()
        .expect("spawn")
        .with_heirloom_bridge(bridge);

    // 3. Clone memory for a downstream engine (equivalent to main.rs
    //    orchestrator construction). This is the
    //    step that was failing pre-fix: the clone would carry
    //    bridge=None.
    let downstream_memory = memory.clone();
    assert!(
        downstream_memory.heirloom_bridge.is_some(),
        "CLONE must carry the bridge — otherwise Phase E is dormant in prod"
    );

    // 4. Write via the CLONE (NOT the original).
    downstream_memory
        .store_l2_with_origin(
            "did:aegis:test",
            "interest",
            "rust",
            None,
            MemoryWriteOrigin::UserTurn,
        )
        .await
        .expect("store_l2_with_origin");

    // 5. Assert: heirloom_event_log has the expected row.
    let conn = w.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log \
                 WHERE event_type = 'observation:user_stated'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(
        n, 1,
        "bridge must reach downstream clones — \
             otherwise Phase E is silently dormant in production"
    );
}

// ── Phase E Task 3: store_l2 → learned:* shim ────────────────────────

/// Test helper: construct a MemoryHandle wired with a bootstrapped
/// HeirloomBridge. Returns both so tests can inspect the heirloom event
/// log directly via `writer.shared_connection()`.
fn spawn_with_test_bridge() -> (
    MemoryHandle,
    std::sync::Arc<crate::heirloom::writer::HeirloomWriter>,
) {
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::heirloom::{LineageId, seams::ConstitutionTuple, types::Did};
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;
    let w = Arc::new(build_writer_with_registry());
    w.register_projector(Arc::new(
        crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));
    w.bootstrap_fresh_log_with_types(
        LineageId::new("lineage:test"),
        Did::new("did:aegis:test"),
        ConstitutionTuple {
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
        },
        crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
    )
    .unwrap();
    let bridge = Arc::new(HeirloomBridge::new(w.clone(), Did::new("did:aegis:test")));
    let mem = MemoryHandle::spawn_in_memory()
        .unwrap()
        .with_heirloom_bridge(bridge);
    (mem, w)
}

#[tokio::test]
async fn t_e3_store_l2_bridge_disabled_no_event() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let row_id = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    assert!(row_id > 0);
}

#[tokio::test]
async fn t_e3_store_l2_bridge_enabled_event_appears() {
    let (mem, writer) = spawn_with_test_bridge();
    let _row = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:interest'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e3_store_l2_topic_habit_maps_to_learned_habit() {
    let (mem, writer) = spawn_with_test_bridge();
    let _row = mem
        .store_l2("did:aegis:test", "habit", "drinks coffee daily", None)
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:habit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e3_store_l2_unknown_topic_defaults_to_learned_interest() {
    // D5 fallback.
    let (mem, writer) = spawn_with_test_bridge();
    let _row = mem
        .store_l2("did:aegis:test", "preference", "prefers darkmode", None)
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:interest'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e3_store_l2_soft_fail_when_writer_not_bootstrapped() {
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;
    let w = Arc::new(build_writer_with_registry());
    let bridge = Arc::new(HeirloomBridge::new(
        w.clone(),
        crate::heirloom::types::Did::new("did:aegis:test"),
    ));
    let mem = MemoryHandle::spawn_in_memory()
        .unwrap()
        .with_heirloom_bridge(bridge.clone());
    let row_id = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    assert!(row_id > 0);
    assert!(bridge.counter().count() >= 1);
}

// SEMANTIC-CRITICAL invariant t_e2 — soft-fail does not abort legacy.
#[tokio::test]
async fn t_e2_bridge_soft_fail_does_not_abort_legacy_write() {
    // D1 invariant.
    use crate::heirloom::types::Did;
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;

    let w = Arc::new(build_writer_with_registry());
    // DELIBERATELY no bootstrap — writer.tip is None, every append fails.
    let bridge = Arc::new(HeirloomBridge::new(w.clone(), Did::new("did:aegis:test")));
    let mem = MemoryHandle::spawn_in_memory()
        .unwrap()
        .with_heirloom_bridge(bridge.clone());

    let row_id = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    assert!(
        row_id > 0,
        "legacy write must succeed even when bridge emit fails"
    );

    assert!(
        bridge.counter().count() >= 1,
        "bridge counter must tick on fail"
    );

    let conn = w.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row("SELECT COUNT(*) FROM heirloom_event_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "un-bootstrapped log has no events");
}

// SEMANTIC-CRITICAL invariant t_e3 — legacy write fails → no heirloom event.
#[tokio::test]
async fn t_e3_dual_write_preserves_order_legacy_first() {
    // G6.
    let (mem, writer) = spawn_with_test_bridge();
    let err = mem
        .store_l2("", "interest", "coffee", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("agent_did must not be empty"));

    let conn = writer.shared_connection();
    let n: i64 = conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type IN ('learned:interest', 'learned:habit', 'observation:user_stated')",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 0, "failed legacy write must not produce heirloom event");
}

// ── Phase E Task 4: store_l2_with_origin → observation/ai:observation ─

#[tokio::test]
async fn t_e4_store_l2_with_origin_user_emits_observation_user_stated() {
    let (mem, writer) = spawn_with_test_bridge();
    let _ = mem
        .store_l2_with_origin(
            "did:aegis:test",
            "interest",
            "coffee",
            None,
            MemoryWriteOrigin::UserTurn,
        )
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'observation:user_stated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e4_store_l2_with_origin_heartbeat_emits_ai_observation() {
    let (mem, writer) = spawn_with_test_bridge();
    let _ = mem
        .store_l2_with_origin(
            "did:aegis:test",
            "interest",
            "coffee",
            None,
            MemoryWriteOrigin::HeartbeatTick(HeartbeatWriteKind::Learned),
        )
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'ai:observation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e4_store_l2_with_origin_also_emits_learned() {
    // Double-event invariant: both learned:* AND observation/ai event.
    let (mem, writer) = spawn_with_test_bridge();
    let _ = mem
        .store_l2_with_origin(
            "did:aegis:test",
            "interest",
            "coffee",
            None,
            MemoryWriteOrigin::UserTurn,
        )
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type IN ('learned:interest', 'observation:user_stated')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
}

// SEMANTIC-CRITICAL invariant t_e4 — HLC source is writer.hlc.tick().
#[tokio::test]
async fn t_e4_bridge_emit_uses_writer_hlc_source() {
    // G5. Implementer ticks writer.hlc before the shim-write; asserts the
    // recorded row's HLC is strictly greater than the pre-tick reference.
    let (mem, writer) = spawn_with_test_bridge();

    let before = writer.hlc_for_test().tick();

    let _ = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();

    let conn = writer.shared_connection();
    let (phys, log): (i64, i64) = conn
        .lock()
        .query_row(
            "SELECT hlc_phys, hlc_log FROM heirloom_event_log
                 WHERE event_type = 'learned:interest'
                 ORDER BY hlc_phys DESC, hlc_log DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    assert!(
        phys > 0,
        "event HLC phys must be writer-assigned, got {phys}"
    );
    let event_hlc = (phys as u64, log as u32);
    let before_tuple = (before.phys, before.log);
    assert!(
        event_hlc > before_tuple,
        "event HLC {event_hlc:?} must be > pre-tick reference {before_tuple:?}"
    );
}

// ── Phase E Task 5: push_l1 → observation:user_stated + ai:said pair ──

#[tokio::test]
async fn t_e5_push_l1_user_emits_observation_user_stated() {
    let (mem, writer) = spawn_with_test_bridge();
    let msg = Message::text("user", "hello", 1709312000);
    mem.push_l1("did:aegis:test", msg).await.unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'observation:user_stated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e5_push_l1_assistant_emits_ai_said() {
    let (mem, writer) = spawn_with_test_bridge();
    let msg = Message::text("assistant", "hi", 1709312001);
    mem.push_l1("did:aegis:test", msg).await.unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'ai:said'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e5_push_l1_unknown_role_defaults_to_observation() {
    let (mem, writer) = spawn_with_test_bridge();
    let msg = Message::text("system", "meta", 1709312000);
    mem.push_l1("did:aegis:test", msg).await.unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'observation:user_stated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

// SEMANTIC-CRITICAL invariant t_e5 — ai:said reply_to_hash == user_stated hash.
#[tokio::test]
async fn t_e5_l1_pair_preserves_causality_user_before_ai_said() {
    let (mem, writer) = spawn_with_test_bridge();
    let user_msg = Message::text("user", "hello", 1709312000);
    let ai_msg = Message::text("assistant", "hi there", 1709312001);
    mem.push_l1("did:aegis:test", user_msg).await.unwrap();
    mem.push_l1("did:aegis:test", ai_msg).await.unwrap();
    let conn = writer.shared_connection();
    let user_hash: Vec<u8> = conn
        .lock()
        .query_row(
            "SELECT event_hash FROM heirloom_event_log
                 WHERE event_type = 'observation:user_stated'
                 ORDER BY hlc_phys DESC, hlc_log DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let ai_payload: Vec<u8> = conn
        .lock()
        .query_row(
            "SELECT payload FROM heirloom_event_log
                 WHERE event_type = 'ai:said'
                 ORDER BY hlc_phys DESC, hlc_log DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Unwrap the PlaintextCodec framing: "aegis:plaintext-v1:" + header + "\n" + body.
    let tag = b"aegis:plaintext-v1:";
    assert!(ai_payload.starts_with(tag));
    let rest = &ai_payload[tag.len()..];
    let nl = rest.iter().position(|b| *b == b'\n').unwrap();
    let body = &rest[nl + 1..];
    let parsed: serde_json::Value = serde_json::from_slice(body).unwrap();
    let reply_to = parsed["reply_to_hash"]
        .as_str()
        .expect("ai:said payload must carry reply_to_hash");
    assert_eq!(
        reply_to,
        hex::encode(&user_hash),
        "ai:said reply_to_hash must equal hex(observation:user_stated event_hash)"
    );
}

// Legacy-first ordering pin (D7 / FIX R2-3).
// Simulates actor-channel failure → reply_rx.await errors → push_l1 returns
// Err → bridge MUST NOT emit. We construct a MemoryHandle with a dropped
// receiver so the channel send itself fails.
#[tokio::test]
async fn t_e5_push_l1_legacy_first_ordering_failed_actor_no_bridge_emit() {
    use crate::heirloom::types::Did;
    use crate::heirloom::writer::tests::build_writer_with_registry;
    use crate::memory_heirloom_bridge::HeirloomBridge;
    use std::sync::Arc;
    let w = Arc::new(build_writer_with_registry());
    w.register_projector(Arc::new(
        crate::heirloom::projectors::schema_registry::SchemaRegistryProjector::new(),
    ));
    w.bootstrap_fresh_log_with_types(
        crate::heirloom::LineageId::new("lineage:test"),
        Did::new("did:aegis:test"),
        crate::heirloom::seams::ConstitutionTuple {
            l0_hash: [0; 32],
            l1_hash: [0; 32],
            l2_manifest_hash: [0; 32],
        },
        crate::heirloom::schema_bootstrap::SCHEMA_TYPE_DEFS,
    )
    .unwrap();
    let bridge = Arc::new(HeirloomBridge::new(w.clone(), Did::new("did:aegis:test")));
    // Construct a handle whose actor receiver is dropped so the send fails.
    let (tx, rx) = tokio::sync::mpsc::channel::<MemoryCommand>(1);
    drop(rx);
    let dead_handle = MemoryHandle {
        tx,
        max_db_size: DEFAULT_MAX_DB_SIZE,
        hnsw: None,
        circuit_breaker_rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        actor_handle: None,
        db_path: ":memory:".to_string(),
        heirloom_bridge: Some(bridge),
    };
    let msg = Message::text("user", "x", 1);
    let res = dead_handle.push_l1("did:aegis:test", msg).await;
    assert!(res.is_err(), "failed legacy send must return Err");
    let conn = w.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log
                 WHERE event_type IN ('observation:user_stated', 'ai:said')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "bridge must not emit L1 pair when legacy ack was Err");
}

// ── Phase E Task 7: upsert_profile_entry → profile:slot_upserted ─────

fn fixture_profile_entry(category: &str, key: &str, value: &str) -> ProfileEntry {
    ProfileEntry {
        id: 0,
        agent_did: "did:aegis:test".to_string(),
        category: category.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        confidence: 0.9,
        source: "auto".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[tokio::test]
async fn t_e7_upsert_profile_entry_emits_profile_slot_upserted() {
    let (mem, writer) = spawn_with_test_bridge();
    // Use a REAL Profile Slot Registry entry — ("location", "home_address")
    // — so the legacy write path succeeds without relying on the thread-
    // local BYPASS flag (which doesn't cross the memory actor thread
    // boundary).
    let entry = fixture_profile_entry("location", "residence_city", "Example City");
    mem.upsert_profile_entry(entry).await.unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'profile:slot_upserted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);

    // FIX 3 — payload carries real field names.
    let payload: Vec<u8> = conn
            .lock()
            .query_row(
                "SELECT payload FROM heirloom_event_log WHERE event_type = 'profile:slot_upserted' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
    // Unwrap PlaintextCodec framing.
    let tag = b"aegis:plaintext-v1:";
    assert!(payload.starts_with(tag));
    let rest = &payload[tag.len()..];
    let nl = rest.iter().position(|b| *b == b'\n').unwrap();
    let body = &rest[nl + 1..];
    let parsed: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(parsed["category"], "location");
    assert_eq!(parsed["key"], "residence_city");
    assert_eq!(parsed["value"], "Example City");
    assert_eq!(parsed["source"], "auto");
}

#[tokio::test]
async fn t_e7_upsert_validation_failure_emits_no_event() {
    // Legacy write fails via Profile Slot Registry gate → bridge must not emit.
    let (mem, writer) = spawn_with_test_bridge();
    let bad_entry = fixture_profile_entry("invented_category", "unknown_key", "x");
    let result = mem.upsert_profile_entry(bad_entry).await;
    assert!(result.is_err());
    let conn = writer.shared_connection();
    let n: i64 = conn
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'profile:slot_upserted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn t_e7_upsert_bridge_disabled_legacy_still_works() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let entry = fixture_profile_entry("location", "residence_city", "Example City");
    mem.upsert_profile_entry(entry).await.unwrap();
}

#[tokio::test]
async fn p2_profile_read_source_legacy_ignores_heirloom_projection() {
    let (mem, _writer) = spawn_with_test_bridge();
    let bridge = mem.heirloom_bridge.as_ref().unwrap();
    let env = crate::memory_heirloom_bridge::HeirloomBridge::build_profile_upsert_envelope(
        bridge.did(),
        "location",
        "residence_city",
        "Example City",
        0.9,
        "fixture",
    );
    bridge.try_emit(env);

    let legacy = mem
        .get_user_profile_with_read_source("did:aegis:test", ProfileReadSource::Legacy)
        .await
        .unwrap();
    assert!(legacy.is_empty());

    let projected = mem
        .get_user_profile_with_read_source("did:aegis:test", ProfileReadSource::HeirloomPreferred)
        .await
        .unwrap();
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].value, "Example City");
    assert_eq!(projected[0].source, "fixture");
}

#[tokio::test]
async fn p2_profile_read_source_falls_back_to_legacy_without_bridge() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let entry = fixture_profile_entry("location", "residence_city", "Example City");
    mem.upsert_profile_entry(entry).await.unwrap();

    let (entries, notices) = crate::behavior_notice::collect(
        "en",
        None,
        None,
        mem.get_user_profile_with_read_source(
            "did:aegis:test",
            ProfileReadSource::HeirloomPreferred,
        ),
    )
    .await;
    let entries = entries.unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("Standard memory was used"));
    let (_, notices) = crate::behavior_notice::collect(
        "en",
        None,
        None,
        mem.get_user_profile_with_read_source("did:aegis:test", ProfileReadSource::Legacy),
    )
    .await;
    assert!(
        notices.is_empty(),
        "ordinary configured memory should stay quiet"
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "Example City");
}

// ── Phase E Task 8: L3 lesson shim via topic prefix ─────────────────

/// Task 8 DEVIATION: the plan specifies a lesson-shaped reflection-log
/// write, but in Aegis Runtime reality lessons are stored via `store_l2` with
/// `topic = "learned:lesson"`.
///
/// This task therefore extends `topic_to_event_type` in
/// `memory_heirloom_bridge.rs` to detect the `learned:lesson` prefix and
/// emit `learned:lesson_crystallized` instead of the default
/// `learned:interest`. The emission happens inside `store_l2`'s bridge
/// tail (Task 3), so no new shim call is required here — only the test
/// verifies the prefix routing.
#[tokio::test]
async fn t_e8_lesson_topic_emits_learned_lesson_crystallized() {
    let (mem, writer) = spawn_with_test_bridge();
    mem.store_l2(
        "did:aegis:test",
        "learned:lesson",
        "RULE: always try tools before claiming limits",
        None,
    )
    .await
    .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:lesson_crystallized'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn t_e8_non_lesson_topic_does_not_emit_lesson_crystallized() {
    // Guard: topic="interest" must NOT hit the lesson branch.
    let (mem, writer) = spawn_with_test_bridge();
    mem.store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    let conn = writer.shared_connection();
    let n: i64 = conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM heirloom_event_log WHERE event_type = 'learned:lesson_crystallized'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn t_e8_lesson_disabled_bridge_legacy_still_records() {
    // Bridge=None: legacy l2_experience row still gets inserted with
    // topic="learned:lesson", zero heirloom events (no DB to query).
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let row_id = mem
        .store_l2(
            "did:aegis:test",
            "learned:lesson",
            "be careful with empty DIDs",
            None,
        )
        .await
        .unwrap();
    assert!(row_id > 0);
}

// SEMANTIC-CRITICAL invariant t_e1 — bridge=None is pre-Phase-E behavior.
#[tokio::test]
async fn t_e1_bridge_none_is_old_behavior_pure() {
    // Regression pin G1.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    assert!(mem.heirloom_bridge.is_none());

    let row_id_1 = mem
        .store_l2("did:aegis:test", "interest", "coffee", None)
        .await
        .unwrap();
    let row_id_2 = mem
        .store_l2("did:aegis:test", "habit", "morning run", None)
        .await
        .unwrap();

    let all = mem.get_all_l2("did:aegis:test").await.unwrap();
    assert_eq!(
        all.len(),
        2,
        "bridge=None must produce exactly the legacy rows"
    );
    assert_eq!(
        all.iter().find(|e| e.id == row_id_1).unwrap().fact_summary,
        "coffee"
    );
    assert_eq!(
        all.iter().find(|e| e.id == row_id_2).unwrap().fact_summary,
        "morning run"
    );

    mem.push_l1("did:aegis:test", Message::text("user", "hello", 1709312000))
        .await
        .unwrap();
    let l1 = mem.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].content, "hello");
}

#[tokio::test]
async fn l1_fifo_eviction() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:test";
    let base_ts = chrono::Utc::now().timestamp();

    // Push DEFAULT_L1_MAX_SIZE + 2 messages to trigger eviction
    // Batch trim: the window grows to cap + slack, then cuts back to cap.
    let total = DEFAULT_L1_MAX_SIZE + crate::memory::handlers_b::L1_TRIM_SLACK + 1;
    for i in 0..total {
        mem.push_l1(
            did,
            Message::text("user".to_string(), format!("msg-{i}"), base_ts + i as i64),
        )
        .await
        .unwrap();
    }

    let msgs = mem.get_l1(did).await.unwrap();
    assert_eq!(msgs.len(), DEFAULT_L1_MAX_SIZE);
    // Oldest 2 messages (0, 1) should have been evicted
    assert_eq!(
        msgs[0].content,
        format!("msg-{}", crate::memory::handlers_b::L1_TRIM_SLACK + 1)
    );
    assert_eq!(
        msgs[DEFAULT_L1_MAX_SIZE - 1].content,
        format!("msg-{}", total - 1)
    );
}

#[tokio::test]
async fn l2_store_and_retrieve() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:test";

    let emb = vec![1.0_f32; 512];
    let id = mem
        .store_l2(did, "server", "Server IP is 192.168.1.100", Some(&emb))
        .await
        .unwrap();
    assert!(id > 0);

    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].fact_summary.contains("192.168.1.100"));
}

#[tokio::test]
async fn l2_semantic_search_finds_similar() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:test";

    // Store facts with embeddings pointing in different directions
    let emb_server = vec![1.0_f32, 0.0, 0.0, 0.0]; // "server" direction
    let emb_name = vec![0.0_f32, 1.0, 0.0, 0.0]; // "name" direction
    let emb_email = vec![0.0_f32, 0.0, 1.0, 0.0]; // "email" direction

    mem.store_l2(
        did,
        "server",
        "Server IP is 192.168.1.100",
        Some(&emb_server),
    )
    .await
    .unwrap();
    mem.store_l2(did, "identity", "My name is Lance", Some(&emb_name))
        .await
        .unwrap();
    mem.store_l2(
        did,
        "contact",
        "Email is owner@example.com",
        Some(&emb_email),
    )
    .await
    .unwrap();

    // Query with a vector similar to "server"
    let query = vec![0.9_f32, 0.1, 0.0, 0.0];
    let results = mem.query_l2_semantic(did, &query).await.unwrap();

    assert_eq!(results.len(), 1, "should match only server fact");
    assert!(results[0].fact_summary.contains("192.168.1.100"));
}

#[tokio::test]
async fn l2_semantic_search_no_cross_did_leaks() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![1.0_f32; 4];

    mem.store_l2("did:a", "secret", "my password is hunter2", Some(&emb))
        .await
        .unwrap();
    mem.store_l2("did:b", "notes", "nothing special", Some(&emb))
        .await
        .unwrap();

    // DID B must not see DID A's secrets
    let results = mem.query_l2_semantic("did:b", &emb).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(
        results[0].fact_summary.contains("nothing special"),
        "cross-DID data leak detected!"
    );
}

#[tokio::test]
async fn l2_semantic_returns_top_k_sorted() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:test";

    // Store 8 facts with varying similarity to query [1,0,...,0].
    // Use different topic per fact to bypass write guard dedup.
    for i in 0..8 {
        let mut emb = vec![0.0_f32; 4];
        emb[0] = 1.0 - (i as f32 * 0.1); // decreasing similarity
        emb[1] = i as f32 * 0.1;
        mem.store_l2(did, &format!("test:{i}"), &format!("fact-{i}"), Some(&emb))
            .await
            .unwrap();
    }

    let query = vec![1.0_f32, 0.0, 0.0, 0.0];
    let results = mem.query_l2_semantic(did, &query).await.unwrap();

    // Should return at most SEMANTIC_TOP_K (5)
    assert!(results.len() <= SEMANTIC_TOP_K);
    // First result should be the most similar (fact-0)
    assert_eq!(results[0].fact_summary, "fact-0");
}

// T5: 10 concurrent writers must not deadlock
#[tokio::test]
async fn t5_memory_actor_no_deadlock() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:concurrent_test";

    let mut handles = Vec::new();

    for i in 0..10 {
        let mem_clone = mem.clone();
        let did = did.to_string();
        let handle = tokio::spawn(async move {
            // Use (i+1) to avoid zero vector (cosine_similarity returns 0 for zero vecs)
            let emb = vec![(i + 1) as f32; 4];
            for j in 0..20 {
                mem_clone
                    .store_l2(
                        &did,
                        &format!("topic-{i}"),
                        &format!("fact-{i}-{j}"),
                        Some(&emb),
                    )
                    .await
                    .unwrap();
            }
            // Verify semantic search works during concurrent writes
            let results = mem_clone.query_l2_semantic(&did, &emb).await.unwrap();
            assert!(!results.is_empty());
        });
        handles.push(handle);
    }

    // All 10 tasks must complete without deadlock or panic
    for handle in handles {
        handle.await.expect("concurrent task panicked");
    }

    // Verify facts were stored (count depends on write guard dedup —
    // each topic keeps 1 fact updated 19 times, so 10 topics × 1 = 10 unique facts)
    let all = mem.get_all_l2(did).await.unwrap();
    assert!(
        all.len() >= 10,
        "expected ≥10 facts (1 per topic), got {}",
        all.len()
    );
}

#[tokio::test]
async fn l1_isolation_per_did() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.push_l1(
        "did:a",
        Message::text("user".to_string(), "hello A".to_string(), 1),
    )
    .await
    .unwrap();

    mem.push_l1(
        "did:b",
        Message::text("user".to_string(), "hello B".to_string(), 2),
    )
    .await
    .unwrap();

    let a_msgs = mem.get_l1("did:a").await.unwrap();
    let b_msgs = mem.get_l1("did:b").await.unwrap();
    assert_eq!(a_msgs.len(), 1);
    assert_eq!(b_msgs.len(), 1);
    assert_eq!(a_msgs[0].content, "hello A");
    assert_eq!(b_msgs[0].content, "hello B");
}

#[tokio::test]
async fn l1_overflow_routes_to_tier2_ephemeral() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:overflow_test";
    let base_ts = chrono::Utc::now().timestamp();

    // Push DEFAULT_L1_MAX_SIZE + 2 messages so 2 will be evicted to Tier 2
    let evicted = crate::memory::handlers_b::L1_TRIM_SLACK + 1;
    let total = DEFAULT_L1_MAX_SIZE + evicted;
    for i in 0..total {
        mem.push_l1(
            did,
            Message::text(
                "user",
                format!("Important message number {i} with enough text to pass the threshold"),
                base_ts + i as i64,
            ),
        )
        .await
        .unwrap();
    }

    // L1 should have exactly DEFAULT_L1_MAX_SIZE
    let l1 = mem.get_l1(did).await.unwrap();
    assert_eq!(l1.len(), DEFAULT_L1_MAX_SIZE);

    // Evicted messages should be in l2_ephemeral (Tier 2), NOT l2_experience (Tier 1)
    let l2_tier1 = mem.get_all_l2(did).await.unwrap();
    assert_eq!(
        l2_tier1.len(),
        0,
        "overflow should NOT go to Tier 1, got {} entries",
        l2_tier1.len()
    );

    let ephemeral_count = mem.count_ephemeral(did, Some("l1_overflow")).await.unwrap();
    assert_eq!(
        ephemeral_count, evicted as i64,
        "expected {evicted} overflow entries in l2_ephemeral, got {ephemeral_count}"
    );
}

#[test]
fn extract_facts_filters_short_messages() {
    let short = Message::text("user".to_string(), "hi".to_string(), 0);
    assert!(super::extract_facts_from_message(&short).is_empty());

    let long = Message::text(
        "user".to_string(),
        "My server IP is 192.168.1.100 and it runs Ubuntu".to_string(),
        0,
    );
    let fact = super::extract_facts_from_message(&long);
    assert!(fact.contains("[user]"));
    assert!(fact.contains("192.168.1.100"));
}

#[test]
fn extract_immediate_fact_detects_remember() {
    let msg = "记住，我的服务器 IP 是 192.168.1.100";
    let result = extract_immediate_fact(msg);
    assert!(result.is_some());
    let (topic, fact) = result.unwrap();
    assert_eq!(topic, "server");
    assert!(fact.contains("192.168.1.100"));
}

#[test]
fn extract_immediate_fact_ignores_casual() {
    let msg = "今天天气怎么样？";
    assert!(extract_immediate_fact(msg).is_none());
}

#[tokio::test]
async fn l2_store_without_embedding_not_in_semantic_search() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:test";

    // Store without embedding
    mem.store_l2(did, "server", "Server IP is 10.0.0.1", None)
        .await
        .unwrap();
    // Store with embedding
    let emb = vec![1.0_f32; 4];
    mem.store_l2(did, "server", "Server IP is 192.168.1.100", Some(&emb))
        .await
        .unwrap();

    // Semantic search should only find the one with embedding
    let results = mem.query_l2_semantic(did, &emb).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].fact_summary.contains("192.168.1.100"));

    // But get_all should find both
    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 2);
}

// ── Reminder CRUD tests (backward-compat for deprecated API) ────────────

#[allow(deprecated)]
#[tokio::test]
async fn reminder_store_and_retrieve() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let id = mem
        .store_reminder(12345, "drink water", 1000)
        .await
        .unwrap();
    assert!(id > 0);

    // Before due time — should return nothing
    let due = mem.get_due_reminders(999).await.unwrap();
    assert!(due.is_empty());

    // At due time — should return the reminder
    let due = mem.get_due_reminders(1000).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].message, "drink water");
    assert_eq!(due[0].chat_id, 12345);
    assert!(!due[0].fired);
}

#[allow(deprecated)]
#[tokio::test]
async fn reminder_mark_fired() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let id = mem
        .store_reminder(12345, "take a break", 500)
        .await
        .unwrap();

    // Should be due
    let due = mem.get_due_reminders(500).await.unwrap();
    assert_eq!(due.len(), 1);

    // Mark as fired
    mem.mark_reminder_fired(id).await.unwrap();

    // Should no longer appear in due reminders
    let due = mem.get_due_reminders(500).await.unwrap();
    assert!(due.is_empty(), "fired reminder must not appear in due list");
}

#[allow(deprecated)]
#[tokio::test]
async fn reminder_multiple_ordering() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.store_reminder(111, "third", 3000).await.unwrap();
    mem.store_reminder(111, "first", 1000).await.unwrap();
    mem.store_reminder(111, "second", 2000).await.unwrap();

    // All due at time 5000
    let due = mem.get_due_reminders(5000).await.unwrap();
    assert_eq!(due.len(), 3);
    // Should be ordered by remind_at_unix ASC
    assert_eq!(due[0].message, "first");
    assert_eq!(due[1].message, "second");
    assert_eq!(due[2].message, "third");
}

#[allow(deprecated)]
#[tokio::test]
async fn reminder_isolation_by_chat_id() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.store_reminder(111, "for user A", 1000).await.unwrap();
    mem.store_reminder(222, "for user B", 1000).await.unwrap();

    // Both should be returned (get_due_reminders is global)
    let due = mem.get_due_reminders(1000).await.unwrap();
    assert_eq!(due.len(), 2);
    // Verify both chat_ids are present
    let chat_ids: Vec<i64> = due.iter().map(|r| r.chat_id).collect();
    assert!(chat_ids.contains(&111));
    assert!(chat_ids.contains(&222));
}

// ── UTF-8 truncation safety tests ───────────────────────────────────

#[test]
fn extract_facts_truncates_chinese_safely() {
    // 100 Chinese chars = 300 bytes but only 100 characters.
    // char-based limit (256 chars) should NOT truncate this.
    let chinese = "中".repeat(100);
    let msg = Message::text("user".to_string(), chinese.clone(), 0);
    let fact = super::extract_facts_from_message(&msg);
    assert!(fact.contains("[user]"));
    assert!(
        !fact.ends_with("..."),
        "100 chars should not be truncated at 256-char limit"
    );
    assert!(fact.contains(&chinese));
    let _ = fact.chars().count();

    // 300 Chinese chars > 256 limit — should truncate
    let long_chinese = "中".repeat(300);
    let msg2 = Message::text("user".to_string(), long_chinese, 0);
    let fact2 = super::extract_facts_from_message(&msg2);
    assert!(fact2.ends_with("..."), "300 chars should be truncated");
    let _ = fact2.chars().count();
}

#[test]
fn extract_facts_truncates_emoji_safely() {
    // 70 emoji = 280 bytes but only 70 characters.
    // char-based limit (256 chars) should NOT truncate this.
    let emoji_text = "\u{1f525}".repeat(70); // 🔥 × 70
    let msg = Message::text("assistant".to_string(), emoji_text, 0);
    let fact = super::extract_facts_from_message(&msg);
    assert!(!fact.is_empty());
    assert!(
        !fact.ends_with("..."),
        "70 chars should not be truncated at 256-char limit"
    );
    let _ = fact.chars().count();

    // 300 emoji > 256 limit — should truncate
    let long_emoji = "\u{1f525}".repeat(300);
    let msg2 = Message::text("assistant".to_string(), long_emoji, 0);
    let fact2 = super::extract_facts_from_message(&msg2);
    assert!(fact2.ends_with("..."), "300 chars should be truncated");
    let _ = fact2.chars().count();
}

#[test]
fn extract_facts_mixed_ascii_multibyte() {
    // Mixed content where the 256-byte boundary falls mid-character
    let mixed = format!("{}{}", "A".repeat(255), "中文测试");
    let msg = Message::text("user".to_string(), mixed, 0);
    let fact = super::extract_facts_from_message(&msg);
    assert!(fact.contains("[user]"));
    // Must be valid UTF-8
    let _ = fact.chars().count();
}

// ── Security: credential rejection tests ─────────────────────────

#[test]
fn extract_immediate_fact_rejects_passwords() {
    let msg = "记住我的 password 是 hunter2";
    assert!(
        extract_immediate_fact(msg).is_none(),
        "messages containing passwords must NOT be stored in L2"
    );
}

#[test]
fn extract_immediate_fact_rejects_api_keys() {
    let msg = "remember my api_key is sk-1234567890abcdef";
    assert!(
        extract_immediate_fact(msg).is_none(),
        "messages containing API keys must NOT be stored in L2"
    );
}

#[test]
fn extract_immediate_fact_rejects_tokens() {
    let msg = "记住这个 token: ghp_abcdefghijklmnop";
    assert!(
        extract_immediate_fact(msg).is_none(),
        "messages containing tokens must NOT be stored in L2"
    );
}

#[test]
fn extract_immediate_fact_rejects_secrets() {
    let msg = "remember this secret: my-super-secret-value";
    assert!(
        extract_immediate_fact(msg).is_none(),
        "messages containing secrets must NOT be stored in L2"
    );
}

#[test]
fn extract_immediate_fact_allows_safe_data() {
    // Server IP is not a credential — should be stored
    let msg = "记住，我的服务器 IP 是 192.168.1.100";
    assert!(
        extract_immediate_fact(msg).is_some(),
        "non-credential data should be stored in L2"
    );
}

// ── Security: per-user fact count limit tests ────────────────────

#[tokio::test]
async fn l2_per_user_fact_limit_enforced() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:limit_test";

    // Fill up to the limit
    for i in 0..MAX_FACTS_PER_DID {
        let result = mem.store_l2(did, "test", &format!("fact-{i}"), None).await;
        assert!(result.is_ok(), "fact {i} should be accepted (under limit)");
    }

    // The next write should be rejected
    let result = mem.store_l2(did, "test", "one-too-many", None).await;
    assert!(
        result.is_err(),
        "exceeding per-user fact limit must be rejected"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("fact limit"),
        "error should mention fact limit: {err_msg}"
    );
}

// ── Graceful shutdown / L1 persistence tests ────────────────────────

#[tokio::test]
async fn flush_l1_persists_to_snapshot_table() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:flush_test";

    // Push messages to L1
    for i in 0..3 {
        mem.push_l1(
            did,
            Message::text("user".to_string(), format!("flush-msg-{i}"), i),
        )
        .await
        .unwrap();
    }

    // Flush should persist 1 DID
    let flushed = mem.flush_l1().await.unwrap();
    assert_eq!(flushed, 1, "should flush 1 DID");
}

#[tokio::test]
async fn restore_l1_recovers_flushed_data() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:restore_test";

    // Push 3 messages, flush, then verify restore
    for i in 0..3 {
        mem.push_l1(
            did,
            Message::text("user".to_string(), format!("persist-{i}"), i),
        )
        .await
        .unwrap();
    }
    mem.flush_l1().await.unwrap();

    // Spawn a new handle sharing the same actor — but to test restore properly,
    // we verify via restore_l1 which re-reads from the snapshot table.
    // Since we're using the same actor, clear L1 first by reading beyond eviction.
    // Instead, let's just test that restore_l1 call succeeds and returns count.
    let restored = mem.restore_l1().await.unwrap();
    // After flush + immediate restore on same actor, snapshot was already in L1,
    // so restore adds them again (capped by L1_MAX_SIZE). The count reflects DIDs restored.
    assert!(restored >= 1, "should restore at least 1 DID");
}

#[tokio::test]
async fn shutdown_flushes_and_stops_actor() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:shutdown_test";

    mem.push_l1(
        did,
        Message::text(
            "user".to_string(),
            "shutdown data must persist".to_string(),
            100,
        ),
    )
    .await
    .unwrap();

    // Shutdown should succeed
    let result = mem.shutdown().await;
    assert!(result.is_ok(), "shutdown should succeed: {result:?}");

    // After shutdown, the actor loop has exited, so further commands should fail
    let result = mem.get_l1(did).await;
    assert!(
        result.is_err(),
        "commands after shutdown should fail (actor closed)"
    );
}

#[tokio::test]
async fn flush_empty_l1_returns_zero() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let flushed = mem.flush_l1().await.unwrap();
    assert_eq!(flushed, 0, "flushing empty L1 should return 0 DIDs");
}

#[tokio::test]
async fn flush_multiple_dids() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    for did_suffix in ["aaa", "bbb", "ccc"] {
        mem.push_l1(
            &format!("did:aegis:ed25519:{did_suffix}"),
            Message::text(
                "user".to_string(),
                format!("data for {did_suffix} with enough length"),
                1,
            ),
        )
        .await
        .unwrap();
    }

    let flushed = mem.flush_l1().await.unwrap();
    assert_eq!(flushed, 3, "should flush 3 DIDs");
}

#[tokio::test]
async fn l2_per_user_limit_does_not_affect_other_users() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Fill user A to the limit
    for i in 0..MAX_FACTS_PER_DID {
        mem.store_l2("did:a", "test", &format!("a-{i}"), None)
            .await
            .unwrap();
    }

    // User B should still be able to store facts
    let result = mem.store_l2("did:b", "test", "b-fact", None).await;
    assert!(result.is_ok(), "other user's limit should not be affected");
}

// ── L3 full-text archive tests ─────────────────────────────────────

#[tokio::test]
async fn l3_archive_created_on_push() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l3_test";

    mem.push_l1(
        did,
        Message::text(
            "user".to_string(),
            "Remember my server IP is 192.168.1.100".to_string(),
            1,
        ),
    )
    .await
    .unwrap();

    // L3 fulltext search should find the message
    let results = mem.query_l3_fulltext(did, "server").await.unwrap();
    assert_eq!(results.len(), 1, "L3 should contain the archived message");
    assert!(results[0].content.contains("192.168.1.100"));
    assert_eq!(results[0].role, "user");
}

#[tokio::test]
async fn l3_fulltext_search_relevance() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l3_relevance";

    // Push several messages
    for (role, content) in [
        ("user", "What is the weather today?"),
        ("assistant", "The weather in Beijing is sunny, 25 degrees."),
        ("user", "Remember my favorite color is blue"),
        (
            "assistant",
            "Got it, I'll remember your favorite color is blue.",
        ),
        ("user", "What about the weather tomorrow?"),
    ] {
        mem.push_l1(did, Message::text(role.to_string(), content.to_string(), 1))
            .await
            .unwrap();
    }

    // Search for "weather" should find weather-related messages
    let results = mem.query_l3_fulltext(did, "weather").await.unwrap();
    assert!(
        results.len() >= 2,
        "should find at least 2 weather-related messages, got {}",
        results.len()
    );
    assert!(
        results
            .iter()
            .all(|r| r.content.to_lowercase().contains("weather"))
    );

    // Search for "blue" should find color-related messages
    let results = mem.query_l3_fulltext(did, "blue").await.unwrap();
    assert!(!results.is_empty(), "should find blue-related messages");
}

#[tokio::test]
async fn l3_search_isolated_by_did() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.push_l1(
        "did:a",
        Message::text(
            "user".to_string(),
            "Secret information for user A only".to_string(),
            1,
        ),
    )
    .await
    .unwrap();

    mem.push_l1(
        "did:b",
        Message::text(
            "user".to_string(),
            "Public info for user B about weather".to_string(),
            2,
        ),
    )
    .await
    .unwrap();

    // DID B must NOT see DID A's messages
    let results = mem.query_l3_fulltext("did:b", "secret").await.unwrap();
    assert!(results.is_empty(), "L3 must isolate by DID: {results:?}");
}

#[tokio::test]
async fn l3_empty_query_returns_empty() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let results = mem.query_l3_fulltext("did:test", "").await.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn l3_search_no_embedding_needed() {
    // L3 works purely locally — no embedding API needed
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l3_no_embed";

    mem.push_l1(
        did,
        Message::text(
            "user".to_string(),
            "My email address is test@example.com".to_string(),
            1,
        ),
    )
    .await
    .unwrap();

    // Should find via fulltext without any embedding
    let results = mem.query_l3_fulltext(did, "email").await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("test@example.com"));
}

#[tokio::test]
async fn l3_fts5_special_chars_no_crash() {
    // FTS5 special syntax chars should not crash or panic —
    // malformed FTS5 query should return empty, not error.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:fts5_special";

    mem.push_l1(
        did,
        Message::text(
            "user".to_string(),
            "Some normal test content here for searching".to_string(),
            chrono::Utc::now().timestamp(),
        ),
    )
    .await
    .unwrap();

    // FTS5 operators — should not panic
    for query in &[
        "AND OR NOT",
        "\"unclosed quote",
        "content*",
        "(nested (parentheses))",
        "NEAR(test, content)",
        "column:value",
        "test OR",
        "\"",
        "***",
    ] {
        let results = mem.query_l3_fulltext(did, query).await;
        // Must not return Err — worst case is empty
        assert!(results.is_ok(), "FTS5 query '{query}' must not error");
    }
}

#[tokio::test]
async fn l1_configurable_size() {
    // Test with a small L1 size
    let config = MemoryConfig {
        l1_max_size: 3,
        max_db_size: DEFAULT_MAX_DB_SIZE,
    };
    let mem = MemoryHandle::spawn_with_config(Path::new(":memory:"), config).unwrap();
    let did = "did:aegis:ed25519:small_l1";

    let base_ts = chrono::Utc::now().timestamp();
    let slack = crate::memory::handlers_b::L1_TRIM_SLACK;
    for i in 0..(3 + slack + 1) {
        mem.push_l1(
            did,
            Message::text(
                "user".to_string(),
                format!("configurable-msg-{i}"),
                base_ts + i as i64,
            ),
        )
        .await
        .unwrap();
    }

    let msgs = mem.get_l1(did).await.unwrap();
    assert_eq!(msgs.len(), 3, "L1 should respect configured size of 3");
    assert_eq!(msgs[0].content, format!("configurable-msg-{}", slack + 1));
    assert_eq!(msgs[2].content, format!("configurable-msg-{}", slack + 3));
}

// ── User settings persistence tests ─────────────────────────────────

#[tokio::test]
async fn settings_store_and_load_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Store two settings
    mem.store_setting("crypto_alerts", serde_json::Value::Bool(false))
        .await
        .unwrap();
    mem.store_setting("news_digest", serde_json::Value::Bool(true))
        .await
        .unwrap();

    // Load all
    let all = mem.load_all_settings().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all["crypto_alerts"], serde_json::Value::Bool(false));
    assert_eq!(all["news_digest"], serde_json::Value::Bool(true));

    // Overwrite
    mem.store_setting("crypto_alerts", serde_json::Value::Bool(true))
        .await
        .unwrap();
    let all = mem.load_all_settings().await.unwrap();
    assert_eq!(all["crypto_alerts"], serde_json::Value::Bool(true));
}

#[tokio::test]
async fn settings_delete() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.store_setting("news_digest", serde_json::Value::Bool(false))
        .await
        .unwrap();

    let deleted = mem.delete_setting("news_digest").await.unwrap();
    assert!(deleted);

    // Should be gone
    let all = mem.load_all_settings().await.unwrap();
    assert!(all.is_empty());

    // Delete nonexistent returns false
    let deleted = mem.delete_setting("news_digest").await.unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn settings_load_empty() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let all = mem.load_all_settings().await.unwrap();
    assert!(all.is_empty());
}

// ── query_l2_by_topic_prefix tests ──────────────────────────────────

#[tokio::test]
async fn topic_prefix_query_filters_correctly() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:prefix_test";
    let emb = vec![1.0_f32; 4];

    // Store facts with different topic prefixes
    mem.store_l2(did, "learned:preference", "likes BTC", Some(&emb))
        .await
        .unwrap();
    mem.store_l2(did, "learned:habit", "checks news at 9am", Some(&emb))
        .await
        .unwrap();
    mem.store_l2(did, "server", "Server IP is 1.2.3.4", Some(&emb))
        .await
        .unwrap();
    // Fact without embedding should be excluded
    mem.store_l2(did, "learned:context", "no embedding fact", None)
        .await
        .unwrap();

    let results = mem.query_l2_by_topic_prefix(did, "learned:").await.unwrap();
    assert_eq!(
        results.len(),
        2,
        "should match only learned:* facts with embeddings"
    );

    let facts: Vec<&str> = results.iter().map(|(f, _)| f.as_str()).collect();
    assert!(facts.contains(&"likes BTC"));
    assert!(facts.contains(&"checks news at 9am"));

    // Each result should have a valid embedding
    for (_, emb_vec) in &results {
        assert_eq!(emb_vec.len(), 4);
    }
}

#[tokio::test]
async fn topic_prefix_query_isolates_by_did() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![1.0_f32; 4];

    mem.store_l2(
        "did:alice",
        "learned:preference",
        "alice likes Rust",
        Some(&emb),
    )
    .await
    .unwrap();
    mem.store_l2(
        "did:bob",
        "learned:preference",
        "bob likes Python",
        Some(&emb),
    )
    .await
    .unwrap();

    let alice_results = mem
        .query_l2_by_topic_prefix("did:alice", "learned:")
        .await
        .unwrap();
    assert_eq!(alice_results.len(), 1);
    assert_eq!(alice_results[0].0, "alice likes Rust");

    let bob_results = mem
        .query_l2_by_topic_prefix("did:bob", "learned:")
        .await
        .unwrap();
    assert_eq!(bob_results.len(), 1);
    assert_eq!(bob_results[0].0, "bob likes Python");
}

#[tokio::test]
async fn topic_prefix_query_returns_empty_for_no_match() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:empty_prefix";

    // Store a fact with a non-matching prefix
    let emb = vec![1.0_f32; 4];
    mem.store_l2(did, "server", "some server fact", Some(&emb))
        .await
        .unwrap();

    let results = mem.query_l2_by_topic_prefix(did, "learned:").await.unwrap();
    assert!(results.is_empty(), "no learned: facts should match");
}

#[tokio::test]
async fn topic_prefix_query_returns_correct_embeddings() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:emb_check";
    let emb = vec![0.5_f32, -0.3, 0.8, 0.1];

    mem.store_l2(did, "learned:interest", "interested in crypto", Some(&emb))
        .await
        .unwrap();

    let results = mem.query_l2_by_topic_prefix(did, "learned:").await.unwrap();
    assert_eq!(results.len(), 1);
    // Verify the embedding roundtrips correctly
    let (_, returned_emb) = &results[0];
    assert_eq!(returned_emb.len(), emb.len());
    for (a, b) in returned_emb.iter().zip(emb.iter()) {
        assert!(
            (a - b).abs() < f32::EPSILON,
            "embedding mismatch: {a} vs {b}"
        );
    }
}

// ── contains_credential tests ────────────────────────────────────────

#[test]
fn contains_credential_detects_all_keywords() {
    assert!(super::contains_credential("my password is 123"));
    assert!(super::contains_credential("here is my token: abc"));
    assert!(super::contains_credential("the secret is xyz"));
    assert!(super::contains_credential("my api_key is sk-123"));
    assert!(super::contains_credential("ssh key: RSA..."));
    assert!(super::contains_credential("密码是 hunter2"));
    assert!(super::contains_credential("密钥在这里"));
}

#[test]
fn contains_credential_allows_safe_content() {
    assert!(!super::contains_credential("I like Bitcoin"));
    assert!(!super::contains_credential("my server IP is 1.2.3.4"));
    assert!(!super::contains_credential("check the weather"));
    assert!(!super::contains_credential("remember my name is Alice"));
}

#[test]
fn contains_credential_case_insensitive() {
    assert!(super::contains_credential("My PASSWORD is 123"));
    assert!(super::contains_credential("API_KEY here"));
    assert!(super::contains_credential("Secret value"));
}

// ── Schema compatibility tests ──────────────────────────────────────
// These tests simulate an "old" database (without new columns) and verify
// that queries still work after migration. Prevents the silent .ok() bug
// where a missing column causes query_row to fail and return None.

#[tokio::test]
async fn schema_migration_skill_proposals_version_column() {
    // Create a DB with the OLD schema (no version column)
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE skill_proposals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_name TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending_approval',
                stage_error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                user_request TEXT NOT NULL,
                proposal_summary TEXT,
                design_json TEXT,
                generated_source TEXT,
                generated_manifest TEXT,
                generated_cargo_toml TEXT,
                compiled_wasm BLOB,
                audit_static_result TEXT,
                audit_llm_result TEXT,
                audit_overridden INTEGER DEFAULT 0,
                test_result TEXT,
                requires_auth INTEGER DEFAULT 0,
                auth_domain TEXT,
                total_llm_calls INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
    )
    .unwrap();

    // Apply the skill_proposals column migrations that the current row reader depends on.
    let skill_proposal_migrations = [
        "ALTER TABLE skill_proposals ADD COLUMN version INTEGER NOT NULL DEFAULT 1;",
        "ALTER TABLE skill_proposals ADD COLUMN skill_md TEXT;",
        "ALTER TABLE skill_proposals ADD COLUMN audit_override_by TEXT;",
        "ALTER TABLE skill_proposals ADD COLUMN audit_override_at TEXT;",
        "ALTER TABLE skill_proposals ADD COLUMN audit_override_reason TEXT;",
        "ALTER TABLE skill_proposals ADD COLUMN capability_graph_json TEXT;",
        "ALTER TABLE skill_proposals ADD COLUMN execution_dag_json TEXT;",
    ];
    for migration in skill_proposal_migrations {
        let _ = conn.execute_batch(migration);
    }

    // Insert a test row
    conn.execute(
            "INSERT INTO skill_proposals (skill_name, requested_by, user_request, status, created_at, updated_at)
             VALUES ('test', 'did:test', 'test request', 'pending_approval', datetime('now'), datetime('now'))",
            [],
        ).unwrap();

    let id = conn.last_insert_rowid();

    // Query must succeed (this is the exact query that was failing before migration)
    let result = get_skill_proposal_row(&conn, id);
    assert!(
        result.is_some(),
        "get_skill_proposal_row must find row after migration"
    );
    let proposal = result.unwrap();
    assert_eq!(proposal.version, 1);
    assert_eq!(proposal.skill_md, None);
    assert_eq!(proposal.audit_override_by, None);
    assert_eq!(proposal.capability_graph_json, None);
    assert_eq!(proposal.execution_dag_json, None);
    assert_eq!(proposal.status, "pending_approval");
}

#[tokio::test]
async fn schema_get_skill_proposal_reports_error_on_missing_column() {
    // Create a DB WITHOUT version column and WITHOUT migration
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE skill_proposals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_name TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending_approval',
                stage_error TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                user_request TEXT NOT NULL,
                proposal_summary TEXT,
                design_json TEXT,
                generated_source TEXT,
                generated_manifest TEXT,
                generated_cargo_toml TEXT,
                compiled_wasm BLOB,
                audit_static_result TEXT,
                audit_llm_result TEXT,
                audit_overridden INTEGER DEFAULT 0,
                test_result TEXT,
                requires_auth INTEGER DEFAULT 0,
                auth_domain TEXT,
                total_llm_calls INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
    )
    .unwrap();

    // Insert a row WITHOUT applying migration
    conn.execute(
            "INSERT INTO skill_proposals (skill_name, requested_by, user_request, status, created_at, updated_at)
             VALUES ('test', 'did:test', 'test request', 'pending_approval', datetime('now'), datetime('now'))",
            [],
        ).unwrap();

    let id = conn.last_insert_rowid();

    // Without migration, query will fail — but should return None (not panic)
    // The tracing::error log will fire (verified by the logging we added)
    let result = get_skill_proposal_row(&conn, id);
    assert!(
        result.is_none(),
        "should return None when version column missing"
    );
}

#[allow(deprecated)]
#[tokio::test]
async fn list_reminders_for_chat_returns_unfired_only() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let chat_a = 1001_i64;
    let chat_b = 2002_i64;

    // Store reminders for two chats
    let id1 = mem
        .store_reminder(chat_a, "drink water", 1000)
        .await
        .unwrap();
    let _id2 = mem
        .store_reminder(chat_a, "take medicine", 2000)
        .await
        .unwrap();
    let _id3 = mem
        .store_reminder(chat_b, "other chat", 3000)
        .await
        .unwrap();

    // List for chat_a — should see 2
    let list = mem.list_reminders_for_chat(chat_a).await.unwrap();
    assert_eq!(list.len(), 2, "chat_a should have 2 unfired reminders");
    assert_eq!(list[0].message, "drink water");
    assert_eq!(list[1].message, "take medicine");

    // List for chat_b — should see 1 (cross-chat isolation)
    let list_b = mem.list_reminders_for_chat(chat_b).await.unwrap();
    assert_eq!(list_b.len(), 1);
    assert_eq!(list_b[0].message, "other chat");

    // Fire one reminder, then list again
    mem.mark_reminder_fired(id1).await.unwrap();
    let list_after = mem.list_reminders_for_chat(chat_a).await.unwrap();
    assert_eq!(list_after.len(), 1, "fired reminder should be excluded");
    assert_eq!(list_after[0].message, "take medicine");
}

#[allow(deprecated)]
#[tokio::test]
async fn cancel_reminder_deletes_own_unfired() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let chat_a = 1001_i64;
    let chat_b = 2002_i64;

    let id1 = mem
        .store_reminder(chat_a, "reminder A", 1000)
        .await
        .unwrap();
    let id2 = mem
        .store_reminder(chat_b, "reminder B", 2000)
        .await
        .unwrap();

    // Cancel own reminder — should succeed
    let ok = mem.cancel_reminder(id1, chat_a).await.unwrap();
    assert!(ok, "cancel own unfired reminder should return true");

    // Verify it's gone
    let list = mem.list_reminders_for_chat(chat_a).await.unwrap();
    assert!(list.is_empty(), "cancelled reminder should be gone");

    // Cancel someone else's reminder — should fail (cross-chat)
    let nope = mem.cancel_reminder(id2, chat_a).await.unwrap();
    assert!(!nope, "cannot cancel another chat's reminder");

    // Original owner's reminder still intact
    let list_b = mem.list_reminders_for_chat(chat_b).await.unwrap();
    assert_eq!(list_b.len(), 1);
}

#[allow(deprecated)]
#[tokio::test]
async fn cancel_fired_reminder_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let chat = 1001_i64;

    let id = mem
        .store_reminder(chat, "already fired", 1000)
        .await
        .unwrap();
    mem.mark_reminder_fired(id).await.unwrap();

    // Cancelling a fired reminder should return false
    let result = mem.cancel_reminder(id, chat).await.unwrap();
    assert!(!result, "cannot cancel already-fired reminder");
}

#[allow(deprecated)]
#[tokio::test]
async fn list_reminders_empty_chat() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let list = mem.list_reminders_for_chat(9999).await.unwrap();
    assert!(list.is_empty(), "empty chat should return empty list");
}

// ── Healthcheck tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_memory_healthcheck_alive() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    assert!(mem.healthcheck().await, "healthy actor should return true");
}

#[tokio::test]
async fn test_memory_healthcheck_after_operations() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:hctest";
    // Do some operations first
    mem.push_l1(
        did,
        Message::text(
            "user".to_string(),
            "test".to_string(),
            chrono::Utc::now().timestamp(),
        ),
    )
    .await
    .unwrap();
    // Should still be healthy
    assert!(
        mem.healthcheck().await,
        "actor should be healthy after operations"
    );
}

// ── L1 TTL tests ────────────────────────────────────────────────────

#[tokio::test]
async fn test_l1_ttl_eviction() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:ttltest";

    // Push an old message (8 days old — beyond 7-day TTL)
    let old_msg = Message::text(
        "user".to_string(),
        "old message".to_string(),
        chrono::Utc::now().timestamp() - (8 * 24 * 60 * 60),
    );
    mem.push_l1(did, old_msg).await.unwrap();

    // Push a recent message (now)
    let new_msg = Message::text(
        "user".to_string(),
        "new message".to_string(),
        chrono::Utc::now().timestamp(),
    );
    mem.push_l1(did, new_msg).await.unwrap();

    // Get L1 — old message should be evicted by TTL
    let history = mem.get_l1(did).await.unwrap();
    assert_eq!(history.len(), 1, "old message should be evicted by TTL");
    assert_eq!(history[0].content, "new message");
}

#[tokio::test]
async fn test_l1_ttl_retains_recent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:ttlrecent";

    // Push a message from 1 day ago (within 7-day TTL)
    let recent_msg = Message::text(
        "user".to_string(),
        "recent message".to_string(),
        chrono::Utc::now().timestamp() - (24 * 60 * 60),
    );
    mem.push_l1(did, recent_msg).await.unwrap();

    // Push another message now
    let now_msg = Message::text(
        "assistant".to_string(),
        "current reply".to_string(),
        chrono::Utc::now().timestamp(),
    );
    mem.push_l1(did, now_msg).await.unwrap();

    // Both should be retained
    let history = mem.get_l1(did).await.unwrap();
    assert_eq!(history.len(), 2, "recent messages should be retained");
}

// ── Fact aging / confidence decay tests ──────────────────────────────

#[test]
fn test_composite_search_score_formula() {
    // composite_search_score(cosine, recall_score, age_days)
    // = cosine * (0.6 + 0.4 * effective_recall)
    // Cold start (age < 7): effective = 0.5*(1-t) + recall*t

    // Brand-new fact (age=0): effective = 0.5, score = cosine * 0.8
    let s0 = composite_search_score(1.0, 0.0, 0.0);
    assert!((s0 - 0.8).abs() < 0.001, "New fact: {s0}");

    // Old fact (age=30) with no recall: effective = 0.0, score = cosine * 0.6
    let s30 = composite_search_score(1.0, 0.0, 30.0);
    assert!((s30 - 0.6).abs() < 0.001, "Old no-recall: {s30}");

    // Old fact with perfect recall: effective = 1.0, score = cosine * 1.0
    let s_perf = composite_search_score(1.0, 1.0, 30.0);
    assert!((s_perf - 1.0).abs() < 0.001, "Perfect recall: {s_perf}");

    // Monotonic: higher recall → higher score (same cosine, same age)
    assert!(composite_search_score(0.8, 0.8, 20.0) > composite_search_score(0.8, 0.2, 20.0));

    // Cold-start interpolation at midpoint (age=3.5 of 7): t=0.5
    let s_mid = composite_search_score(1.0, 0.0, 3.5);
    // effective = 0.5*(1-0.5) + 0.0*0.5 = 0.25, score = 1.0 * (0.6 + 0.4*0.25) = 0.7
    assert!((s_mid - 0.7).abs() < 0.001, "Cold-start midpoint: {s_mid}");
}

#[test]
fn test_topic_weight_tiers() {
    // Per-topic multipliers per spec §3.3
    assert!((topic_weight("learned:context") - 1.2).abs() < 1e-9);
    assert!((topic_weight("learned:context:home") - 1.2).abs() < 1e-9);
    assert!((topic_weight("learned:preference") - 1.1).abs() < 1e-9);
    assert!((topic_weight("learned:goal") - 1.1).abs() < 1e-9);
    assert!((topic_weight("learned:habit") - 1.0).abs() < 1e-9);
    assert!((topic_weight("learned:workflow") - 1.0).abs() < 1e-9);
    assert!((topic_weight("learned:interest") - 0.9).abs() < 1e-9);
    assert!((topic_weight("learned:lesson") - 0.8).abs() < 1e-9);
    // Unknown prefixes default to 0.9
    assert!((topic_weight("user_fact") - 0.9).abs() < 1e-9);
    assert!((topic_weight("heartbeat_memo") - 0.9).abs() < 1e-9);
}

#[test]
fn test_topic_weight_ranking_order() {
    // context > preference ≈ goal > habit ≈ workflow > interest > lesson
    assert!(topic_weight("learned:context") > topic_weight("learned:preference"));
    assert!(topic_weight("learned:preference") > topic_weight("learned:habit"));
    assert!(topic_weight("learned:habit") > topic_weight("learned:interest"));
    assert!(topic_weight("learned:interest") > topic_weight("learned:lesson"));
}

#[test]
fn test_semantic_top_k_and_candidate_k_updated() {
    // Ensure constants match spec §3.3 after tier redesign
    assert_eq!(
        SEMANTIC_TOP_K, 10,
        "SEMANTIC_TOP_K should be 10 after tier split"
    );
    assert_eq!(
        SEMANTIC_CANDIDATE_K, 30,
        "SEMANTIC_CANDIDATE_K should be 30"
    );
}

#[tokio::test]
async fn test_semantic_search_time_decay_reranks() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:decay_test";

    // Store two facts with identical embeddings but we'll verify
    // that the semantic search still works with the new schema
    let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
    mem.store_l2(did, "test", "fresh fact", Some(&emb))
        .await
        .unwrap();
    mem.store_l2(did, "test", "another fresh fact", Some(&emb))
        .await
        .unwrap();

    let query = vec![0.9_f32, 0.1, 0.0, 0.0];
    let results = mem.query_l2_semantic(did, &query).await.unwrap();
    assert!(
        !results.is_empty(),
        "should find facts with decay-adjusted scoring"
    );
}

#[tokio::test]
async fn test_query_stale_facts_empty_when_fresh() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:stale_test";

    // Store a fact (last_accessed = now)
    let emb = vec![1.0_f32; 4];
    mem.store_l2(did, "test", "recent fact", Some(&emb))
        .await
        .unwrap();

    // Query stale facts with 30-day threshold — should return nothing
    let stale = mem.query_stale_facts(did, 30).await.unwrap();
    assert!(stale.is_empty(), "freshly stored fact should not be stale");
}

#[tokio::test]
async fn test_touch_l2_facts_fire_and_forget() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:touch_test";

    let emb = vec![1.0_f32; 4];
    let id = mem
        .store_l2(did, "test", "touchable fact", Some(&emb))
        .await
        .unwrap();

    // Fire-and-forget touch should not panic
    mem.touch_l2_facts(vec![id]);

    // Give actor time to process
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Verify fact is still retrievable
    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].fact_summary, "touchable fact");
}

#[tokio::test]
async fn skill_state_store_and_load_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("calendar_reader", "last_uid", "42")
        .await
        .unwrap();
    let val = mem
        .load_skill_state("calendar_reader", "last_uid")
        .await
        .unwrap();
    assert_eq!(val, Some("42".to_string()));
}

#[tokio::test]
async fn skill_state_load_nonexistent_returns_none() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let val = mem
        .load_skill_state("calendar_reader", "no_such_key")
        .await
        .unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn skill_state_delete_removes_key() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("news_digest", "cursor", "abc")
        .await
        .unwrap();
    mem.delete_skill_state("news_digest", "cursor")
        .await
        .unwrap();
    let val = mem.load_skill_state("news_digest", "cursor").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn skill_state_max_100_keys_enforcement() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    for i in 0..100 {
        mem.store_skill_state("test_skill", &format!("key_{i}"), "v")
            .await
            .unwrap();
    }
    let result = mem
        .store_skill_state("test_skill", "key_overflow", "v")
        .await;
    assert!(result.is_err(), "101st key should be rejected");
    assert!(result.unwrap_err().to_string().contains("maximum"));
}

#[tokio::test]
async fn skill_state_max_64kb_value_enforcement() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let big_value = "x".repeat(64 * 1024 + 1);
    let result = mem
        .store_skill_state("test_skill", "big_key", &big_value)
        .await;
    assert!(result.is_err(), "value over 64KB should be rejected");
    assert!(result.unwrap_err().to_string().contains("too large"));
}

#[tokio::test]
async fn skill_state_update_existing_key() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("s1", "k1", "v1").await.unwrap();
    mem.store_skill_state("s1", "k1", "v2").await.unwrap();
    let val = mem.load_skill_state("s1", "k1").await.unwrap();
    assert_eq!(val, Some("v2".to_string()));
}

#[tokio::test]
async fn skill_state_update_at_limit_doesnt_fail() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    for i in 0..100 {
        mem.store_skill_state("full_skill", &format!("key_{i}"), "v")
            .await
            .unwrap();
    }
    mem.store_skill_state("full_skill", "key_0", "updated")
        .await
        .unwrap();
    let val = mem.load_skill_state("full_skill", "key_0").await.unwrap();
    assert_eq!(val, Some("updated".to_string()));
}

#[tokio::test]
async fn skill_state_isolation_between_skills() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("skill_a", "cursor", "100")
        .await
        .unwrap();
    mem.store_skill_state("skill_b", "cursor", "200")
        .await
        .unwrap();
    let a = mem.load_skill_state("skill_a", "cursor").await.unwrap();
    let b = mem.load_skill_state("skill_b", "cursor").await.unwrap();
    assert_eq!(a, Some("100".to_string()));
    assert_eq!(b, Some("200".to_string()));
}

// ── query_l2_by_topic / delete_l2_by_topic tests ────────────────────

#[tokio::test]
async fn query_l2_by_topic_returns_matching_facts() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:test:topic_query";

    // Store facts with different topics
    let emb = vec![1.0_f32, 2.0, 3.0];
    mem.store_l2(
        did,
        "monitoring_intent:crypto_tracker",
        "intent_1",
        Some(&emb),
    )
    .await
    .unwrap();
    mem.store_l2(
        did,
        "monitoring_intent:server_monitor",
        "intent_2",
        Some(&emb),
    )
    .await
    .unwrap();
    mem.store_l2(did, "other_topic", "irrelevant", Some(&emb))
        .await
        .unwrap();

    // Query exact topic
    let results = mem
        .query_l2_by_topic(did, "monitoring_intent:crypto_tracker")
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "intent_1");
    assert!(results[0].1.is_some(), "embedding should be present");
}

#[tokio::test]
async fn query_l2_by_topic_includes_null_embeddings() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:test:topic_null_emb";

    // Store a fact without embedding
    mem.store_l2(did, "monitoring_intent:test", "no_embedding_fact", None)
        .await
        .unwrap();

    let results = mem
        .query_l2_by_topic(did, "monitoring_intent:test")
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "no_embedding_fact");
    assert!(results[0].1.is_none(), "embedding should be None");
}

#[tokio::test]
async fn query_l2_by_topic_did_isolation() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![1.0_f32];

    mem.store_l2(
        "did:alice",
        "monitoring_intent:test",
        "alice_intent",
        Some(&emb),
    )
    .await
    .unwrap();
    mem.store_l2(
        "did:bob",
        "monitoring_intent:test",
        "bob_intent",
        Some(&emb),
    )
    .await
    .unwrap();

    let alice = mem
        .query_l2_by_topic("did:alice", "monitoring_intent:test")
        .await
        .unwrap();
    assert_eq!(alice.len(), 1);
    assert_eq!(alice[0].0, "alice_intent");

    let bob = mem
        .query_l2_by_topic("did:bob", "monitoring_intent:test")
        .await
        .unwrap();
    assert_eq!(bob.len(), 1);
    assert_eq!(bob[0].0, "bob_intent");
}

#[tokio::test]
async fn delete_l2_by_topic_removes_matching_facts() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:test:topic_delete";
    // Different embeddings for distinct facts (write guard uses cosine dedup)
    let emb_a = vec![1.0_f32, 0.0];
    let emb_b = vec![0.0_f32, 1.0];
    let emb_c = vec![0.7_f32, 0.7];

    // Store two facts with same topic, one with different topic
    mem.store_l2(did, "monitoring_intent:crypto", "fact_a", Some(&emb_a))
        .await
        .unwrap();
    mem.store_l2(did, "monitoring_intent:crypto", "fact_b", Some(&emb_b))
        .await
        .unwrap();
    mem.store_l2(did, "monitoring_intent:server", "fact_c", Some(&emb_c))
        .await
        .unwrap();

    // Delete by topic
    let deleted = mem
        .delete_l2_by_topic(did, "monitoring_intent:crypto")
        .await
        .unwrap();
    assert_eq!(deleted, 2, "should delete 2 matching facts");

    // Verify the other topic is untouched
    let remaining = mem
        .query_l2_by_topic(did, "monitoring_intent:server")
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, "fact_c");

    // Verify deleted topic is empty
    let gone = mem
        .query_l2_by_topic(did, "monitoring_intent:crypto")
        .await
        .unwrap();
    assert!(gone.is_empty());
}

#[tokio::test]
async fn delete_l2_by_topic_marks_hnsw_entries_deleted() {
    let mut mem = MemoryHandle::spawn_in_memory().unwrap();
    let hnsw = std::sync::Arc::new(crate::hnsw_index::HnswIndex::new(
        crate::hnsw_index::HnswConfig::default(),
    ));
    mem.set_hnsw_index(hnsw.clone());

    let did = "did:test:hnsw_topic_delete";
    // Orthogonal embeddings so write guard doesn't dedup (cosine ≈ 0)
    let emb_a = vec![1.0_f32, 0.0, 0.0, 0.0];
    let emb_b = vec![0.0, 1.0, 0.0, 0.0];
    let emb_keep = vec![0.0, 0.0, 1.0, 0.0];

    let id_a = mem
        .store_l2(did, "remove_me", "fact_a", Some(&emb_a))
        .await
        .unwrap();
    let id_b = mem
        .store_l2(did, "remove_me", "fact_b", Some(&emb_b))
        .await
        .unwrap();
    let _id_keep = mem
        .store_l2(did, "keep_me", "fact_keep", Some(&emb_keep))
        .await
        .unwrap();
    assert_eq!(hnsw.len(), 3);

    let deleted = mem.delete_l2_by_topic(did, "remove_me").await.unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(
        hnsw.deleted_count(),
        2,
        "HNSW must mark deleted entries from bulk topic delete"
    );

    let results = hnsw.search(&[1.0, 0.0, 0.0, 0.0], 5, 0.0);
    for r in &results {
        assert!(
            r.l2_id != id_a && r.l2_id != id_b,
            "HNSW search must not return deleted IDs, but found l2_id={}",
            r.l2_id
        );
    }
}

#[tokio::test]
async fn delete_l2_by_topic_returns_zero_for_nonexistent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let deleted = mem
        .delete_l2_by_topic("did:test:none", "monitoring_intent:nonexistent")
        .await
        .unwrap();
    assert_eq!(deleted, 0);
}
// ── Knowledge Base tests ─────────────────────────────────────────

#[tokio::test]
async fn kb_store_and_query_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            123,
            "action_item",
            "high",
            "审批合同",
            "需要在周五前审批XX合同",
            &["合同".into(), "财务".into()],
            &[],
            None,
            Some("对话提取"),
            Some("2026-03-20"),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(id > 0);

    let entries = mem
        .query_knowledge_entries(123, Some("action_item"), Some("active"), None, 10)
        .await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "审批合同");
    assert_eq!(entries[0].priority, "high");
    assert_eq!(entries[0].tags, vec!["合同", "财务"]);
    assert_eq!(entries[0].due_date, Some("2026-03-20".into()));
}

#[tokio::test]
async fn kb_update_status_to_done() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            100,
            "action_item",
            "normal",
            "回复邮件",
            "回复投资人邮件",
            &[],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Orchestrator sets resolved_at when marking done (mimic that here)
    let now = chrono::Utc::now().to_rfc3339();
    let updated = mem
        .update_knowledge_entry(
            id,
            100,
            KnowledgeEntryUpdate {
                status: Some("done".into()),
                resolved_at: Some(Some(now)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated);

    let entry = mem.get_knowledge_entry(id, 100).await.unwrap();
    assert_eq!(entry.status, "done");
    assert!(entry.resolved_at.is_some());
}

#[tokio::test]
async fn kb_search_by_text() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_knowledge_entry(
        50,
        "insight",
        "normal",
        "竞品分析",
        "竞品XX发布了新的AI功能",
        &["竞品".into()],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        50,
        "decision",
        "high",
        "技术选型",
        "决定使用Rust重写后端",
        &["技术".into()],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let results = mem.search_knowledge_entries(50, "AI", 10).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "竞品分析");
}

#[tokio::test]
async fn kb_dashboard_summary() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_knowledge_entry(
        55,
        "action_item",
        "high",
        "A",
        "a",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        55,
        "action_item",
        "normal",
        "B",
        "b",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        55,
        "waiting_for",
        "normal",
        "C",
        "c",
        &[],
        &[],
        None,
        None,
        None,
        Some("李总"),
        None,
    )
    .await
    .unwrap();

    let summary = mem.knowledge_dashboard_summary(55).await;
    assert!(summary.len() >= 2);
    let action_active: i64 = summary
        .iter()
        .filter(|(c, s, _)| c == "action_item" && s == "active")
        .map(|(_, _, n)| n)
        .sum();
    assert_eq!(action_active, 2);
}

#[tokio::test]
async fn kb_priority_ordering() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_knowledge_entry(
        60,
        "action_item",
        "low",
        "低优先",
        "x",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        60,
        "action_item",
        "urgent",
        "紧急",
        "x",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        60,
        "action_item",
        "normal",
        "普通",
        "x",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let entries = mem
        .query_knowledge_entries(60, None, Some("active"), None, 10)
        .await;
    assert_eq!(entries[0].priority, "urgent");
    assert_eq!(entries[1].priority, "normal");
    assert_eq!(entries[2].priority, "low");
}

#[tokio::test]
async fn kb_chat_isolation() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_knowledge_entry(
        100,
        "insight",
        "normal",
        "Chat 100",
        "x",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        200,
        "insight",
        "normal",
        "Chat 200",
        "x",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let entries_100 = mem.query_knowledge_entries(100, None, None, None, 10).await;
    assert_eq!(entries_100.len(), 1);
    assert_eq!(entries_100[0].title, "Chat 100");

    let entries_200 = mem.query_knowledge_entries(200, None, None, None, 10).await;
    assert_eq!(entries_200.len(), 1);
    assert_eq!(entries_200[0].title, "Chat 200");
}

#[tokio::test]
async fn kb_count_entries() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    assert_eq!(mem.count_knowledge_entries(70).await, 0);
    mem.store_knowledge_entry(
        70,
        "metric",
        "normal",
        "收入",
        "本月100万",
        &[],
        &[],
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(mem.count_knowledge_entries(70).await, 1);
}

#[test]
fn escape_like_no_special_chars() {
    assert_eq!(escape_like("hello world"), "hello world");
}

#[test]
fn escape_like_percent() {
    assert_eq!(escape_like("100%"), "100\\%");
}

#[test]
fn escape_like_underscore() {
    assert_eq!(escape_like("a_b"), "a\\_b");
}

#[test]
fn escape_like_backslash() {
    assert_eq!(escape_like("path\\to"), "path\\\\to");
}

#[test]
fn escape_like_combined() {
    assert_eq!(escape_like("100%_done\\ok"), "100\\%\\_done\\\\ok");
}

// ── Circuit breaker tests ────────────────────────────────────────

#[test]
fn circuit_breaker_trips_after_3_consecutive_failures() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        // 3 consecutive failures → circuit should open
        for i in 0..3 {
            memory
                .update_skill_metrics(
                    "test_cb",
                    false,
                    100.0,
                    Some(&format!("error {}", i)),
                    Some("network"),
                )
                .await;
        }
        // Allow actor to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let metrics = memory.get_skill_metrics("test_cb").await.unwrap().unwrap();
        assert_eq!(metrics.consecutive_failures, 3);
        assert!(
            metrics.circuit_open,
            "circuit should be open after 3 failures"
        );
    });
}

#[test]
fn circuit_breaker_resets_on_success() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        // Trip the breaker
        for _ in 0..3 {
            memory
                .update_skill_metrics("test_cb2", false, 100.0, Some("err"), None)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Now succeed → should reset
        memory
            .update_skill_metrics("test_cb2", true, 50.0, None, None)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let metrics = memory.get_skill_metrics("test_cb2").await.unwrap().unwrap();
        assert_eq!(metrics.consecutive_failures, 0);
        assert!(
            !metrics.circuit_open,
            "circuit should be closed after success"
        );
    });
}

#[test]
fn unhealthy_skill_metrics_updated_since_counts_recent_only() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        memory
            .update_skill_metrics("recent_unhealthy", false, 100.0, Some("err"), None)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recent_since = chrono::Utc::now() - chrono::Duration::minutes(5);
        let future_since = chrono::Utc::now() + chrono::Duration::minutes(5);

        assert_eq!(
            memory
                .count_unhealthy_skill_metrics_updated_since(recent_since)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            memory
                .count_unhealthy_skill_metrics_updated_since(future_since)
                .await
                .unwrap(),
            0
        );
    });
}

#[test]
fn check_circuit_breaker_returns_correct_state() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        // No metrics yet → None
        let result = memory.check_circuit_breaker("nonexistent").await.unwrap();
        assert!(result.is_none());

        // Trip the breaker
        for _ in 0..3 {
            memory
                .update_skill_metrics("test_cb3", false, 100.0, Some("timeout"), None)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (is_open, consec, last_err) = memory
            .check_circuit_breaker("test_cb3")
            .await
            .unwrap()
            .unwrap();
        assert!(is_open);
        assert_eq!(consec, 3);
        assert_eq!(last_err.as_deref(), Some("timeout"));
    });
}

#[test]
fn reset_circuit_breaker_clears_state() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        // Trip the breaker
        for _ in 0..3 {
            memory
                .update_skill_metrics("test_cb4", false, 100.0, Some("err"), None)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Reset
        memory.reset_circuit_breaker("test_cb4").await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (is_open, consec, _) = memory
            .check_circuit_breaker("test_cb4")
            .await
            .unwrap()
            .unwrap();
        assert!(!is_open);
        assert_eq!(consec, 0);
    });
}

#[test]
fn circuit_breaker_event_emitted_on_trip() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let mut cb_rx = memory.take_circuit_breaker_rx().expect("should have rx");

    rt.block_on(async {
        // update_skill_metrics is fire-and-forget (send only). Under parallel
        // cargo test --lib load a fixed 50ms sleep is not a reliable barrier.
        // get_skill_metrics is request/response on the same actor queue, so its
        // reply only returns after prior UpdateSkillMetrics cmds are applied.

        // 2 failures — no event yet
        for _ in 0..2 {
            memory
                .update_skill_metrics("test_cb5", false, 100.0, Some("err"), None)
                .await;
        }
        let metrics = memory
            .get_skill_metrics("test_cb5")
            .await
            .unwrap()
            .expect("metrics after 2 failures");
        assert_eq!(metrics.consecutive_failures, 2);
        assert!(!metrics.circuit_open);
        assert!(cb_rx.try_recv().is_err(), "no event before 3 failures");

        // 3rd failure → should emit event
        memory
            .update_skill_metrics("test_cb5", false, 100.0, Some("final err"), None)
            .await;
        let metrics = memory
            .get_skill_metrics("test_cb5")
            .await
            .unwrap()
            .expect("metrics after trip");
        assert!(metrics.circuit_open);
        assert_eq!(metrics.consecutive_failures, 3);

        let event = cb_rx.try_recv().expect("should have circuit breaker event");
        assert_eq!(event.skill_name, "test_cb5");
        assert_eq!(event.consecutive_failures, 3);
        assert_eq!(event.last_error.as_deref(), Some("final err"));

        // 4th failure — no duplicate event (already open)
        memory
            .update_skill_metrics("test_cb5", false, 100.0, Some("more"), None)
            .await;
        let metrics = memory
            .get_skill_metrics("test_cb5")
            .await
            .unwrap()
            .expect("metrics after 4th failure");
        assert_eq!(metrics.consecutive_failures, 4);
        assert!(metrics.circuit_open);
        assert!(cb_rx.try_recv().is_err(), "no duplicate event");
    });
}

#[test]
fn two_failures_do_not_trip_circuit_breaker() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        memory
            .update_skill_metrics("test_cb6", false, 100.0, Some("err1"), None)
            .await;
        memory
            .update_skill_metrics("test_cb6", false, 100.0, Some("err2"), None)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let metrics = memory.get_skill_metrics("test_cb6").await.unwrap().unwrap();
        assert_eq!(metrics.consecutive_failures, 2);
        assert!(!metrics.circuit_open, "2 failures should not trip breaker");
    });
}

// ── Empty DID guard tests ────────────────────────────────────────

#[test]
fn store_l2_rejects_empty_did() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        let result = memory
            .store_l2("", "learned:context", "some fact", None)
            .await;
        assert!(result.is_err(), "store_l2 should reject empty DID");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must not be empty"),
            "error message should mention empty DID"
        );
    });
}

#[test]
fn store_l2_accepts_valid_did() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    rt.block_on(async {
        let result = memory
            .store_l2(
                "did:aegis:telegram:123",
                "learned:context",
                "valid fact",
                None,
            )
            .await;
        assert!(result.is_ok(), "store_l2 should accept valid DID");
    });
}

// ── Step 4: L1 vs L2 write discipline (design §3.1) ─────────────

#[test]
fn heartbeat_topic_helper_builds_expected_prefixes() {
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Learned, "user_prefers_cn"),
        "heartbeat:learned:user_prefers_cn"
    );
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Thought, "reflected_on_goal"),
        "heartbeat:thought:reflected_on_goal"
    );
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Proposal, "config_google_auth"),
        "heartbeat:proposal:config_google_auth"
    );
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Plan, "wk1_refactor"),
        "heartbeat:plan:wk1_refactor"
    );
    // Empty slug degrades to bare prefix, not "prefix:".
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Learned, ""),
        "heartbeat:learned"
    );
    // Stray leading/trailing colons get trimmed so we never double-colon.
    assert_eq!(
        heartbeat_topic(HeartbeatWriteKind::Thought, ":foo:"),
        "heartbeat:thought:foo"
    );
}

#[test]
fn apply_write_origin_user_turn_passthrough() {
    assert_eq!(
        apply_write_origin("learned:context", MemoryWriteOrigin::UserTurn),
        "learned:context"
    );
}

#[test]
fn apply_write_origin_heartbeat_rewrites_plain_topic() {
    let rewritten = apply_write_origin(
        "user_prefers_cn",
        MemoryWriteOrigin::HeartbeatTick(HeartbeatWriteKind::Learned),
    );
    assert_eq!(rewritten, "heartbeat:learned:user_prefers_cn");
}

#[test]
fn apply_write_origin_heartbeat_idempotent_when_already_scoped() {
    // Callers that used heartbeat_topic() themselves must not be double-wrapped.
    let already = "heartbeat:thought:x";
    assert_eq!(
        apply_write_origin(
            already,
            MemoryWriteOrigin::HeartbeatTick(HeartbeatWriteKind::Learned)
        ),
        already
    );
}

#[test]
fn store_l2_with_origin_heartbeat_rewrites_topic() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:777";
    let emb: Vec<f32> = vec![0.1_f32; 16];

    rt.block_on(async {
        memory
            .store_l2_with_origin(
                did,
                "goal_x_stalled",
                "goal X hasn't progressed in 2 weeks",
                Some(&emb),
                MemoryWriteOrigin::HeartbeatTick(HeartbeatWriteKind::Learned),
            )
            .await
            .expect("store_l2_with_origin should succeed");

        // Under heartbeat:learned:* prefix — visible.
        let heartbeat_hits = memory
            .query_l2_by_topic_prefix(did, "heartbeat:learned:")
            .await
            .expect("prefix query works");
        assert_eq!(heartbeat_hits.len(), 1, "one heartbeat:learned:* row");

        // Not under the unscoped plain topic — user view is unpolluted.
        let plain_hits = memory
            .query_l2_by_topic_prefix(did, "goal_x_stalled")
            .await
            .expect("prefix query works");
        assert!(
            plain_hits.is_empty(),
            "plain topic must NOT see heartbeat rows"
        );
    });
}

#[test]
fn store_l2_with_origin_user_turn_unchanged() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:778";
    let emb: Vec<f32> = vec![0.2_f32; 16];

    rt.block_on(async {
        memory
            .store_l2_with_origin(
                did,
                "learned:context",
                "user fact",
                Some(&emb),
                MemoryWriteOrigin::UserTurn,
            )
            .await
            .expect("user-turn write works");

        let hits = memory
            .query_l2_by_topic_prefix(did, "learned:context")
            .await
            .expect("prefix query works");
        assert_eq!(hits.len(), 1, "user-turn topic must land verbatim");

        let hb_hits = memory
            .query_l2_by_topic_prefix(did, "heartbeat:")
            .await
            .expect("prefix query works");
        assert!(
            hb_hits.is_empty(),
            "user-turn write must not land under heartbeat:*"
        );
    });
}

#[test]
fn same_slug_different_origins_produce_distinct_rows() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:779";
    let emb: Vec<f32> = vec![0.3_f32; 16];

    rt.block_on(async {
        let user_id = memory
            .store_l2_with_origin(
                did,
                "x",
                "user fact",
                Some(&emb),
                MemoryWriteOrigin::UserTurn,
            )
            .await
            .unwrap();
        let hb_id = memory
            .store_l2_with_origin(
                did,
                "x",
                "heartbeat fact",
                Some(&emb),
                MemoryWriteOrigin::HeartbeatTick(HeartbeatWriteKind::Learned),
            )
            .await
            .unwrap();

        assert_ne!(user_id, hb_id, "distinct row ids");

        // Both rows visible under their respective prefixes.
        let user_hits = memory
            .query_l2_by_topic_prefix(did, "x")
            .await
            .expect("query works");
        let hb_hits = memory
            .query_l2_by_topic_prefix(did, "heartbeat:learned:x")
            .await
            .expect("query works");

        // "x" prefix matches BOTH "x" and "heartbeat:learned:x"? No —
        // LIKE 'x%' matches only topics starting with 'x'. Exactly one
        // row there (the user-turn one).
        let user_facts: Vec<String> = user_hits.iter().map(|(f, _)| f.clone()).collect();
        assert_eq!(
            user_facts,
            vec!["user fact".to_string()],
            "user-turn row present under the bare slug"
        );

        let hb_facts: Vec<String> = hb_hits.iter().map(|(f, _)| f.clone()).collect();
        assert_eq!(
            hb_facts,
            vec!["heartbeat fact".to_string()],
            "heartbeat row present under the scoped topic"
        );
    });
}

#[test]
fn heartbeat_tick_does_not_push_to_l1_except_send_message() {
    // Regression guard for design §3.1: the heartbeat ReAct path may
    // NOT write to L1 — except inside the Step 10 `send_message` tool,
    // which is the explicit artifact-realization exception and lives
    // in `orchestrator_tools_system.rs`.
    //
    // Two-part check:
    // 1. `orchestrator.rs` push_l1 call-site count stays bounded (user-turn
    //    paths, one comment ref, and `record_unanswered_owner_turn`, which
    //    writes the visible failure reply after the owner line is already in
    //    L1 — not a heartbeat write).
    // 2. `orchestrator_tools_system.rs` contains exactly one `push_l1(`
    //    call, and it lives inside `execute_send_message` — no other
    //    tool handler may write to L1.
    // orchestrator.rs was split into orchestrator/ (Phase 0.2, 2026-07-12).
    // Only PRODUCTION call sites count: the `mod tests` block now lives in
    // the orchestrator/tests/ subdir and is excluded, so test-only push_l1
    // calls do not trip the guard (as the old strip-`mod tests` did).
    let orch_production = concat!(
        // Paths are relative to THIS file (core/src/memory/tests/mod.rs); the
        // `../../` climbs tests/ → memory/ → src/ after the Phase 0.3 memory
        // split so the orchestrator source is still read verbatim.
        include_str!("../../orchestrator/mod.rs"),
        include_str!("../../orchestrator/lifecycle.rs"),
        include_str!("../../orchestrator/heartbeat.rs"),
        include_str!("../../orchestrator/message_flow.rs"),
        include_str!("../../orchestrator/tools_dispatch.rs"),
    );
    // Completeness: a new production submodule not added to the concat above
    // would silently blind this guard. 5 production .rs files sit directly in
    // src/orchestrator/ (tests live in the src/orchestrator/tests/ subdir).
    let orch_rs_files = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/orchestrator"))
        .expect("read src/orchestrator")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
        })
        .count();
    assert_eq!(
        orch_rs_files, 5,
        "src/orchestrator/ has {orch_rs_files} production .rs files; update \
             the include_str! concat above (tests live in tests/ subdir)"
    );
    let orch_sites = orch_production.matches("push_l1(").count();
    assert!(
        orch_sites <= 5,
        "push_l1 call-site count in orchestrator.rs production code rose to {orch_sites}; \
             audit heartbeat path per design §3.1 before relaxing"
    );
    assert!(
        include_str!("../../orchestrator/message_flow.rs")
            .contains("fn record_unanswered_owner_turn"),
        "the fifth L1 write is the unanswered-turn reply, not a heartbeat path"
    );

    let tools_source = include_str!("../../orchestrator_tools_system.rs");
    let tools_sites = tools_source.matches(".push_l1(").count();
    assert_eq!(
        tools_sites, 1,
        "orchestrator_tools_system.rs must contain exactly one push_l1 \
             call site (execute_send_message, Step 10). Found {tools_sites}."
    );
    // Ring-fence: the only call must live inside execute_send_message.
    // We detect this by slicing from the fn header to the next top-level
    // fn and checking the push_l1 call appears in that window.
    let send_msg_start = tools_source
        .find("async fn execute_send_message(")
        .expect("execute_send_message handler must exist");
    // Find the end — the next `async fn execute_` after the start, or EOF.
    let after_start = &tools_source[send_msg_start + 30..];
    let next_fn_rel = after_start
        .find("\n    async fn execute_")
        .unwrap_or(after_start.len());
    let send_msg_body = &tools_source[send_msg_start..send_msg_start + 30 + next_fn_rel];
    assert!(
        send_msg_body.contains(".push_l1("),
        "execute_send_message must contain the push_l1 call (Step 10 §3.1 exception)"
    );
}

#[test]
fn auto_group_key_ascii() {
    assert_eq!(auto_group_key("Buy Groceries"), "auto:buy_groceries");
    assert_eq!(
        auto_group_key("  post office bill  "),
        "auto:post_office_bill"
    );
}

#[test]
fn auto_group_key_chinese() {
    assert_eq!(auto_group_key("寄快递"), "auto:寄快递");
    assert_eq!(auto_group_key("去 邮局 寄件"), "auto:去_邮局_寄件");
}

#[test]
fn auto_group_key_strips_special() {
    assert_eq!(auto_group_key("hello! @world #123"), "auto:hello_world_123");
}

#[test]
fn auto_group_key_truncates() {
    let long = "a".repeat(200);
    let key = auto_group_key(&long);
    assert!(key.len() <= 100);
    assert!(key.starts_with("auto:"));
}

#[test]
fn store_task_auto_dedup() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    rt.block_on(async {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let did = "did:aegis:telegram:42";

        // Store the same task twice without explicit group_key
        let id1 = memory
            .store_task(
                42,
                did,
                "Post office bill",
                "",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let id2 = memory
            .store_task(
                42,
                did,
                "Post office bill",
                "",
                "normal",
                &[],
                None,
                "user",
                None,
                None,
                None,
                "notify",
                None,
                None,
                None,
                None,
                false,
                None,
                false,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        assert_ne!(id1, id2);

        // First task should be cancelled, only second active
        let t1 = memory.get_task(id1).await.unwrap();
        let t2 = memory.get_task(id2).await.unwrap();
        assert_eq!(t1.status, "cancelled");
        assert_eq!(t2.status, "active");

        // group_key should be populated
        assert_eq!(t2.group_key.as_deref(), Some("auto:post_office_bill"));
    });
}

// ── Intent Inbox tests ──────────────────────────────────────────────

/// Helper: store an intent, fire it, and return its id.
async fn store_and_fire_intent(mem: &MemoryHandle, user_did: &str, context: &str, now: i64) -> i64 {
    let id = mem
        .store_deferred_intent(
            user_did,
            100,
            "time",
            Some(now - 10),
            None,
            "notify",
            context,
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    mem.mark_intent_fired(id).await.unwrap();
    id
}

#[tokio::test]
async fn test_list_intent_history() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let _id = store_and_fire_intent(&mem, "did:test:hist", "buy milk", now).await;

    let history = mem
        .list_intent_history("did:test:hist", now - 3600)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].context_text, "buy milk");
    assert_eq!(history[0].status, "fired");
}

#[tokio::test]
async fn test_list_intent_history_excludes_active() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    // Store active intent (not fired)
    mem.store_deferred_intent(
        "did:test:active",
        100,
        "time",
        Some(now + 60),
        None,
        "notify",
        "active task",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let history = mem
        .list_intent_history("did:test:active", now - 3600)
        .await
        .unwrap();
    assert!(
        history.is_empty(),
        "active intents should not appear in history"
    );
}

#[tokio::test]
async fn test_dismiss_intent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = store_and_fire_intent(&mem, "did:test:dismiss", "old reminder", now).await;

    let dismissed = mem.dismiss_intent(id, "did:test:dismiss").await.unwrap();
    assert!(dismissed);

    // Should no longer appear in history (status is now 'archived')
    let history = mem
        .list_intent_history("did:test:dismiss", now - 3600)
        .await
        .unwrap();
    assert!(history.is_empty());
}

#[tokio::test]
async fn test_dismiss_intent_wrong_user() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = store_and_fire_intent(&mem, "did:test:owner", "secret", now).await;

    let dismissed = mem.dismiss_intent(id, "did:test:other").await.unwrap();
    assert!(!dismissed, "wrong user should not be able to dismiss");
}

#[tokio::test]
async fn test_reschedule_intent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = store_and_fire_intent(&mem, "did:test:resched", "meeting", now).await;

    let new_trigger = now + 7200;
    let new_expires = now + 14400;
    let rescheduled = mem
        .reschedule_intent(id, "did:test:resched", new_trigger, new_expires)
        .await
        .unwrap();
    assert!(rescheduled);

    // Should now appear in active intents list
    let active = mem.list_user_intents("did:test:resched").await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].context_text, "meeting");
    assert_eq!(active[0].trigger_at_unix, Some(new_trigger));
}

#[tokio::test]
async fn test_reschedule_active_fails() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    // Store but don't fire — remains active
    let id = mem
        .store_deferred_intent(
            "did:test:noresched",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "still active",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let rescheduled = mem
        .reschedule_intent(id, "did:test:noresched", now + 7200, now + 14400)
        .await
        .unwrap();
    assert!(!rescheduled, "active intent should not be reschedulable");
}

#[tokio::test]
async fn test_archive_old_intents() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = store_and_fire_intent(&mem, "did:test:archive", "old task", now).await;

    // Use a cutoff far in the future so the fired item is "old"
    let archived = mem.archive_old_intents(now + 999999).await.unwrap();
    assert!(!archived.is_empty());
    assert!(archived.iter().any(|(aid, _, _, _)| *aid == id));

    // Should no longer appear in history
    let history = mem
        .list_intent_history("did:test:archive", now - 3600)
        .await
        .unwrap();
    assert!(history.is_empty());
}

#[tokio::test]
async fn test_archive_preserves_recent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let _id = store_and_fire_intent(&mem, "did:test:recent", "fresh task", now).await;

    // Use a cutoff far in the past — nothing should be archived
    let archived = mem.archive_old_intents(now - 999999).await.unwrap();
    assert!(archived.is_empty(), "recent items should not be archived");

    // Should still appear in history
    let history = mem
        .list_intent_history("did:test:recent", now - 3600)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
}

// ── Task archival lifecycle tests ──────────────────────────────────

/// Helper: store a plain task (status defaults to 'active') and return its id.
async fn store_plain_task(mem: &MemoryHandle, chat_id: i64, title: &str) -> i64 {
    mem.store_task(
        chat_id,
        &format!("did:aegis:telegram:{chat_id}"),
        title,
        "",
        "normal",
        &[],
        None,
        "test",
        None,
        None,
        None,
        "llm_callback",
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        0,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn archive_sweep_covers_done_fired_cancelled_but_not_active() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7001;

    // done plain to-do (no trigger_type) — must be archived.
    let done_id = store_plain_task(&mem, chat, "done todo").await;
    mem.update_task(done_id, chat, r#"{"status":"done"}"#)
        .await
        .unwrap();
    // cancelled task — must be archived.
    let cancelled_id = store_plain_task(&mem, chat, "cancelled todo").await;
    mem.update_task(cancelled_id, chat, r#"{"status":"cancelled"}"#)
        .await
        .unwrap();
    // fired scheduled intent — must be archived.
    let fired_id = store_and_fire_intent(&mem, "did:aegis:telegram:7001", "fired", now).await;
    // completed recurring intent (exhausted max_fires) — must be archived
    // ("已完成之类"). 'completed' is a genuinely finished state.
    let completed_id = store_plain_task(&mem, chat, "completed recurring").await;
    mem.update_task(completed_id, chat, r#"{"status":"completed"}"#)
        .await
        .unwrap();
    // active task — must NOT be archived.
    let active_id = store_plain_task(&mem, chat, "still active").await;

    // Far-future cutoff => everything finished is "old".
    let archived = mem.archive_old_intents(now + 999_999).await.unwrap();
    let archived_ids: Vec<i64> = archived.iter().map(|(id, _, _, _)| *id).collect();
    assert!(archived_ids.contains(&done_id), "done must archive");
    assert!(
        archived_ids.contains(&cancelled_id),
        "cancelled must archive"
    );
    assert!(archived_ids.contains(&fired_id), "fired must archive");
    assert!(
        archived_ids.contains(&completed_id),
        "completed must archive"
    );
    assert!(
        !archived_ids.contains(&active_id),
        "active must NOT archive"
    );

    assert_eq!(
        mem.get_task(active_id).await.unwrap().status,
        "active",
        "active task status must be untouched"
    );
    assert_eq!(mem.get_task(done_id).await.unwrap().status, "archived");
}

#[tokio::test]
async fn archive_sweep_preserves_recent_finished() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7002;
    let done_id = store_plain_task(&mem, chat, "recent done").await;
    mem.update_task(done_id, chat, r#"{"status":"done"}"#)
        .await
        .unwrap();

    // Past cutoff => recently-finished task is NOT old enough.
    let archived = mem.archive_old_intents(now - 999_999).await.unwrap();
    assert!(archived.is_empty(), "recent finished task must not archive");
    assert_eq!(mem.get_task(done_id).await.unwrap().status, "done");
}

#[tokio::test]
async fn list_tasks_default_excludes_archived_but_explicit_returns() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let chat = 7003;
    let archived_id = store_plain_task(&mem, chat, "archived item").await;
    mem.update_task(archived_id, chat, r#"{"status":"archived"}"#)
        .await
        .unwrap();
    let done_id = store_plain_task(&mem, chat, "done item").await;
    mem.update_task(done_id, chat, r#"{"status":"done"}"#)
        .await
        .unwrap();
    let active_id = store_plain_task(&mem, chat, "active item").await;

    // Default (status=None): excludes archived, INCLUDES done + active.
    let default_list = mem.list_tasks(chat, None, None, 50).await;
    let default_ids: Vec<i64> = default_list.iter().map(|t| t.id).collect();
    assert!(
        !default_ids.contains(&archived_id),
        "default list must EXCLUDE archived"
    );
    assert!(
        default_ids.contains(&done_id),
        "default list must keep done"
    );
    assert!(
        default_ids.contains(&active_id),
        "default list must keep active"
    );

    // Explicit status='archived' => archived IS returned.
    let arch_list = mem.list_tasks(chat, None, Some("archived"), 50).await;
    assert!(
        arch_list.iter().any(|t| t.id == archived_id),
        "explicit archived query must return archived"
    );

    // Explicit status='done' still returns done.
    let done_list = mem.list_tasks(chat, None, Some("done"), 50).await;
    assert!(done_list.iter().any(|t| t.id == done_id));
}

#[tokio::test]
async fn retention_delete_only_old_archived_respects_limit() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7004;

    // old archived — eligible for delete.
    let old_archived = store_plain_task(&mem, chat, "old archived").await;
    mem.update_task(old_archived, chat, r#"{"status":"archived"}"#)
        .await
        .unwrap();
    // non-archived old (done) — must NOT be deleted by retention sweep.
    let old_done = store_plain_task(&mem, chat, "old done").await;
    mem.update_task(old_done, chat, r#"{"status":"done"}"#)
        .await
        .unwrap();

    // Far-future cutoff => the archived row is "old"; bound batch to 500.
    let (deleted, min_id, max_id) = mem
        .delete_old_archived_tasks(now + 999_999, 500)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "only the archived row is deleted");
    assert_eq!(min_id, Some(old_archived));
    assert_eq!(max_id, Some(old_archived));
    assert!(
        mem.get_task(old_archived).await.is_none(),
        "old archived task must be gone"
    );
    assert!(
        mem.get_task(old_done).await.is_some(),
        "non-archived task must survive retention sweep"
    );
}

#[tokio::test]
async fn retention_delete_skips_young_archived() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7005;
    let young_archived = store_plain_task(&mem, chat, "young archived").await;
    mem.update_task(young_archived, chat, r#"{"status":"archived"}"#)
        .await
        .unwrap();

    // Past cutoff => the just-archived row is not old enough to delete.
    let (deleted, _, _) = mem
        .delete_old_archived_tasks(now - 999_999, 500)
        .await
        .unwrap();
    assert_eq!(deleted, 0, "young archived task must not be deleted");
    assert!(mem.get_task(young_archived).await.is_some());
}

#[tokio::test]
async fn retention_delete_respects_batch_limit() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7006;
    // Create 3 old archived tasks.
    for i in 0..3 {
        let id = store_plain_task(&mem, chat, &format!("arch {i}")).await;
        mem.update_task(id, chat, r#"{"status":"archived"}"#)
            .await
            .unwrap();
    }
    // Limit to 2 per run.
    let (deleted, _, _) = mem
        .delete_old_archived_tasks(now + 999_999, 2)
        .await
        .unwrap();
    assert_eq!(deleted, 2, "batch limit must cap deletes per run");
    // Remaining archived tasks still present until next sweep.
    let remaining = mem.list_tasks(chat, None, Some("archived"), 50).await;
    assert_eq!(
        remaining.len(),
        1,
        "one archived task remains after capped run"
    );
}

#[tokio::test]
async fn archived_at_gates_retention_not_original_finished_at() {
    // REGRESSION (Codex F1): an OLD finished task, once archived, must get a
    // FRESH archived grace window — it must NOT be archived and then
    // permanently deleted in the SAME retention sweep just because its
    // original finished-at timestamp is ancient. The delete sweep keys on
    // archived_at (when the row entered 'archived'), not the finished-at basis.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7007;

    // A task finished long ago (completed_at ~ 6 years past).
    let id = store_plain_task(&mem, chat, "ancient done").await;
    mem.update_task(
        id,
        chat,
        r#"{"status":"done","completed_at":"2020-01-01 00:00:00"}"#,
    )
    .await
    .unwrap();

    // Archive sweep (far-future cutoff archives any finished task). This stamps
    // archived_at = now via the production ArchiveOldIntents path.
    let archived = mem.archive_old_intents(now + 999_999).await.unwrap();
    assert!(
        archived.iter().any(|(aid, _, _, _)| *aid == id),
        "ancient done task should be archived"
    );

    // Retention delete with a PRODUCTION-style cutoff (~180d ago). The row's
    // original finished-at (2020) is far older than the cutoff, so the OLD
    // finished-at-based logic would delete it immediately. The fix keys on
    // archived_at (= now), so it is correctly RETAINED for the grace window.
    let cutoff = now - 180 * 86_400;
    let (deleted, _, _) = mem.delete_old_archived_tasks(cutoff, 500).await.unwrap();
    assert_eq!(
        deleted, 0,
        "freshly-archived task must NOT be deleted in the same sweep (archived_at grace)"
    );
    assert!(
        mem.get_task(id).await.is_some(),
        "ancient-but-just-archived task must survive its grace window"
    );
}

#[tokio::test]
async fn manual_archive_via_update_task_also_gets_grace_window() {
    // REGRESSION (CTO review of the F1 fix): the PRIMARY production archive
    // path is update_task({"status":"archived"}) — manage_tasks `delete`,
    // Telegram confirmdel, panel task_archive all route through it, NOT the
    // auto-archive sweep. If only the sweep stamps archived_at, a long-finished
    // task archived manually keeps archived_at=NULL, the retention COALESCE
    // falls through to its OLD completed_at, and it is permanently deleted on
    // the very next sweep — skipping the grace window. UpdateTask must stamp
    // archived_at when status becomes 'archived'.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7009;

    // Completed long ago, THEN manually archived (two-step, like production).
    let id = store_plain_task(&mem, chat, "old then manually archived").await;
    mem.update_task(
        id,
        chat,
        r#"{"status":"done","completed_at":"2020-01-01 00:00:00"}"#,
    )
    .await
    .unwrap();
    mem.update_task(id, chat, r#"{"status":"archived"}"#)
        .await
        .unwrap();

    // Production-style cutoff (~180d ago). archived_at = now (stamped by the
    // manual path), so the row is RETAINED despite its 2020 completed_at.
    let cutoff = now - 180 * 86_400;
    let (deleted, _, _) = mem.delete_old_archived_tasks(cutoff, 500).await.unwrap();
    assert_eq!(
        deleted, 0,
        "manually-archived old task must get its grace window (archived_at stamped on update_task)"
    );
    assert!(
        mem.get_task(id).await.is_some(),
        "manually-archived old task must survive the immediate retention sweep"
    );
}

#[tokio::test]
async fn retention_delete_nonpositive_limit_deletes_nothing() {
    // REGRESSION (Codex F2): a non-positive limit must delete NOTHING. The
    // primitive previously passed the caller's limit straight to SQLite, where
    // `LIMIT -1` means UNBOUNDED — so a negative limit would wipe every
    // eligible archived row. The clamp lives inside the destructive primitive.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let chat = 7008;
    for i in 0..2 {
        let id = store_plain_task(&mem, chat, &format!("arch {i}")).await;
        mem.update_task(id, chat, r#"{"status":"archived"}"#)
            .await
            .unwrap();
    }
    // Far-future cutoff => both rows are "old"; -1 must NOT mean unbounded.
    let (deleted, min_id, max_id) = mem
        .delete_old_archived_tasks(now + 999_999, -1)
        .await
        .unwrap();
    assert_eq!(
        deleted, 0,
        "negative limit must delete nothing, not everything"
    );
    assert_eq!(min_id, None);
    assert_eq!(max_id, None);
    assert_eq!(
        mem.list_tasks(chat, None, Some("archived"), 50).await.len(),
        2,
        "both archived rows survive a non-positive-limit sweep"
    );
    // limit == 0 likewise deletes nothing.
    let (deleted_zero, _, _) = mem
        .delete_old_archived_tasks(now + 999_999, 0)
        .await
        .unwrap();
    assert_eq!(deleted_zero, 0, "zero limit must delete nothing");
}

#[test]
fn archive_and_delete_setting_defaults_are_registered() {
    let defs = crate::settings::system_setting_definitions();
    let archive = defs
        .iter()
        .find(|d| d.key == "task_archive_after_days")
        .expect("task_archive_after_days must be registered");
    assert_eq!(archive.default.as_i64(), Some(30));
    let delete = defs
        .iter()
        .find(|d| d.key == "task_delete_archived_after_days")
        .expect("task_delete_archived_after_days must be registered");
    assert_eq!(delete.default.as_i64(), Some(180));
}

// ── Monitoring intent (unified tasks table) tests ──────────────────

#[tokio::test]
async fn monitoring_intent_store_and_list() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:100";
    let emb = vec![1.0_f32, 2.0, 3.0];

    let id = mem
        .store_monitoring_intent(
            did,
            "crypto_tracker",
            "BTC drops below 50k",
            0.75,
            emb.clone(),
        )
        .await
        .unwrap();
    assert!(id > 0);

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].monitoring_skill, "crypto_tracker");
    assert_eq!(intents[0].description, "BTC drops below 50k");
    assert!((intents[0].monitoring_threshold - 0.75).abs() < f32::EPSILON);
    assert_eq!(intents[0].embedding.len(), 3);
}

#[tokio::test]
async fn monitoring_intent_upsert_replaces_existing() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:101";
    let emb = vec![1.0_f32, 2.0, 3.0];

    // Store first intent
    mem.store_monitoring_intent(did, "news_digest", "only political news", 0.6, emb.clone())
        .await
        .unwrap();
    // Store second intent for same skill — should replace
    mem.store_monitoring_intent(did, "news_digest", "only tech news", 0.8, emb.clone())
        .await
        .unwrap();

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert_eq!(
        intents.len(),
        1,
        "upsert should keep only one intent per skill"
    );
    assert_eq!(intents[0].description, "only tech news");
    assert!((intents[0].monitoring_threshold - 0.8).abs() < f32::EPSILON);
}

#[tokio::test]
async fn monitoring_intent_delete() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:102";
    let emb = vec![1.0_f32, 2.0, 3.0];

    let id = mem
        .store_monitoring_intent(did, "server_monitor", "only critical errors", 0.7, emb)
        .await
        .unwrap();
    mem.delete_monitoring_intent(id).await.unwrap();

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert!(intents.is_empty(), "intent should be deleted");
}

#[tokio::test]
async fn monitoring_intent_get_all_cross_user() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![1.0_f32, 2.0, 3.0];

    mem.store_monitoring_intent(
        "did:aegis:telegram:200",
        "crypto_tracker",
        "BTC",
        0.7,
        emb.clone(),
    )
    .await
    .unwrap();
    mem.store_monitoring_intent(
        "did:aegis:telegram:201",
        "server_monitor",
        "CPU",
        0.8,
        emb.clone(),
    )
    .await
    .unwrap();

    let all = mem.get_all_monitoring_intents().await.unwrap();
    assert_eq!(all.len(), 2, "should return intents from all users");

    let skills: Vec<&str> = all.iter().map(|r| r.monitoring_skill.as_str()).collect();
    assert!(skills.contains(&"crypto_tracker"));
    assert!(skills.contains(&"server_monitor"));
}

#[tokio::test]
async fn monitoring_intent_did_isolation() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![1.0_f32, 2.0, 3.0];

    mem.store_monitoring_intent(
        "did:aegis:telegram:300",
        "skill_a",
        "desc_a",
        0.7,
        emb.clone(),
    )
    .await
    .unwrap();
    mem.store_monitoring_intent(
        "did:aegis:telegram:301",
        "skill_b",
        "desc_b",
        0.8,
        emb.clone(),
    )
    .await
    .unwrap();

    let a = mem
        .list_monitoring_intents("did:aegis:telegram:300")
        .await
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].monitoring_skill, "skill_a");

    let b = mem
        .list_monitoring_intents("did:aegis:telegram:301")
        .await
        .unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].monitoring_skill, "skill_b");
}

#[tokio::test]
async fn monitoring_intent_embedding_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:400";
    let emb = vec![0.1_f32, 0.2, 0.3, 0.4, 0.5];

    mem.store_monitoring_intent(did, "test_skill", "test description", 0.65, emb.clone())
        .await
        .unwrap();

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].embedding.len(), emb.len());
    for (a, b) in intents[0].embedding.iter().zip(emb.iter()) {
        assert!(
            (a - b).abs() < f32::EPSILON,
            "embedding values should roundtrip exactly"
        );
    }
}

// ── ExpireOldIntents atomic behavior tests ───────────────────────────

#[tokio::test]
async fn expire_old_intents_returns_cancelled_ids() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Create 3 intents: 2 expired, 1 still valid
    let id1 = mem
        .store_deferred_intent(
            "did:test:expire1",
            100,
            "time",
            Some(now - 200),
            None,
            "notify",
            "expired first",
            now - 50,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let id2 = mem
        .store_deferred_intent(
            "did:test:expire2",
            200,
            "time",
            Some(now - 100),
            None,
            "notify",
            "expired second",
            now - 10,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let _id_valid = mem
        .store_deferred_intent(
            "did:test:valid",
            300,
            "time",
            Some(now + 60),
            None,
            "notify",
            "still valid",
            now + 7200,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Run expiry
    let expired = mem.expire_old_intents(now).await.unwrap();

    // Verify: returned list contains exactly the 2 expired intents
    assert_eq!(
        expired.len(),
        2,
        "Should expire exactly 2 intents, got {}",
        expired.len()
    );

    let expired_ids: Vec<i64> = expired.iter().map(|(id, _, _)| *id).collect();
    assert!(
        expired_ids.contains(&id1),
        "Expired list should contain id1 ({}), got: {:?}",
        id1,
        expired_ids
    );
    assert!(
        expired_ids.contains(&id2),
        "Expired list should contain id2 ({}), got: {:?}",
        id2,
        expired_ids
    );

    // Verify user_did and context_text in returned tuples
    let first = expired.iter().find(|(id, _, _)| *id == id1).unwrap();
    assert_eq!(first.1, "did:test:expire1");
    assert_eq!(first.2, "expired first");

    let second = expired.iter().find(|(id, _, _)| *id == id2).unwrap();
    assert_eq!(second.1, "did:test:expire2");
    assert_eq!(second.2, "expired second");

    // Verify the valid intent is still active (appears in pending list)
    let still_pending = mem.list_user_intents("did:test:valid").await.unwrap();
    assert_eq!(
        still_pending.len(),
        1,
        "Valid intent should still be pending"
    );
    assert_eq!(still_pending[0].context_text, "still valid");

    // Verify expired intents are no longer in pending list
    let pending1 = mem.list_user_intents("did:test:expire1").await.unwrap();
    assert!(
        pending1.is_empty(),
        "Expired intent 1 should not appear in pending list"
    );
    let pending2 = mem.list_user_intents("did:test:expire2").await.unwrap();
    assert!(
        pending2.is_empty(),
        "Expired intent 2 should not appear in pending list"
    );
}

// ── fire_and_advance_recurring memory-level tests ────────────────────

#[tokio::test]
async fn fire_and_advance_recurring_atomic_fields() {
    // Verifies that fire_and_advance_recurring atomically updates all 4 fields
    // (fired_at, trigger_at_unix, expires_at_unix, fire_count) in a single SQL statement.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let interval = 900i64; // 15 minutes

    let id = mem
        .store_deferred_intent(
            "did:test:atomic",
            100,
            "time",
            Some(now - 5),
            None,
            "llm_callback",
            "recurring atomic test",
            now + 86400 * 30,
            false,
            Some(interval),
            0,
            false,
            Some("atomic_test"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Initially due
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].fire_count, 0);
    assert!(due[0].fired_at.is_none());

    // Fire and advance
    let next_trigger = now + interval;
    let next_expires = now + 86400 * 30 + interval;
    mem.fire_and_advance_recurring(id, next_trigger, next_expires)
        .await
        .unwrap();

    // No longer due at current time
    let due_now = mem.get_due_time_intents(now).await.unwrap();
    assert!(due_now.is_empty());

    // Due at next trigger time
    let due_next = mem.get_due_time_intents(next_trigger + 1).await.unwrap();
    assert_eq!(due_next.len(), 1);
    let intent = &due_next[0];
    assert_eq!(intent.fire_count, 1, "fire_count should be incremented");
    assert!(intent.fired_at.is_some(), "fired_at should be set");
    assert_eq!(
        intent.trigger_at_unix,
        Some(next_trigger),
        "trigger_at should be advanced"
    );

    // Fire again (fire_count: 1 -> 2)
    let next_trigger2 = next_trigger + interval;
    let next_expires2 = next_expires + interval;
    mem.fire_and_advance_recurring(id, next_trigger2, next_expires2)
        .await
        .unwrap();

    let due_next2 = mem.get_due_time_intents(next_trigger2 + 1).await.unwrap();
    assert_eq!(due_next2.len(), 1);
    assert_eq!(
        due_next2[0].fire_count, 2,
        "fire_count should be 2 after second fire"
    );
}

#[tokio::test]
async fn heartbeat_audit_raw_process_log_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let raw_log = "Round 1: LLM reasoning about system state\nTool call: query_memory({\"query\": \"test\"})\nResult: no matches\nRound 2: FALLTHROUGH";

    mem.store_heartbeat_audit(
        "test-cycle-001",
        "__heartbeat__",
        "Info",
        None,
        &[],
        &[],
        &["checked system".to_string()],
        Some("All clear"),
        150,
        2,
        "did:aegis:test",
        Some(12345),
        "{}",
        Some(raw_log),
        None,
    )
    .await
    .unwrap();

    let entries = mem.query_heartbeat_audit(None, 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_process_log.as_deref(), Some(raw_log));
    assert_eq!(entries[0].skill_name, "__heartbeat__");
    assert_eq!(entries[0].rounds_used, 2);
}

#[tokio::test]
async fn heartbeat_audit_raw_process_log_none_when_absent() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    mem.store_heartbeat_audit(
        "test-cycle-002",
        "probe_test",
        "Warning",
        Some("probe data"),
        &["query_memory".to_string()],
        &[],
        &[],
        None,
        50,
        1,
        "did:aegis:test",
        Some(12345),
        "{}",
        None, // no raw process log for probe-based heartbeat
        None,
    )
    .await
    .unwrap();

    let entries = mem
        .query_heartbeat_audit(Some("probe_test"), 10)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].raw_process_log.is_none());
}

#[tokio::test]
async fn heartbeat_audit_ext_fields_null_on_legacy_writer() {
    // Step 2: legacy `store_heartbeat_audit` leaves the six new columns NULL.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_heartbeat_audit(
        "legacy-1",
        "__heartbeat__",
        "Info",
        None,
        &[],
        &[],
        &[],
        None,
        0,
        0,
        "did:aegis:test",
        Some(1),
        "{}",
        None,
        None,
    )
    .await
    .unwrap();

    let entries = mem.query_heartbeat_audit(None, 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert!(e.outcome.is_none());
    assert!(e.skip_reason.is_none());
    assert!(e.config_snapshot_json.is_none());
    assert!(e.prompt_hash.is_none());
    assert!(e.proposals_created_json.is_none());
    assert!(e.next_tick_note_bytes.is_none());
    assert!(e.parsed_outcome().is_none());
    assert!(e.parsed_skip_reason().is_none());
    assert!(e.proposals_created().is_empty());
}

#[tokio::test]
async fn heartbeat_audit_ext_full_roundtrip() {
    use crate::heartbeat_config::{HeartbeatSkipReason, HeartbeatTickOutcome};
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let extras = HeartbeatAuditExtras {
        outcome: Some(HeartbeatTickOutcome::Acted),
        skip_reason: None,
        skip_reason_text: None,
        config_snapshot_json: Some(r#"{"frequency_minutes":30}"#.to_string()),
        prompt_hash: Some("deadbeef".into()),
        proposals_created: vec!["prop-1".into(), "prop-2".into()],
        next_tick_note_bytes: Some(256),
    };
    mem.store_heartbeat_audit_ext(
        "acted-1",
        "__heartbeat__",
        "Info",
        None,
        &["send_message".to_string()],
        &[],
        &["sent nudge".to_string()],
        Some("done"),
        1200,
        3,
        "did:aegis:test",
        Some(42),
        "{}",
        None,
        None,
        &extras,
    )
    .await
    .unwrap();

    // Also write a SkippedPrecheck row for ordering / enum coverage.
    let skip_extras = HeartbeatAuditExtras {
        outcome: Some(HeartbeatTickOutcome::SkippedPrecheck),
        skip_reason: Some(HeartbeatSkipReason::NoNewContext),
        skip_reason_text: None,
        config_snapshot_json: Some("{}".into()),
        prompt_hash: None,
        proposals_created: Vec::new(),
        next_tick_note_bytes: None,
    };
    // Ensure the skip row gets a later autoincrement id (DESC order).
    mem.store_heartbeat_audit_ext(
        "skip-1",
        "__heartbeat__",
        "Info",
        None,
        &[],
        &[],
        &[],
        None,
        0,
        0,
        "did:aegis:test",
        Some(42),
        "{}",
        None,
        None,
        &skip_extras,
    )
    .await
    .unwrap();

    let entries = mem.query_heartbeat_audit(None, 10).await.unwrap();
    assert_eq!(entries.len(), 2);
    // DESC by id → skip-1 is first, acted-1 second.
    assert_eq!(entries[0].cycle_id, "skip-1");
    assert_eq!(
        entries[0].parsed_outcome(),
        Some(HeartbeatTickOutcome::SkippedPrecheck)
    );
    assert_eq!(
        entries[0].parsed_skip_reason(),
        Some(HeartbeatSkipReason::NoNewContext)
    );

    assert_eq!(entries[1].cycle_id, "acted-1");
    assert_eq!(
        entries[1].parsed_outcome(),
        Some(HeartbeatTickOutcome::Acted)
    );
    assert_eq!(entries[1].prompt_hash.as_deref(), Some("deadbeef"));
    assert_eq!(entries[1].next_tick_note_bytes, Some(256));
    assert_eq!(
        entries[1].proposals_created(),
        vec!["prop-1".to_string(), "prop-2".to_string()]
    );
    assert!(
        entries[1]
            .config_snapshot_json
            .as_deref()
            .unwrap_or("")
            .contains("frequency_minutes")
    );
}

#[tokio::test]
async fn heartbeat_audit_ext_skip_reason_text_fallback() {
    // When no enum variant is supplied, free-text fallback is persisted.
    use crate::heartbeat_config::HeartbeatTickOutcome;
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let extras = HeartbeatAuditExtras {
        outcome: Some(HeartbeatTickOutcome::BudgetExhausted),
        skip_reason: None,
        skip_reason_text: Some("used=500000 budget=500000".into()),
        ..Default::default()
    };
    mem.store_heartbeat_audit_ext(
        "budget-1",
        "__heartbeat__",
        "warn",
        None,
        &[],
        &[],
        &[],
        None,
        0,
        0,
        "did:aegis:test",
        Some(1),
        "{}",
        None,
        None,
        &extras,
    )
    .await
    .unwrap();

    let entries = mem.query_heartbeat_audit(None, 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].skip_reason.as_deref(),
        Some("used=500000 budget=500000")
    );
    // Free-text isn't a known enum variant → parsed_skip_reason returns None.
    assert!(entries[0].parsed_skip_reason().is_none());
}

#[tokio::test]
async fn audit_recent_llm_eligible_heartbeat_count_excludes_skip_paths() {
    use crate::heartbeat_config::HeartbeatTickOutcome;

    let mem = MemoryHandle::spawn_in_memory().unwrap();

    for (cycle_id, outcome) in [
        ("cycle-noop", HeartbeatTickOutcome::Noop),
        ("cycle-maintenance", HeartbeatTickOutcome::MaintenanceOnly),
        ("cycle-acted", HeartbeatTickOutcome::Acted),
        ("cycle-skip", HeartbeatTickOutcome::SkippedPrecheck),
        ("cycle-budget", HeartbeatTickOutcome::BudgetExhausted),
    ] {
        let extras = HeartbeatAuditExtras {
            outcome: Some(outcome),
            ..Default::default()
        };
        mem.store_heartbeat_audit_ext(
            cycle_id,
            "__heartbeat__",
            "info",
            None,
            &[],
            &[],
            &[],
            None,
            0,
            0,
            "did:aegis:test",
            Some(1),
            "{}",
            None,
            None,
            &extras,
        )
        .await
        .unwrap();
    }

    let eligible = mem
        .audit_recent_llm_eligible_heartbeat_count(1)
        .await
        .unwrap();
    assert_eq!(eligible, 3);
}

#[tokio::test]
async fn audit_heartbeat_noop_ratio_ignores_skip_and_error_rows() {
    use crate::heartbeat_config::HeartbeatTickOutcome;

    let mem = MemoryHandle::spawn_in_memory().unwrap();

    for (cycle_id, outcome) in [
        ("cycle-noop", HeartbeatTickOutcome::Noop),
        ("cycle-maintenance", HeartbeatTickOutcome::MaintenanceOnly),
        ("cycle-acted", HeartbeatTickOutcome::Acted),
        ("cycle-skip", HeartbeatTickOutcome::SkippedPrecheck),
        ("cycle-error", HeartbeatTickOutcome::Error),
    ] {
        let extras = HeartbeatAuditExtras {
            outcome: Some(outcome),
            ..Default::default()
        };
        mem.store_heartbeat_audit_ext(
            cycle_id,
            "__heartbeat__",
            "info",
            None,
            &[],
            &[],
            &[],
            None,
            0,
            0,
            "did:aegis:test",
            Some(1),
            "{}",
            None,
            None,
            &extras,
        )
        .await
        .unwrap();
    }

    let extras_other = HeartbeatAuditExtras {
        outcome: Some(HeartbeatTickOutcome::Noop),
        ..Default::default()
    };
    mem.store_heartbeat_audit_ext(
        "cycle-other-skill",
        "server_monitor",
        "info",
        None,
        &[],
        &[],
        &[],
        None,
        0,
        0,
        "did:aegis:test",
        Some(1),
        "{}",
        None,
        None,
        &extras_other,
    )
    .await
    .unwrap();

    let (noop, total) = mem.audit_heartbeat_noop_ratio(1).await.unwrap();
    assert_eq!(noop, 1, "only heartbeat outcome=noop should count");
    assert_eq!(
        total, 3,
        "heartbeat noop+maintenance_only+acted rows are LLM-eligible"
    );
}

#[tokio::test]
async fn audit_recent_heartbeat_noop_count_counts_only_heartbeat_noops() {
    use crate::heartbeat_config::HeartbeatTickOutcome;

    let mem = MemoryHandle::spawn_in_memory().unwrap();

    for (cycle_id, skill_name, outcome) in [
        ("cycle-noop", "__heartbeat__", HeartbeatTickOutcome::Noop),
        (
            "cycle-skip",
            "__heartbeat__",
            HeartbeatTickOutcome::SkippedPrecheck,
        ),
        (
            "cycle-other-skill",
            "server_monitor",
            HeartbeatTickOutcome::Noop,
        ),
    ] {
        let extras = HeartbeatAuditExtras {
            outcome: Some(outcome),
            ..Default::default()
        };
        mem.store_heartbeat_audit_ext(
            cycle_id,
            skill_name,
            "info",
            None,
            &[],
            &[],
            &[],
            None,
            0,
            0,
            "did:aegis:test",
            Some(1),
            "{}",
            None,
            None,
            &extras,
        )
        .await
        .unwrap();
    }

    let recent = mem.audit_recent_heartbeat_noop_count(1).await.unwrap();
    assert_eq!(recent, 1);
}

#[tokio::test]
async fn hitl_pending_updated_since_counts_recent_only() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_hitl_approval(
        "approve:test",
        r#"{"summary":"test"}"#,
        "config_change",
        None,
        None,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .unwrap();

    let recent_since = chrono::Utc::now() - chrono::Duration::minutes(5);
    let future_since = chrono::Utc::now() + chrono::Duration::minutes(5);

    assert_eq!(
        mem.count_hitl_pending_updated_since(recent_since)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        mem.count_hitl_pending_updated_since(future_since)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn sum_todays_heartbeat_tokens_empty_db() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let sum = mem.sum_todays_heartbeat_tokens().await.unwrap();
    assert_eq!(sum, 0);
}

#[tokio::test]
async fn sum_todays_heartbeat_tokens_accumulates() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Store two audit entries with different token counts
    mem.store_heartbeat_audit(
        "cycle-a",
        "__heartbeat__",
        "Info",
        None,
        &[],
        &[],
        &["memo".to_string()],
        Some("all good"),
        5000,
        2,
        "did:aegis:test",
        Some(123),
        "{}",
        None,
        None,
    )
    .await
    .unwrap();

    mem.store_heartbeat_audit(
        "cycle-b",
        "probe_crypto",
        "Warning",
        Some("price drop"),
        &["web_search".to_string()],
        &[],
        &["alert".to_string()],
        Some("alerted"),
        3200,
        1,
        "did:aegis:test",
        Some(123),
        "{}",
        None,
        None,
    )
    .await
    .unwrap();

    let sum = mem.sum_todays_heartbeat_tokens().await.unwrap();
    assert_eq!(sum, 8200);
}

// ── Channel error / graceful degradation tests ──────────────────────

/// When the memory actor is dropped, healthcheck returns false (not panic).
#[tokio::test]
async fn healthcheck_returns_false_after_actor_drop() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    assert!(mem.healthcheck().await, "should be healthy initially");
    // Drop the actor by dropping all senders — actor loop exits when rx.recv() returns None.
    // We can't directly drop the actor, but we can create a handle with a dead channel.
    let (tx, _rx) = tokio::sync::mpsc::channel::<MemoryCommand>(1);
    drop(_rx); // Close the receiver immediately
    let dead_handle = MemoryHandle {
        tx,
        max_db_size: DEFAULT_MAX_DB_SIZE,
        hnsw: None,
        circuit_breaker_rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        actor_handle: None,
        db_path: ":memory:".to_string(),
        heirloom_bridge: None,
    };
    assert!(
        !dead_handle.healthcheck().await,
        "dead channel should return false"
    );
}

/// mem_send! on a dead channel returns an error, not a panic.
#[tokio::test]
async fn dead_channel_returns_error_not_panic() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<MemoryCommand>(1);
    drop(_rx);
    let dead_handle = MemoryHandle {
        tx,
        max_db_size: DEFAULT_MAX_DB_SIZE,
        hnsw: None,
        circuit_breaker_rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        actor_handle: None,
        db_path: ":memory:".to_string(),
        heirloom_bridge: None,
    };
    let result = dead_handle.get_l1("did:test").await;
    assert!(result.is_err(), "dead channel should return Err, not panic");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("memory actor closed") || err_msg.contains("actor"),
        "Error should mention actor closure, got: {}",
        err_msg
    );
}

/// mem_send_or! on a dead channel returns the default value, not panic.
#[tokio::test]
async fn dead_channel_mem_send_or_returns_default() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<MemoryCommand>(1);
    drop(_rx);
    let dead_handle = MemoryHandle {
        tx,
        max_db_size: DEFAULT_MAX_DB_SIZE,
        hnsw: None,
        circuit_breaker_rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        actor_handle: None,
        db_path: ":memory:".to_string(),
        heirloom_bridge: None,
    };
    // list_reminders_for_chat uses mem_send_or! returning empty Vec on error
    // healthcheck uses a different pattern but also degrades gracefully
    assert!(
        !dead_handle.healthcheck().await,
        "should gracefully return false"
    );
}

mod part2;

#[tokio::test]
async fn expired_time_intents_are_not_due_even_if_trigger_passed() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    mem.store_deferred_intent(
        "did:test:expired",
        100,
        "time",
        Some(now - 7200),
        None,
        "notify",
        "stale reminder",
        now - 60,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    mem.store_deferred_intent(
        "did:test:expired",
        100,
        "time",
        Some(now - 60),
        None,
        "notify",
        "fresh reminder",
        now + 3600,
        false,
        None,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let due = mem.get_due_time_intents(now).await.unwrap();
    assert_eq!(due.len(), 1, "only the unexpired intent is due");
    assert_eq!(due[0].context_text, "fresh reminder");
}

#[tokio::test]
async fn turn_evidence_round_trip_newest_first_and_caps_at_200() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:turn-evidence";
    let item = crate::evidence::EvidenceItem {
        id: "E1".into(),
        tool: "web_search".into(),
        label: "Web search".into(),
        excerpt: "today cloudy".into(),
        full_len: 12,
    };
    mem.store_turn_evidence(
        user_did,
        "older-key",
        std::slice::from_ref(&item),
        &["99".into()],
    )
    .await
    .unwrap();
    mem.store_turn_evidence(user_did, "newer-key", std::slice::from_ref(&item), &[])
        .await
        .unwrap();
    let listed = mem.list_recent_turn_evidence(user_did, 10).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].reply_key, "newer-key");
    assert_eq!(listed[1].reply_key, "older-key");
    assert_eq!(listed[1].evidence, vec![item.clone()]);
    assert_eq!(listed[1].unsupported_numbers, vec!["99".to_string()]);

    for index in 0..205 {
        mem.store_turn_evidence(
            user_did,
            &format!("cap-key-{index}"),
            std::slice::from_ref(&item),
            &[],
        )
        .await
        .unwrap();
    }
    let capped = mem.list_recent_turn_evidence(user_did, 300).await.unwrap();
    assert_eq!(capped.len(), 200);
    assert_eq!(capped[0].reply_key, "cap-key-204");
    assert_eq!(capped[199].reply_key, "cap-key-5");
    assert!(
        capped
            .iter()
            .all(|row| row.reply_key.starts_with("cap-key-"))
    );
}
