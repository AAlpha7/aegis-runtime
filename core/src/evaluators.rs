//! Output evaluators — 3-layer review guard pipeline + callback notification checks.
//!
//! The review pipeline replaces the previous single-LLM approach with a tiered system:
//!
//! 1. **Layer 1 (FastPass)**: Deterministic rules — questions, short replies, greetings.
//!    Cost: ~0ms, catches ~60% of replies.
//!
//! 2. **Layer 2 (Heuristic + Semantic)**:
//!    - 2a: Tense/completion signal detection — if no completion markers, pass.
//!    - 2b: Embedding-based semantic matching — compare reply to tool descriptions,
//!      flag when the reply's closest tool wasn't actually called.
//!
//!    Cost: ~50-200ms (one embedding call), catches ~35% of remaining.
//!
//! 3. **Layer 3 (LLM Review)**: Full LLM grounding check, only for replies that
//!    survived layers 1-2 and were flagged by semantic matching.
//!    Cost: ~1-3s, used for ~5% of total replies.
//!
//! All layers are **fail-open**: any error returns the original reply unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionTools, CreateChatCompletionRequest, CreateChatCompletionRequestArgs,
    CreateChatCompletionResponse, ResponseFormat,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use regex::Regex;
use std::sync::LazyLock;

use crate::embedding::EmbeddingClient;
use crate::llm::{
    AegisChatRequest, AegisRequestMetadata, LlmRouter, LlmRouterConfig, LlmTaskClass,
    OpenAiCompatBackend,
};
use crate::orchestrator::NotifyUserRequest;
use crate::settings::RuntimeSettings;

fn route_review_guard_model(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
    label: &str,
) -> Result<String> {
    let config = LlmRouterConfig::from_runtime_settings_for_legacy_chat_path(settings)
        .map_err(anyhow::Error::new)?;
    let backend = OpenAiCompatBackend::new(llm_client.clone(), model);
    let router = LlmRouter::new(Arc::new(backend), config);
    let request = AegisChatRequest {
        model: model.to_string(),
        messages: Vec::new(),
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        tool_choice: None,
        metadata: AegisRequestMetadata {
            task_class: LlmTaskClass::ReviewGuard,
            request_id: None,
            chat_id: None,
            user_visible: false,
            reasoning_effort: None,
        },
    };
    let decision = router.route_chat(&request).map_err(anyhow::Error::new)?;
    tracing::info!(
        label,
        provider = %decision.provider,
        model = %decision.model,
        task_class = ?LlmTaskClass::ReviewGuard,
        is_fallback = decision.is_fallback,
        paid_api_provider = decision.provider.is_paid_api(),
        paid_api_fallback_allowed = settings
            .get("paid_api_fallback_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "[LLM Route]"
    );
    Ok(decision.model)
}

async fn create_review_guard_chat_completion(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    mut request: CreateChatCompletionRequest,
    model: &str,
    label: &str,
    timeout: Duration,
    timeout_context: &'static str,
    failure_context: &'static str,
) -> Result<CreateChatCompletionResponse> {
    request.model = route_review_guard_model(llm_client, settings, model, label)?;
    tokio::time::timeout(timeout, llm_client.chat().create(request))
        .await
        .context(timeout_context)?
        .context(failure_context)
}

// ── Model resolution ─────────────────────────────────────────────────────────

/// Resolve the model for review/evaluation tasks.
/// Priority: review_model setting → GUARD_MODEL env → fallback.
///
/// Uses `get_if_stored` so the env branch is actually reachable when the
/// setting is unset — `get` would always return `Some(default)` and silently
/// shadow `GUARD_MODEL`. Today `review_model`'s registered default is `""`
/// so the `.filter(|s| !s.is_empty())` happens to mask the bug; switching
/// to `get_if_stored` preserves current behavior and makes the chain
/// robust if the default ever changes to non-empty. See structural
/// invariant test `settings::tests::stored_keys_starts_empty_after_new`.
pub fn resolve_review_model(settings: &RuntimeSettings, fallback_model: &str) -> String {
    settings
        .get_if_stored("review_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GUARD_MODEL").ok())
        .unwrap_or_else(|| fallback_model.to_string())
}

// ── 3-Layer Review Pipeline Types ────────────────────────────────────────────

/// Which layer caught/passed the reply.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ReviewLayer {
    /// Layer 1: Fast deterministic pass (greeting, question, short reply).
    FastPass,
    /// Layer 2a: No completion signal detected in the reply.
    NoCompletionSignal,
    /// Layer 2b: Completion signal found but tool was called (or no semantic match).
    SemanticPass,
    /// Layer 2b: Semantic mismatch detected — tool not called.
    SemanticFlag,
    /// Layer 3: LLM review (fallback for ambiguous cases).
    LlmReview,
}

impl ReviewLayer {
    /// Short string label for storage/display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FastPass => "fast_pass",
            Self::NoCompletionSignal => "no_completion_signal",
            Self::SemanticPass => "semantic_pass",
            Self::SemanticFlag => "semantic_flag",
            Self::LlmReview => "llm_review",
        }
    }
}

/// Result of the 3-layer review pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub layer: ReviewLayer,
    pub passed: bool,
    pub tool_matched: Option<String>,
    pub similarity: Option<f32>,
    pub reason: Option<String>,
    pub corrected_reply: Option<String>,
    pub latency_ms: u64,
}

/// Record for effectiveness tracking (stored in DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub timestamp: String,
    pub reply_preview: String,
    pub tools_called: Vec<String>,
    pub result: ReviewResult,
}

/// Aggregate stats from the review_log table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewStats {
    pub total: u64,
    pub fast_pass: u64,
    pub no_completion_signal: u64,
    pub semantic_pass: u64,
    pub semantic_flag: u64,
    pub llm_review: u64,
    pub flagged: u64,
    pub avg_latency_ms: f64,
}

// ── Internal semantic result ─────────────────────────────────────────────────

enum SemanticResult {
    Pass,
    Mismatch { tool: String, similarity: f32 },
    Error(String),
}

/// Compact Action Ledger evidence passed into the review guard.
///
/// The evaluator deliberately depends on this small read-only shape instead of
/// the storage model so completion-claim checks remain optional and cheap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerReceiptEvidence {
    pub action_kind: String,
    pub status: String,
    pub has_receipt: bool,
    pub user_visible_summary: Option<String>,
}

// ── Completion signal markers ────────────────────────────────────────────────

/// Chinese completion markers — past tense / completion indicators.
const ZH_COMPLETION: &[&str] = &[
    "已",
    "完成",
    "设置好",
    "搞定",
    "好了",
    "成功",
    "创建了",
    "发送了",
    "添加了",
    "加入",
    "删除了",
    "保存了",
    "激活",
    "批准了",
    "配置了",
    "安排了",
    "更新了",
    "修改了",
    "部署了",
    "安装了",
    "执行了",
    "处理了",
    "上传了",
    "下载了",
    "转发了",
    "取消了",
    "撤销了",
];

/// English completion markers — past-tense action verbs.
const EN_COMPLETION: &[&str] = &[
    "done",
    "completed",
    "created",
    "sent",
    "saved",
    "deleted",
    "activated",
    "configured",
    "scheduled",
    "added",
    "set up",
    "removed",
    "approved",
    "updated",
    "modified",
    "established",
    "submitted",
    "dispatched",
    "enabled",
    "disabled",
    "installed",
    "deployed",
    "executed",
    "processed",
    "uploaded",
    "downloaded",
    "transferred",
    "cancelled",
    "revoked",
];

/// English negation words that, when appearing within a few words before a
/// completion marker, indicate the action was NOT performed.
/// E.g., "not completed", "hasn't been deleted", "won't be sent".
const EN_NEGATION_WORDS: &[&str] = &[
    "not", "n't", "never", "cannot", "can't", "won't", "didn't", "hasn't", "haven't", "wasn't",
    "weren't", "couldn't",
];

/// External action keywords that should be backed by an executed ledger row
/// with receipt-like evidence before the assistant uses strong completion
/// wording. This is intentionally narrower than `*_COMPLETION` to avoid
/// penalizing read-only summaries such as "I found...".
const LEDGER_RECEIPT_ACTION_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "email_send",
        &[
            "sent the email",
            "sent an email",
            "email has been sent",
            "email was sent",
            "emailed",
            "邮件已发送",
            "已发送邮件",
            "已经发送邮件",
            "邮件发送成功",
        ],
    ),
    (
        "email_draft",
        &[
            "created the draft",
            "created a draft",
            "draft has been created",
            "draft was created",
            "saved the draft",
            "草稿已创建",
            "已创建草稿",
            "已经创建草稿",
        ],
    ),
    (
        "email_forward",
        &[
            "forwarded the email",
            "email has been forwarded",
            "邮件已转发",
            "已转发邮件",
        ],
    ),
    (
        "calendar_create",
        &[
            "created the calendar event",
            "created a calendar event",
            "created the event",
            "calendar event has been created",
            "calendar event was created",
            "scheduled the meeting",
            "日程已创建",
            "已创建日程",
            "已经创建日程",
            "会议已安排",
            "已经安排会议",
        ],
    ),
    (
        "calendar_update",
        &[
            "updated the calendar event",
            "updated the event",
            "rescheduled the meeting",
            "日程已更新",
            "已更新日程",
        ],
    ),
    (
        "telegram_send",
        &[
            "sent the notification",
            "sent the message",
            "notification has been sent",
            "通知已发送",
        ],
    ),
    (
        "runtime_setting_change",
        &[
            "updated the setting",
            "changed the setting",
            "configured the setting",
            "设置已更新",
        ],
    ),
    (
        "skill_deploy",
        &[
            "deployed the skill",
            "deployment completed",
            "已部署技能",
            "部署完成",
        ],
    ),
    (
        "code_commit",
        &[
            "committed the change",
            "commit has been created",
            "已提交代码",
        ],
    ),
    (
        "pull_request_create",
        &[
            "created the pull request",
            "opened the pull request",
            "已创建 pull request",
        ],
    ),
];

// ── Agent review prompt (Layer 3) ────────────────────────────────────────────

/// System prompt for the agent review guard (Layer 3 only).
/// Invoked only when Layer 2b flags a semantic mismatch.
pub const AGENT_REVIEW_PROMPT: &str = "\
You are a grounding reviewer for an AI assistant reply. You will receive:\n\
- AVAILABLE_TOOLS: all tools/skills the assistant has access to\n\
- TOOLS_CALLED: tools actually called this turn\n\
- FLAGGED_TOOL: the tool the reply appears to claim usage of (detected by semantic matching)\n\
- REPLY: the assistant's text reply\n\
\n\
Check the reply for:\n\
\n\
1. ACTION claims: the reply states a state-changing action was completed \
(created, deleted, updated, sent, saved, activated, approved, set, added, \
configured, scheduled, monitored) but the relevant tool was NOT called. Examples:\n\
   - \"任务已创建\" / \"Task created\" without calling manage_tasks\n\
   - \"Reminder set\" without calling schedule_intent\n\
   - ANY claim of having performed an action that requires a tool call\n\
\n\
2. DATA claims: specific real-time data (prices, live statuses, search results, \
system metrics) that can only be obtained via a tool call.\n\
\n\
NOT ungrounded (always pass):\n\
- Questions, offers, instructions, explanations, greetings\n\
- General knowledge, opinions, properly qualified statements\n\
- Referencing data the user provided in their message\n\
- Describing the assistant's own capabilities from AVAILABLE_TOOLS\n\
- Summarizing results from TOOLS_CALLED\n\
\n\
Reply with JSON:\n\
- If grounded: {\"grounded\": true}\n\
- If ungrounded: {\"grounded\": false, \"reason\": \"brief explanation\", \
\"corrected_reply\": \"the reply with false claims edited out, preserving all grounded content\"}";

/// Parsed result of the agent review check.
#[derive(Debug)]
pub struct ReviewVerdict {
    pub grounded: bool,
    pub reason: Option<String>,
    pub corrected_reply: Option<String>,
}

/// Parse the review agent's JSON response, defaulting to `grounded: true` on any parse failure.
pub fn parse_review_verdict(text: &str) -> ReviewVerdict {
    if let Ok(val) = serde_json::from_str::<Value>(text.trim()) {
        let grounded = val
            .get("grounded")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let reason = val
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let corrected_reply = val
            .get("corrected_reply")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        ReviewVerdict {
            grounded,
            reason,
            corrected_reply,
        }
    } else {
        // Fail-open: treat as grounded
        ReviewVerdict {
            grounded: true,
            reason: None,
            corrected_reply: None,
        }
    }
}

// ── 3-Layer Pipeline Entry Point ─────────────────────────────────────────────

/// Run the 3-layer review pipeline on an LLM reply.
///
/// Layers are applied in order; each layer either passes the reply or escalates
/// to the next. All layers are fail-open.
///
/// Arguments:
/// - `llm_client`: OpenAI-compatible client for Layer 3 LLM calls.
/// - `embedder`: Embedding client for Layer 2b semantic matching.
/// - `model`: Model name for the Layer 3 LLM review.
/// - `original_reply`: The LLM's text reply to evaluate.
/// - `tools_called`: Names of tools actually called this turn.
/// - `tools`: Available tool definitions (for Layer 3 context).
/// - `tool_embeddings`: Pre-computed embeddings of tool descriptions (for Layer 2b).
pub async fn run_review_pipeline(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    embedder: &EmbeddingClient,
    model: &str,
    original_reply: &str,
    tools_called: &[String],
    tools: &[ChatCompletionTools],
    tool_embeddings: &HashMap<String, Vec<f32>>,
    ledger_receipts: Option<&[LedgerReceiptEvidence]>,
) -> ReviewResult {
    let start = std::time::Instant::now();

    if let Some(result) = run_review_pipeline_precheck(original_reply, ledger_receipts) {
        return result;
    }

    // Layer 2b: Semantic matching — compare reply embedding to tool description embeddings
    // Skip if no tool embeddings are available yet (lazy init not done)
    if tool_embeddings.is_empty() {
        // Fall through to LLM review as a safety net
        tracing::debug!("[ReviewPipeline] No tool embeddings available, falling through to LLM");
    } else {
        match check_semantic_match(embedder, original_reply, tools_called, tool_embeddings).await {
            SemanticResult::Pass => {
                return ReviewResult {
                    layer: ReviewLayer::SemanticPass,
                    passed: true,
                    tool_matched: None,
                    similarity: None,
                    reason: None,
                    corrected_reply: None,
                    latency_ms: start.elapsed().as_millis() as u64,
                };
            }
            SemanticResult::Mismatch { tool, similarity } => {
                // Layer 3: LLM confirmation — is this really a false claim?
                let llm_result = run_llm_review(
                    llm_client,
                    settings,
                    model,
                    original_reply,
                    tools_called,
                    &tool,
                    tools,
                )
                .await;

                let elapsed = start.elapsed().as_millis() as u64;

                return match llm_result {
                    Ok(verdict) => {
                        if verdict.grounded {
                            // LLM says it's fine — semantic match was a false alarm
                            ReviewResult {
                                layer: ReviewLayer::LlmReview,
                                passed: true,
                                tool_matched: Some(tool),
                                similarity: Some(similarity),
                                reason: None,
                                corrected_reply: None,
                                latency_ms: elapsed,
                            }
                        } else {
                            // LLM confirms: ungrounded claim
                            tracing::info!(
                                tool = %tool,
                                similarity = %similarity,
                                reason = %verdict.reason.as_deref().unwrap_or("unknown"),
                                "[ReviewPipeline] Reply flagged: semantic mismatch confirmed by LLM"
                            );
                            ReviewResult {
                                layer: ReviewLayer::SemanticFlag,
                                passed: false,
                                tool_matched: Some(tool),
                                similarity: Some(similarity),
                                reason: verdict.reason,
                                corrected_reply: verdict.corrected_reply,
                                latency_ms: elapsed,
                            }
                        }
                    }
                    Err(e) => {
                        // Fail-open: LLM error → pass
                        tracing::debug!(
                            error = %e,
                            "[ReviewPipeline] LLM review failed (fail-open)"
                        );
                        ReviewResult {
                            layer: ReviewLayer::LlmReview,
                            passed: true,
                            tool_matched: Some(tool),
                            similarity: Some(similarity),
                            reason: Some(format!("LLM review failed: {e}")),
                            corrected_reply: None,
                            latency_ms: elapsed,
                        }
                    }
                };
            }
            SemanticResult::Error(e) => {
                tracing::debug!(
                    error = %e,
                    "[ReviewPipeline] Semantic match failed, falling through to LLM"
                );
                // Fall through to LLM review below
            }
        }
    }

    // Fallback: direct LLM review (when semantic matching is unavailable)
    let llm_result = run_llm_review(
        llm_client,
        settings,
        model,
        original_reply,
        tools_called,
        "",
        tools,
    )
    .await;

    let elapsed = start.elapsed().as_millis() as u64;

    match llm_result {
        Ok(verdict) => {
            if verdict.grounded {
                ReviewResult {
                    layer: ReviewLayer::LlmReview,
                    passed: true,
                    tool_matched: None,
                    similarity: None,
                    reason: None,
                    corrected_reply: None,
                    latency_ms: elapsed,
                }
            } else {
                tracing::info!(
                    reason = %verdict.reason.as_deref().unwrap_or("unknown"),
                    "[ReviewPipeline] Reply flagged by LLM review (no semantic layer)"
                );
                ReviewResult {
                    layer: ReviewLayer::LlmReview,
                    passed: false,
                    tool_matched: None,
                    similarity: None,
                    reason: verdict.reason,
                    corrected_reply: verdict.corrected_reply,
                    latency_ms: elapsed,
                }
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "[ReviewPipeline] LLM review failed (fail-open)");
            ReviewResult {
                layer: ReviewLayer::LlmReview,
                passed: true,
                tool_matched: None,
                similarity: None,
                reason: Some(format!("LLM review failed: {e}")),
                corrected_reply: None,
                latency_ms: elapsed,
            }
        }
    }
}

/// Cheap deterministic review stages that never need tool embeddings or LLM calls.
///
/// Callers can run this before warming the expensive semantic-review embedding
/// cache. `run_review_pipeline` also uses this helper so the two paths cannot
/// drift.
pub fn run_review_pipeline_precheck(
    original_reply: &str,
    ledger_receipts: Option<&[LedgerReceiptEvidence]>,
) -> Option<ReviewResult> {
    let start = std::time::Instant::now();

    // Layer 1: Fast pass -- questions, greetings, short replies.
    if is_fast_pass(original_reply) {
        return Some(ReviewResult {
            layer: ReviewLayer::FastPass,
            passed: true,
            tool_matched: None,
            similarity: None,
            reason: None,
            corrected_reply: None,
            latency_ms: start.elapsed().as_millis() as u64,
        });
    }

    // Layer 2a: Tense/completion heuristic -- if no completion signal, pass.
    if !has_completion_signal(original_reply) {
        return Some(ReviewResult {
            layer: ReviewLayer::NoCompletionSignal,
            passed: true,
            tool_matched: None,
            similarity: None,
            reason: None,
            corrected_reply: None,
            latency_ms: start.elapsed().as_millis() as u64,
        });
    }

    if let Some((claimed_kind, reason)) =
        missing_ledger_receipt_for_completion_claim(original_reply, ledger_receipts)
    {
        tracing::info!(
            action_kind = %claimed_kind,
            reason = %reason,
            "[ReviewPipeline] Reply flagged: completion claim lacks Action Ledger receipt"
        );
        return Some(ReviewResult {
            layer: ReviewLayer::SemanticFlag,
            passed: false,
            tool_matched: Some(claimed_kind),
            similarity: None,
            reason: Some(reason),
            corrected_reply: Some(
                "I can't verify from the Action Ledger that this external action completed. Please check the action status before treating it as done."
                    .to_string(),
            ),
            latency_ms: start.elapsed().as_millis() as u64,
        });
    }

    None
}

// ── Layer 1: Fast Pass ───────────────────────────────────────────────────────

/// Deterministic fast-pass: replies that structurally cannot contain action claims.
///
/// Catches questions, very short replies, and simple greetings.
///
/// Important: questions that also contain completion signals (e.g.,
/// "I've sent the email. Anything else?") are NOT fast-passed — they must
/// go through Layer 2 to catch false action claims embedded before the
/// trailing question.
fn is_fast_pass(reply: &str) -> bool {
    let trimmed = reply.trim();

    // Empty replies pass trivially
    if trimmed.is_empty() {
        return true;
    }

    // Very short replies (< 20 chars) are unlikely to contain action claims.
    // Check this BEFORE the question check so short bare questions like
    // "What now?" still fast-pass via the length short-circuit.
    if trimmed.len() < 20 {
        return true;
    }

    // Questions end with ? or fullwidth ? — but only if they don't also
    // contain completion markers (which would indicate an action claim
    // followed by a trailing question like "I've sent the email. Anything
    // else?"). Such replies must fall through to Layer 2 validation.
    if (trimmed.ends_with('?') || trimmed.ends_with('\u{FF1F}')) && !has_completion_signal(trimmed)
    {
        return true;
    }

    false
}

// ── Layer 2a: Completion Signal Detection ────────────────────────────────────

/// Check if the reply contains any completion/past-tense markers that suggest
/// it's claiming an action was performed.
///
/// Negation-aware: "I have NOT completed the task" / "任务还没完成" do NOT
/// count as completion signals because the marker is preceded by a negation
/// (English: any of `EN_NEGATION_WORDS` within the previous 4 whitespace
/// tokens; Chinese: structural `没` / `未` / `没有` immediately before the
/// marker).
///
/// All marker occurrences are scanned (not just the first match), because a
/// reply may contain "没完成" early and "已发送" later — the latter still
/// counts as a positive claim.
fn has_completion_signal(reply: &str) -> bool {
    // Chinese: structural negation — "没" / "未" / "没有" immediately before
    // a marker negates it. Scan ALL occurrences, not just the first.
    let zh_hit = ZH_COMPLETION.iter().any(|m| {
        let mut start = 0;
        while let Some(rel) = reply[start..].find(m) {
            let pos = start + rel;
            let before = &reply[..pos];
            let negated =
                before.ends_with('没') || before.ends_with('未') || before.ends_with("没有");
            if !negated {
                return true;
            }
            start = pos + m.len();
        }
        false
    });

    if zh_hit {
        return true;
    }

    // English: 4-token lookback window before each marker occurrence. The
    // `tok.ends_with(neg)` check lets the bare `n't` entry catch every
    // contraction; explicit `didn't`/`hasn't`/... entries remain for
    // tokenizers that strip punctuation.
    let lower = reply.to_lowercase();
    EN_COMPLETION.iter().any(|m| {
        let mut start = 0;
        while let Some(rel) = lower[start..].find(m) {
            let pos = start + rel;
            let before = &lower[..pos];
            let preceding_tokens: Vec<&str> = before.split_whitespace().rev().take(4).collect();
            let negated = preceding_tokens.iter().any(|tok| {
                EN_NEGATION_WORDS
                    .iter()
                    .any(|neg| tok == neg || tok.ends_with(neg))
            });
            if !negated {
                return true;
            }
            start = pos + m.len();
        }
        false
    })
}

// ── Layer 2b: Semantic Matching ──────────────────────────────────────────────

/// Compare the reply's embedding to pre-computed tool description embeddings.
///
/// If the reply is semantically close to a tool's domain (cosine > 0.6) but
/// that tool wasn't actually called, flag it for LLM review.
async fn check_semantic_match(
    embedder: &EmbeddingClient,
    reply: &str,
    tools_called: &[String],
    tool_embeddings: &HashMap<String, Vec<f32>>,
) -> SemanticResult {
    // Get reply embedding
    let reply_vec = match embedder.embed(reply).await {
        Ok(v) => v,
        Err(e) => return SemanticResult::Error(format!("{e}")),
    };

    // Find most similar tool
    let mut best_tool = String::new();
    let mut best_sim: f32 = 0.0;
    for (name, vec) in tool_embeddings {
        let sim = EmbeddingClient::cosine_similarity(&reply_vec, vec);
        if sim > best_sim {
            best_sim = sim;
            best_tool = name.clone();
        }
    }

    // Threshold: 0.6 means the reply is semantically close to a tool's domain
    if best_sim > 0.6 && !tools_called.contains(&best_tool) {
        SemanticResult::Mismatch {
            tool: best_tool,
            similarity: best_sim,
        }
    } else {
        SemanticResult::Pass
    }
}

// ── Layer 3: LLM Review ─────────────────────────────────────────────────────

/// Full LLM grounding check. Called only when Layer 2b flags a potential mismatch.
///
/// Fail-open: returns Err on any failure (timeout, parse, API error).
async fn run_llm_review(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
    original_reply: &str,
    tools_called: &[String],
    flagged_tool: &str,
    tools: &[ChatCompletionTools],
) -> Result<ReviewVerdict> {
    // Build compact available tools list
    let available_tools_list: String = tools
        .iter()
        .filter_map(|t| {
            if let ChatCompletionTools::Function(tool) = t {
                Some(tool.function.name.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let tools_called_list = if tools_called.is_empty() {
        "(none)".to_string()
    } else {
        tools_called.join(", ")
    };

    let review_system = ChatCompletionRequestSystemMessageArgs::default()
        .content(AGENT_REVIEW_PROMPT)
        .build()?
        .into();

    let review_input = format!(
        "AVAILABLE_TOOLS: [{}]\nTOOLS_CALLED: [{}]\nFLAGGED_TOOL: {}\n\nREPLY:\n{}",
        available_tools_list, tools_called_list, flagged_tool, original_reply
    );
    let review_user = ChatCompletionRequestUserMessageArgs::default()
        .content(&*review_input)
        .build()?
        .into();

    let review_request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![review_system, review_user])
        .response_format(ResponseFormat::JsonObject)
        .build()?;

    let review_response = create_review_guard_chat_completion(
        llm_client,
        settings,
        review_request,
        model,
        "agent_review",
        Duration::from_secs(10),
        "Agent review timed out (10s)",
        "Agent review LLM call failed",
    )
    .await?;
    crate::llm_client::log_llm_usage(&review_response, "agent_review");

    let review_text = review_response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or(r#"{"grounded": true}"#);

    Ok(parse_review_verdict(review_text))
}

// ── Legacy single-LLM review (kept for backward compat) ─────────────────────

/// Legacy agent review guard: single LLM call for zero-tool replies.
/// Replaced by `run_review_pipeline` but retained for tests and edge cases.
///
/// Fail-open: any error (timeout, parse failure) returns the original reply.
pub async fn run_agent_review(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    model: &str,
    original_reply: &str,
    tools: &[ChatCompletionTools],
) -> Result<String> {
    let verdict =
        run_llm_review(llm_client, settings, model, original_reply, &[], "", tools).await?;

    if verdict.grounded {
        return Ok(original_reply.to_string());
    }

    tracing::info!(
        reason = %verdict.reason.as_deref().unwrap_or("unknown"),
        "[AgentReview] Reply flagged as ungrounded, using corrected version"
    );

    // Use corrected reply if provided, otherwise return original (fail-open)
    Ok(verdict
        .corrected_reply
        .unwrap_or_else(|| original_reply.to_string()))
}

/// Cheap precheck for callers that want to avoid even opening the ledger unless
/// the reply contains an external-action completion claim.
pub fn reply_may_need_ledger_receipt_check(reply: &str) -> bool {
    has_completion_signal(reply) && !claimed_ledger_action_kinds(reply).is_empty()
}

fn missing_ledger_receipt_for_completion_claim(
    reply: &str,
    ledger_receipts: Option<&[LedgerReceiptEvidence]>,
) -> Option<(String, String)> {
    let claimed_kinds = claimed_ledger_action_kinds(reply);
    if claimed_kinds.is_empty() {
        return None;
    }
    let ledger_receipts = ledger_receipts?;

    for claimed_kind in claimed_kinds {
        let has_matching_receipt = ledger_receipts.iter().any(|entry| {
            entry.action_kind == claimed_kind && entry.status == "executed" && entry.has_receipt
        });
        if !has_matching_receipt {
            return Some((
                claimed_kind.to_string(),
                format!(
                    "completion claim for {claimed_kind} has no recent executed Action Ledger receipt"
                ),
            ));
        }
    }

    None
}

fn claimed_ledger_action_kinds(reply: &str) -> Vec<&'static str> {
    let lower = reply.to_lowercase();
    LEDGER_RECEIPT_ACTION_KEYWORDS
        .iter()
        .filter_map(|(kind, keywords)| {
            keywords
                .iter()
                .any(|keyword| lower.contains(&keyword.to_lowercase()))
                .then_some(*kind)
        })
        .collect()
}

// ── Tool Embedding Pre-computation ──────────────────────────────────────────

/// Pre-compute embeddings for all tool descriptions.
///
/// Called lazily on first review. Results are cached in the orchestrator.
/// Failures for individual tools are silently skipped (partial cache is fine).
pub async fn precompute_tool_embeddings(
    embedder: &EmbeddingClient,
    tools: &[ChatCompletionTools],
) -> HashMap<String, Vec<f32>> {
    let mut map = HashMap::new();
    for tool in tools {
        if let ChatCompletionTools::Function(f) = tool {
            let desc = f
                .function
                .description
                .as_deref()
                .unwrap_or(&f.function.name);
            match embedder.embed(desc).await {
                Ok(vec) => {
                    map.insert(f.function.name.clone(), vec);
                }
                Err(e) => {
                    tracing::debug!(
                        tool = %f.function.name,
                        error = %e,
                        "[ReviewPipeline] Failed to embed tool description"
                    );
                }
            }
        }
    }
    tracing::info!(
        count = map.len(),
        "[ReviewPipeline] Pre-computed tool description embeddings"
    );
    map
}

// ── Factual Grounding Guard: reference intent embeddings ─────────────────────

/// Reference intents for the Factual Grounding Guard.
/// Matched against user message embeddings to detect corrections and
/// time-sensitive queries that require web_search verification.
const FACTUAL_GROUNDING_INTENTS: &[&str] = &[
    "The user is correcting wrong or inaccurate information from my previous response",
    "The user is asking about product pricing, subscription plans, version info, or feature availability",
    "The user is providing a specific fact that contradicts what I said earlier",
];

/// Pre-compute embeddings for factual grounding reference intents.
/// Called once lazily (via `OnceCell`) on first message after startup.
/// Returns a Vec of embedding vectors, one per intent.
pub async fn precompute_factual_grounding_intents(
    embedder: &crate::embedding::EmbeddingClient,
) -> Vec<Vec<f32>> {
    let mut vecs = Vec::with_capacity(FACTUAL_GROUNDING_INTENTS.len());
    for intent in FACTUAL_GROUNDING_INTENTS {
        match embedder.embed(intent).await {
            Ok(v) => vecs.push(v),
            Err(e) => {
                tracing::warn!(
                    intent,
                    error = %e,
                    "[FactualGrounding] Failed to embed reference intent"
                );
            }
        }
    }
    tracing::info!(
        count = vecs.len(),
        "[FactualGrounding] Pre-computed reference intent embeddings"
    );
    vecs
}

// ── Deterministic grounding check ────────────────────────────────────────────

/// Result of deterministic grounding analysis (zero API cost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingReport {
    /// Ratio of grounded claims to total claims (0.0 to 1.0).
    pub grounding_ratio: f32,
    /// Claims not found in any tool result.
    pub ungrounded_claims: Vec<String>,
    /// Number of claims matched in tool results.
    pub grounded_claims: usize,
    /// Total claims extracted.
    pub total_claims: usize,
}

static NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d[\d,]*\.?\d*").unwrap());
static URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s)>\]]+").unwrap());
static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap());
static QUOTED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[""\u{201C}]([^""\u{201D}]+)[""\u{201D}]"#).unwrap());

/// Check if a notification's factual claims are grounded in tool results.
///
/// Extracts numbers, URLs, emails, and quoted strings from the notification,
/// then verifies each appears in at least one tool result. Returns a
/// `GroundingReport` with the ratio and list of ungrounded claims.
///
/// Zero API cost — pure string matching with compiled regexes.
pub fn check_grounding(notification: &str, tool_results: &[String]) -> GroundingReport {
    let corpus: String = tool_results.join(" ");
    let mut claims: Vec<(String, bool)> = Vec::new();

    // Extract numbers (strip commas for matching: "65,000" matches "65000")
    for m in NUM_RE.find_iter(notification) {
        let raw = m.as_str().to_string();
        let normalized = raw.replace(',', "");
        // Skip trivially short numbers (single digit) — too many false positives
        if normalized.len() < 2 {
            continue;
        }
        let grounded = corpus.contains(&normalized) || corpus.contains(m.as_str());
        claims.push((raw, grounded));
    }

    // Extract URLs
    for m in URL_RE.find_iter(notification) {
        let url = m.as_str().to_string();
        let grounded = corpus.contains(m.as_str());
        claims.push((url, grounded));
    }

    // Extract emails
    for m in EMAIL_RE.find_iter(notification) {
        let email = m.as_str().to_string();
        let grounded = corpus.contains(m.as_str());
        claims.push((email, grounded));
    }

    // Extract quoted strings (> 3 chars to skip punctuation artifacts)
    for cap in QUOTED_RE.captures_iter(notification) {
        if let Some(inner) = cap.get(1) {
            let text = inner.as_str().to_string();
            if text.len() > 3 {
                let grounded = corpus.contains(inner.as_str());
                claims.push((text, grounded));
            }
        }
    }

    // Deduplicate claims by value
    claims.sort_by(|a, b| a.0.cmp(&b.0));
    claims.dedup_by(|a, b| a.0 == b.0);

    let total = claims.len();
    let grounded = claims.iter().filter(|(_, g)| *g).count();
    let ungrounded: Vec<String> = claims
        .iter()
        .filter(|(_, g)| !*g)
        .map(|(c, _)| c.clone())
        .collect();

    // When there are zero extractable claims, check whether the corpus actually
    // contains meaningful data.  Structural check: a corpus under 20 chars is
    // almost certainly a placeholder — penalise instead of trivially grounding.
    let grounding_ratio = if total == 0 {
        let trimmed = corpus.trim();
        let is_empty_corpus = trimmed.is_empty() || trimmed.len() < 20;
        if is_empty_corpus && !notification.trim().is_empty() {
            0.3 // low confidence: no claims AND no real tool data
        } else {
            1.0 // general/disclaimer message with real tool data — fine
        }
    } else {
        grounded as f32 / total as f32
    };

    GroundingReport {
        grounding_ratio,
        ungrounded_claims: ungrounded,
        grounded_claims: grounded,
        total_claims: total,
    }
}

/// Enhanced grounding check with source attribution.
///
/// When `source_tools` is non-empty, only checks claims against tool results
/// that contain the named tool (matched by tool name appearing in the result text).
/// Sourced notifications get a +0.1 bonus to grounding_ratio (incentivizes attribution).
/// Falls back to `check_grounding()` when source_tools is empty.
pub fn check_grounding_with_sources(
    notification: &str,
    tool_results: &[String],
    source_tools: &[String],
) -> GroundingReport {
    if source_tools.is_empty() {
        // Unsourced: use original check, no bonus
        return check_grounding(notification, tool_results);
    }
    // Filter tool_results to only those mentioning a declared source tool
    let filtered: Vec<&String> = tool_results
        .iter()
        .filter(|r| source_tools.iter().any(|st| r.contains(st.as_str())))
        .collect();
    let results_for_check: Vec<String> = if filtered.is_empty() {
        // Source declared but no matching results found — check against all
        // (source might be a wasm skill whose output doesn't contain its name)
        tool_results.to_vec()
    } else {
        filtered.into_iter().cloned().collect()
    };
    let mut report = check_grounding(notification, &results_for_check);
    // Bonus: declaring sources signals non-fabrication intent
    report.grounding_ratio = (report.grounding_ratio + 0.1).min(1.0);
    report
}

// ── Callback notification evaluator ──────────────────────────────────────────

/// Evaluator prompt for callback notifications.
/// Checks grounding against tool results and standalone clarity.
pub const CALLBACK_NOTIFICATION_EVALUATOR_PROMPT: &str = "\
You are a notification evaluator. You receive:\n\
- TOOL_RESULTS: outputs from tool calls made this turn\n\
- NOTIFICATIONS: messages about to be sent to the user\n\
\n\
Check each notification for:\n\
1. Grounded: every claim is supported by the TOOL_RESULTS (no hallucinated data)\n\
2. Standalone clarity: the user can understand the notification without seeing internal context\n\
3. Appropriate tone: not over-dramatizing or under-reporting\n\
4. Actionable value: the notification contains information the user can act on. \
Reports of \"no activity\" / \"nothing found\" / \"无动态\" should be REMOVED — \
absence of news is not worth a notification.\n\
\n\
5. If GROUNDING_ANALYSIS is provided, treat claims listed as 'ungrounded' with extra scrutiny. \
A low grounding ratio (< 0.5) strongly suggests hallucination — prefer removing unverifiable claims.\n\
\n\
Reply with JSON:\n\
- If all pass: {\"pass\": true}\n\
- If any fail: {\"pass\": false, \"corrected\": [\"corrected msg 1\", \"corrected msg 2\", ...]} \
where corrected has the same number of messages as NOTIFICATIONS, with minimal edits applied. \
Use empty string \"\" for notifications that should be removed entirely (e.g. no-activity reports).";

/// Parsed result of the notification evaluator.
#[derive(Debug)]
pub struct NotificationVerdict {
    pub pass: bool,
    pub corrected: Vec<String>,
}

/// Parse the notification evaluator's JSON response, defaulting to `pass: true` on any failure.
pub fn parse_notification_verdict(text: &str) -> NotificationVerdict {
    if let Ok(val) = serde_json::from_str::<Value>(text.trim()) {
        let pass = val.get("pass").and_then(|v| v.as_bool()).unwrap_or(true);
        let corrected = val
            .get("corrected")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        NotificationVerdict { pass, corrected }
    } else {
        // Fail-open: treat as pass
        NotificationVerdict {
            pass: true,
            corrected: vec![],
        }
    }
}

/// Evaluate callback notifications before delivery to the user.
///
/// Checks that notifications are grounded in tool results, have appropriate
/// tone, and are coherent as standalone messages. Returns the original
/// notifications on any error (fail-open).
pub async fn evaluate_notifications(
    llm_client: &Client<OpenAIConfig>,
    settings: &RuntimeSettings,
    fallback_model: &str,
    notifications: &[NotifyUserRequest],
    tool_results: &[String],
    grounding_reports: Option<&[GroundingReport]>,
) -> Vec<NotifyUserRequest> {
    // Nothing to evaluate
    if notifications.is_empty() {
        return notifications.to_vec();
    }

    // Runtime toggle — default to enabled
    let enabled = settings
        .get("evaluate_callbacks")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !enabled {
        return notifications.to_vec();
    }

    let review_model = resolve_review_model(settings, fallback_model);

    // Build notification list for the evaluator
    let notif_text: String = notifications
        .iter()
        .enumerate()
        .map(|(i, n)| format!("[{}] {}", i + 1, n.message))
        .collect::<Vec<_>>()
        .join("\n");

    // Summarize tool results (truncate each to keep prompt compact)
    let tool_summary: String = tool_results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let truncated = if r.len() > 500 {
                &r[..r.floor_char_boundary(500)]
            } else {
                r.as_str()
            };
            format!("[Tool {}] {}", i + 1, truncated)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let eval_system = match ChatCompletionRequestSystemMessageArgs::default()
        .content(CALLBACK_NOTIFICATION_EVALUATOR_PROMPT)
        .build()
    {
        Ok(msg) => msg.into(),
        Err(e) => {
            tracing::debug!(error = %e, "Failed to build evaluator system message (fail-open)");
            return notifications.to_vec();
        }
    };

    let grounding_section = if let Some(reports) = grounding_reports {
        let items: Vec<String> = reports
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "[Notification {}] Grounding: {:.0}% ({}/{} claims). Ungrounded: {:?}",
                    i + 1,
                    r.grounding_ratio * 100.0,
                    r.grounded_claims,
                    r.total_claims,
                    r.ungrounded_claims
                )
            })
            .collect();
        format!("\n\nGROUNDING_ANALYSIS:\n{}", items.join("\n"))
    } else {
        String::new()
    };

    let eval_input = format!(
        "TOOL_RESULTS:\n{}\n\nNOTIFICATIONS:\n{}{}",
        tool_summary, notif_text, grounding_section
    );
    let eval_user = match ChatCompletionRequestUserMessageArgs::default()
        .content(&*eval_input)
        .build()
    {
        Ok(msg) => msg.into(),
        Err(e) => {
            tracing::debug!(error = %e, "Failed to build evaluator user message (fail-open)");
            return notifications.to_vec();
        }
    };

    let eval_request = match CreateChatCompletionRequestArgs::default()
        .model(&*review_model)
        .messages(vec![eval_system, eval_user])
        .response_format(ResponseFormat::JsonObject)
        .build()
    {
        Ok(req) => req,
        Err(e) => {
            tracing::debug!(error = %e, "Failed to build evaluator request (fail-open)");
            return notifications.to_vec();
        }
    };

    let eval_response = match create_review_guard_chat_completion(
        llm_client,
        settings,
        eval_request,
        &review_model,
        "notif_eval",
        Duration::from_secs(8),
        "Notification evaluator timed out (8s)",
        "Notification evaluator LLM call failed",
    )
    .await
    {
        Ok(resp) => {
            crate::llm_client::log_llm_usage(&resp, "notif_eval");
            resp
        }
        Err(e) => {
            tracing::debug!(error = %e, "Notification evaluator LLM call failed (fail-open)");
            return notifications.to_vec();
        }
    };

    let eval_text = eval_response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or(r#"{"pass": true}"#);

    let verdict = parse_notification_verdict(eval_text);

    if verdict.pass {
        return notifications.to_vec();
    }

    tracing::info!("[NotificationEvaluator] Notifications corrected by evaluator");

    // Apply corrected messages if the count matches, otherwise fail-open
    let corrected = verdict.corrected;
    if corrected.len() == notifications.len() {
        notifications
            .iter()
            .zip(corrected)
            .filter(|(_, new_msg)| !new_msg.trim().is_empty()) // Remove notifications evaluator blanked out
            .map(|(orig, new_msg)| NotifyUserRequest {
                message: new_msg,
                pin: orig.pin,
                priority: orig.priority,
                source_tools: orig.source_tools.clone(),
            })
            .collect()
    } else {
        tracing::warn!(
            expected = notifications.len(),
            got = corrected.len(),
            "[NotificationEvaluator] Corrected count mismatch, using originals"
        );
        notifications.to_vec()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Layer 1: Fast pass tests ────────────────────────────────────────

    #[test]
    fn fast_pass_question() {
        assert!(is_fast_pass("What would you like me to do?"));
        assert!(is_fast_pass("你想让我做什么\u{FF1F}"));
    }

    #[test]
    fn fast_pass_short_reply() {
        assert!(is_fast_pass("Hello!"));
        assert!(is_fast_pass("Sure thing."));
        assert!(is_fast_pass(""));
        assert!(is_fast_pass("   "));
    }

    #[test]
    fn fast_pass_long_reply_does_not_pass() {
        assert!(!is_fast_pass(
            "I have created the proposal and set up the monitoring for your account."
        ));
    }

    // ── Layer 2a: Completion signal tests ───────────────────────────────

    #[test]
    fn completion_signal_english() {
        assert!(has_completion_signal("I've created the file for you."));
        assert!(has_completion_signal("Task completed successfully."));
        assert!(has_completion_signal(
            "Reminder has been set up for tomorrow."
        ));
        assert!(has_completion_signal("The item was deleted from the list."));
    }

    #[test]
    fn completion_signal_chinese() {
        assert!(has_completion_signal("提案已创建"));
        assert!(has_completion_signal("任务完成了"));
        assert!(has_completion_signal("已添加了监控"));
    }

    #[test]
    fn no_completion_signal() {
        assert!(!has_completion_signal(
            "I can help you with that. Would you like me to proceed?"
        ));
        assert!(!has_completion_signal(
            "Here are your options for the project."
        ));
        assert!(!has_completion_signal("Let me explain how this works."));
    }

    // ── Review verdict parsing tests ────────────────────────────────────

    #[test]
    fn review_verdict_grounded_true() {
        let v = parse_review_verdict(r#"{"grounded": true}"#);
        assert!(v.grounded);
        assert!(v.reason.is_none());
        assert!(v.corrected_reply.is_none());
    }

    #[test]
    fn review_verdict_false_with_correction() {
        let v = parse_review_verdict(
            r#"{"grounded": false, "reason": "claims action without tool", "corrected_reply": "I can help you with that. Would you like me to create a proposal?"}"#,
        );
        assert!(!v.grounded);
        assert_eq!(v.reason.as_deref(), Some("claims action without tool"));
        assert_eq!(
            v.corrected_reply.as_deref(),
            Some("I can help you with that. Would you like me to create a proposal?")
        );
    }

    #[test]
    fn review_verdict_malformed_defaults_grounded() {
        let v = parse_review_verdict("not json at all");
        assert!(
            v.grounded,
            "malformed response must default to grounded (fail-open)"
        );
    }

    #[test]
    fn review_verdict_empty_defaults_grounded() {
        let v = parse_review_verdict("");
        assert!(v.grounded);
    }

    #[test]
    fn review_verdict_missing_field_defaults_grounded() {
        let v = parse_review_verdict(r#"{"reason": "something"}"#);
        assert!(v.grounded);
    }

    #[test]
    fn review_verdict_false_without_correction() {
        let v = parse_review_verdict(r#"{"grounded": false, "reason": "specific BTC price"}"#);
        assert!(!v.grounded);
        assert!(v.corrected_reply.is_none());
    }

    #[test]
    fn agent_review_prompt_is_valid() {
        assert!(AGENT_REVIEW_PROMPT.contains("grounding reviewer"));
        assert!(AGENT_REVIEW_PROMPT.contains("grounded"));
        assert!(AGENT_REVIEW_PROMPT.contains("AVAILABLE_TOOLS"));
        assert!(
            AGENT_REVIEW_PROMPT.contains("ACTION claims"),
            "review must detect action claims (completed operations without tool)"
        );
        assert!(
            AGENT_REVIEW_PROMPT.contains("DATA claims"),
            "review must detect data claims (real-time data without tool)"
        );
        assert!(
            AGENT_REVIEW_PROMPT.contains("corrected_reply"),
            "review must support corrected_reply for editing false claims"
        );
    }

    // ── ReviewResult serialization tests ────────────────────────────────

    #[test]
    fn review_result_serializes() {
        let r = ReviewResult {
            layer: ReviewLayer::FastPass,
            passed: true,
            tool_matched: None,
            similarity: None,
            reason: None,
            corrected_reply: None,
            latency_ms: 0,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"FastPass\""));
        assert!(json.contains("\"passed\":true"));
    }

    #[test]
    fn review_result_deserializes() {
        let json = r#"{"layer":"SemanticFlag","passed":false,"tool_matched":"schedule_intent","similarity":0.72,"reason":"claims scheduling","corrected_reply":null,"latency_ms":150}"#;
        let r: ReviewResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.layer, ReviewLayer::SemanticFlag);
        assert!(!r.passed);
        assert_eq!(r.tool_matched.as_deref(), Some("schedule_intent"));
        assert!((r.similarity.unwrap() - 0.72).abs() < 0.001);
    }

    #[test]
    fn review_layer_as_str() {
        assert_eq!(ReviewLayer::FastPass.as_str(), "fast_pass");
        assert_eq!(
            ReviewLayer::NoCompletionSignal.as_str(),
            "no_completion_signal"
        );
        assert_eq!(ReviewLayer::SemanticPass.as_str(), "semantic_pass");
        assert_eq!(ReviewLayer::SemanticFlag.as_str(), "semantic_flag");
        assert_eq!(ReviewLayer::LlmReview.as_str(), "llm_review");
    }

    #[test]
    fn ledger_receipt_precheck_only_targets_external_action_claims() {
        assert!(reply_may_need_ledger_receipt_check(
            "I've sent the email with the latest update."
        ));
        assert!(reply_may_need_ledger_receipt_check(
            "I sent an email with the latest update."
        ));
        assert!(reply_may_need_ledger_receipt_check("日程已创建。"));
        assert!(reply_may_need_ledger_receipt_check("已经创建日程。"));
        assert!(!reply_may_need_ledger_receipt_check(
            "I found three useful options in the notes."
        ));
        assert!(!reply_may_need_ledger_receipt_check(
            "I can send the email after you approve it."
        ));
    }

    #[test]
    fn review_precheck_fast_passes_questions_without_semantic_work() {
        let result = run_review_pipeline_precheck("Hello! How can I help you?", None)
            .expect("question should finish in cheap precheck");

        assert_eq!(result.layer, ReviewLayer::FastPass);
        assert!(result.passed);
    }

    #[test]
    fn review_precheck_passes_non_action_replies_without_semantic_work() {
        let result =
            run_review_pipeline_precheck("I am here and ready to help with the next step.", None)
                .expect("non-action reply should finish in cheap precheck");

        assert_eq!(result.layer, ReviewLayer::NoCompletionSignal);
        assert!(result.passed);
    }

    #[test]
    fn review_precheck_defers_action_claims_to_semantic_review() {
        assert!(
            run_review_pipeline_precheck("I completed the Hacker News check for you.", None)
                .is_none(),
            "action claims without ledger-specific evidence need semantic review"
        );
    }

    #[test]
    fn ledger_receipt_check_passes_when_optional_evidence_unavailable() {
        assert!(
            missing_ledger_receipt_for_completion_claim("I've sent the email.", None).is_none()
        );
    }

    #[test]
    fn ledger_receipt_check_flags_missing_receipt_for_claimed_kind() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "email_send".to_string(),
            status: "executing".to_string(),
            has_receipt: false,
            user_visible_summary: Some("Sending email".to_string()),
        }];

        let flagged =
            missing_ledger_receipt_for_completion_claim("I've sent the email.", Some(&evidence))
                .expect("email completion claim without receipt should be flagged");

        assert_eq!(flagged.0, "email_send");
        assert!(
            flagged
                .1
                .contains("no recent executed Action Ledger receipt")
        );
    }

    #[test]
    fn ledger_receipt_check_accepts_matching_executed_receipt() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "email_send".to_string(),
            status: "executed".to_string(),
            has_receipt: true,
            user_visible_summary: Some("Email sent".to_string()),
        }];

        assert!(
            missing_ledger_receipt_for_completion_claim("I've sent the email.", Some(&evidence))
                .is_none()
        );
    }

    #[test]
    fn ledger_receipt_check_requires_each_claimed_action_kind() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "email_send".to_string(),
            status: "executed".to_string(),
            has_receipt: true,
            user_visible_summary: Some("Email sent".to_string()),
        }];

        let flagged = missing_ledger_receipt_for_completion_claim(
            "I've sent the email and created the calendar event.",
            Some(&evidence),
        )
        .expect("calendar claim without receipt should still be flagged");

        assert_eq!(flagged.0, "calendar_create");
    }

    #[test]
    fn ledger_receipt_check_requires_each_bilingual_claimed_action_kind() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "email_send".to_string(),
            status: "executed".to_string(),
            has_receipt: true,
            user_visible_summary: Some("Email sent".to_string()),
        }];

        let flagged = missing_ledger_receipt_for_completion_claim(
            "邮件已发送，并且日程已创建。",
            Some(&evidence),
        )
        .expect("Chinese calendar claim without receipt should still be flagged");

        assert_eq!(flagged.0, "calendar_create");
    }

    #[test]
    fn ledger_receipt_check_rejects_failed_rows_even_with_receipt_metadata() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "email_send".to_string(),
            status: "failed".to_string(),
            has_receipt: true,
            user_visible_summary: Some("Email send failed".to_string()),
        }];

        let flagged =
            missing_ledger_receipt_for_completion_claim("I've sent the email.", Some(&evidence))
                .expect("failed ledger row must not satisfy a completion claim");

        assert_eq!(flagged.0, "email_send");
    }

    #[test]
    fn ledger_receipt_check_rejects_stale_or_unrelated_evidence_for_new_claim() {
        let evidence = [LedgerReceiptEvidence {
            action_kind: "calendar_create".to_string(),
            status: "executed".to_string(),
            has_receipt: true,
            user_visible_summary: Some("Old calendar event created".to_string()),
        }];

        let flagged =
            missing_ledger_receipt_for_completion_claim("I've sent the email.", Some(&evidence))
                .expect("unrelated recent evidence must not satisfy a new email claim");

        assert_eq!(flagged.0, "email_send");
    }

    // ── Notification evaluator tests ─────────────────────────────────────

    #[test]
    fn evaluate_notifications_prompt_is_valid() {
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("TOOL_RESULTS"),
            "Prompt must reference TOOL_RESULTS"
        );
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("NOTIFICATIONS"),
            "Prompt must reference NOTIFICATIONS"
        );
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("Grounded"),
            "Prompt must check grounding"
        );
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("Standalone clarity"),
            "Prompt must check standalone clarity"
        );
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("\"pass\""),
            "Prompt must specify pass/fail JSON format"
        );
        assert!(
            CALLBACK_NOTIFICATION_EVALUATOR_PROMPT.contains("\"corrected\""),
            "Prompt must specify corrected field"
        );
    }

    #[test]
    fn review_guard_routing_blocks_explicit_codex_background_by_default() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "llm_provider",
                serde_json::Value::String("codex_oauth".to_string()),
            )
            .unwrap();
        let llm_client = crate::llm_client::create_llm_client("fake", "http://localhost");

        let err = route_review_guard_model(&llm_client, &settings, "test-model", "agent_review")
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("codex_enabled=false")
                || format!("{err:#}").contains("not allowed for task class ReviewGuard"),
            "unexpected routing error: {err:#}"
        );
    }

    #[test]
    fn parse_notification_verdict_pass() {
        let v = parse_notification_verdict(r#"{"pass": true}"#);
        assert!(v.pass);
        assert!(v.corrected.is_empty());
    }

    #[test]
    fn parse_notification_verdict_fail_with_corrections() {
        let v = parse_notification_verdict(
            r#"{"pass": false, "corrected": ["Fixed message 1", "Fixed message 2"]}"#,
        );
        assert!(!v.pass);
        assert_eq!(v.corrected.len(), 2);
        assert_eq!(v.corrected[0], "Fixed message 1");
        assert_eq!(v.corrected[1], "Fixed message 2");
    }

    #[test]
    fn parse_notification_verdict_malformed_defaults_pass() {
        let v = parse_notification_verdict("not json at all");
        assert!(v.pass, "Malformed input should default to pass (fail-open)");
        assert!(v.corrected.is_empty());
    }

    #[test]
    fn parse_notification_verdict_empty_defaults_pass() {
        let v = parse_notification_verdict("");
        assert!(v.pass, "Empty input should default to pass (fail-open)");
    }

    #[test]
    fn parse_notification_verdict_missing_pass_defaults_true() {
        let v = parse_notification_verdict(r#"{"corrected": ["msg"]}"#);
        assert!(
            v.pass,
            "Missing pass field should default to true (fail-open)"
        );
    }

    #[test]
    fn parse_notification_verdict_fail_no_corrections() {
        let v = parse_notification_verdict(r#"{"pass": false}"#);
        assert!(!v.pass);
        assert!(
            v.corrected.is_empty(),
            "Missing corrected should be empty vec"
        );
    }

    #[test]
    fn parse_notification_verdict_empty_string_removal() {
        // When evaluator returns "" for a notification, the filter should drop it
        let v =
            parse_notification_verdict(r#"{"pass": false, "corrected": ["Fixed message", ""]}"#);
        assert!(!v.pass);
        assert_eq!(v.corrected.len(), 2);
        assert_eq!(v.corrected[1], "");

        // Simulate the filter logic from evaluate_notifications
        let originals = [
            NotifyUserRequest {
                message: "BTC at $95,000".to_string(),
                pin: false,
                priority: crate::orchestrator::NotifyPriority::Important,
                source_tools: vec!["crypto_tracker".to_string()],
            },
            NotifyUserRequest {
                message: "无动态".to_string(),
                pin: false,
                priority: crate::orchestrator::NotifyPriority::Important,
                source_tools: vec!["web_search".to_string()],
            },
        ];
        let filtered: Vec<NotifyUserRequest> = originals
            .iter()
            .zip(v.corrected)
            .filter(|(_, new_msg)| !new_msg.trim().is_empty())
            .map(|(orig, new_msg)| NotifyUserRequest {
                message: new_msg,
                pin: orig.pin,
                priority: orig.priority,
                source_tools: orig.source_tools.clone(),
            })
            .collect();
        assert_eq!(filtered.len(), 1, "Empty corrected should be filtered out");
        assert_eq!(filtered[0].message, "Fixed message");
    }

    // ── ReviewStats default tests ───────────────────────────────────────

    #[test]
    fn review_stats_default() {
        let s = ReviewStats::default();
        assert_eq!(s.total, 0);
        assert_eq!(s.fast_pass, 0);
        assert_eq!(s.flagged, 0);
        assert_eq!(s.avg_latency_ms, 0.0);
    }

    // ── Pipeline integration (Layer 1 short-circuit) ────────────────────

    #[test]
    fn fast_pass_covers_edge_cases() {
        // Fullwidth question mark
        assert!(is_fast_pass("这是什么\u{FF1F}"));
        // Empty
        assert!(is_fast_pass(""));
        // Under 20 chars
        assert!(is_fast_pass("OK, got it."));
        // Over 20 chars without question mark — not fast-pass
        assert!(!is_fast_pass("I have completed the task for you now."));
    }

    #[test]
    fn completion_signal_edge_cases() {
        // "set up" as two words
        assert!(has_completion_signal("I've set up your account."));
        // Case insensitive
        assert!(has_completion_signal("TASK COMPLETED SUCCESSFULLY"));
        // No signal — future tense
        assert!(!has_completion_signal("I will help you with that."));
        assert!(!has_completion_signal("I can look into it."));
    }

    // ── Grounding check tests ─────────────────────────────────────────────

    #[test]
    fn grounding_fully_grounded() {
        let report = check_grounding("BTC price is $65,000", &["Current BTC: 65000 USD".into()]);
        assert_eq!(report.grounding_ratio, 1.0);
        assert!(report.ungrounded_claims.is_empty());
    }

    #[test]
    fn grounding_fully_ungrounded() {
        let report = check_grounding("BTC hit $70,000 today", &["ETH price: 3500 USD".into()]);
        assert!(report.grounding_ratio < 1.0);
        assert!(!report.ungrounded_claims.is_empty());
    }

    #[test]
    fn grounding_no_claims_empty_corpus_penalised() {
        // No claims + empty corpus → low confidence (LLM fabricated narrative)
        let report = check_grounding("No updates to report", &[]);
        assert!(
            report.grounding_ratio < 0.5,
            "empty corpus should be penalised"
        );
        assert_eq!(report.total_claims, 0);
    }

    #[test]
    fn grounding_no_claims_with_real_corpus_is_trivially_grounded() {
        // No claims but corpus has real data → fine (general summary)
        let report = check_grounding(
            "No updates to report",
            &["BTC is at 95000 with volume up 12%".to_string()],
        );
        assert_eq!(report.grounding_ratio, 1.0);
        assert_eq!(report.total_claims, 0);
    }

    #[test]
    fn grounding_no_claims_placeholder_corpus_penalised() {
        // No claims + placeholder corpus → penalised
        let report = check_grounding(
            "The search found nothing relevant",
            &["(no results)".to_string()],
        );
        assert!(
            report.grounding_ratio < 0.5,
            "placeholder corpus should be penalised"
        );
    }

    #[test]
    fn grounding_url_check() {
        let report = check_grounding(
            "Check https://example.com/news for details",
            &["Found article at https://example.com/news".into()],
        );
        assert_eq!(report.grounding_ratio, 1.0);
    }

    #[test]
    fn grounding_email_check() {
        let report = check_grounding(
            "New email from alice@example.com",
            &["From: alice@example.com Subject: Hello".into()],
        );
        assert_eq!(report.grounding_ratio, 1.0);
    }

    #[test]
    fn grounding_number_normalization() {
        // "65,000" in notification should match "65000" in tool result
        let report = check_grounding("Price: $65,000", &["price=65000".into()]);
        assert_eq!(report.grounding_ratio, 1.0);
    }

    #[test]
    fn grounding_mixed_claims() {
        let report = check_grounding(
            "BTC is $65,000, ETH is $4,500, SOL is $200",
            &["BTC: 65000, ETH: 4500".into()],
        );
        // 65000 and 4500 grounded, 200 ungrounded
        assert!(report.grounding_ratio > 0.5);
        assert!(report.grounding_ratio < 1.0);
        assert_eq!(report.grounded_claims, 2);
    }

    #[test]
    fn grounding_quoted_string() {
        let report = check_grounding(
            "Subject: \"Project Alpha Update\"",
            &["Email subject: Project Alpha Update".into()],
        );
        assert_eq!(report.grounding_ratio, 1.0);
    }

    #[test]
    fn grounding_chinese_with_numbers() {
        let report = check_grounding(
            "服务器CPU使用率达到95%",
            &["server_monitor: CPU 95%, Memory 60%".into()],
        );
        assert_eq!(report.grounding_ratio, 1.0);
    }

    #[test]
    fn grounding_single_digit_skipped() {
        // Single digits should be skipped (too many false positives)
        let report = check_grounding("Found 3 new items", &["unrelated tool output".into()]);
        // "3" is single digit → skipped, no claims extracted
        assert_eq!(report.total_claims, 0);
        assert_eq!(report.grounding_ratio, 1.0);
    }

    // ── Source-attributed grounding tests ────────────────────────────────

    #[test]
    fn grounding_with_source_filters_to_matching_results() {
        let results = vec![
            "[crypto_tracker] BTC: $95,000".to_string(),
            "[calendar_reader] 3 events today".to_string(),
        ];
        // Source = crypto_tracker → only checks against first result
        let report =
            check_grounding_with_sources("BTC at $95,000", &results, &["crypto_tracker".into()]);
        assert!(
            report.grounding_ratio > 0.9,
            "Should be grounded: ratio={}",
            report.grounding_ratio
        );
    }

    #[test]
    fn grounding_with_wrong_source_has_lower_score() {
        let results = vec![
            "[crypto_tracker] BTC: $95,000".to_string(),
            "[calendar_reader] 3 events today".to_string(),
        ];
        // Declare calendar_reader as source but claim BTC price → not in filtered results
        let report =
            check_grounding_with_sources("BTC at $95,000", &results, &["calendar_reader".into()]);
        // $95,000 is not in calendar_reader results → ungrounded (but gets +0.1 bonus)
        assert!(
            report.grounding_ratio < 0.5,
            "Wrong source should have low grounding: ratio={}",
            report.grounding_ratio
        );
    }

    #[test]
    fn grounding_sourced_gets_bonus() {
        // Use a mixed-grounding scenario where base ratio < 1.0
        // so the +0.1 bonus is visible (doesn't cap at 1.0)
        let results = vec!["BTC: $95,000".to_string()];
        let sourced = check_grounding_with_sources(
            "BTC at $95,000 and ETH at $3,500",
            &results,
            &["crypto_tracker".into()],
        );
        let unsourced =
            check_grounding_with_sources("BTC at $95,000 and ETH at $3,500", &results, &[]);
        // Both have 1/2 claims grounded (95000 found, 3500 not found)
        // Sourced gets +0.1 bonus on top of 0.5 → 0.6
        // Unsourced stays at 0.5
        assert!(
            sourced.grounding_ratio > unsourced.grounding_ratio,
            "Sourced ({}) should be higher than unsourced ({})",
            sourced.grounding_ratio,
            unsourced.grounding_ratio
        );
    }

    #[test]
    fn grounding_empty_source_falls_back() {
        let results = vec!["Data: 12345".to_string()];
        let report = check_grounding_with_sources("Value is 12345", &results, &[]);
        // Should use original check_grounding (no bonus)
        assert_eq!(report.grounding_ratio, 1.0);
        assert_eq!(report.total_claims, 1);
    }

    // ── Anti-hallucination bypass-vector regression tests ────────────────

    #[test]
    fn negation_blocks_completion_signal_en() {
        assert!(!has_completion_signal("I have not completed the task."));
        assert!(!has_completion_signal("The email hasn't been sent."));
        assert!(!has_completion_signal("Account wasn't deleted."));
        assert!(!has_completion_signal("Could not be uploaded."));
    }

    #[test]
    fn negation_blocks_completion_signal_zh() {
        assert!(!has_completion_signal("任务还没完成"));
        assert!(!has_completion_signal("邮件没发送"));
        assert!(!has_completion_signal("账号未删除"));
    }

    #[test]
    fn positive_completion_signal_still_detected_after_negation() {
        // First clause negated, second clause is a real claim — must still flag.
        assert!(has_completion_signal(
            "Task wasn't deleted, but the file was uploaded successfully."
        ));
        assert!(has_completion_signal("先没成功，后来已发送"));
    }

    #[test]
    fn question_with_completion_does_not_fastpass() {
        let reply = "I've sent the email and updated the document. Anything else?";
        assert!(
            !is_fast_pass(reply),
            "question containing completion claim must NOT fast-pass"
        );
    }

    #[test]
    fn bare_question_still_fastpasses() {
        // Short bare questions still fast-pass via length<20 short-circuit.
        assert!(is_fast_pass("What now?"));
        assert!(is_fast_pass("你想做什么？"));
        // Long bare question (no completion markers) also fast-passes.
        assert!(is_fast_pass(
            "Could you please clarify which item I should work on first?"
        ));
    }

    #[test]
    fn new_completion_markers_detected() {
        assert!(has_completion_signal(
            "I deployed the service to production."
        ));
        assert!(has_completion_signal("File uploaded successfully."));
        assert!(has_completion_signal("Subscription cancelled."));
        assert!(has_completion_signal("已部署到生产环境"));
        assert!(has_completion_signal("文件上传了"));
        assert!(has_completion_signal("订阅取消了"));
    }
}
