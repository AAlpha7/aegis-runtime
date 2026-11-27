import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const inlineAssistantSource = readFileSync(new URL('../src/vnext/InlineAssistantPanel.tsx', import.meta.url), 'utf8');
const workViewSource = readFileSync(new URL('../src/vnext/WorkView.tsx', import.meta.url), 'utf8');
const stylesSource = readFileSync(new URL('../src/vnext/styles.css', import.meta.url), 'utf8');

// Render-based assertions go through a single esbuild bundle that re-exports the
// panel plus the LocaleProvider + t/tf, so the provider shares the SAME
// LocaleContext instance the panel reads (a separately-bundled provider would
// create a distinct context object and the locale would never propagate).
// Mirrors vnextWorkRender.test.mjs / vnextChatHomeRender.test.mjs.
const modulePath = process.env.VNEXT_INLINE_ASSISTANT_RENDER_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-inline-assistant-render-test/InlineAssistantPanel.mjs', import.meta.url).href;
const { InlineAssistantPanel, LocaleProvider, t, tf } = await import(modulePath);

const noop = () => {};

// Minimal-but-valid ContextualAssistantResponse so AssistantResponseView renders
// without crashing (it assumes citations/warnings arrays + a visible_answer).
function assistantResult(overrides = {}) {
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

function baseProps(overrides = {}) {
  return {
    text: '',
    placeholder: 'Ask about this item',
    result: null,
    turns: [],
    suggestions: [],
    busy: false,
    onChange: noop,
    onSend: noop,
    onClose: noop,
    ...overrides,
  };
}

// Default render: no LocaleProvider, so the locale defaults to zh (matching the
// LocaleContext default).
function render(props) {
  return renderToStaticMarkup(React.createElement(InlineAssistantPanel, props));
}

// Render under an explicit locale via LocaleProvider, sharing the SAME context
// instance as the panel (both come from the single inlineAssistantTestEntry bundle).
function renderWithLocale(props, displayLanguage) {
  return renderToStaticMarkup(
    React.createElement(LocaleProvider, { displayLanguage }, React.createElement(InlineAssistantPanel, props)),
  );
}

describe('vNext inline assistant panel (structure)', () => {
  it('keeps item collaboration inside the selected item with contextual suggestions', () => {
    // Structural / implementation invariants stay as source assertions — they
    // describe behavior (refs, autosize, keyboard handling) not user-facing copy.
    assert.match(inlineAssistantSource, /export type InlineAssistantSuggestion/);
    assert.match(inlineAssistantSource, /suggestions = \[\]/);
    assert.match(inlineAssistantSource, /role: 'owner' \| 'aegis' \| 'working' \| 'queued'/);
    assert.match(inlineAssistantSource, /timestamp\?: number/);
    assert.match(inlineAssistantSource, /function InlineTurnTime/);
    assert.match(inlineAssistantSource, /chatFullTimestampLabel/);
    assert.match(inlineAssistantSource, /const \[expanded, setExpanded\] = useState\(false\)/);
    assert.match(inlineAssistantSource, /className="vnext-inline-turn-time"/);
    assert.match(inlineAssistantSource, /role="button"/);
    assert.match(inlineAssistantSource, /tabIndex=\{0\}/);
    assert.match(inlineAssistantSource, /onClick=\{\(\) => setExpanded\(\(current\) => !current\)\}/);
    assert.match(inlineAssistantSource, /<InlineTurnTime timestamp=\{turn\.timestamp\} \/>/);
    // The role comparison stays identity-based; only the displayed label is localized.
    assert.match(inlineAssistantSource, /turn\.role === 'queued' \? t\('inlineAssistant\.turn\.queued'\) : t\('inlineAssistant\.turn\.you'\)/);
    // The retryLabel sentinel comparison stays a string compare (prop value from WorkView).
    assert.match(inlineAssistantSource, /retryLabel === 'Check result'/);
    assert.match(stylesSource, /\.vnext-inline-turn\.owner,\s*\.vnext-inline-turn\.queued/);
    assert.match(stylesSource, /\.vnext-inline-turn\.queued/);
    assert.match(stylesSource, /\.vnext-inline-turn-time,\s*\.vnext-chat-time/);
    assert.match(stylesSource, /\.vnext-inline-turn h3/);
    assert.match(inlineAssistantSource, /\{conversation\}[\s\S]*aria-label=\{t\('inlineAssistant\.aria\.suggestions'\)\}[\s\S]*className="vnext-inline-assistant-form"/);
    assert.match(inlineAssistantSource, /panelRef = useRef<HTMLElement/);
    assert.match(inlineAssistantSource, /focus\(\{ preventScroll: true \}\)/);
    assert.doesNotMatch(inlineAssistantSource, /scrollIntoView/);
    assert.match(inlineAssistantSource, /function resizeInlineAssistantInput\(input: HTMLTextAreaElement \| null\)/);
    assert.match(inlineAssistantSource, /input\.style\.height = 'auto'/);
    assert.match(inlineAssistantSource, /Math\.min\(input\.scrollHeight, maxHeight\)/);
    assert.match(inlineAssistantSource, /resizeInlineAssistantInput\(inputRef\.current\);/);
    assert.match(inlineAssistantSource, /rows=\{1\}/);
    assert.match(inlineAssistantSource, /resizeInlineAssistantInput\(event\.currentTarget\);[\s\S]*onChange\(event\.currentTarget\.value\);/);
    assert.match(inlineAssistantSource, /event\.key !== 'Enter' \|\| event\.shiftKey \|\| event\.nativeEvent\.isComposing/);
    assert.match(inlineAssistantSource, /event\.preventDefault\(\);[\s\S]*onSend\(inputRef\.current\?\.value \?\? text\);/);
    assert.match(stylesSource, /\.vnext-inline-assistant-form textarea\s*\{[^}]*min-height: 52px;[^}]*max-height: 128px;[^}]*overflow: hidden;[^}]*resize: none;/s);
    assert.match(inlineAssistantSource, /disabled=\{busy && !text\.trim\(\)\}/);
    assert.doesNotMatch(inlineAssistantSource, /disabled=\{busy\}[\s\S]{0,220}onClick=\{\(\) => onSend\(suggestion\.prompt\)\}/);
    assert.doesNotMatch(inlineAssistantSource, /result\?\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.doesNotMatch(inlineAssistantSource, /result\.response_kind === 'error'[\s\S]{0,260}disabled=\{busy\}/);
    assert.match(inlineAssistantSource, /<VNextIcon name="send" \/>/);
    assert.match(inlineAssistantSource, /className="vnext-inline-send-label"/);
  });

  it('sends suggestion prompts through the same item-scoped assistant handler', () => {
    assert.match(inlineAssistantSource, /onClick=\{\(\) => onSend\(suggestion\.prompt\)\}/);
    assert.match(workViewSource, /buildCommandSuggestions\(item, 'item', locale\)/);
    assert.match(workViewSource, /onAskText=\{\(text\) => submitAssistant\(item, text\)\}/);
    assert.match(workViewSource, /onAskText=\{\(text\) => selected && submitAssistant\(selected, text\)\}/);
    assert.match(workViewSource, /type QueuedInlineAssistantPrompt =/);
    assert.match(workViewSource, /queuedAssistantPrompts/);
    assert.match(workViewSource, /kind: 'prompt' \| 'retry'/);
    assert.match(workViewSource, /function retryAssistant\(\)/);
    assert.match(workViewSource, /onRetryAssistant\?\.\(\)/);
    assert.match(workViewSource, /role: 'queued'/);
    assert.match(workViewSource, /inlineAssistantConversationContextBeforeQueuedTurn/);
    assert.match(workViewSource, /function briefPrompt\(locale: Locale\): string/);
    assert.match(workViewSource, /function draftPrompt\(locale: Locale\): string/);
    assert.match(workViewSource, /appendOwnerTurn\(selected, briefPrompt\(locale\)\)/);
    assert.match(workViewSource, /appendOwnerTurn\(selected, draftPrompt\(locale\)\)/);
    assert.match(workViewSource, /submitAssistant\(selected, briefPrompt\(locale\)\)/);
    assert.match(workViewSource, /submitAssistant\(selected, draftPrompt\(locale\)\)/);
    assert.doesNotMatch(workViewSource, /appendOwnerTurn\('请/);
    assert.doesNotMatch(inlineAssistantSource, /window\.location/);
    assert.doesNotMatch(inlineAssistantSource, /navigate\(/);
  });
});

describe('vNext inline assistant panel (render)', () => {
  it('renders the default heading, collapse control, and send affordance (zh default)', () => {
    const html = render(baseProps());

    assert.ok(html.includes(t('inlineAssistant.label', 'zh')));
    assert.ok(html.includes(`>${t('inlineAssistant.collapse', 'zh')}<`));
    assert.ok(html.includes(`aria-label="${t('inlineAssistant.aria.sendToAegis', 'zh')}"`));
    assert.ok(html.includes(`title="${t('inlineAssistant.aria.sendToAegis', 'zh')}"`));
    assert.ok(html.includes(`>${t('inlineAssistant.send.send', 'zh')}</span>`));
    // The textarea ask aria-label is localized.
    assert.ok(html.includes(`aria-label="${t('inlineAssistant.aria.ask', 'zh')}"`));
  });

  it('uses busy queue copy for the send button when an assistant turn is in flight', () => {
    const html = render(baseProps({ busy: true, text: 'follow up' }));

    assert.ok(html.includes(`aria-label="${t('inlineAssistant.aria.queueForAegis', 'zh')}"`));
    assert.ok(html.includes(`>${t('inlineAssistant.send.queue', 'zh')}</span>`));
    assert.ok(!html.includes(`>${t('inlineAssistant.send.send', 'zh')}</span>`));
  });

  it('localizes turn labels for queued and owner turns', () => {
    const html = render(baseProps({
      turns: [
        { id: 'q1', role: 'queued', text: 'queued prompt' },
        { id: 'o1', role: 'owner', text: 'owner prompt' },
        { id: 'w1', role: 'working', text: '' },
      ],
    }));

    assert.ok(html.includes(`aria-label="${t('inlineAssistant.aria.conversation', 'zh')}"`));
    assert.ok(html.includes(`>${t('inlineAssistant.turn.queued', 'zh')}</span>`));
    assert.ok(html.includes(`>${t('inlineAssistant.turn.you', 'zh')}</span>`));
    // Working turn with empty text falls back to the localized "working" copy.
    assert.ok(html.includes(t('inlineAssistant.turn.working', 'zh')));
    // The owner/queued role text itself is passed through verbatim.
    assert.match(html, /queued prompt/);
    assert.match(html, /owner prompt/);
  });

  it('localizes the suggestions list aria-label and renders suggestion labels verbatim', () => {
    const html = render(baseProps({
      suggestions: [{ id: 's1', label: 'Draft a reply', prompt: 'draft' }],
    }));

    assert.ok(html.includes(`aria-label="${t('inlineAssistant.aria.suggestions', 'zh')}"`));
    assert.match(html, /Draft a reply/);
  });

  it('localizes retry copy and keeps the Check-result sentinel working', () => {
    const errorResult = assistantResult({ response_kind: 'error', visible_answer: "I couldn't finish that." });

    const tryAgain = render(baseProps({ result: errorResult, onRetry: noop }));
    assert.ok(tryAgain.includes(t('inlineAssistant.retry.tryAgain', 'zh')));

    const queueRetry = render(baseProps({ result: errorResult, onRetry: noop, busy: true }));
    assert.ok(queueRetry.includes(t('inlineAssistant.retry.queueRetry', 'zh')));

    const queueCheck = render(baseProps({ result: errorResult, onRetry: noop, busy: true, retryLabel: 'Check result' }));
    assert.ok(queueCheck.includes(t('inlineAssistant.retry.queueCheck', 'zh')));

    // `retryLabel` is the English logic key set by App.tsx ('Check result' /
    // 'Try again'); the displayed text is always localized. 'Check result'
    // maps to its zh display; any other value (incl. the 'Try again' key)
    // maps to the localized tryAgain — never rendered as raw English.
    const checkResultDisplay = render(baseProps({ result: errorResult, onRetry: noop, retryLabel: 'Check result' }));
    assert.ok(checkResultDisplay.includes(t('inlineAssistant.retry.checkResult', 'zh')));
    assert.doesNotMatch(checkResultDisplay, />Check result<\/button>/);

    const tryAgainKey = render(baseProps({ result: errorResult, onRetry: noop, retryLabel: 'Try again' }));
    assert.ok(tryAgainKey.includes(t('inlineAssistant.retry.tryAgain', 'zh')));
    assert.doesNotMatch(tryAgainKey, />Try again<\/button>/);
  });

  it('localizes the inline turn-time aria-label across expand/collapse', () => {
    const html = render(baseProps({
      turns: [{ id: 'o1', role: 'owner', text: 'hi', timestamp: 1_700_000_000_000 }],
    }));

    // Collapsed (default) state surfaces the "show full time" aria-label prefix.
    const showPrefix = t('inlineAssistant.aria.showFullTime', 'zh').split('{time}')[0];
    assert.ok(html.includes(`aria-label="${showPrefix}`));
  });

  it('localizes chrome under both zh and en via LocaleProvider', () => {
    const probeKeys = [
      'inlineAssistant.collapse',
      'inlineAssistant.aria.ask',
      'inlineAssistant.aria.sendToAegis',
      'inlineAssistant.send.send',
    ];

    const props = baseProps({
      suggestions: [{ id: 's1', label: 'Draft', prompt: 'd' }],
    });
    const zhHtml = renderWithLocale(props, 'zh');
    const enHtml = renderWithLocale(props, 'en');

    for (const key of probeKeys) {
      assert.ok(zhHtml.includes(t(key, 'zh')), `zh render should contain ${key} zh copy`);
      assert.ok(enHtml.includes(t(key, 'en')), `en render should contain ${key} en copy`);
    }

    // English render must surface English chrome and never the Chinese copy for
    // a key whose translations actually differ.
    assert.ok(enHtml.includes(t('inlineAssistant.collapse', 'en')), 'en render shows English Collapse');
    assert.ok(!enHtml.includes(t('inlineAssistant.collapse', 'zh')), 'en render must not leak zh Collapse');
    assert.ok(zhHtml.includes(t('inlineAssistant.collapse', 'zh')), 'zh render shows Chinese Collapse');

    // Interpolated turn-time aria-label localizes too.
    const timeProps = baseProps({
      turns: [{ id: 'o1', role: 'owner', text: 'hi', timestamp: 1_700_000_000_000 }],
    });
    const zhTime = renderWithLocale(timeProps, 'zh');
    const enTime = renderWithLocale(timeProps, 'en');
    assert.ok(zhTime.includes(t('inlineAssistant.aria.showFullTime', 'zh').split('{time}')[0]));
    assert.ok(enTime.includes(t('inlineAssistant.aria.showFullTime', 'en').split('{time}')[0]));
  });
});
