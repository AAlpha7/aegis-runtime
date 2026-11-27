use super::*;

async fn verify_chat_choice(
    state: &AdminState,
    provider: &str,
    model: &str,
    effort: Option<String>,
) -> Result<(), (StatusCode, String)> {
    let model = if model.is_empty() {
        crate::model_catalog::provider_default_model(&state.settings, provider)
    } else {
        model.to_owned()
    };
    crate::call_validation::verify(
        state,
        &crate::call_validation::Candidate::Chat {
            provider: provider.into(),
            model,
            effort,
        },
    )
    .await
}

pub(super) async fn webapp_vnext_jev_status(
    State(state): State<Arc<AdminState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    extract_web_console_auth(&state, &headers).await?;
    let store = vnext_xai_credential_store(&state)?;
    crate::jev_config::resolve(Some(&store))
        .map(|config| Json(config.status()))
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Jev configuration is unavailable.".into(),
            )
        })
}

pub(super) async fn webapp_vnext_set_jev(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let principal = extract_web_console_auth(&state, &parts.headers).await?;
    require_web_console_csrf_for_principal(&parts.headers, &principal)?;
    let bytes = axum::body::to_bytes(body, 8 * 1024)
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "jev_invalid_request".into()))?;
    let update: crate::jev_config::JevUpdate = serde_json::from_slice(&bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, "jev_invalid_request".into()))?;
    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let key = {
        let store = vnext_xai_credential_store(&state)?;
        crate::jev_config::prepare_key(&store, &update)
            .map_err(|_| (StatusCode::BAD_REQUEST, "jev_invalid_key_or_consent".into()))?
    };
    if let Some(key) = &key {
        crate::call_validation::verify(
            &state,
            &crate::call_validation::Candidate::Jev { key: key.clone() },
        )
        .await?;
    }
    let store = vnext_xai_credential_store(&state)?;
    crate::jev_config::save_after_probe(&store, update, key.as_deref()).map_err(|err| {
        let code = err.to_string();
        if matches!(
            code.as_str(),
            "jev_consent_required"
                | "jev_remove_requires_off"
                | "jev_invalid_key"
                | "jev_key_required"
        ) {
            (StatusCode::BAD_REQUEST, code)
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, "jev_save_failed".into())
        }
    })?;
    crate::jev_config::resolve(Some(&store))
        .map(|config| Json(config.status()))
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "jev_reload_failed".into()))
}

pub(super) fn vnext_xai_credential_store(
    state: &AdminState,
) -> Result<CredentialStore, (StatusCode, String)> {
    let (Some(db_path), Some(aes_key)) = (&state.credential_db_path, &state.credential_aes_key)
    else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Credential store is not configured.".to_string(),
        ));
    };
    CredentialStore::open(
        crate::encrypted_sqlite::open(db_path).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to open credential store: {err}"),
            )
        })?,
        *aes_key,
    )
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to initialize credential store: {err}"),
        )
    })
}

pub(super) fn vnext_grok_status_value(state: &AdminState) -> serde_json::Value {
    let enabled = state
        .settings
        .get("xai_subscription_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let selected_provider = selected_webapp_chat_provider_key(state);
    let active_for_chat = enabled
        && matches!(
            selected_provider.as_str(),
            "xai_subscription" | "grok_subscription"
        );
    let model = state
        .settings
        .get("xai_subscription_model")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "grok-4.7".to_string());
    let foreground_only = state
        .settings
        .get("xai_subscription_allow_background_tasks")
        .and_then(|value| value.as_bool())
        .map(|allow_background| !allow_background)
        .unwrap_or(true);
    let daily_request_limit = state
        .settings
        .get("xai_subscription_daily_request_limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(100);

    let locale = state.settings.display_language();
    let mut connected = false;
    let mut pending = false;
    let mut account_label: Option<String> = None;
    let mut owner_message = if enabled {
        crate::i18n::t("console.provider.grok.needs_signin", &locale).to_string()
    } else {
        crate::i18n::t("console.provider.grok.available", &locale).to_string()
    };
    let mut owner_next_step =
        Some(crate::i18n::t("console.provider.grok.next.connect", &locale).to_string());

    if let Ok(store) = vnext_xai_credential_store(state) {
        if let Ok(Some(credential)) =
            crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
        {
            connected = true;
            account_label = credential
                .email
                .or(credential.display_name)
                .or(credential.account_id);
            owner_message = if active_for_chat {
                crate::i18n::t("console.provider.grok.active", &locale).to_string()
            } else {
                crate::i18n::t("console.provider.grok.connected", &locale).to_string()
            };
            owner_next_step = if active_for_chat {
                None
            } else {
                Some(crate::i18n::t("console.provider.grok.next.use", &locale).to_string())
            };
        }
        pending = crate::llm::XaiSubscriptionAuthManager::load_pending_device_authorization(&store)
            .ok()
            .flatten()
            .is_some();
        if pending && !connected {
            owner_message = crate::i18n::t("console.provider.grok.pending", &locale).to_string();
            owner_next_step =
                Some(crate::i18n::t("console.provider.grok.next.enter_code", &locale).to_string());
        }
    }

    serde_json::json!({
        "schema_version": 1,
        "provider": "xai_subscription",
        "enabled": enabled,
        "selected_provider": selected_provider,
        "active_for_chat": active_for_chat,
        "connected": connected,
        "pending_device_code": pending,
        "model": model,
        "foreground_only": foreground_only,
        "daily_request_limit": daily_request_limit,
        "account_label": account_label,
        "owner_message": owner_message,
        "owner_next_step": owner_next_step,
    })
}

pub(super) use crate::settings::ChatProviderSelection as WebappChatProviderSelection;

pub(super) async fn select_webapp_chat_provider(
    state: &AdminState,
    selection: WebappChatProviderSelection,
) -> anyhow::Result<()> {
    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    verify_chat_choice(state, selection.provider_key(), "", None)
        .await
        .map_err(|(_, message)| anyhow::anyhow!(message))?;
    let updates = selection
        .runtime_settings()
        .into_iter()
        .filter(|(key, _)| {
            *key != "external_executor_enabled"
                || state.settings.get_if_stored(key) != Some(serde_json::Value::Bool(false))
        })
        .collect();
    persist_runtime_settings_locked(state, updates, _write)
        .await
        .with_context(|| format!("select {} for foreground chat", selection.owner_name()))
}

async fn disable_webapp_chat_provider(
    state: &AdminState,
    selection: WebappChatProviderSelection,
) -> anyhow::Result<()> {
    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let key = match selection {
        WebappChatProviderSelection::Codex => "codex_enabled",
        WebappChatProviderSelection::Grok => "xai_subscription_enabled",
    };
    let mut updates = vec![(key, serde_json::json!(false))];
    let current = current_llm_provider(state);
    if current == selection.provider_key()
        || (selection == WebappChatProviderSelection::Grok && current == "grok_subscription")
    {
        updates.push((
            "llm_provider",
            serde_json::json!(fallback_webapp_chat_provider_after_disable(
                state, selection
            )),
        ));
    }
    persist_runtime_settings_locked(state, updates, _write).await
}

pub(super) fn selected_webapp_chat_provider_key(state: &AdminState) -> String {
    state
        .settings
        .get_if_stored("llm_provider")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "openai_compat".to_string())
}

pub(super) fn fallback_webapp_chat_provider_after_disable(
    state: &AdminState,
    disabled: WebappChatProviderSelection,
) -> &'static str {
    match disabled {
        WebappChatProviderSelection::Codex => {
            if state
                .settings
                .get("xai_subscription_enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                "xai_subscription"
            } else {
                "openai_compat"
            }
        }
        WebappChatProviderSelection::Grok => {
            if state
                .settings
                .get("codex_enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                "codex_oauth"
            } else {
                "openai_compat"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodexCliLoginStatus {
    pub binary_available: bool,
    pub logged_in: bool,
    pub auth_kind: Option<String>,
    pub detail: String,
}

struct CodexCliDeviceLoginSession {
    verification_uri: String,
    user_code: String,
    expires_at: Instant,
    expires_at_unix_ms: u64,
    log_path: String,
    child: tokio::process::Child,
}

static CODEX_CLI_DEVICE_LOGIN_SESSIONS: OnceLock<
    parking_lot::Mutex<HashMap<String, CodexCliDeviceLoginSession>>,
> = OnceLock::new();

fn codex_cli_device_login_sessions()
-> &'static parking_lot::Mutex<HashMap<String, CodexCliDeviceLoginSession>> {
    CODEX_CLI_DEVICE_LOGIN_SESSIONS.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

fn unix_ms_after(duration: Duration) -> u64 {
    SystemTime::now()
        .checked_add(duration)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn strip_ansi_sgr(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

pub(super) fn parse_codex_cli_device_auth_output(raw: &str) -> Option<(String, String)> {
    let clean = strip_ansi_sgr(raw);
    let verification_uri = clean
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                !(ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '.' | '-' | '_'))
            })
        })
        .find(|token| token.starts_with("https://auth.openai.com/codex/device"))?
        .to_string();
    let user_code = clean
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-')))
        .find(|token| {
            token.contains('-')
                && (7..=24).contains(&token.len())
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
        })?
        .to_string();
    Some((verification_uri, user_code))
}

fn codex_cli_device_log_path(principal: &WebConsolePrincipal) -> String {
    let mut hasher = DefaultHasher::new();
    principal.log_subject().hash(&mut hasher);
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!(
        "/tmp/aegis-runtime-codex-device-{:016x}-{unix_ms}.log",
        hasher.finish()
    )
}

fn codex_cli_open_device_log(path: &str) -> io::Result<(std::fs::File, std::fs::File)> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let stderr = file.try_clone()?;
    Ok((file, stderr))
}

fn codex_cli_cleanup_device_sessions_for_principal(principal_key: &str) {
    let now = Instant::now();
    let mut sessions = codex_cli_device_login_sessions().lock();
    sessions.retain(|key, session| {
        let expired = now >= session.expires_at;
        let finished = session.child.try_wait().ok().flatten().is_some();
        let remove = key == principal_key || expired || finished;
        if remove {
            let _ = session.child.start_kill();
            let _ = std::fs::remove_file(&session.log_path);
        }
        !remove
    });
}

fn codex_setting_string(state: &AdminState, key: &str, fallback: &str) -> String {
    state
        .settings
        .get(key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn normalize_codex_model_for_cli(model: &str) -> String {
    model
        .trim()
        .strip_prefix("openai-codex/")
        .unwrap_or_else(|| model.trim())
        .to_string()
}

fn normalize_codex_reasoning_effort(value: &str) -> Option<String> {
    let trimmed = value.trim();
    matches!(trimmed, "low" | "medium" | "high" | "xhigh").then(|| trimmed.to_string())
}

fn fallback_codex_model_options() -> Vec<serde_json::Value> {
    [
        ("gpt-5.5", "GPT-5.5", "medium"),
        ("gpt-5.4", "gpt-5.4", "medium"),
        ("gpt-5.4-mini", "GPT-5.4-Mini", "medium"),
    ]
    .into_iter()
    .map(|(slug, display_name, default_reasoning_level)| {
        serde_json::json!({
            "slug": slug,
            "display_name": display_name,
            "default_reasoning_level": default_reasoning_level,
            "supported_reasoning_levels": ["low", "medium", "high", "xhigh"],
        })
    })
    .collect()
}

pub(super) fn parse_codex_model_options(raw: &str) -> Option<Vec<serde_json::Value>> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let models = parsed.get("models")?.as_array()?;
    let mut options = Vec::new();
    for model in models {
        let slug = model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let visibility = model
            .get("visibility")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("list");
        if visibility == "hidden" {
            continue;
        }
        let supported_reasoning_levels = model
            .get("supported_reasoning_levels")
            .and_then(serde_json::Value::as_array)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| level.get("effort").and_then(serde_json::Value::as_str))
                    .filter_map(normalize_codex_reasoning_effort)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        options.push(serde_json::json!({
            "slug": slug,
            "display_name": model
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(slug),
            "description": model
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            "default_reasoning_level": model
                .get("default_reasoning_level")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("medium"),
            "supported_reasoning_levels": supported_reasoning_levels,
        }));
    }
    (!options.is_empty()).then_some(options)
}

pub(super) fn parse_codex_app_server_model_options(
    value: &serde_json::Value,
) -> Option<Vec<serde_json::Value>> {
    let models = value.pointer("/result/data")?.as_array()?;
    let mut options = Vec::new();
    for model in models {
        if model
            .get("hidden")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let slug = model
            .get("model")
            .or_else(|| model.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let supported_reasoning_levels = model
            .get("supportedReasoningEfforts")
            .and_then(serde_json::Value::as_array)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| {
                        level
                            .get("reasoningEffort")
                            .and_then(serde_json::Value::as_str)
                    })
                    .filter_map(normalize_codex_reasoning_effort)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let default_reasoning_level = model
            .get("defaultReasoningEffort")
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_codex_reasoning_effort)
            .unwrap_or_else(|| {
                supported_reasoning_levels
                    .iter()
                    .find(|level| level.as_str() == "medium")
                    .cloned()
                    .or_else(|| supported_reasoning_levels.first().cloned())
                    .unwrap_or_else(|| "medium".to_string())
            });
        options.push(serde_json::json!({
            "slug": slug,
            "display_name": model
                .get("displayName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(slug),
            "description": model
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            "default_reasoning_level": default_reasoning_level,
            "supported_reasoning_levels": supported_reasoning_levels,
            "is_default": model
                .get("isDefault")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }));
    }
    (!options.is_empty()).then_some(options)
}

async fn write_codex_app_server_admin_message(
    stdin: &mut tokio::process::ChildStdin,
    message: serde_json::Value,
) -> anyhow::Result<()> {
    let line = serde_json::to_string(&message).context("serialize Codex app-server message")?;
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write Codex app-server message")?;
    stdin
        .write_all(b"\n")
        .await
        .context("write Codex app-server newline")?;
    stdin.flush().await.context("flush Codex app-server stdin")
}

async fn codex_app_server_model_options(
    state: &AdminState,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let binary = codex_setting_string(state, "codex_cli_binary", "/usr/bin/codex");
    let codex_home = codex_setting_string(
        state,
        "codex_cli_home",
        &crate::settings::default_codex_home(),
    );
    let mut command = tokio::process::Command::new(&binary);
    command
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .env("CODEX_HOME", &codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(parent) = std::path::Path::new(&codex_home).parent() {
        command.env("HOME", parent);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Unable to start Codex app-server at {binary}"))?;
    let mut stdin = child
        .stdin
        .take()
        .context("Codex app-server stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("Codex app-server stdout unavailable")?;
    let mut lines = BufReader::new(stdout).lines();

    let run = async {
        write_codex_app_server_admin_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "aegis-runtime-admin",
                        "title": "Aegis Runtime Admin",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "requestAttestation": false,
                        "optOutNotificationMethods": [],
                    }
                }
            }),
        )
        .await?;
        loop {
            let Some(line) = lines.next_line().await? else {
                anyhow::bail!("Codex app-server exited before initialize completed");
            };
            let message: serde_json::Value = serde_json::from_str(&line)?;
            if message.get("id").and_then(serde_json::Value::as_u64) == Some(1) {
                if let Some(error) = message.get("error") {
                    anyhow::bail!("Codex app-server initialize failed: {error}");
                }
                break;
            }
        }
        write_codex_app_server_admin_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialized",
            }),
        )
        .await?;
        write_codex_app_server_admin_message(
            &mut stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "model/list",
                "params": {
                    "limit": 100,
                    "includeHidden": false,
                }
            }),
        )
        .await?;
        loop {
            let Some(line) = lines.next_line().await? else {
                anyhow::bail!("Codex app-server exited before model/list completed");
            };
            let message: serde_json::Value = serde_json::from_str(&line)?;
            if message.get("id").and_then(serde_json::Value::as_u64) == Some(2) {
                if let Some(error) = message.get("error") {
                    anyhow::bail!("Codex app-server model/list failed: {error}");
                }
                return parse_codex_app_server_model_options(&message)
                    .context("Codex app-server model/list returned no visible models");
            }
        }
    };

    let result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .context("Codex app-server model/list timed out");
    let _ = stdin.shutdown().await;
    let _ = child.start_kill();
    result?
}

pub(crate) async fn codex_cli_model_options(
    state: &AdminState,
) -> (Vec<serde_json::Value>, &'static str) {
    if let Ok(options) = codex_app_server_model_options(state).await {
        return (options, "codex_app_server_model_list");
    }

    let binary = codex_setting_string(state, "codex_cli_binary", "/usr/bin/codex");
    let codex_home = codex_setting_string(
        state,
        "codex_cli_home",
        &crate::settings::default_codex_home(),
    );
    let mut command = tokio::process::Command::new(&binary);
    command
        .arg("debug")
        .arg("models")
        .arg("--bundled")
        .env("CODEX_HOME", &codex_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = std::path::Path::new(&codex_home).parent() {
        command.env("HOME", parent);
    }
    let output = tokio::time::timeout(Duration::from_secs(4), command.output()).await;
    if let Ok(Ok(output)) = output
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(options) = parse_codex_model_options(&stdout) {
            return (options, "codex_cli_bundled_catalog");
        }
    }
    (fallback_codex_model_options(), "builtin_fallback")
}

pub(super) fn parse_codex_cli_login_status(
    success: bool,
    stdout: &str,
    stderr: &str,
) -> CodexCliLoginStatus {
    let combined = format!("{}\n{}", stdout.trim(), stderr.trim());
    let logged_in_chatgpt = combined.contains("Logged in using ChatGPT");
    let logged_in_api_key = combined.contains("Logged in using API key");
    CodexCliLoginStatus {
        binary_available: true,
        // API-key login is deliberately not accepted for this provider:
        // this integration is subscription-first and must not silently spend
        // OpenAI Platform API credits.
        logged_in: success && logged_in_chatgpt,
        auth_kind: if logged_in_chatgpt {
            Some("ChatGPT".to_string())
        } else if logged_in_api_key {
            Some("API key".to_string())
        } else {
            None
        },
        detail: if combined.trim().is_empty() {
            "Codex CLI produced no login status output.".to_string()
        } else {
            combined.trim().to_string()
        },
    }
}

pub(crate) async fn codex_cli_login_status(state: &AdminState) -> CodexCliLoginStatus {
    let binary = codex_setting_string(state, "codex_cli_binary", "/usr/bin/codex");
    let codex_home = codex_setting_string(
        state,
        "codex_cli_home",
        &crate::settings::default_codex_home(),
    );
    codex_cli_login_status_at(&binary, &codex_home).await
}

/// `codex login status` for an explicit binary + `CODEX_HOME`. Shared by the
/// Settings routes and the pre-boot `setup-auth` CLI so both judge "logged in"
/// by the same rule (ChatGPT subscription login only; API-key login is not
/// accepted for this provider).
pub async fn codex_cli_login_status_at(binary: &str, codex_home: &str) -> CodexCliLoginStatus {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("login")
        .arg("status")
        .env("CODEX_HOME", codex_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = std::path::Path::new(codex_home).parent() {
        command.env("HOME", parent);
    }

    let output = match tokio::time::timeout(Duration::from_secs(5), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return CodexCliLoginStatus {
                binary_available: false,
                logged_in: false,
                auth_kind: None,
                detail: format!("Codex CLI could not be started: {err}"),
            };
        }
        Err(_) => {
            return CodexCliLoginStatus {
                binary_available: true,
                logged_in: false,
                auth_kind: None,
                detail: "Codex CLI login status timed out.".to_string(),
            };
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut parsed = parse_codex_cli_login_status(output.status.success(), &stdout, &stderr);
    if parsed.detail == "Codex CLI produced no login status output." {
        parsed.detail = format!("Codex CLI exited with {}", output.status);
    }
    parsed
}

async fn vnext_codex_status_value(state: &AdminState) -> serde_json::Value {
    let enabled = state
        .settings
        .get("codex_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let selected_provider = selected_webapp_chat_provider_key(state);
    let active_for_chat = enabled && selected_provider == "codex_oauth";
    let model =
        normalize_codex_model_for_cli(&codex_setting_string(state, "codex_model", "gpt-5.5"));
    let reasoning_effort = codex_setting_string(state, "codex_reasoning_effort", "medium");
    let reasoning_effort =
        normalize_codex_reasoning_effort(&reasoning_effort).unwrap_or_else(|| "medium".into());
    let daily_request_limit = state
        .settings
        .get("codex_daily_request_limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(100);
    let cli_binary = codex_setting_string(state, "codex_cli_binary", "/usr/bin/codex");
    let cli_home = codex_setting_string(
        state,
        "codex_cli_home",
        &crate::settings::default_codex_home(),
    );
    let cli_workdir = codex_setting_string(state, "codex_cli_workdir", "/tmp");
    let login = codex_cli_login_status(state).await;
    let (model_options, model_options_source) = codex_cli_model_options(state).await;
    let locale = state.settings.display_language();
    let owner_message = if active_for_chat {
        crate::i18n::t("console.provider.codex.active", &locale).to_string()
    } else if login.logged_in {
        crate::i18n::t("console.provider.codex.connected", &locale).to_string()
    } else if login.auth_kind.as_deref() == Some("API key") {
        crate::i18n::t("console.provider.codex.api_key", &locale).to_string()
    } else if login.binary_available {
        crate::i18n::t("console.provider.codex.not_logged_in", &locale).to_string()
    } else {
        crate::i18n::t("console.provider.codex.not_installed", &locale).to_string()
    };
    let owner_next_step = if login.logged_in {
        if active_for_chat {
            None
        } else {
            Some(crate::i18n::t("console.provider.codex.next.use", &locale).to_string())
        }
    } else {
        Some(crate::i18n::t("console.provider.codex.next.signin", &locale).to_string())
    };

    serde_json::json!({
        "schema_version": 1,
        "provider": "codex_oauth",
        "enabled": enabled,
        "selected_provider": selected_provider,
        "active_for_chat": active_for_chat,
        "connected": login.logged_in,
        "binary_available": login.binary_available,
        "auth_kind": login.auth_kind,
        "model": model,
        "reasoning_effort": reasoning_effort,
        "model_options": model_options,
        "model_options_source": model_options_source,
        "tool_calls_supported": true,
        "tool_routing_policy": "",
        "foreground_only": true,
        "daily_request_limit": daily_request_limit,
        "cli_binary": cli_binary,
        "cli_home": cli_home,
        "cli_workdir": cli_workdir,
        "owner_message": owner_message,
        "owner_next_step": owner_next_step,
        "diagnostic": login.detail,
    })
}

pub(super) async fn webapp_vnext_codex_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    Ok(Json(vnext_codex_status_value(&state).await))
}

async fn validate_vnext_codex_request_body(body_bytes: &[u8]) -> Result<(), (StatusCode, String)> {
    validate_vnext_cost_saving_request_body(body_bytes).await
}

pub(super) async fn webapp_vnext_codex_start_device_code(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex connection request body".to_string(),
        )
    })?;
    validate_vnext_codex_request_body(&body_bytes).await?;

    let login = codex_cli_login_status(&state).await;
    if login.logged_in {
        return Err((
            StatusCode::CONFLICT,
            "Codex CLI is already logged in for Aegis.".to_string(),
        ));
    }

    let principal_key = principal.log_subject();
    codex_cli_cleanup_device_sessions_for_principal(&principal_key);

    let binary = codex_setting_string(&state, "codex_cli_binary", "/usr/bin/codex");
    let codex_home = codex_setting_string(
        &state,
        "codex_cli_home",
        &crate::settings::default_codex_home(),
    );
    let log_path = codex_cli_device_log_path(&principal);
    let (stdout_file, stderr_file) = codex_cli_open_device_log(&log_path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to prepare Codex device login output: {err}"),
        )
    })?;

    let mut command = tokio::process::Command::new(&binary);
    command
        .arg("login")
        .arg("--device-auth")
        .env("CODEX_HOME", &codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    if let Some(parent) = std::path::Path::new(&codex_home).parent() {
        command.env("HOME", parent);
    }
    let child = command.spawn().map_err(|err| {
        let _ = std::fs::remove_file(&log_path);
        (
            StatusCode::BAD_GATEWAY,
            format!("Unable to start Codex device-code sign-in: {err}"),
        )
    })?;

    let started = Instant::now();
    let mut parsed: Option<(String, String)> = None;
    while started.elapsed() < Duration::from_secs(8) {
        if let Ok(raw) = std::fs::read_to_string(&log_path)
            && let Some(device) = parse_codex_cli_device_auth_output(&raw)
        {
            parsed = Some(device);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let Some((verification_uri, user_code)) = parsed else {
        let mut child = child;
        let _ = child.start_kill();
        let output = std::fs::read_to_string(&log_path).unwrap_or_default();
        let _ = std::fs::remove_file(&log_path);
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "Codex CLI did not produce a device code in time: {}",
                strip_ansi_sgr(&output).trim()
            ),
        ));
    };

    let expires_in = Duration::from_secs(15 * 60);
    let expires_at_unix_ms = unix_ms_after(expires_in);
    let _ = std::fs::remove_file(&log_path);
    let session = CodexCliDeviceLoginSession {
        verification_uri: verification_uri.clone(),
        user_code: user_code.clone(),
        expires_at: Instant::now() + expires_in,
        expires_at_unix_ms,
        log_path,
        child,
    };
    codex_cli_device_login_sessions()
        .lock()
        .insert(principal_key, session);

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "pending",
        "verification_uri": verification_uri,
        "verification_uri_complete": verification_uri,
        "user_code": user_code,
        "expires_at_ms": expires_at_unix_ms,
        "interval_secs": 5,
        "message": "Open OpenAI, enter the code, then return here.",
    })))
}

pub(super) async fn webapp_vnext_codex_poll_device_code(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex poll request body".to_string(),
        )
    })?;
    validate_vnext_codex_request_body(&body_bytes).await?;

    let login = codex_cli_login_status(&state).await;
    if login.logged_in {
        codex_cli_cleanup_device_sessions_for_principal(&principal.log_subject());
        return Ok(Json(serde_json::json!({
            "schema_version": 1,
            "status": "connected",
            "message": "Codex CLI is connected with ChatGPT for the Aegis service account.",
        })));
    }

    let principal_key = principal.log_subject();
    let poll_state = {
        let mut sessions = codex_cli_device_login_sessions().lock();
        let Some(session) = sessions.get_mut(&principal_key) else {
            return Err((
                StatusCode::CONFLICT,
                "No Codex device-code sign-in is pending.".to_string(),
            ));
        };
        if Instant::now() >= session.expires_at {
            let _ = session.child.start_kill();
            let log_path = session.log_path.clone();
            sessions.remove(&principal_key);
            let _ = std::fs::remove_file(log_path);
            return Err((
                StatusCode::CONFLICT,
                "Codex device code expired. Start a new code.".to_string(),
            ));
        }
        if let Ok(Some(status)) = session.child.try_wait() {
            let log_path = session.log_path.clone();
            sessions.remove(&principal_key);
            let _ = std::fs::remove_file(log_path);
            Err(status.to_string())
        } else {
            Ok(serde_json::json!({
                "schema_version": 1,
                "status": "pending",
                "verification_uri": session.verification_uri.clone(),
                "verification_uri_complete": session.verification_uri.clone(),
                "user_code": session.user_code.clone(),
                "expires_at_ms": session.expires_at_unix_ms,
                "interval_secs": 5,
                "message": "Still waiting for OpenAI browser confirmation.",
            }))
        }
    };

    match poll_state {
        Ok(value) => Ok(Json(value)),
        Err(status) => {
            let login = codex_cli_login_status(&state).await;
            if login.logged_in {
                return Ok(Json(serde_json::json!({
                    "schema_version": 1,
                    "status": "connected",
                    "message": "Codex CLI is connected with ChatGPT for the Aegis service account.",
                })));
            }
            Err((
                StatusCode::BAD_GATEWAY,
                format!("Codex sign-in stopped before completion: {status}"),
            ))
        }
    }
}

pub(super) async fn webapp_vnext_codex_use_for_chat(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex routing request body".to_string(),
        )
    })?;
    validate_vnext_codex_request_body(&body_bytes).await?;

    let login = codex_cli_login_status(&state).await;
    if !login.logged_in {
        return Err((
            StatusCode::CONFLICT,
            format!("Codex CLI is not logged in for Aegis: {}", login.detail),
        ));
    }

    select_webapp_chat_provider(&state, WebappChatProviderSelection::Codex)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to select Codex for chat: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "connected",
        "message": "Foreground Aegis chat will use the official Codex app-server. Background tasks and paid API fallback remain off.",
    })))
}

pub(super) async fn webapp_vnext_codex_set_model(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 4 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex model request body".to_string(),
        )
    })?;
    validate_vnext_codex_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex model request JSON".to_string(),
        )
    })?;
    let model = value
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(normalize_codex_model_for_cli)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Codex model is required.".to_string(),
            )
        })?;
    let reasoning_effort = value
        .get("reasoning_effort")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_codex_reasoning_effort)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Codex reasoning effort must be low, medium, high, or xhigh.".to_string(),
            )
        })?;
    let (model_options, _) = codex_cli_model_options(&state).await;
    let Some(option) = model_options.iter().find(|option| {
        option
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|slug| slug == model)
    }) else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Codex model is not available in this account catalog: {model}"),
        ));
    };
    let supported = option
        .get("supported_reasoning_levels")
        .and_then(serde_json::Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|level| level == reasoning_effort)
        })
        .unwrap_or(false);
    if !supported {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Reasoning effort {reasoning_effort} is not supported for {model}."),
        ));
    }

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    verify_chat_choice(
        &state,
        "codex_oauth",
        &model,
        Some(reasoning_effort.clone()),
    )
    .await?;
    persist_runtime_settings_locked(
        &state,
        vec![
            ("codex_model", serde_json::Value::String(model.clone())),
            (
                "codex_reasoning_effort",
                serde_json::Value::String(reasoning_effort.clone()),
            ),
        ],
        _write,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save Codex configuration: {err}"),
        )
    })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "connected",
        "message": format!("Codex chat model set to {model} with {reasoning_effort} reasoning."),
    })))
}

pub(super) async fn webapp_vnext_codex_stop_using_for_chat(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Codex routing request body".to_string(),
        )
    })?;
    validate_vnext_codex_request_body(&body_bytes).await?;

    disable_webapp_chat_provider(&state, WebappChatProviderSelection::Codex)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to disable chat routing: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "connected",
        "message": "Aegis stopped routing foreground chat to Codex. The official Codex CLI login is still managed by CODEX_HOME.",
    })))
}

pub(super) async fn webapp_vnext_grok_status(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let principal = extract_web_console_auth(&state, req.headers()).await?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    Ok(Json(vnext_grok_status_value(&state)))
}

async fn validate_vnext_grok_request_body(body_bytes: &[u8]) -> Result<(), (StatusCode, String)> {
    validate_vnext_cost_saving_request_body(body_bytes).await
}

pub(super) async fn webapp_vnext_grok_start_device_code(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Grok connection request body".to_string(),
        )
    })?;
    validate_vnext_grok_request_body(&body_bytes).await?;

    let manager = crate::llm::XaiSubscriptionAuthManager::new(Default::default());
    let authorization = manager.start_device_code().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Unable to start Grok device-code sign-in: {}", err.message),
        )
    })?;
    let store = vnext_xai_credential_store(&state)?;
    crate::llm::XaiSubscriptionAuthManager::store_pending_device_authorization(
        &store,
        &authorization,
    )
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save Grok device-code state: {}", err.message),
        )
    })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "pending",
        "verification_uri": authorization.verification_uri,
        "verification_uri_complete": authorization.verification_uri_complete,
        "user_code": authorization.user_code,
        "expires_at_ms": authorization.expires_at_ms,
        "interval_secs": authorization.interval_secs,
        "message": "Open xAI in your browser, enter the code, then return here.",
    })))
}

pub(super) async fn webapp_vnext_grok_poll_device_code(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Grok poll request body".to_string(),
        )
    })?;
    validate_vnext_grok_request_body(&body_bytes).await?;

    let store = vnext_xai_credential_store(&state)?;
    let authorization =
        crate::llm::XaiSubscriptionAuthManager::load_pending_device_authorization(&store)
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to load Grok device-code state: {}", err.message),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::CONFLICT,
                    "No Grok device-code sign-in is pending.".to_string(),
                )
            })?;
    let manager = crate::llm::XaiSubscriptionAuthManager::new(Default::default());
    let poll = manager
        .poll_device_code_once(&authorization)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to poll Grok device-code sign-in: {}", err.message),
            )
        })?;

    match poll {
        crate::llm::XaiDeviceCodePoll::Pending => Ok(Json(serde_json::json!({
            "schema_version": 1,
            "status": "pending",
            "interval_secs": authorization.interval_secs,
            "message": "Still waiting for xAI browser confirmation.",
        }))),
        crate::llm::XaiDeviceCodePoll::SlowDown { interval_secs } => {
            // Persist the slower cadence so every later poll (and every
            // client reading `interval_secs`) keeps honoring xAI's request
            // instead of resetting to the original interval.
            let slowed = crate::llm::XaiDeviceAuthorization {
                interval_secs,
                ..authorization
            };
            crate::llm::XaiSubscriptionAuthManager::store_pending_device_authorization(
                &store, &slowed,
            )
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to save Grok device-code state: {}", err.message),
                )
            })?;
            Ok(Json(serde_json::json!({
                "schema_version": 1,
                "status": "pending",
                "interval_secs": interval_secs,
                "message": "xAI asked Aegis to poll more slowly.",
            })))
        }
        crate::llm::XaiDeviceCodePoll::Authorized(credential) => {
            crate::llm::XaiSubscriptionAuthManager::store_credential(&store, &credential).map_err(
                |err| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Unable to save Grok credential: {}", err.message),
                    )
                },
            )?;
            let _ =
                crate::llm::XaiSubscriptionAuthManager::delete_pending_device_authorization(&store);
            Ok(Json(serde_json::json!({
                "schema_version": 1,
                "status": "connected",
                "account_label": credential.email.or(credential.display_name).or(credential.account_id),
                "message": "Grok subscription is connected.",
            })))
        }
    }
}

pub(super) async fn webapp_vnext_grok_use_for_chat(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Grok routing request body".to_string(),
        )
    })?;
    validate_vnext_grok_request_body(&body_bytes).await?;

    let store = vnext_xai_credential_store(&state)?;
    let connected = crate::llm::XaiSubscriptionAuthManager::load_credential(&store)
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to load Grok credential: {}", err.message),
            )
        })?
        .is_some();
    if !connected {
        return Err((
            StatusCode::CONFLICT,
            "Connect Grok before using it for Aegis chat.".to_string(),
        ));
    }

    select_webapp_chat_provider(&state, WebappChatProviderSelection::Grok)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to select Grok for chat: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "connected",
        "message": "Foreground Aegis chat will use Grok. Background tasks and paid API fallback remain off.",
    })))
}

pub(super) async fn webapp_vnext_grok_stop_using_for_chat(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Grok routing request body".to_string(),
        )
    })?;
    validate_vnext_grok_request_body(&body_bytes).await?;

    disable_webapp_chat_provider(&state, WebappChatProviderSelection::Grok)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to disable chat routing: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "connected",
        "message": "Aegis stopped routing foreground chat to Grok. The encrypted Grok credential is still stored.",
    })))
}

pub(super) async fn webapp_vnext_grok_revoke(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    let _chat_id = console_chat_id_for_principal(&state, &principal)?;
    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid Grok revoke request body".to_string(),
        )
    })?;
    validate_vnext_grok_request_body(&body_bytes).await?;

    let store = vnext_xai_credential_store(&state)?;
    let deleted_credential = crate::llm::XaiSubscriptionAuthManager::delete_credential(&store)
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to delete Grok credential: {}", err.message),
            )
        })?;
    let _ = crate::llm::XaiSubscriptionAuthManager::delete_pending_device_authorization(&store);
    disable_webapp_chat_provider(&state, WebappChatProviderSelection::Grok)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Credential removed, but chat routing could not be saved: {err}"),
            )
        })?;
    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "status": "revoked",
        "deleted_credential": deleted_credential,
        "message": "Grok subscription credential was removed from Aegis and chat routing was disabled.",
    })))
}

async fn validate_vnext_cost_saving_request_body(
    body_bytes: &[u8],
) -> Result<(), (StatusCode, String)> {
    if body_bytes.is_empty() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_slice(body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid cost-saving request JSON".to_string(),
        )
    })?;
    let schema_version = match value.get("schema_version") {
        Some(raw) => raw
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid assistant request schema version".to_string(),
                )
            })?,
        None => 1,
    };
    validate_context_schema_version(Some(schema_version))?;
    validate_context_idempotency_key(
        value
            .get("idempotency_key")
            .and_then(serde_json::Value::as_str),
    )?;
    Ok(())
}

/// POST /webapp/vnext/settings/cost-saving/pause - Owner-triggered pause for costly background autonomy.
pub(super) async fn webapp_vnext_pause_cost_saving(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid cost-saving request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;

    pause_background_autonomy_for_cost_saving(&state)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to pause cost-saving controls: {err}"),
            )
        })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/cost-saving/resume - Owner-triggered resume for paused background autonomy.
pub(super) async fn webapp_vnext_resume_cost_saving(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid cost-saving request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;

    resume_background_autonomy_after_cost_saving(&state)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to resume cost-saving controls: {err}"),
            )
        })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// Normalizes a requested display-language value to a supported locale.
/// Mirrors the webapp `normalizeLocale`: anything starting with `en` is English,
/// anything starting with `zh` is Chinese; other values are rejected so an
/// unsupported language cannot be persisted (no silent fallback).
pub(super) fn normalize_display_language(raw: &str) -> Option<&'static str> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("en") {
        Some("en")
    } else if lower.starts_with("zh") {
        Some("zh")
    } else {
        None
    }
}

/// POST /webapp/vnext/settings/display-language - Owner-triggered display-language change.
///
/// Channel B (system fixed strings) always renders in the `display_language`
/// setting, so persisting it here makes server-authored strings follow the same
/// language the webapp chrome uses. Mirrors the cost-saving handlers: console
/// auth + CSRF, then persist and return the refreshed settings projection.
pub(super) async fn webapp_vnext_set_display_language(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid display-language request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid display-language request JSON".to_string(),
        )
    })?;
    let requested = value
        .get("display_language")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "display_language is required.".to_string(),
            )
        })?;
    let locale = normalize_display_language(requested).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "display_language must be 'zh' or 'en'.".to_string(),
        )
    })?;

    persist_runtime_setting(
        &state,
        "display_language",
        serde_json::Value::String(locale.to_string()),
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to set display language: {err}"),
        )
    })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// Normalizes a requested `llm_provider` value to the supported set the webapp
/// switcher offers. Only these three are accepted from the webapp; any other
/// value (including registered-but-not-owner-switchable backends like
/// `openai_api`/`local`) is rejected so the endpoint cannot persist a provider
/// the owner UI does not control. The router reads `llm_provider` live, so the
/// persisted value takes effect on the next LLM call with no restart.
pub(super) fn normalize_llm_provider(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "openrouter" => Some("openrouter"),
        "codex_oauth" => Some("codex_oauth"),
        "xai_subscription" => Some("xai_subscription"),
        _ => None,
    }
}

/// POST /webapp/vnext/settings/llm-provider — Owner switches the active LLM
/// provider backend. Mirrors `webapp_vnext_set_display_language`: console auth +
/// CSRF + 2KB body cap + cost-saving body validation. The provider takes effect
/// LIVE (no restart) because the router config is rebuilt from `self.settings`
/// per LLM call. Switching provider also resets `llm_model` to the empty
/// string so the new provider's default is used (an explicit id from the
/// previous provider must not stick). Same-provider writes leave `llm_model`
/// unchanged. Returns the refreshed settings projection.
pub(super) async fn webapp_vnext_set_llm_provider(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid llm-provider request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid llm-provider request JSON".to_string(),
        )
    })?;
    let requested = value
        .get("llm_provider")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "llm_provider is required.".to_string(),
            )
        })?;
    let provider = normalize_llm_provider(requested).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "llm_provider must be 'openrouter', 'codex_oauth', or 'xai_subscription'.".to_string(),
        )
    })?;

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let previous_provider = current_llm_provider(&state);
    let candidate_model = if let Some(model) = value.get("llm_model") {
        model
            .as_str()
            .and_then(normalize_llm_model)
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid model".into()))?
    } else if previous_provider == provider {
        state
            .settings
            .get("llm_model")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        String::new()
    };
    verify_chat_choice(&state, provider, &candidate_model, None).await?;
    let mut updates = vec![(
        "llm_provider",
        serde_json::Value::String(provider.to_string()),
    )];
    // Selecting a subscription activates its adapter. Executor opt-out is separate.
    if let Some(key) = match provider {
        "codex_oauth" => Some("codex_enabled"),
        "xai_subscription" => Some("xai_subscription_enabled"),
        _ => None,
    } {
        updates.push((key, serde_json::json!(true)));
    }
    if matches!(provider, "codex_oauth" | "xai_subscription")
        && state
            .settings
            .get_if_stored("external_executor_enabled")
            .is_none()
    {
        updates.push(("external_executor_enabled", serde_json::json!(true)));
    }
    // Empty llm_model means "use this provider's default". An explicit model
    // id from the previous provider is not valid for the new catalog.
    if previous_provider != provider || value.get("llm_model").is_some() {
        updates.push(("llm_model", serde_json::Value::String(candidate_model)));
    }
    persist_runtime_settings_locked(&state, updates, _write)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to save LLM configuration: {err}"),
            )
        })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// Upper bound on a webapp-supplied `llm_model` id. OpenRouter-style slugs are
/// short (`vendor/model-variant`); a 200-char cap is generous while preventing
/// an oversized value from being persisted into runtime settings.
pub(crate) const LLM_MODEL_MAX_LEN: usize = 200;

/// Normalizes a requested `llm_model` value. Empty means "provider default"
/// (codex → `codex_model`, xAI → `xai_subscription_model`). Non-empty ids are
/// trimmed and length-capped; catalog membership is checked by the setters.
/// The router reads `llm_model` live via `active_model()`, so the persisted
/// value takes effect on the next LLM call with no restart.
pub(super) fn normalize_llm_model(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > LLM_MODEL_MAX_LEN {
        return None;
    }
    Some(trimmed.to_string())
}

fn current_llm_provider(state: &AdminState) -> String {
    state
        .settings
        .get("llm_provider")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "codex_oauth".to_string())
}

/// POST /webapp/vnext/settings/llm-model — Owner sets the active LLM model id.
/// Mirrors `webapp_vnext_set_llm_provider`: console auth + CSRF + 2KB body cap +
/// cost-saving body validation. The model takes effect LIVE (no restart) because
/// `Orchestrator::active_model()` reads `llm_model` from the shared `settings`
/// Arc per LLM call — `persist_runtime_setting` updates that same Arc in addition
/// to persisting to the DB. Returns the refreshed settings projection.
pub(super) async fn webapp_vnext_set_llm_model(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid llm-model request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid llm-model request JSON".to_string(),
        )
    })?;
    let requested = value
        .get("llm_model")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "llm_model is required.".to_string(),
            )
        })?;
    let model = normalize_llm_model(requested).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("llm_model must be at most {LLM_MODEL_MAX_LEN} characters."),
        )
    })?;

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let catalog = crate::model_catalog::load_model_catalog(&state).await;
    let provider = current_llm_provider(&state);
    crate::model_catalog::validate_llm_model_setting(&catalog, &provider, &model)
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;

    verify_chat_choice(&state, &provider, &model, None).await?;
    persist_runtime_setting_locked(
        &state,
        "llm_model",
        serde_json::Value::String(model),
        _write,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to set LLM model: {err}"),
        )
    })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/web-search — Owner sets the web search provider
/// and/or model. Default provider is the Grok subscription (`grok-4.7` on
/// `api.x.ai`). OpenRouter is explicit and keeps native `:online` search.
/// Does not write `xai_subscription_model` or `llm_provider`.
pub(super) async fn webapp_vnext_set_web_search(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid web-search request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid web-search request JSON".to_string(),
        )
    })?;
    let requested_provider = value
        .get("web_search_provider")
        .and_then(serde_json::Value::as_str);
    let requested_model = value
        .get("web_search_model")
        .and_then(serde_json::Value::as_str);
    if requested_provider.is_none() && requested_model.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "web_search_provider or web_search_model is required.".to_string(),
        ));
    }

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let current_provider = state
        .settings
        .get("web_search_provider")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| crate::web_search::WEB_SEARCH_PROVIDER_SUBSCRIPTION.to_string());
    let current_model = state
        .settings
        .get("web_search_model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| crate::web_search::WEB_SEARCH_SUBSCRIPTION_MODEL.to_string());

    let provider = if let Some(raw) = requested_provider {
        crate::web_search::normalize_web_search_provider(raw).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "web_search_provider must be auto, xai_subscription, codex_oauth or openrouter."
                    .to_string(),
            )
        })?
    } else {
        crate::web_search::normalize_web_search_provider(&current_provider)
            .unwrap_or(crate::web_search::WEB_SEARCH_PROVIDER_SUBSCRIPTION)
    };
    let model_input = requested_model.unwrap_or(current_model.as_str());
    let model = crate::web_search::normalize_saved_web_search_model(provider, model_input)
        .map_err(|msg| (StatusCode::BAD_REQUEST, msg.to_string()))?;

    let chat = current_llm_provider(&state);
    let actual_provider = crate::web_search::selected_provider(provider, &chat);
    crate::call_validation::verify(
        &state,
        &crate::call_validation::Candidate::Search {
            provider: actual_provider.into(),
            model: model.clone(),
        },
    )
    .await?;
    persist_runtime_settings_locked(
        &state,
        vec![
            (
                "web_search_provider",
                serde_json::Value::String(provider.to_string()),
            ),
            ("web_search_model", serde_json::Value::String(model)),
        ],
        _write,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to save web search configuration: {err}"),
        )
    })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// GET /webapp/vnext/model-catalog — provider-sourced model lists + current
/// / fallback settings. Console principal auth matches `/webapp/vnext/settings`.
pub(super) async fn webapp_vnext_model_catalog(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, _body) = req.into_parts();
    let _principal = extract_web_console_auth(&state, &parts.headers).await?;

    let catalog = crate::model_catalog::load_model_catalog(&state).await;
    let provider = current_llm_provider(&state);
    let model = state
        .settings
        .get("llm_model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let effective_model =
        crate::model_catalog::resolve_effective_llm_model(&state.settings, &model, &provider);
    let fallback_model = state
        .settings
        .get("fallback_model")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let fallback_provider = catalog
        .provider_for_model(&fallback_model)
        .map(str::to_owned);

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "providers": catalog.providers(),
        "recommended_api_model": catalog.recommended_api_model(),
        "current": {
            "provider": provider,
            "model": model,
            "effective_model": effective_model,
        },
        "fallback": {
            "provider": fallback_provider,
            "model": fallback_model,
        },
    })))
}

/// POST /webapp/vnext/settings/fallback-model — Owner sets the backup model id.
pub(super) async fn webapp_vnext_set_fallback_model(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid fallback-model request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid fallback-model request JSON".to_string(),
        )
    })?;
    let requested = value
        .get("model")
        .or_else(|| value.get("fallback_model"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "model is required.".to_string()))?;
    let model = normalize_llm_model(requested).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("fallback_model must be at most {LLM_MODEL_MAX_LEN} characters."),
        )
    })?;

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    if !model.is_empty() {
        let catalog = crate::model_catalog::load_model_catalog(&state).await;
        crate::model_catalog::validate_fallback_model_setting(&catalog, &model)
            .map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;
        let provider = catalog.provider_for_model(&model).ok_or_else(|| (StatusCode::UNPROCESSABLE_ENTITY, "Cannot verify the selected fallback provider; previous configuration is unchanged.".into()))?;
        verify_chat_choice(&state, provider, &model, None).await?;
    }
    let authorized = !model.is_empty();
    persist_runtime_settings_locked(
        &state,
        vec![
            ("fallback_model", serde_json::Value::String(model)),
            ("fallback_model_authorized", serde_json::json!(authorized)),
        ],
        _write,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to save backup activation".into(),
        )
    })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/settings/paid-api-fallback — Owner toggles whether paid
/// OpenAI-compatible fallback is permitted when the subscription provider is
/// down. Mirrors the display-language endpoint. Read live by the router per
/// call, so the toggle takes effect with no restart. Returns the refreshed
/// settings projection.
pub(super) async fn webapp_vnext_set_paid_api_fallback(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;

    let body_bytes = axum::body::to_bytes(body, 2 * 1024).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid paid-api-fallback request body".to_string(),
        )
    })?;
    validate_vnext_cost_saving_request_body(&body_bytes).await?;
    let value: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid paid-api-fallback request JSON".to_string(),
        )
    })?;
    let enabled = value
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "enabled (boolean) is required.".to_string(),
            )
        })?;

    let _write = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    if enabled {
        // Provider-unavailable fallback sends the original request model to
        // the configured API client. A different backup must not prove it works.
        let model = state
            .orchestrator
            .as_ref()
            .map(|o| o.active_model())
            .or_else(|| {
                state
                    .settings
                    .get("llm_model")
                    .and_then(|v| v.as_str().map(str::to_owned))
            })
            .unwrap_or_default();
        verify_chat_choice(&state, "openrouter", &model, None).await?;
    }
    persist_runtime_setting_locked(
        &state,
        "paid_api_fallback_enabled",
        serde_json::Value::Bool(enabled),
        _write,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to set paid-api fallback: {err}"),
        )
    })?;

    let chat_id = console_chat_id_for_principal(&state, &principal)?;
    let projection =
        build_unified_inbox_projection(&state, chat_id, &state.settings.display_language()).await;
    let settings = vnext_settings_projection(&principal, &projection, Some(&state.settings));

    Ok(Json(serde_json::to_value(settings).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to serialize vNext settings: {err}"),
        )
    })?))
}

/// POST /webapp/vnext/debug-snapshots — Owner-triggered, short-lived UX diagnostic.
pub(super) async fn webapp_vnext_create_debug_snapshot(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<VNextDebugSnapshotResponse>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    require_webapp_debug_snapshots_enabled()?;

    let body_bytes = axum::body::to_bytes(body, WEBAPP_DEBUG_SNAPSHOT_MAX_BYTES + 1)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to read debug snapshot body: {err}"),
            )
        })?;
    if body_bytes.len() > WEBAPP_DEBUG_SNAPSHOT_MAX_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Debug snapshot is too large".into(),
        ));
    }
    let request: VNextDebugSnapshotRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid debug snapshot JSON".to_string(),
        )
    })?;
    validate_context_schema_version(request.schema_version)?;

    let (owner, principal_type) = principal_debug_owner(&principal);
    let surface = sanitize_debug_snapshot_surface(&request.surface);
    let path = sanitize_debug_snapshot_path(&request.path);
    let mut payload = sanitize_debug_snapshot_payload(request.payload);
    if !payload.is_object() {
        payload = serde_json::json!({ "value": payload });
    }
    if let Some(idempotency_key) = request
        .idempotency_key
        .as_deref()
        .filter(|key| !key.is_empty())
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "idempotency_key_hash".into(),
            serde_json::Value::String(short_hash_for_debug(idempotency_key)),
        );
    }

    let payload_json = serde_json::to_string(&payload).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("Unable to serialize debug snapshot: {err}"),
        )
    })?;
    if payload_json.len() > WEBAPP_DEBUG_SNAPSHOT_MAX_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Debug snapshot is too large after sanitization".into(),
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now_unix = chrono::Utc::now().timestamp();
    let expires_at_unix = now_unix + WEBAPP_DEBUG_SNAPSHOT_TTL_SECS;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let response_id = id.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = crate::encrypted_sqlite::open(&db_path)?;
        init_webapp_debug_snapshot_schema(&conn)?;
        cleanup_expired_webapp_debug_snapshots(&conn, now_unix)?;
        conn.execute(
            "INSERT INTO webapp_debug_snapshots (
                id, owner, principal_type, surface, path, payload_json,
                payload_bytes, created_at_unix, expires_at_unix
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id,
                owner,
                principal_type,
                surface,
                path,
                payload_json,
                i64::try_from(payload_json.len()).unwrap_or(i64::MAX),
                now_unix,
                expires_at_unix,
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug snapshot task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to store debug snapshot: {err}"),
        )
    })?;

    Ok(Json(VNextDebugSnapshotResponse {
        schema_version: 1,
        id: response_id,
        expires_at_unix,
    }))
}

/// POST /webapp/vnext/debug-leases — Owner-granted, short-lived diagnostic lease.
pub(super) async fn webapp_vnext_create_debug_lease(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<VNextDebugLeaseResponse>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    require_webapp_debug_snapshots_enabled()?;

    let body_bytes = axum::body::to_bytes(body, 16_384).await.map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to read debug lease body: {err}"),
        )
    })?;
    let request: VNextDebugLeaseRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid debug lease JSON".to_string(),
        )
    })?;
    validate_context_schema_version(request.schema_version)?;

    let (owner, principal_type) = principal_debug_owner(&principal);
    let scope = sanitize_debug_lease_scope(&request.scope);
    let permissions = sanitize_debug_lease_permissions(request.permissions);
    let item_id = sanitize_debug_lease_item_id(request.item_id);
    let reason = sanitize_debug_lease_reason(request.reason);
    let duration_minutes = normalize_debug_lease_duration_minutes(request.duration_minutes);
    // Bearer-token-backed scope: mail (read-only inbox/message/accounts
    // debugging — the mail GET endpoints accept the lease via
    // extract_mail_read_auth; mail mutations never do). Other scopes remain
    // UI-session leases without a token.
    let debug_grant_token = (scope == WEBAPP_DEBUG_SCOPE_MAIL).then(mint_webapp_debug_grant_token);
    let token_hash = debug_grant_token
        .as_deref()
        .map(webapp_debug_grant_token_hash);
    let idempotency_hash = request
        .idempotency_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(short_hash_for_debug);
    let id = format!("lease-{}", uuid::Uuid::new_v4());
    let now_unix = chrono::Utc::now().timestamp();
    let expires_at_unix = now_unix + i64::from(duration_minutes) * 60;
    let permissions_json = serde_json::to_string(&permissions).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("Unable to serialize debug lease permissions: {err}"),
        )
    })?;
    let db_path = webapp_debug_snapshot_db_path(&state);
    let response_id = id.clone();

    let lease = tokio::task::spawn_blocking(move || -> anyhow::Result<WebappDebugLeaseSummary> {
        let conn = crate::encrypted_sqlite::open(&db_path)?;
        init_webapp_debug_lease_schema(&conn)?;
        cleanup_expired_webapp_debug_leases(&conn, now_unix)?;
        conn.execute(
            "INSERT INTO webapp_debug_leases (
                id, owner, principal_type, scope, permissions_json, token_hash, item_id,
                reason, created_at_unix, expires_at_unix, revoked_at_unix
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            rusqlite::params![
                response_id,
                owner,
                principal_type,
                scope,
                permissions_json,
                token_hash,
                item_id,
                match idempotency_hash {
                    Some(hash) => format!("{reason} idempotency={hash}"),
                    None => reason,
                },
                now_unix,
                expires_at_unix,
            ],
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, owner, principal_type, scope, permissions_json, item_id,
                    reason, created_at_unix, expires_at_unix, revoked_at_unix
             FROM webapp_debug_leases
             WHERE id = ?1",
        )?;
        Ok(stmt.query_row([id], read_debug_lease_summary)?)
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug lease task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to store debug lease: {err}"),
        )
    })?;

    Ok(Json(VNextDebugLeaseResponse {
        schema_version: 1,
        owner_message: format!(
            "Debug access for {} is available for {} minutes. It is scoped, audited, revocable, and expires automatically.",
            humanize_machine_label(&lease.scope),
            duration_minutes
        ),
        lease,
        debug_grant_token,
    }))
}

/// GET /webapp/vnext/debug-leases — Active leases for this signed-in owner.
pub(super) async fn webapp_vnext_debug_leases(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers().clone();
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_webapp_debug_snapshots_enabled()?;
    let (owner, _) = principal_debug_owner(&principal);
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let leases =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<WebappDebugLeaseSummary>> {
            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_lease_schema(&conn)?;
            cleanup_expired_webapp_debug_leases(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, scope, permissions_json, item_id,
                        reason, created_at_unix, expires_at_unix, revoked_at_unix
                 FROM webapp_debug_leases
                 WHERE owner = ?1 AND revoked_at_unix IS NULL AND expires_at_unix > ?2
                 ORDER BY created_at_unix DESC
                 LIMIT 20",
            )?;
            let rows =
                stmt.query_map(rusqlite::params![owner, now_unix], read_debug_lease_summary)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Debug lease list task failed: {err}"),
            )
        })?
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list debug leases: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "leases": leases,
    })))
}

/// GET /webapp/vnext/debug-jobs — Recent owner-authorized backend debug jobs.
pub(super) async fn webapp_vnext_debug_jobs(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers().clone();
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_webapp_debug_snapshots_enabled()?;
    let (owner, _) = principal_debug_owner(&principal);
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let jobs =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<WebappDebugJobSummary>> {
            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_job_schema(&conn)?;
            cleanup_expired_webapp_debug_jobs(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, kind, status, summary, log_text, exit_code,
                        lease_id, created_at_unix, started_at_unix, completed_at_unix, expires_at_unix
                 FROM webapp_debug_jobs
                 WHERE owner = ?1
                 ORDER BY created_at_unix DESC
                 LIMIT 10",
            )?;
            let rows = stmt.query_map([owner], read_debug_job_summary)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Debug job list task failed: {err}"),
            )
        })?
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to list debug jobs: {err}"),
            )
        })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "jobs": jobs,
    })))
}

/// GET /webapp/vnext/debug-jobs/:job_id — One owner-authorized backend debug job.
pub(super) async fn webapp_vnext_debug_job(
    State(state): State<Arc<AdminState>>,
    Path(job_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers().clone();
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_webapp_debug_snapshots_enabled()?;
    let (owner, _) = principal_debug_owner(&principal);
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let job = tokio::task::spawn_blocking(
        move || -> anyhow::Result<Option<WebappDebugJobSummary>> {
            let conn = crate::encrypted_sqlite::open(&db_path)?;
            init_webapp_debug_job_schema(&conn)?;
            cleanup_expired_webapp_debug_jobs(&conn, now_unix)?;
            let mut stmt = conn.prepare(
                "SELECT id, owner, principal_type, kind, status, summary, log_text, exit_code,
                        lease_id, created_at_unix, started_at_unix, completed_at_unix, expires_at_unix
                 FROM webapp_debug_jobs
                 WHERE owner = ?1 AND id = ?2
                 LIMIT 1",
            )?;
            match stmt.query_row(rusqlite::params![owner, job_id], read_debug_job_summary) {
                Ok(job) => Ok(Some(job)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(err) => Err(err.into()),
            }
        },
    )
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug job get task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to get debug job: {err}"),
        )
    })?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "Debug job not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "job": job,
    })))
}

/// POST /webapp/vnext/debug-jobs — Owner-authorized backend debug job runner.
pub(super) async fn webapp_vnext_create_debug_job(
    State(state): State<Arc<AdminState>>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    require_webapp_debug_snapshots_enabled()?;

    let body_bytes = axum::body::to_bytes(body, 16_384).await.map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to read debug job body: {err}"),
        )
    })?;
    let request: VNextDebugJobRequest = serde_json::from_slice(&body_bytes).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid debug job JSON".to_string(),
        )
    })?;
    validate_context_schema_version(request.schema_version)?;
    let kind = sanitize_debug_job_kind(&request.kind).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Unsupported debug job kind".to_string(),
        )
    })?;

    let (owner, principal_type) = principal_debug_owner(&principal);
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let job = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let owner = owner.clone();
        let principal_type = principal_type.clone();
        let kind = kind.clone();
        move || -> anyhow::Result<WebappDebugJobSummary> {
            let mut conn = crate::encrypted_sqlite::open(&db_path)?;
            Ok(insert_webapp_debug_job(
                &mut conn,
                &owner,
                &principal_type,
                &kind,
                now_unix,
            )?)
        }
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug job task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to store debug job: {err}"),
        )
    })?;

    spawn_webapp_debug_job_runner(
        WebappDebugJobRuntimeConfig {
            db_path,
            kind,
            base_url: webapp_debug_job_base_url(&state),
            repo_root: webapp_debug_job_repo_root(),
        },
        job.clone(),
    );

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "job": job,
        "owner_message": "Backend debug job started. Progress and its redacted log are available on this job id.",
    })))
}

/// DELETE /webapp/vnext/debug-leases/:lease_id — Revoke an active owner lease.
pub(super) async fn webapp_vnext_revoke_debug_lease(
    State(state): State<Arc<AdminState>>,
    Path(lease_id): Path<String>,
    req: Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let headers = req.headers().clone();
    let principal = extract_web_console_auth(&state, &headers).await?;
    require_web_console_csrf_for_principal(&headers, &principal)?;
    require_webapp_debug_snapshots_enabled()?;
    let (owner, _) = principal_debug_owner(&principal);
    let db_path = webapp_debug_snapshot_db_path(&state);
    let now_unix = chrono::Utc::now().timestamp();
    let revoked = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let conn = crate::encrypted_sqlite::open(&db_path)?;
        init_webapp_debug_lease_schema(&conn)?;
        Ok(conn.execute(
            "UPDATE webapp_debug_leases
             SET revoked_at_unix = ?1
             WHERE id = ?2 AND owner = ?3 AND revoked_at_unix IS NULL",
            rusqlite::params![now_unix, lease_id, owner],
        )?)
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Debug lease revoke task failed: {err}"),
        )
    })?
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to revoke debug lease: {err}"),
        )
    })?;

    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "revoked": revoked,
    })))
}
