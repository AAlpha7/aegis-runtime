//! Integration tests for the subcall runner (`aegis_runtime_core::subcall::run_subcall`).
//!
//! Each test drives the real `Orchestrator` against the shared mock
//! OpenAI-compatible server from `mock_helpers` (pre-seeded chat responses,
//! deterministic embeddings), runs a subcall end to end — spec in → bounded
//! ReAct loop → `final_answer` extraction → schema check → `subcall_log` /
//! `subcall_trace` rows → registry insert — and asserts on the typed
//! `SubcallResult` plus the persisted rows.
//!
//! The memory handle is backed by a temp SQLite file so the tests can read
//! `subcall_log.status` / `subcall_trace.messages_json` back with `rusqlite`
//! (the memory actor opens the DB in WAL mode, so a second connection sees
//! committed rows).
//!
//! Two behaviours of the runner shape these tests:
//!   - `validate_output_schema` is permissive by design: it only checks that
//!     every top-level key of `output_schema` is present in
//!     `structured_output`. Specs here therefore use `{"city": {}}` rather
//!     than JSON-Schema (`{"type": "object", "properties": ...}` would demand
//!     `type` / `properties` keys in the answer).
//!   - `SubcallStatus::ToolBlocked` is never produced by the runner today.
//!     A tool outside `allowed_tools` is blocked per call by
//!     `ToolGate::evaluate_autonomous` (Layer 2 allowlist), the block reason
//!     is fed back to the model as the tool result, and the loop continues.
//!     The allowlist tests below assert that observable contract instead.

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::orchestrator::Orchestrator;
use aegis_runtime_core::settings::RuntimeSettings;
use aegis_runtime_core::subcall::{
    SubcallOrigin, SubcallResult, SubcallSpec, SubcallStatus, run_subcall,
};
use mock_helpers::{MockServerState, spawn_mock_server, text_response, tool_call_response};

const PARENT_REMAINING: usize = 100_000;
const PARENT_CAP: usize = 128_000;

/// A subcall harness: mock LLM + orchestrator + file-backed memory.
struct Harness {
    state: Arc<MockServerState>,
    orch: Orchestrator,
    memory: MemoryHandle,
    db_path: std::path::PathBuf,
    _tmp: tempfile::TempDir,
    _server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(responses: Vec<String>) -> Self {
        let state = Arc::new(MockServerState::new(responses));
        let (addr, server) = spawn_mock_server(state.clone()).await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("subcall.db");
        let memory = MemoryHandle::spawn(&db_path).expect("spawn file-backed memory");
        let settings = Arc::new(RuntimeSettings::new());
        let orch = mock_helpers::build_orchestrator(addr, memory.clone(), settings);
        Self {
            state,
            orch,
            memory,
            db_path,
            _tmp: tmp,
            _server: server,
        }
    }

    async fn run(&self, spec: SubcallSpec) -> SubcallResult {
        run_subcall(
            spec,
            &self.orch,
            &self.memory,
            self.orch.subcall_registry(),
            PARENT_REMAINING,
            PARENT_CAP,
        )
        .await
        .expect("run_subcall must return a typed result, never Err")
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.db_path).expect("open test DB for inspection")
    }

    /// `(status, summary)` from the `subcall_log` row for `id`.
    fn log_row(&self, id: &str) -> (String, Option<String>) {
        self.db()
            .query_row(
                "SELECT status, summary FROM subcall_log WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or_else(|e| panic!("subcall_log row for {id} missing: {e}"))
    }

    /// `messages_json` from the `subcall_trace` row for `id`.
    fn trace_messages(&self, id: &str) -> String {
        self.db()
            .query_row(
                "SELECT messages_json FROM subcall_trace WHERE subcall_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("subcall_trace row for {id} missing: {e}"))
    }
}

fn spec(session: &str, allowed_tools: &[&str], max_rounds: usize) -> SubcallSpec {
    SubcallSpec {
        task: "What is the capital of France?".to_string(),
        context_brief: None,
        allowed_tools: allowed_tools.iter().map(|s| s.to_string()).collect(),
        // Permissive top-level-key schema (see module docs).
        output_schema: json!({"city": {}}),
        max_rounds,
        max_tool_calls: 5,
        max_tokens: 2_000,
        parent_session_id: session.to_string(),
        origin: SubcallOrigin::LlmDirected,
    }
}

fn final_answer(structured_output: serde_json::Value, summary: &str) -> String {
    tool_call_response(
        "call_final",
        "final_answer",
        json!({"structured_output": structured_output, "summary": summary}),
    )
}

// ── final_answer happy path ─────────────────────────────────────────────────

#[tokio::test]
async fn subcall_returns_ok_when_llm_calls_final_answer_with_valid_json() {
    let h = Harness::new(vec![final_answer(
        json!({"city": "Paris"}),
        "Capital of France",
    )])
    .await;

    let result = h.run(spec("sess_ok", &["recall_memory"], 3)).await;

    assert_eq!(result.status, SubcallStatus::Ok, "{result:?}");
    assert_eq!(result.structured_output["city"], "Paris");
    assert_eq!(result.summary, "Capital of France");
    assert_eq!(result.rounds_used, 1);
    assert_eq!(result.tool_calls_made, vec!["final_answer"]);
    assert!(
        result.tokens_used > 0,
        "tokens_used must come from the mock's reported usage, got {}",
        result.tokens_used
    );
    assert_eq!(
        result.last_assistant_text.as_deref(),
        Some("Capital of France")
    );
    assert!(
        result.clamped_budgets.is_empty(),
        "{:?}",
        result.clamped_budgets
    );
    assert_eq!(
        h.state.chat_calls(),
        1,
        "final_answer must short-circuit the loop"
    );

    // Persistence: one subcall_log row with status='ok', one subcall_trace row.
    let (status, summary) = h.log_row(result.id.as_str());
    assert_eq!(status, "ok");
    assert_eq!(summary.as_deref(), Some("Capital of France"));
    let (exists, trace_available, _, tool_outputs) = h
        .memory
        .subcall_fetch_trace(result.id.as_str())
        .await
        .expect("fetch trace");
    assert!(exists && trace_available, "trace row must be written");
    assert_eq!(
        tool_outputs.unwrap_or_default(),
        Vec::<(String, String)>::new(),
        "final_answer is a sentinel and never appears in tool_outputs"
    );
    assert!(
        h.trace_messages(result.id.as_str())
            .contains("final_answer"),
        "trace transcript must include the final_answer call"
    );
    // Registry: the trace is indexed under the parent session.
    assert_eq!(h.orch.subcall_registry().session_count("sess_ok"), 1);
}

/// Negative control for the happy path: the same harness with a
/// `final_answer` that omits the required top-level key must NOT be `Ok`.
/// Proves the schema check (not the mere presence of `final_answer`) is what
/// the previous test exercises.
#[tokio::test]
async fn subcall_reports_invalid_output_when_final_answer_misses_required_key() {
    let h = Harness::new(vec![final_answer(
        json!({"country": "France"}),
        "Forgot the city key",
    )])
    .await;

    let result = h.run(spec("sess_badkey", &["recall_memory"], 3)).await;

    match &result.status {
        SubcallStatus::InvalidOutput { schema_error } => assert!(
            schema_error.contains("missing required field: city"),
            "unexpected schema_error: {schema_error}"
        ),
        other => panic!("expected InvalidOutput, got {other:?}"),
    }
    // The partial output is still surfaced for the caller to inspect.
    assert_eq!(result.structured_output["country"], "France");
    let (status, _) = h.log_row(result.id.as_str());
    assert_eq!(status, "invalid_output");
}

// ── final_answer never called ───────────────────────────────────────────────

#[tokio::test]
async fn subcall_populates_last_assistant_text_when_final_answer_missing() {
    let prose = "I believe the capital is Paris, but I am answering in plain text.";
    let h = Harness::new(vec![text_response(prose)]).await;

    // max_rounds > 1 so the loop's early exit on a text reply is attributed
    // to the missing final_answer, not to the round cap (see next test).
    let result = h.run(spec("sess_prose", &["recall_memory"], 3)).await;

    match &result.status {
        SubcallStatus::InvalidOutput { schema_error } => assert!(
            schema_error.contains("final_answer was never called"),
            "unexpected schema_error: {schema_error}"
        ),
        other => panic!("expected InvalidOutput, got {other:?}"),
    }
    assert_eq!(result.last_assistant_text.as_deref(), Some(prose));
    assert_eq!(
        result.summary, prose,
        "summary falls back to the last assistant text when final_answer never ran"
    );
    assert!(result.tool_calls_made.is_empty());
    assert_eq!(result.structured_output, json!({}));

    // subcall_log has no dedicated last_assistant_text column; the prose is
    // persisted through `summary` (status='invalid_output').
    let (status, summary) = h.log_row(result.id.as_str());
    assert_eq!(status, "invalid_output");
    assert_eq!(summary.as_deref(), Some(prose));
}

/// Spec §7.2: when `final_answer` is missing AND the round cap was the
/// reason the loop ended, the runner reports `BudgetExceeded{max_rounds}`
/// rather than collapsing into `InvalidOutput`. The prose is still kept.
#[tokio::test]
async fn subcall_attributes_missing_final_answer_to_round_cap() {
    let prose = "Out of rounds before calling final_answer.";
    let h = Harness::new(vec![text_response(prose)]).await;

    let result = h.run(spec("sess_roundcap", &["recall_memory"], 1)).await;

    assert_eq!(
        result.status,
        SubcallStatus::BudgetExceeded {
            limit: "max_rounds".to_string(),
            used: 1,
            cap: 1,
        },
        "{result:?}"
    );
    assert_eq!(result.last_assistant_text.as_deref(), Some(prose));
    let (status, _) = h.log_row(result.id.as_str());
    assert_eq!(status, "budget_exceeded");
}

// ── allowlist enforcement ───────────────────────────────────────────────────

/// Script shared by the allowlist pair: round 1 calls `recall_memory`,
/// round 2 returns a valid `final_answer`.
fn recall_then_final_answer() -> Vec<String> {
    vec![
        tool_call_response("call_1", "recall_memory", json!({"query": "capital"})),
        final_answer(json!({"city": "Paris"}), "Capital of France"),
    ]
}

/// The model calls a tool that is NOT in `allowed_tools` (`server_monitor`
/// is allowed, `recall_memory` is not). The gate blocks the call, the block
/// reason is returned to the model as the tool result, nothing is executed
/// or traced for that tool, and the loop continues to `final_answer`.
#[tokio::test]
async fn subcall_blocks_tool_outside_allowlist_and_keeps_it_out_of_trace() {
    let h = Harness::new(recall_then_final_answer()).await;

    let result = h.run(spec("sess_blocked", &["server_monitor"], 3)).await;

    assert_eq!(result.status, SubcallStatus::Ok, "{result:?}");
    assert_eq!(
        result.tool_calls_made,
        vec!["final_answer"],
        "a blocked tool must not be reported as called"
    );
    assert_eq!(result.rounds_used, 2);
    assert_eq!(h.state.chat_calls(), 2);

    let (_, _, _, tool_outputs) = h
        .memory
        .subcall_fetch_trace(result.id.as_str())
        .await
        .expect("fetch trace");
    assert!(
        tool_outputs
            .unwrap_or_default()
            .iter()
            .all(|(name, _)| name != "recall_memory"),
        "blocked tool must leave no subcall_trace tool output"
    );
    let transcript = h.trace_messages(result.id.as_str());
    assert!(
        transcript.contains("not in the autonomous allowlist"),
        "block reason must be fed back to the model; transcript: {transcript}"
    );
    let (status, _) = h.log_row(result.id.as_str());
    assert_eq!(status, "ok");
}

/// Negative control for the allowlist test: the identical script with
/// `recall_memory` allow-listed executes the tool, reports it as called and
/// records its raw output in the trace — so the previous test's block is
/// attributable to the allowlist alone.
#[tokio::test]
async fn subcall_executes_allowlisted_tool_and_records_trace_output() {
    let h = Harness::new(recall_then_final_answer()).await;

    let result = h.run(spec("sess_allowed", &["recall_memory"], 3)).await;

    assert_eq!(result.status, SubcallStatus::Ok, "{result:?}");
    assert_eq!(
        result.tool_calls_made,
        vec!["recall_memory", "final_answer"]
    );

    let (_, _, _, tool_outputs) = h
        .memory
        .subcall_fetch_trace(result.id.as_str())
        .await
        .expect("fetch trace");
    let tool_outputs = tool_outputs.unwrap_or_default();
    assert!(
        tool_outputs
            .iter()
            .any(|(name, body)| name == "recall_memory" && !body.is_empty()),
        "executed tool must be traced with its raw body, got {tool_outputs:?}"
    );
    let transcript = h.trace_messages(result.id.as_str());
    assert!(
        !transcript.contains("not in the autonomous allowlist"),
        "allow-listed tool must not be blocked; transcript: {transcript}"
    );
}
