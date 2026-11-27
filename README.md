# Aegis Runtime

Aegis Runtime powers **Anqi (安琪)**, a personal AI secretary hosted on your
own computer. It keeps task and memory context, follows scheduled work, asks
for authorization when the action requires it, and presents results with
supporting evidence. Owner-chat flows disclose changes to the service, cost or
available capability; [subscription behavior](docs/SUBSCRIPTIONS.md) records
the implemented paths and their limits.

The first release focuses on one owner, Grok/Codex subscriptions, and a web
console that can also be reached from a phone.

**This repository is published for reference. It is not maintained as a
product, and no fixes or roadmap are promised.**
[The acceptance checklist](docs/V1_ACCEPTANCE_CHECKLIST.md) separates
implemented behavior, real verification and unfinished work.
The [clean verification follow-up](docs/verification-2026-09-30-clean-install.md)
tracks the newer settings fixes, replacement package and installation acceptance.

The Rust runtime includes permission gates, an action ledger, signed execution
intents and Wasm-isolated skills. **Wasm isolation applies to Wasm skills, not
every tool or external process.** Built-in tools and subscription executors
have separate host boundaries. Signatures establish integrity and origin, not
that a proposed action is safe or correctly understood. See
[SECURITY.md](SECURITY.md) for the controls and their limits.

## History

Aegis Runtime was developed privately from early 2026 through September 2026
as a single operator's production deployment, then extracted into this
repository as its intended open-source core. The extraction is a curated
snapshot, not the full private history. The original deployment was switched
off in October 2026.

## Architecture

The first release uses the selected Grok/Codex subscription for external
execution and search, with **no automatic search-service fallback**. Paid API
fallback requires a separate opt-in. Connected-account reads require actual
Codex connector access. **Realtime voice is deferred to the next release**;
voice notes remain available with configured STT. See [subscription behavior
and acceptance boundaries](docs/SUBSCRIPTIONS.md).

```
Web Console ── OwnerWorkQueue ── Orchestrator (ReAct + Permission Inversion)
      ▲              │                         │
      │              ▼                         ▼
Telegram        AuthorityEnvelope        Action Ledger
notifications        │                         │
      │              ▼                         ▼
      └────── ContextEnvelope ──────── ReceiptExpectation
                       │
            ┌──────────┼──────────┐
            ▼          ▼          ▼
         Sandbox     Memory     Chronos
         (Wasm)     (L1/L2)   (Heartbeat)
```

The Web Console is the primary owner interface. Telegram is treated as a
notification and deep-link rail until native mobile apps exist.

**Core modules:**

| Module | Purpose |
|---|---|
| **Sandbox (S-LEG)** | Wasm isolation with domain-level network gating (<30ms deny) |
| **Identity (AIP)** | Ed25519 signatures for execution intents; integrity is not a verdict on action safety |
| **Memory** | L1 hot cache + L2 semantic facts + L3 full-text archive via Actor pattern |
| **Snapshot** | HTML semantic extraction (2.5 MB HTML -> 4 KB Markdown) |
| **Orchestrator** | ReAct loop with permission inversion — gate + dispatch + exfil guards stop hallucinated tools |
| **Chronos** | Cron heartbeat engine — silent probes cost 0 tokens |
| **Owner Work Queue** | Single owner-facing queue for items, authority, blockers, and receipts |
| **Web Console** | Primary trust workbench for review, drafting, approval, memory, and automation settings (optional component) |
| **Telegram Notifications** | Redacted mobile alerts, safe quick actions, and Web Console deep links |
| **Logging** | Runtime-reloadable log filter for production diagnostics |

**Shipped example skills** (Wasm, sandboxed): `web_reader`,
`test_no_net` (sandbox demo), `aegis_runtime_wasm_core`. Bring your own —
see [docs/SKILL_DEVELOPMENT.md](docs/SKILL_DEVELOPMENT.md).

**Experimental:** `prompt_opt` / `skill_opt` — an explicit-trigger prompt/skill
optimizer with hard budgets and a strict-improvement gate. It only *proposes*
patches; nothing goes live without owner approval. See
`core/src/prompt_opt/` and treat it as experimental.

**Heirloom is experimental and disabled by default.** Standard memory remains
active. Only `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on` enables the additional event log;
its development custody and policy components are not production guarantees.
See [Heirloom status and opt-in](docs/HEIRLOOM.md).

## Quick Start

**Start with [the durable installation guide](docs/INSTALLATION.md).** Normal
setup installs a systemd user service and generates a private external key file.
WSL additionally needs the included Windows session task. Google login is an
optional advanced integration, disabled by default. Local login needs no Google
project; remote access uses Passkeys.

**Cloud container without a systemd user manager?** Use the explicit
[foreground/container route](docs/INSTALLATION.md#linux-containers-and-foreground-operation).
It separates account-free source builds from account-authorized runtime setup;
it does not install a service, create a database key, or promise background uptime.
For a build-only checkout, run `python3 scripts/foreground-runtime.py build`
directly; it checks build tools without requiring free runtime ports. The guide
places port preflight in the runtime preparation phase.
The login-first sequence below applies to normal account-backed installation.
See the [container candidate verification](docs/verification-2026-10-01-container.md)
for observed results and remaining acceptance.

The order matters: **pick the chat provider and finish its sign-in first**,
then build, then run. A boot without that sign-in either refuses to start
(no reasoner) or serves a console whose chat answers 503 until you
complete two more OAuth sessions — do not set up in that order.

```
1. system deps  →  2. choose auth (grok login / Codex login / API key) + sign in
→  3. build  →  4. bind owner chat before boot  →  5. run + open the login link
```

`scripts/first-run.sh` does steps 2–5 in that order and ends with one real
owner-chat round-trip:

```bash
git clone https://github.com/AAlpha7/aegis-runtime.git && cd aegis-runtime
# Check prerequisites before account login. New installs create a private key.
python3 scripts/managed-runtime.py preflight
scripts/first-run.sh --provider grok      # or: codex | api-key (interactive menu when omitted)
```

For a manual foreground run, omit the injected key; the binary prompts for a
passphrase in the terminal. `first-run.sh --no-start` can do setup this way,
then `scripts/aegis-runtime-local.sh run` prompts again. Portable background
starts require an externally injected key; managed services load their private
key file automatically. Keep the key/passphrase recoverable:
losing it makes both databases unreadable.

The manual equivalent follows.

### Optional phone access

Keep Aegis on your own computer and choose **local**, **private Tailscale VPN
(recommended remote option)**, or **public browser access** during installation.
Use `--access-mode private` or `--access-mode public --accept-public-risk` for an
explicit choice. Public mode has greater exposure and requires its own acknowledgment;
`--yes` does not imply consent. Both remote modes use host-paired Passkeys and HTTPS.
See [phone access, pairing and recovery](docs/REMOTE_ACCESS.md) for prerequisites,
mode changes, and credential-free behavioral tests. Actual mobile/Tailscale network
acceptance remains distinct from local automated verification.

### 1. System dependencies

| Dependency | Why |
|---|---|
| Rust **1.98+** via [rustup](https://rustup.rs) | `rust-toolchain.toml` pins 1.98.1; rustup installs it automatically on first `cargo` |
| `rustup target add wasm32-wasip1` | the skill plugins are `wasm32-wasip1` crates; the `.wasm` files are built on first `make test` when missing. `rust-toolchain.toml` requests the target, so rustup adds it with the pinned toolchain — run the command yourself on any other toolchain |
| Node.js **22** + npm | builds the web console (`webapp/`) — **not needed at runtime** |
| C toolchain, `pkg-config`, OpenSSL headers | native build deps (`scripts/bootstrap-system-deps.sh` installs them on Ubuntu/Debian) |
| `ffmpeg` (optional) | voice notes are transcoded to 16 kHz mono WAV before transcription; text works without it |

```bash
# Ubuntu/Debian
sudo apt-get install -y build-essential pkg-config libssl-dev git ffmpeg
# Install Node.js 22+ with its npm from nodejs.org or a version manager;
# Ubuntu's nodejs/npm packages may not provide a recent enough version.
node --version
# macOS
brew install git node ffmpeg

# Rust (both)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-wasip1
```

Full fresh-machine notes (mold/sccache accelerators, macOS Xcode CLT):
[docs/deployment-prereqs.md](docs/deployment-prereqs.md).

### 2. Choose the auth method and sign in (before building)

Ask in this order — login first, paid keys last. Each row says what the
sign-in satisfies; **owner chat is only satisfied by the last column**.

| Choose | Sign-in (do it now) | Startup / CLI gate | Owner chat (`POST /webapp/vnext/command`) |
|---|---|---|---|
| **Grok** (preferred) | `curl -fsSL https://x.ai/cli/install.sh \| bash` then `grok login --device-auth` (writes `~/.grok/auth.json`) | **Yes** — reasoner gate, `grok -p`, `run_external` | **Not yet.** Grok is **two sessions**: the CLI login above, plus an in-app xAI device code that stores the encrypted chat credential and `llm_provider=xai_subscription`. Do the second one in step 4 (`setup-auth --provider grok`), before boot — or later in Settings → Grok → Connect → **Use for chat** |
| **Codex** | `codex login --device-auth` (ChatGPT subscription; `codex login status` must say `Logged in using ChatGPT`) | **Yes** if `codex` is on `PATH` | **Yes** after the bind in step 4 (`setup-auth --provider codex`, or Settings → Codex → Connect → Use for chat) |
| **API key** (last resort) | put a usable `OPENAI_API_KEY` (not empty / not `sk-or-v1-xxxxxxxx` / `sk-placeholder`) and `OPENAI_API_BASE` in `.env` | **Yes** | **Yes** — this is the default when `llm_provider` is unset (legacy `openai_compat`) |

If none of those is possible, **stop and ask the human**. On an unattended
Cloud VM `~/.grok/auth.json` does **not** survive a new VM: prefer a
persisted Grok/Codex bind (it lives in the data files next to `.env`) or bake
`OPENAI_API_KEY` (and `OPENAI_API_BASE`) into the machine's secret store.

### 3. Build

```bash
# Web console first. The binary serves it from webapp/dist, which is
# generated, not committed — skip this and /ui/ has nothing to serve.
cd webapp && npm ci && npm run build && cd ..

# The service binary (workspace root; default members are core + executor).
cargo build --release
cp -n .env.example .env
```

### 4. Bind owner chat — before the first boot

```bash
./target/release/aegis_runtime_core setup-auth --provider grok      # prints URL + code; polls (≥10s after xAI slow_down)
./target/release/aegis_runtime_core setup-auth --provider codex     # runs `codex login --device-auth` if needed, then binds
./target/release/aegis_runtime_core setup-auth --provider api-key   # validates OPENAI_API_KEY, keeps openai_compat
./target/release/aegis_runtime_core setup-auth status               # exit 0 = chat will work after boot, 2 = not yet
```

`setup-auth` writes exactly what Settings → **Use for chat** writes (the
encrypted credential in `CREDENTIAL_DB_PATH`, `llm_provider` and the
`*_enabled` flags in `MEMORY_DB_PATH`), so run it from the same directory and
with the same database key and env the service will use. Re-running reuses an existing
connection (`--reconnect` to sign in again).

### Optional Hercules delegation

Off by default, and you need to set it up yourself. With it on, the assistant
gets a `delegate_task` tool that hands coding tasks to your own
[Hercules](https://github.com/AAlpha7/hercules-public) server. Aegis Runtime
drafts and creates the task; Hercules runs the agents and verification.

1. Run Hercules with a token and at least one repo in its `repos.yaml`.
2. Set these in `.env`, then restart:

   ```bash
   AEGIS_HERCULES_DELEGATION_ENABLED=1
   HERCULES_API_BASE=http://127.0.0.1:8787
   HERCULES_TOKEN=<your Hercules token>
   ```

Credentials alone do not enable the tool; the flag is required. Keep the
Hercules port on a private network, as its README advises.

The draft, lint, create and fetch calls were tested against hercules-public
`5917173` using its `fake` adapter. A real agent run was not part of that test.
Aegis Runtime currently uses the immediate keyword-based draft and does not read
the later result from Hercules' analyst, so review the task before approving it.

### Database encryption

The memory database (`MEMORY_DB_PATH`) and, when explicitly enabled, the heirloom
event log (`HEIRLOOM_EVENT_LOG_DB`, or next to the memory DB) are encrypted with
SQLCipher. Before the first setup or boot, inject a **random 32-byte key** as
`AEGIS_RUNTIME_DB_KEY_HEX` from a secret manager or service credential source.
Generate one with `openssl rand -hex 32`, store it securely outside the state
directory, and arrange for the same value to be injected on every run. Never
put it in `.env`, a command-line argument, a backup alongside the DB, or Git.
The service captures this variable before loading `.env`; `.env` cannot supply
the database key. A foreground invocation without an injected key instead
prompts for a nonempty passphrase; the same passphrase is needed at every
startup. When creating a new database, the terminal asks for that passphrase
twice to catch a typo; later server starts ask once. Web and phone clients never
receive a database-key prompt. The two input modes use different SQLCipher key derivation, so choose
one mode for a given database and keep it consistent.

An existing plaintext installation needs an explicit, offline copy migration.
Stop the service, checkpoint and close every SQLite writer, and back up the
state directory. For **each** plaintext memory and heirloom database, run:

```bash
# Use the same injected key (or terminal passphrase) as the new service.
cargo run --release -p aegis-runtime-core --bin aegis_runtime_encrypt_db -- \
  /path/to/aegis_runtime_memory.db /path/to/aegis_runtime_memory.encrypted.db
cargo run --release -p aegis-runtime-core --bin aegis_runtime_encrypt_db -- \
  /path/to/heirloom_event_log.db /path/to/heirloom_event_log.encrypted.db
```

The tool reads each plaintext source without modifying it, verifies the new
database, and refuses to overwrite an output. Confirm both encrypted copies
and the backup, then rename the originals out of the configured paths and
replace them with the encrypted copies while the service is stopped. Old
plaintext files, WAL/SHM files, and backups still expose data: retire them
under your storage policy after the new service has opened both databases.
Do not use a copy of a live SQLite file as the source. The credential database
has its existing field encryption and is outside this two-database migration.

For a live, consistent backup of either encrypted database, use
`aegis_runtime_backup_db SOURCE.db BACKUP.db` with the same injected key (or a
foreground passphrase). It uses SQLite's online backup API and verifies that
the output remains encrypted. `scripts/deploy.sh backup`, `upgrade`, and
`update` now use this helper and require the external key; an ordinary
`sqlite3 .backup` cannot open these encrypted files. Keep backup files and the
key in separate custody.

### 5. Configure the rest of `.env`

When you do use an API key, also set `OPENAI_API_BASE`. Everything else is
optional; two settings are worth knowing about:

| Variable | Default / without it |
|---|---|
| `EMBEDDING_API_KEY` / `EMBEDDING_API_BASE` | Grok/Codex: install local Ollama embedder via `scripts/ensure-local-embedding.sh` (nomic-embed-text; first-run does this). API-key path: defaults to `OPENAI_*`. Missing config fails loudly with install steps — not a silent 401. Without a working embedder, semantic recall is off (chat still answers) |
| `AEGIS_RUNTIME_STATE_DIR` | durable DBs/keys + pid/log; default `${XDG_STATE_HOME:-~/.local/state}/aegis-runtime`. first-run wires `MEMORY_DB_PATH` / `IDENTITY_KEY_PATH` / `CREDENTIAL_DB_PATH` here |
| `USER_TIMEZONE` | every owner-facing time renders in the host's zone (UTC when the host has no zone configured) |

**Telegram is optional.** `.env.example` ships `TELOXIDE_TOKEN` /
`ADMIN_CHAT_ID` commented out at the top; uncomment and fill both to enable
the bot. They only add the mobile notification rail, and a configured bot
token also switches off the local login link described below.

Even without Telegram, set **`ADMIN_CHAT_ID=1`** (any stable non-zero integer).
The Web Console workbench APIs fail closed until a chat context exists: `/ui/`
and local login still work, but Today / Inbox / settings / `POST /webapp/vnext/command`
return 403 `Workbench chat context is not configured`.

### 6. Run and open the console

```bash
./target/release/aegis_runtime_core          # foreground, from the checkout root
scripts/aegis-runtime-local.sh start         # manages the installed user service
```

`scripts/aegis-runtime-local.sh` manages the installed user service. An explicit
`first-run.sh --portable` retains the temporary nohup/PID-file path for hosts
without systemd; it is not durable deployment acceptance. **State directory:**
`scripts/first-run.sh` puts durable data under
`${AEGIS_RUNTIME_STATE_DIR:-${XDG_STATE_HOME:-~/.local/state}/aegis-runtime}` —
`aegis_runtime_memory.db`, `aegis_runtime_credentials.db`, `aegis_runtime_identity.key`
(opt-in heirloom log next to the memory DB). Pid + log already live there. Use the
encrypted online backup helper above for live databases; for a stopped
installation, copy the state directory and `.env` while keeping the database
key in separate custody. To keep files in the
checkout instead, set the three `*_PATH` vars to `./…` explicitly.

On startup the process prints, to stderr / the journal:

- `[Reasoner] …` — which of grok build / Codex / API key satisfied the gate;
- the console URL — `http://127.0.0.1:8765/ui/` by default;
- a **local login link**, valid until the process restarts — `http://127.0.0.1:8765/auth/login/local?token=<token>`.

Open the login link in a browser. It creates the owner session and lands on
`/ui/`. That is the first-run UI path; no Google project and no Telegram
bot are needed.

**`GET /health` is not enough.** Setup is not done until one owner chat
round-trip succeeds. On the console port (`OAUTH_CALLBACK_PORT`, default
`8765` — **not** `AXUM_PORT`):

```bash
# 1) mint a session (follow 303; cookie name is __Host-aegis_session)
curl -sS -D- "http://127.0.0.1:8765/auth/login/local?token=$AEGIS_LOCAL_LOGIN_TOKEN"
# 2) CSRF
curl -sS -H "Cookie: __Host-aegis_session=<token>" http://127.0.0.1:8765/api/session
# 3) send / receive — expect HTTP 200 and assistant_response.visible_answer
curl -sS -X POST http://127.0.0.1:8765/webapp/vnext/command \
  -H "Content-Type: application/json" \
  -H "Cookie: __Host-aegis_session=<token>" \
  -H "x-aegis-csrf: <csrf_token>" \
  --data '{"schema_version":1,"input":"Reply with exactly: aegis-chat-ok","mode":"text","intent_kind":"chat","client_context":"home"}'
```

A 503 `Assistant model is unavailable` is usually **configuration**, not a
missing feature: unset `llm_provider` keeps the legacy `openai_compat` path.
It means step 4 was skipped — run `aegis_runtime_core setup-auth status`,
then `setup-auth --provider …` (or bind in Settings: Grok or Codex →
**Use for chat**), or set a usable `OPENAI_API_KEY`. Do not treat a 503 as a
passing setup.

What the defaults mean:

- **Console port.** The console (and the OAuth callback / WebApp API) listens on
  `127.0.0.1:` `OAUTH_CALLBACK_PORT` (default `8765`) — `AEGIS_PUBLIC_URL` is
  optional and only matters when the console must be reachable from outside
  (Telegram Mini App, Google OAuth redirect URIs). Port **3000** (`AXUM_PORT`)
  is the bearer-token **admin API**, not the console.
- **Login.** When neither Google login (`GOOGLE_LOGIN_CLIENT_ID`) nor a
  Telegram bot (`TELOXIDE_TOKEN`) is configured, a local login token
  is generated at every start (valid until restart; it is not consumed on use). `AEGIS_LOCAL_LOGIN_TOKEN=<secret>` sets a fixed
  token explicitly (and enables local login even when Google/Telegram are
  configured); `AEGIS_LOCAL_LOGIN_TOKEN=off` disables it. Google login is
  optional (`GOOGLE_LOGIN_*` in `.env.example`).
- **Console bundle.** The binary resolves `webapp/dist` relative to the
  current directory, then next to the executable, then the source tree
  (`CARGO_MANIFEST_DIR`); `AEGIS_WEBAPP_DIR` overrides. A missing bundle logs a
  `WARN` naming the path it tried — run from the checkout root or set the
  variable.

`.env.example` documents every variable the code reads.

## Test

```bash
make test       # host-native packages: aegis-runtime-core + aegis-executor
make check      # lint + host-native Rust tests + web console tests (needs node)
```

Clean-clone note: `compiled_skills/*.wasm` are gitignored, so the first
`make test` builds every skill for `wasm32-wasip1` before the integration
suites run (several minutes; the target must be installed). Bare
`cargo build --release`, `cargo test` and `cargo clippy` at the workspace root
work too (default members exclude the wasm-only skill crates), but do **not**
use `cargo test --workspace` — skill crates cannot build for the host target.
Narrower loops: [docs/TESTING_GUIDE.md](docs/TESTING_GUIDE.md).

## Documentation

- **[Docs Index](docs/README.md)** — Entry point for the active design set and runbooks
- **[Deployment Guide](docs/DEPLOYMENT.md)** — Step-by-step setup, configuration, production deployment, and troubleshooting (Chinese)
- **[Design Specification](docs/DESIGN.md)** — Full architecture, data structures, security model, and test matrix (Chinese)
- **[Testing Guide](docs/TESTING_GUIDE.md)** — Test layers, commands, and the LLM-boundary checklist
- **[Skill Development](docs/SKILL_DEVELOPMENT.md)** — How to create new Wasm skill plugins (Chinese)
- **[Changelog](CHANGELOG.md)** — What changed, newest first
- **[Code of Conduct](CODE_OF_CONDUCT.md)** — Contributor Covenant 2.1 and where to report conduct issues

## Security

See [SECURITY.md](SECURITY.md) for the vulnerability reporting policy and the
exact unknown-tool handling policy.

Aegis Runtime's security model:
- **Zero-trust by default** — every action requires a signed IntentEnvelope
- **Wasm hard isolation** — skills run in sandboxed instances, destroyed after each call
- **Permission inversion** — LLM output is intercepted; calls must pass the permission gate, resolve in the dispatch table, and clear the cross-turn exfiltration guard
- **Anti-replay** — 5-minute timestamp window on all signed requests
- **Process execution is confined, not absent** — skills (Wasm) can never spawn
  a process. The host does, in a small set of audited call sites (local
  speech-to-text and audio transcoding, headless-browser page fetch, MCP stdio
  clients, the external-executor bridge, the Codex app-server chat path, the
  `journalctl` log viewer, the Codex CLI management endpoints, the owner-only
  WebApp debug job runner, and the operator-run `setup-auth` CLI mode). None of
  them execute LLM-generated shell. Operator-initiated shell work goes through
  the separate **Aegis Executor** service, which runs under its own user and
  accepts only (a) a whitelist of read-only diagnostic commands, bearer-token
  gated, with shell metacharacters and protected paths rejected, or (b)
  mutating commands carrying a valid Ed25519-signed envelope inside a
  5-minute replay window

## License

MIT — see [LICENSE](LICENSE).
