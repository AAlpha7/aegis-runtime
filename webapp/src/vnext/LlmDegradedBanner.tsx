import { useLocale, useT } from '../i18n/LocaleContext';
import { tf } from '../i18n/catalog';
import type { VNextLlmDegraded } from '../api';

/**
 * Sticky, app-shell-level banner that surfaces an LLM-provider outage.
 *
 * Visibility rules (owner-controlled, outage-keyed):
 *  - Renders only when `degraded` is present (backend says the provider is down).
 *    When the backend clears it (next status fetch has `llm_degraded` null), the
 *    parent passes `degraded == null` and the banner disappears on its own.
 *  - The owner can dismiss it. Dismissal is keyed to `degraded.since` (the outage
 *    start timestamp), passed back to the parent so it can be persisted. A NEW
 *    outage carries a different `since`, so `dismissedSince !== degraded.since`
 *    and the banner RE-SHOWS even though the owner dismissed the previous one.
 *
 * The component is intentionally pure over its props (no internal persistence),
 * so its show/dismiss/re-show behavior is unit-testable via static render.
 */
export function LlmDegradedBanner({
  degraded,
  dismissedSince,
  onDismiss,
}: {
  degraded: VNextLlmDegraded | null | undefined;
  /** The `since` value the owner last dismissed (from persisted state). */
  dismissedSince: string | null;
  /** Invoked with the current outage `since` when the owner dismisses. */
  onDismiss: (since: string) => void;
}) {
  const { locale } = useLocale();
  const tr = useT();

  // Nothing to show when healthy, or when the owner already dismissed THIS outage.
  if (!degraded) return null;
  if (dismissedSince !== null && dismissedSince === degraded.since) return null;

  return (
    <div className="vnext-llm-degraded-banner" role="alert" aria-live="assertive">
      <div className="vnext-llm-degraded-copy">
        <strong>{tr('provider.degraded.title')}</strong>
        <p>{tf('provider.degraded.body', locale, { reason: degraded.reason })}</p>
      </div>
      <button
        type="button"
        className="vnext-secondary vnext-llm-degraded-dismiss"
        onClick={() => onDismiss(degraded.since)}
      >
        {tr('provider.degraded.dismiss')}
      </button>
    </div>
  );
}
