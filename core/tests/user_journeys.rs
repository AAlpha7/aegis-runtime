//! User Journey integration tests — end-to-end verification of critical user flows.
//!
//! These tests validate complete user-facing paths through the system, not just
//! individual components. Each test simulates what a real user would experience
//! via Telegram, using mock LLM/embedding servers.
//!
//! Source: docs/quality-regression-prevention-plan.md (Layer 2)
//!
//! Uses the shared mock infrastructure from mock_helpers.rs.

mod common;
mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::mcp_client;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

/// Pre-store an "Always" scope approval for a HITL-gated tool.
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

// ── Journey Tests ───────────────────────────────────────────────────────────

/// Journey: Multi-round ReAct tool chain
///
/// User asks a question → LLM calls tool A → sees result → calls tool B →
/// sees result → generates final text reply.
///
/// Validates: orchestrator correctly chains multiple tool calls across rounds,
/// passing state between rounds and accumulating tools_called.
///
/// Critical path: "多轮 ReAct 工具链" in critical-paths.yaml
#[tokio::test]
async fn journey_multi_round_react_tool_chain() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory to search for server info
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "server configuration"}),
        ),
        // Round 2: LLM calls recall_memory again for deployment info
        tool_call_response(
            "call_2",
            "recall_memory",
            json!({"query": "deployment process"}),
        ),
        // Round 3: LLM synthesizes results into final reply
        text_response("Based on your server config and deployment info, here's a summary."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "tell me about my server setup and how to deploy",
            None,
        )
        .await
        .unwrap();

    // Verify: 3 LLM rounds (2 tool calls + 1 final text)
    assert_eq!(result.rounds_used, 3);
    // Verify: both tool calls recorded
    assert_eq!(result.tools_called, vec!["recall_memory", "recall_memory"]);
    // Verify: final reply is the synthesized text
    assert!(result.reply.contains("summary"));
    // Verify: 3 LLM API calls were made
    assert_eq!(state.chat_calls(), 3);
    // Verify: no tools were blocked
    assert!(result.tools_blocked.is_empty());
}

/// Journey: Recall from L2 memory
///
/// Pre-store facts in L2 memory → user asks to recall → LLM calls recall_memory →
/// orchestrator retrieves stored facts → LLM replies with relevant information.
///
/// Validates: the recall_memory pseudo-tool pipeline including L2 vector search
/// and result formatting.
///
/// Note: This tests the recall half of the "记忆学习与召回" critical path.
/// The write half is exercised by the L2 store tests in `core/src/memory`.
///
/// Critical path: "记忆学习与召回" in critical-paths.yaml
#[tokio::test]
async fn journey_learn_and_recall() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "favorite programming language"}),
        ),
        // Round 2: LLM replies based on recalled facts
        text_response("Based on my memory, your favorite programming language is Rust."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Pre-store a fact in L2 memory with embedding for vector search
    let fact_embedding =
        deterministic_embedding("favorite programming language is Rust", EMBED_DIMS);
    memory
        .store_l2(
            "did:test:user1",
            "learned:preference",
            "User's favorite programming language is Rust",
            Some(&fact_embedding),
        )
        .await
        .unwrap();

    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what's my favorite programming language?",
            None,
        )
        .await
        .unwrap();

    // Verify: 2 rounds (tool call + final text)
    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["recall_memory"]);
    assert!(result.reply.contains("Rust"));

    // Verify: the stored fact is still in L2 (not consumed by recall)
    let stored = memory.get_all_l2("did:test:user1").await.unwrap();
    assert_eq!(stored.len(), 1);
}

/// Journey: Recurring callback execution
///
/// User schedules a recurring task → LLM calls schedule_intent → intent persisted →
/// simulate callback trigger → process_callback_message runs full ReAct loop →
/// callback uses tools and produces a reply.
///
/// Validates: the complete DeferredIntent lifecycle from creation through
/// callback execution with tool calling.
///
/// Critical path: "定时任务回调执行" in critical-paths.yaml
#[tokio::test]
async fn journey_recurring_callback_execution() {
    // Phase 1: User schedules an intent
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls schedule_intent
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 120,
                "execution_mode": "llm_callback",
                "context_text": "Check the Bitcoin price and report"
            }),
        ),
        // Round 2: LLM confirms scheduling
        text_response("I've scheduled a Bitcoin price check in 2 minutes."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings.clone());

    let result = orch
        .process_message(
            "did:test:user1",
            "in 2 minutes check the btc price",
            Some(12345),
        )
        .await
        .unwrap();

    // Verify: intent was created via schedule_intent
    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["schedule_intent"]);

    // Verify: intent persisted in database
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].trigger_type, "time");
    assert_eq!(intents[0].execution_mode, "llm_callback");
    assert!(intents[0].context_text.contains("Bitcoin"));
    assert_eq!(intents[0].chat_id, 12345);
    assert_eq!(intents[0].status, "pending");

    // Phase 2: Simulate callback trigger (what time_poller would do)
    // Push new responses for the callback ReAct loop
    state.push_responses(vec![
        tool_call_response(
            "cb_call_1",
            "recall_memory",
            json!({"query": "bitcoin price tracking"}),
        ),
        text_response("Bitcoin is currently at $96,000. Price has been stable."),
    ]);

    // Build a fresh orchestrator for the callback (simulates callback context)
    let orch2 = build_orchestrator(addr, memory.clone(), settings);

    let callback_result = orch2
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK:depth=0] Check the Bitcoin price and report",
            Some(12345),
        )
        .await
        .unwrap();

    // Verify: callback executed a full ReAct loop with tool call
    assert_eq!(callback_result.rounds_used, 2);
    assert_eq!(callback_result.tools_called, vec!["recall_memory"]);
    assert!(callback_result.reply.contains("Bitcoin"));
    assert!(callback_result.reply.contains("96,000"));
}

// ── Round 8: Remaining 7 Journey Tests ─────────────────────────────────────

/// Journey: HITL approval flow
///
/// Phase 1: LLM calls a hallucinated tool → blocked, appears in tools_blocked.
/// Phase 2: LLM calls update_setting (gated pseudo-tool) with pre-stored approval
///          → tool executes, creates HITL proposal, appears in tools_called.
///
/// Validates: ToolGate blocks unknown tools, allows pre-approved gated tools.
///
/// Critical path: "HITL 审批流程" in critical-paths.yaml
#[tokio::test]
async fn journey_hitl_approval_flow() {
    // Phase 1: Unknown tool should be blocked
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "dangerous_system_exec",
            json!({"cmd": "rm -rf /"}),
        ),
        text_response("I cannot execute system commands."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings.clone());

    let result = orch
        .process_message("did:test:user1", "run a command", None)
        .await
        .unwrap();

    // Verify: unknown tool blocked
    assert!(
        result
            .tools_blocked
            .contains(&"dangerous_system_exec".to_string()),
        "dangerous_system_exec should be blocked, got: {:?}",
        result.tools_blocked
    );
    assert!(
        !result
            .tools_called
            .contains(&"dangerous_system_exec".to_string())
    );

    // Phase 2: Gated tool with pre-stored approval should execute
    pre_approve_tool(&memory, "update_setting").await;
    settings.register_skill("news_digest", "news digest", true);

    state.push_responses(vec![
        tool_call_response(
            "call_2",
            "update_setting",
            json!({"key": "skill_news_digest", "value": false}),
        ),
        text_response("I've submitted the change for your approval."),
    ]);

    let orch2 = build_orchestrator(addr, memory.clone(), settings);

    let result2 = orch2
        .process_message("did:test:user1", "disable news digest", None)
        .await
        .unwrap();

    // Verify: update_setting executed (HITL proposal created)
    assert!(
        result2.tools_called.contains(&"update_setting".to_string()),
        "update_setting should be in tools_called with approval, got: {:?}",
        result2.tools_called
    );
    assert!(result2.tools_blocked.is_empty());
}

/// Journey: Google OAuth setup initiation
///
/// LLM calls setup_google pseudo-tool → tool returns gate-pending status
/// (requires user to click OAuth link) → ReAct loop terminates with the
/// gate-pending result as the reply.
#[tokio::test]
async fn journey_google_oauth_setup() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "setup_google", json!({})),
        text_response("To connect Google, please follow the OAuth link I've provided."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "connect my google calendar", Some(12345))
        .await
        .unwrap();

    assert!(result.rounds_used >= 1);
    assert!(
        !result.reply.is_empty(),
        "Reply should contain setup info, got empty"
    );
}

/// Journey: Calendar tool execution through WASM sandbox
///
/// LLM calls calendar_reader WASM skill → sandbox executes → returns result
/// (error about missing credentials, but the execution path is validated).
///
/// Validates: WASM skill loading + execution through orchestrator for calendar ops.
///
/// Critical path: "日历事件管理" in critical-paths.yaml
#[tokio::test]
async fn journey_calendar_tool_call() {
    let state = Arc::new(MockServerState::new(vec![
        // LLM calls calendar_reader WASM skill
        tool_call_response(
            "call_1",
            "calendar_reader",
            json!({"action": "list_events", "time_range": "today"}),
        ),
        // LLM responds based on calendar result (likely an error about no credentials)
        text_response("I tried to check your calendar but Google credentials are not configured."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    // Load calendar_reader WASM skill into the sandbox
    let orch = build_orchestrator_with_skills(addr, memory, settings, &["calendar_reader"]);

    let result = orch
        .process_message("did:test:user1", "what's on my calendar today?", None)
        .await
        .unwrap();

    // Verify: calendar_reader was executed (in WASM sandbox)
    assert!(
        result.tools_called.contains(&"calendar_reader".to_string()),
        "calendar_reader should be in tools_called, got: {:?}",
        result.tools_called
    );
    assert!(result.tools_blocked.is_empty());
    assert!(!result.reply.is_empty());
}

/// Journey: MCP tool routing
///
/// Validates that the MCP namespaced tool pattern (`mcp__server__tool`) is
/// correctly recognized by the MCP routing infrastructure.
///
/// Note: Full MCP tool execution requires a running MCP server process.
/// This test verifies the parsing/routing layer, not actual execution.
///
/// Critical path: "MCP 工具调用" in critical-paths.yaml
#[tokio::test]
async fn journey_mcp_tool_routing() {
    // Test MCP tool name parsing
    let parsed = mcp_client::parse_namespaced_tool("mcp__github__create_issue");
    assert!(parsed.is_some(), "Should parse mcp__github__create_issue");
    let (server, tool) = parsed.unwrap();
    assert_eq!(server, "github");
    assert_eq!(tool, "create_issue");

    // Verify mcp tool detection
    assert!(mcp_client::is_mcp_tool("mcp__github__create_issue"));
    assert!(!mcp_client::is_mcp_tool("recall_memory"));
    assert!(!mcp_client::is_mcp_tool("mcp_incomplete"));

    // Test that calling MCP tool without server returns error (graceful degradation)
    let mgr = mcp_client::McpClientManager::new();
    let result = mgr
        .call_tool("mcp__github__create_issue", json!({"title": "test"}))
        .await;
    assert!(result.is_err(), "Should fail without connected server");
}
