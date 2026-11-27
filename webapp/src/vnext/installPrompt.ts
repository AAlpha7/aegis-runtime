import { t } from '../i18n/catalog';
import type { Locale } from '../i18n/catalog';

export type OwnerInstallStatus = 'available' | 'installed' | 'manual';

export type OwnerInstallPromptChoice = {
  outcome: 'accepted' | 'dismissed';
  platform?: string;
};

export type OwnerInstallPromptEvent = Event & {
  prompt: () => Promise<void>;
  userChoice: Promise<OwnerInstallPromptChoice>;
};

export function ownerInstallStatusFromSignals({
  promptAvailable,
  displayModeStandalone,
  navigatorStandalone,
}: {
  promptAvailable: boolean;
  displayModeStandalone: boolean;
  navigatorStandalone: boolean;
}): OwnerInstallStatus {
  if (displayModeStandalone || navigatorStandalone) return 'installed';
  if (promptAvailable) return 'available';
  return 'manual';
}

export function currentOwnerInstallStatus(promptAvailable: boolean): OwnerInstallStatus {
  const standaloneMedia = typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(display-mode: standalone)').matches;
  const navigatorStandalone = typeof navigator !== 'undefined'
    && Boolean((navigator as Navigator & { standalone?: boolean }).standalone);
  return ownerInstallStatusFromSignals({
    promptAvailable,
    displayModeStandalone: standaloneMedia,
    navigatorStandalone,
  });
}

export function ownerInstallActionLabel(status: OwnerInstallStatus, locale: Locale = 'en'): string {
  if (status === 'installed') return t('install.action.installed', locale);
  if (status === 'available') return t('install.action.available', locale);
  return t('install.action.manual', locale);
}

export function ownerInstallManualNotice(userAgent: string, locale: Locale = 'en'): string {
  if (/iPad|iPhone|iPod/i.test(userAgent)) {
    return t('install.manual.ios', locale);
  }
  return t('install.manual.generic', locale);
}

export function ownerInstallOutcomeNotice(choice: OwnerInstallPromptChoice, locale: Locale = 'en'): string {
  if (choice.outcome === 'accepted') {
    return t('install.outcome.accepted', locale);
  }
  return t('install.outcome.dismissed', locale);
}
