//! LLM provider/model normalization and settings projection, calendar source delegation, display language, model catalog and model-selection routes.
use super::*;

#[tokio::test]
async fn call_validation_generic_admin_put_and_reset_cannot_bypass() {
    let state = test_state();
    let before = state.settings.get("llm_provider");
    let app = admin_router(state.clone());
    for method in ["PUT", "DELETE"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/settings/llm_provider")
                    .header("Authorization", "Bearer test-secret-token")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"value":"xai_subscription"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(state.settings.get("llm_provider"), before);
        assert!(state.memory.load_all_settings().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn call_validation_provider_and_model_commit_only_after_candidate_probe() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set("llm_provider", serde_json::json!("openrouter"))
        .unwrap();
    state
        .settings
        .set("llm_model", serde_json::json!("old-model"))
        .unwrap();
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state.clone());
    let request = Request::builder().method("POST").uri("/webapp/vnext/settings/llm-provider")
        .header("Cookie", format!("__Host-aegis_session={cookie}"))
        .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf).header("Content-Type", "application/json")
        .body(Body::from(serde_json::json!({"schema_version":1,"idempotency_key":"candidate-pair","llm_provider":"codex_oauth","llm_model":"candidate-model"}).to_string())).unwrap();
    let observed = state.clone();
    let response = crate::call_validation::with_test_probe(Arc::new(move |candidate| {
        assert_eq!(observed.settings.get("llm_provider"), Some(serde_json::json!("openrouter")));
        assert_eq!(observed.settings.get("llm_model"), Some(serde_json::json!("old-model")));
        assert!(matches!(candidate, crate::call_validation::Candidate::Chat {provider,model,..} if provider == "codex_oauth" && model == "candidate-model"));
        Ok(())
    }), app.oneshot(request)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let persisted = state.memory.load_all_settings().await.unwrap();
    assert_eq!(
        persisted.get("llm_provider"),
        Some(&serde_json::json!("codex_oauth"))
    );
    assert_eq!(
        persisted.get("llm_model"),
        Some(&serde_json::json!("candidate-model"))
    );
}

#[tokio::test]
async fn call_validation_failure_preserves_live_and_persisted_settings_and_allows_offline_disable()
{
    let _catalog = seed_test_model_catalogs().await;
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let mut state = webapp_test_state_with_db(Some(db_path.clone()));
    let configured = Arc::get_mut(&mut state).unwrap();
    configured.credential_db_path = Some(db_path.clone());
    configured.credential_aes_key = Some([42; 32]);
    state
        .settings
        .set("llm_provider", serde_json::json!("openrouter"))
        .unwrap();
    state
        .settings
        .set("llm_model", serde_json::json!("deepseek/deepseek-v4-pro"))
        .unwrap();
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    let app = oauth_callback_router(state.clone());
    let keys = [
        "llm_provider",
        "llm_model",
        "fallback_model",
        "fallback_model_authorized",
        "paid_api_fallback_enabled",
        "web_search_provider",
        "web_search_model",
        "codex_enabled",
        "xai_subscription_enabled",
    ];
    let before: Vec<_> = keys.iter().map(|key| state.settings.get(key)).collect();
    let persisted = state.memory.load_all_settings().await.unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for (path, body) in [
        (
            "llm-provider",
            serde_json::json!({"llm_provider":"codex_oauth", "llm_model":"gpt-5.5"}),
        ),
        (
            "llm-model",
            serde_json::json!({"llm_model":"deepseek/deepseek-v4-pro"}),
        ),
        (
            "web-search",
            serde_json::json!({"web_search_provider":"codex_oauth"}),
        ),
        ("fallback-model", serde_json::json!({"model":"gpt-5.5"})),
        ("paid-api-fallback", serde_json::json!({"enabled":true})),
        (
            "jev",
            serde_json::json!({"enabled":true,"consent":true,"api_key":"sk-or-synthetic-rejected-settings-key"}),
        ),
    ] {
        let mut body = body;
        body["schema_version"] = serde_json::json!(1);
        body["idempotency_key"] = serde_json::json!(format!("validation-{path}"));
        if path == "jev" {
            body.as_object_mut().unwrap().remove("schema_version");
            body.as_object_mut().unwrap().remove("idempotency_key");
        }
        let request = Request::builder()
            .method("POST")
            .uri(format!("/webapp/vnext/settings/{path}"))
            .header("Cookie", format!("__Host-aegis_session={cookie}"))
            .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let captured = seen.clone();
        let response = crate::call_validation::with_test_probe(
            Arc::new(move |candidate| {
                use crate::call_validation::Candidate;
                captured.lock().unwrap().push(match candidate {
                    Candidate::Chat {
                        provider, model, ..
                    } => format!("chat:{provider}:{model}"),
                    Candidate::Search { provider, .. } => format!("search:{provider}"),
                    Candidate::Jev { .. } => "jev".into(),
                });
                anyhow::bail!("401 synthetic-private-provider-detail")
            }),
            app.clone().oneshot(request),
        )
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let message = String::from_utf8_lossy(&bytes);
        assert!(!message.contains("synthetic-private-provider-detail"));
        assert!(message.contains("未保存") || message.contains("Not saved"));
        assert_eq!(
            keys.iter()
                .map(|key| state.settings.get(key))
                .collect::<Vec<_>>(),
            before,
            "{path}"
        );
        assert_eq!(
            state.memory.load_all_settings().await.unwrap(),
            persisted,
            "{path}"
        );
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        6,
        "one direct probe per attempted save; no fallback"
    );
    assert_eq!(seen.lock().unwrap()[2], "search:codex_oauth");
    assert_eq!(
        seen.lock().unwrap()[4],
        "chat:openrouter:deepseek/deepseek-v4-pro"
    );
    let backup_request = Request::builder().method("POST").uri("/webapp/vnext/settings/fallback-model")
        .header("Cookie", format!("__Host-aegis_session={cookie}"))
        .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf).header("Content-Type", "application/json")
        .body(Body::from(serde_json::json!({"schema_version":1,"idempotency_key":"verified-backup","model":"gpt-5.5"}).to_string())).unwrap();
    let checked = state.clone();
    let response = crate::call_validation::with_test_probe(
        Arc::new(move |_| {
            assert_eq!(
                checked.settings.get("fallback_model_authorized"),
                Some(serde_json::json!(false))
            );
            Ok(())
        }),
        app.clone().oneshot(backup_request),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state.settings.get("fallback_model_authorized"),
        Some(serde_json::json!(true))
    );
    for (path, body) in [
        (
            "fallback-model",
            serde_json::json!({"schema_version":1,"idempotency_key":"clear-backup", "model":""}),
        ),
        (
            "paid-api-fallback",
            serde_json::json!({"schema_version":1,"idempotency_key":"disable-paid", "enabled":false}),
        ),
        (
            "jev",
            serde_json::json!({"enabled":false,"remove_key":true}),
        ),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/webapp/vnext/settings/{path}"))
            .header("Cookie", format!("__Host-aegis_session={cookie}"))
            .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = crate::call_validation::with_test_probe(
            Arc::new(|_| panic!("disable must not call provider")),
            app.clone().oneshot(request),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
    crate::model_catalog::clear_model_catalog_caches_for_tests();
    assert_eq!(
        state.settings.get("fallback_model_authorized"),
        Some(serde_json::json!(false))
    );
}

#[tokio::test]
async fn jev_settings_router_requires_auth_csrf_and_consent_and_never_returns_key() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let mut state = webapp_test_state_with_db(Some(db_path.clone()));
    let configured = Arc::get_mut(&mut state).unwrap();
    configured.credential_db_path = Some(db_path.clone());
    configured.credential_aes_key = Some([42; 32]);
    let app = settings_router_with_successful_probe(state);
    let (cookie, csrf) = create_test_web_console_session_with_csrf(&db_path);
    let path = "/webapp/vnext/settings/jev";
    let key = "sk-or-synthetic-router-settings-key";
    let anonymous = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        anonymous.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));
    for (data, with_csrf, expected) in [
        (
            serde_json::json!({"enabled": true, "consent": true, "api_key": key}),
            false,
            StatusCode::FORBIDDEN,
        ),
        (
            serde_json::json!({"enabled": true, "api_key": key}),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"enabled": true, "consent": true, "api_key": "synthetic-openai-key-not-openrouter"}),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"enabled": true, "consent": true, "api_key": key}),
            true,
            StatusCode::OK,
        ),
        (serde_json::json!({"enabled": false}), true, StatusCode::OK),
        (
            serde_json::json!({"enabled": true, "consent": true}),
            true,
            StatusCode::OK,
        ),
        (
            serde_json::json!({"enabled": true, "consent": true, "remove_key": true}),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"enabled": false, "remove_key": true}),
            true,
            StatusCode::OK,
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header("Cookie", format!("__Host-aegis_session={cookie}"))
            .header("Content-Type", "application/json");
        if with_csrf {
            request = request.header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(data.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(key));
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("Cookie", format!("__Host-aegis_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["saved_key"], false);
    assert_eq!(status["verified"], false);
}

#[test]
fn normalize_llm_provider_accepts_supported_set_and_rejects_others() {
    // The three webapp-switchable providers (frozen contract).
    assert_eq!(normalize_llm_provider("openrouter"), Some("openrouter"));
    assert_eq!(normalize_llm_provider("codex_oauth"), Some("codex_oauth"));
    assert_eq!(
        normalize_llm_provider("xai_subscription"),
        Some("xai_subscription")
    );
    // Surrounding whitespace is tolerated.
    assert_eq!(normalize_llm_provider("  openrouter "), Some("openrouter"));
    // Everything else is rejected (no silent fallback): registered-but-not
    // owner-switchable backends, unknown values, and empties all 400.
    assert_eq!(normalize_llm_provider("openai_api"), None);
    assert_eq!(normalize_llm_provider("local"), None);
    assert_eq!(normalize_llm_provider("openai_compat"), None);
    assert_eq!(normalize_llm_provider("grok_subscription"), None);
    assert_eq!(normalize_llm_provider(""), None);
    assert_eq!(normalize_llm_provider("OpenRouter"), None);
}

#[test]
fn vnext_settings_projection_reflects_llm_provider_and_paid_api_fallback() {
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
            "llm_provider",
            serde_json::Value::String("xai_subscription".into()),
        )
        .unwrap();
    settings
        .set("paid_api_fallback_enabled", serde_json::Value::Bool(true))
        .unwrap();
    let principal = settings_projection_test_principal();

    let projected = vnext_settings_projection(&principal, &projection, Some(&settings));

    assert_eq!(
        projected.llm_provider, "xai_subscription",
        "projection must surface the persisted llm_provider"
    );
    assert!(
        projected.paid_api_fallback_enabled,
        "projection must surface the persisted paid_api_fallback_enabled"
    );
}

#[test]
fn vnext_settings_projection_llm_provider_defaults_to_openrouter() {
    // Frozen-contract defaults when neither setting was explicitly stored.
    // (`get` returns the registry default for `llm_provider`, but the contract
    // mandates the projection default be "openrouter"/false; the registry seeds
    // codex_oauth, which IS a stored-via-get value, so we assert against a
    // settings instance that has no override and rely on get() returning the
    // registry seed — here we instead clear by using a value that is not the
    // three-set to confirm passthrough, and a fresh-default for fallback.)
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    // No settings at all → hard contract defaults.
    let principal = settings_projection_test_principal();
    let projected = vnext_settings_projection(&principal, &projection, None);
    assert_eq!(
        projected.llm_provider, "openrouter",
        "absent settings must default llm_provider to openrouter"
    );
    assert!(
        !projected.paid_api_fallback_enabled,
        "absent settings must default paid_api_fallback_enabled to false"
    );
    assert_eq!(
        projected.web_search_provider, "auto",
        "absent settings must follow the chat subscription"
    );
    assert_eq!(projected.web_search_model, "");
    assert!(projected.web_search_model_options.is_empty());
    assert!(
        projected
            .web_search_model_options
            .iter()
            .all(|id| !id.contains(":online"))
    );
}

#[test]
fn normalize_llm_model_accepts_arbitrary_slugs_and_rejects_empty_or_oversized() {
    // Arbitrary OpenRouter-style slugs pass through unchanged (no allow-list).
    assert_eq!(
        normalize_llm_model("deepseek/deepseek-v4-flash"),
        Some("deepseek/deepseek-v4-flash".to_string())
    );
    assert_eq!(
        normalize_llm_model("google/gemini-3.1-flash-lite"),
        Some("google/gemini-3.1-flash-lite".to_string())
    );
    // Surrounding whitespace is trimmed.
    assert_eq!(
        normalize_llm_model("  openai/gpt-5  "),
        Some("openai/gpt-5".to_string())
    );
    // Empty means provider default (accepted at normalize; catalog validation
    // separately allows empty llm_model).
    assert_eq!(normalize_llm_model(""), Some(String::new()));
    assert_eq!(normalize_llm_model("   "), Some(String::new()));
    // Over the length cap is rejected.
    let oversized = "a".repeat(LLM_MODEL_MAX_LEN + 1);
    assert_eq!(normalize_llm_model(&oversized), None);
    // Exactly at the cap is accepted.
    let max_len = "b".repeat(LLM_MODEL_MAX_LEN);
    assert_eq!(normalize_llm_model(&max_len), Some(max_len.clone()));
}

#[test]
fn vnext_settings_projection_reflects_llm_model() {
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
            "llm_model",
            serde_json::Value::String("deepseek/deepseek-v4-pro".into()),
        )
        .unwrap();
    let principal = settings_projection_test_principal();

    let projected = vnext_settings_projection(&principal, &projection, Some(&settings));

    assert_eq!(
        projected.llm_model, "deepseek/deepseek-v4-pro",
        "projection must surface the persisted llm_model"
    );
}

#[tokio::test]
async fn calendar_source_is_delegated_when_google_oauth_is_not_configured() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory, None);
    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    let calendar = projection
        .sources
        .iter()
        .find(|source| source.source == "calendar")
        .expect("calendar source");
    assert_eq!(calendar.status, "delegated");
    assert_eq!(calendar.reason.as_deref(), Some("codex_connector"));
}

#[tokio::test]
async fn calendar_source_is_available_when_google_oauth_is_configured() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let state = webapp_test_state_with_orchestrator(memory, None);
    state
        .orchestrator
        .as_ref()
        .expect("test orchestrator")
        .sandbox
        .add_auth_header("www.googleapis.com", "Bearer test-token");
    let projection = build_unified_inbox_projection(&state, 12345678, "en").await;
    let calendar = projection
        .sources
        .iter()
        .find(|source| source.source == "calendar")
        .expect("calendar source");
    assert_eq!(calendar.status, "available");
    assert_eq!(calendar.reason, None);
}

#[test]
fn vnext_source_status_label_delegated() {
    assert_eq!(
        vnext_source_status_label("delegated", "zh"),
        "Codex 访问待验证"
    );
    assert_eq!(
        vnext_source_status_label("delegated", "en"),
        "Codex access not yet verified"
    );
    assert!(!vnext_source_needs_attention("delegated"));
}

#[test]
fn vnext_settings_projection_calendar_delegated_is_ready_codex_row() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "delegated".into(),
            reason: Some("codex_connector".into()),
        }],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let zh_settings = RuntimeSettings::new();
    let zh = vnext_settings_projection(
        &settings_projection_test_principal(),
        &projection,
        Some(&zh_settings),
    );
    let calendar = zh
        .connected_services
        .iter()
        .find(|section| section.label == "Calendar")
        .expect("calendar settings row");
    assert_eq!(calendar.health, "quiet");
    assert_eq!(
        calendar.owner_message,
        "日历读取需要 Codex 连接器；执行前会验证访问权限。"
    );
    assert_eq!(calendar.owner_next_step, None);

    let en_settings = RuntimeSettings::new();
    en_settings
        .set("display_language", serde_json::Value::String("en".into()))
        .unwrap();
    let en = vnext_settings_projection(
        &settings_projection_test_principal(),
        &projection,
        Some(&en_settings),
    );
    let calendar = en
        .connected_services
        .iter()
        .find(|section| section.label == "Calendar")
        .expect("calendar settings row");
    assert_eq!(calendar.health, "quiet");
    assert_eq!(
        calendar.owner_message,
        "Calendar reads require a Codex connector; access is verified before execution."
    );
    assert_eq!(calendar.owner_next_step, None);
}

#[test]
fn normalize_display_language_accepts_supported_locales_and_rejects_others() {
    // zh/en variants normalize to the two supported locales.
    assert_eq!(normalize_display_language("zh"), Some("zh"));
    assert_eq!(normalize_display_language("ZH-CN"), Some("zh"));
    assert_eq!(normalize_display_language(" zh-TW "), Some("zh"));
    assert_eq!(normalize_display_language("en"), Some("en"));
    assert_eq!(normalize_display_language("EN-US"), Some("en"));
    // Unsupported values are rejected (no silent fallback) so the
    // display-language endpoint cannot persist an unsupported locale.
    assert_eq!(normalize_display_language("ja"), None);
    assert_eq!(normalize_display_language(""), None);
    assert_eq!(normalize_display_language("fr"), None);
}
async fn seed_test_model_catalogs() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = crate::model_catalog::lock_model_catalog_caches_for_tests().await;
    crate::model_catalog::clear_model_catalog_caches_for_tests();
    crate::model_catalog::seed_provider_cache_for_tests(crate::model_catalog::ProviderCatalog {
        id: "codex_oauth".into(),
        connected: true,
        models: vec![crate::model_catalog::CatalogModel {
            id: "gpt-5.5".into(),
            display_name: "GPT-5.5".into(),
        }],
        source: "codex_cli_bundled_catalog".into(),
        fetched_at_unix: Some(1),
        default_model: String::new(),
    });
    crate::model_catalog::seed_provider_cache_for_tests(crate::model_catalog::ProviderCatalog {
        id: "xai_subscription".into(),
        connected: false,
        models: vec![],
        source: "unavailable".into(),
        fetched_at_unix: None,
        default_model: String::new(),
    });
    crate::model_catalog::seed_provider_cache_for_tests(crate::model_catalog::ProviderCatalog {
        id: "openrouter".into(),
        connected: true,
        models: vec![crate::model_catalog::CatalogModel {
            id: "deepseek/deepseek-v4-pro".into(),
            display_name: "DeepSeek V4 Pro".into(),
        }],
        source: "openrouter_models_api".into(),
        fetched_at_unix: Some(1),
        default_model: String::new(),
    });
    guard
}

#[tokio::test]
async fn webapp_vnext_model_catalog_returns_schema_and_current_settings() {
    let _guard = seed_test_model_catalogs().await;
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("openrouter".into()),
        )
        .unwrap();
    state
        .settings
        .set(
            "llm_model",
            serde_json::Value::String("deepseek/deepseek-v4-pro".into()),
        )
        .unwrap();
    state
        .settings
        .set(
            "fallback_model",
            serde_json::Value::String("gpt-5.5".into()),
        )
        .unwrap();
    let expected_codex_default =
        crate::model_catalog::provider_default_model(&state.settings, "codex_oauth");
    let expected_xai_default =
        crate::model_catalog::provider_default_model(&state.settings, "xai_subscription");
    let app = settings_router_with_successful_probe(state);
    let cookie_token = create_test_web_console_session(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/model-catalog")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
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
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["current"]["provider"], "openrouter");
    assert_eq!(json["current"]["model"], "deepseek/deepseek-v4-pro");
    assert_eq!(
        json["current"]["effective_model"], "deepseek/deepseek-v4-pro",
        "non-empty llm_model is the effective model"
    );
    assert_eq!(json["fallback"]["provider"], "codex_oauth");
    assert_eq!(json["fallback"]["model"], "gpt-5.5");
    let providers = json["providers"].as_array().expect("providers array");
    assert_eq!(providers.len(), 3);
    assert_eq!(providers[0]["id"], "codex_oauth");
    assert_eq!(providers[0]["source"], "cache");
    assert_eq!(providers[0]["default_model"], expected_codex_default);
    assert_eq!(providers[1]["id"], "xai_subscription");
    assert_eq!(providers[1]["source"], "unavailable");
    assert_eq!(providers[1]["default_model"], expected_xai_default);
    assert_eq!(providers[2]["id"], "openrouter");
    assert_eq!(providers[2]["source"], "cache");
    assert_eq!(providers[2]["default_model"], "");
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}

#[tokio::test]
async fn webapp_vnext_model_catalog_effective_model_uses_provider_default_when_empty() {
    let _guard = seed_test_model_catalogs().await;
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".into()),
        )
        .unwrap();
    state
        .settings
        .set("llm_model", serde_json::Value::String(String::new()))
        .unwrap();
    state
        .settings
        .set("codex_model", serde_json::Value::String("gpt-5.5".into()))
        .unwrap();
    let expected_default =
        crate::model_catalog::provider_default_model(&state.settings, "codex_oauth");
    let app = settings_router_with_successful_probe(state);
    let cookie_token = create_test_web_console_session(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/webapp/vnext/model-catalog")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
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
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["current"]["provider"], "codex_oauth");
    assert_eq!(json["current"]["model"], "");
    assert_eq!(json["current"]["effective_model"], expected_default);
    assert_eq!(json["providers"][0]["default_model"], expected_default);
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}

#[tokio::test]
async fn webapp_vnext_set_llm_model_rejects_unknown_catalog_id() {
    let _guard = seed_test_model_catalogs().await;
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".into()),
        )
        .unwrap();
    let app = settings_router_with_successful_probe(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/llm-model")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "llm_model": "not-in-catalog",
                        "idempotency_key": "model-catalog-reject"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("不在") || text.to_ascii_lowercase().contains("not in"),
        "reject body must be owner-safe unknown-model copy, got: {text}"
    );
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}

#[tokio::test]
async fn webapp_vnext_set_llm_model_accepts_catalog_id() {
    let _guard = seed_test_model_catalogs().await;
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".into()),
        )
        .unwrap();
    let app = settings_router_with_successful_probe(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/llm-model")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "llm_model": "gpt-5.5",
                        "idempotency_key": "model-catalog-accept"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        state
            .settings
            .get("llm_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "gpt-5.5"
    );
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}

/// Switching `llm_provider` must clear an explicit `llm_model` so the new
/// provider's default (empty string) is used. A leftover gpt id from
/// OpenRouter must not survive a switch to Grok.
#[tokio::test]
async fn webapp_vnext_set_llm_provider_clears_llm_model_on_switch() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("openrouter".into()),
        )
        .unwrap();
    state
        .settings
        .set("llm_model", serde_json::Value::String("gpt-5.6-sol".into()))
        .unwrap();
    let app = settings_router_with_successful_probe(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/llm-provider")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "llm_provider": "xai_subscription",
                        "idempotency_key": "provider-switch-clears-model"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["llm_provider"], "xai_subscription");
    assert_eq!(
        state.settings.get("xai_subscription_enabled"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        state.settings.get("external_executor_enabled"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        json["llm_model"], "",
        "provider switch must reset llm_model to the new provider default"
    );
    assert_eq!(
        state
            .settings
            .get("llm_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "xai_subscription"
    );
    assert_eq!(
        state
            .settings
            .get("llm_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "",
        "persisted llm_model must be empty after a provider switch"
    );
}

/// Re-posting the already-active provider must not wipe a chosen model.
#[tokio::test]
async fn webapp_vnext_set_llm_provider_same_provider_keeps_llm_model() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "llm_provider",
            serde_json::Value::String("openrouter".into()),
        )
        .unwrap();
    state
        .settings
        .set(
            "llm_model",
            serde_json::Value::String("deepseek/deepseek-v4-pro".into()),
        )
        .unwrap();
    let app = settings_router_with_successful_probe(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/llm-provider")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "llm_provider": "openrouter",
                        "idempotency_key": "provider-same-keeps-model"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        state
            .settings
            .get("llm_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "deepseek/deepseek-v4-pro",
        "same-provider write must leave the explicit llm_model in place"
    );
}

/// POST /webapp/vnext/settings/fallback-model is CSRF-gated and rejects a body
/// without a model id (router-level evidence for the route-coverage ratchet).
#[tokio::test]
async fn webapp_vnext_set_fallback_model_requires_csrf_and_model() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    let app = settings_router_with_successful_probe(state);
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);
    let body = serde_json::json!({
        "schema_version": 1,
        "idempotency_key": "fallback-model-missing"
    })
    .to_string();

    // Session but no CSRF header: refused before the body is read.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/fallback-model")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "missing CSRF header must be refused, got {}",
        resp.status()
    );

    // CSRF present but no model id: 400 with the owner-safe message.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/fallback-model")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("model is required"),
        "expected the missing-model rejection, got: {text}"
    );
}

/// POST /webapp/vnext/settings/web-search is CSRF-gated, switches the search
/// provider without touching the saved chat model, and keeps `:online` only
/// for an explicit OpenRouter choice.
#[tokio::test]
async fn webapp_vnext_set_web_search_does_not_change_chat_model() {
    let db = tempfile::NamedTempFile::new().unwrap();
    let db_path = db.path().to_string_lossy().to_string();
    let state = webapp_test_state_with_db(Some(db_path.clone()));
    state
        .settings
        .set(
            "xai_subscription_model",
            serde_json::Value::String("grok-4.6".into()),
        )
        .unwrap();
    state
        .settings
        .set(
            "web_search_model",
            serde_json::Value::String("grok-4.7".into()),
        )
        .unwrap();
    let app = settings_router_with_successful_probe(state.clone());
    let (cookie_token, csrf_token) = create_test_web_console_session_with_csrf(&db_path);

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/web-search")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "idempotency_key": "web-search-no-csrf"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        missing.status().is_client_error(),
        "missing CSRF header must be refused, got {}",
        missing.status()
    );

    let empty = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/web-search")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "idempotency_key": "web-search-empty"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    let switched = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/web-search")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, &csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "web_search_provider": "openrouter",
                        "idempotency_key": "web-search-openrouter"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(switched.status(), StatusCode::OK);
    let body = axum::body::to_bytes(switched.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["web_search_provider"], "openrouter");
    assert_eq!(json["web_search_model"], "x-ai/grok-4.7:online");
    assert_eq!(
        state
            .settings
            .get("xai_subscription_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "grok-4.6",
        "search settings must not rewrite the saved chat model"
    );

    let back = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webapp/vnext/settings/web-search")
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "web_search_provider": "xai_subscription",
                        "web_search_model": "grok-4.7",
                        "idempotency_key": "web-search-subscription"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(back.status(), StatusCode::OK);
    assert_eq!(
        state
            .settings
            .get("web_search_provider")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "xai_subscription"
    );
    assert_eq!(
        state
            .settings
            .get("web_search_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "grok-4.7"
    );
    assert_eq!(
        state
            .settings
            .get("xai_subscription_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        "grok-4.6"
    );
}
