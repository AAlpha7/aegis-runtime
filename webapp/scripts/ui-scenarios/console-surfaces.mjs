// Scenario: console UX pass surfaces — home vocabulary, settings height,
// status guidance once, fixed-chrome clearance at rest, needs-you banner outside stream.
// Owner rule 2026-09-06: flow changes ship a recorded operation.
import { createHash } from 'node:crypto';

export const id = 'console-surfaces';
export const title = 'Console surfaces: home/settings/status copy, settings height, fixed-chrome clearance';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/SettingsView.tsx',
  'webapp/src/vnext/StatusView.tsx',
  'webapp/src/vnext/toolPhrases.ts',
  'webapp/src/i18n/catalog.ts',
  'webapp/src/vnext/styles.css',
  'webapp/src/vnext/design/surfaces/chrome.css',
  'webapp/scripts/ui-scenarios/console-surfaces.mjs',
];

const BANNED = /回执|事项|主人|幕僚长/;

/** Open the shared header overflow menu (home + console chrome labels). */
async function openHeaderMenu(page) {
  // Home chat chrome uses chatHome.account.menu ("账户菜单" / "Account menu").
  // Non-home OwnerShellChrome uses app.shell.header.menu ("打开菜单" / "Open menu").
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

/** Click a menuitem whose visible label matches any of the given exact strings. */
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

async function surfaceFingerprint(page) {
  const text = await page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell') || document.querySelector('.vnext-app') || document.body;
    return (shell?.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 240);
  });
  const png = await page.screenshot({ type: 'png' });
  const hash = createHash('md5').update(png).digest('hex');
  return { hash, text };
}

/** Measure scrollHeight on the phone page scroller (`.vnext-shell` overflow-y:auto). */
async function settingsScrollerMetrics(page) {
  return page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    let scroller = null;
    if (shell) {
      const oy = getComputedStyle(shell).overflowY;
      if (oy === 'auto' || oy === 'scroll') scroller = shell;
    }
    if (!scroller) {
      for (const el of document.querySelectorAll('div')) {
        const oy = getComputedStyle(el).overflowY;
        if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight) {
          if (el.classList.contains('vnext-shell') || el.querySelector('.vnext-settings-hero, [data-settings-section="account"]')) {
            scroller = el;
            break;
          }
        }
      }
    }
    if (!scroller) scroller = shell || document.body;
    const text = scroller?.innerText || '';
    return {
      scrollHeight: scroller?.scrollHeight || 0,
      clientHeight: scroller?.clientHeight || 0,
      className: scroller?.className || '',
      banned: /回执|事项|主人|幕僚长/.test(text),
      hasLogout: /退出登录|Log out/.test(text),
      textHead: text.replace(/\s+/g, ' ').trim().slice(0, 120),
    };
  });
}

export async function run({ page, ok, fail, shot, viewport }) {
  const fingerprints = {};

  const recordShot = async (name) => {
    await shot(name);
    const fp = await surfaceFingerprint(page);
    fingerprints[name] = fp;
    console.log(`surface ${name}: md5=${fp.hash} text=${JSON.stringify(fp.text.slice(0, 80))}`);
    return fp;
  };

  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(1500);

  // --- Home ---
  console.log('check surface: home');
  await recordShot('01-home');
  const homeBanned = await page.evaluate((reSource) => {
    const re = new RegExp(reSource);
    const root = document.querySelector('.vnext-app, body');
    return re.test(root?.innerText || '');
  }, BANNED.source);
  if (!homeBanned) ok('home has no banned zh jargon');
  else fail('home shows banned zh jargon', '回执|事项|主人|幕僚长');

  const bannerInStream = await page.evaluate(() => {
    const banner = document.querySelector('[data-testid="aegis-needs-you-banner"]');
    if (!banner) return { present: false, inStream: false };
    const stream = document.querySelector('main.vnext-chat-stream');
    return { present: true, inStream: Boolean(stream && stream.contains(banner)) };
  });
  if (!bannerInStream.present) {
    ok('needs-you banner absent on this seed (optional)');
  } else if (!bannerInStream.inStream) {
    ok('needs-you banner is outside main.vnext-chat-stream');
  } else {
    fail('needs-you banner is inside the chat stream');
  }

  // --- Settings via real header menu (must fail closed if navigation misses) ---
  console.log('check surface: settings');
  await openHeaderMenu(page);
  await clickMenuItem(page, ['设置', 'Settings']);
  await page.locator('.vnext-settings-hero, [data-settings-section="account"]').first().waitFor({
    state: 'visible',
    timeout: 15000,
  });
  await page.waitForTimeout(500);
  await recordShot('02-settings-top');

  if (fingerprints['02-settings-top'].hash === fingerprints['01-home'].hash) {
    fail('02-settings-top equals 01-home', 'navigation never left home');
  } else if (fingerprints['02-settings-top'].text === fingerprints['01-home'].text) {
    fail('02-settings-top text equals 01-home', fingerprints['02-settings-top'].text.slice(0, 80));
  } else {
    ok('settings screenshot differs from home', fingerprints['02-settings-top'].hash.slice(0, 8));
  }

  const settingsMetrics = await settingsScrollerMetrics(page);
  console.log(
    `check settings scrollHeight: ${settingsMetrics.scrollHeight} (scroller=${settingsMetrics.className.split(/\s+/).slice(0, 2).join('.')})`,
  );
  // Device-relative: the STATIC essentials must fit within two screens of this
  // viewport (three on phones narrower than 360px, where every card wraps).
  // Attention cards (sources that currently need the owner) are state, not
  // layout — they are measured separately and excluded from the budget.
  const attentionHeight = await page.evaluate(() => {
    let total = 0;
    document.querySelectorAll('[data-settings-tier="essentials"] .vnext-settings-card.warn').forEach((el) => {
      const cs = getComputedStyle(el);
      total += el.getBoundingClientRect().height + parseFloat(cs.marginTop || '0') + parseFloat(cs.marginBottom || '0') + 12;
    });
    return Math.round(total);
  });
  const screens = viewport.width < 360 ? 3 : 2;
  const budget = viewport.height * screens;
  const essentials = settingsMetrics.scrollHeight - attentionHeight;
  console.log(`settings height: total=${settingsMetrics.scrollHeight} attention=${attentionHeight} essentials=${essentials} budget=${budget} (${screens} screens of ${viewport.height})`);
  if (essentials <= budget) {
    ok(`settings essentials fit ${screens} screens`, `essentials=${essentials} ≤ ${budget}`);
  } else {
    fail(`settings essentials exceed ${screens} screens`, `essentials=${essentials} > ${budget}`);
  }
  console.log('check settings banned jargon');
  if (!settingsMetrics.banned) ok('settings has no banned zh jargon');
  else fail('settings shows banned zh jargon');

  await page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    if (shell) shell.scrollTop = shell.scrollHeight;
    else window.scrollTo(0, document.body.scrollHeight);
  });
  await page.waitForTimeout(500);
  await recordShot('03-settings-scrolled');

  // Expand 高级 only (开发者 stays collapsed). Logout lives on the 你 card.
  await page.evaluate(() => {
    const advanced = document.querySelector('details.vnext-settings-tier.advanced');
    if (advanced) advanced.open = true;
    const developer = document.querySelector('details.vnext-settings-tier.developer');
    if (developer) developer.open = false;
  });
  await page.waitForTimeout(300);
  await page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    if (shell) shell.scrollTop = shell.scrollHeight;
  });
  await page.waitForTimeout(400);
  await recordShot('04-settings-advanced-open');

  const logoutVisible = await page.evaluate(() => {
    const logout = [...document.querySelectorAll('button, [role="button"]')]
      .find((el) => /退出登录|Log out/.test(el.textContent || ''));
    if (!logout) return { present: false, inView: false };
    const r = logout.getBoundingClientRect();
    return {
      present: true,
      inView: r.bottom > 0 && r.top < window.innerHeight,
      text: (logout.textContent || '').trim(),
    };
  });
  console.log('check settings logout', JSON.stringify(logoutVisible));
  if (logoutVisible.present) ok('settings shows logout control', logoutVisible.text);
  else fail('settings missing logout control');

  const advancedMetrics = await page.evaluate(() => {
    const advanced = document.querySelector('details.vnext-settings-tier.advanced');
    const root = document.querySelector('.vnext-app-settings, .vnext-shell, body');
    const text = root?.innerText || '';
    const asciiSentence = /[A-Za-z]+(?:\s+[A-Za-z]+){3,}\./;
    return {
      scrollHeight: advanced?.scrollHeight || 0,
      asciiSentence: asciiSentence.test(text),
      asciiSample: (text.match(/[A-Za-z]+(?:\s+[A-Za-z]+){3,}\./) || [''])[0].slice(0, 80),
    };
  });
  // Same narrow-device rule as the essentials budget above: two screens,
  // three under 360px wide (disclosure hints + captions wrap on 320px).
  const advancedScreens = viewport.width < 360 ? 3 : 2;
  const advancedBudget = viewport.height * advancedScreens;
  console.log(
    `settings advanced height: scrollHeight=${advancedMetrics.scrollHeight} budget=${advancedBudget} (${advancedScreens} screens of ${viewport.height})`,
  );
  if (advancedMetrics.scrollHeight <= advancedBudget) {
    ok(`settings advanced fits ${advancedScreens} screens`, `advanced=${advancedMetrics.scrollHeight} ≤ ${advancedBudget}`);
  } else {
    fail(`settings advanced exceeds ${advancedScreens} screens`, `advanced=${advancedMetrics.scrollHeight} > ${advancedBudget}`);
  }
  if (!advancedMetrics.asciiSentence) ok('settings has no ASCII 4-word sentence');
  else fail('settings shows an ASCII 4-word sentence', advancedMetrics.asciiSample);

  // --- Status via real header menu (console chrome labels) ---
  console.log('check surface: status');
  await openHeaderMenu(page);
  // From settings the console chrome uses "状态"/"Status". Home uses "运行状态"/"Runtime status".
  await clickMenuItem(page, ['状态', 'Status', '运行状态', 'Runtime status']);
  await page.locator('.vnext-status-hero, .vnext-status-metrics').first().waitFor({
    state: 'visible',
    timeout: 15000,
  });
  await page.waitForTimeout(500);
  await recordShot('05-status');

  if (fingerprints['05-status'].hash === fingerprints['04-settings-advanced-open'].hash) {
    fail('05-status equals 04-settings-advanced-open', 'navigation never left settings');
  } else if (fingerprints['05-status'].text === fingerprints['04-settings-advanced-open'].text) {
    fail('05-status text equals 04-settings-advanced-open', fingerprints['05-status'].text.slice(0, 80));
  } else {
    ok('status screenshot differs from settings', fingerprints['05-status'].hash.slice(0, 8));
  }

  console.log('check status 要拍板 count');
  const statusCheck = await page.evaluate(() => {
    const text = document.querySelector('.vnext-app-status, .vnext-shell, body')?.innerText || '';
    const matches = text.match(/要拍板/g) || [];
    return {
      count: matches.length,
      banned: /回执|事项|主人|幕僚长/.test(text),
    };
  });
  if (statusCheck.count <= 1) ok('要拍板 appears at most once on status', `count=${statusCheck.count}`);
  else fail('要拍板 appears more than once on status', `count=${statusCheck.count}`);
  console.log('check status banned jargon');
  if (!statusCheck.banned) ok('status has no banned zh jargon');
  else fail('status shows banned zh jargon');

  // --- No fixed overlay (other than header/nav/composer) covers controls ---
  console.log('check surface: status fixed-chrome clearance');
  await page.evaluate(() => {
    const shell = document.querySelector('.vnext-shell');
    if (shell) shell.scrollTop = Math.min(400, shell.scrollHeight);
    else window.scrollTo(0, 400);
  });
  await page.waitForTimeout(400);
  const fixedClear = await page.evaluate(() => {
    const visible = (el) => {
      const style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const allowed = (el) => {
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
    const intersects = (a, b) => !(
      a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom
    );
    const interactives = [...document.querySelectorAll('button, a[href], summary, select, input, textarea, [role="button"]')]
      .filter(visible);
    const fixedEls = [...document.querySelectorAll('*')].filter((el) => visible(el) && getComputedStyle(el).position === 'fixed');
    const overlaps = [];
    for (const fixed of fixedEls) {
      if (allowed(fixed)) continue;
      const box = fixed.getBoundingClientRect();
      if (box.width < 2 || box.height < 2) continue;
      for (const target of interactives) {
        if (fixed === target || fixed.contains(target) || target.contains(fixed)) continue;
        if (intersects(box, target.getBoundingClientRect())) {
          overlaps.push((fixed.className || fixed.tagName || '').toString().slice(0, 60));
          break;
        }
      }
    }
    return { overlaps };
  });
  if (fixedClear.overlaps.length === 0) {
    ok('no fixed element other than header/nav/composer overlaps interactive controls at rest');
  } else {
    fail('fixed element overlaps interactive controls', fixedClear.overlaps.join(','));
  }

  // Viewport sanity for the packet.
  const size = await page.viewportSize();
  if (size && size.width === viewport.width && size.height === viewport.height) {
    ok('phone viewport', `${size.width}x${size.height}`);
  }
}
