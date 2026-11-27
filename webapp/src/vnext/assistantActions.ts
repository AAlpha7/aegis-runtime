import {
  ApiResponseError,
  fetchVNextCommandStatus,
  submitVNextCommand,
} from '../api';
import { assistantCommandFallbackText } from './assistantFallbackText';
import type {
  ContextualAssistantResponse,
  OwnerWorkItem,
  VNextCommandAttachment,
  VNextConversationContextTurn,
} from '../api';
import { contextualResponseFromVNext, ownerItemFromVNext } from './adapters';
import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import type { VNextCommandScope } from './model';
import { shortCommandLabel } from './presentation';
import {
  ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS,
  requestWithStillWorkingAutoCheck,
  type AssistantProgressCallback,
  type StillWorkingStatus,
} from './assistantRetry';

export interface AssistantCommandResult {
  response: ContextualAssistantResponse;
  workItem: OwnerWorkItem | null;
}

export type AssistantCommandRequest = {
  initData: string;
  csrfToken?: string;
  focusId: string;
  intent: 'chat' | 'explain' | 'draft';
  text: string;
  scope: VNextCommandScope;
  /**
   * Web surface for backend `client_context`. Defaults: general→home, item→item.
   * Panel-first asks pass the owner's current view (work/status/settings/…).
   */
  clientContext?: 'home' | 'work' | 'item' | 'status' | 'settings';
  mode?: 'text' | 'voice';
  idempotencyKey?: string;
  conversationContext?: VNextConversationContextTurn[];
  attachments?: VNextCommandAttachment[];
};

type AssistantCommandRequestImpl = (args: AssistantCommandRequest) => Promise<AssistantCommandResult>;
type SleepImpl = (ms: number) => Promise<void>;

export function buildWorkingAssistantResult(text = '', locale: Locale = 'en'): ContextualAssistantResponse {
  return {
    schema_version: 1,
    response_kind: 'working',
    visible_answer: text
      ? tf('chatHome.working.checkingContext', locale, { command: shortCommandLabel(text) })
      : t('chatHome.working.checkingContextGeneric', locale),
    receipt_expectation: t('assistant.working.receipt', locale),
    citations: [],
    warnings: [],
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

/**
 * "Still working" signals: 504 from the command route's foreground wait,
 * 524/502 from the Cloudflare edge when the origin took too long, 408, or a
 * dropped connection (fetch rejects with a TypeError). All of them leave the
 * server-side turn running under the idempotency key, so the answer can be
 * collected with a status poll.
 */
export const STILL_WORKING_STATUS_CODES: ReadonlySet<number> = new Set([408, 502, 504, 524]);

export function isStillWorkingError(error: unknown): boolean {
  if (error instanceof ApiResponseError) return STILL_WORKING_STATUS_CODES.has(error.statusCode);
  return error instanceof TypeError;
}

function shouldAutoCheckAssistantResult(
  error: unknown,
  idempotencyKey: string | undefined,
  checksUsed: number,
): boolean {
  return Boolean(idempotencyKey)
    && checksUsed < ASSISTANT_STILL_WORKING_RETRY_DELAYS_MS.length
    && isStillWorkingError(error);
}

function resultFromCommandResponse(
  response: Awaited<ReturnType<typeof submitVNextCommand>>,
): AssistantCommandResult {
  return {
    response: contextualResponseFromVNext(response),
    workItem: response.work_item ? ownerItemFromVNext(response.work_item) : null,
  };
}

export async function requestAssistantCommand({
  initData,
  csrfToken,
  focusId,
  intent,
  text,
  scope,
  clientContext,
  mode = 'text',
  idempotencyKey,
  conversationContext = [],
  attachments = [],
}: AssistantCommandRequest): Promise<AssistantCommandResult> {
  const resolvedContext = clientContext
    ?? (scope === 'general' ? 'home' : 'item');
  const response = await submitVNextCommand(
    initData,
    csrfToken,
    text || assistantCommandFallbackText(intent, attachments.length > 0),
    resolvedContext,
    scope === 'general' ? null : focusId,
    mode,
    intent,
    idempotencyKey,
    conversationContext,
    // Attachments are only meaningful on the general-chat path; the backend
    // ignores them for the focused/item scope, so passing them is inert there.
    scope === 'general' ? attachments : [],
  );
  return resultFromCommandResponse(response);
}

export async function requestAssistantResult(
  args: AssistantCommandRequest,
): Promise<ContextualAssistantResponse> {
  const result = await requestAssistantCommand(args);
  return result.response;
}

export async function requestAssistantResultWithAutoCheck({
  onStillWorking,
  ...args
}: Parameters<typeof requestAssistantResult>[0] & {
  onStillWorking?: AssistantProgressCallback;
}): Promise<ContextualAssistantResponse> {
  const result = await requestAssistantCommandWithAutoCheck({ ...args, onStillWorking });
  return result.response;
}

export async function requestAssistantCommandWithAutoCheck({
  onStillWorking,
  ...args
}: AssistantCommandRequest & {
  onStillWorking?: AssistantProgressCallback;
}): Promise<AssistantCommandResult> {
  const { initData, csrfToken, focusId, intent, idempotencyKey } = args;
  const checkStatus = idempotencyKey
    ? async () => {
      const status = await fetchVNextCommandStatus(initData, csrfToken, {
        focusId,
        intentKind: intent,
        idempotencyKey,
      });
      return status.kind === 'done'
        ? { kind: 'done' as const, value: resultFromCommandResponse(status.response) }
        : status;
    }
    : undefined;
  return requestAssistantCommandWithAutoCheckUsing({
    ...args,
    onStillWorking,
    requestCommand: requestAssistantCommand,
    sleepImpl: sleep,
    checkStatus,
  });
}

export async function requestAssistantCommandWithAutoCheckUsing({
  onStillWorking,
  requestCommand,
  sleepImpl,
  checkStatus,
  ...args
}: AssistantCommandRequest & {
  onStillWorking?: AssistantProgressCallback;
  requestCommand: AssistantCommandRequestImpl;
  sleepImpl: SleepImpl;
  checkStatus?: () => Promise<StillWorkingStatus<AssistantCommandResult>>;
}): Promise<AssistantCommandResult> {
  return requestWithStillWorkingAutoCheck({
    args,
    request: requestCommand,
    sleep: sleepImpl,
    shouldRetry: shouldAutoCheckAssistantResult,
    onStillWorking,
    checkStatus,
  });
}
