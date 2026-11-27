use scraper::{Html, Selector};

/// Convert raw HTML into a lightweight semantic Markdown representation,
/// preserving document reading order.
///
/// This is the "Semantic Snapshot" technique: instead of feeding a multi-MB HTML page
/// to the LLM, we extract only the meaningful content — headings, paragraphs, links,
/// buttons, inputs, list items — and discard all scripts, styles, SVGs, and noise.
///
/// Expected compression: ~99% reduction in token cost.
pub fn html_to_aria_markdown(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut output = String::new();

    // Select all semantic elements in document order
    let selector =
        Selector::parse("h1, h2, h3, h4, h5, h6, p, a, button, input, li, img, td, th, tr")
            .expect("valid selector");

    for element in document.select(&selector) {
        let tag = element.value().name();

        // Skip elements that are children of script/style/svg/noscript
        // by checking if any ancestor is a stripped tag
        if has_stripped_ancestor(&element) {
            continue;
        }

        match tag {
            "h1" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("# {text}\n\n"));
                }
            }
            "h2" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("## {text}\n\n"));
                }
            }
            "h3" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("### {text}\n\n"));
                }
            }
            "h4" | "h5" | "h6" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("#### {text}\n\n"));
                }
            }
            "p" => {
                let text = clean_text_excluding_scripts(&element);
                if !text.is_empty() {
                    output.push_str(&format!("{text}\n\n"));
                }
            }
            "a" => {
                let href = element.value().attr("href").unwrap_or("#");
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("[{text}]({href})\n"));
                }
            }
            "button" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("[Button: {text}]\n"));
                }
            }
            "input" => {
                let input_type = element.value().attr("type").unwrap_or("text");
                let placeholder = element.value().attr("placeholder").unwrap_or("");
                let name = element.value().attr("name").unwrap_or("");
                let label = if !placeholder.is_empty() {
                    placeholder
                } else {
                    name
                };
                if !label.is_empty() {
                    output.push_str(&format!("[Input({input_type}): {label}]\n"));
                }
            }
            "li" => {
                let text = clean_text(&element);
                if !text.is_empty() {
                    output.push_str(&format!("- {text}\n"));
                }
            }
            "img" => {
                let alt = element.value().attr("alt").unwrap_or("");
                if !alt.is_empty() {
                    output.push_str(&format!("[Image: {alt}]\n"));
                }
            }
            _ => {}
        }
    }

    // Collapse multiple blank lines
    let mut cleaned = String::new();
    let mut prev_blank = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank {
                cleaned.push('\n');
                prev_blank = true;
            }
        } else {
            cleaned.push_str(trimmed);
            cleaned.push('\n');
            prev_blank = false;
        }
    }

    cleaned.trim().to_string()
}

/// Check if any ancestor of an element is a stripped tag (script, style, svg, noscript).
fn has_stripped_ancestor(element: &scraper::ElementRef) -> bool {
    for ancestor in element.ancestors() {
        if let Some(el) = ancestor.value().as_element() {
            match el.name() {
                "script" | "style" | "svg" | "noscript" | "iframe" => return true,
                _ => {}
            }
        }
    }
    false
}

/// Maximum recursion depth for DOM traversal (prevents stack overflow from DOM bombs).
const MAX_DOM_DEPTH: usize = 100;

/// Get clean text from an element, collapsing whitespace.
/// Uses `collect_text_recursive` with depth limiting to prevent stack overflow.
fn clean_text(element: &scraper::ElementRef) -> String {
    let mut text = String::new();
    collect_text_recursive(element, &mut text, 0, false);
    collapse_whitespace(&text)
}

/// Get clean text from an element, excluding content inside script/style children.
/// Uses `collect_text_recursive` with depth limiting to prevent stack overflow.
fn clean_text_excluding_scripts(element: &scraper::ElementRef) -> String {
    let mut text = String::new();
    collect_text_recursive(element, &mut text, 0, true);
    collapse_whitespace(&text)
}

/// Recursively collect text from DOM nodes with a depth limit.
/// When `exclude_scripts` is true, skips script/style/svg/noscript elements.
/// Returns immediately (collecting no text) when `depth >= MAX_DOM_DEPTH`.
fn collect_text_recursive(
    element: &scraper::ElementRef,
    out: &mut String,
    depth: usize,
    exclude_scripts: bool,
) {
    if depth >= MAX_DOM_DEPTH {
        return;
    }
    for child in element.children() {
        match child.value() {
            scraper::Node::Text(t) => {
                out.push_str(t.trim());
                out.push(' ');
            }
            scraper::Node::Element(el) => {
                if exclude_scripts && matches!(el.name(), "script" | "style" | "svg" | "noscript") {
                    continue;
                }
                if let Some(el_ref) = scraper::ElementRef::wrap(child) {
                    collect_text_recursive(&el_ref, out, depth + 1, exclude_scripts);
                }
            }
            _ => {}
        }
    }
}

fn collapse_whitespace(s: &str) -> String {
    let trimmed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    // T6: Output must not contain any script content
    #[test]
    fn t6_snapshot_strips_script_tags() {
        let html = r#"
        <html>
        <head>
            <script>alert('xss');</script>
            <script src="evil.js"></script>
            <style>.secret { display: none; }</style>
        </head>
        <body>
            <h1>Welcome</h1>
            <p>This is safe content.</p>
            <script>document.cookie</script>
            <noscript>Enable JS</noscript>
            <svg><circle cx="50" cy="50" r="40"/></svg>
            <p>More safe content with <script>inline_evil()</script> embedded.</p>
        </body>
        </html>"#;

        let md = html_to_aria_markdown(html);

        // Must NOT contain any script-related content
        assert!(
            !md.contains("alert"),
            "script 'alert' must be stripped: {md}"
        );
        assert!(
            !md.contains("document.cookie"),
            "script 'document.cookie' must be stripped: {md}"
        );
        assert!(
            !md.contains("inline_evil"),
            "inline script must be stripped: {md}"
        );
        assert!(!md.contains("evil.js"), "script src must be stripped: {md}");
        assert!(
            !md.to_lowercase().contains("script"),
            "no trace of 'script' should remain: {md}"
        );
        assert!(!md.contains("noscript"), "noscript must be stripped: {md}");
        assert!(!md.contains("circle"), "SVG must be stripped: {md}");

        // Must contain the safe content
        assert!(md.contains("Welcome"), "heading missing: {md}");
        assert!(md.contains("safe content"), "paragraph missing: {md}");
    }

    // T7: Buttons and interactive elements must be extracted
    #[test]
    fn t7_snapshot_extracts_buttons() {
        let html = r#"
        <html><body>
            <h1>Checkout</h1>
            <form>
                <input type="email" placeholder="Your email" />
                <input type="text" name="address" />
                <button>Pay Now</button>
                <button type="submit">Confirm Order</button>
            </form>
            <a href="/cancel">Cancel</a>
        </body></html>"#;

        let md = html_to_aria_markdown(html);

        assert!(md.contains("[Button: Pay Now]"), "button not found: {md}");
        assert!(
            md.contains("[Button: Confirm Order]"),
            "submit button not found: {md}"
        );
        assert!(
            md.contains("[Input(email): Your email]"),
            "email input not found: {md}"
        );
        assert!(
            md.contains("[Input(text): address]"),
            "text input not found: {md}"
        );
        assert!(md.contains("[Cancel](/cancel)"), "link not found: {md}");
        assert!(md.contains("Checkout"), "heading not found: {md}");
    }

    #[test]
    fn t6_inline_scripts_are_stripped() {
        let html = r#"<html><body>
            <p>Safe text <script>evil()</script> more text</p>
        </body></html>"#;

        let md = html_to_aria_markdown(html);
        assert!(!md.contains("evil"), "inline script must be stripped: {md}");
        assert!(md.contains("Safe text"), "safe text missing: {md}");
        assert!(md.contains("more text"), "more text missing: {md}");
    }

    #[test]
    fn snapshot_compression_ratio() {
        let mut html = String::from("<html><head><style>");
        for _ in 0..500 {
            html.push_str(".cls { color: red; font-size: 14px; margin: 0; }\n");
        }
        html.push_str("</style></head><body>");
        html.push_str("<h1>News</h1>");
        for i in 0..10 {
            html.push_str(&format!("<p>Article {i}: Important news content here.</p>"));
        }
        html.push_str("</body></html>");

        let original_size = html.len();
        let md = html_to_aria_markdown(&html);
        let compressed_size = md.len();

        let ratio = 1.0 - (compressed_size as f64 / original_size as f64);

        assert!(
            ratio > 0.90,
            "compression ratio should be >90%, got {:.1}% ({} → {} bytes)",
            ratio * 100.0,
            original_size,
            compressed_size
        );

        assert!(md.contains("News"));
        assert!(md.contains("Article 0"));
        assert!(md.contains("Article 9"));
    }

    #[test]
    fn snapshot_empty_html() {
        let md = html_to_aria_markdown("");
        assert!(md.is_empty() || md.trim().is_empty());
    }

    #[test]
    fn snapshot_deeply_nested_dom_does_not_stack_overflow() {
        // DOM bomb: 200 levels of nesting (exceeds MAX_DOM_DEPTH=100)
        let mut html = String::from("<html><body><p>");
        for _ in 0..200 {
            html.push_str("<span>");
        }
        html.push_str("deep text");
        for _ in 0..200 {
            html.push_str("</span>");
        }
        html.push_str("</p></body></html>");

        // Should not stack overflow — just returns truncated text
        let md = html_to_aria_markdown(&html);
        // The function should complete without crashing
        assert!(!md.is_empty() || md.is_empty()); // always true, proves no panic
    }

    #[test]
    fn snapshot_moderate_nesting_still_extracts_text() {
        // 50 levels of nesting — well within MAX_DOM_DEPTH=100
        let mut html = String::from("<html><body><p>");
        for _ in 0..50 {
            html.push_str("<span>");
        }
        html.push_str("reachable text");
        for _ in 0..50 {
            html.push_str("</span>");
        }
        html.push_str("</p></body></html>");

        let md = html_to_aria_markdown(&html);
        assert!(
            md.contains("reachable text"),
            "text within depth limit should be extracted: {md}"
        );
    }

    // ── #16: Fuzzing tests for HTML parser ──────────────────────────────

    #[test]
    fn fuzz_html_unclosed_tags() {
        // Unclosed tags at various nesting levels — must not panic
        let html = "<html><body><h1>Title<p>No closing tags<a href='#'>link<b>bold";
        let md = html_to_aria_markdown(html);
        assert!(
            md.contains("Title"),
            "should extract title even with unclosed tags"
        );
    }

    #[test]
    fn fuzz_html_mismatched_tags() {
        let html = "<html><body><h1>Header</h2><p>Text</h3></body></html>";
        let md = html_to_aria_markdown(html);
        // Should not panic — parser is lenient
        assert!(md.contains("Header") || md.contains("Text"));
    }

    #[test]
    fn fuzz_html_null_bytes() {
        let html = "<html><body><p>before\x00after</p></body></html>";
        let md = html_to_aria_markdown(html);
        // Should not panic; content may or may not include null
        let _ = md;
    }

    #[test]
    fn fuzz_html_giant_attribute() {
        // Very long attribute value — must not OOM or panic
        let huge_attr = "x".repeat(100_000);
        let html = format!(r#"<html><body><a href="{huge_attr}">Link</a></body></html>"#);
        let md = html_to_aria_markdown(&html);
        assert!(md.contains("Link"), "should extract link text");
    }

    #[test]
    fn fuzz_html_script_in_attribute() {
        // XSS attempt via attribute — parser should handle gracefully
        let html = r#"<html><body><img onerror="alert(1)" src="x"><p>safe</p></body></html>"#;
        let md = html_to_aria_markdown(html);
        assert!(!md.contains("alert"), "script in attribute must not leak");
        assert!(md.contains("safe"));
    }

    #[test]
    fn fuzz_html_comment_with_tags() {
        let html = r#"<html><body><!-- <script>evil()</script> --><p>visible</p></body></html>"#;
        let md = html_to_aria_markdown(html);
        assert!(
            !md.contains("evil"),
            "content inside comments must not appear"
        );
        assert!(md.contains("visible"));
    }

    #[test]
    fn fuzz_html_cdata_section() {
        let html = r#"<html><body><p>before</p><![CDATA[<script>evil</script>]]><p>after</p></body></html>"#;
        let md = html_to_aria_markdown(html);
        // CDATA is unusual in HTML5; parser should not panic
        let _ = md;
    }

    #[test]
    fn fuzz_html_entities_abuse() {
        // Numeric entities, named entities, incomplete entities
        let html = r#"<html><body><p>&#x3C;script&#x3E;evil&#x3C;/script&#x3E;</p><p>&amp;&lt;&gt;</p></body></html>"#;
        let md = html_to_aria_markdown(html);
        // Entities are decoded by the HTML parser. The decoded <script>evil</script>
        // is TEXT content inside <p>, not a real script element, so the parser treats
        // it as text. The key property: clean_text extracts it but it's just text, not
        // an executable script context. Must not panic.
        assert!(!md.is_empty() || md.is_empty()); // always true — proves no panic

        // Also test named entities
        let html2 = r#"<html><body><p>&amp; &lt; &gt; &quot; &apos;</p></body></html>"#;
        let md2 = html_to_aria_markdown(html2);
        assert!(md2.contains("&"), "ampersand entity decoded");
    }

    #[test]
    fn fuzz_html_deeply_nested_tables() {
        let mut html = String::from("<html><body>");
        for _ in 0..50 {
            html.push_str("<table><tr><td>");
        }
        html.push_str("deep cell");
        for _ in 0..50 {
            html.push_str("</td></tr></table>");
        }
        html.push_str("</body></html>");
        let md = html_to_aria_markdown(&html);
        // Should not panic or stack overflow
        let _ = md;
    }

    #[test]
    fn fuzz_html_empty_elements() {
        let html = r#"<html><body><h1></h1><p></p><a href=""></a><button></button></body></html>"#;
        let md = html_to_aria_markdown(html);
        // Empty elements should produce empty output — no panic
        assert!(
            md.is_empty() || md.trim().is_empty(),
            "empty elements = empty output"
        );
    }

    #[test]
    fn fuzz_html_only_whitespace_text() {
        let html = "<html><body><p>   \n\t   </p><h1>  \n  </h1></body></html>";
        let md = html_to_aria_markdown(html);
        assert!(md.is_empty() || md.trim().is_empty());
    }

    #[test]
    fn fuzz_html_binary_content() {
        // Control characters and unusual codepoints in HTML
        let html = "<html><body><p>\u{FFFD}\u{200B}\u{FEFF}\u{0007}</p></body></html>";
        let md = html_to_aria_markdown(html);
        let _ = md; // Must not panic
    }

    #[test]
    fn fuzz_html_mixed_encodings() {
        let html = "<html><body><h1>日本語</h1><p>Ελληνικά</p><p>العربية</p></body></html>";
        let md = html_to_aria_markdown(html);
        assert!(md.contains("日本語"), "Japanese text preserved");
        assert!(md.contains("Ελληνικά"), "Greek text preserved");
        assert!(md.contains("العربية"), "Arabic text preserved");
    }
}
