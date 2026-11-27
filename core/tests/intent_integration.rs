//! End-to-end integration tests for the Deferred Intent pseudo-tools.
//!
//! Uses the same mock LLM server pattern as `react_loop_integration.rs`:
//! a local axum server mimics `/v1/chat/completions` and `/v1/embeddings`,
//! with pre-configured response sequences to simulate the LLM calling
//! `schedule_intent`, `list_intents`, and `cancel_intent`.
//!
//! Tests verify the full orchestrator pipeline: tool interception, memory
//! persistence, rate limiting, anti-recursion, and DID isolation.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

mod common;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::intent;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::orchestrator::Orchestrator;
use aegis_runtime_core::settings::RuntimeSettings;

// ── Mock server (same pattern as react_loop_integration.rs) ─────────────────

struct MockServerState {
    chat_responses: Mutex<Vec<String>>,
    chat_call_count: AtomicUsize,
    embed_call_count: AtomicUsize,
}

impl MockServerState {
    fn new(responses: Vec<String>) -> Self {
        Self {
            chat_responses: Mutex::new(responses),
            chat_call_count: AtomicUsize::new(0),
            embed_call_count: AtomicUsize::new(0),
        }
    }
}

#[derive(Deserialize)]
struct MockEmbedRequest {
    #[allow(dead_code)]
    input: String,
    #[allow(dead_code)]
    model: Option<String>,
    #[allow(dead_code)]
    dimensions: Option<u32>,
}

const EMBED_DIMS: usize = 256;

fn fallback_text_response() -> String {
    json!({
        "id": "chatcmpl-fallback",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "(fallback)",
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            },
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

async fn handle_chat(
    State(state): State<Arc<MockServerState>>,
    _body: String,
) -> axum::response::Response {
    state.chat_call_count.fetch_add(1, Ordering::SeqCst);
    let body = {
        let mut queue = state.chat_responses.lock().unwrap();
        if queue.is_empty() {
            fallback_text_response()
        } else {
            queue.remove(0)
        }
    };
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

async fn handle_embed(
    State(state): State<Arc<MockServerState>>,
    Json(req): Json<MockEmbedRequest>,
) -> axum::response::Response {
    state.embed_call_count.fetch_add(1, Ordering::SeqCst);
    let dims = req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize;
    let embedding: Vec<f32> = (0..dims)
        .map(|i| (i as f32 / dims as f32) * 2.0 - 1.0)
        .collect();
    let body = json!({
        "id": "gen-emb-mock",
        "provider": "Mock",
        "object": "list",
        "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
        "model": "mock-embed-model",
        "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.0}
    });
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

async fn spawn_mock_server(state: Arc<MockServerState>) -> (SocketAddr, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .route("/v1/embeddings", post(handle_embed))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

fn build_orchestrator(
    addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
) -> Orchestrator {
    let api_base = format!("http://{}/v1", addr);
    let embedder =
        EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32);
    let sandbox = Arc::new(common::shared_sandbox());
    let identity = Arc::new(AgentIdentity::generate());
    let mut orch = Orchestrator::new(
        "mock-key",
        &api_base,
        "mock-model",
        sandbox,
        identity,
        memory,
        embedder,
        settings,
    );
    orch.set_system_prompt("You are a test assistant.");
    orch
}

// ── Response builders ───────────────────────────────────────────────────────

fn text_response(content: &str) -> String {
    json!({
        "id": "chatcmpl-text",
        "provider": "Mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            },
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

fn tool_call_response(call_id: &str, tool_name: &str, args: Value) -> String {
    json!({
        "id": "chatcmpl-tool",
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
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args.to_string()
                    }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "completed"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 10,
            "total_tokens": 20,
            "prompt_tokens_details": {},
            "completion_tokens_details": {},
            "cost_details": {}
        }
    })
    .to_string()
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// schedule_intent time trigger → stored in DB, returned intent_id.
#[tokio::test]
async fn schedule_time_intent_via_orchestrator() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 120,
                "execution_mode": "notify",
                "context_text": "Check the bitcoin price"
            }),
        ),
        text_response("I've scheduled a reminder to check the bitcoin price in 2 minutes."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "in 2 minutes check btc price",
            Some(12345),
        )
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["schedule_intent"]);

    // Verify intent was persisted
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].trigger_type, "time");
    assert_eq!(intents[0].execution_mode, "notify");
    assert_eq!(intents[0].context_text, "Check the bitcoin price");
    assert_eq!(intents[0].chat_id, 12345);
    assert_eq!(intents[0].status, "pending");
}

/// schedule_intent event trigger → refused, nothing stored.
///
/// Since 382a9db6 (2026-09-12) no event source exists (the mail client was
/// the only producer), so an event trigger of any key is answered with a
/// tool error instead of an intent that would sit "active" forever.
#[tokio::test]
async fn schedule_event_intent_is_refused_without_a_producer() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "event",
                "event_key": "email:new:work",
                "execution_mode": "llm_callback",
                "context_text": "Skill proposal #42 is ready for final approval. Present it to the user."
            }),
        ),
        text_response("I'll notify you when the skill is ready for approval."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "let me know when skill 42 is ready",
            Some(100),
        )
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);

    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(
        intents.is_empty(),
        "an event trigger without a producer must not be stored, got {intents:?}"
    );
}

/// list_intents returns stored intents as JSON.
#[tokio::test]
async fn list_intents_returns_pending() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Pre-populate 2 intents
    memory
        .store_deferred_intent(
            "did:test:user1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "first",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    memory
        .store_deferred_intent(
            "did:test:user1",
            100,
            "event",
            None,
            Some("email:new:other"),
            "llm_callback",
            "second",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "list_intents", json!({})),
        text_response("You have 2 active intents."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what intents are active?", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["list_intents"]);
    assert_eq!(result.rounds_used, 2);
}

/// cancel_intent removes a pending intent.
#[tokio::test]
async fn cancel_intent_removes_from_db() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    let id = memory
        .store_deferred_intent(
            "did:test:user1",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "cancel me",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "cancel_intent", json!({"intent_id": id})),
        text_response(&format!("Intent #{id} has been cancelled.")),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:user1", &format!("cancel intent {id}"), Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["cancel_intent"]);

    // Verify intent was cancelled (no longer in pending list)
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(intents.is_empty());
}

/// cancel_intent rejects wrong DID (DID isolation).
#[tokio::test]
async fn cancel_intent_did_isolation() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    let id = memory
        .store_deferred_intent(
            "did:test:alice",
            100,
            "time",
            Some(now + 60),
            None,
            "notify",
            "alice's intent",
            now + 3600,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Bob tries to cancel Alice's intent
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "cancel_intent", json!({"intent_id": id})),
        text_response("That intent was not found."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:bob", &format!("cancel intent {id}"), Some(200))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["cancel_intent"]);

    // Alice's intent should still be pending
    let intents = memory.list_user_intents("did:test:alice").await.unwrap();
    assert_eq!(intents.len(), 1);
}

/// Active intent rate limit (MAX_ACTIVE_INTENTS=10) blocks new intents.
#[tokio::test]
async fn schedule_intent_rate_limit_active() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Fill up to MAX_ACTIVE_INTENTS
    for i in 0..intent::MAX_ACTIVE_INTENTS {
        memory
            .store_deferred_intent(
                "did:test:user1",
                100,
                "time",
                Some(now + 60 + i),
                None,
                "notify",
                &format!("intent {i}"),
                now + 3600,
                false,
                None,
                0,
                false,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
    }

    // LLM tries to schedule one more
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 60,
                "context_text": "one too many"
            }),
        ),
        text_response("You have too many active intents."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:user1", "schedule another intent", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // Should still be exactly MAX_ACTIVE_INTENTS (the new one was rejected)
    let count = memory.count_active_intents("did:test:user1").await.unwrap();
    assert_eq!(count, intent::MAX_ACTIVE_INTENTS);
}

/// Bounded chain depth: callbacks CAN schedule intents up to MAX_CHAIN_DEPTH.
/// A depth=0 callback scheduling a new intent creates one at depth=1.
#[tokio::test]
async fn schedule_intent_allowed_in_callback_with_bounded_depth() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 60,
                "context_text": "chained intent"
            }),
        ),
        text_response("Scheduled a follow-up."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    // Simulate a callback-originated message with depth=0
    let result = orch
        .process_callback_message(
            "did:test:user1",
            "[INTENT_CALLBACK:depth=0] check something and schedule a follow-up",
            Some(100),
        )
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // Intent should have been stored with chain_depth=1
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(
        intents.len(),
        1,
        "callback at depth 0 should be able to schedule intents"
    );
    assert_eq!(intents[0].chain_depth, 1);
}

/// Bounded chain depth: callbacks at MAX_CHAIN_DEPTH are blocked from scheduling more.
#[tokio::test]
async fn schedule_intent_blocked_at_max_chain_depth() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 60,
                "context_text": "too deep"
            }),
        ),
        text_response("Chain depth exceeded."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    // Simulate a callback at MAX_CHAIN_DEPTH
    let msg = format!(
        "[INTENT_CALLBACK:depth={}] deeply chained",
        intent::MAX_CHAIN_DEPTH
    );
    let result = orch
        .process_callback_message("did:test:user1", &msg, Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // No intent should have been stored — depth exceeded
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(
        intents.is_empty(),
        "callback at max depth should not schedule intents"
    );
}

/// Validation: invalid trigger_type is rejected gracefully.
#[tokio::test]
async fn schedule_intent_invalid_args_rejected() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "magic",
                "context_text": "nope"
            }),
        ),
        text_response("Invalid trigger type."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let result = orch
        .process_message("did:test:user1", "schedule a magic intent", Some(100))
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // No intent stored
    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(intents.is_empty());
}

/// Full flow: schedule → list → cancel, all in separate turns.
#[tokio::test]
async fn full_lifecycle_schedule_list_cancel() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Turn 1: Schedule
    let state1 = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 300,
                "execution_mode": "llm_callback",
                "context_text": "Check server health"
            }),
        ),
        text_response("Scheduled."),
    ]));
    let (addr1, _h1) = spawn_mock_server(state1).await;
    let orch1 = build_orchestrator(addr1, memory.clone(), settings.clone());
    let r1 = orch1
        .process_message("did:test:user1", "in 5 min check server health", Some(100))
        .await
        .unwrap();
    assert_eq!(r1.tools_called, vec!["schedule_intent"]);

    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(intents.len(), 1);
    let intent_id = intents[0].id;

    // Turn 2: List
    let state2 = Arc::new(MockServerState::new(vec![
        tool_call_response("call_2", "list_intents", json!({})),
        text_response("You have 1 active intent."),
    ]));
    let (addr2, _h2) = spawn_mock_server(state2).await;
    let orch2 = build_orchestrator(addr2, memory.clone(), settings.clone());
    let r2 = orch2
        .process_message("did:test:user1", "show my intents", Some(100))
        .await
        .unwrap();
    assert_eq!(r2.tools_called, vec!["list_intents"]);

    // Turn 3: Cancel
    let state3 = Arc::new(MockServerState::new(vec![
        tool_call_response("call_3", "cancel_intent", json!({"intent_id": intent_id})),
        text_response("Cancelled."),
    ]));
    let (addr3, _h3) = spawn_mock_server(state3).await;
    let orch3 = build_orchestrator(addr3, memory.clone(), settings.clone());
    let r3 = orch3
        .process_message(
            "did:test:user1",
            &format!("cancel intent {intent_id}"),
            Some(100),
        )
        .await
        .unwrap();
    assert_eq!(r3.tools_called, vec!["cancel_intent"]);

    // Verify empty
    let final_intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert!(final_intents.is_empty());
}

/// schedule_intent defaults execution_mode to "llm_callback" when omitted (fail-safe).
#[tokio::test]
async fn schedule_intent_defaults_to_llm_callback() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 60,
                "context_text": "hello"
            }),
        ),
        text_response("Scheduled."),
    ]));
    let (addr, _handle) = spawn_mock_server(state).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    orch.process_message("did:test:user1", "remind me", Some(100))
        .await
        .unwrap();

    let intents = memory.list_user_intents("did:test:user1").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].execution_mode, "llm_callback");
}

/// Event intents match on exact event_key via memory layer.
#[tokio::test]
async fn event_intent_matching_through_memory() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let now = chrono::Utc::now().timestamp();

    // Store an event intent directly
    memory
        .store_deferred_intent(
            "did:test:user1",
            100,
            "event",
            None,
            Some("email:new:work"),
            "notify",
            "New mail in work is ready!",
            now + 86400,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Exact match
    let matches = memory
        .get_matching_event_intents("email:new:work", "did:test:user1")
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].context_text, "New mail in work is ready!");

    // Partial match (different status) — should NOT match
    let no_match = memory
        .get_matching_event_intents("email:new:work:archived", "did:test:user1")
        .await
        .unwrap();
    assert!(no_match.is_empty());

    // Different proposal ID — should NOT match
    let no_match2 = memory
        .get_matching_event_intents("email:new:personal", "did:test:user1")
        .await
        .unwrap();
    assert!(no_match2.is_empty());
}
