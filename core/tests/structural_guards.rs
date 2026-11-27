//! Structural Guards — regression tests that catch dangerous structural drift.
//!
//! These tests fail when code structure changes in ways that require human review:
//! adding/removing tools, skills, snapshots, gate policies, or i18n keys.
//!
//! They do NOT test logic — they test that the safety net hasn't shrunk.
//! When a guard trips, update the expected constant AND verify the change is intentional.

mod mock_helpers;

use std::collections::HashSet;
use std::path::PathBuf;

/// Path to compiled Wasm skills directory.
fn compiled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("compiled_skills")
}

/// Path to snapshot files directory.
fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("snapshots")
}

// ── Guard 1: Tool Schema Count ──────────────────────────────────────────────
//
// Every pseudo-tool builder function in tool_schemas.rs produces a schema.
// mcp_pseudo_tool_schemas() collects them all. If this count changes, a tool
// was added or removed — force a human to verify.

/// The number of pseudo-tools exposed via MCP (and thus available to the LLM).
/// Update this constant when intentionally adding or removing a tool.
/// 49 → 51 (2026-07-08, S1 sensor audit): `maps_eta` (0c78afba) and
/// `historical mailbox-account tool` (63a9ee51) shipped without bumping this guard —
/// caught because the S1 verification ran this binary (it is NOT part of the
/// `--lib` pre-commit gate). Both additions verified intentional.
/// 51 → 52 (2026-07-30, aegis-guard-count-reconcile): `x_search` added in
/// 7dbb9a39 (Phase A public X search via Grok) — one builder
/// (`build_x_search_tool`) registered in `mcp_pseudo_tool_schemas()`; ToolGate
/// policy + heartbeat allowlist were wired in that commit, so only this count
/// was left stale (main's `guard_mcp_pseudo_tool_count_stable` red).
/// 52 → 49 (2026-09-04, controlled-self-evolution removal): `manage_initiative`,
/// `research_topic` and `manage_research` deleted with the initiative and
/// research engines.
/// 49 → 48 (2026-09-04, browser-automation removal): `maps_eta` deleted with
/// the host-level headless browser it read Google Maps through (`browser_agent`
/// itself was never in `mcp_pseudo_tool_schemas`).
/// 43 → 37 (2026-09-07, mail-stack removal): mailbox pseudo-tools deleted
/// (`gmail_sender`, `gmail_mark_read`, `send_draft`, `list_drafts`,
/// `list_mail_accounts`, `manage_imap_account`); `setup_gmail` renamed
/// `setup_google`.
/// 37 → 38 (2026-09-07, run_external): `build_run_external_tool` registered
/// so the LLM can hand a self-contained task to grok build / codex.
/// 38 → 39 (2026-09-08, browser_fetch): `build_browser_fetch_tool` registered
/// so the LLM can render one page in the machine's headless Chrome.
/// 39 → 40 (2026-09-20, secure_computer_use): Linux AT-SPI / loopback CU path.
const EXPECTED_MCP_PSEUDO_TOOL_COUNT: usize = 40;

#[test]
fn guard_mcp_pseudo_tool_count_stable() {
    let schemas = aegis_runtime_core::tool_schemas::mcp_pseudo_tool_schemas();
    assert_eq!(
        schemas.len(),
        EXPECTED_MCP_PSEUDO_TOOL_COUNT,
        "MCP pseudo-tool count changed from {EXPECTED_MCP_PSEUDO_TOOL_COUNT} to {}.\n\
         If you added a tool: update EXPECTED_MCP_PSEUDO_TOOL_COUNT.\n\
         If you removed a tool: verify this is intentional, then update the constant.",
        schemas.len()
    );
}

// ── Guard 2: ToolGate Policy Count ──────────────────────────────────────────
//
// Every tool MUST have an explicit ToolPolicy. If a tool is added without a
// policy, it falls through to the default (potentially unsafe). This guard
// catches policy count drift separate from hitl_bypass_detection.rs which
// checks specific tool names.

/// Tools that are intentionally ungated (read-only/display/internal).
/// mcp_pseudo_tool_schemas() includes these but ToolGate only tracks tools
/// that need explicit Allow/Block/RequireApproval policies.
/// This test ensures the set of ungated tools doesn't grow unexpectedly.
const KNOWN_UNGATED_TOOLS: &[&str] = &[
    "check_api_usage",
    "generate_image",
    "list_alert_policies",
    "list_intents",
    "list_monitoring_intents",
    "search_tools",
    "view_heartbeat_audit",
    "view_logs",
];

#[test]
fn guard_tool_gate_ungated_set_stable() {
    let gate = aegis_runtime_core::tool_gate::ToolGate::new();
    let schemas = aegis_runtime_core::tool_schemas::mcp_pseudo_tool_schemas();
    let known: HashSet<&str> = KNOWN_UNGATED_TOOLS.iter().copied().collect();

    let mut new_ungated = Vec::new();
    for schema in &schemas {
        let name = match schema {
            async_openai::types::chat::ChatCompletionTools::Function(f) => &f.function.name,
            _ => continue,
        };
        if !gate.has_policy(name) && !known.contains(name.as_str()) {
            new_ungated.push(name.clone());
        }
    }

    assert!(
        new_ungated.is_empty(),
        "New MCP pseudo-tools WITHOUT a ToolGate policy: {new_ungated:?}\n\
         If read-only: add to KNOWN_UNGATED_TOOLS in this test.\n\
         If state-modifying: add a ToolPolicy in tool_gate.rs::default_policies()."
    );
}

// ── Guard 3: Compiled Wasm Skill Count ──────────────────────────────────────

/// Curated OSS snapshot: `test_no_net` and `web_reader`.
const EXPECTED_WASM_SKILL_COUNT: usize = 2;

#[test]
fn guard_compiled_wasm_skill_count_stable() {
    let dir = compiled_skills_dir();
    let count = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", dir.display()))
        .filter(|entry| {
            entry
                .as_ref()
                .map(|e| e.path().extension().is_some_and(|ext| ext == "wasm"))
                .unwrap_or(false)
        })
        .count();

    assert_eq!(
        count, EXPECTED_WASM_SKILL_COUNT,
        "Compiled Wasm skill count changed from {EXPECTED_WASM_SKILL_COUNT} to {count}.\n\
         If you added a skill: update EXPECTED_WASM_SKILL_COUNT.\n\
         If one is missing: a build may have failed or a skill was deleted."
    );
}

// ── Guard 4: Snapshot File Count ────────────────────────────────────────────

/// 104 → 100 (2026-09-04, browser-automation removal): the four browser
/// snapshots went with the feature — `prompt_module_browser`,
/// `browser_agent_tool`, and the two `summarize_browser_*` approval summaries.
/// 100 → 92 (2026-09-05, SkillForge removal): the eight SkillForge snapshots
/// went with the feature — `prompt_module_skillforge`, the propose/approve/
/// status/delete/fix/retry tool schemas, and `summarize_delete_skill_*`.
/// 92 → 83 (2026-09-07, mail-stack removal): mailbox tool/prompt snapshots
/// deleted with the mail client.
/// 83 → 85 (2026-09-07, run_external): prompt module + tool schema snapshots.
// 2026-09-15: four SDUI snapshots (render_webapp / manage_panels tool schemas,
// sdui / panels prompt modules) were retired on purpose; 87 -> 83.
const EXPECTED_MIN_SNAPSHOT_COUNT: usize = 83;

#[test]
fn guard_snapshot_file_count_stable() {
    // Strangler Phase 0 (2026-07) moved test modules into subdirectories, so
    // snapshots now live in MULTIPLE source-relative `snapshots/` dirs (e.g.
    // src/snapshots/, src/orchestrator/tests/snapshots/). Count recursively
    // under src/ so future splits can't silently shrink the guarded set.
    fn count_snaps(dir: &std::path::Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| {
                let path = e.path();
                if path.is_dir() {
                    count_snaps(&path)
                } else {
                    let name = path.file_name().unwrap().to_str().unwrap();
                    usize::from(name.ends_with(".snap") && !name.ends_with(".snap.new"))
                }
            })
            .sum()
    }
    let dir = snapshots_dir()
        .parent()
        .expect("snapshots_dir has a parent (src/)")
        .to_path_buf();
    let count = count_snaps(&dir);

    assert!(
        count >= EXPECTED_MIN_SNAPSHOT_COUNT,
        "Snapshot count dropped to {count} (expected >= {EXPECTED_MIN_SNAPSHOT_COUNT}). \
         Snapshots may have been accidentally deleted."
    );
}

// ── Guard 5: i18n Key Completeness ──────────────────────────────────────────
//
// Every t("key") call in the codebase must have a corresponding translation.
// We check that the translation table has a minimum number of entries.

#[test]
fn guard_i18n_key_count_stable() {
    // i18n::available_keys() or similar — check that the translation table
    // hasn't shrunk. We test indirectly via known keys.
    let known_keys = [
        "telegram.audio_not_supported",
        "telegram.welcome",
        "telegram.error_generic",
    ];

    for key in &known_keys {
        let translated = aegis_runtime_core::i18n::t(key, "zh");
        assert!(
            !translated.is_empty() && translated != *key,
            "i18n key '{key}' returned empty or untranslated. \
             Translation may have been removed."
        );
    }
}

// ── Guard 7: Heartbeat allowlist ⇄ ToolGate exhaustive sync (Sensor 4) ──────
//
// CLAUDE.md anti-pattern: "if a tool is in the heartbeat allowed_tools list
// (chronos.rs), it MUST also be Allow-compatible in tool_gate.rs
// default_policies()" — otherwise the tool is offered to the heartbeat LLM
// but blocked at execution (wasted completion tokens + redundant fallback).
// Until 2026-07-08 only ~6 named tools were spot-checked (step8a/9b/10
// tests); this guard closes the drift class EXHAUSTIVELY: every tool in
// default_autonomous_allowed_tools() must not evaluate to Block under the
// Heartbeat caller context.

#[test]
fn guard_heartbeat_allowlist_never_blocked_by_tool_gate() {
    use aegis_runtime_core::tool_gate::{CallerContext, GateVerdict, ToolGate};

    let gate = ToolGate::new();
    let allow_list = aegis_runtime_core::chronos::default_autonomous_allowed_tools();
    assert!(
        allow_list.len() >= 20,
        "default_autonomous_allowed_tools shrank to {} entries — verify intentional",
        allow_list.len()
    );

    let mut blocked = Vec::new();
    for tool in &allow_list {
        // "{}" args: the heartbeat-scope decision must not depend on
        // argument content (argument-level guards may still add
        // NeedApproval, which is fine — only Block is a wiring bug).
        let verdict = gate.evaluate(tool, "{}", CallerContext::Heartbeat);
        if matches!(verdict, GateVerdict::Block { .. }) {
            blocked.push(format!("{tool}: {verdict:?}"));
        }
    }

    assert!(
        blocked.is_empty(),
        "Tool(s) on the heartbeat allowlist (chronos.rs \
         default_autonomous_allowed_tools) are BLOCKED by the ToolGate under \
         CallerContext::Heartbeat — the heartbeat LLM will be offered a tool \
         it can never execute:\n  {}\n\
         Fix: give the tool an Allow/HeartbeatAllowed policy in \
         tool_gate.rs::default_policies(), or remove it from the chronos \
         allowlist.",
        blocked.join("\n  ")
    );
}

// ── Guard 6: All tool_schemas build functions succeed ────────────────────────
//
// Every build_*_tool() function in tool_schemas.rs must successfully produce
// a valid schema. This catches schema construction bugs early.

#[test]
fn guard_all_tool_schema_builders_succeed() {
    // mcp_pseudo_tool_schemas() calls every build_*_tool() and collects results.
    // If any builder panics or returns Err, this test will fail.
    let schemas = aegis_runtime_core::tool_schemas::mcp_pseudo_tool_schemas();

    // Verify each schema has a valid function name
    let mut names = HashSet::new();
    for schema in &schemas {
        let name = match schema {
            async_openai::types::chat::ChatCompletionTools::Function(f) => f.function.name.clone(),
            _ => continue,
        };
        assert!(!name.is_empty(), "A tool schema has an empty function name");
        assert!(
            names.insert(name.clone()),
            "Duplicate tool schema name: {name}"
        );
    }
}

// ── Guard 8: no hard-coded model ids outside the allowlist ──────────────────
//
// Provider model ids belong in settings defaults, the model catalog, test
// fixtures, and migration strings — not in UI presets or call-site literals.
// `webapp/src/vnext/SettingsView.tsx` `LLM_MODEL_PRESETS` is intentionally
// NOT allowlisted: the model-catalog job removes it.

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core/ has a parent")
        .to_path_buf()
}

fn path_is_model_id_allowlisted(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    if rel.contains("/tests/")
        || rel.contains("_tests.rs")
        || rel.contains("/admin_tests/")
        || rel.contains("testdata/")
    {
        return true;
    }
    const ALLOWED_FILES: &[&str] = &[
        "core/src/settings.rs",
        "core/src/model_catalog.rs",
        "core/src/memory_migrations.rs",
        "core/src/llm_pricing.rs",
        "core/src/model_capabilities.rs",
        "core/src/llm_client.rs",
        "core/src/llm/xai_subscription.rs",
        "core/src/llm/openai_compat.rs",
        "core/src/llm/codex_oauth.rs",
        "core/src/llm/mod.rs",
        "core/src/llm/backend.rs",
        "core/src/llm/types.rs",
        "core/src/llm/router.rs",
        "core/src/admin/settings_routes.rs",
        "core/src/admin/ops_endpoints.rs",
        "core/src/admin/webapp_gateway.rs",
        "core/src/admin/vnext_projections.rs",
        "core/src/multimodal.rs",
        "core/src/orchestrator/mod.rs",
        "core/src/orchestrator/heartbeat.rs",
        "core/src/orchestrator/message_flow.rs",
        "core/src/orchestrator/lifecycle.rs",
        "core/src/orchestrator_tools_external.rs",
        "core/src/orchestrator_tools_system.rs",
        "core/src/orchestrator_tools_web.rs",
        "core/src/external_executor/mod.rs",
        "core/src/external_executor/grok_acp.rs",
        "core/src/tool_schemas.rs",
        "core/src/data_access_gate.rs",
        "core/src/embedding.rs",
        "core/src/embedding_corpus.rs",
        "core/src/progress_router.rs",
        "core/src/assistant_work_items.rs",
        "core/src/main.rs",
        "core/src/source_guards.rs",
        "core/src/memory/handlers_b.rs",
        "core/src/memory/mod.rs",
        "core/src/prompts.rs",
        "core/src/openrouter.rs",
        "webapp/src/api.ts",
        "webapp/src/i18n/catalog.ts",
    ];
    ALLOWED_FILES.iter().any(|allowed| rel == *allowed)
}

fn line_is_model_id_allowlisted(rel: &str, line: &str) -> bool {
    if path_is_model_id_allowlisted(rel) {
        return true;
    }
    let rel = rel.replace('\\', "/");
    // Codex default is the settings.rs value, not a catalog preset.
    rel.ends_with("webapp/src/vnext/SettingsView.tsx")
        && line.contains("gpt-5.5")
        && !line.contains("deepseek/")
        && !line.contains("google/gemini")
}

fn contains_prefix_then_digit(haystack: &str, prefix: &str) -> bool {
    let mut rest = haystack;
    while let Some(idx) = rest.find(prefix) {
        let after = &rest[idx + prefix.len()..];
        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return true;
        }
        rest = &rest[idx + 1..];
    }
    false
}

fn line_has_hardcoded_model_id(line: &str) -> bool {
    const LITERALS: &[&str] = &[
        "google/gemini",
        "deepseek/",
        "openai/gpt-",
        "anthropic/",
        "x-ai/",
    ];
    LITERALS.iter().any(|needle| line.contains(needle))
        || contains_prefix_then_digit(line, "grok-")
        || contains_prefix_then_digit(line, "gpt-")
}

fn collect_hardcoded_model_id_hits(
    dir: &std::path::Path,
    rel_prefix: &str,
    hits: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            collect_hardcoded_model_id_hits(&path, &format!("{rel_prefix}/{name}"), hits);
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let rel = format!("{rel_prefix}/{name}");
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if !matches!(ext, "rs" | "ts" | "tsx" | "js" | "mjs") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Inline `#[cfg(test)]` modules are test fixtures: stop scanning a
        // Rust file at that marker (by convention it sits at the end).
        let scanned: &str = if ext == "rs" {
            contents
                .find("\n#[cfg(test)]")
                .map(|at| &contents[..at])
                .unwrap_or(&contents)
        } else {
            &contents
        };
        for (idx, line) in scanned.lines().enumerate() {
            if !line_has_hardcoded_model_id(line) {
                continue;
            }
            if line_is_model_id_allowlisted(&rel, line) {
                continue;
            }
            hits.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
        }
    }
}

#[test]
fn guard_no_hardcoded_model_ids_outside_allowlist() {
    let root = repo_root();
    let mut hits = Vec::new();
    collect_hardcoded_model_id_hits(&root.join("core/src"), "core/src", &mut hits);
    collect_hardcoded_model_id_hits(&root.join("webapp/src"), "webapp/src", &mut hits);
    assert!(
        hits.is_empty(),
        "hard-coded model ids outside the allowlist (settings.rs defaults, \
         model_catalog.rs, tests, migrations). SettingsView.tsx LLM_MODEL_PRESETS \
         is not allowlisted — remove it or wait for the model-catalog job.\n{}",
        hits.join("\n")
    );
}
