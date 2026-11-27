# Aegis Runtime Testing Guide

Status: active testing guide (updated 2026-09-15).

## Why This Guide Exists

We've had repeated production bugs that passed all unit tests. The pattern:

| Bug | Root Cause | Why Tests Missed It |
|-----|-----------|-------------------|
| `notify` mode leaked instructions | LLM picked wrong execution_mode | Tests hardcoded the mode |
| Dashboard empty | KB had no data, didn't include intents | Tests pre-filled data |
| Every message 400 for hours after a deploy | smoke used empty evidence and skipped the real turn | Happy path only; no failure-path smoke |

**Common thread**: tests validated internal logic with ideal inputs, but never tested the **LLM output boundary** — where unpredictable JSON enters the system — and deploy smoke never exercised the real owner path.

---

## Running Tests

`make jev-setup-test` runs the real API-key first-run wizard with temporary state,
synthetic credentials, a stub auth binary and PTY input. It verifies explicit
Jev consent, default refusal, compatible-key requirements, saved choices and
unattended behavior without accounts or paid API calls. It is part of `make check`.

Verified settings saves are covered by `cargo test -p aegis-runtime-core --lib
call_validation` and the `jev_` / `settings` filters. These include real local
HTTP probes, semantic-response rejection, failure preserving active/persisted
settings, generic-admin/chat-approval bypass rejection, and offline disable.
`node webapp/scripts/test-remote-access.mjs` exercises staged provider/model
selection and Jev save/error/reload through the browser and encrypted router.
Its example harness injects synthetic upstream outcomes explicitly; that does
not establish real subscription, API or Jev account access.

All commands run at the workspace root. `make` targets are the source of truth
(see `Makefile` and `AGENTS.md`); the raw `cargo` forms are listed for scoping.

```bash
# Everything CI runs locally: agent-harness lint + tests, clippy, host-native
# Rust tests, web console tests (needs python3 and node)
make check

# Focused first-release subscription/connector/search behavior (no accounts)
cargo test -p aegis-runtime-core --lib behavior_notice
cargo test -p aegis-runtime-core --lib egress_gate
cargo test -p aegis-runtime-core --lib computer_use
cargo test -p aegis-runtime-core --lib record_unanswered_owner_turn
cargo test -p aegis-runtime-core --lib subscription_
cargo test -p aegis-runtime-core --lib external_executor::tests
cargo test -p aegis-runtime-core --lib webapp_vnext_voice_
cargo test -p aegis-runtime-core --test react_loop_integration react_web_search

# Real runtime startup/storage regression for default-off Heirloom (no accounts)
make heirloom-mode-test

# Passkey browser journeys + deployment/lifecycle tests (virtual authenticator)
make remote-access-test

# Host-native Rust tests: aegis-runtime-core, aegis-executor, coord-mcp
# (lib + every core/tests/*.rs integration binary)
make test

# Fast loop: fmt + clippy + core lib tests only
make rust-fast-check

# Pre-commit regression sensors
make rust-critical-paths     # = cargo test -p aegis-runtime-core --test smoke_critical_paths --test structural_guards --test user_journeys --test user_journey_v2 --test route_coverage_guard --test hitl_routing_linkage

# Unit tests only (no .wasm files needed)
cargo test -p aegis-runtime-core --lib
cargo test -p aegis-runtime-core --lib -- intent::tests        # one module

# One integration binary (names = files under core/tests/)
cargo test -p aegis-runtime-core --test telegram_callback_mock
cargo test -p aegis-runtime-core --test pseudo_tools_mock

# Wasm skill crates: type-check every skills/* crate for wasm32-wasip1 (CI runs this too)
make skills-check

# Web console
make webapp-test             # npm test + tsc --noEmit + vite build

# Snapshot update after an intentional prompt/schema change
INSTA_UPDATE=1 cargo test -p aegis-runtime-core --lib -- snapshot
# or, with cargo-insta installed: cargo insta review

# E2E tests that call real APIs (#[ignore] by default; credentials are opt-in).
# The harness (core/tests/common/mod.rs) reads keys ONLY from <repo root>/.env.test
# and ONLY when AEGIS_LIVE_TESTS=1; it never reads /opt/aegis-runtime/shared/.env.
# Variables already exported in the shell always win. `.env.test` is gitignored.
AEGIS_LIVE_TESTS=1 cargo test -p aegis-runtime-core --test real_api_e2e -- --ignored
AEGIS_LIVE_TESTS=1 cargo test -p aegis-runtime-core --test live_api_test -- --ignored

# Mutation testing on your diff vs main (cargo-mutants; config in .cargo/mutants.toml)
./scripts/mutants-check.sh

# Performance baseline / comparison (test-reports/baseline.json)
./scripts/test-report.sh --baseline
./scripts/test-report.sh --compare
```

Notes:

- **Do not** run `cargo test --workspace` / `--all`. The workspace contains
  `wasm32-wasip1`-only skill crates (`skills/*`, `aegis_runtime_wasm_core` has a
  `compile_error!` guard for non-wasm targets). Bare `cargo test`, `cargo build`
  and `cargo clippy` at the workspace root are fine — the default members are
  `core`, `executor` and `tools/coord-mcp`.
- **Clean clone**: `compiled_skills/*.wasm` are gitignored. The first
  `make test` builds every skill for `wasm32-wasip1` when they are missing
  (the target must be installed), so it takes several minutes; later runs skip
  up-to-date skills. `SKIP_SKILL_BUILD=1` skips skill compilation entirely for
  core-only iteration (`wasm_skill_completeness` and `structural_guards` then
  need the artifacts to already exist).
- Some tests bind loopback TCP ports for mock servers; a sandbox that blocks
  loopback binds makes them fail for reasons unrelated to the change.

---

## Test Categories

| Type | Location | Runs In CI | Purpose |
|------|----------|-----------|---------|
| Unit | `core/src/*.rs` (`#[cfg(test)]`, plus `*_tests.rs` siblings) | Yes | Internal logic |
| Integration | `core/tests/*.rs` | Yes (`make test SKIP_SKILL_BUILD=0`) | Cross-module behavior against mock LLM servers |
| E2E | `core/tests/real_api_e2e.rs`, `core/tests/live_api_test.rs` | `#[ignore]` | Real API calls |
| Property (proptest) | `mod proptest_*` in unit tests | Yes | Fuzz JSON parsers and security boundaries |
| Snapshots (insta) | `core/src/snapshots/` | Yes | Prompt / tool-schema / rendering regression |
| Structural guards | `core/tests/structural_guards.rs`, `route_coverage_guard.rs`, `hitl_routing_linkage.rs` | Yes | Count / wiring drift sensors |
| Web console | `webapp/tests/*.test.mjs` via `npm test` | Yes | Renderer guards, contracts, i18n parity, view rendering |
| Deploy scripts | `scripts/test-deploy.sh` | Yes | `deploy.sh` behavior |

### Five-Layer Regression Suite

Added 2026-04-10 based on production L3 data analysis. Run before every commit:

```bash
# Pre-commit (fast)
make rust-critical-paths

# Full regression suite
SKIP_SKILL_BUILD=1 cargo test -p aegis-runtime-core --test smoke_critical_paths --test structural_guards \
  --test pseudo_tool_completeness --test user_journey_v2 --test chaos_resilience --test wasm_skill_completeness
```

| Layer | File | Purpose |
|-------|------|---------|
| Structural Guards | `structural_guards.rs` | Catches tool / skill / snapshot / i18n count drift, ungated-tool set drift, hardcoded model ids |
| Smoke | `smoke_critical_paths.rs` | Critical ReAct loop paths |
| Completeness | `pseudo_tool_completeness.rs` | Every pseudo-tool handler reachable |
| User Journeys | `user_journey_v2.rs` | Production-pattern multi-turn scenarios |
| Chaos | `chaos_resilience.rs` | Failure injection / resilience |
| Wasm Skills | `wasm_skill_completeness.rs` | All compiled skills load without error |

**When adding a new tool or skill**, update the expected constants at the top
of `core/tests/structural_guards.rs` (`EXPECTED_MCP_PSEUDO_TOOL_COUNT`,
`EXPECTED_WASM_SKILL_COUNT`, `EXPECTED_MIN_SNAPSHOT_COUNT`,
`KNOWN_UNGATED_TOOLS`, and the model-id allowlist). The file is the source of
truth for the current values; this guide deliberately does not repeat them.

---

## The LLM Boundary Problem

Any function that parses LLM-generated JSON is a **boundary function**. These MUST have:

1. **Alias coverage** — `#[serde(alias = "...")]` for common LLM variations
2. **Fallback parsing** — lenient parser when strict serde fails
3. **Default filling** — empty required fields filled from context
4. **Proptest fuzzing** — random JSON with correct `type` tag but randomized fields
5. **Regression samples** — real production LLM outputs captured as unit-test fixtures next to the parser

### Boundary function inventory

| Function | File | LLM JSON → Rust Struct |
|----------|------|----------------------|
| `validate_intent_request()` | `intent.rs` | tool call args → `ValidatedIntent` |
| `manage_panels` arg extraction | `orchestrator_panels.rs` | tool call args → panel CRUD |
| Pseudo-tool arg extraction (`args.get("field")`) | `orchestrator_tools_*.rs` | tool call arg objects |
| Profile slot writes (`upsert_profile_entry`) | `profile_registry.rs` | `update_profile` args → validated slot |
| `knowledge_base` extract payloads | `orchestrator_tools_knowledge.rs` | LLM entries → `KnowledgeEntry` |

---

## Adding a New Feature: Testing Checklist

### 1. New Pseudo-Tool

```
□ Handler in the relevant orchestrator_tools_*.rs with JSON arg extraction
□ Schema builder in tool_schemas.rs; ToolGate policy in tool_gate.rs (or add to KNOWN_UNGATED_TOOLS with a reason)
□ Unit test: happy path with correct args
□ Unit test: missing required args → structured error
□ Unit test: extra/unexpected args → ignored gracefully
□ Proptest: random arg combinations never panic
□ Snapshot test if tool schema is in prompt
□ Bump EXPECTED_MCP_PSEUDO_TOOL_COUNT in structural_guards.rs
```

### 2. New Serde Struct (parsed from LLM output)

```
□ #[serde(alias = "...")] on fields LLM might name differently
□ #[serde(default)] on optional fields
□ Fallback parser if LLM uses completely unexpected schema
□ Empty-field filling from context
□ Proptest: random JSON with correct type tag
□ Regression sample: capture the first real LLM output as a test fixture
□ Unit test: minimal JSON (only required fields)
□ Unit test: maximal JSON (all fields populated)
```

### 3. New Callback Route (Telegram InlineKeyboard)

```
□ Entry in the CALLBACK_ROUTES static table (core/src/callback_routes.rs), not an if-let chain
□ Ack text in match statement
□ Status label in edit-message match statement
□ Orchestrator starts_with handler (e.g. "approve sysop ", "reject config ")
□ Integration test in telegram_callback_mock.rs
□ Test: valid ID → correct handler called
□ Test: invalid/missing ID → error response
□ hitl_routing_linkage.rs stays green (approval routes must link to a handler)
```

### 4. New Match Statement on User/LLM Input

```
□ All expected values have explicit arms
□ Catch-all arm logs a warning (never silently succeeds)
□ Test covers every explicit arm
□ Test covers the catch-all arm
```

### 5. Menu/Panel System Changes

```
□ No raw Vec<Value> for custom commands — use CustomCommand struct (core/src/menu.rs)
□ Non-custom panel types are unique per chat_id (enforce in create path)
□ sync_* functions: apply_menu_to_telegram() BEFORE save_custom_commands()
□ Validate command name format (1-32 lowercase/digits/underscores) in all paths
□ Test: duplicate panel_type creation rejected
□ Test: startup cleanup removes stale user commands
□ Test: CustomCommand round-trips through JSON (serialize → deserialize)
```

### 6. New Admin / WebApp Route

```
□ Route registered in core/src/admin/server.rs on the right router (admin API vs public console router)
□ Auth: bearer middleware (admin API) or owner session / CSRF (console writes)
□ route_coverage_guard.rs updated if it enumerates the route family
□ WebApp behavior change: scenario under webapp/scripts/ui-scenarios/ + make webapp-auth-browser-smoke
```

---

## Property-Based Testing (proptest)

We use `proptest` to fuzz JSON parsers. Pattern:

```rust
mod proptest_my_feature {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]

        #[test]
        fn my_parser_never_panics(
            field_a in "\\PC{0,100}",  // arbitrary unicode
            field_b in prop::sample::select(vec!["valid1", "valid2", "bogus"]),
            has_optional in proptest::bool::ANY,
        ) {
            let mut obj = serde_json::Map::new();
            obj.insert("required_field".into(), json!(field_a));
            if has_optional {
                obj.insert("optional".into(), json!(field_b));
            }
            let val = serde_json::Value::Object(obj);
            let _ = my_parse_function(&val); // must not panic
        }
    }
}
```

**Key rules**:
- `cases: 200` — enough to find issues, fast enough for CI
- Test the **parse boundary**, not internal logic
- Assert `Ok` or `Err`, never `panic`
- Use `prop::sample::select` for enum-like fields with both valid and invalid values

---

## Callback Routing Exhaustiveness

Every inline keyboard button in `telegram.rs` MUST have:

| Component | Location |
|-----------|----------|
| Callback prefix / route | `core/src/callback_routes.rs` (`CALLBACK_ROUTES` table) |
| Ack text | `telegram.rs` `ack_text` match |
| Status label | `telegram.rs` status match |
| Orchestrator handler | `core/src/orchestrator/message_flow.rs` `starts_with("approve sysop ")` etc. |
| Integration test | `core/tests/telegram_callback_mock.rs` |
| Linkage sensor | `core/tests/hitl_routing_linkage.rs` |

**To verify exhaustiveness**, grep for all callback prefixes and ensure each has every component:
```bash
grep -n 'strip_prefix(".*:")' core/src/telegram.rs
grep -n 'starts_with("approve \|starts_with("reject ' core/src/orchestrator/message_flow.rs
```

---

## Fail-Safe Default Pattern

When a system has two modes (safe vs. unsafe), **always default to the safe mode**:

| Setting | Safe Default | Why |
|---------|-------------|-----|
| `execution_mode` | `llm_callback` | Can handle notify cases; notify can't handle action cases |
| Unknown `execution_mode` | Error | Don't silently accept invalid values |
| Rate-limit DB failure | deny | A storage outage must not become unlimited access |
| Missing ToolGate policy | listed in `KNOWN_UNGATED_TOOLS` or the guard fails | Read-only tools only, by explicit allowlist |

**Anti-pattern**: defaulting to the cheaper/simpler option and trying to detect when the complex option is needed. This requires maintaining keyword lists or heuristics that inevitably miss cases.

---

## Failure-Path Smoke

Happy-path smoke is not enough: the 2026-09-08 incident (primary model timeout
→ every message 503 for hours) was invisible to it. Deploy smoke must also
cover the failure path — a primary-model failure must fall back to the
configured fallback model and must not replay the failed turn. See
`scripts/smoke-web-turn.py` and `scripts/post-deploy-checklist.sh` for the
scripts the deploy gate runs, and `docs/three-tier-test-strategy.md` for how
commit-time, deploy-time and nightly runs relate.

---

## Performance

### Shared Wasmtime Engine

Integration tests must use `common::shared_sandbox()` (or `shared_sandbox_arc()`)
from `core/tests/common/mod.rs` instead of `SandboxEngine::new()`. The Wasmtime
JIT compiler init costs ~400-800ms — sharing one engine across all tests in a
binary saves tens of seconds of cumulative init time.

```rust
mod common;

// GOOD — reuses shared JIT engine
let sandbox = common::shared_sandbox();

// BAD — creates new JIT engine every time (~600ms)
let sandbox = SandboxEngine::new().unwrap();
```

### Mock Error Codes

When mocking LLM failures, use HTTP 422 (not 500). async-openai retries 500s with
exponential backoff, causing 60+s delays in tests. 422 triggers an immediate error.
