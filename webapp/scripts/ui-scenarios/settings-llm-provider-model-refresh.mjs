// Scenario: 设置 → 高级 → AI 与花费 — switching the provider segmented
// control must reset the model <select> to that provider's default and drop
// a stale gpt default label from the previous provider.
export const id = 'settings-llm-provider-model-refresh';
export const title = 'Settings: AI provider switch refreshes model default';
export const surfaces = [
  'webapp/src/vnext/SettingsView.tsx',
  'core/src/admin/settings_routes.rs',
];

async function gotoSettingsAdvanced(page) {
  await page.goto('/ui/?view=settings', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.locator('.vnext-settings-hero, [data-settings-section="account"]').first().waitFor({ state: 'visible', timeout: 15000 });
  await page.waitForTimeout(400);
  await page.evaluate(() => {
    const advanced = document.querySelector('details.vnext-settings-tier.advanced');
    if (advanced) advanced.open = true;
  });
  await page.locator('#vnext-settings-provider-select').waitFor({ state: 'visible', timeout: 10000 });
  await page.waitForTimeout(400);
}

async function readModelSelect(page) {
  return page.evaluate(() => {
    const select = document.querySelector('#vnext-settings-model-select');
    const first = select && select.options[0] ? (select.options[0].textContent || '').trim() : '';
    const value = select ? String(select.value || '') : '';
    const optionIds = select ? Array.from(select.options).map((opt) => opt.value).filter(Boolean) : [];
    const checked = document.querySelector('.vnext-settings-provider-toggle [aria-checked="true"]');
    const provider = checked ? checked.getAttribute('data-seg-value') || '' : '';
    return { first, value, optionIds, provider };
  });
}

async function clickProvider(page, providerId) {
  const btn = page.locator(`.vnext-settings-provider-toggle [data-seg-value="${providerId}"]`).first();
  await btn.waitFor({ state: 'visible', timeout: 10000 });
  const waitCatalog = page.waitForResponse(
    (r) => r.url().includes('/webapp/vnext/model-catalog') && r.request().method() === 'GET',
    { timeout: 15000 },
  ).catch(() => null);
  await btn.click();
  await waitCatalog;
  const waitProvider = page.waitForResponse(
    (r) => r.url().includes('/webapp/vnext/settings/llm-provider') && r.request().method() === 'POST',
    { timeout: 15000 },
  ).catch(() => null);
  await page.getByRole('button', {name: /^(Verify and save|验证并保存)$/}).click();
  const providerRes = await waitProvider;
  await page.waitForTimeout(600);
  return providerRes;
}

export async function run({ page, ok, fail, shot }) {
  await gotoSettingsAdvanced(page);
  const start = await readModelSelect(page);
  if (start.provider === 'xai_subscription') {
    await clickProvider(page, 'openrouter');
    await page.waitForTimeout(400);
  }
  const before = await readModelSelect(page);
  await shot('01-before-switch');
  if (before.first.startsWith('服务商默认') || before.first.toLowerCase().includes('default')) {
    ok('model select shows a provider-default option', before.first);
  } else {
    fail('model select first option is not 服务商默认', JSON.stringify(before));
  }

  const providerRes = await clickProvider(page, 'xai_subscription');
  const after = await readModelSelect(page);
  await shot('02-after-grok');

  if (providerRes && providerRes.status() === 200) {
    ok('llm-provider POST reached the server', `status ${providerRes.status()}`);
  } else {
    fail('llm-provider POST missing or failed', providerRes ? `status ${providerRes.status()}` : 'no request');
  }
  if (after.provider === 'xai_subscription') ok('Grok segmented control is selected', after.provider);
  else fail('provider did not switch to Grok', JSON.stringify(after));
  if (after.value === '') ok('explicit llm_model cleared to provider default', after.value);
  else fail('model select kept a stale explicit id', JSON.stringify(after));
  if (after.first.startsWith('服务商默认') || after.first.toLowerCase().includes('default')) {
    ok('default option label still uses 服务商默认', after.first);
  } else {
    fail('default option label lost 服务商默认', after.first);
  }
  if (!/gpt-/i.test(after.first)) ok('default option is not a leftover gpt id', after.first);
  else fail('default option still names a gpt model after switching to Grok', after.first);
  if (after.optionIds.every((id) => !id.startsWith('openai/') && !id.startsWith('gpt-'))) {
    ok('model options are not leftover OpenRouter gpt ids', after.optionIds.join(','));
  } else {
    fail('model options still include gpt ids after Grok switch', JSON.stringify(after.optionIds));
  }
}
