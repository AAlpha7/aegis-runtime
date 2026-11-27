// Voice: one line above the composer while 安琪 speaks, then it joins the
// left transcript. Search: the receipt shows the owner's question and a
// plain source, never the model instruction.
export const id = 'chat-voice-line-and-receipt';
export const title = 'One voice line, and a search receipt without the model note';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/CeoVoiceAgent.tsx',
  'webapp/src/vnext/SearchReceiptCard.tsx',
  'webapp/src/vnext/AssistantResponseView.tsx',
];

const NOTE = 'Use only these excerpts. Do not fill a gap from memory. Do not call web_search again in this turn.';
const INSTRUCTION = 'Weather forecast for Example City, Washington for tomorrow, Saturday. Return the exact forecast numbers. Do not invent a source.';
const SPOKEN = '语音这一句只在左边。';

const EVIDENCE_EXCERPT = JSON.stringify({
  status: 'ok',
  search_prompt: INSTRUCTION,
  summary: `**Saturday**: Sunny, high 72°F. ${NOTE}`,
  retryable: false,
  note: NOTE,
  verified_citations: [{
    title: '',
    url: 'https://weather.gov/sew',
    snippet: '**Saturday**: Sunny, high 72°F.',
  }],
});

const ANSWER = [
  '明天晴，大约二十度。',
  '',
  '搜索收据',
  `查了：${INSTRUCTION}`,
  '1. 来源 — **Saturday**: Sunny',
  '@@search-receipt',
  JSON.stringify({
    queries: [INSTRUCTION],
    sources: [{
      query: INSTRUCTION,
      title: '',
      url: 'https://weather.gov/sew',
      excerpt: `**Saturday**: Sunny, high 72°F. ${NOTE}`,
    }],
  }),
  '@@end-search-receipt',
].join('\n');

function history() {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '明天天气怎么样?', timestamp: 1_758_000_000, turn_id: 'weather' },
      {
        role: 'aegis',
        text: ANSWER,
        timestamp: 1_758_000_020,
        turn_id: 'weather',
        evidence: [{
          id: 'E1',
          tool: 'web_search',
          label: 'Web search',
          excerpt: EVIDENCE_EXCERPT,
          full_len: 796,
        }],
      },
    ],
    approvals: [],
  };
}

function boxesOverlap(a, b) {
  if (!a || !b) return false;
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
}

export async function run({ page, ok, fail, shot }) {
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({ json: history() }));
  try {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const receipt = page.getByTestId('search-receipt');
    await receipt.waitFor({ state: 'visible', timeout: 30000 });
    const receiptText = await receipt.innerText();
    if (receiptText.includes('明天天气怎么样?') && receiptText.includes('weather.gov')) {
      ok('receipt shows the question and the source name');
    } else fail('receipt missing the question or source name', receiptText);
    if (receiptText.includes('Sunny') && !receiptText.includes('**') && !receiptText.includes(NOTE) && !receiptText.includes('Return the exact')) {
      ok('receipt excerpt is plain and short');
    } else fail('receipt still shows markdown or the model instruction', receiptText.slice(0, 400));

    const source = page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
    await source.waitFor({ state: 'attached', timeout: 15000 });
    const outer = source.locator('xpath=ancestor::details').first();
    if (await outer.count() && !await outer.evaluate((el) => el.open)) {
      await outer.locator(':scope > summary').click();
    }
    await source.waitFor({ state: 'visible', timeout: 15000 });
    const sourceText = await source.innerText();
    const pageText = await page.locator('.vnext-chat-home').innerText();
    if (!sourceText.includes(NOTE) && !sourceText.includes('可原样重试') && !sourceText.includes('来源摘要 · 原文 796') && !sourceText.includes('**')) {
      ok('E1 hides the model note, retry flag, and markdown');
    } else fail('E1 still shows the model payload', sourceText.slice(0, 400));
    if (pageText.includes('明天晴，大约二十度')) ok('the Chinese answer stays');
    else fail('Chinese answer missing', pageText.slice(0, 300));
    await shot('01-receipt');

    const rail = page.locator('.vnext-chat-context-rail');
    await rail.waitFor({ state: 'visible', timeout: 15000 });
    const railText = await rail.innerText();
    if (/上下文/.test(railText) && /附近的/.test(railText)) ok('context panel stays in place');
    else fail('context panel missing', railText);

    await page.evaluate((text) => {
      window.dispatchEvent(new CustomEvent('aegis-voice-caption', { detail: { speaking: true, text } }));
    }, SPOKEN);
    const line = page.getByTestId('ceo-voice-line');
    await line.waitFor({ state: 'visible', timeout: 10000 });
    const lineCount = await page.getByTestId('ceo-voice-line').count();
    const cardCount = await page.locator('.vnext-ceo-voice-caption').count();
    if (lineCount === 1 && cardCount === 0) ok('one speaking line and no corner cards');
    else fail('voice overlay is still a stack', `lines=${lineCount} cards=${cardCount}`);
    const lineBox = await line.boundingBox();
    const railBox = await rail.boundingBox();
    const composerBox = await page.locator('.vnext-chat-composer').first().boundingBox();
    if (lineBox && composerBox && lineBox.y + lineBox.height <= composerBox.y + 4) {
      ok('the line sits just above the composer');
    } else fail('the line is not above the composer', JSON.stringify({ lineBox, composerBox }));
    if (!boxesOverlap(lineBox, railBox)) ok('the line does not cover the context panel');
    else fail('the line overlaps the context panel', JSON.stringify({ lineBox, railBox }));
    await shot('02-speaking-line');

    await page.evaluate((text) => {
      window.dispatchEvent(new CustomEvent('aegis-voice-caption', { detail: { speaking: false, text } }));
    }, SPOKEN);
    await page.locator('article').filter({ hasText: SPOKEN }).first().waitFor({ state: 'visible', timeout: 10000 });
    await line.waitFor({ state: 'hidden', timeout: 10000 });
    const copies = await page.locator('article').filter({ hasText: SPOKEN }).count();
    const corner = await page.locator('.vnext-ceo-voice-caption, [data-testid="ceo-voice-captions"]').count();
    if (copies === 1 && corner === 0) ok('the line retires into the left transcript once');
    else fail('the spoken line was duplicated or stayed in the corner', `copies=${copies} corner=${corner}`);
    const railAfter = await rail.boundingBox();
    if (railAfter && railBox && Math.abs(railAfter.x - railBox.x) < 2 && Math.abs(railAfter.y - railBox.y) < 2) {
      ok('the context panel did not move');
    } else fail('the context panel moved', JSON.stringify({ railBox, railAfter }));
    await shot('03-retired');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute('**/webapp/vnext/chat-history*');
  }
}
