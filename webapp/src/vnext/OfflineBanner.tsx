import { useT } from '../i18n/LocaleContext';

/**
 * Top-of-shell notice while the browser reports no connectivity
 * (`useOnlineStatus` → false). No dismiss: it disappears on reconnect. Reuses
 * the `vnext-llm-degraded-banner` surface so it occupies the single reserved
 * banner row; while offline it replaces the runtime banners, whose data cannot
 * refresh anyway.
 */
export function OfflineBanner({ offline }: { offline: boolean }) {
  const tr = useT();
  if (!offline) return null;
  return (
    <div className="vnext-llm-degraded-banner vnext-offline-banner" role="status" aria-live="polite" data-offline-banner="true">
      <div className="vnext-llm-degraded-copy">
        <strong>{tr('app.offline.banner')}</strong>
      </div>
    </div>
  );
}
