export const id = 'source-nested-tree';
export const title = 'Source panel nests fields under each record and explains missing evidence numbers';
export const surfaces = ['webapp/src/vnext/AssistantResponseView.tsx', 'webapp/src/vnext/SourceExcerpt.tsx', 'webapp/src/vnext/sourceEvidenceText.ts', 'core/src/evidence.rs'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const nested = {
    tasks: [{
      title: '联系家人',
      status: 'fired',
      note: '迁移对 Aegis Runtime 没用',
      items: [{ title: '不要迁', status: 'pending' }],
    }],
  };
  const evidence = [
    { id: 'E3', tool: 'manage_tasks', label: 'Tasks and reminders', full_len: 420, excerpt: JSON.stringify(nested) },
    { id: 'E4', tool: 'web_search', label: 'Web search', full_len: 80, excerpt: '- Call family\n  Status: fired\n  Count: 1' },
    { id: 'E7', tool: 'web_search', label: 'Web search', full_len: 40, excerpt: JSON.stringify({ summary: '{not a record', count: 0 }) },
    { id: 'E9', tool: 'web_search', label: 'Web search', full_len: 60, excerpt: 'Note: keep this sentence.\nThe count is not a field.' },
  ];
  await page.route(pattern, (route) => route.fulfill({ json: {
    schema_version: 1, has_more: false, messages: [{
      role: 'aegis', timestamp: Math.floor(Date.now() / 1000),
      text: '对 Aegis Runtime 没用，不要迁。[E3] [E4] [E7] [E9]',
      evidence,
    }],
  } }));
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const reply = page.locator('article.vnext-chat-message').filter({ hasText: '不要迁' });
    const chip = reply.locator('button[data-evidence-id="E3"]');
    await chip.waitFor({ state: 'visible' });
    if (await chip.getAttribute('data-evidence-fact') === '不要迁') ok('chip names the field used in the sentence');
    else fail('chip is not tied to the sentence field', await chip.getAttribute('data-evidence-fact'));
    await chip.click();
    const source = reply.locator('.vnext-source-evidence-row[data-evidence-id="E3"]');
    await source.waitFor({ state: 'visible' });
    const nestedList = source.locator('ul.vnext-source-tree ul.vnext-source-tree');
    if (await nestedList.count() >= 1) ok('record fields sit in a nested list');
    else fail('source fields are not nested', await source.innerHTML());
    const text = await source.innerText();
    if (/联系家人/.test(text) && /不要迁/.test(text) && /已触发|Triggered/.test(text) && !/\{"tasks"|group_key/.test(text)) ok('nested facts visible without raw JSON');
    else fail('nested facts missing', text);
    const family = source.locator('li', { hasText: '联系家人' }).first();
    const childText = await family.innerText();
    if (childText.includes('不要迁') && /已触发|Triggered/.test(childText)) ok('child fact stays inside its record');
    else fail('child fact left its record', childText);
    const cited = source.locator('[data-source-fact="不要迁"].is-cited-fact');
    if (await cited.count() === 1) ok('the cited field is marked');
    else fail('cited field was not marked', await source.innerHTML());
    const gaps = await reply.locator('.vnext-evidence-gaps').innerText();
    if (gaps.includes('E1') && gaps.includes('E2') && gaps.includes('E5') && gaps.includes('E8') && !/E3/.test(gaps)) ok('missing evidence numbers are named once');
    else fail('gap note missing or lists a real source', gaps);
    if (await reply.locator('.vnext-source-evidence-row[data-evidence-id="E1"]').count() === 0) ok('a missing number is not a fake source row');
    else fail('missing number became a source row');
    const braced = await reply.locator('.vnext-source-evidence-row[data-evidence-id="E7"]').innerText();
    if (braced.includes('{not a record') && !/不完整|incomplete/.test(braced)) ok('a brace note stays text');
    else fail('brace note was treated as broken JSON', braced);
    const prose = await reply.locator('.vnext-source-evidence-row[data-evidence-id="E9"]').innerText();
    if (prose.includes('Note: keep this sentence') && await reply.locator('.vnext-source-evidence-row[data-evidence-id="E9"] ul.vnext-source-tree').count() === 0) ok('prose colons stay prose');
    else fail('prose was parsed as a source tree', prose);
    if (await reply.locator('.tone-evidence details').count() === 0) ok('nested facts are not another disclosure');
    else fail('nested source accordion');
    await source.scrollIntoViewIfNeeded();
    await shot('01-nested-source');
  } finally {
    await page.unroute(pattern);
  }
}
