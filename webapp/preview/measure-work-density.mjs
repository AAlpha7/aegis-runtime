// 402px WebKit density probe for Work view (scan order + decide-card height).
// Usage: PREVIEW_BASE=http://localhost:8799/index.html node preview/measure-work-density.mjs
import { webkit, chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = process.env.SHOT_OUT || '/tmp/work-density-probe';
const PHASE = process.env.SHOT_PHASE || 'measure';
const SCHEMES = ['light', 'dark'];
const ENGINE = process.env.BROWSER_ENGINE || 'webkit';

mkdirSync(OUT, { recursive: true });

async function measure(page, label) {
  const metrics = await page.evaluate(() => {
    const stack = document.querySelector('.vnext-stack');
    const scan = document.querySelector('.vnext-work-scan');
    const lanes = document.querySelector('.vnext-work-mobile-inline-lanes');
    const blocked = document.querySelector('.vnext-blocked-panel');
    const detail = document.querySelector('.vnext-work-inline-selected.vnext-detail, .vnext-detail');
    const actionPanel = document.querySelector('.vnext-action-panel');
    const primaryRow = document.querySelector('.vnext-action-primary-row');
    const ai = document.querySelector('.vnext-item-ai-entry');
    const step = document.querySelector('.vnext-item-primary-step');

    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return {
        top: Math.round(r.top),
        left: Math.round(r.left),
        width: Math.round(r.width),
        height: Math.round(r.height),
        bottom: Math.round(r.bottom),
      };
    };

    let scanOrder = null;
    let lanesOrder = null;
    if (scan && stack) {
      scanOrder = getComputedStyle(scan).order;
    }
    if (lanes && stack) {
      lanesOrder = getComputedStyle(lanes).order;
    }

    const scanAboveLanes = (() => {
      if (!scan || !lanes) return null;
      const sr = scan.getBoundingClientRect();
      const lr = lanes.getBoundingClientRect();
      return sr.bottom <= lr.top + 1 || sr.top < lr.top;
    })();

    return {
      scan: rect(scan),
      lanes: rect(lanes),
      blocked: rect(blocked),
      detail: rect(detail),
      actionPanel: rect(actionPanel),
      primaryRow: rect(primaryRow),
      ai: rect(ai),
      step: rect(step),
      scanOrder,
      lanesOrder,
      scanAboveLanes,
      sideBySide: ai && step
        ? Math.abs(ai.getBoundingClientRect().top - step.getBoundingClientRect().top) < 12
          && ai.getBoundingClientRect().right <= step.getBoundingClientRect().left + 4
        : null,
    };
  });
  console.log(JSON.stringify({ label, phase: PHASE, ...metrics }, null, 2));
  return metrics;
}

const launcher = ENGINE === 'chromium' ? chromium : webkit;
const browser = await launcher.launch();
const results = {};

try {
  for (const colorScheme of SCHEMES) {
    const context = await browser.newContext({
      colorScheme,
      viewport: { width: 402, height: 874 },
      deviceScaleFactor: 2,
    });

    // Work (focal card / density)
    {
      const page = await context.newPage();
      await page.goto(`${BASE}?view=work`, { waitUntil: 'networkidle' });
      await page.waitForTimeout(500);
      const m = await measure(page, `work-${colorScheme}`);
      results[`work-${colorScheme}`] = m;
      const file = `${OUT}/${PHASE}-work-${colorScheme}-402.png`;
      await page.screenshot({ path: file, fullPage: true });
      console.log(`wrote ${file}`);
      await page.close();
    }

    // Decide (paused decision card)
    {
      const page = await context.newPage();
      await page.goto(`${BASE}?view=work-decide`, { waitUntil: 'networkidle' });
      await page.waitForTimeout(500);
      const m = await measure(page, `work-decide-${colorScheme}`);
      results[`work-decide-${colorScheme}`] = m;
      const file = `${OUT}/${PHASE}-work-decide-${colorScheme}-402.png`;
      await page.screenshot({ path: file, fullPage: true });
      console.log(`wrote ${file}`);
      // Also crop blocked panel if present
      const blocked = page.locator('.vnext-blocked-panel');
      if (await blocked.count()) {
        await blocked.first().screenshot({ path: `${OUT}/${PHASE}-blocked-${colorScheme}-402.png` });
      }
      await page.close();
    }

    await context.close();
  }
} finally {
  await browser.close();
}

writeFileSync(`${OUT}/${PHASE}-metrics.json`, JSON.stringify(results, null, 2));
console.log(`metrics -> ${OUT}/${PHASE}-metrics.json`);
if (results['work-decide-light']?.blocked?.height != null) {
  console.log(`DECIDE_CARD_HEIGHT_LIGHT=${results['work-decide-light'].blocked.height}`);
}
if (results['work-light']?.scanAboveLanes != null) {
  console.log(`SCAN_ABOVE_LANES=${results['work-light'].scanAboveLanes}`);
  console.log(`SCAN_ORDER=${results['work-light'].scanOrder} LANES_ORDER=${results['work-light'].lanesOrder}`);
}
