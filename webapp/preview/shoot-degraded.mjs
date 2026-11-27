// Real screenshots of the degraded-provider chat surface (sticky banner + chat
// error header + retry button) to verify there is NO English leak in a zh
// (display_language) session. Captures the `degraded-zh` and `degraded` views.
//
// Usage (from webapp/, with preview server serving on :8799):
//   node preview/shoot-degraded.mjs
// Writes /tmp/degraded-shots/{degraded-zh,degraded}-{light,dark}.png
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://127.0.0.1:8799/index.html';
const OUT = '/tmp/degraded-shots';
const ROUTES = ['degraded-zh', 'degraded'];
const SCHEMES = ['light', 'dark'];

mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
try {
  for (const colorScheme of SCHEMES) {
    const context = await browser.newContext({
      colorScheme,
      viewport: { width: 414, height: 896 },
      deviceScaleFactor: 2,
    });
    for (const route of ROUTES) {
      const page = await context.newPage();
      await page.goto(`${BASE}?view=${route}`, { waitUntil: 'networkidle' });
      await page.waitForTimeout(600);
      const file = `${OUT}/${route}-${colorScheme}.png`;
      await page.screenshot({ path: file, fullPage: true });
      console.log(`wrote ${file}`);
      await page.close();
    }
    await context.close();
  }
} finally {
  await browser.close();
}
