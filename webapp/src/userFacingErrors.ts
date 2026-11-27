import { t, tf } from './i18n/catalog';
import type { Locale } from './i18n/catalog';

type ApiErrorLike = {
  name?: string;
  message?: string;
  statusCode: number;
  problem?: {
    status?: string;
    error?: string;
    error_kind?: string;
  };
};

/**
 * Resolves a system-authored (Channel B) error into owner-facing recovery copy.
 *
 * Channel B strings always render in the configured `display_language`, so the
 * messages route through the i18n catalog rather than hardcoded English. The
 * `locale` defaults to `'en'` for back-compatibility with the few legacy/test
 * call sites that do not yet thread it; production call sites pass the active
 * `display_language`-derived locale.
 */
export function inlineUserFacingError(error: unknown, fallback: string, locale: Locale = 'en'): string {
  const apiError = apiErrorLike(error);
  if (apiError) {
    // Runtime-authored disclosures follow the sanitized model failure text.
    // Preserve them when localizing a 402/5xx recovery message; the ordinary
    // detail limiter must not erase the fallback that preceded the failure.
    const noticeStart = (apiError.message ?? '').indexOf('\n\n⚠️ ');
    if (noticeStart >= 0 && (apiError.statusCode === 402 || apiError.statusCode >= 500)) {
      const message = apiError.message ?? '';
      const problem = apiError.problem && {
        ...apiError.problem,
        error: apiError.problem.error?.split('\n\n⚠️ ')[0],
      };
      const base = inlineUserFacingError({ ...apiError, problem, message: message.slice(0, noticeStart) }, fallback, locale);
      return `${base}${message.slice(noticeStart)}`;
    }
    if (apiError.problem && sensitiveAccessProblem(apiError.problem)) {
      return sensitiveAccessMessage(apiError.problem, apiError.message || fallback, locale);
    }
    // Status 0 = the browser never got a response (offline, DNS, blocked):
    // the request did not reach Aegis, so this is not a server failure.
    if (apiError.statusCode === 0) {
      return tf('error.networkUnreachable', locale, { fallback });
    }
    if (apiError.statusCode === 401) return tf('error.signInAgain', locale, { fallback });
    if (apiError.statusCode === 402) {
      return tf('error.creditsExhausted', locale, { fallback });
    }
    if (apiError.statusCode === 403) {
      if (/csrf/i.test(apiError.message ?? '')) {
        return tf('error.csrfExpired', locale, { fallback });
      }
      if (/mfa_required|recent mfa/i.test(apiError.message ?? '')) {
        return t('error.mfaRequiredShort', locale);
      }
      return tf('error.forbiddenGeneric', locale, { fallback });
    }
    if (apiError.statusCode >= 400
      && apiError.statusCode < 500
      && /action does not apply to this item/i.test(apiError.message ?? '')) {
      return t('error.staleItem', locale);
    }
    if (apiError.statusCode >= 400 && /grok|xai|web push|webapp notification|push notification|service worker|pushmanager|push manager|avatar generation|avatar image|image generation|subscription|credential|token refresh|reconnect/i.test(apiError.message ?? '')) {
      return tf('error.withDetail', locale, { fallback, detail: apiError.message ?? '' });
    }
    if (apiError.statusCode === 408 || apiError.statusCode === 504) {
      return tf('error.timeout', locale, { fallback });
    }
    if (apiError.statusCode === 429) {
      return tf('error.busy', locale, { fallback });
    }
    // Structured backend error contract ({status:"error", error, error_kind,
    // retryable, hint} — e.g. a tool's page1_store_unavailable 503):
    // surface the owner-facing backend `error` string instead of collapsing it
    // into the generic status-bucket copy. Sensitive (mfa/consent) problems
    // were already handled above; specific status buckets (401/403/408/429)
    // keep their localized copy.
    if (apiError.problem && typeof apiError.problem.error === 'string' && apiError.problem.error) {
      return tf('error.withDetail', locale, { fallback, detail: apiError.problem.error });
    }
    if (apiError.statusCode >= 500 && /grok model|model is not available|assistant model is unavailable/i.test(apiError.message ?? '')) {
      // The bare "Assistant model is unavailable" line stays a localized recovery
      // sentence (owner screenshot 2026-09-08). A reason after that phrase, or a
      // mapped provider sentence, is what the owner needs in order to judge the miss.
      const detail = specificModelFailureDetail(apiError.message ?? '');
      if (detail) return tf('error.withDetail', locale, { fallback, detail });
      return tf('error.modelUnavailable', locale, { fallback });
    }
    if (apiError.statusCode >= 500 && /grok|xai|web push|webapp notification|push notification|service worker|pushmanager|push manager|avatar generation|image generation|subscription|credential|token refresh|reconnect/i.test(apiError.message ?? '')) {
      return tf('error.withDetail', locale, { fallback, detail: apiError.message ?? '' });
    }
    if (apiError.statusCode >= 500) {
      return tf('error.incompleteAnswer', locale, { fallback });
    }
    return tf('error.tryAgain', locale, { fallback });
  }
  // Browser-side Web Push failures already carry localized owner copy (see
  // vnext/webPush.ts, name 'WebPushError'); the English regex stays for any
  // legacy caller that still throws a plain Error.
  if (error instanceof Error && (error.name === 'WebPushError' || /webapp notification|web push|push notification|notification|service worker|pushmanager|push manager/i.test(error.message))) {
    return tf('error.withDetail', locale, { fallback, detail: error.message });
  }
  if (isNetworkUnreachable(error)) {
    return tf('error.networkUnreachable', locale, { fallback });
  }
  return fallback;
}

/**
 * A thrown fetch (never got a response): Chrome "Failed to fetch", Firefox
 * "NetworkError when attempting to fetch resource.", WebKit "Load failed".
 * Distinct from a server error, which arrives as an ApiResponseError with a
 * real status code.
 */
function isNetworkUnreachable(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  if (error instanceof TypeError && /fetch|network|load failed/i.test(error.message)) return true;
  return /failed to fetch|networkerror|network request failed|load failed|network/i.test(error.message);
}

function specificModelFailureDetail(message: string): string | null {
  const stripped = message.replace(/^.*?\(\d{3}\):\s*/, '').trim();
  const prefixed = stripped.match(/^assistant model is unavailable\s*[:：]\s*(.+)$/i);
  if (prefixed) {
    const detail = prefixed[1].trim();
    return detail ? detail.slice(0, 240) : null;
  }
  if (/^assistant model is unavailable$/i.test(stripped)) return null;
  if (/grok model|model is not available/i.test(stripped)) return stripped.slice(0, 240);
  return null;
}

function apiErrorLike(error: unknown): ApiErrorLike | null {
  if (!error || typeof error !== 'object') return null;
  const candidate = error as { statusCode?: unknown; name?: unknown; message?: unknown; problem?: unknown };
  if (typeof candidate.statusCode !== 'number' || !Number.isFinite(candidate.statusCode)) return null;
  const problem = candidate.problem && typeof candidate.problem === 'object'
    ? candidate.problem as ApiErrorLike['problem']
    : undefined;
  return {
    statusCode: candidate.statusCode,
    name: typeof candidate.name === 'string' ? candidate.name : undefined,
    message: typeof candidate.message === 'string' ? candidate.message : undefined,
    problem,
  };
}

function sensitiveAccessProblem(problem: NonNullable<ApiErrorLike['problem']>): boolean {
  return problem.status === 'mfa_required'
    || problem.status === 'consent_required'
    || problem.error === 'mfa_required'
    || problem.error === 'consent_required'
    || problem.error_kind === 'mfa_required'
    || problem.error_kind === 'consent_required';
}

function sensitiveAccessMessage(
  problem: NonNullable<ApiErrorLike['problem']>,
  fallback: string,
  locale: Locale,
): string {
  if (problem.status === 'mfa_required'
    || problem.error === 'mfa_required'
    || problem.error_kind === 'mfa_required') {
    return t('error.mfaRequiredBeforeContinue', locale);
  }
  if (problem.status === 'consent_required'
    || problem.error === 'consent_required'
    || problem.error_kind === 'consent_required') {
    return t('error.consentRequired', locale);
  }
  return fallback;
}

/** Matches `LOCAL_LOGIN_EMAIL` on the console session. */
export const LOCAL_HOST_SESSION_EMAIL = 'owner@local.invalid';

export function offerLocalHostStepUp(
  email: string | null | undefined,
  problem: { error?: string; status?: string } | null | undefined,
): boolean {
  if (email !== LOCAL_HOST_SESSION_EMAIL) return false;
  return problem?.error === 'mfa_required' || problem?.status === 'mfa_required';
}
