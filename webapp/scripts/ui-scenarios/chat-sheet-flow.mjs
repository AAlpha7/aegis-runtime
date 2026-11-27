// Historical scenario id retained; v1 now uses the main chat tab instead of
// a duplicate global sheet opener. Item-scoped questions remain in Work.
export const id = 'chat-sheet-flow';
export const title = 'Main chat navigation preserves draft and a single composer';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/ChatHomeView.tsx', 'webapp/src/vnext/OwnerShellChrome.tsx'];
export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  const nav = page.locator('nav.vnext-nav');
  await nav.waitFor({ state: 'visible', timeout: 30000 });
  const input = page.locator('[data-testid="aegis-chat-full"] textarea').first();
  await input.fill('Keep this draft while checking Work');
  await input.blur();
  await nav.locator('button').filter({ hasText: /工作|Work/ }).first().click();
  await page.locator('.vnext-app-work').waitFor();
  if (await page.getByTestId('aegis-chat-sheet-open').count() === 0) ok('no duplicate global chat opener');
  else fail('duplicate chat opener remains');
  await shot('work-without-duplicate-opener');
  await nav.locator('button').first().click();
  await input.waitFor({ state: 'visible' });
  if (await input.inputValue() === 'Keep this draft while checking Work') ok('main chat preserves draft across navigation');
  else fail('chat draft was lost');
  if (await page.locator('textarea:visible').count() === 1) ok('one visible composer');
  else fail('duplicate visible composers');
  await input.fill('');
  await shot('main-chat-restored');
}
