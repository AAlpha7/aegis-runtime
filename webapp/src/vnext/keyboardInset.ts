// The on-screen keyboard covers the bottom of the layout viewport. iOS keeps
// 100dvh at the large layout height, so a composer pinned to that edge sits
// under the keys. The overlap is measured from visualViewport; it is not a
// device height.

export const KEYBOARD_INSET_VAR = '--keyboard-inset';
export const KEYBOARD_VISIBLE_VAR = '--keyboard-visible';
export const KEYBOARD_OFFSET_VAR = '--keyboard-offset';
export const KEYBOARD_OPEN_CLASS = 'is-keyboard-open';
/** Layout viewport left short of the screen by the top safe area after the keyboard closes. */
export const LAYOUT_SHORTFALL_VAR = '--layout-shortfall';

// Sub-keyboard viewport jitter (rounding, a collapsing browser bar) is not a
// keyboard. A real on-screen keyboard is far taller than this.
export const KEYBOARD_OPEN_MIN_PX = 48;

export interface VisualViewportBox {
  height: number;
  offsetTop: number;
}

type KeyboardWindow = Window & {
  __aegisKeyboardViewport?: VisualViewportBox | null;
  __aegisScreenHeight?: number;
};

// A closed keyboard must not keep the top safe area as a bottom gap. A real
// keyboard is far taller than that inset, so a gap within a few pixels of the
// status-bar inset is the leftover layout viewport, not the keys.
export function layoutShortfallPx(
  screenHeight: number,
  innerHeight: number,
  topInsetPx: number,
  keyboardOpen: boolean,
): number {
  if (keyboardOpen) return 0;
  if (!Number.isFinite(screenHeight) || !Number.isFinite(innerHeight) || !Number.isFinite(topInsetPx)) return 0;
  if (topInsetPx <= 0) return 0;
  const gap = Math.round(screenHeight) - Math.round(innerHeight);
  if (gap <= 0 || gap > Math.round(topInsetPx) + 8) return 0;
  return gap;
}

export function keyboardOverlapPx(layoutHeight: number, viewport: VisualViewportBox | null): number {
  if (!viewport || !Number.isFinite(layoutHeight) || layoutHeight <= 0) return 0;
  if (!Number.isFinite(viewport.height) || !Number.isFinite(viewport.offsetTop)) return 0;
  const overlap = layoutHeight - viewport.offsetTop - viewport.height;
  if (!Number.isFinite(overlap) || overlap <= 0) return 0;
  return Math.round(overlap);
}

export function keyboardShouldLift(overlapPx: number, composerFocused: boolean, alreadyOpen: boolean): boolean {
  if (overlapPx < KEYBOARD_OPEN_MIN_PX) return false;
  return composerFocused || alreadyOpen;
}

export interface KeyboardFrame {
  open: boolean;
  /** Visual viewport height. The shell uses this directly. */
  visibleHeight: number;
  offsetTop: number;
  /** How much of the resting screen the keyboard covers. Not subtracted from an already-shrunk dvh. */
  covered: number;
  restingVisible: number;
}

export function nextKeyboardFrame(
  layoutHeight: number,
  viewport: VisualViewportBox | null,
  restingVisiblePx: number,
  composerFocused: boolean,
  alreadyOpen: boolean,
  topInsetPx = 0,
): KeyboardFrame {
  const visibleHeight = viewport && Number.isFinite(viewport.height)
    ? Math.max(0, Math.round(viewport.height))
    : Math.max(0, Math.round(layoutHeight));
  const offsetTop = viewport && Number.isFinite(viewport.offsetTop)
    ? Math.max(0, Math.round(viewport.offsetTop))
    : 0;
  const overlap = keyboardOverlapPx(layoutHeight, viewport);
  const shrink = restingVisiblePx > 0 ? Math.max(0, restingVisiblePx - visibleHeight) : 0;
  const covered = Math.max(overlap, shrink);
  // Blurred composer + a shrink the size of the status inset is the keyboard
  // already gone. Keeping the shell "open" left that inset on the bottom.
  const staleSafe = topInsetPx > 0
    && !composerFocused
    && covered > 0
    && covered <= topInsetPx + 8;
  const open = keyboardShouldLift(staleSafe ? 0 : covered, composerFocused, alreadyOpen);
  return {
    open,
    visibleHeight,
    offsetTop,
    covered,
    restingVisible: open ? restingVisiblePx : visibleHeight,
  };
}

export function readLayoutKeyboardViewport(win: Window): VisualViewportBox | null {
  const injected = (win as KeyboardWindow).__aegisKeyboardViewport;
  if (injected && typeof injected.height === 'number' && typeof injected.offsetTop === 'number') {
    return { height: injected.height, offsetTop: injected.offsetTop };
  }
  const viewport = win.visualViewport;
  if (!viewport) return null;
  return { height: viewport.height, offsetTop: viewport.offsetTop };
}

export function composerOwnsKeyboard(doc: Document): boolean {
  const active = doc.activeElement;
  if (!active || active === doc.body || active === doc.documentElement) return false;
  return Boolean((active as Element).closest?.('.vnext-chat-composer'));
}

let restingVisiblePx = 0;

export function readScreenHeight(win: Window): number {
  const injected = (win as KeyboardWindow).__aegisScreenHeight;
  if (typeof injected === 'number' && Number.isFinite(injected)) return injected;
  const screenHeight = win.screen?.height;
  return typeof screenHeight === 'number' && Number.isFinite(screenHeight) ? screenHeight : 0;
}

export function readTopInsetPx(doc: Document): number {
  const probe = doc.createElement('div');
  probe.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding-top:env(safe-area-inset-top,0px);';
  doc.body.appendChild(probe);
  const px = parseFloat(getComputedStyle(probe).paddingTop) || 0;
  probe.remove();
  return px;
}

export function clearKeyboardInset(doc: Document): void {
  restingVisiblePx = 0;
  const root = doc.documentElement;
  root.classList.remove(KEYBOARD_OPEN_CLASS);
  root.style.removeProperty(KEYBOARD_INSET_VAR);
  root.style.removeProperty(KEYBOARD_VISIBLE_VAR);
  root.style.removeProperty(KEYBOARD_OFFSET_VAR);
  root.style.removeProperty(LAYOUT_SHORTFALL_VAR);
}

export function syncKeyboardInset(win: Window): number {
  const doc = win.document;
  const alreadyOpen = doc.documentElement.classList.contains(KEYBOARD_OPEN_CLASS);
  const topInset = readTopInsetPx(doc);
  const frame = nextKeyboardFrame(
    win.innerHeight,
    readLayoutKeyboardViewport(win),
    restingVisiblePx,
    composerOwnsKeyboard(doc),
    alreadyOpen,
    topInset,
  );
  restingVisiblePx = frame.restingVisible;
  const root = doc.documentElement;
  root.classList.toggle(KEYBOARD_OPEN_CLASS, frame.open);
  const shortfall = layoutShortfallPx(readScreenHeight(win), win.innerHeight, topInset, frame.open);
  if (shortfall > 0) root.style.setProperty(LAYOUT_SHORTFALL_VAR, `${shortfall}px`);
  else root.style.removeProperty(LAYOUT_SHORTFALL_VAR);
  if (!frame.open) {
    root.style.removeProperty(KEYBOARD_INSET_VAR);
    root.style.removeProperty(KEYBOARD_VISIBLE_VAR);
    root.style.removeProperty(KEYBOARD_OFFSET_VAR);
    return 0;
  }
  // Size the shell to the visual viewport. Do not also subtract `covered`
  // from 100dvh: a closed keyboard used to leave that inset on the layout
  // viewport (the blank band under the tab bar).
  root.style.setProperty(KEYBOARD_VISIBLE_VAR, `${frame.visibleHeight}px`);
  root.style.setProperty(KEYBOARD_OFFSET_VAR, `${frame.offsetTop}px`);
  root.style.setProperty(KEYBOARD_INSET_VAR, `${frame.covered}px`);
  if (win.scrollY > 0) win.scrollTo(0, 0);
  return frame.covered;
}
