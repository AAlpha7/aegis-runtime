//! Shared test helpers for E2E / integration tests.
//!
//! Live-test credentials are **opt-in**: `load_test_env()` reads
//! `<repo root>/.env.test` only when `AEGIS_LIVE_TESTS=1` is set, and never
//! touches `/opt/aegis-runtime/shared/.env` (the production secrets). A stray
//! `cargo test -- --ignored` on the deploy host therefore cannot send the
//! real API key upstream. Variables already exported in the environment are
//! always honoured and never overwritten.
//!
//! Usage in test files:
//!   mod common;
//!   // then call common::load_test_env() or common::get_env("KEY")

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Once};

use aegis_runtime_core::sandbox::SandboxEngine;

static LOAD_ENV: Once = Once::new();

/// A shared Wasmtime engine, created once and reused by all tests in the same
/// binary.  Avoids the ~400-800 ms JIT compiler init per `SandboxEngine::new()`.
static SHARED_ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(|| {
    SandboxEngine::default_engine().expect("failed to create shared Wasmtime engine")
});

/// Return a fresh `SandboxEngine` backed by the shared Wasmtime engine.
/// Each call gets its own skill map / replay guard — only the JIT init is shared.
#[allow(dead_code)]
pub fn shared_sandbox() -> SandboxEngine {
    SandboxEngine::from_engine(SHARED_ENGINE.clone())
        .expect("failed to create sandbox from shared engine")
}

/// Arc-wrapped variant for APIs that require `Arc<SandboxEngine>`.
#[allow(dead_code)]
pub fn shared_sandbox_arc() -> Arc<SandboxEngine> {
    Arc::new(shared_sandbox())
}

/// Environment variable that opts a test process into live-credential
/// loading. Anything other than exactly `1` leaves the harness inert.
pub const LIVE_TESTS_OPT_IN: &str = "AEGIS_LIVE_TESTS";

/// Name of the credentials file the harness may read (repo root only).
pub const LIVE_ENV_FILE: &str = ".env.test";

/// Repository root (parent of `core/`), derived from the manifest dir so the
/// path is stable regardless of the current working directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core/ lives inside the repo root")
        .to_path_buf()
}

/// Decide which env file (if any) the harness may load, given the raw value
/// of `AEGIS_LIVE_TESTS`. Pure so it can be tested without mutating the
/// process environment:
///   - `Some("1")` → `Some(<repo root>/.env.test)`
///   - anything else (unset, `0`, `true`, ...) → `None`
///
/// There is deliberately no fallback location: the production env under
/// `/opt/aegis-runtime/shared/.env` must never be read by tests.
pub fn live_env_file(opt_in: Option<&str>) -> Option<PathBuf> {
    match opt_in {
        Some("1") => Some(repo_root().join(LIVE_ENV_FILE)),
        _ => None,
    }
}

/// Whether the current process opted into live tests (`AEGIS_LIVE_TESTS=1`).
#[allow(dead_code)]
pub fn live_tests_enabled() -> bool {
    std::env::var(LIVE_TESTS_OPT_IN).as_deref() == Ok("1")
}

/// Load live-test environment variables (idempotent).
///
/// Only when `AEGIS_LIVE_TESTS=1`: reads `<repo root>/.env.test` without
/// overwriting variables already present in the environment. Without the
/// opt-in this is a no-op — callers that need a key then see whatever the
/// shell exported, or `get_env` panics with instructions.
pub fn load_test_env() {
    LOAD_ENV.call_once(|| {
        let opt_in = std::env::var(LIVE_TESTS_OPT_IN).ok();
        let Some(path) = live_env_file(opt_in.as_deref()) else {
            return;
        };
        match dotenvy::from_path(&path) {
            Ok(()) => eprintln!("[common] loaded live-test env from {}", path.display()),
            Err(e) => eprintln!(
                "[common] {LIVE_TESTS_OPT_IN}=1 but {} could not be loaded: {e}",
                path.display()
            ),
        }
    });
}

/// Get an environment variable, loading the opt-in live env file first.
/// Panics with a clear message if the variable is not set.
#[allow(dead_code)]
pub fn get_env(key: &str) -> String {
    load_test_env();
    std::env::var(key).unwrap_or_else(|_| {
        panic!(
            "{key} not set. Live tests are opt-in:\n  \
             - export {key} in the environment, or\n  \
             - put it in {} and run with {LIVE_TESTS_OPT_IN}=1\n  \
             (the test harness never reads /opt/aegis-runtime/shared/.env)",
            repo_root().join(LIVE_ENV_FILE).display()
        )
    })
}

static INIT_TRACING: Once = Once::new();

/// Initialize tracing for tests (idempotent).
#[allow(dead_code)]
pub fn init_tracing() {
    INIT_TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter("aegis_runtime_core=info")
            .with_test_writer()
            .init();
    });
}
