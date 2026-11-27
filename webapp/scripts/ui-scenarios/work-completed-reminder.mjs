import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-completed-reminder';
export const title = 'Completed recurring reminder stays out of actionable Work';
export const surfaces = ['core/src/admin/inbox_projection.rs', 'webapp/src/vnext/WorkView.tsx'];

// The Rust projection regression supplies the server-side proof. This fixture
// checks that the terminal contract stays out of actionable Work on both phones.
export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/work**';
  let mutations = 0;
  await page.route(pattern, async route => {
    if (route.request().method() !== 'GET') {
      mutations++;
      await route.fulfill({ status: 409, json: { error: 'Completed reminder has no actions' } });
      return;
    }
    const payload = injectedWorkPayload({ generated_at: new Date().toISOString(), sources: [] });
    const item = payload.lanes.find(lane => lane.id === 'follow_up').items[0];
    Object.assign(item, {
      id: 'task:4243', source_id: '4243', source: 'tasks', source_kind: 'task',
      lane: 'done', state: 'completed', urgency: 'silent',
      title: '通勤提醒已完成', summary: '已完成三次提醒，记录已保留。',
      why_it_matters: '提醒次数已用完。', next_safe_action: '查看历史记录。',
      contextual_actions: [], recommended_action: null,
    });
    item.authority.requires_data_grant = false;
    for (const lane of payload.lanes) {
      lane.items = lane.id === 'done' ? [item] : [];
      lane.collapsed = false;
    }
    payload.counts = { now: 0, decide: 0, follow_up: 0, backlog: 0, done: 1 };
    await route.fulfill({ json: payload });
  });
  try {
    for (const pass of ['initial', 'reload']) {
      await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
      const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
      await work.waitFor({ state: 'visible', timeout: 30000 });
      await work.click();
      await page.getByRole('heading', { name: /没有等待你处理的待办。|Nothing is waiting for you\./ }).waitFor({ state: 'visible' });
      if (await page.getByText('通勤提醒已完成', { exact: true }).filter({ visible: true }).count() === 0) {
        ok(`${pass}: completed reminder is excluded from actionable Work`);
      } else fail(`${pass}: completed reminder remains in actionable Work`);
      const stale = page.getByRole('button', { name: /^(结束跟进|完成|明天再看|Close follow-up|Complete|Review tomorrow)$/ }).filter({ visible: true });
      if (await stale.count() === 0) ok(`${pass}: no stale completion or snooze controls`);
      else fail(`${pass}: terminal reminder exposes lifecycle controls`);
      // Capture the destination after the Chat → Work crossfade settles.
      await page.waitForTimeout(500);
      await shot(pass);
    }
    if (mutations === 0) ok('loading terminal reminder sends no lifecycle mutation');
    else fail('loading terminal reminder mutated reminder');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute(pattern);
  }
}
