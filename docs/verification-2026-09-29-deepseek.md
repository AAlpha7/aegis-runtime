# DeepSeek API recommendation candidate

This follow-up implements the owner's choice of DeepSeek V4.1 Flash for the
API recommendation and explicitly authorized paid backup. Grok/Codex remain
the preferred subscription paths. Existing model selections, paid-backup Off,
optional Jev Off, and separately configured search remain unchanged.

## Changes

- New OpenRouter API-key setup without an explicit model stages
  `deepseek/deepseek-v4.1-flash`, announces the model and API billing, and runs
  the existing real connection verification before accepting configuration.
  Failure restores the previous environment file. Exported model choices are
  persisted; saved choices and unrelated API endpoints are preserved.
- Settings shows the recommendation only when a connected OpenRouter catalog
  contains the model. The backend exposes the recommendation in that catalog;
  the frontend does not maintain a separate model-ID preset. The chat recommendation changes a draft, followed by
  Verify and save. The backup button explicitly says Verify and enable and
  discloses paid API use. No recommendation activates merely on page load.
- English/Chinese browser behavior checks exercise selection, rejected probes,
  unchanged persisted settings and disappearance of unavailable recommendations.
  Installer checks cover fresh defaults, explicit choices, endpoint identity
  and complete rollback on a failed probe.

## Verification status

The public OpenRouter model catalog listed the exact model and tool-call
support on 2026-09-29. Catalog availability is not a successful account call or
evidence of model behavior. Pricing varies by routing/provider; the catalog
snapshot used for acceptance preparation showed USD 0.30 input / 1.20 output
per million tokens. See the [model catalog](https://openrouter.ai/api/v1/models)
and [model page](https://openrouter.ai/deepseek/deepseek-v4.1-flash).

Focused installer tests: **14 passed**, including the three new recommendation
cases. The first complete Rust run passed the 4,127 core cases (four ignored)
but failed one structural guard: the initial UI implementation embedded a model
ID. The recommendation was moved to the backend catalog, preserving the guard,
and availability/disconnection tests were added. The first run is not counted
as a passing full gate. After correction, complete `make check` passed:
all-target Clippy, core 4,128 passed/four ignored, executor 41, main 36, the
included integration/doc targets, full frontend suite, setup 14 and installer
21. The catalog recommendation test and all eight structural guards also passed
focused checks. Remote helpers (11), 21 browser journeys and all four simulated
access-mode lifecycle transitions passed; encrypted state persisted. These are
local harness results, not physical phone or real provider acceptance. Counts
overlap and are not summed. The 320px English/Chinese backup screenshots were
inspected: the model name and paid-API disclosure wrap without truncation.
Formatting and release compilation passed. The final bundled frontend passed
all 21 browser journeys again. Extracted archive layout, LF shell scripts and
executable startup passed; the copied acceptance binary matches the release
binary. The temporary Windows-created package wrapper exited 127 after these
successful checks because its final blank line contained a CR character. Its
line endings were normalized, and a separate archive startup/binary-identity
verification completed with exit 0. This was not a packaged-script failure;
the initial wrapper failure is retained in the verification history.

- Candidate executable SHA-256:
  `84cc6111a59c86f1e9cc762c78164bef4dd105f9dd16359a4fb79f6ef1254ae9`.
- Candidate archive SHA-256:
  `ee30d201aadf1877d03fd5a902447e50468099fbf1feb3c9ddb4fbbf911b4652`.
- Tracked-file privacy scan found zero findings, including the new report and
  browser scenario; manual diff review completed. No secrets or raw account
  responses are committed. The temporary verification task was removed after
  completion. Temporary account material was removed after real acceptance below.

## Real-service acceptance and budget

**Passed within the tested scope on 2026-09-29 (owner timezone).** The owner
re-entered a key locally and explicitly reconfirmed its USD 5 provider cap.
The initial temporary collector rejected a key with a different usage checkpoint
under its extra USD 4.50 rule; it did not save that key or make a paid call.
The owner's clarification removed that extra rule, retaining authenticated
metadata checks for a finite USD 5-or-less cap, no periodic reset, remaining
credit and BYOK accounting. The successful entry started at zero recorded usage.
This collector is an ignored acceptance helper, not the product settings UI.

The verified candidate executable ran in a separate state directory and ports,
using its own encrypted databases and temporary credential. Actual checks passed:

- Updated first-run setup selected DeepSeek V4.1 Flash without a supplied model
  and completed the real provider probe before accepting configuration.
- Actual Settings verification/save passed. Paid backup and Jev were initially
  Off. Chinese and English greetings and arithmetic returned the expected
  answers with no tool evidence; rejected replacement returned HTTP 422 and
  preserved the selected provider/model.
- Explicit OpenRouter search configuration verified and saved. Real owner chat
  invoked `web_search`, returned IANA and RFC sources, and retained the actual
  service provenance in the receipt. Jev Off produced no Jev warning. The
  receipt reported `openrouter_native_xai_search` without fallback; this is
  paid API search, not subscription search.
- A loopback test proxy deliberately rejected the primary model while forwarding
  the authorized backup to the real DeepSeek service. The backup answered the
  greeting without tool evidence. Captured live SSE and the final answer both
  disclosed fallback and separate billing. The SSE reader closed after the HTTP
  command completed, matching the UI lifecycle. Paid backup was disabled and
  its model cleared again before stopping the isolated runtime.

Final observed usage was USD **0.039110224**, leaving USD **4.960889776** on this
key. Together with the earlier API/Jev checkpoint of USD 0.435182208, observed
test expenditure was USD **0.474292432**. Provider billing can lag; this is a
recorded checkpoint, not a guaranteed final invoice. See the
[previous report](verification-2026-09-29.md) for the earlier run. Its nano-model
unnecessary-tool failure remains historical; these limited DeepSeek successes
do not establish general tool-selection reliability or a natural provider outage.

## Deployment and cleanup

The primary private trial was upgraded from `1c897f8d` to source `af82da33`,
using the candidate backend and bundled frontend after real acceptance. Before
replacement, the stopped service's database was copied with the SQLCipher
backup helper; the prior executable/frontend remain available for rollback.
The environment configuration and external database key remained byte-identical.
The original database loaded and retained one registered Passkey. Existing
subscription selection and saved optional-service settings were not changed.

Host verification found an active managed service, HTTP 200 health and private
HTTPS access, a Running Windows session task, key mode 0600 and tailnet-only
Serve to the loopback console. Public Funnel was not enabled. The deployed
backend matches the candidate SHA-256 above; both the installed and HTTPS-served
frontend index matched
`2703e162812effd11d6c3f311696c3019c081d40643b25e4f741218dad95219a`.
This deployment did not repeat physical iPhone chat, reboot, logout or sleep
acceptance, and did not bypass remote authentication for a host chat probe.
Earlier physical acceptance remains separately recorded in the checklist.

The isolated runtime exited and its credential, encrypted test databases and
raw logs were removed. The Windows DPAPI key copy was deleted and task-owned
entry helpers closed; only safe acceptance summaries remain outside the repo.
Provider-side key revocation is not claimed. Publication and hosted CI remain
excluded; no further paid test is scheduled.
