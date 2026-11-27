use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use teloxide::prelude::*;
use teloxide::types::{
    BotCommand, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile,
    LinkPreviewOptions, MessageId, ParseMode, ReactionType,
};

use crate::chronos::HeartbeatHandler;
use crate::i18n::{t, tf};
use crate::memory::{MemoryHandle, Message as MemoryMessage};
use crate::models::ProbeSignal;
use crate::multimodal::MediaProcessor;
use crate::orchestrator::Orchestrator;
use crate::telegram_notification_router::{
    NotificationClass, NotificationRoute, PrivacyLevel, proactive_route_request,
    route_telegram_notification,
};
use crate::topic_router::{TopicKind, resolve_thread_id};
use crate::utils::strip_html_tags;

/// Shared tracker for progressive message editing.
/// Maps chat_id → active progress message that gets edited in place.
pub type ProgressTracker = Arc<std::sync::Mutex<std::collections::HashMap<i64, MessageId>>>;

// Re-export rendering utilities so callers can continue using `telegram::*`
pub use crate::telegram_render::{
    escape_html, escape_markdown_v2, markdown_to_telegram_html, split_message,
};

/// Maximum number of retries for Telegram API rate-limit (429) errors.
const MAX_SEND_RETRIES: u32 = 3;

/// When a reply exceeds this many chunks, send as a document instead.
const LONG_OUTPUT_CHUNK_THRESHOLD: usize = 3;

/// Base delay for exponential backoff on 429 errors (seconds).
const BACKOFF_BASE_SECS: u64 = 2;

/// Maximum backoff cap for retries (seconds).
const MAX_BACKOFF_SECS: u64 = 60;

/// Truncation length for error text previews sent to users.
const ERROR_TEXT_PREVIEW_LEN: usize = 500;

/// Maximum number of callback IDs to track for replay prevention.
const CALLBACK_DEDUP_MAX_ENTRIES: usize = 1000;

/// Time-to-live for callback dedup entries (seconds).
const CALLBACK_DEDUP_TTL_SECS: u64 = 60;

fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic payload was not a string".to_string()
    }
}

async fn build_startup_status_html(orchestrator: &Orchestrator) -> String {
    let active_skills = orchestrator
        .memory
        .list_skill_proposals(None, Some(crate::memory::STATUS_ACTIVE))
        .await
        .map(|items| items.len().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let pending_hitl = orchestrator
        .memory
        .list_hitl_pending()
        .await
        .map(|items| items.len().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!(
        "<b>系统已重启 / System restarted</b>\n\n\
         <b>当前重要信息</b>\n\
         • 活跃技能 / Active skills: {}\n\
         • HITL 待审批: {}",
        escape_html(&active_skills),
        escape_html(&pending_hitl)
    )
}

fn should_attach_startup_reply_keyboard(admin_chat_id: i64, route: &NotificationRoute) -> bool {
    route.chat_id == admin_chat_id && route.thread_id.is_none()
}

fn should_send_startup_status_to_telegram(startup_html: &str) -> bool {
    let plain = strip_html_tags(startup_html).to_lowercase();
    [
        "alert",
        "failed",
        "failure",
        "panic",
        "critical",
        "unhealthy",
        "异常",
        "失败",
        "严重",
    ]
    .iter()
    .any(|needle| plain.contains(needle))
}

fn oauth_deeplink_reply_route(admin_chat_id: i64, origin_chat_id: i64) -> NotificationRoute {
    route_telegram_notification(
        admin_chat_id,
        proactive_route_request(
            NotificationClass::OAuthDeepLinkReply,
            PrivacyLevel::PrivateDmOnly,
            Some(origin_chat_id),
        ),
    )
}

/// Bounded dedup set for Telegram callback_query IDs.
///
/// Prevents duplicate processing when Telegram re-delivers the same
/// callback (e.g. user double-clicks an inline button). Entries are
/// evicted when they exceed `CALLBACK_DEDUP_TTL_SECS` or the set
/// grows beyond `CALLBACK_DEDUP_MAX_ENTRIES`.
pub(crate) struct CallbackDedup {
    /// FIFO queue of (callback_id, insert_time) for age-based eviction.
    entries: std::collections::VecDeque<(String, std::time::Instant)>,
    /// Fast membership test.
    ids: std::collections::HashSet<String>,
}

impl CallbackDedup {
    pub(crate) fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            ids: std::collections::HashSet::new(),
        }
    }

    /// Returns `true` if this callback ID was already seen (duplicate).
    /// Otherwise inserts it and returns `false`.
    pub(crate) fn check_and_insert(&mut self, id: &str) -> bool {
        self.evict_stale();
        if self.ids.contains(id) {
            return true;
        }
        // Evict oldest if at capacity
        while self.entries.len() >= CALLBACK_DEDUP_MAX_ENTRIES {
            if let Some((old_id, _)) = self.entries.pop_front() {
                self.ids.remove(&old_id);
            }
        }
        self.ids.insert(id.to_string());
        self.entries
            .push_back((id.to_string(), std::time::Instant::now()));
        false
    }

    /// Remove entries older than the TTL.
    fn evict_stale(&mut self) {
        let cutoff =
            std::time::Instant::now() - std::time::Duration::from_secs(CALLBACK_DEDUP_TTL_SECS);
        while let Some((id, ts)) = self.entries.front() {
            if *ts < cutoff {
                let id = id.clone();
                self.entries.pop_front();
                self.ids.remove(&id);
            } else {
                break;
            }
        }
    }

    /// Number of currently tracked entries (for testing).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Bounded set of approval callbacks currently executing their apply
/// pipeline. Indexed by raw `callback_data` (e.g. `config_approve:config-…`)
/// so a second tap on the same button — which Telegram delivers as a new
/// `callback_query` with a fresh id — is recognised as a duplicate and
/// suppressed, avoiding double-apply and stale "query is too old" errors.
///
/// TTL is deliberately larger than the worst-case apply duration
/// (config-* hot-reload + downstream work can reach ~60 s) so a stuck
/// handler eventually frees its slot instead of locking the button
/// forever.
pub(crate) struct InFlightApprovals {
    entries: std::collections::VecDeque<(String, std::time::Instant)>,
    ids: std::collections::HashSet<String>,
}

/// Max in-flight approvals tracked simultaneously. Pathological upper
/// bound — only the admin chat hits this code path.
const INFLIGHT_APPROVALS_MAX_ENTRIES: usize = 128;

/// TTL after which a stuck in-flight slot is evicted. Must exceed the
/// slowest realistic apply (~60s for config-* hot-reload → reembed) with
/// comfortable headroom.
const INFLIGHT_APPROVALS_TTL_SECS: u64 = 180;

impl InFlightApprovals {
    pub(crate) fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            ids: std::collections::HashSet::new(),
        }
    }

    /// Returns `true` if the key was already in-flight (caller should
    /// short-circuit as a dedup), otherwise inserts and returns `false`.
    pub(crate) fn try_acquire(&mut self, key: &str) -> bool {
        self.evict_stale();
        if self.ids.contains(key) {
            return true;
        }
        while self.entries.len() >= INFLIGHT_APPROVALS_MAX_ENTRIES {
            if let Some((old_id, _)) = self.entries.pop_front() {
                self.ids.remove(&old_id);
            }
        }
        self.ids.insert(key.to_string());
        self.entries
            .push_back((key.to_string(), std::time::Instant::now()));
        false
    }

    /// Release a previously-acquired slot. Idempotent — safe to call
    /// from both the normal-completion path and the panic-guard path.
    pub(crate) fn release(&mut self, key: &str) {
        if self.ids.remove(key) {
            self.entries.retain(|(k, _)| k != key);
        }
    }

    fn evict_stale(&mut self) {
        let cutoff =
            std::time::Instant::now() - std::time::Duration::from_secs(INFLIGHT_APPROVALS_TTL_SECS);
        while let Some((_, ts)) = self.entries.front() {
            if *ts < cutoff {
                if let Some((id, _)) = self.entries.pop_front() {
                    self.ids.remove(&id);
                }
            } else {
                break;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Check if a teloxide error is a Telegram rate-limit (429) error.
fn is_rate_limit_error(err: &teloxide::RequestError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("too many requests") || msg.contains("retry after") || msg.contains("429")
}

/// Extract "retry after N seconds" from a Telegram 429 error message, if present.
fn extract_retry_after(err: &teloxide::RequestError) -> Option<u64> {
    let msg = err.to_string();
    // Telegram error format: "...retry after N..."
    if let Some(idx) = msg.to_lowercase().find("retry after") {
        let after = &msg[idx + 12..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    } else {
        None
    }
}

/// Send a Telegram message with automatic retry on 429 rate-limit errors.
/// Uses exponential backoff with optional server-suggested delay.
/// Returns the MessageId of the sent message on success.
async fn send_with_retry<F, Fut>(
    mut send_fn: F,
) -> std::result::Result<Message, teloxide::RequestError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<Message, teloxide::RequestError>>,
{
    for attempt in 0..=MAX_SEND_RETRIES {
        match send_fn().await {
            Ok(msg) => return Ok(msg),
            Err(e) if is_rate_limit_error(&e) && attempt < MAX_SEND_RETRIES => {
                let server_delay = extract_retry_after(&e);
                let backoff = server_delay.unwrap_or(BACKOFF_BASE_SECS * 2u64.pow(attempt));
                // Cap at MAX_BACKOFF_SECS
                let delay = backoff.min(MAX_BACKOFF_SECS);
                tracing::warn!(
                    attempt = attempt + 1,
                    delay_secs = delay,
                    error = %e,
                    "Telegram rate limited, retrying after backoff"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(teloxide::RequestError::Api(teloxide::ApiError::Unknown(
        "send_with_retry loop exhausted".into(),
    )))
}

/// Convert a raw `i32` thread-id from [`resolve_thread_id`] into teloxide's
/// `ThreadId(MessageId(i32))` wrapper that builder methods like
/// `.message_thread_id(...)` expect. Returns `None` if the input is `None`,
/// which is the DM-mode default in P0. Public so callsites outside this
/// module (main.rs, admin.rs, orchestrator.rs, orchestrator_tools_web.rs)
/// can thread topic routing through raw
/// `bot.send_message/send_photo/send_voice` builders.
pub fn wrap_thread_id(raw: Option<i32>) -> Option<teloxide::types::ThreadId> {
    raw.map(|id| teloxide::types::ThreadId(MessageId(id)))
}

/// Authorization gate for inbound Telegram updates (messages + callback taps).
///
/// Returns `true` only for the owner's **admin DM** — `chat_id == admin_chat_id`
/// (Telegram: the admin's private chat id equals their user id, which we
/// persist as `admin_chat_id`). Linked-forum/group routing was removed, so a
/// group chat is never an authorized surface.
///
/// Rejects everything else: strangers' DMs, any group chat, anonymous /
/// channel-posted messages (no `from`). `sender_user_id` is retained for
/// call-site stability and is not load-bearing (a DM's chat id already equals
/// the sender's user id).
pub(crate) fn is_authorized_chat(
    chat_id: i64,
    sender_user_id: Option<i64>,
    admin_chat_id: i64,
) -> bool {
    let _ = sender_user_id;
    chat_id == admin_chat_id
}

/// Clamp a PERSISTED send-target chat id to the owner's admin chat.
///
/// Forum/linked-group routing was removed: every outbound Telegram message
/// must land in the owner's admin DM. But several rows persist a `chat_id`
/// captured at creation time (`tasks`, `deferred_intents`)
/// — a row created from the old linked group carries that group's (negative)
/// id, and sending to it would post into the defunct group. Any send that
/// reads a persisted chat_id must route through this clamp. Pure + testable;
/// `admin` is `None` only when `ADMIN_CHAT_ID` is unset (dev), where there is
/// no admin to clamp to so the persisted value is kept unchanged.
pub(crate) fn clamp_chat_to_admin(persisted_chat_id: i64, admin: Option<i64>) -> i64 {
    admin.unwrap_or(persisted_chat_id)
}

/// Env-backed [`clamp_chat_to_admin`] → `ChatId`, for the outbound send sites
/// that read a persisted chat id (`tasks`/`deferred_intents`).
pub(crate) fn admin_dm_chat(persisted_chat_id: i64) -> teloxide::types::ChatId {
    let admin = std::env::var("ADMIN_CHAT_ID")
        .ok()
        .and_then(|s| s.parse::<i64>().ok());
    teloxide::types::ChatId(clamp_chat_to_admin(persisted_chat_id, admin))
}

/// Memory namespace for authorized Telegram interactions.
///
/// Only the owner's admin DM is authorized now (forum/linked-group routing was
/// removed). This still maps `sender_user_id == admin` to the canonical admin
/// DID so any future authorized surface shares the admin's L1/L2/L3 history.
pub(crate) fn user_did_for_authorized_chat(
    chat_id: i64,
    sender_user_id: Option<i64>,
    admin_chat_id: i64,
) -> String {
    if chat_id == admin_chat_id || sender_user_id == Some(admin_chat_id) {
        format!("did:aegis:telegram:{admin_chat_id}")
    } else {
        format!("did:aegis:telegram:{chat_id}")
    }
}

/// Ignore bot-authored inbound messages.
///
/// Messages posted by this bot can come back through long-polling as ordinary
/// `message` updates. Those echoes must not enter the normal auth gate, or the
/// bot could reply to its own message with `error.access_denied`.
pub(crate) fn should_ignore_inbound_message(msg: &Message) -> bool {
    msg.from.as_ref().map(|u| u.is_bot).unwrap_or(false)
}

/// Try sending a message with HTML parse mode, optionally inside a forum topic.
/// If Telegram returns a parse error, fall back to plain text (also inside the
/// same topic so the fallback does not leak into a different thread).
/// Retries on 429 rate-limit errors with exponential backoff.
/// Returns the MessageId of the sent message on success.
async fn send_html_impl(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    thread: Option<teloxide::types::ThreadId>,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    use teloxide::payloads::SendMessageSetters;
    let bot_clone = bot.clone();
    let text_owned = text.to_string();
    let result = send_with_retry(|| {
        let b = bot_clone.clone();
        let t = text_owned.clone();
        async move {
            let mut req = b.send_message(chat_id, t).parse_mode(ParseMode::Html);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            req.await
        }
    })
    .await;

    match result {
        Ok(msg) => Ok(msg.id),
        Err(e) if is_parse_error(&e) => {
            tracing::warn!(
                error = %e,
                html_preview = &text[..text.len().min(ERROR_TEXT_PREVIEW_LEN)],
                "HTML parse failed, falling back to plain text"
            );
            let plain = strip_html_tags(text);
            let bot_clone2 = bot.clone();
            send_with_retry(|| {
                let b = bot_clone2.clone();
                let t = plain.clone();
                async move {
                    let mut req = b.send_message(chat_id, t);
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    req.await
                }
            })
            .await
            .map(|msg| msg.id)
        }
        Err(e) => Err(e),
    }
}

/// Try sending a message with HTML parse mode.
/// If Telegram returns a parse error, fall back to plain text.
/// Retries on 429 rate-limit errors with exponential backoff.
/// Returns the MessageId of the sent message on success.
///
/// DM-mode send (no `message_thread_id`). Prefer [`send_html_to_topic`] at new
/// call sites so the forum-mode migration can route to the correct topic.
pub async fn send_html(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    send_html_impl(bot, chat_id, text, None).await
}

/// `send_html`, but routes to the Telegram forum topic registered for
/// `(chat_id, kind)`. In P0 `resolve_thread_id` always returns `None`, so
/// this behaves identically to [`send_html`].
pub async fn send_html_to_topic(
    bot: &Bot,
    chat_id: ChatId,
    kind: TopicKind,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    let thread = wrap_thread_id(resolve_thread_id(chat_id.0, kind).await);
    send_html_impl(bot, chat_id, text, thread).await
}

pub(crate) fn reminder_action_keyboard(task_id: i64, locale: &str) -> InlineKeyboardMarkup {
    use crate::i18n::t;
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                t("reminder.btn.done", locale),
                format!("rem_done:{task_id}"),
            ),
            InlineKeyboardButton::callback(
                t("reminder.btn.snooze", locale),
                format!("rem_snooze:{task_id}"),
            ),
            InlineKeyboardButton::callback(
                t("reminder.btn.delete", locale),
                format!("rem_delete:{task_id}"),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t("reminder.btn.details", locale),
            format!("rem_details:{task_id}"),
        )],
    ])
}

pub async fn send_reminder_html_to_topic(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    task_id: i64,
    locale: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    use teloxide::payloads::SendMessageSetters;
    let thread = wrap_thread_id(resolve_thread_id(chat_id.0, TopicKind::Notif).await);
    let bot_clone = bot.clone();
    let text_owned = text.to_string();
    let keyboard = reminder_action_keyboard(task_id, locale);
    send_with_retry(|| {
        let b = bot_clone.clone();
        let t = text_owned.clone();
        let kb = keyboard.clone();
        async move {
            let mut req = b
                .send_message(chat_id, t)
                .parse_mode(ParseMode::Html)
                .reply_markup(kb);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            req.await
        }
    })
    .await
    .map(|msg| msg.id)
}

fn reminder_snooze_keyboard(task_id: i64, locale: &str) -> InlineKeyboardMarkup {
    use crate::i18n::t;
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                t("reminder.snooze.10min", locale),
                format!("rem_snz10:{task_id}"),
            ),
            InlineKeyboardButton::callback(
                t("reminder.snooze.1hour", locale),
                format!("rem_snz60:{task_id}"),
            ),
            InlineKeyboardButton::callback(
                t("reminder.snooze.tonight", locale),
                format!("rem_snzeve:{task_id}"),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                t("reminder.snooze.tomorrow", locale),
                format!("rem_snztom:{task_id}"),
            ),
            InlineKeyboardButton::callback(
                t("reminder.snooze.next_monday", locale),
                format!("rem_snzmon:{task_id}"),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            t("reminder.btn.cancel", locale),
            format!("rem_cancel:{task_id}"),
        )],
    ])
}

fn reminder_delete_confirm_keyboard(task_id: i64, locale: &str) -> InlineKeyboardMarkup {
    use crate::i18n::t;
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            t("reminder.btn.confirm_delete", locale),
            format!("rem_confirmdel:{task_id}"),
        ),
        InlineKeyboardButton::callback(
            t("reminder.btn.cancel", locale),
            format!("rem_cancel:{task_id}"),
        ),
    ]])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReminderSnoozeOption {
    Minutes10,
    Hour1,
    Tonight,
    Tomorrow,
    NextMonday,
}

impl ReminderSnoozeOption {
    /// Localized duration label (Channel B — owner `display_language`).
    fn label(self, locale: &str) -> &'static str {
        let key = match self {
            Self::Minutes10 => "reminder.snooze.10min",
            Self::Hour1 => "reminder.snooze.1hour",
            Self::Tonight => "reminder.snooze.tonight",
            Self::Tomorrow => "reminder.snooze.tomorrow",
            Self::NextMonday => "reminder.snooze.next_monday",
        };
        crate::i18n::t(key, locale)
    }
}

pub(crate) fn reminder_snooze_target_unix(
    now_utc: chrono::DateTime<chrono::Utc>,
    tz: chrono_tz::Tz,
    option: ReminderSnoozeOption,
) -> i64 {
    use chrono::{Datelike, Duration, LocalResult, TimeZone};
    fn local_time_or_next_valid(
        tz: chrono_tz::Tz,
        date: chrono::NaiveDate,
        hour: u32,
        minute: u32,
    ) -> chrono::DateTime<chrono::Utc> {
        let naive = date
            .and_hms_opt(hour, minute, 0)
            .expect("static reminder snooze time is valid");
        match tz.from_local_datetime(&naive) {
            LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
            LocalResult::Ambiguous(first, _) => first.with_timezone(&chrono::Utc),
            LocalResult::None => match tz.from_local_datetime(&(naive + Duration::hours(1))) {
                LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
                LocalResult::Ambiguous(first, _) => first.with_timezone(&chrono::Utc),
                LocalResult::None => chrono::Utc.from_utc_datetime(&naive),
            },
        }
    }

    match option {
        ReminderSnoozeOption::Minutes10 => (now_utc + Duration::minutes(10)).timestamp(),
        ReminderSnoozeOption::Hour1 => (now_utc + Duration::hours(1)).timestamp(),
        ReminderSnoozeOption::Tonight => {
            let local = now_utc.with_timezone(&tz);
            let target_date = local.date_naive();
            let target = local_time_or_next_valid(tz, target_date, 20, 0);
            if target <= now_utc {
                local_time_or_next_valid(tz, target_date + Duration::days(1), 20, 0).timestamp()
            } else {
                target.timestamp()
            }
        }
        ReminderSnoozeOption::Tomorrow => {
            let local = now_utc.with_timezone(&tz);
            local_time_or_next_valid(tz, local.date_naive() + Duration::days(1), 9, 0).timestamp()
        }
        ReminderSnoozeOption::NextMonday => {
            let local = now_utc.with_timezone(&tz);
            let days_until_monday = (8 - i64::from(local.weekday().number_from_monday())) % 7;
            let days = if days_until_monday == 0 {
                7
            } else {
                days_until_monday
            };
            local_time_or_next_valid(tz, local.date_naive() + Duration::days(days), 9, 0)
                .timestamp()
        }
    }
}

/// Internal helper — see [`send_html_no_preview`]. Optionally places the
/// message inside a forum topic.
async fn send_html_no_preview_impl(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    thread: Option<teloxide::types::ThreadId>,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    use teloxide::payloads::SendMessageSetters;
    let no_preview = LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    };
    let bot_clone = bot.clone();
    let text_owned = text.to_string();
    let result = send_with_retry(|| {
        let b = bot_clone.clone();
        let t = text_owned.clone();
        let lpo = no_preview.clone();
        async move {
            let mut req = b
                .send_message(chat_id, t)
                .parse_mode(ParseMode::Html)
                .link_preview_options(lpo);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            req.await
        }
    })
    .await;

    match result {
        Ok(msg) => Ok(msg.id),
        Err(e) if is_parse_error(&e) => {
            tracing::warn!(
                error = %e,
                html_preview = &text[..text.len().min(ERROR_TEXT_PREVIEW_LEN)],
                "HTML parse failed (no_preview), falling back to plain text"
            );
            let plain = strip_html_tags(text);
            let bot_clone2 = bot.clone();
            send_with_retry(|| {
                let b = bot_clone2.clone();
                let t = plain.clone();
                async move {
                    let mut req = b.send_message(chat_id, t);
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    req.await
                }
            })
            .await
            .map(|msg| msg.id)
        }
        Err(e) => Err(e),
    }
}

/// Like `send_html`, but with link preview disabled.
/// Keeps chat clean when messages contain raw URLs.
///
/// DM-mode send (no `message_thread_id`). Prefer [`send_html_no_preview_to_topic`]
/// at new call sites.
pub async fn send_html_no_preview(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    send_html_no_preview_impl(bot, chat_id, text, None).await
}

/// `send_html_no_preview` routed to the Telegram forum topic for
/// `(chat_id, kind)`. P0 resolves to `None`, so behavior matches
/// [`send_html_no_preview`].
pub async fn send_html_no_preview_to_topic(
    bot: &Bot,
    chat_id: ChatId,
    kind: TopicKind,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    let thread = wrap_thread_id(resolve_thread_id(chat_id.0, kind).await);
    send_html_no_preview_impl(bot, chat_id, text, thread).await
}

/// Internal helper — see [`send_md`]. Optionally places the message inside a
/// forum topic.
async fn send_md_impl(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    thread: Option<teloxide::types::ThreadId>,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    use teloxide::payloads::SendMessageSetters;
    let bot_clone = bot.clone();
    let text_owned = text.to_string();
    let result = send_with_retry(|| {
        let b = bot_clone.clone();
        let t = text_owned.clone();
        async move {
            let mut req = b.send_message(chat_id, t).parse_mode(ParseMode::MarkdownV2);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            req.await
        }
    })
    .await;

    match result {
        Ok(msg) => Ok(msg.id),
        Err(e) if is_parse_error(&e) => {
            tracing::warn!(error = %e, "MarkdownV2 parse failed, falling back to plain text");
            let bot_clone2 = bot.clone();
            let text_owned2 = text.to_string();
            send_with_retry(|| {
                let b = bot_clone2.clone();
                let t = text_owned2.clone();
                async move {
                    let mut req = b.send_message(chat_id, t);
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    req.await
                }
            })
            .await
            .map(|msg| msg.id)
        }
        Err(e) => Err(e),
    }
}

/// Try sending a message with MarkdownV2 parse mode.
/// If Telegram returns a parse error (HTTP 400), fall back to plain text.
/// Retries on 429 rate-limit errors with exponential backoff.
///
/// DM-mode send (no `message_thread_id`). Prefer [`send_md_to_topic`] at new
/// call sites.
pub async fn send_md(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    send_md_impl(bot, chat_id, text, None).await
}

/// `send_md` routed to the Telegram forum topic for `(chat_id, kind)`.
/// P0 resolves to `None`, so behavior matches [`send_md`].
pub async fn send_md_to_topic(
    bot: &Bot,
    chat_id: ChatId,
    kind: TopicKind,
    text: &str,
) -> std::result::Result<MessageId, teloxide::RequestError> {
    let thread = wrap_thread_id(resolve_thread_id(chat_id.0, kind).await);
    send_md_impl(bot, chat_id, text, thread).await
}

/// Check if a teloxide error is a Telegram "Bad Request: can't parse" error.
fn is_parse_error(err: &teloxide::RequestError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("can't parse") || (msg.contains("bad request") && msg.contains("parse"))
}

// ── Bot commands ────────────────────────────────────────────────────────

/// Core commands that are always present in the Telegram menu.
/// These cannot be removed by the LLM or user — only custom commands can be added/removed.
/// Naming convention: full snake_case words (consistent with skill commands).
/// `/heartbeat` and `/admin` were retired with the SDUI panels they opened
/// (2026-09-16): nothing intercepts them any more, so they are not advertised.
pub fn core_commands() -> Vec<BotCommand> {
    vec![BotCommand::new(
        "tasks",
        "\u{1f4cb} \u{4efb}\u{52a1}\u{7ba1}\u{7406} Tasks",
    )]
}

/// Reserved command names that custom/skill commands may not shadow: the
/// commands the orchestrator intercepts before the LLM (`/hb` runs a manual
/// heartbeat; see `intercept_panel_command`).
/// NOTE: "tasks" is intentionally excluded — it is advertised via
/// `core_commands()` but reaches the LLM as a plain message.
pub fn core_command_names() -> std::collections::HashSet<&'static str> {
    ["hb"].into_iter().collect()
}

/// Map legacy single-letter command aliases to their full names.
/// Returns None if the command is not a legacy alias.
#[deprecated(note = "All legacy aliases removed. Callers should be cleaned up.")]
pub fn resolve_command_alias(_cmd: &str) -> Option<&'static str> {
    None
}

/// Legacy alias — returns core commands only (custom commands are loaded from DB).
pub fn bot_commands() -> Vec<BotCommand> {
    core_commands()
}

// ── Reactions ───────────────────────────────────────────────────────────

/// Set a single emoji reaction on a message. Best-effort (errors are logged).
pub async fn react(bot: &Bot, chat_id: ChatId, message_id: MessageId, emoji: &str) {
    let reaction = vec![ReactionType::Emoji {
        emoji: emoji.to_string(),
    }];
    if let Err(e) = bot
        .set_message_reaction(chat_id, message_id)
        .reaction(reaction)
        .await
    {
        tracing::debug!(error = %e, emoji, "Failed to set message reaction (non-fatal)");
    }
}

// ── Document sending ────────────────────────────────────────────────────

/// Send a long text reply as a `.txt` document instead of many message chunks.
///
/// DM-mode send (no `message_thread_id`). Prefer [`send_as_document_to_topic`]
/// at new call sites so the forum-mode migration can route to the correct
/// topic.
pub async fn send_as_document(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    caption: &str,
) -> std::result::Result<(), teloxide::RequestError> {
    send_as_document_impl(bot, chat_id, text, caption, None).await
}

/// `send_as_document` routed to the Telegram forum topic for
/// `(chat_id, kind)`. P0 resolves to `None`, so behavior matches
/// [`send_as_document`].
pub async fn send_as_document_to_topic(
    bot: &Bot,
    chat_id: ChatId,
    kind: TopicKind,
    text: &str,
    caption: &str,
) -> std::result::Result<(), teloxide::RequestError> {
    let thread = wrap_thread_id(resolve_thread_id(chat_id.0, kind).await);
    send_as_document_impl(bot, chat_id, text, caption, thread).await
}

/// Internal helper — see [`send_as_document`]. Optionally places the document
/// inside a forum topic.
async fn send_as_document_impl(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    caption: &str,
    thread: Option<teloxide::types::ThreadId>,
) -> std::result::Result<(), teloxide::RequestError> {
    use teloxide::payloads::SendDocumentSetters;
    let file = InputFile::memory(text.as_bytes().to_vec()).file_name("reply.txt");
    let mut req = bot.send_document(chat_id, file).caption(caption);
    if let Some(tid) = thread {
        req = req.message_thread_id(tid);
    }
    req.await?;
    Ok(())
}

// ── Pin / Unpin messages ─────────────────────────────────────────────────

/// Pin a message in the chat (silently). Best-effort.
pub async fn pin_message(bot: &Bot, chat_id: ChatId, message_id: MessageId) {
    if let Err(e) = bot
        .pin_chat_message(chat_id, message_id)
        .disable_notification(true)
        .await
    {
        tracing::warn!(error = %e, "Failed to pin message (non-fatal)");
    }
}

/// Unpin all messages in the chat. Best-effort.
/// Used when an alert resolves so stale alert pins don't accumulate.
pub async fn unpin_all_messages(bot: &Bot, chat_id: ChatId) {
    if let Err(e) = bot.unpin_all_chat_messages(chat_id).await {
        tracing::warn!(error = %e, "Failed to unpin all messages (non-fatal)");
    }
}

/// Telegram bot handler for Aegis Runtime.
///
/// Receives user messages -> orchestrator.process_message() -> replies.
/// Also implements HeartbeatHandler for Chronos alert escalation.
/// Background `get_me()` probe that keeps the Telegram component of
/// `GET /health` honest after startup.
///
/// Incident 2026-09-15: the token was revoked while the process was running;
/// the only trace was `Api(InvalidToken)` inside teloxide's update listener
/// and `/health` stayed `ok`. This probe records the classified result every
/// [`crate::telegram_health::PROBE_INTERVAL_SECS`], logs at ERROR on the
/// first transition to `InvalidToken` (WARN on repeats, so a dead token does
/// not spam ERROR every five minutes) and at INFO on recovery.
fn spawn_telegram_health_probe(bot: Bot) {
    use crate::telegram_health::{self, PROBE_INTERVAL_SECS, TelegramState};

    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(PROBE_INTERVAL_SECS);
        loop {
            tokio::time::sleep(interval).await;
            match bot.get_me().await {
                Ok(me) => {
                    let previous = telegram_health::record_reachable(me.username());
                    if previous != TelegramState::Reachable {
                        tracing::info!(
                            bot_username = %me.username(),
                            previous_state = ?previous,
                            "Telegram API reachable again (health probe)"
                        );
                    }
                }
                Err(e) => {
                    let state = telegram_health::classify(&e);
                    let previous = telegram_health::record_state(state, Some(e.to_string()));
                    match state {
                        TelegramState::InvalidToken if previous != TelegramState::InvalidToken => {
                            tracing::error!(
                                error = %e,
                                "Telegram rejected the bot token (401 Unauthorized) after startup. Check TELOXIDE_TOKEN; the bot stays offline until the token is fixed and the service restarted"
                            );
                        }
                        TelegramState::InvalidToken => tracing::warn!(
                            error = %e,
                            "Telegram bot token still rejected (health probe)"
                        ),
                        _ => tracing::warn!(
                            error = %e,
                            classification = "network",
                            "Telegram health probe failed"
                        ),
                    }
                }
            }
        }
    });
}

pub struct TelegramBot {
    bot: Bot,
    admin_chat_id: i64,
    /// Multimodal media processor (voice STT, image vision).
    pub media_processor: Arc<MediaProcessor>,
    /// Tracks active progress messages per chat for progressive editing.
    pub progress_messages: ProgressTracker,
}

impl TelegramBot {
    pub fn new(admin_chat_id: i64) -> Self {
        // teloxide reads TELOXIDE_TOKEN from env automatically
        let bot = Bot::from_env();
        let bot_token = std::env::var("TELOXIDE_TOKEN").unwrap_or_else(|_| {
            tracing::warn!("TELOXIDE_TOKEN not set — media processing will fail");
            String::new()
        });
        Self {
            bot,
            admin_chat_id,
            media_processor: Arc::new(MediaProcessor::new(&bot_token)),
            progress_messages: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Get a clone of the inner Bot for sending messages.
    pub fn bot(&self) -> Bot {
        self.bot.clone()
    }

    /// Start the Telegram long-polling loop.
    ///
    /// Each incoming message is forwarded to the orchestrator for processing.
    /// Only messages from ADMIN_CHAT_ID are accepted (single-user MVP).
    ///
    /// **DNS resilience**: Before entering teloxide's `dispatch()` loop, we
    /// verify connectivity via `bot.get_me()` with exponential backoff.
    /// teloxide panics (exit 101) if `get_me()` fails during `dispatch()`,
    /// so we absorb transient DNS / network outages here instead of letting
    /// systemd restart-loop the entire process.
    pub async fn start(self: Arc<Self>, orchestrator: Arc<Orchestrator>) {
        use crate::telegram_health::{self, TelegramState};

        telegram_health::record_state(TelegramState::Starting, None);

        // ── Wait for network / DNS to be available ──────────────────────
        // Retry get_me() with exponential backoff: 5s, 10s, 20s, 40s, 60s, 60s, ...
        //
        // Incident 2026-09-15: a revoked token (401) was retried under the
        // same "DNS/network" WARN every 60 s for hours. A 401 is classified
        // separately, logged at ERROR, retried every 5 min, and published on
        // /health so the smoke test and the operator see it.
        {
            let mut delay = std::time::Duration::from_secs(5);
            let max_delay = std::time::Duration::from_secs(60);
            loop {
                match self.bot.get_me().await {
                    Ok(me) => {
                        tracing::info!(
                            bot_username = %me.username(),
                            "Telegram API reachable, starting dispatcher"
                        );
                        telegram_health::record_reachable(me.username());
                        break;
                    }
                    Err(e) => {
                        let state = telegram_health::classify(&e);
                        telegram_health::record_state(state, Some(e.to_string()));
                        let retry_in = telegram_health::retry_delay(state, delay);
                        match state {
                            TelegramState::InvalidToken => tracing::error!(
                                error = %e,
                                retry_in_secs = retry_in.as_secs(),
                                "Telegram rejected the bot token (401 Unauthorized). Check TELOXIDE_TOKEN; the bot stays offline until the token is fixed"
                            ),
                            _ => {
                                tracing::warn!(
                                    error = %e,
                                    classification = "network",
                                    retry_in_secs = retry_in.as_secs(),
                                    "Telegram API unreachable (DNS/network), retrying\u{2026}"
                                );
                                delay = (delay * 2).min(max_delay);
                            }
                        }
                        tokio::time::sleep(retry_in).await;
                    }
                }
            }
        }

        // A token revoked AFTER startup (the first process this morning) only
        // surfaced as `Api(InvalidToken)` inside teloxide's update listener.
        // Probe get_me() every 5 min so /health reflects it.
        spawn_telegram_health_probe(self.bot.clone());

        // Register bot commands in Telegram's menu button.
        // Load persisted custom commands if any, otherwise use defaults.
        let cmds = orchestrator.get_startup_bot_commands().await;
        match self.bot.set_my_commands(cmds).await {
            Ok(_) => tracing::info!("Bot commands registered successfully"),
            Err(e) => tracing::warn!(error = %e, "Failed to register bot commands (non-fatal)"),
        }

        // Routine startup status belongs in the Web owner timeline. Telegram
        // remains a notification surface for startup failures or alerts.
        {
            use teloxide::types::{KeyboardButton, KeyboardMarkup};
            let locale = orchestrator.settings.display_language();
            use teloxide::payloads::SendMessageSetters;
            let startup_html = build_startup_status_html(orchestrator.as_ref()).await;
            // Startup status is operational noise, not a conversation turn: it
            // used to be recorded as an assistant message in L1/L3 on every
            // restart, and memory recall then surfaced "系统已重启" ×N as
            // "relevant memory" for a greeting (owner report 2026-09-06).
            // Telegram delivery (when warranted) is the only surface now.
            if !should_send_startup_status_to_telegram(&startup_html) {
                tracing::info!(
                    "[Delivery] Recorded routine startup status for Web; Telegram startup notice suppressed"
                );
            } else {
                let route = route_telegram_notification(
                    self.admin_chat_id,
                    proactive_route_request(
                        NotificationClass::StartupKeyboard,
                        PrivacyLevel::SystemOnly,
                        None,
                    ),
                );
                let thread = wrap_thread_id(route.thread_id);
                let mut req = self
                    .bot
                    .send_message(ChatId(route.chat_id), startup_html)
                    .parse_mode(ParseMode::Html);
                if let Some(tid) = thread {
                    req = req.message_thread_id(tid);
                }
                if should_attach_startup_reply_keyboard(self.admin_chat_id, &route) {
                    let keyboard = KeyboardMarkup::new(vec![
                        vec![
                            KeyboardButton::new(t("telegram.kb_status", &locale)),
                            KeyboardButton::new(t("telegram.kb_recall", &locale)),
                        ],
                        vec![
                            KeyboardButton::new(t("telegram.kb_tasks", &locale)),
                            KeyboardButton::new(t("telegram.kb_search", &locale)),
                            KeyboardButton::new(t("telegram.kb_settings", &locale)),
                        ],
                    ])
                    .resize_keyboard()
                    .persistent()
                    .input_field_placeholder(t("telegram.keyboard_placeholder", &locale));
                    req = req.reply_markup(keyboard);
                }
                if let Err(e) = req.await {
                    tracing::warn!(error = %e, "Failed to send startup status (non-fatal)");
                }
            }
        }

        let handler = Update::filter_message().endpoint(
            move |bot: Bot, msg: Message, orch: Arc<Orchestrator>, tg: Arc<TelegramBot>| async move {
                if should_ignore_inbound_message(&msg) {
                    tracing::debug!(
                        chat_id = %msg.chat.id,
                        sender_uid = ?msg.from.as_ref().map(|u| u.id.0 as i64),
                        username = ?msg.from.as_ref().and_then(|u| u.username.as_deref()),
                        "Ignoring bot-authored inbound Telegram message"
                    );
                    return respond(());
                }

                // Authorization: the owner's admin DM only. (Linked-forum
                // group routing was removed — everything is DM-by-default.)
                let sender_uid = msg.from.as_ref().map(|u| u.id.0 as i64);
                if !is_authorized_chat(msg.chat.id.0, sender_uid, tg.admin_chat_id) {
                    tracing::warn!(
                        chat_id = %msg.chat.id,
                        sender_uid = ?sender_uid,
                        username = ?msg.from.as_ref().and_then(|u| u.username.as_deref()),
                        "[Security] Unauthorized access attempt"
                    );
                    // Reply "access denied" ONLY into a private chat (a stray
                    // DM from a non-owner — reactive, single-recipient, useful
                    // feedback). NEVER reply into a group/supergroup/channel:
                    // after forum routing was removed, resolve_thread_id is
                    // always None, so a group reply would post plainly into a
                    // raw non-admin group (e.g. the now-defunct linked forum
                    // -1001000000001) — group spam + a raw non-admin send that
                    // violates the DM-only invariant (independent Codex R1).
                    // Non-private unauthorized chats are silently left.
                    if msg.chat.is_private() {
                        let locale = orch.settings.display_language();
                        send_html_to_topic(
                            &bot,
                            msg.chat.id,
                            TopicKind::Main,
                            &escape_html(t("error.access_denied", &locale)),
                        )
                        .await?;
                    }
                    return respond(());
                }

                // ── Extract media from message ──
                let voice_info: Option<(String, u32)> = msg.voice()
                    .map(|v| (v.file.id.0.clone(), v.duration.seconds()))
                    .or_else(|| msg.audio().map(|a| (a.file.id.0.clone(), a.duration.seconds())));

                let photo_file_id: Option<String> = msg.photo()
                    .and_then(|photos| photos.last().map(|p| p.file.id.0.clone()));

                let document_info: Option<(String, Option<String>, Option<String>)> = msg.document()
                    .map(|d| (
                        d.file.id.0.clone(),
                        d.mime_type.as_ref().map(|m| m.to_string()),
                        d.file_name.clone(),
                    ));

                let video_info: Option<(String, u32, Option<String>)> = msg.video()
                    .map(|v| (
                        v.file.id.0.clone(),
                        v.duration.seconds(),
                        v.file_name.clone(),
                    ))
                    .or_else(|| msg.video_note().map(|vn| (
                        vn.file.id.0.clone(),
                        vn.duration.seconds(),
                        None,
                    )));

                // Extract text + quoted reply context
                let own_text = msg.text()
                    .or_else(|| msg.caption())
                    .unwrap_or("")
                    .trim();

                // Reject excessively large messages to prevent OOM / embedding timeout.
                const MAX_INPUT_LEN: usize = 64 * 1024; // 64 KB

                // If user replied to a previous message, prepend the quoted content
                // so the LLM has full context of what the user is referring to.
                let combined_text = if let Some(reply) = msg.reply_to_message() {
                    let quoted = reply.text()
                        .or_else(|| reply.caption())
                        .unwrap_or("");
                    // Truncate quoted content to prevent oversized payloads from
                    // reply-to messages bypassing the MAX_INPUT_LEN check below.
                    let quoted = &quoted[..quoted.floor_char_boundary(MAX_INPUT_LEN)];
                    let qp = crate::i18n::t("telegram.quote_prefix", crate::i18n::DEFAULT_LOCALE);
                    let rp = crate::i18n::t("telegram.reply_prefix", crate::i18n::DEFAULT_LOCALE);
                    if !quoted.is_empty() && !own_text.is_empty() {
                        format!("{} {}\n\n{} {}", qp, quoted.trim(), rp, own_text)
                    } else if !quoted.is_empty() {
                        format!("{} {}", qp, quoted.trim())
                    } else {
                        own_text.to_string()
                    }
                } else {
                    own_text.to_string()
                };
                let raw_text = combined_text.trim();

                // Skip truly empty messages (no text, no media)
                let has_media = voice_info.is_some()
                    || photo_file_id.is_some()
                    || document_info.is_some()
                    || video_info.is_some();
                if raw_text.is_empty() && !has_media {
                    return respond(());
                }

                if raw_text.len() > MAX_INPUT_LEN {
                    use teloxide::payloads::SendMessageSetters;
                    let thread = wrap_thread_id(
                        resolve_thread_id(msg.chat.id.0, TopicKind::Main).await,
                    );
                    let mut req = bot.send_message(
                        msg.chat.id,
                        crate::i18n::t(
                            "telegram.message_too_long",
                            crate::i18n::DEFAULT_LOCALE,
                        ),
                    );
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    req.await?;
                    return respond(());
                }

                // For text-based checks below, use raw_text
                let text = raw_text;

                // ── Deep linking: /start PARAMETER ──
                // Handles deep links like t.me/botname?start=oauth_success
                if let Some(param) = text.strip_prefix("/start ") {
                    let param = param.trim();
                    if param.starts_with("oauth_") {
                        // OAuth deep link — confirm success through the router's
                        // explicit DM exception instead of topic/group policy.
                        let locale = orch.settings.display_language();
                        let route = oauth_deeplink_reply_route(tg.admin_chat_id, msg.chat.id.0);
                        let _ = send_html_impl(
                            &bot,
                            ChatId(route.chat_id),
                            &markdown_to_telegram_html(
                                t("telegram.oauth_deeplink_success", &locale),
                            ),
                            wrap_thread_id(route.thread_id),
                        )
                        .await;
                        return respond(());
                    }
                    // Unknown /start parameter — treat as regular message
                    // so the LLM can handle it
                }

                // ── Credential paste detection ──
                // Intercept Google OAuth credentials before they reach the LLM.
                // This prevents secrets from being sent to the LLM API.
                if let Some((client_id, client_secret)) = detect_google_credentials(text) {
                    match orch.store_google_client_credentials(&client_id, &client_secret) {
                        Ok(()) => {
                            // Try to delete the message containing secrets
                            let _ = bot.delete_message(msg.chat.id, msg.id).await;
                            if let Err(e) = send_html_to_topic(
                                &bot,
                                msg.chat.id,
                                TopicKind::System,
                                &markdown_to_telegram_html(
                                    "\u{2705} Google OAuth \u{51ed}\u{8bc1}\u{5df2}\u{5b89}\u{5168}\u{5b58}\u{50a8}\u{3002}\n\n\
                                     \u{4f60}\u{7684}\u{6d88}\u{606f}\u{5df2}\u{88ab}\u{5220}\u{9664}\u{4ee5}\u{4fdd}\u{62a4}\u{51ed}\u{8bc1}\u{5b89}\u{5168}\u{3002}\n\
                                     \u{73b0}\u{5728}\u{4f60}\u{53ef}\u{4ee5}\u{8bf4}\u{300c}\u{5e2e}\u{6211}\u{8fde}\u{63a5} Google Calendar\u{300d}\u{6765}\u{5b8c}\u{6210}\u{6388}\u{6743}\u{3002}"
                                ),
                            ).await {
                                tracing::warn!(error = %e, "Failed to send credential storage confirmation to user");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "Failed to store Google credentials");
                            if let Err(send_err) = send_html_to_topic(
                                &bot,
                                msg.chat.id,
                                TopicKind::System,
                                &markdown_to_telegram_html(
                                    &format!("\u{274c} \u{51ed}\u{8bc1}\u{5b58}\u{50a8}\u{5931}\u{8d25}: {e}\n\u{8bf7}\u{68c0}\u{67e5}\u{7cfb}\u{7edf}\u{914d}\u{7f6e}\u{540e}\u{91cd}\u{8bd5}\u{3002}")
                                ),
                            ).await {
                                tracing::warn!(error = %send_err, "Failed to send credential storage error notification to user");
                            }
                        }
                    }
                    return respond(());
                }

                // ── Loose credential leak guard ──
                // If user embeds credentials in natural language, block and warn.
                // This catches "my client_id is xxx and client_secret is yyy" patterns
                // that the structured detector above misses.
                if detect_credential_leak(text) {
                    let _ = bot.delete_message(msg.chat.id, msg.id).await;
                    let _ = send_html_to_topic(
                        &bot,
                        msg.chat.id,
                        TopicKind::System,
                        &markdown_to_telegram_html(
                            "\u{26a0}\u{fe0f} \u{68c0}\u{6d4b}\u{5230}\u{4f60}\u{7684}\u{6d88}\u{606f}\u{4e2d}\u{53ef}\u{80fd}\u{5305}\u{542b} OAuth \u{51ed}\u{8bc1}\u{3002}\n\n\
                             \u{4e3a}\u{4e86}\u{5b89}\u{5168}\u{ff0c}\u{6d88}\u{606f}\u{5df2}\u{88ab}\u{62e6}\u{622a}\u{5220}\u{9664}\u{ff08}\u{51ed}\u{8bc1}\u{4e0d}\u{4f1a}\u{53d1}\u{9001}\u{7ed9} AI\u{ff09}\u{3002}\n\n\
                             \u{8bf7}\u{7528}\u{4ee5}\u{4e0b}\u{683c}\u{5f0f}\u{5355}\u{72ec}\u{53d1}\u{9001}\u{4e00}\u{6761}\u{6d88}\u{606f}\u{ff1a}\n\
                             ```\nclient_id: \u{4f60}\u{7684}ID\nclient_secret: \u{4f60}\u{7684}\u{5bc6}\u{94a5}\n```\n\
                             \u{7cfb}\u{7edf}\u{4f1a}\u{81ea}\u{52a8}\u{68c0}\u{6d4b}\u{5e76}\u{5b89}\u{5168}\u{5b58}\u{50a8}\u{3002}"
                        ),
                    ).await;
                    return respond(());
                }

                // ── Google auth code paste detection (Scenario C: no public URL) ──
                // Detect Google authorization codes (format: 4/0A...) when OAuth is pending.
                if detect_google_auth_code(text)
                    && orch.has_pending_oauth_setup_for_chat(Some(msg.chat.id.0))
                {
                    react(&bot, msg.chat.id, msg.id, "\u{1f440}").await;
                    let code = text.trim();
                    match orch
                        .exchange_google_auth_code_for_chat(code, Some(msg.chat.id.0))
                        .await
                    {
                        Ok(completion_message) => {
                            // Full user-facing message built by the shared
                            // completion builder — base
                            // success text + the restart-transparency hint
                            // when a NEW account was added at runtime.
                            react(&bot, msg.chat.id, msg.id, "\u{1f44d}").await;
                            let _ = send_html_to_topic(
                                &bot,
                                msg.chat.id,
                                TopicKind::System,
                                &markdown_to_telegram_html(&completion_message),
                            ).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "Google auth code exchange failed");
                            let _ = send_html_to_topic(
                                &bot,
                                msg.chat.id,
                                TopicKind::System,
                                &markdown_to_telegram_html(
                                    &format!("\u{274c} \u{6388}\u{6743}\u{7801}\u{4ea4}\u{6362}\u{5931}\u{8d25}: {e}\n\u{8bf7}\u{786e}\u{8ba4}\u{6388}\u{6743}\u{7801}\u{662f}\u{5426}\u{6b63}\u{786e}\u{ff0c}\u{6216}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{6388}\u{6743}\u{6d41}\u{7a0b}\u{3002}")
                                ),
                            ).await;
                        }
                    }
                    return respond(());
                }

                // NOTE: /heartbeat and /admin were retired with the SDUI panels
                // they used to open (2026-09-16) and are no longer advertised in
                // the menu; /hb triggers a manual heartbeat in the orchestrator.

                // ── Manual memo archive ──
                let archive_cmd = text == "/archive"
                    || text.starts_with("/archive@");
                if archive_cmd {
                    let orch_clone = orch.clone();
                    let bot_clone = bot.clone();
                    let chat_id = msg.chat.id;
                    tokio::spawn(async move {
                        match orch_clone.trigger_memo_archive(chat_id.0).await {
                            Ok(Some(content)) => {
                                let preview: String = content.chars().take(200).collect();
                                let _ = send_html_to_topic(
                                    &bot_clone,
                                    chat_id,
                                    TopicKind::Main,
                                    &format!(
                                        "\u{1f4e6} \u{4fbf}\u{7b7e}\u{5df2}\u{5f52}\u{6863}\u{5230}\u{957f}\u{671f}\u{8bb0}\u{5fc6}\u{ff08}{}\u{5b57}\u{ff09}\n\n<i>{}</i>{}",
                                        content.len(),
                                        teloxide::utils::html::escape(&preview),
                                        if content.len() > 200 { "..." } else { "" }
                                    ),
                                ).await;
                            }
                            Ok(None) => {
                                let _ = send_html_to_topic(
                                    &bot_clone,
                                    chat_id,
                                    TopicKind::Main,
                                    "\u{1f4e6} \u{5f53}\u{524d}\u{4fbf}\u{7b7e}\u{4e3a}\u{7a7a}\u{ff0c}\u{65e0}\u{9700}\u{5f52}\u{6863}\u{3002}",
                                ).await;
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Manual memo archive failed");
                                let _ = send_html_to_topic(
                                    &bot_clone,
                                    chat_id,
                                    TopicKind::Main,
                                    &format!("\u{274c} \u{5f52}\u{6863}\u{5931}\u{8d25}: {}", escape_html(&e.to_string())),
                                ).await;
                            }
                        }
                    });
                    return respond(());
                }

                // React with eyes to acknowledge receipt
                react(&bot, msg.chat.id, msg.id, "\u{1f440}").await;

                let user_did =
                    user_did_for_authorized_chat(msg.chat.id.0, sender_uid, tg.admin_chat_id);

                // ── Process multimodal content ──
                let media_proc = tg.media_processor.clone();
                let model_supports_audio = orch.supports_audio_input();
                let processed = media_proc.process_telegram_message(
                    if text.is_empty() { None } else { Some(text) },
                    voice_info.as_ref().map(|(id, dur)| (id.as_str(), *dur)),
                    photo_file_id.as_deref(),
                    document_info.as_ref().map(|(id, mime, fname)| (id.as_str(), mime.as_deref(), fname.as_deref())),
                    video_info.as_ref().map(|(id, dur, fname)| (id.as_str(), *dur, fname.as_deref())),
                    model_supports_audio,
                ).await;

                let (final_text, media_attachments) = match processed {
                    Ok(pm) => (pm.text, pm.attachments),
                    Err(e) => {
                        tracing::warn!(error = %format!("{e:#}"), "multimodal processing failed");
                        (text.to_string(), vec![])
                    }
                };

                // Notify user about voice processing method
                if voice_info.is_some() && !orch.supports_audio_input() {
                    let msg_key = if final_text.is_empty() {
                        // STT failed or produced empty result
                        if tg.media_processor.is_stt_configured() {
                            "telegram.audio_stt_failed"
                        } else {
                            "telegram.audio_stt_not_configured"
                        }
                    } else {
                        "telegram.audio_not_supported"
                    };
                    use teloxide::payloads::SendMessageSetters;
                    let thread = wrap_thread_id(
                        resolve_thread_id(msg.chat.id.0, TopicKind::Main).await,
                    );
                    let mut req = bot.send_message(
                        msg.chat.id,
                        crate::i18n::t(msg_key, crate::i18n::DEFAULT_LOCALE),
                    );
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    let _ = req.await;
                }

                if final_text.is_empty() && media_attachments.is_empty() {
                    return respond(());
                }

                // CEO Intent Contract: same shorthand + free-text chief-of-staff
                // frame as WebApp general chat (docs/design/2026-07-25-ceo-intent-contract.md).
                let locale = orch.settings.display_language();
                let prefer_zh = matches!(
                    locale.as_str(),
                    "zh" | "zh-CN" | "zh-Hans" | "zh_CN" | "zh_Hans"
                ) || locale.to_lowercase().starts_with("zh");
                let (runtime_text, _ceo_hit) =
                    crate::ceo_intent::prepare_ceo_runtime_user_text(&final_text, prefer_zh);

                let result = if media_attachments.is_empty() {
                    orch.process_message(&user_did, &runtime_text, Some(msg.chat.id.0))
                        .await
                } else {
                    orch.process_message_with_media(
                        &user_did,
                        &runtime_text,
                        &media_attachments,
                        Some(msg.chat.id.0),
                    )
                    .await
                };

                match result {
                    Ok(result) => {
                        // React with thumbs up on success (Telegram only allows specific emoji for bot reactions)
                        react(&bot, msg.chat.id, msg.id, "\u{1f44d}").await;

                        // Log multi-round usage for observability
                        if result.rounds_used > 1 {
                            tracing::info!(
                                rounds = result.rounds_used,
                                tools = ?result.tools_called,
                                "Multi-round ReAct turn completed"
                            );
                        }

                        let mut reply = result.reply.clone();

                        // Strip callback protocol tokens from direct messages — LLM sometimes
                        // emits [NO_ACTION] even in non-callback context. Keep the natural text.
                        if reply.contains("[NO_ACTION]") {
                            reply = reply.replace("[NO_ACTION]", "").trim().to_string();
                        }

                        // Append blocked tool warnings (exclude tools that were
                        // retried successfully — LLM often retries with corrected
                        // arguments in a later round)
                        let truly_blocked: Vec<_> = result.tools_blocked.iter()
                            .filter(|t| !result.tools_called.contains(t))
                            .collect();
                        if !truly_blocked.is_empty() {
                            reply.push_str("\n\n\u{1f6ab} [Aegis Runtime Safety] Blocked tools: ");
                            reply.push_str(&truly_blocked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
                        }

                        // Guard against empty replies (Telegram API rejects empty messages)
                        if reply.trim().is_empty() {
                            if result.tools_called.is_empty() {
                                reply = "\u{1f914} \u{6211}\u{4e0d}\u{786e}\u{5b9a}\u{5982}\u{4f55}\u{56de}\u{5e94}\u{ff0c}\u{8bf7}\u{6362}\u{4e2a}\u{65b9}\u{5f0f}\u{518d}\u{8bd5}\u{4e00}\u{6b21}\u{3002}".to_string();
                            } else {
                                // Natural acknowledgment — the orchestrator Empty Retry nudge should prevent
                                // this from happening, but if it does, don't dump tool names on the user.
                                reply = "\u{2705} \u{5df2}\u{5904}\u{7406}\u{5b8c}\u{6bd5}\u{3002}".to_string();
                            }
                        }

                        // Tool disclosure: in debug mode, append tool summary after a separator
                        if !result.tools_called.is_empty() {
                            let disclosure = orch.settings.get("tool_disclosure")
                                .and_then(|v| v.as_str().map(|s| s.to_string()))
                                .unwrap_or_else(|| "natural".to_string());
                            if disclosure == "debug" {
                                reply.push_str("\n\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
                                for tool in &result.tools_called {
                                    reply.push_str(&format!("\n\u{1f527} {tool}"));
                                }
                            }
                        }

                        // Convert LLM markdown to Telegram HTML and send
                        let html_reply = markdown_to_telegram_html(&reply);
                        let chunks = split_message(&html_reply, 4096);
                        tracing::debug!(
                            original_len = reply.len(),
                            html_len = html_reply.len(),
                            chunks = chunks.len(),
                            "Sending LLM reply"
                        );

                        // Clean up progressive editing status message (scope lock before await)
                        let pmid = { tg.progress_messages.lock().unwrap_or_else(|e| e.into_inner()).remove(&msg.chat.id.0) };
                        if let Some(pmid) = pmid
                            && let Err(e) = bot.delete_message(msg.chat.id, pmid).await
                        {
                            tracing::debug!(error = %e, "Progress message already gone");
                        }

                        // Long outputs -> send as document to avoid message spam
                        if chunks.len() > LONG_OUTPUT_CHUNK_THRESHOLD {
                            // Send the plain text as a .txt file with a brief caption
                            let caption = format!(
                                "\u{1f4c4} Reply ({} chars, too long for inline)",
                                reply.len()
                            );
                            if let Err(e) = send_as_document_to_topic(
                                &bot,
                                msg.chat.id,
                                TopicKind::Main,
                                &reply,
                                &caption,
                            )
                            .await
                            {
                                tracing::warn!(error = %e, "Document send failed, falling back to chunks");
                                for chunk in &chunks {
                                    send_html_no_preview_to_topic(
                                        &bot,
                                        msg.chat.id,
                                        TopicKind::Main,
                                        chunk,
                                    )
                                    .await?;
                                }
                            }
                        } else {
                            for chunk in &chunks {
                                send_html_no_preview_to_topic(
                                    &bot,
                                    msg.chat.id,
                                    TopicKind::Main,
                                    chunk,
                                )
                                .await?;
                            }
                        }
                    }
                    Err(e) => {
                        // React with broken heart on error (Telegram only allows specific emoji for bot reactions)
                        react(&bot, msg.chat.id, msg.id, "\u{1f494}").await;

                        // Clean up progressive editing status message on error too (scope lock before await)
                        let pmid = { tg.progress_messages.lock().unwrap_or_else(|e| e.into_inner()).remove(&msg.chat.id.0) };
                        if let Some(pmid) = pmid
                            && let Err(e) = bot.delete_message(msg.chat.id, pmid).await
                        {
                            tracing::debug!(error = %e, "Progress message already gone");
                        }

                        let err_chain = format!("{e:#}");
                        tracing::error!(error = %err_chain, "[Telegram] Error processing message");
                        let locale = orch.settings.display_language();
                        let user_msg = if err_chain.contains("timed out") {
                            t("error.llm_timeout", &locale).to_string()
                        } else if err_chain.contains("rate limit") || err_chain.contains("429") {
                            t("error.rate_limit", &locale).to_string()
                        } else if err_chain.contains("401") || err_chain.contains("auth") {
                            t("error.auth_failed", &locale).to_string()
                        } else if err_chain.contains("connection") || err_chain.contains("connect") {
                            t("error.connection_failed", &locale).to_string()
                        } else {
                            tf("error.processing_error", &locale,
                                &err_chain.chars().take(150).collect::<String>())
                        };
                        send_html_to_topic(
                            &bot,
                            msg.chat.id,
                            TopicKind::Main,
                            &markdown_to_telegram_html(&user_msg),
                        )
                        .await?;
                    }
                }

                respond(())
            },
        );

        // Callback query handler for inline keyboard buttons (HITL approvals)
        let callback_dedup: Arc<parking_lot::Mutex<CallbackDedup>> =
            Arc::new(parking_lot::Mutex::new(CallbackDedup::new()));
        let inflight_approvals: Arc<parking_lot::Mutex<InFlightApprovals>> =
            Arc::new(parking_lot::Mutex::new(InFlightApprovals::new()));
        let callback_handler = Update::filter_callback_query().endpoint(
            move |bot: Bot, q: teloxide::types::CallbackQuery, orch: Arc<Orchestrator>, tg: Arc<TelegramBot>| {
                let dedup = callback_dedup.clone();
                let inflight = inflight_approvals.clone();
                async move {
                let locale = orch.settings.display_language();

                // ── Replay prevention: reject duplicate callback_query IDs ──
                if dedup.lock().check_and_insert(&q.id.0) {
                    tracing::debug!(callback_id = %q.id.0, "Duplicate callback_query rejected");
                    bot.answer_callback_query(q.id.clone()).text(t("callback.already_processed", &locale)).await?;
                    return respond(());
                }

                let data = match q.data {
                    Some(d) => d,
                    None => {
                        bot.answer_callback_query(q.id.clone()).text(t("error.no_data", &locale)).await?;
                        return respond(());
                    }
                };
                let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(tg.admin_chat_id));

                // Authorization: admin's DM or admin's linked forum group only.
                // CallbackQuery::from is always set, so sender_uid is never None
                // in practice — but treat it symmetrically with the message path.
                let sender_uid = Some(q.from.id.0 as i64);
                if !is_authorized_chat(chat_id.0, sender_uid, tg.admin_chat_id) {
                    bot.answer_callback_query(q.id.clone()).text(t("error.access_denied", &locale)).await?;
                    return respond(());
                }

                // Resolve via the callback_routes single-source-of-truth module.
                // See `callback_routes.rs` for the canonical family list.
                let (action, cmd_type, approval_id) = match crate::callback_routes::match_callback_data(&data) {
                    Some((act, ctype, id)) => (act, ctype, id.into_owned()),
                    None => {
                        bot.answer_callback_query(q.id.clone()).text(t("callback.unknown_action", &locale)).await?;
                        return respond(());
                    }
                };

                // Slow-path ⇒ actions that will await `process_message`,
                // which for `config-*` HITL can hot-reload downstream
                // components and take up to ~60 s.
                // We pre-edit the message to "⏳ 处理中…" as soon as we
                // answer the callback, so the user sees < 500 ms visual
                // feedback instead of a stuck button. `clarify` is excluded
                // because it intentionally keeps its inline buttons so the
                // user can still approve/reject after the explanation.
                // Fast-path handlers (`notification`) return early
                // before we reach the pre-edit block and do their own UI.
                let is_slow_path = !matches!(cmd_type, "notification" | "reminder_action")
                    && !matches!(action, "clarify");
                // Key used for re-tap dedup: raw callback_data uniquely
                // identifies "this button on this message", which is what
                // we want (distinct approval_ids stay independent; two
                // taps on the same approve button collapse).
                let inflight_key = data.clone();
                let already_inflight =
                    is_slow_path && inflight.lock().try_acquire(&inflight_key);

                // Acknowledge the button press immediately. When the same
                // button is re-tapped while its apply is still running,
                // swap to "已在处理中" so the user learns their first tap
                // did register.
                let ack_text = if already_inflight {
                    t("callback.already_processing", &locale)
                } else {
                    match action {
                        "approve" | "once" => t("callback.processing_approval", &locale),
                        "intent" => t("callback.approving_intent", &locale),
                        "domain" => t("callback.approving_domain", &locale),
                        "always" => t("callback.approving_always", &locale),
                        "clarify" => t("callback.requesting_clarification", &locale),
                        "later" => t("callback.deferring", &locale),
                        "dismiss" => t("callback.dismissing", &locale),
                        "never" => t("callback.blocking", &locale),
                        "retry" => t("callback.retrying", &locale),
                        "delete" => t("callback.deleting", &locale),
                        "deactivate" => t("callback.deactivating", &locale),
                        "read" | "summarize" | "remind" => t("callback.preparing_followup", &locale),
                        "markread" => t("callback.processing_approval", &locale),
                        "done" | "snooze" | "snz10" | "snz60" | "snzeve" | "snztom" | "snzmon" | "details" => t("callback.preparing_followup", &locale),
                        "confirmdel" => t("callback.deleting", &locale),
                        "useful" | "useless" | "helpful" | "unhelpful" => t("callback.recording_feedback", &locale),
                        "followup" => t("callback.preparing_followup", &locale),
                        "cancel" => t("callback.cancelled", &locale),
                        _ => t("callback.rejecting", &locale),
                    }
                };
                // Best-effort ack: answer_callback_query just shows a cosmetic
                // toast. If it fails (most commonly "query is too old" after
                // user re-taps / reconnects), the acquired inflight slot above
                // is the real protection against double-apply, and `?` here
                // would propagate out WITHOUT releasing the slot — the button
                // would then stay locked until the 180 s TTL eviction. This
                // is exactly the failure mode c987e5c was meant to prevent.
                // Regression note: 2026-04-18 CTO round-1 finding on c987e5c.
                let _ = bot.answer_callback_query(q.id.clone()).text(ack_text).await;

                if already_inflight {
                    // First tap is still running; don't double-apply.
                    return respond(());
                }

                // Pre-edit: swap the message body to "⏳ 处理中…" so the
                // user sees immediate visual progress even if the apply
                // takes up to a minute. Buttons are stripped by omitting
                // reply_markup so re-taps on the same inline message are
                // structurally prevented. Best-effort: failures here
                // (rare — network blips, "message is not modified") do not
                // block the apply path.
                let preedit_msg = if is_slow_path {
                    if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) =
                        &q.message
                    {
                        let original_text = msg.text().unwrap_or("");
                        let status_label = t("callback.processing", &locale);
                        let edited = format!(
                            "{}\n\n<b>{}</b>",
                            escape_html(original_text),
                            status_label
                        );
                        let _ = bot
                            .edit_message_text(chat_id, msg.id, &edited)
                            .parse_mode(ParseMode::Html)
                            .await;
                        Some(msg.id)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Circuit breaker callbacks: translate to LLM-friendly commands
                // and pre-reset the breaker for retry actions.
                let command = if cmd_type == "circuit_breaker" {
                    let skill_name = &approval_id;
                    match action {
                        "deactivate" => format!("Deactivate skill '{}'", skill_name),
                        "retry" => {
                            // Reset breaker + reactivate so the LLM can use it again
                            orch.memory().reset_circuit_breaker(skill_name).await;
                            format!("Skill '{}' circuit breaker has been reset. Reactivate and test it.", skill_name)
                        }
                        _ => format!("{} {} {}", action, cmd_type, approval_id),
                    }
                } else {
                    format!("{} {} {}", action, cmd_type, approval_id)
                };

                // Route to orchestrator
                let user_did =
                    user_did_for_authorized_chat(chat_id.0, sender_uid, tg.admin_chat_id);

                // Notification feedback: direct DB write, no LLM needed
                if cmd_type == "notification" {
                    let helpful = action == "helpful";
                    tracing::info!(
                        notification_id = %approval_id,
                        helpful,
                        "[HITL] Notification feedback recorded"
                    );
                    if let Err(e) = orch.memory().log_notification_feedback(&approval_id, helpful).await {
                        tracing::warn!(error = %e, "[HITL] Failed to store notification feedback");
                    }
                    let reply_text = if helpful {
                        format!("\u{1f44d} {}", t("callback.recording_feedback", &locale))
                    } else {
                        format!("\u{1f44e} {}", t("callback.recording_feedback", &locale))
                    };
                    // Edit original message to show feedback status
                    if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) = &q.message {
                        let original_text = msg.text().unwrap_or("");
                        let status_label = if helpful {
                            t("callback.helpful", &locale)
                        } else {
                            t("callback.not_helpful", &locale)
                        };
                        let edited = format!(
                            "{}\n\n<b>{}</b>",
                            escape_html(original_text),
                            status_label
                        );
                        let _ = bot
                            .edit_message_text(chat_id, msg.id, &edited)
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                    send_html_to_topic(&bot, chat_id, TopicKind::Notif, &escape_html(&reply_text))
                        .await?;
                    return respond(());
                }

                // Reminder/task notification callbacks: direct deterministic
                // execution, no LLM interpretation. Mutating paths enforce
                // chat/user ownership and record Action Ledger evidence.
                if cmd_type == "reminder_action" {
                    if let Err(e) = handle_reminder_action_callback(
                        &bot,
                        &orch,
                        chat_id,
                        q.message.as_ref(),
                        action,
                        &approval_id,
                        &user_did,
                    )
                    .await
                    {
                        tracing::warn!(
                            error = %e,
                            action,
                            task_id = %approval_id,
                            "reminder callback failed"
                        );
                        let _ = send_html_to_topic(
                            &bot,
                            chat_id,
                            TopicKind::Notif,
                            crate::i18n::t(
                                "reminder.cb.action_failed",
                                &orch.settings.display_language(),
                            ),
                        )
                        .await;
                    }
                    return respond(());
                }

                // Draft send approvals: route directly to the draft handlers
                // (same path as the text command `approve <draft_id>` /
                // `reject <draft_id>`). Bypasses process_message so we don't
                // round-trip through the LLM router for a structured action.
                //
                // The apply future is wrapped in `AssertUnwindSafe` +
                // `catch_unwind` so a panic inside the heavy apply path
                // turns into a visible "❌ 内部错误" on the button message
                // (and logs the panic) instead of leaving the message
                // stuck in "⏳ 处理中…" forever. The in-flight slot is
                // released in every terminating branch below.
                let apply_future = async {
                    orch.process_message(&user_did, &command, Some(chat_id.0)).await
                };
                let result_future = match std::panic::AssertUnwindSafe(apply_future)
                    .catch_unwind()
                    .await
                {
                    Ok(inner) => inner,
                    Err(panic) => {
                        let panic_msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "panic payload was not a string".to_string()
                        };
                        tracing::error!(
                            %data,
                            action,
                            cmd_type,
                            approval_id = %approval_id,
                            panic = %panic_msg,
                            "[HITL] callback apply panicked"
                        );
                        // Flip the pre-edit into a visible error state so
                        // the user knows the tap registered but failed.
                        if let Some(msg_id) = preedit_msg {
                            let edited = format!(
                                "<b>{}</b>",
                                t("callback.internal_error", &locale)
                            );
                            let _ = bot
                                .edit_message_text(chat_id, msg_id, &edited)
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                        inflight.lock().release(&inflight_key);
                        return respond(());
                    }
                };
                // Always release the in-flight slot BEFORE the final edit
                // so re-taps after completion don't get spuriously
                // suppressed. Safe — the button is about to be replaced
                // with a terminal status anyway.
                inflight.lock().release(&inflight_key);
                match result_future {
                    Ok(result) => {
                        let html = markdown_to_telegram_html(&result.reply);
                        if result.inline_buttons.is_empty() {
                            send_html_to_topic(&bot, chat_id, TopicKind::Approval, &html).await?;
                        } else {
                            let buttons: Vec<teloxide::types::InlineKeyboardButton> = result.inline_buttons.iter()
                                .map(|(label, data)| teloxide::types::InlineKeyboardButton::callback(label, data))
                                .collect();
                            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![buttons]);
                            use teloxide::payloads::SendMessageSetters;
                            let thread = wrap_thread_id(
                                resolve_thread_id(chat_id.0, TopicKind::Approval).await,
                            );
                            let mut req = bot
                                .send_message(chat_id, &html)
                                .parse_mode(teloxide::types::ParseMode::Html)
                                .reply_markup(keyboard);
                            if let Some(tid) = thread {
                                req = req.message_thread_id(tid);
                            }
                            let _ = req.await;
                        }

                        // Edit the original message to show status.
                        // For "clarify": keep inline buttons so the user can still
                        // approve/reject after reading the clarification.
                        if action != "clarify"
                            && let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) =
                                &q.message
                        {
                            let status = match action {
                                "approve" => t("callback.approved", &locale),
                                "once" => t("callback.approved_once", &locale),
                                "intent" => t("callback.approved_intent", &locale),
                                "domain" => t("callback.approved_domain", &locale),
                                "always" => t("callback.approved_always", &locale),
                                "later" => t("callback.deferred", &locale),
                                "dismiss" => t("callback.dismissed", &locale),
                                "never" => t("callback.blocked", &locale),
                                "reject" => t("callback.rejected", &locale),
                                "retry" => t("callback.retrying_status", &locale),
                                "delete" => t("callback.deleted", &locale),
                                "useful" => t("callback.helpful", &locale),
                                "useless" => t("callback.not_helpful", &locale),
                                "followup" => t("callback.following_up", &locale),
                                "deactivate" => t("callback.deactivated", &locale),
                                "read" | "summarize" | "markread" | "remind" => {
                                    t("callback.following_up", &locale)
                                }
                                "cancel" => t("callback.cancelled", &locale),
                                _ => t("callback.rejected", &locale),
                            };
                            let original_text = msg.text().unwrap_or("");
                            let edited = format!(
                                "{}\n\n<b>{}</b>",
                                escape_html(original_text),
                                status
                            );
                            let _ = bot
                                .edit_message_text(chat_id, msg.id, &edited)
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                    }
                    Err(e) => {
                        send_html_to_topic(
                            &bot,
                            chat_id,
                            TopicKind::Approval,
                            &format!("Error: {}", escape_html(&e.to_string())),
                        )
                        .await?;
                    }
                }

                respond(())
            }},
        );

        let orch = orchestrator;
        let tg = self;

        let combined_handler = dptree::entry().branch(handler).branch(callback_handler);

        let mut dispatcher = Dispatcher::builder(tg.bot.clone(), combined_handler)
            .dependencies(dptree::deps![orch, tg])
            .default_handler(|_| async {})
            .build();
        let dispatcher_result = std::panic::AssertUnwindSafe(dispatcher.dispatch())
            .catch_unwind()
            .await;
        if let Err(panic) = dispatcher_result {
            let panic_msg = panic_payload_to_string(panic.as_ref());
            tracing::error!(
                panic = %panic_msg,
                "Telegram dispatcher panicked; exiting so systemd restarts the bot"
            );
            eprintln!("[Telegram] Dispatcher panicked; exiting for systemd restart: {panic_msg}");
            std::process::exit(101);
        }
    }

    /// Send an alert message to the admin (with HTML formatting).
    ///
    /// Routed to the System topic. Destination is resolved at send time via
    /// [`crate::telegram_notification_router`]: DM when no forum is linked,
    /// linked forum's System topic otherwise. Used by infra / integrity
    /// watchdogs that need to surface failures to the operator.
    pub async fn send_alert(&self, message: &str) -> Result<()> {
        let html = markdown_to_telegram_html(message);
        let route = crate::telegram_notification_router::route_telegram_notification(
            self.admin_chat_id,
            crate::telegram_notification_router::proactive_route_request(
                crate::telegram_notification_router::NotificationClass::SystemStatus,
                crate::telegram_notification_router::PrivacyLevel::SystemOnly,
                None,
            ),
        );
        send_html_impl(
            &self.bot,
            ChatId(route.chat_id),
            &html,
            wrap_thread_id(route.thread_id),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Telegram send failed: {e}"))?;
        Ok(())
    }
}

/// Spawn a background task that listens for orchestrator progress events and
/// sends Telegram typing indicators / brief status messages. Debounced to
/// at most 1 message per 3 seconds per chat.
///
/// Special cases:
/// - Messages containing "Approval Request" are sent as full messages (not just typing)
/// - SysAdmin approval messages get inline keyboard buttons for approve/reject
fn hitl_with_delivery_chat_bound(
    raw: &str,
    source_chat_id: i64,
    delivery_chat_id: i64,
    delivery_thread_id: Option<i32>,
) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object_mut()?;

    let mut allowed_chat_ids = Vec::new();
    for id in [
        object.get("chat_id").and_then(|v| v.as_i64()),
        object.get("delivery_chat_id").and_then(|v| v.as_i64()),
        Some(source_chat_id),
        Some(delivery_chat_id),
    ]
    .into_iter()
    .flatten()
    {
        if !allowed_chat_ids.contains(&id) {
            allowed_chat_ids.push(id);
        }
    }
    if let Some(existing) = object.get("allowed_chat_ids").and_then(|v| v.as_array()) {
        for id in existing.iter().filter_map(|v| v.as_i64()) {
            if !allowed_chat_ids.contains(&id) {
                allowed_chat_ids.push(id);
            }
        }
    }

    object.insert(
        "delivery_chat_id".to_string(),
        serde_json::json!(delivery_chat_id),
    );
    object.insert(
        "routed_from_chat_id".to_string(),
        serde_json::json!(source_chat_id),
    );
    if let Some(thread_id) = delivery_thread_id {
        object.insert(
            "delivery_thread_id".to_string(),
            serde_json::json!(thread_id),
        );
    }
    object.insert(
        "allowed_chat_ids".to_string(),
        serde_json::json!(allowed_chat_ids),
    );

    Some(value.to_string())
}

async fn bind_hitl_delivery_chat(
    memory: &MemoryHandle,
    approval_id: &str,
    source_chat_id: i64,
    delivery_chat_id: i64,
    delivery_thread_id: Option<i32>,
) {
    let Ok(Some(raw)) = memory.load_skill_state("hitl", approval_id).await else {
        tracing::warn!(
            approval_id,
            delivery_chat_id,
            "[HITL] approval request routed before state was available"
        );
        return;
    };
    if raw.is_empty() {
        return;
    }
    let Some(updated) =
        hitl_with_delivery_chat_bound(&raw, source_chat_id, delivery_chat_id, delivery_thread_id)
    else {
        tracing::warn!(
            approval_id,
            delivery_chat_id,
            "[HITL] approval state was not valid JSON; cannot bind delivery chat"
        );
        return;
    };
    if let Err(e) = memory
        .store_skill_state("hitl", approval_id, &updated)
        .await
    {
        tracing::warn!(
            error = %e,
            approval_id,
            delivery_chat_id,
            "[HITL] failed to bind approval delivery chat"
        );
    }
}

/// Mirror an owner-facing notice into the Web owner timeline.
///
/// This is intentionally not gated on current Web presence: the Web UX is the
/// primary owner surface, so scheduled reports, reminders, and system notices
/// must be available there even when the owner was offline at delivery time.
pub async fn record_owner_web_notice(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
) -> bool {
    if let Some((owner_did, text)) =
        record_owner_web_notice_without_push(memory, admin_chat_id, notice).await
    {
        crate::admin::spawn_web_push_owner_notice(memory.clone(), owner_did, text);
    }
    true
}

pub(crate) async fn record_owner_web_notice_without_push(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
) -> Option<(String, String)> {
    record_owner_web_notice_without_push_anchored(memory, admin_chat_id, notice, None, None).await
}

/// Anchor kind for heartbeat alerts recorded in the owner's conversation;
/// `anchor_ref` is the skill name. The chat-history projection collapses
/// consecutive alerts with the same anchor into one bubble with a repeat
/// count (owner 2026-09-12: 15 copies of the disk alert in 26 hours).
pub(crate) const HEARTBEAT_ALERT_ANCHOR_KIND: &str = "heartbeat_alert";

pub(crate) async fn record_owner_web_notice_without_push_anchored(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
    anchor_kind: Option<&str>,
    anchor_ref: Option<&str>,
) -> Option<(String, String)> {
    let owner_did = crate::owner_presence::canonical_owner_did_for_chat(admin_chat_id);
    let text = owner_web_notice_text(notice);
    if text.trim().is_empty() {
        return None;
    }
    if let Err(e) = memory
        .push_l1(
            &owner_did,
            MemoryMessage::text_with_anchor(
                "assistant",
                text.clone(),
                chrono::Utc::now().timestamp(),
                anchor_kind.map(str::to_string),
                anchor_ref.map(str::to_string),
            ),
        )
        .await
    {
        tracing::warn!(error = %e, "[Delivery] Failed to record owner Web notice in L1");
    }
    Some((owner_did, text))
}

/// Mirror the notice to Web and return whether Telegram's full send should be
/// suppressed because Web Push delivered it to at least one saved device.
pub async fn record_owner_web_notice_if_push_delivered(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
) -> bool {
    record_owner_web_notice_if_push_delivered_anchored(memory, admin_chat_id, notice, None, None)
        .await
}

pub async fn record_owner_web_notice_if_push_delivered_anchored(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
    anchor_kind: Option<&str>,
    anchor_ref: Option<&str>,
) -> bool {
    let Some((owner_did, text)) = record_owner_web_notice_without_push_anchored(
        memory,
        admin_chat_id,
        notice,
        anchor_kind,
        anchor_ref,
    )
    .await
    else {
        return false;
    };
    crate::admin::try_owner_web_push_notice(memory, &owner_did, &text).await
}

async fn web_delivery_notice_if_push_delivered(
    memory: &MemoryHandle,
    admin_chat_id: i64,
    notice: &str,
) -> bool {
    let Some(text) = web_delivery_notice_text(notice) else {
        return false;
    };
    let Some((owner_did, text)) =
        record_owner_web_notice_without_push(memory, admin_chat_id, &text).await
    else {
        return false;
    };
    crate::admin::try_owner_web_push_notice(memory, &owner_did, &text).await
}

fn owner_web_notice_text(notice: &str) -> String {
    let plain = strip_html_tags(notice).trim().to_string();
    if plain.chars().count() > 4_000 {
        plain.chars().take(4_000).collect()
    } else {
        plain
    }
}

/// Appends a native web-action hint to an owner web reminder notice, rendered in
/// the owner's `display_language` (Channel B — system-authored; spec §2/§6). The
/// dedup guard checks both the English and Chinese suffix markers so the hint is
/// not re-appended when the notice already carries it (e.g. on retry).
pub(crate) fn owner_web_reminder_notice_text(notice: &str, task_id: i64, locale: &str) -> String {
    let plain = owner_web_notice_text(notice);
    if plain.trim().is_empty() || plain.contains("Web actions:") || plain.contains("网页操作：")
    {
        return plain;
    }
    let actions = crate::i18n::tf("reminder.web_actions", locale, &task_id.to_string());
    format!("{plain}\n\n{actions}")
}

fn web_delivery_notice_text(status: &str) -> Option<String> {
    match owner_progress_kind(status) {
        OwnerProgressKind::ApprovalRequest => {
            Some("Aegis needs a decision. Open Work to review the approval.".into())
        }
        OwnerProgressKind::RoutineOwnerNotice => Some(status.chars().take(4_000).collect()),
        OwnerProgressKind::ToolUsageProgress | OwnerProgressKind::ReactiveProgress => None,
    }
}

pub fn spawn_progress_listener(
    bot: Bot,
    mut rx: tokio::sync::mpsc::Receiver<crate::progress_router::ProgressEvent>,
    progress_tracker: ProgressTracker,
    settings: Arc<crate::settings::RuntimeSettings>,
    memory: MemoryHandle,
    admin_chat_id: i64,
) -> tokio::task::JoinHandle<()> {
    use std::collections::HashMap;
    use teloxide::types::InlineKeyboardButton;
    use teloxide::types::InlineKeyboardMarkup;
    tokio::spawn(async move {
        let mut last_sent: HashMap<i64, tokio::time::Instant> = HashMap::new();
        let debounce = std::time::Duration::from_secs(3);

        while let Some(event) = rx.recv().await {
            // Decode the typed event back to the EXACT legacy `(chat_id, String)`
            // form so every downstream branch (owner_progress_kind classification,
            // approval-id extraction, heartbeat suppression, debounce, rendering)
            // is byte-identical to the pre-typed channel. turn_id is Telegram-irrelevant.
            let raw_chat_id = event.chat_id;
            let status = crate::progress_router::progress_event_to_legacy_string(&event);
            // Web-only frames (assistant token deltas) decode to an empty
            // string. They must not become a Telegram message.
            if status.is_empty() {
                continue;
            }
            let owner_progress_kind = owner_progress_kind(&status);
            let is_approval_request = owner_progress_kind == OwnerProgressKind::ApprovalRequest;

            // Suppress heartbeat process messages from Telegram.
            // These are logged via tracing::info! in orchestrator.rs but NOT sent
            // to the user — they create too much noise. The user can view heartbeat
            // history via the /status dashboard command instead.
            let is_heartbeat_process =
                status.starts_with("\u{1fac0}") || status.starts_with("\u{1f514}");
            if is_heartbeat_process {
                continue;
            }

            // Owner notices: robot = LLM notify_user, clipboard = daily summary,
            // mail = source event. Memo updates are silent.
            let is_broadcast_msg = owner_progress_kind.is_broadcast_notice();
            let is_tool_usage_status = owner_progress_kind == OwnerProgressKind::ToolUsageProgress;
            let proactive_class = owner_progress_kind.notification_class();
            if raw_chat_id == admin_chat_id
                && owner_progress_kind.suppress_for_active_web()
                && web_delivery_notice_if_push_delivered(&memory, admin_chat_id, &status).await
            {
                tracing::info!(
                    status_kind = owner_progress_kind.as_log_kind(),
                    "[Delivery] Web Push delivered; suppressing Telegram proactive send"
                );
                continue;
            }

            // Proactive owner notices (heartbeat broadcasts, approvals, alerts,
            // scheduled briefs) always resolve to the owner's admin DM through
            // TelegramNotificationRouter, which clamps any stale/legacy
            // non-admin chat id to the admin DM (group/forum routing removed).
            // Plain progress/edit messages are reactive (`proactive_class` is
            // None) and stay in the chat where the user interacted — post
            // auth-gate that is always the admin DM.
            let proactive_route = proactive_class.map(|notification_class| {
                crate::telegram_notification_router::route_telegram_notification(
                    admin_chat_id,
                    crate::telegram_notification_router::proactive_route_request(
                        notification_class,
                        crate::telegram_notification_router::PrivacyLevel::PrivateDmOnly,
                        None,
                    ),
                )
            });
            let chat_id = proactive_route.map_or(raw_chat_id, |route| route.chat_id);
            // Approval request messages are always sent immediately (no debounce)
            if is_approval_request {
                // Approvals are owner HITL prompts — always to the admin DM,
                // never a raw chat id, even if proactive_class was unset.
                let approval_route = proactive_route.or_else(|| {
                    Some(crate::telegram_notification_router::route_telegram_notification(
                        admin_chat_id,
                        crate::telegram_notification_router::proactive_route_request(
                            crate::telegram_notification_router::NotificationClass::ApprovalRequest,
                            crate::telegram_notification_router::PrivacyLevel::PrivateDmOnly,
                            None,
                        ),
                    ))
                });
                let chat_id = approval_route.map_or(chat_id, |route| route.chat_id);
                // Extract approval ID BEFORE markdown conversion (works on raw text)
                let approval_id = extract_approval_id(&status);

                // Convert to HTML for Telegram display
                // Gate approvals use mixed HTML+Markdown -- convert handles both
                let html = markdown_to_telegram_html(&status);

                if let Some(approval_id) = approval_id {
                    let approval_thread_id =
                        if let Some(thread_id) = approval_route.and_then(|route| route.thread_id) {
                            Some(thread_id)
                        } else {
                            resolve_thread_id(chat_id, TopicKind::Approval).await
                        };
                    bind_hitl_delivery_chat(
                        &memory,
                        &approval_id,
                        raw_chat_id,
                        chat_id,
                        approval_thread_id,
                    )
                    .await;

                    // Look up the button callback prefix via the canonical
                    // registry. Historical fallback to "sysop" is preserved
                    // for approval_ids that don't match any known family.
                    let cb_prefix = crate::callback_routes::cb_prefix_for_approval_id(&approval_id)
                        .unwrap_or("sysop");

                    let locale = settings.display_language();
                    // Gate approvals get a context-aware 6-button layout
                    let keyboard = if cb_prefix == "gate" {
                        let has_intent = status.contains("Intent:");
                        let has_domain = status.contains("Domain:");

                        let mut buttons: Vec<Vec<InlineKeyboardButton>> = vec![];

                        // Row 1: Approve options
                        let mut row1 = vec![InlineKeyboardButton::callback(
                            t("button.gate_once", &locale),
                            format!("gate_once:{}", approval_id),
                        )];
                        if has_intent {
                            row1.push(InlineKeyboardButton::callback(
                                t("button.gate_intent", &locale),
                                format!("gate_intent:{}", approval_id),
                            ));
                        }
                        buttons.push(row1);

                        // Row 2: Broader approvals
                        let mut row2 = vec![];
                        if has_domain {
                            let domain = extract_domain_from_msg(&status)
                                .unwrap_or_else(|| "\u{6b64}\u{57df}\u{540d}".to_string());
                            row2.push(InlineKeyboardButton::callback(
                                tf(
                                    "button.gate_domain_trust",
                                    &locale,
                                    &truncate_display(&domain, 15),
                                ),
                                format!("gate_domain:{}", approval_id),
                            ));
                        }
                        row2.push(InlineKeyboardButton::callback(
                            t("button.gate_always", &locale),
                            format!("gate_always:{}", approval_id),
                        ));
                        buttons.push(row2);

                        // Row 3: Reject / Clarify
                        buttons.push(vec![
                            InlineKeyboardButton::callback(
                                t("button.gate_reject", &locale),
                                format!("gate_reject:{}", approval_id),
                            ),
                            InlineKeyboardButton::callback(
                                t("button.gate_clarify", &locale),
                                format!("gate_clarify:{}", approval_id),
                            ),
                        ]);

                        InlineKeyboardMarkup::new(buttons)
                    } else {
                        InlineKeyboardMarkup::new(vec![vec![
                            InlineKeyboardButton::callback(
                                t("button.approve", &locale),
                                format!("{}_approve:{}", cb_prefix, approval_id),
                            ),
                            InlineKeyboardButton::callback(
                                t("button.reject", &locale),
                                format!("{}_reject:{}", cb_prefix, approval_id),
                            ),
                        ]])
                    };
                    // Retry up to 3 times with backoff — approval buttons are critical
                    use teloxide::payloads::SendMessageSetters;
                    let approval_thread = wrap_thread_id(approval_thread_id);
                    let mut sent = false;
                    for attempt in 0..3u32 {
                        let mut req = bot
                            .send_message(ChatId(chat_id), &html)
                            .parse_mode(ParseMode::Html)
                            .reply_markup(keyboard.clone());
                        if let Some(tid) = approval_thread {
                            req = req.message_thread_id(tid);
                        }
                        match req.await {
                            Ok(_) => {
                                sent = true;
                                break;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e, chat_id, attempt,
                                    "[HITL] Failed to send approval buttons (retrying)"
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    500 * 2u64.pow(attempt),
                                ))
                                .await;
                            }
                        }
                    }
                    if !sent {
                        tracing::error!(
                            chat_id,
                            "[HITL] All 3 retry attempts failed \u{2014} user cannot approve/reject"
                        );
                    }
                } else {
                    // Email draft or other approval -- send as plain HTML
                    if let Err(e) =
                        send_html_to_topic(&bot, ChatId(chat_id), TopicKind::Approval, &html).await
                    {
                        tracing::warn!(error = %e, chat_id, "[HITL] Failed to send approval request to user");
                    }
                }
                last_sent.insert(chat_id, tokio::time::Instant::now());
                continue;
            }

            let now = tokio::time::Instant::now();
            if !is_broadcast_msg {
                // Progressive editing: 3s debounce
                if let Some(last) = last_sent.get(&chat_id)
                    && now.duration_since(*last) < debounce
                {
                    continue;
                }
            }
            // Broadcast messages: no debounce — a heartbeat cycle produces
            // at most ~10 messages over ~30s, well within Telegram rate limits
            last_sent.insert(chat_id, now);

            if is_broadcast_msg {
                // Send heartbeat/autonomous progress as a real text message
                let html = markdown_to_telegram_html(&status);
                // Forum thread routing was removed; proactive sends have no
                // thread id. resolve_thread_id is dormant (always None).
                let notification_thread =
                    wrap_thread_id(resolve_thread_id(chat_id, TopicKind::Notif).await);

                // Autonomous notifications (e.g. "🤖 [crypto_tracker] ...") get
                // inline 👍/👎 feedback buttons so the user can rate usefulness.
                if status.starts_with("\u{1f916} [") {
                    // Millisecond timestamp alone collides under heartbeat
                    // batch-flushes (two notifications same ms) and the
                    // `INSERT OR IGNORE` in log_notification_feedback silently
                    // drops the second feedback. Salt with 32 random bits.
                    let notif_id = format!(
                        "{}-{:08x}",
                        chrono::Utc::now().timestamp_millis(),
                        rand::random::<u32>(),
                    );
                    let keyboard = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback(
                            "\u{1f44d}",
                            format!("notif_helpful:{}", notif_id),
                        ),
                        InlineKeyboardButton::callback(
                            "\u{1f44e}",
                            format!("notif_unhelpful:{}", notif_id),
                        ),
                    ]]);
                    let no_preview = teloxide::types::LinkPreviewOptions {
                        is_disabled: true,
                        url: None,
                        prefer_small_media: false,
                        prefer_large_media: false,
                        show_above_text: false,
                    };
                    use teloxide::payloads::SendMessageSetters;
                    let mut req = bot
                        .send_message(ChatId(chat_id), &html)
                        .parse_mode(ParseMode::Html)
                        .link_preview_options(no_preview)
                        .reply_markup(keyboard);
                    if let Some(tid) = notification_thread {
                        req = req.message_thread_id(tid);
                    }
                    let _ = req.await;
                } else {
                    let _ = send_html_no_preview_impl(
                        &bot,
                        ChatId(chat_id),
                        &html,
                        notification_thread,
                    )
                    .await;
                }
            } else {
                if is_tool_usage_status {
                    let thread = wrap_thread_id(if let Some(route) = proactive_route {
                        route.thread_id
                    } else {
                        resolve_thread_id(chat_id, TopicKind::Main).await
                    });
                    use teloxide::payloads::SendChatActionSetters;
                    let mut req =
                        bot.send_chat_action(ChatId(chat_id), chat_action_for_status(&status));
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    let _ = req.await;
                    continue;
                }

                // Progressive message editing: send or edit a visible status
                // message instead of a bare typing indicator. The message handler
                // will delete this when the final reply is ready.
                let existing_msg = {
                    progress_tracker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&chat_id)
                        .copied()
                };
                if let Some(msg_id) = existing_msg {
                    // Edit the existing progress message
                    let _ = bot
                        .edit_message_text(ChatId(chat_id), msg_id, &status)
                        .await;
                } else {
                    // Send a new progress message and track it
                    use teloxide::payloads::SendMessageSetters;
                    let thread = wrap_thread_id(resolve_thread_id(chat_id, TopicKind::Main).await);
                    let mut req = bot.send_message(ChatId(chat_id), &status);
                    if let Some(tid) = thread {
                        req = req.message_thread_id(tid);
                    }
                    match req.await {
                        Ok(msg) => {
                            progress_tracker
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(chat_id, msg.id);
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Progress message send failed, falling back to typing");
                            let action = chat_action_for_status(&status);
                            // Topic-route the typing indicator so it lands in
                            // the same topic as the pending reply. P0 carryover
                            // from .claude/active-plan-topic-router-p0.md.
                            use teloxide::payloads::SendChatActionSetters;
                            let mut req = bot.send_chat_action(ChatId(chat_id), action);
                            if let Some(tid) = thread {
                                req = req.message_thread_id(tid);
                            }
                            let _ = req.await;
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
fn is_tool_usage_progress_status(status: &str) -> bool {
    owner_progress_kind(status) == OwnerProgressKind::ToolUsageProgress
}

#[cfg(test)]
fn should_suppress_progress_status_for_web(status: &str) -> bool {
    owner_progress_kind(status).suppress_for_active_web()
}

#[cfg(test)]
fn proactive_progress_notification_class(status: &str) -> Option<NotificationClass> {
    owner_progress_kind(status).notification_class()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerProgressKind {
    ApprovalRequest,
    RoutineOwnerNotice,
    ToolUsageProgress,
    ReactiveProgress,
}

impl OwnerProgressKind {
    fn notification_class(self) -> Option<NotificationClass> {
        match self {
            Self::ApprovalRequest => Some(NotificationClass::ApprovalRequest),
            Self::RoutineOwnerNotice => Some(NotificationClass::RoutineNotification),
            Self::ToolUsageProgress | Self::ReactiveProgress => None,
        }
    }

    fn suppress_for_active_web(self) -> bool {
        matches!(
            self,
            Self::ApprovalRequest | Self::RoutineOwnerNotice | Self::ToolUsageProgress
        )
    }

    fn is_broadcast_notice(self) -> bool {
        matches!(self, Self::RoutineOwnerNotice)
    }

    fn as_log_kind(self) -> &'static str {
        match self {
            Self::ApprovalRequest => "approval",
            Self::RoutineOwnerNotice => "notification",
            Self::ToolUsageProgress => "tool_progress",
            Self::ReactiveProgress => "reactive_progress",
        }
    }
}

fn owner_progress_kind(status: &str) -> OwnerProgressKind {
    if status.contains("Approval Request") {
        OwnerProgressKind::ApprovalRequest
    } else if status.starts_with("\u{1f916}")
        || status.starts_with("\u{1f4cb}")
        || status.starts_with("\u{1f4ec}")
    {
        OwnerProgressKind::RoutineOwnerNotice
    } else if status.starts_with("\u{2699}\u{fe0f} Using ") {
        OwnerProgressKind::ToolUsageProgress
    } else {
        OwnerProgressKind::ReactiveProgress
    }
}

/// Extract approval ID from an approval message.
/// Supports both formats:
///   - Markdown backtick: `Approval ID: \`gate-xxx\``
///   - HTML code tag: `Approval ID: <code>gate-xxx</code>`
///   - Plain text: `Approval ID: gate-xxx` (followed by newline/space/end)
fn extract_approval_id(msg: &str) -> Option<String> {
    // Try backtick format first
    let id = if let Some(start) = msg.find("Approval ID: `") {
        let s = start + "Approval ID: `".len();
        let rest = &msg[s..];
        let end = rest.find('`')?;
        &rest[..end]
    } else if let Some(start) = msg.find("Approval ID: <code>") {
        let s = start + "Approval ID: <code>".len();
        let rest = &msg[s..];
        let end = rest.find("</code>")?;
        &rest[..end]
    } else {
        let start = msg.find("Approval ID: ")?;
        let s = start + "Approval ID: ".len();
        let rest = &msg[s..];
        // Take until whitespace or end
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        &rest[..end]
    };
    // All known HITL approval prefixes — sourced from the canonical
    // callback_routes registry so a new family automatically lands here too.
    if crate::callback_routes::approval_id_prefixes().any(|p| id.starts_with(p)) {
        Some(id.to_string())
    } else {
        None
    }
}

/// Extract a domain from an approval message containing "Domain: <code>...</code>".
fn extract_domain_from_msg(msg: &str) -> Option<String> {
    let marker = "Domain: <code>";
    let start = msg.find(marker)? + marker.len();
    let rest = &msg[start..];
    let end = rest.find("</code>")?;
    let domain = &rest[..end];
    // Validate domain format: alphanumeric, dots, hyphens, max 253 chars
    if domain.len() > 253
        || domain.is_empty()
        || !domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    Some(domain.to_string())
}

/// Truncate a string to `max_chars` chars (not bytes), appending "\u{2026}" if truncated.
/// Used for display labels (e.g. button text) where character count matters.
fn truncate_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 1).collect();
        format!("{}\u{2026}", truncated)
    }
}

/// Pick a ChatAction based on the progress status hint.
/// Uses keyword matching on the status string to provide contextual indicators.
fn chat_action_for_status(status: &str) -> ChatAction {
    let lower = status.to_lowercase();
    if lower.contains("screenshot") || lower.contains("photo") || lower.contains("image") {
        ChatAction::UploadPhoto
    } else if lower.contains("document") || lower.contains("file") || lower.contains("export") {
        ChatAction::UploadDocument
    } else if lower.contains("navigat") {
        ChatAction::FindLocation
    } else {
        ChatAction::Typing
    }
}

async fn handle_reminder_action_callback(
    bot: &Bot,
    orch: &Arc<Orchestrator>,
    chat_id: ChatId,
    message: Option<&teloxide::types::MaybeInaccessibleMessage>,
    action: &str,
    task_id_raw: &str,
    user_did: &str,
) -> Result<()> {
    // Channel B (spec §2/§6): reminder-callback chrome is system-authored, so it
    // is rendered in the owner's `display_language`, not the user's button-press.
    use crate::i18n::t;
    let locale = orch.settings.display_language();
    let Ok(task_id) = task_id_raw.parse::<i64>() else {
        send_html_to_topic(
            bot,
            chat_id,
            TopicKind::Notif,
            t("reminder.cb.invalid_id", &locale),
        )
        .await?;
        return Ok(());
    };

    match action {
        "snooze" => {
            if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) = message {
                let _ = bot
                    .edit_message_reply_markup(chat_id, msg.id)
                    .reply_markup(reminder_snooze_keyboard(task_id, &locale))
                    .await;
            } else {
                send_html_to_topic(
                    bot,
                    chat_id,
                    TopicKind::Notif,
                    t("reminder.cb.pick_snooze", &locale),
                )
                .await?;
            }
            return Ok(());
        }
        "delete" => {
            if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) = message {
                let _ = bot
                    .edit_message_reply_markup(chat_id, msg.id)
                    .reply_markup(reminder_delete_confirm_keyboard(task_id, &locale))
                    .await;
            } else {
                send_html_to_topic(
                    bot,
                    chat_id,
                    TopicKind::Notif,
                    t("reminder.cb.confirm_delete_prompt", &locale),
                )
                .await?;
            }
            return Ok(());
        }
        "cancel" => {
            if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) = message {
                let _ = bot
                    .edit_message_reply_markup(chat_id, msg.id)
                    .reply_markup(reminder_action_keyboard(task_id, &locale))
                    .await;
            }
            return Ok(());
        }
        "details" => {
            let Some(task) = orch.memory().get_task(task_id).await else {
                send_html_to_topic(
                    bot,
                    chat_id,
                    TopicKind::Notif,
                    t("reminder.cb.not_found_or_cleared", &locale),
                )
                .await?;
                return Ok(());
            };
            if task.chat_id != chat_id.0 || task.user_did != user_did {
                send_html_to_topic(
                    bot,
                    chat_id,
                    TopicKind::Notif,
                    t("reminder.cb.not_found", &locale),
                )
                .await?;
                return Ok(());
            }
            // Render the trigger time in the owner's timezone (not raw epoch
            // seconds). The stored unix value is unchanged — display only, so
            // use the display resolver (invalid stored tz → host-zone default,
            // not bare UTC), keeping the label consistent with the WebApp projection.
            let tz = crate::settings::resolve_display_user_tz(&orch.settings);
            let when = task
                .trigger_at_unix
                .and_then(|ts| chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0))
                .map(|dt| {
                    dt.with_timezone(&tz)
                        .format("%Y-%m-%d %H:%M %Z")
                        .to_string()
                })
                .unwrap_or_else(|| t("reminder.details.unset", &locale).to_string());
            let body = if task.body.trim().is_empty() {
                ""
            } else {
                task.body.as_str()
            };
            let html = format!(
                "<b>{title}</b>\n\
                 ID: #{id}\n\
                 {status_label}: {status}\n\
                 {time_label}: {when}\n\
                 {title_label}: {task_title}\n{body}",
                title = t("reminder.details.title", &locale),
                id = task.id,
                status_label = t("reminder.details.status", &locale),
                status = escape_html(&task.status),
                time_label = t("reminder.details.time", &locale),
                when = escape_html(&when),
                title_label = t("reminder.details.title_label", &locale),
                task_title = escape_html(&task.title),
                body = escape_html(body)
            );
            send_html_to_topic(bot, chat_id, TopicKind::Notif, &html).await?;
            return Ok(());
        }
        _ => {}
    }

    let Some(task) = orch.memory().get_task(task_id).await else {
        send_html_to_topic(
            bot,
            chat_id,
            TopicKind::Notif,
            t("reminder.cb.not_found_or_cleared", &locale),
        )
        .await?;
        return Ok(());
    };
    if task.chat_id != chat_id.0 || task.user_did != user_did {
        send_html_to_topic(
            bot,
            chat_id,
            TopicKind::Notif,
            t("reminder.cb.not_found", &locale),
        )
        .await?;
        return Ok(());
    }

    match action {
        "done" => {
            let ledger_action = start_reminder_action_ledger(
                orch,
                crate::action_ledger::ActionKind::ReminderUpdate,
                "done",
                &task,
                user_did,
                None,
            );
            if task.status == "done" {
                finish_reminder_action_ledger(
                    orch,
                    ledger_action.as_deref(),
                    "done",
                    task.id,
                    None,
                );
                edit_reminder_callback_status(
                    bot,
                    chat_id,
                    message,
                    t("reminder.cb.done", &locale),
                )
                .await?;
                return Ok(());
            }
            let updates = serde_json::json!({
                "status": "done",
                "completed_at": chrono::Utc::now().to_rfc3339(),
            });
            match orch
                .memory()
                .update_task(task.id, chat_id.0, &updates.to_string())
                .await
            {
                Ok(true) => {
                    finish_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "done",
                        task.id,
                        None,
                    );
                    edit_reminder_callback_status(
                        bot,
                        chat_id,
                        message,
                        t("reminder.cb.done", &locale),
                    )
                    .await?;
                }
                Ok(false) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "not_found",
                        "task not found or not changed",
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.not_found_short", &locale),
                    )
                    .await?;
                }
                Err(e) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "database_error",
                        &e.to_string(),
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.complete_failed", &locale),
                    )
                    .await?;
                }
            }
        }
        "confirmdel" => {
            let ledger_action = start_reminder_action_ledger(
                orch,
                crate::action_ledger::ActionKind::ReminderDelete,
                "archive",
                &task,
                user_did,
                None,
            );
            let updates = serde_json::json!({"status": "archived"});
            match orch
                .memory()
                .update_task(task.id, chat_id.0, &updates.to_string())
                .await
            {
                Ok(true) => {
                    finish_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "archive",
                        task.id,
                        None,
                    );
                    edit_reminder_callback_status(
                        bot,
                        chat_id,
                        message,
                        t("reminder.cb.snoozed_archived", &locale),
                    )
                    .await?;
                }
                Ok(false) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "not_found",
                        "task not found or not changed",
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.not_found_short", &locale),
                    )
                    .await?;
                }
                Err(e) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "database_error",
                        &e.to_string(),
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.delete_failed", &locale),
                    )
                    .await?;
                }
            }
        }
        "snz10" | "snz60" | "snzeve" | "snztom" | "snzmon" => {
            let Some(option) = reminder_snooze_option_from_action(action) else {
                send_html_to_topic(
                    bot,
                    chat_id,
                    TopicKind::Notif,
                    t("reminder.cb.invalid_snooze", &locale),
                )
                .await?;
                return Ok(());
            };
            let tz = crate::settings::parse_user_tz(&orch.settings);
            let target = reminder_snooze_target_unix(chrono::Utc::now(), tz, option);
            let expires_at = target + crate::intent::MAX_INTENT_LIFETIME_SECS;
            let ledger_action = start_reminder_action_ledger(
                orch,
                crate::action_ledger::ActionKind::ReminderUpdate,
                "snooze",
                &task,
                user_did,
                Some(target),
            );
            let result = if task.status == "active" {
                let updates = serde_json::json!({
                    "trigger_at_unix": target,
                    "expires_at_unix": expires_at,
                    "status": "active",
                    "clear_fields": ["fired_at"],
                });
                orch.memory()
                    .update_task(task.id, chat_id.0, &updates.to_string())
                    .await
            } else {
                orch.memory()
                    .reschedule_intent(task.id, user_did, target, expires_at)
                    .await
            };
            match result {
                Ok(true) => {
                    finish_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "snooze",
                        task.id,
                        Some(target),
                    );
                    let status =
                        crate::i18n::tf("reminder.cb.snoozed_to", &locale, option.label(&locale));
                    edit_reminder_callback_status(bot, chat_id, message, &status).await?;
                }
                Ok(false) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "not_found",
                        "task not found or status cannot be rescheduled",
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.cannot_snooze", &locale),
                    )
                    .await?;
                }
                Err(e) => {
                    fail_reminder_action_ledger(
                        orch,
                        ledger_action.as_deref(),
                        "database_error",
                        &e.to_string(),
                    );
                    send_html_to_topic(
                        bot,
                        chat_id,
                        TopicKind::Notif,
                        t("reminder.cb.snooze_failed", &locale),
                    )
                    .await?;
                }
            }
        }
        _ => {
            send_html_to_topic(
                bot,
                chat_id,
                TopicKind::Notif,
                t("reminder.cb.unknown_action", &locale),
            )
            .await?;
        }
    }
    Ok(())
}

fn reminder_snooze_option_from_action(action: &str) -> Option<ReminderSnoozeOption> {
    match action {
        "snz10" => Some(ReminderSnoozeOption::Minutes10),
        "snz60" => Some(ReminderSnoozeOption::Hour1),
        "snzeve" => Some(ReminderSnoozeOption::Tonight),
        "snztom" => Some(ReminderSnoozeOption::Tomorrow),
        "snzmon" => Some(ReminderSnoozeOption::NextMonday),
        _ => None,
    }
}

async fn edit_reminder_callback_status(
    bot: &Bot,
    chat_id: ChatId,
    message: Option<&teloxide::types::MaybeInaccessibleMessage>,
    status: &str,
) -> Result<()> {
    if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(msg)) = message {
        let original_text = msg.text().unwrap_or("");
        let edited = format!(
            "{}\n\n<b>{}</b>",
            escape_html(original_text),
            escape_html(status)
        );
        let _ = bot
            .edit_message_text(chat_id, msg.id, edited)
            .parse_mode(ParseMode::Html)
            .await;
    } else {
        send_html_to_topic(bot, chat_id, TopicKind::Notif, &escape_html(status)).await?;
    }
    Ok(())
}

fn start_reminder_action_ledger(
    orch: &Arc<Orchestrator>,
    kind: crate::action_ledger::ActionKind,
    action: &str,
    task: &crate::memory::Task,
    user_did: &str,
    snooze_until_unix: Option<i64>,
) -> Option<String> {
    let idempotency_key = format!("reminder-action:{action}:{}", task.id);
    let action_id = format!(
        "{}:{}",
        idempotency_key,
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| chrono::Utc::now().timestamp_micros() * 1_000)
    );
    let result = (|| -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(orch.memory().db_path())?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store.create_proposed(&crate::action_ledger::NewAction {
            action_id: action_id.clone(),
            action_kind: kind,
            requested_by: Some(user_did.to_string()),
            chat_id: Some(task.chat_id),
            input_summary: Some(format!("{} reminder #{}: {}", action, task.id, task.title)),
            user_visible_summary: Some(format!("Reminder {}: {}", action, task.title)),
            idempotency_key: Some(idempotency_key),
            metadata_json: Some(
                serde_json::json!({
                    "task_id": task.id,
                    "callback_action": action,
                    "previous_status": task.status,
                    "snooze_until_unix": snooze_until_unix,
                    "schema_version": 1,
                })
                .to_string(),
            ),
            ..Default::default()
        })?;
        store.transition(
            &action_id,
            crate::action_ledger::ActionStatus::Executing,
            &crate::action_ledger::ActionUpdate {
                tool_call_id: Some("telegram_reminder_callback".to_string()),
                ..Default::default()
            },
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => Some(action_id),
        Err(e) => {
            tracing::warn!(error = %e, task_id = task.id, action, "failed to start reminder callback action ledger row");
            None
        }
    }
}

fn finish_reminder_action_ledger(
    orch: &Arc<Orchestrator>,
    action_id: Option<&str>,
    action: &str,
    task_id: i64,
    snooze_until_unix: Option<i64>,
) {
    let Some(action_id) = action_id else {
        return;
    };
    let result = (|| -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(orch.memory().db_path())?;
        crate::action_ledger::ActionLedgerStore::new(&conn).transition(
            action_id,
            crate::action_ledger::ActionStatus::Executed,
            &crate::action_ledger::ActionUpdate {
                external_receipt: Some(format!("reminder_callback:{action}:{task_id}")),
                tool_call_id: Some("telegram_reminder_callback".to_string()),
                metadata_json: Some(
                    serde_json::json!({
                        "task_id": task_id,
                        "callback_action": action,
                        "snooze_until_unix": snooze_until_unix,
                        "schema_version": 1,
                    })
                    .to_string(),
                ),
                ..Default::default()
            },
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!(error = %e, task_id, action, "failed to finish reminder callback action ledger row");
    }
}

fn fail_reminder_action_ledger(
    orch: &Arc<Orchestrator>,
    action_id: Option<&str>,
    error_kind: &str,
    error_message: &str,
) {
    let Some(action_id) = action_id else {
        return;
    };
    let result = (|| -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(orch.memory().db_path())?;
        crate::action_ledger::ActionLedgerStore::new(&conn).transition(
            action_id,
            crate::action_ledger::ActionStatus::Failed,
            &crate::action_ledger::ActionUpdate {
                tool_call_id: Some("telegram_reminder_callback".to_string()),
                error_kind: Some(error_kind.to_string()),
                error_message: Some(crate::utils::truncate_str(error_message, 500).to_string()),
                ..Default::default()
            },
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!(error = %e, action_id, "failed to fail reminder callback action ledger row");
    }
}

/// HeartbeatHandler that sends alerts via Telegram + formats with LLM context.
pub struct TelegramHeartbeatHandler {
    bot: Bot,
    admin_chat_id: i64,
    memory: Option<MemoryHandle>,
    action_ledger_db_path: Option<String>,
    settings: Option<Arc<crate::settings::RuntimeSettings>>,
}

impl TelegramHeartbeatHandler {
    pub fn new(bot: Bot, admin_chat_id: i64) -> Self {
        Self {
            bot,
            admin_chat_id,
            memory: None,
            action_ledger_db_path: None,
            settings: None,
        }
    }

    pub fn with_memory(mut self, memory: MemoryHandle) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Provide runtime settings so proactive Channel-B labels render in the
    /// owner's `display_language`.
    pub fn with_settings(mut self, settings: Arc<crate::settings::RuntimeSettings>) -> Self {
        self.settings = Some(settings);
        self
    }

    pub fn with_action_ledger_db_path(mut self, db_path: impl Into<String>) -> Self {
        self.action_ledger_db_path = Some(db_path.into());
        self
    }
}

fn format_heartbeat_alert_message(skill_name: &str, payload: &str, locale: &str) -> String {
    let localized = crate::alert_copy::localize_heartbeat_alert(skill_name, payload, locale);
    let emoji = match localized.state {
        crate::alert_copy::AlertState::Resolved => "\u{2705}", // checkmark for resolved
        crate::alert_copy::AlertState::Error => "\u{26a0}\u{fe0f}", // warning for errors
        crate::alert_copy::AlertState::New | crate::alert_copy::AlertState::Ongoing => {
            "\u{1f6a8}" // siren for active alerts
        }
    };
    format!("{emoji} {}\n\n{}", localized.title, localized.body)
}

fn should_pin_heartbeat_alert(payload: &str) -> bool {
    !payload.contains("[RESOLVED]") && !payload.starts_with("Error:")
}

impl HeartbeatHandler for TelegramHeartbeatHandler {
    fn on_alert(
        &self,
        skill_name: &str,
        probe: &ProbeSignal,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        let payload = probe.event_payload.as_deref().unwrap_or("No details");
        let is_resolved = payload.contains("[RESOLVED]");
        let is_error_probe = payload.starts_with("Error:");
        let should_pin = should_pin_heartbeat_alert(payload);
        let locale = self
            .settings
            .as_ref()
            .map(|settings| settings.display_language())
            .unwrap_or_else(|| crate::i18n::DEFAULT_LOCALE.to_string());
        let base_alert_msg = format_heartbeat_alert_message(skill_name, payload, &locale);
        let enrich_disk = crate::alert_copy::payload_has_disk_pressure(skill_name, payload);
        let route = crate::telegram_notification_router::route_telegram_notification(
            self.admin_chat_id,
            crate::telegram_notification_router::proactive_route_request(
                crate::telegram_notification_router::NotificationClass::ProactiveAlert,
                crate::telegram_notification_router::PrivacyLevel::PrivateDmOnly,
                None,
            ),
        );
        let chat_id = ChatId(route.chat_id);
        let thread = wrap_thread_id(route.thread_id);
        let bot = self.bot.clone();
        let skill_owned = skill_name.to_string();
        let ledger_db_path = self.action_ledger_db_path.clone();
        let memory = self.memory.clone();
        let admin_chat_id = self.admin_chat_id;
        let action_id = new_telegram_action_id(chat_id);

        Box::pin(async move {
            // Disk alerts carry the largest directories (owner 2026-09-12), so
            // "check large logs and build artifacts" names the actual paths.
            // Bounded: one executor scan, 180 s cap, cached 90 min; on failure
            // the alert goes out unchanged.
            let mut alert_msg = base_alert_msg;
            if enrich_disk
                && let Some(rows) = crate::executor_client::largest_directories(
                    5,
                    std::time::Duration::from_secs(90),
                )
                .await
            {
                let block = crate::executor_client::largest_directories_block(&rows, &locale);
                if !block.is_empty() {
                    alert_msg.push_str("\n\n");
                    alert_msg.push_str(&block);
                }
            }
            if let Some(memory) = memory.as_ref()
                && record_owner_web_notice_if_push_delivered_anchored(
                    memory,
                    admin_chat_id,
                    &alert_msg,
                    Some(HEARTBEAT_ALERT_ANCHOR_KIND),
                    Some(&skill_owned),
                )
                .await
            {
                if is_resolved {
                    unpin_all_messages(&bot, chat_id).await;
                }
                tracing::info!(
                    skill = %skill_owned,
                    msg_len = alert_msg.len(),
                    "[Delivery] Web Push delivered heartbeat alert; Telegram fallback suppressed"
                );
                return Ok(alert_msg);
            }
            let html = markdown_to_telegram_html(&alert_msg);
            best_effort_start_telegram_send_ledger(
                ledger_db_path.as_deref(),
                &action_id,
                chat_id,
                thread,
                &skill_owned,
                &alert_msg,
            );
            use teloxide::payloads::SendMessageSetters;
            let mut req = bot.send_message(chat_id, &html).parse_mode(ParseMode::Html);
            if let Some(tid) = thread {
                req = req.message_thread_id(tid);
            }
            let msg_result = req.await;
            match msg_result {
                Ok(sent_msg) => {
                    best_effort_finish_telegram_send_ledger(
                        ledger_db_path.as_deref(),
                        &action_id,
                        chat_id,
                        sent_msg.id,
                    );
                    if is_resolved {
                        // Alert resolved — unpin all prior alert pins so they don't accumulate.
                        unpin_all_messages(&bot, chat_id).await;
                        tracing::info!(
                            skill = %skill_owned,
                            msg_len = html.len(),
                            "heartbeat RESOLVED: sent notification and unpinned prior alerts"
                        );
                    } else {
                        let was_pinned = if !should_pin {
                            false
                        } else {
                            pin_message(&bot, chat_id, sent_msg.id).await;
                            true
                        };
                        tracing::info!(
                            skill = %skill_owned,
                            was_pinned,
                            msg_len = html.len(),
                            is_error_probe,
                            "heartbeat alert sent"
                        );
                    }
                }
                Err(e) => {
                    best_effort_fail_telegram_send_ledger(
                        ledger_db_path.as_deref(),
                        &action_id,
                        &e.to_string(),
                    );
                    return Err(anyhow::anyhow!("Telegram alert failed: {e}"));
                }
            }
            Ok(alert_msg)
        })
    }
}

fn new_telegram_action_id(chat_id: ChatId) -> String {
    format!(
        "telegram-send:{}:{}:{:08x}",
        chat_id.0,
        chrono::Utc::now().timestamp_millis(),
        rand::random::<u32>()
    )
}

fn best_effort_start_telegram_send_ledger(
    db_path: Option<&str>,
    action_id: &str,
    chat_id: ChatId,
    thread: Option<teloxide::types::ThreadId>,
    skill_name: &str,
    message: &str,
) {
    if let Some(db_path) = db_path
        && let Err(e) =
            start_telegram_send_ledger(db_path, action_id, chat_id, thread, skill_name, message)
    {
        tracing::warn!(error = %e, action_id, "failed to start Telegram send action ledger row");
    }
}

fn best_effort_finish_telegram_send_ledger(
    db_path: Option<&str>,
    action_id: &str,
    chat_id: ChatId,
    message_id: MessageId,
) {
    if let Some(db_path) = db_path
        && let Err(e) = finish_telegram_send_ledger(db_path, action_id, chat_id, message_id)
    {
        tracing::warn!(error = %e, action_id, "failed to finish Telegram send action ledger row");
    }
}

fn best_effort_fail_telegram_send_ledger(db_path: Option<&str>, action_id: &str, error: &str) {
    if let Some(db_path) = db_path
        && let Err(e) = fail_telegram_send_ledger(db_path, action_id, error)
    {
        tracing::warn!(error = %e, action_id, "failed to mark Telegram send action ledger row failed");
    }
}

fn start_telegram_send_ledger(
    db_path: &str,
    action_id: &str,
    chat_id: ChatId,
    thread: Option<teloxide::types::ThreadId>,
    skill_name: &str,
    message: &str,
) -> anyhow::Result<()> {
    let conn = crate::encrypted_sqlite::open(db_path)?;
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    let topic_thread_id = thread.map(|tid| tid.0.0);
    store.create_proposed(&crate::action_ledger::NewAction {
        action_id: action_id.to_string(),
        action_kind: crate::action_ledger::ActionKind::TelegramSend,
        requested_by: Some("system:heartbeat_alert".to_string()),
        chat_id: Some(chat_id.0),
        input_summary: Some(truncate_action_summary(message)),
        user_visible_summary: Some(format!("Telegram alert for {skill_name}")),
        idempotency_key: Some(action_id.to_string()),
        source_event_id: Some(skill_name.to_string()),
        metadata_json: Some(
            serde_json::json!({
                "channel": "telegram",
                "telegram_thread_id": topic_thread_id,
                "lane": "heartbeat_alert"
            })
            .to_string(),
        ),
        ..Default::default()
    })?;
    store.transition(
        action_id,
        crate::action_ledger::ActionStatus::Executing,
        &crate::action_ledger::ActionUpdate::default(),
    )?;
    Ok(())
}

fn finish_telegram_send_ledger(
    db_path: &str,
    action_id: &str,
    chat_id: ChatId,
    message_id: MessageId,
) -> anyhow::Result<()> {
    let conn = crate::encrypted_sqlite::open(db_path)?;
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    store.transition(
        action_id,
        crate::action_ledger::ActionStatus::Executed,
        &crate::action_ledger::ActionUpdate {
            external_message_id: Some(format!("telegram:{}:{}", chat_id.0, message_id.0)),
            external_receipt: Some(format!("telegram_message_id:{}", message_id.0)),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn fail_telegram_send_ledger(db_path: &str, action_id: &str, error: &str) -> anyhow::Result<()> {
    let conn = crate::encrypted_sqlite::open(db_path)?;
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    store.transition(
        action_id,
        crate::action_ledger::ActionStatus::Failed,
        &crate::action_ledger::ActionUpdate {
            error_kind: Some("telegram_send_failed".to_string()),
            error_message: Some(truncate_action_summary(error)),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn truncate_action_summary(value: &str) -> String {
    const MAX: usize = 240;
    if value.len() <= MAX {
        return value.to_string();
    }
    let mut end = MAX;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

/// `HeartbeatHandler` wrapper that delegates alert/silent to the inner handler
/// and routes the True Heartbeat tick into the orchestrator.
pub struct AutonomousHeartbeatHandler {
    inner: TelegramHeartbeatHandler,
    /// Orchestrator reference for autonomous heartbeat processing.
    /// Set after construction via `set_orchestrator` (breaks creation-order dependency).
    orchestrator: tokio::sync::OnceCell<Arc<crate::orchestrator::Orchestrator>>,
}

impl AutonomousHeartbeatHandler {
    pub fn new(inner: TelegramHeartbeatHandler) -> Self {
        Self {
            inner,
            orchestrator: tokio::sync::OnceCell::new(),
        }
    }

    /// Set the orchestrator reference (called after orchestrator is created).
    pub fn set_orchestrator(&self, orchestrator: Arc<crate::orchestrator::Orchestrator>) {
        let _ = self.orchestrator.set(orchestrator);
    }
}

impl HeartbeatHandler for AutonomousHeartbeatHandler {
    fn on_alert(
        &self,
        skill_name: &str,
        probe: &ProbeSignal,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        self.inner.on_alert(skill_name, probe)
    }

    fn on_silent(&self, skill_name: &str, probe: &ProbeSignal) {
        self.inner.on_silent(skill_name, probe);
    }

    fn on_true_heartbeat(
        &self,
        snapshot: &crate::chronos::SituationSnapshot,
        config: &crate::chronos::AutonomousConfig,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::chronos::AutonomousOutcome>> + Send + '_,
        >,
    > {
        let snapshot_clone = snapshot.clone();
        let config_clone = config.clone();

        Box::pin(async move {
            let orchestrator = match self.orchestrator.get() {
                Some(orch) => orch,
                None => {
                    tracing::warn!(
                        "[TrueHeartbeat] Orchestrator not set \u{2014} skipping heartbeat cycle"
                    );
                    return Ok(crate::chronos::AutonomousOutcome {
                        fell_through_to_alert: true,
                        outcome_tag: crate::chronos::OutcomeTag::Fallthrough,
                        ..Default::default()
                    });
                }
            };

            // DID unification (spec §5.2): use canonical `did:aegis:telegram:{id}`
            // format, not the legacy `did:aegis:{id}` which was missing the
            // `telegram:` transport segment.
            let user_did = format!("did:aegis:telegram:{}", self.inner.admin_chat_id);
            let chat_id = Some(self.inner.admin_chat_id);

            orchestrator
                .run_heartbeat_tick(
                    crate::orchestrator::TickTrigger::Scheduled,
                    &snapshot_clone,
                    &config_clone,
                    &user_did,
                    chat_id,
                )
                .await
        })
    }

    fn on_health_probe(
        &self,
        chat_id: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Some(orchestrator) = self.orchestrator.get() {
                orchestrator.run_health_probes(Some(chat_id)).await;
            }
        })
    }
}

// ── Google OAuth credential/code detection ──────────────────────────────

/// Detect Google OAuth client credentials pasted in a Telegram message.
///
/// Matches patterns like:
/// - `client_id: xxx.apps.googleusercontent.com` + `client_secret: GOCSPx-xxx`
/// - `GOOGLE_CLIENT_ID=xxx` + `GOOGLE_CLIENT_SECRET=xxx`
///
/// Returns `Some((client_id, client_secret))` if both are found.
pub fn detect_google_credentials(text: &str) -> Option<(String, String)> {
    let text_lower = text.to_lowercase();

    // Quick rejection: must contain both identifiers
    if !((text_lower.contains("client_id") && text_lower.contains("client_secret"))
        || (text_lower.contains("\u{5ba2}\u{6237}\u{7aef}")
            && text_lower.contains("\u{5bc6}\u{94a5}")))
    {
        return None;
    }

    let mut client_id = None;
    let mut client_secret = None;

    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        // Pattern: client_id: VALUE or GOOGLE_CLIENT_ID=VALUE
        if lower.starts_with("client_id") || lower.starts_with("google_client_id") {
            let value = extract_credential_value(trimmed);
            if !value.is_empty() {
                client_id = Some(value.to_string());
            }
        }
        // Pattern: client_secret: VALUE or GOOGLE_CLIENT_SECRET=VALUE
        if lower.starts_with("client_secret") || lower.starts_with("google_client_secret") {
            let value = extract_credential_value(trimmed);
            if !value.is_empty() {
                client_secret = Some(value.to_string());
            }
        }
    }

    match (client_id, client_secret) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Some((id, secret)),
        _ => None,
    }
}

/// Extract the value from a `key: value` or `key=value` line.
fn extract_credential_value(line: &str) -> &str {
    // Try `: ` separator first, then `=`, then `:`
    let value = if let Some(pos) = line.find(": ") {
        &line[pos + 2..]
    } else if let Some(pos) = line.find('=') {
        &line[pos + 1..]
    } else if let Some(pos) = line.find(':') {
        &line[pos + 1..]
    } else {
        return "";
    };
    value.trim().trim_matches('"').trim_matches('\'')
}

/// Detect if a natural language message embeds Google OAuth credentials.
///
/// This catches patterns like "my client_secret is GOCSPx-xxx" that the
/// structured `detect_google_credentials()` misses. The goal is to prevent
/// secrets from being sent to the LLM API even when users don't follow
/// the structured paste format.
fn detect_credential_leak(text: &str) -> bool {
    let lower = text.to_lowercase();

    // Must mention both client_id-like and client_secret-like content
    let has_id_keyword = lower.contains("client_id")
        || lower.contains("client id")
        || lower.contains("\u{5ba2}\u{6237}\u{7aef}id");
    let has_secret_keyword = lower.contains("client_secret")
        || lower.contains("client secret")
        || lower.contains("\u{5ba2}\u{6237}\u{7aef}\u{5bc6}\u{94a5}");

    if !has_id_keyword || !has_secret_keyword {
        return false;
    }

    // Check for actual credential-like values (not just discussing the concept)
    // Google client IDs end with .apps.googleusercontent.com
    // Google client secrets start with GOCSPX- or are long alphanumeric strings
    let has_real_id = text.contains(".apps.googleusercontent.com") || text.contains(".apps.google");
    let has_real_secret = lower.contains("gocspx-") || lower.contains("gocspx_");

    // If it has the keywords AND looks like real values, it's a leak
    has_real_id || has_real_secret
}

/// Detect a Google OAuth authorization code in a message.
///
/// Google auth codes typically start with `4/` followed by alphanumeric chars.
/// Only matches single-line messages that look like just a code.
pub fn detect_google_auth_code(text: &str) -> bool {
    let trimmed = text.trim();
    // Must be a single line, start with 4/, and be reasonably long
    !trimmed.contains('\n')
        && trimmed.starts_with("4/")
        && trimmed.len() > 10
        && trimmed.len() < 200
        && trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == '/' || c == '-' || c == '_')
}

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;
