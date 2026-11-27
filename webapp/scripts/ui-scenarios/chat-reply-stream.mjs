// Owner chat paints assistant tokens before the command returns, then replaces
// that provisional bubble with the single stored reply.
export const id = 'chat-reply-stream';
export const title = 'Assistant tokens appear in the chat before the turn finishes';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/turnStream.ts',
  'webapp/src/vnext/App.tsx',
];

const PARTIAL = '早安';
const FULL = '早安，今天适合出门。';

export async function run({ page, ok, fail, shot }) {
  await page.addInitScript(() => {
    class ScriptedEventSource {
      constructor(url) {
        this.url = String(url);
        this.readyState = 1;
        this.listeners = {};
        this.onerror = null;
        const bucket = window.__aegisScriptedSources || [];
        bucket.push(this);
        window.__aegisScriptedSources = bucket;
      }

      addEventListener(type, fn) {
        const list = this.listeners[type] || [];
        list.push(fn);
        this.listeners[type] = list;
      }

      close() {
        this.readyState = 2;
      }
    }
    window.EventSource = ScriptedEventSource;
  });

  let releaseCommand = () => {};
  const gate = new Promise((resolve) => {
    releaseCommand = resolve;
  });
  await page.route('**/webapp/vnext/turns/**/stream-token', (route) => {
    if (route.request().method() !== 'POST') return route.fallback();
    return route.fulfill({
      json: {
        status: 'ok',
        stream_token: 'stream-demo',
        expires_at: Math.floor(Date.now() / 1000) + 60,
        expires_in: 60,
      },
    });
  });
  await page.route('**/webapp/vnext/command', async (route) => {
    if (route.request().method() !== 'POST') {
      await route.fallback();
      return;
    }
    await gate;
    const body = route.request().postDataJSON();
    await route.fulfill({
      json: {
        schema_version: 1,
        generated_at: new Date().toISOString(),
        outcome: 'answered',
        owner_message: '好',
        work_item: null,
        assistant_response: {
          schema_version: 1,
          response_kind: 'answer',
          visible_answer: FULL,
          turn_id: body?.idempotency_key || 'turn-stream',
          receipt_expectation: '',
          citations: [],
          warnings: [],
        },
      },
    });
  });

  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const textarea = page.locator('.vnext-chat-composer textarea').first();
    await textarea.waitFor({ state: 'visible', timeout: 30000 });
    await textarea.fill('今天天气怎么样');
    await page.locator('.vnext-chat-composer button[type="submit"]').first().click();

    await page.waitForFunction(
      () => (window.__aegisScriptedSources || []).length > 0,
      null,
      { timeout: 15000 },
    );
    const emit = (type, data) => page.evaluate(({ type, data }) => {
      const sources = window.__aegisScriptedSources || [];
      const es = sources[sources.length - 1];
      const frame = { data: JSON.stringify(data) };
      for (const fn of es.listeners[type] || []) fn(frame);
    }, { type, data });

    await emit('assistant_delta', { type: 'assistant_delta', turn_id: 't', text: '早' });
    await emit('assistant_delta', { type: 'assistant_delta', turn_id: 't', text: '安' });
    const stream = page.getByTestId('aegis-chat-stream');
    await stream.waitFor({ state: 'visible', timeout: 5000 });
    const mid = (await stream.innerText()).trim();
    if (mid === PARTIAL) ok('partial tokens visible before the turn finishes', mid);
    else fail('stream text before completion', mid);
    if (await page.getByText(FULL, { exact: true }).count() === 0) ok('full reply is still absent');
    else fail('full reply appeared before the command finished');
    await shot('01-partial-tokens');

    await emit('assistant_delta', { type: 'assistant_delta', turn_id: 't', text: '，今天适合出门。' });
    const grown = (await stream.innerText()).trim();
    if (grown === FULL) ok('later tokens append to the same bubble', grown);
    else fail('stream did not append', grown);
    await shot('02-more-tokens');

    releaseCommand();
    const answer = page.locator('article.aegis, article.vnext-chat-message.aegis').filter({ hasText: FULL });
    await page.locator('article').filter({ hasText: FULL }).first().waitFor({ state: 'visible', timeout: 20000 });
    if (await page.getByTestId('aegis-chat-stream').count() === 0) ok('provisional stream is replaced by the stored reply');
    else fail('stream bubble remained after the turn finished');
    const cards = await page.locator('article').filter({ hasText: FULL }).count();
    if (cards === 1) ok('one assistant reply, no duplicate partial');
    else fail('duplicate reply cards', String(cards));
    if (await answer.count()) ok('settled reply uses the aegis card');
    await shot('03-settled');
  } finally {
    releaseCommand();
    await page.unroute('**/webapp/vnext/turns/**/stream-token');
    await page.unroute('**/webapp/vnext/command');
  }
}
