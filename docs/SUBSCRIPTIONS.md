# First release: subscriptions, execution and search

Binding Grok or Codex through `setup-auth` or **Use for chat** enables external
execution unless `external_executor_enabled=false` was explicitly stored.
`external_executor_primary=auto` follows the selected chat subscription:
Grok uses Grok Build ACP; Codex uses Codex app-server. A task stays on that
executor by default (`external_executor_fallback=none`). Paid model fallback
remains disabled unless separately authorized in Settings.

For new **OpenRouter API-key** installations without an explicit model,
`first-run.sh` recommends `deepseek/deepseek-v4.1-flash` and verifies the choice
before accepting setup. Existing model choices and other API endpoints are
preserved. Settings offers the same recommendation only when the connected
OpenRouter catalog lists it: chat selection is staged until **Verify and save**;
the paid-backup button explicitly says **Verify and enable**. A failed probe
preserves the active configuration. This recommendation neither changes an
existing subscription nor enables paid backup, search or Jev on its own.

The matching CLI must be installed and authenticated as the service user.
Grok's CLI login and Aegis's encrypted Grok chat login are separate sessions.
Codex reuses `codex_cli_home`; `AEGIS_EXECUTOR_CODEX_HOME` overrides it.
Explicit executor binary/model environment overrides remain supported.
External tasks run as host processes in fresh scratch directories; they are
not Wasm skills. Review the CLI's sandbox and local configuration when deploying.
API-key environment variables are excluded from external process inheritance.
Codex additionally checks `account/read` and requires a ChatGPT account before
starting a thread. Grok CLI authentication and its own saved billing settings
still require real-host acceptance; installing a binary is not proof of login.

## Search

| Setting | Search route | If unavailable or no usable sources |
| --- | --- | --- |
| `auto` (default) + Grok chat | Grok subscription Responses web search | Visible failure; retry the same service |
| `auto` + Codex chat | Codex app-server live web search | Visible failure; retry the same service |
| `auto` + API-key chat | Unconfigured | Select and verify a search service in Settings |
| `openrouter` (explicit verified selection) | Metered OpenRouter search | Visible failure; no secondary service |

Settings also allows explicit Grok or Codex search. Rebinding chat resets search
to `auto`. An explicitly disabled external executor prevents Codex search.
No missing subscription silently selects OpenRouter. Subscription quotas and
provider restrictions still apply. An empty result may trigger one disclosed
broader-query attempt on the same service; errors do not switch services.

DuckDuckGo has been removed, including the bundled Wasm skill and packaging.
Existing saved `duckduckgo` settings remain visibly unsupported until the owner
selects and validates a replacement; there is no silent provider migration.
The runtime ignores leftover bundled `web_search.wasm` artifacts on upgrades.
Old persisted receipts may retain their historical source names.

Codex search requires a completed `webSearch` item plus parseable result URLs;
model prose alone is not search evidence. Search runs read-only with shell
execution disabled. The public-search privacy gate remains in front of each
supported route. Failed search is not evidence that current information was
verified; the response explicitly offers retry or settings repair.

## Runtime disclosures

Behavior changes are announced by the runtime, not left for the model to mention.
Owner chat sends a live notice and preserves a deduplicated copy in the final
answer after model review and before history storage. Failure of the progress
queue cannot erase that final copy. The notices contain fixed, localized reasons,
not raw provider errors, private queries or credentials.
Approved direct tool replays use a separate scope so they retain the same
disclosures in the human receipt, even though they bypass the chat model.

| Change | Owner is told |
| --- | --- |
| Search unavailable / no sources | No verifiable sources; retry or check Settings. No automatic service switch. |
| Empty search → broader retry | Search terms and scope were broadened; the original question is unchanged. |
| Native X → web search | Coverage, freshness and handle/date filters may differ; actual underlying search source is retained. |
| Subscription unavailable → configured API; model/executor retry | Different service/model, attempted fallback, possible capability differences and separate API billing. Default paid/cross-executor opt-outs remain. |
| Connector task → Codex | Access is checked; reads and text drafts only, no sends or deletions. |
| Page reader → Jina | Third-party URL processing and completeness/freshness limits; both-reader failure is explicit. |
| Enabled Jev temporarily unavailable | Public search retains local privacy checks and holds; failed action assessment pauses the action. Deliberately disabled Jev stays quiet. |
| Opted-in Heirloom profile unavailable | Standard memory used; Heirloom-only details may be missing. |
| Vision routing/media omission; degraded embeddings | Changed model/input or incomplete semantic recall/deduplication. Voice transcripts do not retain audio characteristics. |

Jev's separate API billing and outbound content assessment are disclosed when
the owner enables it in settings. Disabled Jev is an intentional configuration,
not a per-turn failure warning. Provider errors never become notice copy.
Same-provider transport retries and internal parsing defaults that do not change
service, data, cost, capability or outcome are not presented as user decisions.
Existing degraded-status banners and timezone fallback notifications remain.
This turn-scoped mechanism covers owner chat, including research started there;
standalone diagnostics/background jobs retain their own status/receipt paths.

## Jev and subscription-only deployment

Jev is the security decision model in this repository (there is no `jex` adapter).
The current implementation calls OpenRouter's **Decisions API**, independently
of the chat/executor subscription. **It is off by default**, even when an API
key is already configured. In **Settings → Advanced → AI and spending → Jev**,
the owner can save a dedicated OpenRouter key, opt in with disclosure consent,
turn it off or remove the saved key. Saves apply to subsequent outbound,
computer-use and non-CU decisions without restart; requests already sent cannot
be recalled. Saved choices take precedence over environment configuration,
including an explicit saved Off. Keys live in the encrypted credential store
and are never returned by the settings API; only presence/source is shown.
Enabling Jev or replacing its key sends fixed, non-user test content to Decisions
before saving. HTTP success alone is insufficient: the response must contain a
valid verdict and risk probability. This may incur a small API charge. Failure
leaves the encrypted configuration unchanged. The settings badge records a
successful check at save time; it is not a promise of continued availability.
Older settings and environment-only keys remain explicitly unverified until
checked. Disabling and removing a saved key do not require network access.

The same validate-before-save rule applies to the owner settings for chat
provider/model, Codex model/reasoning, search provider/model, backup model and
paid API fallback. Chat provider and model are staged together and submitted
with **Verify and save**. Each check uses the requested provider directly;
working fallback services cannot validate a broken primary. Search needs usable
sources. Validation failures return a safe reason and preserve previous values.
Generic admin writes and chat configuration approvals cannot bypass these
protected settings; they direct the owner to the verified settings flow.
Host bootstrap/environment configuration is not a live-verification record.

Without a saved settings choice, opt in with `AEGIS_JEV_ENABLED=1` and restart the
runtime. This master choice applies to outbound checks, computer use and the
optional non-CU Jev gate; that gate's own flag cannot enable paid calls alone.
When enabled, Jev uses its saved dedicated key or an externally injected API key:
`AEGIS_JEV_API_KEY`, then `OPENROUTER_API_KEY`; the legacy `OPENAI_API_KEY` is
accepted only with an OpenRouter `sk-or-` prefix. An OpenAI key must never be sent
to OpenRouter merely because chat uses it.

Interactive `scripts/first-run.sh --provider api-key` asks whether to enable Jev
after explaining its separate charges and disclosure of outbound/action context.
The default answer is **No**. An explicit Yes enables it only if a compatible
credential is available; otherwise it stays off with instructions to inject one.
The wizard does not copy a separately injected Jev credential into `.env`.
Existing choices are preserved. `--yes`, piped input and unattended setup never
grant Jev consent. Direct `setup-auth --provider api-key` only binds chat; use
the first-run wizard or explicitly configure the master switch to opt into Jev.

When disabled, without that key, or when Jev fails, the existing public-search exception uses
local secret, private-marker and provenance checks. The settings explain this
normal disabled state; only an enabled judge's temporary failure adds a warning.
These heuristics are **not equivalent to Jev** and cannot identify every sensitive
query. Other outbound actions remain held for approval (or blocked without an
owner). Secure computer use without a key now defaults to local **Ask**, retaining
hard denies and the read-only session-probe exception. No Jev verdict is claimed.
High-risk non-CU gating remains opt-in and keeps its local Ask fallback.

The first release can therefore run chat and public search with subscriptions
without making Jev a required paid dependency. Deployments requiring semantic
outbound assessment must configure and validate Jev separately. Neither a
subscription login nor the notice itself supplies that security guarantee.

## Mail, calendar and drive

`run_external` accepts `connector` (an exact app id or display name, such as
`Gmail` or `Google Calendar`). This routes the request to Codex and pages through
`app/list` to verify enabled, accessible account access before starting the task.
Missing access is reported honestly. A subscription does not automatically grant
mailbox access.

The v1 connector path is intended for searching, counting, reading, summarizing,
and composing proposed changes as text. It uses a read-only sandbox without
shell execution, disables destructive app tools, and requests approval for
writes; this adapter does not grant interactive approvals. Sending, deleting,
archiving, creating provider-side drafts and calendar mutations are outside the
v1 acceptance scope. These app controls depend on Codex and provider tool
annotations; they are not a replacement for limited account permissions.
Legacy in-process calendar tools retain their own gates.

## Deferred features and verification

Realtime voice is excluded from the default build. Its status reports
`deferred_to_next_release`; authenticated mint requests return HTTP 501 before
credential lookup or network access, and the WebApp hides the realtime entry.
Voice notes remain available with separately configured STT. The dormant
`experimental-realtime-voice` Cargo feature is for future development and is not
part of first-release acceptance. Heirloom remains opt-in; see [HEIRLOOM.md](HEIRLOOM.md).

Behavior tests use simulated JSON-RPC peers, a real Wasm sandbox with fixed test
output, local HTTP endpoints, and the actual authentication router. They do not
prove live subscription, connector availability or physical-device
behavior. Current results and remaining external acceptance are recorded in
[the verification report](verification-2026-09-27.md).

Protocol references: [Codex app-server](https://developers.openai.com/codex/app-server)
and [configuration reference](https://developers.openai.com/codex/config-reference).
