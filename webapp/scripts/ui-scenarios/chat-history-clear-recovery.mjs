export const id = 'chat-history-clear-recovery';
export const title = 'Clear rejects delayed initial and recovery history without disturbing a new turn';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/chatHistory.ts'];

export async function run({ page, ok, fail, shot }) {
  const historyPattern = '**/webapp/vnext/chat-history*';
  const commandPattern = '**/webapp/vnext/command';
  await page.addInitScript(() => {
    localStorage.clear();
    sessionStorage.clear();
    sessionStorage.setItem('aegis-runtime:vnext:chat-home-history', JSON.stringify({
      schema_version: 1,
      messages: [{id: 'seed', role: 'owner', text: '清空前的本地问题', timestamp: Date.now() - 10000}],
    }));
  });
  for (const mode of ['initial', 'recovery']) {
    let releaseHistory;
    let releaseCommand;
    let heldHistory = false;
    let commandStarted = false;
    let delivered = 0;
    let initialDelivered = false;
    const historyReady = new Promise(resolve => { releaseHistory = resolve; });
    const commandReady = new Promise(resolve => { releaseCommand = resolve; });
    const staleQuestion = `清空前的问题 ${mode}`;
    const staleAnswer = `已清空的旧回答 ${mode}`;
    const freshQuestion = `清空后的新问题 ${mode}`;
    const freshAnswer = `保留的新回答 ${mode}`;
    const timestamp = Math.floor(Date.now() / 1000) - 30;
    await page.route(historyPattern, async route => {
      const hold = !heldHistory && (mode === 'initial' || commandStarted);
      if (hold) {
        heldHistory = true;
        await historyReady;
      }
      await route.fulfill({json: {schema_version: 1, has_more: hold, messages: hold ? [
        {role: 'owner', text: staleQuestion, timestamp},
        {role: 'aegis', text: staleAnswer, timestamp: timestamp + 1},
      ] : []}});
      if (hold) delivered += 1;
      else initialDelivered = true;
    });
    await page.route(commandPattern, async route => {
      const old = !commandStarted && mode === 'recovery';
      commandStarted = true;
      if (old) await commandReady;
      await route.fulfill({json: {
        schema_version: 1, generated_at: new Date().toISOString(), outcome: 'answered',
        owner_message: old ? staleAnswer : freshAnswer,
        assistant_response: {schema_version: 1, response_kind: 'answer', visible_answer: old ? staleAnswer : freshAnswer,
          receipt_expectation: '', citations: [], warnings: []}, work_item: null,
      }});
    });
    try {
      await page.goto('/ui/', {waitUntil: 'domcontentloaded'});
      const input = page.locator('textarea[aria-label]').first();
      await input.waitFor({state: 'visible'});
      if (mode === 'recovery') {
        for (let i = 0; i < 100 && !initialDelivered; i++) await page.waitForTimeout(50);
        if (!initialDelivered) throw new Error('initial history did not finish before recovery');
        await input.fill(staleQuestion);
        await input.press('Enter');
      }
      // Wait for the specific request to be held, not for a timing guess.
      for (let i = 0; i < 200 && !heldHistory; i++) await page.waitForTimeout(50);
      if (!heldHistory) throw new Error(`${mode} history request did not start`);
      await page.getByRole('button', {name: /账户菜单|Account menu/}).click();
      await page.getByRole('menuitem', {name: /清空对话|Clear chat/}).click();
      await page.getByText('清空前的本地问题', {exact: true}).waitFor({state: 'hidden'});
      await input.fill(freshQuestion);
      await input.press('Enter');
      await page.getByText(freshAnswer, {exact: true}).waitFor({state: 'visible'});
      ok(`${mode}: clear releases the composer for a new turn`);
      releaseHistory();
      releaseCommand();
      for (let i = 0; i < 100 && delivered === 0; i++) await page.waitForTimeout(50);
      if (!delivered) throw new Error(`${mode} deferred history was not delivered`);
      await page.waitForTimeout(250);
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      if (await page.getByText(staleAnswer, {exact: true}).count() === 0
        && await page.getByText(staleQuestion, {exact: true}).count() === 0
        && await page.getByTestId('aegis-chat-load-earlier').count() === 0) {
        ok(`${mode}: delayed history and completion cannot restore cleared messages or pagination`);
      } else fail(`${mode}: delayed history restored cleared messages or pagination`);
      if (await page.getByText(freshQuestion, {exact: true}).count() === 1
        && await page.getByText(freshAnswer, {exact: true}).count() === 1) ok(`${mode}: new turn remains visible exactly once`);
      else fail(`${mode}: new turn was lost or duplicated`);
      const persisted = await page.evaluate(() => [sessionStorage, localStorage].flatMap(storage =>
        Object.keys(storage).filter(key => key.includes('chat-home-history')).map(key => storage.getItem(key))).join('\n'));
      if (!persisted.includes(staleAnswer) && !persisted.includes(staleQuestion)) ok(`${mode}: stale history is not persisted`);
      else fail(`${mode}: cleared history returned to browser storage`);
      if (await input.evaluate(el => document.activeElement === el)) ok(`${mode}: delayed responses preserve input focus`);
      else fail(`${mode}: delayed responses stole input focus`);
      const newReply = await page.getByText(freshAnswer, {exact: true}).boundingBox();
      const composer = await page.locator('.vnext-chat-composer').boundingBox();
      if (newReply && composer && newReply.y >= 0 && newReply.y + newReply.height <= composer.y) {
        ok(`${mode}: latest answer stays visible above the composer`);
      } else fail(`${mode}: latest answer scrolled out of view`, JSON.stringify({newReply, composer}));
      await shot(`${mode}-after-clear-and-late-response`);
    } finally {
      releaseHistory();
      releaseCommand();
      await page.unrouteAll({behavior: 'wait'});
    }
  }
}
