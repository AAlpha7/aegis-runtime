use super::*;
// ── de-noise historical projection filter tests ───────────────────────────

#[tokio::test]
async fn chat_history_filters_routine_mail_l1_turns_structurally() {
    // Simulates an L1 timeline that ALREADY has injected routine-mail turns
    // (migration-on-existing-data scenario). Verifies that:
    //   - routine Google NotifyNow L1 turns are removed from the projection
    //   - real dialogue turns are preserved
    //   - high-urgency (security) L1 turns are preserved
    //   - calendar routine L1 turns are removed

    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test";

    // --- Store assistant events in DB so the anchor index can resolve them ---

    // Event 1: routine Google inbox email (medium urgency)
    let routine_calendar_ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:00:00Z".to_string(),
        occurred_at: "2026-06-30T00:00:00Z".to_string(),
        user_did: user_did.to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
        account_id: Some("primary".to_string()),
        title: "Invoice from vendor".to_string(),
        summary: "Invoice #123".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:test:routine-mail-hist-1".to_string(),
        entities: serde_json::json!({ "from": "billing@vendor.com" }),
    };
    let routine_calendar_id = memory
        .store_assistant_event(&routine_calendar_ev)
        .await
        .unwrap();

    // Event 2: high-urgency Google (security alert) — must NOT be filtered
    let security_ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:01:00Z".to_string(),
        occurred_at: "2026-06-30T00:01:00Z".to_string(),
        user_did: user_did.to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::EmailReceived,
        account_id: Some("primary".to_string()),
        title: "Security alert".to_string(),
        summary: "Unrecognised sign-in".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::High,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Act,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:test:security-hist-2".to_string(),
        entities: serde_json::json!({ "from": "noreply@accounts.google.com" }),
    };
    let security_id = memory.store_assistant_event(&security_ev).await.unwrap();

    // Event 3: routine Calendar event — must be filtered
    let routine_cal_ev = crate::assistant_events::AssistantEvent {
        raw_event_id: None,
        observed_at: "2026-06-30T00:02:00Z".to_string(),
        occurred_at: "2026-06-30T00:02:00Z".to_string(),
        user_did: user_did.to_string(),
        source: crate::assistant_events::AssistantEventSource::Calendar,
        kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
        account_id: Some("primary".to_string()),
        title: "Team meeting updated".to_string(),
        summary: "Meeting moved to 3pm".to_string(),
        thread_key: None,
        urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
        actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
        decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
        dedup_key: "calendar:test:routine-cal-hist-3".to_string(),
        entities: serde_json::json!({}),
    };
    let routine_cal_id = memory.store_assistant_event(&routine_cal_ev).await.unwrap();

    // --- Push L1 messages as they would have been injected before the gate ---

    // Use real timestamps (Utc::now() per message) so:
    //   a) L1 TTL eviction (7-day window) retains all messages, and
    //   b) Each L1 timestamp is in the same 300-second dedup bucket as its
    //      corresponding L3 archive timestamp (both ≈ wall-clock time of push).
    //
    // The L1→L3 dedup relies on same bucket; synthetic future offsets (e.g. +190)
    // can cross bucket boundaries and produce spurious L3 duplicates in the output.

    // L1 turn for routine Google (old injection, should be filtered)
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "📬 [primary] Invoice #123 from billing@vendor.com",
                chrono::Utc::now().timestamp(),
                Some("assistant_event".to_string()),
                Some(format!("assistant_event:{routine_calendar_id}")),
            ),
        )
        .await
        .unwrap();

    // L1 turn for security alert (must NOT be filtered)
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "📬 [primary] Security alert: Unrecognised sign-in",
                chrono::Utc::now().timestamp(),
                Some("assistant_event".to_string()),
                Some(format!("assistant_event:{security_id}")),
            ),
        )
        .await
        .unwrap();

    // L1 turn for routine calendar (old injection, should be filtered)
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "📬 [primary] Team meeting moved to 3pm",
                chrono::Utc::now().timestamp(),
                Some("assistant_event".to_string()),
                Some(format!("assistant_event:{routine_cal_id}")),
            ),
        )
        .await
        .unwrap();

    // Plain user dialogue turn (no anchor, must NOT be filtered)
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "Can you summarise my email from the vendor?",
                chrono::Utc::now().timestamp(),
                None,
                None,
            ),
        )
        .await
        .unwrap();

    // Plain assistant reply (no anchor, must NOT be filtered)
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "The vendor invoice #123 is for $500 due next Friday.",
                chrono::Utc::now().timestamp(),
                None,
                None,
            ),
        )
        .await
        .unwrap();

    // --- Run the projection ---
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    let messages = &projection.messages;

    // Routine Google and Calendar L1 turns must be absent
    assert!(
        !messages
            .iter()
            .any(|m| m.text.contains("Invoice #123 from billing@vendor.com")),
        "routine Calendar notification must be filtered from chat history"
    );
    assert!(
        !messages
            .iter()
            .any(|m| m.text.contains("Team meeting moved to 3pm")),
        "routine Calendar notification must be filtered from chat history"
    );

    // Security alert must remain
    assert!(
        messages
            .iter()
            .any(|m| m.text.contains("Security alert: Unrecognised sign-in")),
        "security alert L1 turn must remain in chat history"
    );

    // Dialogue turns must remain
    assert!(
        messages
            .iter()
            .any(|m| m.text.contains("summarise my email")),
        "user dialogue turn must remain in chat history"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.text.contains("invoice #123 is for $500")),
        "assistant reply must remain in chat history"
    );

    // Total: 3 turns (security alert + user question + assistant reply)
    assert_eq!(
        messages.len(),
        3,
        "projection must contain exactly 3 turns: security alert, user dialogue, assistant reply; got {} turns",
        messages.len()
    );
}

#[tokio::test]
async fn chat_history_projection_attaches_turn_evidence_to_matching_aegis_message_only() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test-turn-evidence";
    let now = chrono::Utc::now().timestamp();
    let aegis_text = "预计 44 分钟 [E1]。";
    let evidence = vec![crate::evidence::EvidenceItem {
        id: "E1".into(),
        tool: "commute_traffic_monitor".into(),
        label: "Commute check".into(),
        excerpt: "当前交通约 44 分钟。".into(),
        full_len: 18,
    }];
    memory
        .push_l1(user_did, crate::memory::Message::text("user", "通勤", now))
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text("assistant", aegis_text, now + 1),
        )
        .await
        .unwrap();
    memory
        .store_turn_evidence(
            user_did,
            &crate::evidence::reply_key(aegis_text),
            &evidence,
            &["60-80".into()],
        )
        .await
        .unwrap();
    memory
        .store_turn_evidence(
            user_did,
            &crate::evidence::reply_key("unrelated reply"),
            &evidence,
            &[],
        )
        .await
        .unwrap();

    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(projection.messages.len(), 2);
    assert_eq!(projection.messages[0].role, "owner");
    assert!(projection.messages[0].evidence.is_none());
    assert!(projection.messages[0].unsupported_numbers.is_none());
    assert_eq!(projection.messages[1].role, "aegis");
    assert_eq!(projection.messages[1].text, aegis_text);
    assert_eq!(projection.messages[1].evidence.as_ref(), Some(&evidence));
    assert_eq!(
        projection.messages[1].unsupported_numbers.as_deref(),
        Some(&["60-80".to_string()][..])
    );

    let json = serde_json::to_value(&projection.messages[1]).unwrap();
    assert_eq!(json["role"], "aegis");
    assert_eq!(json["evidence"][0]["id"], "E1");
    assert_eq!(json["unsupported_numbers"][0], "60-80");

    let omitted = serde_json::to_value(&projection.messages[0]).unwrap();
    assert!(omitted.get("evidence").is_none());
    assert!(omitted.get("unsupported_numbers").is_none());
}

#[tokio::test]
async fn chat_history_projection_pages_older_messages_with_before() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test-history-paging";
    let base = chrono::Utc::now().timestamp() - 10_000;
    // 20 turns = 40 messages, more than one window (24).
    for turn in 0..20 {
        let at = base + turn * 10;
        memory
            .push_l1(
                user_did,
                crate::memory::Message::text("user", format!("问题 {turn}"), at),
            )
            .await
            .unwrap();
        memory
            .push_l1(
                user_did,
                crate::memory::Message::text("assistant", format!("回答 {turn}"), at + 1),
            )
            .await
            .unwrap();
    }

    let first = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(first.messages.len(), WEBAPP_VNEXT_CHAT_HISTORY_LIMIT);
    assert!(first.has_more, "40 messages > one window must report more");
    assert_eq!(first.messages.last().unwrap().text, "回答 19");
    let oldest_shown = first.messages[0].timestamp;

    let older = vnext_chat_history_projection(&memory, user_did, Some(oldest_shown)).await;
    assert!(!older.messages.is_empty());
    assert!(older.messages.iter().all(|m| m.timestamp < oldest_shown));
    assert_eq!(
        older.messages.last().unwrap().timestamp,
        oldest_shown - 1 - 8
    );
    assert!(!older.has_more, "16 remaining messages fit in one window");
    assert_eq!(older.messages[0].text, "问题 0");

    let json = serde_json::to_value(&first).unwrap();
    assert_eq!(json["has_more"], true);

    // Boundary tie: messages sharing the second at the window cut are kept
    // together, so paging with `before = oldest shown` never skips any.
    let tied_did = "did:aegis:test-history-tie";
    let tie_at = base + 5_000;
    for turn in 0..12 {
        // turns 0..3 share one second (8 messages at tie_at), the rest are spaced.
        let at = if turn < 4 {
            tie_at
        } else {
            tie_at + 100 + turn * 10
        };
        memory
            .push_l1(
                tied_did,
                crate::memory::Message::text("user", format!("t{turn}"), at),
            )
            .await
            .unwrap();
        memory
            .push_l1(
                tied_did,
                crate::memory::Message::text("assistant", format!("a{turn}"), at),
            )
            .await
            .unwrap();
    }
    for turn in 0..10 {
        let at = tie_at - 1000 + turn * 10;
        memory
            .push_l1(
                tied_did,
                crate::memory::Message::text("user", format!("old{turn}"), at),
            )
            .await
            .unwrap();
    }
    // 8 tied + 16 later + 10 older = 34 messages; a plain 24-cut would land inside the tied group.
    let page = vnext_chat_history_projection(&memory, tied_did, None).await;
    let tied_shown = page
        .messages
        .iter()
        .filter(|m| m.timestamp == tie_at)
        .count();
    assert_eq!(
        tied_shown, 8,
        "the whole same-second group is shown together"
    );
    assert!(page.messages.len() >= WEBAPP_VNEXT_CHAT_HISTORY_LIMIT && page.messages.len() <= 64);
    assert!(page.has_more);
    let older =
        vnext_chat_history_projection(&memory, tied_did, Some(page.messages[0].timestamp)).await;
    assert_eq!(older.messages.len(), 10);
    assert!(older.messages.iter().all(|m| m.text.starts_with("old")));
    assert!(!older.has_more);
}

#[tokio::test]
async fn chat_history_turn_identity_preserves_same_second_turns_and_final_evidence() {
    let state = webapp_test_state_with_db(None);
    let chat_id = 12345678;
    let user_did = webapp_canonical_user_did(chat_id);
    let now = chrono::Utc::now().timestamp();
    let raw = "明天晴。[E1,E2]";
    for (turn_id, source) in [
        ("turn-first", "First forecast"),
        ("turn-second", "Second forecast"),
    ] {
        for (role, text) in [("user", "明天天气如何"), ("assistant", raw)] {
            state
                .memory
                .push_l1(
                    &user_did,
                    crate::memory::Message::text_with_anchor(
                        role,
                        text,
                        now,
                        Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                        Some(turn_id.to_string()),
                    ),
                )
                .await
                .unwrap();
        }
        let mut assistant = runtime_assistant_success(
            crate::orchestrator::OrchestratorResult {
                reply: raw.to_string(),
                tools_called: vec!["search_tools".into(), "web_search".into()],
                tool_outputs: vec![
                    ("search_tools".into(), "metadata".into()),
                    ("web_search".into(), source.into()),
                ],
                ..Default::default()
            },
            build_general_assistant_context_envelope(),
            "orchestrated_response",
            false,
        );
        assistant.turn_id = Some(turn_id.to_string());
        assert_eq!(assistant.visible_answer, "明天晴。 [E2]");
        let response = vnext_command_response(serde_json::to_value(assistant).unwrap());
        persist_vnext_command_turn_evidence(&state, Some(chat_id), &response).await;
        assert_eq!(response["assistant_response"]["turn_id"], turn_id);
    }
    // A third turn has equal text but no finalized evidence; it must not borrow
    // either prior turn's source just because the content matches.
    state
        .memory
        .push_l1(
            &user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                raw,
                now,
                Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                Some("turn-unfinished".to_string()),
            ),
        )
        .await
        .unwrap();
    let projection = vnext_chat_history_projection(&state.memory, &user_did, None).await;
    assert_eq!(
        projection.messages.len(),
        5,
        "equal text in distinct same-second requests must survive; L1/L3 copies must not duplicate"
    );
    for (turn_id, source) in [
        ("turn-first", "First forecast"),
        ("turn-second", "Second forecast"),
    ] {
        let rows = projection
            .messages
            .iter()
            .filter(|message| message.turn_id.as_deref() == Some(turn_id))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        let reply = rows.iter().find(|message| message.role == "aegis").unwrap();
        assert_eq!(reply.text, "明天晴。 [E2]");
        assert_eq!(reply.evidence.as_ref().unwrap()[0].excerpt, source);
    }
    let unfinished = projection
        .messages
        .iter()
        .find(|message| message.turn_id.as_deref() == Some("turn-unfinished"))
        .unwrap();
    assert!(unfinished.evidence.is_none());
    let archive = state.memory.list_recent_l3(&user_did, 20).await.unwrap();
    assert_eq!(archive.len(), 5);
    assert!(
        archive.iter().all(|row| row.anchor_kind.as_deref()
            == Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND))
    );
    assert!(
        archive
            .iter()
            .any(|row| row.anchor_ref.as_deref() == Some("turn-first"))
    );
}

#[tokio::test]
async fn chat_history_turn_identity_resolves_long_reply_evidence_before_clipping() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:long-history-identity";
    let now = chrono::Utc::now().timestamp();
    let raw = format!("{}[E1]", "事实".repeat(2050));
    let evidence = vec![crate::evidence::EvidenceItem {
        id: "E1".into(),
        tool: "web_search".into(),
        label: "Web search".into(),
        excerpt: "Long reply source".into(),
        full_len: 17,
    }];
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text("assistant", &raw, now),
        )
        .await
        .unwrap();
    memory
        .store_turn_evidence(user_did, &crate::evidence::reply_key(&raw), &evidence, &[])
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                &raw,
                now,
                Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                Some("long-turn".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .store_turn_evidence(
            user_did,
            &crate::evidence::turn_key("long-turn"),
            &evidence,
            &[],
        )
        .await
        .unwrap();
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(projection.messages.len(), 2);
    for message in &projection.messages {
        assert_eq!(message.text.chars().count(), 4000);
        assert_eq!(message.evidence.as_ref(), Some(&evidence));
    }
    assert!(
        projection
            .messages
            .iter()
            .any(|message| message.turn_id.as_deref() == Some("long-turn"))
    );
    assert!(
        projection
            .messages
            .iter()
            .any(|message| message.turn_id.is_none())
    );
}

#[tokio::test]
async fn chat_history_turn_identity_empty_evidence_still_removes_unbacked_refs() {
    let state = webapp_test_state_with_db(None);
    let chat_id = 23456789;
    let user_did = webapp_canonical_user_did(chat_id);
    state
        .memory
        .push_l1(
            &user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "Hello [E99]",
                chrono::Utc::now().timestamp(),
                Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                Some("no-source-turn".into()),
            ),
        )
        .await
        .unwrap();
    persist_vnext_command_turn_evidence(&state, Some(chat_id), &serde_json::json!({
        "assistant_response": {"turn_id": "no-source-turn", "visible_answer": "Hello", "evidence": [], "unsupported_numbers": []}
    })).await;
    let projection = vnext_chat_history_projection(&state.memory, &user_did, None).await;
    assert_eq!(projection.messages.len(), 1);
    assert_eq!(projection.messages[0].text, "Hello");
    assert!(projection.messages[0].evidence.is_none());
}

#[test]
fn vnext_chat_history_message_omits_none_evidence_fields() {
    let message = VNextChatHistoryMessage {
        turn_id: None,
        role: "aegis".into(),
        text: "hello".into(),
        timestamp: 1,
        severity: "info".into(),
        latency_ms: None,
        repeat_count: None,
        first_timestamp: None,
        alert_group: None,
        work_item_id: None,
        evidence: None,
        unsupported_numbers: None,
        retry_text: None,
    };
    let json = serde_json::to_value(&message).unwrap();
    assert!(json.get("evidence").is_none());
    assert!(json.get("unsupported_numbers").is_none());
    assert!(json.get("retry_text").is_none());
    assert_eq!(json["role"], "aegis");
    assert_eq!(json["text"], "hello");
}
/// (1) vnext status projection with BOTH degraded blocks populated — the shape
/// that bit production (BE serialized `embedder_degraded`, FE never wired it).
/// Direct call of the production projection builder (the same function the
/// `/webapp/vnext/status` handler uses) is the honest source: the handler's
/// surrounding auth/session plumbing adds nothing to the serialized shape.
#[test]
fn contract_fixture_vnext_status_projection_with_degraded_blocks() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-07-08T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "available".into(),
            reason: None,
        }],
        counts: UnifiedInboxCounts::default(),
        items: Vec::new(),
    };
    let status = vnext_status_projection_with_settings_and_events(
        &projection,
        None,
        Vec::new(),
        Some(crate::orchestrator::LlmDegraded {
            reason: "usage limit reached".into(),
            since: "2026-07-08T00:00:00Z".into(),
            provider: "codex_oauth".into(),
        }),
        Some(crate::orchestrator::EmbedderDegraded {
            since: "2026-07-08T00:00:00Z".into(),
            primary_model: "text-embedding-primary".into(),
            backup_model: Some("text-embedding-backup".into()),
        }),
        "en",
    );
    let mut actual = serde_json::to_value(&status).unwrap();
    // `advanced_diagnostics_available` reads process-global state (env var +
    // a test refcount other #[tokio::test]s in this binary bump concurrently).
    // Pin it to `false` here — the flag's own behavior is covered by the
    // dedicated webapp-debug-snapshot tests. Assert shape BEFORE normalizing:
    // blind assignment would mask the field being removed/renamed/retyped
    // (parent Codex P2) — normalization may only touch the VALUE.
    assert!(
        actual["advanced_diagnostics_available"].is_boolean(),
        "advanced_diagnostics_available must exist as a bool in the real projection, got: {:?}",
        actual.get("advanced_diagnostics_available")
    );
    actual["advanced_diagnostics_available"] = serde_json::Value::Bool(false);
    assert_contract_fixture("vnext_status_projection.json", &actual);
}
#[tokio::test]
async fn webapp_vnext_upload_happy_small_png() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);

    let mut png = TEST_PNG_MAGIC.to_vec();
    png.extend_from_slice(b"tiny-test-png-payload");
    let resp = app
        .oneshot(build_upload_request(
            &cookie_token,
            Some(&csrf_token),
            "pixel.png",
            Some("image/png"),
            &png,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    use base64::Engine as _;
    assert_eq!(json["type"], "image");
    assert_eq!(json["mime_type"], "image/png");
    assert_eq!(json["filename"], "pixel.png");
    let data_url = json["data_url"].as_str().unwrap();
    let expected_b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    assert_eq!(
        data_url,
        format!("data:image/png;base64,{expected_b64}"),
        "data_url must carry the exact uploaded bytes"
    );
}

#[tokio::test]
async fn webapp_vnext_upload_oversize_image_rejected_413() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);

    // One byte over the 5 MB image ceiling (still under the 12 MB raw body
    // cap, so the rejection is the structured per-kind 413, not a transport
    // truncation).
    let mut png = TEST_PNG_MAGIC.to_vec();
    png.resize(VNEXT_UPLOAD_MAX_IMAGE_BYTES + 1, 0u8);
    let resp = app
        .oneshot(build_upload_request(
            &cookie_token,
            Some(&csrf_token),
            "big.png",
            Some("image/png"),
            &png,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn webapp_vnext_upload_rejects_wrong_or_spoofed_mime() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));

    // (a) Unknown magic bytes (plain text) — declared image/png does not help;
    // the magic-number classification is authoritative.
    let resp = oauth_callback_router(state.clone())
        .oneshot(build_upload_request(
            &cookie_token,
            Some(&csrf_token),
            "note.png",
            Some("image/png"),
            b"just some text pretending to be an image",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // (b) Real PNG bytes but a contradictory declared type (spoof) → rejected.
    let mut png = TEST_PNG_MAGIC.to_vec();
    png.extend_from_slice(b"png-bytes");
    let resp = oauth_callback_router(state)
        .oneshot(build_upload_request(
            &cookie_token,
            Some(&csrf_token),
            "sneaky.pdf",
            Some("application/pdf"),
            &png,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn webapp_vnext_upload_rejects_missing_auth_and_missing_csrf() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, _csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));

    let mut png = TEST_PNG_MAGIC.to_vec();
    png.extend_from_slice(b"x");

    // No session at all → auth rejection.
    let resp = oauth_callback_router(state.clone())
        .oneshot(build_upload_request(
            "not-a-real-session",
            None,
            "a.png",
            Some("image/png"),
            &png,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Valid cookie session but NO CSRF header → rejected before any file read.
    let resp = oauth_callback_router(state)
        .oneshot(build_upload_request(
            &cookie_token,
            None,
            "a.png",
            Some("image/png"),
            &png,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webapp_vnext_turn_stream_token_mint_and_events_handshake() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_progress_router(Some(db_path));

    // 1. Mint: console auth + CSRF → ok envelope with a token.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/turns/turn-s2-events/stream-token")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    let token = json["stream_token"].as_str().expect("token minted");
    assert!(json["expires_in"].as_u64().unwrap_or(0) > 0);

    // 2. Events GET with the minted token: SSE handshake succeeds. Assert the
    //    response line + content-type only — the body is an open stream (the
    //    sink stays registered), so it is dropped unread, not polled.
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/webapp/vnext/turns/turn-s2-events/events?stream_token={token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE handshake must be text/event-stream, got {content_type}"
    );
}

#[tokio::test]
async fn webapp_vnext_turn_events_rejects_bad_missing_or_cross_turn_tokens() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_progress_router(Some(db_path));

    // Missing token → 401.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/turns/turn-s2-reject/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Garbage token → 401.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/turns/turn-s2-reject/events?stream_token=bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Token minted for one turn must not open a DIFFERENT turn (no
    // enumeration): mint for A, open B → 403.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/turns/turn-s2-a/stream-token")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let token = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["stream_token"]
        .as_str()
        .unwrap()
        .to_string();
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/webapp/vnext/turns/turn-s2-b/events?stream_token={token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webapp_vnext_turn_stream_token_mint_rejects_missing_csrf() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let state = webapp_test_state_with_progress_router(Some(db_path));

    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/turns/turn-s2-nocsrf/stream-token")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ── W5d: /webapp/console/approvals/{approval_id}/approve + /reject ──────────

#[tokio::test]
async fn webapp_console_approval_approve_and_reject_round_trip() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory.clone(), Some(db_path));

    // Seed two pending approvals in the SAME memory the orchestrator uses.
    for key in ["console-appr-s2-approve", "console-appr-s2-reject"] {
        memory
            .store_skill_state(
                "hitl",
                key,
                &serde_json::json!({
                    "approval_id": key,
                    "chat_id": 12345678,
                    "user_did": "did:aegis:telegram:12345678",
                    "created_at": chrono::Utc::now().to_rfc3339(),
                })
                .to_string(),
            )
            .await
            .unwrap();
    }

    // Approve → 200 ok envelope; the stored approval is consumed.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/console-appr-s2-approve/approve")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["action"], "approve");
    assert_eq!(json["approval_id"], "console-appr-s2-approve");
    let cleared = memory
        .load_skill_state("hitl", "console-appr-s2-approve")
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(cleared.is_empty(), "approved row must be consumed");

    // Reject → 200 ok envelope with action=reject; row consumed.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/console-appr-s2-reject/reject")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["action"], "reject");

    // Consumed/absent approval → 404 (no zombie approvals via the console).
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/console-appr-s2-approve/approve")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webapp_duplicate_egress_hold_is_one_chat_card_and_stays_approved() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory.clone(), Some(db_path));
    let same = "destination: lawyer@example.com\nwould_send: billing note\nstopped: sensitive";
    let other = "destination: other@example.com\nwould_send: a different note\nstopped: untrusted_source\nuntrusted_source: web_page";
    // Approve refuses holds older than 24h. Keep these inside that window, with
    // the surviving card still the earliest of the duplicate pair.
    let now = chrono::Utc::now();
    for (key, args, summary, created_at) in [
        (
            "gate-aaaaaaaaaaaaaaaa",
            r#"{"topic":"billing","fact":"billing note for the lawyer"}"#,
            same,
            (now - chrono::Duration::minutes(3)).to_rfc3339(),
        ),
        (
            "gate-bbbbbbbbbbbbbbbb",
            r#"{"topic":"billing-dup","fact":"duplicate must not run"}"#,
            same,
            (now - chrono::Duration::minutes(2)).to_rfc3339(),
        ),
        (
            "gate-cccccccccccccccc",
            r#"{"topic":"other","fact":"other send stays pending"}"#,
            other,
            (now - chrono::Duration::minutes(1)).to_rfc3339(),
        ),
    ] {
        memory
            .store_skill_state(
                "hitl",
                key,
                &serde_json::json!({
                    "approval_id": key,
                    "tool_name": "write_memory",
                    "tool_args": args,
                    "approval_summary": summary,
                    "user_did": "did:aegis:telegram:12345678",
                    "chat_id": 12345678,
                    "created_at": created_at,
                })
                .to_string(),
            )
            .await
            .unwrap();
    }

    let app = oauth_callback_router(state.clone());
    let work = app
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/work")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(work.status(), StatusCode::OK);
    let work_body = axum::body::to_bytes(work.into_body(), 256 * 1024)
        .await
        .unwrap();
    let work_text = String::from_utf8(work_body.to_vec()).unwrap();
    assert!(
        !work_text.contains("我在应用外动手前")
            && !work_text.contains("before Aegis does anything"),
        "duplicate tool holds must not stack as generic work cards"
    );
    assert!(
        !work_text.contains("lawyer@example.com"),
        "the concrete hold belongs on the chat thread, not the work stack"
    );

    let history = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/chat-history")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(history.status(), StatusCode::OK);
    let history_body = axum::body::to_bytes(history.into_body(), 256 * 1024)
        .await
        .unwrap();
    let history_json: serde_json::Value = serde_json::from_slice(&history_body).unwrap();
    let approvals = history_json["approvals"].as_array().unwrap();
    assert_eq!(approvals.len(), 2, "same egress facts collapse to one card");
    let lawyer = approvals
        .iter()
        .find(|card| {
            card["summary"]
                .as_str()
                .unwrap_or("")
                .contains("lawyer@example.com")
        })
        .expect("egress facts on the chat card");
    assert_eq!(lawyer["approval_id"], "gate-aaaaaaaaaaaaaaaa");
    assert_eq!(lawyer["state"], "pending");
    let summary = lawyer["summary"].as_str().unwrap();
    assert!(summary.contains("destination: lawyer@example.com"));
    assert!(summary.contains("would_send: billing note"));
    assert!(summary.contains("stopped: sensitive"));
    assert!(approvals.iter().any(|card| {
        card["summary"]
            .as_str()
            .unwrap_or("")
            .contains("other@example.com")
    }));

    let approved = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/gate-aaaaaaaaaaaaaaaa/approve")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);
    let approved_body = axum::body::to_bytes(approved.into_body(), 64 * 1024)
        .await
        .unwrap();
    let approved_json: serde_json::Value = serde_json::from_slice(&approved_body).unwrap();
    let approved_message = approved_json["message"].as_str().unwrap_or("");
    assert!(
        approved_message.contains("write_memory") && approved_message.contains("row_id"),
        "approve must replay the stored call, got: {approved_message}"
    );
    let owner_did = "did:aegis:telegram:12345678";
    let stored = memory.get_all_l2(owner_did).await.unwrap();
    assert_eq!(
        stored.len(),
        1,
        "one approve dispatches the stored call once"
    );
    assert_eq!(stored[0].fact_summary, "billing note for the lawyer");
    let dispatch = memory
        .load_skill_state(
            crate::egress_gate::HITL_DISPATCH_SKILL,
            "gate-aaaaaaaaaaaaaaaa",
        )
        .await
        .unwrap()
        .unwrap_or_default();
    assert_eq!(dispatch, "1");

    for key in ["gate-aaaaaaaaaaaaaaaa", "gate-bbbbbbbbbbbbbbbb"] {
        let cleared = memory
            .load_skill_state("hitl", key)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            cleared.is_empty(),
            "{key} must be cleared with its duplicate"
        );
    }
    let distinct = memory
        .load_skill_state("hitl", "gate-cccccccccccccccc")
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(!distinct.is_empty(), "a different send stays pending");

    let reloaded = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/chat-history")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reloaded.status(), StatusCode::OK);
    let reloaded_body = axum::body::to_bytes(reloaded.into_body(), 256 * 1024)
        .await
        .unwrap();
    let reloaded_json: serde_json::Value = serde_json::from_slice(&reloaded_body).unwrap();
    let reloaded_approvals = reloaded_json["approvals"].as_array().unwrap();
    let decided = reloaded_approvals
        .iter()
        .find(|card| card["approval_id"] == "gate-aaaaaaaaaaaaaaaa")
        .expect("approved card survives reload");
    assert_eq!(decided["state"], "approved");
    assert!(
        decided["summary"]
            .as_str()
            .unwrap_or("")
            .contains("destination: lawyer@example.com")
    );
    assert!(
        decided["summary"]
            .as_str()
            .unwrap_or("")
            .contains("would_send: billing note")
    );
    assert!(
        decided["summary"]
            .as_str()
            .unwrap_or("")
            .contains("stopped: sensitive")
    );
    assert!(
        !reloaded_approvals
            .iter()
            .any(|card| card["approval_id"] == "gate-bbbbbbbbbbbbbbbb")
    );
    assert!(
        reloaded_approvals.iter().any(
            |card| card["approval_id"] == "gate-cccccccccccccccc" && card["state"] == "pending"
        )
    );

    let refreshed = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/chat-history")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(refreshed.into_body(), 256 * 1024)
        .await
        .unwrap();
    let stored_again = memory.get_all_l2(owner_did).await.unwrap();
    assert_eq!(
        stored_again.len(),
        1,
        "a second refresh must not dispatch the hold again"
    );
    assert_eq!(stored_again[0].fact_summary, "billing note for the lawyer");
    let dispatch_again = memory
        .load_skill_state(
            crate::egress_gate::HITL_DISPATCH_SKILL,
            "gate-aaaaaaaaaaaaaaaa",
        )
        .await
        .unwrap()
        .unwrap_or_default();
    assert_eq!(dispatch_again, "1");

    let rejected = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/gate-cccccccccccccccc/reject")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::OK);
    let stored_after_reject = memory.get_all_l2(owner_did).await.unwrap();
    assert_eq!(stored_after_reject.len(), 1, "reject must not run the tool");
    let rejected_dispatch = memory
        .load_skill_state(
            crate::egress_gate::HITL_DISPATCH_SKILL,
            "gate-cccccccccccccccc",
        )
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(
        rejected_dispatch.is_empty(),
        "reject must not record a dispatch"
    );
}

#[tokio::test]
async fn local_host_high_risk_approval_needs_a_second_confirmation() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_local_login_session_with_csrf(&db_path);
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory.clone(), Some(db_path.clone()));
    let approval_id = "console-appr-host-confirm";
    memory
        .store_skill_state(
            "hitl",
            approval_id,
            &serde_json::json!({
                "approval_id": approval_id,
                "approval_type": "email_send",
                "chat_id": 12345678,
                "user_did": "did:aegis:telegram:12345678",
                "created_at": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
        )
        .await
        .unwrap();

    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/approvals/{approval_id}/approve"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "mfa_required");
    let pending = memory
        .load_skill_state("hitl", approval_id)
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(
        !pending.is_empty(),
        "approval stays pending until the second confirmation"
    );

    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/host-confirm")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["mfa_level"], "host_confirmed");

    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/approvals/{approval_id}/approve"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["action"], "approve");
}

#[tokio::test]
async fn google_session_cannot_use_local_host_confirmation() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let state = webapp_test_state_with_memory_and_db(
        MemoryHandle::spawn_in_memory().unwrap(),
        Some(db_path.clone()),
    );

    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/host-confirm")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let session = crate::web_console_auth::validate_session(
        &conn,
        &cookie_token,
        chrono::Utc::now().timestamp() as u64,
        30 * 24 * 60 * 60,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        session.mfa_level,
        crate::web_console_auth::MfaLevel::GoogleOnly
    );
}

#[tokio::test]
async fn webapp_console_approval_rejects_missing_csrf_and_bad_id() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory, Some(db_path));

    // Cookie session without CSRF → 403 before any approval lookup.
    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/console-appr-nocsrf/approve")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Malformed approval id (illegal chars) → 400.
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/approvals/bad%20id%21/reject")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn webapp_console_mandate_lifecycle_create_activate_pause_resume_revoke() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    // File-backed memory: the mandate store opens state.memory.db_path().
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path));

    // 1. Create → draft.
    let (status, json) = post_mandate_route(
        state.clone(),
        &cookie_token,
        &csrf_token,
        "/webapp/console/mandates",
        Some(serde_json::json!({
            "schema_version": 1,
            "title": "Reminder digest watch",
            "description": "Summarize reminder metadata into a review card.",
            "scope_kind": "reminders",
            "allowed_reads": ["reminders"],
            "allowed_actions": ["surface_card"],
            "must_ask": ["everything_else"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {json}");
    assert_eq!(json["status"], "ok");
    assert_eq!(json["mandate"]["status"], "draft");
    assert_eq!(json["mandate"]["matching_enabled"], false);
    let mandate_id = json["mandate"]["mandate_id"]
        .as_str()
        .expect("create returns the (opaque) mandate id")
        .to_string();

    // 2-5. Lifecycle transitions, each returning the updated summary.
    for (action, expected_status, matching) in [
        ("activate", "active", true),
        ("pause", "paused", false),
        ("resume", "active", true),
        ("revoke", "revoked", false),
    ] {
        let (status, json) = post_mandate_route(
            state.clone(),
            &cookie_token,
            &csrf_token,
            &format!("/webapp/console/mandates/{mandate_id}/{action}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{action} failed: {json}");
        assert_eq!(json["status"], "ok", "{action}: {json}");
        assert_eq!(
            json["mandate"]["status"], expected_status,
            "{action}: {json}"
        );
        assert_eq!(
            json["mandate"]["matching_enabled"], matching,
            "{action}: {json}"
        );
    }

    // A transition on an unknown mandate id is a clean 404.
    let (status, _json) = post_mandate_route(
        state,
        &cookie_token,
        &csrf_token,
        "/webapp/console/mandates/absent-mandate-id/activate",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn webapp_debug_job_runs_backend_smoke_without_minting_a_grant_token() {
    let _env_lock = WEBAPP_DEBUG_ENV_LOCK.lock().await;
    let _debug_env = TestWebappDebugSnapshotsGuard::enable();
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp
        .path()
        .join("aegis-web-console.sqlite")
        .to_string_lossy()
        .to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let cookie_header = format!("__Host-aegis_session={cookie_token}");

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/debug-jobs")
                .header("Cookie", &cookie_header)
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "kind": "webapp_e2e_suite"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = axum::body::to_bytes(create.into_body(), 64 * 1024)
        .await
        .unwrap();
    let create_text = String::from_utf8(create_body.to_vec()).unwrap();
    assert!(!create_text.contains("debug_grant_token"));
    assert!(!create_text.contains(WEBAPP_DEBUG_GRANT_TOKEN_PREFIX));
    let create_json: serde_json::Value = serde_json::from_str(&create_text).unwrap();
    let job_id = create_json["job"]["id"].as_str().unwrap().to_string();

    // Background job can still be "running" under parallel lib-test load; poll
    // briefly instead of a single 50ms sleep (flake under cargo test --lib).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let job = loop {
        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/webapp/vnext/debug-jobs")
                    .header("Cookie", &cookie_header)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let list_body = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&list_body).unwrap();
        let current = list_json["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == job_id)
            .cloned()
            .expect("debug job listed");
        if current["status"] == WEBAPP_DEBUG_JOB_STATUS_PASSED
            || current["status"] == WEBAPP_DEBUG_JOB_STATUS_FAILED
            || std::time::Instant::now() >= deadline
        {
            break current;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(job["kind"], WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE);
    assert_eq!(job["status"], WEBAPP_DEBUG_JOB_STATUS_PASSED);
    // The dev-harness live smoke was the only job that ever needed to call back
    // in as the owner, so no job mints an internal bearer lease any more: the
    // job row carries no lease and no `webapp_debug_leases` row is created.
    assert_eq!(job["lease_id"], serde_json::Value::Null);
    let log_text = job["log_text"].as_str().unwrap();
    assert!(!log_text.contains(WEBAPP_DEBUG_GRANT_TOKEN_PREFIX));

    let lease_count: i64 = crate::encrypted_sqlite::open(&db_path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM webapp_debug_leases", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(lease_count, 0);
}

#[tokio::test]
async fn chat_history_projection_hides_the_reply_to_a_deploy_smoke_turn() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test-smoke-pairing";
    let base = chrono::Utc::now().timestamp() - 1_000;
    let rows = [
        ("user", "早上好", base),
        ("assistant", "早上好，今天有两件事。", base + 1),
        (
            "user",
            "部署冒烟：用浏览器打开 https://example.com，只告诉我标题。",
            base + 10,
        ),
        ("assistant", "Example Domain", base + 12),
        (
            "user",
            "部署冒烟：看一下我今天日历上有什么安排？简短回答。",
            base + 20,
        ),
        ("assistant", "今天主日历没有安排。[E1]", base + 25),
        ("user", "看看哪些占了大量空间", base + 40),
        ("assistant", "最大的是构建产物。[E1]", base + 70),
    ];
    for (role, text, at) in rows {
        memory
            .push_l1(user_did, crate::memory::Message::text(role, text, at))
            .await
            .unwrap();
    }
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    let texts: Vec<&str> = projection
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "早上好",
            "早上好，今天有两件事。",
            "看看哪些占了大量空间",
            "最大的是构建产物。[E1]"
        ],
        "smoke questions AND their replies stay out of the conversation"
    );
}

#[tokio::test]
async fn chat_history_pairs_a_saved_owner_turn_that_never_got_a_reply() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:unanswered-turn";
    let now = chrono::Utc::now().timestamp() - 30;
    let anchor = Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string());
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "帮我看看这封信",
                now,
                anchor.clone(),
                Some("turn-stuck".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "   ",
                now,
                anchor.clone(),
                Some("turn-blank".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "空白那轮",
                now + 1,
                anchor.clone(),
                Some("turn-blank".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "已经答过",
                now + 2,
                anchor.clone(),
                Some("turn-done".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text("assistant", "信已归档。", now + 3),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "还在处理",
                now + 4,
                anchor,
                Some("turn-running".into()),
            ),
        )
        .await
        .unwrap();

    let unanswered = crate::i18n::t("assistant.turn_unanswered", crate::i18n::DEFAULT_LOCALE);
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    let texts: Vec<&str> = projection
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "帮我看看这封信",
            unanswered,
            "空白那轮",
            unanswered,
            "已经答过",
            "信已归档。",
            "还在处理",
            unanswered,
        ]
    );
    assert!(
        projection
            .messages
            .iter()
            .filter(|m| m.text == unanswered)
            .all(|m| m.role == "aegis" && m.severity == "warning")
    );
    let retries: Vec<Option<&str>> = projection
        .messages
        .iter()
        .map(|message| message.retry_text.as_deref())
        .collect();
    assert_eq!(
        retries,
        vec![
            None,
            Some("帮我看看这封信"),
            None,
            Some("空白那轮"),
            None,
            None,
            None,
            Some("还在处理"),
        ]
    );

    let running = HashSet::from(["turn-running".to_string()]);
    let live = vnext_chat_history_projection_with_inflight(
        &memory,
        user_did,
        None,
        None,
        &running,
        crate::i18n::DEFAULT_LOCALE,
    )
    .await;
    let live_texts: Vec<&str> = live.messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        live_texts,
        vec![
            "帮我看看这封信",
            unanswered,
            "空白那轮",
            unanswered,
            "已经答过",
            "信已归档。",
            "还在处理",
        ]
    );
}

#[tokio::test]
async fn record_unanswered_owner_turn_pairs_the_saved_user_line() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:record-unanswered";
    let turn_id = "turn-record";
    crate::progress_router::with_turn_id(Some(turn_id.to_string()), async {
        memory
            .push_l1(
                user_did,
                crate::memory::Message::text_with_anchor(
                    "user",
                    "还在吗",
                    chrono::Utc::now().timestamp(),
                    Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                    Some(turn_id.to_string()),
                ),
            )
            .await
            .unwrap();
        crate::orchestrator::record_unanswered_owner_turn(
            &memory,
            user_did,
            crate::i18n::DEFAULT_LOCALE,
            None,
            &["已尝试备用模型，仍未完成回复。".to_string()],
        )
        .await
        .unwrap();
        crate::orchestrator::record_unanswered_owner_turn(
            &memory,
            user_did,
            crate::i18n::DEFAULT_LOCALE,
            Some("should not replace the first reply"),
            &[],
        )
        .await
        .unwrap();
    })
    .await;
    let rows = memory.get_l1(user_did).await.unwrap();
    let replies: Vec<_> = rows.iter().filter(|m| m.role == "assistant").collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].anchor_ref.as_deref(), Some(turn_id));
    assert!(replies[0].content.starts_with(crate::i18n::t(
        "assistant.turn_unanswered",
        crate::i18n::DEFAULT_LOCALE
    )));
    assert!(
        replies[0]
            .content
            .contains("已尝试备用模型，仍未完成回复。")
    );
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(projection.messages.len(), 2);
    assert_eq!(
        projection
            .messages
            .iter()
            .filter(|m| m.role == "aegis")
            .count(),
        1
    );
    assert_eq!(projection.messages[1].retry_text.as_deref(), Some("还在吗"));
}

#[tokio::test]
async fn record_unanswered_owner_turn_includes_a_redacted_reason() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:record-unanswered-reason";
    let turn_id = "turn-reason";
    let raw = "connection refused api_key=sk-supersecretvalue token=abcdef";
    crate::progress_router::with_turn_id(Some(turn_id.to_string()), async {
        memory
            .push_l1(
                user_did,
                crate::memory::Message::text_with_anchor(
                    "user",
                    "再看看",
                    chrono::Utc::now().timestamp(),
                    Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string()),
                    Some(turn_id.to_string()),
                ),
            )
            .await
            .unwrap();
        crate::orchestrator::record_unanswered_owner_turn(
            &memory,
            user_did,
            crate::i18n::DEFAULT_LOCALE,
            Some(raw),
            &[],
        )
        .await
        .unwrap();
    })
    .await;
    let rows = memory.get_l1(user_did).await.unwrap();
    let reply = rows
        .iter()
        .find(|message| message.role == "assistant")
        .unwrap();
    let hint = crate::i18n::t("assistant.turn_unanswered", crate::i18n::DEFAULT_LOCALE);
    assert!(reply.content.starts_with(hint));
    assert!(reply.content.contains("原因："));
    assert!(reply.content.contains("connection refused"));
    assert!(!reply.content.contains("sk-supersecretvalue"));
    assert!(!reply.content.contains("abcdef"));
    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(projection.messages[1].retry_text.as_deref(), Some("再看看"));
    assert!(projection.messages[1].text.contains("connection refused"));
}

#[tokio::test]
async fn chat_history_failure_hint_retry_uses_the_preceding_owner_text() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:failure-hint-retry";
    let now = chrono::Utc::now().timestamp() - 20;
    let anchor = Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.to_string());
    let empty = crate::i18n::t("assistant.empty_response", "en");
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "再试一次",
                now,
                anchor.clone(),
                Some("turn-empty".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                empty,
                now + 1,
                anchor.clone(),
                Some("turn-empty".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "user",
                "普通问题",
                now + 2,
                anchor.clone(),
                Some("turn-ok".into()),
            ),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "普通回答。",
                now + 3,
                anchor,
                Some("turn-ok".into()),
            ),
        )
        .await
        .unwrap();

    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    assert_eq!(
        projection.messages[1].retry_text.as_deref(),
        Some("再试一次")
    );
    assert!(projection.messages[3].retry_text.is_none());
    let json = serde_json::to_value(&projection).unwrap();
    assert_eq!(json["messages"][1]["retry_text"], "再试一次");
    assert!(json["messages"][3].get("retry_text").is_none());
}

#[test]
fn inflight_turn_guard_hides_the_turn_only_while_held() {
    let id = "turn-guard-unanswered-only-while-held";
    assert!(!webapp_inflight_turn_ids().contains(id));
    let guard = WebappInflightTurnGuard::enter(id);
    assert!(webapp_inflight_turn_ids().contains(id));
    drop(guard);
    assert!(!webapp_inflight_turn_ids().contains(id));
}

#[tokio::test]
async fn chat_history_projection_carries_reply_latency_for_assistant_rows_only() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test-latency";
    let base = chrono::Utc::now().timestamp() - 1_000;
    let mut asked = crate::memory::Message::text("user", "多久？", base);
    asked.latency_ms = Some(5); // never meaningful on a user row
    memory.push_l1(user_did, asked).await.unwrap();
    let mut reply = crate::memory::Message::text("assistant", "30 秒。", base + 30);
    reply.latency_ms = Some(30_412);
    memory.push_l1(user_did, reply).await.unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text("assistant", "旧回复没有用时。", base + 60),
        )
        .await
        .unwrap();

    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    let json = serde_json::to_value(&projection).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert!(
        messages[0].get("latency_ms").is_none(),
        "owner rows carry no latency"
    );
    assert_eq!(messages[1]["latency_ms"], 30_412);
    assert!(
        messages[2].get("latency_ms").is_none(),
        "rows stored without latency omit the field"
    );
}

#[tokio::test]
async fn chat_history_projection_collapses_consecutive_heartbeat_alerts() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:test-alert-collapse";
    let base = chrono::Utc::now().timestamp() - 100_000;
    let alert = |text: &str, at: i64| {
        crate::memory::Message::text_with_anchor(
            "assistant",
            text,
            at,
            Some(crate::telegram::HEARTBEAT_ALERT_ANCHOR_KIND.to_string()),
            Some("server_monitor".to_string()),
        )
    };
    memory
        .push_l1(user_did, alert("🚨 磁盘用了 74%", base))
        .await
        .unwrap();
    memory
        .push_l1(user_did, alert("🚨 磁盘用了 75%（还没解决）", base + 7_200))
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            alert("🚨 磁盘用了 80%（还没解决）", base + 14_400),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text("user", "看看哪些占了大量空间", base + 15_000),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            user_did,
            alert("🚨 磁盘用了 80%（还没解决）", base + 21_600),
        )
        .await
        .unwrap();
    // A different skill's alert never merges into the disk run.
    memory
        .push_l1(
            user_did,
            crate::memory::Message::text_with_anchor(
                "assistant",
                "🚨 通勤路况",
                base + 21_700,
                Some(crate::telegram::HEARTBEAT_ALERT_ANCHOR_KIND.to_string()),
                Some("commute_traffic_monitor".to_string()),
            ),
        )
        .await
        .unwrap();

    let projection = vnext_chat_history_projection(&memory, user_did, None).await;
    let texts: Vec<&str> = projection
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "🚨 磁盘用了 80%（还没解决）",
            "看看哪些占了大量空间",
            "🚨 磁盘用了 80%（还没解决）",
            "🚨 通勤路况"
        ]
    );
    let collapsed = &projection.messages[0];
    assert_eq!(collapsed.repeat_count, Some(3));
    assert_eq!(collapsed.first_timestamp, Some(base));
    assert_eq!(
        collapsed.timestamp,
        base + 14_400,
        "the newest copy is the one shown"
    );
    assert_eq!(
        projection.messages[2].repeat_count, None,
        "a run of one carries no count"
    );
    let json = serde_json::to_value(&projection).unwrap();
    assert_eq!(json["messages"][0]["repeat_count"], 3);
    assert!(
        json["messages"][0].get("alert_group").is_none(),
        "grouping key is internal"
    );
    assert!(json["messages"][2].get("repeat_count").is_none());
}

#[tokio::test]
async fn webapp_vnext_research_stop_sets_the_stop_flag() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path));

    let denied = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/research/stop")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        denied.status().is_client_error(),
        "stop without CSRF must be refused, got {}",
        denied.status()
    );

    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/research/stop")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");

    let user_did = webapp_canonical_user_did(12345678);
    let flag = memory
        .load_skill_state(crate::search_turn::RESEARCH_STOP_SKILL, &user_did)
        .await
        .unwrap()
        .unwrap_or_default();
    assert_eq!(flag, "1");
}
