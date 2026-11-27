//! Memory/profile detail gates and owner memory actions (delete, pin, confirm, mark wrong, update) with action-ledger receipts; trust, task timeline and receipt detail gates.
use super::*;

#[tokio::test]
async fn webapp_memory_detail_requires_gate_before_raw_read() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_fact = "private legal billing memory account 12345";
    let fact_id = state
        .memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            secret_fact,
            None,
        )
        .await
        .unwrap();
    let cookie_token = create_test_web_console_session(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/webapp/console/memory/items/{fact_id}"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        !body_text.contains(secret_fact),
        "memory detail gate must not fetch or expose raw memory before MFA"
    );
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "mfa_required");
    assert_eq!(json["resource_type"], "memory_item");
    assert_eq!(json["required_level"], "totp_or_web_authn");
    assert_eq!(json["detail"]["status"], "locked");

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::RequireMfa
    );
    assert_eq!(audit.resource_type, "memory_item");
    assert_eq!(audit.purpose, "web_console_memory_detail");
    assert_eq!(
        audit.sensitivity,
        crate::data_access_gate::DataSensitivity::S3Sensitive
    );
}

#[tokio::test]
async fn webapp_memory_detail_rejects_invalid_id_before_gate_audit() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let cookie_token = create_test_web_console_session(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/console/memory/items/not-a-number")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap();
    assert!(
        audit.is_none(),
        "invalid memory item ids must be rejected before DataAccessGate audit"
    );
}

#[tokio::test]
async fn webapp_memory_detail_returns_raw_value_only_after_gate_allows() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_fact = "private memory detail allowed after mfa 67890";
    let fact_id = state
        .memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            secret_fact,
            None,
        )
        .await
        .unwrap();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/webapp/console/memory/items/{fact_id}"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["access"], "allowed");
    assert_eq!(json["detail"]["status"], "available");
    assert_eq!(json["detail"]["fact_summary"], secret_fact);
    assert_eq!(json["detail"]["topic"], "learned:context");
    assert_eq!(json["detail"]["did"], "did:aegis:telegram:12345678");

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::Allowed
    );
    assert_eq!(audit.resource_type, "memory_item");
    assert_eq!(audit.purpose, "web_console_memory_detail");
    assert!(audit.grant_id.is_some());
}

#[tokio::test]
async fn webapp_profile_detail_requires_gate_before_raw_read() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_value = "private profile slot value before mfa 13579";
    let secret_source = "private profile source before mfa";
    let now = chrono::Utc::now().to_rfc3339();
    state
        .memory
        .upsert_profile_entry(crate::memory::ProfileEntry {
            id: 0,
            agent_did: "did:aegis:telegram:12345678".into(),
            category: "preference".into(),
            key: "communication_style".into(),
            value: secret_value.into(),
            confidence: 0.91,
            source: secret_source.into(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    let cookie_token = create_test_web_console_session(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/console/memory/profile/preference/communication_style")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body_text.contains(secret_value));
    assert!(!body_text.contains(secret_source));
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "mfa_required");
    assert_eq!(json["resource_type"], "profile_slot");
    assert_eq!(json["resource_id"], "preference.communication_style");
    assert_eq!(json["detail"]["status"], "locked");

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::RequireMfa
    );
    assert_eq!(audit.resource_type, "profile_slot");
    assert_eq!(audit.purpose, "web_console_profile_detail");
}

#[tokio::test]
async fn webapp_profile_detail_returns_raw_value_only_after_gate_allows() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_value = "private profile slot value after mfa 24680";
    let secret_source = "private profile source after mfa";
    let now = chrono::Utc::now().to_rfc3339();
    state
        .memory
        .upsert_profile_entry(crate::memory::ProfileEntry {
            id: 0,
            agent_did: "did:aegis:telegram:12345678".into(),
            category: "preference".into(),
            key: "communication_style".into(),
            value: secret_value.into(),
            confidence: 0.93,
            source: secret_source.into(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/console/memory/profile/preference/communication_style")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["access"], "allowed");
    assert_eq!(json["detail"]["status"], "available");
    assert_eq!(json["detail"]["value"], secret_value);
    assert_eq!(json["detail"]["source"], secret_source);
    assert_eq!(json["detail"]["category"], "preference");
    assert_eq!(json["detail"]["key"], "communication_style");

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::Allowed
    );
    assert_eq!(audit.resource_type, "profile_slot");
    assert_eq!(audit.purpose, "web_console_profile_detail");
    assert!(audit.grant_id.is_some());
}

#[tokio::test]
async fn webapp_memory_delete_uses_owner_session_without_settings_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private memory delete owner-session 12345",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/delete"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_delete"));
    assert!(
        !memory
            .get_all_l2("did:aegis:telegram:12345678")
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == fact_id),
        "signed-in owner delete should remove the memory row without a Settings/MFA detour"
    );
}

#[tokio::test]
async fn webapp_memory_delete_records_action_ledger_receipt_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private memory delete allowed 67890",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/delete"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_delete"));

    assert!(
        !memory
            .get_all_l2("did:aegis:telegram:12345678")
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == fact_id),
        "owner delete must remove the memory row"
    );
    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(json["ledger_entry_id"].as_i64().unwrap())
        .unwrap()
        .expect("memory delete ledger row");
    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::KbDelete
    );
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
    assert!(entry.external_receipt.is_some());
}

#[tokio::test]
async fn webapp_memory_pin_uses_owner_session_without_settings_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private memory pin owner-session 12345",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/pin"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert_eq!(json["pinned"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_pin"));

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("pinned memory row");
    assert!(item.is_pinned);
}

#[tokio::test]
async fn webapp_memory_pin_records_action_ledger_receipt_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private memory pin allowed 67890",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/pin"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert_eq!(json["pinned"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_pin"));

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("pinned memory row");
    assert!(item.is_pinned);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(json["ledger_entry_id"].as_i64().unwrap())
        .unwrap()
        .expect("memory pin ledger row");
    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::KbUpdate
    );
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
    assert!(entry.external_receipt.is_some());
    let metadata = entry.metadata_json.unwrap_or_default();
    assert!(metadata.contains("\"resource_type\":\"memory_item\""));
    assert!(metadata.contains("\"pinned\":true"));
}

#[tokio::test]
async fn webapp_memory_confirm_works_for_stale_fact_outside_latest_l2_page() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let user_did = "did:aegis:telegram:12345678";
    let fact_id = memory
        .store_l2(
            user_did,
            "learned:preference",
            "private stale memory confirm outside latest page",
            None,
        )
        .await
        .unwrap();
    memory
        .test_backdate_l2_last_accessed(fact_id, 80)
        .await
        .unwrap();
    for index in 0..60 {
        memory
            .store_l2(
                user_did,
                "learned:context",
                &format!("newer memory row {index}"),
                None,
            )
            .await
            .unwrap();
    }
    assert!(
        !memory
            .get_all_l2(user_did)
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == fact_id),
        "test must cover stale facts outside the latest-50 L2 page"
    );

    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/confirm"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["confirmed"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_confirm"));
    assert!(
        memory
            .query_stale_facts(user_did, 60)
            .await
            .unwrap()
            .iter()
            .all(|item| item.id != fact_id),
        "confirming a memory should remove it from stale review"
    );
}

#[tokio::test]
async fn webapp_vnext_memory_review_action_confirms_projected_item() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let user_did = "did:aegis:telegram:12345678";
    let fact_id = memory
        .store_l2(
            user_did,
            "learned:preference",
            "private stale memory confirm through vnext work action",
            None,
        )
        .await
        .unwrap();
    memory.set_l2_pinned(user_did, fact_id, true).await.unwrap();
    memory
        .test_backdate_l2_last_accessed(fact_id, 210)
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/webapp/vnext/work/memory_review:{fact_id}/actions"
                ))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "action_id": "memory.confirm",
                        "idempotency_key": format!("memory-confirm-test:{fact_id}")
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["action_id"], "memory.confirm");
    assert!(
        json["owner_message"]
            .as_str()
            .unwrap_or_default()
            .contains(crate::i18n::t(
                "console.memory.confirmed",
                crate::i18n::DEFAULT_LOCALE,
            )),
        "vNext wrapper should return the memory confirm result, not a generic 400"
    );
}

#[tokio::test]
async fn webapp_memory_unpin_records_action_ledger_receipt_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private memory unpin allowed 67890",
            None,
        )
        .await
        .unwrap();
    assert!(
        memory
            .set_l2_pinned("did:aegis:telegram:12345678", fact_id, true)
            .await
            .unwrap()
    );
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/unpin"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert_eq!(json["pinned"], false);
    assert!(json["receipt"].as_str().unwrap().contains("memory_unpin"));

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("unpinned memory row");
    assert!(!item.is_pinned);
}

#[tokio::test]
async fn webapp_memory_mark_wrong_uses_owner_session_without_settings_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private wrong-memory owner-session 12345",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/mark-wrong"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert_eq!(json["marked_wrong"], true);
    assert!(
        json["receipt"]
            .as_str()
            .unwrap()
            .contains("memory_mark_wrong")
    );

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("marked-wrong memory row");
    assert_eq!(
        item.fact_summary,
        crate::i18n::t("console.memory.marked_wrong", crate::i18n::DEFAULT_LOCALE)
    );
    assert!(
        item.confidence == 0.0,
        "owner-session mark-wrong should lower confidence without a Settings/MFA detour"
    );
}

#[tokio::test]
async fn webapp_memory_mark_wrong_records_receipt_and_hides_original_fact_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "private wrong-memory allowed 67890",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/mark-wrong"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert_eq!(json["marked_wrong"], true);
    assert!(
        json["receipt"]
            .as_str()
            .unwrap()
            .contains("memory_mark_wrong")
    );
    assert_eq!(json["detail"]["confidence"], 0.0);
    assert!(
        !json["detail"]["fact_summary"]
            .as_str()
            .unwrap_or_default()
            .contains("67890"),
        "response tombstone must not echo the original raw fact"
    );

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("marked-wrong memory row");
    assert_eq!(
        item.fact_summary,
        crate::i18n::t("console.memory.marked_wrong", crate::i18n::DEFAULT_LOCALE)
    );
    assert_eq!(item.confidence, 0.0);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(json["ledger_entry_id"].as_i64().unwrap())
        .unwrap()
        .expect("memory mark-wrong ledger row");
    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::KbUpdate
    );
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
    assert!(entry.external_receipt.is_some());
    let metadata = entry.metadata_json.unwrap_or_default();
    assert!(metadata.contains("\"operation\":\"mark_wrong\""));
    assert!(!metadata.contains("67890"));
}

#[tokio::test]
async fn webapp_memory_update_unauthenticated_rejects_before_body_validation() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path));
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/memory/items/1/update")
                .header("Content-Type", "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_memory_update_uses_owner_session_without_settings_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "old memory value",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/update"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"fact_summary": "owner-session memory update"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_update"));

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("updated memory row");
    assert_eq!(item.fact_summary, "owner-session memory update");
    assert_eq!(item.confidence, 1.0);
}

#[tokio::test]
async fn webapp_memory_update_records_action_ledger_receipt_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let fact_id = memory
        .store_l2(
            "did:aegis:telegram:12345678",
            "learned:context",
            "old memory value",
            None,
        )
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/memory/items/{fact_id}/update"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"fact_summary": "new manually corrected memory value"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert!(json["receipt"].as_str().unwrap().contains("memory_update"));

    let item = memory
        .get_all_l2("did:aegis:telegram:12345678")
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fact_id)
        .expect("updated memory row");
    assert_eq!(item.fact_summary, "new manually corrected memory value");
    assert_eq!(item.confidence, 1.0);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let ledger_entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(json["ledger_entry_id"].as_i64().unwrap())
        .unwrap()
        .expect("memory update ledger row");
    assert_eq!(
        ledger_entry.action_kind,
        crate::action_ledger::ActionKind::KbUpdate
    );
    assert_eq!(
        ledger_entry.status,
        crate::action_ledger::ActionStatus::Executed
    );
    assert!(ledger_entry.external_receipt.is_some());
    let metadata = ledger_entry.metadata_json.unwrap_or_default();
    assert!(metadata.contains("\"resource_type\":\"memory_item\""));
    assert!(!metadata.contains("new manually corrected memory value"));
}

#[tokio::test]
async fn webapp_profile_update_unauthenticated_rejects_before_body_validation() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path));
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/memory/profile/preference/communication_style/update")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_profile_update_records_action_ledger_receipt_with_mfa_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let now = chrono::Utc::now().to_rfc3339();
    memory
        .upsert_profile_entry(crate::memory::ProfileEntry {
            id: 0,
            agent_did: "did:aegis:telegram:12345678".into(),
            category: "preference".into(),
            key: "communication_style".into(),
            value: "old concise profile value".into(),
            confidence: 0.71,
            source: "auto".into(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/memory/profile/preference/communication_style/update")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"value": "new manually corrected profile value"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert!(json["receipt"].as_str().unwrap().contains("profile_update"));

    let profile = memory
        .get_user_profile("did:aegis:telegram:12345678")
        .await
        .unwrap();
    let entry = profile
        .iter()
        .find(|entry| entry.category == "preference" && entry.key == "communication_style")
        .expect("updated profile slot");
    assert_eq!(entry.value, "new manually corrected profile value");
    assert_eq!(entry.source, "manual");
    assert_eq!(entry.confidence, 1.0);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let ledger_entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .get_by_id(json["ledger_entry_id"].as_i64().unwrap())
        .unwrap()
        .expect("profile update ledger row");
    assert_eq!(
        ledger_entry.action_kind,
        crate::action_ledger::ActionKind::KbUpdate
    );
    assert_eq!(
        ledger_entry.status,
        crate::action_ledger::ActionStatus::Executed
    );
    assert!(ledger_entry.external_receipt.is_some());
    let metadata = ledger_entry.metadata_json.unwrap_or_default();
    assert!(metadata.contains("\"resource_type\":\"profile_slot\""));
    assert!(!metadata.contains("new manually corrected profile value"));
}

#[tokio::test]
async fn webapp_profile_update_uses_owner_session_without_settings_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory.clone(), Some(db_path.clone()));
    let now = chrono::Utc::now().to_rfc3339();
    memory
        .upsert_profile_entry(crate::memory::ProfileEntry {
            id: 0,
            agent_did: "did:aegis:telegram:12345678".into(),
            category: "preference".into(),
            key: "communication_style".into(),
            value: "old profile value".into(),
            confidence: 0.71,
            source: "auto".into(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await
        .unwrap();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/console/memory/profile/preference/communication_style/update")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"value": "owner-session profile update"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["updated"], true);
    assert!(json["receipt"].as_str().unwrap().contains("profile_update"));

    let profile = memory
        .get_user_profile("did:aegis:telegram:12345678")
        .await
        .unwrap();
    let entry = profile
        .iter()
        .find(|entry| entry.category == "preference" && entry.key == "communication_style")
        .expect("profile slot");
    assert_eq!(entry.value, "owner-session profile update");
    assert_eq!(entry.source, "manual");
    assert_eq!(entry.confidence, 1.0);
}

#[tokio::test]
async fn webapp_trust_detail_placeholders_require_gate_before_raw_read() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_task_title = "private lawyer task title";
    let secret_task_body = "private billing account in task body 12345";
    let task_id = state
        .memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            secret_task_title,
            secret_task_body,
            "high",
            &["private".to_string()],
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
    let secret_summary = "private action receipt summary 12345";
    let entry_id = {
        let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
        crate::action_ledger::ActionLedgerStore::new(&conn)
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: "trust-detail-action".into(),
                action_kind: crate::action_ledger::ActionKind::EmailSend,
                requested_by: Some("did:aegis:telegram:12345678".into()),
                chat_id: Some(12345678),
                input_summary: Some(secret_summary.into()),
                user_visible_summary: Some(secret_summary.into()),
                ..Default::default()
            })
            .unwrap()
            .id
    };
    let cookie_token = create_test_web_console_session(&db_path);
    let app = oauth_callback_router(state);

    for (path, resource_type, purpose, forbidden) in [
        (
            format!("/webapp/console/tasks/{task_id}/timeline"),
            "task_timeline",
            "web_console_task_timeline",
            secret_task_body,
        ),
        (
            format!("/webapp/console/ledger/{entry_id}"),
            "action_ledger_entry",
            "web_console_action_receipt_detail",
            secret_summary,
        ),
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let body_text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !body_text.contains(forbidden),
            "{resource_type} placeholder must not expose raw detail before MFA"
        );
        let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
        assert_eq!(json["status"], "mfa_required");
        assert_eq!(json["resource_type"], resource_type);
        assert_eq!(json["required_level"], "totp_or_web_authn");
        assert_eq!(json["challenge_hint"]["start_url"], "/ui/?tab=system");

        let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
        let audit = crate::data_access_gate::latest_sensitive_read_audit(
            &conn,
            "did:aegis:telegram:12345678",
        )
        .unwrap()
        .expect("sensitive read audit");
        assert_eq!(audit.resource_type, resource_type);
        assert_eq!(audit.purpose, purpose);
        assert_eq!(
            audit.status,
            crate::data_access_gate::SensitiveReadAuditStatus::RequireMfa
        );
    }
}

#[tokio::test]
async fn webapp_local_login_session_can_open_task_body() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_task_title = "private lawyer task title";
    let secret_task_body = "private billing account in task body 12345";
    let task_id = state
        .memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            secret_task_title,
            secret_task_body,
            "high",
            &["private".to_string()],
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
    let cookie_token = create_test_local_login_session(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/webapp/console/tasks/{task_id}/timeline"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["detail"]["task"]["title"], secret_task_title);
    assert_eq!(json["detail"]["task"]["body"], secret_task_body);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let session = crate::web_console_auth::validate_session(
        &conn,
        &cookie_token,
        chrono::Utc::now().timestamp() as u64,
        30 * 24 * 60 * 60,
    )
    .unwrap()
    .expect("local session");
    assert_eq!(
        session.mfa_level,
        crate::web_console_auth::MfaLevel::GoogleOnly
    );
    assert!(session.mfa_verified_at.is_none());
    assert_eq!(session.google_sub, crate::web_console_auth::LOCAL_LOGIN_SUB);
}

#[tokio::test]
async fn webapp_task_timeline_returns_raw_detail_only_after_gate_allows() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_task_title = "private gated task timeline title";
    let secret_task_body = "private gated task body 24680";
    let task_id = state
        .memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            secret_task_title,
            secret_task_body,
            "high",
            &["private".to_string(), "timeline".to_string()],
            Some("2026-05-04"),
            "user",
            Some(1_777_777_777),
            Some("time"),
            Some("test-event-key"),
            "notify",
            Some("private context text 13579"),
            None,
            Some("group-1"),
            Some(3),
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
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/webapp/console/tasks/{task_id}/timeline"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["access"], "allowed");
    assert_eq!(json["detail"]["status"], "available");
    assert_eq!(json["detail"]["task"]["title"], secret_task_title);
    assert_eq!(json["detail"]["task"]["body"], secret_task_body);
    assert_eq!(
        json["detail"]["task"]["context_text"],
        "private context text 13579"
    );
    assert!(
        json["detail"]["timeline"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "scheduled")),
        "timeline should include scheduled trigger evidence"
    );

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::Allowed
    );
    assert_eq!(audit.resource_type, "task_timeline");
    assert_eq!(audit.purpose, "web_console_task_timeline");
    assert!(audit.grant_id.is_some());
}

#[tokio::test]
async fn webapp_task_action_requires_csrf_and_records_receipt() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let task_id = state
        .memory
        .store_task(
            12345678,
            "did:aegis:telegram:12345678",
            "private task title",
            "private task body",
            "normal",
            &[],
            Some("2026-05-04"),
            "user",
            Some(1_777_777_777),
            Some("time"),
            Some("test-task-action"),
            "notify",
            Some("private context"),
            None,
            Some("task-action-group"),
            Some(3),
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
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state.clone());

    let no_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/tasks/{task_id}/action"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"schema_version":1,"action":"complete"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webapp/console/tasks/{task_id}/action"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header("X-Aegis-CSRF", csrf_token)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"schema_version":1,"action":"snooze","preset":"1h"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["resource_type"], "task");
    assert_eq!(json["action"], "snooze");
    assert_eq!(json["updated"], true);
    assert!(json["ledger_entry_id"].as_i64().unwrap() > 0);
    assert!(
        json["receipt"]
            .as_str()
            .unwrap()
            .starts_with("task_snooze:")
    );
    assert!(
        !body_text.contains("private task title") && !body_text.contains("private task body"),
        "task action receipt must not expose task title/body"
    );

    let updated = state.memory.get_task(task_id).await.unwrap();
    assert_eq!(updated.status, "active");
    assert!(updated.trigger_at_unix.unwrap() > 1_777_777_777);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let entry = crate::action_ledger::ActionLedgerStore::new(&conn)
        .list_recent(1)
        .unwrap()
        .pop()
        .expect("ledger entry");
    assert_eq!(entry.chat_id, Some(12345678));
    assert_eq!(entry.status, crate::action_ledger::ActionStatus::Executed);
    assert_eq!(
        entry.action_kind,
        crate::action_ledger::ActionKind::ReminderUpdate
    );
    assert!(entry.external_receipt.unwrap().starts_with("task_snooze:"));
}

#[tokio::test]
async fn webapp_action_receipt_returns_raw_detail_only_after_gate_allows() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let memory = MemoryHandle::spawn(db.path()).unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, Some(db_path.clone()));
    let secret_summary = "private receipt summary 86420";
    let secret_receipt = "calendar:sent:private-receipt-86420";
    let entry_id = {
        let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: "receipt-detail-action".into(),
                action_kind: crate::action_ledger::ActionKind::EmailSend,
                requested_by: Some("did:aegis:telegram:12345678".into()),
                chat_id: Some(12345678),
                input_summary: Some(secret_summary.into()),
                user_visible_summary: Some(secret_summary.into()),
                provider_request_id: Some("provider-request-private".into()),
                trace_id: Some("trace-private".into()),
                ..Default::default()
            })
            .unwrap();
        store
            .transition(
                "receipt-detail-action",
                crate::action_ledger::ActionStatus::Executed,
                &crate::action_ledger::ActionUpdate {
                    external_receipt: Some(secret_receipt.into()),
                    external_message_id: Some("calendar-message-private".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .id
    };
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/webapp/console/ledger/{entry_id}"))
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["access"], "allowed");
    assert_eq!(json["detail"]["status"], "available");
    assert_eq!(json["detail"]["receipt_status"], "Present");
    assert_eq!(json["detail"]["entry"]["input_summary"], secret_summary);
    assert_eq!(json["detail"]["entry"]["external_receipt"], secret_receipt);
    assert_eq!(
        json["detail"]["entry"]["provider_request_id"],
        "provider-request-private"
    );

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let audit =
        crate::data_access_gate::latest_sensitive_read_audit(&conn, "did:aegis:telegram:12345678")
            .unwrap()
            .expect("sensitive read audit");
    assert_eq!(
        audit.status,
        crate::data_access_gate::SensitiveReadAuditStatus::Allowed
    );
    assert_eq!(audit.resource_type, "action_ledger_entry");
    assert_eq!(audit.purpose, "web_console_action_receipt_detail");
    assert!(audit.grant_id.is_some());
}
