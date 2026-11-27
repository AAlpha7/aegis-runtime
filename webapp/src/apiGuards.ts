interface GuardedStreamTokenInfo {
  stream_token: string;
  expires_at: number;
  expires_in: number;
}

interface GuardedSessionInfo {
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

interface GuardedMfaStatus {
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

interface GuardedMfaStatusResponse {
  authenticated: boolean;
  mfa: GuardedMfaStatus;
}

interface GuardedWebAuthnCredentialSummary {
  id: string;
  nickname?: string | null;
  transports: string[];
  backup_eligible?: boolean | null;
  backup_state?: boolean | null;
  created_at: number;
  last_used_at?: number | null;
}

interface GuardedTotpSetup {
  secret_base32: string;
  otpauth_url: string;
  issuer: string;
  account_name: string;
  digits: number;
  period_secs: number;
  algorithm: string;
}

interface GuardedTotpSetupResponse {
  status: string;
  method: string;
  setup: GuardedTotpSetup;
}

interface GuardedConsoleApprovalActionResponse {
  status: string;
  approval_id: string;
  action: 'approve' | 'reject';
  risk: string;
  approval_type: string;
  message: string;
}

interface GuardedConsoleMandateSummary {
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

interface GuardedConsoleMandateLifecycleResponse {
  status: string;
  mandate: GuardedConsoleMandateSummary;
  receipt_expectation: string;
}

interface GuardedConsoleMemoryDeleteResponse {
  status: string;
  resource_type: string;
  resource_id: string;
  deleted: boolean;
  ledger_entry_id: number;
  receipt: string;
}

interface GuardedConsoleMemoryUpdateResponse {
  status: string;
  resource_type: string;
  resource_id: string;
  updated: boolean;
  ledger_entry_id: number;
  receipt: string;
}

interface GuardedConsoleProfileUpdateResponse {
  status: string;
  resource_type: string;
  resource_id: string;
  updated: boolean;
  ledger_entry_id: number;
  receipt: string;
}

interface GuardedOwnerWorkAction {
  label: string;
  label_code?: string | null;
  action_kind: string;
  route?: string | null;
  requires_gate: boolean;
  requires_mfa: boolean;
}

interface GuardedOwnerWorkItem {
  id: string;
  kind: string;
  source_kind?: string;
  title: string;
  title_code?: string | null;
  summary: string;
  owner_visible_reason?: string;
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
  primary_action?: GuardedOwnerWorkAction | null;
  secondary_actions: GuardedOwnerWorkAction[];
  next_safe_action?: string;
  next_safe_action_code?: string | null;
  blocked_by?: string | null;
  blocked_by_code?: string | null;
  receipt_expectation?: string;
  authority?: GuardedOwnerWorkAuthorityEnvelope | null;
  detail_route?: string | null;
  external_url_policy: string;
  related_action_ids: string[];
  related_receipt_ids: string[];
  route_reason: string;
  schema_version: number;
  decision_request?: GuardedVNextDecisionRequest | null;
}

interface GuardedOwnerWorkQueue {
  schema_version: number;
  generated_at: string;
  owner: string;
  sources: { source: string; status: string; reason?: string | null }[];
  counts: Record<string, number>;
  items: GuardedOwnerWorkItem[];
}

interface GuardedOwnerWorkContextGate {
  gate_kind: string;
  reason: string;
  route?: string | null;
  requires_mfa: boolean;
}

interface GuardedOwnerWorkContextEnvelope {
  schema_version: number;
  focus_id: string;
  focus_kind: string;
  focus_source: string;
  visible_fields: Record<string, string>;
  redacted_fields: string[];
  authorized_sensitive_scopes: string[];
  denied_sensitive_scopes: string[];
  allowed_intents: string[];
  required_gate?: GuardedOwnerWorkContextGate | null;
  related_action_ids: string[];
  related_receipt_ids: string[];
  source_event_id?: string | null;
  trace_id?: string | null;
  run_id?: string | null;
}

interface GuardedOwnerWorkAuthorityEnvelope {
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

interface GuardedOwnerWorkResponse {
  schema_version: number;
  response_kind: string;
  visible_answer: string;
  severity?: 'info' | 'notice' | 'important' | 'warning' | 'critical' | null;
  draft?: string | null;
  action_draft?: unknown;
  suggested_intent?: GuardedOwnerIntentSuggestion | null;
  authority?: GuardedOwnerWorkAuthorityEnvelope | null;
  required_gate?: GuardedOwnerWorkContextGate | null;
  receipt_expectation: string;
  citations: string[];
  source_evidence?: GuardedOwnerWorkResponseSourceEvidence[];
  evidence?: GuardedAssistantEvidenceItem[];
  unsupported_numbers?: string[];
  warnings: string[];
  trace_id?: string | null;
  run_id?: string | null;
  provider_request_id?: string | null;
}

interface GuardedJevProvenanceEvidence {
  verdict: string;
  owner_decision?: string | null;
  approval_id?: string | null;
  receipt?: string | null;
  audit_tip?: string | null;
  risk?: string | null;
  model?: string | null;
  reason?: string | null;
}

interface GuardedOwnerWorkResponseSourceEvidence {
  kind: string;
  label: string;
  summary: string;
  detail: string;
  jev?: GuardedJevProvenanceEvidence | null;
}

interface GuardedAssistantEvidenceItem {
  id: string;
  tool: string;
  label: string;
  excerpt: string;
  full_len: number;
}

interface GuardedOwnerIntentSuggestion {
  kind: string;
  label: string;
  summary: string;
  confidence: number;
  next_action_kind: string;
}

interface GuardedVNextActionRef {
  id: string;
  label: string;
  label_code?: string | null;
  kind: string;
  route?: string | null;
  requires_confirmation: boolean;
  requires_mfa: boolean;
  requires_data_gate: boolean;
  expected_receipt: string;
}

interface GuardedVNextDecisionChoice {
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
  tone: string;
}

interface GuardedVNextDecisionRequest {
  id: string;
  title: string;
  title_code?: string | null;
  what_aegis_was_doing: string;
  what_code?: string | null;
  blocked_reason: string;
  blocked_reason_code?: string | null;
  requested_capability: string;
  capability_code?: string | null;
  scope: string;
  scope_code?: string | null;
  choices: GuardedVNextDecisionChoice[];
  safe_details: string[];
  safe_detail_codes?: string[] | null;
  audit_refs: string[];
  receipt_expectation: string;
}

interface GuardedVNextAuthorityEnvelope {
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

interface GuardedVNextWorkItem {
  id: string;
  lane: string;
  source_kind: string;
  source: string;
  source_id: string;
  source_event_id?: string | null;
  title: string;
  title_code?: string | null;
  summary: string;
  why_it_matters: string;
  why_it_matters_code?: string | null;
  state: string;
  urgency: string;
  sensitivity: string;
  created_at: string;
  updated_at: string;
  due_at?: string | null;
  authority: GuardedVNextAuthorityEnvelope;
  recommended_action?: GuardedVNextActionRef | null;
  contextual_actions: GuardedVNextActionRef[];
  next_safe_action: string;
  next_safe_action_code?: string | null;
  blocked_by?: string | null;
  blocked_by_code?: string | null;
  detail_route?: string | null;
  external_url_policy: string;
  route_reason: string;
  related_work_item_ids: string[];
  related_receipt_ids: string[];
  receipt_expectation?: string | null;
  decision_request?: GuardedVNextDecisionRequest | null;
}

interface GuardedVNextStatusSummary {
  label: string;
  health: string;
  owner_message: string;
  owner_next_step?: string | null;
}

interface GuardedVNextSystemEvent {
  id: string;
  kind: string;
  severity: string;
  title: string;
  summary: string;
  occurred_at: number;
  window_start_at: number;
  count: number;
  details: string[];
}

interface GuardedVNextWorkProjection {
  schema_version: number;
  generated_at: string;
  sources: GuardedOwnerWorkQueue['sources'];
  counts: Record<string, number>;
  lanes: Array<{
    id: string;
    title: string;
    collapsed: boolean;
    items: GuardedVNextWorkItem[];
  }>;
}

interface GuardedVNextItemProjection {
  schema_version: number;
  generated_at: string;
  item: GuardedVNextWorkItem;
  safe_source_detail: string;
  history: Array<{ id: string; kind: string; label: string; created_at: string }>;
  collaboration: {
    focus_id: string;
    placeholder: string;
    suggested_prompts: string[];
  };
}

interface GuardedVNextLlmDegraded {
  reason: string;
  since: string;
  provider: string;
}

interface GuardedVNextEmbedderDegraded {
  since: string;
  primary_model: string;
  backup_model: string | null;
}

export type GuardedVNextTelegramRailState = 'unconfigured' | 'starting' | 'reachable' | 'invalid_token' | 'unreachable';

/**
 * Telegram bot connectivity as `/webapp/vnext/status` reports it (incident
 * 2026-09-15: a revoked bot token was invisible in the console for hours).
 * `since_unix` is when the current `state` was entered; `degraded` is the
 * server's own verdict (invalid token immediately, unreachable after ten
 * minutes) so the banner logic does not depend on the phone's clock alone.
 */
export interface GuardedVNextTelegramRail {
  state: GuardedVNextTelegramRailState;
  since_unix: number;
  last_error: string | null;
  degraded: boolean;
}

/** Verdict of the last nightly test run (`components.nightly` on /health). */
export interface GuardedVNextNightlyStatus {
  verdict: string;
  sha: string;
  finished_at_unix: number;
  failed_steps: string[];
  report: string | null;
}

interface GuardedVNextStatusProjection {
  schema_version: number;
  generated_at: string;
  sections: GuardedVNextStatusSummary[];
  system_events: GuardedVNextSystemEvent[];
  advanced_diagnostics_available: boolean;
  llm_degraded?: GuardedVNextLlmDegraded | null;
  embedder_degraded?: GuardedVNextEmbedderDegraded | null;
  /** Absent on servers older than 2026-09-16; null never (the rail always exists). */
  telegram_rail?: GuardedVNextTelegramRail | null;
  /** Absent on older servers; null when no nightly status file exists yet. */
  nightly?: GuardedVNextNightlyStatus | null;
}

interface GuardedVNextSettingsProjection {
  schema_version: number;
  signed_in_owner: string;
  connected_services: GuardedVNextStatusSummary[];
  security: GuardedVNextStatusSummary[];
  notification_preferences: GuardedVNextStatusSummary[];
  cost_saving_paused: boolean;
  advanced_diagnostics_available: boolean;
  user_timezone?: string;
  display_language?: string;
  llm_provider?: string;
  llm_model?: string;
  paid_api_fallback_enabled?: boolean;
  web_search_provider?: string;
  web_search_model?: string;
  web_search_model_options?: string[];
}

interface GuardedCatalogModel {
  id: string;
  display_name: string;
}

interface GuardedProviderCatalog {
  id: string;
  connected: boolean;
  models: GuardedCatalogModel[];
  source: string;
  fetched_at_unix?: number | null;
  default_model?: string;
}

interface GuardedVNextModelCatalog {
  recommended_api_model?: string | null;
  schema_version: number;
  providers: GuardedProviderCatalog[];
  current: { provider: string; model: string; effective_model?: string };
  fallback: { provider?: string | null; model: string };
}

interface GuardedVNextChatHistoryProjection {
  schema_version: number;
  approvals?: Array<{
    approval_id: string;
    tool_name: string;
    summary: string;
    state: 'pending' | 'approved' | 'rejected';
    timestamp: number;
    outcome?: string | null;
  }>;
  messages: Array<{
    role: 'owner' | 'aegis';
    text: string;
    timestamp: number;
    severity?: 'info' | 'notice' | 'important' | 'warning' | 'critical' | null;
    latency_ms?: number;
    repeat_count?: number;
    first_timestamp?: number;
    work_item_id?: string | null;
    evidence?: Array<{
      id: string;
      tool: string;
      label: string;
      excerpt: string;
      full_len: number;
    }>;
    source_evidence?: GuardedOwnerWorkResponseSourceEvidence[];
    unsupported_numbers?: string[];
    retry_text?: string | null;
  }>;
}

interface GuardedVNextAvatarProfileProjection {
  schema_version: number;
  aegis_avatar_url: string;
  owner_avatar_url: string;
  updated_at_unix?: number | null;
  auto_refresh?: {
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
  };
}

interface GuardedVNextCommandResponse {
  schema_version: number;
  generated_at: string;
  outcome: string;
  owner_message: string;
  assistant_response?: unknown;
  work_item?: GuardedVNextWorkItem | null;
  action_draft?: GuardedVNextActionRef | null;
  permission_request?: GuardedVNextAuthorityEnvelope | null;
  receipt?: GuardedVNextReceiptRef | null;
}

interface GuardedVNextReceiptRef {
  id: string;
  kind: string;
  label: string;
  created_at: string;
}

interface GuardedVNextWorkActionResponse {
  schema_version: number;
  generated_at: string;
  status: string;
  item_id: string;
  action_id: string;
  owner_message: string;
  receipt?: GuardedVNextReceiptRef | null;
  raw_result?: unknown;
}

interface GuardedVNextDebugSnapshotResponse {
  schema_version: number;
  id: string;
  expires_at_unix: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNoticeSeverity(value: unknown): value is 'info' | 'notice' | 'important' | 'warning' | 'critical' {
  return value === 'info'
    || value === 'notice'
    || value === 'important'
    || value === 'warning'
    || value === 'critical';
}

function isOwnerWorkAction(value: unknown): value is GuardedOwnerWorkAction {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.label, 80) || !isSafeShortText(value.action_kind, 80)) return false;
  if (!isOptionalSafeShortText(value.label_code, 64)) return false;
  if (value.route !== undefined && value.route !== null && !isSafeInternalRoute(value.route, 256)) return false;
  if (typeof value.requires_gate !== 'boolean' || typeof value.requires_mfa !== 'boolean') return false;
  return true;
}

function isOwnerWorkItem(value: unknown): value is GuardedOwnerWorkItem {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 128)) return false;
  if (!isSafeShortText(value.kind, 32)) return false;
  if (value.source_kind !== undefined && !isSafeShortText(value.source_kind, 64)) return false;
  if (!isSafeShortText(value.title, 240)) return false;
  if (!isOptionalSafeShortText(value.title_code, 64)) return false;
  if (!isSafeShortText(value.summary, 512)) return false;
  if (value.owner_visible_reason !== undefined && !isSafeShortText(value.owner_visible_reason, 256)) return false;
  if (!isOptionalSafeShortText(value.owner_visible_reason_code, 64)) return false;
  if (value.authority_basis !== undefined && !isSafeShortText(value.authority_basis, 64)) return false;
  if (!isSafeShortText(value.source, 64)) return false;
  if (!isSafeShortText(value.source_id, 128)) return false;
  if (value.source_event_id !== undefined && value.source_event_id !== null && !isSafeShortText(value.source_event_id, 128)) return false;
  if (!isSafeShortText(value.status, 64)) return false;
  if (!isSafeShortText(value.urgency, 32)) return false;
  if (!isSafeShortText(value.sensitivity, 32)) return false;
  if (!isSafeShortText(value.privacy_level, 64)) return false;
  if (!isSafeShortText(value.created_at, 80)) return false;
  if (!isSafeShortText(value.updated_at, 80)) return false;
  if (value.due_at !== undefined && value.due_at !== null && !isSafeShortText(value.due_at, 80)) return false;
  if (value.primary_action !== undefined && value.primary_action !== null && !isOwnerWorkAction(value.primary_action)) return false;
  if (!Array.isArray(value.secondary_actions) || value.secondary_actions.length > 8 || !value.secondary_actions.every(isOwnerWorkAction)) return false;
  if (value.next_safe_action !== undefined && !isSafeShortText(value.next_safe_action, 256)) return false;
  if (!isOptionalSafeShortText(value.next_safe_action_code, 64)) return false;
  if (value.blocked_by !== undefined && value.blocked_by !== null && !isSafeShortText(value.blocked_by, 256)) return false;
  if (!isOptionalSafeShortText(value.blocked_by_code, 64)) return false;
  if (value.receipt_expectation !== undefined && !isSafeShortText(value.receipt_expectation, 512)) return false;
  if (value.authority !== undefined && value.authority !== null && !isOwnerWorkAuthorityEnvelope(value.authority)) return false;
  if (value.detail_route !== undefined && value.detail_route !== null && !isSafeInternalRoute(value.detail_route, 256)) return false;
  if (!isSafeShortText(value.external_url_policy, 128)) return false;
  if (!isStringArray(value.related_action_ids, 16)) return false;
  if (!isStringArray(value.related_receipt_ids, 16)) return false;
  if (!isSafeShortText(value.route_reason, 256)) return false;
  if (!isFiniteNumber(value.schema_version)) return false;
  if (value.decision_request !== undefined && value.decision_request !== null && !isVNextDecisionRequest(value.decision_request)) return false;
  return true;
}

function isOwnerWorkSource(value: unknown): value is GuardedOwnerWorkQueue['sources'][number] {
  return isRecord(value)
    && isSafeShortText(value.source, 64)
    && isSafeShortText(value.status, 64)
    && (value.reason === undefined || value.reason === null || isSafeShortText(value.reason, 256));
}

function isOwnerWorkAuthorityEnvelope(value: unknown): value is GuardedOwnerWorkAuthorityEnvelope {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.item_id, 128)) return false;
  if (!isSafeShortText(value.authority_basis, 64)) return false;
  if (!isStringArray(value.visible_fields, 64)) return false;
  if (!isStringArray(value.locked_fields, 64)) return false;
  if (!isStringArray(value.allowed_now, 32)) return false;
  if (typeof value.requires_data_gate !== 'boolean') return false;
  if (typeof value.requires_mfa !== 'boolean') return false;
  if (typeof value.requires_approval !== 'boolean') return false;
  if (!isSafeShortText(value.expected_receipt, 512)) return false;
  if (value.source_mandate_id !== undefined && value.source_mandate_id !== null && !isSafeShortText(value.source_mandate_id, 128)) return false;
  if (!isStringArray(value.audit_links, 16)) return false;
  return true;
}

function isJevProvenanceEvidence(value: unknown): value is GuardedJevProvenanceEvidence {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.verdict, 16)) return false;
  const verdict = value.verdict.trim().toLowerCase();
  if (verdict !== 'allow' && verdict !== 'ask' && verdict !== 'deny') return false;
  if (value.owner_decision !== undefined && value.owner_decision !== null
    && !isSafeShortText(value.owner_decision, 16)) return false;
  if (value.approval_id !== undefined && value.approval_id !== null
    && !isSafeShortText(value.approval_id, 160)) return false;
  if (value.receipt !== undefined && value.receipt !== null
    && !isSafeShortText(value.receipt, 160)) return false;
  if (value.audit_tip !== undefined && value.audit_tip !== null
    && !isSafeShortText(value.audit_tip, 160)) return false;
  if (value.risk !== undefined && value.risk !== null
    && !isSafeShortText(value.risk, 80)) return false;
  if (value.model !== undefined && value.model !== null
    && !isSafeShortText(value.model, 80)) return false;
  if (value.reason !== undefined && value.reason !== null
    && !isSafeShortText(value.reason, 240)) return false;
  return true;
}

function isOwnerWorkResponseSourceEvidence(value: unknown): value is GuardedOwnerWorkResponseSourceEvidence {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.kind, 64)) return false;
  if (!isSafeShortText(value.label, 96)) return false;
  if (!isSafeShortText(value.summary, 240)) return false;
  if (!isSafeShortText(value.detail, 1200)) return false;
  if (value.jev !== undefined && value.jev !== null && !isJevProvenanceEvidence(value.jev)) return false;
  return true;
}

function isAssistantEvidenceItem(value: unknown): value is GuardedAssistantEvidenceItem {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 8)) return false;
  if (!isSafeShortText(value.tool, 64)) return false;
  if (!isSafeShortText(value.label, 96)) return false;
  if (!isSafeShortText(value.excerpt, 1200)) return false;
  if (!isFiniteNumber(value.full_len) || value.full_len < 0) return false;
  return true;
}

function isOwnerIntentSuggestion(value: unknown): value is GuardedOwnerIntentSuggestion {
  if (!isRecord(value)) return false;
  if (!isSuggestedIntentKind(value.kind)) return false;
  if (!isSafeShortText(value.label, 80)) return false;
  if (!isSafeShortText(value.summary, 512)) return false;
  if (!isSafeShortText(value.next_action_kind, 80)) return false;
  if (!isFiniteNumber(value.confidence) || value.confidence < 0 || value.confidence > 1) return false;
  return true;
}

function isSuggestedIntentKind(value: unknown): value is string {
  if (typeof value !== 'string' || !value) return false;
  // Classic work-projection kinds.
  if (
    value === 'reminder'
    || value === 'task'
    || value === 'tracking'
    || value === 'backlog'
    || value === 'decision'
  ) {
    return true;
  }
  // CEO Intent Contract kinds (shorthand + free-text general chat).
  if (value === 'free_text') return true;
  if (/^[a-z][a-z0-9_]{1,48}$/.test(value)) return true;
  return false;
}

function isOwnerWorkContextGate(value: unknown): value is GuardedOwnerWorkContextGate {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.gate_kind, 80)) return false;
  if (!isSafeShortText(value.reason, 512)) return false;
  if (value.route !== undefined && value.route !== null && !isSafeInternalRoute(value.route, 256)) return false;
  if (typeof value.requires_mfa !== 'boolean') return false;
  return true;
}

function isVNextStatusSummary(value: unknown): value is GuardedVNextStatusSummary {
  return isRecord(value)
    && isSafeShortText(value.label, 120)
    && isSafeShortText(value.health, 32)
    && isSafeShortText(value.owner_message, 512)
    && (value.owner_next_step === undefined
      || value.owner_next_step === null
      || isSafeShortText(value.owner_next_step, 512));
}

function isVNextSystemEvent(value: unknown): value is GuardedVNextSystemEvent {
  return isRecord(value)
    && isSafeShortText(value.id, 160)
    && isSafeShortText(value.kind, 80)
    && isSafeShortText(value.severity, 32)
    && isSafeShortText(value.title, 120)
    && isSafeShortText(value.summary, 512)
    && isFiniteNumber(value.occurred_at)
    && isFiniteNumber(value.window_start_at)
    && isFiniteNumber(value.count)
    && value.count >= 1
    && value.count <= 10_000
    && isStringArray(value.details, 8);
}

function isVNextReceiptRef(value: unknown): value is GuardedVNextReceiptRef {
  return isRecord(value)
    && isSafeShortText(value.id, 160)
    && isSafeShortText(value.kind, 80)
    && isSafeShortText(value.label, 512)
    && isSafeShortText(value.created_at, 80);
}

function isVNextActionRef(value: unknown): value is GuardedVNextActionRef {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 120)) return false;
  if (!isSafeShortText(value.label, 120)) return false;
  if (!isOptionalSafeShortText(value.label_code, 64)) return false;
  if (!isSafeShortText(value.kind, 120)) return false;
  if (value.route !== undefined && value.route !== null && !isSafeInternalRoute(value.route, 256)) return false;
  if (typeof value.requires_confirmation !== 'boolean') return false;
  if (typeof value.requires_mfa !== 'boolean') return false;
  if (typeof value.requires_data_gate !== 'boolean') return false;
  if (!isSafeShortText(value.expected_receipt, 512)) return false;
  return true;
}

function isVNextDecisionChoice(value: unknown): value is GuardedVNextDecisionChoice {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 80)) return false;
  if (!isSafeShortText(value.label, 120)) return false;
  if (!isSafeShortText(value.consequence, 512)) return false;
  if (!isSafeShortText(value.risk_level, 40)) return false;
  if (!isSafeShortText(value.applies_to, 64)) return false;
  if (!isOptionalSafeShortText(value.action_kind, 120)) return false;
  if (!isOptionalSafeShortText(value.assistant_prompt, 512)) return false;
  if (value.detail_route !== undefined && value.detail_route !== null && !isSafeInternalRoute(value.detail_route, 256)) return false;
  if (typeof value.requires_confirmation !== 'boolean') return false;
  if (typeof value.enabled !== 'boolean') return false;
  if (!isOptionalSafeShortText(value.disabled_reason, 256)) return false;
  if (!isSafeShortText(value.tone, 40)) return false;
  return true;
}

function isVNextDecisionRequest(value: unknown): value is GuardedVNextDecisionRequest {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 128)) return false;
  if (!isSafeShortText(value.title, 160)) return false;
  if (!isOptionalSafeShortText(value.title_code, 64)) return false;
  if (!isSafeShortText(value.what_aegis_was_doing, 512)) return false;
  if (!isOptionalSafeShortText(value.what_code, 64)) return false;
  if (!isSafeShortText(value.blocked_reason, 512)) return false;
  if (!isOptionalSafeShortText(value.blocked_reason_code, 64)) return false;
  if (!isSafeShortText(value.requested_capability, 256)) return false;
  if (!isOptionalSafeShortText(value.capability_code, 64)) return false;
  if (!isSafeShortText(value.scope, 256)) return false;
  if (!isOptionalSafeShortText(value.scope_code, 64)) return false;
  if (!Array.isArray(value.choices) || value.choices.length > 8 || !value.choices.every(isVNextDecisionChoice)) return false;
  if (!isStringArray(value.safe_details, 8)) return false;
  // Empty string entries are intentional (freeform padding); isStringArray allows them.
  if (value.safe_detail_codes !== undefined && value.safe_detail_codes !== null && !isStringArray(value.safe_detail_codes, 8)) return false;
  if (!isStringArray(value.audit_refs, 16)) return false;
  if (!isSafeShortText(value.receipt_expectation, 512)) return false;
  return true;
}

function isVNextAuthorityEnvelope(value: unknown): value is GuardedVNextAuthorityEnvelope {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.basis, 80)) return false;
  if (!isSafeShortText(value.owner_visible_reason, 512)) return false;
  if (!isStringArray(value.visible_data, 64)) return false;
  if (!isStringArray(value.locked_data, 64)) return false;
  if (!isStringArray(value.allowed_now, 64)) return false;
  if (typeof value.requires_mfa !== 'boolean') return false;
  if (typeof value.requires_approval !== 'boolean') return false;
  if (typeof value.requires_data_grant !== 'boolean') return false;
  if (!isOptionalSafeShortText(value.expected_receipt, 512)) return false;
  if (!isOptionalSafeShortText(value.source_mandate_id, 128)) return false;
  if (!isStringArray(value.audit_refs, 32)) return false;
  return true;
}

function isVNextWorkItem(value: unknown): value is GuardedVNextWorkItem {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 128)) return false;
  if (!['now', 'decide', 'follow_up', 'backlog', 'done'].includes(String(value.lane))) return false;
  if (!isSafeShortText(value.source_kind, 64)) return false;
  if (!isSafeShortText(value.source, 64)) return false;
  if (!isSafeShortText(value.source_id, 128)) return false;
  if (value.source_event_id !== undefined && value.source_event_id !== null && !isSafeShortText(value.source_event_id, 128)) return false;
  if (!isSafeShortText(value.title, 240)) return false;
  if (!isOptionalSafeShortText(value.title_code, 64)) return false;
  if (!isSafeShortText(value.summary, 512)) return false;
  if (!isSafeShortText(value.why_it_matters, 512)) return false;
  if (!isOptionalSafeShortText(value.why_it_matters_code, 64)) return false;
  if (!isSafeShortText(value.state, 80)) return false;
  if (!isSafeShortText(value.urgency, 40)) return false;
  if (!isSafeShortText(value.sensitivity, 32)) return false;
  if (!isSafeShortText(value.created_at, 80)) return false;
  if (!isSafeShortText(value.updated_at, 80)) return false;
  if (!isOptionalSafeShortText(value.due_at, 80)) return false;
  if (!isVNextAuthorityEnvelope(value.authority)) return false;
  if (value.recommended_action !== undefined && value.recommended_action !== null && !isVNextActionRef(value.recommended_action)) return false;
  if (!Array.isArray(value.contextual_actions) || value.contextual_actions.length > 16 || !value.contextual_actions.every(isVNextActionRef)) return false;
  if (!isSafeShortText(value.next_safe_action, 256)) return false;
  if (!isOptionalSafeShortText(value.next_safe_action_code, 64)) return false;
  if (value.blocked_by !== undefined && value.blocked_by !== null && !isSafeShortText(value.blocked_by, 256)) return false;
  if (!isOptionalSafeShortText(value.blocked_by_code, 64)) return false;
  if (value.detail_route !== undefined && value.detail_route !== null && !isSafeInternalRoute(value.detail_route, 256)) return false;
  if (!isSafeShortText(value.external_url_policy, 128)) return false;
  if (!isSafeShortText(value.route_reason, 256)) return false;
  if (!isStringArray(value.related_work_item_ids, 32)) return false;
  if (!isStringArray(value.related_receipt_ids, 32)) return false;
  if (!isOptionalSafeShortText(value.receipt_expectation, 512)) return false;
  if (value.decision_request !== undefined && value.decision_request !== null && !isVNextDecisionRequest(value.decision_request)) return false;
  if (value.lineage !== undefined && value.lineage !== null) {
    if (!Array.isArray(value.lineage) || value.lineage.length > 6 || !value.lineage.every(isWorkLineageNode)) return false;
  }
  return true;
}

function isWorkLineageNode(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (!['occurrence', 'intent', 'origin'].includes(String(value.layer))) return false;
  if (!isSafeShortText(value.kind, 64)) return false;
  if (!isSafeShortText(value.id, 160)) return false;
  if (!isSafeShortText(value.label, 160)) return false;
  if (!isOptionalSafeShortText(value.label_code, 64)) return false;
  if (!isSafeShortText(value.detail, 600)) return false;
  if (value.action_ids !== undefined && value.action_ids !== null && !isStringArray(value.action_ids, 4)) return false;
  if (!isOptionalSafeShortText(value.turn_id, 256)) return false;
  if (value.message_role !== undefined && value.message_role !== null && value.message_role !== 'owner' && value.message_role !== 'aegis') return false;
  return true;
}

function isVisibleFieldMap(value: unknown): value is Record<string, string> {
  return isRecord(value)
    && Object.keys(value).length <= 48
    && Object.entries(value).every(([key, item]) => isSafeShortText(key, 80) && isSafeShortText(item, 1024));
}

function isStringArray(value: unknown, maxLength: number): value is string[] {
  return Array.isArray(value)
    && value.length <= maxLength
    && value.every((item) => typeof item === 'string' && item.length <= 256);
}

function isOptionalSafeShortText(value: unknown, maxLength: number): value is string | null | undefined {
  return value === undefined || value === null || isSafeShortText(value, maxLength);
}

function isRedactedPolicySummary(value: unknown): value is Record<string, unknown> {
  if (!isRecord(value)) return false;
  if (Object.keys(value).length > 12) return false;
  return Object.values(value).every((item) => {
    if (item === null) return true;
    if (typeof item === 'boolean') return true;
    if (typeof item === 'number') return Number.isFinite(item);
    if (typeof item === 'string') return item.length <= 160;
    if (Array.isArray(item)) return item.length <= 12 && item.every((entry) => typeof entry === 'string' && entry.length <= 80);
    if (isRecord(item)) return JSON.stringify(item).length <= 512;
    return false;
  });
}

function isConsoleMandateSummary(value: unknown): value is GuardedConsoleMandateSummary {
  if (!isRecord(value)) return false;
  if (!isFiniteNumber(value.schema_version)) return false;
  if (!isSafeShortText(value.mandate_id, 160)) return false;
  if (!['draft', 'active', 'paused', 'expired', 'revoked'].includes(String(value.status))) return false;
  if (!isSafeShortText(value.title, 160)) return false;
  if (!isSafeShortText(value.description, 1200)) return false;
  if (!isSafeShortText(value.scope_kind, 96)) return false;
  if (!isStringArray(value.allowed_reads, 32)) return false;
  if (!isStringArray(value.allowed_llm_uses, 32)) return false;
  if (!isStringArray(value.allowed_actions, 32)) return false;
  if (!isStringArray(value.must_ask, 32)) return false;
  if (!isRedactedPolicySummary(value.notification_policy)) return false;
  if (!isRedactedPolicySummary(value.budget_policy)) return false;
  if (!isRedactedPolicySummary(value.frequency_policy)) return false;
  if (!isOptionalSafeShortText(value.expiry, 100)) return false;
  if (!isOptionalSafeShortText(value.review_after, 100)) return false;
  if (!isSafeShortText(value.created_at, 100)) return false;
  if (!isSafeShortText(value.updated_at, 100)) return false;
  if (!isSafeShortText(value.created_from, 80)) return false;
  if (!isOptionalSafeShortText(value.source_event_id, 180)) return false;
  if (!isOptionalSafeShortText(value.trace_id, 180)) return false;
  if (!isOptionalSafeShortText(value.run_id, 180)) return false;
  if (typeof value.matching_enabled !== 'boolean') return false;
  if (value.background_autonomy_enabled !== false) return false;
  return true;
}

function isOptionalFiniteNumberOrNull(value: unknown): value is number | null | undefined {
  return value === undefined || value === null || isFiniteNumber(value);
}

function isOptionalBooleanOrNull(value: unknown): value is boolean | null | undefined {
  return value === undefined || value === null || typeof value === 'boolean';
}

const TOTP_SECRET_BASE32 = /^[A-Z2-7]+=*$/;

function isSafeTotpSecret(value: unknown): value is string {
  return typeof value === 'string'
    && value.length >= 16
    && value.length <= 128
    && TOTP_SECRET_BASE32.test(value);
}

function isSafeShortText(value: unknown, maxLength: number): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= maxLength;
}

function isSafeInternalRoute(value: unknown, maxLength: number): value is string {
  return typeof value === 'string'
    && value.length > 0
    && value.length <= maxLength
    && value.startsWith('/')
    && !value.startsWith('//')
    && !/[<>"'\\]/.test(value);
}

function isSafeTotpSetupUrl(value: unknown, secretBase32: string): value is string {
  if (typeof value !== 'string' || value !== value.trim() || value.length > 1024) return false;
  try {
    const url = new URL(value);
    if (url.protocol !== 'otpauth:' || url.hostname !== 'totp') return false;
    if (url.username || url.password) return false;
    if (!url.pathname || url.pathname === '/') return false;
    return url.searchParams.get('secret') === secretBase32;
  } catch {
    return false;
  }
}

export function readSessionInfo(value: unknown): GuardedSessionInfo | null {
  if (!isRecord(value) || typeof value.authenticated !== 'boolean') return null;
  if (value.google_login_enabled !== undefined && typeof value.google_login_enabled !== 'boolean') return null;
  if (value.passkey_login_enabled !== undefined && typeof value.passkey_login_enabled !== 'boolean') return null;
  if (value.principal_type !== undefined && value.principal_type !== 'web_console' && value.principal_type !== 'telegram' && value.principal_type !== 'admin') return null;
  if (value.email !== undefined && typeof value.email !== 'string') return null;
  if (value.mfa_level !== undefined && typeof value.mfa_level !== 'string') return null;
  if (!isOptionalFiniteNumberOrNull(value.mfa_verified_at)) return null;
  if (!isOptionalFiniteNumberOrNull(value.mfa_expires_at)) return null;
  if (value.mfa_freshness_secs !== undefined && !isFiniteNumber(value.mfa_freshness_secs)) return null;
  if (value.protected_access_unlocked !== undefined && typeof value.protected_access_unlocked !== 'boolean') return null;
  if (value.requires_mfa_setup !== undefined && typeof value.requires_mfa_setup !== 'boolean') return null;
  if (value.csrf_token !== undefined && typeof value.csrf_token !== 'string') return null;
  if (value.assistant_name !== undefined && typeof value.assistant_name !== 'string') return null;
  return value as unknown as GuardedSessionInfo;
}

export function readOwnerWorkQueue(value: unknown): GuardedOwnerWorkQueue | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!isSafeShortText(value.owner, 128)) return null;
  if (!Array.isArray(value.sources) || value.sources.length > 24 || !value.sources.every(isOwnerWorkSource)) return null;
  if (!isRecord(value.counts)) return null;
  if (!Object.values(value.counts).every((count) => isFiniteNumber(count) && count >= 0 && count <= 1_000_000)) return null;
  if (!Array.isArray(value.items) || value.items.length > 100 || !value.items.every(isOwnerWorkItem)) return null;
  return value as unknown as GuardedOwnerWorkQueue;
}

export function readVNextWorkProjection(value: unknown): GuardedVNextWorkProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!Array.isArray(value.sources) || value.sources.length > 32 || !value.sources.every(isOwnerWorkSource)) return null;
  if (!isRecord(value.counts)) return null;
  if (!Array.isArray(value.lanes) || value.lanes.length !== 5) return null;
  for (const lane of value.lanes) {
    if (!isRecord(lane)) return null;
    if (!['now', 'decide', 'follow_up', 'backlog', 'done'].includes(String(lane.id))) return null;
    if (!isSafeShortText(lane.title, 80)) return null;
    if (typeof lane.collapsed !== 'boolean') return null;
    if (!Array.isArray(lane.items) || lane.items.length > 100 || !lane.items.every(isVNextWorkItem)) return null;
  }
  return value as unknown as GuardedVNextWorkProjection;
}

export function readVNextItemProjection(value: unknown): GuardedVNextItemProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!isVNextWorkItem(value.item)) return null;
  if (!isSafeShortText(value.safe_source_detail, 1200)) return null;
  if (!Array.isArray(value.history) || value.history.length > 32) return null;
  for (const entry of value.history) {
    if (!isRecord(entry)) return null;
    if (!isSafeShortText(entry.id, 128)) return null;
    if (!isSafeShortText(entry.kind, 80)) return null;
    if (!isSafeShortText(entry.label, 160)) return null;
    if (!isSafeShortText(entry.created_at, 80)) return null;
  }
  if (!isRecord(value.collaboration)) return null;
  if (!isSafeShortText(value.collaboration.focus_id, 128)) return null;
  if (!isSafeShortText(value.collaboration.placeholder, 160)) return null;
  if (!isStringArray(value.collaboration.suggested_prompts, 8)) return null;
  return value as unknown as GuardedVNextItemProjection;
}

/**
 * Defensively validates the optional `llm_degraded` block. A malformed shape is
 * treated as "healthy" (returns `undefined`) rather than failing the whole
 * status projection — a degraded-banner field must never be able to break the
 * status fetch. Returns `null` only when the field is explicitly `null`.
 */
function sanitizeLlmDegraded(value: unknown): GuardedVNextLlmDegraded | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  if (!isSafeShortText(value.reason, 512)) return undefined;
  if (!isSafeShortText(value.since, 80)) return undefined;
  if (!isSafeShortText(value.provider, 64)) return undefined;
  return { reason: value.reason, since: value.since, provider: value.provider };
}

/**
 * Defensively validates the optional `embedder_degraded` block (semantic dedup
 * paused because the primary embedding model is down). Mirrors
 * `sanitizeLlmDegraded`: a malformed shape is coerced to "healthy"
 * (`undefined`) rather than failing the whole status projection. Returns
 * `null` only when the field is explicitly `null`. `backup_model` is
 * normalized to `string | null` (absent → null: no backup exists).
 */
function sanitizeEmbedderDegraded(value: unknown): GuardedVNextEmbedderDegraded | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  if (!isSafeShortText(value.since, 80)) return undefined;
  if (!isSafeShortText(value.primary_model, 128)) return undefined;
  if (value.backup_model !== undefined && value.backup_model !== null && !isSafeShortText(value.backup_model, 128)) return undefined;
  return {
    since: value.since,
    primary_model: value.primary_model,
    backup_model: typeof value.backup_model === 'string' ? value.backup_model : null,
  };
}

const TELEGRAM_RAIL_STATES: readonly GuardedVNextTelegramRailState[] = ['unconfigured', 'starting', 'reachable', 'invalid_token', 'unreachable'];

function isTelegramRailState(value: unknown): value is GuardedVNextTelegramRailState {
  return typeof value === 'string' && (TELEGRAM_RAIL_STATES as readonly string[]).includes(value);
}

/**
 * Defensively validates the optional `telegram_rail` block. Same contract as
 * `sanitizeLlmDegraded`: a malformed shape is coerced to `undefined` (treated
 * as "this server does not report it") rather than failing the status fetch;
 * `null` only when explicitly null. An unknown `state` string is malformed —
 * the banner must never guess what a state it has no copy for means.
 */
export function sanitizeTelegramRail(value: unknown): GuardedVNextTelegramRail | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  if (!isTelegramRailState(value.state)) return undefined;
  if (!isFiniteNumber(value.since_unix) || value.since_unix < 0) return undefined;
  if (value.last_error !== undefined && value.last_error !== null && !isSafeShortText(value.last_error, 512)) return undefined;
  if (value.degraded !== undefined && typeof value.degraded !== 'boolean') return undefined;
  return {
    state: value.state,
    since_unix: Math.floor(value.since_unix),
    last_error: typeof value.last_error === 'string' ? value.last_error : null,
    degraded: value.degraded === true,
  };
}

/**
 * Defensively validates the optional `nightly` block (last nightly test
 * verdict). Malformed → `undefined` (nothing to show); `null` only when
 * explicitly null (no nightly status file yet).
 */
export function sanitizeNightlyStatus(value: unknown): GuardedVNextNightlyStatus | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  if (!isSafeShortText(value.verdict, 32) || !/^[A-Z][A-Z0-9_]*$/.test(value.verdict)) return undefined;
  if (value.sha !== undefined && value.sha !== null && (typeof value.sha !== 'string' || value.sha.length > 64)) return undefined;
  if (!isFiniteNumber(value.finished_at_unix) || value.finished_at_unix < 0) return undefined;
  if (value.failed_steps !== undefined && value.failed_steps !== null && !isStringArray(value.failed_steps, 64)) return undefined;
  if (value.report !== undefined && value.report !== null && !isSafeShortText(value.report, 512)) return undefined;
  return {
    verdict: value.verdict,
    sha: typeof value.sha === 'string' ? value.sha : '',
    finished_at_unix: Math.floor(value.finished_at_unix),
    failed_steps: Array.isArray(value.failed_steps) ? value.failed_steps : [],
    report: typeof value.report === 'string' ? value.report : null,
  };
}

export function readVNextStatusProjection(value: unknown): GuardedVNextStatusProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!Array.isArray(value.sections) || value.sections.length > 16 || !value.sections.every(isVNextStatusSummary)) return null;
  if (!Array.isArray(value.system_events) || value.system_events.length > 16 || !value.system_events.every(isVNextSystemEvent)) return null;
  if (typeof value.advanced_diagnostics_available !== 'boolean') return null;
  // Sanitize the optional degraded blocks to validated shapes (or drop them).
  const llmDegraded = sanitizeLlmDegraded(value.llm_degraded);
  const embedderDegraded = sanitizeEmbedderDegraded(value.embedder_degraded);
  const telegramRail = sanitizeTelegramRail(value.telegram_rail);
  const nightly = sanitizeNightlyStatus(value.nightly);
  return {
    ...value,
    llm_degraded: llmDegraded,
    embedder_degraded: embedderDegraded,
    telegram_rail: telegramRail,
    nightly,
  } as unknown as GuardedVNextStatusProjection;
}

export function readVNextSettingsProjection(value: unknown): GuardedVNextSettingsProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.signed_in_owner, 160)) return null;
  if (!Array.isArray(value.connected_services) || value.connected_services.length > 24 || !value.connected_services.every(isVNextStatusSummary)) return null;
  if (!Array.isArray(value.security) || value.security.length > 12 || !value.security.every(isVNextStatusSummary)) return null;
  if (!Array.isArray(value.notification_preferences) || value.notification_preferences.length > 12 || !value.notification_preferences.every(isVNextStatusSummary)) return null;
  if (typeof value.cost_saving_paused !== 'boolean') return null;
  if (typeof value.advanced_diagnostics_available !== 'boolean') return null;
  // Optional for backward compatibility with servers predating the field.
  if (value.user_timezone !== undefined && !isSafeShortText(value.user_timezone, 64)) return null;
  if (value.display_language !== undefined && !isSafeShortText(value.display_language, 16)) return null;
  if (value.llm_provider !== undefined && !isSafeShortText(value.llm_provider, 32)) return null;
  if (value.llm_model !== undefined && !(value.llm_model === '' || isSafeShortText(value.llm_model, 200))) return null;
  if (value.paid_api_fallback_enabled !== undefined && typeof value.paid_api_fallback_enabled !== 'boolean') return null;
  if (value.web_search_provider !== undefined && !isSafeShortText(value.web_search_provider, 32)) return null;
  if (value.web_search_model !== undefined && !(value.web_search_model === '' || isSafeShortText(value.web_search_model, 200))) return null;
  if (value.web_search_model_options !== undefined) {
    if (!Array.isArray(value.web_search_model_options) || value.web_search_model_options.length > 20) return null;
    if (!value.web_search_model_options.every((entry) => isSafeShortText(entry, 200))) return null;
  }
  return value as unknown as GuardedVNextSettingsProjection;
}

function isCatalogModel(value: unknown): value is GuardedCatalogModel {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 200)) return false;
  if (!isSafeShortText(value.display_name, 200)) return false;
  return true;
}

function isProviderCatalog(value: unknown): value is GuardedProviderCatalog {
  if (!isRecord(value)) return false;
  if (!isSafeShortText(value.id, 32)) return false;
  if (typeof value.connected !== 'boolean') return false;
  if (!isSafeShortText(value.source, 64)) return false;
  if (value.fetched_at_unix !== undefined && value.fetched_at_unix !== null && !isFiniteNumber(value.fetched_at_unix)) return false;
  if (!Array.isArray(value.models) || value.models.length > 1000 || !value.models.every(isCatalogModel)) return false;
  if (!(value.default_model === undefined || value.default_model === '' || isSafeShortText(value.default_model, 200))) return false;
  return true;
}

export function readVNextModelCatalog(value: unknown): GuardedVNextModelCatalog | null {
  if (!isRecord(value)) return null;
  if (value.recommended_api_model !== undefined && value.recommended_api_model !== null
    && !isSafeShortText(value.recommended_api_model, 200)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!Array.isArray(value.providers) || value.providers.length > 10 || !value.providers.every(isProviderCatalog)) return null;
  if (!isRecord(value.current)) return null;
  if (!isSafeShortText(value.current.provider, 32)) return null;
  if (!(value.current.model === '' || isSafeShortText(value.current.model, 200))) return null;
  if (!(value.current.effective_model === undefined || value.current.effective_model === '' || isSafeShortText(value.current.effective_model, 200))) return null;
  if (!isRecord(value.fallback)) return null;
  if (value.fallback.provider !== undefined && value.fallback.provider !== null && !isSafeShortText(value.fallback.provider, 32)) return null;
  if (!(value.fallback.model === '' || isSafeShortText(value.fallback.model, 200))) return null;
  return value as unknown as GuardedVNextModelCatalog;
}

function isVNextChatHistoryEvidenceItem(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (typeof value.id !== 'string' || !/^E\d+$/.test(value.id)) return false;
  if (typeof value.tool !== 'string' || value.tool.length > 96) return false;
  if (typeof value.label !== 'string' || value.label.length > 96) return false;
  if (typeof value.excerpt !== 'string' || value.excerpt.length > 1200) return false;
  if (typeof value.full_len !== 'number' || !Number.isFinite(value.full_len) || value.full_len < 0) return false;
  return true;
}

function isVNextChatHistoryUnsupportedNumbers(value: unknown): boolean {
  if (!Array.isArray(value) || value.length > 32) return false;
  return value.every((item) => typeof item === 'string' && item.length <= 64);
}

export function readVNextChatHistoryProjection(value: unknown): GuardedVNextChatHistoryProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  // Window is 24, extended to keep a same-second group whole (server caps the overrun).
  if (!Array.isArray(value.messages) || value.messages.length > 64) return null;
  if (value.has_more !== undefined && typeof value.has_more !== 'boolean') return null;
  if (value.approvals !== undefined) {
    if (!Array.isArray(value.approvals) || value.approvals.length > 24) return null;
    for (const card of value.approvals) {
      if (!isRecord(card)) return null;
      if (!isSafeShortText(card.approval_id, 160)) return null;
      if (!isSafeShortText(card.tool_name, 120)) return null;
      if (typeof card.summary !== 'string' || card.summary.length > 4_000) return null;
      if (card.state !== 'pending' && card.state !== 'approved' && card.state !== 'rejected') return null;
      if (!isFiniteNumber(card.timestamp)) return null;
      if (card.outcome !== undefined && card.outcome !== null
        && (typeof card.outcome !== 'string' || card.outcome.length > 4_000)) return null;
    }
  }
  for (const message of value.messages) {
    if (!isRecord(message)) return null;
    if (message.role !== 'owner' && message.role !== 'aegis') return null;
    if (message.turn_id !== undefined && message.turn_id !== null && !isSafeShortText(message.turn_id, 256)) return null;
    if (!isSafeShortText(message.text, 4_000)) return null;
    if (!isFiniteNumber(message.timestamp)) return null;
    if (message.severity !== undefined && message.severity !== null && !isNoticeSeverity(message.severity)) return null;
    if (message.latency_ms !== undefined && !isFiniteNumber(message.latency_ms)) return null;
    if (message.repeat_count !== undefined && !isFiniteNumber(message.repeat_count)) return null;
    if (message.first_timestamp !== undefined && !isFiniteNumber(message.first_timestamp)) return null;
    if (message.work_item_id !== undefined && message.work_item_id !== null && !isSafeShortText(message.work_item_id, 160)) return null;
    if (message.evidence !== undefined) {
      if (!Array.isArray(message.evidence) || message.evidence.length > 16 || !message.evidence.every(isVNextChatHistoryEvidenceItem)) {
        return null;
      }
    }
    if (message.unsupported_numbers !== undefined && !isVNextChatHistoryUnsupportedNumbers(message.unsupported_numbers)) {
      return null;
    }
    if (message.retry_text !== undefined && message.retry_text !== null
      && (typeof message.retry_text !== 'string' || message.retry_text.length > 4_000)) {
      return null;
    }
    if (message.source_evidence !== undefined) {
      if (!Array.isArray(message.source_evidence)
        || message.source_evidence.length > 64
        || !message.source_evidence.every(isOwnerWorkResponseSourceEvidence)) {
        return null;
      }
    }
  }
  return value as unknown as GuardedVNextChatHistoryProjection;
}

export function readVNextAvatarProfileProjection(value: unknown): GuardedVNextAvatarProfileProjection | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (typeof value.aegis_avatar_url !== 'string' || value.aegis_avatar_url.length > 700_000) return null;
  if (typeof value.owner_avatar_url !== 'string' || value.owner_avatar_url.length > 700_000) return null;
  if (!isOptionalFiniteNumberOrNull(value.updated_at_unix)) return null;
  if (value.auto_refresh !== undefined) {
    if (!isRecord(value.auto_refresh)) return null;
    if (typeof value.auto_refresh.enabled !== 'boolean') return null;
    if (!isSafeShortText(value.auto_refresh.provider, 80)) return null;
    if (!isFiniteNumber(value.auto_refresh.interval_days)) return null;
    if (typeof value.auto_refresh.image_generation_available !== 'boolean') return null;
    if (!isSafeShortText(value.auto_refresh.status, 80)) return null;
    if (!isSafeShortText(value.auto_refresh.owner_message, 500)) return null;
    if (value.auto_refresh.owner_next_step !== undefined && value.auto_refresh.owner_next_step !== null && !isSafeShortText(value.auto_refresh.owner_next_step, 500)) return null;
    if (!isOptionalFiniteNumberOrNull(value.auto_refresh.last_refresh_at_unix)) return null;
    if (!isOptionalFiniteNumberOrNull(value.auto_refresh.next_refresh_at_unix)) return null;
    if (typeof value.auto_refresh.prompt_hint !== 'string' || value.auto_refresh.prompt_hint.length > 500) return null;
  }
  return value as unknown as GuardedVNextAvatarProfileProjection;
}

export function readVNextCommandResponse(value: unknown): GuardedVNextCommandResponse | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!isSafeShortText(value.outcome, 80)) return null;
  if (!isSafeShortText(value.owner_message, 4096)) return null;
  if (value.work_item !== undefined && value.work_item !== null && !isVNextWorkItem(value.work_item)) return null;
  if (value.action_draft !== undefined && value.action_draft !== null && !isVNextActionRef(value.action_draft)) return null;
  if (value.permission_request !== undefined && value.permission_request !== null && !isVNextAuthorityEnvelope(value.permission_request)) return null;
  if (value.receipt !== undefined && value.receipt !== null && !isVNextReceiptRef(value.receipt)) return null;
  return value as unknown as GuardedVNextCommandResponse;
}

export function readVNextWorkActionResponse(value: unknown): GuardedVNextWorkActionResponse | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.generated_at, 80)) return null;
  if (!isSafeShortText(value.status, 80)) return null;
  if (!isSafeShortText(value.item_id, 128)) return null;
  if (!isSafeShortText(value.action_id, 120)) return null;
  if (!isSafeShortText(value.owner_message, 4096)) return null;
  if (value.receipt !== undefined && value.receipt !== null && !isVNextReceiptRef(value.receipt)) return null;
  return value as unknown as GuardedVNextWorkActionResponse;
}

export function readVNextDebugSnapshotResponse(value: unknown): GuardedVNextDebugSnapshotResponse | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.id, 128)) return null;
  if (!isFiniteNumber(value.expires_at_unix)) return null;
  return value as unknown as GuardedVNextDebugSnapshotResponse;
}

export function readOwnerWorkContextEnvelope(value: unknown): GuardedOwnerWorkContextEnvelope | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.focus_id, 128)) return null;
  if (!isSafeShortText(value.focus_kind, 32)) return null;
  if (!isSafeShortText(value.focus_source, 64)) return null;
  if (!isVisibleFieldMap(value.visible_fields)) return null;
  if (!isStringArray(value.redacted_fields, 32)) return null;
  if (!isStringArray(value.authorized_sensitive_scopes, 32)) return null;
  if (!isStringArray(value.denied_sensitive_scopes, 64)) return null;
  if (!isStringArray(value.allowed_intents, 8)) return null;
  if (value.required_gate !== undefined && value.required_gate !== null && !isOwnerWorkContextGate(value.required_gate)) return null;
  if (!isStringArray(value.related_action_ids, 16)) return null;
  if (!isStringArray(value.related_receipt_ids, 16)) return null;
  if (value.source_event_id !== undefined && value.source_event_id !== null && !isSafeShortText(value.source_event_id, 128)) return null;
  if (value.trace_id !== undefined && value.trace_id !== null && !isSafeShortText(value.trace_id, 128)) return null;
  if (value.run_id !== undefined && value.run_id !== null && !isSafeShortText(value.run_id, 128)) return null;
  return value as unknown as GuardedOwnerWorkContextEnvelope;
}

export function readOwnerWorkResponse(value: unknown): GuardedOwnerWorkResponse | null {
  if (!isRecord(value)) return null;
  if (!isFiniteNumber(value.schema_version)) return null;
  if (!isSafeShortText(value.response_kind, 64)) return null;
  if (!isSafeShortText(value.visible_answer, 4_096)) return null;
  if (value.severity !== undefined && value.severity !== null && !isNoticeSeverity(value.severity)) return null;
  if (value.draft !== undefined && value.draft !== null && !isSafeShortText(value.draft, 4_096)) return null;
  if (value.action_draft !== undefined && value.action_draft !== null) return null;
  if (value.suggested_intent !== undefined && value.suggested_intent !== null && !isOwnerIntentSuggestion(value.suggested_intent)) return null;
  if (value.authority !== undefined && value.authority !== null && !isOwnerWorkAuthorityEnvelope(value.authority)) return null;
  if (value.required_gate !== undefined && value.required_gate !== null && !isOwnerWorkContextGate(value.required_gate)) return null;
  if (!isSafeShortText(value.receipt_expectation, 512)) return null;
  if (!isStringArray(value.citations, 64)) return null;
  if (value.source_evidence !== undefined
    && (!Array.isArray(value.source_evidence)
      || value.source_evidence.length > 64
      || !value.source_evidence.every(isOwnerWorkResponseSourceEvidence))) return null;
  if (value.evidence !== undefined
    && (!Array.isArray(value.evidence)
      || value.evidence.length > 16
      || !value.evidence.every(isAssistantEvidenceItem))) return null;
  if (value.unsupported_numbers !== undefined
    && (!Array.isArray(value.unsupported_numbers)
      || value.unsupported_numbers.length > 32
      || !value.unsupported_numbers.every((item) => isSafeShortText(item, 64)))) return null;
  if (!isStringArray(value.warnings, 32)) return null;
  if (value.trace_id !== undefined && value.trace_id !== null && !isSafeShortText(value.trace_id, 128)) return null;
  if (value.run_id !== undefined && value.run_id !== null && !isSafeShortText(value.run_id, 128)) return null;
  if (value.provider_request_id !== undefined && value.provider_request_id !== null && !isSafeShortText(value.provider_request_id, 160)) return null;
  return value as unknown as GuardedOwnerWorkResponse;
}

export function readStreamTokenInfo(value: unknown): GuardedStreamTokenInfo | null {
  if (!isRecord(value)) return null;
  if (typeof value.stream_token !== 'string' || value.stream_token.length === 0) return null;
  if (!isFiniteNumber(value.expires_at) || !isFiniteNumber(value.expires_in)) return null;
  return value as unknown as GuardedStreamTokenInfo;
}

export function readMfaStatusResponse(value: unknown): GuardedMfaStatusResponse | null {
  if (!isRecord(value) || typeof value.authenticated !== 'boolean' || !isRecord(value.mfa)) return null;
  const { mfa } = value;
  if (typeof mfa.mfa_level !== 'string') return null;
  if (!isOptionalFiniteNumberOrNull(mfa.mfa_verified_at)) return null;
  if (typeof mfa.requires_mfa_setup !== 'boolean') return null;
  if (!isFiniteNumber(mfa.webauthn_registered_count)) return null;
  if (mfa.webauthn_available !== undefined && typeof mfa.webauthn_available !== 'boolean') return null;
  if (mfa.webauthn_status !== undefined && typeof mfa.webauthn_status !== 'string') return null;
  const webauthnAvailable = typeof mfa.webauthn_available === 'boolean'
    ? mfa.webauthn_available
    : undefined;
  const webauthnStatus = typeof mfa.webauthn_status === 'string'
    ? mfa.webauthn_status
    : undefined;
  const totpConfigured = typeof mfa.totp_configured === 'boolean'
    ? mfa.totp_configured
    : mfa.totp_registered;
  if (typeof totpConfigured !== 'boolean') return null;
  if (mfa.preferred_step_up_method !== undefined && typeof mfa.preferred_step_up_method !== 'string') return null;
  const preferredStepUpMethod = typeof mfa.preferred_step_up_method === 'string'
    ? mfa.preferred_step_up_method
    : undefined;
  if (!isFiniteNumber(mfa.recovery_codes_remaining)) return null;
  return {
    authenticated: value.authenticated,
    mfa: {
      mfa_level: mfa.mfa_level,
      mfa_verified_at: mfa.mfa_verified_at,
      requires_mfa_setup: mfa.requires_mfa_setup,
      webauthn_registered_count: mfa.webauthn_registered_count,
      webauthn_available: webauthnAvailable,
      webauthn_status: webauthnStatus,
      totp_configured: totpConfigured,
      totp_registered: totpConfigured,
      preferred_step_up_method: preferredStepUpMethod,
      recovery_codes_remaining: mfa.recovery_codes_remaining,
    },
  };
}

function isWebAuthnCredentialSummary(value: unknown): value is GuardedWebAuthnCredentialSummary {
  if (!isRecord(value)) return false;
  if (typeof value.id !== 'string' || value.id.length === 0) return false;
  if (!(value.nickname === undefined || value.nickname === null || typeof value.nickname === 'string')) return false;
  if (!Array.isArray(value.transports) || !value.transports.every((transport) => typeof transport === 'string')) return false;
  if (!isOptionalBooleanOrNull(value.backup_eligible)) return false;
  if (!isOptionalBooleanOrNull(value.backup_state)) return false;
  if (!isFiniteNumber(value.created_at)) return false;
  if (!isOptionalFiniteNumberOrNull(value.last_used_at)) return false;
  return true;
}

export function readWebAuthnCredentialsResponse(value: unknown): GuardedWebAuthnCredentialSummary[] | null {
  if (!isRecord(value) || !Array.isArray(value.credentials)) return null;
  if (!value.credentials.every(isWebAuthnCredentialSummary)) return null;
  return value.credentials;
}

export function readTotpSetupResponse(value: unknown): GuardedTotpSetupResponse | null {
  if (!isRecord(value) || value.status !== 'ok' || value.method !== 'totp' || !isRecord(value.setup)) return null;
  const { setup } = value;
  if (!isSafeTotpSecret(setup.secret_base32)) return null;
  if (!isSafeTotpSetupUrl(setup.otpauth_url, setup.secret_base32)) return null;
  if (!isSafeShortText(setup.issuer, 80)) return null;
  if (!isSafeShortText(setup.account_name, 254)) return null;
  if (setup.digits !== 6 || setup.period_secs !== 30) return null;
  if (setup.algorithm !== 'SHA256') return null;
  return value as unknown as GuardedTotpSetupResponse;
}

export function readConsoleApprovalActionResponse(value: unknown): GuardedConsoleApprovalActionResponse | null {
  if (!isRecord(value) || value.status !== 'ok') return null;
  if (typeof value.approval_id !== 'string' || value.approval_id.length === 0) return null;
  if (value.action !== 'approve' && value.action !== 'reject') return null;
  if (typeof value.risk !== 'string' || value.risk.length === 0) return null;
  if (typeof value.approval_type !== 'string' || value.approval_type.length === 0) return null;
  if (typeof value.message !== 'string') return null;
  return value as unknown as GuardedConsoleApprovalActionResponse;
}

export function readConsoleMandateLifecycleResponse(value: unknown): GuardedConsoleMandateLifecycleResponse | null {
  if (!isRecord(value) || value.status !== 'ok') return null;
  if (!isConsoleMandateSummary(value.mandate)) return null;
  if (!isSafeShortText(value.receipt_expectation, 512)) return null;
  return value as unknown as GuardedConsoleMandateLifecycleResponse;
}

export function readConsoleMemoryDeleteResponse(value: unknown): GuardedConsoleMemoryDeleteResponse | null {
  if (!isRecord(value) || value.status !== 'ok' || value.resource_type !== 'memory_item') return null;
  if (typeof value.resource_id !== 'string' || value.resource_id.length === 0) return null;
  if (value.deleted !== true) return null;
  if (!isFiniteNumber(value.ledger_entry_id)) return null;
  if (typeof value.receipt !== 'string' || value.receipt.length === 0) return null;
  return value as unknown as GuardedConsoleMemoryDeleteResponse;
}

export function readConsoleMemoryUpdateResponse(value: unknown): GuardedConsoleMemoryUpdateResponse | null {
  if (!isRecord(value) || value.status !== 'ok' || value.resource_type !== 'memory_item') return null;
  if (typeof value.resource_id !== 'string' || value.resource_id.length === 0) return null;
  if (value.updated !== true) return null;
  if (!isFiniteNumber(value.ledger_entry_id)) return null;
  if (typeof value.receipt !== 'string' || value.receipt.length === 0) return null;
  return value as unknown as GuardedConsoleMemoryUpdateResponse;
}

export function readConsoleProfileUpdateResponse(value: unknown): GuardedConsoleProfileUpdateResponse | null {
  if (!isRecord(value) || value.status !== 'ok' || value.resource_type !== 'profile_slot') return null;
  if (typeof value.resource_id !== 'string' || value.resource_id.length === 0) return null;
  if (value.updated !== true) return null;
  if (!isFiniteNumber(value.ledger_entry_id)) return null;
  if (typeof value.receipt !== 'string' || value.receipt.length === 0) return null;
  return value as unknown as GuardedConsoleProfileUpdateResponse;
}
