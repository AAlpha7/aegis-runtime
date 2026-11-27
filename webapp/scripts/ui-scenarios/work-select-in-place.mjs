// Work select-in-place: tapping an item keeps lane/item order, expands under
// the row with full title + state cue chip above the title (no vanish/jump).
import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'work-select-in-place';
export const title = 'Work select-in-place: lanes stay put, full title, cue chip above title';
export const surfaces = [
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/WorkItemRow.tsx',
  'webapp/src/vnext/styles.css',
  'webapp/scripts/ui-scenarios/work-select-in-place.mjs',
];

export async function run({ page, ok, fail, shot }) {
  // The isolated instance's seed queue is not guaranteed: on 2026-09-14 it went
  // from one item to none between two runs, and this scenario answered with a
  // single `ok('…empty queue')` — five real assertions silently stopped running
  // and the set still reported green. Inject the same fixture the desktop
  // control scenario uses so the selection rules always have a subject.
  await page.route('**/webapp/vnext/work*', async (route) => {
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(injectedWorkPayload(base)) });
  });
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first().click();
  await page.waitForTimeout(800);
  await shot('01-work');

  const rows = page.locator('.vnext-work-mobile-inline-lanes .vnext-item-row, .vnext-work-lane .vnext-item-row');
  const rowCount = await rows.count();
  if (rowCount < 1) {
    // With the fixture injected an empty queue means the payload or the render
    // broke, not that this instance happens to be quiet — so it is a failure.
    fail('work: injected payload produced no rows — selection rules unverified');
    await shot('02-empty');
    return;
  }

  // Prefer a non-active row so the click opens selection rather than collapsing.
  // Only a VISIBLE row qualifies: a row inside a collapsed lane resolves but
  // cannot be clicked (2026-09-13 Hercules task 28: 15 s "element is not
  // visible" on row 1 while row 0 was active — a flake unrelated to the diff).
  let targetIndex = 0;
  for (let i = 0; i < rowCount; i += 1) {
    const row = rows.nth(i);
    const active = await row.evaluate((el) => el.classList.contains('active'));
    if (!active && await row.isVisible()) {
      targetIndex = i;
      break;
    }
  }
  // If every visible row is already active (single selected row), leave it open.
  const alreadyActive = await rows.nth(targetIndex).evaluate((el) => el.classList.contains('active'));
  const before = await page.evaluate(() => {
    const mobile = document.querySelector('.vnext-work-mobile-inline-lanes');
    const root = mobile && getComputedStyle(mobile).display !== 'none'
      ? mobile
      : document.querySelector('.vnext-work-layout');
    if (!root) return null;
    const laneTitles = [...root.querySelectorAll('.vnext-work-lane > .vnext-lane-head h2')]
      .map((el) => (el.textContent || '').trim());
    const itemTitles = [...root.querySelectorAll('.vnext-item-title')]
      .map((el) => (el.textContent || '').trim());
    return { laneTitles, itemTitles };
  });
  if (!before) {
    fail('could not snapshot lane/item order before select');
    return;
  }

  if (!alreadyActive) {
    await rows.nth(targetIndex).scrollIntoViewIfNeeded();
    await rows.nth(targetIndex).click();
    await page.waitForTimeout(500);
  }
  await shot('02-selected');

  const after = await page.evaluate(() => {
    const mobile = document.querySelector('.vnext-work-mobile-inline-lanes');
    const root = mobile && getComputedStyle(mobile).display !== 'none'
      ? mobile
      : document.querySelector('.vnext-work-layout');
    if (!root) return null;
    const laneTitles = [...root.querySelectorAll('.vnext-work-lane > .vnext-lane-head h2')]
      .map((el) => (el.textContent || '').trim());
    const itemTitles = [...root.querySelectorAll('.vnext-item-title')]
      .map((el) => (el.textContent || '').trim());
    const active = root.querySelector('.vnext-work-mobile-inline-item.active, .vnext-item-row.active');
    const detail = root.querySelector('.vnext-work-inline-selected.vnext-detail, .vnext-detail-desktop.vnext-detail');
    const hero = detail?.querySelector('.vnext-detail-hero');
    const chip = hero?.querySelector('.vnext-focus-chip');
    const title = hero?.querySelector('h2');
    let chipBeforeTitle = false;
    if (chip && title && hero) {
      const pos = chip.compareDocumentPosition(title);
      chipBeforeTitle = Boolean(pos & Node.DOCUMENT_POSITION_FOLLOWING);
    }
    const titleText = (title?.textContent || '').trim();
    const titleClamped = title ? (() => {
      const cs = getComputedStyle(title);
      const clamp = cs.webkitLineClamp;
      return clamp && clamp !== 'none' && clamp !== 'unset' && Number(clamp) > 0;
    })() : true;
    return {
      laneTitles,
      itemTitles,
      hasActive: Boolean(active),
      hasDetail: Boolean(detail),
      chipText: (chip?.textContent || '').trim(),
      chipBeforeTitle,
      titleText,
      titleClamped,
    };
  });

  if (!after) {
    fail('could not snapshot after select');
    return;
  }

  const lanesSame = JSON.stringify(before.laneTitles) === JSON.stringify(after.laneTitles);
  // Item titles from list rows must remain; selection may add the same title in detail.
  const allBeforeStillPresent = before.itemTitles.every((t) => after.itemTitles.includes(t) || after.titleText === t);
  if (lanesSame) ok('lane order unchanged after select', after.laneTitles.join(' | '));
  else fail('lane order changed', `${before.laneTitles.join('>')} → ${after.laneTitles.join('>')}`);
  if (allBeforeStillPresent && after.itemTitles.length >= before.itemTitles.length) {
    ok('list items still rendered after select', String(after.itemTitles.length));
  } else {
    fail('items vanished after select', JSON.stringify({ before: before.itemTitles, after: after.itemTitles }));
  }
  if (after.hasDetail) ok('selected detail visible');
  else fail('selected detail missing');
  if (after.chipBeforeTitle && after.chipText) ok('state cue chip sits above the title', after.chipText);
  else fail('cue chip not above title', JSON.stringify({ chip: after.chipText, chipBeforeTitle: after.chipBeforeTitle }));
  if (after.titleText && !after.titleClamped) ok('selected title is not CSS-clamped', after.titleText.slice(0, 80));
  else fail('selected title missing or still clamped', JSON.stringify({ title: after.titleText, clamped: after.titleClamped }));

  // Collapse via tapping the active row again (in-place, no jump required).
  // Desktop lane sections stay in the DOM on phones (CSS-hidden once an item
  // is open), so pick the row the owner can actually see.
  const activeRow = page.locator('.vnext-work-mobile-inline-item.active .vnext-item-row:visible, .vnext-item-row.active:visible').first();
  if (await activeRow.count()) {
    await activeRow.click();
    await page.waitForTimeout(400);
    await shot('03-collapsed');
    const stillExpanded = await page.locator('.vnext-work-inline-selected.vnext-detail').count();
    // Desktop keeps the detail pane; mobile inline selected should collapse.
    const mobileInline = page.locator('.vnext-work-mobile-inline-lanes');
    const mobileVisible = await mobileInline.evaluate((el) => getComputedStyle(el).display !== 'none').catch(() => false);
    if (!mobileVisible || stillExpanded === 0) ok('collapse leaves selection closed on phone (or desktop pane)');
    else fail('mobile inline selection did not collapse', String(stillExpanded));
  } else {
    ok('no active row to re-tap (selection may use desktop pane only)');
  }
}
