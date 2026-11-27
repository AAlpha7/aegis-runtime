use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkSourceKind {
    InteractiveMessage,
    AssistantEvent,
    DeferredIntent,
    HeartbeatFollowUp,
    ApprovalFollowUp,
}

impl AssistantWorkSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InteractiveMessage => "interactive_message",
            Self::AssistantEvent => "assistant_event",
            Self::DeferredIntent => "deferred_intent",
            Self::HeartbeatFollowUp => "heartbeat_followup",
            Self::ApprovalFollowUp => "approval_followup",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "interactive_message" => Self::InteractiveMessage,
            "assistant_event" => Self::AssistantEvent,
            "deferred_intent" => Self::DeferredIntent,
            "heartbeat_followup" => Self::HeartbeatFollowUp,
            "approval_followup" => Self::ApprovalFollowUp,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkMode {
    Interactive,
    BackgroundReview,
    BackgroundAction,
    DigestAssembly,
}

impl AssistantWorkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::BackgroundReview => "background_review",
            Self::BackgroundAction => "background_action",
            Self::DigestAssembly => "digest_assembly",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "interactive" => Self::Interactive,
            "background_review" => Self::BackgroundReview,
            "background_action" => Self::BackgroundAction,
            "digest_assembly" => Self::DigestAssembly,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkPriority {
    InterruptNow,
    Soon,
    Later,
}

impl AssistantWorkPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InterruptNow => "interrupt_now",
            Self::Soon => "soon",
            Self::Later => "later",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "interrupt_now" => Self::InterruptNow,
            "soon" => Self::Soon,
            "later" => Self::Later,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkStatus {
    Queued,
    Running,
    WaitingUser,
    Done,
    Cancelled,
}

impl AssistantWorkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingUser => "waiting_user",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "waiting_user" => Self::WaitingUser,
            "done" => Self::Done,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkDeliveryPolicy {
    InlineReply,
    ProactiveMessage,
    DigestOnly,
    Silent,
}

impl AssistantWorkDeliveryPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineReply => "inline_reply",
            Self::ProactiveMessage => "proactive_message",
            Self::DigestOnly => "digest_only",
            Self::Silent => "silent",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "inline_reply" => Self::InlineReply,
            "proactive_message" => Self::ProactiveMessage,
            "digest_only" => Self::DigestOnly,
            "silent" => Self::Silent,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkCallerContext {
    Interactive,
    Callback,
    Heartbeat,
}

impl AssistantWorkCallerContext {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Callback => "callback",
            Self::Heartbeat => "heartbeat",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "interactive" => Self::Interactive,
            "callback" => Self::Callback,
            "heartbeat" => Self::Heartbeat,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWorkAnchorKind {
    EmailThread,
    DeferredIntentGroup,
    DeferredIntent,
    AssistantEvent,
}

impl AssistantWorkAnchorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmailThread => "email_thread",
            Self::DeferredIntentGroup => "deferred_intent_group",
            Self::DeferredIntent => "deferred_intent",
            Self::AssistantEvent => "assistant_event",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "email_thread" => Self::EmailThread,
            "deferred_intent_group" => Self::DeferredIntentGroup,
            "deferred_intent" => Self::DeferredIntent,
            "assistant_event" => Self::AssistantEvent,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantTurnDelivery {
    ProactiveMessage,
    DigestOnly,
    Silent,
}

impl AssistantTurnDelivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProactiveMessage => "proactive_message",
            Self::DigestOnly => "digest_only",
            Self::Silent => "silent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantWorkBudgetProfile {
    pub max_rounds: u8,
    pub max_tool_calls: u8,
    pub timeout_secs: u32,
}

impl AssistantWorkBudgetProfile {
    pub fn bounded_callback_review() -> Self {
        Self {
            max_rounds: 3,
            max_tool_calls: 4,
            timeout_secs: 45,
        }
    }

    pub fn bounded_deferred_intent_action() -> Self {
        Self {
            max_rounds: 3,
            max_tool_calls: 5,
            // 60 s could not hold three grok-4.6 rounds (13–26 s each); the
            // daily summary callback timed out mid-run and its retry repeated
            // every side effect (duplicate KB rows, 2026-09-08 03:10 UTC).
            timeout_secs: 150,
        }
    }

    pub fn heartbeat_synthesis() -> Self {
        Self {
            max_rounds: 2,
            max_tool_calls: 3,
            timeout_secs: 45,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantWorkItem {
    pub created_at: String,
    pub updated_at: String,
    pub user_did: String,
    pub source_kind: AssistantWorkSourceKind,
    pub source_ref: Option<String>,
    pub anchor_kind: Option<AssistantWorkAnchorKind>,
    pub anchor_ref: Option<String>,
    pub assistant_event_id: Option<i64>,
    pub goal: String,
    pub mode: AssistantWorkMode,
    pub priority: AssistantWorkPriority,
    pub status: AssistantWorkStatus,
    pub delivery_policy: AssistantWorkDeliveryPolicy,
    pub caller_context: AssistantWorkCallerContext,
    pub dedup_key: String,
    pub budget_profile: AssistantWorkBudgetProfile,
    pub context: Value,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredAssistantWorkItem {
    pub id: i64,
    #[serde(flatten)]
    pub item: AssistantWorkItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatFollowUpKind {
    Escalation,
    DigestAssembly,
    SilentReprioritization,
}

impl HeartbeatFollowUpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Escalation => "escalation",
            Self::DigestAssembly => "digest_assembly",
            Self::SilentReprioritization => "silent_reprioritization",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantWorkClaimFilter {
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub source_kinds: Vec<String>,
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub caller_contexts: Vec<String>,
}

impl AssistantWorkClaimFilter {
    pub fn callback_review() -> Self {
        Self {
            statuses: vec![AssistantWorkStatus::Queued.as_str().to_string()],
            source_kinds: vec![AssistantWorkSourceKind::AssistantEvent.as_str().to_string()],
            modes: vec![AssistantWorkMode::BackgroundReview.as_str().to_string()],
            caller_contexts: vec![AssistantWorkCallerContext::Callback.as_str().to_string()],
        }
    }

    pub fn deferred_intent_callback() -> Self {
        Self {
            statuses: vec![AssistantWorkStatus::Queued.as_str().to_string()],
            source_kinds: vec![AssistantWorkSourceKind::DeferredIntent.as_str().to_string()],
            modes: vec![AssistantWorkMode::BackgroundAction.as_str().to_string()],
            caller_contexts: vec![AssistantWorkCallerContext::Callback.as_str().to_string()],
        }
    }

    pub fn heartbeat_follow_up() -> Self {
        Self {
            statuses: vec![AssistantWorkStatus::Queued.as_str().to_string()],
            source_kinds: vec![
                AssistantWorkSourceKind::HeartbeatFollowUp
                    .as_str()
                    .to_string(),
            ],
            modes: vec![AssistantWorkMode::BackgroundReview.as_str().to_string()],
            caller_contexts: vec![AssistantWorkCallerContext::Heartbeat.as_str().to_string()],
        }
    }

    pub fn heartbeat_digest_assembly() -> Self {
        Self {
            statuses: vec![AssistantWorkStatus::Queued.as_str().to_string()],
            source_kinds: vec![
                AssistantWorkSourceKind::HeartbeatFollowUp
                    .as_str()
                    .to_string(),
            ],
            modes: vec![AssistantWorkMode::DigestAssembly.as_str().to_string()],
            caller_contexts: vec![AssistantWorkCallerContext::Heartbeat.as_str().to_string()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantProposedTask {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantProposedReminder {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurnOutcome {
    pub generated_at: String,
    pub internal_summary: String,
    pub user_message: Option<String>,
    pub delivery: AssistantTurnDelivery,
    pub notify_priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_task: Option<AssistantProposedTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_reminder: Option<AssistantProposedReminder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up_anchor_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up_anchor_ref: Option<String>,
    #[serde(default)]
    pub needs_user_reply: bool,
    pub tools_called: Vec<String>,
    pub rounds_used: usize,
}

impl AssistantTurnOutcome {
    pub fn silent(summary: impl Into<String>) -> Self {
        Self {
            generated_at: chrono::Utc::now().to_rfc3339(),
            internal_summary: summary.into(),
            user_message: None,
            delivery: AssistantTurnDelivery::Silent,
            notify_priority: None,
            proposed_task: None,
            proposed_reminder: None,
            follow_up_anchor_kind: None,
            follow_up_anchor_ref: None,
            needs_user_reply: false,
            tools_called: Vec::new(),
            rounds_used: 0,
        }
    }
}

pub fn build_callback_review_work_item(
    event: &crate::assistant_events::StoredAssistantEvent,
) -> AssistantWorkItem {
    let now = chrono::Utc::now().to_rfc3339();
    let anchor_kind = event
        .event
        .thread_key
        .as_ref()
        .map(|_| AssistantWorkAnchorKind::EmailThread)
        .or(Some(AssistantWorkAnchorKind::AssistantEvent));
    let anchor_ref = event
        .event
        .thread_key
        .clone()
        .or_else(|| Some(format!("assistant_event:{}", event.id)));
    AssistantWorkItem {
        created_at: now.clone(),
        updated_at: now,
        user_did: event.event.user_did.clone(),
        source_kind: AssistantWorkSourceKind::AssistantEvent,
        source_ref: Some(format!("assistant_event:{}", event.id)),
        anchor_kind,
        anchor_ref: anchor_ref.clone(),
        assistant_event_id: Some(event.id),
        goal: format!(
            "Review the external update, decide whether the user should be interrupted now, and prepare the next best action: {}",
            event.event.summary
        ),
        mode: AssistantWorkMode::BackgroundReview,
        priority: AssistantWorkPriority::InterruptNow,
        status: AssistantWorkStatus::Queued,
        delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
        caller_context: AssistantWorkCallerContext::Callback,
        dedup_key: format!("assistant_work:callback_review:{}", event.event.dedup_key),
        budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
        context: json!({
            "assistant_event_id": event.id,
            "assistant_event_dedup_key": event.event.dedup_key,
            "source": event.event.source.as_str(),
            "kind": event.event.kind.as_str(),
            "decision": event.event.decision.as_str(),
            "thread_key": event.event.thread_key,
            "account_id": event.event.account_id,
            "title": event.event.title,
            "summary": event.event.summary,
            "anchor_kind": anchor_kind.map(|v| v.as_str()),
            "anchor_ref": anchor_ref,
        }),
        result: None,
    }
}

pub fn build_deferred_intent_work_item(
    intent: &crate::memory::DeferredIntent,
) -> AssistantWorkItem {
    let now = chrono::Utc::now().to_rfc3339();
    let fire_ordinal = i64::from(intent.fire_count) + 1;
    let trigger_descriptor = match intent.trigger_type.as_str() {
        "event" => intent
            .event_key
            .as_deref()
            .map(|key| format!("event:{key}"))
            .unwrap_or_else(|| "event:unknown".to_string()),
        "time" => intent
            .trigger_at_unix
            .map(|ts| format!("time:{ts}"))
            .unwrap_or_else(|| "time:unknown".to_string()),
        other => format!("unknown:{other}"),
    };

    AssistantWorkItem {
        created_at: now.clone(),
        updated_at: now,
        user_did: intent.user_did.clone(),
        source_kind: AssistantWorkSourceKind::DeferredIntent,
        source_ref: Some(format!("deferred_intent:{}", intent.id)),
        anchor_kind: intent
            .group_key
            .as_ref()
            .map(|_| AssistantWorkAnchorKind::DeferredIntentGroup)
            .or(Some(AssistantWorkAnchorKind::DeferredIntent)),
        anchor_ref: intent
            .group_key
            .clone()
            .or_else(|| Some(format!("deferred_intent:{}", intent.id))),
        assistant_event_id: None,
        goal: format!(
            "Carry out the scheduled follow-up or triggered action and decide whether the user should hear about it now: {}",
            intent.context_text
        ),
        mode: AssistantWorkMode::BackgroundAction,
        priority: AssistantWorkPriority::Soon,
        status: AssistantWorkStatus::Queued,
        delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
        caller_context: AssistantWorkCallerContext::Callback,
        dedup_key: format!(
            "assistant_work:deferred_intent:{}:{}:{}",
            intent.id, fire_ordinal, trigger_descriptor
        ),
        budget_profile: AssistantWorkBudgetProfile::bounded_deferred_intent_action(),
        context: json!({
            "intent_id": intent.id,
            "chat_id": intent.chat_id,
            "trigger_type": intent.trigger_type,
            "trigger_at_unix": intent.trigger_at_unix,
            "event_key": intent.event_key,
            "execution_mode": intent.execution_mode,
            "context_text": intent.context_text,
            "interval_seconds": intent.interval_seconds,
            "chain_depth": intent.chain_depth,
            "pin_on_fire": intent.pin_on_fire,
            "group_key": intent.group_key,
            "max_fires": intent.max_fires,
            "fire_count": intent.fire_count,
            "fire_ordinal": fire_ordinal,
            "is_callback_origin": intent.is_callback_origin,
            "created_at": intent.created_at,
            "anchor_kind": intent
                .group_key
                .as_ref()
                .map(|_| AssistantWorkAnchorKind::DeferredIntentGroup.as_str())
                .unwrap_or(AssistantWorkAnchorKind::DeferredIntent.as_str()),
            "anchor_ref": intent
                .group_key
                .clone()
                .unwrap_or_else(|| format!("deferred_intent:{}", intent.id)),
        }),
        result: None,
    }
}

pub fn build_heartbeat_escalation_work_item(
    user_did: &str,
    goal: impl Into<String>,
    dedup_key: impl Into<String>,
    anchor_kind: Option<AssistantWorkAnchorKind>,
    anchor_ref: Option<String>,
    reason: impl Into<String>,
) -> AssistantWorkItem {
    let now = chrono::Utc::now().to_rfc3339();
    AssistantWorkItem {
        created_at: now.clone(),
        updated_at: now,
        user_did: user_did.to_string(),
        source_kind: AssistantWorkSourceKind::HeartbeatFollowUp,
        source_ref: Some("heartbeat_followup:escalation".to_string()),
        anchor_kind,
        anchor_ref: anchor_ref.clone(),
        assistant_event_id: None,
        goal: goal.into(),
        mode: AssistantWorkMode::BackgroundReview,
        priority: AssistantWorkPriority::Soon,
        status: AssistantWorkStatus::Queued,
        delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
        caller_context: AssistantWorkCallerContext::Heartbeat,
        dedup_key: dedup_key.into(),
        budget_profile: AssistantWorkBudgetProfile::heartbeat_synthesis(),
        context: json!({
            "heartbeat_follow_up_kind": HeartbeatFollowUpKind::Escalation.as_str(),
            "reason": reason.into(),
            "anchor_kind": anchor_kind.map(|v| v.as_str()),
            "anchor_ref": anchor_ref,
        }),
        result: None,
    }
}

pub fn build_heartbeat_digest_work_item(
    user_did: &str,
    goal: impl Into<String>,
    dedup_key: impl Into<String>,
    reason: impl Into<String>,
    assistant_event_consumer: impl Into<String>,
    first_event_id: i64,
    last_event_id: i64,
    event_count: usize,
) -> AssistantWorkItem {
    let now = chrono::Utc::now().to_rfc3339();
    AssistantWorkItem {
        created_at: now.clone(),
        updated_at: now,
        user_did: user_did.to_string(),
        source_kind: AssistantWorkSourceKind::HeartbeatFollowUp,
        source_ref: Some("heartbeat_followup:digest".to_string()),
        anchor_kind: None,
        anchor_ref: None,
        assistant_event_id: None,
        goal: goal.into(),
        mode: AssistantWorkMode::DigestAssembly,
        priority: AssistantWorkPriority::Later,
        status: AssistantWorkStatus::Queued,
        delivery_policy: AssistantWorkDeliveryPolicy::DigestOnly,
        caller_context: AssistantWorkCallerContext::Heartbeat,
        dedup_key: dedup_key.into(),
        budget_profile: AssistantWorkBudgetProfile::heartbeat_synthesis(),
        context: json!({
            "heartbeat_follow_up_kind": HeartbeatFollowUpKind::DigestAssembly.as_str(),
            "reason": reason.into(),
            "assistant_event_consumer": assistant_event_consumer.into(),
            "first_event_id": first_event_id,
            "last_event_id": last_event_id,
            "event_count": event_count,
        }),
        result: None,
    }
}

pub fn build_heartbeat_silent_reprioritization_work_item(
    user_did: &str,
    goal: impl Into<String>,
    dedup_key: impl Into<String>,
    anchor_kind: Option<AssistantWorkAnchorKind>,
    anchor_ref: Option<String>,
    reason: impl Into<String>,
) -> AssistantWorkItem {
    let now = chrono::Utc::now().to_rfc3339();
    AssistantWorkItem {
        created_at: now.clone(),
        updated_at: now,
        user_did: user_did.to_string(),
        source_kind: AssistantWorkSourceKind::HeartbeatFollowUp,
        source_ref: Some("heartbeat_followup:silent_reprioritization".to_string()),
        anchor_kind,
        anchor_ref: anchor_ref.clone(),
        assistant_event_id: None,
        goal: goal.into(),
        mode: AssistantWorkMode::BackgroundReview,
        priority: AssistantWorkPriority::Later,
        status: AssistantWorkStatus::Queued,
        delivery_policy: AssistantWorkDeliveryPolicy::Silent,
        caller_context: AssistantWorkCallerContext::Heartbeat,
        dedup_key: dedup_key.into(),
        budget_profile: AssistantWorkBudgetProfile::heartbeat_synthesis(),
        context: json!({
            "heartbeat_follow_up_kind": HeartbeatFollowUpKind::SilentReprioritization.as_str(),
            "reason": reason.into(),
            "anchor_kind": anchor_kind.map(|v| v.as_str()),
            "anchor_ref": anchor_ref,
        }),
        result: None,
    }
}

pub fn derive_heartbeat_escalation_work_items(
    user_did: &str,
    items: &[StoredAssistantWorkItem],
    now: chrono::DateTime<chrono::Utc>,
    stale_after: chrono::Duration,
) -> Vec<AssistantWorkItem> {
    let mut derived = Vec::new();

    for row in items {
        if row.item.source_kind == AssistantWorkSourceKind::HeartbeatFollowUp {
            continue;
        }

        let age_is_stale = chrono::DateTime::parse_from_rfc3339(&row.item.updated_at)
            .map(|dt| now.signed_duration_since(dt.with_timezone(&chrono::Utc)) >= stale_after)
            .unwrap_or(false);

        let (reason, goal) = match row.item.status {
            AssistantWorkStatus::WaitingUser if age_is_stale => (
                "waiting_user_overdue",
                format!(
                    "Follow up on unresolved assistant recommendation that is still waiting on the user: {}",
                    row.item.goal
                ),
            ),
            _ => continue,
        };

        let anchor_token = row
            .item
            .anchor_ref
            .clone()
            .unwrap_or_else(|| row.item.dedup_key.clone());
        let dedup_key = format!(
            "assistant_work:heartbeat_escalation:{}:{}",
            reason, anchor_token
        );
        derived.push(build_heartbeat_escalation_work_item(
            user_did,
            goal,
            dedup_key,
            row.item.anchor_kind,
            row.item.anchor_ref.clone(),
            reason,
        ));
    }

    derived
}

pub fn derive_heartbeat_silent_reprioritization_work_items(
    user_did: &str,
    items: &[StoredAssistantWorkItem],
    now: chrono::DateTime<chrono::Utc>,
    stale_after: chrono::Duration,
) -> Vec<AssistantWorkItem> {
    let mut derived = Vec::new();

    for row in items {
        if row.item.source_kind == AssistantWorkSourceKind::HeartbeatFollowUp {
            continue;
        }

        let age_is_stale = chrono::DateTime::parse_from_rfc3339(&row.item.updated_at)
            .map(|dt| now.signed_duration_since(dt.with_timezone(&chrono::Utc)) >= stale_after)
            .unwrap_or(false);

        if row.item.status != AssistantWorkStatus::Queued || !age_is_stale {
            continue;
        }

        let reason = "queued_work_stale";
        let goal = format!(
            "Silently reprioritize stale queued assistant work and decide whether it can remain deferred: {}",
            row.item.goal
        );
        let anchor_token = row
            .item
            .anchor_ref
            .clone()
            .unwrap_or_else(|| row.item.dedup_key.clone());
        let dedup_key = format!(
            "assistant_work:heartbeat_reprioritization:{}:{}",
            reason, anchor_token
        );
        derived.push(build_heartbeat_silent_reprioritization_work_item(
            user_did,
            goal,
            dedup_key,
            row.item.anchor_kind,
            row.item.anchor_ref.clone(),
            reason,
        ));
    }

    derived
}

pub fn derive_heartbeat_digest_work_item(
    user_did: &str,
    active_items: &[StoredAssistantWorkItem],
    digest_events: &[crate::assistant_events::StoredAssistantEvent],
    min_batch: usize,
    window_secs: i64,
) -> Option<AssistantWorkItem> {
    let digest_already_active = active_items.iter().any(|row| {
        row.item.source_kind == AssistantWorkSourceKind::HeartbeatFollowUp
            && row.item.mode == AssistantWorkMode::DigestAssembly
            && matches!(
                row.item.status,
                AssistantWorkStatus::Queued
                    | AssistantWorkStatus::Running
                    | AssistantWorkStatus::WaitingUser
            )
    });
    if digest_already_active || digest_events.is_empty() {
        return None;
    }

    let digest_ready = digest_events.len() >= min_batch
        || if window_secs <= 0 {
            true
        } else {
            digest_events
                .first()
                .and_then(|row| chrono::DateTime::parse_from_rfc3339(&row.event.observed_at).ok())
                .map(|observed| {
                    chrono::Utc::now().signed_duration_since(observed.with_timezone(&chrono::Utc))
                        >= chrono::Duration::seconds(window_secs)
                })
                .unwrap_or(true)
        };
    if !digest_ready {
        return None;
    }

    let first_event_id = digest_events.first().map(|row| row.id)?;
    let last_event_id = digest_events.last().map(|row| row.id)?;
    Some(build_heartbeat_digest_work_item(
        user_did,
        "Assemble a concise digest covering the accumulated lower-signal mail backlog.",
        format!("assistant_work:heartbeat_digest:calendar:{first_event_id}:{last_event_id}"),
        "digest_batch_matured",
        "mail_notifier_digest",
        first_event_id,
        last_event_id,
        digest_events.len(),
    ))
}

pub fn render_callback_review_prompt(
    work_item: &AssistantWorkItem,
    event: &crate::assistant_events::StoredAssistantEvent,
) -> String {
    format!(
        "[ASSISTANT_WORK_ITEM]\n\
         You are continuing the same assistant relationship in a bounded background review turn.\n\
         This is not a fresh user message. Treat it as a high-priority work item that should stay concise, grounded, and action-oriented.\n\n\
         Goal:\n{goal}\n\n\
         Budget:\n\
         - max_rounds: {max_rounds}\n\
         - max_tool_calls: {max_tool_calls}\n\
         - timeout_secs: {timeout_secs}\n\n\
         External event:\n\
         - source: {source}\n\
         - kind: {kind}\n\
         - account: {account}\n\
         - anchor_kind: {anchor_kind}\n\
         - anchor_ref: {anchor_ref}\n\
         - title: {title}\n\
         - summary: {summary}\n\
         - thread_key: {thread_key}\n\
         - observed_at: {observed_at}\n\
         - entities_json: {entities}\n\n\
         Instructions:\n\
         - First decide whether the user should be interrupted now.\n\
         - If yes, call notify_user with a concise message that explains why this matters now and the next best action.\n\
         - If a short draft or follow-up suggestion would help, include it in the notify_user message.\n\
         - If the event is not worth interrupting the user for right now, stay silent.\n\
         - Do not mention internal terms like callback, work item, heartbeat, or assistant event.\n\
         - If you need missing context, use tools sparingly and stay within budget.",
        goal = work_item.goal,
        max_rounds = work_item.budget_profile.max_rounds,
        max_tool_calls = work_item.budget_profile.max_tool_calls,
        timeout_secs = work_item.budget_profile.timeout_secs,
        source = event.event.source.as_str(),
        kind = event.event.kind.as_str(),
        account = event.event.account_id.as_deref().unwrap_or("unknown"),
        anchor_kind = work_item
            .anchor_kind
            .map(|v| v.as_str())
            .unwrap_or("(none)"),
        anchor_ref = work_item.anchor_ref.as_deref().unwrap_or("(none)"),
        title = event.event.title,
        summary = event.event.summary,
        thread_key = event.event.thread_key.as_deref().unwrap_or("(none)"),
        observed_at = event.event.observed_at,
        entities = event.event.entities,
    )
}

pub fn render_deferred_intent_callback_prompt(work_item: &AssistantWorkItem) -> String {
    let context = &work_item.context;
    let trigger_type = context
        .get("trigger_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let trigger_at = context
        .get("trigger_at_unix")
        .and_then(|v| v.as_i64())
        .map(|ts| ts.to_string())
        .unwrap_or_else(|| "(none)".to_string());
    let event_key = context
        .get("event_key")
        .and_then(|v| v.as_str())
        .unwrap_or("(none)");
    let group_key = context
        .get("group_key")
        .and_then(|v| v.as_str())
        .unwrap_or("(none)");
    let fire_ordinal = context
        .get("fire_ordinal")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let recurring = context
        .get("interval_seconds")
        .and_then(|v| v.as_i64())
        .is_some();
    let scheduled_text = context
        .get("context_text")
        .and_then(|v| v.as_str())
        .unwrap_or(work_item.goal.as_str());

    format!(
        "[ASSISTANT_WORK_ITEM]\n\
         You are continuing the same assistant relationship in a bounded background action turn.\n\
         This work item fired from a previously scheduled follow-up or event-triggered callback.\n\
         This is not a fresh user message. Stay concise, grounded, and action-oriented.\n\n\
         Goal:\n{goal}\n\n\
         Budget:\n\
         - max_rounds: {max_rounds}\n\
         - max_tool_calls: {max_tool_calls}\n\
         - timeout_secs: {timeout_secs}\n\n\
         Scheduled callback context:\n\
         - scheduled_instruction: {scheduled_text}\n\
         - trigger_type: {trigger_type}\n\
         - trigger_at_unix: {trigger_at}\n\
         - event_key: {event_key}\n\
         - anchor_kind: {anchor_kind}\n\
         - anchor_ref: {anchor_ref}\n\
         - recurring: {recurring}\n\
         - fire_ordinal: {fire_ordinal}\n\
         - group_key: {group_key}\n\
         - context_json: {context_json}\n\n\
         Instructions:\n\
         - Carry out the scheduled follow-up or check-in.\n\
         - If the user should hear about the result now, call notify_user with a concise update and next best action.\n\
         - If there is nothing worth sending right now, stay silent.\n\
         - This applies even when the scheduled instruction asks you to send a report or summary: \
           if you found NO new information since the previous run, do NOT send a \"nothing new\" \
           message — complete the run silently. The user only wants to hear about actual changes.\n\
         - Use tools sparingly and stay within budget.\n\
         - Do not mention internal terms like callback, work item, heartbeat, or assistant event.",
        goal = work_item.goal,
        max_rounds = work_item.budget_profile.max_rounds,
        max_tool_calls = work_item.budget_profile.max_tool_calls,
        timeout_secs = work_item.budget_profile.timeout_secs,
        scheduled_text = scheduled_text,
        trigger_type = trigger_type,
        trigger_at = trigger_at,
        event_key = event_key,
        anchor_kind = work_item
            .anchor_kind
            .map(|v| v.as_str())
            .unwrap_or("(none)"),
        anchor_ref = work_item.anchor_ref.as_deref().unwrap_or("(none)"),
        recurring = recurring,
        fire_ordinal = fire_ordinal,
        group_key = group_key,
        context_json = context,
    )
}

pub fn render_heartbeat_follow_up_prompt(work_item: &AssistantWorkItem) -> String {
    let follow_up_kind = work_item
        .context
        .get("heartbeat_follow_up_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let reason = work_item
        .context
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    format!(
        "[ASSISTANT_WORK_ITEM]\n\
         You are continuing the same assistant relationship in a bounded heartbeat follow-up turn.\n\
         This work item was synthesized from the assistant's strategic review of ongoing work.\n\
         Do not treat it as a fresh raw event.\n\n\
         Goal:\n{goal}\n\n\
         Budget:\n\
         - max_rounds: {max_rounds}\n\
         - max_tool_calls: {max_tool_calls}\n\
         - timeout_secs: {timeout_secs}\n\n\
         Heartbeat follow-up:\n\
         - kind: {follow_up_kind}\n\
         - reason: {reason}\n\
         - anchor_kind: {anchor_kind}\n\
         - anchor_ref: {anchor_ref}\n\n\
         Instructions:\n\
         - Decide whether the user should hear about this now, or whether it should stay silent.\n\
         - If the user should hear about it now, call notify_user with one concise, context-rich update and the next best action.\n\
         - Do not mention internal terms like heartbeat, work item, callback, or assistant event.\n\
         - Stay grounded in the existing work graph rather than reprocessing raw events from scratch.",
        goal = work_item.goal,
        max_rounds = work_item.budget_profile.max_rounds,
        max_tool_calls = work_item.budget_profile.max_tool_calls,
        timeout_secs = work_item.budget_profile.timeout_secs,
        follow_up_kind = follow_up_kind,
        reason = reason,
        anchor_kind = work_item
            .anchor_kind
            .map(|v| v.as_str())
            .unwrap_or("(none)"),
        anchor_ref = work_item.anchor_ref.as_deref().unwrap_or("(none)"),
    )
}

pub fn render_recent_assistant_work_items_block(
    items: &[StoredAssistantWorkItem],
    limit: usize,
) -> Option<String> {
    if items.is_empty() {
        return None;
    }

    let mut block = String::from(
        "\n\n<active_assistant_work>\n\
         Ongoing assistant work already in flight or waiting on the user:\n",
    );

    for item in items.iter().take(limit) {
        block.push_str(&format!(
            "- [{} {} {}] {} | anchor={}\n",
            item.item.status.as_str(),
            item.item.source_kind.as_str(),
            item.item.mode.as_str(),
            item.item.goal,
            item.item.anchor_ref.as_deref().unwrap_or("(none)")
        ));
    }

    block.push_str("</active_assistant_work>");
    Some(block)
}

pub fn render_heartbeat_assistant_work_summary_block(
    items: &[StoredAssistantWorkItem],
    now: chrono::DateTime<chrono::Utc>,
    stale_after: chrono::Duration,
    limit: usize,
) -> Option<String> {
    if items.is_empty() {
        return None;
    }

    let mut waiting_user = Vec::new();
    let mut stale_queued = Vec::new();
    let mut other_active = Vec::new();

    for item in items {
        match item.item.status {
            AssistantWorkStatus::WaitingUser => waiting_user.push(item),
            AssistantWorkStatus::Queued => {
                let is_stale = chrono::DateTime::parse_from_rfc3339(&item.item.updated_at)
                    .map(|dt| {
                        now.signed_duration_since(dt.with_timezone(&chrono::Utc)) >= stale_after
                    })
                    .unwrap_or(false);
                if is_stale {
                    stale_queued.push(item);
                } else {
                    other_active.push(item);
                }
            }
            AssistantWorkStatus::Running => other_active.push(item),
            AssistantWorkStatus::Done | AssistantWorkStatus::Cancelled => {}
        }
    }

    if waiting_user.is_empty() && stale_queued.is_empty() && other_active.is_empty() {
        return None;
    }

    let mut block = String::from(
        "\n\n<assistant_work_overview>\n\
         Pending assistant work. Treat this as the secretary's current work graph, not raw first-touch events.\n",
    );

    let render_grouped =
        |bucket: &[&StoredAssistantWorkItem],
         line_builder: &dyn Fn(&StoredAssistantWorkItem, usize) -> String| {
            let mut rendered = Vec::new();
            let mut seen_anchors = std::collections::BTreeSet::new();
            for item in bucket {
                let anchor = item.item.anchor_ref.as_deref().unwrap_or("(none)");
                if anchor != "(none)" && !seen_anchors.insert(anchor.to_string()) {
                    continue;
                }
                let related_count = if anchor == "(none)" {
                    1
                } else {
                    bucket
                        .iter()
                        .filter(|candidate| candidate.item.anchor_ref.as_deref() == Some(anchor))
                        .count()
                };
                rendered.push(line_builder(item, related_count));
                if rendered.len() >= limit {
                    break;
                }
            }
            rendered
        };

    if !waiting_user.is_empty() {
        block.push_str("Waiting on the user:\n");
        for line in render_grouped(&waiting_user, &|item, related_count| {
            let grouped_suffix = if related_count > 1 {
                format!(" | grouped_related_work={related_count}")
            } else {
                String::new()
            };
            format!(
                "- {} | anchor={} | source={}{}\n",
                item.item.goal,
                item.item.anchor_ref.as_deref().unwrap_or("(none)"),
                item.item.source_kind.as_str(),
                grouped_suffix,
            )
        }) {
            block.push_str(&line);
        }
    }

    if !stale_queued.is_empty() {
        block.push_str("Stale queued work:\n");
        for line in render_grouped(&stale_queued, &|item, related_count| {
            let grouped_suffix = if related_count > 1 {
                format!(" | grouped_related_work={related_count}")
            } else {
                String::new()
            };
            format!(
                "- {} | anchor={} | updated_at={}{}\n",
                item.item.goal,
                item.item.anchor_ref.as_deref().unwrap_or("(none)"),
                item.item.updated_at,
                grouped_suffix,
            )
        }) {
            block.push_str(&line);
        }
    }

    if !other_active.is_empty() {
        block.push_str("Other active work:\n");
        for line in render_grouped(&other_active, &|item, related_count| {
            let grouped_suffix = if related_count > 1 {
                format!(" | grouped_related_work={related_count}")
            } else {
                String::new()
            };
            format!(
                "- [{} {}] {} | anchor={}{}\n",
                item.item.status.as_str(),
                item.item.source_kind.as_str(),
                item.item.goal,
                item.item.anchor_ref.as_deref().unwrap_or("(none)"),
                grouped_suffix,
            )
        }) {
            block.push_str(&line);
        }
    }

    block.push_str("</assistant_work_overview>");
    Some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_review_work_item_defaults_to_bounded_callback_review() {
        let event = crate::assistant_events::StoredAssistantEvent {
            id: 42,
            event: crate::assistant_events::AssistantEvent {
                raw_event_id: Some(7),
                observed_at: "2026-04-22T12:00:00Z".to_string(),
                occurred_at: "2026-04-22T12:00:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source: crate::assistant_events::AssistantEventSource::Calendar,
                kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                account_id: Some("primary".to_string()),
                title: "Please review contract".to_string(),
                summary: "Boss asked for a contract review".to_string(),
                thread_key: Some("thread-1".to_string()),
                urgency_hint: crate::assistant_events::EventUrgencyHint::High,
                actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
                decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
                dedup_key: "calendar:primary:m-1".to_string(),
                entities: json!({ "from": "boss@example.com" }),
            },
        };

        let item = build_callback_review_work_item(&event);
        assert_eq!(item.source_kind, AssistantWorkSourceKind::AssistantEvent);
        assert_eq!(item.assistant_event_id, Some(42));
        assert_eq!(item.anchor_kind, Some(AssistantWorkAnchorKind::EmailThread));
        assert_eq!(item.anchor_ref.as_deref(), Some("thread-1"));
        assert_eq!(item.mode, AssistantWorkMode::BackgroundReview);
        assert_eq!(item.priority, AssistantWorkPriority::InterruptNow);
        assert_eq!(item.status, AssistantWorkStatus::Queued);
        assert_eq!(
            item.delivery_policy,
            AssistantWorkDeliveryPolicy::ProactiveMessage
        );
        assert_eq!(item.caller_context, AssistantWorkCallerContext::Callback);
        assert_eq!(
            item.budget_profile,
            AssistantWorkBudgetProfile::bounded_callback_review()
        );
        assert_eq!(
            item.context
                .get("assistant_event_id")
                .and_then(|v| v.as_i64()),
            Some(42)
        );
    }

    #[test]
    fn callback_review_prompt_contains_event_context_and_budget() {
        let event = crate::assistant_events::StoredAssistantEvent {
            id: 42,
            event: crate::assistant_events::AssistantEvent {
                raw_event_id: Some(7),
                observed_at: "2026-04-22T12:00:00Z".to_string(),
                occurred_at: "2026-04-22T12:00:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source: crate::assistant_events::AssistantEventSource::Calendar,
                kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                account_id: Some("primary".to_string()),
                title: "Please review contract".to_string(),
                summary: "Boss asked for a contract review".to_string(),
                thread_key: Some("thread-1".to_string()),
                urgency_hint: crate::assistant_events::EventUrgencyHint::High,
                actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
                decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
                dedup_key: "calendar:primary:m-1".to_string(),
                entities: json!({ "from": "boss@example.com" }),
            },
        };
        let item = build_callback_review_work_item(&event);

        let prompt = render_callback_review_prompt(&item, &event);
        assert!(prompt.contains("bounded background review turn"));
        assert!(prompt.contains("Please review contract"));
        assert!(prompt.contains("Boss asked for a contract review"));
        assert!(prompt.contains("\"from\":\"boss@example.com\""));
        assert!(prompt.contains("max_rounds: 3"));
        assert!(prompt.contains("anchor_kind: email_thread"));
        assert!(prompt.contains("anchor_ref: thread-1"));
        assert!(prompt.contains("call notify_user"));
    }

    #[test]
    fn deferred_intent_work_item_defaults_to_bounded_background_action() {
        let intent = crate::memory::DeferredIntent {
            id: 91,
            user_did: "did:aegis:test".to_string(),
            chat_id: 12345,
            trigger_type: "time".to_string(),
            trigger_at_unix: Some(1_776_000_000),
            event_key: None,
            execution_mode: "llm_callback".to_string(),
            context_text: "Check whether the draft was sent and nudge me if it still needs review."
                .to_string(),
            status: "pending".to_string(),
            created_at: "2026-04-23T00:00:00Z".to_string(),
            expires_at_unix: 1_776_003_600,
            fired_at: None,
            is_callback_origin: false,
            retry_count: 0,
            interval_seconds: Some(3600),
            chain_depth: 1,
            pin_on_fire: false,
            group_key: Some("followup:draft-check".to_string()),
            max_fires: Some(10),
            fire_count: 2,
            recur_weekdays: None,
            recur_window_start: None,
            recur_window_end: None,
            recur_timezone: None,
            embedding: None,
        };

        let item = build_deferred_intent_work_item(&intent);
        assert_eq!(item.source_kind, AssistantWorkSourceKind::DeferredIntent);
        assert_eq!(
            item.anchor_kind,
            Some(AssistantWorkAnchorKind::DeferredIntentGroup)
        );
        assert_eq!(item.anchor_ref.as_deref(), Some("followup:draft-check"));
        assert_eq!(item.mode, AssistantWorkMode::BackgroundAction);
        assert_eq!(item.priority, AssistantWorkPriority::Soon);
        assert_eq!(item.status, AssistantWorkStatus::Queued);
        assert_eq!(item.caller_context, AssistantWorkCallerContext::Callback);
        assert_eq!(
            item.budget_profile,
            AssistantWorkBudgetProfile::bounded_deferred_intent_action()
        );
        assert_eq!(
            item.context.get("intent_id").and_then(|v| v.as_i64()),
            Some(91)
        );
        assert_eq!(
            item.context.get("fire_ordinal").and_then(|v| v.as_i64()),
            Some(3)
        );
        assert!(
            item.dedup_key
                .contains("assistant_work:deferred_intent:91:3:time:1776000000")
        );
    }

    #[test]
    fn deferred_intent_prompt_contains_scheduled_context_and_budget() {
        let intent = crate::memory::DeferredIntent {
            id: 92,
            user_did: "did:aegis:test".to_string(),
            chat_id: 12345,
            trigger_type: "event".to_string(),
            trigger_at_unix: None,
            event_key: Some("email:new:primary".to_string()),
            execution_mode: "llm_callback".to_string(),
            context_text:
                "When new mail arrives, summarize the open risks and tell me what to do next."
                    .to_string(),
            status: "pending".to_string(),
            created_at: "2026-04-23T00:00:00Z".to_string(),
            expires_at_unix: 1_776_003_600,
            fired_at: None,
            is_callback_origin: false,
            retry_count: 0,
            interval_seconds: None,
            chain_depth: 0,
            pin_on_fire: false,
            group_key: Some("email:new:primary:triage".to_string()),
            max_fires: None,
            fire_count: 0,
            recur_weekdays: None,
            recur_window_start: None,
            recur_window_end: None,
            recur_timezone: None,
            embedding: None,
        };

        let item = build_deferred_intent_work_item(&intent);
        let prompt = render_deferred_intent_callback_prompt(&item);
        assert!(prompt.contains("bounded background action turn"));
        assert!(prompt.contains("scheduled follow-up or event-triggered callback"));
        assert!(prompt.contains("email:new:primary"));
        assert!(prompt.contains("anchor_kind: deferred_intent_group"));
        assert!(prompt.contains("anchor_ref: email:new:primary:triage"));
        assert!(prompt.contains("fire_ordinal: 1"));
        assert!(prompt.contains("max_tool_calls: 5"));
        assert!(prompt.contains("call notify_user"));
    }

    #[test]
    fn render_recent_assistant_work_items_block_lists_anchor_and_status() {
        let event = crate::assistant_events::StoredAssistantEvent {
            id: 42,
            event: crate::assistant_events::AssistantEvent {
                raw_event_id: Some(7),
                observed_at: "2026-04-22T12:00:00Z".to_string(),
                occurred_at: "2026-04-22T12:00:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source: crate::assistant_events::AssistantEventSource::Calendar,
                kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                account_id: Some("primary".to_string()),
                title: "Please review contract".to_string(),
                summary: "Boss asked for a contract review".to_string(),
                thread_key: Some("thread-1".to_string()),
                urgency_hint: crate::assistant_events::EventUrgencyHint::High,
                actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
                decision: crate::assistant_events::AssistantEventDecision::CallbackNow,
                dedup_key: "calendar:primary:m-1".to_string(),
                entities: json!({ "from": "boss@example.com" }),
            },
        };
        let item = build_callback_review_work_item(&event);
        let stored = StoredAssistantWorkItem { id: 9, item };

        let block = render_recent_assistant_work_items_block(&[stored], 5).unwrap();
        assert!(block.contains("<active_assistant_work>"));
        assert!(block.contains("[queued assistant_event background_review]"));
        assert!(block.contains("anchor=thread-1"));
        assert!(block.contains("contract review"));
    }

    #[test]
    fn render_heartbeat_assistant_work_summary_groups_waiting_and_stale() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-23T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let waiting = StoredAssistantWorkItem {
            id: 1,
            item: AssistantWorkItem {
                created_at: "2026-04-23T09:00:00Z".to_string(),
                updated_at: "2026-04-23T09:30:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::DeferredIntent,
                source_ref: Some("deferred_intent:1".to_string()),
                anchor_kind: Some(AssistantWorkAnchorKind::DeferredIntentGroup),
                anchor_ref: Some("followup:contract".to_string()),
                assistant_event_id: None,
                goal: "Confirm whether to send the contract follow-up".to_string(),
                mode: AssistantWorkMode::BackgroundAction,
                priority: AssistantWorkPriority::Soon,
                status: AssistantWorkStatus::WaitingUser,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Callback,
                dedup_key: "waiting:1".to_string(),
                budget_profile: AssistantWorkBudgetProfile::bounded_deferred_intent_action(),
                context: json!({}),
                result: None,
            },
        };
        let stale = StoredAssistantWorkItem {
            id: 2,
            item: AssistantWorkItem {
                created_at: "2026-04-23T07:00:00Z".to_string(),
                updated_at: "2026-04-23T07:30:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::AssistantEvent,
                source_ref: Some("assistant_event:2".to_string()),
                anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                anchor_ref: Some("thread-9".to_string()),
                assistant_event_id: Some(2),
                goal: "Review the updated legal thread".to_string(),
                mode: AssistantWorkMode::BackgroundReview,
                priority: AssistantWorkPriority::InterruptNow,
                status: AssistantWorkStatus::Queued,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Callback,
                dedup_key: "stale:2".to_string(),
                budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                context: json!({}),
                result: None,
            },
        };
        let running = StoredAssistantWorkItem {
            id: 3,
            item: AssistantWorkItem {
                created_at: "2026-04-23T11:40:00Z".to_string(),
                updated_at: "2026-04-23T11:55:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::AssistantEvent,
                source_ref: Some("assistant_event:3".to_string()),
                anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                anchor_ref: Some("thread-10".to_string()),
                assistant_event_id: Some(3),
                goal: "Summarize the latest pricing thread".to_string(),
                mode: AssistantWorkMode::BackgroundReview,
                priority: AssistantWorkPriority::Soon,
                status: AssistantWorkStatus::Running,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Callback,
                dedup_key: "running:3".to_string(),
                budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                context: json!({}),
                result: None,
            },
        };

        let block = render_heartbeat_assistant_work_summary_block(
            &[waiting, stale, running],
            now,
            chrono::Duration::hours(2),
            5,
        )
        .unwrap();
        assert!(block.contains("<assistant_work_overview>"));
        assert!(block.contains("Waiting on the user:"));
        assert!(block.contains("Stale queued work:"));
        assert!(block.contains("Other active work:"));
        assert!(block.contains("followup:contract"));
        assert!(block.contains("thread-9"));
        assert!(block.contains("thread-10"));
    }

    #[test]
    fn render_heartbeat_assistant_work_summary_groups_same_anchor_items() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-23T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let waiting_primary = StoredAssistantWorkItem {
            id: 1,
            item: AssistantWorkItem {
                created_at: "2026-04-23T09:00:00Z".to_string(),
                updated_at: "2026-04-23T09:30:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::DeferredIntent,
                source_ref: Some("deferred_intent:1".to_string()),
                anchor_kind: Some(AssistantWorkAnchorKind::DeferredIntentGroup),
                anchor_ref: Some("followup:contract".to_string()),
                assistant_event_id: None,
                goal: "Confirm whether to send the contract follow-up".to_string(),
                mode: AssistantWorkMode::BackgroundAction,
                priority: AssistantWorkPriority::Soon,
                status: AssistantWorkStatus::WaitingUser,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Callback,
                dedup_key: "waiting:1".to_string(),
                budget_profile: AssistantWorkBudgetProfile::bounded_deferred_intent_action(),
                context: json!({}),
                result: None,
            },
        };
        let waiting_duplicate = StoredAssistantWorkItem {
            id: 2,
            item: AssistantWorkItem {
                created_at: "2026-04-23T09:10:00Z".to_string(),
                updated_at: "2026-04-23T09:40:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::HeartbeatFollowUp,
                source_ref: Some("heartbeat_followup:escalation".to_string()),
                anchor_kind: Some(AssistantWorkAnchorKind::DeferredIntentGroup),
                anchor_ref: Some("followup:contract".to_string()),
                assistant_event_id: None,
                goal: "Escalate the same contract follow-up after it sat too long".to_string(),
                mode: AssistantWorkMode::BackgroundReview,
                priority: AssistantWorkPriority::Soon,
                status: AssistantWorkStatus::WaitingUser,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Heartbeat,
                dedup_key: "waiting:2".to_string(),
                budget_profile: AssistantWorkBudgetProfile::heartbeat_synthesis(),
                context: json!({}),
                result: None,
            },
        };

        let block = render_heartbeat_assistant_work_summary_block(
            &[waiting_primary, waiting_duplicate],
            now,
            chrono::Duration::hours(2),
            5,
        )
        .unwrap();
        assert!(block.contains("grouped_related_work=2"));
        assert!(
            !block.contains("Escalate the same contract follow-up after it sat too long"),
            "same-anchor waiting work should be grouped into one line"
        );
    }

    #[test]
    fn heartbeat_escalation_work_item_defaults_to_heartbeat_review() {
        let item = build_heartbeat_escalation_work_item(
            "did:aegis:test",
            "Escalate the contract follow-up because it has been waiting on the user for too long.",
            "heartbeat:escalate:followup:contract",
            Some(AssistantWorkAnchorKind::DeferredIntentGroup),
            Some("followup:contract".to_string()),
            "waiting_user_overdue",
        );

        assert_eq!(item.source_kind, AssistantWorkSourceKind::HeartbeatFollowUp);
        assert_eq!(item.mode, AssistantWorkMode::BackgroundReview);
        assert_eq!(item.caller_context, AssistantWorkCallerContext::Heartbeat);
        assert_eq!(
            item.delivery_policy,
            AssistantWorkDeliveryPolicy::ProactiveMessage
        );
        assert_eq!(
            item.context
                .get("heartbeat_follow_up_kind")
                .and_then(|v| v.as_str()),
            Some("escalation")
        );
        assert_eq!(item.anchor_ref.as_deref(), Some("followup:contract"));
    }

    #[test]
    fn heartbeat_digest_work_item_defaults_to_digest_assembly() {
        let item = build_heartbeat_digest_work_item(
            "did:aegis:test",
            "Assemble a digest covering the accumulated low-signal mail backlog.",
            "heartbeat:digest:mail-backlog",
            "digest_batch_matured",
            "mail_notifier_digest",
            41,
            43,
            3,
        );

        assert_eq!(item.source_kind, AssistantWorkSourceKind::HeartbeatFollowUp);
        assert_eq!(item.mode, AssistantWorkMode::DigestAssembly);
        assert_eq!(item.caller_context, AssistantWorkCallerContext::Heartbeat);
        assert_eq!(
            item.delivery_policy,
            AssistantWorkDeliveryPolicy::DigestOnly
        );
        assert_eq!(
            item.context
                .get("heartbeat_follow_up_kind")
                .and_then(|v| v.as_str()),
            Some("digest_assembly")
        );
        assert_eq!(
            item.context
                .get("assistant_event_consumer")
                .and_then(|v| v.as_str()),
            Some("mail_notifier_digest")
        );
        assert_eq!(
            item.context.get("last_event_id").and_then(|v| v.as_i64()),
            Some(43)
        );
    }

    #[test]
    fn heartbeat_follow_up_claim_filter_matches_only_heartbeat_review_work() {
        let filter = AssistantWorkClaimFilter::heartbeat_follow_up();
        assert_eq!(filter.statuses, vec!["queued".to_string()]);
        assert_eq!(filter.source_kinds, vec!["heartbeat_followup".to_string()]);
        assert_eq!(filter.modes, vec!["background_review".to_string()]);
        assert_eq!(filter.caller_contexts, vec!["heartbeat".to_string()]);
    }

    #[test]
    fn heartbeat_digest_claim_filter_matches_only_digest_work() {
        let filter = AssistantWorkClaimFilter::heartbeat_digest_assembly();
        assert_eq!(filter.statuses, vec!["queued".to_string()]);
        assert_eq!(filter.source_kinds, vec!["heartbeat_followup".to_string()]);
        assert_eq!(filter.modes, vec!["digest_assembly".to_string()]);
        assert_eq!(filter.caller_contexts, vec!["heartbeat".to_string()]);
    }

    #[test]
    fn heartbeat_follow_up_prompt_contains_anchor_and_reason() {
        let item = build_heartbeat_escalation_work_item(
            "did:aegis:test",
            "Escalate the contract follow-up because it has been waiting on the user for too long.",
            "heartbeat:escalate:followup:contract",
            Some(AssistantWorkAnchorKind::DeferredIntentGroup),
            Some("followup:contract".to_string()),
            "waiting_user_overdue",
        );
        let prompt = render_heartbeat_follow_up_prompt(&item);
        assert!(prompt.contains("bounded heartbeat follow-up turn"));
        assert!(prompt.contains("kind: escalation"));
        assert!(prompt.contains("reason: waiting_user_overdue"));
        assert!(prompt.contains("anchor_kind: deferred_intent_group"));
        assert!(prompt.contains("anchor_ref: followup:contract"));
        assert!(prompt.contains("notify_user"));
    }

    #[test]
    fn derive_heartbeat_escalation_work_items_only_converts_waiting_user_overdue() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-23T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let rows = vec![
            StoredAssistantWorkItem {
                id: 1,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T08:00:00Z".to_string(),
                    updated_at: "2026-04-23T08:30:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::DeferredIntent,
                    source_ref: Some("deferred_intent:1".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::DeferredIntentGroup),
                    anchor_ref: Some("followup:contract".to_string()),
                    assistant_event_id: None,
                    goal: "Confirm whether to send the contract follow-up".to_string(),
                    mode: AssistantWorkMode::BackgroundAction,
                    priority: AssistantWorkPriority::Soon,
                    status: AssistantWorkStatus::WaitingUser,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "waiting:1".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_deferred_intent_action(),
                    context: json!({}),
                    result: None,
                },
            },
            StoredAssistantWorkItem {
                id: 2,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T08:00:00Z".to_string(),
                    updated_at: "2026-04-23T08:20:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::AssistantEvent,
                    source_ref: Some("assistant_event:2".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                    anchor_ref: Some("thread-9".to_string()),
                    assistant_event_id: Some(2),
                    goal: "Review the updated legal thread".to_string(),
                    mode: AssistantWorkMode::BackgroundReview,
                    priority: AssistantWorkPriority::InterruptNow,
                    status: AssistantWorkStatus::Queued,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "stale:2".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                    context: json!({}),
                    result: None,
                },
            },
            StoredAssistantWorkItem {
                id: 3,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T11:30:00Z".to_string(),
                    updated_at: "2026-04-23T11:50:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::AssistantEvent,
                    source_ref: Some("assistant_event:3".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                    anchor_ref: Some("thread-fresh".to_string()),
                    assistant_event_id: Some(3),
                    goal: "Fresh review work".to_string(),
                    mode: AssistantWorkMode::BackgroundReview,
                    priority: AssistantWorkPriority::Soon,
                    status: AssistantWorkStatus::Queued,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "fresh:3".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                    context: json!({}),
                    result: None,
                },
            },
        ];

        let derived = derive_heartbeat_escalation_work_items(
            "did:aegis:test",
            &rows,
            now,
            chrono::Duration::hours(2),
        );

        assert_eq!(derived.len(), 1);
        assert_eq!(
            derived[0].source_kind,
            AssistantWorkSourceKind::HeartbeatFollowUp
        );
        assert_eq!(
            derived[0]
                .context
                .get("heartbeat_follow_up_kind")
                .and_then(|v| v.as_str()),
            Some("escalation")
        );
        assert!(derived[0].dedup_key.contains("waiting_user_overdue"));
    }

    #[test]
    fn derive_heartbeat_silent_reprioritization_work_items_converts_stale_queued_work() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-23T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let rows = vec![
            StoredAssistantWorkItem {
                id: 1,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T08:00:00Z".to_string(),
                    updated_at: "2026-04-23T08:30:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::DeferredIntent,
                    source_ref: Some("deferred_intent:1".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::DeferredIntentGroup),
                    anchor_ref: Some("followup:contract".to_string()),
                    assistant_event_id: None,
                    goal: "Confirm whether to send the contract follow-up".to_string(),
                    mode: AssistantWorkMode::BackgroundAction,
                    priority: AssistantWorkPriority::Soon,
                    status: AssistantWorkStatus::WaitingUser,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "waiting:1".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_deferred_intent_action(),
                    context: json!({}),
                    result: None,
                },
            },
            StoredAssistantWorkItem {
                id: 2,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T08:00:00Z".to_string(),
                    updated_at: "2026-04-23T08:20:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::AssistantEvent,
                    source_ref: Some("assistant_event:2".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                    anchor_ref: Some("thread-9".to_string()),
                    assistant_event_id: Some(2),
                    goal: "Review the updated legal thread".to_string(),
                    mode: AssistantWorkMode::BackgroundReview,
                    priority: AssistantWorkPriority::InterruptNow,
                    status: AssistantWorkStatus::Queued,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "stale:2".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                    context: json!({}),
                    result: None,
                },
            },
            StoredAssistantWorkItem {
                id: 3,
                item: AssistantWorkItem {
                    created_at: "2026-04-23T11:30:00Z".to_string(),
                    updated_at: "2026-04-23T11:50:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source_kind: AssistantWorkSourceKind::AssistantEvent,
                    source_ref: Some("assistant_event:3".to_string()),
                    anchor_kind: Some(AssistantWorkAnchorKind::EmailThread),
                    anchor_ref: Some("thread-fresh".to_string()),
                    assistant_event_id: Some(3),
                    goal: "Fresh review work".to_string(),
                    mode: AssistantWorkMode::BackgroundReview,
                    priority: AssistantWorkPriority::Soon,
                    status: AssistantWorkStatus::Queued,
                    delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                    caller_context: AssistantWorkCallerContext::Callback,
                    dedup_key: "fresh:3".to_string(),
                    budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                    context: json!({}),
                    result: None,
                },
            },
        ];

        let derived = derive_heartbeat_silent_reprioritization_work_items(
            "did:aegis:test",
            &rows,
            now,
            chrono::Duration::hours(2),
        );

        assert_eq!(derived.len(), 1);
        assert_eq!(
            derived[0].source_kind,
            AssistantWorkSourceKind::HeartbeatFollowUp
        );
        assert_eq!(
            derived[0]
                .context
                .get("heartbeat_follow_up_kind")
                .and_then(|v| v.as_str()),
            Some("silent_reprioritization")
        );
        assert_eq!(
            derived[0].delivery_policy,
            AssistantWorkDeliveryPolicy::Silent
        );
        assert!(derived[0].dedup_key.contains("queued_work_stale"));
    }

    #[test]
    fn derive_heartbeat_digest_work_item_batches_lower_signal_mail() {
        let active_items = vec![StoredAssistantWorkItem {
            id: 1,
            item: AssistantWorkItem {
                created_at: "2026-04-23T11:00:00Z".to_string(),
                updated_at: "2026-04-23T11:30:00Z".to_string(),
                user_did: "did:aegis:test".to_string(),
                source_kind: AssistantWorkSourceKind::AssistantEvent,
                source_ref: Some("assistant_event:1".to_string()),
                anchor_kind: None,
                anchor_ref: None,
                assistant_event_id: Some(1),
                goal: "Review billing notice".to_string(),
                mode: AssistantWorkMode::BackgroundReview,
                priority: AssistantWorkPriority::Soon,
                status: AssistantWorkStatus::Queued,
                delivery_policy: AssistantWorkDeliveryPolicy::ProactiveMessage,
                caller_context: AssistantWorkCallerContext::Callback,
                dedup_key: "assistant_work:callback:test".to_string(),
                budget_profile: AssistantWorkBudgetProfile::bounded_callback_review(),
                context: json!({}),
                result: None,
            },
        }];
        let digest_events = vec![
            crate::assistant_events::StoredAssistantEvent {
                id: 41,
                event: crate::assistant_events::AssistantEvent {
                    raw_event_id: None,
                    observed_at: "2026-04-23T09:00:00Z".to_string(),
                    occurred_at: "2026-04-23T09:00:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source: crate::assistant_events::AssistantEventSource::Calendar,
                    kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                    account_id: Some("primary".to_string()),
                    title: "Weekly digest".to_string(),
                    summary: "newsletter".to_string(),
                    thread_key: Some("t-41".to_string()),
                    urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
                    actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
                    decision: crate::assistant_events::AssistantEventDecision::DigestLater,
                    dedup_key: "calendar:primary:41".to_string(),
                    entities: json!({}),
                },
            },
            crate::assistant_events::StoredAssistantEvent {
                id: 42,
                event: crate::assistant_events::AssistantEvent {
                    raw_event_id: None,
                    observed_at: "2026-04-23T09:05:00Z".to_string(),
                    occurred_at: "2026-04-23T09:05:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source: crate::assistant_events::AssistantEventSource::Calendar,
                    kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                    account_id: Some("primary".to_string()),
                    title: "Promotions".to_string(),
                    summary: "newsletter".to_string(),
                    thread_key: Some("t-42".to_string()),
                    urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
                    actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
                    decision: crate::assistant_events::AssistantEventDecision::DigestLater,
                    dedup_key: "calendar:primary:42".to_string(),
                    entities: json!({}),
                },
            },
            crate::assistant_events::StoredAssistantEvent {
                id: 43,
                event: crate::assistant_events::AssistantEvent {
                    raw_event_id: None,
                    observed_at: "2026-04-23T09:10:00Z".to_string(),
                    occurred_at: "2026-04-23T09:10:00Z".to_string(),
                    user_did: "did:aegis:test".to_string(),
                    source: crate::assistant_events::AssistantEventSource::Calendar,
                    kind: crate::assistant_events::AssistantEventKind::EmailReceived,
                    account_id: Some("primary".to_string()),
                    title: "Another digest".to_string(),
                    summary: "newsletter".to_string(),
                    thread_key: Some("t-43".to_string()),
                    urgency_hint: crate::assistant_events::EventUrgencyHint::Low,
                    actionability_hint: crate::assistant_events::EventActionabilityHint::Passive,
                    decision: crate::assistant_events::AssistantEventDecision::DigestLater,
                    dedup_key: "calendar:primary:43".to_string(),
                    entities: json!({}),
                },
            },
        ];

        let derived = derive_heartbeat_digest_work_item(
            "did:aegis:test",
            &active_items,
            &digest_events,
            3,
            300,
        )
        .expect("digest batch should become work");

        assert_eq!(
            derived.source_kind,
            AssistantWorkSourceKind::HeartbeatFollowUp
        );
        assert_eq!(derived.mode, AssistantWorkMode::DigestAssembly);
        assert_eq!(
            derived.delivery_policy,
            AssistantWorkDeliveryPolicy::DigestOnly
        );
        assert_eq!(
            derived
                .context
                .get("first_event_id")
                .and_then(|v| v.as_i64()),
            Some(41)
        );
        assert_eq!(
            derived
                .context
                .get("last_event_id")
                .and_then(|v| v.as_i64()),
            Some(43)
        );
    }
}
