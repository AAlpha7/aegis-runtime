//! Ed25519 signing for CU intents.
//!
//! Reuses `AgentIdentity` keys (same DID as Wasm intents / executor). Canonical
//! bytes are CU-specific (include `device_id` + lease) so a Wasm envelope
//! signature cannot be replayed as a CU intent.

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::computer_use::intent::{CuIntentBody, CuIntentEnvelope};
use crate::identity::{AgentIdentity, sorted_json};

/// Signed intent ready for Device Agent submission.
pub type SignedCuIntent = CuIntentEnvelope;

/// Thin façade over [`AgentIdentity`] for CU envelopes.
pub struct CuSigner<'a> {
    identity: &'a AgentIdentity,
}

impl<'a> CuSigner<'a> {
    pub fn new(identity: &'a AgentIdentity) -> Self {
        Self { identity }
    }

    pub fn sign(&self, mut body: CuIntentBody) -> Result<CuIntentEnvelope> {
        body.agent_did = self.identity.did.clone();
        let message = canonical_cu_bytes(&body);
        let sig = self.identity.sign_raw(&message);
        Ok(CuIntentEnvelope {
            body,
            signature: hex::encode(sig.to_bytes()),
        })
    }
}

/// Length-prefixed canonical bytes (same anti-boundary style as Wasm intents).
pub fn canonical_cu_bytes(body: &CuIntentBody) -> Vec<u8> {
    let payload = serde_json::json!({
        "version": body.version,
        "intent_id": body.intent_id,
        "agent_did": body.agent_did,
        "device_id": body.device_id,
        "tool": body.tool,
        "args": body.args,
        "issued_at": body.issued_at,
        "lease_expires_at": body.lease_expires_at,
        "policy_verdict": body.policy_verdict,
        "policy_model": body.policy_model,
        "policy_evidence": body.policy_evidence,
    });
    let payload_str = sorted_json(&payload);
    let did = &body.agent_did;
    let intent_id = &body.intent_id;
    let device_id = &body.device_id;
    format!(
        "cu1|{}:{}|{}:{}|{}:{}|{}:{}",
        did.len(),
        did,
        intent_id.len(),
        intent_id,
        device_id.len(),
        device_id,
        payload_str.len(),
        payload_str
    )
    .into_bytes()
}

fn did_to_verifying_key(did: &str) -> Result<VerifyingKey> {
    let hex_part = did
        .strip_prefix("did:aegis:ed25519:")
        .with_context(|| format!("invalid DID format: {did}"))?;
    let bytes = hex::decode(hex_part).context("decoding DID hex")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("DID public key must be 32 bytes"))?;
    VerifyingKey::from_bytes(&arr).context("invalid Ed25519 public key")
}

/// Verify signature only (no lease / device / replay checks).
pub fn verify_cu_signature(envelope: &CuIntentEnvelope) -> Result<()> {
    let vk = did_to_verifying_key(&envelope.body.agent_did)?;
    let message = canonical_cu_bytes(&envelope.body);
    let sig_bytes = hex::decode(&envelope.signature).context("decoding signature hex")?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_arr);
    vk.verify(&message, &signature)
        .context("CU Ed25519 signature verification failed")?;
    Ok(())
}

/// Host-side convenience: reject empty signature early.
pub fn require_signature(envelope: &CuIntentEnvelope) -> Result<()> {
    if envelope.signature.is_empty() {
        bail!("CU intent signature is empty");
    }
    verify_cu_signature(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::policy::PolicyVerdict;
    use serde_json::json;

    fn sample_body(identity: &AgentIdentity) -> CuIntentBody {
        CuIntentBody::new(
            "intent-1",
            identity.did.clone(),
            "dev-loopback-1",
            "system.frontmost_app",
            json!({}),
            1_700_000_000,
            120,
            PolicyVerdict::Ask,
        )
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let identity = AgentIdentity::generate();
        let signed = CuSigner::new(&identity)
            .sign(sample_body(&identity))
            .unwrap();
        assert!(verify_cu_signature(&signed).is_ok());
    }

    #[test]
    fn tampered_tool_is_rejected() {
        let identity = AgentIdentity::generate();
        let mut signed = CuSigner::new(&identity)
            .sign(sample_body(&identity))
            .unwrap();
        signed.body.tool = "shell.exec".into();
        assert!(verify_cu_signature(&signed).is_err());
    }
}
