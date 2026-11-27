// The old global sheet entry was removed. Verify ordinary tab navigation
// never leaves its former page lock or backdrop behind.
export const id = 'chat-sheet-scroll-lock';
export const title = 'Work and chat navigation leaves page scrolling unlocked';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/chatSheetScrollLock.ts'];
export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  const nav = page.locator('nav.vnext-nav');
  await nav.waitFor({ state: 'visible', timeout: 30000 });
  for (let i = 0; i < 3; i++) {
    await nav.locator('button').filter({ hasText: /工作|Work/ }).first().click();
    await page.locator('.vnext-app-work').waitFor();
    await nav.locator('button').first().click();
    await page.getByTestId('aegis-chat-full').waitFor({ state: 'visible' });
  }
  const locked = await page.locator('.vnext-shell').evaluate(el => el.style.overflow === 'hidden' || el.style.overflowY === 'hidden');
  if (!locked && await page.getByTestId('aegis-chat-sheet-backdrop').count() === 0) ok('tab navigation leaves no sheet backdrop or scroll lock');
  else fail('stale sheet lock after navigation');
  await shot('chat-after-repeated-navigation');
}
