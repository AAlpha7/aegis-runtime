import { injectedWorkPayload } from './work-desktop-controls.mjs';

export const id = 'chat-needs-you-gutters';
export const title = 'Mobile needs-you banner stays inside both safe-area gutters';
export const surfaces = ['webapp/src/vnext/design/surfaces/chat-home.css'];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/work*';
  await page.route(pattern, route => {
    const payload = injectedWorkPayload({ generated_at: new Date().toISOString(), sources: [] });
    const item = payload.lanes.find(lane => lane.id === 'follow_up').items[0];
    Object.assign(item, {
      id: 'approval:gutter-check', source_kind: 'approval', source: 'approvals',
      state: 'waiting', lane: 'decide', title: 'Review calendar access',
      summary: 'Aegis needs your decision before continuing.',
    });
    item.authority.requires_approval = true;
    for (const lane of payload.lanes) lane.items = lane.id === 'decide' ? [item] : [];
    payload.counts = { now: 0, decide: 1, follow_up: 0, backlog: 0, done: 0 };
    return route.fulfill({ json: payload });
  });
  const measure = () => page.locator('.vnext-chat-needs-you-banner').evaluate(node => {
    const rect = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return { left: rect.left, right: rect.right, width: innerWidth,
      leftGutter: parseFloat(style.marginLeft), rightGutter: parseFloat(style.marginRight) };
  });
  const fits = rect => rect.left >= rect.leftGutter - 1
    && rect.right <= rect.width - rect.rightGutter + 1;
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const banner = page.locator('.vnext-chat-needs-you-banner');
    await banner.waitFor({ state: 'visible', timeout: 30000 });
    for (const width of [320, 390, 500, 820]) {
      await page.setViewportSize({ width, height: 844 });
      await page.waitForTimeout(150);
      const rect = await measure();
      if (fits(rect)) ok(`banner respects both gutters at ${width}px`, JSON.stringify(rect));
      else fail(`banner clipped at ${width}px`, JSON.stringify(rect));
      await shot(`width-${width}`);
    }
    // Prove the check catches the original full-width-plus-margins regression.
    await page.setViewportSize({ width: 390, height: 844 });
    await banner.evaluate(node => node.style.width = '100%');
    if (!fits(await measure())) ok('negative control detects original width regression');
    else fail('layout assertion missed original width regression');
    await banner.evaluate(node => node.style.removeProperty('width'));
  } finally {
    await page.unroute(pattern);
  }
}
