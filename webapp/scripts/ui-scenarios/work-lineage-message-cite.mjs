import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-lineage-message-cite';
export const title = 'Work lineage chat message jumps to that cite';
export const surfaces = [
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/messageRef.ts',
  'webapp/src/vnext/App.tsx',
  'core/src/admin/inbox_projection.rs',
  'core/src/admin/vnext_projections.rs',
];

export async function run({ page, ok, fail, shot, viewport }) {
  const turnId = 'cite-turn-doorbell';
  const now = Math.floor(Date.now() / 1000);
  const history = {
    schema_version: 1,
    has_more: false,
    messages: [
      { role: 'owner', text: '周末提醒给门铃充电', timestamp: now - 40, turn_id: turnId },
      { role: 'aegis', text: '已设置循环跟踪门铃电量。', timestamp: now - 10, turn_id: turnId },
    ],
  };
  await page.route('**/api/session', (route) => route.fulfill({
    json: {
      authenticated: true,
      principal_type: 'web_console',
      email: 'owner@example.com',
      csrf_token: 'csrf-cite',
      assistant_name: '安琪',
    },
  }));
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({ json: history }));
  await page.route('**/webapp/vnext/work**', async (route) => {
    if (route.request().method() !== 'GET') {
      await route.fulfill({ status: 404, json: {} });
      return;
    }
    const payload = injectedWorkPayload({ generated_at: new Date().toISOString(), sources: [] });
    const lane = payload.lanes.find((entry) => entry.id === 'follow_up');
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
      next_safe_action: '可以打开当时的对话。',
      lineage: [
        {
          layer: 'origin',
          kind: 'message',
          id: 'l3:7',
          label: '对话消息',
          label_code: 'origin.message',
          detail: 'l3:7 · role=owner · 周末提醒给门铃充电',
          turn_id: turnId,
          message_role: 'owner',
          action_ids: [],
        },
        {
          layer: 'origin',
          kind: 'record',
          id: 'task:private:origin',
          label: '记录',
          label_code: 'origin.record',
          detail: 'source=user · created_at=2026-09-23T04:00:00Z',
          action_ids: [],
        },
      ],
    });
    lane.items = [item];
    payload.counts.follow_up = 1;
    await route.fulfill({ json: payload });
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 30000 });
    await work.click();
    const rowHost = () => page.locator('.vnext-work-mobile-inline-item').filter({ hasText: '门铃电量' }).first();
    const row = () => rowHost().locator('.vnext-item-row').first();
    await row().waitFor({ state: 'visible' });
    if (!(await rowHost().evaluate((el) => el.classList.contains('active')))) await row().click();
    const cite = page.locator('[data-lineage-cite]').filter({ visible: true }).first();
    await cite.waitFor({ state: 'visible', timeout: 15000 });
    const ref = await cite.getAttribute('data-lineage-cite');
    const token = (await cite.locator('.vnext-lineage-cite-token').innerText()).trim();
    if (ref && token === `#${ref}` && /^[0-9abcdefghjkmnpqrstvwxyz]{6}$/.test(ref)) ok('lineage shows the stable cite');
    else fail('lineage cite missing', `${token} / ${ref}`);
    const secret = await page.getByText('这是私密正文不应出现').count();
    if (secret === 0) ok('private transcript is not on the card');
    else fail('private transcript leaked onto the card');
    await cite.scrollIntoViewIfNeeded();
    const box = await cite.boundingBox();
    const width = viewport?.width ?? 390;
    const height = viewport?.height ?? 844;
    if (box && box.width >= 44 && box.height >= 44 && box.x >= 0 && box.x + box.width <= width + 1) {
      ok('cite control is a phone tap target');
    } else fail('cite control is outside the phone tap target', JSON.stringify(box));
    await shot('00-lineage-cite');
    await cite.click();
    const landed = page.locator('article.is-cite-target').filter({ has: page.locator(`[data-message-ref="${ref}"]`) }).first();
    await landed.waitFor({ state: 'visible', timeout: 15000 });
    const landedText = await landed.innerText();
    if (landedText.includes('周末提醒给门铃充电') && landedText.includes(token)) ok('chat scrolls to that message and shows the same cite');
    else fail('landed message does not show the excerpt and cite', landedText.slice(0, 240));
    const landedBox = await landed.boundingBox();
    if (landedBox && landedBox.y < height && landedBox.y + 24 > 0) ok('cited message is inside the phone viewport');
    else fail('cited message is not in the phone viewport', JSON.stringify(landedBox));
    await shot('01-chat-cite');
  } catch (error) {
    await shot('failure');
    throw error;
  } finally {
    await page.unroute('**/api/session');
    await page.unroute('**/webapp/vnext/chat-history*');
    await page.unroute('**/webapp/vnext/work**');
  }
}
