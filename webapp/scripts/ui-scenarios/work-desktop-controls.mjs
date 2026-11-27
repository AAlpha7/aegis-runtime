// Desktop 工作 view (owner screenshot 2026-09-14): every phone fix from E12–E18
// lived inside the ≤640px block or missed the nearby-item panel, so on a wide
// screen 结束跟进 / 更多操作 / 明天再看 / 添加更新 / 询问安琪 rendered at five
// widths (282 / 114 / 260 / 77 / 357) and two heights (44 / 40). This scenario
// runs at 1440×900 against an INJECTED work payload (so it measures on every
// instance, including the empty isolated one) and asserts that the expanded
// nearby item's controls share one width and one height. The detail pane is
// held to the same bar — it stacks in one column too, so it is checked for a
// single width AND a single height. Both surfaces must measure the controls
// that actually regressed (结束跟进 / 更多操作 live in a bare `.vnext-action-row`),
// so a nearby panel that yields no action-row control is a FAILURE, not a skip:
// that is exactly how E19 v2 went green while proving nothing.
import { readFileSync } from 'node:fs';

export const id = 'work-desktop-controls';
export const title = 'Desktop work item: one width and one height for every control';
export const surfaces = [
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/styles.css',
];

const FIXTURE = JSON.parse(readFileSync(new URL('./fixtures/work-desktop-item.json', import.meta.url), 'utf8'));

function laneTitle(id) {
  return { now: '现在', decide: '待定', follow_up: '跟进', backlog: '待办', done: '已完成' }[id] || id;
}

/** Two follow-up items so the panel renders its nearby list and one can expand. */
export function injectedWorkPayload(base) {
  const items = [0, 1].map((n) => ({
    ...FIXTURE.item,
    id: `${FIXTURE.item.id}:${n}`,
    title: `${FIXTURE.item.title} ${n + 1}`,
  }));
  const lanes = ['now', 'decide', 'follow_up', 'backlog', 'done'].map((laneId) => ({
    id: laneId,
    title: laneTitle(laneId),
    subtitle: '',
    collapsed: laneId === 'backlog' || laneId === 'done',
    items: laneId === 'follow_up' ? items : [],
  }));
  return {
    ...(base && typeof base === 'object' ? base : {}),
    schema_version: FIXTURE.schema_version,
    lanes,
    counts: { now: 0, decide: 0, follow_up: items.length, backlog: 0, done: 0 },
  };
}

export async function run({ page, ok, fail, shot }) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.route('**/webapp/vnext/work*', async (route) => {
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(injectedWorkPayload(base)),
    });
  });
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  const work = page.locator('nav.vnext-rail-nav button:visible, nav.vnext-nav button:visible').filter({ hasText: /工作|Work/ }).first();
  await work.waitFor({ state: 'visible', timeout: 30000 });
  await work.click();
  await page.locator('.vnext-item-row:visible').first().waitFor({ state: 'visible', timeout: 15000 });
  const rows = page.locator('.vnext-item-row:visible');
  const rowCount = await rows.count();
  if (rowCount === 0) {
    fail('desktop work: injected payload produced no rows');
    await shot('01-no-rows');
    return;
  }
  await rows.first().click();
  await page.waitForTimeout(900);
  // The nearby panel ("更多工作 / N 个附近待办") is a collapsed disclosure; open
  // it, then expand one of its rows so that panel's own control set renders.
  const nearbyPanel = page.locator('.vnext-work-switcher, details:has(.vnext-nearby-work-item)').first();
  if (await nearbyPanel.count()) {
    const summary = nearbyPanel.locator('summary').first();
    if (await summary.count()) {
      await summary.click();
      await page.waitForTimeout(500);
    }
  }
  // Ensure the nearby item is EXPANDED rather than blind-clicking it: the row
  // above may already have expanded it, and a second click would collapse it
  // again (that is how E19 v2 measured nothing and still went green).
  const nearbyInlineCount = () => page.evaluate(() => document.querySelectorAll('.vnext-nearby-inline-detail').length);
  const nearby = page.locator('.vnext-nearby-work-item .vnext-item-row:visible');
  if ((await nearby.count()) && (await nearbyInlineCount()) === 0) {
    await nearby.first().click();
    await page.waitForTimeout(800);
  }
  await shot('01-expanded');

  // Measure the TOP-LEVEL controls of each control row: a button, or the box
  // of a disclosure (its inner summary/actions are legitimately inset). Several
  // hosts can match (the phone inline detail stays in the DOM but hidden on a
  // wide screen), so every visible host contributes.
  const measure = async (sel) => page.evaluate((selector) => {
    const visible = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const hosts = [...document.querySelectorAll(selector)].filter(visible);
    if (hosts.length === 0) return null;
    const rows = hosts.flatMap((host) => [
      ...host.querySelectorAll('.vnext-action-dock-grid, .vnext-action-primary-row-stacked, .vnext-followup-strip, .vnext-action-row'),
    ].filter(visible));
    const controls = rows.flatMap((row) => [...row.children].filter(visible).filter((el) => {
      const tag = el.tagName.toLowerCase();
      // An OPEN disclosure is a container, not a control: it legitimately grows
      // to hold its own action row, so it takes no part in the comparison.
      if (tag === 'details') return !el.open;
      return tag === 'button' || el.classList.contains('vnext-btn');
    }));
    // The ask button can sit directly in the panel, outside any row.
    const bare = hosts.flatMap((host) => [...host.querySelectorAll(':scope > .vnext-btn')].filter(visible));
    return [...new Set([...controls, ...bare])].map((el) => {
      // For a COLLAPSED disclosure the owner sees its summary, not the details
      // box: E19 v2 measured the box (full width) while the summary rendered as
      // a narrow centred pill, so the numbers said "uniform" and the screen did
      // not. Measure what is actually painted.
      const painted = el.tagName.toLowerCase() === 'details' && !el.open
        ? (el.querySelector('summary') || el)
        : el;
      const r = painted.getBoundingClientRect();
      return {
        tag: el.tagName.toLowerCase(),
        text: (el.textContent || '').trim().slice(0, 12),
        w: Math.round(r.width),
        h: Math.round(r.height),
        // An OPEN disclosure legitimately grows past the control height.
        open: el.tagName.toLowerCase() === 'details' && el.hasAttribute('open'),
      };
    });
  }, sel);

  const nearbyControls = await measure('.vnext-nearby-inline-detail');
  // The injected fixture exists precisely so this panel renders, so thin
  // coverage is a defect in the scenario, not an instance quirk. The panel must
  // also yield the action-row pair (primary 结束跟进 + the 更多操作 disclosure);
  // measuring only the strip and the ask button is what let E19 v2 pass while
  // the five-width regression was still unproven.
  const nearbyHasDisclosure = (nearbyControls || []).some((c) => c.tag === 'details');
  if (!nearbyControls || nearbyControls.length < 3 || !nearbyHasDisclosure) {
    fail(
      'desktop work: nearby panel did not yield its action-row controls — the 282/114 offenders are unmeasured',
      JSON.stringify(nearbyControls),
    );
  } else {
    const widths = [...new Set(nearbyControls.map((c) => c.w))];
    const heights = [...new Set(nearbyControls.filter((c) => !c.open).map((c) => c.h))];
    if (widths.length === 1) ok(`desktop work: expanded item controls share one width — ${widths[0]}px`);
    else fail('desktop work: expanded item controls have several widths', JSON.stringify(nearbyControls));
    if (heights.length === 1) ok(`desktop work: expanded item controls share one height — ${heights[0]}px`);
    else fail('desktop work: expanded item controls have several heights', JSON.stringify(nearbyControls));
  }

  const detailControls = await measure('.vnext-detail');
  if (detailControls && detailControls.length >= 2) {
    const heights = [...new Set(detailControls.filter((c) => !c.open).map((c) => c.h))];
    const widths = [...new Set(detailControls.map((c) => c.w))];
    if (heights.length === 1) ok(`desktop work: detail dock controls share one height — ${heights[0]}px`);
    else fail('desktop work: detail dock controls have several heights', JSON.stringify(detailControls));
    if (widths.length === 1) ok(`desktop work: detail dock controls share one width — ${widths[0]}px`);
    else fail('desktop work: detail dock controls have several widths', JSON.stringify(detailControls));
  } else {
    fail('desktop work: detail dock has no control row to compare', JSON.stringify(detailControls));
  }
  await shot('02-controls');
}
