import assert from 'node:assert/strict';
import { test } from 'node:test';

const modulePath = process.env.USER_FACING_ERRORS_MODULE;
if (!modulePath) {
  throw new Error('USER_FACING_ERRORS_MODULE is required');
}

const { inlineUserFacingError, offerLocalHostStepUp } = await import(modulePath);

test('model failure and credit errors retain runtime fallback disclosures in both locales', () => {
  for (const statusCode of [402, 503]) {
    for (const locale of ['zh', 'en']) {
      const notice = locale === 'zh'
        ? '⚠️ 首选服务失败，已尝试备用服务；能力可能不同。'
        : '⚠️ Primary service failed; a fallback service was attempted and capabilities may differ.';
      const message = inlineUserFacingError({ statusCode,
        message: `Assistant model is unavailable: upstream timeout\n\n${notice}`,
      }, 'Request failed', locale);
      assert.ok(message.includes(notice), message);
      assert.equal(message.split(notice).length, 2);
      const structured = inlineUserFacingError({ statusCode,
        message: `Assistant model is unavailable: upstream timeout\n\n${notice}`,
        problem: { status: 'error', error: `upstream timeout\n\n${notice}` },
      }, 'Request failed', locale);
      assert.equal(structured.split(notice).length, 2, structured);
    }
  }
});

test('inlineUserFacingError turns response 5xx failures into user-facing recovery copy', () => {
  const message = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'Request failed (503): upstream unavailable', statusCode: 503 },
    'Request failed',
  );

  assert.equal(
    message,
    'Request failed: I did not get a complete answer. Try again in a moment.',
  );
  assert.equal(message.includes('returned status'), false);
  assert.equal(message.includes('503'), false);
  assert.equal(message.includes('temporarily unavailable'), false);
  assert.equal(message.includes('Connection needs attention'), false);
});

test('inlineUserFacingError preserves subscription avatar recovery details', () => {
  const message = inlineUserFacingError(
    {
      name: 'ApiResponseError',
      message: 'Aegis avatar generation failed (503): Reconnect Grok in Settings before generating Aegis avatar; automatic token refresh failed: invalid_grant',
      statusCode: 503,
    },
    'Aegis avatar generation failed',
  );

  assert.equal(
    message,
    'Aegis avatar generation failed: Aegis avatar generation failed (503): Reconnect Grok in Settings before generating Aegis avatar; automatic token refresh failed: invalid_grant',
  );
  assert.equal(message.includes('I did not get a complete answer'), false);
});

test('inlineUserFacingError preserves avatar validation details', () => {
  const message = inlineUserFacingError(
    {
      name: 'ApiResponseError',
      message: 'Aegis avatar generation failed (400): Avatar image is too large',
      statusCode: 400,
    },
    'Aegis avatar generation failed',
  );

  assert.equal(
    message,
    'Aegis avatar generation failed: Aegis avatar generation failed (400): Avatar image is too large',
  );
  assert.equal(message.includes('Try again'), false);
});

test('inlineUserFacingError turns sensitive access problems into action copy', () => {
  const message = inlineUserFacingError(
    {
      name: 'ApiResponseError',
      message: 'MFA required before this protected action can run.',
      statusCode: 403,
      problem: { status: 'mfa_required' },
    },
    'Request failed',
  );

  assert.equal(message, 'Extra owner confirmation is required before Aegis can continue.');
});

test('inlineUserFacingError explains missing Google consent without security jargon', () => {
  const message = inlineUserFacingError(
    {
      name: 'ApiResponseError',
      message: 'Consent required before sensitive content can be read.',
      statusCode: 403,
      problem: { error_kind: 'consent_required' },
    },
    'Request failed',
  );

  assert.equal(message, 'Google needs permission before this selected detail can open.');
});

test('inlineUserFacingError surfaces a structured backend error (S2 W2: problem.error, not the generic 5xx bucket)', () => {
  const structuredError = {
    name: 'ApiResponseError',
    message: 'Inbox is briefly unavailable while it rebuilds',
    statusCode: 503,
    problem: {
      status: 'error',
      error: 'Inbox is briefly unavailable while it rebuilds',
      error_kind: 'page1_store_unavailable',
      retryable: true,
      hint: 'Retry in a few seconds.',
    },
  };

  assert.equal(
    inlineUserFacingError(structuredError, 'Work queue load failed'),
    'Work queue load failed: Inbox is briefly unavailable while it rebuilds',
  );
  // zh locale renders the localized withDetail frame around the backend string.
  assert.equal(
    inlineUserFacingError(structuredError, '工作队列加载失败', 'zh'),
    '工作队列加载失败：Inbox is briefly unavailable while it rebuilds',
  );
});

test('inlineUserFacingError structured-error branch does not shadow the specific status buckets', () => {
  // 401 with a structured body keeps the sign-in copy (specific bucket wins).
  assert.equal(
    inlineUserFacingError(
      {
        name: 'ApiResponseError',
        message: 'unauthorized',
        statusCode: 401,
        problem: { status: 'error', error: 'unauthorized' },
      },
      'Queue load failed',
    ),
    'Queue load failed: sign in again.',
  );
  // Sensitive problems keep their action copy even though they carry `error`.
  assert.equal(
    inlineUserFacingError(
      {
        name: 'ApiResponseError',
        message: 'mfa',
        statusCode: 403,
        problem: { status: 'mfa_required', error: 'mfa_required' },
      },
      'Request failed',
    ),
    'Extra owner confirmation is required before Aegis can continue.',
  );
});

test('inlineUserFacingError keeps auth and network recovery specific', () => {
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'unauthorized', statusCode: 401 }, 'Queue load failed'),
    'Queue load failed: sign in again.',
  );
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'forbidden', statusCode: 403 }, 'Request failed'),
    'Request failed: this action is not available for the current item or session. Refresh this item, then retry one of the shown actions.',
  );
  assert.equal(
    inlineUserFacingError(new Error('Failed to fetch'), 'Request failed'),
    'Request failed: I could not connect. Check your network, then try again.',
  );
});

test('inlineUserFacingError distinguishes an unreachable network from a server 5xx (both locales)', () => {
  // fetch threw (offline / DNS / captive portal): the request never reached Aegis.
  const offline = new TypeError('Failed to fetch');
  assert.equal(
    inlineUserFacingError(offline, 'Request failed', 'en'),
    'Request failed: I could not connect. Check your network, then try again.',
  );
  assert.equal(
    inlineUserFacingError(offline, '请求失败', 'zh'),
    '请求失败：网络连不上。请检查网络后重试。',
  );
  // WebKit / Firefox spellings of the same failure.
  assert.match(inlineUserFacingError(new TypeError('Load failed'), '请求失败', 'zh'), /网络连不上/);
  assert.match(inlineUserFacingError(new TypeError('NetworkError when attempting to fetch resource.'), '请求失败', 'zh'), /网络连不上/);
  // status 0 = no HTTP response at all: same bucket as a thrown fetch.
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'no response', statusCode: 0 }, '请求失败', 'zh'),
    '请求失败：网络连不上。请检查网络后重试。',
  );
  // A real 5xx means Aegis answered but failed: distinct copy, no network blame.
  const serverError = { name: 'ApiResponseError', message: 'Request failed (503): upstream unavailable', statusCode: 503 };
  assert.equal(
    inlineUserFacingError(serverError, 'Request failed', 'en'),
    'Request failed: I did not get a complete answer. Try again in a moment.',
  );
  assert.equal(
    inlineUserFacingError(serverError, '请求失败', 'zh'),
    '请求失败：我没有得到完整的回复。请稍后重试。',
  );
  assert.doesNotMatch(inlineUserFacingError(serverError, '请求失败', 'zh'), /网络/);
});

test('inlineUserFacingError re-emits localized WebPushError copy behind the localized fallback', () => {
  // vnext/webPush.ts throws already-localized owner copy under the name
  // 'WebPushError'; it must be surfaced verbatim, not collapsed into the
  // generic bucket (the zh text no longer matches the English regex).
  const pushError = Object.assign(new Error('通知服务注册失败：浏览器拒绝了通知权限'), { name: 'WebPushError' });
  assert.equal(
    inlineUserFacingError(pushError, '无法启用 WebApp 通知', 'zh'),
    '无法启用 WebApp 通知：通知服务注册失败：浏览器拒绝了通知权限',
  );
});

test('inlineUserFacingError preserves Web Push browser diagnostics', () => {
  const message = inlineUserFacingError(
    new Error('WebApp notification subscription failed after browser permission was granted: Push service rejected applicationServerKey'),
    'Could not enable WebApp notifications',
  );

  assert.equal(
    message,
    'Could not enable WebApp notifications: WebApp notification subscription failed after browser permission was granted: Push service rejected applicationServerKey',
  );
});

test('inlineUserFacingError preserves Web Push backend diagnostics', () => {
  const message = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'WebApp notification setup failed (400): Invalid Web Push auth key.', statusCode: 400 },
    'Could not enable WebApp notifications',
  );

  assert.equal(
    message,
    'Could not enable WebApp notifications: WebApp notification setup failed (400): Invalid Web Push auth key.',
  );
});

test('inlineUserFacingError explains exhausted model credits without connection wording', () => {
  const message = inlineUserFacingError(
    {
      name: 'ApiResponseError',
      message: 'Aegis model provider credits are exhausted. No action was taken.',
      statusCode: 402,
    },
    "I couldn't finish that",
  );

  assert.equal(
    message,
    "I couldn't finish that: model credits are exhausted. Add credits or switch the approved model setting, then try again.",
  );
  assert.equal(message.includes('could not reach'), false);
  assert.equal(message.includes('provider'), false);
});

test('inlineUserFacingError keeps command failures actionable without raw load wording', () => {
  const message = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'Command failed (400): Load failed', statusCode: 400 },
    "I couldn't finish that",
  );

  assert.equal(message, "I couldn't finish that. Try again.");
  assert.equal(message.includes('Load failed'), false);
  assert.equal(message.includes('Command failed'), false);
});

test('inlineUserFacingError explains stale item actions without raw HTTP wording', () => {
  const message = inlineUserFacingError(
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
  assert.equal(message.includes('400'), false);
  assert.equal(message.includes('does not apply'), false);
  assert.equal(message.includes('Refresh Work'), false);
});

test('inlineUserFacingError does not expose low-level network wording', () => {
  const message = inlineUserFacingError(new Error('NetworkError: Load failed'), "I couldn't finish that");

  assert.equal(message.includes('connection was interrupted'), false);
  assert.equal(message.includes('could not reach'), false);
  assert.equal(message.includes('Load failed'), false);
  assert.equal(message.includes('NetworkError'), false);
  // Network failures get the network-unreachable copy, not the 5xx copy.
  assert.equal(message, "I couldn't finish that: I could not connect. Check your network, then try again.");
  assert.equal(message.includes('I did not get a complete answer'), false);
});

test('inlineUserFacingError resolves an auth error in both locales via the catalog', () => {
  const en = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'unauthorized', statusCode: 401 },
    'Queue load failed',
    'en',
  );
  const zh = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'unauthorized', statusCode: 401 },
    '加载失败',
    'zh',
  );

  assert.equal(en, 'Queue load failed: sign in again.');
  assert.equal(zh, '加载失败：请重新登录。');
  assert.notEqual(en, zh);
});

test('inlineUserFacingError localizes sensitive-access copy for both locales', () => {
  const mfaEn = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'mfa', statusCode: 403, problem: { status: 'mfa_required' } },
    'Request failed',
    'en',
  );
  const mfaZh = inlineUserFacingError(
    { name: 'ApiResponseError', message: 'mfa', statusCode: 403, problem: { status: 'mfa_required' } },
    '请求失败',
    'zh',
  );

  assert.equal(mfaEn, 'Extra owner confirmation is required before Aegis can continue.');
  assert.equal(mfaZh, '在安琪继续之前，需要你再确认一次。');
});

test('inlineUserFacingError localizes the busy/timeout/incomplete recovery copy in zh', () => {
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'busy', statusCode: 429 }, '请求失败', 'zh'),
    '请求失败：安琪当前繁忙。请稍后重试。',
  );
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'slow', statusCode: 504 }, '请求失败', 'zh'),
    '请求失败：耗时过长。请稍后重试。',
  );
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'oops', statusCode: 500 }, '请求失败', 'zh'),
    '请求失败：我没有得到完整的回复。请稍后重试。',
  );
});

test('inlineUserFacingError defaults to English when no locale is passed (back-compat)', () => {
  assert.equal(
    inlineUserFacingError({ name: 'ApiResponseError', message: 'unauthorized', statusCode: 401 }, 'Queue load failed'),
    'Queue load failed: sign in again.',
  );
});

test('model unavailable shows a specific reason and still hides the bare 503 line', () => {
  const detailed = Object.assign(new Error('Command failed (503): Assistant model is unavailable: connection refused'), {
    name: 'ApiResponseError',
    statusCode: 503,
  });
  const zh = inlineUserFacingError(detailed, '我没能完成这件事', 'zh');
  assert.equal(zh, '我没能完成这件事：connection refused');
  assert.doesNotMatch(zh, /Command failed|503|Assistant model is unavailable/);
});

test('local host session offers a second confirmation only for mfa_required', () => {
  assert.equal(offerLocalHostStepUp('owner@local.invalid', { error: 'mfa_required' }), true);
  assert.equal(offerLocalHostStepUp('owner@local.invalid', { status: 'mfa_required' }), true);
  assert.equal(offerLocalHostStepUp('owner@local.invalid', { error: 'forbidden' }), false);
  assert.equal(offerLocalHostStepUp('L@example.com', { error: 'mfa_required' }), false);
  assert.equal(offerLocalHostStepUp(undefined, { error: 'mfa_required' }), false);
});

test('model unavailable never shows the raw 503 provider line to the owner', () => {
  const err = Object.assign(new Error('Command failed (503): Assistant model is unavailable'), {
    name: 'ApiResponseError',
    statusCode: 503,
  });
  const zh = inlineUserFacingError(err, '我没能完成这件事', 'zh');
  assert.equal(zh, '我没能完成这件事：AI 这会儿连不上，我会自动换备用模型，请过一会儿再发一次。');
  assert.doesNotMatch(zh, /Command failed|503|unavailable/);
  const en = inlineUserFacingError(err, "I couldn't finish that", 'en');
  assert.match(en, /backup model/);
  assert.doesNotMatch(en, /Command failed|503/);
});
