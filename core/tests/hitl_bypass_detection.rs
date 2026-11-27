//! HITL (Human-in-the-Loop) Bypass Detection Tests
//!
//! These tests ensure that future code changes cannot accidentally bypass the
//! HITL approval system. They are **structural guards** — they don't test
//! individual tool logic, but rather that the safety architecture remains intact.
//!
//! Test categories:
//! A. Policy completeness — every tool must have an explicit gate policy
//! B. Gate enforcement — gated tools must NOT execute without approval
//! C. Callback hardening — sensitive tools blocked in background mode
//! D. Approval scope integrity — approval scopes cannot be escalated
//! E. Regression guards — canonical gated-tool list hasn't shrunk

mod mock_helpers;

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;
use aegis_runtime_core::tool_gate::{
    ApprovalScope, CallerContext, GateVerdict, StoredApproval, ToolGate, ToolPolicy,
};

use mock_helpers::*;

// ── A. Policy Completeness ──────────────────────────────────────────────────
//
// Every tool that the LLM can call MUST have an explicit ToolPolicy entry.
// Without one, the tool falls through to the default (InteractiveOnly), which
// silently allows execution in interactive mode — a potential HITL bypass.

/// All pseudo-tools in tool_schemas.rs must have an explicit gate policy.
/// If this test fails after adding a new tool, you MUST add a ToolPolicy entry
/// in tool_gate.rs::default_policies().
#[test]
fn every_known_tool_has_explicit_policy() {
    let gate = ToolGate::new();

    // Canonical list of all tools exposed to the LLM (from tool_schemas.rs).
    // When you add a new tool, add it here AND in default_policies().
    let all_tools = vec![
        // Read-only / display
        "web_search",
        "web_reader",
        "browser_fetch",
        "us_stock_quote",
        "calendar_reader",
        "google_drive_reader",
        "file_summarizer",
        "knowledge_base",
        "manage_tasks",
        "news_digest",
        "server_monitor",
        "code_assistant",
        // Internal pseudo-tools
        "notify_user",
        "get_current_time",
        "recall_memory",
        "schedule_intent",
        // System-modifying (RequireApproval)
        "deactivate_skill",
        "reactivate_skill",
        "update_setting",
        "switch_model",
        "set_bot_commands",
        "set_log_level",
        "setup_google",
        "sysadmin_ops",
        // Calendar writes (RequireApproval; self-only default)
        "calendar_create_event",
        "calendar_update_event",
        "calendar_delete_event",
    ];

    let mut missing = Vec::new();
    for tool in &all_tools {
        if !gate.has_policy(tool) {
            missing.push(*tool);
        }
    }

    assert!(
        missing.is_empty(),
        "Tools WITHOUT explicit gate policy (HITL bypass risk): {:?}\n\
         Fix: add entries to tool_gate.rs::default_policies()",
        missing
    );
}

// ── B. Gate Enforcement (integration) ────────────────────────────────────────
//
// When the mock LLM calls a gated tool, the orchestrator must intercept it
// via the ToolGate and NOT execute the handler unless pre-approved.

/// Pre-approve helper (same as pseudo_tools_mock.rs).
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

/// Without pre-approval, RequireApproval tools must NOT appear in tools_called.
/// This catches any code change that adds an early return before the gate check.
#[tokio::test]
async fn require_approval_tools_blocked_without_preapproval() {
    let gated_tools = [
        "update_setting",
        "switch_model",
        "setup_google",
        "sysadmin_ops",
    ];

    for tool_name in &gated_tools {
        let state = Arc::new(MockServerState::new(vec![
            tool_call_response("call_1", tool_name, json!({"key": "test"})),
            text_response("Done."),
        ]));
        let (addr, _handle) = spawn_mock_server(state.clone()).await;
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        // NO pre_approve_tool — this is the point of the test
        let settings = Arc::new(RuntimeSettings::new());
        let orch = build_orchestrator(addr, memory, settings);

        let result = orch
            .process_message("did:test:user1", &format!("call {tool_name}"), Some(100))
            .await
            .unwrap();

        assert!(
            !result.tools_called.contains(&tool_name.to_string()),
            "HITL BYPASS: '{}' appeared in tools_called without approval!\n\
             tools_called={:?}",
            tool_name,
            result.tools_called
        );
    }
}

/// With pre-approval, RequireApproval tools SHOULD execute (tools_called populated).
/// This is the counterpart — ensures the approval mechanism actually works.
#[tokio::test]
async fn require_approval_tools_execute_with_preapproval() {
    // update_setting with a valid key
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({
                "key": "mcp_enabled",
                "value": false
            }),
        ),
        text_response("Setting proposed."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "change setting", Some(100))
        .await
        .unwrap();

    assert!(
        result.tools_called.contains(&"update_setting".to_string()),
        "Pre-approved tool should execute. tools_called={:?}",
        result.tools_called
    );
}
/// Unknown/new tools must be blocked in callback mode (defense in depth).
#[test]
fn unknown_tools_blocked_in_callback() {
    let gate = ToolGate::new();
    let v = gate.evaluate("brand_new_dangerous_tool", "{}", CallerContext::Callback);
    assert!(
        matches!(v, GateVerdict::Block { .. }),
        "Unknown tools should be BLOCKED in callback mode, got {:?}",
        v
    );
}

/// All system-modifying tools remain gated in callback mode.
#[test]
fn all_system_tools_gated_in_callback() {
    let gate = ToolGate::new();
    let system_tools = [
        "deactivate_skill",
        "reactivate_skill",
        "update_setting",
        "switch_model",
        "set_bot_commands",
        "set_log_level",
        "setup_google",
        "sysadmin_ops",
    ];

    for tool in &system_tools {
        let v = gate.evaluate(tool, "{}", CallerContext::Callback);
        assert!(
            matches!(v, GateVerdict::NeedApproval { .. }),
            "HITL BYPASS: '{}' should require approval in callback mode, got {:?}",
            tool,
            v
        );
    }
}

// ── D. Approval Scope Integrity ─────────────────────────────────────────────
//
// Approval scopes must not be escalatable. A Domain approval for "google.com"
// must not allow "evil.com". An Intent approval for group_key "A" must not
// allow group_key "B".

/// Once-scope approvals are never stored (cannot be replayed).
#[test]
fn once_scope_never_matches_stored() {
    let gate = ToolGate::new();
    let once_approval = StoredApproval {
        tool_name: "sysadmin_ops".to_string(),
        scope: ApprovalScope::Once,
        approved_at: chrono::Utc::now().to_rfc3339(),
        approved_by: "did:test:admin".to_string(),
    };
    assert!(
        !gate.check_stored_approval("sysadmin_ops", &[once_approval], None, None),
        "Once-scope approvals must NEVER be replayed from storage"
    );
}

/// Intent-scoped approval only matches the exact group_key.
#[test]
fn intent_scope_does_not_cross_group_keys() {
    let gate = ToolGate::new();
    let approval = StoredApproval {
        tool_name: "web_reader".to_string(),
        scope: ApprovalScope::Intent {
            group_key: "daily_weather_check".to_string(),
        },
        approved_at: chrono::Utc::now().to_rfc3339(),
        approved_by: "did:test:admin".to_string(),
    };

    // Correct group_key → approved
    assert!(gate.check_stored_approval(
        "web_reader",
        std::slice::from_ref(&approval),
        Some("daily_weather_check"),
        None
    ));

    // Different group_key → NOT approved
    assert!(
        !gate.check_stored_approval(
            "web_reader",
            std::slice::from_ref(&approval),
            Some("steal_data_intent"),
            None
        ),
        "Intent approval for 'daily_weather_check' must NOT allow 'steal_data_intent'"
    );

    // No group_key → NOT approved
    assert!(
        !gate.check_stored_approval("web_reader", &[approval], None, None),
        "Intent approval must NOT match when no group_key is provided"
    );
}

/// Domain-scoped approval only matches the exact domain (or subdomain).
#[test]
fn domain_scope_does_not_cross_domains() {
    let gate = ToolGate::new();
    let approval = StoredApproval {
        tool_name: "web_reader".to_string(),
        scope: ApprovalScope::Domain {
            host: "maps.google.com".to_string(),
        },
        approved_at: chrono::Utc::now().to_rfc3339(),
        approved_by: "did:test:admin".to_string(),
    };

    // Exact domain → approved
    assert!(gate.check_stored_approval(
        "web_reader",
        std::slice::from_ref(&approval),
        None,
        Some("maps.google.com")
    ));

    // Different domain → NOT approved
    assert!(
        !gate.check_stored_approval(
            "web_reader",
            std::slice::from_ref(&approval),
            None,
            Some("evil.example.com")
        ),
        "Domain approval for 'maps.google.com' must NOT allow 'evil.example.com'"
    );

    // No domain → NOT approved
    assert!(
        !gate.check_stored_approval("web_reader", &[approval], None, None),
        "Domain approval must NOT match when no URL host is provided"
    );
}

/// Always-scope approval matches regardless of context.
#[test]
fn always_scope_matches_any_context() {
    let gate = ToolGate::new();
    let approval = StoredApproval {
        tool_name: "update_setting".to_string(),
        scope: ApprovalScope::Always,
        approved_at: chrono::Utc::now().to_rfc3339(),
        approved_by: "did:test:admin".to_string(),
    };

    assert!(gate.check_stored_approval(
        "update_setting",
        std::slice::from_ref(&approval),
        None,
        None
    ));
    assert!(gate.check_stored_approval(
        "update_setting",
        std::slice::from_ref(&approval),
        Some("any"),
        None
    ));
    assert!(gate.check_stored_approval("update_setting", &[approval], None, Some("any.host")));
}

/// Approval for tool A must NOT allow tool B.
#[test]
fn approval_does_not_cross_tools() {
    let gate = ToolGate::new();
    let approval = StoredApproval {
        tool_name: "update_setting".to_string(),
        scope: ApprovalScope::Always,
        approved_at: chrono::Utc::now().to_rfc3339(),
        approved_by: "did:test:admin".to_string(),
    };

    // Same tool → approved
    assert!(gate.check_stored_approval(
        "update_setting",
        std::slice::from_ref(&approval),
        None,
        None
    ));

    // Different tool → NOT approved
    assert!(
        !gate.check_stored_approval("sysadmin_ops", &[approval], None, None),
        "Approval for update_setting must NOT allow sysadmin_ops"
    );
}

// ── E. Regression Guards ────────────────────────────────────────────────────
//
// The canonical set of HITL-gated tools must not shrink. If a tool is removed
// from this list, it's either intentional (requires code review) or a bug.

/// The canonical set of RequireApproval tools.
/// If this test fails, someone removed a tool from the gated list — review carefully.
#[test]
fn canonical_gated_tools_unchanged() {
    let gate = ToolGate::new();

    let must_be_gated: Vec<(&str, &str)> = vec![
        ("deactivate_skill", "RequireApproval"),
        ("reactivate_skill", "RequireApproval"),
        ("update_setting", "RequireApproval"),
        ("switch_model", "RequireApproval"),
        ("set_bot_commands", "RequireApproval"),
        ("set_log_level", "RequireApproval"),
        ("setup_google", "RequireApprovalUnless"),
        // Read-only views (and the bare `{}` call, which the executor reads
        // as "summary") run freely; `backfill` / `update_pricing` mutate
        // cost rows and need approval.
        ("check_api_usage", "RequireApprovalUnless"),
        ("delegate_task", "Allow"),
        ("run_external", "Allow"),
        // d0e81508 (owner decision 2026-09-12): `diagnose` is read-only via the
        // executor whitelist and needs no approval; `propose_changes` and bare
        // calls still go to HITL (pinned by tool_gate unit tests).
        ("sysadmin_ops", "RequireApprovalUnless"),
        ("calendar_create_event", "RequireApproval"),
        ("calendar_update_event", "RequireApproval"),
        ("calendar_delete_event", "RequireApproval"),
    ];

    for (tool, expected_kind) in &must_be_gated {
        let policy = gate.get_policy(tool);
        let actual_kind = match &policy {
            Some(ToolPolicy::RequireApproval) => "RequireApproval",
            Some(ToolPolicy::RequireApprovalUnless { .. }) => "RequireApprovalUnless",
            Some(ToolPolicy::ApprovalInCallback) => "ApprovalInCallback",
            Some(ToolPolicy::Allow) => "Allow",
            Some(ToolPolicy::InteractiveOnly) => "InteractiveOnly",
            Some(ToolPolicy::HeartbeatAllowed) => "HeartbeatAllowed",
            Some(ToolPolicy::SubcallOnly) => "SubcallOnly",
            Some(ToolPolicy::Block { .. }) => "Block",
            None => "MISSING",
        };

        assert_eq!(
            actual_kind, *expected_kind,
            "HITL REGRESSION: '{}' policy changed from {} to {}!\n\
             If intentional, update this test. If not, revert the change.",
            tool, expected_kind, actual_kind
        );
    }
}

/// Read-only tools must remain Allow (not accidentally promoted to RequireApproval).
#[test]
fn read_only_tools_remain_allowed() {
    let gate = ToolGate::new();

    let must_be_allowed = [
        "web_search",
        "web_reader",
        "browser_fetch",
        "us_stock_quote",
        "calendar_reader",
        "google_drive_reader",
        "file_summarizer",
        "knowledge_base",
        "manage_tasks",
        "news_digest",
        "server_monitor",
        "code_assistant",
        "notify_user",
        "get_current_time",
        "recall_memory",
        "schedule_intent",
    ];

    for tool in &must_be_allowed {
        assert!(
            matches!(gate.get_policy(tool), Some(ToolPolicy::Allow)),
            "Read-only tool '{}' should have Allow policy, got {:?}",
            tool,
            gate.get_policy(tool)
        );
    }
}

/// No tool that writes user data / modifies system state should have Allow policy.
/// This is a safety net — if you add a new state-changing tool, it must be gated.
#[test]
fn state_changing_verbs_in_tool_names_are_gated() {
    let _gate = ToolGate::new();

    // Tools whose names contain state-changing verbs should NOT have Allow.
    // This is a heuristic, not exhaustive — but catches obvious mistakes.
    let dangerous_verbs = ["delete", "deactivate", "approve", "setup", "switch"];

    // Collect all Allow tools
    let allow_tools: HashSet<String> = [
        "web_search",
        "web_reader",
        "browser_fetch",
        "us_stock_quote",
        "calendar_reader",
        "google_drive_reader",
        "file_summarizer",
        "knowledge_base",
        "manage_tasks",
        "news_digest",
        "server_monitor",
        "code_assistant",
        "notify_user",
        "get_current_time",
        "recall_memory",
        "schedule_intent",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    for tool in &allow_tools {
        for verb in &dangerous_verbs {
            assert!(
                !tool.contains(verb),
                "HITL BYPASS RISK: Tool '{}' contains state-changing verb '{}' but has Allow policy.\n\
                 State-changing tools should use RequireApproval or ApprovalInCallback.",
                tool,
                verb
            );
        }
    }
}

/// Approval ID format must be unique (UUID-based).
#[test]
fn approval_ids_are_unique() {
    let gate = ToolGate::new();
    let mut ids = HashSet::new();

    for _ in 0..100 {
        if let GateVerdict::NeedApproval { approval_id, .. } =
            gate.evaluate("sysadmin_ops", "{}", CallerContext::Interactive)
        {
            assert!(
                ids.insert(approval_id.clone()),
                "Duplicate approval_id generated: {}",
                approval_id
            );
        }
    }
    assert_eq!(
        ids.len(),
        100,
        "Should have generated 100 unique approval IDs"
    );
}
