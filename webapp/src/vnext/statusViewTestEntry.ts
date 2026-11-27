/**
 * Test-only bundle entry: re-exports StatusView alongside LocaleProvider from a
 * single esbuild bundle so the render test can wrap the component in a provider
 * that shares the *same* React context instance. A separately-bundled
 * LocaleProvider would create a distinct context object the view never reads, so
 * the locale would never propagate (StatusView would stay on the zh default).
 * Mirrors workViewTestEntry.ts. Used by
 * `test:vnext-status-settings-render`.
 */
export { StatusView } from './StatusView';
export { LocaleProvider } from '../i18n/LocaleContext';
export { t, tf } from '../i18n/catalog';
