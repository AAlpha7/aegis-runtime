// Attachment download + inline-preview UI proof shots (light + dark) for the
// Work detail attachment list (server-signed download urls). Renders the
// work-detail panel whose fixture carries a previewable pdf, a download-only
// docx, and a metadata-only png.
//   PREVIEW_BASE=http://localhost:8799/index.html node preview/shoot-attachments.mjs
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = process.env.PREVIEW_BASE || 'http://localhost:8799/index.html';
const OUT = process.env.OUT || '/tmp/attachment-shots';
mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
try {
  for (const colorScheme of ['light', 'dark']) {
    const context = await browser.newContext({ colorScheme, viewport: { width: 414, height: 896 }, deviceScaleFactor: 2 });
    const page = await context.newPage();
    await page.goto(`${BASE}?view=work-detail`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(400);
    const list = page.locator('.vnext-attachment-list').first();
    await list.scrollIntoViewIfNeeded();
    await list.screenshot({ path: `${OUT}/attachments-${colorScheme}.png` });
    console.log(`wrote ${OUT}/attachments-${colorScheme}.png`);
    // Open the pdf preview toggle to verify the inline <iframe> preview renders.
    const toggle = page.locator('.vnext-attachment-preview-toggle').first();
    if (await toggle.count()) {
      await toggle.click();
      await page.waitForTimeout(400);
      await list.screenshot({ path: `${OUT}/attachments-preview-open-${colorScheme}.png` });
      console.log(`wrote ${OUT}/attachments-preview-open-${colorScheme}.png`);
    }
    await page.close();
    await context.close();
  }
} finally {
  await browser.close();
}
