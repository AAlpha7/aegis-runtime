//! Truncate stage: MMR (Maximal Marginal Relevance) diversity selection.
//! Spec §4 Q4.4.

use crate::heirloom::query::QueryContext;
use crate::heirloom::rank::{StoredEvent, cosine_similarity};

/// A pre-scored candidate: event + its rank score. Produced by the rank
/// stage and consumed by `truncate()`.
#[derive(Debug, Clone)]
pub struct Scored {
    pub event: StoredEvent,
    pub score: f32,
}

/// MMR lambda: balance between relevance (1.0) and diversity (0.0).
/// Spec §4 Q4.4 default = 0.7 (relevance-leaning).
pub const DEFAULT_MMR_LAMBDA: f32 = 0.7;

/// Spec §4 Q4.4 MMR. Walks the candidate pool greedily: at each step
/// selects the candidate maximizing `λ · relevance − (1 − λ) · max_sim`,
/// where max_sim is the max cosine similarity between the candidate and
/// already-selected events. Stops when budget exhausted or max_results hit.
pub fn truncate(candidates: Vec<Scored>, ctx: &QueryContext, lambda: f32) -> Vec<Scored> {
    let mut remaining = candidates;
    let mut selected: Vec<Scored> = Vec::new();
    let mut budget = ctx.token_budget;

    while !remaining.is_empty() && budget > 0 && selected.len() < ctx.max_results {
        let best_idx = (0..remaining.len()).max_by(|&a, &b| {
            let sa = mmr_score(&remaining[a], &selected, lambda);
            let sb = mmr_score(&remaining[b], &selected, lambda);
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        });

        let Some(idx) = best_idx else { break };
        let chosen = remaining.remove(idx);
        if chosen.event.token_cost > budget {
            // Can't fit; try the next best candidate instead. To avoid
            // infinite loop we skip this one permanently.
            continue;
        }
        budget -= chosen.event.token_cost;
        selected.push(chosen);
    }

    selected
}

fn mmr_score(candidate: &Scored, selected: &[Scored], lambda: f32) -> f32 {
    let relevance = candidate.score;
    let diversity = selected
        .iter()
        .filter_map(|s| match (&candidate.event.embedding, &s.event.embedding) {
            (Some(c), Some(e)) => Some(cosine_similarity(c, e)),
            _ => None,
        })
        .fold(0.0f32, f32::max);
    lambda * relevance - (1.0 - lambda) * diversity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heirloom::enums::SensitivityTier;
    use crate::heirloom::hlc::HlcTimestamp;
    use crate::heirloom::query::{
        CapabilitySet, CoreOnlyGrantProvider, OwnerState, QueryContext, Scene, TemporalClass,
    };
    use crate::heirloom::types::{BundleId, Did, LineageId, WitnessChannel};
    use chrono::{DateTime, Utc};
    use std::sync::Arc;

    fn now_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_stored_event() -> StoredEvent {
        StoredEvent {
            event_hash: [0; 32],
            event_type: "observation:user_stated".to_string(),
            author_did: Did::new("did:aegis:test"),
            lineage_id: LineageId::new("lineage:test"),
            hlc_ts: HlcTimestamp { phys: 100, log: 0 },
            observed_at: now_utc(),
            sensitivity_tier: SensitivityTier::Ordinary,
            bundle_ids: vec![BundleId::new("bundle:general")],
            salience: 0.5,
            entity_refs: vec![],
            temporal_class: TemporalClass::Slow,
            scene_tag: Scene::Unknown,
            access_count: 0,
            embedding: None,
            token_cost: 10,
        }
    }

    fn sample_ctx() -> QueryContext {
        QueryContext::new_current(
            now_utc(),
            Did::new("did:aegis:test"),
            OwnerState::Alive,
            LineageId::new("lineage:test"),
            WitnessChannel::Mcp,
            Scene::Work,
            CapabilitySet::dev_default(),
            2048,
            20,
            Arc::new(CoreOnlyGrantProvider),
        )
    }

    #[test]
    fn t11_truncate_empty_candidates_returns_empty() {
        let out = truncate(vec![], &sample_ctx(), 0.7);
        assert!(out.is_empty());
    }

    #[test]
    fn t11_truncate_respects_max_results() {
        let mut cands = Vec::new();
        for i in 0..10 {
            let mut e = sample_stored_event();
            e.event_hash = [i; 32];
            cands.push(Scored {
                event: e,
                score: (10 - i) as f32,
            });
        }
        let mut ctx = sample_ctx();
        ctx.max_results = 3;
        ctx.token_budget = 10_000;
        let out = truncate(cands, &ctx, 0.7);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn t11_truncate_respects_token_budget() {
        let mut cands = Vec::new();
        for i in 0..5 {
            let mut e = sample_stored_event();
            e.event_hash = [i; 32];
            e.token_cost = 30;
            cands.push(Scored {
                event: e,
                score: (5 - i) as f32,
            });
        }
        let mut ctx = sample_ctx();
        ctx.max_results = 20;
        ctx.token_budget = 100; // only 3 * 30 = 90 fits; 4th (120) exceeds.
        let out = truncate(cands, &ctx, 0.7);
        assert_eq!(out.len(), 3, "only 3 * 30-token events fit in budget 100");
    }

    #[test]
    fn t11_truncate_skips_oversized_candidates() {
        // Budget = 100. Top-scored is 200 tokens (skip). Next-scored is 50.
        // Result must contain only the 50-token event.
        let mut big = sample_stored_event();
        big.event_hash = [0xBB; 32];
        big.token_cost = 200;
        let mut small = sample_stored_event();
        small.event_hash = [0x55; 32];
        small.token_cost = 50;
        let cands = vec![
            Scored {
                event: big,
                score: 1.0,
            },
            Scored {
                event: small,
                score: 0.5,
            },
        ];
        let mut ctx = sample_ctx();
        ctx.token_budget = 100;
        ctx.max_results = 20;
        let out = truncate(cands, &ctx, 0.7);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event.event_hash, [0x55; 32]);
    }

    #[test]
    fn t11_mmr_selects_diverse_top_k_over_redundant() {
        // SEMANTIC-CRITICAL: MMR avoids top-K dominated by near-duplicates.
        // Setup: 3 candidates. A and B are near-duplicates (high cosine).
        // C is orthogonal to both. Scores: A=1.0, B=0.95, C=0.7.
        // Pure relevance would pick A,B; MMR with λ=0.7 picks A,C.
        //
        // Note (deviation from plan Task 10): the plan quotes scores
        // (1.0, 0.95, 0.5) but with λ=0.7 the math actually picks A,B:
        //     mmr(B|{A}) = 0.7 * 0.95 - 0.3 * cos(B,A) ≈ 0.365
        //     mmr(C|{A}) = 0.7 * 0.5 - 0.3 * 0 = 0.35
        // For the diversity principle to flip the choice we need
        //     0.7 * (s_B - s_C) < 0.3 * cos(B,A)   i.e. s_B - s_C < ~0.43.
        // Bumping s_C to 0.7 satisfies this (0.7*0.25 = 0.175 < 0.3). The
        // semantic intent — "when two candidates are close in score but
        // one duplicates an already-selected event, prefer the diverse
        // one" — is preserved.
        let a_emb = vec![1.0, 0.0, 0.0];
        let b_emb = vec![0.99, 0.01, 0.0]; // ~cos 1 with A
        let c_emb = vec![0.0, 1.0, 0.0]; // cos 0 with A

        let mut ev_a = sample_stored_event();
        ev_a.embedding = Some(a_emb);
        ev_a.event_hash = [0xA; 32];
        ev_a.token_cost = 100;
        let mut ev_b = sample_stored_event();
        ev_b.embedding = Some(b_emb);
        ev_b.event_hash = [0xB; 32];
        ev_b.token_cost = 100;
        let mut ev_c = sample_stored_event();
        ev_c.embedding = Some(c_emb);
        ev_c.event_hash = [0xC; 32];
        ev_c.token_cost = 100;

        let scored = vec![
            Scored {
                event: ev_a,
                score: 1.0,
            },
            Scored {
                event: ev_b,
                score: 0.95,
            },
            Scored {
                event: ev_c,
                score: 0.7,
            },
        ];

        let mut ctx = sample_ctx();
        ctx.token_budget = 1000;
        ctx.max_results = 2;
        let out = truncate(scored, &ctx, 0.7);
        assert_eq!(out.len(), 2);
        let hashes: Vec<_> = out.iter().map(|s| s.event.event_hash[0]).collect();
        assert_eq!(hashes, vec![0xA, 0xC], "MMR must prefer A,C over A,B");
    }

    #[test]
    fn t11_mmr_lambda_1_equals_pure_relevance() {
        // With lambda = 1.0 (no diversity weight), MMR degenerates into
        // top-score selection. Three events; scores 1.0, 0.9, 0.8 — output
        // should be 0xA,0xB,0xC in score order even though embeddings are
        // near-duplicates.
        let mut ev_a = sample_stored_event();
        ev_a.embedding = Some(vec![1.0, 0.0]);
        ev_a.event_hash = [0xA; 32];
        ev_a.token_cost = 10;
        let mut ev_b = sample_stored_event();
        ev_b.embedding = Some(vec![1.0, 0.0]);
        ev_b.event_hash = [0xB; 32];
        ev_b.token_cost = 10;
        let mut ev_c = sample_stored_event();
        ev_c.embedding = Some(vec![1.0, 0.0]);
        ev_c.event_hash = [0xC; 32];
        ev_c.token_cost = 10;
        let cands = vec![
            Scored {
                event: ev_a,
                score: 1.0,
            },
            Scored {
                event: ev_b,
                score: 0.9,
            },
            Scored {
                event: ev_c,
                score: 0.8,
            },
        ];
        let mut ctx = sample_ctx();
        ctx.max_results = 3;
        ctx.token_budget = 1000;
        let out = truncate(cands, &ctx, 1.0);
        let hashes: Vec<_> = out.iter().map(|s| s.event.event_hash[0]).collect();
        assert_eq!(hashes, vec![0xA, 0xB, 0xC]);
    }
}
