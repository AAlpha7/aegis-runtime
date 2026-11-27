//! Human-In-The-Loop (HITL) approval and rejection handlers.
//!
//! These methods handle Telegram callback button presses for approve/reject
//! workflows across sysops, config changes, MCP, menus, and sysadmin operations.
//!
//! Implemented as `impl Orchestrator` methods in a separate file for code organization.
//! The Orchestrator struct is defined in `orchestrator.rs`; Rust allows multiple
//! `impl` blocks across files within the same crate.

use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::embedding::EmbeddingClient;
use crate::memory::MemoryHandle;
use crate::menu::CustomCommand;
use crate::orchestrator::{Orchestrator, OrchestratorResult};

// ── HITL approval queue helpers (design §6, Step 9) ──────────────────────
//
// The HITL queue lives in `skill_state` under `skill_name='hitl'`. Step 9
// extended that table with `kind`, `proposed_at_audit_cycle_id`,
// `embedding_json`, and `expires_at` so heartbeat-originated proposals can
// be aged out and deduped against pending peers without forking a second
// queue. These helpers are the thin public surface; actual SQL lives in
// `memory.rs` MemoryCommand handlers.

/// Default per-kind approval TTL in seconds (design §15.1).
///
/// - `config_change` → 24h (the "proposal" kind family lifetime)
/// - anything else (`generic`, unknown) → 1h (existing HITL default,
///   matches the pre-Step-9 fail-closed TTL check in
///   `load_verify_clear_hitl`)
pub fn kind_default_ttl_seconds(kind: &str) -> u64 {
    match kind {
        "config_change" => 24 * 60 * 60,
        _ => 60 * 60,
    }
}

/// Structured outcome of [`Orchestrator::apply_approved_config`]./// Structured outcome of [`Orchestrator::apply_approved_config`].
///
/// `apply_approved_config` feeds TWO channels with different localization
/// contracts, so it must NOT return a single pre-formatted string:
///
/// - **Channel A** (`handle_config_approval`) embeds the result in a
///   `[SYSTEM CALLBACK]` message handed to the LLM. That text is LLM-bound and
///   must stay in the raw English form (the LLM then re-renders it in the
///   user's language). Use [`ConfigApplyResult::callback_string`].
/// - **Channel B** (`create_config_proposal_ex` gate-pre-approved path) renders
///   a DIRECT owner confirmation to Telegram with no LLM in the loop. That text
///   must be localized to the owner's `display_language`. Use
///   [`ConfigApplyResult::localized`].
///
/// All embedded values (setting key, old/new JSON, embedding from/to/reason)
/// are DATA-DERIVED / ARBITRARY, so `localized` interpolates via `tfp`
/// (single-pass, no re-scan) — a value containing a literal `{}` / `{1}` can
/// never clobber a later token.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConfigApplyResult {
    /// heartbeat_config applied and the live config changed. {key, value}.
    UpdatedTo { key: String, value: String },
    /// heartbeat_config payload matched current — no change. {key}.
    Unchanged { key: String },
    /// Apply failed (heartbeat apply error OR generic settings.set error).
    /// `error` is an arbitrary error string concatenated AFTER the prefix.
    ApplyFailed { key: String, error: String },
    /// heartbeat_config payload could not be decoded. {key}.
    InvalidPayload { key: String },
    /// Generic settings key applied. {key, old, new} + optional hot-reload note.
    ChangedFromTo {
        key: String,
        old: String,
        new: String,
        note: HotReloadNote,
    },
}

/// Hot-reload sub-outcome appended after a [`ConfigApplyResult::ChangedFromTo`]
/// success line (only embedding_* keys produce a non-`None` note).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HotReloadNote {
    None,
    HotReloaded {
        from: String,
        to: String,
    },
    RequiresReembed {
        old: String,
        new: String,
        key: String,
    },
    Rejected {
        reason: String,
    },
}

impl HotReloadNote {
    /// Raw English form for the Channel-A LLM callback (mirrors the historical
    /// hardcoded strings exactly).
    fn callback_string(&self) -> String {
        match self {
            HotReloadNote::None => String::new(),
            HotReloadNote::HotReloaded { from, to } => {
                format!(" Hot-reloaded ({from} → {to}).")
            }
            HotReloadNote::RequiresReembed { old, new, key } => format!(
                " Hot-reloaded ({old} → {new}). \u{26A0}\u{FE0F}  The stored embedding \
                 corpus was produced by the old {key} value; cosine similarity against \
                 the new one is not meaningful. Please call `trigger_corpus_reembed` \
                 (user-approved) to recompute all vectors with the new setting."
            ),
            HotReloadNote::Rejected { reason } => format!(
                " \u{26A0}\u{FE0F}  Hot-reload REJECTED: {reason} (setting row stored, but \
                 live client still uses previous value — a restart will pick up the \
                 setting, or propose a valid value)"
            ),
        }
    }

    /// Localized (Channel B) form — all args ARBITRARY → `tfp` (no re-scan).
    fn localized(&self, locale: &str) -> String {
        match self {
            HotReloadNote::None => String::new(),
            HotReloadNote::HotReloaded { from, to } => {
                crate::i18n::tfp("config.exec.hot_reloaded", locale, &[from, to])
            }
            HotReloadNote::RequiresReembed { old, new, key } => {
                crate::i18n::tfp("config.exec.requires_reembed", locale, &[old, new, key])
            }
            HotReloadNote::Rejected { reason } => {
                crate::i18n::tfp("config.exec.hot_reload_rejected", locale, &[reason])
            }
        }
    }
}

impl ConfigApplyResult {
    /// Channel-A form: the raw English one-liner embedded in the LLM
    /// `[SYSTEM CALLBACK]` message. MUST remain byte-identical to the strings
    /// `apply_approved_config` historically returned (the LLM is prompted on
    /// this exact phrasing).
    pub(crate) fn callback_string(&self) -> String {
        match self {
            ConfigApplyResult::UpdatedTo { key, value } => {
                format!("Setting '{key}' updated to {value}.")
            }
            ConfigApplyResult::Unchanged { key } => {
                format!("Setting '{key}' unchanged (payload matched current).")
            }
            ConfigApplyResult::ApplyFailed { key, error } => {
                format!("Failed to apply setting '{key}': {error}")
            }
            ConfigApplyResult::InvalidPayload { key } => {
                format!("Failed to apply setting '{key}': invalid JSON payload")
            }
            ConfigApplyResult::ChangedFromTo {
                key,
                old,
                new,
                note,
            } => format!(
                "Setting '{key}' changed from {old} to {new}.{}",
                note.callback_string()
            ),
        }
    }

    /// Channel-B form: localized to the owner's `display_language`. Arbitrary
    /// args (key/values/error) go through `tfp` (single-pass) or prefix-only +
    /// `format!` concat — never `tf`/`tfn` (which would re-scan a literal `{}`
    /// in an error string and corrupt it).
    pub(crate) fn localized(&self, locale: &str) -> String {
        match self {
            ConfigApplyResult::UpdatedTo { key, value } => {
                crate::i18n::tfp("config.exec.updated_to", locale, &[key, value])
            }
            ConfigApplyResult::Unchanged { key } => {
                crate::i18n::tfp("config.exec.unchanged", locale, &[key])
            }
            ConfigApplyResult::ApplyFailed { key, error } => {
                // PREFIX-only: localized template ends after the key; the
                // arbitrary error is concatenated AFTER (never re-scanned).
                format!(
                    "{}{error}",
                    crate::i18n::tfp("config.exec.apply_failed", locale, &[key])
                )
            }
            ConfigApplyResult::InvalidPayload { key } => {
                crate::i18n::tfp("config.exec.invalid_payload", locale, &[key])
            }
            ConfigApplyResult::ChangedFromTo {
                key,
                old,
                new,
                note,
            } => format!(
                "{}{}",
                crate::i18n::tfp("config.exec.changed_from_to", locale, &[key, old, new]),
                note.localized(locale)
            ),
        }
    }
}

/// Delete expired HITL rows (rows whose `expires_at < now`). Rows bearing
/// the non-expiring sentinel are skipped. Returns number of rows deleted.
/// Idempotent: a second call with the same `now` returns 0.
pub async fn sweep_expired(memory: &MemoryHandle, now: DateTime<Utc>) -> Result<u64> {
    memory.sweep_expired_hitl(now).await
}

/// Find a similar pending HITL proposal of the same `kind` for dedup
/// (design §6, §15.1 cosine threshold 0.85). Returns the matching row's
/// `state_key` if any candidate scores >= `cosine_threshold` when compared
/// with `embedding`. `since` bounds the lookback window (default: 24h).
///
/// Cosine computation uses `EmbeddingClient::cosine_similarity`, reused
/// from the existing vector infrastructure.
pub async fn find_similar_pending(
    memory: &MemoryHandle,
    kind: &str,
    embedding: &[f32],
    cosine_threshold: f32,
    since: DateTime<Utc>,
) -> Result<Option<String>> {
    let candidates = memory
        .list_pending_hitl_with_embeddings(kind, since)
        .await?;
    for (key, emb_json) in candidates {
        let Ok(existing): std::result::Result<Vec<f32>, _> = serde_json::from_str(&emb_json) else {
            tracing::warn!(key = %key, "[HITL] pending row has malformed embedding_json — skipping");
            continue;
        };
        if existing.len() != embedding.len() {
            continue;
        }
        let sim = EmbeddingClient::cosine_similarity(embedding, &existing);
        if sim >= cosine_threshold {
            return Ok(Some(key));
        }
    }
    Ok(None)
}

/// Clear HITL approval state after completing an approval flow.
/// Logs a warning if the cleanup fails (non-fatal: stale state is overwritten
/// by the next approval cycle, but logging aids debugging).
async fn clear_approval_state(memory: &MemoryHandle, approval_id: &str) {
    if let Err(e) = memory.store_skill_state("hitl", approval_id, "").await {
        tracing::warn!(error = %e, approval_id, "Failed to clear HITL approval state");
    }
}

fn hitl_allowed_chat_ids(approval: &Value) -> Vec<i64> {
    let mut ids = Vec::new();
    for key in ["chat_id", "delivery_chat_id"] {
        if let Some(id) = approval.get(key).and_then(|v| v.as_i64())
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    if let Some(arr) = approval.get("allowed_chat_ids").and_then(|v| v.as_array()) {
        for id in arr.iter().filter_map(|v| v.as_i64()) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

impl Orchestrator {
    async fn load_hitl_state(&self, key: &str) -> Option<String> {
        match self.memory.load_skill_state("hitl", key).await {
            Ok(Some(data)) if !data.is_empty() => Some(data),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, key, "Failed to load HITL state");
                None
            }
        }
    }

    // ── Shared HITL helpers ─────────────────────────────────────────────

    /// Load, parse, verify, and clear an HITL approval in one step.
    ///
    /// Returns `Ok(Some((proposal_json, user_did)))` on success.
    /// Returns `Ok(None)` if the approval is not found, already processed,
    /// has corrupted JSON (logged as warning), or fails chat_id verification.
    ///
    /// On `Some`, the approval state has already been cleared (preventing
    /// double-click re-approval).
    async fn load_verify_clear_hitl(
        &self,
        hitl_key: &str,
        chat_id: Option<i64>,
        context: &str,
    ) -> Result<Option<(Value, String)>> {
        let Some(state_data) = self.load_hitl_state(hitl_key).await else {
            return Ok(None);
        };

        let proposal: Value = match serde_json::from_str(&state_data) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e, hitl_key, context,
                    data_len = state_data.len(),
                    "[HITL] Corrupted approval JSON — treating as not-found"
                );
                return Ok(None);
            }
        };

        // TTL check: reject stale approvals older than 1 hour (fail-closed)
        match proposal
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(chrono::DateTime::parse_from_rfc3339)
        {
            Some(Ok(created)) => {
                let age = chrono::Utc::now().signed_duration_since(created);
                if age.num_seconds() > 3600 {
                    tracing::warn!(
                        hitl_key,
                        age_secs = age.num_seconds(),
                        "[HITL] Approval expired (>1h) — clearing stale state"
                    );
                    clear_approval_state(&self.memory, hitl_key).await;
                    return Ok(None);
                }
            }
            _ => {
                // Missing or unparseable created_at — fail-closed for security
                tracing::warn!(
                    hitl_key,
                    "[HITL] Approval missing/invalid created_at — rejecting (fail-closed)"
                );
                clear_approval_state(&self.memory, hitl_key).await;
                return Ok(None);
            }
        }

        if let Some(_denied) = Self::verify_hitl_chat_id(
            &proposal,
            chat_id,
            hitl_key,
            &self.settings.display_language(),
        ) {
            // Chat ID mismatch — return None so caller produces its own error.
            // The verify function already logged the mismatch.
            //
            // HITL-1 fix: clear the approval state before returning. Otherwise
            // a mismatched-chat probe would leave the pending approval intact,
            // allowing replay from the correct chat and enabling double-execute
            // (attacker-controlled side chat triggers a load, the original
            // chat later approves the still-pending prompt). Matches the
            // TTL-expired path's fail-closed discipline above.
            clear_approval_state(&self.memory, hitl_key).await;
            return Ok(None);
        }

        let user_did = proposal
            .get("user_did")
            .and_then(|v| v.as_str())
            .unwrap_or("did:aegis:system")
            .to_string();

        clear_approval_state(&self.memory, hitl_key).await;

        Ok(Some((proposal, user_did)))
    }

    /// Resume the ReAct loop with a HITL callback message.
    /// Wraps `Box::pin(self.process_message(...))` — the standard HITL postamble.
    async fn resume_with_hitl_callback(
        &self,
        user_did: &str,
        callback_msg: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        Box::pin(self.process_message(user_did, callback_msg, chat_id)).await
    }

    pub(crate) fn verify_hitl_chat_id(
        approval: &Value,
        caller_chat_id: Option<i64>,
        approval_id: &str,
        // Channel B — owner-facing denial message renders in display_language.
        // This is a static fn (no &self), so callers thread the resolved locale.
        // SECURITY: locale affects DISPLAY ONLY; the chat-id match logic below is
        // unchanged.
        locale: &str,
    ) -> Option<OrchestratorResult> {
        let stored_chat_id = approval.get("chat_id").and_then(|v| v.as_i64());
        let allowed_chat_ids = hitl_allowed_chat_ids(approval);
        let caller_cid = caller_chat_id;

        match (stored_chat_id, caller_cid) {
            // Caller matches the original chat or a delivery chat explicitly
            // bound when the approval request was routed to a topic/group.
            (_, Some(caller)) if allowed_chat_ids.contains(&caller) => None,
            // Bound approval, but caller is not one of the allowed surfaces.
            (Some(stored), Some(caller)) => {
                tracing::warn!(
                    approval_id,
                    stored_chat_id = stored,
                    allowed_chat_ids = ?allowed_chat_ids,
                    caller_chat_id = caller,
                    "[HITL] Approval rejected — chat_id mismatch"
                );
                Some(OrchestratorResult {
                    reply: crate::i18n::tf("hitl.chat_mismatch", locale, approval_id),
                    ..Default::default()
                })
            }
            // Stored chat_id missing: the request was delivered by web push
            // before any Telegram binding happened. Every caller here has
            // already passed owner auth (console session or admin DM), so an
            // unbound approval is approvable — rejecting it left proposals to
            // expire unanswered (review 2026-09-13).
            (None, Some(caller)) if caller == crate::telegram::admin_dm_chat(0).0 => {
                tracing::info!(
                    approval_id,
                    caller_chat_id = caller,
                    "[HITL] Unbound approval accepted from the owner's chat"
                );
                None
            }
            // Any other chat: still rejected (cross-chat hijack guard).
            (None, Some(caller)) => {
                tracing::warn!(
                    approval_id,
                    caller_chat_id = caller,
                    "[HITL] Approval rejected — no stored chat_id and caller is not the owner chat"
                );
                Some(OrchestratorResult {
                    reply: crate::i18n::tf("hitl.chat_mismatch", locale, approval_id),
                    ..Default::default()
                })
            }
            // Caller has no chat_id (system/MCP internal) — allow
            (_, None) => None,
        }
    }

    /// Handle "approve sysop <id>" — sign commands with Ed25519, execute via
    /// Aegis Executor, then resume the ReAct loop so the LLM can summarize
    /// execution results in the context of the original conversation.
    pub(crate) async fn handle_sysop_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — sysop approval progress shown to the owner renders in
        // display_language (spec §2/§6). Read fresh from settings.
        let locale = self.settings.display_language();
        let Some((approval, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "SysopApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Approval `{}` not found or already processed.",
                approval_id
            )));
        };

        let commands: Vec<String> = approval
            .get("commands")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let plan = approval
            .get("plan")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");

        if commands.is_empty() {
            return Ok(OrchestratorResult::reply(format!(
                "Approval `{}` has no commands to execute.",
                approval_id
            )));
        }

        // Sign the commands with our Ed25519 identity
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let signed_message = format!("{}:{}", timestamp, commands.join("\n"));
        let signature = self.identity.sign_raw(signed_message.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        // Call the Aegis Executor /api/v1/execute endpoint (async job submission)
        let executor_url = std::env::var("AEGIS_EXECUTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9876".to_string());

        // Submit the job
        let submit_result = self
            .http_client
            .post(format!("{}/api/v1/execute", executor_url))
            .timeout(Duration::from_secs(30))
            .json(&serde_json::json!({
                "commands": commands,
                "signature": sig_hex,
                "timestamp": timestamp,
                "timeout_secs": 600,
            }))
            .send()
            .await;

        let job_id = match submit_result {
            Ok(resp) => {
                let body: Value = resp.json().await.unwrap_or_default();
                match body.get("job_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id.to_string(),
                    _ => {
                        let err = body
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let exec_summary = format!("Executor rejected the job: {}", err);
                        let callback_msg = format!(
                            "[SYSTEM CALLBACK — SysAdmin HITL Result]\n\
                             The administrator APPROVED sysadmin operation `{}`.\n\
                             Plan: {}\n\
                             Execution results:\n{}\n\
                             Summarize the outcome for the user. If any command failed, explain what happened.",
                            approval_id, plan, exec_summary
                        );
                        return self
                            .resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
                            .await;
                    }
                }
            }
            Err(e) => {
                let exec_summary = format!("Failed to reach Aegis Executor: {}", e);
                let callback_msg = format!(
                    "[SYSTEM CALLBACK — SysAdmin HITL Result]\n\
                     The administrator APPROVED sysadmin operation `{}`.\n\
                     Plan: {}\n\
                     Execution results:\n{}\n\
                     Summarize the outcome for the user. If any command failed, explain what happened.",
                    approval_id, plan, exec_summary
                );
                return self
                    .resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
                    .await;
            }
        };

        tracing::info!(approval_id, job_id = %job_id, "[HITL] Job submitted to executor, polling for results");

        // Send initial progress to Telegram
        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
            let _ = tx
                .send(crate::progress_router::ProgressEvent::new(
                    cid,
                    crate::progress_router::ProgressPayload::OwnerNotice {
                        text: crate::i18n::tf(
                            "hitl.sysop.job_submitted",
                            &locale,
                            &job_id[..8.min(job_id.len())],
                        ),
                        kind: crate::progress_router::OwnerNoticeKind::System,
                    },
                ))
                .await;
        }

        // Poll for job completion with progress updates
        let poll_url = format!("{}/api/v1/jobs/{}", executor_url, job_id);
        /// Maximum wall-clock time to poll a sysop executor job (15 minutes).
        const SYSOP_POLL_MAX_SECS: u64 = 900;
        /// Interval between consecutive poll requests to the executor.
        const SYSOP_POLL_INTERVAL_SECS: u64 = 5;
        /// Interval between Telegram progress updates to the user.
        const SYSOP_PROGRESS_INTERVAL_SECS: u64 = 30;
        /// Abort after this many consecutive poll failures (= 30s of failures).
        const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 6;

        let max_poll_secs = SYSOP_POLL_MAX_SECS;
        let poll_interval = Duration::from_secs(SYSOP_POLL_INTERVAL_SECS);
        let progress_interval = SYSOP_PROGRESS_INTERVAL_SECS;
        let poll_start = std::time::Instant::now();
        let mut last_progress_at = std::time::Instant::now();
        let mut last_completed_count = 0usize;
        let mut consecutive_poll_failures = 0u32;

        let exec_summary = loop {
            tokio::time::sleep(poll_interval).await;

            if poll_start.elapsed().as_secs() > max_poll_secs {
                break "Job polling timed out after 15 minutes. The job may still be running on the executor. Check with system administrator.".to_string();
            }

            let poll_resp = match self
                .http_client
                .get(&poll_url)
                .timeout(Duration::from_secs(15))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    consecutive_poll_failures += 1;
                    tracing::warn!(job_id = %job_id, error = %e, consecutive_failures = consecutive_poll_failures, "Poll request failed, retrying");
                    if consecutive_poll_failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                        // Executor likely crashed (e.g. OOM killed) — notify user instead of silent retry
                        if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
                            let _ = tx
                                .send(crate::progress_router::ProgressEvent::new(
                                    cid,
                                    crate::progress_router::ProgressPayload::OwnerNotice {
                                        text: crate::i18n::tf(
                                            "hitl.sysop.executor_unreachable",
                                            &locale,
                                            &consecutive_poll_failures.to_string(),
                                        ),
                                        kind: crate::progress_router::OwnerNoticeKind::System,
                                    },
                                ))
                                .await;
                        }
                        break format!(
                            "Executor became unreachable after {} consecutive poll failures (likely OOM-killed or crashed). \
                             Job {} was lost. The commands may have been partially executed.",
                            consecutive_poll_failures, job_id
                        );
                    }
                    continue;
                }
            };

            // Reset failure counter on successful connection
            consecutive_poll_failures = 0;

            let body: Value = match poll_resp.json().await {
                Ok(v) => v,
                Err(_) => continue,
            };

            let status = body
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let completed = body
                .get("commands_completed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let total = body
                .get("commands_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let elapsed = body
                .get("elapsed_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            // Send progress update to Telegram periodically or when new commands complete
            if completed > last_completed_count
                || last_progress_at.elapsed().as_secs() >= progress_interval
            {
                if let (Some(tx), Some(cid)) = (&self.progress_tx, chat_id) {
                    let current_cmd = body
                        .get("current_command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let msg = if status == "running" {
                        if !current_cmd.is_empty() {
                            // Concat the raw command AFTER the localized prefix — never
                            // route an arbitrary command through tfn's bare-{} re-scan
                            // (a cmd like `find . -exec rm {} \;` would be corrupted).
                            format!(
                                "{}{}",
                                crate::i18n::tfn(
                                    "hitl.sysop.progress_with_cmd",
                                    &locale,
                                    &[
                                        &completed.to_string(),
                                        &total.to_string(),
                                        &elapsed.to_string(),
                                    ],
                                ),
                                current_cmd
                            )
                        } else {
                            crate::i18n::tfn(
                                "hitl.sysop.progress",
                                &locale,
                                &[
                                    &completed.to_string(),
                                    &total.to_string(),
                                    &elapsed.to_string(),
                                ],
                            )
                        }
                    } else {
                        crate::i18n::tfn(
                            "hitl.sysop.done",
                            &locale,
                            &[status, &elapsed.to_string()],
                        )
                    };
                    let _ = tx
                        .send(crate::progress_router::ProgressEvent::new(
                            cid,
                            crate::progress_router::ProgressPayload::OwnerNotice {
                                text: msg,
                                kind: crate::progress_router::OwnerNoticeKind::System,
                            },
                        ))
                        .await;
                }
                last_completed_count = completed;
                last_progress_at = std::time::Instant::now();
            }

            if status != "running" {
                // Job finished — format results
                break Self::format_job_results(&body);
            }
        };

        // Approval state already cleared by load_verify_clear_hitl

        tracing::info!(
            approval_id,
            "[HITL] SysAdmin operation approved — resuming ReAct via callback"
        );

        let callback_msg = format!(
            "[SYSTEM CALLBACK — SysAdmin HITL Result]\n\
             The administrator APPROVED sysadmin operation `{}`.\n\
             Plan: {}\n\
             Execution results:\n{}\n\
             Summarize the outcome for the user. If any command failed, explain what happened.",
            approval_id, plan, exec_summary
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    /// Format job results from executor's /api/v1/jobs/:id response into
    /// a human-readable summary for LLM context injection.
    pub(crate) fn format_job_results(body: &Value) -> String {
        let status = body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let elapsed = body
            .get("elapsed_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let mut out = match status {
            "completed" => format!("All commands executed successfully ({}s).\n", elapsed),
            "failed" => format!("Command batch failed ({}s).\n", elapsed),
            "timed_out" => format!("Command timed out ({}s).\n", elapsed),
            "cancelled" => format!("Job was cancelled ({}s).\n", elapsed),
            other => format!("Job ended with status '{}' ({}s).\n", other, elapsed),
        };

        if let Some(results) = body.get("results").and_then(|r| r.as_array()) {
            for r in results {
                let cmd = r.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                let exit = r.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
                let stdout = r.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
                let stderr = r.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!("\n$ {}\n[exit {}]\n", cmd, exit));
                if !stdout.is_empty() {
                    let preview = &stdout[..stdout.len().min(2000)];
                    out.push_str(preview);
                    if stdout.len() > 2000 {
                        out.push_str("\n...(truncated)");
                    }
                    out.push('\n');
                }
                if !stderr.is_empty() {
                    out.push_str(&format!("[stderr] {}\n", &stderr[..stderr.len().min(1000)]));
                }
            }
        }

        out
    }

    /// Handle "reject sysop <id>" — discard the pending operation and resume
    /// the ReAct loop so the LLM can re-plan or inform the user.
    pub(crate) async fn handle_sysop_rejection(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "SysopReject")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Approval `{}` not found (may have already been processed).",
                approval_id
            )));
        };

        let plan = proposal
            .get("plan")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");

        tracing::info!(
            approval_id,
            "[HITL] SysAdmin operation rejected — resuming ReAct for re-planning"
        );

        let callback_msg = format!(
            "[SYSTEM CALLBACK — SysAdmin HITL Rejection]\n\
             The administrator REJECTED sysadmin operation `{}`.\n\
             Rejected plan: {}\n\
             Inform the user that the operation was rejected. \
             If you believe there's an alternative approach, propose it. \
             Otherwise, ask the user for guidance on how to proceed.",
            approval_id, plan
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    // ── Gate HITL: unified tool approval (granular scoping) ──────────────

    /// Count one real dispatch of this approval. Refresh must not call this.
    async fn mark_gate_dispatched(&self, approval_id: &str) {
        let previous = self
            .memory
            .load_skill_state(crate::egress_gate::HITL_DISPATCH_SKILL, approval_id)
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if let Err(error) = self
            .memory
            .store_skill_state(
                crate::egress_gate::HITL_DISPATCH_SKILL,
                approval_id,
                &previous.saturating_add(1).to_string(),
            )
            .await
        {
            tracing::warn!(error = %error, approval_id, "failed to record gate dispatch");
        }
    }

    /// Handle gate approval with a specific scope.
    /// Replaces the former `handle_gate_approval` with scope-aware logic.
    pub(crate) async fn handle_gate_approval_scoped(
        &self,
        approval_id: &str,
        scope: crate::tool_gate::ApprovalScope,
        user_did: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some(state_data) = self.load_hitl_state(approval_id).await else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.gate.not_found_or_expired",
                &locale,
                approval_id,
            )));
        };

        let approval: Value = serde_json::from_str(&state_data)?;

        // Verify the approver is from the same chat that created the request.
        //
        // HITL-1 paired-fix: clear state on chat-id mismatch so a probe from
        // the wrong chat cannot leave the approval pending for the original
        // chat to later approve (double-execute surface). Matches the
        // fail-closed discipline of `load_verify_clear_hitl`. This handler
        // doesn't go through `load_verify_clear_hitl` because it needs the
        // approval data in hand across scope-resolution and post-execute
        // clear (see HITL-2 ordering), so the clear is done inline.
        if let Some(denied) = Self::verify_hitl_chat_id(&approval, chat_id, approval_id, &locale) {
            clear_approval_state(&self.memory, approval_id).await;
            return Ok(denied);
        }

        // HITL-4 fix: reject approvals with missing/empty critical fields
        // instead of silently defaulting. A corrupted approval record
        // (missing tool_name or tool_args) previously executed an empty tool
        // name — either a silent noop or a downstream panic, and the stored
        // tool_args could disagree with what the user originally approved.
        let tool_name_opt = approval.get("tool_name").and_then(|v| v.as_str());
        let tool_args_opt = approval.get("tool_args").and_then(|v| v.as_str());
        let tool_name = match tool_name_opt {
            Some(s) if !s.is_empty() => s,
            _ => {
                tracing::error!(
                    approval_id,
                    field = "tool_name",
                    "[ToolGate] Approval rejected — missing/empty critical field"
                );
                clear_approval_state(&self.memory, approval_id).await;
                return Ok(OrchestratorResult::reply(crate::i18n::t(
                    "hitl.approval_corrupted",
                    &locale,
                )));
            }
        };
        let tool_args = match tool_args_opt {
            Some(s) if !s.is_empty() => s,
            _ => {
                tracing::error!(
                    approval_id,
                    tool_name,
                    field = "tool_args",
                    "[ToolGate] Approval rejected — missing/empty critical field"
                );
                clear_approval_state(&self.memory, approval_id).await;
                return Ok(OrchestratorResult::reply(crate::i18n::t(
                    "hitl.approval_corrupted",
                    &locale,
                )));
            }
        };
        let stored_did = approval
            .get("user_did")
            .and_then(|v| v.as_str())
            .unwrap_or(user_did);

        // Resolve the actual scope — Intent/Domain need context from the approval data.
        // HITL-3: track whether the requested scope was silently downgraded to Once
        // so we can surface this to the user (previously, downgrade was warn-log-only,
        // giving the false impression that a persistent approval was set).
        let mut fallback_notice: Option<&'static str> = None;
        let resolved_scope = match scope {
            crate::tool_gate::ApprovalScope::Intent { group_key } if group_key.is_empty() => {
                // Pull group_key from stored approval data
                let gk = approval
                    .get("intent_group_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if gk.is_empty() {
                    // No intent context — fall back to Once
                    tracing::warn!(
                        approval_id,
                        "[ToolGate] Intent scope requested but no group_key in approval data — falling back to Once"
                    );
                    fallback_notice = Some("hitl.scope_fallback_intent_to_once");
                    crate::tool_gate::ApprovalScope::Once
                } else {
                    crate::tool_gate::ApprovalScope::Intent { group_key: gk }
                }
            }
            crate::tool_gate::ApprovalScope::Domain { host } if host.is_empty() => {
                // Pull host from stored approval data
                let h = approval
                    .get("url_host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if h.is_empty() {
                    // No domain context — fall back to Once
                    tracing::warn!(
                        approval_id,
                        "[ToolGate] Domain scope requested but no url_host in approval data — falling back to Once"
                    );
                    fallback_notice = Some("hitl.scope_fallback_domain_to_once");
                    crate::tool_gate::ApprovalScope::Once
                } else {
                    crate::tool_gate::ApprovalScope::Domain { host: h }
                }
            }
            other => other,
        };

        // Store the approval (except Once — that's ephemeral).
        //
        // HITL-2: we deliberately store BEFORE execute because the stored
        // approval captures the user's *intent* for future calls matching
        // this scope (Intent/Domain/Always). Even if execution fails this
        // round, the user's approval of the class of action stands — a
        // retry triggered later by the LLM will match the stored record
        // without re-prompting. Ephemeral Once scope needs no storage.
        match &resolved_scope {
            crate::tool_gate::ApprovalScope::Once => { /* no storage needed */ }
            other => {
                let record = crate::tool_gate::StoredApproval {
                    tool_name: tool_name.to_string(),
                    scope: other.clone(),
                    approved_at: chrono::Utc::now().to_rfc3339(),
                    approved_by: user_did.to_string(),
                };
                self.store_tool_approval(&record).await;
            }
        }

        // Log the approval scope
        tracing::info!(
            approval_id,
            tool_name,
            ?resolved_scope,
            "[ToolGate] Scoped approval granted"
        );

        // Capture source-task coordinates now (we'll need them after execute).
        let source_task = {
            let tid = approval.get("source_task_id").and_then(|v| v.as_i64());
            let cid = approval.get("source_chat_id").and_then(|v| v.as_i64());
            match (tid, cid) {
                (Some(t), Some(c)) => Some((t, c)),
                _ => None,
            }
        };

        // Direct execution: call the tool handler with stored args, bypassing
        // the LLM entirely. Previous approach relied on the LLM to reconstruct
        // the tool call from a text description, which lost parameters.
        //
        // HITL-2 fix: the pending HITL state is cleared AFTER execute, not
        // before. The previous ordering wiped state at L790 then called
        // execute_approved_tool; if execution returned Err, the user saw the
        // error but the approval_id was already gone, so the LLM's retry via
        // the same approval_id fell through to "not found". Moving the clear
        // below execute keeps the "pending prompt" alive only while it's
        // meaningful — after execute (success or failure), the prompt is no
        // longer pending and must be cleaned up regardless. Retry UX is
        // driven by the LLM re-issuing the tool call, not by double-approval.
        // Jev non-CU Ask resume: run the original tool without re-calling Jev.
        if approval.get("kind").and_then(|v| v.as_str())
            == Some(crate::computer_use::JEV_NON_CU_ASK_KIND)
        {
            let pending: crate::computer_use::NonCuPendingAsk = match serde_json::from_value(
                approval.get("pending_ask").cloned().unwrap_or_default(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    clear_approval_state(&self.memory, approval_id).await;
                    return Ok(OrchestratorResult::reply(format!(
                        "Jev non-CU ask approval corrupted (pending_ask): {e}"
                    )));
                }
            };
            let marked = crate::computer_use::non_cu_gate::with_owner_approved_marker(pending.args);
            let marked_str = marked.to_string();
            self.mark_gate_dispatched(approval_id).await;
            let result = self
                .execute_approved_tool(&pending.tool, &marked_str, stored_did, chat_id)
                .await;
            clear_approval_state(&self.memory, approval_id).await;
            return match result {
                Ok(output) => {
                    let jev = crate::computer_use::JevProvenance::from_decision(
                        &pending.policy,
                        Some("approved"),
                        Some(approval_id),
                        None,
                        None,
                    );
                    Ok(OrchestratorResult::reply(
                        crate::computer_use::stamp_jev_on_json(&output, &jev),
                    ))
                }
                Err(e) => Ok(OrchestratorResult::reply(format!(
                    "Jev non-CU ask approved but execution failed: {e}"
                ))),
            };
        }

        // Secure CU Policy Ask resume: sign+device without re-calling Jev.
        if approval.get("kind").and_then(|v| v.as_str()) == Some("secure_cu_ask") {
            let pending: crate::computer_use::PendingAsk = match serde_json::from_value(
                approval.get("pending_ask").cloned().unwrap_or_default(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    clear_approval_state(&self.memory, approval_id).await;
                    return Ok(OrchestratorResult::reply(format!(
                        "Secure CU ask approval corrupted (pending_ask): {e}"
                    )));
                }
            };
            let pipeline = match self.secure_cu_pipeline() {
                Ok(p) => p,
                Err(e) => {
                    clear_approval_state(&self.memory, approval_id).await;
                    return Ok(OrchestratorResult::reply(format!(
                        "Secure CU pipeline unavailable: {e}"
                    )));
                }
            };
            self.mark_gate_dispatched(approval_id).await;
            let result = match pipeline.continue_after_owner_approval(pending) {
                Ok(out) => Ok(serde_json::json!({
                    "ok": out.receipt.accepted,
                    "status": "owner_approved_ask",
                    "adapter": pipeline.device_adapter(),
                    "device_id": pipeline.device_id(),
                    "policy": {
                        "verdict": out.policy.verdict,
                        "model": out.policy.model,
                        "reason": out.policy.reason,
                    },
                    "receipt": out.receipt,
                    "audit_tip": out.audit_tip,
                    "intent_id": out.intent.body.intent_id,
                })
                .to_string()),
                Err(e) => Err(anyhow::anyhow!("secure_cu_ask resume failed: {e}")),
            };
            clear_approval_state(&self.memory, approval_id).await;
            return match result {
                Ok(output) => Ok(OrchestratorResult::reply(output)),
                Err(e) => Ok(OrchestratorResult::reply(format!(
                    "Secure CU ask approved but execution failed: {e}"
                ))),
            };
        }

        self.mark_gate_dispatched(approval_id).await;
        // Approval replay bypasses the chat loop, so it needs its own scope.
        // Preserve notices in the human receipt, without changing tool JSON.
        let (result, behavior_notices) = crate::behavior_notice::collect(
            &locale,
            self.progress_tx.clone(),
            chat_id,
            Box::pin(self.execute_approved_tool(tool_name, tool_args, stored_did, chat_id)),
        )
        .await;

        // Clear the HITL state — after execute, in both Ok/Err paths.
        clear_approval_state(&self.memory, approval_id).await;

        match result {
            Ok(output) => {
                tracing::info!(
                    approval_id,
                    tool_name,
                    output_len = output.len(),
                    "[ToolGate] Direct execution succeeded"
                );

                // HITL-2: restore auto-suspended recurring task (blocked_by_gate
                // → active) only on Ok. On Err we want the task to stay blocked
                // so the scheduler doesn't re-fire immediately into the same
                // failure — user can clear the block explicitly once the root
                // cause is addressed.
                //
                // INTENTIONAL ASYMMETRY with the reject path: reject ALWAYS
                // restores the task (user explicitly declined; task should be
                // free to re-fire later). Approve-with-execute-Err LEAVES the
                // task blocked (execution attempted and failed; bouncing into
                // the scheduler would likely repeat the same failure — e.g.
                // transient API rate-limit retried instantly, exhausted
                // OAuth token retried before refresh). User investigates via
                // the error reply, then manually re-activates the task once
                // the root cause is fixed. Locked in by the test
                // `execute_failure_clears_state_and_preserves_stored_approval`
                // (exercises the Err path) + `hitl_reject_clears_entry_and_returns_reply`
                // (exercises the reject path). CTO round 2 §2 flagged this
                // asymmetry; after review the user confirmed the behavior is
                // intentional and closed the item as non-bug (Option C, 2026-04-18).
                if let Some((task_id, task_chat_id)) = source_task {
                    let restore_json = serde_json::json!({"status": "active"}).to_string();
                    match self
                        .memory
                        .update_task(task_id, task_chat_id, &restore_json)
                        .await
                    {
                        Ok(true) => {
                            tracing::info!(
                                task_id,
                                approval_id,
                                "[ToolGate] Restored recurring task from blocked_by_gate → active"
                            );
                        }
                        Ok(false) => {
                            tracing::warn!(
                                task_id,
                                approval_id,
                                "[ToolGate] Task not found or not in blocked_by_gate state"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                task_id,
                                approval_id,
                                error = %e,
                                "[ToolGate] Failed to restore task from gate suspension"
                            );
                        }
                    }
                }

                // Format raw tool output for human readability instead of
                // dumping JSON directly to Telegram. Channel B — owner
                // display_language (spec §2/§6); no LLM re-renders this.
                let locale = self.settings.display_language();
                let mut formatted = crate::behavior_notice::prepend(
                    &format_gate_result(tool_name, &output, &locale),
                    &behavior_notices,
                );
                // HITL-3: prepend fallback notice if scope was downgraded.
                // Channel B — owner display_language (spec §2/§6).
                if let Some(key) = fallback_notice {
                    let notice = crate::i18n::t(key, &locale);
                    formatted = format!("{notice}\n\n{formatted}");
                }
                Ok(OrchestratorResult::reply(formatted))
            }
            Err(e) => {
                tracing::error!(
                    approval_id,
                    tool_name,
                    error = %e,
                    "[ToolGate] Direct execution failed"
                );
                let locale = self.settings.display_language();
                // Concat the raw error AFTER the localized prefix — an error string
                // containing a literal `{}` must not pass through tfn's bare-{} re-scan.
                let mut reply = format!(
                    "{}{}",
                    crate::i18n::tfn("hitl.exec_failed", &locale, &[tool_name]),
                    e
                );
                // HITL-3: surface fallback notice on error path too — user
                // still needs to know the scope was downgraded. Channel B.
                if let Some(key) = fallback_notice {
                    let notice = crate::i18n::t(key, &locale);
                    reply = format!("{notice}\n\n{reply}");
                }
                Ok(OrchestratorResult::reply(
                    crate::behavior_notice::append_failure(&reply, &behavior_notices),
                ))
            }
        }
    }

    /// Handle "clarify" — ask the LLM to explain what it wants and why.
    /// Does NOT clear the pending approval — user can still approve/reject afterwards.
    pub(crate) async fn handle_gate_clarify(
        &self,
        approval_id: &str,
        user_did: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some(state_data) = self.load_hitl_state(approval_id).await else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.gate.not_found_or_expired",
                &locale,
                approval_id,
            )));
        };
        let approval: Value = serde_json::from_str(&state_data)?;

        // HITL-1 paired-fix (CTO round-2 §4 sibling): verify the clarifier
        // is the same chat that created the approval. Previously this handler
        // echoed tool_name + tool_args back to the caller without any chat-id
        // check — allowing information disclosure (attacker in a side chat
        // guesses an approval_id and learns what tool the legitimate user is
        // being asked to approve). Unlike the execute paths, we DO NOT clear
        // state on mismatch — the legitimate user in the correct chat should
        // still be able to approve/reject the pending request.
        if let Some(denied) = Self::verify_hitl_chat_id(&approval, chat_id, approval_id, &locale) {
            return Ok(denied);
        }

        // HITL-4 paired-fix: reject on missing critical fields. An empty
        // tool_name would produce a nonsensical clarification prompt
        // ("explain why you need to use '' with these arguments: {}").
        let tool_name = match approval.get("tool_name").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                tracing::error!(
                    approval_id,
                    field = "tool_name",
                    "[HITL] Gate clarify rejected — missing/empty critical field"
                );
                return Ok(OrchestratorResult::reply(crate::i18n::t(
                    "hitl.approval_corrupted",
                    &locale,
                )));
            }
        };
        let tool_args = match approval.get("tool_args").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                tracing::error!(
                    approval_id,
                    field = "tool_args",
                    "[HITL] Gate clarify rejected — missing/empty critical field"
                );
                return Ok(OrchestratorResult::reply(crate::i18n::t(
                    "hitl.approval_corrupted",
                    &locale,
                )));
            }
        };

        // DON'T clear the HITL state — user may still approve after clarification

        let callback_msg = format!(
            "[SYSTEM — Clarification Requested]\n\
             The user wants you to explain why you need to use '{tool_name}' with these arguments: {tool_args}\n\
             Explain clearly:\n\
             1. What you're trying to do and why\n\
             2. What data you'll access\n\
             3. What alternative approaches exist if the user prefers not to approve\n\
             The approval request (ID: {approval_id}) is still pending — the user can still approve or reject after your explanation."
        );
        Box::pin(self.process_message(user_did, &callback_msg, chat_id)).await
    }

    /// Apply an approved config change to live state.
    ///
    /// Shared between the standard [`handle_config_approval`] path (user taps
    /// Approve on the Config proposal buttons) and the Gate-pre-approved path
    /// (Task #12 — Gate approval auto-pre-approves the resulting Config
    /// proposal). Both paths land here so the actual write/hot-reload logic
    /// stays a single source of truth.
    ///
    /// Returns a structured [`ConfigApplyResult`] so each consumer can render
    /// it for its own channel: Channel A (LLM SYSTEM CALLBACK) via
    /// [`ConfigApplyResult::callback_string`] (raw English), Channel B (direct
    /// owner confirmation) via [`ConfigApplyResult::localized`]
    /// (owner `display_language`).
    pub(crate) async fn apply_approved_config(
        &self,
        approval_id: &str,
        key: &str,
        proposed_value: &Value,
    ) -> ConfigApplyResult {
        if crate::call_validation::requires_verified_settings_path(key, proposed_value) {
            let error = crate::call_validation::settings_path_message();
            self.best_effort_runtime_setting_action_failed(
                approval_id,
                "call_validation_required",
                error,
            );
            return ConfigApplyResult::ApplyFailed {
                key: key.into(),
                error: error.into(),
            };
        }
        self.best_effort_runtime_setting_action_transition(
            approval_id,
            crate::action_ledger::ActionStatus::Executing,
            crate::action_ledger::ActionUpdate::default(),
        );

        if key == "heartbeat_config" {
            match crate::heartbeat_config::HeartbeatConfig::from_json(proposed_value) {
                Some(new_cfg) => match self
                    .update_heartbeat_config(
                        new_cfg,
                        crate::heartbeat_config::ConfigAuditActor::User,
                        Some(format!("HITL approval {approval_id}")),
                    )
                    .await
                {
                    Ok(changed) => {
                        self.best_effort_runtime_setting_action_executed(
                            approval_id,
                            key,
                            proposed_value,
                            changed,
                        );
                        tracing::info!(
                            key = %key,
                            changed,
                            "[HITL] heartbeat_config change approved + audit-logged"
                        );
                        if changed {
                            ConfigApplyResult::UpdatedTo {
                                key: key.to_string(),
                                value: proposed_value.to_string(),
                            }
                        } else {
                            ConfigApplyResult::Unchanged {
                                key: key.to_string(),
                            }
                        }
                    }
                    Err(e) => {
                        self.best_effort_runtime_setting_action_failed(
                            approval_id,
                            "heartbeat_config_apply_failed",
                            &e.to_string(),
                        );
                        tracing::error!(key = %key, error = %e, "[HITL] heartbeat_config apply failed");
                        ConfigApplyResult::ApplyFailed {
                            key: key.to_string(),
                            error: e.to_string(),
                        }
                    }
                },
                None => {
                    self.best_effort_runtime_setting_action_failed(
                        approval_id,
                        "invalid_heartbeat_config_payload",
                        "invalid JSON payload",
                    );
                    tracing::error!(key = %key, "[HITL] heartbeat_config payload could not be decoded");
                    ConfigApplyResult::InvalidPayload {
                        key: key.to_string(),
                    }
                }
            }
        } else {
            match self
                .memory
                .commit_runtime_settings_with_embedding(
                    self.settings.clone(),
                    vec![(key.to_owned(), Some(proposed_value.clone()))],
                    Some(self.embedding_client_for_settings()),
                )
                .await
            {
                Ok(mut commit) => {
                    let old = commit.previous.remove(key).flatten();
                    let current = commit
                        .current
                        .remove(key)
                        .unwrap_or_else(|| proposed_value.clone());
                    tracing::info!(key = %key, old = ?old, new = ?proposed_value, "[HITL] Config change approved and applied");

                    let mut hot_reload_note = HotReloadNote::None;
                    if let Some(outcome) = commit.embedding.remove(key) {
                        hot_reload_note = match &outcome {
                            crate::embedding::UpdateOutcome::SilentOk => {
                                tracing::debug!(
                                    key = %key,
                                    "[HITL] apply_settings_update: value already matched live state"
                                );
                                HotReloadNote::None
                            }
                            crate::embedding::UpdateOutcome::HotReloaded { from, to } => {
                                tracing::info!(
                                    key = %key,
                                    from = %from,
                                    to = %to,
                                    "[HITL] apply_settings_update: hot-reloaded"
                                );
                                HotReloadNote::HotReloaded {
                                    from: from.clone(),
                                    to: to.clone(),
                                }
                            }
                            crate::embedding::UpdateOutcome::RequiresCorpusReembed { old, new } => {
                                tracing::warn!(
                                    key = %key,
                                    old = %old,
                                    new = %new,
                                    "[HITL] apply_settings_update: corpus reembed required — \
                                     stored vectors are now cross-model/dim"
                                );
                                HotReloadNote::RequiresReembed {
                                    old: old.clone(),
                                    new: new.clone(),
                                    key: key.to_string(),
                                }
                            }
                            crate::embedding::UpdateOutcome::Rejected { reason } => {
                                tracing::warn!(
                                    key = %key,
                                    reason = %reason,
                                    "[HITL] apply_settings_update: rejected by client — \
                                     setting row was stored but live client unchanged"
                                );
                                HotReloadNote::Rejected {
                                    reason: reason.clone(),
                                }
                            }
                        };
                    }

                    self.best_effort_runtime_setting_action_executed(
                        approval_id,
                        key,
                        &current,
                        true,
                    );
                    ConfigApplyResult::ChangedFromTo {
                        key: key.to_string(),
                        old: old
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or("(none)".into()),
                        new: current.to_string(),
                        note: hot_reload_note,
                    }
                }
                Err(e) => {
                    self.best_effort_runtime_setting_action_failed(
                        approval_id,
                        "runtime_setting_apply_failed",
                        &e.to_string(),
                    );
                    tracing::error!(key = %key, error = %e, "[HITL] Config change apply failed");
                    ConfigApplyResult::ApplyFailed {
                        key: key.to_string(),
                        error: e.to_string(),
                    }
                }
            }
        }
    }

    fn best_effort_runtime_setting_action_executed(
        &self,
        approval_id: &str,
        key: &str,
        proposed_value: &Value,
        changed: bool,
    ) {
        self.best_effort_runtime_setting_action_transition(
            approval_id,
            crate::action_ledger::ActionStatus::Executed,
            crate::action_ledger::ActionUpdate {
                external_receipt: Some(format!("runtime_setting:{key}:{approval_id}")),
                external_event_id: Some(approval_id.to_string()),
                user_visible_summary: Some(if changed {
                    format!("Runtime setting `{key}` updated")
                } else {
                    format!("Runtime setting `{key}` unchanged")
                }),
                metadata_json: Some(
                    serde_json::json!({
                        "approval_id": approval_id,
                        "setting_key": key,
                        "applied_value": proposed_value,
                        "changed": changed,
                    })
                    .to_string(),
                ),
                ..Default::default()
            },
        );
    }

    fn best_effort_runtime_setting_action_failed(
        &self,
        approval_id: &str,
        error_kind: &str,
        error_message: &str,
    ) {
        self.best_effort_runtime_setting_action_transition(
            approval_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some(error_kind.to_string()),
                error_message: Some(error_message.to_string()),
                ..Default::default()
            },
        );
    }

    fn best_effort_runtime_setting_action_transition(
        &self,
        approval_id: &str,
        status: crate::action_ledger::ActionStatus,
        update: crate::action_ledger::ActionUpdate,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let conn = crate::encrypted_sqlite::open(self.memory.db_path())?;
            crate::action_ledger::transition_runtime_setting_action(
                &conn,
                approval_id,
                status,
                &update,
            )?;
            Ok(())
        })();

        if let Err(e) = result {
            tracing::warn!(
                error = %e,
                approval_id,
                status = ?status,
                "failed to update runtime setting action ledger row"
            );
        }
    }

    pub(crate) async fn handle_config_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "ConfigApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.config.not_found_or_processed",
                &locale,
                approval_id,
            )));
        };

        // HITL-4 sibling hardening: reject config proposals with missing
        // `key` — previously this defaulted to "" and would silently call
        // `settings.set("", ...)` (noop or error depending on the store).
        // load_verify_clear_hitl has already cleared state above, so we
        // just return the corruption reply.
        let key = match proposal.get("key").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                tracing::error!(
                    approval_id,
                    field = "key",
                    "[HITL] Config approval rejected — missing/empty critical field"
                );
                return Ok(OrchestratorResult::reply(crate::i18n::t(
                    "hitl.approval_corrupted",
                    &locale,
                )));
            }
        };
        let proposed_value = proposal
            .get("proposed_value")
            .cloned()
            .unwrap_or(Value::Null);

        // Apply via the shared helper. heartbeat_config takes the dedicated
        // audit-logged path (design §7, Step 5); all other keys use the
        // generic settings pipeline — both branches live in
        // `apply_approved_config`.
        let apply_result = self
            .apply_approved_config(approval_id, key, &proposed_value)
            .await;

        // Channel A — this result is embedded in an LLM SYSTEM CALLBACK; keep
        // the raw English form (the LLM re-renders it in the user's language).
        // Do NOT localize here.
        let callback_msg = format!(
            "[SYSTEM CALLBACK — Config HITL Result]\n\
             The user APPROVED config change `{}`.\n\
             {}\n\
             Confirm the change to the user.",
            approval_id,
            apply_result.callback_string()
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    /// Handle "reject config <id>" — discard the proposal and resume
    /// the ReAct loop so the LLM can acknowledge the rejection.
    pub(crate) async fn handle_config_rejection(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "ConfigReject")
            .await?
        else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.config.not_found_maybe_processed",
                &locale,
                approval_id,
            )));
        };

        let key = proposal
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");

        self.best_effort_runtime_setting_action_transition(
            approval_id,
            crate::action_ledger::ActionStatus::Cancelled,
            crate::action_ledger::ActionUpdate {
                external_event_id: Some(approval_id.to_string()),
                user_visible_summary: Some(format!("Runtime setting `{key}` rejected")),
                ..Default::default()
            },
        );

        tracing::info!(approval_id, key, "[HITL] Config change rejected");

        let callback_msg = format!(
            "[SYSTEM CALLBACK — Config HITL Rejection]\n\
             The user REJECTED config change `{}`.\n\
             Rejected setting: {}\n\
             Acknowledge the rejection. If the user wants a different change, ask for details.",
            approval_id, key
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    /// Handle "approve revoke <id>" — execute Google credential revocation.
    pub(crate) async fn handle_revoke_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some((proposal, _user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "RevokeApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.revoke.not_found_or_processed",
                &locale,
                approval_id,
            )));
        };

        let account_name = proposal
            .get("account_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("primary")
            .to_string();

        // Execute revocation. Fail closed: if the selected account's
        // finalized credentials are missing or cannot be deleted, do not report
        // success.
        let revoke_result = if let (Some(db_path), Some(aes_key)) = (
            &self.credentials.credential_db_path,
            &self.credentials.credential_aes_key,
        ) {
            match crate::encrypted_sqlite::open(db_path)
                .map_err(|e| anyhow::anyhow!("DB open: {e}"))
                .and_then(|conn| crate::credential_store::CredentialStore::open(conn, *aes_key))
                .and_then(|store| {
                    crate::google_auth::GoogleAuthManager::delete_from_credential_store_account(
                        &store,
                        &account_name,
                    )
                }) {
                Ok(()) => Ok(()),
                Err(e) => Err(e),
            }
        } else {
            Err(anyhow::anyhow!("credential store is not configured"))
        };

        if let Err(e) = revoke_result {
            tracing::warn!(
                error = %format!("{e:#}"),
                account = %account_name,
                "Failed to delete selected Google OAuth credentials"
            );
            // PREFIX + concat: arbitrary error (`{e:#}`) never routed through
            // tf/tfn. `hitl.revoke.failed` carries only the account identifier.
            return Ok(OrchestratorResult::reply(format!(
                "{}{:#}",
                crate::i18n::tf("hitl.revoke.failed", &locale, &account_name),
                e
            )));
        }

        self.sandbox.clear_dynamic_auth(&["www.googleapis.com"]);

        tracing::info!(
            approval_id,
            account = %account_name,
            "[HITL] Google OAuth revocation approved and executed"
        );

        Ok(OrchestratorResult {
            reply: crate::i18n::tf("hitl.revoke.success", &locale, &account_name),
            ..Default::default()
        })
    }

    pub(crate) async fn handle_mcp_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some((proposal, _user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "McpApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.not_found_or_processed",
                &locale,
                approval_id,
            )));
        };

        let server_key = proposal
            .get("server_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if server_key.is_empty() {
            return Ok(OrchestratorResult {
                reply: crate::i18n::t("hitl.mcp.invalid_missing_key", &locale).to_string(),
                ..Default::default()
            });
        }

        // Add to approved servers list
        let current = self
            .settings
            .get("mcp_approved_servers")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let mut approved: Vec<&str> = current
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !approved.contains(&server_key.as_str()) {
            approved.push(&server_key);
        }
        let new_csv = approved.join(",");
        if let Err(e) = self
            .settings
            .set("mcp_approved_servers", Value::String(new_csv))
        {
            tracing::warn!(error = %e, "Failed to persist MCP approved servers");
        }

        tracing::info!(
            approval_id,
            server = server_key,
            "[HITL] MCP server approved"
        );

        Ok(OrchestratorResult {
            reply: crate::i18n::tf("hitl.mcp_approved", &locale, &server_key),
            ..Default::default()
        })
    }

    /// Handle MCP server rejection.
    pub(crate) async fn handle_mcp_rejection(
        &self,
        approval_id: &str,
        _chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Channel B — owner-facing results render in display_language (spec §2/§6).
        let locale = self.settings.display_language();
        let Some((proposal, _user_did)) = self
            .load_verify_clear_hitl(approval_id, _chat_id, "McpReject")
            .await?
        else {
            return Ok(OrchestratorResult::reply(crate::i18n::tf(
                "hitl.not_found_or_processed",
                &locale,
                approval_id,
            )));
        };

        let server_key = proposal
            .get("server_key")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)");

        tracing::info!(
            approval_id,
            server = server_key,
            "[HITL] MCP server rejected"
        );

        Ok(OrchestratorResult {
            reply: crate::i18n::tf("hitl.mcp_rejected", &locale, server_key),
            ..Default::default()
        })
    }

    pub(crate) async fn handle_menu_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "MenuApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Menu proposal `{}` not found or already processed.",
                approval_id
            )));
        };

        let proposed_custom: Vec<CustomCommand> = proposal
            .get("proposed_custom")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(CustomCommand::from_value).collect())
            .unwrap_or_default();
        let proposed_full = proposal
            .get("proposed_full")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| self.merge_core_and_custom(&proposed_custom));

        // Apply via Telegram API
        let apply_result = match self.apply_menu_to_telegram(&proposed_full).await {
            Ok(_) => {
                self.save_custom_commands(&proposed_custom).await;
                tracing::info!(
                    approval_id = %approval_id,
                    total = proposed_full.len(),
                    custom = proposed_custom.len(),
                    "[HITL] Bot menu change approved and applied"
                );
                format!(
                    "Bot menu updated: {} total commands ({} core + {} custom).",
                    proposed_full.len(),
                    crate::telegram::core_commands().len(),
                    proposed_custom.len(),
                )
            }
            Err(e) => {
                tracing::error!(
                    approval_id = %approval_id,
                    error = %e,
                    "[HITL] Bot menu change failed"
                );
                format!("Failed to update bot menu: {}", e)
            }
        };

        let callback_msg = format!(
            "[SYSTEM CALLBACK — Menu HITL Result]\n\
             The user APPROVED bot menu change `{}`.\n\
             {}\n\
             Confirm the result to the user.",
            approval_id, apply_result
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    /// Handle "reject menu <id>" — discard the menu proposal.
    pub(crate) async fn handle_menu_rejection(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        let Some((_proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "MenuReject")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Menu proposal `{}` not found (may have already been processed).",
                approval_id
            )));
        };

        tracing::info!(approval_id, "[HITL] Bot menu change rejected");

        let callback_msg = format!(
            "[SYSTEM CALLBACK — Menu HITL Rejection]\n\
             The user REJECTED bot menu change `{}`.\n\
             Acknowledge the rejection. If the user wants a different menu, ask for details.",
            approval_id
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    // ── Step 9b: kind-aware dispatcher for heartbeat-originated proposals ────
    //
    // Heartbeat proposals (kind = config_change) live in the same HITL table
    // as every other approval but need bespoke "on approve" semantics: apply a
    // heartbeat_config change via the audit-logged write path. This single
    // entry point reads the HITL row, branches on `kind`, and falls through
    // cleanly for unknown kinds.

    /// Handle `approve heartbeat <id>` — look up the HITL row, dispatch on
    /// `kind`.
    pub(crate) async fn handle_heartbeat_proposal_approval(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "HeartbeatProposalApprove")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Heartbeat proposal `{}` not found or already processed.",
                approval_id
            )));
        };

        let kind = proposal
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match kind.as_str() {
            "config_change" => {
                self.approve_heartbeat_config_change(approval_id, &proposal, &user_did, chat_id)
                    .await
            }
            other => Ok(OrchestratorResult::reply(format!(
                "Heartbeat proposal `{}` has unsupported kind `{}`.",
                approval_id, other
            ))),
        }
    }

    /// Handle `reject heartbeat <id>` — just clear the row and notify.
    pub(crate) async fn handle_heartbeat_proposal_rejection(
        &self,
        approval_id: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        let Some((proposal, user_did)) = self
            .load_verify_clear_hitl(approval_id, chat_id, "HeartbeatProposalReject")
            .await?
        else {
            return Ok(OrchestratorResult::reply(format!(
                "Heartbeat proposal `{}` not found (may have already been processed).",
                approval_id
            )));
        };
        let kind = proposal
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("generic")
            .to_string();
        tracing::info!(approval_id, kind, "[HITL] Heartbeat proposal rejected");
        let callback_msg = format!(
            "[SYSTEM CALLBACK — Heartbeat HITL Rejection]\n\
             The user REJECTED heartbeat proposal `{}` (kind={}).\n\
             Acknowledge the rejection; if the user wants a variant, ask.",
            approval_id, kind
        );
        self.resume_with_hitl_callback(&user_did, &callback_msg, chat_id)
            .await
    }

    async fn approve_heartbeat_config_change(
        &self,
        approval_id: &str,
        proposal: &Value,
        user_did: &str,
        chat_id: Option<i64>,
    ) -> Result<OrchestratorResult> {
        // Re-validate at apply time: the current config may have shifted since
        // the proposal was created, so the merged preview could now be stale.
        let changes = proposal.get("changes").cloned().unwrap_or(Value::Null);
        let reason = proposal
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let current = self.heartbeat_config();
        let mut merged = serde_json::to_value(&current).unwrap_or(Value::Null);
        if let (Some(obj), Some(patch_obj)) = (merged.as_object_mut(), changes.as_object()) {
            for (k, v) in patch_obj {
                if let (Some(sub), Some(patch_sub)) = (
                    obj.get_mut(k).and_then(|x| x.as_object_mut()),
                    v.as_object(),
                ) {
                    for (sk, sv) in patch_sub {
                        sub.insert(sk.clone(), sv.clone());
                    }
                } else {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let new_cfg = match crate::heartbeat_config::HeartbeatConfig::from_json(&merged) {
            Some(c) => c,
            None => {
                return Ok(OrchestratorResult::reply(format!(
                    "Config proposal `{}` payload could not be decoded at apply time.",
                    approval_id
                )));
            }
        };
        if new_cfg.frequency_minutes == 0
            || new_cfg.per_tick.tool_calls == 0
            || new_cfg.dedup.cosine_threshold < 0.0
            || new_cfg.dedup.cosine_threshold > 1.0
            || new_cfg.next_tick_note_max_bytes == 0
        {
            return Ok(OrchestratorResult::reply(format!(
                "Config proposal `{}` failed re-validation at apply time; discarded.",
                approval_id
            )));
        }

        // Step 5 TODO(future) unblock: proposals always originate from the
        // heartbeat LLM (cycle_id is stamped on the HITL row), so actor=Llm
        // is the correct attribution on the user-approved apply path.
        let apply_msg = match self
            .update_heartbeat_config(
                new_cfg,
                crate::heartbeat_config::ConfigAuditActor::Llm,
                Some(format!("HITL heartbeat proposal {approval_id}: {reason}")),
            )
            .await
        {
            Ok(true) => {
                tracing::info!(
                    approval_id,
                    "[HITL] heartbeat_config proposal approved + audit-logged"
                );
                format!(
                    "✅ heartbeat_config updated via proposal `{}`.",
                    approval_id
                )
            }
            Ok(false) => format!(
                "Proposal `{}` applied no changes (already matched current config).",
                approval_id
            ),
            Err(e) => {
                tracing::error!(approval_id, error = %e, "[HITL] heartbeat_config apply failed");
                format!("Failed to apply config change: {e}")
            }
        };

        let callback_msg = format!(
            "[SYSTEM CALLBACK — Heartbeat Config Approved]\n\
             Proposal `{}` was APPROVED.\n\
             {}\n\
             Confirm briefly to the user.",
            approval_id, apply_msg
        );
        self.resume_with_hitl_callback(user_did, &callback_msg, chat_id)
            .await
    }
}

/// Format raw tool output for human-readable Telegram display.
/// Used by gate direct-execution path where there's no LLM to format the result.
///
/// Channel B — this renders STRAIGHT to the owner (no LLM re-render), so the
/// translatable label ("Error: ") resolves in the owner's `locale`
/// (display_language). `tool_name`, the JSON field NAMES, and the tool-authored
/// `message`/`error`/`hint`/data values are arbitrary/identifier content and
/// pass through verbatim (the error text is concatenated AFTER the localized
/// prefix — never re-scanned).
fn format_gate_result(tool_name: &str, raw_output: &str, locale: &str) -> String {
    // Try to parse as JSON and extract meaningful fields
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_output) {
        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        if tool_name == "run_external" {
            let zh = locale.starts_with("zh");
            let completed = status == "ok";
            let label = match v.get("executor").and_then(Value::as_str) {
                Some("grok_acp") => "Grok",
                Some("codex_app_server") => "Codex",
                _ if zh => "外部执行器",
                _ => "External executor",
            };
            let outcome = match (completed, zh) {
                (true, true) => "已完成",
                (true, false) => "completed",
                (false, true) => "未完成，请重试或检查执行器设置",
                (false, false) => "did not complete; retry or check executor settings",
            };
            let icon = if completed { "✅" } else { "⚠️" };
            let mut reply = format!("{icon} {label} · {outcome}");
            if let Some(text) = v.get("final_text").and_then(Value::as_str)
                && !text.trim().is_empty()
            {
                reply.push_str("\n\n");
                reply.push_str(&crate::telegram_render::escape_html(text));
            }
            // The structured receipt retains diagnostics. Local paths, raw
            // stderr and protocol fields are not the owner's task result.
            return reply;
        }
        let is_error = status == "error";

        let icon = if is_error { "⚠️" } else { "✅" };

        // Build message from known fields
        let mut parts = vec![format!("{icon} <b>{tool_name}</b>")];

        if let Some(msg) = v.get("message").and_then(|s| s.as_str()) {
            parts.push(msg.to_string());
        }
        if let Some(err) = v.get("error").and_then(|s| s.as_str()) {
            // PREFIX-only: localized "Error: " label + arbitrary error text
            // concatenated AFTER (an error string may contain a literal `{}`,
            // so it must never pass through tf/tfn re-scan).
            parts.push(format!(
                "{}{err}",
                crate::i18n::t("gate.result.error_prefix", locale)
            ));
        }
        if let Some(hint) = v.get("hint").and_then(|s| s.as_str()) {
            parts.push(format!("💡 {hint}"));
        }
        // Render step-by-step instructions if present
        if let Some(steps) = v.get("steps").and_then(|s| s.as_array()) {
            parts.push(String::new()); // blank line
            for step in steps {
                if let Some(s) = step.as_str() {
                    parts.push(s.to_string());
                }
            }
        }
        // Always surface URL fields — these are actionable and must not be hidden
        for url_key in &["auth_url", "url", "link"] {
            if let Some(url) = v.get(*url_key).and_then(|s| s.as_str()) {
                parts.push(format!("\n{url}"));
            }
        }

        // If no message/error extracted, show a brief summary of data fields
        if parts.len() == 1 {
            // Just the header — add key-value pairs for top-level scalars
            for (k, val) in v.as_object().into_iter().flatten() {
                if k == "status" {
                    continue;
                }
                match val {
                    serde_json::Value::String(s) => parts.push(format!("{k}: {s}")),
                    serde_json::Value::Bool(b) => parts.push(format!("{k}: {b}")),
                    serde_json::Value::Number(n) => parts.push(format!("{k}: {n}")),
                    _ => {} // skip arrays/objects
                }
            }
        }

        parts.join("\n")
    } else {
        // Not JSON — wrap as-is
        format!("✅ <b>{tool_name}</b>\n{raw_output}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── verify_hitl_chat_id ──────────────────────────────────────────────

    #[test]
    fn verify_chat_id_matching() {
        let approval = json!({"chat_id": 12345});
        assert!(
            Orchestrator::verify_hitl_chat_id(&approval, Some(12345), "test-1", "zh").is_none()
        );
    }

    #[test]
    fn verify_delivery_chat_id_matching() {
        let approval = json!({
            "chat_id": 12345,
            "delivery_chat_id": -1001000000001_i64,
            "allowed_chat_ids": [12345, -1001000000001_i64],
        });
        assert!(
            Orchestrator::verify_hitl_chat_id(
                &approval,
                Some(-1001000000001_i64),
                "test-delivery",
                "zh"
            )
            .is_none()
        );
    }

    #[test]
    fn verify_chat_id_mismatch_blocked() {
        let approval = json!({"chat_id": 12345});
        let result = Orchestrator::verify_hitl_chat_id(&approval, Some(99999), "test-2", "zh");
        assert!(result.is_some());
        let reply = result.unwrap().reply;
        // i18n-aware: check for either Chinese or English text
        assert!(
            reply.contains("拒绝访问") || reply.contains("Access denied"),
            "reply = {reply}"
        );
        assert!(reply.contains("test-2"));
    }

    #[test]
    fn verify_chat_id_missing_stored_rejects() {
        // Unbound approvals (no stored chat_id) are rejected for any chat that is
        // not the owner's (cross-chat hijack guard) …
        let approval = json!({"plan": "something"});
        let admin = crate::telegram::admin_dm_chat(0).0;
        let foreign = if admin == 12345 { 54321 } else { 12345 };
        assert!(
            Orchestrator::verify_hitl_chat_id(&approval, Some(foreign), "test-3", "zh").is_some()
        );
        // … but the owner's own chat may approve one (2026-09-13: approvals
        // delivered by web push were never bound and expired unanswered).
        assert!(
            Orchestrator::verify_hitl_chat_id(&approval, Some(admin), "test-3", "zh").is_none()
        );
    }

    #[test]
    fn verify_chat_id_missing_caller_passes() {
        let approval = json!({"chat_id": 12345});
        assert!(Orchestrator::verify_hitl_chat_id(&approval, None, "test-4", "zh").is_none());
    }

    #[test]
    fn verify_chat_id_both_missing_passes() {
        let approval = json!({});
        assert!(Orchestrator::verify_hitl_chat_id(&approval, None, "test-5", "zh").is_none());
    }
    // ── format_job_results ───────────────────────────────────────────────

    #[test]
    fn format_completed_job() {
        let body = json!({
            "status": "completed",
            "elapsed_secs": 10,
            "results": [
                {"command": "echo hello", "exit_code": 0, "stdout": "hello\n", "stderr": ""}
            ]
        });
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("successfully (10s)"), "out = {out}");
        assert!(out.contains("$ echo hello"));
        assert!(out.contains("[exit 0]"));
        assert!(out.contains("hello\n"));
    }

    #[test]
    fn format_failed_job() {
        let body = json!({
            "status": "failed",
            "elapsed_secs": 5,
            "results": [
                {"command": "rm -rf /", "exit_code": 1, "stdout": "", "stderr": "permission denied"}
            ]
        });
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("failed (5s)"), "out = {out}");
        assert!(out.contains("[stderr] permission denied"));
    }

    #[test]
    fn format_timed_out_job() {
        let body = json!({"status": "timed_out", "elapsed_secs": 120});
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("timed out (120s)"), "out = {out}");
    }

    #[test]
    fn format_cancelled_job() {
        let body = json!({"status": "cancelled", "elapsed_secs": 3});
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("cancelled (3s)"), "out = {out}");
    }

    #[test]
    fn format_unknown_status() {
        let body = json!({"status": "partial", "elapsed_secs": 7});
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("'partial'"), "out = {out}");
    }

    #[test]
    fn format_truncates_long_stdout() {
        let long_output = "x".repeat(3000);
        let body = json!({
            "status": "completed",
            "elapsed_secs": 1,
            "results": [{"command": "cat big", "exit_code": 0, "stdout": long_output, "stderr": ""}]
        });
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("...(truncated)"), "out = {out}");
        assert!(!out.contains(&"x".repeat(3000)));
    }

    #[test]
    fn format_empty_results() {
        let body = json!({"status": "completed", "elapsed_secs": 0, "results": []});
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("successfully"), "out = {out}");
        assert!(!out.contains("$ "));
    }

    #[test]
    fn format_missing_fields_defaults() {
        let body = json!({});
        let out = Orchestrator::format_job_results(&body);
        assert!(out.contains("'unknown'"), "out = {out}");
    }

    #[test]
    fn format_gate_result_extracts_json_fields() {
        let raw = r#"{"status":"ok","message":"Google configured","configured":true}"#;
        let out = super::format_gate_result("setup_google", raw, "en");
        assert!(
            out.contains("setup_google"),
            "must include tool name: {out}"
        );
        assert!(
            out.contains("Google configured"),
            "must include message: {out}"
        );
        assert!(
            !out.contains(r#""status""#),
            "must not dump raw JSON keys: {out}"
        );
    }

    #[test]
    fn format_gate_result_handles_non_json() {
        let raw = "plain text result";
        let out = super::format_gate_result("some_tool", raw, "en");
        assert!(out.contains("some_tool"), "must include tool name: {out}");
        assert!(
            out.contains("plain text result"),
            "must include content: {out}"
        );
    }

    #[test]
    fn format_gate_result_error_status() {
        let raw = r#"{"status":"error","error":"Auth failed","hint":"Re-authenticate"}"#;
        let out = super::format_gate_result("setup_google", raw, "en");
        assert!(out.contains("Auth failed"), "must include error: {out}");
        assert!(out.contains("Re-authenticate"), "must include hint: {out}");
        // Channel B: the "Error: " label localizes; the arbitrary error text
        // (which may contain a literal `{}`) survives verbatim after the prefix.
        let en = super::format_gate_result(
            "setup_google",
            r#"{"status":"error","error":"boom at {} token"}"#,
            "en",
        );
        assert!(en.contains("Error: boom at {} token"), "en error: {en}");
        let zh = super::format_gate_result(
            "setup_google",
            r#"{"status":"error","error":"boom at {} token"}"#,
            "zh",
        );
        assert!(
            zh.contains("错误：boom at {} token"),
            "zh error localized + verbatim payload: {zh}"
        );
    }

    #[test]
    fn external_approval_result_keeps_answer_without_raw_diagnostics() {
        let raw = json!({
            "status": "ok", "executor": "codex_app_server",
            "final_text": "Unread messages: 2 <verified>",
            "stderr_tail": "raw protocol warning", "run_dir": "/private/run",
            "fallback_used": false, "stop_reason": "completed"
        })
        .to_string();
        let output = super::format_gate_result("run_external", &raw, "en");
        assert!(output.contains("Codex · completed"));
        assert!(output.contains("Unread messages: 2 &lt;verified&gt;"));
        assert!(!output.contains("raw protocol") && !output.contains("/private/run"));
        for status in ["failed", "error", "timeout"] {
            let raw = json!({"status": status, "executor": "grok_acp"}).to_string();
            let output = super::format_gate_result("run_external", &raw, "zh");
            assert!(output.contains("Grok · 未完成"));
            assert!(!output.contains('✅'));
        }
    }

    #[test]
    fn format_gate_result_surfaces_auth_url() {
        let raw = r#"{"status":"ok","message":"Click to authorize","auth_url":"https://accounts.google.com/o/oauth2/auth?foo=bar"}"#;
        let out = super::format_gate_result("setup_google", raw, "en");
        assert!(
            out.contains("https://accounts.google.com/o/oauth2/auth?foo=bar"),
            "must include auth_url: {out}"
        );
        assert!(
            out.contains("Click to authorize"),
            "must include message: {out}"
        );
    }

    #[test]
    fn format_gate_result_renders_steps() {
        let raw = serde_json::json!({
            "status": "auth_url_ready",
            "message": "Google OAuth",
            "steps": ["Step 1: Do this", "Step 2: Do that"],
            "auth_url": "https://example.com/auth"
        })
        .to_string();
        let out = super::format_gate_result("setup_google", &raw, "en");
        assert!(
            out.contains("Step 1: Do this"),
            "must include step 1: {out}"
        );
        assert!(
            out.contains("Step 2: Do that"),
            "must include step 2: {out}"
        );
        assert!(
            out.contains("https://example.com/auth"),
            "must include auth_url after steps: {out}"
        );
        // Steps should appear before auth_url
        let step_pos = out.find("Step 1").unwrap();
        let url_pos = out.find("https://example.com").unwrap();
        assert!(step_pos < url_pos, "steps must come before auth_url: {out}");
    }

    // ── ConfigApplyResult dual-channel rendering (TAIL 1) ────────────────

    #[test]
    fn config_apply_result_callback_string_is_raw_english_unchanged() {
        use super::{ConfigApplyResult, HotReloadNote};
        // callback_string MUST reproduce the historical hardcoded English forms
        // verbatim (Channel A is LLM-bound; the LLM is prompted on this phrasing).
        assert_eq!(
            ConfigApplyResult::UpdatedTo {
                key: "k".into(),
                value: "true".into()
            }
            .callback_string(),
            "Setting 'k' updated to true."
        );
        assert_eq!(
            ConfigApplyResult::Unchanged { key: "k".into() }.callback_string(),
            "Setting 'k' unchanged (payload matched current)."
        );
        assert_eq!(
            ConfigApplyResult::ApplyFailed {
                key: "k".into(),
                error: "boom".into()
            }
            .callback_string(),
            "Failed to apply setting 'k': boom"
        );
        assert_eq!(
            ConfigApplyResult::InvalidPayload { key: "k".into() }.callback_string(),
            "Failed to apply setting 'k': invalid JSON payload"
        );
        assert_eq!(
            ConfigApplyResult::ChangedFromTo {
                key: "k".into(),
                old: "1".into(),
                new: "2".into(),
                note: HotReloadNote::None,
            }
            .callback_string(),
            "Setting 'k' changed from 1 to 2."
        );
        assert_eq!(
            ConfigApplyResult::ChangedFromTo {
                key: "embedding_model".into(),
                old: "a".into(),
                new: "b".into(),
                note: HotReloadNote::HotReloaded {
                    from: "a".into(),
                    to: "b".into()
                },
            }
            .callback_string(),
            "Setting 'embedding_model' changed from a to b. Hot-reloaded (a → b)."
        );
    }

    #[test]
    fn config_apply_result_localized_uses_locale_and_preserves_arbitrary_args() {
        use super::{ConfigApplyResult, HotReloadNote};
        // Channel B renders in display_language; identity/logic unchanged — only
        // the display label differs. zh must NOT read like the English callback.
        for (loc, needle) in [
            ("zh", "设置 'k' 已更新为"),
            ("en", "Setting 'k' updated to"),
        ] {
            let s = ConfigApplyResult::UpdatedTo {
                key: "k".into(),
                value: "true".into(),
            }
            .localized(loc);
            assert!(s.contains(needle), "updated_to ({loc}): {s}");
        }

        // ARBITRARY args carrying a literal `{}` / `{1}` must survive verbatim
        // (tfp single-pass, no re-scan) in BOTH locales — for the all-arbitrary
        // ChangedFromTo and the PREFIX-only ApplyFailed.
        for loc in ["zh", "en"] {
            let cf = ConfigApplyResult::ChangedFromTo {
                key: "k {0}".into(),
                old: "old {1}".into(),
                new: "new {}".into(),
                note: HotReloadNote::None,
            }
            .localized(loc);
            assert!(cf.contains("k {0}"), "key corrupted ({loc}): {cf}");
            assert!(cf.contains("old {1}"), "old corrupted ({loc}): {cf}");
            assert!(cf.contains("new {}"), "new corrupted ({loc}): {cf}");

            // ApplyFailed is PREFIX-only: arbitrary error concatenated AFTER.
            let af = ConfigApplyResult::ApplyFailed {
                key: "k {1}".into(),
                error: "serde at {} token".into(),
            }
            .localized(loc);
            assert!(af.contains("k {1}"), "fail key corrupted ({loc}): {af}");
            assert!(
                af.ends_with("serde at {} token"),
                "fail error corrupted ({loc}): {af}"
            );

            // Hot-reload note (RequiresReembed) — old/new/key all arbitrary.
            let rr = ConfigApplyResult::ChangedFromTo {
                key: "embedding_x".into(),
                old: "v1".into(),
                new: "v2".into(),
                note: HotReloadNote::RequiresReembed {
                    old: "o {0}".into(),
                    new: "n {}".into(),
                    key: "embedding_x".into(),
                },
            }
            .localized(loc);
            assert!(rr.contains("o {0}"), "reembed old corrupted ({loc}): {rr}");
            assert!(rr.contains("n {}"), "reembed new corrupted ({loc}): {rr}");

            // Rejected note — reason arbitrary.
            let rj = HotReloadNote::Rejected {
                reason: "why {0} {}".into(),
            }
            .localized(loc);
            assert!(rj.contains("why {0} {}"), "reject reason ({loc}): {rj}");
        }
    }

    // ── Step 9: kind_default_ttl_seconds + find_similar_pending ──────────

    #[test]
    fn kind_default_ttl_config_is_24h() {
        assert_eq!(
            super::kind_default_ttl_seconds("config_change"),
            24 * 60 * 60
        );
    }

    #[test]
    fn kind_default_ttl_generic_is_1h() {
        assert_eq!(super::kind_default_ttl_seconds("generic"), 60 * 60);
        assert_eq!(super::kind_default_ttl_seconds("something_else"), 60 * 60);
    }

    #[tokio::test]
    async fn find_similar_pending_detects_above_threshold() {
        let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let now = chrono::Utc::now();
        let emb_a = vec![1.0_f32, 0.0, 0.0];
        let emb_b = vec![0.99_f32, 0.01, 0.0]; // highly similar
        let emb_orthogonal = vec![0.0_f32, 1.0, 0.0];

        mem.store_hitl_approval(
            "candidate-1",
            "{}",
            "config_change",
            None,
            Some(&emb_a),
            now + chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        mem.store_hitl_approval(
            "candidate-2",
            "{}",
            "config_change",
            None,
            Some(&emb_orthogonal),
            now + chrono::Duration::hours(24),
        )
        .await
        .unwrap();

        // Similar vector → dedup hit on candidate-1.
        let hit = super::find_similar_pending(
            &mem,
            "config_change",
            &emb_b,
            0.85,
            now - chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        assert_eq!(hit.as_deref(), Some("candidate-1"));

        // An orthogonal query matches candidate-2 (which is itself orthogonal to emb_a).
        // Use a far-from-both vector to confirm no match.
        let far = vec![0.0_f32, 0.0, 1.0];
        let miss = super::find_similar_pending(
            &mem,
            "config_change",
            &far,
            0.85,
            now - chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn find_similar_pending_respects_kind() {
        let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let now = chrono::Utc::now();
        let emb = vec![1.0_f32, 0.0, 0.0];
        mem.store_hitl_approval(
            "cfg-1",
            "{}",
            "config_change",
            None,
            Some(&emb),
            now + chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        // Query a different kind — must not cross-match the config row.
        let hit = super::find_similar_pending(
            &mem,
            "generic",
            &emb,
            0.85,
            now - chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        assert!(hit.is_none());
    }

    #[tokio::test]
    async fn find_similar_pending_respects_since_window() {
        let mem = crate::memory::MemoryHandle::spawn_in_memory().unwrap();
        let now = chrono::Utc::now();
        let emb = vec![1.0_f32, 0.0, 0.0];
        mem.store_hitl_approval(
            "old-1",
            "{}",
            "config_change",
            None,
            Some(&emb),
            now + chrono::Duration::hours(24),
        )
        .await
        .unwrap();
        // `since` in the future — row's updated_at (= now) is older than
        // the window → no hit.
        let hit = super::find_similar_pending(
            &mem,
            "config_change",
            &emb,
            0.85,
            now + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
        assert!(hit.is_none());
    }

    // ── HITL 4-bug audit regression tests (HITL-1/2/3/4) ────────────────

    /// Helper: build an orchestrator + in-memory DB for HITL gate tests.
    async fn setup_gate_test_orch() -> crate::orchestrator::Orchestrator {
        use crate::memory::MemoryHandle;
        use crate::settings::RuntimeSettings;
        use crate::test_tool_helpers::test_orchestrator;
        use std::sync::Arc;

        let mem = MemoryHandle::spawn_in_memory().unwrap();
        let settings = Arc::new(RuntimeSettings::new());
        test_orchestrator(mem, settings)
    }

    /// Helper: seed a pending HITL approval row under `skill_state[hitl]`
    /// with the supplied JSON payload.
    async fn seed_hitl(orch: &crate::orchestrator::Orchestrator, key: &str, payload: &Value) {
        orch.memory()
            .store_skill_state("hitl", key, &payload.to_string())
            .await
            .unwrap();
    }

    /// HITL-1 · Chat-id-mismatched HITL load must clear the stale approval
    /// state. Previously the mismatch branch returned Ok(None) leaving the
    /// approval in place, which let an attacker from a side chat probe the
    /// same approval_id while the legitimate chat could still approve it
    /// later — enabling double-execute.
    #[tokio::test]
    async fn chat_id_mismatch_clears_state() {
        let orch = setup_gate_test_orch().await;
        let key = "hitl-mismatch-1";
        let proposal = json!({
            "chat_id": 100,
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "plan": "anything",
        });
        seed_hitl(&orch, key, &proposal).await;

        // Sanity: state is present before the mismatched probe.
        let before = orch
            .memory()
            .load_skill_state("hitl", key)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(!before.is_empty(), "precondition: HITL state stored");

        // Probe with a DIFFERENT chat_id — must fail-closed AND clear.
        let result = orch
            .load_verify_clear_hitl(key, Some(200), "test-mismatch")
            .await
            .expect("load_verify_clear_hitl should not error");
        assert!(result.is_none(), "mismatched chat_id must return None");

        let after = orch
            .memory()
            .load_skill_state("hitl", key)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            after.is_empty(),
            "HITL state must be cleared after chat_id mismatch (was: {after:?})"
        );
    }

    /// HITL-2 · On execute failure the pending HITL state must still be
    /// cleared (so the prompt no longer appears pending), and the stored
    /// tool approval for non-Once scopes must persist (so the LLM can
    /// retry without re-prompting). Previously the clear happened BEFORE
    /// execute so on Err the approval_id was gone and retries hit
    /// "not found".
    #[tokio::test]
    async fn execute_failure_clears_state_and_preserves_stored_approval() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-exec-fail-1";

        // Seed approval with a tool name that does not exist — the
        // execute path will return an error-shaped result (from the
        // unknown-tool dispatch fallthrough). We pick Always scope so
        // store_tool_approval is exercised.
        let payload = json!({
            "chat_id": 42,
            "tool_name": "nonexistent_tool_for_hitl2_test",
            "tool_args": "{}",
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        let result = orch
            .handle_gate_approval_scoped(
                approval_id,
                crate::tool_gate::ApprovalScope::Always,
                "did:test:user",
                Some(42),
            )
            .await
            .expect("gate handler should return Ok even when inner execute fails");

        // The HITL state must be cleared regardless of exec outcome.
        let state_after = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            state_after.is_empty(),
            "HITL state must be cleared after execute completes (was: {state_after:?})"
        );

        // Stored approvals must persist for Always scope — this is the
        // "user intent survives execution failure" property.
        let approvals = orch
            .load_tool_approvals("nonexistent_tool_for_hitl2_test")
            .await;
        assert!(
            approvals
                .iter()
                .any(|a| matches!(a.scope, crate::tool_gate::ApprovalScope::Always)),
            "Always-scope approval must have been stored before/around execute even on failure: {approvals:?}"
        );

        // Reply is non-empty and reflects execution having been attempted
        // (either a formatted error or a formatted tool output).
        assert!(
            !result.reply.is_empty(),
            "reply must report outcome, got empty"
        );
    }

    /// HITL-3 · When the caller requests Intent scope but the stored
    /// approval data has no `intent_group_key`, the scope silently
    /// downgrades to Once. The user-facing reply MUST include a
    /// bilingual downgrade notice so the user knows their "persistent"
    /// approval only applies to this call.
    #[tokio::test]
    async fn intent_scope_without_group_key_user_sees_fallback_notice() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-intent-fallback-1";

        // Seed with tool_name + tool_args but NO intent_group_key field.
        let payload = json!({
            "chat_id": 7,
            "tool_name": "nonexistent_tool_for_hitl3_test",
            "tool_args": "{}",
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        let result = orch
            .handle_gate_approval_scoped(
                approval_id,
                crate::tool_gate::ApprovalScope::Intent {
                    group_key: String::new(),
                },
                "did:test:user",
                Some(7),
            )
            .await
            .expect("gate handler should return Ok");

        // Either zh or en wording must appear — matches the two
        // registered i18n strings for hitl.scope_fallback_intent_to_once.
        assert!(
            result.reply.contains("降级")
                || result.reply.contains("downgraded")
                || result.reply.contains("一次性批准")
                || result.reply.contains("one-shot"),
            "reply must mention the Intent→Once downgrade, got: {}",
            result.reply
        );
    }

    /// HITL-4 · An approval JSON missing `tool_name` must be rejected
    /// explicitly: state cleared, bilingual corruption reply returned,
    /// NO stored tool approval side-effect.
    #[tokio::test]
    async fn missing_tool_name_rejects_cleanly() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-corrupt-1";

        // Seed with tool_name omitted entirely — tool_args present.
        let payload = json!({
            "chat_id": 9,
            "tool_args": "{\"x\":1}",
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        let result = orch
            .handle_gate_approval_scoped(
                approval_id,
                crate::tool_gate::ApprovalScope::Always,
                "did:test:user",
                Some(9),
            )
            .await
            .expect("gate handler should return Ok");

        // State must be cleared.
        let state_after = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            state_after.is_empty(),
            "HITL state must be cleared after corrupt-field rejection"
        );

        // Reply must contain the bilingual corruption notice.
        assert!(
            result.reply.contains("批准数据损坏")
                || result.reply.contains("Approval data corrupted"),
            "reply must surface corruption notice, got: {}",
            result.reply
        );

        // No approval should have been stored for the (empty) tool name,
        // nor for any tool — execute_approved_tool must NOT have been
        // called (its side-effect in execute_single_tool pushes a
        // "Unknown tool" tool-result which we cannot directly inspect,
        // but the store_tool_approval side-effect IS inspectable: assert
        // there is no stored approval at all for this approval_id's
        // tool_name).
        //
        // Because tool_name was missing, the only pool to check is the
        // lookup for "" which load_tool_approvals normalizes to empty.
        let approvals = orch.load_tool_approvals("").await;
        assert!(
            approvals.is_empty(),
            "no approval must be stored for empty tool_name, got: {approvals:?}"
        );
    }

    // ── HITL-1/4 paired-fix regression tests (CTO round-2 siblings) ────

    /// HITL-1 paired · `handle_gate_approval_scoped` is the high-impact
    /// sibling of `load_verify_clear_hitl`'s chat-id check: it actually
    /// executes tools. A probe from the wrong chat_id must clear the
    /// stale approval state so the legitimate chat can't later approve
    /// (double-execute surface).
    #[tokio::test]
    async fn gate_approval_chat_id_mismatch_clears_state() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-gate-mismatch-1";
        let payload = json!({
            "chat_id": 100,
            "tool_name": "nonexistent_tool_for_gate_mismatch_test",
            "tool_args": "{}",
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        let before = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(!before.is_empty(), "precondition: state stored");

        // Approver from chat_id=200 when the request came from chat_id=100.
        let result = orch
            .handle_gate_approval_scoped(
                approval_id,
                crate::tool_gate::ApprovalScope::Once,
                "did:test:user",
                Some(200),
            )
            .await
            .expect("gate handler should return Ok(denied) on mismatch");
        // Reply should be the verify_hitl_chat_id denial, not an execution result.
        assert!(!result.reply.is_empty(), "denial reply must be non-empty");

        let after = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            after.is_empty(),
            "gate-approval chat_id mismatch must clear state (was: {after:?})"
        );
    }
    /// HITL-1 paired (CTO round-2 sibling) · `handle_gate_clarify` is the
    /// read-only echo path — it doesn't execute but it DOES echo tool_name
    /// and tool_args back into an LLM prompt visible to the caller. Before
    /// the fix, any chat could clarify any approval_id (information
    /// disclosure). After: chat-id mismatch denies WITHOUT clearing state
    /// (the legitimate chat should still be able to approve/reject).
    #[tokio::test]
    async fn gate_clarify_chat_id_mismatch_denies_without_clearing() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-clarify-mismatch-1";
        let payload = json!({
            "chat_id": 100,
            "tool_name": "leaked_tool_name",
            "tool_args": "{\"secret\":\"value\"}",
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        // Probe clarify from wrong chat.
        let result = orch
            .handle_gate_clarify(approval_id, "did:test:user", Some(200))
            .await
            .expect("clarify handler should return Ok(denied) on mismatch");

        // Denial reply must NOT contain the sensitive tool_name or args.
        assert!(
            !result.reply.contains("leaked_tool_name"),
            "mismatch denial must not leak tool_name: {}",
            result.reply
        );
        assert!(
            !result.reply.contains("secret"),
            "mismatch denial must not leak tool_args: {}",
            result.reply
        );

        // State must remain — legitimate chat still has a pending approval.
        let after = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            !after.is_empty(),
            "clarify mismatch must NOT clear state (legit chat still pending)"
        );
    }

    // ── Task #12 · Gate → Config pre-approval integration ─────────────────

    /// Task #12 end-to-end: user approves the Gate on an `update_setting`
    /// call (Once scope) → `handle_gate_approval_scoped` replays the tool →
    /// `execute_approved_tool` constructs a ToolContext with
    /// `gate_pre_approved=true` → `orchestrator_tools_system.rs` routes
    /// `update_setting` through `create_config_proposal_ex` with the flag →
    /// the setting is applied synchronously without creating a second
    /// pending Config proposal. The user sees ONE approval, not two.
    ///
    /// Regression guard for the "approve twice" UX bug the user reported.
    #[tokio::test]
    async fn gate_approval_auto_preapproves_config_proposal() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hitl-gate-config-preapprove-1";
        let chat_id: i64 = 4242;
        let target_key = "embedding_primary_model";
        let new_value = "qwen/qwen3-embedding-8b";

        // Seed a pending Gate approval for update_setting. This is exactly
        // what `execute_single_tool` stashes when it sees a RequireApproval
        // verdict for update_setting.
        let tool_args = json!({
            "key": target_key,
            "value": new_value,
        })
        .to_string();
        let payload = json!({
            "chat_id": chat_id,
            "tool_name": "update_setting",
            "tool_args": tool_args,
            "user_did": "did:test:user",
            "created_at": chrono::Utc::now().to_rfc3339(),
        });
        seed_hitl(&orch, approval_id, &payload).await;

        // Capture pre-state of the live setting.
        let before = orch.settings.get(target_key);

        // User taps "Approve Once" — drives the whole replay chain.
        let result = orch
            .handle_gate_approval_scoped(
                approval_id,
                crate::tool_gate::ApprovalScope::Once,
                "did:test:user",
                Some(chat_id),
            )
            .await
            .expect("gate handler must return Ok on successful replay");

        // Reply must not carry "pending_human_approval" (that would mean a
        // second Config proposal was created and is awaiting the user).
        assert!(
            !result.reply.contains("pending_human_approval"),
            "Gate-approved update_setting must NOT spawn a second Config proposal awaiting approval — reply: {}",
            result.reply
        );

        // The Gate HITL row is cleared by handle_gate_approval_scoped.
        let gate_state = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            gate_state.is_empty(),
            "Gate HITL state must be cleared after approval (was: {gate_state:?})"
        );

        // The setting landed in live state — the Config proposal was
        // pre-approved and applied synchronously.
        let after = orch.settings.get(target_key);
        assert_ne!(
            before, after,
            "target setting must have changed (before={before:?}, after={after:?})"
        );
        assert_eq!(
            after,
            Some(serde_json::Value::String(new_value.to_string())),
            "setting must match the proposed value — got {:?}",
            after
        );

        // Scan all HITL rows: no config-* proposal should be pending. (The
        // pre-approve path writes then immediately clears its config-* row;
        // a regression would leave one behind.)
        //
        // We lean on the public load_hitl_state helper indirectly via
        // load_skill_state scan of the expected id pattern — since the
        // approval_id is random, we can't address it directly. Instead,
        // verify that `settings.set` succeeded AND the gate row is gone,
        // both of which are already checked above; additionally, the
        // response reply should NOT contain "Config Change Approval
        // Request" (the two-layer approve prompt template).
        assert!(
            !result.reply.contains("Config Change Approval Request"),
            "Gate-approved update_setting must not emit the two-layer approval UI — reply: {}",
            result.reply
        );
    }

    // ── i18n Task 3: HITL Channel-B approval results render in display_language ──
    //
    // These exercise the REAL entry-path handlers (email/draft/mcp/config
    // approvals) under BOTH locales, asserting the owner-facing reply is sourced
    // from the catalog under the configured `display_language` — re-introducing a
    // `DEFAULT_LOCALE` pin or a hardcoded literal fails the test. They also pin
    // that approval LOGIC/identity is unchanged (the email rejection still reports
    // the recipient; the corruption guard still fires) and — critically — that an
    // arbitrary stored string carrying a literal `{}` (a subject line) survives the
    // real entry path uncorrupted (the §1 tf/tfn re-scan trap, end-to-end).

    /// Helper: orchestrator with display_language set to `locale`.
    async fn setup_locale_orch(locale: &str) -> crate::orchestrator::Orchestrator {
        let orch = setup_gate_test_orch().await;
        orch.settings
            .set("display_language", serde_json::json!(locale))
            .expect("set display_language");
        orch
    }
    #[tokio::test]
    async fn mcp_rejection_not_found_uses_display_language() {
        for locale in ["zh", "en"] {
            let orch = setup_locale_orch(locale).await;
            let res = orch
                .handle_mcp_rejection("mcp-absent", Some(1))
                .await
                .expect("handler ok");
            assert_eq!(
                res.reply,
                crate::i18n::tf("hitl.not_found_or_processed", locale, "mcp-absent"),
                "mcp not_found must come from catalog under {locale}"
            );
        }
    }

    #[tokio::test]
    async fn mcp_approval_result_uses_display_language_and_preserves_server_key() {
        for locale in ["zh", "en"] {
            let orch = setup_locale_orch(locale).await;
            let key = "mcp-approve-1";
            let payload = json!({
                "chat_id": 1,
                "server_key": "my-mcp-server",
                "created_at": chrono::Utc::now().to_rfc3339(),
            });
            seed_hitl(&orch, key, &payload).await;

            let res = orch
                .handle_mcp_approval(key, Some(1))
                .await
                .expect("handler ok");
            assert_eq!(
                res.reply,
                crate::i18n::tf("hitl.mcp_approved", locale, "my-mcp-server"),
                "mcp_approved must come from catalog under {locale}"
            );
            // Identity preserved: the server key is in the message.
            assert!(res.reply.contains("my-mcp-server"));
            // Logic preserved: the server was actually added to the approved CSV.
            let csv = orch
                .settings
                .get("mcp_approved_servers")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            assert!(
                csv.split(',').any(|s| s.trim() == "my-mcp-server"),
                "approve must persist server_key (logic unchanged): {csv}"
            );
        }
    }
    #[tokio::test]
    async fn config_approval_not_found_uses_display_language() {
        for locale in ["zh", "en"] {
            let orch = setup_locale_orch(locale).await;
            let res = orch
                .handle_config_approval("cfg-absent", Some(1))
                .await
                .expect("handler ok");
            assert_eq!(
                res.reply,
                crate::i18n::tf("hitl.config.not_found_or_processed", locale, "cfg-absent"),
                "config not_found must come from catalog under {locale}"
            );
        }
    }

    #[tokio::test]
    async fn gate_chat_mismatch_denial_uses_display_language() {
        for locale in ["zh", "en"] {
            let orch = setup_locale_orch(locale).await;
            let key = "gate-mismatch-1";
            let payload = json!({
                "chat_id": 100,
                "tool_name": "some_tool",
                "tool_args": "{}",
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
            });
            seed_hitl(&orch, key, &payload).await;

            // Approve from a DIFFERENT chat → chat-mismatch denial (Channel B).
            let res = orch
                .handle_gate_approval_scoped(
                    key,
                    crate::tool_gate::ApprovalScope::Once,
                    "did:test:owner",
                    Some(200),
                )
                .await
                .expect("handler ok");
            assert_eq!(
                res.reply,
                crate::i18n::tf("hitl.chat_mismatch", locale, key),
                "chat_mismatch denial must come from catalog under {locale}"
            );
        }
    }

    // ── S2 W4c (2026-07-08 audit): per-kind expiry through hitl::sweep_expired ──
    //
    // Proves the no-zombie-approvals contract end-to-end for the three
    // owner-visible approval kinds: a pending row whose `expires_at` is in the
    // past is TERMINAL after one sweep — gone from the store, and the kind's
    // entry handler answers the kind-appropriate "not found / already
    // processed" instead of executing a stale approval.

    #[tokio::test]
    async fn sweep_expired_terminates_mcp_gate_config_approvals_per_kind() {
        let orch = setup_gate_test_orch().await;
        let mem = orch.memory();
        let now = chrono::Utc::now();
        let past_expiry = now - chrono::Duration::hours(2);
        let created = now.to_rfc3339();

        // Payload shapes mirror what each proposer stores (chat_id + user_did
        // + created_at are the load_verify_clear_hitl invariants; the rest is
        // the kind's own execution payload).
        let seeds: [(&str, &str, Value); 3] = [
            (
                "exp-mcp-1",
                "mcp",
                json!({
                    "chat_id": 11,
                    "user_did": "did:test:owner",
                    "created_at": created,
                    "server_key": "expired-mcp",
                }),
            ),
            (
                "exp-gate-1",
                "gate",
                json!({
                    "chat_id": 11,
                    "user_did": "did:test:owner",
                    "created_at": created,
                    "tool_name": "manage_tasks",
                    "tool_args": "{}",
                }),
            ),
            (
                "exp-config-1",
                "config_change",
                json!({
                    "chat_id": 11,
                    "user_did": "did:test:owner",
                    "created_at": created,
                    "key": "sweep_test_setting",
                    "proposed_value": true,
                }),
            ),
        ];
        for (key, kind, payload) in &seeds {
            mem.store_hitl_approval(key, &payload.to_string(), kind, None, None, past_expiry)
                .await
                .unwrap();
            // Precondition: the pending row is loadable before the sweep.
            let stored = mem.load_skill_state("hitl", key).await.unwrap();
            assert!(
                stored.is_some_and(|s| !s.is_empty()),
                "{key}: precondition — pending approval stored"
            );
        }

        // One sweep at `now` deletes exactly the three expired rows.
        let swept = super::sweep_expired(mem, now).await.unwrap();
        assert_eq!(swept, 3, "sweep must delete all three expired approvals");

        // Terminal in the store…
        for (key, _, _) in &seeds {
            let after = mem.load_skill_state("hitl", key).await.unwrap();
            assert!(
                after.is_none() || after.as_deref() == Some(""),
                "{key}: expired approval must be terminal after sweep (was {after:?})"
            );
        }

        // …and terminal at each kind's entry handler: the kind-appropriate
        // not-found reply, never an execution of the swept approval.
        let mcp = orch
            .handle_mcp_approval("exp-mcp-1", Some(11))
            .await
            .expect("mcp handler ok");
        assert_eq!(
            mcp.reply,
            crate::i18n::tf(
                "hitl.not_found_or_processed",
                &orch_locale(&orch),
                "exp-mcp-1"
            ),
            "mcp approval after sweep must be the not-found reply"
        );

        let gate = orch
            .handle_gate_approval_scoped(
                "exp-gate-1",
                crate::tool_gate::ApprovalScope::Once,
                "did:test:owner",
                Some(11),
            )
            .await
            .expect("gate handler ok");
        assert_eq!(
            gate.reply,
            crate::i18n::tf(
                "hitl.gate.not_found_or_expired",
                &orch_locale(&orch),
                "exp-gate-1"
            ),
            "gate approval after sweep must be the not-found reply"
        );

        let config = orch
            .handle_config_approval("exp-config-1", Some(11))
            .await
            .expect("config handler ok");
        assert_eq!(
            config.reply,
            crate::i18n::tf(
                "hitl.config.not_found_or_processed",
                &orch_locale(&orch),
                "exp-config-1"
            ),
            "config approval after sweep must be the not-found reply"
        );

        // Idempotent: a second sweep with the same `now` deletes nothing.
        let swept_again = super::sweep_expired(mem, now).await.unwrap();
        assert_eq!(swept_again, 0, "second sweep must be a no-op");
    }

    fn orch_locale(orch: &crate::orchestrator::Orchestrator) -> String {
        orch.settings.display_language()
    }

    /// Rows bearing the non-expiring sentinel (what plain
    /// `store_skill_state("hitl", …)` rows get) must survive the sweep — only
    /// rows with a REAL past expiry are terminal.
    #[tokio::test]
    async fn sweep_expired_spares_sentinel_and_future_rows() {
        let orch = setup_gate_test_orch().await;
        let mem = orch.memory();
        let now = chrono::Utc::now();

        // Sentinel row (generic path).
        seed_hitl(
            &orch,
            "keep-sentinel-1",
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": now.to_rfc3339(),
                "tool_name": "manage_tasks",
                "tool_args": "{}",
            }),
        )
        .await;
        // Future-expiry row.
        mem.store_hitl_approval(
            "keep-future-1",
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": now.to_rfc3339(),
                "kind": "config_change",
                "topic": "t",
            })
            .to_string(),
            "config_change",
            None,
            None,
            now + chrono::Duration::hours(20),
        )
        .await
        .unwrap();

        assert_eq!(super::sweep_expired(mem, now).await.unwrap(), 0);
        for key in ["keep-sentinel-1", "keep-future-1"] {
            let kept = mem.load_skill_state("hitl", key).await.unwrap();
            assert!(
                kept.is_some_and(|s| !s.is_empty()),
                "{key}: unexpired approval must survive the sweep"
            );
        }
    }

    // ── S2 W4d (2026-07-08 audit): heartbeat-proposal entry handlers ────────

    /// Not-found is answered cleanly by both heartbeat entry points (no
    /// execution, no error).
    #[tokio::test]
    async fn heartbeat_proposal_approval_and_rejection_not_found_are_clean() {
        let orch = setup_gate_test_orch().await;

        let approve = orch
            .handle_heartbeat_proposal_approval("hb-absent-1", Some(11))
            .await
            .expect("approval handler ok");
        assert_eq!(
            approve.reply,
            "Heartbeat proposal `hb-absent-1` not found or already processed."
        );

        let reject = orch
            .handle_heartbeat_proposal_rejection("hb-absent-2", Some(11))
            .await
            .expect("rejection handler ok");
        assert_eq!(
            reject.reply,
            "Heartbeat proposal `hb-absent-2` not found (may have already been processed)."
        );
    }

    /// Approving a stored `config_change` heartbeat proposal APPLIES the
    /// staged change through the audit-logged `update_heartbeat_config` path
    /// (S2 W4e success path) and consumes the row (second approve → not
    /// found). The trailing LLM callback runs against the harness's dead LLM
    /// endpoint, so only the apply side-effect and row lifecycle are asserted.
    #[tokio::test]
    async fn heartbeat_config_change_approval_applies_staged_change() {
        let orch = setup_gate_test_orch().await;
        let before = orch.heartbeat_config();
        assert_ne!(
            before.frequency_minutes, 42,
            "precondition: test value must differ from the default"
        );

        let approval_id = "hb-config-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "kind": "config_change",
                "changes": { "frequency_minutes": 42 },
                "reason": "owner asked for a slower heartbeat",
            }),
        )
        .await;

        // The apply happens BEFORE the trailing LLM resume; the harness's dead
        // LLM endpoint may surface as Err from the resume, so the handler
        // Result is not asserted — the applied side effect is.
        let _ = orch
            .handle_heartbeat_proposal_approval(approval_id, Some(11))
            .await;

        assert_eq!(
            orch.heartbeat_config().frequency_minutes,
            42,
            "approval must apply the staged heartbeat_config change"
        );

        // Row consumed — the double-click answer is not-found.
        let again = orch
            .handle_heartbeat_proposal_approval(approval_id, Some(11))
            .await
            .expect("second approval ok");
        assert_eq!(
            again.reply,
            format!("Heartbeat proposal `{approval_id}` not found or already processed.")
        );
    }

    /// A stale-at-apply-time config proposal (fails re-validation) is
    /// DISCARDED, not applied — and the row is still consumed.
    #[tokio::test]
    async fn heartbeat_config_change_approval_discards_invalid_merge() {
        let orch = setup_gate_test_orch().await;
        let before = orch.heartbeat_config();

        let approval_id = "hb-config-bad-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "kind": "config_change",
                // frequency_minutes == 0 fails apply-time re-validation.
                "changes": { "frequency_minutes": 0 },
                "reason": "corrupted staged change",
            }),
        )
        .await;

        let res = orch
            .handle_heartbeat_proposal_approval(approval_id, Some(11))
            .await
            .expect("approval handler ok");
        assert!(
            res.reply.contains("failed re-validation"),
            "invalid staged change must be discarded, got: {}",
            res.reply
        );
        assert_eq!(
            orch.heartbeat_config().frequency_minutes,
            before.frequency_minutes,
            "discarded proposal must not change the live config"
        );
    }

    /// An unknown proposal kind falls through cleanly (row consumed, explicit
    /// unsupported-kind reply, nothing executed).
    #[tokio::test]
    async fn heartbeat_proposal_unknown_kind_falls_through_cleanly() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hb-weird-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "kind": "mystery_kind",
            }),
        )
        .await;

        let res = orch
            .handle_heartbeat_proposal_approval(approval_id, Some(11))
            .await
            .expect("approval handler ok");
        assert_eq!(
            res.reply,
            format!("Heartbeat proposal `{approval_id}` has unsupported kind `mystery_kind`.")
        );
    }

    /// Rejecting a stored heartbeat proposal discards it (row consumed; a
    /// follow-up approve finds nothing).
    #[tokio::test]
    async fn heartbeat_proposal_rejection_discards_stored_proposal() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "hb-reject-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "kind": "config_change",
                "topic": "t",
                "questions": ["q"],
            }),
        )
        .await;

        // Rejection clears the row FIRST, then resumes the (dead) LLM loop —
        // tolerate an Err from the resume; the cleared row is the contract.
        let _ = orch
            .handle_heartbeat_proposal_rejection(approval_id, Some(11))
            .await;

        let after = orch
            .memory()
            .load_skill_state("hitl", approval_id)
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(after.is_empty(), "rejection must consume the stored row");

        let approve_after = orch
            .handle_heartbeat_proposal_approval(approval_id, Some(11))
            .await
            .expect("post-reject approval ok");
        assert_eq!(
            approve_after.reply,
            format!("Heartbeat proposal `{approval_id}` not found or already processed.")
        );
    }

    // ── S2 W4e (2026-07-08 audit): config + menu approval SUCCESS paths ─────

    /// `handle_config_approval` SUCCESS: the staged generic setting is applied
    /// to the live RuntimeSettings and persisted; the row is consumed.
    #[tokio::test]
    async fn call_validation_chat_approval_cannot_bypass_settings_probe() {
        let orch = setup_gate_test_orch().await;
        let before = orch.settings.get("llm_model");
        let result = orch
            .apply_approved_config(
                "validation-required",
                "llm_model",
                &json!("unverified-model"),
            )
            .await;
        assert!(matches!(result, ConfigApplyResult::ApplyFailed { .. }));
        assert_eq!(orch.settings.get("llm_model"), before);
        assert!(
            !orch
                .memory
                .load_all_settings()
                .await
                .unwrap()
                .contains_key("llm_model")
        );
    }

    #[tokio::test]
    async fn settings_persistence_hitl_failure_keeps_cache_and_failed_receipt() {
        use crate::action_ledger::{ActionLedgerStore, ActionStatus};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hitl-settings.db");
        let memory = crate::memory::MemoryHandle::spawn(&path).unwrap();
        memory
            .store_setting("paid_api_fallback_enabled", json!(true))
            .await
            .unwrap();
        let settings = std::sync::Arc::new(crate::settings::RuntimeSettings::new());
        settings.load_from_map(memory.load_all_settings().await.unwrap());
        let orch = crate::test_tool_helpers::test_orchestrator(memory, settings);
        let conn = rusqlite::Connection::open(&path).unwrap();
        crate::action_ledger::record_runtime_setting_proposed(
            &conn,
            "synthetic-failed-disable",
            "paid_api_fallback_enabled",
            &json!(true),
            &json!(false),
            "did:test:owner",
            None,
            None,
            true,
        )
        .unwrap();
        conn.execute_batch("CREATE TRIGGER reject_setting BEFORE INSERT ON user_settings BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
        let result = orch
            .apply_approved_config(
                "synthetic-failed-disable",
                "paid_api_fallback_enabled",
                &json!(false),
            )
            .await;
        assert!(matches!(result, ConfigApplyResult::ApplyFailed { .. }));
        assert_eq!(
            orch.settings.get("paid_api_fallback_enabled"),
            Some(json!(true))
        );
        assert_eq!(
            orch.memory.load_all_settings().await.unwrap()["paid_api_fallback_enabled"],
            json!(true)
        );
        let entry = ActionLedgerStore::new(&conn)
            .get_by_action_id("runtime-setting:synthetic-failed-disable")
            .unwrap()
            .unwrap();
        assert_eq!(entry.status, ActionStatus::Failed);
        assert!(entry.external_receipt.is_none());
        orch.memory.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn config_approval_success_applies_staged_setting() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "cfg-success-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                // A REGISTERED runtime setting: RuntimeSettings.set rejects
                // unknown keys, so the staged key must be a real one.
                "key": "display_language",
                "proposed_value": "en",
            }),
        )
        .await;

        // apply_approved_config runs BEFORE the trailing LLM resume; tolerate
        // an Err from the dead-LLM resume and assert the applied setting.
        let _ = orch.handle_config_approval(approval_id, Some(11)).await;

        assert_eq!(
            orch.settings.get("display_language"),
            Some(serde_json::json!("en")),
            "approved config change must be applied to live settings"
        );

        // Double-click safety: the second approve is a clean not-found.
        let again = orch
            .handle_config_approval(approval_id, Some(11))
            .await
            .expect("second config approval ok");
        assert_eq!(
            again.reply,
            crate::i18n::tf(
                "hitl.config.not_found_or_processed",
                &orch_locale(&orch),
                approval_id
            )
        );
    }

    /// `handle_menu_approval` SUCCESS: with no Telegram bot wired the API
    /// apply is a no-op Ok, so the approved custom commands are persisted to
    /// `skill_state[system][custom_bot_commands]` and the row is consumed.
    #[tokio::test]
    async fn menu_approval_success_persists_custom_commands() {
        let orch = setup_gate_test_orch().await;
        let approval_id = "menu-success-1";
        seed_hitl(
            &orch,
            approval_id,
            &json!({
                "chat_id": 11,
                "user_did": "did:test:owner",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "proposed_custom": [
                    { "command": "weather", "description": "Daily weather brief" }
                ],
            }),
        )
        .await;

        // The Telegram apply is a no-op Ok (no bot wired) and
        // save_custom_commands persists BEFORE the trailing LLM resume;
        // tolerate an Err from the dead-LLM resume.
        let _ = orch.handle_menu_approval(approval_id, Some(11)).await;

        let saved = orch
            .memory()
            .load_skill_state("system", "custom_bot_commands")
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            saved.contains("weather"),
            "approved custom command must be persisted, got: {saved}"
        );

        let again = orch
            .handle_menu_approval(approval_id, Some(11))
            .await
            .expect("second menu approval ok");
        assert_eq!(
            again.reply,
            format!("Menu proposal `{approval_id}` not found or already processed.")
        );
    }
}
