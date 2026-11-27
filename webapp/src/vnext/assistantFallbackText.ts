/**
 * Text sent when the owner typed nothing.
 *
 * Only the item-page quick actions (explain / draft) have a canned question. A
 * general chat turn with an attachment must reach the runtime with an EMPTY
 * input so the server derives the message from the attachment (voice
 * transcript, image caption, document filename). Before this guard every
 * voice-only send became "请简明说明这件事…不要执行任何外部动作" — the owner's
 * 2026-09-13 "恢复第36项" was answered with an explanation instead of the
 * restore, and the "36" was flagged as an unsupported number because the
 * runtime never saw the owner's words.
 */
export type AssistantCommandIntent = 'chat' | 'explain' | 'draft';

export const ASSISTANT_EXPLAIN_FALLBACK_TEXT =
  '请简明说明这件事是什么、为什么现在出现、我下一步最应该做什么。不要执行任何外部动作。';
export const ASSISTANT_DRAFT_FALLBACK_TEXT =
  '请基于这件事起草一个有用的下一步方案。先给我看，不要直接执行。';

export function assistantCommandFallbackText(
  intent: AssistantCommandIntent,
  hasAttachments: boolean,
): string {
  if (hasAttachments || intent === 'chat') return '';
  return intent === 'draft' ? ASSISTANT_DRAFT_FALLBACK_TEXT : ASSISTANT_EXPLAIN_FALLBACK_TEXT;
}
