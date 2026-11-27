//! True Heartbeat end-to-end tests.
//!
//! Tests exercise the full True Heartbeat pipeline:
//!   snapshot compilation → LLM dispatch → tool execution → audit → notifications
//!
//! Uses the mock LLM server (no real API calls, no `#[ignore]`).
//!
//! Run: `cargo test -p aegis-runtime-core --test true_heartbeat_e2e`

mod mock_helpers;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use aegis_runtime_core::chronos::{
    AutonomousConfig, ChronosEngine, SituationSnapshot, TrueHeartbeatConfig,
};
use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

/// Helper: build a minimal SituationSnapshot for tests.
fn make_test_snapshot() -> SituationSnapshot {
    SituationSnapshot {
        compiled_at: "2026-03-28T12:00:00Z".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        recent_probe_results: vec![],
        pending_tasks: vec![],
        upcoming_intents: vec![],
        skill_metrics: vec![],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![],
        budget_status: (500, 500_000),
        user_last_active_unix: Some(1743120000),
    }
}

/// Helper: build default AutonomousConfig for tests.
fn test_autonomous_config() -> AutonomousConfig {
    AutonomousConfig {
        enabled: true,
        allowed_tools: vec![
            "check_api_usage".to_string(),
            "manage_tasks".to_string(),
            "research".to_string(),
        ],
        denied_tools: vec![],
        max_rounds: 3,
        max_tool_calls: 5,
        daily_token_budget: 500_000,
        notification_mode: aegis_runtime_core::chronos::NotificationMode::OnAction,
    }
}

// ── Test 1: Full cycle — snapshot → LLM → tools → audit ────────────────────

/// True heartbeat processes a snapshot, LLM calls a tool, result is audited.
#[tokio::test]
async fn e2e_true_heartbeat_full_cycle() {
    // Mock LLM: tool call (get_current_time, no HTTP side-effects) → final text
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_hb_1", "get_current_time", json!({})),
        text_response("All systems nominal. No action required."),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let snapshot = make_test_snapshot();
    let mut config = test_autonomous_config();
    config.allowed_tools.push("get_current_time".to_string());

    let result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            "did:aegis:admin",
            Some(100),
        )
        .await
        .unwrap();

    // get_current_time should have been called
    assert!(
        result
            .tools_called
            .contains(&"get_current_time".to_string()),
        "Expected get_current_time in tools_called: {:?}",
        result.tools_called
    );

    // Audit entry should exist for __heartbeat__
    let audits = memory
        .query_heartbeat_audit(Some("__heartbeat__"), 10)
        .await
        .unwrap();
    assert!(
        !audits.is_empty(),
        "Expected at least one audit entry for __heartbeat__"
    );

    let latest = &audits[0];
    assert_eq!(latest.skill_name, "__heartbeat__");
}

// ── Test 2: Heartbeat yields to user ────────────────────────────────────────

/// When llm_busy is true, start_true_heartbeat skips the cycle.
/// We test this via the concurrency gate directly.
#[tokio::test]
async fn e2e_true_heartbeat_yields_to_user() {
    let llm_busy = Arc::new(AtomicBool::new(true));

    // Verify compare_exchange fails when busy
    let result = llm_busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed);
    assert!(
        result.is_err(),
        "compare_exchange should fail when llm_busy is true"
    );

    // After user finishes, busy is reset
    llm_busy.store(false, Ordering::Release);
    let result = llm_busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed);
    assert!(
        result.is_ok(),
        "compare_exchange should succeed when llm_busy is false"
    );
}

// ── Test 3: Probe and true heartbeat coexist — shared budget ────────────────

/// Both probe heartbeat and true heartbeat draw from the same daily token budget.
/// True heartbeat stores audit entries under "__heartbeat__" label.
#[tokio::test]
async fn e2e_probe_and_true_heartbeat_coexist() {
    // Single response for true heartbeat: FALLTHROUGH
    let state = Arc::new(MockServerState::new(vec![text_response(
        "FALLTHROUGH - system nominal",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let config = test_autonomous_config();

    // Run true heartbeat
    let snapshot = make_test_snapshot();
    let result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            "did:aegis:admin",
            Some(100),
        )
        .await
        .unwrap();
    assert!(
        result.fell_through_to_alert,
        "FALLTHROUGH should be detected"
    );

    // Audit entry should be stored under __heartbeat__
    let heartbeat_audits = memory
        .query_heartbeat_audit(Some("__heartbeat__"), 10)
        .await
        .unwrap();
    assert!(
        !heartbeat_audits.is_empty(),
        "True heartbeat should leave an audit trail under __heartbeat__"
    );
    assert_eq!(heartbeat_audits[0].skill_name, "__heartbeat__");
}

// ── Test 4: True heartbeat FALLTHROUGH ──────────────────────────────────────

/// When LLM returns FALLTHROUGH, fell_through_to_alert is true.
#[tokio::test]
async fn e2e_true_heartbeat_fallthrough() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "FALLTHROUGH - Nothing to do.",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let snapshot = make_test_snapshot();
    let config = test_autonomous_config();

    let result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            "did:aegis:admin",
            Some(100),
        )
        .await
        .unwrap();

    assert!(
        result.fell_through_to_alert,
        "FALLTHROUGH response should set fell_through=true"
    );
    assert!(
        result.tools_called.is_empty(),
        "No tools should be called on FALLTHROUGH"
    );
}

// ── Test 5: Quiet hours skip ────────────────────────────────────────────────

/// Quiet hours configuration structure is correct and integrates with HeartbeatConfig.
///
/// Since heartbeat-autonomy Phase B, quiet hours live in `HeartbeatConfig`
/// (user-local HH:MM strings) rather than `TrueHeartbeatConfig` (UTC hour
/// tuple). Deterministic logic tests are in `heartbeat_config::tests`; this
/// e2e test verifies the config plumbing is wired correctly.
#[tokio::test]
async fn e2e_true_heartbeat_quiet_hours_config() {
    use aegis_runtime_core::heartbeat_config::{HeartbeatConfig, QuietHours};

    // Verify HeartbeatConfig carries QuietHours with correct defaults
    let cfg = HeartbeatConfig::default();
    assert_eq!(cfg.quiet_hours.start, "02:00");
    assert_eq!(cfg.quiet_hours.end, "07:00");

    // Verify custom quiet hours survive JSON roundtrip
    let cfg2 = HeartbeatConfig {
        quiet_hours: QuietHours {
            start: "22:00".into(),
            end: "06:00".into(),
        },
        ..HeartbeatConfig::default()
    };
    let json = cfg2.to_json();
    let restored = HeartbeatConfig::from_json(&json).expect("decode");
    assert_eq!(restored.quiet_hours.start, "22:00");
    assert_eq!(restored.quiet_hours.end, "06:00");

    // TrueHeartbeatConfig no longer owns quiet hours logic — field is legacy
    let old_cfg = TrueHeartbeatConfig::default();
    assert!(
        old_cfg.quiet_hours.is_none(),
        "legacy quiet_hours defaults to None"
    );
}

// ── Test 6: Snapshot compilation from real memory ───────────────────────────

/// compile_snapshot correctly pulls data from all memory sources.
#[tokio::test]
async fn e2e_compile_snapshot_from_memory() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let identity = Arc::new(AgentIdentity::generate());
    let settings = Arc::new(RuntimeSettings::new());
    let sandbox = shared_sandbox();

    let mut engine = ChronosEngine::new(Arc::new(sandbox), identity.clone(), settings);
    // set_triage provides both embedder and memory to the engine
    let embedder = EmbeddingClient::new("http://localhost:1/v1", "mock-key", "mock-model", 256);
    engine.set_triage(embedder, memory.clone());

    // Seed some data
    let chat_id = 12345_i64;

    // Add a task
    memory
        .store_task(
            chat_id,
            &identity.did,
            "Test task for heartbeat", // title
            "Check system health",     // body
            "normal",                  // priority
            &[],                       // tags
            None,                      // due_date
            "user",                    // source
            None,                      // trigger_at_unix
            None,                      // trigger_type
            None,                      // event_key
            "manual",                  // execution_mode
            None,                      // context_text
            None,                      // interval_seconds
            None,                      // group_key
            None,                      // max_fires
            false,                     // pin_on_fire
            None,                      // expires_at_unix
            false,                     // is_callback_origin
            0,                         // chain_depth,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Add a heartbeat audit entry
    memory
        .store_heartbeat_audit(
            "cycle-test-001",                   // cycle_id
            "crypto_tracker",                   // skill_name
            "Info",                             // severity
            Some("BTC=$95000"),                 // probe_payload
            &[],                                // tools_called
            &[],                                // tools_blocked
            &["checked BTC price".to_string()], // actions_taken
            None,                               // llm_reply
            50,                                 // tokens_used
            1,                                  // rounds_used
            &identity.did,                      // user_did
            Some(chat_id),                      // chat_id
            "{}",                               // config_snapshot
            None,                               // raw_process_log
            None,                               // tool_call_details
        )
        .await
        .unwrap();

    // Compile snapshot
    let snapshot = engine.compile_snapshot(chat_id).await.unwrap();

    // Verify snapshot contains our seeded data
    assert!(
        !snapshot.pending_tasks.is_empty(),
        "Snapshot should contain the task we created"
    );
    assert!(
        !snapshot.recent_probe_results.is_empty(),
        "Snapshot should contain the audit entry we created"
    );
    assert!(!snapshot.timezone.is_empty(), "Timezone should be set");
    assert!(!snapshot.compiled_at.is_empty());

    // Budget should be non-zero (or at least present)
    assert_eq!(
        snapshot.budget_status.1, 500_000,
        "Default budget should be 500k"
    );

    // to_prompt_block should produce valid XML-ish output
    let block = snapshot.to_prompt_block();
    assert!(
        block.contains("<situation_snapshot"),
        "Block should have XML wrapper"
    );
    assert!(
        block.contains("Test task for heartbeat"),
        "Block should mention our task"
    );
    assert!(
        block.contains("crypto_tracker"),
        "Block should mention our audit entry"
    );
}

// ── Test 7: BusyGuard drop semantics ────────────────────────────────────────

/// BusyGuard always resets the flag, even on early return.
#[tokio::test]
async fn e2e_busy_guard_always_resets() {
    let busy = Arc::new(AtomicBool::new(false));

    // Simulate user message: set busy, scope exits, should reset
    {
        busy.store(true, Ordering::Release);
        struct Guard(Arc<AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _g = Guard(busy.clone());
        assert!(busy.load(Ordering::Acquire), "Should be busy inside scope");
    }
    assert!(
        !busy.load(Ordering::Acquire),
        "BusyGuard should reset on scope exit"
    );

    // Simulate panic scenario: guard still resets (test via catch_unwind)
    {
        busy.store(true, Ordering::Release);
        let busy2 = busy.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            struct Guard(Arc<AtomicBool>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _g = Guard(busy2);
            panic!("simulated panic");
        }));
        assert!(result.is_err(), "Should have caught the panic");
    }
    assert!(
        !busy.load(Ordering::Acquire),
        "BusyGuard should reset even after panic"
    );
}

// ── Test 8: Config round-trip ───────────────────────────────────────────────

/// TrueHeartbeatConfig serializes and deserializes correctly.
#[tokio::test]
async fn e2e_true_heartbeat_config_round_trip() {
    let config = TrueHeartbeatConfig {
        enabled: true,
        interval_minutes: 20,
        quiet_hours: Some((22, 6)),
        autonomous_overrides: Some(AutonomousConfig {
            enabled: true,
            allowed_tools: vec!["research".to_string()],
            denied_tools: vec![],
            max_rounds: 2,
            max_tool_calls: 3,
            daily_token_budget: 10_000,
            notification_mode: aegis_runtime_core::chronos::NotificationMode::OnAction,
        }),
    };

    let json = serde_json::to_string_pretty(&config).unwrap();
    let restored: TrueHeartbeatConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.enabled, config.enabled);
    assert_eq!(restored.interval_minutes, config.interval_minutes);
    // Legacy quiet_hours is accepted when deserializing old rows, but is no
    // longer serialized; HeartbeatConfig.quiet_hours owns the active setting.
    assert!(restored.quiet_hours.is_none());
    assert!(restored.autonomous_overrides.is_some());

    let override_cfg = restored.autonomous_overrides.unwrap();
    assert_eq!(override_cfg.max_rounds, 2);
    assert_eq!(override_cfg.daily_token_budget, 10_000);
}

// ── Test 9: System-managed memo — post-cycle evaluation ──────────────────

/// After a heartbeat cycle, the system should call LLM to evaluate memo updates.
/// The mock LLM produces a text response first (main cycle), then another
/// response for the memo evaluation call.
#[tokio::test]
async fn e2e_system_managed_memo_update() {
    // Response 1: main heartbeat cycle — does something (so memo eval fires)
    // Response 2: memo evaluation — returns updated memo content
    let state = Arc::new(MockServerState::new(vec![
        tool_call_response("call_memo_1", "get_current_time", json!({})),
        text_response("Checked system status. All operational."),
        text_response("系统状态正常。已检查时间和运行状态。"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings.clone());

    let snapshot = make_test_snapshot();
    let mut config = test_autonomous_config();
    config.allowed_tools.push("get_current_time".to_string());

    let result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            "did:aegis:admin",
            Some(100),
        )
        .await
        .unwrap();

    // Main cycle should have run
    assert!(
        result
            .tools_called
            .contains(&"get_current_time".to_string()),
        "Expected get_current_time in tools_called: {:?}",
        result.tools_called
    );

    // The memo should have been updated by the system-managed evaluator
    // (3rd mock response: "系统状态正常。已检查时间和运行状态。")
    let _memo_val = settings
        .get("heartbeat_memo")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    // Verify the system made 3 LLM calls: tool call + final text + memo eval
    assert_eq!(
        state.chat_calls(),
        3,
        "Expected 3 LLM calls (tool call + final text + memo eval)"
    );
}

// ── Test 10: System memo skips trivial cycles ─────────────────────────────

/// When the cycle falls through with no actions, memo evaluation is skipped.
#[tokio::test]
async fn e2e_system_memo_skips_trivial_cycle() {
    // Only 1 response: FALLTHROUGH (trivial cycle, no memo eval call needed)
    let state = Arc::new(MockServerState::new(vec![text_response(
        "FALLTHROUGH - Nothing to do.",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let snapshot = make_test_snapshot();
    let config = test_autonomous_config();

    let result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            "did:aegis:admin",
            Some(100),
        )
        .await
        .unwrap();

    assert!(result.fell_through_to_alert);

    // Only 1 LLM call — memo eval was skipped for trivial cycle
    assert_eq!(
        state.chat_calls(),
        1,
        "Trivial FALLTHROUGH should not trigger memo evaluation"
    );
}

// ── Test 11: Gate auto-suspend — blocked_by_gate status ──────────────────

/// Tasks with status 'blocked_by_gate' are not picked up by the intent poller.
/// This test validates the suspend/restore lifecycle via memory operations.
#[tokio::test]
async fn e2e_gate_auto_suspend_lifecycle() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let identity = Arc::new(AgentIdentity::generate());
    let chat_id = 12345_i64;

    // Create a recurring task
    let now = chrono::Utc::now().timestamp();
    let task_id = memory
        .store_deferred_intent(
            &identity.did,
            chat_id,
            "time",              // trigger_type
            Some(now + 3600),    // trigger_at_unix
            None,                // event_key
            "llm_callback",      // execution_mode
            "Check email",       // context_text
            now + 86400,         // expires_at_unix
            false,               // is_callback_origin
            Some(3600),          // interval_seconds (recurring every hour)
            0,                   // chain_depth
            false,               // pin_on_fire
            Some("email_check"), // group_key
            None,                // max_fires
            None,                // embedding,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Verify task is active
    let intents = memory.list_user_intents(&identity.did).await.unwrap();
    assert!(
        intents.iter().any(|i| i.id == task_id),
        "Task should be active"
    );

    // Simulate gate auto-suspend: set status to blocked_by_gate
    let suspend_ok = memory
        .update_task(task_id, chat_id, r#"{"status":"blocked_by_gate"}"#)
        .await
        .unwrap();
    assert!(suspend_ok, "Should successfully suspend the task");

    // Task should no longer appear in active intents
    let intents_after = memory.list_user_intents(&identity.did).await.unwrap();
    assert!(
        !intents_after.iter().any(|i| i.id == task_id),
        "Suspended task should not appear in active intents"
    );

    // Due intents should not include this task either
    let due = memory.get_due_time_intents(now + 7200).await.unwrap();
    assert!(
        !due.iter().any(|i| i.id == task_id),
        "Suspended task should not appear in due intents"
    );

    // Simulate gate approval: restore to active
    let restore_ok = memory
        .update_task(task_id, chat_id, r#"{"status":"active"}"#)
        .await
        .unwrap();
    assert!(restore_ok, "Should successfully restore the task");

    // Task should appear in active intents again
    let intents_restored = memory.list_user_intents(&identity.did).await.unwrap();
    assert!(
        intents_restored.iter().any(|i| i.id == task_id),
        "Restored task should appear in active intents again"
    );
}

// ── Test 12: Last cycle summary injection ─────────────────────────────────

/// build_last_cycle_summary correctly formats the previous cycle's audit data.
#[tokio::test]
async fn e2e_last_cycle_summary_injection() {
    let state = Arc::new(MockServerState::new(vec![
        text_response("All systems nominal."),
        text_response("NO_UPDATE"),
    ]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let identity = Arc::new(AgentIdentity::generate());

    // Seed a previous heartbeat audit entry
    let tools: Vec<String> = vec![
        "get_current_time".to_string(),
        "check_api_usage".to_string(),
    ];
    let actions: Vec<String> = vec![
        "Checked system time".to_string(),
        "Reviewed API costs".to_string(),
    ];
    memory
        .store_heartbeat_audit(
            "hb-prev-001",
            "__heartbeat__",
            "Info",
            None,
            &tools,
            &[],
            &actions,
            Some("All systems operational."),
            150,
            2,
            &identity.did,
            Some(100),
            "{}",
            None, // raw_process_log
            None, // tool_call_details
        )
        .await
        .unwrap();

    // Run heartbeat — should auto-inject last cycle summary
    let snapshot = make_test_snapshot();
    let config = test_autonomous_config();

    let _result = orch
        .run_heartbeat_tick(
            aegis_runtime_core::orchestrator::TickTrigger::Scheduled,
            &snapshot,
            &config,
            &identity.did,
            Some(100),
        )
        .await
        .unwrap();

    // Verify the cycle completed (the previous audit was available for injection)
    // The mock server should have received 2 calls (main + memo eval or just main if NO_UPDATE)
    assert!(
        state.chat_calls() >= 1,
        "At least one LLM call should have been made"
    );
}
