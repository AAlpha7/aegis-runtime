import { t, type Locale } from '../i18n/catalog';
import { resolveWorkCodeText } from './model';
import { formatInUserTz, parseWireMoment, timezoneOffsetLabel } from './time';

/**
 * Resolves an optional structural code carried in a source-detail payload
 * (e.g. `owner_visible_reason_code`) to localized text, falling back to the
 * prose already resolved by the caller. Unknown/absent codes keep the prose.
 */
function codedField(
  prefix: 'work.reasonCode' | 'work.nextCode' | 'work.actionLabel' | 'work.decide',
  code: unknown,
  prose: string,
  locale: Locale,
): string {
  return resolveWorkCodeText(prefix, typeof code === 'string' ? code : null, locale) ?? prose;
}

/**
 * Format a source-detail timestamp value for owner display in the owner's
 * timezone. Accepts RFC3339 strings or epoch (seconds/millis) numeric strings.
 * Non-timestamp / sentinel values (e.g. 'Unknown', 'Not executed', '') pass
 * through unchanged. The underlying wire payload is never mutated — only the
 * rendered label string is localized. Falls back to the browser timezone (via
 * the shared offset helper) when `userTimezone` is null/invalid; never throws.
 */
export function formatDetailMoment(value: string, userTimezone?: string | null): string {
  const raw = value?.trim();
  if (!raw) return value;
  // Numeric epoch (seconds or millis)?
  // Date-only values (no time-of-day) must not be shifted into a timezone.
  if (/^\d{4}-\d{2}-\d{2}$/.test(raw)) return value;
  let date: Date | null = null;
  if (/^\d{9,}$/.test(raw)) {
    const n = Number(raw);
    date = new Date(n < 10_000_000_000 ? n * 1000 : n);
  } else if (/^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}/.test(raw)) {
    // Shared parser treats no-offset SQLite datetimes as UTC, not browser-local.
    date = parseWireMoment(raw);
  }
  if (!date || !Number.isFinite(date.getTime())) return value;
  const wall = formatInUserTz(
    date,
    userTimezone,
    {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    },
    'en-CA',
  ).replace(',', '');
  if (!wall) return value;
  return `${wall} ${timezoneOffsetLabel(date, userTimezone)}`;
}

export interface DetailAttachment {
  filename: string;
  /** Localized meta line: "mime · size". */
  meta: string;
  /** Server-signed same-origin download url, or null (metadata-only row). */
  downloadUrl: string | null;
  /** Server says this can render inline (raster image / pdf). */
  previewable: boolean;
  /** Raw mime (drives <img> vs <iframe> preview element). */
  mimeType: string;
}

export interface DetailField {
  label: string;
  value: string;
  /**
   * Stable machine id (e.g. 'state', 'due') for consumers that group/filter
   * fields. Behavior must key on this — never on the localized `label` text.
   */
  id?: string;
  /**
   * True when `value` is a localized placeholder (e.g. "Not set"), not real
   * data. Consumers that suppress placeholder values must check this flag —
   * never compare the localized value text.
   */
  placeholder?: boolean;
}

export interface DetailTimelineItem {
  label: string;
  status: string;
  at?: string;
}

export interface SourceDetailView {
  kind: 'task' | 'approval' | 'action_draft' | 'receipt' | 'memory' | 'generic' | 'locked';
  title: string;
  subtitle: string;
  fields: DetailField[];
  body?: string;
  bodyLabel?: string;
  attachments?: DetailAttachment[];
  timeline: DetailTimelineItem[];
  externalUrl?: string;
  warning?: string;
}

const MAX_BODY_CHARS = 200_000;
const CLOSED_TASK_STATES = new Set(['archived', 'cancelled', 'canceled', 'done', 'completed']);

export function taskDetailClosedStatus(payload: unknown): string | null {
  const root = asRecord(payload);
  if (root?.resource_type !== 'task_timeline') return null;
  const detail = asRecord(root.detail);
  const task = asRecord(detail?.task);
  const state = stringField(task?.state, '').trim().toLowerCase();
  return CLOSED_TASK_STATES.has(state) ? state : null;
}

// Module-level pure builder (no React context): the owner's display_language
// locale is threaded in as an explicit required param, mirroring
// sourceDetailPayloadFromOpenError. Wire-enum identity values (status,
// resource_type, action kinds rendered through ownerLabel) stay English.
export function buildSourceDetailView(payload: unknown, locale: Locale, userTimezone?: string | null): SourceDetailView {
  const root = asRecord(payload);
  const status = stringField(root?.status, '');
  if (status === 'mfa_required' || status === 'consent_required') {
    return {
      kind: 'locked',
      title: t('detail.locked.title', locale),
      subtitle: stringField(asRecord(root?.detail)?.message, t('detail.locked.subtitleFallback', locale)),
      fields: [
        { label: t('detail.field.reason', locale), value: ownerFacingLockedReason(status, stringField(root?.reason, ''), locale) },
      ],
      timeline: [],
      warning: t('detail.locked.warning', locale),
    };
  }

  if (root?.resource_type === 'task_timeline') {
    return buildTaskDetail(root, locale, userTimezone);
  }
  if (root?.resource_type === 'approval_review') {
    return buildApprovalDetail(root, locale);
  }
  if (root?.resource_type === 'action_ledger_entry') {
    return buildActionLedgerDetail(root, locale, userTimezone);
  }
  if (root?.resource_type === 'memory_item') {
    return buildMemoryDetail(root, locale, userTimezone);
  }
  if (root?.resource_type === 'owner_work_item') {
    return buildOwnerWorkItemDetail(root, locale);
  }
  if (root?.resource_type === 'source_unavailable') {
    return buildUnavailableDetail(root, locale);
  }

  return {
    kind: 'generic',
    title: t('detail.generic.title', locale),
    subtitle: t('detail.generic.subtitle', locale),
    fields: safeTopLevelFields(root, userTimezone),
    timeline: [],
  };
}

function buildOwnerWorkItemDetail(root: Record<string, unknown>, locale: Locale): SourceDetailView {
  const detail = asRecord(root.detail);
  const title = codedField('work.decide', detail?.title_code, stringField(detail?.title, t('detail.workItem.titleFallback', locale)), locale);
  const summary = stringField(detail?.summary, t('detail.workItem.summaryFallback', locale));
  const reason = codedField('work.reasonCode', detail?.owner_visible_reason_code, stringField(detail?.owner_visible_reason, ''), locale);
  const blockedBy = codedField('work.reasonCode', detail?.blocked_by_code, stringField(detail?.blocked_by, ''), locale);
  const next = codedField('work.nextCode', detail?.next_safe_action_code, stringField(detail?.next_safe_action, t('detail.workItem.nextFallback', locale)), locale);
  const receipt = stringField(detail?.receipt_expectation, '');
  const availableActionCodes = Array.isArray(detail?.available_action_codes) ? detail.available_action_codes : [];
  const availableActions = Array.isArray(detail?.available_actions)
    ? detail.available_actions
      .map((action, index) => codedField('work.actionLabel', availableActionCodes[index], stringField(action, ''), locale))
      .filter(Boolean)
    : [];
  const primaryAction = codedField('work.actionLabel', detail?.primary_action_code, stringField(detail?.primary_action, ''), locale);

  return {
    kind: 'generic',
    title,
    subtitle: summary,
    fields: [
      { label: t('detail.field.next', locale), value: next },
      { label: t('detail.field.state', locale), value: ownerLabel(stringField(detail?.status, 'available')) },
      { label: t('detail.field.kind', locale), value: ownerLabel(stringField(detail?.kind, 'work')) },
      { label: t('detail.field.urgency', locale), value: ownerLabel(stringField(detail?.urgency, 'normal')) },
      ...(primaryAction ? [{ label: t('detail.field.primaryAction', locale), value: primaryAction }] : []),
      ...(availableActions.length > 0 ? [{ label: t('detail.field.availableActions', locale), value: availableActions.join(' · ') }] : []),
      ...(receipt ? [{ label: t('detail.field.receipt', locale), value: receipt }] : []),
    ],
    body: [reason, blockedBy].filter(Boolean).join('\n\n') || undefined,
    bodyLabel: reason || blockedBy ? t('detail.workItem.bodyLabel', locale) : undefined,
    timeline: [],
    warning: t('detail.workItem.warning', locale),
  };
}

function buildUnavailableDetail(root: Record<string, unknown>, locale: Locale): SourceDetailView {
  const detail = asRecord(root.detail);
  return {
    kind: 'generic',
    // Fallbacks reuse the app.sourceDetail.* keys (en values byte-identical)
    // because sourceDetailPayloadFromOpenError feeds this same render vector.
    title: stringField(detail?.title, t('app.sourceDetail.changedTitle', locale)),
    subtitle: stringField(detail?.message, t('detail.unavailable.messageFallback', locale)),
    fields: [
      { label: t('detail.field.state', locale), value: ownerLabel(stringField(detail?.status, 'changed')) },
      { label: t('detail.field.next', locale), value: stringField(detail?.next_step, t('app.sourceDetail.changedNextStep', locale)) },
    ],
    timeline: [],
    warning: t('detail.unavailable.warning', locale),
  };
}

function buildMemoryDetail(root: Record<string, unknown>, locale: Locale, userTimezone?: string | null): SourceDetailView {
  const detail = asRecord(root.detail);
  const factSummary = cleanBody(stringField(detail?.fact_summary, ''));
  const pinned = stringField(detail?.is_pinned, '') === 'true';
  const confidence = stringField(detail?.confidence, 'unknown');

  return {
    kind: 'memory',
    title: stringField(detail?.topic, t('detail.memory.titleFallback', locale)),
    subtitle: t('detail.memory.subtitle', locale),
    fields: [
      { label: t('detail.field.confidence', locale), value: confidence },
      { label: t('detail.field.pinned', locale), value: pinned ? t('detail.value.yes', locale) : t('detail.value.no', locale) },
      { label: t('detail.field.created', locale), value: formatDetailMoment(stringField(detail?.created_at, t('detail.value.unknown', locale)), userTimezone) },
      { label: t('detail.field.state', locale), value: ownerLabel(stringField(detail?.status, 'available')) },
    ],
    body: factSummary,
    bodyLabel: factSummary ? t('detail.memory.bodyLabel', locale) : undefined,
    timeline: [],
    warning: t('detail.memory.warning', locale),
  };
}

function buildApprovalDetail(root: Record<string, unknown>, locale: Locale): SourceDetailView {
  const detail = asRecord(root.detail);
  const title = codedField('work.decide', detail?.title_code, stringField(detail?.title, t('detail.approval.titleFallback', locale)), locale);
  const summary = stringField(detail?.summary, t('detail.approval.summaryFallback', locale));
  const requiresMfa = stringField(detail?.requires_mfa, '') === 'true';
  const nextStep = codedField('work.nextCode', detail?.next_safe_action_code, stringField(detail?.next_safe_action, t('detail.approval.nextFallback', locale)), locale);
  const blockedBy = codedField('work.reasonCode', detail?.blocked_by_code, stringField(detail?.blocked_by, ''), locale);
  const reason = codedField('work.reasonCode', detail?.owner_visible_reason_code, stringField(detail?.owner_visible_reason, ''), locale);
  const receipt = stringField(detail?.receipt_expectation, t('detail.approval.receiptFallback', locale));

  return {
    kind: 'approval',
    title,
    subtitle: summary,
    fields: [
      { label: t('detail.field.decision', locale), value: nextStep },
      { label: t('detail.field.confirmation', locale), value: requiresMfa ? t('detail.value.mfaMayBeRequired', locale) : t('detail.value.sessionEnough', locale) },
      { label: t('detail.field.receipt', locale), value: receipt },
      { label: t('detail.field.state', locale), value: ownerLabel(stringField(detail?.status, 'waiting')) },
    ],
    body: [blockedBy, reason].filter(Boolean).join('\n\n') || undefined,
    bodyLabel: blockedBy || reason ? t('detail.approval.bodyLabel', locale) : undefined,
    timeline: [],
    warning: t('detail.approval.warning', locale),
  };
}

function buildTaskDetail(root: Record<string, unknown>, locale: Locale, userTimezone?: string | null): SourceDetailView {
  const detail = asRecord(root.detail);
  const task = asRecord(detail?.task);
  const timeline = Array.isArray(detail?.timeline)
    ? detail.timeline.map((entry) => asTimelineItem(entry, locale, userTimezone)).filter((item): item is DetailTimelineItem => Boolean(item)).slice(0, 12)
    : [];
  // due_date may be a date-only string OR an RFC3339 instant (task snooze
  // writes due.to_rfc3339()). formatDetailMoment localizes instants and passes
  // date-only values through unchanged. A missing due renders the localized
  // "Not set" placeholder — flagged via `placeholder`, which is how consumers
  // (WorkView taskSummarySubtitle) suppress it; never by matching the text.
  const dueRaw = stringField(task?.due_date, '');

  return {
    kind: 'task',
    title: stringField(task?.title, t('detail.task.titleFallback', locale)),
    subtitle: stringField(task?.body, stringField(task?.context_text, t('detail.task.subtitleFallback', locale))),
    // `id` is the stable machine key consumers group/filter on (WorkView
    // TASK_PRIMARY_FIELD_ORDER) — the localized label is display-only.
    fields: [
      { id: 'state', label: t('detail.field.state', locale), value: ownerLabel(stringField(task?.state, 'unknown')) },
      { id: 'priority', label: t('detail.field.priority', locale), value: ownerLabel(stringField(task?.priority, 'normal')) },
      {
        id: 'due',
        label: t('detail.field.due', locale),
        value: dueRaw ? formatDetailMoment(dueRaw, userTimezone) : t('detail.value.notSet', locale),
        ...(dueRaw ? {} : { placeholder: true }),
      },
      { id: 'mode', label: t('detail.field.mode', locale), value: ownerLabel(stringField(task?.execution_mode, 'manual')) },
    ],
    timeline,
  };
}

function buildActionLedgerDetail(root: Record<string, unknown>, locale: Locale, userTimezone?: string | null): SourceDetailView {
  const detail = asRecord(root.detail);
  const entry = asRecord(detail?.entry);
  const metadata = parseMetadataJson(entry?.metadata_json);
  if (stringField(metadata?.resource_type, '') === 'contextual_action_draft') {
    return buildActionDraftDetail(detail, entry, metadata, locale, userTimezone);
  }

  return buildReceiptDetail(detail, entry, locale, userTimezone);
}

function buildActionDraftDetail(
  detail: Record<string, unknown> | null,
  entry: Record<string, unknown> | null,
  metadata: Record<string, unknown>,
  locale: Locale,
  userTimezone?: string | null,
): SourceDetailView {
  const actionKind = stringField(entry?.action_kind, 'Action draft');
  const draftText = cleanBody(stringField(metadata.draft_text, ''));

  return {
    kind: 'action_draft',
    // `Review ${kind}` stays English for now: ownerLabel(action_kind) renders
    // a wire enum, and a localized template around an English arg would emit
    // mixed-language output. Deferred to the wire-enum→zh map ticket.
    title: `Review ${ownerLabel(actionKind)}`,
    subtitle: t('detail.actionDraft.subtitle', locale),
    fields: [
      { label: t('detail.field.state', locale), value: ownerLabel(stringField(entry?.action_status, stringField(entry?.raw_status, 'proposed'))) },
      { label: t('detail.field.focus', locale), value: ownerLabel(stringField(metadata.focus_kind, 'selected item')) },
      { label: t('detail.field.receipt', locale), value: stringField(detail?.receipt_status, t('detail.value.receiptNotApplicable', locale)) },
      { label: t('detail.field.execution', locale), value: stringField(metadata.execution_enabled, '') === 'true' ? t('detail.value.executionEnabled', locale) : t('detail.value.executionBlocked', locale) },
    ],
    body: draftText,
    bodyLabel: draftText ? t('detail.actionDraft.bodyLabel', locale) : undefined,
    timeline: [
      {
        label: t('detail.actionDraft.timelineSaved', locale),
        status: t('detail.actionDraft.timelineStatus', locale),
        at: formatDetailMoment(stringField(entry?.proposed_at, stringField(entry?.created_at, '')), userTimezone),
      },
    ],
    warning: t('detail.actionDraft.warning', locale),
  };
}

function buildReceiptDetail(
  detail: Record<string, unknown> | null,
  entry: Record<string, unknown> | null,
  locale: Locale,
  userTimezone?: string | null,
): SourceDetailView {
  const receipt = stringField(entry?.external_receipt, '');
  const summary = stringField(entry?.user_visible_summary, stringField(entry?.input_summary, t('detail.receipt.summaryFallback', locale)));

  return {
    kind: 'receipt',
    title: summary,
    subtitle: ownerLabel(stringField(entry?.action_kind, 'Action')),
    fields: [
      { label: t('detail.field.status', locale), value: ownerLabel(stringField(entry?.action_status, 'unknown')) },
      { label: t('detail.field.receipt', locale), value: stringField(detail?.receipt_status, receipt ? t('detail.value.receiptPresent', locale) : t('detail.value.receiptMissing', locale)) },
      { label: t('detail.field.created', locale), value: formatDetailMoment(stringField(entry?.created_at, t('detail.value.unknownLower', locale)), userTimezone) },
      { label: t('detail.field.executed', locale), value: formatDetailMoment(stringField(entry?.executed_at, t('detail.value.notExecuted', locale)), userTimezone) },
    ],
    body: receipt || undefined,
    bodyLabel: receipt ? t('detail.receipt.bodyLabel', locale) : undefined,
    timeline: [],
    warning: receipt ? undefined : t('detail.receipt.warning', locale),
  };
}

function parseMetadataJson(value: unknown): Record<string, unknown> {
  if (typeof value !== 'string' || !value.trim()) return {};
  try {
    const parsed = JSON.parse(value);
    return asRecord(parsed) ?? {};
  } catch {
    return {};
  }
}

const TIMELINE_LABEL_KEYS: Record<string, string> = {
  receipt: 'detail.timeline.label.receipt',
  action: 'detail.timeline.label.action',
  update: 'detail.timeline.label.update',
};

/** Server timeline labels are English tokens (Receipt/Action/Update); show the owner's language. */
function timelineLabel(raw: string, locale: Locale): string {
  const key = TIMELINE_LABEL_KEYS[raw.trim().toLowerCase()];
  return key ? t(key, locale) : raw;
}

function asTimelineItem(value: unknown, locale: Locale, userTimezone?: string | null): DetailTimelineItem | null {
  const record = asRecord(value);
  if (!record) return null;
  return {
    label: timelineLabel(stringField(record.label, stringField(record.kind, t('detail.timeline.eventFallback', locale))), locale),
    status: ownerLabel(stringField(record.status, 'unknown')),
    at: formatDetailMoment(stringField(record.at, stringField(record.at_unix, '')), userTimezone),
  };
}

function safeTopLevelFields(root: Record<string, unknown> | null, userTimezone?: string | null): DetailField[] {
  if (!root) return [];
  return Object.entries(root)
    .filter(([key]) => sourceDetailFieldIsOwnerFacing(key))
    .filter(([, value]) => ['string', 'number', 'boolean'].includes(typeof value))
    .slice(0, 8)
    .map(([label, value]) => {
      const raw = String(value);
      // Localize timestamp-like keys (created_at, updated_at, *_at, *_time …)
      // so generic-fallback details never leak raw RFC3339/epoch wire values.
      const display = isTimestampLikeKey(label) ? formatDetailMoment(raw, userTimezone) : raw;
      return { label: ownerLabel(label), value: display };
    });
}

/** True when the field key denotes a moment (instant) value to localize. */
function isTimestampLikeKey(key: string): boolean {
  const k = key.trim().toLowerCase();
  return /(^|_)(at|time|timestamp|ts)$/.test(k) || k.endsWith('_at_unix') || k.endsWith('_time_unix');
}

function sourceDetailFieldIsOwnerFacing(key: string): boolean {
  const normalized = key.trim().toLowerCase();
  if (!normalized) return false;
  if (normalized === 'resource_type' || normalized === 'schema_version') return false;
  if (normalized === 'id' || normalized.endsWith('_id')) return false;
  if (normalized.endsWith('_ids')) return false;
  return ![
    'trace_id',
    'run_id',
    'turn_id',
    'action_id',
    'mandate_id',
    'tool_call_id',
    'provider_request_id',
    'idempotency_key',
  ].includes(normalized);
}

function ownerFacingLockedReason(status: string, reason: string, locale: Locale): string {
  if (status === 'mfa_required') {
    return t('detail.locked.reasonMfa', locale);
  }
  if (status === 'consent_required') {
    return t('detail.locked.reasonConsent', locale);
  }
  const trimmed = reason.trim();
  if (!trimmed) return t('detail.locked.reasonSensitive', locale);
  const lower = trimmed.toLowerCase();
  if (
    lower.includes('mfa_required')
    || lower.includes('consent_required')
    || lower.includes('data_access_gate')
    || lower.includes('provider_request')
    || lower.includes('trace_id')
    || lower.includes('run_id')
    || /\b[a-z]+_[a-z0-9_]+\b/.test(lower)
  ) {
    return t('detail.locked.reasonSensitive', locale);
  }
  return trimmed;
}

function cleanBody(value: string | undefined): string | undefined {
  if (!value) return undefined;
  const normalized = value
    .replace(/\n?\[HIDDEN CONTENT STRIPPED[^\]]*\]\s*$/i, '')
    .replace(/\s+\n/g, '\n')
    .trim();
  if (!normalized || normalized === '(no text body)') return undefined;
  return normalized.length <= MAX_BODY_CHARS ? normalized : `${normalized.slice(0, MAX_BODY_CHARS).trim()}...`;
}

function ownerLabel(value: string): string {
  const label = value
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/\b([A-Za-z])([A-Za-z]*)\b/g, (word: string, first: string, rest: string) => (
      OWNER_DETAIL_WORD_LABELS[word.toLowerCase()] ?? `${first.toUpperCase()}${rest.toLowerCase()}`
    ));
  return label.replace(/\bExtra confirmation Required\b/g, 'Extra confirmation required');
}

const OWNER_DETAIL_WORD_LABELS: Record<string, string> = {
  api: 'API',
  hitl: 'Owner',
  html: 'HTML',
  kb: 'Knowledge',
  llm: 'AI',
  mfa: 'Extra confirmation',
  oauth: 'Google sign-in',
  pdf: 'PDF',
  sms: 'SMS',
  url: 'URL',
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringField(value: unknown, fallback: string): string {
  if (typeof value === 'string' && value.trim()) return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  return fallback;
}
