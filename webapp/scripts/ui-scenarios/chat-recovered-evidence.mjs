export const id = 'chat-recovered-evidence';
export const title = 'Recovered answer upgrades in place and shows each source once';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/chatHistory.ts', 'webapp/src/vnext/AssistantResponseView.tsx', 'core/src/evidence.rs', 'webapp/src/vnext/adapters.ts'];

export async function run({ page, ok, fail, shot }) {
  const historyPattern = '**/webapp/vnext/chat-history*';
  const commandPattern = '**/webapp/vnext/command';
  const question = '明天天气如何';
  const raw = '明天 Example City 晴到少云，约 11–24°C。[E1]';
  const excerpt = 'Example City 晴到少云，约 11–24°C。来源：https://example.com/weather';
  const evidence = [{id: 'E1', tool: 'web_search', label: 'Web search', excerpt, full_len: excerpt.length}];
  let startedAt = 0;
  let finished = false;
  let release;
  const ready = new Promise((resolve) => { release = resolve; });
  await page.route(historyPattern, (route) => route.fulfill({json: {
    schema_version: 1, has_more: false, messages: startedAt ? [
      {role: 'owner', text: question, timestamp: startedAt},
      {role: 'aegis', text: raw, timestamp: startedAt + 1, latency_ms: 33000, ...(finished ? {evidence} : {})},
    ] : [],
  }}));
  await page.route(commandPattern, async (route) => {
    startedAt = Math.floor(Date.now() / 1000);
    await ready;
    finished = true;
    await route.fulfill({json: {
      schema_version: 1, generated_at: new Date().toISOString(), outcome: 'answered', owner_message: raw, assistant_response: {
        schema_version: 1, response_kind: 'answer', visible_answer: raw.replace('。[E1]', '。 [E1]'),
        latency_ms: 33000, receipt_expectation: '', citations: ['tool:web_search'], warnings: [], evidence,
        source_evidence: [{kind: 'tool', label: 'Web search', summary: 'Example City 晴到少云…', detail: 'Current evidence:\n- Example City 晴到少云…'}],
      }, work_item: null,
    }});
  });
  try {
    await page.goto('/ui/', {waitUntil: 'domcontentloaded'});
    const input = page.locator('textarea[aria-label]').first();
    await input.waitFor({state: 'visible'});
    await input.fill(question);
    await input.press('Enter');
    const replies = page.locator('article').filter({hasText: '明天 Example City'});
    await replies.first().waitFor({state: 'visible', timeout: 20000});
    if (await replies.count() === 1 && await page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]').count() === 0) ok('history restores one plain answer before final response');
    else fail('expected a single recovered answer before completion');
    await shot('01-recovered');
    release();
    const source = page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
    await source.waitFor({state: 'attached'});
    if (await replies.count() === 1) ok('final response upgrades the same bubble');
    else fail('duplicate final bubble');
    if (await replies.locator('[data-latency-ms]').getAttribute('data-latency-ms') === '33000') ok('backend reply duration retained after completion');
    else fail('backend reply duration lost');
    const outer = source.locator('xpath=ancestor::details').first();
    await outer.locator(':scope > summary').click();
    await source.waitFor({state: 'visible'});
    if (await replies.evaluateAll(els => els.every(el => !el.classList.contains('show-actions')))) ok('source disclosure does not toggle bubble reply controls');
    else fail('source disclosure also triggered the bubble action');
    if (await page.locator('.vnext-source-evidence-row').count() === 1) ok('one E1 source row without repeated tool summary');
    else fail('duplicate source rows');
    if ((await source.innerText()).includes('https://example.com/weather')) ok('source URL retained');
    else fail('source URL missing');
    const activate = await page.evaluate(() => navigator.maxTouchPoints > 0) ? 'tap' : 'click';
    await outer.locator(':scope > summary')[activate]();
    await source.waitFor({state: 'hidden'});
    await input.focus();
    await outer.locator(':scope > summary')[activate]();
    await source.waitFor({state: 'visible'});
    ok('first pointer activation opens the source while the composer loses focus');
    // Reproduce touch browsers that dispatch compatibility focus/click in a
    // later task than pointerup. The source must not move before that click.
    const summary = outer.locator(':scope > summary');
    await summary[activate]();
    await source.waitFor({state: 'hidden'});
    await input.focus();
    await page.waitForFunction(() => document.querySelector('.vnext-chat-composer')?.classList.contains('is-composing'));
    await summary.scrollIntoViewIfNeeded();
    const beforeDelayedFocus = await summary.boundingBox();
    await summary.dispatchEvent('pointerdown', {pointerType: 'touch'});
    await summary.dispatchEvent('pointerup', {pointerType: 'touch'});
    await page.waitForTimeout(30);
    await summary.evaluate(el => el.focus());
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const afterDelayedFocus = await summary.boundingBox();
    const typingLayoutHeld = await page.locator('.vnext-chat-composer').evaluate(el => el.classList.contains('is-composing'));
    if (typingLayoutHeld && beforeDelayedFocus && afterDelayedFocus && Math.abs(beforeDelayedFocus.y - afterDelayedFocus.y) <= 1) ok('delayed touch focus keeps the typing layout and source in place until click');
    else fail('delayed touch focus released the typing layout or moved the source before click', JSON.stringify({typingLayoutHeld, beforeDelayedFocus, afterDelayedFocus}));
    await summary.evaluate(el => el.click());
    await source.waitFor({state: 'visible'});
    ok('delayed touch click opens the source');
    await input.focus();
    await input.dispatchEvent('pointerdown', {pointerType: 'touch'});
    await input.dispatchEvent('pointerup', {pointerType: 'touch'});
    await input.evaluate(el => el.blur());
    await page.waitForFunction(() => !document.querySelector('.vnext-chat-composer')?.classList.contains('is-composing'), undefined, {timeout: 2000});
    ok('gesture without a click still restores the composer layout');
    await source.scrollIntoViewIfNeeded();
    await shot('02-final-one-source');
    await page.reload({waitUntil: 'domcontentloaded'});
    await page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]').waitFor({state: 'attached'});
    if (await replies.count() === 1) ok('reload preserves a single answer with evidence');
    else fail('reload restored duplicates');
    if (await page.locator('.vnext-source-evidence-row').count() === 1) ok('reload does not add a translated tool-label duplicate');
    else fail('reload added redundant source records');
    await shot('03-after-reload');
  } finally {
    release();
    await page.unroute(historyPattern);
    await page.unroute(commandPattern);
  }
}
