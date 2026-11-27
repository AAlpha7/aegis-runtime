# Phone access on your own computer

Aegis runs on your computer and stores its databases there. Choose an access mode
during `scripts/first-run.sh`, or run `python3 scripts/remote-access.py` after local
setup. Keep the computer awake and online while accessing it remotely.

| Mode | Phone setup | Who can reach the login endpoint |
| --- | --- | --- |
| Local only (default) | Use the host computer | Host only |
| Private (recommended remote option) | Install/sign into Tailscale and connect its VPN | Devices permitted by your tailnet policy |
| Public (optional) | Any ordinary browser | Anyone on the internet can send requests |

Both remote modes use HTTPS and host-paired Passkeys. Public access requires a
separate risk acknowledgment: a general installer `--yes` is not consent.
HTTPS and Passkeys do not eliminate vulnerabilities or compromised-device risks.
For public deployment, protect accounts and devices, apply security updates,
preserve recovery access, and turn off exposure when it is not needed. The project
still must provide authentication, authorization and accurate security guidance.

## Prerequisites

- Complete [managed local setup](INSTALLATION.md). Use `managed-runtime.py with-key`
  for host helpers to load its external key file, or retain your own external
  injection of `AEGIS_RUNTIME_DB_KEY_HEX`. The key never belongs in a phone or pairing link.
- Install Tailscale and sign in on the same Linux/WSL environment as Aegis.
  Its daemon must be able to reach Aegis at `127.0.0.1`. Windows-host Tailscale plus
  WSL networking is not assumed to work automatically.
- Python 3 is needed for the deployment helper; build the `aegis_runtime_access`
  binary with the other core binaries. Source installers already build it.
- The helper manages `aegis-runtime-local.sh`, which delegates to an installed
  managed user service when present. Existing
  `/opt` systemd deployments need their service environment and restart wired
  separately; do not run the portable helper against an unrelated deployment.
- Tailscale may request HTTPS/Funnel authorization in a browser. The user completes
  account login and device authorization. The project operates no account service.

## Choose and change modes

```bash
# Interactive provider and access choices:
scripts/first-run.sh

# Explicit installation choices (provider authentication still applies):
scripts/first-run.sh --provider api-key --access-mode private
scripts/first-run.sh --provider api-key --access-mode public --accept-public-risk

# Change an existing portable installation:
python3 scripts/remote-access.py --mode private
python3 scripts/remote-access.py --mode public --accept-public-risk
python3 scripts/remote-access.py --mode local
```

An agent must explain the public warning and obtain the user's selection before
supplying `--accept-public-risk`. The helper persists the notice version and checks
the actual Tailscale listener configuration. It closes the previous managed route
before switching and refuses to overwrite unmanaged configuration. If shutdown
cannot be verified, it reports failure rather than claiming private access.
An unrelated concurrent daemon change is preserved even during failure cleanup;
in that case the warning requires local inspection because exposure may remain.

The helper restarts Aegis with the selected `.env` configuration, then configures
Serve or Funnel. A failure leaves an explicit incomplete transition; do not treat
it as a successful deployment. It never automatically reopens a previous public
listener on failure. Keep any existing Cloudflare or other forwarding routes
disabled; the helper only manages the Tailscale configuration it owns.

The host helper prints a private, single-use pairing link, valid for ten minutes.
Open it on the phone and select **Create Passkey**, then **Sign in**. Its secret
is in the URL fragment, not a server query string, and the page removes it from
the address bar. Do not paste that link into public logs or screenshots. Browser
or device cancellation consumes the pairing attempt; generate a new link.

## Additional Passkey, recovery and sign-out

```bash
# Replace the path/origin with this installation's actual values.
# The database key is injected externally, or prompted in the host terminal.
target/release/aegis_runtime_access pair /path/to/memory.db https://computer.tailnet.ts.net

# Lost credentials: local host control + database key are required.
# This revokes all owner Passkeys, owner sessions and pending pairing attempts.
target/release/aegis_runtime_access recover /path/to/memory.db https://computer.tailnet.ts.net --confirm-reset
```

Pair a second credential in advance. To sign out all devices, visit `/access`,
select **Sign out all devices** and verify a Passkey again. Losing the database
key cannot be solved by Passkey recovery. Changing the HTTPS hostname changes
the credential's origin: pair again from the host; do not broaden the allowed origin.

Remote deployments disable the old local login link and host-confirm endpoint.
A request arriving from a loopback proxy is never treated as proof of host ownership.
Legacy Google/Telegram authentication may remain configured; administrators should
review those credentials as additional authorized access paths.

## Private-network policy and service limits

Private access still uses application login. Review Tailscale access rules: the
initial policy can permit all devices in a tailnet to reach each other. Limit
access to the intended user/devices and the Aegis HTTPS port. The helper does not
rewrite existing tailnet policy or silently disable device-key expiry.

Tailscale Personal is currently free for personal use; Funnel remains Beta with
bandwidth limits. Account/device authorization, connectivity and third-party
availability can interrupt remote access. Test these from your actual phone.
Public access has a larger attack surface than private access.

References: [Serve](https://tailscale.com/docs/features/tailscale-serve),
[Funnel](https://tailscale.com/docs/features/tailscale-funnel),
[default access rules](https://tailscale.com/docs/concepts/device-visibility),
[device expiry](https://tailscale.com/docs/features/access-control/key-expiry).

## Troubleshooting private DNS

An active Serve route does not prove that the device opening the URL can resolve
its private hostname. Check on that device, while connected to the correct tailnet:

```bash
tailscale dns status
tailscale dns query YOUR_NODE.YOUR_TAILNET.ts.net
getent hosts YOUR_NODE.YOUR_TAILNET.ts.net
```

If the internal query succeeds but the system lookup fails, inspect whether
Tailscale DNS is disabled and which resolver the OS uses. A WSL installation can
still be forwarding DNS to Windows even though Tailscale runs inside Linux.
After reviewing existing DNS requirements, enabling DNS on that node uses:

```bash
sudo tailscale set --accept-dns=true
```

This changes the node's DNS preference, not its Serve/Funnel exposure. It is not
an automatic installer change: preserve deliberate custom DNS configurations.
The inverse setting restores the preference if necessary. Verify both the private
URL with ordinary HTTPS/certificate validation and an ordinary public-domain
lookup. Do not treat a hosts-file entry, `curl --resolve`, or disabled certificate
verification as evidence that system DNS works. Do not edit the generated WSL
resolver file by hand; check DNS again after the coordinated WSL/Windows recovery
test. Enabling DNS inside WSL does not configure the Windows host or the phone.

An enabled preference alone is not sufficient evidence. If resolution fails again,
inspect `tailscale dns status`, the `/etc/resolv.conf` link and recent Tailscale
DNS diagnostics: WSL can regenerate its resolver configuration while Tailscale
is using it. Record the failed ordinary lookup before intervention. A daemon
reload can temporarily interrupt private access and a later successful lookup
does not prove persistence. Keep actual sleep/reboot recovery open until it is
observed without a manual reload; do not hide the condition with a fixed IP.

See [Tailscale DNS behavior](https://tailscale.com/docs/reference/faq/dns-resolv-conf)
and [Linux DNS integration](https://tailscale.com/docs/reference/linux-dns).

## Credential-free verification

```bash
python3 scripts/test_remote_access.py
cargo test -p aegis-runtime-core --lib remote_access::tests
cargo build -p aegis-runtime-core --bin aegis_runtime_access --example remote_access_harness
cd webapp
npm ci
npx playwright install chromium
# Install Playwright's OS dependencies if required on a fresh Linux host.
node scripts/test-remote-access.mjs
cd ..
python3 scripts/test_access_lifecycle.py
```

The browser suite uses the real HTTP router, SQLCipher and WebAuthn verification,
with a Chromium virtual authenticator. The harness is loopback-only, has no model
provider and is not packaged. Deployment tests simulate the Tailscale CLI: they
cannot prove real tailnet permissions, mobile VPN compatibility or external reachability.
