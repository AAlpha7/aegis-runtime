//! Copy-only migration from a stopped plaintext SQLite database to SQLCipher.

use std::path::PathBuf;

use anyhow::{Result, bail};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(source) = args.next() else {
        bail!("usage: aegis_runtime_encrypt_db SOURCE.db ENCRYPTED.db");
    };
    let Some(output) = args.next() else {
        bail!("usage: aegis_runtime_encrypt_db SOURCE.db ENCRYPTED.db");
    };
    if args.next().is_some() {
        bail!("usage: aegis_runtime_encrypt_db SOURCE.db ENCRYPTED.db");
    }
    let source = PathBuf::from(source);
    let output = PathBuf::from(output);
    let secret = aegis_runtime_core::encrypted_sqlite::load_secret(
        std::env::var("AEGIS_RUNTIME_DB_KEY_HEX").ok(),
        true,
    )?;
    aegis_runtime_core::encrypted_sqlite::encrypt_plaintext_copy(&source, &output, &secret)?;
    println!("verified encrypted copy: {}", output.display());
    println!("plaintext source retained: {}", source.display());
    Ok(())
}
