import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';

const modulePath = process.env.VNEXT_WEB_PUSH_MODULE;
assert.ok(modulePath, 'VNEXT_WEB_PUSH_MODULE is required');

const {
  WebPushError,
  subscribeOwnerWebPush,
  webPushBrowserErrorText,
  webPushCapability,
} = await import(modulePath);

class FakeDomException extends Error {
  constructor(message, name) {
    super(message);
    this.name = name;
  }
}

describe('web push browser errors are owner copy in the console locale', () => {
  it('maps the common DOMException names to zh / en sentences', () => {
    const denied = new FakeDomException('Registration failed - permission denied', 'NotAllowedError');
    assert.equal(webPushBrowserErrorText(denied, 'zh'), '浏览器拒绝了通知权限');
    assert.equal(webPushBrowserErrorText(denied, 'en'), 'the browser refused notification permission');

    const aborted = new FakeDomException('Registration failed - push service error', 'AbortError');
    assert.equal(webPushBrowserErrorText(aborted, 'zh'), '推送服务没有完成订阅，请稍后重试');
    assert.equal(webPushBrowserErrorText(aborted, 'en'), 'the push service did not complete the subscription; try again shortly');

    const notReady = new FakeDomException('Subscription failed - no active Service Worker', 'InvalidStateError');
    assert.equal(webPushBrowserErrorText(notReady, 'zh'), '通知服务尚未就绪，请关闭并重新打开应用');
    assert.equal(webPushBrowserErrorText(notReady, 'en'), 'the notification service is not ready yet; close and reopen the app');
  });

  it('maps network failures (fetch TypeError, NetworkError, WebKit "Load failed") to one network sentence', () => {
    assert.equal(webPushBrowserErrorText(new TypeError('Failed to fetch'), 'zh'), '连不上推送服务，请检查网络');
    assert.equal(webPushBrowserErrorText(new FakeDomException('A network error occurred.', 'NetworkError'), 'en'), 'the push service could not be reached; check your network');
    assert.equal(webPushBrowserErrorText(new TypeError('Load failed'), 'zh'), '连不上推送服务，请检查网络');
    assert.equal(webPushBrowserErrorText(new Error('NetworkError when attempting to fetch resource.'), 'zh'), '连不上推送服务，请检查网络');
  });

  it('keeps a generic localized sentence plus the raw name for unknown errors', () => {
    const odd = new FakeDomException('quota', 'QuotaExceededError');
    assert.equal(webPushBrowserErrorText(odd, 'zh'), '浏览器返回了未知错误（QuotaExceededError）');
    assert.equal(webPushBrowserErrorText(odd, 'en'), 'the browser returned an unknown error (QuotaExceededError)');
    // A bare Error has no informative name: keep its message as the diagnostic.
    assert.equal(
      webPushBrowserErrorText(new Error('Push service rejected applicationServerKey'), 'zh'),
      '浏览器返回了未知错误（Push service rejected applicationServerKey）',
    );
    assert.equal(webPushBrowserErrorText('boom', 'en'), 'the browser returned an unknown error (boom)');
    assert.equal(webPushBrowserErrorText(undefined, 'en'), 'the browser returned an unknown error (unknown)');
    // No English leaks into the zh sentence for a known-name error.
    assert.doesNotMatch(webPushBrowserErrorText(odd, 'zh'), /[a-z]{3,}\s[a-z]{3,}/i);
  });
});

describe('web push capability + permission copy', () => {
  const globalsToRestore = ['window', 'navigator', 'Notification'].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]);
  afterEach(() => {
    for (const [key, descriptor] of globalsToRestore) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });

  it('explains "not a browser" in the requested locale', () => {
    Object.defineProperty(globalThis, 'window', { value: undefined, configurable: true, writable: true });
    assert.deepEqual(webPushCapability('zh'), { supported: false, reason: 'Web 应用通知只能在浏览器里使用。' });
    assert.deepEqual(webPushCapability('en'), { supported: false, reason: 'Web Push is only available in a browser.' });
  });

  it('reports a browser without PushManager in zh', () => {
    Object.defineProperty(globalThis, 'window', { value: { Notification: {} }, configurable: true, writable: true });
    Object.defineProperty(globalThis, 'navigator', { value: { serviceWorker: {} }, configurable: true, writable: true });
    assert.equal(webPushCapability('zh').reason, '这个浏览器无法接收 Web 应用通知。');
    assert.equal(webPushCapability('zh').supported, false);
  });

  it('throws a WebPushError with zh copy when notifications are blocked', async () => {
    const notification = { permission: 'denied' };
    Object.defineProperty(globalThis, 'window', { value: { PushManager: {}, Notification: notification }, configurable: true, writable: true });
    Object.defineProperty(globalThis, 'navigator', { value: { serviceWorker: {} }, configurable: true, writable: true });
    Object.defineProperty(globalThis, 'Notification', { value: notification, configurable: true, writable: true });
    await assert.rejects(
      () => subscribeOwnerWebPush('B'.repeat(87), 'zh'),
      (error) => error instanceof WebPushError
        && error.name === 'WebPushError'
        && error.message === '这个浏览器已屏蔽通知。请在浏览器设置里允许通知后重试。',
    );
    await assert.rejects(
      () => subscribeOwnerWebPush('B'.repeat(87), 'en'),
      (error) => error.message === 'Notifications are blocked for this browser. Allow them in the browser settings, then try again.',
    );
  });
});
