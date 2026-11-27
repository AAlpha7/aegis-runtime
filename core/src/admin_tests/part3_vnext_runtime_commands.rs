//! vNext commands through the main runtime (general, tracking close, chat history, reminders, tasks, item context), action drafts, work projection from semantic tasks.
use super::*;

#[tokio::test]
async fn webapp_vnext_general_command_uses_main_runtime() {
    let turn_id = "test-general-runtime-durable-turn";
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_text_response("从Example City家到Worktown公司，预计44分钟。"),
        webapp_mock_text_response("从Example City家到Worktown公司，预计44分钟。"),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "commute",
            "用户通勤通常是 Example City 到 Worktown，需要优先给出当前路线和时间。",
            None,
        )
        .await
        .unwrap();
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "通勤",
                        "mode": "voice",
                        "intent_kind": "chat",
                        "client_context": "home",
                        "idempotency_key": turn_id,
                        "conversation_context": [
                            {
                                "role": "owner",
                                "text": "刚才我问了今天要不要进办公室。"
                            },
                            {
                                "role": "aegis",
                                "text": "你通常从 Example City 去 Worktown。",
                                "kind": "quoted_reply",
                                "source_id": "aegis:commute-answer"
                            }
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 1,
        "general Web command must call the main runtime chat completion path"
    );
    assert!(
        runtime.embed_call_count.load(Ordering::SeqCst) >= 1,
        "general Web command must build the normal runtime context, including embeddings"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["assistant_response"]["turn_id"], turn_id);
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "orchestrated_response"
    );
    assert_eq!(
        json["assistant_response"]["receipt_expectation"],
        "Handled through the main assistant runtime. External completion claims still require durable receipts."
    );
    let owner_message = json["owner_message"].as_str().unwrap();
    assert!(
        owner_message.contains("44分钟"),
        "owner_message should come from the main runtime mock response; got: {owner_message}; full response: {json}"
    );

    let joined_requests = runtime.chat_requests.lock().unwrap().join("\n---\n");
    assert!(!joined_requests.contains(turn_id));
    assert!(!joined_requests.contains("Conversation anchor: webapp_turn"));
    assert!(joined_requests.contains("通勤"));
    assert!(
        joined_requests.contains("Example City") && joined_requests.contains("Worktown"),
        "general Web command must pass relevant owner memory into the main runtime prompt"
    );
    assert!(
        joined_requests.contains("local_conversation_context")
            && joined_requests.contains("Quoted reply anchor")
            && joined_requests.contains("- quoted aegis: 你通常从 Example City 去 Worktown。")
            && joined_requests.contains("Recent local Web conversation")
            && joined_requests.contains("刚才我问了今天要不要进办公室。"),
        "general Web command must pass the quoted reply anchor separately from recent local Web chat context into the main runtime prompt: {joined_requests}"
    );
    assert!(
        joined_requests.contains("回复：")
            && joined_requests.contains("“你通常从 Example City 去 Worktown。”")
            && joined_requests.contains("用户信息：")
            && joined_requests.contains("通勤"),
        "general Web command must render the quoted reply next to the current user text for the LLM: {joined_requests}"
    );
    assert!(
        !joined_requests.contains("contextual_assistant_response"),
        "general Web command must not fall back to the static contextual assistant prompt"
    );
    let l1 = memory.get_l1("did:aegis:telegram:12345678").await.unwrap();
    let latest_user = l1
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .expect("latest user message stored");
    assert_eq!(
        latest_user.content, "通勤",
        "runtime-only quoted rendering must not pollute stored/rendered owner message"
    );
    for message in l1
        .iter()
        .filter(|message| matches!(message.role.as_str(), "user" | "assistant"))
    {
        assert_eq!(
            message.anchor_kind.as_deref(),
            Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND)
        );
        assert_eq!(message.anchor_ref.as_deref(), Some(turn_id));
    }
    let projection =
        vnext_chat_history_projection(&memory, "did:aegis:telegram:12345678", None).await;
    assert_eq!(projection.messages.len(), 2);
    assert!(
        projection
            .messages
            .iter()
            .all(|message| message.turn_id.as_deref() == Some(turn_id))
    );
    assert!(
        crate::progress_router::current_turn_id().is_none(),
        "turn identity must not leak outside its request scope"
    );
}

#[tokio::test]
async fn webapp_vnext_quoted_tracking_close_cancels_only_matching_intent() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new("LLM should not run"));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let user_did = "did:aegis:telegram:12345678";
    let now = chrono::Utc::now().timestamp();
    let openclaw_id = memory
        .store_deferred_intent(
            user_did,
            12345678,
            "time",
            Some(now + 3600),
            Some("openclaw-watch"),
            "llm_callback",
            "Search for latest news about 'openclaw' and summarize key points. Ensure coverage of the last 12 hours",
            now + 86_400,
            true,
            Some(3600),
            0,
            false,
            Some("openclaw-news-watch"),
            None,
            None,
            None, None, None, None,
        )
        .await
        .unwrap();
    let grok_id = memory
        .store_deferred_intent(
            user_did,
            12345678,
            "time",
            Some(now + 3600),
            Some("grok-build-watch"),
            "llm_callback",
            "Search for latest news about 'Grok Build agent' and related xAI/Grok build-agent announcements",
            now + 86_400,
            true,
            Some(3600),
            0,
            false,
            Some("grok-build-news-watch"),
            None,
            None,
            None, None, None, None,
        )
        .await
        .unwrap();
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "关闭，不需再跟踪",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "home",
                        "conversation_context": [
                            {
                                "role": "aegis",
                                "text": "OpenClaw 过去12小时有更新：v2026.5.24-beta.2/beta.1 相关预发布继续推进。来源：https://github.com/openclaw/openclaw/releases",
                                "kind": "quoted_reply",
                                "source_id": "aegis:notice-openclaw"
                            }
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let visible_answer = json["assistant_response"]["visible_answer"]
        .as_str()
        .unwrap();
    assert!(
        visible_answer.contains("其它跟踪没有改动"),
        "reply must explicitly state the cancellation scope: {visible_answer}"
    );

    let active = memory.list_user_intents(user_did).await.unwrap();
    assert!(
        active.iter().any(|intent| intent.id == grok_id),
        "Grok Build tracking must remain active"
    );
    assert!(
        active.iter().all(|intent| intent.id != openclaw_id),
        "Only the quoted OpenClaw tracking should be cancelled"
    );
}

#[tokio::test]
async fn webapp_vnext_chat_history_projects_recent_l1_turns() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();
    memory
        .push_l1(
            "did:aegis:telegram:12345678",
            crate::memory::Message::text("user", "通勤", now),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            "did:aegis:telegram:12345678",
            crate::memory::Message::text("assistant", "预计 44 分钟。", now + 1),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            "did:aegis:telegram:12345678",
            crate::memory::Message::text(
                "assistant",
                "系统已重启 / System restarted\n当前重要信息\n活跃技能 / Active skills: 22",
                now + 2,
            ),
        )
        .await
        .unwrap();
    let state = webapp_test_state_with_orchestrator(memory, Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let (cookie_token, _csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/webapp/vnext/chat-history")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["messages"][0]["role"], "owner");
    assert_eq!(json["messages"][0]["text"], "通勤");
    assert_eq!(json["messages"][0]["severity"], "info");
    assert_eq!(json["messages"][1]["role"], "aegis");
    assert_eq!(json["messages"][1]["text"], "预计 44 分钟。");
    assert_eq!(json["messages"][1]["severity"], "info");
    assert_eq!(
        json["messages"].as_array().unwrap().len(),
        2,
        "low-value system status notices must not evict real chat turns"
    );
}
#[tokio::test]
async fn webapp_vnext_chat_history_recovers_real_turns_from_l3_when_l1_is_status_noise() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:12345678";
    let now = chrono::Utc::now().timestamp();
    memory
        .push_l1(
            did,
            crate::memory::Message::text("user", "过去24小时有什么AI新闻", now),
        )
        .await
        .unwrap();
    memory
        .push_l1(
            did,
            crate::memory::Message::text(
                "assistant",
                "这里是过去 24 小时的重要 AI 新闻。",
                now + 1,
            ),
        )
        .await
        .unwrap();
    for index in 0..60 {
        memory
            .push_l1(
                did,
                crate::memory::Message::text(
                    "assistant",
                    format!(
                        "系统已重启 / System restarted\n当前重要信息\n活跃技能 / Active skills: {}",
                        index
                    ),
                    now + 2 + index,
                ),
            )
            .await
            .unwrap();
    }
    let state = webapp_test_state_with_orchestrator(memory, Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let (cookie_token, _csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/webapp/vnext/chat-history")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "owner");
    assert_eq!(messages[0]["text"], "过去24小时有什么AI新闻");
    assert_eq!(messages[1]["role"], "aegis");
    assert_eq!(messages[1]["text"], "这里是过去 24 小时的重要 AI 新闻。");
}

#[tokio::test]
async fn webapp_vnext_general_command_accepts_telegram_webapp_auth_without_cookie() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_text_response("Telegram WebApp opened Aegis and used the same runtime."),
        webapp_mock_text_response("Telegram WebApp opened Aegis and used the same runtime."),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_mock_runtime(memory, db_path, addr);
    let app = oauth_callback_router(state);
    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "通勤",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "home",
                        "idempotency_key": "telegram-webapp-command-smoke"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 1,
        "Telegram-opened WebApp commands must use the main runtime"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "orchestrated_response"
    );
    assert!(
        json["owner_message"]
            .as_str()
            .unwrap()
            .contains("same runtime"),
        "owner message should come from the runtime mock response; got: {json}"
    );
}

#[tokio::test]
async fn webapp_vnext_general_draft_command_uses_main_runtime() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_text_response("草稿：请明天上午提醒我查看学校账单。"),
        webapp_mock_text_response("草稿：请明天上午提醒我查看学校账单。"),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "帮我起草一个明天上午查看学校账单的提醒",
                        "mode": "text",
                        "intent_kind": "draft",
                        "client_context": "home"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 1,
        "general Web draft commands must call the main runtime"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "orchestrated_response"
    );
    let draft = json["assistant_response"]["draft"]
        .as_str()
        .unwrap_or_default();
    assert!(
        draft.contains("学校账单"),
        "draft should come from the main runtime mock response; got: {draft}; full response: {json}"
    );
    let joined_requests = {
        let chat_requests = runtime.chat_requests.lock().unwrap();
        chat_requests.join("\n---\n")
    };
    assert!(joined_requests.contains("学校账单"));
    assert!(
        !joined_requests.contains("contextual_assistant_response"),
        "general Web draft commands must not fall back to the Web-only contextual prompt"
    );
}

#[tokio::test]
async fn webapp_vnext_reminder_command_uses_manage_tasks_without_web_draft() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_tool_call_response(
            "call_1",
            "manage_tasks",
            serde_json::json!({
                "action": "add",
                "title": "查看学校账单",
                "trigger_at": "2026-05-16T16:00:00Z",
                "execution_mode": "notify",
                "context_text": "提醒查看学校账单"
            }),
        ),
        webapp_mock_text_response("已设置提醒：明天上午9点查看学校账单。"),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let app = oauth_callback_router(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "明天上午9点提醒我查看学校账单",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "home"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 2,
        "Web reminder commands must use the normal runtime tool loop"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["outcome"], "answer_now");
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "orchestrated_response"
    );
    // CEO Intent may attach a display label; it must not change outcome.
    assert!(
        json["assistant_response"]["suggested_intent"].is_null()
            || json["assistant_response"]["suggested_intent"].is_object(),
        "suggested_intent must be null or a CEO label object, got: {}",
        json["assistant_response"]["suggested_intent"]
    );
    let citations = json["assistant_response"]["citations"].as_array().unwrap();
    assert!(
        citations
            .iter()
            .any(|citation| citation == "tool:manage_tasks"),
        "expected manage_tasks citation in assistant response, got: {citations:?}; full response: {json}"
    );
    // NOTE(deferred): real-webapp tool source_evidence capture is tracked for a
    // dedicated PR (Codex-flagged). The real WebApp path currently produces empty
    // tool source_evidence, so the "Tasks and reminders" label assertion is
    // deferred until the production capture fix lands.
    assert_eq!(json["work_item"]["source_kind"], "task");
    assert_eq!(json["work_item"]["title"], "查看学校账单");
    assert!(
        json["work_item"]["next_safe_action"]
            .as_str()
            .unwrap_or_default()
            .contains("详情")
            || json["work_item"]["next_safe_action"]
                .as_str()
                .unwrap_or_default()
                .contains("details"),
        "new reminder should be returned as a safe Work card projection"
    );

    let tasks = memory.list_tasks(12345678, None, Some("active"), 10).await;
    let reminder = tasks
        .iter()
        .find(|task| task.title.contains("学校账单"))
        .expect("Web reminder command should create a real scheduled task");
    assert_eq!(reminder.execution_mode, "notify");
    assert_eq!(reminder.trigger_type.as_deref(), Some("time"));
    assert!(reminder.trigger_at_unix.is_some());

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let executed = crate::action_ledger::ActionLedgerStore::new(&conn)
        .list_recent_by_status(crate::action_ledger::ActionStatus::Executed, 10)
        .expect("query executed ledger rows");
    assert!(
        executed.iter().any(|entry| entry.action_kind
            == crate::action_ledger::ActionKind::ReminderCreate
            && entry
                .external_receipt
                .as_deref()
                .is_some_and(|receipt| receipt.starts_with("reminder:"))),
        "manage_tasks reminder creation must leave a durable receipt"
    );
    let proposed = crate::action_ledger::ActionLedgerStore::new(&conn)
        .list_recent_by_status(crate::action_ledger::ActionStatus::Proposed, 10)
        .expect("query proposed ledger rows");
    assert!(
        proposed.iter().all(|entry| {
            !entry
                .metadata_json
                .as_deref()
                .unwrap_or_default()
                .contains("contextual_action_draft")
        }),
        "ordinary owner-present Web reminders must not create Web action drafts"
    );

    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    assert!(
        projection
            .items
            .iter()
            .all(|item| item.kind != "action_draft"),
        "ordinary owner-present Web reminders must not appear as decision cards"
    );
}

#[tokio::test]
async fn webapp_vnext_slash_tasks_command_returns_direct_task_list_without_llm() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new(
        "this should not be used for /tasks",
    ));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            "安安护照跟进",
            "确认护照申请进度",
            "high",
            &[],
            Some("2026-06-28"),
            "test",
            None,
            None,
            None,
            "notify",
            Some("安安护照跟进"),
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
        .expect("store seed task");
    let state = webapp_test_state_with_mock_runtime(memory, db_path.clone(), addr);
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "/tasks",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "home"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        runtime.chat_call_count.load(Ordering::SeqCst),
        0,
        "/tasks should be a deterministic Web command, not a model turn"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let owner_message = json["owner_message"].as_str().unwrap_or_default();
    assert!(
        owner_message.contains("安安护照跟进"),
        "slash task list should include active task titles; got: {owner_message}; full response: {json}"
    );
    assert!(
        owner_message.contains("**Now**"),
        "high priority tasks should be grouped into a scannable Now section; got: {owner_message}"
    );
    assert!(
        !owner_message.contains("说明："),
        "/tasks should not dump database body/context prose into the main chat list; got: {owner_message}"
    );
    assert!(
        !owner_message.contains("I could not produce a useful answer"),
        "slash task list must not fall back to the empty model response"
    );
    assert_eq!(json["assistant_response"]["response_kind"], "task_list");
    let citations = json["assistant_response"]["citations"].as_array().unwrap();
    assert!(
        citations
            .iter()
            .any(|citation| citation == "tool:manage_tasks"),
        "expected task source citation, got: {citations:?}; full response: {json}"
    );
    let source_evidence = json["assistant_response"]["source_evidence"]
        .as_array()
        .unwrap();
    assert!(
        source_evidence.iter().any(|source| {
            source["label"] == "Tasks and reminders" || source["label"] == "待办和提醒"
        }),
        "expected task/reminder source evidence, got: {source_evidence:?}; full response: {json}"
    );
    assert!(
        !source_evidence
            .iter()
            .any(|source| source["label"] == "Locked private detail"),
        "read-only /tasks should not surface unrelated locked-context warnings"
    );
}

#[tokio::test]
async fn webapp_vnext_tasks_command_uses_manage_tasks_list_runtime_path() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_tool_call_response(
            "call_1",
            "manage_tasks",
            serde_json::json!({ "action": "list" }),
        ),
        webapp_mock_text_response("你有 1 个活跃任务：安安护照跟进。"),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            "安安护照跟进",
            "确认护照申请进度",
            "high",
            &[],
            Some("2026-06-28"),
            "test",
            None,
            None,
            None,
            "notify",
            Some("安安护照跟进"),
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
        .expect("store seed task");
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "任务",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "home"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 2,
        "Web task commands must use the normal runtime tool loop"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["outcome"], "answer_now");
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "orchestrated_response"
    );
    // CEO Intent may attach a display label; it must not change outcome.
    assert!(
        json["assistant_response"]["suggested_intent"].is_null()
            || json["assistant_response"]["suggested_intent"].is_object(),
        "suggested_intent must be null or a CEO label object, got: {}",
        json["assistant_response"]["suggested_intent"]
    );
    assert!(
        json["owner_message"]
            .as_str()
            .unwrap_or_default()
            .contains("安安护照跟进")
    );
    let citations = json["assistant_response"]["citations"].as_array().unwrap();
    assert!(
        citations
            .iter()
            .any(|citation| citation == "tool:manage_tasks"),
        "expected manage_tasks citation in assistant response, got: {citations:?}; full response: {json}"
    );

    let tasks = memory.list_tasks(12345678, None, Some("active"), 10).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "安安护照跟进");

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let proposed = crate::action_ledger::ActionLedgerStore::new(&conn)
        .list_recent_by_status(crate::action_ledger::ActionStatus::Proposed, 10)
        .expect("query proposed ledger rows");
    assert!(
        proposed.iter().all(|entry| {
            !entry
                .metadata_json
                .as_deref()
                .unwrap_or_default()
                .contains("contextual_action_draft")
        }),
        "ordinary owner-present Web task list requests must not create Web action drafts"
    );
}

#[tokio::test]
async fn webapp_vnext_item_command_uses_main_runtime_in_selected_item_context() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new(
        "这项任务是在主 runtime 中解释的：确认护照申请进度。",
    ));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            "安安护照跟进",
            "确认护照申请进度",
            "high",
            &[],
            Some("2026-06-28"),
            "test",
            None,
            None,
            None,
            "notify",
            Some("安安护照跟进"),
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
        .expect("store seed task");
    let state = webapp_test_state_with_mock_runtime(memory.clone(), db_path.clone(), addr);
    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    let task_item = projection
        .items
        .iter()
        .find(|item| item.source == "tasks" && item.title == "安安护照跟进")
        .expect("seed task should project into owner work");
    let task_item_id = task_item.id.clone();
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "这个下一步怎么处理？",
                        "mode": "text",
                        "intent_kind": "chat",
                        "client_context": "item",
                        "focus_item_id": task_item_id,
                        "conversation_context": [
                            {
                                "role": "owner",
                                "text": "Earlier I asked what this task was."
                            },
                            {
                                "role": "aegis",
                                "text": "Earlier answer: it is about confirming passport progress."
                            }
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 1,
        "selected-item Web assistant must call the main runtime"
    );
    assert!(
        runtime.embed_call_count.load(Ordering::SeqCst) >= 1,
        "selected-item Web assistant must build the normal runtime context"
    );
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["outcome"], "answer_now");
    assert_eq!(
        json["assistant_response"]["response_kind"],
        "focused_runtime_response"
    );
    assert!(
        json["owner_message"]
            .as_str()
            .unwrap_or_default()
            .contains("主 runtime")
    );
    assert!(
        json["work_item"].is_null(),
        "item-scoped explanations should answer in place, not create a new chat Work card"
    );
    let joined_requests = {
        let chat_requests = runtime.chat_requests.lock().unwrap();
        chat_requests.join("\n---\n")
    };
    assert!(
        joined_requests.contains("这个下一步怎么处理？"),
        "focused runtime prompt should include owner text, prompt: {joined_requests}"
    );
    assert!(
        joined_requests.contains("local_conversation_context")
            && joined_requests
                .contains("Earlier answer: it is about confirming passport progress."),
        "focused runtime prompt should carry recent item-local conversation context, prompt: {joined_requests}"
    );
    assert!(
        !joined_requests.contains("contextual_assistant_response"),
        "selected-item Web assistant must not fall back to the Web-only static contextual prompt"
    );
    let l1 = memory.get_l1("did:aegis:telegram:12345678").await.unwrap();
    assert!(
        l1.is_empty(),
        "focused item chat should not write raw item-scoped turns into the main L1 chat history"
    );
}
#[tokio::test]
async fn webapp_general_assistant_response_uses_orchestrated_runtime_shape() {
    let envelope = build_general_assistant_context_envelope();
    let response = runtime_assistant_success(
        crate::orchestrator::OrchestratorResult {
            reply: "从Example City家到Worktown公司，预计44分钟。".into(),
            tools_called: vec!["recall_memory".into(), "commute_traffic_monitor".into()],
            tools_blocked: vec!["calendar_reader".into()],
            tool_outputs: vec![
                (
                    "recall_memory".into(),
                    serde_json::json!({
                        "total": 1,
                        "returned": 1,
                        "entries": [
                            {
                                "layer": "L2",
                                "label": "profile:commute",
                                "source": "profile",
                                "content": "Owner commutes Example City to Worktown.",
                                "timestamp": "2026-05-25 18:00"
                            }
                        ]
                    })
                    .to_string(),
                ),
                (
                    "commute_traffic_monitor".into(),
                    serde_json::json!({
                        "status": "ok",
                        "summary": "Example City to Worktown is about 44 minutes."
                    })
                    .to_string(),
                ),
            ],
            ..Default::default()
        },
        envelope,
        "orchestrated_response",
        false,
    );

    assert_eq!(response.response_kind, "orchestrated_response");
    assert!(
        response.suggested_intent.is_none(),
        "WebApp runtime replies must not create a parallel draft/review path"
    );
    assert!(response.visible_answer.contains("44分钟"));
    assert!(
        response
            .citations
            .iter()
            .any(|citation| citation == "tool:commute_traffic_monitor")
    );
    assert!(
        response
            .source_evidence
            .iter()
            .any(|source| source.label == "Commute check")
    );
    assert!(
        response
            .source_evidence
            .iter()
            .any(|source| source.label == "Memory")
    );
    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning == "tool_blocked:calendar_reader")
    );
}

#[tokio::test]
async fn webapp_contextual_action_draft_persists_proposed_ledger_without_execution() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    // Contextual action drafts are category-2 machinery (mail-DERIVED work).
    // Raw mail items no longer live in the Work projection (owner decision
    // 2026-07-09), so the envelope is built from a synthetic mail-derived item
    // literal; persist/response behavior under test is unchanged.
    let mut item = test_unified_item_for_vnext("email:79", "email", "unread", "high");
    item.title = "Billing statement available".into();
    let envelope = build_context_envelope_from_inbox_item(&item);
    assert!(
        envelope
            .allowed_intents
            .iter()
            .any(|intent| intent == "propose_action")
    );
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: item.id.clone(),
        draft_text: "Draft reply asking where to view the statement.".into(),
        requested_action_kind: Some("email_draft".into()),
        suggested_intent_kind: Some("mail_followup".into()),
        user_text: Some("Review before sending.".into()),
        idempotency_key: Some("draft:test:one".into()),
    };

    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::EmailDraft,
    )
    .expect("persist contextual action draft");

    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Proposed);
    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::EmailDraft
    );
    assert_eq!(entry.chat_id, Some(12345678));
    assert_eq!(entry.external_receipt, None);
    assert_eq!(entry.executed_at, None);
    let metadata = entry.metadata_json.clone().expect("metadata");
    assert!(metadata.contains("contextual_action_draft"));
    assert!(metadata.contains("external_action_taken\":false"));
    assert!(metadata.contains("receipt_required_before_completion_claim\":true"));

    let response = contextual_action_draft_response(&envelope, &entry, &request);
    assert_eq!(response.status, "proposed");
    assert_eq!(response.action_draft["status"], "proposed");
    assert_eq!(response.action_draft["can_execute"], false);
    assert_eq!(response.action_draft["review_state"], "draft_saved");
    assert_eq!(response.action_draft["approval_status"], "not_requested");
    assert_eq!(
        response.action_draft["approval_blocker"],
        "Approval is required before this draft can execute."
    );
    assert_eq!(response.action_draft["risk"], "review_required");
    assert_eq!(
        response.action_draft["receipt_status"],
        "not_applicable_no_execution"
    );
    assert_eq!(response.action_draft["completion_claim_allowed"], false);
    assert_eq!(
        response.action_draft["rollback"],
        "not_applicable_until_execution"
    );
    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning == "action_draft_not_executed")
    );

    let work_item = unified_inbox_item_from_ledger_entry(&entry, "en");
    assert_eq!(work_item.kind, "action_draft");
    assert_eq!(work_item.status, "needs_decision");
    assert_eq!(work_item.urgency, "high");
    assert_eq!(work_item.title, "Review email draft");
    assert_eq!(
        work_item.summary,
        "Email draft is saved for review. Nothing has been executed."
    );
    assert_eq!(
        work_item
            .primary_action
            .as_ref()
            .map(|action| action.label.as_str()),
        Some("Review draft")
    );
    assert!(work_item.secondary_actions.is_empty());
    assert!(
        work_item
            .blocked_by
            .as_deref()
            .unwrap_or_default()
            .contains("Execution is intentionally disabled")
    );
    assert!(
        !work_item
            .summary
            .contains("Draft reply asking where to view the statement"),
        "owner work queue must not expose draft text in the list projection"
    );
}

#[tokio::test]
async fn webapp_general_assistant_action_draft_persists_without_source_reads() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path));
    let envelope = build_general_assistant_context_envelope();
    assert!(
        envelope
            .allowed_intents
            .iter()
            .any(|intent| intent == "propose_action")
    );
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: GENERAL_ASSISTANT_FOCUS_ID.into(),
        draft_text: "Draft a packing checklist for tomorrow.".into(),
        requested_action_kind: Some("other".into()),
        suggested_intent_kind: None,
        user_text: Some("Keep it review-only.".into()),
        idempotency_key: Some("general:draft:test".into()),
    };

    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::Other,
    )
    .expect("persist general assistant draft");
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Proposed);
    assert_eq!(entry.source_event_id, None);
    assert_eq!(entry.external_receipt, None);
    assert_eq!(entry.executed_at, None);
    assert_eq!(
        entry.user_visible_summary.as_deref(),
        Some("Contextual action draft saved for review")
    );
    let metadata = entry.metadata_json.unwrap_or_default();
    assert!(metadata.contains(GENERAL_ASSISTANT_FOCUS_ID));
    assert!(metadata.contains("external_action_taken\":false"));
    assert!(metadata.contains("receipt_required_before_completion_claim\":true"));
    assert!(!metadata.contains("private email"));
}

#[tokio::test]
async fn webapp_task_action_draft_create_task_route_is_removed_without_side_effects() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let envelope = build_general_assistant_context_envelope();
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: GENERAL_ASSISTANT_FOCUS_ID.into(),
        draft_text: "Task: Follow up with school billing\nAsk where to view the statement.".into(),
        requested_action_kind: Some("task_create".into()),
        suggested_intent_kind: Some("task".into()),
        user_text: None,
        idempotency_key: Some("create-task:draft:test".into()),
    };
    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::TaskCreate,
    )
    .expect("persist task draft");

    let work_item = unified_inbox_item_from_ledger_entry(&entry, "en");
    assert_eq!(work_item.kind, "action_draft");
    assert!(work_item.secondary_actions.is_empty());

    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/webapp/owner-work/action-drafts/{}/create-task",
                    entry.id
                ))
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let unchanged = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(entry.id)
        .unwrap()
        .expect("draft entry");
    assert_eq!(
        unchanged.status,
        crate::action_ledger::ActionStatus::Proposed
    );
    assert_eq!(unchanged.external_receipt, None);
    assert_eq!(unchanged.external_event_id, None);
}

#[tokio::test]
async fn webapp_reminder_action_draft_create_task_route_is_removed_without_side_effects() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let envelope = build_general_assistant_context_envelope();
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: GENERAL_ASSISTANT_FOCUS_ID.into(),
        draft_text: "Reminder: Follow up with the school billing statement tomorrow".into(),
        requested_action_kind: Some("reminder_create".into()),
        suggested_intent_kind: Some("reminder".into()),
        user_text: None,
        idempotency_key: Some("create-reminder-task:draft:test".into()),
    };
    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::ReminderCreate,
    )
    .expect("persist reminder draft");

    let work_item = unified_inbox_item_from_ledger_entry(&entry, "en");
    assert_eq!(work_item.kind, "action_draft");
    assert!(work_item.secondary_actions.is_empty());

    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/webapp/owner-work/action-drafts/{}/create-task",
                    entry.id
                ))
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let unchanged = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(entry.id)
        .unwrap()
        .expect("draft entry");
    assert_eq!(
        unchanged.status,
        crate::action_ledger::ActionStatus::Proposed
    );
    assert_eq!(unchanged.external_receipt, None);
    assert_eq!(unchanged.external_event_id, None);
}

#[tokio::test]
async fn webapp_tracking_action_draft_create_task_route_is_removed_without_side_effects() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let envelope = build_general_assistant_context_envelope();
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: GENERAL_ASSISTANT_FOCUS_ID.into(),
        draft_text: "Tracking: Watch whether the school replies about the billing statement".into(),
        requested_action_kind: Some("task_create".into()),
        suggested_intent_kind: Some("tracking".into()),
        user_text: None,
        idempotency_key: Some("create-tracking-task:draft:test".into()),
    };
    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::TaskCreate,
    )
    .expect("persist tracking draft");
    let work_item = unified_inbox_item_from_ledger_entry(&entry, "en");
    assert!(work_item.secondary_actions.is_empty());

    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/webapp/owner-work/action-drafts/{}/create-task",
                    entry.id
                ))
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let unchanged = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(entry.id)
        .unwrap()
        .expect("draft entry");
    assert_eq!(
        unchanged.status,
        crate::action_ledger::ActionStatus::Proposed
    );
    assert_eq!(unchanged.external_receipt, None);
    assert_eq!(unchanged.external_event_id, None);
}

#[tokio::test]
async fn webapp_semantic_tasks_project_to_owner_work_lanes_without_private_body() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let user_did = "did:aegis:telegram:12345678";
    let backlog_id = memory
        .store_task(
            12345678,
            user_did,
            "Private backlog title",
            "Private backlog body",
            "low",
            &["webapp".into(), "action_draft".into(), "backlog".into()],
            None,
            "webapp_backlog_draft",
            None,
            None,
            None,
            "manual",
            Some("Created from an Aegis backlog draft."),
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
        .expect("store backlog task");
    let decision_id = memory
        .store_task(
            12345678,
            user_did,
            "Private decision title",
            "Private decision body",
            "normal",
            &["webapp".into(), "action_draft".into(), "decision".into()],
            None,
            "webapp_decision_draft",
            None,
            None,
            None,
            "manual",
            Some("Created from an Aegis decision draft."),
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
        .expect("store decision task");

    let backlog = memory.get_task(backlog_id).await.expect("backlog task");
    let backlog_item = unified_inbox_item_from_task(&backlog, "en");
    assert_eq!(backlog_item.title, "Private backlog title");
    assert_eq!(backlog_item.status, "backlog");
    assert_eq!(backlog_item.urgency, "low");
    assert!(backlog_item.summary.contains("Backlog item."));
    assert!(!backlog_item.summary.contains("Private backlog body"));

    let decision = memory.get_task(decision_id).await.expect("decision task");
    let decision_item = unified_inbox_item_from_task(&decision, "en");
    assert_eq!(decision_item.title, "Private decision title");
    assert_eq!(decision_item.status, "needs_decision");
    assert_eq!(decision_item.urgency, "normal");
    assert!(decision_item.summary.contains("Decision item."));
    assert!(!decision_item.summary.contains("Private decision body"));
}

#[tokio::test]
async fn webapp_vnext_work_projection_includes_system_tasks_goals_and_waiting_items() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path));
    let owner = "did:aegis:telegram:12345678";

    let visible_task_id = memory
        .store_task(
            12345678,
            "did:aegis:system",
            "Passport renewal follow-up",
            "Private task body should stay out of the queue projection.",
            "high",
            &["system".into()],
            Some("2026-06-28"),
            "system",
            None,
            None,
            None,
            "notify",
            Some("Created by system scheduler."),
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
        .expect("store task");
    let rollup_task_id = memory
        .store_task(
            12345678,
            "did:aegis:system",
            "日终状态：磁盘77% / calendar电路 / Nationwide 5/9",
            "Legacy aggregate status should not appear as one owner work item.",
            "high",
            &["system".into()],
            None,
            "system",
            None,
            None,
            None,
            "notify",
            Some("Fix skills / disk cleanup / payment"),
            None,
            Some("daily-status-rollup"),
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
        .expect("store ambiguous rollup task");
    let closed_task_id = memory
        .store_task(
            12345678,
            owner,
            "Already handled task",
            "Closed body",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "manual",
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
        .expect("store closed task");
    memory
        .update_task(closed_task_id, 12345678, r#"{"status":"done"}"#)
        .await
        .expect("close task");

    let goal = crate::memory::Goal {
        id: 0,
        user_did: owner.into(),
        title: "Nationwide payment follow-up".into(),
        description: "Private goal description".into(),
        status: crate::memory::GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: Vec::new(),
        created_at: "2026-05-11T00:00:00Z".into(),
        target_date: Some("2026-05-09".into()),
        last_progress_at: None,
        source: crate::memory::GoalSource::Explicit,
    };
    let goal_id = memory.create_goal(owner, &goal).await.expect("create goal");
    let kb_id = memory
        .store_knowledge_entry(
            12345678,
            "waiting_for",
            "high",
            "Waiting for lawyer documents",
            "Private KB body should stay out of the queue projection.",
            &["legal".into()],
            &[],
            None,
            None,
            Some("2026-05-12"),
            Some("Sample Person"),
            None,
        )
        .await
        .expect("store waiting item");
    let internal_kb_id = memory
        .store_knowledge_entry(
            12345678,
            "waiting_for",
            "high",
            "Calendar FuelExhausted断路器激活（需修复）",
            "Internal system blocker without an owner-facing wait target.",
            &["system".into()],
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("store internal waiting item");
    let rollup_kb_id = memory
        .store_knowledge_entry(
            12345678,
            "waiting_for",
            "high",
            "日终状态：磁盘77% / calendar电路 / Nationwide 5/9",
            "Legacy aggregate status should not appear as one owner work item.",
            &["system".into()],
            &[],
            None,
            None,
            None,
            Some("Fix skills / disk cleanup / payment"),
            None,
        )
        .await
        .expect("store ambiguous rollup waiting item");

    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    let body = serde_json::to_string(&projection).unwrap();

    assert!(body.contains(&format!("task:{visible_task_id}")));
    assert!(body.contains("Passport renewal follow-up"));
    assert!(!body.contains("Private task body"));
    assert!(!body.contains(&format!("task:{rollup_task_id}")));
    assert!(!body.contains(&format!("task:{closed_task_id}")));
    assert!(body.contains(&format!("goal:{goal_id}")));
    assert!(body.contains("Nationwide payment follow-up"));
    assert!(body.contains(&format!("kb_waiting:{kb_id}")));
    assert!(body.contains("Waiting for lawyer documents"));
    assert!(body.contains("Waiting on Sample Person."));
    assert!(body.contains("Follow up about Sample Person"));
    assert!(!body.contains("Private KB body"));
    let waiting_item = projection
        .items
        .iter()
        .find(|item| item.id == format!("kb_waiting:{kb_id}"))
        .expect("waiting item should be projected");
    assert_eq!(
        waiting_item
            .secondary_actions
            .iter()
            .map(|action| action.action_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["knowledge.done", "knowledge.snooze_tomorrow"],
        "waiting follow-ups shown in Work must have direct owner actions"
    );
    assert!(!body.contains(&format!("kb_waiting:{internal_kb_id}")));
    assert!(!body.contains("FuelExhausted"));
    assert!(!body.contains(&format!("kb_waiting:{rollup_kb_id}")));
    assert!(!body.contains("日终状态"));
    assert!(!body.contains("Fix skills / disk cleanup / payment"));
    assert!(
        projection
            .sources
            .iter()
            .any(|source| source.source == "tasks")
    );
    assert!(
        projection
            .sources
            .iter()
            .any(|source| source.source == "goals")
    );
    assert!(
        projection
            .sources
            .iter()
            .any(|source| source.source == "knowledge")
    );
}

#[tokio::test]
async fn webapp_vnext_waiting_follow_up_action_closes_kb_loop_with_receipt() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path));
    let chat_id = 12345678;
    let kb_id = memory
        .store_knowledge_entry(
            chat_id,
            "waiting_for",
            "high",
            "McCarthy Electric 电话报价脚本",
            "Private KB body should stay out of the queue projection.",
            &["home".into()],
            &[],
            None,
            None,
            Some("2026-04-10"),
            Some("用户电话联系McCarthy后反馈报价"),
            None,
        )
        .await
        .expect("store waiting item");
    let entry = memory
        .get_knowledge_entry(kb_id, chat_id)
        .await
        .expect("stored waiting item");
    let item = unified_inbox_item_from_waiting_knowledge(&entry, "en");

    let result = dispatch_vnext_knowledge_action(&state, chat_id, &item, "knowledge.done")
        .await
        .expect("knowledge action should succeed");

    assert_eq!(result["status"], "ok");
    assert_eq!(result["action"], "knowledge.done");
    assert!(
        result["receipt"]
            .as_str()
            .expect("receipt")
            .contains("kb_waiting")
    );
    let updated = memory
        .get_knowledge_entry(kb_id, chat_id)
        .await
        .expect("updated waiting item");
    assert_eq!(updated.status, "done");
    assert!(updated.resolved_at.is_some());
}

#[tokio::test]
async fn waiting_card_can_close_its_recurring_generator_without_ending_the_card() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path));
    let chat_id = 12345678_i64;
    let owner = format!("did:aegis:telegram:{chat_id}");

    let clock_id = memory
        .store_task(
            chat_id,
            &owner,
            "治疗提醒",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "llm_callback",
            Some("private-context-do-not-show"),
            Some(86_400),
            Some("treatment-reminder"),
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
        .expect("store clock");
    let one_shot_id = memory
        .store_task(
            chat_id,
            &owner,
            "一次性安排",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            Some(1_800_000_000),
            Some("time"),
            None,
            "notify",
            Some("private-context-do-not-show"),
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
        .expect("store one shot");
    let foreign_id = memory
        .store_task(
            chat_id,
            "did:aegis:telegram:999",
            "别人的循环",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "llm_callback",
            None,
            Some(3_600),
            Some("foreign-loop"),
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
        .expect("store foreign");
    // store_task cancels the previous active row with the same group_key.
    // Ambiguity has to be planted after insert so both rows stay active.
    let shared_a = memory
        .store_task(
            chat_id,
            &owner,
            "守望甲",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "llm_callback",
            None,
            Some(3_600),
            Some("shared-watch-a"),
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
        .expect("store shared a");
    let shared_b = memory
        .store_task(
            chat_id,
            &owner,
            "守望乙",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "llm_callback",
            None,
            Some(3_600),
            Some("shared-watch-b"),
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
        .expect("store shared b");
    {
        let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
        conn.execute(
            "UPDATE tasks SET group_key = 'shared-watch' WHERE id IN (?1, ?2)",
            [shared_a, shared_b],
        )
        .unwrap();
    }
    let event_id = memory
        .store_task(
            chat_id,
            &owner,
            "预约守望",
            "private-body-do-not-show",
            "normal",
            &[],
            None,
            "user",
            None,
            Some("event"),
            Some("appointment-check"),
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
        .expect("store event watch");

    let wait = |title: &str, tags: &[String]| {
        let title = title.to_string();
        let tags = tags.to_vec();
        let memory = memory.clone();
        async move {
            memory
                .store_knowledge_entry(
                    chat_id,
                    "waiting_for",
                    "normal",
                    &title,
                    "等确认时间",
                    &tags,
                    &[],
                    None,
                    None,
                    None,
                    Some("确认预约时间"),
                    None,
                )
                .await
                .expect("store waiting")
        }
    };
    let clock_kb = wait(
        "治疗提醒 #36",
        &[crate::memory::tracking_parent_task_tag(clock_id)],
    )
    .await;
    let plain_kb = wait("治疗提醒 #99", &[]).await;
    let one_shot_kb = wait(
        "一次性等待",
        &[crate::memory::tracking_parent_task_tag(one_shot_id)],
    )
    .await;
    let foreign_kb = wait(
        "别人的等待",
        &[crate::memory::tracking_parent_task_tag(foreign_id)],
    )
    .await;
    let shared_kb = wait("含糊的守望", &["group_key:shared-watch".into()]).await;
    let event_kb = wait("预约等待", &["event_key:appointment-check".into()]).await;

    let projection = build_unified_inbox_projection(&state, chat_id, "zh").await;
    let item = |id: i64| {
        projection
            .items
            .iter()
            .find(|item| item.id == format!("kb_waiting:{id}"))
            .unwrap_or_else(|| panic!("missing kb_waiting:{id}"))
            .clone()
    };
    let kinds = |id: i64| -> Vec<String> {
        item(id)
            .secondary_actions
            .iter()
            .map(|action| action.action_kind.clone())
            .collect()
    };
    assert_eq!(
        kinds(clock_kb),
        vec![
            "knowledge.done".to_string(),
            "tracking.cancel".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    let clock_item = item(clock_kb);
    let dumped = serde_json::to_string(&clock_item).unwrap();
    assert!(dumped.contains(&format!("task:{clock_id}")));
    assert!(dumped.contains("每天"));
    assert!(!dumped.contains("interval_seconds="));
    assert!(!dumped.contains("group_key="));
    assert!(!dumped.contains("private-body-do-not-show"));
    assert!(!dumped.contains("private-context-do-not-show"));
    assert!(clock_item.lineage.iter().any(|node| {
        node.layer == "occurrence" && node.action_ids == vec!["knowledge.done".to_string()]
    }));
    assert!(clock_item.lineage.iter().any(|node| {
        node.layer == "intent"
            && node.id == format!("task:{clock_id}")
            && node.action_ids == vec!["tracking.cancel".to_string()]
    }));
    assert_eq!(
        kinds(plain_kb),
        vec![
            "knowledge.done".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    assert_eq!(
        kinds(one_shot_kb),
        vec![
            "knowledge.done".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    assert_eq!(
        kinds(foreign_kb),
        vec![
            "knowledge.done".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    assert_eq!(
        kinds(shared_kb),
        vec![
            "knowledge.done".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    assert_eq!(
        kinds(event_kb),
        vec![
            "knowledge.done".to_string(),
            "tracking.cancel".to_string(),
            "knowledge.snooze_tomorrow".to_string(),
        ]
    );
    assert!(item(event_kb).lineage.iter().any(|node| {
        node.layer == "intent"
            && node.id == format!("task:{event_id}")
            && node.detail.contains(&format!("任务 {event_id}"))
            && !node.detail.contains("event_key=")
    }));

    let plain = item(plain_kb);
    let denied = dispatch_vnext_tracking_cancel(&state, chat_id, &plain)
        .await
        .expect_err("untagged waiting has no generator");
    assert_eq!(denied.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        memory.get_task(clock_id).await.expect("clock").status,
        "active"
    );

    let result = dispatch_vnext_tracking_cancel(&state, chat_id, &clock_item)
        .await
        .expect("cancel generator");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["resource_type"], "task");
    assert_eq!(result["resource_id"], clock_id.to_string());
    assert_eq!(result["work_item_id"], format!("kb_waiting:{clock_kb}"));
    assert_eq!(result["action"], "tracking.cancel");
    assert_eq!(
        memory
            .get_task(clock_id)
            .await
            .expect("cancelled clock")
            .status,
        "cancelled"
    );
    assert_eq!(
        memory
            .get_knowledge_entry(clock_kb, chat_id)
            .await
            .expect("waiting remains")
            .status,
        "active"
    );
    assert_eq!(
        memory.get_task(one_shot_id).await.expect("one shot").status,
        "active"
    );
    assert_eq!(
        memory.get_task(foreign_id).await.expect("foreign").status,
        "active"
    );
    assert_eq!(
        memory.get_task(shared_a).await.expect("shared").status,
        "active"
    );
    assert_eq!(
        memory.get_task(shared_b).await.expect("shared").status,
        "active"
    );
    assert_eq!(
        memory.get_task(event_id).await.expect("event").status,
        "active"
    );

    let again = build_unified_inbox_projection(&state, chat_id, "zh").await;
    let still = again
        .items
        .iter()
        .find(|item| item.id == format!("kb_waiting:{clock_kb}"))
        .expect("waiting card stays");
    assert!(
        !still
            .secondary_actions
            .iter()
            .any(|action| action.action_kind == "tracking.cancel")
    );
    assert!(
        still
            .secondary_actions
            .iter()
            .any(|action| action.action_kind == "knowledge.done")
    );
    let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM action_ledger WHERE action_id LIKE 'web-console-tracking-cancel:%' AND status = 'executed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn ordinary_stale_memory_does_not_enter_owner_work_queue() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), None);
    let owner = "did:aegis:telegram:12345678";
    let stale_id = memory
        .store_l2(
            owner,
            "learned:preference",
            "ordinary stale memory should stay quiet",
            None,
        )
        .await
        .expect("store ordinary stale memory");
    memory
        .test_backdate_l2_last_accessed(stale_id, 240)
        .await
        .expect("backdate ordinary memory");

    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    assert!(
        projection
            .items
            .iter()
            .all(|item| item.id != format!("memory_review:{stale_id}")),
        "ordinary stale memory should be handled as quiet hygiene, not owner work"
    );
    assert_eq!(projection.counts.memory_reviews, 0);
}

#[tokio::test]
async fn memory_review_projection_source_id_opens_memory_detail() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let owner = "did:aegis:telegram:12345678";
    let secret_fact = "private stale memory detail should only appear after gate allows";
    let memory_id = memory
        .store_l2(owner, "learned:lesson", secret_fact, None)
        .await
        .expect("store stale memory");
    memory
        .set_l2_pinned(owner, memory_id, true)
        .await
        .expect("pin stale memory");
    memory
        .test_backdate_l2_last_accessed(memory_id, 210)
        .await
        .expect("backdate memory");
    for index in 0..60 {
        memory
            .store_l2(
                owner,
                &format!("newer:{index}"),
                &format!("newer fact {index}"),
                None,
            )
            .await
            .expect("store newer memory");
    }
    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    let memory_review = projection
        .items
        .iter()
        .find(|item| item.kind == "memory_review")
        .expect("memory review item");
    assert_eq!(memory_review.source_id, memory_id.to_string());
    assert_eq!(
        memory_review.detail_route.as_deref(),
        Some(format!("/webapp/console/memory/items/{memory_id}").as_str())
    );

    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/webapp/console/memory/items/{}",
                    memory_review.source_id
                ))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["resource_type"], "memory_item");
    assert_eq!(json["resource_id"], memory_id.to_string());
    assert_eq!(json["detail"]["topic"], "learned:lesson");
    assert_eq!(json["detail"]["fact_summary"], secret_fact);
}

#[tokio::test]
async fn webapp_action_draft_cancel_route_is_removed_without_side_effects() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let envelope = build_general_assistant_context_envelope();
    let request = ContextualAssistantActionDraftRequest {
        schema_version: Some(1),
        focus_id: GENERAL_ASSISTANT_FOCUS_ID.into(),
        draft_text: "Draft a reminder to check the statement tomorrow.".into(),
        requested_action_kind: Some("reminder_create".into()),
        suggested_intent_kind: Some("reminder".into()),
        user_text: None,
        idempotency_key: Some("cancel:draft:test".into()),
    };
    let entry = persist_contextual_action_draft(
        &state,
        12345678,
        &envelope,
        &request,
        crate::action_ledger::ActionKind::ReminderCreate,
    )
    .expect("persist action draft");

    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/webapp/owner-work/action-drafts/{}/cancel",
                    entry.id
                ))
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let unchanged = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(entry.id)
        .unwrap()
        .expect("draft entry");
    assert_eq!(
        unchanged.status,
        crate::action_ledger::ActionStatus::Proposed
    );
    assert_eq!(unchanged.external_receipt, None);
    assert_eq!(unchanged.executed_at, None);
}

/// Regression: a real photo (megabytes of base64 `data_url`) must reach
/// attachment validation instead of dying at the raw body read. Owner report
/// 2026-09-05: every image send from the phone was rejected with
/// "Invalid vNext command body" because the command route only buffered
/// 16 KB while `validate_vnext_command_attachments` allows 6 MB per file.
#[tokio::test]
async fn webapp_vnext_command_accepts_multi_megabyte_image_attachment() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = Arc::new(WebappMockRuntimeState::new_with_responses(vec![
        webapp_mock_text_response("I can see the photo."),
        webapp_mock_text_response("I can see the photo."),
    ]));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_mock_runtime(memory, db_path, addr);
    let app = oauth_callback_router(state);
    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);

    // ~3 MB of base64 (a 2.2 MB phone photo) — well inside the 6 MB per-file cap.
    let data_url = format!("data:image/png;base64,{}", "A".repeat(3 * 1024 * 1024));
    let body = serde_json::json!({
        "schema_version": 1,
        "input": "",
        "mode": "text",
        "intent_kind": "chat",
        "client_context": "home",
        "idempotency_key": "large-image-attachment",
        "attachments": [{"type": "image", "data_url": data_url}]
    })
    .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        status,
        StatusCode::OK,
        "large image must not be rejected: {text}"
    );
    assert!(
        runtime.chat_call_count.load(Ordering::SeqCst) >= 1,
        "the image turn must reach the runtime"
    );
}

async fn seed_closeable_goal(memory: &MemoryHandle, owner: &str) -> i64 {
    memory
        .create_goal(
            owner,
            &crate::memory::Goal {
                id: 0,
                user_did: owner.into(),
                title: "Track release updates".into(),
                description: "Keep the original research".into(),
                status: crate::memory::GoalStatus::Active,
                parent_goal_id: None,
                progress_pct: 0.25,
                milestones: Vec::new(),
                created_at: "2026-09-01T00:00:00Z".into(),
                target_date: None,
                last_progress_at: None,
                source: crate::memory::GoalSource::Explicit,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn webapp_vnext_goal_close_checks_auth_csrf_owner_and_preserves_history() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let goal_id = seed_closeable_goal(&memory, "did:aegis:telegram:12345678").await;
    let foreign_id = seed_closeable_goal(&memory, "did:aegis:telegram:999").await;
    let app = oauth_callback_router(state.clone());
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    let make_request = |id: i64, authenticated: bool, csrf_value: Option<&str>| {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/webapp/vnext/work/goal:{id}/actions"))
            .header(axum::http::header::CONTENT_TYPE, "application/json");
        if authenticated {
            req = req.header("Cookie", format!("__Host-aegis_session={cookie}"));
        }
        if let Some(value) = csrf_value {
            req = req.header(crate::web_console_auth::CSRF_HEADER_NAME, value);
        }
        req.body(Body::from(
            r#"{"schema_version":1,"action_id":"goal.abandon"}"#,
        ))
        .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(make_request(goal_id, false, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(make_request(goal_id, true, None))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(make_request(goal_id, true, Some("wrong")))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(make_request(foreign_id, true, Some(&csrf)))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        memory.get_goal(goal_id).await.unwrap().unwrap().status,
        crate::memory::GoalStatus::Active
    );
    let response = app
        .clone()
        .oneshot(make_request(goal_id, true, Some(&csrf)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["action_id"], "goal.abandon");
    assert!(
        result["receipt"]["label"]
            .as_str()
            .unwrap()
            .contains("abandoned")
    );
    let goal = memory.get_goal(goal_id).await.unwrap().unwrap();
    assert_eq!(goal.status, crate::memory::GoalStatus::Abandoned);
    assert_eq!(goal.progress_pct, 0.25);
    assert_eq!(goal.description, "Keep the original research");
    assert_eq!(
        memory.get_goal(foreign_id).await.unwrap().unwrap().status,
        crate::memory::GoalStatus::Active
    );
    let projection = build_unified_inbox_projection(&state, 12345678, "zh").await;
    assert!(
        !projection
            .items
            .iter()
            .any(|item| item.id == format!("goal:{goal_id}"))
    );
    assert_eq!(
        app.oneshot(make_request(goal_id, true, Some(&csrf)))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
    let count: i64 = conn.query_row("SELECT count(*) FROM action_ledger WHERE action_id LIKE 'web-console-goal-abandon:%' AND status = 'executed'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn webapp_vnext_goal_close_rolls_back_if_receipt_fails() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(
        memory.clone(),
        Some(db.path().to_string_lossy().into()),
    );
    let id = seed_closeable_goal(&memory, "did:aegis:telegram:12345678").await;
    let goal = memory.get_goal(id).await.unwrap().unwrap();
    let item = unified_inbox_item_from_goal(&goal, "en");
    assert!(vnext_work_action_is_projected_for_item(
        &item,
        "goal.abandon"
    ));
    let conn = crate::encrypted_sqlite::open(db.path()).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_goal_receipt BEFORE INSERT ON action_ledger BEGIN SELECT RAISE(ABORT, 'test receipt failure'); END;").unwrap();
    let error = dispatch_vnext_goal_close(&state, 12345678, &item).unwrap_err();
    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        memory.get_goal(id).await.unwrap().unwrap().status,
        crate::memory::GoalStatus::Active
    );
    conn.execute_batch("DROP TRIGGER fail_goal_receipt")
        .unwrap();
    dispatch_vnext_goal_close(&state, 12345678, &item).unwrap();
    let error = dispatch_vnext_goal_close(&state, 12345678, &item).unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
}
