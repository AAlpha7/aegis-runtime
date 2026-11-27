// ── v2 Component Model host implementations for the S-LEG sandbox ──────
//
// Extracted from sandbox.rs — WIT-generated trait implementations for
// skill_v2 and probe_v2 worlds, plus shared v2 fetch/state logic.

use crate::sandbox::{FetchRecord, HostState};
use crate::sandbox_network::{MAX_FETCH_RESPONSE_BYTES, is_domain_allowed, is_private_ip_addr};
use crate::wit_bindings::{probe_v2, skill_v2};

// ── v2 Component Model host trait implementations ───────────────────────

/// Implement the `types::Host` trait (required by bindgen, typically empty).
impl skill_v2::aegis::skill::types::Host for HostState {}
impl probe_v2::aegis::skill::types::Host for HostState {}

/// Implement `host_v2::Host` for HostState — the same security logic as v1
/// but with clean typed signatures (no pointer arithmetic).
impl skill_v2::aegis::skill::host_v2::Host for HostState {
    fn fetch(&mut self, url: String) -> Result<String, skill_v2::aegis::skill::types::FetchError> {
        host_v2_fetch(self, url)
    }

    fn store_state(&mut self, key: String, value: String) -> Result<(), String> {
        host_v2_store_state(self, key, value)
    }

    fn load_state(&mut self, key: String) -> Option<String> {
        host_v2_load_state(self, key)
    }
}

/// Implement `host_v2::Host` for probe world (same implementation).
impl probe_v2::aegis::skill::host_v2::Host for HostState {
    fn fetch(&mut self, url: String) -> Result<String, probe_v2::aegis::skill::types::FetchError> {
        host_v2_fetch(self, url)
    }

    fn store_state(&mut self, key: String, value: String) -> Result<(), String> {
        host_v2_store_state(self, key, value)
    }

    fn load_state(&mut self, key: String) -> Option<String> {
        host_v2_load_state(self, key)
    }
}

/// Shared v2 fetch implementation — same S-LEG security as v1 but without
/// pointer management. Domain whitelist, rate limiting, DNS rebinding
/// protection, and auth injection all apply identically.
fn host_v2_fetch<E: From<FetchErrorKind>>(state: &mut HostState, url: String) -> Result<String, E> {
    // Rate limit — caps live in HostState (execution-scoped): defaults are
    // MAX_FETCH_CALLS / MAX_FETCH_PER_DOMAIN, raised only via a trusted-host
    // fetch budget (sandbox::effective_fetch_caps).
    state.fetch_count += 1;
    if state.fetch_count > state.max_fetch_calls {
        state.fetch_log.push(FetchRecord {
            url,
            allowed: false,
        });
        return Err(FetchErrorKind::RateLimited.into());
    }

    // Domain whitelist
    let is_allowed = is_domain_allowed(&url, &state.allowed_domains);
    state.fetch_log.push(FetchRecord {
        url: url.clone(),
        allowed: is_allowed,
    });
    if !is_allowed {
        tracing::warn!(skill = %state.skill_name, url = %url, "v2 fetch blocked: domain not in whitelist");
        return Err(FetchErrorKind::DomainBlocked.into());
    }

    // Per-domain rate limiting
    if let Ok(parsed) = url::Url::parse(&url)
        && let Some(host) = parsed.host_str()
    {
        let count = state
            .domain_fetch_counts
            .entry(host.to_string())
            .or_insert(0);
        *count += 1;
        if *count > state.max_fetch_per_domain {
            tracing::warn!(skill = %state.skill_name, domain = %host, limit = state.max_fetch_per_domain, "v2 fetch blocked: per-domain rate limit");
            return Err(FetchErrorKind::RateLimited.into());
        }
    }

    // HTTP fetch (reuses same DNS rebinding / auth injection logic as v1)
    let domains_for_dns = state.allowed_domains.clone();
    let auth_headers = state.auth_headers.clone();
    let pool = state.fetch_pool.clone();

    let fetch_result: Option<Vec<u8>> = pool.install(move || -> Option<Vec<u8>> {
        let parsed = url::Url::parse(&url).ok()?;
        let host = parsed.host_str()?;
        let explicitly_whitelisted = domains_for_dns.iter().any(|d| d == host);

        // DNS resolution + private IP blocking
        let pinned_ips: Vec<std::net::SocketAddr> = {
            use std::net::ToSocketAddrs;
            let port = parsed
                .port()
                .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
            match (host, port).to_socket_addrs() {
                Ok(addrs) => {
                    let addrs_vec: Vec<_> = addrs.collect();
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
                Err(_) => return None,
            }
        };

        let original_host = host.to_string();
        let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 3 {
                attempt.stop()
            } else if let Some(rh) = attempt.url().host_str() {
                if rh == original_host && attempt.url().scheme() == "https" {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            } else {
                attempt.stop()
            }
        });

        let is_localhost = host == "127.0.0.1" || host == "localhost" || host == "::1";
        let timeout_secs = if is_localhost { 60 } else { 15 };
        let mut builder = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .redirect(redirect_policy)
            .user_agent("reqwest/0.12 (Aegis Runtime Skill Runtime)");

        if let Some(addr) = pinned_ips.first() {
            builder = builder.resolve(host, *addr);
        }
        let client = builder.build().ok()?;

        let mut request = client.get(&url);
        if let Some(auth_value) = auth_headers.get(host) {
            request = request.header("Authorization", auth_value);
        }

        let resp = request.send().ok()?;

        use std::io::Read as _;
        let prealloc = resp
            .content_length()
            .map(|cl| cl.min(MAX_FETCH_RESPONSE_BYTES as u64) as usize)
            .unwrap_or(0);
        let mut body = Vec::with_capacity(prealloc);
        let mut reader = resp;
        loop {
            let remaining = MAX_FETCH_RESPONSE_BYTES - body.len();
            if remaining == 0 {
                break;
            }
            let chunk_size = remaining.min(8192);
            let mut chunk = vec![0u8; chunk_size];
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        Some(body)
    });

    match fetch_result {
        Some(body) => {
            let text = String::from_utf8(body)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            Ok(text)
        }
        None => Err(FetchErrorKind::NetworkError.into()),
    }
}

/// Shared v2 store_state implementation.
fn host_v2_store_state(state: &mut HostState, key: String, value: String) -> Result<(), String> {
    let Some(ref memory_handle) = state.memory_handle else {
        return Err("state persistence not available".to_string());
    };
    let skill_name = &state.skill_name;
    // Safe: sandbox.execute() is always called via tokio::task::spawn_blocking,
    // so we're on a blocking thread where Handle::block_on() won't panic.
    // (Same pattern as v1 host functions in sandbox_host_fns.rs.)
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "no tokio runtime for state persistence".to_string())?;
    handle
        .block_on(memory_handle.store_skill_state(skill_name, &key, &value))
        .map_err(|e| format!("state store failed: {e}"))
}

/// Shared v2 load_state implementation.
fn host_v2_load_state(state: &mut HostState, key: String) -> Option<String> {
    let memory_handle = state.memory_handle.as_ref()?;
    let skill_name = &state.skill_name;
    // Safe: called from spawn_blocking context — see store_state comment above.
    let handle = tokio::runtime::Handle::try_current().ok()?;
    handle
        .block_on(memory_handle.load_skill_state(skill_name, &key))
        .ok()
        .flatten()
}

// ── FetchErrorKind ──────────────────────────────────────────────────────

/// Internal fetch error kinds for type-erased conversion.
pub(crate) enum FetchErrorKind {
    DomainBlocked,
    RateLimited,
    NetworkError,
    #[allow(dead_code)]
    InvalidUrl,
    #[allow(dead_code)]
    ResponseTooLarge,
}

impl From<FetchErrorKind> for skill_v2::aegis::skill::types::FetchError {
    fn from(kind: FetchErrorKind) -> Self {
        match kind {
            FetchErrorKind::DomainBlocked => Self::DomainBlocked,
            FetchErrorKind::RateLimited => Self::RateLimited,
            FetchErrorKind::NetworkError => Self::NetworkError,
            FetchErrorKind::InvalidUrl => Self::InvalidUrl,
            FetchErrorKind::ResponseTooLarge => Self::ResponseTooLarge,
        }
    }
}

impl From<FetchErrorKind> for probe_v2::aegis::skill::types::FetchError {
    fn from(kind: FetchErrorKind) -> Self {
        match kind {
            FetchErrorKind::DomainBlocked => Self::DomainBlocked,
            FetchErrorKind::RateLimited => Self::RateLimited,
            FetchErrorKind::NetworkError => Self::NetworkError,
            FetchErrorKind::InvalidUrl => Self::InvalidUrl,
            FetchErrorKind::ResponseTooLarge => Self::ResponseTooLarge,
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_error_kind_converts_to_skill_v2() {
        let err: skill_v2::aegis::skill::types::FetchError = FetchErrorKind::DomainBlocked.into();
        assert!(matches!(
            err,
            skill_v2::aegis::skill::types::FetchError::DomainBlocked
        ));
    }

    #[test]
    fn fetch_error_kind_converts_to_probe_v2() {
        let err: probe_v2::aegis::skill::types::FetchError = FetchErrorKind::RateLimited.into();
        assert!(matches!(
            err,
            probe_v2::aegis::skill::types::FetchError::RateLimited
        ));
    }

    /// Minimal HostState for v2 fetch-cap tests, with explicit
    /// execution-scoped caps (the fields `sandbox::effective_fetch_caps`
    /// resolves into).
    fn budget_test_state(max_fetch_calls: usize, max_fetch_per_domain: usize) -> HostState {
        use std::collections::HashMap;
        use std::sync::Arc;
        HostState {
            allowed_domains: Arc::new(vec!["127.0.0.1".to_string()]),
            allowed_fs_paths: Arc::new(Vec::new()),
            fetch_log: Vec::new(),
            fetch_count: 0,
            domain_fetch_counts: HashMap::new(),
            max_fetch_calls,
            max_fetch_per_domain,
            output_buffer: Vec::new(),
            fetch_response: Vec::new(),
            limiter: crate::sandbox_host_fns::StoreLimiter {
                max_memory: 4 * 1024 * 1024,
                peak_memory: 0,
            },
            auth_headers: Arc::new(HashMap::new()),
            memory_handle: None,
            skill_name: "budget_test".to_string(),
            state_response: Vec::new(),
            fetch_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .expect("build test rayon pool"),
            ),
        }
    }

    #[test]
    fn v2_per_domain_cap_uses_execution_scoped_budget() {
        use crate::sandbox::{MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN, effective_fetch_caps};
        type FetchError = skill_v2::aegis::skill::types::FetchError;

        // Closed loopback port: a fetch that passes the cap gate fails at
        // connect time (NetworkError) without real network I/O.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
            let port = listener.local_addr().expect("local addr").port();
            drop(listener);
            port
        };
        let url = format!("http://127.0.0.1:{port}/");

        // Default caps: once the per-domain allowance is consumed, the next
        // fetch is blocked BEFORE any network attempt.
        let mut state = budget_test_state(MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN);
        state
            .domain_fetch_counts
            .insert("127.0.0.1".to_string(), MAX_FETCH_PER_DOMAIN);
        let err = host_v2_fetch::<FetchError>(&mut state, url.clone()).unwrap_err();
        assert!(
            matches!(err, FetchError::RateLimited),
            "default per-domain cap must still block at MAX_FETCH_PER_DOMAIN"
        );

        // Budgeted caps for the same consumed count: the gate passes and the
        // call proceeds to the (closed-port) network attempt → NetworkError,
        // proving the raised cap (not a bypass) let it through.
        let (total, per_domain) = effective_fetch_caps(Some(30), "budget_test");
        let mut state = budget_test_state(total, per_domain);
        state
            .domain_fetch_counts
            .insert("127.0.0.1".to_string(), MAX_FETCH_PER_DOMAIN);
        let err = host_v2_fetch::<FetchError>(&mut state, url).unwrap_err();
        assert!(
            matches!(err, FetchError::NetworkError),
            "a 30-call budget must lift the per-domain gate for this execution"
        );
    }
}
