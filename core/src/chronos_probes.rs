//! Probe execution, cooldown logic, heartbeat dispatch, and cron scheduler
//! for the Chronos engine.
//!
//! Extracted from `chronos.rs` to keep that file under the 2,000-line limit.
//! All methods are `impl ChronosEngine` — same pattern as `orchestrator_tools_*.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use chrono::Timelike;
use serde_json::Value;

use crate::chronos::{
    AlertState, ChronosEngine, CooldownAction, HEARTBEAT_PROBE_TIMEOUT, HeartbeatHandler,
    HeartbeatOutcome, HeartbeatResult,
};
use crate::embedding::EmbeddingClient;
use crate::models::{AlertSeverity, ProbeSignal, SkillCapability};
use crate::settings::RuntimeSettings;
use crate::trigger::Trigger;

fn heartbeat_outcome_label(outcome: &HeartbeatOutcome) -> &'static str {
    match outcome {
        HeartbeatOutcome::Silent { .. } => "silent",
        HeartbeatOutcome::Escalated { .. } => "escalated",
        HeartbeatOutcome::Suppressed { .. } => "suppressed",
    }
}

fn heartbeat_skill_error_category(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("fuel") || lower.contains("resource") {
        "resource"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("auth") || lower.contains("401") || lower.contains("oauth") {
        "auth"
    } else if lower.contains("rate") || lower.contains("429") {
        "rate_limit"
    } else if lower.contains("network") || lower.contains("fetch") {
        "network"
    } else {
        "bug"
    }
}

fn inject_heartbeat_host_payload_fields(skill_name: &str, payload: &mut Value) {
    inject_heartbeat_host_payload_fields_at(skill_name, payload, chrono::Utc::now().timestamp());
}

fn inject_heartbeat_host_payload_fields_at(skill_name: &str, payload: &mut Value, now_ts: i64) {
    let Value::Object(fields) = payload else {
        return;
    };

    if skill_name == "calendar_reader" {
        let has_valid_current_timestamp = fields
            .get("current_timestamp")
            .and_then(Value::as_i64)
            .is_some_and(|ts| ts > 0);
        if !has_valid_current_timestamp {
            fields.insert("current_timestamp".to_string(), Value::from(now_ts));
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_outcome_label_names_scheduler_log_outcomes() {
        assert_eq!(
            heartbeat_outcome_label(&HeartbeatOutcome::Silent {
                skill_name: "probe".to_string(),
                elapsed_ms: 1
            }),
            "silent"
        );
        assert_eq!(
            heartbeat_outcome_label(&HeartbeatOutcome::Suppressed {
                skill_name: "probe".to_string(),
                elapsed_ms: 1
            }),
            "suppressed"
        );
        assert_eq!(
            heartbeat_outcome_label(&HeartbeatOutcome::Escalated {
                skill_name: "probe".to_string(),
                alert_message: "alert".to_string(),
                elapsed_ms: 1
            }),
            "escalated"
        );
    }
    #[test]
    fn heartbeat_host_fields_add_calendar_timestamp_when_missing() {
        let mut payload = serde_json::json!({"action": "today"});
        inject_heartbeat_host_payload_fields_at("calendar_reader", &mut payload, 1_777_000_000);
        assert_eq!(
            payload.get("current_timestamp").and_then(Value::as_i64),
            Some(1_777_000_000)
        );
    }

    #[test]
    fn heartbeat_host_fields_replace_invalid_calendar_timestamp() {
        let mut payload = serde_json::json!({
            "action": "today",
            "current_timestamp": 0
        });
        inject_heartbeat_host_payload_fields_at("calendar_reader", &mut payload, 1_777_000_001);
        assert_eq!(
            payload.get("current_timestamp").and_then(Value::as_i64),
            Some(1_777_000_001)
        );
    }

    #[test]
    fn heartbeat_host_fields_preserve_valid_calendar_timestamp() {
        let mut payload = serde_json::json!({
            "action": "today",
            "current_timestamp": 1_777_000_002i64
        });
        inject_heartbeat_host_payload_fields_at("calendar_reader", &mut payload, 1_777_000_003);
        assert_eq!(
            payload.get("current_timestamp").and_then(Value::as_i64),
            Some(1_777_000_002)
        );
    }
    #[test]
    fn heartbeat_host_fields_ignore_non_calendar_payload() {
        let mut payload = serde_json::json!({"action": "check_new"});
        inject_heartbeat_host_payload_fields_at("web_reader", &mut payload, 1_777_000_004);
        assert!(payload.get("current_timestamp").is_none());
    }
}

/// Decide whether a cron job should be registered for this skill at startup.
///
/// Registration is skipped when:
/// - the skill has no `cron_schedule` (nothing to register), or
/// - the skill's runtime toggle (`skill_<name>`) is disabled.
///
/// Keeping this a free function (not a method) makes it trivially unit-testable
/// without needing a live `ChronosEngine` / `SandboxEngine`.
///
/// NOTE: Because this runs at service start, toggling a skill from disabled to
/// enabled via `update_setting` will NOT retroactively register the cron — a
/// service restart is required. We deliberately accept this trade-off (documented
/// in the "Skipping cron registration for disabled skill" debug log) since the
/// alternative is either (a) per-tick re-checking, which is what the 2026-04-11
/// fix moved away from because it produced one log line per tick per disabled
/// skill, or (b) a full scheduler-rebuild listener, which is out of scope for
/// this P2 fix.
/// Decision of the runtime cron registration's RECORD step (phase 3 of
/// `register_account_cron_jobs`), extracted pure so the unregister-race
/// branches are deterministically unit-testable (Codex adversarial rounds
/// 1–2).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RecordJobsOutcome {
    /// Our reservation is still current → guids recorded (or reservation
    /// freed on a total add failure so a retry can re-register).
    Recorded,
    /// Our reservation is GONE or was CONSUMED-AND-REPLACED (a concurrent
    /// unregister — possibly followed by a re-register that created a NEW
    /// generation for the same id) — the caller must remove these just-added
    /// jobs from the scheduler, or they orphan forever / double-probe.
    RolledBack(Vec<uuid::Uuid>),
}

/// Record the freshly added job guids under the account's reservation, but
/// ONLY when the slot still carries OUR `generation` — a same-generation
/// check is what stops a stale in-flight registration from recording into a
/// newer re-register's slot (Codex adversarial round 2, HIGH).
/// `had_add_error` with no landed guids frees the reservation for a clean
/// retry.
pub(crate) fn record_account_job_guids(
    runtime_cron: &mut Option<crate::chronos::RuntimeCronContext>,
    account_id: &str,
    generation: u64,
    guids: Vec<uuid::Uuid>,
    had_add_error: bool,
) -> RecordJobsOutcome {
    let Some(ctx) = runtime_cron.as_mut() else {
        // Context dropped (never happens after start, but fail safe): the
        // jobs cannot be tracked → roll them back.
        return RecordJobsOutcome::RolledBack(guids);
    };
    match ctx.account_jobs.get_mut(account_id) {
        Some(slot) if slot.generation == generation => {
            if guids.is_empty() && had_add_error {
                // Nothing landed — drop the reservation for a clean retry.
                ctx.account_jobs.remove(account_id);
            } else {
                slot.guids = guids;
            }
            RecordJobsOutcome::Recorded
        }
        // Different generation: our reservation was unregistered and the id
        // re-registered while our adds were in flight. The new generation
        // owns the slot — our jobs must come back out.
        Some(_) => RecordJobsOutcome::RolledBack(guids),
        // Concurrent unregister consumed the reservation.
        None => RecordJobsOutcome::RolledBack(guids),
    }
}

pub(crate) fn should_register_cron_for_skill(
    cap: &SkillCapability,
    settings: &RuntimeSettings,
) -> bool {
    if cap.cron_schedule.is_none() {
        return false;
    }
    let setting_key = format!("skill_{}", cap.skill_name);
    settings.is_enabled(&setting_key)
}

impl ChronosEngine {
    /// Builder: override alert cooldown duration (useful for testing).
    #[cfg(test)]
    /// Tests: opt back into the quiet-hours hold.
    #[cfg(test)]
    pub fn with_owner_quiet_hours(mut self, honor: bool) -> Self {
        self.honor_owner_quiet_hours = honor;
        self
    }

    /// How long a condition must stay clear before its ✅ goes out
    /// (default 15 min; tests use zero).
    pub fn with_resolve_debounce(mut self, debounce: std::time::Duration) -> Self {
        self.resolve_debounce = debounce;
        self
    }

    pub fn with_alert_cooldown(mut self, cooldown: std::time::Duration) -> Self {
        self.alert_cooldown = cooldown;
        self
    }

    /// Determine the cooldown action for a given skill based on the current
    /// probe result, severity, and the previous alert state.
    /// Owner quiet hours from `heartbeat_config`, evaluated in `user_timezone`.
    pub(crate) fn owner_quiet_hours_now(&self) -> bool {
        if !self.honor_owner_quiet_hours {
            return false;
        }
        let settings = self.settings_ref();
        let Some(cfg) = settings
            .get("heartbeat_config")
            .and_then(|v| crate::heartbeat_config::HeartbeatConfig::from_json(&v))
        else {
            return false;
        };
        let tz = settings
            .get("user_timezone")
            .and_then(|v| v.as_str().map(String::from))
            .and_then(|name| name.parse::<chrono_tz::Tz>().ok())
            .unwrap_or(chrono_tz::Asia::Shanghai);
        cfg.quiet_hours.in_quiet_hours(tz)
    }

    pub(crate) fn cooldown_action(
        &self,
        skill_name: &str,
        severity: AlertSeverity,
        is_alert: bool,
    ) -> CooldownAction {
        let policy = self.get_policy(skill_name);
        let now_utc_hour = chrono::Utc::now().hour() as u8;

        if is_alert {
            // Check if this severity should be delivered at all
            if !policy.should_deliver(severity, now_utc_hour) {
                return CooldownAction::Silent;
            }
            // The owner's quiet hours (heartbeat_config, owner timezone) apply to
            // probe alerts too — only Critical gets through at 04:00.
            if severity != crate::models::AlertSeverity::Critical && self.owner_quiet_hours_now() {
                tracing::info!(
                    skill = skill_name,
                    "[Chronos] alert held: owner quiet hours"
                );
                return CooldownAction::Silent;
            }

            let effective_cooldown = policy.effective_cooldown(self.alert_cooldown_duration());

            // Hold the lock once for both read + write to avoid deadlock.
            let mut state = self.alert_state_lock();
            let action = match state.get(skill_name) {
                Some(prev) if prev.was_alerting => {
                    if prev.last_alert_sent.elapsed() >= effective_cooldown {
                        CooldownAction::Reminder
                    } else {
                        return CooldownAction::Suppress; // no state update needed
                    }
                }
                // Flap guard: a condition that "recovered" and is back inside
                // the cooldown is the same incident — no new 🚨 (production
                // 2026-09-12: 14 alert/recovery pairs a day around 70% disk).
                Some(prev) if prev.last_alert_sent.elapsed() < effective_cooldown => {
                    let sent = prev.last_alert_sent;
                    state.insert(
                        skill_name.to_string(),
                        AlertState {
                            last_alert_sent: sent,
                            was_alerting: true,
                            resolve_pending: false,
                        },
                    );
                    return CooldownAction::Suppress;
                }
                _ => CooldownAction::Send,
            };
            state.insert(
                skill_name.to_string(),
                AlertState {
                    last_alert_sent: std::time::Instant::now(),
                    was_alerting: true,
                    resolve_pending: false,
                },
            );
            action
        } else {
            // Probe is silent
            let mut state = self.alert_state_lock();
            let debounce = self.resolve_debounce;
            match state.get(skill_name).copied() {
                Some(prev) if prev.was_alerting => {
                    // A recovery right after the alert is usually a flap:
                    // hold the ✅ until the condition has stayed clear for
                    // `resolve_debounce` (owed via resolve_pending).
                    let settled = prev.last_alert_sent.elapsed() >= debounce;
                    state.insert(
                        skill_name.to_string(),
                        AlertState {
                            last_alert_sent: prev.last_alert_sent,
                            was_alerting: false,
                            resolve_pending: !settled && policy.send_resolved,
                        },
                    );
                    if settled && policy.send_resolved {
                        CooldownAction::Resolved
                    } else {
                        CooldownAction::Silent
                    }
                }
                Some(prev) if prev.resolve_pending => {
                    if prev.last_alert_sent.elapsed() >= debounce {
                        state.insert(
                            skill_name.to_string(),
                            AlertState {
                                last_alert_sent: prev.last_alert_sent,
                                was_alerting: false,
                                resolve_pending: false,
                            },
                        );
                        CooldownAction::Resolved
                    } else {
                        CooldownAction::Silent
                    }
                }
                _ => CooldownAction::Silent,
            }
        }
    }

    /// Execute a single heartbeat probe for a skill.
    ///
    /// This is the core of the three-tier rocket:
    /// - Signs a proper IntentEnvelope with the system identity
    /// - Goes through full Ed25519 verification (no bypass)
    /// - Parses the ProbeSignal from the Wasm output
    /// - Returns the result for the caller to decide on escalation
    pub fn probe(&self, skill_name: &str, payload: &Value) -> Result<HeartbeatResult> {
        let mut payload = payload.clone();
        inject_heartbeat_host_payload_fields(skill_name, &mut payload);
        self.sandbox_ref()
            .loaded_skills()
            .into_iter()
            .find(|cap| cap.skill_name == skill_name)
            .with_context(|| format!("heartbeat skill '{skill_name}' is not loaded"))?;
        let envelope = self
            .system_identity_ref()
            .sign(skill_name, payload.clone())
            .with_context(|| format!("signing heartbeat envelope for '{skill_name}'"))?;
        let output = self
            .sandbox_ref()
            .execute_system(&envelope)
            .map(|result| (result.output, result.elapsed_ms))
            .with_context(|| format!("heartbeat probe for '{skill_name}' failed"))?;

        let probe: ProbeSignal = serde_json::from_str(&output.0).with_context(|| {
            format!(
                "skill '{skill_name}' returned invalid ProbeSignal: {}",
                output.0
            )
        })?;

        Ok(HeartbeatResult {
            skill_name: skill_name.to_string(),
            probe,
            elapsed_ms: output.1,
        })
    }

    /// Run a heartbeat cycle: probe the skill, then dispatch to handler.
    ///
    /// Alert cooldown logic:
    /// - First alert -> Send (on_alert)
    /// - Repeated alert within cooldown -> Suppress (on_silent, no LLM cost)
    /// - Repeated alert after cooldown -> Reminder (on_alert with "[STILL ONGOING]" prefix)
    /// - Was alerting, now silent -> Resolved (on_alert with "[RESOLVED]" notification)
    /// - Silent to silent -> Silent (on_silent)
    ///
    /// `on_observation()` is always called regardless of cooldown action.
    ///
    /// # Thread-safety note
    /// The Wasm probe is CPU-bound and may block for up to `HEARTBEAT_PROBE_TIMEOUT`
    /// while waiting for HTTP responses inside `host_fetch`. To avoid stalling a
    /// Tokio async worker, execution is offloaded to a `spawn_blocking` thread and
    /// wrapped with a hard timeout. If the probe exceeds the timeout it is aborted
    /// and the outcome is treated as a transient error (not escalated).
    pub async fn run_heartbeat(
        &self,
        skill_name: &str,
        payload: &Value,
        handler: &dyn HeartbeatHandler,
    ) -> Result<HeartbeatOutcome> {
        // Gate: if this skill's toggle is disabled, skip entirely
        let setting_key = format!("skill_{}", skill_name);
        if !self.settings_ref().is_enabled(&setting_key) {
            tracing::debug!(
                skill = skill_name,
                setting = %setting_key,
                "Heartbeat skipped: skill disabled via settings"
            );
            return Ok(HeartbeatOutcome::Silent {
                skill_name: skill_name.to_string(),
                elapsed_ms: 0,
            });
        }

        // Gate: if this skill has been unloaded from the sandbox (e.g. deactivated), skip
        if !self.sandbox_ref().is_skill_loaded(skill_name) {
            tracing::debug!(skill = skill_name, "skipping heartbeat for unloaded skill");
            return Ok(HeartbeatOutcome::Silent {
                skill_name: skill_name.to_string(),
                elapsed_ms: 0,
            });
        }

        // Offload the synchronous Wasm execution + blocking HTTP I/O to a
        // Tokio blocking thread so we do not stall the async worker. Clone only
        // the Arc fields needed by probe(); no heap allocation beyond that.
        let sandbox = self.sandbox_arc();
        let identity = self.system_identity_arc();
        let skill_for_probe = skill_name.to_string();
        let mut payload_for_probe = payload.clone();
        inject_heartbeat_host_payload_fields(skill_name, &mut payload_for_probe);
        self.sandbox_ref()
            .loaded_skills()
            .into_iter()
            .find(|cap| cap.skill_name == skill_name)
            .with_context(|| format!("heartbeat skill '{skill_name}' is not loaded"))?;

        let probe_task = tokio::task::spawn_blocking(move || {
            let (output, elapsed_ms) = {
                let envelope = identity
                    .sign(&skill_for_probe, payload_for_probe)
                    .with_context(|| {
                        format!("signing heartbeat envelope for '{skill_for_probe}'")
                    })?;
                let result = sandbox.execute_system(&envelope).map_err(|e| {
                    // Preserve the original error chain so auth failures, sandbox
                    // traps, etc. are visible in the log rather than hidden behind
                    // a generic "probe failed" wrapper.
                    anyhow::anyhow!(
                        "heartbeat probe for '{}' failed: {}",
                        skill_for_probe,
                        e.root_cause()
                    )
                })?;
                (result.output, result.elapsed_ms)
            };
            let probe: ProbeSignal = serde_json::from_str(&output).with_context(|| {
                format!(
                    "skill '{skill_for_probe}' returned invalid ProbeSignal: {}",
                    output
                )
            })?;
            Ok::<HeartbeatResult, anyhow::Error>(HeartbeatResult {
                skill_name: skill_for_probe,
                probe,
                elapsed_ms,
            })
        });

        // Apply a hard wall-clock cap so a hung probe cannot block indefinitely.
        let heartbeat = match tokio::time::timeout(HEARTBEAT_PROBE_TIMEOUT, probe_task).await {
            Ok(join_result) => match join_result {
                Ok(Ok(heartbeat)) => heartbeat,
                Ok(Err(e)) => {
                    if let Some(memory) = self.memory_ref() {
                        let err = e.to_string();
                        memory
                            .update_skill_metrics(
                                skill_name,
                                false,
                                0.0,
                                Some(&err),
                                Some(heartbeat_skill_error_category(&err)),
                            )
                            .await;
                    }
                    return Err(e);
                }
                Err(e) => {
                    if let Some(memory) = self.memory_ref() {
                        let err = format!("heartbeat spawn_blocking panicked: {e}");
                        memory
                            .update_skill_metrics(skill_name, false, 0.0, Some(&err), Some("bug"))
                            .await;
                    }
                    return Err(e).context("heartbeat spawn_blocking panicked");
                }
            },
            Err(_elapsed) => {
                tracing::warn!(
                    skill = skill_name,
                    timeout_secs = HEARTBEAT_PROBE_TIMEOUT.as_secs(),
                    "heartbeat probe timed out — skipping escalation this cycle"
                );
                if let Some(memory) = self.memory_ref() {
                    let err = format!(
                        "heartbeat probe timed out after {}s",
                        HEARTBEAT_PROBE_TIMEOUT.as_secs()
                    );
                    memory
                        .update_skill_metrics(skill_name, false, 0.0, Some(&err), Some("timeout"))
                        .await;
                }
                return Ok(HeartbeatOutcome::Silent {
                    skill_name: skill_name.to_string(),
                    elapsed_ms: HEARTBEAT_PROBE_TIMEOUT.as_millis(),
                });
            }
        };
        if let Some(memory) = self.memory_ref() {
            memory
                .update_skill_metrics(skill_name, true, heartbeat.elapsed_ms as f64, None, None)
                .await;
        }
        let mut is_alert = heartbeat.probe.require_llm_attention;

        // -- Monitoring triage: semantic relevance --
        if is_alert && let Some(payload_text) = &heartbeat.probe.event_payload {
            if let Some(user_did) = Self::monitoring_intent_owner_did(payload) {
                if let Some((intent_embedding, threshold)) =
                    self.cached_intent(&user_did, skill_name)
                    && let Some(embedder) = self.embedder()
                {
                    // Degraded-mode policy: cached `intent_embedding` was persisted
                    // by `set_monitoring_intent` / `refresh_intent_cache` under the
                    // primary model. When the embedder is on backup, the fresh
                    // `payload_emb` lives in a different vector space and the
                    // cosine score is garbage — silently swallowing legit alerts
                    // or firing irrelevant ones. Fail-open: skip the semantic
                    // triage entirely and let the alert pass through, mirroring
                    // the existing embedding-error branch below.
                    if embedder.is_degraded() {
                        tracing::debug!(
                            skill = skill_name,
                            user_did,
                            "triage skipped (embedder on backup) — passing alert through"
                        );
                    } else {
                        match embedder.embed(payload_text).await {
                            Ok(payload_emb) => {
                                let similarity = EmbeddingClient::cosine_similarity(
                                    &intent_embedding,
                                    &payload_emb,
                                );
                                if similarity < threshold {
                                    tracing::info!(
                                        skill = skill_name,
                                        user_did,
                                        similarity = format!("{:.3}", similarity),
                                        threshold = format!("{:.3}", threshold),
                                        "triage filtered alert (below relevance threshold)"
                                    );
                                    is_alert = false;
                                } else {
                                    tracing::debug!(
                                        skill = skill_name,
                                        user_did,
                                        similarity = format!("{:.3}", similarity),
                                        "triage passed alert (above relevance threshold)"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    skill = skill_name,
                                    user_did,
                                    error = %e,
                                    "triage embedding failed, passing alert through"
                                );
                            }
                        }
                    }
                }
            } else {
                tracing::debug!(
                    skill = skill_name,
                    "triage skipped (heartbeat payload missing user identity) — passing alert through"
                );
            }
        }

        // Determine cooldown action (Mutex held only for HashMap lookup/insert)
        let severity = heartbeat.probe.severity;
        let action = self.cooldown_action(skill_name, severity, is_alert);

        // Log cooldown decision for observability
        {
            let since_last = self
                .alert_state_lock()
                .get(skill_name)
                .map(|s| s.last_alert_sent.elapsed());
            tracing::debug!(
                skill = skill_name,
                action = ?action,
                since_last_alert_ms = since_last.map(|d| d.as_millis() as u64),
                "heartbeat cooldown decision"
            );
        }

        let outcome = match action {
            CooldownAction::Send => {
                // Design §15.2 Step 3: probes no longer trigger their own LLM
                // call. Record the observation to `observed_events` so the
                // next scheduled heartbeat tick reads it from the queue, and
                // still escalate to the standard alert path for user-visible
                // notifications.
                let auto_cfg = self.get_autonomous_config(skill_name);
                if auto_cfg.enabled && self.within_daily_budget(&auto_cfg) {
                    self.record_probe_event(skill_name, &heartbeat.probe).await;
                }
                let alert_msg = handler.on_alert(skill_name, &heartbeat.probe).await?;
                HeartbeatOutcome::Escalated {
                    skill_name: skill_name.to_string(),
                    alert_message: alert_msg,
                    elapsed_ms: heartbeat.elapsed_ms,
                }
            }
            CooldownAction::Reminder => {
                // Modify probe to indicate ongoing alert
                let mut reminder_probe = heartbeat.probe.clone();
                reminder_probe.event_payload = Some(format!(
                    "[STILL ONGOING] {}",
                    heartbeat
                        .probe
                        .event_payload
                        .as_deref()
                        .unwrap_or("alert continues")
                ));
                let auto_cfg = self.get_autonomous_config(skill_name);
                if auto_cfg.enabled && self.within_daily_budget(&auto_cfg) {
                    self.record_probe_event(skill_name, &reminder_probe).await;
                }
                let alert_msg = handler.on_alert(skill_name, &reminder_probe).await?;
                HeartbeatOutcome::Escalated {
                    skill_name: skill_name.to_string(),
                    alert_message: alert_msg,
                    elapsed_ms: heartbeat.elapsed_ms,
                }
            }
            CooldownAction::Resolved => {
                let mut resolved_probe = heartbeat.probe.clone();
                resolved_probe.require_llm_attention = true;
                resolved_probe.event_payload = Some(format!(
                    "\u{2705} [RESOLVED] {} — {}",
                    skill_name,
                    heartbeat
                        .probe
                        .event_payload
                        .as_deref()
                        .unwrap_or("issue resolved"),
                ));
                let alert_msg = handler.on_alert(skill_name, &resolved_probe).await?;
                HeartbeatOutcome::Escalated {
                    skill_name: skill_name.to_string(),
                    alert_message: alert_msg,
                    elapsed_ms: heartbeat.elapsed_ms,
                }
            }
            CooldownAction::Suppress => {
                handler.on_silent(skill_name, &heartbeat.probe);
                HeartbeatOutcome::Suppressed {
                    skill_name: skill_name.to_string(),
                    elapsed_ms: heartbeat.elapsed_ms,
                }
            }
            CooldownAction::Silent => {
                handler.on_silent(skill_name, &heartbeat.probe);
                HeartbeatOutcome::Silent {
                    skill_name: skill_name.to_string(),
                    elapsed_ms: heartbeat.elapsed_ms,
                }
            }
        };

        // Notify handler of the observation for proactive learning (always)
        handler.on_observation(skill_name, &heartbeat.probe, is_alert);

        Ok(outcome)
    }

    /// Record a probe observation into the `observed_events` queue so the
    /// next scheduled heartbeat tick can fold it into its prompt. Design
    /// §15.2 Step 3: probes are context, not LLM callers.
    ///
    /// No-ops silently (with a single debug log) when no memory handle is
    /// attached or the insert fails — the probe path must never panic on
    /// observability plumbing.
    async fn record_probe_event(&self, skill_name: &str, probe: &ProbeSignal) {
        let Some(memory) = self.memory_ref() else {
            tracing::debug!(
                skill = skill_name,
                "[ObservedEvents] memory handle not attached; skipping record"
            );
            return;
        };
        let severity = match probe.severity {
            AlertSeverity::Info => "info",
            AlertSeverity::Warning => "warn",
            AlertSeverity::Critical => "alert",
        };
        let summary = probe
            .event_payload
            .as_deref()
            .unwrap_or("(no event payload)")
            .to_string();
        let payload = probe.probe_data.as_ref();
        if let Err(e) = memory
            .record_observed_event(skill_name, severity, &summary, payload)
            .await
        {
            tracing::warn!(
                skill = skill_name,
                error = %e,
                "[ObservedEvents] failed to record probe observation"
            );
        }
    }

    /// Register all cron-scheduled skills with tokio-cron-scheduler.
    ///
    /// This reads cron_schedule from each loaded SkillCapability and creates
    /// recurring jobs that call `run_heartbeat` on the configured schedule.
    pub async fn start_scheduler(
        self: Arc<Self>,
        handler: Arc<dyn HeartbeatHandler>,
    ) -> Result<tokio_cron_scheduler::JobScheduler> {
        self.start_scheduler_with_accounts(handler, &[]).await
    }

    /// Start the cron scheduler. For multi-account skills, creates per-account jobs.
    pub async fn start_scheduler_with_accounts(
        self: Arc<Self>,
        handler: Arc<dyn HeartbeatHandler>,
        account_ids: &[String],
    ) -> Result<tokio_cron_scheduler::JobScheduler> {
        use tokio_cron_scheduler::JobScheduler;

        let sched = JobScheduler::new()
            .await
            .context("creating job scheduler")?;

        // account_id → guids of its multi-account jobs, seeded here so the
        // runtime add path (`register_account_cron_jobs`) is idempotent
        // against startup-registered accounts and removal can target them.
        let mut startup_account_jobs: HashMap<String, Vec<uuid::Uuid>> = HashMap::new();
        // NO-account fallback jobs a zero-account startup registers for
        // multi-account skills; drained + removed by the first runtime
        // account registration (parent Codex P2 — double-probe fix).
        let mut startup_fallback_guids: Vec<uuid::Uuid> = Vec::new();

        let skills = self.sandbox_ref().loaded_skills();
        for cap in skills {
            // Skip cron registration for disabled skills (avoids 288+ no-op logs/day).
            // Runtime enabling requires a service restart — see helper docs.
            if !should_register_cron_for_skill(&cap, self.settings_ref()) {
                if cap.cron_schedule.is_some() {
                    tracing::debug!(
                        skill = %cap.skill_name,
                        "Skipping cron registration for disabled skill"
                    );
                }
                continue;
            }
            if let Some(ref cron_expr) = cap.cron_schedule {
                // Defensive guard: reject sub-30s cron expressions at
                // registration time. See `trigger::validate_cron_interval`.
                let trigger = Trigger::cron(cron_expr.clone(), &cap.skill_name)?;
                // Multi-account: create per-account jobs
                let payloads: Vec<(serde_json::Value, String)> =
                    if cap.supports_multi_account && !account_ids.is_empty() {
                        account_ids
                            .iter()
                            .map(|acct| {
                                let mut p = cap
                                    .default_payload
                                    .clone()
                                    .unwrap_or_else(|| serde_json::json!({}));
                                // The heartbeat runner resolves this account
                                // into per-call Google auth overrides before
                                // entering the sandbox.
                                p["account"] = serde_json::json!(acct);
                                let flag_key = format!("{}:{}", cap.skill_name, acct);
                                (p, flag_key)
                            })
                            .collect()
                    } else {
                        let p = cap
                            .default_payload
                            .clone()
                            .unwrap_or_else(|| serde_json::json!({}));
                        vec![(p, cap.skill_name.clone())]
                    };

                for (default_payload, flag_key) in payloads {
                    // Use the validated expression from the Trigger so we never
                    // pass an unvalidated string to the scheduler.
                    let cron = trigger
                        .as_cron_str()
                        .expect("Trigger::cron always yields a Cron variant")
                        .to_string();

                    let job = self.build_heartbeat_cron_job(
                        handler.clone(),
                        cap.skill_name.clone(),
                        &cron,
                        default_payload,
                        &flag_key,
                    )?;
                    // Remember which guid belongs to which account so a
                    // runtime account removal can drop exactly its jobs
                    // (multi-account flag keys are "<skill>:<account>").
                    if let Some(account) = flag_key.strip_prefix(&format!("{}:", cap.skill_name)) {
                        startup_account_jobs
                            .entry(account.to_string())
                            .or_default()
                            .push(job.guid());
                    } else if cap.supports_multi_account {
                        // Zero-account startup: this is the NO-account
                        // fallback job for a multi-account skill — record it
                        // so the first runtime account add can remove it
                        // instead of double-probing the new default account.
                        startup_fallback_guids.push(job.guid());
                    }

                    sched.add(job).await.context("adding job to scheduler")?;
                } // end per-payload loop
            }
        }

        sched.start().await.context("starting scheduler")?;
        self.mark_scheduler_started();
        // Capture the runtime-add context: accounts added AFTER startup get
        // their per-account cron jobs through `register_account_cron_jobs`
        // against this same scheduler + handler (multi-mailbox Slice 1 W1).
        // Startup slots all carry generation 0; runtime reservations start at 1.
        let account_jobs = startup_account_jobs
            .into_iter()
            .map(|(account, guids)| {
                (
                    account,
                    crate::chronos::AccountJobsSlot {
                        generation: 0,
                        guids,
                    },
                )
            })
            .collect();
        *self.runtime_cron.lock() = Some(crate::chronos::RuntimeCronContext {
            sched: sched.clone(),
            handler,
            account_jobs,
            next_generation: 1,
            multi_account_fallback_guids: startup_fallback_guids,
        });

        Ok(sched)
    }

    /// Build ONE heartbeat cron job (per skill, or per skill:account for
    /// multi-account skills). Shared by the startup registration loop and the
    /// runtime `register_account_cron_jobs` path so both spawn IDENTICAL jobs.
    fn build_heartbeat_cron_job(
        self: &Arc<Self>,
        handler: Arc<dyn HeartbeatHandler>,
        skill_name: String,
        cron: &str,
        default_payload: serde_json::Value,
        flag_key: &str,
    ) -> Result<tokio_cron_scheduler::Job> {
        use tokio_cron_scheduler::Job;

        let engine = self.clone();
        // Per-skill (or per-skill:account) execution lock
        let running_flag = engine.running_flag(flag_key);
        let job_skill_name = skill_name.clone();

        Job::new_async(cron, move |_uuid, _lock| {
            let engine = engine.clone();
            let handler = handler.clone();
            let skill_name = job_skill_name.clone();
            let payload = default_payload.clone();
            let running_flag = running_flag.clone();
            Box::pin(async move {
                // Try to acquire: if already running, skip this tick
                if running_flag
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    tracing::debug!(
                        skill = %skill_name,
                        "skipping overlapping heartbeat (previous still running)"
                    );
                    return;
                }
                // RAII guard: ensure running_flag is released even on panic
                struct RunningReset(Arc<std::sync::atomic::AtomicBool>);
                impl Drop for RunningReset {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::SeqCst);
                    }
                }
                let _running_guard = RunningReset(running_flag.clone());

                let result = engine
                    .run_heartbeat(&skill_name, &payload, handler.as_ref())
                    .await;

                match result {
                    Ok(outcome) => {
                        let outcome_label = heartbeat_outcome_label(&outcome);
                        tracing::info!(
                            skill = %skill_name,
                            outcome = outcome_label,
                            "[Chronos] Heartbeat completed"
                        );
                        if let HeartbeatOutcome::Escalated { alert_message, .. } = outcome {
                            tracing::info!(
                                skill = %skill_name,
                                alert = %alert_message,
                                "[Chronos] Alert escalated"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::info!(
                            skill = %skill_name,
                            outcome = "error",
                            "[Chronos] Heartbeat completed"
                        );
                        tracing::error!(
                            error = %e,
                            skill = %skill_name,
                            "[Chronos] Heartbeat error"
                        );
                    }
                }
            })
        })
        .with_context(|| format!("creating cron job for '{skill_name}'"))
    }

    /// Register the per-account cron jobs for ONE runtime-added mail account
    /// against the LIVE scheduler (multi-mailbox Slice 1 W1). Mirrors exactly
    /// what `start_scheduler_with_accounts` does per account at startup: one
    /// job per enabled multi-account cron skill, with the account baked into
    /// the payload and a `<skill>:<account>` execution lock.
    ///
    /// Idempotent per account: a second call (or a concurrent one — the
    /// account key is reserved under the lock before any await) returns
    /// `Ok(0)`. Returns the number of jobs added, or `Ok(0)` when no
    /// scheduler is running in this process (deployments without cron
    /// skills; the startup registration loop covers the next restart).
    pub async fn register_account_cron_jobs(self: &Arc<Self>, account_id: &str) -> Result<usize> {
        // Phase 1 (sync, under the lock): dup-guard + snapshot the scheduler
        // and handler. The reservation (empty guids under a FRESH generation)
        // makes concurrent adds for the same account collapse to one winner,
        // and the generation lets the record step detect a remove-and-readd
        // that happened while our adds were in flight (Codex round 2).
        // parking_lot MutexGuard is !Send, so nothing is held across awaits.
        let (sched, handler, generation) = {
            let mut guard = self.runtime_cron.lock();
            let Some(ctx) = guard.as_mut() else {
                tracing::info!(
                    account = %account_id,
                    "[Chronos] no live scheduler — per-account cron jobs will register on next startup"
                );
                return Ok(0);
            };
            if ctx.account_jobs.contains_key(account_id) {
                tracing::debug!(
                    account = %account_id,
                    "[Chronos] per-account cron jobs already registered — skipping"
                );
                return Ok(0);
            }
            let generation = ctx.next_generation;
            ctx.next_generation += 1;
            ctx.account_jobs.insert(
                account_id.to_string(),
                crate::chronos::AccountJobsSlot {
                    generation,
                    guids: Vec::new(),
                },
            );
            (ctx.sched.clone(), ctx.handler.clone(), generation)
        };

        // Phase 2: build one job per enabled multi-account cron skill — the
        // same filter chain as the startup loop (disabled-skill gate, cron
        // validation, per-account payload + flag key).
        let mut built: Vec<tokio_cron_scheduler::Job> = Vec::new();
        let build_result: Result<()> = (|| {
            for cap in self.sandbox_ref().loaded_skills() {
                if !should_register_cron_for_skill(&cap, self.settings_ref()) {
                    continue;
                }
                let Some(ref cron_expr) = cap.cron_schedule else {
                    continue;
                };
                if !cap.supports_multi_account {
                    continue;
                }
                let trigger = Trigger::cron(cron_expr.clone(), &cap.skill_name)?;
                let cron = trigger
                    .as_cron_str()
                    .expect("Trigger::cron always yields a Cron variant")
                    .to_string();
                let mut payload = cap
                    .default_payload
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                payload["account"] = serde_json::json!(account_id);
                let flag_key = format!("{}:{}", cap.skill_name, account_id);
                built.push(self.build_heartbeat_cron_job(
                    handler.clone(),
                    cap.skill_name.clone(),
                    &cron,
                    payload,
                    &flag_key,
                )?);
            }
            Ok(())
        })();
        if let Err(e) = build_result {
            // Roll back OUR reservation so a later retry can re-register —
            // generation-checked so a concurrent remove-and-readd's fresh
            // slot is never deleted by this stale failure path.
            if let Some(ctx) = self.runtime_cron.lock().as_mut()
                && ctx
                    .account_jobs
                    .get(account_id)
                    .is_some_and(|slot| slot.generation == generation)
            {
                ctx.account_jobs.remove(account_id);
            }
            return Err(e);
        }

        // Phase 3: add to the live scheduler, then record the guids.
        let mut guids = Vec::with_capacity(built.len());
        let mut add_err: Option<anyhow::Error> = None;
        for job in built {
            let guid = job.guid();
            match sched.add(job).await {
                Ok(_) => guids.push(guid),
                Err(e) => {
                    add_err = Some(anyhow::anyhow!(e).context(format!(
                        "adding runtime cron job for account '{account_id}'"
                    )));
                    break;
                }
            }
        }
        let added = guids.len();
        let outcome = {
            let mut guard = self.runtime_cron.lock();
            record_account_job_guids(&mut guard, account_id, generation, guids, add_err.is_some())
        };
        if let RecordJobsOutcome::RolledBack(orphans) = outcome {
            // A concurrent unregister removed our reservation while the adds
            // were in flight — possibly followed by a re-register that now
            // owns the slot under a NEWER generation (Codex rounds 1-2,
            // HIGH): the just-added jobs must come back OUT of the scheduler;
            // recording them is impossible (slot gone or not ours) and
            // leaving them would probe a removed account / double-probe.
            for guid in &orphans {
                if let Err(e) = sched.remove(guid).await {
                    tracing::warn!(
                        account = %account_id,
                        job = %guid,
                        error = %e,
                        "[Chronos] failed to roll back cron job after unregister race"
                    );
                }
            }
            tracing::info!(
                account = %account_id,
                jobs = orphans.len(),
                "[Chronos] runtime cron registration rolled back (account removed concurrently)"
            );
            return match add_err {
                Some(e) => Err(e),
                None => Ok(0),
            };
        }
        match add_err {
            // Partial add: keep what landed registered (the guids are
            // recorded for removal); surface the error to the caller.
            Some(e) if added > 0 => Err(e.context(format!(
                "partially registered {added} cron job(s) before the failure"
            ))),
            Some(e) => Err(e),
            None => {
                if added > 0 {
                    tracing::info!(
                        account = %account_id,
                        jobs = added,
                        "[Chronos] per-account cron jobs registered at runtime"
                    );
                    // First-account transition after a zero-account startup:
                    // the NO-account fallback jobs would now double-probe the
                    // new default account every tick (parent Codex P2). Drain
                    // them under the lock (idempotent — later adds see an
                    // empty list) and remove them from the scheduler.
                    let fallback = {
                        let mut guard = self.runtime_cron.lock();
                        guard
                            .as_mut()
                            .map(|ctx| std::mem::take(&mut ctx.multi_account_fallback_guids))
                            .unwrap_or_default()
                    };
                    if !fallback.is_empty() {
                        let mut removed = 0usize;
                        for guid in &fallback {
                            match sched.remove(guid).await {
                                Ok(()) => removed += 1,
                                Err(e) => tracing::warn!(
                                    job = %guid,
                                    error = %e,
                                    "[Chronos] failed to remove no-account fallback cron job"
                                ),
                            }
                        }
                        tracing::info!(
                            removed,
                            total = fallback.len(),
                            "[Chronos] removed no-account fallback cron jobs after first account add"
                        );
                    }
                }
                Ok(added)
            }
        }
    }

    /// Remove the per-account cron jobs registered for `account_id` (runtime
    /// account removal — the mirror of `register_account_cron_jobs`).
    /// Returns the number of jobs removed; `Ok(0)` when none were registered.
    pub async fn unregister_account_cron_jobs(&self, account_id: &str) -> Result<usize> {
        let (sched, guids) = {
            let mut guard = self.runtime_cron.lock();
            let Some(ctx) = guard.as_mut() else {
                return Ok(0);
            };
            match ctx.account_jobs.remove(account_id) {
                Some(slot) => (ctx.sched.clone(), slot.guids),
                None => return Ok(0),
            }
        };
        let mut removed = 0usize;
        for guid in &guids {
            match sched.remove(guid).await {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!(
                    account = %account_id,
                    job = %guid,
                    error = %e,
                    "[Chronos] failed to remove per-account cron job"
                ),
            }
        }
        if removed > 0 {
            tracing::info!(
                account = %account_id,
                jobs = removed,
                "[Chronos] per-account cron jobs removed"
            );
        }
        Ok(removed)
    }
}
