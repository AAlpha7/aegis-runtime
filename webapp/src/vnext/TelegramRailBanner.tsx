import { useLocale, useT } from '../i18n/LocaleContext';
import { tf } from '../i18n/catalog';
import { nowUnixSeconds, telegramRailBannerState } from './statusRails';
import type { VNextTelegramRail } from './statusRails';

/**
 * App-shell banner for the Telegram rail (incident 2026-09-15: the bot token
 * was revoked and the console never said a word).
 *
 * Two tones, one slot:
 *  - `invalid_token` → an alert (role="alert") naming the fix: a new
 *    TELOXIDE_TOKEN in .env and a restart. Shown immediately.
 *  - `unreachable` past the ten-minute blip window → a softer status line
 *    (role="status", polite) saying notifications may be delayed.
 *
 * Dismissal mirrors `LlmDegradedBanner`: keyed to the episode start
 * (`since_unix`) and passed back to the parent for persistence, so a new
 * episode re-shows even after the owner dismissed the previous one. Pure over
 * its props; `nowUnix` is injectable so the render test is deterministic.
 * Reuses the `vnext-llm-degraded-banner` surface (the mobile layout reserves
 * exactly one banner row keyed on that class).
 */
export function TelegramRailBanner({
  rail,
  dismissedSinceUnix,
  onDismiss,
  nowUnix = nowUnixSeconds(),
}: {
  rail: VNextTelegramRail | null | undefined;
  /** The `since_unix` the owner last dismissed (from persisted state). */
  dismissedSinceUnix: number | null;
  /** Invoked with the current episode's `since_unix` when the owner dismisses. */
  onDismiss: (sinceUnix: number) => void;
  nowUnix?: number;
}) {
  const { locale } = useLocale();
  const tr = useT();
  const banner = telegramRailBannerState(rail, dismissedSinceUnix, nowUnix);
  if (!banner) return null;

  const alert = banner.kind === 'invalid_token';
  return (
    <div
      className={alert
        ? 'vnext-llm-degraded-banner vnext-telegram-rail-banner'
        : 'vnext-llm-degraded-banner vnext-telegram-rail-banner soft'}
      role={alert ? 'alert' : 'status'}
      aria-live={alert ? 'assertive' : 'polite'}
      data-telegram-rail={banner.kind}
    >
      <div className="vnext-llm-degraded-copy">
        <strong>{alert ? tr('telegram.rail.invalidToken.title') : tr('telegram.rail.unreachable.title')}</strong>
        <p>
          {alert
            ? tr('telegram.rail.invalidToken.body')
            : tf('telegram.rail.unreachable.body', locale, { minutes: String(banner.minutes) })}
        </p>
      </div>
      <button
        type="button"
        className="vnext-secondary vnext-llm-degraded-dismiss"
        onClick={() => onDismiss(banner.sinceUnix)}
      >
        {tr('telegram.rail.dismiss')}
      </button>
    </div>
  );
}
