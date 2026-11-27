/**
 * Test-only bundle entry: re-exports InlineAssistantPanel alongside LocaleProvider
 * + t/tf from a SINGLE esbuild bundle so the render test can wrap the component in
 * a provider that shares the *same* React context instance. A separately-bundled
 * LocaleProvider would create a distinct context object the panel never reads, so
 * the locale would never propagate. Mirrors workViewTestEntry.ts. Used by
 * `test:vnext-inline-assistant-panel`.
 */
export { InlineAssistantPanel } from './InlineAssistantPanel';
export { LocaleProvider } from '../i18n/LocaleContext';
export { t, tf } from '../i18n/catalog';
