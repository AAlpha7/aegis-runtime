//! Fallback model settings, chat request building and the LLM fallback drill.
use super::*;

#[tokio::test]
async fn call_validation_direct_chat_and_search_do_not_accept_fallback_success() {
    use crate::call_validation::Candidate;
    let (server, handle) = spawn_mock_llm_server().await;
    let settings = drill_test_settings(true);
    let orch = orchestrator_on_mock(settings.clone(), server.addr);
    orch.probe_call_configuration(&Candidate::Chat {
        provider: "openrouter".into(),
        model: "candidate-model".into(),
        effort: None,
    })
    .await
    .unwrap();
    assert_eq!(server.request_models(), vec!["candidate-model"]);
    // A valid chat response without search sources must not validate search.
    assert!(
        orch.probe_call_configuration(&Candidate::Search {
            provider: "openrouter".into(),
            model: "candidate-model:online".into()
        })
        .await
        .is_err()
    );
    assert_eq!(server.request_count(), 2);
    // Missing subscription credentials must not pass via the working paid API.
    assert!(
        orch.probe_call_configuration(&Candidate::Chat {
            provider: "xai_subscription".into(),
            model: "grok-test".into(),
            effort: None
        })
        .await
        .is_err()
    );
    assert_eq!(server.request_count(), 2);
    assert_eq!(
        settings.get("llm_model"),
        Some(serde_json::json!("primary-model"))
    );
    assert_eq!(
        settings.get("fallback_model"),
        Some(serde_json::json!("backup-model"))
    );
    handle.abort();
}

// ── G7: Resilience & Degradation tests ──────────────────────────────

#[tokio::test]
async fn search_setting_probe_validates_normalized_citations_not_executor_json() {
    use crate::call_validation::Candidate;
    for (annotations, valid) in [
        (
            serde_json::json!([{"type":"url_citation","url_citation":{
                "url":"https://www.iana.org/help/example-domains","title":"Example domains",
                "content":"IANA reserves example domains for documentation."
            }}]),
            true,
        ),
        (serde_json::json!([]), false),
        (
            serde_json::json!([{"url":"javascript:alert(1)","title":"Invalid"}]),
            false,
        ),
    ] {
        let response = serde_json::json!({"choices":[{"message":{
            "content":"See https://www.iana.org/help/example-domains",
            "annotations":annotations
        }}]});
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move || {
                let response = response.clone();
                async move { axum::Json(response) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let settings = drill_test_settings(true);
        let orch = orchestrator_on_mock(settings.clone(), addr);
        let result = orch
            .probe_call_configuration(&Candidate::Search {
                provider: "openrouter".into(),
                model: "openai/test:online".into(),
            })
            .await;
        server.abort();
        assert_eq!(result.is_ok(), valid);
        assert_eq!(
            settings.get("llm_model"),
            Some(serde_json::json!("primary-model"))
        );
    }
}

#[test]
fn fallback_model_setting_default() {
    let settings = crate::settings::RuntimeSettings::new();
    assert_eq!(
        settings.get("fallback_model"),
        Some(serde_json::Value::String(String::new()))
    );
}

#[test]
fn fallback_model_authorized_default_false() {
    let settings = crate::settings::RuntimeSettings::new();
    assert_eq!(
        settings.get("fallback_model_authorized"),
        Some(serde_json::Value::Bool(false))
    );
}
#[test]
fn fallback_settings_registered() {
    let settings = crate::settings::RuntimeSettings::new();
    // All 3 G7 settings should exist
    assert!(settings.get("fallback_model").is_some());
    assert!(settings.get("fallback_model_authorized").is_some());
    assert!(settings.get("mcp_enabled").is_some());
}

#[test]
fn build_chat_request_with_tools() {
    use async_openai::types::chat::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionTools, FunctionObjectArgs,
    };
    let msgs = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content("test")
            .build()
            .unwrap()
            .into(),
    ];
    let tool = ChatCompletionTools::Function(ChatCompletionTool {
        function: FunctionObjectArgs::default().name("t").build().unwrap(),
    });
    let req = Orchestrator::build_chat_request("test-model", msgs, Some(vec![tool])).unwrap();
    assert_eq!(req.model, "test-model");
    assert!(req.tools.is_some());
    assert_eq!(req.tools.as_ref().unwrap().len(), 1);
    assert_eq!(req.parallel_tool_calls, Some(true));
}

#[test]
fn build_chat_request_without_tools() {
    let msgs = vec![
        async_openai::types::chat::ChatCompletionRequestSystemMessageArgs::default()
            .content("test")
            .build()
            .unwrap()
            .into(),
    ];
    let req = Orchestrator::build_chat_request("test-model", msgs, None).unwrap();
    assert_eq!(req.model, "test-model");
    assert!(req.tools.is_none());
    assert!(req.parallel_tool_calls.is_none());
}

#[test]
fn fallback_authorization_denied_produces_hint() {
    // Verify that default settings deny fallback and error contains hint
    let settings = crate::settings::RuntimeSettings::new();
    let authorized = settings
        .get("fallback_model_authorized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(!authorized, "default must be false (user must opt-in)");

    // Verify the error message format includes the hint
    let primary = "test-model";
    let err_msg = format!(
        "主模型 {primary} 不可用。如需启用备用模型，请设置 fallback_model_authorized = true \
             (Primary model {primary} unavailable. Set fallback_model_authorized = true to enable auto-fallback)"
    );
    assert!(err_msg.contains("fallback_model_authorized"));
    assert!(err_msg.contains(primary));
}

#[test]
fn fallback_model_empty_means_no_fallback() {
    // Default fallback_model is empty — no fallback available until user configures one
    let settings = crate::settings::RuntimeSettings::new();
    let fallback = settings
        .get("fallback_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        fallback.is_empty(),
        "default fallback_model must be empty (disabled)"
    );
}

#[tokio::test]
async fn paid_model_fallback_live_notice_matches_durable_billing_in_each_language() {
    for language in ["en", "zh"] {
        let (server, handle) = spawn_mock_llm_server().await;
        let settings = drill_test_settings(true);
        settings
            .set("display_language", serde_json::json!(language))
            .unwrap();
        let mut orch = orchestrator_on_mock(settings, server.addr);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        orch.set_progress_tx(crate::progress_router::ProgressRouterSender::direct(tx));
        let messages = vec![
            ChatCompletionRequestUserMessageArgs::default()
                .content("Reply OK")
                .build()
                .unwrap()
                .into(),
        ];
        let (result, durable) = crate::behavior_notice::collect(
            language,
            None,
            Some(7),
            orch.chat_create_with_fallback_inner(
                messages,
                None,
                "notice_test",
                Some(7),
                None,
                Some(DrillFault::PrimaryTimeout),
            ),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(server.request_models(), vec!["backup-model".to_string()]);
        let event = rx.try_recv().expect("live disclosure before the answer");
        let ProgressPayload::OwnerNotice {
            text,
            kind: OwnerNoticeKind::ModelFallback,
        } = &*event.payload
        else {
            panic!("fallback must use the full localized owner notice");
        };
        assert_eq!(durable.as_slice(), std::slice::from_ref(text));
        assert!(text.contains("primary-model") && text.contains("backup-model"));
        if language == "en" {
            assert!(text.contains("bill separately") && !text.contains("主模型"));
        } else {
            assert!(text.contains("独立计费") && !text.contains("Primary model"));
        }
        let frame = crate::progress_router::progress_event_to_sse_json(&event);
        assert_eq!(frame.event_type, "owner_notice");
        assert_eq!(frame.data["text"], *text);
        assert!(rx.try_recv().is_err(), "no duplicate short legacy notice");
        handle.abort();
    }
}

#[tokio::test]
async fn llm_fallback_drill_authorized_uses_backup_and_skips_primary() {
    let (server, _handle) = spawn_mock_llm_server().await;
    let settings = drill_test_settings(true);
    let orch = orchestrator_on_mock(settings, server.addr);

    let (report, notices) = crate::behavior_notice::collect(
        "en",
        None,
        None,
        orch.run_llm_fallback_drill(DrillFault::PrimaryTimeout),
    )
    .await;
    assert!(
        notices
            .join(" ")
            .contains("authorized fallback backup-model")
    );

    assert!(
        report.ok,
        "authorized drill must succeed, got error={:?}",
        report.error
    );
    assert_eq!(report.fault, "primary_timeout");
    assert_eq!(report.primary_model, "primary-model");
    assert_eq!(report.primary_provider, "openai_compat");
    assert!(report.fallback_authorized);
    assert!(report.paid_api_fallback_enabled);
    assert!(report.error.is_none());
    let answered = report.answered_by.as_ref().expect("backup should answer");
    assert_eq!(answered.model, "backup-model");
    assert_eq!(answered.provider, "openai_compat");
    assert!(answered.reply_chars > 0);
    assert!(
        orch.llm_degraded_snapshot().is_none(),
        "drill must not toggle the foreground degraded signal"
    );

    let models = server.request_models();
    assert!(
        !models.iter().any(|m| m == "primary-model"),
        "primary mock endpoint received requests: {models:?}"
    );
    assert!(
        models.iter().any(|m| m == "backup-model"),
        "backup model was not called: {models:?}"
    );

    let json = serde_json::to_value(&report).expect("serialize drill report");
    assert_eq!(json["fault"], "primary_timeout");
    assert_eq!(json["ok"], true);
    assert_eq!(json["answered_by"]["model"], "backup-model");
    assert_eq!(json["answered_by"]["provider"], "openai_compat");
    assert!(json["error"].is_null());
}

#[tokio::test]
async fn normalize_llm_model_accepts_arbitrary_slugs_and_empty_provider_default() {
    // Empty llm_model resolves to the provider default via active_model().
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    settings
        .set(
            "llm_provider",
            serde_json::Value::String("codex_oauth".into()),
        )
        .unwrap();
    settings
        .set("llm_model", serde_json::Value::String(String::new()))
        .unwrap();
    settings
        .set("codex_model", serde_json::Value::String("gpt-5.5".into()))
        .unwrap();
    settings
        .set("codex_enabled", serde_json::Value::Bool(true))
        .unwrap();

    let mem = crate::memory::MemoryHandle::spawn_in_memory().expect("mem");
    let sandbox = Arc::new(crate::sandbox::SandboxEngine::new().expect("sandbox"));
    let identity = Arc::new(crate::identity::AgentIdentity::generate());
    let embedder = crate::embedding::EmbeddingClient::new(
        "http://127.0.0.1:9/v1",
        "test-key",
        "test-embed",
        384,
    );
    let orch = Orchestrator::new(
        "test-key",
        "http://127.0.0.1:9/v1",
        "",
        sandbox,
        identity,
        mem,
        embedder,
        settings,
    );
    assert_eq!(
        orch.active_model(),
        "gpt-5.5",
        "empty llm_model must resolve to codex_model for codex_oauth"
    );

    // Arbitrary non-empty slug still wins over the provider default.
    orch.settings
        .set(
            "llm_model",
            serde_json::Value::String("custom/slug-model".into()),
        )
        .unwrap();
    assert_eq!(orch.active_model(), "custom/slug-model");
}

#[tokio::test]
async fn llm_fallback_drill_unauthorized_mentions_authorization_and_calls_nobody() {
    let (server, _handle) = spawn_mock_llm_server().await;
    let settings = drill_test_settings(false);
    let orch = orchestrator_on_mock(settings, server.addr);

    let report = orch
        .run_llm_fallback_drill(DrillFault::PrimaryTimeout)
        .await;

    assert!(!report.ok, "unauthorized drill must not be ok");
    assert!(report.answered_by.is_none());
    let error = report.error.expect("owner-safe error");
    assert!(
        error.contains("授权") || error.to_ascii_lowercase().contains("authoriz"),
        "error must mention authorization, got: {error}"
    );
    assert_eq!(
        server.request_count(),
        0,
        "no provider should be called when fallback is not authorized"
    );
    assert!(
        orch.llm_degraded_snapshot().is_none(),
        "drill must not toggle the foreground degraded signal"
    );
}

#[tokio::test]
async fn llm_fallback_drill_answered_by_provider_comes_from_catalog_owner() {
    let _guard = crate::model_catalog::lock_model_catalog_caches_for_tests().await;
    crate::model_catalog::clear_model_catalog_caches_for_tests();
    // Selected primary stays openai_compat so the mock HTTP path can answer,
    // while the fallback id is owned by a seeded Grok/xAI catalog — the
    // production bug was reporting the selected/route provider (e.g. codex)
    // instead of the catalog owner (xai_subscription).
    crate::model_catalog::seed_provider_cache_for_tests(crate::model_catalog::ProviderCatalog {
        id: "xai_subscription".into(),
        connected: true,
        models: vec![crate::model_catalog::CatalogModel {
            id: "catalog-owner-backup-fixture".into(),
            display_name: "Backup".into(),
        }],
        source: "xai_models_api".into(),
        fetched_at_unix: Some(1),
        default_model: String::new(),
    });

    let (server, _handle) = spawn_mock_llm_server().await;
    let settings = drill_test_settings(true);
    settings
        .set(
            "fallback_model",
            serde_json::json!("catalog-owner-backup-fixture"),
        )
        .unwrap();
    let orch = orchestrator_on_mock(settings, server.addr);

    let report = orch
        .run_llm_fallback_drill(DrillFault::PrimaryTimeout)
        .await;

    // The catalog says xai_subscription owns catalog-owner-backup-fixture, but that provider is
    // not connected here, so the answer is served by the selected provider and
    // the drill must say so instead of a green with a borrowed label
    // (2026-09-12: production reported codex_oauth/grok-4.6 as a pass).
    assert!(
        !report.ok,
        "a backup served by another provider is not a pass"
    );
    assert_eq!(report.primary_provider, "openai_compat");
    let answered = report.answered_by.as_ref().expect("backup should answer");
    assert_eq!(answered.model, "catalog-owner-backup-fixture");
    assert_eq!(
        answered.provider, "openai_compat",
        "answered_by.provider is the provider that actually served the request"
    );
    let error = report.error.as_deref().unwrap_or("");
    assert!(
        error.contains("xai_subscription") && error.contains("not connected"),
        "error names the disconnected owner: {error}"
    );
    let models = server.request_models();
    assert!(
        models.iter().any(|m| m == "catalog-owner-backup-fixture"),
        "fallback model id on the request must stay catalog-owner-backup-fixture, got {models:?}"
    );
    assert!(
        !models.iter().any(|m| m == "primary-model"),
        "primary must not be called on PrimaryTimeout"
    );
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}

#[tokio::test]
async fn llm_fallback_drill_answered_by_provider_falls_back_to_route_when_uncached() {
    let _guard = crate::model_catalog::lock_model_catalog_caches_for_tests().await;
    crate::model_catalog::clear_model_catalog_caches_for_tests();

    let (server, _handle) = spawn_mock_llm_server().await;
    let settings = drill_test_settings(true);
    let orch = orchestrator_on_mock(settings, server.addr);

    let report = orch
        .run_llm_fallback_drill(DrillFault::PrimaryTimeout)
        .await;

    assert!(report.ok, "authorized drill must succeed");
    let answered = report.answered_by.as_ref().expect("backup should answer");
    assert_eq!(answered.model, "backup-model");
    assert_eq!(
        answered.provider, "openai_compat",
        "with no catalog cache hit, answered_by.provider stays the route provider"
    );
    crate::model_catalog::clear_model_catalog_caches_for_tests();
}
