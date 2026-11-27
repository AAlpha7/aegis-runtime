//! Telegram HTML rendering and message formatting utilities.
//!
//! This module contains:
//! - AST-based Markdown-to-Telegram-HTML conversion (`markdown_to_telegram_html`)
//! - HTML escaping (`escape_html`)
//! - MarkdownV2 escaping (`escape_markdown_v2`)
//! - Table rendering (adaptive horizontal/vertical)
//! - Tag-aware message splitting (`split_message`)
//!
//! Extracted from `telegram.rs` to keep files under the 2,000-line limit (Rule #15).

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

/// Escape special characters for Telegram MarkdownV2 parse mode.
/// See: <https://core.telegram.org/bots/api#markdownv2-style>
pub fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for ch in text.chars() {
        if matches!(
            ch,
            '_' | '*'
                | '['
                | ']'
                | '('
                | ')'
                | '~'
                | '`'
                | '>'
                | '#'
                | '+'
                | '-'
                | '='
                | '|'
                | '{'
                | '}'
                | '.'
                | '!'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ── AST-based Markdown → Telegram HTML renderer ─────────────────────────

/// Escape characters that are special in Telegram HTML mode.
///
/// Escapes `&`, `<`, `>`, `"`, and `'` to prevent injection in
/// both text content and attribute contexts.
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Convert `||spoiler text||` to Telegram spoiler HTML tags.
/// Skips content inside backtick code spans and fenced code blocks.
/// Safe for UTF-8: all scanned delimiters are ASCII, content is sliced.
fn preprocess_spoilers(text: &str) -> String {
    // Fast path: no || in input at all
    if !text.contains("||") {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut last_flush = 0; // tracks un-flushed plain text start

    while i < len {
        // Skip fenced code blocks (``` ... ```)
        if i + 2 < len && bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`' {
            i += 3;
            while i + 2 < len {
                if bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`' {
                    i += 3;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Skip inline code (`...`)
        if bytes[i] == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        // Detect ||spoiler||
        if i + 1 < len && bytes[i] == b'|' && bytes[i + 1] == b'|' {
            // Flush plain text accumulated so far
            out.push_str(&text[last_flush..i]);
            i += 2; // consume opening ||
            let content_start = i;
            let mut found_close = false;
            while i + 1 < len {
                if bytes[i] == b'|' && bytes[i + 1] == b'|' {
                    found_close = true;
                    break;
                }
                // Advance by UTF-8 char width to stay on char boundaries
                if bytes[i] < 0x80 {
                    i += 1;
                } else {
                    i += 1;
                    while i < len && bytes[i] & 0xC0 == 0x80 {
                        i += 1; // skip continuation bytes
                    }
                }
            }
            if found_close && i > content_start {
                out.push_str("<span class=\"tg-spoiler\">");
                out.push_str(&text[content_start..i]);
                out.push_str("</span>");
                i += 2; // consume closing ||
            } else {
                // Unclosed || — emit as literal
                out.push_str("||");
                // i may be at len; safe because content_start..len is valid UTF-8
                out.push_str(&text[content_start..i.min(len)]);
                i = len; // nothing left to scan
            }
            last_flush = i;
            continue;
        }
        i += 1;
    }
    // Flush remaining text
    out.push_str(&text[last_flush..]);
    out
}

/// Process a single markdown event while inside a table.
/// Returns `true` if the event was consumed (caller should `continue`).
fn handle_table_event(
    event: &Event<'_>,
    table_rows: &mut Vec<Vec<String>>,
    current_row: &mut Vec<String>,
    current_cell: &mut String,
    in_table: &mut bool,
    out: &mut String,
) -> bool {
    match event {
        Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
            *current_row = Vec::new();
        }
        Event::Start(Tag::TableCell) => {
            *current_cell = String::new();
        }
        Event::Text(t) => {
            current_cell.push_str(&escape_html(t));
        }
        Event::Code(t) => {
            current_cell.push_str(&escape_html(t));
        }
        Event::End(TagEnd::TableCell) => {
            current_row.push(std::mem::take(current_cell));
        }
        Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
            table_rows.push(std::mem::take(current_row));
        }
        Event::End(TagEnd::Table) => {
            out.push_str(&render_table_adaptive(table_rows));
            out.push('\n');
            table_rows.clear();
            *in_table = false;
        }
        _ => {}
    }
    true
}

/// Convert LLM markdown output to Telegram-compatible HTML using pulldown-cmark AST.
///
/// Supported Telegram HTML tags: `<b>`, `<i>`, `<s>`, `<code>`, `<pre>`,
/// `<a href>`, `<blockquote>`. Tables are rendered as ASCII art inside `<pre>`.
/// Headings are rendered as bold text. Unsupported elements (images, footnotes)
/// degrade gracefully to text.
pub fn markdown_to_telegram_html(text: &str) -> String {
    // Pre-process spoiler syntax: ||text|| → <span class="tg-spoiler">text</span>
    // pulldown-cmark doesn't know about ||...||, so convert before parsing.
    let text = preprocess_spoilers(text);

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(&text, opts);

    let mut out = String::with_capacity(text.len() + text.len() / 4);

    // State for table buffering
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();

    // State for ordered list numbering
    let mut list_stack: Vec<Option<u64>> = Vec::new(); // Some(n) = ordered starting at n, None = unordered

    // Track whether the code block has a language (needs </code></pre> vs </pre>)
    let mut code_block_has_lang = false;

    for event in parser {
        if in_table
            && handle_table_event(
                &event,
                &mut table_rows,
                &mut current_row,
                &mut current_cell,
                &mut in_table,
                &mut out,
            )
        {
            continue;
        }

        match event {
            Event::Start(Tag::Table(_alignments)) => {
                in_table = true;
                table_rows.clear();
            }
            Event::Start(Tag::Paragraph) => {
                // Add paragraph spacing (but not at the very start)
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                out.push('\n');
            }
            Event::Start(Tag::Heading { .. }) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("<b>");
            }
            Event::End(TagEnd::Heading(_)) => {
                out.push_str("</b>\n");
            }
            Event::Start(Tag::BlockQuote(_)) => {
                out.push_str("<blockquote>");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                // Remove trailing newline inside blockquote for cleaner output
                if out.ends_with('\n') {
                    out.pop();
                }
                out.push_str("</blockquote>\n");
            }
            Event::Start(Tag::CodeBlock(kind)) => match kind {
                CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                    code_block_has_lang = true;
                    out.push_str(&format!(
                        "<pre><code class=\"language-{}\">",
                        escape_html(&lang)
                    ));
                }
                _ => {
                    code_block_has_lang = false;
                    out.push_str("<pre>");
                }
            },
            Event::End(TagEnd::CodeBlock) => {
                // Remove trailing newline inside pre
                if out.ends_with('\n') {
                    out.pop();
                }
                if code_block_has_lang {
                    out.push_str("</code></pre>\n");
                } else {
                    out.push_str("</pre>\n");
                }
                code_block_has_lang = false;
            }
            Event::Start(Tag::List(first_num)) => {
                list_stack.push(first_num);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                match list_stack.last() {
                    Some(Some(n)) => {
                        // Ordered list: emit the number
                        out.push_str(&format!("{}. ", n));
                        // Increment counter for next item
                        if let Some(Some(counter)) = list_stack.last_mut() {
                            *counter += 1;
                        }
                    }
                    _ => {
                        out.push_str("\u{2022} ");
                    }
                }
            }
            Event::End(TagEnd::Item) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Emphasis) => {
                out.push_str("<i>");
            }
            Event::End(TagEnd::Emphasis) => {
                out.push_str("</i>");
            }
            Event::Start(Tag::Strong) => {
                out.push_str("<b>");
            }
            Event::End(TagEnd::Strong) => {
                out.push_str("</b>");
            }
            Event::Start(Tag::Strikethrough) => {
                out.push_str("<s>");
            }
            Event::End(TagEnd::Strikethrough) => {
                out.push_str("</s>");
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                out.push_str(&format!("<a href=\"{}\">", escape_html(&dest_url)));
            }
            Event::End(TagEnd::Link) => {
                out.push_str("</a>");
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                // Telegram doesn't support images in HTML; show as link
                let t = if title.is_empty() { "image" } else { &title };
                out.push_str(&format!("[{}: ", escape_html(t)));
                let _ = dest_url; // URL captured by alt text mechanism
            }
            Event::End(TagEnd::Image) => {
                out.push(']');
            }
            Event::Text(t) => {
                out.push_str(&escape_html(&t));
            }
            Event::Code(t) => {
                out.push_str("<code>");
                out.push_str(&escape_html(&t));
                out.push_str("</code>");
            }
            Event::SoftBreak => {
                out.push('\n');
            }
            Event::HardBreak => {
                out.push('\n');
            }
            Event::Rule => {
                out.push_str("\u{2014}\u{2014}\u{2014}\n");
            }
            Event::TaskListMarker(checked) => {
                out.push_str(if checked { "\u{2611} " } else { "\u{2610} " });
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                // Pass raw HTML through — Telegram HTML mode is forgiving
                out.push_str(&html);
            }
            _ => {} // Ignore footnotes, math, metadata, etc.
        }
    }

    // Trim trailing whitespace
    while out.ends_with('\n') || out.ends_with(' ') {
        out.pop();
    }

    out
}

/// Maximum display width for horizontal `<pre>` table layout.
/// Tables wider than this switch to vertical card format.
const TABLE_MAX_DISPLAY_WIDTH: usize = 45;

/// Unescape HTML entities to get plain text for width measurement.
pub(crate) fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// Calculate the display width of an HTML-escaped string, correctly handling
/// CJK double-width characters.
pub(crate) fn display_width(html_escaped: &str) -> usize {
    UnicodeWidthStr::width(unescape_html(html_escaped).as_str())
}

/// Render table rows adaptively:
/// - Narrow tables (<=45 display cols) -> horizontal `<pre>` with proper CJK alignment
/// - Wide tables -> vertical card format using `<b>` headers (no `<pre>`, works on any screen)
///
/// Cell contents are already HTML-escaped.
pub(crate) fn render_table_adaptive(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);

    // Calculate column widths using unicode display width
    let mut col_widths = vec![0usize; num_cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            col_widths[i] = col_widths[i].max(display_width(cell));
        }
    }
    for w in &mut col_widths {
        *w = (*w).clamp(1, 30);
    }

    // Total width: sum of columns + separators + padding
    let total_width: usize = col_widths.iter().sum::<usize>() + num_cols * 3 + 1;

    if total_width <= TABLE_MAX_DISPLAY_WIDTH {
        render_table_horizontal(rows, &col_widths)
    } else {
        render_table_vertical(rows)
    }
}

/// Horizontal `<pre>` table with proper CJK-aware padding.
fn render_table_horizontal(rows: &[Vec<String>], col_widths: &[usize]) -> String {
    let mut out = String::from("<pre>");

    let sep_line: String = {
        let parts: Vec<String> = col_widths
            .iter()
            .map(|w| "\u{2500}".repeat(*w + 2))
            .collect();
        format!("\u{250c}{}\u{2510}\n", parts.join("\u{252c}"))
    };
    let mid_sep: String = {
        let parts: Vec<String> = col_widths
            .iter()
            .map(|w| "\u{2500}".repeat(*w + 2))
            .collect();
        format!("\u{251c}{}\u{2524}\n", parts.join("\u{253c}"))
    };
    let bot_sep: String = {
        let parts: Vec<String> = col_widths
            .iter()
            .map(|w| "\u{2500}".repeat(*w + 2))
            .collect();
        format!("\u{2514}{}\u{2518}\n", parts.join("\u{2534}"))
    };

    for (row_idx, row) in rows.iter().enumerate() {
        if row_idx == 0 {
            out.push_str(&sep_line);
        }
        out.push('\u{2502}');
        for (i, cell) in row.iter().enumerate() {
            let target_width = col_widths.get(i).copied().unwrap_or(1);
            let cell_width = display_width(cell);
            let padding = target_width.saturating_sub(cell_width);
            out.push(' ');
            out.push_str(cell);
            for _ in 0..padding {
                out.push(' ');
            }
            out.push_str(" \u{2502}");
        }
        out.push('\n');
        if row_idx == 0 {
            out.push_str(&mid_sep);
        }
    }
    out.push_str(&bot_sep);
    // Remove trailing newline
    if out.ends_with('\n') {
        out.pop();
    }
    out.push_str("</pre>");
    out
}

/// Vertical card format for wide tables. Each data row is rendered as a card
/// with bold header labels. No `<pre>`, works on any screen width.
fn render_table_vertical(rows: &[Vec<String>]) -> String {
    if rows.len() < 2 {
        // Need at least header + 1 data row
        return rows.first().map(|r| r.join(" | ")).unwrap_or_default();
    }

    let headers = &rows[0];
    let mut out = String::new();

    for (row_idx, row) in rows[1..].iter().enumerate() {
        if row_idx > 0 {
            out.push_str("\u{2014}\u{2014}\u{2014}\n");
        }
        for (i, cell) in row.iter().enumerate() {
            let header = headers.get(i).map(|h| h.as_str()).unwrap_or("?");
            // Unescape for display, re-escape header as bold
            out.push_str(&format!("<b>{}</b>: {}\n", header, cell));
        }
    }
    // Remove trailing newline
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Extract plain text from markdown using pulldown-cmark AST.
/// Used as fallback when HTML rendering fails, and in tests.
#[cfg(test)]
pub(crate) fn markdown_to_plain(text: &str) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(text, opts);
    let mut out = String::with_capacity(text.len());

    for event in parser {
        match event {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                out.push_str(&t);
            }
            Event::SoftBreak | Event::HardBreak => {
                out.push('\n');
            }
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item) => {
                out.push('\n');
            }
            Event::Rule => {
                out.push_str("\u{2014}\u{2014}\u{2014}\n");
            }
            _ => {}
        }
    }

    while out.ends_with('\n') {
        out.pop();
    }
    out
}

// ── Tag-aware message splitting ─────────────────────────────────────────────

/// Split an HTML message into chunks respecting a max byte length.
///
/// This is a **tag-aware** splitter: it tracks open HTML tags (`<b>`, `<i>`,
/// `<pre>`, `<code>`, `<blockquote>`, `<s>`, `<a ...>`) in a stack. When a
/// chunk boundary falls inside open tags, the current chunk is closed with
/// the appropriate closing tags and the next chunk reopens them.
///
/// Handles multi-byte UTF-8 safely (never splits mid-codepoint). Prefers
/// splitting at newlines or spaces.
pub fn split_message(msg: &str, max_len: usize) -> Vec<String> {
    if msg.len() <= max_len {
        return vec![msg.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    // Tags carried over from the previous chunk (opening tags to prepend)
    let mut carry_tags: Vec<String> = Vec::new();

    while start < msg.len() {
        // Calculate overhead from reopening carried tags
        let reopen_overhead: usize = carry_tags.iter().map(|t| t.len()).sum();
        // Reserve space for closing tags at end (worst case, same size as reopen)
        let close_overhead: usize = carry_tags.iter().map(|t| closing_tag_for(t).len()).sum();
        let available = max_len.saturating_sub(reopen_overhead + close_overhead);
        let available = available.max(256); // minimum progress

        let mut end = std::cmp::min(start + available, msg.len());
        // Walk back to a valid UTF-8 char boundary
        while end > start && !msg.is_char_boundary(end) {
            end -= 1;
        }
        // Try to split at a newline or space boundary
        if end < msg.len() {
            let candidate = &msg[start..end];
            if let Some(pos) = candidate.rfind('\n') {
                end = start + pos + 1;
            } else if let Some(pos) = candidate.rfind(' ') {
                end = start + pos + 1;
            }
        }

        let raw = &msg[start..end];

        // Build the chunk: reopen tags + raw content + close tags
        let mut chunk = String::with_capacity(raw.len() + reopen_overhead + close_overhead + 16);

        // Reopen carried tags from previous chunk
        for tag in &carry_tags {
            chunk.push_str(tag);
        }

        chunk.push_str(raw);

        // Scan this raw slice to update the tag stack
        update_tag_stack(&mut carry_tags, raw);

        // If there are still open tags and this isn't the last chunk, close them
        if !carry_tags.is_empty() && end < msg.len() {
            // Close in reverse order
            for tag in carry_tags.iter().rev() {
                chunk.push_str(&closing_tag_for(tag));
            }
        }

        chunks.push(chunk);
        start = end;
    }

    chunks
}

/// Scan an HTML fragment and update the tag stack.
/// Pushes opening tags and pops matching closing tags.
pub(crate) fn update_tag_stack(stack: &mut Vec<String>, html: &str) {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(end) = html[i..].find('>')
        {
            let tag_content = &html[i + 1..i + end];
            if let Some(rest) = tag_content.strip_prefix('/') {
                // Closing tag: pop the most recent matching open tag
                let tag_name = rest.trim();
                // Find and remove the last matching open tag
                if let Some(pos) = stack
                    .iter()
                    .rposition(|open| extract_tag_name(open) == tag_name)
                {
                    stack.remove(pos);
                }
            } else if !tag_content.ends_with('/') && !tag_content.starts_with('!') {
                // Opening tag (not self-closing, not comment)
                let full_tag = &html[i..i + end + 1];
                let tag_name = extract_tag_name(full_tag);
                // Only track Telegram-relevant formatting tags
                if matches!(
                    tag_name,
                    "b" | "i" | "s" | "code" | "pre" | "blockquote" | "a" | "span"
                ) {
                    stack.push(full_tag.to_string());
                }
            }
            i += end + 1;
            continue;
        }
        i += 1;
    }
}

/// Extract the tag name from an opening tag like `<pre>`, `<code class="language-rust">`, `<a href="...">`.
pub(crate) fn extract_tag_name(tag: &str) -> &str {
    let inner = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');
    // Take up to first space (for attributes)
    inner.split_whitespace().next().unwrap_or(inner)
}

/// Generate the closing tag for an opening tag.
pub(crate) fn closing_tag_for(opening: &str) -> String {
    let name = extract_tag_name(opening);
    format!("</{}>", name)
}
