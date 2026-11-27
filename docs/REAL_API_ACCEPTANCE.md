# Real API and Jev acceptance

This runbook prepares A04/A05 in the [v1 checklist](V1_ACCEPTANCE_CHECKLIST.md).
Neither item passes until actual service results are recorded. Existing mocked
settings tests do not establish account entitlement, billing or API availability.

## Before making paid calls

Agree with the owner on the provider, model, enabled test cases and spending
ceiling. Jev is a separate opt-in: an API chat test does not authorize it. Use a
dedicated limited test key and a provider-side spending control where available;
a request-count limit alone cannot guarantee a monetary cap. If the agreed cap
cannot be enforced or verified, resolve that before calling the service.

Use isolated test state, identity, encrypted databases, ports and key storage.
Do not switch the working subscription deployment, copy its credentials, revoke
its Passkeys or inject service failures into it. Do not expose a new public route.
Use fixed synthetic prompts and public documentation, never mailbox or owner data.

Enter credentials through a local hidden prompt or the authenticated Jev password
field. Never paste them in chat, command arguments, screenshots or reports. The
API first-run wizard writes its key to the installation's private `.env`; Jev's
settings field stores its dedicated key in the encrypted credential store. Verify
permissions and exclusions, and do not describe both paths as encrypted storage.
Never print either secret to verify presence.

The current API bootstrap uses `OPENAI_API_KEY`, `OPENAI_API_BASE` and `LLM_MODEL`.
OpenRouter search and Jev need compatible OpenRouter credentials; a working chat
key for another provider proves neither capability. Use the documented
[installation](INSTALLATION.md) and [subscription settings](SUBSCRIPTIONS.md)
flows rather than treating environment configuration as a verified settings save.

## A04: API chat, search and separately authorized fallback

1. With Jev and paid fallback off, validate the chosen chat provider/model before
   saving; run one real synthetic owner-chat request. Record the actual route.
2. Try an invalid model candidate. Confirm visible validation failure and that
   both saved provider and model retain their previous values after reload.
   A successful backup must not validate the failing candidate.
3. Confirm API chat with search set to `auto` reports search as unconfigured.
   Explicitly choose and verify the authorized search provider/model, then search
   for public documentation. Require usable source URLs and matching provenance.
   A failed search must not claim verification or silently choose another service.
4. Only if separately authorized, validate and enable paid fallback. In the
   isolated installation, make the primary unavailable without modifying or
   revoking the owner's real subscription. Confirm the actual fallback route and
   visible cost/service-change notice in both progress and the final result.
   Label the induced primary outage as controlled, not a real provider outage.
5. Restore the isolated baseline and verify fallback off. Record which cases ran,
   which failed or were skipped, and observed billing separately from estimates.

## A05: optional Jev

1. Verify that configured Off stays quiet. In Settings → Advanced → AI and
   spending → Jev, review the separate billing/content disclosure and explicitly
   opt in with a compatible key. A valid Decisions response must precede saving.
2. Reload settings: enabled/verified and key presence should persist, while the
   key value must never be returned. Try a rejected replacement credential and
   confirm that the previous saved configuration is preserved.
3. Use synthetic public content to exercise a real enabled decision. In only the
   isolated test environment, induce unavailability and verify the documented
   notice/hold behavior. Restore connectivity and verify recovery. Keep induced
   failures and real service responses distinct in the report.
4. Disable Jev and remove the saved test key through settings, including while
   the service is unavailable. Confirm saved Off wins over any environment key,
   and normal disabled use has no repetitive warning.

Stop on unexpected routing, missing disclosures, credential exposure or spending
uncertainty. Remove temporary credentials and isolated processes after recording
safe results. Update A04/A05 and the dated verification report; retain failures
and skipped cases. Do not commit keys, databases, account responses or raw logs.
