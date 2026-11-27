import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

export type ActionExecutionState = {
  id: string;
  label: string;
  confirmLabel: string;
};

export function itemActionButtonLabel(
  action: ActionExecutionState,
  armedActionId: string | null | undefined,
  executingActionId: string | null | undefined,
  locale: Locale = 'en',
): string {
  if (executingActionId === action.id) return t('work.interaction.working', locale);
  if (armedActionId === action.id) return t('work.interaction.confirm', locale);
  return action.label;
}

export function itemActionConfirmationHint(
  action: ActionExecutionState & { receiptHint: string },
  locale: Locale = 'en',
): string {
  return tf('work.interaction.confirmationHint', locale, { hint: action.receiptHint });
}

export function assistantResultIsBusy(responseKind: string | null | undefined): boolean {
  return responseKind === 'working';
}

/**
 * Per-request busy-release decision (used in each chat handler's `finally`).
 *
 * `busy` is GLOBAL UX state, but a request that settles AFTER its session was
 * torn down (the private-session epoch or chat-home generation advanced while it
 * was in-flight) must NOT release `busy`, because a NEWER request may already own
 * it (`setBusy(true)`). So a per-request finally only releases when its captured
 * epoch AND generation still match the current ones.
 *
 * The hole this leaves — an in-flight request whose session is torn down never
 * runs an effective release, wedging `busy` true forever — is closed by the
 * session-reset failsafe below (`shouldForceReleaseBusyOnSessionReset`), which
 * fires exactly on the teardown this guard intentionally skips.
 */
export function shouldReleaseBusyOnRequestSettle(
  capturedEpoch: number,
  currentEpoch: number,
  capturedGeneration: number,
  currentGeneration: number,
): boolean {
  return capturedEpoch === currentEpoch && capturedGeneration === currentGeneration;
}

/**
 * Session-reset busy-release failsafe.
 *
 * Bumped (as state) whenever the private-session epoch or chat-home generation
 * advances — i.e. on reconnect/re-auth, logout, or a conversation clear. On any
 * such teardown we force-release `busy` so an in-flight request that can no
 * longer release it itself (its per-request finally is correctly skipped to avoid
 * clobbering a newer request) cannot wedge the global flag.
 *
 * Tick `0` is the initial mount value and must NOT release (nothing was torn
 * down yet); any tick > 0 means a real session reset occurred.
 */
export function shouldForceReleaseBusyOnSessionReset(sessionResetTick: number): boolean {
  return sessionResetTick > 0;
}
