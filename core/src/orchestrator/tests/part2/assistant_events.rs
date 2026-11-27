//! Assistant event notifier and work-item outcome mapping (callback, deferred intent, heartbeat follow-up), delivery message rendering.
use super::*;

#[tokio::test]
async fn persist_assistant_event_enqueues_callback_review_work_item() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);

    let (raw, mut assistant_event) = crate::assistant_events::build_calendar_event_changed_event(
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
    assistant_event.decision = crate::assistant_events::AssistantEventDecision::CallbackNow;

    orch.persist_assistant_event(&raw, &assistant_event)
        .await
        .unwrap();

    let queued = memory
        .list_assistant_work_items("did:aegis:test", Some(&["queued"]), 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].item.mode,
        crate::assistant_work_items::AssistantWorkMode::BackgroundReview
    );
    assert_eq!(
        queued[0].item.assistant_event_id,
        queued[0]
            .item
            .context
            .get("assistant_event_id")
            .and_then(|v| v.as_i64())
    );
}

#[tokio::test]
async fn assistant_event_notifier_cursor_suppresses_repeat_delivery() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    // Use a security-alert event (urgency=High) so it passes the de-noise gate
    // and enters the conversation; the test's purpose is to verify the cursor
    // advances and prevents re-delivery, not to test routing.
    let ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Security alert: new sign-in detected".to_string(),
        summary: "New sign-in from unrecognised device".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:primary:notify-now-quiet-hours".to_string(),
        entities: serde_json::json!({ "from": "security@example.com" }),
    };
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    let delivered = rx.recv().await.expect("first notification");
    assert!(
        crate::progress_router::progress_event_to_legacy_string(&delivered).contains("[primary]")
    );
    assert!(!crate::progress_router::progress_event_to_legacy_string(&delivered).is_empty());
    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].role, "assistant");
    assert!(l1[0].content.contains("[primary]"));
    assert_eq!(l1[0].anchor_kind.as_deref(), Some("assistant_event"));
    assert_eq!(l1[0].anchor_ref.as_deref(), Some("assistant_event:1"));

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        0
    );
    assert!(rx.try_recv().is_err(), "repeat delivery must be suppressed");
}

#[tokio::test]
async fn assistant_event_notifier_commits_after_web_delivery_when_telegram_daily_cap_suppresses() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    settings
        .set("notification_daily_max", serde_json::json!(0))
        .unwrap();

    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    // Use an actionable email (medium urgency, actionability_hint=Act) so it
    // passes the de-noise gate (Act always enters conversation) but remains
    // NotifyPriority::Important (not Urgent) — the daily cap applies to Important.
    // This tests that L1 is written even when Telegram is capped.
    // NOTE: We use Act actionability, not S3 sensitivity, to pass the gate.
    // S3-sensitive routine NotifyNow mail is suppressed by the de-noise classifier
    // per the 2026-06-30 owner decision.
    let ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Please approve the expense report".to_string(),
        summary: "Action required: expense approval pending".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:primary:notify-now-daily-cap".to_string(),
        entities: serde_json::json!({
            "from": "manager@company.example.com"
        }),
    };
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        1
    );
    assert!(
        rx.try_recv().is_err(),
        "daily cap should suppress Telegram only"
    );
    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 1);
    assert!(l1[0].content.contains("[primary]"));

    let still_pending = memory
        .peek_new_assistant_events_for_consumer(
            "did:aegis:test",
            "mail_notifier_immediate",
            10,
            Some(&["notify_now"]),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        still_pending.len(),
        0,
        "daily-cap suppression should still commit after Web timeline delivery"
    );
}

#[tokio::test]
async fn assistant_event_notifier_falls_back_to_telegram_when_web_open_but_no_push_delivered() {
    // New contract: Telegram is suppressed only when a REAL web push was
    // delivered, not merely because a web session is open. Here a web
    // session is marked open but NO push subscription is registered, so no
    // real push can be delivered -> Telegram fallback MUST fire.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let user_did = "did:aegis:test";
    // Web session open, but no push subscription registered.
    crate::owner_presence::mark_owner_web_presence(&memory, user_did, "webapp")
        .await
        .unwrap();

    // Use a security-alert event (urgency=High) so it passes the de-noise gate;
    // this test's purpose is that with no real push subscription, Telegram
    // fallback fires (not the Medium/routine de-noise gate).
    let ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-04-22T00:00:00Z".to_string(),
        occurred_at: "2026-04-22T00:00:00Z".to_string(),
        user_did: user_did.to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Security alert: sign-in attempt".to_string(),
        summary: "Unrecognised device login".to_string(),
        thread_key: Some("t-1".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:primary:notify-now-web-active".to_string(),
        entities: serde_json::json!({ "from": "security@example.com" }),
    };
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications(user_did, 12345, 10)
            .await
            .unwrap(),
        1
    );
    let delivered = rx
        .recv()
        .await
        .expect("no real web push delivered -> Telegram fallback must fire");
    assert!(
        crate::progress_router::progress_event_to_legacy_string(&delivered).contains("[primary]")
    );
    let l1 = memory.get_l1(user_did).await.unwrap();
    assert_eq!(l1.len(), 1);
    assert!(l1[0].content.contains("[primary]"));

    let still_pending = memory
        .peek_new_assistant_events_for_consumer(
            user_did,
            "mail_notifier_immediate",
            10,
            Some(&["notify_now"]),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        still_pending.len(),
        0,
        "delivery should still advance the event cursor"
    );
}
#[tokio::test]
async fn unknown_decision_not_filtered_by_notifier_sql() {
    // Regression: an assistant event with an unrecognised decision string
    // (deserialized as AssistantEventDecision::Unknown by the #[serde(other)]
    // variant) must NOT be silently dropped by the immediate-notifier SQL filter.
    // The SQL filter uses NOT IN to exclude known non-immediate decisions;
    // Unknown is absent from that exclusion list so it is fetched and reaches
    // the conversation gate (fail-open: unknown decisions are delivered).
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    // Store an event with Unknown decision (simulates an unrecognised future decision
    // string that was round-tripped through the DB). assistant_event_enters_conversation
    // returns true for Unknown decision (fail-open), so it should be delivered.
    let ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:00:00Z".to_string(),
        occurred_at: "2026-06-30T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Unknown-decision email".to_string(),
        summary: "Some email with an unrecognised decision".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::Unknown,
        dedup_key: "test:unknown-decision-notifier-sql".to_string(),
        entities: serde_json::json!({}),
    };
    memory.store_assistant_event(&ev).await.unwrap();

    // Before fix: query filtered to decision='notify_now' → 0 events fetched → 0 processed.
    // After fix: no SQL decision filter → Unknown decision fetched → enters conversation → 1.
    let delivered = orch
        .process_assistant_event_notifications("did:aegis:test", 12345, 10)
        .await
        .unwrap();
    assert_eq!(
        delivered, 1,
        "Unknown decision must not be filtered out by the immediate-notifier query"
    );
}

#[tokio::test]
async fn test_notify_now_not_starved_behind_digest_later_backlog() {
    // Regression: when there are more `digest_later` rows than the fetch LIMIT
    // ahead of a `notify_now` row in the DB, the `notify_now` row must still be
    // delivered (not starved). The SQL NOT IN filter excludes `digest_later` rows
    // from the immediate-notifier's page so they do not consume the limit.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(16);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    // Insert MORE `digest_later` rows than the fetch limit (limit=5 below).
    // These rows have lower IDs than the notify_now row that follows.
    for i in 0..7 {
        let ev = crate::assistant_events::AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-06-30T00:00:00Z".to_string(),
            occurred_at: "2026-06-30T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: crate::assistant_events::AssistantEventSource::Calendar,
            kind: crate::assistant_events::AssistantEventKind::EmailReceived,
            account_id: Some("primary".to_string()),
            title: format!("Newsletter {i}"),
            summary: format!("Promotional email {i}"),
            thread_key: Some(format!("t-digest-{i}")),
            urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
            actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
            decision: crate::assistant_events::AssistantEventDecision::DigestLater,
            dedup_key: format!("calendar:primary:starvation-digest-{i}"),
            entities: serde_json::json!({}),
        };
        memory.store_assistant_event(&ev).await.unwrap();
    }

    // Now insert ONE notify_now row (higher ID, behind the digest backlog).
    let notify_ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:00:00Z".to_string(),
        occurred_at: "2026-06-30T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Security alert: new sign-in detected".to_string(),
        summary: "Unrecognised sign-in from new device".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:primary:starvation-notify-now".to_string(),
        entities: serde_json::json!({ "from": "security@example.com" }),
    };
    memory.store_assistant_event(&notify_ev).await.unwrap();

    // Call with limit=5: without the fix, the first 5 digest_later rows would
    // consume the page and the notify_now row would never be reached.
    let processed = orch
        .process_assistant_event_notifications("did:aegis:test", 12345, 5)
        .await
        .unwrap();

    assert_eq!(
        processed, 1,
        "notify_now row must be delivered even when more digest_later rows than the limit precede it"
    );
    let _ = rx
        .recv()
        .await
        .expect("Telegram notification must fire for the notify_now event");
}
#[tokio::test]
async fn denoise_routine_calendar_suppressed_no_l1_push() {
    // Routine Calendar event → suppressed from chat, cursor advances.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:00:00Z".to_string(),
        occurred_at: "2026-06-30T00:00:00Z".to_string(),
        user_did: "did:aegis:test".to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
        account_id: Some("primary".to_string()),
        title: "Team meeting updated".to_string(),
        summary: "Time changed to 3pm".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:primary:routine-cal-denoise".to_string(),
        entities: serde_json::json!({}),
    };
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        0,
        "routine calendar event must not count as delivered"
    );
    assert!(
        rx.try_recv().is_err(),
        "routine calendar must not trigger Telegram"
    );
    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert_eq!(l1.len(), 0, "routine calendar must not appear in L1 chat");

    // Cursor must advance
    let still_pending = memory
        .peek_new_assistant_events_for_consumer(
            "did:aegis:test",
            "mail_notifier_immediate",
            10,
            Some(&["notify_now"]),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        still_pending.len(),
        0,
        "cursor must advance past routine calendar event"
    );
}
#[test]
fn callback_result_to_work_item_outcome_prefers_notification_delivery() {
    let assistant_event = crate::assistant_events::StoredAssistantEvent {
        id: 42,
        event: crate::assistant_events::AssistantEvent {
            raw_event_id: Some(7),
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
        },
    };
    let result = OrchestratorResult {
        reply: "I drafted the next step.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "Boss needs a budget review today. I can draft the reply next.".to_string(),
            pin: false,
            priority: NotifyPriority::Important,
            source_tools: vec!["calendar_reader".to_string()],
        }],
        tools_called: vec!["calendar_reader".to_string()],
        rounds_used: 2,
        ..Default::default()
    };

    let outcome = Orchestrator::callback_result_to_work_item_outcome(&assistant_event, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
    );
    assert_eq!(outcome.notify_priority.as_deref(), Some("important"));
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("email_thread")
    );
    assert_eq!(outcome.follow_up_anchor_ref.as_deref(), Some("t-1"));
    assert_eq!(
        outcome
            .proposed_task
            .as_ref()
            .map(|task| task.title.as_str()),
        Some("Act on: Please review budget")
    );
    assert!(outcome.needs_user_reply);
    assert!(
        outcome
            .user_message
            .as_deref()
            .unwrap_or_default()
            .contains("budget review today")
    );
    assert_eq!(outcome.tools_called, vec!["calendar_reader".to_string()]);
    assert_eq!(outcome.rounds_used, 2);
}

#[test]
fn deferred_intent_result_to_work_item_outcome_prefers_notification_delivery() {
    let intent_context = serde_json::json!({
        "intent_id": 77,
        "context_text": "Follow up on the contract review.",
        "trigger_at_unix": 1_776_000_000,
        "anchor_kind": "deferred_intent_group",
        "anchor_ref": "followup:contract-review",
    });
    let result = OrchestratorResult {
        reply: "I checked the scheduled follow-up.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "The contract review is still waiting on you. I can draft the next nudge now."
                .to_string(),
            pin: false,
            priority: NotifyPriority::Important,
            source_tools: vec!["task_lookup".to_string()],
        }],
        tools_called: vec!["task_lookup".to_string()],
        rounds_used: 2,
        ..Default::default()
    };

    let outcome =
        Orchestrator::deferred_intent_result_to_work_item_outcome(&intent_context, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
    );
    assert_eq!(outcome.notify_priority.as_deref(), Some("important"));
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("deferred_intent_group")
    );
    assert_eq!(
        outcome.follow_up_anchor_ref.as_deref(),
        Some("followup:contract-review")
    );
    assert!(outcome.needs_user_reply);
    assert_eq!(
        outcome
            .proposed_task
            .as_ref()
            .map(|task| task.title.as_str()),
        Some("Follow up on the contract review")
    );
    assert_eq!(
        outcome
            .proposed_task
            .as_ref()
            .and_then(|task| task.anchor_kind.as_deref()),
        Some("deferred_intent_group")
    );
    assert_eq!(
        outcome
            .proposed_reminder
            .as_ref()
            .map(|reminder| reminder.title.as_str()),
        Some("Follow up on the contract review")
    );
    assert_eq!(
        outcome
            .proposed_reminder
            .as_ref()
            .and_then(|reminder| reminder.trigger_at.as_deref()),
        Some("2026-04-12T13:20:00+00:00")
    );
    assert!(
        outcome
            .user_message
            .as_deref()
            .unwrap_or_default()
            .contains("still waiting on you")
    );
    assert_eq!(outcome.tools_called, vec!["task_lookup".to_string()]);
    assert_eq!(outcome.rounds_used, 2);
}

#[test]
fn deferred_intent_result_to_work_item_outcome_keeps_routine_priority_routine() {
    let intent_context = serde_json::json!({
        "intent_id": 79,
        "context_text": "Search for latest news about openclaw.",
        "trigger_at_unix": 1_776_000_000,
        "anchor_kind": "deferred_intent_group",
        "anchor_ref": "followup:openclaw-watch",
    });
    let result = OrchestratorResult {
        reply: "I checked the scheduled OpenClaw watch.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "OpenClaw has a routine beta update.".to_string(),
            pin: false,
            priority: NotifyPriority::Routine,
            source_tools: vec!["web_search".to_string()],
        }],
        tools_called: vec!["web_search".to_string()],
        rounds_used: 2,
        ..Default::default()
    };

    let outcome =
        Orchestrator::deferred_intent_result_to_work_item_outcome(&intent_context, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
    );
    assert_eq!(outcome.notify_priority.as_deref(), Some("routine"));
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("deferred_intent_group")
    );
    assert_eq!(
        outcome.follow_up_anchor_ref.as_deref(),
        Some("followup:openclaw-watch")
    );
}

#[test]
fn deferred_intent_outcome_recurring_suppresses_self_reproposal() {
    // IMG_3939: a recurring intent re-proposed ITSELF as 建议任务 + 建议提醒
    // on every daily delivery. Recurrence is scheduler-owned; these fields
    // are render-only, so a recurring intent must not emit them.
    let intent_context = serde_json::json!({
        "intent_id": 88,
        "context_text": "总结本日学习到的新知识,发送总结报告给用户。",
        "interval_seconds": 86_400,
        "trigger_at_unix": 1_776_000_000,
        "anchor_kind": "deferred_intent_group",
        "anchor_ref": "recurring:daily-summary",
    });
    let result = OrchestratorResult {
        reply: "Summarized the day.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "今日无新日历事件或关键更新。".to_string(),
            pin: false,
            priority: NotifyPriority::Routine,
            source_tools: vec!["recall_memory".to_string()],
        }],
        tools_called: vec!["recall_memory".to_string()],
        rounds_used: 1,
        ..Default::default()
    };

    let outcome =
        Orchestrator::deferred_intent_result_to_work_item_outcome(&intent_context, &result);
    assert!(
        outcome.proposed_task.is_none(),
        "recurring intent must not re-propose itself as a task"
    );
    assert!(
        outcome.proposed_reminder.is_none(),
        "recurring intent must not re-propose itself as a reminder"
    );
    // The one-shot path keeps proposals — covered by
    // deferred_intent_result_to_work_item_outcome_prefers_notification_delivery.
}

#[test]
fn format_delivery_trigger_time_localizes_and_fails_safe() {
    assert_eq!(
        Orchestrator::format_delivery_trigger_time(
            "2026-06-28T05:27:19+00:00",
            "America/Los_Angeles"
        ),
        "2026-06-27 22:27 PDT"
    );
    // Unparseable timestamp or timezone → raw passthrough, never dropped.
    assert_eq!(
        Orchestrator::format_delivery_trigger_time("tomorrow morning", "America/Los_Angeles"),
        "tomorrow morning"
    );
    assert_eq!(
        Orchestrator::format_delivery_trigger_time("2026-06-28T05:27:19+00:00", "Not/AZone"),
        "2026-06-28T05:27:19+00:00"
    );
}

#[test]
fn assistant_l1_exact_duplicate_within_matches_only_recent_assistant_repeats() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = rt.enter();
    let tmp = tempfile::NamedTempFile::new().expect("temp db");
    let memory = crate::memory::MemoryHandle::spawn(tmp.path()).expect("memory");
    let did = "did:aegis:test-dedup";
    let now = chrono::Utc::now().timestamp();

    rt.block_on(async {
        memory
            .push_l1(
                did,
                Message::text("assistant", "嗨！有事吗?", now - 2 * 3600),
            )
            .await
            .expect("push recent assistant message");
        memory
            .push_l1(
                did,
                Message::text("assistant", "旧消息内容", now - 13 * 3600),
            )
            .await
            .expect("push old assistant message");
        memory
            .push_l1(did, Message::text("user", "用户说的话", now - 3600))
            .await
            .expect("push user message");

        // Identical assistant text within 12h → duplicate.
        assert!(assistant_l1_exact_duplicate_within(&memory, did, "嗨！有事吗?", 12).await);
        // Same text but the only prior copy is OUTSIDE the window → not a duplicate.
        assert!(!assistant_l1_exact_duplicate_within(&memory, did, "旧消息内容", 12).await);
        // User-authored identical text must never suppress an assistant send.
        assert!(!assistant_l1_exact_duplicate_within(&memory, did, "用户说的话", 12).await);
        // New content → not a duplicate.
        assert!(!assistant_l1_exact_duplicate_within(&memory, did, "全新的内容", 12).await);
        // Numeric delta = NEW information; the durable gate must be strict
        // (digits preserved), unlike the fuzzy in-memory normalize_for_dedup.
        memory
            .push_l1(
                did,
                Message::text("assistant", "BTC $65,000 现价", now - 3600),
            )
            .await
            .expect("push numeric message");
        assert!(!assistant_l1_exact_duplicate_within(&memory, did, "BTC $65,001 现价", 12).await);
        assert!(assistant_l1_exact_duplicate_within(&memory, did, "BTC  $65,000\n现价", 12).await);
        // Blank content never dedups.
        assert!(!assistant_l1_exact_duplicate_within(&memory, did, "   ", 12).await);
    });
}

#[test]
fn render_assistant_turn_delivery_message_appends_structured_proposals() {
    let outcome = crate::assistant_work_items::AssistantTurnOutcome {
        generated_at: "2026-04-23T01:00:00Z".to_string(),
        internal_summary: "Prepared follow-up suggestions.".to_string(),
        user_message: Some("The contract review is still waiting on you.".to_string()),
        delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
        notify_priority: Some("important".to_string()),
        proposed_task: Some(crate::assistant_work_items::AssistantProposedTask {
            title: "Follow up on the contract review".to_string(),
            notes: Some("I can draft the next nudge now.".to_string()),
            anchor_kind: Some("deferred_intent_group".to_string()),
            anchor_ref: Some("followup:contract-review".to_string()),
        }),
        proposed_reminder: Some(crate::assistant_work_items::AssistantProposedReminder {
            title: "Decide on the contract follow-up".to_string(),
            notes: Some("If you want, I can remind you again tomorrow.".to_string()),
            trigger_at: Some("2026-04-24T09:00:00Z".to_string()),
            anchor_kind: Some("deferred_intent_group".to_string()),
            anchor_ref: Some("followup:contract-review".to_string()),
        }),
        follow_up_anchor_kind: Some("deferred_intent_group".to_string()),
        follow_up_anchor_ref: Some("followup:contract-review".to_string()),
        needs_user_reply: true,
        tools_called: vec!["task_lookup".to_string()],
        rounds_used: 2,
    };

    let rendered =
        Orchestrator::render_assistant_turn_delivery_message(&outcome, "America/Los_Angeles")
            .expect("rendered delivery message");
    assert!(rendered.contains("The contract review is still waiting on you."));
    assert!(rendered.contains("建议任务：Follow up on the contract review"));
    assert!(rendered.contains("I can draft the next nudge now."));
    assert!(rendered.contains("建议提醒：Decide on the contract follow-up"));
    // 2026-04-24T09:00:00Z = 02:00 PDT — rendered in the owner's timezone,
    // not the raw RFC3339 UTC internal format (IMG_3939).
    assert!(rendered.contains("时间：2026-04-24 02:00 PDT"));
    assert!(!rendered.contains("When:"));
    assert!(!rendered.contains("2026-04-24T09:00:00Z"));
}

#[test]
fn render_assistant_turn_delivery_message_omits_duplicate_proposal_notes() {
    let shared_message = "Grok Build has a meaningful new beta update.";
    let outcome = crate::assistant_work_items::AssistantTurnOutcome {
        generated_at: "2026-05-26T16:00:00Z".to_string(),
        internal_summary: "Prepared Grok Build follow-up.".to_string(),
        user_message: Some(shared_message.to_string()),
        delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
        notify_priority: Some("important".to_string()),
        proposed_task: Some(crate::assistant_work_items::AssistantProposedTask {
            title: "Search for Grok Build updates".to_string(),
            notes: Some(shared_message.to_string()),
            anchor_kind: Some("assistant_event".to_string()),
            anchor_ref: Some("event:grok-build".to_string()),
        }),
        proposed_reminder: Some(crate::assistant_work_items::AssistantProposedReminder {
            title: "Check Grok Build again".to_string(),
            notes: Some(format!("  {shared_message}\n")),
            trigger_at: Some("2026-05-26T16:01:50Z".to_string()),
            anchor_kind: Some("assistant_event".to_string()),
            anchor_ref: Some("event:grok-build".to_string()),
        }),
        follow_up_anchor_kind: Some("assistant_event".to_string()),
        follow_up_anchor_ref: Some("event:grok-build".to_string()),
        needs_user_reply: false,
        tools_called: vec!["web_search".to_string()],
        rounds_used: 1,
    };

    let rendered = Orchestrator::render_assistant_turn_delivery_message(&outcome, "UTC")
        .expect("rendered delivery message");
    assert_eq!(rendered.matches(shared_message).count(), 1);
    assert!(rendered.contains("建议任务：Search for Grok Build updates"));
    assert!(rendered.contains("建议提醒：Check Grok Build again"));
    assert!(rendered.contains("时间：2026-05-26 16:01 UTC"));
}

#[test]
fn render_assistant_turn_delivery_message_sanitizes_html_in_proposed_task_title() {
    // C4 paired-fix: proposed_task fields (title + notes) are derived from
    // raw email subject / event summary in
    // `build_assistant_event_proposed_task` and reach Telegram via
    // `markdown_to_telegram_html`, which forwards inline HTML raw. The
    // renderer must strip `<...>` to fullwidth before it composes the
    // outgoing message.
    let outcome = crate::assistant_work_items::AssistantTurnOutcome {
        generated_at: "2026-04-25T01:00:00Z".to_string(),
        internal_summary: "Prepared follow-up suggestions.".to_string(),
        user_message: None,
        delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
        notify_priority: Some("important".to_string()),
        proposed_task: Some(crate::assistant_work_items::AssistantProposedTask {
            title: "Review: <script>alert(1)</script>".to_string(),
            notes: Some("Subject came from <b>attacker</b>.".to_string()),
            anchor_kind: None,
            anchor_ref: None,
        }),
        proposed_reminder: None,
        follow_up_anchor_kind: None,
        follow_up_anchor_ref: None,
        needs_user_reply: false,
        tools_called: vec![],
        rounds_used: 1,
    };

    let rendered = Orchestrator::render_assistant_turn_delivery_message(&outcome, "UTC")
        .expect("rendered delivery message");
    // Raw `<script>` / `<b>` tags must NOT survive into the rendered text.
    assert!(
        !rendered.contains("<script>"),
        "raw <script> tag leaked: {rendered}"
    );
    assert!(
        !rendered.contains("</script>"),
        "raw </script> tag leaked: {rendered}"
    );
    assert!(
        !rendered.contains("<b>"),
        "raw <b> tag leaked in notes: {rendered}"
    );
    // Fullwidth replacements are present (matches the C4 strategy).
    assert!(
        rendered.contains("\u{ff1c}script\u{ff1e}alert(1)\u{ff1c}/script\u{ff1e}"),
        "expected fullwidth-escaped script tag in: {rendered}"
    );
    assert!(
        rendered.contains("\u{ff1c}b\u{ff1e}attacker\u{ff1c}/b\u{ff1e}"),
        "expected fullwidth-escaped <b> tag in notes: {rendered}"
    );
}

#[test]
fn render_assistant_turn_delivery_message_sanitizes_html_in_proposed_reminder_title() {
    // Same C4 paired-fix concern for the reminder branch. Reminder title
    // / notes / trigger_at can carry attacker text via the same
    // `build_*_proposed_reminder` paths feeding off raw event data.
    let outcome = crate::assistant_work_items::AssistantTurnOutcome {
        generated_at: "2026-04-25T01:00:00Z".to_string(),
        internal_summary: "Prepared follow-up suggestions.".to_string(),
        user_message: None,
        delivery: crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage,
        notify_priority: Some("important".to_string()),
        proposed_task: None,
        proposed_reminder: Some(crate::assistant_work_items::AssistantProposedReminder {
            title: "Decide on <iframe src=evil>".to_string(),
            notes: Some("Trailing <a href=phish>link</a> here.".to_string()),
            trigger_at: Some("<when>2026-04-26</when>".to_string()),
            anchor_kind: None,
            anchor_ref: None,
        }),
        follow_up_anchor_kind: None,
        follow_up_anchor_ref: None,
        needs_user_reply: false,
        tools_called: vec![],
        rounds_used: 1,
    };

    let rendered = Orchestrator::render_assistant_turn_delivery_message(&outcome, "UTC")
        .expect("rendered delivery message");
    assert!(
        !rendered.contains("<iframe"),
        "raw <iframe tag leaked: {rendered}"
    );
    assert!(
        !rendered.contains("<a href"),
        "raw <a href tag leaked in notes: {rendered}"
    );
    assert!(
        !rendered.contains("<when>"),
        "raw <when> tag leaked in trigger_at: {rendered}"
    );
    assert!(
        rendered.contains("\u{ff1c}iframe src=evil\u{ff1e}"),
        "expected fullwidth-escaped <iframe> in: {rendered}"
    );
    assert!(
        rendered.contains("\u{ff1c}a href=phish\u{ff1e}link\u{ff1c}/a\u{ff1e}"),
        "expected fullwidth-escaped <a> in notes: {rendered}"
    );
    assert!(
        rendered.contains("\u{ff1c}when\u{ff1e}2026-04-26\u{ff1c}/when\u{ff1e}"),
        "expected fullwidth-escaped <when> in trigger_at: {rendered}"
    );
}

#[test]
fn heartbeat_follow_up_result_to_work_item_outcome_prefers_notification_delivery() {
    let work_item = crate::assistant_work_items::build_heartbeat_escalation_work_item(
        "did:aegis:test",
        "Escalate the contract follow-up because it has been waiting on the user for too long.",
        "heartbeat:escalate:followup:contract",
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::DeferredIntentGroup),
        Some("followup:contract".to_string()),
        "waiting_user_overdue",
    );
    let result = OrchestratorResult {
            reply: "I reviewed the stale follow-up.".to_string(),
            notifications: vec![NotifyUserRequest {
                message: "The contract follow-up has been waiting on you for too long. I recommend deciding whether to send it today.".to_string(),
                pin: false,
                priority: NotifyPriority::Important,
                source_tools: vec!["task_lookup".to_string()],
            }],
            tools_called: vec!["task_lookup".to_string()],
            rounds_used: 2,
            ..Default::default()
        };

    let outcome =
        Orchestrator::heartbeat_follow_up_result_to_work_item_outcome(&work_item, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
    );
    assert_eq!(outcome.notify_priority.as_deref(), Some("important"));
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("deferred_intent_group")
    );
    assert_eq!(
        outcome.follow_up_anchor_ref.as_deref(),
        Some("followup:contract")
    );
    assert_eq!(
        outcome
            .proposed_task
            .as_ref()
            .map(|task| task.title.as_str()),
        Some(
            "Escalate the contract follow-up because it has been waiting on the user for too long"
        )
    );
    assert!(outcome.needs_user_reply);
}

#[test]
fn heartbeat_follow_up_result_to_work_item_outcome_keeps_routine_priority_routine() {
    let work_item = crate::assistant_work_items::build_heartbeat_escalation_work_item(
        "did:aegis:test",
        "Check routine watch updates for OpenClaw.",
        "heartbeat:watch:openclaw",
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::DeferredIntentGroup),
        Some("followup:openclaw-watch".to_string()),
        "scheduled_watch",
    );
    let result = OrchestratorResult {
        reply: "I checked the routine watch.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "OpenClaw has a routine beta update.".to_string(),
            pin: false,
            priority: NotifyPriority::Routine,
            source_tools: vec!["web_search".to_string()],
        }],
        tools_called: vec!["web_search".to_string()],
        rounds_used: 2,
        ..Default::default()
    };

    let outcome =
        Orchestrator::heartbeat_follow_up_result_to_work_item_outcome(&work_item, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::ProactiveMessage
    );
    assert_eq!(outcome.notify_priority.as_deref(), Some("routine"));
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("deferred_intent_group")
    );
}

#[test]
fn heartbeat_follow_up_result_to_work_item_outcome_respects_silent_delivery_policy() {
    let work_item = crate::assistant_work_items::build_heartbeat_silent_reprioritization_work_item(
        "did:aegis:test",
        "Silently reprioritize the stale legal thread review and keep it deferred unless something changed.",
        "heartbeat:reprioritize:thread-9",
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread),
        Some("thread-9".to_string()),
        "queued_work_stale",
    );
    let result = OrchestratorResult {
        reply: "This stale thread can stay deferred until the user returns to it.".to_string(),
        notifications: vec![NotifyUserRequest {
            message: "You should probably revisit the stale legal thread soon.".to_string(),
            pin: false,
            priority: NotifyPriority::Important,
            source_tools: vec!["task_lookup".to_string()],
        }],
        tools_called: vec!["task_lookup".to_string()],
        rounds_used: 1,
        ..Default::default()
    };

    let outcome =
        Orchestrator::heartbeat_follow_up_result_to_work_item_outcome(&work_item, &result);
    assert_eq!(
        outcome.delivery,
        crate::assistant_work_items::AssistantTurnDelivery::Silent
    );
    assert_eq!(outcome.user_message, None);
    assert_eq!(
        outcome.follow_up_anchor_kind.as_deref(),
        Some("email_thread")
    );
    assert_eq!(outcome.follow_up_anchor_ref.as_deref(), Some("thread-9"));
    assert!(outcome.internal_summary.contains("stay deferred"));
}

fn calendar_assistant_event_for_notifier(
    observed_at: &str,
    event_id: &str,
    summary: &str,
    decision: crate::assistant_events::AssistantEventDecision,
    urgency: crate::assistant_events::EventUrgencyHint,
    actionability: crate::assistant_events::EventActionabilityHint,
) -> (
    crate::assistant_events::RawExternalEvent,
    crate::assistant_events::AssistantEvent,
) {
    let (raw, mut ev) = crate::assistant_events::build_calendar_event_changed_event(
        "did:aegis:test",
        observed_at,
        "primary",
        "created",
        "primary",
        event_id,
        summary,
        Some("2026-04-22T10:00:00Z"),
        Some("2026-04-22T11:00:00Z"),
        2,
    );
    ev.decision = decision;
    ev.urgency_hint = urgency;
    ev.actionability_hint = actionability;
    (raw, ev)
}

#[tokio::test]
async fn assistant_event_notifier_emits_only_immediate_messages() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let (raw1, mut ev1) = calendar_assistant_event_for_notifier(
        "2026-04-22T00:00:00Z",
        "ev-1",
        "Security review moved to today",
        crate::assistant_events::AssistantEventDecision::NotifyNow,
        crate::assistant_events::EventUrgencyHint::High,
        crate::assistant_events::EventActionabilityHint::Act,
    );
    let raw1_id = memory.store_raw_external_event(&raw1).await.unwrap();
    ev1.raw_event_id = Some(raw1_id);
    memory.store_assistant_event(&ev1).await.unwrap();

    let (raw2, mut ev2) = calendar_assistant_event_for_notifier(
        "2026-04-21T00:05:00Z",
        "ev-2",
        "Weekly sync",
        crate::assistant_events::AssistantEventDecision::DigestLater,
        crate::assistant_events::EventUrgencyHint::Low,
        crate::assistant_events::EventActionabilityHint::Passive,
    );
    let raw2_id = memory.store_raw_external_event(&raw2).await.unwrap();
    ev2.raw_event_id = Some(raw2_id);
    memory.store_assistant_event(&ev2).await.unwrap();

    let processed = orch
        .process_assistant_event_notifications("did:aegis:test", 12345, 10)
        .await
        .unwrap();
    assert_eq!(processed, 1);

    let delivered = rx.recv().await.expect("immediate notification");
    assert!(
        crate::progress_router::progress_event_to_legacy_string(&delivered)
            .contains("Security review moved to today")
    );
    assert!(
        rx.try_recv().is_err(),
        "digest delivery should now happen through heartbeat work items"
    );
}

#[tokio::test]
async fn assistant_event_notifier_leaves_digest_cursor_for_heartbeat_work() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let (raw, mut ev) = calendar_assistant_event_for_notifier(
        &chrono::Utc::now().to_rfc3339(),
        "ev-1",
        "Weekly sync",
        crate::assistant_events::AssistantEventDecision::DigestLater,
        crate::assistant_events::EventUrgencyHint::Low,
        crate::assistant_events::EventActionabilityHint::Passive,
    );
    let raw_id = memory.store_raw_external_event(&raw).await.unwrap();
    ev.raw_event_id = Some(raw_id);
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        0
    );
    assert!(
        rx.try_recv().is_err(),
        "digest should now be handled by heartbeat-generated work"
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
    assert_eq!(pending.len(), 1, "digest cursor must not advance early");
}

#[tokio::test]
async fn assistant_event_notifier_skips_callback_now_events() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(8);
    orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));

    let (raw, mut ev) = calendar_assistant_event_for_notifier(
        "2026-04-22T00:00:00Z",
        "ev-1",
        "Budget review with the board",
        crate::assistant_events::AssistantEventDecision::CallbackNow,
        crate::assistant_events::EventUrgencyHint::High,
        crate::assistant_events::EventActionabilityHint::Act,
    );
    let raw_id = memory.store_raw_external_event(&raw).await.unwrap();
    ev.raw_event_id = Some(raw_id);
    memory.store_assistant_event(&ev).await.unwrap();

    assert_eq!(
        orch.process_assistant_event_notifications("did:aegis:test", 12345, 10)
            .await
            .unwrap(),
        0
    );
    assert!(rx.try_recv().is_err());
}
