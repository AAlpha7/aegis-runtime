// Focusing the composer is not a keyboard. The field stays above the tab bar
// until a visual-viewport overlap is reported, then sits on that visible edge
// and returns when the overlap drops to zero.
export const id = 'chat-composer-typing';
export const title = 'Chat composer stays above an open keyboard and returns when it closes';
export const surfaces = ['home'];

async function measure(page) {
  return page.evaluate(() => {
    const app = document.querySelector('.vnext-app-chat-home');
    const nav = document.querySelector('nav.vnext-nav');
    const composer = document.querySelector('.vnext-chat-composer');
    const rect = composer ? composer.getBoundingClientRect() : null;
    const navRect = nav ? nav.getBoundingClientRect() : null;
    return {
      keyboardOpen: document.documentElement.classList.contains('is-keyboard-open'),
      navHidden: !nav || getComputedStyle(nav).display === 'none',
      appPaddingBottom: app ? parseFloat(getComputedStyle(app).paddingBottom) : NaN,
      composerBottom: rect ? rect.bottom : NaN,
      navTop: navRect ? navRect.top : NaN,
      inset: parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--keyboard-inset')) || 0,
      viewportHeight: window.innerHeight,
      composing: Boolean(composer && composer.classList.contains('is-composing')),
    };
  });
}

export async function run({ page, ok, fail, shot }) {
  await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
  await page.locator('nav.vnext-nav').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(1200);
  const textarea = page.locator('.vnext-chat-composer textarea').first();
  await textarea.waitFor({ state: 'visible', timeout: 15000 });

  await textarea.click();
  await textarea.type('测试');
  await page.waitForTimeout(300);
  const typing = await measure(page);
  await shot('01-typing');
  if (!typing.composing) fail('composer did not enter is-composing on focus + text');
  else ok('composer enters is-composing while typing');
  if (!typing.keyboardOpen && !typing.navHidden && typing.composerBottom <= typing.navTop + 2) {
    ok('typing without a keyboard keeps the composer above the tab bar');
  } else fail('typing without a keyboard moved the composer', JSON.stringify(typing));

  const overlap = 320;
  await page.evaluate((covered) => {
    window.__aegisKeyboardViewport = { height: window.innerHeight - covered, offsetTop: 0 };
    window.visualViewport?.dispatchEvent(new Event('resize'));
  }, overlap);
  await page.waitForFunction(() => document.documentElement.classList.contains('is-keyboard-open'), undefined, { timeout: 3000 });
  const lifted = await measure(page);
  await shot('02-keyboard');
  const gap = lifted.viewportHeight - lifted.inset - lifted.composerBottom;
  if (lifted.keyboardOpen && lifted.navHidden && lifted.appPaddingBottom === 0 && gap >= -2 && gap <= 12) {
    ok('open keyboard does not cover the composer', `gap=${gap.toFixed(1)}px`);
  } else fail('keyboard covers the composer', JSON.stringify({ ...lifted, gap }));

  await page.evaluate(() => {
    delete window.__aegisKeyboardViewport;
    window.visualViewport?.dispatchEvent(new Event('resize'));
  });
  await textarea.fill('');
  await textarea.blur();
  await page.waitForFunction(() => !document.documentElement.classList.contains('is-keyboard-open'), undefined, { timeout: 3000 });
  await page.waitForTimeout(200);
  const after = await measure(page);
  await shot('03-restored');
  if (!after.keyboardOpen && !after.composing && !after.navHidden && after.composerBottom <= after.navTop + 2) {
    ok('composer returns above the tab bar when the keyboard closes');
  } else fail('layout not restored after the keyboard closes', JSON.stringify(after));
}
