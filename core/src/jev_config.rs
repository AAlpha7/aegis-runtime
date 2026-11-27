//! Owner-controlled Jev configuration, stored atomically in the encrypted vault.
use crate::credential_store::CredentialStore;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedJev {
    pub enabled: bool,
    pub api_key: Option<String>,
    #[serde(default)]
    pub verification: Option<JevVerification>,
}

#[derive(Serialize, Deserialize)]
pub struct JevVerification {
    fingerprint: String,
    model: String,
    checked_at: i64,
}

fn fingerprint(key: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(key.as_bytes()))
}

pub struct ResolvedJev {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub saved_key: bool,
    pub saved_choice: bool,
    pub verified: bool,
    pub checked_at: Option<i64>,
}

impl ResolvedJev {
    pub fn active_key(self) -> Option<String> {
        self.api_key.filter(|_| self.enabled)
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "key_present": self.api_key.is_some(),
            "saved_key": self.saved_key,
            "source": if self.saved_choice { "settings" } else { "environment" },
            "key_source": if self.saved_key { "settings" } else if self.api_key.is_some() { "environment" } else { "none" },
            "verified": self.verified,
            "checked_at": self.checked_at,
        })
    }
}

pub fn load_saved(store: &CredentialStore) -> anyhow::Result<Option<SavedJev>> {
    store
        .get("jev", "configuration")?
        .map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}

pub fn resolve(store: Option<&CredentialStore>) -> anyhow::Result<ResolvedJev> {
    let saved = store.map(load_saved).transpose()?.flatten();
    let saved_choice = saved.is_some();
    let enabled = saved
        .as_ref()
        .map(|s| s.enabled)
        .unwrap_or_else(crate::computer_use::jev_enabled);
    let saved_key = saved.as_ref().is_some_and(|s| s.api_key.is_some());
    let api_key = saved
        .as_ref()
        .and_then(|s| s.api_key.clone())
        .or_else(crate::computer_use::configured_jev_api_key);
    let checked_at = saved
        .as_ref()
        .and_then(|s| s.verification.as_ref())
        .filter(|proof| {
            api_key
                .as_ref()
                .is_some_and(|key| proof.fingerprint == fingerprint(key))
                && proof.model == crate::computer_use::JevModel::from_env().as_str()
        })
        .map(|proof| proof.checked_at);
    Ok(ResolvedJev {
        enabled,
        api_key,
        saved_key,
        saved_choice,
        verified: checked_at.is_some(),
        checked_at,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JevUpdate {
    pub enabled: bool,
    #[serde(default)]
    pub consent: bool,
    pub api_key: Option<String>,
    #[serde(default)]
    pub remove_key: bool,
}

#[cfg(test)]
pub fn save(store: &CredentialStore, update: JevUpdate) -> anyhow::Result<()> {
    save_inner(store, update, None)
}

pub fn prepare_key(store: &CredentialStore, update: &JevUpdate) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(
        !(update.enabled || update.api_key.is_some()) || update.consent,
        "jev_consent_required"
    );
    anyhow::ensure!(
        !update.remove_key || (!update.enabled && update.api_key.is_none()),
        "jev_remove_requires_off"
    );
    if let Some(key) = &update.api_key {
        anyhow::ensure!(valid_key(key.trim()), "jev_invalid_key");
    }
    if update.enabled || update.api_key.is_some() {
        let key = update
            .api_key
            .as_ref()
            .map(|key| key.trim().to_owned())
            .or(resolve(Some(store))?.api_key);
        anyhow::ensure!(key.is_some(), "jev_key_required");
        Ok(key)
    } else {
        Ok(None)
    }
}

pub fn save_after_probe(
    store: &CredentialStore,
    update: JevUpdate,
    probed_key: Option<&str>,
) -> anyhow::Result<()> {
    let candidate = prepare_key(store, &update)?;
    anyhow::ensure!(
        candidate.as_deref() == probed_key,
        "jev_configuration_changed"
    );
    save_inner(store, update, probed_key)
}

fn save_inner(
    store: &CredentialStore,
    update: JevUpdate,
    probed_key: Option<&str>,
) -> anyhow::Result<()> {
    prepare_key(store, &update)?;
    store.with_transaction(|store| {
        let mut saved = load_saved(store)?.unwrap_or_default();
        saved.enabled = update.enabled;
        if update.remove_key {
            saved.api_key = None;
            saved.verification = None;
        }
        if let Some(key) = update.api_key {
            saved.api_key = Some(key.trim().to_owned());
            saved.verification = None;
        }
        if let Some(key) = probed_key {
            saved.verification = Some(JevVerification {
                fingerprint: fingerprint(key),
                model: crate::computer_use::JevModel::from_env().as_str().into(),
                checked_at: chrono::Utc::now().timestamp(),
            });
        }
        anyhow::ensure!(
            !saved.enabled
                || saved.api_key.is_some()
                || crate::computer_use::configured_jev_api_key().is_some(),
            "jev_key_required"
        );
        store.store("jev", "configuration", &serde_json::to_string(&saved)?)
    })
}

fn valid_key(key: &str) -> bool {
    key.starts_with("sk-or-")
        && key.len() > 20
        && key.len() <= 4096
        && !key.contains("xxxxxxxx")
        && !key.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jev_proof_is_bound_to_key_and_old_records_remain_unverified() {
        let store =
            CredentialStore::open(rusqlite::Connection::open_in_memory().unwrap(), [42; 32])
                .unwrap();
        let key = "sk-or-synthetic-verified-test-key";
        let update = || JevUpdate {
            enabled: true,
            consent: true,
            api_key: Some(key.into()),
            remove_key: false,
        };
        assert!(save_after_probe(&store, update(), Some("different-key")).is_err());
        assert!(load_saved(&store).unwrap().is_none());
        save(&store, update()).unwrap();
        assert!(!resolve(Some(&store)).unwrap().verified);
        save_after_probe(&store, update(), Some(key)).unwrap();
        let resolved = resolve(Some(&store)).unwrap();
        assert!(resolved.verified);
        assert!(resolved.checked_at.unwrap() > 0);
        let mut saved = load_saved(&store).unwrap().unwrap();
        saved.api_key = Some("sk-or-synthetic-changed-test-key".into());
        store
            .store(
                "jev",
                "configuration",
                &serde_json::to_string(&saved).unwrap(),
            )
            .unwrap();
        assert!(!resolve(Some(&store)).unwrap().verified);
    }
    #[test]
    fn jev_settings_consent_persistence_disable_and_remove() {
        let store =
            CredentialStore::open(rusqlite::Connection::open_in_memory().unwrap(), [42; 32])
                .unwrap();
        let key = "sk-or-synthetic-settings-test-key";
        assert!(
            save(
                &store,
                JevUpdate {
                    enabled: true,
                    consent: false,
                    api_key: Some(key.into()),
                    remove_key: false
                }
            )
            .is_err()
        );
        assert!(load_saved(&store).unwrap().is_none());
        save(
            &store,
            JevUpdate {
                enabled: true,
                consent: true,
                api_key: Some(key.into()),
                remove_key: false,
            },
        )
        .unwrap();
        assert_eq!(
            resolve(Some(&store)).unwrap().active_key().as_deref(),
            Some(key)
        );
        assert!(
            !resolve(Some(&store))
                .unwrap()
                .status()
                .to_string()
                .contains(key)
        );
        save(
            &store,
            JevUpdate {
                enabled: false,
                consent: false,
                api_key: None,
                remove_key: false,
            },
        )
        .unwrap();
        assert!(resolve(Some(&store)).unwrap().active_key().is_none());
        assert!(load_saved(&store).unwrap().unwrap().api_key.is_some());
        save(
            &store,
            JevUpdate {
                enabled: false,
                consent: false,
                api_key: None,
                remove_key: true,
            },
        )
        .unwrap();
        assert!(load_saved(&store).unwrap().unwrap().api_key.is_none());
    }
}
