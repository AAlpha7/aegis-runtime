//! Host-only pairing and recovery; no model provider or network login required.
use anyhow::{Result, bail};
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3
        || args.len() > 4
        || !["pair", "recover", "status"].contains(&args[0].as_str())
    {
        bail!(
            "usage: aegis_runtime_access pair|status MEMORY.db HTTPS_ORIGIN\n       aegis_runtime_access recover MEMORY.db HTTPS_ORIGIN --confirm-reset"
        );
    }
    if args[0] == "recover" && args.get(3).map(String::as_str) != Some("--confirm-reset") {
        bail!("Recovery revokes ALL owner Passkeys and sessions. Add --confirm-reset to proceed.");
    }
    if args[0] != "recover" && args.len() != 3 {
        bail!("unexpected argument");
    }
    aegis_runtime_core::remote_access::origin_url(&args[2])?;
    if !Path::new(&args[1]).is_file() {
        bail!("Start Aegis locally first; memory database does not exist");
    }
    let secret = aegis_runtime_core::encrypted_sqlite::load_secret(
        std::env::var("AEGIS_RUNTIME_DB_KEY_HEX").ok(),
        false,
    )?;
    aegis_runtime_core::encrypted_sqlite::configure(Path::new(&args[1]), None, secret)?;
    let conn = aegis_runtime_core::encrypted_sqlite::open(&args[1])?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    if args[0] == "status" {
        println!(
            "registered_passkeys={}",
            aegis_runtime_core::remote_access::count(&conn, &args[2])?
        );
        return Ok(());
    }
    if args[0] == "recover" {
        aegis_runtime_core::remote_access::recover(&conn, now)?;
    }
    let code = aegis_runtime_core::remote_access::pair(&conn, &args[2], now)?;
    println!(
        "Open on your phone (private mode requires Tailscale). This single-use link expires in 10 minutes; keep it private:\n{}/access#pair={code}",
        args[2].trim_end_matches('/')
    );
    Ok(())
}
