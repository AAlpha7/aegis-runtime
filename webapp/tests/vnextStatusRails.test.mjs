import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// Bundled by tests/fixtures/statusRailsEntry.ts (see the
// test:vnext-status-rails script): the REAL statusRails / onlineStatus
// modules and banner components, no reimplementation.
const modulePath = process.env.VNEXT_STATUS_RAILS_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-status-rails-test/statusRails.mjs', import.meta.url).href;
const {
  TELEGRAM_RAIL_DISMISSED_KEY,
  TELEGRAM_UNREACHABLE_BANNER_AFTER_SECS,
  nightlyStatusLine,
  readNightlyStatus,
  readTelegramRail,
  readTelegramRailDismissedSince,
  telegramRailBannerState,
  telegramRailRow,
  writeTelegramRailDismissedSince,
  createOnlineStatusStore,
  readOnlineStatus,
  TelegramRailBanner,
  OfflineBanner,
  LocaleProvider,
} = await import(modulePath);

const NOW = 1_789_700_000;
const noop = () => {};

function rail(overrides = {}) {
  return {
    state: 'invalid_token',
    since_unix: NOW - 120,
    last_error: "A Telegram's error: Invalid bot token",
    degraded: true,
    ...overrides,
  };
}

function renderTelegramBanner(props = {}) {
  const { locale, ...componentProps } = props;
  const element = React.createElement(TelegramRailBanner, {
    rail: rail(),
    dismissedSinceUnix: null,
    onDismiss: noop,
    nowUnix: NOW,
    ...componentProps,
  });
  const tree = locale ? React.createElement(LocaleProvider, { displayLanguage: locale }, element) : element;
  return renderToStaticMarkup(tree);
}

function renderOfflineBanner(props = {}) {
  const { locale, ...componentProps } = props;
  const element = React.createElement(OfflineBanner, { offline: true, ...componentProps });
  const tree = locale ? React.createElement(LocaleProvider, { displayLanguage: locale }, element) : element;
  return renderToStaticMarkup(tree);
}

describe('Telegram rail banner state machine', () => {
  it('shows the invalid-token alert immediately and nothing for a healthy or unreported rail', () => {
    assert.deepEqual(telegramRailBannerState(rail(), null, NOW), { kind: 'invalid_token', sinceUnix: NOW - 120, minutes: 2 });
    // Negative controls.
    assert.equal(telegramRailBannerState(rail({ state: 'reachable', degraded: false, last_error: null }), null, NOW), null);
    assert.equal(telegramRailBannerState(rail({ state: 'starting', degraded: false }), null, NOW), null);
    assert.equal(telegramRailBannerState(rail({ state: 'unconfigured', degraded: false }), null, NOW), null);
    assert.equal(telegramRailBannerState(null, null, NOW), null);
    assert.equal(telegramRailBannerState(undefined, null, NOW), null);
  });

  it('hides the banner once the owner dismissed THIS episode, and re-arms when since_unix changes', () => {
    const since = NOW - 120;
    assert.equal(telegramRailBannerState(rail({ since_unix: since }), since, NOW), null, 'dismissed episode stays hidden');
    // A new episode (the bot restarted, the state flipped) restamps since_unix.
    const rearmed = telegramRailBannerState(rail({ since_unix: since + 600 }), since, NOW + 700);
    assert.equal(rearmed?.kind, 'invalid_token');
    assert.equal(rearmed?.sinceUnix, since + 600);
    // Negative control: without the since_unix comparison the dismissal would
    // never re-arm — a dismissal of a different episode must not hide this one.
    assert.notEqual(telegramRailBannerState(rail({ since_unix: since }), since - 1, NOW), null);
  });

  it('treats a fresh unreachable as a blip and a long one as the softer banner', () => {
    const fresh = rail({ state: 'unreachable', since_unix: NOW - 30, degraded: false, last_error: 'dns' });
    assert.equal(telegramRailBannerState(fresh, null, NOW), null);
    // The server's verdict wins even if the local clock disagrees.
    const serverSays = rail({ state: 'unreachable', since_unix: NOW - 30, degraded: true, last_error: 'dns' });
    assert.deepEqual(telegramRailBannerState(serverSays, null, NOW), { kind: 'unreachable', sinceUnix: NOW - 30, minutes: 0 });
    // ...and so does the local clock once the blip window has passed (the
    // projection is not polled, so the client must not wait for a refetch).
    const byClock = rail({ state: 'unreachable', since_unix: NOW - TELEGRAM_UNREACHABLE_BANNER_AFTER_SECS - 60, degraded: false, last_error: 'dns' });
    assert.deepEqual(telegramRailBannerState(byClock, null, NOW), { kind: 'unreachable', sinceUnix: byClock.since_unix, minutes: 11 });
    assert.equal(telegramRailBannerState(byClock, byClock.since_unix, NOW), null, 'the soft banner honours the same dismissal');
  });

  it('persists the dismissed since_unix and survives a broken or absent storage', () => {
    const store = new Map();
    const storage = { getItem: (key) => store.get(key) ?? null, setItem: (key, value) => store.set(key, value) };
    assert.equal(readTelegramRailDismissedSince(storage), null);
    writeTelegramRailDismissedSince(NOW - 120, storage);
    assert.equal(store.get(TELEGRAM_RAIL_DISMISSED_KEY), String(NOW - 120));
    assert.equal(readTelegramRailDismissedSince(storage), NOW - 120);
    store.set(TELEGRAM_RAIL_DISMISSED_KEY, 'garbage');
    assert.equal(readTelegramRailDismissedSince(storage), null);
    const broken = { getItem: () => { throw new Error('quota'); }, setItem: () => { throw new Error('quota'); } };
    assert.equal(readTelegramRailDismissedSince(broken), null);
    assert.doesNotThrow(() => writeTelegramRailDismissedSince(1, broken));
    assert.equal(readTelegramRailDismissedSince(null), null);
  });

  it('reads the rails off a status projection defensively', () => {
    assert.deepEqual(readTelegramRail({ telegram_rail: rail() }), rail());
    assert.equal(readTelegramRail({ telegram_rail: { state: 'exploded', since_unix: 1 } }), null);
    assert.equal(readTelegramRail({}), null);
    assert.equal(readTelegramRail(null), null);
    const nightly = { verdict: 'RED', sha: '02c0d462', finished_at_unix: NOW - 7200, failed_steps: [], report: null };
    assert.deepEqual(readNightlyStatus({ nightly }), nightly);
    assert.equal(readNightlyStatus({ nightly: { verdict: 'red' } }), null);
    assert.equal(readNightlyStatus(undefined), null);
  });
});

describe('Telegram rail status row and nightly line', () => {
  it('builds the Connections row with the same grammar as a source row', () => {
    const zh = telegramRailRow(rail(), NOW, 'zh');
    assert.deepEqual(zh, {
      label: 'Telegram 通知',
      statusLabel: 'token 已失效',
      healthy: false,
      reason: '手机端通知已停。请在 .env 里更新 TELOXIDE_TOKEN 并重启安琪。',
    });
    const en = telegramRailRow(rail({ state: 'reachable', degraded: false, last_error: null }), NOW, 'en');
    assert.deepEqual(en, { label: 'Telegram notifications', statusLabel: 'Connected', healthy: true });
    const blip = telegramRailRow(rail({ state: 'unreachable', since_unix: NOW - 90, degraded: false }), NOW, 'en');
    assert.equal(blip.healthy, true);
    assert.equal(blip.reason, 'For 1 minutes now; Aegis keeps retrying.');
    const long = telegramRailRow(rail({ state: 'unreachable', since_unix: NOW - 900, degraded: true }), NOW, 'zh');
    assert.equal(long.healthy, false);
    assert.equal(long.reason, '已持续 15 分钟，安琪会自动重试。');
    assert.equal(telegramRailRow(null, NOW, 'zh'), null);
  });

  it('formats the nightly line zh-first and stays silent for GREEN', () => {
    const red = { verdict: 'RED', sha: '02c0d462', finished_at_unix: NOW - 3 * 3600 - 1, failed_steps: ['cargo test'], report: null };
    assert.equal(nightlyStatusLine(red, NOW, 'zh'), '夜间测试 RED（02c0d462，3 小时前）');
    assert.equal(nightlyStatusLine(red, NOW, 'en'), 'Nightly test run RED (02c0d462, 3 hours ago)');
    assert.equal(nightlyStatusLine({ ...red, verdict: 'BROWSER_RED', finished_at_unix: NOW - 59 * 60 }, NOW, 'zh'), '夜间测试 BROWSER_RED（02c0d462，不到 1 小时前）');
    assert.equal(nightlyStatusLine({ ...red, sha: '' }, NOW, 'en'), 'Nightly test run RED (unknown commit, 3 hours ago)');
    assert.equal(nightlyStatusLine({ ...red, verdict: 'GREEN' }, NOW, 'zh'), null);
    assert.equal(nightlyStatusLine(null, NOW, 'zh'), null);
    assert.equal(nightlyStatusLine(undefined, NOW, 'zh'), null);
  });
});

describe('online status store', () => {
  function fakeSource(initial = true) {
    const listeners = { online: new Set(), offline: new Set() };
    let onLine = initial;
    return {
      source: {
        onLine: () => onLine,
        addEventListener: (type, listener) => listeners[type].add(listener),
        removeEventListener: (type, listener) => listeners[type].delete(listener),
      },
      go(state) {
        onLine = state;
        for (const listener of listeners[state ? 'online' : 'offline']) listener();
      },
      listenerCount: () => listeners.online.size + listeners.offline.size,
    };
  }

  it('reports offline on the offline event and online again on reconnect', () => {
    const fake = fakeSource(true);
    const store = createOnlineStatusStore(fake.source);
    let notified = 0;
    const unsubscribe = store.subscribe(() => { notified += 1; });
    assert.equal(store.getSnapshot(), true);
    fake.go(false);
    assert.equal(notified, 1, 'the offline event wakes the subscriber');
    assert.equal(store.getSnapshot(), false);
    fake.go(true);
    assert.equal(notified, 2);
    assert.equal(store.getSnapshot(), true);
    unsubscribe();
    assert.equal(fake.listenerCount(), 0, 'unsubscribe removes both listeners');
    fake.go(false);
    assert.equal(notified, 2, 'no notification after unsubscribe');
  });

  it('never reports offline when the platform cannot say', () => {
    assert.equal(readOnlineStatus({ onLine: () => undefined, addEventListener: noop, removeEventListener: noop }), true);
    assert.equal(readOnlineStatus({ onLine: () => { throw new Error('no navigator'); }, addEventListener: noop, removeEventListener: noop }), true);
    assert.equal(readOnlineStatus({ onLine: () => false, addEventListener: noop, removeEventListener: noop }), false);
  });
});

describe('Telegram rail and offline banners render', () => {
  it('renders the invalid-token alert with the .env hint in zh and en', () => {
    const zh = renderTelegramBanner({ locale: 'zh' });
    assert.match(zh, /role="alert"/);
    assert.match(zh, /data-telegram-rail="invalid_token"/);
    assert.match(zh, /Telegram 机器人的 token 已失效，手机端通知已停/);
    assert.match(zh, /请在 \.env 里更新 TELOXIDE_TOKEN 并重启安琪。/);
    assert.match(zh, /忽略/);
    assert.doesNotMatch(zh, /soft/);
    const en = renderTelegramBanner({ locale: 'en' });
    assert.match(en, /The Telegram bot token is invalid; phone notifications have stopped/);
    assert.match(en, /Set a new TELOXIDE_TOKEN in \.env and restart Aegis\./);
    assert.match(en, /Dismiss/);
  });

  it('renders the softer unreachable status with the elapsed minutes', () => {
    const html = renderTelegramBanner({
      locale: 'zh',
      rail: rail({ state: 'unreachable', since_unix: NOW - 12 * 60, degraded: true, last_error: 'dns' }),
    });
    assert.match(html, /role="status"/);
    assert.match(html, /aria-live="polite"/);
    assert.match(html, /vnext-telegram-rail-banner soft/);
    assert.match(html, /Telegram 暂时连不上，手机端通知可能延迟/);
    assert.match(html, /已持续 12 分钟，安琪会自动重试。/);
  });

  it('renders nothing when healthy, dismissed, or unreported — and re-shows for a new episode', () => {
    assert.equal(renderTelegramBanner({ rail: null }), '');
    assert.equal(renderTelegramBanner({ rail: rail({ state: 'reachable', degraded: false }) }), '');
    assert.equal(renderTelegramBanner({ dismissedSinceUnix: NOW - 120 }), '');
    assert.match(renderTelegramBanner({ locale: 'en', rail: rail({ since_unix: NOW - 60 }), dismissedSinceUnix: NOW - 120 }), /phone notifications have stopped/);
  });

  it('renders the offline banner only while offline', () => {
    const zh = renderOfflineBanner({ locale: 'zh' });
    assert.match(zh, /data-offline-banner="true"/);
    assert.match(zh, /已离线，等待网络恢复/);
    assert.match(zh, /role="status"/);
    assert.match(renderOfflineBanner({ locale: 'en' }), /Offline\. Waiting for the network to come back\./);
    assert.equal(renderOfflineBanner({ offline: false }), '');
  });
});
