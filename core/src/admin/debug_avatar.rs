use super::*;

pub(super) fn webapp_debug_snapshot_db_path(state: &AdminState) -> String {
    state
        .db_path
        .clone()
        .unwrap_or_else(|| state.memory.db_path().to_string())
}

pub(super) fn webapp_debug_snapshots_enabled() -> bool {
    #[cfg(test)]
    if WEBAPP_DEBUG_SNAPSHOTS_TEST_REFCOUNT.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        return true;
    }
    webapp_debug_snapshots_enabled_from_lookup(|key| std::env::var(key).ok())
}

pub(super) fn webapp_debug_snapshots_enabled_from_lookup(
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> bool {
    lookup("AEGIS_WEBAPP_DEBUG_SNAPSHOTS")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(super) fn require_webapp_debug_snapshots_enabled() -> Result<(), (StatusCode, String)> {
    if webapp_debug_snapshots_enabled() {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            "WebApp debug snapshots are disabled".to_string(),
        ))
    }
}

pub(super) fn init_webapp_debug_snapshot_schema(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webapp_debug_snapshots (
            id TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            principal_type TEXT NOT NULL,
            surface TEXT NOT NULL,
            path TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            payload_bytes INTEGER NOT NULL,
            created_at_unix INTEGER NOT NULL,
            expires_at_unix INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_snapshots_expires
            ON webapp_debug_snapshots(expires_at_unix);
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_snapshots_owner_created
            ON webapp_debug_snapshots(owner, created_at_unix DESC);",
    )
}

pub(super) fn init_webapp_debug_lease_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webapp_debug_leases (
            id TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            principal_type TEXT NOT NULL,
            scope TEXT NOT NULL,
            permissions_json TEXT NOT NULL,
            token_hash TEXT,
            item_id TEXT,
            reason TEXT NOT NULL,
            created_at_unix INTEGER NOT NULL,
            expires_at_unix INTEGER NOT NULL,
            revoked_at_unix INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_leases_owner_active
            ON webapp_debug_leases(owner, expires_at_unix, revoked_at_unix);
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_leases_expires
            ON webapp_debug_leases(expires_at_unix);",
    )?;
    ensure_webapp_debug_lease_column(conn, "token_hash", "TEXT")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_webapp_debug_leases_token_hash
            ON webapp_debug_leases(token_hash);",
    )
}

pub(super) fn init_webapp_debug_job_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webapp_debug_jobs (
            id TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            principal_type TEXT NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            log_text TEXT NOT NULL,
            exit_code INTEGER,
            lease_id TEXT,
            created_at_unix INTEGER NOT NULL,
            started_at_unix INTEGER,
            completed_at_unix INTEGER,
            expires_at_unix INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_jobs_owner_created
            ON webapp_debug_jobs(owner, created_at_unix DESC);
        CREATE INDEX IF NOT EXISTS idx_webapp_debug_jobs_expires
            ON webapp_debug_jobs(expires_at_unix);",
    )
}

fn ensure_webapp_debug_lease_column(
    conn: &rusqlite::Connection,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(webapp_debug_leases)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing?.eq_ignore_ascii_case(column) {
            return Ok(());
        }
    }
    conn.execute_batch(&format!(
        "ALTER TABLE webapp_debug_leases ADD COLUMN {column} {column_type};"
    ))
}

pub(super) fn cleanup_expired_webapp_debug_snapshots(
    conn: &rusqlite::Connection,
    now_unix: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM webapp_debug_snapshots WHERE expires_at_unix <= ?1",
        [now_unix],
    )
}

pub(super) fn cleanup_expired_webapp_debug_leases(
    conn: &rusqlite::Connection,
    now_unix: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM webapp_debug_leases WHERE expires_at_unix <= ?1",
        [now_unix],
    )
}

pub(super) fn cleanup_expired_webapp_debug_jobs(
    conn: &rusqlite::Connection,
    now_unix: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM webapp_debug_jobs WHERE expires_at_unix <= ?1",
        [now_unix],
    )
}

pub(super) fn sanitize_debug_lease_scope(scope: &str) -> String {
    let normalized = scope.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "avatar" | "mail" | "reminders" | "work_item" | "chat" | "settings" | "notifications"
        | "provider" | "status" => normalized,
        _ => "general".into(),
    }
}

pub(super) fn sanitize_debug_job_kind(kind: &str) -> Option<String> {
    let normalized = kind.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "webapp_e2e_suite" | "e2e_suite" | "support_e2e" | "full_e2e" => {
            Some(WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE.into())
        }
        _ => None,
    }
}

pub(super) fn sanitize_debug_lease_permissions(permissions: Option<Vec<String>>) -> Vec<String> {
    let mut output = Vec::new();
    for permission in permissions.unwrap_or_else(|| vec!["inspect".into()]) {
        let normalized = permission.trim().to_ascii_lowercase().replace('-', "_");
        if matches!(
            normalized.as_str(),
            "inspect" | "replay_safe" | "replay_mutating"
        ) && !output.iter().any(|existing| existing == &normalized)
        {
            output.push(normalized);
        }
    }
    if output.is_empty() {
        output.push("inspect".into());
    }
    output
}

pub(super) fn sanitize_debug_lease_item_id(item_id: Option<String>) -> Option<String> {
    item_id.and_then(|raw| {
        let value = raw.trim();
        if value.is_empty() {
            return None;
        }
        let sanitized: String = value
            .chars()
            .take(160)
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '_' | '.' | '/'))
            .collect();
        (!sanitized.is_empty()).then_some(sanitized)
    })
}

pub(super) fn sanitize_debug_lease_reason(reason: Option<String>) -> String {
    let value = reason.unwrap_or_default();
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        "Owner granted short-lived diagnostic access.".into()
    } else {
        truncate_chars(trimmed, 240)
    }
}

pub(super) fn normalize_debug_lease_duration_minutes(duration_minutes: Option<u32>) -> u32 {
    duration_minutes
        .unwrap_or(WEBAPP_DEBUG_LEASE_DEFAULT_MINUTES)
        .clamp(
            WEBAPP_DEBUG_LEASE_MIN_MINUTES,
            WEBAPP_DEBUG_LEASE_MAX_MINUTES,
        )
}

pub(super) fn read_debug_lease_summary(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WebappDebugLeaseSummary> {
    let permissions_json: String = row.get(4)?;
    let permissions = serde_json::from_str::<Vec<String>>(&permissions_json)
        .unwrap_or_else(|_| vec!["inspect".into()]);
    Ok(WebappDebugLeaseSummary {
        id: row.get(0)?,
        owner: row.get(1)?,
        principal_type: row.get(2)?,
        scope: row.get(3)?,
        permissions,
        item_id: row.get(5)?,
        reason: row.get(6)?,
        created_at_unix: row.get(7)?,
        expires_at_unix: row.get(8)?,
        revoked_at_unix: row.get(9)?,
    })
}

pub(super) fn read_debug_job_summary(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WebappDebugJobSummary> {
    let kind: String = row.get(3)?;
    let status: String = row.get(4)?;
    let summary: String = row.get(5)?;
    let log_text: String = row.get(6)?;
    let exit_code: Option<i64> = row.get(7)?;
    let evidence = webapp_debug_job_evidence_for(&kind, &status, &summary, &log_text, exit_code);
    Ok(WebappDebugJobSummary {
        id: row.get(0)?,
        owner: row.get(1)?,
        principal_type: row.get(2)?,
        kind,
        status,
        summary,
        log_text,
        exit_code,
        lease_id: row.get(8)?,
        created_at_unix: row.get(9)?,
        started_at_unix: row.get(10)?,
        completed_at_unix: row.get(11)?,
        expires_at_unix: row.get(12)?,
        evidence,
    })
}

pub(super) fn principal_debug_owner(principal: &WebConsolePrincipal) -> (String, String) {
    match principal {
        WebConsolePrincipal::Telegram(verified) => {
            (verified.user.id.to_string(), "telegram".into())
        }
        WebConsolePrincipal::WebConsole(session) => (session.email.clone(), "web_console".into()),
        #[cfg(test)]
        WebConsolePrincipal::DebugLease(lease) => (
            lease.owner.clone(),
            format!("debug_lease:{}", lease.principal_type),
        ),
    }
}

pub(super) fn mint_webapp_debug_grant_token() -> String {
    let secret =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    format!("{WEBAPP_DEBUG_GRANT_TOKEN_PREFIX}{secret}")
}

pub(super) fn webapp_debug_grant_token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"aegis:webapp-debug-grant:v1:");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
pub(super) fn normalize_webapp_debug_grant_token(raw: &str) -> Option<String> {
    let token = raw.trim();
    if token.len() <= WEBAPP_DEBUG_GRANT_TOKEN_PREFIX.len()
        || token.len() > WEBAPP_DEBUG_GRANT_TOKEN_MAX_CHARS
        || !token.starts_with(WEBAPP_DEBUG_GRANT_TOKEN_PREFIX)
        || !token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }
    Some(token.to_string())
}

pub(super) fn webapp_debug_job_base_url(state: &AdminState) -> Option<String> {
    state
        .public_url
        .clone()
        .or_else(|| std::env::var("AEGIS_PUBLIC_URL").ok())
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn webapp_debug_job_repo_root() -> PathBuf {
    std::env::var("AEGIS_REPO_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("CARGO_MANIFEST_DIR")
                .ok()
                .map(|value| FsPath::new(&value).join(".."))
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(super) fn webapp_debug_job_summary_for_kind(kind: &str, status: &str) -> String {
    match (kind, status) {
        (WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE, WEBAPP_DEBUG_JOB_STATUS_QUEUED) => {
            "Queued WebApp E2E support suite.".into()
        }
        (WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE, WEBAPP_DEBUG_JOB_STATUS_RUNNING) => {
            "Running WebApp E2E support suite.".into()
        }
        _ => "Debug job queued.".into(),
    }
}

pub(super) fn webapp_debug_job_evidence_for(
    kind: &str,
    status: &str,
    summary: &str,
    log_text: &str,
    exit_code: Option<i64>,
) -> WebappDebugJobEvidence {
    let verdict = match (status, kind) {
        (WEBAPP_DEBUG_JOB_STATUS_PASSED, WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE) => "deploy_ready",
        (WEBAPP_DEBUG_JOB_STATUS_PASSED, _) => "path_passed",
        (WEBAPP_DEBUG_JOB_STATUS_FAILED, _) => "blocked",
        (WEBAPP_DEBUG_JOB_STATUS_RUNNING, _) => "running",
        (WEBAPP_DEBUG_JOB_STATUS_QUEUED, _) => "queued",
        _ => "unknown",
    }
    .to_string();
    let phases = match kind {
        WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE => vec![webapp_debug_job_evidence_phase(
            "production_public_auth",
            "Production public/auth",
            webapp_debug_job_phase_status(
                status,
                summary,
                log_text,
                "PASSED: WebApp production public/auth smoke OK",
                "production public/auth smoke",
            ),
            "Public shell, auth boundaries, security headers, assets, OAuth redirect, and debug-job route gates.",
        )],
        _ => Vec::new(),
    };

    WebappDebugJobEvidence {
        schema_version: 1,
        verdict,
        owner_summary: webapp_debug_job_evidence_owner_summary(
            kind, status, summary, &phases, exit_code,
        ),
        phases,
    }
}

fn webapp_debug_job_evidence_phase(
    id: &str,
    label: &str,
    status: String,
    passed_summary: &str,
) -> WebappDebugJobEvidencePhase {
    let summary = match status.as_str() {
        WEBAPP_DEBUG_JOB_STATUS_PASSED => passed_summary.to_string(),
        WEBAPP_DEBUG_JOB_STATUS_FAILED => {
            "This phase failed. Open the raw log for the first failing command and stderr.".into()
        }
        WEBAPP_DEBUG_JOB_STATUS_RUNNING => "This phase is running now.".into(),
        WEBAPP_DEBUG_JOB_STATUS_QUEUED => "This phase is waiting for the backend runner.".into(),
        _ => "No phase-specific evidence was found in the saved log.".into(),
    };
    WebappDebugJobEvidencePhase {
        id: id.into(),
        label: label.into(),
        status,
        summary,
    }
}

fn webapp_debug_job_phase_status(
    job_status: &str,
    job_summary: &str,
    log_text: &str,
    pass_marker: &str,
    failure_hint: &str,
) -> String {
    if log_text.contains(pass_marker) {
        return WEBAPP_DEBUG_JOB_STATUS_PASSED.into();
    }
    match job_status {
        WEBAPP_DEBUG_JOB_STATUS_QUEUED => WEBAPP_DEBUG_JOB_STATUS_QUEUED.into(),
        WEBAPP_DEBUG_JOB_STATUS_RUNNING => WEBAPP_DEBUG_JOB_STATUS_RUNNING.into(),
        WEBAPP_DEBUG_JOB_STATUS_FAILED
            if job_summary
                .to_ascii_lowercase()
                .contains(&failure_hint.to_ascii_lowercase()) =>
        {
            WEBAPP_DEBUG_JOB_STATUS_FAILED.into()
        }
        WEBAPP_DEBUG_JOB_STATUS_FAILED if log_text.contains("FAILED:") => "unknown".into(),
        WEBAPP_DEBUG_JOB_STATUS_PASSED => WEBAPP_DEBUG_JOB_STATUS_PASSED.into(),
        _ => "unknown".into(),
    }
}

fn webapp_debug_job_evidence_owner_summary(
    kind: &str,
    status: &str,
    summary: &str,
    phases: &[WebappDebugJobEvidencePhase],
    exit_code: Option<i64>,
) -> String {
    match status {
        WEBAPP_DEBUG_JOB_STATUS_PASSED if kind == WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE => {
            "Deploy readiness checks passed for public/auth boundaries and the Dev Orchestrator debug-grant path.".into()
        }
        WEBAPP_DEBUG_JOB_STATUS_PASSED => {
            "The checked path passed. Run the full WebApp E2E suite before treating this as deploy readiness.".into()
        }
        WEBAPP_DEBUG_JOB_STATUS_FAILED => {
            let failed_phase = phases
                .iter()
                .find(|phase| phase.status == WEBAPP_DEBUG_JOB_STATUS_FAILED)
                .map(|phase| phase.label.as_str())
                .unwrap_or("an unknown phase");
            match exit_code {
                Some(code) => {
                    format!("{summary} First failed phase: {failed_phase}. Exit code: {code}.")
                }
                None => format!("{summary} First failed phase: {failed_phase}."),
            }
        }
        WEBAPP_DEBUG_JOB_STATUS_RUNNING => {
            "Checks are running. Wait for the final verdict before using this as deployment evidence.".into()
        }
        WEBAPP_DEBUG_JOB_STATUS_QUEUED => {
            "Checks are queued. No deployment verdict is available yet.".into()
        }
        _ => summary.to_string(),
    }
}

/// Redact every `aegis_dbg_`-prefixed grant token out of a debug-job log before
/// it is persisted. Minted tokens always carry that prefix, so the scanner below
/// covers them without the caller having to name the token.
pub(super) fn redact_webapp_debug_job_log(raw: &str) -> String {
    let value = raw.to_string();
    let mut redacted = String::with_capacity(value.len());
    let mut remainder = value.as_str();
    while let Some(pos) = remainder.find(WEBAPP_DEBUG_GRANT_TOKEN_PREFIX) {
        redacted.push_str(&remainder[..pos]);
        let candidate = &remainder[pos..];
        let mut end = 0;
        for (idx, ch) in candidate.char_indices() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                end = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        redacted.push_str("aegis_dbg_[redacted]");
        remainder = &candidate[end..];
    }
    redacted.push_str(remainder);
    truncate_chars(&redacted, WEBAPP_DEBUG_JOB_LOG_MAX_CHARS)
}

/// Insert a queued WebApp debug job.
///
/// Until 2026-09-05 this also minted an internal, bearer-token-backed
/// `dev_orchestrator` lease so the spawned smoke could drive the Dev Harness
/// routes as the owner. Those routes are gone and the remaining job kind (the
/// production public/auth smoke) authenticates nothing, so no lease is created
/// and `webapp_debug_jobs.lease_id` stays NULL for new rows.
pub(super) fn insert_webapp_debug_job(
    conn: &mut rusqlite::Connection,
    owner: &str,
    principal_type: &str,
    kind: &str,
    now_unix: i64,
) -> rusqlite::Result<WebappDebugJobSummary> {
    init_webapp_debug_job_schema(conn)?;
    cleanup_expired_webapp_debug_jobs(conn, now_unix)?;

    let job_id = format!("debug-job-{}", uuid::Uuid::now_v7().simple());
    let expires_at_unix = now_unix + WEBAPP_DEBUG_JOB_TTL_SECS;
    conn.execute(
        "INSERT INTO webapp_debug_jobs (
            id, owner, principal_type, kind, status, summary, log_text, exit_code, lease_id,
            created_at_unix, started_at_unix, completed_at_unix, expires_at_unix
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', NULL, NULL, ?7, NULL, NULL, ?8)",
        rusqlite::params![
            job_id,
            owner,
            principal_type,
            kind,
            WEBAPP_DEBUG_JOB_STATUS_QUEUED,
            webapp_debug_job_summary_for_kind(kind, WEBAPP_DEBUG_JOB_STATUS_QUEUED),
            now_unix,
            expires_at_unix,
        ],
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, owner, principal_type, kind, status, summary, log_text, exit_code,
                lease_id, created_at_unix, started_at_unix, completed_at_unix, expires_at_unix
         FROM webapp_debug_jobs
         WHERE id = ?1",
    )?;
    stmt.query_row([job_id], read_debug_job_summary)
}

async fn update_webapp_debug_job_started(db_path: String, job_id: String, kind: String) {
    let now_unix = chrono::Utc::now().timestamp();
    if let Err(err) = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        init_webapp_debug_job_schema(&conn)?;
        conn.execute(
            "UPDATE webapp_debug_jobs
             SET status = ?1, summary = ?2, started_at_unix = COALESCE(started_at_unix, ?3)
             WHERE id = ?4",
            rusqlite::params![
                WEBAPP_DEBUG_JOB_STATUS_RUNNING,
                webapp_debug_job_summary_for_kind(&kind, WEBAPP_DEBUG_JOB_STATUS_RUNNING),
                now_unix,
                job_id,
            ],
        )?;
        Ok(())
    })
    .await
    {
        tracing::warn!(error = %err, "Debug job start marker task failed");
    }
}

async fn finish_webapp_debug_job(
    db_path: String,
    job_id: String,
    lease_id: Option<String>,
    outcome: WebappDebugJobRunOutcome,
) {
    let now_unix = chrono::Utc::now().timestamp();
    if let Err(err) = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        init_webapp_debug_job_schema(&conn)?;
        init_webapp_debug_lease_schema(&conn)?;
        conn.execute(
            "UPDATE webapp_debug_jobs
             SET status = ?1, summary = ?2, log_text = ?3, exit_code = ?4, completed_at_unix = ?5
             WHERE id = ?6",
            rusqlite::params![
                if outcome.passed {
                    WEBAPP_DEBUG_JOB_STATUS_PASSED
                } else {
                    WEBAPP_DEBUG_JOB_STATUS_FAILED
                },
                outcome.summary,
                outcome.log_text,
                outcome.exit_code,
                now_unix,
                job_id,
            ],
        )?;
        if let Some(lease_id) = lease_id {
            conn.execute(
                "UPDATE webapp_debug_leases
                 SET revoked_at_unix = COALESCE(revoked_at_unix, ?1)
                 WHERE id = ?2",
                rusqlite::params![now_unix, lease_id],
            )?;
        }
        Ok(())
    })
    .await
    {
        tracing::warn!(error = %err, "Debug job finish marker task failed");
    }
}

#[cfg(not(test))]
pub(super) fn spawn_webapp_debug_job_runner(
    config: WebappDebugJobRuntimeConfig,
    job: WebappDebugJobSummary,
) {
    tokio::spawn(async move {
        update_webapp_debug_job_started(
            config.db_path.clone(),
            job.id.clone(),
            config.kind.clone(),
        )
        .await;
        let outcome = run_webapp_debug_job(config.clone()).await;
        finish_webapp_debug_job(config.db_path, job.id, job.lease_id, outcome).await;
    });
}

#[cfg(test)]
pub(super) fn spawn_webapp_debug_job_runner(
    config: WebappDebugJobRuntimeConfig,
    job: WebappDebugJobSummary,
) {
    tokio::spawn(async move {
        update_webapp_debug_job_started(
            config.db_path.clone(),
            job.id.clone(),
            config.kind.clone(),
        )
        .await;
        let log_text = redact_webapp_debug_job_log(&format!(
            "test harness accepted {}\nbase_url={:?}\nrepo_root={}",
            config.kind,
            config.base_url,
            config.repo_root.display(),
        ));
        finish_webapp_debug_job(
            config.db_path,
            job.id,
            job.lease_id,
            WebappDebugJobRunOutcome {
                passed: true,
                summary: "Debug job test harness completed.".into(),
                log_text,
                exit_code: Some(0),
            },
        )
        .await;
    });
}

#[cfg(not(test))]
async fn run_webapp_debug_job(config: WebappDebugJobRuntimeConfig) -> WebappDebugJobRunOutcome {
    match config.kind.as_str() {
        WEBAPP_DEBUG_JOB_KIND_WEBAPP_E2E_SUITE => run_webapp_e2e_suite_debug_job(config).await,
        other => WebappDebugJobRunOutcome {
            passed: false,
            summary: format!("Unknown debug job kind: {other}"),
            log_text: format!("Rejected unknown debug job kind: {other}"),
            exit_code: None,
        },
    }
}

#[cfg(not(test))]
async fn run_webapp_e2e_suite_debug_job(
    config: WebappDebugJobRuntimeConfig,
) -> WebappDebugJobRunOutcome {
    let production = run_webapp_production_smoke_debug_job(config).await;
    let combined_log = format!(
        "=== WebApp E2E Support Suite ===\n\n--- Production public/auth smoke ---\n{}",
        production.log_text
    );

    WebappDebugJobRunOutcome {
        passed: production.passed,
        summary: if production.passed {
            "WebApp E2E support suite passed.".into()
        } else {
            "WebApp E2E support suite failed in production public/auth smoke.".into()
        },
        log_text: redact_webapp_debug_job_log(&combined_log),
        exit_code: if production.passed {
            Some(0)
        } else {
            production.exit_code
        },
    }
}

#[cfg(not(test))]
async fn run_webapp_production_smoke_debug_job(
    config: WebappDebugJobRuntimeConfig,
) -> WebappDebugJobRunOutcome {
    let Some(base_url) = config.base_url.clone() else {
        return webapp_debug_job_missing_base_url("WebApp production public/auth smoke");
    };

    let envs = [("AEGIS_WEBAPP_BASE_URL", base_url)];
    run_webapp_debug_job_script(
        &config,
        "test-webapp-production-smoke.sh",
        "WebApp production public/auth smoke",
        &envs,
        WEBAPP_DEBUG_JOB_SUITE_TIMEOUT_SECS,
    )
    .await
}

#[cfg(not(test))]
fn webapp_debug_job_missing_base_url(label: &str) -> WebappDebugJobRunOutcome {
    WebappDebugJobRunOutcome {
        passed: false,
        summary: format!("{label} could not run: public WebApp URL is not configured."),
        log_text: "AEGIS_PUBLIC_URL/public_url is not configured for the backend debug runner."
            .into(),
        exit_code: None,
    }
}

#[cfg(not(test))]
async fn run_webapp_debug_job_script(
    config: &WebappDebugJobRuntimeConfig,
    script_name: &str,
    label: &str,
    envs: &[(&str, String)],
    timeout_secs: u64,
) -> WebappDebugJobRunOutcome {
    let script_path = config.repo_root.join("scripts").join(script_name);
    if !script_path.is_file() {
        return WebappDebugJobRunOutcome {
            passed: false,
            summary: format!("{label} could not run: smoke script is missing."),
            log_text: format!(
                "Expected smoke script at {}. Check AEGIS_REPO_ROOT.",
                script_path.display()
            ),
            exit_code: None,
        };
    }

    let mut command = tokio::process::Command::new(&script_path);
    command
        .current_dir(&config.repo_root)
        .env_remove("AEGIS_WEBAPP_SESSION_COOKIE")
        .env_remove("AEGIS_WEBAPP_DEBUG_GRANT_HEADER")
        .env_remove("AEGIS_WEBAPP_DEBUG_GRANT_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }

    let command_display = format!("{} ({label})", script_path.display());
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => {
                return WebappDebugJobRunOutcome {
                    passed: false,
                    summary: format!("{label} failed to start: {err}"),
                    log_text: redact_webapp_debug_job_log(&format!(
                        "command: {command_display}\nspawn error: {err}"
                    )),
                    exit_code: None,
                };
            }
            Err(_) => {
                return WebappDebugJobRunOutcome {
                    passed: false,
                    summary: format!("{label} timed out."),
                    log_text: redact_webapp_debug_job_log(&format!(
                        "command: {command_display}\ntimeout after {timeout_secs} seconds"
                    )),
                    exit_code: None,
                };
            }
        };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let log_text = redact_webapp_debug_job_log(&format!(
        "command: {command_display}\n\n[stdout]\n{stdout}\n[stderr]\n{stderr}"
    ));
    let passed = output.status.success();
    WebappDebugJobRunOutcome {
        passed,
        summary: if passed {
            format!("{label} passed.")
        } else {
            format!("{label} failed.")
        },
        log_text,
        exit_code: output.status.code().map(i64::from),
    }
}

pub(super) fn sanitize_debug_snapshot_surface(surface: &str) -> String {
    let normalized = surface.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "home" | "work" | "receipts" | "instructions" | "status" | "settings" | "unknown" => {
            normalized
        }
        _ => "unknown".into(),
    }
}

pub(super) fn sanitize_debug_snapshot_path(path: &str) -> String {
    let value = path.trim();
    if value.is_empty() {
        return "/ui/".into();
    }

    let truncated = value.chars().take(2048).collect::<String>();
    let parsed = url::Url::parse(&truncated).or_else(|_| {
        url::Url::parse(&format!(
            "https://aegis.local{}",
            if truncated.starts_with('/') {
                truncated.clone()
            } else {
                format!("/{truncated}")
            }
        ))
    });

    if let Ok(url) = parsed {
        let mut sanitized = url.path().trim().chars().take(512).collect::<String>();
        if sanitized.is_empty() {
            sanitized = "/ui/".into();
        }
        if let Some(query) = url.query().filter(|query| !query.is_empty()) {
            sanitized.push_str("?query_hash=");
            sanitized.push_str(&short_hash_for_debug(query));
        }
        return sanitized;
    }

    let (without_fragment, _) = truncated.split_once('#').unwrap_or((&truncated, ""));
    let (raw_path, raw_query) = without_fragment
        .split_once('?')
        .map(|(path, query)| (path, Some(query)))
        .unwrap_or((without_fragment, None));
    let mut sanitized = raw_path.trim().chars().take(512).collect::<String>();
    if sanitized.is_empty() {
        sanitized = "/ui/".into();
    }
    if let Some(query) = raw_query.filter(|query| !query.is_empty()) {
        sanitized.push_str("?query_hash=");
        sanitized.push_str(&short_hash_for_debug(query));
    }
    sanitized
}

pub(super) fn short_hash_for_debug(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..8])
}

fn debug_snapshot_key_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "csrf",
        "token",
        "secret",
        "password",
        "credential",
        "refresh",
        "access_token",
        "html",
        "body",
        "attachment",
        "raw",
        "source_detail",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

pub(super) fn sanitize_debug_snapshot_payload(value: serde_json::Value) -> serde_json::Value {
    fn sanitize_with_key(key: Option<&str>, value: serde_json::Value) -> serde_json::Value {
        if key.is_some_and(debug_snapshot_key_is_sensitive) {
            return serde_json::Value::String("[redacted]".into());
        }

        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.into_iter()
                    .map(|(child_key, child_value)| {
                        let sanitized = sanitize_with_key(Some(&child_key), child_value);
                        (child_key, sanitized)
                    })
                    .collect(),
            ),
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .into_iter()
                    .take(120)
                    .map(|child| sanitize_with_key(None, child))
                    .collect(),
            ),
            serde_json::Value::String(text) => serde_json::Value::String(
                text.chars()
                    .take(WEBAPP_DEBUG_SNAPSHOT_MAX_STRING_CHARS)
                    .collect(),
            ),
            other => other,
        }
    }

    sanitize_with_key(None, value)
}

pub(super) fn read_debug_snapshot_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WebappDebugSnapshotRecord> {
    let payload_json: String = row.get(5)?;
    Ok(WebappDebugSnapshotRecord {
        id: row.get(0)?,
        owner: row.get(1)?,
        principal_type: row.get(2)?,
        surface: row.get(3)?,
        path: row.get(4)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| {
            serde_json::json!({
                "error": "payload_json_parse_failed"
            })
        }),
        payload_bytes: row.get(6)?,
        created_at_unix: row.get(7)?,
        expires_at_unix: row.get(8)?,
    })
}

pub(super) fn read_debug_snapshot_summary(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WebappDebugSnapshotSummary> {
    Ok(WebappDebugSnapshotSummary {
        id: row.get(0)?,
        owner: row.get(1)?,
        principal_type: row.get(2)?,
        surface: row.get(3)?,
        path: row.get(4)?,
        payload_bytes: row.get(5)?,
        created_at_unix: row.get(6)?,
        expires_at_unix: row.get(7)?,
    })
}

fn init_webapp_avatar_profile_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS webapp_avatar_profiles (
            owner TEXT PRIMARY KEY,
            aegis_avatar_url TEXT NOT NULL,
            owner_avatar_url TEXT NOT NULL,
            updated_at_unix INTEGER NOT NULL
        );",
    )
}

fn default_vnext_avatar_profile() -> VNextAvatarProfileProjection {
    VNextAvatarProfileProjection {
        schema_version: 1,
        aegis_avatar_url: WEBAPP_DEFAULT_AEGIS_AVATAR_URL.into(),
        owner_avatar_url: String::new(),
        updated_at_unix: None,
        auto_refresh: default_vnext_avatar_auto_refresh_projection(),
    }
}

fn default_vnext_avatar_auto_refresh_projection() -> VNextAvatarAutoRefreshProjection {
    VNextAvatarAutoRefreshProjection {
        enabled: false,
        provider: "xai_subscription".into(),
        interval_days: 7,
        image_generation_available: false,
        status: "disabled".into(),
        owner_message: "Weekly self-avatar refresh is off.".into(),
        owner_next_step: Some(
            "Enable it after connecting an image-capable subscription provider.".into(),
        ),
        last_refresh_at_unix: None,
        next_refresh_at_unix: None,
        prompt_hint: String::new(),
    }
}

fn load_vnext_avatar_profile(
    conn: &rusqlite::Connection,
    owner: &str,
) -> rusqlite::Result<VNextAvatarProfileProjection> {
    init_webapp_avatar_profile_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT aegis_avatar_url, owner_avatar_url, updated_at_unix
         FROM webapp_avatar_profiles
         WHERE owner = ?1",
    )?;
    let mut rows = stmt.query([owner])?;
    let Some(row) = rows.next()? else {
        return Ok(default_vnext_avatar_profile());
    };
    Ok(VNextAvatarProfileProjection {
        schema_version: 1,
        aegis_avatar_url: row.get(0)?,
        owner_avatar_url: row.get(1)?,
        updated_at_unix: Some(row.get(2)?),
        auto_refresh: default_vnext_avatar_auto_refresh_projection(),
    })
}

fn store_vnext_avatar_profile(
    conn: &rusqlite::Connection,
    owner: &str,
    profile: &VNextAvatarProfileProjection,
) -> rusqlite::Result<()> {
    init_webapp_avatar_profile_schema(conn)?;
    let updated_at = profile
        .updated_at_unix
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    conn.execute(
        "INSERT INTO webapp_avatar_profiles
            (owner, aegis_avatar_url, owner_avatar_url, updated_at_unix)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner) DO UPDATE SET
            aegis_avatar_url = excluded.aegis_avatar_url,
            owner_avatar_url = excluded.owner_avatar_url,
            updated_at_unix = excluded.updated_at_unix",
        rusqlite::params![
            owner,
            &profile.aegis_avatar_url,
            &profile.owner_avatar_url,
            updated_at
        ],
    )?;
    Ok(())
}

fn list_vnext_avatar_profile_owners(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    init_webapp_avatar_profile_schema(conn)?;
    let mut stmt =
        conn.prepare("SELECT owner FROM webapp_avatar_profiles ORDER BY updated_at_unix DESC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

fn decode_avatar_data_url(
    value: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, &'static str, &'static str), (StatusCode, String)> {
    if value.len() > max_bytes.saturating_mul(2) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Avatar image is too large".to_string(),
        ));
    }
    let Some((prefix, encoded)) = value.split_once(',') else {
        return Err((StatusCode::BAD_REQUEST, "Invalid avatar image".to_string()));
    };
    let (mime, extension) = match prefix.to_ascii_lowercase().as_str() {
        "data:image/png;base64" => ("image/png", "png"),
        "data:image/jpeg;base64" | "data:image/jpg;base64" => ("image/jpeg", "jpg"),
        "data:image/webp;base64" => ("image/webp", "webp"),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Avatar image must be PNG, JPEG, or WebP".to_string(),
            ));
        }
    };
    if encoded.is_empty()
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid avatar image".to_string()));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid avatar image".to_string()))?;
    if decoded.len() > max_bytes {
        return Err((
            StatusCode::BAD_REQUEST,
            "Avatar image is too large".to_string(),
        ));
    }
    Ok((decoded, mime, extension))
}

fn validate_avatar_data_url(value: &str) -> Result<(), (StatusCode, String)> {
    decode_avatar_data_url(value, WEBAPP_AVATAR_MAX_DATA_URL_BYTES).map(|_| ())
}

pub(super) fn webapp_generated_avatar_owner_hash(owner: &str) -> String {
    let digest = Sha256::digest(owner.trim().to_ascii_lowercase().as_bytes());
    hex_prefix(&digest, 16)
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| {
            let high = byte >> 4;
            let low = byte & 0x0f;
            [hex_digit(high), hex_digit(low)]
        })
        .take(chars)
        .collect()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + (value - 10)) as char,
    }
}

pub(super) fn webapp_generated_avatar_storage_dir(state: &AdminState) -> std::path::PathBuf {
    if let Ok(path) = std::env::var("AEGIS_WEBAPP_GENERATED_AVATAR_DIR") {
        return path.into();
    }
    let db_path = webapp_debug_snapshot_db_path(state);
    FsPath::new(&db_path)
        .parent()
        .map(|parent| parent.join("webapp_generated").join("avatars"))
        .unwrap_or_else(|| std::path::PathBuf::from("webapp_generated/avatars"))
}

pub(super) fn is_safe_generated_avatar_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 160
        && !filename.contains("..")
        && filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && matches!(
            filename.rsplit('.').next(),
            Some("png") | Some("jpg") | Some("jpeg") | Some("webp")
        )
}

pub(super) fn generated_avatar_content_type(filename: &str) -> &'static str {
    match filename.rsplit('.').next() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

fn sniff_avatar_image_kind(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"RIFF")
        && bytes.len() >= 12
        && bytes.get(8..12) == Some(&b"WEBP"[..])
    {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

pub(super) fn validate_generated_avatar_url(value: &str) -> bool {
    let Some(filename) = value.strip_prefix(WEBAPP_GENERATED_AVATAR_URL_PREFIX) else {
        return false;
    };
    is_safe_generated_avatar_filename(filename)
}

pub(super) fn store_generated_aegis_avatar_image_for_owner(
    state: &AdminState,
    owner: &str,
    data_url: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, (StatusCode, String)> {
    let (bytes, _declared_mime, declared_extension) =
        decode_avatar_data_url(data_url, WEBAPP_GENERATED_AVATAR_MAX_BYTES)?;
    let (_mime, extension) =
        sniff_avatar_image_kind(&bytes).unwrap_or(("application/octet-stream", declared_extension));
    let owner_hash = webapp_generated_avatar_owner_hash(owner);
    let image_hash = hex_prefix(&Sha256::digest(&bytes), 16);
    let filename = format!(
        "aegis-{}-{}-{}.{}",
        owner_hash,
        now.timestamp_millis(),
        image_hash,
        extension
    );
    let dir = webapp_generated_avatar_storage_dir(state);
    std::fs::create_dir_all(&dir).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to prepare generated avatar storage: {err}"),
        )
    })?;
    let path = dir.join(&filename);
    std::fs::write(&path, bytes).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save generated avatar image: {err}"),
        )
    })?;
    Ok(format!("{WEBAPP_GENERATED_AVATAR_URL_PREFIX}{filename}"))
}

pub(super) fn validate_vnext_avatar_url(
    raw: &str,
    allow_default_aegis_asset: bool,
) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if allow_default_aegis_asset
        && (value == WEBAPP_DEFAULT_AEGIS_AVATAR_URL
            || WEBAPP_LEGACY_AEGIS_AVATAR_URLS.contains(&value))
    {
        return Ok(WEBAPP_DEFAULT_AEGIS_AVATAR_URL.into());
    }
    if validate_generated_avatar_url(value) {
        return Ok(value.into());
    }
    validate_avatar_data_url(value)?;
    Ok(value.into())
}

pub(super) fn validate_vnext_avatar_update(
    request: VNextAvatarProfileUpdateRequest,
) -> Result<VNextAvatarProfileProjection, (StatusCode, String)> {
    validate_context_schema_version(request.schema_version)?;
    let aegis_avatar_url = validate_vnext_avatar_url(&request.aegis_avatar_url, true)?;
    let owner_avatar_url = validate_vnext_avatar_url(&request.owner_avatar_url, false)?;
    Ok(VNextAvatarProfileProjection {
        schema_version: 1,
        aegis_avatar_url: if aegis_avatar_url.is_empty() {
            WEBAPP_DEFAULT_AEGIS_AVATAR_URL.into()
        } else {
            aegis_avatar_url
        },
        owner_avatar_url,
        updated_at_unix: Some(chrono::Utc::now().timestamp()),
        auto_refresh: default_vnext_avatar_auto_refresh_projection(),
    })
}

fn vnext_avatar_setting_bool(state: &AdminState, key: &str, fallback: bool) -> bool {
    state
        .settings
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(fallback)
}

fn vnext_avatar_setting_u32(state: &AdminState, key: &str, fallback: u32) -> u32 {
    state
        .settings
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(fallback)
}

fn vnext_avatar_setting_i64(state: &AdminState, key: &str, fallback: i64) -> i64 {
    state
        .settings
        .get(key)
        .and_then(|value| value.as_i64())
        .unwrap_or(fallback)
}

pub(super) fn vnext_avatar_setting_string(state: &AdminState, key: &str, fallback: &str) -> String {
    state
        .settings
        .get(key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| fallback.to_string())
}

pub(super) fn sanitize_avatar_prompt_hint(raw: &str) -> String {
    // One line, no control characters, capped — the owner's text goes straight
    // into the image prompt.
    let collapsed = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    collapsed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect::<String>()
        .trim_end_matches(['.', '。', ',', '，'])
        .to_string()
}

pub(super) fn vnext_avatar_next_refresh_at(
    last_refresh_at: Option<i64>,
    interval_days: u32,
) -> Option<i64> {
    let interval_secs = i64::from(interval_days.max(1)).saturating_mul(24 * 60 * 60);
    last_refresh_at.map(|last| last.saturating_add(interval_secs))
}

pub(super) fn vnext_avatar_auto_refresh_due(
    now_unix: i64,
    last_refresh_at: Option<i64>,
    interval_days: u32,
) -> bool {
    vnext_avatar_next_refresh_at(last_refresh_at, interval_days).is_none_or(|next| now_unix >= next)
}

pub(super) async fn vnext_avatar_auto_refresh_projection(
    state: &AdminState,
) -> VNextAvatarAutoRefreshProjection {
    let enabled = vnext_avatar_setting_bool(state, "aegis_avatar_auto_refresh_enabled", false);
    let provider = vnext_avatar_setting_string(
        state,
        "aegis_avatar_auto_refresh_provider",
        "xai_subscription",
    );
    let interval_days =
        vnext_avatar_setting_u32(state, "aegis_avatar_auto_refresh_interval_days", 7).clamp(1, 90);
    let last_raw = vnext_avatar_setting_i64(state, "aegis_avatar_auto_refresh_last_at_unix", 0);
    let last_refresh_at_unix = (last_raw > 0).then_some(last_raw);
    let next_refresh_at_unix = enabled
        .then_some(())
        .and_then(|_| vnext_avatar_next_refresh_at(last_refresh_at_unix, interval_days));
    let prompt_hint = sanitize_avatar_prompt_hint(&vnext_avatar_setting_string(
        state,
        "aegis_avatar_auto_refresh_prompt_hint",
        "",
    ));

    let mut image_generation_available = false;
    let mut status = if enabled { "attention" } else { "disabled" }.to_string();
    let owner_message: String;
    let mut owner_next_step = if enabled {
        Some("Connect or refresh an image-capable subscription provider.".to_string())
    } else {
        Some("Enable weekly refresh after connecting Grok image generation.".to_string())
    };

    if provider == "xai_subscription" {
        match vnext_xai_credential_store(state).and_then(|store| {
            crate::llm::XaiSubscriptionAuthManager::load_credential(&store).map_err(|err| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("Unable to inspect Grok credential: {}", err.message),
                )
            })
        }) {
            Ok(Some(credential))
                if !credential.needs_refresh(crate::llm::xai_subscription::now_ms()) =>
            {
                image_generation_available = true;
                status = if enabled { "ready" } else { "available" }.into();
                owner_message = if enabled {
                    "Aegis can refresh her avatar weekly through the connected Grok image capability. No paid API fallback is allowed.".into()
                } else {
                    "Grok image capability is connected. Weekly self-avatar refresh is off until you enable it.".into()
                };
                owner_next_step = if enabled {
                    next_refresh_at_unix.map_or_else(
                        || {
                            Some(
                                "First automatic refresh will run on the next background check."
                                    .into(),
                            )
                        },
                        |next| Some(format!("Next automatic refresh is due at Unix {next}.")),
                    )
                } else {
                    Some("Use Generate now or enable weekly refresh.".into())
                };
            }
            Ok(Some(_)) => {
                image_generation_available = true;
                status = if enabled { "ready" } else { "available" }.into();
                owner_message =
                    "Grok is connected. Aegis will refresh the access token before image generation; reconnect Grok only if that refresh fails."
                        .into();
                owner_next_step = Some(
                    "Use Generate Aegis avatar, or reconnect Grok if generation still fails."
                        .into(),
                );
            }
            Ok(None) => {
                owner_message =
                    "Grok image generation needs a connected Grok subscription first.".into();
            }
            Err((_, message)) => {
                owner_message = message;
            }
        }
    } else {
        owner_message =
            "Only xai_subscription is currently wired for subscription-first avatar images.".into();
        owner_next_step = Some("Switch the avatar provider to xai_subscription.".into());
    }

    VNextAvatarAutoRefreshProjection {
        enabled,
        provider,
        interval_days,
        image_generation_available,
        status,
        owner_message,
        owner_next_step,
        last_refresh_at_unix,
        next_refresh_at_unix,
        prompt_hint,
    }
}

pub(super) async fn load_vnext_avatar_profile_for_owner(
    state: &AdminState,
    owner: String,
) -> Result<VNextAvatarProfileProjection, (StatusCode, String)> {
    let db_path = webapp_debug_snapshot_db_path(state);
    let mut projection = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        Ok(load_vnext_avatar_profile(&conn, &owner)?)
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to load avatar profile: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to load avatar profile: {err}"),
        )
    })?;
    projection.auto_refresh = vnext_avatar_auto_refresh_projection(state).await;
    Ok(projection)
}

pub(super) async fn save_vnext_avatar_profile_for_owner(
    state: &AdminState,
    owner: String,
    profile: VNextAvatarProfileProjection,
) -> Result<VNextAvatarProfileProjection, (StatusCode, String)> {
    let db_path = webapp_debug_snapshot_db_path(state);
    let mut saved = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        store_vnext_avatar_profile(&conn, &owner, &profile)?;
        Ok(profile)
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save avatar profile: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save avatar profile: {err}"),
        )
    })?;
    saved.auto_refresh = vnext_avatar_auto_refresh_projection(state).await;
    Ok(saved)
}

/// Default subject when the owner has not described one (shown to the owner as
/// the field placeholder so they know what they are overriding).
pub(super) const AEGIS_AVATAR_DEFAULT_SUBJECT: &str = "an elegant female humanoid AI assistant in her 20s, soft studio lighting, clean neutral background";

pub(super) fn build_aegis_avatar_prompt(prompt_hint: &str) -> String {
    // Describe ONLY the desired visual. No proper nouns (Aegis Runtime/Aegis) and no
    // date — the image model renders any such strings as literal text inside the
    // avatar. The explicit negative instruction keeps the output free of any text,
    // logos, or watermarks.
    //
    // Owner 2026-09-12: the owner's own description IS the subject — it fully
    // replaces the default look rather than nudging it. Only the technical
    // framing (square, head-and-shoulders, no text) stays fixed.
    let hint = sanitize_avatar_prompt_hint(prompt_hint);
    let subject = if hint.is_empty() {
        AEGIS_AVATAR_DEFAULT_SUBJECT.to_string()
    } else {
        hint
    };
    format!(
        "Square profile avatar: {subject}. \
         Centered head-and-shoulders framing. \
         Do not include any text, letters, words, numbers, logos, or watermarks in the image."
    )
}

async fn load_ready_xai_subscription_credential_for_feature(
    state: &AdminState,
    feature: &str,
) -> Result<crate::llm::XaiSubscriptionCredential, (StatusCode, String)> {
    let store = vnext_xai_credential_store(state)?;
    let mut credential = crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(|err| (StatusCode::SERVICE_UNAVAILABLE, err.message))?
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Connect Grok before {feature}. No paid fallback was used."),
            )
        })?;
    if credential.needs_refresh(crate::llm::xai_subscription::now_ms()) {
        let manager = crate::llm::XaiSubscriptionAuthManager::new(Default::default());
        credential = manager
            .refresh_credential(&credential)
            .await
            .map_err(|err| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "Reconnect Grok in Settings before {feature}; automatic token refresh failed: {}",
                        err.message
                    ),
                )
            })?;
        crate::llm::XaiSubscriptionAuthManager::store_credential(&store, &credential).map_err(
            |err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to save refreshed Grok credential: {}", err.message),
                )
            },
        )?;
    }
    Ok(credential)
}

pub(super) async fn generate_and_store_aegis_avatar(
    state: &AdminState,
    owner: String,
    prompt_hint: String,
    automatic: bool,
) -> Result<VNextAegisAvatarGenerateResponse, (StatusCode, String)> {
    let provider = vnext_avatar_setting_string(
        state,
        "aegis_avatar_auto_refresh_provider",
        "xai_subscription",
    );
    if provider != "xai_subscription" {
        return Err((
            StatusCode::CONFLICT,
            "Aegis avatar generation currently supports xai_subscription only; no paid fallback was used."
                .into(),
        ));
    }
    let credential =
        load_ready_xai_subscription_credential_for_feature(state, "generating Aegis avatar")
            .await?;
    let api_base =
        vnext_avatar_setting_string(state, "xai_subscription_api_base", "https://api.x.ai/v1");
    let backend = crate::llm::XaiSubscriptionBackend::with_credential_and_api_base(
        "grok-imagine-image-2.0",
        credential,
        api_base,
    );
    let prompt = build_aegis_avatar_prompt(&prompt_hint);
    let generated = backend
        .generate_image_data_url(&prompt, "1:1")
        .await
        .map_err(|err| (StatusCode::SERVICE_UNAVAILABLE, err.message))?;
    let avatar_url = store_generated_aegis_avatar_image_for_owner(
        state,
        &owner,
        &generated.data_url,
        chrono::Utc::now(),
    )?;

    let mut profile = load_vnext_avatar_profile_for_owner(state, owner.clone()).await?;
    profile.aegis_avatar_url = avatar_url;
    profile.updated_at_unix = Some(chrono::Utc::now().timestamp());
    let mut saved = save_vnext_avatar_profile_for_owner(state, owner, profile).await?;
    if automatic {
        persist_runtime_setting(
            state,
            "aegis_avatar_auto_refresh_last_at_unix",
            serde_json::json!(chrono::Utc::now().timestamp()),
        )
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to record avatar refresh timestamp: {err}"),
            )
        })?;
        saved.auto_refresh = vnext_avatar_auto_refresh_projection(state).await;
    }

    Ok(VNextAegisAvatarGenerateResponse {
        schema_version: 1,
        status: "updated".into(),
        owner_message: if automatic {
            "Aegis refreshed her weekly avatar through the connected Grok image capability.".into()
        } else {
            "Aegis avatar was generated and saved to the server profile.".into()
        },
        provider,
        provider_request_id: generated.provider_request_id,
        profile: saved,
    })
}

pub fn spawn_vnext_avatar_auto_refresh_loop(state: Arc<AdminState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(err) = maybe_refresh_due_aegis_avatars(&state).await {
                tracing::warn!(error = %err, "Aegis avatar auto-refresh check failed");
            }
        }
    });
}

async fn maybe_refresh_due_aegis_avatars(state: &AdminState) -> anyhow::Result<usize> {
    if !vnext_avatar_setting_bool(state, "aegis_avatar_auto_refresh_enabled", false) {
        return Ok(0);
    }
    let interval_days =
        vnext_avatar_setting_u32(state, "aegis_avatar_auto_refresh_interval_days", 7).clamp(1, 90);
    let last_raw = vnext_avatar_setting_i64(state, "aegis_avatar_auto_refresh_last_at_unix", 0);
    let last_refresh_at = (last_raw > 0).then_some(last_raw);
    if !vnext_avatar_auto_refresh_due(
        chrono::Utc::now().timestamp(),
        last_refresh_at,
        interval_days,
    ) {
        return Ok(0);
    }

    let db_path = webapp_debug_snapshot_db_path(state);
    let owners = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<String>> {
        let conn = crate::encrypted_sqlite::open(db_path)?;
        Ok(list_vnext_avatar_profile_owners(&conn)?)
    })
    .await
    .context("avatar owner list task failed")??;
    if owners.is_empty() {
        tracing::info!("Aegis avatar auto-refresh is due but no owner avatar profile exists yet");
        return Ok(0);
    }
    let prompt_hint =
        vnext_avatar_setting_string(state, "aegis_avatar_auto_refresh_prompt_hint", "");
    let mut refreshed = 0usize;
    for owner in owners {
        match generate_and_store_aegis_avatar(state, owner, prompt_hint.clone(), true).await {
            Ok(_) => refreshed += 1,
            Err((status, message)) => {
                tracing::warn!(%status, error = %message, "Aegis avatar auto-refresh failed for owner");
            }
        }
    }
    Ok(refreshed)
}
