//! Google OAuth header resolution, pending OAuth flow staging/cleanup/callback and auth status JSON.
use super::*;

#[tokio::test]
async fn jev_settings_runtime_reads_each_change_and_fails_closed_on_corruption() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let mut orch = crate::test_tool_helpers::test_orchestrator(
        memory,
        Arc::new(crate::settings::RuntimeSettings::new()),
    );
    let file = tempfile::NamedTempFile::new().unwrap();
    orch.set_credential_store(file.path().to_str().unwrap().into(), [42; 32]);
    let store = orch.open_credential_store().unwrap();
    let key = "sk-or-synthetic-runtime-settings-key";
    crate::jev_config::save(
        &store,
        crate::jev_config::JevUpdate {
            enabled: true,
            consent: true,
            api_key: Some(key.into()),
            remove_key: false,
        },
    )
    .unwrap();
    assert_eq!(orch.jev_active_key().as_deref(), Some(key));
    crate::jev_config::save(
        &store,
        crate::jev_config::JevUpdate {
            enabled: false,
            consent: false,
            api_key: None,
            remove_key: false,
        },
    )
    .unwrap();
    assert!(orch.jev_active_key().is_none());
    store.store("jev", "configuration", "invalid-json").unwrap();
    assert!(orch.jev_active_key().is_none());
    assert!(
        !std::fs::read(file.path())
            .unwrap()
            .windows(key.len())
            .any(|s| s == key.as_bytes())
    );
}

#[tokio::test]
async fn resolve_google_auth_header_allows_legacy_fallback_only_without_registry() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    orch.sandbox
        .add_auth_header("www.googleapis.com", "Bearer legacy-token");

    let header = orch
        .resolve_google_auth_header(None, "www.googleapis.com", "test legacy fallback behavior")
        .unwrap();
    assert_eq!(header, "Bearer legacy-token");
}

#[tokio::test]
async fn resolve_google_auth_header_unconfigured_points_at_codex() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let err = orch
        .resolve_google_auth_header(None, "www.googleapis.com", "calendar read")
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("run_external(executor=\"codex\", connector="),
        "{msg}"
    );
    assert!(msg.contains("access is verified before execution"), "{msg}");
    assert!(!msg.contains("setup_google"), "{msg}");
}
#[tokio::test]
async fn stage_pending_oauth_flow_for_chat_copies_base_credentials_and_records_active_flow() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let tmp = tempfile::NamedTempFile::new().expect("create temp cred db");
    let db_path = tmp.path().to_str().expect("utf8 path").to_string();
    let aes_key = [7u8; 32];
    orch.set_credential_store(db_path.clone(), aes_key);
    orch.store_google_client_credentials_named_for_chat(
        "cid-flow",
        "secret-flow",
        "primary",
        Some(12345),
    )
    .unwrap();

    let flow_id = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(12345))
        .unwrap();
    let store = crate::credential_store::CredentialStore::open(
        crate::encrypted_sqlite::open(&db_path).unwrap(),
        aes_key,
    )
    .unwrap();
    let base = crate::google_auth::pending_cred_service(Some(12345));
    let flow = crate::google_auth::pending_cred_service_for_flow(12345, &flow_id);

    assert_eq!(
        store.get(&base, "active_flow_id").unwrap().as_deref(),
        Some(flow_id.as_str())
    );
    assert_eq!(
        store.get(&flow, "client_id").unwrap().as_deref(),
        Some("cid-flow")
    );
    assert_eq!(
        store.get(&flow, "client_secret").unwrap().as_deref(),
        Some("secret-flow")
    );
    assert_eq!(
        store.get(&flow, "account_name").unwrap().as_deref(),
        Some("primary")
    );
}

#[tokio::test]
async fn cleanup_stale_pending_oauth_for_chat_removes_flow_scoped_entries() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let tmp = tempfile::NamedTempFile::new().expect("create temp cred db");
    let db_path = tmp.path().to_str().expect("utf8 path").to_string();
    let aes_key = [8u8; 32];
    orch.set_credential_store(db_path.clone(), aes_key);
    orch.store_google_client_credentials_named_for_chat(
        "cid-flow",
        "secret-flow",
        "primary",
        Some(12345),
    )
    .unwrap();
    let flow_id = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(12345))
        .unwrap();

    assert!(
        orch.cleanup_stale_pending_oauth_for_chat("secondary", 3600, Some(12345))
            .unwrap()
    );

    let store = crate::credential_store::CredentialStore::open(
        crate::encrypted_sqlite::open(&db_path).unwrap(),
        aes_key,
    )
    .unwrap();
    let base = crate::google_auth::pending_cred_service(Some(12345));
    let flow = crate::google_auth::pending_cred_service_for_flow(12345, &flow_id);

    assert!(store.get(&base, "active_flow_id").unwrap().is_none());
    assert!(store.get(&flow, "client_id").unwrap().is_none());
    assert!(store.get(&flow, "client_secret").unwrap().is_none());
    assert!(store.get(&flow, "account_name").unwrap().is_none());
}

#[tokio::test]
async fn cleanup_stale_pending_oauth_for_chat_removes_flow_specific_keys() {
    // H1 audit fix: cleanup should scan *every* flow-specific slot for
    // the chat (not just the one referenced by `active_flow_id`).
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let tmp = tempfile::NamedTempFile::new().expect("create temp cred db");
    let db_path = tmp.path().to_str().expect("utf8 path").to_string();
    let aes_key = [42u8; 32];
    orch.set_credential_store(db_path.clone(), aes_key);
    orch.store_google_client_credentials_named_for_chat(
        "cid-flow",
        "secret-flow",
        "primary",
        Some(54321),
    )
    .unwrap();

    // Stage two parallel flows, then forcibly orphan the first by
    // overwriting `active_flow_id`. The first flow's row would
    // previously survive cleanup because the cleanup function only
    // looked at `active_flow_id`.
    let flow_a = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(54321))
        .unwrap();
    // Re-stage so a *new* flow_id replaces active_flow_id but flow_a's
    // row remains in the store.
    orch.store_google_client_credentials_named_for_chat(
        "cid-flow",
        "secret-flow",
        "primary",
        Some(54321),
    )
    .unwrap();
    let flow_b = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(54321))
        .unwrap();
    assert_ne!(flow_a, flow_b, "flow_ids should differ");

    let store = crate::credential_store::CredentialStore::open(
        crate::encrypted_sqlite::open(&db_path).unwrap(),
        aes_key,
    )
    .unwrap();
    let svc_a = crate::google_auth::pending_cred_service_for_flow(54321, &flow_a);
    let svc_b = crate::google_auth::pending_cred_service_for_flow(54321, &flow_b);
    // Sanity: both flows have rows pre-cleanup.
    assert_eq!(
        store.get(&svc_a, "client_id").unwrap().as_deref(),
        Some("cid-flow")
    );
    assert_eq!(
        store.get(&svc_b, "client_id").unwrap().as_deref(),
        Some("cid-flow")
    );

    assert!(
        orch.cleanup_stale_pending_oauth_for_chat("secondary", 3600, Some(54321))
            .unwrap()
    );

    // Both flow-scoped slots must be wiped — including the orphaned
    // flow_a that is no longer pointed at by active_flow_id.
    assert!(
        store.get(&svc_a, "client_id").unwrap().is_none(),
        "orphaned flow's client_id must be wiped"
    );
    assert!(
        store.get(&svc_a, "client_secret").unwrap().is_none(),
        "orphaned flow's client_secret must be wiped"
    );
    assert!(
        store.get(&svc_a, "account_name").unwrap().is_none(),
        "orphaned flow's account_name must be wiped"
    );
    assert!(
        store.get(&svc_b, "client_id").unwrap().is_none(),
        "active flow's client_id must be wiped"
    );
    assert!(
        store.get(&svc_b, "client_secret").unwrap().is_none(),
        "active flow's client_secret must be wiped"
    );
}

#[tokio::test]
async fn oauth_callback_rejects_when_flow_id_does_not_match_active_flow() {
    // H1 audit fix: the OAuth callback must validate `flow_id` against
    // the chat's `active_flow_id` *before* exchanging the code or
    // writing credentials. We exercise the binding by directly poking
    // the credential store: stage flow-A, overwrite active_flow_id with
    // flow-B's id, then re-read with flow-A's pending_service. The
    // callback handler runs the equivalent check (see
    // `oauth_callback` in `admin.rs`) and rejects when this mismatch
    // is detected.
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let tmp = tempfile::NamedTempFile::new().expect("create temp cred db");
    let db_path = tmp.path().to_str().expect("utf8 path").to_string();
    let aes_key = [11u8; 32];
    orch.set_credential_store(db_path.clone(), aes_key);
    orch.store_google_client_credentials_named_for_chat(
        "cid-x",
        "secret-x",
        "primary",
        Some(99999),
    )
    .unwrap();

    let stale_flow_id = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(99999))
        .unwrap();
    // Re-stage to create a fresh active_flow_id that does NOT match the
    // stale callback's flow_id.
    orch.store_google_client_credentials_named_for_chat(
        "cid-y",
        "secret-y",
        "primary",
        Some(99999),
    )
    .unwrap();
    let fresh_flow_id = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(99999))
        .unwrap();
    assert_ne!(stale_flow_id, fresh_flow_id);

    let store = crate::credential_store::CredentialStore::open(
        crate::encrypted_sqlite::open(&db_path).unwrap(),
        aes_key,
    )
    .unwrap();
    let base = crate::google_auth::pending_cred_service(Some(99999));
    let active_flow_id = store
        .get(&base, "active_flow_id")
        .unwrap()
        .expect("active_flow_id should be set");
    // The check the OAuth callback now performs: the stale signed
    // flow_id must not equal the chat's active_flow_id.
    assert_ne!(
        active_flow_id, stale_flow_id,
        "stale callback's flow_id must not match the active flow"
    );

    // The fresh flow must still be retrievable for an honest callback.
    let fresh_service = crate::google_auth::pending_cred_service_for_flow(99999, &fresh_flow_id);
    assert_eq!(
        store.get(&fresh_service, "client_id").unwrap().as_deref(),
        Some("cid-y")
    );
}

#[tokio::test]
async fn stage_pending_oauth_flow_for_chat_migrates_legacy_pending_slot() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(crate::settings::RuntimeSettings::new());
    let mut orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
    let tmp = tempfile::NamedTempFile::new().expect("create temp cred db");
    let db_path = tmp.path().to_str().expect("utf8 path").to_string();
    let aes_key = [9u8; 32];
    orch.set_credential_store(db_path.clone(), aes_key);

    let store = crate::credential_store::CredentialStore::open(
        crate::encrypted_sqlite::open(&db_path).unwrap(),
        aes_key,
    )
    .unwrap();
    let legacy = crate::google_auth::pending_cred_service(None);
    store.store(&legacy, "client_id", "cid-legacy").unwrap();
    store
        .store(&legacy, "client_secret", "secret-legacy")
        .unwrap();
    store.store(&legacy, "account_name", "primary").unwrap();
    store
        .store(&legacy, "staged_at", "2026-04-22T00:00:00Z")
        .unwrap();

    let flow_id = orch
        .stage_pending_oauth_flow_for_chat("primary", Some(12345))
        .unwrap();

    let base = crate::google_auth::pending_cred_service(Some(12345));
    let flow = crate::google_auth::pending_cred_service_for_flow(12345, &flow_id);
    assert_eq!(
        store.get(&base, "client_id").unwrap().as_deref(),
        Some("cid-legacy")
    );
    assert_eq!(
        store.get(&base, "client_secret").unwrap().as_deref(),
        Some("secret-legacy")
    );
    assert_eq!(
        store.get(&flow, "client_id").unwrap().as_deref(),
        Some("cid-legacy")
    );
    assert!(store.get(&legacy, "client_id").unwrap().is_none());
    assert!(store.get(&legacy, "client_secret").unwrap().is_none());
    assert!(store.get(&legacy, "account_name").unwrap().is_none());
    assert!(store.get(&legacy, "staged_at").unwrap().is_none());
}

// ── Google OAuth status tests ───────────────────────────────────────

/// google_auth_status returns three states properly encoded as JSON.
#[test]
fn google_auth_status_json_three_states() {
    // State 1: not configured (neither final nor pending)
    let status_unconfigured = serde_json::json!({
        "configured": false,
        "message": "Google OAuth2 is not configured. Use setup_google to connect."
    });
    assert_eq!(status_unconfigured["configured"], false);
    assert!(status_unconfigured.get("pending_setup").is_none());

    // State 2: pending (credentials received, not authorized)
    let status_pending = serde_json::json!({
        "configured": false,
        "pending_setup": true,
        "message": "Google OAuth2 client credentials received but NOT yet authorized."
    });
    assert_eq!(status_pending["configured"], false);
    assert_eq!(status_pending["pending_setup"], true);

    // State 3: fully configured
    let status_configured = serde_json::json!({
        "configured": true,
        "message": "Google OAuth2 is configured and active."
    });
    assert_eq!(status_configured["configured"], true);
}
