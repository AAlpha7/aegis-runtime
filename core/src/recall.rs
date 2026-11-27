//! Recall memory pseudo-tool: unified memory query across L1/L2/L3 with pagination.
//!
//! Extracted from `orchestrator.rs` to keep the orchestrator focused on control flow.

use serde_json::Value;

use crate::embedding::EmbeddingClient;
use crate::memory::MemoryHandle;

/// Maximum characters per entry in recall_memory results.
const RECALL_ENTRY_MAX_CHARS: usize = 200;

/// Default page size for recall_memory.
const RECALL_DEFAULT_LIMIT: usize = 20;

/// Maximum page size for recall_memory.
const RECALL_MAX_LIMIT: usize = 50;

/// A unified memory entry for recall_memory results.
struct RecallEntry {
    layer: &'static str,
    label: String, // role, topic, etc.
    content: String,
    timestamp: String,
    /// Provenance tag visible to the LLM. None for legacy callers; the
    /// production path always populates this via [`classify_source`].
    source: Option<&'static str>,
    /// Confidence score (0.0–1.0) when known. L1/L3 entries have None;
    /// L2 populates in Phase B of MVP2.
    confidence: Option<f64>,
}

/// Classify an entry's provenance from its layer + topic. Used by the LLM
/// prompt builder to weigh source trust without changing the ranking
/// algorithm. Returned as `&'static str` so no allocation is needed in the
/// hot path.
///
/// Mapping:
/// - `L1` → `"session"` (current or recent conversation turns)
/// - `L2`:
///   - topic starts with `"heartbeat:"` → `"heartbeat"` (AI inner-loop observation)
///   - topic starts with `"learned:"`    → `"learned"`  (AI extraction from conversation)
///   - topic starts with `"research:"`   → `"research"` (AI-synthesized from web/docs)
///   - topic starts with `"profile:"`    → `"profile"`  (rare; most profile data lives in user_profile table)
///   - anything else                      → `"fact"`     (direct write / legacy)
/// - `L3` → `"archive"` (full-text historical log)
/// - unknown layer → `"unknown"` (defensive — not expected in production)
fn classify_source(layer: &str, topic: &str) -> &'static str {
    match layer {
        "L1" => "session",
        "L2" => {
            if topic.starts_with("heartbeat:") {
                "heartbeat"
            } else if topic.starts_with("learned:") {
                "learned"
            } else if topic.starts_with("research:") {
                "research"
            } else if topic.starts_with("profile:") {
                "profile"
            } else {
                "fact"
            }
        }
        "L3" => "archive",
        _ => "unknown",
    }
}

/// Execute the `recall_memory` pseudo-tool: query memory across L1/L2/L3 with pagination.
///
/// When a query is provided, L2 uses embedding-based semantic search (via the embedding API)
/// for high-quality matching even when wording differs. L1 uses substring matching (small data).
/// L3 uses SQLite FTS5 full-text search (already indexed, no embedding needed).
pub async fn execute_recall_memory(
    memory: &MemoryHandle,
    embedder: &EmbeddingClient,
    user_did: &str,
    args: &Value,
) -> String {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let layer = args.get("layer").and_then(|v| v.as_str()).unwrap_or("all");
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(RECALL_MAX_LIMIT))
        .unwrap_or(RECALL_DEFAULT_LIMIT);

    let has_query = !query.is_empty();

    // Collect entries from requested layers
    let mut all_entries: Vec<RecallEntry> = Vec::new();

    // ── L1: session messages (substring match — L1 is small) ──
    if (layer == "all" || layer == "l1")
        && let Ok(messages) = memory.get_l1(user_did).await
    {
        for msg in &messages {
            let content = &msg.content;
            if has_query && !content.to_lowercase().contains(&query.to_lowercase()) {
                continue;
            }
            all_entries.push(RecallEntry {
                layer: "L1",
                label: msg.role.clone(),
                content: truncate_entry(content, RECALL_ENTRY_MAX_CHARS),
                timestamp: chrono::DateTime::from_timestamp(msg.timestamp, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| msg.timestamp.to_string()),
                source: Some(classify_source("L1", "")),
                confidence: None,
            });
        }
    }

    // ── L2: long-term facts (dual-language embedding) ──
    if layer == "all" || layer == "l2" {
        if has_query {
            // Dual-language: embed original + English translation in parallel
            // to handle cross-language recall (e.g. Chinese query → English facts)
            use crate::embedding::EmbeddingClient;

            let needs_translation = EmbeddingClient::is_non_english(query);

            // Degraded-mode policy (approach b): when the embedder is on
            // backup, the query vector lives in a different semantic space
            // than the stored (primary-model) vectors. Cross-model cosine
            // scores are garbage, so we skip the semantic searches entirely
            // and let the existing substring fallback handle recall —
            // lossy but not WRONG. Log at WARN so operators see degraded
            // recall quality during an embedding outage.
            let skip_semantic = embedder.is_degraded();
            if skip_semantic {
                tracing::warn!(
                    query = %query,
                    "[recall_memory] Embedder degraded — skipping L2 semantic search, using substring fallback only"
                );
            }

            // Launch embedding + optional translation concurrently only when
            // we actually plan to use the semantic path.
            let (original_result, translated) = if skip_semantic {
                (
                    Err(anyhow::anyhow!(
                        "embedder degraded — semantic search skipped"
                    )),
                    None,
                )
            } else {
                let embed_original = embedder.embed(query);
                let translate_future = async {
                    if needs_translation {
                        embedder.translate_to_english(query).await
                    } else {
                        None
                    }
                };
                tokio::join!(embed_original, translate_future)
            };

            // Embed the translated query if available
            let translated_embedding = if skip_semantic {
                None
            } else if let Some(ref t) = translated {
                embedder
                    .embed(t)
                    .await
                    .map_err(
                        |e| tracing::warn!(error = %e, "[Recall] Failed to embed translated query"),
                    )
                    .ok()
            } else {
                None
            };

            // Collect semantic results from both embeddings, deduplicate by ID
            let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
            // When skip_semantic is true, `semantic_found` stays false so the
            // substring-fallback block below always runs.
            let mut semantic_found = false;

            // Search with original embedding
            if let Ok(ref emb) = original_result {
                if let Ok(facts) = memory.query_l2_semantic_hybrid(user_did, emb, query).await {
                    for fact in facts {
                        if seen_ids.insert(fact.id) {
                            semantic_found = true;
                            all_entries.push(RecallEntry {
                                layer: "L2",
                                label: fact.topic.clone(),
                                content: truncate_entry(&fact.fact_summary, RECALL_ENTRY_MAX_CHARS),
                                timestamp: fact.created_at.clone(),
                                source: Some(classify_source("L2", &fact.topic)),
                                confidence: Some(fact.confidence),
                            });
                        }
                    }
                }
            } else if let Err(ref e) = original_result {
                tracing::debug!(error = %e, "[recall_memory] Original embedding failed");
            }

            // Search with translated (English) embedding
            if let Some(ref emb) = translated_embedding
                && let Ok(facts) = memory
                    .query_l2_semantic_hybrid(user_did, emb, translated.as_deref().unwrap_or(query))
                    .await
            {
                let mut added = 0usize;
                for fact in facts {
                    if seen_ids.insert(fact.id) {
                        semantic_found = true;
                        added += 1;
                        all_entries.push(RecallEntry {
                            layer: "L2",
                            label: fact.topic.clone(),
                            content: truncate_entry(&fact.fact_summary, RECALL_ENTRY_MAX_CHARS),
                            timestamp: fact.created_at.clone(),
                            source: Some(classify_source("L2", &fact.topic)),
                            confidence: Some(fact.confidence),
                        });
                    }
                }
                if added > 0 {
                    tracing::info!(
                        "[recall_memory] Cross-language search added {added} extra results via translation"
                    );
                }
            }

            // Substring fallback if both semantic searches returned nothing
            // (always triggered in degraded mode — see `skip_semantic` above).
            if !semantic_found {
                tracing::info!(
                    "[recall_memory] Semantic search returned 0 results for query={query:?}, trying substring fallback"
                );
                if let Ok(facts) = memory.get_all_l2(user_did).await {
                    let q_orig = query.to_lowercase();
                    let q_en = translated.as_deref().unwrap_or("").to_lowercase();
                    for fact in &facts {
                        let topic_lc = fact.topic.to_lowercase();
                        let fact_lc = fact.fact_summary.to_lowercase();
                        let matches = topic_lc.contains(&q_orig)
                            || fact_lc.contains(&q_orig)
                            || (!q_en.is_empty()
                                && (topic_lc.contains(&q_en) || fact_lc.contains(&q_en)));
                        if matches && seen_ids.insert(fact.id) {
                            all_entries.push(RecallEntry {
                                layer: "L2",
                                label: fact.topic.clone(),
                                content: truncate_entry(&fact.fact_summary, RECALL_ENTRY_MAX_CHARS),
                                timestamp: fact.created_at.clone(),
                                source: Some(classify_source("L2", &fact.topic)),
                                confidence: Some(fact.confidence),
                            });
                        }
                    }
                }
            }

            // Rolling conversation summaries: written on context compaction,
            // never read before 2026-09-13. Substring match on the query (and
            // its English translation) — they carry no embedding.
            if let Ok(summaries) = memory.list_ephemeral_summaries(user_did, 60).await {
                let q_orig = query.to_lowercase();
                let q_en = translated.as_deref().unwrap_or("").to_lowercase();
                let mut added = 0usize;
                for item in summaries {
                    let hay = format!("{}\n{}", item.topic, item.fact_summary).to_lowercase();
                    let matches = (!q_orig.is_empty() && hay.contains(&q_orig))
                        || (!q_en.is_empty() && hay.contains(&q_en));
                    if matches && added < 6 {
                        added += 1;
                        all_entries.push(RecallEntry {
                            layer: "L2",
                            label: format!("summary:{}", item.topic),
                            content: truncate_entry(&item.fact_summary, RECALL_ENTRY_MAX_CHARS),
                            timestamp: item.created_at.clone(),
                            source: Some("summary"),
                            confidence: None,
                        });
                    }
                }
            }
        } else {
            // No query — list all L2 facts
            if let Ok(facts) = memory.get_all_l2(user_did).await {
                for fact in &facts {
                    all_entries.push(RecallEntry {
                        layer: "L2",
                        label: fact.topic.clone(),
                        content: truncate_entry(&fact.fact_summary, RECALL_ENTRY_MAX_CHARS),
                        timestamp: fact.created_at.clone(),
                        source: Some(classify_source("L2", &fact.topic)),
                        confidence: Some(fact.confidence),
                    });
                }
            }
        }
    }

    // ── L3: full-text archive (FTS5, requires query) ──
    if (layer == "all" || layer == "l3")
        && has_query
        && let Ok(entries) = memory.query_l3_fulltext(user_did, query).await
    {
        for entry in &entries {
            all_entries.push(RecallEntry {
                layer: "L3",
                label: entry.role.clone(),
                content: truncate_entry(&entry.content, RECALL_ENTRY_MAX_CHARS),
                timestamp: entry.created_at.clone(),
                source: Some(classify_source("L3", "")),
                confidence: None,
            });
        }
    }

    // ── Pagination ──
    let total = all_entries.len();

    if total == 0 {
        let hint = if !has_query && (layer == "l3" || layer == "all") {
            " (L3 archive requires a search query)"
        } else {
            ""
        };
        return serde_json::json!({
            "total": 0,
            "returned": 0,
            "offset": offset,
            "has_more": false,
            "message": format!("No matching entries found{hint}."),
            "entries": []
        })
        .to_string();
    }

    let start = offset.min(total);
    let end = (start + limit).min(total);
    let page = &all_entries[start..end];
    let returned = page.len();
    let has_more = end < total;

    let entries_json: Vec<Value> = page
        .iter()
        .map(|e| {
            serde_json::json!({
                "layer": e.layer,
                "label": e.label,
                "content": e.content,
                "timestamp": e.timestamp,
                "source": e.source,
                "confidence": e.confidence,
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "total": total,
        "returned": returned,
        "offset": start,
        "has_more": has_more,
        "entries": entries_json,
    });

    if has_more {
        result["next_offset"] = Value::from(end);
        result["remaining"] = Value::from(total - end);
        result["hint"] = Value::String(format!(
            "Showing entries {}-{} of {}. Call recall_memory with offset={} to see more.",
            start + 1,
            end,
            total,
            end
        ));
    }

    result.to_string()
}

/// Truncate a string to `max` characters at a clean boundary (no mid-char cuts).
pub(crate) fn truncate_entry(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= max {
        return trimmed.to_string();
    }
    let mut end = max;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &trimmed[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingClient;
    use crate::memory::{MemoryHandle, Message};

    /// Create a dummy EmbeddingClient for tests (embedding calls will fail gracefully,
    /// triggering the substring fallback path).
    fn dummy_embedder() -> EmbeddingClient {
        EmbeddingClient::new("http://127.0.0.1:1", "dummy", "dummy", 256)
    }

    #[tokio::test]
    async fn recall_memory_empty_db_returns_zero() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        let args = serde_json::json!({});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 0);
        assert_eq!(parsed["returned"], 0);
        assert_eq!(parsed["has_more"], false);
    }

    #[tokio::test]
    async fn recall_memory_returns_rolling_summaries_by_query() {
        // 2026-09-13: context summaries were written to l2_ephemeral and never
        // read. A query that only appears in a summary must find it, labelled
        // summary:<topic> with source "summary".
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_summary(
                "did:test",
                "owner is planning a home renovation with a 装修预算 of 80k".to_string(),
                vec![("budget".to_string(), "装修预算 8 万".to_string())],
            )
            .await
            .unwrap();
        let args = serde_json::json!({"query": "装修预算", "layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        let summary = entries
            .iter()
            .find(|e| e["label"].as_str().unwrap_or("").starts_with("summary:"))
            .unwrap_or_else(|| panic!("no summary entry in {result}"));
        assert_eq!(summary["source"], "summary");
        assert!(summary["content"].as_str().unwrap().contains("装修预算"));
        // Unrelated query: nothing from the summaries.
        let args = serde_json::json!({"query": "passport", "layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(
            parsed["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| !e["label"].as_str().unwrap_or("").starts_with("summary:"))
        );
    }

    #[tokio::test]
    async fn recall_memory_returns_l2_facts() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:test", "preference", "likes dark mode", None)
            .await
            .unwrap();
        memory
            .store_l2("did:test", "context", "works in fintech", None)
            .await
            .unwrap();

        let args = serde_json::json!({});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 2);
        assert_eq!(parsed["returned"], 2);

        let entries = parsed["entries"].as_array().unwrap();
        assert!(entries.iter().all(|e| e["layer"] == "L2"));
    }

    #[tokio::test]
    async fn recall_memory_returns_l1_messages() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .push_l1("did:test", Message::text("user", "hello", 1709312000))
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l1"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 1);
        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries[0]["layer"], "L1");
        assert_eq!(entries[0]["label"], "user");
    }

    #[tokio::test]
    async fn recall_memory_pagination_works() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        for i in 0..5 {
            memory
                .store_l2(
                    "did:test",
                    &format!("fact-{i}"),
                    &format!("content {i}"),
                    None,
                )
                .await
                .unwrap();
        }

        // Page 1: offset=0, limit=2
        let args = serde_json::json!({"limit": 2, "offset": 0});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 5);
        assert_eq!(parsed["returned"], 2);
        assert_eq!(parsed["has_more"], true);
        assert_eq!(parsed["next_offset"], 2);
        assert_eq!(parsed["remaining"], 3);

        // Page 2: offset=2, limit=2
        let args = serde_json::json!({"limit": 2, "offset": 2});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 5);
        assert_eq!(parsed["returned"], 2);
        assert_eq!(parsed["has_more"], true);

        // Page 3: offset=4, limit=2
        let args = serde_json::json!({"limit": 2, "offset": 4});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 5);
        assert_eq!(parsed["returned"], 1);
        assert_eq!(parsed["has_more"], false);
    }

    #[tokio::test]
    async fn recall_memory_query_filters_l2() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:test", "preference", "likes dark mode", None)
            .await
            .unwrap();
        memory
            .store_l2("did:test", "interest", "follows crypto markets", None)
            .await
            .unwrap();

        let args = serde_json::json!({"query": "crypto"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 1);
        let entries = parsed["entries"].as_array().unwrap();
        assert!(entries[0]["content"].as_str().unwrap().contains("crypto"));
    }

    #[tokio::test]
    async fn recall_memory_did_isolation() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:alice", "secret", "alice's private data", None)
            .await
            .unwrap();
        memory
            .store_l2("did:bob", "fact", "bob's data", None)
            .await
            .unwrap();

        let args = serde_json::json!({});
        let result = execute_recall_memory(&memory, &embedder, "did:alice", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 1);
        let entries = parsed["entries"].as_array().unwrap();
        assert!(entries[0]["content"].as_str().unwrap().contains("alice"));
    }

    #[tokio::test]
    async fn recall_memory_l3_requires_query() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();

        // Without query, L3 is not searched even with layer="l3"
        let args = serde_json::json!({"layer": "l3"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total"], 0);
        let msg = parsed["message"].as_str().unwrap();
        assert!(
            msg.contains("requires"),
            "should hint that L3 requires a query"
        );
    }

    #[test]
    fn truncate_entry_short_text() {
        assert_eq!(truncate_entry("hello", 100), "hello");
    }

    #[test]
    fn truncate_entry_exact_boundary() {
        assert_eq!(truncate_entry("hello", 5), "hello");
    }

    #[test]
    fn truncate_entry_cuts_with_ellipsis() {
        let result = truncate_entry("hello world", 5);
        assert_eq!(result, "hello…");
    }

    #[test]
    fn truncate_entry_multibyte_safe() {
        let result = truncate_entry("你好世界", 7);
        // "你好" = 6 bytes, "世" starts at 6, cut at 6
        assert_eq!(result, "你好…");
    }

    #[tokio::test]
    async fn recall_memory_l1_entries_carry_session_source() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .push_l1("did:test", Message::text("user", "hello", 1709312000))
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l1"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["source"], "session",
            "L1 entries must carry source=session"
        );
    }

    #[tokio::test]
    async fn recall_memory_l2_learned_entries_carry_learned_source() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:test", "learned:preference", "likes dark mode", None)
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["source"], "learned",
            "L2 entry with topic 'learned:*' must carry source=learned"
        );
    }

    #[tokio::test]
    async fn recall_memory_l2_non_learned_topic_falls_back_to_fact() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:test", "preference", "old-style topic", None)
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["source"], "fact",
            "L2 entry with non-prefixed topic falls back to source=fact"
        );
    }

    #[tokio::test]
    async fn recall_memory_l2_heartbeat_topic_tagged_separately() {
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2("did:test", "heartbeat:learned:focus", "working late", None)
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["source"], "heartbeat",
            "heartbeat-prefixed topics must be tagged distinctly from user-learned facts"
        );
    }

    #[tokio::test]
    async fn recall_memory_l2_entry_carries_stored_confidence() {
        // Phase B: L2 entry written via store_l2_with_meta(confidence=0.87)
        // must surface that value in the recall JSON.
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2_with_meta(
                "did:test",
                "learned:preference",
                "likes dark mode",
                None,
                0.87,
                None,
            )
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let conf = entries[0]["confidence"].as_f64();
        assert!(
            conf.is_some(),
            "L2 entry stored with confidence must surface a numeric value, got {:?}",
            entries[0]["confidence"]
        );
        let conf = conf.unwrap();
        assert!(
            (conf - 0.87).abs() < 0.001,
            "expected confidence ≈ 0.87, got {conf}"
        );
    }

    #[tokio::test]
    async fn recall_memory_l2_research_topic_tagged_research() {
        // Historical L2 rows carry topics like
        // "research:synthesis:..." / "research:finding:..." — these must
        // surface as source="research" so the LLM can weight the distinct
        // provenance (synthesized from web/docs, not observed in session).
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .store_l2(
                "did:test",
                "research:synthesis:topic-x",
                "Synthesis from literature search",
                None,
            )
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l2"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["source"], "research",
            "research:-prefixed topics must be tagged source=research"
        );
    }

    #[tokio::test]
    async fn recall_memory_l3_entries_tagged_archive() {
        // L3 is the full-text archive auto-populated via push_l1. Recall
        // requires a query; we seed an L1 message and FTS-search for it.
        let memory = MemoryHandle::spawn_in_memory().unwrap();
        let embedder = dummy_embedder();
        memory
            .push_l1(
                "did:test",
                Message::text(
                    "assistant".to_string(),
                    "historical reply about apples".to_string(),
                    1709000000,
                ),
            )
            .await
            .unwrap();

        let args = serde_json::json!({"layer": "l3", "query": "apples"});
        let result = execute_recall_memory(&memory, &embedder, "did:test", &args).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let entries = parsed["entries"].as_array().unwrap();
        assert!(
            !entries.is_empty(),
            "L3 FTS search for 'apples' must match the seeded row"
        );
        assert_eq!(
            entries[0]["source"], "archive",
            "L3 entries must carry source=archive"
        );
    }
}
