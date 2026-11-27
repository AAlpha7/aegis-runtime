import assert from 'node:assert/strict';

// HTTP failure is injected at the browser boundary. Rust tests separately
// exercise real SQLite statement/commit failures against these handlers.
export async function checkSettingsPersistence({page, context, origin, csrfToken}) {
  const endpoint = `${origin}/webapp/vnext/settings/paid-api-fallback`;
  const before = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
  const write = (enabled) => context.request.post(endpoint, {
    headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
    data: {schema_version: 1, idempotency_key: `persistence-${enabled}`, enabled},
  });
  try {
    for (const language of ['en', 'zh']) {
      assert.equal((await write(true)).status(), 200);
      await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
        headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
        data: {schema_version: 1, idempotency_key: `persistence-${language}`, display_language: language},
      });
      await page.goto(`${origin}/ui/?view=settings`);
      await page.locator('[data-settings-tier="advanced"] > summary').click();
      const toggle = page.locator('.vnext-settings-toggle-line')
        .filter({has: page.locator('strong', {hasText: /^(Paid API fallback|付费 API 回退)$/})})
        .locator('input[type="checkbox"]');
      assert.equal(await toggle.isChecked(), true);
      await page.route(endpoint, route => route.fulfill({status: 500, body: 'Unable to save configuration: synthetic database failure'}));
      const rejected = page.waitForResponse(response => response.url() === endpoint && response.status() === 500);
      await toggle.click();
      await rejected;
      await page.getByRole('status').filter({hasText: /failed|failure|无法|失败|Unable|could not/i}).first().waitFor();
      await toggle.waitFor({state: 'visible'});
      await page.waitForFunction(() => [...document.querySelectorAll('.vnext-settings-toggle-line')]
        .filter(el => /^(Paid API fallback|付费 API 回退)$/.test(el.querySelector('strong')?.textContent || ''))
        .every(el => !el.querySelector('input').disabled));
      assert.equal(await toggle.isChecked(), true, 'failed disable must retain the saved On state');
      assert.equal((await (await context.request.get(`${origin}/webapp/vnext/settings`)).json()).paid_api_fallback_enabled, true);
      await page.unroute(endpoint);
      const saved = page.waitForResponse(response => response.url() === endpoint && response.status() === 200);
      await toggle.click();
      await saved;
      await page.waitForFunction(() => [...document.querySelectorAll('.vnext-settings-toggle-line')]
        .filter(el => /^(Paid API fallback|付费 API 回退)$/.test(el.querySelector('strong')?.textContent || ''))
        .every(el => !el.querySelector('input').checked));
      assert.equal((await (await context.request.get(`${origin}/webapp/vnext/settings`)).json()).paid_api_fallback_enabled, false);
    }
  } finally {
    await page.unroute(endpoint);
    assert.equal((await write(before.paid_api_fallback_enabled)).status(), 200);
    await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
      headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
      data: {schema_version: 1, idempotency_key: 'persistence-restore', display_language: before.display_language || 'en'},
    });
  }
}
