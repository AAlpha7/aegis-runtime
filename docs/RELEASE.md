# Preview release procedure

Keep the repository private until the owner explicitly authorizes publication.
Hosted CI remains manual-only and is not required or dispatched for this release.

1. Review AGENTS.md and the current acceptance checklist. Fetch all intended
   public branches/tags and scan their reachable history, commit messages,
   current files and archive contents for secrets and private data. Run
   `python3 scripts/privacy-audit.py` and `python3 scripts/privacy-history-audit.py`.
   The history scan covers all fetched refs; it does not scan unreachable remote
   objects or replace review of Git author identities. Review
   matches manually without printing secret values. Include image provenance
   and third-party notices. A regex scan is not proof of absence.
2. Run the required local contributor, browser, remote-access and lifecycle
   checks. Record exact results and skipped external acceptance. Do not rerun
   paid tests unless code changes require them and a budget is authorized.
3. Run `make package` on Linux x86_64 with the pinned toolchain and Node 22+,
   then `python3 scripts/test_package.py dist/aegis-runtime-0.1.0-linux-x86_64.tar.gz`.
   It must include LICENSE, original dependency notices and selected exact
   upstream source archives. The generator fails when locked evidence is missing.
4. Review the final diff and privacy scan, merge the reviewed private PR into
   main, and verify main is the intended default checkout. Record the merge SHA.
5. Prepare an annotated `v0.1.0` tag and a **draft**, explicitly marked preview
   release, attach the tested archive and SHA256SUMS, and use
   RELEASE_NOTES_0.1.0.md. Do not move an existing published tag. A draft release
   and private repository must not be described as publicly available.
6. With explicit publication authorization, change visibility and publish the
   prepared release. Then verify an anonymous fresh clone, download checksum
   and documented installation path. Public/anonymous access cannot be accepted
   while the repository remains private. Record any account/admin help required.

Build outputs are made in a fresh temporary stage and the final archive is
replaced only after success. Old `dist/aegis-runtime` content and untracked local
helpers are not inputs. Review new dependency-license supplements whenever
Cargo.lock or package-lock.json changes. Runtime/phone recovery acceptance is
not inferred from packaging or documentation checks.
