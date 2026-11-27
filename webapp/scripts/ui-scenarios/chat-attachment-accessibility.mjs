// Synthetic uploads exercise the actual composer without storing owner files.
export const id = 'chat-attachment-accessibility';
export const title = 'Attachment loading, error recovery and keyboard removal across themes';
export const surfaces = ['webapp/src/vnext/ChatHomeView.tsx', 'webapp/src/vnext/styles.css', 'webapp/src/vnext/design/surfaces/chat-home.css'];

export async function run({ page, ok, fail, shot, viewport }) {
  const uploadPattern = '**/webapp/vnext/upload';
  const filename = '季度报告_需要仔细阅读的附件名称_'.repeat(8) + '.pdf';
  let releaseUpload;
  let uploadSeen;
  let outcome = 'error';
  await page.route(uploadPattern, async route => {
    uploadSeen = true;
    await new Promise(resolve => { releaseUpload = resolve; });
    if (outcome === 'error') await route.fulfill({ status: 503, json: { status: 'error', error: 'The attachment upload could not finish because the document service is temporarily unavailable. Keep your original file and try uploading again in a moment.' } });
    else await route.fulfill({ json: { type: 'document', filename, mime_type: 'application/pdf', data_url: 'data:application/pdf;base64,JVBERi0xLjQK' } });
  });
  const check = (value, label, detail = '') => value ? ok(label, detail) : fail(label, detail);
  try {
    for (const size of [viewport, { width: 1440, height: 900 }]) {
      await page.setViewportSize(size);
      for (const theme of ['light', 'dark']) {
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
        const input = page.locator('.vnext-chat-composer textarea');
        await input.waitFor({ state: 'visible', timeout: 30000 });
        const attach = page.locator('.vnext-chat-attach-button');
        const pill = page.locator('.vnext-chat-attached-file');
        const prefix = `${size.width}px ${theme}`;
        for (const result of ['error', 'success']) {
          outcome = result;
          uploadSeen = false;
          // Leave the phone typing layout using the keyboard before attaching.
          // Returning focus after Remove intentionally hides the auxiliary row.
          await input.press('Tab');
          await attach.waitFor({ state: 'visible' });
          // Enter must activate the real paperclip and native file chooser.
          await attach.focus();
          const chooserPromise = page.waitForEvent('filechooser');
          await attach.press('Enter');
          await (await chooserPromise).setFiles({ name: filename, mimeType: 'application/pdf', buffer: Buffer.from('%PDF-1.4\n') });
          await page.waitForFunction(() => Boolean(document.querySelector('.vnext-chat-attached-file.is-uploading')));
          await page.waitForTimeout(100);
          check(uploadSeen && await attach.isDisabled(), `${prefix} ${result}: upload is pending and paperclip disabled`);
          check(await pill.getAttribute('role') === 'status', `${prefix} ${result}: upload progress announced`);
          releaseUpload();
          await page.locator('.vnext-chat-attached-file:not(.is-uploading)').waitFor({ state: 'visible' });
          const remove = pill.locator('button');
          const geometry = await pill.evaluate(el => {
            const text = el.querySelector('span');
            const button = el.querySelector('button');
            const r = button.getBoundingClientRect();
            return { clipped: text.scrollWidth > text.clientWidth + 1 || text.scrollHeight > text.clientHeight + 1, width: r.width, height: r.height, inViewport: r.left >= 0 && r.right <= innerWidth && r.top >= 0 && r.bottom <= innerHeight, overflow: document.documentElement.scrollWidth > innerWidth + 1 };
          });
          check(geometry.width >= 40 && geometry.height >= 40 && geometry.inViewport && !geometry.overflow, `${prefix} ${result}: removal target fits`, JSON.stringify(geometry));
          if (result === 'error') {
            check(await pill.getAttribute('role') === 'alert', `${prefix}: upload failure announced`);
            check(!geometry.clipped, `${prefix}: complete recovery message visible`, JSON.stringify(geometry));
          } else {
            check((await pill.textContent()).includes(filename), `${prefix}: long filename retained`);
            check(await page.locator('.vnext-chat-composer button[type="submit"]').isEnabled(), `${prefix}: attachment-only send enabled`);
          }
          await attach.focus();
          await attach.press('Shift+Tab');
          check(await remove.evaluate(el => document.activeElement === el), `${prefix} ${result}: remove reachable with Shift+Tab`);
          check(await remove.evaluate(el => el.matches(':focus-visible') && (getComputedStyle(el).outlineStyle !== 'none' || getComputedStyle(el).boxShadow !== 'none')), `${prefix} ${result}: keyboard focus visible`);
          await shot(`${size.width}-${theme}-${result}`);
          await remove.press('Enter');
          await pill.waitFor({ state: 'hidden' });
          check(await input.evaluate(el => document.activeElement === el), `${prefix} ${result}: removal returns focus to composer`);
        }
      }
    }
  } finally {
    releaseUpload?.();
    await page.unroute(uploadPattern);
    await page.setViewportSize(viewport);
  }
}
