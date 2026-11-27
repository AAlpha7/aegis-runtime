//! True Heartbeat loop, snapshot compilation, memo archival, and daily
//! summary generation for the Chronos engine.
//!
//! Extracted from `chronos.rs` to keep that file under the 2,000-line limit.
//! All methods are `impl ChronosEngine` — same pattern as `orchestrator_tools_*.rs`.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Timelike;

use crate::chronos::{
    ChronosEngine, DAILY_SUMMARY_HOUR, DISABLED_CHECK_INTERVAL_SECS, HeartbeatHandler,
    SituationSnapshot, TrueHeartbeatConfig,
};

// Ordinary scheduled backlog should wake the LLM only when it is truly near.
// Broader 24h windows caused every heartbeat to re-review stable reminders and
// then return noop. Urgent/high priority items still wake immediately.
const HEARTBEAT_ACTIONABLE_BACKLOG_WINDOW_SECS: i64 = 2 * 60 * 60;
const HEARTBEAT_MISSED_SCHEDULE_GRACE_SECS: i64 = 10 * 60;

fn heartbeat_error_is_recent(
    entry: &crate::memory::HeartbeatAuditEntry,
    now: chrono::DateTime<chrono::Utc>,
    fresh_window: chrono::Duration,
) -> bool {
    let severity = entry.severity.to_ascii_lowercase();
    if severity != "error" && severity != "critical" {
        return false;
    }

    let parsed = chrono::NaiveDateTime::parse_from_str(&entry.timestamp, "%Y-%m-%d %H:%M:%S")
        .map(|ts| ts.and_utc());
    match parsed {
        Ok(ts) => now.signed_duration_since(ts) <= fresh_window,
        Err(_) => true,
    }
}

fn heartbeat_daily_summary_tool_counts(
    recent: &[&crate::memory::HeartbeatAuditEntry],
) -> (usize, usize, Vec<String>) {
    use std::collections::HashMap;

    let mut meaningful_count = 0usize;
    let mut bookkeeping_count = 0usize;
    let mut tool_freq: HashMap<&str, usize> = HashMap::new();

    for audit in recent {
        for tool in &audit.tools_called {
            if crate::heartbeat_config::is_heartbeat_bookkeeping_tool(tool) {
                bookkeeping_count += 1;
                continue;
            }
            meaningful_count += 1;
            *tool_freq.entry(tool.as_str()).or_default() += 1;
        }
    }

    let mut freq_vec: Vec<_> = tool_freq.into_iter().collect();
    freq_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let tool_summary = freq_vec
        .iter()
        .take(5)
        .map(|(name, count)| format!("{} (\u{00d7}{})", name, count))
        .collect();

    (meaningful_count, bookkeeping_count, tool_summary)
}

fn heartbeat_priority_is_actionable(priority: &str) -> bool {
    matches!(priority.to_ascii_lowercase().as_str(), "urgent" | "high")
}

/// Resolve a "retention age in days" setting into a unix-second cutoff, safely.
///
/// The task archive + DESTRUCTIVE retention-delete sweeps both turn a configurable
/// day-count into `now - days*86400`. `RuntimeSettings` accepts arbitrary JSON
/// numbers for a registered key, so a hostile/typo'd value (e.g. 10^14) would
/// overflow the naive `now - days*86400` and produce a far-future cutoff that
/// makes EVERY archived row eligible for deletion (or panic the heartbeat task in
/// a checked build). We therefore: drop non-positive values, clamp to a sane
/// maximum (~100 years — far beyond any real policy, small enough that the
/// arithmetic can never overflow), and use saturating arithmetic as belt-and-
/// suspenders. A missing/invalid value falls back to `default_days`.
fn retention_cutoff_unix(raw_days: Option<i64>, default_days: i64, now_unix: i64) -> i64 {
    // 100 years. days*86400 ≈ 3.15e12 ≪ i64::MAX (9.2e18), so no overflow.
    const MAX_RETENTION_DAYS: i64 = 36_500;
    let days = raw_days
        .filter(|d| *d > 0)
        .map(|d| d.min(MAX_RETENTION_DAYS))
        .unwrap_or(default_days);
    now_unix.saturating_sub(days.saturating_mul(86_400))
}

fn heartbeat_unix_is_imminent(
    ts: i64,
    now: chrono::DateTime<chrono::Utc>,
    window: chrono::Duration,
) -> bool {
    ts > now.timestamp() && ts <= (now + window).timestamp()
}

fn heartbeat_unix_is_missed(
    ts: i64,
    now: chrono::DateTime<chrono::Utc>,
    grace: chrono::Duration,
) -> bool {
    ts > 0 && ts <= (now - grace).timestamp()
}

fn heartbeat_due_date_is_imminent(
    due_date: &str,
    now: chrono::DateTime<chrono::Utc>,
    window: chrono::Duration,
) -> bool {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(due_date) {
        return dt.with_timezone(&chrono::Utc) <= now + window;
    }

    // Date-only tasks are not precise enough to wake every heartbeat for the
    // whole local day. They still wake while freshly changed via
    // `heartbeat_task_changed_recently`.
    false
}

fn heartbeat_parse_timestamp_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|dt| dt.and_utc())
                .ok()
        })
}

fn heartbeat_audit_entry_used_llm(entry: &crate::memory::HeartbeatAuditEntry) -> bool {
    entry.tokens_used > 0
        || entry.rounds_used > 0
        || entry
            .prompt_hash
            .as_deref()
            .is_some_and(|hash| !hash.trim().is_empty())
}

fn heartbeat_count_llm_wakeups_in_window(
    entries: &[crate::memory::HeartbeatAuditEntry],
    now: chrono::DateTime<chrono::Utc>,
    window: chrono::Duration,
) -> u32 {
    entries
        .iter()
        .filter(|entry| entry.skill_name == "__heartbeat__")
        .filter(|entry| heartbeat_audit_entry_used_llm(entry))
        .filter_map(|entry| heartbeat_parse_timestamp_utc(&entry.timestamp))
        .filter(|ts| *ts <= now && now.signed_duration_since(*ts) <= window)
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn heartbeat_task_changed_recently(
    task: &crate::memory::Task,
    now: chrono::DateTime<chrono::Utc>,
    changed_window: chrono::Duration,
) -> bool {
    heartbeat_parse_timestamp_utc(&task.updated_at)
        .or_else(|| heartbeat_parse_timestamp_utc(&task.created_at))
        .map(|ts| now.signed_duration_since(ts) <= changed_window)
        .unwrap_or(true)
}

fn heartbeat_task_is_actionable(
    task: &crate::memory::Task,
    now: chrono::DateTime<chrono::Utc>,
    changed_window: chrono::Duration,
) -> bool {
    let is_scheduler_owned_time_task = task.trigger_type.as_deref() == Some("time")
        && matches!(task.execution_mode.as_str(), "notify" | "llm_callback")
        && !heartbeat_task_requests_coordination(task);

    if let Some(ts) = task.trigger_at_unix {
        if is_scheduler_owned_time_task {
            if task.fired_at.is_none()
                && heartbeat_unix_is_missed(
                    ts,
                    now,
                    chrono::Duration::seconds(HEARTBEAT_MISSED_SCHEDULE_GRACE_SECS),
                )
            {
                return true;
            }
        } else if heartbeat_unix_is_imminent(
            ts,
            now,
            chrono::Duration::seconds(HEARTBEAT_ACTIONABLE_BACKLOG_WINDOW_SECS),
        ) {
            return true;
        }
    }

    if task
        .due_date
        .as_deref()
        .map(|d| {
            heartbeat_due_date_is_imminent(
                d,
                now,
                chrono::Duration::seconds(HEARTBEAT_ACTIONABLE_BACKLOG_WINDOW_SECS),
            )
        })
        .unwrap_or(false)
    {
        return true;
    }

    heartbeat_priority_is_actionable(&task.priority)
        && heartbeat_task_changed_recently(task, now, changed_window)
}

fn heartbeat_task_requests_coordination(task: &crate::memory::Task) -> bool {
    if matches!(
        task.execution_mode.as_str(),
        "heartbeat_review" | "coordination" | "coordination_review"
    ) {
        return true;
    }

    task.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "heartbeat_review" | "coordination_needed" | "needs_coordination"
        )
    })
}

/// Format the heartbeat-memo archive date in the user's timezone.
///
/// This date string is embedded in the human-readable archive fact
/// (`[心跳便签归档 <date>]`) that the owner sees, so it must reflect the
/// owner's local day — not UTC. Display-only: the wire/storage timestamp of
/// the archived L2 fact is unaffected.
fn format_memo_archive_date(now: chrono::DateTime<chrono::Utc>, user_tz: chrono_tz::Tz) -> String {
    now.with_timezone(&user_tz).format("%Y-%m-%d").to_string()
}

fn heartbeat_snapshot_has_actionable_backlog(
    snapshot: &SituationSnapshot,
    now: chrono::DateTime<chrono::Utc>,
    fresh_changed_window: chrono::Duration,
) -> bool {
    snapshot
        .pending_tasks
        .iter()
        .any(|task| heartbeat_task_is_actionable(task, now, fresh_changed_window))
        || snapshot
            .recent_probe_results
            .iter()
            .any(|p| heartbeat_error_is_recent(p, now, fresh_changed_window))
}

fn heartbeat_tools_did_real_work(tools_called: &[String]) -> bool {
    tools_called
        .iter()
        .any(|tool| crate::heartbeat_config::is_meaningful_heartbeat_tool(tool))
}

impl ChronosEngine {
    // ── True Heartbeat Config ────────────────────────────────────────────

    /// Get the current true heartbeat config.
    pub fn get_true_heartbeat_config(&self) -> TrueHeartbeatConfig {
        self.true_heartbeat_config_ref().read().clone()
    }

    /// Set the true heartbeat config.
    pub fn set_true_heartbeat_config(&self, config: TrueHeartbeatConfig) {
        *self.true_heartbeat_config_ref().write() = config;
    }

    /// Run the two task-lifecycle sweeps once, respecting their throttles:
    /// auto-archive long-finished tasks (~6h) and the DESTRUCTIVE retention-delete
    /// of tasks archived past the configured age (~24h). Cheap bounded SQL; safe to
    /// call frequently — the `last_*` throttle args gate the actual work. `None`
    /// throttle state = "never run yet", so both fire on the first call.
    async fn run_task_lifecycle_sweeps(
        &self,
        last_task_archive_at: &mut Option<chrono::DateTime<chrono::Utc>>,
        last_task_retention_at: &mut Option<chrono::DateTime<chrono::Utc>>,
    ) {
        let Some(mem) = self.memory_ref() else {
            return;
        };
        let now = chrono::Utc::now();
        // Archive sweep — throttled to ~6h between runs.
        let archive_due = last_task_archive_at
            .map(|t| now.signed_duration_since(t).num_hours() >= 6)
            .unwrap_or(true);
        if archive_due {
            *last_task_archive_at = Some(now);
            // Configurable threshold; default 30 days. Non-positive, missing, or
            // absurdly-large values are clamped by retention_cutoff_unix.
            let raw_archive_days = self
                .settings_ref()
                .get("task_archive_after_days")
                .and_then(|v| v.as_i64());
            let archive_days = raw_archive_days
                .filter(|d| *d > 0)
                .map(|d| d.min(36_500))
                .unwrap_or(30);
            let cutoff_unix = retention_cutoff_unix(raw_archive_days, 30, now.timestamp());
            match mem.archive_old_intents(cutoff_unix).await {
                Ok(items) if !items.is_empty() => {
                    tracing::info!(
                        archived = items.len(),
                        archive_after_days = archive_days,
                        "[TaskLifecycle] Task archive sweep moved finished tasks to 'archived'"
                    );
                }
                Ok(_) => {
                    // Log even at 0 so the sweep cadence is auditable and a
                    // post-deploy check can verify the sweep is wired (the marker
                    // appears on every run, not only when it moves rows).
                    tracing::info!(
                        archived = 0,
                        archive_after_days = archive_days,
                        "[TaskLifecycle] Task archive sweep ran (nothing to archive)"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[TaskLifecycle] Task archive sweep failed");
                }
            }
        }
        // Retention delete sweep — DESTRUCTIVE, throttled to ~24h.
        let retention_due = last_task_retention_at
            .map(|t| now.signed_duration_since(t).num_hours() >= 24)
            .unwrap_or(true);
        if retention_due {
            *last_task_retention_at = Some(now);
            // Configurable retention age; default 180 days (~6 months). Non-positive,
            // missing, or absurdly-large values are clamped by retention_cutoff_unix
            // — NEVER delete with a 0/negative threshold, never overflow into a
            // far-future cutoff that would make every archived row eligible.
            let raw_delete_days = self
                .settings_ref()
                .get("task_delete_archived_after_days")
                .and_then(|v| v.as_i64());
            let delete_days = raw_delete_days
                .filter(|d| *d > 0)
                .map(|d| d.min(36_500))
                .unwrap_or(180);
            let cutoff_unix = retention_cutoff_unix(raw_delete_days, 180, now.timestamp());
            // Bound the batch so a backlog can't trigger one giant delete.
            const RETENTION_BATCH_LIMIT: i64 = 500;
            match mem
                .delete_old_archived_tasks(cutoff_unix, RETENTION_BATCH_LIMIT)
                .await
            {
                Ok((0, _, _)) => {
                    // Always log the destructive sweep ran, even at 0, so its
                    // cadence is auditable.
                    tracing::info!(
                        deleted = 0,
                        delete_after_days = delete_days,
                        "[TaskLifecycle] Archived-task retention sweep ran (nothing past retention)"
                    );
                }
                Ok((deleted, min_id, max_id)) => {
                    tracing::info!(
                        deleted,
                        min_id = ?min_id,
                        max_id = ?max_id,
                        delete_after_days = delete_days,
                        batch_limit = RETENTION_BATCH_LIMIT,
                        "[TaskLifecycle] Archived-task retention sweep DELETED old archived tasks"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[TaskLifecycle] Archived-task retention sweep failed");
                }
            }
        }
    }

    /// Spawn the always-on task-lifecycle maintenance loop. This is spawned
    /// UNCONDITIONALLY at startup — independent of `true_heartbeat.enabled` —
    /// because in production the true-heartbeat is disabled, yet the archive +
    /// retention sweeps must still run. (They were previously wired inside the
    /// true-heartbeat loop, which is never even spawned when the heartbeat is
    /// disabled, leaving the sweeps dead in production.) The internal throttles
    /// keep the actual work at ~6h (archive) / ~24h (retention-delete); the first
    /// iteration runs immediately after spawn so the wiring is verifiable right
    /// after deploy.
    pub fn start_task_lifecycle_maintenance(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            const CHECK_INTERVAL_SECS: u64 = 30 * 60;
            let mut last_task_archive_at: Option<chrono::DateTime<chrono::Utc>> = None;
            let mut last_task_retention_at: Option<chrono::DateTime<chrono::Utc>> = None;
            tracing::info!(
                "[TaskLifecycle] Maintenance task started (archive ~6h, retention-delete ~24h)"
            );
            loop {
                engine
                    .run_task_lifecycle_sweeps(
                        &mut last_task_archive_at,
                        &mut last_task_retention_at,
                    )
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(CHECK_INTERVAL_SECS)).await;
            }
        })
    }

    /// Start the True Heartbeat background loop.
    ///
    /// Spawns a tokio task that wakes up every `interval_minutes` and:
    /// 1. Checks `llm_busy` -- if the LLM is busy with a user message, skip this cycle
    /// 2. Checks quiet hours -- if in quiet hours, skip
    /// 3. Checks daily budget -- if exhausted, skip
    /// 4. Compiles a [`SituationSnapshot`] from memory
    /// 5. Dispatches to the handler's `on_true_heartbeat()`
    /// 6. Delivers notifications if the LLM took any actions
    ///
    /// Returns a `JoinHandle` that can be used to abort the loop.
    pub fn start_true_heartbeat(
        self: &Arc<Self>,
        handler: Arc<dyn HeartbeatHandler + Send + Sync>,
        llm_busy: Arc<std::sync::atomic::AtomicBool>,
        chat_id: i64,
        progress_tx: Option<crate::orchestrator::ProgressSender>,
    ) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);

        let user_did = format!("did:aegis:telegram:{}", chat_id);

        tokio::spawn(async move {
            // Track whether we already archived the memo during this quiet period,
            // so we don't archive repeatedly every 15 minutes while in quiet hours.
            let mut archived_this_quiet_period = false;
            // Track whether the daily summary has been sent today (prevents sending twice).
            // Recover from DB so restarts within the same 9 AM window don't re-send.
            let mut daily_summary_sent_date: Option<chrono::NaiveDate> =
                if let Some(mem) = engine.memory_ref() {
                    mem.load_all_settings()
                        .await
                        .ok()
                        .and_then(|s| s.get("_daily_summary_sent_date").cloned())
                        .and_then(|v| v.as_str().map(String::from))
                        .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
                } else {
                    None
                };
            // Adaptive interval: count consecutive NoAction outcomes to extend sleep
            let mut consecutive_no_action: u32 = 0;
            // Track whether we already notified about invalid timezone (once per session)
            let mut tz_fallback_notified = false;

            loop {
                let config = engine.get_true_heartbeat_config();
                if !config.enabled {
                    tracing::debug!("[TrueHeartbeat] Disabled — sleeping before re-check");
                    tokio::time::sleep(std::time::Duration::from_secs(
                        DISABLED_CHECK_INTERVAL_SECS,
                    ))
                    .await;
                    continue;
                }

                // Adaptive interval: extend sleep when consecutive cycles found nothing
                let base_mins = config.interval_minutes.max(1) as u64;
                let effective_mins = match consecutive_no_action {
                    0 => base_mins,           // Normal interval
                    1 => (base_mins * 3) / 2, // 1.5x after 1 idle cycle
                    _ => base_mins * 2,       // 2x after 2+ idle cycles (capped)
                };
                let interval = std::time::Duration::from_secs(effective_mins * 60);
                if consecutive_no_action > 0 {
                    tracing::info!(
                        base_mins,
                        effective_mins,
                        consecutive_no_action,
                        "[TrueHeartbeat] Adaptive interval — extending sleep (idle)"
                    );
                }
                tokio::time::sleep(interval).await;

                // Re-read config in case it changed during sleep
                let config = engine.get_true_heartbeat_config();
                if !config.enabled {
                    tracing::info!("[TrueHeartbeat] Disabled after sleep — skipping cycle");
                    continue;
                }

                // Step 9: Sweep expired HITL approvals once per tick. Cheap
                // DB delete; rows with the non-expiring sentinel are skipped.
                // Runs before gates so stale proposals don't linger even when
                // the tick is going to skip for quiet-hours or precheck.
                if let Some(mem) = engine.memory_ref() {
                    match crate::hitl::sweep_expired(mem, chrono::Utc::now()).await {
                        Ok(n) if n > 0 => {
                            tracing::debug!(
                                swept = n,
                                "[TrueHeartbeat] HITL sweep removed expired proposals"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "[TrueHeartbeat] HITL sweep failed");
                        }
                    }
                }

                // NOTE: task-lifecycle maintenance (auto-archive + retention-delete)
                // is NOT run here. It lives in the always-on
                // start_task_lifecycle_maintenance() task instead, because the
                // true-heartbeat is disabled in production (so this loop never runs)
                // yet the sweeps must still fire. See that method.

                // G7: Run upstream health probes (lightweight, no LLM needed)
                handler.on_health_probe(chat_id).await;

                // Gate 1: LLM busy?
                if llm_busy
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_err()
                {
                    tracing::info!("[TrueHeartbeat] LLM busy — skipping cycle");
                    continue;
                }
                // BusyGuard: ensure we reset llm_busy even on panic
                struct BusyReset(Arc<std::sync::atomic::AtomicBool>);
                impl Drop for BusyReset {
                    fn drop(&mut self) {
                        self.0.store(false, std::sync::atomic::Ordering::Release);
                    }
                }
                let _guard = BusyReset(Arc::clone(&llm_busy));

                // Gate 2: Quiet hours? (uses HeartbeatConfig via RuntimeSettings)
                let user_tz = crate::settings::parse_user_tz(engine.settings_ref());
                let hb_cfg = engine
                    .settings_ref()
                    .get("heartbeat_config")
                    .and_then(|v| crate::heartbeat_config::HeartbeatConfig::from_json(&v))
                    .unwrap_or_default();
                if hb_cfg.quiet_hours.in_quiet_hours(user_tz) {
                    // Archive scratchpad to L2 on first quiet-hours entry
                    if !archived_this_quiet_period {
                        tracing::info!(
                            "[TrueHeartbeat] Quiet hours started — archiving scratchpad to L2"
                        );
                        match engine.archive_heartbeat_memo(&user_did).await {
                            Ok(Some(content)) => {
                                tracing::info!(
                                    len = content.len(),
                                    "[TrueHeartbeat] Scratchpad archived successfully"
                                );
                            }
                            Ok(None) => {
                                tracing::info!("[TrueHeartbeat] No scratchpad content to archive");
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "[TrueHeartbeat] Failed to archive scratchpad");
                            }
                        }
                        archived_this_quiet_period = true;
                    }
                    tracing::info!(
                        quiet_start = %hb_cfg.quiet_hours.start,
                        quiet_end = %hb_cfg.quiet_hours.end,
                        tz = %user_tz,
                        "[TrueHeartbeat] In quiet hours — skipping"
                    );
                    engine
                        .write_heartbeat_tick_audit(
                            chat_id,
                            &user_did,
                            crate::heartbeat_config::HeartbeatTickOutcome::SkippedPrecheck,
                            Some(crate::heartbeat_config::HeartbeatSkipReason::QuietHours),
                            None,
                            None,
                            "info",
                        )
                        .await;
                    continue;
                }

                // Reset the flag when we leave quiet hours
                archived_this_quiet_period = false;

                // Gate 3: Budget exhausted?
                engine.maybe_reset_daily_counter();
                let auto_cfg = config
                    .effective_autonomous_config(&engine.global_autonomous_config_ref().read());
                let used = engine.daily_tokens_used_raw();
                if used >= auto_cfg.daily_token_budget {
                    tracing::info!(
                        used,
                        budget = auto_cfg.daily_token_budget,
                        "[TrueHeartbeat] Daily budget exhausted — skipping"
                    );
                    engine
                        .write_heartbeat_tick_audit(
                            chat_id,
                            &user_did,
                            crate::heartbeat_config::HeartbeatTickOutcome::BudgetExhausted,
                            None,
                            Some(format!(
                                "used={} budget={}",
                                used, auto_cfg.daily_token_budget
                            )),
                            None,
                            "warn",
                        )
                        .await;
                    continue;
                }

                let fresh_changed_window =
                    chrono::Duration::minutes((config.interval_minutes.max(1) * 2) as i64);

                // Gate 4: Zero-cost precheck — skip LLM when nothing actionable
                if let Some(skip_reason) = engine
                    .heartbeat_precheck(chat_id, fresh_changed_window)
                    .await
                {
                    tracing::debug!(
                        reason = %skip_reason,
                        consecutive_no_action,
                        "[TrueHeartbeat] Precheck: nothing actionable — skipping LLM"
                    );
                    engine
                        .write_heartbeat_tick_audit(
                            chat_id,
                            &user_did,
                            crate::heartbeat_config::HeartbeatTickOutcome::SkippedPrecheck,
                            Some(crate::heartbeat_config::HeartbeatSkipReason::NoNewContext),
                            Some(skip_reason),
                            None,
                            "info",
                        )
                        .await;
                    consecutive_no_action = consecutive_no_action.saturating_add(1);
                    continue;
                }

                // Compile snapshot
                tracing::info!("[TrueHeartbeat] Compiling system snapshot...");
                let snapshot = match engine.compile_snapshot(chat_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "[TrueHeartbeat] Failed to compile snapshot");
                        let err_text = format!("snapshot compile error: {e}");
                        engine
                            .write_heartbeat_tick_audit(
                                chat_id,
                                &user_did,
                                crate::heartbeat_config::HeartbeatTickOutcome::Error,
                                None,
                                None,
                                Some(err_text.as_str()),
                                "error",
                            )
                            .await;
                        continue;
                    }
                };

                // Gate 5: Post-snapshot zero-cost check — if system is fully healthy
                // and nothing is pending, skip the LLM call entirely.
                // This catches ~80% of no-op heartbeats that pass the simpler precheck.
                let snapshot_now = chrono::Utc::now();
                let fresh_error_window = fresh_changed_window;
                let snapshot_has_actionable_backlog = heartbeat_snapshot_has_actionable_backlog(
                    &snapshot,
                    snapshot_now,
                    fresh_error_window,
                );
                let has_recent_changed_context = engine
                    .heartbeat_has_recent_changed_backlog_context(snapshot_now - fresh_error_window)
                    .await;
                if !snapshot_has_actionable_backlog && !has_recent_changed_context {
                    tracing::debug!(
                        consecutive_no_action,
                        "[TrueHeartbeat] Snapshot has no actionable backlog — skipping LLM"
                    );
                    engine
                        .write_heartbeat_tick_audit(
                            chat_id,
                            &user_did,
                            crate::heartbeat_config::HeartbeatTickOutcome::SkippedPrecheck,
                            Some(crate::heartbeat_config::HeartbeatSkipReason::NoNewContext),
                            Some("no urgent, imminent, or changed heartbeat context".into()),
                            None,
                            "info",
                        )
                        .await;
                    consecutive_no_action = consecutive_no_action.saturating_add(1);
                    continue;
                }
                if let Some(last_noop_at) = engine.heartbeat_last_noop_timestamp().await
                    && last_noop_at < snapshot_now
                {
                    let since_last_noop = snapshot_now.signed_duration_since(last_noop_at);
                    let actionable_since_last_noop = heartbeat_snapshot_has_actionable_backlog(
                        &snapshot,
                        snapshot_now,
                        since_last_noop,
                    );
                    let changed_since_last_noop = engine
                        .heartbeat_has_recent_changed_backlog_context(last_noop_at)
                        .await;
                    if !actionable_since_last_noop && !changed_since_last_noop {
                        tracing::debug!(
                            %last_noop_at,
                            consecutive_no_action,
                            "[TrueHeartbeat] Previous noop still current - skipping LLM"
                        );
                        engine
                            .write_heartbeat_tick_audit(
                                chat_id,
                                &user_did,
                                crate::heartbeat_config::HeartbeatTickOutcome::SkippedPrecheck,
                                Some(crate::heartbeat_config::HeartbeatSkipReason::NoNewContext),
                                Some(format!(
                                    "previous heartbeat noop still current; no changed context since {last_noop_at}"
                                )),
                                None,
                                "info",
                            )
                            .await;
                        consecutive_no_action = consecutive_no_action.saturating_add(1);
                        continue;
                    }
                }

                // Controlled-thaw runtime gate: when the post-P0 autonomy
                // switch is explicitly enabled, enforce the configured hourly
                // LLM wakeup cap before dispatching to the heartbeat handler.
                // Default-off config returns None, preserving existing P0
                // cleanup behavior.
                if let Some(skip_reason) = engine
                    .controlled_thaw_hourly_skip_reason(&hb_cfg, chrono::Utc::now())
                    .await
                {
                    tracing::info!(
                        reason = %skip_reason,
                        "[TrueHeartbeat] controlled_thaw budget gate — skipping LLM"
                    );
                    engine
                        .write_heartbeat_tick_audit(
                            chat_id,
                            &user_did,
                            crate::heartbeat_config::HeartbeatTickOutcome::SkippedPrecheck,
                            Some(crate::heartbeat_config::HeartbeatSkipReason::Backpressure),
                            Some(skip_reason),
                            None,
                            "info",
                        )
                        .await;
                    consecutive_no_action = consecutive_no_action.saturating_add(1);
                    continue;
                }

                // Dispatch to handler
                tracing::info!("[TrueHeartbeat] Dispatching to handler...");
                match handler.on_true_heartbeat(&snapshot, &auto_cfg).await {
                    Ok(outcome) => {
                        let action_count = outcome.actions_taken.len();
                        let tool_count = outcome.tools_called.len();
                        // Adaptive interval: track consecutive idle cycles
                        let did_real_work = heartbeat_tools_did_real_work(&outcome.tools_called);
                        if did_real_work {
                            consecutive_no_action = 0; // Reset on real activity
                        } else {
                            consecutive_no_action = consecutive_no_action.saturating_add(1);
                        }
                        tracing::info!(
                            actions = action_count,
                            tools = tool_count,
                            fell_through = outcome.fell_through_to_alert,
                            consecutive_no_action,
                            "[TrueHeartbeat] Cycle complete"
                        );
                        if outcome.fell_through_to_alert {
                            tracing::debug!("[TrueHeartbeat] LLM decided no action needed");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "[TrueHeartbeat] Handler error");
                        let err_text = format!("heartbeat handler error: {e}");
                        engine
                            .write_heartbeat_tick_audit(
                                chat_id,
                                &user_did,
                                crate::heartbeat_config::HeartbeatTickOutcome::Error,
                                None,
                                None,
                                Some(err_text.as_str()),
                                "error",
                            )
                            .await;
                    }
                }

                // -- Daily summary check --
                // After each heartbeat cycle, check if it's around 9:00 AM user timezone
                // and we haven't sent today's summary yet.
                if let Some(ref tx) = progress_tx {
                    let tz_fallback = crate::settings::user_tz_default();
                    let tz_name = engine
                        .settings_ref()
                        .get("user_timezone")
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| tz_fallback.clone());
                    let (tz, tz_fallback_used) = match tz_name.parse::<chrono_tz::Tz>() {
                        Ok(parsed) => (parsed, false),
                        Err(_) => {
                            tracing::warn!(
                                tz = %tz_name,
                                fallback = %tz_fallback,
                                "[TrueHeartbeat] Failed to parse timezone, falling back to the host default — \
                                 user should update via /settings timezone"
                            );
                            (crate::settings::parse_tz_str(&tz_fallback), true)
                        }
                    };
                    // Notify user once per session about invalid timezone
                    if tz_fallback_used && !tz_fallback_notified {
                        tz_fallback_notified = true;
                        if let Some(ref tx) = progress_tx
                            && let Err(e) = tx.send(crate::progress_router::ProgressEvent::new(
                                chat_id,
                                crate::progress_router::ProgressPayload::OwnerNotice {
                                    text: format!(
                                        "⚠️ 时区 '{}' 无法识别，已临时使用 {}。请通过 /settings 更新时区设置。",
                                        tz_name, tz_fallback
                                    ),
                                    kind: crate::progress_router::OwnerNoticeKind::System,
                                },
                            )).await {
                                tracing::warn!(error = %e, "[TrueHeartbeat] Failed to send timezone fallback notification");
                            }
                    }
                    let user_now = chrono::Utc::now().with_timezone(&tz);
                    let today = user_now.date_naive();
                    let hour = user_now.hour();

                    // Reload daily_summary_sent_date from DB each cycle to prevent
                    // duplicates across concurrent heartbeat cycles and process restarts.
                    // The startup load (line ~59) seeds the cache; this keeps it fresh.
                    if let Some(mem) = engine.memory_ref()
                        && let Ok(settings) = mem.load_all_settings().await
                    {
                        daily_summary_sent_date = settings
                            .get("_daily_summary_sent_date")
                            .and_then(|v| v.as_str())
                            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
                    }

                    let already_sent = daily_summary_sent_date.map(|d| d == today).unwrap_or(false);

                    // Send between 9:00-9:59 AM user timezone, once per day
                    if hour == DAILY_SUMMARY_HOUR && !already_sent {
                        tracing::info!("[DailySummary] 9:00 AM window — compiling 24h summary");
                        match engine.compile_daily_summary(chat_id).await {
                            Ok(summary) if !summary.is_empty() => {
                                let title = crate::i18n::t(
                                    "chronos.daily_summary_title",
                                    crate::i18n::DEFAULT_LOCALE,
                                );
                                let msg = format!("{}\n\n{}", title, summary);
                                // Persist to DB BEFORE sending to prevent duplicate delivery.
                                // If persistence fails, skip the send entirely — a missed summary
                                // is better than a duplicate summary.
                                let persisted = if let Some(mem) = engine.memory_ref() {
                                    match mem
                                        .store_setting(
                                            "_daily_summary_sent_date",
                                            serde_json::Value::String(
                                                today.format("%Y-%m-%d").to_string(),
                                            ),
                                        )
                                        .await
                                    {
                                        Ok(()) => true,
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "[DailySummary] Failed to persist sent date — skipping send to prevent duplicates"
                                            );
                                            false
                                        }
                                    }
                                } else {
                                    tracing::warn!(
                                        "[DailySummary] No memory backend — skipping send to prevent duplicates"
                                    );
                                    false
                                };
                                if persisted {
                                    daily_summary_sent_date = Some(today);
                                    if let Err(e) = tx.try_send(crate::progress_router::ProgressEvent::new(
                                        chat_id,
                                        crate::progress_router::ProgressPayload::OwnerNotice {
                                            text: msg,
                                            kind: crate::progress_router::OwnerNoticeKind::NotifyUser,
                                        },
                                    )) {
                                        tracing::warn!(error = %e, "[DailySummary] Failed to send (date already persisted, won't retry today)");
                                    } else {
                                        tracing::info!("[DailySummary] Sent successfully");
                                    }
                                }
                            }
                            Ok(_) => {
                                tracing::info!(
                                    "[DailySummary] No activity in last 24h — skipping send"
                                );
                                // Persist the "sent" flag even for empty summaries so we
                                // don't re-compile every cycle during the 9 AM hour.
                                if let Some(mem) = engine.memory_ref()
                                    && let Err(e) = mem
                                        .store_setting(
                                            "_daily_summary_sent_date",
                                            serde_json::Value::String(
                                                today.format("%Y-%m-%d").to_string(),
                                            ),
                                        )
                                        .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "[DailySummary] Failed to persist sent date for empty summary"
                                    );
                                }
                                daily_summary_sent_date = Some(today);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "[DailySummary] Failed to compile");
                            }
                        }
                    }
                }
            }
        })
    }

    /// Archive the heartbeat scratchpad (memo) to L2 long-term memory, then clear it.
    ///
    /// Called automatically when quiet hours begin, or manually via `/archive` command.
    /// Returns the archived content (or None if the memo was empty/missing).
    pub async fn archive_heartbeat_memo(&self, user_did: &str) -> Result<Option<String>> {
        let memory = self
            .memory_ref()
            .context("memory not configured for memo archival")?;

        // Read current memo from settings
        let memo = self
            .settings_ref()
            .get("heartbeat_memo")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        if memo.trim().is_empty() {
            tracing::info!("[MemoArchive] No memo content to archive");
            return Ok(None);
        }

        // Build archival fact with date context. If the memo is an
        // overflow-spill pointer (Step 6), rewrite it to a human-readable
        // archive entry — the full plan is already in L2 under
        // `heartbeat:plan:<slug>`, so we archive the pointer annotated with
        // where to find it instead of the raw JSON.
        let now = chrono::Utc::now();
        // Format the archive date in the user's timezone (not UTC) — this
        // string is surfaced to the user in the archived memo fact. Mirrors
        // the pattern used for SituationSnapshot.compiled_at below.
        // Display string the owner reads — use the display resolver so a
        // skewed/invalid stored tz still yields the canonical default, not UTC.
        let user_tz = crate::settings::resolve_display_user_tz(self.settings_ref());
        let date_str = format_memo_archive_date(now, user_tz);
        let archived_fact = if memo.starts_with('{')
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(memo.trim())
            && v.get("overflow").and_then(|x| x.as_bool()) == Some(true)
        {
            let slug = v.get("slug").and_then(|x| x.as_str()).unwrap_or("?");
            let synopsis = v.get("synopsis").and_then(|x| x.as_str()).unwrap_or("");
            let bytes = v.get("bytes").and_then(|x| x.as_u64()).unwrap_or(0);
            format!(
                "[心跳便签归档 {date_str}] (overflowed to L2)\nFull plan in L2 topic heartbeat:plan:{slug} ({bytes} bytes).\nSynopsis: {synopsis}"
            )
        } else {
            format!("[心跳便签归档 {}]\n{}", date_str, memo)
        };

        // Generate embedding for semantic retrieval. Degraded-mode policy:
        // when the embedder is on backup, don't persist the backup-model
        // vector — HNSW-indexed L2 stores primary-model vectors and mixing
        // poisons similarity scores. The background reembed migration fills
        // NULL-embedding rows once primary recovers.
        let embedding = if let Some(embedder) = self.embedder() {
            if embedder.is_degraded() {
                tracing::debug!(
                    "[MemoArchive] Embedder degraded — storing memo archive without embedding"
                );
                None
            } else {
                match embedder.embed(&archived_fact).await {
                    Ok(emb) => Some(emb),
                    Err(e) => {
                        tracing::warn!(error = %e, "[MemoArchive] Embedding failed, storing without");
                        None
                    }
                }
            }
        } else {
            None
        };

        // Store to L2
        let row_id = memory
            .store_l2(
                user_did,
                "heartbeat_memo_archive",
                &archived_fact,
                embedding.as_deref(),
            )
            .await?;

        tracing::info!(
            row_id,
            memo_len = memo.len(),
            "[MemoArchive] Scratchpad archived to L2"
        );

        // Clear the memo in settings
        memory
            .store_setting("heartbeat_memo", serde_json::Value::String(String::new()))
            .await?;
        // Also update in-memory settings cache
        if let Err(e) = self
            .settings_ref()
            .set("heartbeat_memo", serde_json::Value::String(String::new()))
        {
            tracing::warn!(error = %e, "[MemoArchive] Failed to clear in-memory heartbeat_memo setting");
        }

        Ok(Some(memo))
    }

    /// Compile a structured daily summary from the last 24 hours of heartbeat audit data.
    ///
    /// Returns a human-readable markdown string summarizing all heartbeat activity,
    /// suitable for direct delivery to Telegram. Returns empty string if no activity.
    pub async fn compile_daily_summary(&self, _chat_id: i64) -> Result<String> {
        let memory = self
            .memory_ref()
            .context("memory not configured for daily summary")?;

        // Query last 20 audit entries (covers ~24h with 15-min intervals).
        // Propagate the error so the caller can retry the daily summary later.
        let audits = memory
            .query_heartbeat_audit(None, 20)
            .await
            .context("[DailySummary] Failed to query heartbeat audit log")?;

        if audits.is_empty() {
            return Ok(String::new());
        }

        // Filter to last 24h
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
        let recent: Vec<_> = audits
            .iter()
            .filter(|a| {
                chrono::DateTime::parse_from_rfc3339(&a.timestamp)
                    .or_else(|_| {
                        chrono::DateTime::parse_from_str(&a.timestamp, "%Y-%m-%d %H:%M:%S")
                    })
                    .map(|dt| dt > cutoff)
                    .unwrap_or(true) // include if timestamp unparseable
            })
            .collect();

        if recent.is_empty() {
            return Ok(String::new());
        }

        let mut sections = Vec::new();

        // Count cycles by type
        let true_hb_count = recent
            .iter()
            .filter(|a| a.skill_name == "__heartbeat__")
            .count();
        let probe_count = recent.len() - true_hb_count;
        let total_tokens: i32 = recent.iter().map(|a| a.tokens_used).sum();
        let (meaningful_tool_count, bookkeeping_tool_count, tool_summary) =
            heartbeat_daily_summary_tool_counts(&recent);

        sections.push(format!(
            "\u{1f4ca} 过去24小时 Last 24h: {} heartbeats + {} probes, {} meaningful tool calls, ~{} tokens",
            true_hb_count, probe_count, meaningful_tool_count, total_tokens
        ));

        // Summarize externally meaningful tools only; bookkeeping remains out
        // of headline user-value sections.
        if !tool_summary.is_empty() {
            sections.push(format!(
                "\u{1f527} 工具使用 Tools Used: {}",
                tool_summary.join(", ")
            ));
        }
        if bookkeeping_tool_count > 0 {
            sections.push(format!(
                "Internal maintenance: {} bookkeeping tool calls",
                bookkeeping_tool_count
            ));
        }

        // Latest LLM reply (most recent heartbeat)
        if let Some(last_hb) = recent.iter().find(|a| a.skill_name == "__heartbeat__")
            && let Some(ref reply) = last_hb.llm_reply
        {
            let truncated: String = reply.chars().take(200).collect();
            let ellipsis = if reply.chars().count() > 200 {
                "..."
            } else {
                ""
            };
            sections.push(format!(
                "\u{1f4ac} 最近心跳摘要 Latest Heartbeat: {}{}",
                truncated, ellipsis
            ));
        }

        // Current memo
        let memo = self
            .settings_ref()
            .get("heartbeat_memo")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        if !memo.is_empty() {
            let truncated: String = memo.chars().take(200).collect();
            sections.push(format!("\u{1f4dd} 当前便签 Current Memo: {}", truncated));
        }

        // P1: Tool effectiveness analysis (last 7 days)
        match self.compute_credit_analysis(memory, 7).await {
            Ok(report) if !report.tool_scores.is_empty() => {
                let effective_summary: Vec<String> = report
                    .tool_scores
                    .iter()
                    .take(5)
                    .map(|s| {
                        format!(
                            "{} ({:.0}% cited, \u{00d7}{}, {:.0}ms avg)",
                            s.tool_name,
                            s.effectiveness_score * 100.0,
                            s.times_called,
                            s.avg_latency_ms,
                        )
                    })
                    .collect();
                sections.push(format!(
                    "\u{1f4c8} 工具效果 Tool Effectiveness: {}",
                    effective_summary.join(", ")
                ));

                if !report.most_wasteful.is_empty() {
                    sections.push(format!(
                        "\u{26a0}\u{fe0f} 低效工具 Wasteful: {}",
                        report.most_wasteful.join(", ")
                    ));
                }
            }
            Ok(_) => {} // no data yet
            Err(e) => {
                tracing::warn!(error = %e, "[DailySummary] Credit analysis failed, skipping tool effectiveness section");
            }
        }

        // Budget status
        self.maybe_reset_daily_counter();
        let used = self.daily_tokens_used_raw();
        let budget = self
            .global_autonomous_config_ref()
            .read()
            .daily_token_budget;
        let pct = if budget > 0 {
            used as f64 / budget as f64 * 100.0
        } else {
            0.0
        };
        sections.push(format!(
            "\u{1f4b0} 预算 Budget: {}/{} ({:.0}%)",
            used, budget, pct
        ));

        Ok(sections.join("\n"))
    }

    /// Zero-cost precheck: returns `Some(reason)` if the heartbeat can safely
    /// skip the LLM call because nothing actionable is pending.
    ///
    /// Checks (all DB queries, zero LLM cost):
    /// - Due intents in the next hour
    /// - Pending HITL approval items
    /// - Tripped circuit breakers
    /// - Active tasks with deadlines in the next 2 hours
    ///
    /// Returns `None` when the LLM should run (something needs attention).
    async fn heartbeat_precheck(
        &self,
        chat_id: i64,
        fresh_changed_window: chrono::Duration,
    ) -> Option<String> {
        let memory = self.memory_ref()?;
        let now_dt = chrono::Utc::now();
        let changed_since = now_dt - fresh_changed_window;

        // Run all checks concurrently. Time intents are intentionally excluded:
        // `IntentScheduler` polls and executes due intents independently, so a
        // pending/future reminder is not a reason to spend a heartbeat LLM call.
        let (recent_hitl, recent_unhealthy_metrics, tasks) = tokio::join!(
            memory.count_hitl_pending_updated_since(changed_since),
            memory.count_unhealthy_skill_metrics_updated_since(changed_since),
            memory.list_tasks(chat_id, None, Some("active"), 50),
        );

        if let Ok(count) = recent_hitl
            && count > 0
        {
            return None; // Changed HITL items await
        }
        if let Ok(count) = recent_unhealthy_metrics
            && count > 0
        {
            return None; // Newly changed health conditions need attention
        }
        let actionable_task = tasks
            .iter()
            .any(|t| heartbeat_task_is_actionable(t, now_dt, fresh_changed_window));
        if actionable_task {
            return None; // Urgent or upcoming task
        }

        Some("no changed HITL, changed health issues, or actionable tasks".into())
    }

    async fn heartbeat_has_recent_changed_backlog_context(
        &self,
        changed_since: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        let Some(memory) = self.memory_ref() else {
            return true;
        };

        let (recent_hitl, recent_unhealthy_metrics) = tokio::join!(
            memory.count_hitl_pending_updated_since(changed_since),
            memory.count_unhealthy_skill_metrics_updated_since(changed_since),
        );

        recent_hitl.map(|count| count > 0).unwrap_or(true)
            || recent_unhealthy_metrics
                .map(|count| count > 0)
                .unwrap_or(true)
    }

    async fn heartbeat_last_noop_timestamp(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let memory = self.memory_ref()?;
        let entries = match memory
            .query_heartbeat_audit(Some("__heartbeat__"), 10)
            .await
        {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[TrueHeartbeat] Failed to query last heartbeat noop"
                );
                return None;
            }
        };

        entries.into_iter().find_map(|entry| {
            (entry.outcome.as_deref() == Some("noop"))
                .then(|| heartbeat_parse_timestamp_utc(&entry.timestamp))
                .flatten()
        })
    }

    async fn controlled_thaw_hourly_skip_reason(
        &self,
        cfg: &crate::heartbeat_config::HeartbeatConfig,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<String> {
        let thaw = &cfg.controlled_thaw;
        if !thaw.enabled {
            return None;
        }

        let cap = thaw.max_llm_wakeups_per_hour;
        if cap == 0 {
            return Some("controlled_thaw hourly LLM wakeup cap is 0".to_string());
        }

        let Some(memory) = self.memory_ref() else {
            return Some("controlled_thaw cannot verify hourly budget: memory unavailable".into());
        };

        let entries = match memory
            .query_heartbeat_audit(Some("__heartbeat__"), 100)
            .await
        {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[TrueHeartbeat] controlled_thaw hourly budget query failed"
                );
                return Some("controlled_thaw cannot verify hourly budget".into());
            }
        };
        let used = heartbeat_count_llm_wakeups_in_window(&entries, now, chrono::Duration::hours(1));
        if used >= cap {
            return Some(format!(
                "controlled_thaw hourly LLM wakeup cap reached: {used}/{cap} in the last 60m"
            ));
        }

        None
    }

    /// Step 2 (design §13): write a "bookkeeping" audit row for ticks that
    /// never reach the LLM — quiet hours, daily-budget exhaustion, precheck
    /// skip, snapshot-benign skip, or a handler error. Guarantees every
    /// metronome tick is visible in `heartbeat_audit_log`.
    ///
    /// Best-effort: any storage error is logged and swallowed so a failed
    /// audit write never poisons the heartbeat loop.
    pub(crate) async fn write_heartbeat_tick_audit(
        &self,
        chat_id: i64,
        user_did: &str,
        outcome: crate::heartbeat_config::HeartbeatTickOutcome,
        skip_reason: Option<crate::heartbeat_config::HeartbeatSkipReason>,
        skip_reason_text: Option<String>,
        llm_reply: Option<&str>,
        severity: &str,
    ) {
        let Some(mem) = self.memory_ref() else {
            return;
        };
        let label = "__heartbeat__";
        let cycle_id = format!("{}-{}", label, chrono::Utc::now().timestamp_millis());
        // Snapshot the *effective* heartbeat config for this tick (design §13).
        let cfg_snapshot = serde_json::to_string(&self.get_true_heartbeat_config())
            .unwrap_or_else(|_| "{}".into());
        let extras = crate::memory::HeartbeatAuditExtras {
            outcome: Some(outcome),
            skip_reason,
            skip_reason_text,
            config_snapshot_json: Some(cfg_snapshot.clone()),
            prompt_hash: None,
            proposals_created: Vec::new(),
            next_tick_note_bytes: None,
        };
        if let Err(e) = mem
            .store_heartbeat_audit_ext(
                &cycle_id,
                label,
                severity,
                None,
                &[],
                &[],
                &[],
                llm_reply,
                0,
                0,
                user_did,
                Some(chat_id),
                &cfg_snapshot,
                None,
                None,
                &extras,
            )
            .await
        {
            tracing::warn!(
                error = %e,
                outcome = outcome.as_str(),
                "[TrueHeartbeat] Failed to store skip/error audit row"
            );
        }
    }

    /// Compile a [`SituationSnapshot`] from all available memory data sources.
    ///
    /// This is the "gather" phase of the True Heartbeat: query every relevant
    /// data source and assemble a unified view of system state.
    /// All queries run concurrently via `tokio::join!`. Individual query
    /// failures produce empty data rather than aborting the snapshot.
    pub async fn compile_snapshot(&self, chat_id: i64) -> Result<SituationSnapshot> {
        let memory = self
            .memory_ref()
            .context("memory not configured for true heartbeat snapshot")?;

        let now = chrono::Utc::now();
        let now_unix = now.timestamp();

        let tz_name = self
            .settings_ref()
            .get("user_timezone")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "UTC".to_string());

        // Run all memory queries concurrently
        // Heartbeat memory now lives in the scratchpad (heartbeat_memo setting),
        // so we no longer query __heartbeat__ audit entries here.
        let (probe_results, tasks, intents, metrics, hitl) = tokio::join!(
            memory.query_heartbeat_audit(None, 20),
            memory.list_tasks(chat_id, None, Some("active"), 20),
            memory.get_due_time_intents(now_unix + 3600), // next hour
            memory.get_all_skill_metrics(),
            memory.list_hitl_pending(),
        );

        // Compute budget status
        self.maybe_reset_daily_counter();
        let used = self.daily_tokens_used_raw();
        let config = self.get_true_heartbeat_config();
        let auto_cfg =
            config.effective_autonomous_config(&self.global_autonomous_config_ref().read());

        // Tripped circuit breakers
        let skill_metrics_vec = match metrics {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "[Snapshot] Failed to fetch skill metrics");
                Vec::new()
            }
        };
        let mut tripped = Vec::new();
        for m in &skill_metrics_vec {
            if m.circuit_open {
                tripped.push((
                    m.skill_name.clone(),
                    m.consecutive_failures as i64,
                    m.last_error.clone(),
                ));
            }
        }

        let probe_results_vec = match probe_results {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "[Snapshot] Failed to fetch probe results");
                Vec::new()
            }
        };
        let intents_vec = match intents {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "[Snapshot] Failed to fetch upcoming intents");
                Vec::new()
            }
        };
        let hitl_vec = match hitl {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "[Snapshot] Failed to fetch HITL pending items");
                Vec::new()
            }
        };

        Ok(SituationSnapshot {
            compiled_at: {
                // Format compiled_at in the user's timezone (not UTC)
                let tz = crate::settings::parse_tz_str(&tz_name);
                now.with_timezone(&tz)
                    .format("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string()
            },
            timezone: tz_name,
            recent_probe_results: probe_results_vec,
            pending_tasks: tasks,
            upcoming_intents: intents_vec,
            skill_metrics: skill_metrics_vec,
            tripped_circuit_breakers: tripped,
            hitl_pending: hitl_vec,
            budget_status: (used, auto_cfg.daily_token_budget),
            user_last_active_unix: {
                // Derive from L1 history: find latest user message timestamp
                let user_did = format!("did:aegis:telegram:{}", chat_id);
                match memory.get_l1(&user_did).await {
                    Ok(msgs) => msgs
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .map(|m| m.timestamp),
                    Err(_) => None,
                }
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_memo_archive_date, heartbeat_count_llm_wakeups_in_window,
        heartbeat_daily_summary_tool_counts, heartbeat_error_is_recent,
        heartbeat_snapshot_has_actionable_backlog, heartbeat_tools_did_real_work,
        retention_cutoff_unix,
    };

    #[test]
    fn retention_cutoff_normal_and_default_paths() {
        let now = 1_800_000_000_i64;
        // Missing value -> default.
        assert_eq!(
            retention_cutoff_unix(None, 180, now),
            now - 180 * 86_400,
            "None falls back to default_days"
        );
        // Non-positive -> default (never a 0/negative window).
        assert_eq!(retention_cutoff_unix(Some(0), 30, now), now - 30 * 86_400);
        assert_eq!(retention_cutoff_unix(Some(-7), 30, now), now - 30 * 86_400);
        // Normal positive value is honored verbatim.
        assert_eq!(retention_cutoff_unix(Some(45), 30, now), now - 45 * 86_400);
    }

    #[test]
    fn retention_cutoff_clamps_absurd_value_without_overflow() {
        let now = 1_800_000_000_i64;
        // A hostile/typo'd huge value must NOT overflow into a far-future cutoff
        // (which would make every archived row eligible for permanent deletion),
        // and must NOT panic. It is clamped to the 100-year ceiling, yielding a
        // cutoff far in the PAST (so effectively "never delete").
        let cutoff = retention_cutoff_unix(Some(i64::MAX), 180, now);
        assert!(
            cutoff < now,
            "clamped cutoff stays in the past, never far-future: {cutoff} vs now {now}"
        );
        assert_eq!(cutoff, now - 36_500 * 86_400, "clamped to 100-year ceiling");
        // i64::MAX days would have overflowed `now - days*86400`; confirm we are
        // nowhere near saturation and the result is finite/sane.
        assert!(cutoff > i64::MIN);
    }

    #[test]
    fn memo_archive_date_uses_user_timezone_not_utc() {
        // 2026-04-26T16:30:00Z is still 2026-04-26 in UTC, but already
        // 2026-04-27 01:30 in Asia/Tokyo. The archive date the owner sees
        // must reflect the owner's local day.
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-26T16:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        // Regression: before the fix this used `now.format(..)` in UTC and
        // would have produced "2026-04-26" regardless of tz.
        assert_eq!(
            format_memo_archive_date(now, chrono_tz::Tz::Asia__Tokyo),
            "2026-04-27",
            "archive date must roll over in the user's timezone"
        );
        assert_eq!(
            format_memo_archive_date(now, chrono_tz::Tz::America__New_York),
            "2026-04-26",
            "America/New_York is still the prior day at this instant"
        );
        // UTC sanity check (proves the helper actually applies the tz arg).
        assert_eq!(
            format_memo_archive_date(now, chrono_tz::Tz::UTC),
            "2026-04-26"
        );
    }

    fn fixed_now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-04-26T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn mk_entry(timestamp: &str, severity: &str) -> crate::memory::HeartbeatAuditEntry {
        crate::memory::HeartbeatAuditEntry {
            id: 1,
            cycle_id: "cycle-1".into(),
            skill_name: "__heartbeat__".into(),
            timestamp: timestamp.into(),
            severity: severity.into(),
            probe_payload: None,
            tools_called: Vec::new(),
            tools_blocked: Vec::new(),
            actions_taken: Vec::new(),
            llm_reply: None,
            tokens_used: 0,
            rounds_used: 0,
            user_did: "did:test:1".into(),
            chat_id: Some(1),
            raw_process_log: None,
            tool_call_details: None,
            outcome: Some("error".into()),
            skip_reason: None,
            config_snapshot_json: None,
            prompt_hash: None,
            proposals_created_json: None,
            next_tick_note_bytes: None,
        }
    }

    fn mk_entry_with_tools(tools: &[&str]) -> crate::memory::HeartbeatAuditEntry {
        let mut entry = mk_entry("2026-04-22 05:30:00", "info");
        entry.tools_called = tools.iter().map(|s| s.to_string()).collect();
        entry
    }

    fn mk_llm_entry(timestamp: &str) -> crate::memory::HeartbeatAuditEntry {
        let mut entry = mk_entry(timestamp, "info");
        entry.tokens_used = 42;
        entry.rounds_used = 1;
        entry.prompt_hash = Some("hash".into());
        entry.outcome = Some("noop".into());
        entry
    }

    fn mk_task(
        title: &str,
        priority: &str,
        due_date: Option<&str>,
        trigger_at_unix: Option<i64>,
    ) -> crate::memory::Task {
        crate::memory::Task {
            id: 1,
            chat_id: 100,
            user_did: "did:test".into(),
            title: title.into(),
            body: String::new(),
            priority: priority.into(),
            status: "active".into(),
            tags: Vec::new(),
            due_date: due_date.map(String::from),
            source: "user".into(),
            trigger_at_unix,
            trigger_type: trigger_at_unix.map(|_| "time".into()),
            event_key: None,
            execution_mode: "notify".into(),
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
            created_at: "2026-04-20T00:00:00Z".into(),
            updated_at: "2026-04-20T00:00:00Z".into(),
            completed_at: None,
        }
    }

    fn mark_task_recent(task: &mut crate::memory::Task) {
        task.updated_at = fixed_now().to_rfc3339();
    }

    fn mark_task_needs_coordination(task: &mut crate::memory::Task) {
        task.tags.push("heartbeat_review".into());
    }

    fn mk_snapshot() -> crate::chronos::SituationSnapshot {
        crate::chronos::SituationSnapshot {
            compiled_at: "2026-04-26T10:00:00Z".into(),
            timezone: "UTC".into(),
            recent_probe_results: Vec::new(),
            pending_tasks: Vec::new(),
            upcoming_intents: Vec::new(),
            skill_metrics: Vec::new(),
            tripped_circuit_breakers: Vec::new(),
            hitl_pending: Vec::new(),
            budget_status: (0, 500_000),
            user_last_active_unix: None,
        }
    }

    fn mk_intent(trigger_at_unix: Option<i64>) -> crate::memory::DeferredIntent {
        crate::memory::DeferredIntent {
            id: 1,
            user_did: "did:test".into(),
            chat_id: 100,
            trigger_type: "time".into(),
            trigger_at_unix,
            event_key: None,
            execution_mode: "notify".into(),
            context_text: "Reminder".into(),
            status: "pending".into(),
            created_at: "2026-04-20T00:00:00Z".into(),
            expires_at_unix: fixed_now().timestamp() + 86_400,
            fired_at: None,
            is_callback_origin: false,
            retry_count: 0,
            interval_seconds: None,
            chain_depth: 0,
            pin_on_fire: false,
            group_key: None,
            max_fires: None,
            fire_count: 0,
            recur_weekdays: None,
            recur_window_start: None,
            recur_window_end: None,
            recur_timezone: None,
            embedding: None,
        }
    }

    #[test]
    fn recent_error_blocks_skip() {
        let now = chrono::NaiveDateTime::parse_from_str("2026-04-22 06:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc();
        let entry = mk_entry("2026-04-22 05:30:00", "error");
        assert!(heartbeat_error_is_recent(
            &entry,
            now,
            chrono::Duration::minutes(60)
        ));
    }

    #[test]
    fn stale_error_does_not_block_skip() {
        let now = chrono::NaiveDateTime::parse_from_str("2026-04-22 06:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc();
        let entry = mk_entry("2026-04-22 03:30:00", "error");
        assert!(!heartbeat_error_is_recent(
            &entry,
            now,
            chrono::Duration::minutes(60)
        ));
    }

    #[test]
    fn non_error_severity_never_blocks_skip() {
        let now = chrono::NaiveDateTime::parse_from_str("2026-04-22 06:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc();
        let entry = mk_entry("2026-04-22 05:30:00", "info");
        assert!(!heartbeat_error_is_recent(
            &entry,
            now,
            chrono::Duration::minutes(60)
        ));
    }

    #[test]
    fn daily_summary_tool_counts_exclude_bookkeeping_headlines() {
        let memo = mk_entry_with_tools(&["noop", "update_heartbeat_memo"]);
        let mixed =
            mk_entry_with_tools(&["get_current_time", "send_message", "propose_config_change"]);
        let recent = vec![&memo, &mixed];

        let (meaningful_count, bookkeeping_count, summary) =
            heartbeat_daily_summary_tool_counts(&recent);

        assert_eq!(meaningful_count, 2);
        assert_eq!(bookkeeping_count, 3);
        assert_eq!(
            summary,
            vec![
                "propose_config_change (\u{00d7}1)".to_string(),
                "send_message (\u{00d7}1)".to_string()
            ]
        );
        assert!(!summary.iter().any(|s| s.contains("noop")));
        assert!(!summary.iter().any(|s| s.contains("update_heartbeat_memo")));
    }

    #[test]
    fn heartbeat_noop_tools_do_not_reset_idle_backoff() {
        let tools = vec![
            "noop".to_string(),
            "get_current_time".to_string(),
            "update_heartbeat_memo".to_string(),
        ];

        assert!(!heartbeat_tools_did_real_work(&tools));
    }

    #[test]
    fn heartbeat_user_visible_tool_resets_idle_backoff() {
        let tools = vec!["noop".to_string(), "notify_user".to_string()];

        assert!(heartbeat_tools_did_real_work(&tools));
    }

    #[test]
    fn heartbeat_stable_backlog_does_not_wake_llm() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Long-running ordinary backlog item",
            "normal",
            None,
            None,
        ));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_task_three_hours_away_does_not_wake_llm() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Check mail later today",
            "normal",
            None,
            Some(fixed_now().timestamp() + 3 * 60 * 60),
        ));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_scheduler_owned_imminent_task_does_not_wake_llm() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Remind me to call the bank soon",
            "normal",
            None,
            Some(fixed_now().timestamp() + 90 * 60),
        ));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_coordination_task_near_window_wakes_llm() {
        let mut snapshot = mk_snapshot();
        let mut task = mk_task(
            "Review bank paperwork before the call",
            "normal",
            None,
            Some(fixed_now().timestamp() + 90 * 60),
        );
        mark_task_needs_coordination(&mut task);
        snapshot.pending_tasks.push(task);

        assert!(heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_missed_scheduler_task_wakes_llm() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Send scheduled reminder",
            "normal",
            None,
            Some(fixed_now().timestamp() - 15 * 60),
        ));

        assert!(heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_recently_due_scheduler_task_stays_with_scheduler() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Send scheduled reminder",
            "normal",
            None,
            Some(fixed_now().timestamp() - 5 * 60),
        ));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_stale_urgent_task_without_deadline_does_not_wake_repeatedly() {
        let mut snapshot = mk_snapshot();
        snapshot
            .pending_tasks
            .push(mk_task("Call bank now", "urgent", None, None));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_recent_urgent_task_without_deadline_wakes_once() {
        let mut snapshot = mk_snapshot();
        let mut task = mk_task("Call bank now", "urgent", None, None);
        mark_task_recent(&mut task);
        snapshot.pending_tasks.push(task);

        assert!(heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_error_before_previous_noop_does_not_wake_again() {
        let mut snapshot = mk_snapshot();
        snapshot
            .recent_probe_results
            .push(mk_entry("2026-04-26 09:30:00", "error"));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(20),
        ));
    }

    #[test]
    fn heartbeat_error_after_previous_noop_wakes_once() {
        let mut snapshot = mk_snapshot();
        snapshot
            .recent_probe_results
            .push(mk_entry("2026-04-26 09:50:00", "error"));

        assert!(heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(20),
        ));
    }

    #[test]
    fn heartbeat_coordination_deadline_crossing_window_after_noop_still_wakes() {
        let mut snapshot = mk_snapshot();
        let mut task = mk_task(
            "Call the bank soon",
            "normal",
            None,
            Some(fixed_now().timestamp() + 30 * 60),
        );
        mark_task_needs_coordination(&mut task);
        snapshot.pending_tasks.push(task);

        assert!(heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(20),
        ));
    }

    #[test]
    fn heartbeat_date_only_due_task_does_not_wake_all_day_when_stale() {
        let mut snapshot = mk_snapshot();
        snapshot.pending_tasks.push(mk_task(
            "Review sometime today",
            "normal",
            Some("2026-04-26"),
            None,
        ));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_due_time_intent_does_not_wake_llm_pollers_handle_it() {
        let mut snapshot = mk_snapshot();
        snapshot
            .upcoming_intents
            .push(mk_intent(Some(fixed_now().timestamp())));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn heartbeat_stable_pending_approval_does_not_wake_llm_by_itself() {
        let mut snapshot = mk_snapshot();
        snapshot
            .hitl_pending
            .push(("approve_wire_followup".into(), "pending".into()));

        assert!(!heartbeat_snapshot_has_actionable_backlog(
            &snapshot,
            fixed_now(),
            chrono::Duration::minutes(60),
        ));
    }

    #[test]
    fn controlled_thaw_counts_recent_heartbeat_llm_wakeups_only() {
        let mut other_skill = mk_llm_entry("2026-04-26T09:50:00Z");
        other_skill.skill_name = "server_monitor".into();

        let entries = vec![
            mk_llm_entry("2026-04-26T09:10:00Z"),
            mk_llm_entry("2026-04-26 09:30:00"),
            mk_llm_entry("2026-04-26T08:30:00Z"),
            other_skill,
            mk_entry("2026-04-26T09:55:00Z", "info"),
        ];

        assert_eq!(
            heartbeat_count_llm_wakeups_in_window(
                &entries,
                fixed_now(),
                chrono::Duration::hours(1)
            ),
            2
        );
    }
}
