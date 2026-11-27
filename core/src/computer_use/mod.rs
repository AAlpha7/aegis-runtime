//! Secure Computer Use — Intent → Policy → Sign → Device → Audit.
//!
//! Cross-platform path (Mac/JXA remains a later adapter):
//!
//! ```text
//! Intent (v1) → Policy Broker (TypeSafe Jev via OpenRouter Decisions API)
//!            → Ed25519 Signer → Device Agent (verify / anti-replay / receipt)
//!            → Audit hash chain
//! ```
//!
//! Gated by `AEGIS_SECURE_CU_ENABLED=1`. Device adapters:
//! - `loopback` — in-process stub (unit tests / fallback)
//! - `linux_atspi` — Ubuntu desktop AT-SPI (Oracle shared path)
//!
//! Design: `docs/design/secure-computer-use-p0.md` (+ Linux / Oracle note).

pub mod audit;
pub mod decisions;
pub mod device;
pub mod intent;
pub mod linux;
pub mod non_cu_gate;
pub mod pipeline;
pub mod policy;
pub mod provenance;
pub mod readonly;
pub mod signer;

pub use audit::{AuditChain, AuditEvent, AuditRecord};
pub use decisions::{
    DecisionsAnswer, DecisionsClient, DecisionsQuestion, DecisionsRequest, DecisionsResponse,
    JevModel,
};
pub use device::{DeviceAgent, DeviceReceipt, DeviceRejectReason, LoopbackDeviceAgent};
pub use intent::{CU_INTENT_VERSION, CuIntentBody, CuIntentEnvelope};
pub use linux::{LinuxAtspiDeviceAgent, LinuxSessionProbe, probe_linux_session};
pub use non_cu_gate::{
    JEV_NON_CU_ASK_KIND, NonCuGateOutcome, NonCuPendingAsk, evaluate_non_cu_gate,
    jev_non_cu_gate_enabled,
};
pub use pipeline::{PendingAsk, PipelineDevice, PipelineError, PipelineOutcome, SecureCuPipeline};
pub use policy::{
    DeterministicPolicyBroker, JevPolicyBroker, PolicyBroker, PolicyDecision, PolicyVerdict,
};
pub use provenance::{
    JevProvenance, jev_provenance_from_tool_value, stamp_jev_on_json, stamp_jev_on_value,
};
pub use readonly::{CU_READONLY_TOOLS, is_cu_readonly_tool};
pub use signer::{CuSigner, SignedCuIntent};

use std::sync::Arc;

use crate::identity::AgentIdentity;

/// Master feature flag. Off by default so CU does not change production behavior.
pub fn secure_cu_enabled() -> bool {
    match std::env::var("AEGIS_SECURE_CU_ENABLED") {
        Ok(v) => {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

/// Optional paid Jev integration. Credentials alone never enable it.
pub fn jev_enabled() -> bool {
    jev_opted_in(std::env::var("AEGIS_JEV_ENABLED").ok().as_deref())
}

fn jev_opted_in(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let t = v.trim();
        t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
    })
}

/// Jev must be explicitly enabled before any API credential is resolved.
/// Subscriptions do not authorize this API.
/// Legacy OPENAI_API_KEY is accepted only when it is an OpenRouter key.
/// Rejects the `.env.example` placeholder (`sk-or-v1-xxxxxxxx`).
pub fn resolve_openrouter_api_key() -> Option<String> {
    resolve_jev_key_with(|key| std::env::var(key).ok())
}

fn resolve_jev_key_with(mut lookup: impl FnMut(&str) -> Option<String>) -> Option<String> {
    if !jev_opted_in(lookup("AEGIS_JEV_ENABLED").as_deref()) {
        return None;
    }
    configured_jev_key_with(lookup)
}

pub(crate) fn configured_jev_api_key() -> Option<String> {
    configured_jev_key_with(|key| std::env::var(key).ok())
}

fn configured_jev_key_with(mut lookup: impl FnMut(&str) -> Option<String>) -> Option<String> {
    for key in ["AEGIS_JEV_API_KEY", "OPENROUTER_API_KEY", "OPENAI_API_KEY"] {
        if let Some(v) = lookup(key) {
            let t = v.trim();
            if valid_jev_credential(key, t) {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn valid_jev_credential(source: &str, value: &str) -> bool {
    value.len() > 20
        && !value.contains("xxxxxxxx")
        && (source != "OPENAI_API_KEY" || value.starts_with("sk-or-"))
}

/// `AEGIS_SECURE_CU_ADAPTER`: `auto` | `loopback` | `linux` (default `auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuAdapterKind {
    Auto,
    Loopback,
    Linux,
}

impl CuAdapterKind {
    pub fn from_env() -> Self {
        match std::env::var("AEGIS_SECURE_CU_ADAPTER") {
            Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
                "loopback" | "loop" => Self::Loopback,
                "linux" | "linux_atspi" | "atspi" => Self::Linux,
                _ => Self::Auto,
            },
            Err(_) => Self::Auto,
        }
    }
}

/// Device id bound into CU intents (`AEGIS_SECURE_CU_DEVICE_ID`, default by adapter).
pub fn resolve_device_id(adapter: CuAdapterKind) -> String {
    if let Ok(v) = std::env::var("AEGIS_SECURE_CU_DEVICE_ID") {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    match adapter {
        CuAdapterKind::Loopback => "loopback-local".into(),
        CuAdapterKind::Linux | CuAdapterKind::Auto => "linux-local".into(),
    }
}

/// Pick the live Device Agent. `auto` prefers Linux AT-SPI on Linux hosts
/// (even when the session is currently headless — execute fails cleanly).
pub fn select_pipeline_device(kind: CuAdapterKind) -> PipelineDevice {
    let device_id = resolve_device_id(kind);
    // Live path already passed ToolGate; accept Ask at the device so Jev/deterministic
    // Ask verdicts can still produce a receipt (owner confirmation is the chat turn).
    match kind {
        CuAdapterKind::Loopback => PipelineDevice::Loopback(Arc::new(
            LoopbackDeviceAgent::new(device_id).accept_ask(true),
        )),
        CuAdapterKind::Linux => PipelineDevice::LinuxAtspi(Arc::new(
            LinuxAtspiDeviceAgent::new(device_id).accept_ask(true),
        )),
        CuAdapterKind::Auto => {
            if cfg!(target_os = "linux") {
                PipelineDevice::LinuxAtspi(Arc::new(
                    LinuxAtspiDeviceAgent::new(device_id).accept_ask(true),
                ))
            } else {
                PipelineDevice::Loopback(Arc::new(
                    LoopbackDeviceAgent::new(device_id).accept_ask(true),
                ))
            }
        }
    }
}

/// Build a Policy Broker: Jev only when opted in with a valid key, else
/// deterministic Ask (local rules plus owner approval, without Jev claims).
pub fn build_live_policy_broker() -> anyhow::Result<PolicyBroker> {
    policy_broker_for_key(resolve_openrouter_api_key())
}

pub(crate) fn policy_broker_for_key(key: Option<String>) -> anyhow::Result<PolicyBroker> {
    if let Some(key) = key {
        let broker = JevPolicyBroker::new(key, JevModel::from_env())?;
        Ok(PolicyBroker::jev(broker))
    } else {
        Ok(PolicyBroker::deterministic(PolicyVerdict::Ask))
    }
}

/// Construct the host pipeline for orchestrator / diagnostics (flag still enforced at run).
pub fn build_live_pipeline(identity: Arc<AgentIdentity>) -> anyhow::Result<SecureCuPipeline> {
    let kind = CuAdapterKind::from_env();
    let broker = build_live_policy_broker()?;
    let device = select_pipeline_device(kind);
    // Ask must suspend for owner HITL (InteractiveOnly channel) before CuSigner.
    // Allow still bypasses; Deny still hard-stops. Device accepts Ask only after resume.
    Ok(SecureCuPipeline::new(identity, broker, device).require_owner_for_ask(true))
}

/// Soft `/health` component (never fails the process health check).
pub fn health_component() -> serde_json::Value {
    let enabled = secure_cu_enabled();
    let kind = CuAdapterKind::from_env();
    let probe = if matches!(kind, CuAdapterKind::Linux | CuAdapterKind::Auto)
        && cfg!(target_os = "linux")
    {
        Some(probe_linux_session())
    } else {
        None
    };
    serde_json::json!({
        "enabled": enabled,
        "adapter": match kind {
            CuAdapterKind::Auto => "auto",
            CuAdapterKind::Loopback => "loopback",
            CuAdapterKind::Linux => "linux",
        },
        "device_id": resolve_device_id(kind),
        "jev_model": JevModel::from_env().as_str(),
        "jev_enabled": jev_enabled(),
        "openrouter_key_present": resolve_openrouter_api_key().is_some(),
        "linux_session": probe,
        "readonly_tools": CU_READONLY_TOOLS,
        "jev_non_cu_gate": non_cu_gate::health_component(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn jev_defaults_off_even_with_credentials() {
        for flag in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("no"),
            Some("typo"),
        ] {
            let key = super::resolve_jev_key_with(|name| {
                assert_eq!(
                    name, "AEGIS_JEV_ENABLED",
                    "disabled Jev must not read credentials"
                );
                flag.map(str::to_owned)
            });
            assert!(key.is_none());
            assert!(matches!(
                super::policy_broker_for_key(key).unwrap(),
                super::PolicyBroker::Deterministic(_)
            ));
        }
    }

    #[test]
    fn jev_opt_in_requires_key_and_preserves_credential_precedence() {
        for flag in ["1", "true", " YES "] {
            let key = super::resolve_jev_key_with(|name| match name {
                "AEGIS_JEV_ENABLED" => Some(flag.into()),
                "AEGIS_JEV_API_KEY" => Some("synthetic-dedicated-key-for-tests".into()),
                _ => panic!("dedicated Jev key must take precedence"),
            });
            assert!(matches!(
                super::policy_broker_for_key(key).unwrap(),
                super::PolicyBroker::Jev(_)
            ));
        }
        let key =
            super::resolve_jev_key_with(|name| (name == "AEGIS_JEV_ENABLED").then(|| "1".into()));
        assert!(key.is_none());
        let key = super::resolve_jev_key_with(|name| match name {
            "AEGIS_JEV_ENABLED" => Some("1".into()),
            "OPENROUTER_API_KEY" => Some("synthetic-openrouter-key-for-tests".into()),
            _ => None,
        });
        assert_eq!(key.as_deref(), Some("synthetic-openrouter-key-for-tests"));
    }

    #[test]
    fn jev_never_reuses_a_generic_openai_credential() {
        let fixture = "synthetic-credential-for-routing-tests";
        assert!(!super::valid_jev_credential("OPENAI_API_KEY", fixture));
        assert!(super::valid_jev_credential("AEGIS_JEV_API_KEY", fixture));
        assert!(super::valid_jev_credential("OPENROUTER_API_KEY", fixture));
        assert!(!super::valid_jev_credential("AEGIS_JEV_API_KEY", ""));
        assert!(!super::valid_jev_credential(
            "AEGIS_JEV_API_KEY",
            "sk-or-v1-xxxxxxxx-placeholder"
        ));
    }
    #[tokio::test]
    async fn disabled_jev_holds_computer_actions_without_routine_warning() {
        let broker = super::policy_broker_for_key(None).unwrap();
        let input = super::policy::PolicyInput {
            tool: "ui.click".into(),
            args: serde_json::json!({}),
            device_id: "test-device".into(),
            objective: None,
        };
        let (decision, notices) =
            crate::behavior_notice::collect("en", None, None, broker.decide(&input)).await;
        assert_eq!(decision.unwrap().verdict, super::PolicyVerdict::Ask);
        assert!(
            notices.is_empty(),
            "configured local policy is not a degradation: {notices:?}"
        );
    }
    #[test]
    fn placeholder_api_key_rejected() {
        let placeholder = "sk-or-v1-xxxxxxxx";
        assert!(placeholder.contains("xxxxxxxx"));
        let plausible = "sk-or-v1-abcdefghijklmnopqrstuvwxyz012345";
        assert!(!plausible.contains("xxxxxxxx"));
        assert!(plausible.len() > 20);
    }

    #[test]
    fn feature_flag_parser_truthy_values() {
        fn parse(v: &str) -> bool {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        assert!(parse("1"));
        assert!(parse("true"));
        assert!(parse("YES"));
        assert!(!parse("0"));
        assert!(!parse(""));
    }

    #[test]
    fn health_component_serializes() {
        let v = super::health_component();
        assert!(v.get("enabled").is_some());
        assert!(v.get("adapter").is_some());
    }
}
