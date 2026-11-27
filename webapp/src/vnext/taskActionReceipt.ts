import type { VNextWorkActionResponse } from '../api';
import type { Locale } from '../i18n/catalog';
import { t, tf } from '../i18n/catalog';
import { actionSuccessMessage } from './adapters';
import type { VNextItemActionId } from './model';
import { formatOwnerFacingInstants } from './sourceEvidenceText';

export type OwnerTaskActionReceiptInput = {
  action: string;
  dueAt?: string | null;
  recurring?: boolean;
};

/**
 * Owner-facing receipt for a console task action.
 * The ledger summary stays an English audit sentence (`Task #N marked complete`).
 * This function never reads that sentence. It uses the structured action,
 * the recurring flag, and the due instant.
 */
export function ownerTaskActionReceipt(
  input: OwnerTaskActionReceiptInput,
  locale: Locale,
  userTimezone?: string | null,
): string {
  if (input.action === 'complete' && input.recurring) return t('app.taskAction.completeLoop', locale);
  if (input.action === 'complete') return t('app.taskAction.complete', locale);
  if (input.action === 'delete') return t('app.taskAction.archived', locale);
  if (input.action === 'snooze') {
    const due = input.dueAt ? formatOwnerFacingInstants(input.dueAt, userTimezone) : '';
    if (due && !/\d{4}-\d{2}-\d{2}T/.test(due)) return tf('app.taskAction.snoozed', locale, { due });
    return t('app.reminderUpdated', locale);
  }
  return t('app.reminderUpdated', locale);
}

function consoleTaskActionFields(raw: unknown): OwnerTaskActionReceiptInput | null {
  if (!raw || typeof raw !== 'object') return null;
  const record = raw as Record<string, unknown>;
  const action = typeof record.action === 'string' ? record.action : '';
  if (action !== 'complete' && action !== 'snooze' && action !== 'delete') return null;
  return {
    action,
    dueAt: typeof record.due_at === 'string' ? record.due_at : null,
    recurring: record.recurring === true,
  };
}

/** Work-item actions that share the console task endpoint. */
export function ownerFacingWorkActionMessage(
  actionId: VNextItemActionId,
  response: Pick<VNextWorkActionResponse, 'owner_message' | 'raw_result'>,
  locale: Locale,
  userTimezone?: string | null,
): string {
  const structured = consoleTaskActionFields(response.raw_result);
  if (structured) return ownerTaskActionReceipt(structured, locale, userTimezone);
  const server = response.owner_message?.trim() ?? '';
  return server || actionSuccessMessage(actionId, locale);
}
