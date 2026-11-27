import { createContext, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeLocale, t } from './catalog';
import type { Locale } from './catalog';

/**
 * The webapp's current display locale plus a setter.
 *
 * The locale is initialized from the backend `display_language` value (exposed
 * on the settings projection, see `webapp/src/api.ts`) via `normalizeLocale`,
 * and can be changed at runtime by the language toggle (immediate UI effect;
 * persistence to the backend setting is wired separately in a later task so
 * server-authored Channel-B strings follow too).
 *
 * Sibling of `UserTimezoneContext`: a single thread by which the whole webapp
 * picks up the owner's configured preference.
 */
interface LocaleContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
}

const LocaleContext = createContext<LocaleContextValue>({
  locale: 'zh',
  setLocale: () => {},
});

/** BCP 47 tag stamped on `<html lang>` for a display locale. */
export function documentLanguageTag(locale: Locale): string {
  return locale === 'en' ? 'en' : 'zh-CN';
}

/**
 * Stamps `document.documentElement.lang` so text produced outside a React
 * render (`preferredLocale()` in persisted chat history, voice command
 * dispatch) follows the console locale instead of the shipped `index.html`
 * default. Safe outside a browser (static render, node tests): no-op.
 */
export function stampDocumentLanguage(locale: Locale): void {
  try {
    if (typeof document === 'undefined' || !document.documentElement) return;
    const next = documentLanguageTag(locale);
    if (document.documentElement.lang !== next) document.documentElement.lang = next;
  } catch {
    // Best-effort: a missing/locked DOM must never break rendering.
  }
}

export function LocaleProvider({
  displayLanguage,
  children,
}: {
  /** The backend `display_language` value; `undefined` defaults to `zh`. */
  displayLanguage: string | null | undefined;
  children: ReactNode;
}) {
  const [locale, setLocale] = useState<Locale>(() => normalizeLocale(displayLanguage ?? undefined));
  // Follow a later-arriving / refreshed backend setting. The optimistic
  // language toggle (SettingsView) is unaffected: this only runs when the
  // persisted value itself changes, and on success it already matches.
  useEffect(() => {
    const next = normalizeLocale(displayLanguage ?? undefined);
    setLocale((current) => (current === next ? current : next));
  }, [displayLanguage]);
  // Re-stamp `<html lang>` on every locale change (initial mount included).
  useEffect(() => {
    stampDocumentLanguage(locale);
  }, [locale]);
  const value = useMemo<LocaleContextValue>(() => ({ locale, setLocale }), [locale]);
  return <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>;
}

/** Returns the current locale and its setter. */
export function useLocale(): LocaleContextValue {
  return useContext(LocaleContext);
}

/**
 * Returns a translator bound to the current locale: `useT()(key)` resolves the
 * catalog entry for the active locale. Sibling of the backend `t(key, locale)`.
 */
export function useT(): (key: string) => string {
  const { locale } = useContext(LocaleContext);
  return useMemo(() => (key: string) => t(key, locale), [locale]);
}
