// Scenario: 设置 → 高级 → 网页搜索. Default provider is the Grok subscription.
// A missing sign-in shows a reason and does not pretend OpenRouter is in use.
// Choosing OpenRouter keeps a native :online model.
export const id = 'settings-web-search';
export const title = 'Settings: web search provider and model';
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
  await page.locator('#vnext-settings-search').waitFor({ state: 'visible', timeout: 10000 });
  await page.waitForTimeout(400);
}

async function readSearch(page) {
  return page.evaluate(() => {
    const root = document.querySelector('#vnext-settings-search');
    const checked = root && root.querySelector('[aria-checked="true"]');
    const provider = checked ? checked.getAttribute('data-seg-value') || '' : '';
    const select = document.querySelector('#vnext-settings-search-model-select');
    const model = select ? String(select.value || '') : '';
    const reason = document.querySelector('[data-web-search-reason="not-signed-in"]');
    const reasonText = reason ? (reason.textContent || '').trim() : '';
    return { provider, model, reasonText };
  });
}

async function clickSearchProvider(page, providerId) {
  const btn = page.locator(`#vnext-settings-search [data-seg-value="${providerId}"]`).first();
  await btn.waitFor({ state: 'visible', timeout: 10000 });
  const waitSave = page.waitForResponse(
    (r) => r.url().includes('/webapp/vnext/settings/web-search') && r.request().method() === 'POST',
    { timeout: 15000 },
  ).catch(() => null);
  await btn.click();
  const res = await waitSave;
  await page.waitForTimeout(500);
  return res;
}

export async function run({ page, ok, fail, shot }) {
  await gotoSettingsAdvanced(page);
  const start = await readSearch(page);
  await shot('01-search-default');
  if (start.provider === 'xai_subscription') ok('default search provider is the Grok subscription', start.provider);
  else fail('search provider did not default to the Grok subscription', JSON.stringify(start));
  if (start.model === 'grok-4.7') ok('default subscription search model is grok-4.7', start.model);
  else fail('subscription search model is not grok-4.7', JSON.stringify(start));
  if (!start.model.includes(':online')) ok('default model is not an OpenRouter :online slug', start.model);
  else fail('default search model still uses :online', start.model);
  if (start.reasonText.includes('不会改走 OpenRouter') || start.reasonText.includes('will not switch to OpenRouter')) {
    ok('unsigned Grok subscription shows a visible reason', start.reasonText);
  } else if (!start.reasonText) {
    ok('Grok is signed in, so the unsigned reason stays hidden');
  } else {
    fail('search sign-in reason is not the expected copy', start.reasonText);
  }

  const providerRes = await clickSearchProvider(page, 'openrouter');
  const openrouter = await readSearch(page);
  await shot('02-search-openrouter');
  if (providerRes && providerRes.status() === 200) ok('web-search POST reached the server', `status ${providerRes.status()}`);
  else fail('web-search POST missing or failed', providerRes ? `status ${providerRes.status()}` : 'no request');
  if (openrouter.provider === 'openrouter') ok('OpenRouter search provider is selected', openrouter.provider);
  else fail('search provider did not switch to OpenRouter', JSON.stringify(openrouter));
  if (openrouter.model.endsWith(':online')) ok('OpenRouter search model keeps :online', openrouter.model);
  else fail('OpenRouter search model lost :online', openrouter.model);
  if (!openrouter.reasonText) ok('OpenRouter choice hides the subscription sign-in reason');
  else fail('subscription sign-in reason still shows after choosing OpenRouter', openrouter.reasonText);

  await clickSearchProvider(page, 'xai_subscription');
  const back = await readSearch(page);
  await shot('03-search-subscription');
  if (back.provider === 'xai_subscription' && back.model === 'grok-4.7') {
    ok('switching back restores the subscription model', back.model);
  } else {
    fail('subscription search did not restore grok-4.7', JSON.stringify(back));
  }
}
