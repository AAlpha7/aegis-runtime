//! Skill loading, config validation, and auto-registration — extracted from
//! main.rs for Rule #15 compliance.

use super::*;
use aegis_runtime_core::startup_reasoner::{self, StartupReasoner};

/// Legacy unconditional location of `run_external` scratch directories. Kept
/// only when the directory already exists (installs made by `scripts/deploy.sh`);
/// a checkout that is not installed under `/opt` must not write there.
pub(super) const LEGACY_EXECUTOR_RUNS_DIR: &str = "/opt/aegis-runtime/shared/executor-runs";
const EXECUTOR_RUNS_DIR_NAME: &str = "executor-runs";
const HEIRLOOM_EVENT_LOG_FILE_NAME: &str = "heirloom_event_log.db";

pub(super) fn validate_config() -> Result<StartupReasoner> {
    // EMBEDDING_API_KEY is optional (defaults to OPENAI_API_KEY) but must not be
    // the template placeholder.
    startup_reasoner::validate_embedding_api_key(
        std::env::var("EMBEDDING_API_KEY").ok().as_deref(),
    )?;
    // grok build / Codex / OPENAI_API_KEY — one of three is required.
    let reasoner = startup_reasoner::resolve_startup_reasoner_from_env()?;

    // Validate TELOXIDE_TOKEN format if present
    if let Ok(token) = std::env::var("TELOXIDE_TOKEN")
        && (!token.contains(':') || token.len() < 20)
    {
        anyhow::bail!(
            "TELOXIDE_TOKEN format looks invalid (expected format: 123456789:ABC...). \
                 Get a valid token from @BotFather on Telegram."
        );
    }

    // Validate ADMIN_CHAT_ID is a number if present
    if let Ok(chat_id) = std::env::var("ADMIN_CHAT_ID")
        && chat_id != "0"
        && chat_id.parse::<i64>().is_err()
    {
        anyhow::bail!(
            "ADMIN_CHAT_ID must be a numeric Telegram chat ID (e.g. 123456789). \
                 Use @userinfobot on Telegram to find yours."
        );
    }

    // Validate ADMIN_API_TOKEN minimum length if present
    if let Ok(token) = std::env::var("ADMIN_API_TOKEN")
        && !token.is_empty()
        && token.len() < 16
    {
        anyhow::bail!(
            "ADMIN_API_TOKEN is too short ({} chars, minimum 16). \
                 Generate a strong token with: openssl rand -hex 32",
            token.len()
        );
    }

    Ok(reasoner)
}

/// Parent of the per-run scratch directories `run_external` creates
/// (`<runs_dir>/<run_id>`), resolved in this order:
///
/// 1. `AEGIS_EXECUTOR_RUNS_DIR` when set (trimmed, non-empty);
/// 2. `<directory of MEMORY_DB_PATH>/executor-runs` when `MEMORY_DB_PATH` is
///    set — scratch lives next to the data it belongs to, so an isolated boot
///    with its own DB never writes into another install's runs directory;
/// 3. the legacy [`LEGACY_EXECUTOR_RUNS_DIR`] when that directory already exists
///    (`dir_exists` is injected so the rule is unit-testable);
/// 4. `./executor-runs` relative to the working directory.
pub(super) fn resolve_executor_runs_dir(
    explicit: Option<&str>,
    memory_db_path: Option<&str>,
    dir_exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(dir) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(db) = memory_db_path.map(str::trim).filter(|s| !s.is_empty()) {
        let data_dir = Path::new(db)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        return data_dir.join(EXECUTOR_RUNS_DIR_NAME);
    }
    let legacy = Path::new(LEGACY_EXECUTOR_RUNS_DIR);
    if dir_exists(legacy) {
        return legacy.to_path_buf();
    }
    Path::new(".").join(EXECUTOR_RUNS_DIR_NAME)
}

/// Location of the heirloom event log (Phase E dual-write), resolved as:
///
/// 1. `HEIRLOOM_EVENT_LOG_DB` when set (trimmed, non-empty);
/// 2. `<directory of MEMORY_DB_PATH>/heirloom_event_log.db` — the event log
///    lives next to the memory database it mirrors. With the production unit
///    (`MEMORY_DB_PATH=/opt/aegis-runtime/shared/aegis_runtime_memory.db`) this is the
///    historical `/opt/aegis-runtime/shared/heirloom_event_log.db`; on a plain
///    checkout (default `./aegis_runtime_memory.db`) it is `./heirloom_event_log.db`,
///    so a fresh machine never points at an absent `/opt` tree.
pub(super) fn resolve_heirloom_event_log_path(
    explicit: Option<&str>,
    memory_db_path: &str,
) -> PathBuf {
    if let Some(path) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    let data_dir = Path::new(memory_db_path.trim())
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    data_dir.join(HEIRLOOM_EVENT_LOG_FILE_NAME)
}

/// Auto-register built-in skills (from `compiled_skills/`) in the `skill_proposals`
/// registry table.
///
/// For each `.wasm` file in the directory, if no proposal record exists for that skill_name
/// (regardless of status), we create one with status "active" and store the manifest JSON.
/// If a record already exists (active, deactivated, or any other status), we skip it to
/// respect the user's previous choice.
///
/// This must run BEFORE `load_skills_from_directory()` so the deactivated_skills set is
/// populated correctly from DB records.
pub(super) async fn auto_register_builtin_skills(
    memory: &MemoryHandle,
    compiled_dir: &Path,
) -> Result<()> {
    if !compiled_dir.exists() {
        return Ok(());
    }

    let entries: Vec<_> = std::fs::read_dir(compiled_dir)
        .context("reading compiled_skills directory for auto-registration")?
        .flatten()
        .collect();

    // Fetch all existing proposals once (any status) to avoid per-skill queries
    let all_proposals = memory
        .list_skill_proposals(None, None)
        .await
        .context("listing existing skill registry rows before built-in skill auto-registration")?;
    let existing_skill_names: std::collections::HashSet<String> =
        all_proposals.into_iter().map(|p| p.skill_name).collect();

    let workspace_root = Path::new(".");

    for entry in &entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }

        let skill_name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Skip if any record already exists — respect user's previous choice
        if existing_skill_names.contains(&skill_name) {
            continue;
        }

        // Read manifest if available
        let manifest_path = workspace_root
            .join("skills")
            .join(&skill_name)
            .join("manifest.json");
        let manifest_json = if manifest_path.exists() {
            match std::fs::read_to_string(&manifest_path) {
                Ok(json) => Some(json),
                Err(e) => {
                    eprintln!(
                        "[AutoRegister] Warning: failed to read manifest for '{skill_name}': {e}"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Create proposal record
        match memory
            .create_skill_proposal(
                &skill_name,
                "built-in",
                "Built-in skill auto-registered",
                None,
            )
            .await
        {
            Ok(id) => {
                // Update to active status with manifest
                let updates = SkillProposalUpdate {
                    status: Some(STATUS_ACTIVE.to_string()),
                    generated_manifest: manifest_json.map(Some),
                    ..Default::default()
                };
                if let Err(e) = memory.update_skill_proposal(id, updates).await {
                    eprintln!(
                        "[AutoRegister] Warning: created proposal for '{skill_name}' but failed to update: {e}"
                    );
                } else {
                    eprintln!("[AutoRegister] Registered built-in skill: {skill_name}");
                }
            }
            Err(e) => {
                eprintln!(
                    "[AutoRegister] Warning: failed to create proposal for '{skill_name}': {e}"
                );
            }
        }
    }

    Ok(())
}

/// Load Wasm skills from a directory, reading manifest.json for each.
/// Skills whose names appear in `deactivated` are skipped.
/// Uses rayon to parallelize manifest reading and validation (I/O-bound),
/// then loads wasm modules sequentially into the sandbox.
pub(super) fn load_skills_from_directory(
    sandbox: &SandboxEngine,
    dir: &Path,
    deactivated: &std::collections::HashSet<String>,
) -> Result<()> {
    use rayon::prelude::*;

    if !dir.exists() {
        eprintln!(
            "[Sandbox] Warning: {} not found, no skills loaded",
            dir.display()
        );
        return Ok(());
    }

    let workspace_root = Path::new(".");

    let entries: Vec<_> = std::fs::read_dir(dir)
        .context("reading compiled_skills directory")?
        .flatten()
        .collect();

    // Phase 1: Read manifests and validate in parallel (I/O-bound)
    let skill_files: Vec<Result<(PathBuf, SkillCapability)>> = entries
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                return None;
            }

            let skill_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Ignore the retired bundled search artifact on upgrades.
            if skill_name == "web_search" {
                return None;
            }

            // Skip deactivated skills
            if deactivated.contains(&skill_name) {
                eprintln!("[Sandbox] Skipping deactivated skill: {skill_name}");
                return None;
            }

            Some((path, skill_name))
        })
        .map(|(path, skill_name)| {
            let manifest_path = workspace_root
                .join("skills")
                .join(&skill_name)
                .join("manifest.json");

            let capability = if manifest_path.exists() {
                let json = std::fs::read_to_string(&manifest_path)
                    .with_context(|| format!("reading {}", manifest_path.display()))?;
                serde_json::from_str::<SkillCapability>(&json)
                    .with_context(|| format!("parsing {}", manifest_path.display()))?
            } else {
                eprintln!(
                    "[Sandbox] Warning: No manifest.json for '{}', using default (no network)",
                    skill_name
                );
                SkillCapability {
                    skill_name: skill_name.clone(),
                    max_memory_mb: 2,
                    allowed_domains: vec![],
                    allowed_fs_paths: vec![],
                    cron_schedule: None,
                    description: format!("Skill: {skill_name}"),
                    payload_schema: serde_json::json!({}),
                    default_payload: None,
                    skill_pattern: None,
                    language: "rust".to_string(),
                    is_read_only: false,
                    supports_multi_account: false,
                    timeout_secs: 60,
                    max_fuel: 500_000_000,
                    shadow_mode: false,
                    menu_command: None,
                    menu_description: None,
                    component_model: false,
                    limitations: vec![],
                }
            };

            validate_allowed_domains(&capability.skill_name, &capability.allowed_domains)?;

            Ok((path, capability))
        })
        .collect();

    // Phase 2: Load into sandbox sequentially (requires &self, fast after parallel I/O)
    for result in skill_files {
        let (wasm_path, capability) = result?;
        let skill_name = capability.skill_name.clone();

        // Log Wasm file timestamp for staleness debugging
        let wasm_age = std::fs::metadata(&wasm_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| {
                if d.as_secs() < 3600 {
                    format!("{}m ago", d.as_secs() / 60)
                } else if d.as_secs() < 86400 {
                    format!("{}h ago", d.as_secs() / 3600)
                } else {
                    format!("{}d ago", d.as_secs() / 86400)
                }
            })
            .unwrap_or_else(|| "unknown".into());

        // Check if source is newer than compiled Wasm
        let source_path = workspace_root
            .join("skills")
            .join(&skill_name)
            .join("src")
            .join("lib.rs");
        let source_newer = match (
            std::fs::metadata(&wasm_path).and_then(|m| m.modified()),
            std::fs::metadata(&source_path).and_then(|m| m.modified()),
        ) {
            (Ok(wasm_mtime), Ok(src_mtime)) if src_mtime > wasm_mtime => {
                eprintln!(
                    "[Sandbox] WARNING: {skill_name} source is newer than compiled Wasm — recompile needed!"
                );
                true
            }
            _ => false,
        };

        sandbox
            .load_skill(&wasm_path, capability)
            .with_context(|| format!("loading skill from {}", wasm_path.display()))?;
        eprintln!(
            "[Sandbox] Loaded skill: {skill_name} (wasm compiled {wasm_age}{})",
            if source_newer { ", STALE" } else { "" }
        );
    }

    Ok(())
}

/// Reject skills with overly broad domain whitelists (bare TLDs).
/// Prevents accidental or malicious manifests from granting near-universal network access.
pub(super) fn validate_allowed_domains(skill_name: &str, domains: &[String]) -> Result<()> {
    // Common TLDs that should never appear as standalone allowed_domains entries
    const TLDS: &[&str] = &[
        "com", "org", "net", "io", "dev", "cn", "uk", "de", "fr", "jp", "ru", "us", "eu", "co",
        "me", "info", "biz", "xyz", "app", "ai", "cc", "tv",
    ];
    for d in domains {
        if d == "*" {
            continue; // wildcard is intentional and explicit
        }
        if d.starts_with('.') {
            anyhow::bail!(
                "Skill '{}': allowed_domain '{}' must not start with '.'",
                skill_name,
                d
            );
        }
        if TLDS.contains(&d.to_lowercase().as_str()) {
            anyhow::bail!(
                "Skill '{}': allowed_domain '{}' is a bare TLD — too broad. \
                 Use specific domains (e.g. 'api.example.com') instead.",
                skill_name,
                d
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod config_validation_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn runs_dir_explicit_env_wins_over_everything() {
        let dir = resolve_executor_runs_dir(
            Some("  /srv/aegis/runs  "),
            Some("/opt/aegis-runtime/shared/aegis_runtime_memory.db"),
            |_| true,
        );
        assert_eq!(dir, PathBuf::from("/srv/aegis/runs"));
    }

    #[test]
    fn runs_dir_blank_explicit_env_is_ignored() {
        let dir = resolve_executor_runs_dir(Some("   "), Some("/data/memory.db"), |_| true);
        assert_eq!(dir, PathBuf::from("/data/executor-runs"));
    }

    #[test]
    fn runs_dir_follows_memory_db_directory_even_when_legacy_dir_exists() {
        let probed = RefCell::new(Vec::new());
        let dir = resolve_executor_runs_dir(None, Some("/var/lib/aegis/memory.db"), |p| {
            probed.borrow_mut().push(p.to_path_buf());
            true
        });
        assert_eq!(dir, PathBuf::from("/var/lib/aegis/executor-runs"));
        assert!(
            probed.borrow().is_empty(),
            "legacy dir must not even be consulted when MEMORY_DB_PATH decides"
        );
    }

    #[test]
    fn runs_dir_production_layout_is_unchanged() {
        let dir = resolve_executor_runs_dir(
            None,
            Some("/opt/aegis-runtime/shared/aegis_runtime_memory.db"),
            |_| true,
        );
        assert_eq!(dir, PathBuf::from(LEGACY_EXECUTOR_RUNS_DIR));
    }

    #[test]
    fn runs_dir_bare_db_filename_means_working_directory() {
        let dir = resolve_executor_runs_dir(None, Some("aegis_runtime_memory.db"), |_| true);
        assert_eq!(dir, PathBuf::from("./executor-runs"));
        let dir = resolve_executor_runs_dir(None, Some("./aegis_runtime_memory.db"), |_| true);
        assert_eq!(dir, PathBuf::from("./executor-runs"));
    }

    #[test]
    fn runs_dir_keeps_legacy_opt_path_only_when_it_exists() {
        let probed = RefCell::new(Vec::new());
        let kept = resolve_executor_runs_dir(None, None, |p| {
            probed.borrow_mut().push(p.to_path_buf());
            true
        });
        assert_eq!(kept, PathBuf::from(LEGACY_EXECUTOR_RUNS_DIR));
        assert_eq!(
            probed.borrow().as_slice(),
            &[PathBuf::from(LEGACY_EXECUTOR_RUNS_DIR)],
            "exactly the legacy directory is probed"
        );

        // Negative control: same inputs, directory absent → working directory.
        let fresh = resolve_executor_runs_dir(None, None, |_| false);
        assert_eq!(fresh, PathBuf::from("./executor-runs"));
        assert!(
            !fresh.starts_with("/opt"),
            "fresh checkout must not touch /opt"
        );
    }

    #[test]
    fn heirloom_path_explicit_env_wins() {
        let path = resolve_heirloom_event_log_path(
            Some("  /srv/aegis/heirloom.db  "),
            "/opt/aegis-runtime/shared/aegis_runtime_memory.db",
        );
        assert_eq!(path, PathBuf::from("/srv/aegis/heirloom.db"));
    }

    #[test]
    fn heirloom_path_blank_env_falls_back_to_memory_dir() {
        let path = resolve_heirloom_event_log_path(Some("   "), "/data/memory.db");
        assert_eq!(path, PathBuf::from("/data/heirloom_event_log.db"));
    }

    #[test]
    fn heirloom_path_production_layout_is_unchanged() {
        let path = resolve_heirloom_event_log_path(
            None,
            "/opt/aegis-runtime/shared/aegis_runtime_memory.db",
        );
        assert_eq!(
            path,
            PathBuf::from("/opt/aegis-runtime/shared/heirloom_event_log.db")
        );
    }

    #[test]
    fn heirloom_path_fresh_checkout_stays_in_working_directory() {
        for db in ["./aegis_runtime_memory.db", "aegis_runtime_memory.db"] {
            let path = resolve_heirloom_event_log_path(None, db);
            assert_eq!(path, PathBuf::from("./heirloom_event_log.db"), "db={db}");
            assert!(
                !path.starts_with("/opt"),
                "fresh checkout must not point at the author's /opt tree"
            );
        }
    }
}
