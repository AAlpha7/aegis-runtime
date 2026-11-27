/**
 * X (Twitter) deep links — complement the native X app, don't replace it.
 *
 * Prefer https://x.com/... (and legacy twitter.com) so mobile OS can hand off
 * to the installed X app. Compose uses official intent URLs; we never silent-post.
 */

import { ownerFacingUrlText } from './ownerUrlText';

const X_HOSTS = new Set(['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com', 'mobile.twitter.com']);

/** Normalize to canonical https://x.com/... for opening in browser/app. */
export function normalizeXUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
    const url = new URL(withScheme);
    if (!X_HOSTS.has(url.hostname.toLowerCase())) return null;
    if (url.username || url.password) return null;
    // Canonical host for deep-link handoff.
    url.hostname = 'x.com';
    url.protocol = 'https:';
    // Strip tracking noise; keep path/query needed for status/intent.
    url.hash = '';
    return url.href;
  } catch {
    return null;
  }
}

/** Status id from /user/status/123 or /i/web/status/123 */
export function extractXStatusId(raw: string): string | null {
  const href = normalizeXUrl(raw);
  if (!href) return null;
  try {
    const path = new URL(href).pathname;
    const m = path.match(/\/(?:i\/web\/)?status(?:es)?\/(\d+)/i)
      || path.match(/\/[^/]+\/status(?:es)?\/(\d+)/i);
    return m?.[1] ?? null;
  } catch {
    return null;
  }
}

/** @handle from profile URL path (not status). */
export function extractXHandle(raw: string): string | null {
  const href = normalizeXUrl(raw);
  if (!href) return null;
  try {
    const path = new URL(href).pathname.replace(/\/+$/, '');
    if (/\/status(?:es)?\//i.test(path)) return null;
    const m = path.match(/^\/@?([A-Za-z0-9_]{1,15})$/);
    if (!m) return null;
    const handle = m[1];
    if (['i', 'home', 'explore', 'search', 'settings', 'intent', 'compose'].includes(handle.toLowerCase())) {
      return null;
    }
    return handle;
  } catch {
    return null;
  }
}

export function isXUrl(raw: string): boolean {
  return normalizeXUrl(raw) !== null;
}

const TRACKING_PARAMS = new Set(['s', 't', 'ref_src', 'ref_url', 'src', 'cn', 'mx', 'twclid']);

function statusAuthor(href: string): string | null {
  try {
    const match = new URL(href).pathname.match(/^\/([A-Za-z0-9_]{1,15})\/status(?:es)?\/\d+/i);
    if (!match || match[1].toLowerCase() === 'i') return null;
    return match[1];
  } catch {
    return null;
  }
}

/**
 * One URL per post or profile. Tracking queries and /i/web/status aliases
 * collapse; intent/compose URLs keep their query.
 */
export function canonicalXUrl(raw: string): string | null {
  const href = normalizeXUrl(raw);
  if (!href) return null;
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.pathname.startsWith('/intent/') || url.pathname.startsWith('/compose/')) {
    return url.href;
  }
  const statusId = extractXStatusId(href);
  if (statusId) {
    const author = statusAuthor(href);
    return author
      ? `https://x.com/${author}/status/${statusId}`
      : `https://x.com/i/status/${statusId}`;
  }
  const handle = extractXHandle(href);
  if (handle) return `https://x.com/${handle}`;
  for (const key of [...url.searchParams.keys()]) {
    const lower = key.toLowerCase();
    if (TRACKING_PARAMS.has(lower) || lower.startsWith('utm_')) url.searchParams.delete(key);
  }
  url.hash = '';
  url.search = url.searchParams.toString() ? `?${url.searchParams.toString()}` : '';
  if (url.pathname.length > 1) url.pathname = url.pathname.replace(/\/+$/, '');
  return url.href;
}

function xLinkIdentity(href: string): string | null {
  const statusId = extractXStatusId(href);
  if (statusId) return `status:${statusId}`;
  const handle = extractXHandle(href);
  if (handle) return `user:${handle.toLowerCase()}`;
  return href;
}

function preferRicherXUrl(current: string, next: string): string {
  return !statusAuthor(current) && statusAuthor(next) ? next : current;
}

/** First-seen order. The same status id or handle is kept once, preferring an author path. */
export function dedupeXLinks(links: string[]): string[] {
  const order: string[] = [];
  const byKey = new Map<string, string>();
  for (const raw of links) {
    const canonical = canonicalXUrl(raw);
    if (!canonical) continue;
    const key = xLinkIdentity(canonical);
    if (!key) continue;
    const prev = byKey.get(key);
    if (!prev) {
      order.push(key);
      byKey.set(key, canonical);
      continue;
    }
    byKey.set(key, preferRicherXUrl(prev, canonical));
  }
  return order.map((key) => byKey.get(key) ?? '').filter(Boolean);
}

function xLinkCaption(href: string): string {
  try {
    const status = new URL(href).pathname.match(/^\/([A-Za-z0-9_]{1,15})\/status\/(\d+)$/i);
    if (status && status[1].toLowerCase() !== 'i') return `@${status[1]}`;
    const id = extractXStatusId(href);
    if (id) return `#${id.slice(-4)}`;
    const handle = extractXHandle(href);
    if (handle) return `@${handle}`;
    // pathname is percent-encoded (%E2%80%A6 is an ellipsis). Decode before
    // the length cut so a multibyte sequence is never sliced in half.
    const tail = ownerFacingUrlText(new URL(href).pathname.replace(/^\//, ''));
    return tail.slice(0, 24) || 'x.com';
  } catch {
    return 'x.com';
  }
}

function disambiguatedCaptions(hrefs: string[]): string[] {
  const base = hrefs.map(xLinkCaption);
  const counts = new Map<string, number>();
  for (const caption of base) counts.set(caption, (counts.get(caption) ?? 0) + 1);
  return hrefs.map((href, index) => {
    const caption = base[index] ?? 'x.com';
    if ((counts.get(caption) ?? 0) < 2) return caption;
    const id = extractXStatusId(href);
    return id ? `${caption} · ${id.slice(-4)}` : caption;
  });
}

export type XHandoff = {
  /** Post or profile the primary Open button jumps to. */
  openHref: string;
  /** Reply compose for that same post. Null for profiles and non-status URLs. */
  replyHref: string | null;
  /** Other distinct destinations. Captions differ from the Open button label. */
  extras: Array<{ href: string; caption: string }>;
};

/**
 * One owner surface should show the X handoff once. When a finished answer is
 * already open, its strip owns Open / Draft reply, including the work item's
 * own post. The work dock stays in charge until that answer is on screen.
 */
export function mergeWorkAndAnswerXHandoff(options: {
  assistantOpen: boolean;
  finishedAnswerVisible: boolean;
  workHref: string | null;
  answerLinks: string[];
}): { suppressDock: boolean; extraLinks: string[] } {
  if (!options.assistantOpen || !options.finishedAnswerVisible) {
    return { suppressDock: false, extraLinks: [] };
  }
  const hasHandoff = Boolean(options.workHref) || xHandoffTargets(options.answerLinks) !== null;
  if (!hasHandoff) return { suppressDock: false, extraLinks: [] };
  return {
    suppressDock: true,
    extraLinks: options.workHref ? [options.workHref] : [],
  };
}

/**
 * One handoff row: a single Open action, an optional draft reply for that
 * same post, and at most two other destinations with their own captions.
 */
export function xHandoffTargets(links: string[]): XHandoff | null {
  const unique = dedupeXLinks(links);
  if (unique.length === 0) return null;
  const openHref = unique.find((href) => extractXStatusId(href)) ?? unique[0];
  const extras = unique.filter((href) => href !== openHref).slice(0, 2);
  const captions = disambiguatedCaptions(extras);
  return {
    openHref,
    replyHref: buildXReplyComposeUrl(openHref),
    extras: extras.map((href, index) => ({ href, caption: captions[index] ?? xLinkCaption(href) })),
  };
}

/** Pull first/canonical X URLs from free text (assistant answers, item summaries). */
export function extractXUrlsFromText(text: string, max = 8): string[] {
  if (!text) return [];
  const found: string[] = [];
  // Bare URLs and markdown links.
  const pattern = /https?:\/\/(?:www\.)?(?:x|twitter)\.com\/[^\s)\]>"']+/gi;
  for (const match of text.matchAll(pattern)) {
    found.push(match[0].replace(/[.,;:!?]+$/, ''));
    if (found.length >= 24) break;
  }
  return dedupeXLinks(found).slice(0, max);
}

/** Scan common owner-work fields for an openable X URL. */
export function extractXUrlFromWorkItem(item: {
  title?: string | null;
  summary?: string | null;
  owner_visible_reason?: string | null;
  next_safe_action?: string | null;
  external_url?: string | null;
  source_id?: string | null;
}): string | null {
  const candidates = [
    item.external_url,
    item.source_id && isXUrl(item.source_id) ? item.source_id : null,
    item.title,
    item.summary,
    item.owner_visible_reason,
    item.next_safe_action,
  ];
  for (const c of candidates) {
    if (!c) continue;
    if (isXUrl(c.trim())) {
      const n = normalizeXUrl(c.trim());
      if (n) return n;
    }
    const fromText = extractXUrlsFromText(c, 1)[0];
    if (fromText) return fromText;
  }
  return null;
}

/**
 * Official web intent for composing a post (opens X app/site; owner sends).
 * https://developer.x.com/en/docs/twitter-for-websites/tweet-button/guides/web-intent
 */
export function buildXComposeUrl(options: {
  text?: string;
  replyToStatusId?: string;
  url?: string;
} = {}): string {
  const params = new URLSearchParams();
  const text = options.text?.trim();
  if (text) params.set('text', text.slice(0, 280));
  if (options.replyToStatusId?.trim()) params.set('in_reply_to', options.replyToStatusId.trim());
  if (options.url?.trim()) {
    const n = normalizeXUrl(options.url) ?? options.url.trim();
    params.set('url', n);
  }
  const q = params.toString();
  // x.com intent/post is the current surface; twitter.com/intent/tweet still works.
  return q ? `https://x.com/intent/post?${q}` : 'https://x.com/compose/post';
}

export function buildXReplyComposeUrl(statusUrlOrId: string, text?: string): string | null {
  const id = /^\d+$/.test(statusUrlOrId.trim())
    ? statusUrlOrId.trim()
    : extractXStatusId(statusUrlOrId);
  if (!id) return null;
  return buildXComposeUrl({ text, replyToStatusId: id });
}

/** Safe open: only https X URLs / compose intents. */
export { ownerFacingUrlText } from './ownerUrlText';

export function openXDeepLink(raw: string): boolean {
  const href = normalizeXUrl(raw)
    || (raw.startsWith('https://x.com/intent/') || raw.startsWith('https://twitter.com/intent/')
      ? raw
      : null);
  if (!href || typeof window === 'undefined') return false;
  try {
    window.open(href, '_blank', 'noopener,noreferrer');
    return true;
  } catch {
    return false;
  }
}
