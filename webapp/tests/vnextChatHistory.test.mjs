import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_CHAT_HISTORY_MODULE
  ?? 'file:///tmp/aegis-runtime-webapp-vnext-chat-history-test/vnext/chatHistory.js';

const {
  chatHomeHistoryOwnerKey,
  completeChatHomeTurn,
  chatHomeMessagesFromServerHistory,
  clearChatHomeHistory,
  compactChatHomeHistory,
  mergeChatHomeServerAndLocalHistory,
  readChatHomeHistory,
  writeChatHomeHistory,
} = await import(modulePath);

function memoryStorage() {
  const values = new Map();
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    removeItem(key) {
      values.delete(key);
    },
  };
}

describe('vNext chat history', () => {
  it('uses an owner-scoped persistent key for authenticated chat history', () => {
    assert.equal(
      chatHomeHistoryOwnerKey('aegis-runtime:vnext:chat-home-history', ' L@example.com '),
      'aegis-runtime:vnext:chat-home-history:owner:l@example.com',
    );
    assert.equal(
      chatHomeHistoryOwnerKey('aegis-runtime:vnext:chat-home-history', 'Owner Name+test@example.com'),
      'aegis-runtime:vnext:chat-home-history:owner:owner_name+test@example.com',
    );
    assert.equal(
      chatHomeHistoryOwnerKey('aegis-runtime:vnext:chat-home-history', ''),
      'aegis-runtime:vnext:chat-home-history:owner',
    );
  });

  it('persists only finished owner and Aegis turns', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [
      { id: 'owner:1', role: 'owner', text: '通勤', timestamp: Date.UTC(2026, 4, 18, 10, 40) },
      { id: 'owner:2', role: 'owner', text: '解释', contextLabel: 'Billing statement' },
      { role: 'working' },
      { role: 'queued' },
      {
        id: 'aegis:1',
        role: 'aegis',
        text: 'Use the usual route.',
        severity: 'important',
        timestamp: Date.UTC(2026, 4, 18, 10, 41),
        result: {
          schema_version: 1,
          response_kind: 'answer_now',
          visible_answer: 'Use the usual route.',
          severity: 'important',
          receipt_expectation: 'No action was taken.',
          citations: ['memory:commute'],
          source_evidence: [{
            kind: 'tool',
            label: 'Tasks and reminders',
            summary: 'Reminder #1903 was saved.',
            detail: 'Current evidence:\n- Intent #1903 saved\n- cadence: every 12 hours',
          }],
          warnings: [],
        },
        retryText: '通勤',
        retryIdempotencyKey: 'vnext-chat:retry-key',
        retryLabel: 'Check result',
        retryContext: { itemId: 'email:123', label: 'Billing statement' },
      },
    ]);

    assert.deepEqual(readChatHomeHistory(storage, 'chat').map((message) => message.role), ['owner', 'owner', 'aegis']);
    assert.equal(storage.getItem('chat').includes('queued'), false);
    assert.equal(readChatHomeHistory(storage, 'chat')[0].timestamp, Date.UTC(2026, 4, 18, 10, 40));
    assert.equal(readChatHomeHistory(storage, 'chat')[1].contextLabel, 'Billing statement');
    assert.equal(readChatHomeHistory(storage, 'chat')[2].timestamp, Date.UTC(2026, 4, 18, 10, 41));
    assert.equal(readChatHomeHistory(storage, 'chat')[2].severity, 'important');
    assert.equal(readChatHomeHistory(storage, 'chat')[2].result.visible_answer, 'Use the usual route.');
    assert.equal(readChatHomeHistory(storage, 'chat')[2].result.severity, 'important');
    assert.equal(readChatHomeHistory(storage, 'chat')[2].result.source_evidence[0].label, 'Tasks and reminders');
    assert.match(readChatHomeHistory(storage, 'chat')[2].result.source_evidence[0].detail, /Intent #1903 saved/);
    assert.equal(readChatHomeHistory(storage, 'chat')[2].result.source_evidence[0].jev, undefined);
    assert.equal(readChatHomeHistory(storage, 'chat')[2].retryIdempotencyKey, 'vnext-chat:retry-key');
    assert.equal(readChatHomeHistory(storage, 'chat')[2].retryLabel, 'Check result');
    assert.deepEqual(readChatHomeHistory(storage, 'chat')[2].retryContext, {
      itemId: 'email:123',
      label: 'Billing statement',
    });
  });

  it('keeps low-value system status notices out of the main chat history', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [
      { id: 'owner:1', role: 'owner', text: '通勤', timestamp: 1 },
      {
        id: 'restart:1',
        role: 'aegis',
        text: [
          '系统已重启 / System restarted',
          '当前重要信息',
          '活跃技能 / Active skills: 22',
        ].join('\n'),
        timestamp: 2,
      },
      {
        id: 'integrity:1',
        role: 'aegis',
        text: [
          'System Integrity Report',
          'Invariants: 3/3 ✅',
          'Rate Monitors: 4/4 ✅',
        ].join('\n'),
        timestamp: 3,
      },
      { id: 'reminder:1', role: 'aegis', text: '⏰ Review the lawyer email.', timestamp: 4 },
    ]);

    assert.deepEqual(
      readChatHomeHistory(storage, 'chat').map((message) => message.id),
      ['owner:1', 'reminder:1'],
    );
  });

  it('stores only the safe display subset of assistant results', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [{
      id: 'aegis:1',
      role: 'aegis',
      text: 'Done.',
      result: {
        schema_version: 1,
        response_kind: 'answer',
        visible_answer: 'Done.',
        severity: 'warning',
        receipt_expectation: 'No action was taken.',
        citations: ['visible source', 'tool:gmail_reader', 'visible_fields.subject', 'provider_request_id=req_123'],
        source_evidence: [
          {
            kind: 'tool',
            label: 'Tasks and reminders',
            summary: 'Reminder #1903 was saved.',
            detail: 'Current evidence:\n- Intent #1903 saved\n- context: 关注 Grok Build agent\n- cadence: every 12 hours',
          },
          {
            kind: 'tool',
            label: 'Provider internals',
            summary: 'Must not persist.',
            detail: 'provider_request_id=req_secret',
          },
        ],
        warnings: [
          'safe warning',
          'mfa_required',
          'more_detail_requires_gate',
          'source_content_untrusted',
          'tool_blocked:calendar_reader',
          'trace_id=trace-secret',
        ],
        authority: { allowed_now: ['read_secret'] },
        action_draft: { secret: 'do not persist' },
        required_gate: { private: true },
        provider_request_id: 'provider-secret',
        run_id: 'run-secret',
        trace_id: 'trace-secret',
      },
    }]);

    const raw = storage.getItem('chat');
    assert.equal(raw.includes('authority'), false);
    assert.equal(raw.includes('action_draft'), false);
    assert.equal(raw.includes('required_gate'), false);
    assert.equal(raw.includes('provider-secret'), false);
    assert.equal(raw.includes('run-secret'), false);
    assert.equal(raw.includes('trace-secret'), false);
    assert.equal(raw.includes('provider_request_id'), false);
    assert.equal(raw.includes('visible_fields'), false);
    assert.equal(raw.includes('source_content_untrusted'), false);
    assert.equal(raw.includes('mfa_required'), false);
    assert.equal(raw.includes('tool_blocked'), false);

    const restored = readChatHomeHistory(storage, 'chat')[0].result;
    assert.deepEqual(Object.keys(restored).sort(), [
      'citations',
      'receipt_expectation',
      'response_kind',
      'schema_version',
      'severity',
      'source_evidence',
      'visible_answer',
      'warnings',
    ]);
    assert.equal(restored.severity, 'warning');
    assert.deepEqual(restored.citations, ['visible source', 'Aegis tool']);
    assert.deepEqual(restored.source_evidence, [{
      kind: 'tool',
      label: 'Tasks and reminders',
      summary: 'Reminder #1903 was saved.',
      detail: 'Current evidence:\n- Intent #1903 saved\n- context: 关注 Grok Build agent\n- cadence: every 12 hours',
    }]);
    assert.deepEqual(restored.warnings, [
      'safe warning',
      'Extra confirmation required',
      'More detail needs your approval.',
      'Calendar was unavailable or not authorized.',
    ]);
  });

  it('keeps a source fact when the excerpt also names a secret', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [{
      id: 'aegis:1',
      role: 'aegis',
      text: 'Saved.',
      result: {
        schema_version: 1,
        response_kind: 'answer',
        visible_answer: 'Saved.',
        receipt_expectation: '',
        citations: [],
        warnings: [],
        source_evidence: [
          {
            kind: 'tool',
            label: 'Tasks and reminders',
            summary: 'Call family was saved.',
            detail: 'Call family was saved.\nprovider_request_id=req_secret',
          },
          {
            kind: 'tool',
            label: 'Provider internals',
            summary: 'Must not persist.',
            detail: 'provider_request_id=req_secret',
          },
        ],
        evidence: [
          {
            id: 'E1',
            tool: 'manage_tasks',
            label: 'Tasks and reminders',
            excerpt: 'Call family was saved. turn_id=turn-secret',
            full_len: 80,
          },
        ],
      },
    }]);
    const raw = storage.getItem('chat');
    assert.equal(raw.includes('provider_request_id'), false);
    assert.equal(raw.includes('req_secret'), false);
    assert.equal(raw.includes('turn_id'), false);
    assert.equal(raw.includes('turn-secret'), false);
    const restored = readChatHomeHistory(storage, 'chat')[0].result;
    assert.deepEqual(restored.source_evidence, [{
      kind: 'tool',
      label: 'Tasks and reminders',
      summary: 'Call family was saved.',
      detail: 'Call family was saved.',
    }]);
    assert.equal(restored.evidence[0].excerpt, 'Call family was saved.');
  });

  it('restores runtime notices in the console locale (explicit zh, or the stamped <html lang>)', () => {
    const persisted = [{
      id: 'aegis:1',
      role: 'aegis',
      text: 'answer',
      result: {
        schema_version: 1,
        response_kind: 'answer',
        visible_answer: 'answer',
        receipt_expectation: '',
        citations: [],
        warnings: ['mfa_required', 'more_detail_requires_gate', 'tool_blocked:calendar_reader'],
      },
    }];
    const zhWarnings = ['需要额外确认', '更多细节需要你的批准。', '日历暂不可用或未授权。'];
    const enWarnings = ['Extra confirmation required', 'More detail needs your approval.', 'Calendar was unavailable or not authorized.'];

    // Explicit locale from the console (App passes errorLocale).
    const zhStorage = memoryStorage();
    writeChatHomeHistory(zhStorage, 'chat', persisted, 'zh');
    assert.deepEqual(readChatHomeHistory(zhStorage, 'chat', 'zh')[0].result.warnings, zhWarnings);
    assert.deepEqual(compactChatHomeHistory(persisted, 'zh')[0].result.warnings, zhWarnings);
    assert.deepEqual(compactChatHomeHistory(persisted, 'en')[0].result.warnings, enWarnings);

    // No explicit locale: follows the <html lang> the LocaleProvider stamps
    // (index.html ships zh-CN, so a zh console never falls back to English).
    const hadDocument = Object.prototype.hasOwnProperty.call(globalThis, 'document');
    const previousDocument = globalThis.document;
    try {
      globalThis.document = { documentElement: { lang: 'zh-CN' } };
      const stampedZh = memoryStorage();
      writeChatHomeHistory(stampedZh, 'chat', persisted);
      assert.deepEqual(readChatHomeHistory(stampedZh, 'chat')[0].result.warnings, zhWarnings);

      globalThis.document = { documentElement: { lang: 'en' } };
      const stampedEn = memoryStorage();
      writeChatHomeHistory(stampedEn, 'chat', persisted);
      assert.deepEqual(readChatHomeHistory(stampedEn, 'chat')[0].result.warnings, enWarnings);
    } finally {
      if (hadDocument) globalThis.document = previousDocument;
      else delete globalThis.document;
    }
  });

  it('persists claim evidence and unsupported numbers for inspectable answers', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [{
      id: 'aegis:1',
      role: 'aegis',
      text: '约 44 分钟 [E1]',
      result: {
        schema_version: 1,
        response_kind: 'answer',
        visible_answer: '约 44 分钟 [E1]，出发约 60–80 分钟',
        receipt_expectation: '',
        citations: [],
        warnings: ['unsupported_numbers:1'],
        evidence: [{
          id: 'E1',
          tool: 'commute_traffic_monitor',
          label: '通勤检查',
          excerpt: '当前交通约 44 分钟。',
          full_len: 1234,
        }],
        unsupported_numbers: ['60–80'],
      },
    }]);

    const restored = readChatHomeHistory(storage, 'chat')[0].result;
    assert.deepEqual(restored.evidence, [{
      id: 'E1',
      tool: 'commute_traffic_monitor',
      label: '通勤检查',
      excerpt: '当前交通约 44 分钟。',
      full_len: 1234,
    }]);
    assert.deepEqual(restored.unsupported_numbers, ['60–80']);
    assert.equal(restored.warnings.includes('unsupported_numbers:1'), false);
  });

  it('does not persist returned work cards or inline item thread details', () => {
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [{
      id: 'aegis:work-card',
      role: 'aegis',
      text: 'I found a mail item.',
      result: {
        schema_version: 1,
        response_kind: 'answer',
        visible_answer: 'I found a mail item.',
        receipt_expectation: 'No action was taken.',
        citations: [],
        warnings: [],
      },
      workItem: {
        id: 'email:secret',
        title: 'Sensitive mail subject',
        source_id: 'gmail-secret-id',
        authority: {
          visible_fields: ['subject'],
          locked_fields: ['body', 'html', 'attachments'],
        },
        sourceDetail: {
          body: 'private email body',
          html: '<p>private html body</p>',
          attachments: [{ id: 'attachment-secret' }],
        },
      },
      workItemThread: [
        {
          id: 'thread:1',
          question: 'summarize this private body',
          state: 'working',
          result: {
            visible_answer: 'private thread answer',
            provider_request_id: 'provider-secret',
          },
        },
      ],
    }]);

    const raw = storage.getItem('chat');
    assert.equal(raw.includes('workItem'), false);
    assert.equal(raw.includes('workItemThread'), false);
    assert.equal(raw.includes('Sensitive mail subject'), false);
    assert.equal(raw.includes('gmail-secret-id'), false);
    assert.equal(raw.includes('private email body'), false);
    assert.equal(raw.includes('private html body'), false);
    assert.equal(raw.includes('attachment-secret'), false);
    assert.equal(raw.includes('provider-secret'), false);
    assert.equal(readChatHomeHistory(storage, 'chat')[0].text, 'I found a mail item.');
  });

  it('caps restored history to the most recent turns', () => {
    const messages = Array.from({ length: 30 }, (_, index) => ({
      id: `owner:${index}`,
      role: 'owner',
      text: `message ${index}`,
    }));

    const compacted = compactChatHomeHistory(messages);
    assert.equal(compacted.length, 24);
    assert.equal(compacted[0].text, 'message 6');
    assert.equal(compacted[23].text, 'message 29');
  });

  it('merges server and local chat history in chronological order', () => {
    const serverMessages = [
      { id: 'server:1', role: 'owner', text: 'older server owner', timestamp: 1000 },
      { id: 'server:2', role: 'aegis', text: 'older server answer', timestamp: 2000 },
      { id: 'server:3', role: 'owner', text: 'newer server owner', timestamp: 5000 },
    ];
    const localMessages = [
      { id: 'local:1', role: 'owner', text: 'local between server turns', timestamp: 3000 },
      { id: 'local:2', role: 'working', text: 'working should not persist', timestamp: 3500 },
      { id: 'local:3', role: 'aegis', text: 'local answer between server turns', timestamp: 4000 },
    ];

    const merged = mergeChatHomeServerAndLocalHistory(serverMessages, localMessages);

    assert.deepEqual(merged.map((message) => message.text), [
      'older server owner',
      'older server answer',
      'local between server turns',
      'local answer between server turns',
      'newer server owner',
    ]);
    assert.equal(merged.some((message) => message.role === 'working'), false);
  });

  it('treats an identical Aegis reply as the same turn even minutes apart (504 → 查看结果 tap)', () => {
    const storedAt = 1_700_000_000_000;
    const serverMessages = [
      { id: 'server:1', role: 'owner', text: '看看哪些占了大量空间', timestamp: storedAt - 120_000 },
      { id: 'server:2', role: 'aegis', text: '占用最大的目录：/home/ubuntu 64.9 GB [E1]', timestamp: storedAt },
    ];
    const localMessages = [
      { id: 'local:1', role: 'owner', text: '看看哪些占了大量空间', timestamp: storedAt - 121_000 },
      // The owner tapped 查看结果 five minutes after the server stored the reply.
      { id: 'local:2', role: 'aegis', text: '占用最大的目录：/home/ubuntu 64.9 GB [E1]', timestamp: storedAt + 5 * 60_000 },
    ];
    const merged = mergeChatHomeServerAndLocalHistory(serverMessages, localMessages);
    assert.deepEqual(merged.map((m) => m.role), ['owner', 'aegis']);

    // Owner turns keep the tight window: the same text sent again 5 minutes later is a new turn.
    const resent = mergeChatHomeServerAndLocalHistory(
      [{ id: 'server:1', role: 'owner', text: '再来一次', timestamp: storedAt }],
      [{ id: 'local:1', role: 'owner', text: '再来一次', timestamp: storedAt + 5 * 60_000 }],
    );
    assert.equal(resent.length, 2);
  });

  it('keeps an in-flight chat reply visible while server history has not caught up', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [
        { id: 'server:owner:1', role: 'owner', text: 'NEXUS 流程', timestamp: 1_000 },
      ],
      [
        { id: 'local:owner:1', role: 'owner', text: 'NEXUS 流程', timestamp: 1_000 },
        { id: 'local:working:1', role: 'working', text: 'Still working...', timestamp: 1_100 },
      ],
    );

    assert.deepEqual(merged.map((message) => message.role), ['owner', 'working']);
    assert.equal(merged[1].text, 'Still working...');
  });

  it('keeps an in-flight working reply under its owner after server timestamp refresh', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [
        { id: 'server:owner:1', role: 'owner', text: 'hello', timestamp: 2_000 },
      ],
      [
        { id: 'local:owner:1', role: 'owner', text: 'hello', timestamp: 1_000 },
        {
          id: 'local:working:1',
          role: 'working',
          text: 'Still working. Looking through the allowed tools...',
          timestamp: 1_100,
        },
      ],
    );

    assert.deepEqual(merged.map((message) => message.role), ['owner', 'working']);
    assert.equal(merged[0].text, 'hello');
    assert.equal(merged[1].text, 'Still working. Looking through the allowed tools...');
  });

  it('does not clear an in-flight reply because an unrelated server answer arrived later', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [
        { id: 'server:owner:old', role: 'owner', text: 'old question', timestamp: 1_000 },
        { id: 'server:aegis:old', role: 'aegis', text: 'unrelated later notice', timestamp: 5_000 },
      ],
      [
        { id: 'local:owner:1', role: 'owner', text: 'NEXUS 流程', timestamp: 2_000 },
        { id: 'local:working:1', role: 'working', text: 'Still working...', timestamp: 2_100 },
      ],
    );

    assert.deepEqual(merged.map((message) => message.text), [
      'old question',
      'NEXUS 流程',
      'Still working...',
      'unrelated later notice',
    ]);
    assert.equal(merged.some((message) => message.role === 'working'), true);
  });

  it('replaces an in-flight chat reply when server history restores the completed answer', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [
        { id: 'server:owner:1', role: 'owner', text: 'NEXUS 流程', timestamp: 1_000 },
        { id: 'server:aegis:1', role: 'aegis', text: '带 NEXUS 卡并如实申报。', timestamp: 3_000 },
      ],
      [
        { id: 'local:owner:1', role: 'owner', text: 'NEXUS 流程', timestamp: 1_000 },
        { id: 'local:working:1', role: 'working', text: 'Still working...', timestamp: 1_100 },
      ],
    );

    assert.deepEqual(merged.map((message) => message.role), ['owner', 'aegis']);
    assert.equal(merged[1].text, '带 NEXUS 卡并如实申报。');
  });

  it('does not let server status notices evict local owner conversation', () => {
    const serverMessages = [
      { id: 'server:restart:1', role: 'aegis', text: '系统已重启 / System restarted', timestamp: 10_000 },
      { id: 'server:restart:2', role: 'aegis', text: '系统已重启 / System restarted', timestamp: 20_000 },
      { id: 'server:aegis:1', role: 'aegis', text: 'Route looks normal.', timestamp: 30_000 },
    ];
    const localMessages = [
      { id: 'local:owner:1', role: 'owner', text: '通勤', timestamp: 1_000 },
      { id: 'local:aegis:1', role: 'aegis', text: '预计 44 分钟。', timestamp: 2_000 },
    ];

    const merged = mergeChatHomeServerAndLocalHistory(serverMessages, localMessages);

    assert.deepEqual(merged.map((message) => message.id), [
      'local:owner:1',
      'local:aegis:1',
      'server:aegis:1',
    ]);
  });

  it('deduplicates server-restored turns against local whitespace variants when timestamps are close', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [{ id: 'server:1', role: 'owner', text: '通勤 路况', timestamp: 1000 }],
      [
        { id: 'local:1', role: 'owner', text: '通勤   路况', timestamp: 2000 },
        { id: 'local:2', role: 'aegis', text: '预计 44 分钟。', timestamp: 3000 },
      ],
    );

    assert.deepEqual(merged.map((message) => message.text), ['通勤 路况', '预计 44 分钟。']);
  });

  it('preserves local structured source evidence when server history restores the same turn', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [{ id: 'server:aegis:1', role: 'aegis', text: '已开始关注 Grok Build agent。', timestamp: 1_000 }],
      [{
        id: 'local:aegis:1',
        role: 'aegis',
        text: '已开始关注 Grok Build agent。',
        timestamp: 2_000,
        result: {
          schema_version: 1,
          response_kind: 'answer',
          visible_answer: '已开始关注 Grok Build agent。',
          receipt_expectation: 'No action was taken.',
          citations: ['Tasks and reminders'],
          source_evidence: [{
            kind: 'tool',
            label: 'Tasks and reminders',
            summary: 'Reminder #1903 was saved.',
            detail: 'Current evidence:\n- Intent #1903 saved\n- cadence: every 12 hours',
          }],
          warnings: [],
        },
      }],
    );

    assert.equal(merged.length, 1);
    assert.equal(merged[0].id, 'server:aegis:1');
    assert.equal(merged[0].timestamp, 1_000);
    assert.equal(merged[0].result.source_evidence[0].detail, 'Current evidence:\n- Intent #1903 saved\n- cadence: every 12 hours');
  });

  it('keeps repeated identical owner requests when they happened minutes apart', () => {
    const merged = mergeChatHomeServerAndLocalHistory(
      [
        { id: 'server:1', role: 'owner', text: '11点提醒我发表师父新文章', timestamp: 1_000 },
        { id: 'server:2', role: 'aegis', text: '已设好：今晚 11:00。', timestamp: 2_000 },
      ],
      [
        { id: 'local:1', role: 'owner', text: '11点提醒我发表师父新文章', timestamp: 15 * 60 * 1000 },
        { id: 'local:2', role: 'aegis', text: '已经有了，不重复创建。', timestamp: 15 * 60 * 1000 + 5_000 },
      ],
    );

    assert.deepEqual(merged.map((message) => message.text), [
      '11点提醒我发表师父新文章',
      '已设好：今晚 11:00。',
      '11点提醒我发表师父新文章',
      '已经有了，不重复创建。',
    ]);
  });

  it('maps server history result when evidence, Jev source_evidence, or unsupported numbers are present', () => {
    const mapped = chatHomeMessagesFromServerHistory({
      messages: [
        { role: 'owner', text: '通勤', timestamp: 1_777_000_000 },
        { role: 'aegis', text: '预计 44 分钟。', timestamp: 1_777_000_001 },
        {
          role: 'aegis',
          text: '约 44 分钟 [E1]',
          timestamp: 1_777_000_002,
          evidence: [{
            id: 'E1',
            tool: 'commute_traffic_monitor',
            label: '通勤检查',
            excerpt: '当前交通约 44 分钟。',
            full_len: 1234,
          }],
        },
        {
          role: 'aegis',
          text: '出发约 60–80 分钟',
          timestamp: 1_777_000_003,
          unsupported_numbers: ['60–80'],
        },
        {
          role: 'aegis',
          text: '已上传文件。',
          timestamp: 1_777_000_004,
          source_evidence: [{
            kind: 'tool',
            label: 'Google Drive upload',
            summary: 'File uploaded to Google Drive successfully.',
            detail: 'Current evidence:\n- Tool returned ok: File uploaded.',
            jev: {
              verdict: 'allow',
              audit_tip: 'tip-abc',
              risk: 'high_risk=0.10',
            },
          }],
        },
      ],
    });

    assert.equal(mapped[0].result, undefined);
    assert.equal(mapped[1].result, undefined);
    assert.deepEqual(mapped[2].result, {
      schema_version: 1,
      response_kind: 'answer',
      visible_answer: '约 44 分钟 [E1]',
      receipt_expectation: '',
      citations: [],
      warnings: [],
      evidence: [{
        id: 'E1',
        tool: 'commute_traffic_monitor',
        label: '通勤检查',
        excerpt: '当前交通约 44 分钟。',
        full_len: 1234,
      }],
    });
    assert.deepEqual(mapped[3].result, {
      schema_version: 1,
      response_kind: 'answer',
      visible_answer: '出发约 60–80 分钟',
      receipt_expectation: '',
      citations: [],
      warnings: [],
      unsupported_numbers: ['60–80'],
    });
    assert.equal(mapped[4].result.source_evidence[0].jev.verdict, 'allow');
    assert.equal(mapped[4].result.source_evidence[0].jev.audit_tip, 'tip-abc');
    assert.equal(mapped[4].result.source_evidence[0].label, 'Google Drive upload');
  });

  it('fails closed on malformed payloads and clears on logout cleanup', () => {
    const storage = memoryStorage();
    storage.setItem('chat', '{not-json');
    assert.deepEqual(readChatHomeHistory(storage, 'chat'), []);

    writeChatHomeHistory(storage, 'chat', [{ id: 'owner:1', role: 'owner', text: 'hello' }]);
    assert.equal(readChatHomeHistory(storage, 'chat').length, 1);

    clearChatHomeHistory(storage, 'chat');
    assert.deepEqual(readChatHomeHistory(storage, 'chat'), []);
  });
});


describe('recovered reply completion', () => {
  const raw = '明天晴，11–24°C。[E1]';
  const formatted = '明天晴，11–24°C。 [E1]';
  const owner = { id: 'owner', role: 'owner', text: '明天天气如何', timestamp: 100_000 };
  const recovered = { id: 'server', role: 'aegis', text: raw, timestamp: 133_000, latencyMs: 33000 };
  const final = { id: 'final', role: 'aegis', text: formatted, timestamp: 170_000,
    result: { schema_version: 1, response_kind: 'answer', visible_answer: formatted,
      receipt_expectation: '', citations: [], warnings: ['tool_blocked:other'],
      evidence: [{ id: 'E1', tool: 'web_search', label: 'Web search', excerpt: '晴，11–24°C', full_len: 10 }],
      source_evidence: [{kind: 'context', label: 'Location', summary: 'Location', detail: 'Example City'}] } };

  it('upgrades a recovered answer in place when only citation spacing differs', () => {
    const completed = completeChatHomeTurn([owner, recovered], owner.text, owner.timestamp, final);
    assert.equal(completed.length, 2);
    assert.equal(completed[1].id, 'server');
    assert.equal(completed[1].timestamp, recovered.timestamp);
    assert.deepEqual(completed[1].result, final.result);
  });

  it('merges history arriving after completion without losing the final envelope', () => {
    const projected = { ...recovered, result: { ...final.result, source_evidence: undefined, warnings: [] } };
    const merged = mergeChatHomeServerAndLocalHistory([owner, projected], [owner, final]);
    assert.equal(merged.length, 2);
    assert.deepEqual(merged[1].result, final.result);
  });

  it('fills evidence that was absent from a saved local envelope', () => {
    const local = {...final, result: {...final.result, evidence: undefined}};
    const projected = {...recovered, result: {...final.result, source_evidence: undefined, warnings: []}};
    const merged = mergeChatHomeServerAndLocalHistory([owner, projected], [owner, local]);
    assert.deepEqual(merged[1].result.evidence, final.result.evidence);
    assert.deepEqual(merged[1].result.source_evidence, final.result.source_evidence);
    assert.deepEqual(merged[1].result.warnings, final.result.warnings);
  });

  it('heals an already persisted raw/final duplicate on history refresh', () => {
    const merged = mergeChatHomeServerAndLocalHistory([owner, recovered], [owner, recovered, final]);
    assert.equal(merged.length, 2);
    assert.deepEqual(merged[1].result, final.result);
  });

  it('matches the closest owner timestamp when the same question is queued again', () => {
    const again = {...owner, id: 'again', timestamp: owner.timestamp + 40_000};
    const completed = completeChatHomeTurn([owner, recovered, again], owner.text, owner.timestamp, final);
    assert.equal(completed.length, 3);
    assert.deepEqual(completed[1].result, final.result);
    assert.equal(completed[2].id, 'again');
  });

  it('keeps equal answers to separate owner turns during history merge', () => {
    const again = {...owner, id: 'again', timestamp: 500_000};
    const laterAnswer = {...final, timestamp: 533_000};
    const merged = mergeChatHomeServerAndLocalHistory([owner, recovered], [again, laterAnswer]);
    assert.equal(merged.length, 4);
    assert.equal(merged[1].result, undefined);
    assert.deepEqual(merged[3].result, final.result);
  });

  it('does not collapse different numbers, evidence references or a later question', () => {
    for (const text of ['明天晴，12–24°C。 [E1]', '明天晴，11–24°C。 [E2]']) {
      assert.equal(completeChatHomeTurn([owner, recovered], owner.text, owner.timestamp, {...final, text}).length, 3);
    }
    const again = { ...owner, id: 'again', timestamp: 200_000 };
    const completed = completeChatHomeTurn([owner, recovered, again], again.text, again.timestamp, final);
    assert.equal(completed.length, 4);
    assert.equal(completed[1].result, undefined);
    assert.deepEqual(completed[3].result, final.result);
  });
});

describe('stable chat turn identity', () => {
  const owner = {id: 'local-owner', role: 'owner', text: '明天天气如何', timestamp: 100_000, turnId: 'turn-a'};
  const recovered = {id: 'server-a', role: 'aegis', text: '晴。[E1,E2]', timestamp: 133_000, turnId: 'turn-a'};
  const final = {id: 'final-a', role: 'aegis', text: '晴。 [E2]', timestamp: 170_000, turnId: 'turn-a',
    result: {schema_version: 1, response_kind: 'answer', visible_answer: '晴。 [E2]', turn_id: 'turn-a',
      receipt_expectation: '', citations: [], warnings: [],
      evidence: [{id: 'E2', tool: 'web_search', label: '天气', excerpt: '晴', full_len: 1}]}};

  it('replaces the recovered response when final citation filtering changes the text', () => {
    const messages = completeChatHomeTurn([owner, recovered], owner.text, owner.timestamp, final);
    assert.equal(messages.length, 2);
    assert.equal(messages[1].id, recovered.id);
    assert.equal(messages[1].text, final.text);
    assert.deepEqual(messages[1].result, final.result);
  });

  it('uses identity despite queued delay or a different displayed owner prompt', () => {
    const storedOwner = {...owner, text: 'expanded runtime prompt', timestamp: 600_000};
    const messages = completeChatHomeTurn([storedOwner, recovered], owner.text, owner.timestamp, final);
    assert.equal(messages.length, 2);
    assert.equal(messages[1].text, final.text);
  });

  it('keeps final text and evidence when raw history arrives later, including an existing duplicate', () => {
    for (const current of [[owner, final], [owner, recovered, final]]) {
      const messages = mergeChatHomeServerAndLocalHistory([owner, recovered], current);
      assert.equal(messages.length, 2);
      assert.equal(messages[1].text, final.text);
      assert.deepEqual(messages[1].result, final.result);
    }
  });

  it('matches clipped history without clipping the live final answer', () => {
    const text = '完整回复。'.repeat(850) + ' [E2]';
    const longFinal = {...final, text, result: {...final.result, visible_answer: text}};
    const clipped = {...recovered, text: text.slice(0, 4000)};
    const completed = completeChatHomeTurn([owner, clipped], owner.text, owner.timestamp, longFinal);
    const messages = mergeChatHomeServerAndLocalHistory([owner, clipped], completed);
    assert.equal(messages.length, 2);
    assert.equal(messages[1].text, text);
    assert.equal(messages[1].result.visible_answer, text);
  });

  it('keeps identical answers to distinct request identities within the same second', () => {
    const anotherOwner = {...owner, id: 'owner-b', turnId: 'turn-b'};
    const anotherReply = {...final, id: 'reply-b', turnId: 'turn-b', result: {...final.result, turn_id: 'turn-b'}};
    const messages = mergeChatHomeServerAndLocalHistory([owner, final], [anotherOwner, anotherReply]);
    assert.equal(messages.length, 4);
    assert.equal(completeChatHomeTurn([owner, final, anotherOwner], owner.text, owner.timestamp, anotherReply).length, 4);
  });

  it('preserves identity through server mapping and local storage', () => {
    const mapped = chatHomeMessagesFromServerHistory({messages: [
      {role: 'owner', text: owner.text, timestamp: 100, turn_id: 'turn-a'},
      {role: 'aegis', text: final.text, timestamp: 133, turn_id: 'turn-a', evidence: final.result.evidence},
    ]});
    assert.equal(mapped[0].turnId, 'turn-a');
    assert.equal(mapped[1].turnId, 'turn-a');
    const storage = memoryStorage();
    writeChatHomeHistory(storage, 'chat', [owner, final]);
    const restored = readChatHomeHistory(storage, 'chat');
    assert.equal(restored[0].turnId, 'turn-a');
    assert.equal(restored[1].turnId, 'turn-a');
    assert.equal(restored[1].result.turn_id, 'turn-a');
    assert.equal(mergeChatHomeServerAndLocalHistory(mapped, restored).length, 2);
  });

  it('maps a failure-hint retry_text onto the plain reply as Try again', () => {
    const mapped = chatHomeMessagesFromServerHistory({messages: [
      {role: 'owner', text: '帮我看看这封信', timestamp: 100, turn_id: 'turn-a'},
      {role: 'aegis', text: '这轮没有生成回复。消息已留下，可以再发一次。', timestamp: 100, turn_id: 'turn-a', severity: 'warning', retry_text: '帮我看看这封信'},
      {role: 'aegis', text: '信已归档。', timestamp: 133, turn_id: 'turn-b'},
    ]});
    assert.equal(mapped[1].retryText, '帮我看看这封信');
    assert.equal(mapped[1].retryLabel, 'Try again');
    assert.equal(mapped[1].result, undefined);
    assert.equal(mapped[2].retryText, undefined);
    const merged = mergeChatHomeServerAndLocalHistory(mapped, mapped);
    assert.equal(merged[1].retryText, '帮我看看这封信');
    assert.equal(merged[1].retryLabel, 'Try again');
  });

  it('replaces a timed-out request with its recovered answer and removes obsolete retry controls', () => {
    const failed = {...final, text: 'timeout', retryText: owner.text, retryIdempotencyKey: 'turn-a', retryLabel: 'Check result',
      result: {...final.result, response_kind: 'error', visible_answer: 'timeout'}};
    for (const messages of [
      completeChatHomeTurn([owner, failed], owner.text, owner.timestamp, final),
      mergeChatHomeServerAndLocalHistory([owner, recovered], [owner, failed]),
    ]) {
      assert.equal(messages.length, 2);
      assert.notEqual(messages[1].text, 'timeout');
      assert.notEqual(messages[1].result?.response_kind, 'error');
      assert.equal(messages[1].retryText, undefined);
      assert.equal(messages[1].retryIdempotencyKey, undefined);
    }
  });

  it('retains a finalized command error instead of restoring the unfinished model preamble', () => {
    const raw = {...recovered, text: '正在查询…'};
    const finished = {...final, replyOrigin: 'command', severity: 'warning', text: '查询未完成，请重试。',
      result: {...final.result, response_kind: 'error', visible_answer: '查询未完成，请重试。'}};
    const messages = mergeChatHomeServerAndLocalHistory([owner, raw], [owner, finished]);
    assert.equal(messages.length, 2);
    assert.equal(messages[1].text, finished.text);
    assert.equal(messages[1].severity, 'warning');
    assert.deepEqual(messages[1].result, finished.result);
  });

  it('prefers the final command over a mapped history envelope when healing duplicates', () => {
    const mapped = chatHomeMessagesFromServerHistory({messages: [
      {role: 'owner', text: owner.text, timestamp: 100, turn_id: 'turn-a'},
      {role: 'aegis', text: recovered.text, timestamp: 133, turn_id: 'turn-a', evidence: final.result.evidence},
    ]});
    const finished = {...final, replyOrigin: 'command', result: {...final.result, warnings: ['Keep this warning']}};
    const messages = mergeChatHomeServerAndLocalHistory(mapped, [...mapped, finished]);
    assert.equal(messages.length, 2);
    assert.equal(messages[1].text, final.text);
    assert.deepEqual(messages[1].result, finished.result);
    const saved = compactChatHomeHistory(messages);
    assert.equal(saved[1].replyOrigin, 'command');
    assert.equal(mergeChatHomeServerAndLocalHistory(mapped, saved)[1].replyOrigin, 'command');
  });
});
