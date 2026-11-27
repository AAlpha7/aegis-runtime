import { useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';

/**
 * Aurora Glass motion helpers.
 *
 * The pure functions (`revealDelayMs`, `prefersReducedMotion`) carry the logic
 * and are unit-tested; the hooks are thin glue over them so a DOM is not needed
 * to exercise the behaviour that matters.
 */

/** Stagger delay (ms) for the Nth revealing element, clamped so long lists don't lag. */
export function revealDelayMs(index: number, stepMs = 60, maxMs = 480): number {
  if (!Number.isFinite(index)) {
    // +Infinity should still clamp to the cap; anything else non-finite → 0.
    return index === Number.POSITIVE_INFINITY ? maxMs : 0;
  }
  const n = Math.round(index);
  if (n <= 0) return 0;
  return Math.min(n * stepMs, maxMs);
}

type WinLike = { matchMedia?: (q: string) => { matches: boolean } } | undefined;

/** SSR-safe, throw-safe read of the reduced-motion preference. */
export function prefersReducedMotion(
  win: WinLike = typeof window !== 'undefined' ? (window as unknown as WinLike) : undefined,
): boolean {
  try {
    return Boolean(win?.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
  } catch {
    return false;
  }
}

/** Reactive reduced-motion preference, updating if the user toggles it. */
export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(() => prefersReducedMotion());
  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const onChange = () => setReduced(mq.matches);
    onChange();
    // Safari < 14 only supports addListener.
    if (mq.addEventListener) mq.addEventListener('change', onChange);
    else mq.addListener(onChange);
    return () => {
      if (mq.removeEventListener) mq.removeEventListener('change', onChange);
      else mq.removeListener(onChange);
    };
  }, []);
  return reduced;
}

export interface RevealOptions {
  /** Stagger index for this element (0 = no delay). */
  index?: number;
  stepMs?: number;
  maxMs?: number;
  /** IntersectionObserver rootMargin; defaults to revealing slightly before in view. */
  rootMargin?: string;
  /** Reveal once then stop observing (default true). */
  once?: boolean;
}

/**
 * Reveal-on-enter hook. Returns a ref to attach and a `revealed` flag plus the
 * computed stagger delay (ms). Reveals immediately when reduced motion is on or
 * IntersectionObserver is unavailable (SSR / old WebView), so content is never
 * stuck hidden.
 */
export function useReveal<T extends HTMLElement = HTMLDivElement>(
  options: RevealOptions = {},
): { ref: RefObject<T | null>; revealed: boolean; delayMs: number } {
  const { index = 0, stepMs, maxMs, rootMargin = '0px 0px -8% 0px', once = true } = options;
  const reduced = useReducedMotion();
  const ref = useRef<T | null>(null);
  const [revealed, setRevealed] = useState<boolean>(false);

  useEffect(() => {
    if (reduced || typeof IntersectionObserver === 'undefined') {
      setRevealed(true);
      return;
    }
    const node = ref.current;
    if (!node) return;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            setRevealed(true);
            if (once) observer.disconnect();
          } else if (!once) {
            setRevealed(false);
          }
        }
      },
      { rootMargin },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [reduced, rootMargin, once]);

  return { ref, revealed, delayMs: reduced ? 0 : revealDelayMs(index, stepMs, maxMs) };
}
