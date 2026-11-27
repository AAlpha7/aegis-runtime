export function normalizeMessageTimestamp(value: number | null | undefined): number | undefined {
  if (!Number.isFinite(value)) return undefined;
  const numeric = Number(value);
  if (numeric <= 0) return undefined;
  return numeric < 10_000_000_000 ? numeric * 1000 : numeric;
}

// Cache validated IANA zone names so a bad value is only warned about once and
// repeated formatting calls do not re-pay the Intl validation cost.
const validTimeZoneCache = new Map<string, boolean>();

function isValidTimeZone(timeZone: string): boolean {
  const cached = validTimeZoneCache.get(timeZone);
  if (cached !== undefined) return cached;
  let ok = false;
  try {
    // Constructing a formatter with an invalid timeZone throws RangeError.
    new Intl.DateTimeFormat(undefined, { timeZone });
    ok = true;
  } catch {
    ok = false;
  }
  validTimeZoneCache.set(timeZone, ok);
  return ok;
}

/**
 * Parse a backend wire timestamp string into a `Date`, treating no-offset
 * datetime values as UTC. The backend emits either RFC3339 (with `Z`/offset)
 * or SQLite-style `YYYY-MM-DD HH:MM:SS` (space separator, NO offset — which
 * `datetime('now')` produces and which is implicitly UTC). `new Date()` would
 * parse the latter in the BROWSER timezone, corrupting the instant — so we
 * normalize a no-offset datetime to a UTC RFC3339 string first. Date-only and
 * other forms are passed through to `new Date()` unchanged. Returns `null`
 * when unparseable.
 */
export function parseWireMoment(raw: string): Date | null {
  const value = raw.trim();
  if (!value) return null;
  let normalized = value;
  if (/^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}/.test(value)) {
    const hasOffset = /[zZ]$/.test(value) || /[+-]\d{2}:?\d{2}$/.test(value);
    if (!hasOffset) normalized = `${value.replace(' ', 'T')}Z`;
  }
  const date = new Date(normalized);
  return Number.isFinite(date.getTime()) ? date : null;
}

/**
 * Epoch-millis form of {@link parseWireMoment}: a SQLite no-offset
 * `YYYY-MM-DD HH:MM:SS` value is interpreted as UTC (never browser-local), so
 * comparison / sort / lane-bucketing of wire timestamps is timezone-stable for
 * non-UTC owners. Returns null for empty/unparseable input — callers supply
 * their own fallback sentinel. Use this anywhere a wire timestamp feeds a
 * NUMERIC comparison (not display); never `Date.parse` a wire value directly,
 * which would parse a no-offset value as browser-local.
 */
export function parseWireMomentMs(raw: string | null | undefined): number | null {
  if (!raw) return null;
  const moment = parseWireMoment(raw);
  return moment ? moment.getTime() : null;
}

/**
 * The single place the `timeZone` option is applied. Formats `value` in the
 * owner's configured timezone (`userTimezone`, an IANA string such as
 * `Asia/Shanghai`). When `userTimezone` is null/empty/invalid, falls back to
 * the browser timezone (the historical behavior) by omitting the `timeZone`
 * option. Never throws.
 *
 * `value` may be a `Date`, an epoch-millis number, or a parseable date string.
 * Returns '' for an unparseable/empty value.
 */
export function formatInUserTz(
  value: Date | number | string | null | undefined,
  userTimezone: string | null | undefined,
  opts: Intl.DateTimeFormatOptions,
  locale?: string,
): string {
  if (value === null || value === undefined || value === '') return '';
  // For string inputs, normalize no-offset SQLite datetimes to UTC so they
  // are not parsed in the browser timezone (see parseWireMoment).
  const date = value instanceof Date
    ? value
    : typeof value === 'string'
      ? parseWireMoment(value)
      : new Date(value);
  if (!date || !Number.isFinite(date.getTime())) return '';

  const tz = userTimezone?.trim();
  const useTz = tz && isValidTimeZone(tz) ? tz : undefined;
  const effectiveOpts: Intl.DateTimeFormatOptions = useTz
    ? { ...opts, timeZone: useTz }
    : opts;

  try {
    return new Intl.DateTimeFormat(locale, effectiveOpts).format(date);
  } catch {
    // Defensive: should not happen since the tz is pre-validated, but never
    // throw from a display helper.
    return new Intl.DateTimeFormat(locale, opts).format(date);
  }
}

/**
 * Calendar-day key (YYYY-MM-DD) of a timestamp as observed in the user's
 * timezone. Empty string for missing/invalid timestamps. en-CA yields the
 * ISO-like YYYY-MM-DD ordering regardless of the UI locale, so keys compare
 * as plain strings.
 */
export function chatDayKey(value: number | null | undefined, userTimezone?: string | null): string {
  const timestamp = normalizeMessageTimestamp(value);
  if (!timestamp) return '';
  return formatInUserTz(
    new Date(timestamp),
    userTimezone,
    { year: 'numeric', month: '2-digit', day: '2-digit' },
    'en-CA',
  );
}

export function chatTimestampLabel(
  value: number | null | undefined,
  userTimezone?: string | null,
): string {
  const timestamp = normalizeMessageTimestamp(value);
  if (!timestamp) return '';
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '';
  // hour:minute, zero-padded, in the user's timezone (24h, fixed across
  // locales via the en-GB locale + hourCycle h23).
  return formatInUserTz(
    date,
    userTimezone,
    { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' },
    'en-GB',
  );
}

/**
 * Compute a `UTC±HH:MM` (or `UTC`) offset label for `date` as observed in
 * `userTimezone`. Falls back to the browser offset (NOT a hard-coded `UTC`)
 * when the timezone is null/empty/invalid, so the label always agrees with
 * the wall clock `formatInUserTz` actually rendered.
 */
export function timezoneOffsetLabel(date: Date, userTimezone?: string | null): string {
  const tz = userTimezone?.trim();
  if (tz && isValidTimeZone(tz)) {
    try {
      const parts = new Intl.DateTimeFormat('en-US', {
        timeZone: tz,
        timeZoneName: 'longOffset',
      }).formatToParts(date);
      const name = parts.find((p) => p.type === 'timeZoneName')?.value;
      if (name) {
        // Intl returns e.g. "GMT+8" or "GMT+08:00"; normalize to UTC±HH:MM.
        const match = name.match(/GMT([+-])(\d{1,2})(?::?(\d{2}))?/);
        if (match) {
          const sign = match[1];
          const hours = match[2].padStart(2, '0');
          const minutes = (match[3] ?? '00').padStart(2, '0');
          if (hours === '00' && minutes === '00') return 'UTC';
          return `UTC${sign}${hours}:${minutes}`;
        }
        if (name === 'GMT') return 'UTC';
      }
    } catch {
      // fall through to browser offset
    }
  }
  const offsetMinutes = -date.getTimezoneOffset();
  if (offsetMinutes === 0) return 'UTC';
  const sign = offsetMinutes >= 0 ? '+' : '-';
  const absolute = Math.abs(offsetMinutes);
  const hours = Math.floor(absolute / 60).toString().padStart(2, '0');
  const minutes = (absolute % 60).toString().padStart(2, '0');
  return `UTC${sign}${hours}:${minutes}`;
}

export function chatFullTimestampLabel(
  value: number | null | undefined,
  userTimezone?: string | null,
): string {
  const timestamp = normalizeMessageTimestamp(value);
  if (!timestamp) return '';
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '';
  // YYYY-MM-DD HH:MM:SS, in the user's timezone, with a trailing offset label.
  const wallClock = formatInUserTz(
    date,
    userTimezone,
    {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hourCycle: 'h23',
    },
    'en-CA', // en-CA renders date as YYYY-MM-DD
  ).replace(',', '');
  return `${wallClock} ${timezoneOffsetLabel(date, userTimezone)}`;
}
