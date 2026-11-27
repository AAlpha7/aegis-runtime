// The console is a LIGHT surface, but its CSS still carries dark-era literals
// (styles.css has ~29 hardcoded `rgba(9,22,36,…)`-class fills). Most are inert;
// the ones that are not render as a slate slab inside a white page — the
// expanded work item did exactly that until E20, and nothing caught it because
// every existing assertion measures geometry, not colour.
//
// This scenario locks the ground rule instead of chasing literals: walk the
// rendered page and fail on any sizeable element whose painted background is
// dark while the app ground is light. It names the offenders so the fix is a
// lookup, not a hunt.
import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'light-theme-no-dark-slabs';
export const title = 'Light console: no dark-era slab survives on the page ground';
export const surfaces = ['webapp/src/vnext/styles.css'];

/** Elements that are MEANT to be dark on a light ground. */
const INTENTIONALLY_DARK = [
  'button',            // the primary action is a dark pill by design
  '.vnext-btn',
  '.vnext-avatar',     // avatar/logo discs carry their own artwork
  '.vnext-brandmark',
];

async function scanForSlabs(page) {
  return page.evaluate((intentional) => {
    const parse = (c) => {
      const m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/.exec(c || '');
      return m ? { r: +m[1], g: +m[2], b: +m[3], a: m[4] === undefined ? 1 : +m[4] } : null;
    };
    const lum = ({ r, g, b }) => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
    // The app paints its own ground on .vnext-app; html/body are still dark-era,
    // so body is the wrong reference and would make every check vacuously pass.
    const groundEl = document.querySelector('.vnext-app');
    if (!groundEl) return { ground: null, offenders: [] };
    const ground = parse(getComputedStyle(groundEl).backgroundColor);
    if (!ground || ground.a < 0.5) return { ground: null, offenders: [] };
    const groundLum = lum(ground);
    if (groundLum <= 0.5) return { ground: groundLum, offenders: [], dark: true };

    const offenders = [];
    const seen = new Set();
    for (const el of groundEl.querySelectorAll('*')) {
      const r = el.getBoundingClientRect();
      if (r.width < 40 || r.height < 24) continue;
      const cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none') continue;
      if (intentional.some((sel) => el.matches(sel))) continue;
      const bg = parse(cs.backgroundColor);
      if (!bg || bg.a < 0.15) continue;
      const l = lum(bg);
      if (l >= 0.35) continue;
      const cls = typeof el.className === 'string' ? el.className.trim().split(/\s+/).slice(0, 3).join('.') : '';
      const key = `${el.tagName.toLowerCase()}.${cls}`;
      if (seen.has(key)) continue;
      seen.add(key);
      offenders.push({ sel: key, bg: cs.backgroundColor, lum: +l.toFixed(3), w: Math.round(r.width), h: Math.round(r.height) });
    }
    return { ground: +groundLum.toFixed(3), offenders };
  }, INTENTIONALLY_DARK);
}

export async function run({ page, ok, fail, shot }) {
  await page.setViewportSize({ width: 1440, height: 900 });
  // Without injected items the work page has no expanded panel, so the colour
  // check would run against a page that cannot contain the defect and pass
  // vacuously — verified: with the slab restored, an un-injected run still
  // reported "no dark slab". Same fixture as work-desktop-controls.
  await page.route('**/webapp/vnext/work*', async (route) => {
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(injectedWorkPayload(base)) });
  });
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('.vnext-app').waitFor({ state: 'visible', timeout: 60000 });
  await page.waitForTimeout(1500);

  const check = async (label) => {
    const { ground, offenders, dark } = await scanForSlabs(page);
    if (ground === null) {
      fail(`light theme: ${label} — no .vnext-app ground to measure against`);
      return;
    }
    if (dark) {
      // A deliberately dark console is not this scenario's business.
      ok(`light theme: ${label} — ground is dark (${ground}), slab rule not applicable`);
      return;
    }
    if (offenders.length === 0) ok(`light theme: ${label} — no dark slab on a ${ground} ground`);
    else fail(`light theme: ${label} — dark-era slab still painted`, JSON.stringify(offenders));
  };

  await check('chat');
  await shot('01-chat');

  const workTab = page.locator('button:visible').filter({ hasText: /^\s*工作\s*$/ }).first();
  if (await workTab.count()) {
    await workTab.click();
    await page.waitForTimeout(1500);
    const rows = page.locator('.vnext-item-row:visible');
    if (await rows.count()) {
      await rows.first().click();
      await page.waitForTimeout(1200);
      // Expand a nearby item: its inline detail is where the last slab lived.
      // On the desktop two-pane (2026-09-16) the nearby list lives only inside
      // the collapsed 更多工作 brief, so open that first when no row is visible.
      const inline = () => page.evaluate(() => document.querySelectorAll('.vnext-nearby-inline-detail').length);
      let nearby = page.locator('.vnext-nearby-work-item .vnext-item-row:visible');
      if (!(await nearby.count())) {
        const brief = page.locator('details:has(.vnext-nearby-work-item) > summary').first();
        if (await brief.count()) {
          await brief.click();
          await page.waitForTimeout(500);
          nearby = page.locator('.vnext-nearby-work-item .vnext-item-row:visible');
        }
      }
      if ((await nearby.count()) && (await inline()) === 0) {
        await nearby.first().click();
        await page.waitForTimeout(1000);
      }
    }
    const panels = await page.evaluate(() => document.querySelectorAll('.vnext-nearby-inline-detail').length);
    if (panels === 0) fail('light theme: work — expanded panel never rendered, colour check would be vacuous');
    else ok(`light theme: work — expanded panel rendered (${panels}), colour check has a subject`);
    await check('work');
    await shot('02-work');
  } else {
    fail('light theme: no 工作 tab to open');
  }
}
