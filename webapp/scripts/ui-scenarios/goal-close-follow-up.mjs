import { injectedWorkPayload } from './work-desktop-controls.mjs';
export const id = 'goal-close-follow-up';
export const title = 'Goal follow-up: confirm, failure feedback, retry, receipt and retired controls';
export const surfaces = [
  'core/src/admin/inbox_projection.rs', 'webapp/src/vnext/model.ts',
  'webapp/src/vnext/followUpCleanup.ts', 'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/chatWorkState.ts',
];
export async function run({ page, ok, fail, shot }) {
  let closed = false;
  let actions = 0;
  let commands = 0;
  const pattern = '**/webapp/vnext/work**';
  page.on('request', request => {
    if (new URL(request.url()).pathname === '/webapp/vnext/command' && request.method() === 'POST') commands++;
  });
  await page.route(pattern, async route => {
    const request = route.request();
    if (new URL(request.url()).pathname.endsWith('/actions')) {
      actions++;
      const body = request.postDataJSON();
      const headers = request.headers();
      if (request.method() !== 'POST' || body.schema_version !== 1 || body.action_id !== 'goal.abandon'
          || (!headers.authorization && !headers['x-aegis-csrf'])) {
        fail('invalid authenticated goal action request');
        await route.fulfill({ status: 400, json: { error: 'invalid goal action' } });
        return;
      }
      if (actions === 1) {
        await route.fulfill({ status: 500, json: { error: 'Goal close temporarily unavailable' } });
        return;
      }
      closed = true;
      await route.fulfill({ json: {
        schema_version: 1, generated_at: new Date().toISOString(), status: 'ok',
        item_id: 'goal:4242', action_id: 'goal.abandon',
        owner_message: '已结束目标跟进，原有进度和历史记录已保留。',
        receipt: { id: 'goal-close-fixture', kind: 'action_receipt', label: 'goal:4242:abandoned', created_at: new Date().toISOString() },
        raw_result: { goal_status: 'abandoned' },
      } });
      return;
    }
    const payload = injectedWorkPayload({ generated_at: new Date().toISOString(), sources: [] });
    const lane = payload.lanes.find(lane => lane.id === 'follow_up');
    const item = lane.items[0];
    Object.assign(item, {
      id: 'goal:4242', source_id: '4242', source: 'goals', source_kind: 'goal', state: 'tracking',
      title: '跟踪发行更新', summary: '原有进度 25%，保留研究记录。',
      why_it_matters: '需要停止跟踪的目标', next_safe_action: '给出下一步，或结束目标跟进。', urgency: 'normal',
      contextual_actions: [{ id: 'goal.abandon', kind: 'goal.abandon', label: '结束跟进',
        requires_confirmation: true, requires_data_gate: false, requires_mfa: false,
        route: '/webapp/vnext/work/goal:4242/actions', expected_receipt: '保留历史的关闭记录' }],
    });
    item.authority.requires_data_grant = false;
    lane.items = closed ? [] : [item];
    payload.counts.follow_up = lane.items.length;
    await route.fulfill({ json: payload });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 30000 });
    await work.click();
    const row = page.getByText('跟踪发行更新', { exact: true }).filter({ visible: true }).first();
    await row.waitFor({ state: 'visible' });
    const close = () => page.getByRole('button', { name: /^(结束这一次|End this card)$/ }).filter({ visible: true }).first();
    await close().click();
    if (actions === 0) ok('first click asks for confirmation without mutation');
    else fail('first click mutated the goal');
    await page.getByRole('button', { name: /^(确认？|Confirm\?)$/ }).filter({ visible: true }).first().click();
    await close().waitFor({ state: 'visible' });
    await page.getByText(/Goal close temporarily unavailable/).first().waitFor({ state: 'visible' });
    if (actions === 1 && !closed) ok('failure keeps goal and retry control visible');
    else fail('failed action lost goal or retried unexpectedly');
    await shot('01-failed-close');
    await close().click();
    await page.getByRole('button', { name: /^(确认？|Confirm\?)$/ }).filter({ visible: true }).first().click();
    await page.getByText('已结束目标跟进，原有进度和历史记录已保留。', { exact: true }).filter({ visible: true }).first().waitFor({ state: 'visible' });
    await row.waitFor({ state: 'hidden' });
    if (actions === 2 && closed && await close().count() === 0) ok('success refreshes queue and retires close control');
    else fail('close control survived success or request count incorrect');
    if (commands === 0) ok('goal closure never enters the LLM chat loop');
    else fail('goal closure called assistant command');
    await shot('02-closed-receipt');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute(pattern);
  }
}
