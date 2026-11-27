//! Jev Policy Broker reused for non-CU high-risk tools.
//!
//! Flag-gated (`AEGIS_JEV_NON_CU_GATE_ENABLED`). Off = no behavior change.
//! On: designated tools (currently `google_drive_upload`) go through the same
//! allow|ask|deny compose path as Secure CU. Ask reuses the HITL /
//! `secure_cu_ask` owner-approval pattern (`kind=jev_non_cu_ask`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::computer_use::policy::{
    DeterministicPolicyBroker, JevPolicyBroker, PolicyBroker, PolicyDecision, PolicyInput,
    PolicyVerdict,
};

/// HITL kind stored in `skill_state` (sibling of `secure_cu_ask`).
pub const JEV_NON_CU_ASK_KIND: &str = "jev_non_cu_ask";

/// Concrete danger path wired in this increment.
pub const JEV_NON_CU_GATED_TOOLS: &[&str] = &["google_drive_upload"];

/// Master flag. Off by default.
pub fn jev_non_cu_gate_enabled() -> bool {
    parse_truthy_env("AEGIS_JEV_NON_CU_GATE_ENABLED")
}

/// Optional Oracle/hand-test override: `allow` | `ask` | `deny`.
/// When set (and the master flag is on), skip Decisions HTTP.
pub fn jev_non_cu_force_verdict() -> Option<PolicyVerdict> {
    std::env::var("AEGIS_JEV_NON_CU_FORCE_VERDICT")
        .ok()
        .and_then(|v| PolicyVerdict::parse(&v))
}

pub fn is_jev_non_cu_gated_tool(tool: &str) -> bool {
    JEV_NON_CU_GATED_TOOLS.contains(&tool)
}

fn parse_truthy_env(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

/// Frozen Ask payload for a non-CU tool (no CuSigner).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NonCuPendingAsk {
    pub intent_id: String,
    pub tool: String,
    pub args: Value,
    pub objective: Option<String>,
    pub policy: PolicyDecision,
    pub proposed_at: u64,
}

/// Outcome of the non-CU Jev gate (before the original tool runs).
#[derive(Debug, Clone)]
pub enum NonCuGateOutcome {
    /// Flag off, unknown tool, or already owner-approved resume.
    Bypass,
    Allow(PolicyDecision),
    Ask(NonCuPendingAsk),
    Deny(PolicyDecision),
}

/// Marker written into tool args on HITL resume so execute skips a second Jev call.
pub const JEV_OWNER_APPROVED_KEY: &str = "_jev_owner_approved";

pub fn args_owner_approved(args: &Value) -> bool {
    args.get(JEV_OWNER_APPROVED_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn strip_owner_approved_marker(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        obj.remove(JEV_OWNER_APPROVED_KEY);
    }
    args
}

pub fn with_owner_approved_marker(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        obj.insert(JEV_OWNER_APPROVED_KEY.to_string(), json!(true));
    }
    args
}

/// Deterministic fallback when no OpenRouter key / force-verdict is set.
///
/// Upload-download is Ask (HITL). Hard-deny names stay Deny. Everything else
/// in the gated set defaults to Ask.
pub fn deterministic_non_cu_verdict(tool: &str) -> PolicyVerdict {
    let t = tool.trim().to_ascii_lowercase();
    if t.contains("shell.exec") || t.contains("eval") || t.contains("delete_all") {
        return PolicyVerdict::Deny;
    }
    if t == "google_drive_upload" {
        return PolicyVerdict::Ask;
    }
    PolicyVerdict::Ask
}

pub fn build_non_cu_policy_broker() -> PolicyBroker {
    if let Some(forced) = jev_non_cu_force_verdict() {
        crate::behavior_notice::record(
            "jev_test_override",
            "Jev 判断被主机测试设置覆盖；本次使用预设结果，没有调用 Jev。",
            "A host test setting overrides Jev assessment; this uses a preset verdict without calling Jev.",
        );
        return PolicyBroker::deterministic(forced);
    }
    match JevPolicyBroker::from_env() {
        Ok(broker) => PolicyBroker::jev(broker),
        Err(_) => PolicyBroker::deterministic(deterministic_non_cu_verdict("google_drive_upload")),
    }
}

/// Decide allow|ask|deny for a non-CU tool. Does not execute the tool.
pub async fn decide_non_cu(
    broker: &PolicyBroker,
    tool: impl Into<String>,
    args: Value,
    objective: Option<String>,
) -> Result<PolicyDecision> {
    let tool = tool.into();
    if let Some(forced) = jev_non_cu_force_verdict() {
        return Ok(PolicyDecision {
            verdict: forced,
            model: "forced/env".into(),
            evidence: json!({"rule": "AEGIS_JEV_NON_CU_FORCE_VERDICT"}),
            reason: format!("forced verdict {}", forced.as_str()),
        });
    }
    // Diagnostics `force_all` brokers must keep the requested verdict.
    if let PolicyBroker::Deterministic(b) = broker
        && b.force_all
    {
        return b
            .decide(&PolicyInput {
                tool,
                args,
                device_id: "non-cu".into(),
                objective,
            })
            .await;
    }
    // Deterministic broker already maps system.* / shell.exec; for non-CU
    // upload we want Ask even when the default verdict is Allow.
    if matches!(broker, PolicyBroker::Deterministic(_)) && jev_non_cu_force_verdict().is_none() {
        let mapped = deterministic_non_cu_verdict(&tool);
        if !matches!(mapped, PolicyVerdict::Deny) {
            let inner = DeterministicPolicyBroker {
                default_verdict: mapped,
                force_all: false,
            };
            let mut d = inner
                .decide(&PolicyInput {
                    tool: tool.clone(),
                    args,
                    device_id: "non-cu".into(),
                    objective,
                })
                .await?;
            d.verdict = mapped;
            d.reason = format!("deterministic non-cu verdict {}", mapped.as_str());
            return Ok(d);
        }
    }
    broker
        .decide(&PolicyInput {
            tool,
            args,
            device_id: "non-cu".into(),
            objective,
        })
        .await
}

/// Gate one non-CU call. Flag off / resume / unknown tool → Bypass.
pub async fn evaluate_non_cu_gate(
    tool: &str,
    args: &Value,
    objective: Option<String>,
) -> Result<NonCuGateOutcome> {
    evaluate_non_cu_gate_with_broker(tool, args, objective, build_non_cu_policy_broker()).await
}

pub(crate) async fn evaluate_non_cu_gate_with_broker(
    tool: &str,
    args: &Value,
    objective: Option<String>,
    broker: PolicyBroker,
) -> Result<NonCuGateOutcome> {
    if !jev_non_cu_gate_enabled() {
        return Ok(NonCuGateOutcome::Bypass);
    }
    if !is_jev_non_cu_gated_tool(tool) {
        return Ok(NonCuGateOutcome::Bypass);
    }
    if args_owner_approved(args) {
        return Ok(NonCuGateOutcome::Bypass);
    }

    let decision = decide_non_cu(&broker, tool, args.clone(), objective.clone()).await?;
    match decision.verdict {
        PolicyVerdict::Allow => Ok(NonCuGateOutcome::Allow(decision)),
        PolicyVerdict::Deny => Ok(NonCuGateOutcome::Deny(decision)),
        PolicyVerdict::Ask => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Ok(NonCuGateOutcome::Ask(NonCuPendingAsk {
                intent_id: Uuid::new_v4().to_string(),
                tool: tool.to_string(),
                args: args.clone(),
                objective,
                policy: decision,
                proposed_at: now,
            }))
        }
    }
}

/// `/health` fragment (never fails process health).
pub fn health_component() -> Value {
    json!({
        "enabled": jev_non_cu_gate_enabled(),
        "gated_tools": JEV_NON_CU_GATED_TOOLS,
        "force_verdict": jev_non_cu_force_verdict().map(|v| v.as_str()),
        "openrouter_key_present": crate::computer_use::resolve_openrouter_api_key().is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_default_parser() {
        fn parse(v: &str) -> bool {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
        assert!(parse("1"));
        assert!(!parse("0"));
        assert!(!parse(""));
    }

    #[test]
    fn upload_is_gated_shell_is_not() {
        assert!(is_jev_non_cu_gated_tool("google_drive_upload"));
        assert!(!is_jev_non_cu_gated_tool("shell.exec"));
        assert!(!is_jev_non_cu_gated_tool("secure_computer_use"));
    }

    #[test]
    fn deterministic_maps_upload_ask_and_shell_deny() {
        assert_eq!(
            deterministic_non_cu_verdict("google_drive_upload"),
            PolicyVerdict::Ask
        );
        assert_eq!(
            deterministic_non_cu_verdict("shell.exec"),
            PolicyVerdict::Deny
        );
    }

    #[test]
    fn owner_approved_marker_roundtrip() {
        let raw = json!({"filename": "a.txt", "content_text": "hi"});
        assert!(!args_owner_approved(&raw));
        let marked = with_owner_approved_marker(raw.clone());
        assert!(args_owner_approved(&marked));
        let stripped = strip_owner_approved_marker(marked);
        assert_eq!(stripped, raw);
    }

    #[tokio::test]
    async fn decide_forced_allow_overrides_upload_ask() {
        let broker = PolicyBroker::deterministic_forced(PolicyVerdict::Allow);
        let d = decide_non_cu(
            &broker,
            "google_drive_upload",
            json!({"filename": "x.txt"}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Allow);
    }

    #[tokio::test]
    async fn decide_forced_deny_overrides_upload_ask() {
        let broker = PolicyBroker::deterministic_forced(PolicyVerdict::Deny);
        let d = decide_non_cu(
            &broker,
            "google_drive_upload",
            json!({"filename": "x.txt"}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Deny);
    }

    #[tokio::test]
    async fn decide_deterministic_upload_is_ask() {
        let broker = PolicyBroker::deterministic(PolicyVerdict::Allow);
        let d = decide_non_cu(
            &broker,
            "google_drive_upload",
            json!({"filename": "x.txt"}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(d.verdict, PolicyVerdict::Ask);
    }

    #[tokio::test]
    async fn evaluate_bypasses_when_flag_off() {
        // Flag defaults off in unit tests (env unset).
        let out = evaluate_non_cu_gate("google_drive_upload", &json!({}), None)
            .await
            .unwrap();
        assert!(matches!(out, NonCuGateOutcome::Bypass));
    }

    #[tokio::test]
    async fn evaluate_bypasses_owner_approved_resume() {
        let args = with_owner_approved_marker(json!({"filename": "x.txt"}));
        // Even if someone sets the flag in this process, resume must bypass.
        let out = evaluate_non_cu_gate("google_drive_upload", &args, None)
            .await
            .unwrap();
        assert!(matches!(out, NonCuGateOutcome::Bypass));
    }
}
