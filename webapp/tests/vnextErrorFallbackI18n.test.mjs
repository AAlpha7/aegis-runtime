import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

// Bundled entry re-exports inlineUserFacingError + t (+ tf) from the real
// catalog/userFacingErrors sources (see test:vnext-error-fallback-i18n script).
const modulePath = process.env.VNEXT_ERROR_FALLBACK_MODULE;
assert.ok(modulePath, 'VNEXT_ERROR_FALLBACK_MODULE must be set');
const { inlineUserFacingError, t } = await import(modulePath);

// Representative 401-style error: routes through error.signInAgain, which
// interpolates {fallback} into its template. This is exactly the bug path:
// before the fix, an English fallback literal mixed into the zh template.
const signInError = { statusCode: 401, message: 'unauthorized' };

describe('displayError fallbacks are catalog-localized (no zh/en mix)', () => {
  it('zh: 401 message has no leaked English run from the fallback', () => {
    const out = inlineUserFacingError(
      signInError,
      t('error.fallback.couldNotSaveSnapshot', 'zh'),
      'zh',
    );
    // The fully-localized zh output must not contain any 4+ letter ASCII run.
    // (Before the fix the English fallback "Could not save support snapshot"
    // would appear verbatim inside the Chinese sentence.)
    assert.doesNotMatch(
      out,
      /[A-Za-z]{4,}/,
      `zh error output leaked an English run: ${JSON.stringify(out)}`,
    );
    // And it should be the exact expected fully-zh string.
    assert.equal(out, '无法保存支持快照：请重新登录。');
  });

  it('zh: raw last-resort fallback (no apiError) is also fully zh', () => {
    // A plain object that is NOT apiErrorLike (no numeric statusCode) hits the
    // final `return fallback;` branch in inlineUserFacingError.
    const out = inlineUserFacingError({}, t('error.fallback.couldNotSaveSnapshot', 'zh'), 'zh');
    assert.equal(out, '无法保存支持快照');
    assert.doesNotMatch(out, /[A-Za-z]{4,}/);
  });

  it('en: 401 output is byte-identical to the prior hardcoded English literal', () => {
    const out = inlineUserFacingError(
      signInError,
      t('error.fallback.couldNotSaveSnapshot', 'en'),
      'en',
    );
    assert.equal(out, 'Could not save support snapshot: sign in again.');
  });

  it('en: raw last-resort fallback equals the original English literal', () => {
    const out = inlineUserFacingError({}, t('error.fallback.couldNotSaveSnapshot', 'en'), 'en');
    assert.equal(out, 'Could not save support snapshot');
  });

  it('en fallback values are byte-identical to the former call-site literals', () => {
    // Parity invariant: the en side of every error.fallback.* key must equal the
    // exact English string that used to be passed inline at the call site.
    const expected = {
      'error.fallback.couldNotLoadAegis': 'Could not load Aegis',
      'error.fallback.couldNotOpenDetail': 'Could not open this detail',
      'error.fallback.actionFailed': 'Action failed',
      'error.fallback.reminderActionFailed': 'Reminder action failed',
      'error.fallback.avatarServerSyncFailed': 'Avatar saved on this device, but server sync failed',
      'error.fallback.avatarAutoRefreshFailed': 'Avatar auto-refresh update failed',
      'error.fallback.avatarGenerationFailed': 'Aegis avatar generation failed',
      'error.fallback.couldNotPauseChecks': 'Could not pause automatic checks',
      'error.fallback.couldNotResumeChecks': 'Could not resume automatic checks',
      'error.fallback.couldNotUpdateLanguage': 'Could not update language',
      'error.fallback.couldNotUpdateProvider': 'Could not update AI provider',
      'error.fallback.couldNotUpdateModel': 'Could not update AI model',
      'error.fallback.couldNotUpdatePaidApiFallback': 'Could not update paid API fallback',
      'error.fallback.couldNotEnableWebPush': 'Could not enable WebApp notifications',
      'error.fallback.couldNotDisableWebPush': 'Could not turn off WebApp notifications',
      'error.fallback.couldNotSendWebPushTest': 'Could not send WebApp notification test',
      'error.fallback.couldNotStartCodexSignIn': 'Could not start Codex sign-in',
      'error.fallback.couldNotCheckCodexSignIn': 'Could not check Codex sign-in',
      'error.fallback.couldNotUseCodexForChat': 'Could not use Codex for chat',
      'error.fallback.couldNotUpdateCodexModel': 'Could not update Codex model',
      'error.fallback.couldNotStopUsingCodexForChat': 'Could not stop using Codex for chat',
      'error.fallback.couldNotStartGrokSignIn': 'Could not start Grok sign-in',
      'error.fallback.couldNotCheckGrokSignIn': 'Could not check Grok sign-in',
      'error.fallback.couldNotRevokeGrokConnection': 'Could not revoke Grok connection',
      'error.fallback.couldNotUseGrokForChat': 'Could not use Grok for chat',
      'error.fallback.couldNotStopUsingGrokForChat': 'Could not stop using Grok for chat',
      'error.fallback.couldNotSaveSnapshot': 'Could not save support snapshot',
      'error.fallback.couldNotOpenSupportDebugLease': 'Could not open support debug lease',
      'error.fallback.couldNotRevokeSupportDebugLease': 'Could not revoke support debug lease',
      'error.fallback.couldNotStartBackendDebugJob': 'Could not start backend debug job',
    };
    for (const [key, en] of Object.entries(expected)) {
      assert.equal(t(key, 'en'), en, `en mismatch for ${key}`);
    }
  });

  it('zh fallback values are present and contain no ASCII-letter runs (proper translation)', () => {
    const keys = [
      'error.fallback.couldNotLoadAegis',
      'error.fallback.couldNotOpenDetail',
      'error.fallback.actionFailed',
      'error.fallback.reminderActionFailed',
      'error.fallback.couldNotPauseChecks',
      'error.fallback.couldNotSaveSnapshot',
    ];
    for (const key of keys) {
      const zh = t(key, 'zh');
      assert.ok(zh && zh !== key, `missing zh for ${key}`);
    }
  });
});
