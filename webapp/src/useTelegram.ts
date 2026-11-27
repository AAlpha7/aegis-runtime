/** Hook for Telegram WebApp SDK integration (Bot API 9.6) */

import { useEffect, useState } from 'react';

const MAX_STORAGE_VALUE_CHARS = 16_384;
const TELEGRAM_SDK_SCRIPT_ID = 'aegis-telegram-webapp-sdk';
const TELEGRAM_SDK_SRC = 'https://telegram.org/js/telegram-web-app.js';

export function shouldLoadTelegramSdk(locationLike: Pick<Location, 'hash' | 'search'>): boolean {
  const markerText = `${locationLike.search || ''}&${locationLike.hash || ''}`;
  return /(?:^|[?#&])tgWebApp(?:Data|Version|Platform|ThemeParams)=/i.test(markerText);
}

function currentTelegramWebApp(): TelegramWebApp | undefined {
  return window.Telegram?.WebApp;
}

// ── Main Hook ─────────────────────────────────────────────────────────────────

export function useTelegram() {
  const [tg, setTg] = useState<TelegramWebApp | undefined>(() => currentTelegramWebApp());

  useEffect(() => {
    if (tg || !shouldLoadTelegramSdk(window.location)) return undefined;

    const existing = document.getElementById(TELEGRAM_SDK_SCRIPT_ID) as HTMLScriptElement | null;
    const script = existing ?? document.createElement('script');
    script.id = TELEGRAM_SDK_SCRIPT_ID;
    script.src = TELEGRAM_SDK_SRC;
    script.async = true;
    script.referrerPolicy = 'no-referrer';
    const handleLoad = () => setTg(currentTelegramWebApp());
    script.addEventListener('load', handleLoad);
    if (!existing) {
      document.head.appendChild(script);
    }
    return () => {
      script.removeEventListener('load', handleLoad);
    };
  }, [tg]);

  useEffect(() => {
    if (!tg) return;
    safeCall(() => tg.ready());
    safeCall(() => tg.expand());

    // Bot API 8.0+: fullscreen + orientation lock for console experience
    if (tg.isVersionAtLeast?.('8.0')) {
      safeCall(() => tg.requestFullscreen());
      safeCall(() => tg.lockOrientation());
    }

    // Bot API 7.7+: disable vertical swipes to prevent accidental close
    if (tg.isVersionAtLeast?.('7.7')) {
      safeCall(() => tg.disableVerticalSwipes());
    }
  }, [tg]);

  // Apply safe area insets as CSS vars
  useEffect(() => {
    if (!tg) return;
    applySafeArea(tg);

    if (tg.isVersionAtLeast?.('8.0')) {
      const handler = () => applySafeArea(tg);
      safeCall(() => tg.onEvent('safeAreaChanged', handler));
      safeCall(() => tg.onEvent('contentSafeAreaChanged', handler));
      return () => {
        safeCall(() => tg.offEvent('safeAreaChanged', handler));
        safeCall(() => tg.offEvent('contentSafeAreaChanged', handler));
      };
    }
  }, [tg]);

  return {
    tg,
    initData: tg?.initData ?? '',
    user: tg?.initDataUnsafe?.user,
    colorScheme: tg?.colorScheme ?? 'light',
    themeParams: tg?.themeParams ?? {},
    mainButton: tg?.MainButton,
    secondaryButton: tg?.SecondaryButton,
    backButton: tg?.BackButton,
    settingsButton: tg?.SettingsButton,
    haptic: tg?.HapticFeedback,
    close: () => safeCall(() => tg?.close()),
  };
}

function safeCall<T>(fn: () => T): T | undefined {
  try {
    return fn();
  } catch {
    return undefined;
  }
}

function safeInvoke(fn: () => void): boolean {
  try {
    fn();
    return true;
  } catch {
    return false;
  }
}

export function safeCssColor(value: string | undefined): string | undefined {
  if (!value) return undefined;
  if (/^#[0-9a-f]{3}(?:[0-9a-f]{3})?$/i.test(value)) return value;
  if (/^rgb\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*\)$/i.test(value)) return value;
  if (/^rgba\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*(?:0|1|0?\.\d+)\s*\)$/i.test(value)) return value;
  return undefined;
}

export function safeInsetPx(value: unknown): string {
  const n = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(n)) return '0px';
  return `${Math.max(0, Math.min(240, n))}px`;
}

export function parseObjectJson(value: string | undefined): Record<string, unknown> | null {
  if (!value || value.length > MAX_STORAGE_VALUE_CHARS) return null;
  try {
    const parsed = JSON.parse(value);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
  } catch {
    return null;
  }
  return null;
}

// ── Theme ─────────────────────────────────────────────────────────────────────

/** Apply Telegram theme params as CSS custom properties on :root */
export function applyTelegramTheme(themeParams: TelegramThemeParams) {
  const root = document.documentElement;
  const map: Record<string, string | undefined> = {
    '--tg-bg': themeParams.bg_color,
    '--tg-text': themeParams.text_color,
    '--tg-hint': themeParams.hint_color,
    '--tg-link': themeParams.link_color,
    '--tg-button': themeParams.button_color,
    '--tg-button-text': themeParams.button_text_color,
    '--tg-secondary-bg': themeParams.secondary_bg_color,
    '--tg-header-bg': themeParams.header_bg_color,
    '--tg-accent': themeParams.accent_text_color,
    '--tg-section-bg': themeParams.section_bg_color,
    '--tg-section-header': themeParams.section_header_text_color,
    '--tg-subtitle': themeParams.subtitle_text_color,
    '--tg-destructive': themeParams.destructive_text_color,
  };

  for (const [prop, value] of Object.entries(map)) {
    const safeValue = safeCssColor(value);
    if (safeValue) root.style.setProperty(prop, safeValue);
  }
}

// ── Safe Area ─────────────────────────────────────────────────────────────────

function applySafeArea(tg: TelegramWebApp) {
  const root = document.documentElement;

  if (tg.safeAreaInset) {
    const { top, bottom, left, right } = tg.safeAreaInset;
    root.style.setProperty('--safe-top', safeInsetPx(top));
    root.style.setProperty('--safe-bottom', safeInsetPx(bottom));
    root.style.setProperty('--safe-left', safeInsetPx(left));
    root.style.setProperty('--safe-right', safeInsetPx(right));
  }

  if (tg.contentSafeAreaInset) {
    const { top, bottom, left, right } = tg.contentSafeAreaInset;
    root.style.setProperty('--content-safe-top', safeInsetPx(top));
    root.style.setProperty('--content-safe-bottom', safeInsetPx(bottom));
    root.style.setProperty('--content-safe-left', safeInsetPx(left));
    root.style.setProperty('--content-safe-right', safeInsetPx(right));
  }
}

// ── Home Screen ───────────────────────────────────────────────────────────────

/** Prompt user to add Mini App to home screen (once per session) */
export function promptHomeScreen(tg: TelegramWebApp | undefined) {
  if (!tg?.isVersionAtLeast?.('8.0')) return;
  safeCall(() => tg.checkHomeScreenStatus?.((status) => {
    if (status === 'unknown' || status === 'missed') {
      safeCall(() => tg.addToHomeScreen?.());
    }
  }));
}

// ── DeviceStorage helpers (Bot API 9.0+) ──────────────────────────────────────

/** Save form state to DeviceStorage (survives WebView kill) */
export function saveFormState(
  tg: TelegramWebApp | undefined,
  key: string,
  data: Record<string, unknown>,
) {
  if (!tg?.isVersionAtLeast?.('9.0') || !tg?.DeviceStorage) return;
  try {
    if (!key) return;
    const value = JSON.stringify(data);
    if (value.length > MAX_STORAGE_VALUE_CHARS) return;
    tg.DeviceStorage.setItem(key, value);
  } catch { /* quota exceeded — best effort */ }
}

/** Restore form state from DeviceStorage */
export function restoreFormState(
  tg: TelegramWebApp | undefined,
  key: string,
): Promise<Record<string, unknown> | null> {
  return new Promise((resolve) => {
    if (!tg?.isVersionAtLeast?.('9.0') || !tg?.DeviceStorage) {
      resolve(null);
      return;
    }
    if (!key) {
      resolve(null);
      return;
    }
    if (!safeInvoke(() => tg.DeviceStorage.getItem(key, (err, value) => {
      if (err) { resolve(null); return; }
      resolve(parseObjectJson(value));
    }))) {
      resolve(null);
    }
  });
}

/** Clear saved form state */
export function clearFormState(tg: TelegramWebApp | undefined, key: string) {
  if (!key) return;
  if (!tg?.isVersionAtLeast?.('9.0') || !tg?.DeviceStorage) return;
  safeCall(() => tg.DeviceStorage.removeItem(key));
}

// ── CloudStorage helpers (Bot API 6.9+) ───────────────────────────────────────

const PREFS_KEY = 'aegis_user_prefs';

export interface UserPrefs {
  pinned_tabs?: string[];
  last_console_tab?: string;
  dismissed_prompts?: string[];
}

/** Save user preferences to CloudStorage (read-merge-write to avoid clobbering other fields) */
export function saveUserPrefs(tg: TelegramWebApp | undefined, prefs: Partial<UserPrefs>) {
  if (!tg?.isVersionAtLeast?.('6.9') || !tg?.CloudStorage) return;
  safeInvoke(() => tg.CloudStorage.getItem(PREFS_KEY, (err, existing) => {
    const merged = !err ? parseObjectJson(existing) ?? {} : {};
    Object.assign(merged, prefs);
    const value = JSON.stringify(merged);
    if (value.length <= MAX_STORAGE_VALUE_CHARS) {
      safeCall(() => tg.CloudStorage.setItem(PREFS_KEY, value));
    }
  }));
}

/** Load user preferences from CloudStorage */
export function loadUserPrefs(tg: TelegramWebApp | undefined): Promise<UserPrefs | null> {
  return new Promise((resolve) => {
    if (!tg?.isVersionAtLeast?.('6.9') || !tg?.CloudStorage) {
      resolve(null);
      return;
    }
    if (!safeInvoke(() => tg.CloudStorage.getItem(PREFS_KEY, (err, val) => {
      if (err) { resolve(null); return; }
      resolve(parseObjectJson(val) as UserPrefs | null);
    }))) {
      resolve(null);
    }
  });
}

// ── Biometric Auth (Bot API 7.2+) ─────────────────────────────────────────────

/** Require biometric authentication for sensitive operations.
 *  Returns true if authenticated or if biometrics is unavailable (graceful degradation). */
export function authenticateForSensitive(
  tg: TelegramWebApp | undefined,
  reason: string,
): Promise<boolean> {
  return new Promise((resolve) => {
    if (!tg?.isVersionAtLeast?.('7.2') || !tg?.BiometricManager) {
      resolve(true); // Not supported → allow
      return;
    }
    const started = safeInvoke(() => tg.BiometricManager.init(() => {
      if (!tg.BiometricManager.isBiometricAvailable) {
        resolve(true); // No hardware → allow
        return;
      }
      safeCall(() => tg.BiometricManager.authenticate({ reason }, (ok) => resolve(ok))) ?? resolve(false);
    }));
    if (!started) resolve(false);
  });
}
