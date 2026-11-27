//! Real-LLM end-to-end tests for the Deferred Intent System.
//!
//! These tests call the **actual LLM API** (e.g. minimax/minimax-m2.5 via OpenRouter)
//! to verify that the model correctly uses `schedule_intent`, `list_intents`, and
//! `cancel_intent` tools. This is the gap between our mock-based tests (which always
//! return "correct" tool calls) and production (where the model may hallucinate,
//! use the wrong tool, or not call any tool at all).
//!
//! Run explicitly (requires OPENAI_API_KEY + OPENAI_API_BASE):
//!
//!     cargo test -p aegis-runtime-core --test intent_e2e -- --ignored --nocapture
//!
//! Env vars:
//!   OPENAI_API_KEY    — API key (e.g. OpenRouter key)
//!   OPENAI_API_BASE   — Base URL (default: https://openrouter.ai/api/v1)
//!   LLM_MODEL         — Model (default: minimax/minimax-m2.5)
//!   EMBEDDING_MODEL   — Embedding model (default: openai/text-embedding-3-small)

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use aegis_runtime_core::embedding::EmbeddingClient;
use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::orchestrator::Orchestrator;
use aegis_runtime_core::sandbox::SandboxEngine;
use aegis_runtime_core::settings::RuntimeSettings;

use common::{get_env, init_tracing};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Load all compiled Wasm skills into the sandbox, reading manifests from skills/{name}/manifest.json.
/// This mirrors production's `load_skills_from_directory` so the test sees the same tool count (~24).
fn load_all_skills(sandbox: &SandboxEngine) {
    let root = project_root();
    let compiled_dir = root.join("compiled_skills");
    let skills_dir = root.join("skills");

    if !compiled_dir.exists() {
        eprintln!(
            "[E2E] Warning: compiled_skills/ not found at {}",
            compiled_dir.display()
        );
        return;
    }

    let mut loaded = 0;
    for entry in std::fs::read_dir(&compiled_dir)
        .expect("read compiled_skills")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }

        let skill_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        let manifest_path = skills_dir.join(skill_name).join("manifest.json");

        let capability = if manifest_path.exists() {
            let json = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", manifest_path.display()));
            serde_json::from_str::<SkillCapability>(&json)
                .unwrap_or_else(|e| panic!("parsing {}: {e}", manifest_path.display()))
        } else {
            SkillCapability {
                skill_name: skill_name.to_string(),
                max_memory_mb: 2,
                allowed_domains: vec![],
                allowed_fs_paths: vec![],
                cron_schedule: None,
                description: format!("Skill: {skill_name}"),
                payload_schema: serde_json::json!({}),
                default_payload: None,
                skill_pattern: None,
                language: "rust".to_string(),
                is_read_only: false,
                supports_multi_account: false,
                timeout_secs: 60,
                max_fuel: 500_000_000,
                shadow_mode: false,
                menu_command: None,
                menu_description: None,
                component_model: false,
                limitations: vec![],
            }
        };

        sandbox
            .load_skill(&path, capability)
            .unwrap_or_else(|e| panic!("loading skill {skill_name}: {e}"));

        loaded += 1;
    }

    eprintln!(
        "[E2E] Loaded {loaded} Wasm skills from {}",
        compiled_dir.display()
    );
}

async fn setup_orchestrator() -> (Orchestrator, MemoryHandle) {
    init_tracing();
    let api_key = get_env("OPENAI_API_KEY");
    let api_base =
        std::env::var("OPENAI_API_BASE").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "minimax/minimax-m2.5".into());
    let embed_model =
        std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "openai/text-embedding-3-small".into());

    let memory = MemoryHandle::spawn_in_memory().expect("memory init");
    let sandbox = common::shared_sandbox();

    // Load ALL compiled skills to match production tool count (~24 tools)
    load_all_skills(&sandbox);

    let sandbox = Arc::new(sandbox);
    let identity = Arc::new(AgentIdentity::generate());
    let embedder = EmbeddingClient::new(&api_base, &api_key, &embed_model, 512);
    let settings = Arc::new(RuntimeSettings::new());

    let orch = Orchestrator::new(
        &api_key,
        &api_base,
        &model,
        sandbox,
        identity,
        memory.clone(),
        embedder,
        settings,
    );

    (orch, memory)
}

// ── Test: LLM uses schedule_intent for time-delayed action requests ──────────

/// When a user says "10秒后获取最新AI新闻给我" (get me the latest AI news in 10 seconds),
/// the LLM should call `schedule_intent` with `execution_mode=llm_callback` and
/// a time-based trigger. It should NOT use the old `reminder` skill or just output text.
///
/// This is the exact scenario that failed in production (originally with "5分钟"):
/// the LLM either used `reminder` (text-only) or hallucinated a confirmation without
/// calling any tool. We use 10s (the minimum allowed delay) to keep the test fast.
#[tokio::test]
#[ignore]
async fn e2e_llm_schedules_time_intent_for_delayed_action() {
    let (orch, memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_intent_time";

    let result = orch
        .process_message(user_did, "10秒后获取最新AI新闻给我", Some(12345))
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply ===");
    eprintln!("{}", result.reply);
    eprintln!("\n=== Tools Called ===");
    eprintln!("{:?}", result.tools_called);
    eprintln!("\n=== Tools Blocked ===");
    eprintln!("{:?}", result.tools_blocked);
    eprintln!("=== Rounds Used: {} ===\n", result.rounds_used);

    // CRITICAL CHECK: schedule_intent must have been called
    assert!(
        result.tools_called.contains(&"schedule_intent".to_string()),
        "LLM MUST call schedule_intent for a time-delayed action request.\n\
         Instead called: {:?}\n\
         Reply: {}",
        result.tools_called,
        result.reply
    );

    // Verify the intent was actually stored in the database
    let intents = memory.list_user_intents(user_did).await.unwrap();
    eprintln!("=== Stored Intents ===");
    for intent in &intents {
        eprintln!(
            "  id={} trigger_type={} execution_mode={} context={}",
            intent.id, intent.trigger_type, intent.execution_mode, intent.context_text
        );
    }

    assert!(
        !intents.is_empty(),
        "At least one intent should be stored in the database"
    );

    let intent = &intents[0];
    assert_eq!(
        intent.trigger_type, "time",
        "Should be a time-based trigger"
    );
    assert_eq!(
        intent.execution_mode, "llm_callback",
        "Should use llm_callback to actually fetch news, not just notify"
    );
}

/// Same test but in English — verifies the model handles both languages.
#[tokio::test]
#[ignore]
async fn e2e_llm_schedules_time_intent_english() {
    let (orch, memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_intent_en";

    let result = orch
        .process_message(
            user_did,
            "In 60 seconds, check the latest Bitcoin price and tell me.",
            Some(12345),
        )
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply ===");
    eprintln!("{}", result.reply);
    eprintln!("=== Tools Called: {:?} ===\n", result.tools_called);

    assert!(
        result.tools_called.contains(&"schedule_intent".to_string()),
        "LLM should call schedule_intent for time-delayed actions.\n\
         Called: {:?}\nReply: {}",
        result.tools_called,
        result.reply
    );

    let intents = memory.list_user_intents(user_did).await.unwrap();
    assert!(!intents.is_empty(), "Intent should be stored");
    assert_eq!(intents[0].trigger_type, "time");
    assert_eq!(intents[0].execution_mode, "llm_callback");
}

// ── Test: LLM lists and cancels intents ─────────────────────────────────────

/// After scheduling an intent, the user can ask to list and cancel them.
#[tokio::test]
#[ignore]
async fn e2e_llm_lists_and_cancels_intents() {
    let (orch, memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_list_cancel";

    // Pre-store an intent so there's something to list
    let intent_id = memory
        .store_deferred_intent(
            user_did,
            12345,
            "time",
            Some(chrono::Utc::now().timestamp() + 3600),
            None,
            "notify",
            "Test reminder: check email",
            chrono::Utc::now().timestamp() + 86400,
            false,
            None,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Ask LLM to list intents
    let result = orch
        .process_message(user_did, "What intents do I have scheduled?", Some(12345))
        .await
        .expect("list_intents should work");

    eprintln!("\n=== List Reply ===");
    eprintln!("{}", result.reply);
    eprintln!("=== Tools Called: {:?} ===\n", result.tools_called);

    assert!(
        result.tools_called.contains(&"list_intents".to_string()),
        "LLM should call list_intents.\nCalled: {:?}\nReply: {}",
        result.tools_called,
        result.reply
    );

    // Now ask to cancel
    let result2 = orch
        .process_message(
            user_did,
            &format!("Cancel intent #{}", intent_id),
            Some(12345),
        )
        .await
        .expect("cancel_intent should work");

    eprintln!("\n=== Cancel Reply ===");
    eprintln!("{}", result2.reply);
    eprintln!("=== Tools Called: {:?} ===\n", result2.tools_called);

    assert!(
        result2.tools_called.contains(&"cancel_intent".to_string()),
        "LLM should call cancel_intent.\nCalled: {:?}\nReply: {}",
        result2.tools_called,
        result2.reply
    );

    // Verify the intent is actually cancelled
    let intents = memory.list_user_intents(user_did).await.unwrap();
    assert!(
        intents.is_empty(),
        "Intent should be cancelled (no longer pending)"
    );
}

// ── Test: LLM does NOT use reminder skill for action requests ───────────────

/// This is a regression test for the production bug: LLM must not use the
/// `reminder` skill when the user asks for a future ACTION (not just a text reminder).
#[tokio::test]
#[ignore]
async fn e2e_llm_does_not_use_reminder_for_action_requests() {
    let (orch, _memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_no_reminder";

    let result = orch
        .process_message(user_did, "30秒后帮我查一下比特币的最新价格", Some(12345))
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply ===");
    eprintln!("{}", result.reply);
    eprintln!("=== Tools Called: {:?} ===\n", result.tools_called);

    // The reminder skill should NOT be used for action requests
    assert!(
        !result.tools_called.contains(&"reminder".to_string()),
        "LLM should NOT use reminder skill for action requests.\n\
         reminder is text-only. It should use schedule_intent instead.\n\
         Called: {:?}",
        result.tools_called
    );

    // schedule_intent should be called
    assert!(
        result.tools_called.contains(&"schedule_intent".to_string()),
        "LLM should call schedule_intent for delayed action.\n\
         Called: {:?}\nReply: {}",
        result.tools_called,
        result.reply
    );
}

// ── Test: LLM proactively uses schedule_intent ──────────────────────────────

/// The system prompt tells the LLM to proactively schedule intents when appropriate.
/// Here the user doesn't explicitly say "schedule" or "remind me" — they ask about
/// something that naturally has a future state. The LLM should consider scheduling
/// a follow-up on its own.
///
/// Note: This test is more aspirational — whether the model proactively schedules
/// depends on its reasoning. We check if any scheduling happened.
#[tokio::test]
#[ignore]
async fn e2e_llm_proactive_scheduling_awareness() {
    let (orch, memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_proactive";

    // A message where proactive scheduling would be appropriate
    let result = orch
        .process_message(
            user_did,
            "I set a limit order to buy ETH at $2000. Can you check in 2 minutes \
             whether it got filled?",
            Some(12345),
        )
        .await
        .expect("process_message should not error");

    eprintln!("\n=== LLM Reply ===");
    eprintln!("{}", result.reply);
    eprintln!("=== Tools Called: {:?} ===\n", result.tools_called);

    // This should trigger schedule_intent
    assert!(
        result.tools_called.contains(&"schedule_intent".to_string()),
        "LLM should schedule a time-based check for the ETH limit order.\n\
         Called: {:?}\nReply: {}",
        result.tools_called,
        result.reply
    );

    let intents = memory.list_user_intents(user_did).await.unwrap();
    assert!(!intents.is_empty(), "Should have stored a scheduled intent");
}
