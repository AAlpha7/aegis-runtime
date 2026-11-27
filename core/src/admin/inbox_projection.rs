use super::*;
use crate::i18n::{t, tf, tfp};

fn calendar_source_status(state: &AdminState) -> UnifiedInboxSourceStatus {
    match state.orchestrator.as_ref() {
        Some(orch) => {
            let configured = orch
                .google_auth_status()
                .get("configured")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if configured {
                UnifiedInboxSourceStatus {
                    source: "calendar".into(),
                    status: "available".into(),
                    reason: None,
                }
            } else {
                UnifiedInboxSourceStatus {
                    source: "calendar".into(),
                    status: "delegated".into(),
                    reason: Some("codex_connector".into()),
                }
            }
        }
        None => UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "available".into(),
            reason: None,
        },
    }
}

pub(super) async fn build_unified_inbox_projection(
    state: &AdminState,
    chat_id: i64,
    locale: &str,
) -> UnifiedInboxProjection {
    let generated_at_dt = chrono::Utc::now();
    let generated_at = generated_at_dt.to_rfc3339();
    let owner = format!("did:aegis:telegram:{chat_id}");
    let mut sources = Vec::new();
    let mut items = Vec::new();
    let mut assistant_events = Vec::new();

    match state
        .memory
        .list_recent_assistant_events(&owner, WEBAPP_WORK_PROJECTION_EVENT_SCAN_WINDOW)
        .await
    {
        Ok(events) => {
            sources.push(calendar_source_status(state));
            // Cap PROJECTED items, not scanned events, so a burst of
            // non-calendar events cannot evict calendar items from the queue.
            for item in events
                .iter()
                .filter_map(|event| unified_inbox_active_item_from_assistant_event(event, locale))
                .take(WEBAPP_WORK_PROJECTION_EVENT_ITEM_CAP)
            {
                items.push(item);
            }
            assistant_events = events;
        }
        Err(err) => {
            tracing::warn!(error = %err, "Unable to project assistant events into WebApp inbox");
            sources.push(UnifiedInboxSourceStatus {
                source: "calendar".into(),
                status: "unavailable".into(),
                reason: Some("assistant event query failed".into()),
            });
        }
    }

    // Mandate matching keeps the historical 100-event window even though the
    // projection scan above is deeper (see WEBAPP_MANDATE_MATCH_EVENT_WINDOW).
    let mandate_event_window = &assistant_events[..assistant_events
        .len()
        .min(WEBAPP_MANDATE_MATCH_EVENT_WINDOW)];
    append_mandate_match_items(
        state,
        &owner,
        mandate_event_window,
        &generated_at,
        locale,
        &mut sources,
        &mut items,
    );

    let approvals = state.memory.list_hitl_pending().await.unwrap_or_default();
    sources.push(UnifiedInboxSourceStatus {
        source: "hitl".into(),
        status: "available".into(),
        reason: None,
    });
    let owner_approvals: Vec<(String, String)> = approvals
        .iter()
        .filter(|(_, value)| webapp_hitl_approval_belongs_to_owner(value, chat_id, &owner))
        .map(|(approval_id, value)| (approval_id.clone(), value.clone()))
        .collect();
    let task_approval_requires_mfa: HashMap<i64, bool> = owner_approvals
        .iter()
        .filter_map(|(approval_id, value)| {
            let task_id = hitl_approval_source_task_id(value)?;
            let approval_type = trust_console_approval_type(approval_id, value);
            let risk = trust_console_approval_risk(&approval_type);
            Some((task_id, trust_console_approval_requires_mfa(risk)))
        })
        .collect();
    let owner_tasks = list_open_owner_tasks(state, chat_id).await;
    let open_owner_task_ids: HashSet<i64> = owner_tasks.iter().map(|task| task.id).collect();
    for (approval_id, value) in owner_approvals
        .iter()
        .filter(|(_, value)| {
            // Tool holds are one card on the 安琪 thread. Leaving them here
            // painted every row as the same generic "工具请求 / R2 Medium" card.
            !chat_thread_tool_hold(value)
                && hitl_approval_source_task_id(value)
                    .is_none_or(|task_id| !open_owner_task_ids.contains(&task_id))
        })
        .take(50)
    {
        items.push(unified_inbox_item_from_approval(
            approval_id,
            value,
            &generated_at,
            locale,
        ));
    }

    sources.push(UnifiedInboxSourceStatus {
        source: "tasks".into(),
        status: "available".into(),
        reason: None,
    });
    for task in &owner_tasks {
        if !task_should_surface_to_owner(task) {
            continue;
        }
        let mut item = unified_inbox_item_from_task(task, locale);
        if task_exposes_trigger_conversation(task) {
            attach_trigger_conversation(
                state,
                &task.user_did,
                &task.created_at,
                Some(task.id),
                &mut item.lineage,
                locale,
            )
            .await;
        }
        if task_is_blocked_owner_automation(task)
            && let Some(requires_mfa) = task_approval_requires_mfa.get(&task.id)
        {
            item.secondary_actions = approval_decision_actions(*requires_mfa, locale);
            item.next_safe_action = t("console.blocked.task.next", locale).to_string();
        }
        items.push(item);
    }

    append_research_job_items(
        &state.memory,
        &owner,
        &generated_at,
        locale,
        &mut sources,
        &mut items,
    )
    .await;

    if state
        .orchestrator
        .as_ref()
        .is_some_and(|orchestrator| orchestrator.hercules_configured())
    {
        append_hercules_delegation_items(&state.memory, locale, &mut sources, &mut items).await;
    }

    match state
        .memory
        .list_goals(&owner, Some(crate::memory::GoalStatus::Active))
        .await
    {
        Ok(goals) => {
            sources.push(UnifiedInboxSourceStatus {
                source: "goals".into(),
                status: "available".into(),
                reason: None,
            });
            // Inferred goals with no progress and no milestones are raw fact
            // text ("user wants to monitor …", auto-created 2026-04-12) that
            // the owner never asked to track; they drowned the Work lane.
            for goal in goals
                .iter()
                .filter(|goal| !goal_is_unclaimed_inference(goal))
                .take(25)
            {
                let mut item = unified_inbox_item_from_goal(goal, locale);
                attach_trigger_conversation(
                    state,
                    &goal.user_did,
                    &goal.created_at,
                    None,
                    &mut item.lineage,
                    locale,
                )
                .await;
                items.push(item);
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "Unable to project active goals into WebApp inbox");
            sources.push(UnifiedInboxSourceStatus {
                source: "goals".into(),
                status: "unavailable".into(),
                reason: Some("goal query failed".into()),
            });
        }
    }

    let waiting_items = state
        .memory
        .query_knowledge_entries(chat_id, Some("waiting_for"), Some("active"), None, 25)
        .await;
    sources.push(UnifiedInboxSourceStatus {
        source: "knowledge".into(),
        status: "available".into(),
        reason: None,
    });
    for entry in waiting_items {
        if !waiting_knowledge_should_surface_to_owner(&entry) {
            continue;
        }
        let mut item = unified_inbox_item_from_waiting_knowledge(&entry, locale);
        let mut generator = waiting_generator_from_tasks(&entry, &owner_tasks, &owner).cloned();
        if generator.is_none()
            && let crate::memory::WaitingParentTag::Task(task_id) =
                crate::memory::waiting_parent_tag(&entry.tags)
            && !owner_tasks.iter().any(|task| task.id == task_id)
            && let Some(task) = state.memory.get_task(task_id).await
            && task_is_owned_waiting_generator(&task, &owner, chat_id)
        {
            generator = Some(task);
        }
        if let Some(task) = generator.as_ref() {
            attach_resolved_waiting_generator(&mut item, &entry, task, locale);
        }
        attach_trigger_conversation(
            state,
            &owner,
            &entry.created_at,
            None,
            &mut item.lineage,
            locale,
        )
        .await;
        items.push(item);
    }

    match state
        .memory
        .query_stale_facts(&owner, WEBAPP_MEMORY_REVIEW_OWNER_SURFACE_DAYS)
        .await
    {
        Ok(facts) => {
            sources.push(UnifiedInboxSourceStatus {
                source: "memory".into(),
                status: "available".into(),
                reason: None,
            });
            for fact in facts
                .iter()
                .filter(|fact| memory_review_should_surface_to_owner(fact))
                .take(WEBAPP_MEMORY_REVIEW_OWNER_SURFACE_LIMIT)
            {
                items.push(unified_inbox_item_from_memory_review(fact, locale));
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "Unable to project memory review queue into WebApp inbox");
            sources.push(UnifiedInboxSourceStatus {
                source: "memory".into(),
                status: "unavailable".into(),
                reason: Some("memory review query failed".into()),
            });
        }
    }

    if state.memory.db_path() == ":memory:" {
        sources.push(UnifiedInboxSourceStatus {
            source: "action_ledger".into(),
            status: "unavailable".into(),
            reason: Some("persistent database is not configured".into()),
        });
        items.push(unified_inbox_system_item(
            "action_ledger_unavailable",
            t("console.system.title.unavailable", locale),
            t("console.system.summary.no_db", locale),
            &generated_at,
            locale,
        ));
    } else {
        match crate::encrypted_sqlite::open(state.memory.db_path())
            .map_err(anyhow::Error::from)
            .and_then(|conn| crate::action_ledger::ActionLedgerStore::new(&conn).list_recent(50))
        {
            Ok(entries) => {
                sources.push(UnifiedInboxSourceStatus {
                    source: "action_ledger".into(),
                    status: "available".into(),
                    reason: None,
                });
                let owner_entries: Vec<crate::action_ledger::ActionLedgerEntry> = entries
                    .iter()
                    .filter(|entry| {
                        webapp_action_ledger_entry_belongs_to_owner(entry, chat_id, &owner)
                            && vnext_should_project_action_ledger_entry(entry)
                    })
                    .take(50)
                    .cloned()
                    .collect();
                items.extend(project_ledger_entries_into_inbox(&owner_entries, locale));
            }
            Err(err) => {
                tracing::warn!(error = %err, "Unable to project Action Ledger into WebApp inbox");
                sources.push(UnifiedInboxSourceStatus {
                    source: "action_ledger".into(),
                    status: "unavailable".into(),
                    reason: Some("ledger query failed".into()),
                });
                items.push(unified_inbox_system_item(
                    "action_ledger_query_failed",
                    t("console.system.title.unavailable", locale),
                    t("console.system.summary.query_failed", locale),
                    &generated_at,
                    locale,
                ));
            }
        }
    }

    sources.push(UnifiedInboxSourceStatus {
        source: "alerts".into(),
        status: "deferred".into(),
        reason: Some("not connected yet".into()),
    });

    sort_unified_inbox_items(&mut items);
    items = denoise_work_inbox_items(items, generated_at_dt, locale);
    sort_unified_inbox_items(&mut items);
    if items.len() > 100 {
        items.truncate(100);
    }
    for item in &mut items {
        item.authority = Some(owner_work_item_authority(item));
    }
    let counts = unified_inbox_counts(&items);

    UnifiedInboxProjection {
        schema_version: 1,
        generated_at,
        owner,
        sources,
        counts,
        items,
    }
}

async fn list_open_owner_tasks(state: &AdminState, chat_id: i64) -> Vec<crate::memory::Task> {
    let mut seen = std::collections::HashSet::new();
    let mut tasks = Vec::new();
    for status in [
        "active",
        "blocked_by_gate",
        "failed",
        "dead_letter",
        "snoozed",
        "pending",
    ] {
        for task in state
            .memory
            .list_tasks(chat_id, None, Some(status), 50)
            .await
        {
            if seen.insert(task.id) {
                tasks.push(task);
            }
        }
    }
    // Same row, not a second reminder: a clock loop closed by "complete this
    // occurrence" is put back on the open list.
    for task in state
        .memory
        .list_tasks(chat_id, None, Some("done"), 200)
        .await
    {
        if !closed_clock_loop_should_reopen(&task) || !seen.insert(task.id) {
            continue;
        }
        let interval = task.interval_seconds.unwrap_or(0);
        let now_unix = chrono::Utc::now().timestamp();
        let next = next_open_clock_trigger(now_unix, task.trigger_at_unix, interval);
        let updates = serde_json::json!({
            "status": "active",
            "trigger_at_unix": next,
            "clear_fields": ["completed_at"],
        });
        if let Err(error) = state
            .memory
            .update_task(task.id, chat_id, &updates.to_string())
            .await
        {
            tracing::warn!(error = %error, task_id = task.id, "failed to reopen a closed clock loop");
        }
        let mut reopened = task;
        reopened.status = "active".to_string();
        reopened.trigger_at_unix = Some(next);
        reopened.completed_at = None;
        tasks.push(reopened);
    }
    tasks
}

/// An inferred goal the owner never claimed: raw fact text auto-created from
/// L2 (2026-04-12), progress under 5 % (goal #8 sat at 0.5 %, which `round()`
/// turned into 1) and no milestones. Explicit goals are never filtered.
pub(super) fn goal_is_unclaimed_inference(goal: &crate::memory::Goal) -> bool {
    matches!(goal.source, crate::memory::GoalSource::Inferred)
        && goal.progress_pct < 5.0
        && goal.milestones.is_empty()
}

fn task_should_surface_to_owner(task: &crate::memory::Task) -> bool {
    !task_is_ambiguous_status_rollup(task) && !task_is_system_maintenance_loop(task)
}

/// Loops the system runs for itself (a `migrated` daily "存入系统记忆（L2）"
/// callback from the old runtime) are not the owner's work; they showed up as
/// a 跟踪中 card in 工作 (2026-09-13 walk). Owner-created tasks (`source`
/// "user", webapp drafts) are never affected.
pub(super) fn task_is_system_maintenance_loop(task: &crate::memory::Task) -> bool {
    task.source == "migrated" && task.execution_mode == "llm_callback"
}

fn task_is_ambiguous_status_rollup(task: &crate::memory::Task) -> bool {
    let combined = format!(
        "{} {} {} {} {}",
        task.title,
        task.body,
        task.context_text.as_deref().unwrap_or_default(),
        task.group_key.as_deref().unwrap_or_default(),
        task.tags.join(" ")
    )
    .to_lowercase();
    let title = task.title.trim();
    if title.contains('：') && waiting_knowledge_has_multiple_topics(title) {
        return true;
    }
    combined.contains("日终状态")
        || combined.contains("evening status")
        || combined.contains("daily status")
        || combined.contains("status rollup")
        || combined.contains("status summary")
        || combined.contains("fuel exhausted")
        || combined.contains("fuelexhausted")
        || combined.contains("断路器")
        || combined.contains("circuit breaker")
}

pub(super) fn owner_work_item_authority(item: &UnifiedInboxItem) -> ContextAuthorityEnvelope {
    context_authority_envelope(&build_context_envelope_from_inbox_item(item))
}

fn append_mandate_match_items(
    state: &AdminState,
    owner: &str,
    events: &[crate::assistant_events::StoredAssistantEvent],
    generated_at: &str,
    locale: &str,
    sources: &mut Vec<UnifiedInboxSourceStatus>,
    items: &mut Vec<UnifiedInboxItem>,
) {
    if state.memory.db_path() == ":memory:" {
        sources.push(UnifiedInboxSourceStatus {
            source: "mandates".into(),
            status: "unavailable".into(),
            reason: Some("persistent database is not configured".into()),
        });
        return;
    }

    match crate::encrypted_sqlite::open(state.memory.db_path()).map_err(anyhow::Error::from) {
        Ok(conn) => {
            let mandate_store = crate::user_mandates::UserMandateStore::new(&conn);
            let mandates = match mandate_store.list_for_owner(owner, 100) {
                Ok(mandates) => mandates,
                Err(err) => {
                    tracing::warn!(error = %err, "Unable to list mandates for WebApp inbox");
                    sources.push(UnifiedInboxSourceStatus {
                        source: "mandates".into(),
                        status: "unavailable".into(),
                        reason: Some("mandate query failed".into()),
                    });
                    return;
                }
            };
            let now = chrono::Utc::now();
            let active_mandates = mandates
                .into_iter()
                .filter(|mandate| mandate_allows_metadata_matching(mandate, &now))
                .collect::<Vec<_>>();
            sources.push(UnifiedInboxSourceStatus {
                source: "mandates".into(),
                status: "available".into(),
                reason: None,
            });
            let mut seen = std::collections::HashSet::new();
            for mandate in active_mandates.iter() {
                for event in events {
                    if assistant_event_is_stale_unread(event, &now) {
                        continue;
                    }
                    if !mandate_matches_assistant_event(mandate, event) {
                        continue;
                    }
                    let dedupe_key = format!("{}:{}", mandate.mandate_id, event.id);
                    if !seen.insert(dedupe_key.clone()) {
                        continue;
                    }
                    let notification_projection =
                        state.admin_chat_id.and_then(|admin_chat_id| {
                            let plan = build_mandate_notification_plan(
                                mandate,
                                event,
                                admin_chat_id,
                                None,
                                locale,
                            );
                            let max_per_day = mandate_notification_daily_cap(mandate);
                            match mandate_store.plan_notification(&plan, max_per_day) {
                                Ok(decision) => Some(MandateNotificationProjection::from(decision)),
                                Err(err) => {
                                    tracing::warn!(
                                        error = %err,
                                        mandate = %opaque_unified_inbox_id("mandate", &mandate.mandate_id),
                                        "Unable to persist mandate notification plan"
                                    );
                                    None
                                }
                            }
                        });
                    items.push(unified_inbox_item_from_mandate_match(
                        mandate,
                        event,
                        generated_at,
                        notification_projection.as_ref(),
                        locale,
                    ));
                    if seen.len() >= 25 {
                        return;
                    }
                }
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "Unable to project mandates into WebApp inbox");
            sources.push(UnifiedInboxSourceStatus {
                source: "mandates".into(),
                status: "unavailable".into(),
                reason: Some("mandate query failed".into()),
            });
        }
    }
}

#[derive(Debug)]
struct MandateNotificationProjection {
    idempotency_key: String,
    status: &'static str,
}

impl From<crate::user_mandates::MandateNotificationDecision> for MandateNotificationProjection {
    fn from(decision: crate::user_mandates::MandateNotificationDecision) -> Self {
        match decision {
            crate::user_mandates::MandateNotificationDecision::Planned(record) => Self {
                idempotency_key: record.idempotency_key,
                status: "planned",
            },
            crate::user_mandates::MandateNotificationDecision::Duplicate(record) => Self {
                idempotency_key: record.idempotency_key,
                status: "duplicate",
            },
            crate::user_mandates::MandateNotificationDecision::DailyCapReached(record) => Self {
                idempotency_key: record.idempotency_key,
                status: "daily_cap_suppressed",
            },
        }
    }
}

fn mandate_notification_daily_cap(mandate: &crate::user_mandates::UserMandate) -> usize {
    const DEFAULT_MAX_PER_DAY: usize = 2;
    const HARD_MAX_PER_DAY: usize = 12;

    let Ok(policy) = serde_json::from_str::<serde_json::Value>(&mandate.notification_policy_json)
    else {
        return DEFAULT_MAX_PER_DAY;
    };
    let Some(raw_cap) = policy.get("max_notifications_per_day") else {
        return DEFAULT_MAX_PER_DAY;
    };
    if let Some(cap) = raw_cap.as_i64() {
        return usize::try_from(cap.max(0))
            .unwrap_or(DEFAULT_MAX_PER_DAY)
            .min(HARD_MAX_PER_DAY);
    }
    if let Some(cap) = raw_cap.as_u64() {
        return usize::try_from(cap)
            .unwrap_or(HARD_MAX_PER_DAY)
            .min(HARD_MAX_PER_DAY);
    }
    DEFAULT_MAX_PER_DAY
}

fn mandate_matches_assistant_event(
    mandate: &crate::user_mandates::UserMandate,
    event: &crate::assistant_events::StoredAssistantEvent,
) -> bool {
    let source_matches = match mandate.scope_kind.as_str() {
        "calendar_metadata" => matches!(
            event.event.source,
            crate::assistant_events::AssistantEventSource::Calendar
        ),
        _ => false,
    };
    if !source_matches {
        return false;
    }

    mandate_source_filters_match(&mandate.source_filters_json, event)
}

fn mandate_allows_metadata_matching(
    mandate: &crate::user_mandates::UserMandate,
    now: &chrono::DateTime<chrono::Utc>,
) -> bool {
    if mandate.status != crate::user_mandates::MandateStatus::Active {
        return false;
    }
    let Some(expiry) = mandate.expiry.as_deref() else {
        return true;
    };
    chrono::DateTime::parse_from_rfc3339(expiry)
        .map(|expiry| expiry.with_timezone(&chrono::Utc) > *now)
        .unwrap_or(false)
}

fn mandate_source_filters_match(
    source_filters_json: &str,
    event: &crate::assistant_events::StoredAssistantEvent,
) -> bool {
    let Ok(filters) = serde_json::from_str::<serde_json::Value>(source_filters_json) else {
        return false;
    };
    let Some(filter_map) = filters.as_object() else {
        return true;
    };
    if filter_map.is_empty() {
        return true;
    }

    let mut any_known_filter = false;
    if let Some(expected_domain) = filter_map
        .get("from_domain")
        .and_then(serde_json::Value::as_str)
    {
        any_known_filter = true;
        let Some(sender) = assistant_event_entity_string(event, "from") else {
            return false;
        };
        if !sender
            .to_ascii_lowercase()
            .contains(&expected_domain.to_ascii_lowercase())
        {
            return false;
        }
    }
    if let Some(needle) = filter_map
        .get("from_contains")
        .and_then(serde_json::Value::as_str)
    {
        any_known_filter = true;
        let Some(sender) = assistant_event_entity_string(event, "from") else {
            return false;
        };
        if !sender
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
        {
            return false;
        }
    }
    if let Some(needle) = filter_map
        .get("title_contains")
        .or_else(|| filter_map.get("subject_contains"))
        .and_then(serde_json::Value::as_str)
    {
        any_known_filter = true;
        if !event
            .event
            .title
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
        {
            return false;
        }
    }
    any_known_filter
}

fn mandate_match_source_work_item_id(
    event: &crate::assistant_events::StoredAssistantEvent,
) -> String {
    match event.event.source {
        crate::assistant_events::AssistantEventSource::Calendar => format!("calendar:{}", event.id),
        _ => opaque_unified_inbox_id("event", &event.id.to_string()),
    }
}

fn unified_inbox_item_from_mandate_match(
    mandate: &crate::user_mandates::UserMandate,
    event: &crate::assistant_events::StoredAssistantEvent,
    generated_at: &str,
    notification_plan: Option<&MandateNotificationProjection>,
    locale: &str,
) -> UnifiedInboxItem {
    let source_item_id = mandate_match_source_work_item_id(event);
    let mandate_title: String = mandate.title.chars().take(120).collect();
    let reminder_status = notification_plan
        .map(|plan| mandate_notification_status_label(plan.status, locale).to_string())
        .unwrap_or_else(|| t("console.mandate.reminder.not_prepared", locale).to_string());
    UnifiedInboxItem {
        id: opaque_unified_inbox_id(
            "mandate_match",
            &format!("{}:{}", mandate.mandate_id, event.id),
        ),
        kind: "alert".into(),
        title: tfp("console.mandate.title", locale, &[&mandate_title]),
        title_code: None,
        summary: tfp(
            "console.mandate.summary",
            locale,
            &[
                mandate_scope_label(&mandate.scope_kind, locale),
                assistant_event_source_label(event.event.source, locale),
            ],
        ),
        owner_visible_reason: t("console.mandate.reason", locale).to_string(),
        owner_visible_reason_code: Some("rule_matched_source".into()),
        authority_basis: "active_mandate".into(),
        source: "mandates".into(),
        source_id: opaque_unified_inbox_id("mandate", &mandate.mandate_id),
        source_event_id: Some(source_item_id.clone()),
        status: "matched".into(),
        urgency: unified_inbox_event_urgency(&event.event).into(),
        sensitivity: unified_inbox_event_sensitivity(&event.event, "S2").into(),
        privacy_level: "owner_only".into(),
        created_at: event.event.observed_at.clone(),
        updated_at: generated_at.into(),
        due_at: None,
        primary_action: None,
        secondary_actions: Vec::new(),
        next_safe_action: t("console.mandate.next", locale).to_string(),
        next_safe_action_code: Some("open_source_inspect".into()),
        blocked_by: Some(t("console.mandate.blocked", locale).to_string()),
        blocked_by_code: Some("blocked_by_source_details_locked".into()),
        receipt_expectation: if notification_plan.is_some() {
            t("console.mandate.receipt.planned", locale)
        } else {
            t("console.mandate.receipt.none", locale)
        }
        .to_string(),
        authority: None,
        detail_route: Some(format!("/ui/?view=work&item={source_item_id}")),
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: notification_plan
            .map(|plan| vec![format!("notification:{}", plan.idempotency_key)])
            .unwrap_or_default(),
        route_reason: tfp("console.mandate.route", locale, &[&reminder_status]),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

/// Owner-facing name for an assistant-event source, so it can sit inside a
/// localized sentence instead of leaking the wire id ("calendar").
fn assistant_event_source_label(
    source: crate::assistant_events::AssistantEventSource,
    locale: &str,
) -> &'static str {
    use crate::assistant_events::AssistantEventSource as S;
    match source {
        S::Calendar => t("console.event_source.calendar", locale),
        S::Drive => t("console.event_source.drive", locale),
        S::Chronos => t("console.event_source.chronos", locale),
        S::Telegram => t("console.event_source.telegram", locale),
        S::Browser => t("console.event_source.browser", locale),
        S::Unknown => t("console.event_source.unknown", locale),
    }
}

fn mandate_scope_label(scope: &str, locale: &str) -> &'static str {
    match scope {
        "email_metadata" => t("console.mandate.scope.email", locale),
        "calendar_metadata" => t("console.mandate.scope.calendar", locale),
        "reminders" => t("console.mandate.scope.reminders", locale),
        "system_alerts" => t("console.mandate.scope.alerts", locale),
        "api_costs" => t("console.mandate.scope.costs", locale),
        "approvals_receipts" => t("console.mandate.scope.approvals", locale),
        "memory_review_queue" => t("console.mandate.scope.memory", locale),
        _ => t("console.mandate.scope.generic", locale),
    }
}

fn mandate_notification_status_label(status: &str, locale: &str) -> &'static str {
    match status {
        "planned" => t("console.mandate.notify_status.ready", locale),
        "duplicate" => t("console.mandate.notify_status.shown", locale),
        "daily_cap_suppressed" => t("console.mandate.notify_status.cap", locale),
        "sent" => t("console.mandate.notify_status.sent", locale),
        "cancelled" => t("console.mandate.notify_status.cancelled", locale),
        _ => t("console.mandate.notify_status.waiting", locale),
    }
}

pub(crate) fn build_mandate_notification_plan(
    mandate: &crate::user_mandates::UserMandate,
    event: &crate::assistant_events::StoredAssistantEvent,
    admin_chat_id: i64,
    stored_chat_id: Option<i64>,
    locale: &str,
) -> crate::user_mandates::NewMandateNotificationPlan {
    use crate::telegram_notification_router::{
        NotificationChannel, NotificationClass, NotificationRenderRequest, NotificationUrgency,
        PrivacyLevel, TelegramNotificationRouter, proactive_route_request,
        render_notification_payload,
    };

    let privacy_level = if matches!(
        unified_inbox_event_sensitivity(&event.event, "S2"),
        "S3" | "S4"
    ) {
        PrivacyLevel::SensitivePreferDm
    } else {
        PrivacyLevel::PrivateDmOnly
    };
    let route =
        TelegramNotificationRouter::from_topic_store(admin_chat_id).route(proactive_route_request(
            NotificationClass::ProactiveAlert,
            privacy_level,
            stored_chat_id,
        ));
    // Group/forum routing was removed — every mandate notification is a DM.
    let channel = NotificationChannel::TelegramDm;
    let source_item_id = mandate_match_source_work_item_id(event);
    let detail_url = Some(format!("/ui/?view=work&item={source_item_id}"));
    let rendered = render_notification_payload(NotificationRenderRequest {
        notification_class: NotificationClass::ProactiveAlert,
        sensitivity: match unified_inbox_event_sensitivity(&event.event, "S2") {
            "S0" => crate::data_access_gate::DataSensitivity::S0Public,
            "S1" => crate::data_access_gate::DataSensitivity::S1LowPersonal,
            "S3" => crate::data_access_gate::DataSensitivity::S3Sensitive,
            "S4" => crate::data_access_gate::DataSensitivity::S4HighlySensitive,
            _ => crate::data_access_gate::DataSensitivity::S2Personal,
        },
        urgency: match unified_inbox_event_urgency(&event.event) {
            "urgent" | "high" => NotificationUrgency::Immediate,
            "normal" => NotificationUrgency::SameDay,
            "low" => NotificationUrgency::Digest,
            _ => NotificationUrgency::DashboardOnly,
        },
        channel,
        title: tfp(
            "console.mandate.notify.title",
            locale,
            &[&mandate.title.chars().take(80).collect::<String>()],
        ),
        safe_summary: tfp(
            "console.mandate.notify.summary",
            locale,
            &[assistant_event_source_label(event.event.source, locale)],
        ),
        sensitive_summary: None,
        allow_sensitive_detail: false,
        detail_url: detail_url.clone(),
    });

    crate::user_mandates::NewMandateNotificationPlan {
        mandate_id: mandate.mandate_id.clone(),
        owner_principal: mandate.owner_principal.clone(),
        source_event_id: format!("assistant_event:{}", event.id),
        idempotency_key: opaque_unified_inbox_id(
            "mandate_notification",
            &format!("{}:{}", mandate.mandate_id, event.id),
        ),
        notification_class: "proactive_alert".into(),
        privacy_level: format!("{:?}", privacy_level),
        route_reason: user_facing_notification_route_reason(route.reason).into(),
        route_chat_id: route.chat_id,
        route_thread_id: route.thread_id.map(i64::from),
        title: rendered.title,
        body: rendered.body,
        detail_url: rendered.detail_url.or(detail_url),
        redacted: rendered.redacted,
        trace_id: mandate.trace_id.clone(),
        run_id: mandate.run_id.clone(),
    }
}

/// A stored tool hold belongs on the chat thread, not the work stack.
async fn append_research_job_items(
    memory: &crate::memory::MemoryHandle,
    owner: &str,
    generated_at: &str,
    locale: &str,
    sources: &mut Vec<UnifiedInboxSourceStatus>,
    items: &mut Vec<UnifiedInboxItem>,
) {
    let entries = match memory
        .list_skill_state_entries(crate::search_turn::RESEARCH_JOB_SKILL)
        .await
    {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, "Unable to project research jobs");
            sources.push(UnifiedInboxSourceStatus {
                source: "research".into(),
                status: "unavailable".into(),
                reason: Some(t("console.research.source_unavailable", locale).to_string()),
            });
            return;
        }
    };
    sources.push(UnifiedInboxSourceStatus {
        source: "research".into(),
        status: "available".into(),
        reason: None,
    });
    for (key, raw) in entries {
        let Some(job) = crate::search_turn::parse_research_job(&raw) else {
            continue;
        };
        if !job.user_did.is_empty() && job.user_did != owner {
            continue;
        }
        let status = if job.status == "running" {
            "tracking"
        } else {
            "done"
        };
        let id = if job.id.is_empty() { key } else { job.id };
        let item_id = format!("research:{id}");
        items.push(UnifiedInboxItem {
            id: item_id,
            kind: "research".into(),
            title: job.title,
            title_code: None,
            summary: crate::search_turn::clip(&job.summary, 280),
            owner_visible_reason: t("console.research.reason", locale).to_string(),
            owner_visible_reason_code: None,
            authority_basis: "owner_request".into(),
            source: "research".into(),
            source_id: id,
            source_event_id: None,
            status: status.into(),
            urgency: "normal".into(),
            sensitivity: "S1".into(),
            privacy_level: "owner_only".into(),
            created_at: generated_at.to_string(),
            updated_at: generated_at.to_string(),
            due_at: None,
            primary_action: None,
            secondary_actions: Vec::new(),
            next_safe_action: t("console.research.next", locale).to_string(),
            next_safe_action_code: None,
            blocked_by: None,
            blocked_by_code: None,
            receipt_expectation: t("console.research.receipt", locale).to_string(),
            authority: None,
            detail_route: None,
            external_url_policy: "none".into(),
            related_action_ids: Vec::new(),
            related_receipt_ids: Vec::new(),
            route_reason: t("console.research.route", locale).to_string(),
            lineage: Vec::new(),
            schema_version: 1,
        });
    }
}

fn chat_thread_tool_hold(value: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|json| {
            json.get("tool_name")
                .and_then(|tool| tool.as_str())
                .map(str::to_string)
        })
        .is_some_and(|tool| !tool.trim().is_empty())
}

fn webapp_hitl_approval_belongs_to_owner(value: &str, chat_id: i64, owner: &str) -> bool {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(value) else {
        return true;
    };
    if json
        .get("requested_by")
        .and_then(|value| value.as_str())
        .is_some_and(|requested_by| requested_by != owner)
    {
        return false;
    }
    if json
        .get("chat_id")
        .or_else(|| json.get("target_chat_id"))
        .and_then(|value| value.as_i64())
        .is_some_and(|approval_chat_id| approval_chat_id != chat_id)
    {
        return false;
    }
    true
}

pub(super) fn hitl_approval_source_task_id(value: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("source_task_id")?
        .as_i64()
}

pub(super) fn unified_inbox_active_item_from_assistant_event(
    event: &crate::assistant_events::StoredAssistantEvent,
    locale: &str,
) -> Option<UnifiedInboxItem> {
    match (event.event.source, event.event.kind) {
        (
            crate::assistant_events::AssistantEventSource::Calendar,
            crate::assistant_events::AssistantEventKind::CalendarInviteReceived
            | crate::assistant_events::AssistantEventKind::CalendarEventChanged,
        ) => Some(unified_inbox_item_from_calendar_event(event, locale)),
        _ => None,
    }
}

fn assistant_event_is_stale_unread(
    event: &crate::assistant_events::StoredAssistantEvent,
    now: &chrono::DateTime<chrono::Utc>,
) -> bool {
    assistant_event_time(&event.event.occurred_at)
        .or_else(|| assistant_event_time(&event.event.observed_at))
        .map(|event_time| {
            now.signed_duration_since(event_time).num_days() >= WEBAPP_STALE_UNREAD_EVENT_DAYS
        })
        .unwrap_or(false)
}

fn assistant_event_time(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

pub(super) fn unified_inbox_item_from_calendar_event(
    event: &crate::assistant_events::StoredAssistantEvent,
    locale: &str,
) -> UnifiedInboxItem {
    let action = assistant_event_entity_string(event, "action")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| t("console.calendar.action.changed", locale).to_string());
    let start = assistant_event_entity_string(event, "start").map(ToOwned::to_owned);
    let end = assistant_event_entity_string(event, "end")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| t("console.calendar.end.unknown", locale).to_string());
    let attendee_count = event
        .event
        .entities
        .get("attendee_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let source_event_id = assistant_event_entity_string(event, "event_id")
        .map(|value| opaque_unified_inbox_id("calendar_event", value));
    let attendee_count_s = attendee_count.to_string();

    UnifiedInboxItem {
        id: format!("calendar:{}", event.id),
        kind: "calendar".into(),
        title: event.event.title.chars().take(180).collect(),
        title_code: None,
        summary: tfp(
            "console.calendar.summary",
            locale,
            &[&action, &end, &attendee_count_s],
        ),
        owner_visible_reason: t("console.calendar.reason", locale).to_string(),
        owner_visible_reason_code: Some("calendar_basics_changed".into()),
        authority_basis: "manual_review".into(),
        source: "calendar".into(),
        source_id: event.id.to_string(),
        source_event_id,
        status: action,
        urgency: unified_inbox_event_urgency(&event.event).into(),
        sensitivity: unified_inbox_event_sensitivity(&event.event, "S2").into(),
        privacy_level: "owner_only".into(),
        created_at: event.event.observed_at.clone(),
        updated_at: event.event.occurred_at.clone(),
        due_at: start,
        primary_action: Some(UnifiedInboxAction {
            label: t("console.calendar.action.review", locale).to_string(),
            label_code: Some("review_calendar_basics".into()),
            action_kind: "context".into(),
            route: None,
            requires_gate: false,
            requires_mfa: false,
        }),
        secondary_actions: Vec::new(),
        next_safe_action: t("console.calendar.next", locale).to_string(),
        next_safe_action_code: Some("calendar_basics_review".into()),
        blocked_by: Some(t("console.calendar.blocked", locale).to_string()),
        blocked_by_code: Some("blocked_by_calendar_details_hidden".into()),
        receipt_expectation: t("console.calendar.receipt", locale).to_string(),
        authority: None,
        detail_route: None,
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.calendar.route", locale).to_string(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

fn unified_inbox_event_urgency(event: &crate::assistant_events::AssistantEvent) -> &'static str {
    if matches!(
        event.decision,
        crate::assistant_events::AssistantEventDecision::NotifyNow
            | crate::assistant_events::AssistantEventDecision::CallbackNow
    ) || event.actionability_hint == crate::assistant_events::EventActionabilityHint::Act
    {
        return "high";
    }
    match event.urgency_hint {
        crate::assistant_events::EventUrgencyHint::High => "high",
        crate::assistant_events::EventUrgencyHint::Medium => "normal",
        crate::assistant_events::EventUrgencyHint::Low
        | crate::assistant_events::EventUrgencyHint::Unknown => "low",
    }
}

fn unified_inbox_event_sensitivity(
    event: &crate::assistant_events::AssistantEvent,
    default_sensitivity: &'static str,
) -> &'static str {
    let raw = event
        .entities
        .get("notification_sensitivity")
        .or_else(|| event.entities.get("sensitivity"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim();
    match raw {
        "s0_public" | "S0Public" => "S0",
        "s1_low_personal" | "S1LowPersonal" => "S1",
        "s2_personal" | "S2Personal" => "S2",
        "s3_sensitive" | "S3Sensitive" => "S3",
        "s4_highly_sensitive" | "S4HighlySensitive" => "S4",
        _ => default_sensitivity,
    }
}

/// Work cards for tasks delegated to Hercules (`delegate_task`). Read from
/// the `hercules_delegation` skill state kept by the 30 s reconciliation
/// lane; no network here. Hercules owns execution, so the card only
/// projects state / PR / verified SHA and tells the owner where to act.
pub(super) async fn append_hercules_delegation_items(
    memory: &crate::memory::MemoryHandle,
    locale: &str,
    sources: &mut Vec<UnifiedInboxSourceStatus>,
    items: &mut Vec<UnifiedInboxItem>,
) {
    let entries = match memory
        .list_skill_state_entries(crate::orchestrator_tools_delegation::HERCULES_LINKAGE_SKILL)
        .await
    {
        Ok(entries) => entries,
        Err(_err) => {
            sources.push(UnifiedInboxSourceStatus {
                source: "hercules".into(),
                status: "unavailable".into(),
                reason: Some(t("console.delegation.source_unavailable", locale).to_string()),
            });
            return;
        }
    };
    sources.push(UnifiedInboxSourceStatus {
        source: "hercules".into(),
        status: "available".into(),
        reason: None,
    });
    let mut cards: Vec<UnifiedInboxItem> = entries
        .iter()
        .filter_map(|(task_id, raw)| {
            let link: serde_json::Value = serde_json::from_str(raw).ok()?;
            Some(unified_inbox_item_from_delegation(task_id, &link, locale))
        })
        .collect();
    cards.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    items.extend(cards.into_iter().take(20));
}

fn delegation_state_label(state: &str, locale: &str) -> String {
    let key = match state {
        "review" => "console.delegation.state.review",
        "failed" => "console.delegation.state.failed",
        "done" => "console.delegation.state.done",
        "cancelled" | "deleted" => "console.delegation.state.cancelled",
        "missing" => "console.delegation.state.missing",
        _ => "console.delegation.state.active",
    };
    t(key, locale).to_string()
}

pub(super) fn unified_inbox_item_from_delegation(
    task_id: &str,
    link: &serde_json::Value,
    locale: &str,
) -> UnifiedInboxItem {
    let str_field = |key: &str| {
        link.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let state = str_field("last_state").unwrap_or_else(|| "todo".into());
    let title = str_field("title")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| task_id.to_string());
    let repo = str_field("repo").unwrap_or_else(|| "?".into());
    let pr_url = str_field("pr_url");
    let verified_sha = str_field("verified_sha");
    let created_at = str_field("created_at").unwrap_or_default();
    let updated_at = str_field("updated_at").unwrap_or_else(|| created_at.clone());
    let hold_code = str_field("hold_code").filter(|c| !c.is_empty());

    // Map Hercules states onto the Work view's status vocabulary (client
    // `laneForItem`): review = decision, active = follow-up, terminal = closed.
    let (status, urgency, next_safe_key, route_key) = match state.as_str() {
        "review" => (
            "needs_decision",
            "high",
            "console.delegation.next.review",
            "console.delegation.route.review",
        ),
        "failed" => (
            "failed",
            "high",
            "console.delegation.next.failed",
            "console.delegation.route.failed",
        ),
        "done" => (
            "done",
            "low",
            "console.delegation.next.done",
            "console.delegation.route.done",
        ),
        "cancelled" | "deleted" => (
            "cancelled",
            "low",
            "console.delegation.next.cancelled",
            "console.delegation.route.cancelled",
        ),
        "missing" => (
            "archived",
            "low",
            "console.delegation.next.missing",
            "console.delegation.route.missing",
        ),
        _ => (
            "tracking",
            "normal",
            "console.delegation.next.wait",
            "console.delegation.route.wait",
        ),
    };
    let next_safe_action = match hold_code.as_deref() {
        Some("repo_readiness_incomplete") => t("console.delegation.next.hold_ready", locale),
        Some("plan_pending") | Some("host_approval_pending") => {
            t("console.delegation.next.hold_approval", locale)
        }
        Some(_) => t("console.delegation.next.hold_other", locale),
        None => t(next_safe_key, locale),
    };
    let route_reason = if hold_code.is_some() {
        t("console.delegation.route.hold", locale)
    } else {
        t(route_key, locale)
    };
    let state_label = delegation_state_label(&state, locale);
    let mut summary = tfp("console.delegation.summary", locale, &[&repo, &state_label]);
    if let Some(sha) = verified_sha.as_deref().filter(|s| !s.is_empty()) {
        summary.push_str(" · ");
        summary.push_str(&tfp("console.delegation.verified", locale, &[sha]));
    }
    if let Some(pr) = pr_url.as_deref().filter(|p| !p.is_empty()) {
        summary.push('\n');
        summary.push_str(&tfp("console.delegation.pr", locale, &[pr]));
    }

    UnifiedInboxItem {
        id: opaque_unified_inbox_id("delegation", task_id),
        kind: "delegation".into(),
        title,
        title_code: None,
        summary,
        owner_visible_reason: t("console.delegation.reason", locale).to_string(),
        owner_visible_reason_code: None,
        authority_basis: "direct_owner_request".into(),
        source: "hercules".into(),
        source_id: task_id.to_string(),
        source_event_id: None,
        status: status.into(),
        urgency: urgency.into(),
        sensitivity: "S1".into(),
        privacy_level: "owner_only".into(),
        created_at,
        updated_at,
        due_at: None,
        primary_action: None,
        secondary_actions: Vec::new(),
        next_safe_action: next_safe_action.to_string(),
        next_safe_action_code: None,
        blocked_by: None,
        blocked_by_code: None,
        receipt_expectation: t("console.delegation.receipt", locale).to_string(),
        authority: None,
        detail_route: None,
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: route_reason.into(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

pub(super) fn unified_inbox_item_from_approval(
    approval_id: &str,
    value: &str,
    now: &str,
    locale: &str,
) -> UnifiedInboxItem {
    let approval_type = trust_console_approval_type(approval_id, value);
    let risk = trust_console_approval_risk(&approval_type);
    let requires_mfa = trust_console_approval_requires_mfa(risk);
    let urgency = if requires_mfa { "high" } else { "normal" };
    let sensitivity = if requires_mfa { "S3" } else { "S2" };
    let source_id = opaque_unified_inbox_id("approval_source", approval_id);
    let display = approval_display_for_owner(&approval_type, risk, value, locale);

    UnifiedInboxItem {
        id: opaque_unified_inbox_id("approval", approval_id),
        kind: "approval".into(),
        title: display.title,
        title_code: Some(display.title_code.into()),
        summary: display.summary,
        owner_visible_reason: display.owner_visible_reason,
        owner_visible_reason_code: Some(display.owner_visible_reason_code.into()),
        authority_basis: "manual_review".into(),
        source: "hitl".into(),
        source_id,
        source_event_id: None,
        status: "waiting".into(),
        urgency: urgency.into(),
        sensitivity: sensitivity.into(),
        privacy_level: "owner_only".into(),
        created_at: now.into(),
        updated_at: now.into(),
        due_at: None,
        primary_action: Some(UnifiedInboxAction {
            label: t("console.approval.action.review", locale).to_string(),
            label_code: Some("review_approval".into()),
            action_kind: "navigate".into(),
            route: Some("/ui/?tab=approvals".into()),
            requires_gate: true,
            requires_mfa,
        }),
        secondary_actions: approval_decision_actions(requires_mfa, locale),
        next_safe_action: display.next_safe_action,
        next_safe_action_code: Some(display.next_safe_action_code.into()),
        blocked_by: Some(display.blocked_by),
        blocked_by_code: Some(display.blocked_by_code.into()),
        receipt_expectation: t("console.approval.receipt", locale).to_string(),
        authority: None,
        detail_route: Some("/ui/?tab=approvals".into()),
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.route.approval", locale).to_string(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

fn approval_decision_actions(requires_mfa: bool, locale: &str) -> Vec<UnifiedInboxAction> {
    vec![
        UnifiedInboxAction {
            label: t("console.approval.action.approve", locale).to_string(),
            label_code: Some("approve".into()),
            action_kind: "approval.approve".into(),
            route: None,
            requires_gate: true,
            requires_mfa,
        },
        UnifiedInboxAction {
            label: t("console.approval.action.reject", locale).to_string(),
            label_code: Some("reject".into()),
            action_kind: "approval.reject".into(),
            route: None,
            requires_gate: false,
            requires_mfa: false,
        },
    ]
}

struct OwnerApprovalDisplay {
    title: String,
    title_code: &'static str,
    summary: String,
    owner_visible_reason: String,
    /// Structural code for `owner_visible_reason` (all branches are static).
    owner_visible_reason_code: &'static str,
    next_safe_action: String,
    /// Structural code for `next_safe_action` (all branches are static).
    next_safe_action_code: &'static str,
    blocked_by: String,
    blocked_by_code: &'static str,
}

fn approval_display_for_owner(
    approval_type: &str,
    risk: &str,
    value: &str,
    locale: &str,
) -> OwnerApprovalDisplay {
    let parsed =
        serde_json::from_str::<serde_json::Value>(value).unwrap_or(serde_json::Value::Null);

    if approval_type == "mcp_server" {
        let unknown_server = t("console.approval.server.unknown", locale);
        let unknown_tool = t("console.approval.tool.unknown", locale);
        let server = parsed
            .get("server_key")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(unknown_server);
        let tool = parsed
            .get("tool_requested")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(unknown_tool);
        let command = parsed
            .get("server_command")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                tfp(
                    "console.approval.summary.mcp.command",
                    locale,
                    &[&owner_visible_inline(value, 120)],
                )
            })
            .unwrap_or_default();
        let server_s = owner_visible_inline(server, 80);
        let tool_s = owner_visible_inline(tool, 80);
        return OwnerApprovalDisplay {
            title: t("console.approval.title.tool", locale).to_string(),
            title_code: "approval_title_tool_server",
            summary: tfp(
                "console.approval.summary.mcp",
                locale,
                &[&server_s, &tool_s, &command],
            ),
            owner_visible_reason: t("console.approval.reason.tool", locale).to_string(),
            owner_visible_reason_code: "approval_tool_server",
            next_safe_action: t("console.approval.next.tool", locale).to_string(),
            next_safe_action_code: "approval_tool_server_confirm",
            blocked_by: t("console.approval.blocked.waiting", locale).to_string(),
            blocked_by_code: "approval_waiting_decision",
        };
    }

    OwnerApprovalDisplay {
        title: t("console.approval.title.generic", locale).to_string(),
        title_code: "approval_title_generic",
        summary: user_facing_approval_summary(approval_type, risk, locale),
        owner_visible_reason: t("console.approval.reason.generic", locale).to_string(),
        owner_visible_reason_code: "approval_generic",
        next_safe_action: if trust_console_approval_requires_mfa(risk) {
            t("console.approval.next.high_risk", locale).to_string()
        } else {
            t("console.approval.next.generic", locale).to_string()
        },
        next_safe_action_code: if trust_console_approval_requires_mfa(risk) {
            "approval_high_risk_review"
        } else {
            "approval_review_decide"
        },
        blocked_by: if trust_console_approval_requires_mfa(risk) {
            t("console.approval.blocked.mfa", locale).to_string()
        } else {
            t("console.approval.blocked.owner", locale).to_string()
        },
        blocked_by_code: if trust_console_approval_requires_mfa(risk) {
            "approval_waiting_mfa"
        } else {
            "approval_waiting_owner"
        },
    }
}

fn owner_visible_inline(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let head: String = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    format!("{head}…")
}

fn user_facing_approval_summary(approval_type: &str, risk: &str, locale: &str) -> String {
    tfp(
        "console.approval.summary.generic",
        locale,
        &[user_facing_approval_action(approval_type, locale), risk],
    )
}

fn user_facing_approval_action(approval_type: &str, locale: &str) -> &'static str {
    if approval_type.contains("setting_change") {
        t("console.approval.kind.settings", locale)
    } else if approval_type.contains("mcp_server") {
        t("console.approval.kind.tool_conn", locale)
    } else if approval_type.contains("tool:") {
        t("console.approval.kind.tool", locale)
    } else {
        t("console.approval.kind.waiting", locale)
    }
}

/// Next fire after skipping the current occurrence. Same arithmetic as the
/// tasks-panel `task_skip`: advance from the stored trigger by one interval,
/// or from now when the row has no trigger yet.
pub(super) fn recurring_skip_trigger(
    now_unix: i64,
    trigger_at_unix: Option<i64>,
    interval_seconds: i64,
) -> i64 {
    trigger_at_unix
        .map(|trigger| trigger.saturating_add(interval_seconds))
        .unwrap_or_else(|| now_unix.saturating_add(interval_seconds))
}

/// Next fire for a clock loop that should stay open. A trigger already in
/// the future is kept. A trigger in the past jumps to the next future slot
/// so one acknowledgement does not replay every missed day.
pub(super) fn next_open_clock_trigger(
    now_unix: i64,
    trigger_at_unix: Option<i64>,
    interval_seconds: i64,
) -> i64 {
    match trigger_at_unix {
        Some(trigger) if trigger > now_unix => trigger,
        Some(trigger) if interval_seconds > 0 => {
            let steps = now_unix.saturating_sub(trigger) / interval_seconds + 1;
            trigger.saturating_add(steps.saturating_mul(interval_seconds))
        }
        _ => now_unix.saturating_add(interval_seconds.max(0)),
    }
}

/// Owner-created clock loop that has not used up max_fires. Archived and
/// cancelled rows are not loops waiting to be shown again.
pub(super) fn clock_loop_still_scheduled(task: &crate::memory::Task) -> bool {
    if task_is_system_maintenance_loop(task) {
        return false;
    }
    if task.interval_seconds.is_none_or(|seconds| seconds <= 0) {
        return false;
    }
    if let Some(max) = task.max_fires.filter(|max| *max > 0)
        && i64::from(task.fire_count) >= max
    {
        return false;
    }
    !matches!(
        task.status.as_str(),
        "archived" | "cancelled" | "canceled" | "deleted" | "expired"
    )
}

/// Completing one occurrence wrote status=done on the generator. The work
/// list only loads open statuses, so the same reminder disappeared.
pub(super) fn closed_clock_loop_should_reopen(task: &crate::memory::Task) -> bool {
    task.status == "done" && clock_loop_still_scheduled(task)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskTrackingShape {
    /// `interval_seconds > 0`. The open card is the generator.
    ClockLoop,
    /// Standing watch (event, semantic alert, callback, notify-without-clock).
    StandingIntent,
    /// One trigger, no loop. Closing it closes the only occurrence.
    OneShotIntent,
    /// Owner tracking note with no scheduler parent.
    TrackingNote,
    Ordinary,
}

fn task_clock_recurrence(task: &crate::memory::Task) -> bool {
    task.interval_seconds.is_some_and(|seconds| seconds > 0)
}

fn task_tracking_shape(task: &crate::memory::Task) -> TaskTrackingShape {
    if task_clock_recurrence(task) {
        TaskTrackingShape::ClockLoop
    } else if task_is_background_loop(task) {
        TaskTrackingShape::StandingIntent
    } else if task.trigger_at_unix.is_some() || matches!(task.trigger_type.as_deref(), Some("time"))
    {
        TaskTrackingShape::OneShotIntent
    } else if semantic_task_projection_kind(task) == Some("tracking") {
        TaskTrackingShape::TrackingNote
    } else {
        TaskTrackingShape::Ordinary
    }
}

fn task_console_action(
    task_id: i64,
    locale: &str,
    label_key: &str,
    label_code: &str,
    action_kind: &str,
) -> UnifiedInboxAction {
    UnifiedInboxAction {
        label: t(label_key, locale).to_string(),
        label_code: Some(label_code.into()),
        action_kind: action_kind.into(),
        route: Some(format!("/webapp/vnext/work/task:{task_id}/actions")),
        requires_gate: false,
        requires_mfa: false,
    }
}

fn task_tracking_secondary_actions(
    task: &crate::memory::Task,
    task_closed: bool,
    blocked_owner_automation: bool,
    locale: &str,
) -> Vec<UnifiedInboxAction> {
    if task_closed {
        return Vec::new();
    }
    if blocked_owner_automation {
        return vec![
            UnifiedInboxAction {
                label: t("console.task.action.close_without_running", locale).to_string(),
                label_code: Some("close_without_running".into()),
                action_kind: "task.complete".into(),
                route: Some(format!("/webapp/console/tasks/{}/action", task.id)),
                requires_gate: false,
                requires_mfa: false,
            },
            UnifiedInboxAction {
                label: t("console.task.action.review_tomorrow", locale).to_string(),
                label_code: Some("review_tomorrow".into()),
                action_kind: "task.snooze_tomorrow".into(),
                route: Some(format!("/webapp/console/tasks/{}/action", task.id)),
                requires_gate: false,
                requires_mfa: false,
            },
        ];
    }
    let active = task.status == "active";
    match task_tracking_shape(task) {
        TaskTrackingShape::ClockLoop if active => vec![
            task_console_action(
                task.id,
                locale,
                "console.tracking.action.skip",
                "skip_occurrence",
                "task.skip_occurrence",
            ),
            task_console_action(
                task.id,
                locale,
                "console.tracking.action.cancel",
                "close_tracking",
                "tracking.cancel",
            ),
        ],
        TaskTrackingShape::StandingIntent | TaskTrackingShape::OneShotIntent if active => {
            vec![task_console_action(
                task.id,
                locale,
                "console.tracking.action.cancel",
                "close_tracking",
                "tracking.cancel",
            )]
        }
        TaskTrackingShape::ClockLoop
        | TaskTrackingShape::StandingIntent
        | TaskTrackingShape::OneShotIntent
        | TaskTrackingShape::TrackingNote => vec![
            task_console_action(
                task.id,
                locale,
                "console.tracking.action.end_once",
                "end_this_once",
                "task.complete",
            ),
            task_console_action(
                task.id,
                locale,
                "console.task.action.review_tomorrow",
                "review_tomorrow",
                "task.snooze_tomorrow",
            ),
        ],
        TaskTrackingShape::Ordinary => vec![
            UnifiedInboxAction {
                label: t("console.task.action.complete", locale).to_string(),
                label_code: Some("complete".into()),
                action_kind: "task.complete".into(),
                route: Some(format!("/webapp/console/tasks/{}/action", task.id)),
                requires_gate: false,
                requires_mfa: false,
            },
            UnifiedInboxAction {
                label: t("console.task.action.snooze", locale).to_string(),
                label_code: Some("snooze".into()),
                action_kind: "task.snooze_tomorrow".into(),
                route: Some(format!("/webapp/console/tasks/{}/action", task.id)),
                requires_gate: false,
                requires_mfa: false,
            },
        ],
    }
}

fn lineage_detail(parts: &[String]) -> String {
    owner_visible_inline(&parts.join(" · "), 480)
}

fn task_tracking_lineage(
    task: &crate::memory::Task,
    task_closed: bool,
    blocked_owner_automation: bool,
    locale: &str,
) -> Vec<WorkLineageNode> {
    if task_closed {
        return Vec::new();
    }
    let shape = task_tracking_shape(task);
    if shape == TaskTrackingShape::Ordinary {
        return Vec::new();
    }
    let active = task.status == "active" && !blocked_owner_automation;
    let (layer, label_key, label_code, action_ids) = match shape {
        TaskTrackingShape::ClockLoop => (
            "intent",
            "console.lineage.intent.recurring",
            "intent.recurring",
            if active {
                vec![
                    "task.skip_occurrence".to_string(),
                    "tracking.cancel".to_string(),
                ]
            } else if !blocked_owner_automation {
                vec!["task.complete".to_string()]
            } else {
                Vec::new()
            },
        ),
        TaskTrackingShape::StandingIntent => (
            "intent",
            "console.lineage.intent.watch",
            "intent.watch",
            if active {
                vec!["tracking.cancel".to_string()]
            } else if !blocked_owner_automation {
                vec!["task.complete".to_string()]
            } else {
                Vec::new()
            },
        ),
        TaskTrackingShape::OneShotIntent => (
            "intent",
            "console.lineage.intent.oneshot",
            "intent.oneshot",
            if active {
                vec!["tracking.cancel".to_string()]
            } else if !blocked_owner_automation {
                vec!["task.complete".to_string()]
            } else {
                Vec::new()
            },
        ),
        TaskTrackingShape::TrackingNote => (
            "occurrence",
            "console.lineage.occurrence.card",
            "occurrence.card",
            if blocked_owner_automation {
                Vec::new()
            } else {
                vec!["task.complete".to_string()]
            },
        ),
        TaskTrackingShape::Ordinary => return Vec::new(),
    };
    let parts = task_generator_detail_parts(task, locale);
    let mut nodes = vec![WorkLineageNode {
        layer: layer.into(),
        kind: "task".into(),
        id: format!("task:{}", task.id),
        label: t(label_key, locale).to_string(),
        label_code: Some(label_code.into()),
        detail: lineage_detail(&parts),
        action_ids,
        turn_id: None,
        message_role: None,
    }];
    nodes.push(task_origin_lineage_node(task, locale));
    nodes
}

fn task_generator_detail_parts(task: &crate::memory::Task, locale: &str) -> Vec<String> {
    // The task id is the evidence. group_key / event_key / trigger_type are
    // join keys and enums (an `auto:` group_key is a generated slug). The
    // cadence is real schedule evidence, so it stays as a phrase.
    let mut parts = vec![tf("console.lineage.taskRef", locale, &task.id.to_string())];
    if let Some(interval) = task.interval_seconds.filter(|seconds| *seconds > 0) {
        parts.push(owner_interval_phrase(interval, locale));
    }
    parts
}

fn owner_interval_phrase(seconds: i64, locale: &str) -> String {
    let (count, one_key, many_key) = if seconds % 86_400 == 0 {
        (
            seconds / 86_400,
            "console.lineage.everyDay",
            "console.lineage.everyDays",
        )
    } else if seconds % 3_600 == 0 {
        (
            seconds / 3_600,
            "console.lineage.everyHour",
            "console.lineage.everyHours",
        )
    } else if seconds % 60 == 0 {
        (
            seconds / 60,
            "console.lineage.everyMinute",
            "console.lineage.everyMinutes",
        )
    } else {
        (
            seconds,
            "console.lineage.everySecond",
            "console.lineage.everySeconds",
        )
    };
    if count == 1 {
        t(one_key, locale).to_string()
    } else {
        tf(many_key, locale, &count.to_string())
    }
}

fn owner_task_source_who(source: &str, locale: &str) -> String {
    match source.trim().to_ascii_lowercase().as_str() {
        "user" | "owner" | "manual" | "webapp" | "webapp_tracking_draft" => {
            t("console.lineage.who.you", locale).to_string()
        }
        "assistant" | "aegis" | "auto" | "scheduler" | "intent" | "system" => {
            t("console.lineage.who.aegis", locale).to_string()
        }
        other => owner_visible_inline(other, 40),
    }
}

fn task_is_one_shot_schedule(task: &crate::memory::Task) -> bool {
    !task_clock_recurrence(task)
        && (task.trigger_at_unix.is_some() || matches!(task.trigger_type.as_deref(), Some("time")))
}

fn task_is_owned_waiting_generator(
    task: &crate::memory::Task,
    owner_did: &str,
    chat_id: i64,
) -> bool {
    // Storing a task fills in `group_key` from the title. That must not turn a
    // one-shot time trigger into a loop the waiting card can cancel.
    task.chat_id == chat_id
        && task.user_did == owner_did
        && task.status == "active"
        && !task_is_one_shot_schedule(task)
        && matches!(
            task_tracking_shape(task),
            TaskTrackingShape::ClockLoop | TaskTrackingShape::StandingIntent
        )
}

/// Resolve a waiting row's generator from stored tags only. A similar title is not a parent.
pub(super) fn waiting_generator_from_tasks<'a>(
    entry: &crate::memory::KnowledgeEntry,
    tasks: &'a [crate::memory::Task],
    owner_did: &str,
) -> Option<&'a crate::memory::Task> {
    let owned = |task: &&crate::memory::Task| {
        task_is_owned_waiting_generator(task, owner_did, entry.chat_id)
    };
    match crate::memory::waiting_parent_tag(&entry.tags) {
        crate::memory::WaitingParentTag::Task(task_id) => {
            tasks.iter().find(|task| task.id == task_id && owned(task))
        }
        crate::memory::WaitingParentTag::Group(group_key) => {
            let mut matches = tasks.iter().filter(|task| {
                owned(task) && task.group_key.as_deref().map(str::trim) == Some(group_key.as_str())
            });
            let first = matches.next()?;
            if matches.next().is_some() {
                None
            } else {
                Some(first)
            }
        }
        crate::memory::WaitingParentTag::Event(event_key) => {
            let mut matches = tasks.iter().filter(|task| {
                owned(task) && task.event_key.as_deref().map(str::trim) == Some(event_key.as_str())
            });
            let first = matches.next()?;
            if matches.next().is_some() {
                None
            } else {
                Some(first)
            }
        }
        crate::memory::WaitingParentTag::Absent | crate::memory::WaitingParentTag::Unusable => None,
    }
}

fn attach_resolved_waiting_generator(
    item: &mut UnifiedInboxItem,
    entry: &crate::memory::KnowledgeEntry,
    task: &crate::memory::Task,
    locale: &str,
) {
    if !task_is_owned_waiting_generator(task, &task.user_did, task.chat_id) {
        return;
    }
    let (label_key, label_code) = match task_tracking_shape(task) {
        TaskTrackingShape::ClockLoop => ("console.lineage.intent.recurring", "intent.recurring"),
        TaskTrackingShape::StandingIntent => ("console.lineage.intent.watch", "intent.watch"),
        _ => return,
    };
    let node = WorkLineageNode {
        layer: "intent".into(),
        kind: "task".into(),
        id: format!("task:{}", task.id),
        label: t(label_key, locale).to_string(),
        label_code: Some(label_code.into()),
        detail: lineage_detail(&task_generator_detail_parts(task, locale)),
        action_ids: vec!["tracking.cancel".into()],
        turn_id: None,
        message_role: None,
    };
    if !item.lineage.iter().any(|existing| existing.id == node.id) {
        let insert_at = item
            .lineage
            .iter()
            .position(|existing| existing.layer == "origin")
            .unwrap_or(item.lineage.len());
        item.lineage.insert(insert_at, node);
    }
    if item
        .secondary_actions
        .iter()
        .any(|action| action.action_kind == "tracking.cancel")
    {
        return;
    }
    item.secondary_actions.insert(
        1.min(item.secondary_actions.len()),
        UnifiedInboxAction {
            label: t("console.tracking.action.cancel", locale).to_string(),
            label_code: Some("close_tracking".into()),
            action_kind: "tracking.cancel".into(),
            route: Some(format!(
                "/webapp/vnext/work/kb_waiting:{}/actions",
                entry.id
            )),
            requires_gate: false,
            requires_mfa: false,
        },
    );
}

fn task_origin_lineage_node(task: &crate::memory::Task, locale: &str) -> WorkLineageNode {
    let private_body = task_has_private_title_or_body(task);
    let context = task
        .context_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !private_body);
    let mut parts = Vec::new();
    let who = owner_task_source_who(&task.source, locale);
    if !who.is_empty() {
        parts.push(who);
    }
    let when = owner_visible_inline(&task.created_at, 40);
    if !when.is_empty() {
        parts.push(when);
    }
    let (label_key, label_code) = if context.is_some() {
        ("console.lineage.origin.context", "origin.context")
    } else {
        ("console.lineage.origin.record", "origin.record")
    };
    if let Some(context) = context {
        parts.push(owner_visible_inline(context, 160));
    }
    WorkLineageNode {
        layer: "origin".into(),
        kind: "context".into(),
        id: format!("task:{}:origin", task.id),
        label: t(label_key, locale).to_string(),
        label_code: Some(label_code.into()),
        detail: lineage_detail(&parts),
        action_ids: Vec::new(),
        turn_id: None,
        message_role: None,
    }
}

const TRIGGER_CONVERSATION_WINDOW_SECS: i64 = 600;
const TRIGGER_CLOCK_SKEW_SECS: i64 = 5;

fn parse_rfc3339_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|stamp| stamp.timestamp())
}

fn is_owner_archive_role(role: &str) -> bool {
    matches!(role.trim(), "user" | "owner")
}

fn is_aegis_archive_role(role: &str) -> bool {
    matches!(role.trim(), "assistant" | "aegis")
}

fn is_scheduler_callback_text(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("[INTENT_CALLBACK]") || trimmed.starts_with("[INTENT_CALLBACK:depth=")
}

/// The owner message that was already archived when this row was created, plus
/// the assistant reply from that same turn. A scheduler callback between the
/// owner and the row means this row was not created by that chat.
pub(super) fn triggering_conversation_nodes(
    entries: &[crate::memory::L3Entry],
    created_at_unix: i64,
    exact_turn: bool,
    locale: &str,
) -> Vec<WorkLineageNode> {
    let selected = if exact_turn {
        select_exact_turn_messages(entries)
    } else {
        select_time_window_messages(entries, created_at_unix)
    };
    selected
        .into_iter()
        .map(|entry| conversation_lineage_node(entry, locale))
        .collect()
}

fn select_time_window_messages(
    entries: &[crate::memory::L3Entry],
    created_at_unix: i64,
) -> Vec<&crate::memory::L3Entry> {
    let timed = timed_archive_entries(entries);
    let mut owner: Option<(i64, &crate::memory::L3Entry)> = None;
    for (ts, entry) in timed
        .iter()
        .rev()
        .copied()
        .filter(|(ts, _)| *ts <= created_at_unix + TRIGGER_CLOCK_SKEW_SECS)
    {
        if created_at_unix - ts > TRIGGER_CONVERSATION_WINDOW_SECS {
            break;
        }
        if !is_owner_archive_role(&entry.role) {
            continue;
        }
        if is_scheduler_callback_text(&entry.content) {
            return Vec::new();
        }
        owner = Some((ts, entry));
        break;
    }
    let Some((owner_ts, owner_entry)) = owner else {
        return Vec::new();
    };
    let mut out = vec![owner_entry];
    if let Some((_, assistant)) = timed.iter().copied().find(|(ts, entry)| {
        *ts + TRIGGER_CLOCK_SKEW_SECS >= created_at_unix
            && *ts >= owner_ts
            && *ts - owner_ts <= TRIGGER_CONVERSATION_WINDOW_SECS
            && is_aegis_archive_role(&entry.role)
            && !is_scheduler_callback_text(&entry.content)
    }) {
        out.push(assistant);
    }
    out
}

fn select_exact_turn_messages(entries: &[crate::memory::L3Entry]) -> Vec<&crate::memory::L3Entry> {
    let timed = timed_archive_entries(entries);
    let Some((owner_ts, owner_entry)) = timed.iter().copied().find(|(_, entry)| {
        is_owner_archive_role(&entry.role) && !is_scheduler_callback_text(&entry.content)
    }) else {
        return Vec::new();
    };
    let mut out = vec![owner_entry];
    if let Some((_, assistant)) = timed.iter().copied().find(|(ts, entry)| {
        *ts >= owner_ts
            && is_aegis_archive_role(&entry.role)
            && !is_scheduler_callback_text(&entry.content)
    }) {
        out.push(assistant);
    }
    out
}

fn timed_archive_entries(
    entries: &[crate::memory::L3Entry],
) -> Vec<(i64, &crate::memory::L3Entry)> {
    let mut timed: Vec<(i64, &crate::memory::L3Entry)> = entries
        .iter()
        .filter_map(|entry| parse_rfc3339_unix(&entry.created_at).map(|ts| (ts, entry)))
        .collect();
    timed.sort_by_key(|(ts, entry)| (*ts, entry.id));
    timed
}

fn conversation_lineage_node(entry: &crate::memory::L3Entry, locale: &str) -> WorkLineageNode {
    let role = if is_owner_archive_role(&entry.role) {
        "owner"
    } else {
        "aegis"
    };
    let mut parts = vec![
        format!("l3:{}", entry.id),
        format!("role={role}"),
        owner_visible_inline(&entry.content, 180),
    ];
    let turn_id = webapp_lineage_turn_id(entry);
    if let Some(turn_id) = turn_id.as_deref() {
        parts.push(format!("turn_id={}", owner_visible_inline(turn_id, 80)));
    }
    WorkLineageNode {
        layer: "origin".into(),
        kind: "message".into(),
        id: format!("l3:{}", entry.id),
        label: t("console.lineage.origin.message", locale).to_string(),
        label_code: Some("origin.message".into()),
        detail: lineage_detail(&parts),
        action_ids: Vec::new(),
        message_role: turn_id.as_ref().map(|_| role.to_string()),
        turn_id,
    }
}

/// Turn id the chat cite is derived from. Only a webapp anchor is stable
/// across reload; a time-window excerpt without one stays a quote, not a jump.
fn webapp_lineage_turn_id(entry: &crate::memory::L3Entry) -> Option<String> {
    if entry.anchor_kind.as_deref() != Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND) {
        return None;
    }
    let turn = entry
        .anchor_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if turn.chars().count() > 256 {
        return None;
    }
    Some(turn.to_string())
}

fn attach_conversation_lineage(lineage: &mut Vec<WorkLineageNode>, extra: Vec<WorkLineageNode>) {
    if extra.is_empty() {
        return;
    }
    let room = 6usize.saturating_sub(lineage.len());
    if room == 0 {
        return;
    }
    let insert_at = lineage
        .iter()
        .position(|node| node.layer == "origin")
        .unwrap_or(lineage.len());
    let mut added = 0usize;
    for node in extra {
        if added >= room {
            break;
        }
        if lineage.iter().any(|existing| existing.id == node.id) {
            continue;
        }
        lineage.insert(insert_at + added, node);
        added += 1;
    }
}

async fn attach_trigger_conversation(
    state: &AdminState,
    did: &str,
    created_at: &str,
    task_id: Option<i64>,
    lineage: &mut Vec<WorkLineageNode>,
    locale: &str,
) {
    if lineage.is_empty() {
        return;
    }
    let Some(created_at_unix) = parse_rfc3339_unix(created_at) else {
        return;
    };
    let archive = match state
        .memory
        .list_trigger_archive(did, created_at_unix, task_id)
        .await
    {
        Ok(archive) => archive,
        Err(error) => {
            tracing::debug!(%error, "trigger conversation lookup failed");
            return;
        }
    };
    let nodes = triggering_conversation_nodes(
        &archive.entries,
        created_at_unix,
        archive.exact_turn,
        locale,
    );
    attach_conversation_lineage(lineage, nodes);
}

pub(super) fn unified_inbox_item_from_task(
    task: &crate::memory::Task,
    locale: &str,
) -> UnifiedInboxItem {
    let task_closed = unified_inbox_task_status_is_closed(&task.status);
    let task_projection_kind = semantic_task_projection_kind(task);
    let blocked_owner_automation = task_is_blocked_owner_automation(task);
    let urgency = if task_closed {
        "silent"
    } else if task_projection_kind == Some("backlog") {
        "low"
    } else {
        match task.priority.as_str() {
            "urgent" => "urgent",
            "high" => "high",
            "low" => "low",
            _ => match task.status.as_str() {
                "failed" | "dead_letter" => "high",
                _ => "normal",
            },
        }
    };
    let due_at = task
        .trigger_at_unix
        .and_then(|ts| chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0))
        .map(|dt| dt.to_rfc3339())
        .or_else(|| task.due_date.clone());

    let summary = unified_inbox_task_summary(task, task_closed, locale);
    UnifiedInboxItem {
        id: format!("task:{}", task.id),
        kind: "task".into(),
        title: unified_inbox_task_public_title(task, task_closed, task_projection_kind, locale),
        title_code: None,
        summary,
        owner_visible_reason: unified_inbox_task_owner_reason(
            task,
            task_closed,
            task_projection_kind,
            locale,
        ),
        owner_visible_reason_code: Some(
            unified_inbox_task_owner_reason_code(task, task_closed, task_projection_kind).into(),
        ),
        authority_basis: "direct_request".into(),
        source: "tasks".into(),
        source_id: task.id.to_string(),
        source_event_id: task
            .event_key
            .as_deref()
            .map(|value| opaque_unified_inbox_id("task_event", value)),
        status: unified_inbox_task_public_status(task, task_closed, task_projection_kind),
        urgency: urgency.into(),
        sensitivity: "S2".into(),
        privacy_level: "owner_only".into(),
        created_at: task.created_at.clone(),
        updated_at: task.updated_at.clone(),
        due_at,
        primary_action: Some(UnifiedInboxAction {
            label: t("console.task.action.open", locale).to_string(),
            label_code: Some("open_details".into()),
            action_kind: "navigate".into(),
            route: Some(format!("/webapp/console/tasks/{}/timeline", task.id)),
            requires_gate: true,
            requires_mfa: false,
        }),
        secondary_actions: task_tracking_secondary_actions(
            task,
            task_closed,
            blocked_owner_automation,
            locale,
        ),
        next_safe_action: if blocked_owner_automation {
            blocked_owner_automation_next_safe_action(task, locale).to_string()
        } else if task_closed {
            t("console.task.next.closed", locale).to_string()
        } else {
            t("console.task.next.open", locale).to_string()
        },
        next_safe_action_code: Some(
            if blocked_owner_automation {
                blocked_owner_automation_next_safe_action_code(task)
            } else if task_closed {
                "task_closed_receipts"
            } else if task_has_private_title_or_body(task) {
                // Language-independent. The console hold must not match English prose.
                "task_private_review"
            } else {
                "task_open_or_act"
            }
            .into(),
        ),
        blocked_by: if blocked_owner_automation {
            Some(blocked_owner_automation_blocked_by(task, locale).to_string())
        } else {
            None
        },
        blocked_by_code: if blocked_owner_automation {
            Some(blocked_owner_automation_blocked_by_code(task).into())
        } else {
            None
        },
        receipt_expectation: if task_closed {
            t("console.task.receipt.closed", locale).to_string()
        } else {
            t("console.task.receipt.open", locale).to_string()
        },
        authority: None,
        detail_route: Some(format!("/webapp/console/tasks/{}/timeline", task.id)),
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: if blocked_owner_automation {
            t("console.route.task_paused", locale).to_string()
        } else {
            t("console.route.task_queue", locale).to_string()
        },
        lineage: task_tracking_lineage(task, task_closed, blocked_owner_automation, locale),
        schema_version: 1,
    }
}

fn unified_inbox_task_summary(
    task: &crate::memory::Task,
    task_closed: bool,
    locale: &str,
) -> String {
    if task_closed {
        return t("console.task.summary.closed", locale).to_string();
    }
    if task_is_blocked_owner_automation(task) {
        return blocked_owner_automation_summary(task, locale);
    }
    if task_has_private_title_or_body(task) {
        return tfp(
            "console.task.summary.private",
            locale,
            &[user_facing_private_task_priority(&task.priority, locale)],
        );
    }
    if let Some(kind) = semantic_task_projection_kind(task) {
        // Canned on purpose: list summaries never carry a task body (privacy
        // invariant pinned by admin_tests); the web row hides the generic
        // sentence and lets the timing line speak (2026-09-13 Work walk).
        return t(
            match kind {
                "tracking" => "console.task.summary.tracking",
                "scheduled" => "console.task.summary.scheduled",
                "backlog" => "console.task.summary.backlog",
                "decision" => "console.task.summary.decision",
                _ => "console.task.summary.notes",
            },
            locale,
        )
        .to_string();
    }
    let mode_hint = t(
        match task.execution_mode.as_str() {
            "scheduled" | "notification" => "console.task.mode.scheduled",
            "interactive" | "manual" => "console.task.mode.waiting",
            _ => "console.task.mode.notes",
        },
        locale,
    );
    match task.status.as_str() {
        "waiting" | "tracking" | "scheduled" | "deferred" | "snoozed" => {
            tfp("console.task.summary.follow_up", locale, &[mode_hint])
        }
        "failed" | "dead_letter" => t("console.task.summary.recovery", locale).to_string(),
        _ if task.priority == "urgent" || task.priority == "high" => tfp(
            "console.task.summary.priority",
            locale,
            &[user_facing_task_priority(&task.priority, locale), mode_hint],
        ),
        _ => tfp("console.task.summary.review", locale, &[mode_hint]),
    }
}

fn unified_inbox_task_owner_reason(
    task: &crate::memory::Task,
    task_closed: bool,
    task_projection_kind: Option<&str>,
    locale: &str,
) -> String {
    if task_closed {
        return t("console.task.reason.closed", locale).to_string();
    }
    if task_is_blocked_owner_automation(task) {
        return blocked_owner_automation_reason(task, locale).to_string();
    }
    if let Some(kind) = task_projection_kind {
        return t(
            match kind {
                "tracking" => "console.task.reason.tracking",
                "scheduled" => "console.task.reason.scheduled",
                "backlog" => "console.task.reason.backlog",
                "decision" => "console.task.reason.decision",
                _ => "console.task.reason.review",
            },
            locale,
        )
        .to_string();
    }
    t(
        match task.status.as_str() {
            "waiting" | "tracking" | "scheduled" | "deferred" | "snoozed" => {
                "console.task.reason.follow_up"
            }
            "failed" | "dead_letter" => "console.task.reason.recovery",
            _ => "console.task.reason.active",
        },
        locale,
    )
    .to_string()
}

/// Parallel of [`unified_inbox_task_owner_reason`]: one stable structural code
/// per return branch (all branches are static prose, so every branch has a
/// code). Keep the two functions' branch structures in lockstep.
fn unified_inbox_task_owner_reason_code(
    task: &crate::memory::Task,
    task_closed: bool,
    task_projection_kind: Option<&str>,
) -> &'static str {
    if task_closed {
        return "task_closed_history";
    }
    if task_is_blocked_owner_automation(task) {
        return blocked_owner_automation_reason_code(task);
    }
    if let Some(kind) = task_projection_kind {
        return match kind {
            "tracking" => "task_tracking_loop",
            "scheduled" => "task_scheduled_hold",
            "backlog" => "task_backlog_quiet",
            "decision" => "task_decision_needed",
            _ => "task_owner_review",
        };
    }
    match task.status.as_str() {
        "waiting" | "tracking" | "scheduled" | "deferred" | "snoozed" => "task_follow_up_visible",
        "failed" | "dead_letter" => "task_needs_recovery",
        _ => "task_active_owner_work",
    }
}

fn unified_inbox_task_public_title(
    task: &crate::memory::Task,
    task_closed: bool,
    task_projection_kind: Option<&str>,
    locale: &str,
) -> String {
    if !task_closed && task_is_blocked_owner_automation(task) {
        return blocked_owner_automation_title(task, locale).to_string();
    }
    let fallback = t(
        if task_closed {
            "console.task.title.closed"
        } else if task_projection_kind == Some("tracking") {
            "console.task.title.tracking"
        } else if task_projection_kind == Some("scheduled") {
            "console.task.title.scheduled"
        } else if task_projection_kind == Some("backlog") {
            "console.task.title.backlog"
        } else if task_projection_kind == Some("decision") {
            "console.task.title.decision"
        } else if task.priority == "urgent" {
            "console.task.title.urgent"
        } else if task.priority == "high" {
            "console.task.title.high"
        } else if task.status == "failed" || task.status == "dead_letter" {
            "console.task.title.recovery"
        } else {
            "console.task.title.review"
        },
        locale,
    );
    if task_has_private_title_or_body(task) {
        if task_closed {
            return t("console.task.title.closed_private", locale).to_string();
        }
        return tfp("console.task.title.private", locale, &[fallback]);
    }
    owner_visible_title(&task.title, fallback, 160)
}

fn blocked_owner_automation_title(task: &crate::memory::Task, locale: &str) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => t("console.blocked.title.web_search", locale),
        _ => t("console.blocked.title.generic", locale),
    }
}

fn blocked_owner_automation_summary(task: &crate::memory::Task, locale: &str) -> String {
    let task_label =
        owner_visible_title(&task.title, t("console.blocked.this_request", locale), 180);
    match blocked_owner_automation_kind(task) {
        "web_search" => tfp(
            "console.blocked.summary.web_search",
            locale,
            &[&owner_visible_web_search_request(&task_label)],
        ),
        _ => tfp("console.blocked.summary.generic", locale, &[&task_label]),
    }
}

fn owner_visible_web_search_request(value: &str) -> String {
    let trimmed = value.trim();
    for prefix in [
        "使用 web_search ",
        "使用 web search ",
        "web_search ",
        "web search ",
        "Use web_search to ",
        "Use web search to ",
        "use web_search to ",
        "use web search to ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    trimmed.to_string()
}

fn blocked_owner_automation_reason(task: &crate::memory::Task, locale: &str) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => t("console.blocked.reason.web_search", locale),
        _ => t("console.blocked.reason.generic", locale),
    }
}

/// Parallel of [`blocked_owner_automation_reason`] (structural code).
fn blocked_owner_automation_reason_code(task: &crate::memory::Task) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => "blocked_web_search_pause",
        _ => "blocked_generic_pause",
    }
}

fn blocked_owner_automation_next_safe_action(
    task: &crate::memory::Task,
    locale: &str,
) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => t("console.blocked.next.web_search", locale),
        _ => t("console.blocked.next.generic", locale),
    }
}

/// Parallel of [`blocked_owner_automation_next_safe_action`] (structural code).
fn blocked_owner_automation_next_safe_action_code(task: &crate::memory::Task) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => "blocked_web_search_choice",
        _ => "blocked_generic_choice",
    }
}

/// Owner-facing "what Aegis was doing" prose plus an optional structural code
/// for FE catalog overlay (`work.decide.<code>`). Returns `None` for the code
/// when the raw item summary is passed through (freeform, not a template).
pub(super) fn blocked_owner_automation_what(
    item: &UnifiedInboxItem,
    kind: &str,
    locale: &str,
) -> (String, Option<&'static str>) {
    match kind {
        "web_search" => {
            let summary = item.summary.trim();
            let generic = t("console.blocked.what.web_search.generic_summary", locale);
            if summary.is_empty()
                || summary == generic
                || summary == t("console.blocked.what.web_search.generic_summary", "en")
                || summary == t("console.blocked.what.web_search.generic_summary", "zh")
            {
                (
                    t("console.blocked.what.web_search", locale).to_string(),
                    Some("what_web_search_scheduled"),
                )
            } else {
                (summary.to_string(), None)
            }
        }
        _ => {
            let summary = item.summary.trim();
            if summary.is_empty() {
                (
                    t("console.blocked.what.generic", locale).to_string(),
                    Some("what_generic_boundary"),
                )
            } else {
                (summary.to_string(), None)
            }
        }
    }
}

fn blocked_owner_automation_blocked_by(task: &crate::memory::Task, locale: &str) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => t("console.blocked.by.web_search", locale),
        _ => t("console.blocked.by.generic", locale),
    }
}

/// Parallel of [`blocked_owner_automation_blocked_by`] (structural code).
fn blocked_owner_automation_blocked_by_code(task: &crate::memory::Task) -> &'static str {
    match blocked_owner_automation_kind(task) {
        "web_search" => "blocked_by_web_search_permission",
        _ => "blocked_by_sensitive_step",
    }
}

fn blocked_owner_automation_kind(task: &crate::memory::Task) -> &'static str {
    blocked_owner_automation_kind_from_text(&[
        &task.title,
        &task.body,
        task.context_text.as_deref().unwrap_or_default(),
        task.group_key.as_deref().unwrap_or_default(),
        &task.tags.join(" "),
    ])
}

pub(super) fn blocked_owner_automation_kind_from_text(parts: &[&str]) -> &'static str {
    let haystack = parts.join(" ").to_lowercase();
    if haystack.contains("web_search")
        || haystack.contains("web search")
        || haystack.contains("搜索")
    {
        return "web_search";
    }
    "generic"
}

/// Private-tagged tasks keep the tracking chain and drop the triggering
/// transcript, so the cite jump has nothing to point at.
pub(super) fn task_exposes_trigger_conversation(task: &crate::memory::Task) -> bool {
    !task_has_private_title_or_body(task)
}

fn task_has_private_title_or_body(task: &crate::memory::Task) -> bool {
    task.tags.iter().any(|tag| {
        matches!(
            tag.trim().to_ascii_lowercase().as_str(),
            "private" | "sensitive" | "confidential" | "personal_private"
        )
    })
}

fn unified_inbox_task_public_status(
    task: &crate::memory::Task,
    task_closed: bool,
    task_projection_kind: Option<&str>,
) -> String {
    if task_closed {
        return task.status.clone();
    }
    match task_projection_kind {
        Some("tracking") => "tracking".into(),
        Some("scheduled") => "scheduled".into(),
        Some("backlog") => "backlog".into(),
        Some("decision") => "needs_decision".into(),
        _ => task.status.clone(),
    }
}

fn semantic_task_projection_kind(task: &crate::memory::Task) -> Option<&'static str> {
    match task.source.as_str() {
        "webapp_tracking_draft" => Some("tracking"),
        "webapp_backlog_draft" => Some("backlog"),
        "webapp_decision_draft" => Some("decision"),
        _ => {
            if task.tags.iter().any(|tag| tag == "tracking") {
                Some("tracking")
            } else if task.tags.iter().any(|tag| tag == "backlog") {
                Some("backlog")
            } else if task.tags.iter().any(|tag| tag == "decision") {
                Some("decision")
            } else if task.status == "active" && task_is_background_loop(task) {
                Some("tracking")
            } else if task.status == "active" && task_is_scheduled_task(task) {
                Some("scheduled")
            } else {
                None
            }
        }
    }
}

fn task_is_scheduled_task(task: &crate::memory::Task) -> bool {
    task.trigger_at_unix.is_some() || matches!(task.trigger_type.as_deref(), Some("time"))
}

fn task_is_background_loop(task: &crate::memory::Task) -> bool {
    task.interval_seconds.is_some()
        || task.max_fires.is_some()
        || task.fire_count > 0
        || task.is_callback_origin
        || task.event_key.is_some()
        || (task.group_key.is_some() && task.execution_mode != "manual")
        || (task.execution_mode == "notify" && task.trigger_at_unix.is_none())
        || matches!(
            task.trigger_type.as_deref(),
            Some("event" | "semantic_alert")
        )
        || task.execution_mode == "llm_callback"
}

fn task_is_blocked_owner_automation(task: &crate::memory::Task) -> bool {
    task.status == "blocked_by_gate"
        && (task_is_background_loop(task)
            || task_is_scheduled_task(task)
            || matches!(
                task.execution_mode.as_str(),
                "notify" | "scheduled" | "notification" | "llm_callback"
            ))
}

fn memory_review_should_surface_to_owner(fact: &crate::memory::StaleFact) -> bool {
    fact.is_pinned && fact.days_stale >= WEBAPP_MEMORY_REVIEW_OWNER_SURFACE_DAYS
}

pub(super) fn unified_inbox_item_from_memory_review(
    fact: &crate::memory::StaleFact,
    locale: &str,
) -> UnifiedInboxItem {
    let mut secondary_actions = vec![
        UnifiedInboxAction {
            label: t("console.memory.action.keep", locale).to_string(),
            label_code: Some("keep_using".into()),
            action_kind: "memory.confirm".into(),
            route: Some(format!("/webapp/console/memory/items/{}/confirm", fact.id)),
            requires_gate: false,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: t(
                if fact.is_pinned {
                    "console.memory.action.unpin"
                } else {
                    "console.memory.action.pin"
                },
                locale,
            )
            .to_string(),
            label_code: Some(if fact.is_pinned { "unpin" } else { "pin" }.into()),
            action_kind: if fact.is_pinned {
                "memory.unpin".into()
            } else {
                "memory.pin".into()
            },
            route: Some(format!(
                "/webapp/console/memory/items/{}/{}",
                fact.id,
                if fact.is_pinned { "unpin" } else { "pin" }
            )),
            requires_gate: false,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: t("console.memory.action.wrong", locale).to_string(),
            label_code: Some("mark_wrong".into()),
            action_kind: "memory.mark_wrong".into(),
            route: Some(format!(
                "/webapp/console/memory/items/{}/mark-wrong",
                fact.id
            )),
            requires_gate: false,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: t("console.memory.action.forget", locale).to_string(),
            label_code: Some("forget".into()),
            action_kind: "memory.delete".into(),
            route: Some(format!("/webapp/console/memory/items/{}/delete", fact.id)),
            requires_gate: false,
            requires_mfa: false,
        },
    ];
    if fact.is_pinned {
        secondary_actions.swap(0, 1);
    }

    UnifiedInboxItem {
        id: format!("memory_review:{}", fact.id),
        kind: "memory_review".into(),
        title: tfp(
            "console.memory.title",
            locale,
            &[&fact.topic.chars().take(140).collect::<String>()],
        ),
        title_code: None,
        summary: tfp(
            "console.memory.summary",
            locale,
            &[&fact.days_stale.to_string()],
        ),
        owner_visible_reason: t("console.memory.reason", locale).to_string(),
        owner_visible_reason_code: Some("pinned_memory_review".into()),
        authority_basis: "manual_review".into(),
        source: "memory".into(),
        source_id: fact.id.to_string(),
        source_event_id: None,
        status: if fact.is_pinned {
            "pinned_review".into()
        } else {
            "stale_review".into()
        },
        urgency: if fact.days_stale >= 180 {
            "high".into()
        } else {
            "normal".into()
        },
        sensitivity: "S2".into(),
        privacy_level: "owner_only".into(),
        created_at: fact.created_at.clone(),
        updated_at: fact.last_accessed.clone(),
        due_at: None,
        primary_action: Some(UnifiedInboxAction {
            label: t("console.memory.action.review", locale).to_string(),
            label_code: Some("review_memory_detail".into()),
            action_kind: "navigate".into(),
            route: Some(format!("/webapp/console/memory/items/{}", fact.id)),
            requires_gate: true,
            requires_mfa: false,
        }),
        secondary_actions,
        next_safe_action: t("console.memory.next", locale).to_string(),
        next_safe_action_code: Some("memory_review_choices".into()),
        blocked_by: Some(t("console.memory.blocked", locale).to_string()),
        blocked_by_code: Some("blocked_by_memory_detail_gate".into()),
        receipt_expectation: t("console.memory.receipt", locale).to_string(),
        authority: None,
        detail_route: Some(format!("/webapp/console/memory/items/{}", fact.id)),
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.route.memory", locale).to_string(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

pub(super) fn unified_inbox_item_from_goal(
    goal: &crate::memory::Goal,
    locale: &str,
) -> UnifiedInboxItem {
    let completed = goal
        .milestones
        .iter()
        .filter(|milestone| milestone.completed)
        .count();
    let milestone_summary = if goal.milestones.is_empty() {
        t("console.goal.no_milestones", locale).to_string()
    } else {
        tfp(
            "console.goal.milestones",
            locale,
            &[&completed.to_string(), &goal.milestones.len().to_string()],
        )
    };
    let progress = tfp(
        "console.goal.progress",
        locale,
        &[&format!("{}", goal.progress_pct.round())],
    );
    let target = goal
        .target_date
        .as_ref()
        .map(|date| tfp("console.goal.target", locale, &[date]))
        .unwrap_or_default();
    let updated_at = goal
        .last_progress_at
        .as_ref()
        .unwrap_or(&goal.created_at)
        .clone();

    UnifiedInboxItem {
        id: format!("goal:{}", goal.id),
        kind: "goal".into(),
        title: owner_visible_title(&goal.title, t("console.goal.fallback", locale), 180),
        title_code: None,
        summary: format!("{progress} {milestone_summary}{target}"),
        owner_visible_reason: t("console.goal.reason", locale).to_string(),
        owner_visible_reason_code: Some(
            if goal.milestones.is_empty() && goal.progress_pct.round() <= 0.0 {
                GOAL_NO_PROGRESS_REASON_CODE
            } else {
                "active_goal_ongoing"
            }
            .into(),
        ),
        authority_basis: "direct_request".into(),
        source: "goals".into(),
        source_id: goal.id.to_string(),
        source_event_id: None,
        status: "tracking".into(),
        urgency: "normal".into(),
        sensitivity: "S2".into(),
        privacy_level: "owner_only".into(),
        created_at: goal.created_at.clone(),
        updated_at,
        due_at: goal.target_date.clone(),
        primary_action: None,
        secondary_actions: vec![UnifiedInboxAction {
            label: t("console.tracking.action.end_once", locale).into(),
            label_code: Some("end_this_once".into()),
            action_kind: "goal.abandon".into(),
            route: Some(format!("/webapp/vnext/work/goal:{}/actions", goal.id)),
            requires_gate: false,
            requires_mfa: false,
        }],
        next_safe_action: t("console.goal.next", locale).to_string(),
        next_safe_action_code: Some("goal_next_step".into()),
        blocked_by: None,
        blocked_by_code: None,
        receipt_expectation: t("console.goal.receipt", locale).to_string(),
        authority: None,
        detail_route: None,
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.route.goal", locale).to_string(),
        lineage: goal_tracking_lineage(goal, locale),
        schema_version: 1,
    }
}

fn goal_tracking_lineage(goal: &crate::memory::Goal, locale: &str) -> Vec<WorkLineageNode> {
    let mut parts = vec![
        format!("goal:{}", goal.id),
        format!("progress_pct={}", goal.progress_pct.round() as i64),
        format!("source={}", goal.source.as_str()),
    ];
    if !goal.milestones.is_empty() {
        let completed = goal
            .milestones
            .iter()
            .filter(|milestone| milestone.completed)
            .count();
        parts.push(format!("milestones={completed}/{}", goal.milestones.len()));
    }
    let mut nodes = vec![WorkLineageNode {
        layer: "occurrence".into(),
        kind: "goal".into(),
        id: format!("goal:{}", goal.id),
        label: t("console.lineage.occurrence.goal", locale).to_string(),
        label_code: Some("occurrence.goal".into()),
        detail: lineage_detail(&parts),
        action_ids: vec!["goal.abandon".into()],
        turn_id: None,
        message_role: None,
    }];
    if let Some(parent_id) = goal.parent_goal_id {
        nodes.push(WorkLineageNode {
            layer: "intent".into(),
            kind: "goal".into(),
            id: format!("goal:{parent_id}"),
            label: t("console.lineage.intent.parent_goal", locale).to_string(),
            label_code: Some("intent.parent_goal".into()),
            detail: lineage_detail(&[format!("parent_goal_id={parent_id}")]),
            action_ids: Vec::new(),
            turn_id: None,
            message_role: None,
        });
    }
    nodes.push(WorkLineageNode {
        layer: "origin".into(),
        kind: "record".into(),
        id: format!("goal:{}:origin", goal.id),
        label: t("console.lineage.origin.record", locale).to_string(),
        label_code: Some("origin.record".into()),
        detail: lineage_detail(&[
            format!("created_at={}", owner_visible_inline(&goal.created_at, 40)),
            format!("source={}", goal.source.as_str()),
        ]),
        action_ids: Vec::new(),
        turn_id: None,
        message_role: None,
    });
    nodes
}

pub(super) fn unified_inbox_item_from_waiting_knowledge(
    entry: &crate::memory::KnowledgeEntry,
    locale: &str,
) -> UnifiedInboxItem {
    let wait_target = entry
        .waiting_on
        .as_ref()
        .map(|value| owner_visible_title(value, t("console.waiting.dependency", locale), 120))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| t("console.waiting.dependency", locale).to_string());
    let due = entry
        .due_date
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| tfp("console.waiting.due", locale, &[value]))
        .unwrap_or_default();
    let summary = if entry
        .waiting_on
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        tfp("console.waiting.summary.on", locale, &[&wait_target, &due])
    } else {
        tfp("console.waiting.summary.due", locale, &[&due])
    };
    let has_wait_target = entry
        .waiting_on
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let owner_visible_reason = if has_wait_target {
        tfp("console.waiting.reason.on", locale, &[&wait_target])
    } else {
        t("console.waiting.reason.due", locale).to_string()
    };
    // i18n(prose-only): the wait-target branch interpolates a dynamic value
    // (`wait_target`), so it carries no structural code — prose is the surface.
    let owner_visible_reason_code = if has_wait_target {
        None
    } else {
        Some("waiting_due_date_visible".to_string())
    };
    let next_safe_action = if has_wait_target {
        tfp("console.waiting.next.on", locale, &[&wait_target])
    } else {
        t("console.waiting.next.due", locale).to_string()
    };
    // i18n(prose-only): same dynamic-`wait_target` branch as above.
    let next_safe_action_code = if has_wait_target {
        None
    } else {
        Some("waiting_review_due".to_string())
    };

    UnifiedInboxItem {
        id: format!("kb_waiting:{}", entry.id),
        kind: "waiting_for".into(),
        title: owner_visible_title(&entry.title, t("console.waiting.title", locale), 180),
        title_code: None,
        summary,
        owner_visible_reason,
        owner_visible_reason_code,
        authority_basis: "manual_review".into(),
        source: "knowledge".into(),
        source_id: entry.id.to_string(),
        source_event_id: entry.source_msg_id.map(|id| id.to_string()),
        status: "waiting".into(),
        urgency: match entry.priority.as_str() {
            "urgent" => "urgent",
            "high" => "high",
            "low" => "low",
            _ => "normal",
        }
        .into(),
        sensitivity: "S2".into(),
        privacy_level: "owner_only".into(),
        created_at: entry.created_at.clone(),
        updated_at: entry.updated_at.clone(),
        due_at: entry.due_date.clone(),
        primary_action: None,
        secondary_actions: vec![
            UnifiedInboxAction {
                label: t("console.tracking.action.end_once", locale).to_string(),
                label_code: Some("end_this_once".into()),
                action_kind: "knowledge.done".into(),
                route: Some(format!(
                    "/webapp/console/knowledge/items/{}/action",
                    entry.id
                )),
                requires_gate: false,
                requires_mfa: false,
            },
            UnifiedInboxAction {
                label: t("console.waiting.action.tomorrow", locale).to_string(),
                label_code: Some("review_tomorrow".into()),
                action_kind: "knowledge.snooze_tomorrow".into(),
                route: Some(format!(
                    "/webapp/console/knowledge/items/{}/action",
                    entry.id
                )),
                requires_gate: false,
                requires_mfa: false,
            },
        ],
        next_safe_action,
        next_safe_action_code,
        blocked_by: None,
        blocked_by_code: None,
        receipt_expectation: t("console.waiting.receipt", locale).to_string(),
        authority: None,
        detail_route: None,
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.route.waiting", locale).to_string(),
        lineage: waiting_knowledge_lineage(entry, locale),
        schema_version: 1,
    }
}

fn waiting_knowledge_lineage(
    entry: &crate::memory::KnowledgeEntry,
    locale: &str,
) -> Vec<WorkLineageNode> {
    let mut parts = vec![
        format!("kb_waiting:{}", entry.id),
        "category=waiting_for".to_string(),
        format!("knowledge_id={}", entry.id),
    ];
    if let Some(waiting_on) = entry
        .waiting_on
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!(
            "waiting_on={}",
            owner_visible_inline(waiting_on, 80)
        ));
    }
    if let Some(due) = entry
        .due_date
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("due_date={}", owner_visible_inline(due, 40)));
    }
    let mut nodes = vec![WorkLineageNode {
        layer: "occurrence".into(),
        kind: "knowledge".into(),
        id: format!("kb_waiting:{}", entry.id),
        label: t("console.lineage.occurrence.waiting", locale).to_string(),
        label_code: Some("occurrence.waiting".into()),
        detail: lineage_detail(&parts),
        action_ids: vec!["knowledge.done".into()],
        turn_id: None,
        message_role: None,
    }];
    if let Some(message_id) = entry.source_msg_id {
        let mut origin = vec![format!("source_msg_id={message_id}")];
        if let Some(summary) = entry
            .source_summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            origin.push(format!(
                "source_summary={}",
                owner_visible_inline(summary, 160)
            ));
        }
        nodes.push(WorkLineageNode {
            layer: "origin".into(),
            kind: "message".into(),
            id: format!("message:{message_id}"),
            label: t("console.lineage.origin.message", locale).to_string(),
            label_code: Some("origin.message".into()),
            detail: lineage_detail(&origin),
            action_ids: Vec::new(),
            turn_id: None,
            message_role: None,
        });
    } else {
        nodes.push(WorkLineageNode {
            layer: "origin".into(),
            kind: "record".into(),
            id: format!("kb_waiting:{}:origin", entry.id),
            label: t("console.lineage.origin.record", locale).to_string(),
            label_code: Some("origin.record".into()),
            detail: lineage_detail(&[format!(
                "created_at={}",
                owner_visible_inline(&entry.created_at, 40)
            )]),
            action_ids: Vec::new(),
            turn_id: None,
            message_role: None,
        });
    }
    nodes
}

fn waiting_knowledge_should_surface_to_owner(entry: &crate::memory::KnowledgeEntry) -> bool {
    let has_wait_target = entry
        .waiting_on
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_due_date = entry
        .due_date
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if !has_wait_target && !has_due_date {
        return false;
    }
    !waiting_knowledge_is_ambiguous_rollup(entry)
}

fn waiting_knowledge_is_ambiguous_rollup(entry: &crate::memory::KnowledgeEntry) -> bool {
    let title = entry.title.trim();
    let waiting_on = entry.waiting_on.as_deref().unwrap_or("").trim();
    let combined = format!("{title} {waiting_on}").to_lowercase();
    if title.contains('：') && waiting_knowledge_has_multiple_topics(title) {
        return true;
    }
    if waiting_knowledge_has_multiple_topics(waiting_on) {
        return true;
    }
    combined.contains("日终状态")
        || combined.contains("evening status")
        || combined.contains("daily status")
        || combined.contains("status rollup")
        || combined.contains("status summary")
        || combined.contains("fuel exhausted")
        || combined.contains("fuelexhausted")
        || combined.contains("断路器")
        || combined.contains("circuit breaker")
}

fn waiting_knowledge_has_multiple_topics(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let separator_count = trimmed.matches('/').count()
        + trimmed.matches('+').count()
        + trimmed.matches('；').count()
        + trimmed.matches(';').count()
        + trimmed.matches(" / ").count();
    separator_count >= 2
}

pub(super) fn owner_visible_title(value: &str, fallback: &str, max_chars: usize) -> String {
    let cleaned = owner_visible_title_text(value);
    if cleaned.is_empty() {
        return fallback.to_string();
    }
    let mut out = String::new();
    let mut truncated = false;
    for (index, ch) in cleaned.chars().enumerate() {
        if index >= max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
    }
    if truncated {
        out.push_str("...");
    }
    out
}

fn owner_visible_title_text(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.contains("://") {
        return collapsed;
    }

    let chars: Vec<char> = collapsed.chars().collect();
    let mut out = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '+' {
            let prev = index.checked_sub(1).and_then(|prev| chars.get(prev));
            let next = chars.get(index + 1);
            let adjacent_plus = prev == Some(&'+') || next == Some(&'+');
            if !adjacent_plus {
                if next.is_none() {
                    continue;
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                if !out.is_empty() && !out.ends_with('/') {
                    out.push_str(" / ");
                }
                continue;
            }
        }
        out.push(*ch);
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn user_facing_task_priority(priority: &str, locale: &str) -> &'static str {
    match priority {
        "urgent" => t("console.task.priority.urgent", locale),
        "high" => t("console.task.priority.high", locale),
        "low" => t("console.task.priority.low", locale),
        _ => t("console.task.priority.normal", locale),
    }
}

fn user_facing_private_task_priority(priority: &str, locale: &str) -> &'static str {
    match priority {
        "urgent" => t("console.task.priority.private.urgent", locale),
        "high" => t("console.task.priority.private.high", locale),
        "low" => t("console.task.priority.private.low", locale),
        _ => t("console.task.priority.private.normal", locale),
    }
}

pub(super) fn unified_inbox_task_status_is_closed(status: &str) -> bool {
    matches!(
        status,
        "done" | "completed" | "archived" | "cancelled" | "fired" | "expired" | "deleted"
    )
}

pub(super) fn unified_inbox_item_from_ledger_entry(
    entry: &crate::action_ledger::ActionLedgerEntry,
    locale: &str,
) -> UnifiedInboxItem {
    let reviewable_draft = unified_inbox_ledger_entry_is_reviewable_draft(entry);
    // Prose and its structural code picked ONCE, so the pair cannot drift.
    let blocked_state: Option<(&'static str, &'static str)> = if reviewable_draft {
        Some((
            t("console.receipt.blocked.draft", locale),
            "blocked_by_draft_execution_disabled",
        ))
    } else if trust_console_receipt_status(entry) == "Missing" {
        Some((
            t("console.receipt.blocked.missing", locale),
            "blocked_by_receipt_missing",
        ))
    } else {
        None
    };
    let status = unified_inbox_ledger_entry_status(entry).to_string();
    let draft_text = if reviewable_draft {
        action_draft_entry_draft_text(entry)
    } else {
        None
    };
    let urgency = match entry.status {
        crate::action_ledger::ActionStatus::Failed => "high",
        crate::action_ledger::ActionStatus::Proposed
        | crate::action_ledger::ActionStatus::PendingApproval => "high",
        crate::action_ledger::ActionStatus::Approved => "normal",
        crate::action_ledger::ActionStatus::Executing => "normal",
        crate::action_ledger::ActionStatus::Executed => {
            if trust_console_receipt_status(entry) == "Missing" {
                "high"
            } else {
                "low"
            }
        }
        _ => "silent",
    };
    let updated_at = entry
        .failed_at
        .as_ref()
        .or(entry.executed_at.as_ref())
        .or(entry.executing_at.as_ref())
        .or(entry.approved_at.as_ref())
        .or(entry.proposed_at.as_ref())
        .unwrap_or(&entry.created_at)
        .clone();
    let source_event_id = entry
        .assistant_event_id
        .map(|id| id.to_string())
        .or_else(|| {
            entry
                .source_event_id
                .as_deref()
                .map(|value| opaque_unified_inbox_id("source_event", value))
        });

    UnifiedInboxItem {
        id: format!("receipt:{}", entry.id),
        kind: if reviewable_draft {
            "action_draft".into()
        } else {
            "receipt".into()
        },
        title: unified_inbox_ledger_entry_title(entry, draft_text.as_deref(), locale),
        title_code: None,
        summary: if reviewable_draft {
            user_facing_action_draft_summary(entry, draft_text.as_deref(), locale)
        } else {
            user_facing_receipt_summary(entry, locale)
        },
        owner_visible_reason: if reviewable_draft {
            t("console.receipt.reason.draft", locale).to_string()
        } else {
            t("console.receipt.reason.inspect", locale).to_string()
        },
        owner_visible_reason_code: Some(
            if reviewable_draft {
                "draft_needs_confirmation"
            } else {
                "action_state_inspect"
            }
            .into(),
        ),
        authority_basis: "direct_request".into(),
        source: "action_ledger".into(),
        source_id: entry.id.to_string(),
        source_event_id,
        status,
        urgency: urgency.into(),
        sensitivity: "S2".into(),
        privacy_level: "owner_only".into(),
        created_at: entry.created_at.clone(),
        updated_at,
        due_at: None,
        primary_action: Some(UnifiedInboxAction {
            label: if reviewable_draft {
                t("console.receipt.action.review_draft", locale).to_string()
            } else {
                t("console.receipt.action.open", locale).to_string()
            },
            label_code: Some(
                if reviewable_draft {
                    "review_draft"
                } else {
                    "open_receipt"
                }
                .into(),
            ),
            action_kind: "navigate".into(),
            route: Some(format!("/webapp/console/ledger/{}", entry.id)),
            requires_gate: true,
            requires_mfa: false,
        }),
        secondary_actions: unified_inbox_ledger_entry_secondary_actions(entry),
        next_safe_action: if reviewable_draft {
            t(
                match entry.action_kind {
                    crate::action_ledger::ActionKind::ReminderCreate => {
                        "console.receipt.next.reminder"
                    }
                    crate::action_ledger::ActionKind::TaskCreate => "console.receipt.next.task",
                    _ => "console.receipt.next.draft",
                },
                locale,
            )
            .to_string()
        } else if trust_console_receipt_status(entry) == "Missing" {
            t("console.receipt.next.missing", locale).to_string()
        } else {
            t("console.receipt.next.verify", locale).to_string()
        },
        next_safe_action_code: Some(
            if reviewable_draft {
                match entry.action_kind {
                    crate::action_ledger::ActionKind::ReminderCreate => "draft_reminder_review",
                    crate::action_ledger::ActionKind::TaskCreate => "draft_task_review",
                    _ => "draft_generic_review",
                }
            } else if trust_console_receipt_status(entry) == "Missing" {
                "receipt_missing_verify"
            } else {
                "receipt_verify"
            }
            .into(),
        ),
        blocked_by: blocked_state.map(|(prose, _)| prose.to_string()),
        blocked_by_code: blocked_state.map(|(_, code)| code.to_string()),
        receipt_expectation: if reviewable_draft {
            t("console.receipt.expect.draft", locale).to_string()
        } else if trust_console_receipt_status(entry) == "Missing" {
            t("console.receipt.expect.missing", locale).to_string()
        } else {
            t("console.receipt.expect.verify", locale).to_string()
        },
        authority: None,
        detail_route: Some(format!("/webapp/console/ledger/{}", entry.id)),
        external_url_policy: "none".into(),
        related_action_ids: vec![format!("action:{}", entry.id)],
        related_receipt_ids: vec![format!("receipt:{}", entry.id)],
        route_reason: t("console.route.receipt", locale).to_string(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

fn ledger_entry_is_groupable_completed_receipt(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> bool {
    entry.status == crate::action_ledger::ActionStatus::Executed
        && trust_console_receipt_status(entry) == "Present"
}

fn ledger_entry_timestamp(entry: &crate::action_ledger::ActionLedgerEntry) -> &str {
    entry
        .executed_at
        .as_deref()
        .or(entry.failed_at.as_deref())
        .or(entry.executing_at.as_deref())
        .unwrap_or(&entry.created_at)
}

pub(super) fn parse_work_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    crate::utils::parse_timestamp_dual_fmt(value).or_else(|| {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
            .ok()
            .map(|ndt| ndt.and_utc())
    })
}

fn ledger_entry_utc_day(entry: &crate::action_ledger::ActionLedgerEntry) -> String {
    parse_work_timestamp(ledger_entry_timestamp(entry))
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ledger_entry_timestamp(entry).chars().take(10).collect())
}

pub(super) fn is_grouped_receipt_item(item: &UnifiedInboxItem) -> bool {
    item.kind == "receipt" && item.id.starts_with("receipt:day:")
}

pub(super) fn project_ledger_entries_into_inbox(
    entries: &[crate::action_ledger::ActionLedgerEntry],
    locale: &str,
) -> Vec<UnifiedInboxItem> {
    let mut completed_by_day: std::collections::BTreeMap<
        String,
        Vec<&crate::action_ledger::ActionLedgerEntry>,
    > = std::collections::BTreeMap::new();
    let mut items = Vec::new();
    for entry in entries {
        if !vnext_should_project_action_ledger_entry(entry) {
            continue;
        }
        if ledger_entry_is_groupable_completed_receipt(entry) {
            completed_by_day
                .entry(ledger_entry_utc_day(entry))
                .or_default()
                .push(entry);
        } else {
            items.push(unified_inbox_item_from_ledger_entry(entry, locale));
        }
    }
    for (day, mut day_entries) in completed_by_day.into_iter().rev() {
        day_entries.sort_by(|a, b| ledger_entry_timestamp(b).cmp(ledger_entry_timestamp(a)));
        if let Some(grouped) = unified_inbox_grouped_receipt_item(&day, &day_entries, locale) {
            items.push(grouped);
        }
    }
    items
}

fn unified_inbox_grouped_receipt_item(
    day: &str,
    entries: &[&crate::action_ledger::ActionLedgerEntry],
    locale: &str,
) -> Option<UnifiedInboxItem> {
    let first = entries.first().copied()?;
    let mut item = unified_inbox_item_from_ledger_entry(first, locale);
    let count = entries.len().to_string();
    let first_title = owner_visible_title(&item.title, t("console.item.work_fallback", locale), 40)
        .trim_end_matches('…')
        .trim_end_matches("...")
        .trim()
        .to_string();
    item.id = format!("receipt:day:{day}");
    item.kind = "receipt".into();
    item.title = tfp(
        "console.work.receipts.grouped",
        locale,
        &[&count, &first_title],
    );
    item.title_code = None;
    item.summary = tfp("console.work.receipts.grouped.summary", locale, &[&count]);
    item.owner_visible_reason = t("console.work.receipts.grouped.reason", locale).to_string();
    item.owner_visible_reason_code = Some("receipt_day_group".into());
    item.status = "executed".into();
    item.urgency = "low".into();
    item.source_id = first.id.to_string();
    item.related_action_ids = entries
        .iter()
        .map(|entry| format!("action:{}", entry.id))
        .collect();
    item.related_receipt_ids = entries
        .iter()
        .map(|entry| format!("receipt:{}", entry.id))
        .collect();
    item.detail_route = Some(format!("/webapp/console/ledger/{}", first.id));
    if let Some(action) = item.primary_action.as_mut() {
        action.route = Some(format!("/webapp/console/ledger/{}", first.id));
    }
    Some(item)
}

fn item_age_older_than(
    item: &UnifiedInboxItem,
    now: chrono::DateTime<chrono::Utc>,
    days: i64,
) -> bool {
    parse_work_timestamp(&item.updated_at)
        .is_some_and(|updated| now.signed_duration_since(updated) > chrono::Duration::days(days))
}

/// Structural marker set by `unified_inbox_item_from_goal` for a goal with 0 %
/// progress and no milestone checklist; staleness never parses localized copy.
pub(super) const GOAL_NO_PROGRESS_REASON_CODE: &str = "active_goal_no_progress";
/// Structural marker set once an item has been parked as stale.
pub(super) const STALE_PARKED_REASON_CODE: &str = "follow_up_stale_parked";

fn goal_has_zero_progress_and_no_checklist(item: &UnifiedInboxItem) -> bool {
    item.kind == "goal"
        && item.owner_visible_reason_code.as_deref() == Some(GOAL_NO_PROGRESS_REASON_CODE)
}

fn item_is_tracking_follow_up(item: &UnifiedInboxItem) -> bool {
    item.kind != "goal" && (item.kind == "waiting_for" || vnext_source_kind(item) == "tracking")
}

fn item_is_fired_task_reminder(item: &UnifiedInboxItem) -> bool {
    item.status == "fired" && matches!(item.kind.as_str(), "task" | "reminder")
}

pub(super) fn follow_up_is_stale(
    item: &UnifiedInboxItem,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if item.owner_visible_reason_code.as_deref() == Some(STALE_PARKED_REASON_CODE) {
        return true;
    }
    if goal_has_zero_progress_and_no_checklist(item) {
        return item_age_older_than(item, now, 30);
    }
    if item_is_tracking_follow_up(item) {
        return item_age_older_than(item, now, 14);
    }
    if item_is_fired_task_reminder(item) {
        return item_age_older_than(item, now, 7);
    }
    false
}

pub(super) fn stale_follow_up_reason(item: &UnifiedInboxItem, locale: &str) -> String {
    let key = if item.kind == "goal" {
        "console.work.stale.goal"
    } else if item_is_fired_task_reminder(item) {
        "console.work.stale.fired"
    } else {
        "console.work.stale.tracking"
    };
    t(key, locale).to_string()
}

fn apply_stale_follow_up_reasons(
    items: &mut [UnifiedInboxItem],
    now: chrono::DateTime<chrono::Utc>,
    locale: &str,
) {
    for item in items {
        if follow_up_is_stale(item, now) {
            item.owner_visible_reason = stale_follow_up_reason(item, locale);
            item.owner_visible_reason_code = Some(STALE_PARKED_REASON_CODE.into());
        }
    }
}

fn normalize_work_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn strip_duplicate_meta_suffix(value: &str) -> &str {
    match value.rsplit_once('×') {
        Some((head, digits))
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) =>
        {
            head.trim_end()
        }
        _ => value,
    }
}

fn collapsed_work_ids(item: &UnifiedInboxItem) -> Vec<String> {
    if item.related_action_ids.iter().any(|id| id == &item.id) {
        item.related_action_ids.clone()
    } else {
        vec![item.id.clone()]
    }
}

fn merge_duplicate_task_into(keep: &mut UnifiedInboxItem, other: UnifiedInboxItem, locale: &str) {
    let mut ids = collapsed_work_ids(keep);
    if !ids.contains(&other.id) {
        ids.push(other.id);
    }
    keep.related_action_ids = ids.clone();
    for receipt_id in other.related_receipt_ids {
        if !keep.related_receipt_ids.contains(&receipt_id) {
            keep.related_receipt_ids.push(receipt_id);
        }
    }
    if other.updated_at > keep.updated_at {
        keep.updated_at = other.updated_at;
    }
    let count = ids.len();
    if count > 1 {
        let meta = tfp("console.work.duplicate.meta", locale, &[&count.to_string()]);
        let base_summary = strip_duplicate_meta_suffix(&keep.summary);
        keep.summary = format!("{base_summary} {meta}");
    }
}

fn collapse_duplicate_lane_tasks(
    items: Vec<UnifiedInboxItem>,
    now: chrono::DateTime<chrono::Utc>,
    locale: &str,
) -> Vec<UnifiedInboxItem> {
    let mut out = Vec::new();
    let mut index_by_key: HashMap<(String, String), usize> = HashMap::new();
    for item in items {
        if !matches!(item.kind.as_str(), "task" | "reminder") {
            out.push(item);
            continue;
        }
        let normalized = normalize_work_title(&item.title);
        if normalized.is_empty() {
            out.push(item);
            continue;
        }
        let lane = vnext_lane_for_item_at(&item, now);
        let key = (lane.to_string(), normalized);
        if let Some(&idx) = index_by_key.get(&key) {
            merge_duplicate_task_into(&mut out[idx], item, locale);
        } else {
            index_by_key.insert(key, out.len());
            out.push(item);
        }
    }
    out
}

pub(super) fn denoise_work_inbox_items(
    mut items: Vec<UnifiedInboxItem>,
    now: chrono::DateTime<chrono::Utc>,
    locale: &str,
) -> Vec<UnifiedInboxItem> {
    apply_stale_follow_up_reasons(&mut items, now, locale);
    collapse_duplicate_lane_tasks(items, now, locale)
}

fn unified_inbox_ledger_entry_secondary_actions(
    _entry: &crate::action_ledger::ActionLedgerEntry,
) -> Vec<UnifiedInboxAction> {
    Vec::new()
}

pub(super) fn retired_webapp_action_draft_entry(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> bool {
    if entry.action_id.starts_with("webapp-action-draft:") {
        return true;
    }
    entry
        .metadata_json
        .as_deref()
        .and_then(|metadata| serde_json::from_str::<serde_json::Value>(metadata).ok())
        .and_then(|value| {
            value
                .get("resource_type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|resource_type| resource_type == "contextual_action_draft")
}

pub(super) fn vnext_should_project_action_ledger_entry(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> bool {
    if retired_webapp_action_draft_entry(entry) {
        return false;
    }

    // Pending decisions belong to the canonical HITL queue. Projecting raw
    // Action Ledger proposals made ordinary reminders/tasks look like fake
    // approval work in the owner-facing WebApp.
    matches!(
        entry.status,
        crate::action_ledger::ActionStatus::Executing
            | crate::action_ledger::ActionStatus::Executed
            | crate::action_ledger::ActionStatus::Failed
    )
}

fn action_draft_entry_suggested_intent_kind(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> Option<String> {
    let metadata = entry.metadata_json.as_deref()?;
    let value = serde_json::from_str::<serde_json::Value>(metadata).ok()?;
    value
        .get("suggested_intent_kind")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn action_draft_entry_draft_text(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> Option<String> {
    let metadata = entry.metadata_json.as_deref()?;
    let value = serde_json::from_str::<serde_json::Value>(metadata).ok()?;
    let general_assistant_focus = value
        .get("focus_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|focus_id| focus_id == GENERAL_ASSISTANT_FOCUS_ID)
        || value
            .get("focus_kind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|focus_kind| focus_kind == "assistant");
    if !general_assistant_focus {
        return None;
    }
    action_draft_metadata_draft_text(&value)
}

fn unified_inbox_ledger_entry_is_reviewable_draft(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> bool {
    matches!(
        entry.status,
        crate::action_ledger::ActionStatus::Proposed
            | crate::action_ledger::ActionStatus::PendingApproval
            | crate::action_ledger::ActionStatus::Approved
    )
}

fn unified_inbox_ledger_entry_status(
    entry: &crate::action_ledger::ActionLedgerEntry,
) -> &'static str {
    match entry.status {
        crate::action_ledger::ActionStatus::Proposed => "needs_decision",
        crate::action_ledger::ActionStatus::PendingApproval => "approval_required",
        crate::action_ledger::ActionStatus::Approved => "approved",
        _ => trust_console_action_status_label(entry),
    }
}

fn unified_inbox_ledger_entry_title(
    entry: &crate::action_ledger::ActionLedgerEntry,
    draft_text: Option<&str>,
    locale: &str,
) -> String {
    let subject = ledger_entry_subject(entry, draft_text, locale);
    if unified_inbox_ledger_entry_is_reviewable_draft(entry) {
        if entry.action_kind == crate::action_ledger::ActionKind::EmailDraft && draft_text.is_none()
        {
            return t("console.receipt.draft.title.email", locale).to_string();
        }
        return match entry.action_kind {
            crate::action_ledger::ActionKind::ReminderCreate => {
                tfp("console.receipt.draft.title.reminder", locale, &[&subject])
            }
            crate::action_ledger::ActionKind::TaskCreate => {
                let key = match action_draft_entry_suggested_intent_kind(entry).as_deref() {
                    Some("tracking") => "console.receipt.draft.title.tracking",
                    Some("backlog") => "console.receipt.draft.title.backlog",
                    Some("decision") => "console.receipt.draft.title.decision",
                    _ => "console.receipt.draft.title.task",
                };
                tfp(key, locale, &[&subject])
            }
            _ => tfp("console.receipt.draft.title.generic", locale, &[&subject]),
        };
    }
    let kind_label = localized_action_kind_label(entry.action_kind, locale);
    match entry.status {
        crate::action_ledger::ActionStatus::Proposed => {
            tfp("console.receipt.title.review", locale, &[kind_label])
        }
        crate::action_ledger::ActionStatus::PendingApproval => {
            tfp("console.receipt.title.approve", locale, &[kind_label])
        }
        crate::action_ledger::ActionStatus::Approved => {
            tfp("console.receipt.title.approved", locale, &[kind_label])
        }
        crate::action_ledger::ActionStatus::Failed => {
            tfp("console.receipt.title.failed", locale, &[&subject])
        }
        crate::action_ledger::ActionStatus::Executing => {
            tfp("console.receipt.title.running", locale, &[&subject])
        }
        crate::action_ledger::ActionStatus::Executed => {
            if trust_console_receipt_status(entry) == "Missing" {
                tfp("console.receipt.title.verify", locale, &[&subject])
            } else {
                match entry.action_kind {
                    crate::action_ledger::ActionKind::ReminderCreate => {
                        tfp("console.receipt.title.reminder_sent", locale, &[&subject])
                    }
                    crate::action_ledger::ActionKind::TelegramSend => {
                        tfp("console.receipt.title.notice_sent", locale, &[&subject])
                    }
                    crate::action_ledger::ActionKind::EmailSend => {
                        tfp("console.receipt.title.email_sent", locale, &[&subject])
                    }
                    crate::action_ledger::ActionKind::TaskCreate => {
                        tfp("console.receipt.title.task_saved", locale, &[&subject])
                    }
                    crate::action_ledger::ActionKind::CalendarCreate => {
                        tfp("console.receipt.title.calendar_added", locale, &[&subject])
                    }
                    _ => tfp("console.receipt.title.done", locale, &[&subject]),
                }
            }
        }
        _ => tfp("console.receipt.title.done", locale, &[&subject]),
    }
}

fn ledger_entry_subject(
    entry: &crate::action_ledger::ActionLedgerEntry,
    draft_text: Option<&str>,
    locale: &str,
) -> String {
    let fallback = t("console.receipt.untitled", locale);
    if let Some(draft_text) = draft_text {
        return owner_visible_title(
            &action_draft_task_title(draft_text, action_draft_title_prefixes(entry.action_kind)),
            fallback,
            96,
        );
    }
    localized_action_kind_label(entry.action_kind, locale).to_string()
}

fn localized_action_kind_label(
    kind: crate::action_ledger::ActionKind,
    locale: &str,
) -> &'static str {
    match kind {
        crate::action_ledger::ActionKind::TelegramSend => t("console.receipt.kind.notice", locale),
        crate::action_ledger::ActionKind::EmailSend => t("console.receipt.kind.email", locale),
        crate::action_ledger::ActionKind::EmailDraft => {
            t("console.receipt.kind.email_draft", locale)
        }
        crate::action_ledger::ActionKind::ReminderCreate
        | crate::action_ledger::ActionKind::ReminderUpdate
        | crate::action_ledger::ActionKind::ReminderDelete => {
            t("console.receipt.kind.reminder", locale)
        }
        crate::action_ledger::ActionKind::TaskCreate => t("console.receipt.kind.task", locale),
        crate::action_ledger::ActionKind::CalendarCreate
        | crate::action_ledger::ActionKind::CalendarUpdate
        | crate::action_ledger::ActionKind::CalendarDelete => {
            t("console.receipt.kind.calendar", locale)
        }
        crate::action_ledger::ActionKind::KbMerge
        | crate::action_ledger::ActionKind::KbUpdate
        | crate::action_ledger::ActionKind::KbDelete => t("console.receipt.kind.memory", locale),
        _ => t("console.receipt.kind.generic", locale),
    }
}

fn action_draft_title_prefixes(kind: crate::action_ledger::ActionKind) -> &'static [&'static str] {
    match kind {
        crate::action_ledger::ActionKind::ReminderCreate => {
            &["Reminder:", "Reminder -", "提醒：", "提醒:"]
        }
        crate::action_ledger::ActionKind::TaskCreate => {
            &["Task:", "Task -", "Todo:", "To-do:", "任务：", "任务:"]
        }
        _ => &["Draft:", "Draft -", "草稿：", "草稿:"],
    }
}

pub(super) fn user_facing_action_draft_summary(
    entry: &crate::action_ledger::ActionLedgerEntry,
    draft_text: Option<&str>,
    locale: &str,
) -> String {
    if draft_text.is_none() && entry.action_kind == crate::action_ledger::ActionKind::EmailDraft {
        return t("console.receipt.draft.summary.email", locale).to_string();
    }
    match entry.action_kind {
        crate::action_ledger::ActionKind::ReminderCreate => {
            t("console.receipt.draft.summary.reminder", locale).to_string()
        }
        crate::action_ledger::ActionKind::TaskCreate => {
            let key = match action_draft_entry_suggested_intent_kind(entry).as_deref() {
                Some("tracking") => "console.receipt.draft.summary.tracking",
                Some("backlog") => "console.receipt.draft.summary.backlog",
                Some("decision") => "console.receipt.draft.summary.decision",
                _ => "console.receipt.draft.summary.task",
            };
            t(key, locale).to_string()
        }
        _ => {
            let draft_kind = if draft_text.is_some() {
                t("console.receipt.kind.draft_action", locale)
            } else {
                localized_action_kind_label(entry.action_kind, locale)
            };
            tfp(
                "console.receipt.draft.summary.generic",
                locale,
                &[draft_kind],
            )
        }
    }
}

fn user_facing_receipt_summary(
    entry: &crate::action_ledger::ActionLedgerEntry,
    locale: &str,
) -> String {
    let kind = localized_action_kind_label(entry.action_kind, locale);
    match entry.status {
        crate::action_ledger::ActionStatus::Proposed => {
            tfp("console.receipt.summary.proposed", locale, &[kind])
        }
        crate::action_ledger::ActionStatus::PendingApproval => {
            tfp("console.receipt.summary.pending", locale, &[kind])
        }
        crate::action_ledger::ActionStatus::Approved => {
            tfp("console.receipt.summary.approved", locale, &[kind])
        }
        _ => user_facing_receipt_state_summary(
            unified_inbox_ledger_entry_status(entry),
            trust_console_receipt_status(entry),
            locale,
        ),
    }
}

fn user_facing_receipt_state_summary(status: &str, receipt_status: &str, locale: &str) -> String {
    let receipt_detail = if receipt_status == "Present" {
        t("console.receipt.detail.available", locale)
    } else {
        t("console.receipt.detail.needed", locale)
    };
    tfp(
        "console.receipt.summary.state",
        locale,
        &[user_facing_action_status(status, locale), receipt_detail],
    )
}

fn user_facing_action_status(status: &str, locale: &str) -> &'static str {
    match status {
        "executed" => t("console.receipt.status.completed", locale),
        "executing" => t("console.receipt.status.executing", locale),
        "pending_approval" => t("console.receipt.status.pending", locale),
        "proposed" => t("console.receipt.status.proposed", locale),
        "failed" => t("console.receipt.status.failed", locale),
        "needs_receipt" => t("console.receipt.status.needed", locale),
        _ => t("console.receipt.status.action", locale),
    }
}

fn unified_inbox_system_item(
    source_key: &str,
    title: &str,
    summary: &str,
    now: &str,
    locale: &str,
) -> UnifiedInboxItem {
    UnifiedInboxItem {
        id: opaque_unified_inbox_id("system", source_key),
        kind: "system".into(),
        title: title.into(),
        title_code: None,
        summary: summary.into(),
        owner_visible_reason: t("console.system.reason", locale).to_string(),
        owner_visible_reason_code: Some("system_health_changed".into()),
        authority_basis: "system_health".into(),
        source: "system".into(),
        source_id: opaque_unified_inbox_id("system_source", source_key),
        source_event_id: None,
        status: "informational".into(),
        urgency: "low".into(),
        sensitivity: "S1".into(),
        privacy_level: "system_only".into(),
        created_at: now.into(),
        updated_at: now.into(),
        due_at: None,
        primary_action: None,
        secondary_actions: Vec::new(),
        next_safe_action: t("console.system.next", locale).to_string(),
        next_safe_action_code: Some("system_review_if_needed".into()),
        blocked_by: None,
        blocked_by_code: None,
        receipt_expectation: t("console.system.receipt", locale).to_string(),
        authority: None,
        detail_route: None,
        external_url_policy: "none".into(),
        related_action_ids: Vec::new(),
        related_receipt_ids: Vec::new(),
        route_reason: t("console.route.system", locale).to_string(),
        lineage: Vec::new(),
        schema_version: 1,
    }
}

pub(super) fn opaque_unified_inbox_id(kind: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{kind}:{}", &digest[..16])
}

fn unified_inbox_urgency_rank(urgency: &str) -> u8 {
    match urgency {
        "urgent" => 0,
        "high" => 1,
        "normal" => 2,
        "low" => 3,
        "silent" => 4,
        _ => 5,
    }
}

fn sort_unified_inbox_items(items: &mut [UnifiedInboxItem]) {
    items.sort_by(|a, b| {
        unified_inbox_urgency_rank(&a.urgency)
            .cmp(&unified_inbox_urgency_rank(&b.urgency))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn unified_inbox_counts(items: &[UnifiedInboxItem]) -> UnifiedInboxCounts {
    UnifiedInboxCounts {
        total: items.len(),
        urgent: items.iter().filter(|item| item.urgency == "urgent").count(),
        high: items.iter().filter(|item| item.urgency == "high").count(),
        normal: items.iter().filter(|item| item.urgency == "normal").count(),
        low: items.iter().filter(|item| item.urgency == "low").count(),
        silent: items.iter().filter(|item| item.urgency == "silent").count(),
        approvals: items.iter().filter(|item| item.kind == "approval").count(),
        email: items.iter().filter(|item| item.kind == "email").count(),
        calendar: items.iter().filter(|item| item.kind == "calendar").count(),
        tasks: items.iter().filter(|item| item.kind == "task").count(),
        receipts: items.iter().filter(|item| item.kind == "receipt").count(),
        alerts: items.iter().filter(|item| item.kind == "alert").count(),
        memory_reviews: items
            .iter()
            .filter(|item| item.kind == "memory_review")
            .count(),
        sensitive: items.iter().filter(|item| item.sensitivity != "S1").count(),
    }
}

pub(super) fn forwarded_json_request(
    headers: axum::http::HeaderMap,
    body: serde_json::Value,
) -> Request<axum::body::Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("static vNext forwarded request must build");
    *req.headers_mut() = headers;
    req.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    req
}

fn request_with_headers(headers: axum::http::HeaderMap) -> Request<axum::body::Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::empty())
        .expect("static vNext forwarded request must build");
    *req.headers_mut() = headers;
    req
}

fn webapp_console_json_error_to_string(
    error: (StatusCode, Json<serde_json::Value>),
) -> (StatusCode, String) {
    let message = error
        .1
        .0
        .get("error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.1.0.get("message").and_then(serde_json::Value::as_str))
        .unwrap_or("WebApp action failed")
        .to_string();
    (error.0, message)
}

pub(super) fn webapp_assistant_model_failure(
    error: &dyn std::fmt::Display,
) -> (StatusCode, String) {
    let message = error.to_string();
    let locale = crate::i18n::DEFAULT_LOCALE;
    match crate::orchestrator::classify_owner_failure(&message, locale) {
        Some(crate::orchestrator::OwnerFailureDetail::Credits(text)) => {
            (StatusCode::PAYMENT_REQUIRED, text)
        }
        Some(crate::orchestrator::OwnerFailureDetail::Catalog(text)) => {
            (StatusCode::SERVICE_UNAVAILABLE, text)
        }
        Some(crate::orchestrator::OwnerFailureDetail::Excerpt(text)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Assistant model is unavailable: {text}"),
        ),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Assistant model is unavailable".to_string(),
        ),
    }
}

pub(super) fn webapp_assistant_model_failure_with_notices(
    error: &anyhow::Error,
) -> (StatusCode, String) {
    let (status, text) = webapp_assistant_model_failure(error);
    let notices = error
        .downcast_ref::<crate::behavior_notice::FailedTurnNotices>()
        .map(|context| context.notices.as_slice())
        .unwrap_or(&[]);
    (
        status,
        crate::behavior_notice::append_failure(&text, notices),
    )
}

pub(super) fn validate_vnext_command_request(
    request: &VNextCommandRequest,
) -> Result<(), (StatusCode, String)> {
    validate_context_user_text(Some(&request.input))?;
    // An attachment-only turn is legitimate: a voice note, a photo, or a PDF
    // sent with no typed caption. The message text the orchestrator sees is
    // then derived from the attachment (`vnext_command_effective_input`), which
    // is exactly how the Telegram path behaves. Only a turn carrying NEITHER
    // text NOR an attachment is empty — previously an image-only send was
    // rejected here and the browser dropped it silently.
    if request.attachments.is_empty() {
        validate_general_assistant_user_text(Some(&request.input))?;
    }
    match request.mode.as_str() {
        "text" | "voice" | "voice_placeholder" => {}
        _ => return Err((StatusCode::BAD_REQUEST, "Invalid vNext command mode".into())),
    }
    match request.intent_kind.as_deref().unwrap_or("explain") {
        "chat" | "explain" | "draft" => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext command intent".into(),
            ));
        }
    }
    match request.client_context.as_str() {
        "home" | "work" | "item" | "status" | "settings" => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext client context".into(),
            ));
        }
    }
    if let Some(focus_item_id) = request.focus_item_id.as_deref()
        && !focus_item_id.trim().is_empty()
    {
        validate_context_focus_id(focus_item_id)?;
    }
    validate_vnext_command_conversation_context(request.conversation_context.as_deref())?;
    validate_vnext_command_attachments(&request.attachments)?;
    validate_context_idempotency_key(request.idempotency_key.as_deref())
}

/// Max number of attachments per webapp command turn.
pub(super) const VNEXT_MAX_ATTACHMENTS: usize = 4;
/// Max length of any single attachment `data_url` (~6 MB; base64 ≈ 1.37× raw,
/// so this admits a 5 MB image / part of a 10 MB doc with framing headroom).
pub(super) const VNEXT_MAX_ATTACHMENT_DATA_URL_LEN: usize = 6 * 1024 * 1024;
/// Max total `data_url` bytes across all attachments in one command.
pub(super) const VNEXT_MAX_ATTACHMENTS_TOTAL_LEN: usize = 20 * 1024 * 1024;
/// Raw body cap for `POST /webapp/vnext/command`: everything the attachment
/// validator may accept plus slack for text, context and JSON framing. Must
/// stay ≥ the attachment total, otherwise real photos are rejected as
/// "Invalid vNext command body" before validation ever sees them
/// (owner report 2026-09-05).
pub(super) const VNEXT_COMMAND_MAX_BODY_BYTES: usize = VNEXT_MAX_ATTACHMENTS_TOTAL_LEN + 512 * 1024;
/// Max transcript length for a webapp voice attachment.
pub(super) const VNEXT_MAX_VOICE_TRANSCRIPT_CHARS: usize = 8_000;

/// Slice 5: validate the size/shape of webapp command attachments BEFORE they
/// reach the orchestrator. Rejects oversized single payloads (>~6 MB), an
/// oversized aggregate (>~20 MB), empty/over-long data URLs, and malformed
/// voice transcripts. The `/upload` endpoint already enforces per-file MIME +
/// raw-byte caps; this is the second, payload-level fence on the command body.
pub(super) fn validate_vnext_command_attachments(
    attachments: &[VNextCommandAttachment],
) -> Result<(), (StatusCode, String)> {
    if attachments.len() > VNEXT_MAX_ATTACHMENTS {
        return Err((
            StatusCode::BAD_REQUEST,
            "Too many vNext command attachments".into(),
        ));
    }
    let mut total_len = 0usize;
    for attachment in attachments {
        match attachment {
            VNextCommandAttachment::Image { data_url }
            | VNextCommandAttachment::Document { data_url, .. } => {
                let len = data_url.len();
                if data_url.trim().is_empty() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "Empty vNext attachment data URL".into(),
                    ));
                }
                if !data_url.starts_with("data:") {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "Invalid vNext attachment data URL".into(),
                    ));
                }
                if len > VNEXT_MAX_ATTACHMENT_DATA_URL_LEN {
                    return Err((
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "vNext attachment is too large".into(),
                    ));
                }
                total_len = total_len.saturating_add(len);
            }
            VNextCommandAttachment::Voice { transcript, .. } => {
                if transcript.chars().count() > VNEXT_MAX_VOICE_TRANSCRIPT_CHARS {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "vNext voice transcript is too long".into(),
                    ));
                }
                // A blank transcript carries no message at all: the upload
                // endpoint already rejects a silent recording with 422
                // `voice_empty`, so one arriving here is a malformed client.
                if transcript.trim().is_empty() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "Empty vNext voice transcript".into(),
                    ));
                }
            }
        }
        // Document filename / mime sanity (defensive; the upload endpoint sets these).
        if let VNextCommandAttachment::Document {
            filename,
            mime_type,
            ..
        } = attachment
            && (filename.trim().is_empty()
                || filename.len() > 256
                || mime_type.trim().is_empty()
                || mime_type.len() > 128)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext document attachment metadata".into(),
            ));
        }
    }
    if total_len > VNEXT_MAX_ATTACHMENTS_TOTAL_LEN {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "vNext command attachments are too large".into(),
        ));
    }
    Ok(())
}

/// Cap for `conversation_context` turns accepted per command.
pub(super) const VNEXT_MAX_CONVERSATION_CONTEXT_TURNS: usize = 8;

/// Trim an over-long context to the cap: the first turn (the client puts
/// the screen context there) plus the newest turns.
pub(super) fn trim_vnext_conversation_context(turns: &mut Vec<VNextCommandConversationTurn>) {
    let max = VNEXT_MAX_CONVERSATION_CONTEXT_TURNS;
    if turns.len() <= max {
        return;
    }
    let first = turns.remove(0);
    let keep_from = turns.len().saturating_sub(max - 1);
    turns.drain(..keep_from);
    turns.insert(0, first);
}

pub(super) fn validate_vnext_command_conversation_context(
    turns: Option<&[VNextCommandConversationTurn]>,
) -> Result<(), (StatusCode, String)> {
    let Some(turns) = turns else {
        return Ok(());
    };
    if turns.len() > 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Too many vNext conversation context turns".into(),
        ));
    }
    let mut total_chars = 0usize;
    for turn in turns {
        match turn.role.trim() {
            "owner" | "aegis" => {}
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid vNext conversation context role".into(),
                ));
            }
        }
        let text = turn.text.trim();
        let chars = text.chars().count();
        if chars == 0 || chars > 1_000 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext conversation context text".into(),
            ));
        }
        total_chars += chars;
        if let Some(kind) = turn.kind.as_deref().map(str::trim)
            && !matches!(kind, "recent" | "quoted_reply")
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext conversation context kind".into(),
            ));
        }
        if let Some(source_id) = turn.source_id.as_deref().map(str::trim)
            && (source_id.is_empty()
                || source_id.len() > 160
                || !source_id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.')))
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid vNext conversation context source id".into(),
            ));
        }
    }
    if total_chars > 4_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "vNext conversation context is too large".into(),
        ));
    }
    Ok(())
}

pub(super) fn vnext_command_surface_for_context(client_context: &str) -> &'static str {
    match client_context {
        "home" => "web_chat_home",
        "item" => "web_item_workspace",
        "work" => "web_work_queue",
        "status" => "web_status",
        "settings" => "web_settings",
        _ => "webapp_vnext",
    }
}

pub(super) fn vnext_command_conversation_context_json(
    turns: Option<&[VNextCommandConversationTurn]>,
) -> serde_json::Value {
    let Some(turns) = turns else {
        return serde_json::Value::Null;
    };
    serde_json::Value::Array(
        turns
            .iter()
            .map(|turn| {
                let mut value = serde_json::json!({
                    "role": turn.role.trim(),
                    "text": turn.text.trim(),
                });
                if let Some(kind) = turn
                    .kind
                    .as_deref()
                    .map(str::trim)
                    .filter(|kind| !kind.is_empty())
                {
                    value["kind"] = serde_json::Value::String(kind.to_string());
                }
                if let Some(source_id) = turn
                    .source_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|source_id| !source_id.is_empty())
                {
                    value["source_id"] = serde_json::Value::String(source_id.to_string());
                }
                value
            })
            .collect(),
    )
}

pub(super) fn validate_vnext_work_action_request(
    request: &VNextWorkActionRequest,
) -> Result<(), (StatusCode, String)> {
    match request.action_id.as_str() {
        "task.complete"
        | "task.skip_occurrence"
        | "tracking.cancel"
        | "goal.abandon"
        | "task.snooze_tomorrow"
        | "approval.approve"
        | "approval.reject"
        | "knowledge.done"
        | "knowledge.snooze_tomorrow"
        | "memory.confirm"
        | "memory.pin"
        | "memory.unpin"
        | "memory.mark_wrong"
        | "memory.delete" => {}
        _ => return Err((StatusCode::BAD_REQUEST, "Invalid vNext action id".into())),
    }
    validate_context_idempotency_key(request.idempotency_key.as_deref())
}

pub(super) fn vnext_work_action_is_projected_for_item(
    item: &UnifiedInboxItem,
    action_id: &str,
) -> bool {
    item.secondary_actions
        .iter()
        .any(|action| vnext_action_kind_for_endpoint(&action.action_kind) == action_id)
}

pub(super) async fn dispatch_vnext_work_action(
    state: &Arc<AdminState>,
    headers: &axum::http::HeaderMap,
    chat_id: i64,
    item: &UnifiedInboxItem,
    request: &VNextWorkActionRequest,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let forwarded_headers = headers.clone();
    match request.action_id.as_str() {
        "goal.abandon" => dispatch_vnext_goal_close(state, chat_id, item),
        "tracking.cancel" => dispatch_vnext_tracking_cancel(state, chat_id, item).await,
        "task.skip_occurrence" => dispatch_vnext_skip_occurrence(state, chat_id, item).await,
        "task.complete" | "task.snooze_tomorrow" => {
            if vnext_source_kind(item) != "task" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Action does not apply to this item".into(),
                ));
            }
            let (action, preset) = if request.action_id == "task.complete" {
                ("complete", serde_json::Value::Null)
            } else {
                ("snooze", serde_json::Value::String("tomorrow".into()))
            };
            let body =
                serde_json::json!({ "schema_version": 1, "action": action, "preset": preset });
            webapp_console_task_action(
                State(state.clone()),
                Path(item.source_id.clone()),
                forwarded_json_request(forwarded_headers, body),
            )
            .await
            .map(|json| json.0)
            .map_err(webapp_console_json_error_to_string)
        }
        "approval.approve" | "approval.reject" => {
            if item.kind != "approval"
                && !(vnext_source_kind(item) == "task" && item.status == "blocked_by_gate")
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Action does not apply to this item".into(),
                ));
            }
            let approval_id = resolve_vnext_approval_id_for_item(state, item).await?;
            webapp_console_approval_action(
                state.clone(),
                headers,
                approval_id,
                request.action_id == "approval.approve",
            )
            .await
            .map(|json| json.0)
            .map_err(webapp_console_json_error_to_string)
        }
        "knowledge.done" | "knowledge.snooze_tomorrow" => {
            if item.kind != "waiting_for" || item.source != "knowledge" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Action does not apply to this item".into(),
                ));
            }
            dispatch_vnext_knowledge_action(state, chat_id, item, request.action_id.as_str()).await
        }
        "memory.confirm" | "memory.pin" | "memory.unpin" | "memory.mark_wrong"
        | "memory.delete" => {
            if vnext_source_kind(item) != "memory" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Action does not apply to this item".into(),
                ));
            }
            match request.action_id.as_str() {
                "memory.confirm" => webapp_console_memory_item_confirm(
                    State(state.clone()),
                    Path(item.source_id.clone()),
                    request_with_headers(forwarded_headers),
                )
                .await
                .map(|json| json.0)
                .map_err(webapp_console_json_error_to_string),
                "memory.pin" => webapp_console_memory_item_pin(
                    State(state.clone()),
                    Path(item.source_id.clone()),
                    request_with_headers(forwarded_headers),
                )
                .await
                .map(|json| json.0)
                .map_err(webapp_console_json_error_to_string),
                "memory.unpin" => webapp_console_memory_item_unpin(
                    State(state.clone()),
                    Path(item.source_id.clone()),
                    request_with_headers(forwarded_headers),
                )
                .await
                .map(|json| json.0)
                .map_err(webapp_console_json_error_to_string),
                "memory.mark_wrong" => webapp_console_memory_item_mark_wrong(
                    State(state.clone()),
                    Path(item.source_id.clone()),
                    request_with_headers(forwarded_headers),
                )
                .await
                .map(|json| json.0)
                .map_err(webapp_console_json_error_to_string),
                "memory.delete" => webapp_console_memory_item_delete(
                    State(state.clone()),
                    Path(item.source_id.clone()),
                    request_with_headers(forwarded_headers),
                )
                .await
                .map(|json| json.0)
                .map_err(webapp_console_json_error_to_string),
                _ => Err((StatusCode::BAD_REQUEST, "Invalid vNext action id".into())),
            }
        }
        _ => Err((StatusCode::BAD_REQUEST, "Invalid vNext action id".into())),
    }
}

fn owner_task_for_work_action(
    task: &crate::memory::Task,
    chat_id: i64,
) -> Result<(), (StatusCode, String)> {
    let owner = format!("did:aegis:telegram:{chat_id}");
    if task.chat_id != chat_id || task.user_did != owner {
        return Err((StatusCode::NOT_FOUND, "Task not found".into()));
    }
    Ok(())
}

fn propose_owner_work_receipt(
    state: &Arc<AdminState>,
    chat_id: i64,
    action_record_id: &str,
    action_kind: crate::action_ledger::ActionKind,
    summary: &str,
    resource_type: &str,
    resource_id: i64,
    action_id: &str,
) -> Result<(), (StatusCode, String)> {
    let conn =
        webapp_console_action_ledger_conn(state).map_err(webapp_console_json_error_to_string)?;
    let store = crate::action_ledger::ActionLedgerStore::new(&conn);
    store
        .create_proposed(&crate::action_ledger::NewAction {
            action_id: action_record_id.to_string(),
            action_kind,
            requested_by: Some(format!("did:aegis:telegram:{chat_id}")),
            chat_id: Some(chat_id),
            input_summary: Some(format!(
                "{resource_type} #{resource_id} {action_id} requested"
            )),
            user_visible_summary: Some(summary.to_string()),
            metadata_json: Some(
                serde_json::json!({
                    "resource_type": resource_type,
                    "resource_id": resource_id,
                    "action": action_id,
                    "source": "web_console",
                })
                .to_string(),
            ),
            ..Default::default()
        })
        .and_then(|_| {
            store.transition(
                action_record_id,
                crate::action_ledger::ActionStatus::Executing,
                &crate::action_ledger::ActionUpdate::default(),
            )
        })
        .map_err(|error| {
            tracing::warn!(%error, action_id, resource_id, "Work action receipt create failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create action receipt".to_string(),
            )
        })?;
    Ok(())
}

fn finish_owner_work_receipt(
    state: &Arc<AdminState>,
    action_record_id: &str,
    receipt: &str,
    summary: &str,
    failed: bool,
) -> Result<i64, (StatusCode, String)> {
    let status = if failed {
        crate::action_ledger::ActionStatus::Failed
    } else {
        crate::action_ledger::ActionStatus::Executed
    };
    let update = if failed {
        crate::action_ledger::ActionUpdate {
            error_kind: Some("work_action_failed".into()),
            error_message: Some(summary.to_string()),
            ..Default::default()
        }
    } else {
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.to_string()),
            external_event_id: Some(action_record_id.to_string()),
            user_visible_summary: Some(summary.to_string()),
            ..Default::default()
        }
    };
    webapp_console_transition_action_ledger(state, action_record_id, status, update)
        .map(|entry| entry.id)
        .map_err(|error| {
            tracing::warn!(%error, action_record_id, "Work action receipt update failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Action finished but receipt update failed".to_string(),
            )
        })
}

async fn load_waiting_generator_task(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
) -> Result<crate::memory::Task, (StatusCode, String)> {
    let entry_id = item.source_id.parse::<i64>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid knowledge item id".to_string(),
        )
    })?;
    let entry = state
        .memory
        .get_knowledge_entry(entry_id, chat_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Knowledge item not found".to_string(),
            )
        })?;
    if entry.category != "waiting_for" || entry.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Action does not apply to this item".into(),
        ));
    }
    let owner = format!("did:aegis:telegram:{chat_id}");
    let tasks = list_open_owner_tasks(state, chat_id).await;
    if let Some(task) = waiting_generator_from_tasks(&entry, &tasks, &owner) {
        return Ok(task.clone());
    }
    if let crate::memory::WaitingParentTag::Task(task_id) =
        crate::memory::waiting_parent_tag(&entry.tags)
        && !tasks.iter().any(|task| task.id == task_id)
        && let Some(task) = state.memory.get_task(task_id).await
        && task_is_owned_waiting_generator(&task, &owner, chat_id)
    {
        return Ok(task);
    }
    Err((
        StatusCode::BAD_REQUEST,
        "Action does not apply to this item".into(),
    ))
}

async fn load_owned_task(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
) -> Result<crate::memory::Task, (StatusCode, String)> {
    if vnext_source_kind(item) != "task" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Action does not apply to this item".into(),
        ));
    }
    let task_id = item
        .source_id
        .parse::<i64>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid task id".into()))?;
    let task = state
        .memory
        .get_task(task_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Task not found".to_string()))?;
    owner_task_for_work_action(&task, chat_id)?;
    Ok(task)
}

/// Close the generator. Later fires of this row stop. The row stays as history.
pub(super) async fn dispatch_vnext_tracking_cancel(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let task = if item.kind == "waiting_for" && item.source == "knowledge" {
        load_waiting_generator_task(state, chat_id, item).await?
    } else {
        load_owned_task(state, chat_id, item).await?
    };
    if task.status != "active" {
        return Err((
            StatusCode::CONFLICT,
            "Action does not apply to this item".into(),
        ));
    }
    let locale = state.settings.display_language();
    let summary = t("console.tracking.cancelled", &locale).to_string();
    let now = chrono::Utc::now();
    let action_record_id = format!(
        "web-console-tracking-cancel:{}:{}",
        task.id,
        now.timestamp_millis()
    );
    let receipt = format!("task:{}:cancelled:{action_record_id}", task.id);
    let owner = format!("did:aegis:telegram:{chat_id}");
    propose_owner_work_receipt(
        state,
        chat_id,
        &action_record_id,
        crate::action_ledger::ActionKind::ReminderDelete,
        &summary,
        "task",
        task.id,
        "tracking.cancel",
    )?;
    let cancelled = state
        .memory
        .cancel_intent(task.id, &owner)
        .await
        .map_err(|error| {
            let _ = finish_owner_work_receipt(
                state,
                &action_record_id,
                &receipt,
                &error.to_string(),
                true,
            );
            tracing::warn!(%error, task_id = task.id, "tracking.cancel failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to close tracking".to_string(),
            )
        })?;
    if !cancelled {
        let _ = finish_owner_work_receipt(
            state,
            &action_record_id,
            &receipt,
            "cancel matched no active row",
            true,
        );
        return Err((
            StatusCode::CONFLICT,
            "Action does not apply to this item".into(),
        ));
    }
    let entry_id = finish_owner_work_receipt(state, &action_record_id, &receipt, &summary, false)?;
    Ok(serde_json::json!({
        "status": "ok",
        "resource_type": "task",
        "resource_id": task.id.to_string(),
        "work_item_id": item.id,
        "action": "tracking.cancel",
        "action_id": action_record_id,
        "updated": true,
        "task_status": "cancelled",
        "ledger_entry_id": entry_id,
        "receipt": receipt,
        "summary": summary,
    }))
}

/// Skip the current fire of a clock loop. Status stays active.
async fn dispatch_vnext_skip_occurrence(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let task = load_owned_task(state, chat_id, item).await?;
    let interval = task
        .interval_seconds
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Action does not apply to this item".to_string(),
            )
        })?;
    if task.status != "active" {
        return Err((
            StatusCode::CONFLICT,
            "Action does not apply to this item".into(),
        ));
    }
    let locale = state.settings.display_language();
    let summary = t("console.tracking.skipped", &locale).to_string();
    let now = chrono::Utc::now();
    let next = recurring_skip_trigger(now.timestamp(), task.trigger_at_unix, interval);
    let expires = next.saturating_add(crate::intent::MAX_INTENT_LIFETIME_SECS);
    let action_record_id = format!(
        "web-console-tracking-skip:{}:{}",
        task.id,
        now.timestamp_millis()
    );
    let receipt = format!("task:{}:skipped:{action_record_id}", task.id);
    propose_owner_work_receipt(
        state,
        chat_id,
        &action_record_id,
        crate::action_ledger::ActionKind::ReminderUpdate,
        &summary,
        "task",
        task.id,
        "task.skip_occurrence",
    )?;
    if let Err(error) = state
        .memory
        .reschedule_recurring_intent(task.id, next, expires)
        .await
    {
        let _ =
            finish_owner_work_receipt(state, &action_record_id, &receipt, &error.to_string(), true);
        tracing::warn!(%error, task_id = task.id, "task.skip_occurrence failed");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to skip this occurrence".to_string(),
        ));
    }
    let due_at =
        chrono::DateTime::<chrono::Utc>::from_timestamp(next, 0).map(|stamp| stamp.to_rfc3339());
    let entry_id = finish_owner_work_receipt(state, &action_record_id, &receipt, &summary, false)?;
    Ok(serde_json::json!({
        "status": "ok",
        "resource_type": "task",
        "resource_id": item.source_id,
        "action": "task.skip_occurrence",
        "action_id": action_record_id,
        "updated": true,
        "task_status": "active",
        "due_at": due_at,
        "ledger_entry_id": entry_id,
        "receipt": receipt,
        "summary": summary,
    }))
}

/// Close only the authenticated owner's active goal. The state change and its
/// receipt commit together, so a failed ledger write cannot silently close it.
pub(super) fn dispatch_vnext_goal_close(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
) -> Result<serde_json::Value, (StatusCode, String)> {
    if item.kind != "goal" || item.source != "goals" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Action does not apply to this item".into(),
        ));
    }
    let goal_id = item
        .source_id
        .parse::<i64>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid goal id".into()))?;
    let owner = format!("did:aegis:telegram:{chat_id}");
    let now = chrono::Utc::now();
    let action_id = format!(
        "web-console-goal-abandon:{goal_id}:{}",
        now.timestamp_micros()
    );
    let receipt = format!("goal:{goal_id}:abandoned:{action_id}");
    let summary = t("console.goal.closed", &state.settings.display_language()).to_string();
    let mut conn =
        webapp_console_action_ledger_conn(state).map_err(webapp_console_json_error_to_string)?;
    let persist = (|| -> anyhow::Result<Option<i64>> {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE goals SET status = 'abandoned', last_progress_at = ?1 WHERE id = ?2 AND user_did = ?3 AND status = 'active'",
            rusqlite::params![now.to_rfc3339(), goal_id, owner],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        let store = crate::action_ledger::ActionLedgerStore::new(&tx);
        store.create_proposed(&crate::action_ledger::NewAction {
            action_id: action_id.clone(),
            action_kind: crate::action_ledger::ActionKind::Other,
            requested_by: Some(owner.clone()),
            chat_id: Some(chat_id),
            input_summary: Some(format!("Stop tracking goal #{goal_id}")),
            user_visible_summary: Some(summary.clone()),
            metadata_json: Some(
                serde_json::json!({
                    "resource_type": "goal", "resource_id": goal_id,
                    "action": "goal.abandon", "source": "web_console"
                })
                .to_string(),
            ),
            ..Default::default()
        })?;
        store.transition(
            &action_id,
            crate::action_ledger::ActionStatus::Executing,
            &crate::action_ledger::ActionUpdate::default(),
        )?;
        let entry = store.transition(
            &action_id,
            crate::action_ledger::ActionStatus::Executed,
            &crate::action_ledger::ActionUpdate {
                external_receipt: Some(receipt.clone()),
                external_event_id: Some(action_id.clone()),
                user_visible_summary: Some(summary.clone()),
                ..Default::default()
            },
        )?;
        tx.commit()?;
        Ok(Some(entry.id))
    })();
    let entry_id = persist
        .map_err(|error| {
            tracing::warn!(%error, goal_id, "Goal close transaction failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to close goal".to_string(),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                "Action does not apply to this item".to_string(),
            )
        })?;
    Ok(serde_json::json!({
        "status": "ok", "resource_type": "goal", "resource_id": item.source_id,
        "action": "goal.abandon", "action_id": action_id, "updated": true,
        "goal_status": "abandoned", "ledger_entry_id": entry_id,
        "receipt": receipt, "summary": summary,
    }))
}

pub(super) async fn dispatch_vnext_knowledge_action(
    state: &Arc<AdminState>,
    chat_id: i64,
    item: &UnifiedInboxItem,
    action_id: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let entry_id = item.source_id.parse::<i64>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid knowledge item id".to_string(),
        )
    })?;
    let entry = state
        .memory
        .get_knowledge_entry(entry_id, chat_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Knowledge item not found".to_string(),
            )
        })?;
    if entry.category != "waiting_for" || entry.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Action does not apply to this item".to_string(),
        ));
    }

    let now = chrono::Utc::now();
    let (updates, receipt_suffix, summary, due_at) = match action_id {
        "knowledge.done" => (
            crate::memory::KnowledgeEntryUpdate {
                status: Some("done".into()),
                resolved_at: Some(Some(now.to_rfc3339())),
                ..Default::default()
            },
            "done".to_string(),
            format!(
                "Closed follow-up: {}",
                owner_visible_title(&entry.title, "waiting item", 120)
            ),
            None,
        ),
        "knowledge.snooze_tomorrow" => {
            let due = now + chrono::Duration::days(1);
            (
                crate::memory::KnowledgeEntryUpdate {
                    status: Some("active".into()),
                    due_date: Some(Some(due.to_rfc3339())),
                    ..Default::default()
                },
                "tomorrow".to_string(),
                format!(
                    "Follow-up will resurface tomorrow: {}",
                    owner_visible_title(&entry.title, "waiting item", 120)
                ),
                Some(due.to_rfc3339()),
            )
        }
        _ => return Err((StatusCode::BAD_REQUEST, "Invalid vNext action id".into())),
    };

    let action_record_id = format!(
        "web-console-knowledge-{receipt_suffix}:{}:{}",
        entry_id,
        now.timestamp_millis()
    );
    let receipt = format!("kb_waiting:{entry_id}:{receipt_suffix}:{action_record_id}");

    {
        let conn = webapp_console_action_ledger_conn(state)
            .map_err(webapp_console_json_error_to_string)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_record_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(format!("did:aegis:telegram:{chat_id}")),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Knowledge item #{entry_id} {receipt_suffix} requested")),
                user_visible_summary: Some(summary.clone()),
                idempotency_key: Some(format!(
                    "knowledge-action:{chat_id}:{entry_id}:{receipt_suffix}"
                )),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "knowledge",
                        "resource_id": entry_id,
                        "action": action_id,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_record_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, entry_id, action_id, "Web Console knowledge action ledger create failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create knowledge action receipt".to_string(),
                )
            })?;
    }

    let updated = state
        .memory
        .update_knowledge_entry(entry_id, chat_id, updates)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, entry_id, action_id, "Web Console knowledge action failed");
            let _ = webapp_console_transition_action_ledger(
                state,
                &action_record_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("knowledge_action_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update knowledge item".to_string(),
            )
        })?;
    if !updated {
        let _ = webapp_console_transition_action_ledger(
            state,
            &action_record_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("knowledge_not_updated".into()),
                error_message: Some("Knowledge update changed no rows".into()),
                ..Default::default()
            },
        );
        return Err((
            StatusCode::NOT_FOUND,
            "Knowledge item not found".to_string(),
        ));
    }

    let entry = webapp_console_transition_action_ledger(
        state,
        &action_record_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_record_id.clone()),
            user_visible_summary: Some(summary.clone()),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, entry_id, action_id, "Web Console knowledge action ledger execute failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Knowledge item updated but receipt update failed".to_string(),
        )
    })?;

    Ok(serde_json::json!({
        "status": "ok",
        "resource_type": "knowledge",
        "resource_id": item.source_id.clone(),
        "action": action_id,
        "updated": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "summary": summary,
        "due_at": due_at,
    }))
}

pub(super) async fn resolve_vnext_approval_id_for_item(
    state: &Arc<AdminState>,
    item: &UnifiedInboxItem,
) -> Result<String, (StatusCode, String)> {
    let approvals = state.memory.list_hitl_pending().await.map_err(|error| {
        tracing::warn!(error = %error, "Unable to resolve vNext approval action");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Approval queue is unavailable".to_string(),
        )
    })?;
    if vnext_source_kind(item) == "task" && item.status == "blocked_by_gate" {
        let task_id = item.source_id.parse::<i64>().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Blocked task cannot be resolved to an approval".to_string(),
            )
        })?;
        return approvals
            .into_iter()
            .find_map(|(approval_id, value)| {
                (hitl_approval_source_task_id(&value) == Some(task_id)).then_some(approval_id)
            })
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "Approval for paused task not found".to_string(),
                )
            });
    }
    approvals
        .into_iter()
        .find_map(|(approval_id, _)| {
            (opaque_unified_inbox_id("approval", &approval_id) == item.id).then_some(approval_id)
        })
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Approval not found".to_string()))
}

pub(super) fn vnext_command_response(owner_response: serde_json::Value) -> serde_json::Value {
    let owner_message = owner_response
        .get("visible_answer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Aegis responded.")
        .to_string();
    let response_kind = owner_response
        .get("response_kind")
        .and_then(serde_json::Value::as_str);
    // CEO Intent labels (`suggested_intent`) are display-only ("我理解成…") and
    // must NOT flip outcome to draft_action — that used to make every shorthand
    // / free-text chat look like an action draft. Real drafts use action_draft
    // or a non-empty draft payload.
    let has_action_draft = owner_response
        .get("action_draft")
        .is_some_and(|value| !value.is_null());
    let has_text_draft = owner_response
        .get("draft")
        .is_some_and(|value| match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            _ => true,
        });
    let outcome = if matches!(response_kind, Some("gate_required" | "refusal"))
        && owner_response
            .get("required_gate")
            .is_some_and(|value| !value.is_null())
    {
        "request_permission"
    } else if has_action_draft || has_text_draft {
        "draft_action"
    } else if response_kind == Some("error") {
        "request_clarification"
    } else {
        "answer_now"
    };
    serde_json::json!({
        "schema_version": 1,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "outcome": outcome,
        "owner_message": owner_message,
        "assistant_response": owner_response
    })
}

pub(super) fn is_vnext_tasks_slash_command(input: &str) -> bool {
    let trimmed = input.trim();
    matches!(
        trimmed
            .split_whitespace()
            .next()
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("/tasks")
    )
}

pub(super) async fn webapp_vnext_tasks_command_response(
    state: &AdminState,
    chat_id: i64,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user_tz = crate::settings::parse_user_tz(state.settings.as_ref());
    let mut tasks = state
        .memory
        .list_tasks(chat_id, None, Some("active"), 50)
        .await;
    tasks.sort_by(|a, b| {
        task_priority_rank(&a.priority)
            .cmp(&task_priority_rank(&b.priority))
            .then_with(|| {
                a.trigger_at_unix
                    .unwrap_or(i64::MAX)
                    .cmp(&b.trigger_at_unix.unwrap_or(i64::MAX))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    let answer = format_vnext_tasks_slash_answer(&tasks, user_tz);
    let envelope = build_general_assistant_context_envelope();
    let mut response =
        contextual_assistant_success("chat", answer, envelope, "web-vnext-direct-tasks".into());
    response.response_kind = "task_list".into();
    let locale = state.settings.display_language();
    response.receipt_expectation = t("console.tasks.receipt", &locale).into();
    response.citations = vec!["tool:manage_tasks".into()];
    response.source_evidence = vec![ContextualAssistantSourceEvidence {
        kind: "tool".into(),
        label: t("console.tasks.source.label", &locale).into(),
        summary: t("console.tasks.source.summary", &locale).into(),
        detail: t("console.tasks.source.detail", &locale).into(),
        jev: None,
    }];
    response.warnings.clear();
    let owner_response = serde_json::to_value(response).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize task list response: {err}"),
        )
    })?;
    Ok(Json(vnext_command_response(owner_response)))
}

fn task_priority_rank(priority: &str) -> u8 {
    match priority {
        "urgent" => 0,
        "high" => 1,
        "normal" => 2,
        "low" => 3,
        _ => 4,
    }
}

fn format_vnext_tasks_slash_answer(
    tasks: &[crate::memory::Task],
    user_tz: chrono_tz::Tz,
) -> String {
    if tasks.is_empty() {
        return "当前没有活跃任务或提醒。".into();
    }

    let now = chrono::Utc::now();
    let mut now_items = Vec::new();
    let mut soon_items = Vec::new();
    let mut later_items = Vec::new();
    let mut routine_items = Vec::new();

    for task in tasks {
        let bucket = vnext_task_bucket(task, now);
        match bucket {
            VNextTaskBucket::Now => &mut now_items,
            VNextTaskBucket::Soon => &mut soon_items,
            VNextTaskBucket::Later => &mut later_items,
            VNextTaskBucket::Routine => &mut routine_items,
        }
        .push(task);
    }

    let mut lines = vec![
        format!("当前有 {} 个活跃任务/提醒。", tasks.len()),
        String::new(),
    ];
    push_vnext_task_bucket_lines(&mut lines, "Now", &now_items, user_tz, 4);
    push_vnext_task_bucket_lines(&mut lines, "Soon", &soon_items, user_tz, 4);
    push_vnext_task_bucket_lines(&mut lines, "Later", &later_items, user_tz, 3);
    push_vnext_task_bucket_lines(&mut lines, "Routines", &routine_items, user_tz, 3);

    let shown = now_items.len().min(4)
        + soon_items.len().min(4)
        + later_items.len().min(3)
        + routine_items.len().min(3);
    if tasks.len() > shown {
        lines.push(String::new());
        lines.push(format!(
            "还有 {} 个较低优先项，打开 Work 查看。",
            tasks.len() - shown
        ));
    }
    lines.join("\n")
}

enum VNextTaskBucket {
    Now,
    Soon,
    Later,
    Routine,
}

fn vnext_task_bucket(
    task: &crate::memory::Task,
    now: chrono::DateTime<chrono::Utc>,
) -> VNextTaskBucket {
    if task.interval_seconds.is_some()
        || task
            .trigger_type
            .as_deref()
            .is_some_and(|value| value == "event")
        || matches!(task.execution_mode.as_str(), "llm_callback")
    {
        return VNextTaskBucket::Routine;
    }
    if matches!(task.priority.as_str(), "urgent" | "high") {
        return VNextTaskBucket::Now;
    }
    if let Some(trigger_at) = task.trigger_at_unix
        && let Some(trigger_at) = chrono::DateTime::<chrono::Utc>::from_timestamp(trigger_at, 0)
    {
        let delta = trigger_at.signed_duration_since(now);
        if delta.num_seconds() <= 0 || delta.num_days() <= 7 {
            return VNextTaskBucket::Soon;
        }
    }
    if task.trigger_at_unix.is_some() || task.due_date.is_some() {
        return VNextTaskBucket::Later;
    }
    VNextTaskBucket::Soon
}

fn push_vnext_task_bucket_lines(
    lines: &mut Vec<String>,
    heading: &str,
    tasks: &[&crate::memory::Task],
    user_tz: chrono_tz::Tz,
    limit: usize,
) {
    if tasks.is_empty() {
        return;
    }
    lines.push(format!("**{heading}**"));
    for task in tasks.iter().take(limit) {
        lines.push(format_vnext_task_list_item(task, user_tz));
    }
    if tasks.len() > limit {
        lines.push(format!("- 还有 {} 个", tasks.len() - limit));
    }
    lines.push(String::new());
}

fn format_vnext_task_list_item(task: &crate::memory::Task, user_tz: chrono_tz::Tz) -> String {
    let icon = if task.trigger_at_unix.is_some() {
        "⏰"
    } else {
        "•"
    };
    let title = truncate_owner_evidence_text(&task.title, 72);
    let mut line = format!("{icon} {title}");
    if let Some(time) = format_vnext_task_due_for_owner(task, user_tz) {
        line.push_str(&format!(" — {time}"));
    }
    if matches!(task.priority.as_str(), "urgent" | "high") {
        line.push_str(&format!(" · {}", task.priority));
    }
    line.push_str(&format!(" · #{}", task.id));
    line
}

fn format_vnext_task_due_for_owner(
    task: &crate::memory::Task,
    user_tz: chrono_tz::Tz,
) -> Option<String> {
    if let Some(ts) = task.trigger_at_unix {
        return chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0).map(|dt| {
            dt.with_timezone(&user_tz)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string()
        });
    }
    task.due_date
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) async fn attach_new_vnext_work_item_after_command(
    state: &AdminState,
    chat_id: i64,
    prior_ids: &HashSet<String>,
    response: &mut serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    let locale = state.settings.display_language();
    let projection = build_unified_inbox_projection(state, chat_id, &locale).await;
    let Some(item) = projection
        .items
        .iter()
        .find(|item| !prior_ids.contains(&item.id) && item.kind != "action_draft")
    else {
        return Ok(());
    };
    let work_item = serde_json::to_value(vnext_work_item(item, &locale)).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize command work item: {err}"),
        )
    })?;
    response["work_item"] = work_item;
    Ok(())
}

pub(super) fn vnext_work_action_response(
    item: &UnifiedInboxItem,
    action_id: &str,
    raw_result: serde_json::Value,
    locale: &str,
) -> serde_json::Value {
    let receipt_label = raw_result
        .get("receipt")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            raw_result
                .get("external_receipt")
                .and_then(serde_json::Value::as_str)
        });
    let owner_message = raw_result
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            raw_result
                .get("message")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or_else(|| t("console.action.completed", locale));
    let receipt = receipt_label.map(|label| {
        serde_json::json!({
            "id": raw_result
                .get("action_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(action_id),
            "kind": "action_receipt",
            "label": label,
            "created_at": chrono::Utc::now().to_rfc3339()
        })
    });
    serde_json::json!({
        "schema_version": 1,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "status": "ok",
        "item_id": item.id,
        "action_id": action_id,
        "owner_message": owner_message,
        "receipt": receipt,
        "raw_result": raw_result
    })
}

#[cfg(test)]
mod mandate_match_tests {
    use super::mandate_match_source_work_item_id;
    use super::unified_inbox_item_from_mandate_match;

    fn calendar_event(id: i64, title: &str) -> crate::assistant_events::StoredAssistantEvent {
        crate::assistant_events::StoredAssistantEvent {
            id,
            event: crate::assistant_events::AssistantEvent {
                raw_event_id: None,
                observed_at: "2026-09-25T00:00:00Z".into(),
                occurred_at: "2026-09-25T00:00:00Z".into(),
                user_did: "did:aegis:telegram:1".into(),
                source: crate::assistant_events::AssistantEventSource::Calendar,
                kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
                account_id: None,
                title: title.into(),
                summary: "private calendar summary".into(),
                thread_key: None,
                urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
                actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
                decision: crate::assistant_events::AssistantEventDecision::DigestLater,
                dedup_key: format!("cal-{id}"),
                entities: serde_json::json!({}),
            },
        }
    }

    fn mandate() -> crate::user_mandates::UserMandate {
        crate::user_mandates::UserMandate {
            id: 1,
            mandate_id: "mandate-1".into(),
            title: "看日历".into(),
            description: String::new(),
            owner_principal: "did:aegis:telegram:1".into(),
            status: crate::user_mandates::MandateStatus::Active,
            scope_kind: "calendar_metadata".into(),
            source_filters_json: "{}".into(),
            allowed_reads_json: "[]".into(),
            allowed_llm_uses_json: "[]".into(),
            allowed_actions_json: "[]".into(),
            must_ask_json: "[]".into(),
            notification_policy_json: "{}".into(),
            budget_policy_json: "{}".into(),
            frequency_policy_json: "{}".into(),
            expiry: None,
            review_after: None,
            created_at: "2026-09-25T00:00:00Z".into(),
            updated_at: "2026-09-25T00:00:00Z".into(),
            created_from: "test".into(),
            source_event_id: None,
            trace_id: None,
            run_id: None,
            schema_version: 1,
        }
    }

    #[test]
    fn mandate_match_opens_the_calendar_work_item() {
        let event = calendar_event(42, "牙医预约");
        assert_eq!(mandate_match_source_work_item_id(&event), "calendar:42");
        let item = unified_inbox_item_from_mandate_match(
            &mandate(),
            &event,
            "2026-09-25T00:00:00Z",
            None,
            "zh",
        );
        assert_eq!(item.source_event_id.as_deref(), Some("calendar:42"));
        assert_eq!(
            item.detail_route.as_deref(),
            Some("/ui/?view=work&item=calendar:42")
        );
        assert!(!item.title.contains("牙医"));
        assert!(!item.summary.contains("private calendar summary"));
    }
}

#[cfg(test)]
mod nearby_signal_tests {
    use super::unified_inbox_item_from_goal;
    use super::unified_inbox_item_from_task;
    use super::unified_inbox_item_from_waiting_knowledge;
    use super::unified_inbox_system_item;
    use crate::memory::{Goal, GoalSource, GoalStatus, KnowledgeEntry, Task};

    fn shows_route_reason_as_signal(item: &super::UnifiedInboxItem) -> bool {
        item.blocked_by
            .as_ref()
            .is_none_or(|value| value.trim().is_empty())
            && item
                .due_at
                .as_ref()
                .is_none_or(|value| value.trim().is_empty())
            && item.related_receipt_ids.is_empty()
    }

    fn goal_without_due_date() -> Goal {
        Goal {
            id: 7,
            user_did: "did:key:test".into(),
            title: "给门铃充电".into(),
            description: String::new(),
            status: GoalStatus::Active,
            parent_goal_id: None,
            progress_pct: 0.0,
            milestones: Vec::new(),
            created_at: "2026-09-25T00:00:00Z".into(),
            target_date: None,
            last_progress_at: None,
            source: GoalSource::Explicit,
        }
    }

    fn waiting_without_due_date() -> KnowledgeEntry {
        KnowledgeEntry {
            id: 9,
            chat_id: 1,
            category: "waiting_for".into(),
            priority: "normal".into(),
            status: "active".into(),
            title: "门铃零件".into(),
            body: String::new(),
            tags: Vec::new(),
            related_ids: Vec::new(),
            source_msg_id: None,
            source_summary: None,
            due_date: None,
            waiting_on: None,
            superseded_by: None,
            resolved_at: None,
            created_at: "2026-09-25T00:00:00Z".into(),
            updated_at: "2026-09-25T00:00:00Z".into(),
        }
    }

    fn task_without_due_date() -> Task {
        Task {
            id: 3,
            chat_id: 1,
            user_did: "did:key:test".into(),
            title: "周末给门铃充电".into(),
            body: String::new(),
            priority: "normal".into(),
            status: "active".into(),
            tags: Vec::new(),
            due_date: None,
            source: "user".into(),
            trigger_at_unix: None,
            trigger_type: None,
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
            created_at: "2026-09-25T00:00:00Z".into(),
            updated_at: "2026-09-25T00:00:00Z".into(),
            completed_at: None,
        }
    }

    #[test]
    fn nearby_cards_use_the_display_language_for_the_signal() {
        let goal = unified_inbox_item_from_goal(&goal_without_due_date(), "zh");
        let waiting = unified_inbox_item_from_waiting_knowledge(&waiting_without_due_date(), "zh");
        let task = unified_inbox_item_from_task(&task_without_due_date(), "zh");
        let system =
            unified_inbox_system_item("calendar", "日历", "来源可用", "2026-09-25T00:00:00Z", "zh");
        for item in [&goal, &waiting, &task, &system] {
            assert!(shows_route_reason_as_signal(item));
            assert!(
                item.route_reason
                    .chars()
                    .all(|ch| !ch.is_ascii_alphabetic()),
                "nearby signal still has English: {}",
                item.route_reason
            );
        }
        assert_eq!(goal.route_reason, "这是你列表里还在进行的目标。");
        assert_eq!(waiting.route_reason, "这条待确认的知识在等你看。");
        assert_eq!(task.route_reason, "这件待办来自你的待办列表。");
        assert_eq!(system.route_reason, "这是一条来源是否可用的通知。");

        let english = unified_inbox_item_from_goal(&goal_without_due_date(), "en");
        assert_eq!(
            english.route_reason,
            "This is an active goal from your list."
        );
    }
}
