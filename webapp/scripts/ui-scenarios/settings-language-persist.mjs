// Settings writes must reach the server from a Telegram-principal session
// (no CSRF token). 2026-09-11: the language switch flipped only the settings
// pane and reverted on reload because the client gated every persist* call on
// session.csrf_token. The evidence instance authenticates like the phone.
export const id = 'settings-language-persist';
export const title = 'Settings: language choice is persisted from a Telegram session';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/api.ts',
  'webapp/src/i18n/catalog.ts',
];

async function gotoSettings(page) {
  await page.goto('/ui/?view=settings', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(600);
}

async function clickLanguage(page, label) {
  const btn = page.locator('.vnext-settings-language-toggle button', { hasText: label }).first();
  await btn.waitFor({ state: 'visible', timeout: 10000 });
  const wait = page.waitForResponse((r) => r.url().includes('/webapp/vnext/settings/display-language') && r.request().method() === 'POST', { timeout: 10000 }).catch(() => null);
  await btn.click();
  return wait;
}

const navText = (page) => page.evaluate(() => (document.querySelector('nav.vnext-nav')?.innerText || '').replace(/\s+/g, ' ').trim());

export async function run({ page, ok, fail, shot }) {
  await gotoSettings(page);
  try {
    const res = await clickLanguage(page, 'English');
    await page.waitForTimeout(800);
    await shot('01-english-chosen');
    if (res && res.status() === 200) ok('display-language POST reached the server', `status ${res.status()}`);
    else fail('display-language POST missing or failed', res ? `status ${res.status()}` : 'no request');
    const nav = await navText(page);
    if (/Work/.test(nav) && !/工作/.test(nav)) ok('nav follows the language switch', nav);
    else fail('nav did not switch to English', nav);

    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
    await page.waitForTimeout(900);
    await shot('02-after-reload');
    const navAfter = await navText(page);
    if (/Work/.test(navAfter)) ok('language persists across reload', navAfter);
    else fail('language reverted on reload', navAfter);
  } finally {
    // Leave the shared instance in zh for the scenarios that follow.
    await gotoSettings(page);
    const back = await clickLanguage(page, '中文');
    await page.waitForTimeout(800);
    await shot('03-back-to-zh');
    if (back && back.status() === 200) ok('restored 中文', `status ${back.status()}`);
    else fail('could not restore 中文', back ? `status ${back.status()}` : 'no request');
  }
}
