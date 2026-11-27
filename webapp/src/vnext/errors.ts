import { inlineUserFacingError, offerLocalHostStepUp } from '../userFacingErrors';
export { offerLocalHostStepUp };
import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

export function displayError(error: unknown, fallback = 'Could not load Aegis', locale: Locale = 'en'): string {
  return inlineUserFacingError(error, fallback, locale);
}

export function assistantDisplayError(error: unknown, locale: Locale = 'en'): string {
  return inlineUserFacingError(error, t('error.couldNotFinish', locale), locale);
}

export function isLocalStaticPreviewSessionMiss(error: unknown): boolean {
  if (!error || typeof error !== 'object') return false;
  const candidate = error as { statusCode?: unknown };
  if (candidate.statusCode !== 404) return false;
  return window.location.hostname === '127.0.0.1' || window.location.hostname === 'localhost';
}
