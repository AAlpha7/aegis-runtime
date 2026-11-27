import { t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';
import { sanitizeNightlyStatus, sanitizeTelegramRail } from '../apiGuards';
import type { GuardedVNextNightlyStatus, GuardedVNextTelegramRail, GuardedVNextTelegramRailState } from '../apiGuards';

/**
 * Runtime rails on the status projection (added 2026-09-16 after the revoked
 * Telegram token went unnoticed for hours): the Telegram bot's connectivity
 * and the last nightly verdict. Pure helpers only — the banner state machine,
 * the status-row view model and the nightly line — so the App/Status wiring
 * stays thin and every rule here is unit-tested without a DOM.
 */
export type VNextTelegramRail = GuardedVNextTelegramRail;
export type VNextTelegramRailState = GuardedVNextTelegramRailState;
export type VNextNightlyStatus = GuardedVNextNightlyStatus;

/**
 * Mirrors the server's `UNREACHABLE_DEGRADED_AFTER_SECS`: an `unreachable`
 * younger than this is a blip (DNS hiccup, Telegram 5xx) and gets no banner.
 */
export const TELEGRAM_UNREACHABLE_BANNER_AFTER_SECS = 600;

/**
 * localStorage key holding the `since_unix` of the Telegram outage the owner
 * last dismissed. Same contract as the LLM banner: a dismissed episode stays
 * dismissed across reloads, a new episode (different `since_unix`) re-arms.
 */
export const TELEGRAM_RAIL_DISMISSED_KEY = 'aegis-runtime:vnext:telegram-rail-dismissed-since';

/**
 * The status projection type in `api.ts` predates these rails; read them
 * structurally and re-validate so a caller handing in a raw object (tests,
 * an older cached projection) can never produce a banner from garbage.
 */
export function readTelegramRail(projection: unknown): VNextTelegramRail | null {
  if (!projection || typeof projection !== 'object') return null;
  const raw = (projection as { telegram_rail?: unknown }).telegram_rail;
  return sanitizeTelegramRail(raw) ?? null;
}

export function readNightlyStatus(projection: unknown): VNextNightlyStatus | null {
  if (!projection || typeof projection !== 'object') return null;
  const raw = (projection as { nightly?: unknown }).nightly;
  return sanitizeNightlyStatus(raw) ?? null;
}

export function nowUnixSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

export type TelegramRailBannerKind = 'invalid_token' | 'unreachable';

export interface TelegramRailBannerState {
  kind: TelegramRailBannerKind;
  /** Dismiss key: the episode start the banner belongs to. */
  sinceUnix: number;
  /** Whole minutes the current state has lasted (never negative). */
  minutes: number;
}

function elapsedSeconds(rail: VNextTelegramRail, nowUnix: number): number {
  return Math.max(0, nowUnix - rail.since_unix);
}

/** Server verdict OR the local clock: an `unreachable` past the blip window. */
function unreachableLongEnough(rail: VNextTelegramRail, nowUnix: number): boolean {
  return rail.state === 'unreachable'
    && (rail.degraded || elapsedSeconds(rail, nowUnix) > TELEGRAM_UNREACHABLE_BANNER_AFTER_SECS);
}

/**
 * Banner state machine.
 *
 *  - `invalid_token` → the alert banner, immediately (a 401 never fixes itself).
 *  - `unreachable` → the softer banner, only once the outage is past the blip
 *    window (server `degraded` flag, or ten minutes by the local clock).
 *  - anything else → no banner.
 *  - A dismissal is keyed to `since_unix`; the banner stays hidden for that
 *    episode and re-arms when `since_unix` changes (a new episode, or the
 *    state flipping — the server restamps `since_unix` on every transition).
 */
export function telegramRailBannerState(
  rail: VNextTelegramRail | null | undefined,
  dismissedSinceUnix: number | null,
  nowUnix: number,
): TelegramRailBannerState | null {
  if (!rail) return null;
  if (dismissedSinceUnix !== null && dismissedSinceUnix === rail.since_unix) return null;
  const minutes = Math.floor(elapsedSeconds(rail, nowUnix) / 60);
  if (rail.state === 'invalid_token') return { kind: 'invalid_token', sinceUnix: rail.since_unix, minutes };
  if (unreachableLongEnough(rail, nowUnix)) return { kind: 'unreachable', sinceUnix: rail.since_unix, minutes };
  return null;
}

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

function defaultStorage(): StorageLike | null {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null;
  } catch {
    return null;
  }
}

export function readTelegramRailDismissedSince(storage: StorageLike | null = defaultStorage()): number | null {
  try {
    const raw = storage?.getItem(TELEGRAM_RAIL_DISMISSED_KEY);
    if (!raw) return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed >= 0 ? Math.floor(parsed) : null;
  } catch {
    return null;
  }
}

export function writeTelegramRailDismissedSince(sinceUnix: number, storage: StorageLike | null = defaultStorage()): void {
  try {
    storage?.setItem(TELEGRAM_RAIL_DISMISSED_KEY, String(Math.floor(sinceUnix)));
  } catch {
    // localStorage may be unavailable (private mode / quota); the banner
    // simply re-shows on next load — the safe (never-hide-an-outage) direction.
  }
}

/**
 * The Telegram row for the Status "Connections" list, in the same grammar as
 * `sourceStatusView` (label / status label / healthy / reason). `healthy` is
 * false only for the states that warrant attention: an invalid token, or an
 * `unreachable` past the blip window.
 */
export interface TelegramRailRow {
  label: string;
  statusLabel: string;
  healthy: boolean;
  reason?: string;
}

export function telegramRailRow(
  rail: VNextTelegramRail | null | undefined,
  nowUnix: number,
  locale: Locale,
): TelegramRailRow | null {
  if (!rail) return null;
  const label = t('status.telegram.label', locale);
  const statusLabel = t(`status.telegram.state.${rail.state}`, locale);
  if (rail.state === 'invalid_token') {
    return { label, statusLabel, healthy: false, reason: t('status.telegram.reason.invalidToken', locale) };
  }
  if (rail.state === 'unreachable') {
    const minutes = String(Math.floor(elapsedSeconds(rail, nowUnix) / 60));
    return {
      label,
      statusLabel,
      healthy: !unreachableLongEnough(rail, nowUnix),
      reason: tf('status.telegram.reason.unreachable', locale, { minutes }),
    };
  }
  return { label, statusLabel, healthy: true };
}

export interface SettingsTelegramNote {
  tone: 'ok' | 'quiet' | 'warn';
  text: string;
  state: VNextTelegramRailState;
}

export function settingsTelegramNote(
  rail: VNextTelegramRail | null | undefined,
  nowUnix: number,
  locale: Locale,
): SettingsTelegramNote | null {
  if (!rail) return null;
  const row = telegramRailRow(rail, nowUnix, locale);
  if (!row) return null;
  if (rail.state === 'reachable') {
    return {
      tone: 'ok',
      text: t('settings.notifications.telegramStays', locale),
      state: rail.state,
    };
  }
  if (rail.state === 'unconfigured') {
    return {
      tone: 'quiet',
      text: t('settings.notifications.telegramOff', locale),
      state: rail.state,
    };
  }
  const text = row.reason ? `${row.statusLabel} · ${row.reason}` : row.statusLabel;
  return { tone: row.healthy ? 'quiet' : 'warn', text, state: rail.state };
}

/**
 * Muted one-liner for a failed nightly run: `null` for a green (or absent)
 * verdict, otherwise "夜间测试 RED（02c0d462，3 小时前）". BROWSER_RED and any
 * future non-green verdict read the same way, with the verdict spelled out.
 */
export function nightlyStatusLine(
  nightly: VNextNightlyStatus | null | undefined,
  nowUnix: number,
  locale: Locale,
): string | null {
  if (!nightly || nightly.verdict === 'GREEN') return null;
  const sha = nightly.sha.trim() || t('status.nightly.unknownSha', locale);
  const hours = Math.floor(Math.max(0, nowUnix - nightly.finished_at_unix) / 3600);
  if (hours < 1) return tf('status.nightly.redRecent', locale, { verdict: nightly.verdict, sha });
  return tf('status.nightly.red', locale, { verdict: nightly.verdict, sha, hours: String(hours) });
}
