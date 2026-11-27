import assert from 'node:assert/strict';

// Real application rendering with deterministic transport frames; no model call.
export async function checkProgressLanguage({ page, context, origin, csrfToken }) {
  for (const locale of ['en', 'zh']) {
    const changed = await context.request.post(`${origin}/webapp/vnext/settings/display-language`, {
      headers: {Origin: origin, 'x-aegis-csrf': csrfToken},
      data: {schema_version: 1, display_language: locale, idempotency_key: `progress-${locale}`},
    });
    assert.equal(changed.status(), 200);
    const settings = await (await context.request.get(`${origin}/webapp/vnext/settings`)).json();
    assert(!settings.connected_services.some(service => /hercules/i.test(service.label)));
    await page.goto(`${origin}/ui/?view=settings`);
    await page.locator('[data-settings-section="language"]').waitFor();
    assert.doesNotMatch(await page.locator('body').innerText(), /Hercules/i);
    await page.goto(`${origin}/ui/`);
    const input = page.locator('[data-testid="aegis-chat-full"] textarea').first();
    await input.waitFor();

    let finish;
    const pendingCommand = new Promise(resolve => { finish = resolve; });
    await page.route('**/webapp/vnext/command', async route => {
      await pendingCommand;
      await route.fulfill({json: {
        schema_version: 1, generated_at: new Date().toISOString(), outcome: 'answered', owner_message: 'test complete',
        assistant_response: {schema_version: 1, response_kind: 'answer', visible_answer: 'test complete', citations: [], warnings: []},
      }});
    });
    await page.route('**/webapp/vnext/turns/*/stream-token', route => route.fulfill({json: {
      stream_token: 'local-test-only', expires_at: 9999999999, expires_in: 60,
    }}));
    await page.route('**/webapp/vnext/turns/*/events?*', route => {
      const frames = [
        {type: 'tool_start', tool_name: 'web_search'},
        {type: 'action_phase', phase: 'calling_executor', detail: 'calling Codex'},
        {type: 'owner_notice', kind: 'system', text: '[Executor Codex] Read file (in_progress)'},
        {type: 'tool_done', tool_name: 'web_search'},
        {type: 'done'},
      ];
      return route.fulfill({contentType: 'text/event-stream', body: frames.map(frame =>
        `event: ${frame.type}\ndata: ${JSON.stringify(frame)}\n\n`).join('')});
    });
    try {
      await input.fill(locale === 'en' ? 'Check progress language' : '检查进度语言');
      await input.press('Enter');
      const working = page.locator('.vnext-chat-working-text').last();
      await working.filter({hasText: locale === 'en' ? 'Preparing the reply' : '正在整理回复'}).waitFor({timeout: 15000});
      const text = await working.innerText();
      if (locale === 'en') {
        assert.match(text, /Searching/);
        assert.match(text, /Calling Codex/);
        assert.match(text, /Codex · Working/);
        assert.doesNotMatch(text, /[\u4e00-\u9fff]/);
      } else {
        assert.match(text, /正在搜索/);
        assert.match(text, /正在调用 Codex/);
        assert.match(text, /Codex · 正在执行/);
        assert.doesNotMatch(text, /Searching|Calling|Working|Preparing|Executor|Read file/);
      }
      assert.doesNotMatch(text, /Jev|Hercules/);
    } finally {
      finish();
      await page.locator('.vnext-chat-working-text').waitFor({state: 'hidden', timeout: 15000});
      await page.unroute('**/webapp/vnext/command');
      await page.unroute('**/webapp/vnext/turns/*/stream-token');
      await page.unroute('**/webapp/vnext/turns/*/events?*');
    }
  }
}
