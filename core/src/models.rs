use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Core communication carrier: Intent Envelope.
/// Replaces plaintext API keys. Every operation must be wrapped in a
/// cryptographically-signed envelope.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IntentEnvelope {
    /// Globally unique agent identity (format: "did:aegis:ed25519:<pubkey_hex>")
    pub agent_did: String,
    /// Unix timestamp (seconds). Gateway must reject requests older than 5 minutes
    /// to prevent replay attacks.
    pub timestamp: u64,
    /// Target Wasm skill module name (e.g. "crypto_tracker", "web_reader")
    pub target_skill: String,
    /// Dynamic parameters passed to the Wasm skill
    pub payload: Value,
    /// Ed25519 signature over the canonical fields (hex-encoded).
    /// Covers: agent_did + timestamp + target_skill + payload (JSON-serialized, sorted keys)
    pub signature: String,
}

/// Capability declaration for a Wasm skill module.
/// Read from each skill's manifest.json before instantiation.
/// Implements capability-based security: default-deny everything.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SkillCapability {
    pub skill_name: String,
    /// Maximum allowed memory in MB (default: 2 MB)
    pub max_memory_mb: u32,
    /// Whitelist of domains the skill may make network requests to.
    /// An empty list means zero network access.
    pub allowed_domains: Vec<String>,
    /// Virtual filesystem paths the skill may access (host path -> wasm path).
    /// An empty list means no filesystem access.
    pub allowed_fs_paths: Vec<String>,
    /// Optional cron expression for heartbeat scheduling (e.g. "*/10 * * * * *")
    pub cron_schedule: Option<String>,
    /// Human-readable description exposed to the LLM as a tool definition
    pub description: String,
    /// JSON Schema for the skill's payload, used to validate LLM tool calls
    pub payload_schema: Value,
    /// Default payload for heartbeat/cron invocations.
    /// If None, heartbeat uses `{}`. Allows configuring thresholds in manifest.json.
    #[serde(default)]
    pub default_payload: Option<Value>,
    /// Skill pattern type (e.g. "api_json", "no_network", "probe_heartbeat").
    /// Declared by the skill's manifest; describes its host-call shape.
    #[serde(default)]
    pub skill_pattern: Option<String>,
    /// Execution language for the skill runtime.
    /// "rust" (default) compiles to native Wasm; "javascript" runs in QuickJS Wasm engine.
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether this skill only fetches data (HTTP GET) and never sends user data outward.
    /// Read-only skills are exempt from exfiltration detection.
    #[serde(default)]
    pub is_read_only: bool,
    /// Whether this skill supports multi-account dispatch (e.g. mail skills with personal/work accounts).
    /// If true, an optional `account` parameter is injected into the tool schema.
    #[serde(default)]
    pub supports_multi_account: bool,
    /// Maximum wall-clock execution time in seconds (default: 60, max: 300).
    /// Overrides the global WASM_EXEC_TIMEOUT for this skill.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum fuel (CPU instruction budget) for this skill.
    /// Default: 500_000_000 (500M instructions ≈ a few seconds of pure compute).
    /// Higher values for computation-heavy skills.
    #[serde(default = "default_max_fuel")]
    pub max_fuel: u64,
    /// Whether the skill is in shadow mode (dry-run for write operations).
    /// In shadow mode, host_fetch POST/PUT/DELETE are intercepted and logged
    /// but not actually executed. User must approve to promote to active mode.
    #[serde(default)]
    pub shadow_mode: bool,
    /// Optional Telegram menu command. When set, activating this skill
    /// auto-registers this command in the bot menu; deactivating removes it.
    /// Must be 1-32 lowercase alphanumeric/underscore chars (no leading slash).
    #[serde(default)]
    pub menu_command: Option<String>,
    /// Description shown next to the menu command in Telegram's menu button.
    /// Max 256 chars. Required if `menu_command` is set.
    #[serde(default)]
    pub menu_description: Option<String>,
    /// Whether this skill uses the WIT Component Model (v2) interface.
    /// v2 skills have typed parameters/returns and don't need pointer management.
    /// For probe_heartbeat v2 skills, ProbeSignal field names are compile-time locked.
    #[serde(default)]
    pub component_model: bool,
    /// Known limitations of this skill. Appended to the tool description sent to
    /// the LLM so it doesn't promise capabilities the skill doesn't have.
    /// Example: ["No real-time traffic data", "Cannot detect road closures"]
    #[serde(default)]
    pub limitations: Vec<String>,
}

fn default_timeout_secs() -> u64 {
    60
}

fn default_max_fuel() -> u64 {
    500_000_000
}

fn default_language() -> String {
    "rust".to_string()
}

/// Alert severity level reported by heartbeat probes.
/// Skills that don't set this default to Warning (backward compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum AlertSeverity {
    Info,
    #[default]
    Warning,
    Critical,
}

/// Return value from a Wasm heartbeat probe skill.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProbeSignal {
    /// false = silent success, drop instance (0 tokens spent)
    /// true  = escalate: pull LLM, push alert to admin
    pub require_llm_attention: bool,
    /// Present only when require_llm_attention == true.
    /// Human-readable event description to be summarized by the LLM.
    /// Alias "report" for backward compat with v1 skills that used the wrong field name.
    #[serde(alias = "report")]
    pub event_payload: Option<String>,
    /// Severity level. Defaults to Warning for backward compatibility.
    #[serde(default)]
    pub severity: AlertSeverity,
    /// Optional hints from the skill about what actions might be appropriate.
    /// The LLM is not bound by these — they are suggestions, not directives.
    #[serde(default)]
    pub suggested_actions: Vec<String>,
    /// Structured data from the probe for LLM decision-making.
    #[serde(default)]
    pub probe_data: Option<Value>,
}

/// Descriptor for a peeked batch of items from a ring buffer.
/// Used by the peek-then-drain buffer pattern.
/// The caller peeks N items, processes them, then drains up to `last_seq_id`.
#[derive(Debug, Clone, Copy)]
pub struct PeekBatch {
    /// Number of items in the peeked slice.
    pub count: usize,
    /// The seq_id of the last item in the peeked slice (inclusive upper bound for drain).
    pub last_seq_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn intent_envelope_roundtrip() {
        let env = IntentEnvelope {
            agent_did: "did:aegis:ed25519:abcdef".to_string(),
            timestamp: 1_700_000_000,
            target_skill: "crypto_tracker".to_string(),
            payload: json!({"threshold": 60000}),
            signature: "deadbeef".to_string(),
        };
        let serialized = serde_json::to_string(&env).unwrap();
        let deserialized: IntentEnvelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(env, deserialized);
    }

    #[test]
    fn skill_capability_defaults() {
        let cap: SkillCapability = serde_json::from_str(
            r#"{
            "skill_name": "test",
            "max_memory_mb": 2,
            "allowed_domains": [],
            "allowed_fs_paths": [],
            "cron_schedule": null,
            "description": "A test skill",
            "payload_schema": {}
        }"#,
        )
        .unwrap();
        assert_eq!(cap.max_memory_mb, 2);
        assert!(cap.allowed_domains.is_empty());
        assert!(cap.cron_schedule.is_none());
    }

    #[test]
    fn skill_capability_default_payload_present() {
        let cap: SkillCapability = serde_json::from_str(
            r#"{
            "skill_name": "crypto_tracker",
            "max_memory_mb": 2,
            "allowed_domains": ["api.coingecko.com"],
            "allowed_fs_paths": [],
            "cron_schedule": "*/10 * * * * *",
            "description": "BTC tracker",
            "payload_schema": {},
            "default_payload": {"threshold": 60000.0}
        }"#,
        )
        .unwrap();
        assert!(cap.default_payload.is_some());
        assert_eq!(
            cap.default_payload.unwrap()["threshold"].as_f64(),
            Some(60000.0)
        );
    }

    #[test]
    fn skill_capability_default_payload_missing_is_none() {
        let cap: SkillCapability = serde_json::from_str(
            r#"{
            "skill_name": "test",
            "max_memory_mb": 2,
            "allowed_domains": [],
            "allowed_fs_paths": [],
            "cron_schedule": null,
            "description": "A test skill",
            "payload_schema": {}
        }"#,
        )
        .unwrap();
        assert!(
            cap.default_payload.is_none(),
            "missing default_payload must deserialize as None"
        );
    }

    #[test]
    fn probe_signal_silent() {
        let signal = ProbeSignal {
            require_llm_attention: false,
            event_payload: None,
            severity: AlertSeverity::default(),
            suggested_actions: vec![],
            probe_data: None,
        };
        assert!(!signal.require_llm_attention);
        assert!(signal.event_payload.is_none());
        assert_eq!(signal.severity, AlertSeverity::Warning);
        assert!(signal.suggested_actions.is_empty());
        assert!(signal.probe_data.is_none());
    }

    #[test]
    fn probe_signal_alert() {
        let signal = ProbeSignal {
            require_llm_attention: true,
            event_payload: Some("BTC dropped below 60000".to_string()),
            severity: AlertSeverity::Critical,
            suggested_actions: vec![],
            probe_data: None,
        };
        assert!(signal.require_llm_attention);
        assert!(signal.event_payload.is_some());
        assert_eq!(signal.severity, AlertSeverity::Critical);
    }

    #[test]
    fn probe_signal_with_suggested_actions() {
        let signal = ProbeSignal {
            require_llm_attention: true,
            event_payload: Some("Server CPU at 95%".to_string()),
            severity: AlertSeverity::Critical,
            suggested_actions: vec![
                "Check running processes".to_string(),
                "Scale up instance".to_string(),
            ],
            probe_data: Some(json!({
                "cpu_percent": 95.2,
                "top_processes": ["java", "nginx"],
                "uptime_hours": 72
            })),
        };
        assert_eq!(signal.suggested_actions.len(), 2);
        assert_eq!(signal.suggested_actions[0], "Check running processes");
        assert!(signal.probe_data.is_some());
        let data = signal.probe_data.unwrap();
        assert_eq!(data["cpu_percent"].as_f64(), Some(95.2));
    }

    #[test]
    fn probe_signal_new_fields_backward_compat() {
        // JSON without suggested_actions and probe_data should deserialize with defaults
        let json = r#"{"require_llm_attention": true, "event_payload": "test"}"#;
        let signal: ProbeSignal = serde_json::from_str(json).unwrap();
        assert!(signal.suggested_actions.is_empty());
        assert!(signal.probe_data.is_none());
    }

    #[test]
    fn probe_signal_new_fields_roundtrip() {
        let signal = ProbeSignal {
            require_llm_attention: true,
            event_payload: Some("alert".to_string()),
            severity: AlertSeverity::Warning,
            suggested_actions: vec!["restart_service".to_string()],
            probe_data: Some(json!({"metric": 42})),
        };
        let serialized = serde_json::to_string(&signal).unwrap();
        let deserialized: ProbeSignal = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.suggested_actions.len(), 1);
        assert_eq!(deserialized.suggested_actions[0], "restart_service");
        assert_eq!(
            deserialized.probe_data.unwrap()["metric"].as_i64(),
            Some(42)
        );
    }

    #[test]
    fn probe_signal_severity_backward_compat() {
        // JSON without severity field should deserialize with Warning default
        let json = r#"{"require_llm_attention": true, "event_payload": "test"}"#;
        let signal: ProbeSignal = serde_json::from_str(json).unwrap();
        assert_eq!(signal.severity, AlertSeverity::Warning);
    }

    #[test]
    fn probe_signal_severity_explicit() {
        let json =
            r#"{"require_llm_attention": true, "event_payload": "test", "severity": "Critical"}"#;
        let signal: ProbeSignal = serde_json::from_str(json).unwrap();
        assert_eq!(signal.severity, AlertSeverity::Critical);
    }

    #[test]
    fn alert_severity_ordering() {
        assert!(AlertSeverity::Info < AlertSeverity::Warning);
        assert!(AlertSeverity::Warning < AlertSeverity::Critical);
    }
}
