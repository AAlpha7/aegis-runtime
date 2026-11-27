/*
 * VNextHeaderMenu render/a11y tests (SP2 chrome unification).
 *
 * Drives the shared header overflow/utility menu via renderToStaticMarkup,
 * asserting: the closed trigger's menu-button a11y contract (aria-haspopup,
 * aria-expanded, aria-label, the overflow icon, NO panel); and the open panel's
 * structure (role=menu, role=menuitem per item, labels, the disabled attribute,
 * the destructive tone class, icon rendering). The interactive layer (Escape /
 * click-outside / roving arrow-focus / focus-return) is effect-driven and not
 * SSR-observable; it is exercised manually + via the screenshot harness.
 */
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_HEADER_MENU_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-header-menu-test/VNextHeaderMenu.mjs', import.meta.url).href;
const { VNextHeaderMenu } = await import(modulePath);

const ITEMS = [
  { id: 'status', label: '状态', icon: 'status', onSelect() {} },
  { id: 'settings', label: '设置', icon: 'settings', onSelect() {} },
  { id: 'snapshot', label: '保存中', icon: 'camera', disabled: true, onSelect() {} },
  { id: 'logout', label: '退出登录', tone: 'destructive', onSelect() {} },
];

function render(props) {
  return renderToStaticMarkup(React.createElement(VNextHeaderMenu, props));
}

describe('VNextHeaderMenu', () => {
  it('closed: trigger is a menu button, no panel rendered', () => {
    const html = render({ triggerLabel: '打开菜单', items: ITEMS });
    assert.match(html, /class="vnext-header-menu-trigger"/);
    assert.match(html, /aria-haspopup="menu"/);
    assert.match(html, /aria-expanded="false"/);
    assert.match(html, /aria-label="打开菜单"/);
    assert.match(html, /class="vnext-icon-glyph"|<svg/); // the overflow icon
    assert.doesNotMatch(html, /role="menu"/);
    assert.doesNotMatch(html, /role="menuitem"/);
  });

  it('open: role=menu panel with one menuitem per item', () => {
    const html = render({ triggerLabel: '打开菜单', items: ITEMS, defaultOpen: true });
    assert.match(html, /aria-expanded="true"/);
    assert.match(html, /role="menu"/);
    assert.match(html, /aria-label="打开菜单"/);
    const menuitems = html.match(/role="menuitem"/g) ?? [];
    assert.equal(menuitems.length, ITEMS.length);
    assert.match(html, /状态/);
    assert.match(html, /设置/);
    assert.match(html, /退出登录/);
  });

  it('open: disabled item carries the disabled attribute', () => {
    const html = render({ triggerLabel: '打开菜单', items: ITEMS, defaultOpen: true });
    // The snapshot item (disabled) renders a disabled <button>.
    assert.match(html, /<button[^>]*disabled[^>]*>(?:(?!<\/button>).)*保存中/s);
  });

  it('open: destructive tone applies the destructive class', () => {
    const html = render({ triggerLabel: '打开菜单', items: ITEMS, defaultOpen: true });
    assert.match(html, /vnext-header-menu-item-destructive/);
  });

  it('open: align controls the panel side class', () => {
    const end = render({ triggerLabel: 'x', items: ITEMS, defaultOpen: true, align: 'end' });
    assert.match(end, /vnext-header-menu-end/);
    const start = render({ triggerLabel: 'x', items: ITEMS, defaultOpen: true, align: 'start' });
    assert.match(start, /vnext-header-menu-start/);
  });

  it('open: a separatorBefore row draws a separator and stays a menuitem', () => {
    const html = render({
      triggerLabel: '账户菜单',
      items: [
        { id: 'settings', label: '设置', onSelect() {} },
        { id: 'clear', label: '清空对话', tone: 'destructive', separatorBefore: true, onSelect() {} },
      ],
      defaultOpen: true,
    });
    assert.match(html, /role="separator"/);
    assert.match(html, /vnext-header-menu-item-destructive/);
    assert.equal((html.match(/role="menuitem"/g) ?? []).length, 2);
  });

  it('open: panel uses fixed stacking class so it escapes header frost context', () => {
    const html = render({ triggerLabel: '打开菜单', items: ITEMS, defaultOpen: true });
    assert.match(html, /vnext-header-menu is-open|vnext-header-menu is-open/);
    assert.match(html, /vnext-header-menu-fixed/);
  });
});
