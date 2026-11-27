//! MemoryHandle actor CRUD: KB hygiene/lint, L2 facts, tasks and windowed recurrence, skill state, knowledge base entries, monitoring intents.
use super::*;

#[tokio::test]
async fn kb_hygiene_summary_counts_did_scoped_lint_and_chat_urgent_items() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:test:kb_hygiene";
    let chat_id = 77701;
    let fact_a = mem
        .store_l2(did, "client:legal", "deadline is Monday", None)
        .await
        .unwrap();
    let fact_b = mem
        .store_l2(did, "client:legal", "deadline is Wednesday", None)
        .await
        .unwrap();
    mem.store_lint_finding(
        "contradiction",
        Some(fact_a),
        Some(fact_b),
        Some(0.92),
        Some("facts disagree"),
    )
    .await
    .unwrap();
    mem.store_knowledge_entry(
        chat_id,
        "reference",
        "urgent",
        "Review filing",
        "Needs review",
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

    let summary = mem.kb_hygiene_summary(chat_id, did).await;
    assert_eq!(summary.l2_total, 2);
    assert_eq!(summary.pending_lint_findings, 1);
    assert_eq!(summary.pending_contradictions, 1);
    assert_eq!(summary.pending_connections, 0);
    assert_eq!(summary.urgent_kb_items, 1);
}

#[tokio::test]
async fn kb_hygiene_summary_does_not_count_cross_did_lint() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let fact_a = mem
        .store_l2(
            "did:aegis:test:other",
            "client:legal",
            "other deadline is Monday",
            None,
        )
        .await
        .unwrap();
    let fact_b = mem
        .store_l2(
            "did:aegis:test:other",
            "client:legal",
            "other deadline is Wednesday",
            None,
        )
        .await
        .unwrap();
    mem.store_lint_finding(
        "contradiction",
        Some(fact_a),
        Some(fact_b),
        Some(0.92),
        Some("other facts disagree"),
    )
    .await
    .unwrap();

    let summary = mem
        .kb_hygiene_summary(77702, "did:aegis:test:kb_hygiene")
        .await;
    assert_eq!(summary.l2_total, 0);
    assert_eq!(summary.pending_lint_findings, 0);
    assert_eq!(summary.pending_contradictions, 0);
}

#[tokio::test]
async fn update_lint_finding_status_for_did_respects_owner() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:test:lint_owner";
    let other_did = "did:aegis:test:lint_other";
    let fact_a = mem
        .store_l2(did, "client:legal", "deadline is Monday", None)
        .await
        .unwrap();
    let fact_b = mem
        .store_l2(did, "client:legal", "deadline is Wednesday", None)
        .await
        .unwrap();
    let finding_id = mem
        .store_lint_finding(
            "contradiction",
            Some(fact_a),
            Some(fact_b),
            Some(0.92),
            Some("facts disagree"),
        )
        .await
        .unwrap();

    let wrong_user = mem
        .update_lint_finding_status_for_did(
            finding_id,
            other_did,
            "dismissed",
            Some("dismissed_by_user"),
        )
        .await
        .unwrap();
    assert!(!wrong_user, "cross-DID lint update must be rejected");

    let right_user = mem
        .update_lint_finding_status_for_did(finding_id, did, "dismissed", Some("dismissed_by_user"))
        .await
        .unwrap();
    assert!(right_user, "owner DID should be able to dismiss finding");

    let already_reviewed = mem
        .update_lint_finding_status_for_did(finding_id, did, "resolved", Some("resolved_by_user"))
        .await
        .unwrap();
    assert!(
        !already_reviewed,
        "reviewed lint finding should not be overwritten"
    );

    let findings = mem
        .list_lint_findings_for_did(did, Some("dismissed"), 5)
        .await;
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, finding_id);
    assert_eq!(
        findings[0].user_action.as_deref(),
        Some("dismissed_by_user")
    );
    assert!(
        findings[0].resolved_at.is_some(),
        "reviewed finding should have resolved_at"
    );
}

/// try_send (fire-and-forget) on a full channel doesn't panic.
#[tokio::test]
async fn fire_and_forget_full_channel_no_panic() {
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
    // These fire-and-forget methods should not panic on dead channel
    dead_handle.track_command_usage("test_cmd", "user_123");
    dead_handle.delete_expired_replay_nonces(chrono::Utc::now().timestamp());
    // If we got here without panic, the test passes
}

// ── store_l2 edge case tests ────────────────────────────────────────

#[tokio::test]
async fn store_l2_without_embedding_returns_valid_id() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l2_no_emb";
    let id = mem
        .store_l2(did, "general", "A plain fact", None)
        .await
        .unwrap();
    assert!(id > 0, "should return a positive row ID");

    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].fact_summary, "A plain fact");
    assert_eq!(all[0].topic, "general");
}

#[tokio::test]
async fn store_l2_unicode_topic_and_fact() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l2_unicode";
    let topic = "服务器配置";
    let fact = "主服务器 IP 是 10.0.0.1，备份在 10.0.0.2";
    let id = mem.store_l2(did, topic, fact, None).await.unwrap();
    assert!(id > 0);

    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].topic, topic);
    assert_eq!(all[0].fact_summary, fact);
}

#[tokio::test]
async fn store_l2_with_meta_persists_confidence_and_ttl() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l2_meta";

    // Store a fact with confidence and expiration (far future)
    let expires = (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339();
    let id = mem
        .store_l2_with_meta(
            did,
            "learned:lesson",
            "Always verify",
            None,
            0.95,
            Some(&expires),
        )
        .await
        .unwrap();
    assert!(id > 0);

    // Fact should appear in topic prefix queries (not expired)
    let facts = mem
        .query_l2_by_topic_prefix(did, "learned:lesson")
        .await
        .unwrap();
    // No embedding → won't appear in embedding-required query
    assert!(facts.is_empty()); // embedding IS NOT NULL filter

    // But should appear in get_all_l2
    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].fact_summary, "Always verify");

    // Store a fact with past expiration
    let expired = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let emb = vec![1.0_f32; 4];
    let id2 = mem
        .store_l2_with_meta(
            did,
            "learned:lesson",
            "Stale rule",
            Some(&emb),
            0.7,
            Some(&expired),
        )
        .await
        .unwrap();
    assert!(id2 > 0);

    // Expired fact should be excluded from topic prefix queries
    let facts = mem
        .query_l2_by_topic_prefix(did, "learned:lesson")
        .await
        .unwrap();
    assert!(
        facts.is_empty(),
        "expired fact should not appear in queries"
    );
}

#[tokio::test]
async fn store_l2_multiple_facts_same_topic() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:ed25519:l2_multi";
    // Different embeddings for different facts (realistic — distinct facts produce
    // distinct embeddings). The write guard uses cosine dedup on same-topic entries.
    let emb1 = vec![1.0_f32, 0.0, 0.0, 0.0];
    let emb2 = vec![0.0_f32, 1.0, 0.0, 0.0];

    let id1 = mem
        .store_l2(did, "server", "IP is 10.0.0.1", Some(&emb1))
        .await
        .unwrap();
    let id2 = mem
        .store_l2(did, "server", "OS is Ubuntu 22.04", Some(&emb2))
        .await
        .unwrap();
    assert_ne!(id1, id2, "each fact should get a unique row ID");

    let all = mem.get_all_l2(did).await.unwrap();
    assert_eq!(all.len(), 2);
}

// ── Task CRUD tests ─────────────────────────────────────────────────

/// Helper to create a minimal task and return its ID.
async fn create_test_task(mem: &MemoryHandle, chat_id: i64, did: &str, title: &str) -> i64 {
    mem.store_task(
        chat_id,
        did,
        title,
        "task body",
        "normal",
        &["test".to_string()],
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
    .unwrap()
}

#[tokio::test]
async fn task_store_and_get_roundtrip() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:42";
    let id = mem
        .store_task(
            42,
            did,
            "Buy groceries",
            "milk, eggs, bread",
            "high",
            &["shopping".into(), "personal".into()],
            Some("2026-04-05"),
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
    assert!(id > 0);

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "Buy groceries");
    assert_eq!(task.body, "milk, eggs, bread");
    assert_eq!(task.priority, "high");
    assert_eq!(task.status, "active");
    assert_eq!(task.tags, vec!["shopping", "personal"]);
    assert_eq!(task.due_date, Some("2026-04-05".into()));
    assert_eq!(task.source, "user");
    assert_eq!(task.user_did, did);
    assert_eq!(task.chat_id, 42);
    assert_eq!(task.execution_mode, "notify");
    assert!(!task.pin_on_fire);
    assert!(!task.is_callback_origin);
    assert_eq!(task.chain_depth, 0);
}

#[tokio::test]
async fn task_store_with_scheduling_fields() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:50";
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_task(
            50,
            did,
            "Recurring check",
            "check server status",
            "urgent",
            &[],
            None,
            "system",
            Some(now + 3600),
            Some("time"),
            None,
            "llm_callback",
            Some("check CPU usage"),
            Some(900),
            Some("server_check"),
            Some(10),
            true,
            Some(now + 86400),
            true,
            2,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.trigger_at_unix, Some(now + 3600));
    assert_eq!(task.trigger_type, Some("time".into()));
    assert_eq!(task.execution_mode, "llm_callback");
    assert_eq!(task.context_text, Some("check CPU usage".into()));
    assert_eq!(task.interval_seconds, Some(900));
    assert_eq!(task.group_key, Some("server_check".into()));
    assert_eq!(task.max_fires, Some(10));
    assert!(task.pin_on_fire);
    assert_eq!(task.expires_at_unix, Some(now + 86400));
    assert!(task.is_callback_origin);
    assert_eq!(task.chain_depth, 2);
}

#[tokio::test]
async fn task_store_windowed_recurrence_roundtrips() {
    // A windowed recurring task persists all 4 recur_* fields and reads back
    // exactly ONE task (not N) with weekdays decoded from JSON.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_task(
            42,
            "did:aegis:test",
            "commute estimate",
            "estimate commute",
            "normal",
            &["commute".to_string()],
            None,
            "user",
            Some(now + 3600),
            Some("time"),
            None,
            "llm_callback",
            Some("estimate commute"),
            Some(1200),
            Some("commute_win"),
            None,
            false,
            Some(now + 86400),
            false,
            0,
            Some(vec![2, 3, 4]),
            Some("17:30"),
            Some("18:30"),
            Some("America/Los_Angeles"),
        )
        .await
        .unwrap();

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.recur_weekdays, Some(vec![2, 3, 4]));
    assert_eq!(task.recur_window_start.as_deref(), Some("17:30"));
    assert_eq!(task.recur_window_end.as_deref(), Some("18:30"));
    assert_eq!(task.recur_timezone.as_deref(), Some("America/Los_Angeles"));
    assert_eq!(task.interval_seconds, Some(1200));

    // Exactly one active task for this group_key (the whole point: one task, not N).
    let listed = mem.list_tasks(42, None, Some("active"), 50).await;
    let matching: Vec<_> = listed
        .iter()
        .filter(|t| t.group_key.as_deref() == Some("commute_win"))
        .collect();
    assert_eq!(matching.len(), 1, "windowed schedule must create ONE task");
}

#[tokio::test]
async fn reschedule_recurring_intent_snaps_windowed_to_next_window() {
    // ROOT guard (Codex F7): any NON-firing reschedule (e.g. the panel "skip"
    // action) of a windowed recurring intent must land in the allowed window.
    // The snap is centralized in the RescheduleRecurringIntent funnel, so a
    // caller may pass a raw out-of-window `prev + interval` and it is corrected.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
    let id = mem
        .store_task(
            42,
            "did:aegis:test",
            "commute",
            "estimate commute",
            "normal",
            &[],
            None,
            "user",
            Some(parse("2026-06-03T17:30:00-07:00")),
            Some("time"),
            None,
            "llm_callback",
            Some("estimate commute"),
            Some(1200),
            Some("commute_skip"),
            None,
            false,
            Some(parse("2026-07-03T00:00:00-07:00")),
            false,
            0,
            Some(vec![2, 3, 4]),
            Some("17:30"),
            Some("18:30"),
            Some("America/Los_Angeles"),
        )
        .await
        .unwrap();

    // Simulate a "skip" from the window end: prev 18:30 + 20min = 18:50 Wed,
    // which is OUTSIDE the 17:30-18:30 window.
    let out_of_window = parse("2026-06-03T18:50:00-07:00");
    mem.reschedule_recurring_intent(id, out_of_window, parse("2026-07-03T00:00:00-07:00"))
        .await
        .unwrap();

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(
        task.trigger_at_unix,
        Some(parse("2026-06-04T17:30:00-07:00")),
        "skip must snap the windowed intent to the next valid window start (Thu 17:30)"
    );
}

/// Helper: store a windowed (Tue-Thu 17:30-18:30 PT, 20min) recurring task.
#[cfg(test)]
async fn store_windowed_task(mem: &MemoryHandle, chat_id: i64, group: &str, trigger: i64) -> i64 {
    mem.store_task(
        chat_id,
        "did:aegis:test",
        "commute",
        "estimate commute",
        "normal",
        &[],
        None,
        "user",
        Some(trigger),
        Some("time"),
        None,
        "notify",
        Some("estimate commute"),
        Some(1200),
        Some(group),
        None,
        false,
        Some(trigger + 86400 * 30),
        false,
        0,
        Some(vec![2, 3, 4]),
        Some("17:30"),
        Some("18:30"),
        Some("America/Los_Angeles"),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn update_task_snaps_windowed_trigger_into_window() {
    // ROOT guard (Codex F8): the Telegram snooze ACTIVE path writes the snooze
    // target through update_task. For a windowed recurring task that write must
    // snap into the window — snoozing to 22:00 must not fire out-of-window.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
    let id = store_windowed_task(&mem, 42, "commute_upd", parse("2026-06-03T17:30:00-07:00")).await;

    let out_of_window = parse("2026-06-03T22:00:00-07:00"); // snooze to 22:00 Wed
    let updates = serde_json::json!({
        "trigger_at_unix": out_of_window,
        "status": "active",
        "clear_fields": ["fired_at"],
    });
    mem.update_task(id, 42, &updates.to_string()).await.unwrap();

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(
        task.trigger_at_unix,
        Some(parse("2026-06-04T17:30:00-07:00")),
        "snooze via update_task must snap a windowed recurring task to the next window"
    );
}

#[tokio::test]
async fn reschedule_intent_snaps_windowed_trigger_into_window() {
    // ROOT guard (Codex F8): the Telegram snooze INACTIVE path reactivates a
    // fired/cancelled intent via reschedule_intent — must also snap in-window.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
    let id = store_windowed_task(
        &mem,
        42,
        "commute_resched",
        parse("2026-06-03T17:30:00-07:00"),
    )
    .await;
    mem.mark_intent_fired(id).await.unwrap(); // → status 'fired' (snooze-inactive precondition)

    let out_of_window = parse("2026-06-03T22:00:00-07:00");
    let ok = mem
        .reschedule_intent(
            id,
            "did:aegis:test",
            out_of_window,
            out_of_window + 86400 * 30,
        )
        .await
        .unwrap();
    assert!(ok, "reschedule_intent should update the fired row");

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(
        task.trigger_at_unix,
        Some(parse("2026-06-04T17:30:00-07:00")),
        "reschedule_intent must snap a windowed recurring task to the next window"
    );
}

#[tokio::test]
async fn task_store_no_window_reads_recur_none() {
    // BACKWARD-COMPAT: a classic recurring task (recur_* all None) reads back
    // with all recur_* None and fires on the old fixed-interval path.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    let id = mem
        .store_task(
            42,
            "did:aegis:test",
            "classic",
            "classic recurring",
            "normal",
            &[],
            None,
            "user",
            Some(now + 3600),
            Some("time"),
            None,
            "llm_callback",
            Some("classic"),
            Some(900),
            Some("classic_grp"),
            None,
            false,
            Some(now + 86400),
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let task = mem.get_task(id).await.unwrap();
    assert!(task.recur_weekdays.is_none());
    assert!(task.recur_window_start.is_none());
    assert!(task.recur_window_end.is_none());
    assert!(task.recur_timezone.is_none());
}

#[tokio::test]
async fn task_store_empty_tags() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = create_test_task(&mem, 10, "did:aegis:telegram:10", "No tags task").await;
    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.tags, vec!["test"]); // from helper
}

#[tokio::test]
async fn task_get_nonexistent_returns_none() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let task = mem.get_task(99999).await;
    assert!(task.is_none(), "nonexistent task should return None");
}

#[tokio::test]
async fn task_update_title_and_priority() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:60";
    let id = create_test_task(&mem, 60, did, "Original title").await;

    let updates = serde_json::json!({
        "title": "Updated title",
        "priority": "urgent"
    });
    let updated = mem.update_task(id, 60, &updates.to_string()).await.unwrap();
    assert!(updated, "update should return true when fields changed");

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "Updated title");
    assert_eq!(task.priority, "urgent");
}

#[tokio::test]
async fn task_update_status_to_done() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:61";
    let id = create_test_task(&mem, 61, did, "Finish report").await;

    let now = chrono::Utc::now().to_rfc3339();
    let updates = serde_json::json!({
        "status": "done",
        "completed_at": now,
    });
    let updated = mem.update_task(id, 61, &updates.to_string()).await.unwrap();
    assert!(updated);

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.status, "done");
    assert!(task.completed_at.is_some());
}

#[tokio::test]
async fn task_update_empty_json_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:62";
    let id = create_test_task(&mem, 62, did, "No changes").await;

    let updated = mem.update_task(id, 62, "{}").await.unwrap();
    assert!(!updated, "empty update should return false");
}

#[tokio::test]
async fn task_update_wrong_chat_id_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:63";
    let id = create_test_task(&mem, 63, did, "Protected task").await;

    let updates = serde_json::json!({"title": "Hacked"});
    let updated = mem
        .update_task(id, 9999, &updates.to_string())
        .await
        .unwrap();
    assert!(!updated, "update with wrong chat_id should fail");

    // Verify original is unchanged
    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "Protected task");
}

#[tokio::test]
async fn task_update_invalid_json_returns_error() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:64";
    let id = create_test_task(&mem, 64, did, "Bad update").await;

    let result = mem.update_task(id, 64, "not valid json!!").await;
    assert!(result.is_err(), "invalid JSON should return an error");
}

#[tokio::test]
async fn task_update_null_value_is_skipped() {
    // Semantic: absent key = preserve. `null` is treated as "caller forgot to
    // strip the key", NOT as a clear signal. Prevents NOT-NULL violations when
    // handlers build updates via `args.get(k)` for fields the user omitted.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:65";
    let id = mem
        .store_task(
            65,
            did,
            "Has due date",
            "",
            "normal",
            &[],
            Some("2026-05-01"),
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

    // Pass null alongside a real change — null is skipped, title is applied.
    let updates = serde_json::json!({"due_date": null, "title": "Updated"});
    let updated = mem.update_task(id, 65, &updates.to_string()).await.unwrap();
    assert!(updated);

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "Updated");
    assert_eq!(
        task.due_date.as_deref(),
        Some("2026-05-01"),
        "null value must be skipped, preserving the original due_date"
    );
}

#[tokio::test]
async fn task_update_clear_fields_sets_nullable_column_to_null() {
    // clear_fields is the explicit clear mechanism for nullable columns
    // (introduced alongside the 2026-04-17 null-skip fix, so `{col: null}`
    // no longer works). Only CLEAR_FIELDS_ALLOWED columns are honored.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:68";
    let id = create_test_task(&mem, 68, did, "Seeded").await;

    // Seed nullable action timestamps so there's something to clear.
    let updates = serde_json::json!({
        "status": "done",
        "completed_at": "2026-02-02T00:00:00Z",
        "fired_at": "2026-02-02T00:00:00Z",
    });
    mem.update_task(id, 68, &updates.to_string()).await.unwrap();
    let pre = mem.get_task(id).await.unwrap();
    assert_eq!(pre.completed_at.as_deref(), Some("2026-02-02T00:00:00Z"));
    assert_eq!(pre.fired_at.as_deref(), Some("2026-02-02T00:00:00Z"));

    // Restore via clear_fields
    let updates = serde_json::json!({
        "status": "active",
        "clear_fields": ["completed_at", "fired_at"],
    });
    let updated = mem.update_task(id, 68, &updates.to_string()).await.unwrap();
    assert!(updated);
    let post = mem.get_task(id).await.unwrap();
    assert_eq!(post.status, "active");
    assert!(
        post.completed_at.is_none(),
        "clear_fields must set completed_at to NULL"
    );
    assert!(
        post.fired_at.is_none(),
        "clear_fields must set fired_at to NULL so snoozed reminders can fire again"
    );

    // Requests to clear a NOT NULL column are silently ignored (no crash).
    let updates = serde_json::json!({
        "title": "Still here",
        "clear_fields": ["title"], // title is NOT NULL — ignored
    });
    let updated = mem.update_task(id, 68, &updates.to_string()).await.unwrap();
    assert!(updated);
    let after = mem.get_task(id).await.unwrap();
    assert_eq!(
        after.title, "Still here",
        "clear_fields must not bypass NOT NULL protections"
    );
}

#[tokio::test]
async fn task_update_only_null_fields_returns_false() {
    // If every key is null, nothing is set → update returns false (no-op).
    // This prevents silent NOT-NULL violations: the caller gets a clear signal
    // that their update did nothing.
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:67";
    let id = create_test_task(&mem, 67, did, "Unchanged").await;

    let updates = serde_json::json!({"title": null, "body": null, "priority": null});
    let updated = mem.update_task(id, 67, &updates.to_string()).await.unwrap();
    assert!(!updated, "all-null updates should be a no-op");

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "Unchanged");
    assert_eq!(task.body, "task body"); // from helper
}

#[tokio::test]
async fn task_update_tags() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:66";
    let id = create_test_task(&mem, 66, did, "Tag update test").await;

    let updates = serde_json::json!({"tags": ["new_tag1", "new_tag2"]});
    let updated = mem.update_task(id, 66, &updates.to_string()).await.unwrap();
    assert!(updated);

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.tags, vec!["new_tag1", "new_tag2"]);
}

#[tokio::test]
async fn task_delete_done_task() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:70";
    let id = create_test_task(&mem, 70, did, "Task to delete").await;

    // Mark as done first
    let updates = serde_json::json!({"status": "done"});
    mem.update_task(id, 70, &updates.to_string()).await.unwrap();

    let deleted = mem.delete_task(id, 70).await.unwrap();
    assert!(deleted, "done task should be deletable");

    let task = mem.get_task(id).await;
    assert!(task.is_none(), "task should be gone after delete");
}

#[tokio::test]
async fn task_delete_archived_task() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:71";
    let id = create_test_task(&mem, 71, did, "Archive then delete").await;

    let updates = serde_json::json!({"status": "archived"});
    mem.update_task(id, 71, &updates.to_string()).await.unwrap();

    let deleted = mem.delete_task(id, 71).await.unwrap();
    assert!(deleted, "archived task should be deletable");
}

#[tokio::test]
async fn task_delete_active_task_rejected() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:72";
    let id = create_test_task(&mem, 72, did, "Active task").await;

    // Attempt to delete while still active
    let deleted = mem.delete_task(id, 72).await.unwrap();
    assert!(!deleted, "active task should not be deletable");

    // Task should still exist
    let task = mem.get_task(id).await;
    assert!(task.is_some(), "active task should not be removed");
}

#[tokio::test]
async fn task_delete_wrong_chat_id_rejected() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:73";
    let id = create_test_task(&mem, 73, did, "Cross-chat delete").await;

    let updates = serde_json::json!({"status": "done"});
    mem.update_task(id, 73, &updates.to_string()).await.unwrap();

    let deleted = mem.delete_task(id, 9999).await.unwrap();
    assert!(!deleted, "wrong chat_id should prevent delete");
}

#[tokio::test]
async fn task_delete_nonexistent_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let deleted = mem.delete_task(99999, 1).await.unwrap();
    assert!(!deleted, "deleting nonexistent task should return false");
}

#[tokio::test]
async fn task_list_filters_by_status() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:80";

    let _id1 = create_test_task(&mem, 80, did, "Active task").await;
    let id2 = create_test_task(&mem, 80, did, "Done task").await;
    let _id3 = create_test_task(&mem, 80, did, "Another active").await;

    let updates = serde_json::json!({"status": "done"});
    mem.update_task(id2, 80, &updates.to_string())
        .await
        .unwrap();

    let active = mem.list_tasks(80, None, Some("active"), 50).await;
    // id1 gets cancelled by auto-dedup when id3 is stored (same group_key from title),
    // so we may have 1 or 2 active tasks. Just verify no "done" tasks appear.
    for t in &active {
        assert_eq!(t.status, "active", "should only return active tasks");
    }

    let done = mem.list_tasks(80, None, Some("done"), 50).await;
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id, id2);
}

#[tokio::test]
async fn task_list_empty_returns_empty() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let tasks = mem.list_tasks(99999, None, None, 50).await;
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn task_unicode_title_and_body() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:81";
    let id = mem
        .store_task(
            81,
            did,
            "审批合同",
            "需要在周五前审批XX合同，金额500万",
            "urgent",
            &["合同".into(), "财务".into()],
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

    let task = mem.get_task(id).await.unwrap();
    assert_eq!(task.title, "审批合同");
    assert!(task.body.contains("500万"));
    assert_eq!(task.tags, vec!["合同", "财务"]);
}

// ── Skill state additional tests ─────────────────────────────────────

#[tokio::test]
async fn skill_state_delete_nonexistent_succeeds() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // Deleting a key that never existed should succeed (no-op)
    let result = mem.delete_skill_state("phantom_skill", "no_such_key").await;
    assert!(result.is_ok(), "deleting nonexistent key should not error");
}

#[tokio::test]
async fn skill_state_store_empty_value() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("test_skill", "empty_val", "")
        .await
        .unwrap();
    let val = mem
        .load_skill_state("test_skill", "empty_val")
        .await
        .unwrap();
    assert_eq!(
        val,
        Some("".to_string()),
        "empty string should be stored and retrievable"
    );
}

#[tokio::test]
async fn skill_state_store_unicode_key_and_value() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    mem.store_skill_state("天气技能", "最后查询", "北京市海淀区")
        .await
        .unwrap();
    let val = mem.load_skill_state("天气技能", "最后查询").await.unwrap();
    assert_eq!(val, Some("北京市海淀区".to_string()));
}

#[tokio::test]
async fn skill_state_store_large_value_under_limit() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // Exactly 64KB should succeed
    let value = "x".repeat(64 * 1024);
    let result = mem.store_skill_state("big_skill", "big_key", &value).await;
    assert!(result.is_ok(), "value of exactly 64KB should be accepted");

    let loaded = mem.load_skill_state("big_skill", "big_key").await.unwrap();
    assert_eq!(loaded.as_deref(), Some(value.as_str()));
}

#[tokio::test]
async fn skill_state_delete_then_store_frees_slot() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // Fill to limit
    for i in 0..100 {
        mem.store_skill_state("slot_skill", &format!("k{i}"), "v")
            .await
            .unwrap();
    }
    // At limit, new key fails
    assert!(
        mem.store_skill_state("slot_skill", "overflow", "v")
            .await
            .is_err()
    );

    // Delete one key, then adding a new one should succeed
    mem.delete_skill_state("slot_skill", "k50").await.unwrap();
    let result = mem
        .store_skill_state("slot_skill", "new_key", "new_val")
        .await;
    assert!(
        result.is_ok(),
        "should be able to add after deleting to free a slot"
    );
}

// ── Knowledge entry tests ────────────────────────────────────────────

#[tokio::test]
async fn kb_store_with_all_optional_fields() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let emb = vec![0.5_f32, -0.3, 0.8];
    let id = mem
        .store_knowledge_entry(
            200,
            "decision",
            "urgent",
            "采用微服务架构",
            "经过讨论决定将单体应用拆分为微服务",
            &["架构".into(), "技术".into()],
            &[10, 20],
            Some(42),
            Some("团队会议总结"),
            Some("2026-04-15"),
            Some("CTO审批"),
            Some(&emb),
        )
        .await
        .unwrap();
    assert!(id > 0);

    let entry = mem.get_knowledge_entry(id, 200).await.unwrap();
    assert_eq!(entry.category, "decision");
    assert_eq!(entry.priority, "urgent");
    assert_eq!(entry.title, "采用微服务架构");
    assert!(entry.body.contains("微服务"));
    assert_eq!(entry.tags, vec!["架构", "技术"]);
    assert_eq!(entry.related_ids, vec![10, 20]);
    assert_eq!(entry.source_msg_id, Some(42));
    assert_eq!(entry.source_summary, Some("团队会议总结".into()));
    assert_eq!(entry.due_date, Some("2026-04-15".into()));
    assert_eq!(entry.waiting_on, Some("CTO审批".into()));
    assert_eq!(entry.status, "active");
}

#[tokio::test]
async fn kb_store_minimal_fields() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            201,
            "insight",
            "low",
            "Minimal entry",
            "Just a body",
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
    assert!(id > 0);

    let entry = mem.get_knowledge_entry(id, 201).await.unwrap();
    assert_eq!(entry.title, "Minimal entry");
    assert!(entry.tags.is_empty());
    assert!(entry.related_ids.is_empty());
    assert!(entry.source_msg_id.is_none());
    assert!(entry.due_date.is_none());
    assert!(entry.waiting_on.is_none());
}

#[tokio::test]
async fn kb_get_nonexistent_returns_none() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let entry = mem.get_knowledge_entry(99999, 1).await;
    assert!(entry.is_none());
}

#[tokio::test]
async fn kb_get_wrong_chat_id_returns_none() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            300,
            "reference",
            "normal",
            "Chat 300 only",
            "body",
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

    let entry = mem.get_knowledge_entry(id, 999).await;
    assert!(entry.is_none(), "wrong chat_id should not return the entry");
}

#[tokio::test]
async fn kb_update_multiple_fields() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            310,
            "insight",
            "normal",
            "Original insight",
            "Original body",
            &["old_tag".into()],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let updated = mem
        .update_knowledge_entry(
            id,
            310,
            KnowledgeEntryUpdate {
                title: Some("Updated insight".into()),
                body: Some("Updated body with more detail".into()),
                priority: Some("high".into()),
                tags: Some(vec!["new_tag".into(), "important".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated);

    let entry = mem.get_knowledge_entry(id, 310).await.unwrap();
    assert_eq!(entry.title, "Updated insight");
    assert_eq!(entry.body, "Updated body with more detail");
    assert_eq!(entry.priority, "high");
    assert_eq!(entry.tags, vec!["new_tag", "important"]);
}

#[tokio::test]
async fn kb_update_wrong_chat_id_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id = mem
        .store_knowledge_entry(
            320,
            "metric",
            "normal",
            "Revenue",
            "100M",
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

    let updated = mem
        .update_knowledge_entry(
            id,
            9999,
            KnowledgeEntryUpdate {
                title: Some("Hacked".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!updated, "update with wrong chat_id should fail");
}

#[tokio::test]
async fn kb_update_nonexistent_returns_false() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let updated = mem
        .update_knowledge_entry(
            99999,
            1,
            KnowledgeEntryUpdate {
                status: Some("done".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!updated);
}

#[tokio::test]
async fn kb_update_superseded_by() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let id1 = mem
        .store_knowledge_entry(
            330,
            "decision",
            "normal",
            "Old decision",
            "outdated",
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
    let id2 = mem
        .store_knowledge_entry(
            330,
            "decision",
            "high",
            "New decision",
            "supersedes old",
            &[],
            &[id1],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let updated = mem
        .update_knowledge_entry(
            id1,
            330,
            KnowledgeEntryUpdate {
                status: Some("superseded".into()),
                superseded_by: Some(id2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated);

    let entry = mem.get_knowledge_entry(id1, 330).await.unwrap();
    assert_eq!(entry.status, "superseded");
    assert_eq!(entry.superseded_by, Some(id2));
}

// ── Monitoring intent additional tests ─────────────────────────────

#[tokio::test]
async fn monitoring_intent_delete_nonexistent_succeeds() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    // Deleting a nonexistent intent should succeed (no rows matched)
    let result = mem.delete_monitoring_intent(99999).await;
    assert!(
        result.is_ok(),
        "deleting nonexistent intent should not error"
    );
}

#[tokio::test]
async fn monitoring_intent_multiple_skills_same_user() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:500";
    let emb = vec![1.0_f32, 0.0, 0.0];

    mem.store_monitoring_intent(did, "crypto_tracker", "BTC price drop", 0.7, emb.clone())
        .await
        .unwrap();
    mem.store_monitoring_intent(did, "server_monitor", "CPU spike", 0.8, emb.clone())
        .await
        .unwrap();
    mem.store_monitoring_intent(did, "news_digest", "AI regulation", 0.6, emb.clone())
        .await
        .unwrap();

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert_eq!(
        intents.len(),
        3,
        "user should have 3 intents for different skills"
    );

    let skills: Vec<&str> = intents
        .iter()
        .map(|i| i.monitoring_skill.as_str())
        .collect();
    assert!(skills.contains(&"crypto_tracker"));
    assert!(skills.contains(&"server_monitor"));
    assert!(skills.contains(&"news_digest"));
}

#[tokio::test]
async fn monitoring_intent_delete_then_list_empty() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:501";
    let emb = vec![1.0_f32, 2.0, 3.0];

    let id1 = mem
        .store_monitoring_intent(did, "skill_a", "desc a", 0.5, emb.clone())
        .await
        .unwrap();
    let id2 = mem
        .store_monitoring_intent(did, "skill_b", "desc b", 0.6, emb.clone())
        .await
        .unwrap();

    mem.delete_monitoring_intent(id1).await.unwrap();
    mem.delete_monitoring_intent(id2).await.unwrap();

    let intents = mem.list_monitoring_intents(did).await.unwrap();
    assert!(intents.is_empty(), "all intents should be deleted");
}

#[tokio::test]
async fn monitoring_intent_list_empty_user() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();
    let intents = mem
        .list_monitoring_intents("did:aegis:telegram:nonexistent")
        .await
        .unwrap();
    assert!(intents.is_empty());
}
