// Scenario: 设置 → 高级 → AI 与花费 in view — every select draws its chevron, segmented
// controls carry a visible caption (owner feedback 2026-09-09: "不点不知道是下拉").
export const id = 'settings-ai-card';
export const title = 'Settings AI card in view (chevron, captions, toggle copy)';
export const surfaces = ['settings'];
async function openHeaderMenu(page) {
  // Home chat chrome uses chatHome.account.menu ("账户菜单" / "Account menu").
  // Non-home OwnerShellChrome uses app.shell.header.menu ("打开菜单" / "Open menu").
  const trigger = page.locator(
    [
      '.vnext-header-menu-trigger[aria-label="打开菜单"]',
      '.vnext-header-menu-trigger[aria-label="Open menu"]',
      '.vnext-header-menu-trigger[aria-label="账户菜单"]',
      '.vnext-header-menu-trigger[aria-label="Account menu"]',
    ].join(', '),
  ).first();
  await trigger.waitFor({ state: 'visible', timeout: 15000 });
  await trigger.click();
  await page.locator('[role="menu"]').waitFor({ state: 'visible', timeout: 10000 });
}

/** Click a menuitem whose visible label matches any of the given exact strings. */
async function clickMenuItem(page, labels) {
  const items = page.locator('[role="menuitem"]');
  await items.first().waitFor({ state: 'visible', timeout: 10000 });
  const count = await items.count();
  for (let i = 0; i < count; i += 1) {
    const item = items.nth(i);
    const labelEl = item.locator('.vnext-header-menu-item-label');
    const raw = (await labelEl.count()) ? await labelEl.innerText() : await item.innerText();
    const text = (raw || '').trim();
    if (labels.includes(text)) {
      await item.click();
      return text;
    }
  }
  throw new Error(`menuitem not found among ${JSON.stringify(labels)}; saw ${count} items`);
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(1200);
  await openHeaderMenu(page);
  await clickMenuItem(page, ['设置', 'Settings']);
  await page.locator('.vnext-settings-hero, [data-settings-section="account"]').first().waitFor({ state: 'visible', timeout: 15000 });
  await page.waitForTimeout(500);
  await page.evaluate(() => {
    const advanced = document.querySelector('details.vnext-settings-tier.advanced');
    if (advanced) advanced.open = true;
  });
  await page.waitForTimeout(600);
  const info = await page.evaluate(() => {
    const advanced = document.querySelector('details.vnext-settings-tier.advanced');
    const card = advanced && advanced.querySelector('section, .vnext-settings-card, article');
    const target = card || advanced;
    if (target) target.scrollIntoView({ block: 'start' });
    const selects = Array.from(document.querySelectorAll('.vnext-settings-tier.advanced select'));
    const captions = Array.from(document.querySelectorAll('.vnext-settings-tier.advanced .vnext-segmented-label')).map((n) => n.textContent.trim());
    const firstOptions = selects.map((s) => (s.options[0] ? s.options[0].textContent.trim() : ''));
    const chevron = selects.map((s) => getComputedStyle(s).backgroundImage.startsWith('url('));
    return { selects: selects.length, captions, firstOptions, chevron };
  });
  await page.waitForTimeout(500);
  await shot('01-ai-card');
  console.log('ai-card info', JSON.stringify(info));
  if (info.selects > 0 && info.chevron.every(Boolean)) ok(`every select in 高级 draws a chevron (${info.selects})`); else fail('select without chevron', JSON.stringify(info.chevron));
  if (info.captions.length > 0) ok(`segmented captions: ${info.captions.join(' / ')}`); else fail('no segmented caption in 高级');
  if (info.firstOptions[0] && info.firstOptions[0].startsWith('服务商默认')) ok(`model select first option: ${info.firstOptions[0]}`); else fail('model select first option is not 服务商默认…', JSON.stringify(info.firstOptions));
}
