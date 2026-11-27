# API and Jev live acceptance follow-through

This continues [the v1 checklist](V1_ACCEPTANCE_CHECKLIST.md) on the private
`feat/durable-first-run` candidate. The owner authorized API and Jev tests with
a combined USD 5 ceiling. A dedicated key's metadata confirmed a USD 5 total
limit, no periodic reset and zero initial usage. Credentials were entered in a
local password field, encrypted with Windows user protection and transferred
through stdin into an isolated owner-only Linux directory. No key is in this
report, command arguments or Git.

The tests use separate encrypted databases, identity, loopback ports and runtime
processes. They do not copy subscription credentials or change the existing
phone installation's settings or Passkeys. Prompts contain synthetic markers or
public IANA documentation requests. Environment-injected chat credentials are
not described as encrypted application settings.

## Completed live checks

- The installed binary's API bootstrap probe succeeded with
  `openai/gpt-4.1-mini`; the independent owner-chat route returned the requested
  synthetic marker. The settings provider/model save also passed a real probe.
- An invalid model received HTTP 422 and left both previous provider and model
  unchanged on reload. Automatic search with API chat was rejected as
  unconfigured, without silently selecting a paid search route.
- The installed binary's Jev Decisions probe succeeded. The authenticated Jev
  settings flow saved enabled/verified state after a real probe. An invalid
  replacement key received HTTP 422 and preserved that state. Restarting the
  isolated runtime retained enabled/verified state and saved key presence; the
  settings response did not contain the key value.
- A controlled primary-timeout diagnostic returned a real
  `openai/gpt-4.1-nano` backup answer through OpenRouter after explicit verified
  backup and paid-fallback saves. Both switches were cleared afterwards.
- A separate loopback fault proxy rejected the primary model while forwarding
  the authorized backup to the real API. The owner-chat result contained the
  requested marker and the service/model-change and separate-billing notice.
  This is controlled primary failure with a real paid backup, not evidence of
  a naturally occurring subscription outage. This initial check verified the
  final notice; the final binary's live SSE check is recorded below.

An early read-only billing checkpoint (2026-09-29 07:11:46 UTC) reported USD
0.020399436 charged and USD 4.979600564 remaining on the test key, with zero BYOK
usage. Billing updates can lag; this is a checkpoint, not a final invoice.

## Search validation defect found by the live test

The OpenRouter search request returned a citation, but the settings save rejected
it as lacking sources. Search adapters return Aegis's normalized
`verified_citations` payload; the validator instead parsed the raw Codex executor
`results` schema. The check now uses the normalized lookup parser, which requires
successful status and usable HTTP(S) source URLs. Model prose alone still cannot
validate search.

A regression test exercises the real adapter/probe path with annotated HTTP
responses: valid sources must pass, a prose-only URL must fail, and a JavaScript
URL must fail. It does not inject a successful validation result. The original
live failure is retained; corrected live search subsequently passed below.

## Corrected live search and Jev follow-through

The corrected release binary passed real OpenRouter search verification/save and
returned an IANA source with matching provider evidence. Configured Jev Off
remained quiet. Enabled Jev search passed too. An initial repeated prompt in the
unavailability case returned an answer without tool evidence; it was not counted
as a successful fresh search. A new request with a unique request ID and distinct
public question returned actual search evidence and the Jev-unavailable notice
under a controlled unsupported Jev model. Restoring the model restored verified
state, actual public search and quiet output. With child-process upstream access
blocked, disabling/removing Jev succeeded and saved Off overrode the environment
opt-in. These are isolated configuration faults, not natural provider outages.

The additional real fallback SSE probe found a product defect: the durable reply
included separate API billing, but the typed live fallback event carried only a
short model-switch message. The live and durable paths now use the same complete
localized copy, and the adjacent vision-failure retry uses the same language
selection instead of a mixed-language sentence. New/extended behavior tests
exercise actual fallback attempts in English and Chinese, requiring the live
copy to match the durable disclosure. Both focused cases passed.

The no-tool search answer is also treated as a product gap, not solved merely by
changing the fixture. Explicit English/Chinese web-lookup requests now get one
correction through the normal tool/privacy gates. If no search ran, the final
reply clearly says the sources/provider were not verified instead of reusing the
model's unverified prose. Approval holds and callbacks retain their existing
paths. Tests cover persistent no-tool answers in both languages, successful
correction with a source receipt, explanations and explicit search opt-out.

The live probe also initially waited for an SSE `done` frame. Inspection confirmed
that this owner-command route uses HTTP completion and the UI closes advisory
SSE in its `finally` block; the fixture now follows that contract. The earlier
failed attempts remain recorded here. Final-binary results follow below.

## Final source verification

After both late corrections, complete `make check` passed: all-target Clippy,
core 4,127 passed/four ignored, executor 41, main 36, included integration/doc
targets, the full frontend suite, Jev setup 11 and installer 21. The two new
explicit-search integration cases passed in both focused and full runs. Remote
helpers (11), 20 browser journeys and all four access lifecycle transitions also
passed. Pre-existing opt-in external tests remain separate from these local
checks; counts overlap and are not added together. Formatting, the final release
build and extracted archive startup checks passed. The packaged binary matches
the executable used in the final real-service retests.

## Final artifact, real-service results and deployment

- Executable SHA-256:
  `92b1af4ffaedaf0993017e16f20a0ebdf4f118a1dfc1fe41eb5349c52810306b`.
- Linux archive SHA-256:
  `610b5fdf78acc90187fceb5926bb4b474092ea23d51218af7972212b4ccb5eea`.
- The final executable passed actual search verification/save and source
  retrieval with Jev Off, healthy enabled Jev, controlled unavailable Jev,
  recovery, and offline disable/removal. Saved Off overrode environment opt-in.
  Repeating the earlier explicit search with a fresh request ID returned a new
  actual search receipt and matching OpenRouter/IANA provenance.
- Captured real owner-command SSE and its HTTP result both contained the same
  complete English model-switch and separate-billing notice. The fault proxy
  rejected the primary and forwarded the real backup. The final reply answered
  `HELLO`; the stream closed after HTTP completion without a reader error.
  Offline assertions over this capture verified disclosure without more paid
  calls. Chinese/English equivalence also passed automated behavior tests;
  this particular live SSE capture was English.
- **Unresolved model-quality observation:** the low-cost backup unnecessarily
  read a public encyclopedia page before answering that greeting, and used the
  disclosed Jina reader fallback. The original no-unnecessary-tools assertion
  failed and remains failed; the narrower disclosure pass does not override it.
  Earlier synthetic marker prompts also produced an invented link attempt,
  blocked by the reader's safety checks. This does not establish reliable tool
  selection for this backup model, nor justify enabling paid fallback by default.
  The isolated fallback switches were cleared after each attempt. No account
  data was requested by these synthetic prompts.
- The primary private trial received a backend-only upgrade with a rollback
  binary retained. The actual running executable matches the tested hash.
  Backend, console and ordinary private HTTPS returned 200. Serve remained
  tailnet-only, the existing Passkey count remained one, the database key mode
  remained 0600, and the frontend hash was unchanged. Subscription settings and
  encrypted owner state were retained. A fresh physical-phone chat was not
  repeated after this backend-only update; prior phone acceptance remains a
  separate result, not proof of this new build's phone journey.
- Final key metadata reported **USD 0.435182208 used**, USD 4.564817792 remaining,
  zero BYOK usage, and a USD 5 limit without periodic reset. This is a billing
  checkpoint, not an invoice; provider accounting may lag. No further paid
  requests are scheduled. The protected isolated state and Linux key, Windows
  protected key copy, and temporary verification task were removed; test ports
  were closed. This is local-copy cleanup, not provider-side key revocation.
  The first cleanup attempt stopped before deletion because this isolated WSL
  distribution had no expected Windows mount; exporting the safe summary via
  stdout resolved it. Safe result summaries remain; raw temporary logs and
  databases were removed with the isolated state.

## Historical build interruptions and recovery

Overlapping contributor and release compilation exhausted the WSL VM's 8GB
allocation and swap; kernel diagnostics recorded an OOM-killed Rust compiler.
Primary HTTP health temporarily timed out, then backend and console returned
200 with the installed Windows session task Running. No credential reset or
manual service restart was performed. Later build sessions also ended without
a complete result, including a recorded hangup; those are not passing checks.

The replacement verification task limited compilation to one Cargo job/two CPUs
and a 5GB memory/768MB swap scope. It was also interrupted: WSL returned
`Wsl/Service/E_UNEXPECTED`, and make reported missing child processes. Some Clippy
invocations completed before the interruptions, but no complete post-fix test
gate or release build passed at that checkpoint. The repeated session failures are not attributed
solely to the earlier OOM. No surviving Windows client for the dedicated build
script was found at cleanup; the paid test processes had already exited.

The owner subsequently restarted Windows. Before Linux probes, both host-side
HTTP endpoints returned 200. Ordinary private HTTPS then returned 200; the
existing Passkey count remained one and the external database key retained mode
0600. No pairing or credential reset was performed.

The first complete post-reboot Rust run reported 4,124 core tests passed, one
failed and four ignored. The new search behavior regression passed, but the
source transport inventory rejected its newly added loopback mock route. The
route is now explicitly classified as test-only with an exact count; the guard
still rejects unregistered transports. This failed run is retained rather than
reported as a passing gate. The next run progressed without new failures but
slowed substantially: both test threads waited on `jbd2_log_wait_commit`, with
roughly 77% I/O pressure and negligible CPU/memory pressure. Only that dedicated
verification scope was stopped. The replacement run uses an owner-only temporary
directory on `/dev/shm` for automated fixtures, under the same one-job/two-CPU,
5GB memory limit; no test is filtered out. Release compilation uses disk-backed
temporary storage. These fixture checks do not claim power-loss durability.
Real API acceptance and the existing deployment retain disk-backed encrypted
databases. The replacement core run passed 4,125 tests, with zero failures and
four ignored, including the citation regression and the transport inventory.
Before the late live-notice correction, complete `make check` passed: all-target Clippy, core 4,125 (four
ignored), executor 41, main 36, included integration/doc targets, the full
frontend suite, Jev setup 11 and installer 21. Remote helper tests (11) and all
20 automated browser journeys also passed. Counts overlap and are not summed.

The lifecycle fixture initially timed out in its negative public-mode case:
the Windows-scheduled WSL invocation inherited a terminal, so the helper
correctly prompted for risk acknowledgment instead of taking its noninteractive
rejection path. The fixture now passes `stdin=DEVNULL` to its child commands.
Under the same scheduled invocation, all four private/public/private/local
transitions and encrypted-state persistence passed. The Tailscale daemon is
simulated in that fixture; no real public service was enabled. This changes only
test input handling, not the product's acknowledgment requirement. The initial
corrected search binary built, formatting passed, and its Linux archive passed
extraction/startup checks with an identical packaged binary. The live-notice
correction subsequently passed the final artifact and real-service checks above.

## Remaining boundaries

The low-cost backup's unnecessary tool use remains an observed quality gap.
The verified API/Jev configuration, search evidence, disclosure and cleanup
checks are complete within their stated scope. Publication and hosted CI remain
excluded. The repository and PR remain private and draft; no fully unaided
public-clone installation or natural subscription outage is claimed.
