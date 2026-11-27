# Fresh-machine deployment trial — 2026-09-27

Status: completed with assisted local deployment acceptance. Real Codex chat works, the full check suite passed, and a Windows-side idle check passed after Codex exited. This is not an unaided novice success or a public/mobile deployment acceptance.

## Setup and boundaries

- Repository snapshot: `90e8c4e0b1b66cb46320f9335b050f7fa15e6c64`.
- Fresh, separate WSL distribution `AegisFresh`, Ubuntu 24.04.5 LTS. Existing distributions were retained.
- Codex CLI 0.157.1 installed as an ordinary user and authenticated independently with ChatGPT.
- Clean clone created from a Git bundle of the committed snapshot. The origin still points to the private upstream repository; public anonymous cloning is therefore not tested.
- Windows drive automount and Windows executable interoperability disabled. No project build caches or dependencies copied.
- No existing account credentials transferred; no passwordless root access granted.
- The agent receives repository documentation, but no parent conversation, deployment instructions, or troubleshooting assistance.
- Exact initial prompt: `请部署这个项目。`
- The observer accepted repository trust and reviewed individual dependency-download, build, test, local-service, and read-only permission prompts. No blanket sandbox bypass or persistent root grant was supplied.

## Observations

| ID | Observation | Impact / classification |
| --- | --- | --- |
| F01 | The remote version check failed because the upstream repository is private and the fresh environment has no GitHub login. | Trial limitation. Reassess with a public release; do not confuse this with failure to build the supplied clone. |
| F02 | The agent asked whether to deploy here or on another server, and which chat provider to use. | User interaction required. Evaluate whether the product can provide clearer defaults. |
| F03 | The agent asked the user to inject `AEGIS_RUNTIME_DB_KEY_HEX` securely, without an accessible key-creation/setup flow. | A nontechnical user cannot complete this step unaided from the response. |
| F04 | The initial turn ended before installing missing Rust, Node.js, or C build dependencies. | The initial response combined missing decisions and missing dependencies. Installation did proceed after the requested decisions were supplied. |
| F05 | The agent reported 11 deployment logic tests passing and a clean privacy scan. | Agent-reported checks only; not evidence that the application starts or that real chat works. |
| F06 | After the location, provider, and key were supplied, the agent confirmed a valid key with mode `0600` and a logged-in Codex account. `sudo -n apt-get update` then failed because a password was required. | Trial provisioning limitation: the fresh ordinary user has no configured administrative login. CLI command approval alone does not supply Linux administrator credentials. The agent continued checking user-local toolchain options. |
| F07 | The agent downloaded and installed Rust 1.98.1, downloaded and verified Node.js 22.20.0, and started a locked release build with two Rust build jobs. It also prepared Ollama and requested a user-local `zstd` package for extraction. | Multiple downloads and permission prompts are part of the fresh-machine path. Temporary tool installation locations must be considered when checking restart durability. |
| F08 | The agent moved Node.js and Ollama into persistent user directories, then authored a private `aegis-local` wrapper for environment variables, key-file loading, local embedding startup, and lifecycle commands. | The agent solved temporary runtime placement itself. Deployment required additional agent-authored configuration beyond invoking the repository installer. Later lifecycle failures and repairs are recorded below. |
| F09 | Chromium downloaded successfully but required additional shared libraries on the fresh Ubuntu image. The agent requested downloading those libraries for user-local extraction. | Browser acceptance adds another dependency/setup stage. All 15 browser journeys subsequently passed. |
| F10 | The initial embedding endpoint probe failed. The agent reported that background subprocesses were being reaped when tool commands ended and switched to a long-running Ollama tool session. | Agent diagnosis of an execution-environment lifecycle limitation. A service remaining alive after the agent exits and after WSL restarts must be checked separately. |
| F11 | The agent's deployment instructions explicitly state that no boot autostart is configured or verified and that ongoing execution sessions keep the services alive. | Current chat success does not establish unattended deployment or recovery after a host restart. |
| F12 | After the release build and real chat succeeded, the agent continued compiling a separate development/test build for the full `make check` prescribed by the handoff, while the authentication harness build waited on the same build-directory lock. | Fresh deployment includes substantial release-verification overhead. Consider separating end-user installation acceptance from contributor/release checks after the trial. |
| F13 | After the agent delivered its final answer, the observer exited Codex normally. Independent probes to ports 3000, 8765, and 11434 all failed with connection refused (curl exit 7). | Confirmed deployment durability failure: the delivered services depended on the agent session. The agent's final response mentioned lack of boot autostart but did not clearly say that closing Codex would stop the current service. |
| F14 | After receiving only `我关掉 Codex 以后，页面打不开了。请处理。`, the agent confirmed its omission and discovered that systemd was available after all. It chose systemd user services and lingering itself. | The initial conclusion that no usable service manager had been confirmed was insufficient. The repair solved application session lifetime, but F15 exposed the separate WSL lifetime boundary. |
| F15 | After the systemd fix, immediate post-Codex-exit probes succeeded. Restarting the dedicated WSL also brought enabled services back after startup delay. However, after 20 seconds without Linux commands, `wsl --list --running` returned no running distribution and the Windows-side console request was refused. | Confirmed outer WSL lifetime issue: Linux probes can wake WSL and mask this failure. Acceptance must include an idle interval followed by a host-side request that does not start WSL. |
| F16 | The agent independently diagnosed WSL lifetime behavior, authored a Windows scheduled-task installer and Linux session launcher, and requested execution on Windows because the isolated WSL has neither Windows mounts nor interoperability. | Another required user operation. This boundary is partly created by trial isolation. The observer ran the agent's exact reviewed installer; the task is scoped to the current Windows user and this distribution, uses limited privileges, and transfers no Linux secrets. |

## Authorized follow-up

The user requested that all problems be recorded, the trial continue, and improvements be considered afterward. The user authorized choosing a deployment location/provider and supplying an encryption key.

- Location supplied: current environment, local access only.
- Provider supplied: Codex / ChatGPT subscription.
- A random 32-byte encryption key was generated locally outside the repository, with a private parent directory and a file mode of `0600`. Its value is not in this report or the conversation.
- Only the location, provider choice, and key-file location were sent to the deployment agent, followed by `请继续。`
- Generating the key is explicit observer assistance and must not be counted as independent novice success.
- The agent explicitly asked an administrator to run `apt-get update` and install `build-essential pkg-config libssl-dev`. The observer executed exactly those requested operations as the WSL administrator; they completed successfully. The agent was told only that its requested installation command succeeded. No persistent sudo access was added.

## Outcome

The agent's real local HTTP probe returned status 200 with the expected `aegis-chat-ok` answer through the selected Codex subscription provider. The local embedding request also returned HTTP 200 and 768 dimensions. Health and console endpoints responded successfully. Real chat passed again after the systemd migration; no API key or Grok account was used.

Full `make check` completed successfully. Observed test summaries include core tests: 4,114 passed, zero failed, four ignored; executor: 41 passed. The agent's command completed successfully through the final installer checks. All 15 remote-access browser journeys passed and encrypted state survived the lifecycle harness transitions. Those tests used temporary state and simulated Tailscale, not real external-network or mobile access.

The agent also restarted the actual local deployment and confirmed health afterward, with the key loaded from its external file. It retained a fresh, unused local-login entry for the user. This process restart is distinct from a host/WSL reboot or exiting the agent process.

The agent first delivered after 45 minutes 47 seconds in the resumed deployment turn (including approval and observation delays), with real Codex chat working. The observer exited Codex normally and all three service ports stopped responding. Only the user-visible symptom was returned to the agent, without a technical fix or hint.

No product changes have been made in the primary repository in response to this trial. The deployment agent added its own verification results to the cloned repository's verification report.

The systemd repair completed in a further seven minutes. The agent verified real Codex chat again and normal service restarts. Independent observer checks confirmed immediate survival after Codex exit and automatic application recovery when the dedicated WSL booted. A later idle host-side probe exposed F15; again only the user-visible symptom was returned.

For F15, the observer supplied only the requested reproduction details: direct Windows localhost access, no preview or port-forward page, and loss of access after the environment recovered and was left idle. The agent supplied its own Windows keepalive solution. Its installer ran a two-minute Windows-side verification; Codex was fully exited when prompted. No Linux probes were run by the observer during that interval.

The Windows verification completed with exit zero: the scheduled task remained running, console and health endpoints returned HTTP 200 throughout 120 seconds of host-side checks, and the WSL boot ID was unchanged. Codex was closed when the script prompted and remained closed through completion. The task uses the current Windows user's interactive login with limited privileges; it is not a machine-wide service and does not keep the computer awake through shutdown or sleep. Its logon trigger is configured, but a real Windows logout/login or reboot has not been exercised.

## Assessment and improvement candidates

This section describes the original trial at `90e8c4e`. Subsequent implementation
and verification are recorded separately in
[the current verification report](verification-2026-09-27.md); they do not
retroactively change the trial's assistance or acceptance boundaries.

The repository can be deployed with an independent agent and real subscription chat, but the initial one-sentence request did not produce a durable novice-ready installation. The observer had to supply a key, execute the requested administrator command, report two post-delivery failures, and execute the requested Windows installer. None of the technical fixes were suggested to the agent.

Priorities to evaluate before a new release (not implemented in this trial):

1. Provide a guided external-key creation and custody flow instead of asking a novice to inject a named environment variable.
2. Provide a supported service installer that detects the actual host/service manager, configures persistent key injection, and handles WSL's separate Windows-session lifetime.
3. Make deployment acceptance include exiting the agent, an idle interval, and checking from the real user access surface. Linux-only probes are insufficient for a Windows-hosted WSL installation.
4. Consolidate dependency preflight and required administrator actions into an understandable setup flow; distinguish tool-sandbox permission errors from missing host capability.
5. Separate a focused installation smoke test from contributor/release `make check`, while retaining the latter as a release gate. This trial's initial delivery took about 46 minutes, including observation and approval delays.
6. Give Windows users an appropriate launcher/login entry and clearly state login, sleep, shutdown, and recovery behavior.

Still unverified: public anonymous cloning, Grok/API-key deployment paths, physical mobile/external-network access, actual Windows reboot or logout/login, and long-duration unattended operation. Local browser harness tests and the short idle test do not cover these conditions.

## Evidence and retained state

- Initial feature commit remains `90e8c4e`; the primary repository only gained this observation report during the trial.
- The isolated clone's only tracked modification is its verification report. Agent-authored service wrappers, systemd units, and Windows installer are host deployment configuration, not upstream implementation changes.
- The isolated agent session is `01a0e589-e52b-7b02-bbd4-e3f4164d494c`. Raw sessions and service logs remain private and are not added to source control.
- Active host task: `AegisRuntime-WSL-KeepAlive-AegisFresh`. The final runtime remains local only at `http://127.0.0.1:8765/ui/`.
- Stop the Windows keepalive task with `Stop-ScheduledTask -TaskName AegisRuntime-WSL-KeepAlive-AegisFresh`; its installer also provides a scoped removal mode. Stopping the keepalive is separate from stopping the Linux services.
