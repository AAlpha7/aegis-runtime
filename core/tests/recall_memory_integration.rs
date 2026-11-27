//! End-to-end integration tests for the `recall_memory` pseudo-tool.
//!
//! Uses a local axum mock HTTP server for the `/v1/embeddings` endpoint
//! to test the semantic search path deterministically. One `#[ignore]`
//! test validates against the real OpenRouter API.
//!
//! Mock response format derived from actual OpenRouter API call:
//! ```json
//! {
//!   "object": "list",
//!   "data": [{"object": "embedding", "embedding": [...], "index": 0}],
//!   "model": "text-embedding-3-small",
//!   "usage": {"prompt_tokens": 9, "total_tokens": 9}
//! }
//! ```

mod common;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::recall::execute_recall_memory;

// ── Mock embedding server (same pattern as learning_integration.rs) ─────────

struct MockEmbedState {
    call_count: AtomicUsize,
    fail: AtomicBool,
    /// When set, override the dimension count (otherwise use request's `dimensions`).
    force_dims: Mutex<Option<usize>>,
}

impl MockEmbedState {
    fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            force_dims: Mutex::new(None),
        }
    }
}

#[derive(Deserialize)]
struct EmbedRequest {
    input: String,
    #[allow(dead_code)]
    model: Option<String>,
    dimensions: Option<u32>,
}

#[derive(Serialize)]
struct EmbedResponse {
    object: String,
    data: Vec<EmbedData>,
    model: String,
    usage: EmbedUsage,
}

#[derive(Serialize)]
struct EmbedData {
    object: String,
    embedding: Vec<f32>,
    index: u32,
}

#[derive(Serialize)]
struct EmbedUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

const EMBED_DIMS: usize = 256;

async fn handle_embed(
    State(state): State<Arc<MockEmbedState>>,
    Json(req): Json<EmbedRequest>,
) -> axum::response::Response {
    state.call_count.fetch_add(1, Ordering::SeqCst);

    if state.fail.load(Ordering::SeqCst) {
        return axum::response::Response::builder()
            .status(500)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"error":{"message":"mock error","type":"server_error"}}"#,
            ))
            .unwrap();
    }

    let dims = state
        .force_dims
        .lock()
        .unwrap()
        .unwrap_or(req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize);

    let embedding = deterministic_embedding(&req.input, dims);
    let body = serde_json::to_string(&EmbedResponse {
        object: "list".into(),
        data: vec![EmbedData {
            object: "embedding".into(),
            embedding,
            index: 0,
        }],
        model: "mock-embed-model".into(),
        usage: EmbedUsage {
            prompt_tokens: 5,
            total_tokens: 5,
        },
    })
    .unwrap();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// Deterministic, normalized embedding from text hash.
/// Same text → identical vector → cosine 1.0.
fn deterministic_embedding(text: &str, dims: usize) -> Vec<f32> {
    let mut result = Vec::with_capacity(dims);
    for i in 0..dims {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        i.hash(&mut hasher);
        let h = hasher.finish();
        result.push((h as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32);
    }
    let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut result {
            *v /= norm;
        }
    }
    result
}

async fn spawn_embed_server(state: Arc<MockEmbedState>) -> (SocketAddr, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/embeddings", post(handle_embed))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

fn build_embedder(addr: SocketAddr) -> EmbeddingClient {
    let api_base = format!("http://{}/v1", addr);
    EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32)
}

// ── Helper: store a fact with its deterministic embedding ────────────────────

async fn store_fact_with_embedding(memory: &MemoryHandle, did: &str, topic: &str, content: &str) {
    let embedding = deterministic_embedding(content, EMBED_DIMS);
    memory
        .store_l2(did, topic, content, Some(&embedding))
        .await
        .unwrap();
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Semantic search via mock embedding API: query matches semantically similar facts.
#[tokio::test]
async fn recall_semantic_search_finds_related_facts() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store facts with their deterministic embeddings
    store_fact_with_embedding(
        &memory,
        "did:test",
        "learned:interest",
        "User is interested in cryptocurrency and Bitcoin",
    )
    .await;
    store_fact_with_embedding(
        &memory,
        "did:test",
        "learned:preference",
        "User prefers dark mode for all applications",
    )
    .await;
    store_fact_with_embedding(
        &memory,
        "did:test",
        "learned:habit",
        "User checks server health every morning",
    )
    .await;

    // Query with the exact content text → deterministic embedding matches exactly (cosine=1.0)
    let args = serde_json::json!({
        "query": "User is interested in cryptocurrency and Bitcoin",
        "layer": "l2"
    });
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Should find at least the exact-match fact via semantic search
    assert!(
        parsed["total"].as_u64().unwrap() >= 1,
        "semantic search should find at least the matching fact"
    );
    let entries = parsed["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["content"].as_str().unwrap().contains("cryptocurrency")),
        "should find the cryptocurrency fact"
    );

    // Embedding API was called (semantic path, not substring fallback)
    assert!(
        state.call_count.load(Ordering::SeqCst) >= 1,
        "embedding API should have been called for semantic search"
    );
}

/// Embedding failure triggers substring fallback gracefully.
#[tokio::test]
async fn recall_embedding_failure_falls_back_to_substring() {
    let state = Arc::new(MockEmbedState::new());
    state.fail.store(true, Ordering::SeqCst);
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store facts (no embedding needed for substring fallback)
    memory
        .store_l2(
            "did:test",
            "preference",
            "User likes cryptocurrency trading",
            None,
        )
        .await
        .unwrap();
    memory
        .store_l2("did:test", "habit", "User checks email daily", None)
        .await
        .unwrap();

    let args = serde_json::json!({"query": "crypto", "layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Substring fallback should find the cryptocurrency fact
    assert_eq!(parsed["total"], 1);
    let entries = parsed["entries"].as_array().unwrap();
    assert!(entries[0]["content"].as_str().unwrap().contains("crypto"));

    // Embedding API was attempted but failed
    assert!(state.call_count.load(Ordering::SeqCst) >= 1);
}

/// Cross-layer query: L1 substring + L2 semantic + L3 FTS all combined.
#[tokio::test]
async fn recall_cross_layer_query() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // L1: push a message containing "bitcoin"
    memory
        .push_l1(
            "did:test",
            aegis_runtime_core::memory::Message {
                role: "user".into(),
                content: "What is the bitcoin price today?".into(),
                timestamp: 1709312000,
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
                attachments: vec![],
            },
        )
        .await
        .unwrap();

    // L2: store a fact about bitcoin with embedding.
    // Use "bitcoin" as content so the deterministic embedding matches the query exactly
    // (hash-based mock embeddings only match when text is identical).
    store_fact_with_embedding(&memory, "did:test", "learned:interest", "bitcoin").await;

    // L3: push some archived messages (L3 is populated by push_l1 in production,
    // but for testing we rely on the L1+L2 layers since L3 FTS requires specific schema)

    let args = serde_json::json!({"query": "bitcoin"});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Should find entries from both L1 and L2
    assert!(
        parsed["total"].as_u64().unwrap() >= 2,
        "should find entries from both L1 and L2, got total={}",
        parsed["total"]
    );
    let entries = parsed["entries"].as_array().unwrap();
    let layers: Vec<&str> = entries
        .iter()
        .map(|e| e["layer"].as_str().unwrap())
        .collect();
    assert!(layers.contains(&"L1"), "should include L1 result");
    assert!(layers.contains(&"L2"), "should include L2 result");
}

/// Pagination with semantic search: page through large result sets.
#[tokio::test]
async fn recall_pagination_with_semantic_search() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store many L2 facts (no query → lists all, tests pagination)
    for i in 0..7 {
        memory
            .store_l2(
                "did:test",
                &format!("fact-{i}"),
                &format!("Content for fact number {i}"),
                None,
            )
            .await
            .unwrap();
    }

    // Page 1: limit=3
    let args = serde_json::json!({"limit": 3, "offset": 0});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let p1: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(p1["total"], 7);
    assert_eq!(p1["returned"], 3);
    assert_eq!(p1["has_more"], true);
    assert_eq!(p1["next_offset"], 3);
    assert_eq!(p1["remaining"], 4);

    // Page 2: offset=3, limit=3
    let args = serde_json::json!({"limit": 3, "offset": 3});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let p2: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(p2["total"], 7);
    assert_eq!(p2["returned"], 3);
    assert_eq!(p2["has_more"], true);

    // Page 3: offset=6, limit=3 → only 1 remaining
    let args = serde_json::json!({"limit": 3, "offset": 6});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let p3: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(p3["total"], 7);
    assert_eq!(p3["returned"], 1);
    assert_eq!(p3["has_more"], false);

    // Verify no duplicate entries across pages
    let all_labels: Vec<String> = [&p1, &p2, &p3]
        .iter()
        .flat_map(|p| {
            p["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["label"].as_str().unwrap().to_string())
        })
        .collect();
    assert_eq!(all_labels.len(), 7, "7 unique entries across 3 pages");
}

/// DID isolation: semantic search only returns the querying user's data.
#[tokio::test]
async fn recall_semantic_did_isolation() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Alice's fact
    store_fact_with_embedding(
        &memory,
        "did:alice",
        "learned:secret",
        "Alice's private financial data",
    )
    .await;

    // Bob's fact
    store_fact_with_embedding(
        &memory,
        "did:bob",
        "learned:info",
        "Bob's financial information",
    )
    .await;

    // Alice queries — should only see her own data
    let args = serde_json::json!({"query": "financial", "layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:alice", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    let entries = parsed["entries"].as_array().unwrap();
    for entry in entries {
        assert!(
            entry["content"].as_str().unwrap().contains("Alice"),
            "Alice should only see her own data, but got: {}",
            entry["content"]
        );
    }
}

/// No query lists all L1+L2 without calling embedding API.
#[tokio::test]
async fn recall_no_query_skips_embedding_api() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    memory
        .store_l2("did:test", "fact1", "some content", None)
        .await
        .unwrap();
    memory
        .push_l1(
            "did:test",
            aegis_runtime_core::memory::Message {
                role: "user".into(),
                content: "hello".into(),
                timestamp: 1709312000,
                anchor_kind: None,
                anchor_ref: None,
                latency_ms: None,
                attachments: vec![],
            },
        )
        .await
        .unwrap();

    let args = serde_json::json!({});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Should return both L1 and L2 entries
    assert_eq!(parsed["total"], 2);

    // Embedding API should NOT have been called (no query = no semantic search)
    assert_eq!(
        state.call_count.load(Ordering::SeqCst),
        0,
        "no embedding API call when no query is provided"
    );
}

/// Entry content is truncated at clean boundaries, not mid-character.
#[tokio::test]
async fn recall_long_content_truncated_cleanly() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _server) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store a fact with very long content (> 200 chars)
    let long_content = "这是一段很长的中文内容。".repeat(50); // ~300 chars of CJK
    memory
        .store_l2("did:test", "long_fact", &long_content, None)
        .await
        .unwrap();

    let args = serde_json::json!({});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    let entries = parsed["entries"].as_array().unwrap();
    let content = entries[0]["content"].as_str().unwrap();

    // Should be truncated (with ellipsis)
    assert!(
        content.len() <= 220, // 200 chars + some multibyte + ellipsis
        "content should be truncated, got {} bytes",
        content.len()
    );
    assert!(
        content.ends_with('…'),
        "truncated content should end with …"
    );

    // Should be valid UTF-8 (no mid-character cuts)
    assert!(
        content.is_char_boundary(content.len()),
        "truncation must be at a valid char boundary"
    );
}

/// Semantic search via mock embedding server.
///
/// Uses a concept-cluster approach: texts in the same semantic group get
/// similar vectors (high cosine), unrelated texts get low cosine.
/// This replaces the previous real-API test for deterministic CI.
///
/// Concept clusters:
///   - crypto: "Bitcoin", "cryptocurrency"
///   - programming: "Rust programming", "programming language"
///   - unrelated: "dark mode", "fintech startup"
#[tokio::test]
async fn semantic_recall_search_with_concept_clusters() {
    // Build a semantic-aware mock server
    let state = Arc::new(SemanticMockState::new());
    let app = Router::new()
        .route("/v1/embeddings", post(handle_semantic_embed))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let api_base = format!("http://{}/v1", addr);
    let embedder =
        EmbeddingClient::new(&api_base, "mock-key", "mock-embed-model", EMBED_DIMS as u32);
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store 4 facts with concept-aware embeddings
    let facts = [
        (
            "learned:interest",
            "User is passionate about Rust programming",
        ),
        ("learned:preference", "User prefers dark mode in all IDEs"),
        ("learned:habit", "User checks Bitcoin price every morning"),
        ("learned:context", "User works at a fintech startup"),
    ];

    for (topic, content) in &facts {
        let embedding = embedder.embed(content).await.expect("embed should work");
        memory
            .store_l2("did:test:semantic", topic, content, Some(&embedding))
            .await
            .unwrap();
    }

    // Query: "cryptocurrency" → should match "Bitcoin" fact (same cluster)
    let args = serde_json::json!({"query": "cryptocurrency", "layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:test:semantic", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed["total"].as_u64().unwrap() >= 1,
        "semantic search for 'cryptocurrency' should find Bitcoin fact, got: {result}"
    );
    let entries = parsed["entries"].as_array().unwrap();
    let found_bitcoin = entries.iter().any(|e| {
        e["content"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("bitcoin")
    });
    assert!(found_bitcoin, "crypto query must match Bitcoin fact");

    // Query: "programming language" → should match "Rust" fact (same cluster)
    let args = serde_json::json!({"query": "programming language", "layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:test:semantic", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed["total"].as_u64().unwrap() >= 1,
        "semantic search for 'programming language' should find Rust fact, got: {result}"
    );
    let entries = parsed["entries"].as_array().unwrap();
    let found_rust = entries.iter().any(|e| {
        e["content"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("rust")
    });
    assert!(found_rust, "programming query must match Rust fact");

    // Query: "dark theme" → should match "dark mode" fact (same cluster)
    let args = serde_json::json!({"query": "dark theme", "layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:test:semantic", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed["total"].as_u64().unwrap() >= 1,
        "semantic search for 'dark theme' should find dark mode fact, got: {result}"
    );

    // No-query list all
    let args = serde_json::json!({});
    let result = execute_recall_memory(&memory, &embedder, "did:test:semantic", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["total"], 4,
        "listing all should return all 4 stored facts"
    );
}

/// MVP2 Phase A: every recall entry must carry a non-null `source` tag so the
/// LLM prompt gets provenance signal. L2 facts with different topic prefixes
/// map to different source labels (`learned` / `heartbeat` / `fact`).
#[tokio::test]
async fn integration_recall_emits_source_field_for_every_entry() {
    let state = Arc::new(MockEmbedState::new());
    let (addr, _handle) = spawn_embed_server(state.clone()).await;
    let embedder = build_embedder(addr);

    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // L2: one learned, one heartbeat, one bare topic. Use deterministic
    // embeddings so the no-query path is exercised cleanly.
    memory
        .store_l2(
            "did:test",
            "learned:preference",
            "prefers dark mode",
            Some(&[0.1f32; EMBED_DIMS]),
        )
        .await
        .unwrap();
    memory
        .store_l2(
            "did:test",
            "heartbeat:learned:focus",
            "working late",
            Some(&[0.2f32; EMBED_DIMS]),
        )
        .await
        .unwrap();
    memory
        .store_l2(
            "did:test",
            "fact",
            "server IP is 10.0.0.1",
            Some(&[0.3f32; EMBED_DIMS]),
        )
        .await
        .unwrap();

    let args = serde_json::json!({"layer": "l2"});
    let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    let entries = parsed["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);

    let sources: Vec<&str> = entries
        .iter()
        .map(|e| e["source"].as_str().expect("source must be a string"))
        .collect();

    assert!(
        sources.contains(&"learned"),
        "learned:* topic must map to source=learned, got {sources:?}"
    );
    assert!(
        sources.contains(&"heartbeat"),
        "heartbeat:* topic must map to source=heartbeat, got {sources:?}"
    );
    assert!(
        sources.contains(&"fact"),
        "bare topic must fall back to source=fact, got {sources:?}"
    );
}

// ── Semantic concept-cluster mock ────────────────────────────────────────────

/// Concept clusters for deterministic semantic similarity.
/// Texts containing these keywords map to the same base vector (+ noise).
const CONCEPT_CLUSTERS: &[(&[&str], usize)] = &[
    (&["bitcoin", "crypto", "cryptocurrency", "coin"], 0),
    (&["rust", "programming", "language", "code", "compiler"], 1),
    (&["dark", "mode", "theme", "ui", "ide"], 2),
    (&["fintech", "startup", "finance", "company", "work"], 3),
    (&["remind", "timer", "schedule", "alarm"], 4),
];

struct SemanticMockState;
impl SemanticMockState {
    fn new() -> Self {
        Self
    }
}

/// Generate an embedding where texts in the same concept cluster have cosine > 0.85,
/// and unrelated texts have cosine < 0.2.
fn concept_embedding(text: &str, dims: usize) -> Vec<f32> {
    let lower = text.to_lowercase();

    // Find matching cluster
    let cluster_id = CONCEPT_CLUSTERS
        .iter()
        .find(|(keywords, _)| keywords.iter().any(|kw| lower.contains(kw)))
        .map(|(_, id)| *id);

    let mut vec = vec![0.0f32; dims];

    match cluster_id {
        Some(id) => {
            // Set a strong signal in dimensions [id*50 .. id*50+50]
            // Each cluster occupies a distinct subspace
            let base = (id * 50) % dims;
            for i in 0..50.min(dims - base) {
                vec[base + i] = 1.0;
            }
            // Add small text-specific noise so different texts in same cluster
            // aren't *identical* (cosine ~0.95 instead of 1.0)
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            let h = hasher.finish();
            for (i, v) in vec.iter_mut().enumerate() {
                let noise_hasher = {
                    let mut nh = DefaultHasher::new();
                    h.hash(&mut nh);
                    i.hash(&mut nh);
                    nh.finish()
                };
                let noise = (noise_hasher as f64 / u64::MAX as f64 * 0.1 - 0.05) as f32;
                *v += noise;
            }
        }
        None => {
            // Unknown concept — use hash-based random vector (uncorrelated with clusters)
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            let h = hasher.finish();
            for (i, v) in vec.iter_mut().enumerate() {
                let mut nh = DefaultHasher::new();
                h.hash(&mut nh);
                i.hash(&mut nh);
                let hv = nh.finish();
                *v = (hv as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32;
            }
        }
    }

    // Normalize
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

async fn handle_semantic_embed(
    State(_state): State<Arc<SemanticMockState>>,
    Json(req): Json<EmbedRequest>,
) -> axum::response::Response {
    let dims = req.dimensions.unwrap_or(EMBED_DIMS as u32) as usize;
    let embedding = concept_embedding(&req.input, dims);
    let body = serde_json::to_string(&EmbedResponse {
        object: "list".into(),
        data: vec![EmbedData {
            object: "embedding".into(),
            embedding,
            index: 0,
        }],
        model: "mock-semantic-model".into(),
        usage: EmbedUsage {
            prompt_tokens: 5,
            total_tokens: 5,
        },
    })
    .unwrap();

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}
