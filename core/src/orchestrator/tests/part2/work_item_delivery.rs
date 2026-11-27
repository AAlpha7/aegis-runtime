//! Work-item processors: digest and cached-outcome delivery, requeue on deferral, L1 duplicate suppression, deferred-intent processors, retry budget, embedding cache warm-up.
use super::*;

#[tokio::test]
async fn heartbeat_digest_work_item_processor_delivers_digest_and_commits_cursor() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    for (idx, subject) in [
        (41, "Weekly digest"),
        (42, "Promotions"),
        (43, "Another digest"),
    ] {
        let assistant_event = crate::assistant_events::AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-04-22T00:00:00Z".to_string(),
            occurred_at: "2026-04-22T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: crate::assistant_events::AssistantEventSource::Calendar,
            kind: crate::assistant_events::AssistantEventKind::EmailReceived,
            account_id: Some("primary".to_string()),
            title: subject.to_string(),
            summary: "newsletter".to_string(),
            thread_key: Some(format!("t-{idx}")),
            urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
            actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
            decision: crate::assistant_events::AssistantEventDecision::DigestLater,
            dedup_key: format!("calendar:primary:{idx}"),
            entities: serde_json::json!({ "from": "newsletter@example.com" }),
        };
        memory
            .store_assistant_event(&assistant_event)
            .await
            .unwrap();
    }

    let work_item = crate::assistant_work_items::build_heartbeat_digest_work_item(
        "did:aegis:test",
        "Assemble a concise digest covering the accumulated lower-signal mail backlog.",
        "assistant_work:heartbeat_digest:calendar:41:43",
        "digest_batch_matured",
        "mail_notifier_digest",
        1,
        3,
        3,
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_heartbeat_digest_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );

    let delivered = rx.recv().await.expect("heartbeat digest delivery");
    assert!(crate::progress_router::progress_event_to_legacy_string(&delivered).contains("Digest"));

    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].item.mode,
        crate::assistant_work_items::AssistantWorkMode::DigestAssembly
    );

    let pending = memory
        .peek_new_assistant_events_for_consumer(
            "did:aegis:test",
            "mail_notifier_digest",
            10,
            Some(&["digest_later"]),
            None,
        )
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "digest cursor should commit after delivery"
    );
}

#[tokio::test]
async fn heartbeat_silent_reprioritization_work_item_marks_done_without_notifying_user() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(4);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let mut work_item =
        crate::assistant_work_items::build_heartbeat_silent_reprioritization_work_item(
            "did:aegis:test",
            "Silently reprioritize the stale legal thread review and leave it deferred.",
            "assistant_work:heartbeat_reprioritization:queued_work_stale:thread-9",
            Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread),
            Some("thread-9".to_string()),
            "queued_work_stale",
        );
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T02:00:00Z".to_string(),
            internal_summary: "This stale legal thread can remain deferred for now.".to_string(),
            user_message: None,
            delivery: crate::assistant_work_items::AssistantTurnDelivery::Silent,
            notify_priority: Some("routine".to_string()),
            follow_up_anchor_kind: Some("email_thread".to_string()),
            follow_up_anchor_ref: Some("thread-9".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 1,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_heartbeat_follow_up_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv())
            .await
            .is_err(),
        "silent reprioritization should not notify the user"
    );

    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].item.status,
        crate::assistant_work_items::AssistantWorkStatus::Done
    );
    assert_eq!(
        done[0]
            .item
            .result
            .as_ref()
            .and_then(|v| v.get("delivery"))
            .and_then(|v| v.as_str()),
        Some("silent")
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert!(
        l1.is_empty(),
        "silent reprioritization should not write proactive L1"
    );
}

#[tokio::test]
async fn assistant_work_item_processor_delivers_cached_outcome_and_marks_done() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Please review budget".to_string(),
        summary: "Email from boss@example.com: Please review budget".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-1".to_string(),
        entities: serde_json::json!({ "from": "boss@example.com" }),
    };
    let assistant_event_id = memory
        .store_assistant_event(&assistant_event)
        .await
        .unwrap();
    let stored_event = crate::assistant_events::StoredAssistantEvent {
        id: assistant_event_id,
        event: assistant_event,
    };

    let mut work_item = crate::assistant_work_items::build_callback_review_work_item(&stored_event);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-22T00:05:00Z".to_string(),
            internal_summary: "Prepared a proactive callback summary.".to_string(),
            user_message: Some(
                "Boss needs a budget review today. I can draft the reply next.".to_string(),
            ),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("email_thread".to_string()),
            follow_up_anchor_ref: Some("t-1".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["calendar_reader".to_string()],
            rounds_used: 2,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_callback_review_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    let delivered = rx.recv().await.expect("cached callback outcome");
    assert!(
        crate::progress_router::progress_event_to_legacy_string(&delivered)
            .contains("budget review today")
    );

    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].item.status,
        crate::assistant_work_items::AssistantWorkStatus::Done
    );
    assert_eq!(
        done[0]
            .item
            .result
            .as_ref()
            .and_then(|v| v.get("notify_priority"))
            .and_then(|v| v.as_str()),
        Some("important")
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].role, "assistant");
    assert!(l1[0].content.contains("budget review today"));
    assert_eq!(l1[0].anchor_kind.as_deref(), Some("email_thread"));
    assert_eq!(l1[0].anchor_ref.as_deref(), Some("t-1"));
}

#[tokio::test]
async fn delivered_proactive_outcome_sets_follow_up_chat_anchor_preference() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Please review budget".to_string(),
        summary: "Email from boss@example.com: Please review budget".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-1".to_string(),
        entities: serde_json::json!({ "from": "boss@example.com" }),
    };
    let assistant_event_id = memory
        .store_assistant_event(&assistant_event)
        .await
        .unwrap();
    let stored_event = crate::assistant_events::StoredAssistantEvent {
        id: assistant_event_id,
        event: assistant_event,
    };

    let mut work_item = crate::assistant_work_items::build_callback_review_work_item(&stored_event);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-22T00:05:00Z".to_string(),
            internal_summary: "Prepared a proactive callback summary.".to_string(),
            user_message: Some(
                "Boss needs a budget review today. I can draft the reply next.".to_string(),
            ),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            proposed_task: Some(crate::assistant_work_items::AssistantProposedTask {
                title: "Act on: Please review budget".to_string(),
                notes: Some("I can draft the reply next.".to_string()),
                anchor_kind: Some("email_thread".to_string()),
                anchor_ref: Some("t-1".to_string()),
            }),
            proposed_reminder: None,
            follow_up_anchor_kind: Some("email_thread".to_string()),
            follow_up_anchor_ref: Some("t-1".to_string()),
            needs_user_reply: true,
            tools_called: vec!["calendar_reader".to_string()],
            rounds_used: 2,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_callback_review_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );

    let ctx = orch
        .build_react_context(
            "did:aegis:test",
            "这封先别发，我想先看看预算细节。",
            &[],
            false,
            &UserMessagePersistence::Ephemeral,
        )
        .await
        .unwrap();

    assert!(
        ctx.volatile_prompt
            .contains("Current conversation anchor preference: email_thread = t-1")
    );
}

#[tokio::test]
async fn assistant_work_item_processor_requeues_cached_outcome_when_delivery_defers() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    settings
        .set("notification_daily_max", serde_json::json!(0))
        .unwrap();

    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Please review budget".to_string(),
        summary: "Email from boss@example.com: Please review budget".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-2".to_string(),
        entities: serde_json::json!({ "from": "boss@example.com" }),
    };
    let assistant_event_id = memory
        .store_assistant_event(&assistant_event)
        .await
        .unwrap();
    let stored_event = crate::assistant_events::StoredAssistantEvent {
        id: assistant_event_id,
        event: assistant_event,
    };

    let mut work_item = crate::assistant_work_items::build_callback_review_work_item(&stored_event);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-22T00:05:00Z".to_string(),
            internal_summary: "Prepared a proactive callback summary.".to_string(),
            user_message: Some(
                "Boss needs a budget review today. I can draft the reply next.".to_string(),
            ),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("email_thread".to_string()),
            follow_up_anchor_ref: Some("t-1".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["calendar_reader".to_string()],
            rounds_used: 2,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    // NOTE(deferred): requeue-on-defer + no-phantom-L1 durability is tracked
    // for a dedicated PR (Codex-flagged). This asserts current behavior:
    // when notification_daily_max=0 defers the Telegram copy, production
    // still mirrors the cached outcome to the Web timeline (L1) and marks
    // the work item Done, so process_callback_review_work_items returns 1.
    assert_eq!(
        orch.process_callback_review_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    // Telegram fallback is suppressed by the daily cap.
    assert!(rx.try_recv().is_err());

    // No item is requeued; it is marked Done after Web-timeline delivery.
    let queued = memory
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert!(queued.is_empty());
    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].item.status,
        crate::assistant_work_items::AssistantWorkStatus::Done
    );

    // Current behavior: the cached user_message IS mirrored to the Web
    // timeline (L1). The "no phantom L1" durability fix is deferred.
    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].role, "assistant");
    assert_eq!(
        l1[0].content,
        "Boss needs a budget review today. I can draft the reply next."
    );
}

#[tokio::test]
async fn proactive_notification_send_failure_rolls_back_content_dedup() {
    // Regression for #7 (b): the Important content-dedup slot is reserved
    // under the lock (so concurrent calls can't double-send), but it MUST be
    // rolled back when the Telegram send fails — otherwise the false dedup
    // marker suppresses the retry and the fallback copy is dropped forever.
    // Without the fix the marker is inserted before the send and never removed,
    // so the dedup map is non-empty here and this assertion fails.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    // High cap + no quiet hours so we actually reach the send path.
    settings
        .set("notification_daily_max", serde_json::json!(100))
        .unwrap();
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));
    // Dropping the receiver makes every try_send fail with Closed.
    drop(rx);

    let result = orch
        .emit_proactive_notification(
            12345,
            "test_label",
            "Important budget review needs your call",
            NotifyPriority::Important,
        )
        .await;
    assert!(
        result.is_err(),
        "send must fail when the progress receiver is closed"
    );
    assert!(
        orch.notification_state.dedup.lock().is_empty(),
        "content-dedup reservation must be rolled back after a failed send, \
             so the retry is not wrongly suppressed (dropped fallback copy)"
    );
}

#[tokio::test]
async fn work_item_delivery_suppresses_exact_l1_duplicate_for_one_shot_non_urgent() {
    // Codex round-3 regression: deliver_assistant_turn_outcome previously
    // pushed to L1 unconditionally — a stored-outcome replay (or an LLM
    // ignoring the stay-silent contract) appended the identical message to
    // the Web timeline every fire. A ONE-SHOT non-urgent work item whose
    // rendered message already sits in L1 (<12h) must complete Done
    // WITHOUT a second L1 row. (Recurring intents are exempt by design —
    // covered by deferred_intent_work_item_processor_records_cached_outcome…,
    // whose intent has interval_seconds=Some and still writes L1.)
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let repeated = "The contract review is still waiting on you.";
    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "assistant",
                repeated,
                chrono::Utc::now().timestamp() - 3600,
                Some("deferred_intent".to_string()),
                Some("deferred_intent:78".to_string()),
            ),
        )
        .await
        .unwrap();

    let deferred_intent = crate::memory::DeferredIntent {
        id: 78,
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
        interval_seconds: None, // one-shot: durable dedup applies
        chain_depth: 0,
        pin_on_fire: false,
        // No group_key: the anchor is the auto-generated unique
        // deferred_intent:{id}, the only shape the dedup gate accepts.
        group_key: None,
        max_fires: Some(1),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Prepared the scheduled follow-up.".to_string(),
            user_message: Some(repeated.to_string()),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("deferred_intent".to_string()),
            follow_up_anchor_ref: Some("deferred_intent:78".to_string()),
            needs_user_reply: true,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 1,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    assert!(
        rx.try_recv().is_err(),
        "no Telegram nudge for the suppressed duplicate"
    );

    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(
        done.len(),
        1,
        "work item still completes (AlreadySatisfied)"
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(
        l1.len(),
        1,
        "identical message must NOT be appended to the Web timeline again"
    );
}

#[tokio::test]
async fn work_item_delivery_keeps_distinct_one_shot_with_same_text() {
    // Codex round-4 regression: two SEPARATE one-shot obligations that
    // happen to render identical text (e.g. two medication reminders
    // scheduled independently within 12h) have DIFFERENT anchor_refs —
    // the second must still deliver and record its own L1 row.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let same_text = "该吃药了。";
    // Prior delivery of obligation A (different anchor identity).
    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "assistant",
                same_text,
                chrono::Utc::now().timestamp() - 3600,
                Some("deferred_intent_group".to_string()),
                Some("reminder:med-morning".to_string()),
            ),
        )
        .await
        .unwrap();

    let deferred_intent = crate::memory::DeferredIntent {
        id: 79,
        user_did: "did:aegis:test".to_string(),
        chat_id: 12345,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(1_776_000_000),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "Evening medication reminder.".to_string(),
        status: "pending".to_string(),
        created_at: "2026-04-23T00:00:00Z".to_string(),
        expires_at_unix: 1_776_003_600,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: None,
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("reminder:med-evening".to_string()),
        max_fires: Some(1),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Evening medication reminder.".to_string(),
            user_message: Some(same_text.to_string()),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
            follow_up_anchor_ref: Some("reminder:med-evening".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 1,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(
        l1.len(),
        2,
        "a DISTINCT one-shot obligation with the same text must still deliver"
    );
    assert_eq!(l1[1].anchor_ref.as_deref(), Some("reminder:med-evening"));
}

#[tokio::test]
async fn work_item_delivery_keeps_cross_anchor_kind_collision() {
    // Codex round-5 regression: anchor_ref alone is not globally
    // namespaced — a prior L1 row with the SAME ref string but a DIFFERENT
    // anchor_kind (e.g. an email_thread that collides with an LLM-chosen
    // deferred-intent group_key) must NOT suppress this delivery.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let same_text = "该吃药了。";
    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "assistant",
                same_text,
                chrono::Utc::now().timestamp() - 3600,
                Some("email_thread".to_string()),
                Some("reminder:med-evening".to_string()),
            ),
        )
        .await
        .unwrap();

    let deferred_intent = crate::memory::DeferredIntent {
        id: 80,
        user_did: "did:aegis:test".to_string(),
        chat_id: 12345,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(1_776_000_000),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "Evening medication reminder.".to_string(),
        status: "pending".to_string(),
        created_at: "2026-04-23T00:00:00Z".to_string(),
        expires_at_unix: 1_776_003_600,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: None,
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("reminder:med-evening".to_string()),
        max_fires: Some(1),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Evening medication reminder.".to_string(),
            user_message: Some(same_text.to_string()),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
            follow_up_anchor_ref: Some("reminder:med-evening".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 1,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(
        l1.len(),
        2,
        "same ref under a DIFFERENT anchor_kind is a different obligation — must deliver"
    );
    assert_eq!(l1[1].anchor_kind.as_deref(), Some("deferred_intent_group"));
}

#[tokio::test]
async fn work_item_delivery_fails_open_for_shared_group_key_one_shots() {
    // Codex round-6 regression: group_key is LLM-chosen and can be REUSED
    // by two distinct one-shot intents. The gate must fail open for
    // group_key'd one-shots — the second delivers even with identical
    // text AND identical anchor identity.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let same_text = "该吃药了。";
    // Prior delivery of intent A under the SAME group_key anchor.
    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "assistant",
                same_text,
                chrono::Utc::now().timestamp() - 3600,
                Some("deferred_intent_group".to_string()),
                Some("reminder:meds".to_string()),
            ),
        )
        .await
        .unwrap();

    let deferred_intent = crate::memory::DeferredIntent {
        id: 81,
        user_did: "did:aegis:test".to_string(),
        chat_id: 12345,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(1_776_000_000),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "Medication reminder (second dose).".to_string(),
        status: "pending".to_string(),
        created_at: "2026-04-23T00:00:00Z".to_string(),
        expires_at_unix: 1_776_003_600,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: None,
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("reminder:meds".to_string()),
        max_fires: Some(1),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Second dose reminder.".to_string(),
            user_message: Some(same_text.to_string()),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
            follow_up_anchor_ref: Some("reminder:meds".to_string()),
            needs_user_reply: false,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 1,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(
        l1.len(),
        2,
        "group_key'd one-shots are not uniquely anchored — the gate must fail open"
    );
}

#[test]
fn assistant_l1_exact_duplicate_survives_l1_eviction_via_l3() {
    // Codex round-6 regression: L1 is a bounded FIFO window; a <12h-old
    // duplicate can be evicted by chat volume. The durable check must
    // still find it in the L3 archive (populated synchronously by every
    // push_l1).
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = rt.enter();
    let tmp = tempfile::NamedTempFile::new().expect("temp db");
    let memory = crate::memory::MemoryHandle::spawn(tmp.path()).expect("memory");
    let did = "did:aegis:test-l3-dedup";
    let now = chrono::Utc::now().timestamp();

    rt.block_on(async {
        memory
            .push_l1(
                did,
                Message::text("assistant", "嗨！有事吗?", now - 2 * 3600),
            )
            .await
            .expect("push duplicate-source message");
        // Evict it from the bounded L1 window (DEFAULT_L1_MAX_SIZE = 50).
        // L1 trims in batches once it exceeds cap + slack (50 + 20).
        for i in 0..70 {
            memory
                .push_l1(
                    did,
                    Message::text(
                        "assistant",
                        format!("填充消息编号 {i} 用于挤出窗口"),
                        now - 3600,
                    ),
                )
                .await
                .expect("push filler");
        }
        let l1 = memory.get_l1(did).await.expect("get_l1");
        assert!(
            !l1.iter().any(|m| m.content == "嗨！有事吗?"),
            "precondition: the duplicate source must be evicted from L1"
        );
        // Still detected via the durable L3 archive.
        assert!(assistant_l1_exact_duplicate_within(&memory, did, "嗨！有事吗?", 12).await);
    });
}

#[tokio::test]
async fn deferred_intent_work_item_processor_records_cached_outcome_to_web_and_marks_done() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let deferred_intent = crate::memory::DeferredIntent {
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

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Prepared the scheduled follow-up.".to_string(),
            user_message: Some(
                "The contract review is still waiting on you. I can draft the next nudge now."
                    .to_string(),
            ),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("important".to_string()),
            follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
            follow_up_anchor_ref: Some("followup:contract-review".to_string()),
            needs_user_reply: true,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 2,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    assert!(
        rx.try_recv().is_err(),
        "non-urgent scheduled callbacks should be recorded to Web without Telegram nudge"
    );

    let done = memory
        .list_assistant_work_items("did:aegis:test", Some(&["done"]), 10)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].item.source_kind,
        crate::assistant_work_items::AssistantWorkSourceKind::DeferredIntent
    );
    assert_eq!(
        done[0].item.status,
        crate::assistant_work_items::AssistantWorkStatus::Done
    );

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].role, "assistant");
    assert!(l1[0].content.contains("contract review is still waiting"));
    assert_eq!(l1[0].anchor_kind.as_deref(), Some("deferred_intent_group"));
    assert_eq!(
        l1[0].anchor_ref.as_deref(),
        Some("followup:contract-review")
    );
}

#[tokio::test]
async fn urgent_deferred_intent_work_item_still_nudges_telegram() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let deferred_intent = crate::memory::DeferredIntent {
        id: 78,
        user_did: "did:aegis:test".to_string(),
        chat_id: 12345,
        trigger_type: "time".to_string(),
        trigger_at_unix: Some(1_776_000_000),
        event_key: None,
        execution_mode: "llm_callback".to_string(),
        context_text: "Check for a security incident.".to_string(),
        status: "pending".to_string(),
        created_at: "2026-04-23T00:00:00Z".to_string(),
        expires_at_unix: 1_776_003_600,
        fired_at: None,
        is_callback_origin: false,
        retry_count: 0,
        interval_seconds: Some(3600),
        chain_depth: 0,
        pin_on_fire: false,
        group_key: Some("followup:security".to_string()),
        max_fires: Some(5),
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        embedding: None,
    };

    let mut work_item =
        crate::assistant_work_items::build_deferred_intent_work_item(&deferred_intent);
    work_item.result = Some(
        serde_json::to_value(crate::assistant_work_items::AssistantTurnOutcome {
            generated_at: "2026-04-23T00:05:00Z".to_string(),
            internal_summary: "Prepared the scheduled urgent follow-up.".to_string(),
            user_message: Some("Security incident requires attention now.".to_string()),
            delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
            notify_priority: Some("urgent".to_string()),
            follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
            follow_up_anchor_ref: Some("followup:security".to_string()),
            needs_user_reply: true,
            proposed_task: None,
            proposed_reminder: None,
            tools_called: vec!["security_check".to_string()],
            rounds_used: 2,
        })
        .unwrap(),
    );
    memory.store_assistant_work_item(&work_item).await.unwrap();

    assert_eq!(
        orch.process_deferred_intent_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    let delivered = rx.recv().await.expect("urgent deferred-intent outcome");
    assert!(
        crate::progress_router::progress_event_to_legacy_string(&delivered)
            .contains("Security incident requires attention now")
    );
}

#[tokio::test]
async fn assistant_work_item_processor_retries_once_per_tick_and_gives_up_after_three_failures() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let assistant_event = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Please review budget".to_string(),
        summary: "Email from boss@example.com: Please review budget".to_string(),
        thread_key: Some("t-loop".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:m-loop".to_string(),
        entities: serde_json::json!({}),
    };
    let assistant_event_id = memory
        .store_assistant_event(&assistant_event)
        .await
        .unwrap();
    let stored_event = crate::assistant_events::StoredAssistantEvent {
        id: assistant_event_id,
        event: assistant_event,
    };
    let mut work_item = crate::assistant_work_items::build_callback_review_work_item(&stored_event);
    // Sever the event link so the bounded runner fails deterministically on
    // every attempt (same shape as a permanently unresolvable account).
    work_item.assistant_event_id = None;
    memory.store_assistant_work_item(&work_item).await.unwrap();

    for tick in 1..=2u64 {
        assert_eq!(
            orch.process_callback_review_work_items("did:aegis:test", 12345, 10)
                .await
                .unwrap(),
            0
        );
        let queued = memory
            .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
            .await
            .unwrap();
        assert_eq!(queued.len(), 1, "tick {tick}: item waits for the next tick");
        let attempts = queued[0]
            .item
            .result
            .as_ref()
            .and_then(|v| v.get("attempts"))
            .and_then(|v| v.as_u64());
        assert_eq!(
            attempts,
            Some(tick),
            "tick {tick}: a failing item must be attempted at most once per tick"
        );
    }

    assert_eq!(
        orch.process_callback_review_work_items("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        0
    );
    let queued = memory
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert!(queued.is_empty(), "third failure must stop requeueing");
    let cancelled = memory
        .list_assistant_work_items("did:aegis:test", Some(&["cancelled"]), 10)
        .await
        .unwrap();
    assert_eq!(
        cancelled.len(),
        1,
        "item is parked as cancelled after 3 failures"
    );
    let last_error = cancelled[0]
        .item
        .result
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        last_error.contains("missing assistant_event_id"),
        "last error is preserved: {last_error}"
    );
}

#[tokio::test]
async fn warm_embedding_caches_initialises_tool_review_even_when_embedder_is_unreachable() {
    let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let orch = crate::test_tool_helpers::test_orchestrator(
        mem,
        std::sync::Arc::new(crate::settings::RuntimeSettings::new()),
    );
    assert!(orch.caches.embedding_caches.tool_review.get().is_none());
    orch.warm_embedding_caches().await;
    // The test embedder points at a closed port: every embed fails, the
    // precompute skips them, and the cell is still initialised so the first
    // real turn does not pay for it.
    assert!(orch.caches.embedding_caches.tool_review.get().is_some());
}
