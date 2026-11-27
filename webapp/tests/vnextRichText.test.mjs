import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_RICH_TEXT_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-rich-text-test/RichText.js';
const { RichText, richTextImportanceTestHooks } = await import(modulePath);

const localeModulePath = process.env.VNEXT_LOCALE_CONTEXT_MODULE
  ?? new URL('../i18n/LocaleContext.js', modulePath).href;
const { LocaleProvider } = await import(localeModulePath);

function render(content) {
  return renderToStaticMarkup(React.createElement(RichText, { content }));
}

function renderWithProps(props) {
  return renderToStaticMarkup(React.createElement(RichText, props));
}

function renderWithLocale(displayLanguage, props) {
  return renderToStaticMarkup(
    React.createElement(LocaleProvider, { displayLanguage }, React.createElement(RichText, props)),
  );
}

describe('vNext rich text', () => {
  it('preserves decimal measurements, dotted dates and versions as facts', () => {
    for (const fact of ['24.5°C，晴', '11.5 km remaining', '2026.09.18', '1.2.3', '1.example']) {
      const html = render(fact);
      assert.ok(html.includes(`<p>${fact}</p>`), html);
      assert.doesNotMatch(html, /<ol>/);
    }
  });

  it('preserves every item in order when list types change without blank lines', () => {
    for (const content of [
      '- retained fact\n1. next instruction\n- final fact',
      '1. retained fact\n- next instruction\n2. final fact',
    ]) {
      const html = render(content);
      assert.deepEqual([...html.matchAll(/<li>(.*?)<\/li>/g)].map((match) => match[1]),
        ['retained fact', 'next instruction', 'final fact']);
      assert.equal((html.match(/<(?:ul|ol)(?:\s[^>]*)?>/g) ?? []).length, 3);
    }
  });

  it('retains intentional numeric, parenthesized and Chinese list markers', () => {
    for (const marker of ['1. ', '2.\t', '3)', '(4)', '（五）', '六、', '7、', '八．', '九.']) {
      assert.match(render(`${marker}保留事实`), /<ol(?:\s[^>]*)?><li>保留事实<\/li><\/ol>/);
    }
  });

  it('keeps separate source titles and URLs together and preserves numbering', () => {
    const html = render('1. Overview\nhttps://example.com/overview\n\n2. Reference\nhttps://example.com/reference');
    assert.match(html, /vnext-reference-list/);
    assert.match(html, /<li><a [^>]*href="https:\/\/example.com\/overview"[^>]*>Overview<\/a><\/li>/);
    assert.match(html, /start="2"/);
    assert.doesNotMatch(html, /<p><a/);
  });

  it('makes bare HTTPS source URLs clickable without swallowing punctuation or code', () => {
    const html = render('来源：https://example.com/report。 See (https://example.com/a). `https://example.com/code` [named](https://example.com/named)');
    for (const path of ['report','a','named']) assert.ok(html.includes(`href="https://example.com/${path}"`));
    assert.doesNotMatch(html, /href="https:\/\/example.com\/code/);
    assert.equal((html.match(/<a /g)||[]).length,3);
    assert.match(html, /<\/a>。/);
    assert.match(html, /noopener noreferrer/);
  });

  it('decodes percent-encoded UTF-8 in owner-facing link labels', () => {
    const html = render('见 https://x.com/xianba%E2%80%A6 以及 [xianba%E2%80%A6](https://x.com/alice)');
    assert.match(html, />https:\/\/x\.com\/xianba…</);
    assert.match(html, />xianba…</);
    assert.match(html, /href="https:\/\/x\.com\/xianba%E2%80%A6"/);
    assert.match(html, /href="https:\/\/x\.com\/alice"/);
  });

  it('keeps credentials-bearing bare URLs inert and retains balanced URL parentheses', () => {
    const html = render('https://user:password@example.com/secret https://example.com/wiki/Name_(topic)');
    assert.doesNotMatch(html, /href="https:\/\/user/);
    assert.match(html, /href="https:\/\/example.com\/wiki\/Name_\(topic\)"/);
  });
  it('renders assistant markdown as structured product text', () => {
    const html = render(`# Commute\n\n**Fast route**\n\n- Check traffic\n- Leave by 5:20\n\n> Use current location only after approval.`);

    assert.match(html, /<h3>Commute<\/h3>/);
    assert.match(html, /<strong>Fast route<\/strong>/);
    assert.match(html, /<ul>/);
    assert.match(html, /<li>Check traffic<\/li>/);
    assert.match(html, /<blockquote>Use current location only after approval.<\/blockquote>/);
  });

  it('escapes HTML and rejects unsafe links', () => {
    const html = render(`Open [bad](javascript:alert(1)) <img src=x onerror=alert(1)> and [safe](https://example.com/a).`);

    assert.doesNotMatch(html, /javascript:/);
    assert.doesNotMatch(html, /<img/);
    assert.match(html, /&lt;img src=x onerror=alert\(1\)&gt;/);
    assert.match(html, /href="https:\/\/example.com\/a"/);
  });

  it('keeps code blocks readable without executing content', () => {
    const html = render('```html\n<script>alert(1)</script>\n```');

    // Default (no LocaleProvider) locale is zh: the code fence label is localized.
    assert.match(html, /<figcaption>代码 · html<\/figcaption>/);
    assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  });

  it('renders markdown tables as safe responsive tables', () => {
    const html = render([
      '| Item | Status | Next |',
      '| --- | --- | --- |',
      '| Gmail | blocked | **Reconnect** |',
      '| Commute | ready | [open](https://example.com/route) |',
    ].join('\n'));

    assert.match(html, /class="vnext-rich-table-scroll"/);
    assert.match(html, /<th>Item<\/th>/);
    assert.match(html, /<td>Gmail<\/td>/);
    assert.match(html, /<strong>Reconnect<\/strong>/);
    assert.match(html, /href="https:\/\/example.com\/route"/);
  });

  it('renders checklist bullets as visual status rows without interactive controls', () => {
    const html = render([
      '- [ ] Confirm the route',
      '- [x] Draft the reply',
    ].join('\n'));

    assert.match(html, /class="vnext-rich-check-item"/);
    assert.match(html, /class="vnext-rich-check-item is-checked"/);
    assert.match(html, /Confirm the route/);
    assert.match(html, /Draft the reply/);
    assert.doesNotMatch(html, /<input/);
  });

  it('renders common Chinese assistant bullets and numbered lists structurally', () => {
    const html = render([
      '• 邮件：需要回复',
      '• 提醒：明天检查',
      '',
      '1、先确认账单',
      '2、再归档邮件',
      '',
      '一、确认路线',
      '二、发送提醒',
      '',
      '（一）查看邮件',
      '(二) 安排跟进',
    ].join('\n'));

    assert.match(html, /<ul>/);
    assert.match(html, /<li>邮件：需要回复<\/li>/);
    assert.match(html, /<li>提醒：明天检查<\/li>/);
    assert.match(html, /<ol>/);
    assert.match(html, /<li>先确认账单<\/li>/);
    assert.match(html, /<li>再归档邮件<\/li>/);
    assert.match(html, /<li>确认路线<\/li>/);
    assert.match(html, /<li>发送提醒<\/li>/);
    assert.match(html, /<li>查看邮件<\/li>/);
    assert.match(html, /<li>安排跟进<\/li>/);
  });

  it('renders important information as a reversible visual callout pattern', () => {
    const html = render([
      '系统已重启 / System restarted',
      '当前重要信息',
      '活跃技能 / Active skills: 22',
      'HITL 待审批: 0',
    ].join('\n'));

    assert.match(html, /class="vnext-rich-callout important"/);
    assert.match(html, /‼️/);
    assert.match(html, /<strong><span aria-hidden="true">‼️<\/span> 系统重启<\/strong>/);
    // Default (no LocaleProvider) locale is zh: the Dismiss button is localized.
    assert.match(html, />忽略<\/button>/);
    assert.match(html, /<li>系统已重启<\/li>/);
    assert.match(html, /<li>活跃技能: 22<\/li>/);
    assert.match(html, /<li>待确认: 0<\/li>/);
    assert.doesNotMatch(html, /System restarted/);
    assert.doesNotMatch(html, /Active skills/);
    assert.doesNotMatch(html, /HITL/);
    assert.doesNotMatch(html, /当前重要信息<\/p>/);
  });

  it('uses deterministic important categories for reminders and resource pressure', () => {
    const reminderHtml = render('⏰ 延长公司 Therapist Appointment 周期期限');
    assert.match(reminderHtml, /<strong><span aria-hidden="true">‼️<\/span> 提醒<\/strong>/);
    assert.match(reminderHtml, /<li>⏰ 延长公司 Therapist Appointment 周期期限<\/li>/);

    const resourceHtml = render([
      '🚨 [Aegis Runtime Alert] server_monitor',
      'ALERTS:',
      'Disk usage 86.9% exceeds threshold 70%',
      'Full metrics: CPU normal, Memory normal',
    ].join('\n'));
    assert.match(resourceHtml, /<strong><span aria-hidden="true">‼️<\/span> 警报<\/strong>/);
    assert.match(resourceHtml, /Disk usage 86.9% exceeds threshold 70%/);
    assert.match(resourceHtml, /Full metrics: CPU normal, Memory normal/);
  });

  it('rebuilds important formatting from raw persisted text after refresh', () => {
    const persistedText = [
      '系统已重启 / System restarted',
      '当前重要信息',
      'HITL 待审批: 0',
    ].join('\n');

    assert.equal(render(persistedText), render(persistedText));
    assert.match(render(persistedText), /class="vnext-rich-callout important"/);
  });

  it('uses structured severity as a persisted importance channel without keyword text', () => {
    const html = renderWithProps({
      content: 'Provider balance is below your threshold.',
      importanceSeverity: 'important',
      importanceDismissKey: 'provider-balance',
    });

    assert.match(html, /class="vnext-rich-callout important"/);
    // Rendered without a LocaleProvider the default locale is zh, so the generic
    // "Important" callout label resolves to the catalog's Chinese value.
    assert.match(html, /<strong><span aria-hidden="true">‼️<\/span> 重要<\/strong>/);
    assert.match(html, /Provider balance is below your threshold/);
    assert.match(html, />忽略<\/button>/);
  });

  it('localizes the generic important callout label for both locales', () => {
    const props = {
      content: 'Provider balance is below your threshold.',
      importanceSeverity: 'important',
      importanceDismissKey: 'provider-balance-i18n',
    };

    const zh = renderWithLocale('zh', props);
    const en = renderWithLocale('en', props);

    assert.match(zh, /<strong><span aria-hidden="true">‼️<\/span> 重要<\/strong>/);
    assert.match(en, /<strong><span aria-hidden="true">‼️<\/span> Important<\/strong>/);
    assert.doesNotMatch(zh, /‼️<\/span> Important<\/strong>/);
  });

  it('localizes the warning and critical severity callout titles (no English leak in zh)', () => {
    // Owner complaint: the error card's "WARNING" badge leaked English into a zh
    // session. The severity callout title must resolve through the catalog.
    const warningProps = {
      content: '我没能完成这件事。请重试。',
      importanceSeverity: 'warning',
      importanceDismissKey: 'severity-warning-callout',
    };
    const zhWarn = renderWithLocale('zh', warningProps);
    const enWarn = renderWithLocale('en', warningProps);
    assert.match(zhWarn, /‼️<\/span> 警告<\/strong>/);
    assert.match(enWarn, /‼️<\/span> Warning<\/strong>/);
    assert.doesNotMatch(zhWarn, /Warning<\/strong>/);

    const criticalProps = {
      content: 'Something critical happened.',
      importanceSeverity: 'critical',
      importanceDismissKey: 'severity-critical-callout',
    };
    const zhCrit = renderWithLocale('zh', criticalProps);
    const enCrit = renderWithLocale('en', criticalProps);
    assert.match(zhCrit, /‼️<\/span> 严重<\/strong>/);
    assert.match(enCrit, /‼️<\/span> Critical<\/strong>/);
    assert.doesNotMatch(zhCrit, /Critical<\/strong>/);
  });

  it('lets owner dismiss structured severity after refresh', () => {
    const content = 'Provider balance is below your threshold.';
    const storage = new Map();
    globalThis.window = {
      localStorage: {
        getItem: (key) => storage.get(key) ?? null,
        setItem: (key, value) => storage.set(key, value),
        removeItem: (key) => storage.delete(key),
      },
    };

    richTextImportanceTestHooks.writeImportanceDismissed('provider-balance', true);

    const html = renderWithProps({
      content,
      importanceSeverity: 'important',
      importanceDismissKey: 'provider-balance',
    });
    assert.match(html, /class="vnext-rich-callout important is-dismissed"/);
    assert.match(html, />恢复<\/button>/);

    delete globalThis.window;
  });

  it('restores dismissed importance from the stable raw text key', async () => {
    const content = '⏰ Review this reminder';
    const storage = new Map();
    globalThis.window = {
      localStorage: {
        getItem: (key) => storage.get(key) ?? null,
        setItem: (key, value) => storage.set(key, value),
        removeItem: (key) => storage.delete(key),
      },
    };

    const firstHtml = render(content);
    assert.match(firstHtml, />忽略<\/button>/);

    const key = richTextImportanceTestHooks.stableImportanceDismissKey(content);
    richTextImportanceTestHooks.writeImportanceDismissed(key, true);

    const refreshedHtml = render(content);
    assert.match(refreshedHtml, /class="vnext-rich-callout important is-dismissed"/);
    assert.match(refreshedHtml, />恢复<\/button>/);

    delete globalThis.window;
  });

  it('localizes the code fence label, dismiss/restore buttons, and dismissed title for both locales', () => {
    const codeContent = '```\nplain code\n```';
    const zhCode = renderWithLocale('zh', { content: codeContent });
    const enCode = renderWithLocale('en', { content: codeContent });
    assert.match(zhCode, /<figcaption>代码<\/figcaption>/);
    assert.match(enCode, /<figcaption>Code<\/figcaption>/);

    const calloutProps = {
      content: 'Provider balance is below your threshold.',
      importanceSeverity: 'important',
      importanceDismissKey: 'provider-balance-buttons-i18n',
    };
    const zhCallout = renderWithLocale('zh', calloutProps);
    const enCallout = renderWithLocale('en', calloutProps);
    assert.match(zhCallout, />忽略<\/button>/);
    assert.match(enCallout, />Dismiss<\/button>/);
  });

  it('localizes the empty-alt image fallback for both locales', () => {
    // An image with empty alt text falls back to the localized "Generated image"
    // (catalog key app.richText.generatedImageAlt) instead of leaking English.
    const imageContent = '![](/webapp/vnext/generated-image/sample-img.png)';
    const zhImage = renderWithLocale('zh', { content: imageContent });
    const enImage = renderWithLocale('en', { content: imageContent });
    assert.match(zhImage, /alt="生成的图片"/);
    assert.match(enImage, /alt="Generated image"/);

    // Provided alt text is preserved verbatim (no fallback substitution).
    const namedAlt = '![Route map](/webapp/vnext/generated-image/route.png)';
    const zhNamed = renderWithLocale('zh', { content: namedAlt });
    assert.match(zhNamed, /alt="Route map"/);
  });
});
