use super::*;

/// OAuth2 callback port for Google authentication flow.
pub(super) const OAUTH_CALLBACK_PORT: u16 = 8_765;

/// CSP for the web console. `connect-src` must name the Realtime voice socket
/// origin: the CEO voice agent opens `wss://…` from the browser straight to the
/// provider, and a policy of `'self'` alone makes WebKit reject that socket with
/// `SecurityError: The operation is insecure.` before any audio flows.
pub(super) fn webapp_csp_with_voice_origin(voice_ws_url: &str) -> String {
    let origin = ws_origin(voice_ws_url)
        .or_else(|| ws_origin(crate::voice_session::DEFAULT_WS_URL))
        .unwrap_or_default();
    let connect = if origin.is_empty() {
        "'self'".to_string()
    } else {
        format!("'self' {origin}")
    };
    format!(
        "default-src 'self'; \
         script-src 'self' https://telegram.org; \
         worker-src 'self'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         img-src 'self' https: data: blob:; \
         font-src 'self' data: https://fonts.gstatic.com; \
         media-src 'self' blob:; \
         connect-src {connect}; \
         manifest-src 'self'; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'none'; \
         frame-ancestors 'self' https://web.telegram.org https://*.telegram.org"
    )
}

/// `wss://api.x.ai/v1/realtime?x=1` → `wss://api.x.ai`. None when not a ws URL.
pub(super) fn ws_origin(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    if !matches!(scheme, "ws" | "wss") {
        return None;
    }
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .map(str::trim)
        .filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}"))
}

fn webapp_csp() -> &'static str {
    static CSP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSP.get_or_init(|| webapp_csp_with_voice_origin(&crate::voice_session::voice_ws_url_from_env()))
}

pub(super) const WEBAPP_NO_STORE: &str = "no-store";
const WEBAPP_STATIC_CACHE: &str = "no-store";
const WEBAPP_PAGE_CACHE: &str = "no-store";
pub(super) const WEBAPP_STREAM_TOKEN_TTL_SECS: u64 = 60;
pub(super) const WEBAPP_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
pub(super) const WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE: usize = 60;
pub(super) const WEBAPP_STALE_UNREAD_EVENT_DAYS: i64 = 14;
/// How many recent assistant events the Work projection scans. Raw mail events
/// no longer produce Work items (owner decision 2026-07-09) but still dominate
/// the event stream, so the scan window must be deep enough that a burst of
/// email cannot starve calendar events out of the queue (Codex P2 2026-07-09).
pub(super) const WEBAPP_WORK_PROJECTION_EVENT_SCAN_WINDOW: usize = 400;
/// Cap on items projected from assistant events (the pre-2026-07-09 bound,
/// when the scan window and the item cap were both 100).
pub(super) const WEBAPP_WORK_PROJECTION_EVENT_ITEM_CAP: usize = 100;
/// Mandate matching keeps its historical 100-event window (it must still see
/// raw mail events, and a deeper window would change how many rule-match
/// cards + persisted notification plans older events can generate).
pub(super) const WEBAPP_MANDATE_MATCH_EVENT_WINDOW: usize = 100;
pub(super) const WEBAPP_MEMORY_REVIEW_OWNER_SURFACE_DAYS: i64 = 180;
pub(super) const WEBAPP_MEMORY_REVIEW_OWNER_SURFACE_LIMIT: usize = 3;
pub(super) const WEBAPP_MEMORY_UPDATE_VALUE_MAX_CHARS: usize = 4_000;
pub(super) const WEBAPP_PROFILE_UPDATE_VALUE_MAX_CHARS: usize = 2_000;
pub(super) const WEBAPP_STREAM_TOKEN_RATE_LIMIT_PER_MINUTE: usize = 60;
pub(super) const WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS: u64 = 30 * 24 * 60 * 60;
pub(super) const WEBAPP_DEBUG_SNAPSHOT_TTL_SECS: i64 = 24 * 60 * 60;
pub(super) const WEBAPP_DEBUG_SNAPSHOT_MAX_BYTES: usize = 96 * 1024;
pub(super) const WEBAPP_DEBUG_SNAPSHOT_MAX_STRING_CHARS: usize = 8_000;
pub(super) const WEBAPP_DEBUG_LEASE_DEFAULT_MINUTES: u32 = 8 * 60;
pub(super) const WEBAPP_DEBUG_LEASE_MIN_MINUTES: u32 = 5;
pub(super) const WEBAPP_DEBUG_LEASE_MAX_MINUTES: u32 = 24 * 60;
pub(super) const WEBAPP_DEBUG_GRANT_TOKEN_PREFIX: &str = "aegis_dbg_";
#[cfg(test)]
pub(super) const WEBAPP_DEBUG_GRANT_TOKEN_MAX_CHARS: usize = 128;
/// Debug-lease scope used by remaining inspect-grant tests.
pub(super) const WEBAPP_DEBUG_SCOPE_MAIL: &str = "mail";
pub(super) const WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE: &str = "webapp_e2e_suite";
pub(super) const WEBAPP_DEBUG_JOB_STATUS_QUEUED: &str = "queued";
pub(super) const WEBAPP_DEBUG_JOB_STATUS_RUNNING: &str = "running";
pub(super) const WEBAPP_DEBUG_JOB_STATUS_PASSED: &str = "passed";
pub(super) const WEBAPP_DEBUG_JOB_STATUS_FAILED: &str = "failed";
pub(super) const WEBAPP_DEBUG_JOB_TTL_SECS: i64 = 7 * 24 * 60 * 60;
pub(super) const WEBAPP_DEBUG_JOB_LOG_MAX_CHARS: usize = 32_000;
#[cfg(not(test))]
pub(super) const WEBAPP_DEBUG_JOB_SUITE_TIMEOUT_SECS: u64 = 240;
pub(super) const WEB_PUSH_SUBSCRIPTIONS_SKILL: &str = "web_push_subscriptions";
pub(super) const WEB_PUSH_DELIVERY_RECEIPTS_SKILL: &str = "web_push_delivery_receipts";
pub(super) const WEB_PUSH_MAX_SUBSCRIPTIONS_PER_OWNER: usize = 8;
pub(super) const WEB_PUSH_MAX_DELIVERY_RECEIPTS_PER_OWNER: usize = 80;
pub(super) const WEB_PUSH_MAX_ENDPOINT_CHARS: usize = 2_048;
pub(super) const WEB_PUSH_MAX_KEY_CHARS: usize = 512;
pub(super) const WEB_PUSH_MAX_NOTICE_TITLE_CHARS: usize = 80;
pub(super) const WEB_PUSH_MAX_NOTICE_BODY_CHARS: usize = 240;
pub(super) const WEB_PUSH_MAX_NOTICE_TAG_CHARS: usize = 64;
pub(super) const WEB_PUSH_MAX_NOTICE_URL_CHARS: usize = 240;
pub(super) const WEB_PUSH_VAPID_PRIVATE_KEY_MAX_CHARS: usize = 8_192;
pub(super) const WEB_PUSH_DEFAULT_URL: &str = "/ui/";
pub(super) const WEBAPP_DEFAULT_AEGIS_AVATAR_URL: &str = "/ui/aegis-avatar-v2.jpg";
/// Pre-2026-09-11 default (root path; a 404 under the /ui/ mount). Accepted on
/// input and rewritten to the current default so stored profiles keep working.
pub(super) const WEBAPP_LEGACY_AEGIS_AVATAR_URLS: &[&str] =
    &["/aegis-avatar.svg", "/ui/aegis-avatar.svg"];
pub(super) const WEBAPP_AVATAR_MAX_DATA_URL_BYTES: usize = 700_000;
pub(super) const WEBAPP_GENERATED_AVATAR_MAX_BYTES: usize = 5 * 1024 * 1024;
pub(super) const WEBAPP_GENERATED_AVATAR_URL_PREFIX: &str = "/webapp/vnext/avatar-image/";
pub(super) const WEBAPP_VNEXT_CHAT_HISTORY_LIMIT: usize = 24;
pub(super) const WEBAPP_VNEXT_CHAT_HISTORY_ARCHIVE_SCAN_LIMIT: usize = 240;
/// How long a FINISHED singleflight result is retained (measured from
/// completion) so a late `/command/status` poll can still recover the reply.
const WEBAPP_COMMAND_SINGLEFLIGHT_TTL: Duration = Duration::from_secs(10 * 60);
/// Foreground wait before answering 504 "still working" so the client
/// switches to `/command/status` polling. Must stay under Cloudflare's
/// 100 s origin-response limit, or the edge answers 524 instead and the
/// browser never learns the request is still running (owner report
/// 2026-09-05: a 108 s voice turn left the panel stuck on "处理中").
pub(super) const WEBAPP_COMMAND_SINGLEFLIGHT_WAIT: Duration = Duration::from_secs(75);
/// Hard absolute ceiling for an entry whose bg task is STILL running. An
/// in-flight entry is never reaped by the completion TTL (that silently lost
/// the result of very long turns and let a retry start duplicate work); this
/// ceiling only bounds a genuinely hung task so the map cannot leak forever.
/// The orchestrator's own worst case (MAX_ROUNDS × per-call LLM timeout) is
/// well under an hour.
const WEBAPP_COMMAND_SINGLEFLIGHT_INFLIGHT_MAX: Duration = Duration::from_secs(60 * 60);

// Refcount (not a bool): concurrent tests have OVERLAPPING, non-nested guard
// lifetimes, so a swap/restore bool would let one guard's Drop disable snapshots
// while another test is still mid-request. Snapshots are enabled while the count
// is > 0 (any guard alive). See TestWebappDebugSnapshotsGuard.
#[cfg(test)]
pub(super) static WEBAPP_DEBUG_SNAPSHOTS_TEST_REFCOUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

type WebappCommandSingleflightResult = Result<serde_json::Value, (StatusCode, String)>;

pub(super) struct WebappCommandSingleflightEntry {
    created_at: Instant,
    /// Set once when the bg task stores its result. Used so the completion TTL
    /// is measured from completion (not from `created_at`), and so the reaper
    /// can tell a still-running entry (`None`) from a finished one.
    completed_at: parking_lot::Mutex<Option<Instant>>,
    result: parking_lot::Mutex<Option<WebappCommandSingleflightResult>>,
    notify: tokio::sync::Notify,
}

/// Whether a singleflight entry should be retained by the reaper.
///
/// - Still running (`result` is `None`): keep until a hard absolute ceiling
///   (`INFLIGHT_MAX`). Reaping an in-flight entry by the completion TTL was the
///   silent-loss bug — a turn outliving the 10-min TTL would be evicted mid-run,
///   so its result vanished and a retry started duplicate work.
/// - Finished: keep for `TTL` measured from completion so a late status poll can
///   still recover the reply, regardless of how long the turn took.
fn webapp_command_singleflight_entry_is_live(
    entry: &WebappCommandSingleflightEntry,
    now: Instant,
) -> bool {
    if entry.result.lock().is_none() {
        return now.duration_since(entry.created_at) <= WEBAPP_COMMAND_SINGLEFLIGHT_INFLIGHT_MAX;
    }
    match *entry.completed_at.lock() {
        Some(done) => now.duration_since(done) <= WEBAPP_COMMAND_SINGLEFLIGHT_TTL,
        None => true,
    }
}

static WEBAPP_COMMAND_SINGLEFLIGHT: OnceLock<
    parking_lot::Mutex<HashMap<String, Arc<WebappCommandSingleflightEntry>>>,
> = OnceLock::new();

fn webapp_command_singleflight_map()
-> &'static parking_lot::Mutex<HashMap<String, Arc<WebappCommandSingleflightEntry>>> {
    WEBAPP_COMMAND_SINGLEFLIGHT.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

pub(super) fn webapp_command_singleflight_key(
    principal: &WebConsolePrincipal,
    focus_id: &str,
    intent_kind: &str,
    idempotency_key: &str,
) -> String {
    format!(
        "{}:{focus_id}:{intent_kind}:{idempotency_key}",
        principal.log_subject()
    )
}

pub(super) fn webapp_command_singleflight_start(
    key: String,
) -> (Arc<WebappCommandSingleflightEntry>, bool) {
    let now = Instant::now();
    let mut map = webapp_command_singleflight_map().lock();
    map.retain(|_, entry| webapp_command_singleflight_entry_is_live(entry, now));
    if let Some(entry) = map.get(&key) {
        // A finished failure is not reused: the owner's retry (same
        // idempotency key) must run again instead of replaying a stale 503
        // for up to the completion TTL (owner turn 2026-09-08 17:51 UTC —
        // codex had recovered, the console kept saying "model unavailable").
        let replay_failure = matches!(*entry.result.lock(), Some(Err(_)));
        if !replay_failure {
            return (entry.clone(), false);
        }
        map.remove(&key);
    }
    let entry = Arc::new(WebappCommandSingleflightEntry {
        created_at: now,
        completed_at: parking_lot::Mutex::new(None),
        result: parking_lot::Mutex::new(None),
        notify: tokio::sync::Notify::new(),
    });
    map.insert(key, entry.clone());
    (entry, true)
}

pub(super) fn webapp_command_singleflight_finish(
    entry: &WebappCommandSingleflightEntry,
    result: WebappCommandSingleflightResult,
) {
    // Set `completed_at` BEFORE `result` so any observer that sees `result` is
    // `Some` (via the result mutex's release/acquire) also sees `completed_at`.
    *entry.completed_at.lock() = Some(Instant::now());
    *entry.result.lock() = Some(result);
    entry.notify.notify_waiters();
}

/// Turn ids whose command task is still inside the spawned handler. Chat
/// history must not invent a failure reply for these: the owner line is
/// already in L1, and the assistant row arrives when the task finishes.
static WEBAPP_INFLIGHT_TURN_IDS: OnceLock<parking_lot::Mutex<HashSet<String>>> = OnceLock::new();

fn webapp_inflight_turn_ids_map() -> &'static parking_lot::Mutex<HashSet<String>> {
    WEBAPP_INFLIGHT_TURN_IDS.get_or_init(|| parking_lot::Mutex::new(HashSet::new()))
}

pub(super) struct WebappInflightTurnGuard(String);

impl WebappInflightTurnGuard {
    pub(super) fn enter(turn_id: &str) -> Self {
        let turn_id = turn_id.trim();
        if !turn_id.is_empty() {
            webapp_inflight_turn_ids_map()
                .lock()
                .insert(turn_id.to_string());
        }
        Self(turn_id.to_string())
    }
}

impl Drop for WebappInflightTurnGuard {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            webapp_inflight_turn_ids_map().lock().remove(&self.0);
        }
    }
}

pub(super) fn webapp_inflight_turn_ids() -> HashSet<String> {
    webapp_inflight_turn_ids_map().lock().clone()
}

pub(super) async fn webapp_command_singleflight_wait(
    entry: Arc<WebappCommandSingleflightEntry>,
) -> WebappCommandSingleflightResult {
    loop {
        let notified = entry.notify.notified();
        if let Some(result) = entry.result.lock().clone() {
            return result;
        }
        notified.await;
    }
}

/// Read-only peek into the singleflight map for the status-poll endpoint.
///
/// Returns:
/// - `None` — no entry for this key (404: never started, or reaped after TTL).
/// - `Some(None)` — entry exists but the bg task is still running (202 pending).
/// - `Some(Some(result))` — the bg task finished; `result` is the stored
///   `Ok(owner_response)` or `Err((status, message))`.
///
/// This shares the same map + reap discipline as `start`/`finish`
/// (`webapp_command_singleflight_entry_is_live`): it retains only live entries
/// before reading so a poll never observes a stale entry that `start` would
/// have evicted, and — critically — never reaps a still-running entry, so a
/// poll for a very long turn returns `pending` rather than a spurious 404.
pub(super) fn webapp_command_singleflight_peek(
    key: &str,
) -> Option<Option<WebappCommandSingleflightResult>> {
    let now = Instant::now();
    let mut map = webapp_command_singleflight_map().lock();
    map.retain(|_, entry| webapp_command_singleflight_entry_is_live(entry, now));
    let entry = map.get(key)?;
    Some(entry.result.lock().clone())
}

#[cfg(test)]
pub(super) fn webapp_command_singleflight_clear_for_test(key: &str) {
    webapp_command_singleflight_map().lock().remove(key);
}

/// Deterministic exerciser for `webapp_command_singleflight_entry_is_live`
/// without sleeping. Builds an entry whose `created_at` sits `age` in the past
/// (relative to the evaluated `now`) and, if `completed_ago` is `Some`, marks it
/// finished `completed_ago` before `now`. `completed_ago` must be `<= age`.
#[cfg(test)]
pub(super) fn webapp_command_singleflight_is_live_for_test(
    age: Duration,
    completed_ago: Option<Duration>,
) -> bool {
    let created = Instant::now();
    let now = created + age;
    let entry = WebappCommandSingleflightEntry {
        created_at: created,
        completed_at: parking_lot::Mutex::new(completed_ago.map(|ago| now - ago)),
        result: parking_lot::Mutex::new(
            completed_ago
                .map(|_| -> WebappCommandSingleflightResult { Ok(serde_json::Value::Null) }),
        ),
        notify: tokio::sync::Notify::new(),
    };
    webapp_command_singleflight_entry_is_live(&entry, now)
}

// ── System metrics types ────────────────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct SystemMetrics {
    pub(super) cpu_load_1m: f64,
    pub(super) cpu_load_5m: f64,
    pub(super) cpu_load_15m: f64,
    pub(super) memory_total_kb: u64,
    pub(super) memory_available_kb: u64,
    pub(super) memory_used_percent: f64,
    pub(super) disk_total_bytes: u64,
    pub(super) disk_available_bytes: u64,
    pub(super) disk_used_percent: f64,
    pub(super) num_cpus: usize,
}

/// Shared state for the admin API.
pub struct AdminState {
    pub sandbox: Arc<SandboxEngine>,
    pub memory: MemoryHandle,
    /// Bearer token for API authentication. If None, all protected endpoints return 403.
    pub api_token: Option<String>,
    /// Shared runtime settings.
    pub settings: Arc<RuntimeSettings>,
    /// Credential store DB path and AES key (open connections on demand).
    pub credential_db_path: Option<String>,
    pub credential_aes_key: Option<[u8; 32]>,
    /// Google OAuth2 manager (if configured).
    pub google_auth: parking_lot::RwLock<Option<Arc<GoogleAuthManager>>>,
    /// Telegram bot handle for sending OAuth completion notifications.
    pub telegram_bot: Option<teloxide::Bot>,
    /// Agent identity (Ed25519 key) for HMAC state signing.
    pub identity: Option<Arc<AgentIdentity>>,
    /// Public URL for OAuth callback (e.g. "https://aegis.example.com").
    pub public_url: Option<String>,
    /// Telegram bot token for WebApp initData HMAC verification.
    pub bot_token: Option<String>,
    /// Admin Telegram chat ID for WebApp access control.
    pub admin_chat_id: Option<i64>,
    /// Orchestrator handle for resuming WebApp submissions.
    pub orchestrator: Option<Arc<crate::orchestrator::Orchestrator>>,
    /// Channel to emit IntentEvents (e.g. from Google push).
    pub event_tx: Option<tokio::sync::mpsc::Sender<crate::intent::IntentEvent>>,
    /// Path to the shared SQLite DB for watermarks.
    pub db_path: Option<String>,
    /// Shared progress-event router (Slice 2): the SAME `ProgressRouter`
    /// instance whose `progress_sender()` the orchestrator emits through. Lets
    /// the per-turn SSE endpoint `register_turn` a webapp sink that the
    /// orchestrator's events then fan out to. `None` in test harnesses /
    /// configs without a wired router.
    pub progress_router: Option<Arc<crate::progress_router::ProgressRouter>>,
    /// Token behind `GET /auth/login/local` (README on-ramp for installs with
    /// neither Google Login nor Telegram). `None` disables the route (404).
    /// Policy: `web_console_auth::local_login_token_from_lookup`.
    pub local_login_token: Option<String>,
}

pub(super) fn oauth_success_notification_destination(
    callback_chat_id: i64,
    admin_chat_id: Option<i64>,
) -> (teloxide::types::ChatId, Option<teloxide::types::ThreadId>) {
    let admin_chat_id = admin_chat_id.unwrap_or(callback_chat_id);
    let route = crate::telegram_notification_router::route_telegram_notification(
        admin_chat_id,
        crate::telegram_notification_router::proactive_route_request(
            crate::telegram_notification_router::NotificationClass::SystemStatus,
            crate::telegram_notification_router::PrivacyLevel::SystemOnly,
            Some(callback_chat_id),
        ),
    );

    (
        teloxide::types::ChatId(route.chat_id),
        crate::telegram::wrap_thread_id(route.thread_id),
    )
}

fn oauth_callback_rate_limit_key(state_param: &str) -> String {
    let mut hasher = DefaultHasher::new();
    state_param.hash(&mut hasher);
    format!("state:{:016x}", hasher.finish())
}

/// Resolve the live Google auth manager for admin/status/monitoring paths.
pub fn resolve_live_google_auth_manager(state: &AdminState) -> Option<Arc<GoogleAuthManager>> {
    state.google_auth.read().clone()
}

#[allow(dead_code)]
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
pub(super) struct SkillInfo {
    pub(super) name: String,
    pub(super) max_memory_mb: u32,
    pub(super) allowed_domains: Vec<String>,
    pub(super) cron_schedule: Option<String>,
    pub(super) description: String,
}

#[derive(Serialize)]
pub(super) struct SkillsResponse {
    pub(super) count: usize,
    pub(super) skills: Vec<SkillInfo>,
}

/// Constant-time string comparison to prevent timing attacks on token validation.
///
/// Delegates to the `subtle` crate's `ConstantTimeEq` trait, which guarantees
/// fixed-time comparison regardless of where (or whether) the inputs differ.
pub(super) fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Bearer token authentication middleware.
///
/// Checks the `Authorization: Bearer <token>` header against AdminState.api_token.
/// If no token is configured, all requests are rejected with 403 (secure by default).
async fn require_auth(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let expected = match &state.api_token {
        Some(t) => t,
        None => {
            return (
                StatusCode::FORBIDDEN,
                "Admin API token not configured. Set ADMIN_API_TOKEN in .env",
            )
                .into_response();
        }
    };

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];
            if constant_time_eq(token, expected) {
                next.run(req).await
            } else {
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            "Missing Authorization: Bearer <token> header",
        )
            .into_response(),
    }
}

async fn list_webapp_debug_snapshots(
    State(state): State<Arc<AdminState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_webapp_debug_snapshots_enabled()?;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let snapshots =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<WebappDebugSnapshotSummary>> {
            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_snapshot_schema(&conn)?;
            cleanup_expired_webapp_debug_snapshots(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, surface, path, payload_bytes,
                        created_at_unix, expires_at_unix
                 FROM webapp_debug_snapshots
                 ORDER BY created_at_unix DESC
                 LIMIT 20",
            )?;
            let rows = stmt.query_map([], read_debug_snapshot_summary)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Debug snapshot list task failed: {err}"),
            )
        })?
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list debug snapshots: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "snapshots": snapshots,
    })))
}

async fn get_webapp_debug_snapshot(
    State(state): State<Arc<AdminState>>,
    Path(snapshot_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_webapp_debug_snapshots_enabled()?;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let snapshot = tokio::task::spawn_blocking(
        move || -> anyhow::Result<Option<WebappDebugSnapshotRecord>> {
            use rusqlite::OptionalExtension as _;

            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_snapshot_schema(&conn)?;
            cleanup_expired_webapp_debug_snapshots(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, surface, path, payload_json,
                        payload_bytes, created_at_unix, expires_at_unix
                 FROM webapp_debug_snapshots
                 WHERE id = ?1",
            )?;
            Ok(stmt
                .query_row([snapshot_id], read_debug_snapshot_record)
                .optional()?)
        },
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug snapshot get task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to get debug snapshot: {err}"),
        )
    })?;

    let snapshot = snapshot.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "Debug snapshot not found".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "snapshot": snapshot,
    })))
}

async fn delete_webapp_debug_snapshot(
    State(state): State<Arc<AdminState>>,
    Path(snapshot_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_webapp_debug_snapshots_enabled()?;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let deleted = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let conn = crate::encrypted_sqlite::open(&db_path)?;
        init_webapp_debug_snapshot_schema(&conn)?;
        Ok(conn.execute(
            "DELETE FROM webapp_debug_snapshots WHERE id = ?1",
            [snapshot_id],
        )?)
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug snapshot delete task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to delete debug snapshot: {err}"),
        )
    })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "deleted": deleted,
    })))
}

async fn list_webapp_debug_leases(
    State(state): State<Arc<AdminState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_webapp_debug_snapshots_enabled()?;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let leases =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<WebappDebugLeaseSummary>> {
            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_lease_schema(&conn)?;
            cleanup_expired_webapp_debug_leases(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, scope, permissions_json, item_id,
                        reason, created_at_unix, expires_at_unix, revoked_at_unix
                 FROM webapp_debug_leases
                 WHERE revoked_at_unix IS NULL
                 ORDER BY created_at_unix DESC
                 LIMIT 50",
            )?;
            let rows = stmt.query_map([], read_debug_lease_summary)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Debug lease list task failed: {err}"),
            )
        })?
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list debug leases: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "leases": leases,
    })))
}

async fn webapp_security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let mut response = next.run(req).await;
    let status = response.status();
    let headers = response.headers_mut();

    if let Ok(csp) = axum::http::HeaderValue::from_str(webapp_csp()) {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, csp);
    }
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    let cache_control = webapp_cache_control_for(&path, status);
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static(cache_control),
    );

    response
}

pub(super) fn webapp_cache_control_for(path: &str, status: StatusCode) -> &'static str {
    if path.starts_with("/ui/assets/") && status.is_success() {
        WEBAPP_STATIC_CACHE
    } else if (path.starts_with("/ui/") || path == "/ui") && status.is_success() {
        WEBAPP_PAGE_CACHE
    } else {
        WEBAPP_NO_STORE
    }
}

// ── GET /heartbeat-audit ────────────────────────────────────────────────────

/// GET /heartbeat-audit?limit=20&skill=__heartbeat__
/// Returns recent heartbeat audit entries with optional raw process log.
async fn handle_heartbeat_audit(
    Query(params): Query<HeartbeatAuditParams>,
    State(state): State<Arc<AdminState>>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(20).min(100);
    let skill_filter = params.skill.as_deref();

    match state
        .memory
        .query_heartbeat_audit(skill_filter, limit)
        .await
    {
        Ok(entries) => {
            let response: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "cycle_id": e.cycle_id,
                        "skill_name": e.skill_name,
                        "timestamp": e.timestamp,
                        "severity": e.severity,
                        "probe_payload": e.probe_payload,
                        "tools_called": e.tools_called,
                        "tools_blocked": e.tools_blocked,
                        "actions_taken": e.actions_taken,
                        "llm_reply": e.llm_reply,
                        "tokens_used": e.tokens_used,
                        "rounds_used": e.rounds_used,
                        "raw_process_log": e.raw_process_log,
                    })
                })
                .collect();
            Json(serde_json::json!({ "entries": response })).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to query heartbeat audit");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to query audit log: {}", e)
                })),
            )
                .into_response()
        }
    }
}

/// GET /heartbeat-dashboard
/// Returns a self-contained HTML page that displays heartbeat audit data
/// with expand/collapse for raw process logs. Fetches data from /heartbeat-audit.
async fn handle_heartbeat_dashboard(headers: axum::http::HeaderMap) -> impl IntoResponse {
    // Extract bearer token from the request to pass to the JS fetch calls
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Aegis Runtime Heartbeat Dashboard</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0d1117; color: #c9d1d9; padding: 16px; max-width: 900px; margin: 0 auto; }}
  h1 {{ font-size: 1.4em; margin-bottom: 16px; color: #58a6ff; }}
  .stats {{ display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap; }}
  .stat {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px 16px; min-width: 120px; }}
  .stat-value {{ font-size: 1.6em; font-weight: 700; color: #58a6ff; }}
  .stat-label {{ font-size: 0.85em; color: #8b949e; margin-top: 4px; }}
  .entry {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; margin-bottom: 12px; overflow: hidden; }}
  .entry-header {{ display: flex; justify-content: space-between; align-items: center; padding: 12px 16px; cursor: pointer; user-select: none; }}
  .entry-header:hover {{ background: #1c2128; }}
  .entry-label {{ font-weight: 600; }}
  .entry-meta {{ display: flex; gap: 12px; font-size: 0.85em; color: #8b949e; }}
  .entry-body {{ padding: 0 16px 16px; display: none; }}
  .entry-body.open {{ display: block; }}
  .tag {{ display: inline-block; padding: 2px 8px; border-radius: 12px; font-size: 0.75em; font-weight: 600; }}
  .tag-heartbeat {{ background: #1f3a2a; color: #56d364; }}
  .tag-probe {{ background: #2a1f1f; color: #f78166; }}
  .section {{ margin-top: 12px; }}
  .section-title {{ font-size: 0.85em; font-weight: 600; color: #8b949e; margin-bottom: 6px; text-transform: uppercase; letter-spacing: 0.5px; }}
  .tools-list {{ display: flex; gap: 6px; flex-wrap: wrap; }}
  .tool-chip {{ background: #1c2128; border: 1px solid #30363d; border-radius: 4px; padding: 2px 8px; font-size: 0.85em; font-family: monospace; }}
  .raw-log {{ background: #0d1117; border: 1px solid #30363d; border-radius: 6px; padding: 12px; font-family: 'Fira Code', 'Cascadia Code', monospace; font-size: 0.82em; line-height: 1.6; white-space: pre-wrap; word-break: break-word; max-height: 500px; overflow-y: auto; }}
  .reply-text {{ background: #0d1117; border-left: 3px solid #58a6ff; padding: 8px 12px; font-size: 0.9em; line-height: 1.5; white-space: pre-wrap; }}
  .chevron {{ transition: transform 0.2s; display: inline-block; }}
  .chevron.open {{ transform: rotate(90deg); }}
  .loading {{ text-align: center; padding: 40px; color: #8b949e; }}
  .error {{ background: #2a1f1f; border: 1px solid #f85149; border-radius: 8px; padding: 16px; color: #f78166; }}
  .refresh-btn {{ background: #21262d; border: 1px solid #30363d; color: #c9d1d9; padding: 6px 14px; border-radius: 6px; cursor: pointer; font-size: 0.85em; }}
  .refresh-btn:hover {{ background: #30363d; }}
  .toolbar {{ display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }}
  .filter {{ display: flex; gap: 8px; align-items: center; }}
  .filter select {{ background: #161b22; border: 1px solid #30363d; color: #c9d1d9; padding: 4px 8px; border-radius: 4px; }}
</style>
</head>
<body>
<div class="toolbar">
  <h1>🫀 Heartbeat Dashboard</h1>
  <div class="filter">
    <select id="filter-skill">
      <option value="">All / 全部</option>
      <option value="__heartbeat__">True Heartbeat / 真心跳</option>
    </select>
    <button class="refresh-btn" onclick="loadData()">↻ Refresh / 刷新</button>
  </div>
</div>
<div class="stats" id="stats"></div>
<div id="entries"><div class="loading">Loading / 加载中...</div></div>

<script>
const TOKEN = "{token}";
const BASE = window.location.origin;

async function loadData() {{
  const filter = document.getElementById('filter-skill').value;
  const url = filter
    ? `${{BASE}}/heartbeat-audit?limit=50&skill=${{encodeURIComponent(filter)}}`
    : `${{BASE}}/heartbeat-audit?limit=50`;

  try {{
    const res = await fetch(url, {{
      headers: {{ 'Authorization': `Bearer ${{TOKEN}}` }}
    }});
    if (!res.ok) throw new Error(`HTTP ${{res.status}}`);
    const data = await res.json();
    render(data.entries);
  }} catch (e) {{
    document.getElementById('entries').innerHTML =
      `<div class="error">Failed to load: ${{e.message}}</div>`;
  }}
}}

function render(entries) {{
  // Stats
  const totalTokens = entries.reduce((s, e) => s + (e.tokens_used || 0), 0);
  const hbCount = entries.filter(e => e.skill_name === '__heartbeat__').length;
  const probeCount = entries.length - hbCount;
  const withRaw = entries.filter(e => e.raw_process_log).length;

  document.getElementById('stats').innerHTML = `
    <div class="stat"><div class="stat-value">${{entries.length}}</div><div class="stat-label">Total Cycles / 总周期</div></div>
    <div class="stat"><div class="stat-value">${{hbCount}}</div><div class="stat-label">True Heartbeat / 真心跳</div></div>
    <div class="stat"><div class="stat-value">${{probeCount}}</div><div class="stat-label">Probe / 探针</div></div>
    <div class="stat"><div class="stat-value">${{totalTokens.toLocaleString()}}</div><div class="stat-label">Tokens Used / 使用量</div></div>
    <div class="stat"><div class="stat-value">${{withRaw}}</div><div class="stat-label">With Process Log / 含过程</div></div>
  `;

  // Entries
  if (entries.length === 0) {{
    document.getElementById('entries').innerHTML =
      '<div class="loading">No entries found / 暂无记录</div>';
    return;
  }}

  document.getElementById('entries').innerHTML = entries.map((e, i) => {{
    const isHb = e.skill_name === '__heartbeat__';
    const tagClass = isHb ? 'tag-heartbeat' : 'tag-probe';
    const tagText = isHb ? '真心跳' : `探针: ${{e.skill_name}}`;

    const tools = (e.tools_called || []).map(t =>
      `<span class="tool-chip">${{esc(t)}}</span>`
    ).join('');

    const actions = (e.actions_taken || []).map(a =>
      `<li>${{esc(a)}}</li>`
    ).join('');

    const rawLog = e.raw_process_log
      ? `<div class="section">
           <div class="section-title">Raw Process Log / 原始过程</div>
           <div class="raw-log">${{esc(e.raw_process_log)}}</div>
         </div>`
      : '';

    const reply = e.llm_reply
      ? `<div class="section">
           <div class="section-title">LLM Reply / 回复</div>
           <div class="reply-text">${{esc(e.llm_reply)}}</div>
         </div>`
      : '';

    return `
      <div class="entry">
        <div class="entry-header" onclick="toggle(${{i}})">
          <div>
            <span class="chevron" id="chev-${{i}}">▶</span>
            <span class="tag ${{tagClass}}">${{tagText}}</span>
            <span class="entry-label">${{esc(e.cycle_id)}}</span>
          </div>
          <div class="entry-meta">
            <span>${{e.tokens_used}} tok</span>
            <span>${{e.rounds_used}} rounds</span>
            <span>${{esc(e.timestamp)}}</span>
          </div>
        </div>
        <div class="entry-body" id="body-${{i}}">
          ${{tools ? `<div class="section"><div class="section-title">Tools Called / 工具调用</div><div class="tools-list">${{tools}}</div></div>` : ''}}
          ${{actions ? `<div class="section"><div class="section-title">Actions / 动作</div><ul>${{actions}}</ul></div>` : ''}}
          ${{reply}}
          ${{rawLog}}
          ${{e.severity !== 'Info' ? `<div class="section"><div class="section-title">Severity</div><span>${{esc(e.severity)}}</span></div>` : ''}}
        </div>
      </div>`;
  }}).join('');
}}

function toggle(i) {{
  const body = document.getElementById('body-' + i);
  const chev = document.getElementById('chev-' + i);
  body.classList.toggle('open');
  chev.classList.toggle('open');
}}

function esc(s) {{
  if (!s) return '';
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}}

document.getElementById('filter-skill').addEventListener('change', loadData);
loadData();
</script>
</body>
</html>"##,
        token = token
    );

    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate",
        )],
        axum::response::Html(html),
    )
        .into_response()
}

/// GET /api/cost-summary — JSON cost analytics for programmatic consumption.
async fn handle_cost_summary_api(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    let daily = state
        .memory
        .query_daily_cost_totals(30)
        .await
        .unwrap_or_default();
    let by_label = state
        .memory
        .query_cost_by_label(7)
        .await
        .unwrap_or_default();
    let by_model = state
        .memory
        .query_cost_by_model(7)
        .await
        .unwrap_or_default();

    let daily_json: Vec<serde_json::Value> = daily
        .iter()
        .map(|d| serde_json::json!({ "date": d.date, "cost": d.cost, "calls": d.calls }))
        .collect();
    let label_json: Vec<serde_json::Value> = by_label
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l.label, "cost": l.total_cost,
                "calls": l.call_count, "avg_latency_ms": l.avg_latency_ms
            })
        })
        .collect();
    let model_json: Vec<serde_json::Value> = by_model
        .iter()
        .map(|m| {
            serde_json::json!({
                "model": m.model, "cost": m.total_cost,
                "prompt_tokens": m.prompt_tokens, "completion_tokens": m.completion_tokens
            })
        })
        .collect();

    let total_30d: f64 = daily.iter().map(|d| d.cost).sum();
    let total_7d: f64 = daily.iter().rev().take(7).map(|d| d.cost).sum();
    let today_cost: f64 = daily.last().map(|d| d.cost).unwrap_or(0.0);

    Json(serde_json::json!({
        "status": "ok",
        "summary": {
            "total_30d_usd": total_30d,
            "total_7d_usd": total_7d,
            "today_usd": today_cost,
        },
        "daily": daily_json,
        "by_label": label_json,
        "by_model": model_json,
    }))
}

/// GET /cost-dashboard — Self-contained HTML cost analytics dashboard.
async fn handle_cost_dashboard(
    State(state): State<Arc<AdminState>>,
    _headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let daily = state
        .memory
        .query_daily_cost_totals(30)
        .await
        .unwrap_or_default();
    let by_label = state
        .memory
        .query_cost_by_label(7)
        .await
        .unwrap_or_default();
    let by_model = state
        .memory
        .query_cost_by_model(7)
        .await
        .unwrap_or_default();

    let total_30d: f64 = daily.iter().map(|d| d.cost).sum();
    let total_7d: f64 = daily.iter().rev().take(7).map(|d| d.cost).sum();
    let today_cost: f64 = daily.last().map(|d| d.cost).unwrap_or(0.0);
    let total_calls_30d: i64 = daily.iter().map(|d| d.calls).sum();

    // Budget settings
    let budget_daily: f64 = state
        .settings
        .get("budget_daily_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(5.0);
    let budget_weekly: f64 = state
        .settings
        .get("budget_weekly_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(25.0);
    let budget_monthly: f64 = state
        .settings
        .get("budget_monthly_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(80.0);

    // Build cost bar chart data
    let max_daily_cost = daily.iter().map(|d| d.cost).fold(0.0f64, f64::max);
    let bar_chart_rows: String = daily
        .iter()
        .map(|d| {
            let pct = if max_daily_cost > 0.0 {
                (d.cost / max_daily_cost * 100.0).min(100.0)
            } else {
                0.0
            };
            let short_date = d.date.get(5..).unwrap_or(&d.date);
            format!(
                "<div class=\"bar-row\"><span class=\"bar-label\">{}</span>\
                 <div class=\"bar-track\"><div class=\"bar-fill\" style=\"width:{:.0}%\"></div></div>\
                 <span class=\"bar-val\">${:.4}</span></div>",
                short_date, pct, d.cost
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Feature cost table
    let label_rows: String = by_label
        .iter()
        .map(|l| {
            format!(
                "<tr><td>{}</td><td>${:.4}</td><td>{}</td><td>{:.0}ms</td></tr>",
                l.label, l.total_cost, l.call_count, l.avg_latency_ms
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Model cost table
    let model_rows: String = by_model
        .iter()
        .map(|m| {
            let total_tokens = m.prompt_tokens + m.completion_tokens;
            format!(
                "<tr><td>{}</td><td>${:.4}</td><td>{}K</td><td>{}K</td></tr>",
                m.model,
                m.total_cost,
                m.prompt_tokens / 1000,
                total_tokens / 1000
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Budget status bars
    let daily_pct = if budget_daily > 0.0 {
        (today_cost / budget_daily * 100.0).min(100.0)
    } else {
        0.0
    };
    let weekly_pct = if budget_weekly > 0.0 {
        (total_7d / budget_weekly * 100.0).min(100.0)
    } else {
        0.0
    };
    let monthly_pct = if budget_monthly > 0.0 {
        (total_30d / budget_monthly * 100.0).min(100.0)
    } else {
        0.0
    };

    fn budget_color(pct: f64) -> &'static str {
        if pct >= 100.0 {
            "#f85149"
        } else if pct >= 80.0 {
            "#d29922"
        } else {
            "#56d364"
        }
    }

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Aegis Runtime Cost Dashboard</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0d1117; color: #c9d1d9; padding: 16px; max-width: 960px; margin: 0 auto; }}
  h1 {{ font-size: 1.4em; margin-bottom: 16px; color: #58a6ff; }}
  h2 {{ font-size: 1.1em; margin: 20px 0 10px; color: #8b949e; }}
  .stats {{ display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap; }}
  .stat {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px 16px; min-width: 120px; flex: 1; }}
  .stat-value {{ font-size: 1.6em; font-weight: 700; color: #58a6ff; }}
  .stat-label {{ font-size: 0.85em; color: #8b949e; margin-top: 4px; }}
  table {{ width: 100%; border-collapse: collapse; background: #161b22; border-radius: 8px; overflow: hidden; margin-bottom: 16px; }}
  th {{ background: #1c2128; text-align: left; padding: 8px 12px; font-size: 0.85em; color: #8b949e; }}
  td {{ padding: 8px 12px; border-top: 1px solid #21262d; font-size: 0.9em; }}
  .bar-row {{ display: flex; align-items: center; gap: 8px; margin-bottom: 3px; }}
  .bar-label {{ width: 50px; font-size: 0.75em; color: #8b949e; text-align: right; flex-shrink: 0; }}
  .bar-track {{ flex: 1; height: 14px; background: #21262d; border-radius: 4px; overflow: hidden; }}
  .bar-fill {{ height: 100%; background: #58a6ff; border-radius: 4px; min-width: 1px; }}
  .bar-val {{ width: 70px; font-size: 0.75em; color: #c9d1d9; font-family: monospace; }}
  .chart {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px 16px; margin-bottom: 16px; max-height: 400px; overflow-y: auto; }}
  .budget {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px 16px; margin-bottom: 16px; }}
  .budget-row {{ display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }}
  .budget-label {{ width: 90px; font-size: 0.85em; color: #8b949e; }}
  .budget-track {{ flex: 1; height: 18px; background: #21262d; border-radius: 6px; overflow: hidden; }}
  .budget-fill {{ height: 100%; border-radius: 6px; transition: width 0.3s; }}
  .budget-val {{ width: 120px; font-size: 0.8em; color: #c9d1d9; font-family: monospace; text-align: right; }}
  .toolbar {{ display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }}
  .refresh-btn {{ background: #21262d; border: 1px solid #30363d; color: #c9d1d9; padding: 6px 14px; border-radius: 6px; cursor: pointer; font-size: 0.85em; }}
  .refresh-btn:hover {{ background: #30363d; }}
</style>
</head>
<body>
<div class="toolbar">
  <h1>API Cost Dashboard</h1>
  <button class="refresh-btn" onclick="location.reload()">Refresh</button>
</div>

<div class="stats">
  <div class="stat"><div class="stat-value">${total_30d:.4}</div><div class="stat-label">30-Day Total</div></div>
  <div class="stat"><div class="stat-value">${total_7d:.4}</div><div class="stat-label">7-Day Total</div></div>
  <div class="stat"><div class="stat-value">${today_cost:.4}</div><div class="stat-label">Today</div></div>
  <div class="stat"><div class="stat-value">{total_calls_30d}</div><div class="stat-label">Calls (30d)</div></div>
</div>

<h2>Budget Status</h2>
<div class="budget">
  <div class="budget-row">
    <span class="budget-label">Daily</span>
    <div class="budget-track"><div class="budget-fill" style="width:{daily_pct:.0}%;background:{daily_color}"></div></div>
    <span class="budget-val">${today_cost:.4} / ${budget_daily:.2}</span>
  </div>
  <div class="budget-row">
    <span class="budget-label">Weekly</span>
    <div class="budget-track"><div class="budget-fill" style="width:{weekly_pct:.0}%;background:{weekly_color}"></div></div>
    <span class="budget-val">${total_7d:.4} / ${budget_weekly:.2}</span>
  </div>
  <div class="budget-row">
    <span class="budget-label">Monthly</span>
    <div class="budget-track"><div class="budget-fill" style="width:{monthly_pct:.0}%;background:{monthly_color}"></div></div>
    <span class="budget-val">${total_30d:.4} / ${budget_monthly:.2}</span>
  </div>
</div>

<h2>30-Day Cost Trend</h2>
<div class="chart">
{bar_chart_rows}
</div>

<h2>Feature Cost (7 days)</h2>
<table>
  <thead><tr><th>Label</th><th>Cost</th><th>Calls</th><th>Avg Latency</th></tr></thead>
  <tbody>{label_rows}</tbody>
</table>

<h2>Model Cost (7 days)</h2>
<table>
  <thead><tr><th>Model</th><th>Cost</th><th>Prompt Tok</th><th>Total Tok</th></tr></thead>
  <tbody>{model_rows}</tbody>
</table>

</body>
</html>"##,
        total_30d = total_30d,
        total_7d = total_7d,
        today_cost = today_cost,
        total_calls_30d = total_calls_30d,
        budget_daily = budget_daily,
        budget_weekly = budget_weekly,
        budget_monthly = budget_monthly,
        daily_pct = daily_pct,
        weekly_pct = weekly_pct,
        monthly_pct = monthly_pct,
        daily_color = budget_color(daily_pct),
        weekly_color = budget_color(weekly_pct),
        monthly_color = budget_color(monthly_pct),
        bar_chart_rows = bar_chart_rows,
        label_rows = label_rows,
        model_rows = model_rows,
    );

    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate",
        )],
        axum::response::Html(html),
    )
        .into_response()
}

/// Build the axum router for the admin control panel.
///
/// Public routes (no auth): `/health`, `/system_metrics`.
/// - The admin API binds to `127.0.0.1` only (localhost), so public ≠ internet-facing.
/// - `/system_metrics` must be public because WASM skills fetch it via
///   `host_fetch` which cannot inject auth headers. It only exposes
///   CPU/memory/disk stats (same data readable from /proc).
///
/// Protected routes (require `Authorization: Bearer <ADMIN_API_TOKEN>`):
/// `/skills`, `/memory_status`.
pub fn admin_router(state: Arc<AdminState>) -> Router {
    // Protected routes — require bearer token
    let protected = Router::new()
        .route("/skills", get(list_skills))
        .route("/memory_status", get(memory_status))
        .route("/reminders", get(list_pending_reminders))
        .route("/l2/{did}", get(list_l2_facts))
        .route("/l2/{did}/{id}", delete(delete_l2_fact))
        .route("/settings", get(list_settings))
        .route("/settings/{key}", put(update_setting).delete(reset_setting))
        .route("/diagnostics/llm_fallback_drill", post(llm_fallback_drill))
        .route("/diagnostics/settings_audit", get(settings_audit))
        .route("/diagnostics/secure_cu", post(secure_cu_smoke))
        .route("/diagnostics/jev_non_cu", post(jev_non_cu_smoke))
        .route("/skills/registry", get(list_skill_registry))
        .route(
            "/skills/registry/{id}",
            get(get_skill_registry_entry).delete(delete_skill_registry_entry),
        )
        .route(
            "/skills/registry/{id}/deactivate",
            post(deactivate_skill_registry_entry),
        )
        .route("/auth/google/setup", post(google_auth_setup))
        .route("/auth/google/store", post(google_auth_store))
        .route("/auth/google/status", get(google_auth_status))
        .route("/auth/google/revoke", post(google_auth_revoke))
        .route("/heartbeat-audit", get(handle_heartbeat_audit))
        .route("/heartbeat-dashboard", get(handle_heartbeat_dashboard))
        .route("/cost-dashboard", get(handle_cost_dashboard))
        .route("/api/cost-summary", get(handle_cost_summary_api))
        .route("/webapp-debug-snapshots", get(list_webapp_debug_snapshots))
        .route(
            "/webapp-debug-snapshots/{snapshot_id}",
            get(get_webapp_debug_snapshot).delete(delete_webapp_debug_snapshot),
        )
        .route("/webapp-debug-leases", get(list_webapp_debug_leases))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // Public routes — no auth required.
    // Accessible only from localhost (127.0.0.1 binding).
    let public = Router::new()
        .route("/health", get(health))
        .route("/health/memory", get(health_memory))
        .route("/system_metrics", get(system_metrics));

    public.merge(protected).with_state(state)
}

/// Where the console bundle (`webapp/dist`) was looked for and what won.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebappDistResolution {
    /// Directory mounted at `/ui/`: the first candidate holding an
    /// `index.html`, or the first candidate at all when none does.
    pub dir: PathBuf,
    /// Whether `dir` actually holds an `index.html`.
    pub found: bool,
    /// Every candidate considered, in order.
    pub tried: Vec<PathBuf>,
}

/// Ordered places the console bundle may live.
///
/// `AEGIS_WEBAPP_DIR` (`env_override`) wins outright when set. Otherwise:
/// `./webapp/dist` (a checkout run from the repo root), the two
/// binary-relative layouts (`target/<profile>/` inside a checkout, and an
/// install that ships `webapp/dist` next to the binary), and last the cargo
/// manifest, which is only set under `cargo run`/`cargo test` — a release
/// binary never sees it, which is why it used to 404 silently.
pub fn webapp_dist_candidates(
    env_override: Option<&str>,
    exe_dir: Option<&FsPath>,
    manifest_dir: Option<&str>,
) -> Vec<PathBuf> {
    if let Some(dir) = env_override.map(str::trim).filter(|dir| !dir.is_empty()) {
        return vec![PathBuf::from(dir)];
    }
    let mut candidates = vec![PathBuf::from("./webapp/dist")];
    if let Some(exe_dir) = exe_dir {
        candidates.push(exe_dir.join("../../webapp/dist"));
        candidates.push(exe_dir.join("webapp/dist"));
    }
    if let Some(manifest) = manifest_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        candidates.push(FsPath::new(manifest).join("../webapp/dist"));
    }
    candidates
}

/// Pick the first candidate that has an `index.html`; when none does, fall
/// back to the first candidate flagged `found: false` so the router still
/// mounts something and the caller can warn. `has_index` is injected so the
/// choice is unit-testable without a filesystem.
pub fn choose_webapp_dist_dir(
    candidates: &[PathBuf],
    has_index: impl Fn(&FsPath) -> bool,
) -> WebappDistResolution {
    let hit = candidates.iter().find(|dir| has_index(dir));
    WebappDistResolution {
        dir: hit
            .or_else(|| candidates.first())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("./webapp/dist")),
        found: hit.is_some(),
        tried: candidates.to_vec(),
    }
}

/// Resolve the bundle for this process and log the outcome. A miss is a WARN
/// naming every path tried and the fix, because the symptom otherwise is a
/// silent 404 on /ui/ under an INFO line claiming the console is served.
fn resolve_webapp_dist_dir() -> WebappDistResolution {
    let env_override = std::env::var("AEGIS_WEBAPP_DIR").ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(FsPath::to_path_buf));
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok();
    let candidates = webapp_dist_candidates(
        env_override.as_deref(),
        exe_dir.as_deref(),
        manifest_dir.as_deref(),
    );
    let resolution = choose_webapp_dist_dir(&candidates, |dir| dir.join("index.html").is_file());
    if resolution.found {
        tracing::info!(
            webapp_dir = %resolution.dir.display(),
            "Serving WebApp static files from /ui/"
        );
    } else {
        let tried = resolution
            .tried
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            webapp_dir = %resolution.dir.display(),
            tried = %tried,
            "Web Console bundle not found (no index.html in any candidate); /ui/ will answer 404. \
             Build it with `cd webapp && npm ci && npm run build`, or point AEGIS_WEBAPP_DIR at a built bundle"
        );
    }
    resolution
}

/// Build a minimal public router for the OAuth2 callback endpoint and WebApp API.
///
/// This router is intended to be served on a **public** address (0.0.0.0)
/// separate from the localhost-only admin API. It exposes:
/// 1. Google OAuth2 redirect callback
/// 2. WebApp API for Telegram Mini Apps (schema fetch, data submission)
pub fn oauth_callback_router(state: Arc<AdminState>) -> Router {
    let webapp_dir = resolve_webapp_dist_dir().dir;

    let router = Router::new()
        .merge(super::remote_routes::router())
        .route("/auth/callback/google", get(google_oauth_public_callback))
        .route("/auth/login/google", get(web_console_google_login_start))
        .route("/auth/login/local", get(web_console_local_login))
        .route("/api/session", get(web_console_session))
        .route("/api/logout", post(web_console_logout))
        .route("/mfa/status", get(web_console_mfa_status))
        .route(
            "/mfa/webauthn/credentials",
            get(web_console_webauthn_credentials),
        )
        .route(
            "/mfa/webauthn/register/start",
            post(web_console_pending_mfa_endpoint),
        )
        .route(
            "/mfa/webauthn/register/finish",
            post(web_console_pending_mfa_endpoint),
        )
        .route(
            "/mfa/webauthn/assert/start",
            post(web_console_pending_mfa_endpoint),
        )
        .route(
            "/mfa/webauthn/assert/finish",
            post(web_console_pending_mfa_endpoint),
        )
        .route("/mfa/totp/setup/start", post(web_console_totp_setup_start))
        .route(
            "/mfa/totp/setup/finish",
            post(web_console_totp_setup_finish),
        )
        .route("/mfa/totp/verify", post(web_console_totp_verify))
        .route("/mfa/host-confirm", post(web_console_host_confirm))
        .route(
            "/mfa/recovery-code/consume",
            post(web_console_pending_mfa_endpoint),
        )
        .route(
            "/mfa/recovery-codes/regenerate",
            post(web_console_pending_mfa_endpoint),
        )
        .route("/mfa/disable", post(web_console_pending_mfa_endpoint))
        // Owner-facing WebApp API.
        .route("/webapp/vnext/home", get(webapp_vnext_home))
        .route("/webapp/vnext/work", get(webapp_vnext_work))
        .route("/webapp/vnext/work/{item_id}", get(webapp_vnext_item))
        .route("/webapp/vnext/chat-history", get(webapp_vnext_chat_history))
        .route(
            "/webapp/vnext/avatar-profile",
            get(webapp_vnext_avatar_profile).put(webapp_vnext_update_avatar_profile),
        )
        .route(
            "/webapp/vnext/avatar-profile/auto-refresh",
            put(webapp_vnext_update_avatar_auto_refresh),
        )
        .route(
            "/webapp/vnext/avatar-profile/generate-aegis",
            post(webapp_vnext_generate_aegis_avatar),
        )
        .route(
            "/webapp/vnext/avatar-image/{filename}",
            get(webapp_vnext_avatar_image),
        )
        .route(
            "/webapp/vnext/generated-image/{filename}",
            get(webapp_vnext_generated_image),
        )
        .route("/webapp/vnext/command", post(webapp_vnext_command))
        .route(
            "/webapp/vnext/research/stop",
            post(webapp_vnext_research_stop),
        )
        // Slice 5: multimodal upload — accepts one multipart file, validates MIME
        // by header + magic-number, caps size, base64-encodes in-memory, returns a
        // data_url descriptor for the composer to attach to a subsequent command.
        // The raw-body cap is enforced in the axum body extractor (DefaultBodyLimit),
        // BEFORE the bytes are buffered, so an oversized upload is rejected at the
        // transport layer (not after a full in-memory read).
        .route(
            "/webapp/vnext/upload",
            post(webapp_vnext_upload).layer(DefaultBodyLimit::max(VNEXT_UPLOAD_MAX_BODY_BYTES)),
        )
        .route(
            "/webapp/vnext/command/status",
            get(webapp_vnext_command_status),
        )
        // Slice 2: per-turn progress SSE — mint a short-lived chat-bound token
        // (console auth + CSRF), then open the EventSource with it.
        .route(
            "/webapp/vnext/turns/{turn_id}/stream-token",
            post(webapp_vnext_turn_stream_token),
        )
        .route(
            "/webapp/vnext/turns/{turn_id}/events",
            get(webapp_vnext_turn_events),
        )
        .route(
            "/webapp/vnext/work/{item_id}/actions",
            post(webapp_vnext_work_action),
        )
        .route("/webapp/vnext/status", get(webapp_vnext_status))
        .route("/webapp/vnext/settings", get(webapp_vnext_settings))
        .route(
            "/webapp/vnext/model-catalog",
            get(webapp_vnext_model_catalog),
        )
        .route("/webapp/vnext/presence", post(webapp_vnext_presence))
        .route(
            "/webapp/vnext/voice/session",
            post(webapp_vnext_voice_session),
        )
        .route("/webapp/vnext/voice/status", get(webapp_vnext_voice_status))
        .route(
            "/webapp/vnext/settings/web-push/status",
            get(webapp_vnext_web_push_status),
        )
        .route(
            "/webapp/vnext/settings/web-push/subscribe",
            post(webapp_vnext_web_push_subscribe),
        )
        .route(
            "/webapp/vnext/settings/web-push/unsubscribe",
            post(webapp_vnext_web_push_unsubscribe),
        )
        .route(
            "/webapp/vnext/settings/web-push/test",
            post(webapp_vnext_web_push_test),
        )
        .route(
            "/webapp/vnext/settings/codex/status",
            get(webapp_vnext_codex_status),
        )
        .route(
            "/webapp/vnext/settings/codex/start-device-code",
            post(webapp_vnext_codex_start_device_code),
        )
        .route(
            "/webapp/vnext/settings/codex/poll-device-code",
            post(webapp_vnext_codex_poll_device_code),
        )
        .route(
            "/webapp/vnext/settings/codex/use-for-chat",
            post(webapp_vnext_codex_use_for_chat),
        )
        .route(
            "/webapp/vnext/settings/codex/model",
            post(webapp_vnext_codex_set_model),
        )
        .route(
            "/webapp/vnext/settings/codex/stop-using-for-chat",
            post(webapp_vnext_codex_stop_using_for_chat),
        )
        .route(
            "/webapp/vnext/settings/grok/status",
            get(webapp_vnext_grok_status),
        )
        .route(
            "/webapp/vnext/settings/grok/start-device-code",
            post(webapp_vnext_grok_start_device_code),
        )
        .route(
            "/webapp/vnext/settings/grok/poll-device-code",
            post(webapp_vnext_grok_poll_device_code),
        )
        .route(
            "/webapp/vnext/settings/grok/use-for-chat",
            post(webapp_vnext_grok_use_for_chat),
        )
        .route(
            "/webapp/vnext/settings/grok/stop-using-for-chat",
            post(webapp_vnext_grok_stop_using_for_chat),
        )
        .route(
            "/webapp/vnext/settings/grok/revoke",
            post(webapp_vnext_grok_revoke),
        )
        .route(
            "/webapp/vnext/settings/cost-saving/pause",
            post(webapp_vnext_pause_cost_saving),
        )
        .route(
            "/webapp/vnext/settings/cost-saving/resume",
            post(webapp_vnext_resume_cost_saving),
        )
        .route(
            "/webapp/vnext/settings/display-language",
            post(webapp_vnext_set_display_language),
        )
        .route(
            "/webapp/vnext/settings/llm-provider",
            post(webapp_vnext_set_llm_provider),
        )
        .route(
            "/webapp/vnext/settings/jev",
            get(webapp_vnext_jev_status).post(webapp_vnext_set_jev),
        )
        .route(
            "/webapp/vnext/settings/llm-model",
            post(webapp_vnext_set_llm_model),
        )
        .route(
            "/webapp/vnext/settings/fallback-model",
            post(webapp_vnext_set_fallback_model),
        )
        .route(
            "/webapp/vnext/settings/paid-api-fallback",
            post(webapp_vnext_set_paid_api_fallback),
        )
        .route(
            "/webapp/vnext/settings/web-search",
            post(webapp_vnext_set_web_search),
        )
        .route(
            "/webapp/vnext/debug-snapshots",
            post(webapp_vnext_create_debug_snapshot),
        )
        .route(
            "/webapp/vnext/debug-leases",
            get(webapp_vnext_debug_leases).post(webapp_vnext_create_debug_lease),
        )
        .route(
            "/webapp/vnext/debug-leases/{lease_id}",
            delete(webapp_vnext_revoke_debug_lease),
        )
        .route(
            "/webapp/vnext/debug-jobs",
            get(webapp_vnext_debug_jobs).post(webapp_vnext_create_debug_job),
        )
        .route(
            "/webapp/vnext/debug-jobs/{job_id}",
            get(webapp_vnext_debug_job),
        )
        .route("/webapp/owner-work/queue", get(webapp_assistant_inbox))
        .route("/webapp/owner-work/context", post(webapp_assistant_context))
        .route(
            "/webapp/console/memory/items/{item_id}",
            get(webapp_console_memory_item),
        )
        .route(
            "/webapp/console/memory/profile/{category}/{key}",
            get(webapp_console_profile_slot),
        )
        .route(
            "/webapp/console/memory/profile/{category}/{key}/update",
            post(webapp_console_profile_slot_update),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/delete",
            post(webapp_console_memory_item_delete),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/update",
            post(webapp_console_memory_item_update),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/confirm",
            post(webapp_console_memory_item_confirm),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/pin",
            post(webapp_console_memory_item_pin),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/unpin",
            post(webapp_console_memory_item_unpin),
        )
        .route(
            "/webapp/console/memory/items/{item_id}/mark-wrong",
            post(webapp_console_memory_item_mark_wrong),
        )
        .route(
            "/webapp/console/tasks/{task_id}/timeline",
            get(webapp_console_task_timeline),
        )
        .route(
            "/webapp/console/tasks/{task_id}/action",
            post(webapp_console_task_action),
        )
        .route(
            "/webapp/console/ledger/{entry_id}",
            get(webapp_console_ledger_entry),
        )
        .route(
            "/webapp/console/approvals/{approval_id}/approve",
            post(webapp_console_approval_approve),
        )
        .route(
            "/webapp/console/approvals/{approval_id}/reject",
            post(webapp_console_approval_reject),
        )
        .route(
            "/webapp/console/mandates",
            post(webapp_console_mandate_create),
        )
        .route(
            "/webapp/console/mandates/{mandate_id}/edit",
            post(webapp_console_mandate_edit),
        )
        .route(
            "/webapp/console/mandates/{mandate_id}/activate",
            post(webapp_console_mandate_activate),
        )
        .route(
            "/webapp/console/mandates/{mandate_id}/pause",
            post(webapp_console_mandate_pause),
        )
        .route(
            "/webapp/console/mandates/{mandate_id}/resume",
            post(webapp_console_mandate_resume),
        )
        .route(
            "/webapp/console/mandates/{mandate_id}/revoke",
            post(webapp_console_mandate_revoke),
        )
        // Bare domain often bookmarked — never leave owners on HTTP 404.
        .route("/", get(|| async { Redirect::temporary("/ui/") }))
        // Serve WebApp frontend static files at /ui/*
        .nest_service(
            "/ui",
            ServeDir::new(&webapp_dir).append_index_html_on_directories(true),
        );

    router
        .layer(middleware::from_fn(webapp_security_headers))
        .with_state(state)
}

/// Query parameters for the Google OAuth2 callback.
#[derive(serde::Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Query parameters for the Web Console Google Login callback.
#[derive(serde::Deserialize)]
struct WebConsoleGoogleLoginCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Query parameters for the local login link.
#[derive(serde::Deserialize)]
struct LocalLoginQuery {
    token: Option<String>,
}

/// Fixed delay before answering a bad local login token: keeps the link from
/// being brute-forced at wire speed and gives a miss no timing tell.
const LOCAL_LOGIN_REJECT_DELAY: Duration = Duration::from_millis(300);

/// GET /auth/login/local?token=… — owner login for installs with neither
/// Google Login nor Telegram (the README on-ramp).
///
/// Enabled only when `AdminState.local_login_token` is set (policy in
/// `web_console_auth::local_login_token_from_lookup`); otherwise the route
/// answers 404 like any unknown path. The presented token is compared in
/// constant time; a wrong or missing one gets a plain 401 after a fixed
/// delay. A match mints a local owner session (`create_local_session`, the
/// only allowlist bypass) and redirects to the console with the session
/// cookie, exactly as the Google callback does.
async fn web_console_local_login(
    State(state): State<Arc<AdminState>>,
    Query(query): Query<LocalLoginQuery>,
) -> Response {
    // A loopback reverse proxy is not proof of a local user. Remote deployments
    // use host-issued pairing and Passkeys, never the reusable local login link.
    if state.public_url.is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(expected) = state.local_login_token.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let presented = query.token.as_deref().unwrap_or("");
    if presented.is_empty() || !constant_time_eq(presented, expected) {
        tokio::time::sleep(LOCAL_LOGIN_REJECT_DELAY).await;
        tracing::warn!("Web Console local login rejected (token mismatch)");
        return (StatusCode::UNAUTHORIZED, "Invalid local login token").into_response();
    }
    let Some(db_path) = state.db_path.clone() else {
        tracing::warn!("Web Console local login refused: no session database configured");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Session store is not configured",
        )
            .into_response();
    };

    let config = web_console_auth::WebConsoleLoginConfig::from_env();
    let cookie_config = config.clone();
    let created = match tokio::task::spawn_blocking(
        move || -> anyhow::Result<web_console_auth::CreatedSession> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            let now = chrono::Utc::now().timestamp().max(0) as u64;
            web_console_auth::create_local_session(&conn, &config, now)
        },
    )
    .await
    {
        Ok(Ok(created)) => created,
        Ok(Err(e)) => return local_login_failure(&e),
        Err(e) => return local_login_failure(&e),
    };
    let cookie =
        match web_console_auth::build_session_cookie_header(&cookie_config, &created.cookie_token)
            .and_then(|cookie| {
                axum::http::HeaderValue::from_str(&cookie).map_err(anyhow::Error::from)
            }) {
            Ok(cookie) => cookie,
            Err(e) => return local_login_failure(&e),
        };
    tracing::info!("Web Console local login succeeded; owner session issued");
    let mut response = Redirect::to("/ui/").into_response();
    response
        .headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie);
    response
}

fn local_login_failure(error: &dyn std::fmt::Display) -> Response {
    tracing::warn!(error = %error, "Web Console local session creation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to create local session",
    )
        .into_response()
}

/// Query parameters for starting Web Console Google Login.
#[derive(serde::Deserialize)]
struct WebConsoleGoogleLoginStartQuery {
    return_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SharedGoogleOAuthClient {
    pub(super) client_id: String,
    pub(super) client_secret: String,
}

pub(super) fn load_shared_google_oauth_client_from_store(
    db_path: &str,
    aes_key: [u8; 32],
) -> anyhow::Result<Option<SharedGoogleOAuthClient>> {
    let store = CredentialStore::open(crate::encrypted_sqlite::open(db_path)?, aes_key)?;
    let Some(client_id) = store.get("google_oauth", "client_id")? else {
        return Ok(None);
    };
    let Some(client_secret) = store.get("google_oauth", "client_secret")? else {
        return Ok(None);
    };
    let client_id = client_id.trim().to_string();
    let client_secret = client_secret.trim().to_string();
    if client_id.is_empty() || client_secret.is_empty() {
        return Ok(None);
    }
    Ok(Some(SharedGoogleOAuthClient {
        client_id,
        client_secret,
    }))
}

pub(super) fn apply_shared_google_oauth_client_to_login_config(
    config: &mut web_console_auth::WebConsoleLoginConfig,
    client: &SharedGoogleOAuthClient,
    client_secret: &mut Option<String>,
) {
    if config.client_id.is_empty() {
        config.client_id = client.client_id.clone();
    }
    if config.client_id == client.client_id && client_secret.is_none() {
        *client_secret = Some(client.client_secret.clone());
    }
    if config.client_id == client.client_id && client_secret.is_some() {
        config.client_secret_present = true;
    }
}

async fn web_console_google_login_config_and_secret(
    state: &AdminState,
) -> Result<(web_console_auth::WebConsoleLoginConfig, Option<String>), (StatusCode, String)> {
    let mut config = web_console_auth::WebConsoleLoginConfig::from_env();
    let mut client_secret = web_console_auth::google_login_client_secret_from_env();
    // Disabled is authoritative even when Workspace credentials already exist.
    if !config.enabled {
        return Ok((config, client_secret));
    }
    if (config.client_id.is_empty() || client_secret.is_none())
        && let (Some(db_path), Some(aes_key)) =
            (&state.credential_db_path, state.credential_aes_key)
    {
        let db_path = db_path.clone();
        let shared_client = tokio::task::spawn_blocking(move || {
            load_shared_google_oauth_client_from_store(&db_path, aes_key)
        })
        .await
        .map_err(|e| {
            tracing::warn!(
                error = %e,
                "Shared Google OAuth client lookup task failed for Web Console Google Login"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Workbench Google Login is not configured".to_string(),
            )
        })?
        .map_err(|e| {
            tracing::warn!(
                error = %e,
                "Shared Google OAuth client lookup failed for Web Console Google Login"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Workbench Google Login is not configured".to_string(),
            )
        })?;

        if let Some(shared_client) = shared_client {
            apply_shared_google_oauth_client_to_login_config(
                &mut config,
                &shared_client,
                &mut client_secret,
            );
        }
    }
    Ok((config, client_secret))
}

/// GET /auth/login/google — Start standalone Web Console Google Login.
async fn web_console_google_login_start(
    State(state): State<Arc<AdminState>>,
    axum::extract::Query(query): axum::extract::Query<WebConsoleGoogleLoginStartQuery>,
) -> Result<Response, (StatusCode, String)> {
    let (config, _) = web_console_google_login_config_and_secret(&state).await?;
    let public_url = state.public_url.as_deref().ok_or_else(|| {
        tracing::warn!("Web Console Google Login requested without AEGIS_PUBLIC_URL");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench Google Login is not configured".to_string(),
        )
    })?;

    let return_to = web_console_auth::sanitize_login_return_to(query.return_to.as_deref());
    let login =
        web_console_auth::build_google_login_url(&config, public_url, return_to).map_err(|e| {
            tracing::warn!(
                error = %e,
                "Web Console Google Login requested but configuration is incomplete"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Workbench Google Login is not configured".to_string(),
            )
        })?;

    let location = axum::http::HeaderValue::from_str(&login.url).map_err(|e| {
        tracing::warn!(error = %e, "Failed to construct Google Login redirect header");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to start Google Login".to_string(),
        )
    })?;

    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert(axum::http::header::LOCATION, location);
    Ok(response)
}

async fn handle_web_console_google_login_callback(
    state: Arc<AdminState>,
    query: WebConsoleGoogleLoginCallbackQuery,
) -> Result<Response, (StatusCode, String)> {
    if query.error.is_some() {
        tracing::warn!("Web Console Google Login returned an OAuth error");
        return Err((StatusCode::BAD_REQUEST, "Google Login failed".to_string()));
    }
    let state_raw = query
        .state
        .as_deref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing state".to_string()))?;
    if query.code.as_deref().unwrap_or_default().trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing authorization code".to_string(),
        ));
    }

    let (config, client_secret) = web_console_google_login_config_and_secret(&state).await?;
    config.validate_for_login().map_err(|e| {
        tracing::warn!(error = %e, "Web Console Google Login callback is not configured");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench Google Login is not configured".to_string(),
        )
    })?;
    let client_secret = client_secret.ok_or_else(|| {
        tracing::warn!("Web Console Google Login client secret is missing");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench Google Login is not configured".to_string(),
        )
    })?;
    let db_path = state.db_path.clone().ok_or_else(|| {
        tracing::warn!("Web Console Google Login callback requested without db_path");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench session storage is not configured".to_string(),
        )
    })?;

    let pending = web_console_auth::consume_pending_login_state(state_raw).map_err(|e| {
        tracing::warn!(error = %e, "Web Console Google Login state validation failed");
        (
            StatusCode::BAD_REQUEST,
            "Invalid or expired state".to_string(),
        )
    })?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to build Web Console Google Login HTTP client");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to verify Google Login".to_string(),
            )
        })?;
    let code = query.code.as_deref().unwrap_or_default();
    let token_response = web_console_auth::exchange_google_login_code(
        &http,
        &config,
        &client_secret,
        &pending,
        code,
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console Google Login token exchange failed");
        (
            StatusCode::BAD_GATEWAY,
            "Failed to verify Google Login".to_string(),
        )
    })?;
    let jwks = web_console_auth::fetch_google_oidc_jwks(&http)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console Google Login JWKS fetch failed");
            (
                StatusCode::BAD_GATEWAY,
                "Failed to verify Google Login".to_string(),
            )
        })?;
    let claims = web_console_auth::verify_google_id_token(
        &token_response.id_token,
        &jwks,
        &config,
        &pending.nonce,
    )
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console Google Login ID token verification failed");
        (
            StatusCode::UNAUTHORIZED,
            "Google Login identity is not allowed".to_string(),
        )
    })?;

    let cookie_config = config.clone();
    let created = tokio::task::spawn_blocking(
        move || -> anyhow::Result<web_console_auth::CreatedSession> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            let now = chrono::Utc::now().timestamp() as u64;
            let mfa_evidence = web_console_auth::google_login_mfa_evidence(
                &claims,
                now,
                crate::data_access_gate::DEFAULT_MFA_FRESHNESS_SECS,
            );
            tracing::info!(
                google_claim_has_auth_time = claims.auth_time.is_some(),
                google_claim_has_iat = claims.iat.is_some(),
                mfa_method = mfa_evidence
                    .as_ref()
                    .map(|evidence| evidence.method.as_str())
                    .unwrap_or("none"),
                "Web Console Google Login private-access evidence evaluated"
            );
            web_console_auth::create_session_with_mfa_evidence(
                &conn,
                &config,
                &claims.sub,
                &claims.email,
                claims.email_verified,
                now,
                mfa_evidence,
            )
        },
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console session creation task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create Workbench session".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console session creation failed");
        (
            StatusCode::UNAUTHORIZED,
            "Google Login identity is not allowed".to_string(),
        )
    })?;

    let cookie =
        web_console_auth::build_session_cookie_header(&cookie_config, &created.cookie_token)
            .map_err(|e| {
                tracing::warn!(error = %e, "Failed to build Web Console session cookie");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create Workbench session".to_string(),
                )
            })?;
    let cookie_header = axum::http::HeaderValue::from_str(&cookie).map_err(|e| {
        tracing::warn!(error = %e, "Failed to construct Web Console Set-Cookie header");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create Workbench session".to_string(),
        )
    })?;
    let location = axum::http::HeaderValue::from_str(
        pending.return_to.as_deref().unwrap_or("/ui/"),
    )
    .map_err(|e| {
        tracing::warn!(error = %e, "Failed to construct Web Console return location");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create Workbench session".to_string(),
        )
    })?;
    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie_header);
    response
        .headers_mut()
        .insert(axum::http::header::LOCATION, location);
    Ok(response)
}

/// GET /api/session — Safe current Web Console session metadata.
///
/// Phase 1 returns an explicit unauthenticated state. Later session phases
/// replace this with cookie validation while preserving this no-secrets shape.
async fn web_console_session(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Json<serde_json::Value> {
    let google_login_enabled = if state.public_url.is_some()
        && web_console_auth::WebConsoleLoginConfig::from_env().enabled
    {
        web_console_google_login_config_and_secret(&state)
            .await
            .is_ok_and(|(config, _)| config.validate_for_login().is_ok())
    } else {
        false
    };
    let passkey_login_enabled = state.public_url.is_some();
    if req
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("TelegramWebApp "))
    {
        return match extract_webapp_auth(&state, req.headers()) {
            Ok(verified) => Json(serde_json::json!({
                "authenticated": true,
                "google_login_enabled": google_login_enabled,
                "passkey_login_enabled": passkey_login_enabled,
                "principal_type": "telegram",
                "assistant_name": state
                    .settings
                    .assistant_display_name(&state.settings.display_language()),
                "mfa_level": "none",
                "mfa_verified_at": null,
                "mfa_expires_at": null,
                "mfa_freshness_secs": WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS,
                "protected_access_unlocked": false,
                "requires_mfa_setup": false,
                "telegram_user_id": verified.user.id,
            })),
            Err((_status, _message)) => Json(serde_json::json!({
                "authenticated": false,
                "google_login_enabled": google_login_enabled,
                "passkey_login_enabled": passkey_login_enabled,
                "requires_mfa_setup": true,
                "protected_access_unlocked": false,
                "mfa_freshness_secs": WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS,
            })),
        };
    }

    let Some(session) = web_console_session_from_request(&state, req.headers()).await else {
        return Json(serde_json::json!({
            "authenticated": false,
            "google_login_enabled": google_login_enabled,
            "passkey_login_enabled": passkey_login_enabled,
            "requires_mfa_setup": true,
            "protected_access_unlocked": false,
            "mfa_freshness_secs": WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS,
        }));
    };
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let mfa_freshness_secs = WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS;
    let mfa_expires_at = protected_access_mfa_expires_at(&session, mfa_freshness_secs);
    let protected_access_unlocked = mfa_expires_at.is_some_and(|expires_at| {
        session
            .mfa_verified_at
            .is_some_and(|verified_at| verified_at <= now)
            && now <= expires_at
    });
    let requires_mfa_setup = web_console_session_requires_mfa_setup(&state, &session).await;

    Json(serde_json::json!({
        "authenticated": true,
        "principal_type": "web_console",
        "google_login_enabled": google_login_enabled,
        "passkey_login_enabled": passkey_login_enabled,
        "email": session.email,
        "mfa_level": session.mfa_level.as_str(),
        "mfa_verified_at": session.mfa_verified_at,
        "mfa_expires_at": mfa_expires_at,
        "mfa_freshness_secs": mfa_freshness_secs,
        "protected_access_unlocked": protected_access_unlocked,
        "requires_mfa_setup": requires_mfa_setup,
        "csrf_token": web_console_auth::csrf_token_for_session(&session),
        "assistant_name": state
            .settings
            .assistant_display_name(&state.settings.display_language()),
    }))
}

async fn web_console_session_requires_mfa_setup(
    state: &AdminState,
    session: &web_console_auth::WebConsoleSession,
) -> bool {
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    if protected_access_mfa_expires_at(session, WEB_CONSOLE_PROTECTED_READ_FRESHNESS_SECS)
        .is_some_and(|expires_at| {
            session
                .mfa_verified_at
                .is_some_and(|verified_at| verified_at <= now)
                && now <= expires_at
        })
    {
        return false;
    }
    let Some(db_path) = state.db_path.clone() else {
        return session.mfa_level == web_console_auth::MfaLevel::GoogleOnly;
    };
    let fallback_requires_setup = session.mfa_level == web_console_auth::MfaLevel::GoogleOnly;
    let session = session.clone();
    match tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        Ok(web_console_auth::get_mfa_status(&conn, &session)?.requires_mfa_setup)
    })
    .await
    {
        Ok(Ok(requires_mfa_setup)) => requires_mfa_setup,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "Web Console session MFA setup lookup failed");
            fallback_requires_setup
        }
        Err(error) => {
            tracing::warn!(error = %error, "Web Console session MFA setup lookup task failed");
            fallback_requires_setup
        }
    }
}

fn protected_access_mfa_expires_at(
    session: &web_console_auth::WebConsoleSession,
    freshness_secs: u64,
) -> Option<u64> {
    match session.mfa_level {
        web_console_auth::MfaLevel::GoogleFresh
        | web_console_auth::MfaLevel::Totp
        | web_console_auth::MfaLevel::WebAuthn => session
            .mfa_verified_at
            .map(|verified_at| verified_at.saturating_add(freshness_secs)),
        web_console_auth::MfaLevel::None
        | web_console_auth::MfaLevel::GoogleOnly
        | web_console_auth::MfaLevel::RecoveryCode
        | web_console_auth::MfaLevel::Host
        | web_console_auth::MfaLevel::HostConfirmed => None,
    }
}

async fn web_console_logout(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Response, (StatusCode, String)> {
    let config = web_console_auth::WebConsoleLoginConfig::from_env();
    if let (Some(db_path), Some(cookie_header)) = (
        state.db_path.as_ref(),
        req.headers()
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    ) && let Some(cookie_token) =
        web_console_auth::extract_cookie_value(cookie_header, &config.session_cookie_name)
    {
        let session = web_console_session_from_request(&state, req.headers())
            .await
            .ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    "Invalid Workbench session".to_string(),
                )
            })?;
        require_web_console_csrf(req.headers(), &session)?;
        let db_path = db_path.clone();
        let revoke_result = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            web_console_auth::revoke_session(
                &conn,
                &cookie_token,
                chrono::Utc::now().timestamp() as u64,
            )
        })
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console logout task failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to log out".to_string(),
            )
        })?;
        if let Err(e) = revoke_result {
            tracing::warn!(error = %e, "Web Console logout revocation failed");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to log out".to_string(),
            ));
        }
    }

    let clear_cookie =
        web_console_auth::build_clear_session_cookie_header(&config).map_err(|e| {
            tracing::warn!(error = %e, "Invalid Web Console session cookie config");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Workbench session is not configured".to_string(),
            )
        })?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    let header = axum::http::HeaderValue::from_str(&clear_cookie).map_err(|e| {
        tracing::warn!(error = %e, "Failed to build logout Set-Cookie header");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to log out".to_string(),
        )
    })?;
    response
        .headers_mut()
        .insert(axum::http::header::SET_COOKIE, header);
    Ok(response)
}

async fn web_console_mfa_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let session = web_console_session_from_request(&state, req.headers())
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    let db_path = state.db_path.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench MFA storage is not configured".to_string(),
        )
    })?;
    let status =
        tokio::task::spawn_blocking(move || -> anyhow::Result<web_console_auth::MfaStatus> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            web_console_auth::get_mfa_status(&conn, &session)
        })
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console MFA status task failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load MFA status".to_string(),
            )
        })?
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console MFA status lookup failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load MFA status".to_string(),
            )
        })?;

    Ok(Json(serde_json::json!({
        "authenticated": true,
        "mfa": status,
        "passkey_login_url": state.public_url.as_ref().map(|_| "/access"),
    })))
}

async fn web_console_webauthn_credentials(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let session = web_console_session_from_request(&state, req.headers())
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    let db_path = state.db_path.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench MFA storage is not configured".to_string(),
        )
    })?;
    let credentials = tokio::task::spawn_blocking(
        move || -> anyhow::Result<Vec<web_console_auth::WebAuthnCredentialSummary>> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            web_console_auth::list_webauthn_credentials(&conn, &session.google_sub)
        },
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console WebAuthn credential listing task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to load WebAuthn credentials".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console WebAuthn credential listing failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to load WebAuthn credentials".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "credentials": credentials,
    })))
}

#[derive(Debug, Deserialize)]
struct TotpCodeRequest {
    code: String,
}

fn web_console_mfa_storage(state: &AdminState) -> Result<(String, [u8; 32]), (StatusCode, String)> {
    match (&state.db_path, &state.credential_aes_key) {
        (Some(db_path), Some(aes_key)) => Ok((db_path.clone(), *aes_key)),
        _ => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Workbench MFA storage is not configured".to_string(),
        )),
    }
}

async fn parse_totp_code_request(
    body: axum::body::Body,
) -> Result<TotpCodeRequest, (StatusCode, String)> {
    let bytes = axum::body::to_bytes(body, 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid TOTP request body".to_string(),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid TOTP request JSON".to_string(),
        )
    })
}

async fn web_console_totp_setup_start(
    State(state): State<Arc<AdminState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let session = web_console_session_from_request(&state, &headers)
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    require_web_console_csrf(&headers, &session)?;
    let (db_path, aes_key) = web_console_mfa_storage(&state)?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let setup =
        tokio::task::spawn_blocking(move || -> anyhow::Result<web_console_auth::TotpSetup> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            web_console_auth::start_totp_setup(&conn, &aes_key, &session, "Aegis Runtime", now)
        })
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console TOTP setup start task failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to start TOTP setup".to_string(),
            )
        })?
        .map_err(|e| {
            tracing::warn!(error = %e, "Web Console TOTP setup start failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to start TOTP setup".to_string(),
            )
        })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "method": "totp",
        "setup": setup,
    })))
}

async fn web_console_totp_setup_finish(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let session = web_console_session_from_request(&state, &headers)
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    require_web_console_csrf(&headers, &session)?;
    let payload = parse_totp_code_request(body).await?;
    let (db_path, aes_key) = web_console_mfa_storage(&state)?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let verified = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        web_console_auth::finish_totp_setup(&conn, &aes_key, &session, &payload.code, now)
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console TOTP setup finish task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to finish TOTP setup".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console TOTP setup finish failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to finish TOTP setup".to_string(),
        )
    })?;
    if !verified {
        return Err((StatusCode::UNAUTHORIZED, "Invalid TOTP code".to_string()));
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "method": "totp",
        "mfa_level": "totp",
    })))
}

async fn web_console_host_confirm(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if state.public_url.is_some() {
        return Err((
            StatusCode::FORBIDDEN,
            "Host confirmation is disabled for remote access; use a Passkey".into(),
        ));
    }
    let session = web_console_session_from_request(&state, req.headers())
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    require_web_console_csrf(req.headers(), &session)?;
    if session.google_sub != web_console_auth::LOCAL_LOGIN_SUB {
        return Err((
            StatusCode::FORBIDDEN,
            "Host confirmation is only for the local host session".to_string(),
        ));
    }
    let db_path = state.db_path.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Session storage is unavailable".to_string(),
        )
    })?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        web_console_auth::confirm_local_host_step_up(&conn, &session, now)
    })
    .await
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console host confirmation task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to record host confirmation".to_string(),
        )
    })?
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console host confirmation failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to record host confirmation".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "method": "local_host_confirm",
        "mfa_level": "host_confirmed",
    })))
}

async fn web_console_totp_verify(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let session = web_console_session_from_request(&state, &headers)
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    require_web_console_csrf(&headers, &session)?;
    let payload = parse_totp_code_request(body).await?;
    let (db_path, aes_key) = web_console_mfa_storage(&state)?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let verified = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        web_console_auth::verify_totp_step_up(&conn, &aes_key, &session, &payload.code, now)
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console TOTP verify task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to verify TOTP".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console TOTP verify failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to verify TOTP".to_string(),
        )
    })?;
    if !verified {
        return Err((StatusCode::UNAUTHORIZED, "Invalid TOTP code".to_string()));
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "method": "totp",
        "mfa_level": "totp",
    })))
}

async fn web_console_pending_mfa_endpoint(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let descriptor = pending_mfa_endpoint_descriptor(req.uri().path());
    let session = web_console_session_from_request(&state, req.headers())
        .await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;
    require_web_console_csrf(req.headers(), &session)?;

    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "status": "error",
            "error": "mfa_endpoint_not_implemented",
            "error_kind": "mfa_endpoint_not_implemented",
            "feature": descriptor.feature,
            "message": descriptor.message,
            "retryable": false,
        })),
    ))
}

struct PendingMfaEndpointDescriptor {
    feature: &'static str,
    message: &'static str,
}

fn pending_mfa_endpoint_descriptor(path: &str) -> PendingMfaEndpointDescriptor {
    match path {
        "/mfa/webauthn/register/start" => PendingMfaEndpointDescriptor {
            feature: "webauthn_register_start",
            message: "Passkey registration is not available in this build. Use TOTP for local MFA.",
        },
        "/mfa/webauthn/register/finish" => PendingMfaEndpointDescriptor {
            feature: "webauthn_register_finish",
            message: "Passkey registration is not available in this build. Use TOTP for local MFA.",
        },
        "/mfa/webauthn/assert/start" => PendingMfaEndpointDescriptor {
            feature: "webauthn_assert_start",
            message: "Passkey verification is not available in this build. Use TOTP for local MFA.",
        },
        "/mfa/webauthn/assert/finish" => PendingMfaEndpointDescriptor {
            feature: "webauthn_assert_finish",
            message: "Passkey verification is not available in this build. Use TOTP for local MFA.",
        },
        "/mfa/totp/setup/start" => PendingMfaEndpointDescriptor {
            feature: "totp_setup_start",
            message: "TOTP setup is unavailable from this endpoint. Refresh the page and use the Security settings panel.",
        },
        "/mfa/totp/setup/finish" => PendingMfaEndpointDescriptor {
            feature: "totp_setup_finish",
            message: "TOTP setup is unavailable from this endpoint. Refresh the page and use the Security settings panel.",
        },
        "/mfa/totp/verify" => PendingMfaEndpointDescriptor {
            feature: "totp_verify",
            message: "TOTP verification is unavailable from this endpoint. Refresh the page and use the Security settings panel.",
        },
        "/mfa/recovery-code/consume" => PendingMfaEndpointDescriptor {
            feature: "recovery_code_consume",
            message: "Recovery-code sign-in is not available in this build.",
        },
        "/mfa/recovery-codes/regenerate" => PendingMfaEndpointDescriptor {
            feature: "recovery_codes_regenerate",
            message: "Recovery-code regeneration is not available in this build.",
        },
        "/mfa/disable" => PendingMfaEndpointDescriptor {
            feature: "mfa_disable",
            message: "MFA cannot be disabled from this WebApp.",
        },
        _ => PendingMfaEndpointDescriptor {
            feature: "unknown_mfa_endpoint",
            message: "This MFA action is not available in this WebApp.",
        },
    }
}

pub(super) fn require_web_console_csrf(
    headers: &axum::http::HeaderMap,
    session: &web_console_auth::WebConsoleSession,
) -> Result<(), (StatusCode, String)> {
    let expected = web_console_auth::csrf_token_for_session(session);
    let provided = headers
        .get(web_console_auth::CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if provided == expected {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Missing or invalid CSRF token".to_string(),
        ))
    }
}

pub(super) async fn web_console_session_from_request(
    state: &AdminState,
    headers: &axum::http::HeaderMap,
) -> Option<web_console_auth::WebConsoleSession> {
    let db_path = state.db_path.as_ref()?.clone();
    let config = web_console_auth::WebConsoleLoginConfig::from_env();
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())?;
    let cookie_token =
        web_console_auth::extract_cookie_value(cookie_header, &config.session_cookie_name)?;
    tokio::task::spawn_blocking(
        move || -> anyhow::Result<Option<web_console_auth::WebConsoleSession>> {
            let conn = crate::encrypted_sqlite::open(db_path)?;
            web_console_auth::init_session_schema(&conn)?;
            web_console_auth::validate_session(
                &conn,
                &cookie_token,
                chrono::Utc::now().timestamp() as u64,
                config.session_idle_ttl_secs,
            )
        },
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console session validation task failed");
    })
    .ok()?
    .map_err(|e| {
        tracing::warn!(error = %e, "Web Console session validation failed");
    })
    .ok()
    .flatten()
}

/// Query parameters for the heartbeat audit endpoint.
#[derive(Debug, serde::Deserialize)]
struct HeartbeatAuditParams {
    limit: Option<usize>,
    skill: Option<String>,
}

/// GET /auth/callback/google — Shared public Google OAuth2 callback.
///
/// Web Console login states are consumed first. Other states continue through
/// the Google/Calendar authorization flow: verify HMAC state, exchange the code
/// for tokens, store them, hot-reload sandbox auth, and notify the user.
async fn google_oauth_public_callback(
    State(state): State<Arc<AdminState>>,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, (StatusCode, String)> {
    if query
        .state
        .as_deref()
        .is_some_and(web_console_auth::has_pending_login_state)
    {
        return handle_web_console_google_login_callback(
            state,
            WebConsoleGoogleLoginCallbackQuery {
                code: query.code,
                state: query.state,
                error: query.error,
            },
        )
        .await;
    }

    // Handle errors from Google
    if let Some(error) = &query.error {
        // HTML-escape to prevent reflected XSS via crafted error parameter
        let escaped_error = error
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#x27;");
        return Ok(
            axum::response::Html(google_oauth_callback_error_html(&escaped_error)).into_response(),
        );
    }

    let code = query.code.as_deref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing 'code' parameter".to_string(),
        )
    })?;
    let state_param = query.state.as_deref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing 'state' parameter".to_string(),
        )
    })?;
    if let Some(limit) = google_auth::check_google_oauth_callback_rate_limit(
        &oauth_callback_rate_limit_key(state_param),
    ) {
        tracing::warn!(
            endpoint = "oauth_callback",
            retry_after_secs = limit.retry_after_secs,
            "OAuth callback rate limit exceeded"
        );
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "授权回调尝试过于频繁，请 {} 秒后重试。/ Too many authorization callback attempts. Try again in {} seconds.",
                limit.retry_after_secs, limit.retry_after_secs
            ),
        ));
    }

    // Verify HMAC state and extract chat_id
    let identity = state.identity.as_ref().ok_or_else(|| {
        tracing::error!(endpoint = "oauth_callback", "Identity not configured for OAuth callback");
        (StatusCode::INTERNAL_SERVER_ERROR, "认证服务暂不可用，请联系管理员。/ Authentication service unavailable. Please contact the administrator.".to_string())
    })?;
    let oauth_state = google_auth::verify_oauth_state_details(
        state_param,
        &identity.signing_key_bytes(),
    )
    .map_err(|e| {
        tracing::warn!(error = %e, endpoint = "oauth_callback", "OAuth state validation failed");
        (
            StatusCode::BAD_REQUEST,
            "认证失败，请重试。/ Authentication failed. Please try again.".to_string(),
        )
    })?;
    let chat_id = oauth_state.chat_id;

    // Read pending client credentials from credential store
    let (db_path, aes_key) = match (&state.credential_db_path, &state.credential_aes_key) {
        (Some(p), Some(k)) => (p.clone(), *k),
        _ => {
            tracing::error!(
                endpoint = "oauth_callback",
                "Credential store not configured"
            );
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "认证服务暂不可用，请联系管理员。/ Authentication service unavailable. Please contact the administrator.".to_string(),
            ));
        }
    };

    let db_path_clone = db_path.clone();
    let pending_service = google_auth::pending_cred_service_for_flow(chat_id, &oauth_state.flow_id);
    let incoming_flow_id = oauth_state.flow_id.clone();
    let (client_id, client_secret, account_name) = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String, String)> {
        let cred_store = CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path_clone)
                .map_err(|e| anyhow::anyhow!("DB open: {e}"))?,
            aes_key,
        )?;
        // Bind the callback to the chat's currently-active OAuth flow.
        //
        // Without this check, a stale state-string from an abandoned flow
        // could complete and silently overwrite the credentials that a
        // newer, user-initiated flow is about to write — a "last writer
        // wins" race when two browser tabs are open. We compare the
        // signed `flow_id` from the OAuth state against `active_flow_id`
        // recorded by `stage_pending_oauth_flow_for_chat` and reject
        // the callback before any code exchange or credential write.
        let base_pending_service = google_auth::pending_cred_service(Some(chat_id));
        let active_flow_id = cred_store
            .get(&base_pending_service, "active_flow_id")?;
        match active_flow_id.as_deref() {
            Some(active) if active == incoming_flow_id => {}
            Some(active) => {
                // Log only the first 8 chars of each flow_id so the
                // tracing breadcrumb stays useful for debugging without
                // dumping the full identifier into logs.
                let active_prefix: String = active.chars().take(8).collect();
                let incoming_prefix: String = incoming_flow_id.chars().take(8).collect();
                anyhow::bail!(
                    "oauth flow expired or superseded: incoming flow_id does not match active flow_id for chat {chat_id} (active prefix '{active_prefix}', incoming prefix '{incoming_prefix}')",
                );
            }
            None => {
                anyhow::bail!(
                    "oauth flow expired or superseded: no active flow_id staged for chat {chat_id}",
                );
            }
        }
        let cid = cred_store
            .get(&pending_service, "client_id")?
            .ok_or_else(|| anyhow::anyhow!("No pending OAuth client_id — run setup_google first"))?;
        let cs = cred_store
            .get(&pending_service, "client_secret")?
            .ok_or_else(|| anyhow::anyhow!("No pending OAuth client_secret"))?;
        let name = cred_store
            .get(&pending_service, "account_name")?
            .unwrap_or_else(|| "primary".to_string());
        Ok((cid, cs, name))
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, endpoint = "oauth_callback", "Credential read task failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "认证处理失败，请重试。/ Authentication processing failed. Please try again.".to_string())
    })?
    .map_err(|e| {
        let msg = format!("{e}");
        if msg.contains("oauth flow expired or superseded") {
            tracing::warn!(
                endpoint = "oauth_callback",
                chat_id,
                detail = %msg,
                "rejecting OAuth callback — flow_id mismatch or no active flow"
            );
            (
                StatusCode::BAD_REQUEST,
                "认证流程已过期或已被新流程取代，请重新发起授权。/ Authentication flow expired or superseded. Please start a new authorization.".to_string(),
            )
        } else {
            tracing::warn!(error = %e, endpoint = "oauth_callback", "Failed to read pending credentials");
            (StatusCode::INTERNAL_SERVER_ERROR, "认证处理失败，请重试。/ Authentication processing failed. Please try again.".to_string())
        }
    })?;

    // Build redirect_uri (must match what was used in the auth URL)
    let public_url = state.public_url.as_deref().ok_or_else(|| {
        tracing::error!(endpoint = "oauth_callback", "AEGIS_PUBLIC_URL not configured");
        (StatusCode::INTERNAL_SERVER_ERROR, "认证服务配置错误，请联系管理员。/ Authentication service misconfigured. Please contact the administrator.".to_string())
    })?;
    let redirect_uri = format!("{public_url}/auth/callback/google");

    // Exchange code for tokens (blocking HTTP call)
    let cid: String = client_id;
    let cs: String = client_secret;
    let code_owned = code.to_string();
    let ruri = redirect_uri.clone();
    let (manager, granted_scopes) = tokio::task::spawn_blocking(move || {
        GoogleAuthManager::exchange_code_with_redirect(&cid, &cs, &code_owned, &ruri)
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, endpoint = "oauth_callback", "Token exchange task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "认证处理失败，请重试。/ Authentication processing failed. Please try again."
                .to_string(),
        )
    })?
    .map_err(|e| {
        tracing::warn!(error = %e, endpoint = "oauth_callback", "Token exchange failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "授权码交换失败，请重试。/ Authorization code exchange failed. Please try again."
                .to_string(),
        )
    })?;

    let manager = Arc::new(manager);

    let cred_service = crate::google_auth::google_oauth_cred_service(&account_name);
    let manager_save = manager.clone();
    let db_path_save = db_path.clone();
    let pending_service = google_auth::pending_cred_service_for_flow(chat_id, &oauth_state.flow_id);
    let scope_audit_context = tokio::task::spawn_blocking(
        move || -> anyhow::Result<Option<google_auth::GoogleScopeUpgradeAuditContext>> {
            let cred_store = CredentialStore::open(
                crate::encrypted_sqlite::open(&db_path_save)
                    .map_err(|e| anyhow::anyhow!("DB open: {e}"))?,
                aes_key,
            )?;
            let scope_audit_context =
                google_auth::load_scope_upgrade_audit_context(&cred_store, &pending_service)?;
            manager_save.save_to_credential_store_named(&cred_store, &cred_service)?;
            for key in &["client_id", "client_secret", "account_name", "staged_at"] {
                if let Err(e) = cred_store.delete(&pending_service, key) {
                    tracing::warn!(error = %e, key, "Failed to clean up pending credential");
                }
            }
            if let Err(e) =
                google_auth::delete_scope_upgrade_audit_context(&cred_store, &pending_service)
            {
                tracing::warn!(error = %e, "Failed to clean up scope upgrade audit context");
            }
            let base_pending_service = google_auth::pending_cred_service(Some(chat_id));
            if let Err(e) = cred_store.delete(&base_pending_service, "active_flow_id") {
                tracing::warn!(error = %e, "Failed to clean up active_flow_id");
            }
            if let Err(e) =
                google_auth::delete_scope_upgrade_audit_context(&cred_store, &base_pending_service)
            {
                tracing::debug!(error = %e, "no base scope upgrade audit context to clear");
            }
            Ok(scope_audit_context)
        },
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "oauth_callback", "Credential save task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "认证处理失败，请重试。/ Authentication processing failed. Please try again."
                .to_string(),
        )
    })?
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "oauth_callback", "Failed to save credentials");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "认证处理失败，请重试。/ Authentication processing failed. Please try again."
                .to_string(),
        )
    })?;

    state
        .sandbox
        .set_dynamic_auth(&["www.googleapis.com"], manager.clone());
    *state.google_auth.write() = Some(manager.clone());

    tracing::info!(
        chat_id,
        account = %account_name,
        scopes = ?granted_scopes,
        "Google OAuth2 setup completed via public callback"
    );

    if let Some(context) = scope_audit_context {
        let record =
            google_auth::build_scope_upgrade_audit_record(&context, &granted_scopes, "callback");
        let key = format!(
            "scope-upgrade-{}-{:016x}",
            chrono::Utc::now().timestamp_millis(),
            rand::random::<u64>()
        );
        if let Err(e) = state
            .memory
            .store_skill_state("google_auth_audit", &key, &record.to_string())
            .await
        {
            tracing::warn!(error = %e, key, "failed to store Google scope upgrade audit record");
        }
    }

    if let Some(bot) = &state.telegram_bot {
        use teloxide::payloads::SendMessageSetters;
        use teloxide::prelude::*;
        let msg = format!(
            "Google Calendar/Drive connected for account `{account_name}` (scopes: {granted_scopes:?})."
        );
        let (target_chat_id, thread) =
            oauth_success_notification_destination(chat_id, state.admin_chat_id);
        let mut req = bot.send_message(target_chat_id, msg);
        if let Some(tid) = thread {
            req = req.message_thread_id(tid);
        }
        if let Err(e) = req.await {
            tracing::warn!(
                error = %e,
                callback_chat_id = chat_id,
                target_chat_id = target_chat_id.0,
                "Failed to send OAuth success notification to Telegram"
            );
        }
    }

    Ok(axum::response::Html(google_oauth_callback_success_html()).into_response())
}
