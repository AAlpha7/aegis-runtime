//! Orchestrator tests, wave 2. Child modules are split by topic under `part2/`; this hub keeps the chat-context tests.
use super::*;

mod assistant_events;
mod google_oauth;
mod llm_fallback;
mod notification_policy;
mod tool_dispatch;
mod work_item_delivery;

#[tokio::test]
async fn background_callback_context_does_not_persist_user_prompt_into_memory() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let callback_prompt = "Remember that my favorite color is blue, then review this update.";
    let expected_topic = crate::memory::extract_immediate_fact(callback_prompt)
        .expect("remember signal should parse")
        .0;

    orch.build_react_context(
        "did:aegis:test",
        callback_prompt,
        &[],
        true,
        &UserMessagePersistence::Ephemeral,
    )
    .await
    .unwrap();

    let l1 = memory.get_l1("did:aegis:test").await.unwrap();
    assert!(l1.is_empty(), "ephemeral callback prompt must not enter L1");

    let l2 = memory
        .query_l2_by_topic("did:aegis:test", &expected_topic)
        .await
        .unwrap();
    assert!(
        l2.is_empty(),
        "ephemeral callback prompt must not store immediate facts in L2"
    );
}

#[tokio::test]
async fn direct_chat_context_includes_active_work_items_with_matching_anchor_priority() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings);
    let now = chrono::Utc::now().timestamp();

    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "assistant",
                "Alice sent a contract thread update.",
                now - 1,
                Some("email_thread".to_string()),
                Some("thread-xyz".to_string()),
            ),
        )
        .await
        .unwrap();

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
        thread_key: Some("thread-xyz".to_string()),
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
        decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
        dedup_key: "calendar:primary:thread-xyz".to_string(),
        entities: serde_json::json!({}),
    };
    let event_id = memory
        .store_assistant_event(&assistant_event)
        .await
        .unwrap();
    let item = crate::assistant_work_items::build_callback_review_work_item(
        &crate::assistant_events::StoredAssistantEvent {
            id: event_id,
            event: assistant_event,
        },
    );
    memory.store_assistant_work_item(&item).await.unwrap();

    let second_item = crate::assistant_work_items::build_heartbeat_escalation_work_item(
        "did:aegis:test",
        "Follow up on the same contract thread after the queued review.",
        "assistant_work:heartbeat_escalation:waiting_user_overdue:thread-xyz",
        Some(crate::assistant_work_items::AssistantWorkAnchorKind::EmailThread),
        Some("thread-xyz".to_string()),
        "waiting_user_overdue",
    );
    memory
        .store_assistant_work_item(&second_item)
        .await
        .unwrap();

    let active_items = memory
        .list_assistant_work_items(
            "did:aegis:test",
            Some(&["queued", "running", "waiting_user"]),
            8,
        )
        .await
        .unwrap();
    assert_eq!(
        active_items.len(),
        2,
        "test setup must create two active work items"
    );

    // Transport identity must not displace the meaningful work/thread anchor.
    memory
        .push_l1(
            "did:aegis:test",
            Message::text_with_anchor(
                "user",
                "Current Web request",
                now,
                Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                Some("web-request-id".to_string()),
            ),
        )
        .await
        .unwrap();
    assert!(
        memory
            .get_l1("did:aegis:test")
            .await
            .unwrap()
            .iter()
            .any(|message| message.anchor_kind.as_deref() == Some("email_thread")),
        "test setup must retain the work anchor inside the L1 freshness window"
    );
    let ctx = orch
        .build_react_context(
            "did:aegis:test",
            "这个先别发，我想先看看细节。",
            &[],
            false,
            &UserMessagePersistence::Ephemeral,
        )
        .await
        .unwrap();

    assert!(ctx.volatile_prompt.contains("<turn_memory_context>"));
    assert!(ctx.volatile_prompt.contains("<active_assistant_work>"));
    assert!(
        ctx.volatile_prompt
            .contains("Current conversation anchor preference: email_thread = thread-xyz")
    );
    assert!(
        ctx.volatile_prompt
            .contains("Please review contract changes")
    );
    assert!(ctx.volatile_prompt.contains("grouped_related_work=2"));
    assert!(
        !ctx.volatile_prompt
            .contains("Follow up on the same contract thread after the queued review."),
        "same-anchor work should be grouped instead of repeated line-by-line"
    );
}

#[tokio::test]
async fn chat_history_window_limits_prompt_history_from_l1() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory.clone(), settings.clone());
    let did = "did:aegis:history-window";
    let base = chrono::Utc::now().timestamp();
    for i in 0..40 {
        memory
            .push_l1(
                did,
                crate::memory::Message::text("user", format!("msg-{i}"), base + i),
            )
            .await
            .unwrap();
    }

    let ctx = orch
        .build_react_context(
            did,
            "latest turn",
            &[],
            false,
            &UserMessagePersistence::Ephemeral,
        )
        .await
        .unwrap();
    // 40 stored + the current turn = 41; excess 17 drops one 8-chunk → 25.
    assert_eq!(
        ctx.history.len(),
        25,
        "default window is 24 (+ chunk slack)"
    );
    assert_eq!(
        ctx.history.last().map(|m| m.content.as_str()),
        Some("latest turn")
    );

    settings
        .set("chat_history_window_messages", serde_json::json!(10))
        .unwrap();
    let ctx = orch
        .build_react_context(
            did,
            "honoured",
            &[],
            false,
            &UserMessagePersistence::Ephemeral,
        )
        .await
        .unwrap();
    // 41 total, window 10: excess 31 drops 24 → 17 kept.
    assert_eq!(ctx.history.len(), 17, "setting is honoured (chunked drop)");
}
