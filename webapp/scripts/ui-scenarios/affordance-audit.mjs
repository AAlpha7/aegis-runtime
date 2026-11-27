// Permanent console affordance audit: disclosures look like controls, no fixed
// overlays cover interactive targets, icon-only controls are named, and tap
// targets outside nav/segmented tracks stay ≥40px. Runs on every device pass.
export const id = 'affordance-audit';
export const title = 'Affordance audit: disclosures, fixed overlaps, names, tap targets';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/vnext/StatusView.tsx',
  'webapp/src/vnext/WorkView.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/AssistantResponseView.tsx',
  'webapp/src/vnext/styles.css',
  'webapp/src/vnext/design/surfaces/console.css',
  'webapp/src/vnext/design/surfaces/chrome.css',
  'webapp/src/i18n/catalog.ts',
];

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
      return;
    }
  }
  throw new Error(`menu item not found: ${labels.join(' | ')}`);
}

async function auditSurface(page) {
  return page.evaluate(() => {
    const visible = (el) => {
      if (!(el instanceof Element)) return false;
      const style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };

    const intersects = (a, b) => !(
      a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom
    );

    const unstyledSummaries = [...document.querySelectorAll('summary')]
      .filter(visible)
      .filter((el) => !el.classList.contains('vnext-disclosure-summary'))
      .map((el) => (el.textContent || '').trim().slice(0, 80));

    const fixedAllowed = (el) => {
      const style = getComputedStyle(el);
      if (style.pointerEvents === 'none') return true;
      return Boolean(
        el.closest('nav.vnext-nav')
        || el.closest('header')
        || el.classList.contains('vnext-header')
        || el.classList.contains('vnext-chat-topbar')
        || el.classList.contains('vnext-owner-shell-chrome')
        || el.closest('.vnext-chat-composer')
        || el.classList.contains('vnext-chat-composer')
        || el.classList.contains('vnext-chat-panel-host')
        || el.classList.contains('vnext-hidden-file-input')
        || el.classList.contains('vnext-skip-link')
      );
    };

    const interactiveSelector = 'button, a[href], summary, select, input, textarea, [role="button"]';
    const interactives = [...document.querySelectorAll(interactiveSelector)].filter(visible);
    const fixedEls = [...document.querySelectorAll('*')].filter((el) => {
      if (!visible(el)) return false;
      const pos = getComputedStyle(el).position;
      return pos === 'fixed';
    });

    const fixedOverlaps = [];
    for (const fixed of fixedEls) {
      if (fixedAllowed(fixed)) continue;
      const fabBox = fixed.getBoundingClientRect();
      if (fabBox.width < 2 || fabBox.height < 2) continue;
      for (const target of interactives) {
        if (fixed === target || fixed.contains(target) || target.contains(fixed)) continue;
        if (intersects(fabBox, target.getBoundingClientRect())) {
          fixedOverlaps.push({
            fixed: (fixed.className || fixed.tagName || '').toString().slice(0, 80),
            target: (target.getAttribute('aria-label') || target.textContent || target.tagName || '').toString().trim().slice(0, 80),
          });
          break;
        }
      }
    }

    const unnamedControls = interactives
      .filter((el) => el.matches('button, a[href]'))
      .filter((el) => {
        const text = (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
        const aria = (el.getAttribute('aria-label') || '').trim();
        const title = (el.getAttribute('title') || '').trim();
        return !text && !aria && !title;
      })
      .map((el) => (el.className || el.tagName || '').toString().slice(0, 80));

    const smallTargets = interactives
      .filter((el) => (
        !el.closest('nav')
        && !el.closest('.vnext-segmented')
        && !el.closest('.vnext-seg')
        && !el.closest('[role="tablist"]')
        && !el.closest('[role="radiogroup"]')
      ))
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return r.height > 0 && r.height < 40;
      })
      .map((el) => ({
        label: (el.getAttribute('aria-label') || el.textContent || el.className || el.tagName || '').toString().trim().slice(0, 80),
        height: Math.round(el.getBoundingClientRect().height * 10) / 10,
      }));

    // Header subtitle must show its whole sentence (状态's was ellipsised).
    const truncatedSubtitles = [...document.querySelectorAll('.vnext-header-subtitle')]
      .filter(visible)
      .filter((el) => el.scrollWidth > el.clientWidth + 1)
      .map((el) => (el.textContent || '').trim().slice(0, 80));

    // The bundled 安琪 avatar must actually load (it 404'd at the root path).
    const brokenAvatars = [...document.querySelectorAll('.vnext-avatar.aegis')]
      .filter(visible)
      .filter((el) => {
        const img = el.querySelector('img');
        return !img || !img.complete || img.naturalWidth === 0;
      }).length;

    return {
      unstyledSummaries,
      fixedOverlaps,
      unnamedControls,
      smallTargets,
      truncatedSubtitles,
      brokenAvatars,
    };
  });
}

/** Top-of-page position of the phone scroller (`.vnext-shell`) or the window. */
async function scrollTop(page) {
  return page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    return shell ? shell.scrollTop : window.scrollY;
  });
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(800);

  const totals = {
    unstyledSummaries: 0,
    fixedOverlaps: 0,
    unnamedControls: 0,
    smallTargets: 0,
    truncatedSubtitles: 0,
    brokenAvatars: 0,
    surfaces: {},
  };

  const record = (name, result) => {
    totals.surfaces[name] = {
      unstyledSummaries: result.unstyledSummaries.length,
      fixedOverlaps: result.fixedOverlaps.length,
      unnamedControls: result.unnamedControls.length,
      smallTargets: result.smallTargets.length,
      truncatedSubtitles: result.truncatedSubtitles.length,
      brokenAvatars: result.brokenAvatars,
    };
    totals.truncatedSubtitles += result.truncatedSubtitles.length;
    totals.brokenAvatars += result.brokenAvatars;
    if (result.truncatedSubtitles.length === 0) ok(`${name}: header subtitle not truncated`);
    else fail(`${name}: truncated header subtitle`, JSON.stringify(result.truncatedSubtitles));
    if (result.brokenAvatars === 0) ok(`${name}: 安琪 avatar image loaded`);
    else fail(`${name}: broken 安琪 avatar image`, String(result.brokenAvatars));
    totals.unstyledSummaries += result.unstyledSummaries.length;
    totals.fixedOverlaps += result.fixedOverlaps.length;
    totals.unnamedControls += result.unnamedControls.length;
    totals.smallTargets += result.smallTargets.length;
    if (result.unstyledSummaries.length === 0) ok(`${name}: disclosures styled`);
    else fail(`${name}: unstyled summaries`, JSON.stringify(result.unstyledSummaries.slice(0, 5)));
    if (result.fixedOverlaps.length === 0) ok(`${name}: no fixed overlaps`);
    else fail(`${name}: fixed overlaps`, JSON.stringify(result.fixedOverlaps.slice(0, 5)));
    if (result.unnamedControls.length === 0) ok(`${name}: controls named`);
    else fail(`${name}: unnamed controls`, JSON.stringify(result.unnamedControls.slice(0, 5)));
    if (result.smallTargets.length === 0) ok(`${name}: tap targets ≥40px`);
    else fail(`${name}: small targets`, JSON.stringify(result.smallTargets.slice(0, 5)));
  };

  // Home
  await shot('01-home');
  record('home', await auditSurface(page));

  // Work
  await page.locator('nav.vnext-nav button').filter({ hasText: /工作|Work/ }).first().click();
  await page.waitForTimeout(600);
  await shot('02-work');
  record('work', await auditSurface(page));
  // Landing on 工作 auto-selects the first priority item; that must not scroll
  // the 查找 card and queue summary under the sticky header.
  const workLandingTop = await scrollTop(page);
  if (workLandingTop <= 1) ok('work: landing stays at the top', `scrollTop=${workLandingTop}`);
  else fail('work: landing scrolled away from the top', `scrollTop=${workLandingTop}`);

  if (await page.getByTestId('aegis-chat-sheet-open').count() === 0) ok('work: no duplicate global chat opener');
  else fail('work: duplicate global chat opener');

  // Settings — open all tiers and scroll to bottom
  await openHeaderMenu(page);
  await clickMenuItem(page, ['设置', 'Settings']);
  await page.waitForTimeout(700);
  const advanced = page.locator('[data-settings-tier="advanced"]');
  if (await advanced.count()) {
    await advanced.evaluate((el) => { el.open = true; });
  }
  const developer = page.locator('[data-settings-tier="developer"]');
  if (await developer.count()) {
    await developer.evaluate((el) => { el.open = true; });
  }
  await page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    if (shell) shell.scrollTop = shell.scrollHeight;
    else window.scrollTo(0, document.body.scrollHeight);
  });
  await page.waitForTimeout(400);
  await shot('03-settings');
  record('settings', await auditSurface(page));

  // Status
  await openHeaderMenu(page);
  await clickMenuItem(page, ['状态', 'Status']);
  await page.waitForTimeout(700);
  await page.evaluate(() => {
    document.querySelectorAll('.vnext-app-status details.vnext-disclosure').forEach((el) => {
      el.open = true;
    });
  });
  await page.waitForTimeout(300);
  await shot('04-status');
  record('status', await auditSurface(page));

  console.log(`AFFORDANCE_JSON ${JSON.stringify(totals)}`);
  if (
    totals.unstyledSummaries === 0
    && totals.fixedOverlaps === 0
    && totals.unnamedControls === 0
    && totals.smallTargets === 0
    && totals.truncatedSubtitles === 0
    && totals.brokenAvatars === 0
  ) {
    ok('affordance audit clean on this device');
  } else {
    fail('affordance audit found issues', JSON.stringify(totals));
  }
}
