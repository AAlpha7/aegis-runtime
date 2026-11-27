export const id = 'chat-failure-retry';
export const title = 'A failure hint offers retry that resends the owner message';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/chatHistory.ts',
  'core/src/admin/vnext_projections.rs',
];

const HINT = '这轮没有生成回复。消息已留下，可以再发一次。原因：connection refused';
const EMPTY = '助手返回了空回复。';
const OWNER = '帮我看看这封信';

export async function run({ page, ok, fail, shot }) {
  const historyPattern = '**/webapp/vnext/chat-history*';
  const commandPattern = '**/webapp/vnext/command';
  const now = Math.floor(Date.now() / 1000);
  const history = {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: OWNER, timestamp: now - 40, turn_id: 'turn-stuck' },
      {
        role: 'aegis',
        text: HINT,
        timestamp: now - 40,
        turn_id: 'turn-stuck',
        severity: 'warning',
        retry_text: OWNER,
      },
      { role: 'owner', text: '空白那轮', timestamp: now - 30, turn_id: 'turn-empty' },
      {
        role: 'aegis',
        text: EMPTY,
        timestamp: now - 30,
        turn_id: 'turn-empty',
        severity: 'info',
        retry_text: '空白那轮',
      },
      { role: 'owner', text: '已经答过', timestamp: now - 20, turn_id: 'turn-done' },
      { role: 'aegis', text: '信已归档。', timestamp: now - 19, turn_id: 'turn-done' },
    ],
  };
  let commandBody = null;
  let commandCsrf = '';
  await page.route(historyPattern, (route) => route.fulfill({ json: history }));
  await page.route(commandPattern, async (route) => {
    if (route.request().method() !== 'POST') {
      await route.fallback();
      return;
    }
    commandBody = route.request().postDataJSON();
    commandCsrf = route.request().headers()['x-aegis-csrf'] || '';
    await route.fulfill({
      json: {
        schema_version: 1,
        generated_at: new Date().toISOString(),
        outcome: 'answered',
        owner_message: '已重新发送。',
        work_item: null,
        assistant_response: {
          schema_version: 1,
          response_kind: 'answer',
          visible_answer: '已重新处理这封信。',
          turn_id: commandBody?.idempotency_key || 'turn-retry',
          receipt_expectation: '',
          citations: [],
          warnings: [],
        },
      },
    });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const hintCard = page.locator('article').filter({ hasText: HINT });
    const retry = hintCard.getByTestId('aegis-chat-retry');
    await retry.waitFor({ state: 'visible', timeout: 30000 });
    const label = (await retry.innerText()).trim();
    if (label === '重试') ok('failure hint shows 重试');
    else fail('failure hint retry label', label);
    const emptyCard = page.locator('article').filter({ hasText: EMPTY });
    if ((await emptyCard.getByTestId('aegis-chat-retry').innerText()).trim() === '重试') {
      ok('empty reply hint shows 重试');
    } else fail('empty reply hint has no 重试');
    const archived = page.locator('article').filter({ hasText: '信已归档。' });
    if (await archived.getByTestId('aegis-chat-retry').count() === 0) ok('ordinary reply has no retry');
    else fail('ordinary reply offered retry');
    await retry.scrollIntoViewIfNeeded();
    await shot('01-failure-hint-retry');
    await retry.click();
    const answer = page.locator('article').filter({ hasText: '已重新处理这封信。' });
    await answer.waitFor({ state: 'visible', timeout: 20000 });
    if (commandBody?.input === OWNER) ok('retry resends the owner message');
    else fail('retry sent the wrong text', JSON.stringify(commandBody?.input));
    if (commandCsrf.trim()) ok('retry sends the session CSRF token');
    else fail('retry omitted the CSRF token');
    await answer.scrollIntoViewIfNeeded();
    await shot('02-retry-sent');
  } finally {
    await page.unroute(historyPattern);
    await page.unroute(commandPattern);
  }
}
