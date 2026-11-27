import { Fragment, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { useLocale, useT } from '../i18n/LocaleContext';
import type { ContextualAssistantResponse, NoticeSeverity, OwnerWorkItem } from '../api';
import { AssistantResponseView, SourceEvidenceContent, sourceEvidenceItems } from './AssistantResponseView';
import { AegisAvatar, OwnerAvatar } from './Avatar';
import type { AvatarProfile } from './avatarPrefs';
import type { SourceDetailView } from './detail';
import { itemActionIconName, VNextIcon } from './icons';
import { VNextButton } from './VNextButton';
import type { VNextHeaderMenuItem } from './VNextHeaderMenu';
import { OwnerShellChrome, type OwnerShellSummon } from './OwnerShellChrome';
import type { VNextHomeSignal } from './model';
import { buildSupportedItemActions, isClosedItem, itemNeedsDecision, ownerVisibleItemTitle, ownerVisibleNextSafeActionText, ownerVisiblePrimaryActionLabel, ownerVisibleReasonCue, ownerVisibleWorkCue, ownerVisibleWorkTitle } from './model';
// isClosedItem used to drop desk cards after a successful lifecycle action.
import type { VNextItemAction, VNextItemActionId } from './model';
import { itemActionButtonLabel, itemActionConfirmationHint } from './interactionState';
import { assistantResultTitle, classNames, itemMeta, ownerUtilityMenuItemIds } from './presentation';
import {
  ownerFacingApprovalAction,
  ownerFacingApprovalWhy,
  resolveCeoShorthand,
  voicePendingDecision,
} from './ceoIntent';
import { parseEgressHoldCard, type EgressHoldCard } from './egressHoldCard';
import {
  isSearchHold,
  omitGatePendingOwnerAcks,
  readSearchOutcome,
  searchHoldIdentity,
  searchOwnerQuestion,
} from './searchApproval';
export {
  isGatePendingOwnerAck,
  isSearchHold,
  omitGatePendingOwnerAcks,
  ownerVisibleSearchOutcome,
  readSearchOutcome,
  searchHoldIdentity,
} from './searchApproval';
import { splitSearchReceipt } from './searchReceipt';
import { ResearchStopControl, SearchReceiptCard } from './SearchReceiptCard';
import { formatVoiceDuration } from './voiceNote';
import { copyMessageRef, messageCiteToken, messagePublicRef } from './messageRef';

export { messageCiteToken, messagePublicRef };
import { chatDayKey, chatFullTimestampLabel, chatTimestampLabel, formatInUserTz, normalizeMessageTimestamp } from './time';
import { useUserTimezone } from './UserTimezoneContext';
import { RichText } from './RichText';
import type { DeliveryNoticeAction } from './notificationActions';
import { parseDeliveryNotice } from './notificationActions';
import { SourceDetailPanel } from './WorkView';

export type ChatHomeApprovalState = 'pending' | 'approved' | 'rejected';

/** After 批准, the same search card moves through these before it settles. */
export type ChatHomeApprovalPhase = 'running' | 'done' | 'failed';

/**
 * What a SENT owner turn carried alongside (or instead of) its text.
 *
 * A voice note renders as mic + transcript + duration — the transcript is the
 * server's speech-to-text result, so the owner can see exactly what Aegis
 * heard. Assistant messages never carry this.
 */
export type ChatHomeMessageAttachment =
  | { kind: 'voice'; transcript: string; durationSecs: number | null }
  | { kind: 'image'; dataUrl: string }
  | { kind: 'document'; filename: string };

export type ChatHomeMessage = {
  id: string;
  turnId?: string;
  replyOrigin?: 'command' | 'history' | 'transport-error';
  role: 'owner' | 'aegis' | 'working' | 'queued' | 'approval';
  text: string;
  /** Provisional assistant text while tokens are still arriving. Cleared on
   *  draft reset and dropped with the working bubble when the turn settles. */
  streamText?: string;
  /** Present on an owner turn that carried a voice note / image / document. */
  attachment?: ChatHomeMessageAttachment;
  timestamp?: number;
  /** Aegis replies: ms from the owner's message reaching the server to the reply
      being stored (server-measured; the live turn falls back to the client clock). */
  latencyMs?: number;
  /** Collapsed heartbeat alerts: how many consecutive copies this bubble stands for. */
  repeatCount?: number;
  /** When the collapsed run started (ms epoch). */
  firstTimestamp?: number;
  severity?: NoticeSeverity | null;
  quote?: ChatHomeQuote;
  contextLabel?: string;
  result?: ContextualAssistantResponse;
  workItem?: OwnerWorkItem | null;
  workItemAnchorId?: string | null;
  workItemThread?: ChatHomeWorkThreadEntry[];
  workItemUnavailable?: boolean;
  retryText?: string;
  retryIdempotencyKey?: string;
  retryLabel?: string;
  retryContext?: ChatHomeAttachedContext;
  retryQuote?: ChatHomeQuote;
  // In-chat HITL approval card (Slice 4). Present only for `role === 'approval'`.
  // `summary` is the excerpt the backend streamed (≤500 chars, never raw
  // html/shell). `approvalState` drives the card: pending shows both buttons,
  // approved/rejected shows a disabled state chip.
  approvalId?: string;
  toolName?: string;
  summary?: string;
  approvalState?: ChatHomeApprovalState;
  /** Search holds only. `running` is the in-progress line under 已批准. */
  approvalPhase?: ChatHomeApprovalPhase;
  /** Owner-readable search result or failure, without tool JSON. */
  approvalOutcome?: string;
  /** Quiet footer when the turn used an executor (`run_external` / `delegate_task`). */
  executorFooter?: string | null;
};

/**
 * Pure reducer (Slice 4): append an in-chat approval card for `approvalId` to
 * `messages`. Deduplicates by `approvalId` so a reconnect/replay of the same
 * `need_approval` SSE frame never stacks two cards. The summary is the streamed
 * excerpt (≤500 chars, never raw html/shell). Extracted as a standalone helper
 * so the App-level streaming logic stays thin and this transition is unit-tested.
 */
/**
 * Date dividers for the chat stream (IMG_3939: cross-day messages ran
 * together — 23:05 followed by 01:00 read like time going backwards).
 * Returns messageId → divider label for every message that STARTS a new
 * calendar day (in the owner's timezone) relative to the previous DATED
 * message; the first dated message also gets one. Messages without a usable
 * timestamp never break a day group. Pure — unit-tested directly; `nowMs`
 * is injectable so 今天/昨天 labels are deterministic in tests.
 */
/**
 * The message's effective DATED timestamp: its own when present, else the
 * newest dated entry hidden inside its collapsed-notice range. Shared by the
 * collapse day-guard and the divider labeling so a collapsed group whose
 * visible card is undated still anchors (and labels) its day correctly.
 */
export function effectiveDatedTimestamp(
  message: { timestamp?: number; collapsedNotice?: { timestamps?: number[] } },
): number | undefined {
  return (
    normalizeMessageTimestamp(message.timestamp)
    ?? [...(message.collapsedNotice?.timestamps ?? [])]
      .map((value) => normalizeMessageTimestamp(value))
      .filter((value): value is number => Boolean(value))
      .pop()
  );
}

export function chatDateDividerLabels(
  messages: { id: string; timestamp?: number; collapsedNotice?: { timestamps?: number[] } }[],
  userTimezone: string | null | undefined,
  locale: Locale,
  nowMs: number = Date.now(),
): Record<string, string> {
  const labels: Record<string, string> = {};
  const todayKey = chatDayKey(nowMs, userTimezone);
  // Yesterday = today's LOCAL calendar date minus one day, via UTC date
  // arithmetic on the formatted parts — NOT nowMs - 24h, which lands on the
  // wrong local date when the previous day was 23h long (DST spring-forward;
  // Codex finding).
  const yesterdayKey = (() => {
    const [y, m, d] = todayKey.split('-').map(Number);
    if (!y || !m || !d) return '';
    const prev = new Date(Date.UTC(y, m - 1, d));
    prev.setUTCDate(prev.getUTCDate() - 1);
    const mm = String(prev.getUTCMonth() + 1).padStart(2, '0');
    const dd = String(prev.getUTCDate()).padStart(2, '0');
    return `${prev.getUTCFullYear()}-${mm}-${dd}`;
  })();
  let lastDayKey = '';
  for (const message of messages) {
    const normalized = effectiveDatedTimestamp(message);
    if (!normalized) continue;
    const dayKey = chatDayKey(normalized, userTimezone);
    if (!dayKey || dayKey === lastDayKey) continue;
    lastDayKey = dayKey;
    if (dayKey === todayKey) {
      labels[message.id] = t('chatHome.date.today', locale);
    } else if (dayKey === yesterdayKey) {
      labels[message.id] = t('chatHome.date.yesterday', locale);
    } else {
      const sameYear = dayKey.slice(0, 4) === todayKey.slice(0, 4);
      labels[message.id] = formatInUserTz(
        new Date(normalized),
        userTimezone,
        sameYear
          ? { month: 'long', day: 'numeric', weekday: 'short' }
          : { year: 'numeric', month: 'long', day: 'numeric', weekday: 'short' },
        locale === 'zh' ? 'zh-CN' : 'en-US',
      );
    }
  }
  return labels;
}

function egressStoppedText(card: EgressHoldCard, locale: Locale): string {
  const why = t(`chatHome.approval.stopped.${card.stopped}`, locale);
  if (card.stopped === 'untrusted_source' && card.untrustedSource) {
    const kind = t(`chatHome.approval.untrusted.${card.untrustedSource}`, locale);
    return `${why} (${kind})`;
  }
  return why;
}

function ApprovalFacts({
  message,
  locale,
  ownerQuestion,
}: {
  message: ChatHomeMessage;
  locale: Locale;
  ownerQuestion: string;
}) {
  const egress = parseEgressHoldCard(message.summary);
  if (egress && isSearchHold(message.summary, message.toolName)) {
    return (
      <>
        {ownerQuestion ? (
          <div>
            <dt>{t('chatHome.approval.searchQuestion', locale)}</dt>
            <dd data-testid="search-owner-question">{ownerQuestion}</dd>
          </div>
        ) : null}
        <div>
          <dt>{t('chatHome.approval.whyLabel', locale)}</dt>
          <dd data-testid="egress-stopped">{egressStoppedText(egress, locale)}</dd>
        </div>
      </>
    );
  }
  if (egress) {
    return (
      <>
        <div>
          <dt>{t('chatHome.approval.destinationLabel', locale)}</dt>
          <dd data-testid="egress-destination">{egress.destination}</dd>
        </div>
        <div>
          <dt>{t('chatHome.approval.wouldSendLabel', locale)}</dt>
          <dd data-testid="egress-would-send">{egress.wouldSend}</dd>
        </div>
        <div>
          <dt>{t('chatHome.approval.whyLabel', locale)}</dt>
          <dd data-testid="egress-stopped">{egressStoppedText(egress, locale)}</dd>
        </div>
      </>
    );
  }
  return (
    <>
      <div>
        <dt>{t('chatHome.approval.whatLabel', locale)}</dt>
        <dd>
          {message.summary?.trim()
            || ownerFacingApprovalAction(message.toolName, locale)}
        </dd>
      </div>
      <div>
        <dt>{t('chatHome.approval.whyLabel', locale)}</dt>
        <dd>{ownerFacingApprovalWhy(message.toolName, locale)}</dd>
      </div>
    </>
  );
}

export function replaceApprovalMessagesFromServer(
  messages: ChatHomeMessage[],
  cards: ReadonlyArray<{
    approval_id: string;
    tool_name?: string;
    summary?: string;
    state: ChatHomeApprovalState;
    timestamp: number;
    outcome?: string | null;
  }> | undefined,
): ChatHomeMessage[] {
  if (!cards) return messages;
  const previous = new Map(
    messages
      .filter((message) => message.role === 'approval' && message.approvalId)
      .map((message) => [message.approvalId as string, message]),
  );
  const serverIds = new Set(cards.map((card) => card.approval_id));
  const liveOnly = messages.filter((message) => (
    message.role === 'approval'
    && Boolean(message.approvalId)
    && !serverIds.has(message.approvalId ?? '')
    && (
      (message.approvalState !== 'approved' && message.approvalState !== 'rejected')
      || message.approvalPhase === 'running'
      || message.approvalPhase === 'done'
      || message.approvalPhase === 'failed'
    )
  ));
  const fromServer = cards.map((card) => {
    const prior = previous.get(card.approval_id);
    const stored = readSearchOutcome(card.outcome);
    const localAhead = Boolean(
      prior
      && prior.approvalState === 'approved'
      && card.state === 'pending'
      && (prior.approvalPhase === 'running' || prior.approvalPhase === 'done' || prior.approvalPhase === 'failed'),
    );
    const phase = localAhead ? prior?.approvalPhase : (stored ? stored.phase : prior?.approvalPhase);
    const outcome = localAhead
      ? prior?.approvalOutcome
      : (stored ? (stored.text || undefined) : prior?.approvalOutcome);
    return {
      id: `approval:${card.approval_id}`,
      role: 'approval' as const,
      text: '',
      timestamp: normalizeMessageTimestamp(card.timestamp),
      approvalId: card.approval_id,
      toolName: card.tool_name || undefined,
      summary: card.summary || prior?.summary || undefined,
      approvalState: localAhead ? 'approved' as const : card.state,
      ...(phase ? { approvalPhase: phase } : {}),
      ...(outcome ? { approvalOutcome: outcome } : {}),
    };
  });
  return messages.filter((message) => message.role !== 'approval').concat(fromServer, liveOnly);
}

export function appendApprovalMessage(
  messages: ChatHomeMessage[],
  params: { approvalId: string; toolName?: string | null; summary?: string | null; timestamp?: number },
): ChatHomeMessage[] {
  if (messages.some((message) => message.role === 'approval' && message.approvalId === params.approvalId)) {
    return messages;
  }
  const identity = searchHoldIdentity(params.summary, params.toolName);
  if (identity && messages.some((message) => (
    message.role === 'approval'
    && message.approvalState !== 'rejected'
    && searchHoldIdentity(message.summary, message.toolName) === identity
  ))) {
    return messages;
  }
  return messages.concat({
    id: `approval:${params.approvalId}`,
    role: 'approval',
    text: '',
    timestamp: params.timestamp ?? Date.now(),
    approvalId: params.approvalId,
    toolName: params.toolName ?? undefined,
    summary: params.summary ?? undefined,
    approvalState: 'pending',
  });
}

/**
 * Pure reducer (Slice 4): set the `approvalState` of the approval card matching
 * `approvalId`. Used for the optimistic flip on action and the revert on
 * failure. Non-approval messages and other approvals are untouched.
 */
export function setApprovalState(
  messages: ChatHomeMessage[],
  approvalId: string,
  state: ChatHomeApprovalState,
): ChatHomeMessage[] {
  return messages.map((message) => (
    message.role === 'approval' && message.approvalId === approvalId
      ? { ...message, approvalState: state }
      : message
  ));
}

export function setApprovalProgress(
  messages: ChatHomeMessage[],
  approvalId: string,
  patch: {
    approvalState?: ChatHomeApprovalState;
    approvalPhase?: ChatHomeApprovalPhase | null;
    approvalOutcome?: string | null;
  },
): ChatHomeMessage[] {
  return messages.map((message) => {
    if (message.role !== 'approval' || message.approvalId !== approvalId) return message;
    const next: ChatHomeMessage = { ...message };
    if (patch.approvalState) next.approvalState = patch.approvalState;
    if (patch.approvalPhase !== undefined) {
      next.approvalPhase = patch.approvalPhase ?? undefined;
    }
    if (patch.approvalOutcome !== undefined) {
      next.approvalOutcome = patch.approvalOutcome ?? undefined;
    }
    return next;
  });
}

type ChatHomeDisplayMessage = ChatHomeMessage & {
  collapsedNotice?: {
    labelKey: string;
    detailKey: string;
    count: number;
    firstTimestamp?: number;
    timestamps: number[];
    hiddenNotices: CollapsedNoticeItem[];
  };
};

type CollapsedNoticeItem = {
  id: string;
  text: string;
  timestamp?: number;
  severity?: NoticeSeverity | null;
};

export type ChatHomeQuote = {
  sourceId?: string;
  role: 'owner' | 'aegis';
  text: string;
  timestamp?: number;
};

export type ChatHomeWorkThreadEntry = {
  id: string;
  question: string;
  timestamp?: number;
  responseTimestamp?: number;
  itemStatus?: string | null;
  state?: 'queued' | 'working';
  workingText?: string;
  result?: ContextualAssistantResponse;
};

export type ChatHomeAttachedContext = {
  itemId: string;
  label: string;
};

/**
 * Composer voice-note state, projected from `voiceNote.ts` for rendering.
 * Deliberately display-shaped (a formatted elapsed label, a resolved error
 * message) so the view holds no timing or i18n logic of its own.
 */
export type ChatHomeVoiceNote =
  | { status: 'idle' }
  | { status: 'recording'; elapsedLabel: string }
  | { status: 'uploading' }
  | { status: 'attached'; transcript: string; durationLabel: string }
  | { status: 'error'; message: string };

/**
 * Slice 5: a multimodal attachment the owner has uploaded and is holding in the
 * composer, pending the next command submit. Mirrors the backend
 * `VNextCommandAttachment` (image/document) plus the display filename. Voice
 * uses the existing voice-input path and is not held as a pending attachment.
 */
export type ChatHomePendingAttachment = {
  kind: 'image' | 'document';
  dataUrl: string;
  filename: string;
  mimeType: string;
};

type HomeBriefCard = {
  label: string;
  text: string;
};

type PlainNoticePresentation = {
  title: string;
  previewLines: string[];
  meta: string[];
  detailLabel: string;
  detailHint: string;
};

export type ChatHomeWorkActionResult = {
  ok: boolean;
  message: string;
  nextStatus: string | null;
  itemUnavailable?: boolean;
};

export type ChatHomeWorkDetailResult =
  | { ok: true; detail: SourceDetailView; message?: string }
  | { ok: false; message: string };

export type ChatHomeNoticeActionResult = {
  ok: boolean;
  message: string;
};

/**
 * A locale-bound translator threaded into the top-level helper functions that
 * produce owner-facing text (they cannot call the `useT()` hook themselves).
 * `t(key)` resolves a plain catalog entry; `f(key, params)` interpolates.
 */
type Translator = {
  locale: Locale;
  t: (key: string) => string;
  f: (key: string, params: Record<string, string>) => string;
};

function makeTranslator(tr: (key: string) => string, locale: Locale): Translator {
  return {
    locale,
    t: tr,
    f: (key, params) => tf(key, locale, params),
  };
}

// The `label` keys map starter prompts to localized catalog entries; the `prompt`
// text is what is actually sent to Aegis and stays raw (it is the request body,
// not chrome).
const STARTER_PROMPTS = [
  { id: 'commute', labelKey: 'chatHome.starter.commute', prompt: '通勤', icon: 'status' },
  { id: 'tasks', labelKey: 'chatHome.starter.tasks', prompt: '任务', icon: 'work' },
  { id: 'missing', labelKey: 'chatHome.starter.focus', prompt: '我现在有什么需要注意的吗？', icon: 'spark' },
] as const;

type ComposerMode = 'line' | 'half';

type NoticeCollapseRule = {
  id: string;
  labelKey: string;
  detailKey: string;
  windowMs: number;
  groupKey: (message: ChatHomeMessage) => string | null;
};

const SIX_HOURS_MS = 6 * 60 * 60 * 1000;
const LONG_PLAIN_NOTICE_CHAR_LIMIT = 900;
const LONG_PLAIN_NOTICE_LINE_LIMIT = 12;
const BRIEF_SECTION_HEADINGS = [
  'Calendar',
  '日历',
  'Active Tasks',
  '进行中的待办',
  'Waiting For',
  '在等',
  'Pending Approvals',
  '待你确认',
  'Alerts',
  '提醒',
  'Recent Actions',
  '最近动作',
  'Suggested Actions',
  '建议动作',
  'Warnings',
  'Actions',
  '动作',
  'Still Pending',
  '仍未完成',
  'Durable Facts',
  '记住的事实',
];

const BRIEF_HEADING_ZH: Record<string, string> = {
  calendar: '日历',
  'active tasks': '进行中的待办',
  'waiting for': '在等',
  'pending approvals': '待你确认',
  alerts: '提醒',
  'recent actions': '最近动作',
  'suggested actions': '建议动作',
  warnings: '提醒',
  actions: '动作',
  'still pending': '仍未完成',
  'durable facts': '记住的事实',
};

const BRIEF_LINE_ZH: Record<string, string> = {
  'Calendar not queried; do not assume there are no events.': '没去问日历；不要当成没有事件。',
  'Calendar queried; no events were provided.': '已问过日历；这次没有事件。',
  'Calendar unavailable; do not assume there are no events.': '日历不可用；不要当成没有事件。',
};

const NOTICE_COLLAPSE_RULES: NoticeCollapseRule[] = [
  {
    id: 'system_restart',
    labelKey: 'chatHome.collapse.restart.label',
    detailKey: 'chatHome.collapse.restart.detail',
    windowMs: SIX_HOURS_MS,
    groupKey: (message) => {
      if (message.role !== 'aegis' || message.result || message.workItem) return null;
      return /系统已重启|system restarted/i.test(message.text) ? 'system_restart' : null;
    },
  },
  {
    id: 'system_integrity_report',
    labelKey: 'chatHome.collapse.integrity.label',
    detailKey: 'chatHome.collapse.integrity.detail',
    windowMs: SIX_HOURS_MS,
    groupKey: (message) => {
      if (message.role !== 'aegis' || message.result || message.workItem) return null;
      return /^System Integrity Report\b/i.test(message.text.trim()) ? 'system_integrity_report' : null;
    },
  },
];

function nextComposerMode(mode: ComposerMode): ComposerMode {
  if (mode === 'line') return 'half';
  return 'line';
}

/** Action-oriented size menu label: what the next tap will do (not cryptic 1L). */
export function composerSizeActionLabel(mode: ComposerMode, locale: Locale): string {
  return mode === 'line'
    ? t('chatHome.composer.expand', locale)
    : t('chatHome.composer.collapse', locale);
}

/** Enter during IME candidate confirmation must not send. keyCode 229 is the
 *  extra signal some browsers emit when isComposing is still false. */
function isImeEnter(event: KeyboardEvent): boolean {
  return event.isComposing || event.keyCode === 229;
}

function resizeComposerInput(input: HTMLTextAreaElement | null, mode: ComposerMode = 'line') {
  if (!input) return;
  if (mode !== 'line') {
    input.style.height = '';
    return;
  }
  const computed = window.getComputedStyle(input);
  const maxHeight = Number.parseFloat(computed.maxHeight || '') || 128;
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, maxHeight)}px`;
}

function isNearStreamBottom(stream: HTMLElement): boolean {
  return stream.scrollHeight - stream.scrollTop - stream.clientHeight < 96;
}

function chatHomeResultTitle(result: ContextualAssistantResponse, tr: Translator): string {
  const kind = result.response_kind.trim().toLowerCase();
  if (kind === 'orchestrated_response' || kind === 'explanation' || kind === 'answer') {
    return tr.t('chatHome.role.aegis');
  }
  return assistantResultTitle(result, tr.locale);
}

// Maps the DUAL-USE English retry logic key (stored in App.tsx for `=== 'Check
// result'` comparisons) to its localized display label. The stored value is
// kept English on purpose; only the rendered text is localized.
function retryDisplayLabel(retryLabel: string | undefined, tr: Translator): string {
  if (retryLabel === 'Check result') return tr.t('chatHome.retry.checkResult');
  return tr.t('chatHome.retry.tryAgain');
}

function ChatRetryAction({
  message,
  busy,
  tr,
  onRetry,
}: {
  message: ChatHomeMessage;
  busy: boolean;
  tr: Translator;
  onRetry: (text: string, idempotencyKey?: string, context?: ChatHomeAttachedContext, quote?: ChatHomeQuote) => void;
}) {
  if (!message.retryText) return null;
  return (
    <div className="vnext-action-row">
      <VNextButton
        variant="secondary"
        size="sm"
        data-testid="aegis-chat-retry"
        onClick={() => onRetry(message.retryText || '', message.retryIdempotencyKey, message.retryContext, message.retryQuote)}
      >
        {busy ? (message.retryLabel === 'Check result' ? tr.t('chatHome.retry.queueCheck') : tr.t('chatHome.retry.queueRetry')) : retryDisplayLabel(message.retryLabel, tr)}
      </VNextButton>
    </div>
  );
}

function workCardNextStep(item: OwnerWorkItem, tr: Translator): string {
  if (isClosedItem(item)) {
    return tr.t('chatHome.work.handledNote');
  }
  const openWhenUseful = tr.t('chatHome.work.openWhenUseful');
  const rawNext = ownerVisibleNextSafeActionText(item, tr.locale) || ownerVisiblePrimaryActionLabel(item, openWhenUseful, 116, tr.locale);
  return ownerVisibleItemTitle(
    rawNext,
    openWhenUseful,
    116,
  );
}

function workCardProofText(item: OwnerWorkItem, tr: Translator): string {
  const raw = (item.authority?.expected_receipt || item.receipt_expectation || '').trim();
  if (!raw) return '';
  if (/provider receipt/i.test(raw)) {
    return tr.t('chatHome.work.serviceReceiptKept');
  }
  if (/no external action was executed|reading or drafting alone does not execute/i.test(raw)) {
    return tr.t('chatHome.work.noOutsideAction');
  }
  return ownerVisibleItemTitle(raw, raw, 116);
}

function workCardPrimaryActionLabel(item: OwnerWorkItem, tr: Translator): string {
  return ownerVisiblePrimaryActionLabel(item, tr.t('chatHome.work.open'), 40, tr.locale);
}

/** "3.2" under ten seconds, whole seconds above; never negative. */
export function formatLatencySeconds(latencyMs: number): string {
  const seconds = Math.max(0, latencyMs) / 1000;
  if (seconds < 10) return (Math.round(seconds * 10) / 10).toFixed(1);
  return String(Math.round(seconds));
}

/**
 * Reply duration for an Aegis message: the server-measured value when the
 * row carries one, otherwise the gap to the owner message right before it
 * (second precision, history stored before 2026-09-12). Gaps over an hour or
 * negative gaps are not a turn and yield nothing.
 */
export function replyLatencyMs(messages: ChatHomeMessage[], index: number): number | undefined {
  const message = messages[index];
  if (!message || message.role !== 'aegis') return undefined;
  if (typeof message.latencyMs === 'number' && Number.isFinite(message.latencyMs) && message.latencyMs >= 0) {
    return message.latencyMs;
  }
  if (message.repeatCount || message.workItem) return undefined;
  const previous = messages[index - 1];
  if (!previous || previous.role !== 'owner') return undefined;
  const reply = normalizeMessageTimestamp(message.timestamp);
  const asked = normalizeMessageTimestamp(previous.timestamp);
  if (!reply || !asked) return undefined;
  const gap = reply - asked;
  if (gap < 0 || gap > 60 * 60 * 1000) return undefined;
  return gap;
}

/** Collapsed heartbeat alerts: "已重复 15 次，首次 昨天 03:39". */
function RepeatedAlertNote({ count, firstTimestamp }: { count: number; firstTimestamp?: number }) {
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const first = normalizeMessageTimestamp(firstTimestamp);
  const firstLabel = first ? chatFullTimestampLabel(first, userTimezone) : '';
  return (
    <p className="vnext-chat-repeat" data-repeat-count={count}>
      {firstLabel
        ? tf('chatHome.alert.repeatedSince', locale, { count: String(count), first: firstLabel })
        : tf('chatHome.alert.repeated', locale, { count: String(count) })}
    </p>
  );
}

function MessageTime({ timestamp, latencyMs }: { timestamp?: number; latencyMs?: number }) {
  const [expanded, setExpanded] = useState(false);
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const normalized = normalizeMessageTimestamp(timestamp);
  const label = expanded
    ? chatFullTimestampLabel(normalized, userTimezone)
    : chatTimestampLabel(normalized, userTimezone);
  const fullLabel = chatFullTimestampLabel(normalized, userTimezone);
  if (!label || !normalized) return null;
  const hasLatency = typeof latencyMs === 'number' && Number.isFinite(latencyMs) && latencyMs >= 0;
  const seconds = hasLatency ? formatLatencySeconds(latencyMs) : '';
  // Owner 2026-09-12: how long the reply took, quietly next to the time; the
  // tap that expands the time also spells it out.
  const latencyLabel = hasLatency
    ? tf(expanded ? 'chatHome.latency.full' : 'chatHome.latency.compact', locale, { seconds })
    : '';
  return (
    <time
      className="vnext-chat-time"
      dateTime={new Date(normalized).toISOString()}
      role="button"
      tabIndex={0}
      title={hasLatency ? `${fullLabel} · ${tf('chatHome.latency.full', locale, { seconds })}` : fullLabel}
      aria-label={tf(expanded ? 'chatHome.time.hideFull' : 'chatHome.time.showFull', locale, { time: fullLabel })}
      onClick={() => setExpanded((current) => !current)}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        setExpanded((current) => !current);
      }}
    >
      {label}
      {hasLatency && (
        <span className="vnext-chat-latency" data-latency-ms={Math.round(latencyMs)}>
          {latencyLabel}
        </span>
      )}
    </time>
  );
}

function MessageCite({
  role,
  turnId,
  timestamp,
  text,
  fallbackId,
}: {
  role: 'owner' | 'aegis';
  turnId?: string | null;
  timestamp?: number | null;
  text?: string | null;
  fallbackId?: string | null;
}) {
  const { locale } = useLocale();
  const ref = messagePublicRef({ role, turnId, timestamp, text, fallbackId });
  const token = messageCiteToken(ref);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = window.setTimeout(() => setCopied(false), 1400);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <button
      type="button"
      className={copied ? 'vnext-chat-ref is-copied' : 'vnext-chat-ref'}
      data-message-ref={ref}
      aria-label={tf(copied ? 'chatHome.message.copiedRef' : 'chatHome.message.copyRef', locale, { ref: token })}
      title={token}
      onClick={() => {
        void copyMessageRef(token).then((ok) => {
          if (ok) setCopied(true);
        });
      }}
    >
      {copied ? t('chatHome.message.refCopied', locale) : token}
    </button>
  );
}

function ChatMessageCite({
  message,
}: {
  message: Pick<ChatHomeMessage, 'role' | 'turnId' | 'timestamp' | 'text' | 'id' | 'result'>;
}) {
  if (message.role !== 'owner' && message.role !== 'aegis') return null;
  const text = message.role === 'aegis'
    ? (message.result?.visible_answer || message.text)
    : message.text;
  return (
    <MessageCite
      role={message.role}
      turnId={message.turnId || message.result?.turn_id}
      timestamp={message.timestamp}
      text={text}
      fallbackId={message.id}
    />
  );
}

function CollapsedNoticeBadge({ notice, tr }: { notice?: ChatHomeDisplayMessage['collapsedNotice']; tr: Translator }) {
  const userTimezone = useUserTimezone();
  if (!notice || notice.count < 2) return null;
  const first = chatTimestampLabel(notice.firstTimestamp, userTimezone);
  const last = chatTimestampLabel(notice.timestamps[notice.timestamps.length - 1], userTimezone);
  const range = first && last && first !== last ? `${first}-${last}` : (last || first || '');
  const label = tr.t(notice.labelKey);
  return (
    <details className="vnext-chat-repeat vnext-disclosure">
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{notice.count} {label}</strong>
          {range && <em className="vnext-disclosure-hint">{range}</em>}
        </span>
      </summary>
      <p>{notice.count - 1} {tr.t(notice.detailKey)}</p>
      <div className="vnext-chat-repeat-list" aria-label={tr.f('chatHome.collapsed.earlierLabel', { label })}>
        {notice.hiddenNotices.map((item) => (
          <article key={item.id} className="vnext-chat-repeat-item">
            <div>
              <strong>{tr.t('chatHome.role.earlier')}</strong>
              <MessageTime timestamp={item.timestamp} />
            </div>
            <RichText content={item.text} importanceSeverity={item.severity} importanceDismissKey={item.id} />
          </article>
        ))}
      </div>
    </details>
  );
}

function quoteRoleLabel(role: ChatHomeQuote['role'], tr: Translator): string {
  return tr.t(role === 'aegis' ? 'chatHome.role.aegis' : 'chatHome.role.you');
}

function quotePreviewText(value: string): string {
  const normalized = value.trim().replace(/\s+/g, ' ');
  if (normalized.length <= 180) return normalized;
  return `${normalized.slice(0, 177).trim()}...`;
}

function ChatQuotePreview({ quote, tr }: { quote: ChatHomeQuote; tr: Translator }) {
  return (
    <blockquote className="vnext-chat-quote-preview">
      <span>{tr.f('chatHome.quote.replyingTo', { role: quoteRoleLabel(quote.role, tr) })}</span>
      <p>{quotePreviewText(quote.text)}</p>
    </blockquote>
  );
}

/**
 * The paragraph body of an owner turn.
 *
 * Empty when the turn was a voice note whose text is already shown inside the
 * voice chip, or an attachment-only send whose caption the backend supplies —
 * printing either again would read as a stutter.
 */
export function messageBodyText(message: {
  text: string;
  attachment?: ChatHomeMessageAttachment;
}): string {
  const text = message.text ?? '';
  const attachment = message.attachment;
  if (attachment?.kind === 'voice' && text.trim() === attachment.transcript.trim()) return '';
  return text;
}

/** Sent-attachment chip inside an owner bubble. */
function ChatMessageAttachment({
  attachment,
  tr,
}: {
  attachment: ChatHomeMessageAttachment;
  tr: Translator;
}) {
  if (attachment.kind === 'voice') {
    const duration = formatVoiceDuration(attachment.durationSecs);
    return (
      <span className="vnext-chat-message-voice" aria-label={tr.t('chatHome.voiceNote.sentAria')}>
        <VNextIcon name="voice" />
        <span className="vnext-chat-message-voice-text">{attachment.transcript}</span>
        {duration && <em className="vnext-chat-message-voice-duration">{duration}</em>}
      </span>
    );
  }
  if (attachment.kind === 'image') {
    return (
      <span className="vnext-chat-message-attachment is-image">
        <img src={attachment.dataUrl} alt={tr.t('chatHome.message.imageAlt')} />
      </span>
    );
  }
  return (
    <span className="vnext-chat-message-attachment">
      <VNextIcon name="paperclip" />
      <span>{attachment.filename}</span>
    </span>
  );
}

function quoteFromChatMessage(message: ChatHomeMessage): ChatHomeQuote | null {
  if (message.role !== 'owner' && message.role !== 'aegis') return null;
  const text = (message.role === 'aegis' ? message.result?.visible_answer || message.text : message.text).trim();
  if (!text) return null;
  return {
    sourceId: message.id,
    role: message.role,
    text,
    ...(message.timestamp ? { timestamp: message.timestamp } : {}),
  };
}

export function collapsedChatHomeMessages(
  messages: ChatHomeMessage[],
  userTimezone?: string | null,
): ChatHomeDisplayMessage[] {
  const collapsed: ChatHomeDisplayMessage[] = [];
  for (const message of messages) {
    const match = collapsibleNoticeMatch(message);
    const previous = collapsed[collapsed.length - 1];
    const previousMatch = previous ? collapsibleNoticeMatch(previous) : null;
    if (
      match
      && previous
      && previousMatch
      && previousMatch.key === match.key
      && previousMatch.rule.id === match.rule.id
      && noticesAreNearby(previous, message, match.rule)
      // Never merge notices across an owner-local day boundary: the collapsed
      // card shows time-only values, so a 23:05→01:00 merge would hide the
      // date change the dividers exist to surface (Codex round-3).
      && noticesShareOwnerDay(previous, message, userTimezone)
    ) {
      const previousNotice = previous.collapsedNotice;
      const hiddenNotices = [
        ...(previousNotice?.hiddenNotices ?? []),
        collapsedNoticeItem(previous),
      ];
      collapsed[collapsed.length - 1] = {
        ...message,
        id: `${previous.id}:collapsed:${message.id}`,
        collapsedNotice: {
          labelKey: match.rule.labelKey,
          detailKey: match.rule.detailKey,
          count: hiddenNotices.length + 1,
          firstTimestamp: previousNotice?.firstTimestamp ?? previous.timestamp,
          timestamps: [
            ...(previousNotice?.timestamps ?? [previous.timestamp].filter((value): value is number => Boolean(value))),
            ...(message.timestamp ? [message.timestamp] : []),
          ],
          hiddenNotices,
        },
      };
      continue;
    }
    collapsed.push(message);
  }
  return collapsed;
}

function noticesShareOwnerDay(
  previous: ChatHomeDisplayMessage,
  current: ChatHomeMessage,
  userTimezone?: string | null,
): boolean {
  // The group's day anchor is its LAST DATED timestamp — the visible card's
  // own timestamp when dated, else the newest dated entry hidden inside the
  // collapsed range. Falling back only to previous.timestamp would let an
  // UNDATED duplicate bridge two dated days: Jan5+undated merges into an
  // undated card, and a Jan6 notice would then merge too, hiding the
  // midnight boundary (Codex round-4).
  const previousDated = effectiveDatedTimestamp(previous);
  const currentTime = normalizeMessageTimestamp(current.timestamp);
  // An undated CURRENT asserts no day; a group with NO dated anchor at all
  // keeps the status-quo collapse (same as noticesAreNearby's undated path).
  if (!previousDated || !currentTime) return true;
  return chatDayKey(previousDated, userTimezone) === chatDayKey(currentTime, userTimezone);
}

function noticesAreNearby(previous: ChatHomeDisplayMessage, current: ChatHomeMessage, rule: NoticeCollapseRule): boolean {
  const previousTime = previous.timestamp;
  const currentTime = current.timestamp;
  if (!previousTime || !currentTime) return true;
  return Math.abs(currentTime - previousTime) <= rule.windowMs;
}

function collapsibleNoticeMatch(message: ChatHomeMessage): { rule: NoticeCollapseRule; key: string } | null {
  for (const rule of NOTICE_COLLAPSE_RULES) {
    const key = rule.groupKey(message);
    if (key) return { rule, key: `${rule.id}:${key}` };
  }
  return null;
}

function collapsedNoticeItem(message: ChatHomeDisplayMessage | ChatHomeMessage): CollapsedNoticeItem {
  return {
    id: message.id,
    text: message.text,
    severity: message.severity,
    ...(message.timestamp ? { timestamp: message.timestamp } : {}),
  };
}

export function ChatHomeView({
  email,
  messages,
  brief,
  value,
  busy,
  snapshotMessage,
  onChange,
  onSubmit,
  onSuggestion,
  onClearConversation,
  onOpenWork,
  onOpenStatus,
  onOpenSettings,
  voiceNote,
  onStartVoiceNote,
  onStopVoiceNote,
  onCancelVoiceNote,
  onRetry,
  onApprovalAction,
  onDecideApproval,
  onStopResearch,
  quotedMessage,
  onQuoteMessage,
  hasEarlier = false,
  loadingEarlier = false,
  earlierError,
  onLoadEarlier,
  onClearQuote,
  onAskWorkItem,
  onWorkItemAction,
  onOpenWorkDetail,
  onNoticeAction,
  attachedContext,
  onAttachContext,
  onClearAttachedContext,
  pendingAttachment,
  attachmentUploading = false,
  attachmentError,
  onAttachFile,
  onClearAttachment,
  contextSignals = [],
  needsYouItems = [],
  installOffered = true,
  onInstallOwnerApp,
  logoutBusy = false,
  onLogout,
  workBadgeCount = 0,
  workBadgeLabel = 'Work',
  workBadgeTone: _workBadgeTone = 'neutral',
  avatarProfile,
  voiceSummon = null,
  presentationMode = 'full',
  onCloseSheet,
  citeFocus = null,
  voiceLiveLine = '',
  voiceApproval = null,
  onVoiceApproval,
}: {
  email?: string;
  messages: ChatHomeMessage[];
  brief?: string;
  value: string;
  busy: boolean;
  snapshotMessage: string;
  contextSignals?: VNextHomeSignal[];
  /** Up to 3 owner work items that need a decision or action on the home desk. */
  needsYouItems?: OwnerWorkItem[];
  workBadgeCount?: number;
  workBadgeLabel?: string;
  workBadgeTone?: 'neutral' | 'warn';
  avatarProfile: AvatarProfile;
  /** Top-bar Aegis summon (duplex voice). When set, replaces static mark. */
  voiceSummon?: OwnerShellSummon | null;
  /**
   * `full` = Aegis tab (replaces Home). `sheet` = bottom sheet over Work/
   * Status/Settings. Same component + parent state in both modes.
   */
  presentationMode?: 'full' | 'sheet';
  /** Sheet mode only — drag-handle / close dismisses the overlay. */
  onCloseSheet?: () => void;
  /** Work-card jump: scroll to this cite, or leave the token visible when the
   *  message is not in the loaded window. */
  citeFocus?: { ref: string; token: string } | null;
  /** One line while 安琪 is speaking. It sits above the composer, then leaves. */
  voiceLiveLine?: string;
  /** One search card above that line while duplex voice waits for 批准 / 先不要. */
  voiceApproval?: { approvalId: string; query: string } | null;
  onVoiceApproval?: (action: 'approve' | 'reject') => void;
  onChange: (value: string) => void;
  onSubmit: (value?: string) => void;
  onSuggestion: (prompt: string, displayText?: string) => void;
  onClearConversation: () => void;
  onOpenWork: (itemId?: string | null) => void;
  onOpenStatus: () => void;
  onOpenSettings: () => void;
  /**
   * Composer voice-note state. The mic records a real voice note (audio →
   * server-side speech-to-text), NOT browser dictation: `recording` replaces
   * the text row with a timer + Cancel/Stop, `uploading` shows the transcribe
   * wait, `attached` holds a transcript pill until Send.
   */
  voiceNote?: ChatHomeVoiceNote;
  onStartVoiceNote?: () => void;
  onStopVoiceNote?: () => void;
  /** Discard the in-flight recording or the held note. */
  onCancelVoiceNote?: () => void;
  onRetry: (text: string, idempotencyKey?: string, context?: ChatHomeAttachedContext, quote?: ChatHomeQuote) => void;
  onApprovalAction?: (approvalId: string, action: 'approve' | 'reject') => void;
  /** 批准 / 先不要 while a chat card is waiting. True means the utterance was the decision, not a new turn. */
  onDecideApproval?: (action: 'approve' | 'reject') => boolean;
  onStopResearch?: () => void;
  quotedMessage?: ChatHomeQuote | null;
  onQuoteMessage: (quote: ChatHomeQuote) => void;
  onClearQuote: () => void;
  /** Older server history exists; show 加载更早 at the top of the stream. */
  hasEarlier?: boolean;
  loadingEarlier?: boolean;
  earlierError?: string;
  onLoadEarlier?: () => void;
  onAskWorkItem: (messageId: string, item: OwnerWorkItem, text: string) => void;
  onWorkItemAction: (item: OwnerWorkItem, action: VNextItemAction) => Promise<ChatHomeWorkActionResult>;
  onOpenWorkDetail: (item: OwnerWorkItem) => Promise<ChatHomeWorkDetailResult>;
  onNoticeAction: (action: DeliveryNoticeAction) => Promise<ChatHomeNoticeActionResult>;
  attachedContext?: ChatHomeAttachedContext | null;
  onAttachContext: (context: ChatHomeAttachedContext) => void;
  onClearAttachedContext: () => void;
  pendingAttachment?: ChatHomePendingAttachment | null;
  attachmentUploading?: boolean;
  attachmentError?: string;
  onAttachFile?: (file: File) => void;
  onClearAttachment?: () => void;
  /** When the app is not installed, the menu offers one install action. Hidden once installed. */
  installOffered?: boolean;
  onInstallOwnerApp: () => void;
  logoutBusy?: boolean;
  onLogout: () => void;
}) {
  const trFn = useT();
  const { locale } = useLocale();
  const tr = makeTranslator(trFn, locale);
  const hasConversation = messages.length > 0;
  const userTimezone = useUserTimezone();
  const displayMessages = collapsedChatHomeMessages(omitGatePendingOwnerAcks(messages), userTimezone);
  // 今天/昨天 labels must roll over while the app stays open across local
  // midnight (Codex): a cheap minutely check that only commits state (and so
  // only re-renders) when the owner-tz DAY KEY actually changes — the
  // same-value setState bails out on every other tick.
  const [dividerNowMs, setDividerNowMs] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => {
      setDividerNowMs((current) =>
        chatDayKey(Date.now(), userTimezone) === chatDayKey(current, userTimezone)
          ? current
          : Date.now(),
      );
    }, 60_000);
    return () => clearInterval(id);
  }, [userTimezone]);
  const dateDividers = useMemo(
    () => chatDateDividerLabels(displayMessages, userTimezone, locale, dividerNowMs),
    [displayMessages, userTimezone, locale, dividerNowMs],
  );
  const [contextPickerOpen, setContextPickerOpen] = useState(false);
  const [researchStopSent, setResearchStopSent] = useState(false);
  const [composerMode, setComposerMode] = useState<ComposerMode>('line');
  // Phone composer: while typing, the field takes the full row and the
  // bottom nav steps aside (owner screenshot 2026-09-08: four controls left a
  // one-line field about 40% of the screen wide).
  const [composerFocused, setComposerFocused] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const sendReasonId = useId();
  const composerPointerActive = useRef(false);
  const composerBlurPending = useRef(false);
  useEffect(() => {
    let finishTimer: ReturnType<typeof setTimeout> | undefined;
    const finish = () => {
      clearTimeout(finishTimer);
      composerPointerActive.current = false;
      if (!composerBlurPending.current) return;
      composerBlurPending.current = false;
      setComposerFocused(document.activeElement === inputRef.current);
    };
    const start = () => {
      clearTimeout(finishTimer);
      composerPointerActive.current = true;
    };
    // A blur during pointerdown must not restore the mobile nav/banner until
    // the click has its target. Otherwise the target moves before pointerup.
    // Touch compatibility focus/click events can arrive after pointerup in
    // another task. Keep the guard until click; only use the timer to settle
    // gestures that never produce a click (including cancelled activation).
    const end = () => {
      clearTimeout(finishTimer);
      finishTimer = setTimeout(finish, 500);
    };
    document.addEventListener('pointerdown', start, true);
    document.addEventListener('pointerup', end, true);
    document.addEventListener('click', finish, true);
    document.addEventListener('pointercancel', finish, true);
    window.addEventListener('blur', finish);
    return () => {
      clearTimeout(finishTimer);
      document.removeEventListener('pointerdown', start, true);
      document.removeEventListener('pointerup', end, true);
      document.removeEventListener('click', finish, true);
      document.removeEventListener('pointercancel', finish, true);
      window.removeEventListener('blur', finish);
    };
  }, [inputRef]);
  const [openActionsId, setOpenActionsId] = useState<string | null>(null);
  // Recording (or waiting on the transcript) takes over the composer row.
  const voiceRecording = voiceNote?.status === 'recording' || voiceNote?.status === 'uploading';
  // Send stays live with an empty textarea whenever the composer holds
  // something: a transcribed voice note, or an uploaded file. The backend
  // supplies the caption for a file-only turn.
  const canSendAttachmentOnly = voiceNote?.status === 'attached' || Boolean(pendingAttachment);
  // Any full-width pill above the control row pushes the controls to row 2.
  const hasPendingAttachmentRow = Boolean(pendingAttachment)
    || attachmentUploading
    || Boolean(attachmentError)
    || voiceNote?.status === 'attached'
    || voiceNote?.status === 'error';
  // Scroll-driven topbar collapse (iOS large-title): full identity at the top of
  // the conversation, slim bar once scrolled into the messages. Only meaningful
  // when hasConversation (the stream is the scroll container then).
  const [topbarCollapsed, setTopbarCollapsed] = useState(false);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  // CEO desk: items dismissed after a successful local action until the queue
  // refresh drops them. Receipt is a one-line status under the desk heading.
  const [dismissedNeedsYouIds, setDismissedNeedsYouIds] = useState<string[]>([]);
  const [deskReceipt, setDeskReceipt] = useState('');
  const streamRef = useRef<HTMLElement | null>(null);
  const citeScrolledRef = useRef<string | null>(null);
  const [citeLanded, setCiteLanded] = useState(false);
  useLayoutEffect(() => {
    if (!citeFocus?.ref) {
      citeScrolledRef.current = null;
      setCiteLanded(false);
      return undefined;
    }
    const stream = streamRef.current;
    const escaped = window.CSS?.escape ? window.CSS.escape(citeFocus.ref) : citeFocus.ref;
    const button = stream?.querySelector<HTMLElement>(`[data-message-ref="${escaped}"]`);
    const article = button?.closest('article');
    if (!stream || !article) {
      setCiteLanded(false);
      return undefined;
    }
    stream.querySelectorAll('.is-cite-target').forEach((el) => el.classList.remove('is-cite-target'));
    article.classList.add('is-cite-target');
    if (citeScrolledRef.current !== citeFocus.ref) {
      const streamTop = stream.getBoundingClientRect().top;
      const articleTop = article.getBoundingClientRect().top;
      stream.scrollTop += articleTop - streamTop - 12;
      citeScrolledRef.current = citeFocus.ref;
    }
    setCiteLanded(true);
    return undefined;
  }, [citeFocus?.ref, messages, presentationMode]);
  // Set when 加载更早 is pressed: keep the owner's place while older turns are
  // prepended (otherwise the stream would jump to the oldest loaded turn).
  // Anchored to the message that was first on screen: the prepended block can
  // keep growing after the commit (fonts, avatars, disclosures), so the restore
  // repeats on the next frames instead of trusting one scrollHeight delta.
  const prependAnchorRef = useRef<{ messageId: string; offset: number } | null>(null);
  // Length at which a prepend was just restored: the bottom auto-scroll effect
  // below must not undo it (it fires on the same commit when a turn is in flight).
  const prependRestoredLengthRef = useRef<number | null>(null);
  useLayoutEffect(() => {
    const anchor = prependAnchorRef.current;
    const stream = streamRef.current;
    if (!anchor || !stream) return;
    prependAnchorRef.current = null;
    prependRestoredLengthRef.current = messages.length;
    const restore = () => {
      const el = stream.querySelector<HTMLElement>(`[data-message-id="${anchor.messageId}"]`);
      if (!el) return;
      const streamTop = stream.getBoundingClientRect().top;
      const elTop = el.getBoundingClientRect().top;
      stream.scrollTop += elTop - streamTop - anchor.offset;
    };
    restore();
    window.requestAnimationFrame(restore);
    const timers = [120, 320, 600].map((ms) => window.setTimeout(restore, ms));
    return () => timers.forEach((id) => window.clearTimeout(id));
  }, [messages.length]);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const streamNearBottomRef = useRef(true);
  const streamClassName = [
    'vnext-chat-stream',
    busy ? 'is-working' : '',
    hasConversation ? '' : 'is-empty',
  ].filter(Boolean).join(' ');
  const briefCard = compactHomeBrief(brief, tr);
  const briefText = briefCard?.text ?? '';
  const dismissedNeedsYouSet = useMemo(() => new Set(dismissedNeedsYouIds), [dismissedNeedsYouIds]);
  const visibleNeedsYouItems = useMemo(
    () => needsYouItems.filter((item) => !dismissedNeedsYouSet.has(item.id) && !isClosedItem(item)),
    [needsYouItems, dismissedNeedsYouSet],
  );
  // When needs-you cards already show the same decisions, hide the hero brief
  // so the empty desk does not triple-list one item (screenshot audit).
  const showHeroBrief = Boolean(briefText) && visibleNeedsYouItems.length === 0;
  const leadContextSignal = !hasConversation && showHeroBrief ? contextSignals[0] : null;
  const visibleContextSignals = leadContextSignal ? contextSignals.slice(1) : contextSignals;
  const visibleSignals = visibleContextSignals.slice(0, 3);
  const hiddenSignalCount = Math.max(visibleContextSignals.length - visibleSignals.length, 0);
  const homeClassName = [
    'vnext-chat-home',
    hasConversation ? 'has-conversation' : 'is-empty',
    contextPickerOpen ? 'has-context-picker' : '',
    visibleNeedsYouItems.length > 0 ? 'has-needs-you-desk' : '',
    presentationMode === 'sheet' ? 'is-sheet' : 'is-full',
    `composer-${composerMode}`,
  ].join(' ');
  const threadActivityKey = messages
    .map((message) => `${message.id}:${message.workItemThread?.map((entry) => `${entry.id}:${entry.result ? 'done' : entry.state ?? 'working'}`).join(',') ?? ''}`)
    .join('|');

  function handleSignal(signal: VNextHomeSignal) {
    setContextPickerOpen(false);
    if (signal.id === 'sources') {
      onOpenSettings();
      return;
    }
    if (signal.id === 'quiet') {
      onSuggestion('我现在有什么需要注意的吗？');
      return;
    }
    onOpenWork(signal.targetItemId);
  }

  function handleAttachSignal(signal: VNextHomeSignal) {
    if (!signal.targetItemId) {
      handleSignal(signal);
      return;
    }
    onAttachContext({
      itemId: signal.targetItemId,
      label: signal.summary || signal.label,
    });
    setContextPickerOpen(false);
  }

  function selectQuote(message: ChatHomeMessage) {
    const quote = quoteFromChatMessage(message);
    if (!quote) return;
    onQuoteMessage(quote);
    window.requestAnimationFrame(() => inputRef.current?.focus({ preventScroll: true }));
  }

  function renderContextSignals(mode: 'open' | 'attach' = 'open', showAll = false) {
    const signals = showAll ? visibleContextSignals : visibleSignals;
    const overflowCount = showAll ? 0 : hiddenSignalCount;
    if (signals.length === 0) return null;
    return (
      <div
        className="vnext-chat-context-strip"
        aria-label={mode === 'attach' ? tr.t('chatHome.context.attachable') : tr.t('chatHome.context.active')}
      >
        {signals.map((signal) => (
          <button
            key={signal.id}
            type="button"
            className={`tone-${signal.tone}`}
            onClick={() => (mode === 'attach' ? handleAttachSignal(signal) : handleSignal(signal))}
          >
            <span>{signal.label}</span>
            <strong>{signal.summary}</strong>
          </button>
        ))}
        {overflowCount > 0 && (
          <button
            type="button"
            className="vnext-chat-context-more"
            aria-label={tr.f(overflowCount === 1 ? 'chatHome.context.openMoreOne' : 'chatHome.context.openMoreMany', { count: String(overflowCount) })}
            onClick={() => onOpenWork()}
          >
            <span>{tr.t('chatHome.session.work')}</span>
            <strong>{tr.f(overflowCount === 1 ? 'chatHome.context.workItemsOne' : 'chatHome.context.workItemsMany', { count: String(overflowCount) })}</strong>
          </button>
        )}
      </div>
    );
  }

  async function handleNeedsYouAction(item: OwnerWorkItem, action: VNextItemAction): Promise<ChatHomeWorkActionResult> {
    const result = await onWorkItemAction(item, action);
    if (result.ok) {
      const closed = Boolean(result.nextStatus && isClosedItem({ ...item, status: result.nextStatus }));
      // Dismiss when the action closed the item, or any successful decision-like
      // action so the CEO desk clears immediately without waiting for refresh.
      if (closed || result.itemUnavailable || itemNeedsDecision(item) || action.id.startsWith('approval.')) {
        setDismissedNeedsYouIds((current) => (current.includes(item.id) ? current : current.concat(item.id)));
      }
      setDeskReceipt(result.message || tr.t('chatHome.needsYou.done'));
    }
    return result;
  }

  function dismissNeedsYouBanner(itemId: string) {
    setDismissedNeedsYouIds((current) => (current.includes(itemId) ? current : current.concat(itemId)));
  }

  /** One dismissible banner above the composer — never inside the timeline. */
  function renderNeedsYouBanner() {
    if (visibleNeedsYouItems.length === 0) return null;
    const lead = visibleNeedsYouItems[0];
    const moreCount = Math.max(visibleNeedsYouItems.length - 1, 0);
    const title = ownerVisibleWorkTitle(lead, undefined, 96, locale) || lead.title || tr.t('chatHome.needsYou.title');
    return (
      <aside
        className="vnext-chat-needs-you vnext-chat-needs-you-banner"
        data-testid="aegis-needs-you-banner"
        aria-label={tr.t('chatHome.needsYou.bannerAria')}
      >
        <div className="vnext-chat-needs-you-banner-body">
          <p>
            <span className="vnext-chat-needs-you-kicker">{tr.t('chatHome.needsYou.kicker')}</span>
            {tr.f('chatHome.needsYou.banner', { title })}
          </p>
          {moreCount > 0 && (
            <em>
              {tr.f(
                moreCount === 1 ? 'chatHome.needsYou.moreOnWorkOne' : 'chatHome.needsYou.moreOnWorkMany',
                { count: String(moreCount) },
              )}
            </em>
          )}
        </div>
        <div className="vnext-chat-needs-you-banner-actions">
          <button
            type="button"
            className="vnext-chat-needs-you-handle"
            onClick={() => onOpenWork(lead.id)}
          >
            {tr.t('chatHome.needsYou.handle')}
          </button>
          <button
            type="button"
            className="vnext-chat-needs-you-later"
            aria-label={tr.t('chatHome.needsYou.dismissAria')}
            onClick={() => dismissNeedsYouBanner(lead.id)}
          >
            {tr.t('chatHome.needsYou.later')}
          </button>
        </div>
      </aside>
    );
  }

  function renderNeedsYouDesk(opts: { compact?: boolean; className?: string } = {}) {
    // Empty-home / desktop-rail still use a compact desk when there is no
    // conversation yet; conversation mode uses the banner above the composer.
    const { compact = false, className } = opts;
    if (hasConversation) return null;
    if (visibleNeedsYouItems.length === 0 && !deskReceipt && visibleContextSignals.length === 0 && workBadgeCount === 0) {
      return null;
    }
    const cards = compact ? visibleNeedsYouItems.slice(0, 1) : visibleNeedsYouItems;
    const moreCount = compact ? Math.max(visibleNeedsYouItems.length - cards.length, 0) : 0;
    return (
      <section className={classNames('vnext-chat-needs-you', compact && 'is-compact', className)} aria-label={tr.t('chatHome.needsYou.title')}>
        <div className="vnext-chat-needs-you-heading">
          <p className="vnext-kicker">{tr.t('chatHome.needsYou.kicker')}</p>
          <h3>{tr.t('chatHome.needsYou.title')}</h3>
        </div>
        {deskReceipt && (
          <p className="vnext-chat-needs-you-receipt" role="status">{deskReceipt}</p>
        )}
        {visibleNeedsYouItems.length > 0 ? (
          <div className="vnext-chat-needs-you-cards" aria-label={tr.t('chatHome.needsYou.cardsLabel')}>
            {cards.map((item) => (
              <ChatWorkCard
                key={item.id}
                item={item}
                busy={busy}
                thread={[]}
                onOpenDetail={(target) => onOpenWorkDetail(target)}
                onAsk={(text) => onAskWorkItem(`needs-you:${item.id}`, item, text)}
                onAction={(action) => handleNeedsYouAction(item, action)}
              />
            ))}
          </div>
        ) : (
          !compact && renderContextSignals()
        )}
        {moreCount > 0 && (
          <button type="button" className="vnext-chat-needs-you-open-work" onClick={() => onOpenWork()}>
            <VNextIcon name="work" />
            <span>{tr.f(moreCount === 1 ? 'chatHome.needsYou.moreOne' : 'chatHome.needsYou.moreMany', { count: String(moreCount) })}</span>
          </button>
        )}
        {visibleNeedsYouItems.length === 0 && !compact && visibleContextSignals.length === 0 && workBadgeCount > 0 && (
          <button type="button" className="vnext-chat-needs-you-open-work" onClick={() => onOpenWork()}>
            <VNextIcon name="work" />
            <span>{workBadgeLabel || tr.t('chatHome.destinations.workHint')}</span>
          </button>
        )}
      </section>
    );
  }

  // Reset the collapse when leaving conversation state (e.g. clear chat). The
  // scroll handler only runs on stream scroll, so without this a collapsed
  // topbar would strand into the NEXT conversation whose first (short) message
  // never fires a corrective scroll event. Empty → start-at-top → expanded.
  useEffect(() => {
    if (!hasConversation) setTopbarCollapsed(false);
  }, [hasConversation]);

  // Drop dismissals for items that left the prop list (queue refresh).
  useEffect(() => {
    const live = new Set(needsYouItems.map((item) => item.id));
    setDismissedNeedsYouIds((current) => current.filter((id) => live.has(id)));
  }, [needsYouItems]);

  // One-line desk receipt auto-clears so it never becomes permanent chrome.
  useEffect(() => {
    if (!deskReceipt) return undefined;
    const timer = window.setTimeout(() => setDeskReceipt(''), 4500);
    return () => window.clearTimeout(timer);
  }, [deskReceipt]);

  useEffect(() => {
    const stream = streamRef.current;
    if (!stream) return;
    if (prependRestoredLengthRef.current === messages.length) {
      prependRestoredLengthRef.current = null;
      return;
    }
    const latestRole = messages[messages.length - 1]?.role;
    const shouldAutoScroll =
      messages.length <= 1
      || streamNearBottomRef.current
      || latestRole === 'owner'
      || latestRole === 'queued'
      || latestRole === 'working';
    if (!shouldAutoScroll) return;
    const behavior: ScrollBehavior = messages.length > 1 ? 'smooth' : 'auto';
    stream.scrollTo({ top: stream.scrollHeight, behavior });
    streamNearBottomRef.current = true;
    setShowJumpToLatest(false);
    window.requestAnimationFrame(() => {
      stream.scrollTop = stream.scrollHeight;
      streamNearBottomRef.current = true;
      bottomRef.current?.scrollIntoView({ block: 'end', behavior });
    });
  }, [messages.length, busy, threadActivityKey]);

  // Pulling the sheet up on another page must land on the latest turn, not
  // the top of the history (owner: "no place to type" — the composer sits
  // below a long timeline otherwise). Same scroll as a new message.
  useEffect(() => {
    if (presentationMode !== 'sheet') return;
    const stream = streamRef.current;
    if (!stream) return;
    stream.scrollTop = stream.scrollHeight;
    streamNearBottomRef.current = true;
    window.requestAnimationFrame(() => {
      stream.scrollTop = stream.scrollHeight;
      bottomRef.current?.scrollIntoView({ block: 'end' });
    });
  }, [presentationMode]);

  useEffect(() => {
    resizeComposerInput(inputRef.current, composerMode);
  }, [value, composerMode]);

  // Re-focus only when the owner changes the composer size, never on mount:
  // a load-time focus put the phone composer into its typing layout (nav
  // hidden, keyboard guess) with no keyboard open (2026-09-08).
  const composerModeSeenRef = useRef(false);
  useEffect(() => {
    if (!composerModeSeenRef.current) {
      composerModeSeenRef.current = true;
      return;
    }
    inputRef.current?.focus({ preventScroll: true });
  }, [composerMode]);

  // Everyday overflow is an overlay (portaled, fixed). Settings and runtime
  // status stay here. Pause-checks and support snapshot live in Settings.
  // Clear and sign-out stay at the bottom, separated, both destructive.
  // Utility peers still come from the shared helper so destinations cannot drift.
  const utilityItems = ownerUtilityMenuItemIds().map((id): VNextHeaderMenuItem => (
    id === 'status'
      ? { id, label: tr.t('chatHome.account.aegisStatus'), icon: 'status', onSelect: onOpenStatus }
      : { id, label: tr.t('chatHome.account.settings'), icon: 'settings', onSelect: onOpenSettings }
  ));
  const everydayItems = (['settings', 'status'] as const).flatMap((id) => {
    const item = utilityItems.find((entry) => entry.id === id);
    return item ? [item] : [];
  });
  const sessionItems: VNextHeaderMenuItem[] = [];
  if (hasConversation) {
    sessionItems.push({
      id: 'clear',
      label: tr.t('chatHome.account.clearChat'),
      icon: 'trash',
      tone: 'destructive',
      separatorBefore: true,
      onSelect: onClearConversation,
    });
  }
  sessionItems.push({
    id: 'logout',
    label: logoutBusy ? tr.t('chatHome.account.signingOut') : tr.t('chatHome.account.signOut'),
    icon: 'logout',
    tone: 'destructive',
    disabled: logoutBusy,
    separatorBefore: !hasConversation,
    onSelect: onLogout,
  });
  const homeMenuItems: VNextHeaderMenuItem[] = [
    ...(installOffered ? [{
      id: 'install',
      label: tr.t('install.action.installOnDevice'),
      icon: 'home' as const,
      onSelect: onInstallOwnerApp,
    }] : []),
    ...everydayItems,
    ...sessionItems,
  ];
  // CEO Intent Contract: one-word / short utterance → visible understanding cue.
  const shorthandIntent = resolveCeoShorthand(value, locale);
  const attachmentBlockingSend = Boolean(attachmentUploading);
  const sendDisabled = attachmentBlockingSend || (!value.trim() && !canSendAttachmentOnly);
  const needTextReason = tr.t('chatHome.composer.needText');
  const sendDisabledReason = attachmentBlockingSend
    ? tr.t('chatHome.composer.waitUpload')
    : needTextReason;
  // The placeholder already says to type. A second floating sentence repeats
  // it. Other disabled states (an upload still in flight) keep an adjacent reason.
  const showAdjacentSendReason = sendDisabled && sendDisabledReason !== needTextReason;
  const showComposerMeta = Boolean(
    shorthandIntent
    || attachedContext
    || hasPendingAttachmentRow
    || quotedMessage,
  );

  function submitComposer() {
    if (attachmentBlockingSend) return;
    const raw = (inputRef.current?.value ?? value).trim();
    if (!raw) {
      if (canSendAttachmentOnly) onSubmit('');
      return;
    }
    const decision = voicePendingDecision(raw);
    if (decision && onDecideApproval?.(decision)) {
      onChange('');
      return;
    }
    const resolved = resolveCeoShorthand(raw, locale);
    if (resolved) {
      onSuggestion(resolved.expansion, raw);
      return;
    }
    onSubmit(raw);
  }

  return (
    <section
      className={homeClassName}
      aria-label={tr.t('chatHome.shell.chatHome')}
      data-testid={presentationMode === 'sheet' ? 'aegis-chat-sheet' : 'aegis-chat-full'}
      data-presentation-mode={presentationMode}
    >
      {presentationMode === 'sheet' ? (
        <div className="vnext-chat-sheet-chrome">
          <button
            type="button"
            className="vnext-chat-sheet-handle"
            aria-label={tr.t('chatHome.sheet.handleAria')}
            data-testid="aegis-chat-sheet-handle"
            onClick={onCloseSheet}
          >
            <span className="vnext-chat-sheet-handle-bar" aria-hidden="true" />
          </button>
          <button
            type="button"
            className="vnext-chat-sheet-close"
            aria-label={tr.t('chatHome.sheet.closeAria')}
            data-testid="aegis-chat-sheet-close"
            onClick={onCloseSheet}
          >
            <VNextIcon name="close" />
          </button>
        </div>
      ) : null}
      {/* Sheet mode: the page beneath already shows the shell header, so the
          panel renders only handle + close there (owner: no duplicate chrome). */}
      {presentationMode === 'full' ? (
        <OwnerShellChrome
          variant="chat"
          kicker={tr.t('chatHome.kicker.privateSecretary')}
          title={tr.t('chatHome.role.aegis')}
          collapsed={hasConversation && topbarCollapsed}
          summon={voiceSummon}
          leading={voiceSummon ? undefined : (
            <div className="vnext-chat-identity-label">
              <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-mark" />
              <span>
                <span className="vnext-kicker">{tr.t('chatHome.kicker.privateSecretary')}</span>
                <strong>{tr.t('chatHome.role.aegis')}</strong>
              </span>
            </div>
          )}
          /* Peers (Work) live in bottom nav / brand rail only — one path. */
          statusText={busy ? tr.t('chatHome.session.working') : tr.f('chatHome.session.signedIn', { name: email || tr.t('chatHome.role.you') })}
          menuTriggerLabel={tr.t('chatHome.account.menu')}
          menuItems={homeMenuItems}
        />
      ) : null}

      <main
        ref={streamRef}
        className={streamClassName}
        data-testid={presentationMode === 'sheet' ? 'aegis-chat-sheet-stream' : undefined}
        aria-live="polite"
        aria-relevant="additions text"
        onScroll={(event) => {
          streamNearBottomRef.current = isNearStreamBottom(event.currentTarget);
          setShowJumpToLatest((current) => {
            const next = hasConversation && !streamNearBottomRef.current;
            return current === next ? current : next;
          });
          // 24px hysteresis: expanded only while essentially pinned to the top.
          // Gated on hasConversation so scrolling the (also overflow-y:auto) empty
          // hero never writes a collapsed value that would strand into the next
          // conversation's first short message. Empty stream → always expanded.
          setTopbarCollapsed(hasConversation && event.currentTarget.scrollTop > 24);
        }}
      >
        {hasConversation && hasEarlier && onLoadEarlier && (
          <button
            type="button"
            className="vnext-chat-load-earlier vnext-btn vnext-btn-secondary vnext-btn-sm"
            data-testid="aegis-chat-load-earlier"
            disabled={loadingEarlier}
            onClick={() => {
              const stream = streamRef.current;
              if (stream) {
                const streamTop = stream.getBoundingClientRect().top;
                const first = [...stream.querySelectorAll<HTMLElement>('[data-message-id]')]
                  .find((el) => el.getBoundingClientRect().bottom > streamTop);
                if (first) prependAnchorRef.current = { messageId: first.dataset.messageId || '', offset: first.getBoundingClientRect().top - streamTop };
              }
              onLoadEarlier();
            }}
          >
            {loadingEarlier ? tr.t('chatHome.loadEarlier.loading') : tr.t('chatHome.loadEarlier.label')}
          </button>
        )}
        {earlierError && <p role="status" className="vnext-chat-inline-note" data-testid="aegis-chat-earlier-error">{earlierError}</p>}
        {citeFocus && !citeLanded && (
          <p role="status" className="vnext-chat-cite-focus" data-testid="aegis-chat-cite-focus" data-cite-focus={citeFocus.ref}>
            {tr.f('chatHome.citeFocus.outsideWindow', { ref: citeFocus.token })}
          </p>
        )}
        {!hasConversation && (
          <section className="vnext-chat-empty-stage">
            <section className="vnext-chat-hero">
              <div className="vnext-chat-presence" aria-hidden="true">
                <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-presence-avatar" />
                <span className="vnext-chat-presence-ring one" />
                <span className="vnext-chat-presence-ring two" />
              </div>
              <p className="vnext-kicker">{tr.t('chatHome.hero.ready')}</p>
              <h2>{tr.t('chatHome.hero.tellAegis')}</h2>
              <p className="vnext-chat-hero-hint">{tr.t('chatHome.hero.hint')}</p>
              {showHeroBrief && (
                <button
                  type="button"
                  className="vnext-chat-brief-card"
                  aria-label={tr.t('chatHome.hero.openBrief')}
                  onClick={() => {
                    const leadSignal = contextSignals[0];
                    if (leadSignal) {
                      handleSignal(leadSignal);
                    } else {
                      onSuggestion('我现在有什么需要注意的吗？');
                    }
                  }}
                >
                  <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
                  <span>
                    <em>{briefCard?.label ?? tr.t('chatHome.brief.firstLook')}</em>
                    <strong>{briefText}</strong>
                  </span>
                </button>
              )}
              <div className="vnext-chat-starters" aria-label={tr.t('chatHome.hero.starterPrompts')}>
                {STARTER_PROMPTS.map((prompt) => {
                  const promptLabel = tr.t(prompt.labelKey);
                  return (
                    <button
                      key={prompt.id}
                      type="button"
                      onClick={() => onSuggestion(prompt.prompt, promptLabel)}
                    >
                      <VNextIcon name={prompt.icon} />
                      <span>{promptLabel}</span>
                    </button>
                  );
                })}
              </div>
              {visibleNeedsYouItems.length === 0 && renderNeedsYouDesk()}
            </section>
          </section>
        )}

        {displayMessages.map((message, messageIndex) => (
          <Fragment key={message.id}>
            {dateDividers[message.id] && (
              <div className="vnext-chat-date-divider" role="separator" aria-label={dateDividers[message.id]}>
                <span>{dateDividers[message.id]}</span>
              </div>
            )}
          <article
            data-message-id={message.id}
            className={`vnext-chat-message ${message.role}${message.streamText ? ' is-streaming' : ''}${openActionsId === message.id ? ' show-actions' : ''}`}
            onClick={(event) => {
              // Tap a bubble to reveal its reply action (owner 2026-09-08:
              // the always-on arrow and the 你 label made bubbles heavy).
              if ((event.target as HTMLElement).closest('button, a, input, textarea, select, summary')) return;
              setOpenActionsId((current) => (current === message.id ? null : message.id));
            }}
          >
            {(message.role === 'owner' || message.role === 'queued') && (
              <>
                <span className="vnext-chat-message-author">
                  <OwnerAvatar src={avatarProfile.ownerAvatarUrl} email={email} className="vnext-chat-avatar" />
                  {message.role === 'queued' && <span>{tr.t('chatHome.role.queued')}</span>}
                  <MessageTime timestamp={message.timestamp} />
                  {message.role === 'owner' && <ChatMessageCite message={message} />}
                  {message.role === 'owner' && (
                    <>
                      <button
                        type="button"
                        className="vnext-chat-quote-button"
                        aria-label={tr.t('chatHome.message.replyToMessage')}
                        title={tr.t('chatHome.message.replyToMessage')}
                        onClick={() => selectQuote(message)}
                      >
                        <VNextIcon name="reply" />
                      </button>
                    </>
                  )}
                </span>
                {message.contextLabel && (
                  <span className="vnext-chat-message-context">
                    <VNextIcon name="work" />
                    {tr.f('chatHome.message.using', { label: message.contextLabel })}
                  </span>
                )}
                {message.quote && <ChatQuotePreview quote={message.quote} tr={tr} />}
                {message.attachment && (
                  <ChatMessageAttachment attachment={message.attachment} tr={tr} />
                )}
                {/* A voice note's transcript already renders inside its chip —
                    do not print it twice when it IS the message text. */}
                {messageBodyText(message) && <p>{messageBodyText(message)}</p>}
                {message.role === 'queued' && (
                  <span className="vnext-chat-message-state">{tr.t('chatHome.role.queued')}</span>
                )}
              </>
            )}
            {message.role === 'working' && (
              <div className="vnext-chat-working-body">
                <p className="vnext-chat-working-head">
                  <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
                  <span className="vnext-chat-working-dot" aria-hidden="true" />
                  <MessageTime timestamp={message.timestamp} />
                </p>
                {message.streamText ? (
                  <p className="vnext-chat-stream-text" data-testid="aegis-chat-stream">{message.streamText}</p>
                ) : null}
                <p className="vnext-chat-working-text">{message.text}</p>
                <ResearchStopControl
                  text={message.text}
                  stopped={researchStopSent}
                  onStop={() => {
                    setResearchStopSent(true);
                    onStopResearch?.();
                  }}
                  label={tr.t('chatHome.search.stop')}
                  stoppedLabel={tr.t('chatHome.search.stopped')}
                />
              </div>
            )}
            {message.role === 'approval' && (
              <div className="vnext-chat-answer-card vnext-chat-approval-card vnext-ceo-review-card">
                <div className="vnext-chat-answer-heading">
                  <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
                  <strong>
                    <VNextIcon name="shield" /> {tr.t('chatHome.approval.title')}
                  </strong>
                  <MessageTime timestamp={message.timestamp} />
                </div>
                {/* CEO Review Card: What / Why / Approve — never raw tool ids as primary.
                    An egress hold replaces those lines with destination, the
                    outbound body, and why the gate stopped. */}
                <dl className="vnext-ceo-review-facts">
                  <ApprovalFacts
                    message={message}
                    locale={locale}
                    ownerQuestion={searchOwnerQuestion(message, displayMessages, messageIndex)}
                  />
                </dl>
                {isSearchHold(message.summary, message.toolName) && message.approvalPhase === 'running' && (
                  <p className="vnext-chat-approval-progress" data-testid="search-approval-progress" role="status">
                    {tr.t('chatHome.approval.searchRunning')}
                  </p>
                )}
                {isSearchHold(message.summary, message.toolName) && message.approvalPhase === 'done' && (
                  <p className="vnext-chat-approval-outcome" data-testid="search-approval-result">
                    {message.approvalOutcome?.trim() || tr.t('chatHome.approval.searchContinued')}
                  </p>
                )}
                {isSearchHold(message.summary, message.toolName) && message.approvalPhase === 'failed' && (
                  <p className="vnext-chat-approval-outcome is-failed" data-testid="search-approval-failure" role="alert">
                    {message.approvalOutcome?.trim() || tr.t('chatHome.approval.searchFailed')}
                  </p>
                )}
                {(!message.approvalState || message.approvalState === 'pending') ? (
                  <div className="vnext-action-row vnext-chat-approval-actions">
                    <VNextButton
                      variant="primary"
                      className="vnext-chat-approval-approve"
                      disabled={!message.approvalId}
                      onClick={() => {
                        if (message.approvalId && onApprovalAction) onApprovalAction(message.approvalId, 'approve');
                      }}
                    >
                      {tr.t('chatHome.approval.approve')}
                    </VNextButton>
                    <VNextButton
                      variant="secondary"
                      className="vnext-chat-approval-reject"
                      disabled={!message.approvalId}
                      onClick={() => {
                        if (message.approvalId && onApprovalAction) onApprovalAction(message.approvalId, 'reject');
                      }}
                    >
                      {tr.t('chatHome.approval.reject')}
                    </VNextButton>
                  </div>
                ) : (
                  <span className={`vnext-chat-approval-state ${message.approvalState}`}>
                    {tr.t(message.approvalState === 'approved' ? 'chatHome.approval.approved' : 'chatHome.approval.rejected')}
                  </span>
                )}
              </div>
            )}
            {message.role === 'aegis' && message.result && (
              <div className="vnext-chat-answer-card">
                <div className="vnext-chat-answer-heading">
                  <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
                  <strong>{chatHomeResultTitle(message.result, tr)}</strong>
                  <MessageTime timestamp={message.timestamp} latencyMs={replyLatencyMs(displayMessages, messageIndex)} />
                  <ChatMessageCite message={message} />
                  <button
                    type="button"
                    className="vnext-chat-quote-button"
                    aria-label={tr.t('chatHome.message.replyToAnswer')}
                    title={tr.t('chatHome.message.replyToAnswer')}
                    onClick={() => selectQuote(message)}
                  >
                    <VNextIcon name="reply" />
                  </button>

                </div>
                <CollapsedNoticeBadge notice={message.collapsedNotice} tr={tr} />
                <AssistantResponseView result={message.result} ownerQuestion={precedingOwnerQuestion(displayMessages, messageIndex)} />
                {message.executorFooter && (
                  <p className="vnext-chat-executor-footer" data-testid="aegis-executor-footer">{message.executorFooter}</p>
                )}
                {message.workItem && (
                  <ChatWorkCard
                    item={message.workItem}
                    unavailable={Boolean(message.workItemUnavailable)}
                    busy={busy}
                    thread={message.workItemThread ?? []}
                    onOpenDetail={(item) => onOpenWorkDetail(item)}
                    onAsk={(text) => {
                      if (message.workItem) onAskWorkItem(message.id, message.workItem, text);
                    }}
                    onAction={(action) => {
                      if (!message.workItem) {
                        return Promise.resolve({ ok: false, message: tr.t('chatHome.work.noWorkItem'), nextStatus: null });
                      }
                      return onWorkItemAction(message.workItem, action);
                    }}
                  />
                )}
                {message.result.response_kind === 'error' && (
                  <ChatRetryAction message={message} busy={busy} tr={tr} onRetry={onRetry} />
                )}
              </div>
            )}
            {message.role === 'aegis' && !message.result && (
              <div className="vnext-chat-answer-card compact">
                <div className="vnext-chat-answer-heading">
                  <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
                  <strong>{tr.t('chatHome.role.aegis')}</strong>
                  <MessageTime timestamp={message.timestamp} latencyMs={replyLatencyMs(displayMessages, messageIndex)} />
                  <ChatMessageCite message={message} />
                  <button
                    type="button"
                    className="vnext-chat-quote-button"
                    aria-label={tr.t('chatHome.message.replyToAnswer')}
                    title={tr.t('chatHome.message.replyToAnswer')}
                    onClick={() => selectQuote(message)}
                  >
                    <VNextIcon name="reply" />
                  </button>

                </div>
                <CollapsedNoticeBadge notice={message.collapsedNotice} tr={tr} />
                <AegisPlainMessage
                  text={message.text}
                  severity={message.severity}
                  importanceDismissKey={message.id}
                  inlineWorkAvailable={Boolean(message.workItem)}
                  ownerQuestion={precedingOwnerQuestion(displayMessages, messageIndex)}
                  onAction={(action) => onNoticeAction(action)}
                />
                <ResearchStopControl
                  text={message.text}
                  stopped={researchStopSent}
                  onStop={() => {
                    setResearchStopSent(true);
                    onStopResearch?.();
                  }}
                  label={tr.t('chatHome.search.stop')}
                  stoppedLabel={tr.t('chatHome.search.stopped')}
                />
                <ChatRetryAction message={message} busy={busy} tr={tr} onRetry={onRetry} />
                {message.repeatCount && message.repeatCount > 1 ? (
                  <RepeatedAlertNote count={message.repeatCount} firstTimestamp={message.firstTimestamp} />
                ) : null}
                {message.workItem && (
                  <ChatWorkCard
                    item={message.workItem}
                    unavailable={Boolean(message.workItemUnavailable)}
                    busy={busy}
                    thread={message.workItemThread ?? []}
                    onOpenDetail={(item) => onOpenWorkDetail(item)}
                    onAsk={(text) => onAskWorkItem(message.id, message.workItem as OwnerWorkItem, text)}
                    onAction={(action) => onWorkItemAction(message.workItem as OwnerWorkItem, action)}
                  />
                )}
              </div>
            )}
          </article>
          </Fragment>
        ))}

        {snapshotMessage && (
          <p className={snapshotMessage.startsWith('Saved') ? 'vnext-chat-inline-note ok' : 'vnext-chat-inline-note'}>
            {snapshotMessage}
          </p>
        )}
        <div ref={bottomRef} className="vnext-chat-bottom-anchor" aria-hidden="true" />
      </main>
      {hasConversation && showJumpToLatest ? (
        <button
          type="button"
          className="vnext-chat-jump-latest"
          data-testid="aegis-chat-jump-latest"
          onClick={() => {
            const stream = streamRef.current;
            if (!stream) return;
            streamNearBottomRef.current = true;
            setShowJumpToLatest(false);
            // The stream's CSS scroll-behavior is smooth, and Chrome still
            // animates scrollTo({ behavior: 'auto' }). An in-between scroll
            // event would show this control again. Force an instant jump.
            const previous = stream.style.scrollBehavior;
            stream.style.scrollBehavior = 'auto';
            stream.scrollTop = stream.scrollHeight;
            stream.style.scrollBehavior = previous;
          }}
        >
          {tr.t('chatHome.jumpLatest')}
        </button>
      ) : null}

      {/* Desktop conversation: context signals only in the rail (needs-you is the banner). */}
      {hasConversation && visibleSignals.length > 0 && (
        <aside className="vnext-chat-context-rail" aria-label={tr.t('chatHome.context.desktop')}>
          <p className="vnext-kicker">{tr.t('chatHome.context.contextKicker')}</p>
          <h2>{tr.t('chatHome.context.nearbyWork')}</h2>
          {renderContextSignals()}
        </aside>
      )}

      {/* One dismissible needs-you banner pinned above the composer (never in the stream). */}
      {renderNeedsYouBanner()}

      {contextPickerOpen && (
        <section className="vnext-chat-context-picker" aria-label={tr.t('chatHome.context.attachToChat')}>
          <div>
            <p className="vnext-kicker">{tr.t('chatHome.context.attachKicker')}</p>
            <h2>{tr.t('chatHome.context.chooseConsider')}</h2>
          </div>
          {visibleContextSignals.length > 0 ? renderContextSignals('attach', true) : (
            <p className="vnext-chat-context-empty">{tr.t('chatHome.context.noneWaiting')}</p>
          )}
          <div className="vnext-action-row">
            <VNextButton variant="secondary" onClick={() => setContextPickerOpen(false)}>
              {tr.t('chatHome.context.close')}
            </VNextButton>
          </div>
        </section>
      )}

      <div className="vnext-composer-stack">
      {voiceApproval ? (
        <section
          className="vnext-voice-approval-card"
          data-testid="voice-approval-card"
          data-approval-id={voiceApproval.approvalId}
          role="region"
          aria-label={tr.t('voice.panel.needsConfirm')}
        >
          <p className="vnext-kicker">{tr.t('voice.panel.needsConfirm')}</p>
          <p className="vnext-voice-approval-query" data-testid="voice-approval-query">{voiceApproval.query}</p>
          <div className="vnext-voice-approval-actions">
            <button
              type="button"
              className="vnext-btn vnext-btn-primary"
              data-testid="voice-approval-approve"
              onClick={() => onVoiceApproval?.('approve')}
            >
              {tr.t('voice.panel.approve')}
            </button>
            <button
              type="button"
              className="vnext-btn vnext-btn-secondary"
              data-testid="voice-approval-reject"
              onClick={() => onVoiceApproval?.('reject')}
            >
              {tr.t('voice.panel.reject')}
            </button>
          </div>
        </section>
      ) : null}
      {voiceLiveLine ? (
        <p className="vnext-voice-live-line" data-testid="ceo-voice-line" role="status">
          <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-chat-avatar" />
          <span>{voiceLiveLine}</span>
        </p>
      ) : null}
      <form
        className={[
          'vnext-chat-composer',
          `mode-${composerMode}`,
          composerFocused || value.trim() ? 'is-composing' : '',
          attachedContext ? 'has-attached-context' : '',
          hasPendingAttachmentRow ? 'has-pending-attachment' : '',
          quotedMessage ? 'has-reply-context' : '',
          voiceRecording ? 'is-recording' : '',
          busy ? 'is-busy' : '',
        ].filter(Boolean).join(' ')}
        aria-busy={busy}
        data-mode={composerMode}
        onSubmit={(event) => {
          event.preventDefault();
          submitComposer();
        }}
      >
        {showComposerMeta && (
        <div className="vnext-chat-composer-meta">
        {shorthandIntent && (
          <p className="vnext-chat-intent-cue" role="status" aria-label={tr.t('chatHome.intent.cueAria')}>
            <span>{tr.f('chatHome.intent.understood', { label: shorthandIntent.label })}</span>
            {shorthandIntent.needsReviewHint && (
              <em>{tr.t('chatHome.intent.reviewHint')}</em>
            )}
          </p>
        )}
        {attachedContext && (
          <span className="vnext-chat-attached-context">
            <VNextIcon name="work" />
            <span>{tr.f('chatHome.message.using', { label: attachedContext.label })}</span>
            <button type="button" aria-label={tr.t('chatHome.composer.removeAttached')} onClick={onClearAttachedContext}>
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        {/* Slice 5: pending file-attachment pill — mirrors the attached-context pill.
            Shows uploading / error / attached states; the attached state offers a
            remove button. Voice attachments reuse the existing voice path. */}
        {attachmentUploading && (
          <span className="vnext-chat-attached-file is-uploading" role="status">
            <VNextIcon name="paperclip" />
            <span>{tr.t('chatHome.composer.uploading')}</span>
          </span>
        )}
        {!attachmentUploading && attachmentError && (
          <span className="vnext-chat-attached-file is-error" role="alert">
            <VNextIcon name="paperclip" />
            <span>{attachmentError}</span>
            <button type="button" aria-label={tr.t('chatHome.composer.removeFile')} onClick={() => {
              onClearAttachment?.();
              inputRef.current?.focus({ preventScroll: true });
            }}>
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        {!attachmentUploading && !attachmentError && pendingAttachment && (
          <span className="vnext-chat-attached-file">
            <VNextIcon name="paperclip" />
            <span>{tr.f('chatHome.composer.fileAttached', { filename: pendingAttachment.filename })}</span>
            <button type="button" aria-label={tr.t('chatHome.composer.removeFile')} onClick={() => {
              onClearAttachment?.();
              inputRef.current?.focus({ preventScroll: true });
            }}>
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        {/* Held voice note: the SERVER's transcript, so the owner can see what
            Aegis actually heard before he sends it. */}
        {voiceNote?.status === 'attached' && (
          <span className="vnext-chat-attached-file is-voice">
            <VNextIcon name="voice" />
            <span>{voiceNote.transcript}</span>
            {voiceNote.durationLabel && <em>{voiceNote.durationLabel}</em>}
            <button
              type="button"
              aria-label={tr.t('chatHome.voiceNote.remove')}
              onClick={() => onCancelVoiceNote?.()}
            >
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        {voiceNote?.status === 'error' && (
          <span className="vnext-chat-attached-file is-error" role="alert">
            <VNextIcon name="voice" />
            <span>{voiceNote.message}</span>
            <button
              type="button"
              aria-label={tr.t('chatHome.voiceNote.remove')}
              onClick={() => onCancelVoiceNote?.()}
            >
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        {quotedMessage && (
          <span className="vnext-chat-reply-context">
            <VNextIcon name="reply" />
            <span>
              <em>{tr.f('chatHome.quote.replyingTo', { role: quoteRoleLabel(quotedMessage.role, tr) })}</em>
              <strong>{quotePreviewText(quotedMessage.text)}</strong>
            </span>
            <button type="button" aria-label={tr.t('chatHome.composer.removeQuoted')} onClick={onClearQuote}>
              <VNextIcon name="close" />
            </button>
          </span>
        )}
        </div>
        )}
        {/* Hidden file input (image/PDF). Mounted unconditionally so BOTH the
            visible paperclip and the "more" menu's Attach item can trigger it.
            iOS only honours `.click()` inside the tap handler itself, so both
            call it synchronously — never after an await. */}
        {onAttachFile && (
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*,application/pdf"
            className="vnext-chat-file-input"
            style={{ display: 'none' }}
            onChange={(event) => {
              const file = event.currentTarget.files?.[0];
              // Reset the input so re-selecting the same file fires onChange again.
              event.currentTarget.value = '';
              if (file) onAttachFile(file);
            }}
          />
        )}
        {voiceRecording ? (
          /* Recording replaces the text row: nothing else in the composer is
             actionable mid-recording, and the owner needs the elapsed time and
             an unmistakable way out. */
          <div className="vnext-chat-recording" role="status" aria-live="polite">
            <span className="vnext-chat-recording-dot" aria-hidden="true" />
            <span className="vnext-chat-recording-label">
              {voiceNote?.status === 'uploading'
                ? tr.t('chatHome.voiceNote.transcribing')
                : tr.t('chatHome.voiceNote.recording')}
            </span>
            {voiceNote?.status === 'recording' && (
              <em className="vnext-chat-recording-time">{voiceNote.elapsedLabel}</em>
            )}
            <button
              type="button"
              className="vnext-chat-recording-cancel"
              onClick={() => onCancelVoiceNote?.()}
            >
              {tr.t('chatHome.voiceNote.cancel')}
            </button>
            {voiceNote?.status === 'recording' && (
              <button
                type="button"
                className="vnext-chat-recording-stop"
                onClick={() => onStopVoiceNote?.()}
              >
                <VNextIcon name="send" />
                <span>{tr.t('chatHome.voiceNote.stop')}</span>
              </button>
            )}
          </div>
        ) : (
          <>
            <textarea
              ref={inputRef}
              value={value}
              rows={1}
              onChange={(event) => {
                resizeComposerInput(event.currentTarget, composerMode);
                onChange(event.currentTarget.value);
              }}
              onKeyDown={(event) => {
                if (event.key !== 'Enter' || event.shiftKey || isImeEnter(event.nativeEvent)) return;
                event.preventDefault();
                submitComposer();
              }}
              placeholder={busy ? tr.t('chatHome.composer.busyPlaceholder') : tr.t('chatHome.composer.placeholder')}
              aria-label={tr.t('chatHome.composer.tellAria')}
              onFocus={() => { composerBlurPending.current = false; setComposerFocused(true); }}
              onBlur={() => {
                if (composerPointerActive.current) composerBlurPending.current = true;
                else setComposerFocused(false);
              }}
            />
            {/* Line mode flattens this row into the composer grid (display:contents).
                Half mode keeps it as the toolbar under the field, so attach is
                never stranded above the textarea. */}
            <div className="vnext-chat-composer-tools">
            {/* Visible paperclip: the owner could not find Attach when it lived
                only inside the "more" menu (it stays there too). */}
            {onAttachFile && (
              <button
                type="button"
                className="vnext-chat-attach-button"
                aria-label={tr.t('chatHome.composer.attachFile')}
                title={tr.t('chatHome.composer.attachFile')}
                disabled={attachmentUploading}
                onClick={() => fileInputRef.current?.click()}
              >
                <VNextIcon name="paperclip" />
              </button>
            )}
            <button
              type="button"
              className="vnext-chat-context-button"
              aria-label={tr.t('chatHome.composer.attachContext')}
              title={tr.t('chatHome.composer.attachContext')}
              aria-expanded={contextPickerOpen}
              onClick={() => setContextPickerOpen((open) => !open)}
            >
              <VNextIcon name="link" />
            </button>
            {/* Composer size toggle stays visible while typing. Hiding it with
                the other phone controls trapped the owner in half mode. */}
            <button
              type="button"
              className="vnext-chat-size-button"
              aria-label={composerSizeActionLabel(composerMode, locale)}
              title={composerSizeActionLabel(composerMode, locale)}
              aria-pressed={composerMode !== 'line'}
              onClick={() => setComposerMode((mode) => nextComposerMode(mode))}
            >
              <VNextIcon name={composerMode === 'line' ? 'expand' : 'collapse'} />
            </button>
            {/* Mic = VOICE NOTE (record → upload → server transcribes), not
                dictation. The duplex summon in the top bar is separate. */}
            {onStartVoiceNote && (
              <button
                type="button"
                className="vnext-chat-voice"
                aria-label={tr.t('chatHome.voiceNote.record')}
                title={tr.t('chatHome.voiceNote.record')}
                onClick={() => onStartVoiceNote()}
              >
                <VNextIcon name="voice" />
              </button>
            )}
            {showAdjacentSendReason && (
              <span id={sendReasonId} className="vnext-chat-composer-send-reason">
                {sendDisabledReason}
              </span>
            )}
            <button
              type="submit"
              disabled={sendDisabled}
              aria-describedby={showAdjacentSendReason ? sendReasonId : undefined}
              aria-label={busy ? tr.t('chatHome.composer.queueForAegis') : tr.t('chatHome.composer.sendToAegis')}
              title={showAdjacentSendReason ? sendDisabledReason : (busy ? tr.t('chatHome.composer.queueForAegis') : tr.t('chatHome.composer.sendToAegis'))}
            >
              <VNextIcon name="send" />
              <span>{busy ? tr.t('chatHome.composer.queue') : tr.t('chatHome.composer.send')}</span>
            </button>
            </div>
          </>
        )}
      </form>
      </div>
    </section>
  );
}

function compactHomeBrief(value: string | null | undefined, tr: Translator): HomeBriefCard | null {
  const text = value?.trim() ?? '';
  if (!text) return null;
  const withoutPeriod = (input: string) => input.replace(/\.$/, '').trim();
  const decision = text.match(/^Needs your decision:\s*(.+)$/i);
  if (decision) return { label: tr.t('chatHome.brief.decisionWaiting'), text: withoutPeriod(decision[1] ?? '') };
  const attention = text.match(/^Needs attention:\s*(.+)$/i);
  if (attention) return { label: tr.t('chatHome.brief.startHere'), text: withoutPeriod(attention[1] ?? '') };
  const ready = text.match(/^Ready to review:\s*(.+)$/i);
  if (ready) return { label: tr.t('chatHome.brief.readyToReview'), text: withoutPeriod(ready[1] ?? '') };
  if (/^Nothing urgent is asking for you right now\.$/i.test(text)) {
    return { label: tr.t('chatHome.brief.quiet'), text: tr.t('chatHome.brief.quietRightNow') };
  }
  return { label: tr.t('chatHome.brief.firstLook'), text: withoutPeriod(text) };
}

function PlainNoticeSummaryCard({
  body,
  presentation,
  severity,
  tr,
}: {
  body: string;
  presentation: PlainNoticePresentation;
  severity?: NoticeSeverity | null;
  tr: Translator;
}) {
  const tone = severity ? ` tone-${severity}` : '';
  const severityLabel = plainNoticeSeverityLabel(severity, tr);
  return (
    <div className={`vnext-chat-long-notice-summary${tone}`}>
      <div className="vnext-chat-long-notice-head">
        <div>
          {severityLabel && <span className="vnext-chat-long-notice-severity">{severityLabel}</span>}
          <strong>{presentation.title}</strong>
        </div>
        <em>{presentation.detailHint}</em>
      </div>
      {presentation.previewLines.length > 0 && (
        <ul className="vnext-chat-long-notice-preview">
          {presentation.previewLines.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      )}
      {presentation.meta.length > 0 && (
        <div className="vnext-chat-long-notice-meta">
          {presentation.meta.map((item) => (
            <span key={item}>{item}</span>
          ))}
        </div>
      )}
      <details className="vnext-chat-long-notice vnext-disclosure">
        <summary className="vnext-disclosure-summary">
          <span className="vnext-disclosure-copy">
            <strong>{presentation.detailLabel}</strong>
            <em className="vnext-disclosure-hint">{presentation.detailHint}</em>
          </span>
        </summary>
        <RichText content={ownerBriefBody(body, tr.locale)} />
      </details>
    </div>
  );
}

function plainNoticePresentation(body: string, tr: Translator): PlainNoticePresentation | null {
  const text = body.trim();
  if (!text || !shouldCollapsePlainNotice(text)) return null;
  const brief = briefPlainNoticePresentation(text, tr);
  if (brief) return brief;
  const suggested = suggestedPlainNoticePresentation(text, tr);
  if (suggested) return suggested;
  return genericPlainNoticePresentation(text, tr);
}

function shouldCollapsePlainNotice(text: string): boolean {
  if (looksLikeBriefNotice(text)) return true;
  if (/(^|\n)\s*Suggested (task|reminder):/i.test(text) && text.length > 360) return true;
  const lineCount = text.replace(/\r\n?/g, '\n').split('\n').filter((line) => line.trim()).length;
  return text.length > LONG_PLAIN_NOTICE_CHAR_LIMIT || lineCount > LONG_PLAIN_NOTICE_LINE_LIMIT;
}

function briefPlainNoticePresentation(text: string, tr: Translator): PlainNoticePresentation | null {
  if (!looksLikeBriefNotice(text)) return null;
  const activeTasks = meaningfulSectionLines(noticeSectionLinesAny(text, ['Active Tasks', '进行中的待办']), { keepWarnings: false });
  const recentActions = meaningfulSectionLines(noticeSectionLinesAny(text, ['Recent Actions', '最近动作']), { keepWarnings: false });
  const warnings = meaningfulSectionLines(noticeSectionLines(text, 'Warnings'), { keepWarnings: true });
  const pendingApprovals = meaningfulSectionLines(noticeSectionLinesAny(text, ['Pending Approvals', '待你确认']), { keepWarnings: false });
  const rawTitle = noticeTitle(text);
  const title = compactNoticeLine(
    (tr.locale === 'zh' ? localizeBriefHeading(rawTitle) : rawTitle) || tr.t('chatHome.notice.aegisBrief'),
    120,
  );
  const previewLines = activeTasks.slice(0, 3).map((line) => compactNoticeLine(line, 120));
  if (previewLines.length === 0) {
    previewLines.push(...firstMeaningfulNoticeLines(text, 3).map((line) => compactNoticeLine(line, 120)));
  }
  const meta = [
    countLabel(activeTasks.length, 'chatHome.count.activeTaskOne', 'chatHome.count.activeTaskMany', tr),
    countLabel(recentActions.length, 'chatHome.count.recentActionOne', 'chatHome.count.recentActionMany', tr),
    countLabel(pendingApprovals.length, 'chatHome.count.pendingApprovalOne', 'chatHome.count.pendingApprovalMany', tr),
    countLabel(warnings.length, 'chatHome.count.warningOne', 'chatHome.count.warningMany', tr),
  ].filter(Boolean);
  return {
    title,
    previewLines,
    meta,
    detailLabel: looksLikeEveningReview(text) ? tr.t('chatHome.notice.fullReview') : tr.t('chatHome.notice.fullBrief'),
    detailHint: tr.f('chatHome.notice.lines', { count: String(nonEmptyNoticeLineCount(text)) }),
  };
}

function suggestedPlainNoticePresentation(text: string, tr: Translator): PlainNoticePresentation | null {
  const suggestedKinds = suggestedNoticeKinds(text, tr);
  if (suggestedKinds.length === 0) return null;
  const visibleLead = stripSuggestedNoticeBlocks(text);
  const title = compactNoticeLine(noticeTitle(visibleLead) || firstMeaningfulNoticeLines(visibleLead, 1)[0] || tr.t('chatHome.notice.aegisNotice'), 130);
  const previewLines = firstMeaningfulNoticeLines(visibleLead, 2)
    .filter((line) => line !== title)
    .map((line) => compactNoticeLine(line, 130));
  return {
    title,
    previewLines,
    meta: [tr.f('chatHome.notice.suggestedAttached', { kinds: suggestedKinds.join(tr.t('chatHome.notice.suggestedJoin')) })],
    detailLabel: tr.t('chatHome.notice.fullMessage'),
    detailHint: tr.f('chatHome.notice.lines', { count: String(nonEmptyNoticeLineCount(text)) }),
  };
}

function genericPlainNoticePresentation(text: string, tr: Translator): PlainNoticePresentation {
  const title = compactNoticeLine(noticeTitle(text) || firstMeaningfulNoticeLines(text, 1)[0] || tr.t('chatHome.notice.aegisNotice'), 130);
  const previewLines = firstMeaningfulNoticeLines(text, 4)
    .filter((line) => line !== title)
    .slice(0, 3)
    .map((line) => compactNoticeLine(line, 130));
  return {
    title,
    previewLines,
    meta: [],
    detailLabel: tr.t('chatHome.notice.fullMessage'),
    detailHint: tr.f('chatHome.notice.lines', { count: String(nonEmptyNoticeLineCount(text)) }),
  };
}

function looksLikeBriefNotice(text: string): boolean {
  const title = noticeTitle(text);
  return /^(?:Daily Brief|Evening Review)\b/i.test(title) || /的今日摘要|的晚间回顾/.test(title);
}

function looksLikeEveningReview(text: string): boolean {
  const title = noticeTitle(text);
  return /^Evening Review\b/i.test(title) || /的晚间回顾/.test(title);
}

function localizeBriefHeading(title: string): string {
  const daily = title.match(/^Daily Brief for\s+(\S+)(?:\s+\(([^)]+)\))?/i);
  if (daily) return datedBriefTitle(daily[1], daily[2], t('chatHome.notice.dailyBrief', 'zh'));
  const evening = title.match(/^Evening Review for\s+(\S+)(?:\s+\(([^)]+)\))?/i);
  if (evening) return datedBriefTitle(evening[1], evening[2], t('chatHome.notice.eveningReview', 'zh'));
  return BRIEF_HEADING_ZH[title.toLowerCase()] ?? title;
}

function datedBriefTitle(date: string, zone: string | undefined, label: string): string {
  return zone ? `${date} 的${label}（${zone}）` : `${date} 的${label}`;
}

function ownerBriefBody(text: string, locale: Locale): string {
  if (locale !== 'zh' || !looksLikeBriefNotice(text)) return text;
  return text.split('\n').map((line) => {
    const heading = line.match(/^(#{1,3}\s+)(.*)$/);
    if (heading) return `${heading[1]}${localizeBriefHeading(heading[2].trim())}`;
    const trimmed = line.trim().replace(/^_|_$/g, '');
    const mapped = BRIEF_LINE_ZH[trimmed];
    if (mapped) return line.replace(trimmed, mapped);
    if (trimmed.startsWith('Generated at:')) return line.replace('Generated at:', '生成于：');
    return line;
  }).join('\n');
}

function noticeSectionLinesAny(text: string, headings: string[]): string[] {
  for (const heading of headings) {
    if (text.replace(/\r\n?/g, '\n').split('\n').some((line) => sectionHeadingMatches(line, heading))) {
      return noticeSectionLines(text, heading);
    }
  }
  return [];
}

function noticeTitle(text: string): string {
  const line = text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((item) => stripMarkdownPrefix(item))
    .find((item) => item.trim());
  return line?.trim() ?? '';
}

function noticeSectionLines(text: string, heading: string): string[] {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const headingIndex = lines.findIndex((line) => sectionHeadingMatches(line, heading));
  if (headingIndex < 0) return [];
  const sectionLines: string[] = [];
  for (let index = headingIndex + 1; index < lines.length; index += 1) {
    const line = lines[index] ?? '';
    if (isKnownBriefSectionHeading(line)) break;
    if (line.trim()) sectionLines.push(stripMarkdownPrefix(line));
  }
  return sectionLines;
}

function sectionHeadingMatches(line: string, heading: string): boolean {
  const normalized = stripMarkdownPrefix(line).trim().replace(/:$/, '');
  return normalized.toLowerCase() === heading.toLowerCase();
}

function isKnownBriefSectionHeading(line: string): boolean {
  return BRIEF_SECTION_HEADINGS.some((heading) => sectionHeadingMatches(line, heading));
}

function meaningfulSectionLines(lines: string[], options: { keepWarnings: boolean }): string[] {
  return lines
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((line) => {
      if (/^no\b/i.test(line)) return false;
      if (!options.keepWarnings && /not queried/i.test(line)) return false;
      return true;
    });
}

function stripMarkdownPrefix(line: string): string {
  return line
    .trim()
    .replace(/^#{1,4}\s+/, '')
    .replace(/^[-*]\s+/, '')
    .replace(/^•\s*/, '')
    .replace(/^_([^_]+)_$/, '$1')
    .trim();
}

function firstMeaningfulNoticeLines(text: string, limit: number): string[] {
  return text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map(stripMarkdownPrefix)
    .filter((line) => line && !/^Generated at:/i.test(line))
    .slice(0, limit);
}

function compactNoticeLine(value: string, limit: number): string {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (normalized.length <= limit) return normalized;
  return `${normalized.slice(0, Math.max(0, limit - 3)).trim()}...`;
}

function countLabel(count: number, singularKey: string, pluralKey: string, tr: Translator): string {
  if (count <= 0) return '';
  return tr.f(count === 1 ? singularKey : pluralKey, { count: String(count) });
}

function nonEmptyNoticeLineCount(text: string): number {
  return text.replace(/\r\n?/g, '\n').split('\n').filter((line) => line.trim()).length;
}

function suggestedNoticeKinds(text: string, tr: Translator): string[] {
  const kinds: string[] = [];
  if (/(^|\n)\s*Suggested task:/i.test(text)) kinds.push(tr.t('chatHome.notice.suggestedTask'));
  if (/(^|\n)\s*Suggested reminder:/i.test(text)) kinds.push(tr.t('chatHome.notice.suggestedReminder'));
  return kinds;
}

function stripSuggestedNoticeBlocks(text: string): string {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const visible: string[] = [];
  for (const line of lines) {
    if (/^\s*Suggested (task|reminder):/i.test(line)) break;
    visible.push(line);
  }
  return visible.join('\n').trim();
}

function plainNoticeSeverityLabel(severity: NoticeSeverity | null | undefined, tr: Translator): string {
  if (severity === 'critical') return tr.t('chatHome.severity.critical');
  if (severity === 'warning') return tr.t('chatHome.severity.warning');
  if (severity === 'important') return tr.t('chatHome.severity.important');
  return '';
}

function precedingOwnerQuestion(messages: Array<{ role: string; text: string }>, index: number): string {
  for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
    const row = messages[cursor];
    if (!row) continue;
    if (row.role === 'owner' && row.text.trim()) return row.text.trim();
    if (row.role === 'aegis') break;
  }
  return '';
}

function AegisPlainMessage({
  text,
  severity,
  importanceDismissKey,
  inlineWorkAvailable = false,
  ownerQuestion = '',
  onAction,
}: {
  text: string;
  severity?: NoticeSeverity | null;
  importanceDismissKey?: string;
  inlineWorkAvailable?: boolean;
  ownerQuestion?: string;
  onAction: (action: DeliveryNoticeAction) => Promise<ChatHomeNoticeActionResult>;
}) {
  const trFn = useT();
  const { locale } = useLocale();
  const tr = makeTranslator(trFn, locale);
  const [confirmingCommand, setConfirmingCommand] = useState('');
  const [executingCommand, setExecutingCommand] = useState('');
  const [actionMessage, setActionMessage] = useState('');
  const [actionMessageKind, setActionMessageKind] = useState<'receipt' | 'error'>('receipt');
  const displayText = webFirstPlainNoticeText(text, inlineWorkAvailable, tr);
  const receiptSplit = splitSearchReceipt(displayText);
  const plainSources = splitPlainMessageSources(receiptSplit.body, locale);
  const notice = parseDeliveryNotice(plainSources.body, locale);
  const bodyText = notice.body || plainSources.body || displayText;
  const presentation = plainNoticePresentation(bodyText, tr);
  const hasActions = notice.actions.length > 0;
  return (
    <div className="vnext-chat-plain-answer">
      {presentation ? (
        <PlainNoticeSummaryCard
          body={bodyText}
          presentation={presentation}
          severity={severity}
          tr={tr}
        />
      ) : (
        <RichText
          content={bodyText}
          importanceSeverity={severity}
          importanceDismissKey={importanceDismissKey}
        />
      )}
      {receiptSplit.receipt && <SearchReceiptCard receipt={receiptSplit.receipt} ownerQuestion={ownerQuestion} tr={tr} />}
      {plainSources.sourceContent && (
        <details className="vnext-answer-card is-collapsible tone-evidence vnext-disclosure">
          <summary className="vnext-disclosure-summary">
            <span>{tr.t('chatHome.plain.sources')}</span>
          </summary>
          <SourceEvidenceContent content={plainSources.sourceContent} locale={locale} />
        </details>
      )}
      {hasActions && (
        <details open className="vnext-chat-action-disclosure vnext-chat-notice-action-card vnext-disclosure" aria-label={tr.t('chatHome.plain.aegisActions')}>
          <summary className="vnext-disclosure-summary">
            <span>
              <VNextIcon name="spark" />
              {tr.t('chatHome.plain.actions')}
            </span>
            <em>{notice.actions.length}</em>
          </summary>
          <div className="vnext-chat-notice-actions">
            {notice.actions.map((action) => {
              const confirming = confirmingCommand === action.command;
              return (
                <button
                  key={action.command}
                  type="button"
                  className={`tone-${action.tone}${confirming ? ' is-confirming' : ''}`}
                  title={action.detail}
                  disabled={executingCommand !== ''}
                  onClick={() => {
                    if (action.requiresConfirmation && !confirming) {
                      setConfirmingCommand(action.command);
                      setActionMessage('');
                      return;
                    }
                    setConfirmingCommand('');
                    setExecutingCommand(action.command);
                    setActionMessage('');
                    void onAction(action)
                      .then((result) => {
                        setActionMessageKind(result.ok ? 'receipt' : 'error');
                        setActionMessage(result.message);
                      })
                      .catch((error) => {
                        setActionMessageKind('error');
                        setActionMessage(error instanceof Error ? error.message : tr.t('chatHome.work.actionFailed'));
                      })
                      .finally(() => setExecutingCommand(''));
                  }}
                >
                  <VNextIcon name={deliveryActionIcon(action)} />
                  <span>
                    <strong>{executingCommand === action.command ? tr.t('chatHome.work.working') : confirming ? tr.t('chatHome.work.confirm') : action.label}</strong>
                    <em>{action.detail}</em>
                  </span>
                </button>
              );
            })}
          </div>
          {actionMessage && (
            <p className={actionMessageKind === 'error' ? 'vnext-chat-work-action-note error' : 'vnext-chat-work-action-note'}>
              {actionMessage}
            </p>
          )}
        </details>
      )}
    </div>
  );
}

function splitPlainMessageSources(text: string, locale: Locale): { body: string; sourceContent: string } {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const heading = lines[index]?.trim() ?? '';
    if (!/^(sources|source|来源|证据)\s*[:：]?$/i.test(heading)) continue;
    const sourceLines = lines.slice(index + 1)
      .map((line) => line.trim())
      .filter(Boolean);
    if (sourceLines.length === 0 || sourceLines.length > 8) continue;
    if (!sourceLines.every(looksLikePlainSourceLabel)) continue;
    const sourceContent = sourceLines.join('\n');
    if (sourceEvidenceItems(sourceContent, locale).length === 0) continue;
    return {
      body: lines.slice(0, index).join('\n').trim(),
      sourceContent,
    };
  }
  const inlineSources: string[] = [];
  const bodyLines = lines.filter((line) => {
    const match = line.trim().match(/^(?:[-*]\s*)?(?:sources?|来源|证据)\s*[:：]\s*(.+)$/i);
    if (!match) return true;
    const sourceLabel = inlinePlainSourceLabel(match[1] ?? '');
    if (!sourceLabel || !looksLikePlainSourceLabel(sourceLabel)) return true;
    if (sourceEvidenceItems(sourceLabel, locale).length === 0) return true;
    inlineSources.push(sourceLabel);
    return false;
  });
  if (inlineSources.length > 0 && inlineSources.length <= 8) {
    return {
      body: bodyLines.join('\n').trim(),
      sourceContent: inlineSources.join('\n'),
    };
  }
  return { body: text, sourceContent: '' };
}

function webFirstPlainNoticeText(text: string, inlineWorkAvailable: boolean, tr: Translator): string {
  const normalized = text.replace(/\s+/g, ' ').trim();
  const decisionEn = t('chatHome.notice.needsApprovalDecision', 'en');
  const decisionZh = t('chatHome.notice.needsApprovalDecision', 'zh');
  const localized = normalized === decisionEn || normalized === decisionZh
    ? tr.t('chatHome.notice.needsApprovalDecision')
    : text;
  if (!inlineWorkAvailable) return localized;
  return localized.replace(
    /Sensitive item needs review:\s*Open the Web Console and complete step-up MFA to view details\./i,
    tr.t('chatHome.plain.sensitiveReview'),
  );
}

function looksLikePlainSourceLabel(line: string): boolean {
  const label = line.replace(/^\s*[-*]\s*/, '').trim();
  if (!label) return false;
  if (/^https?:\/\//i.test(label)) return label.length <= 2048;
  if (label.length > 80) return false;
  if (/[。！？.!?]$/.test(label)) return false;
  if (/[，,；;]/.test(label)) return false;
  return true;
}

function inlinePlainSourceLabel(value: string): string {
  const trimmed = value.trim();
  const url = trimmed.match(/https?:\/\/\S+/i)?.[0]?.replace(/[)\].,;，。；]+$/, '');
  if (url) return url;
  return trimmed;
}

function deliveryActionIcon(action: DeliveryNoticeAction) {
  if (action.tone === 'destructive') return 'close';
  if (action.label === 'Later') return 'clock';
  if (action.label === 'Retry' || action.label === 'Run') return 'spark';
  return 'check';
}

function ChatWorkCard({
  item,
  unavailable = false,
  busy,
  thread,
  onOpenDetail,
  onAsk,
  onAction,
}: {
  item: OwnerWorkItem;
  unavailable?: boolean;
  busy: boolean;
  thread: ChatHomeWorkThreadEntry[];
  onOpenDetail: (item: OwnerWorkItem) => Promise<ChatHomeWorkDetailResult>;
  onAsk: (text: string) => void;
  onAction: (action: VNextItemAction) => Promise<ChatHomeWorkActionResult>;
}) {
  const userTimezone = useUserTimezone();
  const trFn = useT();
  const { locale } = useLocale();
  const tr = makeTranslator(trFn, locale);
  const [askOpen, setAskOpen] = useState(false);
  const [askText, setAskText] = useState('');
  const [armedAction, setArmedAction] = useState<VNextItemActionId | null>(null);
  const [executingAction, setExecutingAction] = useState<VNextItemActionId | null>(null);
  const [actionMessage, setActionMessage] = useState('');
  const [actionMessageKind, setActionMessageKind] = useState<'hint' | 'receipt' | 'error'>('hint');
  const [optimisticStatus, setOptimisticStatus] = useState<string | null>(null);
  const [localUnavailable, setLocalUnavailable] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [detailBusy, setDetailBusy] = useState(false);
  const [sourceDetail, setSourceDetail] = useState<SourceDetailView | null>(null);
  const [detailMessage, setDetailMessage] = useState('');
  const [detailMessageKind, setDetailMessageKind] = useState<'hint' | 'error'>('hint');
  const askInputRef = useRef<HTMLTextAreaElement | null>(null);
  const detailRequestRef = useRef(0);
  const trimmedAsk = askText.trim();
  const cardUnavailable = unavailable || localUnavailable;
  const visibleItem = optimisticStatus ? { ...item, status: optimisticStatus } : item;
  const visibleThread = thread.filter((entry) => !entry.itemStatus || entry.itemStatus === visibleItem.status);
  const reasonCue = ownerVisibleReasonCue(visibleItem, locale);
  const nextStep = cardUnavailable ? tr.t('chatHome.work.refreshedHidden') : workCardNextStep(visibleItem, tr);
  const proofText = cardUnavailable ? tr.t('chatHome.work.noNewAction') : workCardProofText(visibleItem, tr);
  const primaryActionLabel = workCardPrimaryActionLabel(visibleItem, tr);
  const detailActionLabel = detailBusy ? tr.t('chatHome.work.opening') : detailOpen ? tr.t('chatHome.work.hideDetails') : primaryActionLabel;
  const inlineActions = cardUnavailable ? [] : buildSupportedItemActions(visibleItem, locale);

  function toggleDetail() {
    if (detailBusy) return;
    if (detailOpen) {
      setDetailOpen(false);
      return;
    }
    if (sourceDetail) {
      setDetailOpen(true);
      return;
    }
    const requestId = detailRequestRef.current + 1;
    detailRequestRef.current = requestId;
    setDetailBusy(true);
    setDetailMessage('');
    void onOpenDetail(visibleItem)
      .then((result) => {
        if (detailRequestRef.current !== requestId) return;
        if (result.ok) {
          setSourceDetail(result.detail);
          setDetailOpen(true);
          setDetailMessageKind('hint');
          setDetailMessage(result.message ?? '');
          return;
        }
        setDetailMessageKind('error');
        setDetailMessage(result.message);
      })
      .catch((error) => {
        if (detailRequestRef.current !== requestId) return;
        setDetailMessageKind('error');
        setDetailMessage(error instanceof Error ? error.message : tr.t('chatHome.work.couldNotOpenDetail'));
      })
      .finally(() => {
        if (detailRequestRef.current === requestId) setDetailBusy(false);
      });
  }

  function submitAsk() {
    if (!trimmedAsk) return;
    onAsk(trimmedAsk);
    setAskText('');
    setAskOpen(false);
  }

  useEffect(() => {
    if (!askOpen) return;
    askInputRef.current?.focus();
    resizeComposerInput(askInputRef.current);
  }, [askOpen]);

  useEffect(() => {
    resizeComposerInput(askInputRef.current);
  }, [askText]);

  useEffect(() => {
    setLocalUnavailable(false);
    setArmedAction(null);
    setExecutingAction(null);
    setActionMessage('');
    setActionMessageKind('hint');
    setOptimisticStatus(null);
    setDetailOpen(false);
    setDetailBusy(false);
    setSourceDetail(null);
    setDetailMessage('');
    setDetailMessageKind('hint');
    detailRequestRef.current += 1;
  }, [item.id, item.status, unavailable]);

  return (
    <section className="vnext-chat-work-card" aria-label={tr.t('chatHome.work.cardLabel')}>
      <div className="vnext-chat-work-card-top">
        <span>{itemNeedsDecision(visibleItem) ? tr.t('chatHome.work.decision') : tr.t('chatHome.work.work')}</span>
        <em>{itemMeta(visibleItem, locale)}</em>
      </div>
      <h3>{ownerVisibleWorkTitle(visibleItem, tr.t('chatHome.work.title'), 120, locale)}</h3>
      <p>{ownerVisibleWorkCue(visibleItem, 160, userTimezone, locale)}</p>
      {reasonCue && (
        <p className="vnext-chat-work-reason">
          <VNextIcon name="spark" />
          <span>{reasonCue}</span>
        </p>
      )}
      <div className="vnext-chat-work-facts" aria-label={tr.t('chatHome.work.factsLabel')}>
        <span>
          <em>{tr.t('chatHome.work.next')}</em>
          <strong>{nextStep}</strong>
        </span>
        {proofText && (
          <span>
            <em>{tr.t('chatHome.work.receipt')}</em>
            <strong>{proofText}</strong>
          </span>
        )}
      </div>
      <div className="vnext-action-row">
        {!cardUnavailable && (
          <VNextButton variant="secondary" aria-expanded={detailOpen} disabled={detailBusy} leadingIcon={<VNextIcon name={detailOpen ? 'close' : 'work'} />} onClick={toggleDetail}>
            {detailActionLabel}
          </VNextButton>
        )}
        <VNextButton variant="secondary" leadingIcon={<VNextIcon name="spark" />} onClick={() => setAskOpen((open) => !open)}>
          {tr.t('chatHome.work.askAegis')}
        </VNextButton>
      </div>
      {inlineActions.length > 0 && (
        <details className="vnext-chat-action-disclosure vnext-chat-work-action-disclosure vnext-disclosure" aria-label={tr.t('chatHome.work.actionsLabel')}>
          <summary className="vnext-disclosure-summary">
            <span>
              <VNextIcon name="check" />
              {tr.t('chatHome.work.actions')}
            </span>
            <em>{inlineActions.length}</em>
          </summary>
          <div className="vnext-chat-work-quick-actions" aria-label={tr.t('chatHome.work.quickActionsLabel')}>
            {inlineActions.map((action) => (
              <VNextButton
                key={action.id}
                variant={action.tone === 'destructive' ? 'destructive' : 'secondary'}
                size="sm"
                leadingIcon={<VNextIcon name={itemActionIconName(action.id, action.tone)} />}
                disabled={executingAction !== null}
                onClick={() => {
                  if (armedAction !== action.id) {
                    setArmedAction(action.id);
                    setActionMessageKind('hint');
                    setActionMessage(itemActionConfirmationHint(action, locale));
                    return;
                  }
                  setExecutingAction(action.id);
                  setActionMessageKind('hint');
                  setActionMessage('');
                  void onAction(action).then((result) => {
                    setArmedAction(null);
                    if (result.ok) {
                      setOptimisticStatus(result.nextStatus);
                    } else if (isStaleWorkRefreshMessage(result.message) && result.itemUnavailable) {
                      setLocalUnavailable(true);
                      setOptimisticStatus(null);
                    }
                    setActionMessageKind(result.ok ? 'receipt' : isStaleWorkRefreshMessage(result.message) ? 'hint' : 'error');
                    setActionMessage(result.message);
                  }).finally(() => setExecutingAction(null));
                }}
              >
                {itemActionButtonLabel(action, armedAction, executingAction, locale)}
              </VNextButton>
            ))}
          </div>
        </details>
      )}
      {actionMessage && actionMessageKind === 'receipt' ? (
        <div className="vnext-chat-work-receipt" role="status" aria-label={tr.t('chatHome.work.receiptLabel')}>
          <VNextIcon name="receipt" />
          <span>
            <em>{tr.t('chatHome.work.receipt')}</em>
            <strong>{actionMessage}</strong>
          </span>
        </div>
      ) : actionMessage ? (
        <p className={actionMessageKind === 'error' ? 'vnext-chat-work-action-note error' : 'vnext-chat-work-action-note'}>
          {actionMessage}
        </p>
      ) : null}
      {detailMessage && (
        <p className={detailMessageKind === 'error' ? 'vnext-chat-work-action-note error' : 'vnext-chat-work-action-note'}>
          {detailMessage}
        </p>
      )}
      {detailOpen && sourceDetail && (
        <div className="vnext-chat-work-detail" aria-label={tr.t('chatHome.work.openedDetail')}>
          <SourceDetailPanel sourceDetail={sourceDetail} collapsed={sourceDetail.kind === 'task'} />
        </div>
      )}
      {askOpen && (
        <form
          className="vnext-chat-work-ask"
          onSubmit={(event) => {
            event.preventDefault();
            submitAsk();
          }}
        >
          <textarea
            ref={askInputRef}
            value={askText}
            rows={1}
            aria-label={tr.t('chatHome.work.askAboutItem')}
            placeholder={tr.t('chatHome.work.askPlaceholder')}
            onChange={(event) => {
              resizeComposerInput(event.currentTarget);
              setAskText(event.currentTarget.value);
            }}
            onKeyDown={(event) => {
              if (event.key !== 'Enter' || event.shiftKey || isImeEnter(event.nativeEvent)) return;
              event.preventDefault();
              submitAsk();
            }}
          />
          <button
            type="submit"
            disabled={busy && !trimmedAsk}
            aria-label={busy ? tr.t('chatHome.work.queueAboutItem') : tr.t('chatHome.work.askAboutItem')}
          >
            <VNextIcon name="send" />
            <span>{busy ? tr.t('chatHome.composer.queue') : tr.t('chatHome.composer.send')}</span>
          </button>
        </form>
      )}
      {visibleThread.length > 0 && (
        <div className="vnext-chat-work-thread" aria-label={tr.t('chatHome.work.threadLabel')}>
          {visibleThread.map((entry) => (
            <article key={entry.id} className="vnext-chat-work-thread-entry">
              <p className="vnext-chat-work-question">
                <span>{tr.t('chatHome.role.you')}</span>
                <MessageTime timestamp={entry.timestamp} />
                <MessageCite
                  role="owner"
                  timestamp={entry.timestamp}
                  text={entry.question}
                  fallbackId={entry.id}
                />
                {entry.question}
              </p>
              {entry.result ? (
                <>
                  <p className="vnext-chat-work-response-label">
                    <span>{tr.t('chatHome.role.aegis')}</span>
                    <MessageTime timestamp={entry.responseTimestamp} />
                    <MessageCite
                      role="aegis"
                      turnId={entry.result.turn_id}
                      timestamp={entry.responseTimestamp}
                      text={entry.result.visible_answer}
                      fallbackId={`${entry.id}:reply`}
                    />
                  </p>
                  <AssistantResponseView result={entry.result} ownerQuestion={entry.question} />
                  {entry.result.response_kind === 'error' && (
                    <div className="vnext-action-row">
                      <VNextButton variant="secondary" onClick={() => onAsk(entry.question)}>
                        {tr.t('chatHome.retry.tryAgain')}
                      </VNextButton>
                    </div>
                  )}
                </>
              ) : entry.state === 'queued' ? (
                <p className="vnext-chat-work-queued">
                  <span className="vnext-chat-working-dot" aria-hidden="true" />
                  {tr.t('chatHome.work.queuedNext')}
                </p>
              ) : (
                <p className="vnext-chat-work-working">
                  <span className="vnext-chat-working-dot" aria-hidden="true" />
                  <span className="vnext-chat-working-text">{entry.workingText || tr.t('chatHome.work.checkingItem')}</span>
                </p>
              )}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function isStaleWorkRefreshMessage(value: string): boolean {
  // App.tsx produces this message from the catalog (errorLocale), so match the
  // exact catalog value for every supported locale instead of an English-only
  // prefix regex — otherwise zh sessions would lose the 'hint' presentation.
  const trimmed = value.trim();
  return trimmed === t('app.action.itemChangedRefreshedQueue', 'en')
    || trimmed === t('app.action.itemChangedRefreshedQueue', 'zh');
}
