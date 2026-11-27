import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const modulePath = process.env.VNEXT_ASSISTANT_RESPONSE_VIEW_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-assistant-response-view-test/AssistantResponseView.mjs', import.meta.url).href;
const { AssistantResponseView, assistantResponseSections, sourceEvidenceItems, SourceEvidenceContent, LocaleProvider, UserTimezoneProvider, t, tf } = await import(modulePath);

function response(overrides = {}) {
  return {
    schema_version: 1,
    response_kind: 'answer',
    visible_answer: 'Answer: Ready.',
    draft: null,
    action_draft: null,
    suggested_intent: null,
    authority: null,
    required_gate: null,
    receipt_expectation: 'No external action was executed; receipts are required before completion claims.',
    citations: [],
    warnings: [],
    ...overrides,
  };
}

// Default render pins the locale to 'en' so existing byte-exact HTML assertions
// stay valid; pass displayLanguage='zh' to exercise the Chinese chrome.
function render(result, displayLanguage = 'en', ownerQuestion = '') {
  return renderToStaticMarkup(
    React.createElement(
      LocaleProvider,
      { displayLanguage },
      React.createElement(AssistantResponseView, { result, ownerQuestion }),
    ),
  );
}

function renderInZone(result, zone, displayLanguage = 'en') {
  return renderToStaticMarkup(
    React.createElement(
      LocaleProvider,
      { displayLanguage },
      React.createElement(
        UserTimezoneProvider,
        { value: zone },
        React.createElement(AssistantResponseView, { result }),
      ),
    ),
  );
}

// The exported helpers thread an explicit locale (mirroring the component path);
// tests that assert internal section identity default to 'en'.
function secs(result, locale = 'en') {
  return assistantResponseSections(result, locale);
}

describe('vNext assistant response view', () => {
  it('preserves runtime fallback disclosures before a restored search receipt', () => {
    for (const [locale, notice] of [
      ['en', 'The preferred search is unavailable. DuckDuckGo was used; coverage and freshness may differ.'],
      ['zh', '首选搜索不可用，已使用 DuckDuckGo；来源覆盖和时效可能不同。'],
    ]) {
      const saved = JSON.parse(JSON.stringify(response({ visible_answer: [
        `⚠️ ${notice}`, '', 'Answer: Found a source.', '',
        '搜索收据', '查了：release', '@@search-receipt',
        JSON.stringify({ queries: ['release'], sources: [] }), '@@end-search-receipt',
      ].join('\n') })));
      const html = render(saved, locale);
      assert.ok(html.includes(notice), html);
      assert.doesNotMatch(html, /data-testid="search-receipt"/);
      assert.doesNotMatch(html, /@@search-receipt/);
    }
  });
  it('preserves indentation and literal boilerplate inside code blocks', () => {
    const code = '```python\nif ready:\n    print("No external action was taken.")\n    # Sources: literal comment\n```';
    const sections = secs(response({visible_answer: `Answer: Example\n${code}`, receipt_expectation: ''}));
    assert.equal(sections.length, 1);
    assert.ok(sections[0].content.includes(code));
    assert.match(render(response({visible_answer: code})), /    print\(&quot;No external action was taken\.&quot;\)/);
  });

  it('keeps a long code block intact across the more-details boundary', () => {
    const code = ['```python', 'if ready:', ...Array.from({length: 10}, (_, i) => `    print(${i})`), '```'].join('\n');
    const html = render(response({visible_answer: `Answer: Example\n${code}\nAfter the code.`, receipt_expectation: ''}));
    assert.equal((html.match(/<pre><code>/g) || []).length, 1);
    assert.match(html, /    print\(9\)\n?<\/code><\/pre>/);
    assert.ok(html.indexOf('</pre>') < html.indexOf('vnext-answer-more'));
    assert.match(html, /After the code\./);
  });

  it('turns labeled assistant text into answer and next-action cards', () => {
    const sections = secs(response({
      visible_answer: [
        '结论：现在可以处理。',
        '',
        '下一步：',
        '- 打开邮件',
        '- 准备回复',
      ].join('\n'),
      receipt_expectation: '',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer', 'Next actions']);
    assert.equal(sections[0].tone, 'answer');
    assert.equal(sections[1].tone, 'action');
  });

  it('adds warning, citation, and receipt policy sections from response fields', () => {
    const sections = secs(response({
      visible_answer: 'Summary: No action needed.',
      // Soft gate noise is suppressed; use a real runtime warning for Safety.
      warnings: ['tool_blocked:gmail_reader'],
      citations: ['Gmail event metadata'],
      receipt_expectation: 'Completion requires a Gmail update receipt.',
    }));

    assert.deepEqual(
      sections.map((section) => section.title),
      ['Answer', 'Safety', 'Sources', 'Receipt'],
    );
    assert.match(sections[1].content, /unavailable|not authorized|Gmail/i);
  });

  it('suppresses soft more_detail_requires_gate from the Safety wall', () => {
    const sections = secs(response({
      visible_answer: 'Summary: No action needed.',
      warnings: ['more_detail_requires_gate', 'source_content_untrusted'],
      citations: [],
      receipt_expectation: '',
    }));
    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
  });

  it('keeps receipt details available without making the first answer a wall of text', () => {
    const html = render(response({
      visible_answer: [
        'Answer: Reminder is ready.',
        'Next: approve it when you are ready.',
      ].join('\n'),
      citations: ['tool:manage_tasks'],
      receipt_expectation: 'Completion requires a task scheduler receipt.',
    }));

    assert.match(html, /class="vnext-answer-card tone-action"/);
    assert.doesNotMatch(html, /<details class="vnext-answer-card vnext-disclosure is-collapsible tone-evidence">/);
    assert.doesNotMatch(html, /<summary class="vnext-disclosure-summary"><span>Sources<\/span><\/summary>/);
    assert.match(html, /<details class="vnext-answer-card vnext-disclosure is-collapsible tone-done">/);
    assert.match(html, /<summary class="vnext-disclosure-summary"><span>Receipt<\/span><\/summary>/);
    assert.doesNotMatch(html, /<summary class="vnext-disclosure-summary"><span>Proof<\/span><\/summary>/);
    assert.doesNotMatch(html, /<details[^>]*tone-caution/);
  });

  it('does not turn the default no-action receipt into a noisy answer card', () => {
    const sections = secs(response({
      visible_answer: 'Answer: No action needed.',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
  });

  it('does not render generic no-action receipt blocks for assistant errors', () => {
    const html = render(response({
      response_kind: 'error',
      visible_answer: "I couldn't finish that. Try again.",
      receipt_expectation: 'No action was taken.',
    }));

    assert.match(html, /I couldn&#x27;t finish that/);
    assert.doesNotMatch(html, /<summary class="vnext-disclosure-summary"><span>Receipt<\/span><\/summary>/);
    assert.doesNotMatch(html, /No action was taken/);
  });

  it('suppresses the zh no-action receipt boilerplate exactly like the en value', () => {
    // App.tsx now produces receipt_expectation from the catalog per
    // display_language; the zh catalog value must stay as invisible as the en
    // literal it replaced.
    const zhReceipt = t('app.receipt.noActionTaken', 'zh');
    assert.equal(zhReceipt, '未执行任何操作。');
    const html = render(response({
      response_kind: 'error',
      visible_answer: "I couldn't finish that. Try again.",
      receipt_expectation: zhReceipt,
    }), 'zh');

    assert.doesNotMatch(html, /未执行任何操作/);
    const sections = secs(response({ response_kind: 'error', receipt_expectation: zhReceipt }));
    assert.equal(sections.some((section) => section.id === 'receipt'), false);
  });

  it('does not repeat a generic Answer heading inside the answer card', () => {
    const html = render(response({
      visible_answer: 'Answer: Ready.',
      receipt_expectation: '',
    }));

    assert.match(html, /aria-label="Answer"/);
    assert.doesNotMatch(html, /<span>Answer<\/span>/);
  });

  it('renders structured assistant severity as an owner-dismissible importance card', () => {
    const html = render(response({
      visible_answer: 'Provider balance is below your threshold.',
      severity: 'important',
      receipt_expectation: '',
    }));

    assert.match(html, /class="vnext-rich-callout important"/);
    assert.match(html, /<strong><span aria-hidden="true">‼️<\/span> Important<\/strong>/);
    assert.match(html, />Dismiss<\/button>/);
    assert.match(html, /Provider balance is below your threshold/);
  });

  it('keeps long answer bodies readable by folding extra detail in place', () => {
    const html = render(response({
      visible_answer: [
        'Answer: Here is the short operational summary.',
        '- First useful point.',
        '- Second useful point.',
        '- Third useful point.',
        '- Fourth useful point.',
        '- Fifth useful point.',
        '- Sixth useful point.',
        '- Seventh useful point.',
      ].join('\n'),
      receipt_expectation: '',
    }));

    assert.match(html, /Here is the short operational summary/);
    assert.match(html, /class="vnext-answer-more vnext-disclosure"/);
    assert.match(html, /<summary class="vnext-disclosure-summary">More detail<\/summary>/);
    assert.match(html, /Seventh useful point/);
  });

  it('hides low-value routing classifier lines from owner-visible chat answers', () => {
    const sections = secs(response({
      visible_answer: [
        'General request',
        'Your daily commute is Example City to Worktown.',
        'Next: ask for a live traffic check when you need it.',
      ].join('\n'),
      receipt_expectation: '',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer', 'Next actions']);
    assert.equal(
      sections[0].content,
      'Your daily commute is Example City to Worktown.',
    );
    assert.equal(sections[1].content, 'ask for a live traffic check when you need it.');
    assert.doesNotMatch(render(response({
      visible_answer: 'General request\nYour daily commute is Example City to Worktown.',
      receipt_expectation: '',
    })), /General request/);
  });

  it('uses one owner-facing voice across proactive, digest, and heartbeat answers', () => {
    for (const classifier of ['Bounded proactive review', 'Digest summary', 'Heartbeat escalation', 'System check']) {
      const sections = secs(response({
        visible_answer: [
          classifier,
          'Answer: Review the Gmail connection before expecting new mail summaries.',
          'Receipt policy: No action was taken.',
        ].join('\n'),
        receipt_expectation: 'No external action was executed.',
      }));

      assert.deepEqual(sections.map((section) => section.title), ['Answer']);
      assert.equal(sections[0].content, 'Review the Gmail connection before expecting new mail summaries.');
      assert.doesNotMatch(render(response({
        visible_answer: [
          classifier,
          'Answer: Review the Gmail connection before expecting new mail summaries.',
          'Receipt policy: No action was taken.',
        ].join('\n'),
        receipt_expectation: 'No external action was executed.',
      })), new RegExp(`${classifier}|Receipt policy|No action was taken`, 'i'));
    }
  });

  it('removes repeated no-action boilerplate from normal answer text', () => {
    const sections = secs(response({
      visible_answer: [
        'General request',
        'Your daily commute is Example City to Worktown.',
        'No external action was taken.',
      ].join('\n'),
      receipt_expectation: 'No external action was executed.',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
    assert.equal(sections[0].content, 'Your daily commute is Example City to Worktown.');
    assert.doesNotMatch(render(response({
      visible_answer: [
        'Answer: Your daily commute is Example City to Worktown.',
        'No action was taken.',
      ].join('\n'),
      receipt_expectation: 'No external action was executed.',
    })), /No action was taken|No external action was executed/);
  });

  it('hides generic runtime receipt policy from normal chat answers', () => {
    const sections = secs(response({
      visible_answer: 'Answer: 从Example City家到Worktown公司，预计44分钟。',
      response_kind: 'orchestrated_response',
      receipt_expectation: 'Handled through the main assistant runtime. External completion claims still require durable receipts.',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
    assert.doesNotMatch(render(response({
      visible_answer: 'Answer: 从Example City家到Worktown公司，预计44分钟。',
      response_kind: 'orchestrated_response',
      receipt_expectation: 'Handled through the main assistant runtime. External completion claims still require durable receipts.',
    })), /main assistant runtime|durable receipts|<summary class="vnext-disclosure-summary"><span>Receipt<\/span><\/summary>/);
  });

  it('hides generic runtime receipt policy from assistant errors too', () => {
    const html = render(response({
      visible_answer: "I couldn't finish that. Try again.",
      response_kind: 'error',
      receipt_expectation: 'Handled through the main assistant runtime. External completion claims still require durable receipts.',
    }));

    assert.match(html, /I couldn&#x27;t finish that/);
    assert.doesNotMatch(html, /main assistant runtime|durable receipts|<summary class="vnext-disclosure-summary"><span>Receipt<\/span><\/summary>/);
  });

  it('rewrites legacy runtime connection errors before rendering assistant text', () => {
    const html = render(response({
      response_kind: 'error',
      visible_answer: 'Aegis could not finish: could not reach Aegis. No action was taken. Try again.',
      receipt_expectation: 'No action was taken.',
    }));

    assert.match(html, /I did not get a complete answer\. Try again\./);
    assert.doesNotMatch(html, /could not reach Aegis|connection was interrupted|No action was taken/);
  });

  it('removes runtime source/privacy disclaimers without hiding the answer', () => {
    const sections = secs(response({
      visible_answer: 'Answer: Decision ready: review calendar access. One more item is queued. No source item or private content was opened.',
      receipt_expectation: '',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
    assert.equal(sections[0].content, 'Decision ready: review calendar access. One more item is queued.');
    assert.doesNotMatch(render(response({
      visible_answer: 'Answer: Decision ready: review calendar access. One more item is queued. No source item or private content was opened.',
      receipt_expectation: '',
    })), /No source item|private content was opened/);
  });

  it('treats explain labels as routing noise, not owner-facing headings', () => {
    const sections = secs(response({
      visible_answer: [
        'Explain',
        'Item command used vNext path for Billing statement available.',
        'No external action was taken.',
      ].join('\n'),
      receipt_expectation: '',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
    assert.equal(
      sections[0].content,
      'Item command used vNext path for Billing statement available.',
    );
    assert.doesNotMatch(render(response({
      visible_answer: 'Explain\nItem command used vNext path.',
      receipt_expectation: '',
    })), />Explain</);

    const labeled = secs(response({
      visible_answer: '解释：这封邮件需要你查看账单。',
      receipt_expectation: '',
    }));
    assert.deepEqual(labeled.map((section) => section.title), ['Answer']);
    assert.equal(labeled[0].content, '这封邮件需要你查看账单。');
  });

  it('hides low-value runtime caveats and internal field citations from normal answers', () => {
    const sections = secs(response({
      visible_answer: 'Answer: Queue brief prepared.',
      warnings: ['source_content_untrusted', 'redacted_fields_excluded'],
      citations: ['visible_fields.title', 'visible_fields.summary'],
      receipt_expectation: 'No external action was executed.',
    }));

    assert.deepEqual(sections.map((section) => section.title), ['Answer']);
  });

  it('renders structured cards without allowing unsafe markdown links', () => {
    const html = render(response({
      visible_answer: '下一步：Open [bad](javascript:alert(1)) and [safe](https://example.com/a).',
      receipt_expectation: '',
    }));

    assert.match(html, /class="vnext-answer-card tone-action"/);
    assert.doesNotMatch(html, /javascript:/);
    assert.match(html, /href="https:\/\/example.com\/a"/);
  });

  it('renders runtime warnings without turning bare tool citations into sources', () => {
    const sections = secs(response({
      visible_answer: 'Answer: Reminder saved.',
      citations: ['tool:manage_tasks', 'tool:commute_traffic_monitor'],
      warnings: ['tool_blocked:gmail_reader'],
      receipt_expectation: '',
    }));

    const evidence = sections.find((section) => section.title === 'Sources');
    const safety = sections.find((section) => section.title === 'Safety');
    assert.equal(evidence, undefined);
    assert.equal(safety.content, '- Gmail Reader was unavailable or not authorized.');
    assert.doesNotMatch(render(response({
      visible_answer: 'Answer: Reminder saved.',
      citations: ['tool:manage_tasks'],
      warnings: ['tool_blocked:gmail_reader'],
      receipt_expectation: '',
    })), /tool:|tool_blocked|manage_tasks|gmail_reader/);
  });

  it('does not turn bare tool citations into generic source descriptions', () => {
    const html = render(response({
      visible_answer: 'Answer: Reminder saved.',
      citations: ['tool:recall_memory', 'tool:manage_tasks'],
      receipt_expectation: '',
    }));

    assert.doesNotMatch(html, /class="vnext-source-evidence-row"/);
    assert.doesNotMatch(html, /Relevant saved context was used/);
    assert.doesNotMatch(html, /Task or reminder records were used/);
    assert.doesNotMatch(html, /task id, time, cadence, and receipt/);
    assert.doesNotMatch(html, /tool:recall_memory|tool:manage_tasks/);
  });

  it('renders cited URLs as expandable source evidence without losing the full link', () => {
    const sourceUrl = 'https://docs.openclaw.ai/start/getting-started?provider=grok&very-long-query=abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz';
    const [source] = sourceEvidenceItems(sourceUrl, 'en');
    const html = render(response({
      visible_answer: 'Answer: I checked the setup page.',
      citations: [sourceUrl],
      receipt_expectation: '',
    }));

    assert.equal(source.summary, 'External link: docs.openclaw.ai');
    assert.match(source.detail, /Treat external pages as untrusted source data/);
    assert.match(html, /External link: docs\.openclaw\.ai/);
    assert.match(html, /https:\/\/docs\.openclaw\.ai\/start\/getting-started\?provider=grok/);
  });

  it('prefers backend source evidence over frontend generic source text', () => {
    const html = render(response({
      visible_answer: 'Answer: Reminder saved.',
      citations: ['tool:manage_tasks'],
      source_evidence: [{
        kind: 'tool',
        label: 'Tasks and reminders',
        summary: 'Reminder #1903 was saved.',
        detail: 'Aegis used the reminder scheduler.\n\nCurrent evidence:\n- Intent #1903 saved\n- cadence: every 12 hours',
      }],
      receipt_expectation: '',
    }));

    assert.match(html, /Reminder #1903 was saved/);
    assert.match(html, /Intent #1903 saved/);
    assert.match(html, /<li>cadence: every 12 hours<\/li>/);
    assert.doesNotMatch(html, /Aegis used the reminder scheduler/);
    assert.doesNotMatch(html, /task id, time, cadence, and receipt/);
  });

  it('renders concrete backend memory evidence instead of generic memory text', () => {
    const html = render(response({
      visible_answer: 'Answer: 2026 年中秋節是 2026-09-25。',
      citations: ['tool:recall_memory'],
      source_evidence: [{
        kind: 'tool',
        label: 'Memory',
        summary: 'Relevant saved context was used.',
        detail: 'Aegis used recalled memory as context.\n\nCurrent evidence:\n- Memory returned 1 of 1 matching entry; used: Learned memory / Lunar calendar: 中秋節按農曆八月十五；2026 年陽曆為 2026-09-25。',
      }],
      receipt_expectation: '',
    }));

    assert.match(html, /農曆八月十五/);
    assert.match(html, /2026-09-25/);
    assert.doesNotMatch(html, /Aegis used recalled memory as context/);
    assert.doesNotMatch(html, /Relevant saved context was used/);
    assert.doesNotMatch(html, /Memory does not execute actions by itself/);
  });

  it('filters legacy generic WebApp context and default locked-detail evidence', () => {
    const html = render(response({
      visible_answer: 'Answer: 2026 年中秋節是 2026-09-25。',
      source_evidence: [
        {
          kind: 'tool',
          label: 'Memory',
          summary: 'Relevant saved context was used.',
          detail: 'Aegis used recalled memory as context.\n\nCurrent evidence:\n- 中秋節按農曆八月十五；2026 年陽曆為 2026-09-25。',
        },
        {
          kind: 'context',
          label: 'Visible Webapp context',
          summary: '4 safe fields visible.',
          detail: [
            'Aegis used these owner-visible fields:',
            '- Boundary: Owner prompt plus backend-selected relevant owner memory when available',
            '- Mode: general_assistant',
            '- Relevant owner memory: User has incoming USPS package delivery expected on Wednesday, May 6, 2026 by 9:00pm.',
            '- Safe actions: Answer questions or draft reviewable text.',
          ].join('\n'),
        },
        {
          kind: 'locked',
          label: 'Locked private detail',
          summary: '4 private fields withheld.',
          detail: [
            'These private fields were withheld:',
            '- Gmail body',
            '- Calendar private detail',
            '- Raw memory',
            '- Attachments',
          ].join('\n'),
        },
      ],
      receipt_expectation: '',
    }));

    assert.match(html, /農曆八月十五/);
    assert.doesNotMatch(html, /Visible Webapp context|USPS package|Locked private detail|Gmail body|Raw memory/);
  });

  it('renders runtime warnings and citations without internal machine labels', () => {
    const html = render(response({
      visible_answer: 'Answer: I need one more check.',
      citations: ['data_access_gate', 'llm_provider'],
      warnings: ['mfa_required', 'consent_required'],
      receipt_expectation: '',
    }));

    assert.match(html, /Access check/);
    assert.match(html, /AI service/);
    assert.match(html, /Extra confirmation required/);
    assert.match(html, /Google permission required/);
    assert.doesNotMatch(html, /data_access_gate|llm_provider|mfa_required|consent_required|Llm|Mfa/i);
  });
});

describe('vNext assistant response view — i18n (zh chrome)', () => {
  it('localizes section title display while keeping English title identity', () => {
    // Section titles are still the internal English identity for logic/dedupe.
    const sectionList = secs(response({
      visible_answer: 'Summary: No action needed.',
      warnings: ['tool_blocked:gmail_reader'],
      citations: ['Gmail event metadata'],
      receipt_expectation: 'Completion requires a Gmail update receipt.',
    }), 'zh');
    assert.deepEqual(
      sectionList.map((section) => section.title),
      ['Answer', 'Safety', 'Sources', 'Receipt'],
    );

    // But the rendered chrome is Chinese.
    const html = render(response({
      visible_answer: 'Summary: No action needed.',
      warnings: ['tool_blocked:gmail_reader'],
      citations: ['Gmail event metadata'],
      receipt_expectation: 'Completion requires a Gmail update receipt.',
    }), 'zh');
    assert.match(html, new RegExp(`<span>${t('assistantResponse.title.safety', 'zh')}</span>`));
    assert.match(html, new RegExp(`<summary class="vnext-disclosure-summary"><span>${t('assistantResponse.title.sources', 'zh')}</span></summary>`));
    assert.match(html, new RegExp(`<summary class="vnext-disclosure-summary"><span>${t('assistantResponse.title.receipt', 'zh')}</span></summary>`));
    assert.match(html, new RegExp(`aria-label="${t('assistantResponse.title.answer', 'zh')}"`));
    // English chrome must NOT leak.
    assert.doesNotMatch(html, /<span>Safety<\/span>|<span>Sources<\/span>|<span>Receipt<\/span>/);
  });

  it('localizes the More detail disclosure summary', () => {
    const html = render(response({
      visible_answer: [
        'Answer: Here is the short operational summary.',
        '- First useful point.',
        '- Second useful point.',
        '- Third useful point.',
        '- Fourth useful point.',
        '- Fifth useful point.',
        '- Sixth useful point.',
        '- Seventh useful point.',
      ].join('\n'),
      receipt_expectation: '',
    }), 'zh');

    assert.match(html, new RegExp(`<summary class="vnext-disclosure-summary">${t('assistantResponse.moreDetail', 'zh')}</summary>`));
    assert.doesNotMatch(html, /<summary class="vnext-disclosure-summary">More detail<\/summary>/);
  });

  it('localizes source-evidence summary text for cited URLs', () => {
    const sourceUrl = 'https://docs.openclaw.ai/start';
    const [source] = sourceEvidenceItems(sourceUrl, 'zh');
    assert.equal(source.summary, tf('assistantResponse.source.externalLink', 'zh', { host: 'docs.openclaw.ai' }));
    assert.match(source.detail, /不受信任的来源数据/);

    const html = render(response({
      visible_answer: 'Answer: I checked the setup page.',
      citations: [sourceUrl],
      receipt_expectation: '',
    }), 'zh');
    assert.match(html, /外部链接：docs\.openclaw\.ai/);
    assert.doesNotMatch(html, /External link:/);
  });

  it('localizes runtime warnings (blocked tool) and suppresses soft gate noise', () => {
    const html = render(response({
      visible_answer: 'Answer: Reminder saved.',
      warnings: ['tool_blocked:gmail_reader', 'more_detail_requires_gate'],
      receipt_expectation: '',
    }), 'zh');

    assert.match(html, new RegExp(tf('assistantResponse.warning.toolUnavailable', 'zh', { tool: 'Gmail Reader' })));
    // Soft more_detail_requires_gate must not reappear as "更多详情需要你的批准".
    assert.doesNotMatch(html, new RegExp(t('assistantResponse.warning.moreDetailNeedsApproval', 'zh')));
    assert.doesNotMatch(html, /was unavailable or not authorized|More detail needs your approval/);
  });

  it('localizes humanized machine labels for citations and warnings', () => {
    const html = render(response({
      visible_answer: 'Answer: I need one more check.',
      citations: ['data_access_gate', 'llm_provider'],
      warnings: ['mfa_required', 'consent_required'],
      receipt_expectation: '',
    }), 'zh');

    assert.match(html, new RegExp(t('assistantResponse.machineLabel.dataAccessGate', 'zh')));
    assert.match(html, new RegExp(t('assistantResponse.machineLabel.llmProvider', 'zh')));
    assert.match(html, new RegExp(t('assistantResponse.machineLabel.mfaRequired', 'zh')));
    assert.match(html, new RegExp(t('assistantResponse.machineLabel.consentRequired', 'zh')));
    assert.doesNotMatch(html, /Access check|AI service|Extra confirmation required|Google permission required/);
  });

  it('localizes the legacy runtime connection-error rewrite', () => {
    const html = render(response({
      response_kind: 'error',
      visible_answer: 'Aegis could not finish: could not reach Aegis. No action was taken. Try again.',
      receipt_expectation: 'No action was taken.',
    }), 'zh');

    assert.match(html, new RegExp(t('assistantResponse.incompleteAnswer', 'zh')));
    assert.doesNotMatch(html, /I did not get a complete answer|could not reach Aegis/);
  });
});

const commuteEvidence = {
  id: 'E1',
  tool: 'commute_traffic_monitor',
  label: '通勤检查',
  excerpt: '当前交通约 44 分钟。',
  full_len: 1234,
};

describe('vNext assistant response view — evidence refs and estimates', () => {
  it('renders matching [E1] as a chip and a missing id as muted text', () => {
    const html = render(response({
      visible_answer: '约 44 分钟 [E1]，另有 [E9]',
      evidence: [commuteEvidence],
      receipt_expectation: '',
    }));

    assert.match(html, /<button type="button" class="vnext-evidence-ref" data-evidence-id="E1"/);
    assert.match(html, />E1<\/button>/);
    assert.doesNotMatch(html, /\[E1\]/);
    assert.match(html, /<span class="vnext-evidence-missing">E9<\/span>/);
    assert.doesNotMatch(html, /data-evidence-id="E9"/);
    assert.doesNotMatch(html, /\[E9\]/);
    assert.match(html, /<strong>E1 · /);
    assert.doesNotMatch(html, /来源摘要|Source excerpt/);
  });

  it('renders [E1, E2] and [E1][E2] as separate chips', () => {
    const html = render(response({
      visible_answer: 'A [E1, E2] and [E1][E2]',
      evidence: [
        commuteEvidence,
        { id: 'E2', tool: 'web_search', label: '网页', excerpt: 'route notes', full_len: 40 },
      ],
      receipt_expectation: '',
    }));

    const chips = html.match(/class="vnext-evidence-ref" data-evidence-id="E[12]"/g) ?? [];
    assert.equal(chips.length, 4);
    assert.doesNotMatch(html, /\[E1/);
  });

  it('links each chip to its readable source without a nested disclosure', () => {
    const html = render(response({visible_answer: '约 44 分钟 [E1]', evidence: [commuteEvidence], receipt_expectation: ''}));
    const target = html.match(/aria-controls="([^"]+)"/)[1];
    assert.ok(html.includes(`id="${target}"`));
    assert.match(html, /aria-expanded="false"/);
    assert.equal((html.match(/<details/g) || []).length, 1);
    assert.match(html, /data-evidence-id="E1"[^>]*class="vnext-source-evidence-row"/);
    assert.match(html, /当前交通约 44 分钟/);
  });

  it('wraps the first unsupported number and shows a count line', () => {
    const html = render(response({
      visible_answer: '约 44 分钟 [E1]，出发约 60–80 分钟',
      evidence: [commuteEvidence],
      unsupported_numbers: ['60–80'],
      receipt_expectation: '',
    }));

    assert.match(html, /<span class="vnext-estimate" title="Estimate — no tool data behind it" aria-label="Estimate — no tool data behind it">60–80<\/span>/);
    assert.equal((html.match(/class="vnext-estimate"/g) ?? []).length, 1);
    assert.match(html, /1 estimated number\(s\)/);
    assert.match(html, /<button type="button" class="vnext-btn vnext-btn-ghost vnext-btn-sm vnext-estimate-toggle" aria-pressed="false">Highlight estimates<\/button>/);
  });

  it('hides the count line when there are no unsupported numbers', () => {
    const html = render(response({
      visible_answer: '约 44 分钟 [E1]',
      evidence: [commuteEvidence],
      receipt_expectation: '',
    }));

    assert.doesNotMatch(html, /vnext-estimate-note/);
    assert.doesNotMatch(html, /estimated number/);
    assert.doesNotMatch(html, /推算数字/);
  });

  it('does not render the unsupported_numbers warning string', () => {
    const html = render(response({
      visible_answer: 'Answer: Ready.',
      warnings: ['unsupported_numbers:2'],
      receipt_expectation: '',
    }));

    assert.doesNotMatch(html, /unsupported_numbers/);
    assert.doesNotMatch(html, /<span>Safety<\/span>/);
    assert.deepEqual(secs(response({
      visible_answer: 'Answer: Ready.',
      warnings: ['unsupported_numbers:2'],
      receipt_expectation: '',
    })).map((section) => section.title), ['Answer']);
  });

  it('does not wrap a markdown link whose text contains an unsupported number', () => {
    const html = render(response({
      visible_answer: 'See [60–80](https://example.com/a) for the range.',
      unsupported_numbers: ['60–80'],
      receipt_expectation: '',
    }));

    assert.match(html, /href="https:\/\/example.com\/a"/);
    assert.doesNotMatch(html, /class="vnext-estimate"/);
    assert.match(html, /1 estimated number\(s\)/);
  });

  it('localizes estimate chrome and keeps claim evidence inspectable', () => {
    const html = render(response({
      visible_answer: '约 44 分钟 [E1]，出发约 60–80 分钟',
      evidence: [commuteEvidence],
      unsupported_numbers: ['60–80'],
      receipt_expectation: '',
    }), 'zh');

    assert.match(html, new RegExp(`aria-label="${t('assistantResponse.evidence.ref', 'zh').replace('{id}', 'E1')}"`));
    assert.match(html, new RegExp(t('assistantResponse.estimate.title', 'zh')));
    assert.match(html, new RegExp(tf('assistantResponse.estimate.count', 'zh', { count: '1' })));
    assert.match(html, new RegExp(t('assistantResponse.estimate.toggle', 'zh')));
    assert.doesNotMatch(html, /来源摘要/);
    assert.doesNotMatch(html, /回执|事项|主人|幕僚长/);
  });

  it('shows the original-length note only when the excerpt was actually cut', () => {
    const clipped = '甲'.repeat(600);
    const cut = render(response({ evidence: [{ ...commuteEvidence, excerpt: clipped, full_len: 800 }] }), 'zh');
    assert.match(cut, /来源摘要 · 原文 800 字符/);
    const omitted = 'Status: error\nShowing 2 of 9 source entries.';
    const dropped = render(response({ evidence: [{ ...commuteEvidence, excerpt: omitted, full_len: omitted.length + 40 }] }), 'zh');
    assert.match(dropped, /来源摘要 · 原文/);
    const complete = JSON.stringify({ count: 1, tasks: [{ title: '联系家人', status: 'fired' }] });
    const whole = render(response({ evidence: [{ ...commuteEvidence, excerpt: complete, full_len: 5000 }] }), 'zh');
    assert.match(whole, /联系家人/);
    assert.doesNotMatch(whole, /来源摘要/);
  });

  it('points a chip at the single field named in the preceding sentence', () => {
    const excerpt = JSON.stringify({ tasks: [{ title: '联系家人', status: 'fired', count: 1 }] });
    const evidence = [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders', excerpt, full_len: excerpt.length }];
    const linked = render(response({ visible_answer: '因为联系家人。[E1]', evidence }), 'zh');
    assert.match(linked, /data-evidence-fact="联系家人"/);
    assert.match(linked, /data-source-fact="联系家人"/);
    const ambiguous = render(response({ visible_answer: '联系家人已触发 [E1]', evidence }), 'zh');
    assert.doesNotMatch(ambiguous, /data-evidence-fact=/);
  });
});


describe('readable source evidence', () => {
  it('renders task JSON as source facts, without routing metadata', () => {
    const excerpt = JSON.stringify({ count: 2, status: 'ok', status_filter: 'all', tasks: [
      { title: '联系家人', status: 'fired', id: 'task-42', group_key: 'internal-routing', trigger_at_unix: 1788750000 },
      { title: '准备材料', due_date: '2026-12-01', body: '携带原件' },
    ] });
    const html = renderInZone(response({ evidence: [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders', excerpt, full_len: 400 }] }), 'UTC', 'zh');
    for (const fact of ['联系家人', '已触发', '准备材料', '2026-12-01', '携带原件', '条目数', '2026-09-07 03:00 UTC', '编号', 'task-42', 'internal-routing']) assert.ok(html.includes(fact), fact);
    assert.doesNotMatch(html, /status_filter|group_key|execution_mode|&quot;tasks&quot;/);
  });
  it('shows a scheduled instant in the owner timezone and leaves a date-only due date alone', () => {
    const excerpt = JSON.stringify({ title: '联系家人', trigger_at_unix: 1788750000, due_date: '2026-12-01' });
    const html = renderInZone(response({ evidence: [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders', excerpt, full_len: excerpt.length }] }), 'Asia/Shanghai', 'zh');
    assert.match(html, /2026-09-07 11:00 UTC\+08:00/);
    assert.match(html, /2026-12-01/);
    assert.doesNotMatch(html, /2026-09-07 03:00/);
  });
  it('keeps concrete values and links from unfamiliar JSON sources', () => {
    const html = render(response({ evidence: [{ id: 'E1', tool: 'weather', label: 'Weather',
      excerpt: JSON.stringify({ temperature: 0, available: false, budgets: [10, 20], url: 'https://example.com/source' }), full_len: 80 }] }));
    assert.match(html, /Temperature: 0/);
    assert.match(html, /Available: false/);
    assert.match(html, /Budgets:/);
    assert.match(html, /https:\/\/example\.com\/source/);
    assert.doesNotMatch(html, /&quot;temperature&quot;/);
  });
  it('does not pretend a clipped historical JSON fragment is readable evidence', () => {
    const html = render(response({ evidence: [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders',
      excerpt: '{"tasks":[{"title":"partial', full_len: 800 }] }), 'zh');
    assert.match(html, /历史来源的内容不完整/);
    assert.doesNotMatch(html, /partial|&quot;tasks&quot;/);
  });
  it('localizes new server summaries without treating fired as completed', () => {
    const html = render(response({ evidence: [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders',
      excerpt: 'Count: 1\n\nTasks: - 联系家人\n\nStatus: fired', full_len: 100 }] }), 'zh');
    assert.match(html, /状态: 已触发/);
    assert.doesNotMatch(html, /已完成|Status: fired/);
  });
});

 it('renders the historical schedule validation error as actionable Chinese', () => {
    const html = render(response({ evidence: [{ id: 'E2', tool: 'manage_tasks', label: 'Tasks and reminders',
      excerpt: JSON.stringify({ error: 'Invalid schedule parameters / 定时参数无效', error_kind: 'validation_error',
        hint: 'Check trigger_type, trigger_value, and interval format.', retryable: false, status: 'error' }), full_len: 250 }] }), 'zh');
    for (const fact of ['任务未创建', '参数校验失败', '请确认开始时间', '需先修正设置']) assert.ok(html.includes(fact), fact);
    assert.doesNotMatch(html, /trigger_value|validation_error|&quot;error&quot;/);
 });

it('localizes server-formatted expired-start errors without hiding the remedy', () => {
  const html = render(response({ evidence: [{ id: 'E2', tool: 'manage_tasks', label: 'Tasks and reminders',
    excerpt: 'Status: error\n\nError: Schedule start time has passed / 开始时间已过\n\nError kind: validation_error\n\nHint: Choose a new future trigger_at or use delay_seconds >= 10 for a relative start. Do not retry the same expired timestamp.\n\nRetryable: false', full_len: 300 }] }), 'zh');
  for (const fact of ['开始时间已过', '任务未创建', '至少 10 秒后', '不要沿用已过期的时间', '需先修正设置']) assert.ok(html.includes(fact), fact);
    assert.doesNotMatch(html, /trigger_at|delay_seconds|validation_error/);
});

it('translates a schedule hint that carries an extra clause', () => {
  const html = render(response({ evidence: [{ id: 'E2', tool: 'manage_tasks', label: 'Tasks and reminders',
    excerpt: JSON.stringify({
      error: 'Invalid schedule parameters / 定时参数无效 (see log)',
      hint: 'Check trigger_type, trigger_value, and interval format. Wait for delay_seconds.',
      status: 'fired extra',
    }), full_len: 320 }] }), 'zh');
  assert.match(html, /定时参数无效，任务未创建/);
  assert.match(html, /请确认开始时间/);
  assert.match(html, /延迟秒数/);
  assert.match(html, /已触发/);
  assert.doesNotMatch(html, /trigger_type|trigger_value|delay_seconds|fired/);
});


it('renders a truncated tool summary only once when E1 already contains its evidence', () => {
  const excerpt = 'Example City sunny, 11–24°C. Forecast: https://example.com/weather';
  const html = render(response({ visible_answer: 'Sunny. [E1]',
    evidence: [{id: 'E1', tool: 'web_search', label: 'Web search', excerpt, full_len: excerpt.length}],
    source_evidence: [
      {kind: 'tool', label: 'Web search', summary: 'Example City sunny…', detail: 'Current evidence:\n- Example City sunny…'},
      {kind: 'tool', label: 'Web search', summary: 'Additional source', detail: 'Additional source: https://example.com/other'},
      {kind: 'context', label: 'Location', summary: 'Saved location', detail: 'Example City'},
    ],
  }));
  assert.equal((html.match(/class="vnext-source-evidence-row"/g) || []).length, 3);
  assert.equal((html.match(/Example City sunny/g) || []).length, 1);
  assert.match(html, /example.com\/weather/);
  assert.match(html, /example.com\/other/);
  assert.match(html, /Saved location/);
});

describe('source panel integrity across replies and representations', () => {
  it('recognizes persisted translated tool labels without discarding distinct citations', () => {
    const html = render(response({visible_answer: 'Forecast [E1]',
      citations: ['网络搜索', 'https://example.com/other'],
      evidence: [{id:'E1',tool:'web_search',label:'Web search',excerpt:'Forecast facts',full_len:14}],
    }), 'zh');
    assert.equal((html.match(/class="vnext-source-evidence-row"/g) || []).length, 2);
    assert.match(html, /Forecast facts/);
    assert.match(html, /https:\/\/example.com\/other/);
    assert.doesNotMatch(html, /安琪将此来源作为依据/);
  });
  it('keeps legacy empty evidence navigable with an explicit no-details message', () => {
    const html = render(response({visible_answer: 'No result [E1]', evidence: [{...commuteEvidence, excerpt: '', full_len: 2}]}), 'zh');
    assert.match(html, /data-evidence-id="E1" aria-controls=/);
    assert.match(html, /来源未返回详细内容/);
    assert.doesNotMatch(html, /vnext-evidence-missing/);
  });
  it('formats legacy null and scalar arrays without confusing evidence references for JSON', () => {
    const withExcerpt = excerpt => render(response({evidence: [{...commuteEvidence, excerpt}]}), 'zh');
    assert.match(withExcerpt('null'), /来源未返回详细内容/);
    const array = withExcerpt('[-1, true, null]');
    assert.match(array, /-1/);
    assert.match(array, /true/);
    assert.doesNotMatch(array, /\[-1/);
    assert.match(withExcerpt('[E1] remains a literal reference'), /\[E1\] remains a literal reference/);
  });
  it('does not interpret a source heading inside quoted code as a new panel', () => {
    const html = render(response({visible_answer:'Example:\n```text\nSources: preserved sample\n# Heading inside code\n```'}));
    assert.match(html,/Sources: preserved sample/);
    assert.match(html,/# Heading inside code/);
    assert.doesNotMatch(html,/tone-evidence/);
    assert.match(html,/<pre><code>/);
  });

  it('retains a citation URL that is only a prefix of another source URL', () => {
    const sections = secs(response({evidence:[{...commuteEvidence,excerpt:'https://example.com/report-long'}],citations:['https://example.com/report']}));
    assert.ok(sections.find(s=>s.title==='Sources').sourceEvidence.some(s=>s.label==='https://example.com/report'));
  });
  it('uses a unique target per reply even when both cite E1', () => {
    const html = renderToStaticMarkup(React.createElement(LocaleProvider, {displayLanguage: 'en'},
      React.createElement(React.Fragment, null, ...['First', 'Second'].map(text =>
        React.createElement(AssistantResponseView, {key: text, result: response({visible_answer: `${text} [E1]`, evidence: [commuteEvidence]})})))));
    const targets = [...html.matchAll(/aria-controls="([^"]+)"/g)].map(match => match[1]);
    assert.equal(targets.length, 2);
    assert.equal(new Set(targets).size, 2);
    for (const target of targets) assert.equal(html.split(`id="${target}"`).length - 1, 1);
  });

  it('merges textual Sources and structured evidence without discarding either', () => {
    const sections = secs(response({visible_answer: 'Answer: Ready [E1]\nSources: - Web search',
      evidence: [{...commuteEvidence, label: 'Web search'}],
      source_evidence: [{kind: 'tool', label: 'Web search', summary: 'Unique fact', detail: 'A unique fact from the source'}],
      citations: ['https://example.com/independent']}));
    const sources = sections.filter(section => section.title === 'Sources');
    assert.equal(sources.length, 1);
    assert.equal(sources[0].claimEvidence.length, 1);
    assert.ok(sources[0].sourceEvidence.some(item => item.detail.includes('A unique fact')));
    assert.ok(sources[0].sourceEvidence.some(item => item.label === 'https://example.com/independent'));
  });

  it('preserves explanatory source prose instead of inventing generic source cards', () => {
    const html = render(response({visible_answer: 'Answer: Ready.\nSources: This count excludes Spam and Trash.'}));
    assert.match(html, /This count excludes Spam and Trash\./);
    assert.doesNotMatch(html, /Source referenced by Aegis|vnext-source-evidence-row/);
  });

  it('makes citations in next actions usable too', () => {
    const html = render(response({visible_answer: 'Answer: Ready.\nNext: Review the finding [E1]', evidence: [commuteEvidence]}));
    assert.match(html, /data-evidence-id="E1" aria-controls=/);
    assert.doesNotMatch(html, /\[E1\]/);
  });

  it('keeps status and source URL beside a JSON summary', () => {
    const html = render(response({evidence: [{...commuteEvidence, excerpt: JSON.stringify({summary: 'Request could not finish.', status: 'error', url: 'https://example.com/report', hint: 'Try after reconnecting.'})}]}));
    for (const fact of ['Request could not finish.', 'Failed', 'https://example.com/report', 'Try after reconnecting.']) assert.ok(html.includes(fact), fact);
  });

  it('unwraps nested text JSON without hiding sibling evidence', () => {
    const html = render(response({evidence: [{...commuteEvidence, excerpt: JSON.stringify({content: [{type: 'text', text: JSON.stringify({summary: 'Nested fact', url: 'https://example.com/nested'})}], count: 1})}]}));
    assert.match(html, /Nested fact/);
    assert.match(html, /https:\/\/example.com\/nested/);
    assert.doesNotMatch(html, /&quot;summary&quot;|&quot;text&quot;/);
    assert.match(html, /Count: 1/);
  });

  it('preserves prototype-shaped source values and labels in both locales', () => {
    for (const locale of ['en', 'zh']) {
      for (const status of ['constructor', 'toString', '__proto__']) {
        const excerpt = JSON.stringify({status, error: status, hint: status, count: 0, available: false, summary: 'Retained sibling'});
        const html = render(response({evidence: [{...commuteEvidence, excerpt}]}), locale);
        for (const label of locale === 'zh' ? ['状态', '错误', '处理建议'] : ['Status', 'Error', 'Hint']) {
          assert.ok(html.includes(`${label}: ${status}`), `${locale} ${label}: ${status}`);
        }
        for (const fact of ['Retained sibling', 'Available: false']) assert.ok(html.includes(fact));
        assert.doesNotMatch(html, /undefined|function Object|native code/);
      }
      const html = render(response({evidence: [{...commuteEvidence, excerpt: '{"__proto__":"source fact","count":0}'}]}), locale);
      assert.match(html, /proto\s*: source fact/);
      assert.doesNotMatch(html, /\[object Object\]/);
      const plain = render(response({evidence: [{...commuteEvidence, excerpt: '__proto__: literal field\nconstructor: retained value'}]}), locale);
      assert.ok(plain.includes('__proto__: literal field'));
      assert.ok(plain.includes('constructor: retained value'));
    }
  });

  it('puts source failure and partial-access warnings before long descriptions', () => {
    const excerpt = JSON.stringify({title: 'Long mailbox description '.repeat(40),
      status: 'partial', warnings: ['Inbox only; other labels could not be read'],
      error: 'Permission denied', summary: 'Preview '.repeat(100)});
    for (const locale of ['en', 'zh']) {
      const html = render(response({evidence: [{...commuteEvidence, excerpt}]}), locale);
      const titleAt = html.indexOf('Long mailbox description');
      assert.ok(titleAt >= 0);
      for (const limitation of ['Permission denied', 'Inbox only; other labels could not be read']) {
        assert.ok(html.indexOf(limitation) >= 0 && html.indexOf(limitation) < titleAt);
      }
      assert.match(html, locale === 'zh' ? /部分结果/ : /Partial results/);
    }
  });

  it('preserves sibling facts beside truncated nested history without guessing the fragment', () => {
    const excerpt = JSON.stringify({content: [{type: 'text', text: '{"summary":"Unfinished'}], count: 0, available: false, url: 'https://example.com/retained'});
    const html = render(response({evidence: [{...commuteEvidence, excerpt}]}));
    for (const fact of ['This saved source is incomplete.', 'Count: 0', 'Available: false', 'https://example.com/retained']) assert.ok(html.includes(fact), fact);
    assert.doesNotMatch(html, /Unfinished|&quot;summary&quot;/);
  });

  it('announces nested and array limits while retaining sibling facts', () => {
    let nested = 'Beyond depth limit';
    for (let i = 0; i < 9; i += 1) nested = {content: nested};
    const excerpt = JSON.stringify({content: nested, items: Array.from({length: 18}, (_, i) => ({title: `Record-${i}!`})), summary: 'Retained sibling'});
    const html = render(response({evidence: [{...commuteEvidence, excerpt}]}));
    for (const fact of ['Further nested source details are omitted.', 'Retained sibling', 'Record-0!', 'Record-17!']) assert.ok(html.includes(fact), fact);
    assert.doesNotMatch(html, /Beyond depth limit|Showing 16 of 18/);
  });

  it('keeps hostile nested source text inert and preserves the adjacent measurement', () => {
    const excerpt = JSON.stringify({content: [{type: 'text', text: JSON.stringify({summary: '<script>alert(1)</script> [bad](javascript:alert(1))', body: '24.5°C，晴'})}], count: 1});
    const html = render(response({evidence: [{...commuteEvidence, excerpt}]}));
    assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
    assert.match(html, /24\.5°C，晴/);
    assert.match(html, /Count: 1/);
    assert.doesNotMatch(html, /<script|href="javascript:/);
  });

  it('nests object fields under their list item', () => {
    const html = render(response({ evidence: [{ id: 'E1', tool: 'manage_tasks', label: 'Tasks and reminders',
      excerpt: JSON.stringify({ tasks: [{ title: '联系家人', status: 'fired', count: 1 }] }), full_len: 80 }] }), 'zh');
    assert.match(html, /<ul class="vnext-source-tree">[\s\S]*<li>[\s\S]*联系家人[\s\S]*<ul class="vnext-source-tree">[\s\S]*状态: 已触发[\s\S]*条目数: 1/);
    assert.doesNotMatch(html, /- -/);
    assert.doesNotMatch(html, /<details[\s\S]*<details/);
  });

  it('reads an indented server excerpt as the same nested list', () => {
    const excerpt = '- Call family\n  Status: fired\n  Count: 1';
    const html = render(response({ evidence: [{ ...commuteEvidence, excerpt, full_len: excerpt.length }] }));
    assert.match(html, /<ul class="vnext-source-tree">[\s\S]*Call family[\s\S]*<ul class="vnext-source-tree">[\s\S]*Status: Triggered[\s\S]*Count: 1/);
    assert.doesNotMatch(html, /class="vnext-evidence-full-len"/);
  });

  it('keeps newlines inside a source string instead of reading them as fields', () => {
    const html = render(response({ evidence: [{ ...commuteEvidence, excerpt: JSON.stringify({ note: 'line one\nStatus: fired', count: 1 }) }] }), 'zh');
    assert.match(html, /line one\nStatus: fired/);
    assert.match(html, /条目数: 1/);
    assert.doesNotMatch(html, /状态: 已触发/);
    const fenced = 'Note:\n  ```\n  line one\n  Status: fired\n  ```\nCount: 1';
    const server = render(response({ evidence: [{ ...commuteEvidence, excerpt: fenced, full_len: fenced.length }] }), 'zh');
    assert.match(server, /line one\nStatus: fired/);
    assert.match(server, /条目数: 1/);
    assert.doesNotMatch(server, /状态: 已触发/);
  });

  it('keeps a nested brace note that is not truncated JSON', () => {
    const html = render(response({ evidence: [{ ...commuteEvidence, excerpt: JSON.stringify({ summary: '{not a record', count: 1 }) }] }));
    assert.match(html, /\{not a record/);
    assert.match(html, /Count: 1/);
    assert.doesNotMatch(html, /incomplete|不完整/);
  });

  it('leaves an ordinary colon sentence in prose untouched', () => {
    const html = render(response({ evidence: [{ ...commuteEvidence, excerpt: 'Note: keep this sentence.\nThe count is not a field.' }] }), 'zh');
    assert.match(html, /Note: keep this sentence/);
    assert.doesNotMatch(html, /vnext-source-tree/);
  });

  it('explains evidence numbers that kept a slot but returned nothing', () => {
    const html = render(response({
      visible_answer: '结论 [E3] [E9]',
      evidence: [
        { id: 'E3', tool: 'web_search', label: 'Web search', excerpt: '{"summary":"A"}', full_len: 20 },
        { id: 'E4', tool: 'web_search', label: 'Web search', excerpt: '{"summary":"B"}', full_len: 20 },
        { id: 'E7', tool: 'web_search', label: 'Web search', excerpt: '{"summary":"C"}', full_len: 20 },
        { id: 'E9', tool: 'web_search', label: 'Web search', excerpt: '{"summary":"D"}', full_len: 20 },
      ],
    }), 'zh');
    assert.match(html, /E1、E2、E5、E6、E8 没有带来可查看的来源/);
    assert.doesNotMatch(html, /data-evidence-id="E1"/);
    assert.equal((html.match(/class="vnext-source-evidence-row"/g) || []).length, 4);
  });

  it('does not repeat a summary already present in detail', () => {
    const html = render(response({source_evidence: [{label:'Local record', summary:'A concrete finding', detail:'A concrete finding\nAdditional context'}]}));
    assert.equal(html.split('A concrete finding').length - 1, 1);
    assert.match(html, /Additional context/);
  });

  it('does not repeat a generic summary shortened with an ellipsis', () => {
    const finding = `${'具体发现'.repeat(40)}。后续还有一条。`;
    const html = render(response({ source_evidence: [{ label: 'Local record', summary: 'Relevant saved context was used.', detail: finding }] }), 'zh');
    assert.equal(html.split('具体发现').length - 1, finding.split('具体发现').length - 1);
    assert.match(html, /后续还有一条/);
  });

  it('renders Jev provenance fields when the broker ran', () => {
    const html = render(response({
      visible_answer: 'Answer: Uploaded.',
      source_evidence: [{
        kind: 'tool',
        label: 'Google Drive upload',
        summary: 'File uploaded to Google Drive successfully.',
        detail: 'Current evidence:\n- Tool returned ok: File uploaded to Google Drive successfully.',
        jev: {
          verdict: 'allow',
          audit_tip: 'tip-abc',
          risk: 'high_risk=0.10',
          approval_id: 'jev-ask-skip',
          receipt: 'int-1',
        },
      }],
      receipt_expectation: '',
    }));
    assert.match(html, /vnext-jev-provenance/);
    assert.match(html, /data-jev-verdict="allow"/);
    assert.match(html, /tip-abc/);
    assert.match(html, /high_risk=0\.10/);
    assert.match(html, /jev-ask-skip/);
    assert.match(html, /int-1/);
  });

  it('renders one Open in X button when the same post is cited three ways', () => {
    const html = render(response({
      visible_answer: [
        'Answer: 帖里所谓接口不是正式 API。',
        'https://x.com/alice/status/111?s=20',
        'https://twitter.com/alice/status/111',
        'https://x.com/i/web/status/111',
        'https://x.com/bob/status/222',
      ].join('\n'),
      source_evidence: [{
        kind: 'tool',
        label: 'https://x.com/alice/status/111',
        summary: '帖子',
        detail: 'https://x.com/alice/status/111?s=20',
      }],
      receipt_expectation: '',
    }), 'zh');
    assert.equal((html.match(/class="vnext-x-open-btn"/g) || []).length, 1);
    assert.equal((html.match(/class="vnext-x-open-btn is-secondary"/g) || []).length, 1);
    assert.equal((html.match(/class="vnext-x-open-extra"/g) || []).length, 1);
    assert.match(html, />在 X 中打开</);
    assert.match(html, />在 X 中起草回复</);
    assert.match(html, />@bob</);
    assert.doesNotMatch(html, /vnext-x-source-actions/);
    assert.doesNotMatch(html, /vnext-x-open-strip-row/);
  });

  it('folds a work-item post into the same Open in X button', () => {
    const html = render(response({
      visible_answer: 'Answer: 另一条帖子。\nhttps://x.com/bob/status/222',
      receipt_expectation: '',
    }), 'zh');
    const withWorkPost = renderToStaticMarkup(
      React.createElement(
        LocaleProvider,
        { displayLanguage: 'zh' },
        React.createElement(AssistantResponseView, {
          result: response({
            visible_answer: 'Answer: 另一条帖子。\nhttps://x.com/bob/status/222',
            receipt_expectation: '',
          }),
          extraXLinks: ['https://x.com/alice/status/111?s=20', 'https://twitter.com/alice/status/111'],
        }),
      ),
    );
    assert.equal((html.match(/class="vnext-x-open-btn"/g) || []).length, 1);
    assert.equal((withWorkPost.match(/class="vnext-x-open-btn"/g) || []).length, 1);
    assert.equal((withWorkPost.match(/class="vnext-x-open-btn is-secondary"/g) || []).length, 1);
    assert.match(withWorkPost, />@alice</);
  });

  it('does not invent an X handoff when the answer has no X URL', () => {
    const html = render(response({
      visible_answer: 'Answer: 没有帖子链接。',
      receipt_expectation: '',
    }), 'zh');
    assert.doesNotMatch(html, /vnext-x-open-strip/);
    assert.doesNotMatch(html, /在 X 中打开/);
  });

  it('does not invent Jev provenance for a plain tool source', () => {
    const html = render(response({
      visible_answer: 'Answer: Reminder saved.',
      source_evidence: [{
        kind: 'tool',
        label: 'Tasks and reminders',
        summary: 'Reminder #1903 was saved.',
        detail: 'Current evidence:\n- Intent #1903 saved',
      }],
      receipt_expectation: '',
    }));
    assert.doesNotMatch(html, /vnext-jev-provenance/);
    assert.doesNotMatch(html, /data-jev-verdict/);
  });

  it('shows the owner question and a plain source, not the model search instruction', () => {
    const note = 'Use only these excerpts. Do not fill a gap from memory. Do not call web_search again in this turn.';
    const instruction = 'Weather forecast for Example City, Washington for tomorrow, Saturday. Return the exact forecast numbers. Do not invent a source.';
    const excerpt = JSON.stringify({
      status: 'ok',
      search_prompt: instruction,
      summary: `**Saturday**: Sunny, high 72°F. ${note}`,
      retryable: false,
      note,
      verified_citations: [{ title: '', url: 'https://weather.gov/sew', snippet: '**Saturday**: Sunny, high 72°F.' }],
    });
    const receipt = [
      '明天晴，大约二十度。',
      '',
      '搜索收据',
      `查了：${instruction}`,
      '1. 来源 — **Saturday**: Sunny',
      '@@search-receipt',
      JSON.stringify({
        queries: [instruction],
        sources: [{ query: instruction, title: '', url: 'https://weather.gov/sew', excerpt: `**Saturday**: Sunny, high 72°F. ${note}` }],
      }),
      '@@end-search-receipt',
    ].join('\n');
    const html = render(response({
      visible_answer: receipt,
      evidence: [{ id: 'E1', tool: 'web_search', label: 'Web search', excerpt, full_len: 796 }],
      receipt_expectation: '',
    }), 'zh', '明天天气怎么样?');
    assert.match(html, /明天晴，大约二十度/);
    assert.doesNotMatch(html, /明天天气怎么样\?/);
    assert.match(html, /weather\.gov/);
    assert.match(html, /Sunny/);
    assert.doesNotMatch(html, /Use only these excerpts/);
    assert.doesNotMatch(html, /Do not call web_search/);
    assert.doesNotMatch(html, /Return the exact/);
    assert.doesNotMatch(html, /\*\*/);
    assert.doesNotMatch(html, /可原样重试/);
    assert.doesNotMatch(html, /来源摘要 · 原文 796/);
  });
  it('keeps the completed answer first and the successful source list below it', () => {
    const body = 'Here are the official references.\n\n1. Serve overview\nhttps://tailscale.com/docs/features/tailscale-serve\n\n2. Command reference\nhttps://tailscale.com/docs/reference/tailscale-cli/serve';
    const text = `${body}\n搜索收据\n查了：repeat\n@@search-receipt\n${JSON.stringify({queries:['repeat'],sources:[{title:'Serve overview',url:'https://tailscale.com/docs/features/tailscale-serve',excerpt:'Official documentation'}]})}\n@@end-search-receipt`;
    const html = render(response({visible_answer:text,receipt_expectation:''}));
    assert.ok(html.indexOf('Here are the official references') < html.indexOf('data-testid="search-receipt"'));
    assert.doesNotMatch(html, /vnext-answer-more/);
    assert.match(html, /class="vnext-reference-list"/);
    assert.match(html, /<a [^>]*href="https:\/\/tailscale.com\/docs\/features\/tailscale-serve"[^>]*>Serve overview<\/a>/);
    assert.doesNotMatch(html, /查了|repeat/);
  });
  it('presents failed search as missing evidence and folds technical diagnostics once', () => {
    const excerpt=JSON.stringify({status:'error',source:'duckduckgo_wasm',fallback_attempted:true,fallback_used:false,reason:'duckduckgo_no_usable_results'});
    const html=render(response({visible_answer:'本次搜索未成功。',receipt_expectation:'',evidence:[{id:'E1',tool:'web_search',label:'Web search',excerpt,full_len:excerpt.length}],source_evidence:[{kind:'tool',label:'Web search',summary:'Tool returned status: error',detail:'Current evidence:\nTool returned status: error'}]}),'zh');
    assert.match(html,/未取得可用搜索来源/);
    assert.match(html,/>检索状态</);
    assert.match(html,/诊断详情/);
    assert.doesNotMatch(html,/E1 ·|Tool returned status/);
    assert.match(html,/<details class="vnext-search-diagnostics">/);
  });
  it('recognizes the real server formatted search failure, not only raw JSON', () => {
    const excerpt='Fallback attempted: true\nFallback used: false\nPrimary provider: codex_oauth\nReason: duckduckgo_no_usable_results\nSource: duckduckgo_wasm\nStatus: error';
    const html=render(response({visible_answer:'本次搜索未成功。',receipt_expectation:'',evidence:[{id:'E1',tool:'web_search',label:'Web search',excerpt,full_len:excerpt.length}]}),'zh');
    assert.match(html,/>检索状态</);
    assert.match(html,/<details class="vnext-search-diagnostics">/);
    assert.doesNotMatch(html,/E1 ·/);
  });
  it('shows the recorded search service and fallback instead of inferring from model prose', () => {
    const receipt={queries:['query'],services:[{source:'codex_subscription',fallback_used:false},{source:'duckduckgo_wasm',fallback_used:true}],sources:[{title:'Serve',url:'https://tailscale.com/docs',excerpt:''}]};
    const html=render(response({visible_answer:'找到官方文档。\n搜索收据\n@@search-receipt\n'+JSON.stringify(receipt)+'\n@@end-search-receipt',receipt_expectation:''}),'zh');
    assert.match(html,/Codex 订阅 · DuckDuckGo（兜底）/);
    assert.doesNotMatch(html,/codex_subscription/);
  });
  it('consolidates successful receipt and inspectable search evidence in one source panel', () => {
    const receipt={queries:[],sources:[{title:'Serve',url:'https://tailscale.com/docs',excerpt:'Official'}]};
    const html=render(response({visible_answer:'已找到文档 [E1]。\n搜索收据\n@@search-receipt\n'+JSON.stringify(receipt)+'\n@@end-search-receipt',receipt_expectation:'',evidence:[{id:'E1',tool:'web_search',label:'Web search',excerpt:'Serve\nhttps://tailscale.com/docs',full_len:38}]}),'zh');
    assert.equal((html.match(/class="vnext-answer-card vnext-disclosure is-collapsible tone-evidence"/g)||[]).length,1);
    assert.match(html,/data-testid="search-receipt"/);
    assert.match(html,/E1 · 检索记录/);
    assert.doesNotMatch(html,/class="vnext-answer-card tone-evidence vnext-search-sources"/);
  });
});
