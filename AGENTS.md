# Aegis Runtime agent handoff

This repository is the standalone open-source extraction. Do not assume access
to the original private application, another machine, accounts, or production data.
Read `README.md`, `SECURITY.md`, `docs/REMOTE_ACCESS.md`, and the latest verification
report under `docs/` before continuing. Check Git status and remote changes first;
preserve unrelated work and use a separate branch for overlapping tasks.

For an ordinary deployment request, follow `docs/INSTALLATION.md`: preflight,
managed setup, one real owner-chat probe, and the Windows session task/host-side
idle check on WSL. Do not run the full contributor suite merely to install, or
invent a temporary wrapper and call it durable. The checks below remain required
for source changes and release verification. Report skipped external acceptance.
On a Linux container without a systemd user manager, follow the explicit
foreground/container section of that guide. Account-free build and harness
verification are separate from real runtime login/chat and durable deployment.
Never silently switch a requested managed installation to portable mode.

## Security boundaries

- Memory and heirloom SQLite use SQLCipher. Inject the database key externally;
  never add `.env`, database files, login URLs, pairing codes, secrets or raw logs.
- Remote access uses a loopback backend plus Tailscale Serve (private) or Funnel
  (public). Public mode requires its own explicit risk acknowledgment.
- `core/src/remote_access.rs` verifies Passkeys with `webauthn-rs`. Challenge state
  remains server-side. Host-only pairing/recovery is in `aegis_runtime_access`.
- Never infer host ownership from a proxy's loopback connection or forwarded
  headers. Remote configurations disable the local-login and host-confirm routes.
- A successful mock is not evidence of real mobile or external-network access.

## Reproducible verification without accounts

Linux/WSL2 is the supported script route. Use the pinned Rust toolchain and Node 22+.
Respect `CARGO_TARGET_DIR`; choose build parallelism appropriate to available memory.

```bash
make check
python3 scripts/test_remote_access.py
cargo build -p aegis-runtime-core --bin aegis_runtime_access --example remote_access_harness
cd webapp
npm ci
npx playwright install chromium
node scripts/test-remote-access.mjs
cd ..
python3 scripts/test_access_lifecycle.py
```

The browser test launches a loopback harness with temporary encrypted databases
and a virtual authenticator. It does not need subscription login or API keys.
Install Playwright OS dependencies if a fresh Linux host reports missing libraries.
Run `python3 scripts/privacy-audit.py` before publishing; it is a supplement to
manual review, not a guarantee. Update the verification report with actual results
and explicit unverified conditions. Never mark a test passed merely because it built.
