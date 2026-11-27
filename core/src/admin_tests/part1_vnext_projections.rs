//! vNext home/work/status projections: lanes, decision reason codes, cost-saving status, follow-up staleness, zh owner copy.
use super::*;

#[test]
fn completed_recurring_task_projects_without_stale_actions() {
    let mut task = test_task_for_projection();
    task.status = "completed".into();
    task.interval_seconds = Some(3600);
    task.max_fires = Some(3);
    task.fire_count = 3;
    for locale in ["en", "zh"] {
        let item = unified_inbox_item_from_task(&task, locale);
        assert!(
            item.secondary_actions.is_empty(),
            "{locale}: completed reminder exposes actions"
        );
        assert_eq!(item.urgency, "silent");
        assert_eq!(item.status, "completed");
        assert_eq!(vnext_lane_for_item(&item), "done");
        assert!(!vnext_work_action_is_projected_for_item(
            &item,
            "task.complete"
        ));
        assert!(!vnext_work_action_is_projected_for_item(
            &item,
            "task.snooze_tomorrow"
        ));
    }
}

#[test]
fn work_projection_still_projects_calendar_events() {
    let event = crate::assistant_events::StoredAssistantEvent {
        id: 77,
        event: crate::assistant_events::AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-05-21T00:00:00Z".into(),
            occurred_at: "2026-05-21T00:00:00Z".into(),
            user_did: "did:aegis:telegram:12345678".into(),
            source: crate::assistant_events::AssistantEventSource::Calendar,
            kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
            account_id: Some("primary".into()),
            title: "Team meeting updated".into(),
            summary: "Meeting moved".into(),
            thread_key: None,
            urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
            actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
            decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
            dedup_key: "calendar:test:still-projects".into(),
            entities: serde_json::json!({}),
        },
    };
    let item = unified_inbox_active_item_from_assistant_event(&event, "en")
        .expect("calendar events still project into Work (Work is their only home)");
    assert_eq!(item.kind, "calendar");
    assert_eq!(item.id, "calendar:77");
}
// ── i18n structural codes (owner_visible_reason_code / next_safe_action_code /
//    label_code): stable identifiers the webapp resolves against its catalog;
//    the English prose stays on the wire as the fallback + Telegram surface. ──

#[test]
fn unified_inbox_calendar_item_emits_structural_codes() {
    let event = crate::assistant_events::StoredAssistantEvent {
        id: 77,
        event: crate::assistant_events::AssistantEvent {
            raw_event_id: None,
            observed_at: "2026-05-21T00:00:00Z".into(),
            occurred_at: "2026-05-21T00:00:00Z".into(),
            user_did: "did:aegis:telegram:12345678".into(),
            source: crate::assistant_events::AssistantEventSource::Calendar,
            kind: crate::assistant_events::AssistantEventKind::CalendarEventChanged,
            account_id: Some("primary".into()),
            title: "Team meeting updated".into(),
            summary: "Meeting moved".into(),
            thread_key: None,
            urgency_hint: crate::assistant_events::EventUrgencyHint::Medium,
            actionability_hint: crate::assistant_events::EventActionabilityHint::Review,
            decision: crate::assistant_events::AssistantEventDecision::NotifyNow,
            dedup_key: "calendar:test:codes".into(),
            entities: serde_json::json!({}),
        },
    };

    let item = unified_inbox_item_from_calendar_event(&event, "en");
    assert_eq!(
        item.owner_visible_reason_code.as_deref(),
        Some("calendar_basics_changed")
    );
    assert_eq!(
        item.next_safe_action_code.as_deref(),
        Some("calendar_basics_review")
    );
    assert_eq!(
        item.primary_action
            .as_ref()
            .and_then(|action| action.label_code.as_deref()),
        Some("review_calendar_basics")
    );
}

#[test]
fn unified_inbox_approval_item_emits_structural_codes() {
    // Generic approval payload ("{}"): the fallback OwnerApprovalDisplay branch.
    let item =
        unified_inbox_item_from_approval("config-approval-123", "{}", "2026-05-10T00:00:00Z", "en");
    assert_eq!(
        item.owner_visible_reason_code.as_deref(),
        Some("approval_generic")
    );
    let next_code = item
        .next_safe_action_code
        .as_deref()
        .expect("approval emits a next_safe_action code");
    assert!(
        next_code == "approval_review_decide" || next_code == "approval_high_risk_review",
        "generic approval next code must be one of the two static branches, got {next_code}"
    );
    assert_eq!(
        item.primary_action
            .as_ref()
            .and_then(|action| action.label_code.as_deref()),
        Some("review_approval")
    );
    assert_eq!(
        item.secondary_actions[0].label_code.as_deref(),
        Some("approve")
    );
    assert_eq!(
        item.secondary_actions[1].label_code.as_deref(),
        Some("reject")
    );
}

#[test]
fn unified_inbox_items_without_codes_omit_the_code_keys() {
    // Codes are OPTIONAL: an item built without them must not serialize the
    // keys at all (skip_serializing_if), so older FE guards stay untouched.
    let item = test_unified_item_for_vnext("task-plain", "task", "waiting", "normal");
    assert!(item.owner_visible_reason_code.is_none());

    let work = vnext_work_item(&item, "en");
    let json = serde_json::to_value(&work).unwrap();
    assert!(json.get("why_it_matters_code").is_none());
    assert!(json.get("next_safe_action_code").is_none());
    assert!(
        json["recommended_action"].get("label_code").is_none(),
        "absent label_code must be omitted from the serialized action"
    );
}
#[test]
fn vnext_status_projection_surfaces_settings_audit_as_attention() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings
        .set("fallback_model_authorized", serde_json::json!(true))
        .unwrap();
    settings
        .set("fallback_model", serde_json::json!(""))
        .unwrap();
    settings
        .set("external_executor_enabled", serde_json::json!(true))
        .unwrap();

    let status = vnext_status_projection_with_settings(&projection, Some(&settings), "zh");
    let health = status
        .sections
        .iter()
        .find(|section| section.label == "System Health")
        .expect("System Health section");
    assert_eq!(health.health, "attention");
    assert_eq!(
        health.owner_message,
        "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。"
    );
    assert!(
        status.sections.iter().any(|section| {
            section.health == "attention"
                && section.label == "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。"
        }),
        "finding should appear as an attention row with title = message"
    );
}

#[test]
fn webapp_runtime_rejects_unfinished_preamble_as_answer() {
    assert!(runtime_reply_is_unfinished_preamble(
        "我会按 OpenAI 官方文档来回答，避免用过期印象。",
        &["recall_memory".to_string()]
    ));
    assert!(runtime_reply_is_unfinished_preamble(
        "I'll check the official documentation and answer from it.",
        &[]
    ));
    assert!(!runtime_reply_is_unfinished_preamble(
        "OpenAI Codex app-server can broker foreground reasoning through an official logged-in app server while Aegis keeps ToolGate authoritative.",
        &["recall_memory".to_string()]
    ));
    assert!(!runtime_reply_is_unfinished_preamble(
        "我会提醒你处理这件事。",
        &["manage_tasks".to_string()]
    ));
}

#[test]
fn vnext_work_action_request_accepts_approval_gate_actions() {
    for action_id in [
        "approval.approve",
        "approval.reject",
        "memory.confirm",
        "memory.pin",
        "memory.unpin",
        "memory.mark_wrong",
        "memory.delete",
        "task.skip_occurrence",
        "tracking.cancel",
    ] {
        validate_vnext_work_action_request(&VNextWorkActionRequest {
            schema_version: Some(1),
            action_id: action_id.to_string(),
            idempotency_key: Some(format!("vnext:test:{action_id}")),
        })
        .expect("projected owner actions should be valid vNext item actions");
    }
}

#[test]
fn vnext_work_action_must_be_projected_by_item() {
    let mut approval = test_unified_item_for_vnext("approval:1", "approval", "waiting", "high");
    approval.source = "hitl".into();
    approval.secondary_actions = vec![UnifiedInboxAction {
        label: "Approve".into(),
        label_code: None,
        action_kind: "approval.approve".into(),
        route: None,
        requires_gate: false,
        requires_mfa: false,
    }];

    assert!(vnext_work_action_is_projected_for_item(
        &approval,
        "approval.approve"
    ));
    assert!(
        !vnext_work_action_is_projected_for_item(&approval, "approval.reject"),
        "approval actions are independent owner choices and must be projected individually"
    );

    let mut memory =
        test_unified_item_for_vnext("memory_review:1", "memory_review", "stale_review", "normal");
    memory.source = "memory".into();
    memory.secondary_actions = vec![UnifiedInboxAction {
        label: "Keep using".into(),
        label_code: None,
        action_kind: "memory.confirm".into(),
        route: Some("/webapp/console/memory/items/1/confirm".into()),
        requires_gate: false,
        requires_mfa: false,
    }];

    assert!(vnext_work_action_is_projected_for_item(
        &memory,
        "memory.confirm"
    ));
    assert!(
        !vnext_work_action_is_projected_for_item(&memory, "memory.delete"),
        "memory actions must also be projected before execution"
    );
}

#[test]
fn memory_review_projection_exposes_real_review_actions() {
    let item = unified_inbox_item_from_memory_review(
        &crate::memory::StaleFact {
            id: 42,
            topic: "learned:preference".into(),
            fact_summary: "owner prefers real action buttons".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
            last_accessed: "2026-03-01T00:00:00Z".into(),
            days_stale: 79,
            is_pinned: false,
        },
        "en",
    );

    assert_eq!(
        item.secondary_actions
            .iter()
            .map(|action| vnext_action_kind_for_endpoint(&action.action_kind))
            .collect::<Vec<_>>(),
        vec![
            "memory.confirm",
            "memory.pin",
            "memory.mark_wrong",
            "memory.delete",
        ]
    );
    assert!(
        item.next_safe_action.contains("Keep using"),
        "owner-facing promise must match the projected actions"
    );
    assert!(
        item.secondary_actions
            .iter()
            .all(|action| !action.requires_mfa),
        "owner-local memory review actions should not send the signed-in owner to Settings/MFA"
    );
}

#[test]
fn vnext_work_action_response_does_not_fabricate_approval_receipt() {
    let mut item = test_unified_item_for_vnext("approval:abc", "approval", "waiting", "normal");
    item.source = "hitl".into();
    let response = vnext_work_action_response(
        &item,
        "approval.reject",
        serde_json::json!({
            "status": "ok",
            "approval_id": "config-test",
            "action": "reject",
            "risk": "low",
            "approval_type": "config",
            "message": "Approval rejected."
        }),
        "en",
    );

    assert_eq!(response["owner_message"], "Approval rejected.");
    assert!(
        response["receipt"].is_null(),
        "approval wrapper must not invent receipt evidence when the HITL result did not return one"
    );
}

#[tokio::test]
async fn vnext_approval_action_resolves_opaque_work_item_to_hitl_id() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    memory
        .store_skill_state(
            "hitl",
            "config-approval-123",
            &serde_json::json!({
                "kind": "config",
                "chat_id": 12345678,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "key": "email_notifications",
                "value": false
            })
            .to_string(),
        )
        .await
        .unwrap();
    let state = webapp_test_state_with_memory_and_db(memory, None);
    let item =
        unified_inbox_item_from_approval("config-approval-123", "{}", "2026-05-10T00:00:00Z", "en");
    assert_eq!(
        item.secondary_actions
            .iter()
            .map(|action| action.action_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["approval.approve", "approval.reject"]
    );

    let resolved = resolve_vnext_approval_id_for_item(&state, &item)
        .await
        .expect("resolve approval");
    assert_eq!(resolved, "config-approval-123");
}
#[test]
fn vnext_mcp_approval_has_owner_meaningful_detail() {
    let item = unified_inbox_item_from_approval(
        "mcp-filesystem",
        &serde_json::json!({
            "server_key": "filesystem",
            "tool_requested": "read_file",
            "server_command": "npx -y @modelcontextprotocol/server-filesystem /tmp",
            "chat_id": 12345678,
            "created_at": "2026-05-21T00:00:00Z"
        })
        .to_string(),
        "2026-05-21T00:00:00Z",
        "en",
    );

    assert_eq!(item.title, "Approve tool connection?");
    assert!(
        item.summary.contains("filesystem") && item.summary.contains("read_file"),
        "MCP approval summary should name server and requested tool: {}",
        item.summary
    );
    assert_eq!(item.urgency, "high");
}

#[test]
fn vnext_work_projection_groups_owner_lanes_and_authority() {
    let ready = test_unified_item_for_vnext("ready", "calendar", "unread", "normal");
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "available".into(),
            reason: Some("assistant event metadata only".into()),
        }],
        counts: UnifiedInboxCounts::default(),
        items: vec![
            ready,
            test_unified_item_for_vnext("decision", "calendar", "needs_decision", "high"),
            test_unified_item_for_vnext("later", "task", "backlog", "low"),
            test_unified_item_for_vnext("closed", "task", "done", "normal"),
        ],
    };

    let work = vnext_work_projection(&projection, "en");
    assert_eq!(work.schema_version, 1);
    assert_eq!(
        work.lanes
            .iter()
            .map(|lane| lane.id.as_str())
            .collect::<Vec<_>>(),
        vec!["now", "decide", "follow_up", "backlog", "done"]
    );
    assert_eq!(work.lanes[0].items[0].id, "ready");
    assert_eq!(work.lanes[0].items[0].source_kind, "calendar");
    assert_eq!(work.lanes[0].items[0].source, "calendar");
    assert_eq!(work.lanes[0].items[0].source_id, "ready");
    assert_eq!(
        work.lanes[0].items[0].detail_route.as_deref(),
        Some("/webapp/console/test")
    );
    assert_eq!(
        work.lanes[0].items[0]
            .recommended_action
            .as_ref()
            .and_then(|action| action.route.as_deref()),
        Some("/webapp/console/test")
    );
    assert!(work.lanes[0].items[0].contextual_actions.is_empty());
    assert_eq!(
        work.lanes[0].items[0].related_work_item_ids,
        vec!["action:ready".to_string()]
    );
    assert_eq!(
        work.lanes[1].items[0].authority.basis,
        "direct_owner_request"
    );
    assert!(work.lanes[3].collapsed);
    assert!(work.lanes[4].collapsed);
}

#[test]
fn vnext_work_projection_keeps_workbench_bounded() {
    let mut items = Vec::new();
    for index in 0..20 {
        items.push(test_unified_item_for_vnext(
            &format!("decision-{index}"),
            "email",
            "needs_decision",
            "high",
        ));
        items.push(test_unified_item_for_vnext(
            &format!("ready-{index}"),
            "email",
            "unread",
            "normal",
        ));
        items.push(test_unified_item_for_vnext(
            &format!("follow-{index}"),
            "waiting_for",
            "waiting",
            "normal",
        ));
        items.push(test_unified_item_for_vnext(
            &format!("backlog-{index}"),
            "task",
            "backlog",
            "low",
        ));
        items.push(test_unified_item_for_vnext(
            &format!("done-{index}"),
            "task",
            "done",
            "normal",
        ));
    }
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items,
    };

    let work = vnext_work_projection(&projection, "en");
    assert_eq!(
        work.lanes
            .iter()
            .find(|lane| lane.id == "decide")
            .unwrap()
            .items
            .len(),
        12
    );
    assert_eq!(
        work.lanes
            .iter()
            .find(|lane| lane.id == "now")
            .unwrap()
            .items
            .len(),
        12
    );
    assert_eq!(
        work.lanes
            .iter()
            .find(|lane| lane.id == "follow_up")
            .unwrap()
            .items
            .len(),
        12
    );
    assert_eq!(
        work.lanes
            .iter()
            .find(|lane| lane.id == "backlog")
            .unwrap()
            .items
            .len(),
        6
    );
    assert_eq!(
        work.lanes
            .iter()
            .find(|lane| lane.id == "done")
            .unwrap()
            .items
            .len(),
        0
    );
}

#[test]
fn vnext_home_prefers_decisions_before_ready_work() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![
            test_unified_item_for_vnext("ready", "email", "unread", "normal"),
            test_unified_item_for_vnext("decision", "email", "needs_decision", "high"),
        ],
    };

    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Needs your decision: Item decision.");
    assert_eq!(home.decision_previews[0].id, "decision");
    assert_eq!(home.top_now[0].id, "ready");
}

#[test]
fn vnext_lane_does_not_promote_gated_detail_navigation_to_decision() {
    let mut item = test_unified_item_for_vnext("waiting", "task", "waiting", "normal");
    item.primary_action = Some(UnifiedInboxAction {
        label: "Open details".into(),
        label_code: None,
        action_kind: "navigate".into(),
        route: Some("/webapp/console/tasks/1/timeline".into()),
        requires_gate: true,
        requires_mfa: false,
    });

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "follow_up");
    assert_eq!(
        vnext_home_brief_for_item(&projected, "en"),
        "Following up: Item waiting."
    );
}

#[test]
fn vnext_action_ledger_projection_does_not_duplicate_pending_decisions() {
    let proposed_reminder = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::Proposed,
        crate::action_ledger::ActionKind::ReminderCreate,
    );
    assert!(
        !vnext_should_project_action_ledger_entry(&proposed_reminder),
        "ordinary reminder proposals must not become fake WebApp decision cards"
    );

    let pending_email = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::PendingApproval,
        crate::action_ledger::ActionKind::EmailSend,
    );
    assert!(
        !vnext_should_project_action_ledger_entry(&pending_email),
        "pending approvals are projected from canonical HITL, not raw ledger rows"
    );

    let executed_email = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionKind::EmailUpdate,
    );
    assert!(vnext_should_project_action_ledger_entry(&executed_email));

    let failed_email = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::Failed,
        crate::action_ledger::ActionKind::EmailSend,
    );
    assert!(vnext_should_project_action_ledger_entry(&failed_email));
}

#[test]
fn retired_webapp_action_drafts_are_not_projected_as_main_work() {
    let entry = crate::action_ledger::ActionLedgerEntry {
        id: 77,
        action_id: "draft:77".into(),
        action_kind: crate::action_ledger::ActionKind::ReminderCreate,
        status: crate::action_ledger::ActionStatus::Proposed,
        requested_by: Some("owner".into()),
        chat_id: Some(12345678),
        created_at: "2026-05-10T00:00:00Z".into(),
        proposed_at: Some("2026-05-10T00:00:00Z".into()),
        approved_at: None,
        executing_at: None,
        executed_at: None,
        failed_at: None,
        cancelled_at: None,
        input_summary: Some("reminder draft".into()),
        user_visible_summary: Some("Contextual action draft saved for review".into()),
        external_receipt: None,
        external_message_id: None,
        external_event_id: None,
        tool_call_id: None,
        assistant_event_id: None,
        provider_request_id: None,
        idempotency_key: Some("draft:77".into()),
        error_kind: None,
        error_message: None,
        retry_policy: None,
        trace_id: None,
        run_id: None,
        mandate_id: None,
        source_event_id: None,
        metadata_json: Some(
            serde_json::json!({
                "schema_version": 1,
                "resource_type": "contextual_action_draft",
                "focus_id": "assistant:general",
                "focus_kind": "assistant",
                "draft_text": "明天上午9点提醒我给律师回邮件",
                "suggested_intent_kind": "reminder"
            })
            .to_string(),
        ),
    };

    assert!(retired_webapp_action_draft_entry(&entry));
    let item = unified_inbox_item_from_ledger_entry(&entry, "en");
    assert_eq!(item.kind, "action_draft");
    assert_eq!(item.status, "needs_decision");
    assert_eq!(item.title, "Review reminder: 明天上午9点提醒我给律师回邮件");
    assert_eq!(
        item.summary,
        "Review the proposed reminder before Aegis saves it. Nothing has been saved or sent yet."
    );
    assert!(!item.title.contains("reminder create"));
    // blocked_by and its structural code come from ONE tuple; pin the pairing so
    // a future edit cannot leave the code describing a different sentence.
    assert_eq!(
        item.blocked_by.as_deref(),
        Some(
            "Execution is intentionally disabled for this draft until an approval route and receipt path are connected."
        )
    );
    assert_eq!(
        item.blocked_by_code.as_deref(),
        Some("blocked_by_draft_execution_disabled")
    );
    assert_eq!(vnext_work_item(&item, "en").source_kind, "receipt");
    assert!(item.secondary_actions.is_empty());
}

#[test]
fn action_draft_title_skips_assistant_boilerplate() {
    let title = action_draft_task_title(
        "Aegis interpreted this as Reminder; review before any durable work item or external action is created.\n\n**明天 (2026-05-13 周三) 概览** (当前 2026-05-12 00:45 PDT)：\n\n### 日历\n暂无事件。",
        &["Reminder:", "提醒:"],
    );
    assert_eq!(
        title,
        "明天 (2026-05-13 周三) 概览 (当前 2026-05-12 00:45 PDT)："
    );
}

#[test]
fn action_draft_summary_skips_assistant_boilerplate() {
    let entry = crate::action_ledger::ActionLedgerEntry {
        id: 647,
        action_id: "draft:647".into(),
        action_kind: crate::action_ledger::ActionKind::ReminderCreate,
        status: crate::action_ledger::ActionStatus::Proposed,
        requested_by: Some("owner".into()),
        chat_id: Some(12345678),
        created_at: "2026-05-12T07:47:40Z".into(),
        proposed_at: Some("2026-05-12T07:47:40Z".into()),
        approved_at: None,
        executing_at: None,
        executed_at: None,
        failed_at: None,
        cancelled_at: None,
        input_summary: Some("reminder draft".into()),
        user_visible_summary: Some("Contextual action draft saved for review".into()),
        external_receipt: None,
        external_message_id: None,
        external_event_id: None,
        tool_call_id: None,
        assistant_event_id: None,
        provider_request_id: None,
        idempotency_key: None,
        error_kind: None,
        error_message: None,
        retry_policy: None,
        trace_id: None,
        run_id: None,
        mandate_id: None,
        source_event_id: None,
        metadata_json: None,
    };
    let summary = user_facing_action_draft_summary(
        &entry,
        Some(
            "Aegis interpreted this as Reminder; review before any durable work item or external action is created.\n\n**明天 (2026-05-13 周三) 概览** (当前 2026-05-12 00:45 PDT)：\n\n### 日历\n暂无事件。",
        ),
        "en",
    );

    assert_eq!(
        summary,
        "Review the proposed reminder before Aegis saves it. Nothing has been saved or sent yet."
    );
    assert!(!summary.contains("Aegis interpreted this as"));
    assert!(!summary.contains("**"));
}

#[test]
fn blocked_by_gate_owner_work_enters_decision_attention() {
    let mut item = test_unified_item_for_vnext("gate", "task", "blocked_by_gate", "normal");
    item.title = "使用 web_search 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文".into();

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "decide");
    assert!(projected.contextual_actions.is_empty());

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(
        home.brief,
        "Needs attention: 使用 web_search 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文."
    );
    assert_eq!(home.decision_previews[0].id, "gate");
    assert!(home.top_now.is_empty());
    assert!(home.follow_up_highlights.is_empty());
}

#[test]
fn blocked_standing_instruction_enters_today_attention() {
    let mut task = test_task_for_projection();
    task.title = "Check server disk usage every morning".into();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("server-disk-check".into());

    let item = unified_inbox_item_from_task(&task, "en");
    assert_eq!(item.status, "blocked_by_gate");
    assert!(item.blocked_by.is_some());
    assert_eq!(item.title, "Automation paused");
    assert_eq!(
        item.summary,
        "Automation paused before continuing: Check server disk usage every morning"
    );
    assert_eq!(
        item.owner_visible_reason,
        "A sensitive step needs owner review before Aegis continues."
    );
    assert_eq!(
        item.blocked_by.as_deref(),
        Some("Aegis paused this automation before a sensitive step.")
    );

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "decide");
    assert_eq!(
        projected
            .contextual_actions
            .iter()
            .map(|action| action.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["task.complete", "task.snooze_tomorrow"]
    );
    let decision = projected
        .decision_request
        .as_ref()
        .expect("blocked automation should project a decision request");
    assert_eq!(decision.title, "Automation needs your choice");
    assert_eq!(decision.title_code.as_deref(), Some("title_generic"));
    assert_eq!(
        decision.what_aegis_was_doing,
        "Automation paused before continuing: Check server disk usage every morning"
    );
    assert!(decision.what_code.is_none());
    assert_eq!(
        decision.requested_capability,
        "Continue past the protected boundary for this automation."
    );
    assert_eq!(
        decision.capability_code.as_deref(),
        Some("cap_generic_boundary")
    );
    assert_eq!(decision.scope_code.as_deref(), Some("scope_blocked_run"));
    assert!(
        decision
            .safe_details
            .iter()
            .any(|detail| detail.contains("No live approval request is attached"))
    );
    let safe_codes = decision
        .safe_detail_codes
        .as_ref()
        .expect("blocked path emits safe_detail_codes");
    assert_eq!(safe_codes.len(), decision.safe_details.len());
    assert_eq!(
        safe_codes.as_slice(),
        [
            "safe_no_external_action",
            "safe_visible_metadata",
            "safe_no_live_approval"
        ]
    );
    assert_eq!(
        decision
            .choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "approve_unavailable",
            "skip_unavailable",
            "revise_rule",
            "view_safe_details",
            "review_tomorrow",
            "close_pause"
        ]
    );
    let approve_unavailable = decision
        .choices
        .iter()
        .find(|choice| choice.id == "approve_unavailable")
        .expect("missing-approval state should be explicit");
    assert!(!approve_unavailable.enabled);
    assert_eq!(
        approve_unavailable.disabled_reason.as_deref(),
        Some("No live approval request is attached to this pause.")
    );
    assert_eq!(
        decision.choices[2].assistant_prompt.as_deref(),
        Some(
            "Help me revise this paused automation rule. First explain what it is trying to do, then propose safer narrower options. Do not execute any external action."
        )
    );
    assert_eq!(
        decision.choices[4].action_kind.as_deref(),
        Some("task.snooze_tomorrow")
    );

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Needs attention: Automation paused.");
    assert_eq!(home.decision_previews[0].id, "task:42");
    assert!(home.top_now.is_empty());
    assert!(home.follow_up_highlights.is_empty());
}

#[test]
fn high_priority_waiting_items_stay_tracking_not_ready_work() {
    let mut item = test_unified_item_for_vnext("blocked", "waiting_for", "waiting", "high");
    item.title = "日终状态：磁盘77%+calendar电路+Nationwide 5/9".into();
    item.summary = "Waiting-for item from the knowledge base.".into();

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "follow_up");
    assert_eq!(
        projected.title,
        "日终状态：磁盘77% / calendar电路 / Nationwide 5/9"
    );

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Nothing urgent is asking for you right now.");
    assert!(home.top_now.is_empty());
    assert_eq!(home.follow_up_highlights[0].id, "blocked");
}

#[test]
fn hitl_approval_source_task_id_parses_gate_task_link() {
    assert_eq!(
        hitl_approval_source_task_id(r#"{"source_task_id":5,"tool_name":"calendar_reader"}"#),
        Some(5)
    );
    assert_eq!(
        hitl_approval_source_task_id(r#"{"tool_name":"calendar_reader"}"#),
        None
    );
    assert_eq!(hitl_approval_source_task_id("not-json"), None);
}

#[test]
fn owner_visible_title_cleans_machine_compound_titles() {
    assert_eq!(
        owner_visible_title("日终状态：磁盘77%+calendar电路+Nationwide 5/9", "Task", 180),
        "日终状态：磁盘77% / calendar电路 / Nationwide 5/9"
    );
    assert_eq!(
        owner_visible_title("使用 web_search 搜索过去6小时 Elon Musk+", "Task", 180),
        "使用 web_search 搜索过去6小时 Elon Musk"
    );
    assert_eq!(owner_visible_title("C++ notes", "Task", 180), "C++ notes");
}

#[test]
fn blocked_web_search_keeps_owner_visible_request_context() {
    let mut task = test_task_for_projection();
    task.title =
        "使用 web_search 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话+".into();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("scheduled-web-search".into());

    let item = unified_inbox_item_from_task(&task, "en");
    assert_eq!(item.title, "Web search paused");
    assert_eq!(
        item.summary,
        "Scheduled search paused: 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话"
    );
    assert_eq!(
        item.blocked_by.as_deref(),
        Some("Running the external search step needs owner permission.")
    );

    let projected = vnext_work_item(&item, "en");
    let decision = projected
        .decision_request
        .as_ref()
        .expect("blocked web search should project a decision request");
    assert_eq!(decision.title, "Web search needs your choice");
    assert_eq!(decision.title_code.as_deref(), Some("title_web_search"));
    assert_eq!(
        decision.what_aegis_was_doing,
        "Scheduled search paused: 搜索过去6小时 Elon Musk (@elonmusk) 在 X 上的最新推文（英文原话"
    );
    // Raw summary passthrough: no template code for freeform what-text.
    assert_eq!(decision.what_code, None);
    assert_eq!(
        decision.requested_capability,
        "Use the web search tool for this scheduled request."
    );
    assert_eq!(
        decision.capability_code.as_deref(),
        Some("cap_web_search_tool")
    );
}

#[test]
fn blocked_decision_request_emits_template_codes_with_and_without_live_approval() {
    // Missing live approval → conditional safe_no_live_approval bullet.
    let mut no_approval =
        test_unified_item_for_vnext("task:disk-pause", "task", "blocked_by_gate", "high");
    no_approval.title = "Automation paused".into();
    no_approval.summary =
        "Automation paused before continuing: Check server disk usage every morning".into();
    no_approval.owner_visible_reason =
        "A sensitive step needs owner review before Aegis continues.".into();
    no_approval.blocked_by = Some("Aegis paused this automation before a sensitive step.".into());
    no_approval.detail_route = Some("/webapp/console/tasks/42/timeline".into());
    no_approval.secondary_actions = Vec::new();

    let decision = vnext_decision_request(&no_approval, "en").expect("blocked decision");
    assert_eq!(decision.title_code.as_deref(), Some("title_generic"));
    assert!(decision.what_code.is_none());
    assert_eq!(
        decision.capability_code.as_deref(),
        Some("cap_generic_boundary")
    );
    assert_eq!(decision.scope_code.as_deref(), Some("scope_blocked_run"));
    let codes = decision
        .safe_detail_codes
        .as_ref()
        .expect("safe_detail_codes");
    assert_eq!(codes.len(), decision.safe_details.len());
    assert_eq!(
        codes.as_slice(),
        [
            "safe_no_external_action",
            "safe_visible_metadata",
            "safe_no_live_approval"
        ]
    );

    // Live approval present → only the two constant safe bullets.
    let mut with_approval = no_approval.clone();
    with_approval.secondary_actions = vec![
        UnifiedInboxAction {
            label: "Approve".into(),
            label_code: None,
            action_kind: "approval.approve".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: "Reject".into(),
            label_code: None,
            action_kind: "approval.reject".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
    ];
    let decision =
        vnext_decision_request(&with_approval, "en").expect("blocked decision with approval");
    let codes = decision
        .safe_detail_codes
        .as_ref()
        .expect("safe_detail_codes");
    assert_eq!(codes.len(), decision.safe_details.len());
    assert_eq!(
        codes.as_slice(),
        ["safe_no_external_action", "safe_visible_metadata"]
    );
    assert!(
        !decision
            .safe_details
            .iter()
            .any(|d| d.contains("No live approval request"))
    );
}

#[test]
fn blocked_decision_request_what_code_none_for_generic_summary_passthrough() {
    let mut item = test_unified_item_for_vnext("task:generic", "task", "blocked_by_gate", "normal");
    item.title = "Custom automation paused".into();
    item.summary = "Owner-specific freeform summary about the custom automation.".into();
    item.blocked_by = Some("Aegis paused this automation before a sensitive step.".into());

    let decision = vnext_decision_request(&item, "en").expect("blocked decision");
    assert_eq!(decision.title_code.as_deref(), Some("title_generic"));
    assert_eq!(
        decision.what_aegis_was_doing,
        "Owner-specific freeform summary about the custom automation."
    );
    assert_eq!(decision.what_code, None);
    assert_eq!(
        decision.capability_code.as_deref(),
        Some("cap_generic_boundary")
    );
}

#[test]
fn approval_decision_request_emits_scope_and_leading_safe_code_only() {
    let mut item = test_unified_item_for_vnext("approval:7", "approval", "needs_decision", "high");
    item.title = "Approve marking mail read?".into();
    item.summary = "mark 1 selected Calendar event as read".into();
    item.next_safe_action = "Approve or reject this request.".into();
    item.blocked_by = Some("Waiting for your approve or reject decision.".into());
    item.secondary_actions = vec![
        UnifiedInboxAction {
            label: "Approve".into(),
            label_code: None,
            action_kind: "approval.approve".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: "Reject".into(),
            label_code: None,
            action_kind: "approval.reject".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
    ];

    let decision = vnext_decision_request(&item, "en").expect("approval decision");
    assert_eq!(decision.title_code, None);
    assert_eq!(decision.what_code, None);
    assert_eq!(decision.capability_code, None);
    assert_eq!(
        decision.scope_code.as_deref(),
        Some("scope_approval_request")
    );
    assert_eq!(decision.scope, "This approval request only.");
    let codes = decision
        .safe_detail_codes
        .as_ref()
        .expect("approval safe_detail_codes");
    assert_eq!(codes.len(), decision.safe_details.len());
    assert_eq!(codes[0], "safe_no_outside_action");
    assert!(codes[1..].iter().all(|c| c.is_empty()));
    assert_eq!(
        decision.safe_details[0],
        "No outside action runs until you choose."
    );
}

#[test]
fn vnext_work_item_wire_carries_blocked_by_and_decision_reason_codes() {
    // End-to-end on the path the WebApp actually calls (/webapp/vnext/work):
    // a real projected blocked automation must serialize BOTH the item-level
    // blocked_by_code and the decision-level blocked_reason_code, and they must
    // describe the sentence the payload actually carries.
    let mut task = test_task_for_projection();
    task.title = "Check server disk usage every morning".into();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("server-disk-check".into());

    let item = unified_inbox_item_from_task(&task, "en");
    let work = vnext_work_item(&item, "en");
    let json = serde_json::to_value(&work).unwrap();

    assert_eq!(
        json["blocked_by"],
        "Aegis paused this automation before a sensitive step."
    );
    assert_eq!(json["blocked_by_code"], "blocked_by_sensitive_step");
    assert_eq!(
        json["decision_request"]["blocked_reason"],
        "Aegis paused this automation before a sensitive step."
    );
    assert_eq!(
        json["decision_request"]["blocked_reason_code"],
        "blocked_by_sensitive_step"
    );
    // The projected title is derived from task content, so it stays uncoded.
    assert!(json.get("title_code").is_none());
}

#[test]
fn vnext_work_item_wire_carries_approval_title_code() {
    // Approval rows keep a static server title; its code must reach the FE so
    // the row title and the decide-card title cannot render in two languages.
    let item = unified_inbox_item_from_approval(
        "gate-mail-777",
        &serde_json::json!({
            "approval_id": "gate-mail-777",
            "tool_name": "calendar_update_event",
            "tool_args": serde_json::json!({ "message_ids": ["abc123"] }).to_string(),
        })
        .to_string(),
        "2026-07-24T00:00:00Z",
        "en",
    );
    assert_eq!(item.title, "Approval required");

    let json = serde_json::to_value(vnext_work_item(&item, "en")).unwrap();
    assert_eq!(json["title"], "Approval required");
    assert_eq!(json["title_code"], "approval_title_generic");
    assert_eq!(json["blocked_by_code"], "approval_waiting_owner");
}

#[test]
fn blocked_decision_request_uses_blocked_by_code_not_owner_reason_code() {
    let mut item =
        test_unified_item_for_vnext("task:disk-pause", "task", "blocked_by_gate", "high");
    item.title = "Automation paused".into();
    item.summary =
        "Automation paused before continuing: Check server disk usage every morning".into();
    item.owner_visible_reason =
        "A sensitive step needs owner review before Aegis continues.".into();
    item.owner_visible_reason_code = Some("blocked_generic_pause".into());
    item.blocked_by = Some("Aegis paused this automation before a sensitive step.".into());
    item.blocked_by_code = Some("blocked_by_sensitive_step".into());

    let decision = vnext_decision_request(&item, "en").expect("blocked decision");
    assert_eq!(
        decision.blocked_reason,
        "Aegis paused this automation before a sensitive step."
    );
    assert_eq!(
        decision.blocked_reason_code.as_deref(),
        Some("blocked_by_sensitive_step")
    );
    assert_ne!(
        decision.blocked_reason_code.as_deref(),
        item.owner_visible_reason_code.as_deref()
    );
}

#[test]
fn blocked_decision_request_falls_back_to_owner_visible_reason_code() {
    let mut item =
        test_unified_item_for_vnext("task:mail-pause", "task", "blocked_by_gate", "high");
    item.title = "Mail triage paused".into();
    item.owner_visible_reason =
        "This may read private email content, so Aegis paused before continuing.".into();
    item.owner_visible_reason_code = Some("blocked_generic_pause".into());
    item.blocked_by = None;
    item.blocked_by_code = None;

    let decision = vnext_decision_request(&item, "en").expect("blocked decision");
    assert_eq!(
        decision.blocked_reason,
        "This may read private email content, so Aegis paused before continuing."
    );
    assert_eq!(
        decision.blocked_reason_code.as_deref(),
        Some("blocked_generic_pause")
    );
}

#[test]
fn approval_decision_request_emits_title_capability_and_reason_codes() {
    let mut item = test_unified_item_for_vnext("approval:7", "approval", "needs_decision", "high");
    item.title = "Approve marking mail read?".into();
    item.title_code = Some("approval_title_generic".into());
    item.summary = "mark 1 selected Calendar event as read".into();
    item.owner_visible_reason =
        "Aegis needs your explicit approval before changing Calendar state.".into();
    item.owner_visible_reason_code = Some("approval_generic".into());
    item.next_safe_action =
        "Approve only if these are the messages you meant to mark read; otherwise reject.".into();
    item.next_safe_action_code = Some("approval_review_decide".into());
    item.blocked_by = Some("Waiting for your approve or reject decision.".into());
    item.blocked_by_code = Some("approval_waiting_decision".into());
    item.secondary_actions = vec![
        UnifiedInboxAction {
            label: "Approve".into(),
            label_code: None,
            action_kind: "approval.approve".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
        UnifiedInboxAction {
            label: "Reject".into(),
            label_code: None,
            action_kind: "approval.reject".into(),
            route: None,
            requires_gate: true,
            requires_mfa: false,
        },
    ];

    let decision = vnext_decision_request(&item, "en").expect("approval decision");
    assert_eq!(
        decision.title_code.as_deref(),
        Some("approval_title_generic")
    );
    assert_eq!(
        decision.capability_code.as_deref(),
        Some("nextCode:approval_review_decide")
    );
    assert_eq!(
        decision.blocked_reason_code.as_deref(),
        Some("approval_generic")
    );
    let codes = decision
        .safe_detail_codes
        .as_ref()
        .expect("safe_detail_codes");
    assert_eq!(codes.len(), decision.safe_details.len());
    // Each code must describe the bullet at the SAME index (the builder pairs
    // them in one tuple list; this pins the pairing, not just the length).
    assert_eq!(
        decision.safe_details[0],
        "No outside action runs until you choose."
    );
    assert_eq!(codes[0], "safe_no_outside_action");
    assert_eq!(decision.safe_details[1], item.summary);
    assert_eq!(codes[1], "", "interpolated summary stays freeform");
    assert_eq!(decision.safe_details[2], item.next_safe_action);
    assert_eq!(codes[2], "nextCode:approval_review_decide");
    assert_eq!(
        decision.safe_details[3],
        item.blocked_by.clone().expect("blocked_by")
    );
    assert_eq!(codes[3], "reasonCode:approval_waiting_decision");
}

#[test]
fn approval_decision_request_clears_title_code_when_title_truncated() {
    let mut item =
        test_unified_item_for_vnext("approval:long", "approval", "needs_decision", "high");
    // 130 chars of 'A' — owner_visible_title truncates at 120 and appends "...".
    item.title = "A".repeat(130);
    item.title_code = Some("approval_title_generic".into());
    item.secondary_actions = vec![UnifiedInboxAction {
        label: "Approve".into(),
        label_code: None,
        action_kind: "approval.approve".into(),
        route: None,
        requires_gate: true,
        requires_mfa: false,
    }];

    let decision = vnext_decision_request(&item, "en").expect("approval decision");
    assert_eq!(decision.title.chars().count(), 123); // 120 + "..."
    assert_eq!(decision.title_code, None);
}

#[test]
fn close_paused_task_update_marks_done_without_running_blocked_step() {
    let mut task = test_task_for_projection();
    task.title =
        "检查 primary 账户的未读邮件，使用 server_monitor 自动分类：将标记为‘重要’的邮".into();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("primary-mail-check".into());

    let (updates, action_kind, receipt_suffix, summary, due_at) = build_webapp_task_action_update(
        &task,
        "complete",
        None,
        chrono::Utc::now(),
        chrono_tz::Tz::UTC,
    )
    .expect("complete action should be valid for a paused task");

    assert_eq!(updates["status"], "done");
    assert!(updates.get("completed_at").is_some());
    assert_eq!(
        action_kind,
        crate::action_ledger::ActionKind::ReminderUpdate
    );
    assert_eq!(receipt_suffix, "complete");
    assert_eq!(summary, format!("Task #{} marked complete", task.id));
    assert!(due_at.is_none());
}

#[test]
fn completing_a_clock_loop_keeps_the_same_row_open() {
    let mut task = test_task_for_projection();
    task.id = 2138;
    task.title = "上传文章".into();
    task.status = "active".into();
    task.execution_mode = "notify".into();
    task.trigger_type = Some("time".into());
    task.interval_seconds = Some(86_400);
    task.fire_count = 7;
    task.source = "user".into();
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T06:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let future = now.timestamp() + 3_600;
    task.trigger_at_unix = Some(future);

    let (updates, _, _, summary, due_at) =
        build_webapp_task_action_update(&task, "complete", None, now, chrono_tz::Tz::UTC)
            .expect("complete");

    assert_eq!(updates["status"], "active");
    assert_eq!(updates["trigger_at_unix"], future);
    assert!(updates.get("clear_fields").is_some());
    assert_ne!(updates["status"], "done");
    assert_eq!(summary, "Task #2138 marked complete");
    assert!(due_at.is_some());

    task.status = "done".into();
    assert!(closed_clock_loop_should_reopen(&task));
    let item = unified_inbox_item_from_task(
        &crate::memory::Task {
            status: "active".into(),
            ..task
        },
        "zh",
    );
    assert!(item.title.contains("上传文章"), "{}", item.title);
    assert_eq!(item.status, "tracking");
}

#[test]
fn exhausted_or_one_shot_complete_still_closes() {
    let mut one_shot = test_task_for_projection();
    one_shot.status = "done".into();
    assert!(!closed_clock_loop_should_reopen(&one_shot));

    let mut exhausted = test_task_for_projection();
    exhausted.status = "done".into();
    exhausted.interval_seconds = Some(86_400);
    exhausted.max_fires = Some(3);
    exhausted.fire_count = 3;
    assert!(!closed_clock_loop_should_reopen(&exhausted));
    assert!(!clock_loop_still_scheduled(&exhausted));
}

#[test]
fn snoozed_task_summary_uses_user_timezone_not_utc() {
    let mut task = test_task_for_projection();
    task.execution_mode = "notify".into();
    task.trigger_at_unix = Some(1_779_485_400);
    let now = chrono::DateTime::parse_from_rfc3339("2026-05-23T04:38:14Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let (updates, action_kind, receipt_suffix, summary, due_at) = build_webapp_task_action_update(
        &task,
        "snooze",
        Some("1h"),
        now,
        chrono_tz::America::Los_Angeles,
    )
    .expect("snooze action should be valid");

    assert_eq!(updates["status"], "active");
    assert_eq!(
        action_kind,
        crate::action_ledger::ActionKind::ReminderUpdate
    );
    assert_eq!(receipt_suffix, "1h");
    assert_eq!(
        summary,
        format!("Task #{} snoozed until 2026-05-22 22:38 PDT", task.id)
    );
    assert_eq!(due_at.as_deref(), Some("2026-05-23T05:38:14+00:00"));
    assert_eq!(updates["due_date"], "2026-05-23T05:38:14+00:00");
}

#[test]
fn active_background_loops_are_tracking_not_today_focus() {
    let mut task = test_task_for_projection();
    task.execution_mode = "notify".into();
    task.trigger_type = Some("time".into());
    task.trigger_at_unix = Some((chrono::Utc::now() + chrono::Duration::hours(12)).timestamp());
    task.interval_seconds = Some(24 * 60 * 60);

    let item = unified_inbox_item_from_task(&task, "en");
    assert_eq!(item.status, "tracking");
    assert_eq!(item.urgency, "normal");
    assert!(item.summary.contains("Tracking item."));
    assert!(item.owner_visible_reason.contains("open follow-up loop"));

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "follow_up");
    assert_eq!(
        projected
            .contextual_actions
            .iter()
            .map(|action| action.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["task.skip_occurrence", "tracking.cancel"]
    );
    assert!(projected.lineage.iter().any(|node| node.layer == "intent"));
    assert!(
        projected
            .lineage
            .iter()
            .all(|node| node.layer != "occurrence")
    );
    assert!(projected.lineage.iter().any(
        |node| node.detail.contains("Every day") && !node.detail.contains("interval_seconds=")
    ));

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Nothing urgent is asking for you right now.");
    assert!(home.top_now.is_empty());
    assert_eq!(home.follow_up_highlights[0].id, "task:42");
}

#[test]
fn active_notify_loop_without_due_time_stays_follow_up() {
    let mut task = test_task_for_projection();
    task.title = "通勤低交通晚高峰警报 (Tue-Thu 17:00-19:00 <60min，首次通知后跳过)".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("commute-low-traffic".into());

    let item = unified_inbox_item_from_task(&task, "en");
    assert_eq!(item.status, "tracking");
    assert!(item.summary.contains("Tracking item."));

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "follow_up");

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Nothing urgent is asking for you right now.");
    assert!(home.top_now.is_empty());
}

#[test]
fn near_term_scheduled_tasks_enter_today_focus() {
    let mut task = test_task_for_projection();
    task.execution_mode = "notify".into();
    task.trigger_type = Some("time".into());
    task.trigger_at_unix = Some((chrono::Utc::now() + chrono::Duration::hours(12)).timestamp());

    let item = unified_inbox_item_from_task(&task, "en");
    assert_eq!(item.status, "scheduled");
    assert!(item.summary.contains("Scheduled task."));
    assert!(item.owner_visible_reason.contains("scheduled check"));

    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.lane, "now");

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "en");
    assert_eq!(
        home.brief,
        "Coming up: Check commute if traffic drops below one hour."
    );
    assert_eq!(home.top_now[0].id, "task:42");
}

#[test]
fn vnext_home_source_attention_uses_specific_owner_language() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "auth_required".into(),
            reason: None,
        }],
        counts: UnifiedInboxCounts::default(),
        items: Vec::new(),
    };

    let home = vnext_home_projection(&projection, "en");
    assert_eq!(home.brief, "Calendar: Google sign-in needs refresh.");
    assert_eq!(
        home.status_entry.owner_message,
        "Calendar: Google sign-in needs refresh."
    );
    assert_eq!(
        home.status_entry.owner_next_step.as_deref(),
        Some("Refresh the Google connection from Settings before expecting new private updates.")
    );
}

#[test]
fn vnext_status_projection_uses_owner_language_without_internal_metadata_terms() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![UnifiedInboxSourceStatus {
            source: "calendar".into(),
            status: "available".into(),
            reason: Some("assistant event metadata only".into()),
        }],
        counts: UnifiedInboxCounts::default(),
        items: vec![
            test_unified_item_for_vnext("ready", "email", "unread", "normal"),
            test_unified_item_for_vnext("decision", "email", "needs_decision", "high"),
            test_unified_item_for_vnext("waiting", "task", "tracking", "normal"),
        ],
    };

    let status = vnext_status_projection(&projection, "en");
    let body = serde_json::to_string(&status).unwrap();

    assert!(body.contains("Background Review"));
    assert!(body.contains("Recommendations"));
    assert!(body.contains("need your choice"));
    assert!(body.contains("waiting or tracking"));
    assert!(body.contains("receipts before claiming"));
    assert!(body.contains("Receipts"));
    for forbidden in [
        "Proof",
        "projecting safe metadata",
        "current work items are visible",
        "safe metadata",
        "raw private content here",
    ] {
        assert!(
            !body.contains(forbidden),
            "vNext status should use owner language, not internal wording: {forbidden}"
        );
    }
}

#[test]
fn vnext_status_projection_collapses_low_value_system_events_outside_chat() {
    let events = vnext_system_events_from_l3(
        vec![
            crate::memory::L3Entry {
                id: 1,
                agent_did: "did:test".into(),
                role: "assistant".into(),
                content:
                    "系统已重启 / System restarted\n当前重要信息\n活跃技能 / Active skills: 22"
                        .into(),
                created_at: "2026-05-22T05:00:00+00:00".into(),
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
            },
            crate::memory::L3Entry {
                id: 2,
                agent_did: "did:test".into(),
                role: "assistant".into(),
                content:
                    "系统已重启 / System restarted\n当前重要信息\n活跃技能 / Active skills: 22"
                        .into(),
                created_at: "2026-05-22T06:00:00+00:00".into(),
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
            },
            crate::memory::L3Entry {
                id: 3,
                agent_did: "did:test".into(),
                role: "assistant".into(),
                content: "# Daily Brief for 2026-05-22\nCalendar not queried; Calendar not queried"
                    .into(),
                created_at: "2026-05-22T16:00:00+00:00".into(),
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
            },
            crate::memory::L3Entry {
                id: 4,
                agent_did: "did:test".into(),
                role: "user".into(),
                content: "普通聊天不应该进入系统事件".into(),
                created_at: "2026-05-22T17:00:00+00:00".into(),
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
            },
        ],
        "en",
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "incomplete_daily_brief");
    assert_eq!(events[0].severity, "notice");
    assert_eq!(events[1].kind, "restart");
    assert_eq!(events[1].count, 2);
    assert_eq!(events[1].title, "System restarted");
    assert!(
        events[1]
            .details
            .iter()
            .any(|detail| detail.contains("Active skills: 22")),
        "{:?}",
        events[1].details
    );
    assert!(
        events[1]
            .details
            .iter()
            .all(|detail| !detail.contains("System restarted")),
        "{:?}",
        events[1].details
    );

    let zh = vnext_system_events_from_l3(
        vec![crate::memory::L3Entry {
            id: 1,
            agent_did: "did:test".into(),
            role: "assistant".into(),
            content: "系统已重启 / System restarted\n活跃技能 / Active skills: 22\nHITL 待审批: 0"
                .into(),
            created_at: "2026-05-22T05:00:00+00:00".into(),
            anchor_kind: None,
            anchor_ref: None,
            latency_ms: None,
        }],
        "zh",
    );
    let restart = zh.iter().find(|event| event.kind == "restart").unwrap();
    let detail = restart.details.join("\n");
    assert!(detail.contains("活跃技能：22"), "{detail}");
    assert!(detail.contains("待你确认：0"), "{detail}");
    assert!(!detail.contains("Active skills"), "{detail}");
    assert!(!detail.contains("System restarted"), "{detail}");
    assert!(!detail.contains("HITL"), "{detail}");
}

#[test]
fn vnext_status_projection_can_surface_cost_control_cadence() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings
        .set(
            "health_probe_interval_minutes",
            serde_json::Value::Number(serde_json::Number::from(120)),
        )
        .unwrap();

    let status = vnext_status_projection_with_settings(&projection, Some(&settings), "en");
    let cost = status
        .sections
        .iter()
        .find(|section| section.label == "Cost controls")
        .expect("cost controls section should be present when settings are supplied");

    assert_eq!(cost.health, "ready");
    assert!(
        cost.owner_message
            .contains("system health checks every 120 minutes")
    );
    for forbidden in ["heartbeat", "memory consolidation", "health probes"] {
        assert!(
            !cost.owner_message.contains(forbidden),
            "cost controls should not expose internal module wording: {forbidden}"
        );
    }
    assert!(
        cost.owner_next_step
            .as_deref()
            .unwrap_or("")
            .contains("Settings")
    );
}

#[test]
fn vnext_status_projection_reports_cost_saving_mode_when_background_autonomy_is_paused() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings
        .set(
            "health_probe_interval_minutes",
            serde_json::Value::Number(serde_json::Number::from(0)),
        )
        .unwrap();
    settings
        .set(
            "true_heartbeat_config",
            serde_json::json!({
                "enabled": false,
                "interval_minutes": 120,
                "autonomous_overrides": null
            }),
        )
        .unwrap();
    settings
        .set(
            "heartbeat_config",
            serde_json::json!({
                "frequency_minutes": 120,
                "quiet_hours": {"start": "00:00", "end": "23:59"},
                "next_tick_note_max_bytes": 1024,
                "per_tick": {"tool_calls": 0, "send_message": 0},
                "dedup": {"cosine_threshold": 0.95},
                "controlled_thaw": {
                    "enabled": false,
                    "max_llm_wakeups_per_hour": 0,
                    "max_background_proposals_per_day": 0,
                    "require_approval_for_external_actions": true
                }
            }),
        )
        .unwrap();

    let status = vnext_status_projection_with_settings(&projection, Some(&settings), "en");
    let cost = status
        .sections
        .iter()
        .find(|section| section.label == "Cost controls")
        .expect("cost controls section should be present when settings are supplied");

    assert_eq!(cost.health, "quiet");
    assert!(cost.owner_message.contains("Cost-saving mode is on"));
    assert!(
        cost.owner_message
            .contains("self-started background work is paused")
    );
    assert!(
        cost.owner_message
            .contains("checks requested by you still run")
    );
    for forbidden in ["heartbeat", "memory consolidation"] {
        assert!(
            !cost.owner_message.contains(forbidden),
            "cost controls should use owner-facing language, not internal term: {forbidden}"
        );
    }
}

#[test]
fn vnext_cost_saving_status_requires_scheduled_skill_checks_to_be_paused() {
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: vec![],
        counts: UnifiedInboxCounts::default(),
        items: vec![],
    };
    let settings = RuntimeSettings::new();
    settings.register_skill("calendar_reader", "Calendar heartbeat checks", true);
    settings
        .set("skill_calendar_reader", serde_json::Value::Bool(true))
        .unwrap();
    settings
        .set("health_probe_interval_minutes", serde_json::json!(0))
        .unwrap();
    settings
        .set(
            "true_heartbeat_config",
            serde_json::json!({
                "enabled": false,
                "interval_minutes": 120,
                "autonomous_overrides": null
            }),
        )
        .unwrap();
    settings
        .set(
            "heartbeat_config",
            serde_json::json!({
                "frequency_minutes": 120,
                "quiet_hours": {"start": "00:00", "end": "23:59"},
                "next_tick_note_max_bytes": 1024,
                "per_tick": {"tool_calls": 0, "send_message": 0},
                "dedup": {"cosine_threshold": 0.95},
                "controlled_thaw": {
                    "enabled": false,
                    "max_llm_wakeups_per_hour": 0,
                    "max_background_proposals_per_day": 0,
                    "require_approval_for_external_actions": true
                }
            }),
        )
        .unwrap();

    assert!(
        !vnext_cost_saving_paused(&settings),
        "a scheduled skill check left enabled should prevent claiming full cost-saving pause"
    );
    let status = vnext_status_projection_with_settings(&projection, Some(&settings), "en");
    let cost = status
        .sections
        .iter()
        .find(|section| section.label == "Cost controls")
        .expect("cost controls section should be present when settings are supplied");

    assert_eq!(cost.health, "ready");
    assert!(
        cost.owner_message
            .contains("1 of 1 scheduled skill checks can run")
    );
}

#[tokio::test]
async fn webapp_settings_cost_saving_pause_disables_background_autonomy() {
    let state = test_state();
    state
        .settings
        .register_skill("calendar_reader", "Calendar heartbeat checks", true);
    state
        .settings
        .register_skill("server_monitor", "Server heartbeat checks", true);
    state
        .settings
        .set("skill_calendar_reader", serde_json::Value::Bool(true))
        .unwrap();
    state
        .settings
        .set("skill_server_monitor", serde_json::Value::Bool(true))
        .unwrap();
    for key in [
        "heartbeat_autonomy_v2",
        "news_digest",
        "calendar_briefing",
        "server_monitoring",
        "github_alerts",
        "email_notifications",
        "skill_news_digest",
    ] {
        state
            .memory
            .store_setting(key, serde_json::Value::Bool(true))
            .await
            .unwrap();
    }
    state
        .settings
        .set("health_probe_interval_minutes", serde_json::json!(60))
        .unwrap();
    state
        .settings
        .set(
            "true_heartbeat_config",
            serde_json::json!({
                "enabled": true,
                "interval_minutes": 15,
                "autonomous_overrides": null
            }),
        )
        .unwrap();
    let mut heartbeat_config = crate::heartbeat_config::HeartbeatConfig::default();
    heartbeat_config.per_tick.tool_calls = 8;
    heartbeat_config.per_tick.send_message = 1;
    heartbeat_config.controlled_thaw.enabled = true;
    state
        .settings
        .set("heartbeat_config", heartbeat_config.to_json())
        .unwrap();

    pause_background_autonomy_for_cost_saving(&state)
        .await
        .expect("pause background autonomy");

    assert!(!state.settings.is_enabled("skill_calendar_reader"));
    assert!(!state.settings.is_enabled("skill_server_monitor"));
    assert_eq!(
        state
            .settings
            .get_u64_clamped("health_probe_interval_minutes", 60, 0, 24 * 60),
        0
    );
    assert_eq!(
        state
            .settings
            .get("true_heartbeat_config")
            .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool)),
        Some(false)
    );
    let paused_heartbeat = state
        .settings
        .get("heartbeat_config")
        .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(&value))
        .expect("paused heartbeat config");
    assert_eq!(paused_heartbeat.per_tick.tool_calls, 0);
    assert_eq!(paused_heartbeat.per_tick.send_message, 0);
    assert!(!paused_heartbeat.controlled_thaw.enabled);
    assert!(vnext_cost_saving_paused(&state.settings));

    let persisted = state.memory.load_all_settings().await.unwrap();
    assert_eq!(
        persisted.get("heartbeat_autonomy_v2"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        persisted.get("news_digest"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        persisted.get("skill_news_digest"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        persisted.get("skill_calendar_reader"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        persisted.get("skill_server_monitor"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        persisted
            .get("heartbeat_config")
            .and_then(|value| crate::heartbeat_config::HeartbeatConfig::from_json(value))
            .map(|config| config.per_tick.tool_calls),
        Some(0)
    );
}

#[test]
fn vnext_conversation_context_is_trimmed_to_screen_plus_newest() {
    let mut turns: Vec<VNextCommandConversationTurn> = (0..12)
        .map(|i| VNextCommandConversationTurn {
            role: if i % 2 == 0 {
                "owner".into()
            } else {
                "aegis".into()
            },
            text: format!("t{i}"),
            kind: None,
            source_id: None,
        })
        .collect();
    trim_vnext_conversation_context(&mut turns);
    assert_eq!(turns.len(), VNEXT_MAX_CONVERSATION_CONTEXT_TURNS);
    assert_eq!(turns[0].text, "t0", "screen/first turn kept");
    assert_eq!(turns[1].text, "t5");
    assert_eq!(turns.last().unwrap().text, "t11");
    assert!(validate_vnext_command_conversation_context(Some(&turns)).is_ok());

    let mut short: Vec<VNextCommandConversationTurn> = vec![VNextCommandConversationTurn {
        role: "owner".into(),
        text: "only".into(),
        kind: None,
        source_id: None,
    }];
    trim_vnext_conversation_context(&mut short);
    assert_eq!(short.len(), 1);
}

fn assert_zh_owner_copy_is_not_ascii_or_jargon(blob: &str) {
    for banned in ["回执", "事项", "幕僚长", "主人", "边界", "拦截", "投影"] {
        assert!(
            !blob.contains(banned),
            "zh copy contains banned jargon {banned}: {blob}"
        );
    }
    for sentence in blob.split(['。', '.', '！', '!', '？', '?']) {
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        if words.len() >= 4 && trimmed.is_ascii() {
            panic!("zh copy still has an ASCII-only sentence: {trimmed}");
        }
    }
}

#[test]
fn inbox_projection_zh_locale_has_no_ascii_only_sentence_or_banned_jargon() {
    let mut task = test_task_for_projection();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("server-disk-check".into());
    let item = unified_inbox_item_from_task(&task, "zh");
    let blob = format!(
        "{}\n{}\n{}\n{}\n{}",
        item.title,
        item.summary,
        item.owner_visible_reason,
        item.next_safe_action,
        item.blocked_by.as_deref().unwrap_or("")
    );
    assert_zh_owner_copy_is_not_ascii_or_jargon(&blob);
}

#[test]
fn console_handlers_zh_locale_has_no_ascii_only_sentence_or_banned_jargon() {
    let mut task = test_task_for_projection();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("server-disk-check".into());
    let item = unified_inbox_item_from_task(&task, "zh");
    let decision = vnext_decision_request(&item, "zh").expect("blocked decision");
    let blob = format!(
        "{}\n{}\n{}\n{}",
        decision.title,
        decision.what_aegis_was_doing,
        decision.requested_capability,
        decision
            .choices
            .iter()
            .map(|choice| format!(
                "{} {} {}",
                choice.label,
                choice.consequence,
                choice.disabled_reason.as_deref().unwrap_or("")
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_zh_owner_copy_is_not_ascii_or_jargon(&blob);
}

#[test]
fn vnext_projections_zh_locale_has_no_ascii_only_sentence_or_banned_jargon() {
    let mut task = test_task_for_projection();
    task.status = "blocked_by_gate".into();
    task.execution_mode = "notify".into();
    task.group_key = Some("server-disk-check".into());
    let item = unified_inbox_item_from_task(&task, "zh");
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: "2026-05-10T00:00:00Z".into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items: vec![item],
    };
    let home = vnext_home_projection(&projection, "zh");
    let status = vnext_status_projection(&projection, "zh");
    let blob = format!(
        "{}\n{}",
        home.brief,
        status
            .sections
            .iter()
            .map(|section| format!("{} {}", section.label, section.owner_message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_zh_owner_copy_is_not_ascii_or_jargon(&blob);
}

fn test_completed_ledger_entry(
    id: i64,
    executed_at: &str,
) -> crate::action_ledger::ActionLedgerEntry {
    let mut entry = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::Executed,
        crate::action_ledger::ActionKind::EmailSend,
    );
    entry.id = id;
    entry.action_id = format!("test-action:{id}");
    entry.executed_at = Some(executed_at.into());
    entry.created_at = executed_at.into();
    entry.external_receipt = Some(format!("ext-{id}"));
    entry
}

fn test_failed_ledger_entry(id: i64, failed_at: &str) -> crate::action_ledger::ActionLedgerEntry {
    let mut entry = test_action_ledger_entry_for_vnext(
        crate::action_ledger::ActionStatus::Failed,
        crate::action_ledger::ActionKind::EmailSend,
    );
    entry.id = id;
    entry.action_id = format!("test-action:{id}");
    entry.failed_at = Some(failed_at.into());
    entry.created_at = failed_at.into();
    entry
}

fn test_follow_up_item(id: &str, kind: &str, status: &str, updated_at: &str) -> UnifiedInboxItem {
    let mut item = test_unified_item_for_vnext(id, kind, status, "normal");
    item.updated_at = updated_at.into();
    item.created_at = updated_at.into();
    item
}

fn test_goal_follow_up(
    id: &str,
    updated_at: &str,
    zero_progress: bool,
    has_checklist: bool,
) -> UnifiedInboxItem {
    let mut item = test_follow_up_item(id, "goal", "tracking", updated_at);
    item.owner_visible_reason_code = Some(
        if zero_progress && !has_checklist {
            GOAL_NO_PROGRESS_REASON_CODE
        } else {
            "active_goal_ongoing"
        }
        .into(),
    );
    item.summary = match (zero_progress, has_checklist) {
        (true, false) => "0% progress. No milestone checklist is attached.".into(),
        (true, true) => "0% progress. 0/3 milestones complete.".into(),
        (false, false) => "40% progress. No milestone checklist is attached.".into(),
        (false, true) => "40% progress. 1/3 milestones complete.".into(),
    };
    item
}

fn work_lane_items<'a>(work: &'a VNextWorkProjection, lane_id: &str) -> &'a [VNextWorkItem] {
    work.lanes
        .iter()
        .find(|lane| lane.id == lane_id)
        .map(|lane| lane.items.as_slice())
        .unwrap_or(&[])
}

#[test]
fn executed_receipts_same_day_group_into_one_backlog_row() {
    let day = "2026-09-07T12:00:00Z";
    let mut entries: Vec<_> = (1..=5)
        .map(|id| test_completed_ledger_entry(id, day))
        .collect();
    entries.push(test_failed_ledger_entry(99, day));

    let items = project_ledger_entries_into_inbox(&entries, "en");
    let grouped: Vec<_> = items
        .iter()
        .filter(|item| is_grouped_receipt_item(item))
        .collect();
    let failed: Vec<_> = items
        .iter()
        .filter(|item| item.status == "failed")
        .collect();

    assert_eq!(
        grouped.len(),
        1,
        "five executed receipts on one day become one row"
    );
    assert_eq!(grouped[0].kind, "receipt");
    assert_eq!(vnext_source_kind(grouped[0]), "receipt");
    assert_eq!(grouped[0].title, "5 completed today (Email sent: email…)");
    assert_eq!(grouped[0].related_receipt_ids.len(), 5);
    assert_eq!(
        grouped[0].detail_route.as_deref(),
        Some("/webapp/console/ledger/1")
    );
    assert_eq!(failed.len(), 1, "failed entries still project individually");
    assert!(!is_grouped_receipt_item(failed[0]));

    let zh_items = project_ledger_entries_into_inbox(&entries[..5], "zh");
    assert_eq!(zh_items.len(), 1);
    assert_eq!(
        zh_items[0].title,
        "最近记录：5 条已完成（已发出邮件：邮件…）"
    );
}

#[test]
fn follow_up_is_stale_zero_progress_goal_after_30_days() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let stale = test_goal_follow_up("goal:stale", "2026-08-07T00:00:00Z", true, false);
    let on_boundary = test_goal_follow_up("goal:edge", "2026-08-08T00:00:00Z", true, false);
    let with_checklist = test_goal_follow_up("goal:list", "2026-05-01T00:00:00Z", true, true);
    let progressing = test_goal_follow_up("goal:move", "2026-05-01T00:00:00Z", false, false);

    assert!(follow_up_is_stale(&stale, now));
    assert!(!follow_up_is_stale(&on_boundary, now));
    assert!(!follow_up_is_stale(&with_checklist, now));
    assert!(!follow_up_is_stale(&progressing, now));
}

#[test]
fn follow_up_is_stale_tracking_item_after_14_days() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let stale = test_follow_up_item(
        "kb_waiting:1",
        "waiting_for",
        "waiting",
        "2026-08-23T00:00:00Z",
    );
    let fresh = test_follow_up_item(
        "kb_waiting:2",
        "waiting_for",
        "waiting",
        "2026-08-24T00:00:00Z",
    );

    assert!(follow_up_is_stale(&stale, now));
    assert!(!follow_up_is_stale(&fresh, now));
}

#[test]
fn follow_up_is_stale_fired_task_after_7_days() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let stale = test_follow_up_item("task:fired", "task", "fired", "2026-08-30T00:00:00Z");
    let recent = test_follow_up_item("task:fired-new", "task", "fired", "2026-08-31T00:00:00Z");

    assert!(follow_up_is_stale(&stale, now));
    assert!(!follow_up_is_stale(&recent, now));
}

#[test]
fn duplicate_tasks_in_the_same_lane_collapse_with_count_and_links() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut first = test_follow_up_item("task:1", "task", "waiting", "2026-09-06T00:00:00Z");
    first.title = "  Passport  renewal ".into();
    let mut second = test_follow_up_item("task:2", "task", "waiting", "2026-09-06T01:00:00Z");
    second.title = "passport renewal".into();
    let mut other_lane = test_follow_up_item("task:3", "task", "backlog", "2026-09-06T00:00:00Z");
    other_lane.title = "passport renewal".into();
    other_lane.urgency = "low".into();

    let collapsed = denoise_work_inbox_items(vec![first, second, other_lane], now, "en");
    let follow_ups: Vec<_> = collapsed
        .iter()
        .filter(|item| vnext_lane_for_item_at(item, now) == "follow_up")
        .collect();
    assert_eq!(follow_ups.len(), 1);
    assert!(follow_ups[0].summary.contains("×2"));
    assert_eq!(
        follow_ups[0].related_action_ids,
        vec!["task:1".to_string(), "task:2".to_string()]
    );
    assert_eq!(
        collapsed
            .iter()
            .filter(|item| vnext_lane_for_item_at(item, now) == "backlog")
            .count(),
        1
    );
}

#[test]
fn vnext_home_brief_stays_on_quiet_key_and_hides_stale_or_receipt_highlights() {
    let now = "2026-09-07T00:00:00Z";
    let stale_goal = test_goal_follow_up("goal:stale", "2026-05-01T00:00:00Z", true, false);
    let stale_tracking = test_follow_up_item(
        "kb_waiting:old",
        "waiting_for",
        "waiting",
        "2026-05-01T00:00:00Z",
    );
    let mut live = test_follow_up_item("kb_waiting:live", "waiting_for", "waiting", now);
    live.title = "Wait for the school".into();
    let receipt_entries: Vec<_> = (1..=3)
        .map(|id| test_completed_ledger_entry(id, now))
        .collect();
    let receipts = project_ledger_entries_into_inbox(&receipt_entries, "en");

    let mut items = vec![stale_goal, stale_tracking, live];
    items.extend(receipts);
    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: now.into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items,
    };

    let en = vnext_home_projection(&projection, "en");
    assert_eq!(en.brief, crate::i18n::t("console.home.brief.quiet", "en"));
    assert_eq!(en.follow_up_highlights.len(), 1);
    assert_eq!(en.follow_up_highlights[0].id, "kb_waiting:live");
    assert!(
        en.follow_up_highlights
            .iter()
            .all(|item| item.source_kind != "receipt")
    );

    let zh = vnext_home_projection(&projection, "zh");
    assert_eq!(zh.brief, "这会儿没什么急着找你的。");
    assert_eq!(zh.brief, crate::i18n::t("console.home.brief.quiet", "zh"));
}

#[test]
fn work_page_production_shape_parks_stale_groups_receipts_and_collapses_dupes() {
    let now = "2026-09-07T00:00:00Z";
    let mut items = vec![
        test_goal_follow_up("goal:a", "2026-05-01T00:00:00Z", true, false),
        test_goal_follow_up("goal:b", "2026-04-01T00:00:00Z", true, false),
        test_goal_follow_up("goal:c", "2026-03-01T00:00:00Z", true, false),
        test_follow_up_item(
            "kb_waiting:pr",
            "waiting_for",
            "waiting",
            "2026-08-01T00:00:00Z",
        ),
    ];
    let mut passport_a = test_follow_up_item("task:pass-a", "task", "waiting", now);
    passport_a.title = "Passport renewal".into();
    let mut passport_b = test_follow_up_item("task:pass-b", "task", "waiting", now);
    passport_b.title = "passport  renewal".into();
    items.push(passport_a);
    items.push(passport_b);
    for (id, title) in [
        ("task:dup-a1", "School billing"),
        ("task:dup-a2", "school billing"),
        ("task:dup-b1", "Lawyer callback"),
        ("task:dup-b2", "lawyer callback"),
        ("kb_waiting:live-1", "Wait for a person"),
        ("kb_waiting:live-2", "Wait for the contractor"),
    ] {
        let kind = if id.starts_with("task:") {
            "task"
        } else {
            "waiting_for"
        };
        let mut item = test_follow_up_item(id, kind, "waiting", now);
        item.title = title.into();
        items.push(item);
    }
    let receipt_entries: Vec<_> = (1..=6)
        .map(|id| test_completed_ledger_entry(id, now))
        .collect();
    items.extend(project_ledger_entries_into_inbox(&receipt_entries, "en"));

    assert_eq!(
        items
            .iter()
            .filter(|item| vnext_lane_for_item(item) == "follow_up")
            .count(),
        12,
        "fixture should start from today's 12 follow-up shape"
    );

    let projection = UnifiedInboxProjection {
        schema_version: 1,
        generated_at: now.into(),
        owner: "owner".into(),
        sources: Vec::new(),
        counts: UnifiedInboxCounts::default(),
        items,
    };
    let work = vnext_work_projection(&projection, "zh");
    let follow_up = work_lane_items(&work, "follow_up");
    let backlog = work_lane_items(&work, "backlog");
    let done = work_lane_items(&work, "done");
    assert!(
        follow_up.len() <= 5,
        "follow_up should drop stale and duplicate rows, got {}",
        follow_up.len()
    );
    assert_eq!(
        backlog.len(),
        1,
        "six executed receipts collapse to one day row"
    );
    assert!(
        done.len() >= 4,
        "stale goals and tracking should park in done, got {}",
        done.len()
    );
    assert!(done.iter().all(|item| item.lane == "done"));
    assert!(
        done.iter()
            .any(|item| { item.why_it_matters == crate::i18n::t("console.work.stale.goal", "zh") })
    );
    assert!(
        follow_up
            .iter()
            .any(|item| item.summary.contains('×') && item.related_work_item_ids.len() >= 2)
    );
}

#[test]
fn goal_projection_marks_zero_progress_without_checklist_structurally() {
    let goal = crate::memory::Goal {
        id: 7,
        user_did: "did:test:owner".into(),
        title: "Learn Spanish".into(),
        description: "Private goal description".into(),
        status: crate::memory::GoalStatus::Active,
        parent_goal_id: None,
        progress_pct: 0.0,
        milestones: Vec::new(),
        created_at: "2026-05-11T00:00:00Z".into(),
        target_date: None,
        last_progress_at: None,
        source: crate::memory::GoalSource::Explicit,
    };
    let item = unified_inbox_item_from_goal(&goal, "zh");
    assert_eq!(
        item.owner_visible_reason_code.as_deref(),
        Some(GOAL_NO_PROGRESS_REASON_CODE)
    );

    let moving = crate::memory::Goal {
        progress_pct: 12.0,
        ..goal.clone()
    };
    let item = unified_inbox_item_from_goal(&moving, "zh");
    assert_eq!(
        item.owner_visible_reason_code.as_deref(),
        Some("active_goal_ongoing")
    );
}

#[test]
fn vnext_receipt_group_bounds_reference_previews_without_changing_full_group() {
    let mut item =
        test_unified_item_for_vnext("receipt:day:2026-09-17", "receipt", "executed", "low");
    item.summary = "42 completed actions".into();
    item.related_action_ids = (1..=42).map(|id| format!("action:{id}")).collect();
    item.related_receipt_ids = (1..=42).map(|id| format!("receipt:{id}")).collect();
    let projected = vnext_work_item(&item, "en");
    assert_eq!(projected.summary, "42 completed actions");
    assert_eq!(projected.related_work_item_ids.len(), 32);
    assert_eq!(projected.related_receipt_ids.len(), 32);
    assert_eq!(projected.related_work_item_ids.first().unwrap(), "action:1");
    assert_eq!(projected.related_receipt_ids.last().unwrap(), "receipt:32");
    let context = build_context_envelope_from_inbox_item(&item);
    assert_eq!(context.related_action_ids.len(), 16);
    assert_eq!(context.related_receipt_ids.len(), 16);
    assert_eq!(
        context.visible_fields.get("summary").unwrap(),
        "42 completed actions"
    );
    assert_eq!(item.related_action_ids.len(), 42);
    assert_eq!(item.related_receipt_ids.len(), 42);
}

#[test]
fn recurring_skip_trigger_advances_one_interval() {
    assert_eq!(recurring_skip_trigger(1_000, Some(5_000), 3_600), 8_600);
    assert_eq!(recurring_skip_trigger(1_000, None, 3_600), 4_600);
}

#[test]
fn one_shot_intent_only_offers_close_tracking() {
    let mut task = test_task_for_projection();
    task.execution_mode = "notify".into();
    task.trigger_type = Some("time".into());
    task.trigger_at_unix = Some(1_800_000_000);
    task.context_text = Some("周末给门铃充电".into());

    let item = unified_inbox_item_from_task(&task, "zh");
    let kinds: Vec<_> = item
        .secondary_actions
        .iter()
        .map(|action| action.action_kind.as_str())
        .collect();
    assert_eq!(kinds, vec!["tracking.cancel"]);
    assert!(item.lineage.iter().any(|node| {
        node.layer == "intent"
            && node.detail.contains("任务 42")
            && !node.detail.contains("trigger_type=")
            && !node.detail.contains("group_key=")
            && node.action_ids == vec!["tracking.cancel".to_string()]
    }));
    assert!(item.lineage.iter().any(|node| {
        node.layer == "origin"
            && node.detail.contains("你")
            && node.detail.contains("2026-05-10T00:00:00Z")
            && node.detail.contains("周末给门铃充电")
            && !node.detail.contains("context_text=")
            && !node.detail.contains("source=")
            && !node.detail.contains("created_at=")
    }));
    assert!(item.lineage.iter().all(|node| node.layer != "occurrence"));
}

#[test]
fn task_source_line_names_who_when_and_wording() {
    let mut task = test_task_for_projection();
    task.id = 2142;
    task.title = "准备太太生日礼物".into();
    task.source = "user".into();
    task.created_at = "2026-09-24 06:43:38".into();
    task.context_text = Some("🎁 准备 Hello Kitty 可爱风".into());
    task.group_key = Some("auto:准备太太生日礼物hello_kitty可爱风".into());
    task.trigger_type = Some("time".into());
    task.trigger_at_unix = Some(1_800_000_000);

    let item = unified_inbox_item_from_task(&task, "zh");
    let intent = item
        .lineage
        .iter()
        .find(|node| node.layer == "intent")
        .expect("intent hop");
    assert_eq!(intent.detail, "任务 2142");
    let origin = item
        .lineage
        .iter()
        .find(|node| node.layer == "origin")
        .expect("origin hop");
    assert_eq!(
        origin.detail,
        "你 · 2026-09-24 06:43:38 · 🎁 准备 Hello Kitty 可爱风"
    );
    let dumped = serde_json::to_string(&item).unwrap();
    assert!(!dumped.contains("group_key="));
    assert!(!dumped.contains("source="));
    assert!(!dumped.contains("created_at="));
    assert!(!dumped.contains("context_text="));
    assert!(!dumped.contains("auto:准备太太生日礼物"));
}

#[test]
fn tracking_note_without_generator_ends_only_this_card() {
    let mut task = test_task_for_projection();
    task.source = "webapp_tracking_draft".into();
    task.execution_mode = "manual".into();

    let item = unified_inbox_item_from_task(&task, "zh");
    let kinds: Vec<_> = item
        .secondary_actions
        .iter()
        .map(|action| action.action_kind.as_str())
        .collect();
    assert_eq!(kinds, vec!["task.complete", "task.snooze_tomorrow"]);
    assert!(
        item.lineage.iter().any(|node| node.layer == "occurrence"
            && node.action_ids == vec!["task.complete".to_string()])
    );
    assert!(
        !item
            .secondary_actions
            .iter()
            .any(|action| action.action_kind == "tracking.cancel")
    );
}

#[test]
fn waiting_knowledge_lineage_follows_source_message() {
    let entry = crate::memory::KnowledgeEntry {
        id: 4,
        chat_id: 9,
        category: "waiting_for".into(),
        priority: "normal".into(),
        status: "active".into(),
        title: "门铃电池".into(),
        body: "等回复".into(),
        tags: vec![],
        related_ids: vec![],
        source_msg_id: Some(99),
        source_summary: Some("记得换门铃电池".into()),
        due_date: Some("2026-09-30".into()),
        waiting_on: Some("供应商".into()),
        superseded_by: None,
        resolved_at: None,
        created_at: "2026-09-01T00:00:00Z".into(),
        updated_at: "2026-09-01T00:00:00Z".into(),
    };
    let item = unified_inbox_item_from_waiting_knowledge(&entry, "zh");
    assert!(
        item.secondary_actions
            .iter()
            .any(|action| action.action_kind == "knowledge.done")
    );
    assert!(
        !item
            .secondary_actions
            .iter()
            .any(|action| action.action_kind == "tracking.cancel")
    );
    assert!(item.lineage.iter().any(|node| {
        node.layer == "occurrence"
            && node.id == "kb_waiting:4"
            && node.detail.contains("waiting_on=供应商")
            && node.action_ids == vec!["knowledge.done".to_string()]
    }));
    assert!(item.lineage.iter().any(|node| {
        node.layer == "origin"
            && node.id == "message:99"
            && node.detail.contains("source_msg_id=99")
            && node.detail.contains("source_summary=记得换门铃电池")
    }));
}

fn waiting_entry_for_projection(tags: Vec<String>) -> crate::memory::KnowledgeEntry {
    crate::memory::KnowledgeEntry {
        id: 64,
        chat_id: 12345678,
        category: "waiting_for".into(),
        priority: "normal".into(),
        status: "active".into(),
        title: "治疗提醒 #36".into(),
        body: "private-waiting-body".into(),
        tags,
        related_ids: vec![],
        source_msg_id: None,
        source_summary: None,
        due_date: None,
        waiting_on: Some("确认预约时间".into()),
        superseded_by: None,
        resolved_at: None,
        created_at: "2026-03-30T20:00:38Z".into(),
        updated_at: "2026-03-30T20:00:38Z".into(),
    }
}

#[test]
fn waiting_parent_tag_uses_only_an_explicit_pointer() {
    use crate::memory::{WaitingParentTag, waiting_parent_tag};

    assert_eq!(waiting_parent_tag(&[]), WaitingParentTag::Absent);
    assert_eq!(
        waiting_parent_tag(&["治疗提醒 #36".into()]),
        WaitingParentTag::Absent
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:36".into()]),
        WaitingParentTag::Task(36)
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:36".into(), "tracking_task:36".into()]),
        WaitingParentTag::Task(36)
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:36".into(), "group_key:loop".into()]),
        WaitingParentTag::Task(36)
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:36".into(), "tracking_task:37".into()]),
        WaitingParentTag::Unusable
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:".into()]),
        WaitingParentTag::Unusable
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:nope".into()]),
        WaitingParentTag::Unusable
    );
    assert_eq!(
        waiting_parent_tag(&["tracking_task:0".into()]),
        WaitingParentTag::Unusable
    );
    assert_eq!(
        waiting_parent_tag(&["group_key:treatment".into()]),
        WaitingParentTag::Group("treatment".into())
    );
    assert_eq!(
        waiting_parent_tag(&["group_key:a".into(), "group_key:b".into()]),
        WaitingParentTag::Unusable
    );
    assert_eq!(
        waiting_parent_tag(&["event_key:door".into()]),
        WaitingParentTag::Event("door".into())
    );
}

#[test]
fn waiting_generator_requires_an_owned_active_loop() {
    let owner = "owner";
    let mut clock = test_task_for_projection();
    clock.id = 36;
    clock.interval_seconds = Some(86_400);
    clock.group_key = Some("treatment-reminder".into());
    clock.execution_mode = "llm_callback".into();
    clock.body = "private-body-do-not-show".into();
    clock.context_text = Some("private-context-do-not-show".into());

    let tagged = waiting_entry_for_projection(vec![crate::memory::tracking_parent_task_tag(36)]);
    let clock_only = [clock.clone()];
    assert_eq!(
        waiting_generator_from_tasks(&tagged, &clock_only, owner).map(|task| task.id),
        Some(36)
    );

    let numbered = waiting_entry_for_projection(vec![]);
    assert!(waiting_generator_from_tasks(&numbered, &[clock.clone()], owner).is_none());

    let mut one_shot = clock.clone();
    one_shot.id = 7;
    one_shot.interval_seconds = None;
    one_shot.group_key = Some("auto-from-title".into());
    one_shot.execution_mode = "notify".into();
    one_shot.trigger_type = Some("time".into());
    one_shot.trigger_at_unix = Some(1_800_000_000);
    let one_shot_entry =
        waiting_entry_for_projection(vec![crate::memory::tracking_parent_task_tag(7)]);
    assert!(waiting_generator_from_tasks(&one_shot_entry, &[one_shot], owner).is_none());

    let mut foreign = clock.clone();
    foreign.user_did = "did:other".into();
    assert!(waiting_generator_from_tasks(&tagged, &[foreign], owner).is_none());

    let mut cancelled = clock.clone();
    cancelled.status = "cancelled".into();
    assert!(waiting_generator_from_tasks(&tagged, &[cancelled], owner).is_none());

    let unusable =
        waiting_entry_for_projection(vec!["tracking_task:36".into(), "tracking_task:99".into()]);
    assert!(waiting_generator_from_tasks(&unusable, &[clock.clone()], owner).is_none());

    let mut other = clock.clone();
    other.id = 37;
    let group_entry = waiting_entry_for_projection(vec!["group_key:treatment-reminder".into()]);
    let ambiguous = [clock.clone(), other];
    assert!(waiting_generator_from_tasks(&group_entry, &ambiguous, owner).is_none());
    assert_eq!(
        waiting_generator_from_tasks(&group_entry, &clock_only, owner).map(|task| task.id),
        Some(36)
    );

    let mut watch = test_task_for_projection();
    watch.id = 8;
    watch.event_key = Some("appointment".into());
    watch.execution_mode = "notify".into();
    let event_entry = waiting_entry_for_projection(vec!["event_key:appointment".into()]);
    let watch_only = [watch.clone()];
    assert_eq!(
        waiting_generator_from_tasks(&event_entry, &watch_only, owner).map(|task| task.id),
        Some(8)
    );
    let mut watch_two = watch.clone();
    watch_two.id = 9;
    let both_watches = [watch, watch_two];
    assert!(waiting_generator_from_tasks(&event_entry, &both_watches, owner).is_none());

    let mut note = test_task_for_projection();
    note.id = 11;
    note.source = "webapp_tracking_draft".into();
    let note_entry =
        waiting_entry_for_projection(vec![crate::memory::tracking_parent_task_tag(11)]);
    assert!(waiting_generator_from_tasks(&note_entry, &[note], owner).is_none());
}

#[test]
fn goal_lineage_names_parent_without_inventing_a_close() {
    let goal = crate::memory::Goal {
        id: 8,
        user_did: "did:test:owner".into(),
        title: "发行更新".into(),
        description: "private".into(),
        status: crate::memory::GoalStatus::Active,
        parent_goal_id: Some(3),
        progress_pct: 25.0,
        milestones: Vec::new(),
        created_at: "2026-05-11T00:00:00Z".into(),
        target_date: None,
        last_progress_at: None,
        source: crate::memory::GoalSource::Explicit,
    };
    let item = unified_inbox_item_from_goal(&goal, "zh");
    assert_eq!(
        item.secondary_actions
            .iter()
            .map(|action| action.action_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["goal.abandon"]
    );
    let parent = item
        .lineage
        .iter()
        .find(|node| node.id == "goal:3")
        .expect("parent goal");
    assert_eq!(parent.layer, "intent");
    assert!(parent.detail.contains("parent_goal_id=3"));
    assert!(parent.action_ids.is_empty());
    assert!(
        item.lineage.iter().any(|node| node.layer == "occurrence"
            && node.action_ids == vec!["goal.abandon".to_string()])
    );
    assert!(
        !item
            .lineage
            .iter()
            .any(|node| node.detail.contains("private"))
    );
}

fn archive_row(id: i64, role: &str, content: &str, created_at: &str) -> crate::memory::L3Entry {
    crate::memory::L3Entry {
        id,
        agent_did: "did:aegis:telegram:1".into(),
        role: role.into(),
        content: content.into(),
        created_at: created_at.into(),
        anchor_kind: None,
        anchor_ref: None,
        latency_ms: None,
    }
}

#[test]
fn triggering_conversation_uses_the_turn_that_created_the_row() {
    let created = chrono::DateTime::parse_from_rfc3339("2026-09-23T04:00:30+00:00")
        .unwrap()
        .timestamp();
    let entries = vec![
        archive_row(1, "user", "周末提醒给门铃充电", "2026-09-23T04:00:00+00:00"),
        archive_row(
            2,
            "assistant",
            "已设置循环跟踪门铃电量。",
            "2026-09-23T04:00:40+00:00",
        ),
    ];
    let nodes = triggering_conversation_nodes(&entries, created, false, "zh");
    assert_eq!(nodes.len(), 2);
    assert!(nodes[0].detail.contains("l3:1"));
    assert!(nodes[0].detail.contains("role=owner"));
    assert!(nodes.iter().all(|node| node.turn_id.is_none()));
    assert!(nodes[0].detail.contains("周末提醒给门铃充电"));
    assert!(nodes[1].detail.contains("l3:2"));
    assert!(nodes[1].detail.contains("role=aegis"));
    assert!(
        nodes
            .iter()
            .all(|node| node.layer == "origin" && node.action_ids.is_empty())
    );
}

#[test]
fn triggering_conversation_does_not_cross_a_scheduler_callback() {
    let created = chrono::DateTime::parse_from_rfc3339("2026-09-23T04:10:00+00:00")
        .unwrap()
        .timestamp();
    let entries = vec![
        archive_row(1, "user", "周末提醒给门铃充电", "2026-09-23T04:00:00+00:00"),
        archive_row(
            2,
            "user",
            "[INTENT_CALLBACK:depth=1] check doorbell",
            "2026-09-23T04:09:50+00:00",
        ),
    ];
    let nodes = triggering_conversation_nodes(&entries, created, false, "zh");
    assert!(nodes.is_empty());
}

#[test]
fn triggering_conversation_ignores_an_older_chat() {
    let created = chrono::DateTime::parse_from_rfc3339("2026-09-23T05:00:00+00:00")
        .unwrap()
        .timestamp();
    let entries = vec![archive_row(
        1,
        "user",
        "周末提醒给门铃充电",
        "2026-09-23T04:00:00+00:00",
    )];
    assert!(triggering_conversation_nodes(&entries, created, false, "zh").is_empty());
}

#[test]
fn exact_turn_conversation_keeps_that_turns_messages() {
    let entries = vec![
        archive_row(7, "user", "把门铃跟踪关掉", "2026-09-23T04:00:00+00:00"),
        archive_row(
            8,
            "assistant",
            "整条跟踪已关闭。",
            "2026-09-23T04:00:20+00:00",
        ),
    ];
    let nodes = triggering_conversation_nodes(&entries, 0, true, "zh");
    assert_eq!(nodes.len(), 2);
    assert!(nodes[0].detail.contains("把门铃跟踪关掉"));
    assert!(nodes[1].detail.contains("整条跟踪已关闭"));
    assert!(nodes.iter().all(|node| node.turn_id.is_none()));
}

#[test]
fn conversation_lineage_carries_the_webapp_turn_for_the_cite() {
    let mut owner = archive_row(7, "user", "把门铃跟踪关掉", "2026-09-23T04:00:00+00:00");
    owner.anchor_kind = Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.into());
    owner.anchor_ref = Some("cite-turn-doorbell".into());
    let mut assistant = archive_row(
        8,
        "assistant",
        "整条跟踪已关闭。",
        "2026-09-23T04:00:20+00:00",
    );
    assistant.anchor_kind = Some(crate::progress_router::WEBAPP_TURN_ANCHOR_KIND.into());
    assistant.anchor_ref = Some("cite-turn-doorbell".into());
    let nodes = triggering_conversation_nodes(&[owner, assistant], 0, true, "zh");
    assert_eq!(nodes[0].id, "l3:7");
    assert_eq!(nodes[0].turn_id.as_deref(), Some("cite-turn-doorbell"));
    assert_eq!(nodes[0].message_role.as_deref(), Some("owner"));
    assert_eq!(nodes[1].id, "l3:8");
    assert_eq!(nodes[1].message_role.as_deref(), Some("aegis"));
    assert!(nodes[0].detail.contains("turn_id=cite-turn-doorbell"));
}

#[test]
fn private_tagged_task_marks_review_before_action() {
    let mut task = test_task_for_projection();
    task.tags = vec!["private".into()];
    task.priority = "high".into();
    let item = unified_inbox_item_from_task(&task, "zh");
    assert_eq!(
        item.next_safe_action_code.as_deref(),
        Some("task_private_review"),
        "a private task must carry a language-independent review code"
    );
    assert!(item.title.contains('私'));
    assert!(!item.summary.to_ascii_lowercase().contains("stay private"));

    let mut confidential = test_task_for_projection();
    confidential.tags = vec!["confidential".into()];
    assert_eq!(
        unified_inbox_item_from_task(&confidential, "en")
            .next_safe_action_code
            .as_deref(),
        Some("task_private_review")
    );

    let ordinary = unified_inbox_item_from_task(&test_task_for_projection(), "zh");
    assert_eq!(
        ordinary.next_safe_action_code.as_deref(),
        Some("task_open_or_act")
    );

    let mut closed = test_task_for_projection();
    closed.tags = vec!["private".into()];
    closed.status = "completed".into();
    assert_eq!(
        unified_inbox_item_from_task(&closed, "zh")
            .next_safe_action_code
            .as_deref(),
        Some("task_closed_receipts"),
        "a closed private task is already done"
    );
}

#[test]
fn private_tagged_task_does_not_expose_a_trigger_transcript() {
    let mut task = test_task_for_projection();
    task.tags = vec!["private".into()];
    task.context_text = Some("周末提醒给门铃充电".into());
    task.interval_seconds = Some(3600);
    task.execution_mode = "auto".into();
    assert!(!task_exposes_trigger_conversation(&task));
    let item = unified_inbox_item_from_task(&task, "zh");
    assert!(!item.lineage.is_empty());
    assert!(item.lineage.iter().all(|node| node.kind != "message"));
    assert!(item.lineage.iter().all(|node| node.turn_id.is_none()));
    assert!(
        item.lineage
            .iter()
            .all(|node| !node.detail.contains("周末提醒给门铃充电"))
    );
    let mut open = test_task_for_projection();
    open.interval_seconds = Some(3600);
    open.execution_mode = "auto".into();
    assert!(task_exposes_trigger_conversation(&open));
}

fn history_row(turn: Option<&str>, timestamp: i64, text: &str) -> VNextChatHistoryMessage {
    VNextChatHistoryMessage {
        role: "owner".into(),
        turn_id: turn.map(str::to_string),
        text: text.into(),
        timestamp,
        severity: "info".into(),
        latency_ms: None,
        repeat_count: None,
        first_timestamp: None,
        alert_group: None,
        work_item_id: None,
        evidence: None,
        unsupported_numbers: None,
        retry_text: None,
    }
}

#[test]
fn chat_history_window_includes_a_turn_older_than_the_newest_page() {
    // 80 rows, page 24: the newest slice starts at timestamp 57. The cited
    // turn sits before that page, with older rows still ahead of its window.
    let messages = || {
        (1..=80)
            .map(|index| {
                let turn = (index == 41).then_some("old-turn");
                history_row(turn, index, &format!("row-{index}"))
            })
            .collect()
    };
    let (newest, newest_more) = select_chat_history_window(messages(), None);
    assert!(newest_more);
    assert!(newest.iter().all(|message| message.timestamp >= 57));
    assert!(newest.iter().all(|message| message.turn_id.is_none()));
    let (focused, focused_more) = select_chat_history_window(messages(), Some("old-turn"));
    assert!(focused_more);
    assert!(
        focused
            .iter()
            .any(|message| message.turn_id.as_deref() == Some("old-turn"))
    );
    assert!(focused.iter().any(|message| message.timestamp < 57));
    let (missing, missing_more) = select_chat_history_window(
        (1..=80)
            .map(|index| history_row(None, index, &format!("row-{index}")))
            .collect(),
        Some("missing-turn"),
    );
    assert!(missing_more);
    assert!(missing.iter().all(|message| message.timestamp >= 57));
}
