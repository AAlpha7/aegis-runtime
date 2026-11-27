//! Owner-configured web search routing.
//!
//! Automatic search follows the chat subscription without a secondary service.
//! Grok uses the chat device-login credential; Codex uses app-server.
//! The OpenRouter native-search slug is
//! sent only when `web_search_provider` is explicitly `openrouter`, and that
//! request always carries an output cap plus an explicit reasoning effort so
//! one search cannot run uncapped at the provider's high default.
//!
//! Model id literals live in `llm_client` (the allowlisted catalog). This
//! module only routes and builds request bodies.

use serde_json::{Value, json};

pub use crate::llm_client::WEB_SEARCH_MODEL as WEB_SEARCH_OPENROUTER_MODEL;
pub use crate::llm_client::WEB_SEARCH_SUBSCRIPTION_MODEL;

/// Explicit Grok subscription search.
pub const WEB_SEARCH_PROVIDER_SUBSCRIPTION: &str = "xai_subscription";
/// Setting `web_search_provider` when the owner opts into OpenRouter.
pub const WEB_SEARCH_PROVIDER_OPENROUTER: &str = "openrouter";
pub const WEB_SEARCH_PROVIDER_CODEX: &str = "codex_oauth";

/// Resolve automatic routing without turning an API-key chat into a paid search.
pub fn selected_provider<'a>(provider: &'a str, chat: &str) -> &'a str {
    if provider != "auto" && !provider.is_empty() {
        return provider;
    }
    match chat {
        "xai_subscription" => WEB_SEARCH_PROVIDER_SUBSCRIPTION,
        "codex_oauth" => WEB_SEARCH_PROVIDER_CODEX,
        _ => "unconfigured",
    }
}

/// Hard cap on one search completion. Shared by both providers so a single
/// search cannot emit the uncapped reasoning trace that billed ~$2 on
/// OpenRouter's high default.
pub const WEB_SEARCH_OUTPUT_TOKEN_CAP: u32 = 8192;

/// Explicit effort below xAI's implicit `high` default.
pub const WEB_SEARCH_REASONING_EFFORT: &str = "low";

pub const WEB_SEARCH_MODEL_MAX_LEN: usize = 200;

/// Search must supply usable public links.
pub fn parse_search_results(raw: &str) -> Option<(String, Vec<Value>)> {
    let value: Value = serde_json::from_str(raw).ok()?;
    if value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| s != "ok")
    {
        return None;
    }
    let results = value.get("results")?.as_array()?;
    let citations: Vec<Value> = results.iter().filter(|item| {
        item.get("url").and_then(Value::as_str).and_then(|url| url::Url::parse(url).ok())
            .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                && url.username().is_empty() && url.password().is_none())
    }).take(8).map(|item| json!({"url":item["url"], "title":item.get("title").and_then(Value::as_str).unwrap_or(""), "snippet":item.get("snippet").and_then(Value::as_str).unwrap_or("")})).collect();
    if citations.is_empty() {
        return None;
    }
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            results
                .iter()
                .take(8)
                .filter_map(|item| item.get("snippet").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        });
    Some((summary, citations))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSearchRoute {
    Codex {
        model: String,
    },
    Unavailable,
    Subscription {
        model: String,
    },
    OpenRouter {
        model: String,
        max_tokens: u32,
        reasoning_effort: &'static str,
    },
}

pub fn normalize_web_search_provider(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "auto" => Some("auto"),
        WEB_SEARCH_PROVIDER_CODEX => Some(WEB_SEARCH_PROVIDER_CODEX),
        WEB_SEARCH_PROVIDER_SUBSCRIPTION => Some(WEB_SEARCH_PROVIDER_SUBSCRIPTION),
        WEB_SEARCH_PROVIDER_OPENROUTER => Some(WEB_SEARCH_PROVIDER_OPENROUTER),
        _ => None,
    }
}

fn model_charset_ok(raw: &str) -> bool {
    !raw.is_empty()
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-'))
}

fn bare_model_id(model: &str) -> String {
    let trimmed = model.trim();
    let without_online = trimmed.strip_suffix(":online").unwrap_or(trimmed);
    without_online
        .rsplit('/')
        .next()
        .unwrap_or(without_online)
        .trim()
        .to_string()
}

/// Subscription model id. Non-grok slugs (for example an OpenRouter vendor
/// path the owner had selected) resolve to the subscription default.
pub fn subscription_model_id(model: &str) -> String {
    let bare = bare_model_id(model);
    if bare.starts_with("grok-") {
        bare
    } else {
        WEB_SEARCH_SUBSCRIPTION_MODEL.to_string()
    }
}

/// OpenRouter slug with the native `:online` suffix. Bare ids gain the vendor
/// prefix defined beside the other search slugs.
pub fn openrouter_online_slug(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return WEB_SEARCH_OPENROUTER_MODEL.to_string();
    }
    crate::llm_client::openrouter_vendor_online_slug(trimmed)
}

pub fn resolve_web_search_route(provider: &str, model: &str) -> WebSearchRoute {
    if provider == WEB_SEARCH_PROVIDER_CODEX {
        return WebSearchRoute::Codex {
            model: model.to_string(),
        };
    }
    if !matches!(
        provider,
        WEB_SEARCH_PROVIDER_SUBSCRIPTION | WEB_SEARCH_PROVIDER_OPENROUTER
    ) {
        return WebSearchRoute::Unavailable;
    }
    if normalize_web_search_provider(provider) == Some(WEB_SEARCH_PROVIDER_OPENROUTER) {
        WebSearchRoute::OpenRouter {
            model: openrouter_online_slug(model),
            max_tokens: WEB_SEARCH_OUTPUT_TOKEN_CAP,
            reasoning_effort: WEB_SEARCH_REASONING_EFFORT,
        }
    } else {
        // Only an explicit Grok route reaches the subscription here.
        WebSearchRoute::Subscription {
            model: subscription_model_id(model),
        }
    }
}

pub fn effective_web_search(provider: &str, model: &str) -> (&'static str, String) {
    match resolve_web_search_route(provider, model) {
        WebSearchRoute::Codex { .. } => (WEB_SEARCH_PROVIDER_CODEX, String::new()),
        WebSearchRoute::Unavailable => ("auto", String::new()),
        WebSearchRoute::Subscription { model } => (WEB_SEARCH_PROVIDER_SUBSCRIPTION, model),
        WebSearchRoute::OpenRouter { model, .. } => (WEB_SEARCH_PROVIDER_OPENROUTER, model),
    }
}

/// Normalize an owner-supplied model for the given provider. Empty means the
/// provider default.
pub fn normalize_saved_web_search_model(provider: &str, raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > WEB_SEARCH_MODEL_MAX_LEN {
        return Err("web_search_model must be at most 200 characters.");
    }
    if !trimmed.is_empty() && !model_charset_ok(trimmed) {
        return Err("web_search_model contains unsupported characters.");
    }
    let (effective_provider, model) = effective_web_search(provider, trimmed);
    if effective_provider == WEB_SEARCH_PROVIDER_SUBSCRIPTION && !model.starts_with("grok-") {
        return Err("subscription search model must be a grok model id.");
    }
    Ok(model)
}

pub fn openrouter_search_body(
    model: &str,
    prompt: &str,
    max_tokens: u32,
    reasoning_effort: &str,
) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
        "reasoning_effort": reasoning_effort,
        "reasoning": {"effort": reasoning_effort},
    })
}

pub fn subscription_search_body(model: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "input": [{"role": "user", "content": prompt}],
        "tools": [{"type": "web_search"}],
        "max_output_tokens": WEB_SEARCH_OUTPUT_TOKEN_CAP,
        "reasoning": {"effort": WEB_SEARCH_REASONING_EFFORT},
    })
}

pub fn subscription_not_signed_in_result() -> String {
    json!({
        "status": "error",
        "error": "Grok 订阅尚未登录，网页搜索没有改走 OpenRouter。请在设置的「服务」里登录 Grok，或把搜索服务改成 OpenRouter。 Grok subscription is not signed in. Web search was not sent to OpenRouter. Sign in to Grok under Settings → Services, or set the search provider to OpenRouter.",
        "reason": "xai_subscription_not_signed_in",
        "provider": WEB_SEARCH_PROVIDER_SUBSCRIPTION,
    })
    .to_string()
}

pub fn subscription_auth_unusable_result() -> String {
    json!({
        "status": "error",
        "error": "Grok 订阅登录无法使用，网页搜索没有改走 OpenRouter。请在设置里重新登录 Grok。 Grok subscription sign-in could not be used. Web search was not sent to OpenRouter. Sign in to Grok again in Settings.",
        "reason": "xai_subscription_auth_unusable",
        "provider": WEB_SEARCH_PROVIDER_SUBSCRIPTION,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_parser_rejects_errors_empty_results_and_non_web_links() {
        assert_eq!(
            normalize_saved_web_search_model("codex_oauth", "grok-4.7").unwrap(),
            ""
        );
        for raw in [
            r#"{"status":"error","results":[{"url":"https://example.com"}]}"#,
            r#"{"status":"ok","results":[]}"#,
            r#"{"results":[{"url":"file:///etc/passwd"},{"url":"javascript:alert(1)"}]}"#,
            "a plausible answer without source evidence",
        ] {
            assert!(parse_search_results(raw).is_none());
        }
        let (summary, sources) = parse_search_results(r#"{"status":"ok","results":[{"url":"https://example.com","title":"Example","snippet":"Verified excerpt"}]}"#).unwrap();
        assert_eq!(summary, "Verified excerpt");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["snippet"], "Verified excerpt");
    }

    #[test]
    fn automatic_route_follows_subscription_and_api_chat_requires_explicit_search() {
        assert_eq!(selected_provider("auto", "codex_oauth"), "codex_oauth");
        assert_eq!(
            selected_provider("auto", "xai_subscription"),
            "xai_subscription"
        );
        assert_eq!(selected_provider("auto", "openrouter"), "unconfigured");
        assert_eq!(selected_provider("openrouter", "codex_oauth"), "openrouter");
        let route = resolve_web_search_route("", "");
        assert_eq!(route, WebSearchRoute::Unavailable);
        let (provider, model) = effective_web_search(
            WEB_SEARCH_PROVIDER_SUBSCRIPTION,
            WEB_SEARCH_SUBSCRIPTION_MODEL,
        );
        assert_eq!(provider, WEB_SEARCH_PROVIDER_SUBSCRIPTION);
        assert_eq!(model, "grok-4.7");
        assert!(!model.contains(":online"));
        assert!(!model.starts_with("x-ai/"));
    }

    #[test]
    fn unknown_provider_does_not_fall_through_to_openrouter() {
        let route = resolve_web_search_route("unknown", "grok-4.7");
        assert!(matches!(route, WebSearchRoute::Unavailable));
        assert!(normalize_web_search_provider("duckduckgo").is_none());
        assert_eq!(
            resolve_web_search_route("duckduckgo", ""),
            WebSearchRoute::Unavailable
        );
        assert!(normalize_web_search_provider("openrouter-online").is_none());
        assert!(normalize_web_search_provider("").is_none());
    }

    #[test]
    fn openrouter_choice_keeps_online_suffix_and_caps_output() {
        let route = resolve_web_search_route(WEB_SEARCH_PROVIDER_OPENROUTER, "grok-4.7");
        let WebSearchRoute::OpenRouter {
            model,
            max_tokens,
            reasoning_effort,
        } = route
        else {
            panic!("openrouter provider must stay on the :online route");
        };
        assert_eq!(model, WEB_SEARCH_OPENROUTER_MODEL);
        assert!(model.ends_with(":online"));
        assert_eq!(max_tokens, WEB_SEARCH_OUTPUT_TOKEN_CAP);
        assert_eq!(reasoning_effort, "low");
        assert_ne!(reasoning_effort, "high");

        let body =
            openrouter_search_body(&model, "latest rust release", max_tokens, reasoning_effort);
        assert_eq!(body["model"], WEB_SEARCH_OPENROUTER_MODEL);
        assert_eq!(body["max_tokens"], WEB_SEARCH_OUTPUT_TOKEN_CAP);
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("reasoning_effort").is_some());
        assert_ne!(body["reasoning_effort"], "high");

        assert_eq!(
            openrouter_online_slug("x-ai/grok-4.6"),
            "x-ai/grok-4.6:online"
        );
        assert_eq!(
            openrouter_online_slug("x-ai/grok-4.6:online"),
            "x-ai/grok-4.6:online"
        );
        assert_eq!(
            openrouter_online_slug("google/gemini-2.5-flash"),
            "google/gemini-2.5-flash:online"
        );
    }

    #[test]
    fn subscription_body_uses_responses_web_search_and_a_cap() {
        let body = subscription_search_body("grok-4.7", "query");
        assert_eq!(body["model"], "grok-4.7");
        assert_eq!(body["tools"][0]["type"], "web_search");
        assert_eq!(body["max_output_tokens"], WEB_SEARCH_OUTPUT_TOKEN_CAP);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("messages").is_none());
        assert_eq!(subscription_model_id("x-ai/grok-4.6:online"), "grok-4.6");
        assert_eq!(
            subscription_model_id("google/gemini-2.5-flash:online"),
            WEB_SEARCH_SUBSCRIPTION_MODEL
        );
    }

    #[test]
    fn unsigned_subscription_error_names_the_reason_and_refuses_openrouter() {
        let raw = subscription_not_signed_in_result();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["reason"], "xai_subscription_not_signed_in");
        assert_eq!(parsed["provider"], WEB_SEARCH_PROVIDER_SUBSCRIPTION);
        let error = parsed["error"].as_str().unwrap();
        assert!(error.contains("没有改走 OpenRouter"));
        assert!(error.contains("was not sent to OpenRouter"));
        assert!(!error.contains(":online"));
        assert!(!raw.contains("sk-"));
    }

    #[test]
    fn saved_model_normalizes_per_provider_without_accepting_junk() {
        assert_eq!(
            normalize_saved_web_search_model(WEB_SEARCH_PROVIDER_SUBSCRIPTION, " grok-4.6 ")
                .unwrap(),
            "grok-4.6"
        );
        assert_eq!(
            normalize_saved_web_search_model(WEB_SEARCH_PROVIDER_OPENROUTER, "grok-4.6").unwrap(),
            "x-ai/grok-4.6:online"
        );
        assert!(
            normalize_saved_web_search_model(WEB_SEARCH_PROVIDER_SUBSCRIPTION, "not a model")
                .is_err()
        );
        assert!(
            normalize_saved_web_search_model(WEB_SEARCH_PROVIDER_OPENROUTER, &"a".repeat(201))
                .is_err()
        );
    }
}
