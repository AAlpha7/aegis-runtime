# V1 decisions, fixes and acceptance checklist

Updated: 2026-09-30. This is the current tracking entry for the owner's requests,
including the deployment trials and iPhone search screenshots. Historical reports
retain their original results; this checklist does not retroactively turn assisted
or simulated results into independent acceptance.

The v0.1.0 preview candidate remains private. Publication requires explicit owner
authorization; preparation of a tag or draft does not make it a public release.
The latest follow-through is recorded in
[clean verification and installation](verification-2026-09-30-clean-install.md),
including an unexplained initial Heirloom status-check failure and outstanding
real installation acceptance. The old draft is superseded. Earlier evidence is
in [publication preparation](verification-2026-09-30-release.md),
[DeepSeek follow-through](verification-2026-09-29-deepseek.md) and
[2026-09-29 verification](verification-2026-09-29.md); earlier reports remain
historical snapshots.

Status terms: **implemented** means code and local checks exist; **accepted**
means the specified real scenario was exercised; **pending** needs further work,
evidence or a decision; **failed** preserves an observed failure; **deferred** and
**excluded** are explicit scope choices. Implementation alone does not close
external acceptance. Close an item only with a dated result and its limitations.

## Product decisions and behavior

| ID | Requirement | Current status and evidence | Remaining work / closure condition |
| --- | --- | --- | --- |
| P01 | Prefer Grok/Codex subscriptions for chat, external execution and search; paid API only by explicit choice. | Implemented; real Codex chat/search and Grok setup/chat/search/external execution accepted. [Latest evidence](verification-2026-09-28.md). | Explicit API path remains A04. Never silently switch to paid calls. |
| P02 | Connected-account/mail work uses an external executor with real capability and authorization. | Actual Codex connector discovery and owner-approved Gmail unread-inbox count completed through Aegis approval and external execution. | A06 is accepted only for this read-only count; no sending, deletion or broader mail-management acceptance. |
| P03 | Owner explicitly requests complete DDG removal. | Removed skill source, workspace/package inclusion, configuration choice and automatic fallback. Leftover compiled skill is ignored. Old settings require explicit repair; no silent substitution. | Closed by implementation and rejection/no-fallback tests. Historical CAPTCHA failure remains A02; Google/Bing scraping is not substituted. |
| P04 | Disclose actual fallbacks and hidden behavior changes. | Cross-path review completed for the named provider/model/executor, paid billing, broader query, native-X, Jina, media, embedding and Heirloom paths. Fixed routine Jev noise, Heirloom failure notices and live fallback billing copy. [Latest evidence](verification-2026-09-29.md). | A09 records the reviewed scope. Controlled real API/Jev fault results are A04/A05; these do not prove every natural outage. |
| P05 | Jev is optional and off by default; API-key onboarding may offer it. | Implemented. Explicit consent and compatible credential required; unattended setup cannot consent. [Subscription contract](SUBSCRIPTIONS.md). | Real opt-in and recovery: A05. Normal configured Off must remain quiet (U02). |
| P06 | Integrate Jev cleanly into Settings. | Implemented: bilingual card, disclosure/billing consent, encrypted write-only key, Save/Cancel, recheck and removal. Browser behavior checks in [verification](verification-2026-09-27.md). | Keep real API validation separate from synthetic settings tests. |
| P07 | Validate each call configuration before saving. | Implemented for chat provider/model, Codex model/reasoning, search provider/model, backup model, paid fallback and Jev. A failed probe preserves previous values; another provider cannot validate a broken candidate. | A03–A05 for real credentials. Disabling/removing an optional service does not need a network probe. Environment values alone are not verified settings. |
| P08 | Heirloom stays experimental/off for v1. | Implemented; [Heirloom policy](HEIRLOOM.md). Disabled path avoids initialization and preserves old data. | Optional subsystem is outside stable v1 guarantees; keep standard encrypted memory and key checks mandatory. |
| P09 | Google login defaults off for nontechnical setup. | Implemented; configured sign-in methods are advertised accurately. Real private session endpoint showed Google disabled. | Preserve explicit opt-in on upgrades; do not require a Google project for normal installation. |
| P10 | Realtime voice belongs to the next version. | **Deferred explicitly.** Configured voice-note transcription is distinct. | Do not re-enable realtime voice or count it as an unfinished v1 blocker. |
| P11 | Keep repository private; do not run hosted CI. | **Excluded explicitly:** publication and hosted CI. Workflow remains manual-only; no workflow dispatched. | Any future publication needs fresh authorization, privacy/manual review and release evidence review. Anonymous public cloning is unavailable while private. |
| P12 | Position Anqi as a personal secretary: persistent follow-through, appropriate authorization and checkable results. | README now describes the actual single-owner release and separates Wasm skills from host/external execution. Signatures are not represented as proof that an action is safe. | Native installation packaging and broader product polish remain follow-up work, not delivered features. |

## User-visible defects from the iPhone trial

U01–U06 have implemented fixes; deployment evidence is in the latest report.
The owner reviewed the updated phone interface and reported that sources and
related presentation appear fixed (A01). This is user-reported feedback, not a
claim of exhaustive Safari regression coverage. Old messages are not rewritten.
U07 and U08 follow the owner's subsequent approval and avatar request.

| ID | Reported problem | Implemented fix and verification |
| --- | --- | --- |
| U01 | A reply appeared partly, then changed into a different/empty receipt presentation. | Distinguish commentary from final answer; wait for turn completion; hide empty receipts; keep answer before evidence. Browser regression checks stable answer position when receipts arrive. |
| U02 | Long repeated warnings about deliberately disabled Jev. | Public search, lifecycle setup, computer-use and non-CU decisions are quiet when Jev is Off. Only an enabled but unavailable judge gets a concise degradation notice. Local sensitive-content/untrusted-source holds remain; no Jev verdict is invented. |
| U03 | Reference title and URL separated, numbering reset, details fold inserted between them. | Title links directly to HTTP(S) URL; ordered numbering continues; reference lists are not split by long-answer folding. Mobile viewport checks passed. |
| U04 | Confusing sources, raw English fallback fields and duplicate error cards. | One source panel; plain failure explanation; actual service provenance; technical diagnostics folded; duplicate generic error evidence removed. Actual formatted server output and JSON are both tested. |
| U05 | Recorded search failed while prose claimed another successful search. | Fixed Codex sandbox wire enums; parse final search response correctly; disable unrecorded native chat search/shell/app routes so Aegis tools own authorization/evidence. Preserve source and fallback status through model and owner receipts. Real runtime search returned official URLs with Codex and Grok subscriptions. Grok Responses annotations are parsed; prose without verifiable URLs cannot become evidence. |
| U06 | Secure-access page looked crude and inconsistent. | Shared theme tokens, responsive layout, light/dark colors, bilingual labels and retry/pairing/recovery states. Browser screenshots reviewed; earlier real phone login accepted. |
| U07 | Owner questions the header sparkle button’s value. | Removed the duplicate global chat-sheet opener after owner approval. The bottom chat tab and item-scoped questions remain. Browser checks at 320px/light and 390px/dark passed: one composer, draft preserved across navigation, no stale sheet backdrop or scroll lock. |
| U08 | Give Anqi a warm, elegant, futuristic humanoid secretary default avatar. | Added a locally served fictional portrait. Both old built-in avatar paths map to it; custom portraits remain unchanged. Avatar validation/migration tests and mobile image-load/crop checks passed. [Asset and generation prompt](AVATAR.md). Owner supplied an actual iPhone screenshot showing the new portrait and a fresh successful greeting exchange. This visual/chat baseline passed on Wi-Fi; no new cellular or credential-recovery result is claimed. |
| U09 | During real recovery, rejected old credentials suggest checking the network. | **Fixed, verified and deployed.** Authentication rejection now offers conditional host-reset/new-pairing guidance; a failed network request separately suggests checking connectivity/Tailscale. All 19 browser journeys and the full contributor gate passed. Both messages were checked in English/Chinese, including retry/recovery and inspected 320px screenshots. Deployed HTTPS script matches; the existing phone credential remains registered. A07 remains accepted; no new physical rejection test is claimed. |
| U10 | Hercules appears even though v1 does not connect it by default. | Source fix gates Work/source projection on the actual configured delegation client and omits disabled-integration instructions from the default model prompt. Existing history is preserved. Live trial process inspection found the flag off and no API base/token; these values were not exposed. Default-off and configured projection tests pass. Browser checks confirm absence from settings in both languages. Full gate passed; deployed runtime and private HTTPS frontend hashes match. |
| U11 | Reply progress mixes Chinese and English. | Tool/phase/done text, executor footers and known protocol notices now use the turn's display language in main chat, follow-up and item panel. Generic approval and computer use no longer imply Jev ran. Unknown notices retain their disclosure; unknown executor status does not invent success or continued execution. Bilingual helper tests and actual app rendering with deterministic SSE passed; full gate passed and the primary private trial is updated. |

Evidence: [external follow-up](acceptance-follow-up-2026-09-27.md),
[earlier UI/settings checks](verification-2026-09-27.md), and
[`test-search-presentation.mjs`](../webapp/scripts/test-search-presentation.mjs).
390px/light and 320px/dark Chromium results are not physical Safari results.

## Installation and blind-trial follow-through

The original [trial report](fresh-machine-trial-2026-09-27.md) retains every
observation F01–F16. The [second trial](acceptance-follow-up-2026-09-27.md#second-independent-deployment-trial)
used a fresh clone, ordinary agent and only the initial prompt
`请部署这个项目。`. Account login, provider selection and exact bounded administrator
operations were assisted. No troubleshooting solution was supplied during the
second Linux installation. Neither trial is claimed as fully unaided novice success.

| ID | Observations / requirement | Current follow-through and boundary |
| --- | --- | --- |
| D01 | F01: fresh environment cannot anonymously fetch a private repository. | Retained trial limitation; public clone acceptance is excluded until publication is authorized. |
| D02 | F02–F04, F06–F07: location/provider/key/dependencies and permission prompts confused or interrupted initial setup. | Guided first-run, preflight, documented bounded admin steps and automatic external key creation implemented. Existing state with missing key stops safely. Real second trial still needed ordinary account/admin interaction. |
| D03 | F05, F09, F12: passing tests/builds was confused with installation acceptance; contributor checks added substantial install time/dependencies. | Ordinary installation now requires focused preflight, managed setup and real owner chat. Full contributor checks remain a source/release gate; browser test dependencies are not ordinary install prerequisites. |
| D04 | F08, F10–F14: temporary wrappers/session-held services stopped after closing the agent. | Supported systemd user installation with persistent tool/key paths and embedding lifecycle. Real service harness verifies exit survival, crash recovery, wrong-key refusal and preservation on removal. Real second-trial owner chat and restart passed. |
| D05 | F15–F16: WSL stopped while idle; Linux probes could wake it and conceal the problem. | Current-user limited Windows session task and launchers implemented. Both trials passed the later 1,806-second/97-round Windows-only interval after agent/daemon/shell exit. No Linux/WSL filesystem probes ran during it. See assisted recovery caveat below. |
| D06 | Second trial: missing Ollama/extraction dependency and occupied embedding/application ports. | Agent independently resolved dependency/embedding setup and selected distinct ports. Fresh application preflight now rejects occupied, invalid or duplicate ports before building; existing listeners are preserved and port changes are explicit. Real-socket tests passed. Do not imply every dependency is automatically installed. |
| D07 | Local probe Secure-cookie handling and remote launchers used the wrong access path. | Loopback cookie exception scoped to exact origin and tested across two servers; remote launcher uses private HTTPS and starts without restarting. Local login visibly confirms its restart. Real owner probe and launcher checks passed. |
| D08 | Upgrades/removal must preserve encrypted user state and keys. | Private deployment retained original SQLCipher data, key and Passkeys with rollback copies. Scoped removal/key edge cases tested; independent embedding retained. Managed state is no longer called portable. |
| D09 | Windows session task exited nonzero during the second trial. | Original failure retained. A later real Scheduler retry probe also failed to recover within 110 seconds. Added an explicit bounded Windows supervisor; real failed-child recovery passed. Both existing session tasks migrated and HTTP readiness passed. Primary-trial Modern Standby/wake and owner phone chat subsequently passed under A08. | Root cause of original WSL timeouts is unproven. Actual reboot and a separate logout/login subsequently passed under A08. |
| D10 | Private HTTPS previously needed explicit node-IP resolution on the test host/node. | Enabling Tailscale DNS initially restored ordinary private HTTPS and public DNS. During U09 deployment verification, WSL regenerated its resolver link despite `accept-dns=true`; Tailscale logged that its configuration was overwritten. Reloading Tailscale restored ordinary DNS/HTTPS; the new script and page hashes matched. No WSL boot configuration or upstream DNS provider was changed. After actual Modern Standby/wake, ordinary private/public DNS and HTTPS passed on the first follow-up without repair. A subsequent full reboot also passed ordinary DNS/HTTPS without repair. A separate logout/login also passed ordinary DNS/HTTPS on its first follow-up without repair under A08; this does not erase the earlier recurrence. Serve remains tailnet-only. |

## Outstanding real acceptance and required assistance

Use [the real recovery runbook](REAL_RECOVERY_ACCEPTANCE.md) for coordinated A07/A08.
A07 has actual phone evidence below; A08 has separate Modern Standby/wake,
reboot/sign-in and logout/login evidence for the primary trial.
Use [the API/Jev acceptance runbook](REAL_API_ACCEPTANCE.md) for A04/A05.
The owner authorized both with a USD 5 total ceiling. Host recovery, real search
and Jev results, and the late live-billing-disclosure correction are recorded in
the latest report. Final artifacts, real retests, deployment and temporary-key
cleanup are complete. The low-cost backup's unnecessary tool use remains a
separate failed quality observation; no further paid calls are scheduled.

| ID | Status | Next verification and evidence required | User assistance |
| --- | --- | --- | --- |
| A01 | **Owner review received** | Owner reviewed the updated interface and said sources and related presentation appear fixed. No new search-display defect was reported. This does not separately prove every intermittent stream transition; retain regressions and watch for recurrence. | No repeat of the same source-display check requested. |
| A02 | **Failed historical / retired** | DDG returned CAPTCHA/no usable sources. Owner requested complete removal; runtime and package no longer offer or invoke it. Failure/no-result paths now stop clearly without service substitution. | No DDG acceptance remains required. |
| A03 | **Accepted for tested account** | Official Grok CLI login, isolated encrypted application setup validation, actual owner chat/search with official source URLs, and owner-approved Grok ACP execution all passed without fallback. | No further login currently required. Provider availability remains account-dependent. |
| A04 | **Configuration/search/disclosure accepted; backup quality gap retained** | Real API bootstrap/chat/save and rejected-save preservation passed. Final-binary search returned real IANA sources and matching provider evidence, including the repeated-query retest. Controlled primary failure used the real API backup; captured live and final notices now match and disclose separate billing. Full gate, archive checks, private backend deployment and temporary-credential cleanup completed. | The backup unnecessarily read a public page before answering a greeting; its no-unnecessary-tools check failed. Do not claim reliable tool selection or a natural subscription outage. Total observed API/Jev usage: USD 0.435182208, subject to billing lag. |
| A05 | **Accepted within tested scope; cleanup complete** | Real Decisions probe, enabled/verified save/restart and rejected replacement preservation passed. Final-binary real search is quiet with Jev Off and with healthy enabled Jev; an unsupported-model fault produces a notice, and recovery restores quiet verified search. Offline disable/removal succeeds and saved Off overrides environment opt-in. | Temporary test key copies and isolated state removed; provider-side key revocation not claimed. The first repeated search without tool evidence remains an unsuccessful historical attempt; the source fix and final real repeated-query retest are recorded separately. |
| A06 | **Accepted within scope** | Actual account discovery found Gmail accessible. Owner expressly authorized only inbox unread-message count. Aegis approval invoked Codex and returned a count via the reported INBOX messagesUnread field. | No message bodies, sends, edits or deletions authorized. Broader connector operations remain untested. |
| A07 | **Accepted for tested iPhone** | Owner explicitly authorized recovery. Host credential count changed from one to zero; owner reported and showed failed old-Passkey login. A fresh pairing link was issued without another reset. Owner then confirmed success after instructions to enroll/sign in with Face ID and send a chat on cellular with Tailscale. Host recheck found one registered credential, active service and tailnet-only Serve. | No further credential reset needed. Cellular/login/chat are owner-reported; host status corroborates registration, not device transport. U09 tracks the rejection wording. A08 remains separate. |
| A08 | **Sleep/wake, reboot and separate logout/login accepted for primary trial** | Primary-trial Modern Standby and subsequent full Windows restart both have Windows-only observation before Linux probes plus owner-reported cellular/Tailscale phone chat. Restart changed Windows/WSL boot identities and runtime start time; the installed service loaded its existing encrypted state/key without setup. The first reboot sample had health unavailable; the next recorded Running/200/200. Ordinary private/public DNS and HTTPS then passed without intervention, original Passkey retained and key mode 0600. | A separate logout/login retained the Windows boot identity but changed the interactive session and WSL boot; Windows-only observation recorded full readiness about 29 seconds after login, before Linux probes. Owner reported phone chat success; ordinary DNS/HTTPS, existing encrypted state/key and one Passkey passed follow-up without repair. No availability while logged out, S3/hibernate or independent second-trial physical acceptance is claimed. |
| A09 | **Reviewed within documented scope** | P04 paths reviewed; fault/behavior tests and real Grok-to-Codex connector-routing notice exercised. Configured Off stays quiet; actual failures retain notices. Live API fallback billing and controlled Jev failure/recovery now have A04/A05 evidence. | Does not imply exhaustive coverage of every background job or naturally occurring provider outages. |
| A10 | **Accepted within tested scope; deployed and cleaned up** | DeepSeek V4.1 Flash passed real first-run validation, Settings save/rejected replacement, Chinese/English greetings and arithmetic without tools, actual search with service/source receipts, and controlled primary failure with live/final billing disclosure. Complete source gate, 21 browser journeys and archive checks had passed. Backend/frontend deployed privately with existing configuration, key and one Passkey preserved; host HTTPS/health and Windows task checks passed. | Owner reconfirmed the provider's USD 5 cap. This run used USD 0.039110224; combined observed API/Jev/DeepSeek expenditure was USD 0.474292432, subject to billing lag. Temporary key/state/raw logs removed. No new physical phone/reboot acceptance or general model-reliability claim. See [the follow-through report](verification-2026-09-29-deepseek.md). |
| A11 | **Local publication preparation accepted** | Project LICENSE, 608 dependency notices, exact selected upstream sources and Rust toolchain notices included. Fresh-stage packaging rejects stale files, private paths and runtime state; archive headers have neutral identities and safe modes. Full local gate, eight focused packaging/history/compiler checks, final archive crypto/startup checks and 21 final-frontend browser journeys passed. See [release verification](verification-2026-09-30-release.md). | Public visibility and anonymous-clone/download acceptance remain separate. Historical author-email preference requires owner choice before public exposure. No hosted CI or additional paid calls. The initial OpenSSL path-remapping failure is preserved in the report. |

Already accepted separately: user-reported iPhone Face ID/Passkey sign-in and
cellular chat with Tailscale; VPN-off private-tab denial; reauthentication/chat
after the requested all-device sign-out. Private Serve was used, not public
Funnel. Real Codex runtime search and chat passed. A01 has its own owner feedback;
the subsequent physical recovery is accepted under A07.

### A12 — post-review settings repairs and clean installation

PR 3 (`e208d6c`) contains both reviewed P2 repairs. A fresh source/cache/configuration
run passed `make check` (Rust 4,515 passed/47 existing ignored), frontend checks,
22 browser journeys and encrypted lifecycle transitions. Its first Heirloom
status assertion failed; later passing retries were followed by a reproduced
SIGSEGV after the expected verdict. C01's CLI teardown repair (`747dc42`) passed
the full gate again, all 22 browser journeys and its 32-process regression.
Cold-cache packaging exposed C02; `45f04ca` adds locked-source preparation and
passed negative/positive regressions plus real archive checks. The package has
been extracted under a new ordinary account and its preflight passed. Fresh
provider login, managed service/chat and Windows task/idle acceptance remain
pending. Existing AegisFresh deployment acceptance
does not mean PR 3 is deployed. A11 describes the superseded candidate's historical
packaging checks, not approval to publish that older artifact. See the current
[clean verification report](verification-2026-09-30-clean-install.md).

## Verification accounting and release boundary

- Current follow-up results and unfinished work are recorded in
  [clean verification](verification-2026-09-30-clean-install.md); historical deployed
  results remain in [2026-09-29 verification](verification-2026-09-29.md). Do not combine overlapping
  focused counts or treat a build as a live acceptance result.
- Earlier verification reports preserve their own code snapshots and counts.
  Real account/device acceptance stays separate from local harness results.
- The 30-minute host-only interval passed **after manual task recovery**, while
  Windows was awake and the user logged in. Processes predated the interval,
  automatic restart counts were zero, boot identities unchanged, database headers
  encrypted and key modes 0600. It proves neither cold recovery nor unattended
  installation. Preserve both the failure and the later pass.
- Keep secrets, authentication/pairing links, database files and raw account logs
  out of this checklist and Git. Record safe result summaries and report links.

On every follow-up, update the relevant stable IDs with evidence; do not delete
failed observations after a later fix. Recheck open decisions, acceptance and
explicit exclusions before describing v1 as ready.
