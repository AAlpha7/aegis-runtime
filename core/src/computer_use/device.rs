//! Device Agent — verify signature, bind device, anti-replay, emit receipt.
//!
//! P0 shipped [`LoopbackDeviceAgent`]. Linux AT-SPI lives in
//! [`crate::computer_use::linux`]; both share [`ProtocolGate`] so verify /
//! device bind / lease / anti-replay / policy reject stay identical.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::computer_use::intent::CuIntentEnvelope;
use crate::computer_use::policy::PolicyVerdict;
use crate::computer_use::signer::verify_cu_signature;

/// Why the Device Agent refused to execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRejectReason {
    TamperedSignature,
    ExpiredLease,
    WrongDevice,
    Replay,
    PolicyDeny,
    PolicyAskRequired,
    UnsupportedTool,
    /// Display / AT-SPI / session bus missing (headless cloud, no desktop).
    AdapterUnavailable,
    Other(String),
}

impl DeviceRejectReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::TamperedSignature => "tampered_signature",
            Self::ExpiredLease => "expired_lease",
            Self::WrongDevice => "wrong_device",
            Self::Replay => "replay",
            Self::PolicyDeny => "policy_deny",
            Self::PolicyAskRequired => "policy_ask_required",
            Self::UnsupportedTool => "unsupported_tool",
            Self::AdapterUnavailable => "adapter_unavailable",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Execution receipt returned to the host (audit + caller).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceReceipt {
    pub intent_id: String,
    pub device_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<DeviceRejectReason>,
    /// Stub result payload (P0 loopback echoes tool name).
    pub result: Value,
    pub observed_at: u64,
    /// BLAKE3 hex of receipt body (without this field) for audit chaining.
    pub receipt_hash: String,
}

pub trait DeviceAgent: Send + Sync {
    fn device_id(&self) -> &str;
    fn execute(&self, envelope: &CuIntentEnvelope) -> Result<DeviceReceipt>;
}

/// Shared verify / bind / lease / anti-replay / policy checks.
pub struct ProtocolGate {
    device_id: String,
    /// When false, `Ask` verdicts are rejected (simulates no owner confirmation yet).
    accept_ask: bool,
    seen: ParkingMutex<HashMap<String, u64>>,
    by_time: ParkingMutex<BTreeMap<(u64, String), ()>>,
    clock: Mutex<Option<u64>>,
}

impl ProtocolGate {
    pub fn new(device_id: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            accept_ask: false,
            seen: ParkingMutex::new(HashMap::new()),
            by_time: ParkingMutex::new(BTreeMap::new()),
            clock: Mutex::new(None),
        }
    }

    pub fn accept_ask(mut self, accept: bool) -> Self {
        self.accept_ask = accept;
        self
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Test hook: freeze "now" for lease checks.
    pub fn set_now_for_test(&self, now: u64) {
        *self.clock.lock().expect("clock lock") = Some(now);
    }

    pub fn now(&self) -> u64 {
        if let Ok(guard) = self.clock.lock()
            && let Some(n) = *guard
        {
            return n;
        }
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn check_replay(&self, signature_hex: &str, now: u64) -> bool {
        let mut seen = self.seen.lock();
        let mut by_time = self.by_time.lock();
        let eviction = now.saturating_sub(600);
        let keep = by_time.split_off(&(eviction, String::new()));
        for (_, sig) in by_time.keys() {
            seen.remove(sig);
        }
        *by_time = keep;
        if seen.contains_key(signature_hex) {
            return false;
        }
        seen.insert(signature_hex.to_string(), now);
        by_time.insert((now, signature_hex.to_string()), ());
        true
    }

    /// Run protocol checks. `Ok(now)` means the adapter may run the tool.
    pub fn validate(&self, envelope: &CuIntentEnvelope) -> Result<u64, DeviceRejectReason> {
        let now = self.now();

        if envelope.body.device_id != self.device_id {
            return Err(DeviceRejectReason::WrongDevice);
        }

        if let Err(err) = verify_cu_signature(envelope) {
            tracing::warn!(error = %err, "CU device agent signature verify failed");
            return Err(DeviceRejectReason::TamperedSignature);
        }

        if now > envelope.body.lease_expires_at {
            return Err(DeviceRejectReason::ExpiredLease);
        }

        if !self.check_replay(&envelope.signature, now) {
            return Err(DeviceRejectReason::Replay);
        }

        match envelope.body.policy_verdict {
            PolicyVerdict::Deny => return Err(DeviceRejectReason::PolicyDeny),
            PolicyVerdict::Ask if !self.accept_ask => {
                return Err(DeviceRejectReason::PolicyAskRequired);
            }
            PolicyVerdict::Allow | PolicyVerdict::Ask => {}
        }

        Ok(now)
    }

    pub fn reject(
        &self,
        envelope: &CuIntentEnvelope,
        reason: DeviceRejectReason,
        now: u64,
    ) -> DeviceReceipt {
        let mut receipt = DeviceReceipt {
            intent_id: envelope.body.intent_id.clone(),
            device_id: self.device_id.clone(),
            accepted: false,
            reject_reason: Some(reason),
            result: serde_json::json!({"status": "rejected"}),
            observed_at: now,
            receipt_hash: String::new(),
        };
        finalize_receipt_hash(&mut receipt);
        receipt
    }

    pub fn accept(&self, envelope: &CuIntentEnvelope, now: u64, result: Value) -> DeviceReceipt {
        let mut receipt = DeviceReceipt {
            intent_id: envelope.body.intent_id.clone(),
            device_id: self.device_id.clone(),
            accepted: true,
            reject_reason: None,
            result,
            observed_at: now,
            receipt_hash: String::new(),
        };
        finalize_receipt_hash(&mut receipt);
        receipt
    }

    /// Reject with a custom result payload (e.g. AT-SPI probe details).
    pub fn reject_with_result(
        &self,
        envelope: &CuIntentEnvelope,
        reason: DeviceRejectReason,
        now: u64,
        result: Value,
    ) -> DeviceReceipt {
        let mut receipt = DeviceReceipt {
            intent_id: envelope.body.intent_id.clone(),
            device_id: self.device_id.clone(),
            accepted: false,
            reject_reason: Some(reason),
            result,
            observed_at: now,
            receipt_hash: String::new(),
        };
        finalize_receipt_hash(&mut receipt);
        receipt
    }
}

/// In-process Device Agent for protocol red-team vectors and unit tests.
pub struct LoopbackDeviceAgent {
    gate: ProtocolGate,
}

impl LoopbackDeviceAgent {
    pub fn new(device_id: impl Into<String>) -> Self {
        Self {
            gate: ProtocolGate::new(device_id),
        }
    }

    pub fn accept_ask(mut self, accept: bool) -> Self {
        self.gate = self.gate.accept_ask(accept);
        self
    }

    /// Test hook: freeze "now" for lease checks.
    pub fn set_now_for_test(&self, now: u64) {
        self.gate.set_now_for_test(now);
    }
}

impl DeviceAgent for LoopbackDeviceAgent {
    fn device_id(&self) -> &str {
        self.gate.device_id()
    }

    fn execute(&self, envelope: &CuIntentEnvelope) -> Result<DeviceReceipt> {
        let now = match self.gate.validate(envelope) {
            Ok(n) => n,
            Err(reason) => return Ok(self.gate.reject(envelope, reason, self.gate.now())),
        };

        // Loopback never touches a real OS — reject obvious shell tools.
        if envelope.body.tool.contains("shell") || envelope.body.tool.contains("eval") {
            return Ok(self
                .gate
                .reject(envelope, DeviceRejectReason::UnsupportedTool, now));
        }

        let result = if crate::computer_use::readonly::is_cu_readonly_tool(&envelope.body.tool) {
            crate::computer_use::readonly::loopback_readonly_result(&envelope.body.tool)
        } else {
            serde_json::json!({
                "status": "ok",
                "adapter": "loopback",
                "tool": envelope.body.tool,
                "note": "stub — no OS side effects",
            })
        };

        Ok(self.gate.accept(envelope, now, result))
    }
}

/// Recompute `receipt_hash` after mutating receipt fields.
pub fn finalize_receipt_hash(receipt: &mut DeviceReceipt) {
    receipt.receipt_hash.clear();
    let bytes = serde_jcs::to_string(&*receipt)
        .unwrap_or_else(|_| "{}".to_string())
        .into_bytes();
    receipt.receipt_hash = hex::encode(blake3::hash(&bytes).as_bytes());
}

/// Helper used by pipeline tests when a hard error is preferred over a receipt.
pub fn bail_on_reject(receipt: &DeviceReceipt) -> Result<()> {
    if receipt.accepted {
        return Ok(());
    }
    bail!(
        "device agent rejected intent {}: {}",
        receipt.intent_id,
        receipt
            .reject_reason
            .as_ref()
            .map(DeviceRejectReason::as_str)
            .unwrap_or("unknown")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::intent::CuIntentBody;
    use crate::computer_use::policy::PolicyVerdict;
    use crate::computer_use::signer::CuSigner;
    use crate::identity::AgentIdentity;
    use serde_json::json;

    fn sign_for(
        device_id: &str,
        lease_secs: u64,
        verdict: PolicyVerdict,
    ) -> (AgentIdentity, CuIntentEnvelope) {
        let identity = AgentIdentity::generate();
        let now = 1_700_000_000_u64;
        let body = CuIntentBody::new(
            "intent-loop",
            identity.did.clone(),
            device_id,
            "system.frontmost_app",
            json!({}),
            now,
            lease_secs,
            verdict,
        );
        let env = CuSigner::new(&identity).sign(body).unwrap();
        (identity, env)
    }

    #[test]
    fn accepts_allow_on_matching_device() {
        let (_id, env) = sign_for("dev-1", 300, PolicyVerdict::Allow);
        let agent = LoopbackDeviceAgent::new("dev-1");
        agent.set_now_for_test(1_700_000_000);
        let receipt = agent.execute(&env).unwrap();
        assert!(receipt.accepted);
        assert!(!receipt.receipt_hash.is_empty());
    }

    #[test]
    fn rejects_wrong_device() {
        let (_id, env) = sign_for("dev-A", 300, PolicyVerdict::Allow);
        let agent = LoopbackDeviceAgent::new("dev-B");
        agent.set_now_for_test(1_700_000_000);
        let receipt = agent.execute(&env).unwrap();
        assert!(!receipt.accepted);
        assert_eq!(receipt.reject_reason, Some(DeviceRejectReason::WrongDevice));
    }

    #[test]
    fn rejects_expired_lease() {
        let (_id, env) = sign_for("dev-1", 10, PolicyVerdict::Allow);
        let agent = LoopbackDeviceAgent::new("dev-1");
        agent.set_now_for_test(1_700_000_000 + 11);
        let receipt = agent.execute(&env).unwrap();
        assert_eq!(
            receipt.reject_reason,
            Some(DeviceRejectReason::ExpiredLease)
        );
    }

    #[test]
    fn rejects_replay() {
        let (_id, env) = sign_for("dev-1", 300, PolicyVerdict::Allow);
        let agent = LoopbackDeviceAgent::new("dev-1");
        agent.set_now_for_test(1_700_000_000);
        assert!(agent.execute(&env).unwrap().accepted);
        let second = agent.execute(&env).unwrap();
        assert_eq!(second.reject_reason, Some(DeviceRejectReason::Replay));
    }

    #[test]
    fn loopback_accepts_new_readonly_tools() {
        for tool in [
            "system.accessible_tree",
            "system.focused_control",
            "system.screenshot_digest",
        ] {
            let identity = AgentIdentity::generate();
            let now = 1_700_000_000_u64;
            let body = CuIntentBody::new(
                format!("intent-{tool}"),
                identity.did.clone(),
                "dev-1",
                tool,
                json!({}),
                now,
                300,
                PolicyVerdict::Allow,
            );
            let env = CuSigner::new(&identity).sign(body).unwrap();
            let agent = LoopbackDeviceAgent::new("dev-1");
            agent.set_now_for_test(now);
            let receipt = agent.execute(&env).unwrap();
            assert!(receipt.accepted, "{tool}");
            assert_eq!(receipt.result["adapter"], "loopback");
        }
    }

    #[test]
    fn rejects_tampered_signature() {
        let (_id, mut env) = sign_for("dev-1", 300, PolicyVerdict::Allow);
        let mut bytes = hex::decode(&env.signature).unwrap();
        bytes[0] ^= 0xff;
        env.signature = hex::encode(bytes);
        let agent = LoopbackDeviceAgent::new("dev-1");
        agent.set_now_for_test(1_700_000_000);
        let receipt = agent.execute(&env).unwrap();
        assert_eq!(
            receipt.reject_reason,
            Some(DeviceRejectReason::TamperedSignature)
        );
    }
}
