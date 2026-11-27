/**
 * Test-only bundle entry: re-exports VNextSegmented from a single esbuild graph
 * so the node:test render suite (tests/vnextSegmented.test.mjs) can import it
 * with react/react-dom marked external. Not imported by the app.
 */
export { VNextSegmented } from './VNextSegmented';
