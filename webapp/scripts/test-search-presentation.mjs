// Real browser regression for answer completion, source links and mobile layout.
// Run after npm run build: node scripts/test-search-presentation.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, readdir, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium } from 'playwright';

const web = fileURLToPath(new URL('..', import.meta.url));
const output = resolve(web, '../target/search-ui');
await mkdir(output, { recursive: true });
const assets = await readdir(resolve(web, 'dist/assets'));
const css = (await Promise.all(assets.filter(name => name.endsWith('.css')).map(name => readFile(resolve(web, 'dist/assets', name), 'utf8')))).join('\n');
const bundle = await build({
  stdin: { resolveDir: web, loader: 'tsx', contents: `
    import React from 'react';
    import { createRoot } from 'react-dom/client';
    import { AssistantResponseView, LocaleProvider } from './src/vnext/assistantResponseViewTestEntry';
    const root = createRoot(document.getElementById('root'));
    window.showAnswer = (result) => root.render(<LocaleProvider displayLanguage="zh"><AssistantResponseView result={result}/></LocaleProvider>);
  ` }, bundle: true, write: false, format: 'iife', define: { 'import.meta.env': '{}' },
});
const server = createServer((req, res) => {
  if (req.url === '/app.js') { res.setHeader('Content-Type', 'text/javascript'); res.end(bundle.outputFiles[0].text); return; }
  res.setHeader('Content-Type', 'text/html; charset=utf-8');
  res.end(`<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><style>${css}
    body { margin:0; } main.vnext-app { display:block; padding:16px; min-height:100vh; background:var(--b-bg); color:var(--b-text); }
    article { padding:16px; border:1px solid var(--b-hairline); border-radius:18px; background:var(--b-surface); }
    h1 { font-size:18px; margin:0 0 20px; }
  </style><main class="vnext-app vnext-app-chat-home"><article><h1>安琪</h1><div id="root"></div></article></main><script src="/app.js"></script>`);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true });
const body = 'Tailscale Serve 可以将本机服务通过 HTTPS 分享给同一私有网络中的设备。\n\n1. Tailscale Serve 总览\nhttps://tailscale.com/docs/features/tailscale-serve\n\n2. 命令参考\nhttps://tailscale.com/docs/reference/tailscale-cli/serve';
const base = { response_kind:'answer', visible_answer:body, receipt_expectation:'', citations:[], warnings:[] };
try {
  for (const [width, colorScheme] of [[390, 'light'], [320, 'dark']]) {
    const page = await browser.newPage({ viewport:{width,height:844}, colorScheme });
    const errors=[];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => typeof window.showAnswer === 'function');
    await page.evaluate(result => window.showAnswer(result), base);
    const answer = page.locator('.tone-answer');
    await answer.waitFor();
    const before = await answer.boundingBox();
    const receipt={queries:['内部搜索提示不应显示'],services:[{source:'codex_subscription',fallback_used:false}],sources:[{title:'Tailscale Serve 总览',url:'https://tailscale.com/docs/features/tailscale-serve',excerpt:'Tailscale 官方文档'}]};
    await page.evaluate(result => window.showAnswer(result), {...base, evidence:[{id:'E1',tool:'web_search',label:'Web search',excerpt:'Tailscale 官方文档\nhttps://tailscale.com/docs/features/tailscale-serve',full_len:90}], visible_answer:body+'\n搜索收据\n查了：内部搜索提示不应显示\n@@search-receipt\n'+JSON.stringify(receipt)+'\n@@end-search-receipt'});
    await page.locator('[data-testid="search-receipt"]').waitFor({state:'attached'});
    const after=await answer.boundingBox();
    assert.equal(after.y,before.y,'completion must not move an existing answer below a receipt');
    assert.equal(await page.locator('.vnext-answer-more').count(),0,'references must stay together');
    assert.equal(await page.locator('.vnext-reference-list a').count(),2);
    assert(!await page.getByText('内部搜索提示不应显示').count());
    assert.equal(await page.locator('.tone-evidence > summary').count(),1,'one consolidated source panel');
    await page.locator('.tone-evidence > summary').click();
    assert(await page.locator('[data-testid="search-receipt"] a').isVisible());
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),'no mobile horizontal overflow');
    await page.screenshot({path:resolve(output,`success-${width}-${colorScheme}.png`),fullPage:true});
    const excerpt='Fallback attempted: false\nReason: codex_search_failed\nSource: codex_subscription\nStatus: error';
    await page.evaluate(result => window.showAnswer(result), {...base,visible_answer:'本次搜索未成功，暂时无法核实最新资料。', evidence:[{id:'E1',tool:'web_search',label:'Web search',excerpt,full_len:excerpt.length}],source_evidence:[{kind:'tool',label:'Web search',summary:'Tool returned status: error',detail:'Current evidence:\nTool returned status: error'}]});
    if (!await page.getByText('未取得可用搜索来源',{exact:true}).isVisible()) await page.getByText('检索状态',{exact:true}).click();
    assert(await page.getByText('未取得可用搜索来源',{exact:true}).isVisible());
    assert(!await page.getByText(/Fallback attempted/).isVisible());
    assert.equal(await page.locator('.vnext-search-diagnostics[open]').count(),0);
    assert.equal(await page.locator('.vnext-source-evidence-row').count(),1);
    await page.screenshot({path:resolve(output,`failure-${width}-${colorScheme}.png`),fullPage:true});
    assert.deepEqual(errors,[]);
    await page.close();
    console.log(`PASS: ${width}px ${colorScheme}: stable answer, linked titles, folded diagnostics, no overflow`);
  }
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
