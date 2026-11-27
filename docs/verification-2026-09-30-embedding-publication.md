# Ordered embedding settings publication — 2026-09-30

## Finding and scope

Base: `85a0d0147f09492403d56b8bcb7a6bd17bf8e80b`. That commit closes the
DB/cache persistence P2 and remains independently reviewable. A subsequent
independent review identified a **pre-existing, separate P2** in the next step:
admin PUT/reset and HITL changed the embedding client after awaiting the commit.
It was not introduced by the DB/cache repair.

The supplied independent fixture paused request A's response processing after
commit, completed request B, then resumed A. Both PUTs returned 200; DB/cache
contained B, but the live embedding client contained A. Cancelling A after its
commit left the client unchanged. The fixture and its five-test passing log were
read as reviewer-supplied evidence; this follow-up adds its own positive
regressions against the actual HTTP and HITL entry points.

Only the isolated repair branch is changed. No production service, database,
permission, deployment, credential, paid model or personal mailbox was touched.
No new dependency, schema migration or network operation is added.

## Implementation

1. For commits carrying a live client, split the existing five embedding setting
   updates into pure preparation and infallible local application. Preparation
   validates the complete batch and normalizes model names before any database
   write. An invalid later candidate
   rejects the entire batch; DB, cache and client stay unchanged.
2. Pass the existing shared client clone and prepared plans into the same memory
   actor command. After SQLite COMMIT succeeds, the actor publishes the settings
   cache and then the client updates before replying or accepting the next
   command. A cancelled or delayed HTTP/HITL future cannot skip or repeat them.
3. Return previous/current values and hot-update outcomes as a commit receipt.
   PUT/reset/HITL only render that receipt; they never apply the client again.
   A delayed reset response describes its own reset, even if a later PUT won.

The current embedding hot-update path does not rebuild an HTTP transport or
contact a provider. The construction-failure tests cover building a valid local
update plan; they do not pretend to test an absent remote-client constructor.
Client mutexes cover only short in-memory swaps, never SQLite work, an await or
network I/O. Unrelated setting writes use the existing actor queue without any
new global embedding lock. Corpus-reembed warnings remain intact; no automatic
reembedding is performed.

## Verification

Seven new regressions exercise actual production entry points with temporary
SQLite databases, the real in-process embedding client and synthetic credentials:

- PUT, reset and HITL: deterministically pause the actor, enqueue A, resume the
  actor while leaving A's handler unpolled, then either cancel A or complete B
  before resuming A. Client/cache/DB and commit-specific response values agree.
- Invalid update-plan construction, including a valid first batch item followed
  by an invalid dimension; retry and normalized model persistence.
- Real SQLite statement, deferred-COMMIT and DELETE failures: no client update,
  preserved DB/cache, successful retry.
- A database write lock does not block a concurrent client read.
- Sixteen rapid embedding writes interleaved with unrelated language writes.

The prior smaller embedding cooldown regression now exercises the actor commit
receipt rather than simulating the obsolete post-await handler sequence.

Initial focused run: all seven new regressions passed (11.38 seconds).
Final `cargo fmt --all -- --check` and `make check`: passed. Clippy passed with
warnings denied. Native Rust: 4,515 passed, zero failed, 47 existing ignored
(core library: 4,146 passed, four ignored, 675.75 seconds). All seven new
regressions also passed in this final full run. Frontend TAP: 1,063 passed,
zero failed. Setup/installer/release/privacy-history/compiler fixture suites:
43 passed. Script messages about installation/removal refer to mocked fixture
services, not actual host services.

Final frontend production build and `make remote-access-test`: passed. Remote
Python tests: 11; focused Rust remote-access tests: six; virtual-authenticator
browser journeys: all 22; isolated encrypted-state lifecycle transitions: all
four. No real accounts were used. These harness process restarts do not attest
to a real phone or the owner's deployed service restarting.

Final privacy audit: zero findings. `git diff --check`: passed. Raw local test
evidence is in `../embedding-final-aggregate.log` and `../embedding-remote.log`;
these temporary-fixture logs are not added to the repository.

Commands use the existing Ubuntu WSL toolchain, isolated source checkout/cache,
one compiler job and offline dependencies:

```sh
export CARGO_BUILD_JOBS=1 CARGO_NET_OFFLINE=true npm_config_offline=true
export npm_config_audit=false npm_config_fund=false
cargo fmt --all -- --check
SKIP_SKILL_BUILD=1 make check
SKIP_SKILL_BUILD=1 make remote-access-test
python3 scripts/privacy-audit.py
git diff --check
```

## Limits and handoff

- This guarantees ordered completion of DB/cache/client publication for the
  three changed persistence paths, including request cancellation. Separate
  component reads are not one cross-component transaction snapshot.
- Existing in-flight embedding requests, automatic promotion/revert workflows,
  and legacy direct in-memory client updates are outside this repair. Startup
  environment/default priority and corpus migration behavior are unchanged.
  The plain DB/cache API does not discover arbitrary client instances: manually
  calling it and then separately calling `apply_settings_update` remains an
  unsafe persistent-update pattern. The three real entry points no longer do so.
- HITL action-ledger finalization and user-facing delivery remain handler-owned
  best-effort operations; this patch makes the live client cancellation-safe,
  not every post-commit notification or ledger transition. Cancelling that
  handler can still leave an execution receipt unresolved. This is a remaining
  boundary, not a newly introduced client/DB inconsistency.
- No physical phone, real owner reboot or provider acceptance is repeated.
  The accepted AegisFresh deployment remains unchanged and does not yet contain
  either repair. No claim of absolute safety is made.

The follow-up is a separate local commit on top of `85a0d014`; rollback is a
revert of that follow-up and requires no schema migration. Publication and
deployment still require explicit parent approval. Deliver a full commit ID,
SHA-256 of the exact exported diff, and a verification manifest/raw commit
object so the independent reviewer can verify the snapshot without changing
Git ownership protections or `safe.directory`.
