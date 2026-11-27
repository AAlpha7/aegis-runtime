//! Computer Use Intent Envelope v1.
//!
//! Sibling to Wasm `IntentEnvelope`, not a replacement. CU intents bind a
//! tool call to a specific `device_id` and lease window so a Device Agent can
//! reject wrong-device / expired leases before any OS adapter runs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::computer_use::policy::PolicyVerdict;

/// Wire version for the CU intent schema. Bump only with an explicit migration.
pub const CU_INTENT_VERSION: u32 = 1;

/// Unsigned intent body (everything covered by the Ed25519 signature).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CuIntentBody {
    pub version: u32,
    /// Stable id for audit / anti-replay correlation (UUID string).
    pub intent_id: String,
    /// Host agent DID (`did:aegis:ed25519:<pubkey_hex>`).
    pub agent_did: String,
    /// Target device identity. Device Agent rejects mismatches.
    pub device_id: String,
    /// Logical tool name (P1+: `system.frontmost_app`, `browser.open_url`, …).
    pub tool: String,
    /// Tool arguments (JSON object). Keep small; no secrets.
    pub args: Value,
    /// Unix seconds when the host issued this intent.
    pub issued_at: u64,
    /// Unix seconds when the lease expires. Device rejects after this.
    pub lease_expires_at: u64,
    /// Policy Broker verdict recorded into the signed payload (tamper-evident).
    pub policy_verdict: PolicyVerdict,
    /// Optional Policy Broker model id that produced the verdict (e.g. Jev).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_model: Option<String>,
    /// Optional confidence / probability snapshot from Jev (opaque JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_evidence: Option<Value>,
}

/// Signed CU intent envelope ready for Device Agent submission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CuIntentEnvelope {
    pub body: CuIntentBody,
    /// Hex-encoded Ed25519 signature over [`crate::computer_use::signer::canonical_cu_bytes`].
    pub signature: String,
}

impl CuIntentBody {
    pub fn new(
        intent_id: impl Into<String>,
        agent_did: impl Into<String>,
        device_id: impl Into<String>,
        tool: impl Into<String>,
        args: Value,
        issued_at: u64,
        lease_secs: u64,
        policy_verdict: PolicyVerdict,
    ) -> Self {
        Self {
            version: CU_INTENT_VERSION,
            intent_id: intent_id.into(),
            agent_did: agent_did.into(),
            device_id: device_id.into(),
            tool: tool.into(),
            args,
            issued_at,
            lease_expires_at: issued_at.saturating_add(lease_secs),
            policy_verdict,
            policy_model: None,
            policy_evidence: None,
        }
    }
}
