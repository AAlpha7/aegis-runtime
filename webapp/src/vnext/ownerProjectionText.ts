import type { VNextStatusSummary } from '../api';
import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

// Owner-facing copy for the cost-saving (paused-automation) section. Localized;
// en values stay byte-identical to the prior exported constants.
export function costSavingOwnerMessage(locale: Locale = 'en'): string {
  return t('work.projection.costSaving.message', locale);
}

function costSavingOwnerNextStep(locale: Locale): string {
  return t('work.projection.costSaving.nextStep', locale);
}

const INTERNAL_OPERATION_TERMS = [
  'heart' + 'beat',
  'health' + ' probes',
  'memory' + ' consolidation',
  'initiative',
  'reflection',
  'narrative',
  'autonomous',
  'internal loop',
];

function containsInternalOperationTerms(message: string): boolean {
  const lower = message.toLowerCase();
  return INTERNAL_OPERATION_TERMS.some((term) => lower.includes(term));
}

export function ownerMessageForProjectionSection(section: VNextStatusSummary, locale: Locale = 'en'): string {
  const message = section.owner_message.trim();
  if (section.label === 'Cost controls' && (!message || containsInternalOperationTerms(message))) {
    return costSavingOwnerMessage(locale);
  }
  return ownerFacingProjectionText(message, section.label, 'message', locale);
}

export function ownerNextStepForProjectionSection(section: VNextStatusSummary, locale: Locale = 'en'): string | null | undefined {
  if (section.label === 'Cost controls' && section.owner_next_step) {
    return costSavingOwnerNextStep(locale);
  }
  return section.owner_next_step
    ? ownerFacingProjectionText(section.owner_next_step, section.label, 'next_step', locale)
    : section.owner_next_step;
}

// Server status tokens are matched against English keywords (these stay English
// — they identify the failure class). The owner-facing replacement text is
// localized. The non-matching tail returns the raw server `value` unchanged
// (server content is intentionally not translated here).
function ownerFacingProjectionText(message: string, _label: string, kind: 'message' | 'next_step', locale: Locale): string {
  const value = message.trim();
  // The card already shows the localized section label as its title, so the
  // empty branch uses a generic "this area" phrasing rather than the label.
  if (!value) return kind === 'next_step' ? t('work.projection.empty.nextStep', locale) : t('work.projection.empty.message', locale);
  const lower = value.toLowerCase();
  if (lower.includes('mfa_required') || lower.includes('mfa required')) {
    return t('work.projection.mfa', locale);
  }
  if (lower.includes('consent_required') || lower.includes('consent required')) {
    return kind === 'next_step'
      ? t('work.projection.consent.nextStep', locale)
      : t('work.projection.consent.message', locale);
  }
  if (lower.includes('calendar_reader') || lower.includes('calendar reader')) {
    return kind === 'next_step' ? t('work.projection.calendar.nextStep', locale) : t('work.projection.calendar.message', locale);
  }
  if (lower.includes('fuel') && lower.includes('exhaust')) {
    return kind === 'next_step' ? t('work.projection.credits.nextStep', locale) : t('work.projection.credits.message', locale);
  }
  if (
    lower.includes('model provider credits')
    || lower.includes('credits are exhausted')
    || (lower.includes('credits') && (lower.includes('recover') || lower.includes('restart')))
  ) {
    return kind === 'next_step' ? t('work.projection.credits.nextStep', locale) : t('work.projection.credits.message', locale);
  }
  if (lower.includes('rate_limit') || lower.includes('rate limit')) {
    return kind === 'next_step' ? t('work.projection.rateLimit.nextStep', locale) : t('work.projection.rateLimit.message', locale);
  }
  if (lower.includes('tool_gate') || lower.includes('data_access_gate') || lower.includes('protected action')) {
    return kind === 'next_step' ? t('work.projection.safetyBoundary.nextStep', locale) : t('work.projection.safetyBoundary.message', locale);
  }
  if (lower.includes('source_content_untrusted') || lower.includes('redacted_fields')) {
    return kind === 'next_step' ? t('work.projection.hiddenDetail.nextStep', locale) : t('work.projection.hiddenDetail.message', locale);
  }
  if (lower.includes('projector not connected')) {
    return t('work.projection.notConnected', locale);
  }
  if (lower.includes('metadata only')) {
    return t('work.projection.metadataOnly', locale);
  }
  if (containsInternalOperationTerms(value)) {
    return kind === 'next_step'
      ? t('work.projection.empty.nextStep', locale)
      : t('work.projection.quiet.message', locale);
  }
  return value;
}
