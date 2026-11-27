import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

// Built by `npm run test:vnext-chat-work-state` as a single esbuild bundle
// (chatWorkState + the i18n catalog in one module). Never run via direct
// `node --test` — the stale-bundle false-pass trap applies.
const modulePath = process.env.VNEXT_CHAT_WORK_STATE_MODULE;
if (!modulePath) throw new Error('VNEXT_CHAT_WORK_STATE_MODULE is required');
const {
  syncChatWorkMessagesWithQueue,
  updateChatWorkMessagesAfterItemStatus,
} = await import(modulePath);

function item(overrides = {}) {
  return {
    id: overrides.id ?? 'email:42',
    kind: overrides.kind ?? 'email',
    source: overrides.source ?? 'gmail',
    source_id: overrides.source_id ?? '42',
    title: overrides.title ?? 'Billing statement available',
    summary: overrides.summary ?? 'Safe summary',
    status: overrides.status ?? 'unread',
    urgency: overrides.urgency ?? 'normal',
    sensitivity: overrides.sensitivity ?? 'S2',
    privacy_level: overrides.privacy_level ?? 'S2',
    created_at: '2026-05-17T00:00:00Z',
    updated_at: '2026-05-17T00:00:00Z',
    secondary_actions: [],
    external_url_policy: 'none',
    related_action_ids: [],
    related_receipt_ids: [],
    route_reason: 'test',
    schema_version: 1,
    ...overrides,
  };
}

function queue(items) {
  return {
    schema_version: 1,
    generated_at: '2026-05-17T00:00:00Z',
    owner: 'owner',
    sources: [],
    counts: {},
    items,
  };
}

describe('vNext chat work state', () => {
  it('updates the embedded work card and clears stale item answers after a status change', () => {
    const messages = [
      {
        id: 'aegis:1',
        role: 'aegis',
        text: 'work card',
        workItem: item({ status: 'unread' }),
        workItemThread: [{ id: 'thread:1', question: 'Explain', result: { visible_answer: 'Unread.' } }],
      },
    ];

    const updated = updateChatWorkMessagesAfterItemStatus(messages, 'email:42', 'archived');

    assert.equal(updated[0].workItem.status, 'archived');
    assert.deepEqual(updated[0].workItemThread, []);
  });

  it('syncs cards from the refreshed queue and drops stale threads only when status changed', () => {
    const stableThread = [{ id: 'thread:stable', question: 'Why?' }];
    const staleThread = [{ id: 'thread:stale', question: 'Explain' }];
    const messages = [
      {
        id: 'aegis:stable',
        role: 'aegis',
        text: 'stable card',
        workItem: item({ id: 'task:1', kind: 'task', status: 'active', title: 'Old task title' }),
        workItemThread: stableThread,
      },
      {
        id: 'aegis:stale',
        role: 'aegis',
        text: 'stale card',
        workItem: item({ id: 'email:42', status: 'unread' }),
        workItemThread: staleThread,
      },
    ];

    const synced = syncChatWorkMessagesWithQueue(messages, queue([
      item({ id: 'task:1', kind: 'task', status: 'active', title: 'Updated task title' }),
      item({ id: 'email:42', status: 'archived' }),
    ]), 'en');

    assert.equal(synced[0].workItem.title, 'Updated task title');
    assert.equal(synced[0].workItemThread, stableThread);
    assert.equal(synced[1].workItem.status, 'archived');
    assert.deepEqual(synced[1].workItemThread, []);
    assert.equal(synced[1].workItemUnavailable, false);
  });

  it('attaches anchored proactive notices to matching work items from the queue', () => {
    const messages = [
      {
        id: 'aegis:notice',
        role: 'aegis',
        text: 'Sensitive item needs review.',
        workItemAnchorId: 'email:42',
      },
    ];

    const synced = syncChatWorkMessagesWithQueue(messages, queue([
      item({ id: 'email:42', title: 'Anchored mail' }),
    ]), 'en');

    assert.equal(synced[0].workItem.title, 'Anchored mail');
    assert.deepEqual(synced[0].workItemThread, []);
    assert.equal(synced[0].workItemUnavailable, false);
  });

  it('marks embedded work cards unavailable when refreshed queue no longer contains the item', () => {
    const staleThread = [{ id: 'thread:stale', question: 'Archive?' }];
    const messages = [
      {
        id: 'aegis:stale',
        role: 'aegis',
        text: 'stale card',
        workItem: item({
          id: 'email:42',
          status: 'unread',
          secondary_actions: [{ label: 'Archive', action_kind: 'mail.archive' }],
        }),
        workItemThread: staleThread,
      },
    ];

    const synced = syncChatWorkMessagesWithQueue(messages, queue([]), 'en');

    assert.equal(synced[0].workItem.status, 'done');
    assert.equal(synced[0].workItem.summary, 'This item changed after it was shown here. Aegis refreshed the work queue and hid the old actions.');
    assert.deepEqual(synced[0].workItem.secondary_actions, []);
    assert.deepEqual(synced[0].workItemThread, []);
    assert.equal(synced[0].workItemUnavailable, true);
  });

  it('preserves already closed embedded work cards when they leave the active queue', () => {
    const messages = [
      {
        id: 'aegis:archived',
        role: 'aegis',
        text: 'archived card',
        workItem: item({
          id: 'email:42',
          status: 'archived',
          summary: 'Archived. Aegis recorded the receipt.',
          secondary_actions: [{ label: 'Archive', action_kind: 'mail.archive' }],
        }),
        workItemThread: [],
        workItemUnavailable: false,
      },
    ];

    const synced = syncChatWorkMessagesWithQueue(messages, queue([]), 'en');

    assert.equal(synced[0].workItem.status, 'archived');
    assert.equal(synced[0].workItem.summary, 'Archived. Aegis recorded the receipt.');
    assert.equal(synced[0].workItemUnavailable, false);
  });

  it('localizes the stale-card replacement strings per locale while status stays a wire enum', () => {
    const makeMessages = () => [
      {
        id: 'aegis:stale',
        role: 'aegis',
        text: 'stale card',
        workItem: item({ id: 'email:42', status: 'unread' }),
        workItemThread: [],
      },
    ];

    const zh = syncChatWorkMessagesWithQueue(makeMessages(), queue([]), 'zh');
    // status is identity (CLOSED_CHAT_WORK_STATUSES comparison) — never localized.
    assert.equal(zh[0].workItem.status, 'done');
    assert.equal(zh[0].workItem.summary, '此条目在显示后已发生变化。安琪已刷新工作队列并隐藏旧操作。');
    assert.equal(zh[0].workItem.next_safe_action, '如仍需要，请问安琪。');
    assert.equal(zh[0].workItemUnavailable, true);

    const en = syncChatWorkMessagesWithQueue(makeMessages(), queue([]), 'en');
    assert.equal(en[0].workItem.status, 'done');
    assert.equal(en[0].workItem.summary, 'This item changed after it was shown here. Aegis refreshed the work queue and hid the old actions.');
    assert.equal(en[0].workItem.next_safe_action, 'Ask Aegis if you still need it.');
  });
});


it('keeps an abandoned goal closed after it leaves the active queue', () => {
  const goal = item({ id: 'goal:42', kind: 'goal', source: 'goals', status: 'tracking' });
  const changed = updateChatWorkMessagesAfterItemStatus([{ workItem: goal }], 'goal:42', 'abandoned');
  const synced = syncChatWorkMessagesWithQueue(changed, { items: [] }, 'zh');
  assert.equal(synced[0].workItem.status, 'abandoned');
  assert.equal(synced[0].workItemUnavailable, false);
});
