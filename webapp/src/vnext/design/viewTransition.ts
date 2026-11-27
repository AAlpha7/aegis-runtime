/*
 * View Transitions helper (Direction B ④-JS interaction layer).
 *
 * Wraps a DOM-updating callback in the View Transitions API so navigating
 * between top-level views cross-fades instead of hard-cutting (the hard cut is
 * a big part of what reads "static"). It is strictly progressive enhancement:
 * when the API is missing (Safari/older browsers) or the owner prefers reduced
 * motion, the update runs directly with zero visual difference.
 *
 * `flushSync` is required so React applies the state update synchronously
 * inside the transition callback — otherwise the API snapshots the OLD DOM and
 * nothing animates.
 *
 * Spec: docs/2026-05-31-webapp-aurora-glass-redesign-design.md §④ interaction
 */
import { flushSync } from 'react-dom';
import { prefersReducedMotion } from './useReveal';

type DocLike = { startViewTransition?: (cb: () => void) => unknown } | undefined;
type WinLike = Parameters<typeof prefersReducedMotion>[0];

/**
 * Pure, testable predicate: should this view change animate? True only when the
 * API exists AND reduced motion is not requested. SSR/throw-safe via the args.
 */
export function canViewTransition(
  doc: DocLike = typeof document === 'undefined' ? undefined : (document as DocLike),
  win: WinLike = typeof window === 'undefined' ? undefined : (window as WinLike),
): boolean {
  return typeof doc?.startViewTransition === 'function' && !prefersReducedMotion(win);
}

/**
 * Run `update` inside a View Transition when supported; otherwise run it
 * directly. Always runs `update` exactly once either way — so callers get the
 * same state change regardless of animation support.
 */
export function runViewTransition(update: () => void): void {
  if (!canViewTransition()) {
    update();
    return;
  }
  (document as DocLike)!.startViewTransition!(() => flushSync(update));
}
