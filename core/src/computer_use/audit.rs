//! Append-only audit hash chain for Secure Computer Use events.
//!
//! Each record's `event_hash` is BLAKE3 over RFC 8785 canonical JSON of the
//! record with `event_hash` cleared, chained via `prev_hash`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    IntentProposed,
    PolicyDecided,
    IntentSigned,
    DeviceAccepted,
    DeviceRejected,
    PipelineFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditRecord {
    pub seq: u64,
    pub event: AuditEvent,
    pub intent_id: String,
    pub observed_at: u64,
    pub prev_hash: String,
    pub payload: Value,
    pub event_hash: String,
}

#[derive(Debug, Default)]
pub struct AuditChain {
    records: Vec<AuditRecord>,
    tip: [u8; 32],
}

impl AuditChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tip_hex(&self) -> String {
        hex::encode(self.tip)
    }

    pub fn records(&self) -> &[AuditRecord] {
        &self.records
    }

    pub fn append(
        &mut self,
        event: AuditEvent,
        intent_id: impl Into<String>,
        observed_at: u64,
        payload: Value,
    ) -> Result<&AuditRecord> {
        let seq = self.records.len() as u64 + 1;
        let mut record = AuditRecord {
            seq,
            event,
            intent_id: intent_id.into(),
            observed_at,
            prev_hash: hex::encode(self.tip),
            payload,
            event_hash: String::new(),
        };
        record.event_hash = hash_record(&record)?;
        let tip_bytes = hex::decode(&record.event_hash).context("decoding event_hash")?;
        let mut tip = [0u8; 32];
        tip.copy_from_slice(&tip_bytes);
        self.tip = tip;
        self.records.push(record);
        Ok(self.records.last().expect("just pushed"))
    }

    /// Recompute hashes and ensure the chain is intact.
    pub fn verify(&self) -> Result<()> {
        let mut tip = [0u8; 32];
        for (i, record) in self.records.iter().enumerate() {
            if record.prev_hash != hex::encode(tip) {
                anyhow::bail!("audit chain break at seq {}: bad prev_hash", i + 1);
            }
            let expected = hash_record(record)?;
            if expected != record.event_hash {
                anyhow::bail!("audit chain break at seq {}: bad event_hash", i + 1);
            }
            let bytes = hex::decode(&record.event_hash)?;
            tip.copy_from_slice(&bytes);
        }
        Ok(())
    }
}

fn hash_record(record: &AuditRecord) -> Result<String> {
    let mut for_hash = record.clone();
    for_hash.event_hash.clear();
    let bytes = serde_jcs::to_string(&for_hash)
        .context("JCS serialize audit record")?
        .into_bytes();
    Ok(hex::encode(blake3::hash(&bytes).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chain_verifies_and_detects_tamper() {
        let mut chain = AuditChain::new();
        chain
            .append(AuditEvent::IntentProposed, "i1", 1, json!({"a": 1}))
            .unwrap();
        chain
            .append(AuditEvent::PolicyDecided, "i1", 2, json!({"v": "ask"}))
            .unwrap();
        assert!(chain.verify().is_ok());
        chain.records[1].payload = json!({"v": "allow"});
        assert!(chain.verify().is_err());
    }
}
