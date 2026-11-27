//! Runtime-reloadable log level control.
//!
//! Initializes tracing with a `reload::Layer<EnvFilter>` so that the
//! log filter can be changed at runtime without restarting the process.

use std::sync::OnceLock;

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, reload, util::SubscriberInitExt};

/// Type alias for the reload handle (avoids leaking complex generics).
type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

/// Global handle for reloading the log filter at runtime.
static FILTER_HANDLE: OnceLock<FilterHandle> = OnceLock::new();

/// Initialize tracing with a reloadable `EnvFilter`.
///
/// Must be called exactly once at startup (before any tracing macros).
/// Respects `RUST_LOG` env var; defaults to `"info"`.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let (filter_layer, handle) = reload::Layer::new(filter);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt::layer().with_target(true).with_writer(std::io::stderr))
        .init();

    let _ = FILTER_HANDLE.set(handle);
}

/// Get the current log filter directive string.
pub fn current_level() -> String {
    match FILTER_HANDLE.get() {
        Some(handle) => {
            let mut result = String::new();
            let _ = handle.modify(|filter| {
                result = format!("{filter}");
            });
            if result.is_empty() {
                "info".to_string()
            } else {
                result
            }
        }
        None => "info (handle not initialized)".to_string(),
    }
}

/// Reload the log filter with a new directive string.
///
/// Accepts standard `RUST_LOG` syntax:
/// - Simple level: `"debug"`, `"warn"`, `"trace"`
/// - Module-specific: `"info,aegis_runtime_core::orchestrator=debug"`
/// - Combined: `"warn,aegis_runtime_core=debug,hyper=error"`
pub fn set_level(directive: &str) -> Result<(), String> {
    let handle = FILTER_HANDLE
        .get()
        .ok_or_else(|| "log filter handle not initialized".to_string())?;

    let new_filter =
        EnvFilter::try_new(directive).map_err(|e| format!("invalid filter directive: {e}"))?;

    handle
        .reload(new_filter)
        .map_err(|e| format!("failed to reload filter: {e}"))?;

    tracing::info!(directive, "Log level changed at runtime");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_directives() {
        // These should parse without error
        assert!(EnvFilter::try_new("info").is_ok());
        assert!(EnvFilter::try_new("debug").is_ok());
        assert!(EnvFilter::try_new("warn,aegis_runtime_core=debug").is_ok());
        assert!(EnvFilter::try_new("error,aegis_runtime_core::orchestrator=trace").is_ok());
    }

    #[test]
    fn invalid_directive_rejected() {
        // EnvFilter rejects malformed syntax (not unknown module names, which are valid targets)
        assert!(EnvFilter::try_new("=invalid=syntax==").is_err());
    }
}
