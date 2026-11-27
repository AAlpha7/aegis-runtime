/*
 * VNextButton render tests (SP1).
 *
 * Drives the shared action-button component via renderToStaticMarkup, asserting:
 * every variant + size renders its class, iconOnly hides the label and keeps the
 * required aria-label, loading sets aria-busy + disables, the anchor mode renders
 * <a href>, and leading/trailing icons render their slots.
 */
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_BUTTON_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-button-test/VNextButton.mjs', import.meta.url).href;
const { VNextButton } = await import(modulePath);

const ICON = React.createElement('svg', { 'data-test-icon': 'x' });

function renderButton(props) {
  return renderToStaticMarkup(React.createElement(VNextButton, props));
}

describe('VNextButton', () => {
  it('renders each variant class', () => {
    for (const variant of ['primary', 'cta', 'secondary', 'ghost', 'destructive']) {
      const html = renderButton({ variant, children: 'x' });
      assert.match(html, new RegExp(`vnext-btn-${variant}`));
      assert.match(html, /class="vnext-btn /);
    }
  });

  it('renders each size class', () => {
    for (const size of ['sm', 'md', 'lg', 'icon']) {
      const html = renderButton({ size, children: 'x' });
      assert.match(html, new RegExp(`vnext-btn-${size}`));
    }
  });

  it('defaults to primary + md', () => {
    const html = renderButton({ children: 'x' });
    assert.match(html, /vnext-btn-primary/);
    assert.match(html, /vnext-btn-md/);
  });

  it('variant + size classes (cta lg)', () => {
    const html = renderButton({ variant: 'cta', size: 'lg', children: '部署' });
    assert.match(html, /vnext-btn-cta/);
    assert.match(html, /vnext-btn-lg/);
    assert.match(html, /部署/);
  });

  it('iconOnly hides the label + keeps the aria-label', () => {
    const html = renderButton({ iconOnly: true, size: 'icon', 'aria-label': '添加', children: ICON });
    assert.match(html, /vnext-btn-icononly/);
    assert.match(html, /aria-label="添加"/);
    // No label span is rendered in icon-only mode (the icon goes in vnext-btn-ico).
    assert.doesNotMatch(html, /vnext-btn-label/);
    assert.match(html, /vnext-btn-ico/);
  });

  it('non-iconOnly renders a label span around children', () => {
    const html = renderButton({ children: '保存' });
    assert.match(html, /<span class="vnext-btn-label">保存<\/span>/);
  });

  it('loading sets aria-busy + disabled + spinner, no label-icon leak', () => {
    const html = renderButton({ loading: true, leadingIcon: ICON, children: '保存' });
    assert.match(html, /aria-busy="true"/);
    assert.match(html, /disabled=""/);
    assert.match(html, /vnext-btn-spinner/);
    assert.match(html, /vnext-btn-loading/);
    // leadingIcon must NOT render while loading (spinner replaces it).
    assert.doesNotMatch(html, /data-test-icon/);
  });

  it('disabled (not loading) sets disabled but not aria-busy', () => {
    const html = renderButton({ disabled: true, children: 'x' });
    assert.match(html, /disabled=""/);
    assert.doesNotMatch(html, /aria-busy/);
  });

  it('fullWidth adds the full class', () => {
    const html = renderButton({ fullWidth: true, children: 'x' });
    assert.match(html, /vnext-btn-full/);
  });

  it('leading + trailing icons render their slots', () => {
    const html = renderButton({ leadingIcon: ICON, trailingIcon: ICON, children: 'x' });
    const slots = html.match(/vnext-btn-ico/g) || [];
    assert.equal(slots.length, 2);
  });

  it('renders a button element with type=button by default', () => {
    const html = renderButton({ children: 'x' });
    assert.match(html, /^<button /);
    assert.match(html, /type="button"/);
  });

  it('honors an explicit type=submit', () => {
    const html = renderButton({ type: 'submit', children: 'x' });
    assert.match(html, /type="submit"/);
  });

  it('as="a" renders an anchor with href + merges className', () => {
    const html = renderButton({ as: 'a', href: 'https://x.example', className: 'extra', children: 'x' });
    assert.match(html, /^<a /);
    assert.match(html, /href="https:\/\/x\.example"/);
    assert.match(html, /vnext-btn /);
    assert.match(html, /extra/);
  });

  it('forwards arbitrary props (title, data-*) to the button', () => {
    const html = renderButton({ title: '提示', 'data-x': 'y', children: 'x' });
    assert.match(html, /title="提示"/);
    assert.match(html, /data-x="y"/);
  });
});
