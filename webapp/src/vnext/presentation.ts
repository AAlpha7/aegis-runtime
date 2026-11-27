import type { ContextualAssistantResponse, OwnerWorkItem } from '../api';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { ownerVisibleNextSafeActionText, resolveWorkCodeText } from './model';
import { formatInUserTz } from './time';

export function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

export function itemMeta(item: OwnerWorkItem, locale: Locale = 'en'): string {
  const closed = isClosedMetaStatus(item.status);
  const kindToken = (item.kind || '').trim().toLowerCase();
  const kind = kindToken === 'task' || kindToken === 'reminder'
      ? t('work.meta.task', locale)
      : kindToken === 'action_draft'
        ? t('work.meta.draft', locale)
        : kindToken === 'receipt'
          ? t('work.meta.receipt', locale)
          : kindToken === 'approval' || kindToken === 'decision' || kindToken === 'tool_approval'
            ? t('work.meta.decision', locale)
            : kindToken === 'goal'
              ? t('work.meta.goal', locale)
              : titleCaseLabel(item.kind);
  const urgency = !closed && item.urgency && item.urgency.toLowerCase() !== 'normal'
    ? tf('work.meta.priority', locale, { level: urgencyLevelLabel(item.urgency, locale) })
    : '';
  const status = ownerMetaStatusLabel(item.status, locale);
  return [urgency, kind, status].filter(Boolean).join(' · ');
}

/** True when the row/detail meta carries a non-normal urgency (amber dot + text). */
export function itemHasPriority(item: OwnerWorkItem): boolean {
  if (isClosedMetaStatus(item.status)) return false;
  const urgency = (item.urgency || '').trim().toLowerCase();
  return Boolean(urgency) && urgency !== 'normal';
}

// Server urgency is an English/severity token (high/urgent/low/s0/s1). Localize
// the known levels. s0/s1 are internal severity codes (both in HIGH_URGENCY) —
// map them to the urgent/high labels so neither locale leaks raw jargon ("S0").
// Anything else falls back to a title-cased raw token.
export function urgencyLevelLabel(value: string, locale: Locale = 'en'): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === 'high' || normalized === 's1') return t('work.meta.level.high', locale);
  if (normalized === 'urgent' || normalized === 's0') return t('work.meta.level.urgent', locale);
  if (normalized === 'low') return t('work.meta.level.low', locale);
  return titleCaseLabel(value);
}

function isClosedMetaStatus(value: string | null | undefined): boolean {
  const normalized = value?.trim().toLowerCase();
  return Boolean(normalized && ['done', 'executed', 'fired', 'cancelled', 'canceled', 'archived', 'deleted', 'completed', 'abandoned'].includes(normalized));
}

function ownerMetaStatusLabel(value: string | null | undefined, locale: Locale = 'en'): string {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return '';
  // tracking/watching: the focus chip already says 跟踪中 — no second (English) copy.
  if (['new', 'active', 'waiting', 'blocked_by_gate', 'paused', 'scheduled', 'tracking', 'watching'].includes(normalized)) return '';
  if (normalized === 'unread') return t('work.meta.unread', locale);
  if (['executed', 'completed', 'done'].includes(normalized)) return t('work.meta.completed', locale);
  if (normalized === 'abandoned') return t('work.meta.followUpClosed', locale);
  if (normalized === 'failed') return t('work.meta.failed', locale);
  if (normalized === 'error') return t('work.meta.error', locale);
  return titleCaseLabel(normalized);
}

const MACHINE_SLUG_LABELS: Record<string, string> = {
  title: 'work.field.title',
  summary: 'work.field.summary',
  sender: 'work.field.sender',
  status: 'work.field.status',
  body: 'work.field.body',
  subject: 'work.field.subject',
  due: 'work.field.due',
  due_date: 'work.field.due',
  attachments: 'work.field.attachments',
  html: 'work.field.body',
  brief: 'work.slug.brief',
  draft: 'work.slug.draft',
  explain: 'work.slug.explain',
};

function isMachineSlug(value: string): boolean {
  return /^[a-z][a-z0-9_]*$/.test(value);
}

/** Label for a context key. Unknown machine slugs stay off a Chinese screen. */
export function ownerFacingFieldLabel(value: string, locale: Locale): string | null {
  const raw = value.trim();
  if (!raw) return null;
  const key = MACHINE_SLUG_LABELS[raw.toLowerCase()];
  if (key) return t(key, locale);
  if (locale === 'zh' && isMachineSlug(raw)) return null;
  return titleCaseLabel(raw);
}

export function compactOwnerList(values: string[], emptyLabel: string, limit = 3, locale: Locale = 'en'): string {
  const cleaned = values.map((value) => ownerFacingFieldLabel(value, locale)).filter((value): value is string => Boolean(value));
  if (cleaned.length === 0) return emptyLabel;
  if (cleaned.length <= limit) return cleaned.join(', ');
  return tf('work.list.andMore', locale, { shown: cleaned.slice(0, limit).join(', '), count: String(cleaned.length - limit) });
}

export function ownerTimeLabel(
  value: string | null | undefined,
  userTimezone?: string | null,
  locale: Locale = 'en',
): string {
  const raw = value?.trim();
  if (!raw) return '';
  // Date-only values have no time-of-day; rendering them in a timezone would
  // shift the displayed day. Return the raw date unchanged.
  if (/^\d{4}-\d{2}-\d{2}$/.test(raw)) return raw;
  // Pass the raw string so formatInUserTz applies the shared wire-timestamp
  // parser (no-offset SQLite datetimes treated as UTC, not browser-local).
  // zh renders "9月17日 20:09" (24h) so the detail matches the list row's
  // 跟踪到 · … line; en keeps the short en-US style.
  const formatted = locale === 'zh'
    ? formatInUserTz(raw, userTimezone, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }, 'zh-CN')
    : formatInUserTz(raw, userTimezone, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }, 'en-US');
  return formatted || raw;
}

export function titleCaseLabel(value: string): string {
  return value
    .split(/[_\s-]+/)
    .filter(Boolean)
    .map((part) => part[0]?.toUpperCase() + part.slice(1).toLowerCase())
    .join(' ');
}

export function actionLabel(item: OwnerWorkItem, locale: Locale = 'en'): string {
  return ownerVisibleNextSafeActionText(item, locale)
    || resolveWorkCodeText('work.actionLabel', item.primary_action?.label_code, locale)
    || item.primary_action?.label
    || t('work.primaryAction.open', locale);
}

export function shortCommandLabel(text: string): string {
  const trimmed = text.trim().replace(/\s+/g, ' ');
  if (trimmed.length <= 40) return `"${trimmed}"`;
  return `"${trimmed.slice(0, 37)}..."`;
}

export function shortSuggestionLabel(text: string): string {
  const trimmed = text.trim().replace(/\s+/g, ' ');
  if (trimmed.length <= 24) return trimmed;
  return `${trimmed.slice(0, 21)}...`;
}

export function assistantResultTitle(
  result: Pick<ContextualAssistantResponse, 'response_kind' | 'visible_answer'>,
  locale: Locale = 'en',
): string {
  const kind = result.response_kind.trim().toLowerCase();
  if (kind === 'working') return t('assistantTitle.working', locale);
  if (kind === 'error') {
    if (/model (provider )?credits|credits are exhausted/i.test(result.visible_answer)) {
      return t('assistantTitle.error.creditsNeeded', locale);
    }
    if (/grok model|model is not available|assistant model is unavailable/i.test(result.visible_answer)) {
      return t('assistantTitle.error.modelSetting', locale);
    }
    if (/did not receive a final answer|did not get a complete answer|temporarily unavailable|connection was interrupted|could not reach/i.test(result.visible_answer)) {
      return t('assistantTitle.error.anotherTry', locale);
    }
    return t('assistantTitle.error.couldNotFinish', locale);
  }
  if (kind === 'draft') return t('assistantTitle.draft', locale);
  if (kind === 'request_clarification') return t('assistantTitle.requestClarification', locale);
  if (kind === 'action_proposal') return t('assistantTitle.actionProposal', locale);
  if (kind === 'action_receipt') return t('assistantTitle.actionReceipt', locale);
  if (kind === 'orchestrated_response' || kind === 'explanation') return t('assistantTitle.answer', locale);
  return t('assistantTitle.answer', locale);
}

export function isActionSuccessMessage(message: string): boolean {
  return [
    // English (server-authored receipts + actionSuccessMessage en values).
    'Done.',
    'Draft saved',
    'Draft deleted',
    'Archived.',
    'Reminder task saved',
    'Task saved',
    'Task updated',
    'Completed action',
    'Approval decision sent',
    'Memory updated',
    // Chinese (localized actionSuccessMessage values — see work.success.* catalog).
    '任务已更新',
    '审批决定',
    '记忆已更新',
    '完成。',
    '已完成',
    '这条提醒已删除',
    '已改到',
    'Marked complete.',
    'Marked this occurrence done.',
    'This reminder was deleted.',
    'Snoozed until ',
  ].some((prefix) => message.startsWith(prefix));
}


/** Stable IDs for owner utility menu entries shared by home + non-home chrome. */
export const OWNER_UTILITY_MENU_IDS = ['status', 'settings'] as const;
export type OwnerUtilityMenuId = (typeof OWNER_UTILITY_MENU_IDS)[number];

/** Everyday owner destinations in the shell overflow menu. */
export function ownerUtilityMenuItemIds(): OwnerUtilityMenuId[] {
  return [...OWNER_UTILITY_MENU_IDS];
}

/** True when a menu id is an everyday utility peer. */
export function isOwnerUtilityMenuId(id: string): boolean {
  return (OWNER_UTILITY_MENU_IDS as readonly string[]).includes(id);
}

/**
 * Everyday primary surfaces in the rail (Chat / Work). The phone bottom nav
 * keeps these two destinations and inserts a permanent centre chat opener.
 * Destination ids must stay identical on home and non-home.
 */
export const OWNER_PRIMARY_NAV_IDS = ['home', 'work'] as const;
export type OwnerPrimaryNavId = (typeof OWNER_PRIMARY_NAV_IDS)[number];

export function ownerPrimaryNavItemIds(): OwnerPrimaryNavId[] {
  return [...OWNER_PRIMARY_NAV_IDS];
}

/** Shared class names for owner shell frost chrome (header + chat topbar). */
export function ownerShellChromeClass(kind: 'header' | 'chat-topbar'): string {
  return kind === 'header' ? 'vnext-header vnext-owner-shell-chrome' : 'vnext-chat-topbar vnext-owner-shell-chrome';
}
