import { useSyncExternalStore } from 'react';

/**
 * Browser connectivity as a subscribable store: `navigator.onLine` plus the
 * window `online` / `offline` events. The app shell shows the offline banner
 * while this reads false and hides it on reconnect.
 *
 * The store is built over an injectable source so the event wiring is
 * unit-testable in node (see tests/vnextStatusRails.test.mjs); the hook binds
 * it to the real window.
 */
export interface OnlineStatusSource {
  /** `navigator.onLine`; undefined when the platform does not report it. */
  onLine(): boolean | undefined;
  addEventListener(type: 'online' | 'offline', listener: () => void): void;
  removeEventListener(type: 'online' | 'offline', listener: () => void): void;
}

export interface OnlineStatusStore {
  subscribe(listener: () => void): () => void;
  /** true when online, or when the platform cannot say (never a false alarm). */
  getSnapshot(): boolean;
}

export function readOnlineStatus(source: OnlineStatusSource): boolean {
  try {
    return source.onLine() !== false;
  } catch {
    return true;
  }
}

export function createOnlineStatusStore(source: OnlineStatusSource): OnlineStatusStore {
  return {
    subscribe(listener) {
      source.addEventListener('online', listener);
      source.addEventListener('offline', listener);
      return () => {
        source.removeEventListener('online', listener);
        source.removeEventListener('offline', listener);
      };
    },
    getSnapshot: () => readOnlineStatus(source),
  };
}

function windowSource(): OnlineStatusSource | null {
  if (typeof window === 'undefined') return null;
  return {
    onLine: () => (typeof navigator !== 'undefined' ? navigator.onLine : undefined),
    addEventListener: (type, listener) => window.addEventListener(type, listener),
    removeEventListener: (type, listener) => window.removeEventListener(type, listener),
  };
}

const ALWAYS_ONLINE: OnlineStatusStore = {
  subscribe: () => () => {},
  getSnapshot: () => true,
};

const BROWSER_STORE: OnlineStatusStore = (() => {
  const source = windowSource();
  return source ? createOnlineStatusStore(source) : ALWAYS_ONLINE;
})();

/** `true` while the browser reports connectivity (or cannot report at all). */
export function useOnlineStatus(store: OnlineStatusStore = BROWSER_STORE): boolean {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, () => true);
}
