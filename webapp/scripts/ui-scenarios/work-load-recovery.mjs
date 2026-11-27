import { injectedWorkPayload } from './work-desktop-controls.mjs';
export const id = 'work-load-recovery';
export const title = 'Work load failures stay scoped and clear after successful refresh';
export const surfaces = ['webapp/src/vnext/App.tsx', 'webapp/src/i18n/catalog.ts', 'core/src/admin/console_handlers.rs'];

export async function run({page, ok, fail, shot}) {
  let recovered = false;
  const workPattern = '**/webapp/vnext/work';
  const commandPattern = '**/webapp/vnext/command';
  await page.route(workPattern, async (route) => {
    const payload = injectedWorkPayload({generated_at:new Date().toISOString(), sources:[]});
    const lane = payload.lanes.find(l => l.id === 'follow_up');
    const item = lane.items[0];
    item.title = '42 completed actions';
    item.summary = '42 completed actions remain in history';
    // Reproduce the old wire mismatch, then the bounded preview from the fixed server.
    item.related_work_item_ids = Array.from({length:recovered?32:42}, (_,i)=>`action:${i+1}`);
    item.related_receipt_ids = Array.from({length:recovered?32:42}, (_,i)=>`receipt:${i+1}`);
    await route.fulfill({json:payload});
  });
  await page.route(commandPattern, async (route) => {
    recovered = true;
    await route.fulfill({json:{schema_version:1, generated_at:new Date().toISOString(), outcome:'answered', owner_message:'Work refresh check complete', assistant_response:{schema_version:1,response_kind:'answer',visible_answer:'Work refresh check complete',receipt_expectation:'',citations:[],warnings:[]}, work_item:null}});
  });
  try {
    await page.goto('/ui/',{waitUntil:'domcontentloaded'});
    const notice = page.locator('.vnext-chat-inline-note').filter({hasText:/工作列表暂时无法加载|The Work list could not load/});
    await notice.waitFor({state:'visible'});
    if(await page.getByText(/无法加载 Aegis|Could not load Aegis/).count()===0) ok('failed Work projection is not reported as whole-app outage');
    else fail('misleading whole-app error');
    const input = page.locator('textarea[aria-label]').first();
    if(await input.isEnabled()) ok('chat remains usable while Work is unavailable');
    else fail('chat disabled');
    await shot('01-work-only-warning');
    await input.fill('Check Work refresh');
    await input.press('Enter');
    await page.getByText('Work refresh check complete',{exact:true}).first().waitFor({state:'visible'});
    await notice.waitFor({state:'hidden'});
    ok('successful queue refresh clears the old warning');
    await input.blur();
    await page.locator('nav.vnext-nav button').filter({hasText:/工作|Work/}).first().click();
    await page.getByText('42 completed actions',{exact:true}).filter({visible:true}).first().waitFor({state:'visible'});
    ok('group remains visible with its full count');
    await shot('02-work-restored');
  } finally {
    await page.unroute(workPattern);
    await page.unroute(commandPattern);
  }
}
