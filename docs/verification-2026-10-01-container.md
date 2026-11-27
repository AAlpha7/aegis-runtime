# Explicit container/foreground candidate — 2026-10-01

Status: **private implementation candidate; real owner deployment not accepted**.
This is an informed repair session, not a new blind first-time installation test.
The original failed trial and its transcript remain unchanged outside Git; all
12 files in its checksum manifest were verified unchanged after implementation.

## Source and environment

Base main: `db442a018d8c6f615529fca36d4329c6bdd289bb`.
Implementation: `95f1cf41f4d956c6d54bf8715594da0a13908c98`, on local branch
`fix/container-foreground`. This report, its README link and the explicit cloud
dependency/network guidance are a documentation-only follow-up. No push, PR, main
merge, publication or production change was performed.

The same Debian 13.6 x86_64 cloud container was used. PID 1 is `tail`, with no
systemd user manager. Node 24.19.0, npm 11.9.0, Python 3.12.14, GCC 14.2.0,
OpenSSL headers 3.5.7 and Codex were preinstalled. The Rust 1.98.1/rustup and
Wasm target installed in the first trial were reused. No application state or
provider login was imported. The cgroup limits are four CPU equivalents and
16 GiB memory; the workspace filesystem is approximately 32 GB. Build parallelism
was two jobs per Cargo command. This is not an empty machine or cold OS test.

## What changed

- `scripts/foreground-runtime.py` provides explicit `preflight`, `build` and
  `run`. Build does not require accounts, systemd, free runtime ports or keys.
  Run uses the existing foreground exec path, preserving signals and exit status.
  It rejects a managed installation and rejects unattended startup without an
  externally supplied valid database key. It neither generates a key nor logs in.
- `scripts/managed-runtime.py` shares tool/port checks; defaults retain managed
  service and lingering requirements. The foreground port check allows TCP
  TIME_WAIT but still rejects a live listener. A behavior regression first failed
  without this change and passed after it; that failure log is retained.
- The `--no-start` message now directs users to foreground `run`, not background
  `start`. Supervisor comments, README, AGENTS and INSTALLATION distinguish this
  route from durable systemd installation and temporary `--portable` operation.
- Twelve CLI behavior tests and an encrypted harness lifecycle test cover the
  new route. They are wired into the existing installer/remote test targets.
  No Rust runtime, frontend implementation, dependency lock or existing assertion
  was changed, and no test was weakened.

## Verification

| Check | Observed result |
| --- | --- |
| Foreground preflight | Exit 0 without systemd; both source and prepared-host checks |
| Normal managed preflight | Exit 1 for unavailable systemd user manager, still enforced |
| Source build through new entry | Exit 0; production web build and release runtime/executor build; Cargo release phase 16m 05s |
| Rust format and Clippy | Exit 0, Clippy with warnings denied |
| Full `make check` | Exit 2: 4,511 native passes, 4 failures, 47 existing ignored across 39 result groups; core library 4,143 passed/3 failed/4 ignored in 521.11s |
| Frontend tests, independently completed | 1,063 reported passes, zero failures; counts can overlap across scripts |
| Installer behavior | 21 existing managed tests and 12 new foreground tests passed |
| Existing wizard / release fixtures | 14 wizard tests and 10 packaging/history/compiler tests passed |
| Remote helper tests | 11 passed; no real Tailscale daemon/account |
| Existing encrypted harness build | Exit 0, 6m 37s reported by Cargo |
| New foreground harness lifecycle | Both starts served HTTP and UI resources, reopened the encrypted database, terminated by signal, and rejected a wrong key without changing database bytes |
| Existing access lifecycle | All four private/public/private/local transitions passed using simulated Tailscale; encrypted state retained |
| Chromium installation | Failed: official Playwright download returned HTTP 403 `Domain forbidden`; no permissions or download policy changed |
| Browser / virtual-authenticator journeys | Not run: required browser unavailable |
| Real application foreground launch | Stopped before exec, exit 1: unattended startup needs external `AEGIS_RUNTIME_DB_KEY_HEX`; no key generated |
| Real account / provider owner chat | Not run; no login requested and no paid call made |
| Actual systemd / WSL / container replacement persistence | Not run; fixtures are not evidence of host lifetime guarantees |
| Diff/shell syntax and tracked privacy scan | Passed; zero privacy rule findings, with manual review still distinct |

The first account-free build stopped in npm because its default cache under the
restricted home directory could not be created (ENOENT). The failed output is
retained. Explicit `npm_config_cache` in the writable workspace allowed dependency
installation and the build to finish. Package integrity/TLS checks were retained;
no lockfile or dependency version was changed. Documentation now explains writable
cache locations and rustup's `--no-modify-path` option for restricted agents.
The production build emitted the existing large-chunk warning; Cargo reported an
upstream future-incompatibility notice for `proc-macro-error2`, not a Clippy failure.

The three native failures were existing `orchestrator_tools_browser` tests:
`pipeline_text_mode_without_chrome`, `pipeline_truncates_long_body`, and
`pipeline_selector_returns_matching_element`. The first explicitly failed its
public DNS assertion for `example.com`; an independent system lookup also failed
with `EAI_AGAIN` (temporary name-resolution failure). The other two returned an
error where they expected success and use that same domain/DNS path. This supports
a shared environment DNS explanation, although those two assertions do not print
their underlying error. They use fake Chrome and are independent of the separate
Chromium download denial. No production DNS guard, test assertion, resolver or
network policy was changed; the failed full gate was not retried into a pass.
The fourth failure was `wasm_integration::real_web_reader_allowed_domain`, which
also uses `https://example.com/page` and received `blocked` instead of `ok`.
Its assertion does not print the underlying network error; the observed DNS
failure is consistent with that result, not a separately proven root cause.
The command completed the remaining native targets with `--no-fail-fast`; Make
then stopped before its frontend/Python targets. Those targets' passing results
above came from the independent completed commands, not a passing full gate.

Wasm skills were compiled from source before native tests; the Makefile then
reused those artifacts by its normal canary rule. Neither historical results nor
the browser download failure are counted as passing execution. Early local logs
contain interleaved output from concurrent build/test processes; later commands
have individual timestamped logs. Raw application login output is not published.

## Remaining acceptance

The new launcher can build and execute an account-free local fixture in this
container. A real owner installation still needs explicit authorization for a
recoverable external key/passphrase and durable application state, fresh provider
login/binding, a real owner-chat round trip, and a separately configured embedder
before semantic recall is accepted. The foreground script supplies no host restart
policy or volume/key custody. Container replacement, logout, reboot, WSL lifetime,
phone access and real provider behavior need their own observed checks.

No real application `.env`, default state directory or managed key directory was
created in this session. All lifecycle keys and databases were disposable test
fixtures. No credentials from other projects/machines, security changes, live
Tailscale exposure, production access or paid API calls were used.

Run the next first-time acceptance in a newly provisioned task using only the
candidate repository documentation. Do not transfer this session's state, account
information or troubleshooting conversation, and do not call this repair a blind
acceptance. A host that permits the official browser download (or already has the
matching browser installed) is required for the pending browser suite.
A full native gate also needs the public DNS resolution expected by those fixtures.
