use super::*;
use crate::models::SkillCapability;
use crate::settings::RuntimeSettings;
use serde_json::Value;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shared SandboxEngine for all chronos tests — avoids creating dozens of
/// Wasmtime engines + rayon thread pools which causes test binary hangs.
static SHARED_SANDBOX: LazyLock<Arc<SandboxEngine>> =
    LazyLock::new(|| Arc::new(SandboxEngine::new().expect("shared test sandbox")));

fn shared_sandbox() -> Arc<SandboxEngine> {
    SHARED_SANDBOX.clone()
}

fn make_probe_skill_wat(require_attention: bool, event_payload: Option<&str>) -> String {
    let json = serde_json::to_string(&serde_json::json!({
        "require_llm_attention": require_attention,
        "event_payload": event_payload,
    }))
    .expect("probe signal serializes");

    let json_bytes: Vec<String> = json.bytes().map(|b| format!("\\{b:02x}")).collect();
    let json_escaped = json_bytes.join("");

    format!(
        r#"(module
                (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
                (import "aegis" "host_write_output" (func $write (param i32 i32)))
                (import "aegis" "host_get_payload_len" (func $plen (result i32)))
                (import "aegis" "host_get_payload" (func $pget (param i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "{json_escaped}")
                (func (export "run")
                    (call $write (i32.const 0) (i32.const {json_len}))
                )
            )"#,
        json_escaped = json_escaped,
        json_len = json.len(),
    )
}

fn default_cap(name: &str) -> SkillCapability {
    SkillCapability {
        skill_name: name.to_string(),
        max_memory_mb: 2,
        allowed_domains: vec![],
        allowed_fs_paths: vec![],
        cron_schedule: Some("0 */5 * * * *".to_string()),
        description: "heartbeat probe".to_string(),
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
}

/// Mock handler that tracks call counts for all three callbacks.
struct MockHandler {
    alert_count: AtomicUsize,
    silent_count: AtomicUsize,
    observation_count: AtomicUsize,
    last_observation_was_alert: Mutex<Option<bool>>,
}

impl MockHandler {
    fn new() -> Self {
        Self {
            alert_count: AtomicUsize::new(0),
            silent_count: AtomicUsize::new(0),
            observation_count: AtomicUsize::new(0),
            last_observation_was_alert: Mutex::new(None),
        }
    }
}

impl HeartbeatHandler for MockHandler {
    fn on_alert(
        &self,
        skill_name: &str,
        probe: &ProbeSignal,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        self.alert_count.fetch_add(1, Ordering::SeqCst);
        let msg = format!(
            "ALERT from {}: {}",
            skill_name,
            probe.event_payload.as_deref().unwrap_or("no details")
        );
        Box::pin(async move { Ok(msg) })
    }

    fn on_silent(&self, _skill_name: &str, _probe: &ProbeSignal) {
        self.silent_count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_observation(&self, _skill_name: &str, _probe: &ProbeSignal, was_alert: bool) {
        self.observation_count.fetch_add(1, Ordering::SeqCst);
        *self.last_observation_was_alert.lock() = Some(was_alert);
    }
}

// T9: Wasm returns require_llm_attention: false → 0 LLM calls, 0 Telegram pushes
#[tokio::test]
async fn t9_chronos_silent_drop_saves_tokens() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(false, None);
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("silent_probe"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("silent_probe", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    // Must be Silent outcome
    assert!(
        matches!(outcome, HeartbeatOutcome::Silent { .. }),
        "expected Silent, got {outcome:?}"
    );

    // 0 LLM calls (on_alert never called)
    assert_eq!(
        handler.alert_count.load(Ordering::SeqCst),
        0,
        "silent probe must trigger 0 LLM calls"
    );

    // on_silent was called exactly once
    assert_eq!(
        handler.silent_count.load(Ordering::SeqCst),
        1,
        "silent probe must call on_silent exactly once"
    );
}

#[tokio::test]
async fn heartbeat_success_resets_stale_skill_circuit_breaker() {
    let sandbox = Arc::new(SandboxEngine::new().expect("sandbox"));
    let wat = make_probe_skill_wat(false, None);
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("recovering_probe"))
        .unwrap();

    let memory = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");
    for _ in 0..3 {
        memory
            .update_skill_metrics(
                "recovering_probe",
                false,
                100.0,
                Some("old fuel exhaustion"),
                Some("resource"),
            )
            .await;
    }
    let tripped = memory
        .get_skill_metrics("recovering_probe")
        .await
        .unwrap()
        .expect("metrics exist");
    assert!(tripped.circuit_open, "test setup should open the circuit");

    let mut chronos = ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    chronos.set_memory_for_tests(memory.clone());
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("recovering_probe", &serde_json::json!({}), &handler)
        .await
        .unwrap();
    assert!(
        matches!(outcome, HeartbeatOutcome::Silent { .. }),
        "successful probe should still return its normal outcome"
    );

    let recovered = memory
        .get_skill_metrics("recovering_probe")
        .await
        .unwrap()
        .expect("metrics still exist");
    assert_eq!(recovered.consecutive_failures, 0);
    assert!(
        !recovered.circuit_open,
        "a valid heartbeat probe proves the skill runtime recovered"
    );
}

// T10: Wasm returns require_llm_attention: true → exactly 1 LLM call + 1 push
#[tokio::test]
async fn t10_chronos_alert_escalation_triggers_push() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("BTC dropped below $50000!"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("alert_probe"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("alert_probe", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    // Must be Escalated outcome
    match &outcome {
        HeartbeatOutcome::Escalated {
            alert_message,
            skill_name,
            ..
        } => {
            assert_eq!(skill_name, "alert_probe");
            assert!(
                alert_message.contains("BTC dropped below"),
                "alert must contain event payload: {alert_message}"
            );
        }
        other => panic!("expected Escalated, got {other:?}"),
    }

    // Exactly 1 LLM call (on_alert called once)
    assert_eq!(
        handler.alert_count.load(Ordering::SeqCst),
        1,
        "alert probe must trigger exactly 1 LLM call"
    );

    // 0 silent calls
    assert_eq!(
        handler.silent_count.load(Ordering::SeqCst),
        0,
        "alert probe must NOT call on_silent"
    );
}

#[test]
fn probe_returns_valid_heartbeat_result() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(false, Some("all normal"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("health_check"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let result = chronos
        .probe("health_check", &serde_json::json!({}))
        .unwrap();

    assert_eq!(result.skill_name, "health_check");
    assert!(!result.probe.require_llm_attention);
    assert_eq!(result.probe.event_payload.as_deref(), Some("all normal"));
}

#[test]
fn probe_unregistered_skill_errors() {
    let sandbox = shared_sandbox();
    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );

    let result = chronos.probe("nonexistent", &serde_json::json!({}));
    assert!(result.is_err());
}

#[tokio::test]
async fn multiple_silent_probes_zero_escalations() {
    let sandbox = shared_sandbox();

    // Load 3 silent probes
    for i in 0..3 {
        let wat = make_probe_skill_wat(false, None);
        let mut cap = default_cap(&format!("probe_{i}"));
        cap.cron_schedule = None;
        sandbox.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();
    }

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    for i in 0..3 {
        chronos
            .run_heartbeat(&format!("probe_{i}"), &serde_json::json!({}), &handler)
            .await
            .unwrap();
    }

    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.silent_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn disabled_setting_skips_probe_entirely() {
    let sandbox = shared_sandbox();
    // Load a skill that would escalate if actually probed
    let wat = make_probe_skill_wat(true, Some("should never fire"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("crypto_tracker"))
        .unwrap();

    let settings = Arc::new(RuntimeSettings::new());
    // Register and disable the skill toggle
    settings.register_skill("crypto_tracker", "crypto alerts", true);
    settings
        .set("skill_crypto_tracker", serde_json::Value::Bool(false))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        settings,
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("crypto_tracker", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    // Must be Silent with 0ms (gated, no probe executed)
    match &outcome {
        HeartbeatOutcome::Silent { elapsed_ms, .. } => {
            assert_eq!(*elapsed_ms, 0, "gated skip should have 0ms elapsed");
        }
        other => panic!("expected Silent (gated), got {other:?}"),
    }

    // No handler calls at all (including observation — probe was gated)
    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.silent_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.observation_count.load(Ordering::SeqCst), 0);
}

// ── on_observation callback tests ────────────────────────────────────

#[tokio::test]
async fn on_observation_called_for_silent_probe() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(false, Some("all clear"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("obs_silent"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    chronos
        .run_heartbeat("obs_silent", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    assert_eq!(
        handler.observation_count.load(Ordering::SeqCst),
        1,
        "on_observation must be called once for silent probe"
    );
    let was_alert = handler.last_observation_was_alert.lock();
    assert_eq!(
        *was_alert,
        Some(false),
        "was_alert should be false for silent probe"
    );
}

#[tokio::test]
async fn on_observation_called_for_alert_probe() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("critical alert"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("obs_alert"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    chronos
        .run_heartbeat("obs_alert", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    assert_eq!(
        handler.observation_count.load(Ordering::SeqCst),
        1,
        "on_observation must be called once for alert probe"
    );
    let was_alert = handler.last_observation_was_alert.lock();
    assert_eq!(
        *was_alert,
        Some(true),
        "was_alert should be true for alert probe"
    );
}

#[tokio::test]
async fn on_observation_accumulates_across_probes() {
    let sandbox = shared_sandbox();
    for i in 0..3 {
        let wat = make_probe_skill_wat(false, Some(&format!("event-{i}")));
        let mut cap = default_cap(&format!("multi_obs_{i}"));
        cap.cron_schedule = None;
        sandbox.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();
    }

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    for i in 0..3 {
        chronos
            .run_heartbeat(&format!("multi_obs_{i}"), &serde_json::json!({}), &handler)
            .await
            .unwrap();
    }

    assert_eq!(
        handler.observation_count.load(Ordering::SeqCst),
        3,
        "on_observation must be called once per heartbeat"
    );
}

// ── Alert cooldown tests ─────────────────────────────────────────────

/// Helper to build a ChronosEngine with default sandbox (for cooldown_action unit tests).
fn make_chronos_for_cooldown() -> ChronosEngine {
    ChronosEngine::new(
        shared_sandbox(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    )
    // Production waits 15 min before a ✅; these unit tests exercise the
    // transitions, not the clock.
    .with_resolve_debounce(Duration::ZERO)
}

#[tokio::test]
async fn first_alert_sends_immediately() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("disk 90%"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("cd_alert"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("cd_alert", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    assert!(
        matches!(outcome, HeartbeatOutcome::Escalated { .. }),
        "first alert must be Escalated, got {outcome:?}"
    );
    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 1);
}

#[test]
fn cooldown_first_alert_returns_send() {
    let chronos = make_chronos_for_cooldown();
    // Use a skill not in default_alert_policies so it gets AlertPolicy::default()
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
}

#[test]
fn cooldown_repeat_alert_within_window_returns_suppress() {
    let chronos = make_chronos_for_cooldown();

    // First alert → Send
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Second alert → Suppress (within 4h default cooldown)
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Suppress
    ));
}

#[test]
fn cooldown_repeat_alert_after_expiry_returns_reminder() {
    let chronos = make_chronos_for_cooldown().with_alert_cooldown(Duration::from_secs(0));

    // First alert → Send
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Second alert — cooldown=0 means it's always expired
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Reminder
    ));
}

#[test]
fn cooldown_alert_to_silent_returns_resolved() {
    let chronos = make_chronos_for_cooldown();

    // Alert → Send
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Silent → Resolved (default policy has send_resolved=true)
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, false),
        CooldownAction::Resolved
    ));
}

#[test]
fn cooldown_silent_to_silent_returns_silent() {
    let chronos = make_chronos_for_cooldown();

    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, false),
        CooldownAction::Silent
    ));
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, false),
        CooldownAction::Silent
    ));
}

#[test]
fn cooldown_flap_inside_cooldown_is_one_incident() {
    // 🚨 → "recovered" → 🚨 again within the cooldown: no second alert, and
    // the ✅ waits for the resolve debounce (production 2026-09-12: 14
    // alert/recovery pairs per day around a 70% disk threshold).
    let chronos = make_chronos_for_cooldown()
        .with_resolve_debounce(Duration::from_secs(3600))
        .with_alert_cooldown(Duration::from_secs(7200));
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Warning, false),
        CooldownAction::Silent
    ));
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Warning, true),
        CooldownAction::Suppress
    ));
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Warning, false),
        CooldownAction::Silent
    ));
}

#[test]
fn owner_quiet_hours_hold_non_critical_probe_alerts() {
    // heartbeat_config quiet hours covering the whole day in the owner's zone:
    // Warning is held, Critical still goes out.
    let settings = RuntimeSettings::new();
    let mut cfg = crate::heartbeat_config::HeartbeatConfig::default();
    cfg.quiet_hours.start = "00:00".to_string();
    cfg.quiet_hours.end = "23:59".to_string();
    settings
        .set("heartbeat_config", serde_json::to_value(&cfg).unwrap())
        .expect("set heartbeat_config");
    settings
        .set("user_timezone", serde_json::json!("America/Los_Angeles"))
        .expect("set tz");
    let chronos = ChronosEngine::new(
        shared_sandbox(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(settings),
    )
    .with_resolve_debounce(Duration::ZERO)
    .with_owner_quiet_hours(true);
    assert!(chronos.owner_quiet_hours_now());
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Warning, true),
        CooldownAction::Silent
    ));
    assert!(matches!(
        chronos.cooldown_action("disk", AlertSeverity::Critical, true),
        CooldownAction::Send
    ));
}

#[test]
fn cooldown_independent_per_skill() {
    let chronos = make_chronos_for_cooldown();

    // Skill A alerts
    assert!(matches!(
        chronos.cooldown_action("skill_a", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Skill B also gets its own first alert
    assert!(matches!(
        chronos.cooldown_action("skill_b", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Skill A repeat — suppressed
    assert!(matches!(
        chronos.cooldown_action("skill_a", AlertSeverity::Warning, true),
        CooldownAction::Suppress
    ));
    // Skill B repeat — suppressed
    assert!(matches!(
        chronos.cooldown_action("skill_b", AlertSeverity::Warning, true),
        CooldownAction::Suppress
    ));
}

#[test]
fn cooldown_resolved_then_new_alert_sends() {
    // With the cooldown elapsed (zero here) a fresh alert after a recovery is
    // a new incident → Send. Inside the cooldown it is the same incident and
    // is suppressed (see cooldown_flap_inside_cooldown_is_one_incident).
    let chronos = make_chronos_for_cooldown().with_alert_cooldown(Duration::ZERO);

    // Alert → Send
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Resolve
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, false),
        CooldownAction::Resolved
    ));
    // New alert → Send again (not suppress — it was resolved)
    assert!(matches!(
        chronos.cooldown_action("test_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
}

// ── AlertPolicy tests ───────────────────────────────────────────────

#[test]
fn test_alert_policy_default() {
    let policy = AlertPolicy::default();
    assert_eq!(policy.delivery, DeliveryMode::WarningAndAbove);
    assert!(policy.send_resolved);
    assert_eq!(policy.cooldown_secs, 0);
    assert!(policy.quiet_hours.is_none());
    // Default delivers Warning and above
    assert!(policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(policy.should_deliver(AlertSeverity::Critical, 12));
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
}

#[test]
fn test_alert_policy_critical_only() {
    let policy = AlertPolicy {
        delivery: DeliveryMode::CriticalOnly,
        ..Default::default()
    };
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
    assert!(!policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(policy.should_deliver(AlertSeverity::Critical, 12));
}

#[test]
fn test_alert_policy_silent() {
    let policy = AlertPolicy {
        delivery: DeliveryMode::Silent,
        ..Default::default()
    };
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
    assert!(!policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(!policy.should_deliver(AlertSeverity::Critical, 12));
}

#[test]
fn test_alert_policy_send_resolved_false() {
    let chronos = make_chronos_for_cooldown();
    // Set a policy with send_resolved=false for the test skill
    chronos.set_policy(
        "no_resolve_skill",
        AlertPolicy {
            send_resolved: false,
            ..Default::default()
        },
    );

    // Alert → Send
    assert!(matches!(
        chronos.cooldown_action("no_resolve_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Transition to silent → should be Silent (not Resolved) because send_resolved=false
    assert!(matches!(
        chronos.cooldown_action("no_resolve_skill", AlertSeverity::Warning, false),
        CooldownAction::Silent
    ));
}

#[test]
fn test_alert_policy_quiet_hours() {
    let policy = AlertPolicy {
        quiet_hours: Some((9, 17)), // 9am-5pm UTC
        ..Default::default()
    };
    // During quiet hours, only Critical passes
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
    assert!(!policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(policy.should_deliver(AlertSeverity::Critical, 12));
    // Outside quiet hours, WarningAndAbove passes
    assert!(policy.should_deliver(AlertSeverity::Warning, 20));
    assert!(!policy.should_deliver(AlertSeverity::Info, 20));
}

#[test]
fn test_alert_policy_quiet_hours_overnight() {
    let policy = AlertPolicy {
        quiet_hours: Some((22, 6)), // 10pm-6am UTC
        ..Default::default()
    };
    // In range
    assert!(policy.in_quiet_hours(23));
    assert!(policy.in_quiet_hours(0));
    assert!(policy.in_quiet_hours(3));
    assert!(policy.in_quiet_hours(5));
    // Out of range
    assert!(!policy.in_quiet_hours(6));
    assert!(!policy.in_quiet_hours(12));
    assert!(!policy.in_quiet_hours(21));
    // During overnight quiet hours, only Critical passes
    assert!(!policy.should_deliver(AlertSeverity::Warning, 23));
    assert!(policy.should_deliver(AlertSeverity::Critical, 23));
}

#[test]
fn test_alert_policy_per_skill_cooldown() {
    // Reuse single chronos instance to avoid multiple SandboxEngine inits
    let chronos = make_chronos_for_cooldown();

    // Test 1: per-skill cooldown of 1 second
    chronos.set_policy(
        "fast_skill",
        AlertPolicy {
            cooldown_secs: 1,
            ..Default::default()
        },
    );
    assert!(matches!(
        chronos.cooldown_action("fast_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));

    // Test 2: skill with cooldown_secs=0 uses global default (4h)
    chronos.set_policy(
        "instant_skill",
        AlertPolicy {
            cooldown_secs: 0, // use global default
            ..Default::default()
        },
    );
    assert!(matches!(
        chronos.cooldown_action("instant_skill", AlertSeverity::Warning, true),
        CooldownAction::Send
    ));
    // Global default is 4h, so repeat within seconds → Suppress
    assert!(matches!(
        chronos.cooldown_action("instant_skill", AlertSeverity::Warning, true),
        CooldownAction::Suppress
    ));
}

#[test]
fn test_severity_backward_compat() {
    // ProbeSignal without severity field deserializes as Warning
    let json = r#"{"require_llm_attention": true, "event_payload": "disk full"}"#;
    let signal: ProbeSignal = serde_json::from_str(json).unwrap();
    assert_eq!(signal.severity, AlertSeverity::Warning);
}

#[test]
fn test_should_deliver_all_mode() {
    let policy = AlertPolicy {
        delivery: DeliveryMode::All,
        ..Default::default()
    };
    assert!(policy.should_deliver(AlertSeverity::Info, 12));
    assert!(policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(policy.should_deliver(AlertSeverity::Critical, 12));
}

#[test]
fn test_effective_cooldown() {
    let global = Duration::from_secs(14400); // 4 hours
    let policy_default = AlertPolicy::default();
    assert_eq!(policy_default.effective_cooldown(global), global);

    let policy_custom = AlertPolicy {
        cooldown_secs: 3600,
        ..Default::default()
    };
    assert_eq!(
        policy_custom.effective_cooldown(global),
        Duration::from_secs(3600)
    );
}

#[test]
fn test_policy_crud() {
    let chronos = make_chronos_for_cooldown();

    // Default policies are loaded
    let policies = chronos.list_policies();
    assert!(policies.contains_key("server_monitor"));
    assert!(policies.contains_key("commute_traffic_monitor"));

    // Get policy for unknown skill → default
    let unknown = chronos.get_policy("unknown_skill");
    assert_eq!(unknown.delivery, DeliveryMode::WarningAndAbove);
    assert!(unknown.send_resolved);

    // Set custom policy
    chronos.set_policy(
        "custom_skill",
        AlertPolicy {
            delivery: DeliveryMode::CriticalOnly,
            send_resolved: false,
            ..Default::default()
        },
    );
    let custom = chronos.get_policy("custom_skill");
    assert_eq!(custom.delivery, DeliveryMode::CriticalOnly);
    assert!(!custom.send_resolved);

    // Remove → reverts to default
    chronos.remove_policy("custom_skill");
    let reverted = chronos.get_policy("custom_skill");
    assert_eq!(reverted.delivery, DeliveryMode::WarningAndAbove);
}

#[tokio::test]
async fn resolved_notification_on_transition() {
    // Pre-seed alert state, then probe a silent skill to test the resolved path.
    let sandbox = shared_sandbox();
    let wat_silent = make_probe_skill_wat(false, Some("disk normal"));
    sandbox
        .load_skill_from_bytes(wat_silent.as_bytes(), default_cap("cd_resolved_silent"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    )
    .with_resolve_debounce(Duration::ZERO);

    // Pre-seed: simulate that this skill was previously alerting
    {
        let mut state = chronos.alert_state.lock();
        state.insert(
            "cd_resolved_silent".to_string(),
            AlertState {
                last_alert_sent: Instant::now(),
                was_alerting: true,
                resolve_pending: false,
            },
        );
    }

    let handler = MockHandler::new();
    let outcome = chronos
        .run_heartbeat("cd_resolved_silent", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    match &outcome {
        HeartbeatOutcome::Escalated { alert_message, .. } => {
            assert!(
                alert_message.contains("[RESOLVED]"),
                "resolved notification must contain [RESOLVED]: {alert_message}"
            );
        }
        other => panic!("expected Escalated (resolved), got {other:?}"),
    }
    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 1);
    // on_observation should also be called
    assert_eq!(handler.observation_count.load(Ordering::SeqCst), 1);
}

// ── spawn_blocking non-regression tests ──────────────────────────────

#[tokio::test]
async fn run_heartbeat_does_not_block_async_worker() {
    let sandbox = shared_sandbox();

    // Two independent skills that produce silent probes (fast execution).
    for name in &["nb_skill_a", "nb_skill_b"] {
        let wat = make_probe_skill_wat(false, None);
        let mut cap = default_cap(name);
        cap.cron_schedule = None;
        sandbox.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();
    }

    let identity = std::sync::Arc::new(AgentIdentity::generate());
    let settings = std::sync::Arc::new(RuntimeSettings::new());

    let chronos_a = std::sync::Arc::new(ChronosEngine::new(
        Arc::clone(&sandbox),
        Arc::clone(&identity),
        Arc::clone(&settings),
    ));
    let chronos_b = std::sync::Arc::new(ChronosEngine::new(
        Arc::clone(&sandbox),
        Arc::clone(&identity),
        settings,
    ));
    let handler = std::sync::Arc::new(MockHandler::new());

    let empty_payload_a = serde_json::json!({});
    let empty_payload_b = serde_json::json!({});
    let (r_a, r_b) = tokio::join!(
        chronos_a.run_heartbeat("nb_skill_a", &empty_payload_a, handler.as_ref()),
        chronos_b.run_heartbeat("nb_skill_b", &empty_payload_b, handler.as_ref()),
    );

    assert!(r_a.is_ok(), "skill_a heartbeat failed: {:?}", r_a);
    assert!(r_b.is_ok(), "skill_b heartbeat failed: {:?}", r_b);

    // Both must be silent
    assert!(matches!(r_a.unwrap(), HeartbeatOutcome::Silent { .. }));
    assert!(matches!(r_b.unwrap(), HeartbeatOutcome::Silent { .. }));
}

#[tokio::test]
async fn run_heartbeat_timeout_returns_silent_not_error() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("would alert"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("timeout_probe"))
        .unwrap();

    let chronos = Arc::new(ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    ));

    let handler = MockHandler::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_micros(1),
        chronos.run_heartbeat("timeout_probe", &serde_json::json!({}), &handler),
    )
    .await;

    match outcome {
        Err(_elapsed) => {
            // Timeout path taken — expected outcome for near-zero deadline.
        }
        Ok(Ok(HeartbeatOutcome::Silent { .. })) => {
            // Probe completed before our external 1us deadline (fast CI).
        }
        Ok(Ok(other)) => {
            let _ = other;
        }
        Ok(Err(e)) => {
            panic!("run_heartbeat returned unexpected error: {e}");
        }
    }
}

#[tokio::test]
async fn unloaded_skill_heartbeat_skipped() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("should never fire"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("phantom_skill"))
        .unwrap();
    sandbox.unload_skill("phantom_skill");

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("phantom_skill", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    match &outcome {
        HeartbeatOutcome::Silent { elapsed_ms, .. } => {
            assert_eq!(*elapsed_ms, 0, "unloaded skip should have 0ms elapsed");
        }
        other => panic!("expected Silent (unloaded), got {other:?}"),
    }

    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.silent_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.observation_count.load(Ordering::SeqCst), 0);
}

// ── Semantic triage tests ──────────────────────────────────────────────

#[tokio::test]
async fn triage_no_intent_passes_through() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("BTC dropped below $50000!"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("triage_pass"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("triage_pass", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    assert!(
        matches!(outcome, HeartbeatOutcome::Escalated { .. }),
        "no intent = all alerts pass through, got {outcome:?}"
    );
    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn triage_filters_low_relevance_alert() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("routine health check OK"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("triage_filter"))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );

    let intent_embedding = vec![1.0; 128];
    chronos.cache_intent("did:test:user", "triage_filter", intent_embedding, 0.99);

    let handler = MockHandler::new();
    let outcome = chronos
        .run_heartbeat("triage_filter", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    // No embedder → triage skipped → passes through
    assert!(
        matches!(outcome, HeartbeatOutcome::Escalated { .. }),
        "no embedder = triage skipped (fail-open), got {outcome:?}"
    );
    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 1);
}

#[test]
fn triage_cache_operations() {
    let chronos = ChronosEngine::new(
        shared_sandbox(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );

    // Initially empty
    assert!(chronos.intent_cache.read().is_empty());

    // Cache an intent for one user
    let embedding = vec![1.0, 2.0, 3.0];
    chronos.cache_intent("did:test:alice", "crypto_tracker", embedding.clone(), 0.7);
    {
        let cache = chronos.intent_cache.read();
        assert_eq!(cache.len(), 1);
        let (cached_emb, cached_threshold) = cache
            .get(&("did:test:alice".to_string(), "crypto_tracker".to_string()))
            .unwrap();
        assert_eq!(cached_emb, &embedding);
        assert!((cached_threshold - 0.7).abs() < f32::EPSILON);
    }

    // Same skill for another user must coexist, not overwrite Alice.
    let new_embedding = vec![4.0, 5.0, 6.0];
    chronos.cache_intent("did:test:bob", "crypto_tracker", new_embedding.clone(), 0.8);
    {
        let cache = chronos.intent_cache.read();
        assert_eq!(cache.len(), 2);
        let (cached_emb, cached_threshold) = cache
            .get(&("did:test:bob".to_string(), "crypto_tracker".to_string()))
            .unwrap();
        assert_eq!(cached_emb, &new_embedding);
        assert!((cached_threshold - 0.8).abs() < f32::EPSILON);
    }
    let (alice_emb, alice_threshold) = chronos
        .cached_intent("did:test:alice", "crypto_tracker")
        .expect("alice cache entry");
    assert_eq!(alice_emb, embedding);
    assert!((alice_threshold - 0.7).abs() < f32::EPSILON);

    // Uncache only Alice
    chronos.uncache_intent("did:test:alice", "crypto_tracker");
    {
        let cache = chronos.intent_cache.read();
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key(&("did:test:bob".to_string(), "crypto_tracker".to_string())));
    }

    // Uncache nonexistent — no panic
    chronos.uncache_intent("did:test:alice", "nonexistent");
    chronos.uncache_intent("did:test:bob", "crypto_tracker");
    assert!(chronos.intent_cache.read().is_empty());
}

#[test]
fn monitoring_intent_owner_did_prefers_user_did_then_chat_id() {
    let payload = serde_json::json!({
        "user_did": "did:test:alice",
        "chat_id": 12345i64
    });
    assert_eq!(
        ChronosEngine::monitoring_intent_owner_did(&payload).as_deref(),
        Some("did:test:alice")
    );

    let payload = serde_json::json!({ "chat_id": 12345i64 });
    assert_eq!(
        ChronosEngine::monitoring_intent_owner_did(&payload).as_deref(),
        Some("did:aegis:telegram:12345")
    );

    let payload = serde_json::json!({});
    assert!(
        ChronosEngine::monitoring_intent_owner_did(&payload).is_none(),
        "missing identity should not guess a monitoring-intent owner"
    );
}

#[tokio::test]
async fn refresh_intent_cache_keeps_same_skill_separate_per_user() {
    let memory = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");
    let mut chronos = ChronosEngine::new(
        shared_sandbox(),
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    chronos.set_memory_for_tests(memory.clone());

    memory
        .store_monitoring_intent(
            "did:test:alice",
            "calendar_reader",
            "Alice monitor",
            0.7,
            vec![1.0, 0.0, 0.0],
        )
        .await
        .expect("store alice intent");
    memory
        .store_monitoring_intent(
            "did:test:bob",
            "calendar_reader",
            "Bob monitor",
            0.8,
            vec![0.0, 1.0, 0.0],
        )
        .await
        .expect("store bob intent");

    chronos.refresh_intent_cache("did:test:unused").await;

    let (alice_emb, alice_threshold) = chronos
        .cached_intent("did:test:alice", "calendar_reader")
        .expect("alice cached");
    assert_eq!(alice_emb, vec![1.0, 0.0, 0.0]);
    assert!((alice_threshold - 0.7).abs() < f32::EPSILON);

    let (bob_emb, bob_threshold) = chronos
        .cached_intent("did:test:bob", "calendar_reader")
        .expect("bob cached");
    assert_eq!(bob_emb, vec![0.0, 1.0, 0.0]);
    assert!((bob_threshold - 0.8).abs() < f32::EPSILON);
}
#[tokio::test]
async fn disabled_skill_heartbeat_skipped_without_wasm_execution() {
    let sandbox = shared_sandbox();
    let wat = make_probe_skill_wat(true, Some("should never fire — skill disabled"));
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("disabled_skill"))
        .unwrap();

    let settings = Arc::new(RuntimeSettings::new());
    settings.register_skill("disabled_skill", "test skill", true);
    settings
        .set("skill_disabled_skill", Value::Bool(false))
        .unwrap();

    let chronos = ChronosEngine::new(
        sandbox.clone(),
        Arc::new(AgentIdentity::generate()),
        settings,
    );
    let handler = MockHandler::new();

    let outcome = chronos
        .run_heartbeat("disabled_skill", &serde_json::json!({}), &handler)
        .await
        .unwrap();

    match &outcome {
        HeartbeatOutcome::Silent {
            skill_name,
            elapsed_ms,
        } => {
            assert_eq!(skill_name, "disabled_skill");
            assert_eq!(*elapsed_ms, 0, "disabled skip should have 0ms elapsed");
        }
        other => panic!("expected Silent (disabled), got {other:?}"),
    }

    assert_eq!(handler.alert_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.silent_count.load(Ordering::SeqCst), 0);
    assert_eq!(handler.observation_count.load(Ordering::SeqCst), 0);
}

#[test]
fn monitoring_intent_serialization() {
    let intent = MonitoringIntent {
        skill_name: "crypto_tracker".to_string(),
        description: "BTC price drops below $50k".to_string(),
        threshold: 0.7,
    };
    let json = serde_json::to_string(&intent).unwrap();
    let parsed: MonitoringIntent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.skill_name, "crypto_tracker");
    assert_eq!(parsed.description, "BTC price drops below $50k");
    assert!((parsed.threshold - 0.7).abs() < f32::EPSILON);
}

#[test]
fn muted_policy_suppresses_all_alerts() {
    let future_ts = chrono::Utc::now().timestamp() + 3600;
    let policy = AlertPolicy {
        muted_until: Some(future_ts),
        delivery: DeliveryMode::All,
        ..Default::default()
    };
    assert!(policy.is_muted());
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
    assert!(!policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(!policy.should_deliver(AlertSeverity::Critical, 12));
}

#[test]
fn expired_mute_resumes_delivery() {
    let past_ts = chrono::Utc::now().timestamp() - 10;
    let policy = AlertPolicy {
        muted_until: Some(past_ts),
        delivery: DeliveryMode::WarningAndAbove,
        ..Default::default()
    };
    assert!(!policy.is_muted());
    assert!(policy.should_deliver(AlertSeverity::Warning, 12));
    assert!(policy.should_deliver(AlertSeverity::Critical, 12));
    assert!(!policy.should_deliver(AlertSeverity::Info, 12));
}

#[test]
fn unmuted_policy_delivers_normally() {
    let policy = AlertPolicy::default();
    assert!(!policy.is_muted());
    assert!(policy.muted_until.is_none());
    assert!(policy.should_deliver(AlertSeverity::Warning, 12));
}

#[test]
fn muted_until_serialization_roundtrip() {
    let future_ts = chrono::Utc::now().timestamp() + 7200;
    let policy = AlertPolicy {
        muted_until: Some(future_ts),
        ..Default::default()
    };
    let json = serde_json::to_string(&policy).unwrap();
    let deserialized: AlertPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.muted_until, Some(future_ts));
    assert!(deserialized.is_muted());
}

#[test]
fn muted_until_absent_in_json_defaults_to_none() {
    let json = r#"{"delivery":"All","send_resolved":true,"cooldown_secs":0,"pin_messages":true}"#;
    let policy: AlertPolicy = serde_json::from_str(json).unwrap();
    assert!(policy.muted_until.is_none());
    assert!(!policy.is_muted());
}

// ── Autonomous heartbeat tests ──────────────────────────────────────

#[test]
fn autonomous_config_defaults() {
    let cfg = AutonomousConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.max_rounds, 5);
    assert_eq!(cfg.max_tool_calls, 8);
    assert!(cfg.allowed_tools.is_empty());
    assert!(cfg.denied_tools.is_empty());
    assert_eq!(cfg.daily_token_budget, 500_000);
    assert_eq!(cfg.notification_mode, NotificationMode::OnAction);
}

#[test]
fn autonomous_config_serde_roundtrip() {
    let cfg = AutonomousConfig {
        enabled: true,
        max_rounds: 5,
        max_tool_calls: 10,
        allowed_tools: vec!["web_search".to_string(), "notify_user".to_string()],
        denied_tools: vec!["send_email".to_string()],
        daily_token_budget: 100_000,
        notification_mode: NotificationMode::Always,
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let deserialized: AutonomousConfig = serde_json::from_str(&json).unwrap();
    assert!(deserialized.enabled);
    assert_eq!(deserialized.max_rounds, 5);
    assert_eq!(deserialized.max_tool_calls, 10);
    assert_eq!(deserialized.allowed_tools.len(), 2);
    assert_eq!(deserialized.denied_tools, vec!["send_email"]);
    assert_eq!(deserialized.daily_token_budget, 100_000);
    assert_eq!(deserialized.notification_mode, NotificationMode::Always);
}

#[test]
fn autonomous_config_backward_compat() {
    let cfg: AutonomousConfig = serde_json::from_str("{}").unwrap();
    assert!(!cfg.enabled);
    assert_eq!(cfg.max_rounds, 5);
    assert_eq!(cfg.max_tool_calls, 8);
    assert!(cfg.allowed_tools.is_empty());
    assert!(cfg.denied_tools.is_empty());
    assert_eq!(cfg.daily_token_budget, 500_000);
    assert_eq!(cfg.notification_mode, NotificationMode::OnAction);
}

// autonomous_outcome_serde: DELETED in Step 3 (design §15.2). AutonomousOutcome
// is now an alias for OrchestratorResult (non-serializable: holds
// NotifyUserRequest handles). Audit writes discrete columns, not the struct
// shape, so JSON roundtrip was never on the hot path.

#[test]
fn within_daily_budget_tracking() {
    let chronos = make_chronos_for_cooldown();
    let cfg = AutonomousConfig {
        daily_token_budget: 1000,
        ..Default::default()
    };

    assert!(chronos.within_daily_budget(&cfg));

    chronos.track_autonomous_tokens(500);
    assert!(chronos.within_daily_budget(&cfg));

    chronos.track_autonomous_tokens(500);
    assert!(!chronos.within_daily_budget(&cfg));

    chronos.track_autonomous_tokens(1);
    assert!(!chronos.within_daily_budget(&cfg));
}

#[test]
fn daily_budget_reset_on_date_change() {
    let chronos = make_chronos_for_cooldown();
    let cfg = AutonomousConfig {
        daily_token_budget: 100,
        ..Default::default()
    };

    chronos.track_autonomous_tokens(100);
    assert!(!chronos.within_daily_budget(&cfg));

    {
        let mut date = chronos.daily_token_reset_date.lock();
        *date = "1970-01-01".to_string();
    }

    assert!(chronos.within_daily_budget(&cfg));
    assert_eq!(chronos.daily_autonomous_tokens.load(Ordering::Relaxed), 0);
}

#[test]
fn autonomous_config_per_skill_and_global() {
    let chronos = make_chronos_for_cooldown();

    let cfg = chronos.get_autonomous_config("any_skill");
    assert!(!cfg.enabled);

    chronos.set_global_autonomous_config(AutonomousConfig {
        enabled: true,
        max_rounds: 7,
        ..Default::default()
    });
    let cfg = chronos.get_autonomous_config("any_skill");
    assert!(cfg.enabled);
    assert_eq!(cfg.max_rounds, 7);

    chronos.set_autonomous_config(
        "special_skill",
        AutonomousConfig {
            enabled: true,
            max_rounds: 1,
            max_tool_calls: 2,
            ..Default::default()
        },
    );
    let special = chronos.get_autonomous_config("special_skill");
    assert_eq!(special.max_rounds, 1);
    assert_eq!(special.max_tool_calls, 2);
    let other = chronos.get_autonomous_config("other_skill");
    assert_eq!(other.max_rounds, 7);
}

#[test]
fn default_autonomous_allowed_tools_not_empty() {
    let tools = default_autonomous_allowed_tools();
    assert!(!tools.is_empty());
    assert!(tools.contains(&"web_search".to_string()));
    assert!(tools.contains(&"notify_user".to_string()));
    assert!(tools.contains(&"server_monitor".to_string()));
}

#[test]
fn notification_mode_default_is_on_action() {
    let mode = NotificationMode::default();
    assert_eq!(mode, NotificationMode::OnAction);
}

#[test]
fn notification_mode_serde_roundtrip() {
    for mode in &[
        NotificationMode::Always,
        NotificationMode::OnAction,
        NotificationMode::Never,
    ] {
        let json = serde_json::to_string(mode).unwrap();
        let deserialized: NotificationMode = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, mode);
    }
}

#[test]
fn probe_signal_with_suggested_actions_chronos() {
    let json = r#"{
            "require_llm_attention": true,
            "event_payload": "CPU at 95%",
            "severity": "Critical",
            "suggested_actions": ["restart_nginx", "scale_up"],
            "probe_data": {"cpu": 95, "mem": 42}
        }"#;
    let signal: ProbeSignal = serde_json::from_str(json).unwrap();
    assert_eq!(signal.suggested_actions.len(), 2);
    assert_eq!(signal.suggested_actions[0], "restart_nginx");
    assert!(signal.probe_data.is_some());
    let data = signal.probe_data.unwrap();
    assert_eq!(data["cpu"].as_i64(), Some(95));
    assert_eq!(data["mem"].as_i64(), Some(42));
}

#[test]
fn probe_signal_without_new_fields_backward_compat_chronos() {
    let json = r#"{"require_llm_attention": false, "event_payload": null}"#;
    let signal: ProbeSignal = serde_json::from_str(json).unwrap();
    assert!(signal.suggested_actions.is_empty());
    assert!(signal.probe_data.is_none());
}

// ── True Heartbeat Tests ─────────────────────────────────────────────

#[test]
fn true_heartbeat_config_defaults() {
    let config = TrueHeartbeatConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.interval_minutes, 30);
    assert!(config.quiet_hours.is_none());
    assert!(config.autonomous_overrides.is_none());
}

#[test]
fn true_heartbeat_config_serde_roundtrip() {
    let config = TrueHeartbeatConfig {
        enabled: true,
        interval_minutes: 30,
        quiet_hours: None, // legacy field, skip_serializing
        autonomous_overrides: Some(AutonomousConfig {
            max_rounds: 2,
            ..AutonomousConfig::default()
        }),
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: TrueHeartbeatConfig = serde_json::from_str(&json).unwrap();
    assert!(restored.enabled);
    assert_eq!(restored.interval_minutes, 30);
    // quiet_hours is skip_serializing, so roundtrip produces None
    assert!(restored.quiet_hours.is_none());
    assert_eq!(
        restored.autonomous_overrides.as_ref().unwrap().max_rounds,
        2
    );
}

#[test]
fn true_heartbeat_config_deserializes_legacy_quiet_hours() {
    // Old DB rows may still have quiet_hours — verify backward compat deserialization
    let json = r#"{"enabled":true,"interval_minutes":30,"quiet_hours":[22,6]}"#;
    let config: TrueHeartbeatConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.quiet_hours, Some((22, 6)));
}

#[test]
fn true_heartbeat_effective_config_uses_override() {
    let fallback = AutonomousConfig {
        max_rounds: 3,
        max_tool_calls: 5,
        ..AutonomousConfig::default()
    };
    let config = TrueHeartbeatConfig {
        autonomous_overrides: Some(AutonomousConfig {
            max_rounds: 2,
            max_tool_calls: 3,
            ..AutonomousConfig::default()
        }),
        ..TrueHeartbeatConfig::default()
    };
    let effective = config.effective_autonomous_config(&fallback);
    assert_eq!(effective.max_rounds, 2);
    assert_eq!(effective.max_tool_calls, 3);
}

#[test]
fn true_heartbeat_effective_config_falls_back_to_global() {
    let fallback = AutonomousConfig {
        max_rounds: 3,
        max_tool_calls: 5,
        ..AutonomousConfig::default()
    };
    let config = TrueHeartbeatConfig::default();
    let effective = config.effective_autonomous_config(&fallback);
    assert_eq!(effective.max_rounds, 3);
    assert_eq!(effective.max_tool_calls, 5);
}

/// Helper to create a minimal Task for snapshot tests.
fn make_test_task(title: &str, priority: &str, due_date: Option<&str>) -> crate::memory::Task {
    crate::memory::Task {
        id: 1,
        chat_id: 100,
        user_did: "did:test".to_string(),
        title: title.to_string(),
        body: "".to_string(),
        priority: priority.to_string(),
        status: "active".to_string(),
        tags: vec![],
        due_date: due_date.map(|s| s.to_string()),
        source: "user".to_string(),
        trigger_at_unix: None,
        trigger_type: None,
        event_key: None,
        execution_mode: "notify".to_string(),
        context_text: None,
        interval_seconds: None,
        group_key: None,
        max_fires: None,
        fire_count: 0,
        recur_weekdays: None,
        recur_window_start: None,
        recur_window_end: None,
        recur_timezone: None,
        fired_at: None,
        retry_count: 0,
        chain_depth: 0,
        pin_on_fire: false,
        expires_at_unix: None,
        is_callback_origin: false,
        created_at: "2026-03-28T00:00:00Z".to_string(),
        updated_at: "2026-03-28T00:00:00Z".to_string(),
        completed_at: None,
    }
}

#[test]
fn snapshot_to_prompt_block_format() {
    let snapshot = SituationSnapshot {
        compiled_at: "2026-03-28T14:30:00Z".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        recent_probe_results: vec![],
        pending_tasks: vec![make_test_task(
            "Prepare presentation",
            "high",
            Some("2026-03-29"),
        )],
        upcoming_intents: vec![],
        skill_metrics: vec![crate::memory::SkillMetrics {
            skill_name: "server_monitor".to_string(),
            total_calls: 100,
            success_count: 98,
            error_count: 2,
            avg_latency_ms: 150.0,
            last_error: Some("timeout".to_string()),
            error_categories: Default::default(),
            consecutive_failures: 2,
            circuit_open: false,
        }],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![("set_alert_policy:crypto".to_string(), "pending".to_string())],
        budget_status: (12000, 50000),
        user_last_active_unix: Some(chrono::Utc::now().timestamp() - 720),
    };

    let block = snapshot.to_prompt_block();

    assert!(block.starts_with("<situation_snapshot"));
    assert!(block.ends_with("</situation_snapshot>"));
    assert!(block.contains("TOKEN BUDGET: 12000/50000"));
    assert!(block.contains("USER LAST ACTIVE:"));
    assert!(block.contains("PENDING TASKS (1)"));
    assert!(block.contains("Prepare presentation"));
    assert!(block.contains("SYSTEM HEALTH CONCERNS:"));
    assert!(block.contains("server_monitor: 2 consecutive failures"));
    assert!(block.contains("PENDING APPROVALS (1)"));
    assert!(block.contains("set_alert_policy:crypto"));
}

#[test]
fn snapshot_to_prompt_block_all_healthy() {
    let snapshot = SituationSnapshot {
        compiled_at: "2026-03-28T14:30:00Z".to_string(),
        timezone: "UTC".to_string(),
        recent_probe_results: vec![],
        pending_tasks: vec![],
        upcoming_intents: vec![],
        skill_metrics: vec![crate::memory::SkillMetrics {
            skill_name: "crypto_tracker".to_string(),
            total_calls: 50,
            success_count: 50,
            error_count: 0,
            avg_latency_ms: 100.0,
            last_error: None,
            error_categories: Default::default(),
            consecutive_failures: 0,
            circuit_open: false,
        }],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![],
        budget_status: (0, 50000),
        user_last_active_unix: None,
    };

    let block = snapshot.to_prompt_block();
    assert!(block.contains("All 1 skills healthy"));
    assert!(block.contains("USER LAST ACTIVE: unknown"));
    assert!(!block.contains("PENDING TASKS"));
    assert!(!block.contains("PENDING APPROVALS"));
    assert!(!block.contains("LAST HEARTBEAT ACTIONS"));
}

#[test]
fn shared_budget_between_probe_and_true_heartbeat() {
    let sandbox = shared_sandbox();
    let engine = ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let auto_cfg = AutonomousConfig {
        enabled: true,
        daily_token_budget: 1000,
        ..AutonomousConfig::default()
    };

    assert!(engine.within_daily_budget(&auto_cfg));
    engine.track_autonomous_tokens(600);

    assert!(engine.within_daily_budget(&auto_cfg));
    engine.track_autonomous_tokens(500);

    assert!(!engine.within_daily_budget(&auto_cfg));
}

#[test]
fn true_heartbeat_config_get_set() {
    let sandbox = shared_sandbox();
    let engine = ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    );
    let config = engine.get_true_heartbeat_config();
    assert!(!config.enabled);

    engine.set_true_heartbeat_config(TrueHeartbeatConfig {
        enabled: true,
        interval_minutes: 30,
        ..TrueHeartbeatConfig::default()
    });

    let config = engine.get_true_heartbeat_config();
    assert!(config.enabled);
    assert_eq!(config.interval_minutes, 30);
}

// ── Daily summary timezone (TDD — Bug #4) ───────────────────────────

#[test]
fn daily_summary_timezone_uses_correct_key() {
    let settings = Arc::new(RuntimeSettings::new());
    settings
        .set(
            "user_timezone",
            serde_json::Value::String("America/New_York".into()),
        )
        .unwrap();

    let tz = settings
        .get("user_timezone")
        .and_then(|v| v.as_str().map(String::from));
    assert_eq!(tz, Some("America/New_York".to_string()));

    let wrong = settings.get("timezone");
    assert!(
        wrong.is_none(),
        "\"timezone\" is not a registered key — code must use \"user_timezone\""
    );
}

// ── P1 Credit Assignment Tests ──────────────────────────────────────

#[test]
fn tool_call_detail_json_roundtrip() {
    let detail = ToolCallDetail {
        tool_name: "web_search".into(),
        round: 2,
        latency_ms: 1500,
        result_size: 4096,
        was_blocked: false,
        result_preview: "Search results for query: 最新新闻...".into(),
    };
    let json = serde_json::to_string(&detail).unwrap();
    let parsed: ToolCallDetail = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.tool_name, "web_search");
    assert_eq!(parsed.round, 2);
    assert_eq!(parsed.latency_ms, 1500);
    assert_eq!(parsed.result_size, 4096);
    assert!(!parsed.was_blocked);
    assert!(parsed.result_preview.contains("最新新闻"));
}

#[test]
fn tool_call_detail_vec_roundtrip() {
    let details = vec![
        ToolCallDetail {
            tool_name: "recall_memory".into(),
            round: 1,
            latency_ms: 50,
            result_size: 200,
            was_blocked: false,
            result_preview: "Found 3 relevant facts...".into(),
        },
        ToolCallDetail {
            tool_name: "web_reader".into(),
            round: 1,
            latency_ms: 3000,
            result_size: 8192,
            was_blocked: false,
            result_preview: "Page content: <html>...".into(),
        },
        ToolCallDetail {
            tool_name: "send_email".into(),
            round: 2,
            latency_ms: 0,
            result_size: 0,
            was_blocked: true,
            result_preview: String::new(),
        },
    ];
    let json = serde_json::to_string(&details).unwrap();
    let parsed: Vec<ToolCallDetail> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].tool_name, "recall_memory");
    assert!(parsed[2].was_blocked);
}

#[test]
fn outcome_tag_default_is_no_action() {
    let tag: OutcomeTag = Default::default();
    assert_eq!(tag, OutcomeTag::NoAction);
}

#[test]
fn outcome_tag_serde_roundtrip() {
    for tag in [
        OutcomeTag::Success,
        OutcomeTag::Fallthrough,
        OutcomeTag::NoAction,
    ] {
        let json = serde_json::to_string(&tag).unwrap();
        let parsed: OutcomeTag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tag);
    }
}

#[test]
fn credit_analysis_report_serde() {
    let report = CreditAnalysisReport {
        analysis_date: "2026-03-30".into(),
        tool_scores: vec![ToolCreditScore {
            tool_name: "recall_memory".into(),
            times_called: 10,
            times_cited: 8,
            effectiveness_score: 0.8,
            wasted_calls: 2,
            avg_latency_ms: 45.5,
        }],
        top_effective: vec!["recall_memory".into()],
        most_wasteful: vec![],
    };
    let json = serde_json::to_string(&report).unwrap();
    let parsed: CreditAnalysisReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.analysis_date, "2026-03-30");
    assert_eq!(parsed.tool_scores.len(), 1);
    assert_eq!(parsed.tool_scores[0].effectiveness_score, 0.8);
    assert_eq!(parsed.top_effective, vec!["recall_memory"]);
}

// ── Adaptive heartbeat interval tests ──────────────────────────────

#[test]
fn snapshot_compiled_at_uses_user_timezone() {
    let snap = SituationSnapshot {
        compiled_at: "2026-04-01T22:13:05-07:00".to_string(),
        timezone: "America/Los_Angeles".to_string(),
        recent_probe_results: vec![],
        pending_tasks: vec![],
        upcoming_intents: vec![],
        skill_metrics: vec![],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![],
        budget_status: (0, 500_000),
        user_last_active_unix: None,
    };
    let block = snap.to_prompt_block();
    assert!(
        block.contains("2026-04-01T22:13:05-07:00"),
        "compiled_at should be in user timezone, got: {}",
        &block[..200.min(block.len())]
    );
    assert!(!block.contains("compiled=\"2026-04-01T22:13:05Z\""));
}

#[test]
fn snapshot_upcoming_intents_use_user_timezone() {
    let snap = SituationSnapshot {
        compiled_at: "2026-04-02T10:00:00+08:00".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        recent_probe_results: vec![],
        pending_tasks: vec![],
        upcoming_intents: vec![crate::memory::DeferredIntent {
            id: 1,
            user_did: "did:test".into(),
            chat_id: 123,
            trigger_type: "time".into(),
            trigger_at_unix: Some(1743580800),
            event_key: None,
            execution_mode: "notify".into(),
            context_text: "check email".into(),
            status: "pending".into(),
            created_at: "2026-04-01".into(),
            expires_at_unix: 0,
            fired_at: None,
            is_callback_origin: false,
            retry_count: 0,
            interval_seconds: None,
            chain_depth: 0,
            pin_on_fire: false,
            group_key: None,
            max_fires: None,
            fire_count: 0,
            recur_weekdays: None,
            recur_window_start: None,
            recur_window_end: None,
            recur_timezone: None,
            embedding: None,
        }],
        skill_metrics: vec![],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![],
        budget_status: (0, 500_000),
        user_last_active_unix: None,
    };
    let block = snap.to_prompt_block();
    assert!(
        block.contains("16:00"),
        "Intent trigger time should be in user timezone (CST), got: {}",
        block
    );
}

#[test]
fn snapshot_probe_results_use_user_timezone() {
    let snap = SituationSnapshot {
        compiled_at: "2026-04-02T10:00:00+08:00".to_string(),
        timezone: "America/Los_Angeles".to_string(),
        recent_probe_results: vec![crate::memory::HeartbeatAuditEntry {
            id: 1,
            cycle_id: "test-1".into(),
            skill_name: "test_skill".into(),
            timestamp: "2026-04-02 05:30:00".into(),
            severity: "Info".into(),
            probe_payload: None,
            tools_called: vec!["web_search".into()],
            tools_blocked: vec![],
            actions_taken: vec![],
            llm_reply: None,
            tokens_used: 100,
            rounds_used: 1,
            user_did: "did:test".into(),
            chat_id: Some(123),
            raw_process_log: None,
            tool_call_details: None,
            outcome: None,
            skip_reason: None,
            config_snapshot_json: None,
            prompt_hash: None,
            proposals_created_json: None,
            next_tick_note_bytes: None,
        }],
        pending_tasks: vec![],
        upcoming_intents: vec![],
        skill_metrics: vec![],
        tripped_circuit_breakers: vec![],
        hitl_pending: vec![],
        budget_status: (0, 500_000),
        user_last_active_unix: None,
    };
    let block = snap.to_prompt_block();
    assert!(
        block.contains("22:30 PDT"),
        "Probe timestamp should be in user timezone (PDT), got: {}",
        block
    );
}

// autonomous_outcome_with_outcome_tag_serde: DELETED in Step 3 — see note
// above on autonomous_outcome_serde. The struct-level serde contract is no
// longer part of the type.

// Step 6: archive_heartbeat_memo handles a pointer-mode memo without panicking.
#[tokio::test]
async fn archive_heartbeat_memo_accepts_pointer_shape() {
    let sandbox = shared_sandbox();
    let settings = Arc::new(RuntimeSettings::new());
    let memory = crate::memory::MemoryHandle::spawn_in_memory().expect("in-memory DB");

    // Seed a pointer-mode memo (Step 6 overflow shape).
    let pointer = serde_json::json!({
        "overflow": true,
        "slug": "20260415-beef0102",
        "synopsis": "Week-1 refactor plan.",
        "bytes": 15_360,
        "at": "2026-04-15T12:34:56+00:00",
    });
    memory
        .store_setting(
            "heartbeat_memo",
            serde_json::Value::String(pointer.to_string()),
        )
        .await
        .expect("seed pointer memo");
    settings
        .set(
            "heartbeat_memo",
            serde_json::Value::String(pointer.to_string()),
        )
        .expect("mirror into cache");

    let mut chronos = ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        settings.clone(),
    );
    // set_triage wires memory (+ embedder stub we skip — archive tolerates None).
    chronos.set_memory_for_tests(memory.clone());

    let archived = chronos
        .archive_heartbeat_memo("did:aegis:test")
        .await
        .expect("archive should succeed");
    assert!(
        archived.is_some(),
        "pointer memo should archive (not empty)"
    );
    let body = archived.unwrap();
    assert!(body.contains("overflow"), "raw pointer returned to caller");

    // Memo setting cleared.
    let cleared = memory
        .load_all_settings()
        .await
        .expect("load settings")
        .get("heartbeat_memo")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    assert!(cleared.is_empty(), "memo cleared after archive");
}

// ── cron registration gating tests ──────────────────────────────────
//
// These exercise the pure helper `should_register_cron_for_skill`
// that gates the scheduler registration loop in `start_scheduler_with_accounts`.
// See 2026-04-17 fix removing the stale self_audit `disabled_skill_cron` check.

#[test]
fn cron_registration_skipped_for_disabled_skill() {
    use crate::chronos_probes::should_register_cron_for_skill;

    let settings = RuntimeSettings::new();
    settings.register_skill("calendar_reader", "inbox scan", true);
    settings
        .set("skill_calendar_reader", Value::Bool(false))
        .unwrap();

    let cap = default_cap("calendar_reader"); // has `0 */5 * * * *`

    assert!(
        !should_register_cron_for_skill(&cap, &settings),
        "disabled skill must NOT register a cron job (regression: 2026-04-17 log-spam)"
    );
}

#[test]
fn cron_registration_active_for_enabled_skill() {
    use crate::chronos_probes::should_register_cron_for_skill;

    let settings = RuntimeSettings::new();
    settings.register_skill("calendar_reader", "inbox scan", true);
    settings
        .set("skill_calendar_reader", Value::Bool(true))
        .unwrap();

    let cap = default_cap("calendar_reader");

    assert!(
        should_register_cron_for_skill(&cap, &settings),
        "enabled skill with a cron_schedule MUST register a cron job"
    );
}

#[test]
fn cron_registration_skipped_when_no_schedule() {
    use crate::chronos_probes::should_register_cron_for_skill;

    let settings = RuntimeSettings::new();
    settings.register_skill("no_cron", "on-demand only", true);
    settings.set("skill_no_cron", Value::Bool(true)).unwrap();

    let mut cap = default_cap("no_cron");
    cap.cron_schedule = None;

    assert!(
        !should_register_cron_for_skill(&cap, &settings),
        "skill without cron_schedule has nothing to register"
    );
}

#[test]
fn cron_registration_default_treats_unregistered_as_enabled() {
    // `is_enabled` returns true for unknown keys (opt-out model). Verify the
    // helper picks that up so we don't silently drop cron jobs for skills
    // that ship without an explicit toggle registered.
    use crate::chronos_probes::should_register_cron_for_skill;

    let settings = RuntimeSettings::new();
    // Intentionally do NOT call `register_skill` / `set`.
    let cap = default_cap("unregistered_skill");

    assert!(
        should_register_cron_for_skill(&cap, &settings),
        "unregistered skill should default to enabled (matches is_enabled semantics)"
    );
}

// ── Multi-mailbox Slice 1 W1: runtime per-account cron registration ─────────

/// Startup-parity + live-add: `start_scheduler_with_accounts` seeds the
/// runtime-cron context with the startup accounts (dup-guarded), a
/// runtime-added account gets exactly the same per-account job set, a repeat
/// add is a no-op, and removal frees the account for a clean re-add.
#[tokio::test]
async fn runtime_account_cron_jobs_register_dedupe_and_unregister() {
    let sandbox = Arc::new(SandboxEngine::new().expect("sandbox"));
    let wat = make_probe_skill_wat(false, None);
    let mut cap = default_cap("multi_mail_probe");
    cap.supports_multi_account = true;
    sandbox.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

    let chronos = Arc::new(ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    ));

    // Before the scheduler starts there is nothing to add jobs to — the
    // startup registration loop owns the account (transparent no-op).
    assert_eq!(
        chronos.register_account_cron_jobs("early").await.unwrap(),
        0,
        "no live scheduler yet -> Ok(0), never an error"
    );

    let handler: Arc<dyn HeartbeatHandler> = Arc::new(MockHandler::new());
    let mut sched = chronos
        .clone()
        .start_scheduler_with_accounts(handler, &["startup_acct".to_string()])
        .await
        .expect("scheduler starts");

    // Startup-registered account is already covered — the runtime add path
    // must be a dup-guarded no-op for it (no double heartbeats).
    assert_eq!(
        chronos
            .register_account_cron_jobs("startup_acct")
            .await
            .unwrap(),
        0,
        "startup-registered account must not get duplicate jobs"
    );

    // Live-add: one multi-account cron skill loaded -> exactly one new job.
    assert_eq!(
        chronos
            .register_account_cron_jobs("runtime_acct")
            .await
            .unwrap(),
        1,
        "runtime-added account gets one job per multi-account cron skill"
    );
    // Idempotent repeat.
    assert_eq!(
        chronos
            .register_account_cron_jobs("runtime_acct")
            .await
            .unwrap(),
        0,
        "second add for the same account must be a no-op"
    );

    // Removal mirror: drops the job and frees the account for a re-add
    // (credential rotation flow: remove -> add).
    assert_eq!(
        chronos
            .unregister_account_cron_jobs("runtime_acct")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        chronos
            .unregister_account_cron_jobs("runtime_acct")
            .await
            .unwrap(),
        0,
        "removing an unknown account is a no-op"
    );
    assert_eq!(
        chronos
            .register_account_cron_jobs("runtime_acct")
            .await
            .unwrap(),
        1,
        "account can be re-added after removal"
    );

    sched.shutdown().await.ok();
}

/// A skill WITHOUT `supports_multi_account` never produces per-account jobs
/// through the runtime add path (mirrors the startup loop's payload split).
#[tokio::test]
async fn runtime_account_cron_jobs_skip_single_account_skills() {
    let sandbox = Arc::new(SandboxEngine::new().expect("sandbox"));
    let wat = make_probe_skill_wat(false, None);
    // default_cap: cron_schedule set, supports_multi_account = false.
    sandbox
        .load_skill_from_bytes(wat.as_bytes(), default_cap("single_probe"))
        .unwrap();

    let chronos = Arc::new(ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    ));
    let handler: Arc<dyn HeartbeatHandler> = Arc::new(MockHandler::new());
    let mut sched = chronos
        .clone()
        .start_scheduler_with_accounts(handler, &["acct".to_string()])
        .await
        .expect("scheduler starts");

    assert_eq!(
        chronos
            .register_account_cron_jobs("new_acct")
            .await
            .unwrap(),
        0,
        "single-account skills must not spawn per-account jobs"
    );

    sched.shutdown().await.ok();
}

/// Parent Codex P2: a ZERO-account startup registers a NO-account fallback
/// cron job for multi-account skills. Once the first runtime account is
/// added, that fallback would resolve to the new default account and
/// double-probe it every tick — the first successful runtime registration
/// must therefore drain the fallback guids and remove the jobs. A second
/// account add sees an empty list (idempotent).
#[tokio::test]
async fn first_runtime_account_add_removes_zero_account_fallback_jobs() {
    let sandbox = Arc::new(SandboxEngine::new().expect("sandbox"));
    let wat = make_probe_skill_wat(false, None);
    let mut cap = default_cap("fallback_probe");
    cap.supports_multi_account = true;
    sandbox.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

    let chronos = Arc::new(ChronosEngine::new(
        sandbox,
        Arc::new(AgentIdentity::generate()),
        Arc::new(RuntimeSettings::new()),
    ));
    let handler: Arc<dyn HeartbeatHandler> = Arc::new(MockHandler::new());
    // ZERO accounts at startup -> the multi-account skill gets one
    // no-account fallback job, recorded for later removal.
    let mut sched = chronos
        .clone()
        .start_scheduler_with_accounts(handler, &[])
        .await
        .expect("scheduler starts");
    {
        let guard = chronos.runtime_cron.lock();
        let ctx = guard.as_ref().expect("context installed");
        assert_eq!(
            ctx.multi_account_fallback_guids.len(),
            1,
            "zero-account startup must record the fallback job for removal"
        );
    }

    // First runtime add: registers the per-account job AND drains/removes
    // the fallback (no double-probe of the new default account).
    assert_eq!(
        chronos.register_account_cron_jobs("first").await.unwrap(),
        1,
        "first runtime account gets its per-account job"
    );
    {
        let guard = chronos.runtime_cron.lock();
        let ctx = guard.as_ref().expect("context installed");
        assert!(
            ctx.multi_account_fallback_guids.is_empty(),
            "fallback guids must be drained by the first account add"
        );
    }

    // Second add: nothing left to drain; normal registration only.
    assert_eq!(
        chronos.register_account_cron_jobs("second").await.unwrap(),
        1,
        "second account registers normally with no fallback interference"
    );

    sched.shutdown().await.ok();
}
/// Codex adversarial rounds 1-2 (HIGH): an unregister that consumes the
/// in-flight reservation while `register_account_cron_jobs` is awaiting
/// `sched.add()` — INCLUDING the remove-then-readd interleaving where a NEW
/// generation now owns the slot — must make the stale registration ROLL BACK
/// its just-added jobs instead of orphaning them or hijacking the new
/// generation's slot. The record-step decision is pure — test it directly.
#[test]
fn record_account_job_guids_rolls_back_when_reservation_was_unregistered() {
    use crate::chronos::{AccountJobsSlot, RuntimeCronContext};
    use crate::chronos_probes::{RecordJobsOutcome, record_account_job_guids};

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let sched = rt
        .block_on(tokio_cron_scheduler::JobScheduler::new())
        .expect("sched");
    let handler: Arc<dyn HeartbeatHandler> = Arc::new(MockHandler::new());
    let mk_ctx = || RuntimeCronContext {
        sched: sched.clone(),
        handler: handler.clone(),
        account_jobs: std::collections::HashMap::new(),
        next_generation: 1,
        multi_account_fallback_guids: Vec::new(),
    };
    let reserve = |ctx: &mut Option<RuntimeCronContext>, generation: u64| {
        ctx.as_mut().unwrap().account_jobs.insert(
            "acct".into(),
            AccountJobsSlot {
                generation,
                guids: Vec::new(),
            },
        );
    };
    let guid = uuid::Uuid::new_v4();

    // Our reservation intact (same generation) → guids recorded in the slot.
    let mut ctx = Some(mk_ctx());
    reserve(&mut ctx, 1);
    assert_eq!(
        record_account_job_guids(&mut ctx, "acct", 1, vec![guid], false),
        RecordJobsOutcome::Recorded
    );
    assert_eq!(
        ctx.as_ref()
            .unwrap()
            .account_jobs
            .get("acct")
            .map(|s| s.guids.clone()),
        Some(vec![guid]),
        "guids must be recorded for later removal"
    );

    // Reservation GONE (concurrent unregister) → RolledBack carries the guids
    // the caller must remove from the scheduler.
    let mut ctx = Some(mk_ctx());
    assert_eq!(
        record_account_job_guids(&mut ctx, "acct", 1, vec![guid], false),
        RecordJobsOutcome::RolledBack(vec![guid]),
        "a consumed reservation must trigger rollback, never an orphan"
    );

    // Remove-then-READD interleaving (round 2): the slot exists but under a
    // NEWER generation — the stale registration must roll back and must NOT
    // touch the new generation's slot.
    let mut ctx = Some(mk_ctx());
    reserve(&mut ctx, 2); // re-register's fresh reservation
    assert_eq!(
        record_account_job_guids(&mut ctx, "acct", 1, vec![guid], false),
        RecordJobsOutcome::RolledBack(vec![guid]),
        "a superseded generation must roll back, not hijack the new slot"
    );
    assert_eq!(
        ctx.as_ref().unwrap().account_jobs.get("acct"),
        Some(&AccountJobsSlot {
            generation: 2,
            guids: Vec::new(),
        }),
        "the new generation's reservation must be left untouched"
    );

    // Total add failure with our reservation intact → reservation freed so a
    // retry can re-register cleanly.
    let mut ctx = Some(mk_ctx());
    reserve(&mut ctx, 1);
    assert_eq!(
        record_account_job_guids(&mut ctx, "acct", 1, Vec::new(), true),
        RecordJobsOutcome::Recorded
    );
    assert!(
        !ctx.as_ref().unwrap().account_jobs.contains_key("acct"),
        "total failure must free the reservation"
    );
}
