//! `aegis_runtime_core setup-auth` — bind owner chat to a provider BEFORE the
//! service boots.
//!
//! The Web Console path (Settings → Grok/Codex → Connect → **Use for chat**)
//! only exists once the service is up, which used to leave a fresh install
//! answering owner chat with 503 until two more sessions were completed. This
//! headless equivalent writes the same encrypted credential and the same
//! runtime settings ([`ChatProviderSelection::runtime_settings`]) into the data
//! files the service opens at boot, so the first boot already has a working
//! chat provider.
//!
//! Usage (from the checkout root or with the same env the service uses):
//!
//! ```text
//! aegis_runtime_core setup-auth --provider grok      # xAI device code (≥10s after slow_down) + Use for chat
//! aegis_runtime_core setup-auth --provider codex     # codex login (ChatGPT) + Use for chat
//! aegis_runtime_core setup-auth --provider api-key   # validate OPENAI_API_KEY, keep the legacy openai_compat path
//! aegis_runtime_core setup-auth status               # exit 0 = chat will work after boot, 2 = not yet
//! ```

use super::*;
use aegis_runtime_core::llm::{
    XaiDeviceAuthorization, XaiDeviceCodePoll, XaiSubscriptionAuthConfig,
    XaiSubscriptionAuthManager,
};
use aegis_runtime_core::settings::ChatProviderSelection;
use aegis_runtime_core::startup_reasoner;

pub(super) const DEFAULT_IDENTITY_KEY_PATH: &str = "./aegis_runtime_identity.key";
pub(super) const DEFAULT_CREDENTIAL_DB_PATH: &str = "./aegis_runtime_credentials.db";
pub(super) const DEFAULT_MEMORY_DB_PATH: &str = "./aegis_runtime_memory.db";

/// Exit code for `status` when owner chat would still be unavailable after boot.
pub(super) const SETUP_AUTH_NOT_READY: i32 = 2;

const GROK_CLI_INSTALL_ONE_LINER: &str = "curl -fsSL https://x.ai/cli/install.sh | bash";

pub(super) fn env_or(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SetupAuthCommand {
    Grok { reconnect: bool },
    Codex,
    ApiKey,
    Status,
    VerifyJev,
}

/// Parse `setup-auth …` from the raw argv (without the program name). Returns
/// `Ok(None)` when the invocation is not a `setup-auth` call at all so normal
/// boot proceeds.
pub(super) fn parse_setup_auth_args<I>(args: I) -> Result<Option<SetupAuthCommand>>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    if args.next().as_deref() != Some("setup-auth") {
        return Ok(None);
    }
    let mut provider: Option<String> = None;
    let mut reconnect = false;
    let mut status = false;
    let mut verify_jev = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "status" => status = true,
            "verify-jev" => verify_jev = true,
            "--reconnect" => reconnect = true,
            "--provider" => {
                provider = Some(args.next().ok_or_else(|| {
                    anyhow::anyhow!("setup-auth: --provider needs a value (grok|codex|api-key)")
                })?);
            }
            other if other.starts_with("--provider=") => {
                provider = Some(other["--provider=".len()..].to_string());
            }
            other => anyhow::bail!("setup-auth: unknown argument `{other}`"),
        }
    }
    if verify_jev {
        if status || provider.is_some() || reconnect {
            anyhow::bail!("setup-auth: `verify-jev` takes no other flags");
        }
        return Ok(Some(SetupAuthCommand::VerifyJev));
    }
    if status {
        if provider.is_some() || reconnect {
            anyhow::bail!("setup-auth: `status` takes no other flags");
        }
        return Ok(Some(SetupAuthCommand::Status));
    }
    match provider.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("grok" | "xai") => Ok(Some(SetupAuthCommand::Grok { reconnect })),
        Some("codex") if !reconnect => Ok(Some(SetupAuthCommand::Codex)),
        Some("api-key" | "api_key" | "apikey") if !reconnect => Ok(Some(SetupAuthCommand::ApiKey)),
        Some("codex" | "api-key" | "api_key" | "apikey") => {
            anyhow::bail!("setup-auth: --reconnect only applies to --provider grok")
        }
        Some(other) => anyhow::bail!(
            "setup-auth: unknown provider `{other}` (expected grok, codex, or api-key)"
        ),
        None => anyhow::bail!(
            "setup-auth: missing --provider grok|codex|api-key (or `setup-auth status`)"
        ),
    }
}

fn say(msg: impl AsRef<str>) {
    eprintln!("[setup-auth] {}", msg.as_ref());
}

struct DataFiles {
    identity_key: String,
    credential_db: String,
    memory_db: String,
}

impl DataFiles {
    fn from_env() -> Self {
        Self {
            identity_key: env_or("IDENTITY_KEY_PATH", DEFAULT_IDENTITY_KEY_PATH),
            credential_db: env_or("CREDENTIAL_DB_PATH", DEFAULT_CREDENTIAL_DB_PATH),
            memory_db: env_or("MEMORY_DB_PATH", DEFAULT_MEMORY_DB_PATH),
        }
    }

    /// Same identity → same AES key → the service can decrypt what we store.
    fn open_credential_store(&self) -> Result<credential_store::CredentialStore> {
        ensure_parent_dir(&self.identity_key)?;
        ensure_parent_dir(&self.credential_db)?;
        let identity = AgentIdentity::load_or_generate(Path::new(&self.identity_key))
            .with_context(|| format!("loading identity key {}", self.identity_key))?;
        let aes_key = credential_store::derive_key_from_ed25519(&identity.signing_key_bytes());
        let conn = aegis_runtime_core::encrypted_sqlite::open(&self.credential_db)
            .with_context(|| format!("opening credential store {}", self.credential_db))?;
        credential_store::CredentialStore::open(conn, aes_key)
            .map_err(|e| anyhow::anyhow!("initializing credential store: {e}"))
    }

    fn open_memory(&self) -> Result<MemoryHandle> {
        ensure_parent_dir(&self.memory_db)?;
        MemoryHandle::spawn_with_config(
            Path::new(&self.memory_db),
            aegis_runtime_core::memory::MemoryConfig::default(),
        )
        .with_context(|| format!("opening memory database {}", self.memory_db))
    }

    /// Persisted `llm_provider`, or `None` when the database does not exist yet
    /// (`status` must not create data files on a machine that never booted).
    async fn stored_chat_provider(&self) -> Result<Option<String>> {
        if !Path::new(&self.memory_db).exists() {
            return Ok(None);
        }
        let memory = self.open_memory()?;
        let settings = memory.load_all_settings().await?;
        Ok(settings
            .get("llm_provider")
            .and_then(|value| value.as_str().map(str::to_owned)))
    }
}

fn ensure_parent_dir(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating data directory {}", parent.display()))?;
    }
    Ok(())
}

/// Write exactly what Settings → **Use for chat** writes, validating every key
/// against the runtime registry first so a typo can never reach SQLite.
async fn persist_settings(
    memory: &MemoryHandle,
    entries: Vec<(&str, serde_json::Value)>,
) -> Result<()> {
    let registry = Arc::new(RuntimeSettings::new());
    let existing = memory.load_all_settings().await?;
    let updates = entries
        .into_iter()
        .filter(|(key, _)| {
            *key != "external_executor_enabled"
                || existing.get(*key) != Some(&serde_json::Value::Bool(false))
        })
        .map(|(key, value)| (key.to_owned(), Some(value)))
        .collect();
    memory.commit_runtime_settings(registry, updates).await?;
    Ok(())
}

async fn verified_settings(
    memory: &MemoryHandle,
    entries: Vec<(&str, serde_json::Value)>,
    probe: impl std::future::Future<Output = Result<()>>,
) -> Result<i32> {
    say("Checking a real response with the selected provider (uses its quota); no paid fallback.");
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(70), probe).await,
        Ok(Ok(()))
    ) {
        say(
            "Validation failed or timed out. Call settings were not saved; the previous selection is unchanged.",
        );
        return Ok(SETUP_AUTH_NOT_READY);
    }
    persist_settings(memory, entries).await?;
    Ok(0)
}

async fn setup_settings(memory: &MemoryHandle) -> Result<RuntimeSettings> {
    let settings = RuntimeSettings::new();
    settings.load_from_map(memory.load_all_settings().await?);
    Ok(settings)
}

fn setting_text(settings: &RuntimeSettings, name: &str) -> String {
    settings
        .get(name)
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

// ── grok ────────────────────────────────────────────────────────────

struct GrokCliSession {
    binary: Option<PathBuf>,
    auth_file: Option<PathBuf>,
}

fn grok_cli_session() -> GrokCliSession {
    let grok_bin = env_or("AEGIS_EXECUTOR_GROK_BIN", "grok");
    let on_path =
        startup_reasoner::host_binary_available(&grok_bin).then(|| PathBuf::from(&grok_bin));
    let official = startup_reasoner::official_grok_bin().filter(|p| p.is_file());
    let auth_file = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".grok").join("auth.json"))
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false));
    GrokCliSession {
        binary: on_path.or(official),
        auth_file,
    }
}

fn describe_grok_cli(session: &GrokCliSession) -> String {
    match (&session.binary, &session.auth_file) {
        (Some(bin), Some(auth)) => {
            format!("present ({} + session {})", bin.display(), auth.display())
        }
        (Some(bin), None) => format!(
            "installed at {} but NOT logged in — run: grok login --device-auth",
            bin.display()
        ),
        (None, _) => format!("not installed — run: {GROK_CLI_INSTALL_ONE_LINER}"),
    }
}

async fn run_grok(files: &DataFiles, reconnect: bool) -> Result<i32> {
    let store = files.open_credential_store()?;
    let existing = XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(|e| anyhow::anyhow!("reading stored Grok credential: {e}"))?;
    match existing {
        Some(credential) if !reconnect => {
            say(format!(
                "Grok (in-app) already connected{} — reusing it (pass --reconnect to sign in again)",
                credential
                    .email
                    .or(credential.display_name)
                    .or(credential.account_id)
                    .map(|label| format!(" as {label}"))
                    .unwrap_or_default()
            ));
        }
        _ => connect_grok_device_code(&store).await?,
    }

    let memory = files.open_memory()?;
    let settings = setup_settings(&memory).await?;
    let credential = XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(|_| anyhow::anyhow!("Grok credential unavailable"))?
        .context("Grok credential unavailable")?;
    let model = setting_text(&settings, "xai_subscription_model");
    let probe = async {
        use aegis_runtime_core::llm::{
            AegisChatRequest, AegisMessage, AegisMessageRole, AegisRequestMetadata, LlmBackend,
            LlmTaskClass, XaiSubscriptionBackend,
        };
        let backend = XaiSubscriptionBackend::with_credential(&model, credential);
        let response = backend
            .chat(AegisChatRequest {
                model: model.clone(),
                messages: vec![AegisMessage {
                    role: AegisMessageRole::User,
                    content:
                        "Reply with exactly AEGIS_OK. Connection check only; do not use tools."
                            .into(),
                }],
                temperature: None,
                max_tokens: Some(128),
                tools: vec![],
                tool_choice: None,
                metadata: AegisRequestMetadata {
                    task_class: LlmTaskClass::ForegroundUserRequest,
                    ..Default::default()
                },
            })
            .await?;
        anyhow::ensure!(
            !response.message.content.trim().is_empty(),
            "empty probe response"
        );
        Ok(())
    };
    let result = verified_settings(
        &memory,
        ChatProviderSelection::Grok.runtime_settings(),
        probe,
    )
    .await?;
    if result != 0 {
        return Ok(result);
    }
    say(
        "Use for chat: llm_provider=xai_subscription, xai_subscription_enabled=true (background use + paid fallback stay off)",
    );

    let cli = grok_cli_session();
    say(format!(
        "grok CLI (startup gate): {}",
        describe_grok_cli(&cli)
    ));
    if cli.binary.is_none() || cli.auth_file.is_none() {
        say(
            "Owner chat is bound, but the service will refuse to boot until the grok CLI session above exists (or OPENAI_API_KEY / codex is available).",
        );
        return Ok(SETUP_AUTH_NOT_READY);
    }
    say("READY — start the service; owner chat uses Grok from the first boot.");
    Ok(0)
}

async fn connect_grok_device_code(store: &credential_store::CredentialStore) -> Result<()> {
    let manager = XaiSubscriptionAuthManager::new(XaiSubscriptionAuthConfig::default());
    let mut authorization = manager
        .start_device_code()
        .await
        .map_err(|e| anyhow::anyhow!("starting xAI device code: {}", e.message))?;
    // Mirror the console flow so Settings shows "pending" if someone looks.
    XaiSubscriptionAuthManager::store_pending_device_authorization(store, &authorization)
        .map_err(|e| anyhow::anyhow!("saving pending device authorization: {}", e.message))?;

    let open_url = authorization
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| authorization.verification_uri.clone());
    eprintln!();
    eprintln!("  Open   : {open_url}");
    eprintln!("  Code   : {}", authorization.user_code);
    eprintln!(
        "  Expires: in {} min",
        authorization
            .expires_at_ms
            .saturating_sub(now_ms())
            .div_ceil(60_000)
    );
    eprintln!();
    say(format!(
        "Waiting for xAI confirmation (polling every {}s; slow_down raises this to >= {}s)…",
        authorization.interval_secs,
        aegis_runtime_core::llm::xai_subscription::XAI_DEVICE_CODE_SLOW_DOWN_MIN_INTERVAL_SECS
    ));

    let credential = loop {
        tokio::time::sleep(std::time::Duration::from_secs(authorization.interval_secs)).await;
        match manager.poll_device_code_once(&authorization).await {
            Ok(XaiDeviceCodePoll::Pending) => {}
            Ok(XaiDeviceCodePoll::SlowDown { interval_secs }) => {
                authorization = XaiDeviceAuthorization {
                    interval_secs,
                    ..authorization
                };
                XaiSubscriptionAuthManager::store_pending_device_authorization(
                    store,
                    &authorization,
                )
                .map_err(|e| {
                    anyhow::anyhow!("saving slowed device authorization: {}", e.message)
                })?;
                say(format!(
                    "xAI asked to slow down — polling every {interval_secs}s now"
                ));
            }
            Ok(XaiDeviceCodePoll::Authorized(credential)) => break credential,
            Err(err) => {
                let _ = XaiSubscriptionAuthManager::delete_pending_device_authorization(store);
                anyhow::bail!("xAI device sign-in failed: {}", err.message);
            }
        }
    };

    XaiSubscriptionAuthManager::store_credential(store, &credential)
        .map_err(|e| anyhow::anyhow!("saving Grok credential: {}", e.message))?;
    let _ = XaiSubscriptionAuthManager::delete_pending_device_authorization(store);
    say(format!(
        "Grok (in-app) connected{}",
        credential
            .email
            .or(credential.display_name)
            .or(credential.account_id)
            .map(|label| format!(" as {label}"))
            .unwrap_or_default()
    ));
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── codex ───────────────────────────────────────────────────────────

fn codex_binary() -> String {
    let configured = env_or("AEGIS_EXECUTOR_CODEX_BIN", "codex");
    if startup_reasoner::host_binary_available(&configured) {
        return configured;
    }
    "codex".to_string()
}

/// Absolute path for the `codex_cli_binary` setting (the registry default is
/// `/usr/bin/codex`, which is wrong on npm-installed hosts).
fn resolve_on_path(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

async fn run_codex(files: &DataFiles) -> Result<i32> {
    let binary = codex_binary();
    let codex_home = aegis_runtime_core::settings::default_codex_home();
    let Some(binary_path) = resolve_on_path(&binary) else {
        say(format!(
            "Codex CLI `{binary}` not found on PATH — install it (npm install -g @openai/codex) and re-run"
        ));
        return Ok(SETUP_AUTH_NOT_READY);
    };
    let binary_str = binary_path.display().to_string();

    let mut login =
        aegis_runtime_core::admin::codex_cli_login_status_at(&binary_str, &codex_home).await;
    if !login.logged_in {
        say(format!(
            "Codex CLI not logged in with ChatGPT ({}) — starting `codex login --device-auth` (CODEX_HOME={codex_home})",
            login.detail.lines().next().unwrap_or("no detail")
        ));
        let status = tokio::process::Command::new(&binary_path)
            .arg("login")
            .arg("--device-auth")
            .env("CODEX_HOME", &codex_home)
            .status()
            .await
            .with_context(|| format!("running {binary_str} login --device-auth"))?;
        if !status.success() {
            anyhow::bail!("`codex login --device-auth` exited with {status}");
        }
        login =
            aegis_runtime_core::admin::codex_cli_login_status_at(&binary_str, &codex_home).await;
        if !login.logged_in {
            anyhow::bail!(
                "Codex CLI still not logged in with ChatGPT after login: {}",
                login.detail
            );
        }
    }
    say(format!(
        "Codex CLI logged in ({})",
        login.auth_kind.as_deref().unwrap_or("ChatGPT")
    ));

    let memory = files.open_memory()?;
    let mut entries = ChatProviderSelection::Codex.runtime_settings();
    entries.push((
        "codex_cli_binary",
        serde_json::Value::String(binary_str.clone()),
    ));
    entries.push((
        "codex_cli_home",
        serde_json::Value::String(codex_home.clone()),
    ));
    let settings = setup_settings(&memory).await?;
    let model = setting_text(&settings, "codex_model");
    let effort = setting_text(&settings, "codex_reasoning_effort");
    let cwd = std::env::current_dir()?.display().to_string();
    let result = verified_settings(
        &memory,
        entries,
        aegis_runtime_core::orchestrator::verify_codex_connection(
            &binary_str,
            &codex_home,
            &cwd,
            &model,
            if effort.is_empty() {
                None
            } else {
                Some(&effort)
            },
        ),
    )
    .await?;
    if result != 0 {
        return Ok(result);
    }
    say(format!(
        "Use for chat: llm_provider=codex_oauth, codex_enabled=true, codex_cli_binary={binary_str}, codex_cli_home={codex_home}"
    ));
    say("READY — start the service; owner chat uses Codex from the first boot.");
    Ok(0)
}

// ── api key ─────────────────────────────────────────────────────────

async fn run_api_key(files: &DataFiles) -> Result<i32> {
    let key = std::env::var("OPENAI_API_KEY").ok();
    if let Err(err) = startup_reasoner::validate_llm_api_key(key.as_deref()) {
        say(format!("OPENAI_API_KEY is not usable: {err}"));
        say(
            "Put a real key (and OPENAI_API_BASE) in .env, then re-run `setup-auth --provider api-key`.",
        );
        return Ok(SETUP_AUTH_NOT_READY);
    }
    let memory = files.open_memory()?;
    let settings = setup_settings(&memory).await?;
    let stored_model = setting_text(&settings, "llm_model");
    let model = if stored_model.trim().is_empty() {
        env_or("LLM_MODEL", "")
    } else {
        stored_model
    };
    let client = aegis_runtime_core::llm_client::create_llm_client(
        key.as_deref().unwrap_or_default(),
        &env_or("OPENAI_API_BASE", "https://api.openai.com/v1"),
    );
    let probe = async {
        use async_openai::types::chat::{
            ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
        };
        anyhow::ensure!(
            !model.trim().is_empty(),
            "Choose LLM_MODEL before validating the API provider"
        );
        let request = CreateChatCompletionRequestArgs::default()
            .model(&model)
            .messages([ChatCompletionRequestUserMessageArgs::default()
                .content("Reply with exactly AEGIS_OK. Connection check only.")
                .build()?
                .into()])
            .max_completion_tokens(128u32)
            .build()?;
        let response = client.chat().create(request).await?;
        anyhow::ensure!(
            response
                .choices
                .first()
                .and_then(|c| c.message.content.as_ref())
                .is_some_and(|text| !text.trim().is_empty()),
            "empty probe response"
        );
        Ok(())
    };
    let result = verified_settings(
        &memory,
        vec![
            (
                "llm_provider",
                serde_json::Value::String("openai_compat".into()),
            ),
            ("codex_enabled", serde_json::Value::Bool(false)),
            ("xai_subscription_enabled", serde_json::Value::Bool(false)),
        ],
        probe,
    )
    .await?;
    if result != 0 {
        return Ok(result);
    }
    say("Use for chat: llm_provider=openai_compat (API key path); Grok/Codex chat routing off");
    say("READY — start the service; owner chat uses OPENAI_API_KEY from the first boot.");
    Ok(0)
}

// ── status ──────────────────────────────────────────────────────────

async fn run_status(files: &DataFiles) -> Result<i32> {
    let cli = grok_cli_session();
    say(format!("grok CLI       : {}", describe_grok_cli(&cli)));

    let grok_in_app =
        if Path::new(&files.credential_db).exists() && Path::new(&files.identity_key).exists() {
            let store = files.open_credential_store()?;
            XaiSubscriptionAuthManager::load_credential(&store)
                .map_err(|e| anyhow::anyhow!("reading stored Grok credential: {e}"))?
        } else {
            None
        };
    say(format!(
        "Grok (in-app)  : {}",
        match &grok_in_app {
            Some(c) => format!(
                "connected{}",
                c.email
                    .clone()
                    .or(c.display_name.clone())
                    .or(c.account_id.clone())
                    .map(|l| format!(" as {l}"))
                    .unwrap_or_default()
            ),
            None => "not connected — setup-auth --provider grok".to_string(),
        }
    ));

    let codex_home = aegis_runtime_core::settings::default_codex_home();
    let codex_logged_in = match resolve_on_path(&codex_binary()) {
        Some(bin) => {
            let login = aegis_runtime_core::admin::codex_cli_login_status_at(
                &bin.display().to_string(),
                &codex_home,
            )
            .await;
            say(format!(
                "Codex CLI      : {} ({})",
                if login.logged_in {
                    "logged in"
                } else {
                    "not logged in"
                },
                login.detail.lines().next().unwrap_or("")
            ));
            login.logged_in
        }
        None => {
            say("Codex CLI      : not installed");
            false
        }
    };

    let api_key_usable =
        startup_reasoner::llm_api_key_usable(std::env::var("OPENAI_API_KEY").ok().as_deref());
    say(format!(
        "OPENAI_API_KEY : {}",
        if api_key_usable {
            "usable"
        } else {
            "unset or placeholder"
        }
    ));

    let provider = files
        .stored_chat_provider()
        .await?
        .unwrap_or_else(|| "openai_compat".to_string());
    say(format!(
        "Chat provider  : {provider} (stored llm_provider; unset = openai_compat)"
    ));

    let boot_gate =
        api_key_usable || (cli.binary.is_some() && cli.auth_file.is_some()) || codex_logged_in;
    let chat_ready = match provider.as_str() {
        "xai_subscription" | "grok_subscription" => grok_in_app.is_some(),
        "codex_oauth" => codex_logged_in,
        _ => api_key_usable,
    };
    if boot_gate && chat_ready {
        say("Verdict        : READY — owner chat will work after boot");
        Ok(0)
    } else {
        say(format!(
            "Verdict        : NOT READY — boot gate {}, chat provider `{provider}` {}. Run: aegis_runtime_core setup-auth --provider grok|codex|api-key",
            if boot_gate {
                "satisfied"
            } else {
                "unsatisfied (need grok CLI session, codex login, or OPENAI_API_KEY)"
            },
            if chat_ready { "bound" } else { "not bound" }
        ));
        Ok(SETUP_AUTH_NOT_READY)
    }
}

pub(super) async fn run_setup_auth(command: SetupAuthCommand) -> Result<i32> {
    let files = DataFiles::from_env();
    say(format!(
        "data files: identity={} credentials={} memory={}",
        files.identity_key, files.credential_db, files.memory_db
    ));
    match command {
        SetupAuthCommand::Grok { reconnect } => run_grok(&files, reconnect).await,
        SetupAuthCommand::Codex => run_codex(&files).await,
        SetupAuthCommand::ApiKey => run_api_key(&files).await,
        SetupAuthCommand::Status => run_status(&files).await,
        SetupAuthCommand::VerifyJev => {
            let key = std::env::var("AEGIS_JEV_API_KEY").unwrap_or_default();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(70),
                aegis_runtime_core::call_validation::verify_jev(&key),
            )
            .await;
            if matches!(result, Ok(Ok(()))) {
                say("Jev live validation passed; no settings changed by this probe");
                Ok(0)
            } else {
                say(
                    "Jev validation failed or timed out; no settings changed (provider details withheld)",
                );
                Ok(SETUP_AUTH_NOT_READY)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn non_setup_auth_argv_is_ignored() {
        assert_eq!(parse_setup_auth_args(argv(&[])).unwrap(), None);
        assert_eq!(
            parse_setup_auth_args(argv(&["--print-pubkey"])).unwrap(),
            None
        );
    }

    #[test]
    fn parses_every_provider_spelling() {
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "--provider", "grok"])).unwrap(),
            Some(SetupAuthCommand::Grok { reconnect: false })
        );
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "--provider=Grok", "--reconnect"])).unwrap(),
            Some(SetupAuthCommand::Grok { reconnect: true })
        );
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "--provider", "codex"])).unwrap(),
            Some(SetupAuthCommand::Codex)
        );
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "--provider", "api-key"])).unwrap(),
            Some(SetupAuthCommand::ApiKey)
        );
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "status"])).unwrap(),
            Some(SetupAuthCommand::Status)
        );
    }

    #[test]
    fn rejects_incomplete_or_unknown_arguments() {
        assert_eq!(
            parse_setup_auth_args(argv(&["setup-auth", "verify-jev"])).unwrap(),
            Some(SetupAuthCommand::VerifyJev)
        );
        assert!(parse_setup_auth_args(argv(&["setup-auth", "verify-jev", "status"])).is_err());
        assert!(
            parse_setup_auth_args(argv(&["setup-auth", "verify-jev", "--provider", "codex"]))
                .is_err()
        );
        assert!(parse_setup_auth_args(argv(&["setup-auth"])).is_err());
        assert!(parse_setup_auth_args(argv(&["setup-auth", "--provider"])).is_err());
        assert!(parse_setup_auth_args(argv(&["setup-auth", "--provider", "ollama"])).is_err());
        assert!(parse_setup_auth_args(argv(&["setup-auth", "--bogus"])).is_err());
        assert!(
            parse_setup_auth_args(argv(&["setup-auth", "status", "--provider", "grok"])).is_err()
        );
        assert!(
            parse_setup_auth_args(argv(&["setup-auth", "--provider", "codex", "--reconnect"]))
                .is_err()
        );
    }

    #[test]
    fn env_or_ignores_blank_values() {
        // SAFETY: test-local variable name nobody else reads.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("AEGIS_SETUP_AUTH_TEST_BLANK", "   ");
        }
        assert_eq!(
            env_or("AEGIS_SETUP_AUTH_TEST_BLANK", "fallback"),
            "fallback"
        );
        assert_eq!(
            env_or("AEGIS_SETUP_AUTH_TEST_UNSET", "fallback"),
            "fallback"
        );
    }

    #[tokio::test]
    async fn api_key_path_persists_openai_compat_and_disables_subscriptions() {
        let dir = tempfile::tempdir().unwrap();
        let files = DataFiles {
            identity_key: dir.path().join("id.key").display().to_string(),
            credential_db: dir.path().join("cred.db").display().to_string(),
            memory_db: dir.path().join("nested/memory.db").display().to_string(),
        };
        let memory = files.open_memory().unwrap();
        persist_settings(&memory, ChatProviderSelection::Grok.runtime_settings())
            .await
            .unwrap();
        let stored = memory.load_all_settings().await.unwrap();
        assert_eq!(stored["llm_provider"], "xai_subscription");
        assert_eq!(stored["llm_model"], "");
        assert_eq!(stored["xai_subscription_enabled"], true);
        assert_eq!(stored["codex_enabled"], false);
        assert_eq!(stored["paid_api_fallback_enabled"], false);
        assert_eq!(stored["external_executor_enabled"], true);
        assert_eq!(stored["external_executor_primary"], "auto");
        assert_eq!(stored["external_executor_fallback"], "none");
        persist_settings(
            &memory,
            vec![("external_executor_enabled", serde_json::json!(false))],
        )
        .await
        .unwrap();
        persist_settings(&memory, ChatProviderSelection::Codex.runtime_settings())
            .await
            .unwrap();
        assert_eq!(
            memory.load_all_settings().await.unwrap()["external_executor_enabled"],
            false
        );
        assert_eq!(
            files.stored_chat_provider().await.unwrap().as_deref(),
            Some("codex_oauth")
        );
    }

    #[tokio::test]
    async fn failed_probe_preserves_previous_provider_settings() {
        let dir = tempfile::tempdir().unwrap();
        let files = DataFiles {
            identity_key: dir.path().join("id.key").display().to_string(),
            credential_db: dir.path().join("cred.db").display().to_string(),
            memory_db: dir.path().join("memory.db").display().to_string(),
        };
        let memory = files.open_memory().unwrap();
        persist_settings(&memory, ChatProviderSelection::Grok.runtime_settings())
            .await
            .unwrap();
        let before = memory.load_all_settings().await.unwrap();
        let result = verified_settings(
            &memory,
            ChatProviderSelection::Codex.runtime_settings(),
            async { anyhow::bail!("synthetic failed provider") },
        )
        .await
        .unwrap();
        assert_eq!(result, SETUP_AUTH_NOT_READY);
        assert_eq!(memory.load_all_settings().await.unwrap(), before);
        let result = verified_settings(
            &memory,
            ChatProviderSelection::Codex.runtime_settings(),
            async { Ok(()) },
        )
        .await
        .unwrap();
        assert_eq!(result, 0);
        assert_eq!(
            memory.load_all_settings().await.unwrap()["llm_provider"],
            "codex_oauth"
        );
    }

    #[tokio::test]
    async fn status_does_not_create_a_memory_db() {
        let dir = tempfile::tempdir().unwrap();
        let files = DataFiles {
            identity_key: dir.path().join("id.key").display().to_string(),
            credential_db: dir.path().join("cred.db").display().to_string(),
            memory_db: dir.path().join("memory.db").display().to_string(),
        };
        assert_eq!(files.stored_chat_provider().await.unwrap(), None);
        assert!(!Path::new(&files.memory_db).exists());
    }

    #[test]
    fn persist_settings_rejects_unknown_keys() {
        let registry = RuntimeSettings::new();
        assert!(
            registry
                .set("not_a_real_setting", serde_json::Value::Bool(true))
                .is_err()
        );
        for (key, value) in ChatProviderSelection::Codex.runtime_settings() {
            registry.set(key, value).unwrap();
        }
    }

    #[tokio::test]
    async fn settings_persistence_setup_batch_rolls_back_after_later_write_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.db");
        let memory = MemoryHandle::spawn(&path).unwrap();
        memory
            .store_setting("llm_provider", serde_json::json!("openai_compat"))
            .await
            .unwrap();
        let before = memory.load_all_settings().await.unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_model BEFORE INSERT ON user_settings WHEN NEW.key = 'llm_model' BEGIN SELECT RAISE(ABORT, 'synthetic setup failure'); END;").unwrap();
        assert!(
            persist_settings(&memory, ChatProviderSelection::Codex.runtime_settings())
                .await
                .is_err()
        );
        assert_eq!(memory.load_all_settings().await.unwrap(), before);
        conn.execute_batch("DROP TRIGGER reject_model").unwrap();
        assert!(
            persist_settings(
                &memory,
                vec![
                    ("llm_provider", serde_json::json!("codex_oauth")),
                    ("not_a_real_setting", serde_json::json!(true))
                ]
            )
            .await
            .is_err()
        );
        assert_eq!(memory.load_all_settings().await.unwrap(), before);
        persist_settings(&memory, ChatProviderSelection::Codex.runtime_settings())
            .await
            .unwrap();
        let committed = memory.load_all_settings().await.unwrap();
        memory.shutdown().await.unwrap();
        let reopened = MemoryHandle::spawn(&path).unwrap();
        assert_eq!(reopened.load_all_settings().await.unwrap(), committed);
        reopened.shutdown().await.unwrap();
    }
}
