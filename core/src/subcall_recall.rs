//! Deterministic keyword-ranking for SubcallRegistry::recall.
//!
//! Isolated from subcall.rs so it can be unit-tested without a live
//! orchestrator. See docs/subcall-primitive-design.md §5.2 for the
//! full specification.
//!
//! NOTE: All items here are consumed by `SubcallRegistry::recall` which is
//! wired up in Task 1.7 (per docs/subcall-primitive-implementation-plan.md).
//! Until that lands, the module is staged-in but unused — suppress dead-code
//! lints module-wide rather than re-decorating each item.
#![allow(dead_code)]

const WINDOW_SIZE: usize = 500;
const MIN_TOKEN_LEN: usize = 3;
const FIRST_MATCH_BONUS_THRESHOLD: usize = 200;
const FIRST_MATCH_BONUS: i64 = 5;

/// Minimal stop-word list — kept small and structural (no domain knowledge).
/// Deliberately short: anything we fail to filter here the query author can
/// just not include in their query.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "are", "was", "were", "been", "have",
    "has", "had", "but", "not", "you", "your",
];

#[derive(Debug, Clone)]
pub struct Fragment {
    pub tool_name: String,
    pub text: String,
    pub score: i64,
}

/// Rank raw tool outputs by relevance to `query`. Returns up to 3
/// fragments with score >= 1, sorted descending.
pub fn rank_fragments(query: &str, tool_outputs: &[(String, String)]) -> Vec<Fragment> {
    let terms = tokenize_query(query);
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<Fragment> = Vec::new();
    for (tool_name, raw) in tool_outputs {
        for window in sliding_windows(raw, WINDOW_SIZE) {
            let score = score_window(window.text, &terms, window.start);
            if score >= 1 {
                scored.push(Fragment {
                    tool_name: tool_name.clone(),
                    text: window.text.to_string(),
                    score,
                });
            }
        }
    }

    scored.sort_by_key(|a| std::cmp::Reverse(a.score));
    scored.truncate(3);
    scored
}

pub fn tokenize_query(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= MIN_TOKEN_LEN)
        .map(|t| t.to_lowercase())
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .collect()
}

struct Window<'a> {
    text: &'a str,
    start: usize,
}

fn sliding_windows(text: &str, size: usize) -> Vec<Window<'_>> {
    if text.len() <= size {
        return vec![Window { text, start: 0 }];
    }
    let step = size / 2; // 50% overlap so matches near a boundary still score
    let mut windows = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        let end = (start + size).min(bytes.len());
        // Clamp to char boundary
        let end = clamp_char_boundary(text, end);
        let start_c = clamp_char_boundary(text, start);
        if start_c < end {
            windows.push(Window {
                text: &text[start_c..end],
                start: start_c,
            });
        }
        if end == bytes.len() {
            break;
        }
        start += step;
    }
    windows
}

fn clamp_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn score_window(window: &str, terms: &[String], window_start: usize) -> i64 {
    // TODO(Task 1.7): lowercase the full tool_output once before iterating
    // windows — current per-window clone allocates O(windows × window_size)
    // bytes which is wasteful on large (50KB+) tool outputs.
    let lower = window.to_lowercase();
    let mut unique_matched: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut total_matches: i64 = 0;
    // Track earliest absolute match position so the bonus is awarded per spec
    // §5.2 ("bonus if the match lands within the first 200 chars"), not per
    // window-start approximation.
    let mut earliest_match_abs: Option<usize> = None;
    for term in terms {
        for (pos, _) in lower.match_indices(term.as_str()) {
            unique_matched.insert(term.as_str());
            total_matches += 1;
            let abs = window_start + pos;
            earliest_match_abs = Some(match earliest_match_abs {
                Some(e) => e.min(abs),
                None => abs,
            });
        }
    }
    if unique_matched.is_empty() {
        return 0;
    }
    let mut score = unique_matched.len() as i64 * 10 + total_matches;
    if earliest_match_abs.is_some_and(|p| p < FIRST_MATCH_BONUS_THRESHOLD) {
        score += FIRST_MATCH_BONUS;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_filters_stop_words_and_short() {
        let terms = tokenize_query("the auth middleware is at /etc/auth");
        assert!(!terms.iter().any(|t| t == "the"));
        assert!(terms.iter().any(|t| t == "auth"));
        assert!(terms.iter().any(|t| t == "middleware"));
        assert!(!terms.iter().any(|t| t == "is"));
    }

    #[test]
    fn empty_query_returns_empty_terms() {
        assert!(tokenize_query("").is_empty());
        assert!(tokenize_query("   ").is_empty());
    }

    #[test]
    fn rank_returns_empty_when_no_matches() {
        let outs = vec![(
            "web_search".to_string(),
            "The quick brown fox jumps over the lazy dog".to_string(),
        )];
        let frags = rank_fragments("nonexistent", &outs);
        assert!(frags.is_empty());
    }

    #[test]
    fn rank_scores_unique_terms_higher() {
        let outs = vec![(
            "web_search".to_string(),
            "auth middleware handles auth auth auth everywhere".to_string(),
        )];
        let frags = rank_fragments("auth middleware handles", &outs);
        assert!(!frags.is_empty());
        assert!(frags[0].score > 0);
    }

    #[test]
    fn rank_returns_at_most_three_fragments() {
        let huge = "auth ".repeat(2000); // many windows
        let outs = vec![("web_search".to_string(), huge)];
        let frags = rank_fragments("auth", &outs);
        assert!(frags.len() <= 3);
    }

    #[test]
    fn rank_bonus_for_early_match() {
        let early = "auth appears early\n".to_string() + &"filler ".repeat(1000);
        let late = "filler ".repeat(1000) + "auth appears late";
        let outs_early = vec![("a".to_string(), early.clone())];
        let outs_late = vec![("b".to_string(), late.clone())];
        let frags_early = rank_fragments("auth", &outs_early);
        let frags_late = rank_fragments("auth", &outs_late);
        assert!(!frags_early.is_empty() && !frags_late.is_empty());
        assert!(
            frags_early[0].score >= frags_late[0].score,
            "early-match bonus should produce >= score"
        );
    }

    #[test]
    fn sliding_windows_small_text_single_window() {
        let w = sliding_windows("short", 500);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn sliding_windows_large_text_overlapping() {
        let text = "x".repeat(1200);
        let w = sliding_windows(&text, 500);
        assert!(
            w.len() >= 3,
            "expected overlapping windows, got {}",
            w.len()
        );
    }

    #[test]
    fn sliding_windows_handles_multibyte_utf8() {
        // Each CJK char is 3 bytes; 4 chars × 200 = 2400 bytes, giving
        // overlapping windows that must clamp to valid char boundaries.
        let text = "中文查询".repeat(200);
        let w = sliding_windows(&text, 500);
        assert!(w.len() >= 3, "expected multiple windows");
        // All windows must be valid UTF-8 (to_lowercase must not panic).
        for window in &w {
            let _ = window.text.to_lowercase();
            assert!(text.is_char_boundary(window.start));
        }
    }

    #[test]
    fn rank_bonus_awarded_only_when_match_lands_in_first_200() {
        // Match at abs position 199 in window[0..500] → bonus applies.
        let early_within = "auth".to_string() + &" ".repeat(500);
        let frags = rank_fragments("auth", &[("t".into(), early_within)]);
        assert!(!frags.is_empty());
        let score_with_bonus = frags[0].score;

        // Match at abs position 250 in window[0..500] → no bonus (match
        // position ≥ 200), but window still scores the match.
        let late_within = " ".repeat(250) + "auth" + &" ".repeat(500);
        let frags = rank_fragments("auth", &[("t".into(), late_within)]);
        assert!(!frags.is_empty());
        let score_no_bonus = frags[0].score;

        assert!(
            score_with_bonus > score_no_bonus,
            "early-in-window match should outscore mid-window match (got {} vs {})",
            score_with_bonus,
            score_no_bonus
        );
        assert_eq!(
            score_with_bonus - score_no_bonus,
            FIRST_MATCH_BONUS,
            "difference should equal FIRST_MATCH_BONUS"
        );
    }
}
