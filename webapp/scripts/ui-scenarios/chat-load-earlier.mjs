// Chat home: the server window is 24 messages; older turns are reachable
// with 加载更早 (owner walk 2026-09-11: 70 seeded turns, only 59–70 visible,
// no way back). The button pages with ?before= and keeps the owner's place.
export const id = 'chat-load-earlier';
export const title = 'Chat home: 加载更早 pages older history in place';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/api.ts',
  'webapp/src/apiGuards.ts',
  'core/src/admin/vnext_projections.rs',
  'core/src/admin/vnext_routes.rs',
];

const articles = (page) => page.locator('.vnext-chat-stream article.vnext-chat-message');

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(700);
  const before = await articles(page).count();
  const btn = page.locator('[data-testid="aegis-chat-load-earlier"]');
  if (await btn.count()) ok('加载更早 offered on a long history', `${before} messages shown`);
  else { fail('加载更早 missing on a 40-turn history', `${before} messages shown`); return; }

  await page.evaluate(() => { const s = document.querySelector('.vnext-chat-stream'); if (s) s.scrollTop = 0; });
  await page.waitForTimeout(300);
  const firstBefore = (await articles(page).first().innerText().catch(() => '')).replace(/\s+/g, ' ');
  const wait = page.waitForResponse((r) => r.url().includes('/webapp/vnext/chat-history?before='), { timeout: 10000 }).catch(() => null);
  await btn.click();
  const res = await wait;
  await page.waitForTimeout(700);
  await shot('01-after-load-earlier');
  if (res && res.status() === 200) ok('paged request reached the server', res.url().replace(/^.*\/webapp/, '/webapp'));
  else fail('no ?before= request', res ? String(res.status()) : 'none');
  const after = await articles(page).count();
  if (after > before) ok('older turns prepended', `${before} → ${after}`);
  else fail('message count did not grow', `${before} → ${after}`);
  const firstAfter = (await articles(page).first().innerText().catch(() => '')).replace(/\s+/g, ' ');
  if (firstAfter && firstAfter !== firstBefore) ok('oldest visible turn is now older', firstAfter.slice(0, 60));
  else fail('first turn unchanged after loading earlier', firstAfter.slice(0, 60));
  const anchor = await page.evaluate((text) => {
    const s = document.querySelector('.vnext-chat-stream');
    const el = [...document.querySelectorAll('.vnext-chat-stream article.vnext-chat-message')].find((a) => a.innerText.replace(/\s+/g, ' ').startsWith(text));
    if (!s || !el) return null;
    const r = el.getBoundingClientRect(); const sr = s.getBoundingClientRect();
    return { top: Math.round(r.top - sr.top), height: Math.round(sr.height) };
  }, firstBefore.slice(0, 40));
  if (anchor && anchor.top >= -20 && anchor.top <= anchor.height) ok('owner keeps their place after prepend', JSON.stringify(anchor));
  else fail('stream jumped after prepend', JSON.stringify(anchor));
}
