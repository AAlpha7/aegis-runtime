// Test-only bundle entry: re-exports the real error-formatting machinery and
// the i18n catalog accessor so the regression test can exercise the exact
// production code path (no reimplementation).
export { inlineUserFacingError } from '../../src/userFacingErrors';
export { t, tf } from '../../src/i18n/catalog';
