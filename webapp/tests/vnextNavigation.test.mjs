import assert from 'node:assert/strict';
import { test } from 'node:test';

const modulePath = process.env.VNEXT_NAVIGATION_MODULE || '../src/vnext/navigation.ts';
const {
  buildVNextLocationPath,
  parseVNextLocationState,
  writeVNextLocationState,
} = await import(modulePath);

/**
 * Minimal history-stack fake for writeVNextLocationState (the real function
 * takes a `Pick<Window, 'history' | 'location'>`, so no DOM mock is needed).
 * Models the entry stack + current index the way a browser does, including
 * pushState truncating the forward tail.
 */
function fakeWindow(initialHref) {
  const stack = [new URL(initialHref).href];
  let index = 0;
  const currentUrl = () => new URL(stack[index]);
  const windowRef = {
    get location() {
      const url = currentUrl();
      return { href: url.href, pathname: url.pathname, search: url.search, hash: url.hash };
    },
    history: {
      pushState(_state, _title, next) {
        stack.splice(index + 1);
        stack.push(new URL(next, currentUrl()).href);
        index = stack.length - 1;
      },
      replaceState(_state, _title, next) {
        stack[index] = new URL(next, currentUrl()).href;
      },
    },
  };
  return {
    windowRef,
    // Simulates the browser-back half of an iOS edge swipe (the app's
    // popstate handler then re-parses location).
    back() {
      if (index > 0) index -= 1;
      return currentUrl();
    },
    entries: () => stack.slice(),
    position: () => index,
  };
}

test('parseVNextLocationState defaults to Home and ignores unknown views', () => {
  assert.deepEqual(parseVNextLocationState(''), { view: 'home', itemId: null });
  assert.deepEqual(parseVNextLocationState('?view=unknown&item=email%3A42'), { view: 'home', itemId: null });
});

test('parseVNextLocationState keeps item focus only for Work links', () => {
  assert.deepEqual(parseVNextLocationState('?view=work&item=email%3A42'), { view: 'work', itemId: 'email:42' });
  assert.deepEqual(parseVNextLocationState('?view=mail&item=primary%3A18cabc'), { view: 'home', itemId: null });
  assert.deepEqual(parseVNextLocationState('?view=status&item=email%3A42'), { view: 'status', itemId: null });
  assert.deepEqual(parseVNextLocationState('?view=settings&item=email%3A42'), { view: 'settings', itemId: null });
  assert.deepEqual(parseVNextLocationState('?item=email%3A42'), { view: 'home', itemId: null });
});

test('writeVNextLocationState no-ops when the URL is unchanged (no duplicate push)', () => {
  const fake = fakeWindow('https://aegis.example.org/ui/?view=work&item=task%3A1');
  writeVNextLocationState(fake.windowRef, { view: 'work', itemId: 'task:1' }, 'push');
  assert.equal(fake.entries().length, 1, 'identical target URL must not push');
});

test('buildVNextLocationPath preserves unrelated query params while routing work item focus', () => {
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?foo=1#today', { view: 'work', itemId: 'email:42' }),
    '/ui/?foo=1&view=work&item=email%3A42#today',
  );
});

test('buildVNextLocationPath removes item outside Work and keeps Home URLs quiet', () => {
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?view=work&item=email%3A42&foo=1', { view: 'status', itemId: 'email:42' }),
    '/ui/?view=status&foo=1',
  );
  assert.deepEqual(parseVNextLocationState('?view=proof'), { view: 'home', itemId: null });
  assert.deepEqual(parseVNextLocationState('?view=instructions'), { view: 'home', itemId: null });
  // The retired Dev Harness route now falls back to Home like any unknown view.
  assert.deepEqual(parseVNextLocationState('?view=dev'), { view: 'home', itemId: null });
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?view=work&item=email%3A42&foo=1', { view: 'home', itemId: null }),
    '/ui/?foo=1',
  );
});

test('buildVNextLocationPath clears Settings section anchors outside Settings', () => {
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?view=settings#vnext-settings-security', { view: 'home', itemId: null }),
    '/ui/',
  );
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?view=settings#vnext-settings-alerts', { view: 'work', itemId: 'task:1' }),
    '/ui/?view=work&item=task%3A1',
  );
  assert.equal(
    buildVNextLocationPath('https://aegis.example.org/ui/?view=settings#vnext-settings-services', { view: 'settings', itemId: null }),
    '/ui/?view=settings#vnext-settings-services',
  );
});
