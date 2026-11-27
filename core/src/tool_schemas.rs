//! Tool schema builders for LLM function calling.
//!
//! This module contains all `build_*_tool()` functions that construct
//! `ChatCompletionTools` schema definitions for the orchestrator's tool
//! definitions. Extracted from orchestrator.rs for maintainability.

use anyhow::Result;
use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObjectArgs};

use crate::agent_skills::AgentSkill;
use crate::models::SkillCapability;

// ---------------------------------------------------------------------------
// Strict-provider schema validator
// ---------------------------------------------------------------------------

/// Verify `parameters.required ⊆ parameters.properties` for object schemas.
///
/// Gemini (Google AI Studio) rejects the entire request with HTTP 400
/// `INVALID_ARGUMENT` when any name in `required` is absent from `properties`.
/// OpenAI-compatible providers tolerate the divergence, which made this class
/// of bug invisible until the vision-routing path (commit 08f2e5d) started
/// forwarding tool schemas to Gemini. This helper is the canonical gate —
/// every tool-registration site that trusts an externally-authored schema
/// (Wasm skill capabilities, MCP server `input_schema`) must call it and
/// drop/reject non-conforming entries rather than poison the list.
///
/// Returns `Ok(())` when `required` is absent/empty, or when every name in
/// `required` appears as a key in `properties`. Returns `Err(msg)` otherwise.
///
/// Intentionally does NOT gate on `type == "object"`: Gemini enforces the
/// required-subset-of-properties invariant regardless of whether the schema
/// declares `type`, and real-world builders sometimes omit `type` or use
/// `type: ["object", "null"]` (non-string). A stricter check here protects
/// the LLM call from all those shapes.
pub(crate) fn validate_object_schema(
    schema: &serde_json::Value,
) -> std::result::Result<(), String> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    if required.is_empty() {
        return Ok(());
    }
    let properties_keys: std::collections::BTreeSet<&str> = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let missing: Vec<&str> = required
        .iter()
        .filter(|r| !properties_keys.contains(*r))
        .copied()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "required={:?} lists properties not declared in properties={:?}; missing={:?}",
            required, properties_keys, missing
        ))
    }
}

// ---------------------------------------------------------------------------
// Multimodal pseudo-tool definitions (TTS + Image Generation)
// ---------------------------------------------------------------------------

pub(crate) fn build_text_to_speech_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "description": "The text to convert to speech. Max 15000 characters."
            },
            "language": {
                "type": "string",
                "description": "BCP-47 language code (e.g. 'zh', 'en', 'ja') or 'auto' for detection. Default: 'auto'."
            }
        },
        "required": ["text"]
    });
    let function = FunctionObjectArgs::default()
        .name("text_to_speech")
        .description(
            "Convert text to speech audio and send as a voice message to the user. \
             ONLY use this when the user EXPLICITLY asks for voice/audio output \
             (e.g. '读给我听', '用语音回复', 'read it aloud', 'send as voice'). \
             Do NOT use for normal text replies.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_generate_image_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {
                "type": "string",
                "description": "Detailed description of the image to generate. Be specific about style, composition, lighting, etc."
            },
            "aspect_ratio": {
                "type": "string",
                "enum": ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
                "description": "Image aspect ratio. Default: '1:1'. Use '16:9' for landscape, '9:16' for portrait."
            }
        },
        "required": ["prompt"]
    });
    let function = FunctionObjectArgs::default()
        .name("generate_image")
        .description(
            "Generate an image from a text description and send it to the user. \
             Use when the user asks to create, draw, generate, or design an image.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `final_answer` sentinel tool for the subcall primitive.
///
/// Per `docs/subcall-primitive-design.md` §4.3, a subcall's LLM context
/// MUST call `final_answer(...)` to return structured output. The tool is
/// NOT dispatched to any executor — `run_autonomous_react_loop` intercepts
/// it, captures the parsed arguments into `OrchestratorResult::final_answer_args`,
/// and short-circuits the ReAct loop. The schema is deliberately permissive:
/// `structured_output` is a free-form object (the caller's `SubcallSpec.output_schema`
/// constrains the shape downstream via a presence-check validator, not here)
/// and `summary` is a short prose recap.
pub(crate) fn build_final_answer_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "structured_output": {
                "type": "object",
                "description": "The structured answer matching the caller's output_schema. \
                    Keys must match what the system prompt asked for."
            },
            "summary": {
                "type": "string",
                "description": "Short (1-3 sentence) human-readable summary of the answer. \
                    Used for recall + audit."
            }
        },
        "required": ["structured_output", "summary"]
    });

    let function = FunctionObjectArgs::default()
        .name("final_answer")
        .description(
            "Return the final structured output for this subcall and terminate the task. \
             You MUST call this exactly once when you have completed the task. \
             After calling it, the subcall ends immediately — no further reasoning or \
             tool calls will run. Populate `structured_output` with the shape requested \
             in the system prompt, and `summary` with a concise prose recap.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Names of pseudo-tools that shadow Wasm skills of the same name.
/// Wasm skills with these names are skipped in tool definitions.
pub(crate) const PSEUDO_SHADOW_NAMES: &[&str] = &[
    "web_search",
    "x_search",
    "personal_tasks",
    "configure_autonomous",
    "view_heartbeat_audit",
];

/// First sentence of a skill description, capped at `max_chars` (used for
/// deferred Wasm stubs so the tool table stays small).
pub(crate) fn skill_stub_summary(description: &str) -> String {
    const MAX_CHARS: usize = 120;
    let first = description
        .split_inclusive(['.', '。', '!', '！'])
        .next()
        .unwrap_or(description)
        .trim();
    if first.chars().count() <= MAX_CHARS {
        first.to_string()
    } else {
        let cut: String = first.chars().take(MAX_CHARS.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Build a lightweight stub tool definition (name + one-line description only).
/// ~50 tokens per stub vs ~300 for a full schema. Used for Tier 2 admin tools.
/// If called with empty args, auto-resolve returns a smart default or full schema.
pub(crate) fn build_stub_tool(name: &str, description: &str) -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    });
    let function = FunctionObjectArgs::default()
        .name(name)
        .description(format!(
            "[DEFERRED] {} — call with no args for default, or search_tools(\"{}\") for full schema.",
            description, name
        ))
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `search_tools` pseudo-tool: retrieves full schemas for deferred tools.
pub(crate) fn build_search_tools_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Tool name (exact match) or description of what you need (fuzzy match). Returns full schemas of up to 5 matching tools."
            }
        },
        "required": ["query"]
    });
    let function = FunctionObjectArgs::default()
        .name("search_tools")
        .description(
            "Search for tools marked [DEFERRED] in your tool list. Returns their full schemas \
             so you can call them. Use exact tool name for best results.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `web_search` pseudo-tool definition.
///
/// Uses the Agent-to-Agent delegation pattern: the main LLM writes a prompt
/// that is sent directly to a search model (Grok) with internet access.
/// This gives the main LLM full control over what kind of response it needs
/// (summary, verbatim quotes, structured data, etc.).
pub(crate) fn build_web_search_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "search_prompt": {
                "type": "string",
                "description": "A prompt sent directly to a search model (Grok) that has \
                 real-time internet access including X/Twitter. Write a clear instruction \
                 for what you need. Examples:\n\
                 - For summaries: 'Search for latest news about X and summarize the key points'\n\
                 - For verbatim quotes: 'Find Elon Musk latest tweet on X. Return the EXACT \
                   original text verbatim, do not paraphrase or summarize'\n\
                 - For data: 'What is the current Bitcoin price in USD?'\n\
                 The search model will search the web and X/Twitter, then respond according \
                 to your instructions."
            }
        },
        "required": ["search_prompt"]
    });

    let function = FunctionObjectArgs::default()
        .name("web_search")
        .description(
            "Delegate a search task to a real-time search model (Grok with web + X/Twitter access). \
             You are directly instructing another AI model — write a clear prompt telling it exactly \
             what information you need and in what format. The search model will search the internet \
             and X/Twitter in real-time and respond according to your instructions. \
             Use this for current events, recent news, social media posts, real-time data, \
             or anything beyond your training data cutoff. MUST use for: product pricing, \
             subscription plans, API tiers, feature availability, version numbers, release dates, \
             company announcements, and any verifiable factual claims about third-party services.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `x_search` pseudo-tool — public X (Twitter) search via Grok/xAI.
///
/// Phase A of X integration: read-only public posts (no user OAuth). Prefer this
/// over generic `web_search` when the owner needs posts, handles, or threads on X.
pub(crate) fn build_x_search_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "What to find on X (formerly Twitter). Keywords, topics, questions, \
                 or a paste of an x.com status URL to summarize. Required."
            },
            "allowed_x_handles": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional. Only consider posts from these handles (no @, max 20)."
            },
            "excluded_x_handles": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional. Exclude posts from these handles (no @, max 20). \
                 Mutually exclusive with allowed_x_handles."
            },
            "from_date": {
                "type": "string",
                "description": "Optional start date YYYY-MM-DD (inclusive)."
            },
            "to_date": {
                "type": "string",
                "description": "Optional end date YYYY-MM-DD (inclusive)."
            }
        },
        "required": ["query"]
    });

    let function = FunctionObjectArgs::default()
        .name("x_search")
        .description(
            "Search public posts on X (Twitter) via Grok/xAI — keyword, topics, handles, or threads. \
             Use for social signals, account updates, or 'what is @handle saying'. \
             Returns a grounded summary plus citations when available. \
             READ-ONLY public data: does NOT post, reply, like, or access the owner's private mentions. \
             For general web facts outside X, use web_search instead.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build a ChatCompletionTools from a SkillCapability.
pub(crate) fn build_tool_from_capability(cap: &SkillCapability) -> Result<ChatCompletionTools> {
    let mut schema = cap.payload_schema.clone();

    // Inject optional `account` parameter for multi-account skills (manifest-driven)
    if cap.supports_multi_account
        && let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut())
    {
        props.insert(
                "account".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Google account ID (e.g. 'personal', 'work'). Omit for default account."
                }),
            );
    }

    // Gemini compat: required names must all appear in properties. Run AFTER
    // multi-account injection so the added `account` property is visible.
    // On violation, warn + propagate Err; the orchestrator's filter_map drops
    // the Err case, so the malformed tool is silently omitted from the list
    // sent to the LLM instead of poisoning every Gemini request.
    if let Err(msg) = validate_object_schema(&schema) {
        tracing::warn!(
            skill = %cap.skill_name,
            error = %msg,
            "[tool_schemas] Skill has malformed input_schema — dropping from LLM tool list (Gemini strict-validation guard)"
        );
        anyhow::bail!(
            "skill {:?} has malformed input_schema: {msg}",
            cap.skill_name
        );
    }

    // Gemini compat: skills loaded without a manifest.json (e.g. active rows in
    // skill_proposals with all metadata NULL) get a default SkillCapability with
    // payload_schema = {}. Gemini/OpenRouter appears to synthesize `required`
    // entries from an empty parameters object and then rejects its own synthesis
    // with HTTP 400 ("required[N] references undefined property"). Normalizing
    // the empty object to the canonical "no-args" form {"type":"object",
    // "properties":{}} tells Gemini unambiguously that the tool accepts no
    // parameters.
    let is_empty = schema.as_object().is_some_and(|m| m.is_empty());
    if is_empty {
        tracing::info!(
            skill = %cap.skill_name,
            "[tool_schemas] Normalizing empty payload_schema to {{type:object,properties:{{}}}} for Gemini strict-validation compat"
        );
        schema = serde_json::json!({"type": "object", "properties": {}});
    }

    // Append limitations to description so LLM doesn't promise unsupported capabilities.
    let description = if cap.limitations.is_empty() {
        cap.description.clone()
    } else {
        let limits = cap
            .limitations
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "{}\n\nLIMITATIONS (do NOT promise beyond these):\n{}",
            cap.description, limits
        )
    };

    let function = FunctionObjectArgs::default()
        .name(&*cap.skill_name)
        .description(&description)
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `update_setting` pseudo-tool definition for LLM function calling.
pub(crate) fn build_update_setting_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "key": {
                "type": "string",
                "description": "Setting key (e.g. skill_server_monitor, user_timezone, display_language)"
            },
            "value": {
                "description": "New value: boolean for toggles, IANA string for user_timezone, number for numeric keys"
            }
        },
        "required": ["key", "value"]
    });

    let function = FunctionObjectArgs::default()
        .name("update_setting")
        .description(
            "Update a runtime setting (skill_<name> toggles, user_timezone, display_language, \
             mcp_enabled, embedding_*). HITL approval required. Call only when the owner asks, \
             or when timezone is clearly wrong (ask first). See the settings prompt module \
             for key constraints.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `switch_model` pseudo-tool definition for LLM function calling.
pub(crate) fn build_switch_model_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "model": {
                "type": "string",
                "description": "Full model ID (e.g. 'google/gemini-3.1-flash-lite-preview', 'minimax/minimax-m2.5', 'anthropic/claude-sonnet-4.6')"
            }
        },
        "required": ["model"]
    });

    let function = FunctionObjectArgs::default()
        .name("switch_model")
        .description(
            "Switch the LLM model. Use when the user asks to change the AI model. \
             Requires the user's explicit permission. Common models: \
             google/gemini-3.1-flash-lite-preview, minimax/minimax-m2.5, \
             anthropic/claude-sonnet-4.6",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `trigger_corpus_reembed` pseudo-tool definition.
///
/// Destructively NULLs the existing `l2_experience.embedding` column and
/// re-embeds every fact with the live primary model. Used after a startup
/// consent alert tells the user the primary model has drifted away from the
/// model that produced the persisted corpus (see `embedding_corpus.rs`).
///
/// Requires human approval (ToolPolicy::RequireApproval) — this destroys the
/// existing vectors and takes minutes to recompute.
///
/// `scope` MVP: only `"all"` is implemented. `"since_date"` returns a
/// not-implemented error to preserve the schema for a future incremental pass.
pub(crate) fn build_trigger_corpus_reembed_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "scope": {
                "type": "string",
                "enum": ["all", "since_date"],
                "description": "Reembed scope. 'all' (MVP) recomputes every \
                    L2 vector in the corpus. 'since_date' is reserved for a \
                    future incremental pass and currently returns \
                    not_implemented. Default 'all'."
            }
        },
        "additionalProperties": false
    });

    let function = FunctionObjectArgs::default()
        .name("trigger_corpus_reembed")
        .description(
            "Re-embed ALL persisted L2 memory vectors with the live primary \
             embedding model. Use when the user has approved the \
             model-change consent alert (or asks you to 'reembed all memory' \
             / '重新计算所有 embedding' / 'rebuild vectors'). The call will \
             (1) set every `l2_experience.embedding` and `l2_ephemeral.embedding` \
             to NULL, (2) re-embed each fact sequentially via the current \
             primary model (~10 req/sec), (3) persist the new vectors, (4) \
             request an HNSW rebuild, (5) update the `embedding_corpus_model` \
             marker. Response fields: status, rows_cleared, rows_reembedded, \
             rows_failed, walltime_sec, model_used. Destructive — requires \
             human approval. Only one reembed may run at a time (second call \
             returns status='already_in_progress').",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `unset_embedding_primary_promotion` pseudo-tool definition.
///
/// Clears the three `embedding_primary_promoted_*` user_settings rows AND
/// hot-reloads the in-process `EmbeddingClient` so the env-configured primary
/// takes effect immediately — no service restart required.
///
/// Surprise case: revert restores the values captured AT STARTUP, not whatever
/// is currently in `.env`. If the operator edited `.env` between startup and
/// revert, those edits still require a process restart to take effect.
///
/// Requires human approval in all caller contexts (registered as
/// `ToolPolicy::RequireApproval` in `tool_gate.rs::default_policies`).
pub(crate) fn build_unset_embedding_primary_promotion_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    });

    let function = FunctionObjectArgs::default()
        .name("unset_embedding_primary_promotion")
        .description(
            "Revert a persisted 24h permanent embedding-primary promotion and \
             restore the env-configured primary IN-PROCESS (hot-reload, no \
             service restart). Use ONLY when the user explicitly asks to \
             revert a prior promotion. Deletes the \
             `embedding_primary_promoted_{from,to,at}` settings and atomically \
             swaps the live EmbeddingClient back to the env-original \
             `EMBEDDING_MODEL` / `EMBEDDING_BACKUP_MODEL` captured at process \
             start. If operator edited `.env` AFTER startup, those edits still \
             require a restart (revert restores the startup snapshot, not the \
             current file). \
             ⚠️ WARNING: this restores the boot-time env snapshot, NOT the \
             currently-approved settings primary. If the user approved a \
             settings-based primary change after boot (via `update_setting` \
             with key `embedding_primary_model`, or a `switch_model`-style \
             HITL flow), this tool will UNDO that change too — the live \
             primary reverts all the way back to the env value captured at \
             process start. To set the live primary to a specific model \
             WITHOUT blowing away a settings-based change, use `update_setting` \
             / `switch_model`, not this tool. \
             Response fields: status, message, was_promoted, \
             reverted_from, reverted_to, restart_required (always false), hint.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `notify_user` pseudo-tool for opt-in callback notifications.
///
/// In callback mode, text replies are never sent to the user. The LLM must
/// call this tool to explicitly deliver a notification message.
pub(crate) fn build_notify_user_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "message": {
                "type": "string",
                "description": "The message to send to the user. Keep it concise and actionable."
            },
            "pin": {
                "type": "boolean",
                "description": "Whether to pin the message for visibility. Default false. Use for critical alerts only."
            },
            "priority": {
                "type": "string",
                "enum": ["urgent", "important", "routine"],
                "description": "Notification priority. urgent = always deliver immediately (security, errors). important = deliver with 24h dedup (default). routine = include in daily summary only, not sent immediately."
            },
            "sensitivity": {
                "type": "string",
                "enum": ["s0_public", "s1_low_personal", "s2_personal", "s3_sensitive", "s4_highly_sensitive"],
                "description": "Optional privacy label for the notification content. Use s3_sensitive or s4_highly_sensitive when the message is based on email bodies, legal/financial/medical content, private images, credentials, or other sensitive personal details; those are automatically redacted before delivery."
            },
            "safe_summary": {
                "type": "string",
                "description": "Optional non-sensitive fallback summary safe for Telegram/group previews. Required in spirit when sensitivity is s3_sensitive or s4_highly_sensitive; never include private names, body excerpts, account numbers, legal/medical details, or secrets."
            },
            "source_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Tool names whose results this notification is based on (e.g. [\"server_monitor\", \"web_search\"]). Improves delivery reliability — notifications with declared sources are prioritized."
            }
        },
        "required": ["message"]
    });

    let function = FunctionObjectArgs::default()
        .name("notify_user")
        .description(
            "Send a notification message to the user via their active channel (e.g. Telegram). \
             Use this ONLY when you have actionable information to report. In callback/heartbeat \
             contexts, your text reply is NOT sent to the user — only messages sent through this \
             tool reach the user. Set priority: 'urgent' for critical alerts, 'important' (default) \
             for actionable updates, 'routine' for informational items (daily summary only).",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `get_current_time` pseudo-tool definition for LLM function calling.
/// Returns real-time clock in the user's configured timezone.
pub(crate) fn build_get_current_time_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {},
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("get_current_time")
        .description(
            "Get the current date and time in the user's configured timezone. \
             Call this whenever you need to know or report the current time. \
             NEVER guess the time — always call this tool.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `generate_personal_brief` pseudo-tool for manual brief/review generation.
pub(crate) fn build_generate_personal_brief_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["daily_brief", "evening_review"],
                "description": "Which deterministic report to generate. Defaults to daily_brief."
            },
            "input": {
                "type": "object",
                "description": "Optional full deterministic DailyBriefInput or EveningReviewInput JSON. If omitted, the tool returns an honest skeleton with calendar marked not_queried and includes recent Action Ledger summaries, pending approvals, and active tasks when available."
            },
            "polish": {
                "type": "boolean",
                "description": "Optional foreground-only LLM polish. Defaults to false. If provider policy or paid-API gates do not allow it, the deterministic markdown is returned unchanged."
            }
        },
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("generate_personal_brief")
        .description(
            "Generate a Daily Brief or Evening Review for a foreground user request. \
             By default this tool formats deterministic structured input and does not call an LLM, \
             scheduler, Telegram delivery, or paid API. With polish=true it may optionally ask the \
             routed foreground LLM provider to polish the deterministic Markdown only when provider \
             policy allows; paid API usage remains gated. If input is omitted, it produces an \
             honest skeleton instead of inventing calendar or task data, and includes \
             recent Action Ledger summaries, pending approvals, and active tasks when available.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `recall_memory` pseudo-tool definition for LLM function calling.
///
/// Allows the LLM to actively query its own memory across all layers (L1/L2/L3)
/// with pagination support for large result sets.
pub(crate) fn build_recall_memory_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Optional search term. If empty/omitted, returns all L1+L2 entries. \
                               If provided, searches across L1 (substring), L2 (substring), and L3 (full-text)."
            },
            "layer": {
                "type": "string",
                "enum": ["all", "l1", "l2", "l3"],
                "description": "Memory layer to query. Default: 'all'. \
                               L3 full-text search requires a query."
            },
            "offset": {
                "type": "integer",
                "description": "Pagination offset (default 0). Use to retrieve more results after truncation."
            },
            "limit": {
                "type": "integer",
                "description": "Max entries per page (default 20, max 50)."
            }
        },
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("recall_memory")
        .description(
            "Query your own memory across all layers. Use this when you need to recall \
             what you know about the user, review past conversations, or search for \
             specific information in your memory. Without a query, lists all stored \
             facts and recent messages. With a query, searches across all memory layers. \
             Supports pagination via offset/limit for large result sets.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `view_logs` pseudo-tool definition for viewing Aegis Runtime system logs.
///
/// Allows the LLM to read journalctl logs for the aegis-runtime service on demand,
/// enabling log-based debugging without manual SSH access.
pub(crate) fn build_view_logs_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "lines": {
                "type": "integer",
                "description": "Number of log lines to retrieve (default 50, max 200)."
            },
            "since": {
                "type": "string",
                "description": "Only show logs since this time, e.g. '5 minutes ago', '1 hour ago', '2024-01-15 10:00:00'."
            },
            "filter": {
                "type": "string",
                "description": "Case-insensitive text filter applied to log lines. Only lines containing this substring are returned."
            },
            "priority": {
                "type": "string",
                "enum": ["error", "warning", "info", "all"],
                "description": "Log priority filter. 'error' = errors only, 'warning' = warnings+errors, 'info' = info+warnings+errors, 'all' = everything (default)."
            }
        },
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("view_logs")
        .description(
            "View Aegis Runtime system logs from journalctl. Use this to debug issues, \
             check recent activity, or investigate errors. Supports filtering by \
             time range, priority level, and text search.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `set_log_level` pseudo-tool definition for runtime log control.
pub(crate) fn build_set_log_level_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["get", "set"],
                "description": "Action: 'get' returns current level, 'set' changes it."
            },
            "level": {
                "type": "string",
                "description": "Log filter directive (RUST_LOG syntax). Simple: 'debug', 'warn'. Module-specific: 'info,aegis_runtime_core::orchestrator=debug'. Required for 'set' action."
            }
        },
        "required": ["action"]
    });

    let function = FunctionObjectArgs::default()
        .name("set_log_level")
        .description(
            "Get or change the runtime log level without restarting. \
             Use 'get' to check current level, 'set' to change it. \
             Accepts RUST_LOG syntax for fine-grained module filtering. \
             Requires user approval before changing.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `set_bot_commands` pseudo-tool for modifying the Telegram command menu.
///
/// The menu has two layers:
/// - **Core commands** (s, sk, x, h) — always present, cannot be removed.
/// - **Custom commands** — managed by LLM (with HITL approval) or auto-registered by skills.
pub(crate) fn build_set_bot_commands_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["get", "add", "remove", "set", "reset"],
                "description": "Action to perform:\n\
                    - 'get': view current full menu (core + custom), no approval needed\n\
                    - 'add': add or update custom commands (HITL approval required)\n\
                    - 'remove': remove custom commands by name (HITL approval required, cannot remove core commands)\n\
                    - 'set': replace ALL custom commands (HITL approval required, core commands preserved)\n\
                    - 'reset': remove all user-added custom commands, keep core + skill-auto-registered (HITL approval required)"
            },
            "commands": {
                "type": "array",
                "description": "Array of commands for 'add' or 'set' actions. Each item has 'command' and 'description'.",
                "items": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Command name without leading slash (lowercase, digits, underscore only, max 32 chars). Cannot be a core command name (s/sk/d/x/h)."
                        },
                        "description": {
                            "type": "string",
                            "description": "Human-readable description shown in Telegram menu (max 256 chars)"
                        }
                    },
                    "required": ["command", "description"]
                }
            },
            "command_names": {
                "type": "array",
                "description": "Array of command names to remove (for 'remove' action only). Cannot include core commands.",
                "items": {
                    "type": "string"
                }
            }
        },
        "required": ["action"]
    });

    let function = FunctionObjectArgs::default()
        .name("set_bot_commands")
        .description(
            "Manage the Telegram bot command menu. The menu has core commands (s, sk, x, h) \
             that are always present, plus custom commands that can be added/removed. \
             Use 'get' to view current menu. Use 'add'/'remove'/'set'/'reset' to propose changes (requires user approval). \
             Skills with menu_command in their manifest auto-register commands on activation. \
             Max 100 commands; each: 1-32 lowercase chars, description ≤256 chars.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `deactivate_skill` pseudo-tool definition.
pub(crate) fn build_deactivate_skill_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Name of the skill to deactivate"
            }
        },
        "required": ["skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("deactivate_skill")
        .description(
            "Deactivate a Wasm skill. The skill is unloaded from the sandbox and stays \
             disabled across restarts; its compiled artifact is preserved so it can be \
             reactivated later. Use this if a skill is broken or no longer needed.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `reactivate_skill` pseudo-tool definition.
pub(crate) fn build_reactivate_skill_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Name of the deactivated skill to reactivate"
            }
        },
        "required": ["skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("reactivate_skill")
        .description(
            "Reactivate a previously deactivated Wasm skill. The skill is reloaded from \
             its compiled Wasm into the sandbox and becomes available again. \
             Only works for skills in 'deactivated' status.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `manage_skill` pseudo-tool for health checks and dependency lookups.
pub(crate) fn build_manage_skill_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["health", "affected_intents"],
                "description": "Action to perform on the skill"
            },
            "skill_name": {
                "type": "string",
                "description": "Name of the skill to manage"
            }
        },
        "required": ["action", "skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("manage_skill")
        .description(
            "Inspect a Wasm skill. Actions: 'health' shows execution success/error rates, \
             latency, circuit-breaker state and recent error categories; \
             'affected_intents' lists recurring intents that reference this skill.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the unified `manage_tasks` pseudo-tool.
pub(crate) fn build_manage_tasks_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["add", "update", "complete", "reopen", "delete", "list", "schedule"],
                "description": "add=create task; update=modify fields; complete=mark done; reopen=undo a complete/archive (back to active, e.g. the owner tapped Done by mistake); delete=archive; list=query; schedule=add/modify trigger"
            },
            "task_id": {
                "type": "integer",
                "description": "Required for update/complete/reopen/delete. update does not change status."
            },
            "clear_fields": {
                "type": "array",
                "items": { "type": "string" },
                "description": "For update: fields to CLEAR (set to empty): due_date, trigger_at_unix, trigger_type, event_key, context_text, interval_seconds, group_key, max_fires. Use this for \"把截止日期去掉\" instead of sending null."
            },
            "title": {
                "type": "string",
                "description": "Task title (required for add)"
            },
            "body": {
                "type": "string",
                "description": "Task details"
            },
            "priority": {
                "type": "string",
                "enum": ["urgent", "high", "normal", "low"]
            },
            "due_date": {
                "type": "string",
                "description": "ISO date (YYYY-MM-DD)"
            },
            "tags": {
                "type": "array",
                "items": {"type": "string"}
            },
            "trigger_at": {
                "type": "string",
                "description": "When to trigger: ISO datetime, date+time, or time-only (with timezone)"
            },
            "timezone": {
                "type": "string",
                "description": "IANA timezone for trigger_at interpretation"
            },
            "trigger_type": {
                "type": "string",
                "enum": ["time", "event"],
                "description": "time=delay/clock; event=system event"
            },
            "event_key": {
                "type": "string",
                "description": "Event key for event triggers"
            },
            "execution_mode": {
                "type": "string",
                "enum": ["notify", "llm_callback"],
                "description": "notify=send text (free); llm_callback=full LLM turn with tools"
            },
            "context_text": {
                "type": "string",
                "description": "Self-contained callback prompt or notify text. Callback has no conversation access."
            },
            "source_summary": {
                "type": "string",
                "description": "Optional evidence summary. Do not assert conclusions from truncated/preview sources."
            },
            "interval_seconds": {
                "type": "integer",
                "description": "Recurring interval in seconds (min 30s). Requires group_key."
            },
            "group_key": {
                "type": "string",
                "description": "Semantic dedup key for recurring tasks; upserts the existing key."
            },
            "max_fires": {
                "type": "integer",
                "description": "Max fire count for recurring tasks"
            },
            "recur_weekdays": {
                "type": "array",
                "items": {"type": "integer", "minimum": 1, "maximum": 7},
                "description": "ISO weekdays 1=Mon..7=Sun for windowed recurrence. Requires interval_seconds."
            },
            "recur_window_start": {
                "type": "string",
                "description": "Local 'HH:MM' window start. Requires recur_window_end + interval_seconds."
            },
            "recur_window_end": {
                "type": "string",
                "description": "Local 'HH:MM' window end. Requires recur_window_start + interval_seconds."
            },
            "recur_timezone": {
                "type": "string",
                "description": "IANA timezone for the window/weekdays. Invalid names are rejected; default UTC."
            },
            "pin": {
                "type": "boolean",
                "description": "Pin notification message in chat"
            },
            "delay_seconds": {
                "type": "integer",
                "description": "Relative delay for scheduling (alternative to trigger_at)"
            },
            "status": {
                "type": "string",
                "description": "For list: filter by status — active (default), done, archived, fired, cancelled, or all (no filter). complete/reopen/delete act on task_id directly; do not list first when the owner named the id."
            },
            "limit": {
                "type": "integer",
                "description": "For list: max results (default 30, max 50)"
            }
        },
        "required": ["action"]
    });

    let function = FunctionObjectArgs::default()
        .name("manage_tasks")
        .description(
            "Unified task manager: add, update, complete, reopen, delete, list, schedule. \
             Recurring needs interval_seconds + group_key. Windowed recurring is ONE \
             task with recur_weekdays + window + timezone — never one task per slot. \
             context_text must be self-contained. Prefer this over schedule_intent.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `manage_goals` pseudo-tool for long-term goal tracking (G5).
pub(crate) fn build_manage_goals_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["create", "update", "list", "complete", "pause", "resume", "abandon", "add_milestone", "complete_milestone", "get"],
                "description": "Goal management action"
            },
            "goal_id": {
                "type": "integer",
                "description": "Goal ID (required for update/complete/pause/resume/abandon/add_milestone/complete_milestone/get)"
            },
            "title": {
                "type": "string",
                "description": "Goal title (required for create)"
            },
            "description": {
                "type": "string",
                "description": "Goal description with context"
            },
            "source_summary": {
                "type": "string",
                "description": "Optional evidence summary. Unconfirmed preview/truncated/no-text sources require the full source or user confirmation before asserting progress or decisions."
            },
            "progress_pct": {
                "type": "number",
                "description": "Progress as 0.0-1.0 (for update)"
            },
            "target_date": {
                "type": "string",
                "description": "Target completion date (ISO 8601)"
            },
            "parent_goal_id": {
                "type": "integer",
                "description": "Parent goal ID for sub-goals"
            },
            "milestone_title": {
                "type": "string",
                "description": "Milestone title (for add_milestone)"
            },
            "milestone_index": {
                "type": "integer",
                "description": "0-based milestone index (for complete_milestone)"
            },
            "status_filter": {
                "type": "string",
                "enum": ["active", "paused", "completed", "abandoned", "all"],
                "description": "Status filter for list (default: active; 'all' shows every status)"
            },
            "source": {
                "type": "string",
                "enum": ["explicit", "inferred"],
                "description": "Goal source: explicit (user said) or inferred (detected from conversation). Default: explicit"
            }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("manage_goals")
        .description(
            "Manage user's long-term goals with hierarchical decomposition and progress tracking.\n\n\
             ACTIONS:\n\
             - create: New goal (requires title). Use parent_goal_id for sub-goals.\n\
             - update: Change title/description/progress_pct/target_date\n\
             - list: Show goals (default: active only; use status_filter for others)\n\
             - complete/pause/resume/abandon: Change goal status\n\
             - add_milestone: Add a milestone checkpoint to a goal\n\
             - complete_milestone: Mark a milestone as done (auto-updates progress)\n\
             - get: Get full details of a single goal\n\n\
             When user discusses plans, ambitions, or projects — create goals proactively.\n\
             When milestones are done or progress is made — update proactively.\n\
             Never create/update goals or progress from snippets/previews/truncated/no-text sources unless the full source or user confirmation is available.\n\
             Source 'inferred' for goals detected from conversation; 'explicit' when user says directly."
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `knowledge_base` pseudo-tool for CEO Nerve Center.
pub(crate) fn build_knowledge_base_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["extract", "query", "search", "update", "dashboard", "keep_entry", "archive_entry", "restore_entry", "preview_merge_entries", "merge_entries", "preview_delete_entry", "delete_entry", "dismiss_lint_finding", "resolve_lint_finding"],
                "description": "extract=store; query/search/update/dashboard=lookup; keep/archive/restore=review; preview_* then merge/delete (preview_token+confirm); lint dismiss/resolve"
            },
            "entries": {
                "type": "array",
                "description": "For extract: array of entries to store",
                "items": {
                    "type": "object",
                    "properties": {
                        "category": {
                            "type": "string",
                            "enum": ["decision", "insight", "waiting_for", "metric", "reference"],
                            "description": "waiting_for/decision/insight/metric/reference. Tasks use manage_tasks."
                        },
                        "priority": { "type": "string", "enum": ["urgent", "high", "normal", "low"] },
                        "title": { "type": "string", "description": "One-line title (concise)" },
                        "body": { "type": "string", "description": "Structured details" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for cross-referencing" },
                        "due_date": { "type": "string", "description": "ISO date for action_item deadlines" },
                        "waiting_on": { "type": "string", "description": "Who/what we're waiting for (waiting_for category)" },
                        "source_summary": { "type": "string", "description": "Brief summary of the conversation context this was extracted from" }
                    },
                    "required": ["category", "title", "body"]
                }
            },
            "category": { "type": "string", "description": "For query: filter by category" },
            "status": { "type": "string", "description": "For query/update: filter by or set status (active/done/archived/superseded)" },
            "priority": { "type": "string", "description": "For query/update: filter by or set priority" },
            "query": { "type": "string", "description": "For search: text to search in titles, bodies, tags" },
            "entry_id": { "type": "integer", "description": "Entry ID for update/keep/archive/restore/preview_delete/delete" },
            "source_entry_ids": { "type": "array", "items": { "type": "integer" }, "description": "Source KB IDs archived/superseded by merge" },
            "target_entry_id": { "type": "integer", "description": "Surviving KB entry ID for preview_merge/merge" },
            "merged_title": { "type": "string", "description": "For merge_entries: optional replacement title for the surviving target entry" },
            "merged_body": { "type": "string", "description": "For merge_entries: optional replacement body for the surviving target entry" },
            "reason": { "type": "string", "description": "For merge/delete review: user-visible reason for the cleanup action" },
            "preview_token": { "type": "string", "description": "Required for merge/delete. Copy from the matching preview_* result after owner confirmation." },
            "confirm": { "type": "boolean", "description": "Must be true for merge/delete, only after the owner confirmed the preview_token." },
            "lint_finding_id": { "type": "integer", "description": "For dismiss_lint_finding/resolve_lint_finding: ID from kb_hygiene.pending_lint_findings.recent" },
            "title": { "type": "string", "description": "For update: new title" },
            "body": { "type": "string", "description": "For update: new body" },
            "due_date": { "type": "string", "description": "For update: new due date (null to clear)" },
            "waiting_on": { "type": "string", "description": "For update: new waiting_on value" },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "For update: new tags" },
            "limit": { "type": "integer", "description": "Max results to return (default 20, max 50)" }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("knowledge_base")
        .description(
            "CEO knowledge base. extract after conversations with facts/decisions. \
             query/search/update/dashboard for lookup. keep/archive/restore for review. \
             merge/delete require preview_* then preview_token + confirm=true. \
             Tasks go to manage_tasks, not a KB category.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `check_api_usage` pseudo-tool for querying OpenRouter account credits and usage.
pub(crate) fn build_check_api_usage_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["balance", "activity", "summary", "local", "reconcile", "update_pricing", "backfill"],
                "description": "balance = check remaining credits; activity = usage breakdown by model/day (last 30 days); summary = balance + today's activity combined; local = query local cost tracking by label/model; reconcile = compare local vs OpenRouter costs; update_pricing = refresh pricing cache; backfill = fill NULL cost_usd rows"
            },
            "date": {
                "type": "string",
                "description": "Optional: filter activity by a single UTC date (YYYY-MM-DD). Only used with action='activity'."
            },
            "days": {
                "type": "integer",
                "description": "Optional: number of days to query for action='local' (default 7)."
            }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("check_api_usage")
        .description(
            "Query the OpenRouter API account: check remaining credits/balance, \
             view usage breakdown by model and day (tokens, cost, requests), \
             get a combined summary, or query local cost analytics. \
             Use when user asks about API costs, spending, token usage, \
             account balance, or cost tracking.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `activate_agent_skill` pseudo-tool for loading external Agent Skills.
pub(crate) fn build_activate_agent_skill_tool(
    skills: &[AgentSkill],
) -> Result<ChatCompletionTools> {
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "enum": names,
                "description": "Name of the external Agent Skill to activate"
            }
        },
        "required": ["name"]
    });
    let function = FunctionObjectArgs::default()
        .name("activate_agent_skill")
        .description(
            "Activate an external Agent Skill by loading its full instructions into context. \
             Use this when a task matches one of the discovered skills in <agent_skills>.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `calendar_create_event` pseudo-tool for Google Calendar event creation.
///
/// SAFETY: RequireApproval-gated; self-only by default — supplying any attendee
/// other than the authenticated user's own email requires the setting
/// `calendar_invite_others_enabled=true`. See `docs/calendar-write-capability-plan.md`.
pub(crate) fn build_calendar_create_event_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "summary":     {"type": "string", "description": "Event title (1-200 chars)"},
            "start":       {"type": "string", "description": "RFC 3339 datetime, e.g. 2026-04-20T09:00:00+08:00"},
            "end":         {"type": "string", "description": "RFC 3339 datetime, must be > start"},
            "description": {"type": "string", "description": "Optional body (<=5000 chars)"},
            "location":    {"type": "string"},
            "attendees": {
                "type":  "array",
                "items": {"type": "string", "description": "Attendee email"},
                "description": "Optional. Non-self attendees require calendar_invite_others_enabled=true (default off)."
            },
            "timezone":    {"type": "string", "description": "IANA zone, optional (defaults to calendar primary tz)"},
            "calendar_id": {"type": "string", "description": "Default: primary"},
            "account":     {"type": "string", "description": "Google account id — omit for the default account"}
        },
        "required": ["summary", "start", "end"]
    });

    let function = FunctionObjectArgs::default()
        .name("calendar_create_event")
        .description(
            "Create a Google Calendar event on the authenticated user's calendar. \
             Every invocation surfaces a HITL approval prompt (RequireApproval). \
             SELF-ONLY BY DEFAULT: supplying any attendee other than the user's own \
             email fails with `error_kind: invite_others_disabled` unless the setting \
             `calendar_invite_others_enabled` has been turned on by the user. \
             On that error, relay the request to the user to enable the setting \
             (via update_setting, HITL) rather than silently dropping attendees. \
             Times must be RFC 3339.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `calendar_update_event` pseudo-tool. Partial / PATCH semantics:
/// unspecified fields are preserved.
pub(crate) fn build_calendar_update_event_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "event_id":    {"type": "string"},
            "calendar_id": {"type": "string", "description": "Default: primary"},
            "summary":     {"type": "string", "description": "Optional — omit to keep existing"},
            "start":       {"type": "string", "description": "RFC 3339 datetime"},
            "end":         {"type": "string", "description": "RFC 3339 datetime"},
            "description": {"type": "string"},
            "location":    {"type": "string"},
            "attendees":   {"type": "array", "items": {"type": "string"}, "description": "If supplied, REPLACES the existing attendee list. Subject to the same invite_others_disabled policy as calendar_create_event. Omit to preserve the existing list."},
            "timezone":    {"type": "string"},
            "account":     {"type": "string"}
        },
        "required": ["event_id"]
    });

    let function = FunctionObjectArgs::default()
        .name("calendar_update_event")
        .description(
            "Update an existing Google Calendar event (PATCH semantics — unspecified \
             fields are preserved, same UPDATE discipline as manage_tasks). \
             Every invocation surfaces a HITL approval prompt. Same attendee policy \
             as calendar_create_event: non-self attendees require \
             calendar_invite_others_enabled=true.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `calendar_delete_event` pseudo-tool. Deletion is irreversible —
/// the HITL prompt (via `ToolGate::RequireApproval`) must make that clear.
pub(crate) fn build_calendar_delete_event_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "event_id":    {"type": "string"},
            "calendar_id": {"type": "string", "description": "Default: primary"},
            "account":     {"type": "string"}
        },
        "required": ["event_id"]
    });

    let function = FunctionObjectArgs::default()
        .name("calendar_delete_event")
        .description(
            "Delete a Google Calendar event. IRREVERSIBLE — surface that in your \
             summary to the user so they approve with full awareness. Attendees \
             (if any) receive a cancellation notice automatically. Every \
             invocation surfaces a HITL approval prompt.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `setup_google` pseudo-tool for Telegram-driven Google OAuth setup
/// (Calendar + Drive).
pub(crate) fn build_setup_google_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["status", "setup", "upgrade-scopes", "revoke"],
                "description": "status = check Google Calendar/Drive connection; setup = start OAuth flow for a NEW account (generates auth URL); upgrade-scopes = request additional scopes on an ALREADY-connected account via incremental authorization (preserves existing token); revoke = disconnect Google Calendar/Drive"
            },
            "account_name": {
                "type": "string",
                "description": "User-chosen name for this Google account (e.g. 'personal', 'work'). Required when adding a second account. If omitted, defaults to 'primary'."
            },
            "scopes": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional, only honoured when action=upgrade-scopes. Explicit list of OAuth scope URLs to request on the auth URL instead of the full DEFAULT_SCOPES. Each entry must appear in the server-side allowlist (DEFAULT_SCOPES ∪ READONLY_SCOPES); unknown scopes return status=error, error_kind=scope_not_allowed. Omit to request DEFAULT_SCOPES (most common). Because include_granted_scopes=true is set, the token's effective scope set is always the UNION of previously granted scopes and the requested subset."
            }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("setup_google")
        .description(
            "Set up or manage Google Calendar and Drive access via Google OAuth2. \
             Use 'status' to check if Google Calendar/Drive is connected. \
             Use 'setup' to start the OAuth authorization flow — this generates an auth URL \
             that the user must click to authorize. If no client_id/client_secret are stored, \
             tell the user to paste them in a SEPARATE message using this exact format:\n\
             client_id: xxx\nclient_secret: yyy\n\
             The system will auto-detect and securely store them (the message is deleted immediately). \
             NEVER include client_id or client_secret in your tool call or message text — \
             they must be pasted by the user directly so they are intercepted before reaching the AI. \
             Use 'upgrade-scopes' when the user is ALREADY connected but needs additional \
             Google permissions (e.g. they set up read-only earlier and now want calendar \
             write). Unlike 'setup' it does NOT require them to re-paste credentials. \
             Because the URL sets prompt=consent (required to obtain a fresh refresh_token), \
             Google WILL display the full consent screen for every scope in the request; \
             however, include_granted_scopes=true ensures the issued token carries the UNION \
             of previously-granted and newly-requested scopes, so existing grants are never \
             lost. Use this when 'setup' returns already_configured but the user is asking \
             for a capability that the current token lacks. \
             Optionally pass `scopes: [...]` (subset of DEFAULT_SCOPES ∪ READONLY_SCOPES) to \
             narrow the consent-screen request to just those scopes — the token still ends \
             up with the union. \
             Use 'revoke' to disconnect Google Calendar/Drive. \
             IMPORTANT: When presenting the auth URL to the user, send it as a raw URL \
             (not markdown link) so they can click it directly in Telegram.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `google_drive_upload` pseudo-tool.
pub(crate) fn build_google_drive_upload_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "filename": {
                "type": "string",
                "description": "Filename to create in Google Drive, e.g. report.md or notes.txt"
            },
            "content_text": {
                "type": "string",
                "description": "UTF-8 text content to upload as the file body"
            },
            "mime_type": {
                "type": "string",
                "description": "Optional MIME type, default text/plain; common values: text/plain, text/markdown, text/csv, application/json"
            },
            "folder_id": {
                "type": "string",
                "description": "Optional Drive folder ID to upload into. Omit for My Drive root."
            },
            "account": {
                "type": "string",
                "description": "Google account ID to use (e.g. primary, work). Omit for the default account."
            }
        },
        "required": ["filename", "content_text"]
    });
    let function = FunctionObjectArgs::default()
        .name("google_drive_upload")
        .description(
            "Upload a NEW text file to Google Drive. Creates/uploads only; \
             this tool does NOT delete files. Use it to save reports, notes, \
             CSV/JSON exports, or other generated text artifacts to Drive. \
             Requires Google OAuth2 with drive.file scope. Prefer explicit \
             filenames and MIME types. If the user asks to overwrite or delete \
             an existing file, explain that deletion is not supported by this tool.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ── Deferred Intent pseudo-tool builders ─────────────────────────────────

/// Build the `schedule_intent` pseudo-tool definition.
pub(crate) fn build_schedule_intent_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "trigger_type": {
                "type": "string",
                "enum": ["time", "event"],
                "description": "Trigger type: 'time' for delay-based, 'event' for system event-based"
            },
            "trigger_at": {
                "type": "string",
                "description": "Absolute time (ISO 8601, date+time, or time-only). Prefer over delay_seconds for clock times."
            },
            "timezone": {
                "type": "string",
                "description": "IANA name or UTC offset used when trigger_at has no offset."
            },
            "delay_seconds": {
                "type": "integer",
                "description": "Relative delay in seconds (10s–365d). Prefer trigger_at for clock times."
            },
            "event_key": {
                "type": "string",
                "description": "Event key to match (required for event triggers, e.g. 'email:new:primary')"
            },
            "execution_mode": {
                "type": "string",
                "enum": ["notify", "llm_callback"],
                "description": "notify=send context_text (no LLM); llm_callback=full turn with tools (default)"
            },
            "context_text": {
                "type": "string",
                "description": "Notify text or self-contained callback prompt. Callback has no conversation access."
            },
            "interval_seconds": {
                "type": "integer",
                "description": "Recurring interval in seconds (min 30s, max 365d). Requires group_key. time triggers only."
            },
            "group_key": {
                "type": "string",
                "description": "Recurring dedup key (alnum/_/-, max 100). Upserts an existing pending key."
            },
            "max_fires": {
                "type": "integer",
                "description": "Stop after N fires. Omit for unlimited."
            },
            "recur_weekdays": {
                "type": "array",
                "items": {"type": "integer", "minimum": 1, "maximum": 7},
                "description": "ISO weekdays 1=Mon..7=Sun for windowed recurrence. Requires interval_seconds."
            },
            "recur_window_start": {
                "type": "string",
                "description": "Local 'HH:MM' window start. Requires recur_window_end + interval_seconds."
            },
            "recur_window_end": {
                "type": "string",
                "description": "Local 'HH:MM' window end. Requires recur_window_start + interval_seconds."
            },
            "recur_timezone": {
                "type": "string",
                "description": "IANA timezone for the window/weekdays. Invalid names are rejected; default UTC."
            },
            "pin": {
                "type": "boolean",
                "description": "Pin the notify message in Telegram after sending."
            }
        },
        "required": ["trigger_type", "context_text"]
    });

    let function = FunctionObjectArgs::default()
        .name("schedule_intent")
        .description(
            "Schedule a future action. One-shot: trigger_type + context_text + (trigger_at or delay_seconds). \
             Recurring: also interval_seconds + group_key (group_key alone is still one-shot). \
             Windowed: add weekdays + window + timezone on ONE intent. Prefer manage_tasks for new work. \
             context_text must be self-contained.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `list_intents` pseudo-tool definition.
pub(crate) fn build_list_intents_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {},
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("list_intents")
        .description("List all active (pending) deferred intents for the current user.")
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `cancel_intent` pseudo-tool definition.
pub(crate) fn build_cancel_intent_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "intent_id": {
                "type": "integer",
                "description": "The ID of the deferred intent to cancel"
            }
        },
        "required": ["intent_id"]
    });

    let function = FunctionObjectArgs::default()
        .name("cancel_intent")
        .description("Cancel a pending deferred intent by its ID.")
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ── Monitoring intent pseudo-tool builders ───────────────────────────────

/// Build the `set_monitoring_intent` pseudo-tool definition.
pub(crate) fn build_set_monitoring_intent_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Name of the skill to set monitoring intent for (e.g. 'server_monitor')"
            },
            "description": {
                "type": "string",
                "description": "Natural language description of what alerts are relevant."
            },
            "threshold": {
                "type": "number",
                "description": "Cosine similarity threshold (0.0-1.0). Higher = stricter filtering. Default: 0.7"
            }
        },
        "required": ["skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("set_monitoring_intent")
        .description(
            "Set monitoring filters for a heartbeat skill. Uses semantic relevance against the \
             description to decide which heartbeat alerts are worth notifying the owner about.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `list_monitoring_intents` pseudo-tool definition.
pub(crate) fn build_list_monitoring_intents_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {}
    });

    let function = FunctionObjectArgs::default()
        .name("list_monitoring_intents")
        .description("List all registered monitoring intents (alert filters) across skills.")
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `remove_monitoring_intent` pseudo-tool definition.
pub(crate) fn build_remove_monitoring_intent_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Name of the skill to remove monitoring intent from"
            }
        },
        "required": ["skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("remove_monitoring_intent")
        .description(
            "Remove monitoring filters for a skill. Any semantic intent for that skill will be cleared.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ── Alert policy pseudo-tool builders ─────────────────────────────────

/// Build the `set_alert_policy` pseudo-tool definition.
pub(crate) fn build_set_alert_policy_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Name of the skill to configure alert policy for"
            },
            "delivery": {
                "type": "string",
                "enum": ["all", "warning_and_above", "critical_only", "silent"],
                "description": "Which severity levels to push. Default: warning_and_above"
            },
            "send_resolved": {
                "type": "boolean",
                "description": "Whether to send RESOLVED notifications when alert clears. Default: true"
            },
            "cooldown_secs": {
                "type": "integer",
                "description": "Cooldown between repeated alerts in seconds. 0 = global default (4h). Default: 0"
            },
            "quiet_hours": {
                "type": "string",
                "description": "UTC quiet hours range like '22-06' (10pm-6am). During quiet hours only Critical alerts pass. Empty string or null to disable. Default: disabled"
            },
            "pin_messages": {
                "type": "boolean",
                "description": "Whether to pin alert messages in Telegram. Default: true"
            },
            "mute_hours": {
                "type": "number",
                "description": "Temporarily mute ALL alerts for this skill for N hours (max 720 = 30 days). \
                    Alerts resume automatically after the mute expires. Pass 0 or null to unmute immediately."
            }
        },
        "required": ["skill_name"]
    });

    let function = FunctionObjectArgs::default()
        .name("set_alert_policy")
        .description(
            "Configure the alert delivery policy for a specific skill's heartbeat alerts. \
             Controls which severity levels are pushed, whether RESOLVED notifications are sent, \
             per-skill cooldown duration, quiet hours, pin behavior, and temporary muting. \
             Only explicitly provided fields are changed; others keep their current values.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `list_alert_policies` pseudo-tool definition.
pub(crate) fn build_list_alert_policies_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {}
    });

    let function = FunctionObjectArgs::default()
        .name("list_alert_policies")
        .description(
            "List all configured alert policies for heartbeat skills, showing delivery mode, \
             cooldown, quiet hours, and resolved notification settings.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ─── MCP management pseudo-tools ─────────────────────────────────────────────

pub(crate) fn build_mcp_list_servers_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {}
    });
    let function = FunctionObjectArgs::default()
        .name("mcp_list_servers")
        .description(
            "List all configured MCP (Model Context Protocol) servers and their connection status, \
             including available tools from each server.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_mcp_reconnect_server_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "server": {
                "type": "string",
                "description": "Name/key of the MCP server to reconnect"
            }
        },
        "required": ["server"]
    });
    let function = FunctionObjectArgs::default()
        .name("mcp_reconnect_server")
        .description(
            "Reconnect to a specific MCP server. Use when a server appears disconnected \
             or is returning errors.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_configure_autonomous_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["enable", "disable", "configure", "status"],
                "description": "Action to perform on autonomous heartbeat mode"
            },
            "skill_name": {
                "type": "string",
                "description": "Skill name for per-skill config. Omit for global config."
            },
            "max_rounds": {
                "type": "integer",
                "description": "Maximum LLM reasoning rounds per heartbeat cycle (1-10)"
            },
            "max_tool_calls": {
                "type": "integer",
                "description": "Maximum tool calls per heartbeat cycle (1-20)"
            },
            "allowed_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Allowlist of tools the LLM may call during autonomous mode"
            },
            "denied_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Denylist of tools to block even if in allowlist"
            },
            "daily_token_budget": {
                "type": "integer",
                "description": "Maximum tokens per day for autonomous actions (default: 50000)"
            },
            "notification_mode": {
                "type": "string",
                "enum": ["Always", "OnAction", "Never"],
                "description": "When to notify user about autonomous actions"
            }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("configure_autonomous")
        .description(
            "Configure autonomous heartbeat mode. Enable/disable/configure the LLM's ability \
             to take autonomous actions during heartbeat cycles within approved safety bounds.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `update_heartbeat_memo` pseudo-tool for heartbeat scratchpad persistence.
///
/// Allows the heartbeat LLM to maintain a persistent memo across cycles within a day.
/// The memo is stored in user_settings and injected into the system prompt each cycle.
/// Heartbeat-only pseudo-tool: signal that no action is warranted this tick.
///
/// Step 8a (design §4.3). The LLM calls `noop` to explicitly end the tick
/// with no side effects. The heartbeat ReAct loop treats this as terminal:
/// it records the tool result, then stops without another LLM round.
pub(crate) fn build_noop_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "reason": {
                "type": "string",
                "description": "Optional short reason for doing nothing (e.g. 'no new signals')."
            }
        }
    });
    let function = FunctionObjectArgs::default()
        .name("noop")
        .description(
            "Declare explicitly that no action is warranted this heartbeat tick. \
             Preferred over an empty reply when nothing needs doing — keeps the \
             audit log clean and signals 'I evaluated and chose inaction'.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Heartbeat-only pseudo-tool: persist a long-lived learned fact into L2 under
/// the `heartbeat:learned:*` topic scope (design §3.1, §4.3).
pub(crate) fn build_write_memory_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "topic": {
                "type": "string",
                "description": "Short topic slug (e.g. 'user_prefers_cn'). Rewritten to 'heartbeat:learned:<topic>' at write time."
            },
            "fact": {
                "type": "string",
                "description": "The fact content. Plain text; concise."
            },
            "confidence": {
                "type": "number",
                "description": "Optional confidence score in [0.0, 1.0]."
            }
        },
        "required": ["topic", "fact"]
    });
    let function = FunctionObjectArgs::default()
        .name("write_memory")
        .description(
            "Persist a learned fact into long-term memory (L2) under the \
             heartbeat:learned:* scope. Use for durable observations about the \
             user, system, or environment that should outlast the current tick.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Heartbeat-only pseudo-tool: write/replace the `heartbeat_memo` note that
/// the next tick will read. Thin wrapper over `write_heartbeat_memo` (Step 6)
/// surfacing inline-vs-spill storage to the LLM.
pub(crate) fn build_write_next_tick_note_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "Full replacement content for the next-tick note (plain text)."
            }
        },
        "required": ["content"]
    });
    let function = FunctionObjectArgs::default()
        .name("write_next_tick_note")
        .description(
            "Replace the persistent next-tick note (heartbeat_memo) with the \
             given content. Overflow past the configured cap is handled \
             transparently by spilling the full content to L2 under \
             heartbeat:plan:<slug> and keeping a pointer + synopsis in the memo.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Heartbeat-only pseudo-tool (Step 9b): propose a change to `heartbeat_config`
/// for user approval. Creates a HITL row with `kind="config_change"`
/// (design §7).
pub(crate) fn build_propose_config_change_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "changes": {
                "type": "object",
                "description": "Partial HeartbeatConfig overlay. Merged onto current config; merged result must pass HeartbeatConfig validation."
            },
            "reason": {
                "type": "string",
                "description": "Why propose this change — shown to the user."
            }
        },
        "required": ["changes", "reason"]
    });
    let function = FunctionObjectArgs::default()
        .name("propose_config_change")
        .description(
            "Propose a change to heartbeat_config for user approval. The merged \
             config is validated before the proposal is queued; on approve, the \
             change is applied via Orchestrator::update_heartbeat_config with \
             actor=llm. Heartbeat-only.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Heartbeat-only pseudo-tool (Step 10): send the one permitted proactive
/// message per tick (design §5). Three safety gates enforced server-side:
/// rate limit (`heartbeat_config.per_tick.send_message`), source citation
/// (cited L2 row must exist, belong to same did, be ≤30d old), and
/// embedding-based dedup vs the last 24h of sent messages to the same chat.
pub(crate) fn build_send_message_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "body": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4000,
                "description": "Message content to deliver to the user. Plain text / markdown; rendered via the same outbound pipeline as user-turn replies."
            },
            "source_id": {
                "type": "integer",
                "description": "L2 row id the message is grounded in. Must resolve AND be ≤30 days old AND belong to the same did — server-validated."
            },
            "kind": {
                "type": "string",
                "enum": ["text"],
                "description": "Output modality. Only 'text' is implemented; reserved for future voice/image."
            }
        },
        "required": ["body", "source_id"]
    });
    let function = FunctionObjectArgs::default()
        .name("send_message")
        .description(
            "Send the single proactive message allowed per heartbeat tick. \
             Must cite a fresh (≤30d) L2 row via `source_id`. Rejected on \
             rate-limit, missing/stale/cross-tenant source, or semantic \
             duplicate of a recent send. Up to 2 rejections may be retried \
             within a tick; the 3rd is non-retryable. Heartbeat-only.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_update_heartbeat_memo_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "The complete memo content. Replaces the previous memo entirely."
            }
        },
        "required": ["content"]
    });

    let function = FunctionObjectArgs::default()
        .name("update_heartbeat_memo")
        .description(
            "Update your persistent heartbeat scratchpad/memo. This memo persists across \
             heartbeat cycles within a day. Use it to track what you've done, what to check \
             next, and observations. Write the COMPLETE memo content (not a diff). At quiet \
             hours, this memo is archived and reset.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_update_profile_tool() -> Result<ChatCompletionTools> {
    // Enumerate canonical slots from the registry so the LLM can only choose a
    // valid (category, key) pair. Built at runtime once per tool-list
    // construction — cost is negligible (17 slots).
    let slots_hint: Vec<String> = crate::profile_registry::all_slots()
        .iter()
        .map(|s| format!("{}.{} — {}", s.category, s.key, s.description))
        .collect();
    let slots_catalog = slots_hint.join("\n");

    // Build oneOf branches so the provider enforces a valid slot choice.
    let one_of: Vec<serde_json::Value> = crate::profile_registry::all_slots()
        .iter()
        .map(|s| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "category": { "const": s.category },
                    "key": { "const": s.key },
                    "value": { "type": "string", "maxLength": s.max_len.min(256) }
                },
                "required": ["category", "key", "value"],
                "additionalProperties": false
            })
        })
        .collect();

    let schema = serde_json::json!({
        "type": "object",
        "oneOf": one_of
    });

    let function = FunctionObjectArgs::default()
        .name("update_profile")
        .description(format!(
            "Update the user's stable Profile (Tier 0) when the user explicitly \
             shares information that belongs in one of the canonical slots listed \
             below. Use this INSTEAD of just remembering for long-term facts. \
             The (category, \
             key) pair MUST be one of the canonical slots. Do NOT invent keys. \
             If the value doesn't match the slot's semantic role (e.g. a city \
             in home_address), do NOT call this tool — the registry validator \
             will reject the write. source is always 'auto' (the system will \
             treat LLM-initiated writes as assistant-captured, not user-manual).\n\n\
             Canonical slots:\n{}",
            slots_catalog
        ))
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

pub(crate) fn build_view_heartbeat_audit_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": {
                "type": "string",
                "description": "Filter by skill name. Omit for all skills."
            },
            "limit": {
                "type": "integer",
                "description": "Maximum entries to return (default: 20, max: 100)"
            }
        }
    });
    let function = FunctionObjectArgs::default()
        .name("view_heartbeat_audit")
        .description(
            "View the audit log of autonomous heartbeat actions. Shows what the LLM did \
             during each autonomous cycle including tools called, actions taken, and tokens used.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ── Companion Growth Tools ────────────────────────────────────────────────

/// Build the `view_growth` pseudo-tool for viewing companion growth status.
#[allow(dead_code)] // Handler not yet wired — will be enabled when execute path is ready
pub(crate) fn build_view_growth_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "detail_level": {
                "type": "string",
                "enum": ["summary", "full"],
                "description": "summary: overview with dimension depths and stage. \
                 full: includes personality weights, recent growth events, velocity trends."
            }
        },
        "required": []
    });

    let function = FunctionObjectArgs::default()
        .name("view_growth")
        .description(
            "View the companion's growth status, personality profile, and recent growth events. \
             Shows dimensions, stage, milestones.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `adjust_personality` pseudo-tool for tuning personality traits.
#[allow(dead_code)] // Handler not yet wired — will be enabled when execute path is ready
pub(crate) fn build_adjust_personality_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "trait_name": {
                "type": "string",
                "enum": ["warmth", "formality", "humor", "directness", "verbosity"],
                "description": "The personality trait to adjust."
            },
            "value": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 1.0,
                "description": "New value for the trait (0.0 to 1.0)."
            },
            "reason": {
                "type": "string",
                "description": "Brief reason for the adjustment."
            }
        },
        "required": ["trait_name", "value", "reason"]
    });

    let function = FunctionObjectArgs::default()
        .name("adjust_personality")
        .description(
            "Adjust a personality trait of the companion. Values range from 0.0 to 1.0. \
             Use when the user wants the companion to be more/less warm, formal, humorous, \
             direct, or verbose.",
        )
        .parameters(schema)
        .build()?;

    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `delegate_task` pseudo-tool — hand a coding task to Hercules.
///
/// Aegis Runtime only drafts/creates/reads tasks; Hercules runs and verifies them
/// (`docs/2026-09-05-hercules-delegation-design.md`). `draft` and `status`
/// are read-only (auto-allowed); `create` is gated because it spends
/// executor time, and on production Hercules dispatches paid runs.
pub(crate) fn build_delegate_task_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["draft", "create", "status"],
                "description": "draft: turn an intent into a task draft + acceptance lint (read-only). create: create the Hercules task from a reviewed draft. status: read a delegated task (or list all known delegations when task_id is omitted)."
            },
            "intent": {
                "type": "string",
                "description": "Plain-language description of the change wanted (for draft)"
            },
            "repo": {
                "type": "string",
                "description": "Hercules repo key, e.g. \"aegis\" (for draft/create; default aegis)"
            },
            "title": {
                "type": "string",
                "description": "Task title (for create; take it from the draft)"
            },
            "body": {
                "type": "string",
                "description": "Task body in Hercules format — must contain a '## Definition of Done' section (for create; take it from the draft, edit if needed)"
            },
            "priority": {
                "type": "integer",
                "description": "Priority 0-100 (for create; default from draft)"
            },
            "labels": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Extra labels (for create). Linkage labels are added automatically."
            },
            "work_item_id": {
                "type": "integer",
                "description": "Aegis Runtime work item this delegation belongs to, if any (for create)"
            },
            "adapter": {
                "type": "string",
                "enum": ["grok", "codex", "claude", "cursor"],
                "description": "Coding runtime Hercules should use (for create; default grok, the owner's subscription runtime)"
            },
            "attempt": {
                "type": "integer",
                "description": "Re-delegation counter (for create; default 1). The task id is derived from title/body, so re-delegating the same spec after a cancel needs attempt=2, 3, …"
            },
            "task_id": {
                "type": "string",
                "description": "Hercules task id (for status)"
            }
        },
        "required": ["action"]
    });
    let function = FunctionObjectArgs::default()
        .name("delegate_task")
        .description(
            "Delegate a coding task to Hercules. Workflow: draft (read-only) → show the owner → \
             create → status. Never claim done until status reports it. Default adapter is grok. \
             Re-delegate the same spec with attempt=2,3,…",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `run_external` pseudo-tool — hand a self-contained task to grok
/// build (primary) or codex (fallback) and return a receipt.
pub(crate) fn build_run_external_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "The full self-contained instruction for the external coding agent"
            },
            "executor": {
                "type": "string",
                "enum": ["auto", "grok", "codex"],
                "default": "auto",
                "description": "auto follows the selected subscription. Explicit grok/codex never falls back. A connector request always requires Codex."
            },
            "connector": {
                "type": "string",
                "description": "Required for connected account reads: exact app id or display name, for example Gmail or Google Calendar. Codex verifies enabled and accessible before running. No connector mutations in v1."
            },
            "timeout_secs": {
                "type": "integer",
                "minimum": 30,
                "maximum": 3600,
                "description": "Wall-clock timeout for this run (default from external_executor_timeout_secs)"
            },
            "context": {
                "type": "string",
                "description": "Optional extra material appended to the prompt under a Context heading"
            }
        },
        "required": ["task"]
    });
    let function = FunctionObjectArgs::default()
        .name("run_external")
        .description(
            "Run one self-contained task in a fresh scratch directory and return a receipt. Omit executor to follow the selected chat subscription (Grok or Codex). Subscription CLIs must be installed and signed in on this host. For connected mailbox, calendar or drive reads, set connector to its exact app id or display name (for example Gmail); this routes to Codex and verifies access before execution. Connector mutations are disabled in v1; prepare proposed changes as text only. No cross-provider fallback by default. Never claim results absent from the receipt.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `secure_computer_use` pseudo-tool — Intent → Policy → Sign → Device.
pub(crate) fn build_secure_computer_use_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "cu_tool": {
                "type": "string",
                "description": "Secure CU logical tool name. Supported: system.session_probe, system.frontmost_app, system.accessible_tree (bounded depth), system.focused_control, system.screenshot_digest. Never shell.exec / eval."
            },
            "args": {
                "type": "object",
                "description": "Arguments for cu_tool (JSON object; keep small, no secrets)"
            },
            "objective": {
                "type": "string",
                "description": "Optional owner objective for the Policy Broker (Jev)"
            }
        },
        "required": ["cu_tool"]
    });
    let function = FunctionObjectArgs::default()
        .name("secure_computer_use")
        .description(
            "Run one Secure Computer Use intent on this host (Linux AT-SPI when available, else loopback). Goes through Policy Broker → Ed25519 sign → Device Agent verify/anti-replay → audit chain. Read-only tools: system.session_probe, system.frontmost_app, system.accessible_tree, system.focused_control, system.screenshot_digest. Never claim OS side effects the receipt does not contain. Disabled unless AEGIS_SECURE_CU_ENABLED=1.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

/// Build the `browser_fetch` pseudo-tool — render one page in the machine's
/// headless Chrome (JS runs) and return readable text or a screenshot.
pub(crate) fn build_browser_fetch_tool() -> Result<ChatCompletionTools> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "http(s) page to render"
            },
            "mode": {
                "type": "string",
                "enum": ["text", "screenshot"],
                "default": "text",
                "description": "text = readable page text; screenshot = full-page PNG shown to the owner"
            },
            "selector": {
                "type": "string",
                "description": "Optional CSS selector; text mode returns only the first matching element"
            },
            "wait_ms": {
                "type": "integer",
                "minimum": 0,
                "maximum": 10000,
                "default": 1500,
                "description": "Extra time for scripts to settle before capture"
            },
            "max_chars": {
                "type": "integer",
                "minimum": 500,
                "maximum": 20000,
                "default": 6000
            }
        },
        "required": ["url"]
    });
    let function = FunctionObjectArgs::default()
        .name("browser_fetch")
        .description(
            "Render one web page in the machine's real headless Chrome (JavaScript runs) and return its readable text, or a screenshot the owner can see. Use when web_reader fails or the page needs JS (maps, dashboards, single-page apps, charts). One page per call; no logins, no clicks — for multi-step browsing use run_external. Never put street addresses, plate numbers or phone numbers in the URL.",
        )
        .parameters(schema)
        .build()?;
    Ok(ChatCompletionTools::Function(ChatCompletionTool {
        function,
    }))
}

// ── MCP Server Tool Aggregator ─────────────────────────────────────────────

/// Return pseudo-tool schemas suitable for MCP server registration.
///
/// Excludes Telegram-specific tools that have no meaning in MCP context
/// (notify_user, set_bot_commands, text_to_speech).
pub fn mcp_pseudo_tool_schemas() -> Vec<ChatCompletionTools> {
    let builders: &[fn() -> Result<ChatCompletionTools>] = &[
        // Core
        build_get_current_time_tool,
        build_recall_memory_tool,
        build_update_profile_tool,
        build_web_search_tool,
        build_x_search_tool,
        build_search_tools_tool,
        build_generate_image_tool,
        // System
        build_update_setting_tool,
        build_switch_model_tool,
        build_unset_embedding_primary_promotion_tool,
        build_trigger_corpus_reembed_tool,
        build_view_logs_tool,
        build_set_log_level_tool,
        build_check_api_usage_tool,
        // Tasks & Knowledge
        build_manage_tasks_tool,
        build_knowledge_base_tool,
        // Skill management
        build_deactivate_skill_tool,
        build_reactivate_skill_tool,
        build_manage_skill_tool,
        // Google account + Drive
        build_setup_google_tool,
        build_google_drive_upload_tool,
        // Calendar (write)
        build_calendar_create_event_tool,
        build_calendar_update_event_tool,
        build_calendar_delete_event_tool,
        // Intents & Monitoring
        build_schedule_intent_tool,
        build_list_intents_tool,
        build_cancel_intent_tool,
        build_set_monitoring_intent_tool,
        build_list_monitoring_intents_tool,
        build_remove_monitoring_intent_tool,
        build_set_alert_policy_tool,
        build_list_alert_policies_tool,
        // Heartbeat
        build_update_heartbeat_memo_tool,
        build_view_heartbeat_audit_tool,
        build_configure_autonomous_tool,
        // Goals (G5)
        build_manage_goals_tool,
        build_delegate_task_tool,
        build_run_external_tool,
        build_browser_fetch_tool,
        build_secure_computer_use_tool,
        // Companion Growth (handlers not yet wired — uncomment when execute path is ready)
        // build_view_growth_tool,
        // build_adjust_personality_tool,
    ];
    builders
        .iter()
        .filter_map(|b| match b() {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(error = %e, "MCP tool schema builder failed");
                None
            }
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
#[path = "tool_schemas_tests.rs"]
mod tests;
