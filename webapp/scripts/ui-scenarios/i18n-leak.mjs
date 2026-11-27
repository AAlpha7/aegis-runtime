// Permanent scenario: the Chinese console must not show raw English values
// that leaked from server tokens (status codes, labels, reasoning levels).
// Owner walk 2026-09-11 found: 工作 row "Failed", timeline "Receipt", 状态
// "delegated codex_connector" / "Memory", evidence "Tasks and reminders",
// thinking depth "low/medium/high/xhigh". Brand and product words are allowed.
export const id = 'i18n-leak';
export const title = 'zh console shows no raw English tokens';
export const surfaces = [
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/WorkItemRow.tsx',
  'webapp/src/vnext/StatusView.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/vnext/AssistantResponseView.tsx',
  'webapp/src/vnext/detail.ts',
  'webapp/src/vnext/model.ts',
  'webapp/src/vnext/presentation.ts',
  'webapp/src/i18n/catalog.ts',
];

const ALLOWED = new Set([
  'aegis', 'hercules', 'grok', 'codex', 'openrouter', 'telegram', 'google', 'gmail', 'web', 'api', 'e2e',
  'x', 'ios', 'pwa', 'utc', 'ok', 'id', 'url', 'ai', 'app', 'evidence', 'reply', 'english', 'push',
  'drive', 'calendar', 'chrome', 'safari', 'mcp', 'oauth', 'json', 'http', 'https', 'llm', 'vapid',
]);
// Tokens the walk actually saw leaking; a scan of every visible text node
// catches new ones of the same shape (a bare lowercase English word).
const KNOWN_LEAKS = /\b(Failed|Receipt|delegated|codex_connector|Tasks and reminders|xhigh)\b/;

async function scan(page) {
  return page.evaluate((allowedList) => {
    const allowed = new Set(allowedList);
    const out = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      const text = (node.textContent || '').replace(/\s+/g, ' ').trim();
      if (!text) continue;
      const el = node.parentElement;
      if (!el) continue;
      const style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden') continue;
      // Chat bubbles carry the owner's and the model's own words (an English
      // "Thanks" is not a leak); only fixed UI copy is scanned.
      if (el.closest('.vnext-sr-live, [aria-hidden="true"], code, pre, .vnext-version, [data-i18n-raw], .vnext-chat-message, .vnext-chat-answer-card')) continue;
      // Version stamps, model slugs, emails, urls: never flagged.
      if (/^v\d{8}\.|\/|@|\.(?:com|me|io)\b|^\d/.test(text)) continue;
      // A bare English word or snake_case token standing alone in a zh UI.
      if (/^[A-Za-z]+(?:_[A-Za-z0-9]+)*$/.test(text) && !allowed.has(text.toLowerCase())) {
        out.push({ text, at: (el.className || el.tagName || '').toString().slice(0, 60) });
      }
    }
    return out;
  }, [...ALLOWED]);
}

async function check(page, name, ok, fail, shot) {
  await page.waitForTimeout(300);
  await shot(name);
  const leaks = await scan(page);
  const body = await page.evaluate(() => document.body.innerText);
  const known = body.match(KNOWN_LEAKS);
  if (leaks.length === 0 && !known) ok(`${name}: no raw English tokens`);
  else fail(`${name}: raw English tokens`, JSON.stringify({ known: known?.[0] ?? null, bare: leaks.slice(0, 8) }));
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/?view=work', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(500);
  const row = page.locator('button.vnext-item-row').first();
  if (await row.count()) { await row.click().catch(() => {}); await page.waitForTimeout(400); }
  await page.evaluate(() => { document.querySelectorAll('details').forEach((d) => { d.open = true; }); });
  await check(page, '01-work-expanded', ok, fail, shot);

  await page.goto('/ui/?view=status', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(500);
  await page.evaluate(() => { document.querySelectorAll('details').forEach((d) => { d.open = true; }); });
  await check(page, '02-status-expanded', ok, fail, shot);

  await page.goto('/ui/?view=settings', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(500);
  await page.evaluate(() => { document.querySelectorAll('details').forEach((d) => { d.open = true; }); });
  await check(page, '03-settings-expanded', ok, fail, shot);

  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(500);
  await page.evaluate(() => { document.querySelectorAll('details').forEach((d) => { d.open = true; }); });
  await check(page, '04-home-expanded', ok, fail, shot);
}
