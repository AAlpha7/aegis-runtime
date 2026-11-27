// Slice 5 visual proof: the zh chat-home composer holding a pending image
// attachment. Shows the clip/attach button + the pending-attachment PILL
// (已附加：收据.png) with its remove button. Mobile (414x896) + desktop (1280x900).
//
// Usage (from webapp/, preview server serving on $PREVIEW_BASE):
//   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-chat-attachment.mjs
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = '/tmp/chat-attachment-shots';
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
    await page.goto(`${BASE}?view=chat-attachment-zh`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(600);
    // Assert the pending-attachment pill + attach button actually rendered.
    const body = await page.locator('body').innerText();
    for (const needle of ['已附加：收据.png']) {
      if (!body.includes(needle)) {
        throw new Error(`attachment pill text "${needle}" not found in render`);
      }
    }
    // The clip button + remove button are aria-labelled (not text) — assert via DOM.
    const attachButton = await page.locator('.vnext-chat-attach-button[aria-label="添加附件"]').count();
    if (attachButton < 1) throw new Error('attach (clip) button not found');
    const removeButton = await page.locator('.vnext-chat-attached-file button[aria-label="移除附件"]').count();
    if (removeButton < 1) throw new Error('attachment remove button not found');
    const file = `${OUT}/chat-attachment-zh-${vp.name}.png`;
    await page.screenshot({ path: file, fullPage: true });
    console.log(`wrote ${file} (found pill + attach + remove: yes)`);
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
