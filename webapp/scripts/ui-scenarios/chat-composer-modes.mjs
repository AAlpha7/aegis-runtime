// Composer size toggle and the empty-send reason. Phone viewport: the toggle
// stays reachable while a draft is focused, and half mode keeps attach / size /
// send on one toolbar under the field.
export const id = 'chat-composer-modes';
export const title = 'Chat composer: empty send shows why, size toggle survives typing, half mode is one toolbar';
export const surfaces = ['home'];

async function box(page, selector) {
  return page.locator(selector).first().evaluate((el) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return {
      x: r.x,
      y: r.y,
      w: r.width,
      h: r.height,
      display: s.display,
    };
  });
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(800);
  const textarea = page.locator('.vnext-chat-composer textarea').first();
  await textarea.waitFor({ state: 'visible', timeout: 15000 });
  const send = page.locator('.vnext-chat-composer button[type="submit"]').first();
  const reason = page.locator('.vnext-chat-composer-send-reason').first();
  const size = page.locator('.vnext-chat-size-button').first();

  if (await send.isDisabled()) ok('empty send starts disabled');
  else fail('empty send is enabled and would no-op');
  const placeholder = await textarea.getAttribute('placeholder');
  if (placeholder && placeholder.trim().length > 0) ok('placeholder already says to type', placeholder);
  else fail('composer placeholder is empty');
  if (await reason.count() === 0) ok('empty send does not add a second sentence');
  else fail('empty send still shows a floating reason', (await reason.textContent()) || '');
  await shot('01-empty-reason');

  await size.click();
  await page.waitForFunction(() => document.querySelector('.vnext-chat-composer')?.dataset.mode === 'half');
  // Mode change focuses the field, which hides attach until blur. Wait for that
  // focus, then blur: the field is still empty, so the paperclip returns.
  await page.waitForTimeout(300);
  await textarea.blur();
  await page.waitForTimeout(200);
  const attach = await box(page, '.vnext-chat-attach-button');
  const field = await box(page, '.vnext-chat-composer textarea');
  const halfSend = await box(page, '.vnext-chat-composer button[type="submit"]');
  if (attach.display !== 'none' && attach.y + 8 >= field.y + field.h - 8) ok('attach sits on the toolbar, not above the field');
  else fail('attach stranded above the half-mode field', JSON.stringify({ attach, field }));
  if (halfSend.x > attach.x && Math.abs((attach.y + attach.h / 2) - (halfSend.y + halfSend.h / 2)) <= 12) {
    ok('half mode send stays on the trailing end of the toolbar');
  } else fail('half mode send left the toolbar', JSON.stringify({ attach, halfSend }));
  await shot('02-half-toolbar');

  await textarea.click();
  await textarea.fill('测试');
  await page.waitForTimeout(200);
  if (await send.isEnabled()) ok('send enables once there is text');
  else fail('send stayed disabled after typing');
  if (await reason.count() === 0) ok('reason leaves once send is enabled');
  else fail('reason still shown after the field has text');
  const sizeWhileTyping = await box(page, '.vnext-chat-size-button');
  if (sizeWhileTyping.display !== 'none' && sizeWhileTyping.w > 0) ok('size toggle stays visible while typing');
  else fail('size toggle hidden while typing', JSON.stringify(sizeWhileTyping));
  await shot('03-typing-size-visible');

  await size.click();
  await page.waitForFunction(() => document.querySelector('.vnext-chat-composer')?.dataset.mode === 'line');
  const back = await box(page, '.vnext-chat-size-button');
  if (back.display !== 'none' && back.w > 0) ok('size toggle still available after collapsing');
  else fail('size toggle missing after collapsing', JSON.stringify(back));
  await shot('04-back-to-line');
}
