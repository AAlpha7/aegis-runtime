//! Owner work queue projection: retired routes, safe metadata, blocked-task HITL inlining, work context/respond routes, Hercules delegation cards.
use super::*;

#[tokio::test]
async fn retired_webapp_assistant_alias_routes_return_not_found() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    for (method, path) in [
        ("GET", "/webapp/assistant/inbox"),
        ("POST", "/webapp/assistant/context"),
        ("POST", "/webapp/assistant/respond"),
        ("GET", "/webapp/console"),
        ("GET", "/webapp/console/panels"),
        ("GET", "/webapp/console/system"),
        ("GET", "/webapp/console/mail"),
        ("GET", "/webapp/console/memory"),
        // SDUI pipeline retired 2026-09-16 (negative control: these were
        // live, auth-gated routes until then).
        ("GET", "/webapp/schema/test-id"),
        ("POST", "/webapp/submit"),
        ("POST", "/webapp/action"),
        ("POST", "/webapp/cancel/test-id"),
        ("POST", "/webapp/stream-token"),
        ("GET", "/webapp/stream/test-id"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"focus_id":"assistant:general"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{method} {path}");
    }
}

#[tokio::test]
async fn webapp_owner_work_queue_projects_safe_metadata_only() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), None);
    let approval_secret = "send private Lillian billing statement to lawyer";
    let task_secret_title = "Call lawyer about Lillian billing";
    let task_secret_body = "Private account number 12345";
    let memory_secret_fact = "Lillian private memory source text account 24680";
    let ledger_secret_action_id = "private-lillian-billing-action";
    let ledger_secret_summary = "private statement provider account 67890";
    let ledger_secret_receipt = "provider-secret-receipt-abc123";

    memory
        .store_hitl_approval(
            "approval-sensitive-1",
            &serde_json::json!({
                "tool_name": "calendar_create_event",
                "summary": approval_secret,
                "body": approval_secret,
                "requested_by": "did:aegis:telegram:12345678"
            })
            .to_string(),
            "tool_approval",
            None,
            None,
            chrono::Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .unwrap();
    memory
        .store_hitl_approval(
            "approval-other-owner",
            &serde_json::json!({
                "tool_name": "calendar_create_event",
                "summary": "other owner private approval summary",
                "requested_by": "did:aegis:telegram:99999",
                "chat_id": 99999
            })
            .to_string(),
            "tool_approval",
            None,
            None,
            chrono::Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .unwrap();
    let task_id = memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            task_secret_title,
            task_secret_body,
            "high",
            &["private".to_string()],
            Some("2026-05-04"),
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
    let closed_task_id = memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            "closed high task title",
            "closed high task body",
            "high",
            &["private".to_string()],
            Some("2026-05-04"),
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
    {
        let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
        conn.execute(
            "UPDATE tasks SET status = 'cancelled' WHERE id = ?1",
            rusqlite::params![closed_task_id],
        )
        .unwrap();
    }
    let stale_memory_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "billing_review",
            memory_secret_fact,
            None,
        )
        .await
        .unwrap();
    memory
        .set_l2_pinned("did:aegis:telegram:12345678", stale_memory_id, true)
        .await
        .unwrap();
    memory
        .test_backdate_l2_last_accessed(stale_memory_id, 210)
        .await
        .unwrap();
    {
        let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: ledger_secret_action_id.into(),
                action_kind: crate::action_ledger::ActionKind::EmailSend,
                requested_by: Some("did:aegis:telegram:12345678".into()),
                chat_id: Some(12345678),
                input_summary: Some(ledger_secret_summary.into()),
                user_visible_summary: Some(ledger_secret_summary.into()),
                ..Default::default()
            })
            .unwrap();
        store
            .transition(
                ledger_secret_action_id,
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(ledger_secret_receipt.into()),
                    provider_request_id: Some("provider-request-secret".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: "other-owner-private-action".into(),
                action_kind: crate::action_ledger::ActionKind::EmailSend,
                requested_by: Some("did:aegis:telegram:99999".into()),
                chat_id: Some(99999),
                input_summary: Some("other owner private summary".into()),
                ..Default::default()
            })
            .unwrap();
    }

    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/owner-work/queue")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    for forbidden in [
        approval_secret,
        task_secret_title,
        task_secret_body,
        memory_secret_fact,
        ledger_secret_action_id,
        ledger_secret_summary,
        ledger_secret_receipt,
        "provider-request-secret",
        "approval-other-owner",
        "other owner private approval summary",
        "other-owner-private-action",
        "other owner private summary",
    ] {
        assert!(
            !body_text.contains(forbidden),
            "assistant inbox must not expose sensitive source payload: {forbidden}"
        );
    }

    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["owner"], "did:aegis:telegram:12345678");
    assert_eq!(json["counts"]["approvals"], 0);
    assert_eq!(json["counts"]["tasks"], 1);
    assert_eq!(json["counts"]["receipts"], 1);
    assert_eq!(json["counts"]["memory_reviews"], 1);
    assert_eq!(json["counts"]["sensitive"], 3);
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != format!("task:{closed_task_id}")),
        "closed task should stay out of the active owner work queue"
    );
    assert!(
        !body_text.contains("工具请求") && !body_text.contains("Tool request"),
        "a tool hold is one card on the chat thread, not a generic work card"
    );
    assert!(
        !body_text.contains("我在应用外动手前")
            && !body_text.contains("Review it before Aegis does anything outside the app.")
    );
    assert!(!body_text.contains("payload redacted"));
    assert!(!body_text.contains("status=active; priority=high"));
    assert!(!body_text.contains("status=executed; receipt=Present"));
    assert!(
        body_text.contains("私密高优先级待办") || body_text.contains("Private high-priority task")
    );
    assert!(
        body_text.contains("标题和正文不在这张列表里")
            || body_text.contains(
                "Task title and body stay private in this list; open details to review them."
            )
    );
    assert!(!body_text.contains("Private title/body stay closed"));
    assert!(
        body_text.contains("记忆复查：billing_review")
            || body_text.contains("Memory review: billing_review")
    );
    assert!(
        body_text.contains("继续依赖前，请你看一眼")
            || body_text.contains("Review it before Aegis keeps relying on it")
    );
    assert!(
        body_text.contains("打开核对发生了什么")
            || body_text
                .contains("Completed action. Receipt available. Open it to verify what happened.")
            || body_text.contains("最近记录：1 条已完成")
            || body_text.contains("1 completed today")
    );
    assert!(body_text.contains(&format!("task:{task_id}")));
}

#[tokio::test]
async fn blocked_task_projection_inlines_related_hitl_decision_actions() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), None);
    let chat_id = 12345678;
    let user_did = "did:aegis:telegram:12345678";
    let task_id = memory
        .store_task(
            chat_id,
            user_did,
            "Check primary mail",
            "Use calendar_reader, then triage important messages.",
            "normal",
            &["automation".to_string()],
            None,
            "user",
            None,
            None,
            None,
            "llm_callback",
            None,
            None,
            Some("primary-mail-triage"),
            None,
            false,
            None,
            true,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    memory
        .update_task(
            task_id,
            chat_id,
            &serde_json::json!({"status": "blocked_by_gate"}).to_string(),
        )
        .await
        .unwrap();
    memory
        .store_hitl_approval(
            "gate-mail-triage",
            &serde_json::json!({
                "approval_id": "gate-mail-triage",
                "tool_name": "calendar_reader",
                "tool_args": "{\"label\":\"primary\"}",
                "user_did": user_did,
                "chat_id": chat_id,
                "is_callback": true,
                "intent_group_key": "primary-mail-triage",
                "source_task_id": task_id,
                "source_chat_id": chat_id,
                "created_at": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
            "tool_approval",
            None,
            None,
            chrono::Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .unwrap();

    let projection = build_unified_inbox_projection(&state, chat_id, "en").await;
    let task_item = projection
        .items
        .iter()
        .find(|item| item.id == format!("task:{task_id}"))
        .expect("blocked task projected");
    assert!(
        !projection
            .items
            .iter()
            .any(|item| item.id == opaque_unified_inbox_id("approval", "gate-mail-triage")),
        "linked HITL approval should not duplicate the blocked task in owner work"
    );
    assert_eq!(task_item.status, "blocked_by_gate");
    assert_eq!(
        task_item
            .secondary_actions
            .iter()
            .map(|action| action.action_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["approval.approve", "approval.reject"]
    );
    assert!(
        task_item
            .next_safe_action
            .contains("Approve this run, skip it"),
        "blocked task should explain that the owner can decide in place"
    );

    let resolved = resolve_vnext_approval_id_for_item(&state, task_item)
        .await
        .expect("blocked task resolves to related HITL approval");
    assert_eq!(resolved, "gate-mail-triage");
}

#[test]
fn trust_console_labels_telegram_send_as_notification() {
    assert_eq!(
        trust_console_action_kind_label(crate::action_ledger::ActionKind::TelegramSend),
        "Notification sent"
    );
}

#[tokio::test]
async fn webapp_owner_work_queue_reports_unavailable_ledger_safely() {
    let state = webapp_test_state();
    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/owner-work/queue")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["source"] == "action_ledger" && source["status"] == "unavailable")
    );
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "system" && item["sensitivity"] == "S1")
    );
}

#[tokio::test]
async fn webapp_owner_work_context_requires_auth_and_csrf_for_cookie_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);

    let unauth = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/owner-work/context")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"focus_id":"email:1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauth
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .unwrap(),
        "no-store"
    );

    let cookie_token = create_test_web_console_session(&db_path);
    let missing_csrf = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/owner-work/context")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"focus_id":"email:1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webapp_owner_work_respond_public_route_is_retired() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let request_body = serde_json::json!({
        "schema_version": 1,
        "focus_id": GENERAL_ASSISTANT_FOCUS_ID,
        "intent_kind": "explain",
        "user_text": "Explain what I should review next."
    })
    .to_string();

    let unauth = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/owner-work/respond")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::NOT_FOUND);

    let cookie_token = create_test_web_console_session(&db_path);
    let authenticated = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/owner-work/respond")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_work_omits_unconfigured_hercules_even_with_stale_history() {
    let state = webapp_test_state_with_orchestrator(MemoryHandle::spawn_in_memory().unwrap(), None);
    state.memory.store_skill_state(
        crate::orchestrator_tools_delegation::HERCULES_LINKAGE_SKILL,
        "old-delegation",
        &serde_json::json!({"hercules_task_id": "old-delegation", "title": "Old task", "last_state": "in_progress"}).to_string(),
    ).await.unwrap();
    for locale in ["en", "zh"] {
        let projection = build_unified_inbox_projection(&state, 12345678, locale).await;
        assert!(
            !projection
                .sources
                .iter()
                .any(|source| source.source == "hercules")
        );
        assert!(
            !projection
                .items
                .iter()
                .any(|item| item.source == "hercules")
        );
    }
    assert!(
        state
            .memory
            .load_skill_state(
                crate::orchestrator_tools_delegation::HERCULES_LINKAGE_SKILL,
                "old-delegation"
            )
            .await
            .unwrap()
            .is_some(),
        "hiding a disabled integration must not delete history"
    );
}

#[tokio::test]
async fn owner_work_projects_hercules_delegations_as_cards() {
    let state = webapp_test_state_with_orchestrator(MemoryHandle::spawn_in_memory().unwrap(), None);
    state
        .orchestrator
        .as_ref()
        .unwrap()
        .set_hercules_client(Arc::new(crate::hercules_client::HerculesClient::new(
            "http://127.0.0.1:9",
            "test-token",
            false,
        )));
    let chat_id = 12345678_i64;
    for (id, link) in [
        (
            "t-review",
            serde_json::json!({
                "hercules_task_id": "t-review",
                "title": "Add --version flag",
                "repo": "aegis",
                "last_state": "review",
                "pr_url": "https://github.com/example/aegis/pull/12",
                "verified_sha": "abc1234",
                "created_at": "2026-09-05T08:00:00Z",
                "updated_at": "2026-09-05T09:00:00Z"
            }),
        ),
        (
            "t-running",
            serde_json::json!({
                "hercules_task_id": "t-running",
                "title": "Split crates",
                "repo": "aegis",
                "last_state": "in_progress",
                "created_at": "2026-09-05T08:00:00Z",
                "updated_at": "2026-09-05T08:30:00Z"
            }),
        ),
        (
            "t-done",
            serde_json::json!({
                "hercules_task_id": "t-done",
                "title": "Old task",
                "repo": "aegis",
                "last_state": "done",
                "created_at": "2026-09-01T08:00:00Z",
                "updated_at": "2026-09-02T08:00:00Z"
            }),
        ),
    ] {
        state
            .memory
            .store_skill_state(
                crate::orchestrator_tools_delegation::HERCULES_LINKAGE_SKILL,
                id,
                &link.to_string(),
            )
            .await
            .unwrap();
    }

    let projection = build_unified_inbox_projection(&state, chat_id, "en").await;
    assert!(
        projection
            .sources
            .iter()
            .any(|s| s.source == "hercules" && s.status == "available"),
        "hercules must be listed as a source: {:?}",
        projection
            .sources
            .iter()
            .map(|s| &s.source)
            .collect::<Vec<_>>()
    );
    let find = |task_id: &str| {
        projection
            .items
            .iter()
            .find(|item| item.kind == "delegation" && item.source_id == task_id)
            .unwrap_or_else(|| panic!("delegation card for {task_id} missing"))
    };
    let review = find("t-review");
    assert_eq!(review.title, "Add --version flag");
    assert_eq!(
        review.status, "needs_decision",
        "a PR waiting for a human is a decision"
    );
    assert_eq!(review.urgency, "high");
    assert!(
        review
            .summary
            .contains("https://github.com/example/aegis/pull/12"),
        "{}",
        review.summary
    );
    assert!(review.summary.contains("abc1234"), "{}", review.summary);
    assert_eq!(review.source, "hercules");
    assert_eq!(review.updated_at, "2026-09-05T09:00:00Z");

    let running = find("t-running");
    assert_eq!(running.status, "tracking");
    assert_eq!(running.urgency, "normal");
    assert!(!running.summary.contains("held"), "{}", running.summary);

    state
        .memory
        .store_skill_state(
            crate::orchestrator_tools_delegation::HERCULES_LINKAGE_SKILL,
            "t-held",
            &serde_json::json!({
                "hercules_task_id": "t-held", "title": "Held", "repo": "aegis",
                "last_state": "todo", "hold_code": "repo_readiness_incomplete",
                "created_at": "2026-09-05T08:00:00Z", "updated_at": "2026-09-05T08:10:00Z"
            })
            .to_string(),
        )
        .await
        .unwrap();
    let projection = build_unified_inbox_projection(&state, chat_id, "en").await;
    let held = projection
        .items
        .iter()
        .find(|item| item.kind == "delegation" && item.source_id == "t-held")
        .expect("held card");
    assert!(
        !held.summary.contains("repo_readiness_incomplete"),
        "hold slugs stay off the card: {}",
        held.summary
    );
    assert!(!held.summary.contains("held:"), "{}", held.summary);
    assert!(held.summary.contains("In progress"), "{}", held.summary);
    assert!(
        held.next_safe_action.contains("Mark it ready"),
        "{}",
        held.next_safe_action
    );

    let done = find("t-done");
    assert_eq!(done.status, "done");
    assert_eq!(done.urgency, "low");
    assert!(
        !done.next_safe_action.to_lowercase().contains("merged"),
        "done means verified, not merged: {}",
        done.next_safe_action
    );
}
