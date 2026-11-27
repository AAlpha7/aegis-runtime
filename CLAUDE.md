# CLAUDE.md — Aegis Runtime

**Read [`AGENTS.md`](./AGENTS.md) first.** It is the canonical agent guide for this repo (security boundaries, verification commands). This file only exists so Claude Code auto-loads an entry point.

- Role (Lance, 2026-10-09): aegis-runtime is the **single code upstream** for Aegis. AAlpha7/aegisflow becomes the private deployment/config layer that depends on it. Port fixes here first; do not fork logic back into aegisflow.
- Privacy: this is an open-source extraction, but publication is NOT authorized yet. Never add secrets, `.env`, databases, login URLs, pairing codes, raw logs, personal data or private-repo details. Run `python3 scripts/privacy-audit.py` for any change that touches docs or data.
- Verification: follow AGENTS.md. Never call a test passed because it built; state what you could not run (e.g. real Tailscale/Passkey/mobile access).
- Coordination ledger: AAlpha7/codex-continuity, project `aegis-runtime` (`projects/aegis-runtime/CURRENT_STATE.md`).
- Open PRs only; Lance merges. No repo writes Mon–Fri 09:00–18:00 Pacific.
- Gates needing Lance's yes: making the repo public or publishing a release, email, X posts, purchases, destructive deletes.
- After work: summarize in the PR with the commands you ran and their results.
