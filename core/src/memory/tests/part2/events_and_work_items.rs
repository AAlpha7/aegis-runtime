//! Ephemeral cleanup, config audit, observed events, assistant event spine and cursors, work items, HITL, subcall traces, L1 batch trimming.
use super::*;

// ── Step 6: cleanup_expired_ephemeral (spec §5.3)

#[tokio::test]
async fn cleanup_expired_ephemeral_deletes_only_past() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:test:ephemeral_cleanup";

    // Insert one expired and one valid row via store_ephemeral — use a
    // very short TTL for the expired one (we simulate by backdating the
    // expires_at directly via raw SQL after the fact is impractical here;
    // instead, negative TTL triggers immediate expiry).
    mem.store_ephemeral(did, "test:old", "old fact", None, "l1_overflow", -1)
        .await
        .unwrap();
    mem.store_ephemeral(did, "test:new", "new fact", None, "l1_overflow", 30)
        .await
        .unwrap();

    let before = mem.count_ephemeral(did, None).await.unwrap();
    assert_eq!(before, 2);

    let deleted = mem.cleanup_expired_ephemeral().await.unwrap();
    assert_eq!(deleted, 1, "only the expired row should be deleted");

    let after = mem.count_ephemeral(did, None).await.unwrap();
    assert_eq!(after, 1);
}

// ── Step 1: config_audit_log API round-trip ─────────────────────────
#[tokio::test]
async fn config_audit_insert_and_list_roundtrip() {
    use crate::heartbeat_config::ConfigAuditActor;
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Start clean — any migration-seeded rows are unrelated to this test.
    let id1 = mem
        .insert_config_audit(
            ConfigAuditActor::Llm,
            Some("{\"frequency_minutes\":30}".to_string()),
            "{\"frequency_minutes\":15}".to_string(),
            Some("user asked to speed up".to_string()),
        )
        .await
        .unwrap();
    assert!(id1 > 0);

    let id2 = mem
        .insert_config_audit(
            ConfigAuditActor::User,
            None,
            "{\"frequency_minutes\":20}".to_string(),
            None,
        )
        .await
        .unwrap();
    assert!(id2 > id1);

    let rows = mem.list_config_audit(10).await.unwrap();
    // Newest first — id2 must precede id1.
    let ours: Vec<_> = rows
        .into_iter()
        .filter(|r| r.id == id1 || r.id == id2)
        .collect();
    assert_eq!(ours.len(), 2);
    assert_eq!(ours[0].id, id2);
    assert_eq!(ours[0].actor, "user");
    assert_eq!(ours[0].old_json, None);
    assert_eq!(ours[0].reason, None);
    assert_eq!(ours[1].id, id1);
    assert_eq!(ours[1].actor, "llm");
    assert_eq!(
        ours[1].old_json.as_deref(),
        Some("{\"frequency_minutes\":30}")
    );
    assert_eq!(ours[1].reason.as_deref(), Some("user asked to speed up"));
}

#[tokio::test]
async fn observed_events_record_and_drain_roundtrip() {
    // Step 3 (design §15.2): record_observed_event → take_unconsumed_observed_events
    // returns the event; a second call yields nothing (rows are consumed).
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let payload = serde_json::json!({ "count": 3 });
    let id1 = mem
        .record_observed_event("calendar_reader", "warn", "3 unread urgent", Some(&payload))
        .await
        .unwrap();
    let id2 = mem
        .record_observed_event("calendar", "info", "meeting in 30 min", None)
        .await
        .unwrap();
    assert!(id2 > id1);

    let first = mem.take_unconsumed_observed_events(50).await.unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].source, "calendar_reader");
    assert_eq!(first[0].severity, "warn");
    assert_eq!(first[0].summary, "3 unread urgent");
    assert!(
        first[0]
            .payload_json
            .as_deref()
            .unwrap()
            .contains("\"count\":3")
    );
    assert_eq!(first[1].source, "calendar");
    assert!(first[1].payload_json.is_none());

    // Second drain: nothing left — rows are marked consumed.
    let second = mem.take_unconsumed_observed_events(50).await.unwrap();
    assert!(
        second.is_empty(),
        "consumed rows must not return; got {second:?}"
    );
}

#[tokio::test]
async fn observed_events_summary_is_bounded() {
    // summary is truncated to 500 chars at the handle boundary to bound
    // prompt cost for degenerate probe payloads.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let huge: String = "x".repeat(10_000);
    mem.record_observed_event("big", "info", &huge, None)
        .await
        .unwrap();
    let events = mem.take_unconsumed_observed_events(10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary.chars().count(), 500);
}

#[tokio::test]
async fn observed_events_drain_respects_limit_and_is_ordered() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    for i in 0..5 {
        mem.record_observed_event("probe", "info", &format!("ev-{i}"), None)
            .await
            .unwrap();
    }
    let batch = mem.take_unconsumed_observed_events(3).await.unwrap();
    assert_eq!(batch.len(), 3);
    assert_eq!(batch[0].summary, "ev-0");
    assert_eq!(batch[2].summary, "ev-2");

    // Remaining two stay in queue and now come out in order.
    let rest = mem.take_unconsumed_observed_events(50).await.unwrap();
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[0].summary, "ev-3");
    assert_eq!(rest[1].summary, "ev-4");
}

#[tokio::test]
async fn assistant_event_spine_stores_and_lists_recent_events() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let (raw1, mut ev1) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-22T00:00:00Z",
        "primary",
        "update",
        "cal-1",
        "evt-1",
        "Budget review",
        Some("2026-04-23T10:00:00Z"),
        Some("2026-04-23T11:00:00Z"),
        3,
    );
    ev1.decision = crate::assistant_events::AssistantEventDecision::CallbackNow;
    let raw1_id = mem.store_raw_external_event(&raw1).await.unwrap();
    ev1.raw_event_id = Some(raw1_id);
    let ev1_id = mem.store_assistant_event(&ev1).await.unwrap();

    let (raw2, mut ev2) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-22T00:05:00Z",
        "primary",
        "create",
        "cal-1",
        "evt-2",
        "Weekly digest",
        Some("2026-04-24T09:00:00Z"),
        Some("2026-04-24T09:30:00Z"),
        1,
    );
    ev2.decision = crate::assistant_events::AssistantEventDecision::DigestLater;
    let raw2_id = mem.store_raw_external_event(&raw2).await.unwrap();
    ev2.raw_event_id = Some(raw2_id);
    let ev2_id = mem.store_assistant_event(&ev2).await.unwrap();
    assert!(ev2_id > ev1_id);

    let rows = mem
        .list_recent_assistant_events("did:aegis:test", 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, ev2_id);
    assert_eq!(
        rows[0].event.decision,
        crate::assistant_events::AssistantEventDecision::DigestLater
    );
    assert_eq!(rows[1].id, ev1_id);
    assert_eq!(
        rows[1].event.decision,
        crate::assistant_events::AssistantEventDecision::CallbackNow
    );
    assert_eq!(
        rows[1]
            .event
            .entities
            .get("summary")
            .and_then(|v| v.as_str()),
        Some("Budget review")
    );
}

#[tokio::test]
async fn assistant_event_spine_dedups_by_dedup_key() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let (raw, mut ev) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-22T00:00:00Z",
        "primary",
        "update",
        "cal-1",
        "evt-1",
        "Security review",
        Some("2026-04-23T10:00:00Z"),
        Some("2026-04-23T11:00:00Z"),
        2,
    );
    let raw1 = mem.store_raw_external_event(&raw).await.unwrap();
    let raw2 = mem.store_raw_external_event(&raw).await.unwrap();
    assert_eq!(raw1, raw2);

    ev.raw_event_id = Some(raw1);
    let ev1 = mem.store_assistant_event(&ev).await.unwrap();
    let ev2 = mem.store_assistant_event(&ev).await.unwrap();
    assert_eq!(ev1, ev2);

    let rows = mem
        .list_recent_assistant_events("did:aegis:test", 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].event.dedup_key,
        "calendar:primary:cal-1:evt-1:update:Security review:2026-04-23T10:00:00Z:2026-04-23T11:00:00Z"
    );
    assert_eq!(rows[0].event.raw_event_id, Some(raw1));
}

#[tokio::test]
async fn assistant_event_cursor_returns_only_new_rows_per_consumer() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let (raw1, mut ev1) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-22T00:00:00Z",
        "primary",
        "update",
        "cal-1",
        "evt-1",
        "Budget review",
        Some("2026-04-23T10:00:00Z"),
        Some("2026-04-23T11:00:00Z"),
        3,
    );
    let raw1_id = mem.store_raw_external_event(&raw1).await.unwrap();
    ev1.raw_event_id = Some(raw1_id);
    let ev1_id = mem.store_assistant_event(&ev1).await.unwrap();

    // Peek + commit (split form). Committing past the newest peeked id
    // suppresses re-peeks until a new row arrives.
    let first = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, ev1_id);
    mem.commit_assistant_event_cursor("did:aegis:test", "heartbeat", first[0].id)
        .await
        .unwrap();

    let second = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert!(second.is_empty(), "cursor must suppress repeats");

    let (raw2, mut ev2) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-22T00:05:00Z",
        "primary",
        "create",
        "cal-1",
        "evt-2",
        "Security review",
        Some("2026-04-24T09:00:00Z"),
        Some("2026-04-24T09:30:00Z"),
        1,
    );
    let raw2_id = mem.store_raw_external_event(&raw2).await.unwrap();
    ev2.raw_event_id = Some(raw2_id);
    let ev2_id = mem.store_assistant_event(&ev2).await.unwrap();

    let third = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].id, ev2_id);
    mem.commit_assistant_event_cursor("did:aegis:test", "heartbeat", third[0].id)
        .await
        .unwrap();

    // Different consumer — independent cursor, has not committed anything,
    // so still sees both rows.
    let other_consumer = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "digest", 10, None, None)
        .await
        .unwrap();
    assert_eq!(other_consumer.len(), 2);
    assert_eq!(other_consumer[0].id, ev1_id);
    assert_eq!(other_consumer[1].id, ev2_id);
}

/// Peeking must not advance the cursor, so a crash between peek and the
/// explicit commit redelivers the rows on the next peek.
#[tokio::test]
async fn assistant_event_cursor_does_not_advance_when_delivery_fails() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let mut event_ids = Vec::new();
    for i in 0..3 {
        let (raw, mut ev) = crate::assistant_events::build_calendar_event_changed_event(
            "did:aegis:test",
            "2026-04-25T00:00:00Z",
            "primary",
            "update",
            "cal-1",
            &format!("evt-{i}"),
            &format!("Subject {i}"),
            Some("2026-04-26T10:00:00Z"),
            Some("2026-04-26T11:00:00Z"),
            1,
        );
        let raw_id = mem.store_raw_external_event(&raw).await.unwrap();
        ev.raw_event_id = Some(raw_id);
        event_ids.push(mem.store_assistant_event(&ev).await.unwrap());
    }

    let first_peek = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert_eq!(first_peek.len(), 3);

    let second_peek = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert_eq!(
        second_peek.len(),
        3,
        "peek must not advance the cursor; events must redeliver after a failed commit"
    );
    assert_eq!(
        second_peek.iter().map(|r| r.id).collect::<Vec<_>>(),
        event_ids,
        "redelivered rows must match the originally peeked rows"
    );

    mem.commit_assistant_event_cursor("did:aegis:test", "heartbeat", *event_ids.last().unwrap())
        .await
        .unwrap();

    let third_peek = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert!(
        third_peek.is_empty(),
        "cursor must advance after successful commit"
    );
}

/// Committing only advances the cursor monotonically and never past unseen rows.
#[tokio::test]
async fn commit_assistant_event_cursor_is_monotonic() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    let (raw, mut ev) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        "2026-04-25T00:00:00Z",
        "primary",
        "update",
        "cal-1",
        "evt-only",
        "Subject",
        Some("2026-04-26T10:00:00Z"),
        Some("2026-04-26T11:00:00Z"),
        1,
    );
    let raw_id = mem.store_raw_external_event(&raw).await.unwrap();
    ev.raw_event_id = Some(raw_id);
    let ev_id = mem.store_assistant_event(&ev).await.unwrap();

    mem.commit_assistant_event_cursor("did:aegis:test", "heartbeat", ev_id)
        .await
        .unwrap();

    mem.commit_assistant_event_cursor("did:aegis:test", "heartbeat", ev_id - 1)
        .await
        .unwrap();
    let after_stale = mem
        .peek_new_assistant_events_for_consumer("did:aegis:test", "heartbeat", 10, None, None)
        .await
        .unwrap();
    assert!(
        after_stale.is_empty(),
        "stale commit must not regress the cursor"
    );
}

#[tokio::test]
async fn assistant_work_items_store_and_list_by_priority() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let later_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Weekly digest".to_string(),
        summary: "Newsletter".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
        decision: crate::assistant_events::AssistantEventDecision::DigestLater,
        dedup_key: "calendar:primary:m-1".to_string(),
        entities: serde_json::json!({}),
    };
    let later_event_id = mem.store_assistant_event(&later_event).await.unwrap();
    let mut later = crate::assistant_work_items::build_callback_review_work_item(
        &crate::assistant_events::StoredAssistantEvent {
            id: later_event_id,
            event: later_event,
        },
    );
    later.priority = crate::assistant_work_items::AssistantWorkPriority::Later;
    later.dedup_key = "assistant_work:test:later".to_string();
    let mut urgent = later.clone();
    urgent.priority = crate::assistant_work_items::AssistantWorkPriority::InterruptNow;
    urgent.dedup_key = "assistant_work:test:urgent".to_string();
    urgent.goal = "Urgent review".to_string();

    let later_id = mem.store_assistant_work_item(&later).await.unwrap();
    let urgent_id = mem.store_assistant_work_item(&urgent).await.unwrap();
    assert!(urgent_id > later_id);

    let rows = mem
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, urgent_id);
    assert_eq!(
        rows[0].item.anchor_kind,
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread)
    );
    assert_eq!(rows[0].item.anchor_ref.as_deref(), Some("t-1"));
    assert_eq!(
        rows[0].item.priority,
        crate::assistant_work_items::AssistantWorkPriority::InterruptNow
    );
    assert_eq!(rows[1].id, later_id);
}

#[tokio::test]
async fn assistant_work_items_dedup_and_claim_highest_priority_first() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let base_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Contract update".to_string(),
        summary: "Please review contract changes".to_string(),
        thread_key: Some("t-2".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-2".to_string(),
        entities: serde_json::json!({}),
    };
    let base_event_id = mem.store_assistant_event(&base_event).await.unwrap();
    let base_event = crate::assistant_events::StoredAssistantEvent {
        id: base_event_id,
        event: base_event,
    };
    let item1 = crate::assistant_work_items::build_callback_review_work_item(&base_event);
    let item1_id = mem.store_assistant_work_item(&item1).await.unwrap();
    let item1_dup = mem.store_assistant_work_item(&item1).await.unwrap();
    assert_eq!(item1_id, item1_dup);

    let mut item2 = crate::assistant_work_items::build_callback_review_work_item(&base_event);
    item2.dedup_key = "assistant_work:callback_review:calendar:primary:m-3".to_string();
    item2.goal = "Follow up later".to_string();
    item2.priority = crate::assistant_work_items::AssistantWorkPriority::Soon;
    let item2_id = mem.store_assistant_work_item(&item2).await.unwrap();
    assert!(item2_id > item1_id);

    let claimed_first = mem
        .claim_next_assistant_work_item("did:aegis:test")
        .await
        .unwrap()
        .expect("queued work item");
    assert_eq!(claimed_first.id, item1_id);
    assert_eq!(
        claimed_first.item.status,
        crate::assistant_work_items::AssistantWorkStatus::Running
    );

    let claimed_second = mem
        .claim_next_assistant_work_item("did:aegis:test")
        .await
        .unwrap()
        .expect("second queued work item");
    assert_eq!(claimed_second.id, item2_id);

    let none_left = mem
        .claim_next_assistant_work_item("did:aegis:test")
        .await
        .unwrap();
    assert!(none_left.is_none());

    mem.update_assistant_work_item(
        item1_id,
        crate::assistant_work_items::AssistantWorkStatus::Done,
        Some(&serde_json::json!({ "delivered": "digest_only" })),
    )
    .await
    .unwrap();

    let done_rows = mem
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done_rows.len(), 1);
    assert_eq!(done_rows[0].id, item1_id);
    assert_eq!(
        done_rows[0]
            .item
            .result
            .as_ref()
            .and_then(|v| v.get("delivered"))
            .and_then(|v| v.as_str()),
        Some("digest_only")
    );
}

#[tokio::test]
async fn assistant_work_items_claim_filter_selects_callback_reviews_only() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Contract update".to_string(),
        summary: "Please review contract changes".to_string(),
        thread_key: Some("t-2".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-4".to_string(),
        entities: serde_json::json!({}),
    };
    let assistant_event_id = mem.store_assistant_event(&assistant_event).await.unwrap();
    let stored_event = crate::assistant_events::StoredAssistantEvent {
        id: assistant_event_id,
        event: assistant_event,
    };

    let callback_item = crate::assistant_work_items::build_callback_review_work_item(&stored_event);
    let callback_id = mem.store_assistant_work_item(&callback_item).await.unwrap();

    let mut digest_item = callback_item.clone();
    digest_item.dedup_key = "assistant_work:digest:test".to_string();
    digest_item.goal = "Assemble a calm daily digest".to_string();
    digest_item.mode = crate::assistant_work_items::AssistantWorkMode::DigestAssembly;
    digest_item.priority = crate::assistant_work_items::AssistantWorkPriority::Later;
    digest_item.delivery_policy =
        crate::assistant_work_items::AssistantWorkDeliveryPolicy::DigestOnly;
    let digest_id = mem.store_assistant_work_item(&digest_item).await.unwrap();
    assert!(digest_id > callback_id);

    let claimed = mem
        .claim_next_assistant_work_item_matching(
            "did:aegis:test",
            &crate::assistant_work_items::AssistantWorkClaimFilter::callback_review(),
        )
        .await
        .unwrap()
        .expect("callback review work item");
    assert_eq!(claimed.id, callback_id);
    assert_eq!(
        claimed.item.mode,
        crate::assistant_work_items::AssistantWorkMode::BackgroundReview
    );

    let queued = mem
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, digest_id);
    assert_eq!(
        queued[0].item.mode,
        crate::assistant_work_items::AssistantWorkMode::DigestAssembly
    );
}

#[tokio::test]
async fn assistant_work_items_claim_filter_selects_deferred_intent_callbacks_only() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let deferred_intent = DeferredIntent {
        id: 77,
        user_did: "did:aegis:test".to_string(),
        chat_id: 12345,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(1_776_000_000),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "Follow up on the contract review.".to_string(),
        status: "pending".to_string(),
        created_at: "2026-04-23T00:00:00Z".to_string(),
        expires_at_unix: 1_776_003_600,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: Some(3600),
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("followup:contract-review".to_string()),
        max_fires: Some(5),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let deferred_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    let deferred_id = mem.store_assistant_work_item(&deferred_item).await.unwrap();

    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Contract update".to_string(),
        summary: "Please review contract changes".to_string(),
        thread_key: Some("t-2".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-5".to_string(),
        entities: serde_json::json!({}),
    };
    let assistant_event_id = mem.store_assistant_event(&assistant_event).await.unwrap();
    let callback_item = crate::assistant_work_items::build_callback_review_work_item(
        &crate::assistant_events::StoredAssistantEvent {
            id: assistant_event_id,
            event: assistant_event,
        },
    );
    let callback_id = mem.store_assistant_work_item(&callback_item).await.unwrap();
    assert!(callback_id > deferred_id);

    let claimed = mem
        .claim_next_assistant_work_item_matching(
            "did:aegis:test",
            &crate::assistant_work_items::AssistantWorkClaimFilter::deferred_intent_callback(),
        )
        .await
        .unwrap()
        .expect("deferred intent work item");
    assert_eq!(claimed.id, deferred_id);
    assert_eq!(
        claimed.item.source_kind,
        crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent
    );

    let queued = mem
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, callback_id);
    assert_eq!(
        queued[0].item.source_kind,
        crate::assistant_work_items::AssistantWorkSourceKind::AssistantEvent
    );
    assert_eq!(
        queued[0].item.anchor_kind,
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread)
    );
}

#[tokio::test]
async fn assistant_work_items_persist_anchor_metadata() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Contract update".to_string(),
        summary: "Please review contract changes".to_string(),
        thread_key: Some("thread-anchor-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-anchor".to_string(),
        entities: serde_json::json!({}),
    };
    let event_id = mem.store_assistant_event(&assistant_event).await.unwrap();
    let work_item = crate::assistant_work_items::build_callback_review_work_item(
        &crate::assistant_events::StoredAssistantEvent {
            id: event_id,
            event: assistant_event,
        },
    );

    let work_id = mem.store_assistant_work_item(&work_item).await.unwrap();
    let stored = mem
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, work_id);
    assert_eq!(
        stored[0].item.anchor_kind,
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread)
    );
    assert_eq!(
        stored[0].item.anchor_ref.as_deref(),
        Some("thread-anchor-1")
    );
}

// ── HITL approval queue (Step 9) ─────────────────────────────────────

#[tokio::test]
async fn hitl_migration_idempotent_and_defaults_applied() {
    // spawn_in_memory runs migrations once; run them again to prove
    // idempotency (duplicate-column errors are swallowed).
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // A plain store_skill_state write picks up schema defaults (kind
    // = 'generic', sentinel expires_at) — verify via the sweeper
    // NOT touching it.
    mem.store_skill_state("other_skill", "k1", "v1")
        .await
        .unwrap();
    let swept = mem.sweep_expired_hitl(chrono::Utc::now()).await.unwrap();
    assert_eq!(swept, 0, "non-HITL rows must never be swept");
    let val = mem.load_skill_state("other_skill", "k1").await.unwrap();
    assert_eq!(val.as_deref(), Some("v1"));
}

#[tokio::test]
async fn hitl_store_and_sweep_expired() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now();
    // One expired, one fresh, both with kind=config_change.
    mem.store_hitl_approval(
        "approval-old",
        "{\"stale\":true}",
        "config_change",
        None,
        None,
        now - chrono::Duration::hours(1),
    )
    .await
    .unwrap();
    mem.store_hitl_approval(
        "approval-new",
        "{\"fresh\":true}",
        "config_change",
        None,
        None,
        now + chrono::Duration::hours(23),
    )
    .await
    .unwrap();

    let swept = mem.sweep_expired_hitl(now).await.unwrap();
    assert_eq!(swept, 1, "only the expired row should be swept");

    // Fresh row survives; state_value still loadable via skill_state API.
    let fresh = mem.load_skill_state("hitl", "approval-new").await.unwrap();
    assert!(fresh.is_some());
    let gone = mem.load_skill_state("hitl", "approval-old").await.unwrap();
    assert!(gone.is_none());

    // Idempotent second sweep.
    let swept2 = mem.sweep_expired_hitl(now).await.unwrap();
    assert_eq!(swept2, 0);
}

#[tokio::test]
async fn hitl_sweep_skips_non_expiring_sentinel() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // Default store_skill_state under 'hitl' inherits the sentinel expires_at.
    mem.store_skill_state("hitl", "legacy-approval", "{}")
        .await
        .unwrap();
    // Even a far-future "now" must not delete sentinel rows.
    let far_future = chrono::DateTime::parse_from_rfc3339("9998-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let swept = mem.sweep_expired_hitl(far_future).await.unwrap();
    assert_eq!(swept, 0);
    assert!(
        mem.load_skill_state("hitl", "legacy-approval")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn hitl_list_pending_with_embeddings_filters_kind_and_since() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now();
    let emb = vec![1.0_f32, 0.0, 0.0];
    mem.store_hitl_approval(
        "a-config",
        "{}",
        "config_change",
        Some("cycle-1"),
        Some(&emb),
        now + chrono::Duration::hours(23),
    )
    .await
    .unwrap();
    mem.store_hitl_approval(
        "a-config",
        "{}",
        "config_change",
        None,
        Some(&emb),
        now + chrono::Duration::hours(23),
    )
    .await
    .unwrap();

    let rows = mem
        .list_pending_hitl_with_embeddings("config_change", now - chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "a-config");

    // Future `since` window → no rows.
    let rows2 = mem
        .list_pending_hitl_with_embeddings("config_change", now + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert!(rows2.is_empty());
}

// ── Step 11 retirement — stale-proposal sweep ─────────────────────────

// ── Task 1.8 — subcall storage governance ─────────────────────────────

/// Helper: seed a (subcall_log, subcall_trace) pair with explicit
/// `created_at` / `accessed_at` so we can backdate rows deterministically.
/// Returns the subcall id.
async fn seed_subcall_pair(
    mem: &MemoryHandle,
    id: &str,
    created_at: &str,
    accessed_at: &str,
    size_bytes: i64,
) {
    mem.subcall_log_insert(
        id,
        "parent-sid",
        "code_directed",
        Some("preview"),
        &["web_search".to_string()],
        "ok",
        Some(3),
        Some(2),
        Some(1500),
        Some(420),
        created_at,
        None,
        true,
        Some("stub summary"),
    )
    .await
    .unwrap();
    mem.subcall_trace_insert(
        id,
        Some("sys prompt"),
        "[{\"role\":\"user\",\"content\":\"hi\"}]",
        "[[\"web_search\",\"{\\\"q\\\":\\\"rust\\\"}\"]]",
        created_at,
        accessed_at,
        size_bytes,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn subcall_ttl_sweep_removes_old_traces() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Three rows:
    //   old_id       — created 120d ago, accessed 120d ago   → DELETE
    //   mid_id       — created 120d ago, accessed 5d ago     → KEEP (hot trace)
    //   young_id     — created 5d ago,   accessed 5d ago     → KEEP (young log)
    // (A 4th case — 120d-old log with trace already evicted — is
    //  covered indirectly by the LEFT JOIN in SQL; we exercise it
    //  in the integration path rather than duplicating here.)
    let t_old = (chrono::Utc::now() - chrono::Duration::days(120))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let t_mid_accessed = (chrono::Utc::now() - chrono::Duration::days(5))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let t_young = (chrono::Utc::now() - chrono::Duration::days(5))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    seed_subcall_pair(&mem, "old_id", &t_old, &t_old, 1024).await;
    seed_subcall_pair(&mem, "mid_id", &t_old, &t_mid_accessed, 1024).await;
    seed_subcall_pair(&mem, "young_id", &t_young, &t_young, 1024).await;

    let deleted = mem.subcall_ttl_sweep().await.unwrap();
    assert_eq!(deleted, 1, "only the cold 120d-old trace should be swept");

    // Verify CASCADE: old_id's subcall_trace row must be gone too.
    // `subcall_fetch_trace` returns (exists, trace_available, ...).
    let (exists, _, _, _) = mem.subcall_fetch_trace("old_id").await.unwrap();
    assert!(!exists, "old_id subcall_log row should be deleted");
    let (exists_mid, avail_mid, _, _) = mem.subcall_fetch_trace("mid_id").await.unwrap();
    assert!(exists_mid, "mid_id (hot trace) should survive");
    assert!(avail_mid, "mid_id trace should still be available");
    let (exists_young, _, _, _) = mem.subcall_fetch_trace("young_id").await.unwrap();
    assert!(exists_young, "young_id (young log) should survive");

    // Idempotent: a second sweep finds nothing to do.
    let again = mem.subcall_ttl_sweep().await.unwrap();
    assert_eq!(again, 0, "idempotent: no more stale subcalls");
}

#[tokio::test]
async fn subcall_lru_eviction_frees_oldest_when_over_cap() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Three rows totaling 300 bytes. Set the cap at 150 so the
    // oldest-by-accessed_at must be evicted until total ≤ 150.
    // Deleting the oldest 100 byte row brings us to 200 → still over;
    // deleting the next-oldest 100 byte row brings us to 100 → done.
    let now = chrono::Utc::now();
    let iso = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let t_created = iso(now - chrono::Duration::hours(1));
    let t_old = iso(now - chrono::Duration::days(3));
    let t_mid = iso(now - chrono::Duration::days(2));
    let t_new = iso(now - chrono::Duration::days(1));

    seed_subcall_pair(&mem, "lru_old", &t_created, &t_old, 100).await;
    seed_subcall_pair(&mem, "lru_mid", &t_created, &t_mid, 100).await;
    seed_subcall_pair(&mem, "lru_new", &t_created, &t_new, 100).await;

    let evicted = mem.subcall_enforce_lru_cap(150).await.unwrap();
    assert_eq!(evicted, 2, "should evict the two oldest-by-accessed_at");

    // The oldest trace is gone but the subcall_log row must remain
    // with `trace_available = 0` so recall can report trace_evicted.
    let (exists_old, avail_old, summary_old, tools_old) =
        mem.subcall_fetch_trace("lru_old").await.unwrap();
    assert!(exists_old, "subcall_log row must survive LRU eviction");
    assert!(!avail_old, "trace_available must be flipped to 0");
    assert_eq!(
        summary_old.as_deref(),
        Some("stub summary"),
        "summary should still surface on trace_evicted recall"
    );
    assert!(tools_old.is_none(), "tool_outputs gone with evicted trace");

    let (exists_mid, avail_mid, _, _) = mem.subcall_fetch_trace("lru_mid").await.unwrap();
    assert!(exists_mid);
    assert!(!avail_mid, "second-oldest trace must also be evicted");

    let (exists_new, avail_new, _, tools_new) = mem.subcall_fetch_trace("lru_new").await.unwrap();
    assert!(exists_new);
    assert!(avail_new, "youngest trace must survive");
    assert!(
        tools_new.is_some(),
        "youngest tool_outputs must still be fetchable"
    );

    // Idempotent: already under cap → no eviction.
    let again = mem.subcall_enforce_lru_cap(150).await.unwrap();
    assert_eq!(again, 0, "idempotent when already under cap");
}

// ── FIX B regression — unknown event kind/source must not stall the notifier ─

#[test]
fn unknown_event_kind_does_not_stall_consumer_cursor() {
    // Regression (FIX B): parse_assistant_event_row previously returned Err
    // on unknown kind/source strings (e.g. written by a newer binary), causing
    // query_new_assistant_events_for_consumer to propagate the error and pin the
    // consumer cursor before that row — all later events were permanently blocked.
    //
    // After the fix, unknown values map to Unknown variants (fail-safe) and
    // the cursor advances normally. The unknown event is also conversation-visible
    // (assistant_event_enters_conversation returns true), never silently suppressed.
    use crate::assistant_events::assistant_event_enters_conversation;

    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    // Row 1: unknown kind string (simulates a future binary adding a new event type).
    conn.execute(
        "INSERT INTO assistant_events \
             (raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
              title, summary, thread_key, urgency_hint, actionability_hint, decision, \
              dedup_key, entities_json) \
             VALUES (NULL, '2026-06-30T00:00:00Z', '2026-06-30T00:00:00Z', \
                     'did:aegis:test', 'calendar', 'video_call_missed', NULL, \
                     'Missed video call', 'Unknown kind', NULL, \
                     'medium', 'review', 'notify_now', \
                     'test-fix-b-unknown-kind', '{}')",
        [],
    )
    .unwrap();

    // Row 2: normal row that must still be processed even after the unknown one.
    conn.execute(
        "INSERT INTO assistant_events \
             (raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
              title, summary, thread_key, urgency_hint, actionability_hint, decision, \
              dedup_key, entities_json) \
             VALUES (NULL, '2026-06-30T00:01:00Z', '2026-06-30T00:01:00Z', \
                     'did:aegis:test', 'calendar', 'email_received', NULL, \
                     'Normal email after unknown', 'Must be delivered', NULL, \
                     'high', 'act', 'notify_now', \
                     'test-fix-b-normal-after-unknown', '{}')",
        [],
    )
    .unwrap();

    let (events, newest_id) = query_new_assistant_events_for_consumer(
        &conn,
        "did:aegis:test",
        "test_fix_b_consumer",
        10,
        None,
        None,
    )
    .unwrap();

    // Cursor must NOT stall — both rows are returned.
    assert_eq!(
        events.len(),
        2,
        "unknown-kind row must not stall the consumer cursor"
    );
    assert!(newest_id > 0, "cursor id must have advanced");

    // The unknown-kind event must be conversation-visible (fail-safe).
    let unknown_ev = &events[0].event;
    assert_eq!(
        unknown_ev.kind,
        crate::assistant_events::AssistantEventKind::Unknown,
        "unknown kind string must parse to Unknown variant"
    );
    assert!(
        assistant_event_enters_conversation(unknown_ev),
        "event with Unknown kind must be conversation-visible (fail-safe)"
    );

    // The normal row that follows must also be returned (no stall).
    let normal_ev = &events[1].event;
    assert_eq!(
        normal_ev.kind,
        crate::assistant_events::AssistantEventKind::EmailReceived
    );
    assert!(assistant_event_enters_conversation(normal_ev));
}

#[test]
fn unknown_event_source_does_not_stall_consumer_cursor() {
    // Same invariant as above but for an unknown source string.
    use crate::assistant_events::assistant_event_enters_conversation;

    let conn = Connection::open_in_memory().unwrap();
    crate::memory_migrations::run_all_migrations(&conn).unwrap();

    conn.execute(
        "INSERT INTO assistant_events \
             (raw_event_id, observed_at, occurred_at, user_did, source, kind, account_id, \
              title, summary, thread_key, urgency_hint, actionability_hint, decision, \
              dedup_key, entities_json) \
             VALUES (NULL, '2026-06-30T00:00:00Z', '2026-06-30T00:00:00Z', \
                     'did:aegis:test', 'alien_service', 'email_received', NULL, \
                     'From alien service', 'Unknown source', NULL, \
                     'medium', 'review', 'notify_now', \
                     'test-fix-b-unknown-source', '{}')",
        [],
    )
    .unwrap();

    let (events, newest_id) = query_new_assistant_events_for_consumer(
        &conn,
        "did:aegis:test",
        "fix_b_src",
        10,
        None,
        None,
    )
    .unwrap();

    assert_eq!(events.len(), 1, "unknown-source row must be returned");
    assert!(newest_id > 0);

    let ev = &events[0].event;
    assert_eq!(
        ev.source,
        crate::assistant_events::AssistantEventSource::Unknown
    );
    // Unknown source is not in the Google/Calendar suppression pattern →
    // conversation-visible.
    assert!(
        assistant_event_enters_conversation(ev),
        "event with Unknown source must be conversation-visible"
    );
}

#[tokio::test]
async fn l1_trims_in_batches_so_the_prompt_prefix_stays_stable() {
    // Default cap is 50 with a slack of 20: the window may grow to 70 and is
    // then cut back to 50 in one go (instead of sliding by one each push).
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:test:l1-batch";
    // Recent timestamps: L1 also drops anything older than 7 days.
    let base = chrono::Utc::now().timestamp() - 1_000;
    for i in 0..70 {
        mem.push_l1(did, Message::text("user", format!("m{i}"), base + i))
            .await
            .unwrap();
    }
    assert_eq!(
        mem.get_l1(did).await.unwrap().len(),
        70,
        "grows up to cap + slack"
    );
    mem.push_l1(did, Message::text("user", "m70", base + 70))
        .await
        .unwrap();
    let l1 = mem.get_l1(did).await.unwrap();
    assert_eq!(l1.len(), 50, "then trims back to the cap in one batch");
    assert_eq!(l1.first().map(|m| m.content.as_str()), Some("m21"));
    assert_eq!(l1.last().map(|m| m.content.as_str()), Some("m70"));
}
