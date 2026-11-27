# Changelog

All notable changes to Aegis Runtime are documented here, newest first.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Owner call settings now require a direct successful probe before saving:
  chat provider/model, Codex reasoning, search, backup and paid API fallback,
  plus Jev enable/key changes. Failure preserves the old configuration and
  displays a safe reason; disabling/removal works offline. Provider/model
  edits are staged together, and generic writes cannot bypass verification.
- Settings now includes an optional Jev card with explicit consent, encrypted
  key storage, save/cancel, disable/removal and live application to subsequent
  safety checks. Status distinguishes configuration from verified API access.
- Interactive API-key setup offers Jev with a default-No choice, separate billing
  and data-sharing disclosure. General `--yes` and unattended setup do not opt in.
- Jev is off by default across runtime entry points; API keys alone cannot enable
  it. Explicit `AEGIS_JEV_ENABLED=1` plus a valid key opts into the paid API.
- Search and other material route/capability changes now produce runtime notices
  that survive model review, failed turns and history reload; DuckDuckGo fallback
  attempts, success and failure are distinct. X search retains its true source.
- Jev accepts a dedicated `AEGIS_JEV_API_KEY` and rejects generic OpenAI keys for
  OpenRouter. Without Jev, public search discloses its local checks; no-key
  computer actions default to Ask, with existing hard-deny/probe rules retained.
- Subscription binding enables the matching external executor while preserving
  explicit opt-out. Automatic search follows Grok/Codex, falling back to signed
  DuckDuckGo Wasm execution; paid search requires explicit selection.
- Codex external tasks verify ChatGPT login; connector tasks verify actual app
  access. The v1 connector scope is reads and text proposals.
- Realtime voice is deferred: default builds refuse session minting and hide the
  realtime entry. Configured STT voice notes remain available.
- Heirloom is disabled by default. Only `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on`
  enables experimental initialization, encrypted event storage and dual-write.
  Disabled startup leaves existing Heirloom files untouched, ignores its path
  and profile-read preference, and keeps standard encrypted memory active.
  Explicit opt-in still verifies existing encrypted state before startup.

### Added
- Initial public release as **Aegis Runtime** (renamed from the private
  AegisFlow codebase): zero-trust agent runtime in a single Rust binary —
  Ed25519-signed intents, Wasm-sandboxed skills, gated tool execution,
  three-layer memory, Chronos heartbeat, owner work queue, web console.
- Example Wasm skills: `web_search`, `web_reader`, `test_no_net` (sandbox
  demo), `aegis_runtime_wasm_core`.
- `prompt_opt` / `skill_opt` (experimental): explicit-trigger prompt/skill
  optimizer with hard budgets and a strict-improvement gate. Proposes patches
  only — nothing goes live without owner approval.
- `scripts/first-run.sh`: auth-first fresh-machine setup ending in one real
  owner-chat round-trip.
- `SECURITY.md`: documents the three-layer unknown-tool policy
  (permission gate → dispatch table → cross-turn exfiltration guard).
