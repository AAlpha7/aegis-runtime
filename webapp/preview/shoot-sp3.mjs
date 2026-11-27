// SP3 Work-view i18n leak-cleanup screenshots on Playwright WEBKIT (the owner's
// iPhone 16 Pro client is iOS Safari/WebKit). Captures Work list + detail in zh
// (dark+light) plus an en control, at 414x896 @3x.
//
// Usage (from webapp/, preview server serving on :8799):
//   node preview/shoot-sp3.mjs
// Writes /tmp/sp3-shots/{route}-{scheme}.png
import { webkit } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = '/tmp/sp3-shots';

const SHOTS = [
  { route: 'work-zh', schemes: ['dark', 'light'] },
  { route: 'work-detail-zh', schemes: ['dark', 'light'] },
  { route: 'work-decide-zh', schemes: ['dark'] },
  { route: 'work-en', schemes: ['dark'] },
  { route: 'work-detail-en', schemes: ['dark'] },
  // SP3 round 2-3: Status/Settings i18n (Connections disclosure, fallback
  // projection operations grid, source/section labels). zh dark+light + en control.
  { route: 'status-zh', schemes: ['dark', 'light'] },
  { route: 'status-en', schemes: ['dark'] },
  { route: 'settings-zh', schemes: ['dark', 'light'] },
  { route: 'settings-en', schemes: ['dark'] },
];

mkdirSync(OUT, { recursive: true });

const browser = await webkit.launch();
try {
  for (const { route, schemes } of SHOTS) {
    for (const colorScheme of schemes) {
      const context = await browser.newContext({
        colorScheme,
        viewport: { width: 414, height: 896 },
        deviceScaleFactor: 3,
      });
      const page = await context.newPage();
      await page.goto(`${BASE}?view=${route}`, { waitUntil: 'networkidle' });
      await page.waitForTimeout(700);
      const file = `${OUT}/${route}-${colorScheme}.png`;
      await page.screenshot({ path: file, fullPage: true });
      console.log(`wrote ${file}`);
      await page.close();
      await context.close();
    }
  }
} finally {
  await browser.close();
}
