//! Admin API surface: health, bearer auth, Google revoke, reminders, L2 facts, settings, LLM fallback drill, webapp auth denial and security headers.
use super::*;

#[test]
fn web_console_login_can_reuse_shared_google_oauth_client_from_store() {
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let db_path = tmp.path().to_str().unwrap().to_string();
    let aes_key = [44u8; 32];
    {
        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                .unwrap();
        store_google_credentials(
            &store,
            "google_oauth",
            "shared-client.apps.googleusercontent.com",
        )
        .unwrap();
    }

    let shared = load_shared_google_oauth_client_from_store(&db_path, aes_key)
        .unwrap()
        .expect("shared Google OAuth client");
    let mut config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
        "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let mut client_secret = None;

    apply_shared_google_oauth_client_to_login_config(&mut config, &shared, &mut client_secret);

    assert_eq!(config.client_id, "shared-client.apps.googleusercontent.com");
    assert!(config.client_secret_present);
    assert_eq!(client_secret.as_deref(), Some("client-secret"));
    assert!(config.validate_for_login().is_ok());
}

#[test]
fn web_console_login_does_not_pair_mismatched_client_id_with_shared_secret() {
    let shared = SharedGoogleOAuthClient {
        client_id: "shared-client.apps.googleusercontent.com".into(),
        client_secret: "client-secret".into(),
    };
    let mut config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
        "GOOGLE_LOGIN_CLIENT_ID" => Some("other-client.apps.googleusercontent.com".into()),
        "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let mut client_secret = None;

    apply_shared_google_oauth_client_to_login_config(&mut config, &shared, &mut client_secret);

    assert_eq!(config.client_id, "other-client.apps.googleusercontent.com");
    assert!(!config.client_secret_present);
    assert!(client_secret.is_none());
    assert!(config.validate_for_login().is_err());
}

#[test]
fn oauth_success_notification_uses_router_admin_fallback() {
    let callback_chat_id = 222_333_444;
    let admin_chat_id = 123_456_789;

    let (target_chat_id, thread) =
        oauth_success_notification_destination(callback_chat_id, Some(admin_chat_id));

    assert_eq!(target_chat_id, teloxide::types::ChatId(admin_chat_id));
    assert_eq!(thread, None);
}

#[test]
fn webapp_google_oauth_callback_pages_point_back_to_aegis_webapp() {
    let success = google_oauth_callback_success_html();
    let failure = google_oauth_callback_error_html("access_denied");

    assert!(success.contains("Google Calendar and Drive have been connected to Aegis."));
    assert!(success.contains("continue in the WebApp"));
    assert!(failure.contains("continue in Aegis"));
    assert!(!success.contains("return to Telegram"));
    assert!(!failure.contains("return to Telegram"));
    assert!(!success.contains("Aegis Runtime"));
}

#[test]
fn oauth_success_notification_preserves_callback_fallback_without_admin_config() {
    let callback_chat_id = 222_333_444;

    let (target_chat_id, thread) = oauth_success_notification_destination(callback_chat_id, None);

    assert_eq!(target_chat_id, teloxide::types::ChatId(callback_chat_id));
    assert_eq!(thread, None);
}

/// GET /health and return `(http status, parsed body)`.
async fn fetch_health() -> (StatusCode, serde_json::Value) {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    // The Telegram snapshot is process-global; the guard serialises the
    // health tests and resets it to Unconfigured before and after.
    let _telegram = crate::telegram_health::test_guard();

    let (status, json) = fetch_health().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");

    // Incident 2026-09-15: /health knew nothing about the bot, so a revoked
    // token left it at "ok" for six hours. The component is always present.
    let telegram = &json["components"]["telegram"];
    assert!(
        telegram.is_object(),
        "components.telegram missing from /health: {json}"
    );
    assert_eq!(telegram["state"], "unconfigured");
    assert_eq!(telegram["consecutive_failures"], 0);
    assert_eq!(telegram["bot_username"], serde_json::Value::Null);
    assert_eq!(telegram["last_error"], serde_json::Value::Null);
    assert!(
        telegram["since_unix"].as_i64().is_some(),
        "since_unix must be a unix timestamp: {telegram}"
    );
}

#[tokio::test]
async fn health_reports_degraded_when_telegram_rejects_the_token() {
    use crate::telegram_health::{TelegramState, record_state};
    let _telegram = crate::telegram_health::test_guard();

    record_state(
        TelegramState::InvalidToken,
        Some("A Telegram's error: Invalid bot token".to_string()),
    );

    let (status, json) = fetch_health().await;
    // Still HTTP 200: deploy.sh / smoke_test.sh call this with `curl -f`, and
    // a dead bot must not make the whole service look down.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "degraded");

    let telegram = &json["components"]["telegram"];
    assert_eq!(telegram["state"], "invalid_token");
    assert_eq!(telegram["consecutive_failures"], 1);
    assert_eq!(
        telegram["last_error"],
        "A Telegram's error: Invalid bot token"
    );
    // The other components are untouched by the bot's state.
    assert_eq!(json["components"]["memory_actor"], "ok");
}

#[tokio::test]
async fn health_reports_unreachable_telegram_as_degraded_only_after_ten_minutes() {
    use crate::telegram_health::{TelegramState, now_unix, record_state_at};
    let _telegram = crate::telegram_health::test_guard();

    // Fresh outage (30 s): a blip, status stays ok but the state is visible.
    record_state_at(
        TelegramState::Unreachable,
        Some("A network error: dns".to_string()),
        now_unix() - 30,
    );
    let (status, json) = fetch_health().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
    assert_eq!(json["components"]["telegram"]["state"], "unreachable");

    // Same outage, now 11 minutes old: degraded, still HTTP 200.
    crate::telegram_health::set_since_unix_for_tests(now_unix() - 660);
    let (status, json) = fetch_health().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["components"]["telegram"]["state"], "unreachable");

    // Recovery flips it back without a restart.
    crate::telegram_health::record_reachable("aegis_bot");
    let (_, json) = fetch_health().await;
    assert_eq!(json["status"], "ok");
    assert_eq!(json["components"]["telegram"]["state"], "reachable");
    assert_eq!(json["components"]["telegram"]["bot_username"], "aegis_bot");
}

#[tokio::test]
async fn health_endpoint_needs_no_auth() {
    // Health should work even without any auth header
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn system_metrics_is_public() {
    // /system_metrics is public (localhost-only, needed by server_monitor WASM skill)
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/system_metrics")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/system_metrics must be public for server_monitor"
    );

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("cpu_load_1m").is_some());
    assert!(json.get("memory_total_kb").is_some());
    assert!(json.get("disk_total_bytes").is_some());
    assert!(json.get("num_cpus").is_some());
    assert!(json["num_cpus"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn system_metrics_returns_valid_json() {
    // Regression test: server_monitor WASM skill must receive valid JSON,
    // not a 401/403 error page. If this test fails, server_monitor will
    // alert with "Failed to parse system_metrics".
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/system_metrics")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();

    // This is what server_monitor does — if this panics, the skill
    // would generate a "Failed to parse" alert.
    let json: serde_json::Value = serde_json::from_slice(&body)
        .expect("system_metrics must return valid JSON (server_monitor depends on this)");

    // Verify all fields that server_monitor deserializes
    assert!(json["cpu_load_1m"].is_f64(), "missing cpu_load_1m");
    assert!(json["cpu_load_5m"].is_f64(), "missing cpu_load_5m");
    assert!(json["cpu_load_15m"].is_f64(), "missing cpu_load_15m");
    assert!(json["memory_total_kb"].is_u64(), "missing memory_total_kb");
    assert!(
        json["memory_available_kb"].is_u64(),
        "missing memory_available_kb"
    );
    assert!(
        json["memory_used_percent"].is_f64(),
        "missing memory_used_percent"
    );
    assert!(
        json["disk_total_bytes"].is_u64(),
        "missing disk_total_bytes"
    );
    assert!(
        json["disk_available_bytes"].is_u64(),
        "missing disk_available_bytes"
    );
    assert!(
        json["disk_used_percent"].is_f64(),
        "missing disk_used_percent"
    );
    assert!(json["num_cpus"].is_u64(), "missing num_cpus");
}

#[tokio::test]
async fn skills_with_valid_token() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
    assert!(json["skills"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn protected_endpoint_rejects_missing_token() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/skills")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_endpoint_rejects_wrong_token() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_endpoint_rejects_no_bearer_prefix() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Basic dXNlcjpwYXNz")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_token_configured_returns_403() {
    let app = admin_router(test_state_with_token(None));
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Bearer anything")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "unconfigured token must return 403, not 200"
    );
}

// ── constant_time_eq security tests ─────────────────────────────────

#[test]
fn constant_time_eq_equal_values() {
    assert!(super::constant_time_eq("secret-token", "secret-token"));
}

#[test]
fn constant_time_eq_different_values() {
    assert!(!super::constant_time_eq("secret-token", "wrong-token!"));
}

#[test]
fn constant_time_eq_different_lengths() {
    assert!(!super::constant_time_eq("short", "longer-value"));
}

#[test]
fn constant_time_eq_empty() {
    assert!(super::constant_time_eq("", ""));
}

#[test]
fn constant_time_eq_single_bit_difference() {
    let a = "secret-token-value-1234567890";
    let mut b = a.as_bytes().to_vec();
    // Flip one bit in the last byte — must still be rejected
    let last = b.len() - 1;
    b[last] ^= 1;
    let b_str = String::from_utf8(b).unwrap();
    assert!(
        !super::constant_time_eq(a, &b_str),
        "single-bit difference must be detected"
    );
}

#[test]
fn constant_time_eq_first_byte_differs() {
    assert!(!super::constant_time_eq("Xsecret-token", "Ysecret-token"));
}

#[tokio::test]
async fn auth_with_constant_time_comparison() {
    // Verify the middleware correctly accepts/rejects tokens
    // (integration test that the constant_time_eq path works end-to-end)
    let app = admin_router(test_state());

    // Valid token
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Off-by-one-char token
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/skills")
        .header("Authorization", "Bearer test-secret-tokeN")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "off-by-one-char token must be rejected"
    );
}
#[test]
fn resolve_live_google_auth_manager_uses_legacy_mirror_only_when_registry_empty() {
    with_test_runtime(|| {
        let state = test_state_with_registry();
        let legacy_manager = test_google_manager("legacy-refresh");
        *state.google_auth.write() = Some(legacy_manager.clone());

        let resolved = super::resolve_live_google_auth_manager(&state).unwrap();
        assert!(Arc::ptr_eq(&resolved, &legacy_manager));
    });
}
#[tokio::test]
async fn google_revoke_named_account_deletes_only_named_credentials() {
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let db_path = tmp.path().to_str().unwrap().to_string();
    let aes_key = [42u8; 32];
    {
        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                .unwrap();
        store_google_credentials(&store, "google_oauth", "primary-cid").unwrap();
        store_google_credentials(&store, "google_oauth:work", "work-cid").unwrap();
    }

    let app = admin_router(test_state_with_credential_store(db_path.clone(), aes_key));
    let req = Request::builder()
        .method("POST")
        .uri("/auth/google/revoke")
        .header("Authorization", "Bearer test-secret-token")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"account_name":"work"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["account_name"], "work");

    let store =
        CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key).unwrap();
    assert_eq!(
        store.get("google_oauth", "client_id").unwrap().as_deref(),
        Some("primary-cid")
    );
    assert!(
        store
            .get("google_oauth:work", "client_id")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn google_revoke_unknown_named_account_fails_closed() {
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let db_path = tmp.path().to_str().unwrap().to_string();
    let aes_key = [43u8; 32];
    {
        let store =
            CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key)
                .unwrap();
        store_google_credentials(&store, "google_oauth", "primary-cid").unwrap();
    }

    let app = admin_router(test_state_with_credential_store(db_path.clone(), aes_key));
    let req = Request::builder()
        .method("POST")
        .uri("/auth/google/revoke")
        .header("Authorization", "Bearer test-secret-token")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"account_name":"missing"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let store =
        CredentialStore::open(crate::encrypted_sqlite::open(&db_path).unwrap(), aes_key).unwrap();
    assert_eq!(
        store.get("google_oauth", "client_id").unwrap().as_deref(),
        Some("primary-cid")
    );
}

// ── Admin API extension tests ───────────────────────────────────────

#[tokio::test]
async fn reminders_endpoint_returns_empty_list() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/reminders")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
    assert!(json["reminders"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn reminders_endpoint_requires_auth() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/reminders")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reminders_endpoint_shows_pending() {
    let state = test_state();
    // Store a scheduled task (reminder) in the unified tasks table
    let future_ts = chrono::Utc::now().timestamp() + 3600;
    state
        .memory
        .store_task(
            12345,
            "did:aegis:telegram:12345",
            "test reminder",
            "test reminder",
            "normal",
            &[],
            None,
            "user",
            Some(future_ts),
            Some("time"),
            None,
            "notify",
            Some("test reminder"),
            None,
            None,
            None,
            false,
            Some(future_ts + 3600),
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let app = admin_router(state);
    let req = Request::builder()
        .uri("/reminders")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["reminders"][0]["message"], "test reminder");
    assert_eq!(json["reminders"][0]["chat_id"], 12345);
}

#[tokio::test]
async fn l2_endpoint_returns_facts_for_did() {
    let state = test_state();
    state
        .memory
        .store_l2("did:aegis:test:abc", "topic1", "fact one", None)
        .await
        .unwrap();
    state
        .memory
        .store_l2("did:aegis:test:abc", "topic2", "fact two", None)
        .await
        .unwrap();

    let app = admin_router(state);
    let req = Request::builder()
        .uri("/l2/did:aegis:test:abc")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 2);
    assert_eq!(json["did"], "did:aegis:test:abc");
    // L2 returns ORDER BY created_at DESC, so most recent first
    assert_eq!(json["facts"][0]["topic"], "topic2");
    assert_eq!(json["facts"][1]["topic"], "topic1");
}

#[tokio::test]
async fn l2_endpoint_invalid_did_returns_400() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/l2/did:nonexistent:xyz")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "invalid DID prefix should be rejected"
    );
}

#[tokio::test]
async fn l2_endpoint_valid_did_no_facts_returns_empty() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/l2/did:aegis:test:nonexistent")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
    assert!(json["facts"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn l2_endpoint_requires_auth() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .uri("/l2/did:aegis:test:abc")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_l2_removes_fact() {
    let state = test_state();
    let id = state
        .memory
        .store_l2("did:aegis:test:del", "topic", "to be deleted", None)
        .await
        .unwrap();

    let app = admin_router(state.clone());
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/l2/did:aegis:test:del/{id}"))
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], true);

    // Verify it's actually gone
    let facts = state.memory.get_all_l2("did:aegis:test:del").await.unwrap();
    assert!(facts.is_empty());
}

#[tokio::test]
async fn delete_l2_nonexistent_returns_false() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .method("DELETE")
        .uri("/l2/did:aegis:test:nope/99999")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], false);
}

#[tokio::test]
async fn delete_l2_requires_auth() {
    let app = admin_router(test_state());
    let req = Request::builder()
        .method("DELETE")
        .uri("/l2/did:aegis:test:abc/1")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_l2_wrong_did_does_not_delete() {
    let state = test_state();
    let id = state
        .memory
        .store_l2("did:aegis:test:owner", "topic", "my fact", None)
        .await
        .unwrap();

    // Try to delete with a different DID
    let app = admin_router(state.clone());
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/l2/did:aegis:test:other/{id}"))
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], false, "wrong DID must not delete");

    // Verify original fact still exists
    let facts = state
        .memory
        .get_all_l2("did:aegis:test:owner")
        .await
        .unwrap();
    assert_eq!(facts.len(), 1);
}

// ── Settings endpoint tests ─────────────────────────────────────────

#[tokio::test]
async fn settings_list_returns_all() {
    let state = test_state();
    let app = admin_router(state);
    let req = Request::builder()
        .uri("/settings")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 64 KiB read cap: the settings payload grows as new SettingDefs are added
    // (each carries a bilingual description); the old 16 KiB cap was crossed once
    // the task-archive-lifecycle settings landed. This is a test-only buffer, not
    // a production limit.
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let settings = json["settings"].as_array().unwrap();
    assert!(settings.len() >= 2, "should list at least system settings");
}

#[tokio::test]
async fn settings_update_valid_key() {
    let state = test_state();
    let app = admin_router(state.clone());
    let req = Request::builder()
        .method("PUT")
        .uri("/settings/mcp_enabled")
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"value":false}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["key"], "mcp_enabled");
    assert_eq!(json["value"], false);

    // Verify the setting was actually updated in-memory
    assert!(!state.settings.is_enabled("mcp_enabled"));
}

#[tokio::test]
async fn settings_update_unknown_key_rejected() {
    let state = test_state();
    let app = admin_router(state);
    let req = Request::builder()
        .method("PUT")
        .uri("/settings/nonexistent_key")
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"value":true}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_requires_auth() {
    let state = test_state();
    let app = admin_router(state);
    let req = Request::builder()
        .uri("/settings")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn llm_fallback_drill_requires_auth() {
    let state = webapp_test_state_with_orchestrator(MemoryHandle::spawn_in_memory().unwrap(), None);
    let app = admin_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/diagnostics/llm_fallback_drill")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"fault":"primary_timeout"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn llm_fallback_drill_returns_503_when_backup_is_not_authorized() {
    let state = webapp_test_state_with_orchestrator(MemoryHandle::spawn_in_memory().unwrap(), None);
    let app = admin_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/diagnostics/llm_fallback_drill")
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"fault":"primary_timeout"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(resp.into_body(), 16_384)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["fault"], "primary_timeout");
    assert_eq!(json["fallback_authorized"], false);
    let error = json["error"].as_str().unwrap_or("");
    assert!(
        error.contains("授权") || error.to_ascii_lowercase().contains("authoriz"),
        "503 body must mention authorization, got: {error}"
    );
}

#[tokio::test]
async fn llm_fallback_drill_returns_200_when_backup_answers() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let runtime = std::sync::Arc::new(WebappMockRuntimeState::new("OK"));
    let (addr, _handle) = spawn_webapp_mock_runtime(runtime.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_mock_runtime(memory, db_path, addr);
    state
        .settings
        .set(
            "fallback_model",
            serde_json::Value::String("backup-model".into()),
        )
        .unwrap();
    state
        .settings
        .set("fallback_model_authorized", serde_json::Value::Bool(true))
        .unwrap();
    state
        .settings
        .set("paid_api_fallback_enabled", serde_json::Value::Bool(true))
        .unwrap();
    state
        .settings
        .set("vision_routing_enabled", serde_json::Value::Bool(false))
        .unwrap();

    let app = admin_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/diagnostics/llm_fallback_drill")
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"fault":"primary_timeout"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 16_384)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["fault"], "primary_timeout");
    assert_eq!(json["answered_by"]["model"], "backup-model");
    assert!(json["error"].is_null());
}

#[tokio::test]
async fn settings_audit_requires_auth() {
    let state = test_state();
    let app = admin_router(state);
    let req = Request::builder()
        .uri("/diagnostics/settings_audit")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn settings_audit_returns_authorized_backup_without_model() {
    let state = test_state();
    state
        .settings
        .set("fallback_model_authorized", serde_json::json!(true))
        .unwrap();
    state
        .settings
        .set("fallback_model", serde_json::json!(""))
        .unwrap();
    state
        .settings
        .set("display_language", serde_json::json!("zh"))
        .unwrap();
    let app = admin_router(state);
    let req = Request::builder()
        .uri("/diagnostics/settings_audit")
        .header("Authorization", "Bearer test-secret-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 16_384)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let findings = json["findings"].as_array().expect("findings array");
    let codes: Vec<&str> = findings
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"fallback_authorized_without_model"),
        "codes={codes:?}"
    );
    let authorized = findings
        .iter()
        .find(|finding| finding["code"] == "fallback_authorized_without_model")
        .unwrap();
    assert_eq!(authorized["severity"], "attention");
    assert_eq!(
        authorized["message"],
        "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。"
    );
}

#[tokio::test]
async fn webapp_vnext_home_requires_auth() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let req = Request::builder()
        .uri("/webapp/vnext/home")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_api_routes_deny_unauthenticated_requests() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let cases = vec![
        (
            "vnext_home",
            Request::builder()
                .uri("/webapp/vnext/home")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "mfa_status",
            Request::builder()
                .uri("/mfa/status")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "webauthn_credentials",
            Request::builder()
                .uri("/mfa/webauthn/credentials")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_memory_item",
            Request::builder()
                .uri("/webapp/console/memory/items/1")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_profile_slot",
            Request::builder()
                .uri("/webapp/console/memory/profile/preference/communication_style")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_task_timeline",
            Request::builder()
                .uri("/webapp/console/tasks/1/timeline")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_ledger_entry",
            Request::builder()
                .uri("/webapp/console/ledger/1")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "owner_work_queue",
            Request::builder()
                .uri("/webapp/owner-work/queue")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_home",
            Request::builder()
                .uri("/webapp/vnext/home")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_work",
            Request::builder()
                .uri("/webapp/vnext/work")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_item",
            Request::builder()
                .uri("/webapp/vnext/work/item:1")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_command",
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/command")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "input": "Review today.",
                        "mode": "text",
                        "intent_kind": "explain",
                        "client_context": "home"
                    })
                    .to_string(),
                ))
                .unwrap(),
        ),
        (
            "vnext_command_status",
            Request::builder()
                .uri("/webapp/vnext/command/status?idempotency_key=poll")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_work_action",
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/work/item:1/actions")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "action_id": "mail.mark_read"
                    })
                    .to_string(),
                ))
                .unwrap(),
        ),
        (
            "vnext_status",
            Request::builder()
                .uri("/webapp/vnext/status")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_settings",
            Request::builder()
                .uri("/webapp/vnext/settings")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_debug_leases",
            Request::builder()
                .uri("/webapp/vnext/debug-leases")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "vnext_debug_lease",
            Request::builder()
                .method("DELETE")
                .uri("/webapp/vnext/debug-leases/lease-1")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "owner_work_context",
            Request::builder()
                .method("POST")
                .uri("/webapp/owner-work/context")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"focus_id":"assistant:general"}"#))
                .unwrap(),
        ),
        (
            "webauthn_register_start",
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/register/start")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "webauthn_register_finish",
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/register/finish")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "webauthn_assert_start",
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/assert/start")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "webauthn_assert_finish",
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/assert/finish")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "totp_setup_start",
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/start")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "totp_setup_finish",
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/finish")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "totp_verify",
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/verify")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "recovery_code_consume",
            Request::builder()
                .method("POST")
                .uri("/mfa/recovery-code/consume")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "recovery_codes_regenerate",
            Request::builder()
                .method("POST")
                .uri("/mfa/recovery-codes/regenerate")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "mfa_disable",
            Request::builder()
                .method("POST")
                .uri("/mfa/disable")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_create",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_edit",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates/production-smoke/edit")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_activate",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates/production-smoke/activate")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_pause",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates/production-smoke/pause")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_resume",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates/production-smoke/resume")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "console_mandate_revoke",
            Request::builder()
                .method("POST")
                .uri("/webapp/console/mandates/production-smoke/revoke")
                .body(Body::empty())
                .unwrap(),
        ),
    ];

    for (label, req) in cases {
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{label} should deny unauthenticated access"
        );
    }
}

#[tokio::test]
async fn webapp_api_responses_include_security_headers() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let req = Request::builder()
        .uri("/webapp/vnext/home")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let headers = resp.headers();
    assert!(
        headers
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.contains("default-src 'self'")
                    && value.contains("script-src 'self' https://telegram.org")
                    && value.contains("worker-src 'self'")
                    && value.contains("img-src 'self' https: data: blob:")
                    && value.contains("manifest-src 'self'")
                    && value.contains(
                        "frame-ancestors 'self' https://web.telegram.org https://*.telegram.org",
                    )
            })
    );
    assert!(
        headers.get(axum::http::header::X_FRAME_OPTIONS).is_none(),
        "Telegram Mini App framing relies on explicit CSP frame-ancestors, not X-Frame-Options"
    );
    // Regression (owner 2026-07-26: "点悬浮头像后我说话她没反应" + repeated
    // "The operation is insecure."): the CEO voice orb opens a Realtime
    // WebSocket from the browser to the provider. With `connect-src 'self'`
    // WebKit throws SecurityError before any audio flows, so the served policy
    // must name that socket origin.
    let csp = headers
        .get(axum::http::header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .expect("CSP header");
    let voice_origin = crate::admin::ws_origin(&crate::voice_session::voice_ws_url_from_env())
        .expect("voice ws origin");
    assert!(
        csp.contains(&format!("connect-src 'self' {voice_origin};")),
        "CSP must allow the Realtime voice socket origin {voice_origin}, got: {csp}"
    );
    assert_eq!(
        headers.get(axum::http::header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        headers
            .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    assert_eq!(
        headers.get(axum::http::header::REFERRER_POLICY).unwrap(),
        "no-referrer"
    );
}
#[tokio::test]
async fn sensitive_webapp_and_console_endpoints_are_no_store() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    for path in [
        "/api/session",
        "/mfa/status",
        "/webapp/vnext/home",
        "/webapp/vnext/work",
        "/webapp/vnext/work/item:1",
        "/webapp/vnext/command",
        "/webapp/vnext/work/item:1/actions",
        "/webapp/vnext/status",
        "/webapp/vnext/dev-orchestrator",
        "/webapp/vnext/settings",
        "/webapp/owner-work/queue",
        "/webapp/owner-work/context",
        "/webapp/console/mandates",
        "/webapp/console/mandates/production-smoke/activate",
        "/webapp/console/memory/items/1",
        "/webapp/console/tasks/1/timeline",
        "/webapp/console/ledger/1",
        "/webapp/console/mail/events/1/body",
    ] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store",
            "{path}"
        );
    }
}

/// Clears one feature flag for the guard's lifetime. Callers hold `WEBAPP_DEBUG_ENV_LOCK`.
struct ClearedEnvVar {
    key: &'static str,
    saved: Option<String>,
}

impl ClearedEnvVar {
    #[allow(unsafe_code)]
    fn clear(key: &'static str) -> Self {
        let saved = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, saved }
    }
}

impl Drop for ClearedEnvVar {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        match &self.saved {
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

async fn post_diagnostics(path: &str) -> (StatusCode, serde_json::Value) {
    let state = test_state();
    let app = admin_router(state);
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 16_384)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

#[tokio::test]
async fn diagnostics_secure_cu_stays_disabled_without_flag() {
    let _guard = WEBAPP_DEBUG_ENV_LOCK.lock().await;
    let _cleared = ClearedEnvVar::clear("AEGIS_SECURE_CU_ENABLED");
    let (status, json) = post_diagnostics("/diagnostics/secure_cu").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"], "disabled");
}

#[tokio::test]
async fn diagnostics_jev_non_cu_stays_disabled_without_flag() {
    let _guard = WEBAPP_DEBUG_ENV_LOCK.lock().await;
    let _cleared = ClearedEnvVar::clear("AEGIS_JEV_NON_CU_GATE_ENABLED");
    let (status, json) = post_diagnostics("/diagnostics/jev_non_cu").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"], "disabled");
}
