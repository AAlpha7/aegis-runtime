# Heirloom: experimental, disabled by default

The first open-source release uses the standard encrypted memory database for
conversation history, learned facts and user profiles. Heirloom is an additional
event log and projection experiment. Its source and tests remain available, but
it is outside the default runtime path and the stable release guarantees.

## Default behavior and upgrades

With `AEGIS_RUNTIME_HEIRLOOM_BRIDGE` unset, `off`, or anything other than the exact
value `on`, the service and `setup-auth` do not initialize, create, preflight, read
or write the Heirloom database. Its parent directory is not created. An existing
`HEIRLOOM_EVENT_LOG_DB` or `AEGIS_RUNTIME_PROFILE_READ_SOURCE` setting does not
enable it. Standard memory encryption and wrong-key checks remain mandatory.

Earlier snapshots enabled dual-write when the switch was unset. Upgrading now
stops those writes. Existing Heirloom databases, WAL/SHM files and backups are
preserved; there is no automatic deletion, conversion or migration back into
standard memory. Any data present only in Heirloom will not appear in default
profile reads. Keep the old data and its key if you need to inspect it later.
An old plaintext file stays plaintext while inactive. Explicit operator backup
and migration commands can still operate on retained files.

## Developer opt-in

Set this in the runtime environment or configuration and restart:

```sh
AEGIS_RUNTIME_HEIRLOOM_BRIDGE=on
```

This creates or opens the SQLCipher event log, normally next to `MEMORY_DB_PATH`,
and enables dual-write after successful initialization. `HEIRLOOM_EVENT_LOG_DB`
overrides that path. The same externally injected database key (or terminal
passphrase mode) is required. Existing plaintext files and wrong keys fail the
startup preflight when enabled; use the documented offline copy migration if
needed. Application initialization failures after preflight warn and fall back
to standard memory.

Profile reads still default to standard memory. To try projections too, set
`AEGIS_RUNTIME_PROFILE_READ_SOURCE=heirloom_preferred` (`heirloom` is an alias).
This takes effect only with the subsystem enabled and a working bridge; missing
or failed projections fall back to standard profiles.

Remove the opt-in or set it to `off`, then restart, to disable the subsystem.
Events generated while disabled are not automatically backfilled on re-enable;
do not treat a re-enabled event log as a complete history.

## Limits

The runtime still uses `SoftEnclave::new_dev(42)`, an internal plaintext payload
codec, permissive ethics validation, a no-op welfare oracle, and a fixed living
owner state. SQLCipher protects stored database pages but does not supply
independent key custody, payload confidentiality against the running host,
enforced ethics, or a finished inheritance/ownership lifecycle. Startup prints
an experimental warning when enabled. See [SECURITY.md](../SECURITY.md).

## Verification without accounts

```sh
make check
make heirloom-mode-test
make remote-access-test
```

The mode test runs the real service with disposable encrypted state and provider
URLs restricted to loopback. It checks disabled startup, old-file preservation,
setup-auth isolation, explicit enable/disable/re-enable, and wrong-key rejection.
Library tests cover standard memory and the experimental writer/projector paths.
These checks do not verify a real provider chat, hardware key custody, or a
physical phone.
