//! SQLCipher protection for the memory and heirloom databases.
//!
//! The service registers active paths before opening any database. Tests and
//! library consumers that do not register paths retain ordinary SQLite. An
//! existing plaintext database deliberately fails to open after registration;
//! `aegis_runtime_encrypt_db` performs an explicit, copy-only migration.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, backup::Backup};
use zeroize::Zeroizing;

static CONFIG: OnceLock<EncryptionConfig> = OnceLock::new();

struct EncryptionConfig {
    paths: HashSet<PathBuf>,
    secret: Secret,
}

/// Key material supplied outside the database directory. The raw-key option
/// is intended for a secret manager; the passphrase option prompts on a TTY.
/// Neither variant is written to `.env` or to the database directory.
pub enum Secret {
    Raw(Zeroizing<[u8; 32]>),
    Passphrase(Zeroizing<String>),
}

impl Secret {
    pub fn from_external_hex(value: &str) -> Result<Self> {
        if value.len() != 64 {
            bail!("AEGIS_RUNTIME_DB_KEY_HEX must contain exactly 64 hex characters");
        }
        let bytes = hex::decode(value).context("AEGIS_RUNTIME_DB_KEY_HEX is not hex")?;
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("database key must be 32 bytes"))?;
        Ok(Self::Raw(Zeroizing::new(key)))
    }

    fn pragma_key(&self) -> Zeroizing<String> {
        match self {
            Self::Raw(key) => Zeroizing::new(format!(
                "PRAGMA key = \"x'{}'\";",
                hex::encode(key.as_ref())
            )),
            Self::Passphrase(passphrase) => Zeroizing::new(format!(
                "PRAGMA key = '{}';",
                passphrase.replace('\'', "''")
            )),
        }
    }
}

/// Capture the externally injected key before `dotenvy` reads `.env`. When no
/// external key was injected, a foreground invocation can ask for a passphrase.
/// Background services must use the external-key path and fail closed.
pub fn load_secret(external_hex: Option<String>, confirm_new_passphrase: bool) -> Result<Secret> {
    if let Some(value) = external_hex {
        let value = Zeroizing::new(value);
        return Secret::from_external_hex(&value);
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "encrypted databases need AEGIS_RUNTIME_DB_KEY_HEX from an external secret manager; \
             a foreground terminal may instead enter a passphrase"
        );
    }
    let passphrase = Zeroizing::new(
        rpassword::prompt_password("Aegis database passphrase: ")
            .context("reading database passphrase from terminal")?,
    );
    if passphrase.is_empty() || passphrase.contains('\0') {
        bail!("database passphrase must be nonempty and contain no NUL byte");
    }
    if confirm_new_passphrase {
        let confirmation = Zeroizing::new(
            rpassword::prompt_password("Confirm new database passphrase: ")
                .context("confirming database passphrase from terminal")?,
        );
        if passphrase != confirmation {
            bail!("database passphrases did not match; no database was created");
        }
    }
    Ok(Secret::Passphrase(passphrase))
}

/// Register active database paths once, before any connection is opened.
/// A disabled Heirloom database is neither registered nor opened by preflight.
pub fn configure(memory_path: &Path, heirloom_path: Option<&Path>, secret: Secret) -> Result<()> {
    let memory = std::path::absolute(memory_path).context("resolving memory DB path")?;
    let mut paths = HashSet::from([memory.clone()]);
    if let Some(heirloom_path) = heirloom_path {
        let heirloom = std::path::absolute(heirloom_path).context("resolving heirloom DB path")?;
        if memory == heirloom {
            bail!("memory and heirloom databases must use different paths");
        }
        paths.insert(heirloom);
    }
    CONFIG
        .set(EncryptionConfig { paths, secret })
        .map_err(|_| anyhow::anyhow!("database encryption already configured"))
}

/// Check active existing databases before the service can degrade a failed
/// heirloom bridge into a warning. A plaintext file or wrong key must stop
/// startup when that subsystem is enabled.
pub fn preflight_existing() -> Result<()> {
    let config = CONFIG
        .get()
        .context("database encryption has not been configured")?;
    for path in &config.paths {
        if path.exists() {
            let conn = open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("opening protected database {}", path.display()))?;
            drop(conn);
        }
    }
    Ok(())
}

fn key_for(path: &Path) -> rusqlite::Result<Option<&'static Secret>> {
    let Some(config) = CONFIG.get() else {
        return Ok(None);
    };
    let absolute = std::path::absolute(path).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(config.paths.contains(&absolute).then_some(&config.secret))
}

fn key_connection(conn: &Connection, secret: &Secret) -> rusqlite::Result<()> {
    // PRAGMA key must be the first database operation on this handle. The
    // version query prevents a normal SQLite build from silently ignoring it.
    let pragma = secret.pragma_key();
    conn.execute_batch(&pragma)?;
    let version: String = conn.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
    if version.is_empty() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    // Force key validation now. Plaintext files and wrong keys fail here,
    // before any application migration or write can touch them.
    let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
    conn.execute_batch(
        "PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON; PRAGMA cipher_memory_security=ON;",
    )?;
    Ok(())
}

fn reserve_private_file(path: &Path) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?;
    Ok(())
}

/// Open a file-backed database, applying SQLCipher to registered paths.
pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Connection> {
    let path = path.as_ref();
    let secret = key_for(path)?;
    let conn = Connection::open(path)?;
    if let Some(secret) = secret {
        key_connection(&conn, secret)?;
    }
    Ok(conn)
}

/// Open with flags, applying the same key to read-only connections.
pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let path = path.as_ref();
    let secret = key_for(path)?;
    let conn = Connection::open_with_flags(path, flags)?;
    if let Some(secret) = secret {
        key_connection(&conn, secret)?;
    }
    Ok(conn)
}

/// Copy a stopped plaintext SQLite database into a new encrypted file. The
/// source is left untouched. The output only appears at
/// its requested name after export and integrity checks succeed.
pub fn encrypt_plaintext_copy(source: &Path, output: &Path, secret: &Secret) -> Result<()> {
    if !source.is_file() {
        bail!("source database does not exist: {}", source.display());
    }
    if output.exists() {
        bail!("encrypted output already exists: {}", output.display());
    }
    let mut temporary_name = output.as_os_str().to_os_string();
    temporary_name.push(".aegis-encrypting");
    let temporary = PathBuf::from(temporary_name);
    if temporary.exists() {
        bail!("temporary output already exists: {}", temporary.display());
    }
    if std::path::absolute(source)? == std::path::absolute(output)? {
        bail!("source and encrypted output must differ");
    }

    // SQLCipher's ATTACH/export needs a read-write connection even though no
    // statement writes to `main`. Do not allow SQLite to create a source.
    let source_conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening plaintext source {}", source.display()))?;
    source_conn.execute_batch("PRAGMA temp_store=MEMORY;")?;
    let check: String = source_conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        bail!("plaintext source failed quick_check: {check}");
    }
    let user_version: i64 = source_conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let application_id: i64 =
        source_conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;

    // `create_new` reserves the temporary path so ATTACH cannot overwrite an
    // unrelated file. SQLCipher initializes an empty file when attaching it.
    reserve_private_file(&temporary)?;
    let result = (|| -> Result<()> {
        let escaped = temporary.to_string_lossy().replace('\'', "''");
        let key = secret.pragma_key();
        let key_value = key
            .strip_prefix("PRAGMA key = ")
            .and_then(|s| s.strip_suffix(';'))
            .expect("key PRAGMA has a fixed shape");
        let attach = Zeroizing::new(format!(
            "ATTACH DATABASE '{escaped}' AS encrypted KEY {key_value};"
        ));
        source_conn.execute_batch(&attach)?;
        source_conn.execute_batch("SELECT sqlcipher_export('encrypted');")?;
        source_conn.execute_batch(&format!(
            "PRAGMA encrypted.user_version={user_version}; \
             PRAGMA encrypted.application_id={application_id};"
        ))?;
        source_conn.execute_batch("DETACH DATABASE encrypted;")?;

        let encrypted = Connection::open(&temporary)?;
        key_connection(&encrypted, secret)?;
        let check: String = encrypted.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            bail!("encrypted output failed quick_check: {check}");
        }
        let mut integrity = encrypted.prepare("PRAGMA cipher_integrity_check")?;
        let mut rows = integrity.query([])?;
        if rows.next()?.is_some() {
            bail!("encrypted output failed cipher_integrity_check");
        }
        drop(rows);
        drop(integrity);
        drop(encrypted);
        // Same-directory hard link is an atomic no-clobber publish. `rename`
        // would overwrite a file created at `output` after our earlier check.
        std::fs::hard_link(&temporary, output)?;
        std::fs::remove_file(&temporary)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Take a consistent online backup of an encrypted database, including WAL
/// content, into another encrypted file. SQLCipher requires identical cipher
/// settings for the source and destination of SQLite's backup API.
pub fn backup_encrypted_copy(source: &Path, output: &Path, secret: &Secret) -> Result<()> {
    if !source.is_file() {
        bail!("source database does not exist: {}", source.display());
    }
    if output.exists() || std::path::absolute(source)? == std::path::absolute(output)? {
        bail!("backup output must be new and different from the source");
    }
    let mut temporary_name = output.as_os_str().to_os_string();
    temporary_name.push(".aegis-backing-up");
    let temporary = PathBuf::from(temporary_name);
    reserve_private_file(&temporary)
        .with_context(|| format!("reserving backup output {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        let source_conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        key_connection(&source_conn, secret)?;
        let mut destination = Connection::open(&temporary)?;
        key_connection(&destination, secret)?;
        {
            let backup = Backup::new(&source_conn, &mut destination)?;
            backup.run_to_completion(128, Duration::from_millis(50), None)?;
        }
        let check: String = destination.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            bail!("backup failed quick_check: {check}");
        }
        let mut integrity = destination.prepare("PRAGMA cipher_integrity_check")?;
        if integrity.query([])?.next()?.is_some() {
            bail!("backup failed cipher_integrity_check");
        }
        drop(integrity);
        drop(destination);
        std::fs::hard_link(&temporary, output)?;
        std::fs::remove_file(&temporary)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_secret() -> Secret {
        Secret::Raw(Zeroizing::new([0xA5; 32]))
    }

    #[test]
    fn encrypted_file_and_wal_do_not_expose_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memory.db");
        let conn = Connection::open(&path).unwrap();
        key_connection(&conn, &test_secret()).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE secret(value TEXT);")
            .unwrap();
        conn.execute(
            "INSERT INTO secret(value) VALUES (?1)",
            ["private-memory-marker"],
        )
        .unwrap();
        let wal = std::fs::read(path.with_extension("db-wal")).unwrap();
        assert!(
            !wal.windows(b"private-memory-marker".len())
                .any(|w| w == b"private-memory-marker")
        );
        drop(conn);
        let file = std::fs::read(&path).unwrap();
        assert!(!file.starts_with(b"SQLite format 3"));
        assert!(
            !file
                .windows(b"private-memory-marker".len())
                .any(|w| w == b"private-memory-marker")
        );
        let reopened = Connection::open(&path).unwrap();
        key_connection(&reopened, &test_secret()).unwrap();
        let value: String = reopened
            .query_row("SELECT value FROM secret", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "private-memory-marker");
        let wrong = Connection::open(&path).unwrap();
        assert!(key_connection(&wrong, &Secret::Raw(Zeroizing::new([0x5A; 32]))).is_err());
        let readonly =
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        key_connection(&readonly, &test_secret()).unwrap();
    }

    #[test]
    fn plaintext_migration_preserves_source_and_user_version() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("old.db");
        let output = dir.path().join("new.db");
        let conn = Connection::open(&source).unwrap();
        conn.execute_batch("PRAGMA user_version=7; CREATE TABLE memories(value TEXT);")
            .unwrap();
        conn.execute("INSERT INTO memories(value) VALUES (?1)", ["old-secret"])
            .unwrap();
        drop(conn);
        let plaintext = Connection::open(&source).unwrap();
        assert!(key_connection(&plaintext, &test_secret()).is_err());
        drop(plaintext);
        encrypt_plaintext_copy(&source, &output, &test_secret()).unwrap();
        assert!(
            std::fs::read(&source)
                .unwrap()
                .starts_with(b"SQLite format 3")
        );
        assert!(
            !std::fs::read(&output)
                .unwrap()
                .starts_with(b"SQLite format 3")
        );
        let encrypted = Connection::open(&output).unwrap();
        key_connection(&encrypted, &test_secret()).unwrap();
        let value: String = encrypted
            .query_row("SELECT value FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "old-secret");
        let version: i64 = encrypted
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 7);
    }

    #[test]
    fn online_backup_copies_encrypted_wal_content_without_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("memory.db");
        let output = dir.path().join("backup.db");
        let conn = Connection::open(&source).unwrap();
        key_connection(&conn, &test_secret()).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE private(value TEXT);")
            .unwrap();
        conn.execute(
            "INSERT INTO private(value) VALUES (?1)",
            ["wal-secret-marker"],
        )
        .unwrap();
        backup_encrypted_copy(&source, &output, &test_secret()).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"));
        assert!(
            !bytes
                .windows(b"wal-secret-marker".len())
                .any(|w| w == b"wal-secret-marker")
        );
        let backup = Connection::open(&output).unwrap();
        key_connection(&backup, &test_secret()).unwrap();
        let value: String = backup
            .query_row("SELECT value FROM private", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "wal-secret-marker");
        let wrong_output = dir.path().join("wrong-key.db");
        assert!(
            backup_encrypted_copy(
                &source,
                &wrong_output,
                &Secret::Raw(Zeroizing::new([0x5A; 32])),
            )
            .is_err()
        );
        assert!(!wrong_output.exists());
    }
}
