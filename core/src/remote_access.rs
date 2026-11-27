//! Single-owner Passkey login. Challenges and pairing tokens never leave server storage
//! except for the browser challenge and a short-lived, host-issued pairing secret.
//! The WebAuthn state serialization feature is used ONLY for server-side SQLCipher rows.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::*;

const TTL: u64 = 300;
const MAX_PENDING: u64 = 64;

pub fn validate_deployment(
    mode: &str,
    origin: Option<&str>,
    bind: &str,
    accepted: &str,
) -> Result<()> {
    match mode {
        "" => return Ok(()), // Existing manually configured deployments remain compatible.
        "local" => {
            if origin.is_some_and(|s| !s.is_empty()) {
                bail!("Local mode cannot have AEGIS_PUBLIC_URL");
            }
        }
        "private" | "public" => {
            let url = origin_url(origin.context("Remote access requires AEGIS_PUBLIC_URL")?)?;
            if url.scheme() != "https" {
                bail!("Remote access requires HTTPS");
            }
            if mode == "public" && accepted != "2026-09-27" {
                bail!(
                    "Public access requires explicit risk acknowledgment through scripts/remote-access.py"
                );
            }
        }
        _ => bail!("AEGIS_ACCESS_MODE must be local, private or public"),
    }
    if bind != "127.0.0.1" && bind != "::1" {
        bail!("Managed access modes require a loopback console listener");
    }
    Ok(())
}

pub fn origin_url(origin: &str) -> Result<Url> {
    let url = Url::parse(origin)?;
    let local = url.scheme() == "http" && url.host_str() == Some("localhost");
    if (!local && url.scheme() != "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Passkey origin must be an HTTPS origin (http://localhost is for local testing)");
    }
    Ok(url)
}

fn webauthn(origin: &str) -> Result<Webauthn> {
    let url = origin_url(origin)?;
    Ok(
        WebauthnBuilder::new(url.host_str().context("missing host")?, &url)?
            .rp_name("Aegis Runtime")
            .build()?,
    )
}

pub fn init(conn: &Connection) -> Result<()> {
    crate::web_console_auth::init_session_schema(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS remote_owner (id INTEGER PRIMARY KEY CHECK(id=1), uuid TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS remote_passkeys (id TEXT PRIMARY KEY, origin TEXT NOT NULL, data TEXT NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS remote_pairing (hash TEXT PRIMARY KEY, origin TEXT NOT NULL, expires INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS remote_challenges (id TEXT PRIMARY KEY, origin TEXT NOT NULL, purpose TEXT NOT NULL, data TEXT NOT NULL, expires INTEGER NOT NULL);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO remote_owner VALUES (1, ?1)",
        [Uuid::new_v4().to_string()],
    )?;
    Ok(())
}

fn secret() -> Result<String> {
    crate::web_console_auth::generate_local_login_token()
}

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Only called by the local administrative CLI, never by an unauthenticated route.
pub fn pair(conn: &Connection, origin: &str, now: u64) -> Result<String> {
    origin_url(origin)?;
    init(conn)?;
    let token = secret()?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM remote_pairing", [])?;
    tx.execute("DELETE FROM remote_challenges WHERE purpose='register'", [])?;
    tx.execute(
        "INSERT INTO remote_pairing VALUES (?1, ?2, ?3)",
        params![hash(&token), origin, (now + 600) as i64],
    )?;
    tx.commit()?;
    Ok(token)
}

pub fn recover(conn: &Connection, now: u64) -> Result<()> {
    init(conn)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM remote_passkeys", [])?;
    tx.execute("DELETE FROM remote_pairing", [])?;
    tx.execute("DELETE FROM remote_challenges", [])?;
    revoke_sessions(&tx, now)?;
    tx.commit()?;
    Ok(())
}

pub fn revoke_sessions(conn: &Connection, now: u64) -> Result<()> {
    conn.execute(
        "UPDATE web_console_sessions SET revoked_at=?1 WHERE google_sub=?2 AND revoked_at IS NULL",
        params![now as i64, crate::web_console_auth::LOCAL_LOGIN_SUB],
    )?;
    Ok(())
}

fn credentials(conn: &Connection, origin: &str) -> Result<Vec<Passkey>> {
    let mut stmt = conn.prepare("SELECT data FROM remote_passkeys WHERE origin=?1 ORDER BY id")?;
    let rows = stmt.query_map([origin], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}

pub fn count(conn: &Connection, origin: &str) -> Result<u64> {
    init(conn)?;
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM remote_passkeys WHERE origin=?1",
        [origin],
        |r| r.get::<_, i64>(0),
    )? as u64)
}

fn save_challenge(
    conn: &Connection,
    origin: &str,
    purpose: &str,
    data: String,
    now: u64,
) -> Result<String> {
    conn.execute(
        "DELETE FROM remote_challenges WHERE expires<=?1",
        [now as i64],
    )?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM remote_challenges", [], |r| r.get(0))?;
    if n >= MAX_PENDING as i64 {
        bail!("too many pending authentication attempts; retry shortly");
    }
    let id = secret()?;
    conn.execute(
        "INSERT INTO remote_challenges VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, origin, purpose, data, (now + TTL) as i64],
    )?;
    Ok(id)
}

fn take_challenge(
    conn: &Connection,
    origin: &str,
    purpose: &str,
    id: &str,
    now: u64,
) -> Result<String> {
    // DELETE RETURNING is atomic across connections. Invalid attempts consume the
    // challenge too; a failed signature cannot later replay the same challenge.
    conn.query_row("DELETE FROM remote_challenges WHERE id=?1 AND origin=?2 AND purpose=?3 AND expires>?4 RETURNING data", params![id, origin, purpose, now as i64], |r| r.get(0))
        .optional()?.context("invalid or expired challenge")
}

pub fn register_start(conn: &Connection, origin: &str, token: &str, now: u64) -> Result<Value> {
    init(conn)?;
    if token.len() != 64 {
        bail!("invalid pairing code");
    }
    let w = webauthn(origin)?;
    let tx = conn.unchecked_transaction()?;
    if count(&tx, origin)? >= 16 {
        bail!("credential limit reached; recover from the host to reset");
    }
    let changed = tx.execute(
        "DELETE FROM remote_pairing WHERE hash=?1 AND origin=?2 AND expires>?3",
        params![hash(token), origin, now as i64],
    )?;
    if changed != 1 {
        bail!("invalid or expired pairing code");
    }
    let uuid: String =
        tx.query_row("SELECT uuid FROM remote_owner WHERE id=1", [], |r| r.get(0))?;
    let exclude = credentials(&tx, origin)?
        .iter()
        .map(|p| p.cred_id().clone())
        .collect();
    let (options, state) = w.start_passkey_registration(
        Uuid::parse_str(&uuid)?,
        "owner",
        "Aegis owner",
        Some(exclude),
    )?;
    let id = save_challenge(&tx, origin, "register", serde_json::to_string(&state)?, now)?;
    tx.commit()?;
    Ok(json!({"challenge_id": id, "options": options}))
}

pub fn register_finish(
    conn: &Connection,
    origin: &str,
    id: &str,
    response: Value,
    now: u64,
) -> Result<()> {
    init(conn)?;
    let tx = conn.unchecked_transaction()?;
    // Hold the write lock through verification and insertion so host recovery or
    // pairing rotation cannot be followed by an in-flight credential resurrection.
    let state = take_challenge(&tx, origin, "register", id, now)?;
    let result = (|| -> Result<()> {
        let state: PasskeyRegistration = serde_json::from_str(&state)?;
        let response: RegisterPublicKeyCredential = serde_json::from_value(response)?;
        let credential = webauthn(origin)?.finish_passkey_registration(&response, &state)?;
        if count(&tx, origin)? >= 16 {
            bail!("credential limit reached; recover from the host to reset");
        }
        let id = serde_json::to_string(credential.cred_id())?;
        tx.execute(
            "INSERT INTO remote_passkeys VALUES (?1, ?2, ?3, ?4)",
            params![id, origin, serde_json::to_string(&credential)?, now as i64],
        )?;
        Ok(())
    })();
    // Even invalid responses burn their challenge.
    tx.commit()?;
    result
}

pub fn login_start(conn: &Connection, origin: &str, now: u64) -> Result<Value> {
    init(conn)?;
    let w = webauthn(origin)?;
    let creds = credentials(conn, origin)?;
    if creds.is_empty() {
        bail!("pair a Passkey from the host first");
    }
    let (options, state) = w.start_passkey_authentication(&creds)?;
    let tx = conn.unchecked_transaction()?;
    let id = save_challenge(&tx, origin, "login", serde_json::to_string(&state)?, now)?;
    tx.commit()?;
    Ok(json!({"challenge_id": id, "options": options}))
}

pub fn login_finish(
    conn: &Connection,
    origin: &str,
    id: &str,
    response: Value,
    now: u64,
) -> Result<crate::web_console_auth::CreatedSession> {
    init(conn)?;
    let state: PasskeyAuthentication =
        serde_json::from_str(&take_challenge(conn, origin, "login", id, now)?)?;
    let response: PublicKeyCredential = serde_json::from_value(response)?;
    let result = webauthn(origin)?.finish_passkey_authentication(&response, &state)?;
    if !result.user_verified() {
        bail!("user verification required");
    }
    let tx = conn.unchecked_transaction()?;
    let mut matched = false;
    for mut passkey in credentials(&tx, origin)? {
        if passkey.update_credential(&result).is_some() {
            tx.execute(
                "UPDATE remote_passkeys SET data=?1 WHERE id=?2",
                params![
                    serde_json::to_string(&passkey)?,
                    serde_json::to_string(passkey.cred_id())?
                ],
            )?;
            matched = true;
        }
    }
    if !matched {
        bail!("credential was revoked");
    }
    let session = crate::web_console_auth::create_passkey_session(&tx, now)?;
    tx.commit()?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ORIGIN: &str = "https://device.example.ts.net";
    #[test]
    fn managed_modes_require_consent_https_and_loopback() {
        assert!(validate_deployment("public", Some(ORIGIN), "127.0.0.1", "").is_err());
        assert!(validate_deployment("public", Some(ORIGIN), "127.0.0.1", "2026-09-27").is_ok());
        assert!(validate_deployment("private", Some(ORIGIN), "127.0.0.1", "").is_ok());
        assert!(validate_deployment("private", Some(ORIGIN), "0.0.0.0", "").is_err());
        assert!(
            validate_deployment("private", Some("http://localhost:8765"), "127.0.0.1", "").is_err()
        );
        assert!(validate_deployment("local", Some(ORIGIN), "127.0.0.1", "").is_err());
        assert!(validate_deployment("local", None, "127.0.0.1", "").is_ok());
        assert!(validate_deployment("typo", None, "127.0.0.1", "").is_err());
    }
    #[test]
    fn pairing_requires_host_secret_and_is_single_use_expiring_and_origin_bound() {
        let conn = Connection::open_in_memory().unwrap();
        let code = pair(&conn, ORIGIN, 100).unwrap();
        assert!(register_start(&conn, ORIGIN, &"0".repeat(64), 101).is_err());
        assert!(register_start(&conn, "https://other.example.ts.net", &code, 101).is_err());
        let challenge = register_start(&conn, ORIGIN, &code, 101).unwrap();
        assert!(register_start(&conn, ORIGIN, &code, 101).is_err());
        assert!(
            register_finish(
                &conn,
                ORIGIN,
                challenge["challenge_id"].as_str().unwrap(),
                json!({}),
                102
            )
            .is_err()
        );
        assert_eq!(count(&conn, ORIGIN).unwrap(), 0);
        assert!(
            take_challenge(
                &conn,
                ORIGIN,
                "register",
                challenge["challenge_id"].as_str().unwrap(),
                103
            )
            .is_err()
        );
        let code = pair(&conn, ORIGIN, 200).unwrap();
        assert!(register_start(&conn, ORIGIN, &code, 800).is_err());
    }
    #[test]
    fn pairing_rotation_invalidates_pending_registration() {
        let conn = Connection::open_in_memory().unwrap();
        let code = pair(&conn, ORIGIN, 100).unwrap();
        let pending = register_start(&conn, ORIGIN, &code, 101).unwrap();
        let replacement = pair(&conn, ORIGIN, 102).unwrap();
        assert!(
            take_challenge(
                &conn,
                ORIGIN,
                "register",
                pending["challenge_id"].as_str().unwrap(),
                103
            )
            .is_err()
        );
        assert!(register_start(&conn, ORIGIN, &replacement, 103).is_ok());
    }
    #[test]
    fn challenge_consumption_and_limits_fail_closed() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let id = save_challenge(&conn, ORIGIN, "login", "state".into(), 100).unwrap();
        assert!(take_challenge(&conn, ORIGIN, "register", &id, 101).is_err());
        assert_eq!(
            take_challenge(&conn, ORIGIN, "login", &id, 101).unwrap(),
            "state"
        );
        assert!(take_challenge(&conn, ORIGIN, "login", &id, 101).is_err());
        for _ in 0..MAX_PENDING {
            save_challenge(&conn, ORIGIN, "login", "state".into(), 100).unwrap();
        }
        assert!(save_challenge(&conn, ORIGIN, "login", "state".into(), 101).is_err());
        assert!(save_challenge(&conn, ORIGIN, "login", "state".into(), 400).is_ok());
    }
    #[test]
    fn recovery_revokes_sessions_pairing_and_challenges() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let session = crate::web_console_auth::create_passkey_session(&conn, 100).unwrap();
        let code = pair(&conn, ORIGIN, 100).unwrap();
        recover(&conn, 101).unwrap();
        assert!(register_start(&conn, ORIGIN, &code, 102).is_err());
        assert!(
            crate::web_console_auth::validate_session(&conn, &session.cookie_token, 102, 600)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn rejects_insecure_or_ambiguous_origins() {
        for url in [
            "http://example.com",
            "https://a.test/path",
            "https://u:p@a.test",
            "https://a.test/#token",
            "https://a.test/?x=1",
        ] {
            assert!(origin_url(url).is_err(), "{url}");
        }
        assert!(origin_url(ORIGIN).is_ok());
        assert!(origin_url("http://localhost:8765").is_ok());
    }
}
