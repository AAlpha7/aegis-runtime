import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.I18N_CATALOG_MODULE;

if (!modulePath) {
  throw new Error('I18N_CATALOG_MODULE must point at the compiled catalog.js file');
}

const { CATALOG, t, normalizeLocale } = await import(modulePath);

const LOCALES = ['zh', 'en'];

describe('i18n catalog parity', () => {
  it('every key has both zh and en translations', () => {
    const missing = [];
    for (const [key, entry] of Object.entries(CATALOG)) {
      for (const loc of LOCALES) {
        if (typeof entry?.[loc] !== 'string' || entry[loc].length === 0) {
          missing.push(`${key}:${loc}`);
        }
      }
    }
    assert.deepEqual(missing, [], `catalog gaps: ${missing.join(', ')}`);
  });

  it('t() resolves a known key for both locales', () => {
    assert.equal(t('callout.important', 'zh'), '重要');
    assert.equal(t('callout.important', 'en'), 'Important');
  });

  it('t() returns a visible sentinel for an unknown key', () => {
    assert.equal(t('does.not.exist', 'zh'), '[MISSING:does.not.exist]');
  });

  it('normalizeLocale maps en* to en and everything else to zh', () => {
    assert.equal(normalizeLocale('en'), 'en');
    assert.equal(normalizeLocale('EN-US'), 'en');
    assert.equal(normalizeLocale('zh'), 'zh');
    assert.equal(normalizeLocale('zh-CN'), 'zh');
    assert.equal(normalizeLocale(undefined), 'zh');
    assert.equal(normalizeLocale(''), 'zh');
  });
});
