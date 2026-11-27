export const id = 'chat-search-approval';
export const title = 'One search approval stays on its card through the result';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/searchApproval.ts',
];

const SUMMARY = [
  'destination: search-provider',
  'would_send: Find the latest posts about the launch',
  'stopped: untrusted_source',
  'untrusted_source: tool_result',
  'owner_question: 帮我看看最近的帖子在说什么',
].join('\n');

function history(state, outcome) {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      {
        role: 'owner',
        text: '帮我看看最近的帖子在说什么',
        timestamp: 1_758_000_000,
        turn_id: 'search-turn',
      },
      {
        role: 'aegis',
        text: '⏳ Tool action · 等待你批准 / awaiting your approval',
        timestamp: 1_758_000_050,
        turn_id: 'search-turn',
      },
    ],
    approvals: [{
      approval_id: 'gate-bbbbbbbbbbbbbbbb',
      tool_name: 'web_search',
      summary: SUMMARY,
      state,
      timestamp: 1_758_000_100,
      ...(outcome ? { outcome } : {}),
    }],
  };
}

function emptyWork() {
  const lanes = ['now', 'decide', 'follow_up', 'backlog', 'done'].map((id) => ({
    id,
    title: id,
    subtitle: '',
    collapsed: false,
    items: [],
  }));
  return {
    schema_version: 1,
    generated_at: '2026-09-26T00:00:00Z',
    sources: [],
    lanes,
    counts: { now: 0, decide: 0, follow_up: 0, backlog: 0, done: 0 },
  };
}

export async function run({ page, ok, fail, shot }) {
  let decided = false;
  let posts = 0;
  let releaseApproval;
  const approvalGate = new Promise((resolve) => {
    releaseApproval = resolve;
  });
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({
    json: history(
      decided ? 'approved' : 'pending',
      decided ? 'result\n最近的帖子提到发布会。' : '',
    ),
  }));
  await page.route('**/webapp/vnext/work**', (route) => route.fulfill({ json: emptyWork() }));
  await page.route('**/webapp/console/approvals/**', async (route) => {
    posts += 1;
    await approvalGate;
    decided = true;
    return route.fulfill({
      json: {
        status: 'ok',
        approval_id: 'gate-bbbbbbbbbbbbbbbb',
        action: 'approve',
        risk: 'R2 Medium',
        approval_type: 'tool:web_search',
        message: '{"status":"ok","summary":"最近的帖子提到发布会。","note":"Use only these excerpts."}',
      },
    });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const question = page.getByTestId('search-owner-question');
    await question.waitFor({ state: 'visible', timeout: 30000 });
    const questionText = await question.innerText();
    if (questionText.includes('帮我看看最近的帖子在说什么')) ok('search card shows the owner question');
    else fail('owner question missing', questionText);
    const stopped = await page.getByTestId('egress-stopped').innerText();
    if (/来源不可信/.test(stopped)) ok('untrusted source stays in Chinese');
    else fail('stopped reason missing', stopped);
    const body = await page.locator('body').innerText();
    if (!body.includes('search-provider') && !body.includes('web_search') && !body.includes('Tool action') && !body.includes('等待你批准')) {
      ok('tool dump and the second wait line are gone');
    } else fail('tool dump or wait line is still on screen', body.slice(0, 500));
    if (await page.locator('.vnext-chat-approval-card').count() === 1) ok('one search card');
    else fail('expected one search card');
    await shot('01-one-search-card');

    await page.getByRole('button', { name: '批准' }).click();
    await page.getByTestId('search-approval-progress').waitFor({ state: 'visible', timeout: 15000 });
    const running = await page.locator('.vnext-chat-approval-card').innerText();
    if (running.includes('已批准') && running.includes('正在继续这次搜索')) ok('same card shows in progress after approve');
    else fail('in-progress state missing', running);
    if (await page.locator('.vnext-chat-approval-card').count() === 1) ok('still one card while searching');
    else fail('a second card appeared while searching');
    await shot('02-search-running');

    releaseApproval();
    await page.getByTestId('search-approval-result').waitFor({ state: 'visible', timeout: 15000 });
    const settled = await page.locator('.vnext-chat-approval-card').innerText();
    if (settled.includes('已批准') && settled.includes('最近的帖子提到发布会。') && !settled.includes('正在继续这次搜索')) {
      ok('same card shows the search result');
    } else fail('result missing after approve', settled);
    if (await page.getByRole('button', { name: '批准' }).count() === 0) ok('approve button leaves after the decision');
    else fail('approve button stayed');
    await shot('03-search-result');

    const box = page.getByRole('textbox').last();
    await box.fill('批准');
    await box.press('Enter');
    await page.getByText('这次搜索已经批准').waitFor({ state: 'visible', timeout: 15000 });
    if (posts === 1 && await page.locator('.vnext-chat-approval-card').count() === 1) {
      ok('a repeated 批准 does not open another hold');
    } else fail('repeated 批准 created another hold', `posts=${posts}`);
    await shot('04-repeat-approve');

    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByText('已批准').waitFor({ state: 'visible', timeout: 30000 });
    await page.getByText('最近的帖子提到发布会。').waitFor({ state: 'visible', timeout: 15000 });
    const reloaded = await page.locator('body').innerText();
    if (
      await page.locator('.vnext-chat-approval-card').count() === 1
      && reloaded.includes('帮我看看最近的帖子在说什么')
      && !reloaded.includes('Tool action')
      && !reloaded.includes('等待你批准')
      && !reloaded.includes('search-provider')
    ) {
      ok('reload keeps one approved card and the result');
    } else fail('reload brought the wait line or a second card back');
    await shot('05-reload');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    releaseApproval();
    await page.unroute('**/webapp/vnext/chat-history*');
    await page.unroute('**/webapp/vnext/work**');
    await page.unroute('**/webapp/console/approvals/**');
  }
}
