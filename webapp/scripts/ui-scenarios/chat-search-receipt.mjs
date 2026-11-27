import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'chat-search-receipt';
export const title = 'Search receipt, one research row, and a stop control';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/SearchReceiptCard.tsx',
  'webapp/src/vnext/searchReceipt.ts',
];

const RECEIPT = [
  '答案不在这些来源里。',
  '',
  '搜索收据',
  '查了：zzzz-unique-topic',
  '1. Example — short excerpt',
  '   https://example.com/a',
  '@@search-receipt',
  JSON.stringify({
    queries: ['zzzz-unique-topic'],
    sources: [{
      query: 'zzzz-unique-topic',
      title: 'Example',
      url: 'https://example.com/a',
      excerpt: 'short excerpt',
    }],
  }),
  '@@end-search-receipt',
].join('\n');

function history() {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '查一下 zzzz-unique-topic', timestamp: 1_758_000_000, turn_id: 'search-turn' },
      { role: 'aegis', text: '正在核对：zzzz-unique-topic', timestamp: 1_758_000_010, turn_id: 'search-turn' },
      { role: 'aegis', text: RECEIPT, timestamp: 1_758_000_020, turn_id: 'search-turn' },
    ],
    approvals: [],
  };
}

function workPayload() {
  const payload = injectedWorkPayload({ generated_at: '2026-09-26T00:00:00Z', sources: [] });
  const lane = payload.lanes.find((entry) => entry.id === 'follow_up');
  lane.items = [{
    ...lane.items[0],
    id: 'research:1',
    title: '调研：zzzz-unique-topic',
    summary: '结论：答案不在已看过的来源里。',
  }];
  payload.counts.follow_up = 1;
  return payload;
}

export async function run({ page, ok, fail, shot }) {
  let stopped = false;
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({ json: history() }));
  await page.route('**/webapp/vnext/work**', (route) => route.fulfill({ json: workPayload() }));
  await page.route('**/webapp/vnext/research/stop', (route) => {
    stopped = true;
    return route.fulfill({ json: { status: 'ok' } });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const receipt = page.getByTestId('search-receipt');
    await receipt.waitFor({ state: 'visible', timeout: 30000 });
    const receiptText = await receipt.innerText();
    if (receiptText.includes('zzzz-unique-topic') && receiptText.includes('https://example.com/a')) {
      ok('receipt shows the query and the source');
    } else fail('receipt missing query or source', receiptText);
    if (receiptText.includes('short excerpt') && !receiptText.includes('x'.repeat(400))) {
      ok('receipt excerpt is short');
    } else fail('receipt looks like a full page', receiptText.slice(0, 200));
    if ((await page.locator('[data-testid="search-receipt"]').count()) === 1) {
      ok('one receipt on the thread');
    } else fail('receipt stack');

    const stop = page.getByTestId('research-stop');
    await stop.waitFor({ state: 'visible', timeout: 15000 });
    await stop.click();
    await page.getByTestId('research-stopped').waitFor({ state: 'visible', timeout: 15000 });
    if (stopped) ok('stop control tells the server to stop research');
    else fail('stop control did not call the server');
    await shot('01-receipt-and-stop');

    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 15000 });
    await work.click();
    const researchRow = page.getByRole('button', { name: /调研：zzzz-unique-topic/ }).filter({ visible: true });
    await researchRow.first().waitFor({ state: 'visible', timeout: 15000 });
    const lookupRows = await page.locator('.vnext-item-title').filter({ hasText: 'web_search' }).filter({ visible: true }).count();
    if ((await researchRow.count()) === 1 && lookupRows === 0) {
      ok('work records the research job, not each lookup');
    } else fail('work page gained a row per search', `rows=${await researchRow.count()} lookups=${lookupRows}`);
    await shot('02-one-research-job');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute('**/webapp/vnext/chat-history*');
    await page.unroute('**/webapp/vnext/work**');
    await page.unroute('**/webapp/vnext/research/stop');
  }
}
