#![allow(unsafe_code)] // WASM FFI requires unsafe for host function calls and memory management
//! Aegis Runtime WASM Skill Core
//!
//! Shared boilerplate for all Aegis Runtime WASM skills. Provides:
//! - Host FFI bindings (fetch, payload I/O, output, file access)
//! - Thread-local output buffer management
//! - Typed payload reading helpers
//! - ProbeSignal for heartbeat-capable skills
//! - URL encoding utilities (RFC 3986 + query-string variant)
//!
//! # Usage
//! ```ignore
//! use aegis_runtime_wasm_core as core;
//!
//! #[unsafe(no_mangle)] // edition 2024 — bare #[no_mangle] is a hard error
//! pub extern "C" fn run() {
//!     let payload: MyPayload = match core::read_payload() {
//!         Ok(p) => p,
//!         Err(e) => { core::write_json(&serde_json::json!({"status":"error","error":e.to_string()})); return; }
//!     };
//!     let resp = core::fetch_url("https://example.com").unwrap();
//!     core::write_json(&serde_json::json!({"status":"ok","data": String::from_utf8_lossy(&resp)}));
//! }
//! ```

use std::cell::RefCell;

// Re-export for convenience so skills don't need separate serde/serde_json deps
// (they still need them in Cargo.toml for derive macros, but these re-exports
// help with type compatibility)
pub use serde;
pub use serde_json;

// ── Host FFI Bindings ───────────────────────────────────────────────────────
//
// Declared here once. The WASM linker strips imports that are never called,
// so skills that don't use network access won't import host_fetch etc.

// All FFI pointer types use i32, which is correct for wasm32 (32-bit address space).
// Prevent compilation on non-wasm32 targets where i32 would truncate 64-bit pointers.
#[cfg(not(target_pointer_width = "32"))]
compile_error!(
    "aegis_runtime_wasm_core is designed for wasm32 targets only. \
     i32 pointer types assume 32-bit address space. \
     Build with: cargo build --target wasm32-wasip1"
);

#[link(wasm_import_module = "aegis")]
unsafe extern "C" {
    pub fn host_fetch(url_ptr: i32, url_len: i32) -> i32;
    pub fn host_write_output(ptr: i32, len: i32);
    pub fn host_get_payload_len() -> i32;
    pub fn host_get_payload(dest_ptr: i32);
    pub fn host_get_fetch_response_len() -> i32;
    pub fn host_get_fetch_response(dest_ptr: i32) -> i32;
    pub fn host_read_file(path_ptr: i32, path_len: i32) -> i32;
    pub fn host_list_files() -> i32;
    pub fn host_store_state(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    pub fn host_load_state(key_ptr: i32, key_len: i32) -> i32;
    pub fn host_get_state_response_len() -> i32;
    pub fn host_get_state_response(dest_ptr: i32) -> i32;
}

// ── Output Buffer ───────────────────────────────────────────────────────────

thread_local! {
    static OUTPUT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Write raw bytes to the host output channel.
///
/// The bytes are pinned in linear memory so the host can read them.
/// Typically called via [`write_json`] instead.
pub fn write_output_bytes(bytes: Vec<u8>) {
    OUTPUT_BUF.with(|buf| {
        *buf.borrow_mut() = bytes;
        let b = buf.borrow();
        unsafe { host_write_output(b.as_ptr() as i32, b.len() as i32) };
    });
}

/// Serialize a value to JSON and write it to the host output channel.
///
/// If serialization fails (which should never happen for well-formed structs),
/// writes an error JSON `{"status":"error","error":"serialization failed"}` instead
/// of silently producing empty output.
pub fn write_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_vec(value) {
        Ok(bytes) => write_output_bytes(bytes),
        Err(_) => {
            // Fallback: emit a minimal error so the host doesn't receive silence.
            let fallback = br#"{"status":"error","error":"serialization failed"}"#;
            write_output_bytes(fallback.to_vec());
        }
    }
}

// ── Payload Reading ─────────────────────────────────────────────────────────

/// Read the raw payload bytes sent by the host.
///
/// Returns an empty `Vec` if the payload length is zero.
pub fn read_payload_bytes() -> Vec<u8> {
    let len = unsafe { host_get_payload_len() } as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len];
    unsafe { host_get_payload(buf.as_mut_ptr() as i32) };
    buf
}

/// Read and deserialize the host payload as a typed struct.
///
/// # Errors
/// Returns `serde_json::Error` if the payload is empty or malformed.
pub fn read_payload<T: serde::de::DeserializeOwned>() -> Result<T, serde_json::Error> {
    let bytes = read_payload_bytes();
    serde_json::from_slice(&bytes)
}

// ── Network Fetch ───────────────────────────────────────────────────────────

/// Fetch a URL via the host's HTTP proxy.
///
/// Returns the raw response bytes on success, or an error string on failure.
/// The host enforces domain whitelisting per the skill's security manifest.
/// On failure, logs the URL and host return code to stderr for diagnostics.
pub fn fetch_url(url: &str) -> Result<Vec<u8>, &'static str> {
    // No guest-side cap. Callers that already truncate their output should use
    // [`fetch_url_limited`] so a host body larger than the skill memory limit
    // returns an error instead of aborting inside `vec![0u8; len]`.
    fetch_url_limited(url, usize::MAX)
}

/// Fetch a URL, but refuse to allocate when the host body exceeds `max_bytes`.
///
/// The host will hand the guest a body of up to 10 MiB. Skill linear memory is
/// only a few megabytes and WASI panics abort, so `vec![0u8; len]` on an
/// oversized body becomes an uncatchable wasm trap. Checking the length first
/// turns that into `Err("response too large")`.
pub fn fetch_url_limited(url: &str, max_bytes: usize) -> Result<Vec<u8>, &'static str> {
    let rc = unsafe { host_fetch(url.as_ptr() as i32, url.len() as i32) };
    if rc != 0 {
        eprintln!("[aegis-wasm-core] fetch_url: host_fetch returned {rc} for {url}");
        return Err("host_fetch failed");
    }
    let len = unsafe { host_get_fetch_response_len() } as usize;
    if len > max_bytes {
        eprintln!(
            "[aegis-wasm-core] fetch_url: response of {len} bytes exceeds limit {max_bytes} for {url}"
        );
        return Err("response too large");
    }
    let mut buf = vec![0u8; len];
    let status = unsafe { host_get_fetch_response(buf.as_mut_ptr() as i32) };
    if status != 0 {
        eprintln!("[aegis-wasm-core] fetch_url: host_get_fetch_response returned {status}");
        return Err("host_get_fetch_response failed");
    }
    Ok(buf)
}

/// Fetch a URL and parse the response as a JSON string.
///
/// Convenience wrapper around [`fetch_url`] + UTF-8 conversion.
pub fn fetch_url_as_string(url: &str) -> Result<String, String> {
    let bytes = fetch_url(url).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Fetch a URL and parse the response as a `serde_json::Value`.
pub fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let bytes = fetch_url(url).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

// ── Auth-failure detection for skills calling host_fetch ──────────────────
//
// host_fetch (sandbox_host_fns.rs) returns a 401 response as a synthetic
// JSON body of shape:
//   {"status":"error","error":"HTTP 401: ...","error_kind":"auth_failure",
//    "retryable":false,"hint":"..."}
// with rc=0 (success). Skills that parse the response without checking
// for the top-level "error" key would silently treat it as data and
// return e.g. zero items. Use [`check_for_api_error`] inside every
// fetch_json wrapper so token expiry surfaces as a structured
// SkillError instead of a silent no-op.

/// Inspect a parsed JSON body for the host-injected auth-failure shape
/// or for a Google API top-level `error` object. Returns `Ok(())` if the
/// body looks like normal data, or a [`SkillError`] otherwise.
///
/// Recognized error shapes:
///
/// 1. `{ "status": "error", "error": "...", "error_kind": "auth_failure" }`
///    — synthesized by host_fetch when the upstream returned HTTP 401.
///    The skill should NOT treat this as data.
///
/// 2. `{ "error": { "code": N, "message": "..." } }` — Google's standard
///    API error envelope (used by Google / Calendar / Drive). 404 / 403 /
///    quota errors arrive in this shape with rc=0.
///
/// 3. `{ "error": "free-form string" }` (non-Google) — flagged as a
///    generic API error.
pub fn check_for_api_error(value: &serde_json::Value) -> Result<(), SkillError> {
    // Shape 1: host-synthesized auth_failure (status=error + error_kind set).
    let status_is_error = value
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("error"))
        .unwrap_or(false);
    let error_kind = value.get("error_kind").and_then(|v| v.as_str());
    if status_is_error && error_kind == Some("auth_failure") {
        let msg = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("authentication failed");
        let hint = value.get("hint").and_then(|v| v.as_str()).unwrap_or(
            "OAuth2 token may have expired. Run setup_google (action=\"setup\") to re-authenticate.",
        );
        return Err(
            SkillError::new(ErrorKind::AuthFailure, msg.to_string()).with_hint(hint.to_string())
        );
    }
    // Shape 1b: status=error but a different/missing error_kind — still
    // an error envelope from the host or upstream.
    if status_is_error && value.get("error").is_some() {
        let msg = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("upstream returned error");
        let kind = match error_kind {
            Some("rate_limit") => ErrorKind::RateLimit,
            Some("not_found") => ErrorKind::NotFound,
            Some(_) | None => ErrorKind::NetworkError,
        };
        return Err(SkillError::new(kind, msg.to_string()));
    }
    // Shape 2: Google API standard error object.
    if let Some(err) = value.get("error")
        && let Some(obj) = err.as_object()
    {
        let code = obj.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let message = obj
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        let kind = match code {
            401 => ErrorKind::AuthFailure,
            403 => ErrorKind::AuthFailure,
            404 => ErrorKind::NotFound,
            429 => ErrorKind::RateLimit,
            _ => ErrorKind::NetworkError,
        };
        let hint = if code == 401 || code == 403 {
            "OAuth2 token may have expired or lacks the required scope. Run setup_google (action=\"setup\") to re-authenticate."
        } else {
            "Verify the request parameters and account configuration"
        };
        return Err(SkillError::new(kind, format!("API error {code}: {message}")).with_hint(hint));
    }
    // Shape 3: free-form string error key (rare; non-Google).
    if let Some(err_str) = value.get("error").and_then(|v| v.as_str()) {
        return Err(SkillError::new(
            ErrorKind::NetworkError,
            format!("upstream error: {err_str}"),
        ));
    }
    Ok(())
}

// ── File Access (sandboxed) ─────────────────────────────────────────────────

/// Read a file from the sandboxed inbox directory via the host.
///
/// The host enforces path whitelisting per the skill's security manifest.
/// Returns file contents as bytes, or an error.
pub fn read_file(path: &str) -> Result<Vec<u8>, &'static str> {
    let rc = unsafe { host_read_file(path.as_ptr() as i32, path.len() as i32) };
    if rc != 0 {
        eprintln!("[aegis-wasm-core] read_file: host_read_file returned {rc} for {path}");
        return Err("host_read_file failed");
    }
    let len = unsafe { host_get_fetch_response_len() } as usize;
    let mut buf = vec![0u8; len];
    let status = unsafe { host_get_fetch_response(buf.as_mut_ptr() as i32) };
    if status != 0 {
        eprintln!("[aegis-wasm-core] read_file: host_get_fetch_response returned {status}");
        return Err("host_get_fetch_response failed");
    }
    Ok(buf)
}

/// List files in the sandboxed inbox directory.
///
/// Returns the JSON-encoded file listing from the host.
pub fn list_files() -> Result<Vec<u8>, &'static str> {
    let rc = unsafe { host_list_files() };
    if rc != 0 {
        eprintln!("[aegis-wasm-core] list_files: host_list_files returned {rc}");
        return Err("host_list_files failed");
    }
    let len = unsafe { host_get_fetch_response_len() } as usize;
    let mut buf = vec![0u8; len];
    let status = unsafe { host_get_fetch_response(buf.as_mut_ptr() as i32) };
    if status != 0 {
        eprintln!("[aegis-wasm-core] list_files: host_get_fetch_response returned {status}");
        return Err("host_get_fetch_response failed");
    }
    Ok(buf)
}

// ── State Persistence ───────────────────────────────────────────────────────

/// Load a value from persistent state by key.
///
/// Returns `T::default()` if the key does not exist (rc <= 0) or the response
/// is empty. Logs to stderr and returns `T::default()` if deserialization fails.
/// State is scoped per-skill by the host runtime.
pub fn load_state<T: serde::de::DeserializeOwned + Default>(key: &str) -> T {
    let result = unsafe { host_load_state(key.as_ptr() as i32, key.len() as i32) };
    if result <= 0 {
        return T::default();
    }
    let len = unsafe { host_get_state_response_len() } as usize;
    if len == 0 {
        return T::default();
    }
    let mut buf = vec![0u8; len];
    unsafe { host_get_state_response(buf.as_mut_ptr() as i32) };
    match serde_json::from_slice(&buf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[aegis-wasm-core] load_state({key}): deserialization failed: {e}");
            T::default()
        }
    }
}

/// Save a value to persistent state by key.
///
/// Serializes the value as JSON and stores it in the host's per-skill state.
/// Returns `true` on success, `false` on failure.
/// Logs to stderr on failure for diagnostics.
pub fn save_state<T: serde::Serialize>(key: &str, val: &T) -> bool {
    let Ok(json) = serde_json::to_string(val) else {
        eprintln!("[aegis-wasm-core] save_state({key}): serialization failed");
        return false;
    };
    let rc = unsafe {
        host_store_state(
            key.as_ptr() as i32,
            key.len() as i32,
            json.as_ptr() as i32,
            json.len() as i32,
        )
    };
    if rc != 0 {
        eprintln!("[aegis-wasm-core] save_state({key}): host_store_state returned {rc}");
    }
    rc == 0
}

// ── ProbeSignal (heartbeat skills) ──────────────────────────────────────────

/// Signal returned by heartbeat-capable skills to indicate whether
/// the orchestrator's LLM attention is needed.
#[derive(serde::Serialize)]
pub struct ProbeSignal {
    pub require_llm_attention: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_payload: Option<String>,
}

impl ProbeSignal {
    /// No alert needed — the skill checked and everything is normal.
    pub fn quiet() -> Self {
        Self {
            require_llm_attention: false,
            event_payload: None,
        }
    }

    /// Alert — something requires LLM attention. Include a human-readable payload.
    pub fn alert(payload: impl Into<String>) -> Self {
        Self {
            require_llm_attention: true,
            event_payload: Some(payload.into()),
        }
    }

    /// Serialize and write this signal to the host output channel.
    pub fn write(&self) {
        write_json(self);
    }
}

// ── Structured Error Type ─────────────────────────────────────────────────

/// Categorized error kinds for downstream routing and retry logic.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Input validation failure (bad payload, missing fields)
    InvalidInput,
    /// Network/HTTP failure (timeout, DNS, connection refused)
    NetworkError,
    /// Authentication failure (expired token, missing credentials)
    AuthFailure,
    /// The requested resource was not found
    NotFound,
    /// Rate limit or quota exceeded
    RateLimit,
    /// Parsing failure (JSON, HTML, etc.)
    ParseError,
    /// Internal logic error (unexpected state)
    InternalError,
}

/// Structured skill error producing the host-expected JSON format:
/// `{status, error, error_kind, retryable, hint}`
#[derive(Debug, Clone)]
pub struct SkillError {
    pub message: String,
    pub kind: ErrorKind,
    pub retryable: bool,
    pub hint: Option<String>,
    pub action: Option<String>,
}

impl SkillError {
    /// Create a new error with the given kind.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        let retryable = matches!(kind, ErrorKind::NetworkError | ErrorKind::RateLimit);
        Self {
            message: message.into(),
            kind,
            retryable,
            hint: None,
            action: None,
        }
    }

    /// Add a user-facing hint for recovery guidance.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Add an action context (for multi-action skills).
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Write this error to the host output channel as structured JSON.
    pub fn write(&self) {
        let mut obj = serde_json::json!({
            "status": "error",
            "error": self.message,
            "error_kind": self.kind,
            "retryable": self.retryable,
        });
        if let Some(ref hint) = self.hint {
            obj["hint"] = serde_json::json!(hint);
        }
        if let Some(ref action) = self.action {
            obj["action"] = serde_json::json!(action);
        }
        write_json(&obj);
    }
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<String> for SkillError {
    fn from(msg: String) -> Self {
        Self::new(ErrorKind::InternalError, msg)
    }
}

impl From<serde_json::Error> for SkillError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorKind::ParseError, format!("JSON error: {e}"))
    }
}

// ── Error Output Helpers (legacy, use SkillError for new code) ────────────

// LEGACY — use SkillError::new(...).write() for new code.
// Will be #[deprecated] once all existing skills are migrated.

/// Write a minimal error response: `{"status":"error","error":"<msg>"}`.
///
/// **Deprecated pattern.** Use [`SkillError::new`] for new code — it includes
/// `error_kind`, `retryable`, and `hint` fields expected by the host.
pub fn write_error(error: &str) {
    write_json(&serde_json::json!({"status": "error", "error": error}));
}

/// Write an error response with an action field.
///
/// **Deprecated pattern.** Use [`SkillError::new`]`.with_action(...)` instead.
pub fn write_action_error(action: &str, error: &str) {
    write_json(&serde_json::json!({"status": "error", "action": action, "error": error}));
}

// ── URL Encoding ────────────────────────────────────────────────────────────

const HEX_UPPER: [u8; 16] = *b"0123456789ABCDEF";

/// RFC 3986 percent-encoding. Preserves unreserved characters (A-Z, a-z, 0-9, `-`, `_`, `.`, `~`).
///
/// Use this for path segments and general URI components.
pub fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX_UPPER[(byte >> 4) as usize]));
                encoded.push(char::from(HEX_UPPER[(byte & 0x0F) as usize]));
            }
        }
    }
    encoded
}

/// Query-string encoding. Like [`url_encode`] but encodes spaces as `+`.
///
/// Use this for query parameter values in `application/x-www-form-urlencoded` format.
pub fn url_encode_query(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX_UPPER[(byte >> 4) as usize]));
                encoded.push(char::from(HEX_UPPER[(byte & 0x0F) as usize]));
            }
        }
    }
    encoded
}

// ── Allocator Export ────────────────────────────────────────────────────────

/// Standard WASM allocator export for host-side memory allocation.
///
/// The host calls this to allocate memory in the guest's linear memory
/// before writing payload data. Capped at 4 MiB for safety.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(size: i32) -> i32 {
    if size <= 0 || size > 4 * 1024 * 1024 {
        return 0;
    }
    let Ok(layout) = std::alloc::Layout::from_size_align(size as usize, 1) else {
        return 0;
    };
    unsafe { std::alloc::alloc(layout) as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_for_api_error_passes_for_normal_data() {
        let v = serde_json::json!({"messages": [{"id": "abc"}], "resultSizeEstimate": 1});
        assert!(check_for_api_error(&v).is_ok());
    }

    #[test]
    fn check_for_api_error_passes_for_empty_object() {
        let v = serde_json::json!({});
        assert!(check_for_api_error(&v).is_ok());
    }

    #[test]
    fn check_for_api_error_detects_host_auth_failure_envelope() {
        // Mirrors the body that sandbox_host_fns.rs synthesizes on HTTP 401.
        let v = serde_json::json!({
            "status": "error",
            "error": "HTTP 401: authentication failed for www.googleapis.com. Check OAuth2 credentials.",
            "error_kind": "auth_failure",
            "retryable": false,
            "hint": "OAuth2 token may have expired or credentials are invalid. Try re-authenticating via setup_google.",
        });
        let err = check_for_api_error(&v).expect_err("should reject auth_failure body");
        assert_eq!(err.kind, ErrorKind::AuthFailure);
        assert!(
            err.message.contains("HTTP 401"),
            "message should propagate; got: {}",
            err.message
        );
    }

    #[test]
    fn check_for_api_error_detects_google_error_object_401() {
        let v = serde_json::json!({
            "error": {
                "code": 401,
                "message": "Invalid Credentials",
                "status": "UNAUTHENTICATED",
            }
        });
        let err = check_for_api_error(&v).expect_err("should reject 401 google error");
        assert_eq!(err.kind, ErrorKind::AuthFailure);
        assert!(err.message.contains("401"));
        assert!(err.message.contains("Invalid Credentials"));
    }

    #[test]
    fn check_for_api_error_detects_google_error_object_404() {
        let v = serde_json::json!({
            "error": {"code": 404, "message": "Requested entity was not found."}
        });
        let err = check_for_api_error(&v).expect_err("should reject 404 google error");
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    #[test]
    fn check_for_api_error_detects_google_error_object_429() {
        let v = serde_json::json!({
            "error": {"code": 429, "message": "Rate limit exceeded."}
        });
        let err = check_for_api_error(&v).expect_err("should reject 429 google error");
        assert_eq!(err.kind, ErrorKind::RateLimit);
    }

    #[test]
    fn check_for_api_error_detects_freeform_error_string() {
        let v = serde_json::json!({"error": "something went wrong"});
        let err = check_for_api_error(&v).expect_err("should reject string error");
        assert_eq!(err.kind, ErrorKind::NetworkError);
    }
}
