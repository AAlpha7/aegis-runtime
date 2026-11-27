import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_CEO_VOICE_PANEL_MODULE;
assert.ok(modulePath, 'VNEXT_CEO_VOICE_PANEL_MODULE is required');

const {
  panelItemsForAskResult,
  settlePanelWorking,
  mapViewToClientContext,
  buildPanelConversationContext,
  truncateScreenContext,
  CEO_SCREEN_CONTEXT_BUDGET,
  liveSpeakingLine,
  voiceApprovalQuery,
  scrubToolWaitDump,
  ownerContinuationLine,
  voiceGateToolMessage,
} = await import(modulePath);

describe('floating CEO panel stream settlement', () => {
  it('maps shell views to backend client_context without forcing home', () => {
    assert.equal(mapViewToClientContext('work', null), 'work');
    assert.equal(mapViewToClientContext('status', null), 'status');
    assert.equal(mapViewToClientContext('settings', null), 'settings');
    assert.equal(mapViewToClientContext('home', null), 'home');
  });

  it('does not force item scope from sticky selectedId on free-form panel turns', () => {
    // Owner bug: research questions were answered as the previously selected
    // passport tracking task because home/work still had focus_item_id set.
    assert.equal(mapViewToClientContext('home', 'task:passport-followup'), 'home');
    assert.equal(mapViewToClientContext('work', 'task:1'), 'work');
    assert.equal(mapViewToClientContext('home', '  '), 'home');
  });

  it('always leads conversation_context with screen context', () => {
    const turns = buildPanelConversationContext({
      screenContext: 'surface=work\nneeds_you=(none)',
      stream: [
        { id: 'u1', type: 'user', text: '上一句' },
        { id: 'a1', type: 'assistant', text: '上一答' },
        { id: 'u2', type: 'user', text: '这封怎么办' },
      ],
      excludeLatestUserText: '这封怎么办',
    });
    assert.ok(turns[0].text.includes('surface=work'));
    assert.ok(turns[0].text.includes('situational only'));
    assert.ok(turns[0].text.includes('do not pivot'));
    assert.equal(turns[0].role, 'aegis');
    assert.deepEqual(
      turns.slice(1).map((t) => t.text),
      ['上一句', '上一答'],
    );
  });

  it('mirrors a secretary reply into the panel stream', () => {
    let n = 0;
    const nextId = () => `id-${++n}`;
    const items = panelItemsForAskResult(
      { kind: 'replied', text: '日历已就绪' },
      'zh',
      nextId,
    );
    assert.equal(items.length, 1);
    assert.equal(items[0].type, 'assistant');
    assert.equal(items[0].text, '日历已就绪');
  });

  it('surfaces queue and error kinds without dropping the owner turn', () => {
    let n = 0;
    const nextId = () => `id-${++n}`;
    const queued = panelItemsForAskResult({ kind: 'queued' }, 'zh', nextId);
    assert.equal(queued[0].type, 'action');
    assert.match(queued[0].detail, /排队/);

    const failed = panelItemsForAskResult(
      { kind: 'error', text: '网络超时' },
      'zh',
      nextId,
    );
    assert.equal(failed[0].type, 'error');
    assert.equal(failed[0].text, '网络超时');
  });

  it('replaces the working placeholder with the reply', () => {
    let n = 0;
    const nextId = () => `id-${++n}`;
    const stream = [
      { id: 'u1', type: 'user', text: '今天' },
      { id: 'w1', type: 'working', text: '…' },
    ];
    const next = settlePanelWorking(
      stream,
      'w1',
      { kind: 'replied', text: '今天有 2 件事需要你。' },
      'zh',
      nextId,
    );
    assert.equal(next.length, 2);
    assert.equal(next[0].type, 'user');
    assert.equal(next[1].type, 'assistant');
    assert.equal(next[1].text, '今天有 2 件事需要你。');
    assert.ok(!next.some((item) => item.type === 'working'));
  });

  it('does not invent a reply for empty/skipped results', () => {
    let n = 0;
    const nextId = () => `id-${++n}`;
    assert.deepEqual(panelItemsForAskResult({ kind: 'empty' }, 'en', nextId), []);
    assert.deepEqual(panelItemsForAskResult({ kind: 'skipped' }, 'en', nextId), []);
  });

  it('keeps one speaking line and retires it when the utterance ends', () => {
    assert.equal(liveSpeakingLine(true, '  明天晴，大约二十度。  '), '明天晴，大约二十度。');
    assert.equal(liveSpeakingLine(false, '明天晴，大约二十度。'), '');
  });

  it('hides a tool-wait dump from the speaking line and keeps the search text', () => {
    const summary = [
      'destination: search-provider',
      'would_send: Find the approximate geographic coordinates of Example City.',
      'stopped: untrusted_source',
      'untrusted_source: tool_result',
    ].join('\n');
    assert.equal(liveSpeakingLine(true, 'Tool action · 等待你批准 / awaiting your approval'), '');
    assert.equal(voiceApprovalQuery(summary), 'Find the approximate geographic coordinates of Example City.');
    assert.equal(voiceApprovalQuery('web_search'), '');
    assert.equal(scrubToolWaitDump('等待你批准这次搜索。'), '');
    assert.equal(scrubToolWaitDump('明天晴，大约二十度。'), '明天晴，大约二十度。');
    assert.equal(ownerContinuationLine('明天晴，大约二十度。', 'approve'), '明天晴，大约二十度。');
    assert.equal(ownerContinuationLine('destination: search-provider', 'approve'), '已批准，正在继续查。');
    assert.equal(ownerContinuationLine('anything', 'reject'), '先不查了。');
    assert.equal(voiceGateToolMessage('等待你批准这次搜索。', 'zh'), '需要你确认：请说「批准」或点卡片上的批准。');
    assert.doesNotMatch(voiceGateToolMessage('Tool action · 等待你批准', 'zh'), /Tool action|web_search|destination:/);
  });

  it('truncates screen context to the soft ~2KB budget on a line boundary when possible', () => {
    assert.equal(CEO_SCREEN_CONTEXT_BUDGET, 2048);
    const short = 'surface=work\nfocus_title=ok';
    assert.equal(truncateScreenContext(short), short);
    const long = Array.from({ length: 80 }, (_, i) => `line_${i}=${'x'.repeat(40)}`).join('\n');
    const truncated = truncateScreenContext(long, 500);
    assert.ok(truncated.length <= 500);
    assert.ok(truncated.endsWith('\n…') || truncated.includes('…'));
    assert.ok(!truncated.includes('line_79='));
  });
});

describe('conversation context merge honours the server cap', () => {
  it('keeps the screen turn first and the newest turns, never more than 8', async (context) => {
    const modulePath = process.env.VNEXT_CEO_VOICE_PANEL_MODULE;
    if (!modulePath) { context.skip('compiled module not provided'); return; }
    const mod = await import(modulePath);
    const turn = (i, role = 'owner') => ({ role, text: `t${i}` });
    const panel = [{ role: 'aegis', text: 'screen' }, ...Array.from({ length: 6 }, (_, i) => turn(100 + i))];
    const history = Array.from({ length: 6 }, (_, i) => turn(i));
    const merged = mod.mergeConversationContext(panel, history);
    assert.equal(merged.length, mod.VNEXT_MAX_CONVERSATION_CONTEXT_TURNS);
    assert.equal(merged[0].text, 'screen');
    assert.equal(merged[merged.length - 1].text, 't105');
    assert.equal(mod.VNEXT_MAX_CONVERSATION_CONTEXT_TURNS, 8);
    assert.deepEqual(mod.mergeConversationContext([{ role: 'aegis', text: 'screen' }], []), [{ role: 'aegis', text: 'screen' }]);
  });
});
