/** API client for Aegis Runtime WebApp backend */

import { readConsoleApprovalActionResponse, readConsoleMandateLifecycleResponse, readConsoleMemoryDeleteResponse, readConsoleMemoryUpdateResponse, readConsoleProfileUpdateResponse, readMfaStatusResponse, readOwnerWorkContextEnvelope, readOwnerWorkQueue, readSessionInfo, readStreamTokenInfo, readTotpSetupResponse, readVNextAvatarProfileProjection, readVNextChatHistoryProjection, readVNextCommandResponse, readVNextDebugSnapshotResponse, readVNextItemProjection, readVNextModelCatalog, readVNextSettingsProjection, readVNextStatusProjection, readVNextWorkActionResponse, readVNextWorkProjection, readWebAuthnCredentialsResponse } from './apiGuards';
import { fetchAssistantCommandWithTransientRetry, fetchJsonWithOneNetworkRetry } from './apiRetry';

const API_BASE = import.meta.env?.VITE_API_BASE ?? '';

function idempotencyKey(prefix: string): string {
  const random = globalThis.crypto?.randomUUID?.();
  if (random) return `${prefix}:${random}`;
  return `${prefix}:${Date.now().toString(36)}:${Math.random().toString(36).slice(2)}`;
}

function authHeaders(initData: string, csrfToken?: string): HeadersInit {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (initData.trim()) {
    headers.Authorization = `TelegramWebApp ${initData}`;
  }
  if (csrfToken) {
    headers['X-Aegis-CSRF'] = csrfToken;
  }
  return headers;
}

export interface SessionInfo {
  authenticated: boolean;
  google_login_enabled?: boolean;
  passkey_login_enabled?: boolean;
  principal_type?: 'web_console' | 'telegram' | 'admin';
  email?: string;
  mfa_level?: string;
  mfa_verified_at?: number | null;
  mfa_expires_at?: number | null;
  mfa_freshness_secs?: number;
  protected_access_unlocked?: boolean;
  requires_mfa_setup?: boolean;
  csrf_token?: string;
  assistant_name?: string;
}

export interface SensitiveAccessProblem {
  status?: 'mfa_required' | 'consent_required' | 'error';
  error?: string;
  error_kind?: string;
  required_level?: string;
  reason?: string;
  freshness_secs?: number;
  resource_type?: string;
  resource_id?: string;
  /** Backend structured-error contract: whether retrying the same call can succeed. */
  retryable?: boolean;
  hint?: string;
  challenge_hint?: {
    start_url?: string;
  };
}

export interface VNextConversationContextTurn {
  role: 'owner' | 'aegis';
  text: string;
  kind?: 'recent' | 'quoted_reply';
  source_id?: string;
}

/**
 * Slice 5: a multimodal attachment payload sent with a vNext command. Tagged by
 * `type` to mirror the backend `VNextCommandAttachment` enum. Image/Document
 * carry a base64 `data_url` (produced by `/webapp/vnext/upload`); Voice carries
 * a pre-transcribed transcript (no audio bytes).
 */
export type VNextCommandAttachment =
  | { type: 'image'; data_url: string }
  | { type: 'voice'; transcript: string; duration_secs?: number }
  | { type: 'document'; data_url: string; filename: string; mime_type: string };

export class ApiResponseError extends Error {
  readonly statusCode: number;
  readonly problem?: SensitiveAccessProblem;

  constructor(message: string, statusCode: number, problem?: SensitiveAccessProblem) {
    super(message);
    this.name = 'ApiResponseError';
    this.statusCode = statusCode;
    this.problem = problem;
  }
}

export function isSensitiveAccessProblem(value: unknown): value is SensitiveAccessProblem {
  if (!value || typeof value !== 'object') return false;
  const problem = value as SensitiveAccessProblem;
  return problem.status === 'mfa_required'
    || problem.status === 'consent_required'
    || problem.error === 'mfa_required'
    || problem.error === 'consent_required'
    || problem.error_kind === 'mfa_required'
    || problem.error_kind === 'consent_required';
}

async function parseJsonOrText(res: Response): Promise<{ json?: unknown; text: string }> {
  const text = await res.text();
  if (!text.trim()) return { text };
  try {
    return { json: JSON.parse(text), text };
  } catch {
    return { text };
  }
}

function problemMessage(problem: SensitiveAccessProblem, fallback: string): string {
  if (problem.error === 'mfa_required' || problem.error_kind === 'mfa_required' || problem.status === 'mfa_required') {
    return 'Extra owner confirmation is required before Aegis can continue.';
  }
  if (problem.error === 'consent_required' || problem.error_kind === 'consent_required' || problem.status === 'consent_required') {
    return 'Google needs permission before this selected detail can open.';
  }
  return problem.error || fallback;
}

/**
 * Detects the backend's structured tool-error contract on a NON-2xx body:
 * `{status:"error", error, error_kind?, retryable?, hint?}` (e.g. a tool
 * `page1_store_unavailable` 503). Attaching it as the thrown
 * `ApiResponseError.problem` lets consumers surface the owner-facing `error`
 * string and honor `retryable`/`hint` instead of a generic status-code
 * message. Only consulted on error responses — a 200 body with
 * `status:"error"` is a structured RESULT the caller parses, never a throw.
 */
function isStructuredErrorBody(value: unknown): value is SensitiveAccessProblem {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const body = value as SensitiveAccessProblem;
  return typeof body.error === 'string' && body.error.length > 0;
}

async function expectJson(res: Response, fallback: string): Promise<unknown> {
  const parsed = await parseJsonOrText(res);
  if (!res.ok) {
    const problem = isSensitiveAccessProblem(parsed.json)
      ? parsed.json
      : isStructuredErrorBody(parsed.json)
        ? parsed.json
        : undefined;
    throw new ApiResponseError(
      problem ? problemMessage(problem, fallback) : `${fallback} (${res.status}): ${parsed.text}`,
      res.status,
      problem,
    );
  }
  if (isSensitiveAccessProblem(parsed.json)) {
    throw new ApiResponseError(problemMessage(parsed.json, fallback), res.status, parsed.json);
  }
  return parsed.json ?? {};
}

async function expectDetailJson(res: Response, fallback: string): Promise<unknown> {
  const parsed = await parseJsonOrText(res);
  if (!res.ok) {
    const problem = isSensitiveAccessProblem(parsed.json) ? parsed.json : undefined;
    throw new ApiResponseError(
      problem ? problemMessage(problem, fallback) : `${fallback} (${res.status}): ${parsed.text}`,
      res.status,
      problem,
    );
  }
  return parsed.json ?? {};
}

export async function fetchSession(initData = ''): Promise<SessionInfo> {
  const res = await fetch(`${API_BASE}/api/session`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Session check failed');
  const session = readSessionInfo(json);
  if (!session) throw new ApiResponseError('Session check failed: unexpected response', res.status);
  return session;
}

export async function logout(csrfToken: string): Promise<void> {
  const res = await fetch(`${API_BASE}/api/logout`, {
    method: 'POST',
    headers: {
      'X-Aegis-CSRF': csrfToken,
    },
    credentials: 'include',
  });

  if (!res.ok) await expectJson(res, 'Logout failed');
}

export interface MfaStatus {
  mfa_level: string;
  mfa_verified_at?: number | null;
  requires_mfa_setup: boolean;
  webauthn_registered_count: number;
  webauthn_available?: boolean;
  webauthn_status?: string;
  totp_configured: boolean;
  totp_registered: boolean;
  preferred_step_up_method?: string;
  recovery_codes_remaining: number;
}

export interface MfaStatusResponse {
  authenticated: boolean;
  mfa: MfaStatus;
}

export interface WebAuthnCredentialSummary {
  id: string;
  nickname?: string | null;
  transports: string[];
  backup_eligible?: boolean | null;
  backup_state?: boolean | null;
  created_at: number;
  last_used_at?: number | null;
}

export interface TotpSetup {
  secret_base32: string;
  otpauth_url: string;
  issuer: string;
  account_name: string;
  digits: number;
  period_secs: number;
  algorithm: string;
}

export interface ConsoleApprovalActionResult {
  status: string;
  approval_id: string;
  action: 'approve' | 'reject';
  risk: string;
  approval_type: string;
  message: string;
}

export interface ConsoleMandateSummary {
  schema_version: number;
  mandate_id: string;
  status: string;
  title: string;
  description: string;
  scope_kind: string;
  allowed_reads: string[];
  allowed_llm_uses: string[];
  allowed_actions: string[];
  must_ask: string[];
  notification_policy: Record<string, unknown>;
  budget_policy: Record<string, unknown>;
  frequency_policy: Record<string, unknown>;
  expiry?: string | null;
  review_after?: string | null;
  created_at: string;
  updated_at: string;
  created_from: string;
  source_event_id?: string | null;
  trace_id?: string | null;
  run_id?: string | null;
  matching_enabled: boolean;
  background_autonomy_enabled: boolean;
}

export interface ConsoleMandateLifecycleResult {
  status: string;
  mandate: ConsoleMandateSummary;
  receipt_expectation: string;
}

export interface ConsoleMandateDraftRequest {
  schema_version: 1;
  title: string;
  description: string;
  scope_kind: 'email_metadata' | 'calendar_metadata' | 'system_alerts' | 'api_costs' | 'approvals' | 'receipts' | 'memory_review_queue';
  source_filters: Record<string, string>;
  allowed_reads: string[];
  allowed_llm_uses: string[];
  allowed_actions: string[];
  must_ask: string[];
  notification_policy: Record<string, unknown>;
  budget_policy: Record<string, unknown>;
  frequency_policy: Record<string, unknown>;
}

export interface ConsoleMemoryDeleteResult {
  status: string;
  resource_type: string;
  resource_id: string;
  deleted: boolean;
  ledger_entry_id: number;
  receipt: string;
}

export interface ConsoleTaskActionResult {
  status: string;
  resource_type: string;
  resource_id: string;
  action: 'complete' | 'snooze' | 'delete';
  updated: boolean;
  ledger_entry_id: number;
  receipt: string;
  summary: string;
  due_at?: string | null;
  /** True when complete kept a clock loop on the same row. */
  recurring?: boolean;
}

export interface ConsoleMemoryUpdateResult {
  status: string;
  resource_type: string;
  resource_id: string;
  updated: boolean;
  ledger_entry_id: number;
  receipt: string;
}

export interface ConsoleProfileUpdateResult {
  status: string;
  resource_type: string;
  resource_id: string;
  updated: boolean;
  ledger_entry_id: number;
  receipt: string;
}

/**
 * Mint a short-lived stream token for the per-turn progress SSE stream
 * (Slice 3). `turnId` is the command's `idempotency_key`; the backend
 * (`POST /webapp/vnext/turns/:turn_id/stream-token`, console auth + CSRF) binds
 * the token to this turn + the principal's chat, so it can only open the
 * matching `…/events` SSE GET. Returns the raw token string for the EventSource
 * query string. Advisory-only: callers should swallow failures and degrade to a
 * generic working message (the final result still arrives via the status-poll).
 */
export async function mintVNextTurnStreamToken(
  initData: string,
  csrfToken: string | undefined,
  turnId: string,
): Promise<string> {
  const res = await fetch(
    `${API_BASE}/webapp/vnext/turns/${encodeURIComponent(turnId)}/stream-token`,
    {
      method: 'POST',
      headers: authHeaders(initData, csrfToken),
      credentials: 'include',
      body: JSON.stringify({}),
    },
  );

  const json = await expectJson(res, 'Turn stream token failed');
  const tokenInfo = readStreamTokenInfo(json);
  if (!tokenInfo) {
    throw new ApiResponseError('Turn stream token failed: unexpected response', res.status);
  }
  return tokenInfo.stream_token;
}

export async function fetchMfaStatus(): Promise<MfaStatusResponse> {
  const res = await fetch(`${API_BASE}/mfa/status`, {
    credentials: 'include',
  });

  const json = await expectJson(res, 'MFA status failed');
  const status = readMfaStatusResponse(json);
  if (!status) throw new ApiResponseError('MFA status failed: unexpected response', res.status);
  return status;
}

export async function fetchWebAuthnCredentials(): Promise<WebAuthnCredentialSummary[]> {
  const res = await fetch(`${API_BASE}/mfa/webauthn/credentials`, {
    credentials: 'include',
  });

  const json = await expectJson(res, 'WebAuthn credential load failed');
  const credentials = readWebAuthnCredentialsResponse(json);
  if (!credentials) throw new ApiResponseError('WebAuthn credential load failed: unexpected response', res.status);
  return credentials;
}

export async function startTotpSetup(csrfToken: string): Promise<TotpSetup> {
  const res = await fetch(`${API_BASE}/mfa/totp/setup/start`, {
    method: 'POST',
    headers: authHeaders('', csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, 'TOTP setup failed');
  const setup = readTotpSetupResponse(json);
  if (!setup) throw new ApiResponseError('TOTP setup failed: unexpected response', res.status);
  return setup.setup;
}

export async function finishTotpSetup(csrfToken: string, code: string): Promise<void> {
  const res = await fetch(`${API_BASE}/mfa/totp/setup/finish`, {
    method: 'POST',
    headers: authHeaders('', csrfToken),
    credentials: 'include',
    body: JSON.stringify({ code }),
  });

  await expectJson(res, 'TOTP setup verification failed');
}

export async function confirmLocalHostStepUp(csrfToken: string): Promise<void> {
  const res = await fetch(`${API_BASE}/mfa/host-confirm`, {
    method: 'POST',
    headers: authHeaders('', csrfToken),
    credentials: 'include',
  });
  await expectJson(res, 'Host confirmation failed');
}

export async function verifyTotp(csrfToken: string, code: string): Promise<void> {
  const res = await fetch(`${API_BASE}/mfa/totp/verify`, {
    method: 'POST',
    headers: authHeaders('', csrfToken),
    credentials: 'include',
    body: JSON.stringify({ code }),
  });

  await expectJson(res, 'TOTP verification failed');
}

export async function approveConsoleApproval(
  approvalId: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleApprovalActionResult> {
  return consoleApprovalAction(approvalId, 'approve', initData, csrfToken);
}

export async function stopOwnerResearch(initData: string, csrfToken?: string): Promise<void> {
  const res = await fetch(`${API_BASE}/webapp/vnext/research/stop`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({}),
  });
  await expectJson(res, 'Research stop failed');
}

export async function rejectConsoleApproval(
  approvalId: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleApprovalActionResult> {
  return consoleApprovalAction(approvalId, 'reject', initData, csrfToken);
}

export async function updateConsoleMandateLifecycle(
  mandateId: string,
  action: 'activate' | 'pause' | 'resume' | 'revoke',
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMandateLifecycleResult> {
  const res = await fetch(`${API_BASE}/webapp/console/mandates/${encodeURIComponent(mandateId)}/${action}`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, `Mandate ${action} failed`);
  const result = readConsoleMandateLifecycleResponse(json);
  if (!result) throw new ApiResponseError(`Mandate ${action} failed: unexpected response`, res.status);
  return result;
}

export async function createConsoleMandateDraft(
  draft: ConsoleMandateDraftRequest,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMandateLifecycleResult> {
  const res = await fetch(`${API_BASE}/webapp/console/mandates`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify(draft),
  });

  const json = await expectJson(res, 'Mandate draft create failed');
  const result = readConsoleMandateLifecycleResponse(json);
  if (!result) throw new ApiResponseError('Mandate draft create failed: unexpected response', res.status);
  return result;
}

export async function editConsoleMandateDraft(
  mandateId: string,
  draft: ConsoleMandateDraftRequest,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMandateLifecycleResult> {
  const res = await fetch(`${API_BASE}/webapp/console/mandates/${encodeURIComponent(mandateId)}/edit`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify(draft),
  });

  const json = await expectJson(res, 'Mandate draft edit failed');
  const result = readConsoleMandateLifecycleResponse(json);
  if (!result) throw new ApiResponseError('Mandate draft edit failed: unexpected response', res.status);
  return result;
}

export async function updateConsoleTaskAction(
  taskId: string,
  action: 'complete' | 'snooze' | 'delete',
  initData: string,
  csrfToken?: string,
  preset?: '10m' | '1h' | 'tomorrow',
): Promise<ConsoleTaskActionResult> {
  const res = await fetch(`${API_BASE}/webapp/console/tasks/${encodeURIComponent(taskId)}/action`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      action,
      ...(preset ? { preset } : {}),
    }),
  });

  const json = await expectJson(res, `Task ${action} failed`);
  if (!json || typeof json !== 'object') {
    throw new ApiResponseError(`Task ${action} failed: unexpected response`, res.status);
  }
  const result = json as Partial<ConsoleTaskActionResult>;
  if (
    result.status !== 'ok'
    || result.resource_type !== 'task'
    || typeof result.resource_id !== 'string'
    || result.action !== action
    || result.updated !== true
    || typeof result.ledger_entry_id !== 'number'
    || typeof result.receipt !== 'string'
    || typeof result.summary !== 'string'
  ) {
    throw new ApiResponseError(`Task ${action} failed: unexpected response`, res.status);
  }
  return result as ConsoleTaskActionResult;
}

async function consoleApprovalAction(
  approvalId: string,
  action: 'approve' | 'reject',
  initData: string,
  csrfToken?: string,
): Promise<ConsoleApprovalActionResult> {
  const res = await fetch(`${API_BASE}/webapp/console/approvals/${encodeURIComponent(approvalId)}/${action}`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, `Approval ${action} failed`);
  const result = readConsoleApprovalActionResponse(json);
  if (!result) throw new ApiResponseError(`Approval ${action} failed: unexpected response`, res.status);
  return result;
}

export async function deleteConsoleMemoryItem(
  itemId: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMemoryDeleteResult> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/items/${encodeURIComponent(itemId)}/delete`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Memory delete failed');
  const result = readConsoleMemoryDeleteResponse(json);
  if (!result) throw new ApiResponseError('Memory delete failed: unexpected response', res.status);
  return result;
}

export async function fetchConsoleMemoryItem(
  itemId: string,
  initData: string,
  csrfToken?: string,
): Promise<unknown> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/items/${encodeURIComponent(itemId)}`, {
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  return expectDetailJson(res, 'Memory detail failed');
}

export async function updateConsoleMemoryItem(
  itemId: string,
  factSummary: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMemoryUpdateResult> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/items/${encodeURIComponent(itemId)}/update`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({ fact_summary: factSummary }),
  });

  const json = await expectJson(res, 'Memory update failed');
  const result = readConsoleMemoryUpdateResponse(json);
  if (!result) throw new ApiResponseError('Memory update failed: unexpected response', res.status);
  return result;
}

export async function setConsoleMemoryItemPinned(
  itemId: string,
  pinned: boolean,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMemoryUpdateResult> {
  const action = pinned ? 'pin' : 'unpin';
  const res = await fetch(`${API_BASE}/webapp/console/memory/items/${encodeURIComponent(itemId)}/${action}`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, `Memory ${action} failed`);
  const result = readConsoleMemoryUpdateResponse(json);
  if (!result) throw new ApiResponseError(`Memory ${action} failed: unexpected response`, res.status);
  return result;
}

export async function markConsoleMemoryItemWrong(
  itemId: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleMemoryUpdateResult> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/items/${encodeURIComponent(itemId)}/mark-wrong`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Memory mark-wrong failed');
  const result = readConsoleMemoryUpdateResponse(json);
  if (!result) throw new ApiResponseError('Memory mark-wrong failed: unexpected response', res.status);
  return result;
}

export async function fetchConsoleProfileSlot(
  category: string,
  key: string,
  initData: string,
  csrfToken?: string,
): Promise<unknown> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/profile/${encodeURIComponent(category)}/${encodeURIComponent(key)}`, {
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  return expectDetailJson(res, 'Profile detail failed');
}

export async function updateConsoleProfileSlot(
  category: string,
  key: string,
  value: string,
  initData: string,
  csrfToken?: string,
): Promise<ConsoleProfileUpdateResult> {
  const res = await fetch(`${API_BASE}/webapp/console/memory/profile/${encodeURIComponent(category)}/${encodeURIComponent(key)}/update`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({ value }),
  });

  const json = await expectJson(res, 'Profile update failed');
  const result = readConsoleProfileUpdateResponse(json);
  if (!result) throw new ApiResponseError('Profile update failed: unexpected response', res.status);
  return result;
}

export async function fetchConsoleTaskTimeline(
  taskId: string,
  initData: string,
  csrfToken?: string,
): Promise<unknown> {
  const res = await fetch(`${API_BASE}/webapp/console/tasks/${encodeURIComponent(taskId)}/timeline`, {
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  return expectDetailJson(res, 'Task detail failed');
}

export async function fetchConsoleLedgerEntry(
  entryId: string,
  initData: string,
  csrfToken?: string,
): Promise<unknown> {
  const res = await fetch(`${API_BASE}/webapp/console/ledger/${encodeURIComponent(entryId)}`, {
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  return expectDetailJson(res, 'Action receipt detail failed');
}

// ── Console API (Phase 1b) ────────────────────────────────────────────────

/** One hop of a work-card source chain. Smaller hops come first. */
export interface WorkLineageNode {
  layer: string;
  kind: string;
  id: string;
  label: string;
  label_code?: string | null;
  detail: string;
  action_ids?: string[];
  /** Webapp turn id for a chat-message hop. With `message_role`, this is the cite. */
  turn_id?: string | null;
  /** `owner` or `aegis`. Set only together with `turn_id`. */
  message_role?: string | null;
}

export interface OwnerWorkAction {
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

export interface OwnerWorkItem {
  id: string;
  kind: string;
  source_kind?: string;
  title: string;
  /** Optional structural code for `title` (`work.decide.<code>`). */
  title_code?: string | null;
  summary: string;
  owner_visible_reason?: string;
  /**
   * Optional structural code for `owner_visible_reason`
   * (`work.reasonCode.<code>`); absent/unknown falls back to the prose.
   */
  owner_visible_reason_code?: string | null;
  authority_basis?: string;
  source: string;
  source_id: string;
  source_event_id?: string | null;
  status: string;
  urgency: string;
  sensitivity: string;
  privacy_level: string;
  created_at: string;
  updated_at: string;
  due_at?: string | null;
  primary_action?: OwnerWorkAction | null;
  secondary_actions: OwnerWorkAction[];
  next_safe_action?: string;
  /** Optional structural code for `next_safe_action` (`work.nextCode.<code>`). */
  next_safe_action_code?: string | null;
  blocked_by?: string | null;
  /** Optional structural code for `blocked_by` (`work.reasonCode.<code>`). */
  blocked_by_code?: string | null;
  receipt_expectation?: string;
  authority?: AuthorityEnvelope | null;
  detail_route?: string | null;
  external_url_policy: string;
  related_action_ids: string[];
  related_receipt_ids: string[];
  route_reason: string;
  schema_version: number;
  decision_request?: VNextDecisionRequest | null;
  /** Source chain. Absent on items that are not a follow-up. */
  lineage?: WorkLineageNode[];
}

export interface OwnerWorkQueue {
  schema_version: number;
  generated_at: string;
  owner: string;
  sources: { source: string; status: string; reason?: string | null }[];
  counts: Record<string, number>;
  items: OwnerWorkItem[];
}

export interface OwnerWorkContextGate {
  gate_kind: string;
  reason: string;
  route?: string | null;
  requires_mfa: boolean;
}

export interface OwnerWorkContextEnvelope {
  schema_version: number;
  focus_id: string;
  focus_kind: string;
  focus_source: string;
  visible_fields: Record<string, string>;
  redacted_fields: string[];
  authorized_sensitive_scopes: string[];
  denied_sensitive_scopes: string[];
  allowed_intents: string[];
  required_gate?: OwnerWorkContextGate | null;
  related_action_ids: string[];
  related_receipt_ids: string[];
  source_event_id?: string | null;
  trace_id?: string | null;
  run_id?: string | null;
}

export interface AuthorityEnvelope {
  item_id: string;
  authority_basis: string;
  visible_fields: string[];
  locked_fields: string[];
  allowed_now: string[];
  requires_data_gate: boolean;
  requires_mfa: boolean;
  requires_approval: boolean;
  expected_receipt: string;
  source_mandate_id?: string | null;
  audit_links: string[];
}

export interface VNextStatusSummary {
  label: string;
  health: string;
  owner_message: string;
  owner_next_step?: string | null;
}

export interface VNextActionRef {
  id: string;
  label: string;
  /** Optional structural code for `label` (`work.actionLabel.<code>`). */
  label_code?: string | null;
  kind: string;
  route?: string | null;
  requires_confirmation: boolean;
  requires_mfa: boolean;
  requires_data_gate: boolean;
  expected_receipt: string;
}

export interface VNextDecisionChoice {
  id: string;
  label: string;
  consequence: string;
  risk_level: string;
  applies_to: string;
  action_kind?: string | null;
  assistant_prompt?: string | null;
  detail_route?: string | null;
  requires_confirmation: boolean;
  enabled: boolean;
  disabled_reason?: string | null;
  tone: 'normal' | 'destructive' | string;
}

export interface VNextDecisionRequest {
  id: string;
  title: string;
  /** Structural code for `title` (catalog key `work.decide.<code>`). */
  title_code?: string | null;
  what_aegis_was_doing: string;
  /** Structural code for `what_aegis_was_doing`. Absent when freeform. */
  what_code?: string | null;
  blocked_reason: string;
  /**
   * Structural code for `blocked_reason` (`work.reasonCode.<code>`). Chosen at
   * the same site as the string so they cannot drift.
   */
  blocked_reason_code?: string | null;
  requested_capability: string;
  /** Structural code for `requested_capability`. */
  capability_code?: string | null;
  scope: string;
  /** Structural code for `scope`. */
  scope_code?: string | null;
  choices: VNextDecisionChoice[];
  safe_details: string[];
  /**
   * Aligned 1:1 with `safe_details`; empty string marks an uncoded (freeform)
   * entry. Length mismatch → FE keeps server strings (defensive).
   */
  safe_detail_codes?: string[] | null;
  audit_refs: string[];
  receipt_expectation: string;
}

export interface VNextAuthorityEnvelope {
  basis: string;
  owner_visible_reason: string;
  visible_data: string[];
  locked_data: string[];
  allowed_now: string[];
  requires_mfa: boolean;
  requires_approval: boolean;
  requires_data_grant: boolean;
  expected_receipt?: string | null;
  source_mandate_id?: string | null;
  audit_refs: string[];
}

export interface VNextProjectedWorkItem {
  id: string;
  lane: string;
  source_kind: string;
  source: string;
  source_id: string;
  source_event_id?: string | null;
  title: string;
  /** Optional structural code for `title` (`work.decide.<code>`). */
  title_code?: string | null;
  summary: string;
  why_it_matters: string;
  /**
   * Optional stable structural code for `why_it_matters` (i18n catalog key
   * `work.reasonCode.<code>`). Absent/unknown codes fall back to the prose.
   */
  why_it_matters_code?: string | null;
  state: string;
  urgency: string;
  sensitivity: string;
  created_at: string;
  updated_at: string;
  due_at?: string | null;
  authority: VNextAuthorityEnvelope;
  recommended_action?: VNextActionRef | null;
  contextual_actions: VNextActionRef[];
  next_safe_action: string;
  /** Optional structural code for `next_safe_action` (`work.nextCode.<code>`). */
  next_safe_action_code?: string | null;
  blocked_by?: string | null;
  /** Optional structural code for `blocked_by` (`work.reasonCode.<code>`). */
  blocked_by_code?: string | null;
  detail_route?: string | null;
  external_url_policy: string;
  route_reason: string;
  related_work_item_ids: string[];
  related_receipt_ids: string[];
  receipt_expectation?: string | null;
  decision_request?: VNextDecisionRequest | null;
  lineage?: WorkLineageNode[];
}

export interface VNextWorkProjection {
  schema_version: number;
  generated_at: string;
  sources: { source: string; status: string; reason?: string | null }[];
  counts: OwnerWorkQueue['counts'];
  lanes: Array<{
    id: string;
    title: string;
    collapsed: boolean;
    items: VNextProjectedWorkItem[];
  }>;
}

export interface VNextItemProjection {
  schema_version: number;
  generated_at: string;
  item: VNextProjectedWorkItem;
  safe_source_detail: string;
  history: Array<{ id: string; kind: string; label: string; created_at: string }>;
  collaboration: {
    focus_id: string;
    placeholder: string;
    suggested_prompts: string[];
  };
}

/**
 * Present (non-null) only when the LLM provider is currently degraded/unavailable.
 * Drives the sticky degraded banner in the app shell. Absent or null means the
 * provider is healthy — consumers MUST treat a missing/null value as "no banner".
 */
export interface VNextLlmDegraded {
  /** Owner-facing reason string (already localized server-side / model-authored). */
  reason: string;
  /**
   * RFC3339 timestamp marking when the current outage began. The banner dismiss
   * is keyed to this value so a NEW outage (different `since`) re-shows.
   */
  since: string;
  /** The degraded provider (openrouter | codex_oauth | xai_subscription). */
  provider: string;
}

/**
 * Present (non-null) only when the primary embedding model is degraded — embed
 * calls are served from the backup model (or fail entirely when no backup
 * exists) and semantic dedup is PAUSED, so duplicate proposals/reminders can
 * leak through. Drives the sticky embedder-degraded banner in the app shell.
 * Absent or null means healthy — consumers MUST render nothing in that case.
 */
export interface VNextEmbedderDegraded {
  /** RFC3339 timestamp of when the current degraded episode began (dismiss key). */
  since: string;
  /** The primary (preferred) embedding model that is currently down. */
  primary_model: string;
  /** The backup model now serving embed calls (`null` if no backup exists). */
  backup_model: string | null;
}

export interface VNextStatusProjection {
  schema_version: number;
  generated_at: string;
  sections: VNextStatusSummary[];
  system_events: VNextSystemEvent[];
  advanced_diagnostics_available: boolean;
  /**
   * Present only when the LLM provider is degraded/unavailable; null or absent
   * when healthy. Validated defensively by `readVNextStatusProjection`: a
   * malformed shape is coerced to `undefined` (treated as healthy) rather than
   * rejecting the whole projection.
   */
  llm_degraded?: VNextLlmDegraded | null;
  /**
   * Present only when the primary embedding model is degraded (semantic dedup
   * paused); null or absent when healthy. Same defensive-validation contract
   * as `llm_degraded`.
   */
  embedder_degraded?: VNextEmbedderDegraded | null;
}

export interface VNextSystemEvent {
  id: string;
  kind: string;
  severity: 'quiet' | 'notice' | 'warning' | 'critical' | string;
  title: string;
  summary: string;
  occurred_at: number;
  window_start_at: number;
  count: number;
  details: string[];
}

export interface VNextSettingsProjection {
  schema_version: number;
  signed_in_owner: string;
  connected_services: VNextStatusSummary[];
  security: VNextStatusSummary[];
  notification_preferences: VNextStatusSummary[];
  cost_saving_paused: boolean;
  advanced_diagnostics_available: boolean;
  /**
   * Owner's configured IANA timezone (e.g. `Asia/Shanghai`). Passed to
   * `Intl.DateTimeFormat({ timeZone })` so user-facing times render in the
   * owner's timezone rather than the browser's. Optional for forward/backward
   * compatibility with servers that predate this field; consumers fall back to
   * the browser timezone when absent.
   */
  user_timezone?: string;
  /**
   * The owner's configured display language (`'zh'` or `'en'`). Drives the
   * webapp locale (see `webapp/src/i18n`). Optional for forward/backward
   * compatibility with servers that predate this field; consumers default to
   * `'zh'` when absent (via `normalizeLocale`).
   */
  display_language?: string;
  /**
   * The owner's configured LLM provider. One of `'openrouter'` | `'codex_oauth'`
   * | `'xai_subscription'`. Optional for forward/backward compatibility with
   * servers that predate this field; the provider selector treats an absent
   * value as `'openrouter'` (the historical default).
   */
  llm_provider?: string;
  /**
   * The owner's configured LLM model id from the current provider's catalog.
   * Empty means "provider default". Optional for forward/backward compatibility
   * with servers that predate this field.
   */
  llm_model?: string;
  /**
   * Whether the owner has opted into falling back to the paid metered API when
   * the configured (subscription/oauth) provider is unavailable. Optional for
   * forward/backward compatibility; consumers default to `false` when absent.
   */
  paid_api_fallback_enabled?: boolean;
  /**
   * Web search provider. `'xai_subscription'` (default) uses the signed-in
   * Grok subscription. `'openrouter'` keeps native `:online` search.
   * Optional for servers that predate the field; the selector defaults to
   * `'xai_subscription'`.
   */
  web_search_provider?: string;
  /**
   * Effective web search model. Subscription default is `grok-4.7`.
   * OpenRouter values include the `:online` suffix.
   */
  web_search_model?: string;
  /**
   * Models the search selector may offer for `web_search_provider`.
   * The server owns the list so the console does not hard-code slugs.
   */
  web_search_model_options?: string[];
}

export interface VNextCatalogModel {
  id: string;
  display_name: string;
}

export interface VNextProviderCatalog {
  id: string;
  connected: boolean;
  models: VNextCatalogModel[];
  source: string;
  fetched_at_unix?: number | null;
  /** Optional provider default model id when the catalog exposes one. */
  default_model?: string;
}

export interface VNextModelCatalog {
  schema_version: number;
  /** Catalog-confirmed suggestion only; does not authorize or select a model. */
  recommended_api_model?: string | null;
  providers: VNextProviderCatalog[];
  current: {
    provider: string;
    model: string;
    /** Optional resolved default model for the "provider default" option label. */
    effective_model?: string;
  };
  fallback: { provider?: string | null; model: string };
}

export interface VNextWebPushStatus {
  schema_version: number;
  configured: boolean;
  send_configured: boolean;
  subscribed: boolean;
  subscription_count: number;
  public_key?: string | null;
  last_delivery?: VNextWebPushDeliverySummary | null;
  owner_message: string;
  owner_next_step?: string | null;
}

export interface VNextWebPushDeliverySummary {
  id: string;
  status: string;
  delivered: number;
  attempted: number;
  removed: number;
  created_at_unix: number;
}

export interface VNextWebPushTestResponse {
  schema_version: number;
  status: string;
  attempted: number;
  delivered: number;
  failed: number;
  removed: number;
  receipt: string;
  owner_message: string;
}

export interface VNextGrokStatus {
  schema_version: number;
  provider: 'xai_subscription';
  enabled: boolean;
  selected_provider: string;
  active_for_chat: boolean;
  connected: boolean;
  pending_device_code: boolean;
  model: string;
  foreground_only: boolean;
  daily_request_limit: number;
  account_label?: string | null;
  owner_message: string;
  owner_next_step?: string | null;
}

export interface VNextCodexStatus {
  schema_version: number;
  provider: 'codex_oauth';
  enabled: boolean;
  selected_provider: string;
  active_for_chat: boolean;
  connected: boolean;
  binary_available: boolean;
  auth_kind?: string | null;
  model: string;
  reasoning_effort: string;
  model_options: VNextCodexModelOption[];
  model_options_source: string;
  tool_calls_supported: boolean;
  tool_routing_policy: string;
  foreground_only: boolean;
  daily_request_limit: number;
  cli_binary: string;
  cli_home: string;
  cli_workdir: string;
  owner_message: string;
  owner_next_step?: string | null;
  diagnostic?: string | null;
}

export interface VNextCodexModelOption {
  slug: string;
  display_name: string;
  description?: string;
  default_reasoning_level: string;
  supported_reasoning_levels: string[];
}

export interface VNextCodexDeviceCodeResponse {
  schema_version: number;
  status: 'pending';
  verification_uri: string;
  verification_uri_complete?: string | null;
  user_code: string;
  expires_at_ms: number;
  interval_secs: number;
  message: string;
}

export interface VNextCodexPollResponse {
  schema_version: number;
  status: 'pending' | 'connected';
  verification_uri?: string | null;
  verification_uri_complete?: string | null;
  user_code?: string | null;
  expires_at_ms?: number;
  interval_secs?: number;
  message: string;
}

export interface VNextGrokDeviceCodeResponse {
  schema_version: number;
  status: 'pending';
  verification_uri: string;
  verification_uri_complete?: string | null;
  user_code: string;
  expires_at_ms: number;
  interval_secs: number;
  message: string;
}

export interface VNextGrokPollResponse {
  schema_version: number;
  status: 'pending' | 'connected' | 'revoked';
  interval_secs?: number;
  account_label?: string | null;
  message: string;
}

export interface VNextChatApprovalCard {
  approval_id: string;
  tool_name: string;
  summary: string;
  state: 'pending' | 'approved' | 'rejected';
  timestamp: number;
  /** `result` or `failure` plus the sentence shown on a settled search card. */
  outcome?: string | null;
}

export interface VNextChatHistoryProjection {
  schema_version: number;
  /** Older messages exist before messages[0]; page with `before`. */
  has_more?: boolean;
  /** Tool holds for this thread. Absent on older servers. */
  approvals?: VNextChatApprovalCard[];
  messages: Array<{
    turn_id?: string | null;
    role: 'owner' | 'aegis';
    text: string;
    timestamp: number;
    severity?: NoticeSeverity | null;
    work_item_id?: string | null;
    evidence?: ContextualAssistantEvidenceItem[];
    source_evidence?: ContextualAssistantSourceEvidence[];
    unsupported_numbers?: string[];
    /** Owner text to send again when this row is a failure hint. */
    retry_text?: string | null;
  }>;
}

export type NoticeSeverity = 'info' | 'notice' | 'important' | 'warning' | 'critical';

export interface VNextAvatarProfileProjection {
  schema_version: number;
  aegis_avatar_url: string;
  owner_avatar_url: string;
  updated_at_unix?: number | null;
  auto_refresh?: VNextAvatarAutoRefreshProjection;
}

export interface VNextAvatarAutoRefreshProjection {
  enabled: boolean;
  provider: string;
  interval_days: number;
  image_generation_available: boolean;
  status: string;
  owner_message: string;
  owner_next_step?: string | null;
  last_refresh_at_unix?: number | null;
  next_refresh_at_unix?: number | null;
  prompt_hint: string;
}

export interface VNextAegisAvatarGenerateResponse {
  schema_version: number;
  status: string;
  owner_message: string;
  provider: string;
  provider_request_id?: string | null;
  profile: VNextAvatarProfileProjection;
}

export interface VNextReceiptRef {
  id: string;
  kind: string;
  label: string;
  created_at: string;
}

export interface VNextCommandResponse {
  schema_version: number;
  generated_at: string;
  outcome: string;
  owner_message: string;
  assistant_response?: unknown;
  work_item?: VNextProjectedWorkItem | null;
  action_draft?: VNextActionRef | null;
  permission_request?: VNextAuthorityEnvelope | null;
  receipt?: VNextReceiptRef | null;
}

export interface VNextWorkActionResponse {
  schema_version: number;
  generated_at: string;
  status: string;
  item_id: string;
  action_id: string;
  owner_message: string;
  receipt?: VNextReceiptRef | null;
  raw_result?: unknown;
}

export interface VNextDebugSnapshotResponse {
  schema_version: number;
  id: string;
  expires_at_unix: number;
}

export interface VNextDebugLease {
  id: string;
  owner: string;
  principal_type: string;
  scope: string;
  permissions: string[];
  item_id?: string | null;
  reason: string;
  created_at_unix: number;
  expires_at_unix: number;
  revoked_at_unix?: number | null;
}

export interface VNextDebugLeaseResponse {
  schema_version: number;
  lease: VNextDebugLease;
  owner_message: string;
  debug_grant_token?: string | null;
}

export interface VNextDebugLeasesResponse {
  schema_version: number;
  leases: VNextDebugLease[];
}

export interface VNextDebugLeaseRevokeResponse {
  schema_version: number;
  revoked: number;
}

export interface VNextDebugJob {
  id: string;
  owner: string;
  principal_type: string;
  kind: string;
  status: string;
  summary: string;
  log_text: string;
  exit_code?: number | null;
  lease_id?: string | null;
  created_at_unix: number;
  started_at_unix?: number | null;
  completed_at_unix?: number | null;
  expires_at_unix: number;
  evidence?: VNextDebugJobEvidence;
}

export interface VNextDebugJobEvidence {
  schema_version: number;
  verdict: string;
  owner_summary: string;
  phases: VNextDebugJobEvidencePhase[];
}

export interface VNextDebugJobEvidencePhase {
  id: string;
  label: string;
  status: string;
  summary: string;
}

export interface VNextDebugJobsResponse {
  schema_version: number;
  jobs: VNextDebugJob[];
}

export interface VNextDebugJobResponse {
  schema_version: number;
  job: VNextDebugJob;
  owner_message?: string;
}

export interface ContextualAssistantResponse {
  schema_version: number;
  turn_id?: string | null;
  response_kind: string;
  visible_answer: string;
  severity?: NoticeSeverity | null;
  draft?: string | null;
  action_draft?: unknown;
  suggested_intent?: {
    kind: string;
    label: string;
    summary: string;
    confidence: number;
    next_action_kind: string;
  } | null;
  authority?: AuthorityEnvelope | null;
  required_gate?: OwnerWorkContextGate | null;
  /** Server-measured turn duration in ms (owner 2026-09-12). */
  latency_ms?: number;
  receipt_expectation: string;
  citations: string[];
  source_evidence?: ContextualAssistantSourceEvidence[];
  evidence?: ContextualAssistantEvidenceItem[];
  unsupported_numbers?: string[];
  warnings: string[];
  trace_id?: string | null;
  run_id?: string | null;
  provider_request_id?: string | null;
}

export interface JevProvenanceEvidence {
  verdict: string;
  owner_decision?: string | null;
  approval_id?: string | null;
  receipt?: string | null;
  audit_tip?: string | null;
  risk?: string | null;
  model?: string | null;
  reason?: string | null;
}

export interface ContextualAssistantSourceEvidence {
  kind: string;
  label: string;
  summary: string;
  detail: string;
  /** Present only when this tool actually passed the Jev policy broker. */
  jev?: JevProvenanceEvidence | null;
}

export interface ContextualAssistantEvidenceItem {
  id: string;
  tool: string;
  label: string;
  excerpt: string;
  full_len: number;
}

export async function fetchOwnerWorkQueue(initData: string): Promise<OwnerWorkQueue> {
  const res = await fetch(`${API_BASE}/webapp/owner-work/queue`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Queue load failed');
  const inbox = readOwnerWorkQueue(json);
  if (!inbox) throw new ApiResponseError('Queue load failed: unexpected response', res.status);
  return inbox;
}

export async function fetchVNextWork(initData: string): Promise<VNextWorkProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/work`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Work load failed');
  const work = readVNextWorkProjection(json);
  if (!work) throw new ApiResponseError('Work load failed: unexpected response', res.status);
  return work;
}

export async function fetchVNextItem(initData: string, itemId: string): Promise<VNextItemProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/work/${encodeURIComponent(itemId)}`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Item load failed');
  const item = readVNextItemProjection(json);
  if (!item) throw new ApiResponseError('Item load failed: unexpected response', res.status);
  return item;
}

export async function fetchVNextStatus(initData: string): Promise<VNextStatusProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/status`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Status load failed');
  const status = readVNextStatusProjection(json);
  if (!status) throw new ApiResponseError('Status load failed: unexpected response', res.status);
  return status;
}

export async function fetchVNextModelCatalog(initData: string): Promise<VNextModelCatalog> {
  const res = await fetch(`${API_BASE}/webapp/vnext/model-catalog`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Model catalog load failed');
  const catalog = readVNextModelCatalog(json);
  if (!catalog) throw new ApiResponseError('Model catalog load failed: unexpected response', res.status);
  return catalog;
}

export async function fetchVNextSettings(initData: string): Promise<VNextSettingsProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Settings load failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Settings load failed: unexpected response', res.status);
  return settings;
}

function readVNextWebPushStatus(value: unknown): VNextWebPushStatus | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.schema_version !== 'number') return null;
  if (typeof record.configured !== 'boolean') return null;
  if (typeof record.send_configured !== 'boolean') return null;
  if (typeof record.subscribed !== 'boolean') return null;
  if (typeof record.subscription_count !== 'number') return null;
  if (record.public_key !== undefined && record.public_key !== null && typeof record.public_key !== 'string') return null;
  if (record.last_delivery !== undefined && record.last_delivery !== null && !readVNextWebPushDeliverySummary(record.last_delivery)) return null;
  if (typeof record.owner_message !== 'string') return null;
  if (record.owner_next_step !== undefined && record.owner_next_step !== null && typeof record.owner_next_step !== 'string') return null;
  return value as VNextWebPushStatus;
}

function readVNextWebPushDeliverySummary(value: unknown): VNextWebPushDeliverySummary | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== 'string') return null;
  if (typeof record.status !== 'string') return null;
  if (typeof record.delivered !== 'number') return null;
  if (typeof record.attempted !== 'number') return null;
  if (typeof record.removed !== 'number') return null;
  if (typeof record.created_at_unix !== 'number') return null;
  return value as VNextWebPushDeliverySummary;
}

function readVNextWebPushTestResponse(value: unknown): VNextWebPushTestResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.schema_version !== 'number') return null;
  if (typeof record.status !== 'string') return null;
  if (typeof record.attempted !== 'number') return null;
  if (typeof record.delivered !== 'number') return null;
  if (typeof record.failed !== 'number') return null;
  if (typeof record.removed !== 'number') return null;
  if (typeof record.receipt !== 'string') return null;
  if (typeof record.owner_message !== 'string') return null;
  return value as VNextWebPushTestResponse;
}

export async function fetchVNextWebPushStatus(initData: string): Promise<VNextWebPushStatus> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/web-push/status`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'WebApp notification status failed');
  const status = readVNextWebPushStatus(json);
  if (!status) throw new ApiResponseError('WebApp notification status failed: unexpected response', res.status);
  return status;
}

export async function subscribeVNextWebPush(
  initData: string,
  csrfToken: string | undefined,
  subscription: PushSubscription,
): Promise<VNextWebPushStatus> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/web-push/subscribe`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      subscription: subscription.toJSON(),
      user_agent: navigator.userAgent,
    }),
  });

  const json = await expectJson(res, 'WebApp notification setup failed');
  const status = readVNextWebPushStatus(json);
  if (!status) throw new ApiResponseError('WebApp notification setup failed: unexpected response', res.status);
  return status;
}

export async function unsubscribeVNextWebPush(
  initData: string,
  csrfToken: string | undefined,
  endpoint: string,
): Promise<VNextWebPushStatus> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/web-push/unsubscribe`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({ endpoint }),
  });

  const json = await expectJson(res, 'WebApp notification unsubscribe failed');
  const status = readVNextWebPushStatus(json);
  if (!status) throw new ApiResponseError('WebApp notification unsubscribe failed: unexpected response', res.status);
  return status;
}

export async function testVNextWebPush(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextWebPushTestResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/web-push/test`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({ schema_version: 1 }),
  });

  const json = await expectJson(res, 'WebApp notification test failed');
  const result = readVNextWebPushTestResponse(json);
  if (!result) throw new ApiResponseError('WebApp notification test failed: unexpected response', res.status);
  return result;
}

function readVNextGrokStatus(value: unknown): VNextGrokStatus | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.provider !== 'xai_subscription') return null;
  if (typeof record.enabled !== 'boolean') return null;
  if (typeof record.selected_provider !== 'string') return null;
  if (typeof record.active_for_chat !== 'boolean') return null;
  if (typeof record.connected !== 'boolean') return null;
  if (typeof record.pending_device_code !== 'boolean') return null;
  if (typeof record.model !== 'string') return null;
  if (typeof record.foreground_only !== 'boolean') return null;
  if (typeof record.daily_request_limit !== 'number') return null;
  if (typeof record.owner_message !== 'string') return null;
  return value as VNextGrokStatus;
}

function readVNextCodexStatus(value: unknown): VNextCodexStatus | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.provider !== 'codex_oauth') return null;
  if (typeof record.enabled !== 'boolean') return null;
  if (typeof record.selected_provider !== 'string') return null;
  if (typeof record.active_for_chat !== 'boolean') return null;
  if (typeof record.connected !== 'boolean') return null;
  if (typeof record.binary_available !== 'boolean') return null;
  if (typeof record.model !== 'string') return null;
  if (typeof record.reasoning_effort !== 'string') return null;
  if (!Array.isArray(record.model_options)) return null;
  for (const option of record.model_options) {
    if (!option || typeof option !== 'object' || Array.isArray(option)) return null;
    const optionRecord = option as Record<string, unknown>;
    if (typeof optionRecord.slug !== 'string') return null;
    if (typeof optionRecord.display_name !== 'string') return null;
    if (typeof optionRecord.default_reasoning_level !== 'string') return null;
    if (!Array.isArray(optionRecord.supported_reasoning_levels)) return null;
    if (!optionRecord.supported_reasoning_levels.every((level) => typeof level === 'string')) return null;
  }
  if (typeof record.model_options_source !== 'string') return null;
  if (typeof record.tool_calls_supported !== 'boolean') return null;
  if (typeof record.tool_routing_policy !== 'string') return null;
  if (typeof record.foreground_only !== 'boolean') return null;
  if (typeof record.daily_request_limit !== 'number') return null;
  if (typeof record.cli_binary !== 'string') return null;
  if (typeof record.cli_home !== 'string') return null;
  if (typeof record.cli_workdir !== 'string') return null;
  if (typeof record.owner_message !== 'string') return null;
  return value as VNextCodexStatus;
}

function readVNextGrokDeviceCodeResponse(value: unknown): VNextGrokDeviceCodeResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.status !== 'pending') return null;
  if (typeof record.verification_uri !== 'string') return null;
  if (typeof record.user_code !== 'string') return null;
  if (typeof record.expires_at_ms !== 'number') return null;
  if (typeof record.interval_secs !== 'number') return null;
  if (typeof record.message !== 'string') return null;
  return value as VNextGrokDeviceCodeResponse;
}

function readVNextCodexDeviceCodeResponse(value: unknown): VNextCodexDeviceCodeResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.status !== 'pending') return null;
  if (typeof record.verification_uri !== 'string') return null;
  if (typeof record.user_code !== 'string') return null;
  if (typeof record.expires_at_ms !== 'number') return null;
  if (typeof record.interval_secs !== 'number') return null;
  if (typeof record.message !== 'string') return null;
  return value as VNextCodexDeviceCodeResponse;
}

function readVNextCodexPollResponse(value: unknown): VNextCodexPollResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.status !== 'pending' && record.status !== 'connected') return null;
  if (typeof record.message !== 'string') return null;
  return value as VNextCodexPollResponse;
}

function readVNextGrokPollResponse(value: unknown): VNextGrokPollResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.status !== 'pending' && record.status !== 'connected' && record.status !== 'revoked') return null;
  if (typeof record.message !== 'string') return null;
  return value as VNextGrokPollResponse;
}

export async function fetchVNextGrokStatus(initData: string): Promise<VNextGrokStatus> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/grok/status`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });
  const json = await expectJson(res, 'Grok status load failed');
  const status = readVNextGrokStatus(json);
  if (!status) throw new ApiResponseError('Grok status load failed: unexpected response', res.status);
  return status;
}

export async function fetchVNextCodexStatus(initData: string): Promise<VNextCodexStatus> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/codex/status`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });
  const json = await expectJson(res, 'Codex status load failed');
  const status = readVNextCodexStatus(json);
  if (!status) throw new ApiResponseError('Codex status load failed: unexpected response', res.status);
  return status;
}

export async function startVNextCodexDeviceCode(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextCodexDeviceCodeResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/codex/start-device-code`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-codex-start-device-code'),
    }),
  });
  const json = await expectJson(res, 'Codex sign-in failed');
  const response = readVNextCodexDeviceCodeResponse(json);
  if (!response) throw new ApiResponseError('Codex sign-in failed: unexpected response', res.status);
  return response;
}

export async function pollVNextCodexDeviceCode(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextCodexPollResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/codex/poll-device-code`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-codex-poll-device-code'),
    }),
  });
  const json = await expectJson(res, 'Codex sign-in check failed');
  const response = readVNextCodexPollResponse(json);
  if (!response) throw new ApiResponseError('Codex sign-in check failed: unexpected response', res.status);
  return response;
}

export async function startVNextGrokDeviceCode(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextGrokDeviceCodeResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/grok/start-device-code`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-grok-start-device-code'),
    }),
  });
  const json = await expectJson(res, 'Grok sign-in failed');
  const response = readVNextGrokDeviceCodeResponse(json);
  if (!response) throw new ApiResponseError('Grok sign-in failed: unexpected response', res.status);
  return response;
}

export async function pollVNextGrokDeviceCode(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextGrokPollResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/grok/poll-device-code`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-grok-poll-device-code'),
    }),
  });
  const json = await expectJson(res, 'Grok sign-in check failed');
  const response = readVNextGrokPollResponse(json);
  if (!response) throw new ApiResponseError('Grok sign-in check failed: unexpected response', res.status);
  return response;
}

export async function revokeVNextGrok(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextGrokPollResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/grok/revoke`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-grok-revoke'),
    }),
  });
  const json = await expectJson(res, 'Grok revoke failed');
  const response = readVNextGrokPollResponse(json);
  if (!response) throw new ApiResponseError('Grok revoke failed: unexpected response', res.status);
  return response;
}

export async function useVNextGrokForChat(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextGrokPollResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/grok/use-for-chat`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-grok-use-for-chat'),
    }),
  });
  const json = await expectJson(res, 'Grok routing failed');
  const response = readVNextGrokPollResponse(json);
  if (!response) throw new ApiResponseError('Grok routing failed: unexpected response', res.status);
  return response;
}

export async function stopUsingVNextGrokForChat(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextGrokPollResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/grok/stop-using-for-chat`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-grok-stop-using-for-chat'),
    }),
  });
  const json = await expectJson(res, 'Grok routing update failed');
  const response = readVNextGrokPollResponse(json);
  if (!response) throw new ApiResponseError('Grok routing update failed: unexpected response', res.status);
  return response;
}

export async function useVNextCodexForChat(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextCodexPollResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/codex/use-for-chat`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-codex-use-for-chat'),
    }),
  });
  const json = await expectJson(res, 'Codex routing failed');
  const response = readVNextCodexPollResponse(json);
  if (!response) throw new ApiResponseError('Codex routing failed: unexpected response', res.status);
  return response;
}

export async function setVNextCodexModel(
  initData: string,
  csrfToken: string | undefined,
  model: string,
  reasoningEffort: string,
): Promise<VNextCodexPollResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/codex/model`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey(`vnext-codex-model-${model}-${reasoningEffort}`),
      model,
      reasoning_effort: reasoningEffort,
    }),
  });
  const json = await expectJson(res, 'Codex model update failed');
  const response = readVNextCodexPollResponse(json);
  if (!response) throw new ApiResponseError('Codex model update failed: unexpected response', res.status);
  return response;
}

export async function stopUsingVNextCodexForChat(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextCodexPollResponse> {
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/codex/stop-using-for-chat`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      idempotency_key: idempotencyKey('vnext-codex-stop-using-for-chat'),
    }),
  });
  const json = await expectJson(res, 'Codex routing update failed');
  const response = readVNextCodexPollResponse(json);
  if (!response) throw new ApiResponseError('Codex routing update failed: unexpected response', res.status);
  return response;
}

export async function fetchVNextChatHistory(
  initData: string,
  options: { before?: number; turn?: string } = {},
): Promise<VNextChatHistoryProjection> {
  const params = new URLSearchParams();
  if (options.before && Number.isFinite(options.before)) params.set('before', String(Math.floor(options.before)));
  const turn = options.turn?.trim();
  if (turn) params.set('turn', turn);
  const encoded = params.toString();
  const query = encoded ? `?${encoded}` : '';
  const res = await fetch(`${API_BASE}/webapp/vnext/chat-history${query}`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Chat history load failed');
  const history = readVNextChatHistoryProjection(json);
  if (!history) throw new ApiResponseError('Chat history load failed: unexpected response', res.status);
  return history;
}

export async function fetchVNextAvatarProfile(initData: string): Promise<VNextAvatarProfileProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/avatar-profile`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Avatar profile load failed');
  const profile = readVNextAvatarProfileProjection(json);
  if (!profile) throw new ApiResponseError('Avatar profile load failed: unexpected response', res.status);
  return profile;
}

export async function updateVNextAvatarProfile(
  initData: string,
  csrfToken: string | undefined,
  profile: { aegis_avatar_url: string; owner_avatar_url: string },
): Promise<VNextAvatarProfileProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    ...profile,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/avatar-profile`, {
    method: 'PUT',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Avatar profile save failed');
  const saved = readVNextAvatarProfileProjection(json);
  if (!saved) throw new ApiResponseError('Avatar profile save failed: unexpected response', res.status);
  return saved;
}

export async function updateVNextAvatarAutoRefresh(
  initData: string,
  csrfToken: string | undefined,
  enabled: boolean,
  promptHint: string,
): Promise<VNextAvatarProfileProjection> {
  const res = await fetch(`${API_BASE}/webapp/vnext/avatar-profile/auto-refresh`, {
    method: 'PUT',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      enabled,
      prompt_hint: promptHint,
    }),
  });

  const json = await expectJson(res, 'Avatar auto-refresh save failed');
  const saved = readVNextAvatarProfileProjection(json);
  if (!saved) throw new ApiResponseError('Avatar auto-refresh save failed: unexpected response', res.status);
  return saved;
}

export async function generateVNextAegisAvatar(
  initData: string,
  csrfToken: string | undefined,
  promptHint: string,
): Promise<VNextAegisAvatarGenerateResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/avatar-profile/generate-aegis`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      prompt_hint: promptHint,
    }),
  });

  const json = await expectJson(res, 'Aegis avatar generation failed');
  if (!json || typeof json !== 'object' || Array.isArray(json)) {
    throw new ApiResponseError('Aegis avatar generation failed: unexpected response', res.status);
  }
  const record = json as Record<string, unknown>;
  const profile = readVNextAvatarProfileProjection(record.profile);
  if (record.schema_version !== 1 || typeof record.status !== 'string' || typeof record.owner_message !== 'string' || typeof record.provider !== 'string' || !profile) {
    throw new ApiResponseError('Aegis avatar generation failed: unexpected response', res.status);
  }
  return {
    schema_version: 1,
    status: record.status,
    owner_message: record.owner_message,
    provider: record.provider,
    provider_request_id: typeof record.provider_request_id === 'string' ? record.provider_request_id : null,
    profile,
  };
}

export async function pauseVNextCostSaving(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey('vnext-cost-saving-pause'),
  });
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/cost-saving/pause`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Cost controls update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Cost controls update failed: unexpected response', res.status);
  return settings;
}

export async function resumeVNextCostSaving(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey('vnext-cost-saving-resume'),
  });
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/cost-saving/resume`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Cost controls update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Cost controls update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists the owner's display language (`'zh'` | `'en'`) via the dedicated
 * webapp settings endpoint, so server-authored Channel-B strings follow the
 * same setting the webapp chrome uses. Returns the refreshed settings
 * projection (its `display_language` reflects the new value).
 */
export async function setDisplayLanguage(
  initData: string,
  csrfToken: string | undefined,
  value: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-display-language-${value}`),
    display_language: value,
  });
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/settings/display-language`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Language update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Language update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists the owner's chosen LLM provider (`'openrouter'` | `'codex_oauth'` |
 * `'xai_subscription'`) via the dedicated webapp settings endpoint. Mirrors
 * {@link setDisplayLanguage}: same console-auth + CSRF, returns the refreshed
 * settings projection (its `llm_provider` reflects the new value).
 */
export async function setLlmProvider(
  initData: string,
  csrfToken: string | undefined,
  value: string,
  model?: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-llm-provider-${value}`),
    llm_provider: value,
    ...(model !== undefined ? { llm_model: model } : {}),
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/llm-provider`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Provider update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Provider update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists the owner's chosen LLM model id from the current provider catalog
 * via the dedicated webapp settings endpoint. Mirrors {@link setLlmProvider}:
 * same console-auth + CSRF, returns the refreshed settings projection.
 */
export async function setLlmModel(
  initData: string,
  csrfToken: string | undefined,
  model: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-llm-model-${model}`),
    llm_model: model,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/llm-model`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Model update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Model update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists the backup model used when the primary provider/model fails.
 * Empty `model` clears the backup. Mirrors {@link setLlmModel}.
 */
export async function setFallbackModel(
  initData: string,
  csrfToken: string | undefined,
  model: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-fallback-model-${model || 'none'}`),
    model,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/fallback-model`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Backup model update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Backup model update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists whether the owner wants to fall back to the paid metered API when
 * the configured provider is unavailable. Mirrors {@link setDisplayLanguage}:
 * same console-auth + CSRF, returns the refreshed settings projection (its
 * `paid_api_fallback_enabled` reflects the new value).
 */
/**
 * Persists the owner's web search provider (`'xai_subscription'` | `'openrouter'`).
 * Switching provider rewrites `web_search_model` into that provider's id shape
 * and does not change the saved chat model.
 */
export async function setWebSearchProvider(
  initData: string,
  csrfToken: string | undefined,
  provider: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-web-search-provider-${provider}`),
    web_search_provider: provider,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/web-search`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });
  const json = await expectJson(res, 'Web search update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Web search update failed: unexpected response', res.status);
  return settings;
}

/**
 * Persists the owner's web search model for the current search provider.
 */
export async function setWebSearchModel(
  initData: string,
  csrfToken: string | undefined,
  model: string,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-web-search-model-${model}`),
    web_search_model: model,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/web-search`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });
  const json = await expectJson(res, 'Web search update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Web search update failed: unexpected response', res.status);
  return settings;
}

export async function setPaidApiFallback(
  initData: string,
  csrfToken: string | undefined,
  enabled: boolean,
): Promise<VNextSettingsProjection> {
  const body = JSON.stringify({
    schema_version: 1,
    idempotency_key: idempotencyKey(`vnext-paid-api-fallback-${enabled ? 'on' : 'off'}`),
    enabled,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/settings/paid-api-fallback`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Provider update failed');
  const settings = readVNextSettingsProjection(json);
  if (!settings) throw new ApiResponseError('Provider update failed: unexpected response', res.status);
  return settings;
}

export interface JevStatus {
  enabled: boolean;
  key_present: boolean;
  saved_key: boolean;
  source: 'settings' | 'environment';
  key_source: 'settings' | 'environment' | 'none';
  verified: boolean;
  checked_at?: number | null;
}
export interface JevUpdate { enabled: boolean; consent: boolean; api_key?: string; remove_key?: boolean }
export async function requestJevSettings(initData: string, csrfToken?: string, update?: JevUpdate): Promise<JevStatus> {
  const response = await fetch(`${API_BASE}/webapp/vnext/settings/jev`, {
    method: update ? 'POST' : 'GET', credentials: 'include',
    headers: authHeaders(initData, csrfToken),
    ...(update ? { body: JSON.stringify(update) } : {}),
  });
  // Keep secret-bearing request details and arbitrary upstream error text out of UI.
  if (!response.ok) {
    if (response.status === 422) throw new Error(await response.text());
    throw new Error('jev_settings_failed');
  }
  const result = await response.json();
  if (typeof result.enabled !== 'boolean' || typeof result.key_present !== 'boolean'
    || typeof result.saved_key !== 'boolean' || typeof result.verified !== 'boolean'
    || !['settings', 'environment'].includes(result.source)
    || !['settings', 'environment', 'none'].includes(result.key_source)) throw new Error('jev_settings_failed');
  return result as JevStatus;
}

export async function markVNextPresence(
  initData: string,
  csrfToken: string | undefined,
  surface = 'web',
): Promise<void> {
  await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/presence`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      surface,
      idempotency_key: idempotencyKey('vnext-presence'),
    }),
  });
}

export async function submitVNextCommand(
  initData: string,
  csrfToken: string | undefined,
  input: string,
  clientContext: 'home' | 'work' | 'item' | 'status' | 'settings',
  focusItemId?: string | null,
  mode: 'text' | 'voice' = 'text',
  intentKind: 'chat' | 'explain' | 'draft' = 'explain',
  commandIdempotencyKey?: string,
  conversationContext: VNextConversationContextTurn[] = [],
  attachments: VNextCommandAttachment[] = [],
): Promise<VNextCommandResponse> {
  const body = JSON.stringify({
    schema_version: 1,
    input,
    mode,
    intent_kind: intentKind,
    focus_item_id: focusItemId,
    client_context: clientContext,
    idempotency_key: commandIdempotencyKey || idempotencyKey('vnext-command'),
    ...(conversationContext.length > 0 ? { conversation_context: conversationContext } : {}),
    ...(attachments.length > 0 ? { attachments } : {}),
  });
  const res = await fetchAssistantCommandWithTransientRetry(`${API_BASE}/webapp/vnext/command`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Command failed');
  const response = readVNextCommandResponse(json);
  if (!response) throw new ApiResponseError('Command failed: unexpected response', res.status);
  return response;
}

export type VNextCommandStatusResult =
  | { kind: 'pending' }
  | { kind: 'gone' }
  | { kind: 'done'; response: VNextCommandResponse };

/**
 * Cheap poll for a command that is still running server-side (the command
 * route answers 504 after its foreground wait, and Cloudflare cuts anything
 * over 100 s with a 524). 202 = still working, 404 = unknown/expired,
 * 200 = the same final envelope the command route would have returned.
 */
export async function fetchVNextCommandStatus(
  initData: string,
  csrfToken: string | undefined,
  params: { focusId: string; intentKind: string; idempotencyKey: string },
): Promise<VNextCommandStatusResult> {
  const query = new URLSearchParams({
    focus_id: params.focusId,
    intent_kind: params.intentKind,
    idempotency_key: params.idempotencyKey,
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/command/status?${query.toString()}`, {
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });
  if (res.status === 202) return { kind: 'pending' };
  if (res.status === 404) return { kind: 'gone' };
  const json = await expectJson(res, 'Command status failed');
  const response = readVNextCommandResponse(json);
  if (!response) throw new ApiResponseError('Command status failed: unexpected response', res.status);
  return { kind: 'done', response };
}

export type VNextVoiceSession = {
  schema_version: number;
  client_secret: string;
  expires_in_seconds: number;
  model: string;
  ws_url: string;
  max_session_seconds: number;
};

export type VNextVoiceStatus = {
  schema_version: number;
  realtime_available: boolean;
  model: string;
  max_session_seconds: number;
  /** xai-subscription | grok-cli | api-key | stub */
  key_source?: string | null;
  subscription_connected?: boolean;
  /** Connected in Settings but access token expired / not mint-ready. */
  subscription_stale?: boolean;
};

/** Whether Realtime mint is configured (subscription / CLI preferred). */
export async function fetchVNextVoiceStatus(initData = ''): Promise<VNextVoiceStatus> {
  const res = await fetch(`${API_BASE}/webapp/vnext/voice/status`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });
  const json = await expectJson(res, 'Voice status failed');
  const record = json && typeof json === 'object' ? json as Record<string, unknown> : null;
  if (!record || typeof record.realtime_available !== 'boolean') {
    throw new ApiResponseError('Voice status failed: unexpected response', res.status);
  }
  return {
    schema_version: typeof record.schema_version === 'number' ? record.schema_version : 1,
    realtime_available: record.realtime_available,
    model: typeof record.model === 'string' ? record.model : 'grok-voice-think-fast-2.0',
    max_session_seconds: typeof record.max_session_seconds === 'number' ? record.max_session_seconds : 600,
    key_source: typeof record.key_source === 'string' ? record.key_source : null,
    subscription_connected: typeof record.subscription_connected === 'boolean'
      ? record.subscription_connected
      : undefined,
    subscription_stale: typeof record.subscription_stale === 'boolean'
      ? record.subscription_stale
      : undefined,
  };
}

/** Mint ephemeral Realtime client secret (auth + CSRF). */
export async function mintVNextVoiceSession(
  initData: string,
  csrfToken: string | undefined,
): Promise<VNextVoiceSession> {
  const res = await fetch(`${API_BASE}/webapp/vnext/voice/session`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: '{}',
  });
  const json = await expectJson(res, 'Voice session failed');
  const record = json && typeof json === 'object' ? json as Record<string, unknown> : null;
  if (
    !record
    || typeof record.client_secret !== 'string'
    || typeof record.model !== 'string'
    || typeof record.ws_url !== 'string'
  ) {
    throw new ApiResponseError('Voice session failed: unexpected response', res.status);
  }
  return {
    schema_version: typeof record.schema_version === 'number' ? record.schema_version : 1,
    client_secret: record.client_secret,
    expires_in_seconds: typeof record.expires_in_seconds === 'number' ? record.expires_in_seconds : 300,
    model: record.model,
    ws_url: record.ws_url,
    max_session_seconds: typeof record.max_session_seconds === 'number' ? record.max_session_seconds : 600,
  };
}

export interface VNextUploadResult {
  type: 'image' | 'document' | 'voice';
  /** Base64 payload. Absent for `voice`: the audio stays on the server, which
   *  transcribes it — only the transcript rides the later command. */
  data_url?: string;
  filename: string;
  mime_type: string;
  /** `voice` only: the server-side speech-to-text result. */
  transcript?: string;
  /** `voice` only: probed duration in seconds, or `null` when ffprobe could
   *  not read one. */
  duration_secs?: number | null;
}

function readVNextUploadResult(value: unknown): VNextUploadResult | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.filename !== 'string') return null;
  if (typeof record.mime_type !== 'string') return null;
  if (record.type === 'voice') {
    // A voice upload is only useful if it carries transcribed speech; an empty
    // transcript is a server contract violation (the endpoint answers 422
    // `voice_empty` instead), so refuse it rather than attach a blank note.
    if (typeof record.transcript !== 'string' || !record.transcript.trim()) return null;
    return {
      type: 'voice',
      filename: record.filename,
      mime_type: record.mime_type,
      transcript: record.transcript,
      duration_secs: typeof record.duration_secs === 'number' ? record.duration_secs : null,
    };
  }
  if (record.type !== 'image' && record.type !== 'document') return null;
  if (typeof record.data_url !== 'string' || !record.data_url) return null;
  return {
    data_url: record.data_url,
    type: record.type,
    filename: record.filename,
    mime_type: record.mime_type,
  };
}

/**
 * Upload one multimodal file to the webapp upload endpoint.
 *
 * Images/PDFs come back as a base64 `data_url` descriptor. A recorded voice
 * note (`audio/webm|ogg|mp4|wav|mpeg`) is transcribed ON THE SERVER and comes
 * back as `{ type: 'voice', transcript, duration_secs }` — the audio bytes
 * never round-trip. The server validates MIME by header + magic number and
 * caps size per kind. Sends `multipart/form-data` — do NOT set Content-Type
 * explicitly (the browser sets the boundary); we only forward auth + CSRF.
 */
export async function uploadVNextAttachment(
  initData: string,
  csrfToken: string | undefined,
  file: File,
): Promise<VNextUploadResult> {
  const form = new FormData();
  form.append('file', file, file.name);
  const headers: Record<string, string> = {};
  if (initData.trim()) headers.Authorization = `TelegramWebApp ${initData}`;
  if (csrfToken) headers['X-Aegis-CSRF'] = csrfToken;
  const res = await fetch(`${API_BASE}/webapp/vnext/upload`, {
    method: 'POST',
    headers,
    credentials: 'include',
    body: form,
  });
  const json = await expectJson(res, 'Upload failed');
  const result = readVNextUploadResult(json);
  if (!result) throw new ApiResponseError('Upload failed: unexpected response', res.status);
  return result;
}

export async function submitVNextWorkAction(
  initData: string,
  csrfToken: string | undefined,
  itemId: string,
  actionId: string,
): Promise<VNextWorkActionResponse> {
  const body = JSON.stringify({
    schema_version: 1,
    action_id: actionId,
    idempotency_key: idempotencyKey(`vnext-action:${itemId}:${actionId}`),
  });
  const res = await fetchJsonWithOneNetworkRetry(`${API_BASE}/webapp/vnext/work/${encodeURIComponent(itemId)}/actions`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Action failed');
  const response = readVNextWorkActionResponse(json);
  if (!response) throw new ApiResponseError('Action failed: unexpected response', res.status);
  return response;
}

export async function submitVNextDebugSnapshot(
  initData: string,
  csrfToken: string | undefined,
  surface: string,
  path: string,
  payload: unknown,
): Promise<VNextDebugSnapshotResponse> {
  const body = JSON.stringify({
    schema_version: 1,
    surface,
    path,
    payload,
    idempotency_key: idempotencyKey('vnext-debug-snapshot'),
  });
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-snapshots`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body,
  });

  const json = await expectJson(res, 'Snapshot failed');
  const response = readVNextDebugSnapshotResponse(json);
  if (!response) throw new ApiResponseError('Snapshot failed: unexpected response', res.status);
  return response;
}

function readVNextDebugLease(value: unknown): VNextDebugLease | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== 'string') return null;
  if (typeof record.owner !== 'string') return null;
  if (typeof record.principal_type !== 'string') return null;
  if (typeof record.scope !== 'string') return null;
  if (!Array.isArray(record.permissions) || !record.permissions.every((permission) => typeof permission === 'string')) return null;
  if (typeof record.reason !== 'string') return null;
  if (typeof record.created_at_unix !== 'number') return null;
  if (typeof record.expires_at_unix !== 'number') return null;
  return value as VNextDebugLease;
}

function readVNextDebugLeaseResponse(value: unknown): VNextDebugLeaseResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  if (typeof record.owner_message !== 'string') return null;
  const lease = readVNextDebugLease(record.lease);
  if (!lease) return null;
  return {
    schema_version: 1,
    owner_message: record.owner_message,
    lease,
    debug_grant_token: typeof record.debug_grant_token === 'string' ? record.debug_grant_token : null,
  };
}

function readVNextDebugLeasesResponse(value: unknown): VNextDebugLeasesResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  if (!Array.isArray(record.leases)) return null;
  const leases = record.leases.map(readVNextDebugLease);
  if (leases.some((lease) => !lease)) return null;
  return {
    schema_version: 1,
    leases: leases as VNextDebugLease[],
  };
}

function readVNextDebugLeaseRevokeResponse(value: unknown): VNextDebugLeaseRevokeResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  if (typeof record.revoked !== 'number') return null;
  return { schema_version: 1, revoked: record.revoked };
}

function readVNextDebugJobEvidencePhase(value: unknown): VNextDebugJobEvidencePhase | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== 'string') return null;
  if (typeof record.label !== 'string') return null;
  if (typeof record.status !== 'string') return null;
  if (typeof record.summary !== 'string') return null;
  return record as unknown as VNextDebugJobEvidencePhase;
}

function readVNextDebugJobEvidence(value: unknown): VNextDebugJobEvidence | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  if (typeof record.verdict !== 'string') return null;
  if (typeof record.owner_summary !== 'string') return null;
  if (!Array.isArray(record.phases)) return null;
  const phases = record.phases.map(readVNextDebugJobEvidencePhase);
  if (phases.some((phase) => !phase)) return null;
  return {
    schema_version: 1,
    verdict: record.verdict,
    owner_summary: record.owner_summary,
    phases: phases as VNextDebugJobEvidencePhase[],
  };
}

function readVNextDebugJob(value: unknown): VNextDebugJob | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== 'string') return null;
  if (typeof record.owner !== 'string') return null;
  if (typeof record.principal_type !== 'string') return null;
  if (typeof record.kind !== 'string') return null;
  if (typeof record.status !== 'string') return null;
  if (typeof record.summary !== 'string') return null;
  if (typeof record.log_text !== 'string') return null;
  if (record.exit_code !== null && record.exit_code !== undefined && typeof record.exit_code !== 'number') return null;
  if (record.lease_id !== null && record.lease_id !== undefined && typeof record.lease_id !== 'string') return null;
  if (typeof record.created_at_unix !== 'number') return null;
  if (record.started_at_unix !== null && record.started_at_unix !== undefined && typeof record.started_at_unix !== 'number') return null;
  if (record.completed_at_unix !== null && record.completed_at_unix !== undefined && typeof record.completed_at_unix !== 'number') return null;
  if (typeof record.expires_at_unix !== 'number') return null;
  let evidence: VNextDebugJobEvidence | undefined;
  if (record.evidence !== undefined) {
    const parsedEvidence = readVNextDebugJobEvidence(record.evidence);
    if (!parsedEvidence) return null;
    evidence = parsedEvidence;
  }
  return {
    ...(record as unknown as VNextDebugJob),
    evidence,
  };
}

function readVNextDebugJobsResponse(value: unknown): VNextDebugJobsResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  if (!Array.isArray(record.jobs)) return null;
  const jobs = record.jobs.map(readVNextDebugJob);
  if (jobs.some((job) => !job)) return null;
  return {
    schema_version: 1,
    jobs: jobs as VNextDebugJob[],
  };
}

function readVNextDebugJobResponse(value: unknown): VNextDebugJobResponse | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (record.schema_version !== 1) return null;
  const job = readVNextDebugJob(record.job);
  if (!job) return null;
  if (record.owner_message !== null && record.owner_message !== undefined && typeof record.owner_message !== 'string') return null;
  return {
    schema_version: 1,
    job,
    owner_message: typeof record.owner_message === 'string' ? record.owner_message : undefined,
  };
}

export async function createVNextDebugLease(
  initData: string,
  csrfToken: string | undefined,
  scope: 'avatar' | 'reminders' | 'work_item' | 'chat' | 'settings' | 'notifications' | 'provider' | 'status' | 'general',
  reason: string,
  permissions: Array<'inspect' | 'replay_safe' | 'replay_mutating'> = ['inspect'],
  durationMinutes = 15,
  itemId?: string | null,
): Promise<VNextDebugLeaseResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-leases`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      scope,
      permissions,
      duration_minutes: durationMinutes,
      ...(itemId ? { item_id: itemId } : {}),
      reason,
      idempotency_key: idempotencyKey(`vnext-debug-lease:${scope}`),
    }),
  });

  const json = await expectJson(res, 'Debug access failed');
  const response = readVNextDebugLeaseResponse(json);
  if (!response) throw new ApiResponseError('Debug access failed: unexpected response', res.status);
  return response;
}

export async function fetchVNextDebugLeases(initData: string): Promise<VNextDebugLeasesResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-leases`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Debug access list failed');
  const response = readVNextDebugLeasesResponse(json);
  if (!response) throw new ApiResponseError('Debug access list failed: unexpected response', res.status);
  return response;
}

export async function revokeVNextDebugLease(
  initData: string,
  csrfToken: string | undefined,
  leaseId: string,
): Promise<VNextDebugLeaseRevokeResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-leases/${encodeURIComponent(leaseId)}`, {
    method: 'DELETE',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Debug access revoke failed');
  const response = readVNextDebugLeaseRevokeResponse(json);
  if (!response) throw new ApiResponseError('Debug access revoke failed: unexpected response', res.status);
  return response;
}

export async function fetchVNextDebugJobs(initData: string): Promise<VNextDebugJobsResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-jobs`, {
    headers: authHeaders(initData),
    credentials: 'include',
  });

  const json = await expectJson(res, 'Debug jobs load failed');
  const response = readVNextDebugJobsResponse(json);
  if (!response) throw new ApiResponseError('Debug jobs load failed: unexpected response', res.status);
  return response;
}

export async function createVNextDebugJob(
  initData: string,
  csrfToken: string | undefined,
  kind: 'webapp_e2e_suite',
  reason: string,
): Promise<VNextDebugJobResponse> {
  const res = await fetch(`${API_BASE}/webapp/vnext/debug-jobs`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      schema_version: 1,
      kind,
      reason,
      idempotency_key: idempotencyKey(`vnext-debug-job:${kind}`),
    }),
  });

  const json = await expectJson(res, 'Debug job start failed');
  const response = readVNextDebugJobResponse(json);
  if (!response) throw new ApiResponseError('Debug job start failed: unexpected response', res.status);
  return response;
}

export async function fetchOwnerWorkContext(
  initData: string,
  csrfToken: string | undefined,
  focusId: string,
  intentKind = 'explain',
): Promise<OwnerWorkContextEnvelope> {
  const res = await fetch(`${API_BASE}/webapp/owner-work/context`, {
    method: 'POST',
    headers: authHeaders(initData, csrfToken),
    credentials: 'include',
    body: JSON.stringify({
      focus_id: focusId,
      intent_kind: intentKind,
    }),
  });

  const json = await expectJson(res, 'Context load failed');
  const context = readOwnerWorkContextEnvelope(json);
  if (!context) throw new ApiResponseError('Context load failed: unexpected response', res.status);
  return context;
}
