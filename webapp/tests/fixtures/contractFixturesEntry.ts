/**
 * Test-only bundle entry for the FE↔BE contract-fixture sensor
 * (tests/contractFixtures.test.mjs). Re-exports the REAL production
 * parsers/guards + api functions so the checked-in backend goldens in
 * tests/fixtures/contract/*.json are parsed by exactly the code the app
 * ships. Not imported by the app.
 */
export { readVNextStatusProjection } from '../../src/apiGuards';
