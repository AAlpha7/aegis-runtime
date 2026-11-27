# v0.1.0 preview publication preparation

Started 2026-09-29; continued 2026-09-30 in the owner's timezone.

This follow-up closes release packaging and disclosure work after the real
[DeepSeek acceptance](verification-2026-09-29-deepseek.md). It does not authorize
public visibility, publish a release, invoke hosted CI or add paid model calls.
The runtime code and existing private deployment are unchanged by this work.

## Changes and review

- Include the project LICENSE in the archive and require it in archive checks.
- Build each archive in a fresh temporary stage; only tracked scripts/docs/skill
  sources, named runtime binaries, the two current Wasm skills and the frontend
  build are included. Old staging files, untracked helpers and retired DDG
  artifacts are excluded. Reject frontend symlinks and runtime state files.
  Failure preserves the previously completed archive.
- Initial artifact review found the builder's home path in compiled panic/error
  strings. The distribution build now remaps Rust and native C/C++ paths and
  rebuilds both host binaries and Wasm skills. Archive headers omit builder
  user/group identities. Extraction checks reject remaining home/workspace paths
  in all five executables and both Wasm files. This is build hygiene, not a
  runtime behavior change; do not equate the new artifact hash to an older build.
  The first remapped build still contained OpenSSL's configured engine/module
  directories and CFLAGS version text. The package guard rejected it before
  replacing the previous archive (overall packaging phase exit 2). This is a
  retained failed attempt, not a passing full release gate. A compiler shim now
  maps those build-directory defines without storing private prefixes in CFLAGS;
  a real C probe verifies both `__FILE__` and the engine-directory define.
- The prebuilt Linux archive uses the Ubuntu 24.04/glibc 2.39 baseline; this is
  stated in the release notes rather than implying support for every Linux host.
- Generate offline notices for 605 locked Rust normal/build dependencies across
  Linux x86_64 and Wasm, plus React, React DOM and scheduler. Include nested native
  notices such as SQLCipher and the pinned Rust toolchain's original copyright
  documents. Dependency inventories contain exact versions and source checksums.
- Bundle 17 original, checksum-verified crate source archives for MPL components,
  ITT API and the Apache standard-text supplements. Other source URLs remain in
  the inventory. Supplemental files have recorded provenance and SHA-256; Git
  preserves their original bytes, including upstream whitespace. All 22 stored
  supplemental document hashes matched the Git index. This is an inventory and
  notice review, not an independent legal opinion or exhaustive dependency audit.
- Add release notes and a reproducible private-to-public release procedure.

## Privacy scope

The initial history scan covered all fetched/local branches: 33 commits and
1,083 unique blobs. Ten URL matches were manually reviewed as the documented
`computer.tailnet.ts.net` and `device/other.example.ts.net` examples; there were
no unresolved matches. Commit-message credential checks found zero matches.
The reusable scanner reports positions only and exempts only those exact sample
origins. Tests ensure an example cannot mask a second, non-example origin.

The four tracked binary asset paths were inspected. The generated portrait
contains only three pixel-unit EXIF fields, with no location/identity fields or
XMP marker. The three PNG icons contain no text or EXIF metadata chunks. Portrait
generation provenance remains in AVATAR.md. No account state or raw acceptance
logs are committed. Git author identities are intentional metadata and were
reviewed separately; the owner was asked whether to retain the existing real
author email before publication. No history rewrite was performed.

The current-file and reachable-history scans are supplements to manual review.
They do not prove absence of every secret, inspect unreachable server objects or
constitute a hosted security audit. A public clone remains unverified while the
repository is private.

## Verification and release state

The complete local `make check` passed, including core 4,128 passed/four ignored,
executor 41, main 36, all-target Clippy, integration/doc targets, frontend tests,
14 setup and 21 installer cases. Four packaging behavior tests and three history
rule tests and one real native-compiler probe passed. Counts overlap and are not summed. Formatting, 11 remote helper
checks, 21 real-router browser journeys and four simulated access lifecycle
transitions passed; encrypted state persisted. No real account was used in these
checks. The first remapped packaging attempt failed for the reason recorded above.
After the packaging-only corrections, the focused checks were repeated and the
remapped release rebuild completed successfully. The final archive passed layout,
LICENSE/third-party notice, source-archive, permissions, builder-path exclusion,
shell syntax and executable-startup checks. Its actual encrypt/backup helpers
successfully migrated and backed up a disposable synthetic database, rejected a
wrong key and left no plaintext header/marker in either encrypted result.

The final frontend passed all 21 browser journeys again. Its build stamp changed
with the build date/base commit; no frontend product source changed in this
follow-up. The release backend SHA-256 is
`1c61d410c4dea65d42d4da94a1b4d3268f1272fec50a825063efab66416fcb29`;
the final frontend index SHA-256 is
`5d6497c0c23aa2947abd52bd0be557b87d3e689ca696504a6e1f8eb350f72017`.
The archive's final digest is supplied in SHA256SUMS alongside the draft asset,
avoiding a self-referential archive hash in its own documentation.

Current-file privacy scanning found zero findings. Windows Git diff checks
passed; a WSL diff invocation initially treated existing Windows line endings as
changes, and returned cleanly when run with the matching `core.autocrlf=true`
setting. No broad source normalization was performed. The history review above
is the pre-merge snapshot; repeat it after final refs are updated.

Local release checks are complete. GitHub merge/tag/draft state must be verified
from repository metadata; public visibility is still a separate owner decision.
The existing private service retains its earlier real-provider-accepted build.
This build-only follow-up made no new paid calls and does not claim new physical
iPhone/reboot acceptance or anonymous public downloads.
