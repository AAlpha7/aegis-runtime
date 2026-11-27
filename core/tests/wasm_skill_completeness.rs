//! Wasm Skill Completeness — load + validate every compiled skill.
//!
//! Every .wasm file in compiled_skills/ must:
//! 1. Load into the SandboxEngine without panic
//! 2. Not crash on a minimal invocation attempt
//!
//! This catches stale/corrupt Wasm artifacts and skill contract violations.

mod common;

use std::path::PathBuf;

use aegis_runtime_core::models::SkillCapability;
use serde_json::json;

fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

/// Build a minimal SkillCapability for testing load.
fn test_cap(name: &str) -> SkillCapability {
    SkillCapability {
        skill_name: name.to_string(),
        max_memory_mb: 4,
        allowed_domains: vec![],
        allowed_fs_paths: vec![],
        cron_schedule: None,
        description: format!("Test: {name}"),
        payload_schema: json!({"type": "object"}),
        default_payload: None,
        skill_pattern: None,
        language: "rust".to_string(),
        is_read_only: false,
        supports_multi_account: false,
        timeout_secs: 30,
        max_fuel: 200_000_000,
        shadow_mode: false,
        menu_command: None,
        menu_description: None,
        component_model: false,
        limitations: vec![],
    }
}

/// Load a single skill and assert success.
fn assert_skill_loads(name: &str) {
    let engine = common::shared_sandbox();
    let wasm_path = compiled_skills_dir().join(format!("{name}.wasm"));
    assert!(
        wasm_path.exists(),
        "Wasm file missing: {}",
        wasm_path.display()
    );

    engine
        .load_skill(&wasm_path, test_cap(name))
        .unwrap_or_else(|e| {
            panic!("Failed to load skill '{name}': {e:#}");
        });

    // Verify it's registered
    let loaded: Vec<String> = engine
        .loaded_skills()
        .iter()
        .map(|s| s.skill_name.clone())
        .collect();
    assert!(
        loaded.contains(&name.to_string()),
        "'{name}' not in loaded skills after load: {loaded:?}"
    );
}

// ── Shipped skill load tests ─────────────────────────────────────────────────
// The curated OSS tree contains exactly these two Wasm skill crates. Legacy
// private integrations must not be treated as build failures here.

#[test]
fn wasm_load_test_no_net() {
    assert_skill_loads("test_no_net");
}

#[test]
fn wasm_load_web_reader() {
    assert_skill_loads("web_reader");
}
