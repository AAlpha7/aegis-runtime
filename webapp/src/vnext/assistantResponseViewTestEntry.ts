/**
 * Test-only bundle entry: re-exports AssistantResponseView and its owner-facing
 * helpers alongside LocaleProvider + t/tf from a SINGLE esbuild bundle so the
 * render test can wrap the component in a provider that shares the *same* React
 * context instance. A separately-bundled LocaleProvider would create a distinct
 * context object the view never reads, so the locale would never propagate.
 * Mirrors workViewTestEntry.ts. Used by `test:vnext-assistant-response-view`.
 */
export {
  AssistantResponseView,
  assistantResponseSections,
  sourceEvidenceItems,
  SourceEvidenceContent,
  evidenceRowId,
} from './AssistantResponseView';
export { parseJevProvenance } from './jevProvenance';
export { LocaleProvider } from '../i18n/LocaleContext';
export { UserTimezoneProvider } from './UserTimezoneContext';
export { t, tf } from '../i18n/catalog';
