// 安琪: a thumb control back to the latest turn when the transcript is
// scrolled up, hidden at the bottom. Work: an already-open card does not
// say to open the detail, and 来源 reads as who / when / wording.
import { readFileSync } from 'node:fs';

export const id = 'chat-jump-and-opened-source';
export const title = 'Chat jump-to-latest and opened work card source line';
export const surfaces = [
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/styles.css',
  'webapp/src/i18n/catalog.ts',
  'webapp/scripts/ui-scenarios/chat-jump-and-opened-source.mjs',
];

const FIXTURE = JSON.parse(readFileSync(new URL('./fixtures/work-desktop-item.json', import.meta.url), 'utf8'));

const GIFT_TITLE = '准备太太生日礼物';
const WORDING = '🎁 准备 Hello Kitty 可爱风';
const WHEN = '2026-09-24 06:43:38';
const OPEN_DETAIL = '打开详情查看私人任务';

function historyPayload() {
  const now = Math.floor(Date.now() / 1000);
  return {
    schema_version: 1,
    has_more: false,
    messages: Array.from({ length: 16 }, (_, n) => ({
      role: n % 2 === 0 ? 'owner' : 'aegis',
      text: `第 ${n + 1} 条。${'生日礼物要可爱一点。'.repeat(8)}`,
      timestamp: now - (16 - n) * 60,
      turn_id: `jump-turn-${n}`,
    })),
  };
}

function giftItem() {
  return {
    ...FIXTURE.item,
    id: 'task:2142',
    lane: 'now',
    source_kind: 'task',
    source: 'tasks',
    source_id: '2142',
    title: GIFT_TITLE,
    summary: 'Task to review.',
    state: 'active',
    urgency: 'normal',
    next_safe_action: 'Review the task, then choose.',
    next_safe_action_code: 'task_open_or_act',
    blocked_by: null,
    lineage: [
      {
        layer: 'intent',
        kind: 'task',
        id: 'task:2142',
        label: '一次性安排',
        label_code: 'intent.oneshot',
        detail: '任务 2142',
        action_ids: [],
      },
      {
        layer: 'origin',
        kind: 'context',
        id: 'task:2142:origin',
        label: '当时的说明',
        label_code: 'origin.context',
        detail: `你 · ${WHEN} · ${WORDING}`,
        action_ids: [],
      },
    ],
  };
}

function workPayload(base) {
  const item = giftItem();
  const lanes = ['now', 'decide', 'follow_up', 'backlog', 'done'].map((id) => ({
    id,
    title: { now: '现在', decide: '待定', follow_up: '跟进', backlog: '待办', done: '已完成' }[id],
    subtitle: '',
    collapsed: id === 'backlog' || id === 'done',
    items: id === 'now' ? [item] : [],
  }));
  return {
    ...(base && typeof base === 'object' ? base : {}),
    schema_version: 1,
    generated_at: (base && base.generated_at) || new Date().toISOString(),
    sources: Array.isArray(base?.sources) ? base.sources : [],
    lanes,
    counts: { now: 1, decide: 0, follow_up: 0, backlog: 0, done: 0 },
  };
}

export async function run({ page, ok, fail, shot }) {
  await page.route('**/webapp/vnext/chat-history*', (route) => {
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(historyPayload()) });
  });
  await page.route('**/webapp/vnext/work*', async (route) => {
    if (route.request().method() !== 'GET') {
      await route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
      return;
    }
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(workPayload(base)),
    });
  });

  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav:visible, nav.vnext-rail-nav:visible').first().waitFor({ state: 'visible', timeout: 30000 });
  await page.locator('.vnext-chat-stream article.vnext-chat-message').first().waitFor({ state: 'visible', timeout: 15000 });
  await page.evaluate(() => {
    const stream = document.querySelector('.vnext-chat-stream');
    if (stream) stream.scrollTop = 0;
  });
  await page.waitForTimeout(200);

  const jump = page.locator('[data-testid="aegis-chat-jump-latest"]');
  if (await jump.count() !== 1) {
    fail('scrolled-up chat shows exactly one jump control', `count=${await jump.count()}`);
  } else {
    ok('scrolled-up chat shows exactly one jump control');
  }
  const label = ((await jump.innerText().catch(() => '')) || '').trim();
  if (label === '回到最新') ok('jump control label is 回到最新');
  else fail('jump control label is 回到最新', label);

  const place = await page.evaluate(() => {
    const button = document.querySelector('[data-testid="aegis-chat-jump-latest"]');
    const composer = document.querySelector('.vnext-chat-composer');
    const nav = document.querySelector('nav.vnext-nav');
    if (!button || !composer) return null;
    const b = button.getBoundingClientRect();
    const c = composer.getBoundingClientRect();
    const n = nav ? nav.getBoundingClientRect() : null;
    return {
      height: Math.round(b.height),
      bottom: Math.round(b.bottom),
      composerTop: Math.round(c.top),
      navTop: n ? Math.round(n.top) : null,
      viewport: window.innerHeight,
    };
  });
  if (!place) {
    fail('jump control is measurable above the composer');
  } else if (place.height >= 44 && place.bottom <= place.composerTop + 1 && (place.navTop == null || place.bottom <= place.navTop)) {
    ok('jump control is a thumb target above the composer and the tab bar', JSON.stringify(place));
  } else {
    fail('jump control is a thumb target above the composer and the tab bar', JSON.stringify(place));
  }
  await shot('01-jump-visible');

  await jump.click();
  await page.waitForTimeout(400);
  if (await jump.count() === 0) ok('jump control hides at the latest message');
  else {
    const scroll = await page.evaluate(() => [...document.querySelectorAll('.vnext-chat-stream')].map((stream) => ({
      top: Math.round(stream.scrollTop),
      height: stream.scrollHeight,
      client: stream.clientHeight,
      gap: stream.scrollHeight - stream.scrollTop - stream.clientHeight,
    })));
    fail('jump control hides at the latest message', `count=${await jump.count()} ${JSON.stringify(scroll)}`);
  }
  await shot('02-jump-hidden');

  const work = page.locator('nav.vnext-nav:visible button, nav.vnext-rail-nav:visible button').filter({ hasText: /工作|Work/ }).first();
  await work.click();
  const row = page.locator('.vnext-item-title:visible').filter({ hasText: GIFT_TITLE }).first();
  try {
    await row.waitFor({ state: 'visible', timeout: 8000 });
  } catch (err) {
    const body = await page.locator('body').innerText().catch(() => '');
    fail('birthday card is on the work list', body.replace(/\s+/g, ' ').slice(0, 700));
    await shot('03-work-missing');
    return;
  }
  const opened = page.locator('.vnext-work-inline-selected:visible, .vnext-nearby-inline-detail:visible').first();
  if (await opened.count() === 0) await row.click();
  await opened.waitFor({ state: 'visible', timeout: 10000 });
  const text = (await opened.innerText()).replace(/\s+/g, ' ');
  if (!text.includes(OPEN_DETAIL)) ok('opened card does not say to open the detail');
  else fail('opened card does not say to open the detail', text.slice(0, 240));
  if (text.includes('任务 2142') && text.includes('你') && text.includes(WHEN) && text.includes(WORDING)) {
    ok('source line keeps the task, the time, and the original wording');
  } else {
    fail('source line keeps the task, the time, and the original wording', text.slice(0, 400));
  }
  if (!text.includes('group_key=') && !text.includes('source=') && !text.includes('context_text=') && !text.includes('created_at=') && !text.includes('auto:准备太太生日礼物')) {
    ok('source line has no field names or generated slug');
  } else {
    fail('source line has no field names or generated slug', text.slice(0, 400));
  }
  await shot('03-opened-source');
}
