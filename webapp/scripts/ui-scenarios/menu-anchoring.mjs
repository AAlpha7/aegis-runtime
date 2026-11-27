// Shared menu placement (owner 2026-09-12 IMG "菜单没对齐"): the panel is
// portaled to body and every entrance keyframe ends in `transform: none`, so a
// view that keeps an identity transform can no longer re-anchor the
// position:fixed panel. Asserts the panel's right edge sits on the trigger's
// right edge on the home (gear) menu and the settings menu, that the composer
// carries a direct size toggle instead of the old "…" menu, that Attach exists
// exactly once, and that Aegis replies show their reply duration.
export const id = 'menu-anchoring';
export const title = 'Menus anchor to their trigger; composer toggle; reply duration';
export const surfaces = [
  'webapp/src/vnext/VNextHeaderMenu.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/styles.css',
  'webapp/src/vnext/design/motion.css',
];

const TRIGGER = [
  '.vnext-header-menu-trigger[aria-label="打开菜单"]',
  '.vnext-header-menu-trigger[aria-label="Open menu"]',
  '.vnext-header-menu-trigger[aria-label="账户菜单"]',
  '.vnext-header-menu-trigger[aria-label="Account menu"]',
].join(', ');

async function measureMenu(page) {
  return page.evaluate((selector) => {
    const trigger = document.querySelector(selector);
    const panel = document.querySelector('.vnext-header-menu-panel');
    if (!trigger || !panel) return null;
    const t = trigger.getBoundingClientRect();
    const p = panel.getBoundingClientRect();
    const inBody = panel.parentElement?.parentElement === document.body;
    return {
      rightDelta: Math.round(p.right - t.right),
      belowTrigger: p.top >= t.bottom,
      onScreen: p.left >= 0 && p.right <= window.innerWidth + 1 && p.bottom <= window.innerHeight + 1,
      inBody,
    };
  }, TRIGGER);
}

async function checkMenu(page, ok, fail, label) {
  const trigger = page.locator(TRIGGER).first();
  await trigger.waitFor({ state: 'visible', timeout: 15000 });
  await trigger.click();
  await page.locator('.vnext-header-menu-panel').waitFor({ state: 'visible', timeout: 5000 });
  await page.waitForTimeout(150);
  const m = await measureMenu(page);
  if (!m) { fail(`${label}: menu panel not measurable`); return; }
  if (Math.abs(m.rightDelta) <= 3 && m.belowTrigger && m.onScreen) {
    ok(`${label}: panel anchored to its trigger`, `right edge delta ${m.rightDelta}px, portaled=${m.inBody}`);
  } else {
    fail(`${label}: panel drifted from its trigger`, JSON.stringify(m));
  }
  await page.keyboard.press('Escape');
  await page.waitForTimeout(100);
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(900); // let the entrance animations finish
  await checkMenu(page, ok, fail, 'home');
  await shot('01-home-menu');

  const sizeToggle = page.locator('.vnext-chat-size-button');
  const moreButtons = await page.locator('.vnext-chat-more-button').count();
  if ((await sizeToggle.count()) === 1 && moreButtons === 0) ok('composer has a direct size toggle and no "…" menu');
  else fail('composer "…" menu still present or toggle missing', `toggle=${await sizeToggle.count()} more=${moreButtons}`);
  const attachCount = await page.locator('[aria-label="添加附件"], [aria-label="Attach file"]').count();
  if (attachCount === 1) ok('Attach appears exactly once (paperclip)');
  else fail('Attach duplicated or missing', String(attachCount));
  const beforeMode = await page.locator('.vnext-chat-composer').getAttribute('class');
  await sizeToggle.click();
  await page.waitForTimeout(200);
  const afterMode = await page.locator('.vnext-chat-composer').getAttribute('class');
  if (beforeMode !== afterMode && /mode-half/.test(afterMode || '')) ok('size toggle expands the composer', afterMode);
  else fail('size toggle did not change the composer mode', `${beforeMode} → ${afterMode}`);
  await shot('02-composer-expanded');
  // While the field has focus the composer is "composing" and hides its
  // secondary controls (pre-existing behaviour); blur, then collapse.
  await page.locator('.vnext-chat-composer textarea').evaluate((el) => el.blur());
  await page.waitForTimeout(150);
  await sizeToggle.click();
  await page.waitForTimeout(150);
  const collapsed = await page.locator('.vnext-chat-composer').getAttribute('class');
  if (/mode-line/.test(collapsed || '')) ok('size toggle collapses the composer again', collapsed);
  else fail('size toggle did not collapse the composer', collapsed || '');

  const latency = page.locator('.vnext-chat-stream .vnext-chat-latency');
  const latencyCount = await latency.count();
  if (latencyCount > 0) {
    const text = (await latency.first().innerText()).trim();
    if (/\d/.test(text)) ok('Aegis replies show their reply duration', `${latencyCount} shown, e.g. "${text}"`);
    else fail('reply duration chip has no number', text);
  } else {
    fail('no reply duration on any Aegis reply', '0 chips on the seeded history');
  }

  // Settings is reached through the menu itself (no direct route).
  await page.locator(TRIGGER).first().click();
  const items = page.locator('[role="menuitem"]');
  await items.first().waitFor({ state: 'visible', timeout: 10000 });
  const count = await items.count();
  for (let i = 0; i < count; i += 1) {
    const text = ((await items.nth(i).innerText()) || '').trim();
    if (text === '设置' || text === 'Settings') { await items.nth(i).click(); break; }
  }
  await page.locator('.vnext-app-settings').waitFor({ state: 'visible', timeout: 15000 });
  await page.waitForTimeout(900);
  await checkMenu(page, ok, fail, 'settings');
  await shot('03-settings-menu');
}
