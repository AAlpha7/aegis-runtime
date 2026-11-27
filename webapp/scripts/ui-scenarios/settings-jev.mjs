import assert from 'node:assert/strict';

export const id = 'settings-jev';
export const title = 'Optional Jev: consent, save and cancel';
export const surfaces = ['webapp/src/vnext/JevSettings.tsx', 'webapp/src/vnext/SettingsView.tsx'];

// This reusable visual scenario uses a synthetic settings endpoint. The real
// auth/router/encrypted persistence path is covered in test-remote-access.mjs.
export async function run({page, ok, shot}) {
  let status = {enabled: false, key_present: false, saved_key: false, source: 'settings', key_source: 'none', verified: false};
  let writes = 0;
  const routePattern = '**/webapp/vnext/settings/jev';
  await page.route(routePattern, async route => {
    if (route.request().method() === 'POST') {
      const update = route.request().postDataJSON();
      assert.equal(update.consent, true);
      assert.equal(update.enabled, true);
      writes += 1;
      status = {...status, enabled: true, key_present: true, saved_key: true, key_source: 'settings', verified: true};
    }
    await route.fulfill({json: status});
  });
  try {
    await page.goto('/ui/?view=settings');
    await page.locator('[data-settings-tier="advanced"] > summary').click();
    const card = page.locator('.vnext-jev-card');
    const toggle = card.getByRole('switch');
    const save = card.getByRole('button', {name: /^(Save|保存)$/});
    await toggle.waitFor();
    assert.equal(await toggle.isChecked(), false);
    await toggle.check();
    await card.getByLabel('OpenRouter API key', {exact: true}).fill('sk-or-synthetic-ui-scenario-key');
    assert.equal(await save.isEnabled(), false);
    assert.equal(writes, 0);
    await card.getByRole('checkbox').check();
    await save.click();
    await card.getByRole('status').filter({hasText: /Saved\.|已保存/}).waitFor();
    assert.equal(writes, 1);
    assert.equal(await card.getByLabel('OpenRouter API key', {exact: true}).inputValue(), '');
    // Legacy saved keys can be verified without changing or re-entering them.
    status = {...status, verified: false};
    await card.getByRole('button', {name: /^(Reload|重新加载)$/}).click();
    await card.locator('.vnext-jev-state').filter({hasText: /unverified|未验证/}).waitFor();
    assert.equal(await save.isEnabled(), false);
    await card.getByRole('checkbox').check();
    assert.equal(await save.isEnabled(), true);
    await save.click();
    await card.getByRole('status').filter({hasText: /Saved\.|已保存/}).waitFor();
    assert.equal(writes, 2);
    await toggle.uncheck();
    await card.getByRole('button', {name: /^(Cancel|取消)$/}).click();
    assert.equal(await toggle.isChecked(), true);
    assert.equal(writes, 2);
    await card.scrollIntoViewIfNeeded();
    await shot('configured-verified-fixture');
    ok('Jev requires consent; Save clears the key; legacy keys can be verified; Cancel does not write');
  } finally {
    await page.unroute(routePattern);
  }
}
