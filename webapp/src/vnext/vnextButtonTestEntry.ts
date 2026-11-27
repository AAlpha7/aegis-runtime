/**
 * Test-only bundle entry: re-exports VNextButton from a single esbuild graph so
 * the node:test render suite (tests/vnextButton.test.mjs) can import it with
 * react/react-dom marked external. Not imported by the app.
 */
export { VNextButton } from './VNextButton';
