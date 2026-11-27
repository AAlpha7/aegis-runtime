# Release verification — 2026-09-26

This record covers the curated Aegis Runtime tree on a private working branch.
No public repository or deployment was changed.

## Checks completed

| Check | Result |
| --- | --- |
| Three shipped Wasm skills (`web_search`, `web_reader`, `test_no_net`) | Built from source for `wasm32-wasip1` |
| `make test SKIP_SKILL_BUILD=1` | Passed, including host core and executor tests |
| `make check SKIP_SKILL_BUILD=1` | Passed: Clippy, host tests, and WebApp tests |
| Final-source `cargo fmt --all -- --check` and Clippy | Passed after adding the first-run passphrase confirmation |
| `make build` and `make package` (release profile) | Passed with an external `CARGO_TARGET_DIR`; package includes the service, executor, and both database tools |
| WebApp `npm run build` and `npm audit` | Build passed; audit reported zero vulnerabilities after lockfile update |
| `bash -n` on changed shell scripts | Passed |

The host checks ran on Ubuntu 24.04 under WSL with Rust 1.98.1, Node
22.23.3, `CARGO_BUILD_JOBS=1`, and a separate Linux cargo target directory.
Network-dependent tests marked `ignored` by the project were not treated as
passing live-service tests.

## Database confidentiality verification

Both the legacy memory database and the heirloom event log now use SQLCipher
when opened by the service. A synthetic two-database CLI migration retained
each original plaintext file and produced encrypted files that ordinary SQLite
rejected. Neither encrypted file nor the encrypted backup contained its test
plaintext marker. The backup CLI accepted the correct key, rejected a wrong
key without leaving an output file, and the service rejected both a wrong key
and an unmigrated plaintext database during startup preflight.

Unit tests also covered encrypted WAL content, wrong-key reads, migration
metadata, and a consistent online backup including WAL changes. An external
key supplied by the parent environment worked; a key placed only in `.env`
was rejected. In terminal passphrase mode, a mismatched first-run confirmation
created no output; matching confirmation migrated successfully, and a later
backup requested the passphrase once.

This is **data-at-rest** protection for the two SQLite databases. It does not
encrypt old plaintext copies, all other files, process memory, or the
heirloom development seams. Environment-injected keys can be inherited by
child processes; keep the service account and its child programs trusted.

## First-run boundary

The scripted first-run path was exercised with a synthetic database key and
the required Node version. With no real provider credential, it stopped at
provider selection/validation (step 2: `OPENAI_API_KEY` unset or placeholder),
before `setup-auth`; the earlier handoff's prediction of a step-5 stop is not
the current script behavior. No credential was invented. Separately, a
synthetic encrypted state reached `setup-auth status` and exited 2 with the
expected `NOT READY` verdict because no owner chat provider was connected.

## Remaining release verification and publication

This is a source release for fresh installations. Publishing it does not
require a production database key, an existing database, or a data migration.
A new installation creates new encrypted memory and heirloom databases after
the installer supplies a key or terminal passphrase.

1. The real Grok/Codex/API-key login and live chat/WebApp smoke test remain
   unverified. Provider access and a physical phone session were unavailable
   for this verification; completing these checks would finish the human-login
   portion of the first-run acceptance test.
2. Review the documented single-owner and heirloom development limitations in
   `SECURITY.md`, and decide when to publish the repository. No public push
   or deployment is part of this verification.

## Optional migration for existing installations

The migration helper is only for an operator who already has compatible
plaintext memory or heirloom databases, for example from an earlier runtime
build. It is not part of a fresh installation or a prerequisite for this
open-source release. Such an operator must stop writers, copy-migrate both
databases, verify the copies, and separately retire old plaintext files and
backups. The synthetic migration tests above validate that optional tool;
they do not imply that this source release contains user databases or that an
existing Aegisflow installation has been migrated.
