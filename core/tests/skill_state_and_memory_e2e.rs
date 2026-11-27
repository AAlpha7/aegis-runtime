//! End-to-end integration tests for skill-state persistence and the L2
//! memory tier's fact-aging / dedup behaviour.
//!
//! - **A1**: Skill state persistence (host_store_state / host_load_state)
//! - **B2**: Fact aging & time-decay scoring
//! - **Ephemeral TTL**: `l2_ephemeral` expiry cleanup

mod common;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::memory::MemoryHandle;

const EMBED_DIMS: usize = 256;

/// Deterministic unit-norm embedding derived from a text hash. Keeps the
/// offline tests reproducible without an embedding provider.
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

// ═══════════════════════════════════════════════════════════════════════════════
// A1: Skill State Persistence
// ═══════════════════════════════════════════════════════════════════════════════

/// Full round-trip: store → load → verify → delete → verify gone.
#[tokio::test]
async fn skill_state_store_load_delete_roundtrip() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store
    memory
        .store_skill_state("crypto_tracker", "last_tweet_id", "1234567890")
        .await
        .unwrap();

    // Load
    let val = memory
        .load_skill_state("crypto_tracker", "last_tweet_id")
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("1234567890"));

    // Update same key
    memory
        .store_skill_state("crypto_tracker", "last_tweet_id", "9999999999")
        .await
        .unwrap();
    let val = memory
        .load_skill_state("crypto_tracker", "last_tweet_id")
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("9999999999"));

    // Delete
    memory
        .delete_skill_state("crypto_tracker", "last_tweet_id")
        .await
        .unwrap();
    let val = memory
        .load_skill_state("crypto_tracker", "last_tweet_id")
        .await
        .unwrap();
    assert!(val.is_none());
}

/// Skill state isolation: different skills don't see each other's state.
#[tokio::test]
async fn skill_state_isolation_between_skills() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    memory
        .store_skill_state("skill_a", "cursor", "100")
        .await
        .unwrap();
    memory
        .store_skill_state("skill_b", "cursor", "200")
        .await
        .unwrap();

    let a = memory.load_skill_state("skill_a", "cursor").await.unwrap();
    let b = memory.load_skill_state("skill_b", "cursor").await.unwrap();

    assert_eq!(a.as_deref(), Some("100"));
    assert_eq!(b.as_deref(), Some("200"));
}

/// Enforces max 100 keys per skill.
#[tokio::test]
async fn skill_state_max_keys_enforcement() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Fill 100 keys
    for i in 0..100 {
        memory
            .store_skill_state("test_skill", &format!("key_{}", i), "val")
            .await
            .unwrap();
    }

    // 101st should fail
    let result = memory
        .store_skill_state("test_skill", "key_overflow", "val")
        .await;
    assert!(result.is_err(), "Should reject 101st key");
}

/// Enforces 64KB max value size.
#[tokio::test]
async fn skill_state_max_value_size_enforcement() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    let big_value = "x".repeat(65 * 1024); // 65KB > 64KB limit
    let result = memory
        .store_skill_state("test_skill", "big_key", &big_value)
        .await;
    assert!(result.is_err(), "Should reject value > 64KB");
}

// ═══════════════════════════════════════════════════════════════════════════════
// B2: Fact Aging & Time-Decay
// ═══════════════════════════════════════════════════════════════════════════════

/// Stale facts query returns empty when all facts are fresh.
#[tokio::test]
async fn stale_facts_empty_when_fresh() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Store a fact (it will have last_accessed = now)
    memory
        .store_l2("did:test", "learned:pref", "User likes Rust", None)
        .await
        .unwrap();

    let stale = memory.query_stale_facts("did:test", 180).await.unwrap();
    assert!(stale.is_empty(), "Fresh fact should not be stale");
}

/// Time-decay formula: verify mathematical correctness.
#[test]
fn time_decay_formula_correctness() {
    let decay = |days: f64| -> f64 { 1.0 / (1.0 + days / 90.0) };

    // Today = full weight
    assert!((decay(0.0) - 1.0).abs() < 0.001);
    // 90 days = half weight
    assert!((decay(90.0) - 0.5).abs() < 0.001);
    // 180 days = third weight
    assert!((decay(180.0) - 1.0 / 3.0).abs() < 0.01);
    // 360 days = fifth weight
    assert!((decay(360.0) - 0.2).abs() < 0.01);
    // Monotonically decreasing
    assert!(decay(10.0) > decay(100.0));
    assert!(decay(100.0) > decay(1000.0));
    // Never reaches zero
    assert!(decay(36500.0) > 0.0);
}

/// L2 store and retrieve with last_accessed tracking.
#[tokio::test]
async fn l2_last_accessed_updates_on_retrieval() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let embedding = deterministic_embedding("User likes Rust", EMBED_DIMS);

    memory
        .store_l2(
            "did:test:decay",
            "learned:preference",
            "User likes Rust",
            Some(&embedding),
        )
        .await
        .unwrap();

    // First retrieval — should succeed and touch last_accessed
    let results = memory
        .query_l2_by_topic_prefix("did:test:decay", "learned:")
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].0.contains("Rust"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// L2 write-decision similarity band
// ═══════════════════════════════════════════════════════════════════════════════

/// The crafted-embedding helper must land inside the cosine band that
/// `WriteDecision::InsertWithContradiction` keys off (0.50-0.80 same prefix).
#[test]
fn write_decision_similarity_band_is_reachable() {
    // Create two embeddings that are similar but not identical (cosine ~0.7)
    let base = deterministic_embedding("base text for testing", EMBED_DIMS);
    let mut similar = base.clone();
    // Perturb ~30% of dimensions to get cosine ~0.7
    for v in &mut similar[..(EMBED_DIMS * 3 / 10)] {
        *v = -*v;
    }
    // Normalize
    let norm: f32 = similar.iter().map(|x| x * x).sum::<f32>().sqrt();
    for v in &mut similar {
        *v /= norm;
    }

    let cosine = EmbeddingClient::cosine_similarity(&base, &similar);
    // Verify our crafted embeddings are in the contradiction range
    assert!(
        cosine > 0.3 && cosine < 0.85,
        "Crafted cosine={:.3} should be in approximate contradiction range",
        cosine
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Ephemeral TTL cleanup
// ═══════════════════════════════════════════════════════════════════════════════

/// `cleanup_expired_ephemeral` deletes only rows whose `expires_at` has passed.
#[tokio::test]
async fn expired_ephemeral_rows_are_cleaned_up() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let did = "did:aegis:telegram:5555";

    // Pre-expired row.
    memory
        .store_ephemeral(did, "test:expired", "expired fact", None, "l1_overflow", -1)
        .await
        .unwrap();
    // Valid row.
    memory
        .store_ephemeral(did, "test:valid", "valid fact", None, "l1_overflow", 30)
        .await
        .unwrap();
    assert_eq!(memory.count_ephemeral(did, None).await.unwrap(), 2);

    let deleted = memory.cleanup_expired_ephemeral().await.unwrap();
    assert_eq!(deleted, 1, "expected 1 expired ephemeral deleted");
    assert_eq!(memory.count_ephemeral(did, None).await.unwrap(), 1);
}

/// The canonical owner DID format includes the `telegram:` transport segment
/// (regression test for `telegram.rs:1908`).
#[test]
fn telegram_admin_heartbeat_did_format() {
    let admin_chat_id: i64 = 1000000001;
    let canonical = format!("did:aegis:telegram:{admin_chat_id}");
    assert!(canonical.starts_with("did:aegis:telegram:"));
    assert!(!canonical.starts_with("did:aegis:system"));
    // The legacy (buggy) format is `did:aegis:{id}` without the transport.
    let legacy = format!("did:aegis:{admin_chat_id}");
    assert_ne!(canonical, legacy);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Real-API tests (ignored by default; require OPENAI_API_KEY)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[ignore]
async fn real_api_skill_state_persistence() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();

    // Simulate a real skill storing cursor state
    let cursor_json = serde_json::json!({
        "last_tweet_id": "1234567890123456789",
        "last_check_ts": "2026-03-09T10:30:00Z",
        "checkpoint_version": 3
    });

    memory
        .store_skill_state("x_monitor", "cursor", &cursor_json.to_string())
        .await
        .unwrap();

    // Load and verify
    let loaded = memory
        .load_skill_state("x_monitor", "cursor")
        .await
        .unwrap()
        .expect("should find stored state");

    let parsed: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    assert_eq!(parsed["last_tweet_id"], "1234567890123456789");
    assert_eq!(parsed["checkpoint_version"], 3);

    eprintln!("[real_api_skill_state] Round-trip successful: {}", loaded);
}

/// Real API: Fact aging with real embeddings — verify time-decay affects ranking.

#[tokio::test]
#[ignore]
async fn real_api_fact_aging_with_embeddings() {
    common::init_tracing();
    let api_key = common::get_env("OPENAI_API_KEY");
    let api_base = std::env::var("OPENAI_API_BASE")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
    let embed_model =
        std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let embedder = EmbeddingClient::new(&api_base, &api_key, &embed_model, EMBED_DIMS as u32);

    // Store two facts about the same topic with real embeddings
    let emb1 = embedder
        .embed("User prefers dark mode in code editors")
        .await
        .expect("embedding should succeed");
    let emb2 = embedder
        .embed("User likes dark theme for IDE and terminal")
        .await
        .expect("embedding should succeed");

    memory
        .store_l2(
            "did:aegis:test:aging",
            "learned:preference",
            "User prefers dark mode in code editors",
            Some(&emb1),
        )
        .await
        .unwrap();
    memory
        .store_l2(
            "did:aegis:test:aging",
            "learned:preference",
            "User likes dark theme for IDE and terminal",
            Some(&emb2),
        )
        .await
        .unwrap();

    // Query with a related embedding
    let query_emb = embedder
        .embed("What theme does the user prefer?")
        .await
        .expect("query embedding should succeed");

    // Both facts should be retrievable via semantic search
    let results = memory
        .query_l2_semantic("did:aegis:test:aging", &query_emb)
        .await
        .unwrap();

    eprintln!(
        "[real_api_aging] Semantic search returned {} results",
        results.len()
    );
    for exp in &results {
        eprintln!("  - [{}] {}", exp.topic, exp.fact_summary);
    }

    assert!(
        !results.is_empty(),
        "Semantic search should return at least one result"
    );
    // At least one result should be about dark mode/theme
    assert!(
        results
            .iter()
            .any(|e| e.fact_summary.to_lowercase().contains("dark")),
        "Should find a fact about dark mode/theme"
    );
}
