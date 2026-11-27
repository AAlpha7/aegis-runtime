export const id = 'chat-turn-identity';
export const title = 'One reply per command despite changed citations, clipped history, and reload';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/vnext/chatHistory.ts', 'core/src/admin/vnext_routes.rs', 'core/src/admin/vnext_projections.rs'];

export async function run({ page, ok, fail, shot }) {
  const historyPattern = '**/webapp/vnext/chat-history*';
  const commandPattern = '**/webapp/vnext/command';
  const cases = [
    { name: 'citations', question: '核对明天天气', marker: '天况核对', raw: '天况核对：明天晴。[E1,E2]', final: '天况核对：明天晴。 [E2]' },
    // Exceed the 4,000-character persisted-history cap while staying within
    // the command response contract's 4,096-character limit.
    { name: 'clipped', question: '给我完整的核对报告', marker: '完整报告',
      raw: `完整报告：${'这是一条已核对的独立记录。'.repeat(310)}`.slice(0, 4000),
      final: `完整报告：${'这是一条已核对的独立记录。'.repeat(310)}完整结尾确认。 [E2]` },
  ];
  for (const [caseIndex, sample] of cases.entries()) {
    let release;
    let initialHistoryDelivered = false;
    let historyRequests = 0;
    const commandReady = new Promise(resolve => { release = resolve; });
    const turns = [];
    const excerpt = '已核对的测试来源：https://example.com/verified';
    const evidence = [{ id: 'E2', tool: 'web_search', label: 'Web search', excerpt, full_len: excerpt.length }];
    await page.route(historyPattern, async route => {
      await route.fulfill({json: {schema_version: 1, has_more: false, messages: turns.flatMap(turn => [
        {role: 'owner', text: sample.question, timestamp: turn.timestamp, turn_id: turn.id},
        {role: 'aegis', text: sample.raw, timestamp: turn.timestamp + 1, turn_id: turn.id, latency_ms: 33000,
          ...(turn.finished ? {evidence} : {})},
      ])}});
      initialHistoryDelivered = true;
      historyRequests += 1;
    });
    await page.route(commandPattern, async route => {
      const body = route.request().postDataJSON();
      const turn = {id: body.idempotency_key, timestamp: Math.floor(Date.now() / 1000), finished: false};
      turns.push(turn);
      if (turns.length === 1) await commandReady;
      turn.finished = true;
      await route.fulfill({json: {
        schema_version: 1, generated_at: new Date().toISOString(), outcome: 'answered', owner_message: sample.final,
        assistant_response: {schema_version: 1, response_kind: 'answer', visible_answer: sample.final,
          turn_id: turn.id, latency_ms: 33000, receipt_expectation: '', citations: ['tool:web_search'], warnings: [], evidence},
        work_item: null,
      }});
    });
    try {
      await page.goto('/ui/', {waitUntil: 'domcontentloaded'});
      const input = page.locator('textarea[aria-label]').first();
      await input.waitFor({state: 'visible'});
      for (let attempt = 0; attempt < 100 && !initialHistoryDelivered; attempt++) await page.waitForTimeout(50);
      if (!initialHistoryDelivered) throw new Error(`${sample.name}: initial history was not delivered`);
      // Use the actual clear flow: clearing storage beneath a live React tree
      // races its persistence effect and can contaminate the next case.
      if (caseIndex > 0) {
        await page.getByRole('button', {name: /账户菜单|Account menu/}).click();
        await page.getByRole('menuitem', {name: /清空对话|Clear chat/}).click();
      }
      await input.fill(sample.question);
      await input.press('Enter');
      const replies = page.locator('article').filter({hasText: sample.marker});
      await replies.first().waitFor({state: 'visible', timeout: 20000});
      if (await replies.count() === 1 && turns.length === 1 && turns[0].id) ok(`${sample.name}: pending command restores exactly one history reply with a command identity`);
      else fail(`${sample.name}: recovery did not restore one identified reply`);
      await shot(`${sample.name}-01-before-final`);
      const finalResponse = page.waitForResponse(response => response.url().endsWith('/webapp/vnext/command') && response.request().method() === 'POST');
      release();
      await (await finalResponse).finished();
      await replies.locator('.vnext-source-evidence-row[data-evidence-id="E2"]').first().waitFor({state: 'attached'});
      await page.waitForTimeout(100);
      if (await replies.count() === 1) ok(`${sample.name}: changed final answer updates the recovered bubble`);
      else fail(`${sample.name}: finalization duplicated the reply`, `reply count=${await replies.count()}`);
      if (sample.name === 'clipped') {
        if ((await replies.last().textContent()).includes('完整结尾确认')) ok('clipped: completion retains the full answer beyond the history limit');
        else fail('clipped: completion lost the end of the full answer');
      } else {
        if (await replies.first().locator('.vnext-evidence-ref[data-evidence-id="E2"]').count() === 1
          && !(await replies.first().innerText()).includes('E1')) ok('citations: the recovered bubble displays the finalized citation selection');
        else fail('citations: the recovered bubble still displays preliminary citations');
      }
      await replies.last().scrollIntoViewIfNeeded();
      await shot(`${sample.name}-02-after-final`);
      const beforeReload = historyRequests;
      await page.reload({waitUntil: 'domcontentloaded'});
      await input.waitFor({state: 'visible'});
      for (let attempt = 0; attempt < 100 && historyRequests === beforeReload; attempt++) await page.waitForTimeout(50);
      await page.waitForTimeout(100);
      if (historyRequests > beforeReload && await replies.count() === 1) ok(`${sample.name}: history after completion and reload keeps one reply`);
      else fail(`${sample.name}: reload duplicates or loses the reply`, `reply count=${await replies.count()}`);
      await input.fill(sample.question);
      await input.press('Enter');
      for (let attempt = 0; attempt < 100 && turns.length < 2; attempt++) await page.waitForTimeout(50);
      await page.waitForFunction(marker => [...document.querySelectorAll('article')].filter(el => el.textContent.includes(marker)).length >= 2, sample.marker);
      await page.waitForTimeout(100);
      if (turns.length === 2 && turns[0].id !== turns[1].id && await replies.count() === 2) ok(`${sample.name}: identical subsequent question has a distinct command and a second reply`);
      else fail(`${sample.name}: different turns were collapsed or duplicated`, `reply count=${await replies.count()}`);
      const beforeSecondReload = historyRequests;
      await page.reload({waitUntil: 'domcontentloaded'});
      await input.waitFor({state: 'visible'});
      for (let attempt = 0; attempt < 100 && historyRequests === beforeSecondReload; attempt++) await page.waitForTimeout(50);
      await page.waitForTimeout(100);
      const questions = page.locator('article').filter({hasText: sample.question});
      if (historyRequests > beforeSecondReload && await replies.count() === 2 && await questions.count() === 2) {
        ok(`${sample.name}: reload preserves both genuine turns exactly once`);
      } else fail(`${sample.name}: reload merged genuine turns or retained duplicates`, `owners=${await questions.count()}, replies=${await replies.count()}`);
      await replies.last().scrollIntoViewIfNeeded();
      await shot(`${sample.name}-03-two-real-turns`);
    } catch (error) {
      await shot(`${sample.name}-unexpected-error`);
      throw error;
    } finally {
      release();
      await page.unroute(historyPattern);
      await page.unroute(commandPattern);
    }
  }
}
