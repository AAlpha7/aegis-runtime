import { preferredLocale, t, tf } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

export type WebPushCapability = {
  supported: boolean;
  reason: string;
};

/**
 * Browser-side Web Push failure. The message is already owner copy in the
 * console locale; `userFacingErrors` recognizes the name and re-emits the
 * message verbatim behind the localized "could not enable…" fallback.
 */
export class WebPushError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'WebPushError';
  }
}

const ACTIVATION_TIMEOUT_ERROR = 'ServiceWorkerActivationTimeout';

export function webPushCapability(locale: Locale = preferredLocale()): WebPushCapability {
  if (typeof window === 'undefined' || typeof navigator === 'undefined') {
    return { supported: false, reason: t('webPush.capability.noBrowser', locale) };
  }
  if (!('serviceWorker' in navigator)) {
    return { supported: false, reason: t('webPush.capability.noServiceWorker', locale) };
  }
  if (!('PushManager' in window)) {
    return { supported: false, reason: t('webPush.capability.noPushManager', locale) };
  }
  if (!('Notification' in window)) {
    return { supported: false, reason: t('webPush.capability.noNotification', locale) };
  }
  return { supported: true, reason: t('webPush.capability.supported', locale) };
}

export function urlBase64ToUint8Array(value: string): Uint8Array<ArrayBuffer> {
  const padding = '='.repeat((4 - (value.length % 4)) % 4);
  const base64 = `${value}${padding}`.replace(/-/g, '+').replace(/_/g, '/');
  const raw = window.atob(base64);
  const output = new Uint8Array(new ArrayBuffer(raw.length));
  for (let index = 0; index < raw.length; index += 1) {
    output[index] = raw.charCodeAt(index);
  }
  return output;
}

function bytesFromBufferSource(value: BufferSource): Uint8Array {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
}

function pushSubscriptionUsesPublicKey(subscription: PushSubscription, publicKey: string): boolean {
  const currentKey = subscription.options.applicationServerKey;
  if (!currentKey) return true;
  const currentBytes = bytesFromBufferSource(currentKey);
  const expectedBytes = urlBase64ToUint8Array(publicKey);
  if (currentBytes.byteLength !== expectedBytes.byteLength) return false;
  for (let index = 0; index < expectedBytes.byteLength; index += 1) {
    if (currentBytes[index] !== expectedBytes[index]) return false;
  }
  return true;
}

async function ownerServiceWorkerRegistration(locale: Locale): Promise<ServiceWorkerRegistration> {
  try {
    const registration = await navigator.serviceWorker.register('/ui/service-worker.js', {
      scope: '/ui/',
      updateViaCache: 'none',
    });
    registration.update().catch(() => {});
    return await waitForServiceWorkerActive(registration);
  } catch (error) {
    if (errorName(error) === ACTIVATION_TIMEOUT_ERROR) {
      throw new WebPushError(t('webPush.error.activateTimeout', locale));
    }
    throw new WebPushError(tf('error.withDetail', locale, {
      fallback: t('webPush.error.registerFailed', locale),
      detail: webPushBrowserErrorText(error, locale),
    }));
  }
}

async function waitForServiceWorkerActive(registration: ServiceWorkerRegistration): Promise<ServiceWorkerRegistration> {
  if (registration.active) return registration;

  return new Promise<ServiceWorkerRegistration>((resolve, reject) => {
    const candidate = registration.installing ?? registration.waiting ?? null;
    const timeout = window.setTimeout(() => {
      cleanup();
      // Sentinel: the caller maps it to owner copy in the console locale.
      const error = new Error('service worker did not activate in time');
      error.name = ACTIVATION_TIMEOUT_ERROR;
      reject(error);
    }, 8000);

    function cleanup() {
      window.clearTimeout(timeout);
      candidate?.removeEventListener('statechange', onStateChange);
      navigator.serviceWorker.removeEventListener('controllerchange', onStateChange);
    }

    function onStateChange() {
      if (!registration.active) return;
      cleanup();
      resolve(registration);
    }

    candidate?.addEventListener('statechange', onStateChange);
    navigator.serviceWorker.addEventListener('controllerchange', onStateChange);
    onStateChange();
  });
}

export async function currentWebPushSubscription(locale: Locale = preferredLocale()): Promise<PushSubscription | null> {
  const registration = await ownerServiceWorkerRegistration(locale);
  return registration.pushManager.getSubscription();
}

export async function subscribeOwnerWebPush(publicKey: string, locale: Locale = preferredLocale()): Promise<PushSubscription> {
  const capability = webPushCapability(locale);
  if (!capability.supported) throw new WebPushError(capability.reason);
  if (Notification.permission === 'denied') {
    throw new WebPushError(t('webPush.error.permissionBlocked', locale));
  }
  const permission = Notification.permission === 'granted'
    ? 'granted'
    : await Notification.requestPermission();
  if (permission !== 'granted') {
    throw new WebPushError(t('webPush.error.permissionNotGranted', locale));
  }
  const registration = await ownerServiceWorkerRegistration(locale);
  const existing = await registration.pushManager.getSubscription();
  if (existing) {
    if (pushSubscriptionUsesPublicKey(existing, publicKey)) return existing;
    await existing.unsubscribe();
  }
  try {
    return await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(publicKey),
    });
  } catch (firstError) {
    try {
      return await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: publicKey,
      });
    } catch (secondError) {
      const second = webPushBrowserErrorText(secondError, locale);
      const first = webPushBrowserErrorText(firstError, locale);
      const detail = first === second
        ? second
        : tf('webPush.error.subscribeDetail', locale, { second, first });
      throw new WebPushError(tf('error.withDetail', locale, {
        fallback: t('webPush.error.subscribeFailed', locale),
        detail,
      }));
    }
  }
}

export async function unsubscribeOwnerWebPush(locale: Locale = preferredLocale()): Promise<string | null> {
  const registration = await ownerServiceWorkerRegistration(locale);
  const subscription = await registration.pushManager.getSubscription();
  if (!subscription) return null;
  const endpoint = subscription.endpoint;
  await subscription.unsubscribe();
  return endpoint;
}

/**
 * Owner copy for an error raised by the browser's Notification / PushManager
 * / ServiceWorker APIs. Common DOMException names map to a full localized
 * sentence; anything else keeps a generic localized message plus the raw
 * name (or, for a bare `Error`, its message) in parentheses so support can
 * still tell what the browser said.
 */
export function webPushBrowserErrorText(error: unknown, locale: Locale): string {
  const name = errorName(error);
  const message = errorMessage(error);
  if (name === 'NotAllowedError') return t('webPush.browserError.notAllowed', locale);
  if (name === 'AbortError') return t('webPush.browserError.abort', locale);
  if (name === 'InvalidStateError') return t('webPush.browserError.invalidState', locale);
  if (name === ACTIVATION_TIMEOUT_ERROR) return t('webPush.error.activateTimeout', locale);
  if (
    name === 'NetworkError'
    || (name === 'TypeError' && /fetch|network|load failed/i.test(message))
    || /failed to fetch|networkerror|network request failed|load failed/i.test(message)
  ) {
    return t('webPush.browserError.network', locale);
  }
  const raw = name && name !== 'Error' ? name : (message || 'unknown');
  return tf('webPush.browserError.unknown', locale, { name: raw.slice(0, 120) });
}

function errorName(error: unknown): string {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException) return error.name;
  if (error instanceof Error && typeof error.name === 'string') return error.name;
  if (error && typeof error === 'object' && typeof (error as { name?: unknown }).name === 'string') {
    return (error as { name: string }).name;
  }
  return '';
}

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.message.trim()) return error.message.trim();
  if (typeof error === 'string' && error.trim()) return error.trim();
  return '';
}
