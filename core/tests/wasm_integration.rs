mod common;

use std::path::PathBuf;

use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::memory::MemoryHandle;
use aegis_runtime_core::models::SkillCapability;
use aegis_runtime_core::sandbox::SandboxEngine;
use serde_json::json;
fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

fn web_reader_cap() -> SkillCapability {
    SkillCapability {
        skill_name: "web_reader".to_string(),
        max_memory_mb: 4,
        allowed_domains: vec![
            "news.ycombinator.com".to_string(),
            "example.com".to_string(),
        ],
        allowed_fs_paths: vec![],
        cron_schedule: None,
        description: "Fetches a URL and returns the content".to_string(),
        payload_schema: json!({"type": "object", "properties": {"url": {"type": "string"}}}),
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
}

/// Helper: sign and execute a skill via the full S-LEG pipeline.
fn sign_and_execute(
    engine: &SandboxEngine,
    skill_name: &str,
    payload: serde_json::Value,
) -> aegis_runtime_core::sandbox::SkillResult {
    let identity = AgentIdentity::generate();
    let envelope = identity.sign(skill_name, payload).unwrap();
    engine.execute(&envelope).unwrap()
}

#[test]
fn real_web_reader_allowed_domain() {
    let dir = compiled_skills_dir();
    let wasm_path = dir.join("web_reader.wasm");
    if !wasm_path.exists() {
        panic!(
            "web_reader.wasm not found. Run: cargo build --target wasm32-wasip1 --release -p web-reader"
        );
    }

    let engine = common::shared_sandbox();
    engine.load_skill(&wasm_path, web_reader_cap()).unwrap();

    let result = sign_and_execute(
        &engine,
        "web_reader",
        json!({"url": "https://example.com/page"}),
    );

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["status"], "ok");
    assert_eq!(output["fetch_code"], 0);
}

#[test]
fn real_web_reader_blocked_domain() {
    let dir = compiled_skills_dir();
    let wasm_path = dir.join("web_reader.wasm");

    let engine = common::shared_sandbox();
    engine.load_skill(&wasm_path, web_reader_cap()).unwrap();

    let result = sign_and_execute(
        &engine,
        "web_reader",
        json!({"url": "https://evil-site.com/steal"}),
    );

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["status"], "blocked");
    assert!(output["fetch_code"].as_i64().unwrap() < 0);
}
// ── todo_list_manager (stateful, no-network) ────────────────────────────

fn todo_list_cap() -> SkillCapability {
    SkillCapability {
        skill_name: "todo_list_manager".to_string(),
        max_memory_mb: 8,
        allowed_domains: vec![],
        allowed_fs_paths: vec![],
        cron_schedule: None,
        description: "Manage todo tasks with persistent state".to_string(),
        payload_schema: json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["add","list","complete","delete"]},
                "title": {"type": "string"},
                "due_date": {"type": "string"},
                "tag": {"type": "string"},
                "task_id": {"type": "integer"}
            },
            "required": ["action"]
        }),
        default_payload: Some(json!({"action": "list"})),
        skill_pattern: Some("no_network".to_string()),
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
}

/// Helper: execute a skill in spawn_blocking (sandbox uses block_on internally
/// for state persistence, which conflicts with the tokio test runtime).
async fn exec_blocking(
    engine: &std::sync::Arc<SandboxEngine>,
    env: &aegis_runtime_core::models::IntentEnvelope,
) -> aegis_runtime_core::sandbox::SkillResult {
    let e = engine.clone();
    let env = env.clone();
    tokio::task::spawn_blocking(move || e.execute(&env).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn todo_list_add_and_list() {
    let dir = compiled_skills_dir();
    let wasm_path = dir.join("todo_list_manager.wasm");
    if !wasm_path.exists() {
        eprintln!("Skipping: todo_list_manager.wasm not found");
        return;
    }

    let engine = std::sync::Arc::new(common::shared_sandbox());
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    engine.set_memory_handle(memory);
    engine.load_skill(&wasm_path, todo_list_cap()).unwrap();
    let identity = AgentIdentity::generate();

    // Add a task
    let env = identity
        .sign(
            "todo_list_manager",
            json!({"action": "add", "title": "Buy milk", "tag": "personal"}),
        )
        .unwrap();
    let result = exec_blocking(&engine, &env).await;
    let out: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(out["status"], "success");
    assert_eq!(out["tasks"][0]["title"], "Buy milk");
    assert_eq!(out["tasks"][0]["completed"], false);

    // List tasks
    let env = identity
        .sign("todo_list_manager", json!({"action": "list"}))
        .unwrap();
    let result = exec_blocking(&engine, &env).await;
    let out: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(out["status"], "success");
    assert_eq!(out["tasks"].as_array().unwrap().len(), 1);

    // Complete the task
    let env = identity
        .sign(
            "todo_list_manager",
            json!({"action": "complete", "task_id": 1}),
        )
        .unwrap();
    let result = exec_blocking(&engine, &env).await;
    let out: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(out["status"], "success");
    assert_eq!(out["tasks"][0]["completed"], true);

    // Error: empty title
    let env = identity
        .sign("todo_list_manager", json!({"action": "add", "title": ""}))
        .unwrap();
    let result = exec_blocking(&engine, &env).await;
    let out: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(out["status"], "error");
}

#[tokio::test]
async fn todo_list_delete() {
    let dir = compiled_skills_dir();
    let wasm_path = dir.join("todo_list_manager.wasm");
    if !wasm_path.exists() {
        eprintln!("Skipping: todo_list_manager.wasm not found");
        return;
    }

    let engine = std::sync::Arc::new(common::shared_sandbox());
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    engine.set_memory_handle(memory);
    engine.load_skill(&wasm_path, todo_list_cap()).unwrap();
    let identity = AgentIdentity::generate();

    // Add two tasks
    let env = identity
        .sign(
            "todo_list_manager",
            json!({"action": "add", "title": "Task A"}),
        )
        .unwrap();
    exec_blocking(&engine, &env).await;
    let env = identity
        .sign(
            "todo_list_manager",
            json!({"action": "add", "title": "Task B"}),
        )
        .unwrap();
    exec_blocking(&engine, &env).await;

    // Delete first
    let env = identity
        .sign(
            "todo_list_manager",
            json!({"action": "delete", "task_id": 1}),
        )
        .unwrap();
    let result = exec_blocking(&engine, &env).await;
    let out: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(out["status"], "success");
    assert_eq!(out["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(out["tasks"][0]["title"], "Task B");
}
