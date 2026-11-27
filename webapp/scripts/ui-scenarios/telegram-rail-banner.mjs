// Scenario: Telegram rail + nightly verdict + offline banner (incident
// 2026-09-15: the production bot token was revoked at 03:15 UTC and the
// console never said a word for six hours).
//
// Injects `telegram_rail` / `nightly` into the real /webapp/vnext/status body
// (same route-fulfil pattern as light-theme-no-dark-slabs) and proves:
//   1. an invalid token shows the alert banner with the .env hint, in the
//      viewport, on the chat home;
//   2. dismissing it hides it, a reload keeps it hidden (localStorage), and a
//      NEW episode (different since_unix) re-arms it;
//   3. Status lists the Telegram row under Connections (warn) and the muted
//      nightly line — no banner for the nightly;
//   4. going offline shows the offline banner at the top and reconnecting
//      hides it.
export const id = 'telegram-rail-banner';
export const title = 'Telegram rail banner: invalid token alert, dismissal + re-arm, status row, nightly line, offline notice';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/TelegramRailBanner.tsx',
  'webapp/src/vnext/OfflineBanner.tsx',
  'webapp/src/vnext/onlineStatus.ts',
  'webapp/src/vnext/statusRails.ts',
  'webapp/src/vnext/StatusView.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/apiGuards.ts',
  'webapp/src/i18n/catalog.ts',
  'webapp/scripts/ui-scenarios/telegram-rail-banner.mjs',
];

const NOW = Math.floor(Date.now() / 1000);

/** Same header-menu helpers as console-surfaces (home vs console chrome labels). */
async function openHeaderMenu(page) {
  const trigger = page.locator(
    [
      '.vnext-header-menu-trigger[aria-label="打开菜单"]',
      '.vnext-header-menu-trigger[aria-label="Open menu"]',
      '.vnext-header-menu-trigger[aria-label="账户菜单"]',
      '.vnext-header-menu-trigger[aria-label="Account menu"]',
    ].join(', '),
  ).first();
  await trigger.waitFor({ state: 'visible', timeout: 15000 });
  await trigger.click();
  await page.locator('[role="menu"]').waitFor({ state: 'visible', timeout: 10000 });
}

async function clickMenuItem(page, labels) {
  const items = page.locator('[role="menuitem"]');
  await items.first().waitFor({ state: 'visible', timeout: 10000 });
  const count = await items.count();
  for (let i = 0; i < count; i += 1) {
    const item = items.nth(i);
    const labelEl = item.locator('.vnext-header-menu-item-label');
    const raw = (await labelEl.count()) ? await labelEl.innerText() : await item.innerText();
    const text = (raw || '').trim();
    if (labels.includes(text)) {
      await item.click();
      return text;
    }
  }
  throw new Error(`menuitem not found among ${JSON.stringify(labels)}; saw ${count} items`);
}

/** Route /webapp/vnext/status through the real backend and overlay the rails. */
async function injectStatusRails(page, rails) {
  await page.unroute('**/webapp/vnext/status*').catch(() => {});
  await page.route('**/webapp/vnext/status*', async (route) => {
    const response = await route.fetch().catch(() => null);
    let base = null;
    if (response) {
      try { base = await response.json(); } catch { base = null; }
    }
    const body = {
      schema_version: 1,
      generated_at: new Date().toISOString(),
      sections: [],
      system_events: [],
      advanced_diagnostics_available: false,
      llm_degraded: null,
      embedder_degraded: null,
      ...(base && typeof base === 'object' ? base : {}),
      ...rails,
    };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
}

function invalidTokenRail(sinceUnix) {
  return {
    telegram_rail: {
      state: 'invalid_token',
      since_unix: sinceUnix,
      last_error: "A Telegram's error: Invalid bot token",
      degraded: true,
    },
    nightly: {
      verdict: 'RED',
      sha: '02c0d462',
      finished_at_unix: NOW - 3 * 3600 - 120,
      failed_steps: ['cargo test'],
      report: null,
    },
  };
}

export async function run({ page, context, ok, fail, shot, inViewport }) {
  const firstEpisode = NOW - 6 * 3600;
  await injectStatusRails(page, invalidTokenRail(firstEpisode));
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('.vnext-app').waitFor({ state: 'visible', timeout: 60000 });

  // 1. Alert banner, in the viewport, with the fix.
  // App shell readiness does not mean the asynchronous status request has
  // arrived. Wait for the actual injected state before measuring the banner.
  const banner = page.locator('[data-telegram-rail="invalid_token"]');
  await banner.waitFor({ state: 'visible', timeout: 15000 });
  if (await banner.count()) {
    const { visible, box } = await inViewport(banner.first());
    if (visible) ok('invalid token: alert banner in the viewport', JSON.stringify(box));
    else fail('invalid token: banner rendered but outside the viewport', JSON.stringify(box));
    const text = (await banner.first().innerText()).replace(/\s+/g, ' ');
    if (/token 已失效|token is invalid/i.test(text) && /TELOXIDE_TOKEN/.test(text)) ok('invalid token: copy names the outage and the .env fix', text);
    else fail('invalid token: banner copy missing the outage/.env hint', text);
    const role = await banner.first().getAttribute('role');
    if (role === 'alert') ok('invalid token: role=alert'); else fail('invalid token: expected role=alert', String(role));
  } else {
    fail('invalid token: no [data-telegram-rail="invalid_token"] banner rendered');
  }
  await shot('01-invalid-token-banner');

  // 2. Dismiss → hidden; reload keeps it hidden; a new episode re-arms it.
  const dismiss = banner.first().locator('button');
  if (await dismiss.count()) {
    await dismiss.first().click();
    await page.waitForTimeout(300);
    if ((await banner.count()) === 0) ok('dismiss: banner hidden after tapping 忽略');
    else fail('dismiss: banner still present after tapping 忽略');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('.vnext-app').waitFor({ state: 'visible', timeout: 60000 });
    await page.waitForTimeout(1500);
    if ((await banner.count()) === 0) ok('dismiss: same episode stays hidden across a reload (localStorage)');
    else fail('dismiss: same episode came back after a reload');
    await shot('02-dismissed');

    await injectStatusRails(page, invalidTokenRail(firstEpisode + 3600));
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('.vnext-app').waitFor({ state: 'visible', timeout: 60000 });
    await banner.waitFor({ state: 'visible', timeout: 15000 });
    if (await banner.count()) ok('re-arm: a new since_unix shows the banner again despite the earlier dismissal');
    else fail('re-arm: a new episode did not re-show the banner');
    await shot('03-rearmed');
  } else {
    fail('dismiss: no dismiss button inside the banner');
  }

  // 3. Status: Telegram row under Connections + muted nightly line, no banner for it.
  try {
    await openHeaderMenu(page);
    await clickMenuItem(page, ['运行状态', 'Runtime status', '状态', 'Status']);
    await page.locator('.vnext-status-hero').waitFor({ state: 'visible', timeout: 15000 });
    await page.waitForTimeout(600);
    await page.evaluate(() => {
      for (const details of document.querySelectorAll('details.vnext-disclosure')) details.open = true;
    });
    await page.waitForTimeout(300);
    const row = page.locator('[data-telegram-rail-row="true"]');
    if (await row.count()) {
      const cls = (await row.first().getAttribute('class')) || '';
      const text = (await row.first().innerText()).replace(/\s+/g, ' ');
      if (/\bwarn\b/.test(cls) && /Telegram/.test(text)) ok('status: Telegram row listed under Connections with warn', text);
      else fail('status: Telegram row present but not warn / not labelled', `${cls} :: ${text}`);
    } else {
      fail('status: no Telegram row under Connections');
    }
    const nightly = page.locator('[data-nightly-line="true"]');
    if (await nightly.count()) {
      const text = (await nightly.first().innerText()).trim();
      if (/夜间测试 RED|Nightly test run RED/.test(text) && /02c0d462/.test(text)) ok('status: muted nightly line', text);
      else fail('status: nightly line has unexpected copy', text);
      const cls = (await nightly.first().getAttribute('class')) || '';
      if (/vnext-muted/.test(cls) && !/vnext-llm-degraded-banner/.test(cls)) ok('status: nightly is muted text, not a banner');
      else fail('status: nightly line is not muted / is a banner', cls);
    } else {
      fail('status: no nightly line for a RED verdict');
    }
    await shot('04-status-rail-row');
  } catch (err) {
    fail('status: could not open the status view', String(err && err.message ? err.message : err));
  }

  // 4. Offline → banner at the top; back online → gone.
  const offline = page.locator('[data-offline-banner="true"]');
  await context.setOffline(true);
  await page.waitForTimeout(500);
  if (await offline.count()) {
    const { visible, box } = await inViewport(offline.first());
    const text = (await offline.first().innerText()).trim();
    if (visible && /已离线|Offline/.test(text)) ok('offline: banner shown at the top while offline', text);
    else fail('offline: banner present but not visible / wrong copy', `${JSON.stringify(box)} :: ${text}`);
  } else {
    fail('offline: no [data-offline-banner] after context.setOffline(true)');
  }
  await shot('05-offline');
  await context.setOffline(false);
  await page.waitForTimeout(500);
  if ((await offline.count()) === 0) ok('offline: banner hidden again on reconnect');
  else fail('offline: banner still shown after reconnect');
}
