/**
 * Plan C tab-signal screenshots.
 *
 * Captures the bottom nav (.vnext-nav) with both signals lit:
 *   - Chat tab: unseen-dot (calm accent)
 *   - Work tab: warn-toned count badge (3 decisions)
 *
 * Both dark (default) and light colorSchemes at 414x200 (viewport shows only
 * the nav area). WebKit is used as the rendering engine (owner's iPhone target).
 *
 * Usage (from webapp/):
 *   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-planC.mjs
 */
import { webkit } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = new URL('./planC/', import.meta.url).pathname;
const SCHEMES = ['dark', 'light'];

mkdirSync(OUT, { recursive: true });

const browser = await webkit.launch();
try {
  for (const colorScheme of SCHEMES) {
    const context = await browser.newContext({
      colorScheme,
      viewport: { width: 414, height: 200 },
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    await page.goto(`${BASE}?view=planC-nav`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(400);
    const file = `${OUT}nav-badges-${colorScheme}.png`;
    await page.screenshot({ path: file, fullPage: false });
    console.log(`wrote ${file}`);
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
