//! One owner turn's public web search, and the separate research job.
//!
//! A normal turn shows one outcome: a receipt on the reply, or the existing
//! approval card. Public search runs at most twice. The second lookup runs
//! only when the first is empty or clearly off the query. Several model
//! queries collapse to one broad public query; if any fragment carries tasks,
//! messages, or secrets, the sensitivity gate sees that one combined outbound.

use serde_json::{Value, json};

pub const PUBLIC_LOOKUP_BUDGET: u8 = 2;
pub const RESEARCH_LOOKUP_BUDGET: u8 = 8;
pub const SOURCE_LIMIT: usize = 5;
pub const EXCERPT_CHARS: usize = 280;

pub const RECEIPT_BEGIN: &str = "@@search-receipt";
pub const RECEIPT_END: &str = "@@end-search-receipt";
pub const NOT_SENT: &str = "这次没有发出去。";
pub const PROGRESS_PREFIX: &str = "正在核对：";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExcerpt {
    pub title: String,
    pub url: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupRecord {
    pub query: String,
    pub source: String,
    pub fallback_used: bool,
    pub sources: Vec<SourceExcerpt>,
    pub answered: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchTurnLedger {
    pub lookups_used: u8,
    pub records: Vec<LookupRecord>,
    pub suppress_outbound: bool,
    pub query_override: Option<String>,
    pub gate_args_override: Option<String>,
    pub force_sensitive_hold: bool,
    pub promote_research: bool,
    /// The owner's question for this turn. Receipts show this, not the
    /// model's search-tool instruction.
    pub owner_question: String,
}

impl SearchTurnLedger {
    pub fn begin_round(&mut self) {
        self.query_override = None;
        self.gate_args_override = None;
        self.force_sensitive_hold = false;
    }

    pub fn both_public_lookups_missed(&self) -> bool {
        self.lookups_used >= PUBLIC_LOOKUP_BUDGET
            && self.records.len() >= PUBLIC_LOOKUP_BUDGET as usize
            && self
                .records
                .iter()
                .all(|record| !record.answered && !record.failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchFragment {
    pub id: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchRoundPlan {
    pub keep_id: String,
    pub search_query: String,
    pub gate_query: String,
    pub force_sensitive_hold: bool,
    pub skip_ids: Vec<String>,
}

pub fn owner_asked_for_research(message: &str) -> bool {
    let text = message.trim();
    if text.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    lower.contains("深入调研")
        || lower.contains("帮我调研")
        || lower.contains("做个调研")
        || lower.contains("调研一下")
        || lower.contains("deep research")
        || lower.contains("research this")
}

pub fn owner_wants_checkable_answer(message: &str) -> bool {
    let text = message.trim();
    if text.chars().count() < 4 {
        return false;
    }
    let lower = text.to_lowercase();
    !matches!(
        lower.as_str(),
        "你好" | "谢谢" | "感谢" | "好的" | "ok" | "okay" | "thanks" | "thank you" | "hello" | "hi"
    )
}

/// Explicit current web-lookup requests, not generic mentions of search.
pub fn owner_requests_web_lookup(message: &str) -> bool {
    let lower = message.trim().to_lowercase();
    if [
        "explain",
        "translate",
        "what is",
        "how does",
        "how to",
        "解释",
        "请解释",
        "翻译",
        "请翻译",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || [
            "do not search",
            "don't search",
            "do not use web_search",
            "don't use web_search",
            "不要搜索",
            "不用搜索",
            "不要联网",
            "不用联网",
            "不要使用 web_search",
        ]
        .iter()
        .any(|text| lower.contains(text))
    {
        return false;
    }
    [
        "use web_search",
        "call web_search",
        "search the web",
        "search online",
        "web search for",
        "look up online",
        "search again now",
        "联网搜索",
        "联网查",
        "网上搜索",
        "上网搜索",
        "上网查",
        "在线搜索",
        "使用 web_search",
        "调用 web_search",
    ]
    .iter()
    .any(|text| lower.contains(text))
}

pub fn missing_requested_lookup_reply(language: &str) -> String {
    if language.starts_with("zh") {
        "本轮未实际执行网页搜索，无法核实来源或搜索提供商。请重试。".into()
    } else {
        "No web search ran for this request, so the sources and search provider have not been verified. Please retry.".into()
    }
}

#[cfg(test)]
mod explicit_lookup_request_tests {
    #[test]
    fn recognizes_requests_without_turning_explanations_or_opt_out_into_searches() {
        for text in [
            "Use web_search to find IANA docs",
            "Call web_search now",
            "请联网搜索官方文档",
            "请上网查来源",
        ] {
            assert!(super::owner_requests_web_lookup(text), "{text}");
        }
        for text in [
            "Explain how to use web_search",
            "请解释如何使用 web_search",
            "Do not use web_search",
            "不要联网搜索",
            "搜索设置是什么",
            "你好",
        ] {
            assert!(!super::owner_requests_web_lookup(text), "{text}");
        }
    }
}

pub fn fragment_has_private_material(text: &str, secrets: &[String]) -> bool {
    if secrets.iter().any(|secret| {
        secret.len() >= 12 && !secret.trim().is_empty() && text.contains(secret.as_str())
    }) {
        return true;
    }
    let lower = text.to_lowercase();
    const MARKERS: &[&str] = &[
        "begin private key",
        "begin rsa private key",
        "api_key=",
        "sk-or-",
        "sk-ant-",
        "[tool_result",
        "任务：",
        "任务:",
        "我的任务",
        "我的消息",
        "发件人",
        "[引用消息]",
    ];
    MARKERS
        .iter()
        .any(|marker| text.contains(marker) || lower.contains(marker))
}

pub fn plan_web_search_round(
    fragments: &[WebSearchFragment],
    secrets: &[String],
) -> Option<WebSearchRoundPlan> {
    let usable: Vec<&WebSearchFragment> = fragments
        .iter()
        .filter(|fragment| !fragment.query.trim().is_empty())
        .collect();
    if usable.is_empty() {
        return None;
    }
    let private = usable
        .iter()
        .any(|fragment| fragment_has_private_material(&fragment.query, secrets));
    let broad = broadest_query(usable.iter().map(|fragment| fragment.query.as_str()));
    let keep = usable
        .iter()
        .min_by_key(|fragment| specificity(&fragment.query))?;
    let gate_query = if private {
        usable
            .iter()
            .map(|fragment| fragment.query.trim())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        broad.clone()
    };
    let skip_ids = usable
        .iter()
        .filter(|fragment| fragment.id != keep.id)
        .map(|fragment| fragment.id.clone())
        .collect();
    Some(WebSearchRoundPlan {
        keep_id: keep.id.clone(),
        search_query: if private { gate_query.clone() } else { broad },
        gate_query,
        force_sensitive_hold: private,
        skip_ids,
    })
}

pub fn folded_search_result(held: bool) -> String {
    if held {
        json!({
            "status": "not_sent",
            "retryable": false,
            "error": "没有另开发出。敏感内容停在同一张批准上。",
        })
        .to_string()
    } else {
        json!({
            "status": "not_sent",
            "retryable": false,
            "error": "没有另发。这一轮收成了一次公开搜索。",
        })
        .to_string()
    }
}

pub fn outbound_stopped_result() -> String {
    json!({
        "status": "not_sent",
        "retryable": false,
        "error": NOT_SENT,
    })
    .to_string()
}

pub fn budget_stopped_result() -> String {
    json!({
        "status": "error",
        "retryable": false,
        "error": "这一轮的公开搜索已经用完，没有再发。",
    })
    .to_string()
}

pub fn cap_sources(citations: &[Value], _summary: &str) -> Vec<SourceExcerpt> {
    let mut sources = Vec::new();
    let mut seen = Vec::new();
    for citation in citations {
        if sources.len() >= SOURCE_LIMIT {
            break;
        }
        let url = citation_url(citation);
        if url.is_empty() {
            continue;
        }
        let key = normalize_url(&url);
        if seen.iter().any(|existing: &String| existing == &key) {
            continue;
        }
        seen.push(key);
        let title = citation
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let snippet = citation
            .get("snippet")
            .or_else(|| citation.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("");
        sources.push(SourceExcerpt {
            title: clip(&title, 120),
            url,
            excerpt: clip(snippet, EXCERPT_CHARS),
        });
    }
    sources
}

pub fn lookup_from_payload(query: &str, payload: &str) -> LookupRecord {
    let parsed: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let source = parsed
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let fallback_used = parsed
        .get("fallback_used")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let status = parsed.get("status").and_then(Value::as_str).unwrap_or("");
    if status != "ok" {
        return LookupRecord {
            query: query.to_string(),
            source,
            fallback_used,
            sources: Vec::new(),
            answered: false,
            failed: true,
        };
    }
    let summary = parsed.get("summary").and_then(Value::as_str).unwrap_or("");
    let citations = parsed
        .get("verified_citations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let sources = cap_sources(&citations, summary);
    let answered = lookup_answered(query, &sources, summary);
    LookupRecord {
        query: query.to_string(),
        source,
        fallback_used,
        sources,
        answered,
        failed: false,
    }
}

pub fn needs_second_lookup(record: &LookupRecord) -> bool {
    !record.failed && !record.answered
}

pub fn lookup_answered(query: &str, sources: &[SourceExcerpt], summary: &str) -> bool {
    if is_no_result_phrase(summary) && sources.iter().all(|source| source.url.is_empty()) {
        return false;
    }
    if sources.is_empty() {
        return false;
    }
    let tokens = significant_tokens(query);
    if tokens.is_empty() {
        return !sources.is_empty() || !is_no_result_phrase(summary);
    }
    let haystack = source_haystack(sources, summary);
    tokens.iter().any(|token| haystack.contains(token.as_str()))
}

pub fn broaden_query(query: &str) -> String {
    let mut text = query.trim().to_string();
    for operator in ["site:", "filetype:", "intitle:"] {
        text = strip_operator(&text, operator);
    }
    text = text.replace(['"', '「', '」'], "");
    let collapsed = collapse_ws(text.trim());
    if !collapsed.is_empty() && collapsed != query.trim() {
        return collapsed;
    }
    format!("broader public sources for {}", query.trim())
}

pub const RESEARCH_JOB_SKILL: &str = "research_job";
pub const RESEARCH_STOP_SKILL: &str = "research_stop";

#[derive(Debug, Clone)]
pub struct NamedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub fn query_from_tool_args(args: &str) -> String {
    let value: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    value
        .get("search_prompt")
        .or_else(|| value.get("query"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn is_timeout_payload(payload: &str) -> bool {
    payload.contains("web search timed out")
}

pub fn public_lookup_open(ledger: &SearchTurnLedger) -> bool {
    if ledger.suppress_outbound || ledger.lookups_used >= PUBLIC_LOOKUP_BUDGET {
        return false;
    }
    !matches!(
        ledger.records.last(),
        Some(record) if record.answered || record.failed
    )
}

pub fn note_hold_result(ledger: &mut SearchTurnLedger, tool_text: &str) {
    if tool_text.contains("pending_human_approval") {
        ledger.suppress_outbound = true;
    }
}

/// Collapse every `web_search` in this round to one kept call.
/// Returns the tool-call ids that must not be sent.
pub fn arm_search_round(
    ledger: &mut SearchTurnLedger,
    calls: &[NamedToolCall],
    secrets: &[String],
) -> Vec<String> {
    let fragments: Vec<WebSearchFragment> = calls
        .iter()
        .filter(|call| call.name == "web_search")
        .map(|call| WebSearchFragment {
            id: call.id.clone(),
            query: query_from_tool_args(&call.arguments),
        })
        .collect();
    ledger.begin_round();
    let Some(plan) = plan_web_search_round(&fragments, secrets) else {
        return Vec::new();
    };
    ledger.query_override = Some(plan.search_query);
    ledger.gate_args_override = Some(
        json!({
            "query": plan.gate_query,
            "search_prompt": plan.gate_query,
        })
        .to_string(),
    );
    ledger.force_sensitive_hold = plan.force_sensitive_hold;
    plan.skip_ids
}

pub fn combined_sources(records: &[LookupRecord]) -> Vec<SourceExcerpt> {
    let mut sources = Vec::new();
    let mut seen = Vec::new();
    for record in records {
        for source in &record.sources {
            if sources.len() >= SOURCE_LIMIT.saturating_mul(records.len().max(1)) {
                break;
            }
            if source.url.is_empty() {
                sources.push(source.clone());
                continue;
            }
            let key = normalize_url(&source.url);
            if seen.iter().any(|existing: &String| existing == &key) {
                continue;
            }
            seen.push(key);
            sources.push(source.clone());
        }
    }
    sources
}

pub fn model_payload(
    query: &str,
    records: &[LookupRecord],
    provider_summary: &str,
    owner_question: &str,
) -> String {
    let sources = combined_sources(records);
    let summary = if sources.is_empty() {
        String::from("No verifiable sources returned. This lookup did not verify new information.")
    } else if sources.iter().all(|source| source.excerpt.is_empty())
        && !provider_summary.trim().is_empty()
    {
        clip(provider_summary, EXCERPT_CHARS)
    } else {
        sources
            .iter()
            .map(|source| {
                let title = if source.title.is_empty() {
                    source.url.as_str()
                } else {
                    source.title.as_str()
                };
                format!("{title}: {}", source.excerpt)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let citations: Vec<Value> = sources
        .iter()
        .filter(|source| !source.url.is_empty())
        .map(|source| {
            json!({
                "url": source.url,
                "title": source.title,
                "snippet": source.excerpt,
            })
        })
        .collect();
    let mut body = json!({
        "status": "ok",
        "search_services": search_services(records),
        "search_prompt": query,
        "summary": summary,
        "retryable": false,
        "note": "Use only these excerpts and the provider summary. The summary is a synthesis, not a quotation from any individual page. Do not fill a gap from memory. Do not call web_search again in this turn.",
    });
    let owner_query = clip(owner_question.trim(), 160);
    if !owner_query.is_empty() {
        body["owner_query"] = json!(owner_query);
    }
    if !citations.is_empty() {
        body["verified_citations"] = Value::Array(citations);
    }
    if records.len() > 1 {
        body["lookups"] = json!(
            records
                .iter()
                .map(|record| json!({
                    "query": record.query,
                    "answered": record.answered,
                }))
                .collect::<Vec<_>>()
        );
    }
    body.to_string()
}

pub fn append_receipt(reply: &str, records: &[LookupRecord]) -> String {
    append_receipt_with_question(reply, records, "")
}

pub fn append_receipt_with_question(
    reply: &str,
    records: &[LookupRecord],
    owner_question: &str,
) -> String {
    if records.is_empty() || reply.contains(RECEIPT_BEGIN) || reply.contains("等待你批准") {
        return reply.to_string();
    }
    let suffix = receipt_block_with_question(records, owner_question);
    if reply.trim().is_empty() {
        suffix
    } else {
        format!("{}\n\n{suffix}", reply.trim_end())
    }
}

pub fn receipt_block(records: &[LookupRecord]) -> String {
    receipt_block_with_question(records, "")
}

pub fn receipt_block_with_question(records: &[LookupRecord], owner_question: &str) -> String {
    let queries = receipt_queries(records, owner_question);
    let human = human_receipt(&queries, records);
    let payload = json!({
        "queries": queries,
        "services": search_services(records),
        "sources": records.iter().flat_map(|record| {
            record.sources.iter().map(|source| json!({
                "query": queries.first().cloned().unwrap_or_else(|| owner_facing_query(&record.query)),
                "title": owner_source_title(source),
                "url": source.url,
                "excerpt": owner_facing_excerpt(&source.excerpt),
            }))
        }).collect::<Vec<_>>(),
    });
    format!("{human}\n{RECEIPT_BEGIN}\n{payload}\n{RECEIPT_END}")
}

fn search_services(records: &[LookupRecord]) -> Vec<Value> {
    let mut services = Vec::new();
    for record in records {
        if record.source.is_empty() {
            continue;
        }
        let service = json!({"source":record.source,"fallback_used":record.fallback_used});
        if !services.contains(&service) {
            services.push(service);
        }
    }
    services
}

#[test]
fn search_service_survives_model_and_owner_receipts() {
    for (source, fallback) in [
        ("codex_subscription", false),
        ("openrouter_native_xai_search", false),
    ] {
        let raw = json!({"status":"ok","source":source,"fallback_used":fallback,
            "summary":"Serve documentation","verified_citations":[{"title":"Serve","url":"https://tailscale.com/docs"}]}).to_string();
        let record = lookup_from_payload("Serve", &raw);
        let records = vec![record];
        let model: Value = serde_json::from_str(&model_payload("Serve", &records, "", "")).unwrap();
        assert_eq!(
            model["search_services"][0],
            json!({"source":source,"fallback_used":fallback})
        );
        let receipt = receipt_block(&records);
        let payload = receipt
            .split(RECEIPT_BEGIN)
            .nth(1)
            .unwrap()
            .split(RECEIPT_END)
            .next()
            .unwrap();
        let owner: Value = serde_json::from_str(payload).unwrap();
        assert_eq!(owner["services"], model["search_services"]);
    }
}

pub fn missed_lookup_reply(records: &[LookupRecord]) -> String {
    let query = records
        .first()
        .map(|record| record.query.as_str())
        .unwrap_or("");
    let sources = human_source_lines(records);
    format!(
        "查了：{query}\n看过：\n{sources}\n答案不在这些来源里。\n\n{}",
        receipt_block(records)
    )
}

pub fn research_progress_line(scope: &str) -> String {
    format!("{PROGRESS_PREFIX}{}", clip(scope, 80))
}

pub fn research_queries(scope: &str, tried: &[String]) -> Vec<String> {
    let scope = scope.trim();
    let mut planned = vec![
        scope.to_string(),
        broaden_query(scope),
        format!("official source {scope}"),
        format!("{scope} primary document"),
        format!("{scope} documentation"),
        format!("{scope} announcement"),
        format!("{scope} reference"),
        format!("public record {scope}"),
    ];
    planned.retain(|query| !query.trim().is_empty());
    planned.dedup();
    planned.retain(|query| !tried.iter().any(|previous| previous.trim() == query.trim()));
    planned.truncate(RESEARCH_LOOKUP_BUDGET as usize);
    planned
}

pub fn research_result_text(scope: &str, records: &[LookupRecord], stopped_early: bool) -> String {
    let answered: Vec<&LookupRecord> = records.iter().filter(|record| record.answered).collect();
    let conclusion = if let Some(hit) = answered.first() {
        let excerpt = hit
            .sources
            .iter()
            .find(|source| !source.excerpt.is_empty())
            .map(|source| source.excerpt.as_str())
            .unwrap_or("");
        format!("结论：这些来源里和「{scope}」对得上的是：{excerpt}")
    } else {
        "结论：答案不在已看过的来源里。".to_string()
    };
    let missing = if answered.is_empty() {
        format!("没查到：{scope}")
    } else {
        "没查到：来源之外的部分没有补写。".to_string()
    };
    let stopped = if stopped_early {
        "\n停在这里，剩下的查找没有发出。"
    } else {
        ""
    };
    format!(
        "调研结果\n范围：{scope}\n{conclusion}\n最相关的来源：\n{}\n{missing}{stopped}\n\n{}",
        human_source_lines(records),
        receipt_block(records)
    )
}

pub fn failure_line(query: &str) -> String {
    format!("没搜到：{query}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchJobView {
    pub id: String,
    pub user_did: String,
    pub title: String,
    pub summary: String,
    pub status: String,
}

pub fn research_job_json(
    id: &str,
    user_did: &str,
    scope: &str,
    status: &str,
    summary: &str,
) -> Value {
    json!({
        "id": id,
        "user_did": user_did,
        "title": format!("调研：{}", clip(scope, 80)),
        "summary": summary,
        "status": status,
        "scope": scope,
    })
}

pub fn parse_research_job(raw: &str) -> Option<ResearchJobView> {
    let value: Value = serde_json::from_str(raw).ok()?;
    Some(ResearchJobView {
        id: value.get("id")?.as_str()?.to_string(),
        user_did: value
            .get("user_did")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("调研")
            .to_string(),
        summary: value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("done")
            .to_string(),
    })
}

fn receipt_queries(records: &[LookupRecord], owner_question: &str) -> Vec<String> {
    let owner = clip(owner_question.trim(), 160);
    if !owner.is_empty() {
        return vec![owner];
    }
    let mut queries = Vec::new();
    for record in records {
        let query = owner_facing_query(&record.query);
        if query.is_empty() || queries.iter().any(|existing: &String| existing == &query) {
            continue;
        }
        queries.push(query);
    }
    queries
}

fn human_receipt(queries: &[String], records: &[LookupRecord]) -> String {
    let shown = queries.join("；");
    format!("搜索收据\n查了：{shown}\n{}", human_source_lines(records))
}

/// True when `text` is an instruction for the model, not a question for the owner.
pub fn looks_like_model_instruction(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("use only these excerpts")
        || lower.contains("do not fill a gap")
        || lower.contains("do not call web_search")
        || lower.contains("return the exact")
        || lower.contains("grounding rule")
        || lower.contains("do not invent")
}

pub fn owner_facing_query(query: &str) -> String {
    let kept = instruction_free_text(query);
    clip(&kept, 160)
}

pub fn owner_facing_excerpt(text: &str) -> String {
    clip(&instruction_free_text(text), 180)
}

pub fn strip_markdown_markers(text: &str) -> String {
    let without_links = strip_markdown_links(text);
    collapse_ws(&without_links.replace("**", "").replace("__", ""))
}

fn instruction_free_text(text: &str) -> String {
    let plain = strip_markdown_markers(text);
    let kept: Vec<String> = split_sentences(&plain)
        .into_iter()
        .filter(|sentence| !looks_like_model_instruction(sentence))
        .collect();
    collapse_ws(&kept.join(" "))
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        buf.push(ch);
        if matches!(ch, '.' | '。' | '!' | '！' | '?' | '？' | '\n') {
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            buf.clear();
        }
    }
    let rest = buf.trim();
    if !rest.is_empty() {
        sentences.push(rest.to_string());
    }
    sentences
}

fn strip_markdown_links(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '['
            && let Some(end) = chars[index + 1..].iter().position(|ch| *ch == ']')
        {
            let label_end = index + 1 + end;
            if chars.get(label_end + 1) == Some(&'(')
                && let Some(url_end) = chars[label_end + 2..].iter().position(|ch| *ch == ')')
            {
                out.extend(chars[index + 1..label_end].iter());
                index = label_end + 2 + url_end + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn owner_source_title(source: &SourceExcerpt) -> String {
    let title = strip_markdown_markers(&source.title);
    if !title.is_empty() && title != "来源" && !title.eq_ignore_ascii_case("source") {
        return clip(&title, 120);
    }
    url_host(&source.url).unwrap_or_default()
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

fn human_source_lines(records: &[LookupRecord]) -> String {
    let mut lines = Vec::new();
    let mut index = 1usize;
    for record in records {
        if record.sources.is_empty() {
            lines.push(format!("{index}. （这一次没有来源）"));
            index += 1;
            continue;
        }
        for source in &record.sources {
            let named = owner_source_title(source);
            let title = if named.is_empty() {
                "来源"
            } else {
                named.as_str()
            };
            let excerpt = owner_facing_excerpt(&source.excerpt);
            if source.url.is_empty() {
                lines.push(format!("{index}. {title} — {excerpt}"));
            } else {
                lines.push(format!("{index}. {title} — {excerpt}\n   {}", source.url));
            }
            index += 1;
        }
    }
    if lines.is_empty() {
        "（没有来源）".to_string()
    } else {
        lines.join("\n")
    }
}

fn broadest_query<'a>(queries: impl Iterator<Item = &'a str>) -> String {
    queries
        .min_by_key(|query| specificity(query))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn specificity(query: &str) -> (u32, usize) {
    let mut score = 0u32;
    let lower = query.to_lowercase();
    if query.contains('"') || query.contains('「') {
        score += 5;
    }
    if lower.contains("site:") || lower.contains("filetype:") || lower.contains("intitle:") {
        score += 5;
    }
    if query.chars().filter(|ch| ch.is_ascii_digit()).count() >= 4 {
        score += 2;
    }
    score += (query.chars().count() / 80) as u32;
    (score, query.chars().count())
}

fn citation_url(citation: &Value) -> String {
    let raw = citation
        .as_str()
        .or_else(|| citation.get("url").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    match url::Url::parse(raw) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            raw.to_owned()
        }
        _ => String::new(),
    }
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

pub fn clip(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(max_chars).collect();
    format!("{clipped}…")
}

fn is_no_result_phrase(summary: &str) -> bool {
    let lower = summary.trim().to_lowercase();
    lower.is_empty()
        || lower == "(no results)"
        || lower.contains("no results")
        || lower.contains("没有结果")
        || lower.contains("未找到")
        || lower.contains("nothing matching")
}

fn source_haystack(sources: &[SourceExcerpt], summary: &str) -> String {
    let mut parts = vec![summary.to_lowercase()];
    for source in sources {
        parts.push(source.title.to_lowercase());
        parts.push(source.url.to_lowercase());
        parts.push(source.excerpt.to_lowercase());
    }
    parts.join("\n")
}

fn significant_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() {
            buf.push(ch);
        } else if !buf.is_empty() {
            push_token(&mut tokens, &buf);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        push_token(&mut tokens, &buf);
    }
    tokens
}

fn push_token(tokens: &mut Vec<String>, raw: &str) {
    let lower = raw.to_lowercase();
    const STOP: &[&str] = &[
        "the", "a", "an", "of", "to", "for", "and", "or", "in", "on", "what", "how", "latest",
        "please", "搜索", "一下", "的", "了", "是", "吗", "呢", "帮我",
    ];
    if STOP.contains(&lower.as_str()) {
        return;
    }
    let chars = lower.chars().count();
    if chars < 2 {
        return;
    }
    if lower.is_ascii() && lower.len() < 3 {
        return;
    }
    tokens.push(lower);
}

fn strip_operator(text: &str, operator: &str) -> String {
    let lower = text.to_lowercase();
    let Some(start) = lower.find(operator) else {
        return text.to_string();
    };
    let after = start + operator.len();
    let rest = text.get(after..).unwrap_or("");
    let end = rest
        .find(char::is_whitespace)
        .map(|index| after + index)
        .unwrap_or(text.len());
    let mut out = String::new();
    out.push_str(text.get(..start).unwrap_or(""));
    out.push_str(text.get(end..).unwrap_or(""));
    out
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Owner-facing text for a replayed search. Prefix is `result` or `failure`
/// so the chat card can show the lookup without keeping the tool JSON.
pub fn owner_search_outcome_text(reply: &str) -> String {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return "failure".to_string();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let status = value
            .get("status")
            .and_then(|item| item.as_str())
            .unwrap_or("");
        let error = value
            .get("error")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .trim();
        if status == "error" || !error.is_empty() {
            return if error.is_empty() {
                "failure".to_string()
            } else {
                format!("failure\n{}", clip(error, 500))
            };
        }
        if let Some(summary) = value.get("summary").and_then(|item| item.as_str()) {
            let summary = summary.trim();
            if !summary.is_empty() {
                return format!("result\n{}", clip(summary, 500));
            }
        }
    }
    if trimmed.starts_with("审批失败") || trimmed.starts_with('⏰') {
        return format!("failure\n{}", clip(trimmed, 500));
    }
    format!("result\n{}", clip(trimmed, 500))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_public_queries_become_one_broad_query() {
        let plan = plan_web_search_round(
            &[
                WebSearchFragment {
                    id: "a".into(),
                    query: "site:example.com \"rust 1.85.0 release notes\"".into(),
                },
                WebSearchFragment {
                    id: "b".into(),
                    query: "rust release".into(),
                },
            ],
            &[],
        )
        .unwrap();
        assert_eq!(plan.search_query, "rust release");
        assert_eq!(plan.gate_query, "rust release");
        assert!(!plan.force_sensitive_hold);
        assert_eq!(plan.skip_ids.len(), 1);
        assert!(!plan.search_query.contains("site:"));
        assert!(!plan.gate_query.contains('\n'));
    }

    #[test]
    fn a_private_fragment_holds_one_combined_outbound() {
        let secret = "sk-or-secret-value-123456";
        let plan = plan_web_search_round(
            &[
                WebSearchFragment {
                    id: "a".into(),
                    query: "rust release".into(),
                },
                WebSearchFragment {
                    id: "b".into(),
                    query: format!("任务：把合同寄出 {secret}"),
                },
            ],
            &[secret.to_string()],
        )
        .unwrap();
        assert!(plan.force_sensitive_hold);
        assert!(plan.gate_query.contains("rust release"));
        assert!(plan.gate_query.contains("任务："));
        assert!(plan.gate_query.contains(secret));
        assert_eq!(plan.skip_ids.len(), 1);
    }

    #[test]
    fn sources_keep_provider_order_and_short_excerpts() {
        let long = "x".repeat(2000);
        let citations = vec![
            json!({"url": "https://example.com/a/", "title": "A", "snippet": long}),
            json!({"url": "https://example.com/a", "title": "dup", "snippet": "again"}),
            json!({"url": "https://example.com/b", "title": "B", "snippet": "short"}),
            json!({"url": "https://example.com/c", "title": "C", "snippet": "c"}),
            json!({"url": "https://example.com/d", "title": "D", "snippet": "d"}),
            json!({"url": "https://example.com/e", "title": "E", "snippet": "e"}),
            json!({"url": "https://example.com/f", "title": "F", "snippet": "dropped"}),
        ];
        let sources = cap_sources(&citations, &"p".repeat(5000));
        assert_eq!(sources.len(), 5);
        assert_eq!(sources[0].url, "https://example.com/a/");
        assert!(sources[0].excerpt.chars().count() <= EXCERPT_CHARS + 1);
        assert!(!sources.iter().any(|source| source.url.contains("/f")));
        assert!(!sources[0].excerpt.contains(&"x".repeat(400)));
    }

    #[test]
    fn second_lookup_runs_only_when_empty_or_off_query() {
        let empty = LookupRecord {
            query: "rust release".into(),
            source: String::new(),
            fallback_used: false,
            sources: Vec::new(),
            answered: false,
            failed: false,
        };
        assert!(needs_second_lookup(&empty));
        let off = LookupRecord {
            query: "rust release".into(),
            source: String::new(),
            fallback_used: false,
            sources: vec![SourceExcerpt {
                title: "Baking".into(),
                url: "https://example.com/cake".into(),
                excerpt: "A sponge recipe".into(),
            }],
            answered: false,
            failed: false,
        };
        assert!(needs_second_lookup(&off));
        let hit = LookupRecord {
            query: "rust release".into(),
            source: String::new(),
            fallback_used: false,
            sources: vec![SourceExcerpt {
                title: "Rust release".into(),
                url: "https://blog.rust-lang.org".into(),
                excerpt: "Rust 1.85".into(),
            }],
            answered: true,
            failed: false,
        };
        assert!(!needs_second_lookup(&hit));
        let failed = LookupRecord {
            query: "rust release".into(),
            source: String::new(),
            fallback_used: false,
            sources: Vec::new(),
            answered: false,
            failed: true,
        };
        assert!(!needs_second_lookup(&failed));
        assert_ne!(broaden_query("rust release"), "rust release");
        assert!(!broaden_query("site:example.com rust").contains("site:"));
    }

    #[test]
    fn missed_lookups_name_the_query_and_do_not_invent() {
        let records = vec![LookupRecord {
            query: "zzzz-unique-topic".into(),
            source: String::new(),
            fallback_used: false,
            sources: vec![SourceExcerpt {
                title: "Unrelated".into(),
                url: "https://example.com/other".into(),
                excerpt: "not the topic".into(),
            }],
            answered: false,
            failed: false,
        }];
        let text = missed_lookup_reply(&records);
        assert!(text.contains("zzzz-unique-topic"));
        assert!(text.contains("https://example.com/other"));
        assert!(text.contains("答案不在这些来源里"));
        assert!(!text.contains("I remember"));
    }

    #[test]
    fn research_budget_stops_at_eight_distinct_lookups() {
        let queries = research_queries("rust release", &["rust release".into()]);
        assert!(queries.len() <= RESEARCH_LOOKUP_BUDGET as usize);
        assert!(!queries.iter().any(|query| query == "rust release"));
        let records = vec![LookupRecord {
            query: "official source rust release".into(),
            source: String::new(),
            fallback_used: false,
            sources: vec![SourceExcerpt {
                title: "Rust".into(),
                url: "https://blog.rust-lang.org".into(),
                excerpt: "Rust 1.85 release".into(),
            }],
            answered: true,
            failed: false,
        }];
        let done = research_result_text("rust release", &records, false);
        assert!(done.contains("结论"));
        assert!(done.contains("https://blog.rust-lang.org"));
        assert!(done.contains("没查到"));
        let stopped = research_result_text("rust release", &records, true);
        assert!(stopped.contains("剩下的查找没有发出"));
        assert!(research_progress_line("rust release").starts_with(PROGRESS_PREFIX));
    }

    #[test]
    fn ordinary_turn_budget_stays_two_after_research_constants() {
        assert_eq!(PUBLIC_LOOKUP_BUDGET, 2);
        assert_eq!(RESEARCH_LOOKUP_BUDGET, 8);
        assert_ne!(PUBLIC_LOOKUP_BUDGET, RESEARCH_LOOKUP_BUDGET);
        assert!(owner_asked_for_research("请深入调研这件事"));
        assert!(!owner_asked_for_research("搜索最新新闻"));
        assert!(!owner_wants_checkable_answer("谢谢"));
        assert!(owner_wants_checkable_answer("搜索最新AI新闻"));
    }

    #[test]
    fn receipt_shows_the_owner_question_not_the_model_instruction() {
        let instruction = "Weather forecast for Example City, Washington for tomorrow, Saturday. Return the exact forecast numbers. Do not invent a source.";
        let records = vec![LookupRecord {
            query: instruction.into(),
            source: String::new(),
            fallback_used: false,
            sources: vec![SourceExcerpt {
                title: String::new(),
                url: "https://weather.gov/sew".into(),
                excerpt: "**Saturday**: Sunny, high 72°F. Use only these excerpts. Do not fill a gap from memory.".into(),
            }],
            answered: true,
            failed: false,
        }];
        let block = receipt_block_with_question(&records, "明天天气怎么样?");
        assert!(block.contains("查了：明天天气怎么样?"));
        assert!(block.contains("weather.gov"));
        assert!(block.contains("Sunny"));
        assert!(!block.contains("**"));
        assert!(!block.contains("Return the exact"));
        assert!(!block.contains("Use only these excerpts"));
        assert!(!block.contains("Do not invent"));
        let payload = model_payload(
            instruction,
            &records,
            "**Saturday**: Sunny",
            "明天天气怎么样?",
        );
        assert!(payload.contains("明天天气怎么样?"));
        assert!(payload.contains("Use only these excerpts"));
        let outcome = owner_search_outcome_text(&payload);
        assert!(outcome.starts_with("result\n"));
        assert!(outcome.contains("Sunny"));
        assert!(!outcome.contains("\"note\""));
        assert_eq!(
            owner_search_outcome_text(r#"{"status":"error","error":"provider down"}"#),
            "failure\nprovider down"
        );
        assert_eq!(owner_search_outcome_text(""), "failure");
        assert!(owner_search_outcome_text("审批失败: timeout").starts_with("failure\n"));
    }
}

#[test]
fn plausible_prose_and_unsafe_links_never_become_search_evidence() {
    let raw = json!({"status":"ok", "summary":"Rust 1.99 released at https://example.com",
        "verified_citations":[{"url":"javascript:alert(1)"},{"url":"https://secret@example.com"}]})
    .to_string();
    let record = lookup_from_payload("Rust release", &raw);
    assert!(record.sources.is_empty());
    assert!(!record.answered);
    let payload = model_payload("Rust release", &[record], "Rust 1.99 is confirmed", "");
    assert!(!payload.contains("1.99"));
    assert!(payload.contains("did not verify new information"));
}

#[test]
fn search_summary_is_not_misattributed_to_the_first_page() {
    let sources = cap_sources(
        &[json!({"url":"https://example.com", "title":"Official"})],
        "I searched using Codex",
    );
    assert_eq!(sources.len(), 1);
    assert!(sources[0].excerpt.is_empty());
    let record = LookupRecord {
        query: "Official".into(),
        source: "codex_subscription".into(),
        fallback_used: false,
        sources,
        answered: true,
        failed: false,
    };
    let payload: Value = serde_json::from_str(&model_payload(
        "Official",
        &[record],
        "Provider synthesis",
        "",
    ))
    .unwrap();
    assert_eq!(payload["summary"], "Provider synthesis");
    assert_eq!(payload["verified_citations"][0]["snippet"], "");
}
