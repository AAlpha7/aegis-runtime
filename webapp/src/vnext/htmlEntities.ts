/*
 * Some API message snippets arrive HTML-entity-encoded (the API pre-escapes
 * them for direct innerHTML use: &#39; &amp; &quot; …). We render them as
 * React TEXT nodes (which escape again), so without decoding, the raw entity
 * shows literally in the inbox list ("I can&#39;t edit…" — owner IMG_3938).
 * Stored rows are already encoded, so decode happens at display time, not
 * ingestion. Pure string transform — no DOM — so it stays SSR-safe for the
 * renderToStaticMarkup test harness.
 *
 * Single left-to-right pass: double-encoded input ("&amp;#39;") decodes one
 * layer ("&#39;") instead of collapsing to an apostrophe, which is the
 * correct behavior for display of a single decode step.
 */
const NAMED_ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
};

export function decodeHtmlEntities(input: string): string {
  if (!input.includes('&')) return input;
  return input.replace(/&(#[xX]?[0-9a-fA-F]+|[a-zA-Z]+);/g, (match, body: string) => {
    if (body.startsWith('#')) {
      const hex = body[1] === 'x' || body[1] === 'X';
      const digits = body.slice(hex ? 2 : 1);
      if (!hex && !/^[0-9]+$/.test(digits)) return match;
      const code = Number.parseInt(digits, hex ? 16 : 10);
      // Reject out-of-range and lone-surrogate code points rather than
      // fabricating invalid UTF-16.
      if (!Number.isFinite(code) || code < 0 || code > 0x10ffff) return match;
      if (code >= 0xd800 && code <= 0xdfff) return match;
      return String.fromCodePoint(code);
    }
    return NAMED_ENTITIES[body.toLowerCase()] ?? match;
  });
}
