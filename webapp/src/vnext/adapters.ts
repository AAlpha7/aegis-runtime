import type {
  ContextualAssistantResponse,
  OwnerWorkAction,
  OwnerWorkItem,
  OwnerWorkQueue,
  VNextActionRef,
  VNextCommandResponse,
  VNextProjectedWorkItem,
  VNextWorkProjection,
} from '../api';
import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import type { VNextItemActionId } from './model';

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

function isClosedOwnerQueueItem(item: OwnerWorkItem): boolean {
  const status = item.status.trim().toLowerCase();
  const urgency = item.urgency.trim().toLowerCase();
  return urgency === 'silent'
    || ['done', 'executed', 'fired', 'cancelled', 'canceled', 'archived', 'deleted', 'completed', 'abandoned'].includes(status);
}

function hasMultipleRollupTopics(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  const separatorCount = (trimmed.match(/\//g) ?? []).length
    + (trimmed.match(/\+/g) ?? []).length
    + (trimmed.match(/[;；]/g) ?? []).length;
  return separatorCount >= 2;
}

function isAmbiguousStatusRollupText(title: string, text: string): boolean {
  const normalized = `${title} ${text}`.toLowerCase();
  if (title.includes('：') && hasMultipleRollupTopics(title)) return true;
  return normalized.includes('日终状态')
    || normalized.includes('evening status')
    || normalized.includes('daily status')
    || normalized.includes('status rollup')
    || normalized.includes('status summary')
    || normalized.includes('fuel exhausted')
    || normalized.includes('fuelexhausted')
    || normalized.includes('断路器')
    || normalized.includes('circuit breaker');
}

function isAmbiguousStatusRollupVNextItem(item: VNextProjectedWorkItem): boolean {
  return isAmbiguousStatusRollupText(item.title, [
    item.summary,
    item.why_it_matters,
    item.next_safe_action,
    item.blocked_by,
    item.route_reason,
    item.source_kind,
    item.source,
  ].filter(Boolean).join(' '));
}

export function contextualResponseFromVNext(response: VNextCommandResponse): ContextualAssistantResponse {
  const raw = response.assistant_response;
  if (isRecord(raw) && typeof raw.visible_answer === 'string') {
    const warnings = Array.isArray(raw.warnings) && raw.warnings.every((warning) => typeof warning === 'string')
      ? raw.warnings
      : [];
    const citations = Array.isArray(raw.citations) && raw.citations.every((citation) => typeof citation === 'string')
      ? raw.citations
      : [];
    const sourceEvidence = Array.isArray(raw.source_evidence) && raw.source_evidence.every(isSourceEvidence)
      ? raw.source_evidence
      : undefined;
    const evidence = Array.isArray(raw.evidence) && raw.evidence.every(isClaimEvidence)
      ? raw.evidence.slice(0, 16)
      : undefined;
    const unsupportedNumbers = Array.isArray(raw.unsupported_numbers)
      && raw.unsupported_numbers.length <= 32
      && raw.unsupported_numbers.every((item) => typeof item === 'string')
      ? raw.unsupported_numbers
      : undefined;
    return {
      schema_version: typeof raw.schema_version === 'number' ? raw.schema_version : 1,
      response_kind: typeof raw.response_kind === 'string' ? raw.response_kind : response.outcome,
      visible_answer: raw.visible_answer,
      ...(typeof raw.turn_id === 'string' && raw.turn_id.trim() && raw.turn_id.length <= 256
        ? { turn_id: raw.turn_id.trim() } : {}),
      ...(typeof raw.latency_ms === 'number' && Number.isFinite(raw.latency_ms) && raw.latency_ms >= 0
        ? { latency_ms: raw.latency_ms } : {}),
      severity: isNoticeSeverity(raw.severity) ? raw.severity : null,
      draft: typeof raw.draft === 'string' ? raw.draft : null,
      action_draft: raw.action_draft,
      suggested_intent: isRecord(raw.suggested_intent) ? raw.suggested_intent as ContextualAssistantResponse['suggested_intent'] : null,
      authority: isRecord(raw.authority) ? raw.authority as unknown as ContextualAssistantResponse['authority'] : null,
      required_gate: isRecord(raw.required_gate) ? raw.required_gate as unknown as ContextualAssistantResponse['required_gate'] : null,
      receipt_expectation: typeof raw.receipt_expectation === 'string'
        ? raw.receipt_expectation
        : 'No external action was executed; receipts are required before completion claims.',
      citations,
      source_evidence: sourceEvidence,
      ...(evidence && evidence.length > 0 ? { evidence } : {}),
      ...(unsupportedNumbers && unsupportedNumbers.length > 0 ? { unsupported_numbers: unsupportedNumbers } : {}),
      warnings,
      trace_id: typeof raw.trace_id === 'string' ? raw.trace_id : null,
      run_id: typeof raw.run_id === 'string' ? raw.run_id : null,
      provider_request_id: typeof raw.provider_request_id === 'string' ? raw.provider_request_id : null,
    };
  }

  return {
    schema_version: 1,
    response_kind: response.outcome,
    visible_answer: response.owner_message,
    severity: null,
    draft: null,
    suggested_intent: null,
    authority: null,
    required_gate: null,
    receipt_expectation: response.receipt?.label ?? 'No external action was executed; receipts are required before completion claims.',
    citations: [],
    source_evidence: [],
    warnings: [],
  };
}

function isNoticeSeverity(value: unknown): value is NonNullable<ContextualAssistantResponse['severity']> {
  return value === 'info'
    || value === 'notice'
    || value === 'important'
    || value === 'warning'
    || value === 'critical';
}

function isJevField(value: unknown): boolean {
  if (value === undefined || value === null) return true;
  if (!isRecord(value) || typeof value.verdict !== 'string') return false;
  const verdict = value.verdict.trim().toLowerCase();
  return verdict === 'allow' || verdict === 'ask' || verdict === 'deny';
}

function isSourceEvidence(value: unknown): value is NonNullable<ContextualAssistantResponse['source_evidence']>[number] {
  return isRecord(value)
    && typeof value.kind === 'string'
    && typeof value.label === 'string'
    && typeof value.summary === 'string'
    && typeof value.detail === 'string'
    && isJevField(value.jev);
}

function isClaimEvidence(value: unknown): value is NonNullable<ContextualAssistantResponse['evidence']>[number] {
  return isRecord(value)
    && typeof value.id === 'string'
    && typeof value.tool === 'string'
    && typeof value.label === 'string'
    && typeof value.excerpt === 'string'
    && typeof value.full_len === 'number'
    && Number.isFinite(value.full_len)
    && value.full_len >= 0;
}

export function actionSuccessMessage(actionId: VNextItemActionId, locale: Locale = 'en'): string {
  if (actionId === 'goal.abandon') return t('work.success.goalClosed', locale);
  if (actionId === 'tracking.cancel') return t('work.success.trackingClosed', locale);
  if (actionId === 'task.skip_occurrence') return t('work.success.occurrenceSkipped', locale);
  if (actionId.startsWith('task.')) return t('work.success.task', locale);
  if (actionId.startsWith('approval.')) return t('work.success.approval', locale);
  if (actionId.startsWith('memory.')) return t('work.success.memory', locale);
  return t('work.success.generic', locale);
}

export function ownerKindFromVNext(item: VNextProjectedWorkItem): string {
  if (item.source_kind === 'reminder') return 'task';
  if (item.source_kind === 'tracking' || item.source_kind === 'backlog' || item.source_kind === 'decision') return 'task';
  return item.source_kind;
}

export function ownerActionFromVNext(action: VNextActionRef): OwnerWorkAction {
  return {
    label: action.label,
    label_code: action.label_code ?? null,
    action_kind: action.kind,
    route: action.route,
    requires_gate: action.requires_data_gate,
    requires_mfa: action.requires_mfa,
  };
}

export function ownerItemFromVNext(item: VNextProjectedWorkItem): OwnerWorkItem {
  return {
    id: item.id,
    kind: ownerKindFromVNext(item),
    source_kind: item.source_kind,
    title: item.title,
    title_code: item.title_code ?? null,
    summary: item.summary,
    owner_visible_reason: item.why_it_matters,
    owner_visible_reason_code: item.why_it_matters_code ?? null,
    authority_basis: item.authority.basis,
    source: item.source,
    source_id: item.source_id,
    source_event_id: item.source_event_id,
    status: item.state,
    urgency: item.urgency,
    sensitivity: item.sensitivity,
    privacy_level: item.sensitivity === 'S0' || item.sensitivity === 'S1' ? 'metadata' : 'private',
    created_at: item.created_at,
    updated_at: item.updated_at,
    due_at: item.due_at,
    primary_action: item.recommended_action ? ownerActionFromVNext(item.recommended_action) : null,
    secondary_actions: item.contextual_actions
      .filter((action) => !action.kind.startsWith('draft.'))
      .map(ownerActionFromVNext),
    next_safe_action: item.next_safe_action,
    next_safe_action_code: item.next_safe_action_code ?? null,
    blocked_by: item.blocked_by,
    blocked_by_code: item.blocked_by_code ?? null,
    receipt_expectation: item.receipt_expectation ?? item.authority.expected_receipt ?? undefined,
    authority: {
      item_id: item.id,
      authority_basis: item.authority.basis,
      visible_fields: item.authority.visible_data,
      locked_fields: item.authority.locked_data,
      allowed_now: item.authority.allowed_now,
      requires_data_gate: item.authority.requires_data_grant,
      requires_mfa: item.authority.requires_mfa,
      requires_approval: item.authority.requires_approval,
      expected_receipt: item.authority.expected_receipt ?? item.receipt_expectation ?? 'A receipt is required before Aegis claims completion.',
      source_mandate_id: item.authority.source_mandate_id,
      audit_links: item.authority.audit_refs,
    },
    detail_route: item.detail_route,
    external_url_policy: item.external_url_policy,
    related_action_ids: item.related_work_item_ids,
    related_receipt_ids: item.related_receipt_ids,
    route_reason: item.route_reason,
    schema_version: 1,
    decision_request: item.decision_request ?? null,
    lineage: Array.isArray(item.lineage) && item.lineage.length > 0 ? item.lineage : undefined,
  };
}

export function ownerQueueFromVNextWork(work: VNextWorkProjection): OwnerWorkQueue {
  const items = work.lanes.flatMap((lane) => lane.items
    .filter((item) => item.source_kind !== 'action_draft')
    .filter((item) => !isAmbiguousStatusRollupVNextItem(item))
    .map(ownerItemFromVNext)
    .filter((item) => !isClosedOwnerQueueItem(item)));
  return {
    schema_version: 1,
    generated_at: work.generated_at,
    owner: 'owner',
    sources: work.sources,
    counts: work.counts,
    items,
  };
}
