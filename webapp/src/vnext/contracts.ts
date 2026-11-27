export type SecretaryLaneId = 'decide' | 'now' | 'follow_up' | 'backlog' | 'done';
export type SecretarySensitivity = 'S0' | 'S1' | 'S2' | 'S3' | 'S4';
export type SecretarySourceKind =
  | 'calendar'
  | 'reminder'
  | 'task'
  | 'tracking'
  | 'decision'
  | 'backlog'
  | 'receipt'
  | 'mandate'
  | 'memory'
  | 'system';
export type SecretaryAuthorityBasis =
  | 'direct_owner_request'
  | 'active_mandate'
  | 'system_health'
  | 'manual_review';
export type SecretaryCommandOutcomeKind =
  | 'answer_now'
  | 'open_work_item'
  | 'create_work_item'
  | 'draft_action'
  | 'request_clarification'
  | 'request_permission'
  | 'create_reminder'
  | 'create_tracking'
  | 'create_backlog_item'
  | 'propose_workflow_change'
  | 'show_status';
export type SecretaryCommandIntentKind = 'chat' | 'explain' | 'draft';

export const VNEXT_PHASE0_SURFACES = ['home', 'work', 'item', 'command', 'status', 'settings'] as const;
export const VNEXT_WORK_LANES = ['decide', 'now', 'follow_up', 'backlog', 'done'] as const;
export const VNEXT_API_PROJECTIONS = [
  'GET /webapp/vnext/home',
  'GET /webapp/vnext/work',
  'GET /webapp/vnext/work/:id',
  'POST /webapp/vnext/command',
  'POST /webapp/vnext/work/:id/actions',
  'GET /webapp/vnext/status',
  'GET /webapp/vnext/settings',
  'POST /webapp/vnext/settings/cost-saving/pause',
  'POST /webapp/vnext/settings/cost-saving/resume',
  'GET /webapp/vnext/settings/web-push/status',
  'POST /webapp/vnext/settings/web-push/subscribe',
  'POST /webapp/vnext/settings/web-push/unsubscribe',
  'POST /webapp/vnext/settings/web-push/test',
  'GET /webapp/vnext/debug-leases',
  'POST /webapp/vnext/debug-leases',
  'DELETE /webapp/vnext/debug-leases/:id',
  'GET /webapp/vnext/debug-jobs',
  'POST /webapp/vnext/debug-jobs',
  'GET /webapp/vnext/debug-jobs/:id',
] as const;

export interface SecretaryReceiptRef {
  id: string;
  kind: string;
  label: string;
  created_at: string;
}

export interface SecretaryActionRef {
  id: string;
  label: string;
  /**
   * Optional stable structural code for `label` (i18n catalog key
   * `work.actionLabel.<code>`). Absent/unknown codes fall back to `label`.
   */
  label_code?: string | null;
  kind: string;
  route?: string | null;
  requires_confirmation: boolean;
  requires_mfa: boolean;
  requires_data_gate: boolean;
  expected_receipt: string;
}

export interface SecretaryAuthorityEnvelope {
  basis: SecretaryAuthorityBasis;
  owner_visible_reason: string;
  visible_data: string[];
  locked_data: string[];
  allowed_now: string[];
  requires_mfa: boolean;
  requires_approval: boolean;
  requires_data_grant: boolean;
  expected_receipt: string | null;
  source_mandate_id?: string | null;
  audit_refs: string[];
}

export interface SecretaryWorkItem {
  id: string;
  lane: SecretaryLaneId;
  source_kind: SecretarySourceKind;
  source: string;
  source_id: string;
  source_event_id?: string | null;
  title: string;
  /** Optional structural code for `title` (`work.decide.<code>`). */
  title_code?: string | null;
  summary: string;
  why_it_matters: string;
  /**
   * Optional structural code for `why_it_matters` (`work.reasonCode.<code>`);
   * absent/unknown codes fall back to the prose.
   */
  why_it_matters_code?: string | null;
  state: string;
  urgency: string;
  sensitivity: SecretarySensitivity;
  created_at: string;
  updated_at: string;
  due_at?: string | null;
  authority: SecretaryAuthorityEnvelope;
  recommended_action: SecretaryActionRef | null;
  contextual_actions: SecretaryActionRef[];
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
  receipt_expectation: string | null;
}

export interface SecretaryHomeProjection {
  schema_version: number;
  generated_at: string;
  brief: string;
  top_now: SecretaryWorkItem[];
  decision_previews: SecretaryWorkItem[];
  follow_up_highlights: SecretaryWorkItem[];
  status_entry: SecretaryStatusSummary;
}

export interface SecretaryWorkProjection {
  schema_version: number;
  generated_at: string;
  lanes: Array<{
    id: SecretaryLaneId;
    title: string;
    collapsed: boolean;
    items: SecretaryWorkItem[];
  }>;
}

export interface SecretaryItemProjection {
  schema_version: number;
  generated_at: string;
  item: SecretaryWorkItem;
  safe_source_detail: string;
  history: SecretaryReceiptRef[];
  collaboration: {
    focus_id: string;
    placeholder: string;
    suggested_prompts: string[];
  };
}

export interface SecretaryCommandRequest {
  schema_version?: number;
  input: string;
  mode: 'text' | 'voice';
  intent_kind?: SecretaryCommandIntentKind;
  focus_item_id?: string | null;
  client_context: 'home' | 'work' | 'item' | 'status' | 'settings';
  idempotency_key?: string | null;
}

export interface SecretaryCommandResponse {
  schema_version: number;
  generated_at: string;
  outcome: SecretaryCommandOutcomeKind;
  owner_message: string;
  work_item?: SecretaryWorkItem | null;
  permission_request?: SecretaryAuthorityEnvelope | null;
  receipt?: SecretaryReceiptRef | null;
}

/** Per-turn inspectable grounding on a Web console assistant reply. */
export interface SecretaryAssistantEvidenceItem {
  id: string;
  tool: string;
  label: string;
  excerpt: string;
  full_len: number;
}

export interface SecretaryWorkActionRequest {
  schema_version?: number;
  action_id: string;
  idempotency_key?: string | null;
}

export interface SecretaryStatusSummary {
  label: string;
  health: 'ready' | 'attention' | 'quiet' | 'degraded';
  owner_message: string;
  owner_next_step?: string | null;
}

export interface SecretaryStatusProjection {
  schema_version: number;
  generated_at: string;
  sections: SecretaryStatusSummary[];
  advanced_diagnostics_available: boolean;
}

export interface SecretarySettingsProjection {
  schema_version: number;
  signed_in_owner: string;
  connected_services: SecretaryStatusSummary[];
  security: SecretaryStatusSummary[];
  notification_preferences: SecretaryStatusSummary[];
  advanced_diagnostics_available: boolean;
}
