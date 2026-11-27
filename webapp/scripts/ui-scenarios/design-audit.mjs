// Scenario: design audit captures — phone screenshots of every owner surface
// (home, header menu, work list, expanded item, more-actions, nearby detail,
// settings, status) so a design review has real pixels to look at, plus one
// behavioural check the owner reported on 2026-09-16: opening a work item must
// keep the row's top edge where it was (expand downward), not re-scroll.
import { readFileSync } from 'node:fs';

export const id = 'design-audit';
export const title = 'Design audit captures: every owner surface on a phone and desktop; expand keeps the top edge; desktop panes scroll on their own';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/WorkItemRow.tsx',
  'webapp/src/vnext/styles.css',
  'webapp/src/vnext/design/surfaces/chrome.css',
  'webapp/scripts/ui-scenarios/design-audit.mjs',
];

const FIXTURE = JSON.parse(readFileSync(new URL('./fixtures/work-desktop-item.json', import.meta.url), 'utf8'));

function laneTitle(laneId) {
  return { now: '现在', decide: '待定', follow_up: '跟进', backlog: '待办', done: '已完成' }[laneId] || laneId;
}

function item(n, lane, title, summary) {
  return { ...FIXTURE.item, id: `${FIXTURE.item.id}:${lane}:${n}`, lane, title, summary };
}

/** A realistic queue: one urgent, one decision, two follow-ups, one backlog. */
export function auditWorkPayload(base) {
  const byLane = {
    now: [item(0, 'now', '回复部署 46 后的 Telegram 401 告警', '机器人 token 自 9 月 15 日起被拒绝，手机通知已停。')],
    decide: [item(0, 'decide', '选定开源许可证', '发布前必须有 LICENSE；候选 Apache-2.0 / MIT。')],
    follow_up: [
      item(0, 'follow_up', '在等用户确认需要恢复哪些误完成任务', '跟进用户确认需要恢复哪些误完成任务，推迟，或让我结束这件事。'),
      item(1, 'follow_up', '等 GitHub Discussions 开启后回填链接', '模板里的链接在开启前会 404。'),
    ],
    backlog: [item(0, 'backlog', '拆分剩余的大测试文件', 'orchestrator/tests/mod.rs 与 memory/tests/mod.rs 仍超过 5000 行。')],
    done: [],
  };
  const lanes = ['now', 'decide', 'follow_up', 'backlog', 'done'].map((laneId) => ({
    id: laneId,
    title: laneTitle(laneId),
    subtitle: '',
    collapsed: laneId === 'backlog' || laneId === 'done',
    items: byLane[laneId],
  }));
  const counts = Object.fromEntries(Object.entries(byLane).map(([k, v]) => [k, v.length]));
  return { ...(base && typeof base === 'object' ? base : {}), schema_version: FIXTURE.schema_version, lanes, counts };
}

async function openHeaderMenu(page) {
  const trigger = page.locator([
    '.vnext-header-menu-trigger[aria-label="打开菜单"]',
    '.vnext-header-menu-trigger[aria-label="Open menu"]',
    '.vnext-header-menu-trigger[aria-label="账户菜单"]',
    '.vnext-header-menu-trigger[aria-label="Account menu"]',
  ].join(', ')).first();
  await trigger.waitFor({ state: 'visible', timeout: 15000 });
  await trigger.click();
  await page.locator('[role="menu"]').waitFor({ state: 'visible', timeout: 10000 });
}

/** Close the header menu whatever way the current build supports. */
async function closeHeaderMenu(page) {
  const menu = page.locator('[role="menu"]');
  if (!(await menu.count()) || !(await menu.first().isVisible().catch(() => false))) return;
  await page.keyboard.press('Escape');
  await menu.first().waitFor({ state: 'hidden', timeout: 2000 }).catch(async () => {
    await page.mouse.click(5, 400);
    await menu.first().waitFor({ state: 'hidden', timeout: 2000 }).catch(async () => {
      const trigger = page.locator('.vnext-header-menu-trigger').first();
      if (await trigger.count()) await trigger.click({ force: true });
    });
  });
  await page.waitForTimeout(250);
}

async function clickMenuItem(page, labels) {
  const items = page.locator('[role="menu"] [role="menuitem"], [role="menu"] button');
  const n = await items.count();
  for (let i = 0; i < n; i++) {
    const text = ((await items.nth(i).innerText().catch(() => '')) || '').trim();
    if (labels.some((l) => text === l || text.startsWith(l))) { await items.nth(i).click(); return true; }
  }
  return false;
}

async function capturePass(theme, { page, ok, fail, shot }) {
  await page.emulateMedia({ colorScheme: theme });
  const tag = (name) => `${theme}-${name}`;

  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('.vnext-nav button, nav button').first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await page.waitForTimeout(1200);
  await shot(tag('home-rest'));
  ok(`${theme}: home captured`);

  try {
    await openHeaderMenu(page);
    await shot(tag('home-menu'));
    await closeHeaderMenu(page);
  } catch (e) {
    fail(`${theme}: header menu did not open`, String(e));
    await closeHeaderMenu(page).catch(() => {});
  }

  const work = page.locator('button').filter({ hasText: /^\s*(工作|Work)\s*$/ }).filter({ visible: true }).first();
  if (!(await work.count())) { fail(`${theme}: no 工作 tab`); return; }
  try {
    await work.click({ timeout: 8000 });
  } catch (e) {
    // An overlay may still be intercepting pointer events: dismiss and retry once.
    await closeHeaderMenu(page).catch(() => {});
    await work.click({ timeout: 8000, force: true });
  }
  await page.waitForTimeout(1000);
  await shot(tag('work-list'));

  const rows = page.locator('.vnext-item-row').filter({ visible: true });
  const rowCount = await rows.count();
  if (rowCount === 0) { fail(`${theme}: work list rendered no rows from the injected payload`); return; }
  ok(`${theme}: work list rendered ${rowCount} rows`);

  // Behavioural check (owner rule 2026-09-16): opening a work item expands it
  // downward — the row's top edge must not move and the title must stay fully
  // visible below the sticky header.
  const TARGET_TITLE = '在等用户确认需要恢复哪些误完成任务';
  const target = page.locator('.vnext-item-row').filter({ visible: true }).filter({ hasText: TARGET_TITLE }).first();
  if (!(await target.count())) {
    fail(`${theme}: target row "${TARGET_TITLE}" not rendered`);
  } else {
    await target.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);
    const measure = async () => target.evaluate((el) => {
      const rect = el.getBoundingClientRect();
      const header = document.querySelector('.vnext-header, .vnext-owner-shell-chrome, header');
      const headerBottom = header ? header.getBoundingClientRect().bottom : 0;
      return { top: rect.top, headerBottom, visibleBelowHeader: rect.top >= headerBottom - 1 };
    });
    const before = await measure();
    await target.click();
    await page.waitForTimeout(900);
    await shot(tag('work-expanded'));
    const after = await measure().catch(() => null);
    if (!after) {
      fail(`${theme}: target row disappeared after expansion (re-rendered elsewhere?)`);
    } else {
      const delta = Math.round(after.top - before.top);
      const detail = `row top ${Math.round(before.top)} -> ${Math.round(after.top)}, header bottom ${Math.round(after.headerBottom)}`;
      if (Math.abs(delta) <= 4 && after.visibleBelowHeader) ok(`${theme}: expand keeps the top edge and the title stays visible (${detail})`);
      else fail(`${theme}: expand moved the row by ${delta}px or hid it under the header (${detail})`);
    }
  }

  const more = page.locator('button, summary').filter({ hasText: /更多操作|More actions/ }).filter({ visible: true }).first();
  if (await more.count()) {
    await more.click();
    await page.waitForTimeout(500);
    await shot(tag('work-more-actions'));
    ok(`${theme}: more-actions captured`);
  } else {
    ok(`${theme}: no 更多操作 control on this item (skipped)`);
  }

  const nearby = page.locator('.vnext-nearby-work-item .vnext-item-row').filter({ visible: true }).first();
  if (await nearby.count()) {
    await nearby.scrollIntoViewIfNeeded();
    await nearby.click();
    await page.waitForTimeout(600);
    await shot(tag('work-nearby-expanded'));
    ok(`${theme}: nearby detail captured`);
  } else {
    ok(`${theme}: no nearby list rendered (skipped)`);
  }

  for (const [name, labels] of [['settings', ['设置', 'Settings']], ['status', ['状态', 'Status', '系统状态']]]) {
    try {
      await openHeaderMenu(page);
      const clicked = await clickMenuItem(page, labels);
      if (!clicked) { await page.keyboard.press('Escape'); fail(`${theme}: menu has no ${labels[0]} item`); continue; }
      await page.waitForTimeout(900);
      await shot(tag(`${name}-top`));
      await page.evaluate(() => { const s = document.scrollingElement || document.documentElement; s.scrollTop = 900; const shell = document.querySelector('.vnext-shell, .vnext-app'); if (shell) shell.scrollTop = 900; });
      await page.waitForTimeout(400);
      await shot(tag(`${name}-scrolled`));
      ok(`${theme}: ${name} captured`);
    } catch (e) {
      fail(`${theme}: ${name} capture failed`, String(e));
    }
  }
}


/** Desktop pass (owner report 2026-09-16, 2000px dark screenshot): the window must not
 *  scroll, the detail pane must scroll on its own, and the header must span the
 *  same width as the two-pane container. */
async function desktopPass(theme, { page, ok, fail, shot }) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: theme });
  const tag = (name) => `${theme}-desktop-${name}`;
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('.vnext-nav button, nav button').first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await page.waitForTimeout(1200);
  await shot(tag('home'));
  const work = page.locator('button').filter({ hasText: /^\s*(工作|Work)\s*$/ }).filter({ visible: true }).first();
  if (!(await work.count())) { fail(`${theme} desktop: no 工作 tab`); return; }
  await work.click({ timeout: 8000 });
  await page.waitForTimeout(1000);
  await shot(tag('work'));
  const rows = page.locator('.vnext-item-row').filter({ visible: true });
  if (!(await rows.count())) { fail(`${theme} desktop: work list rendered no rows`); return; }
  await rows.nth(Math.min(2, (await rows.count()) - 1)).click();
  await page.waitForTimeout(900);
  await shot(tag('work-expanded'));

  const m = await page.evaluate(() => {
    const doc = document.scrollingElement || document.documentElement;
    const windowScrolls = doc.scrollHeight > window.innerHeight + 2;
    const header = document.querySelector('.vnext-app-work .vnext-header, .vnext-header');
    const hr = header ? header.getBoundingClientRect() : null;
    // The detail pane: the element that hosts the expanded item on desktop.
    const detail = document.querySelector('.vnext-detail, .vnext-work-detail, [data-work-detail]');
    const pane = detail ? detail.closest('[class*="pane"], [class*="detail"]') || detail : null;
    let paneScrolls = null, paneRect = null, paneOverflow = null;
    if (pane) {
      const cs = getComputedStyle(pane);
      paneRect = pane.getBoundingClientRect();
      paneOverflow = cs.overflowY;
      const before = pane.scrollTop;
      pane.scrollTop = 200;
      paneScrolls = pane.scrollTop !== before || pane.scrollHeight <= pane.clientHeight + 2;
      pane.scrollTop = before;
    }
    // Two-pane container width: the common ancestor of list and detail.
    const list = document.querySelector('.vnext-item-row');
    const container = list && pane ? (() => { let a = list.parentElement; while (a && !a.contains(pane)) a = a.parentElement; return a; })() : null;
    const cr = container ? container.getBoundingClientRect() : null;
    return { windowScrolls, header: hr && { left: hr.left, right: hr.right, bottom: hr.bottom }, paneScrolls, paneOverflow, paneRect: paneRect && { top: paneRect.top, right: paneRect.right, bottom: paneRect.bottom, scrollHeight: pane.scrollHeight, clientHeight: pane.clientHeight }, container: cr && { left: cr.left, right: cr.right, top: cr.top } };
  });
  if (m.windowScrolls) fail(`${theme} desktop: the window itself scrolls (document taller than the viewport) — the app shell must own scrolling`);
  else ok(`${theme} desktop: the window does not scroll`);
  if (m.paneScrolls === null) fail(`${theme} desktop: detail pane not found for the scroll check`);
  else if (m.paneScrolls) ok(`${theme} desktop: detail pane scrolls on its own (overflow-y=${m.paneOverflow}, ${m.paneRect.scrollHeight}/${m.paneRect.clientHeight})`);
  else fail(`${theme} desktop: detail pane cannot scroll (overflow-y=${m.paneOverflow}, scrollHeight ${m.paneRect.scrollHeight} > clientHeight ${m.paneRect.clientHeight})`);
  if (m.header && m.container) {
    const dl = Math.round(m.header.left - m.container.left), dr = Math.round(m.container.right - m.header.right);
    if (Math.abs(dl) <= 2 && Math.abs(dr) <= 2) ok(`${theme} desktop: header spans the two-pane container (Δleft ${dl}px, Δright ${dr}px)`);
    else fail(`${theme} desktop: header width does not match the two-pane container (Δleft ${dl}px, Δright ${dr}px)`);
    if (m.paneRect && m.paneRect.top >= m.header.bottom - 1) ok(`${theme} desktop: detail pane starts below the header`);
    else if (m.paneRect) fail(`${theme} desktop: detail pane top (${Math.round(m.paneRect.top)}) is under the header (bottom ${Math.round(m.header.bottom)})`);
  } else {
    fail(`${theme} desktop: header or two-pane container not found for the width check`);
  }
  await page.setViewportSize({ width: 390, height: 844 });
}

export async function run(ctx) {
  const { page } = ctx;
  await page.route('**/webapp/vnext/work*', async (route) => {
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) { try { base = await response.json(); } catch { base = null; } }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(auditWorkPayload(base)) });
  });
  await capturePass('light', ctx);
  await capturePass('dark', ctx);
  await desktopPass('dark', ctx);
  await desktopPass('light', ctx);
}
