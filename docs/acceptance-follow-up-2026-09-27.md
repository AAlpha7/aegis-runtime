# External acceptance follow-up — 2026-09-27

Current cross-report tracking: [V1 decisions and acceptance checklist](V1_ACCEPTANCE_CHECKLIST.md).

This continues [release verification](verification-2026-09-27.md) and the
[first independent deployment trial](fresh-machine-trial-2026-09-27.md).
The repository remains private. Hosted CI and publication are explicitly excluded
by the owner; the workflow is manual-only and no workflow was dispatched.

## Confirmed on the upgraded deployment

The existing trial was upgraded to the managed installer on the private
`feat/durable-first-run` branch. Its encrypted state and original external key
were preserved, with private backups before upgrade. The managed Linux service
and current-user Windows session task replaced the legacy runtime service/task.
The separately managed embedding service was retained. The old user command now
delegates to the repository lifecycle entry, with its original file backed up.

- The new setup path validated the actual Codex subscription before saving.
- Real authenticated owner chat returned the expected probe marker.
- Tailscale account authorization and private Serve HTTPS activation succeeded.
  The actual HTTPS session endpoint advertised Passkeys and Google disabled.
  No public Funnel listener was requested or enabled.
- The owner reported successful iPhone Safari Passkey/Face ID sign-in and actual
  chat over cellular data with Tailscale connected. This is physical-device
  acceptance reported by the operator, distinct from virtual browser tests.
- The owner also confirmed that a new Safari private tab could not connect with
  Tailscale disconnected, then reported successful re-authentication and chat
  after the requested sign-out-all-devices flow with the VPN reconnected.
  Windows reboot, logout/login and sleep/resume are not yet verified. The later
  closed-agent host interval is recorded below.

Complete local `make check` passed again after the launcher change: core library
4,114 passed/4 ignored, executor 41 passed, main binary 35 passed, integration and
documentation tests, complete frontend tests, 11 Jev setup tests and 19 managed
installer tests. The separate privacy scan returned zero findings. These are
local results; ignored external-service cases remain unverified.
The updated Linux archive passed extracted-layout, shell syntax/line-ending and
executable-startup checks. It reused the already verified unchanged Rust and
WebApp artifacts, included the current installer scripts, and was not uploaded.

## Live DuckDuckGo acceptance: blocked

The signed `web_search` Wasm skill made a real public query without a model API
key. DuckDuckGo returned a CAPTCHA/bot-block page for the current IP; the skill
returned `status=error`, `error_kind=rate_limit` and no usable sources. This is
failed live search acceptance, not successful fallback. No CAPTCHA bypass or
automatic paid-provider switch was attempted.

The old ignored network test accepted an error/CAPTCHA or missing artifact as a
pass. It now requires the built skill and usable public source URLs using the
same result parser as the runtime. The corrected test failed on the observed
CAPTCHA, as it should. The complete local suite result above predates this
test-only tightening; the targeted live run is recorded separately. Simulated
behavior tests cover explicit fallback-attempt and fallback-failure notices,
but a real owner-chat fallback presentation still needs acceptance.

## Defects found during real acceptance

1. Python's cookie jar did not send the Secure owner cookie on the loopback HTTP
   installation probe. The exact configured loopback origin now has the same
   exception used by browsers. Two real local servers verify authentication at
   the intended origin and rejection of cookie delivery to the other port.
   Remote origins cannot use this exception. The real Codex probe then passed.
2. Remote configuration left managed status and Windows launchers pointing at
   the local console, whose local-login route is disabled in remote mode.
   They now select the configured HTTPS Passkey entry, while readiness checks
   remain local. Remote sign-in starts the service if necessary without a
   restart; the launcher explicitly mentions the private VPN requirement.
  Nineteen managed-installer tests passed, including this URL separation. The
  Windows test executes the actual launcher action bodies with synthetic WSL and
  browser handlers: remote Open/Login call start and HTTPS, while local Login
  still restarts and obtains a fresh one-time entry. Existing task ownership and
  path checks also passed. The real Windows task reports the correct HTTPS entry.

## Second independent deployment trial

A fresh Ubuntu 24.04 WSL distribution received only a clean committed source
clone and an independently installed Codex CLI. Windows mounts/interoperability
are disabled. No project dependency caches, application data, encryption key or
existing account credentials were copied. The user completed a new ChatGPT
device authorization. The initial agent prompt was only `请部署这个项目。`.

The observer reviews individual permission requests. An attempted automatic
approval mode was rejected before execution; the trial uses interactive,
request-by-request approval. No persistent root grant is provided.

Observed so far:

- Anonymous remote checking remains unavailable because the repository is private.
- The agent independently read the installation guide, detected missing build
  tools and distinguished sandbox denial from actual systemd availability.
- It requested a provider choice and bounded administrator dependency/lingering
  operations. The observer selected Codex and carried out the exact requested
  administrator commands, which completed successfully. This is explicitly
  assisted setup, not unaided novice
  success. No troubleshooting solution was supplied to the agent.
- Initial distribution provisioning hit a Windows WSL service crash; retry
  succeeded. This host event is separate from application lifecycle acceptance
  and is not a successful Windows reboot test.

The second agent independently installed Rust 1.98.1 and verified the official
Node 22 package checksum, then invoked the repository first-run installer. It
encountered an absent Ollama dependency and installed it from the official
distribution. The installer did not claim success or keep unvalidated provider
configuration after the initial failure. Because this host's WSL distributions
share ports, the agent created a separate persistent embedding unit on another
loopback port, downloaded the model and completed the release/frontend build.
The installer then detected no usable local login on the default console port.
The agent independently diagnosed another deployment occupying the default
ports and reran setup with its own ports. No troubleshooting instructions
were supplied. Real Codex owner chat returned HTTP 200 and the expected marker,
and the service remained healthy after its restart. The observer performed the
agent's exact final Windows session-task installation, which succeeded, then
closed the deployment CLI. The host interval passed after the assisted Windows
task recovery described below; power/session transitions remain pending. Full local regression ran
separately in the development checkout, not as an installation prerequisite in
the blind trial.

## Search and response defects from the iPhone trial

The owner's screenshots are a failed search acceptance: Aegis recorded no usable
Codex or DuckDuckGo sources, while the chat text claimed a separate successful
search. They also exposed an empty receipt inserted before the answer, repeated
disabled-Jev warnings, separated source titles/URLs, repeated numbering and raw
tool fields presented as source evidence.

- The external Codex adapter sent `readOnly` / `workspaceWrite`; the real CLI
  rejected those values, which must be `read-only` / `workspace-write`. The test
  transport now checks the actual wire values.
- A real read-only protocol probe with the existing subscription reproduced the
  rejected value, then observed a completed web-search item and official source
  URLs with the corrected value. This probe alone is not runtime acceptance.
- External final-answer text now replaces preceding commentary before structured
  search parsing. Foreground chat also waits for turn completion and does not
  interpret commentary as the final answer. Its native web/shell/app routes are
  explicitly disabled so Aegis tools own authorization and evidence recording.
- Disabled Jev is no longer announced on each ordinary public search. Local
  private-content and untrusted-source holds remain. An enabled but unavailable
  judge still emits a concise degradation notice.
- Empty receipts are hidden, successful sources follow the answer, reference
  titles link directly to their URLs, and reference lists are not split by the
  long-answer fold. Failed search has a plain explanation and collapsed
  diagnostics instead of duplicate evidence cards.

Focused Codex tests passed (55), as did response/rich-text rendering tests and
the production frontend build. The real browser presentation regression passed
at 390px/light and 320px/dark: answer position stays stable when the source
receipt arrives, titles remain linked, diagnostic details start folded, and
there is no horizontal overflow. Screenshots were inspected. These are desktop
Chromium mobile viewports, not a repeat of physical iPhone acceptance.

The subsequent isolated runtime probe passed through the actual owner-login and
chat-command endpoints using the existing authorized Codex subscription and a
fresh temporary encrypted state. Aegis's `web_search` evidence contained two
official Tailscale documentation URLs. The persisted source receipt recorded
`source=codex_subscription`, `fallback_used=false`, and no disabled-Jev notice
appeared. No account credentials or original owner data were copied. The first
runtime probe had exposed lost service provenance in the condensed source
receipt; this is now preserved and checked through both the model and owner
receipt paths. The successful source list and inspectable tool record share one
source panel. Failure rendering is tested with the actual server's formatted
text as well as JSON, not just synthetic raw JSON.

## Remaining acceptance

Real Grok, explicit API-key and Jev probes need suitable user accounts/configuration.
Successful live DuckDuckGo fallback, optional connected
account operations, host-assisted credential recovery and Windows power/session
transitions still require results. No pending item is marked passed
based on compilation, simulation or an existing process alone. Authentication
links, pairing secrets, database keys and raw account logs are excluded from this
report and the repository.

## Post-fix regression and deployment

The complete local `make check` passed: core 4,116/4 ignored, executor 41,
main 35, integration/doc tests, full frontend suite, Jev setup 11 and installer
21. The final source-provenance addition separately passed 54 search-focused
Rust tests and all-target Clippy. The final frontend passed 90 response-view
tests, rich-text tests and the mobile browser presentation checks. Eleven
remote helper tests, all 17 real WebAuthn browser journeys and four actual
supervisor/access-mode transitions passed. Privacy scan: zero findings.
The local release archive passed extracted-layout, LF script and executable
startup checks. Hosted CI was not run.

The live-tested binary and final frontend were deployed to the existing private
trial with rollback copies. The original key, encrypted databases and Passkeys
were retained. Read-back confirmed the running deployment's binary hash and
served frontend bundle match the tested artifacts. A fresh physical iPhone
presentation check after this upgrade is awaiting the owner.

The second blind trial's port conflict is now rejected during fresh preflight,
before the expensive build. An occupied listener is preserved; the installer
asks for explicit free ports instead of silently changing them. Tests use real
loopback sockets for occupied/free/invalid configurations. The ordinary setup
no longer describes managed state as portable or warns about disabled Heirloom.

The second trial's Windows session task later exited with a nonzero task result.
The host also showed intermittent WSL connection timeouts during this session;
that observation alone does not establish the cause. Starting the task again
restored a running task and Windows HTTP 200 responses for health and console.
The independent Codex CLI, its daemon/remaining idle trial shell and all build
and test commands were closed before starting the 30-minute host-only interval.
This recovery was assisted and is not evidence of automatic reboot recovery.

Both installations subsequently passed 1,806 seconds with 97 Windows-side
sample rounds. Every round checked the session task and both health/console
HTTP endpoints. No Linux command or WSL filesystem access occurred inside that
interval. Starting/ending boot identities were unchanged. Post-interval checks
showed both runtime processes predated the interval and had zero automatic
restarts; neither memory database had a plaintext SQLite header and both
external key files retained mode 0600. This is awake, logged-in Windows lifetime
acceptance after the recorded manual recovery, not reboot/logout/sleep or
unaided recovery acceptance. The separate independent deployment's source clone
was not given fixes or troubleshooting hints during its Linux installation.
