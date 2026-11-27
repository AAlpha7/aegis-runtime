import assert from 'node:assert/strict';
import { test } from 'node:test';

const modulePath = process.env.VNEXT_CHAT_SHEET_SCROLL_LOCK_MODULE
  || '../src/vnext/chatSheetScrollLock.ts';
const {
  applyChatSheetScrollLock,
  releaseChatSheetScrollLock,
  isChatSheetScrollLockActive,
  findChatSheetPageScroller,
} = await import(modulePath);

function fakeDocWin(scrollY = 0) {
  const htmlStyle = { overflow: '' };
  const bodyStyle = { overflow: '', position: '', top: '', width: '' };
  let y = scrollY;
  const scrollCalls = [];
  const doc = {
    documentElement: { style: htmlStyle },
    body: { style: bodyStyle },
  };
  const win = {
    get scrollY() {
      return y;
    },
    scrollTo(x, nextY) {
      scrollCalls.push([x, nextY]);
      y = nextY;
    },
  };
  return { doc, win, htmlStyle, bodyStyle, scrollCalls, getY: () => y, setY: (next) => { y = next; } };
}

function fakeScroller(scrollTop = 0, overflow = '') {
  const style = { overflow };
  let top = scrollTop;
  return {
    style,
    get scrollTop() {
      return top;
    },
    set scrollTop(next) {
      top = next;
    },
  };
}

test('restores the captured scroll position on release', () => {
  const warmup = fakeDocWin(0);
  releaseChatSheetScrollLock(warmup.doc, warmup.win);

  const { doc, win, htmlStyle, bodyStyle, scrollCalls } = fakeDocWin(240);
  assert.equal(isChatSheetScrollLockActive(), false);

  applyChatSheetScrollLock(null, doc, win);
  assert.equal(isChatSheetScrollLockActive(), true);
  assert.equal(htmlStyle.overflow, 'hidden');
  assert.equal(bodyStyle.overflow, 'hidden');
  assert.equal(bodyStyle.position, 'fixed');
  assert.equal(bodyStyle.top, '-240px');
  assert.equal(bodyStyle.width, '100%');

  // Double apply is idempotent: styles stay on the first capture.
  applyChatSheetScrollLock(null, doc, win);
  assert.equal(bodyStyle.top, '-240px');
  assert.equal(isChatSheetScrollLockActive(), true);

  releaseChatSheetScrollLock(doc, win);
  assert.equal(isChatSheetScrollLockActive(), false);
  assert.equal(htmlStyle.overflow, '');
  assert.equal(bodyStyle.overflow, '');
  assert.equal(bodyStyle.position, '');
  assert.equal(bodyStyle.top, '');
  assert.equal(bodyStyle.width, '');
  assert.deepEqual(scrollCalls, [[0, 240]]);

  // Double release is idempotent: no second scrollTo.
  releaseChatSheetScrollLock(doc, win);
  assert.deepEqual(scrollCalls, [[0, 240]]);
});

test('locks the scroller element with overflow hidden and restores scrollTop', () => {
  const warmup = fakeDocWin(0);
  releaseChatSheetScrollLock(warmup.doc, warmup.win);

  const { doc, win, htmlStyle, bodyStyle } = fakeDocWin(0);
  const scroller = fakeScroller(200, 'auto');

  applyChatSheetScrollLock(scroller, doc, win);
  assert.equal(isChatSheetScrollLockActive(), true);
  assert.equal(scroller.style.overflow, 'hidden');
  assert.equal(scroller.scrollTop, 200);
  // Document fallback still applied.
  assert.equal(htmlStyle.overflow, 'hidden');
  assert.equal(bodyStyle.overflow, 'hidden');
  assert.equal(bodyStyle.position, 'fixed');

  // Simulate a gesture that would have moved the scroller if unlocked.
  scroller.scrollTop = 480;

  releaseChatSheetScrollLock(doc, win);
  assert.equal(isChatSheetScrollLockActive(), false);
  assert.equal(scroller.style.overflow, 'auto');
  assert.equal(scroller.scrollTop, 200);
  assert.equal(htmlStyle.overflow, '');
  assert.equal(bodyStyle.position, '');
});

test('findChatSheetPageScroller returns the overflow-y ancestor then .vnext-shell', () => {
  const shell = {
    classList: { contains: (t) => t === 'vnext-shell' },
    parentElement: null,
  };
  const main = {
    classList: { contains: () => false },
    parentElement: shell,
  };
  const host = {
    classList: { contains: () => false },
    parentElement: main,
  };

  const overflowY = new Map([
    [host, 'visible'],
    [main, 'visible'],
    [shell, 'auto'],
  ]);
  const found = findChatSheetPageScroller(
    host.parentElement,
    (el) => overflowY.get(el) || 'visible',
    () => null,
  );
  assert.equal(found, shell);

  const fallbackOnly = {
    classList: { contains: () => false },
    parentElement: null,
  };
  const viaSelector = findChatSheetPageScroller(
    fallbackOnly,
    () => 'visible',
    (sel) => (sel === '.vnext-shell' ? shell : null),
  );
  assert.equal(viaSelector, shell);
});
