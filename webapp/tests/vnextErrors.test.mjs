import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_ERRORS_MODULE;
if (!modulePath) {
  throw new Error('VNEXT_ERRORS_MODULE is required');
}

const { displayError, assistantDisplayError, isLocalStaticPreviewSessionMiss } = await import(modulePath);

describe('vNext owner-facing errors', () => {
  it('keeps action and detail errors on the shared sanitized error path', () => {
    assert.equal(
      displayError(
        { name: 'ApiResponseError', message: 'Command failed (400): Load failed', statusCode: 400 },
        'Action failed',
      ),
      'Action failed. Try again.',
    );
    // A thrown fetch is reported as an unreachable network, distinct from a
    // server failure (which still reads "I did not get a complete answer").
    assert.equal(
      displayError(new Error('NetworkError: Load failed'), 'Could not open this detail'),
      'Could not open this detail: I could not connect. Check your network, then try again.',
    );
    assert.equal(
      displayError({ name: 'ApiResponseError', message: 'Request failed (502): bad gateway', statusCode: 502 }, 'Could not open this detail'),
      'Could not open this detail: I did not get a complete answer. Try again in a moment.',
    );
  });

  it('explains stale or mismatched item actions without exposing raw HTTP errors', () => {
    const message = displayError(
      {
        name: 'ApiResponseError',
        message: 'Action failed (400): Action does not apply to this item',
        statusCode: 400,
      },
      'Action failed',
    );

    assert.equal(
      message,
      'This item changed before the action ran. Open it again, then choose one of the available actions.',
    );
    assert.doesNotMatch(message, /400|does not apply|Action failed/i);
  });

  it('does not leak low-level provider or transport wording from assistant failures', () => {
    const message = assistantDisplayError({
      name: 'ApiResponseError',
      message: 'Aegis model provider credits are exhausted. No action was taken.',
      statusCode: 402,
    });

    assert.equal(
      message,
      "I couldn't finish that: model credits are exhausted. Add credits or switch the approved model setting, then try again.",
    );
    assert.doesNotMatch(message, /provider|No action was taken/i);
  });

  it('localizes the assistant failure fallback into the display_language (zh)', () => {
    // Owner complaint: assistant errors leaked the English "I couldn't finish
    // that" fallback into a zh session. Both the generic fallback and the
    // {fallback}-prefixed specific templates must render in zh.
    const generic = assistantDisplayError({ name: 'Error', message: 'Something broke' }, 'zh');
    assert.equal(generic, '我没能完成这件事');
    assert.doesNotMatch(generic, /finish/i);

    const credits = assistantDisplayError({
      name: 'ApiResponseError',
      message: 'Aegis model provider credits are exhausted. No action was taken.',
      statusCode: 402,
    }, 'zh');
    assert.match(credits, /^我没能完成这件事：模型额度已用尽/);
    assert.doesNotMatch(credits, /I couldn't finish that|provider/i);
  });

  it('recognizes local static preview misses without depending on api runtime classes', () => {
    const originalLocation = globalThis.window?.location;
    globalThis.window = {
      location: { hostname: '127.0.0.1' },
    };

    try {
      assert.equal(isLocalStaticPreviewSessionMiss({ statusCode: 404 }), true);
      assert.equal(isLocalStaticPreviewSessionMiss({ statusCode: 500 }), false);
    } finally {
      if (originalLocation) {
        globalThis.window = { location: originalLocation };
      } else {
        delete globalThis.window;
      }
    }
  });
});
