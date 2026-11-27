// Owner menu on the 安琪 page: everyday rows are Settings and Runtime status.
// Pause-checks and support snapshot moved into Settings. Clear and sign-out
// stay at the bottom, separated, both red. The panel stays an overlay.
export const id = 'owner-menu-regroup';
export const title = 'Owner menu is short; pause and snapshot live in Settings';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/vnext/VNextHeaderMenu.tsx',
  'webapp/src/vnext/App.tsx',
];

const DECISION_EN = 'Aegis needs a decision. Open Work to review the approval.';
const DECISION_ZH = '安琪需要你做决定。打开「工作」查看这次批准。';

function history() {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'aegis', text: DECISION_EN, timestamp: 1_758_000_100 },
    ],
  };
}

async function revealSnapshots(route) {
  if (route.request().method() !== 'GET') {
    await route.continue();
    return;
  }
  const response = await route.fetch();
  const type = response.headers()['content-type'] || '';
  if (!type.includes('json')) {
    await route.fulfill({ response });
    return;
  }
  let json;
  try {
    json = await response.json();
  } catch {
    await route.fulfill({ response });
    return;
  }
  if (json && typeof json === 'object') json.advanced_diagnostics_available = true;
  await route.fulfill({ response, json });
}

async function menuLabels(page) {
  const items = page.locator('.vnext-header-menu-panel [role="menuitem"]');
  await items.first().waitFor({ state: 'visible', timeout: 10000 });
  const count = await items.count();
  const labels = [];
  for (let i = 0; i < count; i += 1) {
    const labelEl = items.nth(i).locator('.vnext-header-menu-item-label');
    const raw = (await labelEl.count()) ? await labelEl.innerText() : await items.nth(i).innerText();
    labels.push((raw || '').replace(/\s+/g, ' ').trim());
  }
  return labels;
}

export async function run({ page, ok, fail, shot }) {
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({ json: history() }));
  await page.route('**/webapp/vnext/settings', revealSnapshots);
  await page.route('**/webapp/vnext/status', revealSnapshots);

  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(800);
  const nav = ((await page.locator('nav.vnext-nav').innerText()) || '').replace(/\s+/g, ' ');
  const zh = /工作/.test(nav) && !/\bWork\b/.test(nav);
  const decision = page.getByText(zh ? DECISION_ZH : DECISION_EN, { exact: true });
  if (await decision.count()) ok('approval card follows the interface language', zh ? 'zh' : 'en');
  else fail('approval card did not follow the interface language', nav);
  if (zh && await page.getByText(DECISION_EN, { exact: true }).count()) {
    fail('Chinese UI still shows the English approval card');
  } else if (!zh && await page.getByText(DECISION_ZH, { exact: true }).count()) {
    fail('English UI still shows the Chinese approval card');
  } else ok('the other language is not on the approval card');

  const composer = page.locator('.vnext-chat-composer').first();
  await composer.waitFor({ state: 'visible', timeout: 15000 });
  const before = await composer.boundingBox();
  const trigger = page.locator(
    [
      '.vnext-header-menu-trigger[aria-label="账户菜单"]',
      '.vnext-header-menu-trigger[aria-label="Account menu"]',
    ].join(', '),
  ).first();
  await trigger.click();
  await page.locator('.vnext-header-menu-panel').waitFor({ state: 'visible', timeout: 5000 });
  await page.waitForTimeout(150);
  const after = await composer.boundingBox();
  const delta = before && after ? Math.abs(after.y - before.y) : 99;
  if (delta <= 2) ok('opening the menu does not push the composer', `delta ${delta}px`);
  else fail('opening the menu pushed the composer', `delta ${delta}px`);
  const panelPosition = await page.locator('.vnext-header-menu-panel').evaluate((el) => getComputedStyle(el).position);
  if (panelPosition === 'fixed') ok('menu panel is an overlay', panelPosition);
  else fail('menu panel is not an overlay', panelPosition);

  const labels = await menuLabels(page);
  const everyday = zh ? ['设置', '运行状态'] : ['Settings', 'Runtime status'];
  const session = zh ? ['清空对话', '退出登录'] : ['Clear chat', 'Sign out'];
  const hidden = zh
    ? ['暂停自动检查', '恢复自动检查', '保存支持快照', '已安装到此设备', 'Aegis 状态']
    : ['Pause automatic checks', 'Resume automatic checks', 'Save support snapshot', 'Installed on this device', 'Aegis status'];
  const install = zh ? '安装到此设备' : 'Install on this device';
  if (everyday.every((label) => labels.includes(label))) ok('everyday rows are settings and runtime status', labels.join(' | '));
  else fail('everyday rows missing', labels.join(' | '));
  if (hidden.some((label) => labels.includes(label))) fail('moved or status rows still in the menu', labels.join(' | '));
  else ok('pause, snapshot, and installed-status are not menu rows');
  const installCount = labels.filter((label) => label === install).length;
  if (installCount <= 1 && !labels.includes(zh ? '已安装到此设备' : 'Installed on this device')) {
    ok(installCount === 1 ? 'install is one action' : 'install row hidden because the app is installed');
  } else fail('install row is not a single action', labels.join(' | '));
  if (session.every((label) => labels.includes(label))) ok('clear and sign-out stay at the bottom', labels.slice(-2).join(' | '));
  else fail('clear or sign-out missing', labels.join(' | '));
  const tail = labels.slice(-session.length);
  if (tail.join('|') === session.join('|')) ok('session rows are last');
  else fail('session rows are not last', tail.join(' | '));

  const destructive = await page.locator('.vnext-header-menu-panel .vnext-header-menu-item-destructive .vnext-header-menu-item-label').allInnerTexts();
  const destructiveLabels = destructive.map((text) => text.replace(/\s+/g, ' ').trim());
  if (session.every((label) => destructiveLabels.includes(label)) && destructiveLabels.length === session.length) {
    ok('clear and sign-out are the red rows', destructiveLabels.join(' | '));
  } else fail('clear and sign-out are not both red', destructiveLabels.join(' | '));
  const separator = await page.locator('.vnext-header-menu-panel [role="separator"]').count();
  if (separator === 1) ok('session rows are visually separated');
  else fail('menu separator missing', String(separator));
  await shot('01-short-menu');

  const settingsLabel = zh ? '设置' : 'Settings';
  await page.locator('.vnext-header-menu-panel [role="menuitem"]').filter({ hasText: settingsLabel }).first().click();
  await page.locator('[data-settings-section="runtime"]').waitFor({ state: 'visible', timeout: 15000 });
  const pause = zh ? '暂停自动检查' : 'Pause automatic checks';
  const snapshot = zh ? '保存支持快照' : 'Save support snapshot';
  const runtimeText = ((await page.locator('[data-settings-section="runtime"]').innerText()) || '').replace(/\s+/g, ' ');
  if (runtimeText.includes(pause)) ok('pause automatic checks is in Settings', pause);
  else fail('pause automatic checks is not in Settings', runtimeText.slice(0, 240));
  if (runtimeText.includes(snapshot)) ok('save support snapshot is in Settings', snapshot);
  else fail('save support snapshot is not in Settings', runtimeText.slice(0, 240));
  await shot('02-settings-runtime');
}
