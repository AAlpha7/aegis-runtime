import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_TIME_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-time-test/time.js';
const { formatInUserTz, chatTimestampLabel, chatFullTimestampLabel, timezoneOffsetLabel, parseWireMoment, parseWireMomentMs } = await import(modulePath);

// A single fixed UTC instant: 2026-01-15T20:30:00Z.
// In Asia/Shanghai (UTC+8) that is 2026-01-16 04:30 (next day).
// In America/New_York (UTC-5 in January) that is 2026-01-15 15:30.
const FIXED_UTC_MS = Date.UTC(2026, 0, 15, 20, 30, 0);

describe('formatInUserTz applies the user timezone (not browser/UTC)', () => {
  it('renders the same instant differently under two user timezones', () => {
    const opts = {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    };
    const shanghai = formatInUserTz(FIXED_UTC_MS, 'Asia/Shanghai', opts, 'en-CA').replace(',', '');
    const newYork = formatInUserTz(FIXED_UTC_MS, 'America/New_York', opts, 'en-CA').replace(',', '');

    // Proves the timeZone option is actually applied: the wall-clock strings
    // differ and match the expected local time in each zone.
    assert.equal(shanghai, '2026-01-16 04:30');
    assert.equal(newYork, '2026-01-15 15:30');
    assert.notEqual(shanghai, newYork);
  });

  it('falls back to a non-throwing render for null/empty/invalid timezones', () => {
    const opts = { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };
    // None of these should throw; they fall back to the browser timezone.
    assert.doesNotThrow(() => formatInUserTz(FIXED_UTC_MS, null, opts, 'en-GB'));
    assert.doesNotThrow(() => formatInUserTz(FIXED_UTC_MS, '', opts, 'en-GB'));
    assert.doesNotThrow(() => formatInUserTz(FIXED_UTC_MS, 'Not/AZone', opts, 'en-GB'));
    // Invalid tz must not change behavior vs no-tz (both fall back to browser).
    assert.equal(
      formatInUserTz(FIXED_UTC_MS, 'Not/AZone', opts, 'en-GB'),
      formatInUserTz(FIXED_UTC_MS, null, opts, 'en-GB'),
    );
  });

  it('treats SQLite-style no-offset datetimes as UTC (string input)', () => {
    // '2026-01-15 20:30:00' is the SQLite datetime('now') form (UTC, no offset).
    // Must render identically regardless of process TZ (suite also runs under
    // TZ=America/New_York). In Asia/Shanghai it is 04:30 next day.
    const opts = { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };
    assert.equal(
      formatInUserTz('2026-01-15 20:30:00', 'Asia/Shanghai', opts, 'en-CA').replace(',', ''),
      '2026-01-16 04:30',
    );
    // RFC3339 with Z must agree with the SQLite form.
    assert.equal(
      formatInUserTz('2026-01-15T20:30:00Z', 'Asia/Shanghai', opts, 'en-CA').replace(',', ''),
      '2026-01-16 04:30',
    );
  });

  it('parseWireMoment normalizes no-offset datetimes to the same instant as RFC3339 Z', () => {
    assert.equal(parseWireMoment('2026-01-15 20:30:00').getTime(), Date.UTC(2026, 0, 15, 20, 30, 0));
    assert.equal(parseWireMoment('2026-01-15T20:30:00Z').getTime(), Date.UTC(2026, 0, 15, 20, 30, 0));
    assert.equal(parseWireMoment('not-a-date'), null);
  });

  it('parseWireMomentMs gives UTC-stable epoch ms for no-offset wire values (comparison/sort path)', () => {
    // The bug: comparison sites (work-item lane bucketing + sort) used raw
    // Date.parse, which reads a no-offset SQLite `YYYY-MM-DD HH:MM:SS` as
    // browser-local — wrong lane/order for non-UTC owners. This suite runs under
    // TZ=America/New_York too, so the equality to Date.UTC proves it is NOT
    // parsed browser-local.
    assert.equal(parseWireMomentMs('2026-01-15 20:30:00'), Date.UTC(2026, 0, 15, 20, 30, 0));
    assert.equal(parseWireMomentMs('2026-01-15T20:30:00Z'), Date.UTC(2026, 0, 15, 20, 30, 0));
    assert.equal(parseWireMomentMs('not-a-date'), null);
    assert.equal(parseWireMomentMs(null), null);
    assert.equal(parseWireMomentMs(''), null);
  });

  it('returns empty string for unparseable / empty values', () => {
    const opts = { hour: '2-digit', minute: '2-digit' };
    assert.equal(formatInUserTz(null, 'Asia/Shanghai', opts), '');
    assert.equal(formatInUserTz(undefined, 'Asia/Shanghai', opts), '');
    assert.equal(formatInUserTz('', 'Asia/Shanghai', opts), '');
    assert.equal(formatInUserTz('not-a-date', 'Asia/Shanghai', opts), '');
  });
});

describe('chat timestamp labels honor the user timezone', () => {
  it('chatTimestampLabel shows the wall-clock hour:minute in the user tz', () => {
    // Epoch SECONDS form (normalizeMessageTimestamp multiplies <1e10 by 1000).
    const epochSeconds = FIXED_UTC_MS / 1000;
    assert.equal(chatTimestampLabel(epochSeconds, 'Asia/Shanghai'), '04:30');
    assert.equal(chatTimestampLabel(epochSeconds, 'America/New_York'), '15:30');
  });

  it('chatFullTimestampLabel includes the user-tz wall clock and offset label', () => {
    const epochSeconds = FIXED_UTC_MS / 1000;
    assert.equal(
      chatFullTimestampLabel(epochSeconds, 'Asia/Shanghai'),
      '2026-01-16 04:30:00 UTC+08:00',
    );
    assert.equal(
      chatFullTimestampLabel(epochSeconds, 'America/New_York'),
      '2026-01-15 15:30:00 UTC-05:00',
    );
  });
});

describe('timezoneOffsetLabel never leaks a mismatched UTC label', () => {
  const date = new Date(FIXED_UTC_MS);

  it('returns the explicit offset for a valid user timezone', () => {
    assert.equal(timezoneOffsetLabel(date, 'Asia/Shanghai'), 'UTC+08:00');
    assert.equal(timezoneOffsetLabel(date, 'America/New_York'), 'UTC-05:00');
    assert.equal(timezoneOffsetLabel(date, 'UTC'), 'UTC');
  });

  it('falls back to the BROWSER offset (not hard-coded UTC) for invalid/null tz', () => {
    // The label for an invalid/null tz must match what the browser-offset
    // computation produces — i.e. it must agree with the wall clock that
    // formatInUserTz renders (which also falls back to the browser tz).
    // This proves it never returns a mismatched 'UTC' under a non-UTC runtime.
    const offsetMinutes = -date.getTimezoneOffset();
    let expected;
    if (offsetMinutes === 0) {
      expected = 'UTC';
    } else {
      const sign = offsetMinutes >= 0 ? '+' : '-';
      const abs = Math.abs(offsetMinutes);
      const hh = String(Math.floor(abs / 60)).padStart(2, '0');
      const mm = String(abs % 60).padStart(2, '0');
      expected = `UTC${sign}${hh}:${mm}`;
    }
    assert.equal(timezoneOffsetLabel(date, null), expected);
    assert.equal(timezoneOffsetLabel(date, ''), expected);
    assert.equal(timezoneOffsetLabel(date, 'Not/AZone'), expected);
  });
});
