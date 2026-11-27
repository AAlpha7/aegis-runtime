export const id = 'source-limitations';
export const title = 'Sources keep failures and partial-access warnings ahead of long descriptions';
export const surfaces = ['core/src/evidence.rs', 'webapp/src/vnext/sourceEvidenceText.ts'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const evidence = [
    { id: 'E1', tool: 'run_external', label: 'Mailbox lookup', full_len: 1800,
      excerpt: JSON.stringify({title: '邮箱查询说明。'.repeat(100), status: 'error',
        error: '没有权限读取邮箱，未取得未读数量。'}) },
    { id: 'E2', tool: 'run_external', label: 'Partial mailbox lookup', full_len: 1800,
      excerpt: JSON.stringify({summary: '已读取的邮件预览。'.repeat(100), status: 'partial',
        warnings: ['仅查询收件箱；其他标签权限不足，不能确认全部未读数量。']}) },
  ];
  await page.route(pattern, (route) => route.fulfill({ json: {
    schema_version: 1, has_more: false,
    messages: [{role: 'aegis', timestamp: Math.floor(Date.now() / 1000),
      text: '无法确认全部未读邮件数量：一次查询失败，另一次仅返回部分结果。[E1, E2]', evidence}],
  } }));
  try {
    for (const pass of ['initial', 'reload']) {
      await page.goto('/ui/', {waitUntil: 'domcontentloaded'});
      const first = page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
      await first.waitFor({state: 'attached', timeout: 30000});
      const disclosure = first.locator('xpath=ancestor::details').first();
      if (!await disclosure.evaluate((el) => el.open)) await disclosure.locator(':scope > summary').click();
      for (const [id, limitation, description] of [
        ['E1', '没有权限读取邮箱', '邮箱查询说明'],
        ['E2', '其他标签权限不足', '已读取的邮件预览'],
      ]) {
        const row = page.locator(`.vnext-source-evidence-row[data-evidence-id="${id}"]`);
        const text = await row.innerText();
        if (text.indexOf(limitation) >= 0 && text.indexOf(limitation) < text.indexOf(description)
            && text.slice(0, 600).includes(limitation)) ok(`${pass} ${id} limitation precedes description`);
        else fail(`${pass} ${id} limitation buried`, text.slice(0, 180));
        if (id === 'E2') {
          if (/部分结果|Partial results/.test(text)) ok(`${pass} partial status is explicit`);
          else fail(`${pass} partial status missing`);
        }
        // These rows exceed the viewport. Capture the limitation itself, not
        // the row's tail or an intermediate frame of the page's smooth scroll.
        const line = row.locator('p').filter({hasText: limitation}).first();
        const height = await page.evaluate(() => window.innerHeight);
        let box;
        let visible = false;
        // History hydration can still settle the timeline after reload. Reposition
        // for up to two seconds; retain the same visibility requirement.
        for (let attempt = 0; attempt < 20; attempt += 1) {
          await line.evaluate((el) => el.scrollIntoView({block: 'center', behavior: 'instant'}));
          await page.waitForTimeout(100);
          box = await line.boundingBox();
          visible = Boolean(box && box.y >= 60 && box.y + box.height <= height - 150);
          if (visible) break;
        }
        if (visible) ok(`${pass} ${id} limitation is in screenshot viewport`);
        else fail(`${pass} ${id} limitation outside screenshot viewport`, JSON.stringify(box));
        await shot(`${pass}-${id}-limitation`);
      }
    }
  } finally {
    await page.unroute(pattern);
  }
}
