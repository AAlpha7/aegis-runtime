//! Loopback-only behavioral-test harness using the REAL authentication router.
//! No model credentials, test-only auth bypass, or public listener. Not packaged.
use aegis_runtime_core::{
    admin::{AdminState, oauth_callback_router},
    encrypted_sqlite,
    memory::MemoryHandle,
    remote_access,
    sandbox::SandboxEngine,
    settings::RuntimeSettings,
};
use std::{path::Path, sync::Arc};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 2, "usage: remote_access_harness TEST_DB PORT");
    let port: u16 = args[1].parse()?;
    let origin = format!("http://localhost:{port}");
    encrypted_sqlite::configure(
        Path::new(&args[0]),
        None,
        encrypted_sqlite::load_secret(std::env::var("AEGIS_RUNTIME_DB_KEY_HEX").ok(), false)?,
    )?;
    let conn = encrypted_sqlite::open(&args[0])?;
    remote_access::init(&conn)?;
    drop(conn);
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    let engine = wasmtime::Engine::new(&config)?;
    let state = Arc::new(AdminState {
        sandbox: Arc::new(SandboxEngine::from_engine(engine)?),
        memory: MemoryHandle::spawn_in_memory()?,
        api_token: None,
        settings: Arc::new(RuntimeSettings::new()),
        credential_db_path: Some(args[0].clone()),
        credential_aes_key: Some([42; 32]), // Synthetic test-only key; DB still uses SQLCipher.
        google_auth: parking_lot::RwLock::new(None),
        telegram_bot: None,
        identity: None,
        public_url: Some(origin),
        bot_token: None,
        admin_chat_id: Some(1),
        orchestrator: None,
        event_tx: None,
        db_path: Some(args[0].clone()),
        progress_router: None,
        local_login_token: Some("test-local-token-must-not-work-remotely".into()),
    });
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!("ready");
    // Explicit injected upstream response for account-free UI tests. Production
    // routers never install this middleware or read a bypass environment flag.
    let router = oauth_callback_router(state)
        .layer(axum::middleware::from_fn(
            |req: axum::extract::Request, next: axum::middleware::Next| async move {
                aegis_runtime_core::call_validation::with_test_probe(
                    Arc::new(|candidate| {
                        if let aegis_runtime_core::call_validation::Candidate::Jev { key } =
                            candidate
                            && key.contains("rejected")
                        {
                            anyhow::bail!("401 synthetic validation failure");
                        }
                        Ok(())
                    }),
                    next.run(req),
                )
                .await
            },
        ))
        .route("/health", axum::routing::get(|| async { "ok" }));
    axum::serve(listener, router).await?;
    Ok(())
}
