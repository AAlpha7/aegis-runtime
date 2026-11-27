import assert from 'node:assert/strict';
import { test } from 'node:test';

const modulePath = process.env.VNEXT_AUTO_UPDATE_MODULE || '../src/vnext/autoUpdate.ts';
const { extractBundleId, shouldReloadForUpdate } = await import(modulePath);

test('extractBundleId pulls the main bundle hash from a script src or index html', () => {
  assert.equal(extractBundleId('https://aegis.example.org/ui/assets/index-acc91585.js'), 'acc91585');
  assert.equal(
    extractBundleId('<script type="module" src="/ui/assets/index-deadbeef.js"></script>'),
    'deadbeef',
  );
  // The CSS bundle must NOT be mistaken for the JS bundle id.
  assert.equal(extractBundleId('/ui/assets/index-aa16b2fa.css'), null);
  assert.equal(extractBundleId(''), null);
  assert.equal(extractBundleId(null), null);
  assert.equal(extractBundleId(undefined), null);
});

test('shouldReloadForUpdate reloads ONLY on a valid, differing hash pair', () => {
  // Differ → reload.
  assert.equal(shouldReloadForUpdate('aaaa1111', 'bbbb2222'), true);
  // Same → no reload (the common steady state).
  assert.equal(shouldReloadForUpdate('aaaa1111', 'aaaa1111'), false);
  // Any uncertainty → no reload (never spurious, never loops).
  assert.equal(shouldReloadForUpdate(null, 'bbbb2222'), false);
  assert.equal(shouldReloadForUpdate('aaaa1111', null), false);
  assert.equal(shouldReloadForUpdate(null, null), false);
  assert.equal(shouldReloadForUpdate('', ''), false);
  // Malformed hashes (wrong length / non-hex) → no reload.
  assert.equal(shouldReloadForUpdate('short', 'bbbb2222'), false);
  assert.equal(shouldReloadForUpdate('aaaa1111', 'ZZZZZZZZ'), false);
  assert.equal(shouldReloadForUpdate('aaaa11119', 'bbbb2222'), false);
});
