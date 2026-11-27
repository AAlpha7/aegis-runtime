# V1 follow-through verification — 2026-09-28

Source change: `bf3747f`. Scope: private `feat/durable-first-run` candidate; no publication or hosted CI.
The owner requested complete removal of DDG and completion of collected defects.
The [stable checklist](V1_ACCEPTANCE_CHECKLIST.md) tracks P01–P12, U01–U11,
D01–D10 and A01–A09. Earlier reports retain their original failures and limits.

## Changes and actual observations

- Removed the bundled DDG skill, workspace member, settings choice and automatic
  fallback. Upgrade loaders ignore a leftover `web_search.wasm`; archives exclude
  both legacy source and binary even if stale packaging staging contains them.
  Unknown/retired saved settings remain visible as needing explicit repair.
  API-key chat does not implicitly authorize paid search. Google/Bing scraping
  is not substituted.
- Subscription search stops with an explicit no-source/failure notice and offers
  retry/settings repair. A prose summary without usable public HTTP(S) citations
  cannot count as search evidence or be fed back as verified information.
- Real Grok acceptance initially found response prose but zero parsed citations.
  Added the documented Responses `url_citation` annotations (flat and nested),
  alongside top-level citations. The later actual owner search returned official
  Tailscale URLs in tool evidence with `xai_subscription_web_search` provenance
  and `fallback_used=false`. Links invented in prose are never promoted.
- Final Codex acceptance exposed a second source-presentation defect: the parser
  discarded per-source snippets and substituted the search summary on the first
  page. Snippets are now retained; provider synthesis stays separate and is never
  attributed as an excerpt from a particular page. A no-source response still
  cannot use synthesis as evidence.
- Disabled Jev is quiet in search, lifecycle, computer-use and non-CU paths.
  Enabled-but-unavailable judgments retain concise failure/hold notices.
  Opted-in Heirloom profile fallback and failed extra writes now disclose missing
  details or possible divergence; normal standard memory remains quiet.
- Cross-path disclosure review covered provider/model/executor change, explicit
  API billing, broader search, native-X to web, Jina including both-reader failure,
  omitted media/transcription, embedding degradation, and Heirloom read/write
  failure. Existing notices were retained; focused fault tests cover the changed
  paths. This is not a claim that every background job or live API was accepted.
- Real external execution exposed an incorrect “sent via Telegram” approval
  message on the web route, and raw protocol stderr/paths in the approval result.
  Pending wording is now surface-neutral. External approval results show service,
  completion state and answer; diagnostics remain in the structured receipt.
  Failed/error/timeout results cannot display a success icon.

## Account acceptance

Testing used isolated encrypted application state and an external private key;
original trial settings and data were preserved. No credentials, device codes,
login links, raw account logs or personal message content are in this report.

- Grok: official CLI login and application device authorization succeeded. Setup
  validation, owner chat/search, and owner-approved `run_external` with Grok ACP
  returned the requested harmless marker. Executor was `grok_acp`, model
  `grok-4.7`, stop `end_turn`, no fallback. This proves the tested account path,
  not all subscription entitlements.
- Connector discovery: actual Codex app-server `app/list` reported Gmail enabled
  and accessible. Discovery alone was not counted as a mail operation.
- With explicit owner approval limited to unread inbox count, Aegis routed the
  approved connector task to Codex and visibly explained that service choice.
  The completed receipt reported an exact message count using
  `list_labels(label_names=["INBOX"])` → `messagesUnread`, without pagination.
  The account-specific count is omitted here. No body reading, sending, marking,
  moving, deleting or attachment access was requested or authorized. This narrow
  receipt does not validate general mail management.
- The final `bf3747f` binary repeated both real subscription searches successfully.
  Codex returned distinct page excerpts instead of search-process narration; Grok
  returned official tool-evidence URLs. Both retained their own provenance, no
  fallback and no disabled-Jev warning. External execution above was exercised
  before the subsequent presentation-only receipt change.
- Real paid API and Jev Decisions acceptance remains unperformed: no compatible
  test credentials or paid scope were supplied. Jev remains off.

## Windows recovery

The earlier session-task failure and manual restart remain historical facts.
A new temporary real Scheduler probe with an increased retry setting still did
not recover its failed child within 110 seconds; changing Scheduler retry count
alone was not accepted as a fix.

Added an explicit bounded Windows supervisor that retries WSL hold exit/failure,
including unexpected clean exit, inside the current user's limited logon task.
A real temporary scheduled task using that supervisor recovered a deliberately
failed child and stayed running on its next attempt. The test used a short retry
interval; production uses 60 seconds and a bounded 999-attempt loop. Test task
and temporary files were removed. Ownership validation accepts only the exact
owned current/legacy action and preserves unrelated tasks.

Both live trial tasks migrated to the hidden supervisor. Their host-side health
and console HTTP checks passed. Linux service, keys and encrypted data were
preserved; the second independent trial's Linux source baseline was not replaced.
The previous 1,806-second awake/logged-in idle result predates this change and is
not a reboot or supervisor recovery result. Actual reboot, logout/login and
sleep/resume still require coordinated user timing. No surprise power transition
was performed, and the original WSL timeout cause remains unproven.

## Local verification

The broad `make check` run passed core 4,120 / 4 ignored, executor 41,
main 36 and the other integration/doc targets, but stopped overall because two
search integration fixtures still supplied prose without citations. The fixtures
were corrected to supply structured provider citations; the complete ReAct target
then passed 35 / 5 ignored. Structural guards passed 8 and the two remaining Wasm
skills loaded. An older guard requiring the retired artifact was removed/updated;
the stale local compiled search artifact was removed as well.

The rest of the check recipe was run explicitly: full frontend test command,
Jev setup 11 and installer 21 passed. Subsequent approval-display changes passed
48 HITL tests and all-target Clippy. Final snippet-preservation changes passed 71 search-focused tests, the complete
ReAct target (35 / 5 ignored), formatting and all-target Clippy. This is not one
green invocation over the final tree; overlapping focused counts must not be added.

Remote helper 11, WebAuthn browser journeys 17, and supervisor/access-mode
transitions 4 passed. Browser settings initially expected the retired provider to
save successfully; the corrected journey requires rejection and passed. The
320px/dark and 390px/light search presentation checks passed and screenshots were
visually inspected: stable answer position, linked reference titles, one source
panel, folded raw diagnostics and no horizontal overflow. These are Chromium
results with a virtual authenticator, not physical Safari/Face ID.

Windows behavior tests passed; the real supervisor fault experiment is described
above. Wasm target check passed. Final release archive layout, absence of retired search source/binary, LF scripts
and extracted runtime startup passed. Privacy scan reported zero
findings and `git diff --check` passed. Dependency future-compatibility and web
bundle-size warnings remain non-failing warnings, not claimed fixes.

## Private deployment

The original private trial clone fast-forwarded to `bf3747f`; the service was
restarted with the tested binary and frontend. Host health and console returned
HTTP 200, the served index matched the built file, and the retired compiled search
artifact was removed from the active directory. Existing encrypted databases,
external key and Passkeys were preserved with a local rollback copy.

- Runtime SHA-256: `d0b4c9d5a8e6438856b581718b897049ee7cfa8980e38d0e721502c7a18c44ef`
- Frontend index SHA-256: `08a49c2f184c4567f9d11f9d0e325887adcf0027b36ce0703da9fcece0c10e71`

The owner subsequently reviewed the updated phone interface and reported that
sources and related presentation appear fixed. This is actual owner feedback,
not inferred from HTTP checks or a claim of exhaustive Safari regression coverage.
At that snapshot the header sparkle button was still a simplification proposal.
The owner subsequently approved its removal; the follow-up below records the
implementation. Old persisted messages are not rewritten by an upgrade.
Historical receipts may still identify their actual former DDG source.

## Approved header and default portrait follow-up

Removed the duplicate global header chat-sheet opener. The two bottom tabs and
item-scoped questions remain. Added the requested warm, elegant, futuristic
humanoid secretary default portrait, served locally as a 233 KiB JPEG. Both
legacy built-in paths migrate to it; custom uploaded/generated portraits remain
unchanged. [The asset and complete generation prompt](AVATAR.md) are recorded.

Avatar-focused Rust tests passed 10; full frontend tests and production build
passed. Remote-access browser verification passed 18 journeys, including new
320px/light and 390px/dark checks for portrait loading, one composer, draft
preservation, absence of the redundant opener, no horizontal overflow and no
stale scroll lock after repeated navigation. Screenshots were visually inspected
for the circular crop and header layout. Remote helper tests passed 11 and all
four access lifecycle transitions passed. These are Chromium/virtual-authenticator
results, not new physical iPhone acceptance.

The rebuilt release archive passed extracted layout, LF shell script and startup
checks. Formatting and privacy audit passed (zero tracked-file findings, still
requiring manual review). One complete `make check` invocation passed on the
final source at `faab7bf`: core 4,123 passed / 4 ignored, executor 41, main 36,
all included integration/doc targets, full frontend suite, Jev setup 11 and
installer 21. Unlike the earlier snapshot above, this is a single green full
gate over the final implementation, not a sum of overlapping focused reruns.

The primary private trial clone fast-forwarded to `faab7bf` and received the
tested runtime/frontend with rollback copies. Original encrypted state, external
key and Passkeys were retained; the independent second trial's Linux source was
not replaced. Windows health returned HTTP 200; the served index and portrait
matched the build. Both Windows session tasks remained running. Serve still
proxies the loopback console in tailnet-only mode. Ordinary Tailnet DNS lookup
failed on the test host/node; HTTPS with the reported node IP explicitly resolved
to the same hostname succeeded with normal certificate verification and returned
the exact portrait hash. This checks Serve and TLS, not phone DNS or a new owner
chat. No fresh physical-phone acceptance is claimed for this visual update.

- Runtime SHA-256: `391dae536287f88f0eb36f8cadf2f909ccf4682ebb240b43e5da5b51bd0e53b1`
- Frontend index SHA-256: `7a338ab947fb554fad05b052ecf7cd7a44d1f8c38c5c83ac8d66c8cf2a89521d`
- Portrait SHA-256: `50e9f2e6be4ad042f2579de000c56a6346ed262aba5a16f23220f5f581292c89`

## DNS diagnosis and recovery preparation follow-up

The earlier ordinary-DNS failure was reproduced and diagnosed on the primary
trial node. `tailscale dns query` returned the correct node address while the
system resolver failed; `tailscale dns status` reported DNS acceptance disabled.
WSL's generated resolver pointed to the Windows DNS tunnel. Enabled only this
node's `accept-dns` preference, after confirming the supplied tailnet DNS setup.
No Serve/Funnel routes, account, key or application data were changed.

Afterward, ordinary system lookup resolved the private hostname, HTTPS fetched
the exact recorded portrait hash without `--resolve` or a certificate bypass,
and a public-domain lookup also succeeded. The service remained active and Serve
remained tailnet-only. This repairs the node-side condition; it does not configure
Windows-host DNS or prove survival of a WSL/Windows reboot. A08 must repeat it.
The Tailscale internal-query/system-query distinction is now documented for setup.

The supported access helper read the existing credential status successfully
using the managed external key without exposing it. No real credentials were
revoked. The owner has been asked for the updated phone baseline and coordinated
recovery timing; those results remain pending. A [recovery runbook](REAL_RECOVERY_ACCEPTANCE.md)
now specifies real old-credential rejection and Windows-only observations before
any WSL wake-up probes. Preparing that procedure does not close A07 or A08.

README positioning now follows the approved personal-secretary scope, corrects
the implication that every tool runs in Wasm, and records signature limits and
private-candidate status. The security document's stale disabled-Jev warning was
aligned with the implemented quiet-Off behavior. These are documentation and
deployment-configuration changes; runtime/frontend source and tested artifacts
are unchanged from `faab7bf`. No new full contributor run is claimed or needed
for this documentation-only follow-up. The tracked-file privacy audit returned
zero findings and `git diff --check` passed; documents were reviewed manually.

## Owner phone baseline after the update

The owner supplied a real iPhone Safari screenshot showing the new humanoid
portrait in the header and assistant messages, plus a fresh greeting and a
successful assistant reply. This closes the pending updated-avatar/display and
basic-chat baseline in U08. The screenshot shows Wi-Fi, so it is not a new
cellular-network result. It does not demonstrate fresh Passkey authentication,
old-credential rejection, recovery, or restart survival. No credentials were
revoked and no computer power transition was performed for this check. The
personal screenshot and conversation history were not added to the repository.

## Actual iPhone Passkey recovery acceptance

After the owner expressly agreed to proceed, the existing managed host helper
ran recovery once against the original encrypted database and HTTPS origin.
Host status changed from one registered Passkey to zero. The owner attempted
the old sign-in and supplied a screenshot plus confirmation that login failed.
The page's generic network/retry wording is misleading for this expected
rejection; U09 records that presentation follow-up without asserting a more
specific server failure reason than the evidence supports.

A new single-use pairing link was then issued using `pair`, not another reset.
The owner was instructed to enroll and sign in with Face ID, turn off Wi-Fi,
retain Tailscale, and send a recovery-test message. They confirmed success.
The subsequent host recheck found one registered Passkey, an active/ready
application service and tailnet-only Serve. This accepts A07 for the tested
iPhone: old-credential rejection and restored login/chat. The cellular result
is owner-reported; it is not inferred from host health or the earlier Wi-Fi image.

The original database, external key and application data were retained. No
computer reboot, sign-out or sleep was performed. Pairing links, key material,
credential IDs, screenshots and raw account logs are not stored in this report.

## U09 login-failure guidance follow-up

Source `34febf8` distinguishes a failed transport request from a rejected
Passkey request. The latter conditionally explains how to pair again after a
host reset without asserting that reset caused every rejection. Both English
and Chinese messages preserve retry; raw server diagnostics remain hidden.

All 19 real-router/WebAuthn browser journeys passed using temporary encrypted
state and a virtual authenticator. Added assertions cover transport failure and
old-credential rejection after host recovery, language switching, enabled retry
controls and 320px layout. Both recovery screenshots were inspected. An initial
test attempt left the test page in Chinese; the test now restores English before
continuing. A subsequent attempt timed out entering the workspace despite a
valid session; the complete rerun with fresh frontend assets passed. The cause
of that timeout is not established; failure screenshots are now retained locally
when requested. No real phone reset was repeated for this automated check.

The 11 remote-access helper tests, all four encrypted access-mode lifecycle
transitions, release packaging and extracted-package startup check also passed.
The complete `make check` passed on this source: core 4,123 (4 ignored), executor
41, main 36, remaining integration/doc targets, all-target Clippy, full frontend
suite, Jev setup 11 and installer 21. Formatting passed. Existing compiled Wasm
artifacts were reused (`SKIP_SKILL_BUILD=1`); this was not a fresh Wasm rebuild.
Tracked-file privacy scanning found zero findings and diff whitespace checks
passed; this supplements manual review.

The primary trial was upgraded with rollback artifacts, retaining the existing
database/key and one registered Passkey. Service readiness passed. Deployed
runtime SHA-256 is `0a6a0c5baedf581a1b7af833cd3d531b186c4a608055e4878c0d9b8cad5dae29`.
The access script served by loopback and private HTTPS matches source SHA-256
`eae8b08ff4a547427136e62711ca0de25f1192ba43972cf024c406453f9e4ed6`.
The served frontend index matches `e570df76f6ed9a34fa62bc5248563d74594fc5f54affeb8f451ab2f22f16ce41`.
No real credential reset or fresh phone chat was performed for this deployment.

The first ordinary private HTTPS probe failed DNS resolution again. Tailscale
DNS was enabled, but WSL had recreated `/etc/resolv.conf` pointing to its host
resolver; Tailscale logs confirmed that the resolver configuration was overwritten.
Internal Tailscale lookup still resolved the node. Loopback and an explicit-IP
HTTPS diagnostic matched the script; those did not count as normal DNS success.
Reapplying the existing preference alone did not fix the immediate check.
After reloading `tailscaled` and allowing configuration to settle, ordinary
private/public DNS and normal HTTPS both passed with the hashes above. The key
mode remained 0600 and Serve remained tailnet-only. A proposed resolver-link
change stopped at its failed prerequisite, so neither that link nor `/etc/wsl.conf`
was manually changed. No upstream DNS provider was substituted.

This is a recovered recurrence, not proven durable DNS repair. A08 must observe
actual sleep/login/reboot recovery and DNS settling before any manual Linux
intervention. Diagnostic references: [Tailscale Linux DNS](https://tailscale.com/docs/reference/linux-dns)
and [WSL network configuration](https://learn.microsoft.com/en-us/windows/wsl/wsl-config).

## Actual Windows Modern Standby and phone recovery (A08)

The owner followed the sleep/wake procedure and reported that phone chat
succeeded. Windows Kernel-Power events 506/507 independently record Modern
Standby entry at 04:17:31 UTC and exit at 04:22:45 UTC on September 29 (the
evening of September 28 locally), roughly five minutes apart. This is the
observed sleep mode; it is not evidence for S3 sleep or hibernation.

A prestarted Windows-only observer queried Task Scheduler and the loopback
health/console endpoints. Its first post-wake round was timestamped 04:22:52 UTC,
about seven seconds after the exit event, and recorded Running/200/200. This
is a sample time, not an exact recovery-latency measurement. One round during
standby entry could not read task state while HTTP still returned 200; the next
round followed a roughly 318-second observation gap. No availability during
sleep is claimed. Further Windows-only reads confirmed the running task and both
200 responses before any new Linux probe, manual service start or DNS repair.

The subsequent Linux checks passed ordinary private-hostname and public-domain
resolution on the first attempt. Normal HTTPS returned the expected U09 access
script hash. The service remained active with zero automatic restarts, the same
pre-sleep runtime start time and unchanged WSL boot identity. The existing
Passkey count remained one, key permissions 0600, and Serve tailnet-only. No
daemon reload, DNS change, pairing or application restart was needed in this
follow-up. The temporary Windows observer was stopped after evidence collection.

This accepts primary-trial Modern Standby/wake plus the owner's phone-chat
result under the requested cellular/Tailscale procedure. Device transport and
chat are owner-reported; host probes do not establish them independently. An
existing session may have been reused, so no fresh Face ID challenge is claimed.
This result does not prove cold key reload, logout/login or reboot persistence,
or erase the earlier DNS recurrence. The independent second trial was not part
of this physical-device acceptance. No runtime source changed in this follow-up;
the prior full source checks remain the applicable results.

## Actual Windows reboot and cold service recovery (A08)

After the owner agreed to restart, a temporary limited-user Windows logon task
recorded only the existing Aegis task state and host HTTP endpoints. It did not
start WSL, Aegis or any service. The owner restarted, signed in and confirmed
phone chat success under the requested cellular/Tailscale procedure.

Windows boot time changed to 04:49:18 UTC on September 29. The first observer
round at 04:49:57 had the task Running, no health response and console HTTP 200;
the next at 04:50:05 had Running/200/200. These are observation timestamps, not
exact startup latency. Before any follow-up WSL invocation, Windows-only checks
again confirmed task and HTTP readiness. No manual task/service start occurred.

Read-only Linux checks then found a new WSL boot identity and a new runtime
start time, active service, zero service restarts, key file mode 0600 and the
original one registered Passkey. Existing encrypted state loaded through the
installed key injection without reconfiguration or pairing. Ordinary private
and public DNS passed on the first follow-up, as did private HTTPS with the
expected U09 script hash. Serve remained tailnet-only; no DNS repair was used.
The temporary observer task was stopped and unregistered after collection.

This accepts the primary trial's Windows restart/sign-in, cold service/key
recovery and owner-reported phone chat. It does not independently certify phone
transport, a fresh Face ID challenge, or a separate Windows logout/login cycle.
The historical DNS recurrence remains recorded and is not erased by this pass.

## Disabled Hercules and bilingual live progress (U10/U11)

Source `7b9bf2d` follows `f8b02c8`. The default runtime already refused to create
a Hercules client without explicit opt-in and credentials, but the owner-work
projection unconditionally advertised an available Hercules source. Settings
then displayed that source. The projection now requires an actual configured
client; stale delegation history is retained but does not advertise an inactive
connection. The default external-executor prompt no longer describes an unused
Hercules integration. Existing optional, explicitly configured behavior remains.
Read-only inspection of the primary runtime's process configuration found its
delegation flag off and API base/token absent; no credential values were printed.

Frontend tool, phase and completion copy was hardcoded in Chinese even when the
rest of the UI used English. These helpers and executor footers now receive the
turn's display language in main chat, follow-up and the item panel. Known model
fallback and executor protocol notices are localized; unknown owner notices are
preserved so cost, authorization and failure disclosures are not silently lost.
Opaque phase details use the structured phase's localized label; approval detail
remains on its card. Generic approval/computer-use progress no longer claims
Jev ran. Unknown executor statuses remain neutral rather than implying success
or continued execution. Actual model answers and user text are not translated.

Focused owner-work tests passed (10), including default-off with stale history
and explicitly configured delegation. Tool-phrase tests (5) and working-progress
tests (7) passed. Twenty real-router/WebAuthn browser journeys passed against the
built app, including English/Chinese settings without Hercules and rendered live
progress fed deterministic SSE frames. Those frames are a UI test, not a live
provider run. Eleven access helper tests, four encrypted lifecycle transitions,
frontend build and extracted-package startup also passed. The complete
`make check` passed: core 4,124 (4 ignored), executor 41, main 36, included
integration/doc targets, Clippy, full frontend suite, Jev setup 11 and installer
21. Existing compiled Wasm artifacts were reused; no fresh Wasm build is claimed.
The final neutral-status edge case was included in the frontend suite, rebuilt
package and repeated 20-journey browser run.

The primary trial now runs the corrected artifacts. An initial deployment command
found that the earlier reboot had removed its temporary upgrade script; it did
not stop or change the service. Retransferring the same rollback-capable helper
completed the upgrade. Runtime SHA-256 matches
`538e019f3aef56a6c2e58d96b54fbfadd37bd91cb161ecd30ea223de19d58442`;
normal private HTTPS returns the frontend index matching
`fb6bcbf6e3028f2cf5301035eae2c5bde80dbf58cb05b8b4b3e3426f76ad8f80`.
Service readiness, tailnet-only Serve, original one Passkey and key mode 0600
were retained. A fresh process check again found Hercules disabled with no
configured API base/token. No configuration, credentials or history were reset.
These deployed-artifact checks do not claim a new physical-phone UI review or
provider chat after this update. Privacy scanning found zero tracked-file
findings and whitespace checks passed; no raw logs or secrets were committed.

## Separate Windows logout/login acceptance (A08)

The owner confirmed successful phone chat after the coordinated Windows logout,
wait and sign-in instructions, without manually launching WSL or Aegis. Windows
session events recorded logout at 2026-09-29 05:49:49 UTC and login at 05:56:41
UTC. The Windows boot identity stayed unchanged while the interactive session
changed, establishing a separate logout/login transition rather than a reboot.

A temporary Windows-only observer automatically started at sign-in. It only
queried the existing task and HTTP endpoints; it did not invoke WSL or start
services. Samples at 05:56:44 and 05:56:55 recorded the supervisor Running,
console HTTP 200 and health unavailable. The first fully ready sample at
05:57:10 recorded Running/200/200, about 29 seconds after the login event;
this is sampled readiness, not an exact startup-duration measurement. Subsequent
samples remained ready. These observations preceded all Linux follow-up probes.

The first read-only WSL follow-up found a new WSL boot identity and an active
runtime started at 05:56:48 UTC with zero systemd restarts. Ordinary private and
public DNS resolved, and normal private HTTPS returned the deployed frontend
index matching `fb6bcbf6e3028f2cf5301035eae2c5bde80dbf58cb05b8b4b3e3426f76ad8f80`.
The existing encrypted state remained readable with the installed key helper,
one Passkey remained registered, key permissions were 0600, and Serve remained
tailnet-only. No DNS repair, service restart, setup or credential reset was
performed. The temporary observation task was removed; the installed Aegis
session supervisor remained Running.

This closes the primary trial's separate logout/login acceptance alongside its
earlier Modern Standby and reboot checks. Phone chat is owner-reported; this does
not establish availability while Windows is logged out, a fresh Face ID challenge,
S3/hibernate support or physical lifecycle acceptance for the independent second
trial. The earlier DNS recurrence remains documented. This update only records
acceptance evidence; the source validation counts above are unchanged.

## Remaining verification

- Paid API and optional Jev live validation with separately authorized credentials.
- Anonymous clone/install remains unavailable while the repository is private.
  The two assisted trials are not represented as fully unaided novice success.

Realtime voice remains deferred; Heirloom and Google login remain off by default.
The repository and PR remain private/draft. Publication and hosted CI are excluded.
