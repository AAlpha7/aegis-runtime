# Contributing to Aegis Runtime

> This repository is published for reference and is not actively maintained.
> Issues and pull requests may not get a response. Forks are welcome.

## How to Contribute

### Reporting Bugs

Open a [GitHub Issue](https://github.com/AAlpha7/aegis-runtime/issues) with:
- Steps to reproduce
- Expected vs actual behavior
- Rust version (`rustc --version`) and OS

### Submitting Changes

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes
4. Ensure all checks pass. Prerequisites: Rust 1.98+ (`rust-toolchain.toml`
   pins 1.98.1 and requests the `wasm32-wasip1` target; with another toolchain
   run `rustup target add wasm32-wasip1` yourself) and Node 22 for the web
   console.
   ```bash
   make check                             # harness lint/tests + clippy + make test + make webapp-test
   ```
   or, while iterating:
   ```bash
   make rust-fast-check                   # fmt + clippy + core lib tests (fastest loop)
   make test-unit                         # core lib tests only
   make test                              # native Rust tests (core, executor, coord-mcp)
   make webapp-test                       # web console: unit tests + typecheck + build
   cargo insta test --check               # snapshot regression
   ./scripts/mutants-check.sh             # mutation testing on your diff
   ```
   On a clean clone the first `make test` also compiles the Wasm skills into
   `compiled_skills/` (gitignored; needs the `wasm32-wasip1` target). Later
   runs skip that step while the `.wasm` files exist (`SKIP_SKILL_BUILD`
   auto-detects; `make test SKIP_SKILL_BUILD=0` forces a rebuild).
   Bare `cargo build` / `cargo test` / `cargo clippy` act on the host crates
   (`aegis-runtime-core`, `aegis-executor`, `coord-mcp`) through the workspace
   `default-members`. **Do not run `cargo test --workspace`.** The 19
   `wasm32-wasip1` crates under `skills/` cannot build for the host target, so
   a workspace-wide run fails for reasons unrelated to your change; use
   `make skills-check` to type-check them for their real target.
5. Submit a pull request to `main`

### Creating New Skills

See [docs/SKILL_DEVELOPMENT.md](docs/SKILL_DEVELOPMENT.md) for the complete guide on building Wasm skill plugins.

## Testing Infrastructure

Aegis Runtime uses a multi-layer testing strategy:

- **Unit tests** — standard `#[test]` in each module
- **Property-based tests** ([proptest](https://proptest-rs.github.io/proptest/)) — security-critical modules (sanitize, SSRF, sandbox limits) are fuzzed with random inputs to verify invariants (never-panic, idempotence, boundary preservation)
- **Snapshot tests** ([insta](https://insta.rs/)) — output-sensitive modules (markdown rendering, system prompts, tool schemas, error messages) have `.snap` files that catch accidental regressions. Run `cargo insta review` after intentional output changes
- **Mutation testing** ([cargo-mutants](https://mutants.rs/)) — detects "changed logic but didn't add test" gaps. Run `./scripts/mutants-check.sh` to test mutations in your diff vs main

## Code Standards

- **Zero Clippy warnings** — all code must pass `cargo clippy -- -D warnings`
- **Test coverage** — new features should include tests; security modules require proptest properties
- **No unsafe in skills** — only the `alloc` export and host function FFI calls may use `unsafe`
- **No forbidden dependencies** — Docker, LangChain, and a Python or Node.js
  runtime requirement are prohibited (Node is a build-time dependency of the
  web console only).
- **Process spawning is restricted, not banned** — Wasm skills can never spawn
  a process. Host-side `std::process::Command` exists in a handful of host call sites (listed in README § Security: local
  STT, headless browser fetch, MCP stdio, external-executor bridge, the Codex
  CLI management endpoints, an avatar debug endpoint) plus the Aegis Executor
  service. Adding a new call site needs a reason in the PR description.

## Security

For security vulnerabilities, see [SECURITY.md](SECURITY.md). Do not open public issues for security bugs.
