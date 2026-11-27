# Aegis Runtime 0.1.0 preview

Anqi is a personal AI secretary hosted on your own computer, with a web console
you can reach from a phone. This is a single-owner preview release, intended for
early adopters willing to follow installation and account setup instructions.

## Included

- Grok/Codex subscription chat, web search and authorized external execution.
  Connected-account tools require actual connector access and authorization.
- Explicit API mode, with verified DeepSeek V4.1 Flash recommendation on
  OpenRouter. Paid fallback requires opt-in and discloses separate billing.
- Encrypted local memory, persistent work and scheduling, an action ledger and
  source receipts. Wasm skills are isolated; external processes have separate
  host boundaries described in SECURITY.md.
- Managed Linux/systemd installation and a Windows session supervisor for WSL2.
  Private Tailscale Serve access uses host-paired Passkeys. The host must remain
  awake, online and, for the Windows session supervisor, signed in.
- English/Chinese settings and progress messages, mobile layouts, improved
  source presentation and a locally served fictional humanoid default portrait.
- Atomic settings persistence: a failed database write leaves the previous
  live configuration intact. Grouped changes commit together. Embedding client
  updates follow the same commit order, including cancelled or delayed requests.
- CLI shutdown waits for database background work before returning its exit
  status, fixing a reproduced crash after an otherwise correct setup verdict.
- Release packaging prepares locked dependency sources before offline license
  collection, so an initially empty Cargo cache is supported.

## Deliberate defaults and limits

Google login, Jev, paid fallback, Hercules delegation and experimental Heirloom
are off unless explicitly configured. DDG is removed; failed subscription
search does not silently switch services. Realtime voice is deferred; configured
voice-note transcription is separate. There are no native mobile applications
or multi-user guarantees. This archive targets Linux x86_64; WSL2 uses the same
Linux runtime plus the supplied Windows session installer.
The prebuilt archive is built on Ubuntu 24.04 and requires glibc 2.39 or newer;
older distributions should build from source instead of using this binary.

Real validation covers the tested Grok/Codex accounts, limited API/Jev/DeepSeek
scenarios, iPhone Passkey/private access/chat and primary-host recovery. The
installation trials required account and bounded administrator interaction;
they were not fully unaided novice deployments. Controlled provider faults do
not demonstrate every naturally occurring outage or general model reliability.
Review [the acceptance checklist](V1_ACCEPTANCE_CHECKLIST.md) and the dated
verification reports for exact results, historical failures and exclusions.

Configuration publication and action-receipt delivery have different guarantees.
Cancelling a HITL handler after its settings commit can still leave its action
receipt unresolved even though the configuration took effect. The settings
fixes do not make every post-commit notification or ledger transition durable.

## Install or upgrade

Start with [INSTALLATION.md](INSTALLATION.md). Choose and verify your provider
before starting the service. For an existing installation, retain the original
external encryption key and make a verified encrypted backup before upgrading.
Never replace an existing data key or rerun recovery just to update the program.
Remote access is described in [REMOTE_ACCESS.md](REMOTE_ACCESS.md).

The archive includes the project LICENSE, THIRD_PARTY.md, generated third-party
notices, a checksummed dependency inventory and selected exact upstream source
archives. SHA256SUMS accompanies the release. Publication status is controlled by
the repository/release visibility; this document alone is not a public release.
