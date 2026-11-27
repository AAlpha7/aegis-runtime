# Settings persistence repair — 2026-09-30

Follow-up: the separate pre-existing embedding-client publication gap found
after this DB/cache repair is documented in
`verification-2026-09-30-embedding-publication.md`. The results below describe
the original independently reviewable commit, not closure of every derived
runtime-client or notification path.

## Scope and source

Owner authorized an isolated repair, failure/recovery tests and a continuity
correction. This branch is based on `main` / PR 2 merge
`4a66b4cc2fc3318ffd91d1a062fe42d18b40be6a` (merged 07:26:27 UTC).
Remote was fetched again before the handoff. No push, merge, deployment,
permission change, production restart, paid provider call or personal-mail read
was performed. Work used an independent Windows clone and Ubuntu WSL fixtures.

## Reproduced defect (P2)

On the unchanged baseline, seed `paid_api_fallback_enabled=true` in a temporary
database, reopen with the database quota set to one byte, then save `false`
through `persist_runtime_setting`. The save returns an error, but the live value
becomes `false`; reopening the database restores `true`. The regression
`settings_persistence_quota_failure_preserves_cache_and_restart` failed on the
baseline with exactly that mismatch. Multi-key provider/model, search and backup
updates also used independent writes, allowing partial persistence.

## Repair

- Validate and normalize all keys before writing. The memory actor applies each
  settings batch in one SQLite transaction and publishes the cache only after
  COMMIT succeeds. Reset metadata and effective defaults publish together.
- A cache snapshot sees the complete batch. No cache lock spans SQLite I/O.
  The actor owns publication even if the waiting request is cancelled.
- Call-setting validation keeps its owned mutex guard through actor commit and
  publication. Related disable/reset writes share that lock. Unrelated settings,
  such as language, use actor serialization without waiting for a provider probe.
- Group provider/model/activation, subscription selection, Codex reasoning,
  search provider/model and fallback model/authorization updates. First-run
  setup uses the same transaction API. Respect an explicit executor Off setting.
- Generic admin writes/resets and HITL config apply now report persistence
  failure; they no longer claim success after changing only memory. Browser
  regression covers a failed paid-fallback disable and successful retry in
  English and Chinese.

No schema migration or new dependency is required. Raw startup/migration setting
methods and legacy heartbeat configuration flows remain outside this grouped API.
Separate `get()` calls can span a commit; this is not a transaction snapshot of
an entire model invocation. Credential revocation and settings storage are still
separate operations; a routing-save error after revocation is reported explicitly.

## Verification

All provider probes in these tests are injected success/failure fixtures;
browser uses a loopback harness, synthetic encrypted database and virtual
WebAuthn authenticator. The existing Ubuntu Rust/Node/Chromium installations and
offline dependency caches were used; no system dependency was installed.

| Check | Actual result |
| --- | --- |
| Baseline quota regression | Failed as expected: live false, reopened disk true |
| Frontend `npm test`, `npm run build` | 1,063 tests passed; build passed |
| Clippy, all targets, `-D warnings` | Passed |
| Core library; main binary tests | 4,139 passed / 4 ignored; 37 passed |
| New settings persistence regressions | All 12 passed in the final native run |
| Complete native Rust aggregate (39 result groups) | 4,508 passed / 0 failed / 47 ignored; exit 0 |
| Setup / managed installer / release packaging / privacy history / compiler scripts | 14 / 21 / 4 / 3 / 1 passed; exit 0 |
| `scripts/test_remote_access.py` | 11 passed |
| Browser `webapp/scripts/test-remote-access.mjs` | 22 journeys passed, including bilingual failed-disable/retry |
| `scripts/test_access_lifecycle.py` | All four fixture transitions and encrypted restart-state checks passed |
| `scripts/privacy-audit.py`; `git diff --check` | 0 findings; passed |

Two earlier browser attempts stopped in pre-existing navigation/authentication
steps while compilation and another suite ran concurrently. A subsequent full
22-journey rerun passed after compilation ended. Resource contention is a possible
explanation, not a proven root cause; the earlier failures are not counted as
passes. A superseded intermediate full Rust run was stopped to reduce duplicate
load; only the final complete run counts below.

Reproducible account-free aggregate (with existing Node 22, Chromium and Rust
dependencies; `CARGO_TARGET_DIR` may point to an isolated cache):

```sh
export CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true npm_config_offline=true
SKIP_SKILL_BUILD=1 make check
(cd webapp && npm run build)
SKIP_SKILL_BUILD=1 make remote-access-test
python3 scripts/privacy-audit.py
git diff --check
```

This execution ran the `make check` constituents with the frontend command
separately, using `npm ci --offline --no-audit --no-fund`. Existing ignored
`test_no_net.wasm` and `web_reader.wasm` fixtures were reused. On a fresh clone,
build the skill fixtures first or omit `SKIP_SKILL_BUILD=1`.
The 47 native skips are existing repository-declared ignores, including live
provider/network and staging checks; none were enabled for this repair. The
4,139-test core run took 696.80 seconds. Final native/script gate exit code was 0.

Independent static review first found cancellation could release the validation
lock too early. This was repaired and given a deterministic actor-barrier test.
The second review confirmed guard ownership, failure release, lock ordering and
HITL failure receipts. A further review confirmed the narrowed validation key
set includes Codex executable/home/workdir and the subscription API base.

## Deployment evidence and limits

The read-only inspection before this repair found:

- Windows source `C:\AI\aegis-runtime`: clean `main@4a66b4c`.
- Main AegisFresh deployment `/home/agent/aegis-runtime`: clean
  `acceptance/durable-v2@af82da33411a41d62400fcded5489c27e65b1969`.
  Running binary SHA-256
  `84cc6111a59c86f1e9cc762c78164bef4dd105f9dd16359a4fb79f6ef1254ae9`;
  installed and HTTPS-served frontend index SHA-256
  `2703e162812effd11d6c3f311696c3019c081d40643b25e4f741218dad95219a`.
  Product source matches the PR 2 merge; later merge changes concern packaging
  and documentation. A different release-package binary hash is not proof that
  the running accepted build was replaced.
- AegisTrialV2 is a separate old experiment at `5e0dbaa`; its service is also
  active. Its modified historical verification document was left untouched.
- Main health/UI responded; settings required authentication; local login was
  absent in remote mode. Backend listeners were loopback, Tailscale Serve
  private, Funnel false. Key-file metadata was checked without reading values.

The historical A10 DeepSeek acceptance, private deployment and temporary-data
cleanup are recorded in `verification-2026-09-29-deepseek.md`; they are not tests
repeated by this repair. No physical iPhone, host reboot, real subscription,
provider-side key revocation or live DeepSeek acceptance was repeated. This fix
is not deployed and does not change those historical claims.

## Rollback and next step

Before integration, review the local diff/commit and final gate results. Revert
the repair commit to undo the source change; there is no database migration.
No production rollback is needed because no runtime was changed. Publication
and deployment require the parent's explicit approval of the concrete change.
