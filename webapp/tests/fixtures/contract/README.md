# FE↔BE contract fixtures (S1 Sensor 2)

Checked-in golden JSON bodies that **both sides read**:

- **Rust** (`core/src/admin_tests.rs`, tests named `contract_fixture_*`)
  produces each body from the REAL backend — oneshot through
  `oauth_callback_router` where a test harness exists, or the production
  body constructor directly where a live provider would be required — and
  golden-compares it against the file here.
- **FE** (`webapp/tests/contractFixtures.test.mjs`, runner
  `npm run test:contract-fixtures`) loads every `*.json` in this directory
  and runs it through the REAL matching FE parser/guard
  (`readVNextStatusProjection`, `sendVNextMail`, `createVNextMailImapAccount`,
  `createVNextMailGmailSetupUrl` via mocked `fetch`), asserting the parse
  succeeds and that **no top-level fixture key is silently dropped** unless
  it is listed in that shape's explicit `KNOWN_DROPPED` set.

A fixture with no FE assertion entry fails the FE test (completeness guard),
so a new backend shape cannot be goldened without deciding its FE handling.

## Updating after an intentional backend change

```bash
# 1. Regenerate the fixtures from the real backend:
AEGIS_UPDATE_CONTRACT_FIXTURES=1 SKIP_SKILL_BUILD=1 \
  cargo test -p aegis-runtime-core --lib contract_fixture_

# 2. Review the git diff of this directory — every changed field is a wire
#    contract change the FE must handle.

# 3. Re-run the FE side and update the parser/guard + KNOWN_DROPPED entries
#    in the SAME commit:
cd webapp && npm run test:contract-fixtures
```

Notes:
- `gmail_setup_url_ok.json`'s `auth_url` has its `state` query param pinned to
  `STATE_NORMALIZED` (it is an HMAC over a per-request random flow id);
  everything before `&state=` is byte-exact.
- `vnext_status_projection.json` pins `advanced_diagnostics_available` to
  `false` (the live value reads process-global env/test state).
