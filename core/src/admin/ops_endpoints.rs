use super::*;

pub(super) fn google_oauth_callback_error_html(escaped_error: &str) -> String {
    format!(
        "<html><body><h2>Authorization Failed</h2><p>Google returned error: {escaped_error}</p>\
         <p>You can close this tab and continue in Aegis.</p></body></html>"
    )
}

pub(super) fn google_oauth_callback_success_html() -> String {
    "<html><body>\
     <h2>Authorization Successful!</h2>\
     <p>Google Calendar and Drive have been connected to Aegis. You can close this tab and continue in the WebApp.</p>\
     <script>setTimeout(function(){window.close()},3000)</script>\
     </body></html>"
        .to_string()
}

pub(super) async fn health(
    State(state): State<Arc<AdminState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let memory_ok = state.memory.healthcheck().await;
    let db_status = state.memory.check_db_size().await;
    let skill_count = state.sandbox.loaded_skills().len();
    let mut computer_use_health = crate::computer_use::health_component();
    let jev_config = if state.credential_db_path.is_some() {
        vnext_xai_credential_store(&state)
            .ok()
            .and_then(|store| crate::jev_config::resolve(Some(&store)).ok())
    } else {
        crate::jev_config::resolve(None).ok()
    };
    computer_use_health["jev_enabled"] =
        serde_json::json!(jev_config.as_ref().is_some_and(|c| c.enabled));
    computer_use_health["openrouter_key_present"] =
        serde_json::json!(jev_config.as_ref().is_some_and(|c| c.api_key.is_some()));
    computer_use_health["jev_configuration_available"] = serde_json::json!(jev_config.is_some());
    computer_use_health["jev_non_cu_gate"]["openrouter_key_present"] =
        computer_use_health["openrouter_key_present"].clone();

    let hnsw_stats = if let Some(idx) = state.memory.hnsw_index() {
        serde_json::json!({
            "ready": idx.is_ready(),
            "size": idx.len(),
            "deleted": idx.deleted_count(),
        })
    } else {
        serde_json::json!({ "ready": false, "size": 0 })
    };

    let db_json = match db_status {
        Ok(s) => serde_json::json!({
            "ok": !s.exceeded,
            "size_mb": s.current_bytes / 1_048_576,
            "max_mb": s.max_bytes / 1_048_576,
        }),
        Err(_) => serde_json::json!({ "ok": false, "error": "unreachable" }),
    };

    // Incident 2026-09-15: the bot token was revoked and /health stayed "ok"
    // for six hours because it knew nothing about Telegram. The bot's state
    // degrades the top-level status (invalid token immediately, unreachable
    // after ten minutes) but keeps HTTP 200: deploy.sh and smoke_test.sh
    // call this with `curl -f`, and a dead bot must not read as a dead core.
    let telegram = crate::telegram_health::snapshot();
    let telegram_degraded = telegram.is_degraded_at(crate::telegram_health::now_unix());
    let telegram_json = serde_json::to_value(&telegram).unwrap_or(serde_json::Value::Null);

    // The same lesson applies to the two runtime signals the console already
    // shows as banners: an exhausted LLM provider or a dead primary embedder
    // was visible in the webapp but invisible to `curl /health`. Both are
    // `None` when healthy and serialise as `null`.
    let llm_degraded = state
        .orchestrator
        .as_ref()
        .and_then(|o| o.llm_degraded_snapshot());
    let embedder_degraded = state
        .orchestrator
        .as_ref()
        .and_then(|o| o.embedder_degraded_snapshot());
    let runtime_degraded =
        telegram_degraded || llm_degraded.is_some() || embedder_degraded.is_some();

    // Last nightly verdict, written by the nightly script (another change
    // owns the writer). Purely informational: a red nightly is not a broken
    // production process, so it never touches `status`, and a missing or
    // garbled file never fails the endpoint.
    let nightly = read_nightly_status();

    let all_ok = memory_ok && db_json["ok"].as_bool().unwrap_or(false);
    let status_code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let status = health_status_label(all_ok, runtime_degraded);

    (
        status_code,
        Json(serde_json::json!({
            "status": status,
            "version": env!("CARGO_PKG_VERSION"),
            "components": {
                "memory_actor": if memory_ok { "ok" } else { "unhealthy" },
                "database": db_json,
                "hnsw": hnsw_stats,
                "skills_loaded": skill_count,
                "telegram": telegram_json,
                "llm_degraded": llm_degraded,
                "embedder_degraded": embedder_degraded,
                "nightly": nightly,
                "secure_computer_use": computer_use_health,
            }
        })),
    )
}

/// Top-level `/health` status. `all_ok` is the core (memory actor + database);
/// `runtime_degraded` is any of the soft signals (Telegram, LLM provider,
/// embedder) that leave the process serving but the owner under-served.
pub(super) fn health_status_label(all_ok: bool, runtime_degraded: bool) -> &'static str {
    if all_ok && !runtime_degraded {
        "ok"
    } else {
        "degraded"
    }
}

/// Where the nightly script leaves its verdict when `AEGIS_NIGHTLY_STATUS_FILE`
/// is unset.
pub(super) const DEFAULT_NIGHTLY_STATUS_FILE: &str =
    "/opt/aegis-runtime/shared/status/nightly-status.json";

/// Upper bound on the status file; anything larger is not the small JSON
/// document the nightly script writes and is ignored rather than parsed.
const NIGHTLY_STATUS_MAX_BYTES: u64 = 64 * 1024;

/// Verdict of the last nightly test run, as written by the nightly script to
/// `AEGIS_NIGHTLY_STATUS_FILE`:
///
/// ```json
/// {"verdict":"GREEN","sha":"02c0d462","finished_at_unix":1789700000,
///  "failed_steps":[],"report":"/opt/aegis-runtime/shared/nightly/2026-09-16.md"}
/// ```
///
/// Exposed verbatim as `components.nightly` on `/health` and as `nightly` on
/// the console status projection. `verdict` is `GREEN`, `RED` or
/// `BROWSER_RED` today; the reader keeps it as an upper-cased string so a new
/// verdict from the script does not turn into "no nightly".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NightlyStatus {
    pub(crate) verdict: String,
    #[serde(default)]
    pub(crate) sha: String,
    pub(crate) finished_at_unix: i64,
    #[serde(default)]
    pub(crate) failed_steps: Vec<String>,
    #[serde(default)]
    pub(crate) report: Option<String>,
}

/// Path of the nightly status file: `AEGIS_NIGHTLY_STATUS_FILE`, else
/// [`DEFAULT_NIGHTLY_STATUS_FILE`].
pub(super) fn nightly_status_path() -> PathBuf {
    std::env::var("AEGIS_NIGHTLY_STATUS_FILE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NIGHTLY_STATUS_FILE))
}

/// Read the nightly verdict from the configured path. `None` when the file
/// is absent, oversized, or not the expected JSON — never an error, because
/// `/health` must not fail on a diagnostic side channel.
pub(crate) fn read_nightly_status() -> Option<NightlyStatus> {
    read_nightly_status_from(&nightly_status_path())
}

/// [`read_nightly_status`] for an explicit path (the testable half).
pub(crate) fn read_nightly_status_from(path: &FsPath) -> Option<NightlyStatus> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::debug!(path = %path.display(), error = %err, "nightly status file unreadable");
            return None;
        }
    };
    if !metadata.is_file() || metadata.len() > NIGHTLY_STATUS_MAX_BYTES {
        tracing::debug!(
            path = %path.display(),
            bytes = metadata.len(),
            "nightly status file ignored: not a regular file or too large"
        );
        return None;
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            tracing::debug!(path = %path.display(), error = %err, "nightly status file unreadable");
            return None;
        }
    };
    let mut parsed: NightlyStatus = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::debug!(path = %path.display(), error = %err, "nightly status file is not the expected JSON");
            return None;
        }
    };
    parsed.verdict = parsed.verdict.trim().to_ascii_uppercase();
    if parsed.verdict.is_empty() || parsed.verdict.len() > 32 {
        tracing::debug!(path = %path.display(), "nightly status file has no usable verdict");
        return None;
    }
    parsed.sha = parsed.sha.trim().to_string();
    Some(parsed)
}

pub(super) async fn health_memory(
    State(state): State<Arc<AdminState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let healthy = state.memory.healthcheck().await;
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(serde_json::json!({
            "status": if healthy { "ok" } else { "unhealthy" },
            "component": "memory_actor",
        })),
    )
}

pub(super) async fn list_skills(State(state): State<Arc<AdminState>>) -> Json<SkillsResponse> {
    let skills: Vec<SkillInfo> = state
        .sandbox
        .loaded_skills()
        .into_iter()
        .map(|cap| SkillInfo {
            name: cap.skill_name.clone(),
            max_memory_mb: cap.max_memory_mb,
            allowed_domains: cap.allowed_domains.clone(),
            cron_schedule: cap.cron_schedule.clone(),
            description: cap.description.clone(),
        })
        .collect();

    Json(SkillsResponse {
        count: skills.len(),
        skills,
    })
}

// ── /memory_status endpoint ──────────────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct MemoryStatusResponse {
    current_bytes: u64,
    current_gb: f64,
    max_bytes: u64,
    max_gb: f64,
    usage_percent: f64,
    exceeded: bool,
}

pub(super) async fn memory_status(
    State(state): State<Arc<AdminState>>,
) -> Json<MemoryStatusResponse> {
    let status = state
        .memory
        .check_db_size()
        .await
        .unwrap_or(crate::memory::DbSizeStatus {
            current_bytes: 0,
            max_bytes: 0,
            exceeded: false,
        });

    let current_gb = status.current_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let max_gb = status.max_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let usage_percent = if status.max_bytes > 0 {
        (status.current_bytes as f64 / status.max_bytes as f64) * 100.0
    } else {
        0.0
    };

    Json(MemoryStatusResponse {
        current_bytes: status.current_bytes,
        current_gb,
        max_bytes: status.max_bytes,
        max_gb,
        usage_percent,
        exceeded: status.exceeded,
    })
}

// ── /reminders endpoint (now reads from unified tasks table) ─────────────

#[derive(Serialize)]
pub(super) struct RemindersResponse {
    count: usize,
    reminders: Vec<ReminderInfo>,
}

#[derive(Serialize)]
struct ReminderInfo {
    id: i64,
    chat_id: i64,
    message: String,
    remind_at_unix: i64,
    created_at: String,
}

pub(super) async fn list_pending_reminders(
    State(state): State<Arc<AdminState>>,
) -> Json<RemindersResponse> {
    // Read all active scheduled tasks from the unified tasks table (admin view).
    // list_tasks with chat_id=0 won't match real data, so we use list_all_active_scheduled_tasks.
    let tasks = state.memory.list_all_active_scheduled_tasks(50).await;
    let reminders: Vec<ReminderInfo> = tasks
        .into_iter()
        .map(|t| ReminderInfo {
            id: t.id,
            chat_id: t.chat_id,
            message: t.title,
            remind_at_unix: t.trigger_at_unix.unwrap_or(0),
            created_at: t.created_at,
        })
        .collect();

    Json(RemindersResponse {
        count: reminders.len(),
        reminders,
    })
}

// ── /l2/:did endpoint ───────────────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct L2FactsResponse {
    did: String,
    count: usize,
    facts: Vec<L2FactInfo>,
}

#[derive(Serialize)]
struct L2FactInfo {
    id: i64,
    topic: String,
    fact_summary: String,
    created_at: String,
}

/// Validate DID format: must start with "did:aegis:" and be ≤256 chars.
pub(super) fn validate_did(did: &str) -> Result<(), (StatusCode, String)> {
    if did.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            "DID too long (max 256 chars)".into(),
        ));
    }
    if !did.starts_with("did:aegis:") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid DID format (expected did:aegis:...)".into(),
        ));
    }
    Ok(())
}

pub(super) async fn list_l2_facts(
    State(state): State<Arc<AdminState>>,
    Path(did): Path<String>,
) -> Result<Json<L2FactsResponse>, (StatusCode, String)> {
    validate_did(&did)?;
    let facts: Vec<L2FactInfo> = state
        .memory
        .get_all_l2(&did)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| L2FactInfo {
            id: e.id,
            topic: e.topic,
            fact_summary: e.fact_summary,
            created_at: e.created_at,
        })
        .collect();

    Ok(Json(L2FactsResponse {
        did,
        count: facts.len(),
        facts,
    }))
}

// ── DELETE /l2/:did/:id endpoint ────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct DeleteL2Response {
    deleted: bool,
}

pub(super) async fn delete_l2_fact(
    State(state): State<Arc<AdminState>>,
    Path((did, id)): Path<(String, i64)>,
) -> Result<Json<DeleteL2Response>, (StatusCode, String)> {
    validate_did(&did)?;
    match state.memory.delete_l2(&did, id).await {
        Ok(deleted) => Ok(Json(DeleteL2Response { deleted })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete L2 fact: {e}"),
        )),
    }
}

// ── /settings endpoints ──────────────────────────────────────────────────────

pub(super) async fn list_settings(State(state): State<Arc<AdminState>>) -> Json<serde_json::Value> {
    let snapshots = state.settings.all();
    Json(serde_json::json!({ "settings": snapshots }))
}

#[derive(serde::Deserialize)]
pub(super) struct UpdateSettingBody {
    value: serde_json::Value,
}

pub(super) async fn update_setting(
    State(state): State<Arc<AdminState>>,
    Path(key): Path<String>,
    Json(body): Json<UpdateSettingBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Special-case heartbeat_config: route through the orchestrator helper
    // so the change is persisted, audit-logged, and hot-reloaded atomically
    // (design §7, Step 5). Falls back to the generic path when no
    // orchestrator handle is attached (e.g. in some test harnesses).
    if key == "heartbeat_config"
        && let Some(ref orch) = state.orchestrator
    {
        let Some(new_cfg) = crate::heartbeat_config::HeartbeatConfig::from_json(&body.value) else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "heartbeat_config payload could not be decoded",
                })),
            ));
        };
        match orch
            .update_heartbeat_config(
                new_cfg,
                crate::heartbeat_config::ConfigAuditActor::User,
                Some("admin HTTP".into()),
            )
            .await
        {
            Ok(changed) => {
                return Ok(Json(serde_json::json!({
                    "key": key,
                    "value": body.value,
                    "changed": changed,
                })));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                ));
            }
        }
    }

    if key == "llm_model" || key == "fallback_model" {
        let model = body
            .value
            .as_str()
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("{key} must be a string") })),
                )
            })?
            .to_string();
        if model.chars().count() > LLM_MODEL_MAX_LEN {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("{key} must be at most {LLM_MODEL_MAX_LEN} characters"),
                })),
            ));
        }
        let catalog = crate::model_catalog::load_model_catalog(&state).await;
        let validation = if key == "llm_model" {
            let provider = state
                .settings
                .get("llm_provider")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "codex_oauth".to_string());
            crate::model_catalog::validate_llm_model_setting(&catalog, &provider, &model)
        } else {
            crate::model_catalog::validate_fallback_model_setting(&catalog, &model)
        };
        if let Err(msg) = validation {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            ));
        }
    }

    if crate::call_validation::requires_verified_settings_path(&key, &body.value) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": crate::call_validation::settings_path_message()})),
        ));
    }
    state
        .settings
        .prepare_updates(vec![(key.clone(), Some(body.value.clone()))])
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;
    match state
        .memory
        .commit_runtime_settings_with_embedding(
            state.settings.clone(),
            vec![(key.clone(), Some(body.value.clone()))],
            state
                .orchestrator
                .as_ref()
                .map(|orch| orch.embedding_client_for_settings()),
        )
        .await
    {
        Ok(mut commit) => {
            let old = commit.previous.remove(&key).flatten();
            let current = commit.current.remove(&key);

            let hot_reload = commit.embedding.remove(&key).map(|outcome| {
                log_embedding_hot_reload(&key, &outcome, "admin HTTP update_setting");
                update_outcome_to_json(&outcome)
            });

            Ok(Json(serde_json::json!({
                "key": key,
                "value": current,
                "previous": old,
                "hot_reload": hot_reload,
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

#[derive(Default, serde::Deserialize)]
pub(super) struct LlmFallbackDrillBody {
    #[serde(default)]
    fault: Option<String>,
}

fn fallback_drill_failed_report(fault: &'static str) -> crate::orchestrator::FallbackDrillReport {
    crate::orchestrator::FallbackDrillReport {
        fault,
        primary_provider: String::new(),
        primary_model: String::new(),
        fallback_authorized: false,
        paid_api_fallback_enabled: false,
        answered_by: None,
        elapsed_ms: 0,
        ok: false,
        error: Some(format!(
            "{} ({})",
            crate::i18n::t("diagnostics.fallback_drill.backup_failed", "zh"),
            crate::i18n::t("diagnostics.fallback_drill.backup_failed", "en")
        )),
    }
}

/// POST /diagnostics/llm_fallback_drill — fault-injected backup-model drill.
///
/// Optional JSON body `{ "fault": "primary_timeout" }` (the only fault, and
/// the default). HTTP 200 when the backup answered; 503 when it did not
/// (JSON body is the report in both cases).
pub(super) async fn llm_fallback_drill(
    State(state): State<Arc<AdminState>>,
    body: Option<Json<LlmFallbackDrillBody>>,
) -> (StatusCode, Json<crate::orchestrator::FallbackDrillReport>) {
    let fault_raw = body
        .as_ref()
        .and_then(|Json(b)| b.fault.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let fault = match fault_raw {
        None | Some("primary_timeout") => crate::orchestrator::DrillFault::PrimaryTimeout,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(fallback_drill_failed_report("primary_timeout")),
            );
        }
    };

    let Some(orch) = state.orchestrator.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(fallback_drill_failed_report(fault.as_str())),
        );
    };

    let report = orch.run_llm_fallback_drill(fault).await;
    let status = if report.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(report))
}

pub(super) async fn reset_setting(
    State(state): State<Arc<AdminState>>,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let default = crate::settings::RuntimeSettings::new()
        .get(&key)
        .unwrap_or(serde_json::Value::Null);
    if crate::call_validation::requires_verified_settings_path(&key, &default) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": crate::call_validation::settings_path_message()})),
        ));
    }
    state
        .settings
        .prepare_updates(vec![(key.clone(), None)])
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;
    match state
        .memory
        .commit_runtime_settings_with_embedding(
            state.settings.clone(),
            vec![(key.clone(), None)],
            state
                .orchestrator
                .as_ref()
                .map(|orch| orch.embedding_client_for_settings()),
        )
        .await
    {
        Ok(mut commit) => {
            let old = commit.previous.remove(&key).flatten();
            let current = commit.current.remove(&key);

            let hot_reload = commit.embedding.remove(&key).map(|outcome| {
                log_embedding_hot_reload(&key, &outcome, "admin HTTP reset_setting");
                update_outcome_to_json(&outcome)
            });

            Ok(Json(serde_json::json!({
                "key": key,
                "value": current,
                "previous": old,
                "reset_to_default": true,
                "hot_reload": hot_reload,
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

/// Serialise an [`UpdateOutcome`] into a compact JSON envelope for the admin
/// HTTP response so the caller can confirm the live client was actually
/// refreshed (not just the DB row).
fn update_outcome_to_json(outcome: &crate::embedding::UpdateOutcome) -> serde_json::Value {
    use crate::embedding::UpdateOutcome;
    match outcome {
        UpdateOutcome::SilentOk => serde_json::json!({ "status": "silent_ok" }),
        UpdateOutcome::HotReloaded { from, to } => serde_json::json!({
            "status": "hot_reloaded",
            "from": from,
            "to": to,
        }),
        UpdateOutcome::RequiresCorpusReembed { old, new } => serde_json::json!({
            "status": "requires_corpus_reembed",
            "old": old,
            "new": new,
            "hint": "stored vectors are cross-model/dim — call trigger_corpus_reembed to recompute",
        }),
        UpdateOutcome::Rejected { reason } => serde_json::json!({
            "status": "rejected",
            "reason": reason,
        }),
    }
}

/// Emit a uniformly-formatted tracing event for an `embedding_*` hot-reload
/// triggered from the admin HTTP path (update or reset). Mirrors the shape of
/// the log lines emitted by the HITL path so operators can grep the two paths
/// under a single pattern.
fn log_embedding_hot_reload(key: &str, outcome: &crate::embedding::UpdateOutcome, source: &str) {
    use crate::embedding::UpdateOutcome;
    match outcome {
        UpdateOutcome::SilentOk => {
            tracing::debug!(key = %key, %source, "[admin] apply_embedding_setting: value already matched live state");
        }
        UpdateOutcome::HotReloaded { from, to } => {
            tracing::info!(key = %key, from = %from, to = %to, %source, "[admin] apply_embedding_setting: hot-reloaded");
        }
        UpdateOutcome::RequiresCorpusReembed { old, new } => {
            tracing::warn!(key = %key, old = %old, new = %new, %source, "[admin] apply_embedding_setting: corpus reembed required");
        }
        UpdateOutcome::Rejected { reason } => {
            tracing::warn!(key = %key, reason = %reason, %source, "[admin] apply_embedding_setting: rejected by client");
        }
    }
}

// ── Skill registry endpoints (skill_proposals rows) ──────────────────────────

pub(super) async fn list_skill_registry(
    State(state): State<Arc<AdminState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.memory.list_skill_proposals(None, None).await {
        Ok(proposals) => {
            let items: Vec<serde_json::Value> = proposals
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "skill_name": p.skill_name,
                        "requested_by": p.requested_by,
                        "status": p.status,
                        "stage_error": p.stage_error,
                        "user_request": p.user_request,
                        "requires_auth": p.requires_auth,
                        "created_at": p.created_at,
                        "updated_at": p.updated_at,
                    })
                })
                .collect();
            Ok(Json(serde_json::json!({
                "total": items.len(),
                "skills": items,
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list skill registry: {e}"),
        )),
    }
}

pub(super) async fn get_skill_registry_entry(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.memory.get_skill_proposal(id).await {
        Ok(Some(p)) => Ok(Json(serde_json::json!({
            "id": p.id,
            "skill_name": p.skill_name,
            "requested_by": p.requested_by,
            "status": p.status,
            "stage_error": p.stage_error,
            "user_request": p.user_request,
            "proposal_summary": p.proposal_summary,
            "requires_auth": p.requires_auth,
            "auth_domain": p.auth_domain,
            "created_at": p.created_at,
            "updated_at": p.updated_at,
        }))),
        Ok(None) => Err((StatusCode::NOT_FOUND, "Skill not found".to_string())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )),
    }
}

pub(super) async fn delete_skill_registry_entry(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.memory.delete_skill_proposal(id).await {
        Ok(true) => Ok(Json(serde_json::json!({"deleted": true, "id": id}))),
        Ok(false) => Err((StatusCode::NOT_FOUND, format!("Skill row {id} not found"))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete skill row: {e}"),
        )),
    }
}

pub(super) async fn deactivate_skill_registry_entry(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Get the registry row to find the skill name
    let proposal = match state.memory.get_skill_proposal(id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Skill not found".to_string())),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            ));
        }
    };

    // Unload from sandbox
    let unloaded = state.sandbox.unload_skill(&proposal.skill_name);

    // Update status
    if let Err(e) = state
        .memory
        .update_skill_proposal(
            id,
            crate::memory::SkillProposalUpdate {
                status: Some(STATUS_DEACTIVATED.to_string()),
                ..Default::default()
            },
        )
        .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to update skill status: {e}"),
        ));
    }

    Ok(Json(serde_json::json!({
        "id": id,
        "skill_name": proposal.skill_name,
        "deactivated": true,
        "unloaded_from_sandbox": unloaded,
    })))
}

// ── /system_metrics endpoint ─────────────────────────────────────────────────

pub(super) async fn system_metrics() -> Json<SystemMetrics> {
    let (load_1m, load_5m, load_15m) = read_loadavg();
    let (mem_total, mem_available) = read_meminfo();
    let (disk_total, disk_available) = read_disk_usage();
    let num_cpus = read_num_cpus();

    let mem_used_percent = if mem_total > 0 {
        (1.0 - mem_available as f64 / mem_total as f64) * 100.0
    } else {
        0.0
    };

    let disk_used_percent = if disk_total > 0 {
        (1.0 - disk_available as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    Json(SystemMetrics {
        cpu_load_1m: load_1m,
        cpu_load_5m: load_5m,
        cpu_load_15m: load_15m,
        memory_total_kb: mem_total,
        memory_available_kb: mem_available,
        memory_used_percent: mem_used_percent,
        disk_total_bytes: disk_total,
        disk_available_bytes: disk_available,
        disk_used_percent,
        num_cpus,
    })
}

/// Read CPU load averages from /proc/loadavg.
/// Returns (1m, 5m, 15m). Falls back to (0, 0, 0) on error.
pub(super) fn read_loadavg() -> (f64, f64, f64) {
    let content = match std::fs::read_to_string("/proc/loadavg") {
        Ok(c) => c,
        Err(_) => return (0.0, 0.0, 0.0),
    };
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() < 3 {
        return (0.0, 0.0, 0.0);
    }
    let l1 = parts[0].parse().unwrap_or(0.0);
    let l5 = parts[1].parse().unwrap_or(0.0);
    let l15 = parts[2].parse().unwrap_or(0.0);
    (l1, l5, l15)
}

/// Read memory info from /proc/meminfo.
/// Returns (total_kb, available_kb). Falls back to (0, 0) on error.
fn read_meminfo() -> (u64, u64) {
    let content = match std::fs::read_to_string("/proc/meminfo") {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = parse_meminfo_value(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = parse_meminfo_value(rest);
        }
    }
    (total, available)
}

/// Parse a /proc/meminfo value line like "  12345 kB" → 12345.
fn parse_meminfo_value(s: &str) -> u64 {
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Read disk usage for "/" using libc::statvfs.
/// Returns (total_bytes, available_bytes). Falls back to (0, 0) on error.
#[allow(unsafe_code)] // libc::statvfs requires unsafe FFI
fn read_disk_usage() -> (u64, u64) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path = CString::new("/").unwrap();
    let mut stat = MaybeUninit::<libc::statvfs>::uninit();

    let ret = unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) };
    if ret != 0 {
        return (0, 0);
    }

    let stat = unsafe { stat.assume_init() };
    let total = stat.f_blocks * stat.f_frsize;
    let available = stat.f_bavail * stat.f_frsize;
    (total, available)
}

/// Count the number of CPUs from /proc/cpuinfo.
pub(super) fn read_num_cpus() -> usize {
    let content = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let count = content
        .lines()
        .filter(|line| line.starts_with("processor"))
        .count();
    if count == 0 { 1 } else { count }
}

// ── Google OAuth2 endpoints ──────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(super) struct GoogleSetupRequest {
    client_id: String,
    client_secret: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

#[derive(Serialize)]
pub(super) struct GoogleAuthStatusResponse {
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct GoogleRevokeRequest {
    #[serde(default)]
    account_name: Option<String>,
}

// ── Google OAuth2 ───────────────────────────────────────────────────────────

/// POST /auth/google/setup — Start OAuth2 setup flow.
///
/// Request body: `{ "client_id": "...", "client_secret": "...", "scopes": [...] }`
///
/// **Two-phase flow:**
/// 1. First call returns `auth_url` — user opens it in browser.
/// 2. Server listens on localhost:8765 for the OAuth callback.
/// 3. On callback, exchanges code for tokens, stores encrypted, returns success.
pub(super) async fn google_auth_setup(
    State(state): State<Arc<AdminState>>,
    Json(body): Json<GoogleSetupRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (db_path, aes_key) = match (&state.credential_db_path, &state.credential_aes_key) {
        (Some(p), Some(k)) => (p.clone(), *k),
        _ => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Credential store not configured".to_string(),
            ));
        }
    };

    let scopes = body.scopes.unwrap_or_else(|| {
        google_auth::DEFAULT_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect()
    });

    let auth_url = GoogleAuthManager::build_auth_url(&body.client_id, &scopes, OAUTH_CALLBACK_PORT);

    let client_id = body.client_id.clone();
    let client_secret = body.client_secret.clone();
    let scopes_clone = scopes.clone();

    // setup_flow is async (axum callback server). After it completes,
    // we save credentials in spawn_blocking (rusqlite::Connection is !Send).
    let (manager, result) = GoogleAuthManager::setup_flow(
        &client_id,
        &client_secret,
        &scopes_clone,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_setup", "OAuth setup flow failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "认证设置失败，请重试。/ Authentication setup failed. Please try again.".to_string(),
        )
    })?;

    // Save credentials to encrypted store (sync, in blocking thread)
    let manager_for_save = manager.clone();
    tokio::task::spawn_blocking(move || {
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path).map_err(|e| anyhow::anyhow!("DB open: {e}"))?,
            aes_key,
        )?;
        manager_for_save.save_to_credential_store(&cred_store)
    })
    .await
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_setup", "Credential save task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "凭据保存失败，请重试。/ Failed to save credentials. Please try again.".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_setup", "Failed to save credentials");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "凭据保存失败，请重试。/ Failed to save credentials. Please try again.".to_string(),
        )
    })?;

    // Register the new manager for live use. Keep the legacy global mirror
    // updated as a backward-compat/bootstrap shim, but prefer the account
    // registry as the real source of truth.
    state
        .sandbox
        .set_dynamic_auth(&["www.googleapis.com"], manager.clone());
    *state.google_auth.write() = Some(manager.clone());

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": result.message,
        "scopes": result.scopes,
        "auth_url": auth_url,
    })))
}

/// POST /auth/google/store — Directly store OAuth2 credentials (no browser flow).
///
/// Request body: `{ "client_id": "...", "client_secret": "...", "refresh_token": "...", "scopes": "scope1,scope2" }`
///
/// Use this when you already have a refresh token (e.g. obtained via manual code exchange).
/// Credentials are encrypted with AES-256-GCM and stored in the credential store.
pub(super) async fn google_auth_store(
    State(state): State<Arc<AdminState>>,
    Json(body): Json<GoogleStoreRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (db_path, aes_key) = match (&state.credential_db_path, &state.credential_aes_key) {
        (Some(p), Some(k)) => (p.clone(), *k),
        _ => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Credential store not configured".to_string(),
            ));
        }
    };

    if body.client_id.is_empty() || body.client_secret.is_empty() || body.refresh_token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "client_id, client_secret, and refresh_token are all required".to_string(),
        ));
    }

    let scopes: Vec<String> = if body.scopes.is_empty() {
        google_auth::READONLY_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        body.scopes
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let manager = Arc::new(GoogleAuthManager::new(
        body.client_id,
        body.client_secret,
        body.refresh_token,
        scopes.clone(),
    ));

    // Validate the refresh token actually works (blocking HTTP call)
    let mgr_validate = manager.clone();
    let validation = tokio::task::spawn_blocking(move || mgr_validate.validate())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, endpoint = "google_auth_store", "Validation task failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "凭据验证失败，请重试。/ Credential validation failed. Please try again."
                    .to_string(),
            )
        })?;

    if let Err(e) = validation {
        tracing::warn!(error = %e, endpoint = "google_auth_store", "Refresh token validation failed");
        return Err((
            StatusCode::BAD_REQUEST,
            "刷新令牌验证失败，请检查凭据。/ Refresh token validation failed. Please check your credentials.".to_string(),
        ));
    }

    // Save to encrypted credential store (blocking — rusqlite is !Send)
    let mgr_save = manager.clone();
    tokio::task::spawn_blocking(move || {
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path).map_err(|e| anyhow::anyhow!("DB open: {e}"))?,
            aes_key,
        )?;
        mgr_save.save_to_credential_store(&cred_store)
    })
    .await
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_store", "Credential save task failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "凭据保存失败，请重试。/ Failed to save credentials. Please try again.".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_store", "Failed to save credentials");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "凭据保存失败，请重试。/ Failed to save credentials. Please try again.".to_string(),
        )
    })?;

    state
        .sandbox
        .set_dynamic_auth(&["www.googleapis.com"], manager.clone());
    *state.google_auth.write() = Some(manager.clone());

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Google OAuth2 credentials stored and validated",
        "scopes": scopes,
    })))
}

#[derive(serde::Deserialize)]
pub(super) struct GoogleStoreRequest {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    #[serde(default)]
    scopes: String,
}

/// GET /auth/google/status — Check Google OAuth2 status.
pub(super) async fn google_auth_status(
    State(state): State<Arc<AdminState>>,
) -> Json<GoogleAuthStatusResponse> {
    let mgr_clone = resolve_live_google_auth_manager(&state);
    match mgr_clone {
        None => Json(GoogleAuthStatusResponse {
            configured: false,
            scopes: None,
            token_valid: None,
            error: None,
        }),
        Some(mgr) => {
            let scopes = mgr.scopes().to_vec();
            // validate() does a blocking HTTP call — run in spawn_blocking
            let validation = tokio::task::spawn_blocking(move || mgr.validate()).await;
            match validation {
                Ok(Ok(())) => Json(GoogleAuthStatusResponse {
                    configured: true,
                    scopes: Some(scopes),
                    token_valid: Some(true),
                    error: None,
                }),
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, endpoint = "google_auth_status", "Token validation failed");
                    Json(GoogleAuthStatusResponse {
                        configured: true,
                        scopes: Some(scopes),
                        token_valid: Some(false),
                        error: Some("令牌验证失败。/ Token validation failed.".to_string()),
                    })
                }
                Err(e) => {
                    tracing::error!(error = %e, endpoint = "google_auth_status", "Validation task failed");
                    Json(GoogleAuthStatusResponse {
                        configured: true,
                        scopes: Some(scopes),
                        token_valid: Some(false),
                        error: Some("验证任务失败。/ Validation task failed.".to_string()),
                    })
                }
            }
        }
    }
}

/// POST /auth/google/revoke — Revoke Google OAuth2 tokens.
pub(super) async fn google_auth_revoke(
    State(state): State<Arc<AdminState>>,
    body: Option<Json<GoogleRevokeRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (db_path, aes_key) = match (&state.credential_db_path, &state.credential_aes_key) {
        (Some(p), Some(k)) => (p.clone(), *k),
        _ => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Credential store not configured".to_string(),
            ));
        }
    };
    let account_name = body
        .and_then(|Json(body)| body.account_name)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "primary".to_string());

    let mgr_clone = resolve_live_google_auth_manager(&state);
    if let Some(mgr) = mgr_clone
        && let Err(e) = tokio::task::spawn_blocking(move || {
            if let Err(e) = mgr.revoke() {
                tracing::warn!(error = %e, "Token revocation at Google failed (proceeding with local cleanup)");
            }
        })
        .await {
            tracing::warn!(error = %e, "Google token revocation task panicked");
        }

    // Delete from credential store (blocking — rusqlite is !Send)
    let account_name_for_delete = account_name.clone();
    tokio::task::spawn_blocking(move || {
        let cred_store = crate::credential_store::CredentialStore::open(
            crate::encrypted_sqlite::open(&db_path)
                .map_err(|e| anyhow::anyhow!("DB open: {e}"))?,
            aes_key,
        )?;
        GoogleAuthManager::delete_from_credential_store_account(&cred_store, &account_name_for_delete)
    })
    .await
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_revoke", "Credential delete task failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "凭据删除失败，请重试。/ Failed to delete credentials. Please try again.".to_string())
    })?
    .map_err(|e| {
        tracing::error!(error = %e, endpoint = "google_auth_revoke", "Failed to delete credentials");
        (StatusCode::INTERNAL_SERVER_ERROR, "凭据删除失败，请重试。/ Failed to delete credentials. Please try again.".to_string())
    })?;

    state.sandbox.clear_dynamic_auth(&["www.googleapis.com"]);
    *state.google_auth.write() = None;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "account_name": account_name,
        "message": "Google OAuth2 tokens revoked and deleted for selected account",
    })))
}

#[cfg(test)]
mod health_signal_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

    fn test_engine() -> wasmtime::Engine {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        wasmtime::Engine::new(&config).expect("test engine init")
    }

    /// Minimal `AdminState` (mirrors `admin_tests::test_state`), optionally
    /// carrying an orchestrator whose degraded snapshots the test can flip.
    fn state_with_orchestrator(
        with_orchestrator: bool,
    ) -> (
        Arc<AdminState>,
        Option<Arc<crate::orchestrator::Orchestrator>>,
    ) {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        let orchestrator = with_orchestrator.then(|| {
            // Port 1 is never listened on: no request in these tests reaches
            // the network, the orchestrator only holds the degraded snapshots.
            let api_base = "http://127.0.0.1:1/v1";
            let embedder =
                crate::embedding::EmbeddingClient::new(api_base, "test-key", "test-embed", 384);
            Arc::new(crate::orchestrator::Orchestrator::new(
                "test-key",
                api_base,
                "test-model",
                Arc::new(SandboxEngine::from_engine(test_engine()).unwrap()),
                Arc::new(AgentIdentity::generate()),
                memory.clone(),
                embedder,
                settings.clone(),
            ))
        });
        let state = Arc::new(AdminState {
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
            bot_token: None,
            admin_chat_id: None,
            orchestrator: orchestrator.clone(),
            event_tx: None,
            db_path: None,
            progress_router: None,
            local_login_token: None,
        });
        (state, orchestrator)
    }

    async fn fetch_health(state: Arc<AdminState>) -> (StatusCode, serde_json::Value) {
        let app = admin_router(state);
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    #[test]
    fn health_status_label_folds_runtime_signals() {
        assert_eq!(health_status_label(true, false), "ok");
        assert_eq!(health_status_label(true, true), "degraded");
        assert_eq!(health_status_label(false, false), "degraded");
        assert_eq!(health_status_label(false, true), "degraded");
    }

    #[tokio::test]
    async fn health_always_carries_the_runtime_signal_keys() {
        let _telegram = crate::telegram_health::test_guard();
        let (state, _) = state_with_orchestrator(false);

        let (status, json) = fetch_health(state).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        let components = json["components"].as_object().expect("components object");
        // Without an orchestrator (or when healthy) the signals are explicit
        // nulls, never missing keys: a dashboard must be able to tell "healthy"
        // from "this build does not report it".
        for key in ["llm_degraded", "embedder_degraded"] {
            assert!(
                components.contains_key(key),
                "components.{key} missing: {json}"
            );
            assert!(
                components[key].is_null(),
                "components.{key} should be null: {json}"
            );
        }
        // The nightly verdict is read from the deployment's status file, so
        // this environment-dependent field is only pinned to "present and
        // either null or an object with a verdict".
        assert!(
            components.contains_key("nightly"),
            "components.nightly missing: {json}"
        );
        let nightly = &components["nightly"];
        assert!(
            nightly.is_null() || nightly["verdict"].is_string(),
            "components.nightly must be null or a verdict object: {nightly}"
        );
        assert!(
            components.contains_key("secure_computer_use"),
            "components.secure_computer_use missing: {json}"
        );
        assert!(
            components["secure_computer_use"]["enabled"].is_boolean(),
            "components.secure_computer_use.enabled missing: {json}"
        );
    }

    #[tokio::test]
    async fn health_reports_degraded_while_the_llm_provider_is_exhausted() {
        let _telegram = crate::telegram_health::test_guard();
        let (state, orchestrator) = state_with_orchestrator(true);
        let orchestrator = orchestrator.expect("orchestrator");

        // Negative control: a healthy orchestrator keeps the status at ok.
        let (status, json) = fetch_health(state.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert!(json["components"]["llm_degraded"].is_null());

        *orchestrator.llm_degraded.write() = Some(crate::orchestrator::LlmDegraded {
            reason: "Codex usage limit".into(),
            since: "2026-09-16T01:00:00Z".into(),
            provider: "codex_oauth".into(),
        });
        let (status, json) = fetch_health(state.clone()).await;
        // Still HTTP 200 (deploy/smoke use `curl -f`); the label flips.
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "degraded");
        assert_eq!(
            json["components"]["llm_degraded"]["reason"],
            "Codex usage limit"
        );
        assert_eq!(
            json["components"]["llm_degraded"]["provider"],
            "codex_oauth"
        );
        assert_eq!(
            json["components"]["llm_degraded"]["since"],
            "2026-09-16T01:00:00Z"
        );
        assert!(json["components"]["embedder_degraded"].is_null());

        // Recovery clears the snapshot and the label follows.
        *orchestrator.llm_degraded.write() = None;
        let (_, json) = fetch_health(state).await;
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn health_reports_degraded_while_the_embedder_is_down() {
        let _telegram = crate::telegram_health::test_guard();
        let (state, orchestrator) = state_with_orchestrator(true);
        let orchestrator = orchestrator.expect("orchestrator");

        orchestrator.mark_embedder_degraded("text-embedding-primary".into(), None);
        let (status, json) = fetch_health(state.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "degraded");
        let embedder = &json["components"]["embedder_degraded"];
        assert_eq!(embedder["primary_model"], "text-embedding-primary");
        assert!(embedder["backup_model"].is_null());
        assert!(embedder["since"].is_string());
        assert!(json["components"]["llm_degraded"].is_null());

        orchestrator.clear_embedder_degraded();
        let (_, json) = fetch_health(state).await;
        assert_eq!(json["status"], "ok");
        assert!(json["components"]["embedder_degraded"].is_null());
    }

    #[test]
    fn nightly_status_reads_a_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nightly-status.json");
        std::fs::write(
            &path,
            r#"{"verdict":"red","sha":" 02c0d462 ","finished_at_unix":1789700000,
                "failed_steps":["cargo test","ui-scenarios"],
                "report":"/opt/aegis-runtime/shared/nightly/2026-09-16.md"}"#,
        )
        .unwrap();

        let parsed = read_nightly_status_from(&path).expect("valid file parses");
        assert_eq!(parsed.verdict, "RED", "verdict is normalised to upper case");
        assert_eq!(parsed.sha, "02c0d462");
        assert_eq!(parsed.finished_at_unix, 1789700000);
        assert_eq!(parsed.failed_steps, vec!["cargo test", "ui-scenarios"]);
        assert_eq!(
            parsed.report.as_deref(),
            Some("/opt/aegis-runtime/shared/nightly/2026-09-16.md")
        );

        // The optional fields really are optional: the minimal shape parses.
        std::fs::write(&path, r#"{"verdict":"GREEN","finished_at_unix":1}"#).unwrap();
        let parsed = read_nightly_status_from(&path).expect("minimal file parses");
        assert_eq!(parsed.verdict, "GREEN");
        assert_eq!(parsed.sha, "");
        assert!(parsed.failed_steps.is_empty());
        assert_eq!(parsed.report, None);
        assert_eq!(parsed.verdict, "GREEN");
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json["verdict"], "GREEN");
        assert_eq!(json["failed_steps"], serde_json::json!([]));
        assert!(json["report"].is_null());
    }

    #[test]
    fn nightly_status_is_none_for_missing_or_garbage_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nightly-status.json");

        // Missing file: the common case on a fresh install.
        assert_eq!(read_nightly_status_from(&path), None);

        // Garbage: a half-written file must not fail /health.
        std::fs::write(&path, "{\"verdict\":\"GRE").unwrap();
        assert_eq!(read_nightly_status_from(&path), None);
        std::fs::write(&path, "not json at all").unwrap();
        assert_eq!(read_nightly_status_from(&path), None);

        // Wrong shape: JSON, but not the nightly document.
        std::fs::write(&path, r#"{"status":"ok"}"#).unwrap();
        assert_eq!(read_nightly_status_from(&path), None);
        std::fs::write(&path, r#"{"verdict":"   ","finished_at_unix":1}"#).unwrap();
        assert_eq!(
            read_nightly_status_from(&path),
            None,
            "blank verdict is unusable"
        );
        std::fs::write(&path, r#"[{"verdict":"GREEN","finished_at_unix":1}]"#).unwrap();
        assert_eq!(read_nightly_status_from(&path), None);

        // A directory at the path, or an oversized file, is ignored too.
        assert_eq!(read_nightly_status_from(dir.path()), None);
        let big = format!(
            r#"{{"verdict":"GREEN","finished_at_unix":1,"sha":"{}"}}"#,
            "x".repeat(NIGHTLY_STATUS_MAX_BYTES as usize + 1)
        );
        std::fs::write(&path, big).unwrap();
        assert_eq!(read_nightly_status_from(&path), None);
    }

    #[test]
    fn nightly_status_path_defaults_to_the_shared_directory() {
        // Only meaningful when the operator has not overridden it (tests do
        // not mutate process env; see sandbox_network for why).
        if std::env::var_os("AEGIS_NIGHTLY_STATUS_FILE").is_none() {
            assert_eq!(
                nightly_status_path(),
                PathBuf::from(DEFAULT_NIGHTLY_STATUS_FILE)
            );
        }
    }
}
