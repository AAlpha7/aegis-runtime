//! Real SQLite failures; no provider, production state or credentials.
use super::*;
use serde_json::json;

fn snapshot(settings: &RuntimeSettings) -> HashMap<String, serde_json::Value> {
    settings
        .all()
        .into_iter()
        .map(|s| (s.key, s.value))
        .collect()
}

async fn fixture() -> (tempfile::TempDir, Arc<AdminState>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.db");
    let memory = MemoryHandle::spawn(&path).unwrap();
    for (key, value) in [
        ("llm_provider", json!("openrouter")),
        ("llm_model", json!("old-model")),
        ("fallback_model", json!("old-backup")),
        ("fallback_model_authorized", json!(true)),
        ("paid_api_fallback_enabled", json!(true)),
        ("web_search_provider", json!("openrouter")),
        ("web_search_model", json!("old-search")),
    ] {
        memory.store_setting(key, value).await.unwrap();
    }
    let state = webapp_test_state_with_memory_and_db(memory, Some(path.to_string_lossy().into()));
    state
        .settings
        .load_from_map(state.memory.load_all_settings().await.unwrap());
    (dir, state)
}

fn reject_key(state: &AdminState, key: &str, operation: &str) {
    assert!(matches!(operation, "INSERT" | "DELETE"));
    let row = if operation == "INSERT" { "NEW" } else { "OLD" };
    // Only hardcoded synthetic setting names enter this fixture DDL.
    let conn = rusqlite::Connection::open(state.memory.db_path()).unwrap();
    conn.execute_batch(&format!("CREATE TRIGGER reject_setting BEFORE {operation} ON user_settings WHEN {row}.key = '{key}' BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;")).unwrap();
}

fn remove_failure(state: &AdminState) {
    rusqlite::Connection::open(state.memory.db_path())
        .unwrap()
        .execute_batch("DROP TRIGGER reject_setting")
        .unwrap();
}

#[tokio::test]
async fn settings_persistence_commit_failure_preserves_entire_configuration() {
    let (_dir, state) = fixture().await;
    let live = snapshot(&state.settings);
    let disk = state.memory.load_all_settings().await.unwrap();
    // All INSERTs succeed; a deferred FK fails only at COMMIT. The memory
    // actor enables foreign_keys on its connection, as in production.
    let conn = rusqlite::Connection::open(state.memory.db_path()).unwrap();
    conn.execute_batch("CREATE TABLE fault_parent (id INTEGER PRIMARY KEY);
        CREATE TABLE fault_child (id INTEGER REFERENCES fault_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER fail_commit AFTER INSERT ON user_settings WHEN NEW.key = 'llm_model'
        BEGIN INSERT INTO fault_child VALUES (999); END;").unwrap();
    assert_eq!(
        save(
            state.clone(),
            "llm-provider",
            json!({"llm_provider":"codex_oauth", "llm_model":"candidate-model"})
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(snapshot(&state.settings), live);
    assert_eq!(state.memory.load_all_settings().await.unwrap(), disk);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM fault_child", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    state.memory.shutdown().await.unwrap();
}

async fn save(state: Arc<AdminState>, path: &str, mut body: serde_json::Value) -> StatusCode {
    let (cookie, csrf) =
        create_test_web_console_session_with_csrf(state.db_path.as_deref().unwrap());
    body["schema_version"] = json!(1);
    body["idempotency_key"] = json!("synthetic-settings-write");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/webapp/vnext/settings/{path}"))
        .header("Cookie", format!("__Host-aegis_session={cookie}"))
        .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    settings_router_with_successful_probe(state)
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn settings_persistence_quota_failure_preserves_cache_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("settings.db");
    let seed = MemoryHandle::spawn(&db).unwrap();
    seed.store_setting("paid_api_fallback_enabled", json!(true))
        .await
        .unwrap();
    seed.shutdown().await.unwrap();
    let mut state = test_state();
    Arc::get_mut(&mut state).unwrap().memory = MemoryHandle::spawn_with_limit(&db, 1).unwrap();
    state
        .settings
        .load_from_map(state.memory.load_all_settings().await.unwrap());
    let result = persist_runtime_setting(&state, "paid_api_fallback_enabled", json!(false)).await;
    assert!(result.is_err());
    let live = state.settings.get("paid_api_fallback_enabled");
    state.memory.shutdown().await.unwrap();
    let restarted = MemoryHandle::spawn(&db).unwrap();
    let disk = restarted.load_all_settings().await.unwrap();
    restarted.shutdown().await.unwrap();
    assert_eq!(disk["paid_api_fallback_enabled"], json!(true));
    assert_eq!(
        live,
        Some(json!(true)),
        "failed save must not change live configuration"
    );
}

#[tokio::test]
async fn settings_persistence_later_write_failure_rolls_back_complete_http_groups() {
    for (path, rejected, body) in [
        (
            "llm-provider",
            "llm_model",
            json!({"llm_provider":"codex_oauth","llm_model":"candidate-model"}),
        ),
        (
            "web-search",
            "web_search_model",
            json!({"web_search_provider":"codex_oauth","web_search_model":""}),
        ),
        (
            "fallback-model",
            "fallback_model_authorized",
            json!({"model":""}),
        ),
    ] {
        let (_dir, state) = fixture().await;
        let live = snapshot(&state.settings);
        let disk = state.memory.load_all_settings().await.unwrap();
        reject_key(&state, rejected, "INSERT");
        assert_eq!(
            save(state.clone(), path, body.clone()).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path}"
        );
        assert_eq!(snapshot(&state.settings), live, "{path}: cache");
        assert_eq!(
            state.memory.load_all_settings().await.unwrap(),
            disk,
            "{path}: rollback"
        );
        remove_failure(&state);
        assert_eq!(
            save(state.clone(), path, body).await,
            StatusCode::OK,
            "{path}: retry"
        );
        let expected = state.memory.load_all_settings().await.unwrap();
        let db = state.memory.db_path().to_owned();
        state.memory.shutdown().await.unwrap();
        let restarted = MemoryHandle::spawn(FsPath::new(&db)).unwrap();
        assert_eq!(restarted.load_all_settings().await.unwrap(), expected);
        let restored = RuntimeSettings::new();
        restored.load_from_map(expected);
        assert_eq!(
            snapshot(&restored),
            snapshot(&state.settings),
            "{path}: restart"
        );
        restarted.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn settings_persistence_subscription_selection_has_no_partial_activation() {
    let (_dir, state) = fixture().await;
    let before = snapshot(&state.settings);
    let disk = state.memory.load_all_settings().await.unwrap();
    reject_key(&state, "paid_api_fallback_enabled", "INSERT");
    let result = crate::call_validation::with_test_probe(
        Arc::new(|_| Ok(())),
        select_webapp_chat_provider(&state, WebappChatProviderSelection::Codex),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(snapshot(&state.settings), before);
    assert_eq!(state.memory.load_all_settings().await.unwrap(), disk);
    assert_eq!(state.settings.get_if_stored("codex_enabled"), None);
    state.memory.shutdown().await.unwrap();
}

#[tokio::test]
async fn settings_persistence_paid_disable_and_reset_failures_never_report_success() {
    let (_dir, state) = fixture().await;
    reject_key(&state, "paid_api_fallback_enabled", "INSERT");
    assert_eq!(
        save(state.clone(), "paid-api-fallback", json!({"enabled":false})).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let response = admin_router(state.clone())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/paid_api_fallback_enabled")
                .header("Authorization", "Bearer test-secret-token")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"value":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        state.settings.get("paid_api_fallback_enabled"),
        Some(json!(true))
    );
    assert_eq!(
        state.memory.load_all_settings().await.unwrap()["paid_api_fallback_enabled"],
        json!(true)
    );
    remove_failure(&state);
    reject_key(&state, "paid_api_fallback_enabled", "DELETE");
    let response = admin_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/settings/paid_api_fallback_enabled")
                .header("Authorization", "Bearer test-secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        state.settings.get_if_stored("paid_api_fallback_enabled"),
        Some(json!(true))
    );
    remove_failure(&state);
    assert_eq!(
        save(state.clone(), "paid-api-fallback", json!({"enabled":false})).await,
        StatusCode::OK
    );
    assert_eq!(
        state.memory.load_all_settings().await.unwrap()["paid_api_fallback_enabled"],
        json!(false)
    );
    state.memory.shutdown().await.unwrap();
}
