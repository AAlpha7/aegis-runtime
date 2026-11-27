import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_USE_REVEAL_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-use-reveal-test/useReveal.js', import.meta.url).href;
const { revealDelayMs, prefersReducedMotion } = await import(modulePath);

describe('revealDelayMs (stagger math)', () => {
  it('returns 0 for the first item and non-positive indices', () => {
    assert.equal(revealDelayMs(0), 0);
    assert.equal(revealDelayMs(-3), 0);
  });

  it('scales linearly by step before the clamp', () => {
    assert.equal(revealDelayMs(3, 60, 480), 180);
    assert.equal(revealDelayMs(1, 60, 480), 60);
  });

  it('clamps at the max so long lists do not lag', () => {
    assert.equal(revealDelayMs(100, 60, 480), 480);
  });

  it('rounds fractional indices and ignores non-finite input', () => {
    assert.equal(revealDelayMs(2.4, 50, 1000), 100);
    assert.equal(revealDelayMs(Number.NaN), 0);
    assert.equal(revealDelayMs(Number.POSITIVE_INFINITY, 60, 480), 480);
  });
});

describe('prefersReducedMotion (reduced-motion gate)', () => {
  it('is false when there is no window (SSR-safe)', () => {
    assert.equal(prefersReducedMotion(undefined), false);
  });

  it('reflects the matchMedia result', () => {
    const win = (matches) => ({ matchMedia: () => ({ matches }) });
    assert.equal(prefersReducedMotion(win(true)), true);
    assert.equal(prefersReducedMotion(win(false)), false);
  });

  it('fails safe to false when matchMedia throws', () => {
    const win = { matchMedia: () => { throw new Error('nope'); } };
    assert.equal(prefersReducedMotion(win), false);
  });
});
