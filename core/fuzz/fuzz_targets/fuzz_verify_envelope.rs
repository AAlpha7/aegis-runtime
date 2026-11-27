//! Fuzz the IntentEnvelope signature verification path.
//!
//! Goals:
//! - Ensure `verify_envelope` never panics on arbitrary input
//! - Detect subtle edge cases in canonical_message construction
//!   (e.g., length-prefix confusion via crafted field values)
//! - Confirm anti-replay timestamp checks are robust
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use aegis_runtime_core::identity::{canonical_message, sorted_json, verify_envelope};
use aegis_runtime_core::models::IntentEnvelope;

/// Structured fuzzer input — gives the fuzzer better coverage guidance
/// than raw bytes by generating valid-ish envelopes.
#[derive(Debug, Arbitrary)]
struct FuzzEnvelope {
    agent_did: String,
    timestamp: u64,
    target_skill: String,
    /// Raw bytes that will be interpreted as JSON payload
    payload_bytes: Vec<u8>,
    signature: String,
}

fuzz_target!(|input: FuzzEnvelope| {
    // Construct a JSON payload from arbitrary bytes (may or may not be valid JSON)
    let payload = match serde_json::from_slice::<serde_json::Value>(&input.payload_bytes) {
        Ok(v) => v,
        // If not valid JSON, wrap as a string
        Err(_) => serde_json::Value::String(String::from_utf8_lossy(&input.payload_bytes).into()),
    };

    let envelope = IntentEnvelope {
        agent_did: input.agent_did.clone(),
        timestamp: input.timestamp,
        target_skill: input.target_skill.clone(),
        payload: payload.clone(),
        signature: input.signature,
    };

    // Must not panic — any panic is a bug
    let _ = verify_envelope(&envelope);

    // Also fuzz canonical_message directly — it must be deterministic
    // and never panic regardless of input
    let cm1 = canonical_message(&input.agent_did, input.timestamp, &input.target_skill, &payload);
    let cm2 = canonical_message(&input.agent_did, input.timestamp, &input.target_skill, &payload);
    assert_eq!(cm1, cm2, "canonical_message must be deterministic");

    // sorted_json must not panic on any Value
    let _ = sorted_json(&payload);
});
