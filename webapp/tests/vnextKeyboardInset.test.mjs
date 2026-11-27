import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_KEYBOARD_INSET_MODULE
  ?? 'file:///tmp/aegis-runtime-vnext-keyboard-inset-test/keyboardInset.mjs';
const { keyboardOverlapPx, keyboardShouldLift, nextKeyboardFrame, layoutShortfallPx, KEYBOARD_OPEN_MIN_PX } = await import(modulePath);

describe('keyboard overlap from the visual viewport', () => {
  it('measures the covered band and ignores a closed keyboard', () => {
    assert.equal(keyboardOverlapPx(844, { height: 524, offsetTop: 0 }), 320);
    assert.equal(keyboardOverlapPx(844, { height: 500, offsetTop: 24 }), 320);
    assert.equal(keyboardOverlapPx(844, { height: 844, offsetTop: 0 }), 0);
    assert.equal(keyboardOverlapPx(844, null), 0);
    assert.equal(keyboardOverlapPx(0, { height: 100, offsetTop: 0 }), 0);
  });

  it('lifts only for a real keyboard, and stays lifted until the inset returns to zero', () => {
    assert.equal(keyboardShouldLift(KEYBOARD_OPEN_MIN_PX - 1, true, false), false);
    assert.equal(keyboardShouldLift(320, false, false), false);
    assert.equal(keyboardShouldLift(320, true, false), true);
    assert.equal(keyboardShouldLift(320, false, true), true);
    assert.equal(keyboardShouldLift(0, true, true), false);
  });

  it('sizes the shell to the visual viewport when dvh has already shrunk', () => {
    const full = nextKeyboardFrame(874, { height: 538, offsetTop: 0 }, 874, true, false);
    assert.equal(full.open, true);
    assert.equal(full.visibleHeight, 538);
    assert.equal(full.covered, 336);
    assert.equal(full.restingVisible, 874);
    // innerHeight already matches the visual viewport (resizes-content).
    // The shell stays 538px; it must not also subtract the 336px cover.
    const shrunk = nextKeyboardFrame(538, { height: 538, offsetTop: 0 }, 874, true, true);
    assert.equal(shrunk.open, true);
    assert.equal(shrunk.visibleHeight, 538);
    assert.equal(shrunk.covered, 336);
    assert.equal(shrunk.restingVisible, 874);
    const closed = nextKeyboardFrame(874, { height: 874, offsetTop: 0 }, 874, false, false);
    assert.equal(closed.open, false);
    assert.equal(closed.visibleHeight, 874);
    assert.equal(closed.restingVisible, 874);
  });

  it('does not keep a top-safe-area gap as an open keyboard after blur', () => {
    // Screen 874, layout viewport stuck at 812 (the 62px status inset).
    const stale = nextKeyboardFrame(812, { height: 812, offsetTop: 0 }, 874, false, true, 62);
    assert.equal(stale.open, false);
    assert.equal(stale.covered, 62);
    const stillTyping = nextKeyboardFrame(812, { height: 812, offsetTop: 0 }, 874, true, true, 62);
    assert.equal(stillTyping.open, true);
  });

  it('extends the shell only for a closed-keyboard shortfall the size of the status inset', () => {
    assert.equal(layoutShortfallPx(874, 812, 62, false), 62);
    assert.equal(layoutShortfallPx(874, 812, 62, true), 0);
    assert.equal(layoutShortfallPx(874, 874, 62, false), 0);
    assert.equal(layoutShortfallPx(874, 500, 62, false), 0);
    assert.equal(layoutShortfallPx(844, 800, 0, false), 0);
  });
});
