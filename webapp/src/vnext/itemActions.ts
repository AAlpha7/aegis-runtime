import {
  fetchConsoleLedgerEntry,
  fetchConsoleMemoryItem,
  fetchConsoleTaskTimeline,
  submitVNextWorkAction,
} from '../api';
import type { OwnerWorkItem, VNextWorkActionResponse } from '../api';
import type { Locale } from '../i18n/catalog';
import { ownerFacingWorkActionMessage } from './taskActionReceipt';
import { isApprovalItem, isTaskLikeItem } from './model';
import type { VNextItemAction } from './model';

export type SourceDetailResult =
  | { kind: 'detail'; detail: unknown }
  | { kind: 'message'; message: string };

export async function openSelectedSourceDetail({
  item,
  initData,
  csrfToken,
}: {
  item: OwnerWorkItem;
  initData: string;
  csrfToken?: string;
}): Promise<SourceDetailResult> {
  if (isTaskLikeItem(item)) {
    return { kind: 'detail', detail: await fetchConsoleTaskTimeline(item.source_id, initData, csrfToken) };
  }
  if (isApprovalItem(item)) {
    return {
      kind: 'detail',
      detail: {
        status: 'ok',
        resource_type: 'approval_review',
        detail: {
          title: item.title,
          title_code: item.title_code ?? undefined,
          summary: item.summary,
          status: item.status,
          urgency: item.urgency,
          sensitivity: item.sensitivity,
          owner_visible_reason: item.owner_visible_reason,
          owner_visible_reason_code: item.owner_visible_reason_code ?? undefined,
          next_safe_action: item.next_safe_action,
          next_safe_action_code: item.next_safe_action_code ?? undefined,
          blocked_by: item.blocked_by,
          blocked_by_code: item.blocked_by_code ?? undefined,
          receipt_expectation: item.receipt_expectation,
          requires_mfa: item.secondary_actions.some((action) => action.requires_mfa),
        },
      },
    };
  }
  if (item.kind === 'receipt' || item.kind === 'action_draft') {
    return { kind: 'detail', detail: await fetchConsoleLedgerEntry(item.source_id, initData, csrfToken) };
  }
  if (item.kind === 'memory_review' || item.source_kind === 'memory' || item.source === 'memory') {
    return { kind: 'detail', detail: await fetchConsoleMemoryItem(item.source_id, initData, csrfToken) };
  }
  return { kind: 'detail', detail: ownerWorkItemSafeDetail(item) };
}

function ownerWorkItemSafeDetail(item: OwnerWorkItem): unknown {
  return {
    status: 'ok',
    resource_type: 'owner_work_item',
    detail: {
      title: item.title,
      title_code: item.title_code ?? undefined,
      summary: item.summary,
      kind: item.kind,
      status: item.status,
      urgency: item.urgency,
      sensitivity: item.sensitivity,
      privacy_level: item.privacy_level,
      owner_visible_reason: item.owner_visible_reason,
      owner_visible_reason_code: item.owner_visible_reason_code ?? undefined,
      next_safe_action: item.next_safe_action,
      next_safe_action_code: item.next_safe_action_code ?? undefined,
      blocked_by: item.blocked_by,
      blocked_by_code: item.blocked_by_code ?? undefined,
      receipt_expectation: item.receipt_expectation,
      primary_action: item.primary_action?.label ?? '',
      primary_action_code: item.primary_action?.label_code ?? undefined,
      // Parallel to available_actions: the structural label code per action
      // ('' when the action has none) so the detail renderer can localize.
      available_actions: item.secondary_actions.map((action) => action.label).filter(Boolean),
      available_action_codes: item.secondary_actions
        .filter((action) => Boolean(action.label))
        .map((action) => action.label_code ?? ''),
      related_actions_count: item.related_action_ids.length,
      related_receipts_count: item.related_receipt_ids.length,
    },
  };
}

export async function executeSelectedItemAction({
  item,
  action,
  initData,
  csrfToken,
  locale = 'en',
}: {
  item: OwnerWorkItem;
  action: VNextItemAction;
  initData: string;
  csrfToken?: string;
  locale?: Locale;
}): Promise<VNextWorkActionResponse> {
  const response = await submitVNextWorkAction(initData, csrfToken, item.id, action.id);
  return {
    ...response,
    owner_message: ownerFacingWorkActionMessage(action.id, response, locale),
  };
}
