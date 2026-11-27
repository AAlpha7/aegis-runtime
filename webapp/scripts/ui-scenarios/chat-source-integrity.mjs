export const id = 'chat-source-integrity';
export const title = 'Sources open once, stay with their reply and retain complete facts';
export const surfaces = ['webapp/src/vnext/AssistantResponseView.tsx', 'webapp/src/vnext/sourceEvidenceText.ts', 'webapp/src/vnext/design/surfaces/rich-text.css', 'core/src/evidence.rs'];

export async function run({page, ok, fail, shot}) {
  const pattern = '**/webapp/vnext/chat-history*';
  const now = Math.floor(Date.now() / 1000);
  const code = ['if ready:', '    print("No external action was taken.")', ...Array.from({length: 8}, (_, i) => `    print(${i})`)].join('\n');
  const evidence = (summary) => [{id:'E1',tool:'web_search',label:'Web search',full_len:200,
    excerpt:JSON.stringify({summary,url:'https://example.com/source',status:'ok',count:0})}];
  await page.route(pattern, route => route.fulfill({json:{schema_version:1,has_more:false,messages:[
    {role:'owner',text:'First evidence question',timestamp:now-40},
    {role:'aegis',text:'First evidence answer [E1]',timestamp:now-30,evidence:[{...evidence('')[0],excerpt:'',full_len:2}]},
    {role:'owner',text:'Second evidence question',timestamp:now-20},
    {role:'aegis',text:`Second evidence answer [E1]\n\n\`\`\`python\n${code}\n\`\`\`\nAfter the code.`,timestamp:now-10,evidence:evidence('Second source fact')},
  ]}}));
  try {
    await page.goto('/ui/',{waitUntil:'domcontentloaded'});
    const second=page.locator('article.vnext-chat-message').filter({hasText:'Second evidence answer'});
    const first=page.locator('article.vnext-chat-message').filter({hasText:'First evidence answer'});
    const chip=second.locator('button[data-evidence-id="E1"]');
    await chip.waitFor({state:'visible'});
    await chip.click();
    const target=await chip.getAttribute('aria-controls');
    const scoped=second.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
    await scoped.waitFor({state:'visible'});
    if(await scoped.getAttribute('id')===target && await first.locator('.tone-evidence').evaluate(el=>!el.open)) ok('second E1 opens only the second reply source');
    else fail('citation crossed into another reply');
    const ids=await page.locator('.vnext-source-evidence-row[id]').evaluateAll(els=>els.map(el=>el.id));
    if(new Set(ids).size===ids.length && ids.length===2) ok('evidence IDs are unique across turns'); else fail('duplicate source IDs');
    if(await second.locator('.tone-evidence details').count()===0) ok('one disclosure exposes all source facts'); else fail('nested source accordion');
    const text=await scoped.innerText();
    if(text.includes('Second source fact') && text.includes('https://example.com/source') && /条目数: 0|Count: 0/.test(text) && !text.includes('"summary"')) ok('summary keeps sibling URL and zero count without raw JSON');
    else fail('source facts lost or raw JSON leaked',text);
    if(await scoped.locator('a[href="https://example.com/source"]').count()===1) ok('original source is a usable link'); else fail('source URL is not clickable');
    const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>window.innerWidth+1);
    if(!overflow) ok('source panel stays within the viewport'); else fail('horizontal page overflow');
    await shot('01-second-source');
    await second.locator('.tone-evidence > summary').click();
    await scoped.waitFor({state:'hidden'});
    // Native <details> changes visibility before its queued toggle event updates
    // React's citation state. Wait for that state, retaining a bounded failure.
    await second.locator('button[data-evidence-id="E1"][aria-expanded="false"]').waitFor({state:'attached',timeout:2000});
    if(await chip.getAttribute('aria-expanded')==='false') ok('citation state follows source collapse'); else fail('stale accessibility expansion state');
    await chip.click();
    await scoped.waitFor({state:'visible'});
    ok('source can be reopened from the same citation');
    await shot('02-reopened');
    const codeBlock = second.locator('pre code');
    if (await codeBlock.count() === 1 && await codeBlock.innerText() === code) ok('code stays complete and indented across answer preview');
    else fail('answer preview split or rewrote the code block');
    if (await codeBlock.evaluate(el => getComputedStyle(el).borderTopWidth === '0px' && getComputedStyle(el).display === 'block')) ok('code block avoids inline-code border fragments');
    else fail('code block inherited inline decoration');
    if (await codeBlock.evaluate(el => {
      const pre = el.parentElement;
      if (pre.scrollWidth <= pre.clientWidth) return true;
      pre.scrollLeft = 16;
      const scrollable = pre.scrollLeft > 0;
      pre.scrollLeft = 0;
      return scrollable;
    })) ok('long code lines remain horizontally scrollable');
    else fail('long code lines are clipped without scrolling');
    const more = second.locator('.vnext-answer-more > summary');
    if (await more.evaluate(el => getComputedStyle(el).color === getComputedStyle(el.closest('article').querySelector('.vnext-rich-text')).color)) ok('more-details control uses readable theme ink');
    else fail('more-details control uses unreadable legacy ink');
    await more.click();
    await second.getByText('After the code.', {exact:true}).waitFor({state:'visible'});
    ok('more-details control reveals the rest of the response');
    await codeBlock.scrollIntoViewIfNeeded();
    await shot('03-intact-code');
    await first.locator('button[data-evidence-id="E1"]').click();
    const emptySource = first.locator('.vnext-source-evidence-row[data-evidence-id="E1"]');
    await emptySource.waitFor({state:'visible'});
    if (/来源未返回详细内容|No source details returned/.test(await emptySource.innerText())) ok('legacy empty source remains navigable and explains missing details');
    else fail('empty source lost its citation or explanation');
    await shot('04-empty-source');
  } finally { await page.unroute(pattern); }
}
