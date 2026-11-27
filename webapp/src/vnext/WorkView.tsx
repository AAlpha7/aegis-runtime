import { useEffect, useMemo, useRef, useState } from 'react';
import type { MutableRefObject, ReactNode } from 'react';
import type {
  ContextualAssistantResponse,
  OwnerWorkContextEnvelope,
  OwnerWorkItem,
  VNextDecisionChoice,
  VNextDecisionRequest,
  VNextConversationContextTurn,
  VNextItemProjection,
} from '../api';
import type { DetailAttachment, DetailField, SourceDetailView } from './detail';
import { collectXLinksFromAssistantResult } from './AssistantResponseView';
import { InlineAssistantPanel } from './InlineAssistantPanel';
import type { InlineAssistantTurn } from './InlineAssistantPanel';
import { itemActionButtonLabel } from './interactionState';
import { authorityBasisLabel, blockedAutomationWhatHappened, buildCommandSuggestions, buildSupportedItemActions, isClosedItem, isTaskLikeItem, itemIsBlockedByGate, itemNeedsDecision, itemNeedsPrivateTaskAccess, laneForItem, ownerVisibleItemTitle, ownerVisibleNextSafeActionText, ownerVisiblePrimaryActionLabel, ownerVisibleReasonCue, ownerVisibleWorkCue, ownerVisibleWorkTitle, presentOwnerWorkSurfaceActions, workDetailAskPlacement, workScanFiltersDefaultExpanded, workScanChromeDefaultExpanded, resolveWorkCodeText } from './model';
import type { OwnerWorkSurfaceControl, VNextItemAction, VNextItemActionId } from './model';
import { itemActionIconName, VNextIcon } from './icons';
import type { VNextIconName } from './icons';
import { VNextButton } from './VNextButton';
import { lineageChatCite } from './messageRef';
import type { LineageChatCite } from './messageRef';
import { VNextSegmented } from './VNextSegmented';
import { parseWireMomentMs } from './time';
import {
  classNames,
  compactOwnerList,
  isActionSuccessMessage,
  itemHasPriority,
  ownerFacingFieldLabel,
  ownerTimeLabel,
  titleCaseLabel,
  urgencyLevelLabel,
} from './presentation';
import { RichText } from './RichText';
import { useUserTimezone } from './UserTimezoneContext';
import { WorkItemRow } from './WorkItemRow';
import { useLocale } from '../i18n/LocaleContext';
import { catalogHas, t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import {
  followUpCloseAction,
  followUpQuickControls,
  isGenericFollowUpReason,
  isGenericFollowUpReceiptExpectation,
  shouldExecuteFollowUpCleanup,
  type FollowUpQuickControl,
} from './followUpCleanup';
import {
  buildXReplyComposeUrl,
  extractXUrlFromWorkItem,
  mergeWorkAndAnswerXHandoff,
  openXDeepLink,
} from './xDeepLink';

/**
 * Localized copy for one decide-card choice, resolved by stable server semantic
 * id (`choice.id`) plus finite risk/scope enums. Catalog keys:
 *   work.decide.choice.<id>.label / .consequence
 *   work.decide.risk.<level>
 *   work.decide.scope.<applies_to>
 * Unknown ids/enums fall back to the server strings verbatim (forward-compat).
 */
export function resolveDecisionChoiceCopy(
  choice: Pick<VNextDecisionChoice, 'id' | 'label' | 'consequence' | 'risk_level' | 'applies_to'>,
  locale: Locale,
): { label: string; consequence: string; riskLabel: string; scopeLabel: string } {
  const labelKey = `work.decide.choice.${choice.id}.label`;
  const consequenceKey = `work.decide.choice.${choice.id}.consequence`;
  const riskKey = `work.decide.risk.${choice.risk_level}`;
  const scopeKey = `work.decide.scope.${choice.applies_to}`;
  return {
    label: catalogHas(labelKey) ? t(labelKey, locale) : choice.label,
    consequence: catalogHas(consequenceKey) ? t(consequenceKey, locale) : choice.consequence,
    riskLabel: catalogHas(riskKey) ? t(riskKey, locale) : choice.risk_level,
    // Unknown scopes keep the pre-localization `_ → space` beautification.
    scopeLabel: catalogHas(scopeKey) ? t(scopeKey, locale) : choice.applies_to.replace(/_/g, ' '),
  };
}

export type LaneProjection = {
  id: string;
  title: string;
  subtitle: string;
  items: OwnerWorkItem[];
  collapsed: boolean;
};

export type WorkScanScope = 'all' | 'needs_me' | 'quiet';
export type WorkScanSort = 'lane' | 'priority' | 'updated';

type QueuedInlineAssistantPrompt = {
  id: string;
  itemId: string;
  text: string;
  kind: 'prompt' | 'retry';
};

export function filterWorkForScan(
  items: OwnerWorkItem[],
  options: { query?: string; scope?: WorkScanScope; sort?: WorkScanSort },
): OwnerWorkItem[] {
  const query = (options.query ?? '').trim().toLowerCase();
  const scope = options.scope ?? 'all';
  const sort = options.sort ?? 'lane';
  const tokens = query.split(/\s+/).filter(Boolean);
  const laneOrder = new Map(items.map((item, index) => [item.id, index] as const));
  const filtered = items.filter((item) => {
    const actionable = itemShouldAppearInNearbyWork(item);
    if (scope === 'needs_me' && !actionable) return false;
    if (scope === 'quiet' && actionable) return false;
    if (tokens.length === 0) return true;
    const haystack = workScanText(item);
    return tokens.every((token) => haystack.includes(token));
  });
  if (sort === 'priority') {
    return [...filtered].sort((a, b) => workScanPriorityScore(b) - workScanPriorityScore(a)
      || workScanDueScore(a.due_at) - workScanDueScore(b.due_at)
      || (laneOrder.get(a.id) ?? 0) - (laneOrder.get(b.id) ?? 0));
  }
  if (sort === 'updated') {
    return [...filtered].sort((a, b) => workScanUpdatedScore(b.updated_at) - workScanUpdatedScore(a.updated_at)
      || (laneOrder.get(a.id) ?? 0) - (laneOrder.get(b.id) ?? 0));
  }
  return filtered;
}


function assistantBriefLabel(item: OwnerWorkItem, locale: Locale): string {
  return item.kind === 'approval' ? t('work.action.explain', locale) : t('work.action.brief', locale);
}


function renderOwnerWorkSurfaceControl(
  control: OwnerWorkSurfaceControl,
  {
    item,
    locale,
    sourceDetailBusy,
    armedAction,
    armedActionItemId,
    executingAction,
    onOpenSource,
    onAction,
    onOpenAssistant,
    onBrief,
    onDraft,
    primary,
  }: {
    item: OwnerWorkItem;
    locale: Locale;
    sourceDetailBusy: boolean;
    armedAction: VNextItemActionId | null;
    armedActionItemId: string | null;
    executingAction: VNextItemActionId | null;
    onOpenSource: () => void;
    onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
    onOpenAssistant: (prefill?: string) => void;
    onBrief: () => void;
    onDraft: () => void;
    primary: boolean;
  },
) {
  if (control.kind === 'open_detail') {
    if (primary) {
      return (
        <button
          key={control.id}
          type="button"
          className="vnext-item-primary-step"
          aria-label={t('work.aria.openDetails', locale)}
          onClick={onOpenSource}
          disabled={sourceDetailBusy}
        >
          <VNextIcon name="work" />
          <strong>{control.label}</strong>
        </button>
      );
    }
    return (
      <VNextButton
        key={control.id}
        variant="secondary"
        size="sm"
        leadingIcon={<VNextIcon name="work" />}
        aria-label={t('work.aria.openDetails', locale)}
        onClick={onOpenSource}
        disabled={sourceDetailBusy}
      >
        {control.label}
      </VNextButton>
    );
  }
  if (control.kind === 'item_action') {
    const action = control.action;
    return (
      <VNextButton
        key={control.id}
        variant={primary ? 'primary' : action.tone === 'destructive' ? 'destructive' : 'secondary'}
        size={primary ? 'md' : 'sm'}
        className={classNames(
          armedActionItemId === item.id && executingAction === action.id && 'is-executing',
        )}
        leadingIcon={<VNextIcon name={itemActionIconName(action.id, action.tone)} />}
        disabled={executingAction !== null}
        aria-busy={armedActionItemId === item.id && executingAction === action.id}
        onClick={() => onAction(item, action)}
      >
        {itemActionButtonLabel(action, armedActionItemId === item.id ? armedAction : null, executingAction, locale)}
      </VNextButton>
    );
  }
  if (control.kind === 'open_x' || control.kind === 'draft_in_x') {
    return (
      <VNextButton
        key={control.id}
        variant={primary ? 'primary' : 'secondary'}
        size={primary ? 'md' : 'sm'}
        className={classNames(
          control.kind === 'open_x' ? 'vnext-work-open-in-x' : 'vnext-work-draft-in-x',
          primary && 'vnext-item-primary-step',
        )}
        leadingIcon={<VNextIcon name={control.kind === 'draft_in_x' ? 'instructions' : 'spark'} />}
        aria-label={control.label}
        title={control.label}
        onClick={() => openXDeepLink(control.href)}
      >
        {control.label}
      </VNextButton>
    );
  }
  // assistant brief/draft
  return (
    <VNextButton
      key={control.id}
      variant="secondary"
      size="sm"
      leadingIcon={<VNextIcon name={control.id === 'draft' ? 'instructions' : 'spark'} />}
      onClick={() => {
        onOpenAssistant();
        if (control.id === 'draft') onDraft();
        else onBrief();
      }}
    >
      {control.label}
    </VNextButton>
  );
}

/**
 * Owner rule (2026-09-16): tapping a row expands it DOWNWARD with the row's
 * top edge fixed. Never scroll to fit the expanded height. The row is put
 * back exactly where the owner tapped it (`tappedTop`) — the previously
 * expanded item above it may have collapsed in the same commit and pulled the
 * whole list up — unless that spot is under the sticky header, in which case
 * it sits just below the header. With no tap position (automatic selection,
 * keyboard) only the hidden-under-header case scrolls. The console scroller
 * has `overflow-anchor: none` (chrome.css) so the browser cannot re-anchor
 * on content below the row while the detail grows.
 */
export function keepRowTopInPlace(host: HTMLElement, tappedTop: number | null) {
  const scroller = host.closest<HTMLElement>('.vnext-shell');
  if (!scroller) return;
  const header = scroller.querySelector<HTMLElement>('.vnext-owner-shell-chrome');
  const headerBottom = header ? header.getBoundingClientRect().bottom : scroller.getBoundingClientRect().top;
  const minTop = headerBottom + 8;
  const rowTop = host.getBoundingClientRect().top;
  const desired = Math.max(tappedTop ?? rowTop, minTop);
  const delta = rowTop - desired;
  if (Math.abs(delta) < 1) return;
  scroller.scrollTop += delta;
}

const DESKTOP_LAYOUT_QUERY = '(min-width: 821px)';

/**
 * True on the two-pane (desktop) layout, false on phones and during static
 * render. The phone's inline lanes (row + detail grown in place) are not
 * mounted on desktop: the detail lives in the right pane, and a hidden copy
 * of it would be the first `.vnext-detail` in the document.
 */
function useDesktopLayout(): boolean {
  const [desktop, setDesktop] = useState(() => (
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(DESKTOP_LAYOUT_QUERY).matches
      : false
  ));
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return undefined;
    const query = window.matchMedia(DESKTOP_LAYOUT_QUERY);
    const update = () => setDesktop(query.matches);
    update();
    query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);
  return desktop;
}

function briefPrompt(locale: Locale): string {
  return t('work.assistant.prompt.brief', locale);
}

function draftPrompt(locale: Locale): string {
  return t('work.assistant.prompt.draft', locale);
}

function nextAssistantTurnId(seq: MutableRefObject<number>, prefix: string): string {
  seq.current += 1;
  return `${prefix}:${seq.current}`;
}

function trimAssistantTurns(turns: InlineAssistantTurn[]): InlineAssistantTurn[] {
  return turns.slice(-10);
}

function inlineAssistantConversationContext(turns: InlineAssistantTurn[]): VNextConversationContextTurn[] {
  return turns.flatMap((turn): VNextConversationContextTurn[] => {
    if (turn.role === 'owner' && turn.text) return [{ role: 'owner', text: turn.text }];
    if (turn.role === 'aegis' && turn.result?.visible_answer) {
      return [{ role: 'aegis', text: turn.result.visible_answer }];
    }
    return [];
  }).slice(-8);
}

function inlineAssistantConversationContextBeforeQueuedTurn(
  turns: InlineAssistantTurn[],
  queuedTurnId: string,
): VNextConversationContextTurn[] {
  return inlineAssistantConversationContext(turns.filter((turn) => turn.id !== queuedTurnId));
}

export function WorkView({
  lanes,
  expandedLaneIds,
  selected,
  requestedItemId = null,
  context,
  itemProjection,
  sourceDetail,
  sourceDetailBusy,
  assistantResult,
  assistantResultItemId,
  actionMessage,
  workNotice,
  actionsRetired = false,
  armedAction,
  armedActionItemId,
  executingAction,
  busy,
  hostStepUpBusy = false,
  onHostStepUp,
  onSelect,
  onToggleLane,
  onOpenSource,
  onAction,
  onExecuteAction,
  onBrief,
  onDraft,
  onAskText,
  onRetryAssistant,
  onRetryAssistantLabel,
  onOpenLineageMessage,
}: {
  lanes: LaneProjection[];
  expandedLaneIds: string[];
  selected: OwnerWorkItem | null;
  /** Item the owner asked for by id (deep link, back/forward, "open in 工作",
   *  voice). Scrolls into view like a tap; the App's automatic first-priority
   *  pick does not. */
  requestedItemId?: string | null;
  context: OwnerWorkContextEnvelope | null;
  itemProjection: VNextItemProjection | null;
  sourceDetail: SourceDetailView | null;
  sourceDetailBusy: boolean;
  assistantResult: ContextualAssistantResponse | null;
  assistantResultItemId: string | null;
  actionMessage: string;
  workNotice: string;
  actionsRetired: boolean;
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  busy: boolean;
  hostStepUpBusy?: boolean;
  onHostStepUp?: () => void;
  onSelect: (id: string | null) => void;
  onToggleLane: (laneId: string) => void;
  onOpenSource: () => void;
  onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
  /** One-shot execute (no arm) for clear owner cleanup commands in the item assistant. */
  onExecuteAction?: (item: OwnerWorkItem, action: VNextItemAction) => void;
  onBrief: () => void;
  onDraft: () => void;
  onAskText: (item: OwnerWorkItem, text: string, conversationContext?: VNextConversationContextTurn[]) => void;
  onRetryAssistant?: () => void;
  onRetryAssistantLabel?: string;
  /** One tap from a 「对话消息」 hop to that chat cite. */
  onOpenLineageMessage?: (cite: LineageChatCite) => void;
}) {
  const { locale } = useLocale();
  const [assistantComposerItemId, setAssistantComposerItemId] = useState<string | null>(null);
  const [assistantTextByItemId, setAssistantTextByItemId] = useState<Record<string, string>>({});
  const [assistantTurnsByItemId, setAssistantTurnsByItemId] = useState<Record<string, InlineAssistantTurn[]>>({});
  const [queuedAssistantPrompts, setQueuedAssistantPrompts] = useState<QueuedInlineAssistantPrompt[]>([]);
  const [expandedNearbyItemId, setExpandedNearbyItemId] = useState<string | null>(null);
  const [workScanQuery, setWorkScanQuery] = useState('');
  const [workScanScope, setWorkScanScope] = useState<WorkScanScope>('all');
  const [workScanSort, setWorkScanSort] = useState<WorkScanSort>('lane');
  const lastAssistantResultRef = useRef<ContextualAssistantResponse | null>(null);
  const turnSeqRef = useRef(0);
  const selectedInlineRef = useRef<HTMLDivElement | null>(null);
  const expandedNearbyRef = useRef<HTMLDivElement | null>(null);
  // Desktop right pane: scrolls on its own and starts at the top for each
  // newly selected item (owner 2026-09-16 desktop pass).
  const desktopDetailRef = useRef<HTMLElement | null>(null);
  const desktopLayout = useDesktopLayout();
  // Viewport top of the row the owner just tapped (null for automatic
  // selection). After the commit the row is scrolled back to exactly this
  // position so it never moves — even when an expanded item above it collapsed.
  const tappedRowTopRef = useRef<number | null>(null);
  const tappedNearbyTopRef = useRef<number | null>(null);
  const previousSelectedIdRef = useRef<string | null>(null);
  // Set by an owner tap in this view. The App also selects on its own (first
  // priority item on landing, queue refreshes); those must not move the page —
  // landing on 工作 used to scroll the 查找 card and queue summary under the
  // sticky header before the owner touched anything.
  const ownerSelectedRef = useRef(false);
  const visibleLanes = lanes.filter((lane) => lane.items.length > 0);
  const hasItems = visibleLanes.length > 0;
  const allVisibleItems = visibleLanes.flatMap((lane) => lane.items);
  const firstLane = visibleLanes[0] ?? null;
  const firstItem = selected ?? firstLane?.items[0] ?? null;
  const attentionLanes = lanes.filter((lane) => lane.id !== 'backlog' && lane.id !== 'done');
  const activeAttentionLanes = attentionLanes.filter((lane) => lane.items.length > 0);
  const quietLanes = lanes.filter((lane) => lane.id === 'backlog' || lane.id === 'done');
  const activeQuietLanes = quietLanes.filter((lane) => lane.items.length > 0);
  const selectedQuietLane = selected
    ? quietLanes.find((lane) => lane.items.some((item) => item.id === selected.id))
    : null;
  const switcherLanes = selected
    ? [...activeAttentionLanes, ...(selectedQuietLane ? [selectedQuietLane] : [])]
    : [...activeAttentionLanes, ...activeQuietLanes];
  const allNearbyCandidates = selected
    ? switcherLanes.flatMap((lane) => lane.items).filter((item) => item.id !== selected.id)
    : [];
  const actionableNearbyItems = allNearbyCandidates.filter(itemShouldAppearInNearbyWork);
  const quietNearbyItems = allNearbyCandidates.filter((item) => !itemShouldAppearInNearbyWork(item));
  const nearbyItems = actionableNearbyItems.slice(0, 8);
  const quietNearbyCount = quietNearbyItems.length;
  const queueSummary = workQueueSummary(visibleLanes, selected, nearbyItems.length, quietNearbyCount, locale);
  const workScanActive = workScanQuery.trim().length > 0 || workScanScope !== 'all' || workScanSort !== 'lane';
  // 查找 lists *other* work, so the selected item stays out of the pool — except
  // on a one-item queue, where excluding it showed "共 0 项".
  const workScanPool = allVisibleItems.length <= 1 || !selected
    ? allVisibleItems
    : allVisibleItems.filter((item) => item.id !== selected.id);
  const workScanResults = filterWorkForScan(workScanPool, {
    query: workScanQuery,
    scope: workScanScope,
    sort: workScanSort,
  });
  const workItemsById = useMemo(
    () => new Map(lanes.flatMap((lane) => lane.items).map((item) => [item.id, item] as const)),
    [lanes],
  );

  useEffect(() => {
    setAssistantComposerItemId(null);
    setQueuedAssistantPrompts([]);
    setExpandedNearbyItemId(null);
  }, [selected?.id]);

  useEffect(() => {
    const pane = desktopDetailRef.current;
    if (pane) pane.scrollTop = 0;
  }, [selected?.id]);

  useEffect(() => {
    const previousId = previousSelectedIdRef.current;
    const nextId = selected?.id ?? null;
    previousSelectedIdRef.current = nextId;
    const ownerSelected = ownerSelectedRef.current;
    ownerSelectedRef.current = false;
    // Only when the owner opens a newly selected item — never on collapse,
    // never for the App's automatic selection. Skip when the mobile inline
    // host is CSS-hidden (desktop two-pane).
    const tappedTop = tappedRowTopRef.current;
    tappedRowTopRef.current = null;
    if (!nextId || nextId === previousId || !(ownerSelected || nextId === requestedItemId)) return;
    const host = selectedInlineRef.current;
    if (!host || host.offsetParent === null) return;
    keepRowTopInPlace(host, tappedTop);
  }, [selected?.id, requestedItemId]);

  useEffect(() => {
    // Same rule for a nearby row that the owner expands inline.
    const tappedTop = tappedNearbyTopRef.current;
    tappedNearbyTopRef.current = null;
    const host = expandedNearbyRef.current;
    if (!expandedNearbyItemId || !host || host.offsetParent === null) return;
    keepRowTopInPlace(host, tappedTop);
  }, [expandedNearbyItemId]);

  useEffect(() => {
    if (!assistantResultItemId || !assistantResult || lastAssistantResultRef.current === assistantResult) return;
    lastAssistantResultRef.current = assistantResult;
    setAssistantComposerItemId(assistantResultItemId);
    setAssistantTurnsForItem(assistantResultItemId, (current) => {
      const withoutWorking = current.filter((turn) => turn.role !== 'working');
      if (assistantResult.response_kind === 'working') {
        return trimAssistantTurns([
          ...withoutWorking,
          {
            id: nextAssistantTurnId(turnSeqRef, 'working'),
            role: 'working',
            text: t('work.assistant.working', locale),
            timestamp: Date.now(),
          },
        ]);
      }
      return trimAssistantTurns([
        ...withoutWorking,
        {
          id: nextAssistantTurnId(turnSeqRef, 'aegis'),
          role: 'aegis',
          result: assistantResult,
          timestamp: Date.now(),
        },
      ]);
    });
  }, [assistantResult, assistantResultItemId, locale]);

  function assistantTextForItem(item: OwnerWorkItem): string {
    return assistantTextByItemId[item.id] ?? '';
  }

  function assistantTurnsForItem(item: OwnerWorkItem): InlineAssistantTurn[] {
    return assistantTurnsByItemId[item.id] ?? [];
  }

  function assistantResultForItem(item: OwnerWorkItem | null): ContextualAssistantResponse | null {
    if (!item || item.id !== assistantResultItemId) return null;
    return assistantResult;
  }

  function setAssistantTextForItem(itemId: string, text: string) {
    setAssistantTextByItemId((current) => ({ ...current, [itemId]: text }));
  }

  function setAssistantTurnsForItem(itemId: string, update: (turns: InlineAssistantTurn[]) => InlineAssistantTurn[]) {
    setAssistantTurnsByItemId((current) => ({
      ...current,
      [itemId]: update(current[itemId] ?? []),
    }));
  }

  function appendOwnerTurn(item: OwnerWorkItem, text: string) {
    const prompt = text.trim();
    if (!prompt) return;
    setAssistantComposerItemId(item.id);
    setAssistantTurnsForItem(item.id, (current) => trimAssistantTurns([
      ...current.filter((turn) => turn.role !== 'working'),
      {
        id: nextAssistantTurnId(turnSeqRef, 'owner'),
        role: 'owner',
        text: prompt,
        timestamp: Date.now(),
      },
    ]));
  }

  function openAssistant(item: OwnerWorkItem, prefill = '') {
    setAssistantComposerItemId(item.id);
    if (prefill) {
      setAssistantTextForItem(item.id, prefill);
    }
  }

  function submitAssistant(item: OwnerWorkItem, text: string) {
    const prompt = text.trim();
    if (!prompt) return;
    // Close follow-ups via receipt-backed actions instead of another LLM "确认吗?" loop.
    if (shouldExecuteFollowUpCleanup(item, prompt, assistantTurnsForItem(item), locale)) {
      const closeAction = followUpCloseAction(item, locale);
      if (closeAction) {
        openAssistant(item);
        appendOwnerTurn(item, prompt);
        setAssistantTextForItem(item.id, '');
        if (onExecuteAction) onExecuteAction(item, closeAction);
        else onAction(item, closeAction);
        return;
      }
    }
    if (busy) {
      const queuedTurnId = nextAssistantTurnId(turnSeqRef, 'queued');
      setAssistantComposerItemId(item.id);
      setAssistantTurnsForItem(item.id, (current) => trimAssistantTurns([
        ...current,
        {
          id: queuedTurnId,
          role: 'queued',
          text: prompt,
          timestamp: Date.now(),
        },
      ]));
      setQueuedAssistantPrompts((current) => [
        ...current,
        { id: queuedTurnId, itemId: item.id, text: prompt, kind: 'prompt' },
      ]);
      setAssistantTextForItem(item.id, '');
      return;
    }
    const conversationContext = inlineAssistantConversationContext(assistantTurnsForItem(item));
    openAssistant(item);
    appendOwnerTurn(item, prompt);
    setAssistantTextForItem(item.id, '');
    onAskText(item, prompt, conversationContext);
  }

  useEffect(() => {
    if (busy || queuedAssistantPrompts.length === 0) return;
    const nextPrompt = queuedAssistantPrompts.find((prompt) => workItemsById.has(prompt.itemId));
    if (!nextPrompt) return;
    const nextItem = workItemsById.get(nextPrompt.itemId);
    if (!nextItem) return;
    setQueuedAssistantPrompts((current) => current.filter((prompt) => prompt.id !== nextPrompt.id));
    const currentTurns = assistantTurnsByItemId[nextItem.id] ?? [];
    const conversationContext = inlineAssistantConversationContextBeforeQueuedTurn(currentTurns, nextPrompt.id);
    setAssistantTurnsForItem(nextItem.id, (current) => current.map((turn) => (
      turn.id === nextPrompt.id ? { ...turn, role: 'owner' as const } : turn
    )));
    if (nextPrompt.kind === 'retry') {
      onRetryAssistant?.();
      return;
    }
    setAssistantComposerItemId(nextItem.id);
    onAskText(nextItem, nextPrompt.text, conversationContext);
  }, [assistantTurnsByItemId, busy, onAskText, onRetryAssistant, queuedAssistantPrompts, workItemsById]);

  function retryAssistant() {
    if (!selected || !onRetryAssistant) return;
    // The english label stays the identity compared here; the displayed/sent
    // retry text localizes.
    const retryText = onRetryAssistantLabel === 'Check result' ? t('work.assistant.retry.checkResult', locale) : t('work.assistant.retry.tryAgain', locale);
    if (busy) {
      const queuedTurnId = nextAssistantTurnId(turnSeqRef, 'queued');
      setAssistantComposerItemId(selected.id);
      setAssistantTurnsForItem(selected.id, (current) => trimAssistantTurns([
        ...current,
        {
          id: queuedTurnId,
          role: 'queued',
          text: retryText,
          timestamp: Date.now(),
        },
      ]));
      setQueuedAssistantPrompts((current) => [
        ...current,
        { id: queuedTurnId, itemId: selected.id, text: retryText, kind: 'retry' },
      ]);
      return;
    }
    appendOwnerTurn(selected, retryText);
    openAssistant(selected);
    onRetryAssistant();
  }

  function runBrief() {
    if (!selected) return;
    if (busy) {
      submitAssistant(selected, briefPrompt(locale));
      return;
    }
    appendOwnerTurn(selected, briefPrompt(locale));
    openAssistant(selected);
    onBrief();
  }

  function runDraft() {
    if (!selected) return;
    if (busy) {
      submitAssistant(selected, draftPrompt(locale));
      return;
    }
    appendOwnerTurn(selected, draftPrompt(locale));
    openAssistant(selected);
    onDraft();
  }

  function runDecisionPrompt(prompt: string, label: string) {
    if (!selected) return;
    if (busy) {
      submitAssistant(selected, prompt);
      return;
    }
    appendOwnerTurn(selected, label);
    openAssistant(selected);
    setAssistantTextForItem(selected.id, '');
    onAskText(selected, prompt, inlineAssistantConversationContext(assistantTurnsForItem(selected)));
  }

  function selectItem(id: string, opts?: { toggle?: boolean }) {
    if (opts?.toggle && id === selected?.id) {
      // Row re-tap collapses in place; scroll effect skips when selection clears.
      onSelect(null);
      return;
    }
    if (id !== selected?.id) {
      setAssistantComposerItemId(null);
    }
    ownerSelectedRef.current = true;
    onSelect(id);
  }

  // C4: arrow-key roving between item rows. The rows are native buttons (Tab +
  // Enter already work); this lets Up/Down move focus to the adjacent row in DOM
  // order across lanes, the expected behaviour for a vertical list. Pure focus
  // management — no selection/state change until the owner activates a row.
  function onRowKeyNav(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const target = event.target as HTMLElement;
    if (!target.classList?.contains('vnext-item-row')) return;
    // Only rove among VISIBLE rows: the same .vnext-item-row class is rendered
    // inside the closed work-brief <details> and CSS-hidden responsive containers
    // (desktop vs mobile), so an unfiltered list could pick a hidden row as next
    // — focus() would no-op while preventDefault swallowed the key. offsetParent
    // is null for any element with a display:none ancestor.
    const rows = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('.vnext-item-row'))
      .filter((row) => row.offsetParent !== null);
    const index = rows.indexOf(target);
    if (index < 0) return;
    const next = event.key === 'ArrowDown' ? rows[index + 1] : rows[index - 1];
    if (next) {
      event.preventDefault();
      next.focus();
    }
  }

  function toggleNearbyDetail(id: string) {
    setExpandedNearbyItemId((current) => current === id ? null : id);
  }

  function renderNearbyItem(item: OwnerWorkItem) {
    const expanded = expandedNearbyItemId === item.id;
    return (
      <div
        key={item.id}
        ref={expanded ? expandedNearbyRef : undefined}
        className={classNames('vnext-nearby-work-item', expanded && 'is-expanded')}
      >
        <WorkItemRow
          item={item}
          active={expanded}
          compact
          onClick={(event) => {
            tappedNearbyTopRef.current = event.currentTarget.getBoundingClientRect().top;
            toggleNearbyDetail(item.id);
          }}
        />
        {expanded && (
          <div className="vnext-work-expand">
            <NearbyInlineDetail
              item={item}
              assistantOpen={assistantComposerItemId === item.id}
              assistantText={assistantTextForItem(item)}
              assistantResult={assistantResultForItem(item)}
              assistantTurns={assistantTurnsForItem(item)}
              armedAction={armedAction}
              armedActionItemId={armedActionItemId}
              executingAction={executingAction}
              busy={busy}
              onAction={onAction}
              onOpenAssistant={(prefill) => openAssistant(item, prefill)}
              onAskTextChange={(text) => setAssistantTextForItem(item.id, text)}
              onAskText={(text) => submitAssistant(item, text)}
              onCloseAssistant={() => setAssistantComposerItemId(null)}
              onOpenLineageMessage={onOpenLineageMessage}
            />
          </div>
        )}
      </div>
    );
  }

  function focusLane(lane: LaneProjection) {
    if (lane.items.length === 0) {
      onToggleLane(lane.id);
      return;
    }
    if (lane.collapsed && !expandedLaneIds.includes(lane.id)) {
      onToggleLane(lane.id);
    }
    selectItem(lane.items[0].id);
  }

  function renderItemDetail(className?: string) {
    return (
      <ItemDetail
        item={selected}
        context={context}
        itemProjection={itemProjection}
        sourceDetail={sourceDetail}
        sourceDetailBusy={sourceDetailBusy}
        assistantResult={assistantResultForItem(selected)}
        actionMessage={actionMessage}
        hostStepUpBusy={hostStepUpBusy}
        onHostStepUp={onHostStepUp}
        actionsRetired={actionsRetired}
        armedAction={armedAction}
        armedActionItemId={armedActionItemId}
        executingAction={executingAction}
        busy={busy}
        assistantOpen={Boolean(selected && assistantComposerItemId === selected.id)}
        assistantText={selected ? assistantTextForItem(selected) : ''}
        assistantTurns={selected ? assistantTurnsForItem(selected) : []}
        onOpenSource={onOpenSource}
        onAction={onAction}
        onDecisionPrompt={runDecisionPrompt}
        onBrief={runBrief}
        onDraft={runDraft}
        onOpenAssistant={(prefill) => selected && openAssistant(selected, prefill)}
        onAskTextChange={(text) => selected && setAssistantTextForItem(selected.id, text)}
        onAskText={(text) => selected && submitAssistant(selected, text)}
        onCloseAssistant={() => setAssistantComposerItemId(null)}
        onRetryAssistant={retryAssistant}
        onRetryAssistantLabel={onRetryAssistantLabel}
        onOpenLineageMessage={onOpenLineageMessage}
        className={className}
        hostRef={className === 'vnext-detail-desktop' ? desktopDetailRef : undefined}
      />
    );
  }

  function renderMobileLaneItem(item: OwnerWorkItem) {
    const active = item.id === selected?.id;
    return (
      <div
        key={item.id}
        ref={active ? selectedInlineRef : undefined}
        className={classNames('vnext-work-mobile-inline-item', active && 'active')}
      >
        <WorkItemRow
          item={item}
          active={active}
          onClick={(event) => {
            tappedRowTopRef.current = event.currentTarget.getBoundingClientRect().top;
            selectItem(item.id, { toggle: true });
          }}
        />
        {active ? <div className="vnext-work-expand">{renderItemDetail('vnext-work-inline-selected')}</div> : null}
      </div>
    );
  }

  return (
    <div className="vnext-work-layout" onKeyDown={onRowKeyNav}>
      <section className="vnext-stack">
        {!hasItems && (
          <section className="vnext-panel vnext-empty-work">
            <p className="vnext-kicker">{t('work.kicker', locale)}</p>
            <h2>{t('work.empty.headline', locale)}</h2>
            <p className="vnext-muted">{t('work.empty.body', locale)}</p>
          </section>
        )}
        <WorkActionMessage message={workNotice} />
        {hasItems && (
          <WorkScanControls
            query={workScanQuery}
            scope={workScanScope}
            sort={workScanSort}
            active={workScanActive}
            resultCount={workScanResults.length}
            totalCount={workScanPool.length}
            onQueryChange={setWorkScanQuery}
            onScopeChange={setWorkScanScope}
            onSortChange={setWorkScanSort}
            onClear={() => {
              setWorkScanQuery('');
              setWorkScanScope('all');
              setWorkScanSort('lane');
            }}
          />
        )}
        {hasItems && (allVisibleItems.length > 1 || visibleLanes.length > 1) && (
          <details className={classNames('vnext-work-brief', 'vnext-disclosure', selected && 'is-secondary')} aria-label={t('work.aria.queueBrief', locale)} open={!selected}>
            <summary className="vnext-disclosure-summary">
              <span className="vnext-disclosure-copy">
                <p className="vnext-kicker">{workQueueKicker(selected, nearbyItems.length, quietNearbyCount, locale)}</p>
                <h2>{queueSummary.headline}</h2>
                <p className="vnext-disclosure-hint">{queueSummary.body}</p>
              </span>
            </summary>
            {firstItem && !selected && (
              <button type="button" className="vnext-work-brief-action" onClick={() => selectItem(firstItem.id)}>
                <span>{t('work.startWith', locale)}</span>
                <strong>{ownerVisibleWorkTitle(firstItem, t('work.title.theFirstItem', locale), 150, locale)}</strong>
              </button>
            )}
            <div className="vnext-work-lane-strip" aria-label={t('work.aria.lanes', locale)}>
              {switcherLanes.map((lane) => (
                <button
                  key={lane.id}
                  type="button"
                  className={classNames(
                    lane.items.length === 0 && 'is-empty',
                    lane.items.some((item) => item.id === selected?.id) && 'active',
                    lane.id,
                  )}
                  aria-label={workLaneAriaLabel(lane.id, locale)}
                  aria-pressed={lane.items.some((item) => item.id === selected?.id)}
                  onClick={() => focusLane(lane)}
                >
                  <span>{lane.title}</span>
                  <strong>{lane.items.length}</strong>
                  <em>{laneCue(lane, locale)}</em>
                </button>
              ))}
            </div>
            {selected && !workScanActive && nearbyItems.length > 0 && (
              <div className="vnext-work-switcher-list" aria-label={t('work.aria.otherItems', locale)}>
                {nearbyItems.map((item) => renderNearbyItem(item))}
              </div>
            )}
            {selected && !workScanActive && quietNearbyItems.length > 0 && (
              <div className="vnext-quiet-nearby-list" aria-label={t('work.aria.quietWatching', locale)}>
                <p className="vnext-quiet-nearby-note">
                  {tf('work.quietWatching.note', locale, {
                    count: String(quietNearbyCount),
                    loopWord: t(quietNearbyCount === 1 ? 'work.loop.one' : 'work.loop.other', locale),
                  })}
                </p>
                {quietNearbyItems.slice(0, 8).map((item) => renderNearbyItem(item))}
              </div>
            )}
          </details>
        )}
        {hasItems && workScanActive && (
          <WorkScanResults
            items={workScanResults}
            selected={selected}
            onSelect={selectItem}
          />
        )}
        {selected && !workScanActive && nearbyItems.length > 0 && (
          <section className="vnext-panel vnext-work-desktop-nearby" aria-label={t('work.aria.nextPreview', locale)}>
            <div>
              <p className="vnext-kicker">{t('work.next.kicker', locale)}</p>
              <h2>{t('work.next.heading', locale)}</h2>
            </div>
            <div className="vnext-work-switcher-list" aria-label={t('work.aria.nearbyItems', locale)}>
              {nearbyItems.slice(0, 4).map((item) => renderNearbyItem(item))}
            </div>
          </section>
        )}
        {/* Desktop two-pane: the lane list stays while an item is open (the
            detail sits in the right pane). Phones switch to the inline lanes
            below; `has-selection` hides these there (styles.css). */}
        {hasItems && !workScanActive && visibleLanes.map((lane) => {
          const collapsed = lane.collapsed && !expandedLaneIds.includes(lane.id);
          const laneItemWord = t(lane.items.length === 1 ? 'work.item.one' : 'work.item.other', locale);
          const countLabel = tf('work.itemCount', locale, { count: String(lane.items.length), itemWord: laneItemWord });
          return (
            <section key={lane.id} className={classNames('vnext-panel', 'vnext-work-lane', lane.id, collapsed && 'is-collapsed', selected && 'has-selection')}>
              <button
                type="button"
                className="vnext-lane-head"
                onClick={() => onToggleLane(lane.id)}
                aria-expanded={!collapsed}
              >
                <div>
                  <h2>{lane.title}</h2>
                  {!collapsed && <p>{lane.subtitle}</p>}
                </div>
                <span className="vnext-lane-count">{collapsed ? tf('work.itemCountHidden', locale, { count: String(lane.items.length), itemWord: laneItemWord }) : countLabel}</span>
              </button>
              {!collapsed && lane.items.map((item) => (
                <WorkItemRow
                  key={item.id}
                  item={item}
                  active={item.id === selected?.id}
                  onClick={() => selectItem(item.id)}
                />
              ))}
            </section>
          );
        })}
        {hasItems && selected && (
          // Phone only: on the two-pane layout the detail is the right pane and
          // a hidden inline copy would shadow it as the document's first
          // `.vnext-detail`.
          desktopLayout ? null : (
          <div className="vnext-work-mobile-inline-lanes" aria-label={t('work.aria.mobileInlineLanes', locale)}>
            {visibleLanes.map((lane) => {
              const laneHasSelected = lane.items.some((item) => item.id === selected.id);
              const collapsed = lane.collapsed && !expandedLaneIds.includes(lane.id) && !laneHasSelected;
              const laneItemWord = t(lane.items.length === 1 ? 'work.item.one' : 'work.item.other', locale);
              const countLabel = tf('work.itemCount', locale, { count: String(lane.items.length), itemWord: laneItemWord });
              return (
                <section key={lane.id} className={classNames('vnext-panel', 'vnext-work-lane', lane.id, collapsed && 'is-collapsed')}>
                  <button
                    type="button"
                    className="vnext-lane-head"
                    onClick={() => onToggleLane(lane.id)}
                    aria-expanded={!collapsed}
                  >
                    <div>
                      <h2>{lane.title}</h2>
                      {!collapsed && <p>{lane.subtitle}</p>}
                    </div>
                    <span className="vnext-lane-count">{collapsed ? tf('work.itemCountHidden', locale, { count: String(lane.items.length), itemWord: laneItemWord }) : countLabel}</span>
                  </button>
                  {!collapsed && lane.items.map((item) => renderMobileLaneItem(item))}
                </section>
              );
            })}
          </div>
          )
        )}
      </section>
      {renderItemDetail('vnext-detail-desktop')}
    </div>
  );
}

function ItemDetail({
  item,
  context,
  itemProjection,
  sourceDetail,
  sourceDetailBusy,
  assistantResult,
  actionMessage,
  hostStepUpBusy = false,
  onHostStepUp,
  actionsRetired = false,
  armedAction,
  armedActionItemId,
  executingAction,
  busy,
  assistantOpen,
  assistantText,
  assistantTurns,
  onOpenSource,
  onAction,
  onDecisionPrompt,
  onBrief,
  onDraft,
  onOpenAssistant,
  onAskTextChange,
  onAskText,
  onCloseAssistant,
  onRetryAssistant,
  onRetryAssistantLabel,
  onOpenLineageMessage,
  className,
  hostRef,
}: {
  item: OwnerWorkItem | null;
  context: OwnerWorkContextEnvelope | null;
  itemProjection: VNextItemProjection | null;
  sourceDetail: SourceDetailView | null;
  sourceDetailBusy: boolean;
  assistantResult: ContextualAssistantResponse | null;
  actionMessage: string;
  hostStepUpBusy?: boolean;
  onHostStepUp?: () => void;
  actionsRetired: boolean;
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  busy: boolean;
  assistantOpen: boolean;
  assistantText: string;
  assistantTurns: InlineAssistantTurn[];
  onOpenSource: () => void;
  onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
  onDecisionPrompt: (prompt: string, label: string) => void;
  onBrief: () => void;
  onDraft: () => void;
  onOpenAssistant: (prefill?: string) => void;
  onAskTextChange: (text: string) => void;
  onAskText: (text: string, conversationContext?: VNextConversationContextTurn[]) => void;
  onCloseAssistant: () => void;
  onRetryAssistant?: () => void;
  onRetryAssistantLabel?: string;
  onOpenLineageMessage?: (cite: LineageChatCite) => void;
  className?: string;
  /** The scrolling pane element (desktop right pane) for scroll resets. */
  hostRef?: MutableRefObject<HTMLElement | null>;
}) {
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  if (!item) {
    return (
      <aside ref={hostRef} className={classNames('vnext-detail', className)}>
        <p className="vnext-muted">{t('work.detail.selectPrompt', locale)}</p>
      </aside>
    );
  }

  const visibleEntries = context ? Object.entries(context.visible_fields).slice(0, 6) : [];
  const authority = item.authority;
  const itemActions = buildSupportedItemActions(item, locale);
  const visibleData = authority
    ? authority.visible_fields
    : visibleEntries.map(([key]) => key);
  const lockedData = authority
    ? authority.locked_fields
    : context?.redacted_fields ?? [];
  const allowedNow = authority
    ? authority.allowed_now
    : itemActions.map((action) => action.label);
  const detailAccessStillClosed = !sourceDetail && (Boolean(authority?.requires_mfa) || lockedData.length > 0);
  const relatedWorkCount = item.related_action_ids.length + item.related_receipt_ids.length;
  const serverSafeDetail = itemProjection?.safe_source_detail?.trim();
  const history = itemProjection?.history ?? [];
  const openedTaskDetail = sourceDetail?.kind === 'task' ? sourceDetail : null;
  const blockedByGate = itemIsBlockedByGate(item);
  const closedItem = isClosedItem(item);
  const title = blockedByGate
    ? ownerVisibleWorkTitle(item, t('work.title.pausedAutomation', locale), 220, locale)
    : openedTaskDetail?.title || ownerVisibleWorkTitle(item, t('work.title.selectedItem', locale), 220, locale);
  const needsPrivateDetail = itemNeedsPrivateTaskAccess(item);
  const privateDetailStillClosed = needsPrivateDetail && !sourceDetail;
  const delayReceiptActionsUntilDetail = privateDetailStillClosed;
  const visibleItemActions = delayReceiptActionsUntilDetail || actionsRetired ? [] : itemActions;
  const dockItemActions = visibleItemActions.filter((action) => !lineageHostedActionIds(item).has(action.id));
  const summary = blockedByGate
    ? blockedByGateSummary(item, locale)
    : closedItem
      ? ownerVisibleWorkCue(item, 220, userTimezone, locale)
      : openedTaskDetail?.subtitle || ownerVisibleWorkCue(item, 220, userTimezone, locale);
  // This panel is already the opened detail. `sourceDetail` is a second fetch,
  // not the open state, so waiting for it left `task_open_or_act` ("open the
  // detail") on a card the owner had already opened. A still-locked private
  // body is the case that still needs that instruction.
  const summaryForOpenCard = privateDetailStillClosed || !isOpenDetailInstruction(summary, locale)
    ? summary
    : '';
  const nextSafeMove = privateDetailStillClosed || !isTaskLikeItem(item)
    ? safeNextMoveLabel(item, title, locale)
    : detailOpenNextMoveLabel(item, locale);
  const usefulReason = closedItem ? '' : ownerVisibleReasonCue(item, locale);
  const timingLabel = item.due_at ? ownerTimeLabel(item.due_at, userTimezone, locale) : '';
  const followUpControls = followUpQuickControls(item, locale);
  const followUpItem = laneForItem(item) === 'follow_up'
    || ['waiting', 'tracking'].includes(item.status?.trim().toLowerCase() ?? '');
  const focusLabel = itemNeedsDecision(item)
    ? t('work.focus.decision', locale)
    : privateDetailStillClosed
      ? t('work.focus.needsDetail', locale)
    : blockedByGate
      ? t('work.focus.paused', locale)
    : isClosedItem(item)
      ? t('work.focus.handled', locale)
    : laneForItem(item) === 'follow_up'
      ? t('work.focus.tracking', locale)
      : item.status?.toLowerCase() === 'scheduled'
        ? t('work.focus.comingUp', locale)
        : t('work.focus.ready', locale);
  // Follow-up intent grid: only signal that helps decide (when + next). Drop
  // generic "why" / policy receipts that restate the lane.
  // 为何 that merely repeats the summary above it is noise (2026-09-13 walk).
  const showFollowUpWhy = Boolean(
    usefulReason && !isGenericFollowUpReason(usefulReason) && usefulReason.trim() !== summary.trim(),
  );
  const showFollowUpReceipt = (() => {
    if (closedItem || followUpItem) return false;
    return !isGenericFollowUpReceiptExpectation(item.receipt_expectation);
  })();

  let moreActions: ReactNode = null;
  let xAnswerExtras: string[] = [];
  return (
    <aside ref={hostRef} className={classNames('vnext-detail', followUpItem && 'is-follow-up', className)}>
      <section className="vnext-detail-hero" aria-label={t('work.aria.selectedItem', locale)}>
        <div className="vnext-detail-title-row">
          <span className={classNames('vnext-focus-chip', (itemNeedsDecision(item) || blockedByGate) && 'warn')}>{focusLabel}</span>
        </div>
        <h2 title={title}>{title}</h2>
        {summaryForOpenCard ? <p>{summaryForOpenCard}</p> : null}
        <span className={classNames('vnext-item-meta', itemHasPriority(item) && 'is-priority')}>{detailItemMeta(item, locale)}</span>
        {!blockedByGate && !privateDetailStillClosed && (
          <div className="vnext-work-intent-row" aria-label={t('work.aria.currentSummary', locale)}>
            {timingLabel && (
              <span>
                <em>{t('work.intent.when', locale)}</em>
                <strong>{timingLabel}</strong>
              </span>
            )}
            {showFollowUpWhy && (
              <span>
                <em>{t('work.intent.why', locale)}</em>
                <strong>{usefulReason}</strong>
              </span>
            )}
            <span>
              <em>{sourceDetail ? t('work.intent.detail', locale) : stateCueLabel(item, needsPrivateDetail, locale)}</em>
              <strong>{nextSafeMove}</strong>
            </span>
            {showFollowUpReceipt && (() => {
              const rawProof = item.receipt_expectation?.trim() || '';
              if (!rawProof || isGenericFollowUpReceiptExpectation(rawProof)) return null;
              const proof = ownerFacingReceiptText(rawProof, locale);
              return (
              <span className="vnext-work-trust-receipt">
                <em>{t('work.trust.receiptLabel', locale)}</em>
                <strong>{proof}</strong>
              </span>
              );
            })()}
          </div>
        )}
      </section>

      <WorkLineagePanel
        item={item}
        actions={visibleItemActions}
        armedAction={armedAction}
        armedActionItemId={armedActionItemId}
        executingAction={executingAction}
        onAction={onAction}
        onOpenLineageMessage={onOpenLineageMessage}
      />

      {!blockedByGate && (
        <section className="vnext-mini-panel vnext-action-panel" aria-label={t('work.aria.workWithAegis', locale)}>
          <div className="vnext-action-primary-row vnext-action-primary-row-stacked">
            {(() => {
              // Follow-ups rarely need "打开详情" as the primary — put 清理 first.
              const offerOpenDetail = !sourceDetail && (!followUpItem || privateDetailStillClosed);
              const xHref = extractXUrlFromWorkItem(item);
              const xDraftHref = xHref ? buildXReplyComposeUrl(xHref) : null;
              const finishedAnswers = [
                ...assistantTurns.flatMap((turn) => (
                  turn.result && turn.result.response_kind !== 'working' ? [turn.result] : []
                )),
                ...(assistantResult && assistantResult.response_kind !== 'working' ? [assistantResult] : []),
              ];
              const answerXLinks = finishedAnswers.flatMap((answer) => collectXLinksFromAssistantResult(answer));
              const xHandoff = mergeWorkAndAnswerXHandoff({
                assistantOpen: workDetailAskPlacement(assistantOpen) === 'inline_panel',
                finishedAnswerVisible: finishedAnswers.length > 0,
                workHref: xHref,
                answerLinks: answerXLinks,
              });
              xAnswerExtras = xHandoff.extraLinks;
              const surface = presentOwnerWorkSurfaceActions({
                item,
                locale,
                sourceDetailOpen: Boolean(sourceDetail),
                includeOpenDetail: offerOpenDetail,
                openDetailLabel: sourceDetailBusy
                  ? t('work.action.opening', locale)
                  : ownerVisiblePrimaryActionLabel(item, t('work.action.openDetails', locale), 40, locale),
                openX: xHref && !xHandoff.suppressDock
                  ? {
                      href: xHref,
                      label: t('work.x.openInX', locale),
                      draftHref: xDraftHref || undefined,
                      draftLabel: xDraftHref ? t('work.x.draftReplyInX', locale) : undefined,
                    }
                  : null,
                includeAssistantChips: !privateDetailStillClosed && !closedItem && !followUpItem,
                briefLabel: assistantBriefLabel(item, locale),
                draftLabel: t('work.action.draft', locale),
                itemActions: dockItemActions,
              });
              if (!surface.primary && surface.secondary.length === 0) return null;
              const controlProps = {
                item,
                locale,
                sourceDetailBusy,
                armedAction,
                armedActionItemId,
                executingAction,
                onOpenSource,
                onAction,
                onOpenAssistant,
                onBrief,
                onDraft,
              };
              const hasReceiptActions = Boolean(
                (surface.primary?.kind === 'item_action')
                || surface.secondary.some((c) => c.kind === 'item_action'),
              );
              // The secondary actions disclosure renders AFTER the follow-up
              // buttons (below) so the stack reads: one filled primary, then
              // the outlined buttons, then the disclosure rows (owner 2026-09-16).
              moreActions = surface.secondary.length > 0 ? (
                <details
                  className="vnext-work-more-actions vnext-disclosure"
                  aria-label={t('work.aria.moreActions', locale)}
                >
                  <summary className="vnext-disclosure-summary">{t('work.moreActions', locale)}</summary>
                  {hasReceiptActions && (
                    <div className="vnext-item-receipt-actions" aria-label={t('work.aria.receiptActions', locale)}>
                      <span>{itemActionGroupLabel(item, locale)}</span>
                    </div>
                  )}
                  <div className="vnext-action-row vnext-work-more-actions-row" aria-label={t('work.aria.quickActions', locale)}>
                    {surface.secondary.map((control) =>
                      renderOwnerWorkSurfaceControl(control, { ...controlProps, primary: false }),
                    )}
                  </div>
                </details>
              ) : null;
              if (!surface.primary) return null;
              return (
                <div className="vnext-action-dock-grid" aria-label={t('work.aria.itemControls', locale)}>
                  {renderOwnerWorkSurfaceControl(surface.primary, { ...controlProps, primary: true })}
                </div>
              );
            })()}
            {workDetailAskPlacement(assistantOpen) === 'inline_panel' ? (
              <InlineAssistantPanel
                text={assistantText}
                placeholder={t('work.askAegis.placeholderItem', locale)}
                result={assistantResult}
                turns={assistantTurns}
                extraXLinks={xAnswerExtras}
                suggestions={buildCommandSuggestions(item, 'item', locale)}
                busy={busy}
                onChange={onAskTextChange}
                onSend={onAskText}
                onClose={onCloseAssistant}
                onRetry={onRetryAssistant}
                retryLabel={onRetryAssistantLabel}
              />
            ) : (
              <VNextButton
                variant="ghost"
                size="sm"
                className="vnext-work-ask-compact"
                leadingIcon={<VNextIcon name="spark" />}
                aria-label={t('work.aria.askThisItem', locale)}
                title={sourceDetail ? t('work.askAegis.usingOpenedDetail', locale) : t('work.askAegis.usingItemContext', locale)}
                onClick={() => onOpenAssistant()}
              >
                {t('work.askAegis', locale)}
              </VNextButton>
            )}
          </div>
          {followUpControls.length > 0 && (
            <FollowUpPromptStrip
              controls={followUpControls}
              item={item}
              armedAction={armedAction}
              armedActionItemId={armedActionItemId}
              executingAction={executingAction}
              onPrompt={(prompt) => onOpenAssistant(prompt)}
              onCloseAction={(action) => onAction(item, action)}
            />
          )}
          {moreActions}
        </section>
      )}

      {blockedByGate && (
        <BlockedByGatePanel
          item={item}
          assistantOpen={assistantOpen}
          assistantText={assistantText}
          assistantResult={assistantResult}
          assistantTurns={assistantTurns}
          onDecisionPrompt={onDecisionPrompt}
          itemActions={visibleItemActions}
          onAction={onAction}
          onAskTextChange={onAskTextChange}
          onAskText={onAskText}
          onCloseAssistant={onCloseAssistant}
          onRetryAssistant={onRetryAssistant}
          onRetryAssistantLabel={onRetryAssistantLabel}
          onOpen={onOpenSource}
          visibleData={visibleData}
          lockedData={lockedData}
          armedAction={armedAction}
          armedActionItemId={armedActionItemId}
          executingAction={executingAction}
          busy={busy}
          sourceDetailBusy={sourceDetailBusy}
        />
      )}

      <WorkActionMessage message={actionMessage} />
      {onHostStepUp && (
        <div className="vnext-action-row">
          <VNextButton
            variant="primary"
            className="vnext-host-step-up"
            disabled={hostStepUpBusy}
            onClick={onHostStepUp}
          >
            {t('work.approval.hostConfirm', locale)}
          </VNextButton>
        </div>
      )}

      {sourceDetail && <SourceDetailPanel sourceDetail={sourceDetail} collapsed={sourceDetail.kind === 'task'} />}

      {!closedItem && !blockedByGate && !privateDetailStillClosed && !detailAccessStillClosed && (
        <AuthorityPanel
          authorityBasis={authority?.authority_basis || item.authority_basis}
          allowedNow={allowedNow}
          visibleData={visibleData}
          lockedData={lockedData}
          receipt={authority?.expected_receipt || item.receipt_expectation || 'A receipt is required before Aegis claims completion.'}
          requiresMfa={Boolean(authority?.requires_mfa)}
          requiresApproval={Boolean(authority?.requires_approval)}
          detailOpen={Boolean(sourceDetail)}
          visibleEntries={visibleEntries}
          auditLinks={authority?.audit_links ?? []}
        />
      )}

      {(serverSafeDetail && serverSafeDetail !== item.summary) && (
        <details className="vnext-mini-panel vnext-supporting-panel vnext-disclosure">
          <summary className="vnext-disclosure-summary">
            <span className="vnext-disclosure-copy">
              <strong>{t('work.sourceNote.summary', locale)}</strong>
              <em className="vnext-disclosure-hint">{t('work.sourceNote.title', locale)}</em>
            </span>
          </summary>
          <p>{serverSafeDetail}</p>
        </details>
      )}

      {(relatedWorkCount > 0 || history.length > 0) && (
        <HistoryPanel
          dueAt={item.due_at}
          actionCount={item.related_action_ids.length}
          receiptCount={item.related_receipt_ids.length}
          history={history}
        />
      )}
    </aside>
  );
}

function itemShouldAppearInNearbyWork(item: OwnerWorkItem): boolean {
  if (isClosedItem(item)) return false;
  if (itemNeedsDecision(item) || itemIsBlockedByGate(item) || itemNeedsPrivateTaskAccess(item)) return true;
  if (buildSupportedItemActions(item).length > 0) return true;
  const lane = laneForItem(item);
  return lane === 'now' || lane === 'decide';
}

function workScanText(item: OwnerWorkItem): string {
  const extended = item as OwnerWorkItem & { why_it_matters?: string; owner_visible_reason?: string };
  return [
    ownerVisibleWorkTitle(item),
    ownerVisibleWorkCue(item),
    ownerVisibleReasonCue(item),
    item.summary,
    extended.why_it_matters,
    extended.owner_visible_reason,
    item.next_safe_action,
    item.blocked_by,
    item.source_kind,
    item.source,
    item.status,
    item.kind,
    ...lineageNodes(item).flatMap((node) => [node.label, node.detail, node.id]),
  ].filter(Boolean).join(' ').toLowerCase();
}

function workScanPriorityScore(item: OwnerWorkItem): number {
  let score = 0;
  if (itemNeedsDecision(item)) score += 80;
  if (itemIsBlockedByGate(item)) score += 70;
  if (itemNeedsPrivateTaskAccess(item)) score += 55;
  if (laneForItem(item) === 'now') score += 42;
  if (laneForItem(item) === 'follow_up') score += 18;
  if (item.urgency?.toLowerCase() === 'high') score += 24;
  if (item.urgency?.toLowerCase() === 'normal') score += 8;
  if (isClosedItem(item)) score -= 90;
  if (!itemShouldAppearInNearbyWork(item)) score -= 18;
  return score;
}

function workScanDueScore(value?: string | null): number {
  // UTC-stable parse (incl. SQLite no-offset) so the scan sort order doesn't
  // shift for non-UTC owners — see parseWireMomentMs.
  const parsed = parseWireMomentMs(value);
  return parsed ?? Number.MAX_SAFE_INTEGER;
}

function workScanUpdatedScore(value?: string | null): number {
  const parsed = parseWireMomentMs(value);
  return parsed ?? 0;
}

function itemIsQuietTrackingOnly(item: OwnerWorkItem): boolean {
  return !isClosedItem(item)
    && laneForItem(item) === 'follow_up'
    && !itemNeedsDecision(item)
    && !itemIsBlockedByGate(item)
    && !itemNeedsPrivateTaskAccess(item)
    && buildSupportedItemActions(item).length === 0;
}

function ownerVisibleSignalCue(item: OwnerWorkItem, userTimezone: string | null | undefined, locale: Locale): string {
  const savedState = t('work.signal.savedWorkState', locale);
  const blockedBy = resolveWorkCodeText('work.reasonCode', item.blocked_by_code, locale)
    ?? item.blocked_by?.trim();
  if (blockedBy) return ownerVisibleItemTitle(blockedBy, savedState, 160);
  if (item.due_at) return tf('work.signal.savedTiming', locale, { time: ownerTimeLabel(item.due_at, userTimezone, locale) });
  if (item.related_receipt_ids.length > 0) return t('work.signal.hasReceipt', locale);
  const routeReason = item.route_reason?.trim();
  if (routeReason) return ownerVisibleItemTitle(routeReason, savedState, 160);
  return t('work.signal.fromSavedState', locale);
}

function WorkActionMessage({ message }: { message: string }) {
  const { locale } = useLocale();
  if (!message) return null;
  if (isActionSuccessMessage(message)) {
    return (
      <div className="vnext-work-action-receipt" role="status" aria-label={t('work.aria.actionReceipt', locale)}>
        <VNextIcon name="receipt" />
        <span>
          <em>{t('work.receipt.label', locale)}</em>
          <strong>{message}</strong>
        </span>
      </div>
    );
  }
  return <p className="vnext-action-message">{message}</p>;
}

function WorkScanControls({
  query,
  scope,
  sort,
  active,
  resultCount,
  totalCount,
  onQueryChange,
  onScopeChange,
  onSortChange,
  onClear,
}: {
  query: string;
  scope: WorkScanScope;
  sort: WorkScanSort;
  active: boolean;
  resultCount: number;
  totalCount: number;
  onQueryChange: (value: string) => void;
  onScopeChange: (value: WorkScanScope) => void;
  onSortChange: (value: WorkScanSort) => void;
  onClear: () => void;
}) {
  const { locale } = useLocale();
  // Non-default query/scope/sort force chrome+filters open so the owner sees what is active.
  const chromeForcedOpen = workScanChromeDefaultExpanded({ query, scope, sort });
  const filtersForcedOpen = workScanFiltersDefaultExpanded({ query, scope, sort });
  const [chromeOpen, setChromeOpen] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const chromeExpanded = chromeForcedOpen || chromeOpen;
  const filtersExpanded = filtersForcedOpen || filtersOpen;
  return (
    <section className={classNames('vnext-work-scan', active && 'is-active')} aria-label={t('work.aria.scanControls', locale)}>
      <details
        className="vnext-work-scan-chrome vnext-disclosure"
        open={chromeExpanded}
        onToggle={(event) => {
          if (chromeForcedOpen) {
            event.currentTarget.open = true;
            return;
          }
          setChromeOpen(event.currentTarget.open);
        }}
      >
        <summary className="vnext-disclosure-summary">
          <span className="vnext-work-scan-chrome-summary vnext-disclosure-copy">
            <strong>{t('work.scan.find', locale)}</strong>
            <em className="vnext-disclosure-hint">{active
              ? tf('work.scan.ratio', locale, { shown: String(resultCount), total: String(totalCount) })
              : t('work.scan.chromeHint', locale)}</em>
          </span>
        </summary>
        <div className="vnext-work-scan-chrome-body">
      <label className="vnext-work-scan-search">
        <span className="vnext-sr-only">{t('work.scan.find', locale)}</span>
        <input
          type="search"
          value={query}
          placeholder={t('work.scan.searchPlaceholder', locale)}
          onChange={(event) => onQueryChange(event.currentTarget.value)}
        />
      </label>
      <details
        className="vnext-work-scan-filters vnext-disclosure"
        open={filtersExpanded}
        onToggle={(event) => {
          // When filters are forced open by active query/scope/sort, keep them open.
          if (filtersForcedOpen) {
            event.currentTarget.open = true;
            return;
          }
          setFiltersOpen(event.currentTarget.open);
        }}
      >
        <summary className="vnext-disclosure-summary">{t('work.scan.filters', locale)}</summary>
        <div className="vnext-work-scan-filter-body">
          <VNextSegmented
            className="vnext-work-scan-group"
            mode="radio"
            size="sm"
            label={t('work.aria.range', locale)}
            ariaLabel={t('work.aria.range', locale)}
            value={scope}
            onChange={(value) => onScopeChange(value as WorkScanScope)}
            options={[
              { value: 'all', label: t('work.scan.scope.all', locale), ariaLabel: t('work.scan.scope.all', locale) },
              { value: 'needs_me', label: t('work.scan.scope.needsMe', locale), ariaLabel: t('work.scan.scope.needsMe', locale) },
              { value: 'quiet', label: t('work.scan.scope.quiet', locale), ariaLabel: t('work.scan.scope.quiet', locale) },
            ]}
          />
          <VNextSegmented
            className="vnext-work-scan-group"
            mode="radio"
            size="sm"
            label={t('work.aria.sort', locale)}
            ariaLabel={t('work.aria.sort', locale)}
            value={sort}
            onChange={(value) => onSortChange(value as WorkScanSort)}
            options={[
              { value: 'lane', label: t('work.scan.sort.lane', locale), ariaLabel: t('work.scan.sort.lane', locale) },
              { value: 'priority', label: t('work.scan.sort.priority', locale), ariaLabel: t('work.scan.sort.priority', locale) },
              { value: 'updated', label: t('work.scan.sort.updated', locale), ariaLabel: t('work.scan.sort.updated', locale) },
            ]}
          />
        </div>
      </details>
      <div className="vnext-work-scan-count" aria-live="polite">
        {active
          ? tf('work.scan.ratio', locale, { shown: String(resultCount), total: String(totalCount) })
          : tf('work.scan.total', locale, { count: String(totalCount) })}
      </div>
      {active && <VNextButton variant="ghost" size="sm" className="vnext-work-scan-clear" onClick={onClear}>{t('work.scan.clear', locale)}</VNextButton>}
        </div>
      </details>
    </section>
  );
}

function WorkScanResults({
  items,
  selected,
  onSelect,
}: {
  items: OwnerWorkItem[];
  selected: OwnerWorkItem | null;
  onSelect: (id: string) => void;
}) {
  const { locale } = useLocale();
  const headingLabel = selected ? t('work.scan.matchingOther', locale) : t('work.scan.matching', locale);
  return (
    <section className="vnext-panel vnext-work-scan-results" aria-label={selected ? t('work.aria.matchingOther', locale) : t('work.aria.matching', locale)}>
      <div className="vnext-panel-heading">
        <div>
          <p className="vnext-kicker">{headingLabel}</p>
          <h2>{items.length === 0 ? t('work.scan.noMatches', locale) : tf('work.scan.matchCount', locale, { count: String(items.length), word: t(items.length === 1 ? 'work.scan.match.one' : 'work.scan.match.other', locale) })}</h2>
        </div>
      </div>
      {items.length === 0 ? (
        <p className="vnext-muted">{t('work.scan.emptyHint', locale)}</p>
      ) : (
        <div className="vnext-work-scan-list">
          {items.map((item) => (
            <WorkItemRow
              key={item.id}
              item={item}
              active={false}
              compact={Boolean(selected)}
              onClick={() => onSelect(item.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function lineageNodes(item: OwnerWorkItem) {
  return Array.isArray(item.lineage) ? item.lineage : [];
}

function lineageHostedActionIds(item: OwnerWorkItem): Set<string> {
  const ids = new Set<string>();
  for (const node of lineageNodes(item)) {
    for (const actionId of node.action_ids ?? []) ids.add(actionId);
  }
  return ids;
}

function lineageNodeLabel(node: NonNullable<OwnerWorkItem['lineage']>[number], locale: Locale): string {
  const key = node.label_code ? `work.lineage.${node.label_code}` : '';
  return key && catalogHas(key) ? t(key, locale) : node.label;
}

function lineageLayerLabel(layer: string, locale: Locale): string {
  const key = `work.lineage.layer.${layer}`;
  return catalogHas(key) ? t(key, locale) : layer;
}

function WorkLineagePanel({
  item,
  actions,
  armedAction,
  armedActionItemId,
  executingAction,
  onAction,
  onOpenLineageMessage,
}: {
  item: OwnerWorkItem;
  actions: VNextItemAction[];
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
  onOpenLineageMessage?: (cite: LineageChatCite) => void;
}) {
  const { locale } = useLocale();
  const nodes = lineageNodes(item);
  if (nodes.length === 0) return null;
  return (
    <section className="vnext-source-evidence-list vnext-work-lineage" aria-label={t('work.lineage.heading', locale)}>
      <strong>{t('work.lineage.heading', locale)}</strong>
      {nodes.map((node) => {
        const hosted = actions.filter((action) => (node.action_ids ?? []).includes(action.id));
        const cite = onOpenLineageMessage ? lineageChatCite(node) : null;
        return (
          <section key={`${node.layer}:${node.id}`} className="vnext-source-evidence-row" data-lineage-layer={node.layer}>
            <div className="vnext-source-evidence-heading">
              <strong>{lineageLayerLabel(node.layer, locale)} · {lineageNodeLabel(node, locale)}</strong>
            </div>
            {cite ? (
              <button
                type="button"
                className="vnext-lineage-cite"
                data-lineage-cite={cite.ref}
                data-testid="aegis-lineage-cite"
                aria-label={tf('work.lineage.openMessage', locale, { ref: cite.token })}
                onClick={() => onOpenLineageMessage?.(cite)}
              >
                <span className="vnext-source-evidence-detail" data-i18n-raw="true">{node.detail}</span>
                <span className="vnext-lineage-cite-token">{cite.token}</span>
              </button>
            ) : (
              <div className="vnext-source-evidence-detail" data-i18n-raw="true">{node.detail}</div>
            )}
            {hosted.length > 0 && (
              <div className="vnext-action-row">
                {hosted.map((action) => (
                  <VNextButton
                    key={action.id}
                    variant={action.tone === 'destructive' ? 'destructive' : 'secondary'}
                    size="sm"
                    className={classNames(
                      armedActionItemId === item.id && executingAction === action.id && 'is-executing',
                    )}
                    leadingIcon={<VNextIcon name={itemActionIconName(action.id, action.tone)} />}
                    disabled={executingAction !== null}
                    aria-busy={armedActionItemId === item.id && executingAction === action.id}
                    onClick={() => onAction(item, action)}
                  >
                    {itemActionButtonLabel(action, armedActionItemId === item.id ? armedAction : null, executingAction, locale)}
                  </VNextButton>
                ))}
              </div>
            )}
          </section>
        );
      })}
    </section>
  );
}

function NearbyInlineDetail({
  item,
  assistantOpen,
  assistantText,
  assistantResult,
  assistantTurns,
  armedAction,
  armedActionItemId,
  executingAction,
  busy,
  onAction,
  onOpenAssistant,
  onAskTextChange,
  onAskText,
  onCloseAssistant,
  onOpenLineageMessage,
}: {
  item: OwnerWorkItem;
  assistantOpen: boolean;
  assistantText: string;
  assistantResult: ContextualAssistantResponse | null;
  assistantTurns: InlineAssistantTurn[];
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  busy: boolean;
  onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
  onOpenAssistant: (prefill?: string) => void;
  onAskTextChange: (text: string) => void;
  onAskText: (text: string) => void;
  onCloseAssistant: () => void;
  onOpenLineageMessage?: (cite: LineageChatCite) => void;
}) {
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const actions = buildSupportedItemActions(item, locale);
  const dockActions = actions.filter((action) => !lineageHostedActionIds(item).has(action.id));
  const reason = ownerVisibleReasonCue(item, locale);
  const nearbyTitle = ownerVisibleWorkTitle(item, t('work.title.thisItem', locale), 120, locale);
  const next = itemNeedsPrivateTaskAccess(item) || !isTaskLikeItem(item)
    ? safeNextMoveLabel(item, nearbyTitle, locale)
    : detailOpenNextMoveLabel(item, locale);
  const timing = item.due_at ? ownerTimeLabel(item.due_at, userTimezone, locale) : '';
  // The 信号 line degrades to "已保存时间：…" when the only signal is the due
  // time — which the row above (跟踪到 · …) and the 何时 row already show
  // (2026-09-14 desktop screenshot: the same timestamp three times).
  const rawSignal = ownerVisibleSignalCue(item, userTimezone, locale);
  const signal = timing && rawSignal.includes(timing) ? '' : rawSignal;
  const followUpControls = followUpQuickControls(item, locale);
  return (
    <section className="vnext-nearby-inline-detail" aria-label={t('work.aria.expandedItem', locale)}>
      <div className="vnext-nearby-inline-facts">
        {reason && (
          <span>
            <em>{t('work.intent.why', locale)}</em>
            <strong>{reason}</strong>
          </span>
        )}
        {signal && (
          <span>
            <em>{t('work.intent.signal', locale)}</em>
            <strong>{signal}</strong>
          </span>
        )}
        <span>
          <em>{t('work.intent.next', locale)}</em>
          <strong>{next}</strong>
        </span>
        {timing && (
          <span>
            <em>{t('work.intent.when', locale)}</em>
            <strong>{timing}</strong>
          </span>
        )}
      </div>
      <WorkLineagePanel
        item={item}
        actions={actions}
        armedAction={armedAction}
        armedActionItemId={armedActionItemId}
        executingAction={executingAction}
        onAction={onAction}
        onOpenLineageMessage={onOpenLineageMessage}
      />
      {dockActions.length > 0 && (
        <div className="vnext-item-receipt-actions vnext-nearby-actions" aria-label={t('work.aria.inlineActions', locale)}>
          <span>{itemActionGroupLabel(item, locale)}</span>
          <div className="vnext-action-row">
            {(() => {
              const [primaryAction, ...secondaryActions] = dockActions;
              return (
                <>
                  <VNextButton
                    key={primaryAction.id}
                    variant={primaryAction.tone === 'destructive' ? 'destructive' : 'primary'}
                    size="sm"
                    className={classNames(
                      armedActionItemId === item.id && executingAction === primaryAction.id && 'is-executing',
                    )}
                    leadingIcon={<VNextIcon name={itemActionIconName(primaryAction.id, primaryAction.tone)} />}
                    disabled={executingAction !== null}
                    aria-busy={armedActionItemId === item.id && executingAction === primaryAction.id}
                    onClick={() => onAction(item, primaryAction)}
                  >
                    {itemActionButtonLabel(primaryAction, armedActionItemId === item.id ? armedAction : null, executingAction, locale)}
                  </VNextButton>
                  {secondaryActions.length > 0 && (
                    <details className="vnext-work-more-actions vnext-disclosure" aria-label={t('work.aria.moreActions', locale)}>
                      <summary className="vnext-disclosure-summary">{t('work.moreActions', locale)}</summary>
                      <div className="vnext-action-row">
                        {secondaryActions.map((action) => (
                          <VNextButton
                            key={action.id}
                            variant={action.tone === 'destructive' ? 'destructive' : 'secondary'}
                            size="sm"
                            className={classNames(
                              armedActionItemId === item.id && executingAction === action.id && 'is-executing',
                            )}
                            leadingIcon={<VNextIcon name={itemActionIconName(action.id, action.tone)} />}
                            disabled={executingAction !== null}
                            aria-busy={armedActionItemId === item.id && executingAction === action.id}
                            onClick={() => onAction(item, action)}
                          >
                            {itemActionButtonLabel(action, armedActionItemId === item.id ? armedAction : null, executingAction, locale)}
                          </VNextButton>
                        ))}
                      </div>
                    </details>
                  )}
                </>
              );
            })()}
          </div>
        </div>
      )}
      {actions.length === 0 && itemIsQuietTrackingOnly(item) && (
        <p className="vnext-muted">{t('work.nearby.quietNoAction', locale)}</p>
      )}
      {followUpControls.length > 0 && (
        <FollowUpPromptStrip
          controls={followUpControls}
          item={item}
          armedAction={armedAction}
          armedActionItemId={armedActionItemId}
          executingAction={executingAction}
          onPrompt={(prompt) => onOpenAssistant(prompt)}
          onCloseAction={(action) => onAction(item, action)}
        />
      )}
      {!assistantOpen ? (
        <VNextButton
          variant="ghost"
          size="sm"
          className="vnext-work-ask-compact"
          leadingIcon={<VNextIcon name="spark" />}
          aria-label={t('work.aria.askExpandedItem', locale)}
          title={t('work.askAegis.usingExpandedItem', locale)}
          onClick={() => onOpenAssistant()}
        >
          {t('work.askAegis', locale)}
        </VNextButton>
      ) : (
        <InlineAssistantPanel
          text={assistantText}
          placeholder={t('work.askAegis.placeholderExpanded', locale)}
          result={assistantResult}
          turns={assistantTurns}
          suggestions={buildCommandSuggestions(item, 'item', locale)}
          busy={busy}
          onChange={onAskTextChange}
          onSend={onAskText}
          onClose={onCloseAssistant}
        />
      )}
    </section>
  );
}

function FollowUpPromptStrip({
  controls,
  item,
  armedAction,
  armedActionItemId,
  executingAction,
  onPrompt,
  onCloseAction,
}: {
  controls: FollowUpQuickControl[];
  item: OwnerWorkItem;
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  onPrompt: (prompt: string) => void;
  onCloseAction: (action: VNextItemAction) => void;
}) {
  const { locale } = useLocale();
  return (
    <div className="vnext-followup-strip" aria-label={t('work.aria.followUpChoices', locale)}>
      <span>{t('work.askAegis', locale)}</span>
      {controls.map((control) => {
        if (control.kind === 'close_action') {
          // Keep strip label "清理"; only swap to confirm/working when armed.
          const armedHere = armedActionItemId === item.id ? armedAction : null;
          const label = itemActionButtonLabel(
            { ...control.action, label: control.label },
            armedHere,
            executingAction,
            locale,
          );
          return (
            <VNextButton
              key={control.id}
              variant="secondary"
              size="sm"
              className={classNames(
                armedActionItemId === item.id && executingAction === control.action.id && 'is-executing',
              )}
              disabled={executingAction !== null}
              aria-busy={armedActionItemId === item.id && executingAction === control.action.id}
              onClick={() => onCloseAction(control.action)}
            >
              {label}
            </VNextButton>
          );
        }
        return (
          <VNextButton key={control.id} variant="secondary" size="sm" onClick={() => onPrompt(control.prompt)}>
            {control.label}
          </VNextButton>
        );
      })}
    </div>
  );
}

function detailItemMeta(item: OwnerWorkItem, locale: Locale): string {
  const kind = item.kind === 'task' || item.kind === 'reminder'
      ? t('work.meta.task', locale)
      : item.kind === 'action_draft'
        ? t('work.meta.draft', locale)
        : item.kind === 'receipt'
          ? t('work.meta.receipt', locale)
          : titleCaseLabel(item.kind);
  const closed = isClosedItem(item);
  const urgency = !closed && item.urgency && item.urgency.toLowerCase() !== 'normal'
    ? tf('work.meta.priority', locale, { level: urgencyLevelLabel(item.urgency, locale) })
    : '';
  const status = closed ? titleCaseLabel(item.status || '') : '';
  return [urgency, kind, status].filter(Boolean).join(' · ');
}

function HistoryPanel({
  dueAt,
  actionCount,
  receiptCount,
  history,
}: {
  dueAt?: string | null;
  actionCount: number;
  receiptCount: number;
  history: NonNullable<VNextItemProjection['history']>;
}) {
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const summaryParts = [
    dueAt ? tf('work.timeline.dueAt', locale, { time: ownerTimeLabel(dueAt, userTimezone, locale) }) : '',
    actionCount > 0 ? tf('work.count.actions', locale, { count: String(actionCount), word: t(actionCount === 1 ? 'work.action.one' : 'work.action.plural', locale) }) : '',
    receiptCount > 0 ? tf('work.count.receipts', locale, { count: String(receiptCount), word: t(receiptCount === 1 ? 'work.receiptWord.one' : 'work.receiptWord.other', locale) }) : '',
    history.length > 0 ? tf('work.count.updates', locale, { count: String(history.length), word: t(history.length === 1 ? 'work.update.one' : 'work.update.other', locale) }) : '',
  ].filter(Boolean);
  return (
    <details className="vnext-mini-panel vnext-supporting-panel vnext-disclosure">
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{t('work.timeline.summary', locale)}</strong>
          <em className="vnext-disclosure-hint">{summaryParts.join(' · ') || t('work.timeline.available', locale)}</em>
        </span>
      </summary>
      {dueAt && <p><strong>{t('work.timeline.due', locale)}</strong>: {ownerTimeLabel(dueAt, userTimezone, locale)}</p>}
      {actionCount > 0 && <p><strong>{t('work.timeline.actions', locale)}</strong>: {actionCount}</p>}
      {receiptCount > 0 && <p><strong>{t('work.timeline.receipts', locale)}</strong>: {receiptCount}</p>}
      {history.map((entry) => (
        <p key={entry.id}><strong>{historyKindLabel(entry.kind, locale)}</strong>: {entry.label}{entry.created_at ? ` · ${ownerTimeLabel(entry.created_at, userTimezone, locale)}` : ''}</p>
      ))}
    </details>
  );
}

/** History entry kinds are server tokens (receipt/action/update); show the owner's language. */
function historyKindLabel(kind: string, locale: Locale): string {
  const key = ({ receipt: 'detail.timeline.label.receipt', action: 'detail.timeline.label.action', update: 'detail.timeline.label.update' } as Record<string, string>)[kind.trim().toLowerCase()];
  return key ? t(key, locale) : titleCaseLabel(kind);
}

function isOpenDetailInstruction(text: string, locale: Locale): boolean {
  return text.trim() === t('work.nextCode.task_open_or_act', locale).trim();
}

function safeNextMoveLabel(item: OwnerWorkItem, visibleTitle: string, locale: Locale): string {
  if (itemNeedsPrivateTaskAccess(item) || visibleTitle.toLowerCase().includes('private task needs review')) {
    return t('work.nextMove.openDetails', locale);
  }
  if (isClosedItem(item)) {
    return t('work.nextMove.handled', locale);
  }
  const askOrOpen = t('work.nextMove.askOrOpen', locale);
  const rawNext = ownerVisibleNextSafeActionText(item, locale) || ownerVisiblePrimaryActionLabel(item, askOrOpen, 160, locale);
  return ownerVisibleItemTitle(rawNext, askOrOpen, 160);
}

function detailOpenNextMoveLabel(item: OwnerWorkItem, locale: Locale): string {
  if (isClosedItem(item)) return t('work.nextMove.handled', locale);
  if (isTaskLikeItem(item)) return t('work.nextMove.reviewTask', locale);
  return t('work.nextMove.reviewBelow', locale);
}

function itemActionGroupLabel(item: OwnerWorkItem, locale: Locale): string {
  if (itemHasApprovalActions(item)) return t('work.group.approveReject', locale);
  if (itemIsBlockedByGate(item) && isTaskLikeItem(item)) return t('work.group.pausedChoices', locale);
  if (itemNeedsDecision(item)) return t('work.group.decision', locale);
  if (laneForItem(item) === 'follow_up' || ['waiting', 'tracking'].includes(item.status?.trim().toLowerCase() ?? '')) return t('work.group.followUp', locale);
  if (isTaskLikeItem(item)) return t('work.group.task', locale);
  return t('work.group.actions', locale);
}

function itemHasApprovalActions(item: OwnerWorkItem): boolean {
  return item.secondary_actions.some((action) => (
    action.action_kind === 'approval.approve' || action.action_kind === 'approval.reject'
  ));
}

export function SourceDetailPanel({ sourceDetail, collapsed = false }: { sourceDetail: SourceDetailView; collapsed?: boolean }) {
  const { locale } = useLocale();
  if (collapsed) {
    const summaryKicker = sourceDetail.kind === 'task' ? t('work.source.taskDetail', locale) : t('work.source.sourceDetail', locale);
    const summaryTitle = sourceDetail.title;
    const summarySubtitle = sourceDetail.kind === 'task'
      ? taskSummarySubtitle(sourceDetail, locale)
      : sourceDetail.subtitle;
    return (
      <details className="vnext-mini-panel vnext-source-panel vnext-source-panel-collapsed vnext-disclosure">
        <summary className="vnext-disclosure-summary">
          <span className="vnext-disclosure-copy">
            <strong>{summaryKicker}</strong>
            <em className="vnext-disclosure-hint">{summaryTitle}{summarySubtitle ? ` · ${summarySubtitle}` : ''}</em>
          </span>
        </summary>
        <ExternalSourceLink sourceDetail={sourceDetail} />
        <SourceDetailContent sourceDetail={sourceDetail} />
      </details>
    );
  }

  const title = sourceDetail.title;
  return (
    <section className="vnext-mini-panel vnext-source-panel">
      <h3>{title}</h3>
      <p>{sourceDetail.subtitle}</p>
      <ExternalSourceLink sourceDetail={sourceDetail} />
      <SourceDetailContent sourceDetail={sourceDetail} />
    </section>
  );
}

function SourceDetailContent({ sourceDetail }: { sourceDetail: SourceDetailView }) {
  const { locale } = useLocale();
  if (sourceDetail.kind === 'task') return <TaskDetailContent sourceDetail={sourceDetail} />;
  return (
    <>
      <SourceMetadataContent sourceDetail={sourceDetail} />
      {sourceDetail.body && (
        sourceDetail.kind === 'action_draft' ? (
          <>
            <h3>{sourceDetail.bodyLabel || t('work.source.draft', locale)}</h3>
            <RichText content={sourceDetail.body} className="vnext-rich-text-draft" />
          </>
        ) : (
          <>
            <h3>{sourceDetail.bodyLabel || t('work.source.detail', locale)}</h3>
            <RichText content={sourceDetail.body} className="vnext-rich-text-detail" />
          </>
        )
      )}
    </>
  );
}

const TASK_PRIMARY_FIELD_ORDER = ['state', 'due', 'mode', 'priority'];

function normalizedFieldLabel(value: string): string {
  return value.trim().toLowerCase();
}

// Behavior keys on the stable DetailField.id (set by buildTaskDetail) — the
// label is localized display text and must never be compared. The label
// fallback only covers legacy/hand-built fixtures without ids.
function detailFieldKey(field: DetailField): string {
  return field.id ?? normalizedFieldLabel(field.label);
}

function taskPrimaryFields(sourceDetail: SourceDetailView): DetailField[] {
  return TASK_PRIMARY_FIELD_ORDER.flatMap((key) => {
    const field = sourceDetail.fields.find((candidate) => detailFieldKey(candidate) === key);
    return field ? [field] : [];
  });
}

function taskSecondaryFields(sourceDetail: SourceDetailView): DetailField[] {
  const primaryKeys = new Set(TASK_PRIMARY_FIELD_ORDER);
  return sourceDetail.fields.filter((field) => !primaryKeys.has(detailFieldKey(field)));
}

function taskSummarySubtitle(sourceDetail: SourceDetailView, locale: Locale): string {
  const dueField = sourceDetail.fields.find((field) => detailFieldKey(field) === 'due');
  // A placeholder due ("Not set" / "未设置") is suppressed via the structured
  // flag — never by matching the localized value text.
  const due = dueField && !dueField.placeholder ? dueField.value : '';
  const state = sourceDetail.fields.find((field) => detailFieldKey(field) === 'state')?.value;
  return [state, due].filter(Boolean).join(' · ') || sourceDetail.subtitle || t('work.source.taskDetailsHistory', locale);
}

function TaskDetailContent({ sourceDetail }: { sourceDetail: SourceDetailView }) {
  const { locale } = useLocale();
  const primaryFields = taskPrimaryFields(sourceDetail);
  const secondaryFields = taskSecondaryFields(sourceDetail);
  const hasHistory = secondaryFields.length > 0
    || Boolean(sourceDetail.attachments?.length)
    || sourceDetail.timeline.length > 0
    || Boolean(sourceDetail.warning);
  return (
    <>
      <section className="vnext-task-summary-card" aria-label={t('work.aria.taskSummary', locale)}>
        {primaryFields.length > 0 && (
          <div className="vnext-task-summary-grid">
            {primaryFields.map((field) => (
              <span key={`${field.label}:${field.value}`}>
                <strong>{field.label}</strong>
                <em>{field.value}</em>
              </span>
            ))}
          </div>
        )}
        {sourceDetail.subtitle && <p>{sourceDetail.subtitle}</p>}
        {sourceDetail.body && (
          <>
            <h3>{sourceDetail.bodyLabel || t('work.source.note', locale)}</h3>
            <RichText content={sourceDetail.body} className="vnext-rich-text-detail" />
          </>
        )}
      </section>
      {hasHistory && (
        <details className="vnext-inline-disclosure vnext-task-history vnext-disclosure">
          <summary className="vnext-disclosure-summary">{t('work.source.taskHistory', locale)}</summary>
          <SourceMetadataContent sourceDetail={sourceDetail} fields={secondaryFields} />
        </details>
      )}
    </>
  );
}

function SourceMetadataContent({
  sourceDetail,
  fields = sourceDetail.fields,
}: {
  sourceDetail: SourceDetailView;
  fields?: Array<{ label: string; value: string }>;
}) {
  const { locale } = useLocale();
  return (
    <>
      {fields.length > 0 && (
        <div className="vnext-detail-fields">
          {fields.map((field) => (
            <span key={`${field.label}:${field.value}`}>
              <strong>{field.label}</strong>
              <em>{field.value}</em>
            </span>
          ))}
        </div>
      )}
      {sourceDetail.attachments?.length ? (
        <div className="vnext-attachment-list" aria-label={t('work.aria.attachmentMetadata', locale)}>
          <h3>{t('work.source.attachments', locale)}</h3>
          {sourceDetail.attachments.map((attachment, index) => (
            <AttachmentRow key={`${attachment.filename}:${attachment.meta}:${index}`} attachment={attachment} />
          ))}
        </div>
      ) : null}
      {sourceDetail.timeline.length > 0 && (
        <div className="vnext-timeline">
          {sourceDetail.timeline.map((event) => (
            <p key={`${event.label}:${event.at || event.status}`}>
              <strong>{event.label}</strong> · {event.status}{event.at ? ` · ${event.at}` : ''}
            </p>
          ))}
        </div>
      )}
      {sourceDetail.warning && <p className="vnext-warning">{sourceDetail.warning}</p>}
    </>
  );
}

function AttachmentRow({ attachment }: { attachment: DetailAttachment }) {
  const { locale } = useLocale();
  const [previewOpen, setPreviewOpen] = useState(false);
  const url = attachment.downloadUrl;
  const isPdf = attachment.mimeType.split(';')[0].trim().toLowerCase() === 'application/pdf';
  return (
    <div className="vnext-attachment-row">
      <span className="vnext-attachment-meta">
        <strong>{attachment.filename}</strong>
        <em>{attachment.meta}</em>
      </span>
      {url ? (
        <span className="vnext-attachment-actions">
          {attachment.previewable && (
            <button type="button" className="vnext-attachment-preview-toggle" onClick={() => setPreviewOpen((v) => !v)}>
              {previewOpen ? t('work.source.attachmentHidePreview', locale) : t('work.source.attachmentPreview', locale)}
            </button>
          )}
          {/* download attr names the saved file; the server serves attachment
              disposition for non-previewable types so a click downloads. */}
          <a className="vnext-attachment-download" href={url} download={attachment.filename} target="_blank" rel="noopener noreferrer">
            {t('work.source.attachmentDownload', locale)}
          </a>
        </span>
      ) : null}
      {url && attachment.previewable && previewOpen ? (
        <div className="vnext-attachment-preview">
          {isPdf ? (
            <iframe title={attachment.filename} src={url} className="vnext-attachment-preview-pdf" />
          ) : (
            <img alt={attachment.filename} src={url} className="vnext-attachment-preview-img" />
          )}
        </div>
      ) : null}
    </div>
  );
}

function ExternalSourceLink({ sourceDetail }: { sourceDetail: SourceDetailView }) {
  const { locale } = useLocale();
  if (!sourceDetail.externalUrl) return null;
  return (
    <div className="vnext-action-row vnext-source-actions">
      <a href={sourceDetail.externalUrl} target="_blank" rel="noopener noreferrer">{externalSourceLabel(sourceDetail, locale)}</a>
    </div>
  );
}

function externalSourceLabel(sourceDetail: SourceDetailView, locale: Locale): string {
  if (sourceDetail.kind === 'task') return t('work.source.openTaskSource', locale);
  if (sourceDetail.kind === 'receipt') return t('work.source.openReceiptSource', locale);
  return t('work.source.openSource', locale);
}

function blockedByGateSummary(item: OwnerWorkItem, locale: Locale): string {
  return blockedAutomationWhatHappened(item, locale);
}

function BlockedByGatePanel({
  item,
  assistantOpen,
  assistantText,
  assistantResult,
  assistantTurns,
  onDecisionPrompt,
  itemActions,
  onAction,
  onAskTextChange,
  onAskText,
  onCloseAssistant,
  onRetryAssistant,
  onRetryAssistantLabel,
  onOpen,
  visibleData,
  lockedData,
  armedAction,
  armedActionItemId,
  executingAction,
  busy,
  sourceDetailBusy,
}: {
  item: OwnerWorkItem;
  assistantOpen: boolean;
  assistantText: string;
  assistantResult: ContextualAssistantResponse | null;
  assistantTurns: InlineAssistantTurn[];
  onDecisionPrompt: (prompt: string, label: string) => void;
  itemActions: VNextItemAction[];
  onAction: (item: OwnerWorkItem, action: VNextItemAction) => void;
  onAskTextChange: (text: string) => void;
  onAskText: (text: string, conversationContext?: VNextConversationContextTurn[]) => void;
  onCloseAssistant: () => void;
  onRetryAssistant?: () => void;
  onRetryAssistantLabel?: string;
  onOpen: () => void;
  visibleData: string[];
  lockedData: string[];
  armedAction: VNextItemActionId | null;
  armedActionItemId: string | null;
  executingAction: VNextItemActionId | null;
  busy: boolean;
  sourceDetailBusy: boolean;
}) {
  const { locale } = useLocale();
  const decision = localizeDecisionRequest(
    item.decision_request ?? fallbackBlockedDecisionRequest(item, locale),
    locale,
  );
  const trustChips = blockedTrustChips(visibleData, lockedData, itemActions.length > 0, locale);
  const itemActionById = new Map(itemActions.map((action) => [action.id, action]));
  // First enabled choice with a resolvable action_kind gets primary visual weight;
  // if none, first enabled non-detail choice. detail_route stays lightest.
  // Click handlers are unchanged — weight is presentation only.
  const primaryChoiceId = decision.choices.find((choice) => {
    if (!choice.enabled || !choice.action_kind) return false;
    return itemActionById.has(choice.action_kind as VNextItemActionId);
  })?.id
    ?? decision.choices.find((choice) => choice.enabled && !choice.detail_route)?.id
    ?? null;
  return (
    <section
      className="vnext-mini-panel vnext-blocked-panel vnext-ceo-review-card"
      aria-label={t('work.aria.pausedDecision', locale)}
    >
      <div className="vnext-mini-heading-row">
        <h3>{t('work.blocked.ceoTitle', locale)}</h3>
        <span>{t('work.blocked.noActionRan', locale)}</span>
      </div>
      {decision.title && decision.title !== t('work.blocked.ceoTitle', locale) && (
        <p className="vnext-ceo-review-subtitle">{decision.title}</p>
      )}
      {/* CEO Review shape: What / Why — capability jargon demoted. */}
      <dl className="vnext-ceo-review-facts">
        <div>
          <dt>{t('work.blocked.whatLabel', locale)}</dt>
          <dd>{decision.requested_capability || decision.what_aegis_was_doing}</dd>
        </div>
        <div>
          <dt>{t('work.blocked.whyLabel', locale)}</dt>
          <dd>{decision.blocked_reason}</dd>
        </div>
        {decision.what_aegis_was_doing
          && decision.requested_capability
          && decision.what_aegis_was_doing !== decision.requested_capability && (
          <div>
            <dt>{t('work.blocked.contextLabel', locale)}</dt>
            <dd>{decision.what_aegis_was_doing}</dd>
          </div>
        )}
      </dl>
      {decision.scope && <p className="vnext-decision-scope">{decision.scope}</p>}
      <div className="vnext-blocked-trust" aria-label={t('work.aria.pausedTrust', locale)}>
        {trustChips.map((chip) => (
          <span key={chip.label}>
            <VNextIcon name={chip.icon} />
            <strong>{chip.label}</strong>
          </span>
        ))}
      </div>
      <div className="vnext-decision-choice-list" aria-label={t('work.aria.decisionChoices', locale)}>
        {decision.choices.map((choice) => {
          const copy = resolveDecisionChoiceCopy(choice, locale);
          const action = choice.action_kind ? itemActionById.get(choice.action_kind as VNextItemActionId) : undefined;
          const actionUnavailable = Boolean(choice.action_kind && !action);
          const disabled = !choice.enabled
            || actionUnavailable
            || sourceDetailBusy && Boolean(choice.detail_route)
            || executingAction !== null && Boolean(action);
          // Freeform server reasons stay verbatim behind a localized marker; the
          // catalog choiceUnavailable fallback is already a full localized
          // sentence, so it renders clean (no prefix).
          const disabledReason = choice.disabled_reason
            ? `${t('work.decide.unavailable', locale)}${choice.disabled_reason}`
            : (actionUnavailable ? t('work.blocked.choiceUnavailable', locale) : '');
          const itemScopedArmedAction = armedActionItemId === item.id ? armedAction : null;
          const choiceLabel = action && (itemScopedArmedAction === action.id || executingAction === action.id)
            ? itemActionButtonLabel(action, itemScopedArmedAction, executingAction, locale)
            : copy.label;
          const isPrimary = choice.id === primaryChoiceId && choice.tone !== 'destructive';
          // Server action choices carry BOTH action_kind and detail_route; only
          // choices that actually navigate on click (no resolvable action) get
          // the lightest "detail" weight — matches the onClick branch order.
          const isDetail = Boolean(choice.detail_route) && !action;
          return (
            <button
              key={choice.id}
              type="button"
              className={classNames(
                'vnext-decision-choice',
                isPrimary && 'is-primary',
                !isPrimary && !isDetail && 'is-secondary',
                isDetail && 'is-detail',
                choice.tone === 'destructive' && 'danger',
                (!choice.enabled || actionUnavailable) && 'disabled',
              )}
              disabled={disabled}
              onClick={() => {
                if (!choice.enabled || actionUnavailable) return;
                if (action) {
                  onAction(item, action);
                  return;
                }
                if (choice.detail_route) {
                  onOpen();
                  return;
                }
                if (choice.assistant_prompt) {
                  onDecisionPrompt(choice.assistant_prompt, copy.label);
                }
              }}
            >
              <span className="vnext-decision-choice-main">
                <strong>
                  {choiceLabel}
                  <span className="vnext-decision-choice-meta">
                    {copy.scopeLabel} · {copy.riskLabel}
                  </span>
                </strong>
                <em>{copy.consequence}</em>
              </span>
              {disabledReason && <span className="vnext-decision-choice-disabled">{disabledReason}</span>}
            </button>
          );
        })}
      </div>
      {decision.safe_details.length > 0 && (
        <details className="vnext-inline-disclosure vnext-decision-safe-details vnext-disclosure">
          <summary className="vnext-disclosure-summary">{t('work.blocked.safeDetails', locale)}</summary>
          <ul>
            {decision.safe_details.map((detail) => <li key={detail}>{detail}</li>)}
          </ul>
        </details>
      )}
      {assistantOpen && (
        <InlineAssistantPanel
          text={assistantText}
          placeholder={t('work.askAegis.placeholderPaused', locale)}
          result={assistantResult}
          turns={assistantTurns}
          suggestions={[
            {
              id: 'blocked.explain',
              label: t('work.blocked.suggest.whyPaused.label', locale),
              prompt: t('work.blocked.suggest.whyPaused.prompt', locale),
            },
            {
              id: 'blocked.revise',
              label: t('work.blocked.suggest.revise.label', locale),
              prompt: t('work.blocked.suggest.revise.prompt', locale),
            },
            {
              id: 'blocked.close',
              label: t('work.blocked.suggest.canClose.label', locale),
              prompt: t('work.blocked.suggest.canClose.prompt', locale),
            },
          ]}
          busy={busy}
          onChange={onAskTextChange}
          onSend={onAskText}
          onClose={onCloseAssistant}
          onRetry={onRetryAssistant}
          retryLabel={onRetryAssistantLabel}
        />
      )}
    </section>
  );
}

/**
 * Localize a SERVER-built decision_request under non-en locales only.
 *
 * en output stays byte-identical (early return, no overlay). Under other
 * locales:
 *   - blocked_reason ← decision.blocked_reason_code (work.reasonCode.*)
 *   - title / what / capability / scope ← decision-level codes (work.decide.*;
 *     capability/safe_detail codes may use `namespace:code` form)
 *   - safe_details element-wise via safe_detail_codes ("" = freeform keep;
 *     length mismatch keeps server strings)
 * Unknown/missing codes always fall back to the server English string.
 */
export function localizeDecisionRequest(
  decision: VNextDecisionRequest,
  locale: Locale,
): VNextDecisionRequest {
  if (locale === 'en') return decision;

  const title = resolveWorkCodeText('work.decide', decision.title_code, locale)
    ?? decision.title;
  const what_aegis_was_doing = resolveWorkCodeText('work.decide', decision.what_code, locale)
    ?? decision.what_aegis_was_doing;
  const requested_capability = resolveWorkCodeText('work.decide', decision.capability_code, locale)
    ?? decision.requested_capability;
  const scope = resolveWorkCodeText('work.decide', decision.scope_code, locale)
    ?? decision.scope;
  const blocked_reason = resolveWorkCodeText('work.reasonCode', decision.blocked_reason_code, locale)
    ?? decision.blocked_reason;

  let safe_details = decision.safe_details;
  const codes = decision.safe_detail_codes;
  if (Array.isArray(codes) && codes.length === decision.safe_details.length) {
    safe_details = decision.safe_details.map((server, index) => {
      const code = codes[index];
      if (typeof code !== 'string' || !code.trim()) return server;
      return resolveWorkCodeText('work.decide', code, locale) ?? server;
    });
  }

  return {
    ...decision,
    title,
    what_aegis_was_doing,
    blocked_reason,
    requested_capability,
    scope,
    safe_details,
  };
}

function fallbackBlockedDecisionRequest(item: OwnerWorkItem, locale: Locale): VNextDecisionRequest {
  // Mirror server precedence: blocked_by_code when blocked_by is non-empty, else
  // owner_visible_reason_code. Keeps code and chosen string family aligned.
  const blockedByTrimmed = typeof item.blocked_by === 'string' ? item.blocked_by.trim() : '';
  const blocked_reason_code = blockedByTrimmed
    ? (item.blocked_by_code ?? null)
    : (item.owner_visible_reason_code ?? null);

  // en: historical precedence (blocked_by first) — byte-identical to today.
  // non-en: localize the code that belongs to the string this card actually
  // shows; if that code is unknown, render the server sentence VERBATIM. An
  // earlier revision fell back to the OTHER field's localized text to avoid a
  // mixed-locale card — that renders a different sentence than the one the
  // server chose, which is the exact defect this slice removes.
  let blocked_reason: string;
  if (locale === 'en') {
    blocked_reason = (blockedByTrimmed
      || resolveWorkCodeText('work.reasonCode', item.owner_visible_reason_code, locale)
      || item.owner_visible_reason)
      || t('work.fallback.blockedReason', locale);
  } else {
    blocked_reason = resolveWorkCodeText('work.reasonCode', blocked_reason_code, locale)
      || blockedByTrimmed
      || item.owner_visible_reason
      || t('work.fallback.blockedReason', locale);
  }

  return {
    id: `decision:${item.id}`,
    title: t('work.fallback.title', locale),
    what_aegis_was_doing: blockedAutomationWhatHappened(item, locale),
    blocked_reason,
    blocked_reason_code,
    requested_capability: ownerVisibleNextSafeActionText(item, locale) || t('work.fallback.requestedCapability', locale),
    scope: t('work.fallback.scope', locale),
    choices: [
      {
        id: 'explain',
        label: t('work.fallback.choice.label', locale),
        consequence: t('work.fallback.choice.consequence', locale),
        risk_level: 'low',
        applies_to: 'this_item',
        assistant_prompt: t('work.fallback.choice.assistantPrompt', locale),
        requires_confirmation: false,
        enabled: true,
        tone: 'normal',
      },
    ],
    safe_details: [t('work.fallback.safeDetail', locale)],
    audit_refs: [],
    // English sentinel: consumed by ownerFacingReceiptText regex mapping; not displayed verbatim.
    receipt_expectation: item.receipt_expectation || 'A receipt is required before Aegis claims completion.',
  };
}

function blockedTrustChips(visibleData: string[], lockedData: string[], hasReceiptActions: boolean, locale: Locale): Array<{ icon: VNextIconName; label: string }> {
  const hasVisibleData = visibleData.length > 0;
  return [
    {
      icon: 'shield',
      label: hasVisibleData ? t('work.trust.visibleMetadata', locale) : t('work.trust.safeSummaryOnly', locale),
    },
    {
      icon: 'close',
      label: lockedData.length > 0 ? t('work.trust.privateLocked', locale) : t('work.trust.privateUnopened', locale),
    },
    {
      icon: 'receipt',
      label: hasReceiptActions ? t('work.trust.receiptsRequired', locale) : t('work.trust.noActionRan', locale),
    },
  ];
}

function AuthorityPanel({
  authorityBasis,
  allowedNow,
  visibleData,
  lockedData,
  receipt,
  requiresMfa,
  requiresApproval,
  detailOpen,
  visibleEntries,
  auditLinks,
}: {
  authorityBasis: string | null | undefined;
  allowedNow: string[];
  visibleData: string[];
  lockedData: string[];
  receipt: string;
  requiresMfa: boolean;
  requiresApproval: boolean;
  detailOpen: boolean;
  visibleEntries: Array<[string, unknown]>;
  auditLinks: string[];
}) {
  const { locale } = useLocale();
  const lockedLabel = lockedData.length > 0
    ? tf('work.authority.lockedCount', locale, { count: String(lockedData.length) })
    : t('work.authority.noExtraLock', locale);
  const allowedLabel = compactOwnerList(allowedNow, t('work.authority.askFirst', locale), 3, locale);
  const visibleLabel = compactOwnerList(visibleData, t('work.authority.visibleBasics', locale), 3, locale);
  const hasLockedContent = requiresMfa || lockedData.length > 0;
  const statusLabel = detailOpen ? t('work.authority.open', locale) : hasLockedContent ? t('work.authority.locked', locale) : t('work.authority.visible', locale);
  const safetyLine = detailOpen
    ? t('work.authority.safety.open', locale)
    : hasLockedContent
    ? t('work.authority.safety.locked', locale)
    : requiresApproval
    ? t('work.authority.safety.approval', locale)
    : t('work.authority.safety.default', locale);
  const receiptLine = ownerFacingReceiptText(receipt, locale);
  return (
    <details className="vnext-mini-panel vnext-authority-panel vnext-authority-panel-collapsed vnext-disclosure">
      <summary className="vnext-disclosure-summary">
        <span className="vnext-disclosure-copy">
          <strong>{t('work.authority.access', locale)}</strong>
          <em className="vnext-disclosure-hint">{statusLabel} · {safetyLine}</em>
        </span>
      </summary>
      <div className="vnext-trust-summary">
        <span>
          <strong>{t('work.authority.allowed', locale)}</strong>
          {allowedLabel}
        </span>
        <span>
          <strong>{t('work.authority.visible', locale)}</strong>
          {visibleLabel}
        </span>
        <span>
          <strong>{t('work.authority.locked', locale)}</strong>
          {lockedLabel}
        </span>
      </div>
      <p className="vnext-receipt-line"><strong>{t('work.authority.why', locale)}</strong>: {authorityBasisLabel(authorityBasis, locale)}</p>
      <p className="vnext-receipt-line"><strong>{t('work.receipt.label', locale)}</strong>: {receiptLine}</p>
      {(visibleEntries.length > 0 || auditLinks.length > 0) && (
        <details className="vnext-inline-disclosure vnext-disclosure">
          <summary className="vnext-disclosure-summary">{t('work.authority.contextUsed', locale)}</summary>
          {visibleEntries.length > 0 && (
            <div className="vnext-detail-fields">
              {visibleEntries.map(([key, value]) => {
                const label = ownerFacingFieldLabel(key, locale);
                const shown = ownerFacingFieldValue(String(value), locale);
                if (!label || !shown) return null;
                return (
                  <span key={key}>
                    <strong>{label}</strong>
                    <em>{shown}</em>
                  </span>
                );
              })}
            </div>
          )}
          {auditLinks.length > 0 ? <p className="vnext-muted">{tf('work.authority.record', locale, { links: auditLinks.join(', ') })}</p> : null}
        </details>
      )}
    </details>
  );
}

function ownerFacingFieldValue(value: string, locale: Locale): string {
  const raw = value.trim();
  if (!raw) return '';
  if (locale !== 'zh') return raw;
  const known: Record<string, string> = {
    unread: '未读',
    read: '已读',
    pending: '待处理',
    active: '进行中',
    done: '已完成',
    owner_only: '仅自己',
  };
  if (known[raw.toLowerCase()]) return known[raw.toLowerCase()];
  if (/^[a-z][a-z0-9_]*$/.test(raw) || /^s\d$/i.test(raw)) return '';
  return raw;
}

function ownerFacingReceiptText(value: string, locale: Locale): string {
  const trimmed = value.trim();
  if (!trimmed) return t('work.receiptText.default', locale);
  if (/provider receipt/i.test(trimmed)) return t('work.receiptText.provider', locale);
  if (/task mutations are recorded with an action ledger receipt or a failure state/i.test(trimmed)) {
    return t('work.receiptText.task', locale);
  }
  if (/no external action can be claimed complete unless a receipt is recorded/i.test(trimmed)) {
    return t('work.receiptText.externalComplete', locale);
  }
  if (/a receipt is required before aegis claims completion/i.test(trimmed)) {
    return t('work.receiptText.complete', locale);
  }
  if (locale === 'zh' && !/[\u4e00-\u9fff]/.test(trimmed)) {
    return t('work.receiptText.default', locale);
  }
  return trimmed.replace(/\bAction Ledger receipt\b/g, 'Aegis receipt');
}

function workQueueHeadline(lanes: LaneProjection[], locale: Locale): string {
  const decisionLane = lanes.find((lane) => lane.id === 'decide');
  if (decisionLane?.items.length) {
    return decisionLane.items.length === 1
      ? t('work.queue.oneDecision', locale)
      : tf('work.queue.decisionsNeedYou', locale, { count: String(decisionLane.items.length) });
  }
  const nowLane = lanes.find((lane) => lane.id === 'now');
  if (nowLane?.items.length) {
    return tf('work.queue.readyToHandle', locale, {
      count: String(nowLane.items.length),
      itemWord: t(nowLane.items.length === 1 ? 'work.item.one' : 'work.item.other', locale),
    });
  }
  const followUpLane = lanes.find((lane) => lane.id === 'follow_up');
  if (followUpLane?.items.length) {
    return tf('work.queue.loopsTracked', locale, {
      count: String(followUpLane.items.length),
      loopWord: t(followUpLane.items.length === 1 ? 'work.loop.one' : 'work.loop.other', locale),
    });
  }
  return t('work.queue.organizedQuietly', locale);
}

function workQueueSummary(
  lanes: LaneProjection[],
  selected: OwnerWorkItem | null,
  actionableNearbyCount = 0,
  quietNearbyCount = 0,
  locale: Locale = 'zh',
): { headline: string; body: string } {
  const total = lanes.reduce((count, lane) => count + lane.items.length, 0);
  if (!selected) {
    return {
      headline: workQueueHeadline(lanes, locale),
      body: tf('work.queue.bodyDefault', locale, {
        count: String(total),
        itemWord: t(total === 1 ? 'work.item.one' : 'work.item.other', locale),
      }),
    };
  }

  if (actionableNearbyCount === 0) {
    if (quietNearbyCount > 0) {
      return {
        headline: tf('work.queue.quietHeadline', locale, {
          count: String(quietNearbyCount),
          loopWord: t(quietNearbyCount === 1 ? 'work.loop.one' : 'work.loop.other', locale),
        }),
        body: t('work.queue.quietBody', locale),
      };
    }
    return {
      headline: t('work.queue.onlyActive', locale),
      body: t('work.queue.onlyActiveBody', locale),
    };
  }

  return {
    headline: tf('work.queue.nearbyHeadline', locale, {
      count: String(actionableNearbyCount),
      itemWord: t(actionableNearbyCount === 1 ? 'work.nearbyItem.one' : 'work.nearbyItem.other', locale),
    }),
    body: quietNearbyCount > 0
      ? tf('work.queue.nearbyBodyQuiet', locale, {
        count: String(quietNearbyCount),
        loopWord: t(quietNearbyCount === 1 ? 'work.loop.one' : 'work.loop.other', locale),
      })
      : t('work.queue.nearbyBodyDefault', locale),
  };
}

function workQueueKicker(
  selected: OwnerWorkItem | null,
  actionableNearbyCount: number,
  quietNearbyCount: number,
  locale: Locale,
): string {
  if (!selected) return t('work.queue.overviewKicker', locale);
  if (actionableNearbyCount === 0 && quietNearbyCount > 0) return t('work.queue.watchingKicker', locale);
  return t('work.queue.moreWorkKicker', locale);
}

function laneCue(lane: LaneProjection, locale: Locale): string {
  if (lane.items.length === 0) return t('work.laneCue.quiet', locale);
  if (lane.id === 'decide') return t('work.laneCue.needsYou', locale);
  if (lane.id === 'now') return t('work.laneCue.readyNow', locale);
  if (lane.id === 'follow_up') return t('work.laneCue.tracking', locale);
  if (lane.id === 'backlog') return t('work.laneCue.later', locale);
  return t('work.laneCue.done', locale);
}

function workLaneAriaLabel(laneId: string, locale: Locale): string {
  switch (laneId) {
    case 'now':
      return t('work.aria.laneNow', locale);
    case 'decide':
      return t('work.aria.laneDecide', locale);
    case 'follow_up':
      return t('work.aria.laneFollowUp', locale);
    case 'backlog':
      return t('work.aria.laneBacklog', locale);
    case 'done':
      return t('work.aria.laneDone', locale);
    default:
      return t('work.aria.laneToggle', locale);
  }
}

function stateCueLabel(item: OwnerWorkItem, needsPrivateDetail: boolean, locale: Locale): string {
  if (needsPrivateDetail) return t('work.stateCue.openDetailFirst', locale);
  if (itemNeedsDecision(item)) return t('work.stateCue.waitingForYou', locale);
  if (laneForItem(item) === 'follow_up') return t('work.stateCue.tracking', locale);
  if (item.status?.toLowerCase() === 'scheduled') return t('work.stateCue.scheduled', locale);
  return t('work.stateCue.next', locale);
}
