/**
 * Test-only bundle entry: re-exports WorkView alongside LocaleProvider from a
 * single esbuild bundle so the render test can wrap the component in a provider
 * that shares the *same* React context instance (a separately-bundled provider
 * would create a distinct context object that WorkView never reads). Used by
 * `test:vnext-work-render`.
 */
export { WorkView, filterWorkForScan, resolveDecisionChoiceCopy, localizeDecisionRequest } from './WorkView';
export { presentOwnerWorkSurfaceActions, workDetailAskPlacement, workScanFiltersDefaultExpanded, workScanChromeDefaultExpanded } from './model';
export { LocaleProvider } from '../i18n/LocaleContext';
export { t, tf } from '../i18n/catalog';
export { lineageChatCite, messagePublicRef } from './messageRef';
