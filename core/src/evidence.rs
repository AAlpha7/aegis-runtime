//! Per-turn inspectable evidence for Web console assistant replies.
//!
//! Tool outputs the reply was built from are kept as numbered `EvidenceItem`s
//! (E1, E2, …). Numbers in the reply that no tool output or owner message
//! supports are listed separately — no evidence, not a fact.

use regex::Regex;
use std::sync::LazyLock;

const EVIDENCE_ITEM_CAP: usize = 16;
const EXCERPT_CHAR_CAP: usize = 600;
const CLIP_OMISSION: &str = "Further source entries are omitted.";

fn showing_note(kept: usize, total: usize) -> String {
    format!("Showing {kept} of {total} source entries.")
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    pub tool: String,
    pub label: String,
    pub excerpt: String,
    pub full_len: usize,
}

/// Tools whose output is bookkeeping, not evidence the owner can inspect.
/// They keep their call slot (the model numbers refs by call order) but get
/// no item, so a ref to them is stripped by [`strip_unbacked_refs`].
const META_TOOLS: &[&str] = &["search_tools"];

/// Keys whose string value is the owner-readable part of a JSON tool output.
const EXCERPT_KEYS: &[&str] = &[
    "summary", "answer", "result", "content", "text", "message", "output", "detail",
];

/// Ids follow the call order (E1 = first tool called this turn) so they match
/// the numbering the model was told to use; empty and meta outputs keep their
/// slot but produce no item.
pub fn evidence_items(tool_outputs: &[(String, String)]) -> Vec<EvidenceItem> {
    tool_outputs
        .iter()
        .enumerate()
        .filter(|(_, (tool, output))| {
            !META_TOOLS.contains(&tool.as_str()) && !collapse_ws(output).is_empty()
        })
        .take(EVIDENCE_ITEM_CAP)
        .map(|(index, (tool, output))| EvidenceItem {
            id: format!("E{}", index + 1),
            tool: tool.clone(),
            label: evidence_tool_label(tool),
            excerpt: evidence_excerpt(output),
            full_len: output.chars().count(),
        })
        .collect()
}

/// Format structured evidence before clipping it: raw JSON is not owner copy.
///
/// JSON becomes an indented tree (two spaces per level, `- ` for each array
/// item, `Label: value` for a scalar field). The 600-character cap drops whole
/// trailing nodes and says how many entries fit, so a clip never ends inside
/// a field. A single oversized node is still cut on a character boundary.
/// Multiline text keeps its line breaks inside a fence. Plain single-line
/// text stays collapsed.
fn evidence_excerpt(output: &str) -> String {
    let trimmed = output.trim();
    let readable = match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => owner_web_search_excerpt(&value)
            .unwrap_or_else(|| clip_evidence_blocks(&evidence_blocks(&value, 0))),
        Err(_) => collapse_ws(output).chars().take(EXCERPT_CHAR_CAP).collect(),
    };
    if readable.trim().is_empty() {
        return "No source details returned.".into();
    }
    readable
}

/// Owner copy of a web-search tool result. The JSON also carries fields the
/// model needs (`note`, `retryable`, the raw `search_prompt` instruction).
/// Those stay in the tool result and stay out of this excerpt.
fn owner_web_search_excerpt(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    let note = object
        .get("note")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let prompt = object
        .get("search_prompt")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let citations = object
        .get("verified_citations")
        .and_then(|item| item.as_array());
    if prompt.is_empty()
        && citations.is_none()
        && !crate::search_turn::looks_like_model_instruction(note)
    {
        return None;
    }
    if object.get("status").and_then(|item| item.as_str()) == Some("error") {
        return None;
    }
    let mut lines = Vec::new();
    let owner = object
        .get("owner_query")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let question = if owner.trim().is_empty() {
        crate::search_turn::owner_facing_query(prompt)
    } else {
        crate::search_turn::clip(owner.trim(), 160)
    };
    if !question.is_empty() && !crate::search_turn::looks_like_model_instruction(&question) {
        lines.push(question);
    }
    if let Some(citations) = citations {
        for citation in citations.iter().take(3) {
            let title = citation
                .get("title")
                .and_then(|item| item.as_str())
                .unwrap_or("");
            let url = citation
                .get("url")
                .and_then(|item| item.as_str())
                .unwrap_or("");
            let snippet = citation
                .get("snippet")
                .or_else(|| citation.get("content"))
                .and_then(|item| item.as_str())
                .unwrap_or("");
            let name = if title.trim().is_empty() {
                url_host(url).unwrap_or_default()
            } else {
                crate::search_turn::strip_markdown_markers(title)
            };
            let excerpt = crate::search_turn::owner_facing_excerpt(snippet);
            if name.is_empty() && excerpt.is_empty() && url.is_empty() {
                continue;
            }
            let mut row = name;
            if !excerpt.is_empty() {
                if !row.is_empty() {
                    row.push_str(": ");
                }
                row.push_str(&excerpt);
            }
            if !url.is_empty() {
                if !row.is_empty() {
                    row.push('\n');
                }
                row.push_str(url);
            }
            if !row.is_empty() {
                lines.push(row);
            }
        }
    }
    if lines.iter().all(|line| !line.contains("http"))
        && let Some(summary) = object.get("summary").and_then(|item| item.as_str())
    {
        let excerpt = crate::search_turn::owner_facing_excerpt(summary);
        if !excerpt.is_empty() {
            lines.push(excerpt);
        }
    }
    let text = lines.join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn url_host(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?.trim();
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn evidence_blocks(value: &serde_json::Value, depth: usize) -> Vec<String> {
    if depth > 6 {
        return vec!["Further nested source details are omitted.".into()];
    }
    match value {
        serde_json::Value::String(text) => {
            // A JSON string that itself holds an object is the same node, not
            // an extra depth level. MCP text blocks are stored that way.
            if let Ok(nested) = serde_json::from_str::<serde_json::Value>(text.trim())
                && (nested.is_object() || nested.is_array())
            {
                return evidence_blocks(&nested, depth);
            }
            text_leaf(text)
        }
        serde_json::Value::Array(items) => {
            let mut blocks = Vec::new();
            for item in items {
                let text = evidence_blocks(item, depth + 1).join("\n");
                if !text.is_empty() {
                    blocks.push(bullet_block(&text));
                }
            }
            blocks
        }
        serde_json::Value::Object(object) => object_blocks(object, depth),
        serde_json::Value::Null => Vec::new(),
        _ => vec![value.to_string()],
    }
}

/// Root objects keep failures ahead of long titles so the excerpt cap cannot
/// hide them. Nested records lead with their name, then the same fields.
const ROOT_FIELDS: &[(&str, &str)] = &[
    ("status", "Status"),
    ("error", "Error"),
    ("error_kind", "Error kind"),
    ("warning", "Warning"),
    ("warnings", "Warnings"),
    ("hint", "Hint"),
    ("title", ""),
    ("name", ""),
    ("count", "Count"),
    ("due_date", "Due"),
    ("trigger_at_unix", "Scheduled"),
    ("url", "URL"),
    ("summary", ""),
    ("answer", ""),
    ("result", ""),
    ("content", ""),
    ("text", ""),
    ("message", ""),
    ("output", ""),
    ("detail", ""),
    ("body", ""),
];
const NESTED_LEAD_FIELDS: &[(&str, &str)] = &[
    ("title", ""),
    ("name", ""),
    ("summary", ""),
    ("text", ""),
    ("message", ""),
    ("answer", ""),
    ("result", ""),
    ("output", ""),
    ("detail", ""),
    ("body", ""),
];

fn object_blocks(object: &serde_json::Map<String, serde_json::Value>, depth: usize) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if depth > 0 {
        append_known_fields(&mut blocks, object, depth, NESTED_LEAD_FIELDS, &mut seen);
    }
    append_known_fields(&mut blocks, object, depth, ROOT_FIELDS, &mut seen);
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "title"
                | "name"
                | "count"
                | "status"
                | "error"
                | "error_kind"
                | "warning"
                | "warnings"
                | "hint"
                | "due_date"
                | "trigger_at_unix"
                | "url"
                | "body"
                | "index"
                | "status_filter"
                | "execution_mode"
                | "trigger_type"
                | "schema_version"
        ) || EXCERPT_KEYS.contains(&key.as_str())
            || (key == "type"
                && value == "text"
                && object.get("text").is_some_and(|v| v.is_string()))
            || (key == "note"
                && value
                    .as_str()
                    .is_some_and(crate::search_turn::looks_like_model_instruction))
            || (key == "retryable"
                && object.get("status").and_then(|status| status.as_str()) == Some("ok"))
            || (key == "search_prompt"
                && value
                    .as_str()
                    .is_some_and(crate::search_turn::looks_like_model_instruction))
            || is_model_only_field(key, value)
        {
            continue;
        }
        let unlabeled =
            value.is_array() && matches!(key.as_str(), "tasks" | "items" | "results" | "entries");
        if unlabeled {
            push_field(&mut blocks, "", value, depth);
        } else {
            let label = humanize_machine_label(key);
            push_field(&mut blocks, &label, value, depth);
        }
    }
    blocks
}

fn append_known_fields<'a>(
    blocks: &mut Vec<String>,
    object: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
    keys: &[(&'a str, &str)],
    seen: &mut std::collections::HashSet<&'a str>,
) {
    for (key, label) in keys {
        if !seen.insert(*key) {
            continue;
        }
        let Some(value) = object.get(*key) else {
            continue;
        };
        if *key == "trigger_at_unix" {
            let text = value
                .as_i64()
                .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                .map(|date| date.to_rfc3339())
                .unwrap_or_default();
            if !text.is_empty() {
                blocks.push(field_block(label, &text));
            }
        } else {
            push_field(blocks, label, value, depth);
        }
    }
}

fn push_field(blocks: &mut Vec<String>, label: &str, value: &serde_json::Value, depth: usize) {
    let child = evidence_blocks(value, depth + 1);
    if child.is_empty() {
        return;
    }
    if label.is_empty() {
        blocks.extend(child);
        return;
    }
    blocks.push(field_block(label, &child.join("\n")));
}

fn field_block(label: &str, text: &str) -> String {
    if label.is_empty() {
        return text.to_string();
    }
    // A list or a multiline value stays a child of this field, not a sibling line.
    if text.contains('\n') || text.starts_with("- ") {
        return format!("{label}:\n{}", indent_block(text, 2));
    }
    format!("{label}: {text}")
}

fn bullet_block(text: &str) -> String {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return "-".into();
    };
    let mut out = format!("- {first}");
    for line in lines {
        out.push('\n');
        out.push_str("  ");
        out.push_str(line);
    }
    out
}

fn indent_block(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keep whole nodes. When a later node does not fit, drop it and everything
/// after it, then say how many of the entries fit. An oversized first node is
/// clipped on a character boundary only because there is no earlier fact to keep.
fn clip_evidence_blocks(blocks: &[String]) -> String {
    if blocks.is_empty() {
        return String::new();
    }
    let total = blocks.len();
    let omission_len = CLIP_OMISSION.chars().count();
    let reserve_note = showing_note(total.saturating_sub(1), total);
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        let sep = if kept.is_empty() { 0 } else { 1 };
        let more = index + 1 < blocks.len();
        let reserve = if more {
            1 + reserve_note.chars().count()
        } else {
            0
        };
        let need = sep + block.chars().count();
        if used + need + reserve <= EXCERPT_CHAR_CAP {
            used += need;
            kept.push(block);
            continue;
        }
        if kept.is_empty() {
            let omit = block.chars().count() > EXCERPT_CHAR_CAP;
            let budget = if omit {
                EXCERPT_CHAR_CAP.saturating_sub(1 + omission_len)
            } else {
                EXCERPT_CHAR_CAP
            };
            let clipped: String = block.chars().take(budget).collect();
            if omit {
                return format!("{clipped}\n{CLIP_OMISSION}");
            }
            return clipped;
        }
        let note = showing_note(kept.len(), total);
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&note);
        return out;
    }
    kept.join("\n")
}

fn text_leaf(text: &str) -> Vec<String> {
    if !text.contains('\n') {
        let collapsed = collapse_ws(text);
        return if collapsed.is_empty() {
            Vec::new()
        } else {
            vec![collapsed]
        };
    }
    let body = text
        .lines()
        .map(collapse_line)
        .collect::<Vec<_>>()
        .join("\n");
    if collapse_ws(&body).is_empty() {
        return Vec::new();
    }
    // A fence keeps `Key: value` lines inside the string from being read as fields.
    vec![format!("```\n{body}\n```")]
}

fn collapse_line(line: &str) -> String {
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - rest.len()];
    format!("{indent}{}", collapse_ws(rest.trim_end()))
}

static EVIDENCE_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\[(E\d+(?:\s*,\s*E\d+)*)\]").expect("evidence ref regex"));

/// Removes `[E#]` refs the reply makes to ids that have no evidence item, so a
/// ref to a bookkeeping tool or a made-up number never reaches the owner.
pub fn strip_unbacked_refs(reply: &str, items: &[EvidenceItem]) -> String {
    EVIDENCE_REF
        .replace_all(reply, |caps: &regex::Captures<'_>| {
            let kept: Vec<&str> = caps[1]
                .split(',')
                .map(str::trim)
                .filter(|id| items.iter().any(|item| item.id == *id))
                .collect();
            if kept.is_empty() {
                String::new()
            } else {
                format!(" [{}]", kept.join(", "))
            }
        })
        .into_owned()
}

/// Numbers in `reply` that do not appear in any tool output or in `owner_text`.
///
/// Comparison is on the numeric value (separators, units, and ± stripped). A
/// range is supported only if both ends are supported. A number is treated as
/// supported when a source number is within ±1.5% or ±1 unit of it.
pub fn unsupported_numbers(
    reply: &str,
    tool_outputs: &[(String, String)],
    owner_text: &str,
) -> Vec<String> {
    let sources = {
        let mut numbers = Vec::new();
        for (_, output) in tool_outputs {
            extract_numbers_with(output, &mut numbers, false);
        }
        extract_numbers_with(owner_text, &mut numbers, false);
        numbers
    };
    let mut found = Vec::new();
    extract_numbers(reply, &mut found);
    found
        .into_iter()
        .filter(|token| !token_is_supported(token, &sources) && !token.ignore_if_unmatched)
        .map(|token| token.display)
        .collect()
}

pub(crate) fn evidence_tool_label(tool: &str) -> String {
    let normalized = tool.trim().trim_start_matches("skill_");
    match normalized {
        "recall_memory" => "Memory".into(),
        "manage_tasks" | "schedule_intent" => "Tasks and reminders".into(),
        "calendar_reader" => "Calendar".into(),
        "web_search" => "Web search".into(),
        "commute_traffic_monitor" => "Commute check".into(),
        "check_api_usage" => "API cost tracker".into(),
        "server_monitor" => "Server monitor".into(),
        _ => humanize_machine_label(normalized),
    }
}

/// Notes and hint fields written for the model ("Do not call…", "Treat it as…")
/// are not owner copy. A short factual note still shows.
fn is_model_only_field(key: &str, value: &serde_json::Value) -> bool {
    if matches!(key, "analysis_hint" | "interpretation_warning") {
        return true;
    }
    if key == "note"
        && let Some(text) = value.as_str()
    {
        return text.starts_with("GROUNDING RULE") || is_model_instruction(text);
    }
    false
}

fn is_model_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("do not ")
        || lower.starts_with("use only ")
        || lower.starts_with("treat ")
        || lower.contains("treat it as")
}

pub(crate) fn humanize_machine_label(value: &str) -> String {
    let mut words = value
        .split(['_', '-', '.'])
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase());
    let Some(first) = words.next() else {
        return "Source".into();
    };
    let mut output = String::new();
    uppercase_first(&first, &mut output);
    for word in words {
        output.push(' ');
        output.push_str(&word);
    }
    output
}

fn uppercase_first(value: &str, output: &mut String) {
    let mut chars = value.chars();
    if let Some(first) = chars.next() {
        output.extend(first.to_uppercase());
        output.extend(chars);
    }
}

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Stable key for an assistant reply, used to reattach stored evidence after a
/// device change. Whitespace is collapsed so L1/L3 formatting drift still hits.
pub fn reply_key(text: &str) -> String {
    // History stores the runtime reply before strip_unbacked_refs adds spacing.
    // Keep the canonical form compatible with existing final-envelope keys.
    let normalized = EVIDENCE_REF.replace_all(text, |caps: &regex::Captures<'_>| {
        format!(
            " [{}]",
            caps[1]
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    blake3::hash(collapse_ws(normalized.trim()).as_bytes())
        .to_hex()
        .to_string()
}

/// Shares the existing evidence store with legacy reply hashes without making
/// equal text from two different requests share a source record.
pub fn turn_key(turn_id: &str) -> String {
    format!("webapp_turn:{turn_id}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParsedValue {
    Scalar(f64),
    TimeSecs(u32),
}

#[derive(Debug, Clone)]
struct NumberToken {
    display: String,
    values: Vec<ParsedValue>,
    ignore_if_unmatched: bool,
}

static MASK_SPANS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?i)https?://[^\s)]+
        | www\.[^\s)]+
        | \[[^\]]*\]\([^)]*\)
        | <https?://[^>]+>
        | \[E\d+\]
        | \b(?:19|20)\d{2}-\d{2}-\d{2}\b
        | \#\d+
        | \b(?:No|NO|no)\.\s?\d+
        ",
    )
    .expect("evidence mask regex")
});

static LIST_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*\d+\.(?:[ \t]|$)").expect("list marker regex"));

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<time_range>
            \d{1,2}:\d{2}(?::\d{2})?
            [ \t]*[–—-][ \t]*
            \d{1,2}:\d{2}(?::\d{2})?
        )
        |
        (?P<percent>
            [±+\-]?
            (?:\d{1,3}(?:,\d{3})+|\d+)
            (?:\.\d+)?
            [ \t]*%
        )
        |
        (?P<num_range>
            [±+\-]?
            (?:\d{1,3}(?:,\d{3})+|\d+)
            (?:\.\d+)?
            [ \t]*[–—-][ \t]*
            [±+\-]?
            (?:\d{1,3}(?:,\d{3})+|\d+)
            (?:\.\d+)?
        )
        |
        (?P<time>
            \d{1,2}:\d{2}(?::\d{2})?
        )
        |
        (?P<number>
            [±+\-]?
            (?:\d{1,3}(?:,\d{3})+|\d+)
            (?:\.\d+)?
            (?:分钟|小时|秒钟|秒|点钟|点|minutes?|mins?|hours?|hrs?|km/h|mph|kph|km|mi)?
        )
        ",
    )
    .expect("number token regex")
});

static RANGE_SEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[–—-]").expect("range separator regex"));

/// Like `MASK_SPANS` but keeps `#36` / `No. 12` — in a tool OUTPUT those are
/// the only place an id is printed ("Task #36 completed."), and masking them
/// there flagged a correct "第36项已恢复" as unsupported.
static SOURCE_MASK_SPANS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?i)https?://[^\s)]+
        | www\.[^\s)]+
        | \[[^\]]*\]\([^)]*\)
        | <https?://[^>]+>
        | \[E\d+\]
        | \b(?:19|20)\d{2}-\d{2}-\d{2}\b
        ",
    )
    .expect("evidence source mask regex")
});

fn mask_ignored_spans_with(text: &str, mask_ids: bool) -> String {
    let spans: &Regex = if mask_ids {
        &MASK_SPANS
    } else {
        &SOURCE_MASK_SPANS
    };
    let masked = spans.replace_all(text, |caps: &regex::Captures| space_mask(caps));
    LIST_MARKER
        .replace_all(&masked, |caps: &regex::Captures| space_mask(caps))
        .into_owned()
}

fn space_mask(caps: &regex::Captures) -> String {
    caps.get(0)
        .map(|m| m.as_str().chars().map(|_| ' ').collect())
        .unwrap_or_default()
}

fn extract_numbers(text: &str, out: &mut Vec<NumberToken>) {
    extract_numbers_with(text, out, true);
}

/// `mask_ids = false` for source texts (tool outputs, owner words): an id
/// printed as `#36` there supports "36" in the reply.
fn extract_numbers_with(text: &str, out: &mut Vec<NumberToken>, mask_ids: bool) {
    let masked = mask_ignored_spans_with(text, mask_ids);
    for caps in TOKEN.captures_iter(&masked) {
        let Some(full) = caps.get(0) else {
            continue;
        };
        let display = full.as_str().trim();
        if display.is_empty() {
            continue;
        }
        let rest = &masked[full.end()..];
        if let Some(token) = token_from_match(&caps, display, rest) {
            out.push(token);
        }
    }
}

fn token_from_match(caps: &regex::Captures, display: &str, rest: &str) -> Option<NumberToken> {
    if caps.name("time_range").is_some() {
        let (left, right) = split_range(display)?;
        let left_t = parse_time(left)?;
        let right_t = parse_time(right)?;
        let ignore = time_hour(left_t) <= 12 && time_hour(right_t) <= 12;
        return Some(NumberToken {
            display: display.to_string(),
            values: vec![
                ParsedValue::TimeSecs(left_t),
                ParsedValue::TimeSecs(right_t),
            ],
            ignore_if_unmatched: ignore,
        });
    }
    if caps.name("percent").is_some() {
        let value = parse_quantity(display)?;
        return Some(NumberToken {
            display: display.to_string(),
            values: vec![ParsedValue::Scalar(value)],
            ignore_if_unmatched: is_year(value),
        });
    }
    if caps.name("num_range").is_some() {
        let (left, right) = split_range(display)?;
        let left_v = parse_quantity(left)?;
        let right_v = parse_quantity(right)?;
        let ignore = range_ends_ignorable(left_v, right_v);
        return Some(NumberToken {
            display: display.to_string(),
            values: vec![ParsedValue::Scalar(left_v), ParsedValue::Scalar(right_v)],
            ignore_if_unmatched: ignore,
        });
    }
    if caps.name("time").is_some() {
        let time = parse_time(display)?;
        return Some(NumberToken {
            display: display.to_string(),
            values: vec![ParsedValue::TimeSecs(time)],
            ignore_if_unmatched: time_hour(time) <= 12,
        });
    }
    if caps.name("number").is_some() {
        let value = parse_quantity(display)?;
        let ignore = is_year(value) || is_bare_small_count(value, display, rest);
        return Some(NumberToken {
            display: display.to_string(),
            values: vec![ParsedValue::Scalar(value)],
            ignore_if_unmatched: ignore,
        });
    }
    None
}

fn split_range(display: &str) -> Option<(&str, &str)> {
    let sep = RANGE_SEP.find(display)?;
    if sep.start() == 0 {
        return None;
    }
    let left = display[..sep.start()].trim();
    let right = display[sep.end()..].trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some((left, right))
}

fn parse_quantity(raw: &str) -> Option<f64> {
    let trimmed = raw
        .trim()
        .trim_start_matches(['±', '+'])
        .trim_end_matches(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '-'));
    let normalized: String = trimmed.chars().filter(|c| *c != ',').collect();
    if normalized.is_empty() || normalized == "-" || normalized == "." {
        return None;
    }
    normalized.parse().ok()
}

fn parse_time(raw: &str) -> Option<u32> {
    let mut parts = raw.trim().split(':');
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = match parts.next() {
        Some(value) => value.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(hour * 3600 + minute * 60 + second)
}

fn time_hour(time_secs: u32) -> u32 {
    time_secs / 3600
}

fn is_year(value: f64) -> bool {
    value.fract() == 0.0 && (1900.0..=2100.0).contains(&value)
}

fn is_bare_small_count(value: f64, display: &str, rest: &str) -> bool {
    if value.fract() != 0.0 || !(1.0..=12.0).contains(&value) {
        return false;
    }
    if quantity_token_has_unit(display) || follower_is_time_unit(rest) {
        return false;
    }
    true
}

fn quantity_token_has_unit(display: &str) -> bool {
    display.chars().any(|c| {
        !(c.is_ascii_digit()
            || c == '.'
            || c == ','
            || c == '-'
            || c == '+'
            || c == '±'
            || c.is_ascii_whitespace())
    })
}

fn follower_is_time_unit(rest: &str) -> bool {
    let rest = rest.trim_start();
    rest.starts_with('点')
        || rest.starts_with("时")
        || rest.starts_with("分钟")
        || rest.starts_with("小时")
        || rest.starts_with("分")
        || rest.starts_with("hour")
        || rest.starts_with("min")
        || rest.starts_with("sec")
}

fn range_ends_ignorable(left: f64, right: f64) -> bool {
    fn end_ignorable(value: f64) -> bool {
        is_year(value) || (value.fract() == 0.0 && (1.0..=12.0).contains(&value))
    }
    end_ignorable(left) && end_ignorable(right)
}

fn token_is_supported(token: &NumberToken, sources: &[NumberToken]) -> bool {
    token.values.iter().all(|value| {
        sources
            .iter()
            .flat_map(|source| source.values.iter())
            .any(|source| values_match(value, source))
    })
}

fn values_match(left: &ParsedValue, right: &ParsedValue) -> bool {
    match (left, right) {
        (ParsedValue::Scalar(a), ParsedValue::Scalar(b)) => {
            scalars_close(*a, *b) || unit_conversion_close(*a, *b)
        }
        (ParsedValue::TimeSecs(a), ParsedValue::TimeSecs(b)) => a == b,
        _ => false,
    }
}

/// A reply number that is the source number in another unit — MB↔GB↔TB by
/// 1024 or 1000, seconds↔minutes↔hours by 60 — counts as supported (owner
/// 2026-09-12: "64.9 GB" was flagged although the tool said 64872 MB).
fn unit_conversion_close(reply: f64, source: f64) -> bool {
    const FACTORS: [f64; 6] = [1000.0, 1024.0, 1_000_000.0, 1_048_576.0, 60.0, 3600.0];
    if reply <= 0.0 || source <= 0.0 {
        return false;
    }
    FACTORS.iter().any(|factor| {
        let converted_down = source / factor;
        let converted_up = source * factor;
        (converted_down >= 0.05 && (reply - converted_down).abs() <= converted_down.max(0.1) * 0.02)
            || (reply - converted_up).abs() <= converted_up * 0.02
    })
}

fn scalars_close(left: f64, right: f64) -> bool {
    let diff = (left - right).abs();
    if diff <= 1.0 {
        return true;
    }
    let scale = left.abs().max(right.abs());
    scale > 0.0 && diff <= scale * 0.015
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_key_collapses_whitespace() {
        assert_eq!(reply_key("a  b\n c"), reply_key("a b c"));
    }

    #[test]
    fn reply_key_matches_history_and_final_citation_spacing() {
        let raw = "明天晴，11–24°C。[E1]";
        let formatted = "明天晴，11–24°C。 [E1]";
        assert_eq!(reply_key(raw), reply_key(formatted));
        // Existing stored keys from the final envelope remain valid.
        assert_eq!(
            reply_key(raw),
            blake3::hash(formatted.as_bytes()).to_hex().to_string()
        );
        assert_eq!(reply_key("答案[E1,E2]"), reply_key("答案 [E1, E2]"));
        assert_ne!(reply_key(raw), reply_key("明天晴，11–24°C。[E2]"));
        assert_ne!(reply_key(raw), reply_key("明天晴，12–24°C。[E1]"));
    }

    #[test]
    fn evidence_items_skip_meta_tools_but_keep_call_slots() {
        let items = evidence_items(&[
            ("search_tools".into(), "{\"tools_found\":0}".into()),
            (
                "web_search".into(),
                "{\"summary\":\"今日多云\",\"note\":\"GROUNDING RULE\"}".into(),
            ),
            ("calendar_reader".into(), "   ".into()),
            ("recall_memory".into(), "plain text".into()),
        ]);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["E2", "E4"]);
        assert_eq!(items[0].excerpt, "今日多云");
        assert_eq!(items[1].excerpt, "plain text");
    }

    #[test]
    fn web_search_evidence_hides_the_model_instruction() {
        let output = serde_json::json!({
            "status": "ok",
            "search_prompt": "Weather forecast for Example City, Washington for tomorrow. Return the exact forecast numbers.",
            "summary": "**Saturday**: Sunny, high 72°F. Use only these excerpts. Do not fill a gap from memory.",
            "retryable": false,
            "note": "Use only these excerpts. Do not fill a gap from memory. Do not call web_search again in this turn.",
            "owner_query": "明天天气怎么样?",
            "verified_citations": [{
                "title": "",
                "url": "https://weather.gov/sew",
                "snippet": "**Saturday**: Sunny, high 72°F."
            }]
        });
        let items = evidence_items(&[("web_search".into(), output.to_string())]);
        let excerpt = &items[0].excerpt;
        assert!(excerpt.contains("明天天气怎么样?"), "{excerpt}");
        assert!(excerpt.contains("weather.gov"), "{excerpt}");
        assert!(excerpt.contains("Sunny"), "{excerpt}");
        assert!(!excerpt.contains("**"), "{excerpt}");
        assert!(!excerpt.contains("Use only these excerpts"), "{excerpt}");
        assert!(!excerpt.contains("Return the exact"), "{excerpt}");
        assert!(!excerpt.contains("Retryable"), "{excerpt}");
        assert!(!excerpt.contains("可原样重试"), "{excerpt}");
        assert!(excerpt.chars().count() < 600, "{}", excerpt.chars().count());
    }

    #[test]
    fn evidence_excerpt_drops_model_instructions_and_keeps_facts() {
        let instructed = serde_json::json!({
            "summary": "本地估计 1.20",
            "query": "天气",
            "note": "Treat it as incomplete. Do not recompute the billed total.",
            "analysis_hint": "Do not recompute provider_usage_usd from tokens.",
            "interpretation_warning": "Do not recompute the provider total from prompt tokens."
        });
        let excerpt = evidence_excerpt(&instructed.to_string());
        assert!(excerpt.contains("本地估计 1.20"), "{excerpt}");
        assert!(excerpt.contains("天气"), "{excerpt}");
        assert!(
            !excerpt.to_ascii_lowercase().contains("do not"),
            "{excerpt}"
        );
        assert!(
            !excerpt.to_ascii_lowercase().contains("treat it as"),
            "{excerpt}"
        );
        assert!(
            !excerpt.to_ascii_lowercase().contains("analysis"),
            "{excerpt}"
        );

        let factual = serde_json::json!({"summary": "ok", "note": "line one"});
        let kept = evidence_excerpt(&factual.to_string());
        assert!(kept.contains("line one"), "{kept}");
    }

    #[test]
    fn task_evidence_formats_facts_before_clipping_json() {
        let output = serde_json::json!({"count": 2, "status": "ok", "status_filter": "all",
            "schema_version": 1, "execution_mode": "inline", "trigger_type": "at",
            "tasks": [{"title": "Call family", "status": "fired", "id": 123,
                "group_key": "internal-routing", "trigger_at_unix": 1788750000_i64},
                {"title": "Renew document", "due_date": "2026-12-01", "body": "Bring forms"}]});
        let excerpt = evidence_excerpt(&output.to_string());
        for fact in [
            "Count: 2",
            "Call family",
            "Status: fired",
            "Id: 123",
            "Group key: internal-routing",
            "Scheduled: 2026-",
            "Renew document",
            "Due: 2026-12-01",
            "Bring forms",
        ] {
            assert!(excerpt.contains(fact), "missing {fact}: {excerpt}");
        }
        assert!(!excerpt.contains("group_key"));
        assert!(!excerpt.contains("status_filter"));
        assert!(!excerpt.contains("schema_version"));
        assert!(!excerpt.contains("execution_mode"));
        assert!(!excerpt.contains("trigger_type"));
        assert!(evidence_excerpt(r#"{"budgets":[10,20]}"#).contains("Budgets:"));
        assert!(!excerpt.contains('{'));
        assert_eq!(
            evidence_excerpt("{\"tasks\":[]}"),
            "No source details returned."
        );
    }

    #[test]
    fn evidence_summary_retains_sibling_facts_and_nested_content() {
        let output = serde_json::json!({
            "status": "error", "summary": "Lookup did not finish",
            "url": "https://example.com/report", "hint": "Reconnect first",
            "content": [{"type": "text", "text": "{\"summary\":\"Nested finding\",\"count\":0}"}]
        });
        let excerpt = evidence_excerpt(&output.to_string());
        for fact in [
            "Status: error",
            "Lookup did not finish",
            "https://example.com/report",
            "Reconnect first",
            "Nested finding",
            "Count: 0",
        ] {
            assert!(excerpt.contains(fact), "missing {fact}: {excerpt}");
        }
        assert!(!excerpt.contains("Type: text"));
        assert!(!excerpt.contains('{'));
    }

    #[test]
    fn source_failure_and_partial_access_survive_long_descriptions() {
        for output in [
            serde_json::json!({"title": "Mailbox ".repeat(100), "status": "error",
                "error": "Permission denied", "summary": "No count was obtained"}),
            serde_json::json!({"summary": "Message preview ".repeat(100), "status": "partial",
                "warnings": ["Inbox only; other labels could not be read"]}),
        ] {
            let excerpt = evidence_excerpt(&output.to_string());
            let limitation = if output["status"] == "error" {
                "Permission denied"
            } else {
                "Inbox only; other labels could not be read"
            };
            assert!(
                excerpt.contains(limitation),
                "source limitation lost: {excerpt}"
            );
            assert!(excerpt.chars().count() <= EXCERPT_CHAR_CAP);
        }
    }

    #[test]
    fn evidence_arrays_keep_every_entry_that_fits() {
        let items = serde_json::json!((0..18).collect::<Vec<_>>());
        let excerpt = evidence_excerpt(&items.to_string());
        assert_eq!(
            excerpt
                .lines()
                .filter(|line| line.starts_with("- "))
                .count(),
            18
        );
        assert!(excerpt.contains("- 0"));
        assert!(excerpt.contains("- 17"));
        assert!(!excerpt.contains("Showing "));
    }

    #[test]
    fn evidence_arrays_name_how_many_entries_fit() {
        let items: Vec<serde_json::Value> = (0..40)
            .map(|i| serde_json::Value::String(format!("Record-{i:02}-{}", "x".repeat(30))))
            .collect();
        let excerpt = evidence_excerpt(&serde_json::Value::Array(items).to_string());
        assert!(excerpt.contains("Record-00"), "{excerpt}");
        assert!(excerpt.contains(" of 40 source entries."), "{excerpt}");
        assert!(!excerpt.contains("Record-39"), "{excerpt}");
        assert!(
            excerpt.chars().count() <= EXCERPT_CHAR_CAP,
            "{}",
            excerpt.chars().count()
        );
    }

    #[test]
    fn empty_json_evidence_has_a_nonempty_owner_excerpt() {
        for output in ["[]", "null", "\"   \"", "[null]", "{}"] {
            let items = evidence_items(&[("web_search".into(), output.into())]);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].id, "E1");
            assert_eq!(items[0].excerpt, "No source details returned.");
            assert_eq!(items[0].full_len, output.chars().count());
        }
    }

    #[test]
    fn strip_unbacked_refs_keeps_only_backed_ids() {
        let items = evidence_items(&[
            ("search_tools".into(), "{}".into()),
            ("web_search".into(), "{\"summary\":\"x\"}".into()),
        ]);
        assert_eq!(
            strip_unbacked_refs("无法读取日历。[E1]", &items),
            "无法读取日历。"
        );
        assert_eq!(
            strip_unbacked_refs("多云 [E2]，约 3 度 [E1, E2]", &items),
            "多云 [E2]，约 3 度 [E2]"
        );
        assert_eq!(strip_unbacked_refs("无引用", &items), "无引用");
    }

    #[test]
    fn evidence_items_ids_order_and_excerpt_cap() {
        let long = "α".repeat(800);
        let items = evidence_items(&[
            ("web_search".into(), long.clone()),
            ("commute_traffic_monitor".into(), "  hello   world  ".into()),
            ("skill_check_api_usage".into(), "ok".into()),
        ]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "E1");
        assert_eq!(items[0].tool, "web_search");
        assert_eq!(items[0].label, "Web search");
        assert_eq!(items[0].excerpt.chars().count(), 600);
        assert_eq!(items[0].full_len, 800);
        assert_eq!(items[1].id, "E2");
        assert_eq!(items[1].label, "Commute check");
        assert_eq!(items[1].excerpt, "hello world");
        assert_eq!(items[1].full_len, "  hello   world  ".chars().count());
        assert_eq!(items[2].id, "E3");
        assert_eq!(items[2].label, "API cost tracker");

        let many: Vec<(String, String)> = (0..20)
            .map(|i| (format!("tool_{i}"), format!("out {i}")))
            .collect();
        let capped = evidence_items(&many);
        assert_eq!(capped.len(), 16);
        assert_eq!(capped[0].id, "E1");
        assert_eq!(capped[15].id, "E16");
        assert_eq!(capped[15].label, "Tool 15");
    }

    #[test]
    fn unsupported_commute_rounding_and_range() {
        let reply = "约 44 分钟 [E1]，8:00–9:30 出发约 60–80 分钟";
        let tools = vec![(
            "commute_traffic_monitor".into(),
            "drive time 43.70 minutes".into(),
        )];
        assert_eq!(
            unsupported_numbers(reply, &tools, ""),
            vec!["60–80".to_string()]
        );
    }

    #[test]
    fn unit_converted_numbers_are_supported() {
        let reply = "占用最大的目录：/home/ubuntu 64.9 GB，/opt/aegis-runtime 33.7 GB，/var/log 548 MB [E1]";
        let tools = vec![(
            "sysadmin_ops".into(),
            "64872\t/home/ubuntu\n33668\t/opt/aegis-runtime\n548\t/var/log".into(),
        )];
        assert!(unsupported_numbers(reply, &tools, "").is_empty());
        // A number that is no conversion of any source stays unsupported.
        assert_eq!(
            unsupported_numbers("合计约 108 GB [E1]", &tools, ""),
            vec!["108".to_string()]
        );
        // 43.7 minutes ↔ 2622 seconds.
        assert!(
            unsupported_numbers("约 44 分钟", &[("t".into(), "2622 seconds".into())], "")
                .is_empty()
        );
    }

    #[test]
    fn owner_text_supports_its_own_numbers() {
        let reply = "那就 7 点出门。";
        assert_eq!(unsupported_numbers(reply, &[], ""), vec!["7".to_string()]);
        assert!(unsupported_numbers(reply, &[], "我 7 点出门").is_empty());
    }

    #[test]
    fn percent_and_time_formats() {
        let tools = vec![(
            "web_search".into(),
            "success rate 15.0%, depart 6:30".into(),
        )];
        assert!(unsupported_numbers("约 15% ，6:30 出发", &tools, "").is_empty());
        assert_eq!(
            unsupported_numbers("约 18%", &[], ""),
            vec!["18%".to_string()]
        );
        assert!(unsupported_numbers("leave at 06:30", &tools, "").is_empty());
    }

    #[test]
    fn ignores_hash_and_no_references() {
        // "#147" / "No. 12" name a thing; they are not estimates the owner
        // should be asked to double-check (every reply carried the chip).
        let reply = "Evidence reply #147: 收到。工单 No. 12 已关闭，#3 待办。";
        assert!(unsupported_numbers(reply, &[], "").is_empty());
        assert_eq!(
            unsupported_numbers("#147 大约要 25 分钟", &[], ""),
            vec!["25".to_string()]
        );
    }

    #[test]
    fn ignores_years_list_markers_counts_and_markdown_urls() {
        let reply = "2026 is fine\n1. First\nSee [docs](https://example.com/path/99) and 3 件.";
        assert!(unsupported_numbers(reply, &[], "").is_empty());
    }

    #[test]
    fn nested_task_fields_stay_under_their_item() {
        let output = serde_json::json!({
            "tasks": [{"title": "Call family", "status": "fired", "count": 1}]
        });
        let excerpt = evidence_excerpt(&output.to_string());
        assert_eq!(excerpt, "- Call family\n  Status: fired\n  Count: 1");
        assert!(!excerpt.contains("- -"));
        assert!(!excerpt.contains('{'));
    }

    #[test]
    fn excerpt_omits_trailing_nodes_without_cutting_a_kept_fact() {
        let output = serde_json::json!({
            "status": "error",
            "error": "Permission denied",
            "summary": "Mailbox ".repeat(80),
        });
        let excerpt = evidence_excerpt(&output.to_string());
        assert!(
            excerpt.starts_with("Status: error\nError: Permission denied\n"),
            "{excerpt}"
        );
        assert!(
            excerpt.ends_with("Showing 2 of 3 source entries."),
            "{excerpt}"
        );
        assert!(!excerpt.contains("Mailbox"), "{excerpt}");
        assert!(excerpt.chars().count() <= EXCERPT_CHAR_CAP);
    }

    #[test]
    fn multiline_source_text_keeps_its_lines() {
        let output = serde_json::json!({"note": "line one\nStatus: fired"});
        let excerpt = evidence_excerpt(&output.to_string());
        assert!(excerpt.contains("```"), "{excerpt}");
        assert!(excerpt.contains("line one\n"), "{excerpt}");
        assert!(excerpt.contains("Status: fired"), "{excerpt}");
        assert!(!excerpt.contains("line one Status"), "{excerpt}");
    }

    #[test]
    fn oversized_first_node_is_cut_on_a_character_boundary() {
        let text = "a".repeat(700);
        let excerpt = evidence_excerpt(&serde_json::Value::String(text).to_string());
        assert!(
            excerpt.ends_with("Further source entries are omitted."),
            "{excerpt}"
        );
        assert!(excerpt.chars().count() <= EXCERPT_CHAR_CAP);
        assert!(!excerpt.contains("Showing "));
    }

    #[test]
    fn json_string_unwrap_does_not_spend_an_extra_depth_level() {
        let mut value = serde_json::Value::String(r#"{"summary":"Deep fact"}"#.into());
        for _ in 0..5 {
            value = serde_json::json!({ "content": value });
        }
        let excerpt = evidence_excerpt(&value.to_string());
        assert!(excerpt.contains("Deep fact"), "{excerpt}");
        assert!(!excerpt.contains("Further nested"), "{excerpt}");
    }
}
