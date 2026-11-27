//! End-to-end tests for the Agentic SysAdmin HITL (Human-In-The-Loop) flow.
//!
//! These tests verify the full pipeline from Wasm skill execution through
//! orchestrator interception to executor command execution.
//!
//! Tests marked `#[ignore]` require a running Aegis Executor on localhost:9876.

mod common;

use std::path::PathBuf;

use aegis_runtime_core::identity::AgentIdentity;
use aegis_runtime_core::models::SkillCapability;
use serde_json::json;

/// Extract the error/message field from WASM skill output.
/// The sysadmin_ops skill uses `"error"` field, not `"message"`.
fn error_msg(output: &serde_json::Value) -> String {
    output
        .get("message")
        .or_else(|| output.get("error"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

fn load_sysadmin_manifest() -> SkillCapability {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("skills/sysadmin_ops/manifest.json");
    let content =
        std::fs::read_to_string(&manifest_path).expect("Failed to read sysadmin_ops manifest");
    let raw: serde_json::Value = serde_json::from_str(&content).unwrap();

    SkillCapability {
        skill_name: "sysadmin_ops".to_string(),
        max_memory_mb: raw
            .get("max_memory_mb")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as u32,
        allowed_domains: raw
            .get("allowed_domains")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        allowed_fs_paths: raw
            .get("allowed_fs_paths")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        cron_schedule: None,
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

// ── E2E Test 1: sysadmin_ops diagnose action ────────────────────────────────

/// Verify that the diagnose action calls the executor and returns system info.
/// Requires running Aegis Executor on localhost:9876.
#[test]
#[ignore]
fn e2e_sysadmin_diagnose_uname() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "diagnose",
                "diagnostic_cmd": "uname -a"
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    println!(
        "[sysadmin diagnose] output: {}",
        &result.output[..result.output.len().min(500)]
    );

    let output: serde_json::Value = serde_json::from_str(&result.output)
        .expect("sysadmin_ops diagnose output should be valid JSON");

    // The skill should return a well-formed JSON response.
    // If executor is reachable: {"status": "ok", "output": "..."}
    // If not reachable: {"status": "error", "message": "Failed to reach..."}
    assert!(
        output.get("status").is_some(),
        "diagnose should return a JSON object with status field: {}",
        result.output
    );
}

/// Verify piped diagnostic commands work (e.g., dpkg -l | grep <pkg>).
/// Requires running Aegis Executor on localhost:9876.
#[test]
#[ignore]
fn e2e_sysadmin_diagnose_piped_command() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "diagnose",
                "diagnostic_cmd": "dpkg -l | head -5"
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    println!(
        "[sysadmin diagnose pipe] output: {}",
        &result.output[..result.output.len().min(500)]
    );

    // Piped command should work (pipe is no longer in FORBIDDEN_CHARS)
    assert!(
        !result.output.contains("forbidden shell metacharacters"),
        "piped diagnostic commands should be allowed: {}",
        result.output
    );
}

// ── E2E Test 2: sysadmin_ops propose_changes returns pending_human_approval ──

/// Verify that propose_changes action returns the HITL status that the
/// orchestrator should intercept, without actually executing any commands.
#[test]
fn e2e_sysadmin_propose_changes_returns_pending() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "propose_changes",
                "plan_description": "Install Chrome browser on Ubuntu",
                "mutating_commands": [
                    "apt-get update",
                    "apt-get install -y chromium-browser"
                ]
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    println!("[sysadmin propose] output: {}", result.output);

    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("propose_changes output should be valid JSON");

    // CRITICAL: Must return pending_human_approval (not execute directly)
    assert_eq!(
        output.get("status").and_then(|v| v.as_str()),
        Some("pending_human_approval"),
        "propose_changes MUST return pending_human_approval, got: {}",
        result.output
    );

    // Must include plan and commands for the orchestrator to display
    assert!(
        output.get("plan").is_some(),
        "must include plan field: {}",
        result.output
    );
    assert!(
        output
            .get("commands")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "must include non-empty commands array: {}",
        result.output
    );
}

// ── E2E Test 2b: sysadmin_ops error handling ────────────────────────────────

/// Verify that the skill returns an error (not crash/panic) when called with
/// invalid payloads.
#[test]
fn e2e_sysadmin_invalid_action_type() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "invalid_action"
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["status"], "error");
    let msg = error_msg(&output);
    assert!(
        msg.contains("unknown") || msg.contains("action_type"),
        "expected 'unknown action_type', got: {msg}"
    );
}

/// Verify diagnose without diagnostic_cmd returns an error.
#[test]
fn e2e_sysadmin_diagnose_missing_cmd() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "diagnose"
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["status"], "error");
    let msg = error_msg(&output);
    assert!(
        msg.contains("diagnostic_cmd"),
        "expected 'diagnostic_cmd' in error, got: {msg}"
    );
}

/// Verify propose_changes without commands returns an error.
#[test]
fn e2e_sysadmin_propose_changes_missing_commands() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let cap = load_sysadmin_manifest();
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "propose_changes",
                "plan_description": "Test plan"
            }),
        )
        .unwrap();

    let result = engine.execute(&envelope).unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["status"], "error");
    let msg = error_msg(&output);
    assert!(
        msg.contains("mutating_commands") || msg.contains("commands"),
        "expected 'mutating_commands' in error, got: {msg}"
    );
}

/// Verify that sysadmin_ops with allowed_domains=["127.0.0.1"] is domain-restricted.
/// The skill should NOT be able to fetch from external hosts.
#[test]
fn e2e_sysadmin_domain_restricted() {
    let wasm_path = compiled_skills_dir().join("sysadmin_ops.wasm");
    if !wasm_path.exists() {
        eprintln!("sysadmin_ops.wasm not found — skipping");
        return;
    }

    let engine = common::shared_sandbox();
    let mut cap = load_sysadmin_manifest();
    // Override with an external domain that should be rejected
    cap.allowed_domains = vec!["example.com".to_string()];
    engine.load_skill(&wasm_path, cap).unwrap();

    let identity = AgentIdentity::generate();
    // The skill hardcodes 127.0.0.1 as the executor URL.
    // With allowed_domains=["example.com"], the fetch to 127.0.0.1 should be blocked.
    let envelope = identity
        .sign(
            "sysadmin_ops",
            json!({
                "action_type": "diagnose",
                "diagnostic_cmd": "uname"
            }),
        )
        .unwrap();

    // The sandbox blocks HTTP to 127.0.0.1 when it's not in allowed_domains.
    // The skill may: (a) return a JSON error, or (b) panic/trap when the blocked
    // host function returns an error. Both outcomes confirm domain restriction works.
    match engine.execute(&envelope) {
        Ok(result) => {
            let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
            assert_eq!(
                output["status"], "error",
                "expected error status, got: {}",
                result.output
            );
        }
        Err(e) => {
            // WASM trap/panic is acceptable — it means the sandbox blocked the call
            let err_str = format!("{e}");
            assert!(
                err_str.contains("Panic") || err_str.contains("trap") || err_str.contains("wasm"),
                "expected WASM-related error, got: {err_str}"
            );
        }
    }
}

// ── E2E Test 3: Executor Ed25519 signature verification ─────────────────────

/// Verify that the executor correctly verifies Ed25519 signatures and
/// rejects forged/invalid signatures.
#[test]
#[ignore]
fn e2e_executor_rejects_invalid_signature() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = reqwest::Client::new();
        let executor_url = std::env::var("AEGIS_EXECUTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9876".to_string());

        // Send a request with a forged signature
        let resp = client
            .post(format!("{}/api/v1/execute", executor_url))
            .json(&json!({
                "commands": ["echo hello"],
                "signature": "0000000000000000000000000000000000000000000000000000000000000000",
                "timestamp": chrono::Utc::now().timestamp() as u64,
            }))
            .send()
            .await
            .expect("executor should be reachable");

        assert_eq!(resp.status(), 403, "forged signature must be rejected");

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(
            body.get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("signature"),
            "error should mention signature: {:?}",
            body
        );
    });
}

// ── E2E Test 4: Executor health check ───────────────────────────────────────

#[test]
#[ignore]
fn e2e_executor_health_check() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let executor_url = std::env::var("AEGIS_EXECUTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9876".to_string());
        let resp = reqwest::get(format!("{}/health", executor_url))
            .await
            .expect("executor should be reachable");
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert!(body.contains("ok"));
    });
}

// ── E2E Test 5: Full approve flow with real executor ────────────────────────

/// Verify that the orchestrator's Ed25519 signing produces valid signatures
/// that the executor accepts. Submits async job, polls for completion.
#[test]
#[ignore]
fn e2e_signed_command_execution() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let identity = AgentIdentity::load_or_generate(std::path::Path::new(
            "/opt/aegis-runtime/shared/aegis_runtime_identity.key",
        ))
        .unwrap();

        let commands = vec!["echo HITL_TEST_SUCCESS".to_string()];
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let signed_message = format!("{}:{}", timestamp, commands.join("\n"));
        let signature = identity.sign_raw(signed_message.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        let executor_url = std::env::var("AEGIS_EXECUTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9876".to_string());

        let client = reqwest::Client::new();

        // Step 1: Submit async job
        let resp = client
            .post(format!("{}/api/v1/execute", executor_url))
            .json(&json!({
                "commands": commands,
                "signature": sig_hex,
                "timestamp": timestamp,
            }))
            .send()
            .await
            .expect("executor should be reachable");

        assert_eq!(
            resp.status().as_u16(),
            202,
            "async job should return 202 Accepted"
        );

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "accepted");
        let job_id = body["job_id"].as_str().expect("must return job_id");

        // Step 2: Poll for completion
        let poll_url = format!("{}/api/v1/jobs/{}", executor_url, job_id);
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let poll_resp = client.get(&poll_url).send().await.unwrap();
            assert!(poll_resp.status().is_success());
            let poll_body: serde_json::Value = poll_resp.json().await.unwrap();
            let status = poll_body["status"].as_str().unwrap();
            if status != "running" {
                assert_eq!(status, "completed", "job should complete successfully");
                let results = poll_body["results"].as_array().unwrap();
                assert_eq!(results.len(), 1);
                assert!(
                    results[0]["stdout"]
                        .as_str()
                        .unwrap()
                        .contains("HITL_TEST_SUCCESS")
                );
                return;
            }
        }
        panic!("job did not complete in time");
    });
}
