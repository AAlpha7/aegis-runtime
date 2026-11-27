//! Shared utility functions used across the orchestrator and other modules.

use anyhow::Result;
use async_openai::types::chat::ChatCompletionRequestMessage;
use serde_json::Value;

use crate::memory;

// Thread-local flag to bypass expensive BPE tokenization in tests.
// When true, uses fast byte-length heuristic (bytes/4) instead of tiktoken.
// Set via `set_fast_token_mode(true)` at test setup; automatically scoped to the thread.
thread_local! {
    static FAST_TOKEN_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Enable or disable fast (heuristic) token estimation for the current thread.
/// Intended for integration tests where BPE precision doesn't matter.
/// No-op in production (fast mode is always off by default).
pub fn set_fast_token_mode(enabled: bool) {
    FAST_TOKEN_MODE.with(|f| f.set(enabled));
}

/// Flat token budget charged per image part. Providers bill images per tile
/// (OpenAI high-detail tops out around 1.1K tokens for a phone photo; xAI is
/// in the same range), never per base64 character.
const IMAGE_PART_TOKEN_ESTIMATE: usize = 1_100;
/// Flat budget per audio part (a webapp/Telegram voice note of under a minute).
const AUDIO_PART_TOKEN_ESTIMATE: usize = 1_000;
/// Flat budget per file part (documents are summarized/extracted before they
/// reach the model today, so this is a ceiling, not a measurement).
const FILE_PART_TOKEN_ESTIMATE: usize = 2_000;
/// Above this many bytes the BPE regex (fancy_regex) can overflow its stack
/// on a single long run; the byte heuristic is accurate enough there.
const BPE_MAX_INPUT_BYTES: usize = 256 * 1024;

/// Estimate total token count of all messages using cl100k_base tokenizer.
/// Image/audio content parts are charged a flat per-part budget (their data
/// URLs are not text). Falls back to byte-length / 4 if tokenizer
/// initialization fails, the input is oversized, or fast mode is enabled.
pub fn estimate_messages_tokens(messages: &[ChatCompletionRequestMessage]) -> usize {
    let fast = FAST_TOKEN_MODE.with(|f| f.get());

    // Use thread-local tokenizer to avoid repeated initialization
    thread_local! {
        static BPE: Option<tiktoken_rs::CoreBPE> = tiktoken_rs::cl100k_base().ok();
    }

    let count_text = |text: &str| -> usize {
        if fast || text.len() > BPE_MAX_INPUT_BYTES {
            return text.len() / 4;
        }
        BPE.with(|bpe| match bpe {
            Some(tokenizer) => tokenizer.encode_with_special_tokens(text).len(),
            None => text.len() / 4,
        })
    };

    messages
        .iter()
        .map(|m| {
            use async_openai::types::chat::{
                ChatCompletionRequestUserMessageContent as UserContent,
                ChatCompletionRequestUserMessageContentPart as Part,
            };
            let body = match m {
                ChatCompletionRequestMessage::User(user) => match &user.content {
                    UserContent::Array(parts) => parts
                        .iter()
                        .map(|part| match part {
                            Part::Text(t) => count_text(&t.text),
                            Part::ImageUrl(_) => IMAGE_PART_TOKEN_ESTIMATE,
                            Part::InputAudio(_) => AUDIO_PART_TOKEN_ESTIMATE,
                            Part::File(_) => FILE_PART_TOKEN_ESTIMATE,
                        })
                        .sum(),
                    UserContent::Text(text) => count_text(text),
                },
                other => count_text(&serde_json::to_string(other).unwrap_or_default()),
            };
            body + 4 // +4 for message overhead
        })
        .sum()
}

/// Parse a timezone string into seconds-east-of-UTC offset.
///
/// Supports formats:
/// - `+08:00`, `-05:00` (fixed offset)
/// - `+8`, `-5` (shorthand hours)
/// - `UTC`, `GMT` (zero offset)
/// - Common abbreviation→offset mappings (CST=+8, EST=-5, PST=-8, JST=+9, etc.)
pub fn parse_timezone_offset(tz: &str) -> Result<i32, ()> {
    let tz = tz.trim();

    // Fixed offset: +08:00, -05:00
    if (tz.starts_with('+') || tz.starts_with('-')) && tz.contains(':') {
        let sign: i32 = if tz.starts_with('-') { -1 } else { 1 };
        let parts: Vec<&str> = tz[1..].split(':').collect();
        if parts.len() == 2
            && let (Ok(h), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>())
            && h <= 14
            && m < 60
        {
            return Ok(sign * (h * 3600 + m * 60));
        }
    }

    // Shorthand: +8, -5
    if (tz.starts_with('+') || tz.starts_with('-')) && !tz.contains(':') {
        let sign: i32 = if tz.starts_with('-') { -1 } else { 1 };
        if let Ok(h) = tz[1..].parse::<i32>()
            && h <= 14
        {
            return Ok(sign * h * 3600);
        }
    }

    // Named abbreviations
    match tz.to_uppercase().as_str() {
        "UTC" | "GMT" => Ok(0),
        "CST" => Ok(8 * 3600),        // China Standard Time
        "JST" => Ok(9 * 3600),        // Japan Standard Time
        "KST" => Ok(9 * 3600),        // Korea Standard Time
        "IST" => Ok(5 * 3600 + 1800), // India Standard Time
        "EST" => Ok(-5 * 3600),       // US Eastern
        "EDT" => Ok(-4 * 3600),       // US Eastern Daylight
        "CST6" => Ok(-6 * 3600),      // US Central (disambiguated)
        "CDT" => Ok(-5 * 3600),       // US Central Daylight
        "MST" => Ok(-7 * 3600),       // US Mountain
        "MDT" => Ok(-6 * 3600),       // US Mountain Daylight
        "PST" => Ok(-8 * 3600),       // US Pacific
        "PDT" => Ok(-7 * 3600),       // US Pacific Daylight
        "CET" => Ok(3600),            // Central European
        "CEST" => Ok(2 * 3600),       // Central European Summer
        "EET" => Ok(2 * 3600),        // Eastern European
        "EEST" => Ok(3 * 3600),       // Eastern European Summer
        "AEST" => Ok(10 * 3600),      // Australian Eastern
        "AEDT" => Ok(11 * 3600),      // Australian Eastern Daylight
        "NZST" => Ok(12 * 3600),      // New Zealand Standard
        "NZDT" => Ok(13 * 3600),      // New Zealand Daylight
        "SGT" => Ok(8 * 3600),        // Singapore
        "HKT" => Ok(8 * 3600),        // Hong Kong
        "WIB" => Ok(7 * 3600),        // Western Indonesia
        "ICT" => Ok(7 * 3600),        // Indochina
        _ => Err(()),
    }
}

/// Fallback message when `sanitize_model_xml` strips all content.
pub(crate) const SANITIZE_FALLBACK_MSG: &str =
    "(I attempted to use a tool but couldn't complete the action. Please try again.)";

/// UTF-8 safe string truncation for logging. Returns a &str that ends on a
/// char boundary, never panicking on multi-byte characters (e.g. Chinese).
pub(crate) fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backward from max_bytes to find a char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// UTF-8 safe string truncation with ellipsis suffix for user-visible messages.
/// Returns an owned String. If truncated, appends "..." to indicate continuation.
pub(crate) fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Unicode-safe character truncation. Takes at most `max_chars` Unicode scalar
/// values and returns an owned String. This never splits multi-byte characters.
/// Prefer this over `.chars().take(N).collect()` for readability and consistency.
pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

/// Extract text content from a ChatCompletionResponseMessage.
pub(crate) fn extract_text_content(
    msg: &async_openai::types::chat::ChatCompletionResponseMessage,
) -> Option<String> {
    msg.content
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

/// Strip model-specific raw XML that some models (e.g. minimax) emit in text
/// when they attempt function calls without the tools API.
/// Example: `<minimax:tool_call>\n<invoke name="foo">...</invoke>\n</minimax:tool_call>`
pub(crate) fn sanitize_model_xml(text: &str) -> String {
    // Fast path: no XML-like tags at all
    if !text.contains("<minimax:") && !text.contains("<tool_call>") {
        return text.to_string();
    }

    // Remove <minimax:tool_call>...</minimax:tool_call> blocks (including nested content)
    let mut result = text.to_string();
    while let Some(start) = result.find("<minimax:tool_call>") {
        if let Some(end) = result.find("</minimax:tool_call>") {
            let tag_end = end + "</minimax:tool_call>".len();
            result = format!("{}{}", &result[..start], result[tag_end..].trim_start());
        } else {
            // Unclosed tag — remove from start to end of string
            result = result[..start].to_string();
            break;
        }
    }

    // Also handle generic <tool_call>...</tool_call> patterns
    while let Some(start) = result.find("<tool_call>") {
        if let Some(end) = result.find("</tool_call>") {
            let tag_end = end + "</tool_call>".len();
            result = format!("{}{}", &result[..start], result[tag_end..].trim_start());
        } else {
            result = result[..start].to_string();
            break;
        }
    }

    let trimmed = result.trim();
    if trimmed.is_empty() {
        SANITIZE_FALLBACK_MSG.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Case-insensitive escaping of `[TOOL_RESULT` and `[/TOOL_RESULT]` markers
/// in tool output strings, preventing prompt-injection boundary attacks that
/// use lowercase, mixed-case, or zero-width character variants.
///
/// The function strips Unicode zero-width characters (ZWJ, ZWNJ, ZWSP, BOM,
/// soft-hyphen, and \u{200B}--\u{200F} directional marks) before matching,
/// then performs case-insensitive replacement of both the opening and closing
/// marker patterns.
pub(crate) fn sanitize_tool_result_markers(input: &str) -> String {
    // Step 1: Strip zero-width / invisible characters that could be inserted
    // between marker characters to evade exact-match detection.
    let stripped: String = input
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200B}' // ZERO WIDTH SPACE
                | '\u{200C}' // ZERO WIDTH NON-JOINER
                | '\u{200D}' // ZERO WIDTH JOINER
                | '\u{200E}' // LEFT-TO-RIGHT MARK
                | '\u{200F}' // RIGHT-TO-LEFT MARK
                | '\u{FEFF}' // BOM / ZERO WIDTH NO-BREAK SPACE
                | '\u{00AD}' // SOFT HYPHEN
                | '\u{2060}' // WORD JOINER
                | '\u{2061}' // FUNCTION APPLICATION
                | '\u{2062}' // INVISIBLE TIMES
                | '\u{2063}' // INVISIBLE SEPARATOR
                | '\u{2064}' // INVISIBLE PLUS
            )
        })
        .collect();

    // Step 2: Case-insensitive replacement via byte scanning.
    // We search for two patterns (case-insensitive):
    //   "[tool_result"    -> "[ TOOL_RESULT (escaped)"
    //   "[/tool_result]"  -> "[ /TOOL_RESULT (escaped)]"
    //
    // Scan through the string; at each position check if the remaining bytes
    // start with either pattern (case-folded). If so, emit the replacement
    // and skip past the matched bytes.
    let bytes = stripped.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + 64);
    let mut i = 0;

    let open_pattern = b"[tool_result"; // 12 bytes
    let close_pattern = b"[/tool_result]"; // 14 bytes

    while i < len {
        // Try close pattern first (longer, avoids partial open match eating it)
        if i + close_pattern.len() <= len
            && bytes[i..i + close_pattern.len()]
                .iter()
                .zip(close_pattern.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            out.push_str("[ /TOOL_RESULT (escaped)]");
            i += close_pattern.len();
        } else if i + open_pattern.len() <= len
            && bytes[i..i + open_pattern.len()]
                .iter()
                .zip(open_pattern.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            out.push_str("[ TOOL_RESULT (escaped)");
            i += open_pattern.len();
        } else {
            // Walk to the next char boundary to avoid splitting multi-byte chars.
            let ch = stripped[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

/// Classify a Wasm skill execution error into a category with a retry hint.
/// Returns `(error_kind, hint)`.
pub(crate) fn classify_skill_error(err: &str) -> (&'static str, &'static str) {
    let lower = err.to_lowercase();

    if lower.contains("connection refused")
        || lower.contains("dns")
        || lower.contains("name resolution")
        || lower.contains("network")
        || lower.contains("connect error")
    {
        ("network", "Transient network error — retry may succeed")
    } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("deadline")
    {
        (
            "timeout",
            "Request timed out — retry with simpler query or try again later",
        )
    } else if lower.contains("fuel exhausted") || lower.contains("fuel") {
        (
            "resource",
            "Wasm execution exceeded CPU fuel limit — do not retry with same input",
        )
    } else if lower.contains("memory")
        && (lower.contains("limit") || lower.contains("exceeded") || lower.contains("oom"))
    {
        (
            "resource",
            "Wasm execution exceeded memory limit — do not retry with same input",
        )
    } else if lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("forbidden")
    {
        ("permission", "Permission denied — do not retry")
    } else {
        (
            "bug",
            "Unexpected skill error — do not retry, report to user",
        )
    }
}

/// Build a sanitized JSON error response for tool results returned to the LLM.
///
/// Logs the full internal error at `warn` level, then returns a generic
/// user/LLM-facing message without Rust error chain details.
pub(crate) fn sanitized_tool_error(
    tool: &str,
    description: &str,
    e: &dyn std::fmt::Display,
    error_kind: &str,
    retryable: bool,
    hint: Option<&str>,
) -> String {
    tracing::warn!(tool, error = %e, "Tool execution failed");
    let mut json = serde_json::json!({
        "status": "error",
        "error": description,
        "error_kind": error_kind,
        "retryable": retryable,
    });
    if let Some(h) = hint {
        json["hint"] = serde_json::json!(h);
    }
    json.to_string()
}

/// Shorthand for the very common "LLM sent invalid JSON as tool arguments" error.
pub(crate) fn json_parse_tool_error(tool: &str, e: &dyn std::fmt::Display) -> String {
    sanitized_tool_error(
        tool,
        "Invalid tool arguments / 工具参数无效",
        e,
        "invalid_input",
        true,
        Some("Check the tool schema and provide valid JSON arguments."),
    )
}

/// Classify an HTTP/network error into a category and retryability hint.
/// Returns `(category, retryable)`. Available for incremental adoption by callsites.
#[allow(dead_code)]
pub(crate) fn categorize_http_error(e: &anyhow::Error) -> (&'static str, bool) {
    let msg = e.to_string().to_lowercase();
    if msg.contains("timeout") || msg.contains("timed out") {
        ("timeout", true)
    } else if msg.contains("429") || msg.contains("rate limit") {
        ("rate_limited", true)
    } else if msg.contains("401") || msg.contains("unauthorized") {
        ("auth_error", false)
    } else if msg.contains("404") || msg.contains("not found") {
        ("not_found", false)
    } else if msg.contains("500") || msg.contains("502") || msg.contains("503") {
        ("server_error", true)
    } else if msg.contains("connection refused") {
        ("connection_refused", false)
    } else if msg.contains("dns") || msg.contains("resolve") {
        ("dns_error", false)
    } else {
        ("unknown", false)
    }
}

/// Convert a KnowledgeEntry to a JSON Value for tool responses.
pub(crate) fn kb_entry_to_json(e: &memory::KnowledgeEntry) -> Value {
    let mut j = serde_json::json!({
        "id": e.id,
        "category": e.category,
        "priority": e.priority,
        "status": e.status,
        "title": e.title,
        "body": e.body,
        "created_at": e.created_at,
    });
    let obj = j.as_object_mut().unwrap();
    if !e.tags.is_empty() {
        obj.insert("tags".into(), serde_json::json!(e.tags));
    }
    if let Some(ref d) = e.due_date {
        obj.insert("due_date".into(), serde_json::json!(d));
    }
    if let Some(ref w) = e.waiting_on {
        obj.insert("waiting_on".into(), serde_json::json!(w));
    }
    if !e.related_ids.is_empty() {
        obj.insert("related_ids".into(), serde_json::json!(e.related_ids));
    }
    if let Some(ref s) = e.source_summary {
        obj.insert("source_summary".into(), serde_json::json!(s));
    }
    j
}

// ── Cross-module shared utilities (consolidated Sprint 7) ─────────────────

/// Strip HTML tags for plain-text fallback.
///
/// Uses a simple state machine that's safe even when code blocks contain `<`
/// or `>`.  Also unescapes the three most common HTML entities.
pub(crate) fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Unescape HTML entities back to plain text
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Parse a timestamp that may be either RFC3339 (`"2026-04-11T12:34:56Z"`) or
/// SQLite's native `datetime('now')` format (`"2026-04-11 12:34:56"`, UTC).
/// Returns `None` on parse failure.
///
/// Needed because rows whose `created_at`/`updated_at` DEFAULT to
/// `datetime('now')` (SQLite format) coexist with values written via Rust
/// code, which tend to use RFC3339. See `feedback_sqlite_datetime_format.md`.
pub(crate) fn parse_timestamp_dual_fmt(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc());
    }
    None
}

/// Strip XML-like tags from text (same pattern as the orchestrator's
/// `sanitize_model_xml`), so stored text cannot smuggle fake `<system>` /
/// `<tool_call>` markers into a later LLM prompt or a Telegram/admin renderer.
pub(crate) fn strip_xml_tags(text: &str) -> String {
    const MAX_TAG_LEN: usize = 10_000;
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Consume everything until '>' or end-of-string, up to MAX_TAG_LEN
            let mut tag_content = String::new();
            let mut found_close = false;
            for inner in chars.by_ref() {
                if inner == '>' {
                    found_close = true;
                    break;
                }
                tag_content.push(inner);
                if tag_content.len() > MAX_TAG_LEN {
                    // Pathological input guard: stop consuming but still
                    // treat everything consumed so far as stripped content.
                    // Continue consuming until '>' to fully strip the tag.
                    for rest in chars.by_ref() {
                        if rest == '>' {
                            found_close = true;
                            break;
                        }
                    }
                    break;
                }
            }
            if !found_close {
                // Not a complete tag (no closing '>'), keep the content
                result.push('<');
                result.push_str(&tag_content);
            }
            // If found_close, the entire <...> tag is stripped (dropped)
        } else {
            result.push(ch);
        }
    }
    result
}

/// Sanitize text before it is persisted and later echoed back to the owner.
///
/// Drops XML-like control tags and terminal control characters that could leak
/// into Telegram/admin renderers, then truncates on a char boundary. Unlike the
/// prompt-only sanitizers it adds no `[USER_INPUT]` wrapper — stored text should
/// stay readable.
pub(crate) fn sanitize_stored_text(text: &str, max_len: usize) -> String {
    let stripped = strip_xml_tags(text);
    let cleaned: String = stripped
        .chars()
        .filter(|&c| c == '\n' || c == '\r' || c == '\t' || (!c.is_ascii() || c >= ' '))
        .collect();

    truncate_with_ellipsis(cleaned.trim(), max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_messages_tokens_counts_images_flat_not_by_base64_length() {
        use async_openai::types::chat::{
            ChatCompletionRequestMessageContentPartImage,
            ChatCompletionRequestMessageContentPartText, ChatCompletionRequestUserMessage,
            ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
            ImageUrl,
        };
        // A 3 MB data URL (a phone photo). Tokenizing it as text is wrong
        // (providers bill images per tile, not per base64 char) and blew the
        // fancy_regex stack in production-shaped tests (2026-09-05).
        let data_url = format!("data:image/png;base64,{}", "A".repeat(3 * 1024 * 1024));
        let msg = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Array(vec![
                ChatCompletionRequestUserMessageContentPart::Text(
                    ChatCompletionRequestMessageContentPartText {
                        text: "看看这张图".to_string(),
                    },
                ),
                ChatCompletionRequestUserMessageContentPart::ImageUrl(
                    ChatCompletionRequestMessageContentPartImage {
                        image_url: ImageUrl {
                            url: data_url,
                            detail: None,
                        },
                    },
                ),
            ]),
            name: None,
        });
        let started = std::time::Instant::now();
        let tokens = estimate_messages_tokens(&[msg]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "image estimate must not tokenize megabytes of base64"
        );
        assert!(
            (500..=3_000).contains(&tokens),
            "one photo plus a few words should be a flat image budget, got {tokens}"
        );
    }

    #[test]
    fn estimate_messages_tokens_survives_huge_plain_text() {
        use async_openai::types::chat::{
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };
        // Pathological single-token run: fancy_regex overflows its stack on
        // multi-megabyte input, so oversized text must take the byte heuristic.
        let msg = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Text("A".repeat(3 * 1024 * 1024)),
            name: None,
        });
        let tokens = estimate_messages_tokens(&[msg]);
        assert!(tokens > 100_000, "must still count as huge, got {tokens}");
    }

    #[test]
    fn strip_html_tags_basic() {
        assert_eq!(strip_html_tags("<b>bold</b> text"), "bold text");
        assert_eq!(strip_html_tags("no tags"), "no tags");
    }

    #[test]
    fn strip_html_tags_code() {
        assert_eq!(
            strip_html_tags("<pre><code>fn main()</code></pre>"),
            "fn main()"
        );
    }

    #[test]
    fn strip_html_tags_entities() {
        assert_eq!(strip_html_tags("a &lt; b &amp; c &gt; d"), "a < b & c > d");
    }

    #[test]
    fn truncate_str_short() {
        assert_eq!(truncate_str("hello", 100), "hello");
    }

    #[test]
    fn truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_cuts() {
        assert_eq!(truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn truncate_str_multibyte() {
        let text = "你好世界"; // 12 bytes (3 per char)
        let result = truncate_str(text, 7);
        assert_eq!(result, "你好"); // 6 bytes — stopped at char boundary
    }

    // ── tool_dispatch! macro utility tests ──────────────────────────────

    /// json_parse_tool_error returns proper JSON with error status, kind, and retryable flag.
    #[test]
    fn json_parse_tool_error_returns_structured_json() {
        let err = serde_json::from_str::<serde_json::Value>("not valid json").unwrap_err();
        let result = json_parse_tool_error("web_search", &err);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["error_kind"], "invalid_input");
        assert_eq!(parsed["retryable"], true);
        assert!(
            parsed["error"]
                .as_str()
                .unwrap()
                .contains("Invalid tool arguments")
        );
        assert!(parsed["hint"].as_str().is_some());
    }

    /// json_parse_tool_error includes i18n bilingual message.
    #[test]
    fn json_parse_tool_error_bilingual() {
        let err = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let result = json_parse_tool_error("calendar_reader", &err);
        // Should contain both English and Chinese
        assert!(result.contains("Invalid tool arguments"));
        assert!(result.contains("工具参数无效"));
    }

    /// sanitized_tool_error without hint omits the hint field.
    #[test]
    fn sanitized_tool_error_no_hint() {
        let result = sanitized_tool_error("test_tool", "Test error", &"oops", "test", false, None);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["retryable"], false);
        assert!(parsed.get("hint").is_none() || parsed["hint"].is_null());
    }

    /// sanitized_tool_error with hint includes it.
    #[test]
    fn sanitized_tool_error_with_hint() {
        let result =
            sanitized_tool_error("test_tool", "Err", &"fail", "test", true, Some("Try again"));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["hint"], "Try again");
    }

    // ── sanitize_tool_result_markers ─────────────────────────────────

    #[test]
    fn sanitize_markers_uppercase() {
        let s = sanitize_tool_result_markers("[TOOL_RESULT skill=x]\ndata\n[/TOOL_RESULT]");
        assert!(!s.contains("[TOOL_RESULT"));
        assert!(!s.contains("[/TOOL_RESULT]"));
        assert!(s.contains("[ TOOL_RESULT (escaped)"));
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    #[test]
    fn sanitize_markers_lowercase() {
        let s = sanitize_tool_result_markers("[tool_result skill=x]\ndata\n[/tool_result]");
        assert!(!s.to_lowercase().contains("[tool_result"));
        assert!(s.contains("[ TOOL_RESULT (escaped)"));
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    #[test]
    fn sanitize_markers_mixed_case() {
        let s = sanitize_tool_result_markers("[Tool_Result foo]\n[/Tool_Result]");
        assert!(!s.contains("[Tool_Result"));
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    #[test]
    fn sanitize_markers_zero_width_space() {
        let s = sanitize_tool_result_markers("[/TOOL\u{200B}_RESULT]");
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    #[test]
    fn sanitize_markers_zwj_stuffed() {
        let evasion = "[\u{200D}/\u{200D}T\u{200D}o\u{200D}o\u{200D}l\u{200D}_\u{200D}R\u{200D}e\u{200D}s\u{200D}u\u{200D}l\u{200D}t\u{200D}]";
        let s = sanitize_tool_result_markers(evasion);
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    #[test]
    fn sanitize_markers_passthrough_normal() {
        let normal = r#"{"status":"ok","items":[1,2,3]}"#;
        assert_eq!(sanitize_tool_result_markers(normal), normal);
    }

    #[test]
    fn sanitize_markers_passthrough_bare_keyword() {
        // "TOOL_RESULT" without brackets should not be touched
        let s = "The TOOL_RESULT was successful";
        assert_eq!(sanitize_tool_result_markers(s), s);
    }

    #[test]
    fn sanitize_markers_cjk_preserved() {
        let input = "结果: [/TOOL_RESULT] 完成";
        let s = sanitize_tool_result_markers(input);
        assert!(s.contains("结果: "));
        assert!(s.contains(" 完成"));
        assert!(s.contains("[ /TOOL_RESULT (escaped)]"));
    }

    // ── categorize_http_error ───────────────────────────────────────

    #[test]
    fn categorize_http_timeout() {
        let e = anyhow::anyhow!("request timed out after 30s");
        assert_eq!(categorize_http_error(&e), ("timeout", true));
    }

    #[test]
    fn categorize_http_rate_limit() {
        let e = anyhow::anyhow!("HTTP 429 Too Many Requests");
        assert_eq!(categorize_http_error(&e), ("rate_limited", true));
    }

    #[test]
    fn categorize_http_auth() {
        let e = anyhow::anyhow!("401 Unauthorized");
        assert_eq!(categorize_http_error(&e), ("auth_error", false));
    }

    #[test]
    fn categorize_http_not_found() {
        let e = anyhow::anyhow!("HTTP 404 not found");
        assert_eq!(categorize_http_error(&e), ("not_found", false));
    }

    #[test]
    fn categorize_http_server_error() {
        let e = anyhow::anyhow!("HTTP 502 Bad Gateway");
        assert_eq!(categorize_http_error(&e), ("server_error", true));
    }

    #[test]
    fn categorize_http_dns() {
        let e = anyhow::anyhow!("dns resolution failed");
        assert_eq!(categorize_http_error(&e), ("dns_error", false));
    }

    #[test]
    fn categorize_http_unknown() {
        let e = anyhow::anyhow!("something weird happened");
        assert_eq!(categorize_http_error(&e), ("unknown", false));
    }

    #[test]
    fn strip_xml_tags_removes_tags() {
        assert_eq!(strip_xml_tags("Hello <b>world</b>!"), "Hello world!");
        assert_eq!(strip_xml_tags("No tags here"), "No tags here");
        assert_eq!(
            strip_xml_tags("<script>alert('xss')</script>"),
            "alert('xss')"
        );
    }

    #[test]
    fn sanitize_stored_text_strips_tags_and_control_chars() {
        assert_eq!(
            sanitize_stored_text("<system>ignore</system>\u{1b}[31mred\u{7}", 200),
            "ignore[31mred"
        );
        assert_eq!(sanitize_stored_text("  keep 中文  ", 200), "keep 中文");
    }

    #[test]
    fn sanitize_stored_text_truncates_on_char_boundary() {
        let out = sanitize_stored_text("中文中文中文", 6);
        assert!(out.chars().count() <= 6, "got {out}");
    }
}
