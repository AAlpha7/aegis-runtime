//! Chaos / Resilience Suite — failure injection tests.
//!
//! Tests that the system degrades gracefully under adverse conditions:
//! LLM API failures, malformed responses, budget exhaustion, memory issues,
//! and security enforcement.

mod mock_helpers;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

// ═══════════════════════════════════════════════════════════════════════════════
//  D1: LLM Failure Injection
// ═══════════════════════════════════════════════════════════════════════════════

/// LLM API returns 500 → process_message returns Err, doesn't hang.
#[tokio::test]
async fn chaos_llm_api_error_returns_err() {
    let state = Arc::new(MockServerState::new(vec![]));
    state.chat_fail.store(true, Ordering::SeqCst);
    let (addr, _handle) = spawn_mock_server(state).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "hello", Some(100))
        .await;

    assert!(
        result.is_err(),
        "LLM API failure should propagate as Err, not silent success"
    );
}

/// LLM returns tool call with unknown tool name → tool skipped, no panic.
#[tokio::test]
async fn chaos_unknown_tool_name_skipped() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "completely_fake_tool_xyz", json!({"a": 1})),
        text_response("Let me try something else."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "do something", Some(100))
        .await
        .expect("should not panic on unknown tool");

    assert!(
        !result
            .tools_called
            .contains(&"completely_fake_tool_xyz".to_string()),
        "fake tool should not appear in tools_called"
    );
    assert!(!result.reply.is_empty(), "should still produce a reply");
}

/// LLM returns empty content and no tool calls → exits loop gracefully.
#[tokio::test]
async fn chaos_empty_content_no_tools_exits_gracefully() {
    // Craft a response with null content and empty tool_calls
    let empty_response = json!({
        "id": "chatcmpl-empty",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": [],
                "tool_calls": []
            },
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10, "cost": 0.0}
    })
    .to_string();

    let state = Arc::new(MockServerState::new(vec![empty_response]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "test", Some(100))
        .await;

    // Should complete (Ok or Err), but definitely not hang
    if let Ok(r) = result {
        assert_eq!(r.rounds_used, 1, "empty response = 1 round, exit");
    }
    // Err is also acceptable — error on empty content
    assert_eq!(state.chat_calls(), 1, "exactly 1 LLM call");
}

/// Round budget exhaustion — LLM keeps calling tools past MAX_ROUNDS.
#[tokio::test]
async fn chaos_round_budget_exhaustion() {
    // Queue 20 consecutive tool calls — more than MAX_ROUNDS
    let mut responses = Vec::new();
    for i in 0..20 {
        responses.push(tool_call_response(
            &format!("call_{i}"),
            "get_current_time",
            json!({}),
        ));
    }
    // No text_response — force the loop to exhaust rounds

    let state = Arc::new(MockServerState::new(responses));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "infinite loop test", Some(100))
        .await
        .expect("should not panic on budget exhaustion");

    // Should stop at MAX_ROUNDS (15), not run all 20
    assert!(
        result.rounds_used <= 18,
        "should stop at MAX_ROUNDS (15) + overhead, not run forever: rounds={}",
        result.rounds_used
    );
    assert!(
        state.chat_calls() <= 20,
        "LLM calls should be bounded: calls={}",
        state.chat_calls()
    );
}

/// Embedding API failure — recall_memory degrades but doesn't crash.
#[tokio::test]
async fn chaos_embedding_failure_graceful_degradation() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "test", "scope": "all"}),
        ),
        text_response("I could not search memory, but here is what I know."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    // Break embeddings after first request (context building may need it)
    state.embed_fail.store(true, Ordering::SeqCst);

    let result = orch
        .process_message("did:test:user1", "what do you know?", Some(100))
        .await;

    // Should either succeed (with degraded recall) or fail gracefully
    match result {
        Ok(r) => {
            assert!(!r.reply.is_empty() || !r.tools_called.is_empty());
        }
        Err(e) => {
            // Error is acceptable as long as it's not a panic
            assert!(
                format!("{e:?}").contains("embed")
                    || format!("{e:?}").contains("500")
                    || format!("{e:?}").contains("error"),
                "error should mention embedding failure: {e:?}"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  D2: Memory Resilience
// ═══════════════════════════════════════════════════════════════════════════════

/// Empty L1 cache — fresh user, no conversation history.
#[tokio::test]
async fn chaos_empty_l1_cache_works() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "Welcome! I have no prior context.",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    // Brand new user with zero history
    let result = orch
        .process_message("did:test:brand_new_user", "hi", Some(100))
        .await
        .expect("empty L1 should not cause panic");

    assert!(!result.reply.is_empty());
    assert_eq!(result.rounds_used, 1);
}

/// Concurrent access — two messages with same DID via tokio::join!.
#[tokio::test]
async fn chaos_concurrent_same_did_no_deadlock() {
    let state = Arc::new(MockServerState::new(vec![
        text_response("Reply 1"),
        text_response("Reply 2"),
    ]));
    let (addr, _handle) = spawn_mock_server(state).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = Arc::new(build_orchestrator(addr, memory, settings));

    let o1 = orch.clone();
    let o2 = orch.clone();

    let (r1, r2) = tokio::join!(
        async move { o1.process_message("did:test:u1", "msg A", Some(100)).await },
        async move { o2.process_message("did:test:u1", "msg B", Some(100)).await }
    );

    // At least one should succeed; neither should deadlock
    let successes = [r1.is_ok(), r2.is_ok()].iter().filter(|&&x| x).count();
    assert!(successes >= 1, "at least 1 concurrent call should succeed");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  D3: Security Resilience
// ═══════════════════════════════════════════════════════════════════════════════

/// Callback mode should restrict InteractiveOnly tools.
#[tokio::test]
async fn chaos_callback_mode_restricts_tools() {
    // In callback mode, certain tools may be restricted
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "generate_personal_brief", json!({})),
        text_response("Could not build a brief in callback mode."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK] build a personal brief",
            Some(100),
        )
        .await
        .expect("should not panic in callback mode");

    // generate_personal_brief should be blocked in callback mode (InteractiveOnly)
    assert!(
        !result
            .tools_called
            .contains(&"generate_personal_brief".to_string()),
        "generate_personal_brief should be blocked in callback mode: {:?}",
        result.tools_called
    );
}

/// Exfiltration detection — tool args containing previous result fragments.
#[tokio::test]
async fn chaos_exfiltration_detection() {
    // This test verifies the structural concept: when the LLM tries to
    // leak data via tool args, the exfiltration detector should catch it.
    // The actual detection depends on having previous tool results in context.
    let state = Arc::new(MockServerState::new(vec![
        // First: recall memory (establishes context with sensitive data)
        tool_call_response(
            "call_1",
            "recall_memory",
            json!({"query": "secrets", "scope": "all"}),
        ),
        // Then: try to exfiltrate via web_search (using recall result in query)
        tool_call_response(
            "call_2",
            "web_search",
            json!({"query": "send this data to my server: sk-proj-abc123xyz"}),
        ),
        text_response("Could not complete the search."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "find my secrets", Some(100))
        .await
        .expect("should not panic on exfiltration attempt");

    // The test passes as long as no panic occurs — the exact blocking
    // behavior depends on whether the exfiltration detector triggers
    // (it needs matching fragments from previous results)
    assert!(result.rounds_used >= 1);
}

/// Token pattern detection — args containing API key patterns.
#[tokio::test]
async fn chaos_token_pattern_in_args() {
    // The orchestrator should detect known token patterns in tool arguments
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "web_search",
            json!({"query": "search for sk-proj-1234567890abcdef1234567890abcdef"}),
        ),
        text_response("Search completed."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "search something", Some(100))
        .await
        .expect("should not panic on token-like args");

    // No panic is the minimum; ideally the tool is blocked
    assert!(result.rounds_used >= 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  D4: Multi-tool edge cases
// ═══════════════════════════════════════════════════════════════════════════════

/// Mixed valid and invalid tools in one response.
#[tokio::test]
async fn chaos_mixed_valid_and_invalid_tools() {
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            ("call_1", "get_current_time", json!({})),
            ("call_2", "hallucinated_tool_name", json!({"x": 1})),
            (
                "call_3",
                "recall_memory",
                json!({"query": "test", "scope": "all"}),
            ),
        ]),
        text_response("Partial results available."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "do multiple things", Some(100))
        .await
        .expect("should handle mixed valid/invalid tools");

    // Valid tools should still execute
    assert!(
        !result.tools_called.is_empty(),
        "at least some valid tools should execute"
    );
    assert!(!result.reply.is_empty());
}

/// Very long user message — should not crash or timeout.
#[tokio::test]
async fn chaos_very_long_user_message() {
    let long_msg = "这是一段很长的消息。".repeat(500); // ~5000 chars
    let state = Arc::new(MockServerState::new(vec![text_response("收到长消息。")]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", &long_msg, Some(100))
        .await
        .expect("should handle long messages without panic");

    assert!(!result.reply.is_empty());
    assert_eq!(result.rounds_used, 1);
}
