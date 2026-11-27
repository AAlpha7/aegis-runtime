//! Consistent online backup of a SQLCipher database without plaintext output.

use std::path::PathBuf;

use anyhow::{Result, bail};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (Some(source), Some(output), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: aegis_runtime_backup_db SOURCE.db BACKUP.db");
    };
    let secret = aegis_runtime_core::encrypted_sqlite::load_secret(
        std::env::var("AEGIS_RUNTIME_DB_KEY_HEX").ok(),
        false,
    )?;
    let source = PathBuf::from(source);
    let output = PathBuf::from(output);
    aegis_runtime_core::encrypted_sqlite::backup_encrypted_copy(&source, &output, &secret)?;
    println!("verified encrypted backup: {}", output.display());
    Ok(())
}
