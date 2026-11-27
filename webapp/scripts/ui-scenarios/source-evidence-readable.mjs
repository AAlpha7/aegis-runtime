export const id = 'source-evidence-readable';
export const title = 'Sources: readable task facts and honest incomplete-history fallback';
export const surfaces = ['webapp/src/vnext/AssistantResponseView.tsx', 'webapp/src/vnext/sourceEvidenceText.ts', 'core/src/evidence.rs'];
export const evidence = [
  { id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders', full_len: 400,
    excerpt: JSON.stringify({ count: 2, status: 'ok', tasks: [
      { title: '联系家人', status: 'fired', id: 'task-42', group_key: 'internal-routing', trigger_at_unix: 1788750000 },
      { title: '准备材料', status: 'pending', due_date: '2026-12-01', body: '携带原件和复印件。' },
    ] }) },
  { id: 'E3', tool: 'manage_tasks', label: 'Tasks and reminders', full_len: 250,
    excerpt: JSON.stringify({error:'Invalid schedule parameters / 定时参数无效',error_kind:'validation_error',hint:'Check trigger_type, trigger_value, and interval format.',retryable:false,status:'error'}) },
  { id: 'E2', tool: 'manage_tasks', label: 'Tasks and reminders', full_len: 900,
    excerpt: '{"tasks":[{"title":"clipped' },
];
export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  await page.route(pattern, (route) => route.fulfill({ json: {
    schema_version: 1, has_more: false, messages: [{ role: 'aegis', timestamp: Math.floor(Date.now() / 1000),
      text: '找到两条记录。[E1] 另一条历史来源需要重新查询。[E2] 定时任务未创建。[E3]', evidence }],
  } }));
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    await page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]').waitFor({ state: 'attached', timeout: 30000 });
    // Open the sources disclosure through its visible summary, with all source facts immediately visible.
    const source = page.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
    const outer = source.locator('xpath=ancestor::details').first();
    if (await outer.count() && !await outer.evaluate((el) => el.open)) {
      await outer.locator(':scope > summary').click();
    }
    const text = await page.locator('.vnext-source-evidence-list').innerText();
    if (/联系家人/.test(text) && /准备材料/.test(text) && /已触发|Triggered/.test(text)) ok('task facts visible');
    else fail('task facts missing', text);
    if (!/group_key|\{"tasks"/.test(text) && /internal-routing/.test(text) && /task-42/.test(text)) ok('record id and group are labeled, raw keys stay hidden');
    else fail('record id or group is missing, or a raw key leaked', text);
    if (/历史来源的内容不完整|saved source is incomplete/.test(text)) ok('incomplete history explained');
    else fail('missing incomplete-history feedback', text);
    if (/任务未创建/.test(text) && /请确认开始时间/.test(text) && !/trigger_value|validation_error/.test(text)) ok('schedule error is actionable Chinese');
    else fail('schedule error is not readable', text);
    await page.locator('.vnext-source-evidence-row[data-evidence-id="E3"]').scrollIntoViewIfNeeded();
    await shot('01-readable-sources');
  } finally {
    await page.unroute(pattern);
  }
}
