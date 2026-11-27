import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import {
  fetchOwnerWorkContext,
  fetchSession,
  fetchVNextDebugLeases,
  fetchVNextDebugJobs,
  fetchVNextAvatarProfile,
  fetchVNextCodexStatus,
  fetchVNextGrokStatus,
  fetchVNextSettings,
  fetchVNextModelCatalog,
  fetchVNextWebPushStatus,
  fetchVNextChatHistory,
  fetchVNextStatus,
  fetchVNextItem,
  generateVNextAegisAvatar,
  logout,
  markVNextPresence,
  pauseVNextCostSaving,
  pollVNextCodexDeviceCode,
  pollVNextGrokDeviceCode,
  resumeVNextCostSaving,
  setDisplayLanguage,
  setLlmProvider,
  setLlmModel,
  setFallbackModel,
  setWebSearchProvider,
  setWebSearchModel,
  setPaidApiFallback,
  revokeVNextDebugLease,
  revokeVNextGrok,
  startVNextGrokDeviceCode,
  startVNextCodexDeviceCode,
  setVNextCodexModel,
  stopUsingVNextGrokForChat,
  stopUsingVNextCodexForChat,
  subscribeVNextWebPush,
  submitVNextDebugSnapshot,
  testVNextWebPush,
  unsubscribeVNextWebPush,
  updateConsoleTaskAction,
  useVNextCodexForChat,
  useVNextGrokForChat,
  updateVNextAvatarAutoRefresh,
  updateVNextAvatarProfile,
  ApiResponseError,
  approveConsoleApproval,
  confirmLocalHostStepUp,
  mintVNextTurnStreamToken,
  rejectConsoleApproval,
  stopOwnerResearch,
  createVNextDebugLease,
  createVNextDebugJob,
  uploadVNextAttachment,
  fetchVNextVoiceStatus,
  mintVNextVoiceSession,
} from '../api';
import type { ContextualAssistantResponse, OwnerWorkContextEnvelope, OwnerWorkItem, OwnerWorkQueue, SessionInfo, VNextAvatarAutoRefreshProjection, VNextCodexDeviceCodeResponse, VNextCodexPollResponse, VNextCodexStatus, VNextDebugJob, VNextGrokDeviceCodeResponse, VNextGrokPollResponse, VNextGrokStatus, VNextItemProjection, VNextSettingsProjection, VNextStatusProjection } from '../api';
import type { VNextCommandAttachment, VNextConversationContextTurn } from '../api';
import type { VNextDebugLease } from '../api';
import type { VNextWebPushStatus } from '../api';
import { useTelegram } from '../useTelegram';
import { AegisAvatar, OwnerAvatar } from './Avatar';
import { avatarProfileFromServer, avatarProfileOwnerKey, avatarProfileToServer, defaultAvatarProfile, readAvatarProfile, writeAvatarProfile } from './avatarPrefs';
import type { AvatarProfile, AvatarRole } from './avatarPrefs';
import { buildWorkingAssistantResult, requestAssistantCommandWithAutoCheck, requestAssistantResultWithAutoCheck } from './assistantActions';
import { appendAssistantDelta, openTurnStream } from './turnStream';
import type { TurnEvent } from './turnStream';
import {
  executorFooterLabel,
  formatExecutorFooter,
  toolDonePhrase,
  workingPhraseForTool,
} from './toolPhrases';
import { appendProgressLine, composeWorkingText, workingPhraseForPhase, workingNotice } from './workingProgress';
import { ChatHomeView, appendApprovalMessage, replaceApprovalMessagesFromServer, setApprovalProgress, setApprovalState } from './ChatHomeView';
import { isGatePendingOwnerAck, isSearchHold, ownerVisibleSearchOutcome } from './searchApproval';
import type { ChatHomeApprovalState, ChatHomeAttachedContext, ChatHomeMessage, ChatHomeMessageAttachment, ChatHomePendingAttachment, ChatHomeQuote, ChatHomeVoiceNote, ChatHomeWorkDetailResult } from './ChatHomeView';
import {
  applyChatSheetScrollLock,
  findChatSheetPageScroller,
  releaseChatSheetScrollLock,
} from './chatSheetScrollLock';
import { fetchVNextBackedOwnerQueue } from './data';
import { assistantDisplayError, displayError, isLocalStaticPreviewSessionMiss, offerLocalHostStepUp } from './errors';
import { VNextIcon } from './icons';
import type { VNextIconName } from './icons';
import type { VNextHeaderMenuItem } from './VNextHeaderMenu';
import {
  currentOwnerInstallStatus,
  ownerInstallManualNotice,
  ownerInstallOutcomeNotice,
} from './installPrompt';
import type { OwnerInstallPromptEvent, OwnerInstallStatus } from './installPrompt';
import {
  itemActionConfirmationHint,
  shouldForceReleaseBusyOnSessionReset,
  shouldReleaseBusyOnRequestSettle,
} from './interactionState';
import { executeSelectedItemAction, openSelectedSourceDetail } from './itemActions';
import {
  buildSupportedItemActions,
  buildVNextHome,
  buildVNextHomeSignals,
  buildVNextLanes,
  isClosedItem,
  laneForItem,
  optimisticStatusForItemAction,
  mandateMatchWorkItemId,
  ownerVisibleCodedTitle,
  ownerVisibleNextSafeActionText,
  ownerVisibleWorkTitle,
} from './model';
import type { VNextItemAction, VNextItemActionId } from './model';
import { GENERAL_ASSISTANT_FOCUS_ID } from './model';
import { readVNextLocationState, writeVNextLocationState } from './navigation';
import type { DeliveryNoticeAction } from './notificationActions';
import { ownerTaskActionReceipt } from './taskActionReceipt';
import { clearKeyboardInset, syncKeyboardInset } from './keyboardInset';
import type { VNextViewId } from './navigation';
import { classNames, ownerPrimaryNavItemIds, ownerUtilityMenuItemIds } from './presentation';
import { OwnerShellChrome, type OwnerShellSummon } from './OwnerShellChrome';
import { buildSourceDetailView, taskDetailClosedStatus } from './detail';
import { buildVNextDebugSnapshotPayload } from './debugSnapshot';
import { chatHomeHistoryOwnerKey, chatHomeMessagesFromServerHistory, completeChatHomeTurn, clearChatHomeHistory, mergeChatHomeServerAndLocalHistory, readChatHomeHistory, writeChatHomeHistory } from './chatHistory';
import type { LineageChatCite } from './messageRef';
import { syncChatWorkMessagesWithQueue, updateChatWorkMessagesAfterItemStatus } from './chatWorkState';
import { SettingsView, routeSettingsSnapshotMessage } from './SettingsView';
import { StatusView } from './StatusView';
import { LlmDegradedBanner } from './LlmDegradedBanner';
import { EmbedderDegradedBanner } from './EmbedderDegradedBanner';
import { TelegramRailBanner } from './TelegramRailBanner';
import { OfflineBanner } from './OfflineBanner';
import { useOnlineStatus } from './onlineStatus';
import { nowUnixSeconds, readNightlyStatus, readTelegramRail, readTelegramRailDismissedSince, telegramRailBannerState, writeTelegramRailDismissedSince } from './statusRails';
import { UserTimezoneProvider } from './UserTimezoneContext';
import { LocaleProvider } from '../i18n/LocaleContext';
import { normalizeLocale, setAssistantName, t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import {
  CeoVoiceAgent,
  type CeoVoiceAgentHandle,
  type CeoVoiceChromeState,
} from './CeoVoiceAgent';
import {
  buildPanelConversationContext,
  mapViewToClientContext,
  truncateScreenContext,
  type SecretaryAskOptions,
  type SecretaryAskResult,
  mergeConversationContext,
  ownerContinuationLine,
  scrubToolWaitDump,
  voiceApprovalQuery,
  voiceContinuationPrompt,
  voiceGateToolMessage,
} from './ceoVoicePanel';
import {
  resolveCeoShorthand,
  resolveUiNavigationIntent,
  splitUiNavigationPrefix,
  voicePendingDecision,
  type CeoUiSurface,
} from './ceoIntent';
import {
  formatVoiceDuration,
  formatVoiceElapsed,
  createVoiceMicSession,
  startVoiceNoteRecording,
  voiceNoteAttachment,
  voiceNoteReducer,
  voiceNoteTranscriptPreview,
  IDLE_VOICE_NOTE,
  type VoiceMicSession,
  type VoiceNoteErrorCode,
  type VoiceNoteRecorderHandle,
  type VoiceNoteRecorderLike,
  type VoiceNoteState,
} from './voiceNote';
import type { CeoHandlers } from './ceoCommands';
import { currentWebPushSubscription, subscribeOwnerWebPush, unsubscribeOwnerWebPush } from './webPush';
import { WorkView } from './WorkView';
import './styles.css';
import './design/index.css';
import { runViewTransition } from './design/viewTransition';

type ViewId = VNextViewId;
type LoadState = 'loading' | 'ready' | 'error';
type VNextActionExecutionResult = {
  ok: boolean;
  message: string;
  nextStatus: string | null;
  itemUnavailable?: boolean;
};

/** CEO desk / Work decisions: surface a one-line secretary confirmation in Chat. */
function workActionResultShouldSurfaceInChat(actionId: VNextItemActionId): boolean {
  return (
    actionId === 'approval.approve'
    || actionId === 'approval.reject'
    || actionId === 'goal.abandon'
    || actionId === 'task.complete'
    || actionId === 'task.skip_occurrence'
    || actionId === 'tracking.cancel'
    || actionId === 'task.snooze_tomorrow'
    || actionId === 'knowledge.done'
    || actionId === 'knowledge.snooze_tomorrow'
    || actionId === 'memory.confirm'
    || actionId === 'memory.mark_wrong'
    || actionId === 'memory.delete'
  );
}

function secretaryConfirmationForWorkAction(ownerMessage: string, locale: Locale): string {
  const trimmed = ownerMessage.trim();
  if (!trimmed) return t('app.action.secretaryDone', locale);
  // Prefer server owner_message when it already reads as a short receipt.
  if (trimmed.length <= 160) return trimmed;
  return tf('app.action.secretaryDoneWithDetail', locale, { detail: trimmed.slice(0, 120) });
}

function sensitiveAccessStatus(problem: ApiResponseError['problem']): 'mfa_required' | 'consent_required' | null {
  if (!problem) return null;
  const status = problem.status || problem.error || problem.error_kind;
  if (status === 'mfa_required' || status === 'consent_required') return status;
  return null;
}

// Module-level pure fn: the owner's display_language locale is threaded in as
// a param (callers pass errorLocale) — no hardcoded locale here. The
// status/resource_type enum values stay English (identity logic).
function sourceDetailPayloadFromOpenError(error: unknown, item: OwnerWorkItem, locale: Locale): unknown | null {
  if (!(error instanceof ApiResponseError)) return null;
  const status = sensitiveAccessStatus(error.problem);
  if (status) {
    return { ...error.problem, status };
  }
  if ((error.statusCode === 400 || error.statusCode === 404) && item.detail_route) {
    return {
      status: 'unavailable',
      resource_type: 'source_unavailable',
      resource_id: item.source_id,
      detail: {
        status: 'changed',
        title: ownerVisibleCodedTitle(item, locale) || t('app.sourceDetail.changedTitle', locale),
        message: t('app.sourceDetail.changedMessage', locale),
        next_step: t('app.sourceDetail.changedNextStep', locale),
      },
    };
  }
  return null;
}

type QueuedChatHomeCommand = {
  text: string;
  displayText: string;
  context: ChatHomeAttachedContext | null;
  quote: ChatHomeQuote | null;
  mode: 'text' | 'voice';
  queuedMessageId: string;
  /** Attachments captured when the turn was queued (voice/image/document). */
  attachments?: VNextCommandAttachment[];
  retryIdempotencyKey?: string;
};
type QueuedChatHomeWorkPrompt = {
  messageId: string;
  item: OwnerWorkItem;
  text: string;
  queuedThreadId: string;
};

const GOOGLE_LOGIN_PATH = '/auth/login/google';
const CHAT_HOME_DRAFT_KEY = 'aegis-runtime:vnext:chat-home-draft';
const CHAT_HOME_HISTORY_KEY = 'aegis-runtime:vnext:chat-home-history';
// Persists the `since` value of the LLM-outage banner the owner last dismissed,
// so a refresh/relaunch does not re-show an already-acknowledged outage, while a
// NEW outage (different `since`) still surfaces.
const LLM_DEGRADED_DISMISSED_KEY = 'aegis-runtime:vnext:llm-degraded-dismissed-since';

function readLlmDegradedDismissedSince(): string | null {
  try {
    return window.localStorage.getItem(LLM_DEGRADED_DISMISSED_KEY);
  } catch {
    return null;
  }
}

function writeLlmDegradedDismissedSince(since: string): void {
  try {
    window.localStorage.setItem(LLM_DEGRADED_DISMISSED_KEY, since);
  } catch {
    // localStorage may be unavailable (private mode / quota); the banner simply
    // re-shows on next load, which is the safe (never-hide-an-outage) direction.
  }
}

// Same persistence contract as LLM_DEGRADED_DISMISSED_KEY, for the
// embedder-degraded (semantic-dedup-paused) banner.
const EMBEDDER_DEGRADED_DISMISSED_KEY = 'aegis-runtime:vnext:embedder-degraded-dismissed-since';

function readEmbedderDegradedDismissedSince(): string | null {
  try {
    return window.localStorage.getItem(EMBEDDER_DEGRADED_DISMISSED_KEY);
  } catch {
    return null;
  }
}

function writeEmbedderDegradedDismissedSince(since: string): void {
  try {
    window.localStorage.setItem(EMBEDDER_DEGRADED_DISMISSED_KEY, since);
  } catch {
    // Same safe direction as the LLM banner: re-show on next load.
  }
}
// Working-state placeholder catalog keys. The visible text is resolved against
// the owner's display_language locale (errorLocale) at each render site so the
// "Aegis is working" turns never leak English into a zh session.
const CHAT_HOME_WORKING_INITIAL_KEY = 'chatHome.working.initial';
const CHAT_HOME_WORKING_SLOW_KEY = 'chatHome.working.slow';
const CHAT_HOME_WORKING_LONG_KEY = 'chatHome.working.long';
const CHAT_HOME_SERVER_RECOVERY_INTERVAL_MS = 5000;
const CHAT_HOME_RECOVERED_REPLY_DUPLICATE_WINDOW_MS = 2 * 60 * 1000;
const MAX_CHAT_HOME_QUEUED_COMMANDS = 4;
const MAX_CHAT_HOME_QUEUED_WORK_PROMPTS = 4;

function chatHomeIdempotencyKey(): string {
  const random = globalThis.crypto?.randomUUID?.();
  if (random) return `vnext-chat:${random}`;
  return `vnext-chat:${Date.now().toString(36)}:${Math.random().toString(36).slice(2)}`;
}

function compactConversationTurnText(text: string): string {
  return text.trim().slice(0, 1000);
}

function quotedConversationTurn(quote: ChatHomeQuote): VNextConversationContextTurn {
  return {
    role: quote.role,
    text: compactConversationTurnText(quote.text),
    kind: 'quoted_reply',
    source_id: quote.sourceId,
  };
}

function readSessionDraft(key: string): string {
  try {
    return window.sessionStorage.getItem(key) ?? '';
  } catch {
    return '';
  }
}

function writeSessionDraft(key: string, value: string) {
  try {
    if (value) {
      window.sessionStorage.setItem(key, value);
    } else {
      window.sessionStorage.removeItem(key);
    }
  } catch {
    // Draft persistence is best-effort; never block the primary assistant path.
  }
}

function authenticatedChatHomeHistoryKey(session: SessionInfo | null): string | null {
  if (!session?.authenticated) return null;
  return chatHomeHistoryOwnerKey(CHAT_HOME_HISTORY_KEY, session.email);
}

function mergeServerChatHistory(serverMessages: ChatHomeMessage[], current: ChatHomeMessage[]): ChatHomeMessage[] {
  return mergeChatHomeServerAndLocalHistory(serverMessages, current);
}

function chatHomeTurnTextKey(text: string): string {
  return text.trim().replace(/\s+/g, ' ');
}

function chatHomeComparableTimestamp(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null;
}

function isSameChatHomeOwnerTurn(message: ChatHomeMessage, ownerText: string, ownerTimestamp: number): boolean {
  if (message.role !== 'owner') return false;
  if (chatHomeTurnTextKey(message.text) !== chatHomeTurnTextKey(ownerText)) return false;
  const messageTimestamp = chatHomeComparableTimestamp(message.timestamp);
  if (!messageTimestamp) return true;
  return Math.abs(messageTimestamp - ownerTimestamp) <= CHAT_HOME_RECOVERED_REPLY_DUPLICATE_WINDOW_MS;
}

function hasChatHomeAegisAfterOwner(
  messages: ChatHomeMessage[],
  ownerText: string,
  ownerTimestamp: number,
  answerText?: string,
): boolean {
  let ownerIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (isSameChatHomeOwnerTurn(messages[index], ownerText, ownerTimestamp)) {
      ownerIndex = index;
      break;
    }
  }
  if (ownerIndex === -1) return false;
  const nextOwnerIndex = messages.findIndex((message, index) => index > ownerIndex && message.role === 'owner');
  const expectedAnswerKey = answerText ? chatHomeTurnTextKey(answerText) : null;
  return messages.some((message, index) => {
    if (message.role !== 'aegis') return false;
    if (index <= ownerIndex || (nextOwnerIndex !== -1 && index >= nextOwnerIndex)) return false;
    if (!expectedAnswerKey) return true;
    return chatHomeTurnTextKey(message.result?.visible_answer || message.text) === expectedAnswerKey;
  });
}

function surfaceTitle(view: ViewId, locale: Locale): string {
  if (view === 'work') return t('app.shell.surface.work.title', locale);
  if (view === 'status') return t('app.shell.surface.status.title', locale);
  if (view === 'settings') return t('app.shell.surface.settings.title', locale);
  return t('app.shell.surface.home.title', locale);
}

function surfaceSubtitle(view: ViewId, locale: Locale): string {
  if (view === 'work') return t('app.shell.surface.work.subtitle', locale);
  if (view === 'status') return t('app.shell.surface.status.subtitle', locale);
  if (view === 'settings') return t('app.shell.surface.settings.subtitle', locale);
  return t('app.shell.surface.home.subtitle', locale);
}

function NavButton({
  view,
  activeView,
  label,
  icon,
  onClick,
  badgeCount,
  badgeTone,
  dot,
  badgeAria,
}: {
  view: ViewId;
  activeView: ViewId;
  label: string;
  icon: VNextIconName;
  onClick: () => void;
  /** Count pill: shown when > 0. Hidden when 0 or undefined. */
  badgeCount?: number;
  /** 'warn' tints the count pill amber (decisions waiting); defaults to brand blue. */
  badgeTone?: 'warn' | 'default';
  /** Calm unseen-activity dot (FYI accent, not alarming). */
  dot?: boolean;
  /** Replaces the button aria-label when a badge/dot is active (screen readers). */
  badgeAria?: string;
}) {
  const showBadge = typeof badgeCount === 'number' && badgeCount > 0;
  const isActive = activeView === view;
  return (
    <button
      type="button"
      className={classNames(isActive && 'active')}
      onClick={onClick}
      aria-label={badgeAria || undefined}
      aria-current={isActive ? 'page' : undefined}
    >
      <VNextIcon name={icon} />
      <span>
        {label}
        {showBadge && (
          <span
            className={classNames('vnext-nav-badge', badgeTone === 'warn' && 'warn')}
            aria-hidden="true"
          >
            {badgeCount}
          </span>
        )}
        {dot && <span className="vnext-nav-dot" aria-hidden="true" />}
      </span>
    </button>
  );
}

function CenterState({
  tone = 'loading',
  kicker,
  title,
  body,
  action,
}: {
  tone?: 'loading' | 'error' | 'auth';
  kicker: string;
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="vnext-app vnext-center">
      <section className={classNames('vnext-center-state', `tone-${tone}`)} aria-live={tone === 'loading' ? 'polite' : undefined}>
        <span className="vnext-center-state-mark" aria-hidden="true">
          <VNextIcon name={tone === 'error' ? 'status' : tone === 'auth' ? 'shield' : 'spark'} />
        </span>
        <p className="vnext-kicker">{kicker}</p>
        <h1>{title}</h1>
        <p>{body}</p>
        {action}
      </section>
    </div>
  );
}

function debugElementMetrics(selector: string) {
  const element = document.querySelector(selector);
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  return {
    selector,
    top: Math.round(rect.top),
    left: Math.round(rect.left),
    width: Math.round(rect.width),
    height: Math.round(rect.height),
    overflow: style.overflow,
    display: style.display,
  };
}

function isDebugElementMetric(value: ReturnType<typeof debugElementMetrics>): value is NonNullable<ReturnType<typeof debugElementMetrics>> {
  return Boolean(value);
}

function debugElementControls(selector: string) {
  const element = document.querySelector(selector);
  if (!element) return null;
  const buttons = Array.from(element.querySelectorAll('button'))
    .map((button) => (button.textContent || '').replace(/\s+/g, ' ').trim())
    .filter(Boolean)
    .map((label) => label.slice(0, 64))
    .slice(0, 20);
  if (buttons.length === 0) return null;
  return { selector, buttons };
}

function isDebugElementControl(value: ReturnType<typeof debugElementControls>): value is NonNullable<ReturnType<typeof debugElementControls>> {
  return Boolean(value);
}

export default function VNextApp() {
  const { initData } = useTelegram();
  const initialLocation = useMemo(() => readVNextLocationState(window.location), []);
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [queue, setQueue] = useState<OwnerWorkQueue | null>(null);
  const [statusProjection, setStatusProjection] = useState<VNextStatusProjection | null>(null);
  const [settingsProjection, setSettingsProjection] = useState<VNextSettingsProjection | null>(null);
  const [webPushStatus, setWebPushStatus] = useState<VNextWebPushStatus | null>(null);
  const [webPushDeviceSubscribed, setWebPushDeviceSubscribed] = useState<boolean | null>(null);
  const [webPushMessage, setWebPushMessage] = useState('');
  const [webPushBusy, setWebPushBusy] = useState(false);
  const [codexStatus, setCodexStatus] = useState<VNextCodexStatus | null>(null);
  const [codexDeviceCode, setCodexDeviceCode] = useState<VNextCodexDeviceCodeResponse | null>(null);
  const [codexMessage, setCodexMessage] = useState('');
  const [grokStatus, setGrokStatus] = useState<VNextGrokStatus | null>(null);
  const [grokDeviceCode, setGrokDeviceCode] = useState<VNextGrokDeviceCodeResponse | null>(null);
  const [grokMessage, setGrokMessage] = useState('');
  const [state, setState] = useState<LoadState>('loading');
  const [error, setError] = useState('');
  const [view, setView] = useState<ViewId>(initialLocation.view);
  // Console surfaces (settings/status/work) start at the top when the view
  // changes; the shell scroller otherwise keeps the previous view's offset and
  // the first card lands behind the sticky header.
  const shellRef = useRef<HTMLDivElement | null>(null);
  const mainRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (view === 'home') return;
    // Phones scroll the document (the shell is flex-auto there); desktop
    // scrolls the shell. Reset both so the first card sits below the header.
    const shell = shellRef.current;
    if (shell) shell.scrollTop = 0;
    // Desktop status/settings scroll `.vnext-main` (the shell is a fixed
    // column there, see chrome.css).
    const main = mainRef.current;
    if (main) main.scrollTop = 0;
    if (typeof window !== 'undefined') window.scrollTo(0, 0);
  }, [view]);
  const [selectedId, setSelectedId] = useState<string | null>(initialLocation.itemId);
  // Item the owner asked for by id (URL, back/forward, navigate('work', id)).
  // 工作 scrolls to it like a tap; the automatic first-priority pick never scrolls.
  const [requestedItemId, setRequestedItemId] = useState<string | null>(initialLocation.itemId);
  // Latest `view` for async queue refreshes: their Work-selection reconciliation
  // must read the view AT APPLY TIME (the owner may have navigated while the
  // fetch was in flight), not a stale closure. Kept in sync every render.
  const viewRef = useRef(view);
  viewRef.current = view;
  const [context, setContext] = useState<OwnerWorkContextEnvelope | null>(null);
  const [itemProjection, setItemProjection] = useState<VNextItemProjection | null>(null);
  const [assistantResult, setAssistantResult] = useState<ContextualAssistantResponse | null>(null);
  const [assistantResultItemId, setAssistantResultItemId] = useState<string | null>(null);
  const [lastItemCommand, setLastItemCommand] = useState<{
    intent: 'chat' | 'explain' | 'draft';
    text: string;
    itemId: string;
    idempotencyKey?: string;
    retryLabel?: string;
    conversationContext?: VNextConversationContextTurn[];
  } | null>(null);
  const [sourceDetail, setSourceDetail] = useState<unknown>(null);
  const [sourceDetailBusy, setSourceDetailBusy] = useState(false);
  const [actionMessagesByItemId, setActionMessagesByItemId] = useState<Record<string, string>>({});
  const [workNotice, setWorkNotice] = useState('');
  const [optimisticStatusByItemId, setOptimisticStatusByItemId] = useState<Record<string, string>>({});
  const [retiredActionItemIds, setRetiredActionItemIds] = useState<Record<string, boolean>>({});
  const [armedAction, setArmedAction] = useState<VNextItemActionId | null>(null);
  const [armedActionItemId, setArmedActionItemId] = useState<string | null>(null);
  const [executingAction, setExecutingAction] = useState<VNextItemActionId | null>(null);
  const [hostStepUp, setHostStepUp] = useState<{ itemId: string; action: VNextItemAction } | null>(null);
  const [hostStepUpBusy, setHostStepUpBusy] = useState(false);
  const [snapshotMessage, setSnapshotMessageRaw] = useState('');
  // Toast tone is tracked explicitly instead of inferred from the message text:
  // the success ("ok") styling must not depend on an English string prefix,
  // which silently failed for localized (e.g. zh) success messages.
  const [snapshotTone, setSnapshotTone] = useState<'neutral' | 'ok'>('neutral');
  const setSnapshotMessage = useCallback((message: string, tone: 'neutral' | 'ok' = 'neutral') => {
    setSnapshotMessageRaw(message);
    setSnapshotTone(message ? tone : 'neutral');
  }, []);
  const [snapshotBusy, setSnapshotBusy] = useState(false);
  const [debugLeaseBusy, setDebugLeaseBusy] = useState(false);
  const [debugLeases, setDebugLeases] = useState<VNextDebugLease[]>([]);
  const [debugGrantToken, setDebugGrantToken] = useState<string | null>(null);
  const [debugJobBusy, setDebugJobBusy] = useState(false);
  const [debugJobs, setDebugJobs] = useState<VNextDebugJob[]>([]);
  // Plan C: chat unseen-activity dot. Set true when an aegis/result message
  // arrives while the owner is NOT on the Chat home view; cleared on navigate('home').
  const [unseenChatActivity, setUnseenChatActivity] = useState(false);
  // Older server history the owner pulled in with 加载更早; shown above the
  // live window and never persisted locally (the local store keeps 24).
  const [earlierChatMessages, setEarlierChatMessages] = useState<ChatHomeMessage[]>([]);
  const [chatCiteFocus, setChatCiteFocus] = useState<{ ref: string; token: string } | null>(null);
  const [chatHistoryHasMore, setChatHistoryHasMore] = useState(false);
  const [loadingEarlierChat, setLoadingEarlierChat] = useState(false);
  const [earlierChatError, setEarlierChatError] = useState('');
  const earlierChatMessagesRef = useRef(earlierChatMessages);
  earlierChatMessagesRef.current = earlierChatMessages;
  const [chatHomeMessages, setChatHomeMessages] = useState<ChatHomeMessage[]>(
    () => readChatHomeHistory(window.sessionStorage, CHAT_HOME_HISTORY_KEY) as ChatHomeMessage[],
  );
  const chatHomeMessagesRef = useRef(chatHomeMessages);
  chatHomeMessagesRef.current = chatHomeMessages;
  const [chatHomeCommand, setChatHomeCommand] = useState(() => readSessionDraft(CHAT_HOME_DRAFT_KEY));
  const [queuedChatHomeCommands, setQueuedChatHomeCommands] = useState<QueuedChatHomeCommand[]>([]);
  const [queuedChatHomeWorkPrompts, setQueuedChatHomeWorkPrompts] = useState<QueuedChatHomeWorkPrompt[]>([]);
  const [chatHomeInputMode, setChatHomeInputMode] = useState<'text' | 'voice'>('text');
  const [chatHomeAttachedContext, setChatHomeAttachedContext] = useState<ChatHomeAttachedContext | null>(null);
  // Slice 5: a file the owner uploaded and is holding in the composer, pending
  // the next general-chat command submit. Cleared on submit (success or queue)
  // and on explicit remove. `attachmentUploading`/`attachmentError` drive the
  // pill's transient states.
  const [chatHomePendingAttachment, setChatHomePendingAttachment] = useState<ChatHomePendingAttachment | null>(null);
  const [chatHomeAttachmentUploading, setChatHomeAttachmentUploading] = useState(false);
  const [chatHomeAttachmentError, setChatHomeAttachmentError] = useState('');
  const [chatHomeQuotedMessage, setChatHomeQuotedMessage] = useState<ChatHomeQuote | null>(null);
  const [chatHomeNotice, setChatHomeNotice] = useState('');
  const [workLoadNotice, setWorkLoadNotice] = useState('');
  const [avatarProfile, setAvatarProfile] = useState<AvatarProfile>(() => defaultAvatarProfile());
  const [avatarAutoRefresh, setAvatarAutoRefresh] = useState<VNextAvatarAutoRefreshProjection | null>(null);
  const [avatarGenerationBusy, setAvatarGenerationBusy] = useState(false);
  const [installPromptEvent, setInstallPromptEvent] = useState<OwnerInstallPromptEvent | null>(null);
  const [installStatus, setInstallStatus] = useState<OwnerInstallStatus>(() => currentOwnerInstallStatus(false));
  const [expandedLaneIds, setExpandedLaneIds] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [costSavingBusy, setCostSavingBusy] = useState(false);
  const [codexBusy, setCodexBusy] = useState(false);
  const [grokBusy, setGrokBusy] = useState(false);
  const [logoutBusy, setLogoutBusy] = useState(false);
  // The `since` of the LLM-outage banner the owner last dismissed. Initialized
  // from localStorage so a dismissed outage stays dismissed across reloads; a
  // new outage carries a different `since` and re-shows (see LlmDegradedBanner).
  const [llmDegradedDismissedSince, setLlmDegradedDismissedSince] = useState<string | null>(
    () => readLlmDegradedDismissedSince(),
  );
  // Same dismissal contract for the embedder-degraded (dedup-paused) banner.
  const [embedderDegradedDismissedSince, setEmbedderDegradedDismissedSince] = useState<string | null>(
    () => readEmbedderDegradedDismissedSince(),
  );
  // Same dismissal contract for the Telegram rail banner, keyed on the
  // episode's `since_unix` (a new episode re-arms; see TelegramRailBanner).
  const [telegramRailDismissedSince, setTelegramRailDismissedSince] = useState<number | null>(
    () => readTelegramRailDismissedSince(),
  );
  // Browser connectivity (navigator.onLine + window online/offline). While
  // false the offline banner takes the single banner slot and the runtime
  // banners step aside — their data cannot refresh without a network anyway.
  const online = useOnlineStatus();
  const privateSessionEpochRef = useRef(0);
  const chatHomeGenerationRef = useRef(0);
  // Plan C: timestamp of the last aegis message the owner has "seen" on Chat home.
  // Initialized to Date.now() so server-hydrated history (old timestamps) never
  // spuriously lights the dot on first load. The derived-signal effect below is the
  // SOLE writer of unseenChatActivity — it covers ALL channels that produce aegis
  // messages (inline handler, appendWorkActionResultToChat, mergeServerChatHistory,
  // syncChatWorkMessagesWithQueue) because they all mutate chatHomeMessages.
  const lastSeenChatAtRef = useRef<number>(Date.now());
  // Plan C: derived-signal effect — single source of truth for unseenChatActivity.
  // Keyed on [chatHomeMessages, view] so it fires whenever either changes. Covers
  // async channels: inline response handler, appendWorkActionResultToChat,
  // mergeServerChatHistory (recovery poll), syncChatWorkMessagesWithQueue (queue
  // completion). No per-site setUnseenChatActivity(true) calls needed.
  useEffect(() => {
    const latestAegisAt = chatHomeMessages.reduce((max, msg) => {
      if (msg.role !== 'aegis') return max;
      // Fail-safe: a missing/≤0 timestamp contributes 0 (never newer than the
      // seen-mark) so a pathological timestampless hydrated message can't SPURIOUSLY
      // light the dot. Every live aegis writer sets timestamp: Date.now(), and the
      // apiGuards reject non-finite server timestamps, so real new replies always
      // carry a valid positive epoch and still light the dot correctly.
      const ts = msg.timestamp && msg.timestamp > 0 ? msg.timestamp : 0;
      return Math.max(max, ts);
    }, 0);
    if (view === 'home') {
      // Owner is on Chat home — absorb all current aegis content into the seen-mark
      // so future arrivals can be compared against it.
      lastSeenChatAtRef.current = Math.max(lastSeenChatAtRef.current, latestAegisAt, Date.now());
      setUnseenChatActivity(false);
    } else {
      setUnseenChatActivity(latestAegisAt > lastSeenChatAtRef.current);
    }
  }, [chatHomeMessages, view]);
  // Bumped (as state, so it re-renders) every time the private-session epoch or
  // the chat-home generation advances — i.e. on reconnect/re-auth, logout, or a
  // conversation clear. Drives the unconditional busy-release effect below so an
  // in-flight request that resolves AFTER its epoch/generation was invalidated
  // (a hung/long Codex call across a reconnect) can never wedge `busy` true. The
  // per-handler finally blocks stay epoch/generation-guarded so a stale request
  // never clobbers a NEWER request's busy=true; this effect is the failsafe that
  // releases busy exactly when the session that owned it is torn down.
  const [sessionResetTick, setSessionResetTick] = useState(0);
  const hydratedChatHomeHistoryKeyRef = useRef<string | null>(null);
  const hydratedAvatarProfileKeyRef = useRef<string | null>(null);
  const didMountViewRef = useRef(false);
  // Synchronous in-flight guard for HITL approval actions: blocks a second tap
  // (e.g. a rapid approve-then-reject double-tap) on the SAME approval before the
  // optimistic React state settles — without it both calls fire and the losing
  // action's 404-as-success would paint the wrong resolved chip.
  const approvalActionInFlightRef = useRef<Set<string>>(new Set());
  // Composer mic = VOICE NOTE (record → upload → server-side STT), not
  // dictation. The top-bar orb owns duplex Realtime / panel dictation
  // separately and is untouched by this.
  const voiceNoteRecorderRef = useRef<VoiceNoteRecorderHandle | null>(null);
  // One microphone grant serves consecutive notes (see VOICE_MIC_IDLE_RELEASE_MS).
  const voiceMicSessionRef = useRef<VoiceMicSession | null>(null);
  // Bumped by cancel so a transcript that lands after the owner gave up is dropped.
  const voiceUploadGenerationRef = useRef(0);
  const [chatHomeVoiceNote, setChatHomeVoiceNote] = useState<VoiceNoteState>(IDLE_VOICE_NOTE);
  useEffect(() => {
    const dropMic = () => {
      if (document.visibilityState === 'hidden') voiceMicSessionRef.current?.dispose();
    };
    document.addEventListener('visibilitychange', dropMic);
    return () => {
      document.removeEventListener('visibilitychange', dropMic);
      voiceMicSessionRef.current?.dispose();
    };
  }, []);
  const [voiceRealtimeAvailable, setVoiceRealtimeAvailable] = useState(false);
  /** Live duplex snapshot for the top-bar Aegis summon control. */
  const [voiceChrome, setVoiceChrome] = useState<CeoVoiceChromeState | null>(null);
  const [voiceLiveLine, setVoiceLiveLine] = useState('');
  /** One in-flow search card while duplex voice is waiting on the owner. */
  const [voiceApproval, setVoiceApproval] = useState<{ approvalId: string; query: string } | null>(null);
  const voiceApprovalSettledRef = useRef(new Set<string>());
  const recentVoiceDecisionRef = useRef<{ at: number; action: 'approve' | 'reject'; line: string } | null>(null);
  const trackVoiceGateRef = useRef<(card: { approvalId: string; toolName: string; summary: string }) => void>(() => {});
  useEffect(() => {
    const onApproval = (event: Event) => {
      const detail = (event as CustomEvent<{ approvalId?: string; toolName?: string; summary?: string }>).detail;
      if (!detail?.approvalId) return;
      trackVoiceGateRef.current({
        approvalId: detail.approvalId,
        toolName: detail.toolName || 'web_search',
        summary: detail.summary || '',
      });
    };
    window.addEventListener('aegis-voice-approval', onApproval);
    return () => window.removeEventListener('aegis-voice-approval', onApproval);
  }, []);
  /** Bottom-sheet chat over Work/Status/Settings (same ChatHomeView instance). */
  const [chatSheetOpen, setChatSheetOpen] = useState(false);
  const chatSheetBackdropRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!chatSheetOpen || view === 'home') return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      setChatSheetOpen(false);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [chatSheetOpen, view]);

  // Lock the real page scroller (`.vnext-shell` on phone layouts) while the
  // sheet is open over a non-Aegis page. overflow:hidden on that element
  // blocks iOS touch scrolling; document fallback covers window-scroll layouts.
  // Restore on close, when switching to the Aegis tab, and on unmount.
  useEffect(() => {
    const shouldLock = chatSheetOpen && view !== 'home';
    if (!shouldLock) {
      releaseChatSheetScrollLock();
      return undefined;
    }
    // Prefer `.vnext-shell` (phone-layout page scroller). When the sheet is
    // portaled to document.body its ancestors no longer include the shell, so
    // the selector fallback in findChatSheetPageScroller is required.
    const host = document.querySelector<HTMLElement>('[data-testid="aegis-chat-panel-host"]');
    const scroller = findChatSheetPageScroller(
      host?.parentElement instanceof HTMLElement ? host.parentElement : null,
      (el) => window.getComputedStyle(el).overflowY,
      (sel) => document.querySelector<HTMLElement>(sel),
    );
    applyChatSheetScrollLock(scroller);
    const backdrop = chatSheetBackdropRef.current
      ?? document.querySelector<HTMLElement>('[data-testid="aegis-chat-sheet-backdrop"]');
    const sheet = document.querySelector<HTMLElement>('[data-testid="aegis-chat-sheet"]');
    const stream = document.querySelector<HTMLElement>('[data-testid="aegis-chat-sheet-stream"]');
    const isInStream = (target: EventTarget | null) => (
      !!stream && target instanceof Node && stream.contains(target)
    );
    const blockTouchMove = (event: TouchEvent) => {
      // Timeline keeps pan-y; chrome/composer/backdrop must not move the page.
      if (isInStream(event.target)) return;
      event.preventDefault();
    };
    const blockWheel = (event: WheelEvent) => {
      if (isInStream(event.target)) return;
      event.preventDefault();
    };
    const opts: AddEventListenerOptions = { passive: false };
    if (backdrop) {
      backdrop.addEventListener('touchmove', blockTouchMove, opts);
      backdrop.addEventListener('wheel', blockWheel, opts);
    }
    // Only gate sheet chrome/composer when the timeline scroller is present so
    // a missed stream query cannot swallow timeline pan/wheel.
    const gateSheetChrome = !!(sheet && stream);
    if (gateSheetChrome && sheet) {
      sheet.addEventListener('touchmove', blockTouchMove, opts);
      sheet.addEventListener('wheel', blockWheel, opts);
    }
    return () => {
      if (backdrop) {
        backdrop.removeEventListener('touchmove', blockTouchMove);
        backdrop.removeEventListener('wheel', blockWheel);
      }
      if (gateSheetChrome && sheet) {
        sheet.removeEventListener('touchmove', blockTouchMove);
        sheet.removeEventListener('wheel', blockWheel);
      }
      releaseChatSheetScrollLock();
    };
  }, [chatSheetOpen, view]);
  const secretaryHandlersRef = useRef<CeoHandlers>({});
  const ceoVoiceAgentRef = useRef<CeoVoiceAgentHandle | null>(null);
  /** Runtime HITL gates raised during voice ask_chief. */
  const pendingVoiceGatesRef = useRef<Array<{
    approvalId: string;
    toolName: string;
    summary: string;
  }>>([]);
  const chatHomeHasTransientMessages = chatHomeMessages.some((message) => (
    message.role === 'working' || message.role === 'queued'
  ));

  const selected = useMemo(
    () => queue?.items.find((item) => item.id === selectedId) ?? null,
    [queue, selectedId],
  );
  const userTimezone = settingsProjection?.user_timezone ?? null;
  // Error/system strings are Channel B (spec §2/§6): they follow the persisted
  // `display_language` setting, not a transient toggle. `VNextApp` itself mounts
  // the LocaleProvider, so it cannot consume `useLocale()`; it reads the setting
  // directly (a pure function, valid in the component body) instead.
  const errorLocale: Locale = normalizeLocale(settingsProjection?.display_language ?? undefined);
  const home = useMemo(() => {
    if (!queue) return null;
    const nextHome = buildVNextHome<OwnerWorkItem>(queue, errorLocale);
    return { ...nextHome, signals: buildVNextHomeSignals(nextHome, userTimezone, errorLocale) };
  }, [queue, userTimezone, errorLocale]);
  const lanes = useMemo(() => queue ? buildVNextLanes<OwnerWorkItem>(queue.items, errorLocale) : [], [queue, errorLocale]);
  const decisionCount = lanes.find((lane) => lane.id === 'decide')?.items.length ?? 0;
  const readyCount = lanes.find((lane) => lane.id === 'now')?.items.length ?? 0;
  const workBadgeCount = decisionCount + readyCount;
  const workBadgeLabel = decisionCount > 0
    ? tf(
        decisionCount === 1 ? 'app.shell.workBadge.decisionsSingular' : 'app.shell.workBadge.decisionsPlural',
        errorLocale,
        { count: String(decisionCount) },
      )
    : readyCount > 0
      ? tf(
          readyCount === 1 ? 'app.shell.workBadge.readySingular' : 'app.shell.workBadge.readyPlural',
          errorLocale,
          { count: String(readyCount) },
        )
      : t('app.shell.workBadge.plain', errorLocale);
  const snapshotAvailable = Boolean(
    statusProjection?.advanced_diagnostics_available
      || settingsProjection?.advanced_diagnostics_available,
  );
  const sourceDetailView = useMemo(
    () => sourceDetail === null || sourceDetail === undefined ? null : buildSourceDetailView(sourceDetail, errorLocale, userTimezone),
    [sourceDetail, errorLocale, userTimezone],
  );
  const selectedActionMessage = selected?.id ? actionMessagesByItemId[selected.id] || '' : '';
  const selectedActionsRetired = selected?.id ? Boolean(retiredActionItemIds[selected.id]) : false;
  function toggleLane(laneId: string) {
    setExpandedLaneIds((current) => current.includes(laneId)
      ? current.filter((id) => id !== laneId)
      : [...current, laneId]);
  }

  function firstPriorityItemId(items: OwnerWorkItem[]): string | null {
    const activeItems = items.filter((item) => !isClosedItem(item));
    const nextLane = buildVNextLanes<OwnerWorkItem>(activeItems, errorLocale).find((lane) => lane.items.length > 0);
    return nextLane?.items[0]?.id ?? null;
  }

  const loadApp = useCallback(async (cancelled: () => boolean) => {
    setState('loading');
    setError('');
    try {
      const nextSession = await fetchSession(initData);
      if (cancelled()) return;
      setAssistantName(nextSession.assistant_name);
      setSession(nextSession);
      if (!nextSession.authenticated) {
        clearPrivateBrowserSessionState();
        setState('ready');
        return;
      }
      // Work queue is the only hard dependency for the shell — but a transient
      // network/timeout blip (or a busy host during deploy compile) must not
      // leave the owner on a dead "无法打开 Aegis" screen. Soft-fail to an
      // empty queue so the rest of the app still opens; retry can refill.
      let queueLoadNotice = '';
      let nextQueue: OwnerWorkQueue;
      try {
        nextQueue = await fetchVNextBackedOwnerQueue(initData);
      } catch (queueError) {
        if (cancelled()) return;
        // Session cookie stale for API but session endpoint still said auth —
        // bounce to login instead of a permanent error wall.
        if (queueError && typeof queueError === 'object' && 'statusCode' in queueError
          && (queueError as { statusCode?: number }).statusCode === 401) {
          clearPrivateBrowserSessionState();
          setSession({ authenticated: false });
          setState('ready');
          return;
        }
        nextQueue = {
          schema_version: 1,
          generated_at: new Date().toISOString(),
          owner: nextSession.email || 'owner',
          sources: [],
          counts: {},
          items: [],
        };
        queueLoadNotice = t('chatHome.notice.workLoadFailed', errorLocale);
      }
      const [nextStatus, nextSettings, nextWebPushStatus, nextDebugLeases, nextDebugJobs] = await Promise.all([
        fetchVNextStatus(initData).catch(() => null),
        fetchVNextSettings(initData).catch(() => null),
        fetchVNextWebPushStatus(initData).catch(() => null),
        fetchVNextDebugLeases(initData).catch(() => null),
        fetchVNextDebugJobs(initData).catch(() => null),
      ]);
      if (cancelled()) return;
      setQueue(nextQueue);
      setWorkLoadNotice(queueLoadNotice);
      syncChatHomeWorkItemsFromQueue(nextQueue);
      setStatusProjection(nextStatus);
      setSettingsProjection(nextSettings);
      setWebPushStatus(nextWebPushStatus);
      void syncWebPushDeviceRegistration(nextWebPushStatus, nextSession.csrf_token);
      setDebugLeases(nextDebugLeases?.leases ?? []);
      setDebugJobs(nextDebugJobs?.jobs ?? []);
      const [nextCodexStatus, nextGrokStatus, nextVoiceStatus] = await Promise.all([
        fetchVNextCodexStatus(initData).catch(() => null),
        fetchVNextGrokStatus(initData).catch(() => null),
        fetchVNextVoiceStatus(initData).catch(() => null),
      ]);
      if (cancelled()) return;
      setCodexStatus(nextCodexStatus);
      setGrokStatus(nextGrokStatus);
      setVoiceRealtimeAvailable(Boolean(nextVoiceStatus?.realtime_available));
      setSelectedId((current) => {
        if (current && nextQueue.items.some((item) => item.id === current)) return current;
        return firstPriorityItemId(nextQueue.items);
      });
      setState('ready');
    } catch (loadError) {
      if (cancelled()) return;
      if (isLocalStaticPreviewSessionMiss(loadError)) {
        clearPrivateBrowserSessionState();
        setSession({ authenticated: false });
        setState('ready');
        return;
      }
      setError(displayError(loadError, t('error.fallback.couldNotLoadAegis', errorLocale), errorLocale));
      setState('error');
    }
  }, [initData]);

  function applyOptimisticStatuses(nextQueue: OwnerWorkQueue, overrides: Record<string, string> = optimisticStatusByItemId): OwnerWorkQueue {
    if (Object.keys(overrides).length === 0) {
      return { ...nextQueue, items: nextQueue.items.filter((item) => !isClosedItem(item)) };
    }
    return {
      ...nextQueue,
      items: nextQueue.items.map((item) => {
        const status = overrides[item.id];
        return status ? { ...item, status } : item;
      }).filter((item) => !isClosedItem(item)),
    };
  }

  function updateQueueItemStatus(itemId: string, status: string) {
    setQueue((current) => current ? applyOptimisticStatuses(current, { ...optimisticStatusByItemId, [itemId]: status }) : current);
  }

  function updateChatHomeWorkItemStatus(itemId: string, status: string) {
    setChatHomeMessages((current) => updateChatWorkMessagesAfterItemStatus(current, itemId, status));
  }

  function settleVoiceUtterance(role: 'owner' | 'aegis', text: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    setChatHomeMessages((current) => {
      const last = current[current.length - 1];
      if (last && last.role === role && last.text === trimmed) return current;
      return current.concat({
        id: `voice:${role}:${Date.now()}:${Math.random().toString(36).slice(2, 8)}`,
        role,
        text: trimmed,
        timestamp: Date.now(),
      });
    });
  }

  function appendWorkActionResultToChat(target: OwnerWorkItem, action: VNextItemAction, message: string) {
    if (!workActionResultShouldSurfaceInChat(action.id)) return;
    const text = secretaryConfirmationForWorkAction(message, errorLocale);
    if (!text) return;
    const result: ContextualAssistantResponse = {
      schema_version: 1,
      response_kind: 'work_action_result',
      visible_answer: text,
      severity: 'notice',
      receipt_expectation: t('app.receipt.workActionNoExternal', errorLocale),
      citations: [],
      warnings: [],
    };
    setChatHomeMessages((current) => current.concat({
      id: `aegis:work-action:${Date.now()}:${Math.random().toString(36).slice(2)}`,
      role: 'aegis',
      text,
      timestamp: Date.now(),
      result,
      contextLabel: ownerVisibleCodedTitle(target, errorLocale)
        ? tf('app.action.workContextLabel', errorLocale, { title: ownerVisibleCodedTitle(target, errorLocale) })
        : t('app.action.workContextFallback', errorLocale),
    }));
  }

  function syncChatHomeWorkItemsFromQueue(nextQueue: OwnerWorkQueue) {
    setChatHomeMessages((current) => syncChatWorkMessagesWithQueue(current, nextQueue, errorLocale));
  }

  function isPrivateSessionEpochCurrent(epoch: number): boolean {
    return privateSessionEpochRef.current === epoch;
  }

  function invalidatePrivateSessionEpoch() {
    privateSessionEpochRef.current += 1;
    setSessionResetTick((tick) => tick + 1);
  }

  async function refreshQueue(
    preferredSelectedId?: string,
    statusOverrides: Record<string, string> = optimisticStatusByItemId,
    sessionEpoch = privateSessionEpochRef.current,
  ): Promise<OwnerWorkQueue | null> {
    const nextQueue = await fetchVNextBackedOwnerQueue(initData);
    if (!isPrivateSessionEpochCurrent(sessionEpoch)) return null;
    setWorkLoadNotice('');
    const projectedQueue = applyOptimisticStatuses(nextQueue, statusOverrides);
    setQueue(projectedQueue);
    syncChatHomeWorkItemsFromQueue(projectedQueue);
    setRetiredActionItemIds((current) => {
      const next = { ...current };
      for (const item of projectedQueue.items) {
        delete next[item.id];
      }
      return next;
    });
    setSelectedId((current) => {
      if (preferredSelectedId && projectedQueue.items.some((item) => item.id === preferredSelectedId)) {
        return preferredSelectedId;
      }
      return current && projectedQueue.items.some((item) => item.id === current)
        ? current
        : firstPriorityItemId(projectedQueue.items);
    });
    return projectedQueue;
  }

  async function refreshQueueAfterAssistant(preferredSelectedId?: string) {
    try {
      await refreshQueue(preferredSelectedId);
    } catch {
      // Assistant answers remain useful even if the work projection refresh
      // briefly fails; the next navigation or page refresh will retry.
    }
  }

  async function loadEarlierChatHistory() {
    if (loadingEarlierChat) return;
    const shown = [...earlierChatMessages, ...chatHomeMessages].filter((message) => typeof message.timestamp === 'number' && message.timestamp > 0);
    if (shown.length === 0) return;
    const oldestMs = Math.min(...shown.map((message) => message.timestamp as number));
    // State timestamps are milliseconds (server ones are normalised on read);
    // the cursor is unix seconds. The guard covers a raw value slipping in.
    const before = oldestMs > 1_000_000_000_000 ? Math.floor(oldestMs / 1000) : oldestMs;
    const sessionEpoch = privateSessionEpochRef.current;
    const chatGeneration = chatHomeGenerationRef.current;
    setLoadingEarlierChat(true);
    setEarlierChatError('');
    try {
      const history = await fetchVNextChatHistory(initData, { before });
      if (!isPrivateSessionEpochCurrent(sessionEpoch) || chatGeneration !== chatHomeGenerationRef.current) return;
      const known = new Set([...earlierChatMessages, ...chatHomeMessages].map((message) => message.id));
      const older = chatHomeMessagesFromServerHistory(history)
        .filter((message) => !known.has(message.id))
        .filter((message) => typeof message.timestamp !== 'number' || message.timestamp < oldestMs);
      setEarlierChatMessages((current) => isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current
        ? [...older, ...current] : current);
      setChatHistoryHasMore((current) => isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current
        ? Boolean(history.has_more) : current);
    } catch {
      if (isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current) {
        setEarlierChatError(t('chatHome.loadEarlier.failed', errorLocale));
      }
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current) setLoadingEarlierChat(false);
    }
  }

  async function syncChatHomeHistoryFromServer(sessionEpoch = privateSessionEpochRef.current) {
    // Clear advances the conversation generation without changing the session.
    // Recheck in updaters too: React may apply them after a later Clear.
    const chatGeneration = chatHomeGenerationRef.current;
    const isCurrent = () => isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current;
    try {
      const history = await fetchVNextChatHistory(initData);
      if (!isCurrent()) return;
      setChatHistoryHasMore((current) => isCurrent() ? Boolean(history.has_more) : current);
      const serverMessages = chatHomeMessagesFromServerHistory(history);
      setChatHomeMessages((current) => {
        if (!isCurrent()) return current;
        const merged = mergeServerChatHistory(serverMessages, current);
        const withQueue = queue ? syncChatWorkMessagesWithQueue(merged, queue, errorLocale) : merged;
        return replaceApprovalMessagesFromServer(withQueue, history.approvals);
      });
    } catch {
      // The in-flight command response remains the primary path; server history
      // recovery is a best-effort fallback for dropped or interrupted replies.
    }
  }

  function setItemActionMessage(itemId: string, message: string) {
    setActionMessagesByItemId((current) => {
      if (!message) {
        const { [itemId]: _removed, ...next } = current;
        return next;
      }
      return { ...current, [itemId]: message };
    });
  }

  useEffect(() => {
    let cancelled = false;
    void loadApp(() => cancelled);
    return () => {
      cancelled = true;
    };
  }, [loadApp]);

  useEffect(() => {
    writeSessionDraft(CHAT_HOME_DRAFT_KEY, chatHomeCommand);
  }, [chatHomeCommand]);

  // Persisted runtime notices (gate / tool warnings) are re-rendered from
  // storage; pass the console locale explicitly so they never fall back to
  // the shipped <html lang> default.
  useEffect(() => {
    writeChatHomeHistory(window.sessionStorage, CHAT_HOME_HISTORY_KEY, chatHomeMessages, errorLocale);
    const ownerHistoryKey = authenticatedChatHomeHistoryKey(session);
    if (!ownerHistoryKey) return;
    const existingOwnerHistory = readChatHomeHistory(window.localStorage, ownerHistoryKey, errorLocale);
    if (chatHomeMessages.length === 0 && existingOwnerHistory.length > 0) return;
    writeChatHomeHistory(window.localStorage, ownerHistoryKey, chatHomeMessages, errorLocale);
  }, [chatHomeMessages, session?.authenticated, session?.email, errorLocale]);

  useEffect(() => {
    const ownerHistoryKey = authenticatedChatHomeHistoryKey(session);
    if (!ownerHistoryKey || hydratedChatHomeHistoryKeyRef.current === ownerHistoryKey) return;
    hydratedChatHomeHistoryKeyRef.current = ownerHistoryKey;
    const ownerHistory = readChatHomeHistory(window.localStorage, ownerHistoryKey, errorLocale) as ChatHomeMessage[];
    const tabHistory = readChatHomeHistory(window.sessionStorage, CHAT_HOME_HISTORY_KEY, errorLocale) as ChatHomeMessage[];
    setChatHomeMessages((current) => {
      if (current.length > 0) {
        writeChatHomeHistory(window.localStorage, ownerHistoryKey, current, errorLocale);
        return current;
      }
      if (ownerHistory.length > 0) return ownerHistory;
      if (tabHistory.length > 0) {
        writeChatHomeHistory(window.localStorage, ownerHistoryKey, tabHistory, errorLocale);
        return tabHistory;
      }
      return current;
    });
  }, [session?.authenticated, session?.email, errorLocale]);

  useEffect(() => {
    if (!session?.authenticated) return;
    let cancelled = false;
    const sessionEpoch = privateSessionEpochRef.current;
    const chatGeneration = chatHomeGenerationRef.current;
    const isCurrent = () => !cancelled && isPrivateSessionEpochCurrent(sessionEpoch) && chatGeneration === chatHomeGenerationRef.current;
    fetchVNextChatHistory(initData)
      .then((history) => {
        if (!isCurrent()) return;
        setChatHistoryHasMore((current) => isCurrent() ? Boolean(history.has_more) : current);
        const serverMessages = chatHomeMessagesFromServerHistory(history);
        setChatHomeMessages((current) => {
          if (!isCurrent()) return current;
          const merged = mergeServerChatHistory(serverMessages, current);
          const withQueue = queue ? syncChatWorkMessagesWithQueue(merged, queue, errorLocale) : merged;
          return replaceApprovalMessagesFromServer(withQueue, history.approvals);
        });
      })
      .catch(() => {
        // Local browser history remains a best-effort fallback when the L1
        // server projection is temporarily unavailable.
      });
    return () => {
      cancelled = true;
    };
  }, [initData, session?.authenticated, session?.email]);

  useEffect(() => {
    if (!session?.authenticated || !chatHomeHasTransientMessages) return;
    let cancelled = false;
    const sessionEpoch = privateSessionEpochRef.current;
    const recover = () => {
      if (cancelled || document.visibilityState !== 'visible') return;
      void syncChatHomeHistoryFromServer(sessionEpoch);
    };
    const interval = window.setInterval(recover, CHAT_HOME_SERVER_RECOVERY_INTERVAL_MS);
    recover();
    const onVisibilityChange = () => recover();
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [chatHomeHasTransientMessages, initData, session?.authenticated, session?.email]);

  useEffect(() => {
    if (!session?.authenticated) {
      hydratedAvatarProfileKeyRef.current = null;
      setAvatarProfile(defaultAvatarProfile());
      setAvatarAutoRefresh(null);
      return;
    }
    let cancelled = false;
    const ownerKey = avatarProfileOwnerKey(session.email);
    if (hydratedAvatarProfileKeyRef.current === ownerKey) return;
    hydratedAvatarProfileKeyRef.current = ownerKey;
    const cachedProfile = readAvatarProfile(window.localStorage, ownerKey);
    setAvatarProfile(cachedProfile);
    fetchVNextAvatarProfile(initData)
      .then((serverProfile) => {
        if (cancelled) return;
        const nextProfile = avatarProfileFromServer(serverProfile);
        writeAvatarProfile(window.localStorage, ownerKey, nextProfile);
        setAvatarProfile(nextProfile);
        setAvatarAutoRefresh(serverProfile.auto_refresh ?? null);
      })
      .catch(() => {
        // Cached avatars keep the UI stable if profile sync is temporarily unavailable.
      });
    return () => {
      cancelled = true;
    };
  }, [initData, session?.authenticated, session?.email]);

  useEffect(() => {
    if (!session?.authenticated) return;
    let cancelled = false;
    const mark = () => {
      if (cancelled || document.visibilityState !== 'visible') return;
      void markVNextPresence(initData, session.csrf_token, view).catch(() => {
        // Presence only decides whether proactive Telegram sends are needed.
      });
    };
    mark();
    const interval = window.setInterval(mark, 60_000);
    document.addEventListener('visibilitychange', mark);
    window.addEventListener('focus', mark);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      document.removeEventListener('visibilitychange', mark);
      window.removeEventListener('focus', mark);
    };
  }, [initData, session?.authenticated, session?.csrf_token, view]);

  useEffect(() => {
    if (!session?.authenticated || !snapshotAvailable) return;
    if (!debugJobs.some((job) => job.status === 'queued' || job.status === 'running')) return;
    let cancelled = false;
    const sessionEpoch = privateSessionEpochRef.current;
    const refreshDebugJobs = () => {
      if (cancelled || document.visibilityState !== 'visible') return;
      void fetchVNextDebugJobs(initData)
        .then((response) => {
          if (!cancelled && isPrivateSessionEpochCurrent(sessionEpoch)) setDebugJobs(response.jobs);
        })
        .catch(() => undefined);
    };
    const interval = window.setInterval(refreshDebugJobs, 3_000);
    refreshDebugJobs();
    window.addEventListener('focus', refreshDebugJobs);
    document.addEventListener('visibilitychange', refreshDebugJobs);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener('focus', refreshDebugJobs);
      document.removeEventListener('visibilitychange', refreshDebugJobs);
    };
  }, [debugJobs, initData, session?.authenticated, snapshotAvailable]);

  useEffect(() => {
    function handleBeforeInstallPrompt(event: Event) {
      event.preventDefault();
      setInstallPromptEvent(event as OwnerInstallPromptEvent);
      setInstallStatus(currentOwnerInstallStatus(true));
    }

    function handleAppInstalled() {
      setInstallPromptEvent(null);
      setInstallStatus('installed');
      setChatHomeNotice(t('chatHome.notice.installed', errorLocale));
    }

    setInstallStatus(currentOwnerInstallStatus(false));
    window.addEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
    window.addEventListener('appinstalled', handleAppInstalled);
    return () => {
      window.removeEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
      window.removeEventListener('appinstalled', handleAppInstalled);
    };
  }, []);

  useEffect(() => {
    function onPopState() {
      const next = readVNextLocationState(window.location);
      runViewTransition(() => {
        setView(next.view);
        setSelectedId(next.itemId);
        setRequestedItemId(next.itemId);
      });
    }
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  // a11y: on a view change (not the initial mount), move focus to the main
  // content region so keyboard + screen-reader users land in the new view
  // instead of being stranded on the now-gone control they activated.
  useEffect(() => {
    if (!didMountViewRef.current) {
      didMountViewRef.current = true;
      return;
    }
    // preventScroll: a plain focus() scrolls <main> to the top of the shell
    // scroller, which parks the first card under the sticky header.
    mainRef.current?.focus({ preventScroll: true });
  }, [view]);

  useEffect(() => {
    if (!session?.authenticated || state !== 'ready') return;
    writeVNextLocationState(window, { view, itemId: selectedId }, 'replace');
  }, [selectedId, session, state, view]);

  // Keep the chat composer on the visual viewport while the keyboard is open.
  // --keyboard-visible is that viewport's height, not a second subtraction from
  // 100dvh (iOS may already have shrunk dvh). The class lives on <html> so a
  // React render cannot drop it. A closed keyboard removes the class and the
  // tab bar returns on the home-indicator inset.
  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const apply = () => { syncKeyboardInset(window); };
    apply();
    const viewport = window.visualViewport;
    viewport?.addEventListener('resize', apply);
    viewport?.addEventListener('scroll', apply);
    window.addEventListener('resize', apply);
    window.addEventListener('focusin', apply);
    window.addEventListener('focusout', apply);
    return () => {
      viewport?.removeEventListener('resize', apply);
      viewport?.removeEventListener('scroll', apply);
      window.removeEventListener('resize', apply);
      window.removeEventListener('focusin', apply);
      window.removeEventListener('focusout', apply);
      clearKeyboardInset(document);
    };
  }, []);

  // Diagnostic overlay for keyboard/viewport behaviour on real devices: open
  // the console with `#vvdebug` in the URL and it prints the live numbers the
  // browser reports (standalone mode, visual viewport, window height). Used to
  // design the keyboard handling from measurements instead of guesses.
  useEffect(() => {
    if (typeof window === 'undefined' || !window.location.hash.includes('vvdebug')) return undefined;
    const box = document.createElement('pre');
    box.setAttribute('data-testid', 'vv-debug');
    box.style.cssText = 'position:fixed;top:0;left:0;z-index:99999;margin:0;padding:4px 6px;font:11px/1.3 monospace;background:rgba(0,0,0,.75);color:#0f0;pointer-events:none;white-space:pre';
    document.body.appendChild(box);
    const vv = window.visualViewport;
    const nav = window.navigator as Navigator & { standalone?: boolean };
    const render = () => {
      const active = document.activeElement;
      box.textContent = [
        `standalone=${String(nav.standalone ?? 'n/a')} ua=${/iPhone|iPad/.test(navigator.userAgent) ? 'ios' : 'other'}`,
        `inner=${window.innerWidth}x${window.innerHeight} screen=${window.screen.width}x${window.screen.height}`,
        `vv=${vv ? `${Math.round(vv.width)}x${Math.round(vv.height)} top=${Math.round(vv.offsetTop)} scale=${vv.scale.toFixed(2)}` : 'none'}`,
        `scrollY=${Math.round(window.scrollY)} active=${active ? active.tagName.toLowerCase() : '-'}`,
      ].join('\n');
    };
    render();
    const timer = window.setInterval(render, 250);
    vv?.addEventListener('resize', render);
    vv?.addEventListener('scroll', render);
    return () => {
      window.clearInterval(timer);
      vv?.removeEventListener('resize', render);
      vv?.removeEventListener('scroll', render);
      box.remove();
    };
  }, []);


  function shownChatTurnId(message: ChatHomeMessage): string | undefined {
    const turnId = message.turnId || message.result?.turn_id;
    return typeof turnId === 'string' && turnId.trim() ? turnId.trim() : undefined;
  }

  async function ensureChatTurnLoaded(turnId: string) {
    const shown = [...earlierChatMessagesRef.current, ...chatHomeMessagesRef.current];
    if (shown.some((message) => shownChatTurnId(message) === turnId)) return;
    const sessionEpoch = privateSessionEpochRef.current;
    const chatGeneration = chatHomeGenerationRef.current;
    try {
      const history = await fetchVNextChatHistory(initData, { turn: turnId });
      if (!isPrivateSessionEpochCurrent(sessionEpoch) || chatGeneration !== chatHomeGenerationRef.current) return;
      const incoming = chatHomeMessagesFromServerHistory(history);
      if (!incoming.some((message) => shownChatTurnId(message) === turnId)) return;
      const currentShown = [...earlierChatMessagesRef.current, ...chatHomeMessagesRef.current];
      const known = new Set(currentShown.map((message) => message.id));
      const oldest = currentShown.reduce((min, message) => (
        typeof message.timestamp === 'number' && message.timestamp > 0 ? Math.min(min, message.timestamp) : min
      ), Number.POSITIVE_INFINITY);
      const extra = incoming
        .filter((message) => !known.has(message.id))
        .filter((message) => typeof message.timestamp === 'number' && message.timestamp < oldest)
        .sort((left, right) => (left.timestamp ?? 0) - (right.timestamp ?? 0));
      if (extra.length === 0) return;
      setEarlierChatMessages((current) => {
        const ids = new Set(current.map((message) => message.id));
        return [...extra.filter((message) => !ids.has(message.id)), ...current];
      });
    } catch {
      // The cite stays visible on the chat even when the older window cannot load.
    }
  }

  function openLineageChatMessage(cite: LineageChatCite) {
    setChatCiteFocus({ ref: cite.ref, token: cite.token });
    navigate('home');
    void ensureChatTurnLoaded(cite.turnId);
  }

  function navigate(nextView: ViewId, nextSelectedId: string | null = selectedId) {
    const requestedSelectedId = nextView === 'work' ? nextSelectedId : null;
    const resolvedSelectedId = nextView === 'work' && !requestedSelectedId && queue
      ? firstPriorityItemId(queue.items)
      : requestedSelectedId;
    // No-op navigation (e.g. tapping the already-active nav item): skip entirely.
    // startViewTransition animates even when the callback changes no DOM, so
    // without this guard an unchanged click would flicker a cross-fade.
    if (nextView === view && resolvedSelectedId === selectedId) {
      return;
    }
    runViewTransition(() => {
      setView(nextView);
      if (nextView === 'home') setChatSheetOpen(false);
      if (resolvedSelectedId !== selectedId) {
        setSelectedId(resolvedSelectedId);
      }
      setRequestedItemId(requestedSelectedId);
    });
    writeVNextLocationState(window, { view: nextView, itemId: resolvedSelectedId });
  }

  useEffect(() => {
    setSourceDetail(null);
    setSourceDetailBusy(false);
    setArmedAction(null);
    setArmedActionItemId(null);
    setExecutingAction(null);
    setItemProjection(null);
    setLastItemCommand(null);
  }, [selectedId]);

  async function openSourceDetail() {
    if (!selected) return;
    const target = selected;
    const sourceItemId = mandateMatchWorkItemId(target);
    if (sourceItemId) {
      setSelectedId(sourceItemId);
      writeVNextLocationState(window, { view: 'work', itemId: sourceItemId });
      setSourceDetail(null);
      return;
    }
    const sessionEpoch = privateSessionEpochRef.current;
    setSourceDetailBusy(true);
    setItemActionMessage(target.id, '');
    try {
      const result = await openSelectedSourceDetail({ item: target, initData, csrfToken: session?.csrf_token });
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      if (result.kind === 'detail') {
        const closedTaskStatus = taskDetailClosedStatus(result.detail);
        if (closedTaskStatus && target.status.toLowerCase() !== closedTaskStatus) {
          const nextOptimisticStatuses = { ...optimisticStatusByItemId, [target.id]: closedTaskStatus };
          setOptimisticStatusByItemId(nextOptimisticStatuses);
          updateQueueItemStatus(target.id, closedTaskStatus);
          updateChatHomeWorkItemStatus(target.id, closedTaskStatus);
          setRetiredActionItemIds((current) => ({ ...current, [target.id]: true }));
          const refreshedQueue = await refreshQueue(target.id, nextOptimisticStatuses, sessionEpoch);
          if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
          const targetStillVisible = Boolean(refreshedQueue?.items.some((item) => item.id === target.id));
          if (!targetStillVisible) {
            return;
          }
        }
        setSourceDetail(result.detail);
      } else {
        setItemActionMessage(target.id, result.message);
        return;
      }
    } catch (detailError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      const fallbackDetail = sourceDetailPayloadFromOpenError(detailError, target, errorLocale);
      if (fallbackDetail) {
        setSourceDetail(fallbackDetail);
        return;
      }
      setItemActionMessage(target.id, displayError(detailError, t('error.fallback.couldNotOpenDetail', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setSourceDetailBusy(false);
    }
  }

  async function openChatHomeWorkDetail(target: OwnerWorkItem): Promise<ChatHomeWorkDetailResult> {
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      let csrfToken = session?.csrf_token;
      const runOpen = (token: string | undefined) => openSelectedSourceDetail({
        item: target,
        initData,
        csrfToken: token,
      });
      let result: Awaited<ReturnType<typeof openSelectedSourceDetail>>;
      try {
        result = await runOpen(csrfToken);
      } catch (firstError) {
        if (!isRefreshableSessionActionError(firstError)) throw firstError;
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
          return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale) };
        }
        result = await runOpen(refreshed?.csrf_token);
      }
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
        return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale) };
      }
      if (result.kind === 'message') {
        return { ok: false, message: result.message };
      }
      const closedTaskStatus = taskDetailClosedStatus(result.detail);
      if (closedTaskStatus && target.status.toLowerCase() !== closedTaskStatus) {
        const nextOptimisticStatuses = { ...optimisticStatusByItemId, [target.id]: closedTaskStatus };
        setOptimisticStatusByItemId(nextOptimisticStatuses);
        updateQueueItemStatus(target.id, closedTaskStatus);
        updateChatHomeWorkItemStatus(target.id, closedTaskStatus);
        setRetiredActionItemIds((current) => ({ ...current, [target.id]: true }));
        void refreshQueue(target.id, nextOptimisticStatuses, sessionEpoch).catch(() => {
          // The opened detail remains usable if the queue refresh misses.
        });
      }
      return { ok: true, detail: buildSourceDetailView(result.detail, errorLocale, userTimezone) };
    } catch (detailError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
        return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale) };
      }
      const fallbackDetail = sourceDetailPayloadFromOpenError(detailError, target, errorLocale);
      if (fallbackDetail) {
        return { ok: true, detail: buildSourceDetailView(fallbackDetail, errorLocale, userTimezone) };
      }
      return { ok: false, message: displayError(detailError, t('error.fallback.couldNotOpenDetail', errorLocale), errorLocale) };
    }
  }

  async function executeConfirmedItemAction(target: OwnerWorkItem, action: VNextItemAction): Promise<VNextActionExecutionResult> {
    const sessionEpoch = privateSessionEpochRef.current;
    setExecutingAction(action.id);
    setHostStepUp((current) => (current?.itemId === target.id ? null : current));
    setItemActionMessage(target.id, '');
    setWorkNotice('');
    try {
      let csrfToken = session?.csrf_token;
      if (!csrfToken && session?.authenticated) {
        const refreshed = await refreshSessionForAction(sessionEpoch);
        csrfToken = refreshed?.csrf_token;
      }
      const runAction = (token: string | undefined) => executeSelectedItemAction({
        item: target,
        action,
        initData,
        csrfToken: token,
        locale: errorLocale,
      });
      let successMessage: string;
      try {
        successMessage = (await runAction(csrfToken)).owner_message;
      } catch (firstError) {
        if (!isRefreshableSessionActionError(firstError)) throw firstError;
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
          return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale), nextStatus: null };
        }
        successMessage = (await runAction(refreshed?.csrf_token)).owner_message;
      }
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
        return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale), nextStatus: null };
      }
      const nextStatus = optimisticStatusForItemAction(action.id, target);
      // Merge inside the updater: two overlapping item actions used to clobber
      // each other's optimistic status through the render-time closure.
      let nextOptimisticStatuses = optimisticStatusByItemId;
      if (nextStatus) {
        setOptimisticStatusByItemId((current) => {
          nextOptimisticStatuses = { ...current, [target.id]: nextStatus };
          return nextOptimisticStatuses;
        });
        updateQueueItemStatus(target.id, nextStatus);
        updateChatHomeWorkItemStatus(target.id, nextStatus);
      }
      setItemActionMessage(target.id, successMessage);
      appendWorkActionResultToChat(target, action, successMessage);
      setArmedAction(null);
      setArmedActionItemId(null);
      const refreshedQueue = await refreshQueue(target.id, nextOptimisticStatuses, sessionEpoch);
      const targetStillVisible = Boolean(refreshedQueue?.items.some((item) => item.id === target.id));
      if (refreshedQueue && !targetStillVisible) {
        setItemActionMessage(target.id, successMessage);
        setWorkNotice(successMessage);
      }
      return { ok: true, message: successMessage, nextStatus };
    } catch (actionError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
        return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale), nextStatus: null };
      }
      const message = displayError(actionError, t('error.fallback.actionFailed', errorLocale), errorLocale);
      setArmedAction(null);
      setArmedActionItemId(null);
      if (offerLocalHostStepUp(session?.email, actionError instanceof ApiResponseError ? actionError.problem : null)) {
        setHostStepUp({ itemId: target.id, action });
        setItemActionMessage(target.id, message);
        return { ok: false, message, nextStatus: null };
      }
      setHostStepUp((current) => (current?.itemId === target.id ? null : current));
      if (isStaleItemActionError(actionError)) {
        // ChatHomeView's isStaleWorkRefreshMessage() matches this exact catalog
        // value (en + zh) to pick the 'hint' presentation.
        const refreshedMessage = t('app.action.itemChangedRefreshedQueue', errorLocale);
        setRetiredActionItemIds((current) => ({ ...current, [target.id]: true }));
        try {
          const refreshedQueue = await refreshQueue(target.id, optimisticStatusByItemId, sessionEpoch);
          const targetStillVisible = Boolean(refreshedQueue?.items.some((item) => item.id === target.id));
          setItemActionMessage(target.id, refreshedMessage);
          if (!targetStillVisible) setWorkNotice(refreshedMessage);
          return { ok: false, message: refreshedMessage, nextStatus: null, itemUnavailable: !targetStillVisible };
        } catch {
          setItemActionMessage(target.id, message);
          return { ok: false, message, nextStatus: null };
        }
      }
      setItemActionMessage(target.id, message);
      return { ok: false, message, nextStatus: null };
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) {
        setExecutingAction(null);
      }
    }
  }

  async function confirmHostStepUp() {
    if (!hostStepUp || !session?.csrf_token) return;
    const target = queue?.items.find((item) => item.id === hostStepUp.itemId)
      ?? (selected?.id === hostStepUp.itemId ? selected : null);
    if (!target) return;
    setHostStepUpBusy(true);
    try {
      await confirmLocalHostStepUp(session.csrf_token);
      await executeConfirmedItemAction(target, hostStepUp.action);
    } catch (error) {
      setItemActionMessage(target.id, displayError(error, t('error.fallback.actionFailed', errorLocale), errorLocale));
    } finally {
      setHostStepUpBusy(false);
    }
  }

  async function executeChatNoticeAction(action: DeliveryNoticeAction): Promise<{ ok: boolean; message: string }> {
    if (!action.directTaskAction) {
      void submitChatHomeCommand(action.command, undefined, undefined, 'text', action.label);
      return { ok: true, message: t('app.action.sentToAegis', errorLocale) };
    }
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      let csrfToken = session?.csrf_token;
      if (!csrfToken && session?.authenticated) {
        const refreshed = await refreshSessionForAction(sessionEpoch);
        csrfToken = refreshed?.csrf_token;
      }
      const runAction = (token: string | undefined) => updateConsoleTaskAction(
        action.directTaskAction?.taskId ?? '',
        action.directTaskAction?.action ?? 'complete',
        initData,
        token,
        action.directTaskAction?.preset,
      );
      let result;
      try {
        result = await runAction(csrfToken);
      } catch (firstError) {
        if (!isRefreshableSessionActionError(firstError)) throw firstError;
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
          return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale) };
        }
        result = await runAction(refreshed?.csrf_token);
      }
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) {
        return { ok: false, message: t('app.action.signedOutBeforeFinished', errorLocale) };
      }
      await refreshQueue(undefined, optimisticStatusByItemId, sessionEpoch);
      return {
        ok: true,
        message: ownerTaskActionReceipt({
          action: result.action,
          dueAt: result.due_at,
          recurring: result.recurring,
        }, errorLocale, userTimezone),
      };
    } catch (error) {
      return { ok: false, message: displayError(error, t('error.fallback.reminderActionFailed', errorLocale), errorLocale) };
    }
  }

  async function refreshSessionForAction(sessionEpoch: number): Promise<SessionInfo | null> {
    const refreshed = await fetchSession(initData);
    if (isPrivateSessionEpochCurrent(sessionEpoch)) {
      setSession(refreshed);
    }
    return refreshed.authenticated ? refreshed : null;
  }

  function isRefreshableSessionActionError(error: unknown): boolean {
    if (!(error instanceof ApiResponseError)) return false;
    if (error.statusCode !== 403) return false;
    if (error.problem?.error === 'mfa_required' || error.problem?.status === 'mfa_required') return false;
    return /csrf|session|permission|forbidden/i.test(error.message);
  }

  function isStaleItemActionError(error: unknown): boolean {
    if (!error || typeof error !== 'object') return false;
    const candidate = error as { statusCode?: unknown; message?: unknown };
    const statusCode = typeof candidate.statusCode === 'number' ? candidate.statusCode : null;
    const message = typeof candidate.message === 'string' ? candidate.message : '';
    return statusCode !== null
      && statusCode >= 400
      && statusCode < 500
      && /action does not apply to this item|work item not found/i.test(message);
  }

  async function executeItemAction(target: OwnerWorkItem, action: VNextItemAction) {
    if (armedAction !== action.id || armedActionItemId !== target.id) {
      setArmedAction(action.id);
      setArmedActionItemId(target.id);
      setItemActionMessage(target.id, itemActionConfirmationHint(action, errorLocale));
      return;
    }
    await executeConfirmedItemAction(target, action);
  }

  useEffect(() => {
    let cancelled = false;
    setContext(null);
    setAssistantResult(null);
    setItemProjection(null);
    if (!selected || !session?.authenticated) return;
    const focus = selected;
    async function loadContext() {
      const [contextResult, itemResult] = await Promise.allSettled([
        fetchOwnerWorkContext(initData, session?.csrf_token, focus.id, 'explain'),
        fetchVNextItem(initData, focus.id),
      ]);
      if (cancelled) return;
      setContext(contextResult.status === 'fulfilled' ? contextResult.value : null);
      setItemProjection(itemResult.status === 'fulfilled' ? itemResult.value : null);
    }
    void loadContext();
    return () => {
      cancelled = true;
    };
  }, [initData, selected?.id, session?.authenticated, session?.csrf_token]);

  async function runAssistant(
    intent: 'chat' | 'explain' | 'draft',
    text = '',
    focusIdOverride?: string,
    retryIdempotencyKey?: string,
    conversationContext: VNextConversationContextTurn[] = [],
  ) {
    const focusId = focusIdOverride ?? selected?.id;
    if (!focusId) return false;
    const sessionEpoch = privateSessionEpochRef.current;
    const commandIdempotencyKey = retryIdempotencyKey || chatHomeIdempotencyKey();
    setLastItemCommand({ intent, text, itemId: focusId, idempotencyKey: retryIdempotencyKey, conversationContext });
    setBusy(true);
    const pendingResult = buildWorkingAssistantResult(text, errorLocale);
    setAssistantResultItemId(focusId);
    setAssistantResult(pendingResult);
    try {
      const result = await requestAssistantResultWithAutoCheck({
        initData,
        csrfToken: session?.csrf_token,
        focusId,
        intent,
        text,
        scope: 'item',
        idempotencyKey: commandIdempotencyKey,
        conversationContext,
        onStillWorking: () => {
          if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
          setAssistantResultItemId(focusId);
          setAssistantResult({
            ...pendingResult,
            // Localized still-working text (the util emits an English default;
            // we substitute the display_language version so zh sessions stay zh).
            visible_answer: t('chatHome.working.stillChecking', errorLocale),
          });
        },
      });
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return false;
      setAssistantResultItemId(focusId);
      setAssistantResult(result);
      void refreshQueueAfterAssistant(focusId);
      return true;
    } catch (assistantError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return false;
      const retryKey = assistantError instanceof ApiResponseError && assistantError.statusCode === 504
        ? commandIdempotencyKey
        : undefined;
      setLastItemCommand({
        intent,
        text,
        itemId: focusId,
        idempotencyKey: retryKey,
        retryLabel: retryKey ? 'Check result' : 'Try again',
        conversationContext,
      });
      const result: ContextualAssistantResponse = {
        schema_version: 1,
        response_kind: 'error',
        visible_answer: assistantDisplayError(assistantError, errorLocale),
        receipt_expectation: t('app.receipt.noActionTaken', errorLocale),
        citations: [],
        warnings: [],
        severity: 'warning',
      };
      setAssistantResultItemId(focusId);
      setAssistantResult(result);
      return false;
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setBusy(false);
    }
  }

  // Slice 5: upload a selected file via the upload endpoint and hold it as the
  // composer's pending attachment. Single-attachment: a new upload replaces any
  // existing pending one. Errors surface in the pill (never throws to the UI).
  async function handleChatHomeAttachFile(file: File) {
    setChatHomeAttachmentError('');
    setChatHomeAttachmentUploading(true);
    try {
      const result = await uploadVNextAttachment(initData, session?.csrf_token, file);
      // The file picker only offers images and PDFs; a `voice` result here
      // would mean the owner picked an audio file, which this path does not
      // carry (voice goes through the recorder).
      if (result.type === 'voice' || !result.data_url) {
        setChatHomePendingAttachment(null);
        setChatHomeAttachmentError(t('chatHome.composer.uploadFailed', errorLocale));
        return;
      }
      setChatHomePendingAttachment({
        kind: result.type,
        dataUrl: result.data_url,
        filename: result.filename,
        mimeType: result.mime_type,
      });
      // A newly attached file replaces any held voice note (one per turn).
      setChatHomeVoiceNote(IDLE_VOICE_NOTE);
    } catch (error) {
      setChatHomePendingAttachment(null);
      // Never a silent drop: show the server's reason when it gave one.
      setChatHomeAttachmentError(
        displayError(error, t('chatHome.composer.uploadFailed', errorLocale), errorLocale),
      );
    } finally {
      setChatHomeAttachmentUploading(false);
    }
  }

  function clearChatHomePendingAttachment() {
    setChatHomePendingAttachment(null);
    setChatHomeAttachmentError('');
  }

  // Convert whatever the composer is holding — a recorded voice note, or an
  // uploaded image/document — into the wire payload. Returns [] when nothing
  // is attached. A voice note wins: recording clears any file attachment.
  function chatHomeCommandAttachments(
    attachment: ChatHomePendingAttachment | null,
    voiceNote: VoiceNoteState = chatHomeVoiceNote,
  ): VNextCommandAttachment[] {
    const voice = voiceNoteAttachment(voiceNote);
    if (voice) {
      return [{
        type: 'voice',
        transcript: voice.transcript,
        ...(voice.durationSecs !== null
          ? { duration_secs: Math.max(0, Math.round(voice.durationSecs)) }
          : {}),
      }];
    }
    if (!attachment) return [];
    if (attachment.kind === 'image') {
      return [{ type: 'image', data_url: attachment.dataUrl }];
    }
    return [{
      type: 'document',
      data_url: attachment.dataUrl,
      filename: attachment.filename,
      mime_type: attachment.mimeType,
    }];
  }

  /**
   * Does the composer hold something sendable with no typed text?
   *
   * Every empty-text gate on the send path consults this. Before, an
   * image-only send hit `if (!text) return` in four places and vanished with
   * no feedback at all — the owner's "附件没反应" bug. The backend derives the
   * message text from the attachment (voice transcript / image caption /
   * document filename), so an empty `input` is legitimate here.
   */
  function chatHomeHasSendableAttachment(): boolean {
    return chatHomeVoiceNote.status === 'attached' || Boolean(chatHomePendingAttachment);
  }

  /** Owner-visible descriptor of what a sent turn carried, for its bubble. */
  function chatHomeSentAttachment(
    attachments: VNextCommandAttachment[],
  ): ChatHomeMessageAttachment | undefined {
    const first = attachments[0];
    if (!first) return undefined;
    if (first.type === 'voice') {
      return {
        kind: 'voice',
        transcript: first.transcript,
        durationSecs: typeof first.duration_secs === 'number' ? first.duration_secs : null,
      };
    }
    if (first.type === 'image') return { kind: 'image', dataUrl: first.data_url };
    return { kind: 'document', filename: first.filename };
  }

  async function submitChatHomeCommand(
    prompt?: string,
    retryIdempotencyKey?: string,
    contextOverride?: ChatHomeAttachedContext,
    modeOverride: 'text' | 'voice' = 'text',
    displayTextOverride?: string,
    queuedOwnerMessageId?: string,
    quoteOverride?: ChatHomeQuote | null,
    // A note that just finished transcribing: the caller's render still holds
    // the pre-upload state, so the attachment rides in explicitly.
    voiceNoteOverride?: VoiceNoteState,
    // A drained queue entry brings the attachments captured when it was queued.
    queuedAttachments?: VNextCommandAttachment[],
  ): Promise<SecretaryAskResult> {
    const text = (prompt ?? chatHomeCommand).trim();
    const voiceNoteForTurn = voiceNoteOverride ?? chatHomeVoiceNote;
    // Attachment-only turns are legitimate (see chatHomeHasSendableAttachment).
    if (!text && !chatHomeHasSendableAttachment() && !voiceNoteOverride) return { kind: 'empty' };
    if (busy && !queuedOwnerMessageId) {
      const queueAttachments = !contextOverride && !chatHomeAttachedContext && !retryIdempotencyKey
        ? chatHomeCommandAttachments(chatHomePendingAttachment, voiceNoteForTurn)
        : [];
      queueChatHomeCommand(prompt, contextOverride, modeOverride, displayTextOverride, retryIdempotencyKey, quoteOverride, queueAttachments);
      return { kind: 'queued' };
    }
    // A voice note with no typed caption shows the owner his own words.
    const displayText = displayTextOverride?.trim()
      || text
      || voiceNoteAttachment(voiceNoteForTurn)?.transcript.trim()
      || '';
    const requestContext = contextOverride ?? chatHomeAttachedContext;
    const requestQuote = quoteOverride !== undefined ? quoteOverride : chatHomeQuotedMessage;
    // Slice 5: attachments ride only the general-chat path (no focus context and
    // not a retry, which replays the original text without the now-cleared file).
    // A drained queued turn carries only what was queued: never pick up an
    // attachment the owner added afterwards for the next message.
    const requestAttachments = queuedOwnerMessageId
      ? (queuedAttachments ?? [])
      : !requestContext && !retryIdempotencyKey
        ? chatHomeCommandAttachments(chatHomePendingAttachment, voiceNoteForTurn)
        : [];
    const sentAttachment = chatHomeSentAttachment(requestAttachments);
    // The transcript IS the message. The wire `text` must carry it: an empty
    // text used to be replaced client-side by the item-page "explain this"
    // prompt, so the runtime answered a question the owner never asked.
    const runtimeText = text
      || (sentAttachment?.kind === 'voice' ? sentAttachment.transcript.trim() : '');
    const sessionEpoch = privateSessionEpochRef.current;
    const chatHomeGeneration = chatHomeGenerationRef.current;
    const commandIdempotencyKey = retryIdempotencyKey || chatHomeIdempotencyKey();
    const screenContext = buildCeoScreenContext();
    // Same server cap as the panel path (screen turn + newest history).
    const conversationContext = mergeConversationContext(
      buildPanelConversationContext({ screenContext, stream: [] }),
      chatHomeConversationContext(requestQuote),
    );
    const clientContext = requestContext
      ? 'item' as const
      : mapViewToClientContext(view, selectedId);
    const ownerId = `owner:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    const workingId = `working:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    const ownerTimestamp = Date.now();
    const updateWorkingMessage = (nextText: string) => {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
      setChatHomeMessages((current) => current.map((message) => (
        message.id === workingId ? { ...message, text: nextText } : message
      )));
    };
    const liveProgressLines: string[] = [];
    let keepPageOpenHint = false;
    const keepPageOpenText = t(CHAT_HOME_WORKING_LONG_KEY, errorLocale);
    const workingFallback = t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale);
    const paintWorkingProgress = (nextLive?: string) => {
      if (nextLive) {
        const next = appendProgressLine(liveProgressLines, nextLive);
        liveProgressLines.splice(0, liveProgressLines.length, ...next);
      }
      updateWorkingMessage(composeWorkingText(
        liveProgressLines,
        keepPageOpenHint,
        keepPageOpenText,
        liveProgressLines.length === 0 ? workingFallback : liveProgressLines.join('\n'),
      ));
    };
    // Composer state belongs to the owner's *current* message. When this call
    // drains a queued turn, the composer may already hold the next message
    // ("Aegis 处理时可继续输入…"): clearing it here wiped that text (2026-09-06
    // evidence run: a typed turn vanished when the previous queued turn drained).
    if (!queuedOwnerMessageId) {
      setChatHomeCommand('');
      setChatHomeInputMode('text');
      if (!contextOverride) {
        setChatHomeAttachedContext(null);
      }
      // Clear the pending attachment once it has been captured for this turn.
      if (requestAttachments.length > 0) {
        setChatHomePendingAttachment(null);
        setChatHomeAttachmentError('');
        setChatHomeVoiceNote(IDLE_VOICE_NOTE);
      }
      setChatHomeQuotedMessage(null);
      setSnapshotMessage('');
    }
    setChatHomeNotice('');
    setChatHomeMessages((current) => {
      const ownerMessage = {
        id: ownerId,
        turnId: commandIdempotencyKey,
        role: 'owner',
        text: displayText,
        timestamp: ownerTimestamp,
        ...(requestQuote ? { quote: requestQuote } : {}),
        ...(requestContext ? { contextLabel: requestContext.label } : {}),
        ...(sentAttachment ? { attachment: sentAttachment } : {}),
      } as ChatHomeMessage;
      let foundQueuedMessage = false;
      const next = queuedOwnerMessageId
        ? current.map((message) => {
          if (message.id !== queuedOwnerMessageId) return message;
          foundQueuedMessage = true;
          return { ...message, role: 'owner' as const, turnId: commandIdempotencyKey };
        })
        : current;
      return [
        ...next,
        ...((queuedOwnerMessageId && foundQueuedMessage) || next.some((message) => message.role === 'owner' && message.turnId === commandIdempotencyKey) ? [] : [ownerMessage]),
        { id: workingId, turnId: commandIdempotencyKey, role: 'working', text: t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale), timestamp: Date.now() },
      ];
    });
    setBusy(true);
    let turnResult: SecretaryAskResult = { kind: 'skipped' };
    const slowTimer = window.setTimeout(() => {
      if (liveProgressLines.length === 0) {
        paintWorkingProgress(t(CHAT_HOME_WORKING_SLOW_KEY, errorLocale));
      }
    }, 2500);
    const longTimer = window.setTimeout(() => {
      keepPageOpenHint = true;
      paintWorkingProgress();
    }, 25000);
    // Live per-turn progress (Slice 3): mint a short-lived stream token for this
    // turn (= commandIdempotencyKey) and open the SSE stream so each progress
    // frame updates the working-message text in place. Advisory + best-effort:
    // any mint/network failure is swallowed and the working message stays
    // generic (today's behavior). The FINAL result still arrives via the
    // auto-check path below — this stream never delivers the answer.
    //
    // `turnStream.settled` lets `finally` tear the stream down even if the mint
    // promise resolves later (fast reply), and lets the mint resolution skip
    // opening a stream the turn has already finished. updateWorkingMessage is
    // epoch/generation-guarded, so a stale turn's frames can never clobber a
    // newer turn's text, and the busy/epoch model is untouched.
    const turnStream: { close: (() => void) | null; settled: boolean } = { close: null, settled: false };
    let turnOpenedApproval = false;
    let turnExecutorLabel: string | null = null;
    const turnStartedAt = ownerTimestamp;
    void mintVNextTurnStreamToken(initData, session?.csrf_token, commandIdempotencyKey)
      .then((streamToken) => {
        if (turnStream.settled) return;
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
        turnStream.close = openTurnStream(
          commandIdempotencyKey,
          streamToken,
          (event: TurnEvent) => {
            switch (event.type) {
              case 'tool_start': {
                const footer = executorFooterLabel(event.tool_name, errorLocale);
                if (footer) turnExecutorLabel = footer;
                paintWorkingProgress(workingPhraseForTool(event.tool_name, errorLocale));
                break;
              }
              case 'tool_done':
                paintWorkingProgress(toolDonePhrase(errorLocale));
                break;
              case 'vision_routed':
                paintWorkingProgress(t('chatHome.working.processingImage', errorLocale));
                break;
              case 'intent_resolved':
                // CEO Intent Contract: stream "我理解成：…" into the working bubble.
                paintWorkingProgress(
                  event.needs_review
                    ? tf('chatHome.working.intentReview', errorLocale, { label: event.label })
                    : tf('chatHome.working.intent', errorLocale, { label: event.label }),
                );
                break;
              case 'action_phase': {
                const footer = executorFooterLabel(event.tool_name || event.detail || '', errorLocale);
                if (footer) turnExecutorLabel = footer;
                paintWorkingProgress(workingPhraseForPhase(event.phase, event.tool_name, event.detail, errorLocale));
                break;
              }
              case 'owner_notice':
                if (event.text.trim()) paintWorkingProgress(workingNotice(event.text.trim(), event.kind, errorLocale));
                break;
              case 'need_approval':
                // Slice 4: render an in-chat approval CARD with the streamed
                // summary excerpt (≤500 chars; never raw html/shell) + buttons.
                // Independent of the working-message lifecycle, so it stays
                // visible after the turn settles. Epoch/generation-guarded like
                // every other chatHomeMessages mutation so a stale turn's frame
                // can't leak a card into a newer session.
                if (!event.approval_id) break;
                if (!isPrivateSessionEpochCurrent(sessionEpoch)) break;
                if (chatHomeGenerationRef.current !== chatHomeGeneration) break;
                turnOpenedApproval = true;
                {
                  const approvalId = event.approval_id;
                  const toolName = event.tool_name;
                  const summary = event.summary;
                  setChatHomeMessages((current) => appendApprovalMessage(current, {
                    approvalId,
                    toolName,
                    summary,
                  }));
                }
                paintWorkingProgress(workingPhraseForPhase('approval_required', event.tool_name, null, errorLocale));
                break;
              case 'assistant_delta':
                if (!event.text) break;
                if (!isPrivateSessionEpochCurrent(sessionEpoch)) break;
                if (chatHomeGenerationRef.current !== chatHomeGeneration) break;
                setChatHomeMessages((current) => current.map((message) => (
                  message.id === workingId
                    ? { ...message, streamText: appendAssistantDelta(message.streamText || '', event.text) }
                    : message
                )));
                break;
              case 'assistant_draft_reset':
                if (!isPrivateSessionEpochCurrent(sessionEpoch)) break;
                if (chatHomeGenerationRef.current !== chatHomeGeneration) break;
                setChatHomeMessages((current) => current.map((message) => (
                  message.id === workingId ? { ...message, streamText: '' } : message
                )));
                break;
              // done/error close the stream via openTurnStream's onClose.
              default:
                break;
            }
          },
          () => {
            turnStream.close = null;
          },
        );
        // The turn settled while the token was minting (fast reply): tear down.
        if (turnStream.settled) {
          turnStream.close?.();
          turnStream.close = null;
        }
      })
      .catch(() => {
        // Advisory stream unavailable — degrade silently to the generic working
        // message. The final result path is unaffected.
      });
    try {
      // CSRF/session self-heal — mirror the work action path
      // (`executeConfirmedItemAction`). The home chat command POST is gated by
      // the same per-session CSRF token; if `session.csrf_token` is stale or
      // absent (e.g. the browser/PWA resumed across a backend restart or the
      // session rotated), the POST 403s. Unlike the work actions, this path
      // previously had no recovery, so EVERY chat turn surfaced the generic
      // "I couldn't finish that" while other actions kept working. Pre-emptively refresh
      // a missing token, and retry once on a refreshable 403, before failing.
      let commandCsrfToken = session?.csrf_token;
      // Only web-console sessions are CSRF-gated; Telegram-authenticated sessions
      // have no csrf_token and don't need one (initData auth), so pre-refreshing
      // for them is a needless GET + failure point (Codex). A stale (present)
      // token is still healed by the 403 retry below regardless of principal.
      if (!commandCsrfToken && session?.authenticated && session?.principal_type === 'web_console') {
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return turnResult;
        // The conversation may have been cleared while the refresh was in
        // flight (clearChatHomeConversation advances the generation, not the
        // epoch); don't POST a turn the user already discarded — else its
        // backend work items / tool calls / push still fire (Codex).
        if (chatHomeGenerationRef.current !== chatHomeGeneration) return turnResult;
        commandCsrfToken = refreshed?.csrf_token;
      }
      const runCommand = (token: string | undefined) => requestAssistantCommandWithAutoCheck({
        initData,
        csrfToken: token,
        focusId: requestContext?.itemId ?? GENERAL_ASSISTANT_FOCUS_ID,
        intent: 'chat',
        text: runtimeText,
        scope: requestContext ? 'item' : 'general',
        clientContext,
        mode: modeOverride,
        idempotencyKey: commandIdempotencyKey,
        conversationContext,
        attachments: requestAttachments,
        // The util emits an English still-working default; substitute the
        // display_language version so zh sessions never see an English flash.
        onStillWorking: () => {
          if (liveProgressLines.length === 0) {
            paintWorkingProgress(t('chatHome.working.stillChecking', errorLocale));
          }
        },
      });
      let command: Awaited<ReturnType<typeof runCommand>>;
      try {
        command = await runCommand(commandCsrfToken);
      } catch (firstCommandError) {
        if (!isRefreshableSessionActionError(firstCommandError)) throw firstCommandError;
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return turnResult;
        if (chatHomeGenerationRef.current !== chatHomeGeneration) return turnResult;
        command = await runCommand(refreshed?.csrf_token);
      }
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return turnResult;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return turnResult;
      const result = command.response;
      const heldSearch = turnOpenedApproval && isGatePendingOwnerAck(result.visible_answer || '');
      if (heldSearch) {
        setChatHomeMessages((current) => current.filter((message) => message.id !== workingId));
        void refreshQueueAfterAssistant(command.workItem?.id ?? requestContext?.itemId);
        turnResult = { kind: 'replied', text: '' };
      } else {
      setChatHomeMessages((current) => {
        const withoutWorking = current.filter((message) => message.id !== workingId);
        const finishedAt = Date.now();
        return completeChatHomeTurn(withoutWorking, displayText, ownerTimestamp, {
          id: `aegis:${Date.now()}:${Math.random().toString(36).slice(2)}`,
          turnId: result.turn_id || commandIdempotencyKey,
          replyOrigin: 'command',
          role: 'aegis' as const,
          text: result.visible_answer,
          severity: result.severity ?? null,
          timestamp: finishedAt,
          latencyMs: typeof result.latency_ms === 'number' && result.latency_ms >= 0
            ? result.latency_ms
            : Math.max(0, finishedAt - turnStartedAt),
          result,
          workItem: command.workItem,
          ...(turnExecutorLabel
            ? { executorFooter: formatExecutorFooter(turnExecutorLabel, finishedAt - turnStartedAt, errorLocale) }
            : {}),
        });
      });
      void refreshQueueAfterAssistant(command.workItem?.id ?? requestContext?.itemId);
      turnResult = { kind: 'replied', text: result.visible_answer || '' };
      }
    } catch (assistantError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return turnResult;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return turnResult;
      const result: ContextualAssistantResponse = {
        schema_version: 1,
        response_kind: 'error',
        visible_answer: assistantDisplayError(assistantError, errorLocale),
        receipt_expectation: t('app.receipt.noActionTaken', errorLocale),
        citations: [],
        warnings: [],
        severity: 'warning',
      };
      const retryKey = assistantError instanceof ApiResponseError && assistantError.statusCode === 504
        ? commandIdempotencyKey
        : undefined;
      setChatHomeMessages((current) => {
        const withoutWorking = current.filter((message) => message.id !== workingId);
        if (withoutWorking.some((message) => message.role === 'aegis' && message.turnId === commandIdempotencyKey)
          || hasChatHomeAegisAfterOwner(withoutWorking, displayText, ownerTimestamp)) {
          return withoutWorking;
        }
        return withoutWorking.concat({
          id: `aegis:${Date.now()}:${Math.random().toString(36).slice(2)}`,
          turnId: commandIdempotencyKey,
          replyOrigin: 'transport-error',
          role: 'aegis' as const,
          text: result.visible_answer,
          severity: result.severity ?? null,
          timestamp: Date.now(),
          result,
          retryText: text,
          retryIdempotencyKey: retryKey,
          retryLabel: retryKey ? 'Check result' : 'Try again',
          ...(requestContext ? { retryContext: requestContext } : {}),
          ...(requestQuote ? { retryQuote: requestQuote } : {}),
        });
      });
      turnResult = { kind: 'error', text: result.visible_answer };
    } finally {
      window.clearTimeout(slowTimer);
      window.clearTimeout(longTimer);
      // Tear down the live-progress stream with the turn. `settled` guards the
      // race where the mint promise resolves AFTER the turn finished (it then
      // skips opening, or closes immediately). Idempotent: openTurnStream's
      // cleanup is a no-op after a natural close.
      turnStream.settled = true;
      turnStream.close?.();
      turnStream.close = null;
      if (shouldReleaseBusyOnRequestSettle(
        sessionEpoch,
        privateSessionEpochRef.current,
        chatHomeGeneration,
        chatHomeGenerationRef.current,
      )) setBusy(false);
    }
    return turnResult;
  }

  // In-chat HITL approval action (Slice 4). Optimistically flips the card's
  // state, calls the EXISTING console approve/reject endpoint, and REVERTS on
  // failure (mirroring the other optimistic-revert patterns). The card is
  // independent of the busy/epoch/working-message lifecycle, so this never
  // touches setBusy. The same approval can also be actioned from Telegram: if
  // the backend reports it already resolved (404 / no longer pending), we keep
  // the optimistic resolved state instead of erroring hard — both surfaces are
  // legitimate.
  async function handleChatHomeApprovalAction(
    approvalId: string,
    action: 'approve' | 'reject',
  ): Promise<{ ok: boolean; message: string } | undefined> {
    // Keep voice gate stack in sync when owner taps the card (or Telegram races).
    pendingVoiceGatesRef.current = pendingVoiceGatesRef.current.filter(
      (g) => g.approvalId !== approvalId,
    );
    // Synchronous in-flight guard: a second tap on the same approval (e.g. a rapid
    // approve-then-reject) must not fire a second endpoint call before the first
    // settles — otherwise the losing call's 404-as-success paints the wrong chip.
    if (approvalActionInFlightRef.current.has(approvalId)) return undefined;
    const targetState: ChatHomeApprovalState = action === 'approve' ? 'approved' : 'rejected';
    const card = chatHomeMessagesRef.current.find(
      (message) => message.role === 'approval' && message.approvalId === approvalId,
    );
    const previousState = card?.approvalState ?? 'pending';
    const searchHold = isSearchHold(card?.summary, card?.toolName);
    // Already resolved (e.g. actioned a frame ago, or from Telegram): don't re-fire.
    if (previousState !== 'pending') return { ok: true, message: '' };
    approvalActionInFlightRef.current.add(approvalId);
    if (action === 'approve' && searchHold) {
      setChatHomeMessages((current) => setApprovalProgress(current, approvalId, {
        approvalState: 'approved',
        approvalPhase: 'running',
        approvalOutcome: null,
      }));
    } else {
      setChatHomeMessages((current) => setApprovalState(current, approvalId, targetState));
    }
    setChatHomeNotice('');
    try {
      const result = action === 'approve'
        ? await approveConsoleApproval(approvalId, initData, session?.csrf_token)
        : await rejectConsoleApproval(approvalId, initData, session?.csrf_token);
      if (action === 'approve' && searchHold) {
        const visible = ownerVisibleSearchOutcome(result.message);
        setChatHomeMessages((current) => setApprovalProgress(current, approvalId, {
          approvalState: 'approved',
          approvalPhase: visible.phase,
          approvalOutcome: visible.text || null,
        }));
      }
      // The server stores the decision. Reload it so refresh keeps the chip
      // and sibling generic cards stay off 工作. `message` is the replayed
      // tool output — the voice call needs it to keep speaking.
      void syncChatHomeHistoryFromServer();
      void refreshQueueAfterAssistant();
      return { ok: true, message: result.message };
    } catch (approvalError) {
      // Already resolved elsewhere (e.g. approved/rejected from Telegram): the
      // backend returns 404 (approval not found). Treat that as success — the
      // owner's intent matched the resolved outcome — and keep the chip.
      if (approvalError instanceof ApiResponseError && approvalError.statusCode === 404) {
        if (action === 'approve' && searchHold) {
          setChatHomeMessages((current) => setApprovalProgress(current, approvalId, {
            approvalState: 'approved',
            approvalPhase: 'done',
            approvalOutcome: null,
          }));
        }
        void syncChatHomeHistoryFromServer();
        void refreshQueueAfterAssistant();
        return { ok: true, message: '' };
      }
      // Genuine failure: revert the optimistic state and surface an error.
      setChatHomeMessages((current) => setApprovalProgress(current, approvalId, {
        approvalState: previousState,
        approvalPhase: null,
        approvalOutcome: null,
      }));
      setChatHomeNotice(t('chatHome.approval.failed', errorLocale));
      return { ok: false, message: '' };
    } finally {
      approvalActionInFlightRef.current.delete(approvalId);
    }
  }

  function queueChatHomeCommand(
    prompt?: string,
    contextOverride?: ChatHomeAttachedContext,
    modeOverride: 'text' | 'voice' = 'text',
    displayTextOverride?: string,
    retryIdempotencyKey?: string,
    quoteOverride?: ChatHomeQuote | null,
    attachments: VNextCommandAttachment[] = [],
  ) {
    const text = (prompt ?? chatHomeCommand).trim();
    // An attachment-only turn (photo, voice note) is a real message even
    // while 安琪 is busy: it used to be dropped here with no feedback.
    if (!text && attachments.length === 0) return;
    const sentAttachment = chatHomeSentAttachment(attachments);
    const displayText = displayTextOverride?.trim()
      || text
      || (sentAttachment?.kind === 'voice' ? sentAttachment.transcript.trim() : '');
    const requestContext = contextOverride ?? chatHomeAttachedContext;
    const requestQuote = quoteOverride !== undefined ? quoteOverride : chatHomeQuotedMessage;
    if (queuedChatHomeCommands.length >= MAX_CHAT_HOME_QUEUED_COMMANDS) {
      if (prompt !== undefined) setChatHomeCommand(text);
      setSnapshotMessage('');
      setChatHomeNotice(tf('chatHome.notice.queueFullCommands', errorLocale, { max: String(MAX_CHAT_HOME_QUEUED_COMMANDS) }));
      return;
    }
    const queuedMessageId = `queued:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    setQueuedChatHomeCommands((current) => [
      ...current,
      { text, displayText, context: requestContext ?? null, quote: requestQuote ?? null, mode: modeOverride, queuedMessageId, retryIdempotencyKey, attachments },
    ]);
    setChatHomeMessages((current) => [
      ...current,
      {
        id: queuedMessageId,
        role: 'queued',
        text: displayText,
        timestamp: Date.now(),
        ...(requestQuote ? { quote: requestQuote } : {}),
        ...(requestContext ? { contextLabel: requestContext.label } : {}),
        ...(sentAttachment ? { attachment: sentAttachment } : {}),
      },
    ]);
    setChatHomeCommand('');
    setChatHomeInputMode('text');
    if (attachments.length > 0) {
      // Captured into the queue entry: it must not ride the NEXT message too.
      setChatHomePendingAttachment(null);
      setChatHomeAttachmentError('');
      setChatHomeVoiceNote(IDLE_VOICE_NOTE);
    }
    if (!contextOverride) {
      setChatHomeAttachedContext(null);
    }
    setChatHomeQuotedMessage(null);
    setSnapshotMessage('');
    setChatHomeNotice(t('chatHome.notice.queued', errorLocale));
  }

  function chatHomeConversationContext(quote?: ChatHomeQuote | null): VNextConversationContextTurn[] {
    const turns = chatHomeMessages.flatMap((message): VNextConversationContextTurn[] => {
      if (message.role === 'owner' && message.text.trim()) {
        return [{ role: 'owner', text: compactConversationTurnText(message.text) }];
      }
      if (message.role === 'aegis') {
        const text = message.result?.visible_answer || message.text;
        if (text.trim()) {
          return [{ role: 'aegis', text: compactConversationTurnText(text) }];
        }
      }
      return [];
    });
    if (!quote) return turns.slice(-8);
    return [...turns.slice(-7), quotedConversationTurn(quote)].slice(-8);
  }

  function updateChatHomeWorkThread(
    messageId: string,
    threadId: string,
    update: { state?: 'queued' | 'working'; workingText?: string; result?: ContextualAssistantResponse; responseTimestamp?: number },
  ) {
    setChatHomeMessages((current) => current.map((message) => {
      if (message.id !== messageId) return message;
      const timestampedUpdate = update.result && !update.responseTimestamp
        ? { ...update, responseTimestamp: Date.now() }
        : update;
      return {
        ...message,
        workItemThread: (message.workItemThread ?? []).map((entry) => (
          entry.id === threadId ? { ...entry, ...timestampedUpdate } : entry
        )),
      };
    }));
  }

  function chatHomeWorkConversationContext(messageId: string, itemStatus?: string | null, excludeThreadId?: string): VNextConversationContextTurn[] {
    const message = chatHomeMessages.find((candidate) => candidate.id === messageId);
    if (!message?.workItemThread?.length) return [];
    return message.workItemThread.filter((entry) => (
      entry.id !== excludeThreadId && (!entry.itemStatus || entry.itemStatus === itemStatus)
    )).flatMap((entry): VNextConversationContextTurn[] => {
      if (entry.state === 'queued') return [];
      const turns: VNextConversationContextTurn[] = [{ role: 'owner', text: compactConversationTurnText(entry.question) }];
      if (entry.result?.visible_answer) {
        turns.push({ role: 'aegis', text: compactConversationTurnText(entry.result.visible_answer) });
      }
      return turns;
    }).slice(-8);
  }

  function queueChatHomeWorkItemAsk(messageId: string, item: OwnerWorkItem, text: string) {
    if (queuedChatHomeWorkPrompts.length >= MAX_CHAT_HOME_QUEUED_WORK_PROMPTS) {
      setSnapshotMessage('');
      setChatHomeNotice(tf('chatHome.notice.queueFullWork', errorLocale, { max: String(MAX_CHAT_HOME_QUEUED_WORK_PROMPTS) }));
      return;
    }
    const queuedThreadId = `work-thread:queued:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    setQueuedChatHomeWorkPrompts((current) => [
      ...current,
      { messageId, item, text, queuedThreadId },
    ]);
    setChatHomeMessages((current) => current.map((message) => (
      message.id === messageId
        ? {
          ...message,
          workItemThread: [
            ...(message.workItemThread ?? []),
            { id: queuedThreadId, question: text, timestamp: Date.now(), itemStatus: item.status, state: 'queued' as const },
          ],
        }
        : message
    )));
    setSnapshotMessage('');
    setChatHomeNotice(t('chatHome.notice.queuedWork', errorLocale));
  }

  async function askChatHomeWorkItem(
    messageId: string,
    item: OwnerWorkItem,
    ownerText: string,
    queuedThreadId?: string,
  ) {
    const text = ownerText.trim();
    if (!text) return;
    if (busy && !queuedThreadId) {
      queueChatHomeWorkItemAsk(messageId, item, text);
      return;
    }
    const sessionEpoch = privateSessionEpochRef.current;
    const chatHomeGeneration = chatHomeGenerationRef.current;
    const commandIdempotencyKey = chatHomeIdempotencyKey();
    const threadId = queuedThreadId ?? `work-thread:${Date.now()}:${Math.random().toString(36).slice(2)}`;
    const conversationContext = chatHomeWorkConversationContext(messageId, item.status, queuedThreadId);
    const updateWorkingMessage = (nextText: string) => {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
      updateChatHomeWorkThread(messageId, threadId, { state: 'working', workingText: nextText });
    };
    const liveProgressLines: string[] = [];
    let keepPageOpenHint = false;
    const keepPageOpenText = t(CHAT_HOME_WORKING_LONG_KEY, errorLocale);
    const workingFallback = t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale);
    const paintWorkingProgress = (nextLive?: string) => {
      if (nextLive) {
        const next = appendProgressLine(liveProgressLines, nextLive);
        liveProgressLines.splice(0, liveProgressLines.length, ...next);
      }
      updateWorkingMessage(composeWorkingText(
        liveProgressLines,
        keepPageOpenHint,
        keepPageOpenText,
        liveProgressLines.length === 0 ? workingFallback : liveProgressLines.join('\n'),
      ));
    };
    setChatHomeMessages((current) => current.map((message) => (
      message.id === messageId
        ? {
          ...message,
          workItemThread: queuedThreadId
            ? (message.workItemThread ?? []).map((entry) => (
              entry.id === queuedThreadId
                ? { ...entry, itemStatus: item.status, state: 'working' as const, workingText: t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale) }
                : entry
            ))
            : [
              ...(message.workItemThread ?? []),
              { id: threadId, question: text, timestamp: Date.now(), itemStatus: item.status, state: 'working' as const, workingText: t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale) },
            ],
        }
        : message
    )));
    setBusy(true);
    const slowTimer = window.setTimeout(() => {
      if (liveProgressLines.length === 0) {
        paintWorkingProgress(t(CHAT_HOME_WORKING_SLOW_KEY, errorLocale));
      }
    }, 2500);
    const longTimer = window.setTimeout(() => {
      keepPageOpenHint = true;
      paintWorkingProgress();
    }, 25000);
    const turnStream: { close: (() => void) | null; settled: boolean } = { close: null, settled: false };
    let streamedReply = '';
    void mintVNextTurnStreamToken(initData, session?.csrf_token, commandIdempotencyKey)
      .then((streamToken) => {
        if (turnStream.settled) return;
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
        turnStream.close = openTurnStream(
          commandIdempotencyKey,
          streamToken,
          (event: TurnEvent) => {
            switch (event.type) {
              case 'tool_start':
                paintWorkingProgress(workingPhraseForTool(event.tool_name, errorLocale));
                break;
              case 'tool_done':
                paintWorkingProgress(toolDonePhrase(errorLocale));
                break;
              case 'vision_routed':
                paintWorkingProgress(t('chatHome.working.processingImage', errorLocale));
                break;
              case 'intent_resolved':
                paintWorkingProgress(
                  event.needs_review
                    ? tf('chatHome.working.intentReview', errorLocale, { label: event.label })
                    : tf('chatHome.working.intent', errorLocale, { label: event.label }),
                );
                break;
              case 'action_phase':
                paintWorkingProgress(workingPhraseForPhase(event.phase, event.tool_name, event.detail, errorLocale));
                break;
              case 'owner_notice':
                if (event.text.trim()) paintWorkingProgress(workingNotice(event.text.trim(), event.kind, errorLocale));
                break;
              case 'need_approval':
                paintWorkingProgress(workingPhraseForPhase('approval_required', event.tool_name, null, errorLocale));
                break;
              case 'assistant_delta':
                if (!event.text) break;
                streamedReply = appendAssistantDelta(streamedReply, event.text);
                updateWorkingMessage(streamedReply);
                break;
              case 'assistant_draft_reset':
                streamedReply = '';
                paintWorkingProgress();
                break;
              default:
                break;
            }
          },
          () => {
            turnStream.close = null;
          },
        );
        if (turnStream.settled) {
          turnStream.close?.();
          turnStream.close = null;
        }
      })
      .catch(() => {
        /* advisory */
      });
    try {
      const result = await requestAssistantResultWithAutoCheck({
        initData,
        csrfToken: session?.csrf_token,
        focusId: item.id,
        intent: 'chat',
        text,
        scope: 'item',
        idempotencyKey: commandIdempotencyKey,
        conversationContext,
        // The util emits an English still-working default; substitute the
        // display_language version so zh sessions never see an English flash.
        onStillWorking: () => {
          if (liveProgressLines.length === 0) {
            paintWorkingProgress(t('chatHome.working.stillChecking', errorLocale));
          }
        },
      });
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
      updateChatHomeWorkThread(messageId, threadId, { result, state: 'working', workingText: undefined });
      void refreshQueueAfterAssistant(item.id);
    } catch (assistantError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      if (chatHomeGenerationRef.current !== chatHomeGeneration) return;
      const result: ContextualAssistantResponse = {
        schema_version: 1,
        response_kind: 'error',
        visible_answer: assistantDisplayError(assistantError, errorLocale),
        receipt_expectation: t('app.receipt.noActionTaken', errorLocale),
        citations: [],
        warnings: [],
        severity: 'warning',
      };
      updateChatHomeWorkThread(messageId, threadId, { result, state: 'working', workingText: undefined });
    } finally {
      window.clearTimeout(slowTimer);
      window.clearTimeout(longTimer);
      turnStream.settled = true;
      turnStream.close?.();
      turnStream.close = null;
      if (shouldReleaseBusyOnRequestSettle(
        sessionEpoch,
        privateSessionEpochRef.current,
        chatHomeGeneration,
        chatHomeGenerationRef.current,
      )) setBusy(false);
    }
  }

  function clearChatHomeConversation() {
    chatHomeGenerationRef.current += 1;
    setSessionResetTick((tick) => tick + 1);
    setBusy(false);
    setChatHomeMessages([]);
    // Older pages pulled in with 加载更早 belong to this conversation/session too.
    setEarlierChatMessages([]);
    setChatHistoryHasMore(false);
    setLoadingEarlierChat(false);
    setEarlierChatError('');
    setChatHomeCommand('');
    setQueuedChatHomeCommands([]);
    setQueuedChatHomeWorkPrompts([]);
    setChatHomeInputMode('text');
    setChatHomeAttachedContext(null);
    setChatHomeQuotedMessage(null);
    setChatHomeNotice(t('chatHome.notice.chatCleared', errorLocale));
    writeSessionDraft(CHAT_HOME_DRAFT_KEY, '');
    clearChatHomeHistory(window.sessionStorage, CHAT_HOME_HISTORY_KEY);
    const ownerHistoryKey = authenticatedChatHomeHistoryKey(session);
    if (ownerHistoryKey) clearChatHomeHistory(window.localStorage, ownerHistoryKey);
  }

  function clearPrivateBrowserSessionState() {
    invalidatePrivateSessionEpoch();
    setQueue(null);
    setStatusProjection(null);
    setSettingsProjection(null);
    setContext(null);
    setSourceDetail(null);
    setSelectedId(null);
    setItemProjection(null);
    setAssistantResult(null);
    setAssistantResultItemId(null);
    setLastItemCommand(null);
    setOptimisticStatusByItemId({});
    setRetiredActionItemIds({});
    setArmedAction(null);
    setArmedActionItemId(null);
    setExecutingAction(null);
    setExpandedLaneIds([]);
    setChatHomeMessages([]);
    // Older pages pulled in with 加载更早 belong to this conversation/session too.
    setEarlierChatMessages([]);
    setChatHistoryHasMore(false);
    setLoadingEarlierChat(false);
    setEarlierChatError('');
    setChatHomeCommand('');
    setQueuedChatHomeCommands([]);
    setQueuedChatHomeWorkPrompts([]);
    setChatHomeInputMode('text');
    setChatHomeAttachedContext(null);
    setChatHomeQuotedMessage(null);
    setChatHomeNotice('');
    setWorkLoadNotice('');
    setAvatarProfile(defaultAvatarProfile());
    setAvatarAutoRefresh(null);
    setWebPushStatus(null);
    setWebPushDeviceSubscribed(null);
    setWebPushMessage('');
    setDebugLeases([]);
    setDebugGrantToken(null);
    setDebugJobs([]);
    hydratedAvatarProfileKeyRef.current = null;
    setSnapshotMessage('');
    writeSessionDraft(CHAT_HOME_DRAFT_KEY, '');
  }

  function persistAvatarProfile(next: AvatarProfile) {
    const ownerKey = avatarProfileOwnerKey(session?.email);
    writeAvatarProfile(window.localStorage, ownerKey, next);
    // Telegram sessions have no CSRF token; the server does not require one for them.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    updateVNextAvatarProfile(initData, session.csrf_token, avatarProfileToServer(next))
      .then((savedProfile) => {
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        const syncedProfile = avatarProfileFromServer(savedProfile);
        setAvatarProfile(syncedProfile);
        setAvatarAutoRefresh(savedProfile.auto_refresh ?? null);
        writeAvatarProfile(window.localStorage, ownerKey, syncedProfile);
      })
      .catch((syncError) => {
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        setSnapshotMessage(displayError(syncError, t('error.fallback.avatarServerSyncFailed', errorLocale), errorLocale));
      });
  }

  function updateAvatarProfile(role: AvatarRole, dataUrl: string) {
    setAvatarProfile((current) => {
      const next = role === 'aegis'
        ? { ...current, aegisAvatarUrl: dataUrl }
        : { ...current, ownerAvatarUrl: dataUrl };
      persistAvatarProfile(next);
      return next;
    });
  }

  function resetAvatarProfile(role: AvatarRole) {
    setAvatarProfile((current) => {
      const defaults = defaultAvatarProfile();
      const next = role === 'aegis'
        ? { ...current, aegisAvatarUrl: defaults.aegisAvatarUrl }
        : { ...current, ownerAvatarUrl: defaults.ownerAvatarUrl };
      persistAvatarProfile(next);
      return next;
    });
  }

  async function toggleAegisAvatarAutoRefresh(enabled: boolean, promptHint: string) {
    if (!session?.authenticated || !session.csrf_token || avatarGenerationBusy) return;
    const ownerKey = avatarProfileOwnerKey(session.email);
    const sessionEpoch = privateSessionEpochRef.current;
    setAvatarGenerationBusy(true);
    setSnapshotMessage('');
    try {
      const savedProfile = await updateVNextAvatarAutoRefresh(initData, session.csrf_token, enabled, promptHint);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      const syncedProfile = avatarProfileFromServer(savedProfile);
      setAvatarProfile(syncedProfile);
      setAvatarAutoRefresh(savedProfile.auto_refresh ?? null);
      writeAvatarProfile(window.localStorage, ownerKey, syncedProfile);
      setSnapshotMessage(enabled ? t('app.avatarRefreshEnabled', errorLocale) : t('app.avatarRefreshPaused', errorLocale));
    } catch (error) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(error, t('error.fallback.avatarAutoRefreshFailed', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setAvatarGenerationBusy(false);
    }
  }

  async function generateAegisAvatarNow(promptHint: string) {
    if (!session?.authenticated || !session.csrf_token || avatarGenerationBusy) return;
    const ownerKey = avatarProfileOwnerKey(session.email);
    const sessionEpoch = privateSessionEpochRef.current;
    const previousAegisAvatarUrl = avatarProfile.aegisAvatarUrl;
    setAvatarGenerationBusy(true);
    setSnapshotMessage('');
    try {
      const generated = await generateVNextAegisAvatar(initData, session.csrf_token, promptHint);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      const syncedProfile = avatarProfileFromServer(generated.profile);
      setAvatarProfile(syncedProfile);
      setAvatarAutoRefresh(generated.profile.auto_refresh ?? null);
      writeAvatarProfile(window.localStorage, ownerKey, syncedProfile);
      // Localized here, not the server's English `owner_message` (owner IMG 2026-09-12).
      setSnapshotMessage(t('app.avatarGenerated', errorLocale));
    } catch (error) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      try {
        const reconciledProfile = await fetchVNextAvatarProfile(initData);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        const syncedProfile = avatarProfileFromServer(reconciledProfile);
        if (
          syncedProfile.aegisAvatarUrl !== previousAegisAvatarUrl
          && syncedProfile.aegisAvatarUrl.startsWith('/webapp/vnext/avatar-image/')
        ) {
          setAvatarProfile(syncedProfile);
          setAvatarAutoRefresh(reconciledProfile.auto_refresh ?? null);
          writeAvatarProfile(window.localStorage, ownerKey, syncedProfile);
          setSnapshotMessage(t('app.avatarGeneratedReconciled', errorLocale));
          return;
        }
      } catch {
        // Fall through to the original error. The server profile did not
        // confirm a saved avatar, so it is safer to show the failure.
      }
      setSnapshotMessage(displayError(error, t('error.fallback.avatarGenerationFailed', errorLocale), errorLocale));
      if (snapshotAvailable && session.csrf_token) {
        void submitVNextDebugSnapshot(
          initData,
          session.csrf_token,
          'settings',
          `${window.location.pathname}${window.location.search}`,
          {
            schema_version: 1,
            feature: 'aegis_avatar_generation',
            failure: displayError(error, t('error.fallback.avatarGenerationFailed', errorLocale), errorLocale),
            prompt_hint_length: promptHint.length,
            image_generation_available: avatarAutoRefresh?.image_generation_available ?? false,
            grok: {
              connected: Boolean(grokStatus?.connected),
              active_for_chat: Boolean(grokStatus?.active_for_chat),
            },
            codex: {
              connected: Boolean(codexStatus?.connected),
              active_for_chat: Boolean(codexStatus?.active_for_chat),
            },
          },
        ).catch(() => undefined);
      }
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setAvatarGenerationBusy(false);
    }
  }

  async function pauseCostSavingControls() {
    if (!session?.csrf_token || costSavingBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCostSavingBusy(true);
    setSnapshotMessage('');
    setChatHomeNotice('');
    try {
      const nextSettings = await pauseVNextCostSaving(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
      const nextStatus = await fetchVNextStatus(initData).catch(() => null);
      if (nextStatus) setStatusProjection(nextStatus);
      setSnapshotMessage(t('app.checksPaused', errorLocale));
    } catch (pauseError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(pauseError, t('error.fallback.couldNotPauseChecks', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCostSavingBusy(false);
    }
  }

  async function resumeCostSavingControls() {
    if (!session?.csrf_token || costSavingBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCostSavingBusy(true);
    setSnapshotMessage('');
    setChatHomeNotice('');
    try {
      const nextSettings = await resumeVNextCostSaving(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
      const nextStatus = await fetchVNextStatus(initData).catch(() => null);
      if (nextStatus) setStatusProjection(nextStatus);
      setSnapshotMessage(t('app.checksResumed', errorLocale));
    } catch (resumeError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(resumeError, t('error.fallback.couldNotResumeChecks', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCostSavingBusy(false);
    }
  }

  async function persistDisplayLanguage(value: Locale) {
    // Telegram-principal sessions carry no CSRF token (the server only requires
    // one for cookie sessions); gating on it silently dropped every settings
    // write from the phone. Gate on the session itself and let the server judge.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setDisplayLanguage(initData, session.csrf_token, value);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
    } catch (languageError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      // Surface the failure to the owner, then re-throw so the caller that owns
      // the optimistic `setLocale` (SettingsView) can revert the live UI locale
      // back to the still-unchanged backend setting (and `errorLocale`).
      setSnapshotMessage(displayError(languageError, t('error.fallback.couldNotUpdateLanguage', errorLocale), errorLocale));
      throw languageError;
    }
  }

  async function persistLlmProvider(value: string, model?: string) {
    // Telegram-principal sessions carry no CSRF token (the server only requires
    // one for cookie sessions); gating on it silently dropped every settings
    // write from the phone. Gate on the session itself and let the server judge.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setLlmProvider(initData, session.csrf_token, value, model);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      // Only commit the refreshed projection on success → the selector reflects
      // the persisted value, never an optimistic-but-unsaved one.
      setSettingsProjection(nextSettings);
    } catch (providerError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      // Surface the failure; leave the existing projection untouched so the
      // control snaps back to the still-current backend value (no inconsistency).
      setSnapshotMessage(displayError(providerError, t('error.fallback.couldNotUpdateProvider', errorLocale), errorLocale));
      throw providerError;
    }
  }

  async function persistLlmModel(value: string) {
    // Telegram-principal sessions carry no CSRF token (the server only requires
    // one for cookie sessions); gating on it silently dropped every settings
    // write from the phone. Gate on the session itself and let the server judge.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setLlmModel(initData, session.csrf_token, value);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      // Only commit the refreshed projection on success → the model field
      // re-syncs to the persisted value (SettingsView's useEffect keys off it).
      setSettingsProjection(nextSettings);
    } catch (modelError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      // Surface the failure; leave the projection untouched so the field's draft
      // reverts to the still-current backend value (no optimistic-unsaved drift).
      setSnapshotMessage(displayError(modelError, t('error.fallback.couldNotUpdateModel', errorLocale), errorLocale));
    }
  }

  async function persistFallbackModel(value: string) {
    // Telegram-principal sessions carry no CSRF token (the server only requires
    // one for cookie sessions); gating on it silently dropped every settings
    // write from the phone. Gate on the session itself and let the server judge.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setFallbackModel(initData, session.csrf_token, value);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
    } catch (fallbackError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(fallbackError, t('error.fallback.couldNotUpdateFallbackModel', errorLocale), errorLocale));
      throw fallbackError;
    }
  }

  async function loadModelCatalog() {
    return fetchVNextModelCatalog(initData);
  }

  async function persistWebSearchProvider(value: string) {
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setWebSearchProvider(initData, session.csrf_token, value);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
    } catch (searchError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(searchError, t('error.fallback.couldNotUpdateWebSearch', errorLocale), errorLocale));
      throw searchError;
    }
  }

  async function persistWebSearchModel(value: string) {
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setWebSearchModel(initData, session.csrf_token, value);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
    } catch (searchError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(searchError, t('error.fallback.couldNotUpdateWebSearch', errorLocale), errorLocale));
      throw searchError;
    }
  }

  async function persistPaidApiFallback(enabled: boolean) {
    // Telegram-principal sessions carry no CSRF token (the server only requires
    // one for cookie sessions); gating on it silently dropped every settings
    // write from the phone. Gate on the session itself and let the server judge.
    if (!session?.authenticated) return;
    const sessionEpoch = privateSessionEpochRef.current;
    try {
      const nextSettings = await setPaidApiFallback(initData, session.csrf_token, enabled);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSettingsProjection(nextSettings);
    } catch (fallbackError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(fallbackError, t('error.fallback.couldNotUpdatePaidApiFallback', errorLocale), errorLocale));
      throw fallbackError;
    }
  }

  function dismissLlmDegradedBanner(since: string) {
    setLlmDegradedDismissedSince(since);
    writeLlmDegradedDismissedSince(since);
  }

  function dismissEmbedderDegradedBanner(since: string) {
    setEmbedderDegradedDismissedSince(since);
    writeEmbedderDegradedDismissedSince(since);
  }

  function dismissTelegramRailBanner(sinceUnix: number) {
    setTelegramRailDismissedSince(sinceUnix);
    writeTelegramRailDismissedSince(sinceUnix);
  }

  async function refreshWebPushStatus() {
    const nextStatus = await fetchVNextWebPushStatus(initData);
    setWebPushStatus(nextStatus);
    void syncWebPushDeviceRegistration(nextStatus, session?.csrf_token);
    return nextStatus;
  }

  async function syncWebPushDeviceRegistration(
    status: VNextWebPushStatus | null,
    csrfToken: string | undefined,
    sessionEpoch = privateSessionEpochRef.current,
  ) {
    if (!status?.configured) {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setWebPushDeviceSubscribed(null);
      return status;
    }
    try {
      const subscription = await currentWebPushSubscription(errorLocale);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return status;
      setWebPushDeviceSubscribed(Boolean(subscription));
      // No CSRF gate: Telegram sessions never carry one, and the phone is
      // exactly where the push subscription needs to reach the server.
      if (!subscription) return status;
      const syncedStatus = await subscribeVNextWebPush(initData, csrfToken, subscription);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return status;
      setWebPushStatus(syncedStatus);
      return syncedStatus;
    } catch {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setWebPushDeviceSubscribed(false);
      return status;
    }
  }

  async function enableWebPushNotifications() {
    if (!session?.csrf_token || webPushBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setWebPushBusy(true);
    setWebPushMessage('');
    try {
      const status = webPushStatus ?? await fetchVNextWebPushStatus(initData);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushStatus(status);
      if (!status.configured || !status.public_key) {
        setWebPushMessage(status.owner_next_step || status.owner_message || t('app.webPush.notConfigured', errorLocale));
        return;
      }
      const subscription = await subscribeOwnerWebPush(status.public_key, errorLocale);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      const nextStatus = await subscribeVNextWebPush(initData, session.csrf_token, subscription);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushStatus(nextStatus);
      setWebPushDeviceSubscribed(true);
      setWebPushMessage(t('app.webPush.enabled', errorLocale));
      const nextSettings = await fetchVNextSettings(initData).catch(() => null);
      if (nextSettings && isPrivateSessionEpochCurrent(sessionEpoch)) setSettingsProjection(nextSettings);
    } catch (pushError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushMessage(displayError(pushError, t('error.fallback.couldNotEnableWebPush', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setWebPushBusy(false);
    }
  }

  async function disableWebPushNotifications() {
    if (!session?.csrf_token || webPushBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setWebPushBusy(true);
    setWebPushMessage('');
    try {
      const endpoint = await unsubscribeOwnerWebPush(errorLocale);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      const nextStatus = endpoint
        ? await unsubscribeVNextWebPush(initData, session.csrf_token, endpoint)
        : await refreshWebPushStatus();
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushStatus(nextStatus);
      setWebPushDeviceSubscribed(false);
      setWebPushMessage(t('app.webPush.disabled', errorLocale));
      const nextSettings = await fetchVNextSettings(initData).catch(() => null);
      if (nextSettings && isPrivateSessionEpochCurrent(sessionEpoch)) setSettingsProjection(nextSettings);
    } catch (pushError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushMessage(displayError(pushError, t('error.fallback.couldNotDisableWebPush', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setWebPushBusy(false);
    }
  }

  async function sendWebPushTestNotification() {
    if (!session?.csrf_token || webPushBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setWebPushBusy(true);
    setWebPushMessage('');
    try {
      const result = await testVNextWebPush(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushMessage(`${result.owner_message} Receipt: ${result.receipt}`);
      const nextStatus = await fetchVNextWebPushStatus(initData).catch(() => null);
      if (nextStatus && isPrivateSessionEpochCurrent(sessionEpoch)) {
        setWebPushStatus(nextStatus);
        void syncWebPushDeviceRegistration(nextStatus, session.csrf_token, sessionEpoch);
      }
    } catch (pushError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setWebPushMessage(displayError(pushError, t('error.fallback.couldNotSendWebPushTest', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setWebPushBusy(false);
    }
  }

  async function refreshVoiceStatus() {
    const nextVoiceStatus = await fetchVNextVoiceStatus(initData).catch(() => null);
    setVoiceRealtimeAvailable(Boolean(nextVoiceStatus?.realtime_available));
    return nextVoiceStatus;
  }

  async function refreshGrokStatus() {
    const nextStatus = await fetchVNextGrokStatus(initData);
    setGrokStatus(nextStatus);
    // Grok connect/refresh is the Realtime mint credential — keep the orb's
    // availability flag in sync (do not wait for a full page reload).
    await refreshVoiceStatus().catch(() => null);
    return nextStatus;
  }

  async function refreshCodexStatus() {
    const nextStatus = await fetchVNextCodexStatus(initData);
    setCodexStatus(nextStatus);
    return nextStatus;
  }

  async function refreshProviderStatuses() {
    const [nextCodexStatus, nextGrokStatus, nextVoiceStatus] = await Promise.all([
      fetchVNextCodexStatus(initData).catch(() => null),
      fetchVNextGrokStatus(initData).catch(() => null),
      fetchVNextVoiceStatus(initData).catch(() => null),
    ]);
    setCodexStatus(nextCodexStatus);
    setGrokStatus(nextGrokStatus);
    setVoiceRealtimeAvailable(Boolean(nextVoiceStatus?.realtime_available));
    return { codex: nextCodexStatus, grok: nextGrokStatus };
  }

  async function startCodexDeviceCode() {
    if (!session?.csrf_token || codexBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCodexBusy(true);
    setCodexMessage('');
    try {
      const response = await startVNextCodexDeviceCode(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexDeviceCode(response);
      setCodexMessage(response.message);
      await refreshCodexStatus().catch(() => null);
    } catch (startError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(displayError(startError, t('error.fallback.couldNotStartCodexSignIn', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCodexBusy(false);
    }
  }

  async function pollCodexDeviceCodeNow() {
    if (!session?.csrf_token || codexBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCodexBusy(true);
    try {
      const response: VNextCodexPollResponse = await pollVNextCodexDeviceCode(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(response.message);
      if (response.status === 'connected') setCodexDeviceCode(null);
      await refreshCodexStatus().catch(() => null);
    } catch (pollError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(displayError(pollError, t('error.fallback.couldNotCheckCodexSignIn', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCodexBusy(false);
    }
  }

  async function useCodexForChat() {
    if (!session?.csrf_token || codexBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCodexBusy(true);
    try {
      const response = await useVNextCodexForChat(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexDeviceCode(null);
      setCodexMessage(response.message);
      await refreshProviderStatuses();
    } catch (routingError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(displayError(routingError, t('error.fallback.couldNotUseCodexForChat', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCodexBusy(false);
    }
  }

  async function setCodexModel(model: string, reasoningEffort: string) {
    if (!session?.csrf_token || codexBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCodexBusy(true);
    try {
      const response = await setVNextCodexModel(initData, session.csrf_token, model, reasoningEffort);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(response.message);
      await refreshCodexStatus().catch(() => null);
    } catch (modelError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(displayError(modelError, t('error.fallback.couldNotUpdateCodexModel', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCodexBusy(false);
    }
  }

  async function stopUsingCodexForChat() {
    if (!session?.csrf_token || codexBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setCodexBusy(true);
    try {
      const response = await stopUsingVNextCodexForChat(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexDeviceCode(null);
      setCodexMessage(response.message);
      await refreshProviderStatuses();
    } catch (routingError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setCodexMessage(displayError(routingError, t('error.fallback.couldNotStopUsingCodexForChat', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setCodexBusy(false);
    }
  }

  async function startGrokDeviceCode() {
    if (!session?.csrf_token || grokBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setGrokBusy(true);
    setGrokMessage('');
    try {
      const response = await startVNextGrokDeviceCode(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokDeviceCode(response);
      setGrokMessage(response.message);
      await refreshGrokStatus().catch(() => null);
    } catch (startError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(displayError(startError, t('error.fallback.couldNotStartGrokSignIn', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setGrokBusy(false);
    }
  }

  async function pollGrokDeviceCodeNow() {
    if (!session?.csrf_token || grokBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setGrokBusy(true);
    try {
      const response: VNextGrokPollResponse = await pollVNextGrokDeviceCode(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(response.message);
      if (response.status === 'connected') setGrokDeviceCode(null);
      await refreshGrokStatus().catch(() => null);
    } catch (pollError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(displayError(pollError, t('error.fallback.couldNotCheckGrokSignIn', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setGrokBusy(false);
    }
  }

  async function revokeGrokConnection() {
    if (!session?.csrf_token || grokBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setGrokBusy(true);
    try {
      const response = await revokeVNextGrok(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokDeviceCode(null);
      setGrokMessage(response.message);
      await refreshProviderStatuses();
    } catch (revokeError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(displayError(revokeError, t('error.fallback.couldNotRevokeGrokConnection', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setGrokBusy(false);
    }
  }

  async function useGrokForChat() {
    if (!session?.csrf_token || grokBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setGrokBusy(true);
    try {
      const response = await useVNextGrokForChat(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(response.message);
      await refreshProviderStatuses();
    } catch (routingError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(displayError(routingError, t('error.fallback.couldNotUseGrokForChat', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setGrokBusy(false);
    }
  }

  async function stopUsingGrokForChat() {
    if (!session?.csrf_token || grokBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setGrokBusy(true);
    try {
      const response = await stopUsingVNextGrokForChat(initData, session.csrf_token);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(response.message);
      await refreshProviderStatuses();
    } catch (routingError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setGrokMessage(displayError(routingError, t('error.fallback.couldNotStopUsingGrokForChat', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setGrokBusy(false);
    }
  }

  async function logoutNow() {
    const csrfToken = session?.csrf_token;
    setLogoutBusy(true);
    setSession({ authenticated: false });
    setState('ready');
    writeVNextLocationState(window, { view: 'home', itemId: null }, 'replace');
    setView('home');
    setBusy(false);
    setSourceDetailBusy(false);
    setCostSavingBusy(false);
    clearPrivateBrowserSessionState();
    clearChatHomeHistory(window.sessionStorage, CHAT_HOME_HISTORY_KEY);
    const ownerHistoryKey = authenticatedChatHomeHistoryKey(session);
    if (ownerHistoryKey) clearChatHomeHistory(window.localStorage, ownerHistoryKey);
    hydratedChatHomeHistoryKeyRef.current = null;
    if (!csrfToken) {
      setLogoutBusy(false);
      return;
    }
    try {
      await logout(csrfToken);
    } catch {
      // The browser has already failed closed locally. The next Google sign-in
      // or server session check is the source of truth for opening private data.
    } finally {
      setLogoutBusy(false);
    }
  }

  async function installOwnerApp() {
    setSnapshotMessage('');
    setChatHomeNotice('');
    const nextStatus = currentOwnerInstallStatus(Boolean(installPromptEvent));
    setInstallStatus(nextStatus);
    if (nextStatus === 'installed') {
      setChatHomeNotice(t('chatHome.notice.alreadyInstalled', errorLocale));
      return;
    }
    if (!installPromptEvent) {
      setChatHomeNotice(ownerInstallManualNotice(navigator.userAgent, errorLocale));
      return;
    }
    const promptEvent = installPromptEvent;
    setInstallPromptEvent(null);
    setInstallStatus('manual');
    try {
      await promptEvent.prompt();
      const choice = await promptEvent.userChoice;
      setChatHomeNotice(ownerInstallOutcomeNotice(choice, errorLocale));
      if (choice.outcome === 'accepted') {
        setInstallStatus('installed');
      }
    } catch {
      setChatHomeNotice(ownerInstallManualNotice(navigator.userAgent, errorLocale));
    }
  }

  async function createDebugSnapshot() {
    if (!snapshotAvailable) {
      setSnapshotMessage(t('app.snapshotNotEnabled', errorLocale));
      setChatHomeNotice(t('app.snapshotNotEnabled', errorLocale));
      return;
    }
    const sessionEpoch = privateSessionEpochRef.current;
    setSnapshotBusy(true);
    setSnapshotMessage('');
    setChatHomeNotice('');
    try {
      const response = await submitVNextDebugSnapshot(
        initData,
        session?.csrf_token,
        view,
        `${window.location.pathname}${window.location.search}`,
        buildVNextDebugSnapshotPayload({
          view,
          selectedId,
          queue,
          home,
          statusProjection,
          settingsProjection,
          commandScope: selectedId ? 'item' : 'general',
          viewport: {
            width: window.innerWidth,
            height: window.innerHeight,
            device_pixel_ratio: window.devicePixelRatio,
            scroll_y: Math.round(window.scrollY),
          },
          layout: [
            debugElementMetrics('.vnext-header'),
            debugElementMetrics('.vnext-work-brief'),
            debugElementMetrics('.vnext-detail'),
            debugElementMetrics('.vnext-chat-home'),
            debugElementMetrics('.vnext-chat-stream'),
            debugElementMetrics('.vnext-chat-hero'),
            debugElementMetrics('.vnext-chat-composer'),
            debugElementMetrics('.vnext-settings-section'),
            debugElementMetrics('.vnext-avatar-auto-card'),
            debugElementMetrics('.vnext-nav'),
          ].filter(isDebugElementMetric),
          controls: [
            debugElementControls('.vnext-detail'),
            debugElementControls('.vnext-work-brief'),
            debugElementControls('.vnext-chat-home'),
            debugElementControls('.vnext-settings-section'),
            debugElementControls('.vnext-avatar-auto-card'),
            debugElementControls('.vnext-nav'),
          ].filter(isDebugElementControl),
        }),
      );
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(tf('app.snapshotSaved', errorLocale, { id: response.id.slice(0, 8) }), 'ok');
    } catch (snapshotError) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(snapshotError, t('error.fallback.couldNotSaveSnapshot', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setSnapshotBusy(false);
    }
  }

  async function createOwnerDebugLease(
    scope: Parameters<typeof createVNextDebugLease>[2],
    reason: string,
    durationMinutes = 15,
    itemId?: string | null,
  ) {
    if (!snapshotAvailable) {
      setSnapshotMessage(t('app.debugLeaseNotEnabled', errorLocale));
      return;
    }
    if (!session?.csrf_token || debugLeaseBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setDebugLeaseBusy(true);
    setSnapshotMessage('');
    try {
      const response = await createVNextDebugLease(
        initData,
        session.csrf_token,
        scope,
        reason,
        ['inspect'],
        durationMinutes,
        itemId ?? null,
      );
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setDebugLeases((current) => [response.lease, ...current.filter((lease) => lease.id !== response.lease.id)]);
      setDebugGrantToken(response.debug_grant_token ?? null);
      setSnapshotMessage(tf('app.debugLeaseOpened', errorLocale, { owner: response.owner_message, id: response.lease.id.slice(0, 8) }));
    } catch (error) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(error, t('error.fallback.couldNotOpenSupportDebugLease', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setDebugLeaseBusy(false);
    }
  }

  async function revokeOwnerDebugLease(leaseId: string) {
    if (!session?.csrf_token || debugLeaseBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setDebugLeaseBusy(true);
    setSnapshotMessage('');
    try {
      await revokeVNextDebugLease(initData, session.csrf_token, leaseId);
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setDebugLeases((current) => current.filter((lease) => lease.id !== leaseId));
      setDebugGrantToken(null);
      setSnapshotMessage(tf('app.debugLeaseRevoked', errorLocale, { id: leaseId.slice(0, 8) }));
    } catch (error) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(error, t('error.fallback.couldNotRevokeSupportDebugLease', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setDebugLeaseBusy(false);
    }
  }

  async function runOwnerDebugJob() {
    if (!snapshotAvailable) {
      setSnapshotMessage(t('app.debugJobNotEnabled', errorLocale));
      return;
    }
    if (!session?.csrf_token || debugJobBusy) return;
    const sessionEpoch = privateSessionEpochRef.current;
    setDebugJobBusy(true);
    setSnapshotMessage('');
    try {
      const response = await createVNextDebugJob(
        initData,
        session.csrf_token,
        'webapp_e2e_suite',
        'Owner authorized backend WebApp E2E support suite.',
      );
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setDebugJobs((current) => [response.job, ...current.filter((job) => job.id !== response.job.id)].slice(0, 10));
      setSnapshotMessage(response.owner_message ?? tf('app.debugJobStarted', errorLocale, { id: response.job.id.slice(0, 12) }));
      window.setTimeout(() => {
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        void fetchVNextDebugJobs(initData)
          .then((jobsResponse) => {
            if (isPrivateSessionEpochCurrent(sessionEpoch)) setDebugJobs(jobsResponse.jobs);
          })
          .catch(() => undefined);
      }, 2500);
    } catch (error) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
      setSnapshotMessage(displayError(error, t('error.fallback.couldNotStartBackendDebugJob', errorLocale), errorLocale));
    } finally {
      if (isPrivateSessionEpochCurrent(sessionEpoch)) setDebugJobBusy(false);
    }
  }

  /** Owner-visible copy for a voice-note failure code. */
  function voiceNoteErrorMessage(code: VoiceNoteErrorCode): string {
    switch (code) {
      case 'permission':
        return t('chatHome.voiceNote.errorPermission', errorLocale);
      case 'unsupported':
        return t('chatHome.voiceNote.errorUnsupported', errorLocale);
      case 'empty':
        return t('chatHome.voiceNote.errorEmpty', errorLocale);
      case 'upload':
        return t('chatHome.voiceNote.errorUpload', errorLocale);
      case 'record':
      default:
        return t('chatHome.voiceNote.errorRecord', errorLocale);
    }
  }

  function applyVoiceNoteEvent(event: Parameters<typeof voiceNoteReducer>[1]) {
    setChatHomeVoiceNote((current) => voiceNoteReducer(current, event));
  }

  /** Display projection of the voice-note machine for the composer. */
  const chatHomeVoiceNoteView: ChatHomeVoiceNote = (() => {
    switch (chatHomeVoiceNote.status) {
      case 'recording':
        return { status: 'recording', elapsedLabel: formatVoiceElapsed(chatHomeVoiceNote.elapsedMs) };
      case 'uploading':
        return { status: 'uploading' };
      case 'attached':
        return {
          status: 'attached',
          // The pill previews the transcript; the FULL text is what gets sent.
          transcript: voiceNoteTranscriptPreview(chatHomeVoiceNote.transcript),
          durationLabel: formatVoiceDuration(chatHomeVoiceNote.durationSecs),
        };
      case 'error':
        return { status: 'error', message: voiceNoteErrorMessage(chatHomeVoiceNote.code) };
      default:
        return { status: 'idle' };
    }
  })();

  /**
   * Composer mic → record a VOICE NOTE.
   *
   * The old behaviour here was browser dictation (`SpeechRecognition` typing
   * into the text field): unstable on iOS and not what the owner asked for. He
   * wants what Telegram does — speak, send the audio, and have Aegis
   * understand it. So: `MediaRecorder` → `/webapp/vnext/upload` → the server
   * transcribes with local whisper. The composer holds the transcript as a
   * pending voice attachment until Send.
   */
  async function startChatHomeVoiceNote() {
    if (chatHomeVoiceNote.status === 'recording' || chatHomeVoiceNote.status === 'uploading') return;
    setChatHomeNotice('');
    setSnapshotMessage('');
    // A voice note replaces any held file attachment (one attachment per turn).
    setChatHomePendingAttachment(null);
    setChatHomeAttachmentError('');
    applyVoiceNoteEvent({ type: 'start' });
    const recorderApi = (window as unknown as {
      MediaRecorder?: {
        new (stream: MediaStream, options?: { mimeType?: string }): MediaRecorder;
        isTypeSupported?: (mime: string) => boolean;
      };
    }).MediaRecorder;
    if (!recorderApi || !navigator.mediaDevices?.getUserMedia) {
      applyVoiceNoteEvent({ type: 'failed', code: 'unsupported' });
      return;
    }
    if (!voiceMicSessionRef.current) {
      voiceMicSessionRef.current = createVoiceMicSession({
        getUserMedia: () => navigator.mediaDevices.getUserMedia({ audio: true }),
        setTimer: (callback, afterMs) => window.setTimeout(callback, afterMs),
        clearTimer: (handleId) => window.clearTimeout(handleId),
      });
    }
    const micSession = voiceMicSessionRef.current;
    // Recording replaces the text row, so whatever was typed when the note
    // started is still there when the transcript lands.
    const typedBeforeRecording = chatHomeCommand.trim();
    const handle = await startVoiceNoteRecording(
      {
        getUserMedia: () => micSession.acquire(),
        releaseStream: (stream) => micSession.release(stream),
        createRecorder: (stream, mimeType) =>
          new recorderApi(
            stream as unknown as MediaStream,
            mimeType ? { mimeType } : undefined,
          ) as unknown as VoiceNoteRecorderLike,
        isTypeSupported: recorderApi.isTypeSupported?.bind(recorderApi),
        makeFile: (chunks, filename, mimeType) =>
          new File(chunks as BlobPart[], filename, { type: mimeType }),
        now: () => Date.now(),
        schedule: (callback, everyMs) => window.setInterval(callback, everyMs),
        unschedule: (handleId) => window.clearInterval(handleId),
      },
      {
        onElapsed: (elapsedMs) => applyVoiceNoteEvent({ type: 'tick', elapsedMs }),
        onComplete: (file) => {
          applyVoiceNoteEvent({ type: 'stop' });
          void uploadChatHomeVoiceNote(file, typedBeforeRecording);
        },
        onError: (code) => {
          voiceNoteRecorderRef.current = null;
          applyVoiceNoteEvent({ type: 'failed', code });
        },
      },
    );
    // A null handle means startVoiceNoteRecording already reported the failure
    // through onError — do NOT reset to idle here, that would wipe the error
    // pill the owner needs to see.
    voiceNoteRecorderRef.current = handle;
  }

  /**
   * Stop recording and send the audio for transcription. The composer flips
   * to "正在转写" on the tap itself: waiting for the browser's `onstop` left
   * the row frozen on "正在录音" for a beat (owner: "点发送没反应").
   */
  function stopChatHomeVoiceNote() {
    const handle = voiceNoteRecorderRef.current;
    voiceNoteRecorderRef.current = null;
    if (!handle) {
      applyVoiceNoteEvent({ type: 'cancel' });
      return;
    }
    applyVoiceNoteEvent({ type: 'stop' });
    handle.stop();
  }

  /** Discard the recording (or the held note) without sending anything. */
  function cancelChatHomeVoiceNote() {
    const handle = voiceNoteRecorderRef.current;
    voiceNoteRecorderRef.current = null;
    voiceUploadGenerationRef.current += 1;
    handle?.cancel();
    applyVoiceNoteEvent({ type: 'cancel' });
  }

  /**
   * Upload the recorded audio; the SERVER transcribes it. Failure is always
   * visible (never a silent drop): 503 = speech-to-text is not configured on
   * the host, 422 = the recording carried no speech.
   */
  async function uploadChatHomeVoiceNote(file: File, typedBeforeRecording = '') {
    const generation = voiceUploadGenerationRef.current;
    try {
      const result = await uploadVNextAttachment(initData, session?.csrf_token, file);
      // The owner cancelled while the transcript was in flight.
      if (generation !== voiceUploadGenerationRef.current) return;
      if (result.type !== 'voice' || !result.transcript?.trim()) {
        applyVoiceNoteEvent({ type: 'failed', code: 'empty' });
        return;
      }
      const attached: VoiceNoteState = {
        status: 'attached',
        transcript: result.transcript.trim(),
        durationSecs: typeof result.duration_secs === 'number' ? result.duration_secs : null,
      };
      // Speak → it goes (what Telegram does). The transcript shows in the
      // owner's bubble, so a mis-hearing is visible and can be corrected with
      // the next message. Typed text beside a note still waits for Send so the
      // caption and the note travel together.
      if (!typedBeforeRecording) {
        // The turn carries the note itself (voiceNoteOverride); the composer
        // must not keep a copy, or a queued turn would offer it a second time.
        setChatHomeVoiceNote(IDLE_VOICE_NOTE);
        void submitChatHomeCommand(
          attached.transcript,
          undefined,
          undefined,
          'voice',
          undefined,
          undefined,
          undefined,
          attached,
        );
        return;
      }
      applyVoiceNoteEvent({ type: 'uploaded', transcript: attached.transcript, durationSecs: attached.durationSecs });
      setChatHomeInputMode('voice');
    } catch (error) {
      if (generation !== voiceUploadGenerationRef.current) return;
      const status = error instanceof ApiResponseError ? error.statusCode : 0;
      applyVoiceNoteEvent({
        type: 'failed',
        code: status === 422 ? 'empty' : status === 503 ? 'unsupported' : 'upload',
      });
    }
  }

  function updateChatHomeCommand(value: string) {
    setChatHomeInputMode('text');
    setChatHomeCommand(value);
  }

  /** Structured page context for Realtime tools + chat asks (~2 KB, no screenshots). */
  function buildCeoScreenContext(): string {
    const lines: string[] = [];
    lines.push(`surface=${view}`);
    if (view === 'home') {
      const needsYou = (home?.decisions ?? []).concat(home?.now ?? []).slice(0, 5);
      if (needsYou.length === 0) {
        lines.push('needs_you=(none)');
      } else {
        lines.push('needs_you:');
        for (const item of needsYou) {
          lines.push(
            `- id=${item.id} status=${item.status} title=${ownerVisibleWorkTitle(item, item.title || item.id, 80, errorLocale)}`,
          );
        }
      }
    } else if (view === 'work') {
      if (selectedId) lines.push(`focus_item_id=${selectedId}`);
      if (selected) {
        lines.push(
          `focus_title=${ownerVisibleWorkTitle(selected, selected.title || selected.id, 120, errorLocale)}`,
        );
        lines.push(`focus_status=${selected.status}`);
        const summary = (selected.summary || '').replace(/\s+/g, ' ').trim().slice(0, 240);
        if (summary) lines.push(`focus_summary=${summary}`);
        const next = ownerVisibleNextSafeActionText(selected, errorLocale).replace(/\s+/g, ' ').trim().slice(0, 160);
        if (next) lines.push(`next_safe_action=${next}`);
        lines.push(`lane=${laneForItem(selected)}`);
      } else {
        lines.push('focus_item=(none)');
      }
    } else if (view === 'status') {
      if (statusProjection?.llm_degraded) {
        lines.push(`health_llm=degraded since=${statusProjection.llm_degraded.since || ''}`);
      }
      if (statusProjection?.embedder_degraded) {
        lines.push(`health_embedder=degraded since=${statusProjection.embedder_degraded.since || ''}`);
      }
      const snapshotTelegramRail = readTelegramRail(statusProjection);
      if (snapshotTelegramRail) {
        lines.push(`health_telegram=${snapshotTelegramRail.state} since_unix=${snapshotTelegramRail.since_unix} degraded=${snapshotTelegramRail.degraded ? '1' : '0'}`);
      }
      const snapshotNightly = readNightlyStatus(statusProjection);
      if (snapshotNightly) {
        lines.push(`nightly=${snapshotNightly.verdict} sha=${snapshotNightly.sha || '?'} finished_at_unix=${snapshotNightly.finished_at_unix}`);
      }
      const sections = statusProjection?.sections ?? [];
      if (sections.length === 0) {
        lines.push('status_sections=(none)');
      } else {
        lines.push('status_sections:');
        for (const section of sections.slice(0, 12)) {
          const msg = (section.owner_message || '').replace(/\s+/g, ' ').trim().slice(0, 120);
          lines.push(`- ${section.label} health=${section.health}${msg ? ` msg=${msg}` : ''}`);
        }
      }
    } else if (view === 'settings') {
      if (settingsProjection) {
        lines.push(`settings.display_language=${settingsProjection.display_language || ''}`);
        lines.push(`settings.user_timezone=${settingsProjection.user_timezone || ''}`);
        lines.push(`settings.llm_provider=${settingsProjection.llm_provider || ''}`);
        lines.push(`settings.llm_model=${settingsProjection.llm_model || ''}`);
        lines.push(`settings.paid_api_fallback=${String(Boolean(settingsProjection.paid_api_fallback_enabled))}`);
        lines.push(`settings.cost_saving_paused=${String(Boolean(settingsProjection.cost_saving_paused))}`);
        for (const section of (settingsProjection.connected_services ?? []).slice(0, 8)) {
          lines.push(`settings.service.${section.label}=${section.health}`);
        }
      } else {
        lines.push('settings=(loading)');
      }
    }
    return truncateScreenContext(lines.join('\n'));
  }

  function applyCeoUiNavigation(surface: CeoUiSurface) {
    if (surface === 'home') navigate('home', null);
    else if (surface === 'work') navigate('work', null);
    else if (surface === 'status') navigate('status', null);
    else if (surface === 'settings') navigate('settings', null);
  }

  function ceoUiOpenedMessage(surface: CeoUiSurface): string {
    if (errorLocale === 'en') return `Opened ${surface} view for the owner.`;
    const label =
      surface === 'work' ? '工作'
        : surface === 'home' ? '首页'
          : surface === 'settings' ? '设置'
            : surface === 'status' ? '状态'
              : surface;
    return `已为你切换到${label}界面。`;
  }

  function trackVoiceGate(card: { approvalId: string; toolName: string; summary: string }) {
    pendingVoiceGatesRef.current = pendingVoiceGatesRef.current
      .filter((g) => g.approvalId !== card.approvalId)
      .concat(card)
      .slice(-6);
    ceoVoiceAgentRef.current?.notifyApproval?.(card);
    const query = voiceApprovalQuery(card.summary)
      || (errorLocale === 'en' ? 'This search' : '这次搜索');
    setVoiceApproval({ approvalId: card.approvalId, query });
  }
  trackVoiceGateRef.current = trackVoiceGate;

  async function settleVoiceApproval(
    approvalId: string,
    action: 'approve' | 'reject',
    options?: { continueCall?: boolean },
  ): Promise<{ ok: boolean; message: string } | undefined> {
    const continueCall = options?.continueCall !== false;
    if (voiceApprovalSettledRef.current.has(approvalId)) {
      return { ok: true, message: recentVoiceDecisionRef.current?.line || '' };
    }
    voiceApprovalSettledRef.current.add(approvalId);
    const holdingLine = action === 'reject' ? '先不查了。' : '已批准，正在继续查。';
    recentVoiceDecisionRef.current = { at: Date.now(), action, line: holdingLine };
    setVoiceApproval((current) => (current?.approvalId === approvalId ? null : current));
    ceoVoiceAgentRef.current?.settleApproval?.(approvalId, action === 'approve' ? 'approved' : 'rejected');
    const result = await handleChatHomeApprovalAction(approvalId, action);
    if (!result) return undefined;
    if (!result.ok) {
      voiceApprovalSettledRef.current.delete(approvalId);
      return result;
    }
    const line = ownerContinuationLine(result.message, action);
    recentVoiceDecisionRef.current = { at: Date.now(), action, line };
    if (continueCall) {
      const continued = ceoVoiceAgentRef.current?.continueTurn(
        voiceContinuationPrompt(line, result.message),
      ) ?? false;
      if (!continued) settleVoiceUtterance('aegis', line);
    }
    return { ok: true, message: line };
  }

  async function settlePendingVoiceGate(action: 'approve' | 'reject') {
    const gate = takeLatestVoiceGate();
    if (!gate) return null;
    const settled = await settleVoiceApproval(gate.approvalId, action, { continueCall: false });
    return {
      ok: settled?.ok !== false,
      message: settled?.message || (action === 'reject' ? '先不查了。' : '已批准，正在继续查。'),
    };
  }

  function takeLatestVoiceGate(): { approvalId: string; toolName: string; summary: string } | null {
    const list = pendingVoiceGatesRef.current;
    if (list.length === 0) return null;
    const gate = list[list.length - 1]!;
    pendingVoiceGatesRef.current = list.slice(0, -1);
    return gate;
  }

  function resolveSecretaryItem(itemId?: unknown): OwnerWorkItem | null {
    const id = typeof itemId === 'string' && itemId.trim() ? itemId.trim() : selectedId;
    if (!id || !queue) return selected ?? null;
    return queue.items.find((item) => item.id === id) ?? selected ?? null;
  }

  /**
   * Panel-primary secretary ask: same runtime as Chat.
   * - Does NOT navigate to home or paint working bubbles on Desk
   * - Always injects current view as screen context
   * - Dual-writes owner/aegis to L1 history (capability preserved)
   * - SSE progress + need_approval HITL via options callbacks
   * - Optional attachments from pending home upload
   */
  async function secretaryAsk(
    text: string,
    mode: 'text' | 'voice' = 'text',
    options?: SecretaryAskOptions,
  ): Promise<SecretaryAskResult> {
    const typed = text.trim();
    // Capture what the composer is holding BEFORE the empty-text gate: an
    // attachment-only turn (voice note, photo, PDF) is a real message. It used
    // to die here with `{ kind: 'empty' }` and no feedback — the owner's
    // "attaching an image does nothing" bug.
    const requestAttachments = options?.includeAttachments
      ? chatHomeCommandAttachments(chatHomePendingAttachment)
      : [];
    const sentAttachment = chatHomeSentAttachment(requestAttachments);
    // A voice note's transcript IS the message when nothing was typed, so the
    // owner sees (and the runtime receives) his own words rather than a
    // placeholder caption.
    const voiceTranscript = sentAttachment?.kind === 'voice' ? sentAttachment.transcript.trim() : '';
    const trimmed = typed || voiceTranscript;
    if (!trimmed && requestAttachments.length === 0) return { kind: 'empty' };

    // Pure UI navigation: switch the shell view here.
    // Skipped for an attachment-only turn: there is no phrase to interpret, and
    // the file must reach the runtime.
    const navSurface = trimmed && requestAttachments.length === 0
      ? resolveUiNavigationIntent(trimmed)
      : null;
    if (navSurface) {
      applyCeoUiNavigation(navSurface);
      return { kind: 'replied', text: ceoUiOpenedMessage(navSurface) };
    }
    const navSplit = trimmed ? splitUiNavigationPrefix(trimmed) : null;
    if (navSplit) {
      applyCeoUiNavigation(navSplit.surface);
    }

    // Client shorthand expand for runtime text; panel may display the short word.
    const shorthand = resolveCeoShorthand(navSplit?.remainder ?? trimmed, errorLocale);
    const runtimeText = shorthand?.expansion ?? (navSplit?.remainder ?? trimmed);
    const displayText = trimmed;

    const sessionEpoch = privateSessionEpochRef.current;
    const screenContext = buildCeoScreenContext();
    const conversationContext = buildPanelConversationContext({
      screenContext,
      stream: options?.recentStream ?? [],
      excludeLatestUserText: displayText,
      maxTurns: 8,
    });
    // Fold Chat home history into context so panel doesn't lose long-thread capability.
    const historyTurns = chatHomeConversationContext(null).slice(-6);
    const mergedContext = mergeConversationContext(conversationContext, historyTurns);

    // Free-form panel/desk talk is always general runtime. Sticky selectedId
    // is situational screen context only (see buildCeoScreenContext) — never
    // force item-scoped focused turns (owner bug: research Q → passport task).
    const clientContext = mapViewToClientContext(view, selectedId);
    const focusId = GENERAL_ASSISTANT_FOCUS_ID;
    const commandIdempotencyKey = chatHomeIdempotencyKey();
    if (requestAttachments.length > 0) {
      setChatHomePendingAttachment(null);
      setChatHomeAttachmentError('');
      setChatHomeVoiceNote(IDLE_VOICE_NOTE);
    }

    const liveProgressLines: string[] = [];
    const paintPanelProgress = (nextLive?: string) => {
      if (nextLive) {
        const next = appendProgressLine(liveProgressLines, nextLive);
        liveProgressLines.splice(0, liveProgressLines.length, ...next);
      }
      const composed = composeWorkingText(
        liveProgressLines,
        false,
        '',
        liveProgressLines.length === 0 ? t(CHAT_HOME_WORKING_INITIAL_KEY, errorLocale) : liveProgressLines.join('\n'),
      );
      options?.onProgress?.(composed);
    };
    const turnStream: { close: (() => void) | null; settled: boolean } = { close: null, settled: false };
    let streamedReply = '';
    void mintVNextTurnStreamToken(initData, session?.csrf_token, commandIdempotencyKey)
      .then((streamToken) => {
        if (turnStream.settled) return;
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return;
        turnStream.close = openTurnStream(
          commandIdempotencyKey,
          streamToken,
          (event: TurnEvent) => {
            switch (event.type) {
              case 'tool_start':
                paintPanelProgress(workingPhraseForTool(event.tool_name, errorLocale));
                break;
              case 'tool_done':
                paintPanelProgress(toolDonePhrase(errorLocale));
                break;
              case 'vision_routed':
                paintPanelProgress(t('chatHome.working.processingImage', errorLocale));
                break;
              case 'intent_resolved':
                paintPanelProgress(
                  event.needs_review
                    ? tf('chatHome.working.intentReview', errorLocale, { label: event.label })
                    : tf('chatHome.working.intent', errorLocale, { label: event.label }),
                );
                break;
              case 'action_phase':
                paintPanelProgress(workingPhraseForPhase(event.phase, event.tool_name, event.detail, errorLocale));
                break;
              case 'owner_notice':
                if (event.text.trim()) paintPanelProgress(workingNotice(event.text.trim(), event.kind, errorLocale));
                break;
              case 'need_approval':
                if (event.approval_id) {
                  options?.onApproval?.({
                    approvalId: event.approval_id,
                    toolName: event.tool_name || 'tool',
                    summary: event.summary || '',
                  });
                }
                paintPanelProgress(workingPhraseForPhase('approval_required', event.tool_name, null, errorLocale));
                break;
              case 'assistant_delta':
                if (!event.text) break;
                streamedReply = appendAssistantDelta(streamedReply, event.text);
                paintPanelProgress();
                options?.onProgress?.(streamedReply);
                break;
              case 'assistant_draft_reset':
                streamedReply = '';
                paintPanelProgress();
                break;
              default:
                break;
            }
          },
          () => {
            turnStream.close = null;
          },
        );
        if (turnStream.settled) {
          turnStream.close?.();
          turnStream.close = null;
        }
      })
      .catch(() => {
        /* advisory */
      });

    try {
      let commandCsrfToken = session?.csrf_token;
      if (!commandCsrfToken && session?.authenticated && session?.principal_type === 'web_console') {
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return { kind: 'skipped' };
        commandCsrfToken = refreshed?.csrf_token;
      }

      const runCommand = (token: string | undefined) => requestAssistantCommandWithAutoCheck({
        initData,
        csrfToken: token,
        focusId,
        intent: 'chat',
        text: runtimeText,
        scope: 'general',
        clientContext,
        mode,
        idempotencyKey: commandIdempotencyKey,
        conversationContext: mergedContext,
        attachments: requestAttachments,
        onStillWorking: () => {
          if (liveProgressLines.length === 0) {
            paintPanelProgress(t('chatHome.working.stillChecking', errorLocale));
          }
        },
      });

      let command: Awaited<ReturnType<typeof runCommand>>;
      try {
        command = await runCommand(commandCsrfToken);
      } catch (firstCommandError) {
        if (!isRefreshableSessionActionError(firstCommandError)) throw firstCommandError;
        const refreshed = await refreshSessionForAction(sessionEpoch);
        if (!isPrivateSessionEpochCurrent(sessionEpoch)) return { kind: 'skipped' };
        command = await runCommand(refreshed?.csrf_token);
      }
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return { kind: 'skipped' };

      const rawAnswer = command.response.visible_answer || '';
      const ownerAnswer = scrubToolWaitDump(rawAnswer);
      const ownerTimestamp = Date.now();
      // Quiet L1 dual-write: preserve history + unseen badge without stealing focus.
      setChatHomeMessages((current) => {
        const withoutTransient = current.filter(
          (message) => message.role !== 'working' && message.role !== 'queued',
        );
        const turnId = command.response.turn_id || commandIdempotencyKey;
        const withOwner = withoutTransient.some((message) => message.role === 'owner' && message.turnId === turnId)
          ? withoutTransient
          : withoutTransient.concat({
            id: `owner:${ownerTimestamp}:panel`,
            turnId,
            role: 'owner' as const,
            text: displayText,
            timestamp: ownerTimestamp,
            ...(sentAttachment ? { attachment: sentAttachment } : {}),
          });
        if (!ownerAnswer) return withOwner;
        return completeChatHomeTurn(withOwner, displayText, ownerTimestamp, {
          id: `aegis:${Date.now()}:panel`,
          turnId,
          replyOrigin: 'command',
          role: 'aegis' as const,
          text: ownerAnswer,
          severity: command.response.severity ?? null,
          timestamp: Date.now(),
          result: { ...command.response, visible_answer: ownerAnswer },
          workItem: command.workItem,
        });
      });

      const refreshFocus = command.workItem?.id;
      void refreshQueueAfterAssistant(refreshFocus);
      return { kind: 'replied', text: rawAnswer };
    } catch (err) {
      if (!isPrivateSessionEpochCurrent(sessionEpoch)) return { kind: 'skipped' };
      return {
        kind: 'error',
        text: assistantDisplayError(err, errorLocale),
      };
    } finally {
      turnStream.settled = true;
      turnStream.close?.();
      turnStream.close = null;
    }
  }

  /**
   * Desk / sheet talk entry — one ChatHomeView composer path.
   * Opens the sheet when the owner is on Work/Status/Settings so the
   * reply paints on the shared timeline (no floating text panel).
   */
  function decideChatApprovalFromUtterance(action: 'approve' | 'reject'): boolean {
    const messages = chatHomeMessagesRef.current;
    const pending = [...messages].reverse().find((message) => (
      message.role === 'approval'
      && Boolean(message.approvalId)
      && (message.approvalState ?? 'pending') === 'pending'
    ));
    if (pending?.approvalId) {
      void handleChatHomeApprovalAction(pending.approvalId, action);
      return true;
    }
    if (action !== 'approve') return false;
    const decidedSearch = [...messages].reverse().find((message) => (
      message.role === 'approval'
      && message.approvalState === 'approved'
      && isSearchHold(message.summary, message.toolName)
    ));
    if (!decidedSearch) return false;
    setChatHomeNotice(t('chatHome.approval.alreadyDecided', errorLocale));
    setSnapshotMessage('');
    return true;
  }

  function deskTalkToAegis(text: string, mode: 'text' | 'voice' = 'text') {
    const trimmed = text.trim();
    const decision = voicePendingDecision(trimmed);
    if (decision && decideChatApprovalFromUtterance(decision)) return;
    if (!trimmed && !chatHomeHasSendableAttachment()) return;
    if (view !== 'home') setChatSheetOpen(true);
    void submitChatHomeCommand(trimmed || undefined, undefined, undefined, mode);
  }

  // Keep handlers fresh for Realtime tool dispatch without rebinding WS.
  secretaryHandlersRef.current = {
    get_screen_context: () => ({
      ok: true,
      message: buildCeoScreenContext(),
    }),
    what_needs_me: () => {
      const items = (home?.decisions ?? []).concat(home?.now ?? []).slice(0, 5);
      if (items.length === 0) {
        return { ok: true, message: errorLocale === 'en' ? 'Nothing needs you right now.' : '现在没有需要你拍板的事。' };
      }
      const lines = items.map(
        (item, index) =>
          `${index + 1}. ${ownerVisibleWorkTitle(item, item.title || item.id, 100, errorLocale)} (${item.status})`,
      );
      return { ok: true, message: lines.join('\n') };
    },
    open_surface: (args) => {
      const surface = typeof args.surface === 'string' ? args.surface : 'home';
      const itemId = typeof args.itemId === 'string' ? args.itemId : null;
      if (surface === 'work') {
        navigate('work', itemId);
        return { ok: true, message: ceoUiOpenedMessage('work') };
      }
      if (
        surface === 'home'
        || surface === 'status'
        || surface === 'settings'
      ) {
        applyCeoUiNavigation(surface);
        return { ok: true, message: ceoUiOpenedMessage(surface) };
      }
      return { ok: false, message: `Unknown surface: ${surface}` };
    },
    ask_chief: async (args) => {
      const text = typeof args.text === 'string' ? args.text : '';
      if (!text.trim()) return { ok: false, message: 'Empty request' };
      // Deterministic UI navigation: intercept pure nav intents here.
      // (secretaryAsk also navigates; we short-circuit pure nav for a crisp tool receipt.)
      const navSurface = resolveUiNavigationIntent(text);
      if (navSurface) {
        applyCeoUiNavigation(navSurface);
        return { ok: true, message: ceoUiOpenedMessage(navSurface) };
      }
      // 「批准。」 / 「先不要」 while a search gate is waiting. Punctuation is
      // not a new chat turn, and a decision the mic already settled must not
      // be asked again.
      const decision = voicePendingDecision(text);
      if (decision) {
        const gate = pendingVoiceGatesRef.current.length > 0 ? takeLatestVoiceGate() : null;
        if (gate) {
          const settled = await settleVoiceApproval(gate.approvalId, decision, { continueCall: false });
          return {
            ok: settled?.ok !== false,
            message: settled?.message || (decision === 'reject' ? '先不查了。' : '已批准，正在继续查。'),
          };
        }
        const recent = recentVoiceDecisionRef.current;
        if (recent && recent.action === decision && Date.now() - recent.at < 8000) {
          return { ok: true, message: recent.line };
        }
      }
      let sawGate = false;
      const result = await secretaryAsk(text, 'voice', {
        onApproval: (card) => {
          sawGate = true;
          trackVoiceGate(card);
        },
      });
      if (result.kind === 'replied') {
        const base = result.text || (errorLocale === 'en' ? 'Done.' : '处理好了。');
        if (sawGate) {
          return { ok: true, message: voiceGateToolMessage(base, errorLocale) };
        }
        return { ok: true, message: base };
      }
      if (result.kind === 'error') {
        return { ok: false, message: result.text };
      }
      if (result.kind === 'empty') {
        return { ok: false, message: errorLocale === 'en' ? 'Empty request' : '请求为空' };
      }
      if (sawGate) {
        return { ok: true, message: voiceGateToolMessage('', errorLocale) };
      }
      return {
        ok: true,
        message: errorLocale === 'en' ? 'Handled.' : '已处理。',
      };
    },
    approve_pending: async (args) => {
      if (pendingVoiceGatesRef.current.length > 0 && !(typeof args.itemId === 'string' && args.itemId.trim())) {
        const settled = await settlePendingVoiceGate('approve');
        if (settled) return settled;
      }
      const item = resolveSecretaryItem(args.itemId);
      if (!item) {
        if (pendingVoiceGatesRef.current.length > 0) {
          const settled = await settlePendingVoiceGate('approve');
          if (settled) return settled;
        }
        return { ok: false, message: errorLocale === 'en' ? 'No item to approve.' : '没有可批准的待办。' };
      }
      const action = buildSupportedItemActions(item, errorLocale).find((a) => a.id === 'approval.approve');
      if (!action) {
        if (pendingVoiceGatesRef.current.length > 0) {
          const settled = await settlePendingVoiceGate('approve');
          if (settled) return settled;
        }
        return { ok: false, message: errorLocale === 'en' ? 'This item is not an approval.' : '这项不是待批准的待办。' };
      }
      const result = await executeConfirmedItemAction(item, action);
      return { ok: result.ok, message: result.message };
    },
    reject_pending: async (args) => {
      if (pendingVoiceGatesRef.current.length > 0 && !(typeof args.itemId === 'string' && args.itemId.trim())) {
        const settled = await settlePendingVoiceGate('reject');
        if (settled) return settled;
      }
      const item = resolveSecretaryItem(args.itemId);
      if (!item) {
        if (pendingVoiceGatesRef.current.length > 0) {
          const settled = await settlePendingVoiceGate('reject');
          if (settled) return settled;
        }
        return { ok: false, message: errorLocale === 'en' ? 'No item to reject.' : '没有可拒绝的待办。' };
      }
      const action = buildSupportedItemActions(item, errorLocale).find((a) => a.id === 'approval.reject');
      if (!action) {
        if (pendingVoiceGatesRef.current.length > 0) {
          const settled = await settlePendingVoiceGate('reject');
          if (settled) return settled;
        }
        return { ok: false, message: errorLocale === 'en' ? 'This item is not an approval.' : '这项不是待批准的待办。' };
      }
      const result = await executeConfirmedItemAction(item, action);
      return { ok: result.ok, message: result.message };
    },
    complete_task: async (args) => {
      const item = resolveSecretaryItem(args.itemId);
      if (!item) return { ok: false, message: errorLocale === 'en' ? 'No task focused.' : '没有聚焦的任务。' };
      const action = buildSupportedItemActions(item, errorLocale).find(
        (a) => a.id === 'task.complete' || a.id === 'knowledge.done' || a.id === 'tracking.cancel',
      );
      if (!action) return { ok: false, message: errorLocale === 'en' ? 'Cannot complete this item.' : '这项不能直接完成。' };
      const result = await executeConfirmedItemAction(item, action);
      return { ok: result.ok, message: result.message };
    },
    draft_reply: async (args) => {
      const item = resolveSecretaryItem(args.itemId);
      const instruction = typeof args.instruction === 'string' ? args.instruction.trim() : '';
      const prompt = instruction
        ? `${errorLocale === 'en' ? 'Draft a reply' : '起草回复'}：${instruction}`
        : errorLocale === 'en'
          ? 'Draft a reply for the focused thread. Do not send.'
          : '针对当前焦点起草回复，先不要发送。';
      if (item) {
        setChatHomeAttachedContext({ itemId: item.id, label: ownerVisibleWorkTitle(item, item.title || item.id, 80, errorLocale) });
      }
      await secretaryAsk(prompt, 'voice', {
        onApproval: (card) => trackVoiceGate(card),
      });
      return { ok: true, message: errorLocale === 'en' ? 'Drafting…' : '正在起草…' };
    },
  };

  // Failsafe busy-release. `busy` is GLOBAL UX state: it gates onSubmit's
  // queue-vs-send branch and the queue-drain effect below. The per-request
  // finally blocks only clear it when their epoch/generation still matches, so
  // that a stale request never releases a NEWER one's busy=true. The hole that
  // leaves: if the epoch/generation advances (reconnect, re-auth, logout,
  // conversation clear) while a request is IN-FLIGHT, that request's finally is
  // skipped and busy wedges true forever — every later message then silently
  // queues. We close it here by force-clearing busy whenever the session is torn
  // down. Safe against double-send: a fresh request only starts after the reset
  // (its own setBusy(true)), and its finally now matches the NEW epoch/generation
  // so the normal guarded release applies; this effect never fires mid-fresh-
  // request because the tick only changes on a session teardown.
  useEffect(() => {
    if (!shouldForceReleaseBusyOnSessionReset(sessionResetTick)) return;
    setBusy(false);
  }, [sessionResetTick]);

  useEffect(() => {
    if (busy || !session?.authenticated) return;
    if (queuedChatHomeCommands.length > 0) {
      const [nextCommand, ...remaining] = queuedChatHomeCommands;
      setQueuedChatHomeCommands(remaining);
      void submitChatHomeCommand(
        nextCommand.text,
        nextCommand.retryIdempotencyKey,
        nextCommand.context ?? undefined,
        nextCommand.mode,
        nextCommand.displayText,
        nextCommand.queuedMessageId,
        nextCommand.quote,
        undefined,
        nextCommand.attachments,
      );
      return;
    }
    if (queuedChatHomeWorkPrompts.length > 0) {
      const [nextPrompt, ...remaining] = queuedChatHomeWorkPrompts;
      setQueuedChatHomeWorkPrompts(remaining);
      void askChatHomeWorkItem(
        nextPrompt.messageId,
        nextPrompt.item,
        nextPrompt.text,
        nextPrompt.queuedThreadId,
      );
    }
  }, [busy, queuedChatHomeCommands, queuedChatHomeWorkPrompts, session?.authenticated]);

  if (state === 'loading') {
    return (
      <CenterState
        kicker={t('app.shell.loading.kicker', errorLocale)}
        title={t('app.shell.loading.title', errorLocale)}
        body={t('app.shell.loading.body', errorLocale)}
      />
    );
  }

  if (state === 'error') {
    return (
      <CenterState
        tone="error"
        kicker={t('app.shell.error.kicker', errorLocale)}
        title={t('app.shell.error.title', errorLocale)}
        body={error}
        action={(
          <button type="button" onClick={() => window.location.reload()}>{t('app.shell.error.retry', errorLocale)}</button>
        )}
      />
    );
  }

  if (!session?.authenticated) {
    return (
      <CenterState
        tone="auth"
        kicker={t('app.shell.auth.kicker', errorLocale)}
        title={t('app.shell.auth.title', errorLocale)}
        body={t(session?.passkey_login_enabled || session?.google_login_enabled ? 'app.shell.auth.body' : 'app.shell.auth.localBody', errorLocale)}
        action={(
          <>
            {session?.passkey_login_enabled === true && <a className="vnext-primary-link" href="/access">Passkey</a>}
            {session?.google_login_enabled === true && <a className="vnext-primary-link" href={GOOGLE_LOGIN_PATH}>{t('app.shell.auth.continueWithGoogle', errorLocale)}</a>}
          </>
        )}
      />
    );
  }

  const telegramRail = readTelegramRail(statusProjection);

  return (
    <UserTimezoneProvider value={settingsProjection?.user_timezone ?? null}>
    <LocaleProvider displayLanguage={settingsProjection?.display_language ?? null}>
    <div className={classNames('vnext-app', `vnext-app-${view}`, view === 'home' && 'vnext-app-chat-home')}>
      <a className="vnext-skip-link" href="#vnext-main-content">{t('app.shell.skipToMain', errorLocale)}</a>
      <div className="vnext-sr-live" aria-live="polite" role="status">{surfaceTitle(view, errorLocale)}</div>
      {/* Offline first: nothing below can refresh without a network, so the
          runtime banners step aside until the browser reports `online`. */}
      <OfflineBanner offline={!online} />
      {online && (
        <>
          <LlmDegradedBanner
            degraded={statusProjection?.llm_degraded}
            dismissedSince={llmDegradedDismissedSince}
            onDismiss={dismissLlmDegradedBanner}
          />
          {/* Single degraded-banner slot: the mobile layout reserves exactly ONE
              banner row (`:has(.vnext-llm-degraded-banner)` height carve-out in
              styles.css), so when the strictly-more-severe LLM banner is VISIBLE
              the embedder banner yields the slot. It surfaces as soon as the LLM
              banner clears (recovery or owner dismissal) — never two stacked
              banners clipping the chat composer during a double outage
              (Codex S2 review). The Telegram rail (a dead bot = no phone
              notifications at all) outranks the embedder (duplicate reminders)
              for the same slot. */}
          {!(statusProjection?.llm_degraded
            && llmDegradedDismissedSince !== statusProjection.llm_degraded.since) && (
            telegramRailBannerState(telegramRail, telegramRailDismissedSince, nowUnixSeconds()) ? (
              <TelegramRailBanner
                rail={telegramRail}
                dismissedSinceUnix={telegramRailDismissedSince}
                onDismiss={dismissTelegramRailBanner}
              />
            ) : (
              <EmbedderDegradedBanner
                degraded={statusProjection?.embedder_degraded}
                dismissedSince={embedderDegradedDismissedSince}
                onDismiss={dismissEmbedderDegradedBanner}
              />
            )
          )}
        </>
      )}
      {/* Primary rail is always mounted (including chat home) so desktop owners
          never lose Work paths. Mobile still hides the rail via CSS and
          uses the shared bottom nav. Destination set is owned by
          ownerPrimaryNavItemIds() so home and non-home cannot drift. */}
      <aside className="vnext-brand-rail" aria-label={t('app.shell.ownerNavigation', errorLocale)}>
        <div className="vnext-brand-lockup">
          <AegisAvatar src={avatarProfile.aegisAvatarUrl} className="vnext-shield-mark" />
          <div>
            <strong>{t('app.shell.brand', errorLocale)}</strong>
            <span>{t('app.shell.tagline.secretary', errorLocale)}</span>
            <span>{t('app.shell.tagline.timeProtected', errorLocale)}</span>
          </div>
        </div>
        <p className="vnext-brand-copy">
          {t('app.shell.brandCopy', errorLocale)}
        </p>
        <nav className="vnext-rail-nav" aria-label={t('app.shell.nav.primaryDesktop', errorLocale)}>
          {ownerPrimaryNavItemIds().map((navId) => (
            navId === 'home' ? (
              <NavButton
                key={navId}
                view="home"
                activeView={view}
                label={t('app.shell.nav.chat', errorLocale)}
                icon="home"
                onClick={() => navigate('home', null)}
                dot={unseenChatActivity}
                badgeAria={unseenChatActivity ? t('app.shell.nav.chatUnseenAria', errorLocale) : undefined}
              />
            ) : navId === 'work' ? (
              <NavButton
                key={navId}
                view="work"
                activeView={view}
                label={t('app.shell.nav.work', errorLocale)}
                icon="work"
                onClick={() => navigate('work')}
                badgeCount={workBadgeCount}
                badgeTone={decisionCount > 0 ? 'warn' : 'default'}
                badgeAria={workBadgeCount > 0 ? workBadgeLabel : undefined}
              />
            ) : null
          ))}
        </nav>
        <div className="vnext-owner-card" aria-label={t('app.shell.ownerCard.label', errorLocale)}>
          <OwnerAvatar src={avatarProfile.ownerAvatarUrl} email={session.email} className="vnext-owner-avatar" />
          <div>
            <strong>{session.email || t('app.shell.ownerCard.youFallback', errorLocale)}</strong>
            <span>{t('app.shell.ownerCard.signedIn', errorLocale)}</span>
          </div>
        </div>
      </aside>

      <div className="vnext-shell" ref={shellRef}>
        {/* Top-bar Aegis avatar = global voice summon (replaces floating orb). */}
        {view !== 'home' && (
          <OwnerShellChrome
            variant="console"
            kicker={t('app.shell.header.kicker', errorLocale)}
            title={surfaceTitle(view, errorLocale)}
            subtitle={surfaceSubtitle(view, errorLocale)}
            summon={session?.authenticated ? {
              avatarUrl: avatarProfile.aegisAvatarUrl,
              live: voiceChrome?.live ?? false,
              agentSpeaking: voiceChrome?.agentSpeaking,
              userSpeaking: voiceChrome?.userSpeaking,
              fault: voiceChrome?.fault,
              statusLine: voiceChrome?.statusLine || undefined,
              voiceLabel: voiceChrome?.voiceLabel ?? t('voice.orb.start', errorLocale),
              voiceHint: voiceChrome?.voiceHint ?? t('voice.orb.hint', errorLocale),
              onToggleVoice: () => ceoVoiceAgentRef.current?.toggleVoice(),
            } satisfies OwnerShellSummon : null}
            menuTriggerLabel={t('app.shell.header.menu', errorLocale)}
            menuItems={[
              ...ownerUtilityMenuItemIds().map((id) => (
                id === 'status'
                  ? {
                      id,
                      label: t('app.shell.header.status', errorLocale),
                      icon: 'status' as const,
                      onSelect: () => navigate('status', null),
                    }
                  : {
                      id,
                      label: t('app.shell.header.settings', errorLocale),
                      icon: 'settings' as const,
                      onSelect: () => navigate('settings', null),
                    }
              )),
              ...(snapshotAvailable
                ? [{
                    id: 'snapshot',
                    label: snapshotBusy ? t('app.shell.header.saving', errorLocale) : t('app.shell.header.snapshot', errorLocale),
                    icon: 'camera' as const,
                    disabled: snapshotBusy,
                    onSelect: () => void createDebugSnapshot(),
                  }]
                : []),
              {
                id: 'logout',
                label: logoutBusy ? t('app.shell.header.signingOut', errorLocale) : t('app.shell.header.signOut', errorLocale),
                icon: 'logout' as const,
                tone: 'destructive' as const,
                disabled: logoutBusy,
                onSelect: () => void logoutNow(),
              },
            ] satisfies VNextHeaderMenuItem[]}
          />
        )}

        {view !== 'home' && (view !== 'settings' || routeSettingsSnapshotMessage(snapshotMessage) === null) && snapshotMessage && (
          <p
            className={classNames('vnext-snapshot-toast', snapshotTone === 'ok' && 'ok')}
            role="status"
          >
            {snapshotMessage}
          </p>
        )}

        <main className="vnext-main" id="vnext-main-content" tabIndex={-1} ref={mainRef}>
          {/* One ChatHomeView instance: full on Aegis tab, sheet when pulled up elsewhere.
              Sheet mode portals to document.body so locking `.vnext-shell` with
              overflow:hidden cannot clip the fixed sheet panel. */}
          {home && (() => {
            const sheetMode = chatSheetOpen && view !== 'home';
            const host = (
              <div
                className={classNames(
                  'vnext-chat-panel-host',
                  view === 'home' ? 'is-full' : sheetMode ? 'is-sheet vnext-app-chat-home' : 'is-dormant',
                )}
                aria-hidden={view !== 'home' && !chatSheetOpen}
                data-testid="aegis-chat-panel-host"
              >
                <ChatHomeView
                  presentationMode={view === 'home' ? 'full' : 'sheet'}
                  citeFocus={chatCiteFocus}
                  voiceLiveLine={voiceLiveLine}
                  voiceApproval={voiceApproval}
                  onVoiceApproval={(action) => {
                    if (!voiceApproval) return;
                    void settleVoiceApproval(voiceApproval.approvalId, action);
                  }}
                  onCloseSheet={() => setChatSheetOpen(false)}
                  email={session.email}
                  messages={earlierChatMessages.length > 0 ? [...earlierChatMessages, ...chatHomeMessages] : chatHomeMessages}
                  hasEarlier={chatHistoryHasMore}
                  loadingEarlier={loadingEarlierChat}
                  earlierError={earlierChatError}
                  onLoadEarlier={() => void loadEarlierChatHistory()}
                  brief={home.brief}
                  value={chatHomeCommand}
                  busy={busy}
                  snapshotMessage={workLoadNotice || chatHomeNotice || snapshotMessage}
                  contextSignals={home.signals}
                  needsYouItems={(() => {
                    const seen = new Set<string>();
                    const items: typeof home.decisions = [];
                    for (const item of [...home.decisions, ...home.now]) {
                      if (seen.has(item.id)) continue;
                      seen.add(item.id);
                      items.push(item);
                      if (items.length >= 3) break;
                    }
                    return items;
                  })()}
                  attachedContext={chatHomeAttachedContext}
                  workBadgeCount={workBadgeCount}
                  workBadgeLabel={workBadgeLabel}
                  workBadgeTone={decisionCount > 0 ? 'warn' : 'neutral'}
                  avatarProfile={avatarProfile}
                  onChange={updateChatHomeCommand}
                  onSubmit={(value) => {
                    const text = (value ?? chatHomeCommand).trim();
                    deskTalkToAegis(text, chatHomeInputMode);
                  }}
                  onSuggestion={(prompt, displayText) => {
                    // Send the expanded intent; the bubble shows the short label
                    // (the label alone — "关注" — was being sent as the message).
                    const runtime = prompt.trim();
                    if (!runtime) return;
                    if (view !== 'home') setChatSheetOpen(true);
                    void submitChatHomeCommand(runtime, undefined, undefined, 'text', displayText?.trim() || undefined);
                  }}
                  onClearConversation={clearChatHomeConversation}
                  onOpenWork={(itemId) => navigate('work', itemId ?? selectedId)}
                  onOpenStatus={() => navigate('status', null)}
                  onOpenSettings={() => navigate('settings', null)}
                  voiceSummon={session?.authenticated && voiceRealtimeAvailable ? {
                    avatarUrl: avatarProfile.aegisAvatarUrl,
                    live: voiceChrome?.live ?? false,
                    agentSpeaking: voiceChrome?.agentSpeaking,
                    userSpeaking: voiceChrome?.userSpeaking,
                    fault: voiceChrome?.fault,
                    statusLine: voiceChrome?.statusLine || undefined,
                    voiceLabel: voiceChrome?.voiceLabel ?? t('voice.orb.start', errorLocale),
                    voiceHint: voiceChrome?.voiceHint ?? t('voice.orb.hint', errorLocale),
                    onToggleVoice: () => ceoVoiceAgentRef.current?.toggleVoice(),
                  } satisfies OwnerShellSummon : null}
                  voiceNote={chatHomeVoiceNoteView}
                  onStartVoiceNote={() => void startChatHomeVoiceNote()}
                  onStopVoiceNote={stopChatHomeVoiceNote}
                  onCancelVoiceNote={cancelChatHomeVoiceNote}
                  onRetry={(text, idempotencyKey, context, quote) => {
                    if (view !== 'home') setChatSheetOpen(true);
                    void submitChatHomeCommand(text, idempotencyKey, context, 'text', undefined, undefined, quote);
                  }}
                  onApprovalAction={(approvalId, action) => void handleChatHomeApprovalAction(approvalId, action)}
                  onDecideApproval={decideChatApprovalFromUtterance}
                  onStopResearch={() => { void stopOwnerResearch(initData, session?.csrf_token); }}
                  quotedMessage={chatHomeQuotedMessage}
                  onQuoteMessage={(quote) => {
                    setChatHomeQuotedMessage(quote);
                    setChatHomeNotice('');
                    setSnapshotMessage('');
                  }}
                  onClearQuote={() => setChatHomeQuotedMessage(null)}
                  onAskWorkItem={(messageId, item, text) => void askChatHomeWorkItem(messageId, item, text)}
                  onWorkItemAction={(item, action) => executeConfirmedItemAction(item, action)}
                  onOpenWorkDetail={(item) => openChatHomeWorkDetail(item)}
                  onNoticeAction={(action) => executeChatNoticeAction(action)}
                  onAttachContext={setChatHomeAttachedContext}
                  onClearAttachedContext={() => setChatHomeAttachedContext(null)}
                  pendingAttachment={chatHomePendingAttachment}
                  attachmentUploading={chatHomeAttachmentUploading}
                  attachmentError={chatHomeAttachmentError}
                  onAttachFile={(file) => void handleChatHomeAttachFile(file)}
                  onClearAttachment={clearChatHomePendingAttachment}
                  installOffered={installStatus !== 'installed'}
                  onInstallOwnerApp={() => void installOwnerApp()}
                  logoutBusy={logoutBusy}
                  onLogout={() => void logoutNow()}
                />
              </div>
            );
            return sheetMode ? createPortal(host, document.body) : host;
          })()}
          {session?.authenticated && voiceRealtimeAvailable && (
            <CeoVoiceAgent
              ref={ceoVoiceAgentRef}
              locale={errorLocale}
              busy={busy}
              enabled
              chromeMode
              realtimeAvailable={voiceRealtimeAvailable}
              mintSession={() => mintVNextVoiceSession(initData, session.csrf_token)}
              buildScreenContext={buildCeoScreenContext}
              handlersRef={secretaryHandlersRef}
              onApprovalAction={(approvalId, action) => void handleChatHomeApprovalAction(approvalId, action)}
              aegisAvatarUrl={avatarProfile.aegisAvatarUrl}
              onChromeState={setVoiceChrome}
              onLiveLine={setVoiceLiveLine}
              onUtteranceSettled={settleVoiceUtterance}
              onVoiceDecision={(action, approvalId) => {
                void settleVoiceApproval(approvalId, action);
              }}
              onNotice={(notice) => {
                setChatHomeNotice(notice);
                setSnapshotMessage('');
              }}
            />
          )}
          {view === 'work' && queue && (
            <WorkView
              lanes={lanes}
              expandedLaneIds={expandedLaneIds}
              selected={selected}
              requestedItemId={requestedItemId}
              context={context}
              itemProjection={itemProjection}
              sourceDetail={sourceDetailView}
              sourceDetailBusy={sourceDetailBusy}
              assistantResult={assistantResult}
              assistantResultItemId={assistantResultItemId}
              actionMessage={selectedActionMessage}
              hostStepUpBusy={hostStepUp?.itemId === selected?.id && hostStepUpBusy}
              onHostStepUp={hostStepUp?.itemId === selected?.id ? () => void confirmHostStepUp() : undefined}
              workNotice={workNotice}
              actionsRetired={selectedActionsRetired}
              armedAction={armedAction}
              armedActionItemId={armedActionItemId}
              executingAction={executingAction}
              busy={busy}
              onSelect={(id) => {
                setSelectedId(id);
                writeVNextLocationState(window, { view: 'work', itemId: id });
              }}
              onToggleLane={toggleLane}
              onOpenSource={() => void openSourceDetail()}
              onAction={(item, action) => void executeItemAction(item, action)}
              onExecuteAction={(item, action) => void executeConfirmedItemAction(item, action)}
              onBrief={() => void runAssistant('explain')}
              onDraft={() => void runAssistant('draft')}
              onAskText={(item, text, conversationContext) => void runAssistant('chat', text, item.id, undefined, conversationContext)}
              onOpenLineageMessage={openLineageChatMessage}
              onRetryAssistant={lastItemCommand && selected?.id === lastItemCommand.itemId
                ? () => void runAssistant(
                  lastItemCommand.intent,
                  lastItemCommand.text,
                  lastItemCommand.itemId,
                  lastItemCommand.idempotencyKey,
                  lastItemCommand.conversationContext ?? [],
                )
                : undefined}
              onRetryAssistantLabel={lastItemCommand && selected?.id === lastItemCommand.itemId
                ? lastItemCommand.retryLabel
                : undefined}
            />
          )}
          {view === 'status' && queue && (
            <StatusView
              queue={queue}
              projection={statusProjection}
              onOpenWork={() => navigate('work')}
              onOpenSettings={() => navigate('settings', null)}
            />
          )}
          {view === 'settings' && queue && (
            <SettingsView
              jevAccess={{ initData, csrfToken: session.csrf_token }}
              email={session.email}
              telegramRail={telegramRail}
              displayName={
                (typeof window !== 'undefined'
                  ? window.Telegram?.WebApp?.initDataUnsafe?.user?.first_name
                  : undefined)
                || undefined
              }
              sources={queue.sources}
              projection={settingsProjection}
              costSavingBusy={costSavingBusy}
              snapshotMessage={snapshotMessage}
              snapshotAvailable={snapshotAvailable}
              snapshotBusy={snapshotBusy}
              logoutBusy={logoutBusy}
              webPushStatus={webPushStatus}
              webPushDeviceSubscribed={webPushDeviceSubscribed}
              webPushBusy={webPushBusy}
              webPushMessage={webPushMessage}
              codexStatus={codexStatus}
              codexDeviceCode={codexDeviceCode}
              codexBusy={codexBusy}
              codexMessage={codexMessage}
              grokStatus={grokStatus}
              grokDeviceCode={grokDeviceCode}
              grokBusy={grokBusy}
              grokMessage={grokMessage}
              avatarProfile={avatarProfile}
              avatarAutoRefresh={avatarAutoRefresh}
              avatarGenerationBusy={avatarGenerationBusy}
              debugLeaseBusy={debugLeaseBusy}
              debugLeases={debugLeases}
              debugGrantToken={debugGrantToken}
              debugJobBusy={debugJobBusy}
              debugJobs={debugJobs}
              onPauseCostSaving={() => void pauseCostSavingControls()}
              onResumeCostSaving={() => void resumeCostSavingControls()}
              onEnableWebPush={() => void enableWebPushNotifications()}
              onDisableWebPush={() => void disableWebPushNotifications()}
              onTestWebPush={() => void sendWebPushTestNotification()}
              onCreateSnapshot={() => void createDebugSnapshot()}
              onCreateDebugLease={(scope, reason, durationMinutes, itemId) => void createOwnerDebugLease(scope, reason, durationMinutes, itemId)}
              onRevokeDebugLease={(leaseId) => void revokeOwnerDebugLease(leaseId)}
              onRunDebugJob={() => void runOwnerDebugJob()}
              onLogout={() => void logoutNow()}
              onStartCodexDeviceCode={() => void startCodexDeviceCode()}
              onPollCodexDeviceCode={() => void pollCodexDeviceCodeNow()}
              onUseCodexForChat={() => void useCodexForChat()}
              onSetCodexModel={(model, reasoningEffort) => void setCodexModel(model, reasoningEffort)}
              onStopUsingCodexForChat={() => void stopUsingCodexForChat()}
              onStartGrokDeviceCode={() => void startGrokDeviceCode()}
              onPollGrokDeviceCode={() => void pollGrokDeviceCodeNow()}
              onRevokeGrok={() => void revokeGrokConnection()}
              onUseGrokForChat={() => void useGrokForChat()}
              onStopUsingGrokForChat={() => void stopUsingGrokForChat()}
              onSetAvatar={updateAvatarProfile}
              onResetAvatar={resetAvatarProfile}
              onToggleAegisAvatarAutoRefresh={(enabled, promptHint) => void toggleAegisAvatarAutoRefresh(enabled, promptHint)}
              onGenerateAegisAvatar={(promptHint) => void generateAegisAvatarNow(promptHint)}
              onSetDisplayLanguage={(value) => persistDisplayLanguage(value)}
              onSetLlmProvider={persistLlmProvider}
              onSetLlmModel={(value) => void persistLlmModel(value)}
              onSetFallbackModel={persistFallbackModel}
              onSetWebSearchProvider={persistWebSearchProvider}
              onSetWebSearchModel={persistWebSearchModel}
              onLoadModelCatalog={() => loadModelCatalog()}
              onSetPaidApiFallback={persistPaidApiFallback}
            />
          )}
        </main>

      </div>

      <nav className="vnext-nav" aria-label={t('app.shell.nav.primary', errorLocale)}>
        <NavButton
          view="home"
          activeView={view}
          label={t('app.shell.nav.chat', errorLocale)}
          icon="home"
          onClick={() => navigate('home', null)}
          dot={unseenChatActivity}
          badgeAria={unseenChatActivity ? t('app.shell.nav.chatUnseenAria', errorLocale) : undefined}
        />
        {/* Two stable destinations; contextual questions stay with their work item. */}
        <NavButton
          view="work"
          activeView={view}
          label={t('app.shell.nav.work', errorLocale)}
          icon="work"
          onClick={() => navigate('work')}
          badgeCount={workBadgeCount}
          badgeTone={decisionCount > 0 ? 'warn' : 'default'}
          badgeAria={workBadgeCount > 0 ? workBadgeLabel : undefined}
        />
      </nav>

      {view !== 'home' && chatSheetOpen && (
        <div
          ref={chatSheetBackdropRef}
          className="vnext-chat-sheet-backdrop"
          data-testid="aegis-chat-sheet-backdrop"
          aria-hidden="true"
          onClick={() => setChatSheetOpen(false)}
        />
      )}
    </div>
    </LocaleProvider>
    </UserTimezoneProvider>
  );
}
