import type {
  ContextualAssistantEvidenceItem,
  ContextualAssistantResponse,
  ContextualAssistantSourceEvidence,
} from '../api';
import { t, tf, preferredLocale, type Locale } from '../i18n/catalog';
import { normalizeMessageTimestamp } from './time';

type ChatHomeAssistantResponse = {
  schema_version: number;
  response_kind: string;
  visible_answer: string;
  turn_id?: string | null;
  severity?: 'info' | 'notice' | 'important' | 'warning' | 'critical' | null;
  draft?: string | null;
  action_draft?: unknown;
  suggested_intent?: unknown;
  authority?: unknown;
  required_gate?: unknown;
  receipt_expectation: string;
  citations: string[];
  source_evidence?: ChatHomePersistedSourceEvidence[];
  evidence?: ChatHomePersistedClaimEvidence[];
  unsupported_numbers?: string[];
  warnings: string[];
  trace_id?: string | null;
  run_id?: string | null;
  provider_request_id?: string | null;
};

type ChatHomePersistedJevProvenance = {
  verdict: string;
  owner_decision?: string;
  approval_id?: string;
  receipt?: string;
  audit_tip?: string;
  risk?: string;
  model?: string;
  reason?: string;
};

type ChatHomePersistedSourceEvidence = {
  kind: string;
  label: string;
  summary: string;
  detail: string;
  jev?: ChatHomePersistedJevProvenance;
};

type ChatHomePersistedClaimEvidence = {
  id: string;
  tool: string;
  label: string;
  excerpt: string;
  full_len: number;
};

export type ChatHomePersistedMessage = {
  id: string;
  turnId?: string;
  replyOrigin?: ReplyOrigin;
  role: 'owner' | 'aegis';
  text: string;
  timestamp?: number;
  latencyMs?: number;
  severity?: 'info' | 'notice' | 'important' | 'warning' | 'critical' | null;
  quote?: ChatHomePersistedQuote;
  contextLabel?: string;
  result?: ChatHomeAssistantResponse;
  retryText?: string;
  retryIdempotencyKey?: string;
  retryLabel?: string;
  retryContext?: ChatHomePersistedContext;
  retryQuote?: ChatHomePersistedQuote;
};

export type ChatHomePersistedQuote = {
  sourceId?: string;
  role: 'owner' | 'aegis';
  text: string;
  timestamp?: number;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
type ReplyOrigin = 'command' | 'history' | 'transport-error';

export type ChatHomePersistedContext = {
  itemId: string;
  label: string;
};

export type ChatHomeMergeableMessage = {
  id: string;
  turnId?: string;
  replyOrigin?: ReplyOrigin;
  retryText?: string;
  retryIdempotencyKey?: string;
  retryLabel?: string;
  retryContext?: ChatHomePersistedContext;
  retryQuote?: ChatHomePersistedQuote;
  result?: ContextualAssistantResponse;
  // 'approval' = the in-chat HITL approval card (Slice 4). It is a live-turn
  // artifact: never persisted (compactChatHomeHistory keeps only owner/aegis)
  // and not preserved across merges (only working/queued are). Included in the
  // role union so the shared persistence/merge helpers accept the full
  // ChatHomeMessage[] without a cast; the runtime filters drop it.
  role: 'owner' | 'aegis' | 'working' | 'queued' | 'approval';
  text: string;
  timestamp?: number;
  severity?: 'info' | 'notice' | 'important' | 'warning' | 'critical' | null;
};

const HISTORY_SCHEMA_VERSION = 1;
const MAX_CHAT_HOME_MESSAGES = 24;
const MAX_TEXT_CHARS = 4000;
const MAX_OWNER_KEY_CHARS = 128;
// Local owner/aegis turns the server has not returned yet. Four was too few
// for a quick burst (three asks answered before the history poll caught up
// lost the oldest pair on merge).
const MAX_LOCAL_CHAT_HISTORY_MERGE_TURNS = 12;
const CHAT_HISTORY_DUPLICATE_WINDOW_MS = 2 * 60 * 1000;
// An identical Aegis reply text is the same turn even when the local bubble was
// appended long after the server stored it (a 504 → 查看结果 tap minutes later):
// the owner saw such replies twice after reload (2026-09-12).
const CHAT_HISTORY_SAME_REPLY_WINDOW_MS = 12 * 60 * 60 * 1000;

type PersistedPayload = {
  schema_version: number;
  messages: ChatHomePersistedMessage[];
};

export type ServerChatHistoryMessage = {
  turn_id?: string | null;
  role: 'owner' | 'aegis';
  text: string;
  timestamp: number;
  severity?: ChatHomePersistedMessage['severity'];
  latency_ms?: number;
  repeat_count?: number;
  first_timestamp?: number;
  work_item_id?: string | null;
  evidence?: ContextualAssistantEvidenceItem[];
  source_evidence?: ContextualAssistantSourceEvidence[];
  unsupported_numbers?: string[];
  retry_text?: string | null;
};

export type ServerMappedChatHomeMessage = {
  id: string;
  turnId?: string;
  replyOrigin?: ReplyOrigin;
  role: 'owner' | 'aegis';
  text: string;
  severity: ChatHomePersistedMessage['severity'];
  timestamp: number | undefined;
  latencyMs?: number;
  repeatCount?: number;
  firstTimestamp?: number;
  workItemAnchorId: string | null;
  retryText?: string;
  retryLabel?: string;
  result?: {
    schema_version: number;
    response_kind: string;
    visible_answer: string;
    turn_id?: string;
    receipt_expectation: string;
    citations: string[];
    warnings: string[];
    evidence?: ChatHomePersistedClaimEvidence[];
    source_evidence?: ChatHomePersistedSourceEvidence[];
    unsupported_numbers?: string[];
  };
};

export function chatHomeMessagesFromServerHistory(
  history: { messages: ServerChatHistoryMessage[] },
): ServerMappedChatHomeMessage[] {
  return history.messages.map((message, index) => {
    const turnId = safeTurnId(message.turn_id);
    const evidence = Array.isArray(message.evidence) && message.evidence.length > 0
      ? message.evidence
      : undefined;
    const sourceEvidence = safePersistedSourceEvidence(message.source_evidence);
    const unsupportedNumbers = Array.isArray(message.unsupported_numbers) && message.unsupported_numbers.length > 0
      ? message.unsupported_numbers
      : undefined;
    const retryText = trimText(message.retry_text).trim();
    const result = message.role === 'aegis' && (evidence || sourceEvidence.length > 0 || unsupportedNumbers)
      ? {
          schema_version: 1,
          response_kind: 'answer',
          visible_answer: message.text,
          ...(turnId ? { turn_id: turnId } : {}),
          receipt_expectation: '',
          citations: [] as string[],
          warnings: [] as string[],
          ...(evidence ? { evidence } : {}),
          ...(sourceEvidence.length > 0 ? { source_evidence: sourceEvidence } : {}),
          ...(unsupportedNumbers ? { unsupported_numbers: unsupportedNumbers } : {}),
        }
      : undefined;
    return {
      id: turnId ? `server:${turnId}:${message.role}` : `server:${message.timestamp}:${index}:${message.role}`,
      ...(turnId ? { turnId } : {}),
      role: message.role,
      ...(message.role === 'aegis' ? { replyOrigin: 'history' as const } : {}),
      text: message.text,
      severity: message.severity ?? null,
      timestamp: normalizeMessageTimestamp(message.timestamp),
      ...(typeof message.latency_ms === 'number' && Number.isFinite(message.latency_ms) && message.latency_ms >= 0
        ? { latencyMs: message.latency_ms }
        : {}),
      ...(typeof message.repeat_count === 'number' && message.repeat_count > 1
        ? {
            repeatCount: message.repeat_count,
            ...(typeof message.first_timestamp === 'number'
              ? { firstTimestamp: normalizeMessageTimestamp(message.first_timestamp) }
              : {}),
          }
        : {}),
      workItemAnchorId: message.work_item_id ?? null,
      ...(result ? { result } : {}),
      ...(retryText ? { retryText, retryLabel: 'Try again' } : {}),
    };
  });
}

export function chatHomeHistoryOwnerKey(baseKey: string, email?: string | null): string {
  const normalized = typeof email === 'string' ? email.trim().toLowerCase() : '';
  if (!normalized) return `${baseKey}:owner`;
  const safeEmail = normalized
    .replace(/[^a-z0-9@._+-]/g, '_')
    .slice(0, MAX_OWNER_KEY_CHARS);
  return `${baseKey}:owner:${safeEmail}`;
}

function trimText(value: unknown): string {
  if (typeof value !== 'string') return '';
  return value.slice(0, MAX_TEXT_CHARS);
}

function safeTurnId(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim().length > 0 && value.length <= 256
    ? value.trim()
    : undefined;
}

function messageTurnId(message: ChatHomeMergeableMessage): string | undefined {
  return safeTurnId(message.turnId) ?? safeTurnId(message.result?.turn_id);
}

function trimStringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map(trimText)
    .filter(Boolean)
    .slice(0, 12);
}

function safePersistedStringList(value: unknown, locale: Locale): string[] {
  const normalized = trimStringList(value)
    .map((item) => ownerSafeRuntimeText(item, locale))
    .filter((item): item is string => Boolean(item));
  return Array.from(new Set(normalized)).slice(0, 12);
}

function isPersistedNoticeSeverity(value: unknown): value is 'info' | 'notice' | 'important' | 'warning' | 'critical' {
  return value === 'info'
    || value === 'notice'
    || value === 'important'
    || value === 'warning'
    || value === 'critical';
}

const SOURCE_SECRET = /provider_request(?:_id)?|trace_id|run_id|turn_id|action_id|tool_call_id|idempotency_key|request_id|api[_ -]?key|access[_ -]?token|refresh[_ -]?token/gi;

function redactSourceSecrets(value: string): string {
  const assignment = new RegExp(`(?:${SOURCE_SECRET.source})\\s*[=:]\\s*\\S+`, 'gi');
  const withoutAssignments = value.replace(assignment, '');
  const withoutWords = withoutAssignments.replace(new RegExp(SOURCE_SECRET.source, 'gi'), '');
  return withoutWords.replace(/[ \t]{2,}/g, ' ').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
}

function safeSourceEvidenceText(value: unknown, limit: number): string {
  if (typeof value !== 'string') return '';
  const redacted = redactSourceSecrets(value);
  if (!redacted || !/[\p{L}\p{N}]/u.test(redacted)) return '';
  return redacted.slice(0, limit);
}

function safePersistedSourceEvidence(value: unknown): ChatHomePersistedSourceEvidence[] {
  if (!Array.isArray(value)) return [];
  const evidence: ChatHomePersistedSourceEvidence[] = [];
  const seen = new Set<string>();
  for (const item of value) {
    if (!item || typeof item !== 'object') continue;
    const raw = item as Record<string, unknown>;
    const kind = safeSourceEvidenceText(raw.kind, 64);
    const label = safeSourceEvidenceText(raw.label, 96);
    const summary = safeSourceEvidenceText(raw.summary, 240);
    const detail = safeSourceEvidenceText(raw.detail, 1200);
    if (!kind || !label || !summary || !detail) continue;
    const jev = safePersistedJevProvenance(raw.jev);
    const key = `${kind}:${label}:${summary}:${detail}:${jev?.verdict ?? ''}:${jev?.approval_id ?? ''}`;
    if (seen.has(key)) continue;
    seen.add(key);
    evidence.push({ kind, label, summary, detail, ...(jev ? { jev } : {}) });
    if (evidence.length >= 12) break;
  }
  return evidence;
}

function safePersistedJevProvenance(value: unknown): ChatHomePersistedJevProvenance | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const verdict = safeSourceEvidenceText(raw.verdict, 16)?.toLowerCase();
  if (verdict !== 'allow' && verdict !== 'ask' && verdict !== 'deny') return undefined;
  const owner = safeSourceEvidenceText(raw.owner_decision, 16)?.toLowerCase();
  const ownerDecision = owner === 'approved' || owner === 'rejected' ? owner : undefined;
  const approvalId = safeSourceEvidenceText(raw.approval_id, 160);
  const receipt = safeSourceEvidenceText(raw.receipt, 160);
  const auditTip = safeSourceEvidenceText(raw.audit_tip, 160);
  const risk = safeSourceEvidenceText(raw.risk, 80);
  const model = safeSourceEvidenceText(raw.model, 80);
  const reason = safeSourceEvidenceText(raw.reason, 240);
  return {
    verdict,
    ...(ownerDecision ? { owner_decision: ownerDecision } : {}),
    ...(approvalId ? { approval_id: approvalId } : {}),
    ...(receipt ? { receipt } : {}),
    ...(auditTip ? { audit_tip: auditTip } : {}),
    ...(risk ? { risk } : {}),
    ...(model ? { model } : {}),
    ...(reason ? { reason } : {}),
  };
}

function safePersistedClaimEvidence(value: unknown): ChatHomePersistedClaimEvidence[] {
  if (!Array.isArray(value)) return [];
  const evidence: ChatHomePersistedClaimEvidence[] = [];
  const seen = new Set<string>();
  for (const item of value) {
    if (!item || typeof item !== 'object') continue;
    const raw = item as Record<string, unknown>;
    const id = safeSourceEvidenceText(raw.id, 8);
    const tool = safeSourceEvidenceText(raw.tool, 64);
    const label = safeSourceEvidenceText(raw.label, 96);
    const excerpt = safeSourceEvidenceText(raw.excerpt, 1200);
    const fullLen = typeof raw.full_len === 'number' && Number.isFinite(raw.full_len) && raw.full_len >= 0
      ? raw.full_len
      : null;
    if (!id || !/^E\d+$/.test(id) || !tool || !label || !excerpt || fullLen === null) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    evidence.push({ id, tool, label, excerpt, full_len: fullLen });
    if (evidence.length >= 16) break;
  }
  return evidence;
}

function safePersistedUnsupportedNumbers(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const numbers: string[] = [];
  const seen = new Set<string>();
  for (const item of value) {
    if (typeof item !== 'string') continue;
    const text = item.trim().slice(0, 64);
    if (!text || seen.has(text)) continue;
    seen.add(text);
    numbers.push(text);
    if (numbers.length >= 32) break;
  }
  return numbers;
}

function ownerSafeRuntimeText(value: string, locale: Locale): string | null {
  const trimmed = value.trim();
  if (!trimmed || /^visible_fields\./i.test(trimmed)) return null;
  const blockedTool = trimmed.match(/^tool_blocked:(.+)$/i);
  if (blockedTool) return tf('runtime.notice.toolUnavailable', locale, { tool: runtimeSourceLabel(blockedTool[1], locale) });
  const tool = trimmed.match(/^tool:(.+)$/i);
  if (tool) return runtimeSourceLabel(tool[1], locale);
  const normalized = trimmed.toLowerCase();
  // Persisted notices used to carry hardcoded English ("Tasks and reminders",
  // "More detail needs your approval.") into a Chinese console.
  const labelKeys: Record<string, string | null> = {
    source_content_untrusted: null,
    redacted_fields_excluded: null,
    more_detail_requires_gate: 'runtime.notice.moreDetailGate',
    data_access_gate: 'runtime.notice.dataAccessGate',
    tool_gate: 'runtime.notice.toolGate',
    mfa_required: 'runtime.notice.mfaRequired',
    consent_required: 'runtime.notice.consentRequired',
    llm_provider: 'runtime.notice.llmProvider',
  };
  if (Object.prototype.hasOwnProperty.call(labelKeys, normalized)) {
    const key = labelKeys[normalized];
    return key ? t(key, locale) : null;
  }
  if (
    normalized.includes('provider_request')
    || normalized.includes('trace_id')
    || normalized.includes('run_id')
    || normalized.includes('turn_id')
    || normalized.includes('action_id')
    || normalized.includes('tool_call_id')
    || normalized.includes('idempotency_key')
    || normalized.includes('request_id')
    || /\b[a-z]+_[a-z0-9_]+\b/.test(normalized)
  ) {
    return null;
  }
  return trimmed;
}

function runtimeSourceLabel(source: string, locale: Locale): string {
  const normalized = source.trim().replace(/^skill_/, '').toLowerCase();
  const labelKeys: Record<string, string> = {
    manage_tasks: 'assistantResponse.runtimeSource.tasks',
    schedule_intent: 'assistantResponse.runtimeSource.reminderScheduler',
    commute_traffic_monitor: 'assistantResponse.runtimeSource.commute',
    recall_memory: 'assistantResponse.runtimeSource.memory',
    knowledge_base: 'assistantResponse.runtimeSource.knowledgeBase',
    calendar_reader: 'assistantResponse.runtimeSource.calendar',
    web_search: 'assistantResponse.runtimeSource.webSearch',
    check_api_usage: 'assistantResponse.runtimeSource.apiCost',
    server_monitor: 'assistantResponse.runtimeSource.serverMonitor',
  };
  const key = labelKeys[normalized];
  return key ? t(key, locale) : t('runtime.source.generic', locale);
}

function persistedContext(value: unknown): ChatHomePersistedContext | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const itemId = trimText(raw.itemId);
  const label = trimText(raw.label);
  if (!itemId || !label) return undefined;
  return { itemId, label };
}

function persistedQuote(value: unknown): ChatHomePersistedQuote | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const role = raw.role === 'owner' || raw.role === 'aegis' ? raw.role : null;
  if (!role) return undefined;
  const text = trimText(raw.text);
  if (!text) return undefined;
  const sourceId = trimText(raw.sourceId);
  const timestamp = typeof raw.timestamp === 'number' && Number.isFinite(raw.timestamp) && raw.timestamp > 0
    ? raw.timestamp
    : undefined;
  return {
    role,
    text,
    ...(sourceId ? { sourceId } : {}),
    ...(timestamp ? { timestamp } : {}),
  };
}

function persistedAssistantResponse(value: unknown, locale: Locale): ChatHomeAssistantResponse | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const responseKind = trimText(raw.response_kind);
  const visibleAnswer = trimText(raw.visible_answer);
  if (!responseKind || !visibleAnswer) return undefined;
  const sourceEvidence = safePersistedSourceEvidence(raw.source_evidence);
  const claimEvidence = safePersistedClaimEvidence(raw.evidence);
  const unsupportedNumbers = safePersistedUnsupportedNumbers(raw.unsupported_numbers);
  const turnId = safeTurnId(raw.turn_id);
  return {
    schema_version: typeof raw.schema_version === 'number' ? raw.schema_version : HISTORY_SCHEMA_VERSION,
    response_kind: responseKind,
    visible_answer: visibleAnswer,
    ...(turnId ? { turn_id: turnId } : {}),
    severity: isPersistedNoticeSeverity(raw.severity) ? raw.severity : null,
    receipt_expectation: trimText(raw.receipt_expectation),
    citations: safePersistedStringList(raw.citations, locale),
    ...(sourceEvidence.length > 0 ? { source_evidence: sourceEvidence } : {}),
    ...(claimEvidence.length > 0 ? { evidence: claimEvidence } : {}),
    ...(unsupportedNumbers.length > 0 ? { unsupported_numbers: unsupportedNumbers } : {}),
    warnings: safePersistedStringList(raw.warnings, locale),
  };
}

function persistedMessage(value: unknown, locale: Locale): ChatHomePersistedMessage | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const role = raw.role === 'owner' || raw.role === 'aegis' ? raw.role : null;
  if (!role) return null;
  const id = trimText(raw.id);
  const text = trimText(raw.text);
  if (!id || !text) return null;
  const result = persistedAssistantResponse(raw.result, locale);
  const turnId = safeTurnId(raw.turnId) ?? safeTurnId(result?.turn_id);
  const replyOrigin = raw.replyOrigin === 'command' || raw.replyOrigin === 'history' || raw.replyOrigin === 'transport-error'
    ? raw.replyOrigin : undefined;
  const timestamp = typeof raw.timestamp === 'number' && Number.isFinite(raw.timestamp) && raw.timestamp > 0
    ? raw.timestamp
    : undefined;
  const latencyMs = role === 'aegis' && typeof raw.latencyMs === 'number' && Number.isFinite(raw.latencyMs) && raw.latencyMs >= 0
    ? Math.round(raw.latencyMs)
    : undefined;
  const contextLabel = trimText(raw.contextLabel);
  const retryText = trimText(raw.retryText);
  const retryIdempotencyKey = trimText(raw.retryIdempotencyKey);
  const retryLabel = trimText(raw.retryLabel);
  const retryContext = persistedContext(raw.retryContext);
  const quote = persistedQuote(raw.quote);
  const retryQuote = persistedQuote(raw.retryQuote);
  return {
    id,
    ...(turnId ? { turnId } : {}),
    ...(replyOrigin ? { replyOrigin } : {}),
    role,
    text,
    severity: isPersistedNoticeSeverity(raw.severity) ? raw.severity : null,
    ...(timestamp ? { timestamp } : {}),
    ...(latencyMs !== undefined ? { latencyMs } : {}),
    ...(quote ? { quote } : {}),
    ...(contextLabel ? { contextLabel } : {}),
    ...(result ? { result } : {}),
    ...(retryText ? { retryText } : {}),
    ...(retryIdempotencyKey ? { retryIdempotencyKey } : {}),
    ...(retryLabel ? { retryLabel } : {}),
    ...(retryText && retryContext ? { retryContext } : {}),
    ...(retryText && retryQuote ? { retryQuote } : {}),
  };
}

/**
 * `locale` drives the owner copy of restored runtime notices (gate / tool
 * warnings). Callers inside the console pass the active display locale; the
 * default follows the `<html lang>` the LocaleProvider stamps.
 */
type ChatHomeHistoryInput = {
  role: 'owner' | 'aegis' | 'working' | 'queued' | 'approval';
};

export function compactChatHomeHistory(
  messages: readonly ChatHomeHistoryInput[],
  locale: Locale = preferredLocale(),
): ChatHomePersistedMessage[] {
  return messages
    .filter((message): message is ChatHomeHistoryInput & { role: 'owner' | 'aegis' } => message.role === 'owner' || message.role === 'aegis')
    .map((message) => persistedMessage(message, locale))
    .filter((message): message is ChatHomePersistedMessage => Boolean(message))
    .filter((message) => !isLowValueSystemChatNotice(message))
    .slice(-MAX_CHAT_HOME_MESSAGES);
}

export function readChatHomeHistory(storage: StorageLike, key: string, locale: Locale = preferredLocale()): ChatHomePersistedMessage[] {
  try {
    const raw = storage.getItem(key);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as PersistedPayload;
    if (!parsed || parsed.schema_version !== HISTORY_SCHEMA_VERSION || !Array.isArray(parsed.messages)) {
      return [];
    }
    return compactChatHomeHistory(parsed.messages, locale);
  } catch {
    return [];
  }
}

export function writeChatHomeHistory(
  storage: StorageLike,
  key: string,
  messages: readonly ChatHomeHistoryInput[],
  locale: Locale = preferredLocale(),
) {
  try {
    const persisted = compactChatHomeHistory(messages, locale);
    if (persisted.length === 0) {
      storage.removeItem(key);
      return;
    }
    storage.setItem(key, JSON.stringify({
      schema_version: HISTORY_SCHEMA_VERSION,
      messages: persisted,
    }));
  } catch {
    // In-session history is convenience only. Never block the assistant.
  }
}

export function mergeChatHomeServerAndLocalHistory<T extends ChatHomeMergeableMessage>(
  serverMessages: T[],
  current: T[],
): T[] {
  const filteredServerMessages = serverMessages.filter((message) => !isLowValueSystemChatNotice(message));
  const filteredCurrent = current.filter((message) => !isLowValueSystemChatNotice(message));
  if (filteredServerMessages.length === 0) return filteredCurrent;
  const sameTurn = (server: T, local: T) => {
    if (!isSameRestoredTurn(server, local)) return false;
    if (messageTurnId(server) && messageTurnId(local)) return true;
    if (server.role !== 'aegis') return true;
    const ownerBefore = (message: T, messages: T[]) => {
      for (let index = messages.indexOf(message) - 1; index >= 0; index -= 1) {
        if (messages[index].role === 'owner') return messages[index];
      }
      return undefined;
    };
    const serverOwner = ownerBefore(server, filteredServerMessages);
    const localOwner = ownerBefore(local, filteredCurrent);
    return !serverOwner || !localOwner || isSameRestoredTurn(serverOwner, localOwner);
  };
  const enrichedServerMessages = filteredServerMessages.map((serverMessage) => {
    const localDuplicates = filteredCurrent.filter((localMessage) => sameTurn(serverMessage, localMessage));
    const localDuplicate = localDuplicates.find((message) => message.replyOrigin === 'command' && message.result)
      ?? localDuplicates.find((message) => finalLocalResult(message))
      ?? localDuplicates[0];
    return localDuplicate ? mergeRestoredServerTurn(serverMessage, localDuplicate) : serverMessage;
  });
  const localTail = filteredCurrent
    .filter((message) => message.role === 'owner' || message.role === 'aegis')
    .filter((message) => !filteredServerMessages.some((serverMessage) => sameTurn(serverMessage, message)))
    .slice(-MAX_LOCAL_CHAT_HISTORY_MERGE_TURNS);
  const stableMessages = [...enrichedServerMessages, ...localTail];
  const transientMessages = filteredCurrent
    .filter((message) => message.role === 'working' || message.role === 'queued')
    .filter((message) => shouldPreserveTransientMessage(message, enrichedServerMessages, filteredCurrent));
  return [...stableMessages, ...transientMessages]
    .map((message, index) => ({ message, index }))
    .sort((left, right) => {
      const leftTime = mergeSortTimestamp(left.message, stableMessages, filteredCurrent);
      const rightTime = mergeSortTimestamp(right.message, stableMessages, filteredCurrent);
      if (leftTime !== rightTime) return leftTime - rightTime;
      return left.index - right.index;
    })
    .map(({ message }) => message)
    .slice(-MAX_CHAT_HOME_MESSAGES);
}

function mergeSortTimestamp<T extends ChatHomeMergeableMessage>(
  message: T,
  stableMessages: T[],
  currentMessages: T[],
): number {
  const ownTime = sortableChatTimestamp(message.timestamp);
  if (message.role !== 'working' && message.role !== 'queued') return ownTime;
  const ownerMessage = previousOwnerMessage(message, currentMessages);
  if (!ownerMessage) return ownTime;
  const restoredOwner = stableMessages.find((stable) => (
    stable.role === 'owner' && isSameRestoredTurn(stable, ownerMessage)
  ));
  if (!restoredOwner) return ownTime;
  const ownerTime = sortableChatTimestamp(restoredOwner.timestamp);
  if (ownerTime === Number.MAX_SAFE_INTEGER) return ownTime;
  if (ownTime === Number.MAX_SAFE_INTEGER) return ownerTime + 1;
  return Math.max(ownTime, ownerTime + 1);
}

function shouldPreserveTransientMessage<T extends ChatHomeMergeableMessage>(
  message: T,
  serverMessages: T[],
  currentMessages: T[],
): boolean {
  if (message.role === 'queued') return true;
  if (hasLocalAnswerAfterTransient(message, currentMessages)) return false;
  const messageTime = sortableChatTimestamp(message.timestamp);
  if (messageTime === Number.MAX_SAFE_INTEGER) return true;
  const ownerMessage = previousOwnerMessage(message, currentMessages);
  if (!ownerMessage) return true;
  const orderedServerMessages = serverMessages
    .map((stable, index) => ({ stable, index }))
    .sort((left, right) => {
      const leftTime = sortableChatTimestamp(left.stable.timestamp);
      const rightTime = sortableChatTimestamp(right.stable.timestamp);
      if (leftTime !== rightTime) return leftTime - rightTime;
      return left.index - right.index;
    })
    .map(({ stable }) => stable);
  const ownerIndex = orderedServerMessages.findIndex((stable) => (
    stable.role === 'owner' && isSameRestoredTurn(stable, ownerMessage)
  ));
  if (ownerIndex === -1) return true;
  const nextOwnerIndex = orderedServerMessages.findIndex((stable, index) => (
    index > ownerIndex && stable.role === 'owner'
  ));
  const hasRecoveredAnswer = orderedServerMessages.some((stable, index) => (
    stable.role === 'aegis'
    && index > ownerIndex
    && (nextOwnerIndex === -1 || index < nextOwnerIndex)
    && sortableChatTimestamp(stable.timestamp) >= messageTime
  ));
  return !hasRecoveredAnswer;
}

function hasLocalAnswerAfterTransient<T extends ChatHomeMergeableMessage>(message: T, currentMessages: T[]): boolean {
  const messageIndex = currentMessages.indexOf(message);
  if (messageIndex < 0) return false;
  const nextOwnerIndex = currentMessages.findIndex((candidate, index) => (
    index > messageIndex && candidate.role === 'owner'
  ));
  return currentMessages.some((candidate, index) => (
    candidate.role === 'aegis'
    && index > messageIndex
    && (nextOwnerIndex === -1 || index < nextOwnerIndex)
  ));
}

function previousOwnerMessage<T extends ChatHomeMergeableMessage>(message: T, currentMessages: T[]): T | null {
  const messageIndex = currentMessages.indexOf(message);
  if (messageIndex <= 0) return null;
  for (let index = messageIndex - 1; index >= 0; index -= 1) {
    const candidate = currentMessages[index];
    if (candidate.role === 'owner') return candidate;
    if (candidate.role === 'aegis') return null;
  }
  return null;
}

// Complete the recovered bubble in place, retaining its stable history position.
// Only match within this owner's turn; equal answers to later questions are real turns.
export function completeChatHomeTurn<T extends ChatHomeMergeableMessage>(
  messages: T[], ownerText: string, ownerTimestamp: number, reply: T,
): T[] {
  const turnId = messageTurnId(reply);
  if (turnId && messages.some((message) => message.role === 'aegis' && messageTurnId(message) === turnId)) {
    let replaced = false;
    return messages.flatMap((message) => {
      if (message.role !== 'aegis' || messageTurnId(message) !== turnId) return [message];
      if (replaced) return [];
      replaced = true;
      return [completedReply(message, reply)];
    });
  }
  let ownerIndex = -1;
  let closestOwnerTime = Number.POSITIVE_INFINITY;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === 'owner' && turnId && messageTurnId(message) === turnId) {
      ownerIndex = index;
      break;
    }
    if (turnId && messageTurnId(message) && messageTurnId(message) !== turnId) continue;
    const distance = Math.abs((message.timestamp ?? ownerTimestamp) - ownerTimestamp);
    if (message.role === 'owner'
      && message.text.trim().replace(/\s+/g, ' ') === ownerText.trim().replace(/\s+/g, ' ')
      && distance <= CHAT_HISTORY_DUPLICATE_WINDOW_MS && distance < closestOwnerTime) {
      ownerIndex = index;
      closestOwnerTime = distance;
    }
  }
  if (ownerIndex < 0) return [...messages, reply];
  const nextOwner = messages.findIndex((message, index) => index > ownerIndex && message.role === 'owner');
  let replaced = false;
  const completed = messages.flatMap((message, index) => {
    if (index <= ownerIndex || (nextOwner >= 0 && index >= nextOwner)
      || message.role !== 'aegis'
      || (turnId && messageTurnId(message) && messageTurnId(message) !== turnId)
      || chatHomeAnswerTextKey(message.text) !== chatHomeAnswerTextKey(reply.text)) return [message];
    if (replaced) return [];
    replaced = true;
    return [completedReply(message, reply)];
  });
  return replaced ? completed : [...messages, reply];
}

function completedReply<T extends ChatHomeMergeableMessage>(message: T, reply: T): T {
  return {
    ...message, ...reply, id: message.id, timestamp: message.timestamp ?? reply.timestamp,
    replyOrigin: 'command',
    retryText: undefined, retryIdempotencyKey: undefined, retryLabel: undefined,
    retryContext: undefined, retryQuote: undefined,
  };
}

// Match the server's citation formatting, without dropping references or facts.
export function chatHomeAnswerTextKey(text: string): string {
  return text.replace(/\s*\[(E\d+(?:\s*,\s*E\d+)*)\]/g, (_, ids: string) =>
    ` [${ids.split(',').map((id) => id.trim()).join(', ')}]`).trim().replace(/\s+/g, ' ');
}

function finalLocalResult(message: ChatHomeMergeableMessage): ContextualAssistantResponse | undefined {
  // A finalized command may itself report an error. Only local transport
  // failures and partial history envelopes yield to authoritative history.
  if (message.replyOrigin === 'history' || message.replyOrigin === 'transport-error'
    || (!message.replyOrigin && message.retryText && message.result?.response_kind === 'error')) return undefined;
  return message.result;
}

function mergeRestoredServerTurn<T extends ChatHomeMergeableMessage>(serverMessage: T, localMessage: T): T {
  const localResult = finalLocalResult(localMessage);
  return {
    ...localMessage,
    ...serverMessage,
    ...(messageTurnId(serverMessage) || messageTurnId(localMessage)
      ? { turnId: messageTurnId(serverMessage) ?? messageTurnId(localMessage) } : {}),
    // The history projection has only a subset of the final command envelope.
    ...(localResult ? { replyOrigin: localMessage.replyOrigin, text: localResult.visible_answer, severity: localMessage.severity, result: {
      ...localResult,
      ...(!localResult.evidence?.length && serverMessage.result?.evidence?.length
        ? { evidence: serverMessage.result.evidence } : {}),
      ...(!localResult.unsupported_numbers?.length && serverMessage.result?.unsupported_numbers?.length
        ? { unsupported_numbers: serverMessage.result.unsupported_numbers } : {}),
      ...(!localResult.source_evidence?.length && serverMessage.result?.source_evidence?.length
        ? { source_evidence: serverMessage.result.source_evidence } : {}),
    } } : { result: serverMessage.result }),
    // A finalized local command keeps its own retry controls. History is the
    // authority for a failure hint (and for a recovered answer, which has none).
    ...(localResult ? {
      retryText: localMessage.retryText,
      retryIdempotencyKey: localMessage.retryIdempotencyKey,
      retryLabel: localMessage.retryLabel,
      retryContext: localMessage.retryContext,
      retryQuote: localMessage.retryQuote,
    } : {
      retryText: serverMessage.retryText,
      retryIdempotencyKey: serverMessage.retryIdempotencyKey,
      retryLabel: serverMessage.retryLabel,
      retryContext: serverMessage.retryContext,
      retryQuote: serverMessage.retryQuote,
    }),
  };
}

function isLowValueSystemChatNotice(message: Pick<ChatHomeMergeableMessage, 'role' | 'text'>): boolean {
  if (message.role !== 'aegis') return false;
  const normalized = message.text.trim().replace(/\s+/g, ' ');
  const lower = normalized.toLowerCase();
  if (!normalized) return false;
  if (lower.includes('system restarted') || normalized.includes('系统已重启')) return true;
  if (lower.startsWith('system integrity report') && lower.includes('invariants:') && lower.includes('rate monitors:')) return true;
  if (
    lower.startsWith('daily brief')
    && lower.includes('calendar not queried')
  ) {
    return true;
  }
  return false;
}

function isSameRestoredTurn(
  serverMessage: ChatHomeMergeableMessage,
  localMessage: ChatHomeMergeableMessage,
): boolean {
  if (serverMessage.role !== localMessage.role) return false;
  const serverTurnId = messageTurnId(serverMessage);
  const localTurnId = messageTurnId(localMessage);
  if (serverTurnId && localTurnId) return serverTurnId === localTurnId;
  if (chatTurnKey(serverMessage) !== chatTurnKey(localMessage)) return false;
  const serverTime = sortableChatTimestamp(serverMessage.timestamp);
  const localTime = sortableChatTimestamp(localMessage.timestamp);
  if (serverTime === Number.MAX_SAFE_INTEGER || localTime === Number.MAX_SAFE_INTEGER) return true;
  const window = serverMessage.role === 'aegis'
    ? CHAT_HISTORY_SAME_REPLY_WINDOW_MS
    : CHAT_HISTORY_DUPLICATE_WINDOW_MS;
  return Math.abs(serverTime - localTime) <= window;
}

function chatTurnKey(message: Pick<ChatHomeMergeableMessage, 'role' | 'text'>): string {
  return `${message.role}\n${message.role === 'aegis' ? chatHomeAnswerTextKey(message.text) : message.text.trim().replace(/\s+/g, ' ')}`;
}

function sortableChatTimestamp(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : Number.MAX_SAFE_INTEGER;
}

export function clearChatHomeHistory(storage: StorageLike, key: string) {
  try {
    storage.removeItem(key);
  } catch {
    // Best-effort cleanup only.
  }
}
