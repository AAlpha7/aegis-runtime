// build.rs — Auto-compile Wasm skills from skills/ directory
//
// For each skill crate under ../skills/, runs:
//   cargo build --target wasm32-wasip1 --release
// and copies the .wasm output to ../compiled_skills/
//
// Environment variables:
//   SKIP_SKILL_BUILD=1  — Skip all WASM skill compilation (fast dev builds)
//   SKILL_BUILD_ONLY=x  — Only build skill "x" (comma-separated for multiple)

use std::path::Path;
use std::process::Command;

fn main() {
    // ── GENESIS_PREV_HASH pin assert ───────────────────────────────────
    // Phase A P1.W1 / spec §2 Q2.2 (Round-9 polish P2.3).
    // Fails the build if core/src/heirloom/genesis.rs pinned constant
    // drifts from blake3(b"aegis:heirloom:genesis:v1").
    //
    // Implementation: extract every `0xNN` byte literal from genesis.rs
    // in source order (regardless of line wrapping — rustfmt may reformat
    // 8-bytes-per-line ↔ 16-bytes-per-line at default width), then
    // compare the resulting byte sequence against the actual blake3 hash.
    {
        let actual = *blake3::hash(b"aegis:heirloom:genesis:v1").as_bytes();
        let source_path = Path::new("src/heirloom/genesis.rs");
        let source = std::fs::read_to_string(source_path)
            .expect("core/src/heirloom/genesis.rs must exist for pin assert");

        // Narrow to the GENESIS_PREV_HASH constant body to avoid matching
        // 0xNN literals from other constants or comments elsewhere in the
        // file. Match the full `pub const GENESIS_PREV_HASH` declaration so
        // any doc-comment mentions of the name (even ones containing 0xNN
        // literal examples) don't poison the scan window.
        let marker = "pub const GENESIS_PREV_HASH";
        let const_start = source
            .find(marker)
            .expect("genesis.rs must declare pub const GENESIS_PREV_HASH");
        let slice_from_marker = &source[const_start..];
        let end = slice_from_marker
            .find("];")
            .expect("genesis.rs GENESIS_PREV_HASH array must close with ];");
        let body = &slice_from_marker[..end];

        // Extract every 0xNN hex byte literal in body order.
        let mut extracted = Vec::with_capacity(32);
        let bytes = body.as_bytes();
        let mut i = 0;
        while i + 3 < bytes.len() {
            if bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                let hi = (bytes[i + 2] as char).to_digit(16);
                let lo = (bytes[i + 3] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    extracted.push((h * 16 + l) as u8);
                    i += 4;
                    continue;
                }
            }
            i += 1;
        }

        if extracted.len() != 32 || extracted.as_slice() != actual.as_slice() {
            eprintln!(
                "\n\n╔════════════════════════════════════════════════════════════╗\n\
                 ║  GENESIS_PREV_HASH pin drift — build blocked              ║\n\
                 ╚════════════════════════════════════════════════════════════╝\n\
                 \n\
                 core/src/heirloom/genesis.rs must contain exactly 32 hex\n\
                 byte literals equalling blake3(b\"aegis:heirloom:genesis:v1\").\n\
                 \n\
                 Expected (hex): {expected_hex}\n\
                 Found    (hex): {actual_hex}\n\
                 Found {count} byte literals in the constant body.\n\
                 ",
                expected_hex = actual
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                actual_hex = extracted
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                count = extracted.len(),
            );
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed=src/heirloom/genesis.rs");
    }

    // Allow skipping WASM skill compilation entirely for fast dev builds.
    // Skills in compiled_skills/ are still loaded at runtime; this just skips
    // recompiling them. Useful when only changing core/ Rust code.
    println!("cargo:rerun-if-env-changed=SKIP_SKILL_BUILD");
    println!("cargo:rerun-if-env-changed=SKILL_BUILD_ONLY");

    if std::env::var("SKIP_SKILL_BUILD").is_ok_and(|v| v == "1" || v == "true") {
        println!("cargo:warning=SKIP_SKILL_BUILD=1: skipping all WASM skill compilation");
        return;
    }

    // IMPORTANT: resolve the manifest directory at build-script runtime, not
    // with `env!()`. In a shared target-dir / multi-worktree setup, `env!`
    // bakes whichever worktree path compiled the build-script binary first,
    // and later invocations can silently keep copying skills into the wrong
    // worktree's `compiled_skills/` directory.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set for core/build.rs");
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .expect("core crate must have a workspace parent");
    let skills_dir = workspace_root.join("skills");
    let output_dir = workspace_root.join("compiled_skills");

    // Create output directory
    std::fs::create_dir_all(&output_dir).expect("failed to create compiled_skills/");

    // Optional filter: only build specific skills
    let only_skills: Option<Vec<String>> = std::env::var("SKILL_BUILD_ONLY")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());

    // Find all skill crates
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        println!("cargo:warning=No skills/ directory found, skipping Wasm compilation");
        return;
    };

    for entry in entries.flatten() {
        let skill_path = entry.path();
        let cargo_toml = skill_path.join("Cargo.toml");

        if !cargo_toml.exists() {
            continue;
        }

        let skill_name = skill_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Only crates that declare `crate-type = ["cdylib"]` are standalone
        // skills. Shared library crates (e.g. aegis_runtime_wasm_core, which
        // the real skills depend on) build fine but produce no .wasm — skip
        // them instead of emitting a confusing "Wasm output not found".
        let toml_text = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
        if !toml_text.contains("cdylib") {
            continue;
        }

        // Apply SKILL_BUILD_ONLY filter
        if let Some(only) = &only_skills
            && !only.iter().any(|s| s == &skill_name)
        {
            continue;
        }

        // Rerun if skill source changes
        println!(
            "cargo:rerun-if-changed={}",
            skill_path.join("src").display()
        );
        println!("cargo:rerun-if-changed={}", cargo_toml.display());

        // Skip recompilation if .wasm is newer than all source files.
        // This avoids redundant cargo invocations when nothing changed.
        let wasm_dst = output_dir.join(format!("{skill_name}.wasm"));
        if wasm_dst.exists()
            && let Ok(wasm_meta) = std::fs::metadata(&wasm_dst)
            && let Ok(wasm_mtime) = wasm_meta.modified()
        {
            let src_dir = skill_path.join("src");
            let needs_rebuild =
                is_any_newer(&src_dir, wasm_mtime) || is_file_newer(&cargo_toml, wasm_mtime);
            if !needs_rebuild {
                // Already up to date
                continue;
            }
        }

        // Build the skill for wasm32-wasip1
        // Use a separate target directory to avoid cargo lock deadlock
        // when build.rs invokes nested cargo builds.
        //
        // Host RUSTFLAGS must NOT leak into this nested wasm build: deploy.sh
        // exports `-C linker=clang -C link-arg=-fuse-ld=mold` (aarch64 host
        // linker flags) as an env var, which applies to EVERY target — the
        // wasm32-wasip1 link then dies with `clang: invalid linker name
        // '-fuse-ld=mold'`, and the failed skill build forces a full
        // aegis-runtime-core recompile on every deploy. Setting the vars to ""
        // (rather than removing them) also masks any config-level rustflags,
        // which a wasm build never wants either.
        let wasm_target_dir = workspace_root.join("target").join("wasm_skills");
        let status = Command::new("cargo")
            .args([
                "build",
                "--target",
                "wasm32-wasip1",
                "--release",
                "--manifest-path",
            ])
            .arg(&cargo_toml)
            .arg("--target-dir")
            .arg(&wasm_target_dir)
            .env("RUSTFLAGS", "")
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .status();

        match status {
            Ok(s) if s.success() => {
                // Copy .wasm to compiled_skills/
                // The wasm file name uses hyphens replaced with underscores
                let wasm_name = skill_name.replace('-', "_");
                let wasm_src = wasm_target_dir
                    .join("wasm32-wasip1")
                    .join("release")
                    .join(format!("{wasm_name}.wasm"));

                if wasm_src.exists() {
                    std::fs::copy(&wasm_src, &wasm_dst).unwrap_or_else(|e| {
                        panic!(
                            "failed to copy {} to {}: {e}",
                            wasm_src.display(),
                            wasm_dst.display()
                        )
                    });
                    println!(
                        "cargo:warning=Compiled skill: {} -> {}",
                        skill_name,
                        wasm_dst.display()
                    );
                } else {
                    println!(
                        "cargo:warning=Wasm output not found at {}, skipping copy",
                        wasm_src.display()
                    );
                }
            }
            Ok(s) => {
                println!(
                    "cargo:warning=Failed to compile skill '{skill_name}' (exit code: {:?})",
                    s.code()
                );
            }
            Err(e) => {
                println!("cargo:warning=Failed to run cargo for skill '{skill_name}': {e}");
            }
        }
    }
}

/// Check if any file under `dir` is newer than `threshold`.
fn is_any_newer(dir: &Path, threshold: std::time::SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true; // Can't read → assume needs rebuild
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if is_any_newer(&path, threshold) {
                return true;
            }
        } else if is_file_newer(&path, threshold) {
            return true;
        }
    }
    false
}

/// Check if a single file is newer than `threshold`.
fn is_file_newer(path: &Path, threshold: std::time::SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_or(true, |mtime| mtime > threshold)
}
