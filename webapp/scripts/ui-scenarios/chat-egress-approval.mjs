export const id = 'chat-egress-approval';
export const title = 'One egress hold sits on the chat thread and stays approved';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/App.tsx',
  'core/src/admin/vnext_projections.rs',
];

const SUMMARY = 'destination: lawyer@example.com\nwould_send: billing note\nstopped: sensitive';

function history(state) {
  return {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '把账单寄给律师', timestamp: 1_758_000_000, turn_id: 'hold-turn' },
    ],
    approvals: [{
      approval_id: 'gate-aaaaaaaaaaaaaaaa',
      tool_name: 'send_email',
      summary: SUMMARY,
      state,
      timestamp: 1_758_000_100,
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
    generated_at: '2026-09-25T00:00:00Z',
    sources: [],
    lanes,
    counts: { now: 0, decide: 0, follow_up: 0, backlog: 0, done: 0 },
  };
}

export async function run({ page, ok, fail, shot }) {
  let decided = false;
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({
    json: history(decided ? 'approved' : 'pending'),
  }));
  await page.route('**/webapp/vnext/work**', (route) => route.fulfill({ json: emptyWork() }));
  await page.route('**/webapp/console/approvals/**', (route) => {
    decided = true;
    return route.fulfill({
      json: {
        status: 'ok',
        approval_id: 'gate-aaaaaaaaaaaaaaaa',
        action: 'approve',
        risk: 'R2 Medium',
        approval_type: 'tool:send_email',
        message: 'approved',
      },
    });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const destination = page.getByTestId('egress-destination');
    await destination.waitFor({ state: 'visible', timeout: 30000 });
    if ((await destination.innerText()).includes('lawyer@example.com')) ok('chat card names the destination');
    else fail('destination missing on the chat card');
    const wouldSend = await page.getByTestId('egress-would-send').innerText();
    if (wouldSend.includes('billing note')) ok('chat card names what would be sent');
    else fail('would-send missing', wouldSend);
    const stopped = await page.getByTestId('egress-stopped').innerText();
    if (/敏感|sensitive/i.test(stopped)) ok('chat card names why it stopped');
    else fail('stopped reason missing', stopped);
    const generic = page.getByText('我在应用外动手前，请你先看一眼。');
    if (await generic.count() === 0) ok('no generic tool-request sentence on the thread');
    else fail('generic tool sentence is still on screen');
    await shot('01-pending-facts');

    await page.getByRole('button', { name: '批准' }).click();
    await page.getByText('已批准').waitFor({ state: 'visible', timeout: 15000 });
    if (await page.getByRole('button', { name: '批准' }).count() === 0) ok('approve control leaves after the decision');
    else fail('approve button stayed after the decision');
    await shot('02-approved');

    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByText('已批准').waitFor({ state: 'visible', timeout: 30000 });
    const reloadedDestination = await page.getByTestId('egress-destination').innerText();
    if (reloadedDestination.includes('lawyer@example.com') && await page.getByRole('button', { name: '批准' }).count() === 0) {
      ok('reload still shows the approved hold and its facts');
    } else fail('approved hold did not survive reload', reloadedDestination);

    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.click();
    await page.getByRole('heading', { name: /没有等待你处理的待办。|Nothing is waiting for you\./ }).waitFor({ state: 'visible' });
    if (await page.getByText('工具请求').count() === 0 && await page.getByText('我在应用外动手前').count() === 0) {
      ok('work has no stack of generic approval cards');
    } else fail('generic approval cards are still on work');
    await shot('03-work-without-stack');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute('**/webapp/vnext/chat-history*');
    await page.unroute('**/webapp/vnext/work**');
    await page.unroute('**/webapp/console/approvals/**');
  }
}
