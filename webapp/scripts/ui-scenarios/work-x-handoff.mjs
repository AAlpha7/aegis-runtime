// Work detail used to keep 「在 X 中打开」 in the dock after the inline answer
// already painted the same label. The answer strip owns that handoff.
import { readFileSync } from 'node:fs';
import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-x-handoff';
export const title = 'Work detail does not repeat Open in X once the answer strip is up';
export const surfaces = [
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/InlineAssistantPanel.tsx',
  'webapp/src/vnext/AssistantResponseView.tsx',
  'webapp/src/vnext/xDeepLink.ts',
];

const FIXTURE = JSON.parse(readFileSync(new URL('./fixtures/work-desktop-item.json', import.meta.url), 'utf8'));

function payload(base) {
  const body = injectedWorkPayload(base);
  const lane = body.lanes.find((entry) => entry.id === 'follow_up');
  const item = {
    ...FIXTURE.item,
    ...lane.items[0],
    id: 'x-handoff:alice',
    title: '帖子交接',
    summary: '同一条帖子 https://twitter.com/alice/status/111?s=20',
    external_url: 'https://twitter.com/alice/status/111?s=20',
  };
  item.authority = { ...item.authority, requires_data_grant: false };
  lane.items = [item];
  body.counts.follow_up = 1;
  return body;
}

export async function run({ page, ok, fail, shot }) {
  await page.route('**/webapp/vnext/work*', async (route) => {
    if (route.request().method() !== 'GET') {
      await route.fallback();
      return;
    }
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    await route.fulfill({ json: payload(base) });
  });
  await page.route('**/webapp/vnext/command', async (route) => {
    if (route.request().method() !== 'POST') {
      await route.fallback();
      return;
    }
    const body = route.request().postDataJSON();
    if (!String(body?.input || '').includes('这条帖子')) {
      await route.fallback();
      return;
    }
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
          visible_answer: '结论：不是正式接口。\nhttps://x.com/alice/status/111\nhttps://x.com/i/web/status/111',
          receipt_expectation: '没有对外发送。',
          citations: ['https://x.com/bob/status/222'],
          warnings: [],
          source_evidence: [{
            kind: 'tool',
            label: 'X',
            summary: '同一条帖子',
            detail: 'https://twitter.com/alice/status/111?s=20',
          }],
        },
      },
    });
  });

  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const work = page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first();
    await work.waitFor({ state: 'visible', timeout: 30000 });
    await work.click();
    const rowHost = () => page.locator('.vnext-work-mobile-inline-item').filter({ hasText: '帖子交接' }).first();
    const row = () => rowHost().locator('.vnext-item-row').first();
    await row().waitFor({ state: 'visible', timeout: 15000 });
    if (!(await rowHost().evaluate((el) => el.classList.contains('active')))) await row().click();

    const detail = () => rowHost().locator('.vnext-detail').first();
    await detail().waitFor({ state: 'visible', timeout: 15000 });
    const dockOpen = () => detail().locator('button.vnext-work-open-in-x');
    const dockDraft = () => detail().locator('button.vnext-work-draft-in-x');
    if (await dockOpen().count() === 1) ok('dock shows one Open in X before the answer');
    else fail('dock open count before answer', String(await dockOpen().count()));
    if (await detail().locator('.vnext-x-open-strip').count() === 0) ok('no answer strip before the answer');
    else fail('strip present before the answer');

    await detail().locator('button.vnext-work-ask-compact').click();
    const box = detail().locator('.vnext-inline-assistant textarea');
    await box.waitFor({ state: 'visible', timeout: 10000 });
    if (await dockOpen().count() === 1) ok('dock keeps Open in X while the owner is still asking');
    else fail('dock open count while asking', String(await dockOpen().count()));

    await box.fill('这条帖子能用吗');
    await detail().locator('.vnext-inline-assistant button[type="submit"]').click();
    const strip = detail().locator('.vnext-x-open-strip');
    await strip.waitFor({ state: 'visible', timeout: 20000 });
    if (await dockOpen().count() === 0 && await dockDraft().count() === 0) ok('dock drops Open in X and draft once the strip is up');
    else fail('dock still has X controls', `open=${await dockOpen().count()} draft=${await dockDraft().count()}`);
    const openCount = await strip.locator('button.vnext-x-open-btn:not(.is-secondary)').count();
    const draftCount = await strip.locator('button.vnext-x-open-btn.is-secondary').count();
    const extraCount = await strip.locator('button.vnext-x-open-extra').count();
    if (openCount === 1) ok('answer strip has one Open in X');
    else fail('strip open count', String(openCount));
    if (draftCount === 1) ok('answer strip has one draft reply');
    else fail('strip draft count', String(draftCount));
    if (extraCount === 1) ok('the other post is a caption, not a second Open in X');
    else fail('strip extra count', String(extraCount));
    const openText = await strip.locator('button.vnext-x-open-btn').allInnerTexts();
    const cloned = openText.filter((label) => label.trim() === '在 X 中打开').length;
    if (cloned === 1) ok('the label 在 X 中打开 appears once');
    else fail('cloned open labels', JSON.stringify(openText));
    await strip.scrollIntoViewIfNeeded();
    await shot('01-answer-owns-handoff');
  } finally {
    await page.unroute('**/webapp/vnext/work*');
    await page.unroute('**/webapp/vnext/command');
  }
}
