//! Web console auth: Google and local login, session shape, logout, MFA/TOTP/WebAuthn, dist resolver, CSRF on vNext mutations.
use super::*;

#[tokio::test]
async fn web_console_google_login_start_fails_closed_without_public_url() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/login/google")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn web_console_google_login_callback_rejects_missing_state() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/callback/google?code=test-code")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn web_console_legacy_google_login_callback_is_not_registered() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/callback/google-login?code=test-code")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shared_google_callback_dispatches_pending_web_console_login_state() {
    let login_config =
        crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
            "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
            "GOOGLE_CLIENT_ID" => Some("client.apps.googleusercontent.com".into()),
            "GOOGLE_CLIENT_SECRET" => Some("secret".into()),
            "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
            "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
            _ => None,
        });
    let login = crate::web_console_auth::build_google_login_url(
        &login_config,
        "https://fixture.example",
        None,
    )
    .expect("login url");
    let parsed = url::Url::parse(&login.url).unwrap();
    let state_param = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state param");

    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/callback/google?state={state_param}&error=access_denied"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    assert_eq!(&body[..], b"Google Login failed");
}

// ── README on-ramp: local login link + console bundle resolution ─────────────
//
// `/auth/login/local` is the only console principal an install without Google
// Login or Telegram has. Each assertion below fails on the matching defect:
// route registered without a token gate (404 case), token accepted without a
// compare (401 case), allowlist still applied to the local identity (303 +
// follow-up case), or the reject delay dropped (elapsed check).

#[test]
fn local_login_token_policy_generates_only_without_google_or_telegram() {
    use crate::web_console_auth::local_login_token_from_lookup;
    let generate = || Ok::<_, anyhow::Error>("generated-token".to_string());
    let never = || -> anyhow::Result<String> { panic!("generator must not run when disabled") };

    // unset + no google + no telegram → generated
    let token = local_login_token_from_lookup(|_| None, generate)
        .unwrap()
        .expect("fresh install gets a generated token");
    assert_eq!(token.token, "generated-token");
    assert!(token.generated);

    // unset + google enabled → None (and the generator is never consulted)
    assert!(
        local_login_token_from_lookup(
            |key| (key == "GOOGLE_LOGIN_ENABLED").then(|| "true".to_string()),
            never
        )
        .unwrap()
        .is_none()
    );
    // unset + telegram configured → None
    assert!(
        local_login_token_from_lookup(
            |key| (key == "TELOXIDE_TOKEN").then(|| "123456:TEST-TOKEN".to_string()),
            never
        )
        .unwrap()
        .is_none()
    );
    // an empty TELOXIDE_TOKEN is "unset"
    assert!(
        local_login_token_from_lookup(|key| (key == "TELOXIDE_TOKEN").then(String::new), generate)
            .unwrap()
            .is_some()
    );
    // "off" → None even on a fresh install
    assert!(
        local_login_token_from_lookup(
            |key| (key == "AEGIS_LOCAL_LOGIN_TOKEN").then(|| " OFF ".to_string()),
            never
        )
        .unwrap()
        .is_none()
    );
    // explicit → that token, not generated, even when Google Login is on
    let token = local_login_token_from_lookup(
        |key| match key {
            "AEGIS_LOCAL_LOGIN_TOKEN" => Some("fixed-token".to_string()),
            "GOOGLE_LOGIN_ENABLED" => Some("true".to_string()),
            _ => None,
        },
        never,
    )
    .unwrap()
    .expect("explicit token enables the link");
    assert_eq!(token.token, "fixed-token");
    assert!(!token.generated);
}

#[tokio::test]
async fn web_console_local_login_is_not_found_when_disabled() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_local_login(Some(db_path), None);
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/login/local?token=anything")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(resp.headers().get(axum::http::header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn web_console_local_login_rejects_wrong_or_missing_token_after_delay() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_local_login(Some(db_path), Some("correct-token"));
    for uri in [
        "/auth/login/local?token=wrong-token",
        "/auth/login/local?token=",
        "/auth/login/local",
    ] {
        let started = std::time::Instant::now();
        let resp = oauth_callback_router(state.clone())
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
        assert!(
            resp.headers().get(axum::http::header::SET_COOKIE).is_none(),
            "{uri} must not issue a session"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(300),
            "{uri}: reject must wait the fixed delay, took {elapsed:?}"
        );
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"Invalid local login token");
    }
}

#[tokio::test]
async fn web_console_local_login_issues_session_cookie_and_console_accepts_it() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let state = webapp_test_state_with_local_login(Some(db_path), Some(token));

    let resp = oauth_callback_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/auth/login/local?token={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/ui/")
    );
    let set_cookie = resp
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("local login sets the session cookie")
        .to_string();
    assert!(
        set_cookie.starts_with("__Host-aegis_session="),
        "{set_cookie}"
    );
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    let cookie_token =
        crate::web_console_auth::extract_cookie_value(&set_cookie, "__Host-aegis_session")
            .expect("cookie carries a token");

    // The cookie must be a real console session: /api/session authenticates
    // it and reports the local principal, so the console loads after the
    // redirect instead of bouncing back to the login screen.
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true, "{body}");
    assert_eq!(body["principal_type"], "web_console");
    assert_eq!(body["email"], crate::web_console_auth::LOCAL_LOGIN_EMAIL);
    assert_eq!(body["csrf_token"].as_str().unwrap().len(), 64);
}

#[test]
fn webapp_dist_resolver_prefers_cwd_then_falls_through_then_reports_miss() {
    let candidates = webapp_dist_candidates(
        None,
        Some(FsPath::new("/opt/aegis-runtime/current")),
        Some("/src/aegis-runtime/core"),
    );
    assert_eq!(
        candidates,
        vec![
            PathBuf::from("./webapp/dist"),
            PathBuf::from("/opt/aegis-runtime/current/../../webapp/dist"),
            PathBuf::from("/opt/aegis-runtime/current/webapp/dist"),
            PathBuf::from("/src/aegis-runtime/core/../webapp/dist"),
        ]
    );

    // cwd wins when it has index.html, even though later candidates also do
    let all = choose_webapp_dist_dir(&candidates, |_| true);
    assert!(all.found);
    assert_eq!(all.dir, PathBuf::from("./webapp/dist"));

    // falls through to the install layout next to the binary
    let install = choose_webapp_dist_dir(&candidates, |dir| {
        dir == FsPath::new("/opt/aegis-runtime/current/webapp/dist")
    });
    assert!(install.found);
    assert_eq!(
        install.dir,
        PathBuf::from("/opt/aegis-runtime/current/webapp/dist")
    );

    // nothing has index.html: first candidate, found=false, every path reported
    let miss = choose_webapp_dist_dir(&candidates, |_| false);
    assert!(!miss.found);
    assert_eq!(miss.dir, PathBuf::from("./webapp/dist"));
    assert_eq!(miss.tried, candidates);

    // no exe dir and no manifest (release binary outside cargo): cwd only
    assert_eq!(
        webapp_dist_candidates(None, None, None),
        vec![PathBuf::from("./webapp/dist")]
    );
}

#[test]
fn webapp_dist_env_override_is_the_only_candidate() {
    let candidates =
        webapp_dist_candidates(Some(" /srv/console "), Some(FsPath::new("/x")), Some("/y"));
    assert_eq!(candidates, vec![PathBuf::from("/srv/console")]);
    let miss = choose_webapp_dist_dir(&candidates, |_| false);
    assert_eq!(miss.dir, PathBuf::from("/srv/console"));
    assert!(
        !miss.found,
        "override is served even without index.html, but flagged"
    );
    // a blank override is ignored rather than mounting an empty path
    assert_eq!(
        webapp_dist_candidates(Some("   "), None, None),
        vec![PathBuf::from("./webapp/dist")]
    );
}

#[tokio::test]
async fn web_console_session_returns_safe_unauthenticated_shape() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], false);
    assert_eq!(body["google_login_enabled"], false);
    assert_eq!(body["passkey_login_enabled"], false);
    assert!(body.get("token").is_none());
    assert!(body.get("cookie").is_none());
}

#[tokio::test]
async fn web_console_session_accepts_authorized_telegram_webapp_principal() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let init_data = make_test_init_data("123456:TEST-TOKEN", 12345678);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["principal_type"], "telegram");
    assert_eq!(body["telegram_user_id"], 12345678);
    assert!(body.get("csrf_token").is_none());
    assert!(body.get("token").is_none());
    assert!(body.get("cookie").is_none());
}

#[tokio::test]
async fn web_console_session_rejects_unauthorized_telegram_webapp_principal() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);
    let init_data = make_test_init_data("123456:TEST-TOKEN", 99999);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("Authorization", format!("TelegramWebApp {init_data}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], false);
    assert!(body.get("csrf_token").is_none());
    assert!(body.get("token").is_none());
    assert!(body.get("cookie").is_none());
}

#[tokio::test]
async fn web_console_session_returns_safe_authenticated_metadata() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["principal_type"], "web_console");
    assert_eq!(body["email"], "l@example.com");
    assert_eq!(body["csrf_token"].as_str().unwrap().len(), 64);
    assert_eq!(body["protected_access_unlocked"], false);
    assert_eq!(
        body["mfa_freshness_secs"],
        WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS
    );
    assert!(body["mfa_expires_at"].is_null());
    assert!(body.get("token").is_none());
}

#[tokio::test]
async fn web_console_session_does_not_ask_setup_when_totp_configured_but_not_fresh() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_totp_configured(&db_path);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["mfa_level"], "google_only");
    assert!(body["mfa_verified_at"].is_null());
    assert_eq!(body["protected_access_unlocked"], false);
    assert_eq!(body["requires_mfa_setup"], false);
    assert!(body["mfa_expires_at"].is_null());
}

#[tokio::test]
async fn web_console_session_asks_setup_when_only_deferred_webauthn_exists() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    insert_test_webauthn_credential(&db_path, "google-sub-test");

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["mfa_level"], "google_only");
    assert_eq!(body["protected_access_unlocked"], false);
    assert_eq!(body["requires_mfa_setup"], true);
    assert!(body["mfa_expires_at"].is_null());
}

#[tokio::test]
async fn web_console_session_unlocks_private_access_after_google_fresh_auth() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_google_fresh_session(&db_path);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["mfa_level"], "google_fresh");
    assert!(body["mfa_verified_at"].is_u64());
    assert!(body["mfa_expires_at"].is_u64());
    assert_eq!(body["protected_access_unlocked"], true);
    assert_eq!(body["requires_mfa_setup"], false);
}

#[tokio::test]
async fn web_console_session_reports_protected_access_window_after_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    mark_test_web_console_session_mfa_verified(&db_path);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let verified_at = body["mfa_verified_at"].as_u64().unwrap();
    let expires_at = body["mfa_expires_at"].as_u64().unwrap();
    assert_eq!(body["mfa_level"], "webauthn");
    assert_eq!(body["protected_access_unlocked"], true);
    assert_eq!(
        expires_at,
        verified_at + WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS
    );
}

#[tokio::test]
async fn web_console_session_keeps_private_access_for_trusted_device_window() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    let verified_at = (chrono::Utc::now().timestamp() as u64)
        .saturating_sub(crate::data_access_gate::DEFAULT_MFA_FRESHNESS_SECS + 60);
    mark_test_web_console_session_mfa_verified_at(&db_path, verified_at);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(
                    "Cookie",
                    format!("__Host-aegis_session={cookie_token}; theme=dark"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mfa_level"], "webauthn");
    assert_eq!(body["protected_access_unlocked"], true);
    assert_eq!(
        body["mfa_expires_at"],
        verified_at + WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS
    );
}

#[tokio::test]
async fn web_console_logout_revokes_session_and_clears_cookie() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let now = chrono::Utc::now().timestamp() as u64;
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/logout")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert!(set_cookie.contains("Max-Age=0"));
    assert!(set_cookie.contains("HttpOnly"));

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    assert!(
        crate::web_console_auth::validate_session(&conn, &cookie_token, now + 1, 30 * 24 * 60 * 60)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn web_console_logout_rejects_missing_csrf_for_cookie_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/logout")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn web_console_mfa_status_requires_auth() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/mfa/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn web_console_mfa_status_returns_setup_required_for_new_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/mfa/status")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["mfa"]["mfa_level"], "google_only");
    assert_eq!(body["mfa"]["requires_mfa_setup"], true);
    assert_eq!(body["mfa"]["webauthn_registered_count"], 0);
    assert_eq!(body["mfa"]["webauthn_available"], false);
    assert_eq!(body["mfa"]["webauthn_status"], "deferred");
    assert_eq!(body["mfa"]["preferred_step_up_method"], "totp");
}

#[tokio::test]
async fn web_console_webauthn_credentials_list_redacted_metadata() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    insert_test_webauthn_credential(&db_path, "google-sub-test");

    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/mfa/webauthn/credentials")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let credential = &body["credentials"][0];
    assert_eq!(credential["id"], "cred-test-1");
    assert_eq!(credential["nickname"], "Test passkey");
    assert!(credential.get("credential_id").is_none());
    assert!(credential.get("public_key_cose").is_none());
    assert!(credential.get("sign_count").is_none());
}

#[tokio::test]
async fn pending_mfa_post_endpoints_require_auth() {
    let state = webapp_test_state();
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/assert/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::CACHE_CONTROL)
            .unwrap(),
        "no-store"
    );
}

#[tokio::test]
async fn pending_mfa_post_endpoints_require_csrf() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let cookie_token = create_test_web_console_session(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/webauthn/assert/start")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pending_mfa_post_endpoints_return_structured_unavailable() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let state = webapp_test_state_with_db(Some(db_path));
    let app = oauth_callback_router(state);

    for (path, feature, message) in [
        (
            "/mfa/webauthn/register/start",
            "webauthn_register_start",
            "Passkey registration is not available in this build. Use TOTP for local MFA.",
        ),
        (
            "/mfa/webauthn/register/finish",
            "webauthn_register_finish",
            "Passkey registration is not available in this build. Use TOTP for local MFA.",
        ),
        (
            "/mfa/webauthn/assert/start",
            "webauthn_assert_start",
            "Passkey verification is not available in this build. Use TOTP for local MFA.",
        ),
        (
            "/mfa/webauthn/assert/finish",
            "webauthn_assert_finish",
            "Passkey verification is not available in this build. Use TOTP for local MFA.",
        ),
        (
            "/mfa/recovery-code/consume",
            "recovery_code_consume",
            "Recovery-code sign-in is not available in this build.",
        ),
        (
            "/mfa/recovery-codes/regenerate",
            "recovery_codes_regenerate",
            "Recovery-code regeneration is not available in this build.",
        ),
        (
            "/mfa/disable",
            "mfa_disable",
            "MFA cannot be disabled from this WebApp.",
        ),
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                    .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED, "{path}");
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["status"], "error", "{path}");
        assert_eq!(body["error"], "mfa_endpoint_not_implemented", "{path}");
        assert_eq!(body["error_kind"], "mfa_endpoint_not_implemented", "{path}");
        assert_eq!(body["feature"], feature, "{path}");
        assert_eq!(body["message"], message, "{path}");
        assert_eq!(body["retryable"], false, "{path}");
    }
}

#[tokio::test]
async fn web_console_totp_setup_finish_and_verify_update_session_mfa() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let aes_key = [7_u8; 32];
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let state = webapp_test_state_with_db_and_mfa_key(db_path.clone(), aes_key);
    let app = oauth_callback_router(state);

    let setup_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/start")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup_resp.status(), StatusCode::OK);
    let setup_body = axum::body::to_bytes(setup_resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let setup_json: serde_json::Value = serde_json::from_slice(&setup_body).unwrap();
    let secret_base32 = setup_json["setup"]["secret_base32"].as_str().unwrap();
    assert_eq!(setup_json["setup"]["algorithm"], "SHA256");
    assert!(
        setup_json["setup"]["otpauth_url"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/")
    );

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let current_counter = now / 30;
    let setup_code =
        crate::web_console_auth::totp_code_from_base32(secret_base32, current_counter).unwrap();
    let finish_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/finish")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "code": setup_code }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(finish_resp.status(), StatusCode::OK);

    let conn = crate::encrypted_sqlite::open(&db_path).unwrap();
    let session =
        crate::web_console_auth::validate_session(&conn, &cookie_token, now, 30 * 24 * 60 * 60)
            .unwrap()
            .unwrap();
    assert_eq!(session.mfa_level, crate::web_console_auth::MfaLevel::Totp);
    assert_eq!(session.mfa_method.as_deref(), Some("totp"));
    let status = crate::web_console_auth::get_mfa_status(&conn, &session).unwrap();
    assert!(status.totp_configured);
    assert!(!status.requires_mfa_setup);
    drop(conn);

    let verify_code =
        crate::web_console_auth::totp_code_from_base32(secret_base32, current_counter + 1).unwrap();
    let verify_resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/verify")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "code": verify_code }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(verify_resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn web_console_totp_rejects_replayed_code() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let aes_key = [8_u8; 32];
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let state = webapp_test_state_with_db_and_mfa_key(db_path, aes_key);
    let app = oauth_callback_router(state);

    let setup_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/start")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let setup_body = axum::body::to_bytes(setup_resp.into_body(), 16 * 1024)
        .await
        .unwrap();
    let setup_json: serde_json::Value = serde_json::from_slice(&setup_body).unwrap();
    let secret_base32 = setup_json["setup"]["secret_base32"].as_str().unwrap();
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let code = crate::web_console_auth::totp_code_from_base32(secret_base32, now / 30).unwrap();

    let finish_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/setup/finish")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({ "code": code }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(finish_resp.status(), StatusCode::OK);

    let replay_resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mfa/totp/verify")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({ "code": code }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay_resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webapp_vnext_mutations_require_auth_and_csrf_for_cookie_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let command_body = serde_json::json!({
        "schema_version": 1,
        "input": "commute",
        "mode": "text",
        "intent_kind": "explain",
        "client_context": "home",
        "idempotency_key": "vnext-command-csrf-test"
    })
    .to_string();
    let action_body = serde_json::json!({
        "schema_version": 1,
        "action_id": "mail.mark_read",
        "idempotency_key": "vnext-action-csrf-test"
    })
    .to_string();
    let debug_snapshot_body = serde_json::json!({
        "schema_version": 1,
        "surface": "home",
        "path": "/ui/?code=secret&state=secret",
        "payload": { "schema_version": 1 },
        "idempotency_key": "vnext-debug-snapshot-csrf-test"
    })
    .to_string();

    let avatar_profile_body = serde_json::json!({
        "schema_version": 1,
        "aegis_avatar_url": "/ui/aegis-avatar.svg",
        "owner_avatar_url": ""
    })
    .to_string();

    for (method, path, body) in [
        ("POST", "/webapp/vnext/command", command_body.as_str()),
        (
            "POST",
            "/webapp/vnext/work/item:1/actions",
            action_body.as_str(),
        ),
        (
            "POST",
            "/webapp/vnext/settings/cost-saving/pause",
            r#"{"schema_version":1,"idempotency_key":"vnext-cost-pause-csrf-test"}"#,
        ),
        (
            "POST",
            "/webapp/vnext/settings/cost-saving/resume",
            r#"{"schema_version":1,"idempotency_key":"vnext-cost-resume-csrf-test"}"#,
        ),
        (
            "POST",
            "/webapp/vnext/presence",
            r#"{"schema_version":1,"surface":"test"}"#,
        ),
        (
            "POST",
            "/webapp/vnext/debug-snapshots",
            debug_snapshot_body.as_str(),
        ),
        (
            "POST",
            "/webapp/vnext/debug-jobs",
            r#"{"schema_version":1,"kind":"webapp_e2e_suite"}"#,
        ),
        (
            "PUT",
            "/webapp/vnext/avatar-profile",
            avatar_profile_body.as_str(),
        ),
    ] {
        let unauth = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        let cookie_token = create_test_web_console_session(&db_path);
        let missing_csrf = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
    }
}
