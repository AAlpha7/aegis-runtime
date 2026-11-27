export const id = 'source-prototype-fields';
export const title = 'Historical sources preserve prototype-shaped fields and sibling facts';
export const surfaces = ['webapp/src/vnext/sourceEvidenceText.ts'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const excerpt = '{"status":"constructor","error":"toString","hint":"__proto__","__proto__":"source fact","count":0,"available":false,"content":[{"text":"{\\"summary\\":\\"Nested retained fact\\"}"}]}';
  await page.route(pattern, route => route.fulfill({ json: {
    schema_version: 1, has_more: false, messages: [{
      role: 'aegis', timestamp: Math.floor(Date.now() / 1000),
      text: '历史来源字段检查 [E1]',
      evidence: [{ id: 'E1', tool: 'web_search', label: 'Web search', full_len: excerpt.length, excerpt }],
    }],
  } }));
  try {
    for (const pass of ['initial', 'reload']) {
      if (pass === 'initial') await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
      else await page.reload({ waitUntil: 'domcontentloaded' });
      const reply = page.locator('article.vnext-chat-message').filter({ hasText: '历史来源字段检查' });
      const chip = reply.locator('button[data-evidence-id="E1"]');
      await chip.waitFor({ state: 'visible' });
      await chip.click();
      const source = reply.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
      await source.waitFor({ state: 'visible' });
      const text = await source.innerText();
      for (const fact of ['constructor', 'toString', '__proto__', 'proto : source fact', 'Available: false', 'Nested retained fact']) {
        if (text.replace(/\s+/g, ' ').includes(fact)) ok(`${pass}: retains ${fact}`);
        else fail(`${pass}: missing ${fact}`, text);
      }
      if (/(?:Count|条目数): 0/.test(text)) ok(`${pass}: zero count retained`);
      else fail(`${pass}: zero count missing`, text);
      if (!/undefined|\[object Object\]|native code/.test(text)) ok(`${pass}: no inherited dictionary value`);
      else fail(`${pass}: inherited dictionary value`, text);
      if (await source.getAttribute('id') === await chip.getAttribute('aria-controls')) ok(`${pass}: citation scoped to its source`);
      else fail(`${pass}: citation scope mismatch`);
      await source.scrollIntoViewIfNeeded();
      await shot(`${pass}-prototype-fields`);
    }
  } finally { await page.unroute(pattern); }
}
