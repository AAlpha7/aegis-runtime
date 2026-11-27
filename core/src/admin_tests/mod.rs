use super::*;
mod embedding_persistence;
mod settings_persistence;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use tokio::task::JoinHandle;
use tower::util::ServiceExt;
use wasmtime::Engine;

static TEST_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    Engine::new(&config).expect("test engine init")
});

/// Serializes tests that mutate process env (voice stub, Google OAuth staging).
/// cargo runs test files in parallel.
pub(crate) static WEBAPP_DEBUG_ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Refcounted so concurrent tests with OVERLAPPING (non-nested) guard lifetimes
/// don't prematurely disable snapshots. The prior swap/restore-`previous` design
/// assumed LIFO nesting, but parallel tests overlap, so one guard's Drop would
/// restore `false` while another lock-holding test was still mid-request (flaky
/// 404 from require_webapp_debug_snapshots_enabled). Snapshots stay enabled
/// while ANY guard is alive (2026-06-21).
struct TestWebappDebugSnapshotsGuard;

impl TestWebappDebugSnapshotsGuard {
    fn enable() -> Self {
        WEBAPP_DEBUG_SNAPSHOTS_TEST_REFCOUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for TestWebappDebugSnapshotsGuard {
    fn drop(&mut self) {
        WEBAPP_DEBUG_SNAPSHOTS_TEST_REFCOUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn test_engine() -> Engine {
    TEST_ENGINE.clone()
}

fn test_state_with_token(token: Option<&str>) -> Arc<AdminState> {
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory: MemoryHandle::spawn_in_memory().unwrap(),
        api_token: token.map(|t| t.to_string()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: None,
        admin_chat_id: None,
        orchestrator: None,
        event_tx: None,
        db_path: None,
        progress_router: None,
        local_login_token: None,
    })
}

fn test_state() -> Arc<AdminState> {
    test_state_with_token(Some("test-secret-token"))
}

fn settings_router_with_successful_probe(state: Arc<AdminState>) -> axum::Router {
    oauth_callback_router(state).layer(axum::middleware::from_fn(
        |req: axum::extract::Request, next: axum::middleware::Next| async move {
            crate::call_validation::with_test_probe(Arc::new(|_| Ok(())), next.run(req)).await
        },
    ))
}
fn with_test_runtime<T>(f: impl FnOnce() -> T) -> T {
    let rt = tokio::runtime::Runtime::new().expect("admin test runtime");
    let _guard = rt.enter();
    let result = f();
    drop(_guard);
    result
}
/// The web-console principal `extract_web_console_auth` reconstructs from a
/// `create_test_web_console_session*` cookie. `log_subject()` (and therefore
/// the singleflight key) only depends on the email, so a synthetic principal
/// with the same email yields the same key the live handler computes.
fn web_console_command_status_principal() -> WebConsolePrincipal {
    WebConsolePrincipal::WebConsole(crate::web_console_auth::WebConsoleSession {
        session_id_hash: "session".into(),
        google_sub: "google-sub-test".into(),
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
    })
}

fn test_google_manager(refresh_token: &str) -> Arc<GoogleAuthManager> {
    Arc::new(GoogleAuthManager::new(
        "test-client".into(),
        "test-secret".into(),
        refresh_token.into(),
        vec!["https://www.googleapis.com/auth/calendar.events".into()],
    ))
}

fn test_state_with_registry() -> Arc<AdminState> {
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory: MemoryHandle::spawn_in_memory().unwrap(),
        api_token: Some("test-secret-token".into()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: None,
        admin_chat_id: None,
        orchestrator: None,
        event_tx: None,
        db_path: None,
        progress_router: None,
        local_login_token: None,
    })
}

fn test_state_with_credential_store(db_path: String, aes_key: [u8; 32]) -> Arc<AdminState> {
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory: MemoryHandle::spawn_in_memory().unwrap(),
        api_token: Some("test-secret-token".into()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: Some(db_path),
        credential_aes_key: Some(aes_key),
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: None,
        admin_chat_id: None,
        orchestrator: None,
        event_tx: None,
        db_path: None,
        progress_router: None,
        local_login_token: None,
    })
}

fn store_google_credentials(
    store: &CredentialStore,
    service: &str,
    client_id: &str,
) -> anyhow::Result<()> {
    store.store(service, "client_id", client_id)?;
    store.store(service, "client_secret", "client-secret")?;
    store.store(service, "refresh_token", "refresh-token")?;
    store.store(service, "scopes", "scope")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Slice 5: multimodal upload + command-attachment tests.
// ---------------------------------------------------------------------------

/// A minimal valid PNG magic header (8-byte signature) + a couple trailing bytes.
fn sample_png_header() -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // start of IHDR length
    bytes
}

/// A minimal PDF magic header.
fn sample_pdf_header() -> Vec<u8> {
    b"%PDF-1.7\n".to_vec()
}

// ── WebApp SDUI tests ───────────────────────────────────────────────

fn webapp_test_state() -> Arc<AdminState> {
    webapp_test_state_with_db(None)
}

fn webapp_test_state_with_db(db_path: Option<String>) -> Arc<AdminState> {
    webapp_test_state_with_memory_and_db(MemoryHandle::spawn_in_memory().unwrap(), db_path)
}

fn webapp_test_state_with_db_and_mfa_key(db_path: String, aes_key: [u8; 32]) -> Arc<AdminState> {
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory: MemoryHandle::spawn_in_memory().unwrap(),
        api_token: Some("test-secret-token".into()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: None,
        credential_aes_key: Some(aes_key),
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: Some("123456:TEST-TOKEN".into()),
        admin_chat_id: Some(12345678),
        orchestrator: None,
        event_tx: None,
        db_path: Some(db_path),
        progress_router: None,
        local_login_token: None,
    })
}

fn webapp_test_state_with_memory_and_db(
    memory: MemoryHandle,
    db_path: Option<String>,
) -> Arc<AdminState> {
    webapp_test_state_with_memory_db_and_local_login(memory, db_path, None)
}

/// Owner-console state with the local login link enabled (`Some(token)`) or
/// disabled (`None`) — the two shapes `/auth/login/local` distinguishes.
fn webapp_test_state_with_local_login(
    db_path: Option<String>,
    local_login_token: Option<&str>,
) -> Arc<AdminState> {
    webapp_test_state_with_memory_db_and_local_login(
        MemoryHandle::spawn_in_memory().unwrap(),
        db_path,
        local_login_token.map(str::to_string),
    )
}

fn webapp_test_state_with_memory_db_and_local_login(
    memory: MemoryHandle,
    db_path: Option<String>,
    local_login_token: Option<String>,
) -> Arc<AdminState> {
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory,
        api_token: Some("test-secret-token".into()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: Some("123456:TEST-TOKEN".into()),
        admin_chat_id: Some(12345678),
        orchestrator: None,
        event_tx: None,
        db_path,
        progress_router: None,
        local_login_token,
    })
}

fn webapp_test_state_with_orchestrator(
    memory: MemoryHandle,
    db_path: Option<String>,
) -> Arc<AdminState> {
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("llm_model", serde_json::Value::String("mock-model".into()))
        .expect("set mock llm model");
    settings
        .set(
            "llm_provider",
            serde_json::Value::String("openai_compat".into()),
        )
        .expect("set mock llm provider");
    let orchestrator = Arc::new(crate::test_tool_helpers::test_orchestrator(
        memory.clone(),
        settings.clone(),
    ));
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory,
        api_token: Some("test-secret-token".into()),
        settings,
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: Some("123456:TEST-TOKEN".into()),
        admin_chat_id: Some(12345678),
        orchestrator: Some(orchestrator),
        event_tx: None,
        db_path,
        progress_router: None,
        local_login_token: None,
    })
}

struct WebappMockRuntimeState {
    chat_responses: Mutex<Vec<String>>,
    chat_requests: Mutex<Vec<String>>,
    chat_call_count: AtomicUsize,
    embed_call_count: AtomicUsize,
}

impl WebappMockRuntimeState {
    fn new(reply: &str) -> Self {
        Self::new_with_responses(vec![webapp_mock_text_response(reply)])
    }

    fn new_with_responses(responses: Vec<String>) -> Self {
        Self {
            chat_responses: Mutex::new(responses),
            chat_requests: Mutex::new(Vec::new()),
            chat_call_count: AtomicUsize::new(0),
            embed_call_count: AtomicUsize::new(0),
        }
    }
}

async fn spawn_webapp_mock_runtime(
    state: Arc<WebappMockRuntimeState>,
) -> (SocketAddr, JoinHandle<()>) {
    async fn handle_chat(
        axum::extract::State(state): axum::extract::State<Arc<WebappMockRuntimeState>>,
        body: String,
    ) -> axum::response::Response {
        state.chat_call_count.fetch_add(1, Ordering::SeqCst);
        let auxiliary_response = webapp_mock_auxiliary_chat_response(&body);
        state.chat_requests.lock().unwrap().push(body);
        let response = if let Some(auxiliary_response) = auxiliary_response {
            auxiliary_response
        } else {
            let mut queue = state.chat_responses.lock().unwrap();
            if queue.is_empty() {
                webapp_mock_text_response("(fallback - response queue exhausted)")
            } else {
                queue.remove(0)
            }
        };
        axum::response::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(response))
            .unwrap()
    }

    async fn handle_embed(
        axum::extract::State(state): axum::extract::State<Arc<WebappMockRuntimeState>>,
        _body: String,
    ) -> axum::response::Response {
        state.embed_call_count.fetch_add(1, Ordering::SeqCst);
        let embedding: Vec<f32> = (0..384).map(|i| i as f32 / 384.0).collect();
        let body = serde_json::json!({
            "object": "list",
            "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
            "model": "mock-embed-model",
            "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.0},
            "provider": "Mock",
            "id": "gen-emb-mock"
        });
        axum::response::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    let app = axum::Router::new()
        .route("/v1/chat/completions", axum::routing::post(handle_chat))
        .route("/v1/embeddings", axum::routing::post(handle_embed))
        // Real providers accept multi-megabyte image data URLs; axum's default
        // 2 MB extractor limit would turn a photo turn into a 413 here.
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

fn webapp_mock_auxiliary_chat_response(body: &str) -> Option<String> {
    if body.contains("Translate the following text to English. Output ONLY the translation") {
        return Some(webapp_mock_text_response("check mail"));
    }
    None
}

fn webapp_mock_text_response(content: &str) -> String {
    serde_json::json!({
        "id": "chatcmpl-webapp-runtime-test",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            },
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

fn webapp_mock_tool_call_response(
    call_id: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> String {
    serde_json::json!({
        "id": "chatcmpl-webapp-runtime-tool-test",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": [],
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args.to_string()
                    }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "completed"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

fn webapp_test_state_with_mock_runtime(
    memory: MemoryHandle,
    db_path: String,
    addr: SocketAddr,
) -> Arc<AdminState> {
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("llm_model", serde_json::Value::String("mock-model".into()))
        .expect("set mock llm model");
    settings
        .set(
            "llm_provider",
            serde_json::Value::String("openai_compat".into()),
        )
        .expect("set mock llm provider");
    let api_base = format!("http://{addr}/v1");
    let embedder =
        crate::embedding::EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", 384);
    let orchestrator = Arc::new(crate::orchestrator::Orchestrator::new(
        "mock-key",
        &api_base,
        "mock-model",
        Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        Arc::new(crate::identity::AgentIdentity::generate()),
        memory.clone(),
        embedder,
        settings.clone(),
    ));
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory,
        api_token: Some("test-secret-token".into()),
        settings,
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: Some("123456:TEST-TOKEN".into()),
        admin_chat_id: Some(12345678),
        orchestrator: Some(orchestrator),
        event_tx: None,
        db_path: Some(db_path),
        progress_router: None,
        local_login_token: None,
    })
}

fn create_test_web_console_session(db_path: &str) -> String {
    let config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
        "GOOGLE_LOGIN_CLIENT_ID" => Some("client".into()),
        "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret".into()),
        "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let now = chrono::Utc::now().timestamp() as u64;
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    crate::web_console_auth::create_session(
        &conn,
        &config,
        "google-sub-test",
        "L@example.com",
        true,
        now,
    )
    .unwrap()
    .cookie_token
}

fn create_test_local_login_session(db_path: &str) -> String {
    let config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let now = chrono::Utc::now().timestamp() as u64;
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    crate::web_console_auth::create_local_session(&conn, &config, now)
        .unwrap()
        .cookie_token
}

fn create_test_web_console_google_fresh_session(db_path: &str) -> String {
    let config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "GOOGLE_LOGIN_ENABLED" => Some("true".into()),
        "GOOGLE_LOGIN_CLIENT_ID" => Some("client".into()),
        "GOOGLE_LOGIN_CLIENT_SECRET" => Some("secret".into()),
        "GOOGLE_LOGIN_ALLOWED_EMAILS" => Some("L@example.com".into()),
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let now = chrono::Utc::now().timestamp() as u64;
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    crate::web_console_auth::create_session_with_mfa_evidence(
        &conn,
        &config,
        "google-sub-test",
        "L@example.com",
        true,
        now,
        Some(crate::web_console_auth::SessionMfaEvidence {
            level: crate::web_console_auth::MfaLevel::GoogleFresh,
            verified_at: now,
            method: "google_auth_time".to_string(),
        }),
    )
    .unwrap()
    .cookie_token
}

fn create_test_local_login_session_with_csrf(db_path: &str) -> (String, String) {
    let config = crate::web_console_auth::WebConsoleLoginConfig::from_lookup(|key| match key {
        "SESSION_COOKIE_SECRET" => Some("cookie-secret".into()),
        _ => None,
    });
    let now = chrono::Utc::now().timestamp() as u64;
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    let cookie_token = crate::web_console_auth::create_local_session(&conn, &config, now)
        .unwrap()
        .cookie_token;
    let session =
        crate::web_console_auth::validate_session(&conn, &cookie_token, now, 30 * 24 * 60 * 60)
            .unwrap()
            .unwrap();
    let csrf_token = crate::web_console_auth::csrf_token_for_session(&session);
    (cookie_token, csrf_token)
}

fn create_test_web_console_session_with_csrf(db_path: &str) -> (String, String) {
    let cookie_token = create_test_web_console_session(db_path);
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    let session = crate::web_console_auth::validate_session(
        &conn,
        &cookie_token,
        chrono::Utc::now().timestamp() as u64,
        30 * 24 * 60 * 60,
    )
    .unwrap()
    .unwrap();
    let csrf_token = crate::web_console_auth::csrf_token_for_session(&session);
    (cookie_token, csrf_token)
}

fn mark_test_web_console_session_mfa_verified(db_path: &str) {
    mark_test_web_console_session_mfa_verified_at(db_path, chrono::Utc::now().timestamp() as u64);
}

fn mark_test_web_console_session_mfa_verified_at(db_path: &str, verified_at: u64) {
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    conn.execute(
        r#"
        UPDATE web_console_sessions
        SET mfa_level = 'webauthn',
            mfa_verified_at = ?1,
            mfa_method = 'test_webauthn'
        "#,
        rusqlite::params![verified_at as i64],
    )
    .unwrap();
}

fn mark_test_web_console_totp_configured(db_path: &str) {
    let now = chrono::Utc::now().timestamp();
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    crate::web_console_auth::init_mfa_schema(&conn).unwrap();
    conn.execute(
        r#"
        INSERT OR REPLACE INTO totp_secrets (
          google_sub,
          secret_encrypted,
          created_at,
          verified_at,
          last_used_counter
        ) VALUES (?1, ?2, ?3, ?3, NULL)
        "#,
        rusqlite::params!["google-sub-test", vec![1_u8; 32], now],
    )
    .unwrap();
}

fn insert_test_webauthn_credential(db_path: &str, google_sub: &str) {
    let conn = crate::encrypted_sqlite::open(db_path).unwrap();
    crate::web_console_auth::init_mfa_schema(&conn).unwrap();
    conn.execute(
        r#"
        INSERT INTO webauthn_credentials (
          id, google_sub, credential_id, public_key_cose, sign_count,
          transports_json, backup_eligible, backup_state, created_at,
          last_used_at, nickname
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)
        "#,
        rusqlite::params![
            "cred-test-1",
            google_sub,
            vec![1_u8, 2, 3],
            vec![4_u8, 5, 6],
            1_i64,
            r#"["internal"]"#,
            1_i64,
            1_i64,
            123_i64,
            "Test passkey",
        ],
    )
    .unwrap();
}

/// Generate valid Telegram WebApp initData for testing.
fn make_test_init_data(bot_token: &str, user_id: i64) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    type HmacSha256 = Hmac<Sha256>;

    let auth_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let user_json = serde_json::json!({
        "id": user_id,
        "first_name": "Test",
        "username": "testadmin"
    })
    .to_string();

    let mut params = BTreeMap::new();
    params.insert("auth_date", auth_date.to_string());
    params.insert("user", user_json.clone());

    let data_check_string: String = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut secret_hmac = HmacSha256::new_from_slice(b"WebAppData").unwrap();
    secret_hmac.update(bot_token.as_bytes());
    let secret_key = secret_hmac.finalize().into_bytes();

    let mut data_hmac = HmacSha256::new_from_slice(&secret_key).unwrap();
    data_hmac.update(data_check_string.as_bytes());
    let hash = hex::encode(data_hmac.finalize().into_bytes());

    format!(
        "auth_date={}&user={}&hash={}",
        auth_date,
        urlencoding::encode(&user_json),
        hash
    )
}

fn test_unified_item_for_vnext(
    id: &str,
    kind: &str,
    status: &str,
    urgency: &str,
) -> UnifiedInboxItem {
    UnifiedInboxItem {
        id: id.into(),
        kind: kind.into(),
        title: format!("Item {id}"),
        title_code: None,
        summary: "Safe summary".into(),
        owner_visible_reason: "Aegis surfaced this for owner review.".into(),
        owner_visible_reason_code: None,
        authority_basis: "direct_request".into(),
        source: if kind == "calendar" {
            "calendar"
        } else {
            "tasks"
        }
        .into(),
        source_id: id.into(),
        source_event_id: Some(format!("source:{id}")),
        status: status.into(),
        urgency: urgency.into(),
        sensitivity: "S2".into(),
        privacy_level: "metadata".into(),
        created_at: "2026-05-10T00:00:00Z".into(),
        updated_at: "2026-05-10T00:01:00Z".into(),
        due_at: None,
        primary_action: Some(UnifiedInboxAction {
            label: "Open".into(),
            label_code: None,
            action_kind: "navigate".into(),
            route: Some("/webapp/console/test".into()),
            requires_gate: status == "needs_decision",
            requires_mfa: false,
        }),
        secondary_actions: Vec::new(),
        next_safe_action: "Open and review.".into(),
        next_safe_action_code: None,
        blocked_by: None,
        blocked_by_code: None,
        receipt_expectation: "A receipt is required before Aegis can claim completion.".into(),
        authority: None,
        detail_route: Some("/webapp/console/test".into()),
        external_url_policy: "internal_only".into(),
        related_action_ids: vec![format!("action:{id}")],
        related_receipt_ids: vec![format!("receipt:{id}")],
        route_reason: "test".into(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

#[test]
fn system_maintenance_loops_and_unclaimed_inferred_goals_stay_out_of_owner_work() {
    // 2026-09-13 Work-page walk: the April "存入系统记忆（L2）" callback (source
    // migrated) and 12 zero-progress inferred goals were the owner's lane.
    let mut task = test_task_for_projection();
    task.source = "migrated".into();
    task.execution_mode = "llm_callback".into();
    assert!(crate::admin::inbox_projection::task_is_system_maintenance_loop(&task));
    task.execution_mode = "notify".into();
    assert!(!crate::admin::inbox_projection::task_is_system_maintenance_loop(&task));
    task.source = "user".into();
    task.execution_mode = "llm_callback".into();
    assert!(!crate::admin::inbox_projection::task_is_system_maintenance_loop(&task));

    let mut goal = crate::memory::Goal {
        id: 8,
        user_did: "did:test".into(),
        title: "user manages family goals like summer Guangzhou trip".into(),
        description: String::new(),
        status: crate::memory::GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.5,
        milestones: Vec::new(),
        created_at: "2026-04-12T00:00:00Z".into(),
        target_date: None,
        last_progress_at: None,
        source: crate::memory::GoalSource::Inferred,
    };
    assert!(
        crate::admin::inbox_projection::goal_is_unclaimed_inference(&goal),
        "0.5% inferred, no milestones"
    );
    goal.progress_pct = 5.0;
    assert!(
        !crate::admin::inbox_projection::goal_is_unclaimed_inference(&goal),
        "progress claims it"
    );
    goal.progress_pct = 0.0;
    goal.source = crate::memory::GoalSource::Explicit;
    assert!(
        !crate::admin::inbox_projection::goal_is_unclaimed_inference(&goal),
        "explicit goals always surface"
    );
}

fn test_task_for_projection() -> crate::memory::Task {
    crate::memory::Task {
        id: 42,
        chat_id: 12345678,
        user_did: "owner".into(),
        title: "Check commute if traffic drops below one hour".into(),
        body: "Private task body should stay gated.".into(),
        priority: "normal".into(),
        status: "active".into(),
        tags: Vec::new(),
        due_date: None,
        source: "user".into(),
        trigger_at_unix: None,
        trigger_type: None,
        event_key: None,
        execution_mode: "manual".into(),
        context_text: None,
        interval_seconds: None,
        group_key: None,
        max_fires: None,
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        fired_at: None,
        retry_count: 0,
        chain_depth: 0,
        pin_on_fire: false,
        expires_at_unix: None,
        is_callback_origin: false,
        created_at: "2026-05-10T00:00:00Z".into(),
        updated_at: "2026-05-10T00:01:00Z".into(),
        completed_at: None,
    }
}

fn test_action_ledger_entry_for_vnext(
    status: crate::action_ledger::ActionStatus,
    action_kind: crate::action_ledger::ActionKind,
) -> crate::action_ledger::ActionLedgerEntry {
    crate::action_ledger::ActionLedgerEntry {
        id: 77,
        action_id: format!("test-action:{}", action_kind.as_str()),
        action_kind,
        status,
        requested_by: Some("did:aegis:telegram:12345678".into()),
        chat_id: Some(12345678),
        created_at: "2026-05-10T00:00:00Z".into(),
        proposed_at: Some("2026-05-10T00:00:00Z".into()),
        approved_at: None,
        executing_at: None,
        executed_at: None,
        failed_at: None,
        cancelled_at: None,
        input_summary: Some("test action".into()),
        user_visible_summary: Some("Test action".into()),
        external_receipt: None,
        external_message_id: None,
        external_event_id: None,
        tool_call_id: None,
        assistant_event_id: None,
        provider_request_id: None,
        idempotency_key: Some("test-action".into()),
        error_kind: None,
        error_message: None,
        retry_policy: None,
        trace_id: None,
        run_id: None,
        mandate_id: None,
        source_event_id: None,
        metadata_json: None,
    }
}

fn settings_projection_test_principal() -> WebConsolePrincipal {
    WebConsolePrincipal::WebConsole(crate::web_console_auth::WebConsoleSession {
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
    })
}

// ── Sensor 2 — FE↔BE contract fixtures (S1 coverage-sensor layer) ────────────
//
// Checked-in goldens under `webapp/tests/fixtures/contract/*.json` that BOTH
// sides read: these Rust tests produce the REAL response JSON (oneshot through
// `oauth_callback_router` where a harness exists; the production constructor
// directly where that is the honest source) and golden-compare it; the FE
// runner (`webapp/tests/contractFixtures.test.mjs`) loads the same files and
// runs each through the REAL matching FE parser/guard. A BE field added
// without FE wiring (the `embedder_degraded` outage class) now fails CI on
// one side or the other instead of shipping silently.
//
// Update flow: `AEGIS_UPDATE_CONTRACT_FIXTURES=1 cargo test -p aegis-runtime-core
// --lib contract_fixture_` rewrites the fixtures; re-run the FE contract test
// and review the diff. See webapp/tests/fixtures/contract/README.md.

fn contract_fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../webapp/tests/fixtures/contract")
        .join(name)
}

fn assert_contract_fixture(name: &str, actual: &serde_json::Value) {
    let path = contract_fixture_path(name);
    if std::env::var("AEGIS_UPDATE_CONTRACT_FIXTURES")
        .ok()
        .as_deref()
        == Some("1")
    {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let pretty = serde_json::to_string_pretty(actual).unwrap() + "\n";
        std::fs::write(&path, pretty).unwrap();
        return;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "contract fixture {} is missing ({e}).\n\
             Generate it: AEGIS_UPDATE_CONTRACT_FIXTURES=1 cargo test -p \
             aegis-runtime-core --lib contract_fixture_",
            path.display()
        )
    });
    let expected: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("contract fixture {} is not JSON: {e}", path.display()));
    assert_eq!(
        actual, &expected,
        "FE↔BE contract drift for fixture '{name}'.\n\
         If the backend change is INTENTIONAL: regenerate with \
         AEGIS_UPDATE_CONTRACT_FIXTURES=1, then update the FE parser/guard and \
         webapp/tests/contractFixtures.test.mjs expectations in the SAME change."
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// S2 W5 (2026-07-08 audit): router-level oneshots for the top uncovered
// owner-visible routes. Each block below removes its route from the
// route-coverage sensor's KNOWN_UNCOVERED exemption list (the ratchet).
// ═════════════════════════════════════════════════════════════════════════════

/// Build a `multipart/form-data` upload request for `/webapp/vnext/upload`.
fn build_upload_request(
    cookie_token: &str,
    csrf_token: Option<&str>,
    filename: &str,
    declared_content_type: Option<&str>,
    data: &[u8],
) -> Request<Body> {
    let boundary = "aegis-test-boundary-7d8f";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    if let Some(ct) = declared_content_type {
        body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
    }
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut builder = Request::builder()
        .method("POST")
        .uri("/webapp/vnext/upload")
        .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(csrf) = csrf_token {
        builder = builder.header(crate::web_console_auth::CSRF_HEADER_NAME, csrf);
    }
    builder.body(Body::from(body)).unwrap()
}

const TEST_PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

// ── W5b: /webapp/vnext/turns/{turn_id}/stream-token + /events ───────────────

/// State variant with a live ProgressRouter so the SSE events route can
/// register a per-turn sink (the plain webapp test states carry `None` and
/// would 503). The telegram sender is a throwaway bounded channel — no test
/// event ever targets the Telegram sink.
fn webapp_test_state_with_progress_router(db_path: Option<String>) -> Arc<AdminState> {
    let (telegram_tx, _telegram_rx) =
        tokio::sync::mpsc::channel::<crate::progress_router::ProgressEvent>(16);
    Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
        memory: MemoryHandle::spawn_in_memory().unwrap(),
        api_token: Some("test-secret-token".into()),
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: None,
        credential_aes_key: None,
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: None,
        bot_token: Some("123456:TEST-TOKEN".into()),
        admin_chat_id: Some(12345678),
        orchestrator: None,
        event_tx: None,
        db_path,
        progress_router: Some(Arc::new(crate::progress_router::ProgressRouter::new(
            telegram_tx,
        ))),
        local_login_token: None,
    })
}

// ── W5e: mandate lifecycle happy paths through the router ───────────────────

async fn post_mandate_route(
    state: Arc<AdminState>,
    cookie_token: &str,
    csrf_token: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let resp = oauth_callback_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Cookie", format!("__Host-aegis_session={cookie_token}"))
                .header(crate::web_console_auth::CSRF_HEADER_NAME, csrf_token)
                .header("content-type", "application/json")
                .body(match body {
                    Some(json) => Body::from(serde_json::to_vec(&json).unwrap()),
                    None => Body::empty(),
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 128 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| serde_json::json!({ "raw": String::from_utf8_lossy(&bytes) }));
    (status, json)
}

// Topic-split parts (2026-09-16). Every file here must also be listed in
// core/tests/route_coverage_guard.rs `ADMIN_TESTS_SRC`, or its routes lose
// coverage evidence and the ratchet fails.
mod part1;
mod part1_vnext_commands;
mod part1_vnext_projections;
mod part2;
mod part2_memory_actions;
mod part2_owner_work;
mod part2_settings_providers;
mod part3;
mod part3_llm_settings;
mod part3_vnext_runtime_commands;
mod part3_web_hardening;
mod part4;
mod part5;
