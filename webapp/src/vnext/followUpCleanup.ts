/**
 * Follow-up cleanup helpers.
 *
 * Owner report: tapping "清理" only opened an assistant chat that re-asked for
 * confirmation forever (清理 → 是 → 确认清理吗?) without calling knowledge.done.
 * Close must be a real item action with a receipt, not a conversational loop.
 */
import type { OwnerWorkItem } from '../api';
import type { Locale } from '../i18n/catalog';
import { t, tf } from '../i18n/catalog';
import {
  buildSupportedItemActions,
  isClosedItem,
  itemIsBlockedByGate,
  laneForItem,
  ownerVisibleWorkTitle,
} from './model';
import type { VNextItemAction } from './model';
import type { InlineAssistantTurn } from './InlineAssistantPanel';

export function isFollowUpTrackingItem(item: OwnerWorkItem): boolean {
  const status = item.status?.trim().toLowerCase() ?? '';
  return laneForItem(item) === 'follow_up' || status === 'waiting' || status === 'tracking';
}

/** Preferred close action for a waiting/tracking follow-up (receipt-backed). */
export function followUpCloseAction(item: OwnerWorkItem, locale: Locale = 'en'): VNextItemAction | null {
  if (!isFollowUpTrackingItem(item) || isClosedItem(item) || itemIsBlockedByGate(item)) return null;
  const actions = buildSupportedItemActions(item, locale);
  return actions.find((action) => (
    action.id === 'tracking.cancel'
    || action.id === 'goal.abandon'
    || action.id === 'knowledge.done'
    || action.id === 'task.complete'
  )) ?? null;
}

export function isStrongFollowUpCleanupCommand(text: string): boolean {
  const value = text.trim();
  if (!value) return false;
  return /^(清理|关闭|完成|关掉|确认清理|直接清理|清理它|关闭它|关掉它|clean\s*up|close|done|complete)$/i.test(value);
}

export function isOwnerCleanupAffirmation(text: string): boolean {
  const value = text.trim();
  if (!value) return false;
  return /^(是|对|好|确认|是的|好的|可以|行|嗯|yes|y|ok|okay|confirm)$/i.test(value);
}

export function isCannedFollowUpCleanupPrompt(text: string, locale: Locale = 'en'): boolean {
  const value = text.trim();
  if (!value) return false;
  // Match catalog cleanup prompt stems (zh + en) even when title is interpolated.
  if (value.includes('检查这个跟进项是否已过时') || value.includes('不丢失有用历史的情况下清理')) return true;
  if (/clean it up without losing useful history/i.test(value)) return true;
  if (/check whether this follow-up is stale or duplicated/i.test(value)) return true;
  // Exact template with empty title edge cases.
  const zh = tf('work.followUp.cleanup.prompt', locale === 'zh' ? 'zh' : locale, { title: '' });
  const en = tf('work.followUp.cleanup.prompt', 'en', { title: '' });
  if (zh && value.startsWith(zh.replace('：', '').slice(0, 12))) return true;
  if (en && value.toLowerCase().startsWith(en.slice(0, 24).toLowerCase())) return true;
  return false;
}


/**
 * When true, the owner text should close the follow-up via item action instead of
 * another assistant round that only re-asks for confirmation.
 */
export function shouldExecuteFollowUpCleanup(
  item: OwnerWorkItem,
  text: string,
  turns: InlineAssistantTurn[] = [],
  locale: Locale = 'en',
): boolean {
  if (!followUpCloseAction(item, locale)) return false;
  if (isCannedFollowUpCleanupPrompt(text, locale)) return true;
  if (isStrongFollowUpCleanupCommand(text)) return true;
  // A bare 好/嗯/ok used to execute the close whenever any of the last six
  // turns mentioned 跟进/确认 — near-certain on a follow-up item — with no
  // confirm and no undo. Acknowledgements are for the assistant, not the item.
  void turns;
  return false;
}

export type FollowUpQuickControl =
  | {
      id: 'followup.update';
      kind: 'prompt';
      label: string;
      prompt: string;
    }
  | {
      id: 'followup.cleanup';
      kind: 'close_action';
      label: string;
      action: VNextItemAction;
    }
  | {
      id: 'followup.cleanup';
      kind: 'prompt';
      label: string;
      prompt: string;
    };

/**
 * Quiet strip next to actions: only "添加更新".
 * Close/cleanup is the primary receipt action (Done/清理) — not a second chip.
 */
export function followUpQuickControls(item: OwnerWorkItem, locale: Locale): FollowUpQuickControl[] {
  if (!isFollowUpTrackingItem(item) || isClosedItem(item) || itemIsBlockedByGate(item)) return [];
  const title = ownerVisibleWorkTitle(item, t('work.title.thisFollowUp', locale), 140, locale);
  const controls: FollowUpQuickControl[] = [
    {
      id: 'followup.update',
      kind: 'prompt',
      label: t('work.followUp.addUpdate', locale),
      prompt: tf('work.followUp.update.prompt', locale, { title }),
    },
  ];
  // No close_action chip when a receipt-backed close already exists in the
  // primary action dock — dual "完成"+"清理" made the panel noisy.
  if (!followUpCloseAction(item, locale)) {
    controls.push({
      id: 'followup.cleanup',
      kind: 'prompt',
      label: t('work.followUp.cleanUp', locale),
      prompt: tf('work.followUp.cleanup.prompt', locale, { title }),
    });
  }
  return controls;
}

/** True when receipt/policy copy is generic boilerplate (no owner value). */
export function isGenericFollowUpReceiptExpectation(text: string | null | undefined): boolean {
  const value = (text || '').trim().toLowerCase();
  if (!value) return true;
  return (
    value.includes('follow-up actions should produce')
    || value.includes('before aegis claims completion')
    || value.includes('跟进操作应在')
    || value.includes('声称完成之前')
    // Server default for a receipt item (console.receipt.expect.missing):
    // restates the rule, says nothing about this action.
    || value.includes('receipt-like evidence is still required')
    || value.includes('还缺像记录一样的证据')
  );
}

/** Generic "why" that only restates the lane — hide in the intent grid. */
export function isGenericFollowUpReason(text: string | null | undefined): boolean {
  const value = (text || '').trim().toLowerCase();
  if (!value) return true;
  return (
    value.includes('open follow-up loop')
    || value.includes('keeping this visible as a follow-up')
    || value.includes('part of an open follow-up')
    || value.includes('waiting on assistant')
    || value.includes('waiting on user')
    || value.includes('继续确认')
    || value.includes('开放的跟进')
    || value.includes('作为跟进')
    // reason_code action_state_inspect: describes what a receipt item is,
    // not why this one needs the owner.
    || value.includes('state to inspect')
    || value.includes('有记录的状态，可查看')
  );
}
