//! Web transport hardening: stream tokens, rate limiter, cache/CSP headers, web push keys and notice policy, debug lease windows, bot-token auth, heartbeat audit, DID validation.
use super::*;

#[tokio::test]
async fn remote_access_routes_serve_assets_and_reject_unconfigured_authentication() {
    let app = oauth_callback_router(webapp_test_state());
    for (path, content_type, marker) in [
        ("/access", "text/html", "Create Passkey"),
        ("/access.js", "text/javascript", "navigator.credentials"),
        ("/access.css", "text/css", "--b-accent"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(content_type)
        );
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains(marker));
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/passkey/login-start")
                .header("content-type", "application/json")
                .header("origin", "https://unconfigured.invalid")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Expired stream tokens must be rejected before any routing happens. The
/// check used to be covered through the retired SDUI `/webapp/stream` route;
/// the per-turn progress stream shares `validate_webapp_stream_token`.
#[tokio::test]
async fn webapp_vnext_turn_events_rejects_expired_stream_token() {
    let state = webapp_test_state();
    let turn_id = "turn-expired-token".to_string();
    let (stream_token, _) = mint_webapp_stream_token(turn_id.clone(), 12345678);
    {
        let mut store = webapp_stream_token_store().lock();
        store.get_mut(&stream_token).unwrap().expires_at =
            webapp_stream_now_secs().saturating_sub(1);
    }
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/webapp/vnext/turns/{turn_id}/events?stream_token={stream_token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn webapp_rate_limiter_blocks_until_window_expires() {
    let limiter = WebAppRateLimiter::default();
    let now = Instant::now();
    assert_eq!(
        limiter.check("principal:schema:task", 2, Duration::from_secs(60), now),
        None
    );
    assert_eq!(
        limiter.check(
            "principal:schema:task",
            2,
            Duration::from_secs(60),
            now + Duration::from_secs(1)
        ),
        None
    );
    assert!(matches!(
        limiter.check(
            "principal:schema:task",
            2,
            Duration::from_secs(60),
            now + Duration::from_secs(2)
        ),
        Some(WebAppRateLimitExceeded { retry_after_secs }) if retry_after_secs > 0
    ));
    assert_eq!(
        limiter.check(
            "principal:schema:task",
            2,
            Duration::from_secs(60),
            now + Duration::from_secs(61)
        ),
        None
    );
}

#[test]
fn webapp_assets_and_shell_are_not_stored_by_shared_caches() {
    assert_eq!(
        webapp_cache_control_for("/ui/assets/index-good.js", StatusCode::OK),
        WEBAPP_NO_STORE
    );
    assert_eq!(
        webapp_cache_control_for("/ui/assets/index-missing.js", StatusCode::NOT_FOUND),
        WEBAPP_NO_STORE
    );
    assert_eq!(
        webapp_cache_control_for(
            "/ui/assets/index-failed.js",
            StatusCode::INTERNAL_SERVER_ERROR
        ),
        WEBAPP_NO_STORE
    );
    assert_eq!(
        webapp_cache_control_for("/ui/", StatusCode::OK),
        WEBAPP_NO_STORE
    );
    assert_eq!(
        webapp_cache_control_for("/ui/service-worker.js", StatusCode::OK),
        WEBAPP_NO_STORE
    );
}

#[test]
fn csp_connect_src_follows_the_configured_voice_socket_origin() {
    // The browser dials the Realtime voice socket directly, so whichever host is
    // configured has to appear in connect-src — WebKit rejects anything else
    // with `SecurityError: The operation is insecure.`
    let csp = webapp_csp_with_voice_origin("wss://api.x.ai/v1/realtime");
    assert!(csp.contains("connect-src 'self' wss://api.x.ai;"), "{csp}");

    let custom = webapp_csp_with_voice_origin("ws://127.0.0.1:8787/v1/realtime?model=x");
    assert!(
        custom.contains("connect-src 'self' ws://127.0.0.1:8787;"),
        "{custom}"
    );

    // Unusable configuration must not silently widen the policy — it falls back
    // to the shipped default origin rather than to a wildcard.
    let broken = webapp_csp_with_voice_origin("not a url");
    assert!(
        broken.contains("connect-src 'self' wss://api.x.ai;"),
        "{broken}"
    );

    assert_eq!(
        ws_origin("wss://api.x.ai/v1/realtime?model=grok-voice-latest").as_deref(),
        Some("wss://api.x.ai")
    );
    assert_eq!(ws_origin("https://api.x.ai/v1/realtime"), None);
    assert_eq!(ws_origin("wss://"), None);
}

#[test]
fn web_push_public_key_requires_valid_vapid_shape() {
    use base64::Engine as _;

    let valid = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![7_u8; 65]);
    let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![7_u8; 64]);

    assert!(validate_web_push_public_key(&valid));
    assert!(!validate_web_push_public_key(""));
    assert!(!validate_web_push_public_key(&short));
    assert!(!validate_web_push_public_key("not base64"));
}

#[test]
fn web_push_private_key_accepts_raw_or_pem_only() {
    use base64::Engine as _;

    let valid_raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![7_u8; 32]);
    let short_raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![7_u8; 31]);
    let pem = "-----BEGIN EC PRIVATE KEY-----\nabc\n-----END EC PRIVATE KEY-----";

    assert!(matches!(
        parse_web_push_vapid_private_key(&valid_raw),
        Some(WebPushVapidPrivateKey::RawBase64(_))
    ));
    assert!(matches!(
        parse_web_push_vapid_private_key(pem),
        Some(WebPushVapidPrivateKey::Pem(_))
    ));
    assert!(parse_web_push_vapid_private_key(&short_raw).is_none());
    assert!(parse_web_push_vapid_private_key("not base64").is_none());
}

#[test]
fn web_push_subscription_validation_rejects_unsafe_endpoints() {
    assert!(validate_web_push_endpoint("https://updates.push.apple.com/3/device/example").is_ok());
    assert_eq!(
        validate_web_push_endpoint("http://updates.push.apple.com/3/device/example")
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        validate_web_push_endpoint("https://user:pass@updates.push.apple.com/3/device/example")
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        validate_web_push_endpoint("javascript:alert(1)")
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn web_push_notice_allows_only_local_ui_targets() {
    assert!(WebPushNotice::new("Aegis", "Body", "tag", "/ui/").is_ok());
    assert!(WebPushNotice::new("Aegis", "Body", "tag", "/ui/?view=work").is_ok());
    assert!(WebPushNotice::new("Aegis", "Body", "tag", "https://evil.test/ui/").is_err());
    assert!(WebPushNotice::new("Aegis", "Body", "tag", "/admin").is_err());
}

#[test]
fn web_push_notice_policy_keeps_routine_status_out_of_phone_push() {
    assert!(web_push_notice_from_owner_timeline_text("系统已重启 / System restarted").is_none());
    assert!(
        web_push_notice_from_owner_timeline_text("System Integrity Report\n\nInvariants: 3/3 ✅")
            .is_none()
    );
    assert!(
        web_push_notice_from_owner_timeline_text("OpenClaw 近 12 小时有更新：release notes")
            .is_none()
    );
}

#[test]
fn web_push_notice_policy_allows_phone_worthy_owner_events() {
    let reminder = web_push_notice_from_owner_timeline_text(
        "⏰ 发表师父新文章\n\nWeb actions: `done reminder 1893`.",
    )
    .expect("reminder should push");
    assert_eq!(reminder.title, "Reminder");
    assert!(!reminder.body.contains("Web actions:"));

    let approval = web_push_notice_from_owner_timeline_text(
        "⏳ calendar_update_event · 等待批准中 / awaiting approval",
    )
    .expect("approval should push");
    assert_eq!(approval.title, "Aegis needs your decision");

    let mail = web_push_notice_from_owner_timeline_text(
        "📬 [primary] Email from noreply@keycdn.com: [KeyCDN] Successful login",
    )
    .expect("mail notice should push");
    assert_eq!(mail.title, "New mail");
}

#[test]
fn web_push_notice_policy_treats_localized_server_alerts_as_important() {
    let notice = web_push_notice_from_owner_timeline_text(
        "🚨 服务器：磁盘快满了（还没解决）\n\n磁盘用了 93%，只剩 13.6 GB（共 192.7 GB），超过 70% 的线。",
    )
    .expect("active server alert should push");
    assert_eq!(notice.title, "服务器：磁盘快满了（还没解决）");
    assert!(notice.body.contains("磁盘用了 93%"));
    assert!(
        notice.tag.starts_with("aegis-important-"),
        "active server alert tag should be important, got {}",
        notice.tag
    );

    let legacy = web_push_notice_from_owner_timeline_text(
        "🚨 [Aegis Runtime Alert] server_monitor\n\nDisk: 93.0% used (threshold: 70%)",
    )
    .expect("legacy alert wrapper should still push");
    assert!(legacy.tag.starts_with("aegis-important-"));
}

#[test]
fn web_push_notice_policy_keeps_resolved_server_alerts_quiet() {
    // 2026-09-13 (batch E10): a recovery is pushed like its alert and shares
    // the alert's tag, so it REPLACES the 🚨 on the phone instead of falling
    // back to Telegram while the 🚨 went to the console. "Quiet" now means
    // "one notification slot per subject", not "never pushed".
    let active = web_push_notice_from_owner_timeline_text(
        "🚨 服务器：磁盘快满了\n\n磁盘用了 72%，只剩 53.9 GB（共 192.7 GB），超过 70% 的线。",
    )
    .expect("active alert is pushed");
    let resolved = web_push_notice_from_owner_timeline_text(
        "✅ 服务器：已恢复正常\n\nCPU 负载 1.63 / 1.52 / 1.57（4 核，上限 12），内存用了 18%（剩 19.7 GB）。",
    )
    .expect("resolved alert replaces the active one");
    assert_eq!(active.tag, resolved.tag, "same subject → same push slot");
    let later = web_push_notice_from_owner_timeline_text(
        "🚨 服务器：磁盘快满了\n\n磁盘用了 70%，只剩 57.7 GB（共 192.7 GB），超过 70% 的线。",
    )
    .expect("re-alert is pushed");
    assert_eq!(
        active.tag, later.tag,
        "a different percentage must not open a new slot"
    );
    let legacy = web_push_notice_from_owner_timeline_text(
        "✅ [Aegis Runtime Alert] server_monitor\n\n[RESOLVED] disk usage recovered",
    );
    assert!(
        legacy.is_some(),
        "legacy resolved alerts ride the same channel"
    );
    let reminder =
        web_push_notice_from_owner_timeline_text("⏰ 提醒：打电话给爸妈").expect("reminder");
    assert_ne!(reminder.tag, active.tag);
}

#[test]
fn web_push_subscription_request_preserves_safe_fields_only() {
    use base64::Engine as _;

    let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![9_u8; 65]);
    let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![3_u8; 16]);
    let record = web_push_subscription_record_from_request(VNextWebPushSubscribeRequest {
        schema_version: Some(1),
        subscription: WebPushBrowserSubscription {
            endpoint: " https://fcm.googleapis.com/fcm/send/device ".into(),
            keys: WebPushSubscriptionKeys {
                p256dh: p256dh.clone(),
                auth: auth.clone(),
            },
        },
        user_agent: Some(format!("{}{}", "Mobile Safari ", "x".repeat(400))),
    })
    .expect("valid subscription should parse");

    assert_eq!(
        record.endpoint,
        "https://fcm.googleapis.com/fcm/send/device"
    );
    assert_eq!(record.keys.p256dh, p256dh);
    assert_eq!(record.keys.auth, auth);
    assert!(record.user_agent.unwrap().len() <= 240);
}

#[test]
fn debug_lease_duration_accepts_owner_selected_windows_up_to_24_hours() {
    assert_eq!(normalize_debug_lease_duration_minutes(None), 8 * 60);
    assert_eq!(normalize_debug_lease_duration_minutes(Some(1)), 5);
    assert_eq!(normalize_debug_lease_duration_minutes(Some(60)), 60);
    assert_eq!(normalize_debug_lease_duration_minutes(Some(8 * 60)), 8 * 60);
    assert_eq!(normalize_debug_lease_duration_minutes(Some(4 * 60)), 4 * 60);
    assert_eq!(
        normalize_debug_lease_duration_minutes(Some(30 * 60)),
        24 * 60
    );
}

#[tokio::test]
async fn webapp_rejects_non_admin_user() {
    let state = webapp_test_state();
    // User ID 99999 != admin_chat_id 12345678
    let init_data = make_test_init_data("123456:TEST-TOKEN", 99999);
    let app = oauth_callback_router(state);

    let req = Request::builder()
        .uri("/webapp/vnext/home")
        .header("Authorization", format!("TelegramWebApp {}", init_data))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webapp_rejects_wrong_bot_token() {
    let state = webapp_test_state();
    // Generate initData with wrong token
    let init_data = make_test_init_data("wrong:TOKEN", 12345678);
    let app = oauth_callback_router(state);

    let req = Request::builder()
        .uri("/webapp/vnext/home")
        .header("Authorization", format!("TelegramWebApp {}", init_data))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Heartbeat audit endpoint tests ─────────────────────────────────

#[tokio::test]
async fn heartbeat_audit_endpoint_requires_auth() {
    let app = admin_router(test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/heartbeat-audit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn heartbeat_audit_endpoint_returns_entries() {
    let state = test_state();

    // Store a test audit entry
    state
        .memory
        .store_heartbeat_audit(
            "test-cycle",
            "__heartbeat__",
            "Info",
            None,
            &[],
            &[],
            &["test action".to_string()],
            Some("test reply"),
            100,
            1,
            "did:aegis:test",
            Some(12345),
            "{}",
            Some("Round 1: test reasoning"),
            None, // tool_call_details
        )
        .await
        .unwrap();

    let app = admin_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/heartbeat-audit?limit=5")
                .header("Authorization", "Bearer test-secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["skill_name"], "__heartbeat__");
    assert_eq!(entries[0]["raw_process_log"], "Round 1: test reasoning");
}

#[tokio::test]
async fn heartbeat_dashboard_returns_html() {
    let state = test_state();
    let app = admin_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/heartbeat-dashboard")
                .header("Authorization", "Bearer test-secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        html.contains("Heartbeat Dashboard"),
        "should contain dashboard title"
    );
    assert!(
        html.contains("/heartbeat-audit"),
        "should reference the API endpoint"
    );
    assert!(
        html.contains("raw_process_log"),
        "should handle raw process log field"
    );
}

// ── DID validation tests ─────────────────────────────────────────

#[test]
fn validate_did_accepts_valid() {
    assert!(validate_did("did:aegis:telegram:12345").is_ok());
    assert!(validate_did("did:aegis:test:abc").is_ok());
}

#[test]
fn validate_did_rejects_invalid_prefix() {
    let result = validate_did("not-a-did");
    assert!(result.is_err());
    let (status, _) = result.unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[test]
fn validate_did_rejects_too_long() {
    let long_did = format!("did:aegis:{}", "x".repeat(300));
    let result = validate_did(&long_did);
    assert!(result.is_err());
    let (status, _) = result.unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[test]
fn validate_did_rejects_empty() {
    assert!(validate_did("").is_err());
}
