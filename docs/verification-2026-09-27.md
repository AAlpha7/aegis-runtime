# Remote access and release handoff — 2026-09-27

Current decisions, completed fixes and remaining acceptance are tracked in the
[V1 checklist](V1_ACCEPTANCE_CHECKLIST.md).

For later real Codex, private Tailscale/iPhone and second independent deployment
results, see [the external acceptance follow-up](acceptance-follow-up-2026-09-27.md).
The historical snapshots below retain their original verification boundaries.

## Latest follow-up: durable installation and consistent secure-access UI

Implemented on `feat/durable-first-run`, following the independent deployment
observations in [the fresh-machine trial](fresh-machine-trial-2026-09-27.md).
That trial's deployed clone remains separate from this candidate.

- Normal first-run now performs dependency/service preflight, creates an external
  private database-key file for new state, and installs a systemd user service.
  Missing keys for existing state stop setup; they are not regenerated. Local
  embedding uses an identified persistent service. Portable lifetime is an
  explicit, visibly disclosed option.
- WSL additionally has a current-user, limited-privilege Windows session task,
  owned Start menu launchers, and a host-side verification command. Repeated
  installation preserves the same key and task. Removal preserves databases,
  keys and independently managed services. No blanket sudo grant or credential
  transfer is used. The login shortcut explains and confirms its service restart.
- First-run verifies the selected Grok/Codex/API chat candidate before storing
  its settings; failed validation retains prior settings and restores staged
  `.env` edits. Jev opt-in also requires a successful semantic probe. Probes do
  not fall back to a different provider to make an invalid candidate pass.
- Google login is explicitly off in new defaults. Existing explicit opt-in is
  retained. `/api/session` describes available sign-in methods, and the console
  hides unconfigured Google/Passkey choices. Local sign-in requires no Google
  project. Heirloom remains off and realtime voice remains deferred.
- `/access` now uses the console's shared theme tokens, responsive spacing,
  light/dark colors, bilingual labels, clear retry states and optional pairing/
  recovery sections. No external fonts, analytics or secret persistence were added.
- The installation guide and agent handoff separate deployment smoke acceptance
  from contributor tests. Packaging includes the correct `webapp/dist` layout,
  managed scripts and installation documentation. A Linux/Windows verification
  workflow was added locally; it has not run on hosted CI.

Observed checks on this candidate:

- Complete `make check`: exit 0. Core library 4,114 passed, 4 ignored; executor
  41 passed; main binary 35 passed. Integration/doc tests, the complete frontend
  suite, 11 Jev wizard tests and 17 managed-installer tests completed successfully.
  Counts from focused reruns are not additional unique tests.
- Final `cargo fmt --all -- --check`, all-target Clippy with warnings denied,
  production WebApp build and optimized Linux release build passed. Cargo still
  reports an upstream future-compatibility warning for `proc-macro-error2 2.0.1`.
- All 17 HTTP/Chromium/SQLCipher/virtual-WebAuthn browser journeys passed,
  including Google hidden by default, local-only guidance, language switching,
  pairing failure/retry, normal login, encrypted restart, revocation and recovery.
  Desktop 1280px, phone 390px dark, and 320px light/error screenshots were inspected;
  the tests assert no horizontal overflow. Screenshots contain synthetic state only.
- Real systemd user-service lifecycle passed: survival after installer-process
  exit, SIGKILL recovery with a new process, wrong-key refusal without changing
  encrypted database bytes, recovery with the original key, and scoped removal
  retaining data/key. This uses an isolated harness, not a live provider.
- Python remote-access checks: 11 passed. The encrypted access lifecycle passed
  its four transitions using simulated Tailscale. Windows installer contract
  checks passed. Installer/key edge cases use a simulated service manager; the
  separate real-systemd checks above establish actual service behavior.
- The actual Windows task was installed twice without duplication. After Linux
  builds/tests exited, a 120-second interval used Windows HTTP and task checks
  only: both endpoints stayed HTTP 200, the task stayed running, and WSL boot IDs
  before/after matched. This was a temporary encrypted harness on the current
  host, with another trial distribution running; it is not a clean-machine,
  30-minute, logout/reboot or live-provider acceptance result.
- A local Linux archive was built from the verified release artifacts. Its
  extracted files, executable argument handling, LF shell scripts and real-host
  `--skip-build` preflight passed `scripts/test_package.py --preflight`. No account
  or chat was used by that archive check. No archive was uploaded.
- Privacy scan returned zero findings across tracked and non-ignored new files;
  this supplements manual review and is not a guarantee.

Failures found and corrected during verification:

1. The new fixed-content API setup probe needed registration in the direct-call
   guard. After adding its explicit-API/no-fallback rationale, the full suite passed.
2. Same-page pairing navigation could retain a previously hidden Passkey action.
   Capabilities now refresh on hash navigation; the actual browser journey passes.
3. Task Scheduler canonicalizes a Windows SID to an account name. Ownership checks
   now resolve it back to a SID, so a legitimate repeat installation succeeds.
4. Quoting the validated WSL distribution/user names in the task's raw arguments
   caused literal quote handling and a distro-not-found failure. Paths remain
   quoted; identity fields use validated unquoted tokens. The real task stays running.
5. A randomly chosen Linux test port fell inside a Windows excluded range. The
   Linux service and task were healthy but Windows HTTP failed. The fixture now
   uses a port actually bindable on Windows. Installer errors and documentation
   distinguish this host networking failure; no automatic port/network fallback
   was introduced.

Remaining acceptance: a fresh independent-agent deployment of this candidate,
real subscription/API/Jev probes of the new setup paths, real mobile/Tailscale
access, 30-minute unattended acceptance, Windows logout/login, reboot and
sleep/resume, anonymous cloning after publication, and hosted CI. The earlier
trial's real Codex chat does not prove this candidate's provider acceptance.
No production account, existing trial deployment or public release was changed.

## Previous follow-up: validate call configuration before saving

Owner call settings now probe the exact candidate before any configuration
write: chat provider/model, Codex model/reasoning, search provider/model,
backup model, paid API fallback and Jev enable/key changes. Main provider/model
selection is staged together. A failed probe returns HTTP 422 with a safe reason
and leaves previous live/persisted values unchanged; another provider cannot
make a failed candidate pass. Search requires usable source URLs. Jev requires
valid decision answers, not just HTTP 200, and records a key/model-bound check
time in the encrypted credential record. Legacy Jev settings remain unverified
and can be checked without re-entering the saved key.

Disabling/removing Jev, clearing backup and disabling paid fallback work
without provider checks. Choosing a backup explicitly describes its automatic
activation and separate API billing; activation happens after its successful
probe and clearing turns it off. Generic admin writes/resets and chat approvals
cannot bypass the protected owner settings flow. The page does not automatically
resubmit a failed save; an uncertain transport result asks the owner to reload.
Host bootstrap/environment values are not evidence of a successful live probe.

Observed verification:

- Final focused Rust: 7 validation behavior tests, 161 settings tests, 20 Jev
  tests, 1 generic-config approval regression and 2 route-coverage checks passed.
  Filters overlap; these counts must not be added as unique tests.
- After registering the direct probe, all 18 source guards and final all-target
  Clippy with warnings denied passed. Git whitespace checks passed.
- Production WebApp build and complete `npm test` passed. Old static wiring
  assertions were updated for staged provider/model saves and saved backup
  summaries; browser tests exercise the actual interactions.
- Final browser acceptance passed all 15 journeys, including staged provider
  changes, a rejected save retaining the prior provider/model, explicit retry,
  Jev rejection without key persistence, successful verification, rechecking a
  legacy saved key without re-entry, removal and restart. Chinese/English 390px
  layout has no horizontal overflow; the Chinese screenshot was visually inspected.
- Python remote-access checks: 11 passed. Privacy scan: zero findings in tracked
  files; this supplements manual review and does not certify untracked content.
- A broader core run finished with 4107 passed, 2 failed, 4 ignored. The failures
  were an old approval test expecting unverified model changes and the direct
  API transport allowlist. The approval test now uses a generic non-call setting,
  with a separate test refusing unverified model changes. The direct probe is
  explicitly registered with its no-fallback rationale and transport test.
  Both failing checks passed their focused reruns.
  The full core suite was not repeated after these focused corrections.

These are account-free local tests. The semantic Jev and API/search probes use
local HTTP fixtures; the browser example installs an explicit synthetic upstream
probe. No real Jev, Grok, Codex or paid API credentials were validated and no real
paid calls were made. Save-time checks prove availability at that time only.
No deployment or publication occurred in this follow-up.

## Previous follow-up: Jev in Settings

Settings > Advanced > AI and spending now includes Jev's optional configuration
card in Chinese and English. It shows off/configured-but-unverified state,
explicit data-sharing/billing consent, a write-only OpenRouter key input,
Save/Cancel, reload and key removal. Removing the saved key also turns Jev off;
host credentials are identified separately and cannot be removed by this page.

The owner-only GET/POST endpoint enforces session authentication, CSRF, bounded
strict JSON and consent. A single encrypted credential-store record commits the
choice and key atomically; keys do not enter runtime-settings projections or
responses. Saved choices override the environment, including Off. Subsequent
outbound/non-CU/computer-use calls resolve the saved choice; unreadable records
fail closed to local checks/approval, not environment billing. Each computer-use
call has its own policy snapshot while retaining its shared device/audit chain.
Already-sent requests are not recalled. Saving performs no live paid API probe.

Observed verification for this follow-up:

- Initial focused Rust: 4 Jev configuration/router/runtime/policy tests, 65
  computer-use tests, 16 outbound-gate tests passed; approval routing 1, React
  loop 35 (5 ignored), route coverage 2 passed. After strengthening concurrent
  policy isolation, the final rerun passed all 65 computer-use and 4 Jev tests.
- WebApp production build, settings rendering and bilingual catalog checks
  passed. The first full frontend run caught the missing new-component UI
  scenario registration. A real executable scenario was added and passed its
  coverage gate; the complete `npm test` rerun passed (exit 0).
- Real HTTP/Chromium/SQLCipher/virtual WebAuthn: 14 journeys passed, including
  Jev consent, save/reload, cancel, simulated save failure/recovery, removal and
  restart persistence. English/Chinese screenshots at 390px were inspected;
  no horizontal overflow. Chinese visual verification uses host-installed fonts
  read-only through an ignored local fontconfig file; fonts are not distributed.
- Privacy scan: zero findings; Git whitespace check passed. Final all-target
  Clippy with warnings denied passed (exit 0).

These are local checks with synthetic credentials, not live Jev/subscription/API
acceptance. No deployment, publication, real account or paid request was made.

## Earlier follow-up: optional Jev question during API-key setup

The interactive first-run API-key wizard now offers Jev after explaining separate
OpenRouter charges and the outbound/action context sent for assessment. The
default is No; only explicit Yes plus a compatible credential enables it.
Existing choices are retained; `--yes`, piped input and unattended setup cannot
grant consent. Generic OpenAI keys and placeholders cannot enable Jev. Separately
injected Jev credentials are not copied into the setup file or printed.

WSL `make jev-setup-test`: 9 tests passed, exercising the actual first-run script
with PTY input, temporary directories, synthetic keys and a stub auth binary.
Cases include yes/no/blank/invalid responses, missing and dedicated credentials,
preserved choices, environment precedence and noninteractive behavior. Bash syntax
and Git whitespace checks passed; privacy scan: zero findings. This scoped suite
is now part of `make check`; the whole check was not rerun for this shell-only
follow-up. No live account, Jev API call, deployment or publication was performed.

## Earlier follow-up: Jev disabled by default

Jev now requires explicit `AEGIS_JEV_ENABLED=1` (or `true`/`yes`) and a valid
API key. Missing, false or unknown flag values keep it disabled even if API
credentials are configured. The shared resolver returns before reading keys
when disabled, covering outbound, computer-use and non-CU production factories.
Local checks and approval holds remain; public-search notices now mention the
disabled state. Health exposes the explicit `jev_enabled` flag.

WSL focused verification: computer-use tests 64 passed (including default-off,
credential precedence, opt-in and missing-key behavior); egress tests 16 passed.
All-target Clippy with warnings denied and Git whitespace checks passed;
the privacy scan reported zero findings. This follow-up uses scoped regression
checks; the earlier full `make check` below predates this small configuration
change. No real Jev service or credentials were used and no deployment occurred.

## Earlier follow-up: visible behavior changes and Jev

Owner turns now collect runtime-authored, localized disclosures independently of
model prose. They stream as owner notices and survive final review, history
storage, and failed-turn recovery. Duplicate notices are collapsed per turn;
concurrent chats are isolated. Search reports preferred-route failure, the
DuckDuckGo attempt and its actual success/failure separately. X search retains
the actual underlying provider instead of labeling every fallback as Grok.
See the disclosure matrix and Jev deployment contract in [SUBSCRIPTIONS.md](SUBSCRIPTIONS.md).

Jev remains an optional, separately billed OpenRouter Decisions integration.
An explicit `AEGIS_JEV_API_KEY` takes precedence; generic OpenAI credentials are
not reused for OpenRouter. Public-search local-rule exceptions are disclosed;
other outbound holds remain. No-key computer actions default to Ask instead of
Allow, retaining hard denies and the read-only session-probe exception.

Verification uses WSL, the pinned Rust toolchain, Node 22.20.0, one build job,
and the existing external Cargo target. No real accounts were used.

| Check | Observed result |
| --- | --- |
| Initial focused behavior | Notices 3/3; search/executors 18/18; egress 17/17; computer-use 62/62. Later tests also cover typed failed-turn notices, credential routing, Retry preservation and HTTP failure notices. |
| First full check | Clippy passed. The new async wrapper caused stack overflow in the core large-image case, intent-engine integration and Telegram callback integration. These were real failures, not passes. |
| Stack repair | The scoped inner future is now heap-allocated. The actual multi-megabyte image test and both affected integration binaries passed without increasing thread stack limits. |
| Model fallback fixture | The focused run exposed a shared catalog fixture collision on `backup-model`. The catalog-owner fixture now uses a unique model id, leaving production routing unchanged. |
| Frontend focused | 85 assistant-response rendering tests and 24 error-copy tests passed, including Chinese/English notices with a restored search receipt and text/JSON failure responses; frontend/preview typechecks and production build passed. |
| Local access regression | 12 HTTP/browser journeys, 11 deployment tests and portable supervisor private/public/private/local encrypted-state lifecycle passed. Tailscale was simulated; browsers used a virtual authenticator. |
| Privacy | Source scan reported zero findings; no credentials or raw logs are release inputs. |
| Full rerun after stack repair | `make check` passed (exit 0): core library 4,099 passed / 4 ignored, executor 41 passed, host binary/integration/doc-test targets and the complete frontend suite passed. |
| Final approval replay follow-up | After the full check, direct approved-tool execution received a separate, heap-allocated disclosure scope so its human receipt retains notices without changing raw tool JSON. The new replay test passed; HITL unit tests 46 passed; routing linkage 1, intent engine 9, React loop 35 (5 ignored), and Telegram callbacks 19 passed. Final all-target Clippy with warnings denied passed. This final localized change was checked with these scoped suites, not another full `make check`. |

The new tests use fixed Wasm output, local HTTP/mock executors and simulated Jev
judges. They establish disclosure behavior, not live DuckDuckGo quality, actual
Jev API access/billing, real subscription execution or external network access.
Realtime voice remains deferred. No deployment, publication or account mutation
was performed.

## Earlier follow-up: subscription execution, search and v1 scope

Implemented on `feat/subscription-first-v1`, carrying the Heirloom follow-up
below. Subscription binding enables the matching executor while preserving
explicit opt-out. Automatic search follows Grok/Codex, with signed DuckDuckGo
Wasm fallback. Paid search requires explicit selection. Codex checks ChatGPT
login and connector access; connector writes are outside v1 acceptance.
Default builds disable realtime minting and hide its WebApp entry. See
[subscription behavior](SUBSCRIPTIONS.md).

Environment: Ubuntu WSL, Rust 1.98.1, Node 22.20.0, one Cargo build job and an
external target directory. Protocol tests use in-memory JSON-RPC peers;
fallback tests execute real Wasm with fixed test output, not live DuckDuckGo.

| Check | Observed result |
| --- | --- |
| Subscription-focused behavior | Final rerun: 32 passed, 1 ignored. Covers automatic Grok/Codex selection, explicit disable, no default cross-provider retry, search evidence, actual signed Wasm fallback and empty-result rejection, alongside existing subscription tests. |
| Prompt snapshots | 21 passed after reviewing the three intentional prompt changes. |
| Settings UI and build | Complete `make webapp-test` passed (exit 0), including 56 settings/status rendering tests. Production WebApp build passed. |
| Final focused checks | Executor/prompt group: 21 passed; settings: 18 passed; search: 7 passed; localization: 23 passed. Includes provider activation, removal of stale search model labels, and unverified connector status. |
| Real HTTP/browser | Twelve journeys passed, including authenticated search-provider changes without stale model labels, HTTP 501 for realtime mint and absence of the voice agent after the actual WebApp mounts. Existing WebAuthn/SQLCipher/recovery journeys remain green. |
| Encrypted startup/lifecycle | Thirteen Heirloom service/setup-auth cases passed; real portable supervisor private/public/private/local transitions and encrypted state persistence passed with a simulated Tailscale executable. |
| Deployment and privacy | Eleven deployment tests passed; privacy scan reported zero findings. |
| Final formatting/lint | Rust formatting and Git whitespace checks passed. Final `cargo clippy -p aegis-runtime-core --all-targets -- -D warnings` passed after the search retry repair. Existing dependency future-compatibility warnings remain. |
| Core library full run and corrections | 4,089 passed, 3 stale expectations failed, 4 ignored. The failures were the previous Google connector hint, Grok-only search prompt assertion and external-tool schema snapshot. All three were updated for the intended behavior and individually rerun successfully. The final protocol-only rerun passed 15 tests. This is a full run plus scoped corrections, not a claim that the initial `make check` exited successfully. |
| Remaining Rust targets | Executor: 41 passed; binary/integration/doc targets completed. The first run also exposed two empty-search retry regressions. After preserving a completed empty primary lookup when the Wasm fallback fails, both affected integration suites were rerun in full: `react_loop_integration` 35 passed/5 ignored; `user_journey_v2` 14 passed. All other targets passed or contained only intentionally ignored live-service cases. |

The first browser attempt overlapped `npm ci` and failed to import Playwright.
After dependency installation completed, the actual browser suite passed.
The search request-count assertion was also corrected to inspect the request's
`model` field: a later chat message can legitimately contain the `:online` model
name inside a route receipt without being an additional paid search request.

These checks do not establish real subscription login or quotas, Grok CLI saved
billing configuration, installed app-server compatibility, live connector
access, live DuckDuckGo availability, mobile access or host reboot. External
connector mutations and realtime voice remain outside this release's acceptance.
No hosted service, production account, deployment or release upload was changed.

## Earlier follow-up: Heirloom disabled by default

The `fix/heirloom-default-off` change follows merged baseline `005533f`.
Only `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on` opts into the experimental subsystem.
Disabled startup excludes its path from SQLCipher registration/preflight and
skips all bridge initialization; stale profile-read settings cannot enable it.
Standard encrypted memory remains active. Existing Heirloom files are retained,
and explicit re-enable still performs encryption preflight. See
[Heirloom status and upgrade behavior](HEIRLOOM.md).

Current verification environment: Ubuntu WSL, Rust 1.98.1, Node 22.20.0,
two Cargo build jobs, an external `CARGO_TARGET_DIR`, temporary encrypted state,
and Chromium's virtual authenticator. No real credentials or public routes used.

| Follow-up check | Observed result |
| --- | --- |
| `make check` | Passed (exit 0): Clippy, host Rust tests and the complete frontend suite. Core library: 4,086 passed, 4 ignored; executor: 41 passed. Binary, integration and doc-test targets also completed successfully; ignored live-service tests remain unverified. |
| `make heirloom-mode-test` / final Python rerun | Passed. Thirteen actual service/setup-auth invocations cover absent/off/unrecognized flags, no directory creation, ignored stale profile preference, inactive plaintext SQLite preservation, enabled plaintext rejection, encrypted creation, off/on restart, ignored inactive path alias, and wrong memory-key rejection. |
| Remote browser journeys | All ten passed against the real router, SQLCipher and WebAuthn verifier. |
| Deployment and lifecycle | Eleven deployment tests passed; real portable supervisor private/public/private/local transitions and encrypted-state persistence passed with a simulated Tailscale executable. |
| Formatting and privacy | Rust formatting check passed; privacy scan found zero unreviewed pattern hits. |

The first Clippy build reported a `web_search` Wasm compilation warning.
A direct build from the unchanged skill source subsequently passed and its
generated Wasm artifact was refreshed. Other dependency future-compatibility
warnings are not test failures. Raw logs and disposable test secrets are not
release inputs.

The new mode test deliberately uses loopback provider URLs and checks startup
and storage isolation, not model responses. Real provider chat, physical mobile
access, OS reboot/secret injection and production Heirloom custody remain
unverified. No release upload or deployed installation was changed.

## Earlier remote-access verification

This record supplements [database verification](verification-2026-09-26.md).
It covers the remote-access change following encryption commit `1d73476`.
The implementation is self-hosted and single-owner; publishing source does not
require an existing database, a production key, or an Aegisflow installation.

## Implemented behavior

- Both memory and heirloom SQLite databases use SQLCipher. External key injection
  is the default for background services; foreground terminal passphrases remain
  available. Phones never receive a database-key prompt.
- Portable installation offers local-only, private Tailscale Serve, and public
  Funnel access. Public mode requires separate, versioned risk acknowledgment.
  Managed remote listeners require HTTPS and a loopback backend.
- Host-issued pairing authorizes Passkey registration. Pairing expires after ten
  minutes; authentication challenges expire after five and are consumed once.
  WebAuthn verifies signed responses and user verification. Server-side state,
  credentials and sessions use the encrypted memory database.
- Origin checks, CSRF, bounded requests, secure HttpOnly cookies, a second
  credential, host recovery, and owner-session revocation are implemented.
  Recovery and pairing rotation invalidate pending registration. Registration
  verification/insertion is serialized with recovery so an in-flight request
  cannot restore a revoked credential afterward.
- Remote configuration disables the legacy local login and host-confirm routes.
  Proxy loopback or forwarded headers never establish host ownership.
- Mode changes close the previous owned route before restarting. Existing or
  concurrently changed unrelated Tailscale configuration is preserved. A cleanup
  failure explicitly warns that exposure may remain instead of claiming success.

## Verification evidence

Environment: Ubuntu WSL, Rust 1.98.1, Node 22.23.3, Python 3, Chromium virtual
CTAP2 authenticator. Four Cargo build jobs were used for development checks and
two for the release build, with an external `CARGO_TARGET_DIR`.

| Check | Observed result |
| --- | --- |
| Full core/executor Rust run | Core library: 4,083 passed, 4 ignored; executor: 41 passed. Integration targets ran; the new admin module initially failed the route sensor completeness check. After adding the module and actual router tests, that target passed (2/2). |
| Final affected Rust checks | Six remote-access unit tests, the new router behavior test, route coverage, formatting and Clippy passed. |
| Frontend | Complete `npm test -- --run` and production build passed; `npm audit --omit=dev` reported zero vulnerabilities. |
| Browser journeys | Ten passed against the actual HTTP router, SQLCipher and WebAuthn verifier, using a virtual authenticator. |
| Deployment logic | Eleven tests cover consent, mode changes, failed shutdown/restart, environment overrides, ownership/concurrent edits, legacy route adoption and daemon response shape. |
| Process lifecycle | Actual portable supervisor start/restart/status/stop and encrypted state survived private → public → private → local transitions; only the Tailscale executable was simulated. |
| Privacy | Tracked/new source scan found zero unreviewed pattern hits. Two exact synthetic security fixtures are explicitly exempted; commit identity uses GitHub noreply. No raw test logs, machine paths, credentials or private continuity contents are release inputs. |
| Shell syntax | Changed shell scripts passed `bash -n`. |
| Release build/package | Optimized build and `make package` passed; archive contains the service, executor, access helper, both database helpers, WebApp and three shipped Wasm skills. |

The browser suite covers rejected unauthenticated requests, spoofed/missing
origins, disabled host shortcuts, real signed registration/login, pairing and
assertion replay, tampered signatures, CSRF, additional credential enrollment,
process restart, copied-cookie revocation, and recovery followed by fresh login.
It also caught a same-page pairing-fragment update bug, now handled through
`hashchange`. The second credential uses a distinct virtual authenticator.

The initial route-sensor failure and browser-test failures were resolved; they
are recorded here rather than treating the first full run as entirely green.
Tests marked ignored for real networks/providers remain ignored. No model API
key, subscription sign-in, Tailscale login or public endpoint was used.

## Continue on another machine

1. Read root `AGENTS.md`, `SECURITY.md` and [REMOTE_ACCESS.md](REMOTE_ACCESS.md).
   Verify branch/remote state and preserve other contributors' changes.
2. Install the pinned Rust toolchain, Node 22+, Python 3 and Playwright Chromium
   dependencies. Run `make check` and `make remote-access-test`. A clean checkout
   builds the shipped Wasm skills; do not skip them unless artifacts exist.
3. Use a disposable external database key and temporary state for deployment
   tests. Never obtain or copy someone else's production database to validate a
   fresh installation.
4. The separate private continuity ledger points to this report. Project facts
   belong here; local paths and execution details belong in that machine's own
   ledger entry. No proprietary source or other projects' ledger content is
   needed to continue this repository.

## Remaining external acceptance

- Actual Tailscale account authorization, HTTPS/Funnel activation, tailnet ACLs,
  device expiry/re-authentication and mobile VPN compatibility.
- Physical phone over cellular data: private access disconnected/connected,
  Passkey availability on the selected platform, public access, and verified
  withdrawal of public reachability after switching modes.
- Real provider login/chat, streaming/reconnect, and host reboot with the
  deployment's secret injection. Process restart tests do not prove OS reboot.
- `/opt` systemd integration: the new mode helper manages the portable supervisor;
  an existing systemd installation needs explicit service wiring.

These limitations prevent claiming a fully certified one-click deployment.
Data-at-rest encryption does not protect live process memory, other files/logs,
old plaintext copies or the heirloom development custody seams. Legacy Google
MFA registration endpoints remain separate from the new `/access` Passkey flow.
