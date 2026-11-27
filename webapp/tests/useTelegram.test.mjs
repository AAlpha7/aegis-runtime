import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const modulePath = process.env.USE_TELEGRAM_MODULE;
const useTelegramSource = readFileSync(new URL('../src/useTelegram.ts', import.meta.url), 'utf8');

if (!modulePath) {
  throw new Error('USE_TELEGRAM_MODULE must point at the compiled useTelegram.js file');
}

const {
  parseObjectJson,
  safeCssColor,
  safeInsetPx,
  shouldLoadTelegramSdk,
} = await import(modulePath);

describe('Telegram runtime guards', () => {
  it('accepts Telegram-style colors and rejects CSS injection shapes', () => {
    assert.equal(safeCssColor('#abc'), '#abc');
    assert.equal(safeCssColor('#aabbcc'), '#aabbcc');
    assert.equal(safeCssColor('rgb(12, 34, 56)'), 'rgb(12, 34, 56)');
    assert.equal(safeCssColor('rgba(12, 34, 56, 0.5)'), 'rgba(12, 34, 56, 0.5)');
    assert.equal(safeCssColor('url(https://example.com/x)'), undefined);
    assert.equal(safeCssColor('var(--tg-bg)'), undefined);
  });

  it('clamps safe area inset values to finite mobile bounds', () => {
    assert.equal(safeInsetPx(12), '12px');
    assert.equal(safeInsetPx(-5), '0px');
    assert.equal(safeInsetPx(Number.NaN), '0px');
    assert.equal(safeInsetPx(999), '240px');
  });

  it('parses only bounded JSON objects from Telegram storage', () => {
    assert.deepEqual(parseObjectJson('{"last_console_tab":"today"}'), { last_console_tab: 'today' });
    assert.equal(parseObjectJson('[1,2,3]'), null);
    assert.equal(parseObjectJson('null'), null);
    assert.equal(parseObjectJson('{'), null);
    assert.equal(parseObjectJson('{"x":"' + 'a'.repeat(17_000) + '"}'), null);
  });

  it('loads the Telegram SDK only for Telegram-opened URLs', () => {
    assert.equal(shouldLoadTelegramSdk({ search: '', hash: '' }), false);
    assert.equal(shouldLoadTelegramSdk({ search: '?view=work', hash: '' }), false);
    assert.equal(shouldLoadTelegramSdk({ search: '', hash: '#tgWebAppData=user%3D42&tgWebAppVersion=9.6' }), true);
    assert.equal(shouldLoadTelegramSdk({ search: '?tgWebAppPlatform=ios', hash: '' }), true);
  });

  it('keeps the lazy Telegram SDK script private when it is needed', () => {
    assert.match(useTelegramSource, /script\.referrerPolicy = 'no-referrer'/);
    assert.match(useTelegramSource, /script\.async = true/);
    assert.match(useTelegramSource, /document\.head\.appendChild\(script\)/);
  });
});
