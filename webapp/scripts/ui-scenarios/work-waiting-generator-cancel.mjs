import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-waiting-generator-cancel';
export const title = 'Waiting card spawned by a loop: end this card, and close the generator';
export const surfaces = [
  'core/src/admin/inbox_projection.rs',
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/model.ts',
  'webapp/src/vnext/App.tsx',
];

function waitingItem(generatorOpen) {
  const contextual = [
    {
      id: 'knowledge.done',
      kind: 'knowledge.done',
      label: '结束这一次',
      requires_confirmation: true,
      requires_data_gate: false,
      requires_mfa: false,
      route: '/webapp/vnext/work/kb_waiting:701/actions',
      expected_receipt: 'waiting receipt',
    },
    {
      id: 'knowledge.snooze_tomorrow',
      kind: 'knowledge.snooze_tomorrow',
      label: '明天再看',
      requires_confirmation: true,
      requires_data_gate: false,
      requires_mfa: false,
      route: '/webapp/vnext/work/kb_waiting:701/actions',
      expected_receipt: 'snooze receipt',
    },
  ];
  if (generatorOpen) {
    contextual.splice(1, 0, {
      id: 'tracking.cancel',
      kind: 'tracking.cancel',
      label: '关闭整条跟踪',
      requires_confirmation: true,
      requires_data_gate: false,
      requires_mfa: false,
      route: '/webapp/vnext/work/kb_waiting:701/actions',
      expected_receipt: 'cancel receipt',
    });
  }
  const lineage = [
    {
      layer: 'occurrence',
      kind: 'knowledge',
      id: 'kb_waiting:701',
      label: '这条等待',
      label_code: 'occurrence.waiting',
      detail: 'kb_waiting:701 · category=waiting_for · knowledge_id=701 · waiting_on=确认预约时间',
      action_ids: ['knowledge.done'],
    },
  ];
  if (generatorOpen) {
    lineage.push({
      layer: 'intent',
      kind: 'task',
      id: 'task:36',
      label: '循环跟踪',
      label_code: 'intent.recurring',
      detail: 'task:36 · interval_seconds=86400 · group_key=treatment-reminder',
      action_ids: ['tracking.cancel'],
    });
  }
  lineage.push({
    layer: 'origin',
    kind: 'record',
    id: 'kb_waiting:701:origin',
    label: '记录',
    label_code: 'origin.record',
    detail: 'created_at=2026-03-30T20:00:38Z',
    action_ids: [],
  });
  return {
    id: 'kb_waiting:701',
    source_id: '701',
    source: 'knowledge',
    source_kind: 'tracking',
    lane: 'follow_up',
    state: 'waiting',
    title: '治疗提醒',
    summary: '等确认预约时间。',
    why_it_matters: '这条等待来自一条还在跑的循环',
    next_safe_action: '可以结束这一张，或关掉整条循环。',
    urgency: 'normal',
    lineage,
    contextual_actions: contextual,
  };
}

export async function run({ page, ok, fail, shot }) {
  let generatorOpen = true;
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
        fail('invalid authenticated waiting action');
        await route.fulfill({ status: 400, json: { error: 'invalid waiting action' } });
        return;
      }
      if (body.action_id === 'tracking.cancel') {
        generatorOpen = false;
        await route.fulfill({ json: {
          schema_version: 1,
          generated_at: new Date().toISOString(),
          status: 'ok',
          item_id: 'kb_waiting:701',
          action_id: 'tracking.cancel',
          owner_message: '整条跟踪已关闭。记录还在。',
          receipt: { id: 'cancel-701', kind: 'action_receipt', label: 'task:36:cancelled', created_at: new Date().toISOString() },
          raw_result: { task_status: 'cancelled', resource_id: '36' },
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
    Object.assign(item, waitingItem(generatorOpen));
    item.authority.requires_data_grant = false;
    lane.items = [item];
    payload.counts.follow_up = 1;
    await route.fulfill({ json: payload });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 30000 });
    await work.click();
    const rowHost = () => page.locator('.vnext-work-mobile-inline-item').filter({ hasText: '治疗提醒' }).first();
    const row = () => rowHost().locator('.vnext-item-row').first();
    await row().waitFor({ state: 'visible' });
    if (!(await rowHost().evaluate((el) => el.classList.contains('active')))) await row().click();
    const interval = page.locator('.vnext-source-evidence-detail').filter({ hasText: 'interval_seconds=86400' }).first();
    await interval.scrollIntoViewIfNeeded();
    await interval.waitFor({ state: 'visible' });
    const endOnce = () => page.getByRole('button', { name: /^(结束这一次|End this card)$/ }).filter({ visible: true }).first();
    const close = () => page.getByRole('button', { name: /^(关闭整条跟踪|Close this tracking)$/ }).filter({ visible: true }).first();
    const confirm = () => page.getByRole('button', { name: /^(确认？|Confirm\?)$/ }).filter({ visible: true }).first();
    await endOnce().waitFor({ state: 'visible' });
    await close().waitFor({ state: 'visible' });
    if (await page.locator('.vnext-source-evidence-detail').filter({ hasText: 'waiting_on=确认预约时间' }).count()) {
      ok('waiting occurrence and generator are both visible');
    } else fail('waiting occurrence detail missing');
    await shot('00-both-actions');
    await close().click();
    if (seen.length === 0) ok('close tracking asks before it mutates');
    else fail('close tracking mutated on the first click');
    await confirm().click();
    await page.getByText('整条跟踪已关闭。记录还在。', { exact: true }).filter({ visible: true }).first().waitFor({ state: 'visible' });
    await endOnce().waitFor({ state: 'visible' });
    if (seen[0] === 'tracking.cancel' && !generatorOpen && await close().count() === 0) {
      ok('closing the generator leaves this waiting card');
    } else fail('waiting card did not keep end-this-card after the generator closed');
    await shot('01-generator-closed');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute(pattern);
  }
}
