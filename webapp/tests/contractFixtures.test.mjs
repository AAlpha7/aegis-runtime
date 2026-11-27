/*
 * Sensor 2 (FE side) — FE↔BE contract-fixture test.
 *
 * The Rust side (core/src/admin_tests.rs `contract_fixture_*`) golden-compares
 * REAL backend response bodies into tests/fixtures/contract/*.json. This test
 * loads every fixture in that directory and runs each through the REAL
 * matching FE parser/guard (bundled from src/ via
 * tests/fixtures/contractFixturesEntry.ts), asserting:
 *
 *  1. the parse succeeds (the FE accepts what the BE actually sends), and
 *  2. NO SILENT FIELD DROP: every top-level key of the fixture is claimed by
 *     EXACTLY one of two explicit per-shape sets:
 *       - handledKeys: keys the FE parser explicitly validates/copies (each
 *         shape's parse() also asserts their values), or
 *       - knownDropped: keys the FE consciously does not carry — documented,
 *         reviewed gaps (the embedder_degraded outage class made visible).
 *     The sets are EXPLICIT rather than derived from `key in parsed` because
 *     a spread-preserving guard (readVNextStatusProjection returns
 *     `{...value, ...}`) keeps unknown keys present at runtime while they
 *     stay untyped/unvalidated/unconsumed — `in parsed` would report such a
 *     field as "handled" and the sensor would pass vacuously (Codex review
 *     2026-07-08).
 *
 * Completeness guard: a fixture file with no SHAPES entry FAILS this test, so
 * a new backend golden cannot land without deciding its FE handling; a new
 * top-level fixture key fails until it is claimed by one of the two sets.
 */
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { describe, it } from 'node:test';

const modulePath = process.env.CONTRACT_FIXTURES_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-contract-fixtures-test/contractFixtures.mjs', import.meta.url).href;
const {
  readVNextStatusProjection,
} = await import(modulePath);

const fixturesDir = new URL('./fixtures/contract/', import.meta.url);

function loadFixture(name) {
  return JSON.parse(readFileSync(new URL(name, fixturesDir), 'utf8'));
}

/**
 * Per-fixture contract: how the FE really consumes the shape.
 *
 * parse(fixture) → runs the REAL parser/api function and asserts the handled
 *                  fields' values (or the real throw behavior).
 * handledKeys    → top-level fixture keys the FE explicitly validates/copies.
 * knownDropped   → top-level fixture keys the FE consciously does not carry.
 *                  Every entry is a documented contract gap; remove it when
 *                  the FE wires the field.
 */
const SHAPES = {
  'vnext_status_projection.json': {
    parse: (fixture) => {
      const parsed = readVNextStatusProjection(fixture);
      assert.ok(parsed, 'readVNextStatusProjection rejected the REAL backend status body');
      // The populated degraded banner block must survive validation — a
      // malformed-block fallback would silently report "healthy".
      assert.deepEqual(parsed.llm_degraded, fixture.llm_degraded);
      // S2 (M1): embedder_degraded is now sanitized + consumed by the
      // app-shell EmbedderDegradedBanner; the populated block must survive
      // validation field-for-field (a malformed-block fallback would silently
      // report "dedup healthy").
      assert.deepEqual(parsed.embedder_degraded, fixture.embedder_degraded);
      assert.equal(parsed.schema_version, fixture.schema_version);
      assert.equal(parsed.generated_at, fixture.generated_at);
      assert.deepEqual(parsed.sections, fixture.sections);
      assert.deepEqual(parsed.system_events, fixture.system_events);
      assert.equal(parsed.advanced_diagnostics_available, fixture.advanced_diagnostics_available);
      return parsed;
    },
    handledKeys: [
      'schema_version',
      'generated_at',
      'sections',
      'system_events',
      'advanced_diagnostics_available',
      'llm_degraded',
      'embedder_degraded',
    ],
    knownDropped: [],
  },
};

describe('FE↔BE contract fixtures', () => {
  const files = readdirSync(fixturesDir).filter((f) => f.endsWith('.json')).sort();

  it('every checked-in fixture has an FE assertion entry (completeness guard)', () => {
    const unmapped = files.filter((f) => !(f in SHAPES));
    assert.deepEqual(
      unmapped,
      [],
      `New contract fixture(s) with no FE parse entry: ${unmapped.join(', ')}. ` +
        'Add a SHAPES entry in tests/contractFixtures.test.mjs deciding how the FE consumes each.',
    );
    const stale = Object.keys(SHAPES).filter((f) => !files.includes(f));
    assert.deepEqual(
      stale,
      [],
      `SHAPES entries with no fixture file: ${stale.join(', ')}. Remove them.`,
    );
    assert.ok(files.length >= 1, `expected the seeded fixture set, found ${files.length}`);
  });

  for (const [name, shape] of Object.entries(SHAPES)) {
    it(`${name}: real FE parser accepts it with no silent top-level field drop`, async () => {
      const fixture = loadFixture(name);
      const parsed = await shape.parse(fixture);
      assert.ok(parsed && typeof parsed === 'object', `${name}: parse produced no object`);

      // The two sets must not overlap — a key cannot be both handled and
      // consciously dropped.
      const overlap = shape.handledKeys.filter((key) => shape.knownDropped.includes(key));
      assert.deepEqual(overlap, [], `${name}: keys in BOTH handledKeys and knownDropped`);

      // Every top-level backend key must be explicitly claimed.
      const unclaimed = Object.keys(fixture).filter(
        (key) => !shape.handledKeys.includes(key) && !shape.knownDropped.includes(key),
      );
      assert.deepEqual(
        unclaimed,
        [],
        `${name}: backend sends top-level key(s) the FE neither validates nor ` +
          `consciously drops: ${unclaimed.join(', ')}. Wire them through the ` +
          "parser/guard (add to handledKeys + assert in parse()), or add them " +
          "to knownDropped with a comment explaining why.",
      );

      // Ratchet: both lists must stay in sync with the fixture (stale entries
      // after a backend field rename/removal must be pruned).
      const staleClaims = [...shape.handledKeys, ...shape.knownDropped].filter(
        (key) => !(key in fixture),
      );
      assert.deepEqual(
        staleClaims,
        [],
        `${name}: handledKeys/knownDropped entries not present in the fixture: ${staleClaims.join(', ')}`,
      );
    });
  }
});
