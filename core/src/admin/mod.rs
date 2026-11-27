use std::collections::{BTreeMap, HashMap, HashSet, VecDeque, hash_map::DefaultHasher};
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::{self};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse as axum_sse;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tower_http::services::ServeDir;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushError, WebPushMessageBuilder,
};

use crate::credential_store::CredentialStore;
use crate::google_auth::{self, GoogleAuthManager};
use crate::identity::AgentIdentity;
use crate::memory::{MemoryHandle, STATUS_DEACTIVATED};
use crate::sandbox::SandboxEngine;
use crate::settings::RuntimeSettings;
use crate::utils::truncate_chars;
use crate::web_console_auth;
use crate::webapp_auth;

mod assistant;
mod console_bridge;
mod console_handlers;
mod debug_avatar;
mod diagnostics_routes;
mod inbox_projection;
mod ops_endpoints;
mod remote_routes;
mod server;
mod settings_routes;
mod vnext_projections;
mod vnext_routes;
mod webapp_gateway;

// Glob visibility mirrors each submodule's most-public item (unused_imports
// warns when a `pub use` glob re-exports nothing actually `pub`): `server`
// and `debug_avatar` carry the module's historical `pub` API; the two
// projection modules expose the historical `pub(crate)` helpers; everything
// else is admin-internal (`pub(super)`), imported here privately so
// `use super::*` in sibling submodules and the `tests`
// children keeps resolving exactly the names the old single-file
// namespace had.
pub use debug_avatar::*;
pub use server::*;

pub(crate) use inbox_projection::*;
pub(crate) use vnext_projections::*;

use assistant::*;
use console_bridge::*;
use console_handlers::*;
use diagnostics_routes::*;
use ops_endpoints::*;
use settings_routes::*;
use vnext_routes::*;
use webapp_gateway::*;

pub use settings_routes::{CodexCliLoginStatus, codex_cli_login_status_at};

/// Catalog loader reuses the Codex login/list helpers without exposing the
/// whole settings_routes surface as `pub`.
pub(crate) async fn codex_cli_login_status_for_catalog(
    state: &AdminState,
) -> settings_routes::CodexCliLoginStatus {
    settings_routes::codex_cli_login_status(state).await
}

pub(crate) async fn codex_cli_model_options_for_catalog(
    state: &AdminState,
) -> (Vec<serde_json::Value>, &'static str) {
    settings_routes::codex_cli_model_options(state).await
}

#[cfg(test)]
#[path = "../admin_tests/mod.rs"]
pub(crate) mod tests;

/// Bind address for the public OAuth callback listener. Loopback by default:
/// the callback is reached through the cloudflared tunnel on the same host,
/// so there is no reason to expose it on every interface. Set
/// `OAUTH_CALLBACK_BIND=0.0.0.0` to restore the old behaviour.
pub fn oauth_callback_bind_addr(override_env: Option<&str>) -> String {
    match override_env.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => "127.0.0.1".to_string(),
    }
}

/// Host to print in the startup on-ramp URLs (`[Web Console] http://…/ui/`
/// and the local login link) for a listener bound to `bind_addr`. An
/// unspecified bind (`0.0.0.0`, `::`) is reachable on loopback and browsers
/// refuse `http://0.0.0.0/`, so it prints as `127.0.0.1`; any concrete
/// address prints as bound.
pub fn web_console_display_host(bind_addr: &str) -> String {
    match bind_addr.trim() {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod oauth_bind_tests {
    #[test]
    fn oauth_callback_binds_loopback_unless_overridden() {
        assert_eq!(super::oauth_callback_bind_addr(None), "127.0.0.1");
        assert_eq!(super::oauth_callback_bind_addr(Some("")), "127.0.0.1");
        assert_eq!(super::oauth_callback_bind_addr(Some("0.0.0.0")), "0.0.0.0");
    }

    #[test]
    fn web_console_display_host_prints_loopback_for_unspecified_binds() {
        assert_eq!(super::web_console_display_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(super::web_console_display_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(super::web_console_display_host("::"), "127.0.0.1");
        assert_eq!(super::web_console_display_host("10.0.0.5"), "10.0.0.5");
        assert_eq!(super::web_console_display_host("aegis.lan"), "aegis.lan");
    }
}
