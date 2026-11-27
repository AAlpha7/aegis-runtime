// iPhone-sized home: light page ground, header and tab bar sit on the device
// safe-area inset with no frost band, owner-facing action receipts follow
// the interface language, a clock-loop reminder stays on Work, and an open
// keyboard does not cover the composer.
export const id = 'phone-safe-chrome';
export const title = 'Phone chrome follows the safe-area inset and owner text is readable';
export const surfaces = [
  'webapp/src/vnext/styles.css',
  'webapp/src/vnext/design/surfaces/chrome.css',
  'webapp/src/vnext/design/surfaces/chat-home.css',
  'webapp/index.html',
  'webapp/src/vnext/ownerUrlText.ts',
  'webapp/src/vnext/xDeepLink.ts',
  'webapp/src/vnext/notificationActions.ts',
  'webapp/src/vnext/sourceEvidenceText.ts',
  'webapp/src/vnext/taskActionReceipt.ts',
  'webapp/src/vnext/keyboardInset.ts',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/design/surfaces/menu.css',
  'webapp/src/vnext/design/surfaces/rich-text.css',
];

// Content inset, the gap under the header, and the gap above the tab bar.
// Chat is the reference: 12px side inset, 12px under the header, composer
// flush with the tab bar. Work must use those same values, not a third one.
async function phoneEdgeGaps(page) {
  return page.evaluate(() => {
    const vw = window.innerWidth;
    const nav = document.querySelector('nav.vnext-nav');
    const header = document.querySelector('.vnext-owner-shell-chrome');
    if (!nav || !header) return null;
    const headerBottom = header.getBoundingClientRect().bottom;
    const chat = document.querySelector('.vnext-app.vnext-app-chat-home');
    if (chat) {
      const stream = document.querySelector('.vnext-chat-stream');
      const composer = document.querySelector('.vnext-chat-composer');
      if (!stream || !composer) return null;
      const streamBox = stream.getBoundingClientRect();
      const composerBox = composer.getBoundingClientRect();
      return {
        page: 'chat',
        insetLeft: composerBox.left,
        insetRight: vw - composerBox.right,
        streamInsetLeft: streamBox.left,
        streamInsetRight: vw - streamBox.right,
        gapUnderHeader: streamBox.top - headerBottom,
        gapAboveTab: nav.getBoundingClientRect().top - composerBox.bottom,
      };
    }
    const main = document.querySelector('.vnext-app-work .vnext-main');
    const shell = document.querySelector('.vnext-app-work .vnext-shell');
    if (!main || !shell) return null;
    const mainBox = main.getBoundingClientRect();
    const probe = document.createElement('div');
    probe.style.cssText = 'height:2000px;flex:0 0 auto;';
    main.appendChild(probe);
    const previous = shell.scrollTop;
    shell.scrollTop = shell.scrollHeight;
    const gapAboveTab = nav.getBoundingClientRect().top - probe.getBoundingClientRect().bottom;
    probe.remove();
    shell.scrollTop = previous;
    return {
      page: 'work',
      insetLeft: mainBox.left,
      insetRight: vw - mainBox.right,
      gapUnderHeader: mainBox.top - headerBottom,
      gapAboveTab,
    };
  });
}

async function desktopChromeBox(page) {
  return page.evaluate(() => {
    const rail = document.querySelector('.vnext-brand-rail');
    const bar = document.querySelector('.vnext-owner-shell-chrome');
    const column = document.querySelector('.vnext-chat-home, .vnext-app-work .vnext-shell');
    if (!rail || getComputedStyle(rail).display === 'none') return null;
    const rb = rail.getBoundingClientRect();
    const bb = bar?.getBoundingClientRect();
    const cb = column?.getBoundingClientRect();
    return {
      railT: rb.top,
      railB: rb.bottom,
      barH: bb?.height ?? null,
      columnB: cb?.bottom ?? null,
      vh: window.innerHeight,
    };
  });
}

async function navGlyphEdges(page, selector) {
  return page.evaluate((selector) => {
    return [...document.querySelectorAll(selector)].flatMap((button) => {
      const box = button.getBoundingClientRect();
      const svg = button.querySelector('svg');
      if (!svg || box.width < 1 || getComputedStyle(button).display === 'none') return [];
      const svgBox = svg.getBoundingClientRect();
      const bb = svg.getBBox();
      const scale = svgBox.width / (svg.viewBox.baseVal.width || 24);
      return [{
        text: (button.innerText || '').replace(/\s+/g, ' ').trim(),
        btnL: box.left,
        svgL: svgBox.left,
        inkFromSvg: bb.x * scale,
      }];
    });
  }, selector);
}

function luminance(rgb) {
  const m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)/.exec(rgb || '');
  if (m) return (0.2126 * Number(m[1]) + 0.7152 * Number(m[2]) + 0.0722 * Number(m[3])) / 255;
  // color-mix() serializes as color(srgb R G B / A) with channels in 0..1.
  const s = /color\(srgb\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)/.exec(rgb || '');
  if (s) return 0.2126 * Number(s[1]) + 0.7152 * Number(s[2]) + 0.0722 * Number(s[3]);
  return null;
}

export async function run({ page, ok, fail, shot }) {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: 'light' });
  // iPhone 16 Pro insets. Chromium without this override reports 0, and the
  // zero-inset path below still has to add no extra band.
  try {
    const client = await page.context().newCDPSession(page);
    await client.send('Emulation.setSafeAreaInsetsOverride', {
      insets: { top: 62, left: 0, bottom: 34, right: 0 },
    });
  } catch { /* safe-area stays 0 */ }
  const now = Math.floor(Date.now() / 1000);
  await page.route('**/webapp/console/tasks/**/action', (route) => route.fulfill({
    json: {
      status: 'ok',
      resource_type: 'task',
      resource_id: '2138',
      action: 'complete',
      updated: true,
      ledger_entry_id: 1,
      receipt: 'task_complete:2138:complete:scenario',
      summary: 'Task #2138 marked complete',
      due_at: null,
      recurring: true,
    },
  }));
  await page.route('**/webapp/vnext/chat-history*', (route) => route.fulfill({
    json: {
      schema_version: 1,
      has_more: false,
      messages: [
        { role: 'owner', text: '这条链接呢', timestamp: now - 20 },
        {
          role: 'aegis',
          text: '帖子在 https://x.com/xianba%E2%80%A6',
          timestamp: now - 10,
          source_evidence: [
            {
              kind: 'tool',
              label: 'X',
              summary: 'X · 2026-09-24T05:11:11.427Z',
              detail: 'https://x.com/alice/status/111 https://x.com/xianba%E2%80%A6',
            },
          ],
        },
        {
          role: 'aegis',
          text: '提醒 · 未完成\n网页操作：`done reminder 1894`、`snooze reminder 1894`、`tomorrow reminder 1894` 或 `delete reminder 1894`。',
          timestamp: now - 5,
        },
      ],
    },
  }));
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.locator('.vnext-x-open-strip').waitFor({ state: 'visible', timeout: 30000 });
  // The chat grid animates in over 520ms. Measure after it settles.
  await page.waitForTimeout(700);

  const chrome = await page.evaluate(() => {
    const before = getComputedStyle(document.body, '::before');
    const after = getComputedStyle(document.body, '::after');
    const filter = (style) => style.backdropFilter || style.webkitBackdropFilter || '';
    return {
      htmlBg: getComputedStyle(document.documentElement).backgroundColor,
      bodyBg: getComputedStyle(document.body).backgroundColor,
      beforeFilter: filter(before),
      afterFilter: filter(after),
      beforeBg: before.backgroundColor,
      status: document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]')?.getAttribute('content') || '',
    };
  });
  const htmlLum = luminance(chrome.htmlBg);
  const bodyLum = luminance(chrome.bodyBg);
  if (htmlLum !== null && htmlLum > 0.8) ok('light document ground', chrome.htmlBg);
  else fail('document ground is still dark', JSON.stringify(chrome));
  if (bodyLum !== null && bodyLum > 0.8) ok('light body ground', chrome.bodyBg);
  else fail('body ground is still dark', chrome.bodyBg);
  if (chrome.htmlBg !== 'rgb(5, 11, 20)' && chrome.bodyBg !== 'rgb(5, 11, 20)') ok('retired #050b14 document slab');
  else fail('document still paints #050b14', JSON.stringify(chrome));
  const blur = (value) => /blur\(/.test(value || '');
  if (!blur(chrome.beforeFilter)) ok('status strip is plain, not a blur');
  else fail('status strip still blurs the header', chrome.beforeFilter);
  if (!/rgba?\([^)]*,\s*0?\.\d+\)/.test(chrome.beforeBg || '') && !/color\(srgb[^)]*\/\s*0?\.\d+/.test(chrome.beforeBg || '')) {
    ok('status strip is opaque', chrome.beforeBg);
  } else fail('status strip is still translucent', chrome.beforeBg);
  const viewportMeta = await page.locator('meta[name="viewport"]').getAttribute('content');
  if (viewportMeta && viewportMeta.includes('viewport-fit=cover') && !viewportMeta.includes('interactive-widget')) {
    ok('viewport does not keep a keyboard inset');
  } else fail('viewport still resizes for the keyboard', viewportMeta || '');
  if (!blur(chrome.afterFilter)) ok('no frost blur on the home indicator');
  else fail('a frost blur still covers the home indicator', JSON.stringify(chrome));
  if (chrome.status === 'default') ok('status bar uses dark glyphs');
  else fail('status bar style', chrome.status);

  const extra = page.locator('.vnext-x-open-extra');
  const caption = ((await extra.first().textContent()) || '').trim();
  if (caption.includes('…') && !caption.includes('%E2%80%A6') && !caption.includes('%')) ok('handoff caption decodes the ellipsis', caption);
  else fail('handoff caption still percent-encoded', caption);
  const bodyText = await page.locator('.vnext-app').innerText();
  if (!bodyText.includes('%E2%80%A6')) ok('owner-facing page has no raw ellipsis encoding');
  else fail('percent-encoded ellipsis still visible');

  const layout = await page.evaluate(() => {
    const probe = document.createElement('div');
    probe.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding-top:env(safe-area-inset-top, 0px);padding-bottom:env(safe-area-inset-bottom, 0px);';
    document.body.appendChild(probe);
    const probeStyle = getComputedStyle(probe);
    const topInset = parseFloat(probeStyle.paddingTop) || 0;
    const bottomInset = parseFloat(probeStyle.paddingBottom) || 0;
    probe.remove();
    const before = getComputedStyle(document.body, '::before');
    const frostHeight = (before.content && before.content !== 'none') ? (parseFloat(before.height) || 0) : 0;
    const header = document.querySelector('.vnext-chat-topbar');
    const headerBox = header ? header.getBoundingClientRect() : null;
    const headerStyle = header ? getComputedStyle(header) : null;
    const avatar = document.querySelector('.vnext-chat-topbar .vnext-owner-shell-summon, .vnext-chat-topbar .vnext-avatar');
    const avatarTop = avatar ? avatar.getBoundingClientRect().top : null;
    const stream = document.querySelector('.vnext-chat-stream');
    const streamTop = stream ? stream.getBoundingClientRect().top : null;
    const streamBox = stream ? stream.getBoundingClientRect() : null;
    const streamClips = stream ? /(auto|scroll|hidden|clip)/.test(getComputedStyle(stream).overflowY) : false;
    const cards = [...document.querySelectorAll('.vnext-chat-message, .vnext-chat-answer-card')];
    // A scrolled thread parks earlier cards above the stream. overflow clips
    // them, so the painted edge is the stream top, not the card's layout top.
    const visibleCard = cards
      .map((el) => el.getBoundingClientRect())
      .find((box) => streamBox && box.bottom > streamBox.top + 1 && box.top < streamBox.bottom);
    const cardTop = visibleCard
      ? (streamClips && streamBox ? Math.max(visibleCard.top, streamBox.top) : visibleCard.top)
      : (cards[0] ? cards[0].getBoundingClientRect().top : null);
    const nav = document.querySelector('nav.vnext-nav');
    const navStyle = nav ? getComputedStyle(nav) : null;
    const navBox = nav ? nav.getBoundingClientRect() : null;
    const navPad = navStyle ? (parseFloat(navStyle.paddingBottom) || 0) : null;
    const navBottom = navStyle ? (parseFloat(navStyle.bottom) || 0) : null;
    const buttons = [...document.querySelectorAll('nav.vnext-nav button')];
    const labelBottom = buttons.reduce((max, button) => Math.max(max, button.getBoundingClientRect().bottom), 0);
    const navFont = buttons[0] ? parseFloat(getComputedStyle(buttons[0]).fontSize) || 0 : 0;
    const wash = getComputedStyle(document.body, '::after');
    const washHeight = (!wash.content || wash.content === 'none') ? 0 : (parseFloat(wash.height) || 0);
    const app = document.querySelector('.vnext-app.vnext-app-chat-home');
    const appOverflow = app ? getComputedStyle(app).overflowY : '';
    const pageBg = getComputedStyle(document.body).backgroundColor;
    const seam = navBox ? document.elementFromPoint(Math.min(12, window.innerWidth - 2), Math.min(window.innerHeight - 1, navBox.bottom + 1)) : null;
    const seamBg = seam ? getComputedStyle(seam).backgroundColor : '';
    const usesTopEnv = [...document.styleSheets].some((sheet) => {
      let text = '';
      try { text = [...sheet.cssRules].map((rule) => rule.cssText).join('\n'); } catch { return false; }
      return text.includes('vnext-app-chat-home') && text.includes('env(safe-area-inset-top');
    });
    const usesBottomEnv = [...document.styleSheets].some((sheet) => {
      let text = '';
      try { text = [...sheet.cssRules].map((rule) => rule.cssText).join('\n'); } catch { return false; }
      return text.includes('vnext-nav') && text.includes('env(safe-area-inset-bottom');
    });
    return {
      topInset,
      bottomInset,
      frostHeight,
      headerTop: headerBox ? headerBox.top : null,
      headerBottom: headerBox ? headerBox.bottom : null,
      headerFilter: headerStyle ? (headerStyle.backdropFilter || headerStyle.webkitBackdropFilter || '') : '',
      headerPosition: headerStyle ? headerStyle.position : '',
      avatarTop,
      streamTop,
      cardTop,
      navPad,
      navBottom,
      navEdge: navBox ? navBox.bottom : null,
      labelBottom,
      navFont,
      washHeight,
      appOverflow,
      viewport: window.innerHeight,
      usesTopEnv,
      usesBottomEnv,
      pageBg,
      seamBg,
    };
  });
  if (layout.usesTopEnv && layout.usesBottomEnv) ok('chrome reads safe-area env insets');
  else fail('chrome does not use safe-area env insets', JSON.stringify(layout));
  if (Math.abs(layout.frostHeight - layout.topInset) <= 1) ok('status frost is only the safe-area', `${layout.frostHeight}px`);
  else fail('status frost is taller than the safe-area', JSON.stringify(layout));
  if (!blur(layout.headerFilter)) ok('header itself is not a blur');
  else fail('header still blurs the thread', layout.headerFilter);
  if (layout.headerTop !== null && Math.abs(layout.headerTop - layout.topInset) <= 1) {
    ok('header starts at the status inset', `${layout.headerTop}px`);
  } else fail('header does not start at the status inset', JSON.stringify(layout));
  if (layout.avatarTop !== null && layout.avatarTop + 1 >= layout.frostHeight) {
    ok('avatar starts below the status frost', `${layout.avatarTop}px`);
  } else fail('avatar is still under the status frost', JSON.stringify(layout));
  if (layout.streamTop !== null && layout.headerBottom !== null && layout.streamTop + 1 >= layout.headerBottom) {
    ok('thread scrollport starts below the header', `${layout.streamTop}px`);
  } else fail('thread still paints under the header', JSON.stringify(layout));
  if (layout.cardTop !== null && layout.headerBottom !== null && layout.cardTop + 1 >= layout.headerBottom) {
    ok('first card starts below the header', `${layout.cardTop}px`);
  } else fail('first card is still under the frost', JSON.stringify(layout));
  if (layout.topInset === 0 && layout.headerTop !== null && layout.headerTop <= 1) ok('zero top inset adds no extra band');
  else if (layout.topInset > 0) ok('top inset is honored', `${layout.topInset}px`);
  else fail('zero top inset still offsets the header', JSON.stringify(layout));
  if (layout.navBottom !== null && layout.navBottom <= 1 && layout.navPad !== null && Math.abs(layout.navPad - layout.bottomInset) <= 1) {
    ok('tab bar fills the home-indicator inset', `pad=${layout.navPad}px`);
  } else fail('tab bar does not sit on the home-indicator inset', JSON.stringify(layout));
  const labelGap = layout.viewport - layout.bottomInset - layout.labelBottom;
  if (layout.labelBottom > 0 && labelGap >= -2 && labelGap <= 8) ok('tab labels sit on the home-indicator inset', `${labelGap}px`);
  else fail('tab labels leave a gap under the bar', JSON.stringify(layout));
  if (layout.navEdge !== null && Math.abs(layout.viewport - layout.navEdge) <= 2) {
    ok('no empty strip under the tab bar', `${layout.navEdge}px`);
  } else fail('tab bar leaves an empty strip above the screen edge', JSON.stringify(layout));
  if (layout.navFont >= 16) ok('tab labels are a readable size', `${layout.navFont}px`);
  else fail('tab labels are still tiny', JSON.stringify(layout));
  if (layout.washHeight <= 1) ok('tab wash does not cover the labels', `${layout.washHeight}px`);
  else fail('tab wash still covers the labels', JSON.stringify(layout));
  if (layout.appOverflow === 'visible') ok('chat home does not clip the fixed tab bar');
  else fail('chat home still clips the fixed tab bar', layout.appOverflow || '');
  const linkWrap = await page.evaluate(() => {
    const card = document.querySelector('.vnext-chat-answer-card');
    if (!card) return null;
    const url = 'https://platform.openai.com/docs/api-reference/chat/create#chat-create-messages-very-long-openai-link';
    const p = document.createElement('p');
    p.className = 'vnext-rich-text';
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.textContent = url;
    p.appendChild(anchor);
    card.appendChild(p);
    const cardBox = card.getBoundingClientRect();
    const linkBox = anchor.getBoundingClientRect();
    const style = getComputedStyle(anchor);
    const overflowWrap = style.overflowWrap;
    const wordBreak = style.wordBreak;
    p.remove();
    return {
      overflow: linkBox.right - cardBox.right,
      overflowWrap,
      wordBreak,
    };
  });
  if (
    linkWrap
    && linkWrap.overflow <= 1
    && linkWrap.overflowWrap === 'anywhere'
    && linkWrap.wordBreak === 'break-all'
  ) ok('long links stay inside the message card', `${linkWrap.overflow.toFixed(1)}px`);
  else fail('long links still leave the message card', JSON.stringify(linkWrap));
  if (layout.headerPosition === 'relative') ok('phone header stays in flow under the status strip');
  else fail('phone header is still sticky under the status strip', layout.headerPosition || '');
  if (layout.bottomInset === 0) ok('zero bottom inset adds no extra band');
  else ok('bottom inset is honored', `${layout.bottomInset}px`);

  // The layout viewport on iPhone stays short by the top safe area after the
  // keyboard closes. The shell has to grow by that amount so the in-flow tab
  // bar, not the page background, is what reaches the screen edge.
  const shortfall = 62;
  await page.evaluate((px) => {
    document.documentElement.style.setProperty('--layout-shortfall', `${px}px`);
  }, shortfall);
  await page.waitForTimeout(50);
  const extended = await page.evaluate((px) => {
    const nav = document.querySelector('nav.vnext-nav');
    const app = document.querySelector('.vnext-app.vnext-app-chat-home');
    const navBox = nav ? nav.getBoundingClientRect() : null;
    const buttons = [...document.querySelectorAll('nav.vnext-nav button')];
    const labelBottom = buttons.reduce((max, button) => Math.max(max, button.getBoundingClientRect().bottom), 0);
    const probe = document.createElement('div');
    probe.style.cssText = 'position:fixed;visibility:hidden;padding-bottom:env(safe-area-inset-bottom, 0px);';
    document.body.appendChild(probe);
    const bottomInset = parseFloat(getComputedStyle(probe).paddingBottom) || 0;
    probe.remove();
    return {
      navEdge: navBox ? navBox.bottom : null,
      viewport: window.innerHeight,
      appHeight: app ? app.getBoundingClientRect().height : null,
      labelBottom,
      bottomInset,
      expected: window.innerHeight + px,
    };
  }, shortfall);
  if (extended.navEdge !== null && Math.abs(extended.navEdge - extended.expected) <= 2) {
    ok('tab bar fills the leftover keyboard inset', `${extended.navEdge}px`);
  } else fail('tab bar still floats above the leftover inset', JSON.stringify(extended));
  // Labels stay where they were: one home-indicator inset above the layout
  // viewport. The leftover is padding below them, not a shove onto the indicator.
  const held = extended.viewport - extended.bottomInset - extended.labelBottom;
  if (extended.labelBottom > 0 && held >= -2 && held <= 8) {
    ok('labels stay clear of the home indicator when the bar fills the leftover', `${held}px`);
  } else fail('filling the leftover moved the labels onto the home indicator', JSON.stringify(extended));
  await page.evaluate(() => {
    document.documentElement.style.removeProperty('--layout-shortfall');
  });

  const send = page.locator('.vnext-chat-composer button[type="submit"]').first();
  const reason = page.locator('.vnext-chat-composer-send-reason');
  if (await send.isDisabled()) ok('empty send stays disabled');
  else fail('empty send is enabled');
  const placeholder = await page.locator('.vnext-chat-composer textarea').first().getAttribute('placeholder');
  if (placeholder && placeholder.trim()) ok('placeholder already asks for text', placeholder);
  else fail('composer placeholder is empty');
  if (await reason.count() === 0) ok('floating send hint is gone');
  else fail('floating send hint is still visible', (await reason.first().textContent()) || '');

  const actions = page.locator('.vnext-chat-notice-actions button');
  await actions.first().waitFor({ state: 'visible', timeout: 15000 });
  const actionCount = await actions.count();
  const actionText = ((await page.locator('.vnext-chat-notice-actions').innerText()) || '').replace(/\s+/g, ' ').trim();
  const labels = ['完成', '稍后提醒', '明天', '删除'];
  if (actionCount === 4 && labels.every((label) => actionText.includes(label))) ok('网页操作 is four separate actions', actionText);
  else fail('网页操作 is not laid out as separate actions', JSON.stringify({ actionCount, actionText }));
  if (!/网页操作|done reminder|snooze reminder|tomorrow reminder|delete reminder/.test(actionText)) ok('action row is not one crushed command sentence');
  else fail('action row still shows the raw command sentence', actionText);
  const actionBoxes = await actions.evaluateAll((nodes) => nodes.map((node) => {
    const label = node.querySelector('strong');
    const labelText = (label?.textContent || '').trim();
    return {
      nowrap: getComputedStyle(node).whiteSpace === 'nowrap',
      lines: label ? label.getClientRects().length : 0,
      labelText,
    };
  }));
  const separate = actionBoxes.length === 4
    && actionBoxes.every((box) => box.nowrap && box.lines === 1 && box.labelText && !box.labelText.includes('reminder'));
  if (separate) ok('each web action is its own line');
  else fail('web actions are still one crushed line', JSON.stringify(actionBoxes));

  const sources = page.locator('.tone-evidence summary').first();
  await sources.click();
  const sourceRow = page.locator('.vnext-source-evidence-row').first();
  await sourceRow.waitFor({ state: 'visible', timeout: 10000 });
  const sourceText = ((await sourceRow.innerText()) || '').trim();
  if (!/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(sourceText) && /\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}/.test(sourceText)) {
    ok('source row shows a wall clock, not an ISO instant', sourceText);
  } else fail('source row still shows a raw ISO instant', sourceText);

  const complete = actions.filter({ hasText: '完成' }).first();
  await complete.click();
  const receipt = page.locator('.vnext-chat-action-disclosure .vnext-chat-work-action-note').first();
  await receipt.waitFor({ state: 'visible', timeout: 15000 });
  const receiptText = ((await receipt.innerText()) || '').trim();
  if (/还会继续/.test(receiptText) && !/marked complete|Task #/i.test(receiptText)) {
    ok('action receipt follows the interface language', receiptText);
  } else fail('action receipt is still an English ledger sentence', receiptText);

  const textarea = page.locator('.vnext-chat-composer textarea').first();
  await textarea.click();
  await textarea.fill('键盘');
  await page.waitForTimeout(200);
  const resting = await page.evaluate(() => {
    const composer = document.querySelector('.vnext-chat-composer');
    const nav = document.querySelector('nav.vnext-nav');
    return {
      open: document.documentElement.classList.contains('is-keyboard-open'),
      navHidden: nav ? getComputedStyle(nav).display === 'none' : true,
      composerBottom: composer ? composer.getBoundingClientRect().bottom : NaN,
      navTop: nav ? nav.getBoundingClientRect().top : NaN,
    };
  });
  if (!resting.open && !resting.navHidden && resting.composerBottom <= resting.navTop + 2) {
    ok('composer rests above the tab bar while the keyboard is closed');
  } else fail('composer is not in its resting place', JSON.stringify(resting));

  const composerBeforeMenu = await page.locator('.vnext-chat-composer').first().boundingBox();
  await page.locator('.vnext-header-menu-trigger').first().click();
  await page.locator('.vnext-header-menu-panel').waitFor({ state: 'visible', timeout: 5000 });
  const composerWithMenu = await page.locator('.vnext-chat-composer').first().boundingBox();
  const menuShift = composerBeforeMenu && composerWithMenu
    ? Math.abs(composerWithMenu.y - composerBeforeMenu.y)
    : Number.POSITIVE_INFINITY;
  if (menuShift <= 1) ok('opening the header menu does not move the composer', `${menuShift}px`);
  else fail('opening the header menu moved the composer', JSON.stringify({ before: composerBeforeMenu, after: composerWithMenu }));
  await page.keyboard.press('Escape');
  await page.locator('.vnext-header-menu-panel').waitFor({ state: 'hidden', timeout: 5000 });
  // Opening the menu focuses a menuitem. The keyboard lift only runs while
  // the composer owns focus, so put the caret back before the covered-band fixture.
  await textarea.click();

  // 320 is a scenario fixture for a covered band, not a phone height.
  const overlap = 320;
  await page.evaluate((covered) => {
    window.__aegisKeyboardViewport = { height: window.innerHeight - covered, offsetTop: 0 };
    window.visualViewport?.dispatchEvent(new Event('resize'));
  }, overlap);
  await page.waitForFunction(() => document.documentElement.classList.contains('is-keyboard-open'), undefined, { timeout: 3000 });
  const lifted = await page.evaluate((expected) => {
    const composer = document.querySelector('.vnext-chat-composer');
    const nav = document.querySelector('nav.vnext-nav');
    const app = document.querySelector('.vnext-app.vnext-app-chat-home');
    const box = composer ? composer.getBoundingClientRect() : null;
    const inset = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--keyboard-inset')) || 0;
    const visible = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--keyboard-visible')) || 0;
    const rules = [];
    const walk = (list) => {
      for (const rule of list) {
        if (rule.selectorText && rule.selectorText.includes('is-keyboard-open')) rules.push(rule.cssText);
        if (rule.cssRules) walk(rule.cssRules);
      }
    };
    for (const sheet of document.styleSheets) {
      try { walk(sheet.cssRules); } catch { /* cross-origin sheet */ }
    }
    const css = rules.join('\n');
    return {
      inset,
      visible,
      expected,
      composerBottom: box ? box.bottom : NaN,
      composerTop: box ? box.top : NaN,
      viewport: window.innerHeight,
      navHidden: !nav || getComputedStyle(nav).display === 'none',
      pad: app ? (parseFloat(getComputedStyle(app).paddingBottom) || 0) : NaN,
      usesVisible: css.includes('var(--keyboard-visible'),
      subtractsInset: /100dvh\s*-\s*var\(--keyboard-inset/.test(css),
      addsSafeBand: /is-keyboard-open[^}]*safe-area-inset-bottom/.test(css),
      hardcodedPhone: /874px|852px|393px/.test(css),
    };
  }, overlap);
  const gap = lifted.visible - lifted.composerBottom;
  if (
    lifted.inset === overlap
    && lifted.visible === lifted.viewport - overlap
    && lifted.composerTop >= 0
    && gap >= -2
    && gap <= 12
    && lifted.navHidden
    && lifted.pad === 0
    && lifted.usesVisible
    && !lifted.subtractsInset
    && !lifted.addsSafeBand
    && !lifted.hardcodedPhone
  ) {
    ok('open keyboard does not cover the composer', `gap=${gap.toFixed(1)}px inset=${lifted.inset}px`);
  } else fail('keyboard covers the composer or leaves a safe-area band', JSON.stringify({ ...lifted, gap }));

  // The layout viewport has already shrunk to the visual viewport, the way
  // interactive-widget=resizes-content does. Overlap against innerHeight is 0.
  // The composer must still sit on that edge, and the tab bar must stay hidden.
  const fullHeight = lifted.viewport;
  await page.setViewportSize({ width: 390, height: fullHeight - overlap });
  await page.evaluate(() => {
    window.__aegisKeyboardViewport = { height: window.innerHeight, offsetTop: 0 };
    window.visualViewport?.dispatchEvent(new Event('resize'));
  });
  await page.waitForTimeout(100);
  const shrunk = await page.evaluate(() => {
    const composer = document.querySelector('.vnext-chat-composer');
    const nav = document.querySelector('nav.vnext-nav');
    const app = document.querySelector('.vnext-app.vnext-app-chat-home');
    const box = composer ? composer.getBoundingClientRect() : null;
    return {
      open: document.documentElement.classList.contains('is-keyboard-open'),
      viewport: window.innerHeight,
      visible: parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--keyboard-visible')) || 0,
      composerBottom: box ? box.bottom : NaN,
      navHidden: !nav || getComputedStyle(nav).display === 'none',
      pad: app ? (parseFloat(getComputedStyle(app).paddingBottom) || 0) : NaN,
    };
  });
  const shrunkGap = shrunk.viewport - shrunk.composerBottom;
  if (
    shrunk.open
    && shrunk.visible === shrunk.viewport
    && shrunkGap >= -2
    && shrunkGap <= 12
    && shrunk.navHidden
    && shrunk.pad === 0
  ) {
    ok('shrunk viewport does not leave a band above the keyboard', `gap=${shrunkGap.toFixed(1)}px`);
  } else fail('keyboard-open gap came back after the viewport shrunk', JSON.stringify({ ...shrunk, shrunkGap }));
  await shot('04-phone-keyboard-composer');

  await page.setViewportSize({ width: 390, height: fullHeight });

  await page.evaluate(() => {
    delete window.__aegisKeyboardViewport;
    window.visualViewport?.dispatchEvent(new Event('resize'));
  });
  await textarea.blur();
  await page.waitForFunction(() => !document.documentElement.classList.contains('is-keyboard-open'), undefined, { timeout: 3000 });
  const restored = await page.evaluate(() => {
    const composer = document.querySelector('.vnext-chat-composer');
    const nav = document.querySelector('nav.vnext-nav');
    return {
      open: document.documentElement.classList.contains('is-keyboard-open'),
      navHidden: nav ? getComputedStyle(nav).display === 'none' : true,
      composerBottom: composer ? composer.getBoundingClientRect().bottom : NaN,
      navTop: nav ? nav.getBoundingClientRect().top : NaN,
    };
  });
  if (!restored.open && !restored.navHidden && restored.composerBottom <= restored.navTop + 2) {
    ok('composer returns above the tab bar when the keyboard closes');
  } else fail('composer did not return to its resting place', JSON.stringify(restored));

  const chatEdges = await phoneEdgeGaps(page);

  await page.locator('.vnext-x-open-strip').scrollIntoViewIfNeeded();
  await shot('01-phone-chrome-decoded-handoff');
  await page.locator('.vnext-chat-notice-actions').scrollIntoViewIfNeeded();
  await shot('02-phone-web-actions');

  await page.getByRole('button', { name: '工作' }).click();
  await page.locator('.vnext-app-work').waitFor({ state: 'visible', timeout: 10000 });
  const workEdges = await phoneEdgeGaps(page);
  const near = (a, b) => Number.isFinite(a) && Number.isFinite(b) && Math.abs(a - b) <= 1;
  const edgesMatch = chatEdges && workEdges
    && near(chatEdges.insetLeft, 12)
    && near(chatEdges.insetRight, 12)
    && near(chatEdges.streamInsetLeft, chatEdges.insetLeft)
    && near(chatEdges.streamInsetRight, chatEdges.insetRight)
    && near(chatEdges.gapUnderHeader, 12)
    && near(chatEdges.gapAboveTab, 0)
    && near(workEdges.insetLeft, chatEdges.insetLeft)
    && near(workEdges.insetRight, chatEdges.insetRight)
    && near(workEdges.gapUnderHeader, chatEdges.gapUnderHeader)
    && near(workEdges.gapAboveTab, chatEdges.gapAboveTab);
  if (edgesMatch) {
    ok(
      'work matches chat edge gaps',
      `inset=${chatEdges.insetLeft}px underHeader=${chatEdges.gapUnderHeader}px aboveTab=${chatEdges.gapAboveTab}px`,
    );
  } else {
    fail('work and chat edge gaps differ', JSON.stringify({ chatEdges, workEdges }));
  }
  const phoneTabs = await navGlyphEdges(page, 'nav.vnext-nav button');
  const phoneIconsAlign = phoneTabs.length === 2
    && near(phoneTabs[0].svgL - phoneTabs[0].btnL, phoneTabs[1].svgL - phoneTabs[1].btnL)
    && near(phoneTabs[0].inkFromSvg, phoneTabs[1].inkFromSvg);
  if (phoneIconsAlign) {
    ok(
      'phone tab icons share a left edge',
      `offset=${(phoneTabs[0].svgL - phoneTabs[0].btnL).toFixed(1)}px ink=${phoneTabs[0].inkFromSvg.toFixed(1)}px`,
    );
  } else fail('phone tab icons do not share a left edge', JSON.stringify(phoneTabs));
  // The same title is also rendered inside the folded queue brief, the desktop
  // lane (display:none on a phone), and a dormant chat sheet. Those copies are
  // hidden. The owner-visible row is the open mobile follow-up lane.
  const loop = page.locator('.vnext-work-mobile-inline-lanes .vnext-item-title', { hasText: '上传文章' }).first();
  try {
    await loop.waitFor({ state: 'visible', timeout: 20000 });
    await loop.scrollIntoViewIfNeeded();
    ok('clock-loop reminder is on the work page');
    await shot('03-phone-work-loop-reminder');
  } catch (error) {
    fail('clock-loop reminder is missing from the work page', error instanceof Error ? error.message : String(error));
  }

  await page.setViewportSize({ width: 1280, height: 800 });
  await page.waitForTimeout(200);
  const workRail = await navGlyphEdges(page, 'nav.vnext-rail-nav button');
  const workChrome = await desktopChromeBox(page);
  await page.locator('nav.vnext-rail-nav').getByRole('button', { name: '安琪' }).click();
  await page.locator('.vnext-app-chat-home').waitFor({ state: 'visible', timeout: 10000 });
  await page.waitForTimeout(700);
  const chatRail = await navGlyphEdges(page, 'nav.vnext-rail-nav button');
  const chatChrome = await desktopChromeBox(page);
  const railLeft = (rows) => rows.length === 2 && near(rows[0].svgL, rows[1].svgL) && near(rows[0].inkFromSvg, rows[1].inkFromSvg);
  const railsMatch = railLeft(workRail) && railLeft(chatRail) && near(workRail[0].svgL, chatRail[0].svgL);
  if (railsMatch) {
    ok('sidebar icons share a left edge on both pages', `${workRail[0].svgL.toFixed(1)}px`);
  } else fail('sidebar icons do not share a left edge', JSON.stringify({ workRail, chatRail }));
  const railsFlush = workChrome && chatChrome
    && near(workChrome.railT, 0) && near(chatChrome.railT, 0)
    && near(workChrome.railB, workChrome.vh) && near(chatChrome.railB, chatChrome.vh)
    && near(workChrome.barH, 52) && near(chatChrome.barH, 52)
    && near(chatChrome.columnB, chatChrome.vh);
  if (railsFlush) {
    ok('sidebar and header share the console edges', `rail=0..${workChrome.vh}px bar=52px`);
  } else fail('sidebar or header edges differ', JSON.stringify({ workChrome, chatChrome }));
}
