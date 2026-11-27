//! Proactive context compaction with deterministic extraction and priority ranking.
//!
//! Ported from Alpha project patterns (`compact.rs` + `summary_compression.rs`).
//! Extracts key context deterministically (zero LLM cost) before optional LLM
//! summarization, and compresses summaries using a 4-tier priority system.

use async_openai::types::chat::ChatCompletionRequestMessage;
use regex::Regex;
use std::sync::LazyLock;

// ── Thresholds ──────────────────────────────────────────────────────────────

/// Token count that triggers proactive compaction (down from 500K).
pub const COMPACT_TRIGGER_TOKENS: usize = 100_000;

/// Minimum messages before compaction is considered (avoids compacting tiny contexts).
pub const MIN_MESSAGES_FOR_COMPACT: usize = 6;

/// Number of recent messages to always preserve (3 user-assistant exchanges).
pub const PRESERVE_RECENT: usize = 6;

// ── Compression budget ──────────────────────────────────────────────────────

/// Budget for compressed summaries (from Alpha's summary_compression.rs).
pub struct CompressionBudget {
    pub max_chars: usize,
    pub max_lines: usize,
    pub max_line_chars: usize,
}

impl Default for CompressionBudget {
    fn default() -> Self {
        Self {
            max_chars: 1200,
            max_lines: 24,
            max_line_chars: 160,
        }
    }
}

// ── Key context extraction ──────────────────────────────────────────────────

/// Deterministically extracted context from messages (zero LLM cost).
#[derive(Debug, Default)]
pub struct KeyContext {
    pub file_paths: Vec<String>,
    pub pending_work: Vec<String>,
    pub tool_names: Vec<String>,
    pub recent_requests: Vec<String>,
    pub current_work: Option<String>,
}

static FILE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\w./\-]+\.(rs|ts|py|json|toml|md|yaml|yml|sql|go|js|tsx|jsx|sh|css|html)")
        .unwrap()
});

const PENDING_KEYWORDS: &[&str] = &[
    "todo",
    "next",
    "pending",
    "remaining",
    "follow up",
    "接下来",
    "待办",
    "还需要",
];

/// Extract key context from messages deterministically (no LLM call).
pub fn extract_key_context(messages: &[ChatCompletionRequestMessage]) -> KeyContext {
    let mut ctx = KeyContext::default();
    let mut seen_files = std::collections::HashSet::new();
    let mut seen_tools = std::collections::HashSet::new();

    for msg in messages {
        let text = message_text(msg);
        if text.is_empty() {
            continue;
        }

        // File paths
        for m in FILE_PATH_RE.find_iter(&text) {
            let path = m.as_str().to_string();
            if seen_files.insert(path.clone()) && ctx.file_paths.len() < 8 {
                ctx.file_paths.push(path);
            }
        }

        // Pending work (from assistant messages)
        if is_assistant(msg) {
            for line in text.lines() {
                let line_lower = line.to_lowercase();
                if PENDING_KEYWORDS.iter().any(|kw| line_lower.contains(kw))
                    && ctx.pending_work.len() < 5
                {
                    let trimmed: String = line.chars().take(160).collect();
                    ctx.pending_work.push(trimmed);
                }
            }
        }

        // Tool names (from tool call messages)
        if let ChatCompletionRequestMessage::Assistant(a) = msg
            && let Some(calls) = &a.tool_calls
        {
            for tc in calls {
                if let async_openai::types::chat::ChatCompletionMessageToolCalls::Function(inner) =
                    tc
                {
                    let name = inner.function.name.clone();
                    if seen_tools.insert(name.clone()) {
                        ctx.tool_names.push(name);
                    }
                }
            }
        }

        // Recent user requests (last 3)
        if is_user(msg) {
            let truncated: String = text.chars().take(160).collect();
            ctx.recent_requests.push(truncated);
        }
    }

    // Keep only last 3 user requests
    if ctx.recent_requests.len() > 3 {
        ctx.recent_requests = ctx.recent_requests.split_off(ctx.recent_requests.len() - 3);
    }

    // Current work: last assistant text
    for msg in messages.iter().rev() {
        if is_assistant(msg) {
            let text = message_text(msg);
            if !text.is_empty() {
                ctx.current_work = Some(text.chars().take(200).collect());
                break;
            }
        }
    }

    // Sort tool names for determinism
    ctx.tool_names.sort();
    ctx.tool_names.dedup();

    ctx
}

// ── Summary formatting ──────────────────────────────────────────────────────

/// Format a structured compaction summary from extracted context.
/// Merges with `existing_summary` if provided (multi-round compaction).
pub fn format_compaction_summary(ctx: &KeyContext, existing_summary: Option<&str>) -> String {
    let mut out = String::with_capacity(1200);
    out.push_str("Conversation summary:\n");

    if let Some(ref work) = ctx.current_work {
        out.push_str(&format!("- Current work: {}\n", work));
    }

    if !ctx.pending_work.is_empty() {
        out.push_str("- Pending work:\n");
        for p in &ctx.pending_work {
            out.push_str(&format!("  - {}\n", p));
        }
    }

    if !ctx.file_paths.is_empty() {
        out.push_str(&format!("- Key files: {}\n", ctx.file_paths.join(", ")));
    }

    if !ctx.tool_names.is_empty() {
        out.push_str(&format!("- Tools used: {}\n", ctx.tool_names.join(", ")));
    }

    if !ctx.recent_requests.is_empty() {
        out.push_str("- Recent user requests:\n");
        for r in &ctx.recent_requests {
            out.push_str(&format!("  - {}\n", r));
        }
    }

    if let Some(prev) = existing_summary {
        out.push_str("- Previously compacted context:\n");
        for line in prev.lines().take(10) {
            out.push_str(&format!("  {}\n", line));
        }
    }

    out
}

// ── Priority-ranked compression ─────────────────────────────────────────────

/// Compress a summary to fit within the budget using 4-tier priority ranking.
/// P0 (never dropped): "Conversation summary:", "Current work:", "Pending work:", etc.
/// P1 (last dropped): Section headers (lines ending with ":")
/// P2: Bullet points ("- " or "  - ")
/// P3 (first dropped): All other text
pub fn compress_summary(summary: &str, budget: &CompressionBudget) -> String {
    let lines: Vec<&str> = summary.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    // Truncate individual lines
    let truncated: Vec<String> = lines
        .iter()
        .map(|l| {
            if l.chars().count() > budget.max_line_chars {
                let s: String = l.chars().take(budget.max_line_chars - 3).collect();
                format!("{}...", s)
            } else {
                l.to_string()
            }
        })
        .collect();

    // Check if already fits
    let joined = truncated.join("\n");
    if joined.len() <= budget.max_chars && truncated.len() <= budget.max_lines {
        return joined;
    }

    // Priority-rank each line
    let ranked: Vec<(usize, &str)> = truncated
        .iter()
        .enumerate()
        .map(|(i, l)| (line_priority(l), truncated[i].as_str()))
        .collect();

    // Drop lowest priority lines first until within budget
    // Sort by priority descending (P3 first) for dropping
    let mut indices_by_priority: Vec<usize> = (0..ranked.len()).collect();
    indices_by_priority.sort_by(|&a, &b| ranked[b].0.cmp(&ranked[a].0));

    let mut keep = vec![true; ranked.len()];
    let mut total_chars: usize = ranked.iter().map(|(_, l)| l.len() + 1).sum();
    let mut total_lines = ranked.len();

    for &idx in &indices_by_priority {
        if total_chars <= budget.max_chars && total_lines <= budget.max_lines {
            break;
        }
        // Never drop P0 lines
        if ranked[idx].0 == 0 {
            continue;
        }
        keep[idx] = false;
        total_chars -= ranked[idx].1.len() + 1;
        total_lines -= 1;
    }

    ranked
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, (_, l))| *l)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether compaction should trigger based on token count and message count.
pub fn should_compact(token_count: usize, message_count: usize) -> bool {
    message_count > MIN_MESSAGES_FOR_COMPACT && token_count >= COMPACT_TRIGGER_TOKENS
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// 4-tier priority: 0 = highest (never drop), 3 = lowest (drop first).
fn line_priority(line: &str) -> usize {
    let trimmed = line.trim();
    // P0: core context lines
    if trimmed.starts_with("Conversation summary:")
        || trimmed.starts_with("- Current work:")
        || trimmed.starts_with("- Pending work:")
        || trimmed.starts_with("- Key files:")
        || trimmed.starts_with("- Tools used:")
        || trimmed.starts_with("- Recent user requests:")
        || trimmed.starts_with("- Previously compacted context:")
    {
        return 0;
    }
    // P1: section headers
    if trimmed.ends_with(':') && !trimmed.starts_with('-') {
        return 1;
    }
    // P2: bullet points
    if trimmed.starts_with("- ") || trimmed.starts_with("  - ") || trimmed.starts_with("  ") {
        return 2;
    }
    // P3: everything else
    3
}

/// Extract text content from a message (best-effort).
fn message_text(msg: &ChatCompletionRequestMessage) -> String {
    match msg {
        ChatCompletionRequestMessage::System(s) => match &s.content {
            async_openai::types::chat::ChatCompletionRequestSystemMessageContent::Text(t) => {
                t.clone()
            }
            async_openai::types::chat::ChatCompletionRequestSystemMessageContent::Array(_) => {
                String::new()
            }
        },
        ChatCompletionRequestMessage::User(u) => match &u.content {
            async_openai::types::chat::ChatCompletionRequestUserMessageContent::Text(t) => {
                t.clone()
            }
            async_openai::types::chat::ChatCompletionRequestUserMessageContent::Array(_) => {
                String::new()
            }
        },
        ChatCompletionRequestMessage::Assistant(a) => a
            .content
            .as_ref()
            .map(|c| match c {
                async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(
                    t,
                ) => t.clone(),
                async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Array(
                    _,
                ) => String::new(),
            })
            .unwrap_or_default(),
        ChatCompletionRequestMessage::Tool(t) => match &t.content {
            async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(s) => {
                s.clone()
            }
            async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(_) => {
                String::new()
            }
        },
        _ => String::new(),
    }
}

fn is_assistant(msg: &ChatCompletionRequestMessage) -> bool {
    matches!(msg, ChatCompletionRequestMessage::Assistant(_))
}

fn is_user(msg: &ChatCompletionRequestMessage) -> bool {
    matches!(msg, ChatCompletionRequestMessage::User(_))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_openai::types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
    };

    fn sys(text: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestSystemMessageArgs::default()
            .content(text)
            .build()
            .unwrap()
            .into()
    }
    fn user(text: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestUserMessageArgs::default()
            .content(text)
            .build()
            .unwrap()
            .into()
    }
    fn asst(text: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestAssistantMessageArgs::default()
            .content(text)
            .build()
            .unwrap()
            .into()
    }
    fn tool(id: &str, text: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestToolMessageArgs::default()
            .tool_call_id(id)
            .content(text)
            .build()
            .unwrap()
            .into()
    }

    #[test]
    fn should_compact_thresholds() {
        assert!(!should_compact(50_000, 10));
        assert!(!should_compact(200_000, 5)); // too few messages
        assert!(should_compact(100_000, 7));
        assert!(should_compact(200_000, 20));
    }

    #[test]
    fn extract_key_context_file_paths() {
        let messages = vec![
            sys("system"),
            user("please fix core/src/orchestrator.rs"),
            asst("I'll look at core/src/utils.rs and core/Cargo.toml"),
        ];
        let ctx = extract_key_context(&messages);
        assert!(
            ctx.file_paths
                .contains(&"core/src/orchestrator.rs".to_string())
        );
        assert!(ctx.file_paths.contains(&"core/src/utils.rs".to_string()));
        assert!(ctx.file_paths.contains(&"core/Cargo.toml".to_string()));
    }

    #[test]
    fn extract_key_context_pending_work() {
        let messages = vec![
            sys("system"),
            asst("Done with step 1. TODO: implement step 2\nNext: write tests"),
        ];
        let ctx = extract_key_context(&messages);
        assert_eq!(ctx.pending_work.len(), 2);
        assert!(ctx.pending_work[0].contains("TODO"));
        assert!(ctx.pending_work[1].contains("Next"));
    }

    #[test]
    fn extract_key_context_recent_requests() {
        let messages = vec![
            sys("system"),
            user("request 1"),
            asst("done"),
            user("request 2"),
            asst("done"),
            user("request 3"),
            asst("done"),
            user("request 4"),
        ];
        let ctx = extract_key_context(&messages);
        assert_eq!(ctx.recent_requests.len(), 3);
        assert_eq!(ctx.recent_requests[0], "request 2");
        assert_eq!(ctx.recent_requests[2], "request 4");
    }

    #[test]
    fn compress_summary_within_budget() {
        let summary = "Conversation summary:\n- Current work: fixing bugs\n- Key files: main.rs";
        let budget = CompressionBudget::default();
        let compressed = compress_summary(summary, &budget);
        assert_eq!(compressed, summary, "already within budget, no change");
    }

    #[test]
    fn compress_summary_drops_low_priority_first() {
        let mut lines = vec!["Conversation summary:".to_string()];
        lines.push("- Current work: important stuff".to_string());
        // Add many P3 lines to exceed budget
        for i in 0..30 {
            lines.push(format!("some filler text line {}", i));
        }
        let summary = lines.join("\n");
        let budget = CompressionBudget {
            max_chars: 200,
            max_lines: 5,
            max_line_chars: 160,
        };
        let compressed = compress_summary(&summary, &budget);
        // P0 lines must survive
        assert!(compressed.contains("Conversation summary:"));
        assert!(compressed.contains("Current work:"));
        // P3 filler mostly dropped
        assert!(compressed.lines().count() <= 5);
    }

    #[test]
    fn format_compaction_summary_with_existing() {
        let ctx = KeyContext {
            file_paths: vec!["main.rs".into()],
            pending_work: vec!["write tests".into()],
            tool_names: vec!["web_search".into()],
            recent_requests: vec!["fix the bug".into()],
            current_work: Some("debugging".into()),
        };
        let summary = format_compaction_summary(&ctx, Some("Old context here"));
        assert!(summary.contains("Current work: debugging"));
        assert!(summary.contains("write tests"));
        assert!(summary.contains("Previously compacted context:"));
        assert!(summary.contains("Old context here"));
    }

    #[test]
    fn line_priority_classification() {
        assert_eq!(line_priority("Conversation summary:"), 0);
        assert_eq!(line_priority("- Current work: foo"), 0);
        assert_eq!(line_priority("- Pending work:"), 0);
        assert_eq!(line_priority("Section header:"), 1);
        assert_eq!(line_priority("- bullet item"), 2);
        assert_eq!(line_priority("  - nested bullet"), 2);
        assert_eq!(line_priority("random text"), 3);
    }

    #[test]
    fn extract_key_context_tool_names() {
        use async_openai::types::chat::{
            ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls, FunctionCall,
        };
        // Build an assistant message with tool_calls
        let mut asst_msg = ChatCompletionRequestAssistantMessageArgs::default()
            .content("I'll search for that")
            .build()
            .unwrap();
        asst_msg.tool_calls = Some(vec![
            ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                id: "call_1".into(),
                function: FunctionCall {
                    name: "web_search".into(),
                    arguments: "{}".into(),
                },
            }),
            ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                id: "call_2".into(),
                function: FunctionCall {
                    name: "recall_memory".into(),
                    arguments: "{}".into(),
                },
            }),
        ]);
        let messages = vec![
            sys("system"),
            user("search for something"),
            asst_msg.into(),
            tool("call_1", "search results"),
            tool("call_2", "memory results"),
        ];
        let ctx = extract_key_context(&messages);
        assert!(ctx.tool_names.contains(&"web_search".to_string()));
        assert!(ctx.tool_names.contains(&"recall_memory".to_string()));
    }
}
