/**
 * Page scroll lock for the chat bottom sheet.
 *
 * On phone layouts (≤820px) the page behind does not scroll through the
 * document — `.vnext-shell` is the real scroller (`overflow-y: auto`). Lock
 * that element with `overflow: hidden` (blocks iOS touch scrolling, unlike
 * `overflow: hidden` on body) and restore its exact `scrollTop` on release.
 * Also apply a document fallback for layouts where `window` still scrolls.
 */

export type ChatSheetScrollLockScroller = {
  style: { overflow: string };
  scrollTop: number;
};

export type ChatSheetScrollLockDocument = {
  documentElement: { style: { overflow: string } };
  body: {
    style: {
      overflow: string;
      position: string;
      top: string;
      width: string;
    };
  } | null;
};

export type ChatSheetScrollLockWindow = {
  scrollY: number;
  scrollTo(x: number, y: number): void;
};

/** Minimal ancestor walk shape for finding the page scroller (unit-testable). */
export type ChatSheetScrollLockAncestor = {
  classList: { contains(token: string): boolean };
  parentElement: ChatSheetScrollLockAncestor | null;
};

type LockSnapshot = {
  scroller: ChatSheetScrollLockScroller | null;
  scrollerOverflow: string;
  scrollerScrollTop: number;
  scrollY: number;
  htmlOverflow: string;
  bodyOverflow: string;
  bodyPosition: string;
  bodyTop: string;
  bodyWidth: string;
};

let activeLock: LockSnapshot | null = null;

function readScrollY(win: ChatSheetScrollLockWindow): number {
  const y = win.scrollY;
  return typeof y === 'number' && Number.isFinite(y) ? y : 0;
}

function readScrollTop(scroller: ChatSheetScrollLockScroller): number {
  const top = scroller.scrollTop;
  return typeof top === 'number' && Number.isFinite(top) ? top : 0;
}

function defaultDoc(): ChatSheetScrollLockDocument | null {
  return typeof document !== 'undefined' ? document : null;
}

function defaultWin(): ChatSheetScrollLockWindow | null {
  return typeof window !== 'undefined' ? window : null;
}

/**
 * Resolve the page scroller behind the sheet: first scrollable ancestor
 * (`overflow-y: auto|scroll|overlay`) from `start`, else `.vnext-shell`.
 */
export function findChatSheetPageScroller<T extends ChatSheetScrollLockAncestor>(
  start: T | null,
  getOverflowY: (el: T) => string,
  querySelector: (selector: string) => T | null,
): T | null {
  let el: T | null = start;
  while (el) {
    const overflowY = getOverflowY(el);
    if (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'overlay') {
      return el;
    }
    if (el.classList.contains('vnext-shell')) {
      return el;
    }
    el = el.parentElement as T | null;
  }
  return querySelector('.vnext-shell');
}

/**
 * Apply the page scroll lock. Idempotent while already locked.
 * When `scroller` is provided it is locked with `overflow: hidden` and its
 * `scrollTop` is captured; the document fallback always runs as well.
 */
export function applyChatSheetScrollLock(
  scroller: ChatSheetScrollLockScroller | null = null,
  doc: ChatSheetScrollLockDocument | null = defaultDoc(),
  win: ChatSheetScrollLockWindow | null = defaultWin(),
): void {
  if (activeLock || !doc?.body || !win) return;
  const html = doc.documentElement;
  const body = doc.body;
  const scrollerScrollTop = scroller ? readScrollTop(scroller) : 0;
  activeLock = {
    scroller,
    scrollerOverflow: scroller ? scroller.style.overflow : '',
    scrollerScrollTop,
    scrollY: readScrollY(win),
    htmlOverflow: html.style.overflow,
    bodyOverflow: body.style.overflow,
    bodyPosition: body.style.position,
    bodyTop: body.style.top,
    bodyWidth: body.style.width,
  };
  if (scroller) {
    scroller.style.overflow = 'hidden';
    scroller.scrollTop = scrollerScrollTop;
  }
  html.style.overflow = 'hidden';
  body.style.overflow = 'hidden';
  body.style.position = 'fixed';
  body.style.top = `-${activeLock.scrollY}px`;
  body.style.width = '100%';
}

/** Release the page scroll lock and restore captured positions. Idempotent. */
export function releaseChatSheetScrollLock(
  doc: ChatSheetScrollLockDocument | null = defaultDoc(),
  win: ChatSheetScrollLockWindow | null = defaultWin(),
): void {
  if (!activeLock || !doc?.body || !win) return;
  const snapshot = activeLock;
  activeLock = null;
  if (snapshot.scroller) {
    snapshot.scroller.style.overflow = snapshot.scrollerOverflow;
  }
  const html = doc.documentElement;
  const body = doc.body;
  html.style.overflow = snapshot.htmlOverflow;
  body.style.overflow = snapshot.bodyOverflow;
  body.style.position = snapshot.bodyPosition;
  body.style.top = snapshot.bodyTop;
  body.style.width = snapshot.bodyWidth;
  win.scrollTo(0, snapshot.scrollY);
  // Restore scroller scrollTop after unfixed body — unfixing can reset
  // descendant scroll offsets in some engines.
  if (snapshot.scroller) {
    snapshot.scroller.scrollTop = snapshot.scrollerScrollTop;
  }
}

/** Whether a lock snapshot is currently held (for tests). */
export function isChatSheetScrollLockActive(): boolean {
  return activeLock !== null;
}
