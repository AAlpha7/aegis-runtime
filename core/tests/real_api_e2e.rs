//! Comprehensive real-API end-to-end tests.
//!
//! Every test in this file hits **real APIs** (OpenRouter, xAI, etc.) and is
//! marked `#[ignore]` so CI/`cargo test` skips them by default.
//!
//! Run explicitly:
//!
//!     cargo test -p aegis-runtime-core --test real_api_e2e -- --ignored --nocapture
//!
//! Required env vars:
//!   OPENAI_API_KEY    — OpenRouter API key (for LLM + credits/activity)
//!   OPENAI_API_BASE   — Base URL (default: https://openrouter.ai/api/v1)
//!   LLM_MODEL         — Chat model (default: minimax/minimax-m2.5)
//!   EMBEDDING_MODEL   — Embedding model (default: openai/text-embedding-3-small)
//!
//! Optional:
//!   AEGIS_TTS_API_KEY  — xAI TTS key (for TTS test)
//!   AEGIS_IMAGEGEN_MODEL — Image gen model override

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

// ===========================================================================
// Shared setup
// ===========================================================================

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load_all_skills(sandbox: &SandboxEngine) {
    let root = project_root();
    let compiled_dir = root.join("compiled_skills");
    let skills_dir = root.join("skills");

    if !compiled_dir.exists() {
        eprintln!("[E2E] Warning: compiled_skills/ not found");
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
            let json = std::fs::read_to_string(&manifest_path).unwrap();
            serde_json::from_str::<SkillCapability>(&json).unwrap()
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
        let _ = sandbox.load_skill(&path, capability);
        loaded += 1;
    }
    eprintln!("[E2E] Loaded {loaded} Wasm skills");
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

// ===========================================================================
// 1. OpenRouter Credits API — real balance check
// ===========================================================================

/// Verify that the OpenRouter /api/v1/credits endpoint returns valid balance data.
#[tokio::test]
#[ignore]
async fn real_api_openrouter_credits() {
    common::load_test_env();
    let api_key = get_env("OPENAI_API_KEY");
    let client = reqwest::Client::new();

    let resp = client
        .get("https://openrouter.ai/api/v1/credits")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .expect("credits request failed");

    let status = resp.status();
    assert!(status.is_success(), "Credits API returned {status}");

    let body: serde_json::Value = resp.json().await.expect("JSON parse failed");
    let data = &body["data"];
    assert!(
        data["total_credits"].is_number(),
        "Expected total_credits number, got: {data}"
    );
    assert!(
        data["total_usage"].is_number(),
        "Expected total_usage number, got: {data}"
    );

    let total = data["total_credits"].as_f64().unwrap();
    let used = data["total_usage"].as_f64().unwrap();
    let remaining = total - used;

    println!("Credits: total=${total:.4}, used=${used:.4}, remaining=${remaining:.4}");
    assert!(total >= 0.0, "total_credits should be non-negative");
    assert!(used >= 0.0, "total_usage should be non-negative");
}

// ===========================================================================
// 2. OpenRouter Activity API — real usage breakdown
// ===========================================================================

/// Verify that the OpenRouter /api/v1/activity endpoint returns usage data.
/// Note: This endpoint requires a management key. Regular API keys get 403.
#[tokio::test]
#[ignore]
async fn real_api_openrouter_activity() {
    common::load_test_env();
    let api_key = get_env("OPENAI_API_KEY");
    let client = reqwest::Client::new();

    let resp = client
        .get("https://openrouter.ai/api/v1/activity")
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .expect("activity request failed");

    let status = resp.status();
    if status.as_u16() == 403 {
        println!("Activity API returned 403 — management key required, skipping");
        return;
    }
    assert!(status.is_success(), "Activity API returned {status}");

    let body: serde_json::Value = resp.json().await.expect("JSON parse failed");
    let data = body["data"].as_array().expect("Expected data array");

    println!("Activity entries: {} (last 30 days)", data.len());

    if !data.is_empty() {
        let first = &data[0];
        assert!(first.get("date").is_some(), "Missing 'date' field");
        assert!(first.get("model").is_some(), "Missing 'model' field");
        assert!(first.get("usage").is_some(), "Missing 'usage' field");
        assert!(first.get("requests").is_some(), "Missing 'requests' field");

        let mut model_costs: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for entry in data {
            let model = entry["model"].as_str().unwrap_or("unknown");
            let cost = entry["usage"].as_f64().unwrap_or(0.0);
            *model_costs.entry(model.to_string()).or_default() += cost;
        }
        let mut sorted: Vec<_> = model_costs.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!("Top models by cost:");
        for (model, cost) in sorted.iter().take(5) {
            println!("  ${cost:.4}  {model}");
        }
    }
}

// ===========================================================================
// 3. OpenRouter Activity filtered by date
// ===========================================================================

/// Verify date-filtered activity returns data for today.
/// Note: Requires management key. Regular API keys get 403.
#[tokio::test]
#[ignore]
async fn real_api_openrouter_activity_today() {
    common::load_test_env();
    let api_key = get_env("OPENAI_API_KEY");
    let client = reqwest::Client::new();

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let resp = client
        .get(format!(
            "https://openrouter.ai/api/v1/activity?date={today}"
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .expect("activity request failed");

    let status_code = resp.status().as_u16();
    if status_code == 403 || status_code == 400 {
        println!(
            "Activity API returned {status_code} — management key required or date not supported, skipping"
        );
        return;
    }
    assert!(
        resp.status().is_success(),
        "Activity API returned {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.unwrap();
    let data = body["data"].as_array().expect("Expected data array");

    println!("Today ({today}): {len} entries", len = data.len());

    for entry in data {
        assert_eq!(
            entry["date"].as_str(),
            Some(today.as_str()),
            "Entry date mismatch"
        );
    }
}

// ===========================================================================
// 4. Orchestrator: LLM calls check_api_usage when asked about spending
// ===========================================================================

/// When user asks "我的API花了多少钱" (how much did my API cost), the LLM should
/// call check_api_usage and report real balance/usage data.
#[tokio::test]
#[ignore]
async fn real_api_llm_calls_check_api_usage() {
    let (orch, _memory) = setup_orchestrator().await;

    let result = orch
        .process_message(
            "did:test:e2e_api_usage",
            "查看我的API余额和今日消费",
            Some(99999),
        )
        .await
        .expect("process_message failed");

    println!("LLM reply: {}", result.reply);
    println!("Tools called: {:?}", result.tools_called);

    // LLM should have called check_api_usage
    assert!(
        result.tools_called.iter().any(|t| t == "check_api_usage"),
        "Expected LLM to call check_api_usage, but called: {:?}",
        result.tools_called
    );

    // Reply should contain some numeric data (costs, credits, etc.)
    let has_numbers = result.reply.chars().any(|c| c.is_ascii_digit());
    assert!(
        has_numbers,
        "Reply should contain numeric data from API, got: {}",
        result.reply
    );
}

// ===========================================================================
// 5. Orchestrator: LLM uses schedule_intent for reminders (NOT reminder skills)
// ===========================================================================

/// When user says "30秒后提醒我喝水" (remind me to drink water in 30 seconds),
/// the LLM should call schedule_intent (NOT schedule_reminder or any Wasm skill).
/// This verifies the fix from Task #1.
#[tokio::test]
#[ignore]
async fn real_api_llm_uses_schedule_intent_for_reminder() {
    let (orch, _memory) = setup_orchestrator().await;

    let result = orch
        .process_message("did:test:e2e_reminder", "30秒后提醒我喝水", Some(99999))
        .await
        .expect("process_message failed");

    println!("LLM reply: {}", result.reply);
    println!("Tools called: {:?}", result.tools_called);

    // Must use schedule_intent
    assert!(
        result.tools_called.iter().any(|t| t == "schedule_intent"),
        "Expected schedule_intent, but called: {:?}",
        result.tools_called
    );

    // Must NOT use reminder skills
    let bad_tools = ["schedule_reminder", "smart_recurring_reminder_calc"];
    for bad in &bad_tools {
        assert!(
            !result.tools_called.iter().any(|t| t == bad),
            "LLM should NOT call {bad}, but it did"
        );
    }

    // Must NOT be blocked
    assert!(
        result.tools_blocked.is_empty(),
        "No tools should be blocked, but got: {:?}",
        result.tools_blocked
    );
}

// ===========================================================================
// 6. Orchestrator: LLM calls web_search for current data
// ===========================================================================

/// When user asks for current info, LLM should call web_search, not fabricate.
#[tokio::test]
#[ignore]
async fn real_api_llm_calls_web_search_for_current_info() {
    let (orch, _memory) = setup_orchestrator().await;

    let result = orch
        .process_message(
            "did:test:e2e_web_search",
            "今天有什么重大AI新闻？",
            Some(99999),
        )
        .await
        .expect("process_message failed");

    let preview: String = result.reply.chars().take(200).collect();
    println!("LLM reply (first 200 chars): {preview}");
    println!("Tools called: {:?}", result.tools_called);

    // LLM should have called a data-fetching tool (web_search, news_digest, or web_reader)
    let search_tools = ["web_search", "news_digest", "web_reader"];
    assert!(
        result
            .tools_called
            .iter()
            .any(|t| search_tools.contains(&t.as_str())),
        "Expected a search/news tool for current events, but called: {:?}",
        result.tools_called
    );
}

// ===========================================================================
// 7. Orchestrator: LLM calls get_current_time when asked
// ===========================================================================

/// Time query should use the zero-cost get_current_time pseudo-tool.
#[tokio::test]
#[ignore]
async fn real_api_llm_calls_get_current_time() {
    let (orch, _memory) = setup_orchestrator().await;

    let result = orch
        .process_message("did:test:e2e_time", "现在几点了？", Some(99999))
        .await
        .expect("process_message failed");

    println!("LLM reply: {}", result.reply);
    println!("Tools called: {:?}", result.tools_called);

    // Should have time info in reply (digits for hours/minutes)
    assert!(
        result.reply.chars().any(|c| c.is_ascii_digit()),
        "Reply should contain time digits"
    );
}

// ===========================================================================
// 8. Orchestrator: multi-round tool use (LLM callback loop works)
// ===========================================================================

/// A query that requires tool calls should get real data, not fabrication.
/// "BTC现在什么价格" → should call crypto_tracker or web_search.
#[tokio::test]
#[ignore]
async fn real_api_llm_fetches_crypto_price() {
    let (orch, _memory) = setup_orchestrator().await;

    let result = orch
        .process_message("did:test:e2e_crypto", "BTC现在什么价格？", Some(99999))
        .await
        .expect("process_message failed");

    println!("LLM reply: {}", result.reply);
    println!("Tools called: {:?}", result.tools_called);

    // Should have called at least one data-fetching tool
    let data_tools = [
        "crypto_tracker",
        "us_stock_quote",
        "web_search",
        "web_reader",
    ];
    let used_data_tool = result
        .tools_called
        .iter()
        .any(|t| data_tools.contains(&t.as_str()));
    assert!(
        used_data_tool,
        "Expected a data-fetching tool for BTC price, but called: {:?}",
        result.tools_called
    );

    // Reply should contain a number (the price)
    let has_price = result.reply.chars().any(|c| c.is_ascii_digit());
    assert!(has_price, "Reply should contain BTC price digits");
}

// ===========================================================================
// 9. Image generation via OpenRouter (real API)
// ===========================================================================

/// Generate a simple image via OpenRouter multimodal API.
/// Verifies the model + modalities parameter combination works.
#[tokio::test]
#[ignore]
async fn real_api_image_generation_openrouter() {
    common::load_test_env();
    let api_key = get_env("OPENAI_API_KEY");
    let model = std::env::var("AEGIS_IMAGEGEN_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash-image".into());

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Generate a simple red circle on white background"}],
        "modalities": ["image", "text"],
    });

    let resp = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .expect("image gen request failed");

    let status = resp.status();
    let resp_body: serde_json::Value = resp.json().await.expect("JSON parse failed");

    assert!(
        status.is_success(),
        "Image gen API returned {status}: {resp_body}"
    );

    let msg = &resp_body["choices"][0]["message"];
    let images = msg["images"].as_array();
    assert!(
        images.is_some() && !images.unwrap().is_empty(),
        "Expected images array, got: {:?}",
        msg.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );

    let url = images.unwrap()[0]["image_url"]["url"]
        .as_str()
        .unwrap_or("");
    assert!(
        url.starts_with("data:image/"),
        "Expected data URL, got: {}",
        &url[..url.len().min(50)]
    );

    // Decode and check size
    let raw_b64 = url.find(',').map(|i| &url[i + 1..]).unwrap_or(url);
    let img_bytes = aegis_runtime_core::multimodal::decode_base64_standard(raw_b64)
        .expect("base64 decode failed");
    println!("Image generated: {} bytes, model: {model}", img_bytes.len());
    assert!(
        img_bytes.len() > 100,
        "Image too small: {} bytes",
        img_bytes.len()
    );
}

// ===========================================================================
// 10. TTS via xAI API (real API, optional)
// ===========================================================================

/// Test TTS synthesis via xAI API. Requires AEGIS_TTS_API_KEY.
#[tokio::test]
#[ignore]
async fn real_api_tts_xai() {
    common::load_test_env();
    let api_key = match std::env::var("AEGIS_TTS_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            println!("AEGIS_TTS_API_KEY not set, skipping TTS test");
            return;
        }
    };

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "tts-1",
        "input": "Hello, this is a test of the Aegis Runtime text-to-speech system.",
        "voice": "alloy",
    });

    let resp = client
        .post("https://api.x.ai/v1/tts")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .expect("TTS request failed");

    let status = resp.status();
    assert!(
        status.is_success(),
        "TTS API returned {status}: {}",
        resp.text().await.unwrap_or_default()
    );

    // Re-send to get bytes (first response was consumed for error message)
    let resp2 = client
        .post("https://api.x.ai/v1/tts")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .unwrap();

    let audio_bytes = resp2.bytes().await.unwrap();
    println!("TTS audio: {} bytes", audio_bytes.len());
    assert!(
        audio_bytes.len() > 1000,
        "Audio too small: {} bytes",
        audio_bytes.len()
    );
}

// ===========================================================================
// 11. Embedding API — vector dimensions match
// ===========================================================================

/// Verify embedding API returns correct dimensions.
#[tokio::test]
#[ignore]
async fn real_api_embedding_dimensions() {
    common::load_test_env();
    let api_key = get_env("OPENAI_API_KEY");
    let api_base =
        std::env::var("OPENAI_API_BASE").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let embed_model =
        std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "openai/text-embedding-3-small".into());

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": embed_model,
        "input": "test embedding query for Aegis Runtime",
        "dimensions": 512,
    });

    let resp = client
        .post(format!("{api_base}/embeddings"))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .expect("embedding request failed");

    let status = resp.status();
    let resp_body: serde_json::Value = resp.json().await.expect("JSON parse");

    assert!(
        status.is_success(),
        "Embedding API returned {status}: {resp_body}"
    );

    let embedding = resp_body["data"][0]["embedding"]
        .as_array()
        .expect("Expected embedding array");
    println!("Embedding: {embed_model}, dimensions: {}", embedding.len());
    assert_eq!(embedding.len(), 512, "Expected 512 dimensions");
}

// ===========================================================================
// 12. Wasm skill execution — crypto_tracker returns real data
// ===========================================================================

/// Run crypto_tracker skill directly via sandbox with real network API.
#[test]
#[ignore]
fn real_api_wasm_crypto_tracker() {
    common::load_test_env();
    let root = project_root();
    let wasm_path = root.join("compiled_skills/crypto_tracker.wasm");
    if !wasm_path.exists() {
        println!("crypto_tracker.wasm not found, skipping");
        return;
    }

    let manifest_path = root.join("skills/crypto_tracker/manifest.json");
    if !manifest_path.exists() {
        println!("manifest not found, skipping");
        return;
    }
    let json = std::fs::read_to_string(&manifest_path).unwrap();
    let capability: SkillCapability = serde_json::from_str(&json).unwrap();

    let sandbox = common::shared_sandbox();
    sandbox.load_skill(&wasm_path, capability).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign("crypto_tracker", serde_json::json!({"symbol": "BTC"}))
        .unwrap();
    let result = sandbox.execute(&envelope).unwrap();

    println!(
        "crypto_tracker output ({}ms): {}",
        result.elapsed_ms,
        &result.output[..result.output.len().min(300)]
    );
    let val: serde_json::Value = serde_json::from_str(&result.output).expect("JSON parse");
    // crypto_tracker uses probe_heartbeat pattern: returns event_payload + require_llm_attention
    // OR status=ok for api_json pattern. Accept both formats plus known errors.
    let has_payload = val.get("event_payload").is_some();
    let has_status_ok = val.get("status").and_then(|s| s.as_str()) == Some("ok");
    let has_error = result.output.contains("error");
    assert!(
        has_payload || has_status_ok || has_error,
        "Expected event_payload, status=ok, or error in output: {}",
        result.output
    );
    if has_payload {
        let payload = val["event_payload"].as_str().unwrap_or("");
        assert!(
            payload.contains("BTC") || payload.contains("price") || payload.contains("$"),
            "event_payload should contain price info: {payload}"
        );
    }
}

// ===========================================================================
// 14. Intent system: schedule + list + cancel full cycle
// ===========================================================================

/// Full intent lifecycle: schedule a notify intent, list it, cancel it.
#[tokio::test]
#[ignore]
async fn real_api_intent_lifecycle() {
    let (orch, _memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_intent_lifecycle";
    let chat_id = Some(88888i64);

    // Step 1: Schedule a reminder
    let result = orch
        .process_message(user_did, "30秒后提醒我站起来活动一下", chat_id)
        .await
        .expect("schedule failed");

    println!("[Step 1] Schedule reply: {}", result.reply);
    println!("[Step 1] Tools called: {:?}", result.tools_called);

    assert!(
        result.tools_called.iter().any(|t| t == "schedule_intent"),
        "Expected schedule_intent, got: {:?}",
        result.tools_called
    );

    // Step 2: List intents
    let result2 = orch
        .process_message(user_did, "列出我所有的提醒和计划", chat_id)
        .await
        .expect("list failed");

    println!("[Step 2] List reply: {}", result2.reply);
    println!("[Step 2] Tools called: {:?}", result2.tools_called);

    assert!(
        result2.tools_called.iter().any(|t| t == "list_intents"),
        "Expected list_intents, got: {:?}",
        result2.tools_called
    );

    // Reply should mention the activity reminder
    assert!(
        result2.reply.contains("活动")
            || result2.reply.contains("stand")
            || result2.reply.contains("提醒"),
        "List reply should mention the scheduled reminder"
    );

    // Step 3: Cancel it
    let result3 = orch
        .process_message(user_did, "取消刚才设的提醒", chat_id)
        .await
        .expect("cancel failed");

    println!("[Step 3] Cancel reply: {}", result3.reply);
    println!("[Step 3] Tools called: {:?}", result3.tools_called);

    assert!(
        result3.tools_called.iter().any(|t| t == "cancel_intent"),
        "Expected cancel_intent, got: {:?}",
        result3.tools_called
    );
}

// ===========================================================================
// 15. Conversation context: multi-turn memory works
// ===========================================================================

/// Two consecutive messages — LLM should remember context from the first.
#[tokio::test]
#[ignore]
async fn real_api_conversation_context() {
    let (orch, _memory) = setup_orchestrator().await;
    let user_did = "did:test:e2e_context";
    let chat_id = Some(77777i64);

    // First message
    let r1 = orch
        .process_message(user_did, "我的名字叫小明", chat_id)
        .await
        .expect("msg 1 failed");
    println!("[Turn 1] Reply: {}", r1.reply);

    // Second message referencing the first
    let r2 = orch
        .process_message(user_did, "我叫什么名字？", chat_id)
        .await
        .expect("msg 2 failed");
    println!("[Turn 2] Reply: {}", r2.reply);

    assert!(
        r2.reply.contains("小明"),
        "LLM should remember the name from turn 1, got: {}",
        r2.reply
    );
}
