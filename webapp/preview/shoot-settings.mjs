// Settings-view contrast proof shots (light + dark) for the button-text
// contrast fix (owner 2026-06-16 "setting那边有一些按钮文字颜色看不清").
// Usage (from webapp/, preview server serving on $PREVIEW_BASE):
//   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-settings.mjs
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = process.env.OUT || '/tmp/settings-contrast';
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
    const page = await context.newPage();
    await page.goto(`${BASE}?view=settings`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(600);
    // Expand every collapsed <details> so debug/model/notification buttons show.
    const summaries = page.locator('details:not([open]) > summary');
    const n = await summaries.count();
    for (let i = 0; i < n; i += 1) {
      try {
        const s = summaries.nth(i);
        if (await s.isVisible()) { await s.click({ timeout: 1500 }); await page.waitForTimeout(120); }
      } catch { /* skip */ }
    }
    await page.waitForTimeout(300);
    const file = `${OUT}/settings-${colorScheme}.png`;
    await page.screenshot({ path: file, fullPage: true });
    console.log(`wrote ${file}`);
    // Crisp per-button element shots for the suspect classes.
    const SUSPECTS = [
      'vnext-settings-support-card', 'vnext-debug-terminal-grant',
      'vnext-debug-duration-picker', 'vnext-debug-scope-select',
      'vnext-debug-grant-token', 'vnext-debug-token-unavailable',
      'vnext-debug-log-toggle', 'vnext-debug-job',
    ];
    for (const cls of SUSPECTS) {
      const sel = '.' + cls.trim().split(/\s+/).join('.');
      const loc = page.locator(sel).first();
      try {
        if (await loc.count() && await loc.isVisible()) {
          const f = `${OUT}/btn-${cls.replace(/\s+/g, '-')}-${colorScheme}.png`;
          await loc.scrollIntoViewIfNeeded();
          await loc.screenshot({ path: f });
          console.log(`  wrote ${f}`);
        } else {
          console.log(`  (absent/hidden: ${sel})`);
        }
      } catch (e) { console.log(`  (err ${sel}: ${e.message})`); }
    }
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
