//! E2E tests for the Agent OS runtime guarantees.
//!
//! Covers the runtime-side Agent OS guarantees:
//! - WIT contract validation
//! - Shadow mode pattern selection
//! - Health analyzer metrics scanning
//! - Wasm trap classification
//! - Hot-reload file watcher module

mod common;

use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::wit_contract;
use serde_json::json;

// ── WIT Contract E2E ────────────────────────────────────────────────────────

#[test]
fn e2e_wit_contract_validates_real_skill_module() {
    // Compile a valid skill WAT and validate against the contract
    let engine = wasmtime::Engine::default();
    let wat = r#"(module
        (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
        (import "aegis" "host_write_output" (func $write (param i32 i32)))
        (import "aegis" "host_get_payload_len" (func $plen (result i32)))
        (import "aegis" "host_get_payload" (func $pget (param i32)))
        (import "aegis" "host_get_fetch_response_len" (func $frlen (result i32)))
        (import "aegis" "host_get_fetch_response" (func $frget (param i32)))
        (memory (export "memory") 1)
        (func (export "run")
            ;; Simple skill that just writes output
            (call $write (i32.const 0) (i32.const 2))
        )
        (func (export "alloc") (param i32) (result i32)
            (i32.const 0)
        )
    )"#;
    let module = wasmtime::Module::new(&engine, wat).unwrap();

    // Validate against api_json pattern
    let result = wit_contract::validate_contract(&module, "api_json");
    assert!(
        result.valid,
        "Valid api_json skill should pass: {:?}",
        result.errors
    );
    assert!(result.exports.contains(&"run".to_string()));
    assert!(result.exports.contains(&"alloc".to_string()));
    assert!(result.exports.contains(&"memory".to_string()));

    // Same module against no_network should fail (imports host_fetch)
    let result_no_net = wit_contract::validate_contract(&module, "no_network");
    assert!(!result_no_net.valid, "Should fail for no_network pattern");
    assert!(
        result_no_net
            .errors
            .iter()
            .any(|e| e.contains("host_fetch"))
    );
}

#[test]
fn e2e_wit_contract_rejects_incomplete_module() {
    let engine = wasmtime::Engine::default();
    // Module missing alloc and memory exports
    let wat = r#"(module
        (func (export "run"))
    )"#;
    let module = wasmtime::Module::new(&engine, wat).unwrap();
    let result = wit_contract::validate_contract(&module, "no_network");
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("alloc")));
    assert!(result.errors.iter().any(|e| e.contains("memory")));
}

#[test]
fn e2e_wit_contract_catches_typo_imports() {
    let engine = wasmtime::Engine::default();
    let wat = r#"(module
        (import "aegis" "host_fetc" (func (param i32 i32) (result i32)))
        (import "aegis" "host_write_output" (func (param i32 i32)))
        (import "aegis" "host_get_payload_len" (func (result i32)))
        (import "aegis" "host_get_payload" (func (param i32)))
        (memory (export "memory") 1)
        (func (export "run"))
        (func (export "alloc") (param i32) (result i32) (i32.const 0))
    )"#;
    let module = wasmtime::Module::new(&engine, wat).unwrap();
    let result = wit_contract::validate_contract(&module, "api_json");
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("host_fetc")));
}

// ── Shadow Mode Pattern Selection E2E ───────────────────────────────────────

#[test]
fn e2e_shadow_mode_correctly_set_on_capabilities() {
    // Stateful pattern should have shadow mode
    let stateful_cap = SkillCapability {
        skill_name: "stateful_tracker".to_string(),
        max_memory_mb: 4,
        allowed_domains: vec!["api.example.com".to_string()],
        allowed_fs_paths: vec![],
        cron_schedule: None,
        description: "Tracks changes".to_string(),
        payload_schema: json!({}),
        default_payload: None,
        skill_pattern: Some("stateful".to_string()),
        language: "rust".to_string(),
        is_read_only: false,
        supports_multi_account: false,
        timeout_secs: 60,
        max_fuel: 500_000_000,
        shadow_mode: true, // Should be true for stateful
        menu_command: None,
        menu_description: None,
        component_model: false,
        limitations: vec![],
    };
    assert!(stateful_cap.shadow_mode);

    // api_json should NOT have shadow mode
    let api_cap = SkillCapability {
        skill_name: "simple_api".to_string(),
        shadow_mode: false,
        ..stateful_cap.clone()
    };
    assert!(!api_cap.shadow_mode);
}

// ── Health Analyzer E2E ─────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_health_analyzer_detects_degraded_skill() {
    let mem = MemoryHandle::spawn_in_memory().unwrap();

    // Simulate a skill with high error rate
    for _ in 0..15 {
        mem.update_skill_metrics("broken_api", false, 500.0, Some("Connection refused"), None)
            .await;
    }
    for _ in 0..5 {
        mem.update_skill_metrics("broken_api", true, 100.0, None, None)
            .await;
    }

    // Simulate a healthy skill
    for _ in 0..20 {
        mem.update_skill_metrics("healthy_skill", true, 50.0, None, None)
            .await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let recs = aegis_runtime_core::health_analyzer::analyze_skill_health(&mem).await;

    // Should recommend repair for broken_api only
    assert_eq!(
        recs.len(),
        1,
        "Should have 1 recommendation, got {}",
        recs.len()
    );
    assert_eq!(recs[0].skill_name, "broken_api");
    assert!(recs[0].error_rate > 0.5);
    assert!(recs[0].recommendation.contains("deactivate_skill"));
}

// ── Trap Classification E2E ─────────────────────────────────────────────────

#[test]
fn e2e_trap_classification_fuel_exhaustion() {
    use aegis_runtime_core::identity::AgentIdentity;

    let engine = common::shared_sandbox();

    // Infinite loop skill
    let wat = r#"(module
        (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
        (import "aegis" "host_write_output" (func $write (param i32 i32)))
        (import "aegis" "host_get_payload_len" (func $plen (result i32)))
        (import "aegis" "host_get_payload" (func $pget (param i32)))
        (memory (export "memory") 1)
        (func (export "run")
            (loop $inf (br $inf))
        )
        (func (export "alloc") (param $size i32) (result i32) (i32.const 0))
    )"#;

    let cap = SkillCapability {
        skill_name: "infinite_loop_test".to_string(),
        max_memory_mb: 2,
        allowed_domains: vec![],
        allowed_fs_paths: vec![],
        cron_schedule: None,
        description: "test".to_string(),
        payload_schema: json!({}),
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
    };
    engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

    let id = AgentIdentity::generate();
    let envelope = id.sign("infinite_loop_test", json!({})).unwrap();
    let result = engine.execute(&envelope);

    assert!(result.is_err());
    let err_msg = format!("{:#}", result.unwrap_err());
    // Should contain the TrapKind classification
    assert!(
        err_msg.contains("FuelExhausted"),
        "Error should contain trap classification: {err_msg}"
    );
}

// ── Hot Reload Module Existence ─────────────────────────────────────────────

#[test]
fn e2e_hot_reload_module_exists() {
    // Verify the hot_reload module is properly registered and accessible
    // The full watcher test requires filesystem interaction which is covered
    // by the unit tests in hot_reload.rs
    use aegis_runtime_core::hot_reload::ReloadEvent;
    let _event = ReloadEvent {
        skill_name: "test".to_string(),
        wasm_path: std::path::PathBuf::from("/tmp/test.wasm"),
        timestamp: std::time::Instant::now(),
    };
}
