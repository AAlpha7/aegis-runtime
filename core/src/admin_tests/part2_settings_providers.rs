//! vNext settings projection, avatar profile, cost-saving pause, assistant model-failure mapping, chat provider selection, Codex CLI parsers.
use super::*;

#[test]
fn vnext_settings_projection_includes_cost_controls_when_settings_are_available() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings
        .set("mcp_enabled", serde_json::Value::Bool(false))
        .unwrap();
    let principal = WebConsolePrincipal::WebConsole(crate::web_console_auth::WebConsoleSession {
        session_id_hash: "session".into(),
        google_sub: "google-sub".into(),
        email: "l@example.com".into(),
        email_verified: true,
        created_at: 1,
        expires_at: 2,
        idle_expires_at: 2,
        last_seen_at: Some(1),
        revoked_at: None,
        mfa_level: crate::web_console_auth::MfaLevel::GoogleFresh,
        mfa_verified_at: Some(1),
        mfa_method: Some("google".into()),
    });

    let projected = vnext_settings_projection(&principal, &projection, Some(&settings));

    assert!(
        projected
            .notification_preferences
            .iter()
            .any(|section| { section.label == "Cost controls" || section.label == "省钱控制" })
    );
}

#[test]
fn vnext_settings_projection_carries_configured_user_timezone() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("America/New_York".into()),
        )
        .unwrap();
    let principal = settings_projection_test_principal();

    let projected = vnext_settings_projection(&principal, &projection, Some(&settings));

    assert_eq!(
        projected.user_timezone, "America/New_York",
        "projection must surface the stored user_timezone IANA string for Intl"
    );
}

#[test]
fn vnext_settings_projection_user_timezone_always_valid_iana() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    // Fresh settings: user_timezone is NOT stored (only a registry default).
    // The projection must surface a non-empty, parseable IANA string (so the
    // frontend's Intl never receives junk). The DB-stored > env > default
    // priority itself is unit-tested in settings.rs; here we only assert the
    // projection never leaks an unusable value. (Avoids mutating process env,
    // which the repo's `unsafe` lint forbids.)
    let settings = RuntimeSettings::new();
    let principal = settings_projection_test_principal();

    let projected = vnext_settings_projection(&principal, &projection, Some(&settings));

    assert!(
        !projected.user_timezone.trim().is_empty(),
        "user_timezone must never be empty"
    );
    assert!(
        projected.user_timezone.parse::<chrono_tz::Tz>().is_ok(),
        "user_timezone must be a valid IANA zone, got {:?}",
        projected.user_timezone
    );
}

#[tokio::test]
async fn webapp_vnext_avatar_profile_round_trips_for_owner_session() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let initial = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/webapp/vnext/avatar-profile")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial.status(), StatusCode::OK);
    let initial_body = axum::body::to_bytes(initial.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let initial_json: serde_json::Value = serde_json::from_slice(&initial_body).unwrap();
    assert_eq!(initial_json["aegis_avatar_url"], "/ui/aegis-avatar-v2.jpg");
    assert_eq!(initial_json["owner_avatar_url"], "");

    let owner_avatar = "data:image/png;base64,YXZhdGFy";
    let aegis_avatar = "data:image/jpeg;base64,YWVnaXM=";
    let saved = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/webapp/vnext/avatar-profile")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "aegis_avatar_url": aegis_avatar,
                        "owner_avatar_url": owner_avatar
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);

    let loaded = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/webapp/vnext/avatar-profile")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(loaded.status(), StatusCode::OK);
    let loaded_body = axum::body::to_bytes(loaded.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let loaded_json: serde_json::Value = serde_json::from_slice(&loaded_body).unwrap();
    assert_eq!(loaded_json["aegis_avatar_url"], aegis_avatar);
    assert_eq!(loaded_json["owner_avatar_url"], owner_avatar);
    assert!(loaded_json["updated_at_unix"].as_i64().is_some());
}

#[tokio::test]
async fn webapp_vnext_avatar_profile_rejects_remote_or_script_urls() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = oauth_callback_router(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    for avatar_url in ["https://example.com/avatar.png", "javascript:alert(1)"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/webapp/vnext/avatar-profile")
                    .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                    .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "schema_version": 1,
                            "aegis_avatar_url": avatar_url,
                            "owner_avatar_url": ""
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn webapp_vnext_pause_cost_saving_turns_off_background_autonomy() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .register_skill("calendar_reader", "Calendar heartbeat checks", true);
    state
        .settings
        .register_skill("server_monitor", "Server heartbeat checks", true);
    state
        .settings
        .set("skill_calendar_reader", serde_json::Value::Bool(true))
        .unwrap();
    state
        .settings
        .set("skill_server_monitor", serde_json::Value::Bool(true))
        .unwrap();
    for key in [
        "heartbeat_autonomy_v2",
        "news_digest",
        "calendar_briefing",
        "server_monitoring",
        "github_alerts",
        "email_notifications",
        "skill_news_digest",
    ] {
        state
            .memory
            .store_setting(key, serde_json::Value::Bool(true))
            .await
            .unwrap();
    }
    state
        .settings
        .set("health_probe_interval_minutes", serde_json::json!(15))
        .unwrap();
    state
        .settings
        .set(
            "true_heartbeat_config",
            serde_json::to_value(crate::chronos::TrueHeartbeatConfig {
                enabled: true,
                interval_minutes: 5,
                quiet_hours: None,
                autonomous_overrides: None,
            })
            .unwrap(),
        )
        .unwrap();
    let mut heartbeat_config = crate::heartbeat_config::HeartbeatConfig::default();
    heartbeat_config.per_tick.tool_calls = 3;
    heartbeat_config.per_tick.send_message = 1;
    heartbeat_config.controlled_thaw.enabled = true;
    heartbeat_config.controlled_thaw.max_llm_wakeups_per_hour = 4;
    heartbeat_config
        .controlled_thaw
        .max_background_proposals_per_day = 3;
    state
        .settings
        .set("heartbeat_config", heartbeat_config.to_json())
        .unwrap();

    let app = oauth_callback_router(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/cost-saving/pause")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(
                    crate::web_console_auth::CSRF_HEADER_NAME,
                    csrf_token.clone(),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "idempotency_key": "pause-cost-saving-test"
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
    assert_eq!(json["cost_saving_paused"], true);
    assert!(vnext_cost_saving_paused(&state.settings));
    for key in ["skill_calendar_reader", "skill_server_monitor"] {
        assert!(!state.settings.is_enabled(key), "{key} should be disabled");
    }
    assert_eq!(
        state
            .settings
            .get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60),
        0
    );
    assert_eq!(
        state
            .settings
            .get("true_heartbeat_config")
            .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool)),
        Some(false)
    );
    let heartbeat = state
        .settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .expect("paused heartbeat config should be stored");
    assert_eq!(heartbeat.per_tick.tool_calls, 0);
    assert_eq!(heartbeat.per_tick.send_message, 0);
    assert!(!heartbeat.controlled_thaw.enabled);
    assert_eq!(heartbeat.controlled_thaw.max_llm_wakeups_per_hour, 0);
    assert_eq!(
        heartbeat.controlled_thaw.max_background_proposals_per_day,
        0
    );
    assert!(
        heartbeat
            .controlled_thaw
            .require_approval_for_external_actions
    );

    let persisted = state.memory.load_all_settings().await.unwrap();
    for key in [
        "heartbeat_autonomy_v2",
        "news_digest",
        "calendar_briefing",
        "server_monitoring",
        "github_alerts",
        "email_notifications",
        "skill_news_digest",
    ] {
        assert_eq!(
            persisted.get(key),
            Some(&serde_json::json!(false)),
            "{key} should be persisted false"
        );
    }

    let snapshot = state
        .settings
        .get("cost_saving_pause_snapshot")
        .and_then(|value| value.as_object().cloned())
        .expect("pause should save a restorable snapshot");
    assert_eq!(
        snapshot.get("skill_calendar_reader"),
        Some(&serde_json::json!(true))
    );

    let resume = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/cost-saving/resume")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "idempotency_key": "resume-cost-saving-test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resume.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resume.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["cost_saving_paused"], false);
    assert!(!vnext_cost_saving_paused(&state.settings));
    for key in ["skill_calendar_reader", "skill_server_monitor"] {
        assert!(state.settings.is_enabled(key), "{key} should be restored");
    }
    assert_eq!(
        state
            .settings
            .get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60),
        15
    );
    assert_eq!(
        state
            .settings
            .get("true_heartbeat_config")
            .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool)),
        Some(true)
    );
    let restored_heartbeat = state
        .settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .expect("restored heartbeat config should be stored");
    assert_eq!(restored_heartbeat.per_tick.tool_calls, 3);
    assert_eq!(restored_heartbeat.per_tick.send_message, 1);
    assert!(restored_heartbeat.controlled_thaw.enabled);
    assert_eq!(
        restored_heartbeat.controlled_thaw.max_llm_wakeups_per_hour,
        4
    );
    assert_eq!(
        restored_heartbeat
            .controlled_thaw
            .max_background_proposals_per_day,
        3
    );
    assert_eq!(
        state.settings.get("cost_saving_pause_snapshot"),
        Some(serde_json::json!({}))
    );
}

#[test]
fn webapp_assistant_model_failure_maps_provider_credit_exhaustion() {
    crate::llm_client::reset_provider_credit_exhaustion_for_test();
    let (status, message) = webapp_assistant_model_failure(
        &"failed to deserialize api response: {\"error\":{\"message\":\"Insufficient credits\",\"code\":402}}",
    );
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        message,
        crate::i18n::t("console.model.credits", crate::i18n::DEFAULT_LOCALE)
    );

    let (status, message) = webapp_assistant_model_failure(&"temporary upstream timeout");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message,
        "Assistant model is unavailable: temporary upstream timeout"
    );

    let (status, message) = webapp_assistant_model_failure(
        &"connection refused api_key=sk-supersecretvalue token=abcdef",
    );
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(message.starts_with("Assistant model is unavailable: "));
    assert!(message.contains("connection refused"));
    assert!(!message.contains("sk-supersecretvalue"));
    assert!(!message.contains("abcdef"));

    let (status, message) = webapp_assistant_model_failure(
        &"xAI subscription chat completion returned 400 Bad Request: {\"code\":\"Client specified an invalid argument\",\"error\":\"Model not found: grok-4.1-fast\"}",
    );
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message,
        crate::i18n::t("console.model.grok_missing", crate::i18n::DEFAULT_LOCALE)
    );

    let (status, message) = webapp_assistant_model_failure(
        &"xAI subscription chat completion returned 400 Bad Request: {\"code\":\"Client specified an invalid argument\",\"error\":\"Invalid arguments passed to the model.\"}",
    );
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message,
        crate::i18n::t("console.model.grok_shape", crate::i18n::DEFAULT_LOCALE)
    );
    crate::llm_client::reset_provider_credit_exhaustion_for_test();
}

#[test]
fn webapp_model_failure_keeps_safe_notices_without_changing_status() {
    let error = crate::behavior_notice::attach_failure(
        anyhow::anyhow!("temporary upstream timeout"),
        vec!["Fallback executor was attempted; capabilities may differ.".into()],
    );
    let (status, message) = webapp_assistant_model_failure_with_notices(&error);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(message.starts_with("Assistant model is unavailable"));
    assert!(message.contains("Fallback executor was attempted"));
}

#[test]
fn webapp_assistant_model_failure_maps_wrapped_primary_failure_during_credit_cooldown() {
    crate::llm_client::reset_provider_credit_exhaustion_for_test();
    crate::llm_client::record_provider_credit_exhaustion("test");

    let (status, message) = webapp_assistant_model_failure(
        &"主模型 x-ai/grok-4.1-fast 不可用。如需启用备用模型，请设置 fallback_model_authorized = true (Primary model x-ai/grok-4.1-fast unavailable. Set fallback_model_authorized = true to enable auto-fallback)",
    );

    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        message,
        crate::i18n::t("console.model.credits", crate::i18n::DEFAULT_LOCALE)
    );
    crate::llm_client::reset_provider_credit_exhaustion_for_test();
}

/// FIX C (i18n): the webapp contextual-assistant owner-facing unavailable /
/// empty-response strings must resolve in both locales with no opposite-language
/// leak. These are Channel-B direct owner display, not LLM-fed.
#[test]
fn webapp_assistant_unavailable_and_empty_localize() {
    for key in [
        "assistant.unavailable.body",
        "assistant.unavailable.warning",
        "assistant.empty_response",
        "assistant.turn_unanswered",
        "assistant.turn_unanswered.with_reason",
    ] {
        let zh = crate::i18n::t(key, "zh");
        let en = crate::i18n::t(key, "en");
        assert_ne!(zh, "[MISSING]", "{key} missing zh");
        assert_ne!(en, "[MISSING]", "{key} missing en");
        // zh must contain Han text and (for these fully-translatable strings) no
        // English alphabetic run — i.e. no opposite-language leak.
        assert!(
            zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "{key} zh must contain Chinese: {zh}"
        );
        assert!(
            !zh.chars().any(|c| c.is_ascii_alphabetic()),
            "{key} zh must not leak English: {zh}"
        );
        // en must be ASCII-letter-bearing and carry no Han characters.
        assert!(
            en.chars().any(|c| c.is_ascii_alphabetic()),
            "{key} en must contain English: {en}"
        );
        assert!(
            !en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "{key} en must not leak Chinese: {en}"
        );
    }

    // The refusal helper carries the localized body through to visible_answer.
    let refusal_zh = contextual_assistant_refusal(
        crate::i18n::t("assistant.unavailable.body", "zh"),
        None,
        vec!["model_unavailable".into()],
    );
    assert_eq!(refusal_zh.response_kind, "refusal");
    assert!(
        refusal_zh
            .visible_answer
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
        "zh refusal body must render in Chinese: {}",
        refusal_zh.visible_answer
    );
    assert!(
        !refusal_zh
            .visible_answer
            .chars()
            .any(|c| c.is_ascii_alphabetic()),
        "zh refusal body must not leak English: {}",
        refusal_zh.visible_answer
    );
}

#[tokio::test]
async fn webapp_contextual_assistant_route_preserves_unstored_provider_default() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory, None);
    let orchestrator = state.orchestrator.as_ref().expect("test orchestrator");

    let route =
        route_webapp_contextual_assistant(orchestrator, &orchestrator.active_model(), 12345678)
            .expect("unstored provider default should use the existing chat route");

    assert_eq!(route.provider, crate::llm::LlmProviderKind::OpenAiCompat);
    assert!(!route.is_fallback);
}

#[tokio::test]
async fn webapp_contextual_assistant_route_enforces_explicit_codex_selection() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".to_string()),
        )
        .unwrap();
    let orchestrator = crate::test_tool_helpers::test_orchestrator(memory, settings);

    let err =
        route_webapp_contextual_assistant(&orchestrator, &orchestrator.active_model(), 12345678)
            .expect_err("explicit Codex selection must keep enforcing codex_enabled");

    assert!(
        format!("{err:#}").contains("codex_enabled=false"),
        "unexpected routing error: {err:#}"
    );
}

#[tokio::test]
async fn webapp_chat_provider_selection_keeps_only_one_active_provider() {
    let state = test_state();

    assert_eq!(selected_webapp_chat_provider_key(&state), "openai_compat");
    let initial_grok_status = vnext_grok_status_value(&state);
    assert_eq!(
        initial_grok_status["selected_provider"].as_str(),
        Some("openai_compat")
    );
    assert_eq!(
        initial_grok_status["active_for_chat"].as_bool(),
        Some(false)
    );

    crate::call_validation::with_test_probe(
        Arc::new(|_| Ok(())),
        select_webapp_chat_provider(&state, WebappChatProviderSelection::Codex),
    )
    .await
    .expect("select Codex");
    assert_eq!(
        state.settings.get("external_executor_enabled"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        state.settings.get("external_executor_primary"),
        Some(serde_json::json!("auto"))
    );
    assert_eq!(
        state.settings.get("web_search_provider"),
        Some(serde_json::json!("auto"))
    );
    state
        .settings
        .set("external_executor_enabled", serde_json::json!(false))
        .unwrap();
    assert_eq!(
        state.settings.get("llm_provider"),
        Some(serde_json::Value::String("codex_oauth".to_string()))
    );
    assert_eq!(
        state.settings.get("codex_enabled"),
        Some(serde_json::Value::Bool(true))
    );
    assert_eq!(
        state.settings.get("xai_subscription_enabled"),
        Some(serde_json::Value::Bool(false))
    );
    assert_eq!(
        state.settings.get("paid_api_fallback_enabled"),
        Some(serde_json::Value::Bool(false))
    );

    crate::call_validation::with_test_probe(
        Arc::new(|_| Ok(())),
        select_webapp_chat_provider(&state, WebappChatProviderSelection::Grok),
    )
    .await
    .expect("select Grok");
    assert_eq!(
        state.settings.get("external_executor_enabled"),
        Some(serde_json::json!(false)),
        "explicit opt-out survives rebind"
    );
    assert_eq!(
        state.settings.get("llm_provider"),
        Some(serde_json::Value::String("xai_subscription".to_string()))
    );
    assert_eq!(
        state.settings.get("codex_enabled"),
        Some(serde_json::Value::Bool(false))
    );
    assert_eq!(
        state.settings.get("xai_subscription_enabled"),
        Some(serde_json::Value::Bool(true))
    );
    assert_eq!(
        state.settings.get("codex_allow_background_tasks"),
        Some(serde_json::Value::Bool(false))
    );
    assert_eq!(
        state
            .settings
            .get("xai_subscription_allow_background_tasks"),
        Some(serde_json::Value::Bool(false))
    );
}

#[tokio::test]
async fn webapp_chat_provider_disable_falls_back_to_safe_base_provider() {
    let state = test_state();
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".to_string()),
        )
        .unwrap();
    state
        .settings
        .set("codex_enabled", serde_json::Value::Bool(false))
        .unwrap();
    state
        .settings
        .set("xai_subscription_enabled", serde_json::Value::Bool(false))
        .unwrap();

    assert_eq!(
        fallback_webapp_chat_provider_after_disable(&state, WebappChatProviderSelection::Codex),
        "openai_compat"
    );

    state
        .settings
        .set("xai_subscription_enabled", serde_json::Value::Bool(true))
        .unwrap();
    assert_eq!(
        fallback_webapp_chat_provider_after_disable(&state, WebappChatProviderSelection::Codex),
        "xai_subscription"
    );

    state
        .settings
        .set("xai_subscription_enabled", serde_json::Value::Bool(false))
        .unwrap();
    assert_eq!(
        fallback_webapp_chat_provider_after_disable(&state, WebappChatProviderSelection::Grok),
        "openai_compat"
    );
}

#[test]
fn codex_cli_status_accepts_chatgpt_login_only() {
    let chatgpt = parse_codex_cli_login_status(true, "Logged in using ChatGPT", "");
    assert!(chatgpt.binary_available);
    assert!(chatgpt.logged_in);
    assert_eq!(chatgpt.auth_kind.as_deref(), Some("ChatGPT"));

    let api_key = parse_codex_cli_login_status(true, "Logged in using API key", "");
    assert!(api_key.binary_available);
    assert!(!api_key.logged_in);
    assert_eq!(api_key.auth_kind.as_deref(), Some("API key"));
}

#[test]
fn codex_cli_device_auth_parser_extracts_official_url_and_code() {
    let raw = "\
Welcome to Codex [v\u{1b}[90m0.132.0\u{1b}[0m]
Follow these steps to sign in with ChatGPT using device code authorization:

1. Open this link in your browser and sign in to your account
   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m

2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m
   \u{1b}[94mCJHA-0F3P0\u{1b}[0m
";

    let parsed = parse_codex_cli_device_auth_output(raw).expect("device auth output parsed");

    assert_eq!(parsed.0, "https://auth.openai.com/codex/device");
    assert_eq!(parsed.1, "CJHA-0F3P0");
}

#[test]
fn codex_model_catalog_parser_preserves_reasoning_levels() {
    let raw = r#"{
      "models": [
        {
          "slug": "gpt-5.5",
          "display_name": "GPT-5.5",
          "default_reasoning_level": "medium",
          "supported_reasoning_levels": [
            {"effort": "low"},
            {"effort": "medium"},
            {"effort": "high"},
            {"effort": "xhigh"}
          ]
        },
        {
          "slug": "hidden-model",
          "display_name": "Hidden",
          "visibility": "hidden",
          "supported_reasoning_levels": [{"effort": "medium"}]
        }
      ]
    }"#;

    let options = parse_codex_model_options(raw).expect("models parsed");

    assert_eq!(options.len(), 1);
    assert_eq!(options[0]["slug"], "gpt-5.5");
    assert_eq!(options[0]["default_reasoning_level"], "medium");
    assert_eq!(
        options[0]["supported_reasoning_levels"],
        serde_json::json!(["low", "medium", "high", "xhigh"])
    );
}

#[test]
fn codex_app_server_model_list_parser_preserves_reasoning_levels() {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": {
            "data": [
                {
                    "id": "gpt-5.5-id",
                    "model": "gpt-5.5",
                    "displayName": "GPT-5.5",
                    "description": "Frontier model",
                    "hidden": false,
                    "supportedReasoningEfforts": [
                        { "reasoningEffort": "low", "description": "Fast" },
                        { "reasoningEffort": "medium", "description": "Balanced" },
                        { "reasoningEffort": "high", "description": "Deep" },
                        { "reasoningEffort": "xhigh", "description": "Extra deep" }
                    ],
                    "defaultReasoningEffort": "medium",
                    "isDefault": true
                },
                {
                    "id": "hidden-id",
                    "model": "hidden-model",
                    "displayName": "Hidden",
                    "hidden": true,
                    "supportedReasoningEfforts": [
                        { "reasoningEffort": "medium", "description": "Balanced" }
                    ],
                    "defaultReasoningEffort": "medium",
                    "isDefault": false
                }
            ],
            "nextCursor": null
        }
    });

    let options = parse_codex_app_server_model_options(&response).expect("models parsed");

    assert_eq!(options.len(), 1);
    assert_eq!(options[0]["slug"], "gpt-5.5");
    assert_eq!(options[0]["display_name"], "GPT-5.5");
    assert_eq!(options[0]["default_reasoning_level"], "medium");
    assert_eq!(options[0]["is_default"], true);
    assert_eq!(
        options[0]["supported_reasoning_levels"],
        serde_json::json!(["low", "medium", "high", "xhigh"])
    );
}

#[test]
fn builtin_avatar_migration_preserves_custom_profiles() {
    for old in ["/aegis-avatar.svg", "/ui/aegis-avatar.svg"] {
        assert_eq!(
            validate_vnext_avatar_url(old, true).unwrap(),
            WEBAPP_DEFAULT_AEGIS_AVATAR_URL
        );
    }
    let custom = "data:image/jpeg;base64,YWVnaXM=";
    assert_eq!(validate_vnext_avatar_url(custom, true).unwrap(), custom);
    assert!(validate_vnext_avatar_url("https://tracker.invalid/avatar.jpg", true).is_err());
}
