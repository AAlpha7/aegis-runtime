# Install and keep Aegis running

The durable installation supports Linux with a systemd user manager, or WSL2
with that manager **and a Windows session task**. Local-only access is the
default. Google login and Jev are optional and off by default; Heirloom is
experimental and off, and realtime voice is deferred. Remote access uses Passkeys.

Linux containers without a systemd user manager have an explicit
[foreground route](#linux-containers-and-foreground-operation) below. It supports
source builds and foreground processes, **not** managed background installation.

Agents: follow this guide. Do not invent temporary background wrappers, copy
another installation's credentials, or run contributor tests during ordinary setup.

## Check before signing in

Run as the intended ordinary Linux user:

```bash
python3 scripts/managed-runtime.py preflight
```

The check identifies missing tools, Node 22+, OpenSSL headers, systemd user-manager
access, lingering and occupied ports before a fresh setup. If another service
uses the defaults, choose free `AXUM_PORT` and `OAUTH_CALLBACK_PORT` values in
`.env`; the installer does not stop that service or silently change ports.
A denied request in an agent sandbox does not mean systemd
is absent: retry through the normal permission flow or a normal host terminal.
On Ubuntu/Debian an administrator can perform these bounded operations:

```bash
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev curl python3 git
sudo loginctl enable-linger "$(id -un)"
```

Lingering keeps this user's services after terminal logout and grants no sudo
access. An account without administrator access needs its host administrator to
run those commands. Do not grant passwordless sudo or run the whole installer as
root. Install Rust via rustup (the repository pins its toolchain) and Node 22+ with
npm; see [prerequisites](deployment-prereqs.md). A prebuilt package uses
`preflight --skip-build` and does not need Rust/Node for building.

## Install and validate

```bash
bash scripts/first-run.sh --provider codex
# Or --provider grok; --provider api-key is an explicit last-resort choice.
# For a prebuilt package, add --skip-build.
```

Follow the account login prompts. Grok needs a CLI login and a separate in-app
device-code login; both are explained. Normal account login, provider choices
and administrator consent are expected user interactions.

New installations generate a random database key in a `0600` file under
`~/.config/aegis-runtime/keys/`, outside the checkout and data directory. The key
is never printed or written into `.env`; the service loads the same file on
every start. `scripts/aegis-runtime-local.sh paths` shows its location. Back up
the key separately from the encrypted databases. File permissions do not protect
against administrators or a compromised user account.

An existing database must retain its original key. Missing keys stop setup;
they are never regenerated over existing state. Set `AEGIS_RUNTIME_DB_KEY_FILE`
to use an existing private key file. An injected environment key is not silently
copied to disk. A passphrase-based database cannot simply switch to a raw key
file: retain its foreground/secret-provider workflow until explicitly migrated.

Setup enables a user service with automatic failure restart, validates the
console and one real owner chat response, and restarts for a fresh owner login
link. The probe uses the selected provider quota. Skipped or failed checks are
not acceptance. Local Ollama also uses a persistent service; an unknown process
on its port is not silently adopted.

```bash
bash scripts/aegis-runtime-local.sh status
bash scripts/aegis-runtime-local.sh login
bash scripts/aegis-runtime-local.sh restart
bash scripts/aegis-runtime-local.sh stop
```

Login links are private. A generated link stays valid until the process restarts
(each start prints a new one), so treat it like a password.
No Google project is needed. Existing explicit Google configuration is retained;
its UI entry appears only when enabled and configured.

## WSL: finish on Windows

After successful chat, Linux setup returns exit **2** on WSL to signal the
remaining Windows step. From Windows PowerShell, specify the actual distribution,
ordinary Linux user and canonical checkout path:

```powershell
.\scripts\install-windows.ps1 -Distribution Ubuntu -LinuxUser youruser -LinuxRoot /home/youruser/aegis-runtime
```

This installs a limited-privilege task for the current Windows user, with a logon
trigger and no saved Linux key/password. It holds a WSL session while that user
is logged in. Another user's task or changed task action is never overwritten.
It works even when WSL Windows mounts/interop are disabled.

The current user's Start menu receives **Aegis** and **Sign in to Aegis**
shortcuts. The first opens the workspace; the second announces a restart before
creating a fresh login link. Keep this Windows script at its installed path.
Shortcuts run in a hidden PowerShell window; errors appear in a dialog. They do
not change the machine's execution policy.

Use the same arguments with `-Action Open` for the workspace, `-Action Login`
to restart and open a fresh private login link, or `-Action Status` for task
status. Login restart interrupts active requests and is explicitly announced.
In remote mode, both launchers use the configured HTTPS Passkey entry instead;
sign-in does not restart the service. Private access requires Tailscale on the
device opening that entry. Local health probes remain separate from browser access.
Sleep, logout, shutdown and network loss interrupt availability.
A configured logon trigger is not evidence of a tested Windows reboot.

Close the deployment agent and its terminals. In separate Windows PowerShell:

```powershell
.\scripts\install-windows.ps1 -Action Verify -Seconds 1800 -Distribution Ubuntu -LinuxUser youruser -LinuxRoot /home/youruser/aegis-runtime
```

The idle interval checks only Windows HTTP and Task Scheduler. Do not run Linux
commands or browse WSL UNC files: those can wake WSL and hide the original
shutdown defect. WSL boot IDs are compared before/after. Windows reboot,
logout/login and sleep/resume need separate real tests.

If Linux responds but Windows refuses the same loopback port, check WSL localhost
forwarding, port conflicts, and `netsh interface ipv4 show excludedportrange protocol=tcp`.
A port randomly available inside Linux may be reserved on Windows. Choose an
explicit available port in configuration and restart; the installer does not
silently switch ports or expose the service on the network.

## Repeat, update and remove

Rerun setup to reuse this checkout's service and original key. Keep installed
paths stable. To update, stop the service, back up encrypted state with the
repository's backup helper, update/build, rerun setup and repeat the chat probe.
Migrate changed state/key paths explicitly.

Remove the Windows task and owned shortcuts first (`-Action Remove` with the same arguments), then
run `scripts/aegis-runtime-local.sh remove` in Linux. Removal preserves data,
keys, provider logins, configuration and independently installed Ollama. It does
not disable lingering, which other services may use.

For [remote access](REMOTE_ACCESS.md), inject the managed key into a host command
without putting its value on the command line:

```bash
python3 scripts/managed-runtime.py with-key python3 scripts/remote-access.py --mode private
```

`--portable` explicitly selects temporary background operation and warns that
agent exit/logout/WSL shutdown may stop it. `--no-start` and `--auth-only` retain
advanced foreground setup; they are not durable installs.

## Linux containers and foreground operation

Choose this route explicitly on a Linux cloud/container host without a systemd
user manager. Do not fake systemd or enable lingering inside an unsupported
container to make managed preflight pass. The normal `first-run.sh` command
still requires systemd; it never silently falls back to this route.

### Build without accounts

On restricted cloud agents, dependency caches must be writable. If the home
directory is read-only, choose writable paths explicitly **before installing
dependencies**, for example (replace `/workspace` for your host):

```bash
export CARGO_HOME=/workspace/aegis-deps/cargo
export RUSTUP_HOME=/workspace/aegis-deps/rustup
export npm_config_cache=/workspace/aegis-deps/npm
# If Rust is not installed, use the official installer without editing HOME:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
source "$CARGO_HOME/env"
rustup target add wasm32-wasip1
```

Keep these environment choices in every build shell. This is dependency storage,
not secret storage. Do not disable TLS or package integrity checks to fix a
download/cache failure. Standard npm/Cargo caches, builds and browser downloads
require disk space and network access to dependency sources.

Install the same Rust 1.98.1, `wasm32-wasip1`, Node 22+ with npm, C compiler,
`pkg-config` and OpenSSL headers listed in the README. Python 3 and Bash are
required. Build dependencies are not bundled. No provider CLI is required just
to build. For a build-only workflow, run this directly from the checkout:

```bash
python3 scripts/foreground-runtime.py build
```

Build checks its own tool prerequisites, then runs `npm ci`, the production web
build, and `cargo build --release -p aegis-runtime-core -p aegis-executor`.
It does not require `preflight` or free runtime ports: an occupied port can block
runtime preparation while the build still succeeds. It creates build outputs
only: no `.env`, login, encryption key,
database or service. A failed command stops the build. `CARGO_TARGET_DIR` and
`CARGO_BUILD_JOBS` are respected; parallelism defaults to two jobs.

### Prepare and run the real application (owner action required)

Check runtime prerequisites before account setup or startup:

```bash
python3 scripts/foreground-runtime.py preflight --skip-build
```

This checks unused loopback ports (defaults 3000 and 8765) and runtime tools,
without calling systemd or a provider. Omit `--skip-build` if you also want to
check build tools while preparing the host for both build and startup. These port
checks belong to runtime preparation, not the build-only workflow. `run` repeats
the runtime checks before executing the application; existing listeners are
never stopped or silently assigned different ports.

An account-free build is not a usable owner-chat installation. Stop here if
provider login or persistent credentials are outside the request's authorization.
Before real startup, the owner must complete the README provider sign-in and
`setup-auth` binding and arrange the same recoverable database key/passphrase
for setup and every run. No login, key generation or auth bypass is built into
the foreground launcher. A headless run requires an externally injected
`AEGIS_RUNTIME_DB_KEY_HEX`; never put it in `.env` or an argument. Interactive
terminals may use the runtime's passphrase prompt. Do not substitute an ephemeral
key for durable application state.

After the owner authorizes login and persistent application state, the existing
wizard can prepare configuration/binding without starting a service.
Choose a writable, owner-controlled persistent `AEGIS_RUNTIME_STATE_DIR` first
if the default home state directory is unavailable. The wizard records database
paths there; the key must remain in separate custody, outside that state volume.

```bash
# The owner must already have chosen the key/passphrase and authorized login.
# This explicitly defers semantic recall: no automatic Ollama service install.
AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING=1 bash scripts/first-run.sh --no-start --skip-build --provider codex
python3 scripts/foreground-runtime.py run
```

`--no-start` still performs account login/binding and writes `.env` and application
state; it is **not** an account-free check. Grok requires both documented sessions;
API-key setup can make separately billed validation calls and needs separate
authorization. Configure a working embedder separately before accepting semantic
memory. The example explicitly leaves semantic recall unavailable until then.

`run` checks unused ports and refuses a checkout with an existing managed
installation. It accepts `AEGIS_RUNTIME_BIN` or the release binary under
`CARGO_TARGET_DIR`, then uses the existing foreground exec path. It preserves
process signals/exit status, starts no nohup process, and installs no restart
policy. It does not publish ports or configure remote access. Keep the console
on loopback; phone access remains a separate authorized setup. Startup logs can
contain private login links; never publish raw runtime logs.

Use the README's browser login and real owner-chat probe after starting. `/health`
alone is insufficient. Keep state and the original key across container replacement
under owner-controlled storage; process supervision, restart policy and volume
custody belong to the host/container operator. Exit, eviction or replacement can
stop the process. No logout/reboot/eviction persistence is promised by this script.

### Account-free verification (not provider acceptance)

For a source change or an explicitly requested verification, run `make check` and
the credential-free commands in [REMOTE_ACCESS.md](REMOTE_ACCESS.md#credential-free-verification).
These work without systemd, but are not all offline: `make check` includes browser
fetch fixtures requiring system DNS to resolve `example.com` publicly. The browser
suite also needs the matching Playwright Chromium download and its OS libraries.
Report sandbox DNS/download failures separately; do not count skipped checks as
passes or change network/security policy to hide them.
The documented `remote_access_harness` starts a real
loopback HTTP router with disposable encrypted databases and a virtual browser
authenticator. Its fixture key is ephemeral; it has no model provider, and must
never be used as the production runtime. `test_access_lifecycle.py` uses that
harness and a simulated Tailscale command, not a real tailnet.

After building the harness and web console, run
`python3 scripts/test_foreground_lifecycle.py` (also part of
`make remote-access-test`). It launches that same fixture through the foreground
entry point, checks HTTP/UI, signals, encrypted reopen and wrong-key rejection,
then deletes its temporary state. It never runs an account-backed chat.

`make installer-test` includes foreground behavior tests: missing tools, occupied
ports, managed-install refusal, build failure propagation, missing/invalid key
rejection, and exec signal/exit behavior. Its child stubs are not runtime/provider
acceptance. Build, loopback harness/UI, real owner login/chat, semantic recall and
container persistence must be reported separately. A new first-time acceptance
trial must start in a newly provisioned task; a repair session is not blind.

## Release verification is separate

Run `make check`, remote/browser/encrypted-lifecycle tests and privacy review on
the release candidate. `make install-smoke` is the small real-deployment check
and uses the selected provider quota. Simulated service managers, virtual
WebAuthn and local providers do not prove real account, device or reboot results.

After `make package`, run `python3 scripts/test_package.py dist/<archive>.tar.gz`
on Linux to check the extracted layout and executable startup without accounts.
Add `--preflight` on a host with its user service manager already configured.
This does not replace a real installation and chat probe.

### Windows task recovery

Re-running the Windows installer updates recovery settings only after checking
ownership of the existing task. The hidden Windows session supervisor retries a
failed or unexpectedly ended WSL hold once per minute, up to 999 attempts.
Installation checks actual host HTTP readiness, not just a running supervisor.
A missed scheduled start runs when available. It does not wake a
sleeping computer or make an interactive-user task available while logged out.
Persistent failures still need repair; inspect `-Action Status` and Task Scheduler.
A process retry test is separate from real reboot, logout/login and sleep/resume
acceptance. Stop/remove remains explicit and is not treated as a failed exit.
