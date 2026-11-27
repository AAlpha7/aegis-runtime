export const id = 'chat-history-clear';
export const title = 'A late older-history response cannot restore a cleared conversation';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/ChatHomeView.tsx'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  const now = Math.floor(Date.now() / 1000);
  let releasePage;
  let pageStarted;
  let pageFinished;
  const started = new Promise(resolve => { pageStarted = resolve; });
  const finished = new Promise(resolve => { pageFinished = resolve; });
  const release = new Promise(resolve => { releasePage = resolve; });
  await page.route(pattern, async route => {
    const older = new URL(route.request().url()).searchParams.has('before');
    if (older) {
      pageStarted();
      await release;
    }
    await route.fulfill({ json: { schema_version: 1, has_more: !older, messages: [
      {role: 'owner', text: older ? 'Late older question' : 'Visible question before clearing', timestamp: now - (older ? 200 : 20)},
      {role: 'aegis', text: older ? 'Late older answer' : 'Visible answer before clearing', timestamp: now - (older ? 190 : 10)},
    ] } });
    if (older) pageFinished();
  });
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    await page.getByTestId('aegis-chat-load-earlier').click();
    await started;
    await page.getByRole('button', { name: /账户菜单|Account menu/ }).click();
    await page.getByRole('menuitem', { name: /清空对话|Clear chat/ }).click();
    await page.getByText('Visible answer before clearing', { exact: true }).waitFor({ state: 'hidden' });
    releasePage();
    await finished;
    // The network response must get a chance to update React before checking.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    if (await page.getByText('Late older answer', { exact: true }).count() === 0
      && await page.getByTestId('aegis-chat-load-earlier').count() === 0
      && await page.getByTestId('aegis-chat-earlier-error').count() === 0) {
      ok('late history response leaves the cleared conversation empty');
    } else fail('late history response restored cleared messages or stale controls');
    await shot('01-cleared-after-late-history');
  } finally {
    releasePage();
    await page.unroute(pattern);
  }
}
