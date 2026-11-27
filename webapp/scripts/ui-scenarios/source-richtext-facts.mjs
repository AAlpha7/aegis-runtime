export const id = 'source-richtext-facts';
export const title = 'Replayed replies and sources preserve measurements and alternating lists';
export const surfaces = ['webapp/src/vnext/RichText.tsx'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const facts = '24.5°C，晴\n11.5 km remaining\n2026.09.18';
  const lists = '- 保留事实 [E1]\n1. next instruction\n- final fact\n2. final instruction';
  await page.route(pattern, route => route.fulfill({ json: {
    schema_version: 1, has_more: false, messages: [{
      role: 'aegis', timestamp: Math.floor(Date.now() / 1000),
      text: `${facts}\n\n${lists}`,
      evidence: [{ id: 'E1', tool: 'web_search', label: 'Web search', full_len: 180,
        excerpt: `${facts}\n\n1. source instruction\n- source fact\n2. last source instruction` }],
    }],
  } }));
  try {
    for (const pass of ['initial', 'reload']) {
      if (pass === 'initial') await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
      else await page.reload({ waitUntil: 'domcontentloaded' });
      const reply = page.locator('article.vnext-chat-message').filter({ hasText: '24.5°C，晴' });
      const chip = reply.locator('button[data-evidence-id="E1"]');
      await chip.waitFor({ state: 'visible' });
      for (const more of await reply.locator('.vnext-answer-more:not([open]) > summary').all()) await more.click();
      const answerText = await reply.innerText();
      if (['24.5°C，晴', '11.5 km remaining', '2026.09.18', '保留事实', 'next instruction', 'final fact', 'final instruction'].every(fact => answerText.includes(fact))) ok(`${pass}: reply retains all facts`);
      else fail(`${pass}: reply lost facts`, answerText);
      await chip.click();
      const source = reply.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
      await source.waitFor({ state: 'visible' });
      const sourceText = await source.innerText();
      if (['24.5°C，晴', '11.5 km remaining', '2026.09.18', 'source instruction', 'source fact', 'last source instruction'].every(fact => sourceText.includes(fact))) ok(`${pass}: source retains all facts`);
      else fail(`${pass}: source lost facts`, sourceText);
      const items = await source.locator('li').allTextContents();
      if (JSON.stringify(items) === JSON.stringify(['source instruction', 'source fact', 'last source instruction'])) ok(`${pass}: alternating list order preserved`);
      else fail(`${pass}: source list order changed`, JSON.stringify(items));
      if (await source.getAttribute('id') === await chip.getAttribute('aria-controls')) ok(`${pass}: retained citation targets its source`);
      else fail(`${pass}: source citation target mismatch`);
      await source.scrollIntoViewIfNeeded();
      await shot(`${pass}-source-facts`);
    }
  } finally { await page.unroute(pattern); }
}
