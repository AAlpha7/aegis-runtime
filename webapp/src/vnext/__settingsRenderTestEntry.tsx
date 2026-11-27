/**
 * Test-only entry that re-exports SettingsView together with the LocaleProvider
 * it consumes, from a SINGLE module graph. The status/settings render test
 * bundles this with esbuild so the provider it wraps SettingsView in shares the
 * exact same LocaleContext instance the component reads (a separately-bundled
 * LocaleContext would create a distinct context object whose Provider would not
 * feed SettingsView, leaving it on the zh default). Not imported by the app.
 */
export {
  SettingsView,
  applyDisplayLanguageChoice,
  SETTINGS_TIER_ORDER,
  settingsTierSections,
  settingsOwnerDisplayName,
} from './SettingsView';
export type { SettingsTierId } from './SettingsView';
export { LlmDegradedBanner } from './LlmDegradedBanner';
export { EmbedderDegradedBanner } from './EmbedderDegradedBanner';
export { LocaleProvider } from '../i18n/LocaleContext';
