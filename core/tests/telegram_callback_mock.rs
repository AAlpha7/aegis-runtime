//! Mock-based integration tests for Telegram callback / HITL approval flows.
//!
//! Tests the orchestrator's inline command interception ("approve sysop X",
//! "reject config Y", etc.) without requiring a real Telegram bot.
//!
//! Also tests credential detection functions (exposed via public API).

mod mock_helpers;

use std::sync::Arc;

use serde_json::json;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::settings::RuntimeSettings;

use mock_helpers::*;

// ── Sysop approval tests ────────────────────────────────────────────────────

/// approve sysop with non-existent approval_id returns "not found".
#[tokio::test]
async fn sysop_approve_not_found() {
    let state = Arc::new(MockServerState::new(vec![text_response("Not found.")]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "approve sysop sysop-nonexistent",
            Some(100),
        )
        .await
        .unwrap();

    // Should return early with a "not found" reply (no LLM call needed)
    assert!(
        result.reply.contains("not found")
            || result.reply.contains("Not found")
            || result.reply.contains("already processed"),
        "Unexpected reply: {}",
        result.reply
    );
    assert_eq!(result.rounds_used, 0);
}

/// reject sysop with non-existent approval_id returns "not found".
#[tokio::test]
async fn sysop_reject_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "reject sysop sysop-abc123", Some(100))
        .await
        .unwrap();

    assert!(
        result.reply.contains("not found")
            || result.reply.contains("Not found")
            || result.reply.contains("already processed"),
        "Unexpected reply: {}",
        result.reply
    );
    assert_eq!(result.rounds_used, 0);
}

/// approve sysop with stored approval state executes the commands.
#[tokio::test]
async fn sysop_approve_with_stored_state() {
    let state = Arc::new(MockServerState::new(vec![text_response(
        "Execution complete.",
    )]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Pre-store an approval state
    let approval_data = json!({
        "commands": ["echo hello"],
        "description": "Test sysop command",
        "tool_call_id": "call_1",
        "user_did": "did:test:user1",
        "chat_id": 100,
        "created_at": chrono::Utc::now().to_rfc3339()
    });
    memory
        .store_skill_state("hitl", "sysop-test123", &approval_data.to_string())
        .await
        .unwrap();

    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "approve sysop sysop-test123", Some(100))
        .await
        .unwrap();

    // Should have processed (not a "not found" reply)
    assert!(
        !result.reply.contains("not found"),
        "Should process the approval: {}",
        result.reply
    );
}

// ── Config approval tests ────────────────────────────────────────────────────

/// approve config with non-existent id returns appropriate message.
#[tokio::test]
async fn config_approve_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings.clone());

    let result = orch
        .process_message("did:test:user1", "approve config config-xyz", Some(100))
        .await
        .unwrap();

    // The not-found reply is localized via the i18n catalog (commit ab99a979),
    // so assert against the catalog message for the orchestrator's resolved
    // locale rather than a hard-coded English substring — keeps the test
    // meaningful and language-agnostic. Mirrors the hitl.rs unit test
    // `config_approval_not_found_uses_display_language`.
    let expected = aegis_runtime_core::i18n::tf(
        "hitl.config.not_found_or_processed",
        &settings.display_language(),
        "config-xyz",
    );
    assert!(
        result.reply.contains(&expected),
        "Unexpected reply: {} (expected catalog message: {})",
        result.reply,
        expected
    );
    assert_eq!(result.rounds_used, 0);
}

/// reject config with non-existent id returns appropriate message.
#[tokio::test]
async fn config_reject_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "reject config config-xyz", Some(100))
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 0);
}

// ── MCP approval tests ──────────────────────────────────────────────────────

/// approve mcp with stored state (valid MCP approval).
#[tokio::test]
async fn mcp_approve_with_stored_state() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());

    // Pre-store MCP approval state
    let approval_data = json!({
        "server_key": "github",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-github"],
        "user_did": "did:test:user1",
        "chat_id": 100,
        "created_at": chrono::Utc::now().to_rfc3339()
    });
    memory
        .store_skill_state("hitl", "mcp-github", &approval_data.to_string())
        .await
        .unwrap();

    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "approve mcp mcp-github", Some(100))
        .await
        .unwrap();

    // Should process (not "not found")
    assert!(
        !result.reply.contains("not found"),
        "Should process MCP approval: {}",
        result.reply
    );
}

/// reject mcp with non-existent id.
#[tokio::test]
async fn mcp_reject_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "reject mcp mcp-nonexistent", Some(100))
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 0);
}

// ── Menu approval tests ─────────────────────────────────────────────────────

/// approve menu with non-existent id.
#[tokio::test]
async fn menu_approve_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message("did:test:user1", "approve menu menu-abc", Some(100))
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 0);
}

// ── Credential detection (unit tests for public functions) ───────────────────

/// detect_google_credentials recognizes standard format.
#[tokio::test]
async fn google_credential_detection_standard_format() {
    let text = "client_id: 123456789.apps.googleusercontent.com\nclient_secret: GOCSPX-abc123xyz";
    let result = aegis_runtime_core::telegram::detect_google_credentials(text);
    assert!(
        result.is_some(),
        "Should detect credentials in standard format"
    );
    let (cid, cs) = result.unwrap();
    assert!(cid.contains("123456789"));
    assert!(cs.contains("GOCSPX"));
}

/// detect_google_credentials rejects normal text.
#[tokio::test]
async fn google_credential_detection_rejects_normal() {
    let text = "Hello, how can I help you today?";
    let result = aegis_runtime_core::telegram::detect_google_credentials(text);
    assert!(result.is_none());
}

/// detect_google_auth_code recognizes OAuth code format.
#[tokio::test]
async fn google_auth_code_detection() {
    assert!(aegis_runtime_core::telegram::detect_google_auth_code(
        "4/0AeanS0abcdefghijklmnopqrstuvwxyz"
    ));
    assert!(!aegis_runtime_core::telegram::detect_google_auth_code(
        "Hello there"
    ));
    assert!(!aegis_runtime_core::telegram::detect_google_auth_code(
        "4/ab"
    )); // too short
}

// ── Panel submission: task_done / task_archive / task_skip ──────────────────

/// task_done via panel submission marks task as done.
#[tokio::test]
async fn panel_task_done_marks_complete() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let chat_id = 42i64;
    let task_id = memory
        .store_task(
            chat_id,
            "did:test:user1",
            "Buy groceries",
            "",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "notify",
            None,
            None,
            None,
            None,
            false,
            None,
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{{\"task_done:{}\":true}}",
        task_id,
    );
    let result = orch
        .process_message("did:test:user1", &msg, Some(chat_id))
        .await
        .unwrap();
    assert!(
        result.reply.contains("Done") || result.reply.contains("已完成"),
        "reply={}",
        result.reply
    );

    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "done");
    assert!(task.completed_at.is_some());
}

/// Replying "done" to a quoted task with exactly one task id should bypass
/// the heavyweight ReAct path.
#[tokio::test]
async fn quoted_task_done_marks_complete_without_llm() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let chat_id = 42i64;
    let task_id = memory
        .store_task(
            chat_id,
            "did:test:user1",
            "Defrost fridge",
            "",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "notify",
            None,
            None,
            None,
            None,
            false,
            None,
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let msg = format!(
        "[引用消息] ⏰ #{} Defrost fridge\n\n[用户回复] done",
        task_id
    );
    let result = orch
        .process_message("did:test:user1", &msg, Some(chat_id))
        .await
        .unwrap();

    assert_eq!(result.rounds_used, 0);
    assert_eq!(state.chat_calls(), 0);
    assert_eq!(state.embed_calls(), 0);
    assert!(result.reply.contains("completed"), "reply={}", result.reply);
    assert!(
        result
            .tools_called
            .contains(&"fast_task_complete".to_string()),
        "tools={:?}",
        result.tools_called
    );

    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "done");
    assert!(task.completed_at.is_some());
}

/// Ambiguous quoted messages should keep the normal ReAct path.
#[tokio::test]
async fn quoted_done_with_multiple_task_ids_does_not_fast_path() {
    let state = Arc::new(MockServerState::new(vec![text_response("normal path")]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let result = orch
        .process_message(
            "did:test:user1",
            "[引用消息] #1 and #2 are both mentioned\n\n[用户回复] done",
            Some(42),
        )
        .await
        .unwrap();

    assert_ne!(result.rounds_used, 0);
    assert!(state.chat_calls() > 0, "expected normal LLM path");
}

/// task_done on non-existent task returns "not found".
#[tokio::test]
async fn panel_task_done_not_found() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory, settings);

    let msg = "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{\"task_done:99999\":true}";
    let result = orch
        .process_message("did:test:user1", msg, Some(42))
        .await
        .unwrap();
    assert!(
        result.reply.contains("not found")
            || result.reply.contains("Not found")
            || result.reply.contains("未找到"),
        "reply={}",
        result.reply
    );
}

/// task_archive via panel submission archives the task.
#[tokio::test]
async fn panel_task_archive() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let chat_id = 42i64;
    let task_id = memory
        .store_task(
            chat_id,
            "did:test:user1",
            "Old task",
            "",
            "low",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "notify",
            None,
            None,
            None,
            None,
            false,
            None,
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{{\"task_archive:{}\":true}}",
        task_id,
    );
    let result = orch
        .process_message("did:test:user1", &msg, Some(chat_id))
        .await
        .unwrap();
    assert!(
        result.reply.contains("Archived") || result.reply.contains("已归档"),
        "reply={}",
        result.reply
    );

    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "archived");
}

/// task_skip on recurring task reschedules to next interval.
#[tokio::test]
async fn panel_task_skip_recurring() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let chat_id = 42i64;
    let now = chrono::Utc::now().timestamp();
    let task_id = memory
        .store_task(
            chat_id,
            "did:test:user1",
            "Daily standup",
            "standup",
            "normal",
            &[],
            None,
            "user",
            Some(now - 60),
            Some("time"),
            None,
            "notify",
            Some("standup"),
            Some(86400),
            Some("standup_daily"),
            None,
            false,
            Some(now + 365 * 86400),
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{{\"task_skip:{}\":true}}",
        task_id,
    );
    let result = orch
        .process_message("did:test:user1", &msg, Some(chat_id))
        .await
        .unwrap();
    assert!(
        result.reply.contains("Skipped")
            || result.reply.contains("rescheduled")
            || result.reply.contains("已跳过"),
        "reply={}",
        result.reply
    );

    // Task should still be active with a future trigger
    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "active");
    assert!(
        task.trigger_at_unix.unwrap() > now,
        "trigger should be in the future"
    );
}

/// task_skip on non-recurring task falls back to done.
#[tokio::test]
async fn panel_task_skip_non_recurring_falls_back_to_done() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    let chat_id = 42i64;
    let task_id = memory
        .store_task(
            chat_id,
            "did:test:user1",
            "One-time task",
            "",
            "normal",
            &[],
            None,
            "user",
            None,
            None,
            None,
            "notify",
            None,
            None,
            None,
            None,
            false,
            None,
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{{\"task_skip:{}\":true}}",
        task_id,
    );
    let result = orch
        .process_message("did:test:user1", &msg, Some(chat_id))
        .await
        .unwrap();
    assert!(
        result.reply.contains("Done") || result.reply.contains("已完成"),
        "reply={}",
        result.reply
    );

    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "done");
}

/// task_skip with wrong chat_id is rejected (IDOR protection).
#[tokio::test]
async fn panel_task_skip_idor_rejected() {
    let state = Arc::new(MockServerState::new(vec![]));
    let (addr, _handle) = spawn_mock_server(state.clone()).await;
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let orch = build_orchestrator(addr, memory.clone(), settings);

    // Task belongs to chat 42
    let task_id = memory
        .store_task(
            42,
            "did:test:user1",
            "Secret task",
            "",
            "normal",
            &[],
            None,
            "user",
            Some(chrono::Utc::now().timestamp()),
            Some("time"),
            None,
            "notify",
            Some("secret"),
            Some(3600),
            Some("secret_group"),
            None,
            false,
            None,
            false,
            0,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Attempt from chat 999
    let msg = format!(
        "[WEBAPP_SUBMISSION callback_id=w-test action=submit panel:tasks]\n{{\"task_skip:{}\":true}}",
        task_id,
    );
    let result = orch
        .process_message("did:test:attacker", &msg, Some(999))
        .await
        .unwrap();
    assert!(
        result.reply.contains("not found")
            || result.reply.contains("Not found")
            || result.reply.contains("未找到"),
        "IDOR should be blocked, reply={}",
        result.reply
    );

    // Task should still be active and unchanged
    let task = memory.get_task(task_id).await.unwrap();
    assert_eq!(task.status, "active");
}
