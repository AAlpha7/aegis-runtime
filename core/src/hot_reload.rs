//! Hot-reload file watcher for the compiled Wasm skills directory.
//!
//! Monitors the `compiled_skills/` directory for `.wasm` file changes and
//! auto-reloads the corresponding skill in the `SandboxEngine`.
//!
//! # Integration status: NOT YET WIRED
//!
//! Fully implemented with debouncing and error recovery, but not called from `main.rs`.
//! To activate, call `spawn_watcher()` after `SandboxEngine` is created in the
//! startup sequence, passing `Arc<SandboxEngine>`, `compiled_skills_dir`, and
//! `skills_dir` paths. Return `WatcherHandle` for graceful shutdown.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::models::SkillCapability;
use crate::sandbox::SandboxEngine;

/// Notification emitted after a skill is successfully hot-reloaded.
#[derive(Debug, Clone)]
pub struct ReloadEvent {
    pub skill_name: String,
    pub wasm_path: PathBuf,
    pub timestamp: Instant,
}

/// Handle returned by `spawn_watcher()` to control the background watcher.
pub struct WatcherHandle {
    /// Send a message on this channel to stop the watcher.
    shutdown_tx: mpsc::Sender<()>,
    /// The join handle for the background task.
    task: tokio::task::JoinHandle<()>,
}

impl WatcherHandle {
    /// Signal the watcher to stop and wait for it to finish.
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.task.await;
    }
}

/// Debounce window — events within this window for the same file are coalesced.
const DEBOUNCE_MS: u64 = 500;

/// Spawn a background watcher that monitors `compiled_skills_dir` for .wasm changes.
///
/// On each detected change, the watcher:
/// 1. Reads the new .wasm bytes
/// 2. Locates the corresponding `skills/<name>/manifest.json`
/// 3. Validates the module by loading it
/// 4. Atomically swaps the skill in the sandbox engine
/// 5. Sends a `ReloadEvent` on `notify_tx`
///
/// Returns a `WatcherHandle` to stop the watcher.
pub fn spawn_watcher(
    sandbox: Arc<SandboxEngine>,
    compiled_skills_dir: PathBuf,
    skills_dir: PathBuf,
    notify_tx: mpsc::Sender<ReloadEvent>,
) -> Result<WatcherHandle> {
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Channel for filesystem events from notify → our async task.
    let (fs_tx, mut fs_rx) = mpsc::channel::<Event>(256);

    // Create the notify watcher (synchronous callback → sends into tokio channel).
    let rt_handle = tokio::runtime::Handle::current();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| match res {
            Ok(event) => {
                let tx = fs_tx.clone();
                rt_handle.spawn(async move {
                    let _ = tx.send(event).await;
                });
            }
            Err(e) => {
                warn!("filesystem watcher error: {e:#}");
            }
        },
        Config::default(),
    )
    .context("creating filesystem watcher")?;

    watcher
        .watch(&compiled_skills_dir, RecursiveMode::NonRecursive)
        .context("watching compiled_skills directory")?;

    info!(
        path = %compiled_skills_dir.display(),
        "skill hot-reload watcher started"
    );

    let task = tokio::spawn(async move {
        // Keep watcher alive for the lifetime of the task.
        let _watcher = watcher;

        // Debounce state: path → last-seen time.
        let mut debounce: HashMap<PathBuf, Instant> = HashMap::new();
        // Pending debounced reloads.
        let mut pending: HashMap<PathBuf, Instant> = HashMap::new();

        let debounce_dur = Duration::from_millis(DEBOUNCE_MS);

        loop {
            // Tick every 100ms to process debounced events.
            let timeout = tokio::time::sleep(Duration::from_millis(100));
            tokio::pin!(timeout);

            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("skill hot-reload watcher shutting down");
                    break;
                }
                Some(event) = fs_rx.recv() => {
                    // Only care about Create and Modify events.
                    let dominated = matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_)
                    );
                    if !dominated {
                        continue;
                    }

                    for path in &event.paths {
                        if path.extension().is_none_or(|ext| ext != "wasm") {
                            continue;
                        }
                        debug!(path = %path.display(), kind = ?event.kind, "wasm file event");
                        let now = Instant::now();
                        debounce.insert(path.clone(), now);
                        pending.insert(path.clone(), now);
                    }
                }
                _ = &mut timeout => {
                    // Process debounced events.
                    let now = Instant::now();
                    let ready: Vec<PathBuf> = pending
                        .iter()
                        .filter(|(path, _)| {
                            debounce
                                .get(*path)
                                .is_none_or(|last| now.duration_since(*last) >= debounce_dur)
                        })
                        .map(|(path, _)| path.clone())
                        .collect();

                    for path in ready {
                        pending.remove(&path);
                        if let Err(e) = reload_skill(&sandbox, &path, &skills_dir, &notify_tx).await {
                            error!(
                                path = %path.display(),
                                error = %format!("{e:#}"),
                                "failed to hot-reload skill"
                            );
                        }
                    }
                }
            }
        }
    });

    Ok(WatcherHandle { shutdown_tx, task })
}

/// Derive the skill name from a .wasm file path (e.g. `crypto_tracker.wasm` → `crypto_tracker`).
fn skill_name_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

/// Load manifest for a skill from `skills/<name>/manifest.json`.
fn load_manifest(skills_dir: &Path, skill_name: &str) -> Result<SkillCapability> {
    let manifest_path = skills_dir.join(skill_name).join("manifest.json");
    let data = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading manifest at {}", manifest_path.display()))?;
    let cap: SkillCapability = serde_json::from_str(&data)
        .with_context(|| format!("parsing manifest for skill '{skill_name}'"))?;
    Ok(cap)
}

/// Perform a single skill reload: read wasm, load manifest, validate, swap.
async fn reload_skill(
    sandbox: &SandboxEngine,
    wasm_path: &Path,
    skills_dir: &Path,
    notify_tx: &mpsc::Sender<ReloadEvent>,
) -> Result<()> {
    let skill_name =
        skill_name_from_path(wasm_path).context("could not derive skill name from wasm path")?;

    // Check if file exists and is non-empty (writes may still be in progress).
    let metadata = tokio::fs::metadata(wasm_path)
        .await
        .with_context(|| format!("stat {}", wasm_path.display()))?;
    if metadata.len() == 0 {
        debug!(skill = %skill_name, "skipping empty wasm file");
        return Ok(());
    }

    let wasm_bytes = tokio::fs::read(wasm_path)
        .await
        .with_context(|| format!("reading wasm from {}", wasm_path.display()))?;

    let capability = load_manifest(skills_dir, &skill_name)?;

    let was_loaded = sandbox.is_skill_loaded(&skill_name);

    // hot_load_skill validates the module (compiles it) and swaps atomically.
    sandbox.hot_load_skill(&wasm_bytes, capability)?;

    let action = if was_loaded { "reloaded" } else { "loaded" };
    info!(
        skill = %skill_name,
        size_bytes = wasm_bytes.len(),
        "{action} via hot-reload"
    );

    let _ = notify_tx
        .send(ReloadEvent {
            skill_name,
            wasm_path: wasm_path.to_path_buf(),
            timestamp: Instant::now(),
        })
        .await;

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::LazyLock;
    use tempfile::TempDir;
    use wasmtime::Engine;

    static TEST_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        Engine::new(&config).expect("test engine init")
    });

    fn test_engine() -> Engine {
        TEST_ENGINE.clone()
    }

    /// Create a minimal valid manifest.json in the skills dir.
    fn write_manifest(skills_dir: &Path, skill_name: &str) {
        let dir = skills_dir.join(skill_name);
        fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "skill_name": skill_name,
            "max_memory_mb": 2,
            "allowed_domains": [],
            "allowed_fs_paths": [],
            "description": "test skill",
            "payload_schema": { "type": "object" },
            "language": "rust"
        });
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }

    #[test]
    fn test_skill_name_from_path() {
        assert_eq!(
            skill_name_from_path(Path::new("/tmp/compiled_skills/crypto_tracker.wasm")),
            Some("crypto_tracker".to_string())
        );
        assert_eq!(
            skill_name_from_path(Path::new("foo.wasm")),
            Some("foo".to_string())
        );
        assert_eq!(skill_name_from_path(Path::new("/")), None);
    }

    #[test]
    fn test_load_manifest_success() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path();
        write_manifest(skills_dir, "test_skill");

        let cap = load_manifest(skills_dir, "test_skill").unwrap();
        assert_eq!(cap.skill_name, "test_skill");
        assert_eq!(cap.max_memory_mb, 2);
    }

    #[test]
    fn test_load_manifest_missing() {
        let tmp = TempDir::new().unwrap();
        let err = load_manifest(tmp.path(), "nonexistent").unwrap_err();
        assert!(format!("{err:#}").contains("reading manifest"));
    }

    #[tokio::test]
    async fn test_watcher_detects_file_change() {
        let compiled_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        write_manifest(skills_dir.path(), "test_detect");

        let sandbox = Arc::new(SandboxEngine::from_engine(test_engine()).unwrap());
        let (notify_tx, mut notify_rx) = mpsc::channel::<ReloadEvent>(16);

        let handle = spawn_watcher(
            sandbox.clone(),
            compiled_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
            notify_tx,
        )
        .unwrap();

        // Write a minimal valid Wasm module.
        // This is the smallest valid Wasm binary: magic + version + empty.
        let minimal_wasm = b"\x00asm\x01\x00\x00\x00";
        let wasm_path = compiled_dir.path().join("test_detect.wasm");

        // Small delay to let the watcher start.
        tokio::time::sleep(Duration::from_millis(200)).await;
        fs::write(&wasm_path, minimal_wasm).unwrap();

        // Wait for the reload event (up to 3 seconds).
        let event = tokio::time::timeout(Duration::from_secs(3), notify_rx.recv()).await;
        match event {
            Ok(Some(evt)) => {
                assert_eq!(evt.skill_name, "test_detect");
            }
            Ok(None) => panic!("channel closed without event"),
            Err(e) => panic!("timed out waiting for reload event: {e}"),
        }

        handle.stop().await;
    }

    #[tokio::test]
    async fn test_watcher_survives_invalid_wasm() {
        let compiled_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        write_manifest(skills_dir.path(), "bad_skill");

        let sandbox = Arc::new(SandboxEngine::from_engine(test_engine()).unwrap());
        let (notify_tx, mut notify_rx) = mpsc::channel::<ReloadEvent>(16);

        let handle = spawn_watcher(
            sandbox.clone(),
            compiled_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
            notify_tx,
        )
        .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Write invalid wasm bytes.
        let wasm_path = compiled_dir.path().join("bad_skill.wasm");
        fs::write(&wasm_path, b"this is not valid wasm").unwrap();

        // Should NOT receive a reload event (the error is logged, not propagated).
        let event = tokio::time::timeout(Duration::from_secs(2), notify_rx.recv()).await;
        assert!(
            event.is_err(),
            "should not get a reload event for invalid wasm"
        );

        // The watcher should still be alive. Write a valid skill to prove it.
        write_manifest(skills_dir.path(), "good_skill");
        let minimal_wasm = b"\x00asm\x01\x00\x00\x00";
        let good_path = compiled_dir.path().join("good_skill.wasm");
        fs::write(&good_path, minimal_wasm).unwrap();

        let event = tokio::time::timeout(Duration::from_secs(3), notify_rx.recv()).await;
        match event {
            Ok(Some(evt)) => assert_eq!(evt.skill_name, "good_skill"),
            _ => panic!("watcher died after invalid wasm — should have survived"),
        }

        handle.stop().await;
    }

    #[tokio::test]
    async fn test_watcher_skips_empty_file() {
        let compiled_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        write_manifest(skills_dir.path(), "empty_skill");

        let sandbox = Arc::new(SandboxEngine::from_engine(test_engine()).unwrap());
        let (notify_tx, mut notify_rx) = mpsc::channel::<ReloadEvent>(16);

        let handle = spawn_watcher(
            sandbox.clone(),
            compiled_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
            notify_tx,
        )
        .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Write an empty file (mid-write scenario).
        let wasm_path = compiled_dir.path().join("empty_skill.wasm");
        fs::write(&wasm_path, b"").unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), notify_rx.recv()).await;
        assert!(event.is_err(), "should skip empty wasm file");

        handle.stop().await;
    }

    #[test]
    fn test_load_manifest_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.json"), "not json").unwrap();

        let err = load_manifest(tmp.path(), "broken").unwrap_err();
        assert!(format!("{err:#}").contains("parsing manifest"));
    }
}
