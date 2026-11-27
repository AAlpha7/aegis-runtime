import { useLocale, useT } from '../i18n/LocaleContext';
import { tf } from '../i18n/catalog';
import type { VNextEmbedderDegraded } from '../api';

/**
 * Sticky, app-shell-level banner that surfaces an embedding-model outage
 * (semantic dedup paused; duplicate proposals/reminders may leak through).
 *
 * Mirrors `LlmDegradedBanner`'s visibility rules exactly:
 *  - Renders only when `degraded` is present (backend says the primary
 *    embedding model is down). Absent/null → renders NOTHING (an advisory
 *    signal must never fabricate an outage — or hide one — from absent data).
 *  - The owner can dismiss it; dismissal is keyed to `degraded.since`. A NEW
 *    outage carries a different `since`, so the banner re-shows.
 *
 * Body copy picks the backup vs unreachable variant from `backup_model`
 * presence: with a backup, embeds still work (weaker model) but dedup is
 * paused; without one, embeds are entirely unavailable.
 *
 * Intentionally pure over its props (no internal persistence) so its
 * show/dismiss/re-show behavior is unit-testable via static render. Reuses the
 * `vnext-llm-degraded-banner` CSS surface so both degraded banners share one
 * styled, theme-aware chrome (no unstyled-new-component trap).
 */
export function EmbedderDegradedBanner({
  degraded,
  dismissedSince,
  onDismiss,
}: {
  degraded: VNextEmbedderDegraded | null | undefined;
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

  const body = degraded.backup_model
    ? tf('embedder.degraded.body.backup', locale, {
        primary: degraded.primary_model,
        backup: degraded.backup_model,
      })
    : tf('embedder.degraded.body.unreachable', locale, {
        primary: degraded.primary_model,
      });

  return (
    <div className="vnext-llm-degraded-banner" role="alert" aria-live="assertive">
      <div className="vnext-llm-degraded-copy">
        <strong>{tr('embedder.degraded.title')}</strong>
        <p>{body}</p>
      </div>
      <button
        type="button"
        className="vnext-secondary vnext-llm-degraded-dismiss"
        onClick={() => onDismiss(degraded.since)}
      >
        {tr('embedder.degraded.dismiss')}
      </button>
    </div>
  );
}
