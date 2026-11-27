// Auto-update for the owner PWA.
//
// An iOS standalone PWA keeps the page it loaded and never reloads on its own,
// so after a webapp deploy the owner can stay stuck on the PRE-deploy bundle
// indefinitely — the service worker has no `fetch` handler, so it isn't serving
// stale assets; the stale view is purely the in-memory session (owner feedback
// 2026-06-16: a width fix shipped but the owner kept seeing the old render).
//
// This polls the no-store `index.html` for the live main-bundle hash and reloads
// when it differs from the bundle THIS session is running — but only at SAFE
// moments (on returning from background, plus one delayed self-heal after load),
// never on a timer mid-interaction, and with a cooldown that makes a reload LOOP
// impossible even if the origin is briefly inconsistent.

const BUNDLE_RE = /index-([a-f0-9]{8})\.js/;
const HASH_RE = /^[a-f0-9]{8}$/;
const COOLDOWN_KEY = 'aegis-auto-update-reloaded-for';
const COOLDOWN_MS = 60_000;
const INITIAL_DELAY_MS = 15_000;

/** Pull the content-hash of the main `index-<hash>.js` bundle from a string
 * (a script `src`, or the index.html text). Returns null when absent. */
export function extractBundleId(text: string | null | undefined): string | null {
  if (!text) return null;
  const m = text.match(BUNDLE_RE);
  return m ? m[1] : null;
}

/** Pure decision: reload ONLY when both ids are valid 8-hex hashes AND differ.
 * Anything uncertain (null / malformed / equal) is a NO so we never reload
 * spuriously or loop. */
export function shouldReloadForUpdate(
  current: string | null | undefined,
  latest: string | null | undefined,
): boolean {
  return (
    !!current &&
    !!latest &&
    HASH_RE.test(current) &&
    HASH_RE.test(latest) &&
    current !== latest
  );
}

function currentBundleId(): string | null {
  const scripts = Array.from(document.querySelectorAll<HTMLScriptElement>('script[src]'));
  for (const s of scripts) {
    const id = extractBundleId(s.src);
    if (id) return id;
  }
  return null;
}

async function fetchLatestBundleId(): Promise<string | null> {
  const res = await fetch('/ui/', { cache: 'no-store' });
  if (!res.ok) return null;
  return extractBundleId(await res.text());
}

/** Returns true if we are inside the cooldown for this exact target version
 * (we already tried to reload to it very recently) — the loop backstop. */
function inCooldown(latest: string): boolean {
  try {
    const raw = sessionStorage.getItem(COOLDOWN_KEY);
    if (!raw) return false;
    const { id, at } = JSON.parse(raw) as { id?: string; at?: number };
    return id === latest && typeof at === 'number' && Date.now() - at < COOLDOWN_MS;
  } catch {
    return false;
  }
}

function markCooldown(latest: string): void {
  try {
    sessionStorage.setItem(COOLDOWN_KEY, JSON.stringify({ id: latest, at: Date.now() }));
  } catch {
    /* sessionStorage unavailable (private mode quota etc.) — proceed without it */
  }
}

export function startAutoUpdate(): void {
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  const current = currentBundleId();
  // Dev server / unhashed build / can't identify our own version → opt out
  // entirely (never reload on an unknown baseline).
  if (!current) return;

  let reloading = false;
  let wasHidden = document.visibilityState === 'hidden';

  const maybeReload = async (): Promise<void> => {
    if (reloading) return;
    let latest: string | null;
    try {
      latest = await fetchLatestBundleId();
    } catch {
      return; // network blip — try again next safe moment
    }
    if (!shouldReloadForUpdate(current, latest)) return;
    if (inCooldown(latest!)) return; // loop backstop
    markCooldown(latest!);
    reloading = true;
    window.location.reload();
  };

  // Safe moment 1: returning to the foreground after being backgrounded — the
  // exact iOS-PWA stale-session case, and a moment the owner is not mid-typing.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
      if (wasHidden) void maybeReload();
      wasHidden = false;
    } else {
      wasHidden = true;
    }
  });

  // Safe moment 2: a one-shot self-heal shortly after load (corrects a session
  // that somehow loaded a stale index.html). After it reloads once, the new
  // page's current === latest, so it does not fire again.
  window.setTimeout(() => {
    void maybeReload();
  }, INITIAL_DELAY_MS);
}
