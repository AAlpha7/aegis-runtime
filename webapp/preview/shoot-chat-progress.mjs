// Slice 3 visual proof: the zh chat-home surface mid-turn whose working message
// shows the localized live-progress text "正在使用 web_search…" (driven by a
// `tool_start` SSE frame in App.tsx). Mobile (414x896) + desktop (1280x900).
//
// Usage (from webapp/, preview server serving on $PREVIEW_BASE):
//   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-chat-progress.mjs
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = '/tmp/chat-progress-shots';
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
    await page.goto(`${BASE}?view=chat-progress-zh`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(600);
    // Assert the live-progress text actually rendered (fail loudly otherwise).
    const body = await page.locator('body').innerText();
    if (!body.includes('正在使用 web_search')) {
      throw new Error('live-progress working text "正在使用 web_search…" not found in render');
    }
    const file = `${OUT}/chat-progress-zh-${vp.name}.png`;
    await page.screenshot({ path: file, fullPage: true });
    console.log(`wrote ${file} (found live-progress text: yes)`);
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
