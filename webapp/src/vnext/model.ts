import { catalogHas, t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { formatInUserTz, parseWireMomentMs } from './time';

export type VNextLaneId = 'now' | 'decide' | 'follow_up' | 'backlog' | 'done';
export type VNextCommandScope = 'item' | 'general';
export const GENERAL_ASSISTANT_FOCUS_ID = 'assistant:general';

export interface VNextWorkAction {
  label: string;
  /**
   * Optional stable structural code for `label` (i18n catalog key
   * `work.actionLabel.<code>`). Absent/unknown codes fall back to `label`.
   */
  label_code?: string | null;
  action_kind: string;
  route?: string | null;
  requires_gate: boolean;
  requires_mfa: boolean;
}

export interface VNextWorkItem {
  id: string;
  kind: string;
  source_kind?: string;
  title: string;
  summary: string;
  status: string;
  urgency: string;
  updated_at: string;
  due_at?: string | null;
  blocked_by?: string | null;
  primary_action?: VNextWorkAction | null;
  secondary_actions: VNextWorkAction[];
}

export interface VNextWorkQueue {
  sources: { source: string; status: string; reason?: string | null }[];
  items: VNextWorkItem[];
}

export interface VNextSourceStatusView {
  label: string;
  statusLabel: string;
  healthy: boolean;
  reason?: string | null;
  nextStep?: string | null;
}

export interface VNextLane {
  id: VNextLaneId;
  title: string;
  subtitle: string;
  items: VNextWorkItem[];
  collapsed: boolean;
}

export interface VNextHome {
  brief: string;
  now: VNextWorkItem[];
  decisions: VNextWorkItem[];
  followUps: VNextWorkItem[];
  sourceIssueCount: number;
  sourceIssueSummary: string | null;
}

export interface VNextHomeSignal {
  id: string;
  label: string;
  summary: string;
  tone: 'neutral' | 'warn' | 'ok';
  targetItemId?: string;
}

export type VNextItemActionId =
  | 'goal.abandon'
  | 'task.complete'
  | 'task.skip_occurrence'
  | 'tracking.cancel'
  | 'task.snooze_tomorrow'
  | 'knowledge.done'
  | 'knowledge.snooze_tomorrow'
  | 'approval.approve'
  | 'approval.reject'
  | 'memory.confirm'
  | 'memory.pin'
  | 'memory.unpin'
  | 'memory.mark_wrong'
  | 'memory.delete';

export interface VNextItemAction {
  id: VNextItemActionId;
  label: string;
  confirmLabel: string;
  receiptHint: string;
  tone: 'normal' | 'destructive';
}

export interface VNextCommandSuggestion {
  id: string;
  label: string;
  prompt: string;
}

type OwnerCueWorkItem = VNextWorkItem & {
  title_code?: string | null;
  next_safe_action?: string;
  next_safe_action_code?: string | null;
  owner_visible_reason?: string;
  owner_visible_reason_code?: string | null;
  route_reason?: string;
  source?: string;
  source_kind?: string;
};

const CLOSED_STATUSES = new Set(['done', 'executed', 'fired', 'cancelled', 'canceled', 'archived', 'deleted', 'completed', 'abandoned']);
const DECISION_STATUSES = new Set(['needs_decision', 'waiting_user', 'approval_required', 'needs_approval']);
const FOLLOW_UP_STATUSES = new Set(['waiting', 'tracking', 'scheduled', 'deferred', 'snoozed']);
const BACKLOG_STATUSES = new Set(['backlog', 'queued_later', 'someday']);
const HIGH_URGENCY = new Set(['urgent', 'high', 's0', 's1']);
const AUTHORITY_BASIS_LABEL_KEYS: Record<string, string> = {
  active_mandate: 'work.basis.activeMandate',
  direct_user_request: 'work.basis.directRequest',
  direct_owner_request: 'work.basis.directRequest',
  direct_request: 'work.basis.directRequest',
  manual_review: 'work.basis.manualReview',
  owner_review: 'work.basis.manualReview',
  system_health: 'work.basis.systemHealth',
};
const SOURCE_LABELS: Record<string, string> = {
  hercules: 'Hercules',
  action_ledger: 'Drafts and receipts',
  alerts: 'System alerts',
  calendar: 'Calendar',
  goals: 'Goals',
  hitl: 'Owner approvals',
  knowledge: 'Waiting items',
  mandates: 'Standing instructions',
  tasks: 'Tasks and reminders',
};
// Display-only localization keyed by the English SOURCE_LABELS value (the
// identity used for filtering/matching). Display sites call this; the English
// label stays the key so equality checks (e.g. Settings' visibleSources filter)
// keep working. Unknown labels (server-derived) pass through untranslated.
const SOURCE_LABEL_DISPLAY_KEYS: Record<string, string> = {
  'Drafts and receipts': 'status.sourceLabel.DraftsAndReceipts',
  'System alerts': 'status.sourceLabel.SystemAlerts',
  'Calendar': 'status.sourceLabel.Calendar',
  'Goals': 'status.sourceLabel.Goals',
  'Owner approvals': 'status.sourceLabel.OwnerApprovals',
  'Waiting items': 'status.sourceLabel.WaitingItems',
  'Standing instructions': 'status.sourceLabel.StandingInstructions',
  'Tasks and reminders': 'status.sourceLabel.TasksAndReminders',
  'Memory': 'status.sourceLabel.Memory',
};
export function localizedSourceLabel(label: string, locale: Locale = 'en'): string {
  const key = SOURCE_LABEL_DISPLAY_KEYS[label];
  return key ? t(key, locale) : label;
}
const QUIET_HOME_BRIEF_KEY = 'work.quietHomeBrief';
const PRIVATE_TASK_REVIEW_TITLE_KEY = 'work.title.privateTaskReview';

function normalized(value: string | null | undefined): string {
  return value?.trim().toLowerCase() ?? '';
}

export function isClosedItem(item: Pick<VNextWorkItem, 'status' | 'urgency'>): boolean {
  return CLOSED_STATUSES.has(normalized(item.status)) || normalized(item.urgency) === 'silent';
}

export function itemNeedsDecision(item: Pick<VNextWorkItem, 'kind' | 'status'>): boolean {
  const kind = normalized(item.kind);
  const status = normalized(item.status);
  return kind === 'approval'
    || DECISION_STATUSES.has(status);
}

export function itemIsBlockedByGate(item: Pick<VNextWorkItem, 'status'>): boolean {
  return normalized(item.status) === 'blocked_by_gate';
}

export function laneForItem(item: VNextWorkItem): VNextLaneId {
  const status = normalized(item.status);
  if (isClosedItem(item)) return 'done';
  if (itemNeedsDecision(item)) return 'decide';
  if (status === 'blocked_by_gate') {
    return 'decide';
  }
  if (item.blocked_by && ['task', 'reminder'].includes(normalized(item.kind))) {
    return 'follow_up';
  }
  if (FOLLOW_UP_STATUSES.has(status)) {
    if (followUpNeedsTodayAttention(item)) return 'now';
    return 'follow_up';
  }
  if (BACKLOG_STATUSES.has(status) || normalized(item.urgency) === 'low') return 'backlog';
  return 'now';
}

function followUpNeedsTodayAttention(item: VNextWorkItem): boolean {
  const status = normalized(item.status);
  if (status === 'failed' || status === 'dead_letter' || status === 'needs_recovery') return true;
  if (status !== 'scheduled') return false;
  if (!item.due_at) return false;
  // Wire value may be a SQLite no-offset `YYYY-MM-DD HH:MM:SS` (UTC) — parse via
  // parseWireMomentMs so the 48h lane threshold is timezone-stable, not browser-local.
  const due = parseWireMomentMs(item.due_at);
  if (due === null) return false;
  return due <= Date.now() + 48 * 60 * 60 * 1000;
}

function attentionScore(item: VNextWorkItem): number {
  if (itemNeedsDecision(item)) return 500;
  if (HIGH_URGENCY.has(normalized(item.urgency))) return 400;
  if (laneForItem(item) === 'follow_up') return 300;
  if (laneForItem(item) === 'now') return 250;
  if (laneForItem(item) === 'backlog') return 100;
  return 0;
}

function timeValue(value: string | null | undefined): number {
  // UTC-stable parse of wire timestamps (incl. SQLite no-offset) for sort order.
  const parsed = parseWireMomentMs(value);
  return parsed ?? Number.POSITIVE_INFINITY;
}

export function sortWorkItems<T extends VNextWorkItem>(items: T[]): T[] {
  return [...items].sort((a, b) => {
    const score = attentionScore(b) - attentionScore(a);
    if (score !== 0) return score;
    const due = timeValue(a.due_at) - timeValue(b.due_at);
    if (due !== 0) return due;
    const updated = timeValue(b.updated_at) - timeValue(a.updated_at);
    if (updated !== 0) return updated;
    return a.title.localeCompare(b.title);
  });
}

export function buildVNextLanes<T extends VNextWorkItem>(items: T[], locale: Locale = 'en'): Array<Omit<VNextLane, 'items'> & { items: T[] }> {
  const grouped: Record<VNextLaneId, T[]> = {
    now: [],
    decide: [],
    follow_up: [],
    backlog: [],
    done: [],
  };

  for (const item of sortWorkItems(items)) {
    grouped[laneForItem(item)].push(item);
  }

  return [
    {
      id: 'decide',
      title: t('work.lane.decide.title', locale),
      subtitle: t('work.lane.decide.subtitle', locale),
      items: grouped.decide,
      collapsed: grouped.decide.length === 0,
    },
    {
      id: 'now',
      title: t('work.lane.now.title', locale),
      subtitle: t('work.lane.now.subtitle', locale),
      items: grouped.now,
      collapsed: grouped.now.length === 0,
    },
    {
      id: 'follow_up',
      title: t('work.lane.followUp.title', locale),
      subtitle: t('work.lane.followUp.subtitle', locale),
      items: grouped.follow_up,
      collapsed: grouped.follow_up.length === 0,
    },
    {
      id: 'backlog',
      title: t('work.lane.backlog.title', locale),
      subtitle: t('work.lane.backlog.subtitle', locale),
      items: grouped.backlog,
      collapsed: true,
    },
    {
      id: 'done',
      title: t('work.lane.done.title', locale),
      subtitle: t('work.lane.done.subtitle', locale),
      items: grouped.done,
      collapsed: true,
    },
  ];
}

export function hasWorkItems(lanes: Array<Pick<VNextLane, 'items'>>): boolean {
  return lanes.some((lane) => lane.items.length > 0);
}

export function buildVNextHome<T extends VNextWorkItem>(queue: VNextWorkQueue & { items: T[] }, locale: Locale = 'en'): Omit<VNextHome, 'now' | 'decisions' | 'followUps'> & {
  now: T[];
  decisions: T[];
  followUps: T[];
} {
  const active = sortWorkItems(queue.items).filter((item) => !isClosedItem(item));
  const decisions = active.filter((item) => laneForItem(item) === 'decide').slice(0, 3);
  const followUps = active.filter((item) => laneForItem(item) === 'follow_up').slice(0, 3);
  const now = active.filter((item) => laneForItem(item) === 'now').slice(0, 3);
  const sourceIssues = queue.sources.filter(sourceNeedsOwnerAttention).map((s) => sourceStatusView(s, locale));
  const sourceIssueCount = sourceIssues.length;
  const sourceIssueSummary = sourceIssues.length === 0
    ? null
    : (() => {
        const label = localizedSourceLabel(sourceIssues[0].label, locale);
        const reason = sourceIssues[0].reason || sourceIssues[0].statusLabel;
        if (sourceIssues.length === 1) return tf('work.source.issueSummary.single', locale, { label, reason });
        if (sourceIssues.length === 2) return tf('work.source.issueSummary.moreOne', locale, { label, reason });
        return tf('work.source.issueSummary.moreMany', locale, { label, reason, count: String(sourceIssues.length - 1) });
      })();
  const lead = decisions[0] ?? now[0];
  const brief = lead ? homeBriefForItem(lead, locale) : t(QUIET_HOME_BRIEF_KEY, locale);

  return { brief, now, decisions, followUps, sourceIssueCount, sourceIssueSummary };
}

function compactTitle(value: string, fallback = 'this item', maxChars = 96): string {
  const trimmed = ownerVisibleTitleText(value);
  if (!trimmed) return fallback;
  if ([...trimmed].length <= maxChars) return trimmed;
  return `${[...trimmed].slice(0, maxChars).join('')}...`;
}

export function ownerVisibleItemTitle(value: string, fallback = 'this item', maxChars = 96): string {
  return compactTitle(value, fallback, maxChars);
}

/**
 * Resolves a backend structural code (`owner_visible_reason_code`,
 * `next_safe_action_code`, `label_code`) against the i18n catalog.
 *
 * Returns the localized text for a KNOWN code, or `null` for an absent or
 * unknown code — callers must then fall back to the backend prose exactly as
 * before (never blank, never the `[MISSING:…]` sentinel). No prose keyword
 * matching happens here: resolution is purely `prefix.<code>` key membership.
 *
 * A code may be written `"<namespace>:<code>"` to resolve under
 * `work.<namespace>.<code>` instead of the caller's default `prefix`. Allowed
 * namespaces: `decide`, `reasonCode`, `nextCode`, `actionLabel`. Unknown
 * namespace or empty half → `null` (caller falls back to the server string).
 */
export function resolveWorkCodeText(
  prefix: 'work.reasonCode' | 'work.nextCode' | 'work.actionLabel' | 'work.decide',
  code: string | null | undefined,
  locale: Locale,
): string | null {
  const trimmed = typeof code === 'string' ? code.trim() : '';
  if (!trimmed) return null;
  const allowedNamespaces = ['decide', 'reasonCode', 'nextCode', 'actionLabel'] as const;
  let key: string;
  if (trimmed.includes(':')) {
    const colon = trimmed.indexOf(':');
    const namespace = trimmed.slice(0, colon);
    const bare = trimmed.slice(colon + 1);
    if (!namespace || !bare || !(allowedNamespaces as readonly string[]).includes(namespace)) {
      return null;
    }
    key = `work.${namespace}.${bare}`;
  } else {
    key = `${prefix}.${trimmed}`;
  }
  return catalogHas(key) ? t(key, locale) : null;
}

/**
 * The item's `next_safe_action` for display: the localized text of a known
 * `next_safe_action_code`, else the backend prose (empty string when neither
 * exists — same falsy behavior as reading `item.next_safe_action` directly, so
 * `ownerVisibleNextSafeActionText(item, locale) || fallback` chains keep
 * working).
 */
export function ownerVisibleNextSafeActionText(
  item: { next_safe_action?: string | null; next_safe_action_code?: string | null },
  locale: Locale = 'en',
): string {
  return resolveWorkCodeText('work.nextCode', item.next_safe_action_code, locale)
    ?? item.next_safe_action
    ?? '';
}

export function ownerVisiblePrimaryActionLabel(
  item: Pick<VNextWorkItem, 'kind' | 'primary_action'> & { source?: string; source_kind?: string },
  fallback = 'Open',
  maxChars = 40,
  locale: Locale = 'en',
): string {
  // Structural twin of the prose special-case below: an approval item's
  // "Review approval" button keeps rendering as the softer "Review request".
  if (isApprovalItem(item) && item.primary_action?.label_code === 'review_approval') {
    return t('work.primaryAction.reviewRequest', locale);
  }
  const coded = resolveWorkCodeText('work.actionLabel', item.primary_action?.label_code, locale);
  if (coded) return ownerVisibleItemTitle(coded, fallback, maxChars);
  const raw = item.primary_action?.label || fallback;
  if (isApprovalItem(item) && raw.trim().toLowerCase() === ['review', 'approval'].join(' ')) {
    return t('work.primaryAction.reviewRequest', locale);
  }
  return ownerVisibleItemTitle(raw, fallback, maxChars);
}

/**
 * The item's title for display: the localized text of a known `title_code`
 * (static server titles such as approvals), else the server prose. Every
 * surface that renders an item title must go through this, or a zh screen ends
 * up mixing a localized sibling with a raw English title.
 */
export function ownerVisibleCodedTitle(
  item: { title?: string | null; title_code?: string | null },
  locale: Locale = 'en',
): string {
  return resolveWorkCodeText('work.decide', item.title_code, locale) ?? item.title ?? '';
}

export function ownerVisibleWorkTitle(item: OwnerCueWorkItem, fallback?: string, maxChars = 96, locale: Locale = 'en'): string {
  // When no explicit fallback is given, resolve a LOCALIZED default so empty-
  // title items don't render the English literal in zh. en === 'this item' so
  // the en output stays byte-identical to the prior default.
  const fb = fallback ?? t('work.title.thisItem', locale);
  if (itemIsBlockedByGate(item)) return blockedAutomationTitle(item, fallback, maxChars, locale);
  // A static server title (approvals) carries a structural code; localize it
  // like every other coded string, and keep the server prose when unknown.
  const title = ownerVisibleItemTitle(ownerVisibleCodedTitle(item, locale), fb, maxChars);
  if (isPrivateTaskPlaceholder(item, title)) return t(PRIVATE_TASK_REVIEW_TITLE_KEY, locale);
  return title;
}

export function blockedAutomationTitle(item: OwnerCueWorkItem, fallback?: string, maxChars = 96, locale: Locale = 'en'): string {
  // Localized default fallback (en === 'Paused automation', byte-identical).
  const fb = fallback ?? t('work.title.pausedAutomationFallback', locale);
  if (blockedAutomationKind(item) === 'web_search') {
    const target = blockedAutomationTarget(item, maxChars - 16);
    return target
      ? ownerVisibleItemTitle(tf('work.title.searchPaused', locale, { target }), t('work.title.webSearchPaused', locale), maxChars)
      : t('work.title.webSearchPaused', locale);
  }
  // Last sibling title reader: go through the same coded-title rule so a
  // future projection that sets title_code on a gated item cannot leak English.
  return ownerVisibleItemTitle(ownerVisibleCodedTitle(item, locale) || fb, fb, maxChars);
}

export function blockedAutomationWhatHappened(item: OwnerCueWorkItem, locale: Locale = 'en'): string {
  if (blockedAutomationKind(item) === 'web_search') {
    const target = blockedAutomationTarget(item, 140);
    return target
      ? tf('work.blocked.searchWhatHappened', locale, { target })
      : t('work.blocked.searchWhatHappenedNoTarget', locale);
  }
  return ownerVisibleSensitiveStepText(
    item.summary || item.owner_visible_reason || item.blocked_by,
    t('work.blocked.genericWhatHappened', locale),
    190,
  );
}

export function blockedAutomationDecisionText(item: OwnerCueWorkItem, locale: Locale = 'en'): string {
  const actions = Array.isArray(item.secondary_actions) ? item.secondary_actions : [];
  const hasApproval = actions.some((action) => (
    normalized(action.action_kind) === 'approval.approve' || normalized(action.action_kind) === 'approval.reject'
  ));
  if (hasApproval) return t('work.blocked.decisionApproval', locale);
  return t('work.blocked.decisionExpired', locale);
}

function blockedAutomationKind(item: OwnerCueWorkItem): 'web_search' | 'generic' {
  const haystack = [
    item.title,
    item.summary,
    item.blocked_by,
    item.next_safe_action,
    item.owner_visible_reason,
    item.source,
    item.source_kind,
    item.kind,
  ].filter(Boolean).join(' ').toLowerCase();
  if (haystack.includes('web_search') || haystack.includes('web search') || haystack.includes('搜索')) return 'web_search';
  return 'generic';
}

function blockedAutomationTarget(item: OwnerCueWorkItem, maxChars = 120): string {
  const candidates = [
    item.title,
    item.summary,
    item.next_safe_action,
    item.blocked_by,
  ];
  for (const candidate of candidates) {
    const title = ownerVisibleTitleText(candidate || '');
    if (!title || isGenericBlockedAutomationTarget(title)) continue;
    return ownerVisibleItemTitle(title.replace(/^Search:\s*/i, ''), '', maxChars);
  }
  return '';
}

function isGenericBlockedAutomationTarget(value: string): boolean {
  const text = value.trim().toLowerCase();
  if (!text) return true;
  return [
    'web search paused',
    'aegis paused before continuing',
    'aegis paused this automation before continuing',
    'aegis was about to run a web search for a scheduled request',
    'review before continuing',
    'continue this paused automation',
    'sensitive step',
  ].some((phrase) => text.includes(phrase));
}

export function ownerVisibleSensitiveStepText(value: string | null | undefined, fallback?: string, maxChars = 190, locale: Locale = 'en'): string {
  // Localized default (en === the prior literal, byte-identical) so an empty
  // value with no explicit fallback doesn't leak English in zh.
  const fb = fallback ?? t('work.sensitiveStepFallback', locale);
  const text = ownerVisibleItemTitle(value || fb, fb, maxChars);
  if (!text) return '';
  return text
    .replace(/\bprotected action\b/gi, 'sensitive step')
    .replace(/\bstanding instruction\b/gi, 'automation');
}

export function ownerVisibleWorkCue(item: OwnerCueWorkItem, maxChars = 140, userTimezone?: string | null, locale: Locale = 'en'): string {
  if (isClosedItem(item)) {
    return t('work.cue.handled', locale);
  }
  if (itemIsBlockedByGate(item)) {
    return ownerVisibleItemTitle(blockedAutomationWhatHappened(item, locale), t('work.cue.pausedFallback', locale), maxChars);
  }
  const summary = ownerVisibleSensitiveStepText(item.summary || '', '', maxChars);
  if (summary && !isGenericWorkSummary(summary)) return summary;
  if (itemNeedsDecision(item)) {
    return ownerVisibleItemTitle(
      ownerVisibleNextSafeActionText(item, locale) || ownerVisiblePrimaryActionLabel(item, t('work.cue.waitingForDecision', locale), maxChars, locale),
      t('work.cue.waitingForDecision', locale),
      maxChars,
    );
  }
  if (laneForItem(item) === 'follow_up') {
    // With a due time the row's timing line already says 跟踪到 · <time>;
    // repeating it as a sentence was the second copy on every card.
    return item.due_at ? '' : t('work.cue.trackingQuietly', locale);
  }
  if (item.status?.toLowerCase() === 'scheduled') {
    return item.due_at
      ? tf('work.cue.scheduledFor', locale, { time: workTimeLabel(item.due_at, userTimezone, locale) })
      : t('work.cue.scheduledNoAction', locale);
  }
  if (isPrivateTaskPlaceholder(item)) {
    return t('work.cue.openPrivateTask', locale);
  }
  return ownerVisibleItemTitle(ownerVisibleNextSafeActionText(item, locale) || ownerVisiblePrimaryActionLabel(item, t('work.cue.openWhenUseful', locale), maxChars, locale), t('work.cue.openWhenUseful', locale), maxChars);
}

export function ownerVisibleWorkTiming(item: VNextWorkItem, userTimezone?: string | null, locale: Locale = 'en'): string {
  const due = workTimeLabel(item.due_at, userTimezone, locale);
  if (!due) return '';
  if (laneForItem(item) === 'follow_up') return `${t('work.timing.watching', locale)} · ${due}`;
  if (item.status?.toLowerCase() === 'scheduled') return `${t('work.timing.scheduled', locale)} · ${due}`;
  return `${t('work.timing.due', locale)} · ${due}`;
}

export function ownerVisibleReason(item: { owner_visible_reason?: string | null; route_reason?: string | null }): string {
  const raw = ownerVisibleSensitiveStepText(item.owner_visible_reason || item.route_reason || '', '', 180);
  if (!raw || isGenericOwnerReason(raw)) return '';
  return raw;
}

/**
 * Reason codes the cue deliberately renders as EMPTY — the structural twin of
 * the `isGenericOwnerReason` prose filter below ("this is in your queue"-style
 * reasons carry no information, so the cue suppresses them). Keep the two in
 * lockstep: a code belongs here iff its backend prose is caught by that filter.
 */
const GENERIC_REASON_CODES = new Set([
  'rule_matched_source', // 'basic source information'
  'task_owner_review', // 'owner review'
  'task_active_owner_work', // 'active owner work'
  'task_tracking_loop', // 'open follow-up loop'
  'blocked_generic_pause', // 'owner review'
]);

export function ownerVisibleReasonCue(item: { owner_visible_reason?: string | null; owner_visible_reason_code?: string | null; route_reason?: string | null; status?: string | null; urgency?: string | null }, locale: Locale = 'en'): string {
  if (
    item.status !== undefined || item.urgency !== undefined
  ) {
    const status = normalized(item.status);
    const urgency = normalized(item.urgency);
    if (CLOSED_STATUSES.has(status) || urgency === 'silent') return '';
  }
  const codeRaw = typeof item.owner_visible_reason_code === 'string' ? item.owner_visible_reason_code.trim() : '';
  if (codeRaw && GENERIC_REASON_CODES.has(codeRaw)) return '';
  const coded = resolveWorkCodeText('work.reasonCode', codeRaw, locale);
  if (coded) return ownerVisibleItemTitle(coded, coded, 72);
  const reason = ownerVisibleReason(item);
  if (!reason) return '';
  const normalizedReason = reason.toLowerCase();
  if (normalizedReason.includes('waiting on') || normalizedReason.includes('waiting for')) {
    return ownerVisibleItemTitle(reason, t('work.reason.waitingDependency', locale), 72);
  }
  if (normalizedReason.includes('post-gate') || normalizedReason.includes('sanitized render')) {
    return '';
  }
  if (normalizedReason.includes('blocked') || normalizedReason.includes('gate')) {
    return t('work.reason.needsApprovalAccess', locale);
  }
  if (normalizedReason.includes('mandate')) {
    return t('work.reason.fromMandate', locale);
  }
  return ownerVisibleItemTitle(reason, reason, 72);
}

function workTimeLabel(value: string | null | undefined, userTimezone?: string | null, locale: Locale = 'en'): string {
  const raw = value?.trim();
  if (!raw) return '';
  // Date-only values (e.g. goal target_date / waiting due_date `2026-05-15`)
  // have no time-of-day; rendering them in a timezone would shift the day
  // (midnight UTC → prior evening in the Americas). Return the raw date.
  if (/^\d{4}-\d{2}-\d{2}$/.test(raw)) return raw;
  // Pass the raw string so formatInUserTz applies the shared wire-timestamp
  // parser (no-offset SQLite datetimes treated as UTC, not browser-local).
  // zh: "9月13日 20:09" (24h); en keeps the short en-US style.
  const formatted = locale === 'zh'
    ? formatInUserTz(raw, userTimezone, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }, 'zh-CN')
    : formatInUserTz(raw, userTimezone, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }, 'en-US');
  return formatted || raw;
}

export function isGenericWorkSummary(summary: string): boolean {
  const normalizedSummary = summary.toLowerCase();
  return [
    // Server canned tracking/backlog sentences (zh + en): every follow-up card
    // repeated them under its title (2026-09-13 Work walk).
    '跟进中。我会把它留在视线里',
    'tracking item. aegis is keeping this visible',
    '还没有里程碑清单',
    'no milestone checklist is attached',
    'task to review',
    'open details',
    'receipt-backed changes',
    'normal priority',
    'high priority',
    'task mode',
    'manual mode',
    'scheduled mode',
    'notification mode',
    'private task title',
    'private title and body',
    'task title and body stay private',
    'tracking item',
    'keeping this visible as a follow-up loop',
  ].some((phrase) => normalizedSummary.includes(phrase));
}

function isPrivateTaskPlaceholder(item: OwnerCueWorkItem, title = item.title || ''): boolean {
  if (normalized(item.kind) !== 'task') return false;
  const haystack = `${title} ${item.summary || ''} ${item.next_safe_action || ''}`;
  return /private high-priority task|private task title\/body|task title and body stay private/i.test(haystack);
}

function isGenericOwnerReason(reason: string): boolean {
  const normalizedReason = reason.toLowerCase();
  return [
    'active owner work',
    "owner's work queue",
    'owner review',
    'surface_card',
    'basic source information',
    'aegis surfaced this task',
    'aegis surfaced this for',
    'this task is in the owner',
    'open follow-up loop',
    'part of an open follow-up loop',
  ].some((phrase) => normalizedReason.includes(phrase));
}

function ownerVisibleTitleText(value: string): string {
  const collapsed = value.trim().replace(/\s+/g, ' ');
  if (!collapsed || collapsed.includes('://')) return collapsed;
  const ownerTitle = humanizeToolInstructionTitle(collapsed) ?? collapsed;
  const chars = [...ownerTitle];
  let out = '';
  for (let index = 0; index < chars.length; index += 1) {
    const ch = chars[index];
    if (ch === '+') {
      const adjacentPlus = chars[index - 1] === '+' || chars[index + 1] === '+';
      if (!adjacentPlus) {
        if (chars[index + 1] === undefined) continue;
        out = out.replace(/\s+$/u, '');
        if (out && !out.endsWith('/')) out += ' / ';
        continue;
      }
    }
    out += ch;
  }
  return prettifyMachineTokens(out.trim().replace(/\s+/g, ' '));
}

function humanizeToolInstructionTitle(value: string): string | null {
  const chineseToolRequest = value.match(/^使用\s+([A-Za-z][A-Za-z0-9_]*)\s+(搜索|查询|查找)\s*(.+)$/iu);
  if (chineseToolRequest) {
    const [, tool, verb, target] = chineseToolRequest;
    if (normalizedToolName(tool) === 'web_search') {
      return `${verb === '搜索' ? 'Search' : 'Look up'}: ${target.trim()}`;
    }
    return `${prettifyMachineTokens(tool)}: ${target.trim()}`;
  }

  const englishToolRequest = value.match(/^use\s+([A-Za-z][A-Za-z0-9_]*)\s+(?:to\s+)?(.+)$/iu);
  if (englishToolRequest) {
    const [, tool, target] = englishToolRequest;
    if (normalizedToolName(tool) === 'web_search') return `Search: ${target.trim()}`;
    return `${prettifyMachineTokens(tool)}: ${target.trim()}`;
  }

  return null;
}

function normalizedToolName(value: string): string {
  return value.trim().toLowerCase();
}

const MACHINE_TOKEN_LABELS: Record<string, string> = {
  api: 'API',
  html: 'HTML',
  hitl: 'Owner',
  kb: 'Knowledge',
  llm: 'AI',
  mfa: 'Extra confirmation',
  oauth: 'Google sign-in',
  pdf: 'PDF',
  sms: 'SMS',
  url: 'URL',
};

function prettifyMachineTokens(value: string): string {
  return value.replace(/\b[A-Za-z][A-Za-z0-9]*(?:_[A-Za-z0-9]+)+\b/g, (token) => (
    token.split('_')
      .map((part, index) => MACHINE_TOKEN_LABELS[part.toLowerCase()] ?? (index === 0 ? titleCaseMachinePart(part) : part.toLowerCase()))
      .join(' ')
  ))
    .replace(/\bCalendar reader\b/g, 'Calendar')
    .replace(/^Monitor:\s+/i, 'Monitor ');
}

function titleCaseMachinePart(value: string): string {
  if (!value) return value;
  return `${value.slice(0, 1).toUpperCase()}${value.slice(1).toLowerCase()}`;
}

export function homeBriefForItem(item: VNextWorkItem, locale: Locale = 'en'): string {
  const title = ownerVisibleWorkTitle(item, t('work.title.thisItem', locale), 96, locale);
  if (itemIsBlockedByGate(item)) return tf('work.homeBrief.needsAttention', locale, { title });
  switch (laneForItem(item)) {
    case 'decide':
      return tf('work.homeBrief.needsDecision', locale, { title });
    case 'now':
      if (normalized(item.status) === 'scheduled') return tf('work.homeBrief.comingUp', locale, { title });
      return tf('work.homeBrief.readyToReview', locale, { title });
    case 'follow_up':
      return tf('work.homeBrief.followingUp', locale, { title });
    case 'backlog':
      return tf('work.homeBrief.savedForLater', locale, { title });
    default:
      return tf('work.homeBrief.startWith', locale, { title });
  }
}

export function sourceNeedsOwnerAttention(source: { status: string }): boolean {
  const status = normalized(source.status);
  return status === 'unavailable'
    || status === 'error'
    || status === 'failed'
    || status === 'auth_required'
    || status === 'needs_auth'
    || status === 'permission_required';
}

export function buildVNextHomeSignals(home: Pick<VNextHome, 'now' | 'decisions' | 'followUps' | 'sourceIssueCount' | 'sourceIssueSummary'>, userTimezone?: string | null, locale: Locale = 'en'): VNextHomeSignal[] {
  const signals: VNextHomeSignal[] = [];
  const firstDecision = home.decisions[0];
  const firstNow = home.now[0];
  const firstFollowUp = home.followUps[0];

  if (firstDecision) {
    const title = signalSummaryForItem(firstDecision, t('work.signalFallback.thisDecision', locale), userTimezone, locale);
    signals.push({
      id: 'decisions',
      label: t('work.signalSummary.needsDecision', locale),
      summary: homeSignalSummary(title),
      tone: 'warn',
      targetItemId: firstDecision.id,
    });
  }
  if (firstNow) {
    const title = signalSummaryForItem(firstNow, t('work.signalFallback.thisWork', locale), userTimezone, locale);
    const needsAccess = itemNeedsPrivateTaskAccess(firstNow);
    signals.push({
      id: 'ready',
      label: needsAccess ? t('work.signalSummary.needsDetail', locale) : t('work.signalSummary.readyNow', locale),
      summary: needsAccess
        ? privateAccessSignalSummary(title, home.now.length, locale)
        : homeSignalSummary(title),
      tone: needsAccess ? 'warn' : 'neutral',
      targetItemId: firstNow.id,
    });
  }
  if (firstFollowUp) {
    const title = signalSummaryForItem(firstFollowUp, t('work.signalFallback.thisFollowUp', locale), userTimezone, locale);
    signals.push({
      id: 'follow_up',
      label: t('work.signalSummary.followingUp', locale),
      summary: homeSignalSummary(title),
      tone: 'neutral',
      targetItemId: firstFollowUp.id,
    });
  }
  if (home.sourceIssueCount > 0) {
    signals.push({
      id: 'sources',
      label: t('work.signalSummary.connectionIssue', locale),
      summary: home.sourceIssueSummary ?? (home.sourceIssueCount === 1
        ? t('work.signalSummary.oneConnection', locale)
        : tf('work.signalSummary.connectionsLimited', locale, { count: String(home.sourceIssueCount) })),
      tone: 'warn',
    });
  }
  if (signals.length === 0) {
    signals.push({
      id: 'quiet',
      label: t('work.signalSummary.quiet', locale),
      summary: t(QUIET_HOME_BRIEF_KEY, locale),
      tone: 'ok',
    });
  }

  return signals;
}

function homeSignalSummary(title: string): string {
  return title;
}

function privateAccessSignalSummary(title: string, count: number, locale: Locale = 'en'): string {
  if (count <= 1) return title;
  return t('work.signalSummary.openWorkHidden', locale);
}

function signalSummaryForItem(item: VNextWorkItem, fallback: string, userTimezone?: string | null, locale: Locale = 'en'): string {
  // Same coded-title rule as ownerVisibleWorkTitle: a static server title
  // (approvals) localizes through work.decide; anything else keeps the prose.
  const title = ownerVisibleItemTitle(ownerVisibleCodedTitle(item as OwnerCueWorkItem, locale), fallback, 56);
  if (!isGenericSignalTitle(title)) return title;
  if (itemIsBlockedByGate(item)) return t('work.signalSummary.pausedForReview', locale);
  if (itemNeedsDecision(item)) return t('work.signalSummary.waitingForChoice', locale);
  if (laneForItem(item) === 'follow_up') {
    return item.due_at
      ? tf('work.signalSummary.trackingUntil', locale, { time: workTimeLabel(item.due_at, userTimezone) })
      : t('work.signalSummary.trackingQuietly', locale);
  }
  if (normalized(item.status) === 'scheduled') {
    return item.due_at
      ? tf('work.signalSummary.scheduledAt', locale, { time: workTimeLabel(item.due_at, userTimezone) })
      : t('work.signalSummary.scheduled', locale);
  }
  if (itemNeedsPrivateTaskAccess(item)) {
    return t(PRIVATE_TASK_REVIEW_TITLE_KEY, locale);
  }
  return ownerVisibleItemTitle(ownerVisibleNextSafeActionText(item as OwnerCueWorkItem, locale) || ownerVisiblePrimaryActionLabel(item, fallback, 56, locale), fallback, 56);
}

export function mandateMatchWorkItemId(item: {
  source?: string | null;
  source_event_id?: string | null;
}): string | null {
  if ((item.source || '').trim().toLowerCase() !== 'mandates') return null;
  const id = (item.source_event_id || '').trim();
  if (!id.startsWith('calendar:')) return null;
  return id;
}

export function itemNeedsPrivateTaskAccess(item: VNextWorkItem): boolean {
  if (normalized(item.kind) !== 'task') return false;
  const code = (item as OwnerCueWorkItem).next_safe_action_code;
  if (typeof code === 'string' && code.trim() === 'task_private_review') return true;
  // Older projections had no code. Those English sentences are the only fallback.
  return /private high-priority task|details need permission|private task title\/body|task title and body stay private/i.test(`${item.title || ''} ${item.summary} ${(item as OwnerCueWorkItem).next_safe_action || ''}`);
}

function isGenericSignalTitle(title: string): boolean {
  const normalizedTitle = title.toLowerCase();
  return [
    'private high-priority task',
    'private task',
    'private reminder',
    'task to review',
    'review task',
    'this work',
    'this decision',
    'this follow-up',
  ].some((phrase) => normalizedTitle.includes(phrase));
}

export function sourceStatusView(source: { source: string; status: string; reason?: string | null }, locale: Locale = 'en'): VNextSourceStatusView {
  const normalizedSource = normalized(source.source);
  // `label` stays the English identity (SOURCE_LABELS) so equality checks keep
  // working; display sites localize it via localizedSourceLabel(). statusLabel/
  // reason/nextStep render directly, so they ARE localized here.
  const label = SOURCE_LABELS[normalizedSource]
    ?? source.source.split('_').filter(Boolean).map((part) => part[0]?.toUpperCase() + part.slice(1)).join(' ')
    ?? source.source;
  const status = normalized(source.status);
  const healthy = status === 'available' || status === 'ok' || status === 'ready' || status === 'deferred' || status === 'delegated';
  const statusLabel = status === 'deferred'
    ? t('work.source.status.quiet', locale)
    : status === 'auth_required' || status === 'needs_auth'
      ? t('work.source.status.authRefresh', locale)
      : status === 'permission_required'
        ? t('work.source.status.permissionNeeded', locale)
        : status === 'unavailable'
          ? t('work.source.status.unavailable', locale)
        : status === 'delegated'
          ? t('work.source.status.delegated', locale)
    : healthy
      ? t('work.source.status.ready', locale)
      : source.status.replace(/_/g, ' ');
  return {
    label,
    statusLabel,
    healthy,
    reason: sourceReasonLabel(source.reason, locale),
    nextStep: sourceNextStep(status, locale),
  };
}

function sourceReasonLabel(reason: string | null | undefined, locale: Locale = 'en'): string | undefined {
  const value = reason?.trim();
  if (!value) return undefined;
  const lower = value.toLowerCase();
  if (['ready', 'ok', 'available'].includes(lower)) {
    return undefined;
  }
  if (lower.includes('mfa_required') || lower.includes('mfa required')) {
    return t('work.projection.mfa', locale);
  }
  if (lower.includes('consent_required') || lower.includes('consent required')) {
    return t('work.source.reason.consent', locale);
  }
  if (lower.includes('calendar_reader') || lower.includes('calendar reader')) {
    return t('work.projection.calendar.message', locale);
  }
  if (lower.includes('rate limit') || lower.includes('rate_limit')) {
    return t('work.source.reason.rateLimit', locale);
  }
  if (lower.includes('circuit')) {
    return t('work.source.reason.circuit', locale);
  }
  if (lower.includes('projector not connected')) {
    return t('work.projection.notConnected', locale);
  }
  if (lower.includes('metadata only')) {
    return t('work.projection.metadataOnly', locale);
  }
  if (lower.includes('not connected yet')) {
    return t('work.source.reason.notConnectedYet', locale);
  }
  // A bare machine token (codex_connector, mcp_bridge) names the plumbing,
  // not a reason the owner can act on; the status column already says it.
  if (/^[a-z0-9]+(?:_[a-z0-9]+)+$/.test(lower) && !lower.includes('required')) {
    return undefined;
  }
  return value;
}

function sourceNextStep(status: string, locale: Locale = 'en'): string | undefined {
  if (status === 'auth_required' || status === 'needs_auth') {
    return t('work.source.nextStep.authRefresh', locale);
  }
  if (status === 'permission_required') {
    return t('work.source.nextStep.permission', locale);
  }
  if (status === 'unavailable' || status === 'error' || status === 'failed') {
    return t('work.source.nextStep.unavailable', locale);
  }
  return undefined;
}

export function authorityBasisLabel(value: string | null | undefined, locale: Locale = 'en'): string {
  const basis = normalized(value);
  if (!basis) return t('work.basis.waitingReview', locale);
  return t(AUTHORITY_BASIS_LABEL_KEYS[basis] ?? 'work.basis.ownerReviewFallback', locale);
}

export function isTaskLikeItem(item: Pick<VNextWorkItem, 'kind'> & { source?: string; source_kind?: string }): boolean {
  const kind = normalized(item.kind);
  const source = normalized(item.source);
  const sourceKind = normalized(item.source_kind);
  return kind === 'task' || kind === 'reminder' || sourceKind === 'task' || source === 'tasks' || source === 'task';
}

export function isApprovalItem(item: Pick<VNextWorkItem, 'kind'> & { source?: string; source_kind?: string }): boolean {
  const kind = normalized(item.kind);
  const source = normalized(item.source);
  const sourceKind = normalized(item.source_kind);
  return kind === 'approval' || sourceKind === 'decision' || source === 'hitl';
}

export function buildSupportedItemActions(item: VNextWorkItem & { source?: string }, locale: Locale = 'en'): VNextItemAction[] {
  if (isClosedItem(item)) return [];
  const status = normalized(item.status);
  const itemLane = normalized((item as { lane?: string }).lane);
  const followUpItem = itemLane === 'follow_up'
    || ['waiting', 'tracking', 'scheduled', 'deferred', 'snoozed'].includes(status);
  const serverActionIds = Array.isArray(item.secondary_actions)
    ? item.secondary_actions
      .map((action) => normalized(action.action_kind))
      .filter(Boolean)
    : [];
  if (serverActionIds.length === 0) return [];
  const actions: VNextItemAction[] = [];

  const allowed = new Set(serverActionIds);
  if (isApprovalItem(item) || allowed.has('approval.approve') || allowed.has('approval.reject')) {
    const blockedTaskApproval = itemIsBlockedByGate(item) && isTaskLikeItem(item);
    const approvalActions: VNextItemAction[] = [{
      id: 'approval.approve',
      label: blockedTaskApproval ? t('work.itemAction.approveRun', locale) : t('work.itemAction.approve', locale),
      confirmLabel: blockedTaskApproval ? t('work.itemAction.confirmApproveRun', locale) : t('work.itemAction.confirmApprove', locale),
      receiptHint: t('work.itemAction.hintApprove', locale),
      tone: 'normal',
    }, {
      id: 'approval.reject',
      label: blockedTaskApproval ? t('work.itemAction.skipRun', locale) : t('work.itemAction.reject', locale),
      confirmLabel: blockedTaskApproval ? t('work.itemAction.confirmSkipRun', locale) : t('work.itemAction.confirmReject', locale),
      receiptHint: t('work.itemAction.hintReject', locale),
      tone: 'destructive',
    }];
    return approvalActions.filter((action) => allowed.has(action.id));
  }

  if (item.kind === 'memory_review' || normalized(item.source_kind) === 'memory' || normalized(item.source) === 'memory') {
    actions.push({
      id: 'memory.confirm',
      label: t('work.itemAction.keepUsing', locale),
      confirmLabel: t('work.itemAction.confirmKeepUsing', locale),
      receiptHint: t('work.itemAction.hintKeepUsing', locale),
      tone: 'normal',
    });
    actions.push({
      id: allowed.has('memory.unpin') ? 'memory.unpin' : 'memory.pin',
      label: allowed.has('memory.unpin') ? t('work.itemAction.unpin', locale) : t('work.itemAction.pin', locale),
      confirmLabel: allowed.has('memory.unpin') ? t('work.itemAction.confirmUnpin', locale) : t('work.itemAction.confirmPin', locale),
      receiptHint: t('work.itemAction.hintPin', locale),
      tone: 'normal',
    });
    actions.push({
      id: 'memory.mark_wrong',
      label: t('work.itemAction.markWrong', locale),
      confirmLabel: t('work.itemAction.confirmMarkWrong', locale),
      receiptHint: t('work.itemAction.hintMarkWrong', locale),
      tone: 'destructive',
    });
    actions.push({
      id: 'memory.delete',
      label: t('work.itemAction.forget', locale),
      confirmLabel: t('work.itemAction.confirmForget', locale),
      receiptHint: t('work.itemAction.hintForget', locale),
      tone: 'destructive',
    });
    return actions.filter((action) => allowed.has(action.id));
  }

  if (allowed.has('task.skip_occurrence')) {
    actions.push({
      id: 'task.skip_occurrence',
      label: t('work.itemAction.endThisOnce', locale),
      confirmLabel: t('work.itemAction.confirmEndThisOnce', locale),
      receiptHint: t('work.itemAction.hintSkipOccurrence', locale),
      tone: 'normal',
    });
  }
  if (allowed.has('tracking.cancel')) {
    actions.push({
      id: 'tracking.cancel',
      label: t('work.itemAction.closeTracking', locale),
      confirmLabel: t('work.itemAction.confirmCloseTracking', locale),
      receiptHint: t('work.itemAction.hintCloseTracking', locale),
      tone: 'destructive',
    });
  }

  if (normalized(item.kind) === 'goal' && allowed.has('goal.abandon')) {
    actions.push({
      id: 'goal.abandon',
      label: t('work.itemAction.endThisCard', locale),
      confirmLabel: t('work.itemAction.confirmEndThisCard', locale),
      receiptHint: t('work.itemAction.hintCloseGoal', locale),
      tone: 'normal',
    });
  }

  if (normalized(item.kind) === 'waiting_for' || normalized(item.source) === 'knowledge' || normalized(item.source_kind) === 'tracking') {
    actions.push({
      id: 'knowledge.done',
      label: followUpItem ? t('work.itemAction.endThisCard', locale) : t('work.itemAction.done', locale),
      confirmLabel: followUpItem ? t('work.itemAction.confirmEndThisCard', locale) : t('work.itemAction.confirmCloseFollowUp', locale),
      receiptHint: t('work.itemAction.hintCloseWaitingLoop', locale),
      tone: 'normal',
    });
    actions.push({
      id: 'knowledge.snooze_tomorrow',
      label: t('work.itemAction.reviewTomorrow', locale),
      confirmLabel: t('work.itemAction.confirmTomorrow', locale),
      receiptHint: t('work.itemAction.hintKeepVisibleTomorrow', locale),
      tone: 'normal',
    });
  }

  if (isTaskLikeItem(item)) {
    if (itemIsBlockedByGate(item)) {
      actions.push({
        id: 'task.complete',
        label: t('work.itemAction.closeWithoutRunning', locale),
        confirmLabel: t('work.itemAction.confirmCloseWithoutRunning', locale),
        receiptHint: t('work.itemAction.hintCloseWithoutRunning', locale),
        tone: 'normal',
      });
      actions.push({
        id: 'task.snooze_tomorrow',
        label: t('work.itemAction.reviewTomorrow', locale),
        confirmLabel: t('work.itemAction.confirmTomorrow', locale),
        receiptHint: t('work.itemAction.hintKeepPausedTomorrow', locale),
        tone: 'normal',
      });
    } else {
      actions.push({
        id: 'task.complete',
        label: followUpItem
          ? t('work.itemAction.endThisCard', locale)
          : t('work.itemAction.complete', locale),
        confirmLabel: followUpItem ? t('work.itemAction.confirmEndThisCard', locale) : t('work.itemAction.confirmComplete', locale),
        receiptHint: t('work.itemAction.hintTaskCompletion', locale),
        tone: 'normal',
      });
      actions.push({
        id: 'task.snooze_tomorrow',
        label: followUpItem ? t('work.itemAction.reviewTomorrow', locale) : t('work.itemAction.snooze', locale),
        confirmLabel: t('work.itemAction.confirmTomorrow', locale),
        receiptHint: t('work.itemAction.hintSnoozeTomorrow', locale),
        tone: 'normal',
      });
    }
  }

  return actions.filter((action) => allowed.has(action.id));
}

/**
 * Owner-facing Work surface control presentation.
 *
 * Goal: never show a flat peer wall of equal-looking actions. Exactly one
 * primary control sits in the primary slot; everything else is secondary
 * progressive disclosure (overflow / "More").
 *
 * Ambient AI entry ("Ask Aegis") is intentionally outside this model — it is
 * context, not a receipt-backed work action.
 */
export type OwnerWorkSurfaceControl =
  | { kind: 'open_detail'; id: 'open_detail'; label: string }
  | { kind: 'item_action'; id: VNextItemActionId; action: VNextItemAction }
  | { kind: 'assistant'; id: 'brief' | 'draft'; label: string }
  /** Open status/profile in the native X app or browser (complement, not a second Timeline). */
  | { kind: 'open_x'; id: 'open_x'; label: string; href: string }
  /** Open X compose/reply intent with optional draft text — owner sends in X. */
  | { kind: 'draft_in_x'; id: 'draft_in_x'; label: string; href: string };

export type OwnerWorkSurfacePresentation = {
  primary: OwnerWorkSurfaceControl | null;
  secondary: OwnerWorkSurfaceControl[];
};

export function presentOwnerWorkSurfaceActions(input: {
  item: VNextWorkItem & { source?: string };
  locale?: Locale;
  /** True when private/source detail is already open for this item. */
  sourceDetailOpen: boolean;
  /** Show the navigate/open primary when detail is not yet open. */
  includeOpenDetail: boolean;
  openDetailLabel: string;
  /** When set, surface Open in X (and optional draft-in-X) as peer open paths. */
  openX?: { href: string; label: string; draftHref?: string; draftLabel?: string } | null;
  /** Brief/Draft assistant chips (non-receipt). */
  includeAssistantChips: boolean;
  briefLabel: string;
  draftLabel: string;
  /** Receipt-backed item actions already filtered for visibility. */
  itemActions: VNextItemAction[];
}): OwnerWorkSurfacePresentation {
  const locale = input.locale ?? 'en';
  void locale;
  const secondary: OwnerWorkSurfaceControl[] = [];
  let primary: OwnerWorkSurfaceControl | null = null;

  const takePrimary = (control: OwnerWorkSurfaceControl) => {
    if (!primary) primary = control;
    else secondary.push(control);
  };

  // 1) Open Work detail, or X as a peer open path when the item is about a post/profile.
  if (!input.sourceDetailOpen) {
    if (input.openX?.href) {
      takePrimary({ kind: 'open_x', id: 'open_x', label: input.openX.label, href: input.openX.href });
      if (input.includeOpenDetail) {
        secondary.push({ kind: 'open_detail', id: 'open_detail', label: input.openDetailLabel });
      }
    } else if (input.includeOpenDetail) {
      takePrimary({ kind: 'open_detail', id: 'open_detail', label: input.openDetailLabel });
    }
  } else if (input.openX?.href) {
    // Detail already open: still offer jump to X as a secondary peer.
    secondary.push({ kind: 'open_x', id: 'open_x', label: input.openX.label, href: input.openX.href });
  }

  if (input.openX?.draftHref && input.openX.draftLabel) {
    secondary.push({
      kind: 'draft_in_x',
      id: 'draft_in_x',
      label: input.openX.draftLabel,
      href: input.openX.draftHref,
    });
  }

  // 2) Receipt-backed item actions: first remaining becomes primary only when
  //    open path did not claim it (detail already open / not offered).
  for (const action of input.itemActions) {
    takePrimary({ kind: 'item_action', id: action.id, action });
  }

  // 3) Assistant chips always demote to secondary when present.
  if (input.includeAssistantChips) {
    secondary.push({ kind: 'assistant', id: 'brief', label: input.briefLabel });
    if (supportsDraftCommandForPresentation(input.item)) {
      secondary.push({ kind: 'assistant', id: 'draft', label: input.draftLabel });
    }
  }

  return { primary, secondary };
}

function supportsDraftCommandForPresentation(item: Pick<VNextWorkItem, 'kind'>): boolean {
  const kind = normalized(item.kind);
  return kind !== 'approval' && kind !== 'receipt';
}


/**
 * Work list FIND/scan chrome: when the owner has not searched or changed
 * filters, scope+sort segmented rows stay progressive-disclosed rather than
 * a permanent dual wall under the search field.
 */
export function workScanFiltersDefaultExpanded(options: {
  query?: string;
  scope?: string;
  sort?: string;
}): boolean {
  const query = (options.query ?? '').trim();
  const scope = (options.scope ?? 'all').trim().toLowerCase() || 'all';
  const sort = (options.sort ?? 'lane').trim().toLowerCase() || 'lane';
  return Boolean(query) || scope !== 'all' || sort !== 'lane';
}

/**
 * Entire FIND chrome (search + filters): collapsed on first paint unless the
 * owner already has an active search/filter (same signal as filters expanded).
 */
export function workScanChromeDefaultExpanded(options: {
  query?: string;
  scope?: string;
  sort?: string;
}): boolean {
  return workScanFiltersDefaultExpanded(options);
}

/**
 * Work item detail: Ask Aegis placement. Closed → compact secondary control
 * (never a full-width peer primary wall). Open → inline assistant panel.
 */
export type WorkDetailAskPlacement = 'compact_secondary' | 'inline_panel';

export function workDetailAskPlacement(assistantOpen: boolean): WorkDetailAskPlacement {
  return assistantOpen ? 'inline_panel' : 'compact_secondary';
}

export function optimisticStatusForItemAction(
  actionId: VNextItemActionId,
  item?: { kind?: string | null; source?: string | null } | null,
): string | null {
  if (actionId === 'goal.abandon') return 'abandoned';
  // Closing the generator of a waiting card does not finish that card.
  // The client remaps waiting rows to kind "task" and keeps source "knowledge".
  if (actionId === 'tracking.cancel') {
    const kind = item?.kind?.trim().toLowerCase() ?? '';
    const source = item?.source?.trim().toLowerCase() ?? '';
    if (kind === 'waiting_for' || source === 'knowledge') return null;
    return 'cancelled';
  }
  if (actionId === 'task.complete') return 'done';
  if (actionId === 'task.skip_occurrence') return null;
  if (actionId === 'task.snooze_tomorrow') return 'snoozed';
  if (actionId === 'knowledge.done') return 'done';
  if (actionId === 'knowledge.snooze_tomorrow') return 'snoozed';
  if (actionId === 'approval.approve' || actionId === 'approval.reject') return 'done';
  if (actionId === 'memory.confirm' || actionId === 'memory.mark_wrong' || actionId === 'memory.delete') return 'done';
  if (actionId === 'memory.pin') return 'pinned_review';
  if (actionId === 'memory.unpin') return 'stale_review';
  return null;
}

export function resolveCommandFocusId(scope: VNextCommandScope, selectedId: string | null | undefined): string | null {
  if (scope === 'general') return GENERAL_ASSISTANT_FOCUS_ID;
  return selectedId || null;
}

export function buildCommandSuggestions(item: (VNextWorkItem & { source?: string }) | null | undefined, scope: VNextCommandScope, locale: Locale = 'en'): VNextCommandSuggestion[] {
  if (!item || scope === 'general') {
    return [
      { id: 'general.missing', label: t('work.suggest.general.missing.label', locale), prompt: t('work.suggest.general.missing.prompt', locale) },
      { id: 'general.today', label: t('work.suggest.general.today.label', locale), prompt: t('work.suggest.general.today.prompt', locale) },
      { id: 'general.followups', label: t('work.suggest.general.followups.label', locale), prompt: t('work.suggest.general.followups.prompt', locale) },
    ];
  }

  const suggestions: VNextCommandSuggestion[] = [
    { id: 'item.next', label: t('work.suggest.item.next.label', locale), prompt: t('work.suggest.item.next.prompt', locale) },
  ];

  if (isTaskLikeItem(item)) {
    suggestions.push(
      { id: 'item.task.tomorrow', label: t('work.suggest.item.tomorrow.label', locale), prompt: t('work.suggest.item.tomorrow.prompt', locale) },
      { id: 'item.task.breakdown', label: t('work.suggest.item.breakdown.label', locale), prompt: t('work.suggest.item.breakdown.prompt', locale) },
    );
  } else {
    suggestions.push(
      { id: 'item.track', label: t('work.suggest.item.trackThis.label', locale), prompt: t('work.suggest.item.trackThis.prompt', locale) },
      { id: 'item.backlog', label: t('work.suggest.item.moveLater.label', locale), prompt: t('work.suggest.item.moveLater.prompt', locale) },
    );
  }

  return suggestions;
}
