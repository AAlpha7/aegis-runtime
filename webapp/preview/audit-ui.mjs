// Systematic UI audit (owner request 2026-07-10: "全面检查ui自己识别和修正问题").
// For every preview fixture view × color scheme, run MECHANICAL detectors and
// emit a JSON report + screenshots for human/model visual review.
//
// Detectors (per view/scheme):
//   overflow      — document horizontal scroll OR any element wider than viewport
//   clipped       — elements extending beyond viewport right edge (>4px) that are
//                   NOT inside a working horizontal scroll container (overflow-x
//                   auto/scroll ancestor, or an explicit [data-audit-scroll])
//   touch-target  — interactive elements (<40px min dimension) on mobile
//   contrast      — text nodes with WCAG-ish contrast < 3.0 vs its background
//   raw-artifact  — rendered text containing =?UTF-8?=, quoted-printable (=E2=),
//                   MIME boundary (------=_Part), unescaped &amp;/&#x, {placeholders}
//   mixed-lang    — zh views whose rendered text contains long English sentences
//                   (heuristic: >6 consecutive ASCII words outside code/emails/urls)
//   hardcoded     — computed colors not derived from tokens is uncheckable at
//                   runtime; instead flag inline style="" attrs carrying colors
//
// KNOWN FALSE-POSITIVE CLASSES (2026-07-10 calibration):
//   * gradient-background contrast — text on obsidian/champagne/garnet gradient
//     button fills used to measure against the PAGE background (gradients are
//     not parseable as a color) and false-flagged at 1.2–1.5:1. Fixed
//     STRUCTURALLY: the background walk now stops at a gradient ancestor and
//     skips the sample (unknowable rather than wrong).
//   * fullPage-sticky artifact — `fullPage:true` screenshots re-position
//     sticky/fixed chrome (chat topbar, bottom nav) once per scroll page; any
//     doubled/misplaced bar seen ONLY in a fullPage capture is a screenshot
//     artifact, not a layout bug. Affects the PNGs, not the JSON findings.
//   * harness chrome — findings whose element exists only in the preview
//     harness shell (not the product) may be suppressed via ALLOWLIST below.
//
// Remaining intentional/known findings are suppressed per-entry via ALLOWLIST
// (each entry needs a comment saying WHY). Everything else lands in the report;
// webapp/preview/audit-baseline.json records the accepted post-polish state and
// the nightly compares new reports against it (report-only, non-gating).
//
// Usage: (cd preview && python3 -m http.server 8797) &
//        node preview/audit-ui.mjs [viewport-width]
// Env:   PREVIEW_BASE — full harness URL (default http://localhost:$AUDIT_PORT/index.html)
//        AUDIT_PORT   — port for the default BASE (default 8797)
//        AUDIT_OUT    — output dir (default /tmp/ui-audit)
//        AUDIT_BASELINE — optional path to a baseline report.json; when set,
//                         per view|scheme|kind counts exceeding the baseline are
//                         printed as "BASELINE-EXCEEDED …" lines (exit stays 0).
import { webkit } from 'playwright';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';

const PORT = process.env.AUDIT_PORT || '8797';
const BASE = process.env.PREVIEW_BASE || `http://localhost:${PORT}/index.html`;
const OUT = process.env.AUDIT_OUT || '/tmp/ui-audit';
mkdirSync(OUT, { recursive: true });

// Suppressions for findings that are KNOWN-intentional or harness-only.
// Match: view (exact), kind (exact), detail substring. Keep this list SHORT and
// every entry commented — an uncommented entry is a review-blocker.
const ALLOWLIST = [
  // Visually-hidden file inputs (attachment upload) are 0/1px by design; they
  // are proxied by a visible ≥40px button, so the "touch-target" hit is noise.
  { kind: 'touch-target', detail: 'vnext-hidden-file-input' },
];
const allowed = (f) =>
  ALLOWLIST.some((a) => (!a.view || a.view === f.view) && (!a.kind || a.kind === f.kind) && (a.detail && f.detail.includes(a.detail)));

const VIEWS = ['home','empty','degraded','degraded-zh','chat-progress-zh','chat-approval-zh','chat-attachment-zh','work','work-decide','work-detail','work-zh','work-en','work-detail-zh','work-detail-en','work-decide-zh','status','status-zh','status-en','settings','settings-zh','settings-en','richtext','assistant','loading','error','button-system','controls-system','header-menu-system'];
const WIDTH = Number(process.argv[2] || 402); // iPhone 16 Pro pt width
const HEIGHT = 874;

const findings = [];

// state-contrast: semantic-state pairs must differ by a PERCEIVABLE visual
// delta, not an alpha whisper (2026-07-22 owner report: two row states were
// indistinguishable — root cause was a 1.6%-alpha background difference plus
// full-bright text on both). For each configured pair, render the view and
// require a TEXT-COLOR delta (>=25 RGB distance) between the two states.
// Background differences and lone marker elements are deliberately NOT
// accepted as evidence — that is exactly the combination that failed. A pair
// whose selectors resolve to nothing is ALSO a finding (vacuous-pass guard).
// Guards the whole "state encoded too subtly" class. Entries are
// { view, a, b, marker, label }; the only configured pair (mail rows) left
// with the mail client, so the list is empty until a new state pair needs it.
const STATE_PAIRS = [];
const rgbDist = (x, y) => {
  const px = x.match(/[\d.]+/g)?.map(Number) ?? [0,0,0,1];
  const py = y.match(/[\d.]+/g)?.map(Number) ?? [0,0,0,1];
  const ax = px.length > 3 ? px[3] : 1, ay = py.length > 3 ? py[3] : 1;
  return Math.sqrt(((px[0]??0)*ax-(py[0]??0)*ay)**2 + ((px[1]??0)*ax-(py[1]??0)*ay)**2 + ((px[2]??0)*ax-(py[2]??0)*ay)**2);
};
async function checkStatePairs(page, view, scheme) {
  for (const pair of STATE_PAIRS.filter((p) => p.view === view)) {
    const got = await page.evaluate((sel) => {
      const g = (q, prop) => { const el = document.querySelector(q); return el ? getComputedStyle(el)[prop] : null; };
      return {
        colorA: g(sel.a, 'color'), colorB: g(sel.b, 'color'),
        hasMarker: !!document.querySelector(sel.marker),
      };
    }, pair);
    if (!got.colorA || !got.colorB) {
      // A configured pair that resolves to nothing means the fixture or a
      // class name drifted — surface it rather than silently passing.
      findings.push({ view, scheme, kind: 'state-contrast',
        detail: `${pair.label}: selector resolved to no element (a=${!!got.colorA} b=${!!got.colorB}) — guard is vacuous` });
      continue;
    }
    const textDelta = rgbDist(got.colorA, got.colorB);
    // Perceivable = the TEXT color itself differs meaningfully between states.
    // Background strings are deliberately NOT accepted as evidence: the failing
    // design had textually-different-but-visually-identical alpha tints (4% vs
    // 2.4%), and a lone marker dot was exactly what the owner reported as
    // insufficient. Text hierarchy is the one lever this sensor certifies.
    if (textDelta < 25) {
      findings.push({ view, scheme, kind: 'state-contrast',
        detail: `${pair.label}: textDelta=${textDelta.toFixed(1)} (need >=25) marker=${got.hasMarker}` });
    }
  }
}
const browser = await webkit.launch();
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: WIDTH, height: HEIGHT }, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  for (const view of VIEWS) {
    try {
      await page.goto(`${BASE}?view=${view}`, { waitUntil: 'networkidle', timeout: 20000 });
      await page.waitForTimeout(400);
      const r = await page.evaluate((vw) => {
        const out = [];
        const doc = document.documentElement;
        if (doc.scrollWidth > vw + 2) out.push({ kind: 'overflow', detail: `scrollWidth ${doc.scrollWidth} > ${vw}` });
        const els = Array.from(document.querySelectorAll('*'));
        // An element inside a WORKING horizontal scroll container is not
        // clipped — the repo norm ("wide content must scroll inside its own
        // overflow-x container") is satisfied, e.g. .vnext-rich-table-scroll.
        const inScrollContainer = (el) => {
          for (let n = el.parentElement; n && n !== doc; n = n.parentElement) {
            const ox = getComputedStyle(n).overflowX;
            if ((ox === 'auto' || ox === 'scroll') && n.clientWidth <= vw) return true;
          }
          return false;
        };
        let clipped = 0; const clippedSamples = [];
        for (const el of els) {
          const rect = el.getBoundingClientRect();
          if (rect.width > 0 && rect.right > vw + 4 && getComputedStyle(el).overflowX !== 'hidden'
              && !el.closest('[data-audit-scroll]') && !inScrollContainer(el)) {
            clipped++; if (clippedSamples.length < 3) clippedSamples.push(`${el.tagName}.${(el.className||'').toString().split(' ')[0]}@${Math.round(rect.right)}`);
          }
        }
        if (clipped) out.push({ kind: 'clipped', detail: `${clipped} els past right edge: ${clippedSamples.join(', ')}` });
        // touch targets
        const interactive = Array.from(document.querySelectorAll('button, a, [role="button"], input, select'));
        const small = interactive.filter((el) => { const r2 = el.getBoundingClientRect(); return r2.width > 0 && (r2.width < 40 || r2.height < 32); }).slice(0, 5)
          .map((el) => `${el.tagName}.${(el.className||'').toString().split(' ')[0]}`);
        if (small.length) out.push({ kind: 'touch-target', detail: small.join(', ') });
        // raw artifacts in rendered text
        const text = document.body.innerText || '';
        for (const pat of [/=\?UTF-8\?/i, /=E2=\d\d/, /------=_Part/, /&amp;|&#x/, /\{\{?[a-z_]+\}\}?/]) {
          const m = text.match(pat);
          if (m) out.push({ kind: 'raw-artifact', detail: `${pat} → "…${text.slice(Math.max(0, m.index - 20), m.index + 30).replace(/\n/g, ' ')}…"` });
        }
        // contrast (sampled: text-bearing leaf elements)
        const parseC = (c) => { const m2 = c.match(/rgba?\(([\d.]+), ([\d.]+), ([\d.]+)(?:, ([\d.]+))?\)/); return m2 ? [+m2[1], +m2[2], +m2[3], m2[4] === undefined ? 1 : +m2[4]] : null; };
        const lum = ([r2, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r2) + 0.7152 * f(g) + 0.0722 * f(b); };
        const scheme = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
        // Walk ancestors for the effective background. A gradient ancestor
        // (backgroundImage) makes the true backdrop unparseable — return null
        // and SKIP the sample instead of falling through to the page color
        // (that fallthrough was the "gradient-background contrast" false-
        // positive class: gold-on-obsidian button labels measured 1.4:1
        // against the light page they never sit on).
        const bgOf = (el) => {
          let n = el;
          while (n && n !== document.documentElement) {
            const cs = getComputedStyle(n);
            const c = parseC(cs.backgroundColor);
            if (c && c[3] > 0.9) return c;
            if (cs.backgroundImage && cs.backgroundImage !== 'none') return null;
            n = n.parentElement;
          }
          return scheme === 'dark' ? [11, 15, 26, 1] : [244, 246, 251, 1];
        };
        const low = [];
        for (const el of els) {
          if (low.length >= 4) break;
          if (!el.childNodes.length || el.children.length) continue;
          const t = (el.textContent || '').trim(); if (t.length < 3) continue;
          const rect = el.getBoundingClientRect(); if (rect.width === 0 || rect.height === 0) continue;
          const fg = parseC(getComputedStyle(el).color); if (!fg || fg[3] < 0.5) continue;
          const bg = bgOf(el); if (!bg) continue;
          const l1 = lum(fg), l2 = lum(bg);
          const ratio = (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
          if (ratio < 3.0) low.push(`"${t.slice(0, 24)}" ${ratio.toFixed(2)}:1`);
        }
        if (low.length) out.push({ kind: 'contrast', detail: low.join(' | ') });
        // inline color styles
        const inline = els.filter((el) => /color\s*:\s*(#|rgb)/i.test(el.getAttribute('style') || '')).slice(0, 3)
          .map((el) => `${el.tagName}.${(el.className||'').toString().split(' ')[0]}`);
        if (inline.length) out.push({ kind: 'hardcoded-inline-color', detail: inline.join(', ') });
        return out;
      }, WIDTH);
      // zh mixed-language heuristic (JS side, needs innerText again)
      if (view.endsWith('-zh')) {
        const text = await page.locator('body').innerText();
        const runs = text.match(/(?:\b[A-Za-z]{2,}\b[ ,.'&-]+){7,}\b[A-Za-z]{2,}\b/g) || [];
        const real = runs.filter((x) => !/@|http|\.com|UTC|GMT|[A-Z]{2,}\d|v\d|Gmail|IMAP|OAuth|Outlook|Hotmail|Codex|Grok|API/i.test(x)).slice(0, 3);
        if (real.length) r.push({ kind: 'mixed-lang', detail: real.map((x) => x.slice(0, 60)).join(' || ') });
      }
      for (const f of r) {
        const finding = { view, scheme, ...f };
        if (!allowed(finding)) findings.push(finding);
      }
      // NOTE: fullPage screenshots re-anchor sticky/fixed bars per scroll page
      // (the "fullPage-sticky artifact" false-positive class) — judge sticky
      // chrome from the top viewport-height only.
      await page.screenshot({ path: `${OUT}/${view}-${scheme}.png`, fullPage: true });
      await checkStatePairs(page, view, scheme);
    } catch (e) {
      findings.push({ view, scheme, kind: 'render-error', detail: String(e).slice(0, 120) });
    }
  }
  await ctx.close();
}
await browser.close();
writeFileSync(`${OUT}/report.json`, JSON.stringify(findings, null, 2));
const byKind = {};
for (const f of findings) byKind[f.kind] = (byKind[f.kind] || 0) + 1;
console.log(`AUDIT DONE: ${findings.length} findings across ${VIEWS.length} views × 2 schemes`);
console.log(JSON.stringify(byKind, null, 1));

// ---- Optional baseline comparison (report-only; exit code stays 0) ---------
// Baseline = a committed report.json snapshot of the ACCEPTED state
// (webapp/preview/audit-baseline.json). Counts are compared per
// view|scheme|kind; regressions print BASELINE-EXCEEDED lines the nightly
// surfaces in its ceo-inbox browser report. render-error is always compared
// (a view that stops rendering must never hide behind the baseline).
const baselinePath = process.env.AUDIT_BASELINE;
if (baselinePath) {
  const keyCounts = (list) => {
    const m = {};
    for (const f of list) { const k = `${f.view}|${f.scheme}|${f.kind}`; m[k] = (m[k] || 0) + 1; }
    return m;
  };
  let exceeded = 0;
  try {
    const baseline = keyCounts(JSON.parse(readFileSync(baselinePath, 'utf8')));
    const current = keyCounts(findings);
    for (const [k, n] of Object.entries(current).sort()) {
      const b = baseline[k] || 0;
      if (n > b) { exceeded++; console.log(`BASELINE-EXCEEDED ${k}: ${n} > ${b}`); }
    }
    console.log(exceeded ? `BASELINE-DELTA: ${exceeded} view|scheme|kind buckets above baseline` : 'BASELINE-OK: no bucket above baseline');
  } catch (e) {
    console.log(`BASELINE-ERROR: ${String(e).slice(0, 120)}`);
  }
}
