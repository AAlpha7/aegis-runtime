import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_VIEW_TRANSITION_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-view-transition-test/viewTransition.js';
const { canViewTransition } = await import(modulePath);

const docWith = (hasApi) => (hasApi ? { startViewTransition() {} } : {});
const winReduced = (matches) => ({ matchMedia: () => ({ matches }) });

describe('canViewTransition (View Transitions gate)', () => {
  it('is true only when the API exists AND reduced motion is not requested', () => {
    assert.equal(canViewTransition(docWith(true), winReduced(false)), true);
  });

  it('is false when the API is missing (Safari/older browsers)', () => {
    assert.equal(canViewTransition(docWith(false), winReduced(false)), false);
  });

  it('is false when the owner prefers reduced motion, even with the API', () => {
    assert.equal(canViewTransition(docWith(true), winReduced(true)), false);
  });

  it('is SSR-safe: false when document/window are undefined', () => {
    assert.equal(canViewTransition(undefined, undefined), false);
  });
});
