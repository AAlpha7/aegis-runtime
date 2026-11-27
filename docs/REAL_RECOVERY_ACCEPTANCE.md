# Real recovery acceptance

This runbook closes A07/A08 in the v1 checklist only after actual device results.
It is not a passed-test report. Coordinate one operation at a time with the owner;
do not restart/log out the computer or revoke real credentials as a preparation step.

## Phone baseline and Passkey recovery (A07)

1. Confirm the phone uses the correct Tailscale account/VPN and can sign in at the
   existing HTTPS origin. Check the updated interface and one real chat response.
2. On the host, confirm the installed access helper, exact existing memory database
   and externally managed key are available. Use `managed-runtime.py with-key` to
   run the documented access-helper `status`; never print or copy the key. An
   existing database must be checked before calling the helper, not created as a
   side effect of a guessed path. Keep the original origin and encrypted data.
3. Obtain the owner's readiness to lose current sessions and Passkeys. Explain
   that host recovery revokes all owner credentials, sessions and pending pairings;
   it does not recover a lost database key or remove unrelated application data.
4. Run the documented `recover ... --confirm-reset` once. Deliver its short-lived
   pairing link privately and keep it out of Git and raw reports. Before enrolling
   the replacement, have the owner check that the previous session/Passkey can no
   longer access the application. Record the actual displayed outcome.
5. Enroll a replacement from the link, sign in using Face ID/Passkey, and verify
   one real chat response on cellular with the VPN connected. Verify the host's
   credential status again. An expired/consumed pairing link needs a new host
   pairing operation, not another reset of working credentials.
6. Record old-access rejection and restored access separately. Stop if either
   fails; virtual-authenticator coverage is not a substitute for these results.

The exact helper syntax and security boundaries are in [remote access](REMOTE_ACCESS.md).

## Windows lifecycle recovery (A08)

Test sleep/wake, sign-out/sign-in, and restart/sign-in separately. The Windows
task belongs to the logged-in user: availability while asleep or logged out is
not promised. Save the owner's work before each transition. Record the expected
outage and a bounded recovery observation period; a configured retry is not a pass.

1. Before a transition, record Windows Task Scheduler state and host HTTP health,
   the current boot identity, runtime artifact identity and a phone-chat baseline.
   Close deployment terminals; do not leave a separate agent-held Linux shell
   responsible for keeping the installation alive.
2. Let the owner perform the agreed transition. After sign-in/wake, initially
   observe only Windows Task Scheduler and the existing host HTTP endpoints.
   Do not run `wsl.exe`, open a WSL terminal, browse WSL UNC paths, or use the
   application's start/login launchers: those can start the environment and mask
   an automatic-recovery failure. Do not manually start a stopped task.
3. Allow up to five minutes for recovery, noting when health/console first return
   and whether the owned session task remains Running. If recovery times out,
   record failure before attempting a repair; a later manual start is a distinct
   assisted-recovery result. Do not include private response bodies in the report.
4. After the host observation, inspect Linux service/boot state, key file permissions
   (not contents), ordinary private DNS, and the tailnet-only Serve configuration.
   Confirm that existing encrypted state and Passkeys still work without new setup.
5. Have the owner use the phone on cellular/VPN for a real chat. Record each
   transition's host recovery, DNS, phone sign-in and chat result independently.

The earlier awake idle test and failed-child supervisor test remain separate
evidence. A07 does not close A08; A08 does not validate paid APIs or Jev.
