//! Shared mock infrastructure for integration tests.
//!
//! Provides a reusable axum mock server that mimics OpenAI-compatible
//! `/v1/chat/completions` and `/v1/embeddings` endpoints, plus helpers
//! to build orchestrators and craft LLM responses.
#![allow(dead_code)]
//!
//! Extracted from `react_loop_integration.rs`, `intent_integration.rs`,
//! and `learning_integration.rs` to eliminate duplication.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::orchestrator::{Orchestrator, set_fast_token_mode};
use aegis_runtime_core::sandbox::SandboxEngine;
use aegis_runtime_core::settings::RuntimeSettings;

/// Shared Wasmtime engine — created once per test binary, avoids ~400-800 ms
/// JIT init on every `SandboxEngine::new()` call.
static SHARED_ENGINE: std::sync::LazyLock<wasmtime::Engine> = std::sync::LazyLock::new(|| {
    SandboxEngine::default_engine().expect("failed to create shared Wasmtime engine")
});

/// Create a fresh `SandboxEngine` backed by the shared Wasmtime engine.
#[allow(dead_code)]
pub fn shared_sandbox() -> SandboxEngine {
    SandboxEngine::from_engine(SHARED_ENGINE.clone()).expect("shared sandbox")
}

// ── Mock server state ────────────────────────────────────────────────────────

/// Shared state for the mock LLM server.
/// Supports a queue of chat responses (FIFO) and optional failure injection.
pub struct MockServerState {
    pub chat_responses: Mutex<Vec<String>>,
    pub chat_call_count: AtomicUsize,
    pub embed_call_count: AtomicUsize,
    pub chat_fail: AtomicBool,
    pub embed_fail: AtomicBool,
}

impl MockServerState {
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            chat_responses: Mutex::new(responses),
            chat_call_count: AtomicUsize::new(0),
            embed_call_count: AtomicUsize::new(0),
            chat_fail: AtomicBool::new(false),
            embed_fail: AtomicBool::new(false),
        }
    }

    pub fn chat_calls(&self) -> usize {
        self.chat_call_count.load(Ordering::SeqCst)
    }

    pub fn embed_calls(&self) -> usize {
        self.embed_call_count.load(Ordering::SeqCst)
    }

    /// Push additional responses onto the queue (for multi-turn tests).
    pub fn push_responses(&self, responses: Vec<String>) {
        let mut queue = self.chat_responses.lock().unwrap();
        queue.extend(responses);
    }
}

// ── Mock endpoint types ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct MockEmbedRequest {
    input: String,
    #[allow(dead_code)]
    model: Option<String>,
    #[allow(dead_code)]
    dimensions: Option<u32>,
}

pub const EMBED_DIMS: usize = 256;

// ── Response builders ────────────────────────────────────────────────────────

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

/// Default fallback when the response queue is exhausted.
pub fn fallback_text_response() -> String {
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

/// Build a text-only chat completion response.
pub fn text_response(content: &str) -> String {
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
        "usage": mock_usage(10, 10)
    })
    .to_string()
}

/// Build a tool-call chat completion response (single tool call).
pub fn tool_call_response(call_id: &str, tool_name: &str, args: Value) -> String {
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
        "usage": mock_usage(10, 10)
    })
    .to_string()
}

/// Build a multi-tool-call chat completion response.
pub fn multi_tool_call_response(calls: Vec<(&str, &str, Value)>) -> String {
    let tool_calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(i, (call_id, name, args))| {
            json!({
                "index": i,
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args.to_string()
                }
            })
        })
        .collect();

    json!({
        "id": "chatcmpl-multi",
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
                "tool_calls": tool_calls
            },
            "logprobs": null,
            "finish_reason": "tool_calls",
            "native_finish_reason": "completed"
        }],
        "usage": mock_usage(20, 20)
    })
    .to_string()
}

// ── Mock endpoint handlers ───────────────────────────────────────────────────

/// `POST /v1/chat/completions` — pops next response from queue.
async fn handle_chat(
    State(state): State<Arc<MockServerState>>,
    _body: String,
) -> axum::response::Response {
    state.chat_call_count.fetch_add(1, Ordering::SeqCst);

    if state.chat_fail.load(Ordering::SeqCst) {
        return axum::response::Response::builder()
            .status(500)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"error":{"message":"mock server error","type":"server_error"}}"#,
            ))
            .unwrap();
    }

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

    if state.embed_fail.load(Ordering::SeqCst) {
        return axum::response::Response::builder()
            .status(500)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"error":{"message":"mock embedding error","type":"server_error"}}"#,
            ))
            .unwrap();
    }

    let dims = req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize;
    let embedding = deterministic_embedding(&req.input, dims);

    let body = json!({
        "object": "list",
        "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
        "model": "mock-embed-model",
        "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.0},
        "provider": "Mock",
        "id": "gen-emb-mock"
    });

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

// ── Server + orchestrator builders ───────────────────────────────────────────

/// Spawn the mock server on a random port.
pub async fn spawn_mock_server(state: Arc<MockServerState>) -> (SocketAddr, JoinHandle<()>) {
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

/// Spawn a prompt-aware mock server whose chat handler inspects the request body
/// and delegates to a custom function to pick the response.
pub async fn spawn_prompt_aware_server(
    handler: fn(&str) -> String,
) -> (SocketAddr, Arc<AtomicUsize>, JoinHandle<()>) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let cc = call_count.clone();

    // Wrap the handler fn pointer + call_count in shared state
    let state = Arc::new((handler as usize, cc));

    #[allow(unsafe_code)] // transmute fn pointer for prompt-aware mock server
    async fn prompt_aware_chat(
        State(state): State<Arc<(usize, Arc<AtomicUsize>)>>,
        body: String,
    ) -> axum::response::Response {
        state.1.fetch_add(1, Ordering::SeqCst);
        let handler: fn(&str) -> String = unsafe { std::mem::transmute(state.0) };
        let response_body = handler(&body);
        axum::response::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(response_body))
            .unwrap()
    }

    async fn embed_fallback(_body: String) -> axum::response::Response {
        let embedding: Vec<f32> = (0..EMBED_DIMS)
            .map(|i| (i as f32 / EMBED_DIMS as f32) * 2.0 - 1.0)
            .collect();
        let body = json!({
            "object": "list",
            "data": [{"object": "embedding", "embedding": embedding, "index": 0}],
            "model": "mock-embed-model",
            "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.0},
            "provider": "Mock",
            "id": "gen-emb-mock"
        });
        axum::response::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    let app = Router::new()
        .route("/v1/chat/completions", post(prompt_aware_chat))
        .route("/v1/embeddings", post(embed_fallback))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, call_count, handle)
}

fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

/// Build a minimal Orchestrator pointed at the mock server.
/// Loads a stable built-in Wasm skill for tool execution tests.
pub fn build_orchestrator(
    addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
) -> Orchestrator {
    build_orchestrator_with_skills(addr, memory, settings, &["server_monitor"])
}

/// Build an Orchestrator with specific Wasm skills loaded.
pub fn build_orchestrator_with_skills(
    addr: SocketAddr,
    memory: MemoryHandle,
    settings: Arc<RuntimeSettings>,
    skill_names: &[&str],
) -> Orchestrator {
    set_fast_token_mode(true);

    let api_base = format!("http://{}/v1", addr);
    let embedder =
        EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32);
    let sandbox = shared_sandbox();

    for name in skill_names {
        let original_path = compiled_skills_dir().join(format!("{name}.wasm"));
        // The OSS snapshot omits these private integrations. Generic routing
        // tests still need a real sandboxed Wasm body under the legacy tool
        // name, so use the shipped no-network fixture. Completeness tests
        // separately verify which actual skills are distributed.
        let wasm_path = if original_path.exists() {
            original_path
        } else if matches!(*name, "server_monitor" | "calendar_reader") {
            compiled_skills_dir().join("test_no_net.wasm")
        } else {
            panic!("Wasm skill not found: {}", original_path.display());
        };
        let (domains, schema) = match *name {
            "server_monitor" => (vec![], json!({"type": "object"})),
            _ => (vec![], json!({"type": "object"})),
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

    let sandbox = Arc::new(sandbox);
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

// ── Embedding helper ─────────────────────────────────────────────────────────

/// Generate a deterministic, normalized embedding vector from text hash.
pub fn deterministic_embedding(text: &str, dims: usize) -> Vec<f32> {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let seed = hasher.finish();
    let mut result: Vec<f32> = (0..dims)
        .map(|i| {
            let v = ((seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(i as u64)) as f32)
                / u64::MAX as f32;
            v * 2.0 - 1.0
        })
        .collect();
    // Normalize to unit vector
    let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut result {
            *v /= norm;
        }
    }
    result
}
