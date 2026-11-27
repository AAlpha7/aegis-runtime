//! Tool handlers for the subcall primitive: `spawn_subcall`, `recall_subcall`.
//!
//! See `docs/subcall-primitive-design.md` §5-§6 and
//! `docs/subcall-primitive-implementation-plan.md` Task 2.3.
//!
//! `final_answer` is intentionally **not** dispatched here — it is intercepted
//! inside the subcall's own ReAct loop (`run_autonomous_react_loop`) and never
//! reaches the parent's tool-dispatch chain.

use anyhow::Result;
use async_openai::types::chat::ChatCompletionMessageToolCall;
use serde_json::{Value, json};

use crate::orchestrator::{Orchestrator, ToolContext};
use crate::subcall::{
    self, DEFAULT_MAX_ROUNDS, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOOL_CALLS, MAX_SUBCALLS_PER_SESSION,
    SubcallId, SubcallOrigin, SubcallSpec,
};

impl Orchestrator {
    /// Dispatch `spawn_subcall` / `recall_subcall` tool calls.
    ///
    /// Returns `Ok(Some(true))` when a tool was dispatched, `Ok(None)` when the
    /// name is not in this handler's domain (so the caller can fall through to
    /// the next `handle_tool_*`).
    pub(crate) async fn handle_tool_subcall(
        &self,
        tool_call: &ChatCompletionMessageToolCall,
        tool_name: &str,
        tool_args_str: &str,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Option<bool>> {
        tool_dispatch!(self, tool_call, tool_name, tool_args_str, ctx,
            "spawn_subcall" => execute_spawn_subcall,
            "recall_subcall" => execute_recall_subcall,
            // `final_answer` is handled inside the subcall's own loop, not
            // the parent's. Leaving this comment as a tripwire for future
            // maintainers tempted to add it here.
        );
        Ok(None)
    }

    async fn execute_spawn_subcall(&self, args: Value, ctx: &mut ToolContext<'_>) -> String {
        // 1. Nested-subcall block: parents can spawn; a running subcall cannot
        //    spawn another subcall.
        if ctx.caller_context_is_subcall() {
            return json!({
                "status": "nested_subcall_attempted",
                "error": "spawn_subcall cannot be called from inside another subcall",
                "error_kind": "nested_subcall_attempted",
                "retryable": false,
                "hint": "Complete the current subcall's final_answer; the parent can spawn the next one."
            })
            .to_string();
        }

        // 2. Parse required args.
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return invalid_input("task is required (non-empty string)"),
        };
        let allowed_tools = match args.get("allowed_tools") {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>(),
            _ => return invalid_input("allowed_tools is required (array of strings)"),
        };
        if allowed_tools.is_empty() {
            return invalid_input("allowed_tools must contain at least one tool name");
        }

        let output_schema = args
            .get("output_schema")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let context_brief = args
            .get("context_brief")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_rounds = args
            .get("max_rounds")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_ROUNDS);
        let max_tool_calls = args
            .get("max_tool_calls")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_TOOL_CALLS);
        let max_tokens = args
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_TOKENS);

        // 3. Session cap pre-check (fast path — `run_subcall` also re-checks,
        //    but failing here avoids the prompt-build / loop-allocation cost).
        let session_id = ctx.session_id();
        let registry = self.subcall_registry();
        let session_count = registry.session_count(&session_id);
        if session_count >= MAX_SUBCALLS_PER_SESSION {
            let existing_ids: Vec<String> = registry
                .session_ids(&session_id)
                .into_iter()
                .map(|i| i.as_str().to_string())
                .collect();
            return json!({
                "status": "session_cap_reached",
                "error_kind": "session_cap_reached",
                "retryable": false,
                "status_detail": {
                    "used": session_count,
                    "cap": MAX_SUBCALLS_PER_SESSION,
                    "existing_subcall_ids": existing_ids,
                },
                "error": format!(
                    "This session has reached its {MAX_SUBCALLS_PER_SESSION}-subcall limit."
                ),
                "hint": "Complete remaining work directly, or use recall_subcall on a prior id."
            })
            .to_string();
        }

        // 4. Construct spec and run the subcall.
        let spec = SubcallSpec {
            task,
            context_brief,
            allowed_tools,
            output_schema,
            max_rounds,
            max_tool_calls,
            max_tokens,
            parent_session_id: session_id,
            origin: SubcallOrigin::LlmDirected,
        };

        let (parent_remaining, parent_cap) = ctx.parent_token_budget();
        // `subcall_registry()` returns `&Arc<SubcallRegistry>`; `run_subcall`
        // wants `&SubcallRegistry` — auto-deref handles it.
        //
        // Note on `Box::pin`: this handler is reachable from
        // `run_autonomous_react_loop` (parent's ReAct loop) and calls
        // `run_subcall`, which in turn calls `run_autonomous_react_loop`
        // (subcall's ReAct loop). Rust's async state machine can't size an
        // infinitely recursive future without indirection, so we box the
        // inner call. The ctx's `caller_context_is_subcall()` guard above
        // bounds the actual nesting depth to 1.
        let fut = Box::pin(subcall::run_subcall(
            spec,
            self,
            self.memory(),
            registry,
            parent_remaining,
            parent_cap,
        ));
        match fut.await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
                json!({
                    "status": "failed",
                    "error_kind": "serialize_result",
                    "retryable": false,
                    "error": e.to_string(),
                })
                .to_string()
            }),
            Err(e) => json!({
                "status": "failed",
                "error": e.to_string(),
                "error_kind": "runner_error",
                "retryable": false,
            })
            .to_string(),
        }
    }

    async fn execute_recall_subcall(&self, args: Value, _ctx: &mut ToolContext<'_>) -> String {
        let subcall_id_str = match args.get("subcall_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return invalid_input("subcall_id is required (non-empty string)"),
        };
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let subcall_id = SubcallId::from_string(subcall_id_str);
        let registry = self.subcall_registry();
        match registry.recall(&subcall_id, &query, self.memory()).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
                json!({
                    "status": "failed",
                    "error_kind": "serialize_result",
                    "retryable": false,
                    "error": e.to_string(),
                })
                .to_string()
            }),
            Err(e) => {
                // Distinguish the "not found" error raised by `recall()` (spec
                // §7.2) from generic failures so the LLM can decide whether to
                // retry with a different id.
                let msg = e.to_string();
                let (error_kind, retryable) = if msg.contains("subcall_id not found") {
                    // Genuine miss — retrying won't help.
                    ("not_found", false)
                } else {
                    // Transient SQLite / DB error (lock, I/O) — retryable per
                    // spec lines 2299-2307.
                    ("recall_error", true)
                };
                json!({
                    "status": "failed",
                    "error": msg,
                    "error_kind": error_kind,
                    "retryable": retryable,
                })
                .to_string()
            }
        }
    }
}

/// Shared `invalid_input` envelope for `spawn_subcall` / `recall_subcall`
/// argument validation failures. Matches the project-wide tool-error shape
/// (`status`/`error`/`error_kind`/`retryable`/`hint`).
fn invalid_input(msg: &str) -> String {
    json!({
        "status": "error",
        "error": msg,
        "error_kind": "invalid_input",
        "retryable": false,
        "hint": "Check the spawn_subcall / recall_subcall tool schema."
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryHandle;
    use crate::settings::RuntimeSettings;
    use crate::test_tool_helpers::{CtxState, make_tool_call, test_orchestrator};
    use crate::tool_gate::CallerContext;
    use std::sync::Arc;

    fn build_orchestrator() -> Orchestrator {
        let memory = MemoryHandle::spawn_in_memory().expect("spawn memory");
        let settings = Arc::new(RuntimeSettings::new());
        test_orchestrator(memory, settings)
    }

    #[tokio::test]
    async fn handle_tool_subcall_returns_none_for_unrelated_tool() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let tc = make_tool_call("some_other_tool", "{}");
        let out = orch
            .handle_tool_subcall(&tc, "some_other_tool", "{}", &mut ctx)
            .await
            .expect("dispatch must not error on unknown names");
        assert!(
            out.is_none(),
            "handler must fall through for unrelated tools"
        );
    }

    #[tokio::test]
    async fn execute_spawn_subcall_rejects_nested_subcall() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx_with_caller(42, CallerContext::Subcall);
        let args = serde_json::json!({
            "task": "do nothing",
            "allowed_tools": ["noop"],
        });
        let out = orch.execute_spawn_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "nested_subcall_attempted");
        assert_eq!(v["error_kind"], "nested_subcall_attempted");
        assert_eq!(v["retryable"], false);
    }

    #[tokio::test]
    async fn execute_spawn_subcall_rejects_missing_task() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let args = serde_json::json!({});
        let out = orch.execute_spawn_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error_kind"], "invalid_input");
        assert!(
            v["error"].as_str().unwrap_or("").contains("task"),
            "error should mention the `task` field, got: {v}"
        );
    }

    #[tokio::test]
    async fn execute_spawn_subcall_rejects_missing_allowed_tools() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let args = serde_json::json!({"task": "x"});
        let out = orch.execute_spawn_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error_kind"], "invalid_input");
        assert!(
            v["error"].as_str().unwrap_or("").contains("allowed_tools"),
            "error should mention allowed_tools, got: {v}"
        );
    }

    #[tokio::test]
    async fn execute_recall_subcall_rejects_missing_subcall_id() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let args = serde_json::json!({});
        let out = orch.execute_recall_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error_kind"], "invalid_input");
    }

    #[tokio::test]
    async fn execute_recall_subcall_returns_not_found_for_unknown_id() {
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let args = serde_json::json!({
            "subcall_id": "sc_does_not_exist",
            "query": "anything",
        });
        let out = orch.execute_recall_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["error_kind"], "not_found");
    }

    #[test]
    fn tool_context_session_id_includes_user_and_chat() {
        let mut state = CtxState::new();
        let ctx = state.ctx_with_chat_id(9);
        assert_eq!(ctx.session_id(), "user:did:test:user:chat:9");
    }

    #[tokio::test]
    async fn execute_spawn_subcall_rejects_empty_allowed_tools() {
        // Guard against the LLM supplying an empty array — design §5
        // requires at least one tool (otherwise the subcall's loop has
        // no surface to act on and will trivially ClampHang).
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx();
        let args = serde_json::json!({"task": "x", "allowed_tools": []});
        let out = orch.execute_spawn_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error_kind"], "invalid_input");
        assert!(
            v["error"]
                .as_str()
                .unwrap_or("")
                .contains("allowed_tools must contain"),
            "error should explain the empty-array case, got: {v}"
        );
    }

    #[tokio::test]
    async fn execute_spawn_subcall_returns_session_cap_reached_when_at_limit() {
        use crate::subcall::{
            MAX_SUBCALLS_PER_SESSION, SubcallId, SubcallOrigin, SubcallResult, SubcallSpec,
            SubcallStatus, SubcallTrace,
        };
        use chrono::Utc;

        // Pre-populate the registry with MAX_SUBCALLS_PER_SESSION traces
        // under the session id the ctx will report, then try to spawn one
        // more. Deterministic, no LLM required.
        let orch = build_orchestrator();
        let mut state = CtxState::new();
        let mut ctx = state.ctx_with_chat_id(7);
        let session_id = ctx.session_id();

        let registry = orch.subcall_registry();
        for _ in 0..MAX_SUBCALLS_PER_SESSION {
            let id = SubcallId::new();
            registry.insert(SubcallTrace {
                id: id.clone(),
                spec: SubcallSpec {
                    task: "filler".to_string(),
                    context_brief: None,
                    allowed_tools: vec!["noop".to_string()],
                    output_schema: serde_json::json!({}),
                    max_rounds: 1,
                    max_tool_calls: 1,
                    max_tokens: 100,
                    parent_session_id: session_id.clone(),
                    origin: SubcallOrigin::LlmDirected,
                },
                result: SubcallResult {
                    id,
                    structured_output: serde_json::json!({}),
                    summary: String::new(),
                    tool_calls_made: vec![],
                    tokens_used: 0,
                    rounds_used: 0,
                    status: SubcallStatus::Ok,
                    clamped_budgets: vec![],
                    last_assistant_text: None,
                },
                messages: vec![],
                tool_outputs: vec![],
                created_at: Utc::now(),
            });
        }

        let args = serde_json::json!({
            "task": "one more",
            "allowed_tools": ["noop"],
        });
        let out = orch.execute_spawn_subcall(args, &mut ctx).await;
        let v: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["status"], "session_cap_reached");
        assert_eq!(v["error_kind"], "session_cap_reached");
        assert_eq!(v["retryable"], false);
        assert_eq!(v["status_detail"]["used"], MAX_SUBCALLS_PER_SESSION);
        assert_eq!(v["status_detail"]["cap"], MAX_SUBCALLS_PER_SESSION);
        assert_eq!(
            v["status_detail"]["existing_subcall_ids"]
                .as_array()
                .map(|a| a.len()),
            Some(MAX_SUBCALLS_PER_SESSION),
        );
    }
}
