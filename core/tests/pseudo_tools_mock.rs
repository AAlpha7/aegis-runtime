//! Mock-based integration tests for pseudo-tools that were previously
//! uncovered by deterministic tests: manage_reminder, check_api_usage,
//! setup_google, text_to_speech, generate_image,
//! and render_webapp.
//!
//! Tests run with `cargo test` (no `#[ignore]`), enabling cargo-mutants
//! coverage of pseudo-tool handling paths in the orchestrator.

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

/// Pre-store an "Always" scope approval for a HITL-gated tool so the ToolGate
/// allows execution in tests. Without this, tools like `update_setting`,
/// `setup_google` gets intercepted by the
/// gate and return `NeedApproval` — never reaching the actual handler.
async fn pre_approve_tool(memory: &MemoryHandle, tool_name: &str) {
    let approvals = serde_json::json!([{
        "tool_name": tool_name,
        "scope": "Always",
        "approved_at": "2025-01-01T00:00:00Z",
        "approved_by": "did:test:admin"
    }]);
    memory
        .store_skill_state(
            "hitl",
            &format!("tool_approvals:{tool_name}"),
            &approvals.to_string(),
        )
        .await
        .expect("pre-approve tool");
}

// ── manage_reminder deprecation shim tests ──────────────────────────────────

/// manage_reminder now returns deprecation error (tool removed from LLM list).
#[tokio::test]
async fn manage_reminder_list() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "manage_reminder",
            json!({
                "action": "list"
            }),
        ),
        text_response("Using manage_tasks instead."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "show my reminders", Some(100))
        .await
        .unwrap();

    // manage_reminder is still intercepted (returns deprecation hint)
    assert_eq!(result.tools_called, vec!["manage_reminder"]);
    assert_eq!(result.rounds_used, 2);
}

/// manage_reminder cancel returns deprecation error — does not actually cancel.
#[tokio::test]
async fn manage_reminder_cancel() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    // Store a scheduled task (new unified system)
    let now = chrono::Utc::now().timestamp();
    let tid = memory
        .store_task(
            100,
            "did:test:user1",
            "Cancel me",
            "Cancel me",
            "normal",
            &[],
            None,
            "user",
            Some(now + 3600),
            Some("time"),
            None,
            "notify",
            Some("Cancel me"),
            None,
            None,
            None,
            false,
            Some(now + 7200),
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "manage_reminder",
            json!({
                "action": "cancel",
                "reminder_id": tid
            }),
        ),
        text_response("Reminder deprecated — use manage_tasks."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message(
            "did:test:user1",
            &format!("cancel reminder {tid}"),
            Some(100),
        )
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["manage_reminder"]);

    // Task should still be active — manage_reminder no longer actually cancels
    let tasks = memory.list_tasks(100, None, Some("active"), 50).await;
    assert_eq!(
        tasks.len(),
        1,
        "Task should still be active since manage_reminder is deprecated"
    );
}

/// manage_reminder cancel with invalid id returns deprecation error.
#[tokio::test]
async fn manage_reminder_cancel_invalid_id() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "manage_reminder",
            json!({
                "action": "cancel",
                "reminder_id": 0
            }),
        ),
        text_response("Deprecated."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "cancel reminder 0", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["manage_reminder"]);
}

/// manage_reminder unknown action returns deprecation error.
#[tokio::test]
async fn manage_reminder_unknown_action() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "manage_reminder",
            json!({
                "action": "destroy"
            }),
        ),
        text_response("Unknown action."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "destroy all reminders", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["manage_reminder"]);
}

// ── check_api_usage tests ────────────────────────────────────────────────────

/// check_api_usage calls the API usage handler (mock server returns error since
/// we're hitting our mock, not OpenRouter — but the pseudo-tool path is exercised).
#[tokio::test]
async fn check_api_usage_exercises_path() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "check_api_usage",
            json!({
                "action": "summary"
            }),
        ),
        text_response("Here's your API usage."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "check my API usage", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["check_api_usage"]);
}
// ── calendar write tests ─────────────────────────────────────────────────────
//
// These exercise the tool dispatch + HITL pre-approve path: each tool is
// gated `RequireApproval` so without pre-approval the call is intercepted
// before reaching the handler. With pre-approval, dispatch lands in
// `handle_tool_calendar` which then applies its own policy checks (attendee
// policy, CRLF guard, invalid-email, missing fields). The in-module unit
// tests (`orchestrator_tools_calendar::tests`) assert the exact error
// payload structure; these integration tests assert the wiring end-to-end.

/// calendar_create_event with valid self-only args reaches the dispatch.
/// Without Google auth it eventually fails at the network layer, but the
/// `tools_called` vec proves the orchestrator routed the call correctly.
#[tokio::test]
async fn calendar_create_event_dispatch_reaches_handler() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "calendar_create_event",
            json!({
                "summary": "Lunch",
                "start": "2026-05-01T12:00:00+08:00",
                "end": "2026-05-01T13:00:00+08:00"
            }),
        ),
        text_response("Calendar not configured."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "calendar_create_event").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "create a lunch event tomorrow", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["calendar_create_event"]);
}

/// calendar_create_event with non-self attendee + default setting: dispatch
/// reaches the handler (no silent gate-block), and the handler emits an
/// `invite_others_disabled` payload. This is the "silent strip" regression
/// guard from feedback_transparent_limits.md — the tool MUST surface the
/// error, not quietly drop attendees.
#[tokio::test]
async fn calendar_create_event_with_other_attendees_fails_invite_disabled() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "calendar_create_event",
            json!({
                "summary": "Meeting",
                "start": "2026-05-01T09:00:00+08:00",
                "end": "2026-05-01T10:00:00+08:00",
                "attendees": ["friend@other.com"]
            }),
        ),
        text_response("Will ask user to enable invites."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "calendar_create_event").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "create meeting with friend@other.com",
            Some(100),
        )
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["calendar_create_event"]);
}

/// calendar_create_event with non-self attendee + setting on: dispatch
/// reaches the handler and passes the attendee-policy check. (It will still
/// fail at Google auth since the test harness has no OAuth, but that's a
/// different layer; the tools_called vec proves the wiring.)
#[tokio::test]
async fn calendar_create_event_with_other_attendees_passes_policy_when_setting_on() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "calendar_create_event",
            json!({
                "summary": "Meeting",
                "start": "2026-05-01T09:00:00+08:00",
                "end": "2026-05-01T10:00:00+08:00",
                "attendees": ["friend@other.com"]
            }),
        ),
        text_response("Calendar write requires real OAuth."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "calendar_create_event").await;
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set(
            "calendar_invite_others_enabled",
            serde_json::Value::Bool(true),
        )
        .expect("set setting");
    let orch = build_orchestrator(addr, memory, settings);

    // Inviting someone else leaves the machine. Pin Jev below the hold
    // threshold so this test reaches the attendee-policy handler.
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let result = orch
        .process_message(
            "did:test:user1",
            "create meeting with friend@other.com",
            Some(100),
        )
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["calendar_create_event"]);
}

/// calendar_update_event with just event_id and no field updates reaches
/// the handler (dispatch path). The handler itself returns "no fields to
/// update" — asserted by the unit test in the module. Here we just assert
/// the wiring.
#[tokio::test]
async fn calendar_update_event_dispatch_reaches_handler() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "calendar_update_event",
            json!({ "event_id": "abc123", "summary": "New title" }),
        ),
        text_response("ok."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "calendar_update_event").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "rename event abc123", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["calendar_update_event"]);
}

/// calendar_delete_event dispatch path — RequireApproval is honored via
/// pre_approve_tool; without that the call would be blocked at the gate and
/// `tools_called` would be empty (not the 1-element vec we assert).
#[tokio::test]
async fn calendar_delete_event_dispatch_reaches_handler() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "calendar_delete_event",
            json!({ "event_id": "abc123" }),
        ),
        text_response("Event deleted."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "calendar_delete_event").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "delete event abc123", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["calendar_delete_event"]);
}
// ── text_to_speech tests ─────────────────────────────────────────────────────

/// text_to_speech without TTS configured returns error.
#[tokio::test]
async fn text_to_speech_no_tts_configured() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "text_to_speech",
            json!({
                "text": "Hello, world!"
            }),
        ),
        text_response("TTS not configured."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "say hello world", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["text_to_speech"]);
}

// ── generate_image tests ─────────────────────────────────────────────────────

/// generate_image without image gen API key falls through to OpenRouter path.
/// The mock server won't have an images endpoint, so it'll error gracefully.
#[tokio::test]
async fn generate_image_exercises_path() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "generate_image",
            json!({
                "prompt": "A beautiful sunset over the ocean",
                "aspect_ratio": "16:9"
            }),
        ),
        text_response("Image generation attempted."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "generate an image of a sunset", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["generate_image"]);
}

// ── render_webapp tests ──────────────────────────────────────────────────────

/// render_webapp without suspended task manager returns error.
#[tokio::test]
async fn render_webapp_not_configured() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "render_webapp",
            json!({
                "schema": {
                    "type": "form",
                    "title": "Test Form",
                    "fields": []
                }
            }),
        ),
        text_response("WebApp not configured."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "show a form", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["render_webapp"]);
}

// ── Multi-tool call test ─────────────────────────────────────────────────────

/// Two pseudo-tools called in the same round.
#[tokio::test]
async fn two_pseudo_tools_same_round() {
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            ("call_1", "manage_reminder", json!({"action": "list"})),
            ("call_2", "check_api_usage", json!({"action": "summary"})),
        ]),
        text_response("Here are your reminders and API usage."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "show reminders and API usage", Some(100))
        .await
        .unwrap();

    assert!(result.tools_called.contains(&"manage_reminder".to_string()));
    assert!(result.tools_called.contains(&"check_api_usage".to_string()));
}

// ── set_monitoring_intent test ───────────────────────────────────────────────
// NOTE: set_monitoring_intent is now gated by RequireApproval (HITL),
// so the tool is NOT directly executed — it enters approval-pending state.

/// set_monitoring_intent with missing params — HITL intercepts before execution.
#[tokio::test]
async fn set_monitoring_intent_missing_params() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "set_monitoring_intent",
            json!({
                "skill_name": "",
                "description": ""
            }),
        ),
        text_response("Approval required for monitoring intent."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "monitor something", Some(100))
        .await
        .unwrap();

    // HITL gate intercepts — tool is not executed, not blocked, just pending approval
    assert!(
        result.tools_called.is_empty(),
        "set_monitoring_intent requires HITL approval, should not be in tools_called"
    );
    assert!(
        result.tools_blocked.is_empty(),
        "NeedApproval is pending, not blocked"
    );
}

/// set_monitoring_intent with invalid threshold — HITL intercepts before execution.
#[tokio::test]
async fn set_monitoring_intent_invalid_threshold() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "set_monitoring_intent",
            json!({
                "skill_name": "crypto_tracker",
                "description": "Alert when BTC drops",
                "threshold": 1.5
            }),
        ),
        text_response("Approval required for monitoring intent."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "monitor crypto with high threshold",
            Some(100),
        )
        .await
        .unwrap();

    // HITL gate intercepts — tool is not executed, not blocked, just pending approval
    assert!(
        result.tools_called.is_empty(),
        "set_monitoring_intent requires HITL approval, should not be in tools_called"
    );
    assert!(
        result.tools_blocked.is_empty(),
        "NeedApproval is pending, not blocked"
    );
}

/// set_monitoring_intent happy path — HITL intercepts before execution.
#[tokio::test]
async fn set_monitoring_intent_success() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "set_monitoring_intent",
            json!({
                "skill_name": "crypto_tracker",
                "description": "Alert when BTC drops below 90000",
                "threshold": 0.8
            }),
        ),
        text_response("Approval required for monitoring intent."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "alert me when BTC drops below 90k",
            Some(100),
        )
        .await
        .unwrap();

    // HITL gate intercepts — tool is not executed, not blocked, just pending approval
    assert!(
        result.tools_called.is_empty(),
        "set_monitoring_intent requires HITL approval, should not be in tools_called"
    );
    assert!(
        result.tools_blocked.is_empty(),
        "NeedApproval is pending, not blocked"
    );
}

// ── update_setting tests ─────────────────────────────────────────────────────

/// update_setting creates a config proposal (HITL flow).
#[tokio::test]
async fn update_setting_creates_config_proposal() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({
                "key": "mcp_enabled",
                "value": true
            }),
        ),
        text_response("Setting change proposed."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings.clone());

    let result = orch
        .process_message("did:test:user1", "enable MCP tools", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["update_setting"]);
    // Setting requires HITL approval, so it shouldn't be changed yet
    let val = settings.get("mcp_enabled");
    assert_eq!(val, Some(serde_json::Value::Bool(false)));
}

/// update_setting with unknown key returns error.
#[tokio::test]
async fn update_setting_unknown_key_error() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({
                "key": "nonexistent_key_12345",
                "value": true
            }),
        ),
        text_response("Unknown setting."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "enable nonexistent feature", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["update_setting"]);
}

// ── notify_user integration tests ─────────────────────────────────────────

/// notify_user tool call through the real pipeline queues a notification.
#[tokio::test]
async fn notify_user_queues_notification() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "c1",
            "notify_user",
            json!({
                "message": "You have 3 new emails.",
                "pin": false
            }),
        ),
        text_response("Done"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "check my emails", Some(100))
        .await
        .unwrap();

    assert_eq!(result.notifications.len(), 1);
    assert_eq!(result.notifications[0].message, "You have 3 new emails.");
    assert!(!result.notifications[0].pin);
    assert!(result.tools_called.contains(&"notify_user".to_string()));
}

/// notify_user with empty message is rejected — no notification queued.
#[tokio::test]
async fn notify_user_empty_message_returns_error() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "c1",
            "notify_user",
            json!({
                "message": ""
            }),
        ),
        text_response("No notification sent"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "notify me about nothing", Some(100))
        .await
        .unwrap();

    assert!(
        result.notifications.is_empty(),
        "Empty message should not produce a notification"
    );
    assert!(result.tools_called.contains(&"notify_user".to_string()));
}

/// notify_user with pin=true preserves the pin flag through the pipeline.
#[tokio::test]
async fn notify_user_pin_flag_preserved() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "c1",
            "notify_user",
            json!({
                "message": "Critical alert!",
                "pin": true
            }),
        ),
        text_response("Done"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "send critical alert", Some(100))
        .await
        .unwrap();

    assert_eq!(result.notifications.len(), 1);
    assert_eq!(result.notifications[0].message, "Critical alert!");
    assert!(result.notifications[0].pin);
}

/// Callback with no notify_user call produces zero notifications (silent).
#[tokio::test]
async fn callback_no_notify_user_is_silent() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "Internal reasoning — nothing to report.",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_callback_message("did:test:user1", "scheduled check: nothing new", Some(100))
        .await
        .unwrap();

    assert!(
        result.notifications.is_empty(),
        "Callback with no notify_user should be silent"
    );
}
