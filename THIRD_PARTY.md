# Third-party components

Aegis Runtime's original code is under [MIT](LICENSE). Dependencies keep their
own licenses. This file does not relicense dependencies or external services.

`make package` collects original LICENSE, COPYING, NOTICE, COPYRIGHT and AUTHORS
documents from the locked Rust sources and installed frontend runtime packages.
The Linux release includes `THIRD_PARTY_NOTICES.txt` and a versioned
`dependency-inventory.json` with original source URLs and checksums. The Rust
inventory conservatively includes Linux/Wasm normal and build dependencies;
it is not a claim that every listed crate is linked into the executable.
Development-only and other-platform dependencies remain in the source lockfiles.
This release packager currently targets Linux x86_64, not other architectures.

Some published crates omit a license file. Reviewed supplemental texts and their
upstream URLs/SHA-256 hashes are in `licenses/supplemental/manifest.json`.
Most come from the exact source revision recorded in the crate. Where a crate
declares an Apache-2.0 alternative or MPL-2.0 without including the text, an
official standard text is supplied and marked as such; no copyright holder is
invented. For ITT API, the BSD-3-Clause alternative is used. Bundled unmodified
`.crate` source archives include MPL components, ITT API and the standard-text
Apache supplements, and are verified against Cargo.lock. They retain original
headers. Source links for the other crates are in the inventory.

The collected documents include nested native components such as SQLCipher,
ring and AWS-LC. The pinned Rust toolchain's original library/compiler copyright
documents are retained in `rust-toolchain-notices`. System shared libraries are supplied by the Linux distribution,
not copied into the release; retain that distribution's notices. Provider CLIs,
Tailscale, Ollama, downloaded models and cloud services are separate installations
with their own terms and licenses; they are not relicensed or bundled here.

The default fictional portrait was generated for this project; its provenance
is recorded in [docs/AVATAR.md](docs/AVATAR.md). It is served locally. The other
project branding assets remain part of this repository. No provider trademarks
or affiliation rights are granted by this project.

To reproduce the notices from a source checkout, run `make package`. Its release
build first fetches the complete locked Cargo source graph, including manifests
needed to resolve other platforms, and the web build runs `npm ci`. The actual
notice collection is offline and still selects only Linux/Wasm dependencies. It fails on
missing source/license evidence or a changed supplemental text. Dependency
updates require reviewing and refreshing the supplemental manifest; do not
silently reuse a notice from a different version.
