use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::memory::MemoryHandle;

const OWNER_WEB_PRESENCE_SKILL: &str = "owner_web_presence";
pub const OWNER_WEB_PRESENCE_TTL_SECS: i64 = 180;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OwnerWebPresence {
    schema_version: u32,
    owner_did: String,
    surface: String,
    last_seen_unix: i64,
}

pub async fn mark_owner_web_presence(
    memory: &MemoryHandle,
    owner_did: &str,
    surface: &str,
) -> Result<()> {
    let presence = OwnerWebPresence {
        schema_version: 1,
        owner_did: owner_did.to_string(),
        surface: surface.chars().take(64).collect(),
        last_seen_unix: chrono::Utc::now().timestamp(),
    };
    memory
        .store_skill_state(
            OWNER_WEB_PRESENCE_SKILL,
            owner_did,
            &serde_json::to_string(&presence)?,
        )
        .await
}

pub fn canonical_owner_did_for_chat(chat_id: i64) -> String {
    std::env::var("OWNER_DID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("ADMIN_CHAT_ID")
                .ok()
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|admin_chat_id| *admin_chat_id != 0)
                .map(|admin_chat_id| format!("did:aegis:telegram:{admin_chat_id}"))
        })
        .unwrap_or_else(|| format!("did:aegis:telegram:{chat_id}"))
}

pub async fn owner_web_presence_is_active(memory: &MemoryHandle, owner_did: &str) -> bool {
    let now_unix = chrono::Utc::now().timestamp();
    let Ok(Some(raw)) = memory
        .load_skill_state(OWNER_WEB_PRESENCE_SKILL, owner_did)
        .await
    else {
        return false;
    };
    owner_web_presence_payload_is_active(&raw, now_unix)
}

pub(crate) fn owner_web_presence_payload_is_active(raw: &str, now_unix: i64) -> bool {
    let Ok(presence) = serde_json::from_str::<OwnerWebPresence>(raw) else {
        return false;
    };
    if presence.schema_version != 1 || presence.owner_did.trim().is_empty() {
        return false;
    }
    let age = now_unix.saturating_sub(presence.last_seen_unix);
    (0..=OWNER_WEB_PRESENCE_TTL_SECS).contains(&age)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_web_presence_payload_expires() {
        let raw = serde_json::json!({
            "schema_version": 1,
            "owner_did": "did:aegis:telegram:123",
            "surface": "web",
            "last_seen_unix": 1_000,
        })
        .to_string();

        assert!(owner_web_presence_payload_is_active(&raw, 1_120));
        assert!(!owner_web_presence_payload_is_active(&raw, 1_181));
    }

    #[test]
    fn owner_web_presence_payload_fails_closed_on_malformed_state() {
        assert!(!owner_web_presence_payload_is_active("not-json", 1_000));
        assert!(!owner_web_presence_payload_is_active(
            r#"{"schema_version":2,"owner_did":"did:aegis:telegram:123","last_seen_unix":1000}"#,
            1_000,
        ));
    }
}
