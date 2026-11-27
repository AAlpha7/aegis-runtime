import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';

const modulePath = process.env.I18N_CATALOG_MODULE;

if (!modulePath) {
  throw new Error('I18N_CATALOG_MODULE must point at the compiled catalog.js file');
}

const { preferredLocale } = await import(modulePath);

const hadDocument = Object.prototype.hasOwnProperty.call(globalThis, 'document');
const previousDocument = globalThis.document;
const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');

function stampLang(lang) {
  globalThis.document = { documentElement: { lang } };
}

function fakeNavigator(language) {
  Object.defineProperty(globalThis, 'navigator', { value: { language }, configurable: true, writable: true });
}

afterEach(() => {
  if (hadDocument) globalThis.document = previousDocument;
  else delete globalThis.document;
  if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor);
  else delete globalThis.navigator;
});

describe('preferredLocale follows the stamped <html lang>', () => {
  it('reads the document language the LocaleProvider stamps', () => {
    stampLang('zh-CN');
    assert.equal(preferredLocale(), 'zh');
    stampLang('en');
    assert.equal(preferredLocale(), 'en');
    stampLang('en-US');
    assert.equal(preferredLocale(), 'en');
    stampLang('zh');
    assert.equal(preferredLocale(), 'zh');
  });

  it('prefers the stamped language over the browser language', () => {
    fakeNavigator('en-GB');
    stampLang('zh-CN');
    assert.equal(preferredLocale(), 'zh');
    fakeNavigator('zh-CN');
    stampLang('en');
    assert.equal(preferredLocale(), 'en');
  });

  it('falls back to the browser language only when nothing is stamped', () => {
    stampLang('');
    fakeNavigator('en-GB');
    assert.equal(preferredLocale(), 'en');
    fakeNavigator('ja-JP');
    assert.equal(preferredLocale(), 'zh');
  });

  it('defaults to zh outside a browser', () => {
    delete globalThis.document;
    Object.defineProperty(globalThis, 'navigator', { value: undefined, configurable: true, writable: true });
    assert.equal(preferredLocale(), 'zh');
  });
});
