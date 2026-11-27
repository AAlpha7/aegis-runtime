/*
 * VNextChip render tests (SP1.5).
 *
 * Drives the shared filter/toggle chip via renderToStaticMarkup, asserting:
 * the base class, selected → is-selected + aria-pressed=true, unselected →
 * aria-pressed=false, size classes, leading icon slot, label rendering,
 * disabled, and that arbitrary props (aria-label, title, data-*) forward.
 */
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_CHIP_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-chip-test/VNextChip.mjs', import.meta.url).href;
const { VNextChip } = await import(modulePath);

const ICON = React.createElement('svg', { 'data-test-icon': 'x' });

function render(props) {
  return renderToStaticMarkup(React.createElement(VNextChip, props));
}

describe('VNextChip', () => {
  it('renders the base class + label', () => {
    const html = render({ children: '全部账号' });
    assert.match(html, /class="vnext-chip /);
    assert.match(html, /<span class="vnext-chip-label">全部账号<\/span>/);
  });

  it('selected → is-selected + aria-pressed=true', () => {
    const html = render({ selected: true, children: 'work' });
    assert.match(html, /is-selected/);
    assert.match(html, /aria-pressed="true"/);
  });

  it('unselected → aria-pressed=false, no is-selected', () => {
    const html = render({ children: 'work' });
    assert.match(html, /aria-pressed="false"/);
    assert.doesNotMatch(html, /is-selected/);
  });

  it('size classes (sm / md default)', () => {
    assert.match(render({ size: 'sm', children: 'x' }), /vnext-chip-sm/);
    assert.match(render({ children: 'x' }), /vnext-chip-md/);
  });

  it('leading icon renders in its slot', () => {
    const html = render({ leadingIcon: ICON, children: 'x' });
    assert.match(html, /vnext-chip-ico/);
    assert.match(html, /data-test-icon/);
  });

  it('renders a button with type=button by default', () => {
    const html = render({ children: 'x' });
    assert.match(html, /^<button /);
    assert.match(html, /type="button"/);
  });

  it('disabled sets disabled', () => {
    const html = render({ disabled: true, children: 'x' });
    assert.match(html, /disabled=""/);
  });

  it('forwards arbitrary props (aria-label, title, data-*)', () => {
    const html = render({ 'aria-label': '按账号筛选 work', title: 'work', 'data-x': 'y', children: 'work' });
    assert.match(html, /aria-label="按账号筛选 work"/);
    assert.match(html, /title="work"/);
    assert.match(html, /data-x="y"/);
  });
});
