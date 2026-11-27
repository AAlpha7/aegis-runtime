use super::*;
use crate::i18n::{t, tfp};

pub(super) fn vnext_work_item(item: &UnifiedInboxItem, locale: &str) -> VNextWorkItem {
    let authority = item
        .authority
        .clone()
        .unwrap_or_else(|| owner_work_item_authority(item));
    let title = owner_visible_title(&item.title, t("console.item.work_fallback", locale), 180);
    let receipt_expectation = if item.receipt_expectation.trim().is_empty() {
        None
    } else {
        Some(item.receipt_expectation.clone())
    };

    VNextWorkItem {
        id: item.id.clone(),
        lane: vnext_lane_for_item(item).to_string(),
        source_kind: vnext_source_kind(item),
        source: item.source.clone(),
        source_id: item.source_id.clone(),
        source_event_id: item.source_event_id.clone(),
        title: title.clone(),
        title_code: item.title_code.clone().filter(|_| title == item.title),
        summary: item.summary.clone(),
        why_it_matters: item.owner_visible_reason.clone(),
        why_it_matters_code: item.owner_visible_reason_code.clone(),
        state: item.status.clone(),
        urgency: item.urgency.clone(),
        sensitivity: item.sensitivity.clone(),
        created_at: item.created_at.clone(),
        updated_at: item.updated_at.clone(),
        due_at: item.due_at.clone(),
        authority: vnext_authority(&authority, item),
        recommended_action: item
            .primary_action
            .as_ref()
            .map(|action| vnext_action_ref(action, item)),
        contextual_actions: item
            .secondary_actions
            .iter()
            .map(|action| vnext_action_ref(action, item))
            .collect(),
        next_safe_action: item.next_safe_action.clone(),
        next_safe_action_code: item.next_safe_action_code.clone(),
        blocked_by: item.blocked_by.clone(),
        blocked_by_code: item.blocked_by_code.clone(),
        detail_route: item.detail_route.clone(),
        external_url_policy: item.external_url_policy.clone(),
        route_reason: item.route_reason.clone(),
        // Queue cards carry bounded reference previews, matching the Web contract.
        // A receipt-day group can contain all 50 recent ledger entries; the
        // full count/history stays in the source item and ledger.
        related_work_item_ids: item.related_action_ids.iter().take(32).cloned().collect(),
        related_receipt_ids: item.related_receipt_ids.iter().take(32).cloned().collect(),
        receipt_expectation,
        decision_request: vnext_decision_request(item, locale),
        lineage: item.lineage.clone(),
    }
}

pub(super) fn vnext_decision_request(
    item: &UnifiedInboxItem,
    locale: &str,
) -> Option<VNextDecisionRequest> {
    if item.status == "blocked_by_gate" {
        return Some(vnext_blocked_decision_request(item, locale));
    }
    if item.kind == "approval" {
        return Some(vnext_approval_decision_request(item, locale));
    }
    None
}

fn vnext_blocked_decision_request(item: &UnifiedInboxItem, locale: &str) -> VNextDecisionRequest {
    let kind = blocked_owner_automation_kind_from_text(&[
        &item.title,
        &item.summary,
        &item.owner_visible_reason,
        item.blocked_by.as_deref().unwrap_or_default(),
        &item.next_safe_action,
        &item.source,
        &item.source_id,
    ]);
    let (what, what_code) = blocked_owner_automation_what(item, kind, locale);
    let (requested_capability, capability_code) = match kind {
        "web_search" => (
            t("console.decide.blocked.cap.web_search", locale),
            "cap_web_search_tool",
        ),
        _ => (
            t("console.decide.blocked.cap.generic", locale),
            "cap_generic_boundary",
        ),
    };
    let (blocked_reason, blocked_reason_code) = match item
        .blocked_by
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => (value.to_string(), item.blocked_by_code.clone()),
        None => (
            item.owner_visible_reason.clone(),
            item.owner_visible_reason_code.clone(),
        ),
    };
    let (title, title_code) = match kind {
        "web_search" => (
            t("console.decide.blocked.title.web_search", locale),
            "title_web_search",
        ),
        _ => (
            t("console.decide.blocked.title.generic", locale),
            "title_generic",
        ),
    };
    let mut choices = Vec::new();
    let projected_actions: Vec<(String, &UnifiedInboxAction)> = item
        .secondary_actions
        .iter()
        .map(|action| (vnext_action_kind_for_endpoint(&action.action_kind), action))
        .collect();
    let approval_approve = projected_actions
        .iter()
        .find(|(kind, _)| kind == "approval.approve")
        .map(|(_, action)| *action);
    let approval_reject = projected_actions
        .iter()
        .find(|(kind, _)| kind == "approval.reject")
        .map(|(_, action)| *action);

    if let Some(action) = approval_approve {
        choices.push(vnext_action_decision_choice(
            "run_once",
            t("console.decide.choice.run_once", locale),
            t("console.decide.choice.run_once.consequence", locale),
            "medium",
            "this_run",
            action,
            "normal",
        ));
    }
    if let Some(action) = approval_reject {
        choices.push(vnext_action_decision_choice(
            "skip_once",
            t("console.decide.choice.skip_once", locale),
            t("console.decide.choice.skip_once.consequence", locale),
            "low",
            "this_run",
            action,
            "normal",
        ));
    }
    if approval_approve.is_none() && approval_reject.is_none() {
        choices.push(VNextDecisionChoice {
            id: "approve_unavailable".into(),
            label: t("console.decide.choice.run_once", locale).to_string(),
            consequence: t(
                "console.decide.choice.approve_unavailable.consequence",
                locale,
            )
            .to_string(),
            risk_level: "medium".into(),
            applies_to: "this_run".into(),
            action_kind: None,
            assistant_prompt: None,
            detail_route: None,
            requires_confirmation: false,
            enabled: false,
            disabled_reason: Some(t("console.decide.choice.disabled_no_approval", locale).into()),
            tone: "normal".into(),
        });
        choices.push(VNextDecisionChoice {
            id: "skip_unavailable".into(),
            label: t("console.decide.choice.skip_once", locale).to_string(),
            consequence: t("console.decide.choice.skip_unavailable.consequence", locale)
                .to_string(),
            risk_level: "low".into(),
            applies_to: "this_run".into(),
            action_kind: None,
            assistant_prompt: None,
            detail_route: None,
            requires_confirmation: false,
            enabled: false,
            disabled_reason: Some(t("console.decide.choice.disabled_no_approval", locale).into()),
            tone: "normal".into(),
        });
    }
    choices.push(VNextDecisionChoice {
        id: "revise_rule".into(),
        label: t("console.decide.choice.revise", locale).to_string(),
        consequence: t("console.decide.choice.revise.consequence", locale).to_string(),
        risk_level: "low".into(),
        applies_to: "rule".into(),
        action_kind: None,
        assistant_prompt: Some(t("console.decide.choice.revise.prompt", locale).into()),
        detail_route: None,
        requires_confirmation: false,
        enabled: true,
        disabled_reason: None,
        tone: "normal".into(),
    });
    if let Some(route) = item
        .detail_route
        .as_ref()
        .filter(|route| !route.trim().is_empty())
    {
        choices.push(VNextDecisionChoice {
            id: "view_safe_details".into(),
            label: t("console.decide.choice.view_details", locale).to_string(),
            consequence: t("console.decide.choice.view_details.consequence", locale).to_string(),
            risk_level: "low".into(),
            applies_to: "this_item".into(),
            action_kind: None,
            assistant_prompt: None,
            detail_route: Some(route.clone()),
            requires_confirmation: false,
            enabled: true,
            disabled_reason: None,
            tone: "normal".into(),
        });
    }
    if let Some((_, action)) = projected_actions
        .iter()
        .find(|(kind, _)| kind == "task.snooze_tomorrow")
    {
        choices.push(vnext_action_decision_choice(
            "review_tomorrow",
            t("console.decide.choice.review_tomorrow", locale),
            t("console.decide.choice.review_tomorrow.consequence", locale),
            "low",
            "this_item",
            action,
            "normal",
        ));
    }
    if let Some((_, action)) = projected_actions
        .iter()
        .find(|(kind, _)| kind == "task.complete")
    {
        choices.push(vnext_action_decision_choice(
            "close_pause",
            t("console.decide.choice.close", locale),
            t("console.decide.choice.close.consequence", locale),
            "low",
            "this_item",
            action,
            "destructive",
        ));
    }

    let mut safe_details = vec![
        t("console.decide.safe.no_external", locale).to_string(),
        t("console.decide.safe.visible_metadata", locale).to_string(),
    ];
    let mut safe_detail_codes = vec![
        "safe_no_external_action".to_string(),
        "safe_visible_metadata".to_string(),
    ];
    if approval_approve.is_none() {
        safe_details.push(t("console.decide.safe.no_live_approval", locale).into());
        safe_detail_codes.push("safe_no_live_approval".to_string());
    }

    VNextDecisionRequest {
        id: format!("decision:{}", item.id),
        title: title.into(),
        title_code: Some(title_code.into()),
        what_aegis_was_doing: what,
        what_code: what_code.map(str::to_string),
        blocked_reason,
        blocked_reason_code,
        requested_capability: requested_capability.into(),
        capability_code: Some(capability_code.into()),
        scope: t("console.decide.scope.blocked_run", locale).into(),
        scope_code: Some("scope_blocked_run".into()),
        choices,
        safe_details,
        safe_detail_codes: Some(safe_detail_codes),
        audit_refs: item.related_action_ids.clone(),
        receipt_expectation: item.receipt_expectation.clone(),
    }
}

fn vnext_approval_decision_request(item: &UnifiedInboxItem, locale: &str) -> VNextDecisionRequest {
    let choices = item
        .secondary_actions
        .iter()
        .filter_map(|action| {
            let kind = vnext_action_kind_for_endpoint(&action.action_kind);
            match kind.as_str() {
                "approval.approve" => Some(vnext_action_decision_choice(
                    "approve",
                    t("console.decide.approval.approve", locale),
                    t("console.decide.approval.approve.consequence", locale),
                    "medium",
                    "this_request",
                    action,
                    "normal",
                )),
                "approval.reject" => Some(vnext_action_decision_choice(
                    "reject",
                    t("console.decide.approval.reject", locale),
                    t("console.decide.approval.reject.consequence", locale),
                    "low",
                    "this_request",
                    action,
                    "destructive",
                )),
                _ => None,
            }
        })
        .collect();
    let title = owner_visible_title(
        &item.title,
        t("console.decide.approval.title_fallback", locale),
        120,
    );
    let safe_pairs = vnext_approval_safe_details(item, locale);
    let (safe_details, safe_detail_codes): (Vec<_>, Vec<_>) = safe_pairs.into_iter().unzip();
    VNextDecisionRequest {
        id: format!("decision:{}", item.id),
        title_code: item.title_code.clone().filter(|_| title == item.title),
        title,
        what_aegis_was_doing: item.summary.clone(),
        what_code: None,
        blocked_reason: item.owner_visible_reason.clone(),
        blocked_reason_code: item.owner_visible_reason_code.clone(),
        requested_capability: item.next_safe_action.clone(),
        capability_code: item.next_safe_action_code.as_deref().map(next_code),
        scope: t("console.decide.approval.scope", locale).into(),
        scope_code: Some("scope_approval_request".into()),
        choices,
        safe_details,
        safe_detail_codes: Some(safe_detail_codes),
        audit_refs: item.related_action_ids.clone(),
        receipt_expectation: item.receipt_expectation.clone(),
    }
}

/// Build a `"nextCode:<code>"` structural code (see `VNextDecisionRequest`'s
/// namespace convention). Using these helpers instead of a raw `format!` keeps
/// the namespace spelling in one place — a typo'd namespace silently degrades
/// to server prose on the FE.
fn next_code(code: &str) -> String {
    format!("nextCode:{code}")
}

/// Build a `"reasonCode:<code>"` structural code. See [`next_code`].
fn reason_code(code: &str) -> String {
    format!("reasonCode:{code}")
}

fn vnext_approval_safe_details(item: &UnifiedInboxItem, locale: &str) -> Vec<(String, String)> {
    let mut details = vec![(
        t("console.decide.approval.safe.no_outside", locale).to_string(),
        "safe_no_outside_action".to_string(),
    )];
    let candidates: [(&str, String); 3] = [
        (item.summary.as_str(), String::new()),
        (
            item.next_safe_action.as_str(),
            item.next_safe_action_code
                .as_deref()
                .map(next_code)
                .unwrap_or_default(),
        ),
        (
            item.blocked_by.as_deref().unwrap_or_default(),
            item.blocked_by_code
                .as_deref()
                .map(reason_code)
                .unwrap_or_default(),
        ),
    ];
    for (value, code) in candidates {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !details.iter().any(|(existing, _)| existing == trimmed) {
            details.push((trimmed.to_string(), code));
        }
    }
    details
}

fn vnext_action_decision_choice(
    id: &str,
    label: &str,
    consequence: &str,
    risk_level: &str,
    applies_to: &str,
    action: &UnifiedInboxAction,
    tone: &str,
) -> VNextDecisionChoice {
    VNextDecisionChoice {
        id: id.into(),
        label: label.into(),
        consequence: consequence.into(),
        risk_level: risk_level.into(),
        applies_to: applies_to.into(),
        action_kind: Some(vnext_action_kind_for_endpoint(&action.action_kind)),
        assistant_prompt: None,
        detail_route: action.route.clone(),
        requires_confirmation: !matches!(action.action_kind.as_str(), "navigate" | "read"),
        enabled: true,
        disabled_reason: None,
        tone: tone.into(),
    }
}

fn vnext_authority(
    authority: &ContextAuthorityEnvelope,
    item: &UnifiedInboxItem,
) -> VNextAuthorityEnvelope {
    VNextAuthorityEnvelope {
        basis: vnext_authority_basis(&authority.authority_basis),
        owner_visible_reason: item.owner_visible_reason.clone(),
        visible_data: authority.visible_fields.clone(),
        locked_data: authority.locked_fields.clone(),
        allowed_now: authority.allowed_now.clone(),
        requires_mfa: authority.requires_mfa,
        requires_approval: authority.requires_approval,
        requires_data_grant: authority.requires_data_gate,
        expected_receipt: if authority.expected_receipt.trim().is_empty() {
            None
        } else {
            Some(authority.expected_receipt.clone())
        },
        source_mandate_id: authority.source_mandate_id.clone(),
        audit_refs: authority.audit_links.clone(),
    }
}

fn vnext_action_ref(action: &UnifiedInboxAction, item: &UnifiedInboxItem) -> VNextActionRef {
    let action_kind = vnext_action_kind_for_endpoint(&action.action_kind);
    VNextActionRef {
        id: action_kind.clone(),
        label: action.label.clone(),
        label_code: action.label_code.clone(),
        kind: action_kind,
        route: action.route.clone(),
        requires_confirmation: !matches!(action.action_kind.as_str(), "navigate" | "read"),
        requires_mfa: action.requires_mfa,
        requires_data_gate: action.requires_gate,
        expected_receipt: item.receipt_expectation.clone(),
    }
}

pub(super) fn vnext_action_kind_for_endpoint(action_kind: &str) -> String {
    match action_kind {
        "memory_confirm" => "memory.confirm",
        "memory_pin" => "memory.pin",
        "memory_unpin" => "memory.unpin",
        "memory_mark_wrong" => "memory.mark_wrong",
        "memory_delete" => "memory.delete",
        _ => action_kind,
    }
    .to_string()
}

pub(super) fn vnext_lane_for_item(item: &UnifiedInboxItem) -> &'static str {
    vnext_lane_for_item_inner(item)
}

pub(super) fn vnext_lane_for_item_at(
    item: &UnifiedInboxItem,
    now: chrono::DateTime<chrono::Utc>,
) -> &'static str {
    if follow_up_is_stale(item, now) {
        return "done";
    }
    vnext_lane_for_item_inner(item)
}

fn vnext_lane_for_item_inner(item: &UnifiedInboxItem) -> &'static str {
    let status = item.status.as_str();
    if unified_inbox_task_status_is_closed(status) || item.urgency == "silent" {
        return "done";
    }
    if item.kind == "approval"
        || matches!(
            status,
            "needs_decision" | "waiting_user" | "approval_required" | "needs_approval"
        )
    {
        return "decide";
    }
    if matches!(
        status,
        "waiting" | "tracking" | "scheduled" | "deferred" | "snoozed"
    ) {
        if vnext_follow_up_needs_today_attention(item) {
            return "now";
        }
        return "follow_up";
    }
    if status == "blocked_by_gate" {
        return "decide";
    }
    if item.blocked_by.is_some() && matches!(item.kind.as_str(), "task" | "reminder") {
        return "follow_up";
    }
    if matches!(status, "backlog" | "queued_later" | "someday") || item.urgency == "low" {
        return "backlog";
    }
    "now"
}

fn vnext_follow_up_needs_today_attention(item: &UnifiedInboxItem) -> bool {
    if matches!(
        item.status.as_str(),
        "failed" | "dead_letter" | "needs_recovery"
    ) {
        return true;
    }
    if item.status != "scheduled" {
        return false;
    }
    let Some(due_at) = item.due_at.as_deref() else {
        return false;
    };
    let Ok(due_at) = chrono::DateTime::parse_from_rfc3339(due_at) else {
        return false;
    };
    let now = chrono::Utc::now();
    due_at.with_timezone(&chrono::Utc) <= now + chrono::Duration::hours(48)
}

pub(super) fn vnext_source_kind(item: &UnifiedInboxItem) -> String {
    match item.kind.as_str() {
        "calendar" => "calendar",
        "approval" => "decision",
        "action_draft" => "receipt",
        "receipt" => "receipt",
        "goal" => "goal",
        "memory_review" => "memory",
        "system" | "alert" => "system",
        "waiting_for" => "tracking",
        "task" | "reminder" => "task",
        other => match item.source.as_str() {
            "calendar" => "calendar",
            "goals" => "goal",
            "knowledge" => "tracking",
            "tasks" => "task",
            "mandates" => "mandate",
            "memory" => "memory",
            "action_ledger" => "receipt",
            "alerts" => "system",
            _ => other,
        },
    }
    .to_string()
}

fn vnext_authority_basis(value: &str) -> String {
    match value {
        "direct_request" | "direct_owner_request" => "direct_owner_request",
        "active_mandate" => "active_mandate",
        "system_health" => "system_health",
        _ => "manual_review",
    }
    .to_string()
}

pub(super) fn vnext_suggested_prompts(item: &UnifiedInboxItem, locale: &str) -> Vec<String> {
    let mut prompts = vec![t("console.prompt.what_should", locale).to_string()];
    match vnext_source_kind(item).as_str() {
        "task" => {
            prompts.push(t("console.prompt.remind_tomorrow", locale).into());
            prompts.push(t("console.prompt.break_steps", locale).into());
        }
        _ => {
            prompts.push(t("console.prompt.track", locale).into());
            prompts.push(t("console.prompt.backlog", locale).into());
        }
    }
    prompts
}

pub(super) fn vnext_status_entry(
    projection: &UnifiedInboxProjection,
    locale: &str,
) -> VNextStatusSummary {
    let attention_sources = projection
        .sources
        .iter()
        .filter(|source| vnext_source_needs_attention(&source.status))
        .count();
    if attention_sources > 0 {
        let first_attention = projection
            .sources
            .iter()
            .find(|source| vnext_source_needs_attention(&source.status));
        VNextStatusSummary {
            label: "System Health".into(), // identity: the WebApp localizes section labels
            health: "attention".into(),
            owner_message: if attention_sources == 1 {
                first_attention
                    .map(|source| vnext_source_attention_summary(source, locale))
                    .unwrap_or_else(|| t("console.status.health.one_attention", locale).into())
            } else if let Some(source) = first_attention {
                let extra = (attention_sources - 1).to_string();
                tfp(
                    "console.status.health.more_limited",
                    locale,
                    &[&vnext_source_attention_summary(source, locale), &extra],
                )
            } else {
                tfp(
                    "console.status.health.n_limited",
                    locale,
                    &[&attention_sources.to_string()],
                )
            },
            owner_next_step: first_attention
                .and_then(|source| vnext_source_next_step(source, locale))
                .or_else(|| Some(t("console.status.health.inspect", locale).into())),
        }
    } else {
        VNextStatusSummary {
            label: "System Health".into(), // identity: the WebApp localizes section labels
            health: "ready".into(),
            owner_message: t("console.status.health.ready", locale).into(),
            owner_next_step: None,
        }
    }
}

pub(super) fn vnext_source_needs_attention(status: &str) -> bool {
    matches!(
        status,
        "unavailable" | "error" | "failed" | "auth_required" | "needs_auth" | "permission_required"
    )
}

/// Owner-facing source name for use inside localized sentences (never as a
/// section `label`, which stays the English identity).
pub(super) fn vnext_source_display_label(source: &str, locale: &str) -> String {
    match source {
        "action_ledger" => t("console.source.action_ledger", locale).into(),
        "alerts" => t("console.source.alerts", locale).into(),
        "calendar" => t("console.source.calendar", locale).into(),
        "goals" => t("console.source.goals", locale).into(),
        "hitl" => t("console.source.hitl", locale).into(),
        "knowledge" => t("console.source.knowledge", locale).into(),
        "mandates" => t("console.source.mandates", locale).into(),
        "memory" => t("console.source.memory", locale).into(),
        "tasks" => t("console.source.tasks", locale).into(),
        _ => vnext_source_label(source, locale),
    }
}

/// Source identity label. The WebApp localizes these (localizedSourceLabel keyed
/// by the English identity), so this stays English regardless of locale.
pub(super) fn vnext_source_label(source: &str, _locale: &str) -> String {
    match source {
        "action_ledger" => "Receipts".into(),
        "alerts" => "System alerts".into(),
        "calendar" => "Calendar".into(),
        "goals" => "Goals".into(),
        "hitl" => "Approvals".into(),
        "knowledge" => "Waiting items".into(),
        "mandates" => "Standing instructions".into(),
        "memory" => "Learning".into(),
        "tasks" => "Tasks and reminders".into(),
        _ => source
            .split('_')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub(super) fn vnext_source_attention_summary(
    source: &UnifiedInboxSourceStatus,
    locale: &str,
) -> String {
    let reason = source
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|reason| vnext_source_reason(reason, locale))
        .unwrap_or_else(|| vnext_source_status_label(&source.status, locale));
    tfp(
        "console.source.attention",
        locale,
        &[&vnext_source_display_label(&source.source, locale), &reason],
    )
}

pub(super) fn vnext_source_status_label(status: &str, locale: &str) -> String {
    match status {
        "auth_required" | "needs_auth" => t("console.source.status.auth", locale).into(),
        "permission_required" => t("console.source.status.permission", locale).into(),
        "unavailable" => t("console.source.status.unavailable", locale).into(),
        "error" => t("console.source.status.error", locale).into(),
        "failed" => t("console.source.status.failed", locale).into(),
        "delegated" => t("console.source.status.delegated", locale).into(),
        _ => status.replace('_', " "),
    }
}

pub(super) fn vnext_source_reason(reason: &str, locale: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("metadata only") {
        t("console.source.reason.metadata", locale).into()
    } else if lower.contains("not connected yet") {
        t("console.source.reason.not_connected", locale).into()
    } else if lower.contains("projector") {
        t("console.source.reason.projector", locale).into()
    } else {
        reason.to_string()
    }
}

pub(super) fn vnext_source_next_step(
    source: &UnifiedInboxSourceStatus,
    locale: &str,
) -> Option<String> {
    match source.status.as_str() {
        "auth_required" | "needs_auth" => Some(t("console.source.next.auth", locale).into()),
        "permission_required" => Some(t("console.source.next.permission", locale).into()),
        "unavailable" | "error" | "failed" => {
            Some(t("console.source.next.unavailable", locale).into())
        }
        "delegated" | "deferred" => None,
        _ => None,
    }
}

pub(super) fn vnext_principal_label(principal: &WebConsolePrincipal, locale: &str) -> String {
    match principal {
        WebConsolePrincipal::Telegram(verified) => tfp(
            "console.principal.telegram",
            locale,
            &[&verified.user.id.to_string()],
        ),
        WebConsolePrincipal::WebConsole(session) => session.email.clone(),
        #[cfg(test)]
        WebConsolePrincipal::DebugLease(lease) => format!("Debug lease {}", lease.id),
    }
}

/// GET /webapp/console/memory/items/:item_id — gated memory detail placeholder.
pub(super) async fn webapp_console_memory_item(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let memory_id = item_id.parse::<i64>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        )
    })?;
    let access = webapp_console_check_detail_access(
        &state,
        req.headers(),
        "memory_item",
        &item_id,
        "web_console_memory_detail",
    )
    .await?;

    match access {
        WebappConsoleDetailAccess::Allow {
            grant_id,
            reason,
            user_did,
            ..
        } => {
            let item = state
                .memory
                .get_l2_by_id(&user_did, memory_id)
                .await
                .map_err(|error| {
                    tracing::warn!(error = %error, "Web Console memory detail lookup failed");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to load memory detail".to_string(),
                    )
                })?
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Memory item not found".to_string()))?;

            Ok(Json(serde_json::json!({
                "status": "ok",
                "resource_type": "memory_item",
                "resource_id": item_id,
                "access": "allowed",
                "grant_id": grant_id,
                "reason": reason,
                "detail": {
                    "status": "available",
                    "id": item.id,
                    "did": item.agent_did,
                    "topic": item.topic,
                    "fact_summary": item.fact_summary,
                    "confidence": item.confidence,
                    "is_pinned": item.is_pinned,
                    "created_at": item.created_at,
                },
            })))
        }
        WebappConsoleDetailAccess::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => Ok(Json(serde_json::json!({
            "status": "mfa_required",
            "resource_type": "memory_item",
            "resource_id": item_id,
            "required_level": required_level,
            "reason": reason,
            "freshness_secs": freshness_secs,
            "challenge_hint": {
                "start_url": "/ui/?tab=system",
            },
            "detail": {
                "status": "locked",
                "message": t("console.mfa.memory", &state.settings.display_language()),
            },
        }))),
    }
}

fn is_safe_profile_slot_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

/// GET /webapp/console/memory/profile/:category/:key — gated profile slot detail.
pub(super) async fn webapp_console_profile_slot(
    State(state): State<Arc<AdminState>>,
    Path((category, key)): Path<(String, String)>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_safe_profile_slot_component(&category) || !is_safe_profile_slot_component(&key) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid profile slot id".to_string(),
        ));
    }
    let resource_id = format!("{category}.{key}");
    let access = webapp_console_check_detail_access(
        &state,
        req.headers(),
        "profile_slot",
        &resource_id,
        "web_console_profile_detail",
    )
    .await?;

    match access {
        WebappConsoleDetailAccess::Allow {
            grant_id,
            reason,
            user_did,
            ..
        } => {
            let entry = state
                .memory
                .get_user_profile(&user_did)
                .await
                .map_err(|error| {
                    tracing::warn!(error = %error, "Web Console profile detail lookup failed");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to load profile slot detail".to_string(),
                    )
                })?
                .into_iter()
                .find(|entry| entry.category == category && entry.key == key)
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Profile slot not found".to_string()))?;

            Ok(Json(serde_json::json!({
                "status": "ok",
                "resource_type": "profile_slot",
                "resource_id": resource_id,
                "access": "allowed",
                "grant_id": grant_id,
                "reason": reason,
                "detail": {
                    "status": "available",
                    "id": entry.id,
                    "did": entry.agent_did,
                    "category": entry.category,
                    "key": entry.key,
                    "value": entry.value,
                    "confidence": entry.confidence,
                    "source": entry.source,
                    "created_at": entry.created_at,
                    "updated_at": entry.updated_at,
                },
            })))
        }
        WebappConsoleDetailAccess::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => Ok(Json(serde_json::json!({
            "status": "mfa_required",
            "resource_type": "profile_slot",
            "resource_id": resource_id,
            "required_level": required_level,
            "reason": reason,
            "freshness_secs": freshness_secs,
            "challenge_hint": {
                "start_url": "/ui/?tab=system",
            },
            "detail": {
                "status": "locked",
                "message": t("console.mfa.profile", &state.settings.display_language()),
            },
        }))),
    }
}

#[derive(serde::Deserialize)]
struct WebappConsoleProfileSlotUpdateRequest {
    value: String,
}

/// POST /webapp/console/memory/profile/:category/:key/update — owner-session profile correction.
pub(super) async fn webapp_console_profile_slot_update(
    State(state): State<Arc<AdminState>>,
    Path((category, key)): Path<(String, String)>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers)
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(&headers, &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;

    if !is_safe_profile_slot_component(&category) || !is_safe_profile_slot_component(&key) {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid profile slot id".to_string(),
        )));
    }
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }
    let resource_id = format!("{category}.{key}");
    check_webapp_rate_limit(
        &principal,
        "console_profile_update",
        &resource_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let bytes = axum::body::to_bytes(body, 16 * 1024).await.map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid profile update request body".to_string(),
        ))
    })?;
    let body: WebappConsoleProfileSlotUpdateRequest =
        serde_json::from_slice(&bytes).map_err(|_| {
            webapp_console_json_error((
                StatusCode::BAD_REQUEST,
                "Invalid profile update request JSON".to_string(),
            ))
        })?;
    let value = body.value.trim();
    if value.is_empty() {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Profile slot value must not be empty".to_string(),
        )));
    }
    if value.chars().count() > WEBAPP_PROFILE_UPDATE_VALUE_MAX_CHARS {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Profile slot value is too long".to_string(),
        )));
    }

    let user_did = format!("did:aegis:telegram:{chat_id}");
    let existing = state
        .memory
        .get_user_profile(&user_did)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console profile update lookup failed");
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load profile slot".to_string(),
            ))
        })?
        .into_iter()
        .find(|entry| entry.category == category && entry.key == key)
        .ok_or_else(|| {
            webapp_console_json_error((StatusCode::NOT_FOUND, "Profile slot not found".to_string()))
        })?;

    let action_id = format!(
        "web-console-profile-update:{}:{}:{}",
        category,
        key,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("profile_update:{resource_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Update profile slot {resource_id}")),
                user_visible_summary: Some(format!("Profile slot {resource_id} update requested")),
                idempotency_key: Some(format!("profile-update:{user_did}:{resource_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "profile_slot",
                        "resource_id": resource_id,
                        "category": category.clone(),
                        "key": key.clone(),
                        "previous_source": existing.source.clone(),
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console profile update ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create profile update receipt".to_string(),
                ))
            })?;
    }

    let now = chrono::Utc::now().to_rfc3339();
    state
        .memory
        .upsert_profile_entry(crate::memory::ProfileEntry {
            id: existing.id,
            agent_did: user_did.clone(),
            category: category.clone(),
            key: key.clone(),
            value: value.to_string(),
            confidence: 1.0,
            source: "manual".into(),
            created_at: existing.created_at,
            updated_at: now,
        })
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console profile update failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("profile_update_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update profile slot".to_string(),
            ))
        })?;

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!("Updated profile slot {resource_id}")),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console profile update ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Profile slot updated but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "profile_slot",
        "resource_id": resource_id,
        "updated": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "detail": {
            "status": "available",
            "category": category,
            "key": key,
            "value": value,
            "confidence": 1.0,
            "source": "manual",
        },
    })))
}

#[derive(serde::Deserialize)]
struct WebappConsoleMemoryItemUpdateRequest {
    fact_summary: String,
}

/// POST /webapp/console/memory/items/:item_id/update — owner-session L2 fact correction.
pub(super) async fn webapp_console_memory_item_update(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers)
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(&headers, &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;

    let memory_id = item_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        ))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }
    check_webapp_rate_limit(
        &principal,
        "console_memory_update",
        &item_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let bytes = axum::body::to_bytes(body, 16 * 1024).await.map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory update request body".to_string(),
        ))
    })?;
    let payload: WebappConsoleMemoryItemUpdateRequest =
        serde_json::from_slice(&bytes).map_err(|_| {
            webapp_console_json_error((
                StatusCode::BAD_REQUEST,
                "Invalid memory update request JSON".to_string(),
            ))
        })?;
    let fact_summary = payload.fact_summary.trim();
    if fact_summary.is_empty() {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Memory fact summary must not be empty".to_string(),
        )));
    }
    if fact_summary.chars().count() > WEBAPP_MEMORY_UPDATE_VALUE_MAX_CHARS {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Memory fact summary is too long".to_string(),
        )));
    }

    let user_did = format!("did:aegis:telegram:{chat_id}");
    let item = webapp_console_get_memory_item_by_id(&state, &user_did, memory_id, "update").await?;

    let action_id = format!(
        "web-console-memory-update:{}:{}",
        memory_id,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("memory_update:{memory_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Update memory item #{memory_id}")),
                user_visible_summary: Some(format!("Memory item #{memory_id} update requested")),
                idempotency_key: Some(format!("memory-update:{user_did}:{memory_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "memory_item",
                        "resource_id": memory_id,
                        "topic": item.topic.clone(),
                        "previous_confidence": item.confidence,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console memory update ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create memory update receipt".to_string(),
                ))
            })?;
    }

    let updated = state
        .memory
        .update_l2(&user_did, memory_id, fact_summary, 1.0)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console memory update failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("memory_update_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update memory item".to_string(),
            ))
        })?;
    if !updated {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("memory_item_missing".into()),
                error_message: Some("Memory item disappeared before update".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Memory item not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!("Updated memory item #{memory_id}")),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console memory update ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Memory updated but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "memory_item",
        "resource_id": item_id,
        "updated": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "detail": {
            "status": "available",
            "id": item.id,
            "did": item.agent_did,
            "topic": item.topic,
            "fact_summary": fact_summary,
            "confidence": 1.0,
            "created_at": item.created_at,
        },
    })))
}

async fn webapp_console_get_memory_item_by_id(
    state: &AdminState,
    user_did: &str,
    memory_id: i64,
    operation: &str,
) -> Result<crate::memory::Experience, (StatusCode, Json<serde_json::Value>)> {
    state
        .memory
        .get_l2_by_id(user_did, memory_id)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, operation, "Web Console memory lookup failed");
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load memory item".to_string(),
            ))
        })?
        .ok_or_else(|| {
            webapp_console_json_error((StatusCode::NOT_FOUND, "Memory item not found".to_string()))
        })
}

/// POST /webapp/console/memory/items/:item_id/confirm — receipt-backed stale-memory review.
pub(super) async fn webapp_console_memory_item_confirm(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let memory_id = item_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        ))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }

    let principal = extract_web_console_auth(&state, req.headers())
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(req.headers(), &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    check_webapp_rate_limit(
        &principal,
        "console_memory_confirm",
        &item_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let user_did = format!("did:aegis:telegram:{chat_id}");
    let item =
        webapp_console_get_memory_item_by_id(&state, &user_did, memory_id, "confirm").await?;

    let action_id = format!(
        "web-console-memory-confirm:{}:{}",
        memory_id,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("memory_confirm:{memory_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Confirm memory item #{memory_id}")),
                user_visible_summary: Some(format!("Memory item #{memory_id} confirmed")),
                idempotency_key: Some(format!("memory-confirm:{user_did}:{memory_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "memory_item",
                        "resource_id": memory_id,
                        "topic": item.topic.clone(),
                        "operation": "confirm",
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console memory confirm ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create memory confirm receipt".to_string(),
                ))
            })?;
    }

    let touched = state
        .memory
        .touch_l2_by_id(&user_did, memory_id)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console memory confirm failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("memory_confirm_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to confirm memory item".to_string(),
            ))
        })?;
    if !touched {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("memory_item_missing".into()),
                error_message: Some("Memory item disappeared before confirm".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Memory item not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!("Confirmed memory item #{memory_id}")),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console memory confirm ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Memory confirmed but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "memory_item",
        "resource_id": item_id,
        "updated": true,
        "confirmed": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "summary": t("console.memory.confirmed", &state.settings.display_language()),
        "detail": {
            "status": "available",
            "id": item.id,
            "did": item.agent_did,
            "topic": item.topic,
            "fact_summary": item.fact_summary,
            "confidence": item.confidence,
            "is_pinned": item.is_pinned,
            "created_at": item.created_at,
        },
    })))
}

pub(super) async fn webapp_console_memory_item_pin(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_memory_item_set_pin(state, item_id, req, true).await
}

pub(super) async fn webapp_console_memory_item_unpin(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_memory_item_set_pin(state, item_id, req, false).await
}

async fn webapp_console_memory_item_set_pin(
    state: Arc<AdminState>,
    item_id: String,
    req: Request<axum::body::Body>,
    pinned: bool,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let memory_id = item_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        ))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }

    let principal = extract_web_console_auth(&state, req.headers())
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(req.headers(), &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    let operation = if pinned { "pin" } else { "unpin" };
    check_webapp_rate_limit(
        &principal,
        "console_memory_pin",
        &format!("{operation}:{item_id}"),
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let user_did = format!("did:aegis:telegram:{chat_id}");
    let item =
        webapp_console_get_memory_item_by_id(&state, &user_did, memory_id, operation).await?;

    let action_id = format!(
        "web-console-memory-{operation}:{}:{}",
        memory_id,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("memory_{operation}:{memory_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("{operation} memory item #{memory_id}")),
                user_visible_summary: Some(format!(
                    "Memory item #{memory_id} {operation} requested"
                )),
                idempotency_key: Some(format!("memory-{operation}:{user_did}:{memory_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "memory_item",
                        "resource_id": memory_id,
                        "topic": item.topic.clone(),
                        "pinned": pinned,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console memory pin ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create memory pin receipt".to_string(),
                ))
            })?;
    }

    let updated = state
        .memory
        .set_l2_pinned(&user_did, memory_id, pinned)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console memory pin failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("memory_pin_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update memory pin".to_string(),
            ))
        })?;
    if !updated {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("memory_item_missing".into()),
                error_message: Some("Memory item disappeared before pin update".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Memory item not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!(
                "{} memory item #{memory_id}",
                if pinned { "Pinned" } else { "Unpinned" }
            )),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console memory pin ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Memory pin updated but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "memory_item",
        "resource_id": item_id,
        "updated": true,
        "pinned": pinned,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "detail": {
            "status": "available",
            "id": item.id,
            "did": item.agent_did,
            "topic": item.topic,
            "fact_summary": item.fact_summary,
            "confidence": item.confidence,
            "is_pinned": pinned,
            "created_at": item.created_at,
        },
    })))
}

/// POST /webapp/console/memory/items/:item_id/mark-wrong — owner-session soft rejection.
pub(super) async fn webapp_console_memory_item_mark_wrong(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let memory_id = item_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        ))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }

    let principal = extract_web_console_auth(&state, req.headers())
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(req.headers(), &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    check_webapp_rate_limit(
        &principal,
        "console_memory_mark_wrong",
        &item_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let user_did = format!("did:aegis:telegram:{chat_id}");
    let item =
        webapp_console_get_memory_item_by_id(&state, &user_did, memory_id, "mark-wrong").await?;

    let action_id = format!(
        "web-console-memory-mark-wrong:{}:{}",
        memory_id,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("memory_mark_wrong:{memory_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbUpdate,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Mark memory item #{memory_id} wrong")),
                user_visible_summary: Some(format!(
                    "Memory item #{memory_id} mark-wrong requested"
                )),
                idempotency_key: Some(format!("memory-mark-wrong:{user_did}:{memory_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "memory_item",
                        "resource_id": memory_id,
                        "topic": item.topic.clone(),
                        "operation": "mark_wrong",
                        "previous_confidence": item.confidence,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console memory mark-wrong ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create memory mark-wrong receipt".to_string(),
                ))
            })?;
    }

    let tombstone = t(
        "console.memory.marked_wrong",
        &state.settings.display_language(),
    );
    let updated = state
        .memory
        .update_l2(&user_did, memory_id, tombstone, 0.0)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console memory mark-wrong failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("memory_mark_wrong_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to mark memory wrong".to_string(),
            ))
        })?;
    if !updated {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("memory_item_missing".into()),
                error_message: Some("Memory item disappeared before mark-wrong".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Memory item not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!("Marked memory item #{memory_id} wrong")),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console memory mark-wrong ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Memory marked wrong but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "memory_item",
        "resource_id": item_id,
        "updated": true,
        "marked_wrong": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "detail": {
            "status": "available",
            "id": item.id,
            "did": item.agent_did,
            "topic": item.topic,
            "fact_summary": tombstone,
            "confidence": 0.0,
            "is_pinned": item.is_pinned,
            "created_at": item.created_at,
        },
    })))
}

/// POST /webapp/console/memory/items/:item_id/delete — owner-session memory delete.
pub(super) async fn webapp_console_memory_item_delete(
    State(state): State<Arc<AdminState>>,
    Path(item_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let memory_id = item_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid memory item id".to_string(),
        ))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }

    let principal = extract_web_console_auth(&state, req.headers())
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(req.headers(), &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    check_webapp_rate_limit(
        &principal,
        "console_memory_delete",
        &item_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;
    let user_did = format!("did:aegis:telegram:{chat_id}");
    let item = webapp_console_get_memory_item_by_id(&state, &user_did, memory_id, "delete").await?;

    let action_id = format!(
        "web-console-memory-delete:{}:{}",
        memory_id,
        chrono::Utc::now().timestamp_millis()
    );
    let receipt = format!("memory_delete:{memory_id}:{action_id}");
    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind: crate::action_ledger::ActionKind::KbDelete,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Delete memory item #{memory_id}")),
                user_visible_summary: Some(format!("Memory item #{memory_id} deletion requested")),
                idempotency_key: Some(format!("memory-delete:{user_did}:{memory_id}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "memory_item",
                        "resource_id": memory_id,
                        "topic": item.topic,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "Web Console memory delete ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create deletion receipt".to_string(),
                ))
            })?;
    }

    let deleted = state
        .memory
        .delete_l2(&user_did, memory_id)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Web Console memory delete failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("memory_delete_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete memory item".to_string(),
            ))
        })?;
    if !deleted {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("memory_item_missing".into()),
                error_message: Some("Memory item disappeared before deletion".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Memory item not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(format!("Deleted memory item #{memory_id}")),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, "Web Console memory delete ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Memory deleted but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "memory_item",
        "resource_id": item_id,
        "deleted": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
    })))
}

/// GET /webapp/console/tasks/:task_id/timeline — gated task timeline placeholder.
pub(super) async fn webapp_console_task_timeline(
    State(state): State<Arc<AdminState>>,
    Path(task_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let parsed_task_id = task_id
        .parse::<i64>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid task id".to_string()))?;
    let access = webapp_console_check_detail_access(
        &state,
        req.headers(),
        "task_timeline",
        &task_id,
        "web_console_task_timeline",
    )
    .await?;

    match access {
        WebappConsoleDetailAccess::Allow {
            grant_id,
            reason,
            user_did,
            chat_id,
        } => {
            let task = state
                .memory
                .get_task(parsed_task_id)
                .await
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Task not found".to_string()))?;
            if task.chat_id != chat_id || task.user_did != user_did {
                return Err((StatusCode::NOT_FOUND, "Task not found".to_string()));
            }
            let timeline = build_webapp_task_timeline_events(&task);

            Ok(Json(serde_json::json!({
                "status": "ok",
                "resource_type": "task_timeline",
                "resource_id": task_id,
                "access": "allowed",
                "grant_id": grant_id,
                "reason": reason,
                "detail": {
                    "status": "available",
                    "task": {
                        "id": task.id,
                        "chat_id": task.chat_id,
                        "user_did": &task.user_did,
                        "title": &task.title,
                        "body": &task.body,
                        "priority": &task.priority,
                        "state": &task.status,
                        "tags": &task.tags,
                        "due_date": &task.due_date,
                        "source": &task.source,
                        "execution_mode": &task.execution_mode,
                        "context_text": &task.context_text,
                        "fire_count": task.fire_count,
                        "retry_count": task.retry_count,
                        "chain_depth": task.chain_depth,
                        "created_at": &task.created_at,
                        "updated_at": &task.updated_at,
                        "completed_at": &task.completed_at,
                    },
                    "timeline": timeline,
                },
            })))
        }
        WebappConsoleDetailAccess::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => Ok(Json(serde_json::json!({
            "status": "mfa_required",
            "resource_type": "task_timeline",
            "resource_id": task_id,
            "required_level": required_level,
            "reason": reason,
            "freshness_secs": freshness_secs,
            "challenge_hint": {
                "start_url": "/ui/?tab=system",
            },
            "detail": {
                "status": "locked",
                "message": t("console.mfa.task", &state.settings.display_language()),
            },
        }))),
    }
}

/// POST /webapp/console/tasks/:task_id/action — receipt-backed task/reminder update.
pub(super) async fn webapp_console_task_action(
    State(state): State<Arc<AdminState>>,
    Path(task_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let parsed_task_id = task_id.parse::<i64>().map_err(|_| {
        webapp_console_json_error((StatusCode::BAD_REQUEST, "Invalid task id".to_string()))
    })?;
    if state.memory.db_path() == ":memory:" {
        return Err(webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable for in-memory console state".to_string(),
        )));
    }

    let (parts, body) = req.into_parts();
    let principal = extract_web_console_auth(&state, &parts.headers)
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(&parts.headers, &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    check_webapp_rate_limit(
        &principal,
        "console_task_action",
        &task_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;

    let body_bytes = axum::body::to_bytes(body, 8 * 1024).await.map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid task action body".to_string(),
        ))
    })?;
    let request: WebappTaskActionRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid task action JSON".to_string(),
        ))
    })?;
    validate_context_schema_version(request.schema_version).map_err(webapp_console_json_error)?;
    let action = request.action.as_str();
    if !matches!(action, "complete" | "snooze" | "delete") {
        return Err(webapp_console_json_error((
            StatusCode::BAD_REQUEST,
            "Invalid task action".to_string(),
        )));
    }

    let user_did = format!("did:aegis:telegram:{chat_id}");
    let task = state.memory.get_task(parsed_task_id).await.ok_or_else(|| {
        webapp_console_json_error((StatusCode::NOT_FOUND, "Task not found".to_string()))
    })?;
    if task.chat_id != chat_id || task.user_did != user_did {
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Task not found".to_string(),
        )));
    }

    let now = chrono::Utc::now();
    let user_tz = crate::settings::parse_user_tz(state.settings.as_ref());
    let (updates, action_kind, receipt_suffix, summary, due_at) =
        build_webapp_task_action_update(&task, action, request.preset.as_deref(), now, user_tz)
            .map_err(webapp_console_json_error)?;
    let action_id = format!(
        "web-console-task-{action}:{}:{}",
        parsed_task_id,
        now.timestamp_millis()
    );
    let receipt = format!("task_{action}:{parsed_task_id}:{receipt_suffix}:{action_id}");

    {
        let conn = webapp_console_action_ledger_conn(&state)?;
        let store = crate::action_ledger::ActionLedgerStore::new(&conn);
        store
            .create_proposed(&crate::action_ledger::NewAction {
                action_id: action_id.clone(),
                action_kind,
                requested_by: Some(user_did.clone()),
                chat_id: Some(chat_id),
                input_summary: Some(format!("Task #{parsed_task_id} {action} requested")),
                user_visible_summary: Some(summary.clone()),
                idempotency_key: Some(format!("task-action:{user_did}:{parsed_task_id}:{action}:{receipt_suffix}")),
                metadata_json: Some(
                    serde_json::json!({
                        "resource_type": "task",
                        "resource_id": parsed_task_id,
                        "action": action,
                        "preset": receipt_suffix,
                        "source": "web_console",
                    })
                    .to_string(),
                ),
                ..Default::default()
            })
            .and_then(|_| {
                store.transition(
                    &action_id,
                    crate::action_ledger::ActionStatus::Executing,
                    &crate::action_ledger::ActionUpdate::default(),
                )
            })
            .map_err(|error| {
                tracing::warn!(error = %error, task_id = parsed_task_id, action, "Web Console task action ledger create failed");
                webapp_console_json_error((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create task action receipt".to_string(),
                ))
            })?;
    }

    let updated = state
        .memory
        .update_task(parsed_task_id, chat_id, &updates.to_string())
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, task_id = parsed_task_id, action, "Web Console task action failed");
            let _ = webapp_console_transition_action_ledger(
                &state,
                &action_id,
                crate::action_ledger::ActionStatus::Failed,
                crate::action_ledger::ActionUpdate {
                    error_kind: Some("task_action_failed".into()),
                    error_message: Some(error.to_string()),
                    ..Default::default()
                },
            );
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update task".to_string(),
            ))
        })?;
    if !updated {
        let _ = webapp_console_transition_action_ledger(
            &state,
            &action_id,
            crate::action_ledger::ActionStatus::Failed,
            crate::action_ledger::ActionUpdate {
                error_kind: Some("task_not_updated".into()),
                error_message: Some("Task update changed no rows".into()),
                ..Default::default()
            },
        );
        return Err(webapp_console_json_error((
            StatusCode::NOT_FOUND,
            "Task not found".to_string(),
        )));
    }

    let entry = webapp_console_transition_action_ledger(
        &state,
        &action_id,
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionUpdate {
            external_receipt: Some(receipt.clone()),
            external_event_id: Some(action_id.clone()),
            user_visible_summary: Some(summary.clone()),
            ..Default::default()
        },
    )
    .map_err(|error| {
        tracing::warn!(error = %error, task_id = parsed_task_id, action, "Web Console task action ledger execute failed");
        webapp_console_json_error((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Task updated but receipt update failed".to_string(),
        ))
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "resource_type": "task",
        "resource_id": task_id,
        "action": action,
        "updated": true,
        "ledger_entry_id": entry.id,
        "receipt": receipt,
        "summary": summary,
        "due_at": due_at,
        "recurring": action == "complete"
            && updates.get("status").and_then(serde_json::Value::as_str) == Some("active"),
    })))
}

/// Completing one fire of a clock loop keeps the same row active and moves
/// the next trigger. One-shot tasks still close.
fn recurring_complete_updates(
    task: &crate::memory::Task,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<serde_json::Value> {
    if !super::inbox_projection::clock_loop_still_scheduled(task) {
        return None;
    }
    let interval = task.interval_seconds?;
    let next = super::inbox_projection::next_open_clock_trigger(
        now.timestamp(),
        task.trigger_at_unix,
        interval,
    );
    Some(serde_json::json!({
        "status": "active",
        "trigger_at_unix": next,
        "clear_fields": ["completed_at"],
    }))
}

pub(super) fn build_webapp_task_action_update(
    task: &crate::memory::Task,
    action: &str,
    preset: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    user_tz: chrono_tz::Tz,
) -> Result<
    (
        serde_json::Value,
        crate::action_ledger::ActionKind,
        String,
        String,
        Option<String>,
    ),
    (StatusCode, String),
> {
    let scheduled = task.trigger_at_unix.is_some() || task.execution_mode == "notify";
    match action {
        "complete" => {
            if let Some(updates) = recurring_complete_updates(task, now) {
                let due = updates
                    .get("trigger_at_unix")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|unix| chrono::DateTime::<chrono::Utc>::from_timestamp(unix, 0))
                    .map(|stamp| stamp.to_rfc3339());
                Ok((
                    updates,
                    crate::action_ledger::ActionKind::ReminderUpdate,
                    "complete".into(),
                    format!("Task #{} marked complete", task.id),
                    due,
                ))
            } else {
                Ok((
                    serde_json::json!({
                        "status": "done",
                        "completed_at": now.to_rfc3339(),
                    }),
                    if scheduled {
                        crate::action_ledger::ActionKind::ReminderUpdate
                    } else {
                        crate::action_ledger::ActionKind::Other
                    },
                    "complete".into(),
                    format!("Task #{} marked complete", task.id),
                    None,
                ))
            }
        }
        "delete" => Ok((
            serde_json::json!({
                "status": "archived",
            }),
            if scheduled {
                crate::action_ledger::ActionKind::ReminderDelete
            } else {
                crate::action_ledger::ActionKind::Other
            },
            "archive".into(),
            format!("Task #{} archived", task.id),
            None,
        )),
        "snooze" => {
            let preset = preset.unwrap_or("1h");
            let delay = match preset {
                "10m" => chrono::Duration::minutes(10),
                "1h" => chrono::Duration::hours(1),
                "tomorrow" => chrono::Duration::days(1),
                _ => {
                    return Err((StatusCode::BAD_REQUEST, "Invalid snooze preset".to_string()));
                }
            };
            let due = now + delay;
            let local_due = due.with_timezone(&user_tz).format("%Y-%m-%d %H:%M %Z");
            Ok((
                serde_json::json!({
                    "status": "active",
                    "trigger_at_unix": due.timestamp(),
                    "due_date": due.to_rfc3339(),
                }),
                crate::action_ledger::ActionKind::ReminderUpdate,
                preset.into(),
                format!("Task #{} snoozed until {}", task.id, local_due),
                Some(due.to_rfc3339()),
            ))
        }
        _ => Err((StatusCode::BAD_REQUEST, "Invalid task action".to_string())),
    }
}

/// GET /webapp/console/ledger/:entry_id — gated action receipt placeholder.
pub(super) async fn webapp_console_ledger_entry(
    State(state): State<Arc<AdminState>>,
    Path(entry_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let parsed_entry_id = entry_id.parse::<i64>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid action ledger id".to_string(),
        )
    })?;
    let access = webapp_console_check_detail_access(
        &state,
        req.headers(),
        "action_ledger_entry",
        &entry_id,
        "web_console_action_receipt_detail",
    )
    .await?;

    match access {
        WebappConsoleDetailAccess::Allow {
            grant_id,
            reason,
            user_did,
            chat_id,
        } => {
            if state.memory.db_path() == ":memory:" {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Action Ledger is unavailable for in-memory console state".to_string(),
                ));
            }
            let entry = crate::encrypted_sqlite::open(state.memory.db_path())
                .map_err(|error| {
                    tracing::warn!(error = %error, "Web Console action ledger storage open failed");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Action Ledger is unavailable".to_string(),
                    )
                })
                .and_then(|conn| {
                    crate::action_ledger::ActionLedgerStore::new(&conn)
                        .get_by_id(parsed_entry_id)
                        .map_err(|error| {
                            tracing::warn!(error = %error, "Web Console action ledger detail lookup failed");
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Failed to load action ledger detail".to_string(),
                            )
                        })
                })?
                .ok_or_else(|| (StatusCode::NOT_FOUND, "Action ledger entry not found".to_string()))?;
            if !webapp_action_ledger_entry_belongs_to_owner(&entry, chat_id, &user_did) {
                return Err((
                    StatusCode::NOT_FOUND,
                    "Action ledger entry not found".to_string(),
                ));
            }
            let action_status = trust_console_action_status_label(&entry);
            let receipt_status = trust_console_receipt_status(&entry);

            Ok(Json(serde_json::json!({
                "status": "ok",
                "resource_type": "action_ledger_entry",
                "resource_id": entry_id,
                "access": "allowed",
                "grant_id": grant_id,
                "reason": reason,
                "detail": {
                    "status": "available",
                    "entry": {
                        "id": entry.id,
                        "action_id": &entry.action_id,
                        "action_kind": entry.action_kind.as_str(),
                        "action_status": action_status,
                        "raw_status": entry.status.as_str(),
                        "requested_by": &entry.requested_by,
                        "chat_id": entry.chat_id,
                        "created_at": &entry.created_at,
                        "proposed_at": &entry.proposed_at,
                        "approved_at": &entry.approved_at,
                        "executing_at": &entry.executing_at,
                        "executed_at": &entry.executed_at,
                        "failed_at": &entry.failed_at,
                        "cancelled_at": &entry.cancelled_at,
                        "input_summary": &entry.input_summary,
                        "user_visible_summary": &entry.user_visible_summary,
                        "external_receipt": &entry.external_receipt,
                        "external_message_id": &entry.external_message_id,
                        "external_event_id": &entry.external_event_id,
                        "tool_call_id": &entry.tool_call_id,
                        "assistant_event_id": entry.assistant_event_id,
                        "provider_request_id": &entry.provider_request_id,
                        "idempotency_key": &entry.idempotency_key,
                        "error_kind": &entry.error_kind,
                        "error_message": &entry.error_message,
                        "retry_policy": &entry.retry_policy,
                        "trace_id": &entry.trace_id,
                        "run_id": &entry.run_id,
                        "mandate_id": &entry.mandate_id,
                        "source_event_id": &entry.source_event_id,
                        "metadata_json": &entry.metadata_json,
                    },
                    "receipt_status": receipt_status,
                },
            })))
        }
        WebappConsoleDetailAccess::RequireMfa {
            required_level,
            reason,
            freshness_secs,
        } => Ok(Json(serde_json::json!({
            "status": "mfa_required",
            "resource_type": "action_ledger_entry",
            "resource_id": entry_id,
            "required_level": required_level,
            "reason": reason,
            "freshness_secs": freshness_secs,
            "challenge_hint": {
                "start_url": "/ui/?tab=system",
            },
            "detail": {
                "status": "locked",
                "message": t("console.mfa.receipt", &state.settings.display_language()),
            },
        }))),
    }
}

pub(super) async fn webapp_console_approval_approve(
    State(state): State<Arc<AdminState>>,
    Path(approval_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_approval_action(state, req.headers(), approval_id, true).await
}

/// POST /webapp/console/approvals/:approval_id/reject — reject an existing HITL approval.
pub(super) async fn webapp_console_approval_reject(
    State(state): State<Arc<AdminState>>,
    Path(approval_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_approval_action(state, req.headers(), approval_id, false).await
}

pub(super) async fn webapp_console_mandate_create(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let principal = extract_web_console_auth(&state, &parts.headers).await?;
    require_web_console_csrf_for_principal(&parts.headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let request = parse_webapp_mandate_request(body).await?;
    let new_mandate = build_new_user_mandate(chat_id, "webapp", None, request)?;
    let conn = webapp_user_mandate_conn(&state).map_err(webapp_mandate_string_error)?;
    let store = crate::user_mandates::UserMandateStore::new(&conn);
    let mandate = store.create_draft(&new_mandate).map_err(|error| {
        tracing::warn!(error = %error, "WebApp mandate draft create failed");
        (
            StatusCode::BAD_REQUEST,
            "Failed to create rule draft".to_string(),
        )
    })?;
    Ok(Json(webapp_mandate_response(
        &mandate,
        &state.settings.display_language(),
    )))
}

pub(super) async fn webapp_console_mandate_edit(
    State(state): State<Arc<AdminState>>,
    Path(mandate_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_webapp_mandate_id(&mandate_id)?;
    let (parts, body) = req.into_parts();
    let principal = extract_web_console_auth(&state, &parts.headers).await?;
    require_web_console_csrf_for_principal(&parts.headers, &principal)?;
    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let owner = format!("did:aegis:telegram:{chat_id}");
    let request = parse_webapp_mandate_request(body).await?;
    let conn = webapp_user_mandate_conn(&state).map_err(webapp_mandate_string_error)?;
    let store = crate::user_mandates::UserMandateStore::new(&conn);
    let existing =
        resolve_webapp_mandate_for_owner(&store, &owner, &mandate_id).map_err(|error| {
            tracing::warn!(
                error = %error,
                mandate = %opaque_unified_inbox_id("mandate_request", &mandate_id),
                "WebApp mandate edit lookup failed"
            );
            (
                StatusCode::BAD_REQUEST,
                "Failed to edit rule draft".to_string(),
            )
        })?;
    let Some(existing) = existing else {
        return Err((StatusCode::NOT_FOUND, "Rule not found".to_string()));
    };
    let new_mandate = build_new_user_mandate(
        chat_id,
        "webapp",
        Some(existing.mandate_id.clone()),
        request,
    )?;
    let mandate = store
        .update_draft(&owner, &existing.mandate_id, &new_mandate)
        .map_err(|error| {
            tracing::warn!(
                error = %error,
                mandate = %opaque_unified_inbox_id("mandate", &existing.mandate_id),
                "WebApp mandate edit failed"
            );
            (
                StatusCode::BAD_REQUEST,
                "Failed to edit rule draft".to_string(),
            )
        })?;
    Ok(Json(webapp_mandate_response(
        &mandate,
        &state.settings.display_language(),
    )))
}

pub(super) async fn webapp_console_mandate_activate(
    State(state): State<Arc<AdminState>>,
    Path(mandate_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_mandate_transition(
        state,
        req.headers(),
        mandate_id,
        crate::user_mandates::MandateStatus::Active,
        true,
    )
    .await
}

pub(super) async fn webapp_console_mandate_pause(
    State(state): State<Arc<AdminState>>,
    Path(mandate_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_mandate_transition(
        state,
        req.headers(),
        mandate_id,
        crate::user_mandates::MandateStatus::Paused,
        false,
    )
    .await
}

pub(super) async fn webapp_console_mandate_resume(
    State(state): State<Arc<AdminState>>,
    Path(mandate_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_mandate_transition(
        state,
        req.headers(),
        mandate_id,
        crate::user_mandates::MandateStatus::Active,
        true,
    )
    .await
}

pub(super) async fn webapp_console_mandate_revoke(
    State(state): State<Arc<AdminState>>,
    Path(mandate_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    webapp_console_mandate_transition(
        state,
        req.headers(),
        mandate_id,
        crate::user_mandates::MandateStatus::Revoked,
        false,
    )
    .await
}

async fn webapp_console_mandate_transition(
    state: Arc<AdminState>,
    headers: &axum::http::HeaderMap,
    mandate_id: String,
    status: crate::user_mandates::MandateStatus,
    require_mfa_for_sensitive: bool,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_webapp_mandate_id(&mandate_id).map_err(webapp_console_json_error)?;
    let principal = extract_web_console_auth(&state, headers)
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(headers, &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    let owner = format!("did:aegis:telegram:{chat_id}");
    let conn = webapp_user_mandate_conn(&state)?;
    let store = crate::user_mandates::UserMandateStore::new(&conn);
    let existing = resolve_webapp_mandate_for_owner(&store, &owner, &mandate_id)
        .map_err(|error| {
            tracing::warn!(
                error = %error,
                mandate = %opaque_unified_inbox_id("mandate_request", &mandate_id),
                "WebApp mandate lookup failed"
            );
            webapp_console_json_error((
                StatusCode::SERVICE_UNAVAILABLE,
                "Rules are unavailable".to_string(),
            ))
        })?
        .ok_or_else(|| {
            webapp_console_json_error((StatusCode::NOT_FOUND, "Rule not found".to_string()))
        })?;
    let real_mandate_id = existing.mandate_id.clone();
    if require_mfa_for_sensitive && webapp_mandate_requires_mfa(&existing) {
        require_web_console_recent_mfa(
            &principal,
            "Sensitive rule activation requires recent MFA",
        )?;
    }
    let mandate = store
        .transition(
            &owner,
            &real_mandate_id,
            status,
            &principal.log_subject(),
            webapp_mandate_transition_summary(status),
        )
        .map_err(|error| {
            tracing::warn!(
                error = %error,
                mandate = %opaque_unified_inbox_id("mandate", &real_mandate_id),
                "WebApp mandate transition failed"
            );
            webapp_console_json_error((
                StatusCode::BAD_REQUEST,
                "Invalid rule transition".to_string(),
            ))
        })?;
    Ok(Json(webapp_mandate_response(
        &mandate,
        &state.settings.display_language(),
    )))
}

fn resolve_webapp_mandate_for_owner(
    store: &crate::user_mandates::UserMandateStore<'_>,
    owner: &str,
    requested_id: &str,
) -> anyhow::Result<Option<crate::user_mandates::UserMandate>> {
    if let Some(mandate) = store.get_for_owner(owner, requested_id)? {
        return Ok(Some(mandate));
    }
    Ok(store
        .list_for_owner(owner, 500)?
        .into_iter()
        .find(|mandate| opaque_unified_inbox_id("mandate", &mandate.mandate_id) == requested_id))
}

async fn parse_webapp_mandate_request(
    body: axum::body::Body,
) -> Result<WebappMandateDraftRequest, (StatusCode, String)> {
    let body_bytes = axum::body::to_bytes(body, 24 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid rule request body".to_string(),
        )
    })?;
    let request: WebappMandateDraftRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid rule request JSON".to_string(),
        )
    })?;
    validate_context_schema_version(request.schema_version)?;
    validate_webapp_mandate_text("title", &request.title, 120)?;
    validate_webapp_mandate_text("description", &request.description, 1_000)?;
    validate_webapp_mandate_scope(&request.scope_kind)?;
    validate_webapp_mandate_string_list("allowed_reads", &request.allowed_reads, 16)?;
    validate_webapp_mandate_string_list("allowed_actions", &request.allowed_actions, 16)?;
    validate_webapp_mandate_string_list("must_ask", &request.must_ask, 24)?;
    if let Some(allowed_llm_uses) = request.allowed_llm_uses.as_ref() {
        validate_webapp_mandate_string_list("allowed_llm_uses", allowed_llm_uses, 12)?;
    }
    validate_webapp_mandate_policy("source_filters", request.source_filters.as_ref())?;
    validate_webapp_mandate_policy("notification_policy", request.notification_policy.as_ref())?;
    validate_webapp_mandate_policy("budget_policy", request.budget_policy.as_ref())?;
    validate_webapp_mandate_policy("frequency_policy", request.frequency_policy.as_ref())?;
    validate_optional_short_timestamp("expiry", request.expiry.as_deref())?;
    validate_optional_short_timestamp("review_after", request.review_after.as_deref())?;
    validate_optional_context_id("source_event_id", request.source_event_id.as_deref())?;
    validate_optional_context_id("trace_id", request.trace_id.as_deref())?;
    validate_optional_context_id("run_id", request.run_id.as_deref())?;
    reject_deferred_mandate_scopes(&request)?;
    Ok(request)
}

fn build_new_user_mandate(
    chat_id: i64,
    created_from: &str,
    mandate_id: Option<String>,
    request: WebappMandateDraftRequest,
) -> Result<crate::user_mandates::NewUserMandate, (StatusCode, String)> {
    let owner = format!("did:aegis:telegram:{chat_id}");
    let mandate_id = mandate_id.unwrap_or_else(|| {
        format!(
            "webapp-mandate:{}:{}",
            chrono::Utc::now().timestamp_millis(),
            opaque_unified_inbox_id("scope", &request.scope_kind)
        )
    });
    validate_webapp_mandate_id(&mandate_id)?;
    Ok(crate::user_mandates::NewUserMandate {
        mandate_id,
        title: request.title.trim().to_string(),
        description: request.description.trim().to_string(),
        owner_principal: owner,
        scope_kind: request.scope_kind,
        source_filters_json: request
            .source_filters
            .unwrap_or_else(|| serde_json::json!({}))
            .to_string(),
        allowed_reads_json: serde_json::to_string(&request.allowed_reads)
            .unwrap_or_else(|_| "[]".into()),
        allowed_llm_uses_json: serde_json::to_string(&request.allowed_llm_uses.unwrap_or_default())
            .unwrap_or_else(|_| "[]".into()),
        allowed_actions_json: serde_json::to_string(&request.allowed_actions)
            .unwrap_or_else(|_| "[]".into()),
        must_ask_json: serde_json::to_string(&request.must_ask).unwrap_or_else(|_| "[]".into()),
        notification_policy_json: request
            .notification_policy
            .unwrap_or_else(|| serde_json::json!({}))
            .to_string(),
        budget_policy_json: request
            .budget_policy
            .unwrap_or_else(|| serde_json::json!({}))
            .to_string(),
        frequency_policy_json: request
            .frequency_policy
            .unwrap_or_else(|| serde_json::json!({}))
            .to_string(),
        expiry: request.expiry,
        review_after: request.review_after,
        created_from: created_from.to_string(),
        source_event_id: request.source_event_id,
        trace_id: request.trace_id,
        run_id: request.run_id,
    })
}

fn validate_webapp_mandate_id(mandate_id: &str) -> Result<(), (StatusCode, String)> {
    let valid = !mandate_id.is_empty()
        && mandate_id.len() <= 180
        && mandate_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.'));
    if valid {
        Ok(())
    } else {
        Err((StatusCode::BAD_REQUEST, "Invalid rule id".to_string()))
    }
}

fn validate_webapp_mandate_text(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Rule {field} must not be empty"),
        ));
    }
    if trimmed.chars().count() > max_chars {
        return Err((StatusCode::BAD_REQUEST, format!("Rule {field} is too long")));
    }
    Ok(())
}

fn validate_webapp_mandate_scope(scope: &str) -> Result<(), (StatusCode, String)> {
    match scope {
        "email_metadata"
        | "calendar_metadata"
        | "reminders"
        | "system_alerts"
        | "api_costs"
        | "approvals"
        | "receipts"
        | "approvals_receipts"
        | "memory_review_queue" => Ok(()),
        _ => Err((StatusCode::BAD_REQUEST, "Invalid rule scope".to_string())),
    }
}

fn validate_webapp_mandate_string_list(
    field: &str,
    values: &[String],
    max_items: usize,
) -> Result<(), (StatusCode, String)> {
    if values.is_empty() || values.len() > max_items {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Rule {field} has invalid item count"),
        ));
    }
    for value in values {
        if value.is_empty()
            || value.len() > 96
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
        {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Rule {field} contains an invalid value"),
            ));
        }
    }
    Ok(())
}

fn validate_webapp_mandate_policy(
    field: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), (StatusCode, String)> {
    if let Some(value) = value {
        let serialized = value.to_string();
        if serialized.len() > 4_096 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Rule {field} is too large"),
            ));
        }
    }
    Ok(())
}

fn validate_optional_short_timestamp(
    field: &str,
    value: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if let Some(value) = value
        && (value.len() > 80 || value.chars().any(|ch| ch.is_control()))
    {
        return Err((StatusCode::BAD_REQUEST, format!("Invalid rule {field}")));
    }
    Ok(())
}

fn validate_optional_context_id(
    field: &str,
    value: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if let Some(value) = value
        && (value.len() > 160
            || value.is_empty()
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.')))
    {
        return Err((StatusCode::BAD_REQUEST, format!("Invalid rule {field}")));
    }
    Ok(())
}

fn reject_deferred_mandate_scopes(
    request: &WebappMandateDraftRequest,
) -> Result<(), (StatusCode, String)> {
    let deferred = [
        "raw_email_body",
        "email_body",
        "attachments",
        "remote_images",
        "legal_document_body",
        "financial_document_body",
        "medical_document_body",
        "payments",
        "purchases",
        "account_changes",
    ];
    let contains_deferred = request
        .allowed_reads
        .iter()
        .chain(request.allowed_actions.iter())
        .any(|item| deferred.iter().any(|blocked| blocked == item));
    if contains_deferred {
        Err((
            StatusCode::BAD_REQUEST,
            "Rule includes a sensitive scope that is not available yet".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn webapp_mandate_transition_summary(status: crate::user_mandates::MandateStatus) -> &'static str {
    match status {
        crate::user_mandates::MandateStatus::Draft => "Rule returned to draft",
        crate::user_mandates::MandateStatus::Active => "Rule activated",
        crate::user_mandates::MandateStatus::Paused => "Rule paused",
        crate::user_mandates::MandateStatus::Expired => "Rule expired",
        crate::user_mandates::MandateStatus::Revoked => "Rule revoked",
    }
}

fn webapp_mandate_requires_mfa(mandate: &crate::user_mandates::UserMandate) -> bool {
    matches!(
        mandate.scope_kind.as_str(),
        "email_metadata"
            | "calendar_metadata"
            | "api_costs"
            | "approvals"
            | "receipts"
            | "approvals_receipts"
            | "memory_review_queue"
    )
}

fn webapp_mandate_response(
    mandate: &crate::user_mandates::UserMandate,
    locale: &str,
) -> serde_json::Value {
    serde_json::json!({
        "status": "ok",
        "mandate": webapp_mandate_summary_json(mandate),
        "receipt_expectation": t("console.mandate.saved", locale),
    })
}

fn webapp_mandate_summary_json(mandate: &crate::user_mandates::UserMandate) -> serde_json::Value {
    serde_json::json!({
        "schema_version": mandate.schema_version,
        "mandate_id": opaque_unified_inbox_id("mandate", &mandate.mandate_id),
        "status": mandate.status.as_str(),
        "title": mandate.title,
        "description": mandate.description,
        "scope_kind": mandate.scope_kind,
        "allowed_reads": parse_string_list_json(&mandate.allowed_reads_json),
        "allowed_llm_uses": parse_string_list_json(&mandate.allowed_llm_uses_json),
        "allowed_actions": parse_string_list_json(&mandate.allowed_actions_json),
        "must_ask": parse_string_list_json(&mandate.must_ask_json),
        "notification_policy": parse_redacted_policy_summary(&mandate.notification_policy_json),
        "budget_policy": parse_redacted_policy_summary(&mandate.budget_policy_json),
        "frequency_policy": parse_redacted_policy_summary(&mandate.frequency_policy_json),
        "expiry": mandate.expiry,
        "review_after": mandate.review_after,
        "created_at": mandate.created_at,
        "updated_at": mandate.updated_at,
        "created_from": mandate.created_from,
        "source_event_id": mandate.source_event_id,
        "trace_id": mandate.trace_id,
        "run_id": mandate.run_id,
        "matching_enabled": mandate.status == crate::user_mandates::MandateStatus::Active,
        "background_autonomy_enabled": false,
    })
}

fn parse_string_list_json(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value).unwrap_or_default()
}

fn parse_redacted_policy_summary(value: &str) -> serde_json::Value {
    let parsed =
        serde_json::from_str::<serde_json::Value>(value).unwrap_or_else(|_| serde_json::json!({}));
    match parsed {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(key, _)| {
                    matches!(
                        key.as_str(),
                        "max_notifications_per_day"
                            | "quiet_hours"
                            | "batching_window"
                            | "min_severity"
                            | "max_llm_calls_per_day"
                            | "max_estimated_cost_per_day"
                            | "dedupe_window"
                            | "route"
                    )
                })
                .collect(),
        ),
        _ => serde_json::json!({}),
    }
}

fn webapp_user_mandate_conn(
    state: &AdminState,
) -> Result<rusqlite::Connection, (StatusCode, Json<serde_json::Value>)> {
    crate::encrypted_sqlite::open(state.memory.db_path()).map_err(|error| {
        tracing::warn!(error = %error, "Web Console mandate store open failed");
        webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Rules are unavailable".to_string(),
        ))
    })
}

fn webapp_mandate_string_error(
    error: (StatusCode, Json<serde_json::Value>),
) -> (StatusCode, String) {
    let (status, Json(value)) = error;
    let message = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Rules are unavailable")
        .to_string();
    (status, message)
}

pub(super) async fn webapp_console_approval_action(
    state: Arc<AdminState>,
    headers: &axum::http::HeaderMap,
    approval_id: String,
    approve: bool,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_webapp_approval_id(&approval_id).map_err(webapp_console_json_error)?;
    let principal = extract_web_console_auth(&state, headers)
        .await
        .map_err(webapp_console_json_error)?;
    require_web_console_csrf_for_principal(headers, &principal)
        .map_err(webapp_console_json_error)?;
    let chat_id =
        console_chat_id_for_principal(&state, &principal).map_err(webapp_console_json_error)?;
    check_webapp_rate_limit(
        &principal,
        "console_approval_action",
        &approval_id,
        WEBAPP_PANEL_ACTION_RATE_LIMIT_PER_MINUTE,
    )
    .map_err(webapp_console_json_error)?;

    let payload = state
        .memory
        .load_skill_state("hitl", &approval_id)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, approval_id, "Web Console approval lookup failed");
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load approval".to_string(),
            ))
        })?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            webapp_console_json_error((StatusCode::NOT_FOUND, "Approval not found".to_string()))
        })?;

    let approval_type = trust_console_approval_type(&approval_id, &payload);
    let risk = trust_console_approval_risk(&approval_type);
    if approve && trust_console_approval_requires_mfa(risk) {
        require_web_console_approval_mfa(&principal, risk)?;
    }

    let orch = state.orchestrator.as_ref().ok_or_else(|| {
        webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Approval routing is unavailable".to_string(),
        ))
    })?;
    let action_name = if approve {
        format!("admin_approve_hitl:{approval_id}")
    } else {
        format!("admin_reject_hitl:{approval_id}")
    };
    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=console-approval action={}]\n{}",
        if approve { "approve" } else { "reject" },
        serde_json::json!({ action_name: true })
    );
    let result = orch
        .handle_panel_submission(&msg, chat_id)
        .await
        .ok_or_else(|| {
            webapp_console_json_error((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Approval action failed".to_string(),
            ))
        })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "approval_id": approval_id,
        "action": if approve { "approve" } else { "reject" },
        "risk": risk,
        "approval_type": approval_type,
        "message": result.reply,
    })))
}

fn validate_webapp_approval_id(approval_id: &str) -> Result<(), (StatusCode, String)> {
    let valid = !approval_id.is_empty()
        && approval_id.len() <= 160
        && approval_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.'));
    if valid {
        Ok(())
    } else {
        Err((StatusCode::BAD_REQUEST, "Invalid approval id".to_string()))
    }
}

pub(super) fn trust_console_approval_requires_mfa(risk: &str) -> bool {
    risk.starts_with("R3") || risk.starts_with("R4")
}

fn require_web_console_approval_mfa(
    principal: &WebConsolePrincipal,
    risk: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    require_web_console_recent_mfa(principal, &format!("{risk} approval requires recent MFA"))
}

fn require_web_console_recent_mfa(
    principal: &WebConsolePrincipal,
    reason: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let policy = web_console_auth::MfaStepUpPolicy::recent_mfa(reason.to_string());
    let decision = match principal {
        WebConsolePrincipal::WebConsole(session) => {
            web_console_auth::require_recent_session_mfa(&policy, session, now)
        }
        WebConsolePrincipal::Telegram(_) => web_console_auth::require_recent_mfa(
            &policy,
            web_console_auth::MfaLevel::None,
            None,
            now,
        ),
        #[cfg(test)]
        WebConsolePrincipal::DebugLease(_) => web_console_auth::require_recent_mfa(
            &policy,
            web_console_auth::MfaLevel::None,
            None,
            now,
        ),
    };
    match decision {
        web_console_auth::MfaStepUpDecision::Satisfied => Ok(()),
        web_console_auth::MfaStepUpDecision::Required(problem) => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::to_value(problem).unwrap_or_else(|_| {
                serde_json::json!({
                    "error": "mfa_required",
                    "required_level": "totp_or_web_authn",
                    "reason": "Recent MFA required",
                })
            })),
        )),
    }
}

fn webapp_console_json_error(error: (StatusCode, String)) -> (StatusCode, Json<serde_json::Value>) {
    let (status, message) = error;
    (
        status,
        Json(serde_json::json!({
            "status": "error",
            "error": message,
        })),
    )
}

pub(super) fn webapp_console_action_ledger_conn(
    state: &AdminState,
) -> Result<rusqlite::Connection, (StatusCode, Json<serde_json::Value>)> {
    crate::encrypted_sqlite::open(state.memory.db_path()).map_err(|error| {
        tracing::warn!(error = %error, "Web Console action ledger open failed");
        webapp_console_json_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Action Ledger is unavailable".to_string(),
        ))
    })
}

pub(super) fn webapp_console_transition_action_ledger(
    state: &AdminState,
    action_id: &str,
    status: crate::action_ledger::ActionStatus,
    update: crate::action_ledger::ActionUpdate,
) -> anyhow::Result<crate::action_ledger::ActionLedgerEntry> {
    let conn = crate::encrypted_sqlite::open(state.memory.db_path())?;
    crate::action_ledger::ActionLedgerStore::new(&conn).transition(action_id, status, &update)
}
