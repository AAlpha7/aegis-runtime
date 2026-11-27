// Slice 4 visual proof: the zh chat-home surface with an in-chat HITL approval
// card. Shows the pending card (summary + 批准/先不要 buttons) plus a resolved
// card (已批准 chip). Mobile (414x896) + desktop (1280x900).
//
// Usage (from webapp/, preview server serving on $PREVIEW_BASE):
//   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-chat-approval.mjs
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = '/tmp/chat-approval-shots';
mkdirSync(OUT, { recursive: true });

const VIEWPORTS = [
  { name: 'mobile', width: 414, height: 896 },
  { name: 'desktop', width: 1280, height: 900 },
];

const browser = await chromium.launch();
try {
  for (const vp of VIEWPORTS) {
    const context = await browser.newContext({
      colorScheme: 'light',
      viewport: { width: vp.width, height: vp.height },
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    await page.goto(`${BASE}?view=chat-approval-zh`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(600);
    // Assert the approval card actually rendered (fail loudly otherwise).
    const body = await page.locator('body').innerText();
    for (const needle of ['需要你确认', '我准备做', '为什么要你看', '批准', '先不要', '已批准', 'billing@northwest.example']) {
      if (!body.includes(needle)) {
        throw new Error(`approval card text "${needle}" not found in render`);
      }
    }
    const file = `${OUT}/chat-approval-zh-${vp.name}.png`;
    await page.screenshot({ path: file, fullPage: true });
    console.log(`wrote ${file} (found approval card text: yes)`);
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
