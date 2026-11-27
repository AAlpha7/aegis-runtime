//! Credit assignment analysis (P1) and prompt optimization (P2) for the
//! Chronos heartbeat engine.
//!
//! Extracted from `chronos.rs` to keep that file under the 2,000-line limit.
//! All methods are `impl ChronosEngine` — same pattern as `orchestrator_tools_*.rs`.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::chronos::{
    ChronosEngine, CreditAnalysisReport, PromptChangeProposal, ToolCallDetail, ToolCreditScore,
};
use crate::embedding::EmbeddingClient;
use crate::memory::MemoryHandle;

fn include_tool_in_credit_analysis(tool_name: &str) -> bool {
    !crate::heartbeat_config::is_heartbeat_bookkeeping_tool(tool_name)
}

impl ChronosEngine {
    // ── P1: Credit Assignment Analysis ─────────────────────────────────

    /// Compute credit analysis over recent heartbeat audit entries.
    ///
    /// Queries audit log entries from the last `lookback_days` that have
    /// tool_call_details populated, then uses embedding-based citation
    /// detection to determine which tool results were actually used by
    /// the LLM in its final response.
    pub async fn compute_credit_analysis(
        &self,
        memory: &crate::memory::MemoryHandle,
        lookback_days: u32,
    ) -> Result<CreditAnalysisReport> {
        let entries = memory
            .query_heartbeat_audit_recent(lookback_days)
            .await
            .context("Failed to query recent heartbeat audit entries")?;

        if entries.is_empty() {
            return Ok(CreditAnalysisReport {
                analysis_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                tool_scores: Vec::new(),
                top_effective: Vec::new(),
                most_wasteful: Vec::new(),
            });
        }

        // Parse tool_call_details from each entry and pair with LLM replies
        let mut all_tool_calls: Vec<(ToolCallDetail, Option<String>)> = Vec::new();
        for entry in &entries {
            if let Some(ref details_json) = entry.tool_call_details
                && let Ok(details) = serde_json::from_str::<Vec<ToolCallDetail>>(details_json)
            {
                for detail in details {
                    if !include_tool_in_credit_analysis(&detail.tool_name) {
                        continue;
                    }
                    all_tool_calls.push((detail, entry.llm_reply.clone()));
                }
            }
        }

        if all_tool_calls.is_empty() {
            return Ok(CreditAnalysisReport {
                analysis_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                tool_scores: Vec::new(),
                top_effective: Vec::new(),
                most_wasteful: Vec::new(),
            });
        }

        // Batch embedding-based citation detection
        let citations = self.batch_detect_citations(&all_tool_calls).await;

        // Aggregate per-tool statistics
        let mut tool_stats: HashMap<String, (u32, u32, u32, f64, i64)> = HashMap::new();
        // key -> (times_called, times_cited, wasted, total_latency_ms, total_tokens)
        for (i, (detail, _reply)) in all_tool_calls.iter().enumerate() {
            let entry = tool_stats
                .entry(detail.tool_name.clone())
                .or_insert((0, 0, 0, 0.0, 0));
            entry.0 += 1; // times_called
            if citations.get(i).copied().unwrap_or(false) {
                entry.1 += 1; // times_cited
            } else if !detail.was_blocked {
                entry.2 += 1; // wasted (called but not cited, not blocked)
            }
            entry.3 += detail.latency_ms as f64; // total_latency
            entry.4 += detail.result_size as i64; // total_tokens (approx via result size)
        }

        let analysis_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut tool_scores: Vec<ToolCreditScore> = Vec::new();

        for (tool_name, (called, cited, wasted, total_latency, total_tokens)) in &tool_stats {
            let citation_rate = if *called > 0 {
                *cited as f64 / *called as f64
            } else {
                0.0
            };
            let effectiveness = citation_rate; // Can be refined with outcome_tag weighting
            let avg_latency = if *called > 0 {
                *total_latency / *called as f64
            } else {
                0.0
            };

            let score = ToolCreditScore {
                tool_name: tool_name.clone(),
                times_called: *called,
                times_cited: *cited,
                effectiveness_score: effectiveness,
                wasted_calls: *wasted,
                avg_latency_ms: avg_latency,
            };

            // Persist to DB
            if let Err(e) = memory
                .store_credit_analysis(
                    &analysis_date,
                    tool_name,
                    *called as i32,
                    *cited as i32,
                    effectiveness,
                    *wasted as i32,
                    avg_latency,
                    (*total_tokens / 4) as i32, // rough token estimate: 4 chars per token
                )
                .await
            {
                tracing::warn!(
                    error = %e, tool_name,
                    "[CreditAnalysis] Failed to store credit analysis"
                );
            }

            tool_scores.push(score);
        }

        // Sort by effectiveness descending
        tool_scores.sort_by(|a, b| {
            b.effectiveness_score
                .partial_cmp(&a.effectiveness_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top_effective: Vec<String> = tool_scores
            .iter()
            .filter(|s| s.effectiveness_score >= 0.5 && s.times_called >= 2)
            .take(5)
            .map(|s| s.tool_name.clone())
            .collect();

        let mut wasteful_sorted: Vec<&ToolCreditScore> =
            tool_scores.iter().filter(|s| s.wasted_calls >= 2).collect();
        wasteful_sorted.sort_by_key(|a| std::cmp::Reverse(a.wasted_calls));
        let most_wasteful: Vec<String> = wasteful_sorted
            .into_iter()
            .take(5)
            .map(|s| s.tool_name.clone())
            .collect();

        Ok(CreditAnalysisReport {
            analysis_date,
            tool_scores,
            top_effective,
            most_wasteful,
        })
    }

    /// Batch detect citations using embedding cosine similarity.
    ///
    /// For each (tool_result_preview, llm_reply) pair, embeds both texts and
    /// compares with cosine similarity. Threshold >= 0.55 = cited.
    /// Uses the existing EmbeddingClient for gemini-embedding-001.
    pub(crate) async fn batch_detect_citations(
        &self,
        tool_calls: &[(ToolCallDetail, Option<String>)],
    ) -> Vec<bool> {
        const CITATION_THRESHOLD: f32 = 0.55;

        let embedder = match self.embedder() {
            Some(e) => e,
            None => {
                tracing::warn!(
                    "[CreditAnalysis] No embedder available, skipping citation detection"
                );
                return vec![false; tool_calls.len()];
            }
        };

        // Collect all unique texts to embed in a single batch
        let mut texts_to_embed: Vec<String> = Vec::new();
        let mut pair_indices: Vec<(usize, usize)> = Vec::new(); // (result_idx, reply_idx) in texts_to_embed

        for (detail, reply) in tool_calls {
            let result_text = &detail.result_preview;
            let reply_text = reply.as_deref().unwrap_or("");

            // Skip if either side is too short for meaningful comparison
            if result_text.len() < 10 || reply_text.len() < 10 {
                pair_indices.push((usize::MAX, usize::MAX)); // sentinel
                continue;
            }

            let result_idx = texts_to_embed.len();
            texts_to_embed.push(result_text.clone());
            let reply_idx = texts_to_embed.len();
            // Truncate reply to first 500 chars for embedding efficiency
            texts_to_embed.push(reply_text.chars().take(500).collect());
            pair_indices.push((result_idx, reply_idx));
        }

        if texts_to_embed.is_empty() {
            return vec![false; tool_calls.len()];
        }

        // Embed all texts (serially -- batch API not available via single-text embed)
        let mut embeddings: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts_to_embed.len());
        for text in &texts_to_embed {
            match embedder.embed(text).await {
                Ok(emb) => embeddings.push(Some(emb)),
                Err(e) => {
                    tracing::warn!(error = %e, "[CreditAnalysis] Embedding failed, marking as not cited");
                    embeddings.push(None);
                }
            }
        }

        // Compare pairs
        let mut results = Vec::with_capacity(tool_calls.len());
        for (result_idx, reply_idx) in &pair_indices {
            if *result_idx == usize::MAX {
                results.push(false);
                continue;
            }
            let cited = match (&embeddings[*result_idx], &embeddings[*reply_idx]) {
                (Some(emb_result), Some(emb_reply)) => {
                    EmbeddingClient::cosine_similarity(emb_result, emb_reply) >= CITATION_THRESHOLD
                }
                _ => false,
            };
            results.push(cited);
        }

        results
    }

    // ── P2: Prompt Optimization Analysis ────────────────────────────────

    /// Analyze recent autonomous cycle failures and tool effectiveness data
    /// to generate prompt modification proposals.
    ///
    /// This is the meta-analysis step: it queries the audit log for patterns
    /// of failure (FALLTHROUGH, empty actions) and correlates with credit
    /// analysis to identify prompt modules that could be improved.
    ///
    /// Returns proposals that must go through HITL approval before being applied.
    pub async fn analyze_prompt_optimization(
        &self,
        memory: &MemoryHandle,
        current_prompts: &HashMap<String, String>,
        lookback_days: u32,
    ) -> Result<Vec<PromptChangeProposal>> {
        let entries = memory
            .query_heartbeat_audit_recent(lookback_days)
            .await
            .context("Failed to query recent audit entries for prompt analysis")?;

        if entries.len() < 5 {
            tracing::info!(
                entry_count = entries.len(),
                "[PromptOptimizer] Insufficient data for optimization (need >= 5 entries)"
            );
            return Ok(Vec::new());
        }

        // Classify outcomes
        let total = entries.len() as f64;
        let fallthroughs = entries
            .iter()
            .filter(|e| {
                e.llm_reply
                    .as_deref()
                    .map(|r| r.trim().to_uppercase().contains("FALLTHROUGH"))
                    .unwrap_or(false)
            })
            .count();
        let no_actions = entries
            .iter()
            .filter(|e| e.actions_taken.is_empty())
            .count();
        let failure_rate = (fallthroughs + no_actions) as f64 / total;

        if failure_rate < 0.15 {
            tracing::info!(
                failure_rate = format!("{:.1}%", failure_rate * 100.0),
                "[PromptOptimizer] Failure rate below threshold (15%) — no optimization needed"
            );
            return Ok(Vec::new());
        }

        // Get credit analysis for wasteful tool insights
        let credit_report = self
            .compute_credit_analysis(memory, lookback_days)
            .await
            .unwrap_or_else(|_| CreditAnalysisReport {
                analysis_date: String::new(),
                tool_scores: Vec::new(),
                top_effective: Vec::new(),
                most_wasteful: Vec::new(),
            });

        // Collect failure pattern summaries
        let mut failure_summaries: Vec<String> = Vec::new();
        for entry in entries
            .iter()
            .filter(|e| {
                e.actions_taken.is_empty()
                    || e.llm_reply
                        .as_deref()
                        .map(|r| r.trim().to_uppercase().contains("FALLTHROUGH"))
                        .unwrap_or(false)
            })
            .take(10)
        {
            let preview = entry
                .llm_reply
                .as_deref()
                .map(|r| r.chars().take(150).collect::<String>())
                .unwrap_or_else(|| "[no reply]".into());
            failure_summaries.push(format!(
                "- Skill: {}, Tools: {:?}, Reply preview: {}",
                entry.skill_name, entry.tools_called, preview,
            ));
        }

        // Build wasteful tool summary
        let wasteful_summary: Vec<String> = credit_report
            .tool_scores
            .iter()
            .filter(|s| s.effectiveness_score < 0.3 && s.times_called >= 3)
            .take(5)
            .map(|s| {
                format!(
                    "- {} — {:.0}% effective, {} wasted calls, {:.0}ms avg latency",
                    s.tool_name,
                    s.effectiveness_score * 100.0,
                    s.wasted_calls,
                    s.avg_latency_ms
                )
            })
            .collect();

        // Identify which prompt modules to suggest changes for
        let mut proposals = Vec::new();

        // If many fallthroughs, suggest improving the proactive/monitoring prompts
        if fallthroughs as f64 / total > 0.1
            && let Some(current) = current_prompts.get("proactive")
        {
            proposals.push(PromptChangeProposal {
                    module_name: "proactive".into(),
                    original_text: current.clone(),
                    modified_text: format!(
                        "{}\n\n<!-- Auto-optimization note: {:.0}% of recent cycles resulted in FALLTHROUGH. \
                         Consider taking more decisive actions when data is available. \
                         Wasteful tools to avoid: {} -->",
                        current,
                        fallthroughs as f64 / total * 100.0,
                        if wasteful_summary.is_empty() { "none identified".into() }
                        else { credit_report.most_wasteful.join(", ") }
                    ),
                    rationale: format!(
                        "High FALLTHROUGH rate ({:.0}%). {} failure patterns observed:\n{}",
                        fallthroughs as f64 / total * 100.0,
                        failure_summaries.len(),
                        failure_summaries.join("\n"),
                    ),
                });
        }

        // If many no-actions, suggest improving monitoring prompts
        if no_actions as f64 / total > 0.2
            && let Some(current) = current_prompts.get("monitoring")
        {
            proposals.push(PromptChangeProposal {
                    module_name: "monitoring".into(),
                    original_text: current.clone(),
                    modified_text: format!(
                        "{}\n\n<!-- Auto-optimization note: {:.0}% of cycles took no action. \
                         Be more proactive about using available tools to gather and report information. -->",
                        current,
                        no_actions as f64 / total * 100.0,
                    ),
                    rationale: format!(
                        "High no-action rate ({:.0}%). The system frequently fails to take any action \
                         when monitoring. {} wasteful tool patterns:\n{}",
                        no_actions as f64 / total * 100.0,
                        wasteful_summary.len(),
                        if wasteful_summary.is_empty() { "none".into() }
                        else { wasteful_summary.join("\n") }
                    ),
                });
        }

        tracing::info!(
            proposal_count = proposals.len(),
            failure_rate = format!("{:.1}%", failure_rate * 100.0),
            fallthroughs,
            no_actions,
            "[PromptOptimizer] Analysis complete"
        );

        Ok(proposals)
    }
}

#[cfg(test)]
mod tests {
    use super::include_tool_in_credit_analysis;

    #[test]
    fn credit_analysis_excludes_heartbeat_bookkeeping_tools() {
        for tool in [
            "noop",
            "update_heartbeat_memo",
            "write_next_tick_note",
            "system_evaluate_memo_update",
            "get_current_time",
            "view_heartbeat_audit",
        ] {
            assert!(
                !include_tool_in_credit_analysis(tool),
                "{tool} should not appear in effectiveness or wasteful summaries"
            );
        }

        for tool in ["send_message", "propose_config_change", "web_search"] {
            assert!(
                include_tool_in_credit_analysis(tool),
                "{tool} should remain eligible for credit analysis"
            );
        }
    }
}
