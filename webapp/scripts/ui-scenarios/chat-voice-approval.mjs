// Duplex voice: one search card above the speaking line. Tap 批准, say
// 「批准。」, or tap 先不要. The raw egress fields stay off the card.
export const id = 'chat-voice-approval';
export const title = 'Voice search approval is one card, and 批准 continues it';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/CeoVoiceAgent.tsx',
  'webapp/src/vnext/App.tsx',
];

const QUERY = 'Find the approximate geographic coordinates of Example City, Washington.';
const SUMMARY = [
  'destination: search-provider',
  `would_send: ${QUERY}`,
  'stopped: untrusted_source',
  'untrusted_source: tool_result',
].join('\n');
const SPOKEN = '我帮你查一下';

function history() {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '明天天气怎么样?', timestamp: 1_758_000_000, turn_id: 'weather' },
    ],
    approvals: [],
  };
}

function boxesOverlap(a, b) {
  if (!a || !b || a.width <= 0 || a.height <= 0 || b.width <= 0 || b.height <= 0) return false;
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
}

function approvalBody(id, action, message) {
  return {
    status: 'ok',
    approval_id: id,
    action,
    risk: 'R1 Low',
    approval_type: 'tool:web_search',
    message,
  };
}

export async function run({ page, ok, fail, shot }) {
  const calls = [];
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({ json: history() }));
  await page.route('**/webapp/console/approvals/**', async (route) => {
    const url = route.request().url();
    const action = url.includes('/reject') ? 'reject' : 'approve';
    const id = decodeURIComponent(url.split('/approvals/')[1]?.split('/')[0] || '');
    calls.push({ action, id });
    const approveCount = calls.filter((call) => call.action === 'approve').length;
    const message = action === 'reject'
      ? 'declined'
      : (approveCount === 1 ? '明天晴，大约二十度。' : '坐标大约是北纬四十八度。');
    await route.fulfill({ json: approvalBody(id, action, message) });
  });

  async function showGate(approvalId) {
    await page.evaluate(({ approvalId: id, summary, spoken }) => {
      window.dispatchEvent(new CustomEvent('aegis-voice-approval', {
        detail: { approvalId: id, toolName: 'web_search', summary },
      }));
      window.dispatchEvent(new CustomEvent('aegis-voice-caption', {
        detail: { speaking: true, text: spoken, role: 'assistant' },
      }));
    }, { approvalId, summary: SUMMARY, spoken: SPOKEN });
  }

  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    await page.locator('article').filter({ hasText: '明天天气怎么样?' }).first().waitFor({ state: 'visible', timeout: 30000 });
    await showGate('gate-voice-tap');
    const card = page.getByTestId('voice-approval-card');
    await card.waitFor({ state: 'visible', timeout: 10000 });
    if (await page.getByTestId('voice-approval-card').count() === 1) ok('one approval card');
    else fail('expected one approval card', String(await page.getByTestId('voice-approval-card').count()));

    const cardText = await card.innerText();
    if (cardText.includes('Example City') && cardText.includes('需要你确认') && cardText.includes('批准') && cardText.includes('先不要')) {
      ok('the card shows the search and the two actions');
    } else fail('card is missing the search or the actions', cardText);
    if (!/destination:|would_send:|web_search|Tool action|untrusted_source|tool_result/.test(cardText)) {
      ok('the card hides field names and tool ids');
    } else fail('card still shows the egress dump', cardText);

    const line = page.getByTestId('ceo-voice-line');
    await line.waitFor({ state: 'visible', timeout: 10000 });
    const cardBox = await card.boundingBox();
    const lineBox = await line.boundingBox();
    const composerBox = await page.locator('.vnext-chat-composer').first().boundingBox();
    const articleBox = await page.locator('article').filter({ hasText: '明天天气怎么样?' }).first().boundingBox();
    if (cardBox && lineBox && !boxesOverlap(cardBox, lineBox) && cardBox.y + cardBox.height <= lineBox.y + 4) {
      ok('the card sits above the speaking line without overlap');
    } else fail('the card overlaps the speaking line', JSON.stringify({ cardBox, lineBox }));
    if (cardBox && articleBox && !boxesOverlap(cardBox, articleBox)) ok('the card does not cover the transcript');
    else fail('the card overlaps the transcript', JSON.stringify({ cardBox, articleBox }));
    if (lineBox && composerBox && lineBox.y + lineBox.height <= composerBox.y + 4) ok('the speaking line stays above the composer');
    else fail('the speaking line overlaps the composer', JSON.stringify({ lineBox, composerBox }));
    if (await page.locator('.vnext-ceo-voice-confirm, .vnext-ceo-voice-confirm-float').count() === 0) {
      ok('no corner approval float');
    } else fail('the corner approval float is still up');
    await shot('01-one-card');

    await page.getByTestId('voice-approval-approve').click();
    await page.locator('article').filter({ hasText: '明天晴，大约二十度。' }).first().waitFor({ state: 'visible', timeout: 15000 });
    await card.waitFor({ state: 'hidden', timeout: 10000 });
    if (calls.some((call) => call.action === 'approve' && call.id === 'gate-voice-tap')) ok('tapping 批准 approves that search');
    else fail('tap did not approve the pending search', JSON.stringify(calls));
    await shot('02-tap-approved');

    await showGate('gate-voice-say');
    await card.waitFor({ state: 'visible', timeout: 10000 });
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent('aegis-voice-caption', {
        detail: { role: 'owner', text: '批准。', speaking: false },
      }));
    });
    await page.locator('article').filter({ hasText: '坐标大约是北纬四十八度。' }).first().waitFor({ state: 'visible', timeout: 15000 });
    await card.waitFor({ state: 'hidden', timeout: 10000 });
    if (calls.some((call) => call.action === 'approve' && call.id === 'gate-voice-say')) ok('saying 批准。 approves the same search');
    else fail('spoken 批准。 did not approve', JSON.stringify(calls));
    await shot('03-spoken-approved');

    await showGate('gate-voice-no');
    await card.waitFor({ state: 'visible', timeout: 10000 });
    await page.getByTestId('voice-approval-reject').click();
    await page.locator('article').filter({ hasText: '先不查了。' }).first().waitFor({ state: 'visible', timeout: 15000 });
    await card.waitFor({ state: 'hidden', timeout: 10000 });
    if (calls.some((call) => call.action === 'reject' && call.id === 'gate-voice-no')) ok('先不要 declines the search');
    else fail('先不要 did not decline', JSON.stringify(calls));
    await shot('04-declined');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute('**/webapp/vnext/chat-history*');
    await page.unroute('**/webapp/console/approvals/**');
  }
}
