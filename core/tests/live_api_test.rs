//! Live API integration tests — test each Wasm skill against real network APIs.
//!
//! Run with: cargo test -p aegis-runtime-core --test live_api_test -- --nocapture
//!
//! These tests make REAL HTTP requests. They require network access and may be
//! flaky if upstream APIs are down.

mod common;

use std::path::PathBuf;

use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::sandbox::SandboxEngine;
use serde_json::json;

fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

fn load_manifest(skill_name: &str) -> SkillCapability {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("skills")
        .join(skill_name)
        .join("manifest.json");
    let content = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|_| panic!("Failed to read manifest for {}", skill_name));
    let raw: serde_json::Value = serde_json::from_str(&content).unwrap();

    let allowed_domains: Vec<String> = raw
        .get("allowed_domains")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let allowed_fs_paths: Vec<String> = raw
        .get("allowed_fs_paths")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let cron = raw
        .get("cron_schedule")
        .and_then(|c| c.as_str())
        .map(String::from);

    SkillCapability {
        skill_name: skill_name.to_string(),
        max_memory_mb: raw
            .get("max_memory_mb")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as u32,
        allowed_domains,
        allowed_fs_paths,
        cron_schedule: cron,
        description: raw
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        payload_schema: raw.get("payload_schema").cloned().unwrap_or(json!({})),
        default_payload: raw.get("default_payload").cloned(),
        skill_pattern: raw
            .get("skill_pattern")
            .and_then(|v| v.as_str())
            .map(String::from),
        language: raw
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("rust")
            .to_string(),
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
}

fn setup_engine_with_skill(skill_name: &str) -> SandboxEngine {
    let engine = common::shared_sandbox();
    let wasm_path = compiled_skills_dir().join(format!("{}.wasm", skill_name));
    assert!(wasm_path.exists(), "{}.wasm not found", skill_name);
    let cap = load_manifest(skill_name);
    engine.load_skill(&wasm_path, cap).unwrap();
    engine
}

fn sign_and_execute(
    engine: &SandboxEngine,
    skill_name: &str,
    payload: serde_json::Value,
) -> aegis_runtime_core::sandbox::SkillResult {
    let identity = AgentIdentity::generate();
    let envelope = identity.sign(skill_name, payload).unwrap();
    engine.execute(&envelope).unwrap()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
#[ignore = "live network — hits real upstream APIs; flaky, run explicitly with --ignored"]
fn live_web_reader_example_com() {
    let engine = setup_engine_with_skill("web_reader");
    let result = sign_and_execute(&engine, "web_reader", json!({"url": "https://example.com"}));

    println!("[web_reader] output length: {}", result.output.len());
    println!("[web_reader] elapsed: {}ms", result.elapsed_ms);

    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("web_reader output should be valid JSON");
    assert_eq!(
        output["status"], "ok",
        "web_reader must succeed: {}",
        result.output
    );
    assert_eq!(output["fetch_code"], 0);
}

#[test]
#[ignore = "live network — hits real upstream APIs; flaky, run explicitly with --ignored"]
fn live_news_digest_hn() {
    let engine = setup_engine_with_skill("news_digest");
    let result = sign_and_execute(
        &engine,
        "news_digest",
        json!({"action": "top_stories", "count": 5}),
    );

    println!("[news_digest] output length: {}", result.output.len());
    println!("[news_digest] elapsed: {}ms", result.elapsed_ms);
    println!(
        "[news_digest] preview: {}",
        &result.output[..result.output.len().min(500)]
    );

    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("news_digest output should be valid JSON");

    // news_digest returns heartbeat format: {require_llm_attention, event_payload}
    // event_payload contains "Hacker News Top X Stories" when successful
    let payload = output
        .get("event_payload")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let has_stories = payload.contains("Stories") || payload.contains("stories");
    let has_status = output.get("status").is_some();
    assert!(
        has_stories || has_status,
        "news_digest must return stories or status: {}",
        result.output
    );
}

#[test]
fn local_no_network_skill() {
    let engine = setup_engine_with_skill("test_no_net");
    let result = sign_and_execute(&engine, "test_no_net", json!({"text":"oss-fixture"}));
    assert!(
        result.output.contains("oss-fixture"),
        "the shipped local Wasm fixture should execute: {}",
        result.output
    );
}

#[test]
#[ignore = "live network — hits real upstream APIs; flaky, run explicitly with --ignored"]
fn live_us_stock_quote() {
    let engine = setup_engine_with_skill("us_stock_quote");
    let result = sign_and_execute(&engine, "us_stock_quote", json!({"symbol": "AAPL"}));

    println!("[us_stock_quote] output length: {}", result.output.len());
    println!("[us_stock_quote] elapsed: {}ms", result.elapsed_ms);
    println!(
        "[us_stock_quote] output: {}",
        &result.output[..result.output.len().min(500)]
    );

    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("us_stock_quote output should be valid JSON");

    // Should return some data (price may or may not be available depending on API)
    assert!(
        output.is_object(),
        "us_stock_quote must return a JSON object"
    );
}

#[test]
#[ignore = "live network — hits real upstream APIs; flaky, run explicitly with --ignored"]
fn live_http_api_get() {
    let engine = setup_engine_with_skill("http_api");
    let result = sign_and_execute(
        &engine,
        "http_api",
        json!({
            "url": "https://httpbin.org/get",
            "method": "GET"
        }),
    );

    println!("[http_api] output length: {}", result.output.len());
    println!("[http_api] elapsed: {}ms", result.elapsed_ms);
    println!(
        "[http_api] preview: {}",
        &result.output[..result.output.len().min(500)]
    );

    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("http_api output should be valid JSON");
    assert_eq!(
        output["status"], "ok",
        "http_api GET must succeed: {}",
        result.output
    );
}
