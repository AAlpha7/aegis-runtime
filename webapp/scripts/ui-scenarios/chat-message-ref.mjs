export const id = 'chat-message-ref';
export const title = 'Each owner and Aegis message shows a stable cite that copies';
export const surfaces = ['webapp/src/vnext/ChatHomeView.tsx', 'webapp/src/vnext/messageRef.ts', 'webapp/src/vnext/design/surfaces/chat-home.css'];

export async function run({ page, context, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const now = Math.floor(Date.now() / 1000);
  const turnId = 'cite-turn-1';
  const history = {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '这条要迁吗', timestamp: now - 40, turn_id: turnId },
      {
        role: 'aegis',
        text: '结论：先别迁。 [E1]',
        timestamp: now - 10,
        turn_id: turnId,
        latency_ms: 174000,
        evidence: [{ id: 'E1', tool: 'web_search', label: 'Web search', excerpt: '公开预览说明', full_len: 6 }],
      },
    ],
  };
  await page.route(pattern, (route) => route.fulfill({ json: history }));
  try {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => {});
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const cites = page.locator('button.vnext-chat-ref');
    await cites.first().waitFor({ state: 'visible', timeout: 30000 });
    const before = await cites.evaluateAll((els) => els.map((el) => el.getAttribute('data-message-ref')));
    if (before.length === 2 && before.every((ref) => /^[0-9abcdefghjkmnpqrstvwxyz]{6}$/.test(ref)) && before[0] !== before[1]) {
      ok('owner and Aegis each show a distinct cite');
    } else fail('cite pair missing or not distinct', JSON.stringify(before));
    await page.evaluate(() => {
      window.__copied = [];
      const writeText = async (text) => { window.__copied.push(text); };
      if (!navigator.clipboard) {
        Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
      } else {
        navigator.clipboard.writeText = writeText;
      }
    });
    const reply = cites.nth(1);
    await reply.click();
    const copiedLabel = (await reply.innerText()).trim();
    const copiedToken = await page.evaluate(() => window.__copied[0]);
    if ((copiedLabel === '已复制' || copiedLabel === 'Copied') && copiedToken === `#${before[1]}`) {
      ok('cite copies its #token and confirms');
    } else fail('copy did not confirm', `${copiedLabel} / ${copiedToken}`);
    await reply.scrollIntoViewIfNeeded();
    await shot('01-cite-copied');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await cites.first().waitFor({ state: 'visible', timeout: 30000 });
    const after = await cites.evaluateAll((els) => els.map((el) => el.getAttribute('data-message-ref')));
    if (after.join() === before.join()) ok('reload keeps both cites');
    else fail('cites changed across reload', JSON.stringify({ before, after }));
    await shot('02-cite-stable');
  } finally {
    await page.unroute(pattern);
  }
}
