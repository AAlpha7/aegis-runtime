# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Aegis Runtime, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, open a **private security advisory** on GitHub:
[Security → Report a vulnerability](https://github.com/AAlpha7/aegis-runtime/security/advisories/new).
That is the only monitored channel.
Non-security bugs use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md); the same private form is the contact named in [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact assessment
- Suggested fix (if any)

**Response expectations.** This repository is published for reference and is
not actively maintained. Reports are read on a best-effort basis; neither a
reply nor a fix is promised. Review the code yourself before relying on it.

## Scope

The following are in scope:
- Wasm sandbox escapes
- S-LEG domain whitelist bypasses
- Ed25519 signature verification flaws
- IntentEnvelope replay attacks
- Memory safety issues in the host process
- Prompt injection vectors that bypass permission inversion
- SQLite injection via memory actor

## Security Architecture

See [docs/DESIGN.md](docs/DESIGN.md) for the full security model, including:
- Zero-trust IntentEnvelope signing
- Wasm hard isolation (per-call instance creation and destruction)
- Permission inversion (LLM output interception)
- Anti-replay timestamp windows
- S-LEG network gating

## How unknown tools are handled

Hallucinated tool names never reach execution. Three independent layers agree
before anything runs:

1. **Permission gate (ToolGate).** Every tool call is checked against a
   registered whitelist with per-tool policies (`Once`, `Intent`, `Domain`,
   `Always`). Unknown tools are blocked by default in autonomous modes
   (heartbeat, callback, subcall).
2. **Dispatch table.** Even a call that passed the gate must resolve in the
   orchestrator's dispatch table. A name with no implementation resolves to
   nothing — there is no dynamic lookup to exploit.
3. **Cross-turn exfiltration guard.** A two-phase attack ("read the result now,
   send a fragment out later") is detected across conversation turns and fails
   closed (`retryable: false`).

In interactive mode the gate may allowlist an unknown name for operator
experimentation, but layers 2 and 3 still apply: without a dispatch-table
entry nothing executes, and exfiltration patterns still fail closed.

## Known limitations

Stated plainly so nobody relies on a guarantee that is not there yet:

- **Remote access is self-hosted and single-owner.** See
  [remote access](docs/REMOTE_ACCESS.md) for host-issued Passkey pairing,
  recovery, session revocation, and private/public modes. Public Funnel access
  exposes the login service to the internet and requires explicit acknowledgment.
  Private Serve access still needs suitable tailnet rules and application login.
  Local automated tests do not certify real mobile networks or account policies.

- **Database encryption is limited to data at rest.** The memory and heirloom
  SQLite files (including their WAL pages) use SQLCipher and require a separate
  key or terminal passphrase. Existing plaintext files and their backups must
  be migrated and retired separately. The running process and anyone with its
  database key can read the data; other files, logs, and exports are outside
  this protection. With environment-variable injection, child processes may
  inherit the key; run the service and its helpers under a dedicated account
  and avoid untrusted child programs.
- **Heirloom is experimental and disabled by default.** Only an explicit
  `AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on` enables it. When disabled, startup does not
  open, create, preflight, read or write its database; existing files remain on
  disk and retain their previous protection level. Standard memory stays active.
  When enabled, the binary wires an
  in-process soft enclave with a fixed dev seed, a plaintext *internal* payload
  codec, and a permissive ethics validator. Database encryption protects a
  copied database file, but these seams do not provide hardware key custody,
  independent payload secrecy, or an ethics enforcement guarantee. See
  [Heirloom status and opt-in](docs/HEIRLOOM.md). An explicit backup or migration
  command can still operate on a retained database; disabling does not erase it.
- **Process execution exists on the host.** Wasm skills cannot spawn processes,
  but the host does for local speech-to-text, headless browser fetch, MCP stdio
  clients, the external-executor bridge, the Codex CLI management endpoints, and
  an avatar debug endpoint. The separate Aegis Executor service is the gated
  path for operator shell work.
- **Subscription execution is a host boundary.** Binding a subscription enables
  the matching CLI unless explicitly disabled. These processes are not Wasm
  sandboxes. Codex checks ChatGPT login; external child environments exclude
  API-key variables. Connector reads require detected app access and use Codex
  read-only/write-approval controls; provider annotations and local CLI policy
  still matter. Realtime voice minting is disabled in the default release.
  See [subscription boundaries](docs/SUBSCRIPTIONS.md).
- **Jev is a separate API boundary, disabled by default.** Only explicit
  settings-page consent or `AEGIS_JEV_ENABLED=1` (or `true`/`yes`), plus a key,
  enables its API calls. Saved settings override the environment, including Off;
  settings keys are encrypted and never echoed. Enabling or changing a key requires
  a successful live Decisions probe before saving (fixed test context, separately billed);
  existing keys do not opt in. Jev uses OpenRouter Decisions, not a
  Grok/ChatGPT subscription. Its input includes proposed outbound content or
  action context and is separately billed. Without Jev, public searches retain
  the existing local-rule exception; configured Off stays quiet, while an
  enabled judge's failure produces a degradation notice. These rules do not
  identify all sensitive content. Other outbound calls hold;
  no-key computer actions default to Ask (hard denies and the session probe
  exception remain). Never treat a local rule result as Jev verification.
- **Single-owner threat model.** Authentication assumes exactly one trusted
  operator. There is no multi-user isolation, no per-user authorization, and no
  tenant separation.
