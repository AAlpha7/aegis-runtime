use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantEventSource {
    Calendar,
    Drive,
    Chronos,
    Telegram,
    Browser,
    /// Any source string not recognised by this binary version.
    /// Fail-safe: unknown sources are never suppressed — they default to
    /// conversation-visible so events from a newer writer are never lost.
    /// Historical mail-client rows stored before 2026-09-07 also land here.
    #[serde(other)]
    Unknown,
}

impl AssistantEventSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Drive => "drive",
            Self::Chronos => "chronos",
            Self::Telegram => "telegram",
            Self::Browser => "browser",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantEventKind {
    EmailReceived,
    EmailThreadUpdated,
    CalendarInviteReceived,
    CalendarEventChanged,
    DriveFileUploaded,
    DriveFileShared,
    DriveCommentAdded,
    /// Any kind string not recognised by this binary version.
    /// Fail-safe: unknown kinds default to conversation-visible.
    #[serde(other)]
    Unknown,
}

impl AssistantEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmailReceived => "email_received",
            Self::EmailThreadUpdated => "email_thread_updated",
            Self::CalendarInviteReceived => "calendar_invite_received",
            Self::CalendarEventChanged => "calendar_event_changed",
            Self::DriveFileUploaded => "drive_file_uploaded",
            Self::DriveFileShared => "drive_file_shared",
            Self::DriveCommentAdded => "drive_comment_added",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventUrgencyHint {
    Low,
    Medium,
    High,
    /// Unknown urgency value — treated as non-High (fails open via decision check).
    #[serde(other)]
    Unknown,
}

impl EventUrgencyHint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventActionabilityHint {
    Passive,
    Review,
    Act,
    /// Unknown actionability value — treated as non-Act (fails open via decision/source check).
    #[serde(other)]
    Unknown,
}

impl EventActionabilityHint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::Review => "review",
            Self::Act => "act",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantEventDecision {
    NotifyNow,
    CallbackNow,
    DigestLater,
    Taskify,
    MemoryOnly,
    Ignore,
    /// Any decision string not recognised by this binary version.
    /// Fail-safe: unknown decisions are NOT NotifyNow, so they always enter
    /// conversation (the `ev.decision != NotifyNow` guard returns true).
    #[serde(other)]
    Unknown,
}

impl AssistantEventDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotifyNow => "notify_now",
            Self::CallbackNow => "callback_now",
            Self::DigestLater => "digest_later",
            Self::Taskify => "taskify",
            Self::MemoryOnly => "memory_only",
            Self::Ignore => "ignore",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawExternalEvent {
    pub observed_at: String,
    pub user_did: String,
    pub source: AssistantEventSource,
    pub source_account_id: Option<String>,
    pub source_event_id: Option<String>,
    pub dedup_key: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantEvent {
    pub raw_event_id: Option<i64>,
    pub observed_at: String,
    pub occurred_at: String,
    pub user_did: String,
    pub source: AssistantEventSource,
    pub kind: AssistantEventKind,
    pub account_id: Option<String>,
    pub title: String,
    pub summary: String,
    pub thread_key: Option<String>,
    pub urgency_hint: EventUrgencyHint,
    pub actionability_hint: EventActionabilityHint,
    pub decision: AssistantEventDecision,
    pub dedup_key: String,
    pub entities: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredAssistantEvent {
    pub id: i64,
    #[serde(flatten)]
    pub event: AssistantEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedEventBridge {
    pub source: String,
    pub severity: String,
    pub summary: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct AssistantEventDispatch {
    pub observed_event: Option<ObservedEventBridge>,
    pub intent_event: Option<crate::intent::IntentEvent>,
}

/// Strip characters that are markup-significant in Telegram HTML mode
/// from user-controlled text. Used by the `render_*` functions because
/// their output flows into `telegram_render::markdown_to_telegram_html`,
/// whose pulldown-cmark parser surfaces inline HTML (`<a href="…">…</a>`,
/// `<script>`, etc.) as `Event::Html` and PASSES IT THROUGH RAW into the
/// final HTML. An attacker-controlled email subject of the form
/// `<a href="https://evil.com">click</a>` would otherwise reach Telegram
/// HTML mode unsanitized and render as a phishable link.
///
/// Using a strip (replace `<`/`>` with the visually equivalent fullwidth
/// `＜`/`＞`) instead of HTML-entity escape avoids double-escaping when the
/// downstream markdown renderer re-escapes plain text. Stored copies of
/// `summary`/`title` (in the raw_external_event payload, the
/// assistant_events row, the intent_event context) keep the original
/// characters — the strip only applies at the rendering boundary.
pub(crate) fn sanitize_for_telegram_markup(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '<' => '\u{ff1c}', // FULLWIDTH LESS-THAN SIGN
            '>' => '\u{ff1e}', // FULLWIDTH GREATER-THAN SIGN
            // Note: we deliberately do NOT touch `&` here — the markdown
            // renderer's `escape_html` will turn `&` into `&amp;` for
            // plain-text Events, which is the correct behaviour. Inline
            // HTML pass-through (Event::Html) is what we're defending
            // against, and that path is gated on `<…>` syntax.
            other => other,
        })
        .collect()
}

pub fn render_assistant_event_notification(event: &AssistantEvent) -> Option<String> {
    let safe_summary = sanitize_for_telegram_markup(&event.summary);
    let account = sanitize_for_telegram_markup(event.account_id.as_deref().unwrap_or("unknown"));
    let body = match event.decision {
        AssistantEventDecision::NotifyNow => safe_summary,
        AssistantEventDecision::CallbackNow => format!("Review needed: {safe_summary}"),
        AssistantEventDecision::DigestLater
        | AssistantEventDecision::Taskify
        | AssistantEventDecision::MemoryOnly
        | AssistantEventDecision::Ignore => return None,
        // Unknown decision: render as a plain notification (fail-open).
        AssistantEventDecision::Unknown => format!("Event: {safe_summary}"),
    };

    let rendered = crate::telegram_notification_router::render_notification_payload(
        crate::telegram_notification_router::NotificationRenderRequest {
            notification_class:
                crate::telegram_notification_router::NotificationClass::RoutineNotification,
            sensitivity: assistant_event_notification_sensitivity(event),
            urgency: assistant_event_notification_urgency(event),
            channel: crate::telegram_notification_router::NotificationChannel::TelegramDm,
            title: "Assistant event needs review".to_string(),
            safe_summary: body,
            sensitive_summary: None,
            allow_sensitive_detail: false,
            detail_url: None,
        },
    );
    let rendered_body = if rendered.redacted {
        format!("{}: {}", rendered.title, rendered.body)
    } else {
        rendered.body
    };
    Some(format!("📬 [{account}] {rendered_body}"))
}

fn render_assistant_event_redacted_preview(
    event: &AssistantEvent,
    summary: &str,
    channel: crate::telegram_notification_router::NotificationChannel,
) -> String {
    let safe_summary = sanitize_for_telegram_markup(summary);
    let rendered = crate::telegram_notification_router::render_notification_payload(
        crate::telegram_notification_router::NotificationRenderRequest {
            notification_class:
                crate::telegram_notification_router::NotificationClass::RoutineNotification,
            sensitivity: assistant_event_notification_sensitivity(event),
            urgency: assistant_event_notification_urgency(event),
            channel,
            title: "Assistant event needs review".to_string(),
            safe_summary,
            sensitive_summary: None,
            allow_sensitive_detail: false,
            detail_url: None,
        },
    );
    if rendered.redacted {
        format!("{}: {}", rendered.title, rendered.body)
    } else {
        rendered.body
    }
}

fn assistant_event_notification_sensitivity(
    event: &AssistantEvent,
) -> crate::data_access_gate::DataSensitivity {
    event
        .entities
        .get("notification_sensitivity")
        .or_else(|| event.entities.get("sensitivity"))
        .and_then(|value| value.as_str())
        .and_then(parse_assistant_event_sensitivity)
        .unwrap_or(match event.source {
            AssistantEventSource::Calendar | AssistantEventSource::Drive => {
                crate::data_access_gate::DataSensitivity::S2Personal
            }
            AssistantEventSource::Chronos
            | AssistantEventSource::Telegram
            | AssistantEventSource::Browser => {
                crate::data_access_gate::DataSensitivity::S1LowPersonal
            }
            // Unknown source: default to S2Personal (conservative).
            AssistantEventSource::Unknown => crate::data_access_gate::DataSensitivity::S2Personal,
        })
}

fn parse_assistant_event_sensitivity(
    raw: &str,
) -> Option<crate::data_access_gate::DataSensitivity> {
    match raw.trim() {
        "s0_public" | "S0Public" => Some(crate::data_access_gate::DataSensitivity::S0Public),
        "s1_low_personal" | "S1LowPersonal" => {
            Some(crate::data_access_gate::DataSensitivity::S1LowPersonal)
        }
        "s2_personal" | "S2Personal" => Some(crate::data_access_gate::DataSensitivity::S2Personal),
        "s3_sensitive" | "S3Sensitive" => {
            Some(crate::data_access_gate::DataSensitivity::S3Sensitive)
        }
        "s4_highly_sensitive" | "S4HighlySensitive" => {
            Some(crate::data_access_gate::DataSensitivity::S4HighlySensitive)
        }
        _ => None,
    }
}

fn assistant_event_notification_urgency(
    event: &AssistantEvent,
) -> crate::telegram_notification_router::NotificationUrgency {
    match event.urgency_hint {
        EventUrgencyHint::High => {
            crate::telegram_notification_router::NotificationUrgency::Immediate
        }
        EventUrgencyHint::Medium => {
            crate::telegram_notification_router::NotificationUrgency::SameDay
        }
        EventUrgencyHint::Low => crate::telegram_notification_router::NotificationUrgency::Digest,
        // Unknown urgency: treat as SameDay (conservative — not immediate, not digest).
        EventUrgencyHint::Unknown => {
            crate::telegram_notification_router::NotificationUrgency::SameDay
        }
    }
}

pub fn render_assistant_event_digest_message(
    events: &[StoredAssistantEvent],
    limit: usize,
) -> Option<String> {
    let digest_events: Vec<&StoredAssistantEvent> = events
        .iter()
        .filter(|row| row.event.decision == AssistantEventDecision::DigestLater)
        .collect();
    if digest_events.is_empty() {
        return None;
    }

    let shown = digest_events.len().min(limit);
    let mut lines = Vec::with_capacity(shown + 2);
    lines.push(format!(
        "📬 Digest: {} lower-signal event(s)",
        digest_events.len()
    ));
    for row in digest_events.iter().take(limit) {
        // Sanitize each user-controlled portion individually because the
        // rendered string flows into Telegram HTML mode via
        // markdown_to_telegram_html. See `sanitize_for_telegram_markup`.
        let account =
            sanitize_for_telegram_markup(row.event.account_id.as_deref().unwrap_or("unknown"));
        let summary = render_assistant_event_redacted_preview(
            &row.event,
            &row.event.summary,
            crate::telegram_notification_router::NotificationChannel::TelegramDm,
        );
        lines.push(format!("- [{account}] {summary}"));
    }
    if digest_events.len() > shown {
        lines.push(format!("…(+{} more)", digest_events.len() - shown));
    }
    Some(lines.join("\n"))
}

pub fn build_assistant_event_dispatch(event: &AssistantEvent) -> AssistantEventDispatch {
    let observed_event = match event.decision {
        AssistantEventDecision::NotifyNow
        | AssistantEventDecision::CallbackNow
        | AssistantEventDecision::DigestLater
        | AssistantEventDecision::Unknown => {
            let severity = match event.urgency_hint {
                EventUrgencyHint::High => "alert",
                EventUrgencyHint::Medium => "warn",
                EventUrgencyHint::Low | EventUrgencyHint::Unknown => "info",
            };
            Some(ObservedEventBridge {
                source: event.source.as_str().to_string(),
                severity: severity.to_string(),
                summary: event.summary.clone(),
                payload: json!({
                    "kind": event.kind.as_str(),
                    "decision": event.decision.as_str(),
                    "title": event.title,
                    "summary": event.summary,
                    "thread_key": event.thread_key,
                    "account_id": event.account_id,
                    "dedup_key": event.dedup_key,
                    "source": event.source.as_str(),
                }),
            })
        }
        AssistantEventDecision::Taskify
        | AssistantEventDecision::MemoryOnly
        | AssistantEventDecision::Ignore => None,
    };

    let intent_event = match (event.kind, event.decision, event.account_id.as_deref()) {
        (
            AssistantEventKind::EmailReceived,
            AssistantEventDecision::CallbackNow,
            Some(account_id),
        ) => Some(crate::intent::IntentEvent {
            key: format!("email:new:{account_id}"),
            context: Some(format!("1 new email requires review:\n- {}", event.summary)),
            user_did: event.user_did.clone(),
        }),
        _ => None,
    };

    AssistantEventDispatch {
        observed_event,
        intent_event,
    }
}

pub fn render_recent_assistant_events_block(
    events: &[StoredAssistantEvent],
    high_signal_limit: usize,
    digest_limit: usize,
) -> Option<String> {
    let mut high_signal = Vec::new();
    let mut queued_reviews = Vec::new();
    let mut digest_groups: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
    let mut recorded_updates = Vec::new();

    for row in events {
        // Sanitize all user-controlled portions because this block flows
        // into the heartbeat prompt and (in some paths) into Telegram HTML
        // mode via markdown_to_telegram_html. See `sanitize_for_telegram_markup`.
        let account =
            sanitize_for_telegram_markup(row.event.account_id.as_deref().unwrap_or("unknown"));
        let safe_summary = render_assistant_event_redacted_preview(
            &row.event,
            &row.event.summary,
            crate::telegram_notification_router::NotificationChannel::TelegramDm,
        );
        match row.event.decision {
            AssistantEventDecision::NotifyNow => {
                if high_signal.len() < high_signal_limit {
                    high_signal.push(format!(
                        "- {} [{} {} {}] {}",
                        row.event.observed_at,
                        row.event.source.as_str(),
                        account,
                        row.event.decision.as_str(),
                        safe_summary
                    ));
                }
            }
            AssistantEventDecision::CallbackNow => {
                if queued_reviews.len() < high_signal_limit {
                    queued_reviews.push(format!(
                        "- {} [{} {} queued_review] {}",
                        row.event.observed_at,
                        row.event.source.as_str(),
                        account,
                        safe_summary
                    ));
                }
            }
            AssistantEventDecision::DigestLater => {
                let key = (row.event.source.as_str().to_string(), account);
                let entry = digest_groups
                    .entry(key)
                    .or_insert_with(|| (0, safe_summary.clone()));
                entry.0 += 1;
                entry.1 = safe_summary.clone();
            }
            AssistantEventDecision::Taskify | AssistantEventDecision::MemoryOnly => {
                if recorded_updates.len() < digest_limit {
                    recorded_updates.push(format!(
                        "- {} [{} {} {}] {}",
                        row.event.observed_at,
                        row.event.source.as_str(),
                        account,
                        row.event.decision.as_str(),
                        safe_summary
                    ));
                }
            }
            AssistantEventDecision::Ignore => {}
            // Unknown decisions are surfaced as high-signal (fail-open — never hide them).
            AssistantEventDecision::Unknown => {
                if high_signal.len() < high_signal_limit {
                    high_signal.push(format!(
                        "- {} [{} {} {}] {}",
                        row.event.observed_at,
                        row.event.source.as_str(),
                        account,
                        row.event.decision.as_str(),
                        safe_summary
                    ));
                }
            }
        }
    }

    if high_signal.is_empty()
        && queued_reviews.is_empty()
        && digest_groups.is_empty()
        && recorded_updates.is_empty()
    {
        return None;
    }

    let mut block =
        String::from("\n\n<recent_assistant_events>\nRecent external events awaiting triage:\n");

    if !high_signal.is_empty() {
        block.push_str("Important now:\n");
        for line in &high_signal {
            block.push_str(line);
            block.push('\n');
        }
    }

    if !queued_reviews.is_empty() {
        block.push_str("Queued reviews already being handled:\n");
        for line in &queued_reviews {
            block.push_str(line);
            block.push('\n');
        }
    }

    if !digest_groups.is_empty() {
        block.push_str("Digest candidates:\n");
        for ((source, account), (count, latest_summary)) in
            digest_groups.into_iter().take(digest_limit)
        {
            block.push_str(&format!(
                "- [{} {} digest_later] {} event(s); latest: {}\n",
                source, account, count, latest_summary
            ));
        }
    }

    if !recorded_updates.is_empty() {
        block.push_str("Recorded updates:\n");
        for line in &recorded_updates {
            block.push_str(line);
            block.push('\n');
        }
    }

    block.push_str("</recent_assistant_events>");
    Some(block)
}
#[allow(clippy::too_many_arguments)]
pub fn build_calendar_event_changed_event(
    user_did: &str,
    observed_at: &str,
    account_id: &str,
    action: &str,
    calendar_id: &str,
    event_id: &str,
    summary: &str,
    start: Option<&str>,
    end: Option<&str>,
    attendee_count: usize,
) -> (RawExternalEvent, AssistantEvent) {
    let title = match action {
        "create" => format!("Calendar event created: {summary}"),
        "update" => format!("Calendar event updated: {summary}"),
        "delete" => format!("Calendar event deleted: {summary}"),
        _ => format!("Calendar event changed: {summary}"),
    };
    let summary_line = match (start, end) {
        (Some(start), Some(end)) if !start.is_empty() && !end.is_empty() => {
            format!("{title} ({start} → {end})")
        }
        _ => title.clone(),
    };
    let dedup_key = format!(
        "calendar:{account_id}:{calendar_id}:{event_id}:{action}:{}:{}:{}",
        summary.trim(),
        start.unwrap_or(""),
        end.unwrap_or("")
    );
    let payload = json!({
        "action": action,
        "calendar_id": calendar_id,
        "event_id": event_id,
        "summary": summary,
        "start": start,
        "end": end,
        "attendee_count": attendee_count,
    });
    let raw = RawExternalEvent {
        observed_at: observed_at.to_string(),
        user_did: user_did.to_string(),
        source: AssistantEventSource::Calendar,
        source_account_id: Some(account_id.to_string()),
        source_event_id: Some(event_id.to_string()),
        dedup_key: dedup_key.clone(),
        payload: payload.clone(),
    };
    let assistant = AssistantEvent {
        raw_event_id: None,
        observed_at: observed_at.to_string(),
        occurred_at: observed_at.to_string(),
        user_did: user_did.to_string(),
        source: AssistantEventSource::Calendar,
        kind: AssistantEventKind::CalendarEventChanged,
        account_id: Some(account_id.to_string()),
        title,
        summary: summary_line,
        thread_key: Some(format!("calendar:{calendar_id}:{event_id}")),
        // Current Calendar signals are generated from user/assistant-initiated
        // write actions, not external push. The assistant already reports the
        // tool result inline, so enqueueing a second bounded callback review
        // would feel like a split personality rather than a helpful secretary.
        // Keep them visible in durable memory + heartbeat until we have
        // truly external/high-signal calendar deltas to escalate.
        urgency_hint: EventUrgencyHint::Low,
        actionability_hint: EventActionabilityHint::Passive,
        decision: AssistantEventDecision::MemoryOnly,
        dedup_key,
        entities: payload,
    };
    (raw, assistant)
}

pub fn build_drive_file_uploaded_event(
    user_did: &str,
    observed_at: &str,
    account_id: &str,
    file_id: &str,
    name: &str,
    mime_type: &str,
    web_view_link: Option<&str>,
    folder_id: Option<&str>,
) -> (RawExternalEvent, AssistantEvent) {
    let title = format!("Drive file uploaded: {name}");
    let summary = match web_view_link {
        Some(_) => format!("Uploaded Drive file '{name}' ({mime_type})"),
        None => format!("Uploaded Drive file '{name}'"),
    };
    let dedup_key = format!("drive:{account_id}:{file_id}:uploaded");
    let payload = json!({
        "file_id": file_id,
        "name": name,
        "mime_type": mime_type,
        "web_view_link": web_view_link,
        "folder_id": folder_id,
    });
    let raw = RawExternalEvent {
        observed_at: observed_at.to_string(),
        user_did: user_did.to_string(),
        source: AssistantEventSource::Drive,
        source_account_id: Some(account_id.to_string()),
        source_event_id: Some(file_id.to_string()),
        dedup_key: dedup_key.clone(),
        payload: payload.clone(),
    };
    let assistant = AssistantEvent {
        raw_event_id: None,
        observed_at: observed_at.to_string(),
        occurred_at: observed_at.to_string(),
        user_did: user_did.to_string(),
        source: AssistantEventSource::Drive,
        kind: AssistantEventKind::DriveFileUploaded,
        account_id: Some(account_id.to_string()),
        title,
        summary,
        thread_key: Some(format!("drive:{file_id}")),
        // Same rationale as Calendar writes above: current Drive events are
        // local upload success records, not external collaboration signals.
        // They belong in continuity/recall, not in a second proactive review
        // lane that duplicates the already-visible tool outcome.
        urgency_hint: EventUrgencyHint::Low,
        actionability_hint: EventActionabilityHint::Passive,
        decision: AssistantEventDecision::MemoryOnly,
        dedup_key,
        entities: payload,
    };
    (raw, assistant)
}

/// Returns `true` if this event should surface in the owner's conversation
/// timeline (chat L1 push + Telegram). Returns `false` for routine Calendar
/// events with a plain NotifyNow decision that are already represented as
/// first-class items in the Work view.
///
/// Historical mail-client NotifyNow rows (source stored as "gmail" before
/// 2026-09-07) now deserialize as Unknown and fail open into conversation.
/// The signals that keep an event in the conversation are: CallbackNow
/// decision, High urgency, Act actionability, any non-NotifyNow decision,
/// or a non-routine source/kind (fail-safe).
///
/// Classification uses ONLY typed, structured fields on the event
/// (source, kind, urgency_hint, actionability_hint, decision).
/// Never matches on rendered text strings.
///
/// FAIL-SAFE: ambiguous or unknown inputs default to `true` — missing a
/// needs-you event is always worse than noise.
// INVARIANT: suppress ONLY when unambiguously routine across EVERY dimension
// (known routine source+kind, plain NotifyNow, known-low urgency, known-passive/review
// actionability). ANY unknown/ambiguous/actionable signal on ANY dimension → deliver (fail open).
// De-noise suppression requires POSITIVE evidence of routine-ness from the available content;
// absence of evidence (body not scanned) → fail open (surface), never suppress.
pub fn assistant_event_enters_conversation(ev: &AssistantEvent) -> bool {
    // CallbackNow always enters — the owner must be looped in.
    if ev.decision == AssistantEventDecision::CallbackNow {
        return true;
    }
    // High urgency always enters (security alerts, urgent matters).
    if ev.urgency_hint == EventUrgencyHint::High {
        return true;
    }
    // Unknown urgency → fail open (ambiguous dimension → deliver).
    if ev.urgency_hint == EventUrgencyHint::Unknown {
        return true;
    }
    // Actionable events (Act) always enter — the owner must decide, not just be
    // informed via the Mail view. Passive and Review are the "inbox noise" cases.
    if ev.actionability_hint == EventActionabilityHint::Act {
        return true;
    }
    // Unknown actionability → fail open (ambiguous dimension → deliver).
    if ev.actionability_hint == EventActionabilityHint::Unknown {
        return true;
    }
    // Only plain NotifyNow Calendar inbox notifications are suppressed.
    // DigestLater, Taskify, MemoryOnly, Ignore and any future decision → true (fail-safe).
    if ev.decision != AssistantEventDecision::NotifyNow {
        return true;
    }
    // Only Calendar with routine inbox kinds is suppressed.
    // Drive, Chronos, Telegram, Browser and any unrecognised kind → true (fail-safe).
    !matches!(
        (ev.source, ev.kind),
        (
            AssistantEventSource::Calendar,
            AssistantEventKind::CalendarInviteReceived | AssistantEventKind::CalendarEventChanged,
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_recent_assistant_events_block_includes_recorded_updates() {
        let (_, calendar_event) = build_calendar_event_changed_event(
            "did:aegis:test",
            "2026-04-22T01:00:00Z",
            "primary",
            "create",
            "primary",
            "evt-1",
            "Standup",
            Some("2026-04-22T09:00:00Z"),
            Some("2026-04-22T09:15:00Z"),
            1,
        );
        let (_, drive_event) = build_drive_file_uploaded_event(
            "did:aegis:test",
            "2026-04-22T01:10:00Z",
            "primary",
            "file-1",
            "notes.txt",
            "text/plain",
            Some("https://drive.example/file-1"),
            None,
        );

        let block = render_recent_assistant_events_block(
            &[
                StoredAssistantEvent {
                    id: 1,
                    event: calendar_event,
                },
                StoredAssistantEvent {
                    id: 2,
                    event: drive_event,
                },
            ],
            8,
            4,
        )
        .expect("block should render");

        assert!(block.contains("Recorded updates:"));
        assert!(block.contains("calendar"));
        assert!(block.contains("drive"));
        assert!(block.contains("Standup"));
        assert!(block.contains("notes.txt"));
    }

    #[test]
    fn calendar_and_drive_builders_default_to_memory_only() {
        let (_, calendar_event) = build_calendar_event_changed_event(
            "did:aegis:test",
            "2026-04-22T01:00:00Z",
            "primary",
            "update",
            "primary",
            "evt-1",
            "1:1",
            Some("2026-04-22T09:00:00Z"),
            Some("2026-04-22T09:30:00Z"),
            0,
        );
        let (_, drive_event) = build_drive_file_uploaded_event(
            "did:aegis:test",
            "2026-04-22T01:10:00Z",
            "primary",
            "file-1",
            "notes.txt",
            "text/plain",
            None,
            Some("folder-1"),
        );

        assert_eq!(calendar_event.source, AssistantEventSource::Calendar);
        assert_eq!(
            calendar_event.kind,
            AssistantEventKind::CalendarEventChanged
        );
        assert_eq!(calendar_event.decision, AssistantEventDecision::MemoryOnly);
        assert_eq!(drive_event.source, AssistantEventSource::Drive);
        assert_eq!(drive_event.kind, AssistantEventKind::DriveFileUploaded);
        assert_eq!(drive_event.decision, AssistantEventDecision::MemoryOnly);
    }
    #[test]
    fn sanitize_for_telegram_markup_replaces_angle_brackets_and_preserves_text() {
        let input = "<a href=\"x\">y</a>";
        let cleaned = sanitize_for_telegram_markup(input);
        assert!(!cleaned.contains('<'));
        assert!(!cleaned.contains('>'));
        // Non-bracket content survives byte-for-byte (full-width replacement
        // changes only `<` and `>`).
        assert!(cleaned.contains("a href"));
        assert!(cleaned.contains('y'));
        assert!(cleaned.contains('"'));
    }
    #[test]
    fn classifier_unknown_source_notify_now_fails_open() {
        // Historical mail-client rows (and any unrecognised source) deserialize
        // as Unknown and must fail open into conversation.
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "unknown",
            "kind": "email_received",
            "account_id": "primary",
            "title": "Payment of $150 received",
            "summary": "Your payment has been processed",
            "thread_key": null,
            "urgency_hint": "medium",
            "actionability_hint": "passive",
            "decision": "notify_now",
            "dedup_key": "unknown:test:payment1",
            "entities": { "notification_sensitivity": "S3Sensitive" }
        }))
        .unwrap();
        assert!(
            assistant_event_enters_conversation(&ev),
            "unknown source with plain NotifyNow must fail open into conversation"
        );
    }

    #[test]
    fn classifier_s3_sensitive_callback_now_still_enters() {
        // Sensitivity does NOT suppress; only routine-ness does.
        // If the same S3 email also triggers CallbackNow (owner must act),
        // it still enters the conversation.
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "calendar",
            "kind": "calendar_invite_received",
            "account_id": "primary",
            "title": "Urgent: legal document requires your signature",
            "summary": "Please sign by EOD",
            "thread_key": null,
            "urgency_hint": "medium",
            "actionability_hint": "act",
            "decision": "callback_now",
            "dedup_key": "calendar:test:legal-callback1",
            "entities": { "notification_sensitivity": "S3Sensitive" }
        }))
        .unwrap();
        assert!(
            assistant_event_enters_conversation(&ev),
            "S3-sensitive email with CallbackNow must still enter conversation"
        );
    }
    #[test]
    fn classifier_calendar_invite_received_routine_returns_false() {
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "calendar",
            "kind": "calendar_invite_received",
            "account_id": "primary",
            "title": "Team meeting",
            "summary": "Weekly team sync",
            "thread_key": null,
            "urgency_hint": "low",
            "actionability_hint": "passive",
            "decision": "notify_now",
            "dedup_key": "calendar:test:invite1",
            "entities": {}
        }))
        .unwrap();
        assert!(!assistant_event_enters_conversation(&ev));
    }

    #[test]
    fn classifier_calendar_event_changed_routine_returns_false() {
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "calendar",
            "kind": "calendar_event_changed",
            "account_id": "primary",
            "title": "Team meeting updated",
            "summary": "Time changed to 3pm",
            "thread_key": null,
            "urgency_hint": "low",
            "actionability_hint": "passive",
            "decision": "notify_now",
            "dedup_key": "calendar:test:changed1",
            "entities": {}
        }))
        .unwrap();
        assert!(!assistant_event_enters_conversation(&ev));
    }

    #[test]
    fn classifier_calendar_high_urgency_enters_conversation() {
        // Calendar invite with urgency=high (e.g. urgent meeting) → enters
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "calendar",
            "kind": "calendar_invite_received",
            "account_id": "primary",
            "title": "Emergency board meeting",
            "summary": "Urgent: board call now",
            "thread_key": null,
            "urgency_hint": "high",
            "actionability_hint": "act",
            "decision": "notify_now",
            "dedup_key": "calendar:test:urgent1",
            "entities": {}
        }))
        .unwrap();
        assert!(assistant_event_enters_conversation(&ev));
    }

    #[test]
    fn classifier_drive_file_uploaded_enters_conversation() {
        // Drive = non-mail/calendar source → true (fail-safe)
        let ev: AssistantEvent = serde_json::from_value(serde_json::json!({
            "raw_event_id": null,
            "observed_at": "2026-06-30T00:00:00Z",
            "occurred_at": "2026-06-30T00:00:00Z",
            "user_did": "did:aegis:test",
            "source": "drive",
            "kind": "drive_file_uploaded",
            "account_id": "primary",
            "title": "File uploaded",
            "summary": "document.pdf uploaded",
            "thread_key": null,
            "urgency_hint": "low",
            "actionability_hint": "passive",
            "decision": "notify_now",
            "dedup_key": "drive:test:file1",
            "entities": {}
        }))
        .unwrap();
        assert!(assistant_event_enters_conversation(&ev));
    }
    // ── FIX B regression (unknown enum variants) ─────────────────────────────

    #[test]
    fn unknown_decision_variant_is_conversation_visible() {
        // Regression (FIX B): an event with an unrecognised decision string
        // (e.g. from a newer binary writing a new kind) must NOT be suppressed.
        // Unknown decision != NotifyNow → enters conversation (fail-safe).
        let ev = AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-06-30T00:00:00Z".to_string(),
            occurred_at: "2026-06-30T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: AssistantEventSource::Calendar,
            kind: AssistantEventKind::EmailReceived,
            account_id: Some("primary".to_string()),
            title: "Future event type".to_string(),
            summary: "Written by a newer version".to_string(),
            thread_key: None,
            urgency_hint: EventUrgencyHint::Medium,
            actionability_hint: EventActionabilityHint::Review,
            decision: AssistantEventDecision::Unknown,
            dedup_key: "calendar:test:fix-b-unknown-decision".to_string(),
            entities: serde_json::json!({}),
        };
        assert!(
            assistant_event_enters_conversation(&ev),
            "unknown decision must default to conversation-visible (fail-safe)"
        );
    }

    #[test]
    fn unknown_source_variant_is_conversation_visible() {
        // Regression (FIX B): unknown source → not in the Google/Calendar
        // suppression pattern → !matches!(...) = true → conversation-visible.
        let ev = AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-06-30T00:00:00Z".to_string(),
            occurred_at: "2026-06-30T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: AssistantEventSource::Unknown,
            kind: AssistantEventKind::Unknown,
            account_id: None,
            title: "New source event".to_string(),
            summary: "From an unknown integration".to_string(),
            thread_key: None,
            urgency_hint: EventUrgencyHint::Medium,
            actionability_hint: EventActionabilityHint::Review,
            decision: AssistantEventDecision::NotifyNow,
            dedup_key: "test:fix-b-unknown-source".to_string(),
            entities: serde_json::json!({}),
        };
        assert!(
            assistant_event_enters_conversation(&ev),
            "unknown source/kind must default to conversation-visible (fail-safe)"
        );
    }
    #[test]
    fn unknown_urgency_not_suppressed() {
        // Unknown urgency must fail open → deliver to conversation even when
        // source/kind/decision would otherwise mark it as routine.
        let ev = AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-06-30T00:00:00Z".to_string(),
            occurred_at: "2026-06-30T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: AssistantEventSource::Calendar,
            kind: AssistantEventKind::EmailReceived,
            account_id: None,
            title: "Test subject".to_string(),
            summary: "Some email".to_string(),
            thread_key: None,
            urgency_hint: EventUrgencyHint::Unknown,
            actionability_hint: EventActionabilityHint::Review,
            decision: AssistantEventDecision::NotifyNow,
            dedup_key: "test:unknown-urgency".to_string(),
            entities: serde_json::json!({}),
        };
        assert!(
            assistant_event_enters_conversation(&ev),
            "Unknown urgency must not be suppressed (fail open)"
        );
    }

    #[test]
    fn unknown_actionability_not_suppressed() {
        // Unknown actionability must fail open → deliver to conversation even when
        // source/kind/decision/urgency would otherwise mark it as routine.
        let ev = AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-06-30T00:00:00Z".to_string(),
            occurred_at: "2026-06-30T00:00:00Z".to_string(),
            user_did: "did:aegis:test".to_string(),
            source: AssistantEventSource::Calendar,
            kind: AssistantEventKind::EmailReceived,
            account_id: None,
            title: "Test subject".to_string(),
            summary: "Some email".to_string(),
            thread_key: None,
            urgency_hint: EventUrgencyHint::Low,
            actionability_hint: EventActionabilityHint::Unknown,
            decision: AssistantEventDecision::NotifyNow,
            dedup_key: "test:unknown-actionability".to_string(),
            entities: serde_json::json!({}),
        };
        assert!(
            assistant_event_enters_conversation(&ev),
            "Unknown actionability must not be suppressed (fail open)"
        );
    }
}
