/**
 * Owner-facing URL text.
 *
 * The URL parser keeps non-ASCII percent-encoded. A UTF-8 ellipsis (U+2026)
 * is the three bytes E2 80 A6, so `URL.pathname` and a raw href show
 * `%E2%80%A6` instead of `…`. Navigation hrefs stay encoded and valid;
 * every owner-visible label passes through this decoder.
 */
export function ownerFacingUrlText(value: string): string {
  if (!value || !/%[0-9A-Fa-f]{2}/.test(value)) return value;
  const bytes: number[] = [];
  let out = '';
  const flush = () => {
    if (bytes.length === 0) return;
    try {
      out += new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(bytes));
    } catch {
      for (const byte of bytes) {
        out += `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
      }
    }
    bytes.length = 0;
  };
  for (let i = 0; i < value.length; i += 1) {
    const hex = value.slice(i + 1, i + 3);
    if (value[i] === '%' && /^[0-9A-Fa-f]{2}$/.test(hex)) {
      bytes.push(Number.parseInt(hex, 16));
      i += 2;
      continue;
    }
    flush();
    out += value[i];
  }
  flush();
  return out;
}
