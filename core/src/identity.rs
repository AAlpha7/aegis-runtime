use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::models::IntentEnvelope;

/// Maximum age of an IntentEnvelope before it is rejected (anti-replay).
const MAX_ENVELOPE_AGE_SECS: u64 = 300; // 5 minutes

/// Hard cap on the number of entries in the ReplayGuard.
/// Prevents unbounded memory growth in long-running production deployments.
/// At 10-second heartbeat intervals, 50,000 entries ≈ ~6 days of signatures.
const MAX_REPLAY_ENTRIES: usize = 50_000;

/// Tracks recently seen envelope signatures to prevent replay within the time window.
/// Signatures are evicted once they are older than MAX_ENVELOPE_AGE_SECS + 60s.
///
/// Uses a dual data structure for efficient operations:
/// - `seen` HashMap for O(1) duplicate lookups
/// - `by_time` BTreeMap keyed by (timestamp, signature) for O(log N) ordered eviction
pub struct ReplayGuard {
    /// Map of signature_hex → timestamp_of_first_seen (O(1) lookup)
    seen: Mutex<HashMap<String, u64>>,
    /// Time-ordered index for O(log N) eviction: (timestamp, signature_hex) → ()
    by_time: Mutex<BTreeMap<(u64, String), ()>>,
}

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayGuard {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
            by_time: Mutex::new(BTreeMap::new()),
        }
    }

    /// Check if this signature has been seen before. If not, record it.
    /// Returns `true` if the signature is fresh (not a replay).
    /// Returns `false` if it's a duplicate (replay attack).
    pub fn check_and_record(&self, signature_hex: &str) -> bool {
        let now = current_unix_secs();
        let mut seen = self.seen.lock();
        let mut by_time = self.by_time.lock();

        // Time-based eviction: remove entries older than the replay window + skew.
        // BTreeMap is sorted by (timestamp, sig), so we split at the threshold
        // and drain everything below it in O(K log N) where K = evicted entries.
        let eviction_threshold = now.saturating_sub(MAX_ENVELOPE_AGE_SECS + 60);
        let split_key = (eviction_threshold, String::new());
        // split_off returns everything >= split_key; we keep that, drain the rest
        let keep = by_time.split_off(&split_key);
        // Everything left in by_time is older than the threshold — remove from seen
        for (_, sig) in by_time.keys() {
            seen.remove(sig);
        }
        *by_time = keep;

        // Hard cap: if still over limit, drop oldest entries until under cap.
        // Since by_time is already sorted, we just pop from the front — O(K log N).
        if seen.len() >= MAX_REPLAY_ENTRIES {
            let drop_count = MAX_REPLAY_ENTRIES / 5;
            for _ in 0..drop_count {
                if let Some(((_, sig), _)) = by_time.pop_first() {
                    seen.remove(&sig);
                } else {
                    break;
                }
            }
        }

        // Check for duplicate
        if seen.contains_key(signature_hex) {
            return false; // replay detected
        }

        seen.insert(signature_hex.to_string(), now);
        by_time.insert((now, signature_hex.to_string()), ());
        true // fresh envelope
    }

    /// Load persisted nonces from SQLite into in-memory maps (call once at startup).
    pub async fn load_from_db(&self, handle: &crate::memory::MemoryHandle) -> Result<usize> {
        let now = current_unix_secs();
        let nonces = handle.load_replay_nonces(now as i64).await?;
        let count = nonces.len();
        let mut seen = self.seen.lock();
        let mut by_time = self.by_time.lock();
        for (sig_hex, expires_at) in nonces {
            if !seen.contains_key(&sig_hex) {
                seen.insert(sig_hex.clone(), expires_at);
                by_time.insert((expires_at, sig_hex), ());
            }
        }
        tracing::info!(count, "Loaded replay nonces from SQLite");
        Ok(count)
    }

    /// Persist a nonce to SQLite (fire-and-forget). Call after check_and_record returns true.
    pub fn persist_nonce(
        &self,
        handle: &crate::memory::MemoryHandle,
        sig_hex: &str,
        expires_at: u64,
    ) {
        handle.persist_replay_nonce(sig_hex.to_string(), expires_at as i64);
    }

    /// Delete expired nonces from SQLite (fire-and-forget). Call periodically.
    pub fn cleanup_db_nonces(&self, handle: &crate::memory::MemoryHandle) {
        let now = current_unix_secs();
        handle.delete_expired_replay_nonces(now as i64);
    }
}

/// Manages an Ed25519 key pair for a single agent identity.
pub struct AgentIdentity {
    signing_key: SigningKey,
    pub did: String,
}

impl AgentIdentity {
    /// Generate a fresh key pair. The DID is derived from the public key.
    pub fn generate() -> Self {
        // Use OS-level CSPRNG directly for cryptographic key material.
        // ed25519-dalek 2.x requires rand_core 0.6 (via curve25519-dalek),
        // while rand 0.9 uses rand_core 0.9. Using getrandom (the shared
        // base of both rand_core versions) avoids this version conflict.
        let mut key_bytes = [0u8; 32];
        getrandom::fill(&mut key_bytes).expect("OS RNG failed");
        let signing_key = SigningKey::from_bytes(&key_bytes);
        let did = pubkey_to_did(signing_key.verifying_key());
        Self { signing_key, did }
    }

    /// Load an existing key pair from a 32-byte hex file, or generate and persist
    /// a new one if the file does not exist.
    pub fn load_or_generate(path: &Path) -> Result<Self> {
        if path.exists() {
            let hex = std::fs::read_to_string(path)
                .with_context(|| format!("reading key file {}", path.display()))?;
            let trimmed = hex.trim();
            // Guard against corrupt/empty key files (e.g., from interrupted write)
            if trimmed.is_empty() {
                tracing::warn!(
                    path = %path.display(),
                    "Key file exists but is empty (likely interrupted write). Regenerating."
                );
                std::fs::remove_file(path)?;
                return Self::load_or_generate(path);
            }
            let bytes = hex::decode(trimmed).with_context(|| "decoding hex key")?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("key file must be exactly 32 bytes"))?;
            let signing_key = SigningKey::from_bytes(&arr);
            let did = pubkey_to_did(signing_key.verifying_key());
            Ok(Self { signing_key, did })
        } else {
            let identity = Self::generate();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Atomic write: write to temp file, sync, then rename to prevent
            // bricking on interrupted writes (power loss, OOM-kill, Ctrl+C).
            let tmp_path = path.with_extension("key.tmp");
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp_path)?;
                file.write_all(hex::encode(identity.signing_key.to_bytes()).as_bytes())?;
                file.sync_all()?;
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&tmp_path, hex::encode(identity.signing_key.to_bytes()))?;
            }
            std::fs::rename(&tmp_path, path).with_context(|| {
                format!("atomic rename {} → {}", tmp_path.display(), path.display())
            })?;
            Ok(identity)
        }
    }

    /// Sign an IntentEnvelope. The signature covers the canonical message:
    ///   agent_did | timestamp | target_skill | payload_json (keys sorted)
    pub fn sign(&self, target_skill: &str, payload: serde_json::Value) -> Result<IntentEnvelope> {
        let timestamp = current_unix_secs();
        let message = canonical_message(&self.did, timestamp, target_skill, &payload);
        let signature: Signature = self.signing_key.sign(&message);
        Ok(IntentEnvelope {
            agent_did: self.did.clone(),
            timestamp,
            target_skill: target_skill.to_string(),
            payload,
            signature: hex::encode(signature.to_bytes()),
        })
    }

    /// Return the verifying key (public key) for this identity.
    /// Used by the orchestrator (Phase 4) to expose the agent's public key.
    #[allow(dead_code)]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Sign arbitrary bytes with the Ed25519 signing key.
    /// Used for HITL executor command signing.
    pub fn sign_raw(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Return the hex-encoded public key (32 bytes → 64 hex chars).
    /// Used to configure the Aegis Executor's AEGIS_PUBKEY_HEX.
    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    /// Return the raw 32-byte signing key. Used for deriving encryption keys
    /// (e.g. credential store AES key via HKDF).
    pub fn signing_key_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }
}

/// Verify an IntentEnvelope:
///   1. Parse the DID to extract the public key.
///   2. Re-derive the canonical message and verify the signature.
///   3. Reject if the timestamp is older than MAX_ENVELOPE_AGE_SECS (anti-replay).
///   4. Reject if the signature has been seen before (nonce deduplication).
pub fn verify_envelope(envelope: &IntentEnvelope) -> Result<()> {
    verify_envelope_with_guard(envelope, None, None)
}

/// Verify with an optional ReplayGuard for nonce-based deduplication.
///
/// `expected_did`: When provided, the envelope's `agent_did` must match this value.
/// This prevents self-signed forgery where an attacker generates their own key pair
/// and crafts envelopes that verify against the DID embedded in the envelope itself.
/// In production, pass the system identity's DID to ensure only envelopes signed by
/// the system key are accepted.
pub fn verify_envelope_with_guard(
    envelope: &IntentEnvelope,
    replay_guard: Option<&ReplayGuard>,
    expected_did: Option<&str>,
) -> Result<()> {
    // Step 0: DID pinning — reject envelopes from unexpected identities
    if let Some(trusted) = expected_did
        && envelope.agent_did != trusted
    {
        bail!(
            "envelope DID mismatch: expected '{}', got '{}'",
            trusted,
            envelope.agent_did
        );
    }

    // Anti-replay: check timestamp freshness
    let now = current_unix_secs();

    // Reject future-dated envelopes (max 60s clock skew tolerance)
    if envelope.timestamp > now + 60 {
        bail!(
            "envelope timestamp is {}s in the future (max skew: 60s)",
            envelope.timestamp - now
        );
    }

    let age = now.saturating_sub(envelope.timestamp);
    if age > MAX_ENVELOPE_AGE_SECS {
        bail!(
            "envelope expired: age {}s exceeds maximum {}s",
            age,
            MAX_ENVELOPE_AGE_SECS
        );
    }

    // Parse DID → verifying key
    let verifying_key =
        did_to_verifying_key(&envelope.agent_did).context("parsing DID from envelope")?;

    // Re-derive canonical message
    let message = canonical_message(
        &envelope.agent_did,
        envelope.timestamp,
        &envelope.target_skill,
        &envelope.payload,
    );

    // Decode and verify signature
    let sig_bytes = hex::decode(&envelope.signature).context("decoding signature hex")?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_arr);

    verifying_key
        .verify(&message, &signature)
        .context("Ed25519 signature verification failed")?;

    // Step 4: Nonce deduplication — reject replayed envelopes
    if let Some(guard) = replay_guard
        && !guard.check_and_record(&envelope.signature)
    {
        bail!(
            "envelope replay detected: signature {} already used",
            &envelope.signature[..16]
        );
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn pubkey_to_did(vk: VerifyingKey) -> String {
    format!("did:aegis:ed25519:{}", hex::encode(vk.as_bytes()))
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

/// Canonical message bytes for signing/verifying.
/// Format: length-prefixed fields to prevent boundary ambiguity attacks.
/// Each field is encoded as `<byte_len>:<data>` (netstring-style).
/// Example: "16:did:aegis:ed25519:...|10:1234567890|14:crypto_tracker|23:{"asset":"BTC",...}"
///
/// This prevents an attacker from crafting a `target_skill` containing `|`
/// characters to shift field boundaries and forge a signature collision.
#[doc(hidden)]
pub fn canonical_message(
    did: &str,
    timestamp: u64,
    target_skill: &str,
    payload: &serde_json::Value,
) -> Vec<u8> {
    // Use sorted-key JSON to ensure deterministic serialization
    let payload_str = sorted_json(payload);
    let ts_str = timestamp.to_string();
    format!(
        "{}:{}|{}:{}|{}:{}|{}:{}",
        did.len(),
        did,
        ts_str.len(),
        ts_str,
        target_skill.len(),
        target_skill,
        payload_str.len(),
        payload_str
    )
    .into_bytes()
}

/// Recursively sort object keys for deterministic JSON serialization.
#[doc(hidden)]
pub fn sorted_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let inner: Vec<String> = pairs
                .iter()
                .map(|(k, v)| {
                    // Use serde_json::to_string to properly escape special
                    // characters in keys (e.g. quotes, backslashes) —
                    // prevents signature collision via crafted key names.
                    // Panic on serialization failure rather than using an unsafe
                    // fallback that could create signature collisions.
                    let escaped_key =
                        serde_json::to_string(k).expect("JSON key serialization should never fail");
                    format!("{}:{}", escaped_key, sorted_json(v))
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(sorted_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    // T1: Valid signature round-trips correctly; tampered fields are rejected.
    #[test]
    fn t1_sign_and_verify_roundtrip() {
        let identity = AgentIdentity::generate();
        let payload = json!({"threshold": 60_000, "asset": "BTC"});
        let envelope = identity.sign("crypto_tracker", payload).unwrap();

        // Valid envelope must pass
        assert!(
            verify_envelope(&envelope).is_ok(),
            "valid envelope should pass verification"
        );
    }

    #[test]
    fn t1_tampered_payload_is_rejected() {
        let identity = AgentIdentity::generate();
        let payload = json!({"threshold": 60_000});
        let mut envelope = identity.sign("crypto_tracker", payload).unwrap();

        // Tamper with the payload after signing
        envelope.payload = json!({"threshold": 1});

        assert!(
            verify_envelope(&envelope).is_err(),
            "tampered payload must be rejected"
        );
    }

    #[test]
    fn t1_tampered_skill_is_rejected() {
        let identity = AgentIdentity::generate();
        let payload = json!({});
        let mut envelope = identity.sign("crypto_tracker", payload).unwrap();

        // Tamper with the target skill name
        envelope.target_skill = "system_exec".to_string();

        assert!(
            verify_envelope(&envelope).is_err(),
            "tampered target_skill must be rejected"
        );
    }

    #[test]
    fn t1_tampered_signature_is_rejected() {
        let identity = AgentIdentity::generate();
        let payload = json!({});
        let mut envelope = identity.sign("web_reader", payload).unwrap();

        // Flip the first byte of the signature
        let mut sig_bytes = hex::decode(&envelope.signature).unwrap();
        sig_bytes[0] ^= 0xFF;
        envelope.signature = hex::encode(sig_bytes);

        assert!(
            verify_envelope(&envelope).is_err(),
            "tampered signature must be rejected"
        );
    }

    #[test]
    fn t1_wrong_key_is_rejected() {
        let identity_a = AgentIdentity::generate();
        let identity_b = AgentIdentity::generate();
        let payload = json!({});

        // Sign with identity A, but set DID to identity B's
        let mut envelope = identity_a.sign("web_reader", payload).unwrap();
        envelope.agent_did = identity_b.did.clone();

        assert!(
            verify_envelope(&envelope).is_err(),
            "envelope signed by wrong key must be rejected"
        );
    }

    // T2: Replay attack — envelope older than 5 minutes must be rejected.
    #[test]
    fn t2_fresh_envelope_is_accepted() {
        let identity = AgentIdentity::generate();
        let envelope = identity.sign("crypto_tracker", json!({})).unwrap();
        assert!(
            verify_envelope(&envelope).is_ok(),
            "fresh envelope must be accepted"
        );
    }

    #[test]
    fn t2_expired_envelope_is_rejected() {
        let identity = AgentIdentity::generate();
        let payload = json!({});

        // Manually craft an envelope with a timestamp 6 minutes in the past
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(361); // 6 min 1 sec ago → expired

        let message = canonical_message(&identity.did, old_timestamp, "crypto_tracker", &payload);
        let sig: Signature = identity.signing_key.sign(&message);

        let stale_envelope = IntentEnvelope {
            agent_did: identity.did.clone(),
            timestamp: old_timestamp,
            target_skill: "crypto_tracker".to_string(),
            payload,
            signature: hex::encode(sig.to_bytes()),
        };

        assert!(
            verify_envelope(&stale_envelope).is_err(),
            "expired envelope must be rejected (anti-replay)"
        );
    }

    #[test]
    fn t2_boundary_just_within_window_is_accepted() {
        let identity = AgentIdentity::generate();
        let payload = json!({});

        // 4 minutes 59 seconds ago — still valid
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(299);

        let message = canonical_message(&identity.did, timestamp, "web_reader", &payload);
        let sig: Signature = identity.signing_key.sign(&message);

        let envelope = IntentEnvelope {
            agent_did: identity.did.clone(),
            timestamp,
            target_skill: "web_reader".to_string(),
            payload,
            signature: hex::encode(sig.to_bytes()),
        };

        assert!(
            verify_envelope(&envelope).is_ok(),
            "envelope within 5-minute window must be accepted"
        );
    }

    #[test]
    fn t2_future_timestamp_is_rejected() {
        let identity = AgentIdentity::generate();
        let payload = json!({});

        // 2 minutes in the future — exceeds 60s clock skew tolerance
        let future_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 120;

        let message = canonical_message(&identity.did, future_timestamp, "web_reader", &payload);
        let sig: Signature = identity.signing_key.sign(&message);

        let envelope = IntentEnvelope {
            agent_did: identity.did.clone(),
            timestamp: future_timestamp,
            target_skill: "web_reader".to_string(),
            payload,
            signature: hex::encode(sig.to_bytes()),
        };

        let result = verify_envelope(&envelope);
        assert!(result.is_err(), "future-dated envelope must be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("future"),
            "error should mention future timestamp: {err_msg}"
        );
    }

    #[test]
    fn t2_minor_clock_skew_is_accepted() {
        let identity = AgentIdentity::generate();
        let payload = json!({});

        // 30 seconds in the future — within 60s tolerance
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30;

        let message = canonical_message(&identity.did, timestamp, "web_reader", &payload);
        let sig: Signature = identity.signing_key.sign(&message);

        let envelope = IntentEnvelope {
            agent_did: identity.did.clone(),
            timestamp,
            target_skill: "web_reader".to_string(),
            payload,
            signature: hex::encode(sig.to_bytes()),
        };

        assert!(
            verify_envelope(&envelope).is_ok(),
            "minor clock skew (30s) must be accepted"
        );
    }

    #[test]
    fn key_persist_and_reload() {
        // Use a temp path
        let dir = std::env::temp_dir().join(format!("aegis_runtime_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("agent.key");

        let id1 = AgentIdentity::load_or_generate(&key_path).unwrap();
        let id2 = AgentIdentity::load_or_generate(&key_path).unwrap();

        // Both loads must yield the same DID
        assert_eq!(id1.did, id2.did, "reloaded identity must match original");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn key_file_created_with_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("aegis_runtime_perm_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("test_perms.key");

        // Ensure no leftover from previous run
        let _ = std::fs::remove_file(&key_path);

        let _identity = AgentIdentity::load_or_generate(&key_path).unwrap();

        let metadata = std::fs::metadata(&key_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "key file must be owner-only (0600), got {:o}",
            mode
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn key_file_create_new_prevents_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "aegis_runtime_overwrite_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("existing.key");

        // Create first identity
        let id1 = AgentIdentity::load_or_generate(&key_path).unwrap();

        // Second call should load, not regenerate
        let id2 = AgentIdentity::load_or_generate(&key_path).unwrap();
        assert_eq!(
            id1.did, id2.did,
            "loading existing key must return same identity"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ── ReplayGuard tests ─────────────────────────────────────────────

    #[test]
    fn replay_guard_blocks_duplicate_signature() {
        let guard = ReplayGuard::new();
        let sig = "abcdef1234567890abcdef1234567890";

        assert!(
            guard.check_and_record(sig),
            "first use of signature must be accepted"
        );
        assert!(
            !guard.check_and_record(sig),
            "duplicate signature must be rejected (replay attack)"
        );
    }

    #[test]
    fn replay_guard_allows_different_signatures() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record("sig_a"));
        assert!(guard.check_and_record("sig_b"));
        assert!(guard.check_and_record("sig_c"));
        // But repeats are still blocked
        assert!(!guard.check_and_record("sig_a"));
    }

    #[test]
    fn replay_guard_with_verify_envelope() {
        let guard = ReplayGuard::new();
        let identity = AgentIdentity::generate();
        let envelope = identity.sign("web_reader", json!({})).unwrap();

        // First verification: passes
        assert!(
            verify_envelope_with_guard(&envelope, Some(&guard), None).is_ok(),
            "first use of envelope must pass"
        );

        // Second verification: replay detected
        let result = verify_envelope_with_guard(&envelope, Some(&guard), None);
        assert!(result.is_err(), "replayed envelope must be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("replay"),
            "error should mention replay: {err_msg}"
        );
    }

    #[test]
    fn replay_guard_concurrent_safety() {
        use std::sync::Arc;
        let guard = Arc::new(ReplayGuard::new());
        let mut handles = vec![];

        // 10 threads all try to submit the same signature
        for _ in 0..10 {
            let g = guard.clone();
            handles.push(std::thread::spawn(move || {
                g.check_and_record("concurrent_sig")
            }));
        }

        let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Exactly one thread should succeed
        let accept_count = results.iter().filter(|&&r| r).count();
        assert_eq!(
            accept_count, 1,
            "exactly one thread should accept the signature, got {accept_count}"
        );
    }

    #[test]
    fn sorted_json_nested_array_with_objects() {
        // Arrays containing objects must have their object keys sorted recursively
        let value = json!({
            "items": [
                {"zebra": 1, "apple": 2},
                {"mango": [{"z_inner": true, "a_inner": false}], "banana": 3}
            ],
            "name": "test"
        });
        let result = sorted_json(&value);

        // Top-level keys sorted: "items" before "name"
        // Inner objects sorted: "apple" before "zebra", "banana" before "mango"
        // Deeply nested object sorted: "a_inner" before "z_inner"
        assert_eq!(
            result,
            r#"{"items":[{"apple":2,"zebra":1},{"banana":3,"mango":[{"a_inner":false,"z_inner":true}]}],"name":"test"}"#
        );
    }

    #[test]
    fn sorted_json_plain_array() {
        // Arrays of primitives should be preserved as-is
        let value = json!([3, 1, 2]);
        assert_eq!(sorted_json(&value), "[3,1,2]");
    }

    #[test]
    fn sorted_json_nested_array_signature_roundtrip() {
        // Verify that signing a payload with nested arrays in objects produces
        // a valid signature (canonical form is deterministic)
        let identity = AgentIdentity::generate();
        let payload = json!({
            "filters": [
                {"z_key": "last", "a_key": "first"},
                {"nested": [{"b": 2, "a": 1}]}
            ]
        });
        let envelope = identity.sign("test_skill", payload).unwrap();
        assert!(
            verify_envelope(&envelope).is_ok(),
            "payload with nested arrays containing objects must round-trip"
        );
    }

    #[test]
    fn replay_guard_hard_cap_prevents_unbounded_growth() {
        let guard = ReplayGuard::new();

        // Fill beyond MAX_REPLAY_ENTRIES
        let overflow = MAX_REPLAY_ENTRIES + 100;
        for i in 0..overflow {
            assert!(
                guard.check_and_record(&format!("sig_{i}")),
                "unique signature {i} must be accepted"
            );
        }

        // After insertion, the guard should have evicted oldest entries
        // to stay near the cap. Verify by checking that a NEW signature
        // is still accepted (guard is functional).
        assert!(
            guard.check_and_record("brand_new_sig"),
            "new signature must still be accepted after cap enforcement"
        );

        // The very first entries should have been evicted (oldest 20%)
        // so re-submitting them should succeed (they're gone from the map).
        assert!(
            guard.check_and_record("sig_0"),
            "sig_0 should have been evicted by hard cap and thus re-acceptable"
        );

        // Recent entries should still be present (not evicted)
        assert!(
            !guard.check_and_record(&format!("sig_{}", overflow - 1)),
            "recent signature should still be tracked (replay blocked)"
        );
    }

    // ── DID pinning tests (Task #27) ─────────────────────────────────

    #[test]
    fn did_pinning_accepts_matching_did() {
        let identity = AgentIdentity::generate();
        let envelope = identity.sign("web_reader", json!({})).unwrap();

        // When expected_did matches the envelope's DID, verification succeeds
        let result = verify_envelope_with_guard(&envelope, None, Some(&identity.did));
        assert!(
            result.is_ok(),
            "envelope with matching DID must be accepted: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn did_pinning_rejects_foreign_did() {
        let identity_a = AgentIdentity::generate();
        let identity_b = AgentIdentity::generate();
        let envelope = identity_a.sign("web_reader", json!({})).unwrap();

        // Verify against identity_b's DID — must reject even though signature is
        // valid for identity_a (this is the self-signed forgery scenario)
        let result = verify_envelope_with_guard(&envelope, None, Some(&identity_b.did));
        assert!(
            result.is_err(),
            "envelope from foreign DID must be rejected"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("DID mismatch"),
            "error should mention DID mismatch: {err_msg}"
        );
    }

    #[test]
    fn did_pinning_none_allows_any_did() {
        let identity = AgentIdentity::generate();
        let envelope = identity.sign("web_reader", json!({})).unwrap();

        // When expected_did is None, any valid signature is accepted (backward compat)
        assert!(
            verify_envelope_with_guard(&envelope, None, None).is_ok(),
            "None expected_did must accept any valid envelope"
        );
    }
}
