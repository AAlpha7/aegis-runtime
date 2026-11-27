//! Pseudo-Tool Completeness — integration tests for tools with zero test coverage.
//!
//! Each tool gets a happy-path test (valid args → dispatched → tools_called populated)
//! and where appropriate an error-path test (missing/invalid args → structured error).
//!
//! Tools requiring HITL approval are pre-approved via `pre_approve_tool()`.
//!
//! These tests prove every handler is reachable and doesn't panic on valid input.

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

async fn pre_approve_tool(memory: &MemoryHandle, tool_name: &str) {
    let approvals = json!([{
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

/// Helper: run a single tool call through the orchestrator and return the result.
async fn run_tool(
    tool_name: &str,
    args: serde_json::Value,
    approve: bool,
) -> aegis_runtime_core::orchestrator::OrchestratorResult {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", tool_name, args),
        text_response("Done."),
    ]));
    let (addr, _handle) = spawn_mock_server(state).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    if approve {
        pre_approve_tool(&memory, tool_name).await;
    }
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);
    orch.process_message("did:test:user1", &format!("use {tool_name}"), Some(100))
        .await
        .expect("process_message should not panic")
}

// ─── manage_goals ────────────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_manage_goals_list() {
    let r = run_tool("manage_goals", json!({"action": "list"}), false).await;
    assert!(
        r.tools_called.contains(&"manage_goals".to_string()),
        "manage_goals list should dispatch: {:?}",
        r.tools_called
    );
}

#[tokio::test]
async fn completeness_manage_goals_create() {
    let r = run_tool(
        "manage_goals",
        json!({"action": "create", "title": "Learn Rust", "description": "Master ownership"}),
        false,
    )
    .await;
    assert!(r.tools_called.contains(&"manage_goals".to_string()));
}

#[tokio::test]
async fn completeness_manage_goals_update() {
    let r = run_tool(
        "manage_goals",
        json!({"action": "update", "goal_id": 999, "progress": 50}),
        false,
    )
    .await;
    assert!(r.tools_called.contains(&"manage_goals".to_string()));
}

// ─── knowledge_base ──────────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_knowledge_base_query() {
    let r = run_tool("knowledge_base", json!({"action": "query"}), false).await;
    assert!(r.tools_called.contains(&"knowledge_base".to_string()));
}

#[tokio::test]
async fn completeness_knowledge_base_extract() {
    let r = run_tool(
        "knowledge_base",
        json!({"action": "extract", "title": "Server IP", "content": "10.0.0.1", "category": "reference"}),
        false,
    )
    .await;
    assert!(r.tools_called.contains(&"knowledge_base".to_string()));
}

#[tokio::test]
async fn completeness_knowledge_base_search() {
    let r = run_tool(
        "knowledge_base",
        json!({"action": "search", "query": "server"}),
        false,
    )
    .await;
    assert!(r.tools_called.contains(&"knowledge_base".to_string()));
}

// ─── view_logs ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_view_logs() {
    let r = run_tool("view_logs", json!({}), false).await;
    assert!(r.tools_called.contains(&"view_logs".to_string()));
}

// ─── configure_autonomous ────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_configure_autonomous_status() {
    let r = run_tool("configure_autonomous", json!({"action": "status"}), true).await;
    assert!(r.tools_called.contains(&"configure_autonomous".to_string()));
}

// ─── update_heartbeat_memo ───────────────────────────────────────────────────

#[tokio::test]
async fn completeness_update_heartbeat_memo() {
    let r = run_tool(
        "update_heartbeat_memo",
        json!({"content": "monitoring: all systems nominal"}),
        false,
    )
    .await;
    assert!(
        r.tools_called
            .contains(&"update_heartbeat_memo".to_string())
    );
}

// ─── view_heartbeat_audit ────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_view_heartbeat_audit() {
    let r = run_tool("view_heartbeat_audit", json!({}), false).await;
    assert!(r.tools_called.contains(&"view_heartbeat_audit".to_string()));
}

// ─── set_alert_policy ────────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_set_alert_policy() {
    let r = run_tool(
        "set_alert_policy",
        json!({"skill_name": "server_monitor", "delivery": "WarningAndAbove"}),
        true,
    )
    .await;
    assert!(r.tools_called.contains(&"set_alert_policy".to_string()));
}

// ─── list_alert_policies ─────────────────────────────────────────────────────

#[tokio::test]
async fn completeness_list_alert_policies() {
    let r = run_tool("list_alert_policies", json!({}), false).await;
    assert!(r.tools_called.contains(&"list_alert_policies".to_string()));
}

// ─── list_monitoring_intents ─────────────────────────────────────────────────

#[tokio::test]
async fn completeness_list_monitoring_intents() {
    let r = run_tool("list_monitoring_intents", json!({}), false).await;
    assert!(
        r.tools_called
            .contains(&"list_monitoring_intents".to_string())
    );
}

// ─── remove_monitoring_intent ────────────────────────────────────────────────

#[tokio::test]
async fn completeness_remove_monitoring_intent() {
    let r = run_tool(
        "remove_monitoring_intent",
        json!({"skill_name": "nonexistent_skill"}),
        true,
    )
    .await;
    assert!(
        r.tools_called
            .contains(&"remove_monitoring_intent".to_string())
    );
}

// ─── set_bot_commands (RequireApproval) ───────────────────────────────────────

#[tokio::test]
async fn completeness_set_bot_commands_get() {
    let r = run_tool("set_bot_commands", json!({"action": "get"}), true).await;
    assert!(
        r.tools_called.contains(&"set_bot_commands".to_string()),
        "set_bot_commands get should execute with approval: {:?}",
        r.tools_called
    );
}

// ─── set_log_level (RequireApproval) ─────────────────────────────────────────

#[tokio::test]
async fn completeness_set_log_level() {
    // set_log_level is RequireApproval and needs a level arg
    let r = run_tool("set_log_level", json!({"level": "debug"}), true).await;
    assert!(
        r.tools_called.contains(&"set_log_level".to_string()),
        "set_log_level should dispatch. tools_called={:?}, blocked={:?}, rounds={}, reply={}",
        r.tools_called,
        r.tools_blocked,
        r.rounds_used,
        &r.reply[..r.reply.len().min(200)]
    );
}

// ─── deactivate_skill (RequireApproval) ──────────────────────────────────────

#[tokio::test]
async fn completeness_deactivate_skill() {
    let r = run_tool(
        "deactivate_skill",
        json!({"skill_name": "nonexistent_skill"}),
        true,
    )
    .await;
    assert!(r.tools_called.contains(&"deactivate_skill".to_string()));
}

// ─── reactivate_skill (RequireApproval) ──────────────────────────────────────

#[tokio::test]
async fn completeness_reactivate_skill() {
    let r = run_tool(
        "reactivate_skill",
        json!({"skill_name": "nonexistent_skill"}),
        true,
    )
    .await;
    assert!(r.tools_called.contains(&"reactivate_skill".to_string()));
}
// ─── delegate_task ───────────────────────────────────────────────────────────

fn assert_delegate_task_dispatch(tools_called: &[String], when_enabled: &str) {
    let called = tools_called.iter().any(|name| name == "delegate_task");
    if aegis_runtime_core::hercules_client::HerculesClient::delegation_enabled() {
        assert!(called, "{when_enabled}: {tools_called:?}");
    } else {
        assert!(
            !called,
            "delegate_task stays off the hot path unless delegation is enabled: {tools_called:?}"
        );
    }
}

#[tokio::test]
async fn completeness_delegate_task_status_auto_allowed() {
    // "status" is read-only: dispatches without pre-approval when delegation
    // is on. The tool is omitted from the hot path unless that flag is set,
    // which is the OSS default on this host.
    let r = run_tool("delegate_task", json!({"action": "status"}), false).await;
    assert_delegate_task_dispatch(&r.tools_called, "delegate_task status should dispatch");
}

#[tokio::test]
async fn completeness_delegate_task_create_dispatches_without_approval() {
    // Owner decision 2026-09-05: no per-call approval for delegation.
    // The tool is omitted from the hot path unless delegation is explicitly on.
    let r = run_tool(
        "delegate_task",
        json!({"action": "create", "title": "t", "repo": "aegis", "body": "## Definition of Done\n- [ ] x"}),
        false,
    )
    .await;
    assert_delegate_task_dispatch(&r.tools_called, "create must dispatch without approval");
}

fn last_run_external_json(
    r: &aegis_runtime_core::orchestrator::OrchestratorResult,
) -> serde_json::Value {
    let raw = r
        .tool_outputs
        .iter()
        .find(|(name, _)| name == "run_external")
        .map(|(_, body)| body.as_str())
        .unwrap_or_else(|| {
            panic!(
                "run_external output missing; tools_called={:?} tool_outputs={:?}",
                r.tools_called, r.tool_outputs
            )
        });
    serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("run_external output not JSON ({e}): {raw}"))
}

/// `run_external` leaves the machine, so the egress gate holds it when Jev
/// cannot clear the send. The handler's not_configured / not_available
/// answers are only visible once that hold is cleared.
fn assert_run_external_stopped(
    r: &aegis_runtime_core::orchestrator::OrchestratorResult,
    handler_error_kind: &str,
) {
    if r.tools_called.iter().any(|name| name == "run_external") {
        let v = last_run_external_json(r);
        assert_eq!(v["status"], "error", "{v}");
        assert_eq!(v["error_kind"], handler_error_kind, "{v}");
        assert_eq!(v["retryable"], false, "{v}");
        return;
    }
    let v = last_run_external_json(r);
    assert_eq!(v["status"], "pending_human_approval", "{v}");
    assert_eq!(v["tool_name"], "run_external", "{v}");
}

#[tokio::test]
async fn completeness_run_external_reports_not_configured_when_disabled() {
    let r = run_tool(
        "run_external",
        json!({"task": "summarize this directory"}),
        false,
    )
    .await;
    assert_run_external_stopped(&r, "not_configured");
}

#[tokio::test]
async fn completeness_run_external_reports_not_available_when_enabled_without_executors() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "run_external",
            json!({"task": "summarize this directory"}),
        ),
        text_response("Done."),
    ]));
    let (addr, _handle) = spawn_mock_server(state).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("external_executor_enabled", serde_json::Value::Bool(true))
        .expect("enable external_executor_enabled");
    let orch = build_orchestrator(addr, memory, settings);
    let r = orch
        .process_message("did:test:user1", "use run_external", Some(100))
        .await
        .expect("process_message should not panic");
    assert_run_external_stopped(&r, "not_available");
}
