//! Deferred Intent System — schedule future actions triggered by time or events.
//!
//! The LLM is stateless: it can only act when the user sends a message. This module
//! gives it the ability to schedule future actions via `schedule_intent`, triggered by
//! time delays or system events (e.g. new-email notifications).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::Mutex;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use crate::i18n::tf;
use crate::memory::MemoryHandle;
use crate::orchestrator::Orchestrator;
use crate::telegram::{
    markdown_to_telegram_html, pin_message, send_html_to_topic, send_reminder_html_to_topic,
};
use crate::topic_router::TopicKind;
use crate::utils::truncate_str;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of active (pending) intents per user.
pub const MAX_ACTIVE_INTENTS: i64 = 50;

/// Maximum number of intents a user can create per day.
pub const MAX_DAILY_INTENTS: i64 = 100;

/// Maximum intent lifetime: 365 days in seconds.
pub const MAX_INTENT_LIFETIME_SECS: i64 = 365 * 24 * 60 * 60;

/// Minimum delay for time-based intents: 10 seconds.
const MIN_DELAY_SECS: i64 = 10;

/// Bounded allowance for a recurring start overtaken by model/tool latency.
const MAX_RECURRING_START_SKEW_SECS: i64 = 5 * 60;

/// Minimum interval for recurring intents: 30 seconds.
const MIN_RECURRING_INTERVAL: i64 = 30;

/// Seconds in one day (for time arithmetic).
const SECS_PER_DAY: i64 = 86_400;

/// Maximum chain depth for bounded callback recursion.
pub const MAX_CHAIN_DEPTH: i32 = 5;

/// Recurring intent expiry: 30 days (much longer than one-shot TTL).
const RECURRING_EXPIRY_SECS: i64 = 30 * 24 * 60 * 60;

/// Polling interval for time-based intents.
const TIME_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Interval for expiry cleanup.
const EXPIRY_CLEANUP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Maximum context text length in characters.
const MAX_CONTEXT_LEN: usize = 2000;

/// Maximum retries before an intent is moved to dead_letter status.
const MAX_INTENT_RETRIES: i32 = 3;

/// Prefix used to identify callback-originated messages (anti-recursion).
pub const CALLBACK_CONTEXT_PREFIX: &str = "[INTENT_CALLBACK]";

// ── Event type ───────────────────────────────────────────────────────────────

/// An event that can trigger deferred intents.
#[derive(Debug, Clone)]
pub struct IntentEvent {
    pub key: String,
    pub context: Option<String>,
    /// The DID of the user who owns this event. Used to scope intent matching
    /// so one tenant cannot trigger another tenant's event-based intents.
    pub user_did: String,
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Background engine that executes deferred intents when their triggers fire.
pub struct DeferredIntentEngine {
    memory: MemoryHandle,
    orchestrator: Arc<Orchestrator>,
    #[allow(dead_code)]
    admin_chat_id: i64,
    event_rx: Receiver<IntentEvent>,
    poll_interval: Duration,
    expiry_interval: Duration,
}

impl DeferredIntentEngine {
    pub fn new(
        memory: MemoryHandle,
        orchestrator: Arc<Orchestrator>,
        admin_chat_id: i64,
        event_rx: Receiver<IntentEvent>,
    ) -> Self {
        Self {
            memory,
            orchestrator,
            admin_chat_id,
            event_rx,
            poll_interval: TIME_POLL_INTERVAL,
            expiry_interval: EXPIRY_CLEANUP_INTERVAL,
        }
    }

    /// Override the time-poll interval (default 30s). Useful for testing.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Override the expiry-cleanup interval (default 10min). Useful for testing.
    pub fn with_expiry_interval(mut self, interval: Duration) -> Self {
        self.expiry_interval = interval;
        self
    }

    /// Start background tasks. Returns join handles for the 3 spawned tasks.
    pub fn start(self, bot: teloxide::Bot) -> Vec<JoinHandle<()>> {
        let bot3 = bot.clone();
        let orch3 = self.orchestrator.clone();
        let memory1 = self.memory.clone();
        let memory2 = self.memory.clone();
        let memory3 = self.memory.clone();
        let orch1 = self.orchestrator.clone();
        let orch2 = self.orchestrator;
        let bot1 = bot.clone();
        let bot2 = bot;
        let poll_interval = self.poll_interval;
        let expiry_interval = self.expiry_interval;

        // Shared guard: prevents reentrant execution of the same intent.
        // If an intent callback fires more events (e.g. new-mail events),
        // those events could match the same intent again, creating a storm.
        // This HashSet tracks intent IDs currently being executed across both tasks.
        let executing_guard: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));

        // Task 1: Time poller — concurrent execution with bounded parallelism
        let executing1 = executing_guard.clone();
        let time_handle = tokio::spawn(async move {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(5));
            let mut interval = tokio::time::interval(poll_interval);
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().timestamp();
                match memory1.get_due_time_intents(now).await {
                    Ok(intents) => {
                        for intent in intents {
                            // Skip if this intent is already being executed
                            {
                                let mut guard = executing1.lock();
                                if !guard.insert(intent.id) {
                                    tracing::debug!(
                                        id = intent.id,
                                        "Skipping reentrant time intent execution"
                                    );
                                    continue;
                                }
                            }
                            let sem = semaphore.clone();
                            let mem = memory1.clone();
                            let orch = orch1.clone();
                            let b = bot1.clone();
                            let exec_guard = executing1.clone();
                            let intent_id = intent.id;
                            tokio::spawn(async move {
                                let _permit = match sem.acquire().await {
                                    Ok(p) => p,
                                    Err(e) => {
                                        tracing::warn!(error = %e, "Intent semaphore closed, stopping execution");
                                        exec_guard.lock().remove(&intent_id);
                                        return;
                                    }
                                };
                                execute_intent(&mem, &orch, &b, &intent).await;
                                exec_guard.lock().remove(&intent_id);
                            });
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "Failed to check due time intents");
                    }
                }
            }
        });

        // Task 2: Event matcher (channel-driven)
        let executing2 = executing_guard;
        let event_handle = tokio::spawn(async move {
            let mut event_rx = self.event_rx;
            while let Some(event) = event_rx.recv().await {
                match memory2
                    .get_matching_event_intents(&event.key, &event.user_did)
                    .await
                {
                    Ok(intents) => {
                        for intent in intents {
                            // Skip if this intent is already being executed (reentrant guard)
                            {
                                let mut guard = executing2.lock();
                                if !guard.insert(intent.id) {
                                    tracing::debug!(id = intent.id, event_key = %event.key, "Skipping reentrant event intent execution");
                                    continue;
                                }
                            }
                            execute_intent(&memory2, &orch2, &bot2, &intent).await;
                            executing2.lock().remove(&intent.id);
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, event_key = %event.key, "Failed to match event intents");
                    }
                }
            }
        });

        // Task 3: Expiry cleanup
        let expiry_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(expiry_interval);
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().timestamp();
                match memory3.expire_old_intents(now).await {
                    Ok(items) if !items.is_empty() => {
                        tracing::info!(expired = items.len(), "Cleaned up expired intents");
                        // The owner used to learn nothing: a reminder that came due
                        // while the host was down was cancelled an hour later in
                        // silence. Send one digest through the reminder channel.
                        notify_missed_reminders(&orch3, &bot3, &items).await;
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "Failed to expire old intents");
                    }
                    _ => {}
                }
            }
        });

        vec![time_handle, event_handle, expiry_handle]
    }
}

// ── Execution ────────────────────────────────────────────────────────────────

/// One owner notice listing reminders that expired unfired (missed while
/// offline). Web push first, Telegram fallback — the same channel a fired
/// reminder uses.
async fn notify_missed_reminders(
    orchestrator: &Arc<Orchestrator>,
    bot: &teloxide::Bot,
    items: &[(i64, String, String)],
) {
    let locale = orchestrator.settings.display_language();
    let text = missed_reminders_text(&locale, items);
    if text.is_empty() {
        return;
    }
    let chat_id = crate::telegram::admin_dm_chat(0);
    if let Some((owner_did, web_notice)) = crate::telegram::record_owner_web_notice_without_push(
        &orchestrator.memory,
        chat_id.0,
        &text,
    )
    .await
        && crate::admin::try_owner_web_push_notice(&orchestrator.memory, &owner_did, &web_notice)
            .await
    {
        return;
    }
    let html = markdown_to_telegram_html(&text);
    if let Err(e) = send_html_to_topic(bot, chat_id, TopicKind::Notif, &html).await {
        tracing::warn!(error = %e, "Failed to send missed-reminder digest");
    }
}

/// Pure: the digest text ("" when nothing is worth telling).
pub fn missed_reminders_text(locale: &str, items: &[(i64, String, String)]) -> String {
    let usable: Vec<&(i64, String, String)> = items
        .iter()
        .filter(|(_, _, context)| !context.trim().is_empty())
        .collect();
    let mut lines: Vec<String> = usable
        .iter()
        .take(10)
        .map(|(id, _, context)| {
            let one_line: String = context.trim().chars().take(80).collect();
            format!("- {one_line} (#{id})")
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    // Only non-blank leftovers count as "more".
    let extra = usable.len().saturating_sub(lines.len());
    if extra > 0 {
        lines.push(if locale.starts_with("zh") {
            format!("- …还有 {extra} 条")
        } else {
            format!("- …and {extra} more")
        });
    }
    let head = if locale.starts_with("zh") {
        "⏰ 这些提醒在我离线时到期了，没能按时提醒你："
    } else {
        "⏰ These reminders came due while I was offline and were missed:"
    };
    format!("{head}\n{}", lines.join("\n"))
}

async fn execute_intent(
    memory: &MemoryHandle,
    orchestrator: &Arc<Orchestrator>,
    bot: &teloxide::Bot,
    intent: &crate::memory::DeferredIntent,
) {
    // Clamp to admin DM: a deferred_intent row created from the old linked
    // group carries that group id; forum routing is gone (Codex R3).
    let chat_id = crate::telegram::admin_dm_chat(intent.chat_id);
    let is_recurring = intent.interval_seconds.is_some();

    // Determine if this is the last fire for a recurring intent with max_fires
    let is_last_fire = is_recurring
        && intent
            .max_fires
            .is_some_and(|max| (intent.fire_count + 1) as i64 >= max);

    // ── Status transition (BEFORE callback execution) ──────────────────
    if is_recurring && !is_last_fire {
        // Atomic fire + advance: keeps status='active', sets next trigger time.
        // If the process crashes during the callback, the poller will naturally
        // re-discover this intent when trigger_at_unix <= now on the next tick.
        let interval = intent.interval_seconds.unwrap(); // safe: is_recurring checked
        let now = chrono::Utc::now().timestamp();
        let next_trigger = next_windowed_trigger(now, intent, interval);
        let next_expires = now + RECURRING_EXPIRY_SECS;
        if let Err(e) = memory
            .fire_and_advance_recurring(intent.id, next_trigger, next_expires)
            .await
        {
            match memory
                .increment_intent_retry(intent.id, MAX_INTENT_RETRIES)
                .await
            {
                Ok(count) => tracing::warn!(
                    id = intent.id, retry_count = count, error = %e,
                    "Failed to fire_and_advance recurring intent; retry count incremented"
                ),
                Err(re) => tracing::error!(
                    id = intent.id, error = %e, retry_error = %re,
                    "Failed to fire_and_advance AND failed to increment retry"
                ),
            }
            return;
        }
        tracing::info!(
            id = intent.id,
            interval = interval,
            next_trigger = next_trigger,
            fire_count = intent.fire_count + 1,
            group_key = ?intent.group_key,
            "Recurring intent fired and advanced to next trigger"
        );
    } else {
        // One-shot intent OR last fire of recurring: mark as 'fired' (terminal state).
        // This matches the original at-most-once semantics for non-recurring intents.
        if let Err(e) = memory.mark_intent_fired(intent.id).await {
            match memory
                .increment_intent_retry(intent.id, MAX_INTENT_RETRIES)
                .await
            {
                Ok(count) => tracing::warn!(
                    id = intent.id, retry_count = count, error = %e,
                    "Failed to mark intent fired; retry count incremented"
                ),
                Err(re) => tracing::error!(
                    id = intent.id, error = %e, retry_error = %re,
                    "Failed to mark fired AND failed to increment retry"
                ),
            }
            return;
        }
        if is_last_fire {
            tracing::info!(
                id = intent.id,
                fire_count = intent.fire_count + 1,
                max_fires = ?intent.max_fires,
                group_key = ?intent.group_key,
                "Recurring intent completed all fires (terminal)"
            );
        }
    }

    match intent.execution_mode.as_str() {
        "notify" => {
            // Fallback: if context_text is empty (LLM didn't fill it), use a default
            // message with the intent ID so the notification doesn't fail with empty text.
            let locale = orchestrator.settings.display_language();
            let text = if intent.context_text.trim().is_empty() {
                reminder_fallback_text(&locale, intent.id)
            } else {
                intent.context_text.clone()
            };
            let web_text =
                crate::telegram::owner_web_reminder_notice_text(&text, intent.id, &locale);
            if let Some((owner_did, web_notice)) =
                crate::telegram::record_owner_web_notice_without_push(
                    &orchestrator.memory,
                    chat_id.0,
                    &web_text,
                )
                .await
            {
                let web_delivered = crate::admin::try_owner_web_push_notice(
                    &orchestrator.memory,
                    &owner_did,
                    &web_notice,
                )
                .await;
                if web_delivered {
                    tracing::info!(
                        id = intent.id,
                        "[Delivery] Web Push delivered reminder; suppressing Telegram fallback"
                    );
                    return;
                }
            }
            let html = markdown_to_telegram_html(&text);
            match send_reminder_html_to_topic(bot, chat_id, &html, intent.id, &locale).await {
                Ok(msg_id) => {
                    if intent.pin_on_fire {
                        pin_message(bot, chat_id, msg_id).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(id = intent.id, error = %e, "Failed to send intent notification");
                }
            }
        }
        "llm_callback" => {
            let work_item = crate::assistant_work_items::build_deferred_intent_work_item(intent);
            match orchestrator
                .memory
                .store_assistant_work_item(&work_item)
                .await
            {
                Ok(work_item_id) => {
                    tracing::info!(
                        id = intent.id,
                        work_item_id,
                        mode = %intent.execution_mode,
                        "Enqueued deferred intent callback as assistant work item"
                    );
                    if let Err(e) = orchestrator
                        .process_deferred_intent_work_items(&intent.user_did, intent.chat_id, 1)
                        .await
                    {
                        tracing::warn!(
                            id = intent.id,
                            work_item_id,
                            error = %e,
                            "Deferred-intent work item left queued after immediate processing attempt"
                        );
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let is_timeout = err_str.contains("timed out");

                    if is_timeout && intent.retry_count < MAX_INTENT_RETRIES {
                        let backoff_secs = 30 * (1 << intent.retry_count.min(4));
                        let now = chrono::Utc::now().timestamp();
                        let new_trigger = now + backoff_secs;
                        let new_expires = now + backoff_secs + 3600;

                        match memory
                            .increment_intent_retry(intent.id, MAX_INTENT_RETRIES)
                            .await
                        {
                            Ok(count) => {
                                if intent.interval_seconds.is_none()
                                    && let Err(re) = memory
                                        .reschedule_intent(
                                            intent.id,
                                            &intent.user_did,
                                            new_trigger,
                                            new_expires,
                                        )
                                        .await
                                {
                                    tracing::warn!(id = intent.id, error = %re, "Failed to reschedule intent for retry");
                                }
                                tracing::warn!(
                                    id = intent.id,
                                    retry_count = count,
                                    backoff_secs,
                                    next_trigger = new_trigger,
                                    "Deferred-intent work item enqueue timed out — rescheduled ({}/{})",
                                    count,
                                    MAX_INTENT_RETRIES
                                );
                            }
                            Err(re) => {
                                tracing::error!(
                                    id = intent.id, error = %e, retry_error = %re,
                                    "Failed to increment retry count for timed-out deferred-intent enqueue"
                                );
                            }
                        }
                    } else {
                        let locale = orchestrator.settings.display_language();
                        let err_msg = scheduled_action_error_text(
                            &locale,
                            is_timeout,
                            intent.retry_count,
                            &err_str,
                        );
                        if crate::telegram::record_owner_web_notice_if_push_delivered(
                            &orchestrator.memory,
                            chat_id.0,
                            &err_msg,
                        )
                        .await
                        {
                            return;
                        }
                        let html_err = markdown_to_telegram_html(&err_msg);
                        if let Err(send_err) =
                            send_html_to_topic(bot, chat_id, TopicKind::Notif, &html_err).await
                        {
                            tracing::warn!(error = %send_err, "Failed to send error notification for deferred-intent callback");
                        }
                    }
                }
            }
        }
        other => {
            tracing::warn!(id = intent.id, mode = %other, "Unknown execution mode");
        }
    }
    // No post-callback reschedule needed — already handled atomically above.
}

// ── Channel-B proactive text builders (i18n Task 3) ──────────────────────────
// These are SYSTEM-authored fixed strings (errors/notifications). They are
// rendered in the owner's `display_language` setting, NOT the language of any
// user message (spec §2 Channel B / §6 classify-by-author). Kept as pure
// functions so they are unit-testable on the real entry path: a regression that
// reintroduces a hardcoded literal fails the test.

/// Fallback text for a `notify` intent whose `context_text` is empty.
pub(crate) fn reminder_fallback_text(locale: &str, intent_id: i64) -> String {
    tf("reminder.fallback", locale, &intent_id.to_string())
}

/// Notification text when a scheduled (deferred-intent) callback fails or times out.
pub(crate) fn scheduled_action_error_text(
    locale: &str,
    is_timeout: bool,
    retry_count: i32,
    err: &str,
) -> String {
    if is_timeout {
        // Localize only the prefix (retry count is a safe integer); concatenate
        // the raw error AFTER it. The error is never routed through placeholder
        // substitution, so an error string containing a literal `{}` cannot be
        // corrupted by a re-scanning formatter.
        format!(
            "{}{}",
            tf(
                "reminder.scheduled_timed_out",
                locale,
                &retry_count.to_string()
            ),
            truncate_str(err, 150)
        )
    } else {
        tf("reminder.scheduled_failed", locale, truncate_str(err, 200))
    }
}

/// Returns true if a callback reply should be suppressed (not sent to user).
/// Empty replies or replies containing `[NO_ACTION]` are suppressed.
/// The LLM sometimes wraps `[NO_ACTION]` with extra text (e.g.,
/// "I checked. [NO_ACTION]"), so we use `contains` instead of strict equality.
///
/// Legacy suppression check. Primary mechanism is now the `notify_user` tool:
/// callback replies are never sent directly — only explicit `notify_user` calls
/// produce user-visible notifications. Kept for backward compatibility
/// (L1 history suppression in orchestrator still uses this).
#[allow(dead_code)] // Used by intent_tests; telegram.rs replaced with inline [NO_ACTION] strip
pub(crate) fn should_suppress_callback(reply: &str) -> bool {
    let trimmed = reply.trim();
    trimmed.is_empty() || trimmed.contains("[NO_ACTION]")
}

// ── Time parsing ─────────────────────────────────────────────────────────────

/// Map a common US timezone abbreviation to a fixed UTC-offset string that
/// `resolve_tz_offset` already understands.
///
/// Unlike IANA names (which depend on the *target* calendar date to pick the
/// right DST arm), abbreviations name a single fixed offset by construction:
/// "EST" always means UTC-5, "EDT" always means UTC-4, etc. Mapping to the
/// offset directly avoids the DST-rollover pitfall where resolving via IANA
/// at call time uses `Utc::now()`'s current offset, which may be wrong for a
/// trigger date on the other side of a DST boundary.
///
/// Kept deliberately strict — only the 9 abbreviations users most commonly
/// write inline in natural language. Ambiguous ones (IST = India OR Israel OR
/// Ireland, BST = British OR Brazil, CST = US OR China OR Cuba) are mapped
/// to their **US** interpretation on purpose; that matches the error message
/// hint, and users outside the US have IANA names and numeric offsets available.
///
/// Returns `None` when `abbr` is not in the allowlist — caller should keep the
/// original strict error.
fn normalize_tz_abbreviation(abbr: &str) -> Option<&'static str> {
    match abbr.to_ascii_uppercase().as_str() {
        "PDT" => Some("-07:00"),
        "PST" => Some("-08:00"),
        "MDT" => Some("-06:00"),
        "MST" => Some("-07:00"),
        "CDT" => Some("-05:00"),
        "CST" => Some("-06:00"),
        "EDT" => Some("-04:00"),
        "EST" => Some("-05:00"),
        "UTC" | "GMT" => Some("+00:00"),
        _ => None,
    }
}

/// If `s` ends with a whitespace-separated recognised TZ abbreviation
/// (e.g. "2026-04-16 18:00 PDT"), strip the suffix and return
/// `(body_without_abbr, fixed_offset_string)`.
///
/// Returns `None` when no such suffix is present, or when the suffix is
/// not in the allowlist (caller falls back to the unchanged parse path).
fn strip_trailing_tz_abbreviation(s: &str) -> Option<(&str, &'static str)> {
    let s = s.trim_end();
    // Find last whitespace; candidate abbreviation is what follows.
    let last_ws = s.rfind(char::is_whitespace)?;
    let suffix = &s[last_ws + 1..];
    // Abbreviations are 2–4 ASCII letters; anything else (digits, offsets, colons)
    // would still be handled by the normal parser paths.
    if suffix.is_empty() || suffix.len() > 4 || !suffix.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let offset = normalize_tz_abbreviation(suffix)?;
    let body = s[..last_ws].trim_end();
    Some((body, offset))
}

/// Parse an absolute trigger time from various formats.
///
/// Supported formats:
/// - ISO 8601 with offset: "2026-03-18T22:00:00-07:00"
/// - ISO 8601 date + time (no offset, uses tz param): "2026-03-18T22:00:00"
/// - Date + time with a trailing US tz abbreviation: "2026-04-16 18:00 PDT"
///   (PDT/PST/MDT/MST/CDT/CST/EDT/EST/UTC/GMT; other abbreviations are rejected)
/// - Time only "HH:MM" or "HH:MM:SS" (today or tomorrow, uses tz param)
///
/// If no timezone info is available (neither in the string nor `tz` param),
/// defaults to UTC.
pub fn parse_trigger_at(s: &str, tz: Option<&str>, now_unix: i64) -> Result<i64> {
    let s = s.trim();

    // Inline TZ abbreviation ("... 18:00 PDT") → strip + delegate with the
    // matching fixed-offset string. This branch runs FIRST so an explicit
    // abbreviation in the string beats the caller-supplied `tz` argument
    // (user's intent is more specific than the ambient default).
    if let Some((body, offset_str)) = strip_trailing_tz_abbreviation(s) {
        return parse_trigger_at(body, Some(offset_str), now_unix);
    }

    // Try full ISO 8601 with offset (e.g. "2026-03-18T22:00:00-07:00")
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp());
    }
    // Try ISO 8601 with space separator (e.g. "2026-03-18 22:00:00-07:00")
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z") {
        return Ok(dt.timestamp());
    }
    // Try space-separated datetime with timezone offset (e.g. "2026-03-18 22:00:00+0800")
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%z") {
        return Ok(dt.timestamp());
    }

    // IANA zone names ("America/Los_Angeles") are resolved at the *target*
    // local date, so a trigger on the far side of a DST change gets that
    // date's offset rather than today's. Abbreviations ("PDT") and numeric
    // offsets stay fixed by construction.
    if let Some(tz_str) = tz.map(str::trim)
        && normalize_tz_abbreviation(tz_str).is_none()
        && let Ok(iana_tz) = tz_str.parse::<chrono_tz::Tz>()
    {
        return parse_local_trigger(s, &iana_tz, now_unix).ok_or_else(|| trigger_parse_error(s));
    }

    // Fixed offset from the tz parameter (default UTC).
    let offset_secs = if let Some(tz_str) = tz {
        resolve_tz_offset(tz_str)?
    } else {
        0 // default to UTC
    };
    let offset = chrono::FixedOffset::east_opt(offset_secs)
        .ok_or_else(|| anyhow::anyhow!("Invalid timezone offset: {offset_secs}s"))?;
    parse_local_trigger(s, &offset, now_unix).ok_or_else(|| trigger_parse_error(s))
}

/// Parse a date+time or time-only string as wall-clock time in `zone`.
///
/// Time-only input means the next occurrence: today if still ahead of
/// `now_unix`, otherwise the same wall-clock time on the next calendar day
/// (not `+86400`, which is off by an hour across a DST change).
fn parse_local_trigger<Z: chrono::TimeZone>(s: &str, zone: &Z, now_unix: i64) -> Option<i64> {
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return resolve_local(naive, zone);
        }
    }

    for fmt in &["%H:%M:%S", "%H:%M"] {
        if let Ok(naive_time) = chrono::NaiveTime::parse_from_str(s, fmt) {
            let today = chrono::DateTime::from_timestamp(now_unix, 0)
                .unwrap_or_default()
                .with_timezone(zone)
                .date_naive();
            let mut day = today;
            for _ in 0..2 {
                if let Some(ts) = resolve_local(day.and_time(naive_time), zone)
                    && ts > now_unix
                {
                    return Some(ts);
                }
                day = day.succ_opt()?;
            }
            return None;
        }
    }
    None
}

/// Wall-clock time in `zone` → unix seconds. An ambiguous time (DST fall-back
/// hour) takes the earlier instant; a skipped time (spring-forward gap) moves
/// one hour later, the same wall-clock reading a person would see.
fn resolve_local<Z: chrono::TimeZone>(naive: chrono::NaiveDateTime, zone: &Z) -> Option<i64> {
    match naive.and_local_timezone(zone.clone()) {
        chrono::LocalResult::Single(dt) => Some(dt.timestamp()),
        chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest.timestamp()),
        // checked_add: `+` panics near chrono's max date, and the input is
        // user/model supplied.
        chrono::LocalResult::None => {
            match naive
                .checked_add_signed(chrono::Duration::hours(1))?
                .and_local_timezone(zone.clone())
            {
                chrono::LocalResult::Single(dt) => Some(dt.timestamp()),
                chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest.timestamp()),
                chrono::LocalResult::None => None,
            }
        }
    }
}

fn trigger_parse_error(s: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Cannot parse trigger_at '{}'. Use ISO 8601 (e.g. '2026-03-18T22:00:00-07:00'), \
         date+time (e.g. '2026-03-18 22:00'), date+time with US tz abbreviation \
         (e.g. '2026-04-16 18:00 PDT'; supports PDT/PST/MDT/MST/CDT/CST/EDT/EST/UTC/GMT), \
         or time-only (e.g. '22:00').",
        s
    )
}

/// Resolve a fixed timezone string to offset seconds.
/// Accepts UTC offsets ("-07:00", "+0800", "UTC-7"),
/// or a small allowlist of US tz abbreviations (PDT/PST/MDT/MST/CDT/CST/EDT/EST/UTC/GMT).
fn resolve_tz_offset(tz: &str) -> Result<i32> {
    let tz = tz.trim();

    // US timezone abbreviation (e.g. "PDT") → fixed UTC offset by construction.
    // See `normalize_tz_abbreviation` for why we don't round-trip through IANA.
    if let Some(offset_str) = normalize_tz_abbreviation(tz) {
        // `offset_str` is always one of our hand-rolled "+HH:MM" / "-HH:MM" strings,
        // which the next block handles uniformly. We just rebind `tz` and fall through.
        return resolve_tz_offset(offset_str);
    }

    // IANA names are handled by `parse_trigger_at` at the target date, never
    // here: a single offset taken from `Utc::now()` is wrong across DST.

    // Try "UTC-7", "UTC+8", etc.
    let stripped = tz.strip_prefix("UTC").or_else(|| tz.strip_prefix("utc"));
    if let Some(rest) = stripped {
        if rest.is_empty() {
            return Ok(0);
        }
        if let Ok(hours) = rest.parse::<i32>() {
            return Ok(hours * 3600);
        }
    }

    // Try "+HH:MM" / "-HH:MM" / "+HHMM" / "-HHMM"
    if (tz.starts_with('+') || tz.starts_with('-')) && tz.len() >= 3 {
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        let num = &tz[1..];
        if let Some((h, m)) = num.split_once(':') {
            if let (Ok(hh), Ok(mm)) = (h.parse::<i32>(), m.parse::<i32>()) {
                return Ok(sign * (hh * 3600 + mm * 60));
            }
        } else if num.len() == 4 {
            if let (Ok(hh), Ok(mm)) = (num[..2].parse::<i32>(), num[2..].parse::<i32>()) {
                return Ok(sign * (hh * 3600 + mm * 60));
            }
        } else if let Ok(hh) = num.parse::<i32>() {
            return Ok(sign * hh * 3600);
        }
    }

    Err(anyhow::anyhow!(
        "Cannot parse timezone '{}'. Use IANA name (e.g. 'America/Los_Angeles'), \
         UTC offset (e.g. 'UTC-7', '-07:00'), US abbreviation \
         (PDT/PST/MDT/MST/CDT/CST/EDT/EST/UTC/GMT), or just omit for UTC.",
        tz
    ))
}

// ── Windowed recurrence ──────────────────────────────────────────────────────

/// Compute the next trigger time for a recurring intent, honoring optional
/// weekday + time-window constraints (Option C).
///
/// When the intent has NO window fields set, this returns `now_unix + interval`
/// — byte-identical to the classic fixed-interval behavior (regression-critical).
///
/// When window fields ARE set, the candidate `now_unix + interval` is converted
/// to local time in `recur_timezone` (default "UTC" if a window is set but no tz
/// is given) and checked against the allowed weekday set (`recur_weekdays`, if
/// set) AND the local time window `[recur_window_start, recur_window_end]` (if
/// set). If the candidate is outside the window/weekday set, it is advanced to
/// the start of the next valid window (this window's start if the candidate is
/// before it on a valid weekday, else the next allowed weekday's window start).
/// Midnight-straddling windows (end &lt; start) are treated as wrapping past
/// midnight (a 24h overflow span).
/// Pure: the next occurrence after `now` that keeps the phase of `scheduled`
/// (whole intervals from the scheduled time); `now + interval` when the row
/// has no usable scheduled time.
pub fn next_trigger_base(now_unix: i64, scheduled: Option<i64>, interval: i64) -> i64 {
    match scheduled.filter(|t| *t > 0 && interval > 0) {
        Some(scheduled) if scheduled >= now_unix => scheduled + interval,
        Some(scheduled) => {
            let missed = (now_unix - scheduled) / interval;
            scheduled + (missed + 1) * interval
        }
        None => now_unix + interval,
    }
}

pub fn next_windowed_trigger(
    now_unix: i64,
    intent: &crate::memory::DeferredIntent,
    interval: i64,
) -> i64 {
    // Advance from the SCHEDULED time in whole intervals, so delivery latency
    // and downtime never shift a "每周一 10:00" reminder to Monday 10:03,
    // then Thursday… (review 2026-09-13). Falls back to now+interval when the
    // row has no trigger time.
    let base = next_trigger_base(now_unix, intent.trigger_at_unix, interval);
    snap_into_window(
        base,
        intent.recur_weekdays.as_deref(),
        intent.recur_window_start.as_deref(),
        intent.recur_window_end.as_deref(),
        intent.recur_timezone.as_deref(),
    )
}

/// Snap `candidate` (a UTC unix timestamp) forward to the first instant that
/// satisfies the windowed-recurrence constraints — allowed weekday set + daily
/// time window in `timezone` — at or after `candidate`. Returns `candidate`
/// unchanged when no window/weekday fields are set, or when it already satisfies
/// them. Shared by two callers so the rule is enforced identically:
///   * the firing path (next fire = `snap_into_window(now + interval, …)`), and
///   * the INITIAL trigger at creation time, so the very first fire also lands
///     in-window instead of firing once at the raw requested delay (Codex F4).
pub(crate) fn snap_into_window(
    candidate: i64,
    weekdays: Option<&[u8]>,
    window_start: Option<&str>,
    window_end: Option<&str>,
    timezone: Option<&str>,
) -> i64 {
    use chrono::{Datelike, TimeZone, Timelike};

    let window = match (window_start, window_end) {
        (Some(s), Some(e)) => match (parse_hh_mm(s), parse_hh_mm(e)) {
            (Some(start), Some(end)) => Some((start, end)),
            // Malformed window → degrade to classic behavior (fail-open to "fire
            // on interval") rather than wedge the recurring intent.
            _ => None,
        },
        _ => None,
    };

    // No constraints at all → classic path (regression guard).
    if weekdays.is_none() && window.is_none() {
        return candidate;
    }

    let tz: chrono_tz::Tz = timezone
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::Tz::UTC);

    let weekday_allowed =
        |day_iso: u8| -> bool { weekdays.map(|w| w.contains(&day_iso)).unwrap_or(true) };

    // Minutes-of-day for window bounds.
    let window_minutes =
        window.map(|((sh, sm), (eh, em))| ((sh * 60 + sm) as i64, (eh * 60 + em) as i64));

    let local = tz.timestamp_opt(candidate, 0).single();
    let Some(local) = local else {
        // Ambiguous/skipped local time (DST). Fall back to classic candidate.
        return candidate;
    };

    let cand_iso = local.weekday().number_from_monday() as u8; // 1..7
    let cand_minutes = (local.hour() * 60 + local.minute()) as i64;

    // Does the candidate already satisfy the constraints?
    let within_window = match window_minutes {
        None => true,
        Some((start, end)) => {
            if end >= start {
                cand_minutes >= start && cand_minutes <= end
            } else {
                // Straddles midnight: [start..24:00) ∪ [00:00..end]
                cand_minutes >= start || cand_minutes <= end
            }
        }
    };
    if weekday_allowed(cand_iso) && within_window {
        return candidate;
    }

    // Otherwise advance to the next valid window start. Search day-by-day from
    // the candidate's local date for up to 8 days (covers any weekday set).
    let start_minutes = window_minutes.map(|(s, _)| s).unwrap_or(0);
    let start_h = (start_minutes / 60) as u32;
    let start_m = (start_minutes % 60) as u32;
    let cand_date = local.date_naive();

    // Bound for the DST-gap forward probe (below): never advance past the window
    // length, so a probed start can never land outside the window. Capped at 180
    // min (DST gaps are ≤1h); ≥1 so weekday-only (no time window) still probes.
    let probe_limit_min: i64 = window_minutes
        .map(|(s, e)| if e >= s { e - s } else { 24 * 60 - s + e })
        .unwrap_or(180)
        .clamp(1, 180);

    // Resolve a local wall-clock window-start (date @ h:m) to a UTC timestamp,
    // robust to DST transitions:
    //   * Single   → that instant.
    //   * Ambiguous (fall-back overlap) → the EARLIER of the two instants.
    //   * None      (spring-forward gap: the wall-clock time doesn't exist) →
    //     advance minute-by-minute (bounded by the window length) to the first
    //     instant that DOES exist, rather than skipping the whole day.
    let resolve_window_start = |date: chrono::NaiveDate, h: u32, m: u32| -> Option<i64> {
        let base = chrono::NaiveTime::from_hms_opt(h, m, 0)?;
        let mut probe = date.and_time(base);
        for _ in 0..=probe_limit_min {
            match tz.from_local_datetime(&probe) {
                chrono::LocalResult::Single(dt) => return Some(dt.timestamp()),
                chrono::LocalResult::Ambiguous(earliest, _) => return Some(earliest.timestamp()),
                chrono::LocalResult::None => {
                    probe += chrono::Duration::minutes(1);
                }
            }
        }
        None
    };

    for add_days in 0..=8 {
        let date = cand_date + chrono::Duration::days(add_days);
        let iso = date.weekday().number_from_monday() as u8;
        if !weekday_allowed(iso) {
            continue;
        }
        let Some(candidate_ts) = resolve_window_start(date, start_h, start_m) else {
            continue; // window start unresolvable this day (gap exceeds window)
        };
        // Must not fire earlier than the configured interval: a valid window
        // start is only acceptable if it is at/after `candidate` (= now+interval).
        // (Using `now_unix` here would let a long interval that jumps past today's
        // window still fire at today's earlier start — firing sooner than asked.)
        if candidate_ts >= candidate {
            return candidate_ts;
        }
    }

    // Fallback (should be unreachable for a non-empty weekday set): classic.
    candidate
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Validated intent request ready for storage.
#[derive(Debug)]
pub struct ValidatedIntent {
    pub trigger_type: String,
    pub trigger_at_unix: Option<i64>,
    pub event_key: Option<String>,
    pub execution_mode: String,
    pub context_text: String,
    pub expires_at_unix: i64,
    /// If set, the intent auto-reschedules after each firing.
    pub interval_seconds: Option<i64>,
    /// Semantic dedup key: same group_key + user_did = upsert (replaces existing).
    pub group_key: Option<String>,
    /// Maximum fire count before auto-cancel.
    pub max_fires: Option<i64>,
    /// Windowed recurrence: ISO weekdays (1=Mon..7=Sun) the intent may fire on.
    pub recur_weekdays: Option<Vec<u8>>,
    /// Windowed recurrence: "HH:MM" local window start.
    pub recur_window_start: Option<String>,
    /// Windowed recurrence: "HH:MM" local window end.
    pub recur_window_end: Option<String>,
    /// Windowed recurrence: IANA timezone the window/weekdays are evaluated in.
    pub recur_timezone: Option<String>,
    /// Warning message when intent configuration looks suspicious (e.g., group_key without interval).
    pub warning: Option<String>,
}

/// Parse and validate a time-based trigger: resolves `trigger_at` / `delay_seconds`,
/// Checks range bounds and expiry; returns `(trigger_at_unix, expires_at_unix, advanced_start)`.
fn validate_time_trigger(
    args: &serde_json::Value,
    now: i64,
    interval_seconds: Option<i64>,
) -> Result<(i64, i64, bool)> {
    let delay_seconds = args.get("delay_seconds").and_then(|v| v.as_i64());
    let trigger_at_str = args.get("trigger_at").and_then(|v| v.as_str());
    let tz_str = args.get("timezone").and_then(|v| v.as_str());

    let mut trigger_at = if let Some(ta) = trigger_at_str {
        // Absolute time: parse ISO 8601 or "HH:MM" / "HH:MM:SS"
        parse_trigger_at(ta, tz_str, now)?
    } else if let Some(delay) = delay_seconds {
        // Relative delay (backward compatible)
        if delay < MIN_DELAY_SECS {
            return Err(anyhow::anyhow!(
                "delay_seconds too short (min {}s)",
                MIN_DELAY_SECS
            ));
        }
        if delay > MAX_INTENT_LIFETIME_SECS {
            return Err(anyhow::anyhow!(
                "delay_seconds too long (max {} days)",
                MAX_INTENT_LIFETIME_SECS / SECS_PER_DAY
            ));
        }
        now + delay
    } else {
        return Err(anyhow::anyhow!(
            "Time trigger requires either 'trigger_at' (absolute time, e.g. '22:00' or '2026-03-18T22:00:00-07:00') \
             or 'delay_seconds' (relative delay in seconds)"
        ));
    };

    // LLM/tool latency can overtake a just-requested recurring start. Preserve
    // its cadence, skip missed occurrences, and still apply the weekday/window
    // constraints below. Never reinterpret a stale one-shot or an old schedule.
    let mut advanced_start = false;
    if trigger_at < now + MIN_DELAY_SECS
        && trigger_at >= now - MAX_RECURRING_START_SKEW_SECS
        && let Some(interval) = interval_seconds.filter(|interval| *interval > 0)
    {
        let distance = now + MIN_DELAY_SECS - trigger_at;
        trigger_at += ((distance - 1) / interval + 1) * interval;
        advanced_start = true;
    }

    // Sanity check: trigger must be in the future (at least MIN_DELAY_SECS)
    if trigger_at < now + MIN_DELAY_SECS {
        return Err(anyhow::anyhow!(
            "trigger_at must be at least {}s in the future (got {}s)",
            MIN_DELAY_SECS,
            trigger_at - now
        ));
    }
    if trigger_at - now > MAX_INTENT_LIFETIME_SECS {
        return Err(anyhow::anyhow!(
            "trigger_at too far in the future (max {} days)",
            MAX_INTENT_LIFETIME_SECS / SECS_PER_DAY
        ));
    }

    // Recurring intents get a 30-day expiry; one-shot gets 1hr grace after trigger
    let expires_at = if interval_seconds.is_some() {
        now + RECURRING_EXPIRY_SECS
    } else {
        trigger_at + 3600
    };
    Ok((trigger_at, expires_at, advanced_start))
}

/// Validate a schedule_intent request from the LLM.
pub fn validate_intent_request(args: &serde_json::Value) -> Result<ValidatedIntent> {
    let trigger_type = args
        .get("trigger_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match trigger_type {
        "time" | "event" => {}
        "" => {
            return Err(anyhow::anyhow!(
                "Missing 'trigger_type' (must be 'time' or 'event')"
            ));
        }
        other => {
            return Err(anyhow::anyhow!(
                "Invalid trigger_type '{}'. Must be 'time' or 'event'.",
                other
            ));
        }
    }

    let context_text = args
        .get("context_text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if context_text.is_empty() {
        return Err(anyhow::anyhow!("Missing 'context_text'"));
    }
    let char_count = context_text.chars().count();
    if char_count > MAX_CONTEXT_LEN {
        return Err(anyhow::anyhow!(
            "context_text too long ({} chars, max {})",
            char_count,
            MAX_CONTEXT_LEN
        ));
    }

    // Default to llm_callback (fail-safe): llm_callback can always handle notify-style
    // messages (LLM just echoes text), but notify cannot handle action-style tasks.
    // This eliminates the class of bugs where LLM picks "notify" for tasks requiring tools.
    let execution_mode = args
        .get("execution_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("llm_callback")
        .to_string();

    match execution_mode.as_str() {
        "notify" | "llm_callback" => {}
        other => {
            return Err(anyhow::anyhow!(
                "Invalid execution_mode '{}'. Must be 'notify' or 'llm_callback'.",
                other
            ));
        }
    }

    let now = chrono::Utc::now().timestamp();

    // Parse optional interval_seconds for recurring intents
    let interval_seconds = args.get("interval_seconds").and_then(|v| v.as_i64());

    // Validate interval_seconds if present
    if let Some(interval) = interval_seconds {
        if trigger_type != "time" {
            return Err(anyhow::anyhow!(
                "interval_seconds is only valid for trigger_type='time'"
            ));
        }
        if interval < MIN_RECURRING_INTERVAL {
            return Err(anyhow::anyhow!(
                "interval_seconds too short (min {}s)",
                MIN_RECURRING_INTERVAL
            ));
        }
        if interval > MAX_INTENT_LIFETIME_SECS {
            return Err(anyhow::anyhow!(
                "interval_seconds too long (max {} days)",
                MAX_INTENT_LIFETIME_SECS / SECS_PER_DAY
            ));
        }
    }

    let (mut trigger_at_unix, event_key, expires_at_unix, advanced_start) = match trigger_type {
        "time" => {
            let (trigger_at, expires_at, advanced) =
                validate_time_trigger(args, now, interval_seconds)?;
            (Some(trigger_at), None, expires_at, advanced)
        }
        "event" => {
            let key = args
                .get("event_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if key.is_empty() {
                return Err(anyhow::anyhow!("Missing 'event_key' for event trigger"));
            }
            let expires_at = now + MAX_INTENT_LIFETIME_SECS;
            (None, Some(key), expires_at, false)
        }
        _ => unreachable!(),
    };

    // Parse group_key (required for recurring intents)
    let group_key = args
        .get("group_key")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if interval_seconds.is_some() && group_key.is_none() {
        return Err(anyhow::anyhow!(
            "Recurring intents require 'group_key' to prevent duplicates. \
             Set group_key to a short identifier for this recurring task's purpose \
             (e.g., 'commute_monitor', 'email_check', 'btc_price')."
        ));
    }

    if let Some(ref gk) = group_key {
        if gk.len() > 100 {
            return Err(anyhow::anyhow!(
                "group_key too long ({} chars, max 100)",
                gk.len()
            ));
        }
        if !gk
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(anyhow::anyhow!(
                "group_key must contain only alphanumeric characters, underscores, and hyphens"
            ));
        }
    }

    // Warn when group_key is set but no interval — likely the LLM forgot interval_seconds
    let warning = if advanced_start {
        Some("The recurring start was overtaken by processing time; skipped missed occurrences and advanced to the next valid occurrence. No catch-up execution was performed. / 循环监控开始时间已过，已跳过错过的时点并顺延至下一个有效时点，不补跑。".to_string())
    } else if group_key.is_some() && interval_seconds.is_none() {
        Some(
            "WARNING: group_key was provided but interval_seconds was not. \
             This creates a ONE-SHOT intent, not recurring. If the user requested \
             recurring monitoring/checking, you MUST include interval_seconds. \
             Call schedule_intent again with interval_seconds to make it recurring."
                .to_string(),
        )
    } else {
        None
    };

    // Parse max_fires (optional fire-count lifecycle)
    let max_fires = args.get("max_fires").and_then(|v| v.as_i64());
    if let Some(mf) = max_fires
        && mf < 1
    {
        return Err(anyhow::anyhow!("max_fires must be >= 1 (got {})", mf));
    }

    // ── Windowed weekday recurrence (Option C) ──────────────────────────────
    // These four optional fields constrain WHEN within a day / which weekdays a
    // recurring intent may fire. They only make sense with interval_seconds +
    // trigger_type='time'; absent → classic fixed-interval behavior (unchanged).
    let recur_weekdays = parse_recur_weekdays(args)?;
    let recur_window_start = parse_window_time(args, "recur_window_start")?;
    let recur_window_end = parse_window_time(args, "recur_window_end")?;
    let recur_timezone = parse_recur_timezone(args)?;

    let has_window =
        recur_weekdays.is_some() || recur_window_start.is_some() || recur_window_end.is_some();
    if has_window {
        // Window semantics are only meaningful for recurring time intents.
        if interval_seconds.is_none() {
            return Err(anyhow::anyhow!(
                "recur_weekdays / recur_window_start / recur_window_end require \
                 interval_seconds (windowed recurrence only applies to recurring intents)"
            ));
        }
        if trigger_type != "time" {
            return Err(anyhow::anyhow!(
                "windowed recurrence (recur_weekdays / recur_window_*) requires trigger_type='time'"
            ));
        }
        // A window needs both bounds to be unambiguous.
        if recur_window_start.is_some() != recur_window_end.is_some() {
            return Err(anyhow::anyhow!(
                "recur_window_start and recur_window_end must be provided together"
            ));
        }
        // An overnight (wrapping) window — end earlier than start — is fully
        // supported on its own, but combining it with a weekday restriction makes
        // the post-midnight segment's "owning weekday" ambiguous (00:00..end
        // belongs to the PREVIOUS calendar day's window). Rather than ship subtly
        // wrong weekday ownership across midnight, reject the combination. Same-day
        // weekday windows and weekday-less overnight windows are both supported.
        if let (Some(start), Some(end)) =
            (recur_window_start.as_deref(), recur_window_end.as_deref())
            && let (Some((sh, sm)), Some((eh, em))) = (parse_hh_mm(start), parse_hh_mm(end))
            && (eh * 60 + em) < (sh * 60 + sm)
            && recur_weekdays.is_some()
        {
            return Err(anyhow::anyhow!(
                "an overnight window (recur_window_end earlier than recur_window_start) \
                 cannot be combined with recur_weekdays; use a same-day window, or drop \
                 the weekday restriction for a nightly overnight window"
            ));
        }

        // Snap the INITIAL trigger into the window too. `next_windowed_trigger`
        // only adjusts SUBSEQUENT fires (when an already-due recurring intent is
        // rescheduled after firing), so without this the FIRST fire would happen
        // at the raw requested `delay_seconds`/`trigger_at` — possibly outside the
        // window (e.g. a small required delay on a morning request for an evening
        // commute window would fire once in the morning). Snap forward to the
        // first valid instant at/after the requested initial trigger (Codex F4).
        if let Some(initial) = trigger_at_unix {
            let snapped = snap_into_window(
                initial,
                recur_weekdays.as_deref(),
                recur_window_start.as_deref(),
                recur_window_end.as_deref(),
                recur_timezone.as_deref(),
            );
            // The snap only moves forward (to the first allowed window). Re-check
            // the upper lifetime bound that validate_time_trigger enforced on the
            // RAW trigger, since the snap can push a near-max request past the max
            // (the MIN_DELAY lower bound is preserved by forward-only movement).
            if snapped - now > MAX_INTENT_LIFETIME_SECS {
                return Err(anyhow::anyhow!(
                    "the first allowed window after snapping is {} days out, beyond the \
                     max intent lifetime ({} days); widen the window or weekday set",
                    (snapped - now) / SECS_PER_DAY,
                    MAX_INTENT_LIFETIME_SECS / SECS_PER_DAY
                ));
            }
            trigger_at_unix = Some(snapped);
        }
    }

    Ok(ValidatedIntent {
        trigger_type: trigger_type.to_string(),
        trigger_at_unix,
        event_key,
        execution_mode,
        context_text,
        expires_at_unix,
        interval_seconds,
        group_key,
        max_fires,
        recur_weekdays,
        recur_window_start,
        recur_window_end,
        recur_timezone,
        warning,
    })
}

/// Parse the optional `recur_weekdays` field into a deduped, sorted `Vec<u8>` of
/// ISO weekdays (1=Mon..7=Sun). Returns `Ok(None)` when absent. Rejects empty
/// arrays and out-of-range values (a silent drop would make a window fire on the
/// wrong days).
fn parse_recur_weekdays(args: &serde_json::Value) -> Result<Option<Vec<u8>>> {
    let Some(v) = args.get("recur_weekdays") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("recur_weekdays must be an array of integers 1..7"))?;
    if arr.is_empty() {
        return Err(anyhow::anyhow!(
            "recur_weekdays must not be empty (omit the field for no weekday restriction)"
        ));
    }
    let mut days: Vec<u8> = Vec::with_capacity(arr.len());
    for d in arr {
        let n = d
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("recur_weekdays entries must be integers"))?;
        if !(1..=7).contains(&n) {
            return Err(anyhow::anyhow!(
                "recur_weekdays entries must be 1..7 (1=Mon..7=Sun), got {}",
                n
            ));
        }
        let n = n as u8;
        if !days.contains(&n) {
            days.push(n);
        }
    }
    days.sort_unstable();
    Ok(Some(days))
}

/// Parse an optional "HH:MM" local-time window bound. Returns `Ok(None)` when
/// absent. Rejects malformed strings and out-of-range hours/minutes.
fn parse_window_time(args: &serde_json::Value, key: &str) -> Result<Option<String>> {
    let Some(v) = args.get(key) else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let s = v
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{} must be a string like '17:30'", key))?
        .trim();
    if s.is_empty() {
        return Ok(None);
    }
    let (h, m) = parse_hh_mm(s)
        .ok_or_else(|| anyhow::anyhow!("{} must be 'HH:MM' (00:00..23:59), got '{}'", key, s))?;
    // Normalize to zero-padded canonical form for stable storage.
    Ok(Some(format!("{h:02}:{m:02}")))
}

/// Parse "HH:MM" into (hour, minute), validating ranges. Returns None if invalid.
pub(crate) fn parse_hh_mm(s: &str) -> Option<(u32, u32)> {
    let (hh, mm) = s.split_once(':')?;
    let h: u32 = hh.parse().ok()?;
    let m: u32 = mm.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// Parse the optional `recur_timezone` field. STRICTLY validates against the
/// IANA database — an unparseable tz is rejected (NOT silently coerced to UTC,
/// which would make a window fire at the wrong wall-clock time). Returns
/// `Ok(None)` when absent.
fn parse_recur_timezone(args: &serde_json::Value) -> Result<Option<String>> {
    let Some(v) = args.get("recur_timezone") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let s = v
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("recur_timezone must be a string (IANA tz name)"))?
        .trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.parse::<chrono_tz::Tz>().is_err() {
        return Err(anyhow::anyhow!(
            "recur_timezone '{}' is not a valid IANA timezone (e.g. 'America/Los_Angeles')",
            s
        ));
    }
    Ok(Some(s.to_string()))
}

/// Check if a message originated from an intent callback (anti-recursion).
pub fn is_callback_context(msg: &str) -> bool {
    msg.starts_with(CALLBACK_CONTEXT_PREFIX) || msg.starts_with("[INTENT_CALLBACK:depth=")
}

/// Parse the chain depth from a callback context message.
/// Returns None if the message is not a callback or has no depth encoding.
/// Format: `[INTENT_CALLBACK:depth=N] ...`
pub fn parse_callback_depth(msg: &str) -> Option<i32> {
    if let Some(rest) = msg.strip_prefix("[INTENT_CALLBACK:depth=")
        && let Some(end) = rest.find(']')
    {
        return rest[..end].parse::<i32>().ok();
    }
    // Legacy format without depth — treat as depth 0
    if msg.starts_with(CALLBACK_CONTEXT_PREFIX) {
        return Some(0);
    }
    None
}

/// Strip the `[INTENT_CALLBACK:depth=N]` prefix from a callback message,
/// returning the raw context_text. Used for cleaner embedding queries.
pub fn strip_callback_prefix(msg: &str) -> &str {
    // Format: "[INTENT_CALLBACK:depth=N] context_text"
    if let Some(rest) = msg.strip_prefix("[INTENT_CALLBACK:depth=")
        && let Some(idx) = rest.find(']')
    {
        return rest[idx + 1..].trim_start();
    }
    // Legacy format: "[INTENT_CALLBACK] context_text"
    if let Some(rest) = msg.strip_prefix(CALLBACK_CONTEXT_PREFIX) {
        return rest.trim_start();
    }
    msg
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "intent_tests.rs"]
mod tests;
