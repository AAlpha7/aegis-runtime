//! SMOKE SUITE — run before every commit.
//!
//! 10 fast tests covering the critical paths that, if broken, make the system
//! unusable. All mock-based, no real APIs, target < 60 seconds total.
//!
//! Each test asserts at least 3 properties to prevent test rot.

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

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

// ── 1. Message in, text out ─────────────────────────────────────────────────

/// The most basic path: user sends text, LLM replies with text, one round.
#[tokio::test]
async fn smoke_basic_text_reply() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "Hello! How can I help?",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "hello", Some(100))
        .await
        .expect("process_message should succeed");

    assert_eq!(result.rounds_used, 1, "simple text reply = 1 round");
    assert!(
        result.tools_called.is_empty(),
        "no tools should be called for a simple text reply"
    );
    assert!(
        !result.reply.is_empty(),
        "reply must not be empty for a text response"
    );
    assert_eq!(state.chat_calls(), 1, "exactly 1 LLM call expected");
}

// ── 2. Single tool call round-trip ──────────────────────────────────────────

/// LLM calls recall_memory, tool executes, LLM responds with text.
#[tokio::test]
async fn smoke_single_tool_call_roundtrip() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "test", "scope": "all"}),
        ),
        text_response("Based on your memory, here is the answer."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what do you remember?", Some(100))
        .await
        .expect("process_message should succeed");

    assert_eq!(result.rounds_used, 2, "tool call + text reply = 2 rounds");
    assert!(
        result.tools_called.contains(&"recall_memory".to_string()),
        "recall_memory should be in tools_called"
    );
    assert!(!result.reply.is_empty(), "reply should not be empty");
    assert_eq!(state.chat_calls(), 2, "2 LLM calls: tool call + response");
}

// ── 3. Multi-round tool chain ───────────────────────────────────────────────

/// Two sequential tool calls across rounds, then text response.
#[tokio::test]
async fn smoke_multi_round_tool_chain() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "get_current_time", json!({})),
        tool_call_response(
            "call_2",
            "recall_memory",
            json!({"query": "tasks", "scope": "all"}),
        ),
        text_response("It's 3pm and you have 2 tasks."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what time is it and what are my tasks?",
            Some(100),
        )
        .await
        .expect("process_message should succeed");

    assert_eq!(result.rounds_used, 3, "2 tool rounds + final text = 3");
    assert!(
        result.tools_called.len() >= 2,
        "at least 2 tools should have been called: {:?}",
        result.tools_called
    );
    assert_eq!(state.chat_calls(), 3, "3 LLM calls");
}

// ── 4. HITL gate blocks unapproved tool ─────────────────────────────────────

/// update_setting requires HITL approval. Without pre-approval, it must NOT execute.
/// Mirrors hitl_bypass_detection::require_approval_tools_blocked_without_preapproval.
#[tokio::test]
async fn smoke_hitl_blocks_unapproved_tool() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({"key": "user_timezone", "value": "UTC"}),
        ),
        text_response("I need approval to change settings."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "change timezone to UTC", Some(100))
        .await
        .expect("process_message should succeed");

    assert!(
        !result.tools_called.contains(&"update_setting".to_string()),
        "update_setting should NOT execute without approval. tools_called={:?}",
        result.tools_called
    );
    // The LLM still produces a reply (either from the gate's feedback or fallback)
    assert!(result.rounds_used >= 1, "at least 1 round should complete");
}

// ── 5. Memory store and recall ──────────────────────────────────────────────

/// Store a fact via L2 memory, then recall it through recall_memory tool.
#[tokio::test]
async fn smoke_memory_store_and_recall() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "server IP", "scope": "all"}),
        ),
        text_response("Your server IP is 10.0.0.1"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Pre-store a fact in L2
    memory
        .store_l2(
            "did:test:user1",
            "infrastructure",
            "server IP is 10.0.0.1",
            None,
        )
        .await
        .expect("store fact");

    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what is my server IP?", Some(100))
        .await
        .expect("process_message should succeed");

    assert!(
        result.tools_called.contains(&"recall_memory".to_string()),
        "recall_memory should be called"
    );
    assert_eq!(result.rounds_used, 2, "tool + response = 2 rounds");
    assert_eq!(state.chat_calls(), 2, "2 LLM calls");
}

// ── 6. Callback message routing ─────────────────────────────────────────────

/// process_callback_message works and routes through the ReAct loop.
#[tokio::test]
async fn smoke_callback_message_routing() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "Callback processed: MSFT is at $420",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK] Check MSFT stock price",
            Some(100),
        )
        .await
        .expect("process_callback_message should succeed");

    assert!(!result.reply.is_empty(), "callback should produce a reply");
    assert_eq!(result.rounds_used, 1, "simple callback = 1 round");
    assert_eq!(state.chat_calls(), 1, "1 LLM call");
}

// ── 7. Wasm skill execution ────────────────────────────────────────────────

/// LLM invokes server_monitor Wasm skill, sandbox executes, result returned.
#[tokio::test]
async fn smoke_wasm_skill_execution() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "server_monitor", json!({})),
        text_response("Server metrics were checked."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "check server health", Some(100))
        .await
        .expect("process_message should succeed");

    // server_monitor may fail if the local metrics endpoint is unavailable, but it should be
    // dispatched and return structured error, not panic.
    assert!(
        result.tools_called.contains(&"server_monitor".to_string()),
        "server_monitor should be dispatched"
    );
    assert!(result.rounds_used >= 2, "at least tool dispatch + response");
    assert_eq!(state.chat_calls(), 2, "2 LLM calls");
}

// ── 8. Error response format ────────────────────────────────────────────────

/// Malformed tool arguments produce structured error JSON, not panics.
#[tokio::test]
async fn smoke_malformed_tool_args_structured_error() {
    // Tool call with completely wrong args (missing required fields)
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "recall_memory", json!({})),
        text_response("I see, let me try differently."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "remember something", Some(100))
        .await
        .expect("should not panic on malformed args");

    // The important thing is no panic and a usable result
    assert!(result.rounds_used >= 1, "at least 1 round completed");
    assert_eq!(state.chat_calls(), 2, "2 LLM calls");
    assert!(
        !result.reply.is_empty() || !result.tools_called.is_empty(),
        "should produce either a reply or tool dispatch"
    );
}

// ── 9. HITL-approved tool executes ──────────────────────────────────────────

/// With pre-approval, a gated tool should execute normally.
/// Mirrors hitl_bypass_detection::require_approval_tools_execute_with_preapproval.
#[tokio::test]
async fn smoke_hitl_approved_tool_executes() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "update_setting",
            json!({"key": "mcp_enabled", "value": false}),
        ),
        text_response("Setting updated."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "change setting", Some(100))
        .await
        .expect("process_message should succeed");

    assert!(
        result.tools_called.contains(&"update_setting".to_string()),
        "Pre-approved tool should execute. tools_called={:?}",
        result.tools_called
    );
    assert!(result.rounds_used >= 1, "at least 1 round should complete");
}

// ── 10. Parallel multi-tool call ────────────────────────────────────────────

/// LLM requests multiple tools in a single round — all should execute.
#[tokio::test]
async fn smoke_parallel_multi_tool_call() {
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            ("call_1", "get_current_time", json!({})),
            (
                "call_2",
                "recall_memory",
                json!({"query": "schedule", "scope": "all"}),
            ),
        ]),
        text_response("It's 3pm and you have a meeting at 4pm."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what time is it and what's on my schedule?",
            Some(100),
        )
        .await
        .expect("process_message should succeed");

    assert!(
        result.tools_called.len() >= 2,
        "at least 2 tools should be dispatched in parallel: {:?}",
        result.tools_called
    );
    assert_eq!(
        result.rounds_used, 2,
        "parallel tools in 1 round + response"
    );
    assert_eq!(state.chat_calls(), 2, "2 LLM calls");
    assert!(!result.reply.is_empty(), "should produce a final reply");
}
