// A compressed PNG's byte size depends on theme and font rendering, not page
// correctness. Decode it and check dimensions/content; flow assertions remain
// responsible for verifying the UI itself.
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { chromium } from 'playwright';
const paths = process.argv.slice(2);
if (!paths.length) throw new Error('Expected screenshot paths');
const browser = await chromium.launch({ executablePath: execFileSync('which', ['chromium'], { encoding: 'utf8' }).trim() });
try {
  const page = await browser.newPage();
  for (const path of paths) {
    const result = await page.evaluate(async (base64) => {
      const img = new Image();
      img.src = `data:image/png;base64,${base64}`;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = 96;
      canvas.height = 96;
      const ctx = canvas.getContext('2d');
      ctx.drawImage(img, 0, 0, 96, 96);
      const pixels = ctx.getImageData(0, 0, 96, 96).data;
      const colors = new Map();
      for (let i = 0; i < pixels.length; i += 4) {
        const key = [pixels[i], pixels[i + 1], pixels[i + 2]].map(v => v >> 4).join(',');
        colors.set(key, (colors.get(key) || 0) + 1);
      }
      return { width: img.width, height: img.height, colors: colors.size,
        variation: 1 - Math.max(...colors.values()) / (96 * 96) };
    }, readFileSync(path).toString('base64'));
    if (result.width < 320 || result.height < 500 || result.colors < 8 || result.variation < 0.01) {
      throw new Error(`Missing screenshot content: ${path}: ${JSON.stringify(result)}`);
    }
    console.log(`Screenshot content OK: ${path}: ${JSON.stringify(result)}`);
  }
} finally {
  await browser.close();
}
