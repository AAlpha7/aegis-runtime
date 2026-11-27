// SP1 button-system visual proof: every migrated view + the gallery, both
// themes, on WebKit (owner's iPhone engine) AND Chromium.
// Usage (from webapp/, server on :8802):
//   PREVIEW_BASE=http://localhost:8802/index.html SP1_VIEWS=button-system node preview/shoot-sp1.mjs
import { webkit, chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8802/index.html';
const OUT = '/tmp/sp1-shots';
mkdirSync(OUT, { recursive: true });

const VIEWS = (process.env.SP1_VIEWS || 'button-system').split(',').map((v) => v.trim()).filter(Boolean);
const THEMES = ['light', 'dark'];
const ENGINES = [['webkit', webkit], ['chromium', chromium]];

for (const [engineName, engine] of ENGINES) {
  const browser = await engine.launch();
  try {
    for (const view of VIEWS) {
      for (const scheme of THEMES) {
        const context = await browser.newContext({
          colorScheme: scheme,
          viewport: { width: 1280, height: 900 },
          deviceScaleFactor: 2,
        });
        const page = await context.newPage();
        await page.goto(`${BASE}?view=${view}`, { waitUntil: 'networkidle' });
        await page.waitForTimeout(500);
        const body = await page.locator('body').innerText();
        if (body.includes('is not wired yet')) {
          throw new Error(`view "${view}" is not wired in harness.tsx`);
        }
        const file = `${OUT}/${view}-${scheme}-${engineName}.png`;
        await page.screenshot({ path: file, fullPage: true });
        console.log(`wrote ${file}`);
        await page.close();
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }
}
