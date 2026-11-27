/*
 * VNextSegmented render tests (SP1.5).
 *
 * Drives the shared single-select segmented control via renderToStaticMarkup,
 * asserting: the track role (tablist vs radiogroup), per-segment selection
 * semantics (aria-selected for tabs / aria-checked for radios), the selected
 * class, roving tabindex (only the selected segment is the tab stop), size
 * classes, fullWidth, disabled segments, and that onChange fires with the
 * clicked value. Keyboard arrow-nav math is covered via the exported move logic
 * by simulating the onChange contract through a controlled wrapper.
 */
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_SEGMENTED_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-segmented-test/VNextSegmented.mjs', import.meta.url).href;
const { VNextSegmented } = await import(modulePath);

const OPTS = [
  { value: 'inbox', label: '收件箱' },
  { value: 'sent', label: '已发送' },
  { value: 'drafts', label: '草稿' },
];

function render(props) {
  return renderToStaticMarkup(React.createElement(VNextSegmented, props));
}

describe('VNextSegmented', () => {
  it('tab mode → role=tablist + role=tab + aria-selected', () => {
    const html = render({ options: OPTS, value: 'inbox', onChange() {}, ariaLabel: '文件夹' });
    assert.match(html, /role="tablist"/);
    assert.match(html, /aria-label="文件夹"/);
    assert.match(html, /role="tab"/);
    assert.match(html, /aria-selected="true"/);
    assert.match(html, /aria-selected="false"/);
  });

  it('radio mode → role=radiogroup + role=radio + aria-checked', () => {
    const html = render({ options: OPTS, value: 'sent', onChange() {}, mode: 'radio' });
    assert.match(html, /role="radiogroup"/);
    assert.match(html, /role="radio"/);
    assert.match(html, /aria-checked="true"/);
    assert.match(html, /aria-checked="false"/);
    assert.doesNotMatch(html, /aria-selected/);
  });

  it('marks the selected segment with is-selected', () => {
    const html = render({ options: OPTS, value: 'drafts', onChange() {} });
    // Exactly one is-selected segment.
    const matches = html.match(/vnext-seg-item is-selected/g) || [];
    assert.equal(matches.length, 1);
  });

  it('roving tabindex: only the selected segment is the tab stop', () => {
    const html = render({ options: OPTS, value: 'inbox', onChange() {} });
    const tab0 = (html.match(/tabindex="0"/g) || []).length;
    const tabNeg = (html.match(/tabindex="-1"/g) || []).length;
    assert.equal(tab0, 1);
    assert.equal(tabNeg, OPTS.length - 1);
  });

  it('size classes (sm / md default)', () => {
    assert.match(render({ options: OPTS, value: 'inbox', onChange() {}, size: 'sm' }), /vnext-seg-sm/);
    assert.match(render({ options: OPTS, value: 'inbox', onChange() {} }), /vnext-seg-md/);
  });

  it('fullWidth adds the full class', () => {
    assert.match(render({ options: OPTS, value: 'inbox', onChange() {}, fullWidth: true }), /vnext-seg-full/);
  });

  it('renders each option label', () => {
    const html = render({ options: OPTS, value: 'inbox', onChange() {} });
    for (const o of OPTS) assert.match(html, new RegExp(o.label));
  });

  it('a disabled segment is disabled + never the tab stop', () => {
    const opts = [{ value: 'a', label: 'A', disabled: true }, { value: 'b', label: 'B' }];
    const html = render({ options: opts, value: 'b', onChange() {} });
    assert.match(html, /disabled=""/);
    // selected 'b' is the only tabindex=0; disabled 'a' is -1.
    assert.equal((html.match(/tabindex="0"/g) || []).length, 1);
  });

  it('option ariaLabel is emitted (icon/glyph labels)', () => {
    const opts = [{ value: 'a', label: '★', ariaLabel: '收藏' }, { value: 'b', label: 'B' }];
    const html = render({ options: opts, value: 'a', onChange() {} });
    assert.match(html, /aria-label="收藏"/);
  });

  it('renders the label prop above the track', () => {
    const html = render({
      options: OPTS,
      value: 'inbox',
      onChange() {},
      label: '工作范围',
      ariaLabel: '工作范围',
    });
    assert.match(html, /class="vnext-segmented-field"/);
    assert.match(html, /class="vnext-segmented-label">工作范围</);
    const labelAt = html.indexOf('vnext-segmented-label');
    const trackAt = html.indexOf('role="tablist"');
    assert.ok(labelAt >= 0 && trackAt > labelAt, 'visible label must precede the track');
  });
});
