# Clean verification and installation — 2026-09-30

Status: **clean source verification passed; installation is still in progress**.
The Heirloom status check exposed a real CLI exit crash; its repair passed the
full gate and process regression. Real installation/chat acceptance requires
fresh owner login. This is not an all-clear for publication; keep drafts private.

## Source and isolation

The fixed product source is PR 3 merge
`e208d6cecaaec48eda5efa85ab52c4e06efdcf06`, preserving the separately reviewed
`85a0d014` DB/cache transaction repair and `02bb5c1c` embedding publication repair.
The Windows checkout fetched remote main before exporting a Git bundle. A new
Linux checkout was created from that bundle and detached at the exact SHA.
This transport does not demonstrate an anonymous clone of the private repository.
The subsequent CLI fix is `747dc424e3c52eb482af670a5e4f3322075be5b3`.
Its separate exported bundle was checked out in the same isolated directory;
only dependencies and outputs produced by this run were reused for its retest.

The run started with empty Cargo registry/build, npm, browser and temporary
directories, no `compiled_skills` or `node_modules`, and a synthetic HOME. The
subprocess environment was explicitly constructed without inherited application
configuration or account credentials. Existing Ubuntu 24.04, system libraries,
Rust 1.98.1 and Node 22.23.3 were reused. This is a clean project/configuration/cache
environment on the existing OS, not a newly installed operating system.

Both Wasm skills were compiled from source. One compiler job was used. The first
full check was intentionally interrupted after reaching passing runtime tests:
its CPU affinity had limited the harness to two test workers. That partial log
was retained and is not counted as a pass. The complete check was then rerun with
16 test workers, four allowed CPUs, a 5 GiB scope limit and only artifacts created
by this clean run. No test was removed or ignored to obtain a result.

## Completed checks

This first table refers to `e208d6c`; the `747dc42` full gate is tracked
separately below rather than inheriting an earlier commit's pass.

| Check | Actual result |
| --- | --- |
| Formatting and Clippy, warnings denied | Passed |
| `make check SKIP_SKILL_BUILD=0` | Exit 0; final complete invocation 954.09 seconds |
| Native Rust tests | 4,515 passed, zero failed, 47 existing ignored across 39 result groups; core library 4,146 passed/four ignored in 731.06 seconds |
| Frontend TAP scripts | 1,063 reported passes, zero failures; counts may overlap across script invocations |
| Setup/installer/packaging/history/compiler fixtures | 43 passed; these are contract fixtures, not a deployed service |
| Frontend production build | Passed; explicit build stamp identifies the product source |
| Fresh Chromium download and browser suite | All 22 real-router/virtual-authenticator journeys passed |
| Remote helpers and lifecycle | Python 11 and focused Rust six passed; four isolated supervisor transitions retained encrypted state; Tailscale was simulated |
| Tracked-file privacy scan | Zero rule findings; manual review remains necessary |
| Reachable bundled-source history scan | Zero rule findings across 38 commits/1,155 unique blobs; not a scan of unreachable or unfetched remote objects |

Native and focused counts overlap; do not add the rows into a unique coverage
total. No real accounts were used in these checks, and virtual WebAuthn does not
prove iPhone, tailnet, cellular-network or Windows reboot behavior.

## C01 — real CLI exit crash reproduced after the expected verdict

The first `make heirloom-mode-test` built the actual runtime successfully and
passed the default/off/unrecognized-mode encrypted-memory assertions. It then
failed at the assertion requiring `setup-auth status` to exit 2 and print
`NOT READY`. The old assertion did not report the actual exit code or a safe
error classification; its private process output was transient. No matching
kernel OOM event was found during inspection.

Two diagnostic runs passed, including one with the original clean environment.
Three further runs under the same four-CPU/memory constraints passed, followed
by a successful rerun of the **unmodified original make target**. These exercised
default-off storage isolation, retained inactive files, explicit encrypted opt-in,
off/on restarts, and wrong-key rejection. They do not establish the cause of the
initial failure or prove that it cannot recur.

Bounded diagnostics subsequently reproduced `exit=-11` (SIGSEGV) after
`NOT READY`, with no observed Rust panic or database-lock/open-error marker.
A separate negative control against the original binary and a strengthened
32-process regression reproduced the same crash. Earlier passing retries did
not establish correctness. Neither reproduction was retried into a pass.

The CLI called `process::exit` while its SQLCipher memory actor could still be
finishing on a Tokio blocking thread. The repair returns the command's status
through `main`, dropping the runtime and joining blocking tasks before process
termination. It applies to both setup-auth and skill-opt exit paths. This removes
the observed unsafe teardown ordering; no native stack trace was captured, so
the precise crashing native function is not claimed.

The behavior regression requires all 32 real status subprocesses to print the
expected verdict and exit exactly 2. Preflight rejection must exit exactly 1,
so a crash cannot count as a successful denial. Diagnostic flags preserve
private output; raw logs, keys and login links are not exposed. The negative
control failed in 19.75 seconds; the repaired regression passed in 25.56 seconds.
Formatting and the affected binary build passed. The full post-fix gate below
also passed; actual installation and account-backed chat remain separate.
Original failed and interrupted logs remain local, outside Git.

The Windows installer contract script also passed: ownership/path checks,
actual remote/local launcher action behavior, scoped legacy action migration,
minute retry and missed-start recovery. It created no real Windows task.

## Full retest of CLI fix `747dc42`

All seven phases exited 0 in the isolated source/cache/configuration environment.
The earlier failed and interrupted runs remain separate records.

| Phase | Observed result |
| --- | --- |
| Formatting and `make check SKIP_SKILL_BUILD=0` | Passed; complete check 933.13 seconds, Clippy warnings denied |
| Native Rust | 4,515 passed, zero failed, 47 existing ignored in 39 groups; core 4,146 passed/four ignored, 710.13 seconds |
| Frontend scripts and production build | 1,063 reported TAP passes, zero failures; build passed in 5.21 seconds |
| Setup/installer/packaging/history/compiler fixtures | 43 passed |
| Remote access and encrypted lifecycle | 89.37 seconds; Python 11, focused Rust six, all 22 virtual-WebAuthn browser journeys and four simulated access-mode transitions passed |
| Heirloom mode and CLI exit regression | Original make target passed in 30.30 seconds, including all 32 status subprocesses and exact exit-code checks |
| Privacy and reachable history scans | Zero rule findings; history 39 commits/1,157 unique blobs; manual/author-metadata review remains separate |

Counts overlap between focused and full runs. No accounts, paid models or real
Tailscale network were used. This is local WSL validation, not cloud or hosted CI.

## Installation and release work still pending

### C02 — cold-cache license collection failure and package repair

The first optimized package attempt on `76adf9e` completed host/Wasm compilation
and the frontend build, then failed in offline Cargo metadata because
`android_system_properties 0.1.5` had never been downloaded. A Linux build alone
does not populate all platform manifests that unfiltered metadata resolves.
The failure log is retained; a package was not accepted from that run.

`45f04cab8f934a90a31cd395661bdd4382f9f716` makes the release builder run
`cargo fetch --locked` before compilation. The full graph is prepared explicitly;
the notice collector still runs offline and selects only Linux/Wasm normal/build
dependencies. Network/fetch failure stops before compilation. Existing
`CARGO_NET_OFFLINE` policy remains respected; a prepopulated cache is then needed.

Both new cold-cache/preparation-failure regressions fail against the old builder
and pass against the fix. The complete packaging/history/compiler fixture target
passes ten tests (previously eight). The same cache that failed then completed
the real `make package` in 35.18 seconds through the shipped flow, with 608 notices
and selected exact sources. Archive verification passed in 2.56 seconds: layout,
permissions, source/license presence, no builder paths, startup, actual encrypted
migration/backup and wrong-key rejection. No manual cache fill was used.

Final archive source: `45f04cab8f934a90a31cd395661bdd4382f9f716`.
Archive SHA-256:
`c7e48a54ace8e9f3e9a36fb6ab4a95c98e5a359b4f0594db4996b25fe9031d94`.
The Rust/frontend code is unchanged from the fully retested `747dc42`; the last
change is the release Python workflow, its regressions and dependency guidance.
The full native/browser suites were not rerun for this packaging-only change.
Final privacy and reachable-history scans found zero rule matches across
41 commits/1,164 unique blobs; author metadata and manual review are separate.

### Actual installation progress

- A separate ordinary Linux account with a locked password and no sudo grant was
  created for an actual managed installation. No old configuration, database,
  encryption key or provider login was copied. Preflight with `--skip-build` passed.
  The verified archive has now been copied with its checksum manifest, checked
  before extraction, extracted as that ordinary user and passed the packaged
  preflight. Its independently chosen ports/configuration were preserved.
- Node's installed tool runtime was copied without account data; copied npm/npx
  launchers initially lost their symlink behavior and were repaired to their
  normal distribution targets before npm succeeded. This was test-environment
  provisioning, not an Aegis code correction.
- The required `zstd` package was installed as a bounded administrator step.
  A new independent Ollama user service/model returned a
  768-dimensional vector on an explicitly selected loopback port. An existing
  port-11434 listener was preserved. The independently managed embedding service
  is a declared installation prerequisite, not proof of Aegis service acceptance.
- Both fresh Codex device authorization attempts expired without completion.
  No provider login was established; request a new code when the owner is ready.
  No real owner-chat result, managed Aegis service, Windows session task or
  host-only idle interval is accepted yet. No paid API call was made.
- Replacement optimized packaging and archive verification passed as above. The
  previous `v0.1.0` draft derives from `4a66b4c`, lacks PR 3 and is explicitly
  marked superseded/do not publish. Existing tags have not been moved. The new
  package remains local; the source/report are in private draft PR 4.
- Read-only inspection confirmed the primary AegisFresh service remains active
  at `af82da33`, with the previously accepted backend/frontend hashes. It has not
  received PR 3. AegisTrialV2 is a separate old experiment and was not changed.
- Continuity reconciliation is prepared in an isolated clone. A started receipt
  reports only this conversation's work; it does not impersonate or acknowledge
  a different participant. The full remote ledger update awaits explicit owner
  destination authorization after automatic approval review rejected that push.

## Remaining boundaries

The settings repairs do not make post-commit HITL ledger finalization or message
delivery cancellation-safe. An action receipt can remain unresolved after its
configuration has taken effect. Existing in-flight embeddings and corpus
migration are outside those repairs. See the two settings verification reports.

Real phone/recovery/provider acceptance from earlier reports remains historical.
There is no new physical-device/reboot, public Funnel, anonymous-download or
hosted-CI acceptance. Author-email privacy and actual public release remain owner
decisions. Append final artifact/install results only after observing them.
