export const id='chat-history-retry';
export const title='Failed older-history loading explains the failure and recovers in place';
export const surfaces=['webapp/src/vnext/App.tsx','webapp/src/vnext/ChatHomeView.tsx','webapp/src/i18n/catalog.ts','webapp/src/vnext/design/surfaces/chat-home.css'];
export async function run({page,ok,fail,shot}) {
  const pattern='**/webapp/vnext/chat-history*';
  const now=Math.floor(Date.now()/1000);
  let failPage=true;
  await page.route(pattern,route=>{
    const older=new URL(route.request().url()).searchParams.has('before');
    if(older&&failPage)return route.fulfill({status:503,json:{error:'temporary_history_failure'}});
    return route.fulfill({json:{schema_version:1,has_more:!older,messages:older
      ?[{role:'owner',text:'Older preserved question',timestamp:now-200},{role:'aegis',text:'Older preserved answer',timestamp:now-190}]
      :[{role:'owner',text:'Recent preserved question',timestamp:now-20},{role:'aegis',text:'Recent preserved answer',timestamp:now-10}]}});
  });
  try {
    await page.goto('/ui/',{waitUntil:'domcontentloaded'});
    const button=page.getByTestId('aegis-chat-load-earlier');
    await button.waitFor({state:'visible'});
    await button.click();
    const error=page.getByTestId('aegis-chat-earlier-error');
    await error.waitFor({state:'visible'});
    if (await page.locator('.vnext-chat-composer textarea').evaluate(el => el.scrollHeight <= el.clientHeight + 2)) ok('empty composer placeholder fits one line without clipping');
    else fail('narrow composer placeholder wraps out of view');
    if(await page.getByText('Recent preserved answer',{exact:true}).count()===1 && await button.isEnabled())ok('failure retains current history and enables retry');
    else fail('history lost or retry disabled');
    await shot('01-retry-feedback');
    failPage=false;
    await button.click();
    await page.getByText('Older preserved answer',{exact:true}).waitFor({state:'visible'});
    await error.waitFor({state:'hidden'});
    if(await page.getByText('Recent preserved answer',{exact:true}).count()===1)ok('retry prepends old history without duplicating current messages');
    else fail('current message duplicated');
    await button.waitFor({state:'hidden'});
    ok('successful pagination clears failure and removes exhausted control');
    await shot('02-recovered');
  } finally {await page.unroute(pattern);}
}
