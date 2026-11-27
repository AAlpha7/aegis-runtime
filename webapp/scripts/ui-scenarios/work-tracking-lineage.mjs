import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-tracking-lineage';
export const title = 'Work tracking lineage: source chain, skip once, close the generator';
export const surfaces = [
  'core/src/admin/inbox_projection.rs',
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/model.ts',
];

export async function run({ page, ok, fail, shot }) {
  let phase = 'open';
  const seen = [];
  const pattern = '**/webapp/vnext/work**';
  page.on('request', request => {
    if (new URL(request.url()).pathname.endsWith('/actions') && request.method() === 'POST') {
      seen.push(request.postDataJSON()?.action_id);
    }
  });
  await page.route(pattern, async route => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname.endsWith('/actions')) {
      const body = request.postDataJSON();
      const headers = request.headers();
      if (request.method() !== 'POST' || body.schema_version !== 1
          || (!headers.authorization && !headers['x-aegis-csrf'])) {
        fail('invalid authenticated tracking action');
        await route.fulfill({ status: 400, json: { error: 'invalid tracking action' } });
        return;
      }
      if (body.action_id === 'task.skip_occurrence') {
        phase = 'skipped';
        await route.fulfill({ json: {
          schema_version: 1,
          generated_at: new Date().toISOString(),
          status: 'ok',
          item_id: 'task:12',
          action_id: 'task.skip_occurrence',
          owner_message: '这一次已跳过。跟踪还在，下次还会出现。',
          receipt: { id: 'skip-1', kind: 'action_receipt', label: 'task:12:skipped', created_at: new Date().toISOString() },
          raw_result: { task_status: 'active' },
        } });
        return;
      }
      if (body.action_id === 'tracking.cancel') {
        phase = 'closed';
        await route.fulfill({ json: {
          schema_version: 1,
          generated_at: new Date().toISOString(),
          status: 'ok',
          item_id: 'task:12',
          action_id: 'tracking.cancel',
          owner_message: '整条跟踪已关闭。记录还在。',
          receipt: { id: 'cancel-1', kind: 'action_receipt', label: 'task:12:cancelled', created_at: new Date().toISOString() },
          raw_result: { task_status: 'cancelled' },
        } });
        return;
      }
      fail(`unexpected action ${body.action_id}`);
      await route.fulfill({ status: 400, json: { error: 'unexpected action' } });
      return;
    }
    const payload = injectedWorkPayload({ generated_at: new Date().toISOString(), sources: [] });
    const lane = payload.lanes.find(entry => entry.id === 'follow_up');
    const item = lane.items[0];
    Object.assign(item, {
      id: 'task:12',
      source_id: '12',
      source: 'tasks',
      source_kind: 'task',
      state: 'tracking',
      title: '门铃电量',
      summary: '循环跟踪门铃电量。',
      why_it_matters: '这是一条还在跑的跟踪',
      next_safe_action: '可以只跳过这一次，或关闭整条。',
      urgency: 'normal',
      lineage: [
        {
          layer: 'intent',
          kind: 'task',
          id: 'task:12',
          label: '循环跟踪',
          label_code: 'intent.recurring',
          detail: 'task:12 · interval_seconds=3600 · group_key=doorbell',
          action_ids: ['task.skip_occurrence', 'tracking.cancel'],
        },
        {
          layer: 'origin',
          kind: 'context',
          id: 'task:12:origin',
          label: '当时的说明',
          label_code: 'origin.context',
          detail: 'context_text=周末给门铃充电',
          action_ids: [],
        },
      ],
      contextual_actions: [
        {
          id: 'task.skip_occurrence',
          kind: 'task.skip_occurrence',
          label: '只结束这一次',
          requires_confirmation: true,
          requires_data_gate: false,
          requires_mfa: false,
          route: '/webapp/vnext/work/task:12/actions',
          expected_receipt: 'skip receipt',
        },
        {
          id: 'tracking.cancel',
          kind: 'tracking.cancel',
          label: '关闭整条跟踪',
          requires_confirmation: true,
          requires_data_gate: false,
          requires_mfa: false,
          route: '/webapp/vnext/work/task:12/actions',
          expected_receipt: 'cancel receipt',
        },
      ],
    });
    item.authority.requires_data_grant = false;
    lane.items = phase === 'closed' ? [] : [item];
    payload.counts.follow_up = lane.items.length;
    await route.fulfill({ json: payload });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 30000 });
    await work.click();
    // The first open card is already selected. A second tap toggles it shut
    // (selectItem toggle), which hides the source chain.
    const rowHost = () => page.locator('.vnext-work-mobile-inline-item').filter({ hasText: '门铃电量' }).first();
    const row = () => rowHost().locator('.vnext-item-row').first();
    await row().waitFor({ state: 'visible' });
    if (!(await rowHost().evaluate((el) => el.classList.contains('active')))) await row().click();
    const interval = page.locator('.vnext-source-evidence-detail').filter({ hasText: 'interval_seconds=3600' }).first();
    await interval.scrollIntoViewIfNeeded();
    await interval.waitFor({ state: 'visible' });
    await page.locator('.vnext-source-evidence-detail').filter({ hasText: 'context_text=周末给门铃充电' }).first().waitFor({ state: 'visible' });
    if (await page.getByText('来源', { exact: true }).filter({ visible: true }).count()) ok('source chain heading is visible');
    else fail('source chain heading missing');
    await shot('00-lineage');
    const skip = () => page.getByRole('button', { name: /^(只结束这一次|End only this time)$/ }).filter({ visible: true }).first();
    const close = () => page.getByRole('button', { name: /^(关闭整条跟踪|Close this tracking)$/ }).filter({ visible: true }).first();
    const confirm = () => page.getByRole('button', { name: /^(确认？|Confirm\?)$/ }).filter({ visible: true }).first();
    await skip().click();
    if (seen.length === 0) ok('skip asks before it mutates');
    else fail('skip mutated on the first click');
    await confirm().click();
    await page.getByText('这一次已跳过。跟踪还在，下次还会出现。', { exact: true }).filter({ visible: true }).first().waitFor({ state: 'visible' });
    await row().waitFor({ state: 'visible' });
    if (seen[0] === 'task.skip_occurrence' && phase === 'skipped') ok('skip keeps the tracking card');
    else fail('skip did not keep the card');
    await shot('01-skipped');
    await close().click();
    await confirm().click();
    await page.getByText('整条跟踪已关闭。记录还在。', { exact: true }).filter({ visible: true }).first().waitFor({ state: 'visible' });
    await page.getByText('门铃电量', { exact: true }).first().waitFor({ state: 'hidden' });
    if (seen[1] === 'tracking.cancel' && phase === 'closed' && await close().count() === 0) ok('close tracking retires the card');
    else fail('close tracking did not retire the card');
    await shot('02-closed');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute(pattern);
  }
}
