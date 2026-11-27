import type { MouseEvent } from 'react';
import type { OwnerWorkItem } from '../api';
import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { useLocale } from '../i18n/LocaleContext';
import {
  isClosedItem,
  itemIsBlockedByGate,
  itemNeedsDecision,
  itemNeedsPrivateTaskAccess,
  laneForItem,
  ownerVisibleWorkTitle,
  ownerVisibleWorkCue,
  ownerVisibleWorkTiming,
} from './model';
import { classNames, itemHasPriority, itemMeta } from './presentation';
import { useUserTimezone } from './UserTimezoneContext';

export function WorkItemRow({
  item,
  active = false,
  compact = false,
  onClick,
}: {
  item: OwnerWorkItem;
  active?: boolean;
  compact?: boolean;
  /** Receives the click event so the caller can record where the row sat at tap time. */
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
}) {
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const timing = ownerVisibleWorkTiming(item, userTimezone, locale);
  const titleText = ownerVisibleWorkTitle(item, undefined, undefined, locale);
  const summaryText = ownerVisibleWorkCue(item, undefined, userTimezone, locale);
  const metaText = itemMeta(item, locale);
  return (
    <button type="button" className={classNames('vnext-item-row', active && 'active', compact && 'compact')} onClick={onClick}>
      <span className="vnext-item-top">
        <span className={classNames('vnext-item-kind', (itemNeedsDecision(item) || itemIsBlockedByGate(item)) && 'warn')}>{focusLabel(item, locale)}</span>
      </span>
      <span className="vnext-item-title" title={titleText}>{titleText}</span>
      {summaryText ? <span className="vnext-item-summary" title={summaryText}>{summaryText}</span> : null}
      {(metaText || timing) && (
        // One meta line: "kind · timing" (owner 2026-09-16 desktop pass — the
        // kind and the tracked-until date used to take a line each).
        <span className="vnext-item-foot">
          {metaText ? <span className={classNames('vnext-item-meta', itemHasPriority(item) && 'is-priority')}>{metaText}</span> : null}
          {timing && <span className="vnext-item-timing">{timing}</span>}
        </span>
      )}
    </button>
  );
}

function focusLabel(item: OwnerWorkItem, locale: Locale): string {
  if (isClosedItem(item)) return t('work.focusChip.handled', locale);
  if (itemIsBlockedByGate(item)) return t('work.focusChip.paused', locale);
  if (itemNeedsDecision(item)) return t('work.focusChip.decision', locale);
  if (itemNeedsPrivateTaskAccess(item)) return t('work.focusChip.needsDetail', locale);
  const lane = laneForItem(item);
  if (lane === 'follow_up') return t('work.focusChip.tracking', locale);
  if (item.status?.toLowerCase() === 'scheduled') return t('work.focusChip.comingUp', locale);
  return t('work.focusChip.ready', locale);
}
