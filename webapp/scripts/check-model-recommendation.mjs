import assert from 'node:assert/strict';
import {mkdirSync} from 'node:fs';
import {resolve, join} from 'node:path';

// Actual Settings UI with deterministic catalog/failed-probe responses. No key
// or paid calls; authenticated persisted settings are read back separately.
export async function checkModelRecommendation({page, context, origin, csrfToken}) {
  const model = 'deepseek/deepseek-v4.1-flash';
  const shots = resolve(import.meta.dirname, '../../target/model-recommendation-ui');
  mkdirSync(shots, {recursive: true});
  const before = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
  let available = true;
  let chatWrites = 0;
  let backupWrites = 0;
  await page.route('**/webapp/vnext/model-catalog', route => route.fulfill({json: {
    schema_version: 1,
    recommended_api_model: available ? model : null,
    providers: [{id: 'openrouter', connected: true, source: 'test', default_model: '',
      models: available ? [{id: model, display_name: 'DeepSeek V4.1 Flash'}] : []}],
    current: {provider: before.llm_provider, model: before.llm_model, effective_model: before.llm_model},
    fallback: {provider: null, model: ''},
  }}));
  const reject = route => route.fulfill({status: 422, body: 'Validation failed: synthetic unavailable service. Not saved.'});
  await page.route('**/webapp/vnext/settings/llm-provider', route => {
    chatWrites++;
    assert.equal(route.request().postDataJSON().llm_model, model);
    assert.equal(route.request().postDataJSON().llm_provider, 'openrouter');
    return reject(route);
  });
  await page.route('**/webapp/vnext/settings/fallback-model', route => {
    backupWrites++;
    assert.equal(route.request().postDataJSON().model, model);
    return reject(route);
  });
  try {
    for (const language of ['en', 'zh']) {
      await page.setViewportSize({width: 320, height: 844});
      await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
        headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
        data: {schema_version: 1, display_language: language, idempotency_key: `recommend-${language}`},
      });
      await page.goto(`${origin}/ui/?view=settings`);
      await page.locator('[data-settings-tier="advanced"] > summary').click();
      const backup = page.getByRole('button', {name: /^(Verify and enable recommended backup|验证并启用推荐备用)/});
      await backup.waitFor();
      await backup.scrollIntoViewIfNeeded();
      assert(await backup.locator('.vnext-btn-label').evaluate(el => el.scrollWidth <= el.clientWidth), 'paid-backup label must not be truncated');
      await page.locator('.vnext-fallback-controls').screenshot({path: join(shots, `${language}-backup.png`)});
      assert.equal(chatWrites, language === 'en' ? 0 : 1);
      assert.equal(backupWrites, language === 'en' ? 0 : 1, 'render must not enable backup');
      await page.locator('.vnext-settings-provider-toggle [data-seg-value="openrouter"]').click();
      await page.getByRole('button', {name: /^(Use recommended model|使用推荐模型)/}).click();
      assert.equal(await page.locator('#vnext-settings-model-select').inputValue(), model);
      assert.equal(chatWrites, language === 'en' ? 0 : 1, 'recommendation is a draft until save');
      await page.getByRole('button', {name: /^(Verify and save|验证并保存)$/}).click();
      await page.getByRole('status').filter({hasText: 'synthetic unavailable service'}).waitFor();
      await backup.click();
      await page.getByRole('status').filter({hasText: 'synthetic unavailable service'}).last().waitFor();
      await page.waitForFunction(() => !document.querySelector('.vnext-fallback-controls button')?.disabled);
      const after = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
      for (const key of ['llm_provider', 'llm_model', 'paid_api_fallback_enabled']) {
        assert.equal(after[key], before[key], `failed probe changed ${key}`);
      }
      assert.match(await page.locator('.vnext-fallback-summary').textContent(), /No backup|没有备用模型/);
    }
    assert.equal(chatWrites, 2);
    assert.equal(backupWrites, 2);
    available = false;
    await page.reload();
    await page.locator('[data-settings-tier="advanced"] > summary').click();
    await page.locator('.vnext-settings-provider-toggle [data-seg-value="openrouter"]').click();
    await page.locator('#vnext-settings-model-select:disabled').waitFor();
    assert.equal(await page.getByRole('button', {name: /^(Use recommended model|使用推荐模型|Verify and enable recommended backup|验证并启用推荐备用)/}).count(), 0);
  } finally {
    await page.setViewportSize({width: 1280, height: 900});
    for (const endpoint of ['model-catalog', 'settings/llm-provider', 'settings/fallback-model']) {
      await page.unroute(`**/webapp/vnext/${endpoint}`);
    }
    await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
      headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
      data: {schema_version: 1, display_language: 'en', idempotency_key: 'recommend-restore-en'},
    });
    await page.goto(`${origin}/ui/?view=settings`);
    await page.locator('[data-settings-tier="advanced"] > summary').click();
  }
}
