//! Encrypted credential store for Aegis Runtime.
//!
//! Stores service credentials (OAuth secrets, API tokens, etc.) encrypted with
//! AES-256-GCM. The encryption key is deterministically derived from the agent's
//! Ed25519 private key via HKDF-SHA256, so credentials are bound to the agent
//! identity and automatically unlocked on startup.
//!
//! SQLite table: `credentials`
//!   - service TEXT PRIMARY KEY (e.g. "github", "google_calendar", "xai_subscription")
//!   - key TEXT NOT NULL (e.g. "password", "token")
//!   - encrypted_value BLOB NOT NULL (nonce ‖ ciphertext ‖ tag)
//!   - created_at TEXT NOT NULL
//!   - updated_at TEXT NOT NULL

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use hkdf::Hkdf;
use sha2::Sha256;

/// Derives a 256-bit AES key from an Ed25519 private key using HKDF-SHA256.
///
/// The `salt` is fixed to "aegis-runtime-credential-store-v1" so the derived key
/// is deterministic and reproducible across restarts.
pub fn derive_key_from_ed25519(signing_key_bytes: &[u8; 32]) -> [u8; 32] {
    let salt = b"aegis-runtime-credential-store-v1";
    let info = b"aes-256-gcm-credential-encryption";

    let hkdf = Hkdf::<Sha256>::new(Some(salt), signing_key_bytes);
    let mut okm = [0u8; 32];
    hkdf.expand(info, &mut okm)
        .expect("HKDF expand should never fail for 32-byte output");
    okm
}

/// Encrypts a plaintext value using AES-256-GCM.
/// Returns `nonce (12 bytes) || ciphertext || tag (16 bytes)`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("AES key init failed: {e}"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("AES-GCM encrypt failed: {e}"))?;

    // nonce (12) || ciphertext+tag
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypts a value produced by `encrypt()`.
/// Input format: `nonce (12 bytes) || ciphertext || tag (16 bytes)`.
pub fn decrypt(key: &[u8; 32], encrypted: &[u8]) -> Result<Vec<u8>> {
    if encrypted.len() < 12 + 16 {
        anyhow::bail!(
            "encrypted data too short: {} bytes (minimum 28)",
            encrypted.len()
        );
    }

    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("AES key init failed: {e}"))?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("AES-GCM decryption failed: wrong key or tampered data"))?;
    Ok(plaintext)
}

/// High-level credential store backed by SQLite.
pub struct CredentialStore {
    conn: rusqlite::Connection,
    key: [u8; 32],
}

impl CredentialStore {
    /// Open (or create) the credential store, using the given SQLite connection
    /// and the AES key derived from the agent identity.
    pub fn open(conn: rusqlite::Connection, aes_key: [u8; 32]) -> Result<Self> {
        // Match WAL + busy_timeout settings from the main memory actor
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;",
        )
        .context("setting credential store SQLite pragmas")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS credentials (
                service TEXT NOT NULL,
                key TEXT NOT NULL,
                encrypted_value BLOB NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (service, key)
            );",
        )
        .context("creating credentials table")?;

        Ok(Self { conn, key: aes_key })
    }

    /// Run `f` inside a single SQLite transaction: every CredentialStore
    /// call made through the borrow passed to `f` commits or rolls back
    /// atomically. Multi-row account updates (credential fields + the
    /// `mail_accounts` metadata row) use this so a partial failure cannot
    /// leave orphan credentials or a metadata row pointing at deleted
    /// credentials (Codex round-5, block1 1b).
    ///
    /// Not re-entrant: nesting `with_transaction` fails at BEGIN.
    pub fn with_transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .context("opening credential-store transaction")?;
        match f(self) {
            Ok(v) => {
                if let Err(commit_err) = self.conn.execute_batch("COMMIT") {
                    // SQLite can leave the transaction open after some
                    // commit failures — roll back so the connection is not
                    // poisoned (writes silently pending / future BEGINs
                    // failing) for follow-up retry logic.
                    if let Err(rb) = self.conn.execute_batch("ROLLBACK") {
                        tracing::warn!(
                            error = %rb,
                            "rollback after failed credential-store commit also failed"
                        );
                    }
                    return Err(anyhow::Error::from(commit_err)
                        .context("committing credential-store transaction"));
                }
                Ok(v)
            }
            Err(e) => {
                if let Err(rb) = self.conn.execute_batch("ROLLBACK") {
                    tracing::warn!(
                        error = %rb,
                        "credential-store transaction rollback failed"
                    );
                }
                Err(e)
            }
        }
    }

    /// Store a credential (encrypts the value before writing).
    pub fn store(&self, service: &str, cred_key: &str, plaintext: &str) -> Result<()> {
        let encrypted = encrypt(&self.key, plaintext.as_bytes())?;
        let now = chrono::Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO credentials (service, key, encrypted_value, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(service, key) DO UPDATE SET
                    encrypted_value = excluded.encrypted_value,
                    updated_at = excluded.updated_at",
                rusqlite::params![service, cred_key, encrypted, now],
            )
            .context("storing credential")?;
        Ok(())
    }

    /// Retrieve and decrypt a credential. Returns None if not found.
    pub fn get(&self, service: &str, cred_key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT encrypted_value FROM credentials WHERE service = ?1 AND key = ?2")?;

        let encrypted: Option<Vec<u8>> =
            match stmt.query_row(rusqlite::params![service, cred_key], |row| row.get(0)) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            };

        match encrypted {
            None => Ok(None),
            Some(blob) => {
                let plaintext = decrypt(&self.key, &blob)?;
                let text = String::from_utf8(plaintext).context("credential is not valid UTF-8")?;
                Ok(Some(text))
            }
        }
    }

    /// Delete a credential. Returns true if it existed.
    pub fn delete(&self, service: &str, cred_key: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM credentials WHERE service = ?1 AND key = ?2",
                rusqlite::params![service, cred_key],
            )
            .context("deleting credential")?;
        Ok(affected > 0)
    }

    /// List all stored services (without decrypting values).
    pub fn list_services(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT service, key FROM credentials ORDER BY service, key")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("listing credentials")?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        // Deterministic test key derived from a fake Ed25519 private key
        derive_key_from_ed25519(&[42u8; 32])
    }

    // ── HKDF key derivation ──

    #[test]
    fn derive_key_is_deterministic() {
        let key1 = derive_key_from_ed25519(&[1u8; 32]);
        let key2 = derive_key_from_ed25519(&[1u8; 32]);
        assert_eq!(key1, key2, "same input must produce same key");
    }

    #[test]
    fn derive_key_differs_for_different_input() {
        let key1 = derive_key_from_ed25519(&[1u8; 32]);
        let key2 = derive_key_from_ed25519(&[2u8; 32]);
        assert_ne!(key1, key2, "different inputs must produce different keys");
    }

    #[test]
    fn derive_key_is_not_trivial() {
        let key = derive_key_from_ed25519(&[0u8; 32]);
        assert_ne!(key, [0u8; 32], "derived key must not be all zeros");
    }

    // ── AES-256-GCM encrypt/decrypt ──

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"my-secret-password-123";
        let encrypted = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() {
        let key = test_key();
        let plaintext = b"same-input";
        let enc1 = encrypt(&key, plaintext).unwrap();
        let enc2 = encrypt(&key, plaintext).unwrap();
        // Different random nonces → different ciphertexts
        assert_ne!(enc1, enc2, "AES-GCM must use random nonces");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key1 = derive_key_from_ed25519(&[1u8; 32]);
        let key2 = derive_key_from_ed25519(&[2u8; 32]);
        let encrypted = encrypt(&key1, b"secret").unwrap();
        let result = decrypt(&key2, &encrypted);
        assert!(result.is_err(), "wrong key must fail decryption");
        assert!(
            result.unwrap_err().to_string().contains("tampered"),
            "error should mention tampering"
        );
    }

    #[test]
    fn decrypt_tampered_ciphertext_fails() {
        let key = test_key();
        let mut encrypted = encrypt(&key, b"secret").unwrap();
        // Flip a byte in the ciphertext portion (after the 12-byte nonce)
        encrypted[15] ^= 0xFF;
        let result = decrypt(&key, &encrypted);
        assert!(result.is_err(), "tampered ciphertext must fail");
    }

    #[test]
    fn decrypt_too_short_fails() {
        let key = test_key();
        let result = decrypt(&key, &[0u8; 10]);
        assert!(result.is_err(), "data shorter than nonce+tag must fail");
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn encrypt_empty_plaintext() {
        let key = test_key();
        let encrypted = encrypt(&key, b"").unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert!(decrypted.is_empty(), "empty plaintext roundtrip must work");
    }

    // ── CredentialStore (SQLite integration) ──

    fn in_memory_store() -> CredentialStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        CredentialStore::open(conn, test_key()).unwrap()
    }

    #[test]
    fn store_and_retrieve_credential() {
        let store = in_memory_store();
        store.store("oauth", "password", "hunter2").unwrap();
        let retrieved = store.get("oauth", "password").unwrap();
        assert_eq!(retrieved.as_deref(), Some("hunter2"));
    }

    /// `with_transaction` atomicity: writes made before a failure roll back
    /// completely (multi-row account updates — credentials + metadata —
    /// rely on this; Codex round-5, block1 1b).
    #[test]
    fn with_transaction_rolls_back_all_writes_on_error() {
        let store = in_memory_store();
        store.store("svc", "pre-existing", "keep-me").unwrap();

        let err = store
            .with_transaction(|store| -> Result<()> {
                store.store("svc", "a", "written-then-rolled-back")?;
                store.store("svc", "b", "also-rolled-back")?;
                anyhow::bail!("simulated mid-update failure")
            })
            .expect_err("closure error must propagate");
        assert!(err.to_string().contains("simulated mid-update failure"));

        assert_eq!(store.get("svc", "a").unwrap(), None, "write a rolled back");
        assert_eq!(store.get("svc", "b").unwrap(), None, "write b rolled back");
        assert_eq!(
            store.get("svc", "pre-existing").unwrap().as_deref(),
            Some("keep-me"),
            "pre-transaction data untouched"
        );
        // The store remains usable after a rollback (no stuck transaction).
        store.store("svc", "after", "works").unwrap();
        assert_eq!(store.get("svc", "after").unwrap().as_deref(), Some("works"));
    }

    /// `with_transaction` success path commits everything.
    #[test]
    fn with_transaction_commits_on_success() {
        let store = in_memory_store();
        store
            .with_transaction(|store| {
                store.store("svc", "a", "v1")?;
                store.store("svc", "b", "v2")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.get("svc", "a").unwrap().as_deref(), Some("v1"));
        assert_eq!(store.get("svc", "b").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let store = in_memory_store();
        let result = store.get("github", "token").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn store_overwrites_existing() {
        let store = in_memory_store();
        store.store("oauth", "password", "old-password").unwrap();
        store.store("oauth", "password", "new-password").unwrap();
        let retrieved = store.get("oauth", "password").unwrap();
        assert_eq!(retrieved.as_deref(), Some("new-password"));
    }

    #[test]
    fn delete_existing_credential() {
        let store = in_memory_store();
        store.store("github", "token", "ghp_xxx").unwrap();
        let deleted = store.delete("github", "token").unwrap();
        assert!(deleted, "delete should return true for existing credential");
        let result = store.get("github", "token").unwrap();
        assert_eq!(result, None, "deleted credential should be gone");
    }

    #[test]
    fn delete_nonexistent_returns_false() {
        let store = in_memory_store();
        let deleted = store.delete("nonexistent", "key").unwrap();
        assert!(
            !deleted,
            "delete should return false for missing credential"
        );
    }

    #[test]
    fn list_services_shows_all() {
        let store = in_memory_store();
        store.store("oauth", "password", "pw1").unwrap();
        store.store("oauth", "host", "calendar.google.com").unwrap();
        store.store("github", "token", "ghp_xxx").unwrap();
        store.store("google_calendar", "token", "ya29.xxx").unwrap();

        let services = store.list_services().unwrap();
        assert_eq!(services.len(), 4);
        assert!(services.contains(&("github".to_string(), "token".to_string())));
        assert!(services.contains(&("oauth".to_string(), "password".to_string())));
        assert!(services.contains(&("oauth".to_string(), "host".to_string())));
    }

    #[test]
    fn multiple_keys_per_service() {
        let store = in_memory_store();
        store.store("oauth", "host", "calendar.google.com").unwrap();
        store.store("oauth", "port", "993").unwrap();
        store.store("oauth", "user", "me@example.com").unwrap();
        store.store("oauth", "password", "secret").unwrap();

        assert_eq!(
            store.get("oauth", "host").unwrap().as_deref(),
            Some("calendar.google.com")
        );
        assert_eq!(store.get("oauth", "port").unwrap().as_deref(), Some("993"));
        assert_eq!(
            store.get("oauth", "user").unwrap().as_deref(),
            Some("me@example.com")
        );
        assert_eq!(
            store.get("oauth", "password").unwrap().as_deref(),
            Some("secret")
        );
    }

    #[test]
    fn credential_data_at_rest_is_encrypted() {
        let store = in_memory_store();
        let secret = "super-secret-api-key-12345";
        store.store("github", "token", secret).unwrap();

        // Read raw encrypted blob from SQLite
        let mut stmt = store
            .conn
            .prepare("SELECT encrypted_value FROM credentials WHERE service = 'github' AND key = 'token'")
            .unwrap();
        let blob: Vec<u8> = stmt.query_row([], |row| row.get(0)).unwrap();

        // The raw blob must NOT contain the plaintext
        let blob_str = String::from_utf8_lossy(&blob);
        assert!(
            !blob_str.contains(secret),
            "plaintext must not appear in encrypted blob"
        );

        // But must be longer than just nonce (contains ciphertext+tag)
        assert!(
            blob.len() >= 12 + secret.len() + 16,
            "encrypted blob must contain nonce + ciphertext + tag"
        );
    }

    #[test]
    fn unicode_credential_roundtrip() {
        let store = in_memory_store();
        let unicode_password = "密码🔑パスワード";
        store.store("service", "pass", unicode_password).unwrap();
        let retrieved = store.get("service", "pass").unwrap();
        assert_eq!(retrieved.as_deref(), Some(unicode_password));
    }
}
