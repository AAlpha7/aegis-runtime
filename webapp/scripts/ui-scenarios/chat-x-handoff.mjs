export const id = 'chat-x-handoff';
export const title = 'X handoff is one open button and one draft reply';
export const surfaces = [
  'webapp/src/vnext/AssistantResponseView.tsx',
  'webapp/src/vnext/xDeepLink.ts',
  'webapp/src/vnext/styles.css',
];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const now = Math.floor(Date.now() / 1000);
  await page.route(pattern, (route) => route.fulfill({
    json: {
      schema_version: 1,
      has_more: false,
      messages: [
        { role: 'owner', text: '这条帖子能用吗', timestamp: now - 20 },
        {
          role: 'aegis',
          text: '结论：帖里的接口不是正式 API。',
          timestamp: now - 10,
          source_evidence: [
            {
              kind: 'tool',
              label: 'X',
              summary: '同一条帖子',
              detail: 'https://x.com/alice/status/111?s=20 https://twitter.com/alice/status/111 https://x.com/i/web/status/111',
            },
            {
              kind: 'tool',
              label: '另一条',
              summary: '另一条帖子',
              detail: 'https://x.com/bob/status/222',
            },
          ],
        },
      ],
    },
  }));
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const strip = page.locator('.vnext-x-open-strip');
    await strip.waitFor({ state: 'visible', timeout: 30000 });
    const openCount = await strip.locator('button.vnext-x-open-btn:not(.is-secondary)').count();
    const draftCount = await strip.locator('button.vnext-x-open-btn.is-secondary').count();
    const extraCount = await strip.locator('button.vnext-x-open-extra').count();
    if (openCount === 1) ok('one Open in X button');
    else fail('open button count', String(openCount));
    if (draftCount === 1) ok('one draft reply button');
    else fail('draft button count', String(draftCount));
    if (extraCount === 1) ok('the other post is a captioned link, not another Open in X');
    else fail('extra link count', String(extraCount));
    const labels = await strip.locator('button.vnext-x-open-btn').allInnerTexts();
    if (labels.length === 2 && labels[0] !== labels[1]) ok('open and draft labels differ');
    else fail('handoff labels', JSON.stringify(labels));
    const oneRow = await strip.locator('.vnext-x-open-strip-actions').evaluate((el) => el.getBoundingClientRect().height < 56);
    if (oneRow) ok('open and draft stay on one row');
    else fail('handoff actions wrapped onto a second row');
    await page.evaluate(() => {
      window.__opened = [];
      window.open = (url) => {
        window.__opened.push(String(url));
        return null;
      };
    });
    await strip.locator('button.vnext-x-open-btn:not(.is-secondary)').click();
    await strip.locator('button.vnext-x-open-btn.is-secondary').click();
    await strip.locator('button.vnext-x-open-extra').click();
    const opened = await page.evaluate(() => window.__opened);
    if (opened[0] === 'https://x.com/alice/status/111') ok('Open in X uses the deduped post');
    else fail('open target', JSON.stringify(opened));
    if (typeof opened[1] === 'string' && opened[1].includes('in_reply_to=111')) ok('draft reply targets that same post');
    else fail('draft target', JSON.stringify(opened));
    if (opened[2] === 'https://x.com/bob/status/222') ok('the other post opens from its own caption');
    else fail('extra target', JSON.stringify(opened));
    await strip.scrollIntoViewIfNeeded();
    await shot('01-single-handoff-row');
  } finally {
    await page.unroute(pattern);
  }
}
