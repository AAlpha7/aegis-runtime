import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_INTERACTION_STATE_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-interaction-state-test/interactionState.js';

const {
  assistantResultIsBusy,
  itemActionButtonLabel,
  itemActionConfirmationHint,
  shouldReleaseBusyOnRequestSettle,
  shouldForceReleaseBusyOnSessionReset,
} = await import(modulePath);

const archiveAction = {
  id: 'mail.archive',
  label: 'Archive',
  confirmLabel: 'Tap again to archive',
  receiptHint: 'Aegis will record an archive receipt.',
};

describe('vNext interaction state helpers', () => {
  it('labels item actions for idle, confirmation, and executing states', () => {
    assert.equal(itemActionButtonLabel(archiveAction, null, null), 'Archive');
    assert.equal(itemActionButtonLabel(archiveAction, 'mail.archive', null), 'Confirm?');
    assert.equal(itemActionButtonLabel(archiveAction, 'mail.archive', 'mail.archive'), 'Working...');
  });

  it('does not leak one action state onto another action', () => {
    assert.equal(itemActionButtonLabel(archiveAction, 'mail.mark_read', null), 'Archive');
    assert.equal(itemActionButtonLabel(archiveAction, null, 'mail.mark_read'), 'Archive');
  });

  it('makes first-tap confirmation hints explicit that no action ran yet', () => {
    assert.equal(
      itemActionConfirmationHint(archiveAction),
      'Not run yet. Tap "Confirm?" to confirm. Aegis will record an archive receipt.',
    );
  });

  it('treats only working assistant responses as busy', () => {
    assert.equal(assistantResultIsBusy('working'), true);
    assert.equal(assistantResultIsBusy('answer_now'), false);
    assert.equal(assistantResultIsBusy('error'), false);
    assert.equal(assistantResultIsBusy(null), false);
  });

  describe('busy-release decisions (the stuck-busy fix)', () => {
    it('per-request finally releases busy when the session is unchanged', () => {
      assert.equal(shouldReleaseBusyOnRequestSettle(3, 3, 5, 5), true);
    });

    it('per-request finally does NOT release busy when the epoch advanced mid-request', () => {
      // A reconnect/re-auth bumped the epoch while this request was in-flight.
      // The stale request must not clobber a newer request's busy=true here.
      assert.equal(shouldReleaseBusyOnRequestSettle(3, 4, 5, 5), false);
    });

    it('per-request finally does NOT release busy when the chat generation advanced mid-request', () => {
      // A conversation clear bumped the generation while this request was in-flight.
      assert.equal(shouldReleaseBusyOnRequestSettle(3, 3, 5, 6), false);
    });

    it('session-reset failsafe force-releases busy on any teardown (tick > 0)', () => {
      // This is the path that proves busy is released even when the per-request
      // finally above is correctly skipped: the moment the epoch/generation
      // advances, the reset tick increments and busy is force-cleared.
      assert.equal(shouldForceReleaseBusyOnSessionReset(1), true);
      assert.equal(shouldForceReleaseBusyOnSessionReset(2), true);
    });

    it('session-reset failsafe does NOT release busy at initial mount (tick === 0)', () => {
      // Nothing has been torn down yet on first mount; must not stomp a fresh
      // in-flight request's busy=true.
      assert.equal(shouldForceReleaseBusyOnSessionReset(0), false);
    });
  });
});
