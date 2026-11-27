//! Tool edge-case tests — boundary conditions and error handling for orchestrator tools.
//!
//! These tests complement the user_journeys tests by covering error paths,
//! malformed arguments, and edge cases that the LLM can produce. Each test
//! validates that the system degrades gracefully rather than panicking or
//! producing incorrect results.
//!
//! Gap addressed: orchestrator_tools_*.rs modules (~9400 lines) had zero
//! unit/integration tests before this file.

mod common;
mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

// ── recall_memory edge cases ────────────────────────────────────────────────

/// recall_memory with empty query should return a result (not panic)
/// and the orchestrator should continue to the final reply.
#[tokio::test]
async fn tool_recall_memory_empty_query() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "recall_memory", json!({"query": ""})),
        text_response("No relevant memories found."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "remember anything?", None)
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["recall_memory"]);
    assert!(!result.reply.is_empty());
}

/// recall_memory with missing query field should still produce a tool result
/// (empty or error), not crash the orchestrator.
#[tokio::test]
async fn tool_recall_memory_missing_query_field() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "recall_memory", json!({})),
        text_response("I couldn't find anything relevant."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what do you remember?", None)
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["recall_memory"]);
}

// ── get_current_time edge cases ─────────────────────────────────────────────

/// get_current_time with valid timezone returns structured time info.
#[tokio::test]
async fn tool_get_current_time_valid_tz() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "get_current_time", json!({"timezone": "+08:00"})),
        text_response("It's currently afternoon in your timezone."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what time is it?", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["get_current_time"]);
    assert_eq!(result.rounds_used, 2);
}

/// get_current_time with invalid timezone should still return (with UTC fallback).
#[tokio::test]
async fn tool_get_current_time_invalid_tz() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "get_current_time",
            json!({"timezone": "invalid_tz_garbage"}),
        ),
        text_response("It's currently morning UTC."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what time is it?", None)
        .await
        .unwrap();

    // Should not panic — falls back to UTC
    assert_eq!(result.tools_called, vec!["get_current_time"]);
    assert_eq!(result.rounds_used, 2);
}

// ── notify_user edge cases ──────────────────────────────────────────────────

/// notify_user with empty message should not crash; message may be dropped.
#[tokio::test]
async fn tool_notify_user_empty_message() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "notify_user",
            json!({"message": "", "priority": "routine"}),
        ),
        text_response("Done."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "send me a notification", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["notify_user"]);
}

// ── malformed JSON args ─────────────────────────────────────────────────────

/// When LLM sends completely invalid JSON as tool arguments,
/// the orchestrator should produce an error tool result and continue.
#[tokio::test]
async fn tool_malformed_json_args_recovers() {
    // Create a response where tool args are invalid JSON
    let malformed_response = serde_json::json!({
        "id": "resp-1",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_bad",
                    "type": "function",
                    "function": {
                        "name": "recall_memory",
                        "arguments": "this is not valid json at all"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    })
    .to_string();

    let state = Arc::new(MockServerState::new(vec![
        malformed_response,
        text_response("I encountered an error with that tool call."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "search my memories", None)
        .await
        .unwrap();

    // Should recover — 2 rounds (failed tool + text reply)
    assert_eq!(result.rounds_used, 2);
    assert!(!result.reply.is_empty());
}

// ── search_tools edge cases ─────────────────────────────────────────────────

/// search_tools with a query that matches nothing should return empty results.
#[tokio::test]
async fn tool_search_tools_no_match() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "search_tools",
            json!({"query": "nonexistent_tool_xyz_123"}),
        ),
        text_response("No matching tools found."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "find a tool for xyz", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["search_tools"]);
    assert_eq!(result.rounds_used, 2);
}

// ── manage_tasks edge cases ─────────────────────────────────────────────────

/// manage_tasks with missing action should return an error tool result.
#[tokio::test]
async fn tool_manage_tasks_missing_action() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "manage_tasks", json!({"title": "test"})),
        text_response("I need to specify an action for task management."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "add a task", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["manage_tasks"]);
    // Should get error result but not crash
    assert_eq!(result.rounds_used, 2);
}

/// manage_tasks add with valid fields should create a task successfully.
#[tokio::test]
async fn tool_manage_tasks_add_success() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "manage_tasks",
            json!({"action": "add", "title": "Buy groceries", "priority": "high"}),
        ),
        text_response("Task created: Buy groceries"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:user1", "add a task to buy groceries", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["manage_tasks"]);
    assert_eq!(result.rounds_used, 2);

    // Verify task was actually created in the DB
    let tasks = memory.list_tasks(0, Some("did:test:user1"), None, 10).await;
    assert!(
        tasks.iter().any(|t| t.title.contains("Buy groceries")),
        "Task should be persisted in DB"
    );
}

// ── schedule_intent edge cases ──────────────────────────────────────────────

/// schedule_intent with past trigger time should be handled gracefully.
#[tokio::test]
async fn tool_schedule_intent_past_time() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 0,
                "context_text": "Reminder that already passed",
                "execution_mode": "notify"
            }),
        ),
        text_response("Reminder set."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "remind me now", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // Should not panic — delay_seconds=0 is valid (fires immediately on next poll)
    assert_eq!(result.rounds_used, 2);
}

// ── Multiple tools in single round ──────────────────────────────────────────

/// LLM calls two tools in one round (parallel tool calls).
/// Orchestrator should process both and return both results.
#[tokio::test]
async fn tool_parallel_calls_in_single_round() {
    // Build a response with two tool calls in one message
    let parallel_response = serde_json::json!({
        "id": "resp-1",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call_a",
                        "type": "function",
                        "function": {
                            "name": "get_current_time",
                            "arguments": "{\"timezone\":\"+08:00\"}"
                        }
                    },
                    {
                        "id": "call_b",
                        "type": "function",
                        "function": {
                            "name": "recall_memory",
                            "arguments": "{\"query\":\"schedule\"}"
                        }
                    }
                ]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    })
    .to_string();

    let state = Arc::new(MockServerState::new(vec![
        parallel_response,
        text_response("It's 3pm and you have no upcoming events."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what time is it and check my schedule",
            None,
        )
        .await
        .unwrap();

    // Both tools should be called in round 1, then final text in round 2
    assert_eq!(result.rounds_used, 2);
    assert_eq!(
        result.tools_called,
        vec!["get_current_time", "recall_memory"]
    );
}

// ── Unknown tool name ───────────────────────────────────────────────────────

/// When LLM hallucinates a tool name that doesn't exist,
/// the orchestrator should return an error and continue.
#[tokio::test]
async fn tool_unknown_name_recovers() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "totally_fake_tool_that_doesnt_exist",
            json!({"arg": "value"}),
        ),
        text_response("Let me try a different approach."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "do something", None)
        .await
        .unwrap();

    // Should recover — error tool result tells LLM the tool doesn't exist
    assert_eq!(result.rounds_used, 2);
    assert!(!result.reply.is_empty());
}
