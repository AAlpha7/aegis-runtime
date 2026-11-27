/**
 * Test-only bundle entry: re-exports VNextChip from a single esbuild graph so
 * the node:test render suite (tests/vnextChip.test.mjs) can import it with
 * react/react-dom marked external. Not imported by the app.
 */
export { VNextChip } from './VNextChip';
