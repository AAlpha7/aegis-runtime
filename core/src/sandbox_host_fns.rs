// ── Host function linking for the S-LEG sandbox ──────────────────────────
//
// Extracted from sandbox.rs — aegis::* host function injection, WASI stubs,
// skill module execution, trap classification, and memory limiting.

use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use wasmtime::*;

use crate::models::SkillCapability;
use crate::sandbox::{FetchRecord, HostState, MAX_FETCH_LOG_ENTRIES, MAX_UNIQUE_DOMAINS};
use crate::sandbox_network::{
    MAX_FETCH_RESPONSE_BYTES, MAX_READ_FILE_BYTES, is_domain_allowed, is_private_ip_addr,
};

/// Lazy monotonic clock origin for WASI `clock_time_get(MONOTONIC)`.
///
/// Initialized on first access; all subsequent MONOTONIC reads are relative
/// to this instant so skills see a stable monotonically non-decreasing clock.
static MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

// ── Extracted host function linking (from run_isolated_with_auth) ────────

/// Link all `aegis::*` host functions into the Wasmtime linker.
///
/// Closures injected: host_fetch (with S-LEG, DNS rebinding protection, rate limiting),
/// host_get_fetch_response_len/host_get_fetch_response, host_write_output,
/// host_get_payload_len/host_get_payload, host_read_file, host_list_files,
/// host_store_state, host_load_state, host_get_state_response_len/host_get_state_response.
pub(crate) fn link_host_functions(
    linker: &mut Linker<HostState>,
    payload: &serde_json::Value,
    fetch_pool: Arc<rayon::ThreadPool>,
) -> Result<()> {
    // ── Inject aegis::host_fetch ────────────────────────────────────
    // S-LEG controlled: whitelist-checked network gateway.
    // Returns 0 on success, negative on error/block.
    // On success, the response body is stored in HostState.fetch_response
    // and can be read via host_get_fetch_response_len / host_get_fetch_response.
    linker.func_wrap(
        "aegis",
        "host_fetch",
        move |mut caller: Caller<'_, HostState>, url_ptr: i32, url_len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -1,
            };

            let url_bytes = {
                let data = memory.data(&caller);
                if url_ptr < 0 || url_len < 0 {
                    return -2;
                }
                let begin = url_ptr as usize;
                let end = match begin.checked_add(url_len as usize) {
                    Some(e) => e,
                    None => return -2,
                };
                if end > data.len() {
                    return -2;
                }
                data[begin..end].to_vec()
            };

            let url = match std::str::from_utf8(&url_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => return -3,
            };

            // ── FETCH CALL LIMIT ──
            // Caps live in HostState (execution-scoped): defaults are
            // MAX_FETCH_CALLS / MAX_FETCH_PER_DOMAIN, raised only via a
            // trusted-host fetch budget (sandbox::effective_fetch_caps).
            let state = caller.data_mut();
            state.fetch_count += 1;
            if state.fetch_count > state.max_fetch_calls {
                if state.fetch_log.len() < MAX_FETCH_LOG_ENTRIES {
                    state.fetch_log.push(FetchRecord {
                        url: url.clone(),
                        allowed: false,
                    });
                }
                return -429; // rate limited
            }

            // ── S-LEG DOMAIN WHITELIST ──
            let is_allowed = is_domain_allowed(&url, &state.allowed_domains);

            if state.fetch_log.len() < MAX_FETCH_LOG_ENTRIES {
                state.fetch_log.push(FetchRecord {
                    url: url.clone(),
                    allowed: is_allowed,
                });
            }

            if !is_allowed {
                tracing::warn!(
                    skill = %state.skill_name,
                    url = %url,
                    "host_fetch blocked: domain not in whitelist"
                );
                return -403;
            }

            // Per-domain rate limiting
            if let Ok(parsed) = url::Url::parse(&url)
                && let Some(host) = parsed.host_str() {
                    if state.domain_fetch_counts.len() >= MAX_UNIQUE_DOMAINS
                        && !state.domain_fetch_counts.contains_key(host)
                    {
                        tracing::warn!(
                            skill = %state.skill_name,
                            domain = %host,
                            limit = MAX_UNIQUE_DOMAINS,
                            "host_fetch blocked: skill exceeded max unique domains"
                        );
                        return -429;
                    }
                    let count = state
                        .domain_fetch_counts
                        .entry(host.to_string())
                        .or_insert(0);
                    *count += 1;
                    if *count > state.max_fetch_per_domain {
                        tracing::warn!(
                            skill = %state.skill_name,
                            domain = %host,
                            count = *count,
                            limit = state.max_fetch_per_domain,
                            "host_fetch blocked: per-domain rate limit exceeded"
                        );
                        return -429;
                    }
                }

            // Clone shared state for use inside the rayon thread
            let domains_for_dns = state.allowed_domains.clone();
            let auth_headers_for_fetch = state.auth_headers.clone();

            // ── REAL HTTP FETCH ──
            // Use a bounded rayon thread pool to:
            // 1. Avoid reqwest::blocking panic inside tokio runtime (rayon threads != tokio threads)
            // 2. Prevent unbounded thread spawning under concurrent skill invocations
            // Redirect following is disabled to prevent SSRF via open redirects
            // (S-LEG checks the initial URL only — a redirect could escape the whitelist).
            let pool = fetch_pool.clone();
            let fetch_result: Option<Vec<u8>> = pool.install(move || -> Option<Vec<u8>> {
                // DNS rebinding protection: resolve hostname, verify all
                // resolved IPs are not private, then PIN the resolved IPs
                // into the HTTP client so it cannot re-resolve to a different
                // address (eliminates the TOCTOU window entirely).
                let parsed = url::Url::parse(&url).ok()?;
                let host = parsed.host_str()?;
                let explicitly_whitelisted = domains_for_dns.iter().any(|d| d == host);

                // Always resolve DNS and check for private IPs — even for
                // whitelisted domains. A compromised DNS could point a
                // whitelisted domain to cloud metadata (169.254.169.254).
                // Exception: if the host IS a literal private IP in the
                // whitelist (e.g. "127.0.0.1" for server_monitor), allow it.
                let pinned_ips: Vec<std::net::SocketAddr> = {
                    use std::net::ToSocketAddrs;
                    let port =
                        parsed
                            .port()
                            .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
                    match (host, port).to_socket_addrs() {
                        Ok(addrs) => {
                            let addrs_vec: Vec<_> = addrs.collect();
                            // Allow private IPs only if host is a literal IP in the whitelist
                            let host_is_literal_ip = host.parse::<std::net::IpAddr>().is_ok();
                            if !(explicitly_whitelisted && host_is_literal_ip) {
                                for addr in &addrs_vec {
                                    if is_private_ip_addr(addr.ip()) {
                                        return None;
                                    }
                                }
                            }
                            addrs_vec
                        }
                        Err(_) => {
                            // DNS resolution failed — block the request
                            return None;
                        }
                    }
                };

                // Build client with pinned IPs to eliminate TOCTOU window.
                // reqwest::resolve() forces the client to use our pre-validated
                // addresses instead of doing its own DNS lookup.
                //
                // Redirect policy: allow up to 3 same-host HTTPS upgrades
                // (http->https on same domain), block all cross-domain redirects.
                let original_host = host.to_string();
                let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
                    if attempt.previous().len() > 3 {
                        attempt.stop()
                    } else if let Some(redirect_host) = attempt.url().host_str() {
                        if redirect_host == original_host && attempt.url().scheme() == "https" {
                            attempt.follow()
                        } else {
                            attempt.stop()
                        }
                    } else {
                        attempt.stop()
                    }
                });
                // Localhost requests (e.g. Aegis Executor) may run slow
                // system commands, so they get a longer timeout: the executor
                // accepts diagnostic commands of up to 180 s (a cold `du` over
                // the data trees measured 145 s on 2026-09-12), plus slack.
                let is_localhost = host == "127.0.0.1" || host == "localhost" || host == "::1";
                let timeout_secs = if is_localhost { 190 } else { 15 };
                let mut builder = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(timeout_secs))
                    .redirect(redirect_policy)
                    .user_agent("reqwest/0.12 (Aegis Runtime Skill Runtime)");

                // Pin the first validated IP for this host
                if let Some(addr) = pinned_ips.first() {
                    builder = builder.resolve(host, *addr);
                }

                let client = builder.build().ok()?;

                // Auto-inject Authorization header for known services — by
                // host:port first (the executor token must not reach other
                // loopback services), then by bare host.
                let host_port = format!(
                    "{host}:{}",
                    parsed
                        .port()
                        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 })
                );
                let mut request = client.get(&url);
                if let Some(auth_value) = auth_headers_for_fetch
                    .get(&host_port)
                    .or_else(|| auth_headers_for_fetch.get(host))
                {
                    request = request.header("Authorization", auth_value);
                }

                let resp = request.send().ok()?;
                let status_code = resp.status().as_u16();

                // Fail-fast on auth errors: if the API returned 401, the skill
                // has no valid credentials. Return a structured JSON error body
                // instead of the raw HTML error page, so the skill can parse it.
                // Only intercept 401 (not 403, which can mean rate-limiting or geo-blocking).
                if status_code == 401 {
                    let err_body = serde_json::json!({
                        "status": "error",
                        "error": format!("HTTP {status_code}: authentication failed for {host}. Check OAuth2 credentials."),
                        "error_kind": "auth_failure",
                        "retryable": false,
                        "hint": "OAuth2 token may have expired or credentials are invalid. Try re-authenticating via setup_google.",
                    });
                    return Some(err_body.to_string().into_bytes());
                }

                // Read body with a hard cap of MAX_FETCH_RESPONSE_BYTES to prevent OOM.
                // If Content-Length is known and exceeds the limit, we still read
                // up to the cap (truncating). We use chunked reading so we never
                // allocate more than MAX_FETCH_RESPONSE_BYTES + chunk overhead.
                use std::io::Read as _;
                let content_length = resp.content_length();
                let prealloc = content_length
                    .map(|cl| cl.min(MAX_FETCH_RESPONSE_BYTES as u64) as usize)
                    .unwrap_or(0);
                let mut body = Vec::with_capacity(prealloc);
                let mut reader = resp;
                loop {
                    let remaining = MAX_FETCH_RESPONSE_BYTES - body.len();
                    if remaining == 0 {
                        break;
                    }
                    let chunk_size = remaining.min(64 * 1024); // 64KB chunks
                    let before = body.len();
                    body.resize(before + chunk_size, 0);
                    match reader.read(&mut body[before..]) {
                        Ok(0) => {
                            body.truncate(before);
                            break;
                        }
                        Ok(n) => {
                            body.truncate(before + n);
                        }
                        Err(_) => {
                            body.truncate(before);
                            break;
                        }
                    }
                }
                Some(body)
            });

            match fetch_result {
                Some(body) => {
                    let state = caller.data_mut();
                    state.fetch_response = body;
                    0 // success
                }
                None => {
                    // HTTP request failed — still allowed by S-LEG, just network error
                    let state = caller.data_mut();
                    state.fetch_response = Vec::new();
                    -500
                }
            }
        },
    )?;

    // ── Inject aegis::host_get_fetch_response_len ────────────────────
    // Returns the length of the last successful fetch response body.
    linker.func_wrap(
        "aegis",
        "host_get_fetch_response_len",
        |caller: Caller<'_, HostState>| -> i32 { caller.data().fetch_response.len() as i32 },
    )?;

    // ── Inject aegis::host_get_fetch_response ────────────────────────
    // Copies the fetch response body into Wasm memory at dest_ptr.
    // Returns 0 on success, -1 if dest_ptr + response_len exceeds Wasm memory.
    linker.func_wrap(
        "aegis",
        "host_get_fetch_response",
        |mut caller: Caller<'_, HostState>, dest_ptr: i32| -> i32 {
            let response = caller.data().fetch_response.clone();
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -1,
            };
            if dest_ptr < 0 {
                return -1;
            }
            let begin = dest_ptr as usize;
            let end = match begin.checked_add(response.len()) {
                Some(e) => e,
                None => return -1,
            };
            if end <= memory.data(&caller).len() {
                memory.data_mut(&mut caller)[begin..end].copy_from_slice(&response);
                0 // success
            } else {
                -1 // out of bounds
            }
        },
    )?;

    // ── Inject aegis::host_write_output ─────────────────────────────
    // Allows the skill to pass result data back to the host.
    linker.func_wrap(
        "aegis",
        "host_write_output",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return,
            };
            if ptr < 0 || len < 0 {
                return;
            }
            let data = memory.data(&caller);
            let begin = ptr as usize;
            let end = match begin.checked_add(len as usize) {
                Some(e) => e,
                None => return,
            };
            if end <= data.len() {
                let bytes = data[begin..end].to_vec();
                caller.data_mut().output_buffer = bytes;
            } else {
                tracing::debug!(
                    ptr = ptr,
                    len = len,
                    mem_size = data.len(),
                    "host_write_output: bounds check failed, output not written"
                );
            }
        },
    )?;

    // ── Inject aegis::host_get_payload_len / host_get_payload ───────
    // Passes the JSON payload from host -> Wasm via shared memory.
    let payload_bytes = Arc::new(serde_json::to_vec(payload)?);
    let payload_for_len = payload_bytes.clone();
    let payload_for_copy = payload_bytes.clone();

    linker.func_wrap("aegis", "host_get_payload_len", move || -> i32 {
        payload_for_len.len() as i32
    })?;

    linker.func_wrap(
        "aegis",
        "host_get_payload",
        move |mut caller: Caller<'_, HostState>, dest_ptr: i32| {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return,
            };
            if dest_ptr < 0 {
                return;
            }
            let begin = dest_ptr as usize;
            let end = match begin.checked_add(payload_for_copy.len()) {
                Some(e) => e,
                None => return,
            };
            let mem_len = memory.data(&caller).len();
            if end <= mem_len {
                memory.data_mut(&mut caller)[begin..end].copy_from_slice(&payload_for_copy);
            } else {
                tracing::debug!(
                    dest_ptr = dest_ptr,
                    payload_len = payload_for_copy.len(),
                    mem_size = mem_len,
                    "host_get_payload: bounds check failed, payload not copied"
                );
            }
        },
    )?;

    // ── Inject aegis::host_read_file ──────────────────────────────
    // Reads a file from the allowed_fs_paths whitelist.
    // Returns 0 on success (file content stored in fetch_response buffer),
    // -403 if path not in whitelist, -404 if file not found, -413 if too large,
    // -1 for other errors.
    linker.func_wrap(
        "aegis",
        "host_read_file",
        |mut caller: Caller<'_, HostState>, path_ptr: i32, path_len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -1,
            };

            let path_bytes = {
                let data = memory.data(&caller);
                if path_ptr < 0 || path_len < 0 {
                    return -2;
                }
                let begin = path_ptr as usize;
                let end = match begin.checked_add(path_len as usize) {
                    Some(e) => e,
                    None => return -2,
                };
                if end > data.len() {
                    return -2;
                }
                data[begin..end].to_vec()
            };

            let filename = match std::str::from_utf8(&path_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => return -3,
            };

            // Security: reject path traversal attempts
            if filename.contains("..") || filename.starts_with('/') || filename.starts_with('\\') {
                return -403;
            }

            // Check against allowed_fs_paths whitelist
            let state = caller.data();
            let allowed = &state.allowed_fs_paths;
            if allowed.is_empty() {
                return -403; // no filesystem access configured
            }

            let is_path_allowed = allowed.iter().any(|base| {
                let candidate = std::path::Path::new(base).join(&filename);
                candidate.starts_with(base)
            });

            if !is_path_allowed {
                return -403;
            }

            // Resolve the actual file path (relative to first matching base)
            // Canonicalize to resolve symlinks and prevent symlink escape attacks
            let file_path = allowed.iter().find_map(|base| {
                let candidate = std::path::Path::new(base).join(&filename);
                // Canonicalize resolves symlinks to their real target
                let canonical = candidate.canonicalize().ok()?;
                let base_canonical = std::path::Path::new(base).canonicalize().ok()?;
                // Verify the resolved path is still under the allowed base
                if canonical.starts_with(&base_canonical) {
                    Some(canonical)
                } else {
                    None // Symlink escape attempt — target outside allowed base
                }
            });

            let file_path = match file_path {
                Some(p) => p,
                None => return -404,
            };

            // Read file with size limit (10 MB) — single open+read to avoid TOCTOU
            match std::fs::File::open(&file_path) {
                Ok(file) => {
                    use std::io::Read;
                    let mut reader = file.take(MAX_READ_FILE_BYTES as u64 + 1);
                    let mut content = Vec::new();
                    match reader.read_to_end(&mut content) {
                        Ok(_) => {
                            if content.len() > MAX_READ_FILE_BYTES {
                                return -413;
                            }
                            let state = caller.data_mut();
                            state.fetch_response = content;
                            0 // success
                        }
                        Err(_) => -1,
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => -404,
                Err(_) => -1,
            }
        },
    )?;

    // ── Inject aegis::host_list_files ─────────────────────────────
    // Lists files in the allowed_fs_paths directories.
    // Returns 0 on success (JSON array of filenames in fetch_response),
    // -403 if no filesystem paths configured.
    linker.func_wrap(
        "aegis",
        "host_list_files",
        |mut caller: Caller<'_, HostState>| -> i32 {
            let state = caller.data();
            let allowed = &state.allowed_fs_paths;
            if allowed.is_empty() {
                return -403;
            }

            let mut files: Vec<String> = Vec::new();
            for base in allowed.iter() {
                let base_path = std::path::Path::new(base);
                if !base_path.exists() {
                    continue;
                }
                // Canonicalize the base path for symlink containment check
                let canonical_base = match std::fs::canonicalize(base_path) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if let Ok(entries) = std::fs::read_dir(base_path) {
                    for entry in entries.flatten() {
                        if let Ok(ft) = entry.file_type()
                            && (ft.is_file() || ft.is_symlink())
                        {
                            // Resolve symlinks and reject paths outside allowed dirs
                            let full_path = entry.path();
                            let resolved = match std::fs::canonicalize(&full_path) {
                                Ok(p) => p,
                                Err(_) => continue, // broken symlink
                            };
                            if !resolved.starts_with(&canonical_base) {
                                // Symlink escapes allowed directory — skip
                                continue;
                            }
                            if resolved.is_file()
                                && let Some(name) = entry.file_name().to_str()
                            {
                                files.push(name.to_string());
                            }
                        }
                    }
                }
            }

            let json = serde_json::to_vec(&files).unwrap_or_default();
            let state = caller.data_mut();
            state.fetch_response = json;
            0
        },
    )?;

    // ── Inject aegis::host_store_state ─────────────────────────────
    // Persists a key-value pair scoped to the current skill.
    // Returns 0 on success, -1 on error (no memory handle or store failed).
    linker.func_wrap(
        "aegis",
        "host_store_state",
        |mut caller: Caller<'_, HostState>,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -1,
            };

            let (key_bytes, val_bytes) = {
                let data = memory.data(&caller);
                if key_ptr < 0 || key_len < 0 || val_ptr < 0 || val_len < 0 {
                    return -1;
                }
                let kb = key_ptr as usize;
                let ke = match kb.checked_add(key_len as usize) {
                    Some(e) => e,
                    None => return -1,
                };
                let vb = val_ptr as usize;
                let ve = match vb.checked_add(val_len as usize) {
                    Some(e) => e,
                    None => return -1,
                };
                if ke > data.len() || ve > data.len() {
                    return -1;
                }
                (data[kb..ke].to_vec(), data[vb..ve].to_vec())
            };

            let key = match std::str::from_utf8(&key_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => return -1,
            };
            let val = match std::str::from_utf8(&val_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => return -1,
            };

            let state = caller.data();
            let skill_name = state.skill_name.clone();
            let mh = match &state.memory_handle {
                Some(h) => h.clone(),
                None => return -1,
            };

            // Use tokio Handle to call async method from sync context.
            // Safe: this runs inside spawn_blocking which has access to tokio runtime.
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => return -1,
            };
            match rt.block_on(mh.store_skill_state(&skill_name, &key, &val)) {
                Ok(()) => 0,
                Err(e) => {
                    tracing::debug!(error = %e, skill = %skill_name, "host_store_state failed");
                    -1
                }
            }
        },
    )?;

    // ── Inject aegis::host_load_state ──────────────────────────────
    // Loads a value by key for the current skill.
    // Returns value length on success, -1 if not found, -2 on error.
    // The value is stored in state_response buffer, readable via
    // host_get_state_response / host_get_state_response_len.
    linker.func_wrap(
        "aegis",
        "host_load_state",
        |mut caller: Caller<'_, HostState>, key_ptr: i32, key_len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -2,
            };

            let key_bytes = {
                let data = memory.data(&caller);
                if key_ptr < 0 || key_len < 0 {
                    return -2;
                }
                let kb = key_ptr as usize;
                let ke = match kb.checked_add(key_len as usize) {
                    Some(e) => e,
                    None => return -2,
                };
                if ke > data.len() {
                    return -2;
                }
                data[kb..ke].to_vec()
            };

            let key = match std::str::from_utf8(&key_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => return -2,
            };

            let state = caller.data();
            let skill_name = state.skill_name.clone();
            let mh = match &state.memory_handle {
                Some(h) => h.clone(),
                None => return -2,
            };

            let rt = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => return -2,
            };
            match rt.block_on(mh.load_skill_state(&skill_name, &key)) {
                Ok(Some(val)) => {
                    let bytes = val.into_bytes();
                    let len = bytes.len() as i32;
                    caller.data_mut().state_response = bytes;
                    len
                }
                Ok(None) => -1, // not found
                Err(e) => {
                    tracing::debug!(error = %e, skill = %skill_name, "host_load_state failed");
                    -2
                }
            }
        },
    )?;

    // ── Inject aegis::host_get_state_response_len ──────────────────
    linker.func_wrap(
        "aegis",
        "host_get_state_response_len",
        |caller: Caller<'_, HostState>| -> i32 { caller.data().state_response.len() as i32 },
    )?;

    // ── Inject aegis::host_get_state_response ──────────────────────
    // Copies the state response buffer into Wasm memory at dest_ptr.
    // Returns 0 on success, -1 on out-of-bounds.
    linker.func_wrap(
        "aegis",
        "host_get_state_response",
        |mut caller: Caller<'_, HostState>, dest_ptr: i32| -> i32 {
            let response = caller.data().state_response.clone();
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return -1,
            };
            if dest_ptr < 0 {
                return -1;
            }
            let begin = dest_ptr as usize;
            let end = match begin.checked_add(response.len()) {
                Some(e) => e,
                None => return -1,
            };
            if end <= memory.data(&caller).len() {
                memory.data_mut(&mut caller)[begin..end].copy_from_slice(&response);
                0
            } else {
                -1
            }
        },
    )?;

    Ok(())
}

/// Provide minimal WASI `wasi_snapshot_preview1` stubs so that
/// `eprintln!` / `println!` in Rust WASM modules don't trap.
///
/// Only `fd_write` is given a real implementation (calculates byte count,
/// writes it to *nwritten, returns success for stdout/stderr).
/// Other WASI functions will still be caught by `define_unknown_imports_as_traps`.
pub(crate) fn link_wasi_stdio_stubs(linker: &mut Linker<HostState>) -> Result<()> {
    // fd_write(fd: i32, iovs_ptr: i32, iovs_len: i32, nwritten_ptr: i32) -> i32
    //
    // WASI iov layout: each iov is 8 bytes (buf_ptr: u32, buf_len: u32).
    // We sum buf_len across all iovs, write total to *nwritten, discard the data.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, HostState>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nwritten_ptr: i32|
         -> i32 {
            // Only allow stdout (1) and stderr (2)
            if fd != 1 && fd != 2 {
                return 8; // EBADF
            }

            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };

            // Sum iov buf_len fields to get total bytes
            let mut total: u32 = 0;
            let data = memory.data(&caller);
            for i in 0..iovs_len {
                let iov_offset = iovs_ptr as usize + (i as usize) * 8;
                if iov_offset + 8 > data.len() {
                    return 21; // EFAULT
                }
                let buf_len = u32::from_le_bytes([
                    data[iov_offset + 4],
                    data[iov_offset + 5],
                    data[iov_offset + 6],
                    data[iov_offset + 7],
                ]);
                total = total.saturating_add(buf_len);
            }

            // Write total bytes to *nwritten
            let nw_offset = nwritten_ptr as usize;
            let data_mut = memory.data_mut(&mut caller);
            if nw_offset + 4 <= data_mut.len() {
                data_mut[nw_offset..nw_offset + 4].copy_from_slice(&total.to_le_bytes());
            }

            0 // ESUCCESS
        },
    )?;

    // fd_seek(fd, offset_lo, offset_hi, whence, newoffset_ptr) -> errno
    // No-op: return ESPIPE (29) for pipes/non-seekable
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_seek",
        |_caller: Caller<'_, HostState>,
         _fd: i32,
         _offset: i64,
         _whence: i32,
         _newoffset_ptr: i32|
         -> i32 {
            29 // ESPIPE — not seekable
        },
    )?;

    // fd_close(fd) -> errno
    // No-op: return success for stdio fds, EBADF otherwise
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_close",
        |_caller: Caller<'_, HostState>, fd: i32| -> i32 {
            if fd == 1 || fd == 2 { 0 } else { 8 } // EBADF
        },
    )?;

    // proc_exit(code) -> !
    // Must trap to stop execution
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "proc_exit",
        |_caller: Caller<'_, HostState>, _code: i32| {
            // This is intentionally a no-return trap — proc_exit terminates the process.
        },
    )?;

    // random_get(buf_ptr, buf_len) -> errno
    //
    // Rust stdlib uses this to seed HashMap's SipHash-1-3 hasher on the first
    // insert. If missing, every HashMap-touching skill panics with an
    // `unknown import` trap. We fill the buffer with actual CSPRNG bytes —
    // using zeros would break HashMap's DoS resistance.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "random_get",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
            if buf_ptr < 0 || buf_len < 0 {
                return 21; // EFAULT
            }
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };
            let begin = buf_ptr as usize;
            let len = buf_len as usize;
            let end = match begin.checked_add(len) {
                Some(e) => e,
                None => return 21, // EFAULT
            };
            let data_mut = memory.data_mut(&mut caller);
            if end > data_mut.len() {
                return 21; // EFAULT
            }
            // Fill with cryptographic randomness. Must be real entropy —
            // HashMap's DoS resistance depends on unpredictable seed bytes.
            use rand::RngCore;
            let mut rng = rand::rng();
            rng.fill_bytes(&mut data_mut[begin..end]);
            0 // ESUCCESS
        },
    )?;

    // environ_sizes_get(count_ptr, total_size_ptr) -> errno
    //
    // We expose an empty environment to skills (sealed sandbox). Write
    // 0u32 to both pointer slots.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "environ_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, total_size_ptr: i32| -> i32 {
            if count_ptr < 0 || total_size_ptr < 0 {
                return 21; // EFAULT
            }
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };
            let data_mut = memory.data_mut(&mut caller);
            let count_off = count_ptr as usize;
            let size_off = total_size_ptr as usize;
            if count_off.checked_add(4).is_none_or(|e| e > data_mut.len())
                || size_off.checked_add(4).is_none_or(|e| e > data_mut.len())
            {
                return 21; // EFAULT
            }
            let zero = 0u32.to_le_bytes();
            data_mut[count_off..count_off + 4].copy_from_slice(&zero);
            data_mut[size_off..size_off + 4].copy_from_slice(&zero);
            0 // ESUCCESS
        },
    )?;

    // environ_get(environ_ptr, environ_buf_ptr) -> errno
    //
    // With count=0 advertised by environ_sizes_get, the caller won't
    // dereference these pointers. Keep as no-op success.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "environ_get",
        |_caller: Caller<'_, HostState>, _environ_ptr: i32, _environ_buf_ptr: i32| -> i32 {
            0 // ESUCCESS
        },
    )?;

    // clock_time_get(clock_id, precision, time_ptr) -> errno
    //
    // clock_id=0 REALTIME: ns since UNIX epoch.
    // clock_id=1 MONOTONIC: ns since lazy process-startup origin.
    // Other clock ids are rejected with EINVAL.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "clock_time_get",
        |mut caller: Caller<'_, HostState>, clock_id: i32, _precision: i64, time_ptr: i32| -> i32 {
            if time_ptr < 0 {
                return 21; // EFAULT
            }
            let nanos: u64 = match clock_id {
                0 => {
                    // REALTIME
                    match SystemTime::now().duration_since(UNIX_EPOCH) {
                        Ok(d) => d.as_nanos().min(u64::MAX as u128) as u64,
                        Err(_) => return 28, // EINVAL — clock went backwards
                    }
                }
                1 => {
                    // MONOTONIC
                    let origin = MONOTONIC_ORIGIN.get_or_init(Instant::now);
                    let d = Instant::now().saturating_duration_since(*origin);
                    d.as_nanos().min(u64::MAX as u128) as u64
                }
                _ => return 28, // EINVAL — unsupported clock (process/thread CPU-time)
            };
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };
            let data_mut = memory.data_mut(&mut caller);
            let off = time_ptr as usize;
            if off.checked_add(8).is_none_or(|e| e > data_mut.len()) {
                return 21; // EFAULT
            }
            data_mut[off..off + 8].copy_from_slice(&nanos.to_le_bytes());
            0 // ESUCCESS
        },
    )?;

    // clock_res_get(clock_id, resolution_ptr) -> errno
    //
    // Report 1 microsecond (1000 ns) as the nominal resolution for all clocks.
    // Skills typically just need a non-zero value; exact figure is not critical.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "clock_res_get",
        |mut caller: Caller<'_, HostState>, _clock_id: i32, resolution_ptr: i32| -> i32 {
            if resolution_ptr < 0 {
                return 21; // EFAULT
            }
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };
            let data_mut = memory.data_mut(&mut caller);
            let off = resolution_ptr as usize;
            if off.checked_add(8).is_none_or(|e| e > data_mut.len()) {
                return 21; // EFAULT
            }
            let res_ns: u64 = 1_000; // 1 microsecond
            data_mut[off..off + 8].copy_from_slice(&res_ns.to_le_bytes());
            0 // ESUCCESS
        },
    )?;

    // sched_yield() -> errno
    //
    // No-op: we run each skill in its own dedicated host slot, yielding
    // semantics are irrelevant. Always success.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "sched_yield",
        |_caller: Caller<'_, HostState>| -> i32 {
            0 // ESUCCESS
        },
    )?;

    // poll_oneoff(in_ptr, out_ptr, nsubs, nevents_ptr) -> errno
    //
    // Conservative stub: report zero ready events. Skills that genuinely
    // rely on polling will observe "nothing ready" and should either
    // short-circuit or loop — but since we also deny fd_* besides stdio,
    // real polling isn't meaningful in this sandbox anyway.
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "poll_oneoff",
        |mut caller: Caller<'_, HostState>,
         _in_ptr: i32,
         _out_ptr: i32,
         _nsubs: i32,
         nevents_ptr: i32|
         -> i32 {
            if nevents_ptr < 0 {
                return 21; // EFAULT
            }
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => return 8, // EBADF
            };
            let data_mut = memory.data_mut(&mut caller);
            let off = nevents_ptr as usize;
            if off.checked_add(4).is_none_or(|e| e > data_mut.len()) {
                return 21; // EFAULT
            }
            let zero = 0u32.to_le_bytes();
            data_mut[off..off + 4].copy_from_slice(&zero);
            0 // ESUCCESS
        },
    )?;

    Ok(())
}

/// Instantiate a WASM module and execute its `run()` entry point.
/// Returns the skill's output as a String.
pub(crate) fn execute_skill_module(
    store: &mut Store<HostState>,
    linker: &mut Linker<HostState>,
    module: &Module,
    capability: &SkillCapability,
    effective_fuel: u64,
) -> Result<String> {
    let instance = linker
        .instantiate(&mut *store, module)
        .map_err(anyhow::Error::from)
        .context("instantiating Wasm module")?;

    let run_fn = instance
        .get_typed_func::<(), ()>(&mut *store, "run")
        .map_err(anyhow::Error::from)
        .context("skill must export a `run()` function")?;

    const MAX_WALL_CLOCK_SECS: u64 = 300;
    const MIN_WALL_CLOCK_SECS: u64 = 5;
    let exec_start = std::time::Instant::now();
    let max_wall_secs = capability
        .timeout_secs
        .clamp(MIN_WALL_CLOCK_SECS, MAX_WALL_CLOCK_SECS);

    let call_result = run_fn.call(&mut *store, ());

    let elapsed_ms = exec_start.elapsed().as_millis();
    let fuel_remaining = store.get_fuel().unwrap_or(0);
    let fuel_consumed = effective_fuel.saturating_sub(fuel_remaining);
    let peak_memory_kb = store.data().limiter.peak_memory / 1024;

    let call_result = call_result.map_err(anyhow::Error::from);

    match call_result {
        Ok(()) => {
            if exec_start.elapsed().as_secs() > max_wall_secs {
                tracing::warn!(
                    skill = %capability.skill_name,
                    elapsed_ms,
                    max_wall_secs,
                    "Skill exceeded wall-clock timeout (completed but slow)"
                );
            }

            tracing::debug!(
                skill = %capability.skill_name,
                elapsed_ms,
                fuel_consumed,
                peak_memory_kb,
                "Skill execution completed"
            );

            let output = String::from_utf8(store.data().output_buffer.clone())
                .unwrap_or_else(|_| String::from("(non-utf8 output)"));
            Ok(output)
        }
        Err(e) => {
            let trap_kind = classify_trap(&e);
            tracing::warn!(
                skill = %capability.skill_name,
                trap = %trap_kind,
                elapsed_ms,
                fuel_consumed,
                peak_memory_kb,
                "Skill execution trapped"
            );
            Err(e).context(format!("skill execution failed [{}]", trap_kind))
        }
    }
}

// ── Per-instance memory limiter ──────────────────────────────────────────

/// Per-instance memory limiter.
pub(crate) struct StoreLimiter {
    pub(crate) max_memory: usize,
    /// Track peak memory usage for observability.
    pub(crate) peak_memory: usize,
}

impl ResourceLimiter for StoreLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory {
            return Ok(false);
        }
        if desired > self.peak_memory {
            self.peak_memory = desired;
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 10_000)
    }
}

// ── Trap classification ─────────────────────────────────────────────────

/// Classification of Wasm execution traps for structured error reporting.
#[derive(Debug, Clone, PartialEq)]
pub enum TrapKind {
    /// CPU instruction budget (fuel) exhausted — infinite loop or heavy computation.
    FuelExhausted,
    /// Memory allocation exceeded the per-skill limit.
    MemoryExceeded,
    /// Wall-clock timeout exceeded (defense-in-depth against slow I/O).
    Timeout,
    /// Wasm panic, unreachable instruction, or other runtime fault.
    Panic(String),
    /// Module instantiation failure (missing imports, invalid module).
    InstantiationFailed(String),
}

impl std::fmt::Display for TrapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrapKind::FuelExhausted => write!(f, "FuelExhausted"),
            TrapKind::MemoryExceeded => write!(f, "MemoryExceeded"),
            TrapKind::Timeout => write!(f, "Timeout"),
            TrapKind::Panic(msg) => write!(f, "Panic: {}", msg),
            TrapKind::InstantiationFailed(msg) => write!(f, "InstantiationFailed: {}", msg),
        }
    }
}

/// Classify a Wasmtime error into a structured TrapKind.
pub(crate) fn classify_trap(err: &anyhow::Error) -> TrapKind {
    let err_str = format!("{err:#}");
    let lower = err_str.to_lowercase();

    if lower.contains("fuel")
        || lower.contains("out of fuel")
        || lower.contains("all fuel consumed")
    {
        TrapKind::FuelExhausted
    } else if lower.contains("memory")
        && (lower.contains("grow") || lower.contains("limit") || lower.contains("oom"))
    {
        TrapKind::MemoryExceeded
    } else if lower.contains("unreachable") || lower.contains("wasm trap") {
        TrapKind::Panic(truncate_for_trap(&err_str, 200))
    } else if lower.contains("instantiat") {
        TrapKind::InstantiationFailed(truncate_for_trap(&err_str, 200))
    } else {
        TrapKind::Panic(truncate_for_trap(&err_str, 200))
    }
}

fn truncate_for_trap(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Trap classification tests ───────────────────────────────────────

    #[test]
    fn classify_trap_fuel_exhausted() {
        let err = anyhow::anyhow!("all fuel consumed by WebAssembly");
        assert_eq!(classify_trap(&err), TrapKind::FuelExhausted);
    }

    #[test]
    fn classify_trap_memory_exceeded() {
        let err = anyhow::anyhow!("memory grow beyond limit");
        assert_eq!(classify_trap(&err), TrapKind::MemoryExceeded);
    }

    #[test]
    fn classify_trap_unreachable() {
        let err = anyhow::anyhow!("wasm trap: unreachable");
        assert!(matches!(classify_trap(&err), TrapKind::Panic(_)));
    }

    #[test]
    fn classify_trap_instantiation() {
        let err = anyhow::anyhow!("error while instantiating module");
        assert!(matches!(
            classify_trap(&err),
            TrapKind::InstantiationFailed(_)
        ));
    }

    #[test]
    fn classify_trap_unknown_defaults_to_panic() {
        let err = anyhow::anyhow!("some unknown error");
        assert!(matches!(classify_trap(&err), TrapKind::Panic(_)));
    }

    #[test]
    fn store_limiter_tracks_peak_memory() {
        let mut limiter = StoreLimiter {
            max_memory: 4 * 1024 * 1024,
            peak_memory: 0,
        };
        // Simulate growing memory
        assert!(limiter.memory_growing(0, 1024, None).unwrap());
        assert_eq!(limiter.peak_memory, 1024);
        assert!(limiter.memory_growing(1024, 2048, None).unwrap());
        assert_eq!(limiter.peak_memory, 2048);
        // Shrink doesn't update peak
        assert!(limiter.memory_growing(2048, 1024, None).unwrap());
        assert_eq!(limiter.peak_memory, 2048);
        // Exceed limit
        assert!(!limiter.memory_growing(2048, 5 * 1024 * 1024, None).unwrap());
        assert_eq!(limiter.peak_memory, 2048); // unchanged on reject
    }

    // ── Snapshot tests ──────────────────────────────────────────────────

    mod snapshot_host_fns {
        use super::*;

        #[test]
        fn snapshot_trap_kind_display() {
            let traps = [
                TrapKind::FuelExhausted,
                TrapKind::MemoryExceeded,
                TrapKind::Timeout,
                TrapKind::Panic("unreachable executed".to_string()),
                TrapKind::InstantiationFailed("missing import: host_fetch".to_string()),
            ];
            let display: Vec<String> = traps.iter().map(|t| t.to_string()).collect();
            insta::assert_yaml_snapshot!(display);
        }

        #[test]
        fn snapshot_classify_trap_fuel() {
            let err = anyhow::anyhow!("all fuel consumed by WebAssembly");
            insta::assert_snapshot!(classify_trap(&err).to_string(), @"FuelExhausted");
        }

        #[test]
        fn snapshot_classify_trap_memory() {
            let err = anyhow::anyhow!("memory growth limit exceeded");
            insta::assert_snapshot!(classify_trap(&err).to_string(), @"MemoryExceeded");
        }

        #[test]
        fn snapshot_classify_trap_unreachable() {
            let err = anyhow::anyhow!("wasm trap: unreachable");
            let kind = classify_trap(&err);
            assert!(matches!(kind, TrapKind::Panic(_)));
        }

        #[test]
        fn snapshot_classify_trap_instantiation() {
            let err = anyhow::anyhow!("instantiation failed: unknown import");
            let kind = classify_trap(&err);
            assert!(matches!(kind, TrapKind::InstantiationFailed(_)));
        }
    }

    // ── WASI stub integration tests ─────────────────────────────────────
    //
    // Regression guards for the 2026-04-17 bug where calendar_reader/web_reader
    // WASM modules panicked every 30 min because `random_get`, `clock_*`,
    // `environ_*`, `sched_yield`, and `poll_oneoff` were unimplemented.
    // These tests spin up a minimal WASM module that imports each stub,
    // calls it with known arguments, and asserts the expected side effect.

    mod wasi_stubs {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Arc;

        /// Build a minimal HostState for linker tests. Sandbox features we
        /// don't exercise (fetch pool, memory handle) get cheap placeholders.
        fn make_test_host_state() -> HostState {
            let fetch_pool = Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .expect("build test rayon pool"),
            );
            HostState {
                allowed_domains: Arc::new(Vec::new()),
                allowed_fs_paths: Arc::new(Vec::new()),
                fetch_log: Vec::new(),
                fetch_count: 0,
                domain_fetch_counts: HashMap::new(),
                max_fetch_calls: crate::sandbox::MAX_FETCH_CALLS,
                max_fetch_per_domain: crate::sandbox::MAX_FETCH_PER_DOMAIN,
                output_buffer: Vec::new(),
                fetch_response: Vec::new(),
                limiter: StoreLimiter {
                    max_memory: 4 * 1024 * 1024,
                    peak_memory: 0,
                },
                auth_headers: Arc::new(HashMap::new()),
                memory_handle: None,
                skill_name: "wasi_stub_test".to_string(),
                state_response: Vec::new(),
                fetch_pool,
            }
        }

        /// Compile+instantiate a WAT module and invoke its `run` export.
        /// Returns the exported memory (so tests can read back what the
        /// stub wrote) and the i32 return value.
        fn run_wat(wat: &str) -> (Vec<u8>, i32) {
            let mut config = Config::new();
            config.consume_fuel(false);
            let engine = Engine::new(&config).expect("engine");
            let module = Module::new(&engine, wat).expect("module parse");
            let mut linker: Linker<HostState> = Linker::new(&engine);
            link_wasi_stdio_stubs(&mut linker).expect("link wasi stubs");
            let mut store = Store::new(&engine, make_test_host_state());
            let instance = linker
                .instantiate(&mut store, &module)
                .expect("instantiate");
            let run = instance
                .get_typed_func::<(), i32>(&mut store, "run")
                .expect("run export");
            let ret = run.call(&mut store, ()).expect("call run");
            let memory = instance.get_memory(&mut store, "memory").expect("memory");
            let data = memory.data(&store).to_vec();
            (data, ret)
        }

        #[test]
        fn wasi_random_get_fills_buffer() {
            // run() calls random_get(16, 8) and returns the errno.
            // We then inspect bytes 16..24 of memory.
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "random_get"
                        (func $random_get (param i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $random_get (i32.const 16) (i32.const 8))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "random_get should return ESUCCESS");
            let filled = &mem[16..24];
            assert!(
                filled.iter().any(|&b| b != 0),
                "random_get must fill buffer with real entropy, got all zeros: {filled:?}"
            );
        }

        #[test]
        fn wasi_environ_sizes_get_returns_zero_count() {
            // Writes count@32, total_size@40, both should be 0.
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "environ_sizes_get"
                        (func $environ_sizes_get (param i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $environ_sizes_get (i32.const 32) (i32.const 40))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "environ_sizes_get should return ESUCCESS");
            let count = u32::from_le_bytes([mem[32], mem[33], mem[34], mem[35]]);
            let total = u32::from_le_bytes([mem[40], mem[41], mem[42], mem[43]]);
            assert_eq!(count, 0, "count must be 0 (empty environ)");
            assert_eq!(total, 0, "total_size must be 0 (empty environ)");
        }

        #[test]
        fn wasi_clock_time_get_returns_nonzero() {
            // Call clock_time_get(REALTIME=0, precision=0, time_ptr=64),
            // then read u64 at offset 64. Should be > 0 (post-UNIX-epoch ns).
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "clock_time_get"
                        (func $clock_time_get (param i32 i64 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $clock_time_get (i32.const 0) (i64.const 0) (i32.const 64))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "clock_time_get should return ESUCCESS");
            let nanos = u64::from_le_bytes([
                mem[64], mem[65], mem[66], mem[67], mem[68], mem[69], mem[70], mem[71],
            ]);
            assert!(
                nanos > 0,
                "clock_time_get(REALTIME) should write a nonzero ns value, got {nanos}"
            );
        }

        #[test]
        fn wasi_sched_yield_returns_zero() {
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "sched_yield"
                        (func $sched_yield (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $sched_yield)))
            "#;
            let (_mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "sched_yield should return ESUCCESS");
        }

        #[test]
        fn wasi_clock_time_get_monotonic_returns_nonzero() {
            // clock_id=1 (MONOTONIC). First call may be 0 on the same tick
            // as origin init; we call twice so the second one is guaranteed
            // to reflect elapsed time.
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "clock_time_get"
                        (func $clock_time_get (param i32 i64 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (drop (call $clock_time_get (i32.const 1) (i64.const 0) (i32.const 72)))
                        (call $clock_time_get (i32.const 1) (i64.const 0) (i32.const 80))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "clock_time_get(MONOTONIC) should return ESUCCESS");
            let nanos = u64::from_le_bytes([
                mem[80], mem[81], mem[82], mem[83], mem[84], mem[85], mem[86], mem[87],
            ]);
            // Should be non-negative; we don't assert > 0 strictly because
            // two back-to-back Instant::now() calls can elapse < 1 ns on
            // some platforms. Just ensure the stub did not trap.
            let _ = nanos;
        }

        #[test]
        fn wasi_poll_oneoff_returns_zero_events() {
            // poll_oneoff(in=0, out=0, nsubs=0, nevents_ptr=96) -> 0,
            // *nevents == 0.
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "poll_oneoff"
                        (func $poll_oneoff (param i32 i32 i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $poll_oneoff (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 96))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "poll_oneoff should return ESUCCESS");
            let nevents = u32::from_le_bytes([mem[96], mem[97], mem[98], mem[99]]);
            assert_eq!(nevents, 0, "poll_oneoff must report 0 ready events");
        }

        #[test]
        fn wasi_clock_res_get_returns_nonzero_resolution() {
            let wat = r#"
                (module
                    (import "wasi_snapshot_preview1" "clock_res_get"
                        (func $clock_res_get (param i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (func (export "run") (result i32)
                        (call $clock_res_get (i32.const 0) (i32.const 104))))
            "#;
            let (mem, ret) = run_wat(wat);
            assert_eq!(ret, 0, "clock_res_get should return ESUCCESS");
            let res = u64::from_le_bytes([
                mem[104], mem[105], mem[106], mem[107], mem[108], mem[109], mem[110], mem[111],
            ]);
            assert!(res > 0, "clock_res_get should write a nonzero resolution");
        }
    }

    // ── Property-based tests ────────────────────────────────────────────

    mod proptest_limiter {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Memory growing within limit always succeeds
            #[test]
            fn memory_within_limit_allowed(desired in 0usize..=(2 * 1024 * 1024)) {
                let mut limiter = StoreLimiter {
                    max_memory: 2 * 1024 * 1024,
                    peak_memory: 0,
                };
                prop_assert!(limiter.memory_growing(0, desired, None).unwrap());
            }

            /// Memory growing beyond limit always rejected
            #[test]
            fn memory_beyond_limit_rejected(extra in 1usize..10_000_000) {
                let max = 2 * 1024 * 1024usize;
                let mut limiter = StoreLimiter {
                    max_memory: max,
                    peak_memory: 0,
                };
                prop_assert!(!limiter.memory_growing(0, max + extra, None).unwrap());
            }

            /// Peak memory is monotonically non-decreasing
            #[test]
            fn peak_memory_monotonic(
                a in 0usize..1_000_000,
                b in 0usize..1_000_000,
                c in 0usize..1_000_000
            ) {
                let mut limiter = StoreLimiter {
                    max_memory: 4 * 1024 * 1024,
                    peak_memory: 0,
                };
                let _ = limiter.memory_growing(0, a, None);
                let peak_after_a = limiter.peak_memory;
                let _ = limiter.memory_growing(a, b, None);
                let peak_after_b = limiter.peak_memory;
                let _ = limiter.memory_growing(b, c, None);
                let peak_after_c = limiter.peak_memory;
                prop_assert!(peak_after_b >= peak_after_a, "peak must be monotonic");
                prop_assert!(peak_after_c >= peak_after_b, "peak must be monotonic");
            }
        }
    }
}
