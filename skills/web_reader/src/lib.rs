#![allow(unsafe_code)] // #[no_mangle] on WASM entry point
// Aegis Runtime Skill: web_reader
//
// Accepts a JSON payload with a "url" field, calls aegis::host_fetch to request
// the URL (subject to S-LEG domain whitelist), reads the response body, and
// writes the result back as JSON output.
//
// Features:
// - Dirty data circuit breaker: detects anti-bot walls, login gates, empty content
// - Jina Reader fallback: for JS-rendered/SPA pages, retries via r.jina.ai
// - HTML-to-text cleaning for better LLM consumption

use aegis_runtime_wasm_core as core;

#[derive(serde::Deserialize)]
struct Payload {
    url: String,
}

#[derive(serde::Serialize)]
struct Output {
    status: &'static str,
    fetch_code: i32,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    via: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

/// Maximum body size returned to the LLM (16KB — enough for article content,
/// small enough to avoid context overflow).
const MAX_BODY_BYTES: usize = 16_384;

/// Minimum useful text length after cleaning. Below this, the page is likely
/// a login wall or anti-bot page.
const MIN_USEFUL_TEXT_LEN: usize = 80;

// ── Dirty data detection ────────────────────────────────────────────────────

/// Anti-bot wall indicators (JS required, Cloudflare challenge, etc.)
const ANTI_BOT_PATTERNS: &[&str] = &[
    "enable javascript",
    "please enable js",
    "javascript is required",
    "javascript is disabled",
    "you need to enable javascript",
    "checking your browser",
    "cf-browser-verification",
    "challenge-platform",
    "just a moment",
    "<noscript>",
    "ray id",
];

/// Login/paywall wall indicators — shorter patterns for broader matching.
/// Uses multi-hit detection (≥2 patterns = login wall) to avoid false positives.
const LOGIN_WALL_PATTERNS: &[&str] = &[
    "log in",
    "sign in",
    "sign up",
    "create an account",
    "create account",
    "login required",
    "you must be logged in",
    "subscribe to read",
    "subscribe to continue",
    "this content is available to subscribers",
    "please enable cookies",
];

/// ASCII case-insensitive substring search.
///
/// Patterns are ASCII, so this matches `haystack.to_ascii_lowercase().contains(needle)`
/// without allocating a second copy of the page.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return n.is_empty();
    }
    h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// Check if the fetched content is garbage (anti-bot wall, login gate, or empty).
/// Returns a human-readable reason if garbage detected, None if content looks valid.
fn detect_garbage(content: &str) -> Option<&'static str> {
    let text_only = strip_html_tags_simple(content);
    let text_trimmed = text_only.trim();

    // Empty or near-empty after tag stripping
    if text_trimmed.len() < MIN_USEFUL_TEXT_LEN {
        return Some("page returned no meaningful text content (likely requires JavaScript)");
    }

    // Anti-bot wall
    for pattern in ANTI_BOT_PATTERNS {
        if contains_ignore_ascii_case(content, pattern) {
            // Only flag if the page is short — long pages may legitimately contain
            // "noscript" as a fallback alongside real content
            if text_trimmed.len() < 500 {
                return Some("page is blocked by anti-bot protection (Cloudflare/JS challenge)");
            }
        }
    }

    // Login wall — count how many distinct patterns match. A real login wall
    // hits multiple patterns (e.g. "log in" + "sign up" + "create an account").
    // A real article might mention "sign up" once. Threshold: ≥2 distinct patterns.
    let login_hits: usize = LOGIN_WALL_PATTERNS
        .iter()
        .filter(|p| contains_ignore_ascii_case(content, p))
        .count();
    if login_hits >= 2 {
        return Some("page requires login or subscription to view content");
    }
    // Single login pattern + short page = also a login wall
    if login_hits == 1 && text_trimmed.len() < 1000 {
        return Some("page requires login or subscription to view content");
    }

    None
}

// ── Simple HTML→text conversion (Wasm-compatible, no regex) ─────────────────

/// Next `n` chars starting at `byte_i`, as a subslice. No heap allocation.
fn chars_at(s: &str, byte_i: usize, n: usize) -> Option<&str> {
    let mut end = byte_i;
    for _ in 0..n {
        let ch = s[end..].chars().next()?;
        end += ch.len_utf8();
    }
    Some(&s[byte_i..end])
}

fn advance_chars(s: &str, byte_i: &mut usize, char_i: &mut usize, n: usize) {
    for _ in 0..n {
        let ch = s[*byte_i..].chars().next().expect("char index in range");
        *byte_i += ch.len_utf8();
        *char_i += 1;
    }
}

/// Strip HTML tags to extract text. Scans the page once and keeps at most
/// [`MAX_BODY_BYTES`] of text.
///
/// The previous implementation collected every scalar into `Vec<char>` (4 bytes
/// each) before the 16 KiB output cap. Under the skill's 4 MiB linear memory
/// that allocation aborted the guest (`panic=abort`) on ordinary article HTML.
fn strip_html_tags_simple(html: &str) -> String {
    let total = html.chars().count();
    let mut result = String::with_capacity((html.len() / 2).min(MAX_BODY_BYTES));
    let mut byte_i = 0usize;
    let mut char_i = 0usize;
    let mut in_tag = false;
    let mut in_script = false;
    let mut omitted_bytes = 0usize;

    let push_text = |result: &mut String, omitted: &mut usize, ch: char| {
        if result.len() >= MAX_BODY_BYTES {
            *omitted += ch.len_utf8();
            return;
        }
        let next = result.len().saturating_add(ch.len_utf8());
        if next > MAX_BODY_BYTES {
            *omitted += ch.len_utf8();
            return;
        }
        result.push(ch);
    };

    while char_i < total {
        let ch = html[byte_i..].chars().next().expect("char index in range");
        let ch_len = ch.len_utf8();

        if in_script {
            if ch == '<' && char_i + 8 < total {
                let lookahead = chars_at(html, byte_i, 9).expect("9 chars in range");
                if lookahead == "</script>" || lookahead == "</SCRIPT>" {
                    in_script = false;
                    advance_chars(html, &mut byte_i, &mut char_i, 9);
                    continue;
                }
            }
            byte_i += ch_len;
            char_i += 1;
            continue;
        }

        if ch == '<' {
            if char_i + 7 < total {
                let lookahead = chars_at(html, byte_i, 7).expect("7 chars in range");
                if lookahead.eq_ignore_ascii_case("<script") {
                    in_script = true;
                    advance_chars(html, &mut byte_i, &mut char_i, 7);
                    continue;
                }
            }
            if char_i + 6 < total {
                let lookahead = chars_at(html, byte_i, 6).expect("6 chars in range");
                if lookahead.eq_ignore_ascii_case("<style") {
                    advance_chars(html, &mut byte_i, &mut char_i, 1);
                    while char_i < total {
                        let cur = html[byte_i..].chars().next().expect("char index in range");
                        if cur == '<' && char_i + 8 < total {
                            let end = chars_at(html, byte_i, 8).expect("8 chars in range");
                            if end.eq_ignore_ascii_case("</style>") {
                                advance_chars(html, &mut byte_i, &mut char_i, 8);
                                break;
                            }
                        }
                        byte_i += cur.len_utf8();
                        char_i += 1;
                    }
                    continue;
                }
            }
            in_tag = true;
            if char_i + 2 < total {
                let next2 = chars_at(html, byte_i + ch_len, 2).expect("2 chars in range");
                let next2_lower = next2.to_ascii_lowercase();
                if next2_lower.starts_with('p')
                    || next2_lower.starts_with('h')
                    || next2_lower.starts_with("br")
                    || next2_lower.starts_with("di")
                    || next2_lower.starts_with("li")
                    || next2_lower.starts_with("tr")
                {
                    // Match the previous scanner: one newline per block tag, not a run.
                    if omitted_bytes == 0 && !result.ends_with('\n') {
                        push_text(&mut result, &mut omitted_bytes, '\n');
                    }
                }
            }
            byte_i += ch_len;
            char_i += 1;
            continue;
        }

        if ch == '>' && in_tag {
            in_tag = false;
            byte_i += ch_len;
            char_i += 1;
            continue;
        }

        if !in_tag {
            push_text(&mut result, &mut omitted_bytes, ch);
        }
        byte_i += ch_len;
        char_i += 1;
    }

    // Decode basic HTML entities
    result = result.replace("&amp;", "&");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&quot;", "\"");
    result = result.replace("&#39;", "'");
    result = result.replace("&nbsp;", " ");

    let result = collapse_whitespace(&result);
    if omitted_bytes == 0 {
        return result;
    }
    let total_bytes = result.len().saturating_add(omitted_bytes);
    let end = result.floor_char_boundary(MAX_BODY_BYTES.min(result.len()));
    format!(
        "{}...\n[truncated, {} chars total]",
        &result[..end],
        total_bytes
    )
}

/// Collapse runs of whitespace and blank lines (max 2 consecutive newlines).
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut newline_count = 0u32;

    for ch in text.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push('\n');
            }
        } else if ch == '\r' {
            // skip \r
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }
    result
}

/// Truncate text to MAX_BODY_BYTES, preserving UTF-8 boundary.
fn truncate_body(text: String) -> String {
    if text.len() <= MAX_BODY_BYTES {
        return text;
    }
    let end = text.floor_char_boundary(MAX_BODY_BYTES);
    format!(
        "{}...\n[truncated, {} chars total]",
        &text[..end],
        text.len()
    )
}

// ── Fetch helpers ───────────────────────────────────────────────────────────

/// Largest host body copied into this skill.
///
/// `web_reader` is capped at 4 MiB of linear memory, of which the module image
/// already occupies about 1.1 MiB. A single buffer of this size fits; the old
/// `Vec<char>` expansion did not, and a second full copy of a ~2 MiB body did
/// not either. Anything larger is refused before allocation so the guest
/// returns a normal error instead of aborting.
const MAX_FETCH_BYTES: usize = 2 * 1024 * 1024;

/// Perform a fetch via the host proxy. Returns (fetch_code, body_option).
/// fetch_code: 0 on success, -1 on host/domain failure, -2 when the body
/// exceeds [`MAX_FETCH_BYTES`].
fn do_fetch(url: &str) -> (i32, Option<String>) {
    match core::fetch_url_limited(url, MAX_FETCH_BYTES) {
        Ok(bytes) => {
            // Valid UTF-8 reuses the fetch buffer. Lossy conversion only
            // allocates when the body is not UTF-8.
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
            };
            (0, Some(text))
        }
        Err("response too large") => (-2, None),
        Err(_) => (-1, None),
    }
}

/// Build Jina Reader URL for a given target URL.
fn jina_reader_url(url: &str) -> String {
    format!("https://r.jina.ai/{}", url)
}

/// Jina returns markdown. Keep it when it has real text; otherwise None.
fn jina_fallback(url: &str) -> Option<String> {
    let (jina_code, jina_body) = do_fetch(&jina_reader_url(url));
    if jina_code < 0 {
        return None;
    }
    let jina_body = jina_body.unwrap_or_default();
    if jina_body.trim().len() < MIN_USEFUL_TEXT_LEN {
        return None;
    }
    Some(truncate_body(jina_body))
}

fn write_output(output: &Output) {
    let output_json = serde_json::to_vec(output).unwrap_or_default();
    core::write_output_bytes(output_json);
}

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    // 1–2. Read and parse payload
    let payload: Payload = match core::read_payload() {
        Ok(p) => p,
        Err(_) => {
            core::SkillError::new(
                core::ErrorKind::InvalidInput,
                "invalid payload: expected {\"url\":\"...\"}",
            )
            .with_hint("Provide a JSON object with a \"url\" field")
            .write();
            return;
        }
    };

    // 3. Try direct fetch first
    let (fetch_code, raw_body) = do_fetch(&payload.url);

    if fetch_code == -1 {
        // Domain blocked or other host-level error
        write_output(&Output {
            status: "blocked",
            fetch_code,
            url: payload.url,
            body: None,
            body_len: None,
            via: None,
            warning: None,
        });
        return;
    }

    if fetch_code == -2 {
        // Body does not fit in skill memory. Jina often returns a smaller
        // rendering of the same URL; if that also does not fit, fail closed.
        if let Some(clean) = jina_fallback(&payload.url) {
            let body_len = clean.len();
            write_output(&Output {
                status: "ok",
                fetch_code: 0,
                url: payload.url,
                body: Some(clean),
                body_len: Some(body_len),
                via: Some("jina_reader"),
                warning: None,
            });
            return;
        }
        write_output(&Output {
            status: "fetch_failed",
            fetch_code,
            url: payload.url,
            body: None,
            body_len: None,
            via: None,
            warning: Some(
                "[CONTENT FETCH FAILED] page is larger than the skill memory budget. \
                 Jina Reader fallback also failed. \
                 Do NOT fabricate or guess content based on search snippets or training data. \
                 Report this failure to the user honestly."
                    .to_string(),
            ),
        });
        return;
    }

    let raw_body = raw_body.unwrap_or_default();
    let raw_len = raw_body.len();

    // 4. Check if direct fetch returned garbage
    if let Some(reason) = detect_garbage(&raw_body) {
        if let Some(clean) = jina_fallback(&payload.url) {
            let body_len = clean.len();
            write_output(&Output {
                status: "ok",
                fetch_code: 0,
                url: payload.url,
                body: Some(clean),
                body_len: Some(body_len),
                via: Some("jina_reader"),
                warning: None,
            });
            return;
        }

        // Both direct and Jina failed — return clear error blocking hallucination
        write_output(&Output {
            status: "fetch_failed",
            fetch_code,
            url: payload.url,
            body: None,
            body_len: Some(raw_len),
            via: None,
            warning: Some(format!(
                "[CONTENT FETCH FAILED] {reason}. \
                 Jina Reader fallback also failed. \
                 Do NOT fabricate or guess content based on search snippets or training data. \
                 Report this failure to the user honestly."
            )),
        });
        return;
    }

    // 5. Direct fetch succeeded with valid content — clean and return.
    // HTML stripping already caps the stored text at MAX_BODY_BYTES.
    let is_html = raw_body.contains('<') && (raw_body.contains("</") || raw_body.contains("/>"));
    let clean = if is_html {
        strip_html_tags_simple(&raw_body)
    } else {
        truncate_body(raw_body)
    };

    let body_len = clean.len();
    write_output(&Output {
        status: "ok",
        fetch_code,
        url: payload.url,
        body: Some(clean),
        body_len: Some(body_len),
        via: Some("direct"),
        warning: None,
    });
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Payload deserialization --

    #[test]
    fn payload_valid() {
        let p: Payload = serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
        assert_eq!(p.url, "https://example.com");
    }

    #[test]
    fn payload_empty_url() {
        let p: Payload = serde_json::from_str(r#"{"url":""}"#).unwrap();
        assert_eq!(p.url, "");
    }

    #[test]
    fn payload_missing_url_errors() {
        let r = serde_json::from_str::<Payload>(r#"{}"#);
        assert!(r.is_err(), "missing required field 'url' should fail");
    }

    #[test]
    fn payload_malformed_json_errors() {
        let r = serde_json::from_str::<Payload>(r#"not json"#);
        assert!(r.is_err());
    }

    #[test]
    fn test_strip_html_basic() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let result = strip_html_tags_simple(html);
        assert!(result.contains("Title"), "heading extracted");
        assert!(result.contains("Hello"), "paragraph text extracted");
        assert!(result.contains("world"), "nested tag text extracted");
        assert!(!result.contains('<'), "no HTML tags remain");
    }

    #[test]
    fn test_strip_html_script_removal() {
        let html = "<p>Before</p><script>alert('xss');</script><p>After</p>";
        let result = strip_html_tags_simple(html);
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn test_strip_html_style_removal() {
        let html = "<style>body{color:red}</style><p>Content</p>";
        let result = strip_html_tags_simple(html);
        assert!(result.contains("Content"));
        assert!(!result.contains("color:red"));
    }

    #[test]
    fn test_strip_html_entities() {
        let html = "A &amp; B &lt; C &gt; D &quot;E&quot;";
        let result = strip_html_tags_simple(html);
        assert!(result.contains("A & B < C > D \"E\""));
    }

    #[test]
    fn test_detect_garbage_antibot() {
        let html =
            "<html><body><noscript>Please enable JavaScript to continue</noscript></body></html>";
        assert!(detect_garbage(html).is_some());
    }

    #[test]
    fn test_detect_garbage_cloudflare() {
        let html =
            "<html><body><p>Checking your browser before accessing the site.</p></body></html>";
        assert!(detect_garbage(html).is_some());
    }

    #[test]
    fn test_detect_garbage_login_wall() {
        let html = "<html><body><p>Log in to X to see this content</p></body></html>";
        assert!(detect_garbage(html).is_some());
    }

    #[test]
    fn test_detect_garbage_empty() {
        assert!(detect_garbage("").is_some());
        assert!(detect_garbage("<html><body></body></html>").is_some());
    }

    #[test]
    fn test_detect_garbage_valid_content() {
        let html = "<html><body><h1>Article Title</h1><p>This is a real article with substantial content about the topic at hand. It contains multiple sentences and real information.</p></body></html>";
        assert!(
            detect_garbage(html).is_none(),
            "valid content should not be flagged"
        );
    }

    #[test]
    fn test_detect_garbage_long_page_with_noscript() {
        // A long real page that happens to have a <noscript> fallback should NOT be flagged
        let mut html = String::from("<html><body><noscript>Enable JS</noscript>");
        for i in 0..50 {
            html.push_str(&format!("<p>Paragraph {} with real content about the article topic and meaningful information.</p>", i));
        }
        html.push_str("</body></html>");
        assert!(
            detect_garbage(&html).is_none(),
            "long page with noscript should not be flagged"
        );
    }

    #[test]
    fn test_truncate_body_short() {
        let text = "Hello world".to_string();
        assert_eq!(truncate_body(text.clone()), text);
    }

    #[test]
    fn test_truncate_body_long() {
        let text = "A".repeat(MAX_BODY_BYTES + 1000);
        let result = truncate_body(text);
        assert!(result.contains("[truncated"));
        assert!(result.len() < MAX_BODY_BYTES + 200); // truncated part + marker
    }

    #[test]
    fn test_jina_reader_url() {
        assert_eq!(
            jina_reader_url("https://x.com/elonmusk"),
            "https://r.jina.ai/https://x.com/elonmusk"
        );
    }

    #[test]
    fn test_detect_garbage_jina_twitter_login_wall() {
        // Simulates what Jina Reader actually returns for x.com/elonmusk:
        // A large page (9KB+) with navigation chrome but "Log in" + "Sign up" = login wall
        let mut page = String::from("Elon Musk (@elonmusk) / X\n\n");
        page.push_str("Don't miss what's happening\nPeople on X are the first to know.\n");
        page.push_str("[Log in](https://x.com/login)\n");
        page.push_str("[Sign up](https://x.com/i/flow/signup)\n\n");
        // Add lots of navigation boilerplate
        for i in 0..100 {
            page.push_str(&format!(
                "Navigation item {} | Trending topic discussion | Sidebar content here\n",
                i
            ));
        }
        assert!(
            detect_garbage(&page).is_some(),
            "Jina Twitter login wall should be detected (multiple login patterns)"
        );
    }

    #[test]
    fn test_detect_garbage_real_article_with_signup() {
        // A real article that happens to mention "sign up" once should NOT be flagged
        let mut article = String::from("<html><body>");
        article.push_str("<h1>How to Build a Startup</h1>");
        for i in 0..30 {
            article.push_str(&format!(
                "<p>Paragraph {} discusses startup strategies, venture capital, and team building.</p>",
                i
            ));
        }
        article.push_str("<p>Sign up for our newsletter to learn more.</p>");
        article.push_str("</body></html>");
        assert!(
            detect_garbage(&article).is_none(),
            "real article with single 'sign up' mention should NOT be flagged"
        );
    }

    #[test]
    fn test_strip_preserves_chinese() {
        let html = "<p>实时金价 <b>每盎司</b> 上涨</p>";
        let result = strip_html_tags_simple(html);
        assert!(result.contains("实时金价"));
        assert!(result.contains("每盎司"));
        assert!(result.contains("上涨"));
        assert!(!result.contains('<'));
    }

    #[test]
    fn test_strip_large_html_truncates_without_char_vector() {
        // Bigger than the old Vec<char> abort threshold (~400 KiB of ASCII
        // under a 4 MiB guest limit). The scanner must keep the start of the
        // article and a truncation marker, and must not build Vec<char>.
        let mut html = String::from("<html><body><h1>Gold spot</h1>");
        let para = "<p>Gold spot price moved with inventory data and London fix commentary for traders.</p>";
        while html.len() < 500_000 {
            html.push_str(para);
        }
        html.push_str("</body></html>");
        let result = strip_html_tags_simple(&html);
        assert!(result.contains("Gold spot"), "keeps the start of the page");
        assert!(result.contains("[truncated"));
        assert!(
            result.len() < MAX_BODY_BYTES + 80,
            "stored text stays near the 16 KiB cap, got {}",
            result.len()
        );
        assert!(
            detect_garbage(&html).is_none(),
            "long article is not garbage"
        );
    }

    #[test]
    fn test_contains_ignore_ascii_case() {
        assert!(contains_ignore_ascii_case(
            "Please Enable JavaScript",
            "enable javascript"
        ));
        assert!(!contains_ignore_ascii_case("article body", "log in"));
    }

    #[test]
    fn test_collapse_whitespace() {
        let text = "Line 1\n\n\n\n\nLine 2\n\n\nLine 3";
        let result = collapse_whitespace(text);
        assert!(!result.contains("\n\n\n"), "no triple newlines");
        assert!(result.contains("Line 1"));
        assert!(result.contains("Line 2"));
        assert!(result.contains("Line 3"));
    }
}
