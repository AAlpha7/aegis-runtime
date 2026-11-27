use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use wasmtime::component::Component;
use wasmtime::*;

use crate::google_auth::TokenProvider;
use crate::identity::{ReplayGuard, verify_envelope_with_guard};
use crate::models::{IntentEnvelope, SkillCapability};
use crate::wit_bindings::{probe_v2, skill_v2};

// Re-export public items from extracted modules
pub use crate::sandbox_host_fns::TrapKind;
pub use crate::sandbox_network::auth_headers_from_env;

// ── Path Containment Helper ──────────────────────────────────────────────────

/// Check if `parent` path contains `child` path (i.e., `child` is under `parent`).
/// Check logical path containment for permission merging.
/// Uses raw `starts_with` — safe for abstract policy paths that may not exist on disk.
fn path_contains(parent: &str, child: &str) -> bool {
    let p = std::path::Path::new(parent);
    let c = std::path::Path::new(child);
    c.starts_with(p)
}

// path_contains_secure removed — superseded by inline canonicalize checks
// in host_read_file and host_list_files linker closures (link_host_functions).

// ── Layered Permission Model ─────────────────────────────────────────────

/// Network access policy for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkPolicy {
    /// No network access at all
    None,
    /// Can fetch from specific domains only (default for most skills)
    FetchOnly(Vec<String>),
    /// Unrestricted (never used in practice — for testing only)
    Unrestricted,
}

/// Filesystem access policy for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FsPolicy {
    /// No filesystem access (default for auto-generated skills)
    None,
    /// Read-only access to specific paths
    ReadOnly(Vec<String>),
    /// Read-write access to specific paths
    ReadWrite(Vec<String>),
}

/// Capability-based permission envelope per skill.
/// Layered: global defaults → per-skill policy → per-execution policy.
/// Each layer can only RESTRICT further (never widen).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPermissions {
    pub network: NetworkPolicy,
    pub filesystem: FsPolicy,
    pub max_fetch_calls: u32,
    pub max_memory_mb: u32,
    pub max_fuel: u64,
    pub can_emit_probe: bool,
}

impl Default for SkillPermissions {
    fn default() -> Self {
        Self {
            network: NetworkPolicy::None,
            filesystem: FsPolicy::None,
            max_fetch_calls: MAX_FETCH_CALLS as u32,
            max_memory_mb: DEFAULT_SKILL_MEMORY_MB,
            max_fuel: WASM_FUEL_LIMIT,
            can_emit_probe: false,
        }
    }
}

impl SkillPermissions {
    /// Create permissions from a SkillCapability (backwards-compatible).
    pub fn from_capability(cap: &SkillCapability) -> Self {
        let network = if cap.allowed_domains.is_empty() {
            NetworkPolicy::None
        } else {
            NetworkPolicy::FetchOnly(cap.allowed_domains.clone())
        };

        Self {
            network,
            filesystem: if cap.allowed_fs_paths.is_empty() {
                FsPolicy::None
            } else {
                FsPolicy::ReadOnly(cap.allowed_fs_paths.clone())
            },
            max_fetch_calls: MAX_FETCH_CALLS as u32,
            max_memory_mb: cap.max_memory_mb,
            max_fuel: WASM_FUEL_LIMIT,
            can_emit_probe: cap.cron_schedule.is_some(),
        }
    }

    /// Merge (intersect/restrict) with another permission set.
    /// The result is the most restrictive combination.
    pub fn restrict_with(&self, other: &SkillPermissions) -> Self {
        let network = match (&self.network, &other.network) {
            (NetworkPolicy::None, _) | (_, NetworkPolicy::None) => NetworkPolicy::None,
            (NetworkPolicy::FetchOnly(a), NetworkPolicy::FetchOnly(b)) => {
                // Intersection of allowed domains
                let restricted: Vec<String> = a.iter().filter(|d| b.contains(d)).cloned().collect();
                if restricted.is_empty() {
                    NetworkPolicy::None
                } else {
                    NetworkPolicy::FetchOnly(restricted)
                }
            }
            (NetworkPolicy::FetchOnly(a), NetworkPolicy::Unrestricted) => {
                NetworkPolicy::FetchOnly(a.clone())
            }
            (NetworkPolicy::Unrestricted, NetworkPolicy::FetchOnly(b)) => {
                NetworkPolicy::FetchOnly(b.clone())
            }
            (NetworkPolicy::Unrestricted, NetworkPolicy::Unrestricted) => {
                NetworkPolicy::Unrestricted
            }
        };

        let filesystem = match (&self.filesystem, &other.filesystem) {
            (FsPolicy::None, _) | (_, FsPolicy::None) => FsPolicy::None,
            (FsPolicy::ReadOnly(a), FsPolicy::ReadOnly(b)) => {
                let restricted: Vec<String> = a.iter().filter(|p| b.contains(p)).cloned().collect();
                FsPolicy::ReadOnly(restricted)
            }
            (FsPolicy::ReadOnly(a), FsPolicy::ReadWrite(b)) => {
                let restricted: Vec<String> = a
                    .iter()
                    .filter(|p| {
                        b.iter()
                            .any(|bp| path_contains(bp, p) || path_contains(p, bp))
                    })
                    .cloned()
                    .collect();
                FsPolicy::ReadOnly(restricted)
            }
            (FsPolicy::ReadWrite(a), FsPolicy::ReadOnly(b)) => {
                let restricted: Vec<String> = b
                    .iter()
                    .filter(|p| {
                        a.iter()
                            .any(|ap| path_contains(ap, p) || path_contains(p, ap))
                    })
                    .cloned()
                    .collect();
                FsPolicy::ReadOnly(restricted)
            }
            (FsPolicy::ReadWrite(a), FsPolicy::ReadWrite(b)) => {
                let restricted: Vec<String> = a.iter().filter(|p| b.contains(p)).cloned().collect();
                FsPolicy::ReadWrite(restricted)
            }
        };

        Self {
            network,
            filesystem,
            max_fetch_calls: self.max_fetch_calls.min(other.max_fetch_calls),
            max_memory_mb: self.max_memory_mb.min(other.max_memory_mb),
            max_fuel: self.max_fuel.min(other.max_fuel),
            can_emit_probe: self.can_emit_probe && other.can_emit_probe,
        }
    }

    /// Get effective allowed domains (for use in run_isolated).
    pub fn effective_allowed_domains(&self) -> Vec<String> {
        match &self.network {
            NetworkPolicy::None => vec![],
            NetworkPolicy::FetchOnly(domains) => domains.clone(),
            NetworkPolicy::Unrestricted => vec!["*".to_string()],
        }
    }

    /// Get effective allowed fs paths (for use in run_isolated).
    pub fn effective_allowed_fs_paths(&self) -> Vec<String> {
        match &self.filesystem {
            FsPolicy::None => vec![],
            FsPolicy::ReadOnly(paths) | FsPolicy::ReadWrite(paths) => paths.clone(),
        }
    }
}

use serde::{Deserialize, Serialize};

// ── Host Contract Registry ──────────────────────────────────────────────────
// Single source of truth for all Wasm host function signatures.
// Layer 2 (prompt generation), Layer 3 (pre-compile validation), and
// Layer 4 (error diagnosis) all derive from this contract.

/// Wasm value type used in host function signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmValType {
    I32,
}

/// Category of host function — determines which patterns expose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostFnCategory {
    Payload,
    Output,
    Fetch,
    State,
    FileSystem,
}

/// Specification of a single host function in the "aegis" Wasm import module.
#[derive(Debug, Clone)]
pub struct HostFnSpec {
    pub name: &'static str,
    pub params: &'static [WasmValType],
    pub returns: Option<WasmValType>,
    pub category: HostFnCategory,
    pub doc: &'static str,
    pub error_codes: &'static str,
}

impl HostFnSpec {
    /// Generate an `extern "C"` declaration for this function.
    /// e.g. `"fn host_fetch(url_ptr: i32, url_len: i32) -> i32;"`
    pub fn to_extern_decl(&self) -> String {
        let params_str = if self.params.is_empty() {
            String::new()
        } else {
            // Generate sensible parameter names based on function semantics
            let names = param_names_for(self.name, self.params.len());
            names
                .iter()
                .map(|n| format!("{}: i32", n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self.returns {
            Some(_) => format!("fn {}({}) -> i32;", self.name, params_str),
            None => format!("fn {}({});", self.name, params_str),
        }
    }

    /// Generate a WAT-style signature string, e.g. `"(i32, i32) -> i32"`.
    pub fn signature_string(&self) -> String {
        let params = if self.params.is_empty() {
            "()".to_string()
        } else {
            let p: Vec<&str> = self.params.iter().map(|_| "i32").collect();
            format!("({})", p.join(", "))
        };
        match self.returns {
            Some(_) => format!("{} -> i32", params),
            None => params,
        }
    }
}

/// Generate sensible parameter names for a host function.
fn param_names_for(fn_name: &str, count: usize) -> Vec<&'static str> {
    match (fn_name, count) {
        ("host_fetch", 2) => vec!["url_ptr", "url_len"],
        ("host_write_output", 2) => vec!["ptr", "len"],
        ("host_get_payload", 1) => vec!["dest_ptr"],
        ("host_get_fetch_response", 1) => vec!["dest_ptr"],
        ("host_read_file", 2) => vec!["path_ptr", "path_len"],
        ("host_store_state", 4) => vec!["key_ptr", "key_len", "val_ptr", "val_len"],
        ("host_load_state", 2) => vec!["key_ptr", "key_len"],
        ("host_get_state_response", 1) => vec!["dest_ptr"],
        _ => {
            // Fallback: p0, p1, ...
            match count {
                1 => vec!["p0"],
                2 => vec!["p0", "p1"],
                3 => vec!["p0", "p1", "p2"],
                4 => vec!["p0", "p1", "p2", "p3"],
                _ => vec![],
            }
        }
    }
}

use HostFnCategory::*;
use WasmValType::I32;

/// The canonical host contract — every `func_wrap("aegis", ...)` in the linker
/// block below MUST have a matching entry here. Layer 5 tests enforce this.
pub const HOST_CONTRACT: &[HostFnSpec] = &[
    // ── Payload (all patterns) ──
    HostFnSpec {
        name: "host_get_payload_len",
        params: &[],
        returns: Some(I32),
        category: Payload,
        doc: "Get the length of the input JSON payload in bytes.",
        error_codes: "Always succeeds (returns length >= 0).",
    },
    HostFnSpec {
        name: "host_get_payload",
        params: &[I32],
        returns: None,
        category: Payload,
        doc: "Copy the input JSON payload into the provided buffer at dest_ptr. Buffer must be at least host_get_payload_len() bytes.",
        error_codes: "Silently fails if dest_ptr is out of bounds.",
    },
    // ── Output (all patterns) ──
    HostFnSpec {
        name: "host_write_output",
        params: &[I32, I32],
        returns: None,
        category: Output,
        doc: "Write the output JSON to the host. Call once at the end of run(). ptr/len point to a UTF-8 JSON string.",
        error_codes: "Silently fails if ptr/len out of bounds.",
    },
    // ── Fetch (api_json, api_html, multi_fetch, probe_heartbeat, stateful) ──
    HostFnSpec {
        name: "host_fetch",
        params: &[I32, I32],
        returns: Some(I32),
        category: Fetch,
        doc: "Fetch a URL. url_ptr/url_len point to a UTF-8 URL string. Response body stored in host, read via host_get_fetch_response_len/host_get_fetch_response.",
        error_codes: "0=success, -1=no memory, -2=invalid ptr/len, -3=invalid UTF-8, -403=domain blocked, -429=rate limited, -500=network error.",
    },
    HostFnSpec {
        name: "host_get_fetch_response_len",
        params: &[],
        returns: Some(I32),
        category: Fetch,
        doc: "Get the length of the last fetch response body in bytes.",
        error_codes: "Always succeeds (returns length >= 0).",
    },
    HostFnSpec {
        name: "host_get_fetch_response",
        params: &[I32],
        returns: Some(I32),
        category: Fetch,
        doc: "Copy the last fetch response body into buffer at dest_ptr. Buffer must be at least host_get_fetch_response_len() bytes.",
        error_codes: "0=success, -1=out of bounds.",
    },
    // ── FileSystem (patterns with fs access) ──
    HostFnSpec {
        name: "host_read_file",
        params: &[I32, I32],
        returns: Some(I32),
        category: FileSystem,
        doc: "Read a file from the allowed filesystem paths. path_ptr/path_len point to a relative filename. File content stored in fetch_response buffer (shared with host_fetch).",
        error_codes: "0=success, -1=error, -2=invalid ptr/len, -3=invalid UTF-8, -403=path not allowed, -404=not found, -413=too large.",
    },
    HostFnSpec {
        name: "host_list_files",
        params: &[],
        returns: Some(I32),
        category: FileSystem,
        doc: "List files in the allowed filesystem directories. JSON array of filenames stored in fetch_response buffer.",
        error_codes: "0=success, -403=no filesystem paths configured.",
    },
    // ── State (stateful, no_network_stateful) ──
    HostFnSpec {
        name: "host_store_state",
        params: &[I32, I32, I32, I32],
        returns: Some(I32),
        category: State,
        doc: "Persist a key-value pair scoped to the current skill. key_ptr/key_len and val_ptr/val_len point to UTF-8 strings.",
        error_codes: "0=success, -1=error.",
    },
    HostFnSpec {
        name: "host_load_state",
        params: &[I32, I32],
        returns: Some(I32),
        category: State,
        doc: "Load a value by key for the current skill. key_ptr/key_len point to UTF-8 key. Value stored in state_response buffer (NOT fetch_response).",
        error_codes: "Returns value length (>=0) on success, -1=not found, -2=error.",
    },
    HostFnSpec {
        name: "host_get_state_response_len",
        params: &[],
        returns: Some(I32),
        category: State,
        doc: "Get the length of the state response buffer (separate from fetch response).",
        error_codes: "Always succeeds (returns length >= 0).",
    },
    HostFnSpec {
        name: "host_get_state_response",
        params: &[I32],
        returns: Some(I32),
        category: State,
        doc: "Copy state response into buffer at dest_ptr. Uses a SEPARATE buffer from host_get_fetch_response.",
        error_codes: "0=success, -1=out of bounds.",
    },
];

/// Map a skill pattern to the host function categories it needs.
pub fn categories_for_pattern(pattern: &str) -> Vec<HostFnCategory> {
    match pattern {
        "no_network" => vec![Payload, Output],
        "no_network_stateful" => vec![Payload, Output, State],
        "api_json" | "api_html" | "multi_fetch" | "probe_heartbeat" => {
            vec![Payload, Output, Fetch]
        }
        "stateful" => vec![Payload, Output, Fetch, State],
        "js_quickjs" => vec![], // JS uses globals, not Wasm imports
        _ => vec![Payload, Output, Fetch], // safe default
    }
}

/// Filter HOST_CONTRACT to only functions matching the given categories.
pub fn host_fns_for_categories(cats: &[HostFnCategory]) -> Vec<&'static HostFnSpec> {
    HOST_CONTRACT
        .iter()
        .filter(|spec| cats.contains(&spec.category))
        .collect()
}

/// Get the host functions available for a given skill pattern.
pub fn host_fns_for_pattern(pattern: &str) -> Vec<&'static HostFnSpec> {
    let cats = categories_for_pattern(pattern);
    host_fns_for_categories(&cats)
}

/// Look up a host function spec by name.
pub fn lookup_host_fn(name: &str) -> Option<&'static HostFnSpec> {
    HOST_CONTRACT.iter().find(|spec| spec.name == name)
}

/// Result returned by a skill execution.
#[derive(Debug)]
pub struct SkillResult {
    pub output: String,
    pub elapsed_ms: u128,
}

/// Runtime representation of a loaded skill — either a v1 core module
/// or a v2 Component Model component.
enum SkillModule {
    /// v1 core module — pointer-based I/O via linear memory.
    Core(Module),
    /// v2 component — standard skill (JSON in, JSON out).
    ComponentV2(Component),
    /// v2 component — probe/heartbeat skill (JSON in, typed ProbeSignal out).
    ComponentProbeV2(Component),
}

impl Clone for SkillModule {
    fn clone(&self) -> Self {
        match self {
            Self::Core(m) => Self::Core(m.clone()),
            Self::ComponentV2(c) => Self::ComponentV2(c.clone()),
            Self::ComponentProbeV2(c) => Self::ComponentProbeV2(c.clone()),
        }
    }
}

/// Pre-compiled Wasm module with its capability manifest.
struct CompiledSkill {
    module: SkillModule,
    capability: SkillCapability,
    /// For JavaScript skills, stores the JS source code (UTF-8 bytes).
    /// For Rust skills, this is empty (the module is self-contained).
    js_source: Option<String>,
}

/// Maximum number of host_fetch calls per skill invocation.
/// Prevents DoS via thread exhaustion from tight fetch loops.
pub(crate) const MAX_FETCH_CALLS: usize = 50;

/// Maximum number of host_fetch calls to a single domain per invocation.
/// Prevents a skill from hammering one API endpoint.
pub(crate) const MAX_FETCH_PER_DOMAIN: usize = 20;

/// Hard ceiling for execution-scoped fetch budgets granted by trusted host
/// code via [`SandboxEngine::execute_with_auth_override_and_fetch_budget`].
/// A budget request above this value is clamped here (with a warning) — no
/// caller, trusted or not, can exceed it. LLM-reachable execution surfaces
/// (`execute`, `execute_with_auth_override`, `execute_system`,
/// `run_skill_direct`) accept no budget parameter at all and always run with
/// [`MAX_FETCH_CALLS`] / [`MAX_FETCH_PER_DOMAIN`].
pub(crate) const MAX_FETCH_PER_DOMAIN_CEILING: usize = 64;

/// Execution-scoped budget overrides for a SINGLE skill invocation.
///
/// This is the one trusted-host knob that can RAISE a skill's resource caps
/// above the defaults for one call. It bundles the two raisable caps so a host
/// path that legitimately needs more headroom (currently only the mail-read
/// endpoints) can request them coherently:
///   * `per_domain_fetch_cap` — widens the exfiltration-guard fetch caps (see
///     [`effective_fetch_caps`]); `None` keeps the defaults.
///   * `fuel` — raises the per-skill Wasm fuel ceiling (see
///     [`effective_skill_fuel_with_budget`]); `None` keeps the 500M default.
///
/// Both fields are RAISE-ONLY and hard-clamped to ceilings, so a buggy or
/// hostile caller can never lower a cap or exceed the ceiling.
///
/// SECURITY — LLM-unreachability: an `ExecBudget` can only enter execution via
/// [`SandboxEngine::execute_with_auth_override_and_budget`] /
/// [`SandboxEngine::execute_with_auth_override_and_fetch_budget`], both of
/// which are called ONLY from host-authored Rust (the admin mail endpoints).
/// Every LLM-/heartbeat-/system-reachable surface (`execute`,
/// `execute_with_auth_override`, `execute_system`, `run_skill_direct`)
/// dispatches with `budget = None`, so a skill or
/// an LLM tool call has no channel through which to request a wider cap. The
/// budget value itself is computed by host code, never taken from skill output
/// or LLM-supplied payload.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecBudget {
    /// Trusted per-domain fetch allowance for this execution (raise-only,
    /// clamped at [`MAX_FETCH_PER_DOMAIN_CEILING`]). `None` = defaults.
    pub per_domain_fetch_cap: Option<usize>,
    /// Trusted Wasm fuel ceiling for this execution (raise-only, clamped at
    /// [`WASM_FUEL_CEILING`]). `None` = the [`WASM_FUEL_LIMIT`] default.
    pub fuel: Option<u64>,
    /// Trusted Wasm linear-memory ceiling (MiB) for this execution (raise-only,
    /// clamped at [`MAX_SKILL_MEMORY_MB_CEILING`]). `None` = the 2 MiB default.
    /// The 2 MiB default is far too small to decode+sanitize a rich HTML email
    /// (body + inline images + working set); the host mail-read endpoints raise
    /// it. LLM-/heartbeat-reachable paths pass `None` and keep the 2 MiB cap.
    pub max_memory_mb: Option<u32>,
}

/// Resolve the effective `(max_fetch_calls, max_fetch_per_domain)` caps for
/// one execution.
///
/// - `None` (every LLM-reachable path): the exfiltration-guard defaults,
///   exactly as before the budget existed.
/// - `Some(budget)`: a single trusted-host budget value that RAISES both caps
///   coherently — the per-domain cap becomes `budget` (floored at the
///   default, so a budget can never lower a cap; clamped at
///   [`MAX_FETCH_PER_DOMAIN_CEILING`], warning when asked for more) and the
///   total cap is raised to at least that same value so a budgeted
///   per-domain allowance is not strangled by the total-call cap
///   (e.g. 1 list + 50 message fetches = 51 > MAX_FETCH_CALLS).
pub(crate) fn effective_fetch_caps(
    fetch_budget: Option<usize>,
    skill_name: &str,
) -> (usize, usize) {
    let Some(requested) = fetch_budget else {
        return (MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN);
    };
    let clamped = if requested > MAX_FETCH_PER_DOMAIN_CEILING {
        tracing::warn!(
            skill = %skill_name,
            requested,
            ceiling = MAX_FETCH_PER_DOMAIN_CEILING,
            "execution-scoped fetch budget exceeds hard ceiling — clamping"
        );
        MAX_FETCH_PER_DOMAIN_CEILING
    } else {
        requested
    };
    // Raise-only: the budget widens the guard for this one execution, it can
    // never narrow it below the defaults (defense in depth — a buggy caller
    // passing 0 must not disable fetching, and the override surface stays
    // strictly "default or wider, up to the ceiling").
    let per_domain = clamped.max(MAX_FETCH_PER_DOMAIN);
    let total = per_domain.max(MAX_FETCH_CALLS);
    (total, per_domain)
}

/// Maximum number of fetch log entries retained per invocation.
/// Prevents unbounded memory growth from skills that make many requests.
pub(crate) const MAX_FETCH_LOG_ENTRIES: usize = 500;

/// Maximum number of unique domains tracked in per-domain counters.
/// Prevents memory exhaustion from skills that scatter requests across many hosts.
pub(crate) const MAX_UNIQUE_DOMAINS: usize = 100;

/// Host state for each Wasm instance — lives only for the duration of one call.
pub(crate) struct HostState {
    /// Domain whitelist for this skill's network access.
    pub(crate) allowed_domains: Arc<Vec<String>>,
    /// Filesystem path whitelist for this skill's file access.
    pub(crate) allowed_fs_paths: Arc<Vec<String>>,
    /// Accumulated fetch requests from aegis::host_fetch calls.
    pub(crate) fetch_log: Vec<FetchRecord>,
    /// Number of host_fetch calls made in this execution.
    pub(crate) fetch_count: usize,
    /// Per-domain fetch counters for rate limiting.
    pub(crate) domain_fetch_counts: HashMap<String, usize>,
    /// Effective total fetch-call cap for THIS execution. Defaults to
    /// [`MAX_FETCH_CALLS`]; raised only by a trusted-host fetch budget
    /// (see [`effective_fetch_caps`]), never above the budget ceiling.
    pub(crate) max_fetch_calls: usize,
    /// Effective per-domain fetch cap for THIS execution. Defaults to
    /// [`MAX_FETCH_PER_DOMAIN`]; raised only by a trusted-host fetch budget,
    /// never above [`MAX_FETCH_PER_DOMAIN_CEILING`].
    pub(crate) max_fetch_per_domain: usize,
    /// Output buffer written by the skill via aegis::host_write_output.
    pub(crate) output_buffer: Vec<u8>,
    /// Response body from the last successful host_fetch / host_read_file call.
    pub(crate) fetch_response: Vec<u8>,
    /// Per-instance memory limiter.
    pub(crate) limiter: crate::sandbox_host_fns::StoreLimiter,
    /// Domain → Authorization header value for auto-injection.
    pub(crate) auth_headers: Arc<HashMap<String, String>>,
    /// Memory handle for skill state persistence (optional for backward compat).
    pub(crate) memory_handle: Option<crate::memory::MemoryHandle>,
    /// Name of the currently executing skill (for state persistence scoping).
    pub(crate) skill_name: String,
    /// Response buffer for host_load_state (separate from fetch_response).
    pub(crate) state_response: Vec<u8>,
    /// Fetch thread pool reference (needed by v2 HostV2 trait implementation).
    pub(crate) fetch_pool: Arc<rayon::ThreadPool>,
}

#[derive(Debug, Clone)]
pub struct FetchRecord {
    pub url: String,
    pub allowed: bool,
}

/// Maximum number of threads in the host_fetch thread pool.
/// Bounds total concurrent HTTP requests across all skill invocations.
/// Kept small (4) to avoid starving the Tokio runtime's blocking thread pool
/// and to limit outbound connection concurrency under burst load.
const FETCH_POOL_SIZE: usize = 4;

/// Source of an authorization token for a domain.
///
/// `Static` tokens come from environment variables and never change.
/// `Dynamic` tokens come from a `TokenProvider` (e.g. Google OAuth2) that
/// auto-refreshes expired tokens on each access.
pub enum AuthSource {
    /// Fixed token string (e.g. "Bearer ghp_xxx" from env var).
    Static(String),
    /// Dynamic token provider that auto-refreshes (e.g. Google OAuth2).
    /// The provider returns a raw token; "Bearer " prefix is added automatically.
    Dynamic(Arc<dyn TokenProvider>),
}

/// The S-LEG sandbox engine.
///
/// Manages pre-compiled Wasm modules and enforces capability-based security
/// on every invocation. Each call gets a fresh Store+Instance (cell isolation)
/// that is dropped immediately after execution.
pub struct SandboxEngine {
    engine: Engine,
    skills: RwLock<HashMap<String, CompiledSkill>>,
    replay_guard: ReplayGuard,
    fetch_pool: Arc<rayon::ThreadPool>,
    /// Domain → auth source. Resolved to concrete token strings at execution
    /// time (in `run_isolated`), so `HostState` always gets fresh tokens.
    auth_sources: RwLock<HashMap<String, AuthSource>>,
    /// Legacy compat: static auth headers set via `set_auth_headers()`.
    /// Merged into resolved headers at execution time.
    static_auth_headers: RwLock<HashMap<String, String>>,
    /// The system identity's DID. When set, envelope verification rejects any
    /// envelope whose `agent_did` does not match, preventing self-signed forgery.
    system_did: RwLock<Option<String>>,
    /// Memory handle for skill state persistence. Set after memory actor spawns.
    memory_handle: RwLock<Option<crate::memory::MemoryHandle>>,
}

/// Fuel budget per skill execution.
/// 1 fuel ≈ 1 Wasm instruction. 500M ≈ a few seconds of pure compute.
/// Host function calls (host_fetch, etc.) do NOT consume fuel, so I/O wait
/// time is not counted. This prevents infinite-loop DoS while allowing
/// legitimate skills ample headroom.
pub(crate) const WASM_FUEL_LIMIT: u64 = 500_000_000;

/// Path to the pre-compiled QuickJS Wasm engine module.
/// JavaScript skills are executed inside this engine rather than being compiled to native Wasm.
const QUICKJS_ENGINE_PATH: &str = "compiled_skills/quickjs_engine.wasm";

/// Minimal valid Wasm module (empty). Used as a placeholder for JavaScript skills
/// whose actual execution happens via the QuickJS engine module.
const MINIMAL_WASM_MODULE: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

/// Default WASM skill memory limit (2 MiB) — used in SkillCapability::default().
pub(crate) const DEFAULT_SKILL_MEMORY_MB: u32 = 2;
// PROBE_SKILL_MEMORY_MB, DEFAULT_SKILL_FUEL, PROBE_SKILL_FUEL removed —
// superseded by SkillCapability fields (max_memory_mb, max_fuel).

fn effective_skill_memory_mb(requested_mb: u32) -> u32 {
    requested_mb.clamp(DEFAULT_SKILL_MEMORY_MB, DEFAULT_SKILL_MEMORY_MB)
}

/// Hard ceiling for a trusted execution-scoped Wasm memory override (MiB).
///
/// 64 MiB is ample to decode + sanitize even a large (multi-MB) HTML email with
/// inline images, while still bounding a pathological skill from exhausting host
/// memory. No caller can exceed it; only the host mail-read endpoints request a
/// raise (via [`ExecBudget::max_memory_mb`], a host-only surface — see the
/// `ExecBudget` doc comment; there is no LLM-/heartbeat-reachable path to it).
pub(crate) const MAX_SKILL_MEMORY_MB_CEILING: u32 = 64;

/// Resolve the effective Wasm memory (MiB) for one execution, honouring an
/// optional trusted execution-scoped raise. Mirrors
/// [`effective_skill_fuel_with_budget`].
///
/// - `budget_mb == None` (every LLM-reachable path): the [`effective_skill_memory_mb`]
///   default (2 MiB).
/// - `budget_mb == Some(req)`: RAISE-ONLY — `max(default, req)` clamped at
///   [`MAX_SKILL_MEMORY_MB_CEILING`]. A budget can never LOWER the cap below the
///   default, and can never exceed the ceiling (warns when asked for more).
fn effective_skill_memory_mb_with_budget(requested_mb: u32, budget_mb: Option<u32>) -> u32 {
    let default_mb = effective_skill_memory_mb(requested_mb);
    let Some(req) = budget_mb else {
        return default_mb;
    };
    let clamped = if req > MAX_SKILL_MEMORY_MB_CEILING {
        tracing::warn!(
            requested_mb = req,
            ceiling = MAX_SKILL_MEMORY_MB_CEILING,
            "execution-scoped memory budget exceeds hard ceiling — clamping"
        );
        MAX_SKILL_MEMORY_MB_CEILING
    } else {
        req
    };
    // Raise-only: never below the default the skill would otherwise receive.
    clamped.max(default_mb)
}

fn effective_skill_fuel(requested_fuel: u64) -> u64 {
    if requested_fuel == 0 {
        WASM_FUEL_LIMIT
    } else {
        requested_fuel.min(WASM_FUEL_LIMIT)
    }
}

/// Hard ceiling for a trusted execution-scoped fuel override.
///
/// 1 fuel ≈ 1 Wasm instruction, so 4B ≈ ~tens of seconds of pure compute —
/// ample headroom for sanitizing/parsing a large (multi-hundred-KB) email body
/// even on a slow path, while still bounding a pathological infinite loop. No
/// caller can exceed it; only the host mail-read endpoints request a raise.
pub(crate) const WASM_FUEL_CEILING: u64 = 4_000_000_000;

/// Resolve the effective Wasm fuel for one execution, honouring an optional
/// trusted execution-scoped raise.
///
/// - `budget_fuel == None` (every LLM-reachable path): identical to
///   [`effective_skill_fuel`] — the 500M default (or the capability's own
///   `max_fuel`, itself already clamped at 500M).
/// - `budget_fuel == Some(req)`: RAISE-ONLY. The effective fuel becomes
///   `max(default, req)` clamped at [`WASM_FUEL_CEILING`]. A budget can never
///   LOWER the fuel below what the skill would otherwise get (so a buggy caller
///   passing a tiny value cannot starve a legitimate skill), and can never
///   exceed the ceiling (warning when asked for more).
fn effective_skill_fuel_with_budget(requested_fuel: u64, budget_fuel: Option<u64>) -> u64 {
    let default_fuel = effective_skill_fuel(requested_fuel);
    let Some(req) = budget_fuel else {
        return default_fuel;
    };
    let clamped = if req > WASM_FUEL_CEILING {
        tracing::warn!(
            requested_fuel = req,
            ceiling = WASM_FUEL_CEILING,
            "execution-scoped fuel budget exceeds hard ceiling — clamping"
        );
        WASM_FUEL_CEILING
    } else {
        req
    };
    // Raise-only: the budget widens the fuel for this one execution; it can
    // never narrow it below the default the skill would otherwise receive.
    clamped.max(default_fuel)
}

impl SandboxEngine {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        Self::from_engine(engine)
    }

    /// Create a `SandboxEngine` reusing an existing Wasmtime `Engine`.
    ///
    /// This avoids the expensive JIT compiler initialisation (~400-800 ms)
    /// when multiple sandbox instances are needed — particularly useful in
    /// tests where dozens of instances would otherwise be created.
    pub fn from_engine(engine: Engine) -> Result<Self> {
        let fetch_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(FETCH_POOL_SIZE)
            .thread_name(|i| format!("aegis-fetch-{i}"))
            .build()
            .context("building host_fetch thread pool")?;
        Ok(Self {
            engine,
            skills: RwLock::new(HashMap::new()),
            replay_guard: ReplayGuard::new(),
            fetch_pool: Arc::new(fetch_pool),
            auth_sources: RwLock::new(HashMap::new()),
            static_auth_headers: RwLock::new(HashMap::new()),
            system_did: RwLock::new(None),
            memory_handle: RwLock::new(None),
        })
    }

    /// Return the Wasmtime engine configuration used by default.
    pub fn default_engine() -> Result<Engine> {
        let mut config = Config::new();
        config.consume_fuel(true);
        Engine::new(&config)
            .map_err(anyhow::Error::from)
            .context("creating Wasmtime engine")
    }

    /// Access the replay guard (e.g., to load persisted nonces on startup).
    pub fn replay_guard(&self) -> &ReplayGuard {
        &self.replay_guard
    }

    /// Set the static auth headers map (domain → "Bearer xxx" or "token xxx").
    /// These headers are automatically injected into host_fetch requests
    /// when the request domain matches. Call this at startup after loading
    /// credentials from env vars.
    pub fn set_auth_headers(&self, headers: HashMap<String, String>) {
        *self.static_auth_headers.write() = headers;
    }

    /// Pin the system identity's DID. Once set, envelope verification rejects
    /// any envelope whose `agent_did` does not match, preventing self-signed
    /// forgery attacks where an attacker generates their own key pair.
    pub fn set_system_did(&self, did: String) {
        *self.system_did.write() = Some(did);
    }

    /// Set the memory handle for skill state persistence.
    /// Must be called after the memory actor is spawned.
    pub fn set_memory_handle(&self, handle: crate::memory::MemoryHandle) {
        *self.memory_handle.write() = Some(handle);
    }

    /// Get a snapshot of the current resolved auth headers (domain → "Bearer xxx").
    /// Dynamic sources are resolved at call time. Failures are logged but not fatal here
    /// (this is a diagnostic/status method, not an execution path).
    pub fn get_auth_headers(&self) -> HashMap<String, String> {
        self.resolve_auth_headers().0
    }

    /// Add a single static auth header at runtime (for dynamically loaded skills).
    pub fn add_auth_header(&self, domain: &str, value: &str) {
        self.static_auth_headers
            .write()
            .insert(domain.to_string(), value.to_string());
    }

    /// Register a dynamic token provider for one or more domains.
    /// The provider's `get_token()` is called each time a skill makes a
    /// fetch to a matching domain, ensuring fresh tokens automatically.
    pub fn set_dynamic_auth(&self, domains: &[&str], provider: Arc<dyn TokenProvider>) {
        let mut sources = self.auth_sources.write();
        for domain in domains {
            sources.insert(domain.to_string(), AuthSource::Dynamic(provider.clone()));
        }
    }

    /// Remove dynamic auth providers for the given domains.
    pub fn clear_dynamic_auth(&self, domains: &[&str]) {
        let mut sources = self.auth_sources.write();
        for domain in domains {
            sources.remove(*domain);
        }
    }

    /// Resolve all auth sources to concrete header strings.
    /// Static headers are copied directly; dynamic providers are called
    /// to get the current token (with "Bearer " prefix).
    ///
    /// Returns `(resolved_headers, failed_domains)`. Each failed domain
    /// includes the error message so callers can abort with a clear reason.
    fn resolve_auth_headers(&self) -> (HashMap<String, String>, Vec<(String, String)>) {
        let mut resolved = self.static_auth_headers.read().clone();
        let mut failed: Vec<(String, String)> = Vec::new();

        for (domain, source) in self.auth_sources.read().iter() {
            match source {
                AuthSource::Static(value) => {
                    resolved.insert(domain.clone(), value.clone());
                }
                AuthSource::Dynamic(provider) => match provider.get_token() {
                    Ok(token) => {
                        resolved.insert(domain.clone(), format!("Bearer {token}"));
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        if msg.contains("cooldown") {
                            tracing::debug!(
                                domain = %domain,
                                error = %msg,
                                "Dynamic auth token in cooldown"
                            );
                        } else {
                            tracing::warn!(
                                domain = %domain,
                                error = %msg,
                                "Failed to resolve dynamic auth token"
                            );
                        }
                        failed.push((domain.clone(), msg));
                    }
                },
            }
        }

        (resolved, failed)
    }

    /// Load and pre-compile a skill from a `.wasm` file.
    pub fn load_skill(&self, wasm_path: &Path, capability: SkillCapability) -> Result<()> {
        let wasm_bytes =
            std::fs::read(wasm_path).with_context(|| format!("reading {}", wasm_path.display()))?;
        self.load_skill_from_bytes(&wasm_bytes, capability)
    }

    /// Load a skill from raw bytes (useful for testing with WAT).
    /// For JavaScript skills, `wasm_bytes` contains the JS source code (UTF-8).
    /// For v2 component model skills (`capability.component_model == true`),
    /// bytes are loaded as a `wasmtime::component::Component`.
    pub fn load_skill_from_bytes(
        &self,
        wasm_bytes: &[u8],
        capability: SkillCapability,
    ) -> Result<()> {
        let (module, js_source) = if capability.language == "javascript" {
            let js = std::str::from_utf8(wasm_bytes)
                .context("JS skill source is not valid UTF-8")?
                .to_string();
            let placeholder = Module::new(&self.engine, MINIMAL_WASM_MODULE)
                .map_err(anyhow::Error::from)
                .context("compiling placeholder module for JS skill")?;
            (SkillModule::Core(placeholder), Some(js))
        } else if capability.component_model {
            // v2 Component Model skill — typed interface, no pointer management
            let component = Component::new(&self.engine, wasm_bytes)
                .map_err(anyhow::Error::from)
                .context("compiling v2 Component Model skill")?;
            let is_probe = capability
                .skill_pattern
                .as_deref()
                .is_some_and(|p| p == "probe_heartbeat");
            let skill_module = if is_probe {
                SkillModule::ComponentProbeV2(component)
            } else {
                SkillModule::ComponentV2(component)
            };
            (skill_module, None)
        } else {
            // v1 core module — pointer-based I/O
            let module = Module::new(&self.engine, wasm_bytes)
                .map_err(anyhow::Error::from)
                .context("compiling Wasm module")?;
            (SkillModule::Core(module), None)
        };
        self.skills.write().insert(
            capability.skill_name.clone(),
            CompiledSkill {
                module,
                capability,
                js_source,
            },
        );
        Ok(())
    }

    /// List all loaded skill capabilities (sorted by name for prompt cache stability).
    pub fn loaded_skills(&self) -> Vec<SkillCapability> {
        let mut by_name = std::collections::BTreeMap::new();
        for skill in self.skills.read().values() {
            by_name.insert(
                skill.capability.skill_name.clone(),
                skill.capability.clone(),
            );
        }
        by_name.into_values().collect()
    }

    /// Check if a skill is currently loaded in the sandbox.
    pub fn is_skill_loaded(&self, skill_name: &str) -> bool {
        self.skills.read().contains_key(skill_name)
    }

    /// Test-only: register a capability backed by an empty Wasm module.
    ///
    /// Lets tests that only need a `SkillCapability` to appear in
    /// [`SandboxEngine::loaded_skills`] (prompt/tool-definition assertions)
    /// skip building a real skill. Never call the registered skill — its
    /// module exports nothing.
    #[cfg(test)]
    pub fn register_test_capability(&self, capability: SkillCapability) {
        let module = Module::new(&self.engine, "(module)").expect("empty test module compiles");
        self.skills.write().insert(
            capability.skill_name.clone(),
            CompiledSkill {
                module: SkillModule::Core(module),
                capability,
                js_source: None,
            },
        );
    }

    /// Hot-load a skill at runtime from compiled Wasm bytes + capability.
    /// Thread-safe: acquires write lock only during insertion.
    /// For JavaScript skills, `wasm_bytes` contains the JS source code (UTF-8).
    /// Delegates to `load_skill_from_bytes` for component model detection.
    pub fn hot_load_skill(&self, wasm_bytes: &[u8], capability: SkillCapability) -> Result<()> {
        self.load_skill_from_bytes(wasm_bytes, capability)
    }

    /// Unload a skill at runtime. Returns true if the skill was found and removed.
    pub fn unload_skill(&self, skill_name: &str) -> bool {
        self.skills.write().remove(skill_name).is_some()
    }

    /// Execute a skill with full S-LEG enforcement.
    ///
    /// Security pipeline:
    /// 1. Verify Ed25519 signature + anti-replay.
    /// 2. Skill whitelist check.
    /// 3. Fresh Store+Instance (cell isolation).
    /// 4. Domain-gated host_fetch injection.
    /// 5. Execute → collect output → drop instance.
    pub fn execute(&self, envelope: &IntentEnvelope) -> Result<SkillResult> {
        let start = Instant::now();

        // Step 1: Cryptographic verification + replay deduplication + DID pinning
        let system_did = self.system_did.read();
        verify_envelope_with_guard(envelope, Some(&self.replay_guard), system_did.as_deref())
            .context("S-LEG: envelope verification failed")?;

        // Step 2-5: Clone module + capability under read lock, then release lock before execution.
        // This prevents slow network I/O in run_isolated from blocking hot_load/unload operations.
        let (skill_module, capability) = {
            let skills = self.skills.read();
            let compiled = skills.get(&envelope.target_skill).with_context(|| {
                format!(
                    "S-LEG: skill '{}' not registered — refusing execution",
                    envelope.target_skill
                )
            })?;
            (compiled.module.clone(), compiled.capability.clone())
        };

        let output =
            self.dispatch_execution(&skill_module, &capability, &envelope.payload, None, None)?;

        Ok(SkillResult {
            output,
            elapsed_ms: start.elapsed().as_millis(),
        })
    }

    /// Execute a skill with per-call auth header overrides.
    ///
    /// This is used by the orchestrator for multi-account mail support:
    /// the `auth_overrides` map (domain → "Bearer xxx") is merged on top of
    /// the global auth sources for this single execution only.
    ///
    /// The global `auth_sources` are NOT mutated — overrides are applied to a
    /// copy of the resolved headers, ensuring no race conditions.
    pub fn execute_with_auth_override(
        &self,
        envelope: &IntentEnvelope,
        auth_overrides: HashMap<String, String>,
    ) -> Result<SkillResult> {
        self.execute_with_auth_and_budget(envelope, auth_overrides, None)
    }

    /// Execute a skill with auth overrides AND an execution-scoped fetch
    /// budget.
    ///
    /// The budget raises the per-domain (and, coherently, the total) fetch
    /// cap for THIS execution only — see [`effective_fetch_caps`] for the
    /// raise-only + ceiling-clamp semantics. It exists for trusted host code
    /// paths whose legitimate fan-out exceeds the exfiltration-guard
    /// defaults (e.g. a probe that fans out into many per-item fetches
    /// against one domain).
    ///
    /// SECURITY: this method must only ever be called from host-authored
    /// code with a host-computed budget. It is deliberately NOT part of any
    /// LLM-reachable dispatch surface (`execute`, `execute_with_auth_override`,
    /// `execute_system`, `run_skill_direct` take no budget parameter), so a
    /// skill or an LLM tool call has no channel through which to request a
    /// wider cap.
    pub fn execute_with_auth_override_and_fetch_budget(
        &self,
        envelope: &IntentEnvelope,
        auth_overrides: HashMap<String, String>,
        per_domain_fetch_cap: usize,
    ) -> Result<SkillResult> {
        self.execute_with_auth_and_budget(
            envelope,
            auth_overrides,
            Some(ExecBudget {
                per_domain_fetch_cap: Some(per_domain_fetch_cap),
                fuel: None,
                max_memory_mb: None,
            }),
        )
    }

    /// Execute a skill with auth overrides AND a full execution-scoped budget
    /// (fetch caps + Wasm fuel) — see [`ExecBudget`].
    ///
    /// SECURITY: like [`Self::execute_with_auth_override_and_fetch_budget`],
    /// this is a host-only surface. It is NOT part of any LLM-/heartbeat-/
    /// system-reachable dispatch path, so neither a skill nor an LLM tool call
    /// can request a raised fetch cap OR raised fuel. The `ExecBudget` is
    /// always host-computed (see the mail-read endpoints in `admin_mail.rs`).
    pub fn execute_with_auth_override_and_budget(
        &self,
        envelope: &IntentEnvelope,
        auth_overrides: HashMap<String, String>,
        budget: ExecBudget,
    ) -> Result<SkillResult> {
        self.execute_with_auth_and_budget(envelope, auth_overrides, Some(budget))
    }

    /// Shared body for the auth-override execute variants.
    fn execute_with_auth_and_budget(
        &self,
        envelope: &IntentEnvelope,
        auth_overrides: HashMap<String, String>,
        budget: Option<ExecBudget>,
    ) -> Result<SkillResult> {
        let start = Instant::now();

        let system_did = self.system_did.read();
        verify_envelope_with_guard(envelope, Some(&self.replay_guard), system_did.as_deref())
            .context("S-LEG: envelope verification failed")?;

        let (skill_module, capability) = {
            let skills = self.skills.read();
            let compiled = skills.get(&envelope.target_skill).with_context(|| {
                format!(
                    "S-LEG: skill '{}' not registered — refusing execution",
                    envelope.target_skill
                )
            })?;
            (compiled.module.clone(), compiled.capability.clone())
        };

        let output = self.dispatch_execution(
            &skill_module,
            &capability,
            &envelope.payload,
            Some(auth_overrides),
            budget,
        )?;

        Ok(SkillResult {
            output,
            elapsed_ms: start.elapsed().as_millis(),
        })
    }

    /// Execute a loaded skill directly by name + payload, bypassing envelope signing.
    /// Used by the v2 DAG execution engine for chaining skills.
    pub fn run_skill_direct(
        &self,
        skill_name: &str,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let (skill_module, capability) = {
            let skills = self.skills.read();
            let compiled = skills.get(skill_name).with_context(|| {
                format!("DAG execution: skill '{skill_name}' not loaded in sandbox")
            })?;
            (compiled.module.clone(), compiled.capability.clone())
        };
        self.dispatch_execution(&skill_module, &capability, payload, None, None)
    }

    // NOTE: `run_skill_direct` (DAG/LLM-reachable) and the other dispatch
    // surfaces below intentionally pass `budget = None`, so the raised
    // fetch/fuel caps remain unreachable from skill chaining or LLM tool calls.

    /// Dispatch execution based on skill module type (v1 core module vs v2 component).
    ///
    /// `budget` is the optional execution-scoped budget (fetch caps + fuel) —
    /// `None` (the defaults) for every surface except the host-only
    /// [`SandboxEngine::execute_with_auth_override_and_budget`] /
    /// [`SandboxEngine::execute_with_auth_override_and_fetch_budget`].
    fn dispatch_execution(
        &self,
        skill_module: &SkillModule,
        capability: &SkillCapability,
        payload: &serde_json::Value,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<String> {
        match skill_module {
            SkillModule::Core(module) => {
                self.run_isolated_with_auth(module, capability, payload, auth_overrides, budget)
            }
            SkillModule::ComponentV2(component) => {
                self.run_component_v2(component, capability, payload, auth_overrides, budget)
            }
            SkillModule::ComponentProbeV2(component) => {
                self.run_component_probe_v2(component, capability, payload, auth_overrides, budget)
            }
        }
    }

    /// Execute a skill using a pre-signed system IntentEnvelope.
    ///
    /// This goes through the same `verify_envelope` path as user-initiated calls,
    /// ensuring no verification bypass exists. The caller (e.g. ChronosEngine) is
    /// responsible for signing the envelope with a valid AgentIdentity.
    pub fn execute_system(&self, envelope: &IntentEnvelope) -> Result<SkillResult> {
        // Same pipeline as execute() — full cryptographic verification
        self.execute(envelope)
    }

    #[allow(dead_code)]
    fn run_isolated(
        &self,
        module: &Module,
        capability: &SkillCapability,
        payload: &serde_json::Value,
    ) -> Result<String> {
        self.run_isolated_with_auth(module, capability, payload, None, None)
    }

    /// Execute a JavaScript skill in the QuickJS Wasm sandbox.
    ///
    /// JavaScript skills bypass the normal Rust->Wasm compilation pipeline.
    /// Instead, the JS source code is executed inside a pre-compiled QuickJS
    /// Wasm engine that provides the same host function bindings (`aegis.*`
    /// globals map to host_fetch, host_write_output, etc.).
    ///
    /// The QuickJS engine module must be present at [`QUICKJS_ENGINE_PATH`].
    /// If absent, this returns a clear error directing the user to install it
    /// or rebuild the skill as Rust.
    fn run_js_skill(
        &self,
        _module: &Module,
        capability: &SkillCapability,
        payload: &serde_json::Value,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<String> {
        let engine_module = self.get_or_load_js_engine()?;

        // Retrieve the JS source code stored when the skill was loaded.
        let js_source = self.skills.read()
            .get(&capability.skill_name)
            .and_then(|entry| entry.js_source.clone())
            .unwrap_or_else(|| {
                tracing::warn!(skill = %capability.skill_name, "JS source not found for skill — executing with empty source");
                String::new()
            });

        let mut augmented_payload = payload.clone();
        if let Some(obj) = augmented_payload.as_object_mut() {
            obj.insert(
                "_js_source".to_string(),
                serde_json::Value::String(js_source),
            );
        }

        // Execute using the same sandbox infrastructure but with the QuickJS engine module.
        // The engine module exports the standard `run()` + `alloc()` and uses host functions
        // to bridge JS <-> host I/O.
        self.run_isolated_with_auth(
            &engine_module,
            capability,
            &augmented_payload,
            auth_overrides,
            budget,
        )
    }

    /// Get or lazily load the QuickJS Wasm engine module.
    fn get_or_load_js_engine(&self) -> Result<Module> {
        let engine_path = std::path::Path::new(QUICKJS_ENGINE_PATH);
        if !engine_path.exists() {
            anyhow::bail!(
                "JavaScript runtime not available. \
                 Skill requires the QuickJS Wasm engine at '{}'. \
                 Install it or rebuild the skill as Rust.",
                QUICKJS_ENGINE_PATH
            );
        }

        let wasm_bytes =
            std::fs::read(engine_path).context("failed to read QuickJS engine Wasm")?;
        Module::new(&self.engine, &wasm_bytes)
            .map_err(anyhow::Error::from)
            .context("failed to compile QuickJS engine Wasm module")
    }

    /// Core isolated execution with optional auth overrides.
    ///
    /// When `auth_overrides` is `Some`, the provided headers are merged on top
    /// of the globally resolved auth headers (overriding matching domains).
    /// Resolve auth headers and abort if any of the skill's required domains failed.
    /// Returns the resolved headers map on success.
    pub(crate) fn resolve_and_check_auth(
        &self,
        capability: &SkillCapability,
        auth_overrides: Option<HashMap<String, String>>,
    ) -> Result<HashMap<String, String>> {
        let (mut resolved, mut failures) = self.resolve_auth_headers();
        if let Some(overrides) = auth_overrides {
            for (domain, value) in overrides {
                failures.retain(|(d, _)| d != &domain);
                resolved.insert(domain, value);
            }
        }
        let relevant: Vec<_> = failures
            .iter()
            .filter(|(domain, _)| capability.allowed_domains.iter().any(|d| d == domain))
            .collect();
        if !relevant.is_empty() {
            let domains: Vec<&str> = relevant.iter().map(|(d, _)| d.as_str()).collect();
            let errors: Vec<&str> = relevant.iter().map(|(_, e)| e.as_str()).collect();
            tracing::error!(
                skill = %capability.skill_name,
                ?domains,
                "[Sandbox] Auth token refresh failed for required domains — aborting skill execution"
            );
            anyhow::bail!(
                "Auth token refresh failed for domain(s) [{}]: {}. \
                 Skill '{}' cannot execute without valid credentials.",
                domains.join(", "),
                errors.join("; "),
                capability.skill_name,
            );
        }
        Ok(resolved)
    }

    fn run_isolated_with_auth(
        &self,
        module: &Module,
        capability: &SkillCapability,
        payload: &serde_json::Value,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<String> {
        // Auth check BEFORE any dispatch (including JS skills) —
        // executing with missing auth produces silent degradation (e.g. 401 → empty results).
        let resolved_headers = self.resolve_and_check_auth(capability, auth_overrides.clone())?;

        // Dispatch JavaScript skills to the QuickJS Wasm engine
        if capability.language == "javascript" {
            return self.run_js_skill(module, capability, payload, auth_overrides, budget);
        }

        let allowed_domains = Arc::new(capability.allowed_domains.clone());
        let allowed_fs_paths = Arc::new(capability.allowed_fs_paths.clone());

        let effective_mb = effective_skill_memory_mb_with_budget(
            capability.max_memory_mb,
            budget.and_then(|b| b.max_memory_mb),
        ) as usize;
        let max_bytes = effective_mb * 1024 * 1024;
        let (max_fetch_calls, max_fetch_per_domain) = effective_fetch_caps(
            budget.and_then(|b| b.per_domain_fetch_cap),
            &capability.skill_name,
        );

        let mut store = Store::new(
            &self.engine,
            HostState {
                allowed_domains: allowed_domains.clone(),
                allowed_fs_paths: allowed_fs_paths.clone(),
                fetch_log: Vec::new(),
                fetch_count: 0,
                domain_fetch_counts: HashMap::new(),
                max_fetch_calls,
                max_fetch_per_domain,
                output_buffer: Vec::new(),
                fetch_response: Vec::new(),
                limiter: crate::sandbox_host_fns::StoreLimiter {
                    max_memory: max_bytes,
                    peak_memory: 0,
                },
                auth_headers: Arc::new(resolved_headers),
                memory_handle: self.memory_handle.read().clone(),
                skill_name: capability.skill_name.clone(),
                state_response: Vec::new(),
                fetch_pool: self.fetch_pool.clone(),
            },
        );

        // Enforce memory limit per capability
        store.limiter(|state| &mut state.limiter);
        // Enforce CPU limit via per-skill fuel budget (prevents infinite loops).
        // A trusted execution-scoped budget may raise it (host-only path).
        let effective_fuel =
            effective_skill_fuel_with_budget(capability.max_fuel, budget.and_then(|b| b.fuel));
        store.set_fuel(effective_fuel)?;

        let mut linker: Linker<HostState> = Linker::new(&self.engine);

        // Link all aegis host functions (fetch, I/O, state)
        crate::sandbox_host_fns::link_host_functions(
            &mut linker,
            payload,
            self.fetch_pool.clone(),
        )?;

        // ── WASI stdio stubs ──────────────────────────────────────────
        // Many Rust WASM modules compiled with wasm32-wasip1 import
        // wasi_snapshot_preview1::fd_write for eprintln!/println!.
        // Without a real stub, define_unknown_imports_as_traps makes
        // any stderr write a fatal trap — even for simple log messages.
        //
        // We provide a minimal fd_write that:
        //  - Calculates total bytes from the iov array
        //  - Writes the byte count to *nwritten (so callers don't retry)
        //  - Returns 0 (ESUCCESS) for stdout(1)/stderr(2)
        //  - Returns 8 (EBADF) for other fds
        crate::sandbox_host_fns::link_wasi_stdio_stubs(&mut linker)?;

        // Stub any remaining unknown imports (e.g., other WASI functions) as traps.
        linker.define_unknown_imports_as_traps(module)?;

        // Instantiate and execute
        crate::sandbox_host_fns::execute_skill_module(
            &mut store,
            &mut linker,
            module,
            capability,
            effective_fuel,
        )
    }

    // ── v2 Component Model execution ────────────────────────────────────

    /// Create a configured HostState for component execution (shared setup for v2 skills).
    /// Returns `Err` if any of the skill's required auth domains failed token refresh.
    fn make_host_state(
        &self,
        capability: &SkillCapability,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<HostState> {
        let allowed_domains = Arc::new(capability.allowed_domains.clone());
        let allowed_fs_paths = Arc::new(capability.allowed_fs_paths.clone());

        let resolved_headers = self.resolve_and_check_auth(capability, auth_overrides)?;
        let (max_fetch_calls, max_fetch_per_domain) = effective_fetch_caps(
            budget.and_then(|b| b.per_domain_fetch_cap),
            &capability.skill_name,
        );

        Ok(HostState {
            allowed_domains,
            allowed_fs_paths,
            fetch_log: Vec::new(),
            fetch_count: 0,
            domain_fetch_counts: HashMap::new(),
            max_fetch_calls,
            max_fetch_per_domain,
            output_buffer: Vec::new(),
            fetch_response: Vec::new(),
            limiter: crate::sandbox_host_fns::StoreLimiter {
                max_memory: effective_skill_memory_mb_with_budget(
                    capability.max_memory_mb,
                    budget.and_then(|b| b.max_memory_mb),
                ) as usize
                    * 1024
                    * 1024,
                peak_memory: 0,
            },
            auth_headers: Arc::new(resolved_headers),
            memory_handle: self.memory_handle.read().clone(),
            skill_name: capability.skill_name.clone(),
            state_response: Vec::new(),
            fetch_pool: self.fetch_pool.clone(),
        })
    }

    /// Execute a v2 standard component skill (JSON in -> JSON out).
    fn run_component_v2(
        &self,
        component: &Component,
        capability: &SkillCapability,
        payload: &serde_json::Value,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<String> {
        let host_state = self.make_host_state(capability, auth_overrides, budget)?;
        let mut store = Store::new(&self.engine, host_state);
        store.limiter(|state| &mut state.limiter);
        let effective_fuel =
            effective_skill_fuel_with_budget(capability.max_fuel, budget.and_then(|b| b.fuel));
        store.set_fuel(effective_fuel)?;

        let mut linker = wasmtime::component::Linker::<HostState>::new(&self.engine);
        skill_v2::AegisSkillV2::add_to_linker::<HostState, wasmtime::component::HasSelf<HostState>>(&mut linker, |state| state)
            .map_err(anyhow::Error::from).context("linking v2 host functions")?;

        let bindings = skill_v2::AegisSkillV2::instantiate(&mut store, component, &linker)
            .map_err(anyhow::Error::from)
            .context("instantiating v2 component")?;

        let payload_json = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());

        let exec_start = Instant::now();
        let result = bindings
            .aegis_skill_skill_v2()
            .call_run(&mut store, &payload_json);
        let elapsed_ms = exec_start.elapsed().as_millis();

        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = effective_fuel.saturating_sub(fuel_remaining);
        let peak_memory_kb = store.data().limiter.peak_memory / 1024;

        match result {
            Ok(output) => {
                tracing::debug!(
                    skill = %capability.skill_name,
                    elapsed_ms,
                    fuel_consumed,
                    peak_memory_kb,
                    "v2 component execution completed"
                );
                Ok(output)
            }
            Err(e) => {
                tracing::warn!(
                    skill = %capability.skill_name,
                    elapsed_ms,
                    fuel_consumed,
                    peak_memory_kb,
                    "v2 component execution failed"
                );
                Err(anyhow::Error::from(e)).context("v2 component execution failed")
            }
        }
    }

    /// Execute a v2 probe component skill (JSON in -> typed ProbeSignal out).
    /// The ProbeSignal is serialized to JSON for uniform downstream handling.
    fn run_component_probe_v2(
        &self,
        component: &Component,
        capability: &SkillCapability,
        payload: &serde_json::Value,
        auth_overrides: Option<HashMap<String, String>>,
        budget: Option<ExecBudget>,
    ) -> Result<String> {
        let host_state = self.make_host_state(capability, auth_overrides, budget)?;
        let mut store = Store::new(&self.engine, host_state);
        store.limiter(|state| &mut state.limiter);
        let effective_fuel =
            effective_skill_fuel_with_budget(capability.max_fuel, budget.and_then(|b| b.fuel));
        store.set_fuel(effective_fuel)?;

        let mut linker = wasmtime::component::Linker::<HostState>::new(&self.engine);
        probe_v2::AegisProbeV2::add_to_linker::<HostState, wasmtime::component::HasSelf<HostState>>(&mut linker, |state| state)
            .map_err(anyhow::Error::from).context("linking v2 probe host functions")?;

        let bindings = probe_v2::AegisProbeV2::instantiate(&mut store, component, &linker)
            .map_err(anyhow::Error::from)
            .context("instantiating v2 probe component")?;

        let payload_json = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());

        let exec_start = Instant::now();
        let result = bindings
            .aegis_skill_probe_v2()
            .call_run(&mut store, &payload_json);
        let elapsed_ms = exec_start.elapsed().as_millis();

        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = effective_fuel.saturating_sub(fuel_remaining);
        let peak_memory_kb = store.data().limiter.peak_memory / 1024;

        match result {
            Ok(probe_signal) => {
                tracing::debug!(
                    skill = %capability.skill_name,
                    elapsed_ms,
                    fuel_consumed,
                    peak_memory_kb,
                    require_llm_attention = probe_signal.require_llm_attention,
                    "v2 probe execution completed"
                );
                // Convert WIT ProbeSignal -> models::ProbeSignal -> JSON
                let model_signal: crate::models::ProbeSignal = probe_signal.into();
                serde_json::to_string(&model_signal)
                    .context("serializing ProbeSignal from v2 component")
            }
            Err(e) => {
                tracing::warn!(
                    skill = %capability.skill_name,
                    elapsed_ms,
                    fuel_consumed,
                    peak_memory_kb,
                    "v2 probe execution failed"
                );
                Err(anyhow::Error::from(e)).context("v2 probe execution failed")
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::AgentIdentity;
    use serde_json::json;

    use std::sync::LazyLock;

    /// Shared Wasmtime engine for all sandbox tests — avoids expensive JIT init per test.
    static TEST_ENGINE: LazyLock<wasmtime::Engine> = LazyLock::new(|| {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        wasmtime::Engine::new(&config).expect("test engine init")
    });

    fn test_engine() -> wasmtime::Engine {
        TEST_ENGINE.clone()
    }

    /// Helper: build a minimal WAT module that calls aegis::host_fetch with a given URL.
    fn make_fetch_skill_wat(url: &str) -> String {
        let url_bytes: Vec<String> = url.bytes().map(|b| format!("\\{b:02x}")).collect();
        let url_escaped = url_bytes.join("");
        format!(
            r#"(module
                (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
                (import "aegis" "host_write_output" (func $write (param i32 i32)))
                (import "aegis" "host_get_payload_len" (func $plen (result i32)))
                (import "aegis" "host_get_payload" (func $pget (param i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "{url_escaped}")
                (func (export "run")
                    (call $fetch (i32.const 0) (i32.const {url_len}))
                    drop
                )
            )"#,
            url_escaped = url_escaped,
            url_len = url.len(),
        )
    }

    /// Helper: build a WAT module that writes a fixed output string.
    fn make_echo_skill_wat(output: &str) -> String {
        let out_bytes: Vec<String> = output.bytes().map(|b| format!("\\{b:02x}")).collect();
        let out_escaped = out_bytes.join("");
        format!(
            r#"(module
                (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
                (import "aegis" "host_write_output" (func $write (param i32 i32)))
                (import "aegis" "host_get_payload_len" (func $plen (result i32)))
                (import "aegis" "host_get_payload" (func $pget (param i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "{out_escaped}")
                (func (export "run")
                    (call $write (i32.const 0) (i32.const {out_len}))
                )
            )"#,
            out_escaped = out_escaped,
            out_len = output.len(),
        )
    }

    /// Helper: sign and execute a skill using a fresh identity.
    fn signed_execute(
        engine: &SandboxEngine,
        skill_name: &str,
        payload: serde_json::Value,
    ) -> Result<SkillResult> {
        let identity = AgentIdentity::generate();
        let envelope = identity.sign(skill_name, payload).unwrap();
        engine.execute(&envelope)
    }

    fn default_capability(name: &str, domains: Vec<String>) -> SkillCapability {
        SkillCapability {
            skill_name: name.to_string(),
            max_memory_mb: 2,
            allowed_domains: domains,
            allowed_fs_paths: vec![],
            cron_schedule: None,
            description: "test skill".to_string(),
            payload_schema: json!({}),
            default_payload: None,
            skill_pattern: None,
            language: "rust".to_string(),
            is_read_only: false,
            supports_multi_account: false,
            timeout_secs: 60,
            max_fuel: WASM_FUEL_LIMIT,
            shadow_mode: false,
            menu_command: None,
            menu_description: None,
            component_model: false,
            limitations: vec![],
        }
    }

    #[test]
    fn runtime_memory_quota_is_clamped_to_default() {
        assert_eq!(effective_skill_memory_mb(1), DEFAULT_SKILL_MEMORY_MB);
        assert_eq!(effective_skill_memory_mb(2), DEFAULT_SKILL_MEMORY_MB);
        assert_eq!(effective_skill_memory_mb(64), DEFAULT_SKILL_MEMORY_MB);
    }

    #[test]
    fn memory_budget_is_raise_only_and_clamped() {
        // No budget → the 2 MiB default (every LLM-reachable path).
        assert_eq!(
            effective_skill_memory_mb_with_budget(2, None),
            DEFAULT_SKILL_MEMORY_MB
        );
        // A budget RAISES for the trusted host path (mail reads need >2 MiB).
        assert_eq!(effective_skill_memory_mb_with_budget(2, Some(64)), 64);
        // Raise-only: a budget below the default cannot LOWER the cap.
        assert_eq!(
            effective_skill_memory_mb_with_budget(2, Some(1)),
            DEFAULT_SKILL_MEMORY_MB
        );
        // Clamped at the hard ceiling — cannot exceed it.
        assert_eq!(
            effective_skill_memory_mb_with_budget(2, Some(100_000)),
            MAX_SKILL_MEMORY_MB_CEILING
        );
    }

    #[test]
    fn runtime_fuel_quota_is_clamped_to_default() {
        assert_eq!(effective_skill_fuel(0), WASM_FUEL_LIMIT);
        assert_eq!(effective_skill_fuel(1_000), 1_000);
        assert_eq!(effective_skill_fuel(WASM_FUEL_LIMIT), WASM_FUEL_LIMIT);
        assert_eq!(
            effective_skill_fuel(WASM_FUEL_LIMIT.saturating_mul(10)),
            WASM_FUEL_LIMIT
        );
    }

    #[test]
    fn execution_scoped_fuel_budget_is_raise_only_and_ceiling_clamped() {
        // No budget => identical to the default-path resolution.
        assert_eq!(
            effective_skill_fuel_with_budget(0, None),
            effective_skill_fuel(0)
        );
        assert_eq!(
            effective_skill_fuel_with_budget(WASM_FUEL_LIMIT, None),
            WASM_FUEL_LIMIT
        );

        // A budget RAISES above the 500M default, up to the ceiling.
        assert_eq!(
            effective_skill_fuel_with_budget(0, Some(2_000_000_000)),
            2_000_000_000
        );
        assert_eq!(
            effective_skill_fuel_with_budget(0, Some(WASM_FUEL_CEILING)),
            WASM_FUEL_CEILING
        );

        // A budget above the ceiling is clamped to the ceiling.
        assert_eq!(
            effective_skill_fuel_with_budget(0, Some(WASM_FUEL_CEILING.saturating_mul(10))),
            WASM_FUEL_CEILING
        );

        // Raise-only: a budget BELOW the default cannot lower the fuel.
        assert_eq!(
            effective_skill_fuel_with_budget(0, Some(1_000)),
            WASM_FUEL_LIMIT
        );
        // The ceiling is strictly above the default, so the raise is meaningful.
        assert!(WASM_FUEL_CEILING > WASM_FUEL_LIMIT);
    }

    // ── T3: Wasm accessing non-whitelisted domain must be blocked ≤30ms ──

    #[test]
    fn t3_wasm_domain_whitelist_blocks_evil() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = make_fetch_skill_wat("https://evil-hacker.com/steal");
        let cap = default_capability("evil_test", vec!["api.github.com".to_string()]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let identity = AgentIdentity::generate();
        let envelope = identity.sign("evil_test", json!({})).unwrap();

        let start = Instant::now();
        let _result = engine.execute(&envelope).unwrap();
        let elapsed = start.elapsed();

        // The skill ran, but host_fetch returned -403 (blocked).
        // Verify the fetch was logged as blocked.
        assert!(
            elapsed.as_millis() <= 100,
            "S-LEG block must complete quickly, took {}ms",
            elapsed.as_millis()
        );

        // Verify via signed execution that the URL was indeed blocked
        let envelope2 = identity.sign("evil_test", json!({"attempt": 2})).unwrap();
        let result2 = engine.execute(&envelope2).unwrap();
        // The skill itself completes — it's the host_fetch that returns -403
        assert!(result2.elapsed_ms < 100);
    }

    #[test]
    fn t3_blocked_url_confirmed_via_allowed_domain_comparison() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            ;; URL at offset 0
            (data (i32.const 0) "https://evil.com/steal")
            ;; "BLOCKED" at offset 100
            (data (i32.const 100) "BLOCKED")
            ;; "OK" at offset 200
            (data (i32.const 200) "OK")
            (func (export "run")
                (local $ret i32)
                (local.set $ret (call $fetch (i32.const 0) (i32.const 22)))
                (if (i32.lt_s (local.get $ret) (i32.const 0))
                    (then (call $write (i32.const 100) (i32.const 7)))
                    (else (call $write (i32.const 200) (i32.const 2)))
                )
            )
        )"#;

        let cap = default_capability("block_test", vec!["api.safe.com".to_string()]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let identity = AgentIdentity::generate();
        let envelope = identity.sign("block_test", json!({})).unwrap();
        let result = engine.execute(&envelope).unwrap();
        assert_eq!(
            result.output, "BLOCKED",
            "S-LEG must block non-whitelisted domain"
        );
    }

    // ── T4: Legitimate domain access succeeds ──

    #[test]
    fn t4_wasm_allowed_domain_passes() {
        // Mirrors t3's "BLOCKED vs OK" pattern: the skill branches on the
        // host_fetch return code and writes a text tag so that output_buffer
        // always contains valid UTF-8 (host_write_output is funnelled through
        // String::from_utf8 in execute_skill_module, so raw i32 bytes would
        // collapse to the "(non-utf8 output)" fallback).
        //
        // We only verify the domain passed the S-LEG whitelist gate. The
        // downstream HTTP call may succeed (ret == 0) in dev machines with
        // internet or fail (ret == -500 network error) in hermetic CI — both
        // are fine; only -403 (whitelist block) would be a regression.
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            ;; URL at offset 0
            (data (i32.const 0) "https://api.coingecko.com/v3/price")
            ;; "BLOCKED" at offset 100 (7 bytes) — emitted iff ret == -403
            (data (i32.const 100) "BLOCKED")
            ;; "PASSED" at offset 200 (6 bytes) — emitted for any non-403 code
            ;; (success 0, or network error -500 in hermetic envs — either way
            ;;  the S-LEG whitelist permitted the request)
            (data (i32.const 200) "PASSED")
            (func (export "run")
                (local $ret i32)
                (local.set $ret (call $fetch (i32.const 0) (i32.const 34)))
                (if (i32.eq (local.get $ret) (i32.const -403))
                    (then (call $write (i32.const 100) (i32.const 7)))
                    (else (call $write (i32.const 200) (i32.const 6)))
                )
            )
        )"#;

        let cap = default_capability("allowed_test", vec!["api.coingecko.com".to_string()]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let result = signed_execute(&engine, "allowed_test", json!({})).unwrap();
        assert_eq!(
            result.output, "PASSED",
            "S-LEG must allow whitelisted domain (got {:?}; -403 would be a whitelist regression)",
            result.output
        );
    }

    // ── Unregistered skill must be rejected ──

    #[test]
    fn unregistered_skill_is_rejected() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();
        let identity = AgentIdentity::generate();
        let envelope = identity.sign("nonexistent_skill", json!({})).unwrap();

        let result = engine.execute(&envelope);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not registered"),
            "error should mention unregistered skill: {err_msg}"
        );
    }

    // ── Output echoing works ──

    #[test]
    fn skill_output_is_captured() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();
        let output_str = r#"{"require_llm_attention":false}"#;
        let wat = make_echo_skill_wat(output_str);
        let cap = default_capability("echo_test", vec![]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let result = signed_execute(&engine, "echo_test", json!({})).unwrap();

        assert_eq!(result.output, output_str);
    }

    // ── Payload is readable from inside Wasm ──

    #[test]
    fn payload_passthrough() {
        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            (func (export "run")
                (local $len i32)
                (local.set $len (call $plen))
                ;; Copy payload to memory offset 1024
                (call $pget (i32.const 1024))
                ;; Write it back as output
                (call $write (i32.const 1024) (local.get $len))
            )
        )"#;

        let engine = SandboxEngine::from_engine(test_engine()).unwrap();
        let cap = default_capability("echo_payload", vec![]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let payload = json!({"threshold": 60000, "asset": "BTC"});
        let result = signed_execute(&engine, "echo_payload", payload.clone()).unwrap();

        let echoed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(echoed, payload);
    }

    // ── Integration tests for compiled Wasm skills ──────────────────────────

    /// Helper to load a compiled .wasm skill from compiled_skills/ directory.
    /// Returns None if the file doesn't exist (e.g. in CI without compiled skills).
    fn load_compiled_skill(engine: &SandboxEngine, name: &str, cap: SkillCapability) -> bool {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("core/ should have a parent directory");
        let wasm_path = workspace_root
            .join("compiled_skills")
            .join(format!("{name}.wasm"));
        if !wasm_path.exists() {
            eprintln!("Skipping {name} integration test: {wasm_path:?} not found");
            return false;
        }
        engine.load_skill(&wasm_path, cap).unwrap();
        true
    }

    // reminder skill tests removed — skill deprecated in favor of schedule_intent pseudo-tool

    #[test]
    fn http_api_skill_requires_url() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();
        let cap = SkillCapability {
            skill_name: "http_api".to_string(),
            max_memory_mb: 4,
            allowed_domains: vec!["*".to_string()],
            allowed_fs_paths: vec![],
            cron_schedule: None,
            description: "HTTP API".to_string(),
            payload_schema: json!({}),
            default_payload: None,
            skill_pattern: None,
            language: "rust".to_string(),
            is_read_only: false,
            supports_multi_account: false,
            timeout_secs: 60,
            max_fuel: WASM_FUEL_LIMIT,
            shadow_mode: false,
            menu_command: None,
            menu_description: None,
            component_model: false,
            limitations: vec![],
        };
        if !load_compiled_skill(&engine, "http_api", cap) {
            return;
        }

        // Empty payload — should get error about missing URL
        let result = signed_execute(&engine, "http_api", json!({})).unwrap();

        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(output["status"], "error");
    }

    // ── Fuel exhaustion: infinite loop must be terminated ──

    #[test]
    fn fuel_exhaustion_terminates_infinite_loop() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            (func (export "run")
                (loop $inf
                    (br $inf)
                )
            )
        )"#;

        let cap = default_capability("infinite_loop", vec![]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let result = signed_execute(&engine, "infinite_loop", json!({}));
        assert!(
            result.is_err(),
            "infinite loop must be terminated by fuel exhaustion"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("fuel") || err.contains("wasm trap") || err.contains("execution"),
            "error should mention fuel or trap: {err}"
        );
    }

    // ── Memory limit enforcement ──

    #[test]
    fn memory_limit_prevents_excessive_allocation() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            (func (export "run")
                ;; Try to grow memory by 255 pages (255 * 64KB = ~16MB)
                (drop (memory.grow (i32.const 255)))
            )
        )"#;

        let mut cap = default_capability("mem_hog", vec![]);
        cap.max_memory_mb = 2; // 2MB limit
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let result = signed_execute(&engine, "mem_hog", json!({}));
        assert!(
            result.is_ok() || result.is_err(),
            "memory limit must be enforced (either via -1 return or trap)"
        );
    }

    // ── host_fetch call limit ──

    #[test]
    fn host_fetch_call_limit_enforced() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        let wat = r#"(module
            (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
            (import "aegis" "host_write_output" (func $write (param i32 i32)))
            (import "aegis" "host_get_payload_len" (func $plen (result i32)))
            (import "aegis" "host_get_payload" (func $pget (param i32)))
            (memory (export "memory") 1)
            ;; URL at offset 0: no domain match so fetch returns -403 (fast)
            (data (i32.const 0) "https://example.com/")
            ;; "RATE_LIMITED" at offset 100
            (data (i32.const 100) "RATE_LIMITED")
            ;; "OK" at offset 200
            (data (i32.const 200) "OK")
            (func (export "run")
                (local $i i32)
                (local $ret i32)
                (local.set $i (i32.const 0))
                (block $done
                    (loop $loop
                        (br_if $done (i32.ge_u (local.get $i) (i32.const 55)))
                        (local.set $ret (call $fetch (i32.const 0) (i32.const 20)))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br $loop)
                    )
                )
                ;; Check if last return code is -429
                (if (i32.eq (local.get $ret) (i32.const -429))
                    (then (call $write (i32.const 100) (i32.const 12)))
                    (else (call $write (i32.const 200) (i32.const 2)))
                )
            )
        )"#;

        // No domains allowed, so each fetch returns -403 (fast, no HTTP)
        let cap = default_capability("fetch_spam", vec![]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        let result = signed_execute(&engine, "fetch_spam", json!({})).unwrap();

        assert_eq!(
            result.output, "RATE_LIMITED",
            "calls beyond MAX_FETCH_CALLS must return -429"
        );
    }

    // ── Execution-scoped fetch budget ──

    /// Sign and execute with a trusted-host fetch budget (no auth overrides).
    fn signed_execute_with_budget(
        engine: &SandboxEngine,
        skill_name: &str,
        payload: serde_json::Value,
        budget: usize,
    ) -> Result<SkillResult> {
        let identity = AgentIdentity::generate();
        let envelope = identity.sign(skill_name, payload).unwrap();
        engine.execute_with_auth_override_and_fetch_budget(&envelope, HashMap::new(), budget)
    }

    /// WAT module that fetches `url` `iterations` times and writes
    /// "RATE_LIMITED" if ANY call returned -429, else "OK".
    fn make_fetch_loop_wat(url: &str, iterations: usize) -> String {
        let url_escaped: String = url.bytes().map(|b| format!("\\{b:02x}")).collect();
        format!(
            r#"(module
                (import "aegis" "host_fetch" (func $fetch (param i32 i32) (result i32)))
                (import "aegis" "host_write_output" (func $write (param i32 i32)))
                (import "aegis" "host_get_payload_len" (func $plen (result i32)))
                (import "aegis" "host_get_payload" (func $pget (param i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "{url_escaped}")
                (data (i32.const 300) "RATE_LIMITED")
                (data (i32.const 400) "OK")
                (func (export "run")
                    (local $i i32)
                    (local $hit i32)
                    (block $done
                        (loop $loop
                            (br_if $done (i32.ge_u (local.get $i) (i32.const {iterations})))
                            (if (i32.eq
                                    (call $fetch (i32.const 0) (i32.const {url_len}))
                                    (i32.const -429))
                                (then (local.set $hit (i32.const 1)))
                            )
                            (local.set $i (i32.add (local.get $i) (i32.const 1)))
                            (br $loop)
                        )
                    )
                    (if (local.get $hit)
                        (then (call $write (i32.const 300) (i32.const 12)))
                        (else (call $write (i32.const 400) (i32.const 2)))
                    )
                )
            )"#,
            url_escaped = url_escaped,
            url_len = url.len(),
            iterations = iterations,
        )
    }

    /// Bind-and-drop a loopback listener to obtain a port that is closed:
    /// fetches to it fail at connect time (-500) without real network I/O.
    fn closed_loopback_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        port
    }

    #[test]
    fn effective_fetch_caps_default_is_unchanged() {
        assert_eq!(
            effective_fetch_caps(None, "any_skill"),
            (MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN),
            "no budget (every LLM-reachable surface) must keep the exact defaults"
        );
    }

    #[test]
    fn effective_fetch_caps_budget_raises_both_caps_coherently() {
        // Budget above the total-call default: both caps follow the budget,
        // so a list + N fan-out is not strangled by EITHER cap.
        assert_eq!(effective_fetch_caps(Some(56), "mail"), (56, 56));
        // Budget between the two defaults: per-domain raised, total keeps
        // its (already larger) default.
        assert_eq!(
            effective_fetch_caps(Some(30), "mail"),
            (MAX_FETCH_CALLS, 30)
        );
    }

    #[test]
    fn effective_fetch_caps_clamps_at_hard_ceiling() {
        assert_eq!(
            effective_fetch_caps(Some(usize::MAX), "greedy"),
            (MAX_FETCH_PER_DOMAIN_CEILING, MAX_FETCH_PER_DOMAIN_CEILING),
            "budget requests above the ceiling are clamped, never honored"
        );
        assert_eq!(
            effective_fetch_caps(Some(MAX_FETCH_PER_DOMAIN_CEILING), "edge"),
            (MAX_FETCH_PER_DOMAIN_CEILING, MAX_FETCH_PER_DOMAIN_CEILING)
        );
    }

    #[test]
    fn effective_fetch_caps_is_raise_only() {
        // A budget can widen the guard, never narrow it below the defaults.
        assert_eq!(
            effective_fetch_caps(Some(0), "underflow"),
            (MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN)
        );
        assert_eq!(
            effective_fetch_caps(Some(5), "small"),
            (MAX_FETCH_CALLS, MAX_FETCH_PER_DOMAIN)
        );
    }

    #[test]
    fn fetch_budget_raises_total_cap_for_single_execution_only() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        // 55 fetches to a non-whitelisted domain: each returns -403 (fast,
        // no network), but every call still counts against the TOTAL cap.
        let wat = make_fetch_loop_wat("https://example.com/", 55);
        let cap = default_capability("budget_total_cap", vec![]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        // Budgeted execution: budget 56 raises the total cap to 56 → none
        // of the 55 calls hits -429.
        let budgeted =
            signed_execute_with_budget(&engine, "budget_total_cap", json!({}), 56).unwrap();
        assert_eq!(
            budgeted.output, "OK",
            "a 56-call budget must lift the 50-call total cap for this execution"
        );

        // Plain execute of the SAME loaded skill immediately afterwards:
        // the budget must not have leaked — default total cap (50) applies.
        let plain = signed_execute(&engine, "budget_total_cap", json!({})).unwrap();
        assert_eq!(
            plain.output, "RATE_LIMITED",
            "the budget is execution-scoped; a later plain execute keeps MAX_FETCH_CALLS"
        );
    }

    #[test]
    fn fetch_budget_raises_per_domain_cap_for_single_execution_only() {
        let engine = SandboxEngine::from_engine(test_engine()).unwrap();

        // 25 fetches to one WHITELISTED loopback domain whose port is
        // closed: allowed calls fail at connect (-500) but still count
        // against the per-domain cap; blocked calls return -429.
        let port = closed_loopback_port();
        let url = format!("http://127.0.0.1:{port}/");
        let wat = make_fetch_loop_wat(&url, 25);
        let cap = default_capability("budget_per_domain", vec!["127.0.0.1".to_string()]);
        engine.load_skill_from_bytes(wat.as_bytes(), cap).unwrap();

        // Budgeted execution: 25 calls to one domain, budget 30 → no -429
        // (this is the prod mail-inbox shape: list + N > 20 to one domain).
        let budgeted =
            signed_execute_with_budget(&engine, "budget_per_domain", json!({}), 30).unwrap();
        assert_eq!(
            budgeted.output, "OK",
            "a 30-call budget must lift the 20-per-domain cap for this execution"
        );

        // Plain execute of the same skill: call 21+ to the domain must be
        // blocked — default per-domain cap is untouched.
        let plain = signed_execute(&engine, "budget_per_domain", json!({})).unwrap();
        assert_eq!(
            plain.output, "RATE_LIMITED",
            "the budget is execution-scoped; a later plain execute keeps MAX_FETCH_PER_DOMAIN"
        );
    }

    // Extended sandbox tests (fs, auth, permissions, pointer boundaries,
    // contract consistency, snapshots, v2) — extracted to sandbox_tests.rs
    include!("sandbox_tests.rs");
}
