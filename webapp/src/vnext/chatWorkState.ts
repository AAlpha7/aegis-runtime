import type { OwnerWorkItem, OwnerWorkQueue } from '../api';
import { t, type Locale } from '../i18n/catalog';

export type ChatWorkStateMessage = {
  workItem?: OwnerWorkItem | null;
  workItemAnchorId?: string | null;
  workItemThread?: unknown[];
  workItemUnavailable?: boolean;
};

const CLOSED_CHAT_WORK_STATUSES = new Set(['done', 'executed', 'fired', 'cancelled', 'canceled', 'archived', 'deleted', 'completed', 'abandoned']);

function isClosedChatWorkStatus(status: string | null | undefined): boolean {
  return CLOSED_CHAT_WORK_STATUSES.has(status?.trim().toLowerCase() ?? '');
}

export function updateChatWorkMessagesAfterItemStatus<T extends ChatWorkStateMessage>(
  messages: T[],
  itemId: string,
  status: string,
): T[] {
  return messages.map((message) => (
    message.workItem?.id === itemId
      ? {
        ...message,
        workItem: { ...message.workItem, status },
        workItemThread: [],
        workItemUnavailable: false,
      }
      : message
  ));
}

// Module-level pure fn (no React context): the owner's display_language locale
// is threaded in as an explicit param, mirroring sourceDetailPayloadFromOpenError.
export function syncChatWorkMessagesWithQueue<T extends ChatWorkStateMessage>(
  messages: T[],
  nextQueue: OwnerWorkQueue,
  locale: Locale,
): T[] {
  const itemsById = new Map(nextQueue.items.map((item) => [item.id, item]));
  return messages.map((message) => {
    if (!message.workItem && message.workItemAnchorId) {
      const anchoredItem = itemsById.get(message.workItemAnchorId);
      if (anchoredItem) {
        return {
          ...message,
          workItem: anchoredItem,
          workItemThread: [],
          workItemUnavailable: false,
        };
      }
    }
    if (!message.workItem) return message;
    const nextItem = itemsById.get(message.workItem.id);
    if (!nextItem) {
      if (isClosedChatWorkStatus(message.workItem.status)) {
        return {
          ...message,
          workItemUnavailable: false,
        };
      }
      return {
        ...message,
        workItem: {
          ...message.workItem,
          // 'done' is a wire-enum identity value (compared by
          // CLOSED_CHAT_WORK_STATUSES and downstream status logic) — never
          // localize it. Only the display strings below localize.
          status: 'done',
          summary: t('chatWork.itemChangedSummary', locale),
          next_safe_action: t('chatWork.itemChangedNextAction', locale),
          // The replacement text above is already localized; a stale
          // next_safe_action_code from the original item would shadow it in
          // code-aware renderers, so clear it explicitly.
          next_safe_action_code: null,
          secondary_actions: [],
        },
        workItemThread: [],
        workItemUnavailable: true,
      };
    }
    const statusChanged = nextItem.status !== message.workItem.status;
    return {
      ...message,
      workItem: nextItem,
      workItemThread: statusChanged ? [] : message.workItemThread,
      workItemUnavailable: false,
    };
  });
}
